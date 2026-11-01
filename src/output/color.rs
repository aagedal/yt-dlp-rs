//! Upstream terminal colors: `minicurses.format_text`, `YoutubeDL.Styles`
//! and the per-stream `--color` policies (`_allow_colors`).
use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

/// Upstream `_allow_colors.STREAM`: `True`, `False` or `'no_color'`
/// (terminal sequences without colors).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Allow {
    #[default]
    No,
    Yes,
    NoColor,
}

impl Allow {
    fn load(value: u8) -> Self {
        match value {
            1 => Self::Yes,
            2 => Self::NoColor,
            _ => Self::No,
        }
    }

    fn store(self) -> u8 {
        match self {
            Self::No => 0,
            Self::Yes => 1,
            Self::NoColor => 2,
        }
    }

    /// Whether terminal sequences (cursor movement, line erasing) are used:
    /// upstream's truthy `_HAVE_FULLCAP`.
    pub fn sequences(self) -> bool {
        self != Self::No
    }
}

static STDOUT: AtomicU8 = AtomicU8::new(0);
static STDERR: AtomicU8 = AtomicU8::new(0);
/// Upstream `_out_files.screen`: stderr when quiet.
static SCREEN_IS_STDERR: AtomicBool = AtomicBool::new(false);

/// Install the resolved policies of both streams.
pub fn set(stdout: Allow, stderr: Allow) {
    STDOUT.store(stdout.store(), Ordering::Relaxed);
    STDERR.store(stderr.store(), Ordering::Relaxed);
}

/// Whether screen messages go to stderr (`quiet`).
pub fn set_screen_is_stderr(stderr: bool) {
    SCREEN_IS_STDERR.store(stderr, Ordering::Relaxed);
}

/// `_allow_colors.out`.
pub fn out() -> Allow {
    Allow::load(STDOUT.load(Ordering::Relaxed))
}

/// `_allow_colors.error`.
pub fn err() -> Allow {
    Allow::load(STDERR.load(Ordering::Relaxed))
}

/// `_allow_colors.screen`.
pub fn screen() -> Allow {
    if SCREEN_IS_STDERR.load(Ordering::Relaxed) {
        err()
    } else {
        out()
    }
}

/// Upstream `YoutubeDL.Styles`.
pub mod styles {
    pub const HEADERS: &str = "yellow";
    pub const EMPHASIS: &str = "light blue";
    pub const FILENAME: &str = "green";
    pub const ID: &str = "green";
    pub const DELIM: &str = "blue";
    pub const ERROR: &str = "red";
    pub const BAD_FORMAT: &str = "light red";
    pub const WARNING: &str = "yellow";
    pub const SUPPRESS: &str = "light black";
}

/// Upstream `FileDownloader.ProgressStyles`.
pub mod progress_styles {
    pub const DOWNLOADED_BYTES: &str = "light blue";
    pub const PERCENT: &str = "light blue";
    pub const ETA: &str = "yellow";
    pub const SPEED: &str = "green";
    pub const ELAPSED: &str = "bold white";
    pub const TOTAL_BYTES: &str = "";
}

const RESET: &str = "\x1b[0m";

fn color(name: &str) -> Option<char> {
    Some(match name {
        "BLACK" => '0',
        "RED" => '1',
        "GREEN" => '2',
        "YELLOW" => '3',
        "BLUE" => '4',
        "PURPLE" => '5',
        "CYAN" => '6',
        "WHITE" => '7',
        _ => return None,
    })
}

/// Upstream `minicurses.format_text` for the formats this port uses:
/// `[style] [light] color [on [light] color]`. An empty format leaves the
/// text unchanged; an invalid one is a programming error.
pub fn format_text(text: &str, format: &str) -> String {
    let format = format.to_uppercase();
    let mut tokens: Vec<&str> = format.split_whitespace().collect();
    let mut background = String::new();
    if tokens.contains(&"ON") {
        let bg = color(tokens.pop().unwrap_or_default()).expect("background color");
        background = format!("4{bg}");
        if tokens.last() == Some(&"LIGHT") {
            background = format!("0;10{bg}");
            tokens.pop();
        }
        assert_eq!(tokens.pop(), Some("ON"), "invalid format {format:?}");
        background = format!("\x1b[{background}m");
    }
    let foreground = match tokens.pop() {
        None => String::new(),
        Some(name) => {
            let fg = color(name).expect("foreground color");
            let mut code = format!("3{fg}");
            if tokens.last() == Some(&"LIGHT") {
                code = format!("9{fg}");
                tokens.pop();
            }
            // `_TEXT_STYLES`, `NORMAL` by default.
            let style = match tokens.last() {
                Some(&"BOLD") => Some('1'),
                Some(&"UNDERLINED") => Some('4'),
                Some(&"NORMAL") => Some('0'),
                _ => None,
            };
            if style.is_some() {
                tokens.pop();
            }
            assert!(tokens.is_empty(), "invalid format {format:?}");
            let style = style.unwrap_or('0');
            format!("\x1b[{style};{code}m")
        }
    };
    if foreground.is_empty() && background.is_empty() {
        return text.to_owned();
    }
    let start = format!("{foreground}{background}");
    format!("{start}{}{RESET}", text.replace(RESET, &start))
}

/// Upstream `_format_text` without an encoding test.
fn styled(allow: Allow, text: &str, style: &str) -> String {
    if allow == Allow::Yes {
        format_text(text, style)
    } else {
        text.to_owned()
    }
}

/// Upstream `_format_out`.
pub fn format_out(text: &str, style: &str) -> String {
    styled(out(), text, style)
}

/// Upstream `_format_err`.
pub fn format_err(text: &str, style: &str) -> String {
    styled(err(), text, style)
}

/// Upstream `_format_screen`.
pub fn format_screen(text: &str, style: &str) -> String {
    styled(screen(), text, style)
}

/// A stderr line with upstream's styled `ERROR:` (`report_error`),
/// `WARNING:` (`report_warning`) or `Deprecated Feature:` prefix.
pub fn err_prefix(line: String) -> String {
    if err() != Allow::Yes {
        return line;
    }
    for (prefix, style) in [
        ("ERROR:", styles::ERROR),
        ("WARNING:", styles::WARNING),
        ("Deprecated Feature:", styles::ERROR),
    ] {
        if let Some(rest) = line.strip_prefix(prefix) {
            let rest = if prefix == "Deprecated Feature:" {
                rest.to_owned()
            } else {
                emphasize_extracted_name(rest)
            };
            return format!("{}{rest}", format_text(prefix, style));
        }
    }
    line
}

/// Messages upstream raises with these literal texts (after `Unable to
/// extract `) instead of `_search_regex`'s, so their names are not styled.
static LITERAL_EXTRACT_MESSAGES: std::sync::LazyLock<regex::Regex> =
    std::sync::LazyLock::new(|| {
        regex::Regex::new(concat!(
            r"^(?:JSON-LD|NRK ID|access credentials|access token|actual video ID|ad data",
            r"|any SproutVideo embed url|any media blocks from webpage|auth token",
            r"|aweme detail info|brightcove Video ID|brightcove reference ID",
            r"|broadcast number|challenge data|client id|content|embedUrl|file ID",
            r"|initial state|main content from webpage|moment id|on-air start time",
            r"|player JS|playlist url|stream host|stream url|the user id",
            r"|token cookie values|user id|video ID|video ID from webpage|video URL",
            r"|video from webpage|video id|video info|video info from html inline initial state",
            r"|video urls|live URL information)$",
            r"|^(?:post title\. Falling back to parsing html page",
            r"|secondary user ID\. |the VOD associated with this livestream",
            r"|token, continuing without authentication|video info, make sure the URL is valid",
            r"|format info for |formats: |metadata: |url for clip |\S+ version information$",
            r"|\S+ format$)",
        ))
        .expect("valid pattern")
    });

/// Upstream `_search_regex`/`_search_json`'s `Unable to extract {_name}`
/// (and the `unable to extract` warning): the name in `Styles.EMPHASIS`.
fn emphasize_extracted_name(message: &str) -> String {
    const ENDS: [&str; 2] = ["; please report", " - Failed to parse JSON"];
    let Some(at) = message
        .find("Unable to extract ")
        .or_else(|| message.find("unable to extract "))
    else {
        return message.to_owned();
    };
    let start = at + "Unable to extract ".len();
    let rest = &message[start..];
    let body = rest.strip_suffix('\n').unwrap_or(rest);
    let end = ENDS
        .iter()
        .filter_map(|end| body.find(end))
        .min()
        .unwrap_or(body.len());
    let name = &body[..end];
    if name.is_empty()
        || LITERAL_EXTRACT_MESSAGES.is_match(body)
        || LITERAL_EXTRACT_MESSAGES.is_match(name)
    {
        return message.to_owned();
    }
    format!(
        "{}{}{}",
        &message[..start],
        format_text(name, styles::EMPHASIS),
        &rest[end..]
    )
}

/// Upstream `_format_out(text, style, fallback, test_encoding=True)`: the
/// fallback unless colors are allowed and stdout's encoding keeps `text`.
pub fn format_out_or(text: &str, style: &str, fallback: &str) -> String {
    if out() != Allow::Yes {
        return fallback.to_owned();
    }
    // `text.encode(encoding, 'ignore')` drops what it cannot represent.
    let encoding = super::encoding::output();
    let bom = encoding.encode("").len();
    let kept = text.chars().all(|c| {
        let mut buffer = [0; 4];
        encoding.encode(c.encode_utf8(&mut buffer)).len() > bom
    });
    format_text(if kept { text } else { fallback }, style)
}

/// Upstream `remove_terminal_sequences` (`\x1b[...m`).
pub fn remove_terminal_sequences(text: &str) -> Cow<'_, str> {
    static SEQUENCE: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new("\x1b\\[[^m]+m").expect("valid pattern"));
    SEQUENCE.replace_all(text, "")
}

/// Upstream `process_color_policy` for one stream: `policy` (default
/// `auto`), whether it `supports_terminal_sequences` (a tty with `TERM`
/// set, or in VT mode on Windows), `TERM` other than `dumb`, and `NO_COLOR`.
pub fn resolve(
    policy: Option<&str>,
    supports_sequences: bool,
    term_allows: bool,
    no_color_env: bool,
) -> Allow {
    match policy.unwrap_or("auto") {
        "always" => Allow::Yes,
        "never" => Allow::No,
        "no_color" => Allow::NoColor,
        policy => {
            let no_color = match policy {
                "auto-tty" => false,
                "no_color-tty" => true,
                _ => no_color_env,
            };
            if term_allows && supports_sequences {
                if no_color { Allow::NoColor } else { Allow::Yes }
            } else {
                Allow::No
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_text_matches_minicurses() {
        // Saved from pinned upstream `minicurses.format_text`.
        assert_eq!(format_text("x", "red"), "\x1b[0;31mx\x1b[0m");
        assert_eq!(format_text("x", "light blue"), "\x1b[0;94mx\x1b[0m");
        assert_eq!(format_text("x", "light black"), "\x1b[0;90mx\x1b[0m");
        assert_eq!(format_text("x", "bold white"), "\x1b[1;37mx\x1b[0m");
        assert_eq!(format_text("x", ""), "x");
        assert_eq!(
            format_text("a\x1b[0mb", "yellow"),
            "\x1b[0;33ma\x1b[0;33mb\x1b[0m"
        );
        assert_eq!(
            format_text("x", "bold green on light blue"),
            "\x1b[1;32m\x1b[0;104mx\x1b[0m"
        );
    }

    #[test]
    fn search_regex_names_are_emphasized() {
        let blue = |name: &str| format_text(name, styles::EMPHASIS);
        assert_eq!(
            emphasize_extracted_name(
                " [x] id: Unable to extract title; please report this issue on  ...\n"
            ),
            format!(
                " [x] id: Unable to extract {}; please report this issue on  ...\n",
                blue("title")
            )
        );
        assert_eq!(
            emphasize_extracted_name(
                " [x] id: Unable to extract player data - Failed to parse JSON"
            ),
            format!(
                " [x] id: Unable to extract {} - Failed to parse JSON",
                blue("player data")
            )
        );
        assert_eq!(
            emphasize_extracted_name(" [x] unable to extract views; please report"),
            format!(" [x] unable to extract {}; please report", blue("views"))
        );
        // Upstream's literal messages keep their plain text.
        for message in [
            " [x] id: Unable to extract video URL; please report this issue",
            " [x] id: Unable to extract JSON-LD",
            " [x] id: Unable to extract url for clip 2",
            " [cda] id: Unable to extract player version information",
            " [x] id: Unable to extract formats: 403",
        ] {
            assert_eq!(emphasize_extracted_name(message), message);
        }
    }

    #[test]
    fn terminal_sequences_are_removed() {
        assert_eq!(remove_terminal_sequences("\x1b[0;31mab\x1b[0m"), "ab");
        assert_eq!(remove_terminal_sequences("plain"), "plain");
        assert_eq!(remove_terminal_sequences("\x1b[m!"), "\x1b[m!");
    }

    #[test]
    fn policies_follow_process_color_policy() {
        use Allow::*;
        for (policy, tty, term, env, expected) in [
            (None, true, true, false, Yes),
            (None, false, true, false, No),
            (None, true, false, false, No),
            (None, true, true, true, NoColor),
            (Some("auto-tty"), true, true, true, Yes),
            (Some("no_color-tty"), true, true, false, NoColor),
            (Some("no_color-tty"), false, true, false, No),
            (Some("always"), false, false, true, Yes),
            (Some("never"), true, true, false, No),
            (Some("no_color"), false, false, false, NoColor),
        ] {
            assert_eq!(resolve(policy, tty, term, env), expected, "{policy:?}");
        }
    }
}
