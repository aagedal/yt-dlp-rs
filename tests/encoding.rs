//! Upstream `--encoding`: messages are encoded with `errors='ignore'`.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Output},
    thread,
};

fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut request = [0; 4096];
            let read = stream.read(&mut request).unwrap_or(0);
            let request = String::from_utf8_lossy(&request[..read]);
            let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
            let (mime, body): (&str, Vec<u8>) = if path == "/p.html" {
                (
                    "text/html; charset=utf-8",
                    "<html><head><title>Ünï ☃ café</title></head><body><video src=\"a.mp4\"></video></body></html>".into(),
                )
            } else {
                ("video/mp4", b"\0\0\0\x18ftypisom media".to_vec())
            };
            let _ = write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(&body);
        }
    });
    format!("{origin}/p.html")
}

fn run(args: &[&str], url: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "-s"])
        .args(args)
        .arg(url)
        .output()
        .unwrap()
}

#[test]
fn messages_drop_characters_the_encoding_cannot_represent() {
    let url = serve();
    let output = run(&["--encoding", "latin-1", "--print", "title"], &url);
    assert_eq!(output.stdout, b"\xdcn\xef  caf\xe9 (1)\n");
    let output = run(&["--encoding", "ASCII", "--print", "title"], &url);
    assert_eq!(output.stdout, b"n  caf (1)\n");
    let output = run(&["--encoding", "utf-16-le", "--print", "id"], &url);
    assert_eq!(output.stdout, b"p\x00-\x001\x00\n\x00");
    // The debug header names the encoding and is itself written unencoded.
    let output = run(&["--encoding", "latin-1", "-v"], &url);
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("[debug] Encodings: locale utf-8, fs utf-8, pref latin-1, "),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn unimplemented_encodings_warn_and_keep_utf8() {
    let url = serve();
    let output = run(&["--encoding", "utf-16", "--print", "title"], &url);
    assert_eq!(String::from_utf8_lossy(&output.stdout), "Ünï ☃ café (1)\n");
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("WARNING: --encoding utf-16 is not supported natively; output is UTF-8")
    );
}
