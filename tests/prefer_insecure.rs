//! Upstream `--prefer-insecure`: `http_scheme()` for protocol-relative URLs.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    thread,
};

/// A page whose JSON-LD media and thumbnail are protocol-relative, and the
/// media itself, over plain HTTP until the test process ends.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let host = listener.local_addr().unwrap().to_string();
    let page = format!(
        "<html><head><title>ld</title><script type=\"application/ld+json\">{{\"@context\":\"https://schema.org\",\"@type\":\"VideoObject\",\"name\":\"LD clip\",\"contentUrl\":\"//{host}/a.mp4\",\"thumbnailUrl\":\"//{host}/t.jpg\"}}</script></head><body></body></html>"
    );
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
            let (mime, body) = if request.starts_with("GET /ld.html") {
                ("text/html", page.as_str())
            } else {
                ("video/mp4", "xx")
            };
            let _ = stream.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .as_bytes(),
            );
        }
    });
    host
}

#[test]
fn protocol_relative_urls_follow_the_http_scheme() {
    let host = serve();
    let page = format!("http://{host}/ld.html");
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(args)
            .arg(&page)
            .output()
            .unwrap()
    };
    let output = run(&["-J", "--prefer-insecure"]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("not supported natively"), "{stderr}");
    let info: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(info["url"], format!("http://{host}/a.mp4"));
    assert_eq!(info["thumbnail"], format!("http://{host}/t.jpg"));
    // By default upstream uses HTTPS, which this plain-HTTP server cannot
    // answer.
    let output = run(&["-J"]);
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("ERROR: [generic] ld: Unable to download webpage"),
        "{stderr}"
    );
}
