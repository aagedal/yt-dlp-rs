//! Upstream `--bidi-workaround`: messages pass through `bidiv`/`fribidi`
//! on a pseudo-terminal; without either, upstream's warning.
#![cfg(unix)]
use std::{os::unix::fs::PermissionsExt, process::Command};

fn run(path: &std::path::Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--bidi-workaround",
            "--print",
            "%(id)s",
            "-s",
            "http://127.0.0.1:1/clip.mp4",
        ])
        .env("PATH", path)
        .env("COLUMNS", "40")
        .output()
        .unwrap()
}

#[test]
fn messages_pass_through_fribidi() {
    let tools = tempfile::tempdir().unwrap();
    let fribidi = tools.path().join("fribidi");
    // Echo the arguments once, then mark each line.
    std::fs::write(
        &fribidi,
        "#!/bin/sh\necho \"args:$*\" >&2\nwhile IFS= read -r line; do printf '<%s>\\n' \"$line\"; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&fribidi, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = format!("{}:/bin:/usr/bin", tools.path().display());
    let output = run(std::path::Path::new(&path));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("args:-c UTF-8 -w 40"), "{stderr}");
    // The pseudo-terminal writes `\r\n`; the final line break is the
    // message's own.
    assert!(
        stderr.contains("<ERROR: [generic] clip: Unable to download webpage"),
        "{stderr}"
    );
    assert!(stderr.contains(">\r\n"), "{stderr:?}");
}

#[test]
fn missing_program_is_upstream_warning() {
    let empty = tempfile::tempdir().unwrap();
    let output = run(empty.path());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with(
            "WARNING: Could not find fribidi executable, ignoring --bidi-workaround. Make sure that  fribidi  is an executable file in one of the directories in your $PATH.\n"
        ),
        "{stderr}"
    );
    assert!(stderr.contains("\nERROR: [generic] clip: "), "{stderr}");
}
