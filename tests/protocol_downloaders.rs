//! Upstream `PROTOCOL_MAP` downloaders for `rtmp`/`rtmpe` (RtmpFD running
//! `rtmpdump`), `rtmp_ffmpeg` and `--downloader rtmp:ffmpeg` (FFmpegFD's
//! `-rtmp_*` options) and `websocket_frag` (WebSocketFragmentFD feeding
//! FFmpeg's stdin), with recording fake executables and a local websocket
//! server.
#![cfg(unix)]
use std::{
    net::TcpListener,
    path::Path,
    process::{Command, Output},
    sync::{Arc, Mutex},
    thread,
};

/// Writes an executable shell script using only builtins (`PATH` holds
/// just the tools directory). Each run records its arguments in
/// `argsN.txt`.
fn script(directory: &Path, name: &str, body: &str) -> String {
    use std::os::unix::fs::PermissionsExt;
    let path = directory.join(name);
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\n{body}\
             dir=\"${{0%/*}}\"\n\
             n=0; if [ -f \"$dir/count\" ]; then read n < \"$dir/count\"; fi; n=$((n+1)); echo $n > \"$dir/count\"\n\
             printf '%s\\n' \"$@\" > \"$dir/args$n.txt\"\n"
        ) + match name {
            "rtmpdump" => {
                "prev=; out=\n\
                 for a in \"$@\"; do case \"$prev\" in -o) out=\"$a\";; esac; prev=\"$a\"; done\n\
                 printf 'RTMPDump v2.4\\r\\nConnecting ...\\n' >&2\n\
                 printf '512.000 kB / 1.00 sec (50.0%%)\\r1024.000 kB / 2.00 sec (99.9%%)\\r' >&2\n\
                 printf 'data' >> \"$out\"\n\
                 printf '\\nDownload complete\\n' >&2\n\
                 set -- ${FAKE_EXITS:-0}\n\
                 i=1; code=0; for c; do if [ $i = $n ]; then code=$c; fi; i=$((i+1)); done\n\
                 exit $code\n"
            }
            _ => {
                "for out; do :; done\n\
                 out=${out#file:}\n\
                 prev=; for a; do if [ \"$prev\" = -i ] && [ \"$a\" = - ]; then\n\
                 while IFS= read -r line; do printf '%s\\n' \"$line\"; done > \"$dir/stdin.txt\"; fi; prev=$a; done\n\
                 printf ffmpeg > \"$out\"\n"
            }
        },
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.display().to_string()
}

fn rtmpdump(directory: &Path) -> String {
    script(
        directory,
        "rtmpdump",
        "case \"$1\" in -h) echo 'RTMPDump v2.4'; exit 0;; esac\n",
    )
}

fn ffmpeg(directory: &Path) -> String {
    script(
        directory,
        "ffmpeg",
        "case \"$1\" in -version) echo 'ffmpeg version 7.0 Copyright'; exit 0;;\n\
         -bsfs) echo 'Bitstream filters:'; echo setts; exit 0;; esac\n",
    )
}

fn args(tools: &Path, run: usize) -> Vec<String> {
    std::fs::read_to_string(tools.join(format!("args{run}.txt")))
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn info(directory: &Path, fields: serde_json::Value) {
    let mut info = serde_json::json!({
        "id": "r", "title": "R", "ext": "flv", "url": "rtmp://127.0.0.1/app/play",
        "protocol": "rtmp", "webpage_url": "https://example.test/r",
        "extractor": "fixture", "extractor_key": "Fixture",
    });
    info.as_object_mut()
        .unwrap()
        .extend(fields.as_object().unwrap().clone());
    std::fs::write(directory.join("r.info.json"), info.to_string()).unwrap();
}

fn run(directory: &Path, tools: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "-o", "o.%(ext)s"])
        .args(args)
        .args(["--load-info-json", "r.info.json"])
        .current_dir(directory)
        .env("PATH", tools)
        .envs(env.iter().copied())
        .output()
        .unwrap()
}

const RTMP_FIELDS: &str = r#"{"player_url": "https://example.test/p.swf",
    "page_url": "https://example.test/r", "app": "app", "play_path": "mp4:play",
    "tc_url": "rtmp://127.0.0.1/app", "flash_version": "LNX 11",
    "rtmp_conn": ["S:a", "B:1"], "rtmp_protocol": "0", "rtmp_real_time": true}"#;

fn basic(extra: &[&str]) -> Vec<String> {
    let mut args: Vec<String> = [
        "--verbose",
        "-r",
        "rtmp://127.0.0.1/app/play",
        "-o",
        "o.flv.part",
        "--swfVfy",
        "https://example.test/p.swf",
        "--pageUrl",
        "https://example.test/r",
        "--app",
        "app",
        "--playpath",
        "mp4:play",
        "--tcUrl",
        "rtmp://127.0.0.1/app",
    ]
    .map(String::from)
    .into();
    args.extend(extra.iter().map(|arg| (*arg).to_owned()));
    args
}

const CONNECTION: [&str; 8] = [
    "--flashVer",
    "LNX 11",
    "--conn",
    "S:a",
    "--conn",
    "B:1",
    "--protocol",
    "0",
];

#[test]
fn rtmpdump_gets_upstream_arguments_and_reports_progress() {
    let tools = tempfile::tempdir().unwrap();
    rtmpdump(tools.path());
    let directory = tempfile::tempdir().unwrap();
    info(directory.path(), serde_json::from_str(RTMP_FIELDS).unwrap());
    let output = run(directory.path(), tools.path(), &["-v", "--newline"], &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}{stderr}");
    let mut expected = basic(&CONNECTION);
    expected.extend(["--realtime", "--resume", "--skip", "1"].map(String::from));
    assert_eq!(args(tools.path(), 1), expected);
    assert!(
        stderr.contains("[debug] Invoking rtmp downloader on \"rtmp://127.0.0.1/app/play\""),
        "{stderr}"
    );
    assert!(
        stderr.contains("[debug] rtmpdump command line: rtmpdump --verbose -r rtmp://127.0.0.1/app/play -o o.flv.part --swfVfy https://example.test/p.swf"),
        "{stderr}"
    );
    // Non-progress lines are shown with `--verbose`; `kB / sec (%)` lines
    // are progress with an estimated total.
    assert!(
        stdout.contains("[rtmpdump] RTMPDump v2.4\n[rtmpdump] Connecting ...\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("[download]  50.0% of ~   1.00MiB at "),
        "{stdout}"
    );
    // The cursor returns to a new line (an empty line with `--newline`).
    assert!(
        stdout.contains(
            "\n\n[rtmpdump] Download complete\n[rtmpdump] Downloaded 4 bytes\n[download] 100% of      4.00B in "
        ),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.flv")).unwrap(),
        b"data"
    );
}

#[test]
fn test_and_no_continue_change_the_arguments_and_no_connect_fails() {
    let tools = tempfile::tempdir().unwrap();
    rtmpdump(tools.path());
    let directory = tempfile::tempdir().unwrap();
    info(directory.path(), serde_json::from_str(RTMP_FIELDS).unwrap());
    let output = run(
        directory.path(),
        tools.path(),
        &["--test", "--no-continue", "--no-progress"],
        &[("FAKE_EXITS", "3")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stdout}{stderr}");
    let mut expected = basic(&["--stop", "1"]);
    expected.extend(CONNECTION.map(String::from));
    expected.push("--realtime".into());
    assert_eq!(args(tools.path(), 1), expected);
    assert!(
        stderr.contains("ERROR: [rtmpdump] Could not connect to RTMP server.\n"),
        "{stderr}"
    );
    assert!(!directory.path().join("o.flv").exists());
}

#[test]
fn a_failed_transfer_is_resumed_with_the_accumulated_arguments() {
    let tools = tempfile::tempdir().unwrap();
    rtmpdump(tools.path());
    let directory = tempfile::tempdir().unwrap();
    info(directory.path(), serde_json::json!({}));
    let output = run(
        directory.path(),
        tools.path(),
        &["--no-progress"],
        &[("FAKE_EXITS", "1 0")],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}{stderr}");
    let first = [
        "--verbose",
        "-r",
        "rtmp://127.0.0.1/app/play",
        "-o",
        "o.flv.part",
        "--resume",
        "--skip",
        "1",
    ];
    assert_eq!(args(tools.path(), 1), first);
    // Upstream's `args += [...]` extended `basic_args` itself.
    let mut second: Vec<&str> = first.into();
    second.extend(["--resume", "--skip", "1"]);
    assert_eq!(args(tools.path(), 2), second);
    assert!(
        stdout.contains(
            "[download] Destination: o.flv\n\n[rtmpdump] Downloaded 4 bytes\n\n[rtmpdump] Downloaded 8 bytes\n"
        ),
        "{stdout}"
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.flv")).unwrap(),
        b"datadata"
    );
}

#[test]
fn live_streams_are_not_resumed_and_exit_codes_are_reported() {
    let tools = tempfile::tempdir().unwrap();
    rtmpdump(tools.path());
    let directory = tempfile::tempdir().unwrap();
    info(
        directory.path(),
        serde_json::json!({"rtmp_live": true, "no_resume": true}),
    );
    let output = run(
        directory.path(),
        tools.path(),
        &["--no-progress"],
        &[("FAKE_EXITS", "1")],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert_eq!(
        args(tools.path(), 1),
        [
            "--verbose",
            "-r",
            "rtmp://127.0.0.1/app/play",
            "-o",
            "o.flv.part",
            "--live"
        ]
    );
    assert!(!tools.path().join("args2.txt").exists());
    assert!(
        stderr.contains("\n\nERROR: rtmpdump exited with code 1\n"),
        "{stderr:?}"
    );
}

#[test]
fn a_missing_rtmpdump_is_reported_like_upstream() {
    let tools = tempfile::tempdir().unwrap();
    let directory = tempfile::tempdir().unwrap();
    info(directory.path(), serde_json::json!({}));
    let output = run(directory.path(), tools.path(), &[], &[]);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stdout.contains("[download] Destination: o.flv\n"),
        "{stdout}"
    );
    assert!(
        stderr.contains(
            "ERROR: RTMP download detected but \"rtmpdump\" could not be run. Please install\n"
        ),
        "{stderr}"
    );
}

#[test]
fn ffmpeg_downloads_rtmp_with_its_rtmp_options() {
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = ffmpeg(tools.path());
    rtmpdump(tools.path());
    let directory = tempfile::tempdir().unwrap();
    let mut fields: serde_json::Value = serde_json::from_str(RTMP_FIELDS).unwrap();
    fields["rtmp_live"] = true.into();
    info(directory.path(), fields);
    let output = run(
        directory.path(),
        tools.path(),
        &[
            "-v",
            "--ffmpeg-location",
            &ffmpeg,
            "--downloader",
            "rtmp:ffmpeg",
            "--test",
        ],
        &[],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("[debug] Invoking ffmpeg downloader on \"rtmp://127.0.0.1/app/play\""),
        "{stderr}"
    );
    assert_eq!(
        args(tools.path(), 1),
        [
            "-y",
            "-loglevel",
            "verbose",
            "-rtmp_swfverify",
            "https://example.test/p.swf",
            "-rtmp_pageurl",
            "https://example.test/r",
            "-rtmp_app",
            "app",
            "-rtmp_playpath",
            "mp4:play",
            "-rtmp_tcurl",
            "rtmp://127.0.0.1/app",
            "-rtmp_flashver",
            "LNX 11",
            "-rtmp_live",
            "live",
            "-rtmp_conn",
            "S:a",
            "-rtmp_conn",
            "B:1",
            "-i",
            "rtmp://127.0.0.1/app/play",
            "-c",
            "copy",
            "-fs",
            "10241",
            "-f",
            "flv",
            "file:o.flv.part"
        ]
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.flv")).unwrap(),
        b"ffmpeg"
    );
    // FFmpegFD does not support `rtmpe`, which stays with RtmpFD.
    std::fs::remove_file(directory.path().join("o.flv")).unwrap();
    std::fs::remove_file(tools.path().join("count")).unwrap();
    info(directory.path(), serde_json::json!({"protocol": "rtmpe"}));
    let output = run(
        directory.path(),
        tools.path(),
        &["-v", "--ffmpeg-location", &ffmpeg, "--downloader", "ffmpeg"],
        &[],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stderr}");
    assert!(
        stderr.contains("[debug] Invoking rtmp downloader on"),
        "{stderr}"
    );
    assert_eq!(args(tools.path(), 1)[0], "--verbose");
}

#[test]
fn rtmp_ffmpeg_formats_use_ffmpeg_or_report_its_absence() {
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = ffmpeg(tools.path());
    let directory = tempfile::tempdir().unwrap();
    info(
        directory.path(),
        serde_json::json!({"protocol": "rtmp_ffmpeg", "app": "app"}),
    );
    let output = run(
        directory.path(),
        tools.path(),
        &["--ffmpeg-location", &ffmpeg],
        &[],
    );
    assert!(output.status.success());
    // Only the `rtmp` protocol gets the RTMP options and `-f flv`.
    assert_eq!(
        args(tools.path(), 1),
        [
            "-y",
            "-hide_banner",
            "-i",
            "rtmp://127.0.0.1/app/play",
            "-c",
            "copy",
            "-f",
            "flv",
            "file:o.flv.part"
        ]
    );
    std::fs::remove_file(directory.path().join("o.flv")).unwrap();
    let output = run(
        directory.path(),
        tools.path(),
        &["--ffmpeg-location", "/nonexistent/ffmpeg"],
        &[],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr.contains(
            "ERROR: m3u8 download detected but ffmpeg could not be found. Please install\n"
        ),
        "{stderr}"
    );
}

/// The recorded request path and headers.
type Recorded = Arc<Mutex<Vec<(String, String)>>>;

/// A websocket server: records the handshake headers, sends a binary and a
/// text message and closes.
// tungstenite's handshake callback returns its `ErrorResponse`.
#[allow(clippy::result_large_err)]
fn websocket_server() -> (String, Recorded) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let headers = Arc::new(Mutex::new(Vec::new()));
    let recorded = Arc::clone(&headers);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            let recorded = Arc::clone(&recorded);
            let callback = move |request: &tungstenite::handshake::server::Request, response| {
                let mut recorded = recorded.lock().unwrap();
                recorded.push(("path".into(), request.uri().to_string()));
                for (name, value) in request.headers() {
                    recorded.push((
                        name.to_string(),
                        value.to_str().unwrap_or_default().to_owned(),
                    ));
                }
                Ok(response)
            };
            let Ok(mut socket) = tungstenite::accept_hdr(stream, callback) else {
                continue;
            };
            let _ = socket.send(tungstenite::Message::binary(b"bin\n".to_vec()));
            let _ = socket.send(tungstenite::Message::text("text\n"));
            let _ = socket.close(None);
            while socket.read().is_ok() {}
        }
    });
    (format!("ws://{address}/live?x=1"), headers)
}

fn websocket_info(directory: &Path, url: &str, live: bool) {
    let mut info = serde_json::json!({
        "id": "w", "title": "W", "ext": "mp4", "url": url, "protocol": "websocket_frag",
        "webpage_url": "https://example.test/w", "extractor": "fixture",
        "extractor_key": "Fixture", "http_headers": {"User-Agent": "UA", "X-Test": "1"},
    });
    if live {
        info["live_status"] = "is_live".into();
    }
    std::fs::write(directory.join("r.info.json"), info.to_string()).unwrap();
}

#[test]
fn websocket_messages_are_piped_into_ffmpeg_and_fixed_up() {
    let (url, headers) = websocket_server();
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = ffmpeg(tools.path());
    let directory = tempfile::tempdir().unwrap();
    websocket_info(directory.path(), &url, true);
    let output = run(
        directory.path(),
        tools.path(),
        &["-v", "--ffmpeg-location", &ffmpeg],
        &[],
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "{stdout}{stderr}");
    assert!(
        stderr.contains(&format!(
            "[debug] Invoking web_socket_fragment downloader on \"{url}\""
        )),
        "{stderr}"
    );
    assert_eq!(
        args(tools.path(), 1),
        [
            "-y",
            "-loglevel",
            "verbose",
            "-i",
            "-",
            "-c",
            "copy",
            "-f",
            "mp4",
            "file:o.mp4.part"
        ]
    );
    assert_eq!(
        std::fs::read_to_string(tools.path().join("stdin.txt")).unwrap(),
        "bin\ntext\n"
    );
    // The handshake sends the format's headers (with the defaults).
    let headers = headers.lock().unwrap().clone();
    let header = |name: &str| {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };
    assert_eq!(header("path").as_deref(), Some("/live?x=1"));
    assert_eq!(header("user-agent").as_deref(), Some("UA"));
    assert_eq!(header("x-test").as_deref(), Some("1"));
    assert_eq!(header("sec-fetch-mode").as_deref(), Some("navigate"));
    assert!(header("cookie").is_none());
    // The connection's end completes a live download, then
    // `FFmpegFixupTimestampPP` (`setts`) and `FFmpegFixupDurationPP` run.
    assert!(
        stdout.contains("[ffmpeg] Interrupted by user\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("[FixupTimestamp] Fixing frame timestamp of \"o.mp4\"\n"),
        "{stdout}"
    );
    assert!(
        stdout.contains("[FixupDuration] Fixing video duration of \"o.mp4\"\n"),
        "{stdout}"
    );
    let timestamp = args(tools.path(), 2);
    let input = timestamp.iter().position(|arg| arg == "-i").unwrap();
    assert_eq!(
        &timestamp[input + 2..input + 13],
        [
            "-c",
            "copy",
            "-bsf",
            "setts=ts=TS-STARTPTS",
            "-map",
            "0",
            "-dn",
            "-ignore_unknown",
            "-ss",
            "0.001",
            "-f"
        ]
    );
    let duration = args(tools.path(), 3);
    let input = duration.iter().position(|arg| arg == "-i").unwrap();
    assert_eq!(
        &duration[input + 2..input + 8],
        ["-map", "0", "-dn", "-ignore_unknown", "-c", "copy"]
    );
    assert_eq!(
        std::fs::read(directory.path().join("o.mp4")).unwrap(),
        b"ffmpeg"
    );
}

#[test]
fn a_finished_websocket_interrupts_other_downloads_and_needs_ffmpeg() {
    let (url, _) = websocket_server();
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = ffmpeg(tools.path());
    let directory = tempfile::tempdir().unwrap();
    websocket_info(directory.path(), &url, false);
    let output = run(
        directory.path(),
        tools.path(),
        &["--ffmpeg-location", &ffmpeg],
        &[],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("ERROR: Interrupted by user\n"), "{stderr}");
    assert!(!directory.path().join("o.mp4").exists());
    let output = run(
        directory.path(),
        tools.path(),
        &["--ffmpeg-location", "/nonexistent/ffmpeg"],
        &[],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        stderr.contains(
            "ERROR: m3u8 download detected but ffmpeg could not be found. Please install\n"
        ),
        "{stderr}"
    );
}
