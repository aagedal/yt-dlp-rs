//! Upstream `RtmpFD` (`rtmp`/`rtmpe` protocols): `rtmpdump` writes the
//! temporary file, with its `kB / sec (percent)` stderr lines as progress
//! and the resume loop for interrupted transfers. The format's RTMP fields
//! (`player_url`, `page_url`, `app`, `play_path`, `tc_url`, `flash_version`,
//! `rtmp_live`, `rtmp_conn`) are also FFmpegFD's `-rtmp_*` options.
use crate::{
    DownloadOptions, DownloadPhase, DownloadProgress, Error, PartFiles, Result, VideoInfo,
};
use serde_json::{Map, Value};
use std::{
    ffi::OsString,
    io::Read,
    ops::ControlFlow,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{LazyLock, mpsc},
    time::Duration,
};

/// Python `str()` of an info-dict value, `None` for a missing or `null` one.
fn py_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        Value::Bool(true) => Some("True".into()),
        Value::Bool(false) => Some("False".into()),
        other => Some(other.to_string()),
    }
}

/// Python truthiness of an info-dict value.
fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64().is_some_and(|number| number != 0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(map)) => !map.is_empty(),
    }
}

/// The RTMP connection fields of a format.
pub(crate) struct Fields {
    pub player_url: Option<String>,
    pub page_url: Option<String>,
    pub app: Option<String>,
    pub play_path: Option<String>,
    pub tc_url: Option<String>,
    pub flash_version: Option<String>,
    pub live: bool,
    /// `rtmp_conn`: a list of entries, or one string.
    pub conn: Vec<String>,
}

impl Fields {
    pub(crate) fn from_extra(extra: &Map<String, Value>) -> Self {
        let text = |key: &str| py_text(extra.get(key));
        let conn = match extra.get("rtmp_conn") {
            Some(Value::Array(entries)) => entries
                .iter()
                .filter_map(|entry| py_text(Some(entry)))
                .collect(),
            Some(Value::String(entry)) => vec![entry.clone()],
            _ => Vec::new(),
        };
        Fields {
            player_url: text("player_url"),
            page_url: text("page_url"),
            app: text("app"),
            play_path: text("play_path"),
            tc_url: text("tc_url"),
            flash_version: text("flash_version"),
            live: truthy(extra.get("rtmp_live")),
            conn,
        }
    }

    /// FFmpegFD's options for an `rtmp` input.
    pub(crate) fn ffmpeg_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        for (option, value) in [
            ("-rtmp_swfverify", &self.player_url),
            ("-rtmp_pageurl", &self.page_url),
            ("-rtmp_app", &self.app),
            ("-rtmp_playpath", &self.play_path),
            ("-rtmp_tcurl", &self.tc_url),
            ("-rtmp_flashver", &self.flash_version),
        ] {
            if let Some(value) = value {
                args.extend([option.to_owned(), value.clone()]);
            }
        }
        if self.live {
            args.extend(["-rtmp_live".into(), "live".into()]);
        }
        for entry in &self.conn {
            args.extend(["-rtmp_conn".into(), entry.clone()]);
        }
        args
    }
}

/// rtmpdump's exit codes.
const RD_SUCCESS: i32 = 0;
const RD_FAILED: i32 = 1;
const RD_INCOMPLETE: i32 = 2;
const RD_NO_CONNECT: i32 = 3;

/// The parameters RtmpFD reads.
pub(crate) struct Params<'a> {
    pub options: &'a DownloadOptions,
    /// `continuedl`.
    pub continue_download: bool,
    /// `--test`.
    pub test: bool,
}

/// One progress line of rtmpdump's stderr: the downloaded bytes and the
/// percentage (`None` for live streams).
fn parse_progress(line: &str) -> Option<(u64, Option<f64>)> {
    static WITH_PERCENT: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"([0-9]+\.[0-9]{3}) kB / [0-9]+\.[0-9]{2} sec \(([0-9]{1,2}\.[0-9])%\)")
            .unwrap()
    });
    static WITHOUT: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"([0-9]+\.[0-9]{3}) kB / [0-9]+\.[0-9]{2} sec").unwrap()
    });
    let bytes = |text: &str| (text.parse::<f64>().unwrap_or(0.0) * 1024.0) as u64;
    if let Some(captures) = WITH_PERCENT.captures(line) {
        return Some((bytes(&captures[1]), captures[2].parse().ok()));
    }
    WITHOUT
        .captures(line)
        .map(|captures| (bytes(&captures[1]), None))
}

fn progress(
    phase: DownloadPhase,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
) -> DownloadProgress {
    DownloadProgress {
        attempt: 1,
        phase,
        downloaded_bytes,
        total_bytes,
        total_bytes_estimate: None,
        stream_index: None,
        stream_count: None,
        fragment_index: None,
        fragment_count: None,
        fragment: None,
        recorded_ms: None,
    }
}

/// How one rtmpdump run ended.
enum Run {
    Exited(i32),
    /// SIGINT during a live download (`KeyboardInterrupt`).
    Interrupted,
}

/// `run_rtmpdump(args)`: progress from stderr (`\r`/`\n`-separated lines),
/// other lines as `[rtmpdump] LINE` with `--verbose`.
fn run_rtmpdump(
    args: &[String],
    interrupt: Option<&crate::interrupt::Guard>,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<Run> {
    let mut child = Command::new(&args[0])
        .args(&args[1..])
        .stdin(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| Error::Reported(format!("rtmpdump could not be run: {error}")))?;
    let mut stderr = child.stderr.take().expect("piped stderr");
    let (sender, lines) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let mut line: Vec<u8> = Vec::new();
        let mut byte = [0u8];
        loop {
            let end = !matches!(stderr.read(&mut byte), Ok(1));
            if end || matches!(byte[0], b'\r' | b'\n') {
                if !line.is_empty() {
                    // `char.decode('ascii', 'replace')`.
                    let text: String = line
                        .iter()
                        .map(|&b| if b.is_ascii() { b as char } else { '\u{FFFD}' })
                        .collect();
                    if sender.send(text).is_err() {
                        return;
                    }
                    line.clear();
                }
                if end {
                    return;
                }
            } else {
                line.push(byte[0]);
            }
        }
    });
    let mut cursor_in_new_line = true;
    let mut interrupted = false;
    loop {
        match lines.recv_timeout(Duration::from_millis(50)) {
            Ok(line) => match parse_progress(&line) {
                Some((downloaded, percent)) => {
                    let mut status = progress(DownloadPhase::Downloading, downloaded, None);
                    status.total_bytes_estimate = percent
                        .filter(|percent| *percent > 0.0)
                        .map(|percent| (downloaded as f64 * 100.0 / percent) as u64);
                    if callback(status).is_break() {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(Error::DownloadCancelled);
                    }
                    cursor_in_new_line = false;
                }
                None if crate::screen::verbose() => {
                    if !cursor_in_new_line {
                        crate::screen::screen("");
                    }
                    cursor_in_new_line = true;
                    crate::screen::screen(&format!("[rtmpdump] {line}"));
                }
                None => {}
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if interrupt.is_some_and(crate::interrupt::Guard::interrupted) {
            interrupted = true;
            break;
        }
        if callback(progress(DownloadPhase::Downloading, 0, None)).is_break() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::DownloadCancelled);
        }
    }
    if interrupted {
        // `proc.kill()` and the live download's `KeyboardInterrupt` handling.
        let _ = child.kill();
        let _ = child.wait();
        return Ok(Run::Interrupted);
    }
    if !cursor_in_new_line {
        crate::screen::screen("");
    }
    let status = child.wait()?;
    Ok(Run::Exited(status.code().unwrap_or(-1)))
}

/// `RtmpFD.real_download`.
pub(crate) fn download(
    params: &Params<'_>,
    info: &VideoInfo,
    destination: &Path,
    callback: &mut dyn FnMut(DownloadProgress) -> ControlFlow<()>,
) -> Result<u64> {
    let options = params.options;
    crate::download::check_destination(destination, options)?;
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent)?;
    let fields = Fields::from_extra(&info.extra);
    let protocol = py_text(info.extra.get("rtmp_protocol"));
    let real_time = truthy(info.extra.get("rtmp_real_time"));
    let no_resume = truthy(info.extra.get("no_resume"));
    let continue_dl = params.continue_download;
    let test = params.test;
    // `report_destination`.
    if callback(progress(DownloadPhase::Downloading, 0, None)).is_break() {
        return Err(Error::DownloadCancelled);
    }
    let mut private = None;
    let temporary: PathBuf = match options.part_files {
        PartFiles::NoPart { .. } => destination.to_owned(),
        PartFiles::Part { .. } => {
            let mut name = destination.as_os_str().to_owned();
            name.push(".part");
            name.into()
        }
        PartFiles::Private => {
            let file = tempfile::Builder::new()
                .prefix(&crate::scratch::prefix(".yt-dlp-rs-", parent))
                .suffix(".part")
                .tempfile_in(parent)?;
            let path = file.path().to_owned();
            private = Some(file);
            path
        }
    };
    // `check_executable('rtmpdump', ['-h'])`.
    if Command::new("rtmpdump")
        .arg("-h")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .is_err()
    {
        return Err(Error::Reported(
            "RTMP download detected but \"rtmpdump\" could not be run. Please install".into(),
        ));
    }
    let mut basic_args: Vec<String> = vec![
        "rtmpdump".into(),
        "--verbose".into(),
        "-r".into(),
        info.url.to_string(),
        "-o".into(),
        temporary.display().to_string(),
    ];
    let mut option = |name: &str, value: &Option<String>| {
        if let Some(value) = value {
            basic_args.extend([name.to_owned(), value.clone()]);
        }
    };
    option("--swfVfy", &fields.player_url);
    option("--pageUrl", &fields.page_url);
    option("--app", &fields.app);
    option("--playpath", &fields.play_path);
    option("--tcUrl", &fields.tc_url);
    if test {
        basic_args.extend(["--stop".into(), "1".into()]);
    }
    if let Some(flash_version) = &fields.flash_version {
        basic_args.extend(["--flashVer".into(), flash_version.clone()]);
    }
    if fields.live {
        basic_args.push("--live".into());
    }
    for entry in &fields.conn {
        basic_args.extend(["--conn".into(), entry.clone()]);
    }
    if let Some(protocol) = &protocol {
        basic_args.extend(["--protocol".into(), protocol.clone()]);
    }
    if real_time {
        basic_args.push("--realtime".into());
    }
    // Upstream's `args = basic_args; args += [...]` extends `basic_args`
    // itself, so the resume loop's commands repeat these options.
    if !no_resume && continue_dl && !fields.live {
        basic_args.push("--resume".into());
    }
    if !fields.live && continue_dl {
        basic_args.extend(["--skip".into(), "1".into()]);
    }
    let debug_args: Vec<OsString> = basic_args[1..].iter().map(OsString::from).collect();
    crate::screen::debug_named_command("rtmpdump", &debug_args);
    let fail = |temporary: &Path, private: bool| {
        if private {
            let _ = std::fs::remove_file(temporary);
        }
    };
    // Upstream `real_download`: interrupting a live download completes it.
    let interrupt = crate::is_live(info).then(crate::interrupt::Guard::install);
    let mut retval = match run_rtmpdump(&basic_args, interrupt.as_ref(), callback)? {
        Run::Exited(code) => code,
        Run::Interrupted => {
            crate::screen::screen("\n[rtmpdump] Interrupted by user");
            RD_SUCCESS
        }
    };
    if retval == RD_NO_CONNECT {
        fail(&temporary, private.is_some());
        return Err(Error::Reported(
            "[rtmpdump] Could not connect to RTMP server.".into(),
        ));
    }
    let size = |path: &Path| std::fs::metadata(path).map(|meta| meta.len());
    while matches!(retval, RD_INCOMPLETE | RD_FAILED) && !test && !fields.live {
        let previous = size(&temporary)?;
        crate::screen::screen(&format!("[rtmpdump] Downloaded {previous} bytes"));
        // "This seems to be needed".
        std::thread::sleep(Duration::from_secs(5));
        let mut args = basic_args.clone();
        args.push("--resume".into());
        if retval == RD_FAILED {
            args.extend(["--skip".into(), "1".into()]);
        }
        retval = match run_rtmpdump(&args, interrupt.as_ref(), callback)? {
            Run::Exited(code) => code,
            Run::Interrupted => return Err(Error::DownloadCancelled),
        };
        let current = size(&temporary)?;
        if previous == current && retval == RD_FAILED {
            break;
        }
        // "Some rtmp streams seem abort after ~ 99.8%. Don't complain for those".
        if previous == current && retval == RD_INCOMPLETE && current > 1024 {
            crate::screen::screen(
                "[rtmpdump] Could not download the whole video. This can happen for some advertisements.",
            );
            retval = RD_SUCCESS;
            break;
        }
    }
    drop(interrupt);
    if retval == RD_SUCCESS || (test && retval == RD_INCOMPLETE) {
        let bytes = size(&temporary)?;
        crate::screen::screen(&format!("[rtmpdump] Downloaded {bytes} bytes"));
        match private {
            Some(file) => crate::download::persist(file, destination, options.overwrite)?,
            None if temporary != destination => std::fs::rename(&temporary, destination)?,
            None => {}
        }
        if callback(progress(DownloadPhase::Completed, bytes, Some(bytes))).is_break() {
            return Err(Error::DownloadCancelled);
        }
        return Ok(bytes);
    }
    fail(&temporary, private.is_some());
    crate::output::encoding::write_message(true, "\n\n");
    let _ = callback(progress(DownloadPhase::Failed, 0, None));
    Err(Error::Reported(format!(
        "rtmpdump exited with code {retval}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn progress_lines_follow_upstream_patterns() {
        assert_eq!(
            parse_progress("512.000 kB / 1.00 sec (50.0%)"),
            Some((524288, Some(50.0)))
        );
        // Live streams have no percentage.
        assert_eq!(parse_progress("1.500 kB / 3.25 sec"), Some((1536, None)));
        assert_eq!(parse_progress("Connecting ..."), None);
    }

    #[test]
    fn ffmpeg_options_follow_ffmpeg_fd() {
        let extra = json!({"player_url": "https://e.test/p.swf", "app": "a", "rtmp_live": true,
            "rtmp_conn": "S:x", "flash_version": null});
        let fields = Fields::from_extra(extra.as_object().unwrap());
        assert_eq!(
            fields.ffmpeg_args(),
            [
                "-rtmp_swfverify",
                "https://e.test/p.swf",
                "-rtmp_app",
                "a",
                "-rtmp_live",
                "live",
                "-rtmp_conn",
                "S:x"
            ]
        );
    }
}
