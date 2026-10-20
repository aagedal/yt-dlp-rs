//! `--compat-options`: upstream's alias expansion and the defaults it
//! changes (`set_compat_opts`), checked against pinned upstream.
use std::{
    io::{Read, Write},
    net::TcpListener,
    path::Path,
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
            let _ = stream.read(&mut request);
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: video/mp4\r\nContent-Length: 5\r\nConnection: close\r\n\r\nmedia",
            );
        }
    });
    origin
}

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("--ignore-config")
        .args(args)
        .current_dir(directory)
        .output()
        .unwrap()
}

fn compat_line(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr)
        .lines()
        .find(|line| line.starts_with("[debug] Compatibility options: "))
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn aliases_and_defaults_follow_set_compat_opts() {
    let origin = serve();
    let url = format!("{origin}/v.mp4");
    let directory = tempfile::tempdir().unwrap();
    // `filename` sets the default template unless -o gives one (then `*`).
    let output = run(
        directory.path(),
        &[
            "--compat-options",
            "filename",
            "-s",
            "--print",
            "filename",
            &url,
        ],
    );
    assert_eq!(String::from_utf8_lossy(&output.stdout), "v-v.mp4\n");
    let output = run(
        directory.path(),
        &[
            "--compat-options",
            "filename",
            "-o",
            "x.%(ext)s",
            "-s",
            "-v",
            &url,
        ],
    );
    assert_eq!(
        compat_line(&output),
        "[debug] Compatibility options: *filename"
    );
    // Year aliases chain; `-NAME` removes.
    let output = run(
        directory.path(),
        &["--compat-options", "2024", "-s", "-v", &url],
    );
    assert_eq!(
        compat_line(&output),
        "[debug] Compatibility options: mtime-by-default"
    );
    let output = run(
        directory.path(),
        &[
            "--compat-options",
            "all,-all,abort-on-error",
            "-s",
            "-v",
            &url,
        ],
    );
    assert_eq!(
        compat_line(&output),
        "[debug] Compatibility options: abort-on-error"
    );
    let output = run(directory.path(), &["--compat-options", "bogus", "-s", &url]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .ends_with("error: wrong OPTS for --compat-options: bogus\n")
    );
    // `no-clean-infojson` keeps private keys in written info JSON.
    let output = run(
        directory.path(),
        &[
            "--compat-options",
            "no-clean-infojson",
            "--write-info-json",
            "--skip-download",
            "-o",
            "o.%(ext)s",
            &url,
        ],
    );
    assert!(output.status.success());
    let json = std::fs::read_to_string(directory.path().join("o.info.json")).unwrap();
    assert!(json.contains("\"original_url\""), "{json}");
}
