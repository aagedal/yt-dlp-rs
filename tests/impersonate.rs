//! `--impersonate`, `--list-impersonate-targets` and extractor requests
//! marked for impersonation. With `--features impersonate`, a local
//! `CONNECT` proxy records the TLS ClientHello, the negotiated ALPN and the
//! first HTTP/2 SETTINGS frame or HTTP/1.1 request head of the `wreq`
//! transport, compared with the default rustls client.
mod support;

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

/// Without a target, a format's or subtitle's `impersonate` field gives
/// HttpFD's warning for each transfer, which then goes ahead plainly, and
/// such a format never goes to an external downloader (pinned upstream's
/// output on the same input).
#[cfg(not(feature = "impersonate"))]
#[test]
fn format_and_subtitle_impersonation_fields_warn_without_targets() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut head = Vec::new();
            let mut byte = [0];
            while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                head.push(byte[0]);
            }
            let (kind, body): (&str, &[u8]) = if head.starts_with(b"GET /sub.vtt") {
                ("text/vtt", b"WEBVTT\n\n00:00.000 --> 00:01.000\nhi\n")
            } else {
                ("video/mp4", b"media bytes")
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(body);
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let info = serde_json::json!({
        "id": "imp",
        "title": "imp",
        "extractor": "generic",
        "extractor_key": "Generic",
        "webpage_url": format!("{origin}/page"),
        "url": format!("{origin}/media.mp4"),
        "ext": "mp4",
        "formats": [{
            "format_id": "0",
            "url": format!("{origin}/media.mp4"),
            "ext": "mp4",
            "impersonate": true,
        }],
        "subtitles": {"en": [{"ext": "vtt", "url": format!("{origin}/sub.vtt"), "impersonate": true}]},
    });
    std::fs::write(directory.path().join("info.json"), info.to_string()).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--load-info-json",
            "info.json",
            "--write-subs",
        ])
        .args(["--downloader", "curl", "-v", "-o", "o.%(ext)s"])
        .current_dir(directory.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let warning = "WARNING: The extractor specified to use impersonation for this download, but no impersonate target is available. If you encounter errors, then see  https://github.com/yt-dlp/yt-dlp#impersonation  for information on installing the required dependencies\n";
    assert_eq!(
        stderr(&output).matches(warning).count(),
        2,
        "{}",
        stderr(&output)
    );
    assert!(stderr(&output).contains(&format!(
        "[debug] Invoking http downloader on \"{origin}/media.mp4\""
    )));
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"media bytes"
    );
    assert!(directory.path().join("o.en.vtt").is_file());
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

    /// Without `--proxy`, impersonated requests follow upstream's
    /// environment routes like the plain client: `HTTPS_PROXY`, else the
    /// `http` route, else `ALL_PROXY` (bypass rules: `src/http/proxy.rs`).
    #[test]
    fn impersonated_requests_use_environment_proxies() {
        for variable in ["HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY"] {
            let (proxy, receiver) = proxy(&[b"h2", b"http/1.1"]);
            let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
            for name in ["HTTP", "HTTPS", "ALL", "NO"] {
                command
                    .env_remove(format!("{name}_PROXY"))
                    .env_remove(format!("{}_proxy", name.to_lowercase()));
            }
            let output = command
                .args(["--ignore-config", "--no-check-certificate", IMPERSONATED])
                .env(variable, &proxy)
                .output()
                .unwrap();
            let captured = receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("{variable}: {}", stderr(&output)));
            assert!(
                captured.cipher_suites.iter().any(|&suite| is_grease(suite)),
                "{variable}"
            );
            assert_eq!(captured.alpn.as_deref(), Some(&b"h2"[..]), "{variable}");
        }
    }

    /// A `--load-info-json` file holding one video with `format`.
    fn info_json(directory: &std::path::Path, format: serde_json::Value) -> String {
        let info = serde_json::json!({
            "id": "media",
            "title": "media",
            "extractor": "generic",
            "extractor_key": "Generic",
            "webpage_url": "https://page.test/",
            "url": format["url"],
            "ext": "mp4",
            "formats": [format],
        });
        let path = directory.join("info.json");
        std::fs::write(&path, info.to_string()).unwrap();
        path.display().to_string()
    }

    /// Media transfers impersonate `--impersonate`'s target (whole files,
    /// `--http-chunk-size` ranges and fragments), and a format's own
    /// `impersonate` field applies to HttpFD transfers but not fragments,
    /// like upstream. Each case's first TLS connection is captured.
    #[test]
    fn media_transfers_send_the_impersonated_fingerprint() {
        let file = serde_json::json!({
            "format_id": "0",
            "url": "https://media.test/v.mp4",
            "ext": "mp4",
        });
        let mut own = file.clone();
        own["impersonate"] = "chrome".into();
        let fragments = serde_json::json!({
            "format_id": "0",
            "url": "https://media.test/v.mpd",
            "ext": "mp4",
            "protocol": "http_dash_segments",
            "fragments": [{"url": "https://media.test/f1.m4s"}],
        });
        let mut own_fragments = fragments.clone();
        own_fragments["impersonate"] = "chrome".into();
        for (name, args, format, impersonated) in [
            ("file", &["--impersonate", "chrome"][..], &file, true),
            (
                "ranges",
                &["--impersonate", "chrome", "--http-chunk-size", "1M"][..],
                &file,
                true,
            ),
            (
                "fragments",
                &["--impersonate", "chrome"][..],
                &fragments,
                true,
            ),
            ("format field", &[][..], &own, true),
            ("format field, fragments", &[][..], &own_fragments, false),
            ("plain", &[][..], &file, false),
        ] {
            let directory = tempfile::tempdir().unwrap();
            let info = info_json(directory.path(), format.clone());
            let (proxy, receiver) = proxy(&[b"h2", b"http/1.1"]);
            let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
                .args([
                    "--ignore-config",
                    "--proxy",
                    &proxy,
                    "--no-check-certificate",
                ])
                .args(["-R", "0", "--fragment-retries", "0", "-o", "o.%(ext)s"])
                .args(args)
                .args(["--load-info-json", &info])
                .current_dir(directory.path())
                .output()
                .unwrap();
            let captured = receiver
                .recv_timeout(Duration::from_secs(10))
                .unwrap_or_else(|_| panic!("{name}: {}", stderr(&output)));
            assert_eq!(
                captured.cipher_suites.iter().any(|&suite| is_grease(suite)),
                impersonated,
                "{name}: {}",
                stderr(&output)
            );
            assert_eq!(
                captured.alpn.as_deref(),
                Some(if impersonated {
                    &b"h2"[..]
                } else {
                    &b"http/1.1"[..]
                }),
                "{name}"
            );
            // A target is available: no warning.
            assert!(!stderr(&output).contains("impersonation"), "{name}");
        }
    }

    /// A local media server: `/media.bin` (with `Range` support) and
    /// `/stall.bin` (64 KiB of a declared 1 MB, then nothing). Records each
    /// request head.
    struct Media {
        url: String,
        body: Arc<Vec<u8>>,
        heads: Arc<std::sync::Mutex<Vec<String>>>,
    }

    impl Media {
        fn new() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", listener.local_addr().unwrap());
            let body: Arc<Vec<u8>> = Arc::new(
                (0..1_500_000_u32)
                    .map(|value| (value % 251) as u8)
                    .collect(),
            );
            let heads = Arc::new(std::sync::Mutex::new(Vec::new()));
            let (served, recorded) = (body.clone(), heads.clone());
            thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    let (body, heads) = (served.clone(), recorded.clone());
                    thread::spawn(move || serve(stream, &body, &heads));
                }
            });
            Self { url, body, heads }
        }

        fn heads(&self) -> Vec<String> {
            self.heads.lock().unwrap().clone()
        }
    }

    fn serve(mut stream: TcpStream, body: &[u8], heads: &std::sync::Mutex<Vec<String>>) {
        let head = read_until(&mut stream, |data| {
            data.windows(4).any(|w| w == b"\r\n\r\n")
        });
        let head = String::from_utf8_lossy(&head).to_ascii_lowercase();
        heads.lock().unwrap().push(head.clone());
        let path = head.split_whitespace().nth(1).unwrap_or("/").to_owned();
        if path == "/stall.bin" {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 1000000\r\n\r\n",
            );
            let _ = stream.write_all(&body[..65536]);
            let _ = stream.flush();
            thread::sleep(Duration::from_secs(30));
            return;
        }
        let range = head
            .lines()
            .find_map(|line| line.strip_prefix("range: bytes="))
            .and_then(|range| range.trim().split_once('-'))
            .map(|(start, end)| {
                let start: usize = start.parse().unwrap();
                let end = end
                    .parse::<usize>()
                    .map_or(body.len() - 1, |end| end.min(body.len() - 1));
                (start, end)
            });
        let (status, extra, part) = match range {
            Some((start, end)) => (
                "206 Partial Content",
                format!("Content-Range: bytes {start}-{end}/{}\r\n", body.len()),
                &body[start..=end],
            ),
            None => ("200 OK", String::new(), body),
        };
        let _ = write!(
            stream,
            "HTTP/1.1 {status}\r\nContent-Type: video/mp4\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
            part.len()
        );
        let _ = stream.write_all(part);
    }

    /// Impersonated media bodies stream unchanged (whole and in ranges),
    /// with the browser's headers but HttpFD's `Accept-Encoding: identity`.
    #[test]
    fn impersonated_media_streams_with_identity_encoding() {
        let media = Media::new();
        for args in [&[][..], &["--http-chunk-size", "500K"][..]] {
            let directory = tempfile::tempdir().unwrap();
            let format = serde_json::json!({
                "format_id": "0",
                "url": format!("{}/media.bin", media.url),
                "ext": "mp4",
            });
            let info = info_json(directory.path(), format);
            let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
                .args([
                    "--ignore-config",
                    "--impersonate",
                    "chrome",
                    "-o",
                    "o.%(ext)s",
                ])
                .args(args)
                .args(["--load-info-json", &info])
                .current_dir(directory.path())
                .output()
                .unwrap();
            assert!(output.status.success(), "{}", stderr(&output));
            assert_eq!(
                std::fs::read(directory.path().join("o.mp4")).unwrap(),
                *media.body,
                "{args:?}"
            );
        }
        let heads = media.heads();
        // One whole-file request, then three or four ranges (upstream
        // randomizes each chunk to 95-100% of the size).
        assert!((4..=5).contains(&heads.len()), "{heads:?}");
        assert!(!heads[0].contains("\r\nrange: "), "{}", heads[0]);
        for head in &heads {
            assert!(head.contains("\r\nsec-ch-ua: "), "{head}");
            assert!(head.contains("\r\naccept-encoding: identity\r\n"), "{head}");
        }
        assert!(
            heads
                .iter()
                .skip(1)
                .all(|head| head.contains("\r\nrange: bytes="))
        );
    }

    /// SIGTERM ends an impersonated media body that stalls, within the
    /// interrupt poll, and keeps upstream's `.part` file with the bytes so far.
    #[test]
    fn a_signal_ends_a_stalled_impersonated_media_body() {
        let media = Media::new();
        let directory = tempfile::tempdir().unwrap();
        let format = serde_json::json!({
            "format_id": "0",
            "url": format!("{}/stall.bin", media.url),
            "ext": "mp4",
        });
        let info = info_json(directory.path(), format);
        let child = super::support::stoppable(&mut Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs")))
            .args([
                "--ignore-config",
                "--impersonate",
                "chrome",
                "-o",
                "o.%(ext)s",
            ])
            .args(["--load-info-json", &info])
            .current_dir(directory.path())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        let started = std::time::Instant::now();
        while media.heads().is_empty() {
            assert!(started.elapsed() < Duration::from_secs(30));
            thread::sleep(Duration::from_millis(20));
        }
        thread::sleep(Duration::from_millis(500));
        let sent = std::time::Instant::now();
        super::support::stop(&child);
        let output = child.wait_with_output().unwrap();
        assert!(
            sent.elapsed() < Duration::from_secs(3),
            "{:?}",
            sent.elapsed()
        );
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        assert!(
            stderr(&output).ends_with("ERROR: Interrupted by user\n"),
            "{}",
            stderr(&output)
        );
        let part = directory.path().join("o.mp4.part");
        assert_eq!(std::fs::read(part).unwrap(), media.body[..65536]);
    }

    /// SIGTERM ends an impersonated request whose TLS handshake stalls
    /// within the interrupt poll, not after the socket timeout.
    #[test]
    fn a_signal_ends_a_stalled_impersonated_request() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (accepted, connected) = mpsc::channel();
        thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = accepted.send(());
                thread::spawn(move || {
                    let _stream = stream;
                    thread::sleep(Duration::from_secs(60));
                });
            }
        });
        let directory = tempfile::tempdir().unwrap();
        let child = super::support::stoppable(&mut std::process::Command::new(env!(
            "CARGO_BIN_EXE_yt-dlp-rs"
        )))
        .args(["--ignore-config", "--impersonate", "chrome", "-s"])
        .arg(format!("https://127.0.0.1:{port}/page"))
        .current_dir(directory.path())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
        connected.recv_timeout(Duration::from_secs(30)).unwrap();
        thread::sleep(Duration::from_millis(300));
        let sent = std::time::Instant::now();
        super::support::stop(&child);
        let output = child.wait_with_output().unwrap();
        assert!(
            sent.elapsed() < Duration::from_secs(3),
            "{:?}",
            sent.elapsed()
        );
        assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
        assert!(
            stderr(&output).ends_with("ERROR: Interrupted by user\n"),
            "{}",
            stderr(&output)
        );
    }
}
