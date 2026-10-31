//! `-o -` streams media to stdout while it downloads (a player reading the
//! pipe starts at once), not after the transfer finishes.
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

const SIZE: usize = 64 * 1024;

/// A server sending `/slow.mp4` as a 188-byte packet every 50 ms (about 17 s).
fn server() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            thread::spawn(move || {
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim().is_empty() {
                        break;
                    }
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: {SIZE}\r\nConnection: close\r\n\r\n"
                );
                if stream.write_all(head.as_bytes()).is_err() {
                    return;
                }
                for chunk in vec![0x47; SIZE].chunks(188) {
                    if stream
                        .write_all(chunk)
                        .and_then(|()| stream.flush())
                        .is_err()
                    {
                        return;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            });
        }
    });
    port
}

#[test]
fn stdout_receives_media_during_the_transfer() {
    let port = server();
    let mut child = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "-o", "-"])
        .arg(format!("http://127.0.0.1:{port}/slow.mp4"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    let mut stdout = child.stdout.take().unwrap();
    let mut buffer = [0; 4096];
    let mut received = 0;
    while received < 4096 {
        let count = stdout.read(&mut buffer).unwrap();
        assert!(count > 0, "stdout ended after {received} bytes");
        assert!(buffer[..count].iter().all(|&byte| byte == 0x47));
        received += count;
    }
    // The whole transfer takes about 17 s.
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "{:?}",
        started.elapsed()
    );
    child.kill().unwrap();
    child.wait().unwrap();
}
