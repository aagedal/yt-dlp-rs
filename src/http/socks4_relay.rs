//! SOCKS4/4a USERID for upstream's `socks4://USER@HOST` proxies. The native
//! SOCKS4 connector always sends an empty USERID, so the client connects to
//! a loopback relay over SOCKS5 instead (`socks5` for SOCKS4's local DNS,
//! `socks5h` for SOCKS4a's remote DNS). The relay sends each CONNECT to the
//! real proxy as SOCKS4 with the user ID inserted (`_setup_socks4`'s
//! packet), answers with the proxy's verdict, then copies both directions.
//!
//! The loopback port is reachable by every local user, so the client
//! authenticates with a random per-relay username and password (RFC 1929);
//! other connections are closed before the proxy is contacted.
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

/// A relay's proxy host, port and user ID.
type RelayKey = (String, u16, Vec<u8>);

/// A running relay: its loopback port and the client's credential (sent as
/// both username and password).
#[derive(Clone)]
pub(crate) struct Relay {
    pub(crate) port: u16,
    pub(crate) token: String,
}

/// Running relays by proxy address and user ID.
static RELAYS: Mutex<BTreeMap<RelayKey, Relay>> = Mutex::new(BTreeMap::new());

/// The relay to `host:port` sending `userid`, started on first use and kept
/// for the process.
pub(crate) fn start(host: &str, port: u16, userid: &[u8], timeout: Duration) -> io::Result<Relay> {
    let key = (host.to_owned(), port, userid.to_vec());
    let mut relays = RELAYS.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(relay) = relays.get(&key) {
        return Ok(relay.clone());
    }
    let mut secret = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut secret)
        .map_err(|_| io::Error::other("no random source for the SOCKS4 relay"))?;
    let token: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let relay = Relay {
        port: listener.local_addr()?.port(),
        token,
    };
    let (host, userid, token) = (key.0.clone(), key.2.clone(), relay.token.clone());
    std::thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let (host, userid, token) = (host.clone(), userid.clone(), token.clone());
            std::thread::spawn(move || {
                let _ = serve(client, &host, port, &userid, &token, timeout);
            });
        }
    });
    relays.insert(key, relay.clone());
    Ok(relay)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

/// `length` bytes, then the field (a SOCKS5 length-prefixed field).
fn read_field(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut length = [0];
    stream.read_exact(&mut length)?;
    let mut field = vec![0; usize::from(length[0])];
    stream.read_exact(&mut field)?;
    Ok(field)
}

/// Compare without stopping at the first difference.
fn same(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0, |difference, (a, b)| difference | (a ^ b))
            == 0
}

/// A SOCKS5 reply with an IPv4 bound address.
fn reply(client: &mut TcpStream, status: u8, address: [u8; 4], port: [u8; 2]) -> io::Result<()> {
    let mut packet = vec![5, status, 0, 1];
    packet.extend(address);
    packet.extend(port);
    client.write_all(&packet)
}

fn serve(
    mut client: TcpStream,
    host: &str,
    port: u16,
    userid: &[u8],
    token: &str,
    timeout: Duration,
) -> io::Result<()> {
    client.set_read_timeout(Some(timeout))?;
    // Method selection: only username/password authentication.
    let mut greeting = [0u8; 1];
    client.read_exact(&mut greeting)?;
    if greeting[0] != 5 {
        return Err(invalid("not a SOCKS5 request"));
    }
    let methods = read_field(&mut client)?;
    if !methods.contains(&2) {
        client.write_all(&[5, 0xff])?;
        return Err(invalid("SOCKS5 client offered no authentication"));
    }
    client.write_all(&[5, 2])?;
    let mut version = [0u8; 1];
    client.read_exact(&mut version)?;
    let username = read_field(&mut client)?;
    let password = read_field(&mut client)?;
    // Both checked, so a mismatch takes as long wherever it is.
    let authenticated =
        same(&username, token.as_bytes()) & same(&password, token.as_bytes()) & (version[0] == 1);
    if !authenticated {
        client.write_all(&[1, 1])?;
        return Err(invalid("SOCKS relay authentication failed"));
    }
    client.write_all(&[1, 0])?;
    // VER, CMD, RSV, ATYP; then the address and port.
    let mut request = [0u8; 4];
    client.read_exact(&mut request)?;
    if request[0] != 5 || request[1] != 1 {
        reply(&mut client, 7, [0; 4], [0; 2])?;
        return Err(invalid("only SOCKS CONNECT is relayed"));
    }
    let (address, domain) = match request[3] {
        1 => {
            let mut address = [0u8; 4];
            client.read_exact(&mut address)?;
            (address, None)
        }
        // SOCKS4a: DSTIP 0.0.0.1, then the domain after the USERID.
        3 => ([0, 0, 0, 1], Some(read_field(&mut client)?)),
        // SOCKS4 has no IPv6.
        _ => {
            reply(&mut client, 8, [0; 4], [0; 2])?;
            return Err(invalid("SOCKS4 cannot reach this address type"));
        }
    };
    let mut destination_port = [0u8; 2];
    client.read_exact(&mut destination_port)?;
    let proxy_address = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unresolved SOCKS proxy"))?;
    let mut proxy = match TcpStream::connect_timeout(&proxy_address, timeout) {
        Ok(proxy) => proxy,
        Err(error) => {
            reply(&mut client, 1, [0; 4], [0; 2])?;
            return Err(error);
        }
    };
    let mut packet = vec![4, 1];
    packet.extend(destination_port);
    packet.extend(address);
    packet.extend_from_slice(userid);
    packet.push(0);
    if let Some(domain) = domain {
        packet.extend(domain);
        packet.push(0);
    }
    proxy.write_all(&packet)?;
    proxy.set_read_timeout(Some(timeout))?;
    // VN, CD, DSTPORT, DSTIP; 0x5a grants the request.
    let mut verdict = [0u8; 8];
    proxy.read_exact(&mut verdict)?;
    let bound = [verdict[4], verdict[5], verdict[6], verdict[7]];
    if verdict[1] != 0x5a {
        // "Connection refused" carries SOCKS4's rejections.
        reply(&mut client, 5, bound, [verdict[2], verdict[3]])?;
        return Err(invalid("SOCKS4 proxy rejected the request"));
    }
    reply(&mut client, 0, bound, [verdict[2], verdict[3]])?;
    client.set_read_timeout(None)?;
    proxy.set_read_timeout(None)?;
    // The tunneled bytes in both directions.
    let (mut client_read, mut proxy_write) = (client.try_clone()?, proxy.try_clone()?);
    let upload = std::thread::spawn(move || {
        let _ = io::copy(&mut client_read, &mut proxy_write);
        let _ = proxy_write.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut proxy, &mut client);
    let _ = client.shutdown(Shutdown::Write);
    let _ = upload.join();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A SOCKS4 server recording the request, granting it and echoing.
    fn server() -> (u16, std::sync::mpsc::Receiver<Vec<u8>>) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut request = vec![0u8; 8];
                stream.read_exact(&mut request).unwrap();
                let nuls = if request[4..7] == [0, 0, 0] && request[7] != 0 {
                    2
                } else {
                    1
                };
                let mut seen = 0;
                while seen < nuls {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    seen += usize::from(byte[0] == 0);
                }
                sender.send(request).unwrap();
                stream.write_all(&[0, 0x5a, 0, 0, 0, 0, 0, 0]).unwrap();
                let mut payload = [0u8; 4];
                stream.read_exact(&mut payload).unwrap();
                stream.write_all(&payload).unwrap();
            }
        });
        (port, receiver)
    }

    /// A SOCKS5 handshake as `user:password`; the method and auth replies.
    fn handshake(port: u16, user: &str, password: &str) -> (TcpStream, [u8; 2], [u8; 2]) {
        let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
        client.write_all(&[5, 1, 2]).unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).unwrap();
        let mut auth = vec![1, user.len() as u8];
        auth.extend(user.as_bytes());
        auth.push(password.len() as u8);
        auth.extend(password.as_bytes());
        client.write_all(&auth).unwrap();
        let mut status = [0u8; 2];
        client.read_exact(&mut status).unwrap();
        (client, method, status)
    }

    #[test]
    fn relay_inserts_the_user_id_for_socks4_and_socks4a() {
        let (upstream, requests) = server();
        let relay = start("127.0.0.1", upstream, b"user", Duration::from_secs(5)).unwrap();
        for (request, expected) in [
            (
                b"\x05\x01\x00\x01\x0a\x00\x00\x01\x00\x50".to_vec(),
                b"\x04\x01\x00\x50\x0a\x00\x00\x01user\x00".to_vec(),
            ),
            (
                b"\x05\x01\x00\x03\x0bexample.com\x01\xbb".to_vec(),
                b"\x04\x01\x01\xbb\x00\x00\x00\x01user\x00example.com\x00".to_vec(),
            ),
        ] {
            let (mut client, method, status) = handshake(relay.port, &relay.token, &relay.token);
            assert_eq!((method, status), ([5, 2], [1, 0]));
            client.write_all(&request).unwrap();
            let mut reply = [0u8; 10];
            client.read_exact(&mut reply).unwrap();
            assert_eq!(reply[..2], [5, 0]);
            client.write_all(b"ping").unwrap();
            let mut echo = [0u8; 4];
            client.read_exact(&mut echo).unwrap();
            assert_eq!(&echo, b"ping");
            assert_eq!(requests.recv().unwrap(), expected);
        }
        // The same proxy and user share one relay.
        let again = start("127.0.0.1", upstream, b"user", Duration::from_secs(5)).unwrap();
        assert_eq!((again.port, again.token), (relay.port, relay.token));
    }

    #[test]
    fn other_local_clients_cannot_use_the_relay() {
        let (upstream, requests) = server();
        let relay = start("127.0.0.1", upstream, b"other", Duration::from_secs(5)).unwrap();
        let (_, _, status) = handshake(relay.port, "guess", "guess");
        assert_eq!(status, [1, 1]);
        // Without authentication (or as a SOCKS4 client) it is refused.
        let mut client = TcpStream::connect(("127.0.0.1", relay.port)).unwrap();
        client.write_all(&[5, 1, 0]).unwrap();
        let mut method = [0u8; 2];
        client.read_exact(&mut method).unwrap();
        assert_eq!(method, [5, 0xff]);
        let mut client = TcpStream::connect(("127.0.0.1", relay.port)).unwrap();
        client
            .write_all(b"\x04\x01\x00\x50\x0a\x00\x00\x01\x00")
            .unwrap();
        assert_eq!(client.read(&mut [0u8; 8]).unwrap_or(0), 0);
        // The proxy never saw a request.
        assert!(requests.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
