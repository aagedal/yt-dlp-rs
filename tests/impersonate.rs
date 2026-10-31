//! `--impersonate`, `--list-impersonate-targets` and extractor requests
//! marked for impersonation. With `--features impersonate`, a local
//! `CONNECT` proxy records the TLS ClientHello, the negotiated ALPN and the
//! first HTTP/2 SETTINGS frame or HTTP/1.1 request head of the `wreq`
//! transport, compared with the default rustls client.
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("--ignore-config")
        .args(args)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn invalid_targets_are_usage_errors() {
    for target in ["a:b:c", "-120", "chrome--1"] {
        let output = run(&["--impersonate", target, "https://example.invalid/"]);
        assert_eq!(output.status.code(), Some(2));
        assert!(
            stderr(&output).contains(&format!(
                "yt-dlp-rs: error: Invalid impersonate target \"{target}\""
            )),
            "{}",
            stderr(&output)
        );
    }
    // Parsed (lowercased) before `--list-impersonate-targets` like upstream.
    let output = run(&["--impersonate", "a:b:c", "--list-impersonate-targets"]);
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn unavailable_targets_report_upstream_error_with_the_full_target() {
    for (target, printed) in [
        ("Chrome-110:Windows-10", "chrome-110:windows-10"),
        ("tor", "tor"),
        ("chrome-1", "chrome-1"),
    ] {
        let output = run(&["--impersonate", target, "https://example.invalid/"]);
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(
            stderr(&output),
            format!(
                "ERROR: Impersonate target \"{printed}\" is not available. Use --list-impersonate-targets to see available targets. You may be missing dependencies required to support this target.\n"
            )
        );
    }
}

#[cfg(not(feature = "impersonate"))]
#[test]
fn default_build_lists_only_unavailable_wreq_targets() {
    let output = run(&["--list-impersonate-targets"]);
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[info] Available impersonate targets\n\
         Client    OS   Source\n\
         ---------------------------------\n\
         Edge      -    wreq (unavailable)\n\
         Firefox   -    wreq (unavailable)\n\
         Safari    -    wreq (unavailable)\n\
         Chrome    -    wreq (unavailable)\n"
    );
    for target in ["chrome", ""] {
        let output = run(&["--impersonate", target, "https://example.invalid/"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(stderr(&output).contains(&format!(
            "ERROR: Impersonate target \"{target}\" is not available."
        )));
    }
}

#[cfg(feature = "impersonate")]
mod transport {
    use super::*;
    use rustls::{
        ServerConfig, ServerConnection, StreamOwned,
        pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer},
    };
    use std::{
        io::{Cursor, Read, Write},
        net::{TcpListener, TcpStream},
        sync::{Arc, mpsc},
        thread,
        time::Duration,
    };

    const CERT: &[u8] = include_bytes!("fixtures/proxies/tls/server.der");
    const KEY: &[u8] = include_bytes!("fixtures/proxies/tls/key.der");

    #[derive(Debug)]
    struct Captured {
        cipher_suites: Vec<u16>,
        extensions: Vec<u16>,
        alpn: Option<Vec<u8>>,
        /// The HTTP/2 SETTINGS entries, or the HTTP/1.1 request head.
        settings: Vec<(u16, u32)>,
        head: String,
    }

    fn is_grease(value: u16) -> bool {
        value & 0x0f0f == 0x0a0a && value >> 8 == value & 0xff
    }

    /// Cipher suites and extension types of a ClientHello record.
    fn parse_hello(record: &[u8]) -> (Vec<u16>, Vec<u16>) {
        let u16_at = |at: usize| u16::from_be_bytes([record[at], record[at + 1]]);
        // Record (5) + handshake header (4) + version (2) + random (32).
        let mut at = 5 + 4 + 2 + 32;
        at += 1 + record[at] as usize;
        let suites_len = u16_at(at) as usize;
        let suites = (0..suites_len / 2)
            .map(|i| u16_at(at + 2 + 2 * i))
            .collect();
        at += 2 + suites_len;
        at += 1 + record[at] as usize;
        let end = at + 2 + u16_at(at) as usize;
        at += 2;
        let mut extensions = Vec::new();
        while at + 4 <= end {
            extensions.push(u16_at(at));
            at += 4 + u16_at(at + 2) as usize;
        }
        (suites, extensions)
    }

    fn read_until(stream: &mut impl Read, done: impl Fn(&[u8]) -> bool) -> Vec<u8> {
        let mut data = Vec::new();
        let mut buffer = [0; 4096];
        while !done(&data) {
            match stream.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(n) => data.extend_from_slice(&buffer[..n]),
            }
        }
        data
    }

    fn capture(mut stream: TcpStream, config: Arc<ServerConfig>) -> Option<Captured> {
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .ok()?;
        let head = read_until(&mut stream, |data| {
            data.windows(4).any(|w| w == b"\r\n\r\n")
        });
        if !head.starts_with(b"CONNECT ") {
            return None;
        }
        stream
            .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
            .ok()?;
        let record = read_until(&mut stream, |data| {
            data.len() >= 5 && data.len() >= 5 + u16::from_be_bytes([data[3], data[4]]) as usize
        });
        let (cipher_suites, extensions) = parse_hello(&record);
        let mut connection = ServerConnection::new(config).ok()?;
        connection.read_tls(&mut Cursor::new(&record)).ok()?;
        connection.process_new_packets().ok()?;
        let mut tls = StreamOwned::new(connection, stream);
        while tls.conn.is_handshaking() {
            tls.conn.complete_io(&mut tls.sock).ok()?;
        }
        let alpn = tls.conn.alpn_protocol().map(<[u8]>::to_vec);
        let mut settings = Vec::new();
        let mut text = String::new();
        if alpn.as_deref() == Some(b"h2") {
            // The connection preface, then the client's SETTINGS frame.
            let data = read_until(&mut tls, |data| {
                data.len() >= 33
                    && data.len()
                        >= 33 + u32::from_be_bytes([0, data[24], data[25], data[26]]) as usize
            });
            assert!(data.starts_with(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"));
            assert_eq!(data[27], 4, "SETTINGS frame");
            let len = u32::from_be_bytes([0, data[24], data[25], data[26]]) as usize;
            for entry in data[33..33 + len].chunks(6) {
                settings.push((
                    u16::from_be_bytes([entry[0], entry[1]]),
                    u32::from_be_bytes([entry[2], entry[3], entry[4], entry[5]]),
                ));
            }
        } else {
            let data = read_until(&mut tls, |data| data.windows(4).any(|w| w == b"\r\n\r\n"));
            text = String::from_utf8_lossy(&data).into_owned();
            let _ = tls.write_all(
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
        Some(Captured {
            cipher_suites,
            extensions,
            alpn,
            settings,
            head: text,
        })
    }

    /// A `CONNECT` proxy offering `alpn`; the first captured connection.
    fn proxy(alpn: &[&[u8]]) -> (String, mpsc::Receiver<Captured>) {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .unwrap()
            .with_no_client_auth()
            .with_single_cert(
                vec![CertificateDer::from(CERT.to_vec())],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(KEY.to_vec())),
            )
            .unwrap();
        config.alpn_protocols = alpn.iter().map(|protocol| protocol.to_vec()).collect();
        let config = Arc::new(config);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (config, sender) = (config.clone(), sender.clone());
                thread::spawn(move || {
                    if let Some(captured) = capture(stream, config) {
                        let _ = sender.send(captured);
                    }
                });
            }
        });
        (url, receiver)
    }

    fn captured(alpn: &[&[u8]], args: &[&str], url: &str) -> (Captured, Output) {
        let (proxy, receiver) = proxy(alpn);
        let mut all = vec!["--proxy", &proxy, "--no-check-certificate"];
        all.extend_from_slice(args);
        all.push(url);
        let output = run(&all);
        let captured = receiver
            .recv_timeout(Duration::from_secs(10))
            .unwrap_or_else(|_| panic!("no TLS connection: {}", stderr(&output)));
        (captured, output)
    }

    /// `mir24tv` requests its page with `impersonate=True`.
    const IMPERSONATED: &str = "https://mir24.tv/news/16635210/x";
    const PLAIN: &str = "https://example.com/video.mp4";

    #[test]
    fn impersonated_requests_send_chrome_tls_and_http2_fingerprints() {
        let (plain, _) = captured(&[b"h2", b"http/1.1"], &[], PLAIN);
        let (chrome, output) = captured(&[b"h2", b"http/1.1"], &["-v"], IMPERSONATED);
        // No "no impersonate target" warning when a target is available.
        assert!(!stderr(&output).contains("attempting impersonation"));
        // Chrome's GREASE values and certificate compression; rustls has
        // neither and offers no HTTP/2.
        assert!(chrome.cipher_suites.iter().any(|&suite| is_grease(suite)));
        assert!(chrome.extensions.iter().any(|&ext| is_grease(ext)));
        assert!(chrome.extensions.contains(&0x001b), "{chrome:?}");
        assert!(!plain.cipher_suites.iter().any(|&suite| is_grease(suite)));
        assert!(!plain.extensions.contains(&0x001b));
        assert_ne!(chrome.cipher_suites, plain.cipher_suites);
        assert_eq!(chrome.alpn.as_deref(), Some(&b"h2"[..]));
        assert_eq!(plain.alpn.as_deref(), Some(&b"http/1.1"[..]));
        // Chrome's HTTP/2 SETTINGS: HEADER_TABLE_SIZE, ENABLE_PUSH,
        // INITIAL_WINDOW_SIZE and MAX_HEADER_LIST_SIZE, in that order.
        assert_eq!(
            chrome.settings,
            [(1, 65536), (2, 0), (4, 6291456), (6, 262144)]
        );
    }

    #[test]
    fn impersonated_headers_replace_std_headers_but_keep_user_agents() {
        let (chrome, _) = captured(&[b"http/1.1"], &[], IMPERSONATED);
        let head = chrome.head.to_ascii_lowercase();
        let names: Vec<&str> = head
            .lines()
            .skip(1)
            .filter_map(|line| line.split_once(':').map(|(name, _)| name))
            .collect();
        // Upstream's resolved default target is the most preferred Chrome
        // (here 149 on Windows), with its headers in Chrome's order.
        assert!(
            head.contains(
                "\r\nuser-agent: mozilla/5.0 (windows nt 10.0; win64; x64) applewebkit/537.36 (khtml, like gecko) chrome/149."
            ),
            "{head}"
        );
        assert!(
            head.contains("\r\nsec-ch-ua-platform: \"windows\"\r\n"),
            "{head}"
        );
        let position = |name: &str| names.iter().position(|found| *found == name).unwrap();
        assert!(position("sec-ch-ua") < position("user-agent"), "{names:?}");
        assert!(position("user-agent") < position("accept"), "{names:?}");
        // Upstream's `std_headers` values are dropped, and the port's
        // `Accept-Encoding: identity` gives way to the browser's.
        assert!(!head.contains("en-us,en;q=0.5"), "{head}");
        assert!(!head.contains("accept-encoding: identity"), "{head}");
        assert!(
            head.contains("\r\naccept-encoding: gzip, deflate, br, zstd\r\n"),
            "{head}"
        );
        // A user agent other than upstream's random one is sent as given.
        let (custom, _) = captured(
            &[b"http/1.1"],
            &["--user-agent", "custom-agent"],
            IMPERSONATED,
        );
        assert!(
            custom
                .head
                .to_ascii_lowercase()
                .contains("\r\nuser-agent: custom-agent\r\n"),
            "{}",
            custom.head
        );
    }

    #[test]
    fn available_targets_are_listed_from_wreq() {
        let output = run(&["--list-impersonate-targets"]);
        assert!(output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        let lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(lines[0], "[info] Available impersonate targets");
        assert!(lines[1].starts_with("Client "), "{stdout}");
        assert!(!stdout.contains("unavailable"), "{stdout}");
        assert!(!stdout.contains("curl_cffi"), "{stdout}");
        // Least preferred first: mobile Safari, ..., the newest Chrome last.
        assert!(
            lines[3].starts_with("Safari-18.1.1   Ios-18.1.1"),
            "{stdout}"
        );
        assert_eq!(*lines.last().unwrap(), "Chrome-149      Windows-10   wreq");
        for row in [
            "Firefox-151     Linux        wreq",
            "Edge-148        Windows-10   wreq",
            "Safari-26.4     Macos        wreq",
        ] {
            assert!(lines.contains(&row), "{row}\n{stdout}");
        }
    }

    #[test]
    fn an_impersonate_option_applies_to_extraction_requests() {
        // The generic extractor's request for a plain URL is impersonated.
        let (chrome, output) = captured(
            &[b"h2", b"http/1.1"],
            &["--impersonate", "chrome", "-s"],
            PLAIN,
        );
        assert!(
            chrome.cipher_suites.iter().any(|&suite| is_grease(suite)),
            "{}",
            stderr(&output)
        );
        assert_eq!(chrome.alpn.as_deref(), Some(&b"h2"[..]));
    }
}
