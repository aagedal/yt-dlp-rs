//! Bounded named printf templates for the represented root metadata fields.
use crate::{Error, Result, VideoInfo};
use std::path::PathBuf;

const MAX_SPEC: usize = 16 * 1024;
const MAX_OUTPUT: usize = 64 * 1024;
const MAX_FIELD: usize = 4096;

fn invalid(message: impl Into<String>) -> Error {
    Error::InvalidTemplate(message.into())
}

#[derive(Default)]
struct Format {
    alternate: bool,
    zero: bool,
    left: bool,
    plus: bool,
    space: bool,
    width: usize,
    precision: Option<usize>,
    conversion: char,
}

enum Value<'a> {
    Text(&'a str),
    Integer(u64),
    Boolean(bool),
}

fn field<'a>(name: &str, info: &'a VideoInfo) -> Result<Option<Value<'a>>> {
    use Value::*;
    Ok(match name {
        "id" => Some(Text(&info.id)),
        "title" => Some(Text(&info.title)),
        "ext" => Some(Text(&info.ext)),
        "extractor" => Some(Text(&info.extractor)),
        "extractor_key" => Some(Text(&info.extractor_key)),
        "format_id" => info.format_id.as_deref().map(Text),
        "url" => Some(Text(info.url.as_str())),
        "webpage_url" => Some(Text(info.webpage_url.as_str())),
        "width" => info.width.map(|n| Integer(n.into())),
        "height" => info.height.map(|n| Integer(n.into())),
        "filesize" => info.filesize.map(Integer),
        "playlist_index" => info.playlist_index.map(|n| Integer(n as u64)),
        "playlist_autonumber" => info.playlist_autonumber.map(|n| Integer(n as u64)),
        "playlist_count" => info.playlist_count.map(|n| Integer(n as u64)),
        "n_entries" => info.n_entries.map(|n| Integer(n as u64)),
        "direct" => Some(Boolean(info.direct)),
        _ => return Err(invalid(format!("unknown or unsupported field {name:?}"))),
    })
}

fn digits(input: &str) -> Result<(Option<usize>, &str)> {
    let count = input.bytes().take_while(u8::is_ascii_digit).count();
    if count == 0 {
        return Ok((None, input));
    }
    let value = input[..count]
        .parse::<usize>()
        .map_err(|_| invalid("format width/precision is too large"))?;
    if value > MAX_FIELD {
        return Err(invalid("format width/precision exceeds 4096"));
    }
    Ok((Some(value), &input[count..]))
}

fn format_spec(mut input: &str) -> Result<(Format, &str)> {
    let mut format = Format::default();
    loop {
        match input.as_bytes().first() {
            Some(b'#') => format.alternate = true,
            Some(b'0') => format.zero = true,
            Some(b'-') => format.left = true,
            Some(b'+') => format.plus = true,
            Some(b' ') => format.space = true,
            _ => break,
        }
        input = &input[1..];
    }
    let (width, rest) = digits(input)?;
    format.width = width.unwrap_or(0);
    input = rest;
    if let Some(rest) = input.strip_prefix('.') {
        let (precision, rest) = digits(rest)?;
        format.precision =
            Some(precision.ok_or_else(|| invalid("precision requires ASCII digits"))?);
        input = rest;
    }
    if input.starts_with(['h', 'l', 'L']) {
        input = &input[1..];
    }
    let Some(conversion) = input.chars().next() else {
        return Err(invalid("missing template conversion"));
    };
    if !matches!(conversion, 's' | 'd' | 'i' | 'u' | 'f' | 'F') {
        return Err(invalid(format!("unsupported conversion {conversion:?}")));
    }
    format.conversion = conversion;
    Ok((format, &input[conversion.len_utf8()..]))
}

fn padded(mut value: String, format: &Format, numeric: bool) -> String {
    let count = value.chars().count();
    if count >= format.width {
        return value;
    }
    let pad = format.width - count;
    if format.left {
        value.extend(std::iter::repeat_n(' ', pad));
        return value;
    }
    if numeric && format.zero {
        let sign = usize::from(value.starts_with(['+', '-', ' ']));
        value.insert_str(sign, &"0".repeat(pad));
        value
    } else {
        format!("{}{value}", " ".repeat(pad))
    }
}

fn text(value: &str, format: &Format) -> String {
    let value = match format.precision {
        Some(n) => value.chars().take(n).collect(),
        None => value.to_owned(),
    };
    padded(value, format, false)
}

fn number(value: &str) -> Result<Option<f64>> {
    if value.len() > MAX_OUTPUT {
        return Err(invalid("template metadata exceeds 64 KiB"));
    }
    let value = value.trim();
    if !value.contains('_') {
        return Ok(value.parse().ok());
    }
    let bytes = value.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'_'
            && (index == 0
                || !bytes[index - 1].is_ascii_digit()
                || !bytes.get(index + 1).is_some_and(u8::is_ascii_digit))
        {
            return Ok(None);
        }
    }
    Ok(value.replace('_', "").parse().ok())
}

fn numeric(number: f64, format: &Format) -> Result<String> {
    let integer = matches!(format.conversion, 'd' | 'i' | 'u');
    if integer && !number.is_finite() {
        return Err(invalid("nonfinite integer template conversion"));
    }
    let negative =
        number.is_sign_negative() && !number.is_nan() && (!integer || number.trunc() != 0.0);
    let magnitude = number.abs();
    let mut value = if integer {
        let mut value = format!("{:.0}", magnitude.trunc());
        if let Some(precision) = format.precision
            && value.len() < precision
        {
            value = format!("{}{value}", "0".repeat(precision - value.len()));
        }
        value
    } else if magnitude.is_nan() {
        "nan".into()
    } else if magnitude.is_infinite() {
        "inf".into()
    } else {
        let precision = format.precision.unwrap_or(6);
        let mut value = format!("{magnitude:.precision$}");
        if format.alternate && precision == 0 {
            value.push('.');
        }
        value
    };
    if format.conversion == 'F' {
        value.make_ascii_uppercase();
    }
    if negative {
        value.insert(0, '-');
    } else if format.plus {
        value.insert(0, '+');
    } else if format.space {
        value.insert(0, ' ');
    }
    Ok(padded(value, format, true))
}

fn substitution(
    key: &str,
    spec: &str,
    info: &VideoInfo,
    options: &super::TemplateOptions,
) -> Result<(String, usize)> {
    let (mut format, remaining) = format_spec(spec)?;
    let consumed = spec.len() - remaining.len();
    let (names, fallback) = key
        .split_once('|')
        .map_or((key, None), |(a, b)| (a, Some(b)));
    let mut chosen = None;
    let mut last_field = "";
    // Validate every alternate, including fields after an available first value.
    for name in names.split(',') {
        let value = field(name, info)?;
        if chosen.is_none() {
            last_field = name;
            chosen = value.filter(|value| !matches!(value, Value::Text("")));
        }
    }
    let default = || -> Result<String> {
        let value = fallback.unwrap_or(&options.na_placeholder);
        if value.is_empty() {
            Ok(String::new())
        } else {
            sanitize(value)
        }
    };
    let Some(value) = chosen else {
        return Ok((safe_substitution(default()?), consumed));
    };
    if &spec[..consumed] == "s" {
        let size = match last_field {
            "playlist_index" => Some(info.last_playlist_index.unwrap_or(0)),
            "playlist_autonumber" => Some(info.n_entries.unwrap_or(0)),
            _ => None,
        };
        if let Some(size) = size {
            format.width = size.to_string().len();
            format.zero = true;
            format.conversion = 'd';
        }
    }
    let result = if format.conversion == 's' {
        let value = match value {
            Value::Text(s) => sanitize(s)?,
            Value::Integer(n) => n.to_string(),
            Value::Boolean(b) => if b { "True" } else { "False" }.into(),
        };
        text(&value, &format)
    } else {
        let number = match value {
            Value::Integer(n) => Some(n as f64),
            Value::Boolean(b) => Some(u8::from(b) as f64),
            Value::Text(s) => number(s)?,
        };
        match number {
            Some(n) => numeric(n, &format)?,
            None => default()?,
        }
    };
    Ok((safe_substitution(result), consumed))
}

fn safe_substitution(value: String) -> String {
    // Precision can expose a dot/space-only component that the initial sanitizer
    // had removed. Preserve normal padding, but do not substitute path aliases.
    if !value.is_empty() && value.trim_matches([' ', '.']).is_empty() {
        "video".into()
    } else {
        value
    }
}

fn sanitize(value: &str) -> Result<String> {
    if value.len() > MAX_OUTPUT {
        return Err(invalid("template metadata/default exceeds 64 KiB"));
    }
    Ok(super::safe_component(value))
}

fn append(output: &mut String, value: &str) -> Result<()> {
    if output.len().saturating_add(value.len()) > MAX_OUTPUT {
        return Err(invalid("expanded output template exceeds 64 KiB"));
    }
    output.push_str(value);
    Ok(())
}

pub(super) fn render(
    template: &str,
    info: &VideoInfo,
    options: &super::TemplateOptions,
) -> Result<PathBuf> {
    if template.len() > MAX_SPEC {
        return Err(invalid("output template exceeds 16 KiB"));
    }
    let mut output = String::new();
    let mut rest = template;
    let mut tokens = 0;
    while let Some(index) = rest.find('%') {
        append(&mut output, &rest[..index])?;
        rest = &rest[index + 1..];
        tokens += 1;
        if tokens > 256 {
            return Err(invalid("output template exceeds 256 tokens"));
        }
        if let Some(tail) = rest.strip_prefix('%') {
            append(&mut output, "%")?;
            rest = tail;
            continue;
        }
        let Some((key, spec)) = rest.strip_prefix('(').and_then(|s| s.split_once(')')) else {
            return Err(invalid("malformed output template token"));
        };
        let (value, consumed) = substitution(key, spec, info, options)?;
        append(&mut output, &value)?;
        rest = &spec[consumed..];
    }
    append(&mut output, rest)?;
    if output.is_empty() || output == "-" {
        return Err(invalid("empty paths and stdout output are unsupported"));
    }
    Ok(output.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info() -> VideoInfo {
        serde_json::from_value(json!({"id":"42","title":"Épisode 影像","ext":"mp4","extractor":"native","extractor_key":"Native","url":"https://example.com/m.mp4","webpage_url":"https://example.com/watch","direct":true,"filesize":9007199254740993_u64,"width":1280,"height":720,"playlist_index":3,"playlist_autonumber":2,"n_entries":120,"__last_playlist_index":1000})).unwrap()
    }
    fn output(template: &str, info: &VideoInfo) -> String {
        render(template, info, &super::super::TemplateOptions::default())
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn typed_values_flags_precision_and_automatic_padding() {
        let info = info();
        assert_eq!(
            output("%(width)+08d-%(height)-6i.%(ext)s", &info),
            "+0001280-720   .mp4"
        );
        assert_eq!(output("%(id)#08.2f-%(height) .0F", &info), "00042.00- 720");
        assert_eq!(
            output("%(filesize)s-%(filesize)d", &info),
            "9007199254740993-9007199254740992"
        );
        assert_eq!(
            output("%(title)12.4s-%(direct)s-%(direct)u", &info),
            "        Épis-True-1"
        );
        assert_eq!(
            output(
                "%(playlist_index)s-%(playlist_autonumber)s-%(playlist_index)1s",
                &info
            ),
            "0003-002-3"
        );
        assert_eq!(output("%(height)#.0f-%(width)ld", &info), "720.-1280");
    }

    #[test]
    fn all_alternates_validate_missing_and_numeric_failures_discard_formatting() {
        let mut info = info();
        info.title.clear();
        assert_eq!(output("%(title,format_id,id|missing)s", &info), "42");
        assert_eq!(output("%(format_id)020.9d", &info), "NA");
        assert_eq!(output("%(extractor|fallback)020d", &info), "fallback");
        assert_eq!(output("%(format_id|)04s-suffix", &info), "-suffix");
        assert!(
            render(
                "%(id,unsupported)s",
                &info,
                &super::super::TemplateOptions::default()
            )
            .is_err()
        );
        assert_eq!(output("%(format_id,playlist_index)s", &info), "0003");
        let options = super::super::TemplateOptions {
            na_placeholder: String::new(),
        };
        assert_eq!(
            render("%(format_id)s-file", &info, &options).unwrap(),
            PathBuf::from("-file")
        );
    }

    #[test]
    fn metadata_and_fallback_are_sanitized_without_reparsing_or_creating_directories() {
        let mut info = info();
        info.title = "../../bad\\name\n%(id)s".into();
        assert_eq!(
            output("directory/%(title)s.%(ext)s", &info),
            "directory/_.._bad_name_%(id)s.mp4"
        );
        assert_eq!(output("%(format_id|../../bad)s", &info), "_.._bad");
        info.title = ". .x".into();
        assert_eq!(output("%(title).2s-file", &info), "video-file");
        assert_eq!(output("%(title).0s-file", &info), "-file");
    }

    #[test]
    fn nonfinite_integer_errors_float_strings_and_negative_zero() {
        let mut info = info();
        info.title = "-nan".into();
        assert_eq!(output("%(title)+F", &info), "+NAN");
        assert!(
            render(
                "%(title)d",
                &info,
                &super::super::TemplateOptions::default()
            )
            .is_err()
        );
        info.title = "-0.1".into();
        assert_eq!(output("%(title)d-%(title).1f", &info), "0--0.1");
        info.title = " 1_000.25 ".into();
        assert_eq!(output("%(title).2f", &info), "1000.25");
        info.title = "1__0".into();
        assert_eq!(output("%(title)010d", &info), "NA");
        info.title = "inf".into();
        assert_eq!(output("%(title)08f", &info), "00000inf");
    }

    #[test]
    fn saved_pinned_template_cases_match_actual_upstream() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/templates/cases.json"))
                .unwrap();
        let mut failures = Vec::new();
        for case in fixture["cases"].as_array().unwrap() {
            let info: VideoInfo = serde_json::from_value(case["info"].clone()).unwrap();
            let options = super::super::TemplateOptions {
                na_placeholder: case["na_placeholder"].as_str().unwrap().into(),
            };
            match render(case["template"].as_str().unwrap(), &info, &options) {
                Ok(actual) if actual.to_string_lossy() == case["expected"].as_str().unwrap() => {}
                actual => failures.push(format!(
                    "{}: {actual:?}, expected {}",
                    case["name"], case["expected"]
                )),
            }
        }
        assert!(failures.is_empty(), "{}", failures.join("\n"));
    }

    #[test]
    fn malformed_unsupported_and_resource_limits_fail_closed() {
        let info = info();
        let options = super::super::TemplateOptions::default();
        for template in [
            "",
            "-",
            "%",
            "%s",
            "%(id)",
            "%(id)q",
            "%(id).s",
            "%(id).d",
            "%(id)4097s",
            "%(id).4097f",
            "%(id)999999999999999999999999999999d",
            "%(id+1)s",
            "%(id,)s",
            "%(id,missing)s",
            "%(id)lls",
        ] {
            assert!(render(template, &info, &options).is_err(), "{template}");
        }
        assert_eq!(output("%%-%(id)s", &info), "%-42");
        assert!(render(&"x".repeat(MAX_SPEC + 1), &info, &options).is_err());
        assert!(render(&"%%".repeat(257), &info, &options).is_err());
        assert!(render(&"%(width)4096s".repeat(17), &info, &options).is_err());
        let mut huge = info.clone();
        huge.title = "x".repeat(MAX_OUTPUT + 1);
        assert!(render("%(title).1s", &huge, &options).is_err());
    }
}
