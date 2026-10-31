//! Upstream `yt_dlp/extractor/rai.py`: `RaiBaseIE`'s relinker XML (HLS and
//! HTTPS formats, the MP4 availability check) and the classes built on it:
//! `RaiIE` (rai.tv), `RaiNewsIE` / `RaiCulturaIE` (their player data, with
//! the `RaiIE` fallback), `RaiPlayIE` / `RaiPlayLiveIE`, `RaiPlaySoundIE` /
//! `RaiPlaySoundLiveIE`, the `RaiPlayPlaylistIE` and
//! `RaiPlaySoundPlaylistIE` program playlists (URL entries) and
//! `RaiSudtirolIE`. Not ported: the `f4m` relinker format and the
//! `_EMBED_REGEX` iframe hand-off of the news classes.
use super::{Extractor, dict_format, dict_info, download_json_note, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

pub struct Rai;
pub struct RaiCultura;
pub struct RaiNews;
pub struct RaiPlay;
pub struct RaiPlayLive;
pub struct RaiPlayPlaylist;
pub struct RaiPlaySound;
pub struct RaiPlaySoundLive;
pub struct RaiPlaySoundPlaylist;
pub struct RaiSudtirol;

const UUID: &str = r"[\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12}";
/// `RaiBaseIE._GEO_COUNTRIES` (`_GEO_BYPASS = False`).
const GEO: &[&str] = &["IT"];
const RAI_NAME: &str = "Rai";
const RAICULTURA_NAME: &str = "RaiCultura";
const RAINEWS_NAME: &str = "RaiNews";
const RAIPLAY_NAME: &str = "RaiPlay";
const RAIPLAYLIVE_NAME: &str = "RaiPlayLive";
const RAIPLAYPLAYLIST_NAME: &str = "RaiPlayPlaylist";
const RAIPLAYSOUND_NAME: &str = "RaiPlaySound";
const RAIPLAYSOUNDLIVE_NAME: &str = "RaiPlaySoundLive";
const RAIPLAYSOUNDPLAYLIST_NAME: &str = "RaiPlaySoundPlaylist";
const RAISUDTIROL_NAME: &str = "RaiSudtirol";

static RAI_VALID: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^https?://[^/]+\.(?:rai\.(?:it|tv))/.+?-(?P<id>{UUID})(?:-.+?)?\.html"
    ))
    .unwrap()
});
// `(?!articoli)` is checked by `news_match`: the regex crate has no lookahead.
static RAINEWS_VALID: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^https?://(?:www\.)?rainews\.it/(?P<path>[^?#]+)-(?P<id>{UUID})(?:-[^/?#]+)?\.html"
    ))
    .unwrap()
});
static RAICULTURA_VALID: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^https?://(?:www\.)?raicultura\.it/(?P<path>[^?#]+)-(?P<id>{UUID})(?:-[^/?#]+)?\.html"
    ))
    .unwrap()
});
static RAIPLAY_VALID: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^(?P<base>https?://(?:www\.)?raiplay\.it/.+?-(?P<id>{UUID}))\.(?:html|json)"
    ))
    .unwrap()
});
static RAIPLAYLIVE_VALID: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?P<base>https?://(?:www\.)?raiplay\.it/dirette/(?P<id>[^/?#&]+))",
    )
    .unwrap()
});
static RAIPLAYPLAYLIST_VALID: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?P<base>https?://(?:www\.)?raiplay\.it/programmi/(?P<id>[^/?#&]+))(?:/(?P<extra_id>[^?#&]+))?",
    )
    .unwrap()
});
static RAIPLAYSOUND_VALID: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^(?P<base>https?://(?:www\.)?raiplaysound\.it/.+?-(?P<id>{UUID}))\.(?:html|json)"
    ))
    .unwrap()
});
static RAIPLAYSOUNDLIVE_VALID: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^(?P<base>https?://(?:www\.)?raiplaysound\.it/(?P<id>[^/?#&]+)$)")
        .unwrap()
});
static RAIPLAYSOUNDPLAYLIST_VALID: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?P<base>https?://(?:www\.)?raiplaysound\.it/(?:programmi|playlist|audiolibri)/(?P<id>[^/?#&]+))(?:/(?P<extra_id>[^?#&]+))?",
    )
    .unwrap()
});
static RAISUDTIROL_VALID: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://rai(?:bz|sudtirol)\.rai\.it/.+media=(?P<id>\w+)")
        .unwrap()
});

static MEDIA_QUALITY_TBR: [(i64, [i64; 2]); 13] = [
    (250, [352, 198]),
    (400, [512, 288]),
    (600, [512, 288]),
    (700, [512, 288]),
    (800, [700, 394]),
    (1200, [736, 414]),
    (1500, [920, 518]),
    (1800, [1024, 576]),
    (2400, [1280, 720]),
    (3200, [1440, 810]),
    (3600, [1440, 810]),
    (5000, [1920, 1080]),
    (10000, [1920, 1080]),
];
static MANIFEST_REG: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"/(?P<id>\w+)(?:_(?P<quality>[\d\,]+))?(?:\.mp4)?(?:\.csmil)?/playlist\.m3u8",
    )
    .unwrap()
});
static CHUNKLIST_AO: LazyLock<Regex> =
    LazyLock::new(|| crate::extractor::url_regex(r"chunklist(?:_b\d+)*_ao[_.]").unwrap());
static CHUNKLIST_VO: LazyLock<Regex> =
    LazyLock::new(|| crate::extractor::url_regex(r"chunklist(?:_b\d+)*_vo[_.]").unwrap());
static CDATA_AFTER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\]\]>)[\r\n\t]+(</)").unwrap());
static CDATA_BEFORE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(>)[\r\n\t]+(<!\[CDATA\[)").unwrap());
static NEWS_PLAYER_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<rainews-player\s*data='").unwrap());
static CULTURA_PLAYER_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<raicultura-player\s*data='").unwrap());
static SUDTIROL_DATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<span class="med_data">(.+?)</span>"#).unwrap());
static SUDTIROL_TITLES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"<span class="med_title">(.+?)</span>"#).unwrap(),
        Regex::new(r"title: '(.+?)',").unwrap(),
    ]
});
static SUDTIROL_URLS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"sources:\s*\[\{file:\s*"(.+?)"\}\]"#).unwrap(),
        Regex::new(r#"<source\s+src="(.+?)"\s+type="application/x-mpegURL""#).unwrap(),
    ]
});
static SUDTIROL_THUMB: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"image: '(.+?)'").unwrap());

/// `suitable` for `RaiIE`.
pub fn rai_suitable(url: &str) -> bool {
    RAI_VALID.is_match(url)
}

/// `suitable` for `RaiNewsIE`: a path not starting with `articoli`.
pub fn rainews_suitable(url: &str) -> bool {
    news_match(&RAINEWS_VALID, url).is_some()
}

/// `suitable` for `RaiCulturaIE`.
pub fn raicultura_suitable(url: &str) -> bool {
    news_match(&RAICULTURA_VALID, url).is_some()
}

/// `suitable` for `RaiPlayIE`.
pub fn raiplay_suitable(url: &str) -> bool {
    RAIPLAY_VALID.is_match(url)
}

/// `suitable` for `RaiPlayLiveIE`.
pub fn raiplaylive_suitable(url: &str) -> bool {
    RAIPLAYLIVE_VALID.is_match(url)
}

/// `suitable` for `RaiPlayPlaylistIE`.
pub fn raiplayplaylist_suitable(url: &str) -> bool {
    RAIPLAYPLAYLIST_VALID.is_match(url)
}

/// `suitable` for `RaiPlaySoundIE`.
pub fn raiplaysound_suitable(url: &str) -> bool {
    RAIPLAYSOUND_VALID.is_match(url)
}

/// `suitable` for `RaiPlaySoundLiveIE`.
pub fn raiplaysoundlive_suitable(url: &str) -> bool {
    RAIPLAYSOUNDLIVE_VALID.is_match(url)
}

/// `suitable` for `RaiPlaySoundPlaylistIE`.
pub fn raiplaysoundplaylist_suitable(url: &str) -> bool {
    RAIPLAYSOUNDPLAYLIST_VALID.is_match(url)
}

/// `suitable` for `RaiSudtirolIE`.
pub fn raisudtirol_suitable(url: &str) -> bool {
    RAISUDTIROL_VALID.is_match(url)
}

/// The match of a news URL's `(?!articoli)` path.
fn news_match<'a>(valid: &'a Regex, url: &'a str) -> Option<regex::Captures<'a>> {
    let found = valid.captures(url)?;
    (!found["path"].starts_with("articoli")).then_some(found)
}

fn begin(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn parse(url: &str) -> Result<Url> {
    Ok(Url::parse(url)?)
}

/// A `KeyError` (upstream's untagged error text).
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Rai {what} are not supported natively"))
}

/// Python's truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(super::nuxt::truthy)
}

fn str_value(value: Option<&Value>) -> Option<&str> {
    value.and_then(Value::as_str)
}

/// `traverse_obj(obj, (key1, key2))`: a nested lookup, with numeric keys
/// indexing arrays.
fn path<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().try_fold(value, |current, key| match current {
        Value::Array(items) => key.parse::<usize>().ok().and_then(|index| items.get(index)),
        _ => current.get(*key),
    })
}

/// Python's `a or b` over optional values, the first truthy one.
fn first_truthy(values: &[Option<&Value>]) -> Option<Value> {
    values
        .iter()
        .flatten()
        .find(|value| super::nuxt::truthy(value))
        .map(|value| (*value).clone())
}

/// `strip_or_none`.
fn strip_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => {
            let text = text.trim();
            if text.is_empty() {
                Value::Null
            } else {
                text.into()
            }
        }
        _ => Value::Null,
    }
}

/// `int_or_none`.
fn int_or_none(value: Option<&Value>) -> Value {
    let number = match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse().ok(),
        _ => None,
    };
    number.map_or(Value::Null, Value::from)
}

/// `int_or_none` of a text (the relinker's XML).
fn int_text(text: Option<&str>) -> Option<i64> {
    text.and_then(|text| text.trim().parse().ok())
}

/// `join_nonempty(*parts, delim=...)`: falsy parts are left out.
fn join_nonempty(parts: &[Option<String>], delim: &str) -> String {
    parts
        .iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .cloned()
        .collect::<Vec<_>>()
        .join(delim)
}

/// `remove_start(text, prefix)`.
fn remove_start<'a>(text: &'a str, prefix: &str) -> &'a str {
    text.strip_prefix(prefix).unwrap_or(text)
}

/// `parse_duration(value)` for a JSON field.
fn duration_value(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => crate::parse_duration(text)
            .and_then(|number| serde_json::to_value(number).ok())
            .unwrap_or(Value::Null),
        Some(Value::Number(number)) => Value::Number(number.clone()),
        _ => Value::Null,
    }
}

/// `unified_timestamp(value)` for a text.
fn timestamp_value(date: Option<&str>) -> Value {
    date.and_then(|date| crate::dates::unified_timestamp(date, true))
        .map_or(Value::Null, Value::from)
}

/// `unified_strdate(value)` for a text.
fn strdate_value(date: Option<&str>) -> Value {
    date.and_then(|date| crate::dates::unified_strdate(date, true))
        .map_or(Value::Null, Value::from)
}

/// `urljoin(base, value)` for a JSON string (`None` for others).
fn join_value(base: &Url, value: &str) -> String {
    // `urljoin` keeps an absolute URL as written (the `url` crate would
    // percent-encode its spaces).
    if value.starts_with("http://") || value.starts_with("https://") {
        return value.to_owned();
    }
    base.join(value)
        .map_or_else(|_| value.to_owned(), |url| url.to_string())
}

/// The subtitle tracks' keys in upstream's order (`ext`, `url`).
fn ordered_subtitles(info: VideoInfo) -> VideoInfo {
    let shape = info
        .extra
        .get(crate::info_order::INFO_SHAPE)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    crate::info_order::mark_info(info, &format!("{shape};subtitle=ext,url"))
}

/// `_get_thumbnails_list`: every truthy value of the mapping.
fn thumbnails(thumbs: Option<&Value>, base: &Url) -> Value {
    let mut out = Vec::new();
    if let Some(Value::Object(map)) = thumbs {
        for value in map.values() {
            if let Value::String(thumb) = value
                && !thumb.is_empty()
            {
                out.push(json!({ "url": join_value(base, thumb) }));
            }
        }
    }
    Value::Array(out)
}

/// `_extract_subtitles(url, video_data)`.
fn subtitles(base: &Url, video: &Value) -> Value {
    let mut array: Vec<Value> = ["subtitlesArray", "subtitleList"]
        .iter()
        .find_map(|key| video.get(*key).filter(|value| super::nuxt::truthy(value)))
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    for key in ["subtitles", "subtitlesUrl"] {
        array.push(json!({ "url": video.get(key).cloned().unwrap_or(Value::Null) }));
    }
    let mut out: Vec<(String, Vec<Value>)> = Vec::new();
    for subtitle in &array {
        let Some(sub_url) = subtitle.get("url").and_then(Value::as_str) else {
            continue;
        };
        if sub_url.is_empty() {
            continue;
        }
        let lang = match subtitle.get("language") {
            Some(Value::String(lang)) if !lang.is_empty() => lang.clone(),
            _ => "it".to_owned(),
        };
        let sub_url = join_value(base, sub_url);
        let ext = super::direct::determine_ext(&sub_url).unwrap_or_else(|| "srt".into());
        let entries = match out.iter().position(|(key, _)| *key == lang) {
            Some(index) => &mut out[index].1,
            None => {
                out.push((lang.clone(), Vec::new()));
                &mut out.last_mut().expect("just pushed").1
            }
        };
        entries.push(json!({ "ext": ext, "url": sub_url }));
        if ext == "stl" {
            let srt = format!("{}srt", &sub_url[..sub_url.len() - 3]);
            entries.push(json!({ "ext": "srt", "url": srt }));
        }
    }
    let mut map = Map::new();
    for (lang, entries) in out {
        map.insert(lang, Value::Array(entries));
    }
    Value::Object(map)
}

fn report_drm(name: &'static str, id: &str) -> Result<()> {
    crate::no_formats::raise_expected(name, Some(id), None, "This video is DRM protected")
}

/// `_extract_relinker_info`'s result.
struct Relinker {
    is_live: Option<bool>,
    duration: Option<Value>,
    formats: Vec<VideoFormat>,
}

/// The relinker's XML, with `fix_cdata` and the `Rai` User-Agent.
fn download_relinker(client: &Client, url: &Url, name: &'static str, id: &str) -> Result<String> {
    crate::screen::screen(&format!("[{name}] {id}: Downloading XML metadata"));
    let mut response =
        super::direct::request_with_headers(client, url, None, &[("User-Agent", "Rai")])?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    let body =
        super::decode_content(response.headers(), body).map_err(crate::Error::ExtractionRead)?;
    let text = String::from_utf8(body)
        .map_err(|_| Error::Reported(format!("[{name}] {id}: Failed to parse XML (not UTF-8)")))?;
    // remove \r\n\t before and after <![CDATA[ ]]> to avoid polluted text
    let text = CDATA_AFTER.replace_all(&text, "$1$2");
    let text = CDATA_BEFORE.replace_all(&text, "$1$2").into_owned();
    Ok(text)
}

/// The child element `tag` of `root`.
fn child<'a, 'input>(
    root: roxmltree::Node<'a, 'input>,
    tag: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    root.children()
        .find(|node| node.is_element() && node.tag_name().name() == tag)
}

/// `xpath_text(root, './tag')`.
fn child_text<'a>(root: roxmltree::Node<'a, '_>, tag: &str) -> Option<&'a str> {
    child(root, tag).and_then(|node| node.text())
}

/// `RaiBaseIE._fix_m3u8_formats`.
fn fix_m3u8_formats(
    client: &Client,
    name: &'static str,
    media_url: &str,
    id: &str,
) -> Result<Vec<VideoFormat>> {
    let Ok(manifest) = Url::parse(media_url) else {
        return Ok(Vec::new());
    };
    let mut formats =
        super::extract_m3u8_master_optional(client, &manifest, name, id, Some("hls"), false, false)
            .map(|info| info.formats)
            .unwrap_or_default();
    for format in &mut formats {
        if format.acodec.as_deref().is_none_or(str::is_empty) {
            format.acodec = Some("mp4a".into());
        }
        if format.vcodec.as_deref().is_none_or(str::is_empty) {
            format.vcodec = Some("avc1".into());
        }
        let manifest_url = format.url.as_str();
        if CHUNKLIST_AO.is_match(manifest_url) {
            format.vcodec = Some("none".into());
        } else if CHUNKLIST_VO.is_match(manifest_url) {
            format.acodec = Some("none".into());
        } else {
            if format.acodec.as_deref() == Some("none") {
                format.acodec = Some("mp4a".into());
            }
            if format.vcodec.as_deref() == Some("none") {
                format.vcodec = Some("avc1".into());
                // `abr` is derived from an audio-only format's `tbr` only
                // at processing time, which now sees a video codec.
                format.abr = None;
            }
        }
    }
    Ok(formats)
}

/// `get_format_info(tbr)` of `_create_http_urls`: the fields of one HTTPS
/// quality.
fn https_quality_fields(
    quality: &str,
    fmts: &[Value],
    id: &str,
) -> Result<Vec<(&'static str, Value)>> {
    let mut br = quality.trim().parse::<i64>().ok().map(|value| value as f64);
    if fmts.len() == 1 && br.is_none_or(|value| value == 0.0) {
        br = fmts[0].get("tbr").and_then(Value::as_f64);
    }
    let tbr: i64 = match br {
        Some(value) if value > 300.0 => (value / 100.0).floor() as i64 * 100,
        _ => 250,
    };
    let percentage = |target: f64| {
        let number = tbr as f64;
        (target - number).abs() < (number * 20.0 / 100.0).min(125.0)
    };
    let quality_size = MEDIA_QUALITY_TBR
        .iter()
        .find(|(key, _)| *key == tbr)
        .map(|(_, size)| *size);
    let mut copy0: Option<&Value> = None;
    let mut copy1: Option<Value> = None;
    for format in fmts {
        if let Some(format_tbr) = format.get("tbr").and_then(Value::as_f64)
            && format_tbr != 0.0
            && percentage(format_tbr)
        {
            copy0 = Some(format);
        }
        if let Some([width, height]) = quality_size
            && format.get("width").and_then(Value::as_i64) == Some(width)
            && format.get("height").and_then(Value::as_i64) == Some(height)
        {
            let mut copy = format.clone();
            copy["tbr"] = Value::from(tbr);
            copy1 = Some(copy);
        }
    }
    let format_copy = copy0
        .cloned()
        .or(copy1)
        .filter(|copy| copy.as_object().is_some_and(|object| !object.is_empty()));
    let format_id = format!("https-{tbr}");
    if let Some(copy) = format_copy {
        let get_or = |key: &str, default: Value| -> Value {
            match copy.get(key) {
                Some(value) if super::nuxt::truthy(value) => value.clone(),
                _ => default,
            }
        };
        Ok(vec![
            ("format_id", format_id.into()),
            ("width", copy.get("width").cloned().unwrap_or(Value::Null)),
            ("height", copy.get("height").cloned().unwrap_or(Value::Null)),
            ("tbr", get_or("tbr", Value::from(tbr))),
            ("vcodec", get_or("vcodec", "avc1".into())),
            ("acodec", get_or("acodec", "mp4a".into())),
            ("fps", get_or("fps", Value::from(25))),
        ])
    } else {
        let Some([width, height]) = quality_size else {
            return Err(Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError({tbr})); {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        Ok(vec![
            ("format_id", format_id.into()),
            ("width", Value::from(width)),
            ("height", Value::from(height)),
            ("tbr", Value::from(tbr)),
            ("vcodec", "avc1".into()),
            ("acodec", "mp4a".into()),
            ("fps", Value::from(25)),
        ])
    }
}

/// `RaiBaseIE._create_http_urls`.
fn create_http_urls(
    client: &Client,
    name: &'static str,
    manifest_url: &str,
    relinker_url: &str,
    all_formats: &[VideoFormat],
    id: &str,
) -> Result<Vec<VideoFormat>> {
    let mp4_template =
        |quality: &str| format!("{relinker_url}&overrideUserAgentRule=mp4-{quality}");
    // Check if MP4 download is available
    crate::screen::screen(&format!("[{name}] {id}: Checking MP4 availability"));
    let available = parse(&mp4_template("*"))
        .and_then(|url| super::direct::head_with_headers(client, &url, &[]).map(|_| ()));
    if let Err(error) = available {
        crate::screen::screen(&format!(
            "[{name}] {id}: MP4 direct download is not available: {}",
            crate::upstream_error_text(&error)
        ));
        return Ok(Vec::new());
    }

    // filter out single-stream formats
    let fmts: Vec<Value> = all_formats
        .iter()
        .filter(|format| {
            format.vcodec.as_deref() != Some("none") && format.acodec.as_deref() != Some("none")
        })
        .map(|format| serde_json::to_value(format).unwrap_or(Value::Null))
        .collect();
    let Some(captures) = MANIFEST_REG.captures(manifest_url) else {
        return Ok(Vec::new());
    };
    let qualities: Vec<String> = match captures.name("quality") {
        Some(quality) => quality.as_str().split(',').map(str::to_owned).collect(),
        None => vec!["*".into()],
    };

    let mut formats = Vec::new();
    for quality in qualities.iter().filter(|quality| !quality.is_empty()) {
        let mut fields = vec![
            ("url", Value::from(mp4_template(quality))),
            ("protocol", "https".into()),
            ("ext", "mp4".into()),
        ];
        fields.extend(https_quality_fields(quality, &fmts, id)?);
        formats.push(dict_format(fields, name, id)?);
    }
    Ok(formats)
}

/// `RaiBaseIE._extract_relinker_info`.
fn extract_relinker(
    client: &Client,
    name: &'static str,
    relinker_url: &str,
    id: &str,
    audio_only: bool,
) -> Result<Relinker> {
    if !(relinker_url.starts_with("http://") || relinker_url.starts_with("https://")) {
        return Ok(Relinker {
            is_live: None,
            duration: None,
            formats: vec![dict_format(vec![("url", relinker_url.into())], name, id)?],
        });
    }
    let relinker = parse(relinker_url)?;
    let query = super::update_url_query(&relinker, &[("output", "64")]);
    let text = download_relinker(client, &query, name, id)?;
    let document = roxmltree::Document::parse(&text).map_err(|error| {
        Error::Reported(format!(
            "[{name}] {id}: Failed to parse XML ({error}); {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    let root = document.root_element();

    if child_text(root, "license_url").is_some_and(|license| license != "{}") {
        report_drm(name, id)?;
    }
    let is_live = child_text(root, "is_live").unwrap_or("N") == "Y";
    let duration = crate::parse_duration(child_text(root, "duration").unwrap_or_default())
        .and_then(|number| serde_json::to_value(number).ok());
    let media_url = root
        .children()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "url"
                && node.attribute("type") == Some("content")
        })
        .and_then(|node| node.text())
        .map(str::to_owned);
    let Some(media_url) = media_url else {
        return Err(Error::Reported(format!(
            "[{name}] The relinker returned no media url; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    // geo flag is a bit unreliable and not properly set all the time
    let geoprotection = child_text(root, "geoprotection").unwrap_or("N") == "Y";

    let ext = super::direct::determine_ext(&media_url)
        .unwrap_or_else(|| "unknown_video".into())
        .to_lowercase();
    let mut formats = Vec::new();
    if ext == "mp3" {
        formats.push(dict_format(
            vec![
                ("url", media_url.as_str().into()),
                ("vcodec", "none".into()),
                ("acodec", "mp3".into()),
                ("format_id", "https-mp3".into()),
            ],
            name,
            id,
        )?);
    } else if ext == "m3u8" || media_url.contains("format=m3u8") {
        formats.extend(fix_m3u8_formats(client, name, &media_url, id)?);
    } else if ext == "f4m" {
        return Err(unsupported("f4m relinker media"));
    } else if ext == "mp4" {
        let bitrate = int_text(child_text(root, "bitrate"));
        // upstream compares `None > 0`, a TypeError, for a missing bitrate
        let Some(bitrate) = bitrate else {
            return Err(unsupported("MP4 relinker media without a bitrate"));
        };
        let tbr = (bitrate > 0).then_some(bitrate);
        let mut format_id = String::from("https");
        if bitrate != 0 {
            format_id.push_str(&format!("-{bitrate}"));
        }
        formats.push(dict_format(
            vec![
                ("url", media_url.as_str().into()),
                ("tbr", tbr.map_or(Value::Null, Value::from)),
                ("format_id", format_id.into()),
            ],
            name,
            id,
        )?);
    } else {
        return Err(Error::Reported(format!(
            "[{name}] {id}: Unrecognized media extension \"{ext}\"; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }

    if (formats.is_empty() && geoprotection) || media_url.contains("/video_no_available.mp4") {
        crate::geo::restricted_metadata_available(
            name,
            &format!("[{name}] {id}: "),
            "This video is not available from your location due to geo restriction",
            GEO,
        )?;
    }

    if !audio_only && !is_live {
        let extra = create_http_urls(client, name, &media_url, relinker_url, &formats, id)?;
        formats.extend(extra);
    }

    Ok(Relinker {
        is_live: Some(is_live),
        duration,
        formats,
    })
}

/// Sets `key` in place, or appends it (Python's dict update).
fn set_or_push(fields: &mut Vec<(&'static str, Value)>, key: &'static str, value: Value) {
    match fields.iter_mut().find(|(name, _)| *name == key) {
        Some(slot) => slot.1 = value,
        None => fields.push((key, value)),
    }
}

/// The fields a relinker result adds to an info dict: `**relinker_info`.
fn relinker_fields(relinker: &Relinker, fields: &mut Vec<(&'static str, Value)>) {
    if let Some(is_live) = relinker.is_live {
        set_or_push(fields, "is_live", Value::Bool(is_live));
    }
    if let Some(duration) = relinker.duration.as_ref().filter(|value| !value.is_null()) {
        set_or_push(fields, "duration", duration.clone());
    }
    set_or_push(fields, "formats", Value::Null);
}

/// `_real_extract` of `RaiPlayIE` (also `RaiPlayLiveIE`).
fn raiplay_extract(
    client: &Client,
    name: &'static str,
    url: &Url,
    base: &str,
    video_id: &str,
) -> Result<VideoInfo> {
    let media = download_json_note(
        client,
        &parse(&format!("{base}.json"))?,
        name,
        video_id,
        "Downloading video JSON",
    )?;
    if truthy(path(&media, &["rights_management", "rights", "drm"])) {
        report_drm(name, video_id)?;
    }
    let video = media
        .get("video")
        .ok_or_else(|| key_error(video_id, "video"))?;
    let content_url =
        str_value(video.get("content_url")).ok_or_else(|| key_error(video_id, "content_url"))?;
    let relinker = extract_relinker(client, name, content_url, video_id, false)?;

    let date_published = join_nonempty(
        &[
            str_value(media.get("date_published")).map(str::to_owned),
            str_value(media.get("time_published")).map(str::to_owned),
        ],
        " ",
    );
    let season = media.get("season");
    let season_text = str_value(season);
    let alt_title = join_nonempty(
        &[
            str_value(media.get("subtitle")).map(str::to_owned),
            str_value(media.get("toptitle")).map(str::to_owned),
        ],
        " - ",
    );
    let id = match str_value(media.get("id")) {
        Some(text) => {
            let text = remove_start(text, "ContentItem-");
            if text.is_empty() { video_id } else { text }
        }
        None => video_id,
    }
    .to_owned();
    let track_info = media.get("track_info");
    let mut fields: Vec<(&'static str, Value)> = vec![
        ("id", id.into()),
        ("display_id", video_id.into()),
        ("title", media.get("name").cloned().unwrap_or(Value::Null)),
        (
            "alt_title",
            strip_or_none(
                (!alt_title.is_empty())
                    .then(|| Value::from(alt_title))
                    .as_ref(),
            ),
        ),
        (
            "description",
            media.get("description").cloned().unwrap_or(Value::Null),
        ),
        (
            "uploader",
            strip_or_none(
                first_truthy(&[
                    path(&media, &["program_info", "channel"]),
                    media.get("channel"),
                ])
                .as_ref(),
            ),
        ),
        (
            "creator",
            strip_or_none(
                first_truthy(&[
                    path(&media, &["program_info", "editor"]),
                    media.get("editor"),
                ])
                .as_ref(),
            ),
        ),
        ("duration", duration_value(video.get("duration"))),
        (
            "timestamp",
            timestamp_value((!date_published.is_empty()).then_some(date_published.as_str())),
        ),
        ("thumbnails", thumbnails(media.get("images"), url)),
        (
            "series",
            path(&media, &["program_info", "name"])
                .cloned()
                .unwrap_or(Value::Null),
        ),
        ("season_number", int_or_none(season)),
        (
            "season",
            match season_text {
                Some(text) if !text.is_empty() && !text.chars().all(|c| c.is_ascii_digit()) => {
                    text.into()
                }
                _ => Value::Null,
            },
        ),
        (
            "episode",
            media.get("episode_title").cloned().unwrap_or(Value::Null),
        ),
        ("episode_number", int_or_none(media.get("episode"))),
        ("subtitles", subtitles(url, video)),
        (
            "release_year",
            int_or_none(track_info.and_then(|info| info.get("edit_year"))),
        ),
    ];
    relinker_fields(&relinker, &mut fields);
    Ok(ordered_subtitles(dict_info(
        url,
        (name, name),
        fields,
        relinker.formats,
    )))
}

impl Extractor for RaiPlay {
    fn key(&self) -> &'static str {
        RAIPLAY_NAME
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(GEO)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = RAIPLAY_VALID.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(RAIPLAY_NAME, url);
        raiplay_extract(client, RAIPLAY_NAME, url, &found["base"], &found["id"]).map(Some)
    }
}

impl Extractor for RaiPlayLive {
    fn key(&self) -> &'static str {
        RAIPLAYLIVE_NAME
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(GEO)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = RAIPLAYLIVE_VALID.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(RAIPLAYLIVE_NAME, url);
        raiplay_extract(client, RAIPLAYLIVE_NAME, url, &found["base"], &found["id"]).map(Some)
    }
}

/// `url_result(url, ie=..., video_id=...)` as a playlist entry.
fn url_entry(target: &str, ie_key: &str, video_id: Option<&str>) -> Result<VideoInfo> {
    let url = parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), ie_key.into());
    if let Some(video_id) = video_id {
        entry.insert("id".into(), video_id.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ie_key,
            "extractor": ie_key.to_lowercase(),
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `playlist_result(entries, id, title, description)`.
fn playlist(
    page: &Url,
    name: &'static str,
    id: &str,
    title: Option<String>,
    description: Option<String>,
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut extra = Map::new();
    extra.insert(
        "description".into(),
        description.map_or(Value::Null, Value::from),
    );
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        "playlist-dict;id,title,description".into(),
    );
    if title.is_none() {
        extra.insert("title".into(), Value::Null);
    }
    ExtractResult::Playlist(PlaylistInfo {
        id: id.to_owned(),
        title: title.unwrap_or_default(),
        webpage_url: page.clone(),
        extractor: name.into(),
        extractor_key: name.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

impl Extractor for RaiPlayPlaylist {
    fn key(&self) -> &'static str {
        RAIPLAYPLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = RAIPLAYPLAYLIST_VALID.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(RAIPLAYPLAYLIST_NAME, url);
        let base = found["base"].to_owned();
        let playlist_id = found["id"].to_owned();
        let extra_id = found.name("extra_id").map(|extra| {
            extra
                .as_str()
                .to_uppercase()
                .trim_end_matches('/')
                .to_owned()
        });

        let program = download_json_note(
            client,
            &parse(&format!("{base}.json"))?,
            RAIPLAYPLAYLIST_NAME,
            &playlist_id,
            "Downloading program JSON",
        )?;
        let mut playlist_title = str_value(program.get("name")).map(str::to_owned);
        let mut entries = Vec::new();
        for block in array(program.get("blocks")) {
            for set in array(block.get("sets")) {
                if let Some(extra_id) = &extra_id {
                    let joined = join_nonempty(
                        &[
                            str_value(block.get("name")).map(str::to_owned),
                            str_value(set.get("name")).map(str::to_owned),
                        ],
                        "/",
                    );
                    if *extra_id != joined.replace(' ', "-").to_uppercase() {
                        continue;
                    }
                    playlist_title = Some(join_nonempty(
                        &[
                            playlist_title.clone(),
                            str_value(set.get("name")).map(str::to_owned),
                        ],
                        " - ",
                    ));
                }
                let Some(set_id) = str_value(set.get("id")).filter(|id| !id.is_empty()) else {
                    continue;
                };
                let Ok(medias) = download_json_note(
                    client,
                    &parse(&format!("{base}/{set_id}.json"))?,
                    RAIPLAYPLAYLIST_NAME,
                    set_id,
                    "Downloading content set JSON",
                ) else {
                    continue;
                };
                for media in array(medias.get("items")) {
                    let Some(path_id) = str_value(media.get("path_id")).filter(|p| !p.is_empty())
                    else {
                        continue;
                    };
                    let video_url = join_value(url, path_id);
                    let video_id = RAIPLAY_VALID
                        .captures(&video_url)
                        .map(|found| found["id"].to_owned());
                    entries.push(url_entry(&video_url, RAIPLAY_NAME, video_id.as_deref())?);
                }
            }
        }
        let description = program
            .get("program_info")
            .and_then(|info| info.get("description"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(Some(playlist(
            url,
            RAIPLAYPLAYLIST_NAME,
            &playlist_id,
            playlist_title,
            description,
            entries,
        )))
    }
}

/// A JSON array's items (`[]` for anything else).
fn array(value: Option<&Value>) -> Vec<&Value> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

/// `_real_extract` of `RaiPlaySoundIE` (also `RaiPlaySoundLiveIE`).
fn raiplaysound_extract(
    client: &Client,
    name: &'static str,
    url: &Url,
    base: &str,
    audio_id: &str,
) -> Result<VideoInfo> {
    let media = download_json_note(
        client,
        &parse(&format!("{base}.json"))?,
        name,
        audio_id,
        "Downloading audio JSON",
    )?;
    let uid = media
        .get("uniquename")
        .and_then(Value::as_str)
        .map(|unique| remove_start(remove_start(unique, "ContentItem-"), "Page-").to_owned());

    let mut relinker_urls: Vec<&str> = Vec::new();
    let candidates = [
        path(&media, &["downloadable_audio", "url"]),
        path(&media, &["audio", "url"]),
        path(&media, &["live", "cards", "0", "audio", "url"]),
    ];
    for candidate in candidates.into_iter().flatten() {
        if let Some(text) = candidate.as_str()
            && !relinker_urls.contains(&text)
        {
            relinker_urls.push(text);
        }
    }
    let mut info: Option<Relinker> = None;
    let mut formats = Vec::new();
    for relinker_url in relinker_urls {
        let relinker = extract_relinker(client, name, relinker_url, audio_id, true)?;
        formats.extend(relinker.formats.clone());
        info = Some(relinker);
    }

    let create_date = media.get("create_date").map(json_text);
    let date_published = match (create_date, media.get("create_time")) {
        (Some(date), time) => {
            let time = time
                .filter(|value| super::nuxt::truthy(value))
                .map_or(String::new(), json_text);
            Some(format!("{date} {time}"))
        }
        (None, _) => path(&media, &["live", "create_date"]).map(json_text),
    };

    let podcast_info = match media.get("podcast_info") {
        Some(info) if super::nuxt::truthy(info) => info.clone(),
        _ => path(&media, &["live", "cards", "0"])
            .cloned()
            .unwrap_or(Value::Object(Map::new())),
    };
    let podcast_images = podcast_info.get("images");

    let mut fields: Vec<(&'static str, Value)> = Vec::new();
    if let Some(relinker) = &info {
        relinker_fields(relinker, &mut fields);
    } else {
        set_or_push(&mut fields, "formats", Value::Null);
    }
    fields.extend(vec![
        (
            "id",
            uid.filter(|id| !id.is_empty())
                .unwrap_or_else(|| audio_id.to_owned())
                .into(),
        ),
        ("display_id", audio_id.into()),
        (
            "title",
            first_truthy(&[media.get("title"), media.get("episode_title")])
                .or_else(|| media.get("title").cloned())
                .unwrap_or(Value::Null),
        ),
        (
            "alt_title",
            strip_or_none(path(&media, &["track_info", "media_name"])),
        ),
        (
            "description",
            media.get("description").cloned().unwrap_or(Value::Null),
        ),
        (
            "uploader",
            strip_or_none(path(&media, &["track_info", "channel"])),
        ),
        (
            "creator",
            strip_or_none(path(&media, &["track_info", "editor"])),
        ),
        ("timestamp", timestamp_value(date_published.as_deref())),
        ("thumbnails", thumbnails(podcast_images, url)),
        (
            "series",
            podcast_info.get("title").cloned().unwrap_or(Value::Null),
        ),
        ("season_number", int_or_none(media.get("season"))),
        (
            "episode",
            media.get("episode_title").cloned().unwrap_or(Value::Null),
        ),
        ("episode_number", int_or_none(media.get("episode"))),
    ]);
    set_or_push(&mut fields, "formats", Value::Null);
    let formats = formats;
    Ok(dict_info(url, (name, name), fields, formats))
}

/// A JSON scalar as Python's `str()` (for a date's text).
fn json_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".into(),
        other => other.to_string(),
    }
}

impl Extractor for RaiPlaySound {
    fn key(&self) -> &'static str {
        RAIPLAYSOUND_NAME
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(GEO)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = RAIPLAYSOUND_VALID.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(RAIPLAYSOUND_NAME, url);
        raiplaysound_extract(client, RAIPLAYSOUND_NAME, url, &found["base"], &found["id"]).map(Some)
    }
}

impl Extractor for RaiPlaySoundLive {
    fn key(&self) -> &'static str {
        RAIPLAYSOUNDLIVE_NAME
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(GEO)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = RAIPLAYSOUNDLIVE_VALID.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(RAIPLAYSOUNDLIVE_NAME, url);
        raiplaysound_extract(
            client,
            RAIPLAYSOUNDLIVE_NAME,
            url,
            &found["base"],
            &found["id"],
        )
        .map(Some)
    }
}

impl Extractor for RaiPlaySoundPlaylist {
    fn key(&self) -> &'static str {
        RAIPLAYSOUNDPLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = RAIPLAYSOUNDPLAYLIST_VALID.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(RAIPLAYSOUNDPLAYLIST_NAME, url);
        let base = found["base"].to_owned();
        let mut playlist_id = found["id"].to_owned();
        let mut program = download_json_note(
            client,
            &parse(&format!("{base}.json"))?,
            RAIPLAYSOUNDPLAYLIST_NAME,
            &playlist_id,
            "Downloading program JSON",
        )?;
        if let Some(extra) = found.name("extra_id") {
            let extra_id = extra.as_str().trim_end_matches('/').to_owned();
            playlist_id.push('_');
            playlist_id.push_str(&extra_id.replace('/', "_"));
            let path_id = array(program.get("filters"))
                .into_iter()
                .find(|filter| {
                    str_value(filter.get("weblink")).is_some_and(|link| link.contains(&extra_id))
                })
                .and_then(|filter| str_value(filter.get("path_id")).map(str::to_owned))
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{RAIPLAYSOUNDPLAYLIST_NAME}] {playlist_id}: Unable to find the filter; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })?;
            let secondary = join_value(&parse("https://www.raiplaysound.it")?, &path_id);
            program = download_json_note(
                client,
                &parse(&secondary)?,
                RAIPLAYSOUNDPLAYLIST_NAME,
                &playlist_id,
                "Downloading program secondary JSON",
            )?;
        }

        let cards = match program.get("cards") {
            Some(cards) if super::nuxt::truthy(cards) => array(Some(cards)),
            _ => array(path(&program, &["block", "cards"])),
        };
        let mut entries = Vec::new();
        for card in cards {
            let Some(path_id) = str_value(card.get("path_id")).filter(|p| !p.is_empty()) else {
                continue;
            };
            entries.push(url_entry(
                &join_value(&parse(&base)?, path_id),
                RAIPLAYSOUND_NAME,
                None,
            )?);
        }
        let title = str_value(program.get("title")).map(str::to_owned);
        let description = path(&program, &["podcast_info", "description"])
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(Some(playlist(
            url,
            RAIPLAYSOUNDPLAYLIST_NAME,
            &playlist_id,
            title,
            description,
            entries,
        )))
    }
}

impl Extractor for Rai {
    fn key(&self) -> &'static str {
        RAI_NAME
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(GEO)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = RAI_VALID.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(RAI_NAME, url);
        rai_extract(client, url, &found["id"]).map(Some)
    }
}

/// `RaiIE._real_extract`: the rai.tv `?json` media.
fn rai_extract(client: &Client, url: &Url, content_id: &str) -> Result<VideoInfo> {
    let media_url = parse(&format!(
        "https://www.rai.tv/dl/RaiTV/programmi/media/ContentItem-{content_id}.html?json"
    ))?;
    let media = download_json_note(
        client,
        &media_url,
        RAI_NAME,
        content_id,
        "Downloading video JSON",
    )?;
    let media_type = str_value(media.get("type")).unwrap_or_default();
    let relinker = if media_type.contains("Audio") {
        let audio_format = str_value(media.get("formatoAudio")).map(str::to_owned);
        let format_id = join_nonempty(&[Some("https".to_owned()), audio_format.clone()], "-");
        let audio_url =
            str_value(media.get("audioUrl")).ok_or_else(|| key_error(content_id, "audioUrl"))?;
        Relinker {
            is_live: None,
            duration: None,
            formats: vec![dict_format(
                vec![
                    ("format_id", format_id.into()),
                    ("url", audio_url.into()),
                    ("ext", audio_format.clone().map_or(Value::Null, Value::from)),
                    ("vcodec", "none".into()),
                    ("acodec", audio_format.map_or(Value::Null, Value::from)),
                ],
                RAI_NAME,
                content_id,
            )?],
        }
    } else if media_type.contains("Video") {
        let media_uri =
            str_value(media.get("mediaUri")).ok_or_else(|| key_error(content_id, "mediaUri"))?;
        extract_relinker(client, RAI_NAME, media_uri, content_id, false)?
    } else {
        return Err(Error::Reported(format!(
            "[{RAI_NAME}] {content_id}: not a media file; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };

    let thumbs = Value::Object(
        ["image", "image_medium", "image_300"]
            .into_iter()
            .map(|key| {
                (
                    key.to_owned(),
                    media.get(key).cloned().unwrap_or(Value::Null),
                )
            })
            .collect(),
    );
    let strip = |value: Option<&Value>| strip_or_none(value);
    let mut fields: Vec<(&'static str, Value)> = vec![
        ("id", content_id.into()),
        (
            "title",
            strip(first_truthy(&[media.get("name"), media.get("title")]).as_ref()),
        ),
        (
            "description",
            match strip(media.get("desc")) {
                Value::Null => Value::Null,
                text => text,
            },
        ),
        ("thumbnails", thumbnails(Some(&thumbs), url)),
        ("uploader", strip(media.get("author"))),
        ("upload_date", strdate_value(str_value(media.get("date")))),
        ("duration", duration_value(media.get("length"))),
        ("subtitles", subtitles(url, &media)),
    ];
    relinker_fields(&relinker, &mut fields);
    Ok(ordered_subtitles(dict_info(
        url,
        (RAI_NAME, RAI_NAME),
        fields,
        relinker.formats,
    )))
}

impl Extractor for RaiNews {
    fn key(&self) -> &'static str {
        RAINEWS_NAME
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(GEO)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = news_match(&RAINEWS_VALID, url.as_str()) else {
            return Ok(None);
        };
        begin(RAINEWS_NAME, url);
        news_extract(client, RAINEWS_NAME, url, &found["id"], &NEWS_PLAYER_START).map(Some)
    }
}

impl Extractor for RaiCultura {
    fn key(&self) -> &'static str {
        RAICULTURA_NAME
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(GEO)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = news_match(&RAICULTURA_VALID, url.as_str()) else {
            return Ok(None);
        };
        begin(RAICULTURA_NAME, url);
        news_extract(
            client,
            RAICULTURA_NAME,
            url,
            &found["id"],
            &CULTURA_PLAYER_START,
        )
        .map(Some)
    }
}

/// `RaiNewsIE._real_extract` (also `RaiCulturaIE`): the page's player data,
/// with the `RaiIE` implementation as the fallback.
fn news_extract(
    client: &Client,
    name: &'static str,
    url: &Url,
    video_id: &str,
    player_start: &Regex,
) -> Result<VideoInfo> {
    let webpage = download_webpage(client, url, name, video_id)?;
    let player_data = match super::search_json_with(player_start, &webpage, &|text| {
        crate::html::clean_html(text)
    }) {
        Some(Some(data)) => data,
        Some(None) => {
            super::search_warning(name, "player_data");
            Value::Object(Map::new())
        }
        None => Value::Object(Map::new()),
    };
    let track_info = player_data
        .get("track_info")
        .cloned()
        .unwrap_or(Value::Object(Map::new()));
    let relinker_url = player_data
        .get("mediapolis")
        .filter(|value| super::nuxt::truthy(value))
        .or_else(|| player_data.get("content_url"))
        .and_then(Value::as_str)
        .map(str::to_owned);

    let Some(relinker_url) = relinker_url.filter(|text| !text.is_empty()) else {
        // fallback on old implementation for some old content
        return match rai_extract(client, url, video_id) {
            Ok(info) => Ok(info),
            // `GeoRestrictedError` is re-raised as is.
            Err(Error::Reported(text)) if text.contains("geo restriction") => {
                Err(Error::Reported(text))
            }
            Err(_) => Err(Error::Reported(format!(
                "[{name}] {video_id}: Relinker URL not found; {}",
                crate::BUG_REPORTS_MESSAGE
            ))),
        };
    };

    let relinker = extract_relinker(
        client,
        name,
        &join_value(url, &relinker_url),
        video_id,
        false,
    )?;
    let title = first_truthy(&[player_data.get("title"), track_info.get("title")])
        .or_else(|| crate::html::og_search_property(&webpage, "title").map(Value::from))
        .unwrap_or(Value::Null);
    let mut fields: Vec<(&'static str, Value)> = vec![
        ("id", video_id.into()),
        ("title", title),
        (
            "upload_date",
            strdate_value(str_value(track_info.get("date"))),
        ),
        (
            "uploader",
            strip_or_none(first_truthy(&[track_info.get("editor")]).as_ref()),
        ),
    ];
    relinker_fields(&relinker, &mut fields);
    Ok(dict_info(url, (name, name), fields, relinker.formats))
}

impl Extractor for RaiSudtirol {
    fn key(&self) -> &'static str {
        RAISUDTIROL_NAME
    }

    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(GEO)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = RAISUDTIROL_VALID.captures(url.as_str()) else {
            return Ok(None);
        };
        begin(RAISUDTIROL_NAME, url);
        let video_id = found["id"].to_owned();
        let webpage = download_webpage(client, url, RAISUDTIROL_NAME, &video_id)?;

        let video_date = first_group(std::slice::from_ref(&*SUDTIROL_DATE), &webpage)
            .map(|text| crate::html::clean_html(&text).trim().to_owned());
        let video_title = first_group(&SUDTIROL_TITLES[..], &webpage)
            .map(|text| crate::html::clean_html(&text).trim().to_owned());
        let video_url = first_group(&SUDTIROL_URLS[..], &webpage);

        let video_url_text = video_url.clone().unwrap_or_default();
        let ext =
            super::direct::determine_ext(&video_url_text).unwrap_or_else(|| "unknown_video".into());
        let formats = if ext == "m3u8" {
            let manifest = parse(&video_url_text)?;
            super::extract_m3u8_fatal_master(
                client,
                &manifest,
                RAISUDTIROL_NAME,
                &video_id,
                None,
                false,
            )?
            .formats
        } else if ext == "mp4" {
            vec![dict_format(
                vec![
                    ("format_id", "https-mp4".into()),
                    ("url", super::proto_relative(&video_url_text).into()),
                    ("width", Value::from(1024)),
                    ("height", Value::from(576)),
                    ("fps", Value::from(25)),
                    ("vcodec", "avc1".into()),
                    ("acodec", "mp4a".into()),
                ],
                RAISUDTIROL_NAME,
                &video_id,
            )?]
        } else {
            return Err(Error::Reported(format!(
                "[{RAISUDTIROL_NAME}] Unrecognized media file: {}; {}",
                video_url.unwrap_or_else(|| "None".into()),
                crate::BUG_REPORTS_MESSAGE
            )));
        };

        let title = join_nonempty(&[video_title.clone(), video_date.clone()], " - ");
        let base = parse("https://raisudtirol.rai.it/")?;
        let thumbnail = SUDTIROL_THUMB
            .captures(&webpage)
            .map(|found| join_value(&base, &found[1]));
        let fields: Vec<(&'static str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("title", title.into()),
            (
                "series",
                video_title
                    .clone()
                    .filter(|_| video_date.is_some())
                    .map_or(Value::Null, Value::from),
            ),
            ("upload_date", strdate_value(video_date.as_deref())),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            ("uploader", "raisudtirol".into()),
            ("formats", Value::Null),
        ];
        Ok(Some(dict_info(
            url,
            (RAISUDTIROL_NAME, RAISUDTIROL_NAME),
            fields,
            formats,
        )))
    }
}

/// The first capture group of the first pattern that matches.
fn first_group(patterns: &[Regex], text: &str) -> Option<String> {
    patterns
        .iter()
        .find_map(|pattern| pattern.captures(text).map(|found| found[1].to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(rai_suitable(
            "https://www.rai.it/dl/RaiTV/programmi/media/ContentItem-efb17665-691c-45d5-a60c-5301333cbb0c.html"
        ));
        assert!(!rai_suitable(
            "https://www.raiplay.it/video/2014/04/Report.html"
        ));
        assert!(rainews_suitable(
            "https://www.rainews.it/video/2024/02/x-31e8017c-845c-43f5-9c48-245b43c3a079.html"
        ));
        assert!(!rainews_suitable(
            "https://www.rainews.it/articoli/2024/02/x-31e8017c-845c-43f5-9c48-245b43c3a079.html"
        ));
        assert!(raicultura_suitable(
            "https://www.raicultura.it/letteratura/articoli/2018/12/Alberto-05ba8775-82b5-45c5-a89d-dd955fbde1fb.html"
        ));
        assert!(raiplay_suitable(
            "https://www.raiplay.it/video/2014/04/Report-del-07042014-cb27157f-9dd0-4aee-b788-b1f67643a391.html"
        ));
        assert!(raiplaylive_suitable(
            "http://www.raiplay.it/dirette/rainews24"
        ));
        assert!(raiplayplaylist_suitable(
            "https://www.raiplay.it/programmi/nondirloalmiocapo/"
        ));
        assert!(raiplaysound_suitable(
            "https://www.raiplaysound.it/audio/2021/12/IL-RUGGITO-DEL-CONIGLIO-1ebae2a7-7cdb-42bb-842e-fe0d193e9707.html"
        ));
        assert!(raiplaysoundlive_suitable(
            "https://www.raiplaysound.it/radio2"
        ));
        assert!(!raiplaysoundlive_suitable(
            "https://www.raiplaysound.it/programmi/ilruggitodelconiglio"
        ));
        assert!(raiplaysoundplaylist_suitable(
            "https://www.raiplaysound.it/programmi/ilruggitodelconiglio"
        ));
        assert!(raisudtirol_suitable(
            "https://raisudtirol.rai.it/la/index.php?media=Ptv1619729460"
        ));
        assert!(!raisudtirol_suitable(
            "https://raisudtirol.rai.it/la/index.php"
        ));
    }
}
