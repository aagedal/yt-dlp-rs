//! The CLI output contract (docs/CLI_CONTRACT.md) against saved pinned
//! upstream output: `--progress-template` statuses and their keys,
//! `--print`/`--print-to-file` stages with `-j`/`-J`/`--print-json`, quiet
//! and simulation, exit codes and a SIGINT during a download. The Rust twin
//! of `tools/capture_cli_contract.py`: the same routes from a local server,
//! the same fake FFmpeg and the same normalization, compared exactly.
//! The port's own SIGTERM and `--keep-partial-on-stop` behavior (no
//! upstream equivalent) is tested by hand below.
#![cfg(unix)]
use serde_json::Value;
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    net::TcpListener,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/cli_contract");

/// Upstream's `FAKE_FFMPEG` in tools/capture_cli_contract.py.
const FAKE_FFMPEG: &str = "#!/bin/sh\n\
    case \"$1\" in -version|-bsfs) echo 'ffmpeg version 7.1-contract'; exit 0;; esac\n\
    for a; do out=${a#file:}; done\n\
    : > \"$out\"\n\
    prev=\n\
    for a; do [ \"$prev\" = -i ] && cat \"${a#file:}\" >> \"$out\"; prev=$a; done\n\
    exit 0\n";

/// A route's bytes: `text`, or `size` bytes of `fill`.
fn body(route: &Value) -> Vec<u8> {
    match route["text"].as_str() {
        Some(text) => text.as_bytes().to_vec(),
        None => vec![
            route["fill"].as_u64().unwrap_or(0) as u8;
            route["size"].as_u64().unwrap() as usize
        ],
    }
}

/// A local server for the fixture's routes. `stalled` is set once a
/// `stall_after` route stops sending; it holds the connection until the
/// server is dropped.
struct Server {
    base: String,
    stalled: Arc<(Mutex<bool>, Condvar)>,
    stop: Arc<AtomicBool>,
}

impl Server {
    fn start(routes: &serde_json::Map<String, Value>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let routes = Arc::new(routes.clone());
        let stalled = Arc::new((Mutex::new(false), Condvar::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let (flag, done) = (stalled.clone(), stop.clone());
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (routes, flag, done) = (routes.clone(), flag.clone(), done.clone());
                thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap_or(0);
                    let path = line.split_whitespace().nth(1).unwrap_or("/");
                    let path = path.split('?').next().unwrap().to_owned();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0
                            || header.trim().is_empty()
                        {
                            break;
                        }
                    }
                    let Some(route) = routes.get(&path) else {
                        let _ = stream.write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                        return;
                    };
                    let data = body(route);
                    let length = route["length"].as_u64().unwrap_or(data.len() as u64);
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n",
                        route["type"].as_str().unwrap()
                    );
                    if stream.write_all(head.as_bytes()).is_err() {
                        return;
                    }
                    let chunk = route["chunk"]
                        .as_u64()
                        .map_or(data.len().max(1), |n| n as usize);
                    let delay = Duration::from_millis(route["delay_ms"].as_u64().unwrap_or(0));
                    let stall = route["stall_after"].as_u64();
                    let mut sent = 0;
                    for part in data.chunks(chunk) {
                        if stream
                            .write_all(part)
                            .and_then(|()| stream.flush())
                            .is_err()
                        {
                            return;
                        }
                        sent += chunk as u64;
                        if stall.is_some_and(|stall| sent >= stall) {
                            let (stalled, wake) = &*flag;
                            *stalled.lock().unwrap() = true;
                            wake.notify_all();
                            while !done.load(Ordering::Relaxed) {
                                thread::sleep(Duration::from_millis(20));
                            }
                            return;
                        }
                        thread::sleep(delay);
                    }
                });
            }
        });
        Self {
            base,
            stalled,
            stop,
        }
    }

    /// Wait until a `stall_after` route stalls.
    fn wait_stalled(&self) {
        let (stalled, wake) = &*self.stalled;
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut guard = stalled.lock().unwrap();
        while !*guard {
            assert!(Instant::now() < deadline, "the download never stalled");
            guard = wake
                .wait_timeout(guard, Duration::from_millis(100))
                .unwrap()
                .0;
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// Upstream `normalize`: replacements, the program name, JSON objects as
/// their top-level keys (without `_version`), consecutive duplicates once.
fn normalize(text: &str, replacements: &[(String, &str)]) -> Vec<String> {
    let mut text = text.to_owned();
    for (old, new) in replacements {
        text = text.replace(old.as_str(), new);
    }
    let text = text.replace("yt-dlp-rs", "yt-dlp");
    let mut lines: Vec<String> = Vec::new();
    for line in text.split('\n') {
        let line = match serde_json::from_str::<Value>(line) {
            Ok(Value::Object(object)) => format!(
                "JSON [{}]",
                object
                    .keys()
                    .filter(|key| *key != "_version")
                    .map(|key| serde_json::to_string(key).unwrap())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            _ => line.to_owned(),
        };
        if lines.last() != Some(&line) {
            lines.push(line);
        }
    }
    lines
}

fn run_case(case: &Value, routes: &serde_json::Map<String, Value>) -> Value {
    let server = Server::start(routes);
    let work = tempfile::tempdir().unwrap();
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = tools.path().join("ffmpeg");
    std::fs::write(&ffmpeg, FAKE_FFMPEG).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    for (name, content) in case["files"].as_object().into_iter().flatten() {
        std::fs::write(
            work.path().join(name),
            content.as_str().unwrap().replace("BASE", &server.base),
        )
        .unwrap();
    }
    let args: Vec<String> = case["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|arg| {
            arg.as_str()
                .unwrap()
                .replace("BASE", &server.base)
                .replace("FFMPEG", &ffmpeg.to_string_lossy())
        })
        .collect();
    use std::os::unix::process::CommandExt;
    // A SIGINT ignored on entry (a background job) would stay ignored.
    let child = Command::new("perl")
        .args(["-e", "$SIG{INT} = 'DEFAULT'; exec @ARGV or die"])
        .arg(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("--ignore-config")
        .args(&args)
        .current_dir(work.path())
        .process_group(0)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    if let Some(signal) = case["signal"].as_str() {
        server.wait_stalled();
        // The client reads the data sent before the stall first.
        thread::sleep(Duration::from_secs(1));
        let name = signal.strip_prefix("SIG").unwrap();
        let status = Command::new("kill")
            .args([&format!("-{name}"), "--", &format!("-{}", child.id())])
            .status()
            .unwrap();
        assert!(status.success());
    }
    let output = child.wait_with_output().unwrap();
    let base = server.base.clone();
    drop(server);
    let real = |path: &Path| {
        std::fs::canonicalize(path)
            .unwrap()
            .to_string_lossy()
            .into_owned()
    };
    let replacements = [
        (real(work.path()), "DIR"),
        (work.path().to_string_lossy().into_owned(), "DIR"),
        (real(tools.path()), "TOOLS"),
        (tools.path().to_string_lossy().into_owned(), "TOOLS"),
        (base, "BASE"),
    ];
    let mut files: Vec<String> = std::fs::read_dir(work.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    let mut result = serde_json::json!({
        "name": case["name"],
        "args": case["args"],
        "code": output.status.code(),
        "stdout": normalize(&String::from_utf8_lossy(&output.stdout), &replacements),
        "stderr": normalize(&String::from_utf8_lossy(&output.stderr), &replacements),
        "files": files,
    });
    if let Some(read) = case["read"].as_array() {
        let contents: serde_json::Map<String, Value> = read
            .iter()
            .map(|name| {
                let name = name.as_str().unwrap();
                let text = std::fs::read_to_string(work.path().join(name)).unwrap_or_default();
                (name.to_owned(), normalize(&text, &replacements).into())
            })
            .collect();
        result["contents"] = contents.into();
    }
    result
}

#[test]
fn the_cli_contract_matches_pinned_upstream() {
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{FIXTURE}/cases.json")).unwrap())
            .unwrap();
    let expected: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{FIXTURE}/expected.json")).unwrap())
            .unwrap();
    let routes = fixture["routes"].as_object().unwrap();
    let cases = fixture["cases"].as_array().unwrap();
    let saved: BTreeMap<&str, &Value> = expected["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| (case["name"].as_str().unwrap(), case))
        .collect();
    for case in cases {
        let name = case["name"].as_str().unwrap();
        assert!(
            saved
                .get(name)
                .is_some_and(|saved| saved["args"] == case["args"]),
            "{name}: no saved upstream output for these arguments (tools/capture_cli_contract.py --write)"
        );
    }
    let failures: Vec<String> = thread::scope(|scope| {
        let handles: Vec<_> = cases
            .iter()
            .map(|case| scope.spawn(move || (case, run_case(case, routes))))
            .collect();
        handles
            .into_iter()
            .filter_map(|handle| {
                let (case, actual) = handle.join().unwrap();
                let name = case["name"].as_str().unwrap();
                let wanted = saved[name];
                (actual != *wanted).then(|| {
                    format!(
                        "{name}:\n  upstream: {}\n  port:     {}",
                        serde_json::to_string(wanted).unwrap(),
                        serde_json::to_string(&actual).unwrap()
                    )
                })
            })
            .collect()
    });
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A case from the fixture with other arguments and signal.
fn variant(name: &str, signal: &str, extra: &[&str]) -> (Value, Value) {
    let fixture: Value =
        serde_json::from_str(&std::fs::read_to_string(format!("{FIXTURE}/cases.json")).unwrap())
            .unwrap();
    let mut case = fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .find(|case| case["name"] == name)
        .unwrap()
        .clone();
    case["signal"] = signal.into();
    // A second video after the stopped one, which is never processed.
    let slow: Value =
        serde_json::from_str(case["files"]["slow.info.json"].as_str().unwrap()).unwrap();
    let mut later = slow.clone();
    later["id"] = "later".into();
    later["formats"][0]["url"] = "BASE/a.mp4".into();
    case["files"]["slow.info.json"] = serde_json::json!([slow, later]).to_string().into();
    let mut args: Vec<Value> = extra.iter().map(|arg| (*arg).into()).collect();
    args.extend(case["args"].as_array().unwrap().iter().cloned());
    case["args"] = args.into();
    (case, fixture["routes"].clone())
}

fn lines(values: &Value) -> Vec<&str> {
    values
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line.as_str().unwrap())
        .collect()
}

/// Upstream has no SIGTERM handler (Python dies of the signal); the port
/// treats it like SIGINT, so a consumer's Stop sees the same result.
#[test]
fn sigterm_ends_a_download_like_sigint() {
    let (case, routes) = variant("sigint-download", "SIGTERM", &[]);
    let result = run_case(&case, routes.as_object().unwrap());
    assert_eq!(result["code"], 1, "{result}");
    assert_eq!(
        lines(&result["stdout"]),
        [
            "before_dl Slow [slow].mp4",
            "downloading|Slow [slow].mp4.part",
            ""
        ]
    );
    assert_eq!(
        lines(&result["stderr"]),
        ["", "ERROR: Interrupted by user", ""]
    );
    assert_eq!(
        lines(&result["files"]),
        ["Slow [slow].mp4.part", "slow.info.json"]
    );
}

/// `--keep-partial-on-stop` (port extension): the received bytes become
/// the final file, `after_move` prints it, and the run still ends with
/// "Interrupted by user" (exit 1) without processing later videos.
#[test]
fn a_stop_with_keep_partial_publishes_and_prints_the_file() {
    for signal in ["SIGINT", "SIGTERM"] {
        let (case, routes) = variant("sigint-download", signal, &["--keep-partial-on-stop"]);
        let result = run_case(&case, routes.as_object().unwrap());
        assert_eq!(result["code"], 1, "{signal}: {result}");
        assert_eq!(
            lines(&result["stdout"]),
            [
                "before_dl Slow [slow].mp4",
                "downloading|Slow [slow].mp4.part",
                "DIR/Slow [slow].mp4",
                ""
            ],
            "{signal}"
        );
        assert_eq!(
            lines(&result["stderr"]),
            ["", "ERROR: Interrupted by user", ""],
            "{signal}"
        );
        assert_eq!(
            lines(&result["files"]),
            ["Slow [slow].mp4", "slow.info.json"],
            "{signal}"
        );
    }
}
