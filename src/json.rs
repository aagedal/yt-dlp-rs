//! JSON text as Python's `json` module reads it: lone UTF-16 surrogate
//! escapes (`"\ud83d"` without its pair), which sites such as VK send in
//! truncated titles, are accepted. serde_json rejects them, so they become
//! U+FFFD (Python keeps the surrogate code point, which it cannot print or
//! encode as UTF-8 either).

use serde::de::DeserializeOwned;
use std::borrow::Cow;

/// `json.loads(text)`.
pub(crate) fn from_str<T: DeserializeOwned>(text: &str) -> serde_json::Result<T> {
    serde_json::from_str(&replace_lone_surrogates(text))
}

fn hex4(bytes: &[u8]) -> Option<u16> {
    let text = std::str::from_utf8(bytes.get(..4)?).ok()?;
    u16::from_str_radix(text, 16).ok()
}

/// The text with each unpaired `\uD800`–`\uDFFF` escape as `\uFFFD`
/// (borrowed when there is none).
pub(crate) fn replace_lone_surrogates(text: &str) -> Cow<'_, str> {
    if !text.contains("\\u") {
        return Cow::Borrowed(text);
    }
    let bytes = text.as_bytes();
    let mut lone = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != b'\\' {
            index += 1;
            continue;
        }
        if bytes.get(index + 1) != Some(&b'u') {
            // Any other escape is two characters (`\\` included).
            index += 2;
            continue;
        }
        let Some(unit) = hex4(&bytes[index + 2..]) else {
            index += 2;
            continue;
        };
        match unit {
            0xD800..=0xDBFF => {
                let rest = &bytes[index + 6..];
                let paired = rest.starts_with(b"\\u")
                    && hex4(&rest[2..]).is_some_and(|low| (0xDC00..=0xDFFF).contains(&low));
                if paired {
                    index += 12;
                } else {
                    lone.push(index);
                    index += 6;
                }
            }
            0xDC00..=0xDFFF => {
                lone.push(index);
                index += 6;
            }
            _ => index += 6,
        }
    }
    if lone.is_empty() {
        return Cow::Borrowed(text);
    }
    let mut output = String::with_capacity(text.len());
    let mut last = 0;
    for start in lone {
        output.push_str(&text[last..start]);
        output.push_str("\\uFFFD");
        last = start + 6;
    }
    output.push_str(&text[last..]);
    Cow::Owned(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn lone_surrogates_parse_like_python_json() {
        let value: Value = from_str(r#"{"a": "x\ud83d", "b": "\udc00y", "c": "😀"}"#).unwrap();
        assert_eq!(
            value,
            json!({"a": "x\u{FFFD}", "b": "\u{FFFD}y", "c": "😀"})
        );
        // An escaped backslash before `u` is not an escape.
        let value: Value = from_str(r#"["\\ud83d", "\\\ud83d"]"#).unwrap();
        assert_eq!(value, json!(["\\ud83d", "\\\u{FFFD}"]));
        assert!(matches!(
            replace_lone_surrogates(r#"{"plain": "é"}"#),
            Cow::Borrowed(_)
        ));
    }
}
