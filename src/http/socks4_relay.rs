//! SOCKS4/4a USERID for upstream's `socks4://USER@HOST` proxies. The native
//! SOCKS4 connector always sends an empty USERID, so it connects to a
//! loopback relay instead, which forwards each CONNECT request to the real
//! proxy with the user ID inserted (`_setup_socks4`'s packet), then copies
//! both directions.
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::Mutex;
use std::time::Duration;

/// SOCKS4 USERID and DOMAIN fields are NUL-terminated; bound what is read.
const MAX_FIELD: usize = 1024;

/// A relay's proxy host, port and user ID.
type RelayKey = (String, u16, Vec<u8>);

/// Running relays by proxy address and user ID: their loopback ports.
static RELAYS: Mutex<BTreeMap<RelayKey, u16>> = Mutex::new(BTreeMap::new());

/// The loopback port of a relay to `host:port` sending `userid`, started on
/// first use and kept for the process.
pub(crate) fn port(host: &str, port: u16, userid: &[u8], timeout: Duration) -> io::Result<u16> {
    let key = (host.to_owned(), port, userid.to_vec());
    let mut relays = RELAYS.lock().unwrap_or_else(|poison| poison.into_inner());
    if let Some(&local) = relays.get(&key) {
        return Ok(local);
    }
    let listener = TcpListener::bind(("127.0.0.1", 0))?;
    let local = listener.local_addr()?.port();
    let (host, userid) = (key.0.clone(), key.2.clone());
    std::thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let (host, userid) = (host.clone(), userid.clone());
            std::thread::spawn(move || {
                let _ = relay(client, &host, port, &userid, timeout);
            });
        }
    });
    relays.insert(key, local);
    Ok(local)
}

fn read_until_nul(stream: &mut TcpStream) -> io::Result<Vec<u8>> {
    let mut field = Vec::new();
    let mut byte = [0];
    loop {
        stream.read_exact(&mut byte)?;
        if byte[0] == 0 {
            return Ok(field);
        }
        if field.len() == MAX_FIELD {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "SOCKS4 field too long",
            ));
        }
        field.push(byte[0]);
    }
}

fn relay(
    mut client: TcpStream,
    host: &str,
    port: u16,
    userid: &[u8],
    timeout: Duration,
) -> io::Result<()> {
    client.set_read_timeout(Some(timeout))?;
    // VN, CD, DSTPORT, DSTIP; the connector's (empty) USERID; for SOCKS4a
    // (DSTIP 0.0.0.x, x != 0) the domain.
    let mut header = [0u8; 8];
    client.read_exact(&mut header)?;
    if header[0] != 4 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "not a SOCKS4 request",
        ));
    }
    read_until_nul(&mut client)?;
    let domain = (header[4..7] == [0, 0, 0] && header[7] != 0)
        .then(|| read_until_nul(&mut client))
        .transpose()?;
    let address = (host, port)
        .to_socket_addrs()?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "unresolved SOCKS proxy"))?;
    let mut proxy = TcpStream::connect_timeout(&address, timeout)?;
    let mut packet = header.to_vec();
    packet.extend_from_slice(userid);
    packet.push(0);
    if let Some(domain) = domain {
        packet.extend(domain);
        packet.push(0);
    }
    proxy.write_all(&packet)?;
    client.set_read_timeout(None)?;
    // The reply and the tunneled bytes in both directions.
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

    #[test]
    fn relay_inserts_the_user_id_for_socks4_and_socks4a() {
        let (upstream, requests) = server();
        let local = super::port("127.0.0.1", upstream, b"user", Duration::from_secs(5)).unwrap();
        for (request, expected) in [
            (
                b"\x04\x01\x00\x50\x0a\x00\x00\x01\x00".to_vec(),
                b"\x04\x01\x00\x50\x0a\x00\x00\x01user\x00".to_vec(),
            ),
            (
                b"\x04\x01\x01\xbb\x00\x00\x00\x01\x00example.com\x00".to_vec(),
                b"\x04\x01\x01\xbb\x00\x00\x00\x01user\x00example.com\x00".to_vec(),
            ),
        ] {
            let mut client = TcpStream::connect(("127.0.0.1", local)).unwrap();
            client.write_all(&request).unwrap();
            let mut reply = [0u8; 8];
            client.read_exact(&mut reply).unwrap();
            assert_eq!(reply[1], 0x5a);
            client.write_all(b"ping").unwrap();
            let mut echo = [0u8; 4];
            client.read_exact(&mut echo).unwrap();
            assert_eq!(&echo, b"ping");
            assert_eq!(requests.recv().unwrap(), expected);
        }
        // The same proxy and user share one relay.
        assert_eq!(
            super::port("127.0.0.1", upstream, b"user", Duration::from_secs(5)).unwrap(),
            local
        );
    }
}
