//! Upstream `--geo-verification-proxy`: requests sent with
//! `geo_verification_headers()` (`Ytdl-request-proxy`) use that proxy, which
//! overrides `--proxy`; other requests keep the normal routing.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{Arc, Mutex},
    thread,
};

/// A proxy that records each request line and refuses it.
fn recording_proxy() -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let lines = Arc::new(Mutex::new(Vec::new()));
    let record = lines.clone();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buffer = [0; 4096];
            let mut request = Vec::new();
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(length) => request.extend_from_slice(&buffer[..length]),
                }
            }
            let request = String::from_utf8_lossy(&request);
            record
                .lock()
                .unwrap()
                .push(request.lines().next().unwrap_or_default().to_owned());
            let _ = stream.write_all(
                b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            );
        }
    });
    (address, lines)
}

#[test]
fn geo_verification_requests_use_their_proxy() {
    let (geo, geo_lines) = recording_proxy();
    let (main, main_lines) = recording_proxy();
    // LegoIE's first request is the media player API with
    // `headers=self.geo_verification_headers()`.
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--proxy",
            &main,
            "--geo-verification-proxy",
            &geo,
            "--extractor-retries",
            "0",
            "-s",
            "https://www.lego.com/nl-nl/kids/videos/classic/creative-storytelling-the-little-puppy-aa24f27c7d5242bc86102ebdc0f24cba",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(!stderr.contains("not supported natively"), "{stderr}");
    let geo_lines = geo_lines.lock().unwrap().clone();
    assert!(
        geo_lines
            .iter()
            .any(|line| line.starts_with("CONNECT services.slingshot.lego.com:443 ")),
        "{geo_lines:?} {stderr}"
    );
    assert!(main_lines.lock().unwrap().is_empty());
}

/// Chaturbate's API and page requests send `geo_verification_headers()` and
/// attempt impersonation: with the `impersonate` feature, the impersonated
/// requests use the proxy too.
#[test]
fn impersonated_geo_verification_requests_use_their_proxy() {
    let (geo, geo_lines) = recording_proxy();
    let (main, main_lines) = recording_proxy();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--proxy",
            &main,
            "--geo-verification-proxy",
            &geo,
            "--extractor-retries",
            "0",
            "-s",
            "https://chaturbate.com/siswet19/",
        ])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    let geo_lines = geo_lines.lock().unwrap().clone();
    assert!(
        geo_lines
            .iter()
            .any(|line| line.starts_with("CONNECT chaturbate.com:443 ")),
        "{geo_lines:?} {stderr}"
    );
    assert!(main_lines.lock().unwrap().is_empty(), "{stderr}");
}

/// Each library client keeps its own `geo_verification_proxy` (also for
/// impersonated requests): building a second client does not reroute the
/// first one's requests.
#[test]
fn library_clients_keep_their_own_geo_verification_proxy() {
    use yt_dlp_rs::{HttpOptions, YoutubeDl, YoutubeOptions};
    let client = |main: &str, geo: &str| {
        let mut http = HttpOptions::default();
        http.proxy = Some(main.to_owned());
        http.geo_verification_proxy = Some(geo.to_owned());
        YoutubeDl::with_options(YoutubeOptions::default(), http).unwrap()
    };
    let (first_geo, first_geo_lines) = recording_proxy();
    let (first_main, first_main_lines) = recording_proxy();
    let (second_geo, second_geo_lines) = recording_proxy();
    let (second_main, second_main_lines) = recording_proxy();
    let first = client(&first_main, &first_geo);
    let second = client(&second_main, &second_geo);
    for (url, host) in [
        (
            "https://www.lego.com/nl-nl/kids/videos/classic/creative-storytelling-the-little-puppy-aa24f27c7d5242bc86102ebdc0f24cba",
            "services.slingshot.lego.com",
        ),
        ("https://chaturbate.com/siswet19/", "chaturbate.com"),
    ] {
        let error = first.extract_info(url).unwrap_err();
        let lines = first_geo_lines.lock().unwrap().clone();
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with(&format!("CONNECT {host}:443 "))),
            "{lines:?} {error}"
        );
    }
    assert!(first_main_lines.lock().unwrap().is_empty());
    assert!(second_geo_lines.lock().unwrap().is_empty());
    assert!(second_main_lines.lock().unwrap().is_empty());
    drop(second);
}
