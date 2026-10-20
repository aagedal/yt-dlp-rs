//! `-j` lines compared exactly with saved pinned upstream output
//! (`tools/capture_info_order.py`): upstream's key order and `None` keys for
//! each represented extractor path, over the same files and headers.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::PathBuf,
    process::Command,
    thread,
};

fn site() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/info_order/site")
}

fn fixture() -> serde_json::Value {
    serde_json::from_str(include_str!("fixtures/info_order/cases.json")).unwrap()
}

/// Serve the fixture site with the capture's `Content-Type` and
/// `Last-Modified` headers.
fn serve(fixture: &serde_json::Value) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let types = fixture["content_types"].as_object().unwrap().clone();
    let modified = fixture["last_modified"].as_str().unwrap().to_owned();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = Vec::new();
            let mut buffer = [0; 4096];
            while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                match stream.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(count) => request.extend_from_slice(&buffer[..count]),
                }
            }
            let request = String::from_utf8_lossy(&request);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let name = path.trim_start_matches('/').split('?').next().unwrap();
            let Ok(body) = std::fs::read(site().join(name))
                .map_err(|_| ())
                .and_then(|body| {
                    (!name.contains("..") && !name.contains('/'))
                        .then_some(body)
                        .ok_or(())
                })
            else {
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
                continue;
            };
            let extension = name.rfind('.').map_or("", |index| &name[index..]);
            let content_type = types
                .get(name)
                .or_else(|| types.get(extension))
                .and_then(serde_json::Value::as_str)
                .unwrap();
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nLast-Modified: {modified}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    origin
}

/// The capture's normalization: `{ORIGIN}`/`{HOST}`, no `epoch` (this port writes no
/// `_version`).
fn normalize(line: &str, origin: &str) -> String {
    let host = origin.split_once("//").unwrap().1;
    let line = line.replace(origin, "{ORIGIN}").replace(host, "{HOST}");
    let epoch = regex::Regex::new(r#""epoch": \d+, "#).unwrap();
    epoch.replace(&line, "").into_owned()
}

#[test]
fn json_lines_match_saved_upstream_key_order_and_none_keys() {
    let fixture = fixture();
    let origin = serve(&fixture);
    let directory = tempfile::tempdir().unwrap();
    let mut failures = Vec::new();
    for case in fixture["cases"].as_array().unwrap() {
        let name = case["name"].as_str().unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", "-j", "--user-agent", "fixture"])
            .arg(format!("{origin}/{name}"))
            .current_dir(directory.path())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let lines: Vec<String> = String::from_utf8(output.stdout)
            .unwrap()
            .lines()
            .map(|line| normalize(line, &origin))
            .collect();
        let expected: Vec<&str> = case["lines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap())
            .collect();
        if lines != expected {
            failures.push(format!(
                "{name}:\n  native   {lines:?}\n  upstream {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
