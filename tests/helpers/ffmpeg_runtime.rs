//! Portable process fixture for the bounded FFmpeg runner.
use std::{io::Write, path::Path, time::Duration};
fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let mode = std::fs::read_to_string("mode").unwrap();
    for (name, _) in std::env::vars_os() {
        assert!(
            name == "SystemRoot" || name == "WINDIR",
            "ambient environment leaked: {name:?}"
        );
    }
    if args == ["-version"] {
        if mode == "version-hang" {
            hang();
        }
        if mode == "bad-version" {
            println!("unrelated tool");
            return;
        }
        println!("ffmpeg version test-fixture");
        return;
    }
    // Match the runner's canonical paths, including Windows verbatim prefixes.
    let directory = std::env::current_dir().unwrap().canonicalize().unwrap();
    let expected: Vec<std::ffi::OsString> = [
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-f",
        "mov",
        "-protocol_whitelist",
        "file",
        "-i",
    ]
    .into_iter()
    .map(Into::into)
    .chain(std::iter::once(
        directory.join("video.mp4").into_os_string(),
    ))
    .chain(
        ["-f", "mov", "-protocol_whitelist", "file", "-i"]
            .into_iter()
            .map(Into::into),
    )
    .chain(std::iter::once(
        directory.join("audio.m4a").into_os_string(),
    ))
    .chain(
        ["-map", "0:v:0", "-map", "1:a:0", "-c", "copy", "-f", "mp4"]
            .into_iter()
            .map(Into::into),
    )
    .chain(std::iter::once(
        directory.join("output.mp4").into_os_string(),
    ))
    .collect();
    assert_eq!(args, expected);
    // Stdin must be closed rather than a potentially blocking writer pipe.
    assert_eq!(
        std::io::Read::read(&mut std::io::stdin(), &mut [0]).unwrap(),
        0
    );
    match mode.as_str() {
        "stderr" => eprintln!("fixture diagnostic"),
        "exit" => std::process::exit(3),
        "empty" => {
            std::fs::write("output.mp4", []).unwrap();
        }
        "missing" => {}
        "stdout-flood" => flood(std::io::stdout()),
        "stderr-flood" => flood(std::io::stderr()),
        "hang" => hang(),
        "success" => std::fs::write(Path::new("output.mp4"), b"merged").unwrap(),
        _ => panic!("unknown fixture mode"),
    }
}
fn hang() {
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}
fn flood(mut pipe: impl Write) {
    loop {
        if pipe.write_all(&[b'x'; 8192]).is_err() {
            return;
        }
    }
}
