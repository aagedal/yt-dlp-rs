//! Upstream `MetadataParserPP` (`--parse-metadata`, `--replace-in-metadata`):
//! fields interpreted from rendered templates and regex replacements, applied
//! to the info dict at the requested stage.
use crate::{VideoInfo, output::TemplateOptions};
use regex::Regex;
use serde_json::Value;

/// Upstream `POSTPROCESS_WHEN`.
pub const WHEN: [&str; 8] = [
    "pre_process",
    "after_filter",
    "video",
    "before_dl",
    "post_process",
    "after_move",
    "after_video",
    "playlist",
];

#[derive(Debug, Clone)]
enum Action {
    Interpret {
        input: String,
        output: String,
        template: String,
        regex: Regex,
    },
    Replace {
        field: String,
        search: String,
        regex: Regex,
        replace: String,
    },
}

/// Actions grouped by stage, in first-use order like upstream's dict.
#[derive(Debug, Clone, Default)]
pub struct MetadataActions {
    stages: Vec<(String, Vec<Action>)>,
}

/// `_dict_from_options_callback(..., allowed_keys=POSTPROCESS_WHEN)`.
fn split_when(value: &str) -> (String, &str) {
    if let Some((when, rest)) = value.split_once(':') {
        let lower = when.to_lowercase();
        if WHEN.contains(&lower.as_str()) {
            return (lower, rest);
        }
    }
    ("pre_process".into(), value)
}

/// `MetadataParserPP.field_to_template`.
fn field_to_template(input: &str) -> Result<String, String> {
    if !input.is_empty() && input.chars().all(|c| c.is_ascii_alphabetic() || c == '_') {
        return Ok(format!("%({input})s"));
    }
    // `validate_outtmpl`.
    crate::output::evaluate_template(input, &placeholder(), &TemplateOptions::default())
        .map_err(|error| error.to_string())?;
    Ok(input.to_owned())
}

fn placeholder() -> VideoInfo {
    serde_json::from_value(
        serde_json::json!({"id": "id", "title": "title", "ext": "mp4",
        "url": "https://example.invalid/", "webpage_url": "https://example.invalid/",
        "extractor": "generic", "extractor_key": "Generic"}),
    )
    .expect("static placeholder")
}

fn word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// `MetadataParserPP.format_to_regex`.
fn format_to_regex(format: &str) -> String {
    if word(format) {
        return format!("(?s)(?P<{format}>.+)");
    }
    static FIELD: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"%\((\w+)\)s").unwrap());
    if !FIELD.is_match(format) {
        return format.to_owned();
    }
    let mut regex = String::new();
    let mut last = 0;
    for captures in FIELD.captures_iter(format) {
        let whole = captures.get(0).unwrap();
        regex.push_str(&regex::escape(&format[last..whole.start()]));
        regex.push_str(&format!("(?P<{}>.+)", &captures[1]));
        last = whole.end();
    }
    regex.push_str(&regex::escape(&format[last..]));
    regex
}

/// A Python `re.sub` replacement string in `regex` crate syntax; errors use
/// `re.error` text with the escape's position.
fn replacement(text: &str) -> Result<String, String> {
    let mut output = String::new();
    let mut chars = text.chars().enumerate().peekable();
    while let Some((position, c)) = chars.next() {
        match c {
            '$' => output.push_str("$$"),
            '\\' => match chars.next().map(|(_, c)| c) {
                Some(d @ '0'..='9') => {
                    let mut group = d.to_string();
                    if let Some((_, next)) =
                        chars.peek().copied().filter(|(_, c)| c.is_ascii_digit())
                    {
                        group.push(next);
                        chars.next();
                    }
                    output.push_str(&format!("${{{group}}}"));
                }
                Some('g') => {
                    if chars.next().map(|(_, c)| c) != Some('<') {
                        return Err(format!("missing < at position {}", position + 2));
                    }
                    let name: String = chars
                        .by_ref()
                        .map(|(_, c)| c)
                        .take_while(|c| *c != '>')
                        .collect();
                    output.push_str(&format!("${{{name}}}"));
                }
                Some('n') => output.push('\n'),
                Some('t') => output.push('\t'),
                Some('r') => output.push('\r'),
                Some('\\') => output.push('\\'),
                Some(other) if other.is_ascii_alphabetic() => {
                    return Err(format!("bad escape \\{other} at position {position}"));
                }
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => {
                    return Err(format!(
                        "bad escape (end of pattern) at position {position}"
                    ));
                }
            },
            other => output.push(other),
        }
    }
    Ok(output)
}

impl MetadataActions {
    fn push(&mut self, when: String, action: Action) {
        match self.stages.iter_mut().find(|(stage, _)| *stage == when) {
            Some((_, actions)) => actions.push(action),
            None => self.stages.push((when, vec![action])),
        }
    }

    /// `--parse-metadata [WHEN:]FROM:TO` (`MetadataFromFieldPP.to_action`).
    pub fn add_interpret(&mut self, value: &str) -> Result<(), String> {
        let (when, spec) = split_when(value);
        let cmd = format!("--parse-metadata {}", crate::output::shell_quote(spec));
        static ACTION: std::sync::LazyLock<fancy_free::Split> =
            std::sync::LazyLock::new(fancy_free::Split::default);
        let (input, output) = ACTION.split(spec).ok_or_else(|| {
            format!(
                "{cmd} is invalid; it should be FROM:TO, not {}",
                crate::output::traverse::py_repr_str(spec)
            )
        })?;
        let input = input.replace("\\:", ":");
        let template =
            field_to_template(&input).map_err(|error| format!("{cmd} is invalid; {error}"))?;
        let regex = Regex::new(&format_to_regex(&output))
            .map_err(|error| format!("{cmd} is invalid; {error}"))?;
        self.push(
            when,
            Action::Interpret {
                input,
                output,
                template,
                regex,
            },
        );
        Ok(())
    }

    /// `--replace-in-metadata [WHEN:]FIELDS REGEX REPLACE`.
    pub fn add_replace(&mut self, fields: &str, search: &str, replace: &str) -> Result<(), String> {
        let (when, fields) = split_when(fields);
        let cmd = format!(
            "--replace-in-metadata {}",
            [fields, search, replace]
                .iter()
                .map(|part| crate::output::shell_quote(part))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let regex = Regex::new(search).map_err(|error| format!("{cmd} is invalid; {error}"))?;
        // Like `re.subn`, the replacement is only parsed when applied.
        let replace = replace.to_owned();
        for field in fields.split(',') {
            self.push(
                when.clone(),
                Action::Replace {
                    field: field.to_owned(),
                    search: search.to_owned(),
                    regex: regex.clone(),
                    replace: replace.clone(),
                },
            );
        }
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.stages.is_empty()
    }

    /// Whether any action runs at `when`.
    pub fn has(&self, when: &str) -> bool {
        self.stages.iter().any(|(stage, _)| stage == when)
    }

    /// Run the actions for `when` on `info` (`MetadataParserPP.run`),
    /// returning the `[MetadataParser]` screen lines and warnings in order.
    /// An `Err` is the exception upstream raises (`re.error` text).
    pub fn run(
        &self,
        when: &str,
        info: &mut VideoInfo,
        options: &TemplateOptions,
    ) -> Result<Vec<Message>, (Vec<Message>, String)> {
        let mut messages = Vec::new();
        let Some((_, actions)) = self.stages.iter().find(|(stage, _)| stage == when) else {
            return Ok(messages);
        };
        let screen = |messages: &mut Vec<Message>, text: String| {
            messages.push(Message::Screen(format!("[MetadataParser] {text}")));
        };
        for action in actions {
            match action {
                Action::Interpret {
                    input,
                    output,
                    template,
                    regex,
                } => {
                    let data = crate::output::evaluate_template(template, info, options)
                        .unwrap_or_default();
                    let Some(captures) = regex.captures(&data) else {
                        screen(
                            &mut messages,
                            format!(
                                "Could not interpret {} as {}",
                                crate::output::traverse::py_repr_str(input),
                                crate::output::traverse::py_repr_str(output)
                            ),
                        );
                        continue;
                    };
                    for name in regex.capture_names().flatten() {
                        let Some(value) = captures.name(name) else {
                            continue;
                        };
                        let value = value.as_str().to_owned();
                        set_field(info, name, Value::String(value.clone()));
                        screen(
                            &mut messages,
                            format!(
                                "Parsed {name} from {}: {}",
                                crate::output::traverse::py_repr_str(template),
                                crate::output::traverse::py_repr_str(&value)
                            ),
                        );
                    }
                }
                Action::Replace {
                    field,
                    search,
                    regex,
                    replace,
                } => {
                    let value = serde_json::to_value(&*info)
                        .ok()
                        .and_then(|dict| dict.get(field).cloned())
                        .filter(|value| !value.is_null());
                    let Some(value) = value else {
                        screen(&mut messages, format!("Video does not have a {field}"));
                        continue;
                    };
                    let Value::String(text) = value else {
                        let kind = match value {
                            Value::Bool(_) => "bool",
                            Value::Number(number) if number.is_f64() => "float",
                            Value::Number(_) => "int",
                            Value::Array(_) => "list",
                            _ => "dict",
                        };
                        messages.push(Message::Warning(format!(
                            "WARNING: Cannot replace in field {field} since it is a {kind}"
                        )));
                        continue;
                    };
                    let count = regex.find_iter(&text).count();
                    let replace = match replacement(replace) {
                        Ok(replace) => replace,
                        Err(error) => return Err((messages, error)),
                    };
                    let replaced = regex.replace_all(&text, replace.as_str()).into_owned();
                    set_field(info, field, Value::String(replaced.clone()));
                    if count > 0 {
                        screen(&mut messages, format!("Changed {field} to: {replaced}"));
                    } else {
                        screen(
                            &mut messages,
                            format!(
                                "Did not find {} in {field}",
                                crate::output::traverse::py_repr_str(search)
                            ),
                        );
                    }
                }
            }
        }
        Ok(messages)
    }
}

/// A parser screen line or warning.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    Screen(String),
    Warning(String),
}

/// `info[key] = value` on the typed info dict: represented fields are
/// replaced through a serialization round trip, others go to `extra`.
pub fn set_field(info: &mut VideoInfo, key: &str, value: Value) {
    let Ok(Value::Object(mut map)) = serde_json::to_value(&*info) else {
        return;
    };
    map.insert(key.to_owned(), value.clone());
    let last_playlist_index = info.last_playlist_index;
    let mut extra = info.extra.clone();
    if let Ok(mut updated) = serde_json::from_value::<VideoInfo>(Value::Object(map)) {
        updated.last_playlist_index = last_playlist_index;
        updated.extra = std::mem::take(&mut extra);
        let represented = serde_json::to_value(&updated)
            .ok()
            .and_then(|dict| dict.get(key).cloned())
            == Some(value.clone());
        if represented {
            updated.extra.remove(key);
        } else {
            updated.extra.insert(key.to_owned(), value);
        }
        *info = updated;
    } else {
        // A represented field of another type keeps its typed value; the
        // text is still available to templates through `extra`.
        info.extra.insert(key.to_owned(), value);
    }
}

/// Upstream's `(?s)(?P<in>.*?)(?<!\\):(?P<out>.+)$` without look-behind.
mod fancy_free {
    #[derive(Default)]
    pub struct Split;

    impl Split {
        /// The first `:` not preceded by a backslash, with a nonempty rest.
        pub fn split(&self, text: &str) -> Option<(String, String)> {
            let bytes = text.as_bytes();
            for (index, byte) in bytes.iter().enumerate() {
                if *byte == b':' && (index == 0 || bytes[index - 1] != b'\\') {
                    let rest = &text[index + 1..];
                    if rest.is_empty() {
                        return None;
                    }
                    return Some((text[..index].to_owned(), rest.to_owned()));
                }
            }
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> VideoInfo {
        serde_json::from_value(serde_json::json!({"id": "x1", "title": "Artist - Song (Live)",
            "ext": "mp4", "url": "https://example.test/v.mp4", "webpage_url": "https://example.test/v",
            "extractor": "generic", "extractor_key": "Generic", "duration": 30}))
        .unwrap()
    }

    #[test]
    fn interpret_and_replace_follow_metadata_parser_pp() {
        // Expected lines printed by pinned upstream MetadataParserPP.
        let mut actions = MetadataActions::default();
        actions
            .add_interpret("title:%(artist)s - %(track)s")
            .unwrap();
        actions
            .add_interpret("%(id)s:(?P<display>x)(?P<missing>z)?")
            .unwrap();
        actions.add_interpret("description:(?P<x>.+)").unwrap();
        actions
            .add_replace("title,track", r"\s*\((\w+)\)", r"[\1]")
            .unwrap();
        actions.add_replace("duration", "3", "4").unwrap();
        actions.add_replace("uploader", "a", "b").unwrap();
        actions.add_replace("title", "zzz", "b").unwrap();
        actions.add_interpret("title:^nomatch$").unwrap();
        let mut video = info();
        let messages = actions
            .run("pre_process", &mut video, &TemplateOptions::default())
            .unwrap();
        let text: Vec<String> = messages
            .iter()
            .map(|message| match message {
                Message::Screen(text) | Message::Warning(text) => text.clone(),
            })
            .collect();
        assert_eq!(
            text,
            [
                "[MetadataParser] Parsed artist from '%(title)s': 'Artist'",
                "[MetadataParser] Parsed track from '%(title)s': 'Song (Live)'",
                "[MetadataParser] Parsed display from '%(id)s': 'x'",
                "[MetadataParser] Parsed x from '%(description)s': 'NA'",
                "[MetadataParser] Changed title to: Artist - Song[Live]",
                "[MetadataParser] Changed track to: Song[Live]",
                "WARNING: Cannot replace in field duration since it is a int",
                "[MetadataParser] Video does not have a uploader",
                "[MetadataParser] Did not find 'zzz' in title",
                "[MetadataParser] Could not interpret 'title' as '^nomatch$'",
            ]
        );
        assert_eq!(video.title, "Artist - Song[Live]");
        assert_eq!(video.extra["artist"], "Artist");
        assert_eq!(video.extra["track"], "Song[Live]");
        assert!(!video.extra.contains_key("title"));
        let rendered = crate::output::evaluate_template(
            "%(artist)s|%(track)s|%(display)s",
            &video,
            &TemplateOptions::default(),
        )
        .unwrap();
        assert_eq!(rendered, "Artist|Song[Live]|x");
    }

    #[test]
    fn invalid_actions_report_upstream_messages() {
        let mut actions = MetadataActions::default();
        assert_eq!(
            actions.add_interpret("title").unwrap_err(),
            "--parse-metadata title is invalid; it should be FROM:TO, not 'title'"
        );
        assert!(
            actions
                .add_interpret("title:(?P<a>")
                .unwrap_err()
                .starts_with("--parse-metadata 'title:(?P<a>' is invalid; ")
        );
        actions.add_interpret("VIDEO:title:%(x)s").unwrap();
        actions.add_replace("after_move:title", "a", "b").unwrap();
        assert!(actions.has("video") && actions.has("after_move") && !actions.has("pre_process"));
        let mut bad = MetadataActions::default();
        bad.add_replace("title", "a", r"x\q").unwrap();
        let (_, error) = bad
            .run("pre_process", &mut info(), &TemplateOptions::default())
            .unwrap_err();
        assert_eq!(error, r"bad escape \q at position 1");
    }
}
