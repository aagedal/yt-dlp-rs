//! Generic page metadata through the CLI: `Last-Modified` timestamps with the
//! derived `upload_date`, OpenGraph description/image and RTA age limits.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

struct Server {
    origin: String,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Server {
    fn new() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let worker = thread::spawn(move || {
            while !done.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
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
                while !request.windows(4).any(|v| v == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                let text = String::from_utf8_lossy(&request);
                let path = text.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let (mime, modified, body): (&str, &str, &[u8]) = match path.as_str() {
                    "/meta.html" => (
                        "text/html",
                        "Wed, 21 Oct 2015 07:28:00 GMT",
                        include_bytes!("fixtures/html5/meta.html"),
                    ),
                    "/plain.html" => (
                        "text/html",
                        "not a date",
                        b"<title>Plain</title><video poster='/p.jpg' src='/media/clip.mp4'></video>",
                    ),
                    "/media/clip.mp4" => ("video/mp4", "Sun, 06 Nov 1994 08:49:37 GMT", b"media"),
                    _ => ("text/plain", "", b"missing"),
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nLast-Modified: {modified}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(body);
            }
        });
        Self {
            origin,
            stop,
            worker: Some(worker),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.worker.take().unwrap().join().unwrap();
    }
}

fn json(url: &str) -> serde_json::Value {
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--proxy", "", "-j", url])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
fn page_and_direct_metadata_follow_upstream_generic_fields() {
    let server = Server::new();
    let page = json(&format!("{}/meta.html", server.origin));
    assert_eq!(page["timestamp"], 1445412480);
    assert_eq!(page["upload_date"], "20151021");
    assert_eq!(page["description"], "Cats & dogs");
    assert_eq!(page["age_limit"], 18);
    // og:image fills a missing poster verbatim (not resolved against the page).
    assert_eq!(page["thumbnail"], "images/og.jpg");
    let direct = json(&format!("{}/media/clip.mp4", server.origin));
    assert_eq!(direct["timestamp"], 784111777);
    assert_eq!(direct["upload_date"], "19941106");
    assert!(direct.get("description").is_none());
    // Unparsable dates are absent; posters win over og:image.
    let plain = json(&format!("{}/plain.html", server.origin));
    assert!(plain.get("timestamp").is_none() && plain.get("upload_date").is_none());
    assert_eq!(plain["thumbnail"], format!("{}/p.jpg", server.origin));
}
