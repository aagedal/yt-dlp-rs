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
        n_entries: Option<usize>,
        last_playlist_index: usize,
    },
    /// Unsanitized `--print` rendering of a processed video, with upstream's
    /// prepared `filename` and newline-joined selected `urls`.
    Print {
        info: &'a VideoInfo,
        filename: std::result::Result<&'a str, &'a Error>,
        urls: &'a str,
        filepath: Option<&'a str>,
        prepared: Option<&'a str>,
    },
    /// Unsanitized `--exec` rendering with the stage's `filepath` and
    /// `_filename` values (when set).
    Exec {
        info: &'a VideoInfo,
        filepath: Option<&'a str>,
        filename: Option<&'a str>,
    },
}

impl Source<'_> {
    /// Unsanitized rendering, as `prepare_outtmpl` without `sanitize`.
    fn print(self) -> bool {
        matches!(self, Source::Print { .. } | Source::Exec { .. })
    }

    fn exec(self) -> bool {
        matches!(self, Source::Exec { .. })
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
    Float(f64),
    Boolean(bool),
    /// A traversed value: owned strings and Python objects (lists, dicts,
    /// negative integers).
    Owned(String),
    Json(serde_json::Value),
}

impl Value<'_> {
    fn from_json(value: serde_json::Value) -> Self {
        match value {
            serde_json::Value::String(text) => Value::Owned(text),
            serde_json::Value::Bool(value) => Value::Boolean(value),
            serde_json::Value::Number(number) if number.is_u64() => {
                Value::Integer(number.as_u64().unwrap_or_default())
            }
            serde_json::Value::Number(number) if number.is_f64() => {
                Value::Float(number.as_f64().unwrap_or_default())
            }
            other => Value::Json(other),
        }
    }

    fn text(&self) -> Option<&str> {
        match self {
            Value::Text(text) => Some(text),
            Value::Owned(text) => Some(text),
            _ => None,
        }
    }

    /// Python `str(value)`.
    fn py_str(&self) -> String {
        match self {
            Value::Text(text) => (*text).to_owned(),
            Value::Owned(text) => text.clone(),
            Value::Integer(n) => n.to_string(),
            Value::Float(n) => crate::screen::py_float(*n),
            Value::Boolean(b) => if *b { "True" } else { "False" }.into(),
            Value::Json(value) => super::traverse::py_str(value),
        }
    }
}

/// Upstream's info dict for traversal: the serialized metadata plus the
/// source's prepared extras.
fn traversal_root(source: Source<'_>) -> serde_json::Value {
    let insert = |mut value: serde_json::Value, extras: Vec<(&str, serde_json::Value)>| {
        if let Some(object) = value.as_object_mut() {
            for (key, extra) in extras {
                object.insert(key.into(), extra);
            }
        }
        value
    };
    let value = |info: &VideoInfo| serde_json::to_value(info).unwrap_or_default();
    match source {
        Source::Video(info) => value(info),
        Source::Print {
            info,
            filename,
            urls,
            filepath,
            prepared,
        } => insert(
            value(info),
            vec![
                ("filepath", filepath.into()),
                ("_filename", prepared.into()),
                ("filename", filename.ok().into()),
                ("urls", urls.into()),
            ],
        ),
        Source::Exec {
            info,
            filepath,
            filename,
        } => insert(
            value(info),
            vec![
                ("filepath", filepath.into()),
                ("_filename", filename.into()),
            ],
        ),
        Source::Playlist {
            info,
            playlist_count,
            n_entries,
            ..
        } => insert(
            serde_json::to_value(info).unwrap_or_default(),
            vec![
                ("playlist_index", 0.into()),
                ("playlist_count", playlist_count.into()),
                ("n_entries", n_entries.into()),
            ],
        ),
    }
}

/// Split alternates at commas outside `{...}` traversal groups.
fn alternates(names: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let (mut depth, mut start) = (0usize, 0);
    for (index, c) in names.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                parts.push(&names[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    parts.push(&names[start..]);
    parts
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
    "description",
    "duration",
    "channel_id",
    "channel_url",
    "uploader_id",
    "uploader_url",
    "view_count",
    "age_limit",
    "playable_in_embed",
    "live_status",
    "media_type",
    "upload_date",
    "timestamp",
    "availability",
    "format",
    "resolution",
    "protocol",
    "vcodec",
    "acodec",
    "dynamic_range",
    "video_ext",
    "audio_ext",
    "duration_string",
    "thumbnail",
    "aspect_ratio",
    "fps",
    "tbr",
    "abr",
    "vbr",
    "asr",
    "audio_channels",
    "fulltitle",
];

/// Whether a represented video field has a value.
pub(crate) fn has_value(info: &VideoInfo, name: &str) -> bool {
    matches!(field(name, Source::Video(info)), Ok(Some(_)))
}

fn field<'a>(name: &str, source: Source<'a>) -> Result<Option<Value<'a>>> {
    use Value::*;
    let info = match source {
        Source::Video(info) => info,
        Source::Print {
            info,
            filename,
            urls,
            filepath,
            prepared,
        } => match name {
            "filepath" => return Ok(filepath.map(Text)),
            "_filename" => return Ok(prepared.map(Text)),
            "filename" => {
                return filename
                    .map(|filename| Some(Text(filename)))
                    .map_err(|error| invalid(format!("output filename failed: {error}")));
            }
            "urls" => return Ok(Some(Text(urls))),
            _ => info,
        },
        Source::Exec {
            info,
            filepath,
            filename,
        } => match name {
            "filepath" => return Ok(filepath.map(Text)),
            "_filename" => return Ok(filename.map(Text)),
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
                "n_entries" => n_entries.map(|n| Integer(n as u64)),
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
        // Only generic direct links have a `direct` field upstream.
        "direct" => info.direct.map(Boolean),
        "description" => info.metadata.description.as_deref().map(Text),
        "duration" => info.metadata.duration.map(Integer),
        "channel_id" => info.metadata.channel_id.as_deref().map(Text),
        "channel_url" => info.metadata.channel_url.as_deref().map(Text),
        "uploader_id" => info.metadata.uploader_id.as_deref().map(Text),
        "uploader_url" => info.metadata.uploader_url.as_deref().map(Text),
        "view_count" => info.metadata.view_count.map(Integer),
        "age_limit" => info.metadata.age_limit.map(|n| Integer(n.into())),
        "playable_in_embed" => info.metadata.playable_in_embed.map(Boolean),
        "live_status" => info.metadata.live_status.as_deref().map(Text),
        "media_type" => info.metadata.media_type.as_deref().map(Text),
        "upload_date" => info.metadata.upload_date.as_deref().map(Text),
        // Negative timestamps are not representable by the typed renderer.
        "timestamp" => info
            .metadata
            .timestamp
            .and_then(|t| u64::try_from(t).ok())
            .map(Integer),
        "availability" => info.metadata.availability.as_deref().map(Text),
        "format" => info.format.as_deref().map(Text),
        "resolution" => info.resolution.as_deref().map(Text),
        "protocol" => info.protocol.as_deref().map(Text),
        "vcodec" => info.vcodec.as_deref().map(Text),
        "acodec" => info.acodec.as_deref().map(Text),
        "dynamic_range" => info.dynamic_range.as_deref().map(Text),
        "video_ext" => info.video_ext.as_deref().map(Text),
        "audio_ext" => info.audio_ext.as_deref().map(Text),
        "duration_string" => info.metadata.duration_string.as_deref().map(Text),
        "fulltitle" => info.fulltitle.as_deref().map(Text),
        "thumbnail" => info.metadata.thumbnail.as_deref().map(Text),
        "aspect_ratio" => info.aspect_ratio.map(Float),
        "fps" => info.fps.map(|fps| match fps {
            crate::info::PyNumber::Int(n) if n >= 0 => Integer(n as u64),
            other => Float(other.as_f64()),
        }),
        "tbr" => info.tbr.map(Float),
        "abr" => info.abr.map(Float),
        "vbr" => info.vbr.map(Float),
        "asr" => info.asr.map(|n| Integer(n.into())),
        "audio_channels" => info.audio_channels.map(|n| Integer(n.into())),
        _ => return Err(invalid(format!("unknown or unsupported field {name:?}"))),
    })
}

fn word(text: &str) -> bool {
    !text.is_empty() && text.chars().all(|c| c.is_alphanumeric() || c == '_')
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
    // A length modifier precedes a conversion; a bare `l` is the list one.
    if input.starts_with(['h', 'l', 'L'])
        && input[1..].starts_with(['s', 'd', 'i', 'u', 'f', 'F', 'j', 'q', 'l'])
    {
        input = &input[1..];
    }
    let Some(conversion) = input.chars().next() else {
        return Err(invalid("missing template conversion"));
    };
    if !matches!(
        conversion,
        's' | 'd' | 'i' | 'u' | 'f' | 'F' | 'j' | 'q' | 'l'
    ) {
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
    if format.conversion == 'q' && !info.exec() {
        return Err(invalid(
            "shell-quoted conversion is only supported for --exec",
        ));
    }
    let (names, fallback) = key
        .split_once('|')
        .map_or((key, None), |(a, b)| (a, Some(b)));
    let mut chosen = None;
    let mut last_field = "";
    let mut root = None;
    // Validate every alternate, including fields after an available first value.
    for name in alternates(names) {
        // Per-run counters come from the caller's options.
        let value = match name {
            "autonumber" => Ok(options.autonumber.get().map(Value::Integer)),
            "video_autonumber" => Ok(options.video_autonumber.get().map(Value::Integer)),
            _ if super::traverse::is_plain(name)
                && (FIELDS.contains(&name)
                    || matches!(name, "filename" | "_filename" | "filepath" | "urls")) =>
            {
                field(name, info)
            }
            // Upstream object traversal over the whole info dict; unknown
            // fields are missing values.
            _ if super::traverse::is_plain(name) && !word(name) => {
                Err(invalid(format!("unknown or unsupported field {name:?}")))
            }
            _ => super::traverse::traverse(root.get_or_insert_with(|| traversal_root(info)), name)
                .map(|value| value.map(Value::from_json)),
        }?;
        if chosen.is_none() {
            last_field = name;
            // Filename sanitization turns empty strings into missing values;
            // printing keeps them, so they do not select an alternate.
            chosen = value.filter(|value| print || value.text() != Some(""));
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
                Source::Video(info) | Source::Print { info, .. } | Source::Exec { info, .. } => {
                    info.last_playlist_index.unwrap_or(0)
                }
                Source::Playlist {
                    last_playlist_index,
                    ..
                } => last_playlist_index,
            }),
            "playlist_autonumber" => match info {
                Source::Video(info) | Source::Print { info, .. } | Source::Exec { info, .. } => {
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
        if last_field == "autonumber" {
            format.width = options.autonumber_size;
            format.zero = true;
            format.conversion = 'd';
        }
    }
    let result = if format.conversion == 'q' {
        // `shell_quote(map(str, variadic(value)))`; scalars ignore `#`.
        let value = value.py_str();
        text(&super::exec::shell_quote(bounded(&value)?), &format)
    } else if format.conversion == 'j' {
        let value = match value {
            Value::Text(s) => python_json_string(bounded(s)?, !format.plus),
            Value::Owned(s) => python_json_string(bounded(&s)?, !format.plus),
            Value::Integer(n) => n.to_string(),
            Value::Float(n) => crate::screen::py_float(n),
            Value::Boolean(b) => b.to_string(),
            Value::Json(value) if format.alternate => {
                super::traverse::py_json_indented(&value, !format.plus)
            }
            Value::Json(value) => super::traverse::py_json(&value, !format.plus),
        };
        // Upstream formats the JSON text with the remaining `%s` flags and
        // sanitizes it in filenames.
        let value = if print {
            bounded(&value)?.to_owned()
        } else {
            sanitize(&value, options)?
        };
        text(&value, &format)
    } else if matches!(format.conversion, 's' | 'l') {
        // `l`: `', '.join(map(str, variadic(value)))` (newlines with `#`).
        let value = match value {
            Value::Json(serde_json::Value::Array(items)) if format.conversion == 'l' => items
                .iter()
                .map(super::traverse::py_str)
                .collect::<Vec<_>>()
                .join(if format.alternate { "\n" } else { ", " }),
            other => other.py_str(),
        };
        let value = if print {
            bounded(&value)?.to_owned()
        } else {
            sanitize(&value, options)?
        };
        text(&value, &format)
    } else {
        let number = match value {
            Value::Integer(n) => Some(n as f64),
            Value::Float(n) => Some(n),
            Value::Boolean(b) => Some(u8::from(b) as f64),
            Value::Text(s) => number(s)?,
            Value::Owned(s) => number(&s)?,
            Value::Json(serde_json::Value::Number(n)) => n.as_f64(),
            Value::Json(_) => None,
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
pub(super) fn python_json_string(value: &str, ensure_ascii: bool) -> String {
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
    // Media `-` is stdout (the caller handles it); typed files derive from it.
    if output.is_empty() || (output == "-" && kind.is_none()) {
        return Err(invalid("empty paths and stdout output are unsupported"));
    }
    if let Some(ext) = kind.and_then(super::filename::forced_extension) {
        let real = match info {
            Source::Video(info) | Source::Print { info, .. } | Source::Exec { info, .. } => {
                Some(info.ext.as_str())
            }
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
            // `escape_outtmpl` keeps other `%` sequences literal in commands.
            if info.exec() {
                append(&mut output, "%")?;
                continue;
            }
            return Err(invalid("malformed output template token"));
        };
        let (value, consumed) = substitution(key, spec, info, options)?;
        append(&mut output, &value)?;
        rest = &spec[consumed..];
    }
    append(&mut output, rest)?;
    Ok(output)
}

/// Each keyed substitution's conversion character and `|default`, in
/// template order, for `--exec` safety validation. Other `%` sequences are
/// literal, as in command rendering.
pub(super) fn exec_fields(template: &str) -> Vec<(Option<char>, Option<&str>)> {
    let mut fields = Vec::new();
    let mut rest = template;
    while let Some(index) = rest.find('%') {
        rest = &rest[index + 1..];
        if let Some(tail) = rest.strip_prefix('%') {
            rest = tail;
            continue;
        }
        let Some((key, spec)) = rest.strip_prefix('(').and_then(|s| s.split_once(')')) else {
            continue;
        };
        let spec = spec.trim_start_matches(|c: char| "-0123456789.*# +".contains(c));
        let spec = spec.strip_prefix(['h', 'l', 'L']).unwrap_or(spec);
        let conversion = spec.chars().next();
        fields.push((conversion, key.split_once('|').map(|(_, default)| default)));
        rest = &spec[conversion.map_or(0, char::len_utf8)..];
    }
    fields
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
        // Unknown fields are missing values, like upstream.
        assert_eq!(output("%(unsupported,id)s", &info), "42");
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
            "%(id.a-b)s",
            "%(id.{x)s",
        ] {
            assert!(
                render(template, Source::Video(&info), &options).is_err(),
                "{template}"
            );
        }
        assert_eq!(output("%%-%(id)s", &info), "%-42");
        // `l` is a length modifier before a conversion, else the list one.
        assert_eq!(output("%(id)lls-%(id)ls-%(id)l", &info), "42s-42-42");
        assert!(render(&"x".repeat(MAX_SPEC + 1), Source::Video(&info), &options).is_err());
        assert!(render(&"%%".repeat(257), Source::Video(&info), &options).is_err());
        assert!(render(&"%(width)4096s".repeat(17), Source::Video(&info), &options).is_err());
        let mut huge = info.clone();
        huge.title = "x".repeat(MAX_OUTPUT + 1);
        assert!(render("%(title).1s", Source::Video(&huge), &options).is_err());
    }
}
