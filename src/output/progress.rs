//! Upstream `FileDownloader.report_progress` lines and the `minicurses`
//! status printers, without colors or terminal control sequences.

/// `format_decimal_suffix(num, '%.2f%sB', factor=1024)` (`format_bytes`).
pub fn format_bytes(bytes: Option<f64>) -> String {
    let Some(num) = bytes.filter(|num| *num >= 0.0) else {
        return "N/A".into();
    };
    const SUFFIXES: [&str; 9] = ["", "Ki", "Mi", "Gi", "Ti", "Pi", "Ei", "Zi", "Yi"];
    let exponent = if num == 0.0 {
        0
    } else {
        ((num.ln() / 1024f64.ln()) as i64).clamp(0, 8) as usize
    };
    let converted = num / 1024f64.powi(exponent as i32);
    format!("{converted:.2}{}B", SUFFIXES[exponent])
}

/// `format_seconds`: `HH:MM:SS`, `--:--:--` past 99 hours.
pub fn format_seconds(seconds: Option<f64>) -> String {
    let Some(seconds) = seconds else {
        return " Unknown".into();
    };
    let msec = seconds * 1000.0;
    let secs = (msec / 1000.0).floor();
    let mins = (secs / 60.0).floor();
    let hours = (mins / 60.0).floor();
    if hours > 99.0 {
        return "--:--:--".into();
    }
    format!(
        "{:02}:{:02}:{:02}",
        hours as i64,
        (mins - hours * 60.0) as i64,
        (secs - mins * 60.0) as i64
    )
}

/// `format_eta`: `format_seconds` without a leading `00:`, right-aligned.
pub fn format_eta(seconds: Option<f64>) -> String {
    let text = format_seconds(seconds);
    format!("{:>8}", text.strip_prefix("00:").unwrap_or(&text))
}

/// `format_speed`.
pub fn format_speed(speed: Option<f64>) -> String {
    match speed {
        None => " Unknown B/s".into(),
        Some(speed) => format!("{:>10}/s", format_bytes(Some(speed))),
    }
}

/// `format_percent`.
pub fn format_percent(percent: Option<f64>) -> String {
    match percent {
        None => "  N/A%".into(),
        Some(percent) => format!("{percent:>5.1}%"),
    }
}

/// Upstream `utils.progress.SmoothValue`.
#[derive(Debug, Clone, Copy)]
struct SmoothValue {
    initial: Option<f64>,
    smooth: Option<f64>,
    value: Option<f64>,
    smoothing: f64,
}

impl SmoothValue {
    fn new(initial: Option<f64>, smoothing: f64) -> Self {
        Self {
            initial,
            smooth: initial,
            value: initial,
            smoothing,
        }
    }

    fn set(&mut self, value: f64) {
        self.value = Some(value);
        self.smooth = Some(match self.smooth {
            None => value,
            Some(smooth) => (1.0 - self.smoothing) * value + self.smoothing * smooth,
        });
    }

    fn reset(&mut self) {
        self.value = self.initial;
        self.smooth = self.initial;
    }
}

/// Upstream `utils.progress.ProgressCalculator` (FragmentFD's progress), for
/// one downloading thread: sizes are per fragment, reset between them.
#[derive(Debug, Clone)]
pub struct ProgressCalculator {
    pub downloaded: u64,
    pub elapsed: f64,
    total: Option<f64>,
    start: std::time::Instant,
    last_update: f64,
    thread_size: u64,
    times: Vec<f64>,
    sizes: Vec<u64>,
    speed: SmoothValue,
    eta: SmoothValue,
}

impl ProgressCalculator {
    const SAMPLING_WINDOW: f64 = 3.0;
    const SAMPLING_RATE: f64 = 0.05;
    const GRACE_PERIOD: f64 = 1.0;

    pub fn new(initial: u64) -> Self {
        Self {
            downloaded: initial,
            elapsed: 0.0,
            total: Some(0.0),
            start: std::time::Instant::now(),
            last_update: 0.0,
            thread_size: 0,
            times: vec![0.0],
            sizes: vec![initial],
            speed: SmoothValue::new(Some(0.0), 0.7),
            eta: SmoothValue::new(None, 0.9),
        }
    }

    pub fn total(&self) -> Option<f64> {
        self.total
    }

    /// The `total` setter: never below what was downloaded.
    pub fn set_total(&mut self, value: Option<f64>) {
        self.total = match value {
            Some(value) if value < self.downloaded as f64 => Some(self.downloaded as f64),
            other => other,
        };
    }

    pub fn thread_reset(&mut self) {
        self.thread_size = 0;
    }

    pub fn speed(&self) -> Option<f64> {
        self.speed.smooth
    }

    pub fn eta(&self) -> Option<f64> {
        self.eta.smooth
    }

    /// `update(size)`: this thread's bytes so far.
    pub fn update(&mut self, size: u64) {
        if size == 0 {
            return;
        }
        let delta = size.saturating_sub(self.thread_size);
        self.thread_size = size;
        let now = self.start.elapsed().as_secs_f64();
        self.downloaded += delta;
        self.elapsed = now;
        if let Some(total) = self.total
            && self.downloaded as f64 > total
        {
            self.total = Some(self.downloaded as f64);
        }
        if self.last_update + Self::SAMPLING_RATE > now {
            return;
        }
        self.last_update = now;
        self.times.push(now);
        self.sizes.push(self.downloaded);
        let offset = self
            .times
            .partition_point(|time| *time < now - Self::SAMPLING_WINDOW);
        self.times.drain(..offset);
        self.sizes.drain(..offset);
        if self.times.len() < 2 {
            self.speed.reset();
            self.eta.reset();
            return;
        }
        let download_time = now - self.times[0];
        if download_time == 0.0 {
            return;
        }
        self.speed
            .set((self.downloaded - self.sizes[0]) as f64 / download_time);
        match (self.total, self.speed.value) {
            (Some(total), Some(speed))
                if total != 0.0 && speed != 0.0 && self.elapsed > Self::GRACE_PERIOD =>
            {
                self.eta.set((total - self.downloaded as f64) / speed);
            }
            _ => self.eta.reset(),
        }
    }
}

/// One `report_progress` status for a download.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProgressStatus {
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub total_bytes_estimate: Option<f64>,
    pub elapsed: Option<f64>,
    pub speed: Option<f64>,
    pub eta: Option<f64>,
    /// Completed fragments and their count, for fragmented downloads.
    pub fragment_index: Option<u32>,
    pub fragment_count: Option<u32>,
    /// Seconds of media in a growing file (`--growing-file`, a port extension).
    pub recorded: Option<f64>,
}

fn bytes(value: Option<f64>) -> String {
    format!("{:>10}", format_bytes(value))
}

/// Upstream `_format_progress`: a `ProgressStyles` field styled when the
/// progress stream (`_out_files.out`) allows colors.
fn styled(text: String, style: &str) -> String {
    use super::color;
    if color::out() == color::Allow::Yes {
        color::format_text(&text, style)
    } else {
        text
    }
}

/// The styled `_..._str` fields of a `downloading` status.
struct Fields {
    percent: String,
    total: String,
    estimate: String,
    downloaded: String,
    speed: String,
    eta: String,
    elapsed: String,
}

impl Fields {
    fn of(status: &ProgressStatus, percent: Option<f64>) -> Self {
        use super::color::progress_styles as style;
        Self {
            percent: styled(format_percent(percent), style::PERCENT),
            total: styled(
                bytes(status.total_bytes.map(|total| total as f64)),
                style::TOTAL_BYTES,
            ),
            estimate: styled(bytes(status.total_bytes_estimate), style::TOTAL_BYTES),
            downloaded: styled(
                bytes(Some(status.downloaded_bytes as f64)),
                style::DOWNLOADED_BYTES,
            ),
            speed: styled(format_speed(status.speed), style::SPEED),
            eta: styled(format_eta(status.eta).trim().to_owned(), style::ETA),
            elapsed: styled(format_seconds(status.elapsed), style::ELAPSED),
        }
    }
}

/// `report_progress`'s `progress`: by total, estimate, or a first empty read.
fn percent(status: &ProgressStatus) -> Option<f64> {
    let downloaded = status.downloaded_bytes as f64;
    status
        .total_bytes
        .map(|total| 100.0 * downloaded / total as f64)
        .or_else(|| {
            status
                .total_bytes_estimate
                .map(|total| 100.0 * downloaded / total)
        })
        .or_else(|| (status.downloaded_bytes == 0).then_some(0.0))
}

/// The `downloading` line (without the `[download] ` prefix).
pub fn downloading_line(status: &ProgressStatus) -> String {
    let Fields {
        percent,
        total,
        estimate,
        downloaded,
        speed,
        eta,
        elapsed,
    } = Fields::of(status, percent(status));
    let mut line = if status.total_bytes.is_some() {
        format!("{percent} of {total} at {speed} ETA {eta}")
    } else if status.total_bytes_estimate.is_some() {
        format!("{percent} of ~{estimate} at {speed} ETA {eta}")
    } else if status.elapsed.is_some() {
        format!("{downloaded} at {speed} ({elapsed})")
    } else {
        format!("{downloaded} at {speed}")
    };
    match (status.fragment_index, status.fragment_count) {
        (Some(index), Some(count)) => line.push_str(&format!(" (frag {index}/{count})")),
        (Some(index), None) => line.push_str(&format!(" (frag {index})")),
        _ => {}
    }
    if let Some(recorded) = status.recorded {
        line.push_str(&format!(", recorded {}", format_seconds(Some(recorded))));
    }
    line
}

/// Upstream `report_progress`'s `downloading` status dict (the
/// `progress` of `--progress-template`), with `_default_template`.
pub fn downloading_dict(status: &ProgressStatus, filename: Option<&str>) -> serde_json::Value {
    let percent = percent(status);
    let fields = Fields::of(status, percent);
    let mut dict = serde_json::json!({
        "status": "downloading",
        "downloaded_bytes": status.downloaded_bytes,
        "total_bytes": status.total_bytes,
        "tmpfilename": filename,
        "filename": filename,
        "eta": status.eta.map(|eta| eta as i64),
        "speed": status.speed,
        "elapsed": status.elapsed,
    });
    let object = dict.as_object_mut().expect("static object");
    if let Some(total) = status.total_bytes_estimate {
        object.insert("total_bytes_estimate".into(), total.into());
    }
    if let Some(index) = status.fragment_index {
        object.insert("fragment_index".into(), index.into());
    }
    if let Some(count) = status.fragment_count {
        object.insert("fragment_count".into(), count.into());
    }
    if let Some(recorded) = status.recorded {
        object.insert("recorded_seconds".into(), recorded.into());
        object.insert(
            "_recorded_str".into(),
            format_seconds(Some(recorded)).into(),
        );
    }
    for (key, value) in [("_eta_str", fields.eta), ("_speed_str", fields.speed)] {
        object.insert(key.into(), value.into());
    }
    object.insert("_percent".into(), percent.into());
    for (key, value) in [
        ("_percent_str", fields.percent),
        ("_total_bytes_str", fields.total),
        ("_total_bytes_estimate_str", fields.estimate),
        ("_downloaded_bytes_str", fields.downloaded),
        ("_elapsed_str", fields.elapsed),
        ("_default_template", downloading_line(status)),
    ] {
        object.insert(key.into(), value.into());
    }
    dict
}

/// Upstream `report_progress`'s `finished` status dict.
pub fn finished_dict(
    total_bytes: u64,
    elapsed: Option<f64>,
    filename: Option<&str>,
) -> serde_json::Value {
    let speed = elapsed
        .filter(|elapsed| *elapsed > 0.0)
        .map(|elapsed| total_bytes as f64 / elapsed);
    use super::color::progress_styles as style;
    serde_json::json!({
        "downloaded_bytes": total_bytes,
        "total_bytes": total_bytes,
        "filename": filename,
        "status": "finished",
        "elapsed": elapsed,
        "speed": speed,
        "_speed_str": styled(format_speed(speed).trim().to_owned(), style::SPEED),
        "_total_bytes_str": styled(bytes(Some(total_bytes as f64)), style::TOTAL_BYTES),
        "_elapsed_str": styled(format_seconds(elapsed), style::ELAPSED),
        "_percent": 100.0,
        "_percent_str": styled(format_percent(Some(100.0)), style::PERCENT),
        "_default_template": finished_line(total_bytes, elapsed),
    })
}

/// The `finished` line: `100% of SIZE in TIME at SPEED`.
pub fn finished_line(total_bytes: u64, elapsed: Option<f64>) -> String {
    let speed = elapsed
        .filter(|elapsed| *elapsed > 0.0)
        .map(|elapsed| total_bytes as f64 / elapsed);
    use super::color::progress_styles as style;
    let mut parts = vec![
        "100%".to_owned(),
        format!(
            "of {}",
            styled(bytes(Some(total_bytes as f64)), style::TOTAL_BYTES)
        ),
    ];
    if let Some(elapsed) = elapsed {
        parts.push(format!(
            "in {}",
            styled(format_seconds(Some(elapsed)), style::ELAPSED)
        ));
    }
    if let Some(speed) = speed {
        parts.push(format!(
            "at {}",
            styled(format_speed(Some(speed)).trim().to_owned(), style::SPEED)
        ));
    }
    parts.join(" ")
}

/// Upstream `MultilinePrinter` (one line) or `BreaklineStatusPrinter`
/// (`--newline`).
#[derive(Debug, Default)]
pub struct StatusPrinter {
    newline: bool,
    preserve: bool,
    /// `_HAVE_FULLCAP`: erase lines with terminal sequences instead of
    /// padding them.
    sequences: bool,
    last_length: usize,
    printed: bool,
}

const ERASE_LINE: &str = "\x1b[K";

impl StatusPrinter {
    /// `preserve` is upstream's `not quiet`: the last status stays visible.
    pub fn new(newline: bool, preserve: bool) -> Self {
        Self {
            newline,
            preserve,
            sequences: false,
            last_length: 0,
            printed: false,
        }
    }

    /// Use terminal sequences (`_HAVE_FULLCAP`, upstream's truthy
    /// `_allow_colors.out`, also with `no_color`).
    pub fn with_sequences(mut self, sequences: bool) -> Self {
        self.sequences = sequences;
        self
    }

    /// The text written for one status line.
    pub fn print(&mut self, text: &str) -> String {
        self.printed = true;
        if self.newline {
            return format!("{text}\n");
        }
        if self.sequences {
            return format!("\r{ERASE_LINE}{text}");
        }
        let length = text.chars().count();
        let padding = self.last_length.saturating_sub(length);
        self.last_length = length;
        format!("\r{text}{}", " ".repeat(padding))
    }

    /// The text written when the download ends.
    pub fn end(&mut self) -> String {
        if self.newline || !self.printed {
            return String::new();
        }
        self.printed = false;
        if self.sequences {
            // `_move_cursor(0)`, then a line break or an erased line.
            return if self.preserve {
                "\r\n".into()
            } else {
                format!("\r{ERASE_LINE}")
            };
        }
        if self.preserve {
            "\n".into()
        } else {
            let length = std::mem::take(&mut self.last_length);
            format!("\r{}\r", " ".repeat(length))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_upstream_progress_formatting_cases() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/progress/cases.json")).unwrap();
        for case in fixture["bytes"].as_array().unwrap() {
            assert_eq!(
                format_bytes(case["value"].as_f64()),
                case["text"].as_str().unwrap()
            );
        }
        for case in fixture["seconds"].as_array().unwrap() {
            let value = case["value"].as_f64();
            assert_eq!(format_seconds(value), case["seconds"].as_str().unwrap());
            assert_eq!(format_eta(value), case["eta"].as_str().unwrap());
        }
        for case in fixture["lines"].as_array().unwrap() {
            let get = |key: &str| case["status"][key].as_f64();
            let status = ProgressStatus {
                downloaded_bytes: case["status"]["downloaded_bytes"].as_u64().unwrap(),
                total_bytes: case["status"]["total_bytes"].as_u64(),
                total_bytes_estimate: get("total_bytes_estimate"),
                elapsed: get("elapsed"),
                speed: get("speed"),
                eta: get("eta"),
                fragment_index: case["status"]["fragment_index"].as_u64().map(|n| n as u32),
                fragment_count: case["status"]["fragment_count"].as_u64().map(|n| n as u32),
                recorded: None,
            };
            let line = if case["status"]["status"] == "finished" {
                finished_line(status.total_bytes.unwrap(), status.elapsed)
            } else {
                downloading_line(&status)
            };
            assert_eq!(line, case["line"].as_str().unwrap(), "{case}");
        }
    }

    #[test]
    fn growing_files_report_the_recorded_time() {
        let status = ProgressStatus {
            downloaded_bytes: 12_939_428,
            elapsed: Some(42.5),
            speed: Some(304_457.0),
            recorded: Some(65.4),
            ..Default::default()
        };
        assert_eq!(
            downloading_line(&status),
            "  12.34MiB at  297.32KiB/s (00:00:42), recorded 00:01:05"
        );
        let dict = downloading_dict(&status, Some("live.mov"));
        assert_eq!(dict["recorded_seconds"], 65.4);
        assert_eq!(dict["_recorded_str"], "00:01:05");
    }

    #[test]
    fn printers_follow_minicurses() {
        let mut printer = StatusPrinter::new(false, true);
        assert_eq!(
            printer.print("[download] long line"),
            "\r[download] long line"
        );
        assert_eq!(printer.print("[download] short"), "\r[download] short    ");
        assert_eq!(printer.end(), "\n");
        let mut quiet = StatusPrinter::new(false, false);
        quiet.print("abc");
        assert_eq!(quiet.end(), "\r   \r");
        let mut breakline = StatusPrinter::new(true, true);
        assert_eq!(breakline.print("x"), "x\n");
        assert_eq!(breakline.end(), "");
        // `_HAVE_FULLCAP`.
        let mut full = StatusPrinter::new(false, true).with_sequences(true);
        assert_eq!(full.print("long line"), "\r\x1b[Klong line");
        assert_eq!(full.print("short"), "\r\x1b[Kshort");
        assert_eq!(full.end(), "\r\n");
        let mut quiet = StatusPrinter::new(false, false).with_sequences(true);
        quiet.print("abc");
        assert_eq!(quiet.end(), "\r\x1b[K");
        let mut breakline = StatusPrinter::new(true, true).with_sequences(true);
        assert_eq!(breakline.print("x"), "x\n");
    }
}
