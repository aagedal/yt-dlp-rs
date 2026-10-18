//! Bounded named printf templates for the represented root metadata fields.
use crate::{Error, PlaylistInfo, Result, VideoInfo};
use std::path::PathBuf;

/// Metadata available to a template: a video, or a playlist with the
/// upstream playlist-file extras (`playlist_index` 0 and selected counts).
#[derive(Clone, Copy)]
pub(super) enum Source<'a> {
    Video(&'a VideoInfo),
    Playlist {
        info: &'a PlaylistInfo,
        playlist_count: usize,
        n_entries: usize,
        last_playlist_index: usize,
    },
    /// Unsanitized `--print` rendering of a processed video, with upstream's
    /// prepared `filename` and newline-joined selected `urls`.
    Print {
        info: &'a VideoInfo,
        filename: std::result::Result<&'a str, &'a Error>,
        urls: &'a str,
    },
}

impl Source<'_> {
    fn print(self) -> bool {
        matches!(self, Source::Print { .. })
    }
}

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

/// Every represented root field; playlists expose a subset.
const FIELDS: &[&str] = &[
    "id",
    "title",
    "ext",
    "extractor",
    "extractor_key",
    "format_id",
    "url",
    "webpage_url",
    "width",
    "height",
    "filesize",
    "playlist_index",
    "playlist_autonumber",
    "playlist_count",
    "n_entries",
    "direct",
];

fn field<'a>(name: &str, source: Source<'a>) -> Result<Option<Value<'a>>> {
    use Value::*;
    let info = match source {
        Source::Video(info) => info,
        Source::Print {
            info,
            filename,
            urls,
        } => match name {
            "filename" => {
                return filename
                    .map(|filename| Some(Text(filename)))
                    .map_err(|error| invalid(format!("output filename failed: {error}")));
            }
            "urls" => return Ok(Some(Text(urls))),
            _ => info,
        },
        Source::Playlist {
            info,
            playlist_count,
            n_entries,
            ..
        } => {
            // Fields absent from upstream's playlist dict fall back to defaults,
            // while unknown names still fail as they do for videos.
            if !FIELDS.contains(&name) {
                return Err(invalid(format!("unknown or unsupported field {name:?}")));
            }
            return Ok(match name {
                "id" => Some(Text(&info.id)),
                "title" => Some(Text(&info.title)),
                "extractor" => Some(Text(&info.extractor)),
                "extractor_key" => Some(Text(&info.extractor_key)),
                "webpage_url" => Some(Text(info.webpage_url.as_str())),
                "playlist_index" => Some(Integer(0)),
                "playlist_count" => Some(Integer(playlist_count as u64)),
                "n_entries" => Some(Integer(n_entries as u64)),
                _ => None,
            });
        }
    };
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
    if !matches!(conversion, 's' | 'd' | 'i' | 'u' | 'f' | 'F' | 'j') {
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
    info: Source<'_>,
    options: &super::TemplateOptions,
) -> Result<(String, usize)> {
    let (mut format, remaining) = format_spec(spec)?;
    let consumed = spec.len() - remaining.len();
    let print = info.print();
    if format.conversion == 'j' && !print {
        return Err(invalid("JSON conversion is only supported for printing"));
    }
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
            // Filename sanitization turns empty strings into missing values;
            // printing keeps them, so they do not select an alternate.
            chosen = value.filter(|value| print || !matches!(value, Value::Text("")));
        }
    }
    let default = || -> Result<String> {
        let value = fallback.unwrap_or(&options.na_placeholder);
        if value.is_empty() {
            Ok(String::new())
        } else if print {
            bounded(value).map(str::to_owned)
        } else {
            sanitize(value, options)
        }
    };
    let Some(value) = chosen else {
        return Ok((default()?, consumed));
    };
    if &spec[..consumed] == "s" {
        let size = match last_field {
            "playlist_index" => Some(match info {
                Source::Video(info) | Source::Print { info, .. } => {
                    info.last_playlist_index.unwrap_or(0)
                }
                Source::Playlist {
                    last_playlist_index,
                    ..
                } => last_playlist_index,
            }),
            "playlist_autonumber" => match info {
                Source::Video(info) | Source::Print { info, .. } => {
                    Some(info.n_entries.unwrap_or(0))
                }
                // Unavailable for playlists, so the default path above applies.
                Source::Playlist { .. } => None,
            },
            _ => None,
        };
        if let Some(size) = size {
            format.width = size.to_string().len();
            format.zero = true;
            format.conversion = 'd';
        }
    }
    let result = if format.conversion == 'j' {
        let value = match value {
            Value::Text(s) => python_json_string(bounded(s)?, !format.plus),
            Value::Integer(n) => n.to_string(),
            Value::Boolean(b) => b.to_string(),
        };
        // Upstream formats the JSON text with the remaining `%s` flags.
        text(&value, &format)
    } else if format.conversion == 's' {
        let value = match value {
            Value::Text(s) if print => bounded(s)?.to_owned(),
            Value::Text(s) => sanitize(s, options)?,
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
    Ok((result, consumed))
}

/// Native containment guard: metadata cannot form a `.`/`..` (or, for Windows,
/// dot/space-only) path component that is not literally in the template.
/// Substitutions cannot contain `/`, so components align with the template's.
fn guard_components(output: &str, template: &str) -> String {
    let literal: Vec<&str> = template.split('/').collect();
    output
        .split('/')
        .enumerate()
        .map(|(index, component)| {
            let alias = !component.is_empty() && component.trim_matches([' ', '.']).is_empty();
            if alias && literal.get(index) != Some(&component) {
                "_"
            } else {
                component
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn bounded(value: &str) -> Result<&str> {
    if value.len() > MAX_OUTPUT {
        return Err(invalid("template metadata/default exceeds 64 KiB"));
    }
    Ok(value)
}

fn sanitize(value: &str, options: &super::TemplateOptions) -> Result<String> {
    Ok(super::sanitize::sanitize_component(
        bounded(value)?,
        options.filename_mode,
    ))
}

/// `_prepare_filename` `trim_file_name`: keep up to two dot-separated
/// extensions and slice the remaining text like Python `no_ext[:length]`.
fn trim_file_name(filename: &str, length: i64) -> String {
    if length == 0 {
        return filename.to_owned();
    }
    let mut parts: Vec<&str> = filename.rsplitn(3, '.').collect();
    parts.reverse();
    let (no_ext, extensions) = parts.split_first().unwrap();
    let count = no_ext.chars().count() as i64;
    let keep = if length < 0 {
        (count + length).max(0)
    } else {
        length.min(count)
    };
    let trimmed: String = no_ext.chars().take(keep as usize).collect();
    std::iter::once(trimmed.as_str())
        .chain(extensions.iter().copied())
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(".")
}

/// Python `json.dumps` for a string, with or without `ensure_ascii`.
fn python_json_string(value: &str, ensure_ascii: bool) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for c in value.chars() {
        match c {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            '\u{8}' => output.push_str("\\b"),
            '\u{c}' => output.push_str("\\f"),
            c if c < ' ' || (ensure_ascii && c > '~') => {
                let mut units = [0; 2];
                for unit in c.encode_utf16(&mut units) {
                    output.push_str(&format!("\\u{unit:04x}"));
                }
            }
            c => output.push(c),
        }
    }
    output.push('"');
    output
}

fn append(output: &mut String, value: &str) -> Result<()> {
    if output.len().saturating_add(value.len()) > MAX_OUTPUT {
        return Err(invalid("expanded output template exceeds 64 KiB"));
    }
    output.push_str(value);
    Ok(())
}

/// Render an explicit filename template (upstream `prepare_filename(outtmpl=...)`).
pub(super) fn render(
    template: &str,
    info: Source<'_>,
    options: &super::TemplateOptions,
) -> Result<PathBuf> {
    render_kind(template, info, options, None)
}

/// Upstream `prepare_filename`: `kind` is an output type (`None` for media or
/// an explicit template). Expands `~`/`$VAR` in the template, renders sanitized
/// metadata, forces the type extension, trims, guards metadata path components,
/// joins `--paths` and optionally forces Windows path sanitization.
pub(super) fn render_kind(
    template: &str,
    info: Source<'_>,
    options: &super::TemplateOptions,
    kind: Option<&str>,
) -> Result<PathBuf> {
    let template = crate::batch::expand_path_text(template);
    let mut output = expand(&template, info, options)?;
    if output.is_empty() || output == "-" {
        return Err(invalid("empty paths and stdout output are unsupported"));
    }
    if let Some(ext) = kind.and_then(super::filename::forced_extension) {
        let real = match info {
            Source::Video(info) | Source::Print { info, .. } => Some(info.ext.as_str()),
            Source::Playlist { .. } => None,
        };
        output = super::filename::replace_extension(&output, ext, real);
    }
    let output = guard_components(&trim_file_name(&output, options.trim_file_name), &template);
    if output.is_empty() {
        return Err(invalid("empty paths and stdout output are unsupported"));
    }
    let output = super::filename::output_path(options, kind, &output);
    // Like upstream get_output_path, forced Windows names sanitize the whole path.
    let output = if options.windows_filenames {
        super::sanitize::force_sanitize_path(&output)
    } else {
        output
    };
    Ok(output.into())
}

/// Expand a template to text: unsanitized for `Source::Print`, otherwise
/// with filename-component sanitization.
pub(super) fn expand(
    template: &str,
    info: Source<'_>,
    options: &super::TemplateOptions,
) -> Result<String> {
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
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info() -> VideoInfo {
        serde_json::from_value(json!({"id":"42","title":"Épisode 影像","ext":"mp4","extractor":"native","extractor_key":"Native","url":"https://example.com/m.mp4","webpage_url":"https://example.com/watch","direct":true,"filesize":9007199254740993_u64,"width":1280,"height":720,"playlist_index":3,"playlist_autonumber":2,"n_entries":120,"__last_playlist_index":1000})).unwrap()
    }
    fn output(template: &str, info: &VideoInfo) -> String {
        render(
            template,
            Source::Video(info),
            &super::super::TemplateOptions::default(),
        )
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
                Source::Video(&info),
                &super::super::TemplateOptions::default()
            )
            .is_err()
        );
        assert_eq!(output("%(format_id,playlist_index)s", &info), "0003");
        let options = super::super::TemplateOptions {
            na_placeholder: String::new(),
            ..Default::default()
        };
        assert_eq!(
            render("%(format_id)s-file", Source::Video(&info), &options).unwrap(),
            PathBuf::from("-file")
        );
    }

    #[test]
    fn metadata_and_fallback_are_sanitized_without_reparsing_or_creating_directories() {
        let mut info = info();
        info.title = "../../bad\\name\n%(id)s".into();
        assert_eq!(
            output("directory/%(title)s.%(ext)s", &info),
            "directory/..⧸..⧸bad⧹name %(id)s.mp4"
        );
        assert_eq!(output("%(format_id|../../bad)s", &info), "..⧸..⧸bad");
        info.title = ". .x".into();
        assert_eq!(output("%(title).2s-file", &info), ". -file");
        assert_eq!(output("%(title).2s/%(title).1s/./x", &info), "_/_/./x");
        assert_eq!(output("%(title).0s-file", &info), "-file");
    }

    #[test]
    fn filename_modes_trimming_and_forced_windows_paths() {
        use super::super::{FilenameMode, TemplateOptions};
        let mut info = info();
        info.title = "Épisode: a/b <c>".into();
        let render_with = |template: &str, options: &TemplateOptions| {
            render(template, Source::Video(&info), options)
                .unwrap()
                .to_string_lossy()
                .into_owned()
        };
        let mut options = TemplateOptions {
            filename_mode: FilenameMode::Restricted,
            ..Default::default()
        };
        assert_eq!(
            render_with("%(title)s.%(ext)s", &options),
            "Episode_-_a_b_c.mp4"
        );
        options.filename_mode = FilenameMode::Minimal;
        assert_eq!(
            render_with("%(title)s.%(ext)s", &options),
            "Épisode: a⧸b <c>.mp4"
        );
        options.windows_filenames = true;
        assert_eq!(
            render_with("x/./%(title)s.%(ext)s", &options),
            "x/Épisode# a⧸b #c#.mp4"
        );
        options = TemplateOptions {
            trim_file_name: 3,
            ..Default::default()
        };
        assert_eq!(
            render_with("%(title)s.%(id)s.%(ext)s", &options),
            "Épi.42.mp4"
        );
        options.trim_file_name = -2;
        assert_eq!(render_with("abcd.%(ext)s", &options), "ab.mp4");
        options.trim_file_name = 2;
        assert_eq!(render_with("abcd", &options), "ab");
        assert_eq!(
            super::super::default_template(true),
            "%(title)s-[%(id)s].%(ext)s"
        );
    }

    #[test]
    fn nonfinite_integer_errors_float_strings_and_negative_zero() {
        let mut info = info();
        info.title = "-nan".into();
        assert_eq!(output("%(title)+F", &info), "+NAN");
        assert!(
            render(
                "%(title)d",
                Source::Video(&info),
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
                ..Default::default()
            };
            match render(
                case["template"].as_str().unwrap(),
                Source::Video(&info),
                &options,
            ) {
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
            assert!(
                render(template, Source::Video(&info), &options).is_err(),
                "{template}"
            );
        }
        assert_eq!(output("%%-%(id)s", &info), "%-42");
        assert!(render(&"x".repeat(MAX_SPEC + 1), Source::Video(&info), &options).is_err());
        assert!(render(&"%%".repeat(257), Source::Video(&info), &options).is_err());
        assert!(render(&"%(width)4096s".repeat(17), Source::Video(&info), &options).is_err());
        let mut huge = info.clone();
        huge.title = "x".repeat(MAX_OUTPUT + 1);
        assert!(render("%(title).1s", Source::Video(&huge), &options).is_err());
    }
}
