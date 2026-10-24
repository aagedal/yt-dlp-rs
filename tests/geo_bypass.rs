//! Geo restriction bypass (`--xff` and its aliases) through the CLI: the
//! fake `X-Forwarded-For` header on extraction and download requests and in
//! the formats' `http_headers`, upstream's debug messages, and the
//! `GeoRestrictedError` retry with a fake IP (pinned upstream's output for
//! fixed blocks is compared in `tests/fixtures/sites`).
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Output},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

/// A local media server recording each request's `X-Forwarded-For`.
struct Server {
    url: String,
    seen: Arc<Mutex<Vec<Option<String>>>>,
    stop: Arc<AtomicBool>,
}

impl Server {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/clip.mp4", listener.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (record, done) = (seen.clone(), stop.clone());
        thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 2048];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&request).into_owned();
                let forwarded = text.lines().find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("x-forwarded-for")
                        .then(|| value.trim().to_owned())
                });
                record.lock().unwrap().push(forwarded);
                let body = b"fake mp4 media";
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body);
            }
        });
        Self { url, seen, stop }
    }

    fn forwarded(&self) -> Vec<Option<String>> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn run(args: &[&str], directory: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("--ignore-config")
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// The `-j` output's top-level and format `X-Forwarded-For` headers.
fn json_headers(output: &Output) -> (Option<String>, Option<String>) {
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let header =
        |headers: &serde_json::Value| headers["X-Forwarded-For"].as_str().map(str::to_owned);
    (
        header(&info["http_headers"]),
        header(&info["formats"][0]["http_headers"]),
    )
}

fn in_block(ip: &str, base: [u8; 4], prefix: u32) -> bool {
    let ip: std::net::Ipv4Addr = ip.parse().unwrap();
    let mask = u32::MAX << (32 - prefix);
    u32::from(ip) & mask == u32::from_be_bytes(base) & mask
}

#[test]
fn an_ip_block_fakes_every_request_and_the_format_headers() {
    let server = Server::start();
    let work = tempfile::tempdir().unwrap();
    let output = run(
        &[
            "-v",
            "--xff",
            "203.0.113.7/32",
            "--no-progress",
            "-o",
            "clip.%(ext)s",
            &server.url,
        ],
        work.path(),
    );
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stderr(&output);
    // `_initialize_geo_bypass`' IP-block debug line, before the extraction.
    assert!(
        text.contains("[debug] [generic] Using fake IP 203.0.113.7 as X-Forwarded-For\n"),
        "{text}"
    );
    // The webpage request and the download (the format's `http_headers`).
    let forwarded = server.forwarded();
    assert!(forwarded.len() >= 2, "{forwarded:?}");
    assert!(
        forwarded
            .iter()
            .all(|value| value.as_deref() == Some("203.0.113.7")),
        "{forwarded:?}"
    );
    assert_eq!(
        std::fs::read(work.path().join("clip.mp4")).unwrap(),
        b"fake mp4 media"
    );

    let output = run(
        &["--geo-bypass-ip-block", "203.0.113.8/32", "-j", &server.url],
        work.path(),
    );
    assert_eq!(
        json_headers(&output),
        (Some("203.0.113.8".into()), Some("203.0.113.8".into()))
    );
}

#[test]
fn a_country_code_fakes_an_address_from_its_block() {
    let server = Server::start();
    let work = tempfile::tempdir().unwrap();
    let output = run(&["-v", "--xff", "no", "-j", &server.url], work.path());
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stderr(&output);
    let line = text
        .lines()
        .find(|line| line.starts_with("[debug] Using fake IP "))
        .expect(&text);
    let ip = line
        .strip_prefix("[debug] Using fake IP ")
        .and_then(|rest| rest.strip_suffix(" (NO) as X-Forwarded-For"))
        .expect(line);
    // `GeoUtils._country_ip_map['NO']`: 84.208.0.0/13.
    assert!(in_block(ip, [84, 208, 0, 0], 13), "{ip}");
    assert_eq!(json_headers(&output), (Some(ip.into()), Some(ip.into())));
    assert_eq!(server.forwarded(), vec![Some(ip.to_owned())]);

    // An unknown code gives no address (`random_ipv4` returns `None`).
    let output = run(
        &["-v", "--geo-bypass-country", "ZZ", "-j", &server.url],
        work.path(),
    );
    assert!(output.status.success());
    assert!(stderr(&output).contains("[debug] Using fake IP None (ZZ) as X-Forwarded-For\n"));
    assert_eq!(json_headers(&output), (None, None));
}

#[test]
fn the_last_geo_option_wins_and_default_sends_nothing() {
    let server = Server::start();
    let work = tempfile::tempdir().unwrap();
    for (args, expected) in [
        (&[][..], None),
        (&["--xff", "default"][..], None),
        (&["--xff", "198.51.100.1/32", "--no-geo-bypass"][..], None),
        (&["--xff", "198.51.100.1/32", "--xff", "NEVER"][..], None),
        (&["--xff", "198.51.100.1/32", "--geo-bypass"][..], None),
        (
            &[
                "--no-geo-bypass",
                "--geo-bypass-ip-block",
                "198.51.100.2/32",
            ][..],
            Some("198.51.100.2"),
        ),
        (
            &["--geo-bypass-country", "DE", "--xff", "198.51.100.3/32"][..],
            Some("198.51.100.3"),
        ),
    ] {
        let mut full = args.to_vec();
        full.extend(["-v", "-j", server.url.as_str()]);
        let output = run(&full, work.path());
        assert!(output.status.success(), "{args:?}: {}", stderr(&output));
        assert_eq!(
            json_headers(&output),
            (expected.map(str::to_owned), expected.map(str::to_owned)),
            "{args:?}"
        );
        assert_eq!(
            stderr(&output).contains("Using fake IP"),
            expected.is_some(),
            "{args:?}"
        );
    }
    assert!(
        server
            .forwarded()
            .iter()
            .flatten()
            .all(|ip| ip.starts_with("198.51.100."))
    );
}

#[test]
fn unsupported_xff_values_are_usage_errors() {
    let work = tempfile::tempdir().unwrap();
    for value in ["foo", "1.2.3.4", "1.2.3.4/x", "256.0.0.0/8"] {
        for option in ["--xff", "--geo-bypass-country", "--geo-bypass-ip-block"] {
            let output = run(&[option, value, "https://example.invalid/"], work.path());
            assert_eq!(output.status.code(), Some(2), "{option} {value}");
            assert!(
                stderr(&output).ends_with(&format!("error: Unsupported --xff \"{value}\"\n")),
                "{option} {value}: {}",
                stderr(&output)
            );
        }
    }
}

#[test]
fn geo_restrictions_retry_with_a_fake_ip_from_their_countries() {
    // ArteTV's `GeoRestrictedError(countries=_COUNTRIES_MAP['DE_FR'])`.
    let proxy = site_proxy::SiteProxy::start("arte");
    let work = tempfile::tempdir().unwrap();
    let output = run(
        &[
            "--proxy",
            &proxy.url,
            "--no-check-certificate",
            "-j",
            "https://api.arte.tv/api/player/v2/config/de/LIVE",
        ],
        work.path(),
    );
    assert_eq!(output.status.code(), Some(1));
    let text = stderr(&output);
    let pattern = regex::Regex::new(
        r"^WARNING: \[ArteTV\] Video is geo restricted\. Retrying extraction with fake IP (\d+\.\d+\.\d+\.\d+) \(([A-Z]{2})\) as X-Forwarded-For\.\nERROR: \[ArteTV\] LIVE: Video restricted to 'DE_FR'\nThis video is available in Saint Barthélemy, Germany, France, French Guiana, Guadeloupe, Saint Martin \(French part\), Martinique, New Caledonia, French Polynesia, Saint Pierre and Miquelon, Réunion, Wallis and Futuna, Mayotte\.\nYou might want to use a VPN or a proxy server \(with --proxy\) to workaround\.\n$",
    )
    .unwrap();
    let found = pattern.captures(&text).expect(&text);
    let country = &found[2];
    assert!(
        [
            "BL", "DE", "FR", "GF", "GP", "MF", "MQ", "NC", "PF", "PM", "RE", "WF", "YT"
        ]
        .contains(&country),
        "{country}"
    );
    let _ip: std::net::Ipv4Addr = found[1].parse().unwrap();

    // No retry when bypassing is off, a country is given or an IP is set.
    for args in [
        &["--no-geo-bypass"][..],
        &["--xff", "DE"][..],
        &["--xff", "192.0.2.1/32"][..],
    ] {
        let mut full = vec!["--proxy", &proxy.url, "--no-check-certificate"];
        full.extend(args);
        full.extend(["-j", "https://api.arte.tv/api/player/v2/config/de/LIVE"]);
        let output = run(&full, work.path());
        assert_eq!(output.status.code(), Some(1));
        let text = stderr(&output);
        assert!(
            text.starts_with("ERROR: [ArteTV] LIVE: Video restricted to 'DE_FR'\n"),
            "{args:?}: {text}"
        );
    }
}
