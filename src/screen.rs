//! Upstream-style informational messages (`to_screen`) and warnings from
//! library code, delivered to a process-wide sink installed by the CLI.
//! Without a sink, messages are discarded.
use std::sync::OnceLock;

/// One message: a `[extractor] …` screen line or a warning text (without
/// the `WARNING: ` prefix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Message<'a> {
    Screen(&'a str),
    Warning(&'a str),
}

type Sink = Box<dyn Fn(Message<'_>) + Send + Sync>;

static SINK: OnceLock<Sink> = OnceLock::new();

/// Install the sink once; later calls are ignored.
pub fn set_sink(sink: impl Fn(Message<'_>) + Send + Sync + 'static) {
    let _ = SINK.set(Box::new(sink));
}

pub(crate) fn screen(text: &str) {
    if let Some(sink) = SINK.get() {
        sink(Message::Screen(text));
    }
}

pub(crate) fn warning(text: &str) {
    if let Some(sink) = SINK.get() {
        sink(Message::Warning(text));
    }
}

/// Upstream `truncate_string(s, left, right)`.
pub(crate) fn truncate_string(text: &str, left: usize, right: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= left + right {
        return text.to_owned();
    }
    let head: String = chars[..left - 3].iter().collect();
    let tail: String = if right > 0 {
        chars[chars.len() - right..].iter().collect()
    } else {
        String::new()
    };
    format!("{head}...{tail}")
}

/// Upstream `InfoExtractor._generic_id`: the unquoted last path component
/// of the URL text without its extension.
pub(crate) fn generic_id(url: &str) -> String {
    let last = url.trim_end_matches('/').rsplit('/').next().unwrap_or("");
    // `os.path.splitext`: the last dot, unless only dots precede it.
    let stem = match last.rfind('.') {
        Some(dot) if last[..dot].bytes().any(|b| b != b'.') => &last[..dot],
        _ => last,
    };
    // `urllib.parse.unquote` with UTF-8 and replacement characters.
    percent_encoding::percent_decode_str(stem)
        .decode_utf8_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generic_ids_and_truncation_follow_upstream() {
        for (url, id) in [
            ("http://h/media/clip.mp4?x=1#f", "clip"),
            ("http://h/dir/", "dir"),
            ("http://127.0.0.1:9/", "127.0.0"),
            ("http://h/a%20b.mp4", "a b"),
            ("http://h/.hidden", ".hidden"),
            ("http://h/..x.y", "..x"),
            ("http://h/%FF.mp4", "\u{fffd}"),
        ] {
            assert_eq!(generic_id(url), id, "{url}");
        }
        let long = format!("http://h/{}", "a".repeat(200));
        let truncated = truncate_string(&long, 100, 20);
        assert_eq!(truncated.chars().count(), 120);
        assert!(truncated.contains("..."));
        assert_eq!(truncate_string("short", 100, 20), "short");
    }
}
