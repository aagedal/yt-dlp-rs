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
}

fn bytes(value: Option<f64>) -> String {
    format!("{:>10}", format_bytes(value))
}

/// The `downloading` line (without the `[download] ` prefix).
pub fn downloading_line(status: &ProgressStatus) -> String {
    let downloaded = status.downloaded_bytes as f64;
    let percent = status
        .total_bytes
        .map(|total| 100.0 * downloaded / total as f64)
        .or_else(|| {
            status
                .total_bytes_estimate
                .map(|total| 100.0 * downloaded / total)
        })
        .or_else(|| (status.downloaded_bytes == 0).then_some(0.0));
    let percent = format_percent(percent);
    let speed = format_speed(status.speed);
    let eta = format_eta(status.eta);
    let eta = eta.trim();
    let mut line = if let Some(total) = status.total_bytes {
        format!(
            "{percent} of {} at {speed} ETA {eta}",
            bytes(Some(total as f64))
        )
    } else if let Some(estimate) = status.total_bytes_estimate {
        format!(
            "{percent} of ~{} at {speed} ETA {eta}",
            bytes(Some(estimate))
        )
    } else if let Some(elapsed) = status.elapsed {
        format!(
            "{} at {speed} ({})",
            bytes(Some(downloaded)),
            format_seconds(Some(elapsed))
        )
    } else {
        format!("{} at {speed}", bytes(Some(downloaded)))
    };
    match (status.fragment_index, status.fragment_count) {
        (Some(index), Some(count)) => line.push_str(&format!(" (frag {index}/{count})")),
        (Some(index), None) => line.push_str(&format!(" (frag {index})")),
        _ => {}
    }
    line
}

/// The `finished` line: `100% of SIZE in TIME at SPEED`.
pub fn finished_line(total_bytes: u64, elapsed: Option<f64>) -> String {
    let speed = elapsed
        .filter(|elapsed| *elapsed > 0.0)
        .map(|elapsed| total_bytes as f64 / elapsed);
    let mut parts = vec![
        "100%".to_owned(),
        format!("of {}", bytes(Some(total_bytes as f64))),
    ];
    if let Some(elapsed) = elapsed {
        parts.push(format!("in {}", format_seconds(Some(elapsed))));
    }
    if let Some(speed) = speed {
        parts.push(format!("at {}", format_speed(Some(speed)).trim()));
    }
    parts.join(" ")
}

/// Upstream `MultilinePrinter` (one line, no terminal sequences) or
/// `BreaklineStatusPrinter` (`--newline`).
#[derive(Debug, Default)]
pub struct StatusPrinter {
    newline: bool,
    preserve: bool,
    last_length: usize,
    printed: bool,
}

impl StatusPrinter {
    /// `preserve` is upstream's `not quiet`: the last status stays visible.
    pub fn new(newline: bool, preserve: bool) -> Self {
        Self {
            newline,
            preserve,
            last_length: 0,
            printed: false,
        }
    }

    /// The text written for one status line.
    pub fn print(&mut self, text: &str) -> String {
        self.printed = true;
        if self.newline {
            return format!("{text}\n");
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
    }
}
