//! Pure human-readable format inventory; never selects or inspects resources.
use crate::{StreamKind, VideoInfo};

const CELL_LIMIT: usize = 128;
const HEADER_LIMIT: usize = 256;
const COLUMNS: [&str; 9] = [
    "ID",
    "EXT",
    "RESOLUTION",
    "TYPE",
    "FILESIZE",
    "TBR",
    "ABR",
    "LANGUAGE",
    "NOTE",
];

/// Render all declared formats, or one non-selectable implicit direct-source row.
/// Counts and sizes are metadata, never inferred from fragments or manifest text.
/// Terminal controls are visible escapes; Unicode width uses character counts.
/// Output has a trailing newline. Resource-URL and encryption fields are not
/// rendered; the function has no network effects.
pub fn render_formats(info: &VideoInfo) -> String {
    let headers = &COLUMNS;
    let mut rows: Vec<[String; 9]> = Vec::new();
    if info.formats.is_empty() {
        rows.push([
            "-".into(),
            safe(&info.ext, CELL_LIMIT),
            resolution(info.stream_kind, info.width, info.height),
            kind(info.stream_kind).into(),
            size(info.filesize),
            "-".into(),
            "-".into(),
            "-".into(),
            "-".into(),
        ]);
    } else {
        for format in &info.formats {
            let mut note = match (format.audio_default, format.audio_autoselect) {
                (true, true) => "[default, autoselect]".to_owned(),
                (true, false) => "[default]".to_owned(),
                (false, true) => "[autoselect]".to_owned(),
                _ => String::new(),
            };
            if let Some(value) = &format.format_note {
                if !note.is_empty() && !value.is_empty() {
                    note.push(' ');
                }
                // Enough input to detect truncation, without copying a large note.
                note.extend(value.chars().take(CELL_LIMIT + 1));
            }
            rows.push([
                safe(&format.format_id, CELL_LIMIT),
                safe(&format.ext, CELL_LIMIT),
                resolution(format.stream_kind, format.width, format.height),
                kind(format.stream_kind).into(),
                size(format.filesize),
                rate(format.tbr),
                rate(format.abr),
                safe(format.language.as_deref().unwrap_or("-"), CELL_LIMIT),
                safe(&note, CELL_LIMIT),
            ]);
        }
    }
    let mut widths: Vec<_> = headers.iter().map(|value| value.chars().count()).collect();
    for row in &rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let prefix = "[info] Available formats for ";
    let mut output = format!(
        "{prefix}{}:\n",
        safe(&info.id, HEADER_LIMIT - prefix.chars().count() - 1)
    );
    line(&mut output, headers.iter().copied(), &widths);
    for row in &rows {
        line(&mut output, row.iter().map(String::as_str), &widths);
    }
    output
}
fn line<'a>(output: &mut String, cells: impl Iterator<Item = &'a str>, widths: &[usize]) {
    for (index, cell) in cells.enumerate() {
        output.push_str(cell);
        if index + 1 < widths.len() {
            output.extend(std::iter::repeat_n(
                ' ',
                widths[index] - cell.chars().count() + 2,
            ));
        }
    }
    output.push('\n');
}
fn kind(kind: StreamKind) -> &'static str {
    match kind {
        StreamKind::Standalone => "standalone",
        StreamKind::AudioOnly => "audio only",
        StreamKind::VideoOnly => "video only",
    }
}
fn resolution(kind: StreamKind, width: Option<u32>, height: Option<u32>) -> String {
    if kind == StreamKind::AudioOnly {
        return "audio only".into();
    }
    match (width, height) {
        (Some(width), Some(height)) => format!("{width}x{height}"),
        (None, Some(height)) => format!("{height}p"),
        (Some(width), None) => format!("{width}x?"),
        _ => "-".into(),
    }
}
fn size(value: Option<u64>) -> String {
    value
        .map(|value| format!("{value}B"))
        .unwrap_or_else(|| "-".into())
}
fn rate(value: Option<f64>) -> String {
    let Some(value) = value.filter(|v| v.is_finite() && *v >= 0.0) else {
        return "-".into();
    };
    let value = if value == 0.0 { 0.0 } else { value };
    let text = value.to_string();
    if text.len() > 24 {
        format!("{value:e}k")
    } else {
        format!("{text}k")
    }
}
fn safe(value: &str, limit: usize) -> String {
    if value.is_empty() {
        return "-".into();
    }
    let mut tokens = Vec::new();
    let mut length = 0usize;
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        let token = if c.is_control()
            || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{2028}' | '\u{2029}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            if (c as u32) <= 255 {
                format!("\\x{:02x}", c as u32)
            } else {
                format!("\\u{{{:x}}}", c as u32)
            }
        } else {
            c.to_string()
        };
        let count = token.chars().count();
        if length + count > limit {
            while length >= limit {
                let token: String = tokens.pop().expect("bounded nonempty output");
                length -= token.chars().count();
            }
            tokens.push("…".into());
            break;
        }
        length += count;
        tokens.push(token);
        if length == limit && chars.peek().is_some() {
            tokens.pop().expect("bounded nonempty output");
            tokens.push("…".into());
            break;
        }
    }
    tokens.concat()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::VideoFormat;
    use serde_json::json;
    fn info() -> VideoInfo {
        serde_json::from_value(json!({"id":"sample", "title":"Sample", "url":"https://secret.example/direct.mp4", "webpage_url":"https://secret.example/watch", "ext":"mp4", "extractor":"test", "extractor_key":"Test", "direct":false})).unwrap()
    }
    fn format(id: &str, kind: StreamKind) -> VideoFormat {
        serde_json::from_value(json!({"format_id":id, "url":"https://secret.example/source", "ext":"mp4", "stream_kind":kind})).unwrap()
    }
    #[test]
    fn implicit_direct_source_has_no_invented_selectable_id_or_estimates() {
        let unknown = render_formats(&info());
        assert!(unknown.starts_with("[info] Available formats for sample:\n"));
        assert_eq!(unknown.lines().count(), 3);
        assert_eq!(
            unknown
                .lines()
                .last()
                .unwrap()
                .split_whitespace()
                .collect::<Vec<_>>(),
            ["-", "mp4", "-", "standalone", "-", "-", "-", "-", "-"]
        );
        assert!(!unknown.contains("https://"));
        let mut direct = info();
        direct.filesize = Some(1024);
        direct.width = Some(1280);
        direct.height = Some(720);
        direct.format_id = Some("0".into());
        let known = render_formats(&direct);
        assert!(known.contains("1280x720"));
        assert!(known.contains("1024B"));
        assert!(known.lines().last().unwrap().starts_with('-'));
        assert!(!known.contains("~"));
    }
    #[test]
    fn adaptive_inventory_preserves_ids_order_types_rates_languages_and_notes() {
        let mut info = info();
        let mut video = format("137", StreamKind::VideoOnly);
        video.width = Some(1920);
        video.height = Some(1080);
        video.tbr = Some(2500.0);
        let mut audio = format("hls-audio-0", StreamKind::AudioOnly);
        audio.ext = "aac".into();
        audio.abr = Some(128.5);
        audio.tbr = Some(130.0);
        audio.language = Some("日本語".into());
        audio.format_note = Some("Original audio".into());
        audio.audio_default = true;
        audio.audio_autoselect = true;
        video.fragments.push(crate::Fragment {
            url: "https://secret.example/part.m4s".parse().unwrap(),
            byte_range: None,
            encryption: Some(crate::HlsAes128 {
                key_url: "https://secret.example/private-key".parse().unwrap(),
                iv: [0; 16],
            }),
        });
        info.formats = vec![video, audio, format("18", StreamKind::Standalone)];
        let before = serde_json::to_value(&info).unwrap();
        let table = render_formats(&info);
        let rows: Vec<_> = table.lines().skip(2).collect();
        assert!(rows[0].starts_with("137 "));
        assert!(rows[1].starts_with("hls-audio-0 "));
        assert!(rows[2].starts_with("18 "));
        for text in [
            "1920x1080",
            "video only",
            "audio only",
            "standalone",
            "2500k",
            "128.5k",
            "130k",
            "日本語",
            "[default, autoselect] Original audio",
        ] {
            assert!(table.contains(text), "{text}");
        }
        assert!(!table.contains("combined"));
        assert!(!table.contains("https://"));
        assert!(!table.contains("private-key"));
        assert_eq!(serde_json::to_value(info).unwrap(), before);
    }
    #[test]
    fn dimension_and_rate_unknowns_are_explicit() {
        assert_eq!(resolution(StreamKind::Standalone, None, Some(720)), "720p");
        assert_eq!(
            resolution(StreamKind::VideoOnly, Some(1280), None),
            "1280x?"
        );
        assert_eq!(rate(Some(-0.0)), "0k");
        for value in [None, Some(f64::NAN), Some(f64::INFINITY), Some(-1.0)] {
            assert_eq!(rate(value), "-");
        }
        assert!(rate(Some(f64::MAX)).len() < 32);
    }
    #[test]
    fn untrusted_metadata_cannot_inject_terminal_controls_or_unbounded_padding() {
        let mut info = info();
        info.id = format!("id\x1b[31m\n{}", "日".repeat(1000));
        let mut source = format("id\n\x1b[2J", StreamKind::Standalone);
        source.ext = "mp4\t\r".into();
        source.language = Some("en\x07".into());
        source.format_note = Some(format!(
            "Unicode 日\x1b[31m\n\t\u{202e}{}",
            "文".repeat(1000)
        ));
        info.formats.push(source);
        let table = render_formats(&info);
        assert_eq!(table.lines().count(), 3);
        assert!(table.lines().next().unwrap().chars().count() <= HEADER_LIMIT);
        assert!(table.contains("\\x1b"));
        assert!(table.contains("\\x0a"));
        assert!(table.contains("\\u{202e}"));
        assert!(table.contains("Unicode 日"));
        assert!(table.contains('…'));
        assert!(!table.chars().any(|c| c.is_control() && c != '\n'));
        assert!(
            table
                .lines()
                .all(|line| line.chars().count() < 9 * (CELL_LIMIT + 2))
        );
        assert_eq!(safe("line\n", 128), r"line\x0a");
        assert_eq!(safe(&"文".repeat(129), 128).chars().count(), 128);
        assert!(safe(&format!("{}\x1b", "a".repeat(127)), 128).ends_with('…'));
    }
}
