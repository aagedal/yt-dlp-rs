//! Upstream `TapTapBaseIE` and its moment, app (`taptap.cn` and `taptap.io`)
//! and post classes (`yt_dlp/extractor/taptap.py`): a detail API's videos as
//! HLS playlist entries, the H.265 playback URL preferred. The video IDs are
//! a Python `set()` upstream, so their order follows CPython's set table; that
//! is reproduced for up to four videos and larger sets are unsupported.
use super::{Extractor, extract_m3u8_master_optional};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::hash::{BuildHasher, Hasher};
use std::sync::LazyLock;
use url::Url;

const NAME: &str = "TapTap";
const CN_X_UA: &str = "V=1&PN=WebApp&LANG=zh_CN&VN_CODE=102&LOC=CN&PLT=PC&DS=Android&UID={uuid}&OS=Windows&OSV=10&DT=PC";
const INTL_X_UA: &str = "V=1&PN=WebAppIntl2&LANG=zh_TW&VN_CODE=115&VN=0.1.0&LOC=CN&PLT=PC&DS=Android&UID={uuid}&CURR=&DT=PC&OS=Windows&OSV=NT%208.0.0";
const CN_VIDEO_API: &str = "https://www.taptap.cn/webapiv2/video-resource/v1/multi-get";
const INTL_VIDEO_API: &str = "https://www.taptap.io/webapiv2/video-resource/v1/multi-get";

/// An info dict's fields in upstream's key order.
type VideoFields = Vec<(&'static str, Value)>;

/// Which upstream class a [`TapTap`] extractor is.
#[derive(Clone, Copy)]
enum Kind {
    Moment,
    App,
    AppIntl,
    PostIntl,
}

pub struct TapTapMoment;
pub struct TapTapApp;
pub struct TapTapAppIntl;
pub struct TapTapPostIntl;

static MOMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://www\.taptap\.cn/moment/(?P<id>\d+)").unwrap());
static APP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://www\.taptap\.cn/app/(?P<id>\d+)").unwrap());
static APP_INTL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://www\.taptap\.io/app/(?P<id>\d+)").unwrap());
static POST_INTL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://www\.taptap\.io/post/(?P<id>\d+)").unwrap());
static H265: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(hev|hvc|hvt)\d").unwrap());

fn pattern(kind: Kind) -> &'static Regex {
    match kind {
        Kind::Moment => &MOMENT,
        Kind::App => &APP,
        Kind::AppIntl => &APP_INTL,
        Kind::PostIntl => &POST_INTL,
    }
}

fn name(kind: Kind) -> &'static str {
    match kind {
        Kind::Moment => "TapTapMoment",
        Kind::App => "TapTapApp",
        Kind::AppIntl => "TapTapAppIntl",
        Kind::PostIntl => "TapTapPostIntl",
    }
}

/// A random UUID4 for `X-UA`.
fn uuid4() -> String {
    let mut bytes = [0_u8; 16];
    for (index, chunk) in bytes.chunks_mut(8).enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_usize(index);
        chunk.copy_from_slice(&hasher.finish().to_be_bytes());
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

/// `_get_api(url, video_id, query)['data']`, with a fresh `X-UA`.
fn get_api(
    client: &Client,
    base: &str,
    id: &str,
    query: &[(&str, &str)],
    x_ua: &str,
) -> Result<Value> {
    let mut url = Url::parse(base)?;
    {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
        pairs.append_pair("X-UA", &x_ua.replace("{uuid}", &uuid4()));
    }
    let response = super::download_json(client, &url, NAME, id)?;
    response.get("data").cloned().ok_or_else(|| {
        Error::UnsupportedHtml5(
            "TapTap API responses without data are not supported natively".into(),
        )
    })
}

/// `int_or_none`.
fn int_value(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `str_or_none` of an integer ID.
fn id_text(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// `clean_html`ed string, if it is one.
fn clean(value: Option<&Value>) -> Option<String> {
    value?.as_str().map(crate::html::clean_html)
}

fn plain(value: Option<&Value>) -> Option<String> {
    value?.as_str().map(str::to_owned)
}

/// The video IDs upstream's `_ID_PATH` finds, in document order.
fn video_ids(kind: Kind, data: &Value) -> Vec<Value> {
    let mut ids = Vec::new();
    let mut push = |container: Option<&Value>| {
        if let Some(videos) = container.and_then(Value::as_array) {
            ids.extend(
                videos
                    .iter()
                    .filter_map(|video| video.get("video_id").cloned()),
            );
        }
    };
    match kind {
        Kind::Moment => {
            let topic = data.get("moment").and_then(|moment| moment.get("topic"));
            push(topic.and_then(|topic| topic.get("videos")));
            push_pin(&mut ids, topic.and_then(|topic| topic.get("pin_video")));
        }
        Kind::App | Kind::AppIntl => {
            push(data.get("app_videos"));
            push(data.get("videos"));
        }
        // The post object is already `data` (`_DATA_PATH = 'post'`).
        Kind::PostIntl => {
            push(data.get("videos"));
            push_pin(&mut ids, data.get("pin_video"));
        }
    }
    ids
}

fn push_pin(ids: &mut Vec<Value>, pin: Option<&Value>) {
    if let Some(id) = pin.and_then(|pin| pin.get("video_id")) {
        ids.push(id.clone());
    }
}

/// `_META_PATH`'s fields in upstream's dict order.
fn metadata(kind: Kind, data: &Value) -> Vec<(&'static str, Value)> {
    let mut fields: Vec<(&str, Option<Value>)> = Vec::new();
    match kind {
        Kind::Moment => {
            let moment = data.get("moment").unwrap_or(&Value::Null);
            let topic = moment.get("topic");
            let user = moment.get("author").and_then(|author| author.get("user"));
            fields.push((
                "timestamp",
                int_value(moment.get("created_time")).map(Value::from),
            ));
            fields.push((
                "modified_timestamp",
                int_value(moment.get("edited_time")).map(Value::from),
            ));
            fields.push((
                "uploader",
                plain(user.and_then(|user| user.get("name"))).map(Value::from),
            ));
            fields.push((
                "uploader_id",
                id_text(user.and_then(|user| user.get("id"))).map(Value::from),
            ));
            fields.push((
                "title",
                plain(topic.and_then(|topic| topic.get("title"))).map(Value::from),
            ));
            fields.push((
                "description",
                plain(topic.and_then(|topic| topic.get("summary"))).map(Value::from),
            ));
        }
        Kind::App | Kind::AppIntl => {
            fields.push(("title", plain(data.get("title")).map(Value::from)));
            let text = data
                .get("description")
                .and_then(|description| description.get("text"));
            fields.push(("description", clean(text).map(Value::from)));
        }
        Kind::PostIntl => {
            let post = data;
            let user = post.get("user");
            fields.push((
                "timestamp",
                int_value(post.get("published_time")).map(Value::from),
            ));
            fields.push((
                "modified_timestamp",
                int_value(post.get("edited_time")).map(Value::from),
            ));
            fields.push((
                "uploader",
                plain(user.and_then(|user| user.get("name"))).map(Value::from),
            ));
            fields.push((
                "uploader_id",
                id_text(user.and_then(|user| user.get("id"))).map(Value::from),
            ));
            fields.push(("title", plain(post.get("title")).map(Value::from)));
            let summary = post
                .get("list_fields")
                .and_then(|fields| fields.get("summary"));
            fields.push(("description", plain(summary).map(Value::from)));
        }
    }
    fields
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .collect()
}

/// CPython's `set(ints)` iteration order for up to four distinct values.
fn python_set_order(values: &[i64]) -> Result<Vec<i64>> {
    let mut unique: Vec<i64> = Vec::new();
    for value in values {
        if !unique.contains(value) {
            unique.push(*value);
        }
    }
    if unique.len() > 4 {
        return Err(Error::UnsupportedHtml5(
            "TapTap pages with more than four videos are not supported natively (Python set order)"
                .into(),
        ));
    }
    // An 8-slot table: a fresh slot is taken by `i`, else CPython's perturbed probe.
    let mask: u64 = 7;
    let mut table: [Option<i64>; 8] = [None; 8];
    for value in unique {
        let hash = value.unsigned_abs();
        let mut perturb = hash;
        let mut slot = hash & mask;
        while table[slot as usize].is_some() {
            perturb >>= 5;
            slot = (slot.wrapping_mul(5).wrapping_add(1).wrapping_add(perturb)) & mask;
        }
        table[slot as usize] = Some(value);
    }
    Ok(table.iter().flatten().copied().collect())
}

/// `_extract_video(video_id)`: the video's H.265 (else H.264) HLS formats,
/// and its `id`, `formats`, `duration` and `thumbnail` fields in that order.
fn extract_video(
    client: &Client,
    kind: Kind,
    video_id: &str,
) -> Result<(Vec<crate::VideoFormat>, VideoFields)> {
    let (video_api, x_ua) = match kind {
        Kind::Moment | Kind::App => (CN_VIDEO_API, CN_X_UA),
        Kind::AppIntl | Kind::PostIntl => (INTL_VIDEO_API, INTL_X_UA),
    };
    let data = get_api(
        client,
        video_api,
        video_id,
        &[("video_ids", video_id)],
        x_ua,
    )?;
    let video = data
        .get("list")
        .and_then(Value::as_array)
        .and_then(|list| list.first())
        .ok_or_else(|| Error::Reported(format!("[{NAME}] {video_id}: list index out of range")))?;
    let play = video.get("play_url");
    // `('play_url', ('url_h265', 'url'), {url_or_none}, any)`.
    let url = ["url_h265", "url"]
        .iter()
        .find_map(|key| {
            play.and_then(|play| play.get(*key))
                .and_then(Value::as_str)
                .filter(|url| Url::parse(url).is_ok())
        })
        .ok_or_else(|| {
            Error::UnsupportedHtml5(
                "TapTap videos without a playback URL are not supported natively".into(),
            )
        })?;
    let manifest = Url::parse(url)?;
    let mut formats =
        extract_m3u8_master_optional(client, &manifest, NAME, video_id, None, false, false)
            .map(|found| found.formats)
            .unwrap_or_default();
    for format in &mut formats {
        if format
            .vcodec
            .as_deref()
            .is_some_and(|codec| H265.is_match(codec))
        {
            format.format_id = if format.format_id.is_empty() {
                "h265".into()
            } else {
                format!("{}_h265", format.format_id)
            };
        }
    }
    let duration = int_value(video.get("info").and_then(|info| info.get("duration")));
    let thumbnail = video.get("thumbnail").and_then(|thumbnail| {
        ["original_url", "url"].iter().find_map(|key| {
            thumbnail
                .get(*key)
                .and_then(Value::as_str)
                .filter(|url| Url::parse(url).is_ok())
        })
    });
    let mut fields: Vec<(&'static str, Value)> =
        vec![("id", video_id.into()), ("formats", Value::Null)];
    if let Some(duration) = duration {
        fields.push(("duration", duration.into()));
    }
    if let Some(thumbnail) = thumbnail {
        fields.push(("thumbnail", thumbnail.into()));
    }
    Ok((formats, fields))
}

/// The whole `_real_extract` for one of the classes.
fn extract(client: &Client, url: &Url, kind: Kind) -> Result<Option<ExtractResult>> {
    let Some(found) = pattern(kind).captures(url.as_str()) else {
        return Ok(None);
    };
    let video_id = found["id"].to_owned();
    crate::screen::set_extractor(name(kind));
    crate::screen::screen(&format!(
        "[{}] Extracting URL: {}",
        name(kind),
        crate::screen::extracting_url(url.as_str())
    ));
    let (info_api, query_key, data_path, x_ua) = match kind {
        Kind::Moment => (
            "https://www.taptap.cn/webapiv2/moment/v3/detail",
            "id",
            None,
            CN_X_UA,
        ),
        Kind::App => (
            "https://www.taptap.cn/webapiv2/app/v4/detail",
            "id",
            None,
            CN_X_UA,
        ),
        Kind::AppIntl => (
            "https://www.taptap.io/webapiv2/i/app/v5/detail",
            "id",
            Some("app"),
            INTL_X_UA,
        ),
        Kind::PostIntl => (
            "https://www.taptap.io/webapiv2/creation/post/v1/detail",
            "id_str",
            Some("post"),
            INTL_X_UA,
        ),
    };
    let response = get_api(client, info_api, &video_id, &[(query_key, &video_id)], x_ua)?;
    let data = match data_path {
        Some(path) => response.get(path).cloned().unwrap_or(Value::Null),
        None => response,
    };
    let metainfo = metadata(kind, &data);
    let mut ids = Vec::new();
    for id in video_ids(kind, &data) {
        match int_value(Some(&id)) {
            Some(id) => ids.push(id),
            None => {
                return Err(Error::UnsupportedHtml5(
                    "TapTap video IDs that are not integers are not supported natively".into(),
                ));
            }
        }
    }
    let mut entries = Vec::new();
    for id in python_set_order(&ids)? {
        // `{**metainfo, **self._extract_video(id)}`.
        let (formats, video_fields) = extract_video(client, kind, &id.to_string())?;
        let mut fields: Vec<(&'static str, Value)> = metainfo.clone();
        for (key, value) in video_fields {
            match fields.iter_mut().find(|(existing, _)| *existing == key) {
                Some(slot) => slot.1 = value,
                None => fields.push((key, value)),
            }
        }
        entries.push(super::dict_info(
            url,
            (name(kind), name(kind)),
            fields,
            formats,
        ));
    }
    let title = metainfo
        .iter()
        .find(|(key, _)| *key == "title")
        .and_then(|(_, value)| value.as_str().map(str::to_owned))
        .unwrap_or_default();
    let mut extra = Map::new();
    let mut keys: Vec<String> = Vec::new();
    for (key, value) in &metainfo {
        keys.push((*key).to_owned());
        if *key != "title" {
            extra.insert((*key).to_owned(), value.clone());
        }
    }
    keys.push("id".into());
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    Ok(Some(ExtractResult::Playlist(PlaylistInfo {
        id: video_id,
        title,
        webpage_url: url.clone(),
        extractor: name(kind).into(),
        extractor_key: name(kind).into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })))
}

macro_rules! taptap_extractor {
    ($ty:ident, $kind:expr) => {
        impl Extractor for $ty {
            fn key(&self) -> &'static str {
                name($kind)
            }

            fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
                Ok(None)
            }

            fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
                extract(client, url, $kind)
            }
        }
    };
}

taptap_extractor!(TapTapMoment, Kind::Moment);
taptap_extractor!(TapTapApp, Kind::App);
taptap_extractor!(TapTapAppIntl, Kind::AppIntl);
taptap_extractor!(TapTapPostIntl, Kind::PostIntl);

/// URL matching for `--list-extractors`.
pub fn moment_suitable(url: &str) -> bool {
    MOMENT.is_match(url)
}

/// URL matching for `--list-extractors`.
pub fn app_suitable(url: &str) -> bool {
    APP.is_match(url)
}

/// URL matching for `--list-extractors`.
pub fn app_intl_suitable(url: &str) -> bool {
    APP_INTL.is_match(url)
}

/// URL matching for `--list-extractors`.
pub fn post_intl_suitable(url: &str) -> bool {
    POST_INTL.is_match(url)
}
