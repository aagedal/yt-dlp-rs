//! Upstream `--color` policies: styled `ERROR:`/`WARNING:` prefixes, format
//! tables and progress lines, compared with pinned upstream's bytes.
use std::{
    io::{Read, Write},
    net::TcpListener,
    process::{Command, Output},
    thread,
};

/// Serve `clip.mp4` (two bytes) until the test process ends.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut buffer = [0; 4096];
            let mut request = Vec::new();
            while !request.windows(4).any(|part| part == b"\r\n\r\n") {
                let length = stream.read(&mut buffer).unwrap();
                if length == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..length]);
            }
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 2\r\nConnection: close\r\n\r\nxx",
            );
        }
    });
    base
}

fn run(args: &[&str], directory: &std::path::Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(args)
        .current_dir(directory)
        .env_remove("NO_COLOR")
        .output()
        .unwrap()
}

#[test]
fn error_prefix_follows_the_stderr_policy() {
    let directory = tempfile::tempdir().unwrap();
    let url = "http://127.0.0.1:1/x.mp4";
    for (args, colored) in [
        (&["--color", "always"][..], true),
        (&["--color", "stderr:always"], true),
        (&["--color", "stdout:always"], false),
        // Output is not a terminal.
        (&[], false),
        (&["--color", "always", "--no-colors"], false),
        (&["--no-colors", "--color", "stdout,stderr:always"], true),
    ] {
        let output = run(&[args, &[url]].concat(), directory.path());
        let stderr = String::from_utf8_lossy(&output.stderr);
        let line = stderr.lines().last().unwrap();
        assert_eq!(
            line.starts_with("\x1b[0;31mERROR:\x1b[0m [generic] x: "),
            colored,
            "{args:?}: {line:?}"
        );
        assert_eq!(line.starts_with("ERROR: "), !colored, "{args:?}");
    }
    // Only the final values are validated, like upstream.
    let output = run(&["--color", "x", "--color", "never", url], directory.path());
    assert_ne!(output.status.code(), Some(2));
}

#[test]
fn format_table_and_progress_are_styled_like_upstream() {
    let base = serve();
    let directory = tempfile::tempdir().unwrap();
    let url = format!("{base}/clip.mp4");
    let output = run(&["--color", "always", "-F", &url], directory.path());
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Rendered by pinned upstream for the same direct link.
    let table = "\x1b[0;33mID\x1b[0m  \x1b[0;33mEXT\x1b[0m \x1b[0;33mRESOLUTION\x1b[0m \x1b[0;33m\x1b[0;34m\u{2502}\x1b[0;33m\x1b[0m \x1b[0;33mPROTO\x1b[0m \x1b[0;33m\x1b[0;34m\u{2502}\x1b[0;33m\x1b[0m \x1b[0;33mVCODEC\x1b[0m  \x1b[0;33mACODEC\x1b[0m";
    assert!(stdout.contains(table), "{stdout:?}");
    assert!(
        stdout.contains("\x1b[0;32mmp4\x1b[0m mp4 unknown    \x1b[0;34m\u{2502}\x1b[0m http  \x1b[0;34m\u{2502}\x1b[0m unknown unknown"),
        "{stdout:?}"
    );
    // `no_color`: terminal sequences without colors.
    let output = run(
        &["--color", "no_color", "-o", "out.%(ext)s", &url],
        directory.path(),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("\r\x1b[K[download] 100.0% of      2.00B at"),
        "{stdout:?}"
    );
    assert!(!stdout.contains("\x1b[0;"), "{stdout:?}");
}
