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

/// Python's `json.loads(text, strict=False)` error, if any: the message and
/// the character offset of its `JSONDecodeError` (CPython's decoder; nesting
/// deeper than 1000 levels, Python's `RecursionError`, is not modeled), for
/// `decode` (whole documents), or `raw_decode` (`ignore_extra`: trailing
/// data is not an error).
fn decode_error(text: &str, raw: bool) -> Option<(String, usize)> {
    let chars: Vec<char> = text.chars().collect();
    let scanner = PyScanner { s: &chars };
    let start = if raw { 0 } else { scanner.whitespace(0) };
    let end = match scanner.scan_once(start, 0) {
        Ok(end) => end,
        Err(PyError::Stop(index)) => return Some(("Expecting value".into(), index)),
        Err(PyError::Decode(message, index)) => return Some((message, index)),
    };
    if raw {
        return None;
    }
    let end = scanner.whitespace(end);
    (end != chars.len()).then(|| ("Extra data".into(), end))
}

/// `LenientJSONDecoder`'s error text, as upstream's `_parse_json` warnings
/// show it: `MSG in 'SNIPPET': line L column C (char P)`.
pub(crate) fn python_decode_error_text(text: &str) -> Option<String> {
    error_text(text, false)
}

/// [`python_decode_error_text`] with `ignore_extra=True`: `raw_decode` of
/// the text with leading (Python) whitespace stripped, as `_search_json`
/// parses.
pub(crate) fn python_raw_decode_error_text(text: &str) -> Option<String> {
    error_text(text.trim_start_matches(crate::batch::is_py_space), true)
}

fn error_text(text: &str, raw: bool) -> Option<String> {
    let (message, pos) = decode_error(text, raw)?;
    let chars: Vec<char> = text.chars().collect();
    // `s[pos - 10:pos + 10]` with Python's negative-index slicing.
    let len = chars.len() as isize;
    let clamp = |index: isize| -> usize {
        let index = if index < 0 { index + len } else { index };
        index.clamp(0, len) as usize
    };
    let (from, to) = (clamp(pos as isize - 10), clamp(pos as isize + 10));
    let snippet: String = if from < to {
        chars[from..to].iter().collect()
    } else {
        String::new()
    };
    let before = &chars[..pos.min(chars.len())];
    let line = before.iter().filter(|c| **c == '\n').count() + 1;
    let column = match before.iter().rposition(|c| *c == '\n') {
        Some(newline) => pos - newline,
        None => pos + 1,
    };
    Some(format!(
        "{message} in {}: line {line} column {column} (char {pos})",
        crate::batch::py_repr(&snippet)
    ))
}

enum PyError {
    /// `StopIteration(idx)`: no value starts here.
    Stop(usize),
    Decode(String, usize),
}

struct PyScanner<'a> {
    s: &'a [char],
}

const PY_WHITESPACE: [char; 4] = [' ', '\t', '\n', '\r'];

impl PyScanner<'_> {
    fn at(&self, index: usize) -> Option<char> {
        self.s.get(index).copied()
    }

    fn starts_with(&self, index: usize, word: &str) -> bool {
        let word: Vec<char> = word.chars().collect();
        self.s.get(index..index + word.len()) == Some(&word[..])
    }

    fn whitespace(&self, mut index: usize) -> usize {
        while self.at(index).is_some_and(|c| PY_WHITESPACE.contains(&c)) {
            index += 1;
        }
        index
    }

    fn decode<T>(message: &str, index: usize) -> Result<T, PyError> {
        Err(PyError::Decode(message.into(), index))
    }

    fn scan_once(&self, index: usize, depth: usize) -> Result<usize, PyError> {
        let Some(next) = self.at(index) else {
            return Err(PyError::Stop(index));
        };
        if depth > 1000 {
            return Self::decode("maximum recursion depth exceeded", index);
        }
        match next {
            '"' => return self.string(index + 1),
            '{' => return self.object(index + 1, depth + 1),
            '[' => return self.array(index + 1, depth + 1),
            'n' if self.starts_with(index, "null") => return Ok(index + 4),
            't' if self.starts_with(index, "true") => return Ok(index + 4),
            'f' if self.starts_with(index, "false") => return Ok(index + 5),
            _ => {}
        }
        if let Some(end) = self.number(index) {
            return Ok(end);
        }
        match next {
            'N' if self.starts_with(index, "NaN") => Ok(index + 3),
            'I' if self.starts_with(index, "Infinity") => Ok(index + 8),
            '-' if self.starts_with(index, "-Infinity") => Ok(index + 9),
            _ => Err(PyError::Stop(index)),
        }
    }

    /// `NUMBER_RE`: `-?(?:0|[1-9][0-9]*)(\.[0-9]+)?([eE][-+]?[0-9]+)?`.
    fn number(&self, index: usize) -> Option<usize> {
        let digits = |mut at: usize| {
            let from = at;
            while self.at(at).is_some_and(|c| c.is_ascii_digit()) {
                at += 1;
            }
            (at > from).then_some(at)
        };
        let mut at = index;
        if self.at(at) == Some('-') {
            at += 1;
        }
        at = match self.at(at)? {
            '0' => at + 1,
            '1'..='9' => digits(at)?,
            _ => return None,
        };
        if self.at(at) == Some('.')
            && let Some(end) = digits(at + 1)
        {
            at = end;
        }
        if matches!(self.at(at), Some('e' | 'E')) {
            let mut exponent = at + 1;
            if matches!(self.at(exponent), Some('+' | '-')) {
                exponent += 1;
            }
            if let Some(end) = digits(exponent) {
                at = end;
            }
        }
        Some(at)
    }

    /// The C `scanstring_unicode(s, end, strict=False)`: the index after the
    /// string.
    fn string(&self, mut end: usize) -> Result<usize, PyError> {
        let begin = end - 1;
        let len = self.s.len();
        let hex = |digit: char| digit.to_digit(16).filter(|_| digit.is_ascii_hexdigit());
        loop {
            let mut next = end;
            while next < len && !matches!(self.s[next], '"' | '\\') {
                next += 1;
            }
            if next == len {
                return Self::decode("Unterminated string starting at", begin);
            }
            next += 1;
            if self.s[next - 1] == '"' {
                return Ok(next);
            }
            if next == len {
                return Self::decode("Unterminated string starting at", begin);
            }
            let escape = self.s[next];
            if escape != 'u' {
                end = next + 1;
                if !matches!(escape, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't') {
                    return Self::decode("Invalid \\escape", end - 2);
                }
                continue;
            }
            next += 1;
            end = next + 4;
            if end > len {
                return Self::decode("Invalid \\uXXXX escape", next - 1);
            }
            let mut unit = 0;
            for &digit in &self.s[next..end] {
                let Some(value) = hex(digit) else {
                    return Self::decode("Invalid \\uXXXX escape", end - 5);
                };
                unit = unit << 4 | value;
            }
            next = end;
            if (0xd800..=0xdbff).contains(&unit)
                && end + 6 < len
                && self.s[next] == '\\'
                && self.s[next + 1] == 'u'
            {
                next += 2;
                end += 6;
                let mut low = 0;
                for &digit in &self.s[next..end] {
                    let Some(value) = hex(digit) else {
                        return Self::decode("Invalid \\uXXXX escape", end - 5);
                    };
                    low = low << 4 | value;
                }
                if !(0xdc00..=0xdfff).contains(&low) {
                    end -= 6;
                }
            }
        }
    }

    /// `JSONObject`.
    fn object(&self, mut end: usize, depth: usize) -> Result<usize, PyError> {
        let mut next = self.at(end);
        if next != Some('"') {
            if next.is_none_or(|c| PY_WHITESPACE.contains(&c)) {
                end = self.whitespace(end);
                next = self.at(end);
            }
            if next == Some('}') {
                return Ok(end + 1);
            }
            if next != Some('"') {
                return Self::decode("Expecting property name enclosed in double quotes", end);
            }
        }
        end += 1;
        loop {
            end = self.string(end)?;
            if self.at(end) != Some(':') {
                end = self.whitespace(end);
                if self.at(end) != Some(':') {
                    return Self::decode("Expecting ':' delimiter", end);
                }
            }
            end += 1;
            end = self.whitespace(end);
            end = match self.scan_once(end, depth) {
                Err(PyError::Stop(index)) => return Self::decode("Expecting value", index),
                other => other?,
            };
            let mut next = self.at(end);
            if next.is_some_and(|c| PY_WHITESPACE.contains(&c)) {
                end = self.whitespace(end + 1);
                next = self.at(end);
            }
            end += 1;
            match next {
                Some('}') => return Ok(end),
                Some(',') => {}
                _ => return Self::decode("Expecting ',' delimiter", end - 1),
            }
            let comma = end - 1;
            end = self.whitespace(end);
            let next = self.at(end);
            end += 1;
            if next != Some('"') {
                if next == Some('}') {
                    return Self::decode("Illegal trailing comma before end of object", comma);
                }
                return Self::decode("Expecting property name enclosed in double quotes", end - 1);
            }
        }
    }

    /// `JSONArray`.
    fn array(&self, mut end: usize, depth: usize) -> Result<usize, PyError> {
        let mut next = self.at(end);
        if next.is_some_and(|c| PY_WHITESPACE.contains(&c)) {
            end = self.whitespace(end + 1);
            next = self.at(end);
        }
        if next == Some(']') {
            return Ok(end + 1);
        }
        loop {
            end = match self.scan_once(end, depth) {
                Err(PyError::Stop(index)) => return Self::decode("Expecting value", index),
                other => other?,
            };
            let mut next = self.at(end);
            if next.is_some_and(|c| PY_WHITESPACE.contains(&c)) {
                end = self.whitespace(end + 1);
                next = self.at(end);
            }
            end += 1;
            match next {
                Some(']') => return Ok(end),
                Some(',') => {}
                _ => return Self::decode("Expecting ',' delimiter", end - 1),
            }
            let comma = end - 1;
            end = self.whitespace(end);
            if self.at(end) == Some(']') {
                return Self::decode("Illegal trailing comma before end of array", comma);
            }
        }
    }
}

#[cfg(test)]
mod python_decode_tests {
    #[test]
    fn decode_errors_match_upstreams_lenient_json_decoder() {
        // `str(e)` of `json.loads(TEXT, cls=LenientJSONDecoder, strict=False)`
        // (upstream fa98866 on CPython 3.15).
        for (text, expected) in [
            (
                "not found",
                Some("Expecting value in 'not found': line 1 column 1 (char 0)"),
            ),
            ("", Some("Expecting value in '': line 1 column 1 (char 0)")),
            (
                "{\"a\":1,}",
                Some(
                    "Illegal trailing comma before end of object in ':1,}': line 1 column 7 (char 6)",
                ),
            ),
            (
                "[1,]",
                Some(
                    "Illegal trailing comma before end of array in '[1,]': line 1 column 3 (char 2)",
                ),
            ),
            (
                "{\"a\" \"b\"}",
                Some("Expecting ':' delimiter in ' \"b\"}': line 1 column 6 (char 5)"),
            ),
            (
                "[1 2]",
                Some("Expecting ',' delimiter in '[1 2]': line 1 column 4 (char 3)"),
            ),
            (
                "{\"a\": \"\\q\"}",
                Some("Invalid \\escape in 'q\"}': line 1 column 8 (char 7)"),
            ),
            (
                "{\"\\u12g4\": 1}",
                Some("Invalid \\uXXXX escape in 'g4\": 1}': line 1 column 4 (char 3)"),
            ),
            (
                "\"abc",
                Some("Unterminated string starting at in '\"abc': line 1 column 1 (char 0)"),
            ),
            (
                "{\n  \"a\": \n}",
                Some("Expecting value in '{\\n  \"a\": \\n}': line 3 column 1 (char 10)"),
            ),
            (
                "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
                Some("Expecting value in '': line 1 column 1 (char 0)"),
            ),
            (
                "[1] x",
                Some("Extra data in '[1] x': line 1 column 5 (char 4)"),
            ),
            ("{\"a\":\"\u{0001}\"}", None),
            (
                "\"\\ud83d\\ude00\" 1",
                Some("Extra data in '3d\\\\ude00\" 1': line 1 column 16 (char 15)"),
            ),
            (
                "é{\"ü\": [1,,2]}\n\n",
                Some("Expecting value in ' [1,': line 1 column 1 (char 0)"),
            ),
        ] {
            assert_eq!(
                super::python_decode_error_text(text).as_deref(),
                expected,
                "{text:?}"
            );
        }
    }
}
