//! Upstream `RCTIPlusIE` and `RCTIPlusSeriesIE` (`yt_dlp/extractor/rcti.py`):
//! an episode, clip, extra, live or missed event of RCTI+ from its visitor
//! token, `/url` and metadata APIs and the HLS master playlist; and a program
//! page's episodes, extras and clips as `url` entries. `RCTIPlusTVIE` (TV and
//! live-channel pages) is not ported and its URLs are not claimed here.
use super::{
    Extractor, dict_info, download_json_note, download_json_note_headers,
    post_json_errnote_optional,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::StatusCode;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hasher};
use std::sync::LazyLock;
use url::Url;

pub struct RctiPlus;
pub struct RctiPlusSeries;

const NAME: &str = "RCTIPlus";
const SERIES_NAME: &str = "RCTIPlusSeries";
const SERIES_KEY: &str = "RCTIPlusSeries";
const REFERER: &str = "https://www.rctiplus.com/";

static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://www\.rctiplus\.com/programs/(?P<id>\d+)/(?P<display_id>[^/?#&]+)(?:/(?P<type>episodes|extras|clips))?",
    )
    .unwrap()
});
static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://www\.rctiplus\.com/(?:programs/\d+?/.*?/)?(?P<type>episode|clip|extra|live-event|missed-event)/(?P<id>\d+)/(?P<display_id>[^/?#&]+)",
    )
    .unwrap()
});

/// `_call_api(url, video_id, note)`: the bearer key's JSON `data` and `meta`.
fn call_api(
    client: &Client,
    url: &Url,
    auth: &str,
    video_id: &str,
    note: &str,
) -> Result<(Value, Value)> {
    call_api_named(client, NAME, url, auth, video_id, note)
}

/// [`call_api`] for an extractor `name` (its `IE_NAME` in the messages).
fn call_api_named(
    client: &Client,
    name: &'static str,
    url: &Url,
    auth: &str,
    video_id: &str,
    note: &str,
) -> Result<(Value, Value)> {
    let json = download_json_note_headers(
        client,
        url,
        name,
        video_id,
        note,
        &[("Authorization", auth)],
    )?;
    let code = json
        .get("status")
        .and_then(|status| status.get("code"))
        .and_then(Value::as_i64)
        .unwrap_or(0);
    if code != 0 {
        let message = json
            .get("status")
            .and_then(|status| status.get("message_client"))
            .and_then(Value::as_str)
            .unwrap_or_default();
        return Err(Error::Reported(format!(
            "[{name}] {video_id}: {name} said: {message}"
        )));
    }
    Ok((
        json.get("data").cloned().unwrap_or(Value::Null),
        json.get("meta").cloned().unwrap_or(Value::Null),
    ))
}

/// `try_get(video_json, lambda x: x[a] < x[b])`: `None` when either field is
/// missing or the two are not comparable.
fn is_less(json: &Value, a: &str, b: &str) -> Option<bool> {
    match (json.get(a)?, json.get(b)?) {
        (Value::Number(x), Value::Number(y)) => Some(x.as_f64()? < y.as_f64()?),
        (Value::String(x), Value::String(y)) => Some(x < y),
        _ => None,
    }
}

fn text(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| !value.is_null()).cloned()
}

/// A non-null string field.
fn string_of(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// `dict_get(data, keys)`: the first key with a non-`None` value.
fn first_of(data: &Value, keys: &[&str]) -> Option<Value> {
    keys.iter().find_map(|key| text(data.get(*key)))
}

/// `str_or_none(value)`.
fn str_or_none(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Null => None,
        Value::String(text) => Some(Value::from(text.clone())),
        other => Some(Value::from(other.to_string())),
    }
}

/// `appierid`: upstream's `'23984824_' + str(random.randint(0, 10000000000))`.
fn appier_id() -> String {
    let random = RandomState::new().build_hasher().finish() % 10_000_000_001;
    format!("23984824_{random}")
}

fn conviva_session(client: &Client, display_id: &str, video_url: &str) -> Result<()> {
    let body = json!({
        "t": "CwsSessionHb",
        "cid": "ff84ae928c3b33064b76dec08f12500465e59a6f",
        "clid": "0",
        "sid": 0,
        "seq": 0,
        "caps": 0,
        "sf": 7,
        "sdk": true,
        "url": video_url,
        "sst": std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default(),
    });
    let response = post_json_errnote_optional(
        client,
        &Url::parse("https://ff84ae928c3b33064b76dec08f12500465e59a6f.cws.conviva.com/0/wsg")?,
        (NAME, display_id),
        (
            "Creating Conviva session",
            "Failed to create Conviva session",
        ),
        &[],
        body.to_string(),
    )?;
    if let Some(response) = response
        && response.get("err").and_then(Value::as_str) != Some("ok")
    {
        let err = response.get("err").map(|err| match err {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        });
        crate::screen::warning(&format!(
            "[{NAME}] Conviva said: {}",
            err.unwrap_or_else(|| "None".into())
        ));
    }
    Ok(())
}

impl Extractor for RctiPlus {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_type = found["type"].to_owned();
        let video_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_real_initialize`: the visitor's access token.
        let auth = visitor_auth(client, &video_id)?;

        let url_version = if video_type == "missed-event" {
            "v2"
        } else {
            "v1"
        };
        let (video_json, _) = call_api(
            client,
            &Url::parse(&format!(
                "https://api.rctiplus.com/api/{url_version}/{video_type}/{video_id}/url?appierid={}",
                appier_id()
            ))?,
            &auth,
            &display_id,
            "Downloading video URL JSON",
        )?;
        let Some(video_url) = string_of(video_json.get("url")) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: An extractor error has occurred. (caused by KeyError('url')); {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };

        let mut is_upcoming = is_less(&video_json, "current_date", "live_at");
        if is_upcoming.is_none() {
            is_upcoming = is_less(&video_json, "current_date", "start_date");
        }
        if is_upcoming == Some(true) {
            let message = match video_json.get("live_label").and_then(Value::as_str) {
                Some(label) if !label.is_empty() => {
                    format!("This event will start at {label}.")
                }
                _ => "This event has not started yet.".to_owned(),
            };
            crate::no_formats::raise_expected(NAME, None, Some(video_id.as_str()), &message)?;
        }
        if video_url.contains("akamaized") {
            // For some Akamai-hosted videos a Conviva session must be created first.
            conviva_session(client, &display_id, &video_url)?;
        }

        let (video_meta, meta_paths) = call_api(
            client,
            &Url::parse(&format!(
                "https://api.rctiplus.com/api/v1/{video_type}/{video_id}"
            ))?,
            &auth,
            &display_id,
            "Downloading video metadata",
        )?;

        let image_path = meta_paths
            .get("image_path")
            .and_then(Value::as_str)
            .unwrap_or("https://rstatic.akamaized.net/media/")
            .to_owned();
        let mut thumbnails = Vec::new();
        // 2000px seems to be the highest resolution that can be given.
        for (id, key) in [
            ("portrait_image", "portrait_image"),
            ("landscape_image", "landscape_image"),
        ] {
            if let Some(image) = string_of(video_meta.get(key)) {
                thumbnails.push(json!({"id": id, "url": format!("{image_path}2000{image}")}));
            }
        }

        let manifest = Url::parse(&video_url)?;
        let formats = match super::direct::request_with_headers(
            client,
            &manifest,
            None,
            &[("Referer", REFERER)],
        ) {
            Ok(response) => {
                super::hls::from_response_with(client, response, &manifest, None, true, None, true)?
                    .map(|info| info.formats)
                    .unwrap_or_default()
            }
            Err(Error::Http(error)) if error.status() == Some(StatusCode::FORBIDDEN) => {
                crate::geo::restricted_metadata_available(
                    NAME,
                    &format!("[{NAME}] {display_id}: "),
                    "This video is not available from your location due to geo restriction",
                    &["ID"],
                )?;
                Vec::new()
            }
            Err(error) => return Err(error),
        };
        let mut formats = formats;
        for format in &mut formats {
            let url = format.url.as_str();
            if url.contains("akamaized") || url.contains("cloudfront") {
                // Referer header is required for akamai/cloudfront CDNs.
                format
                    .http_headers
                    .get_or_insert_with(Map::new)
                    .insert("Referer".into(), REFERER.into());
            }
        }

        let is_live = video_type == "live-event" && is_upcoming != Some(true);
        let live_status = if is_upcoming == Some(true) {
            "is_upcoming"
        } else if is_live {
            "is_live"
        } else if video_type == "missed-event" {
            "was_live"
        } else {
            "not_live"
        };
        let fields = vec![
            (
                "id",
                first_of(&video_meta, &["product_id"])
                    .or_else(|| text(video_json.get("product_id")))
                    .unwrap_or(Value::Null),
            ),
            (
                "title",
                first_of(&video_meta, &["title", "name"])
                    .or_else(|| first_of(&video_json, &["content_name", "assets_name"]))
                    .unwrap_or(Value::Null),
            ),
            ("display_id", json!(display_id)),
            (
                "description",
                video_meta.get("summary").cloned().unwrap_or(Value::Null),
            ),
            (
                "timestamp",
                first_of(&video_meta, &["release_date"])
                    .or_else(|| text(video_json.get("start_date")))
                    .unwrap_or(Value::Null),
            ),
            (
                "duration",
                video_meta.get("duration").cloned().unwrap_or(Value::Null),
            ),
            (
                "categories",
                match text(video_meta.get("genre")) {
                    Some(genre) => json!([genre]),
                    None => Value::Null,
                },
            ),
            (
                "average_rating",
                video_meta
                    .get("star_rating")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "series",
                first_of(&video_meta, &["program_title"])
                    .or_else(|| text(video_json.get("program_title")))
                    .unwrap_or(Value::Null),
            ),
            (
                "season_number",
                video_meta.get("season").cloned().unwrap_or(Value::Null),
            ),
            (
                "episode_number",
                video_meta.get("episode").cloned().unwrap_or(Value::Null),
            ),
            (
                "channel",
                video_json.get("tv_name").cloned().unwrap_or(Value::Null),
            ),
            (
                "channel_id",
                str_or_none(video_json.get("tv_id")).unwrap_or(Value::Null),
            ),
            ("formats", Value::Null),
            ("thumbnails", Value::Array(thumbnails)),
            ("is_live", json!(is_live)),
            ("was_live", json!(video_type == "missed-event")),
            // `_fill_common_fields`'s status from the flags, made explicit.
            ("live_status", json!(live_status)),
            (
                "release_timestamp",
                video_json.get("live_at").cloned().unwrap_or(Value::Null),
            ),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        // The thumbnail dicts are `{id, url}` in upstream's order.
        if let Some(Value::String(shape)) = info.extra.get(crate::info_order::INFO_SHAPE).cloned() {
            info.extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                format!("{shape};thumbnail=id,url").into(),
            );
        }
        Ok(Some(ExtractResult::Video(info)))
    }
}

/// `_real_initialize`: the visitor's access token (`data.access_token`).
fn visitor_auth(client: &Client, note_id: &str) -> Result<String> {
    let visitor = download_json_note(
        client,
        &Url::parse("https://api.rctiplus.com/api/v1/visitor?platform=web")?,
        NAME,
        note_id,
        "Fetching authorization key",
    )?;
    Ok(visitor
        .get("data")
        .and_then(|data| data.get("access_token"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned())
}

/// `RCTIPlusSeriesIE._AGE_RATINGS`.
fn age_rating(code: &str) -> Option<i64> {
    match code {
        "S-SU" | "SU" | "P" => Some(2),
        "A" => Some(7),
        "R" => Some(13),
        "R-R/1" => Some(17),
        "D" => Some(18),
        _ => None,
    }
}

/// `traverse_obj(data, path, expected_type=lambda x: strip_or_none(x) or None)`
/// of a list of `name`s: the non-blank stripped names.
fn names_of(values: Vec<&Value>) -> Value {
    values
        .into_iter()
        .filter_map(|value| value.as_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(Value::from)
        .collect::<Vec<Value>>()
        .into()
}

/// `RCTIPlusSeriesIE._entries(url, ...)`: every page of a list endpoint, each
/// item as a `url` entry with the series metadata. A `not found` list is empty.
fn series_list_entries(
    client: &Client,
    auth: &str,
    url: &str,
    display_id: &str,
    note: &str,
    metadata: &[(&'static str, Value)],
) -> Result<Vec<VideoInfo>> {
    let first = call_api_named(
        client,
        SERIES_NAME,
        &Url::parse(&format!("{url}&length=20&page=0"))?,
        auth,
        display_id,
        note,
    );
    let total_pages = match first {
        Ok((_, meta)) => meta
            .get("pagination")
            .and_then(|pagination| pagination.get("total_page"))
            .and_then(Value::as_i64)
            .unwrap_or(0),
        Err(Error::Reported(message)) if message.contains("not found") => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut entries = Vec::new();
    for page in 1..=total_pages.max(0) {
        let (items, _) = call_api_named(
            client,
            SERIES_NAME,
            &Url::parse(&format!("{url}&length=20&page={page}"))?,
            auth,
            display_id,
            &format!("{note} page {page}"),
        )?;
        for item in items.as_array().into_iter().flatten() {
            entries.push(series_entry(item, metadata)?);
        }
    }
    Ok(entries)
}

/// One program item as a `url` entry (its keys in upstream's dict order).
fn series_entry(item: &Value, metadata: &[(&'static str, Value)]) -> Result<VideoInfo> {
    let field = |key: &str| item.get(key).cloned().unwrap_or(Value::Null);
    let share_link = field("share_link");
    let target = share_link.as_str().unwrap_or_default().to_owned();
    let display_id = item
        .get("title_code")
        .and_then(Value::as_str)
        .map_or(Value::Null, |code| Value::from(code.replace('_', "-")));
    let mut entry = Map::new();
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), share_link);
    entry.insert("ie_key".into(), NAME.into());
    entry.insert("id".into(), field("product_id"));
    entry.insert("title".into(), field("title"));
    entry.insert("display_id".into(), display_id);
    entry.insert("description".into(), field("summary"));
    entry.insert("timestamp".into(), field("release_date"));
    entry.insert("duration".into(), field("duration"));
    entry.insert("season_number".into(), field("season"));
    entry.insert("episode_number".into(), field("episode"));
    for (key, value) in metadata {
        entry.insert((*key).into(), value.clone());
    }
    url_entry_map(&target, entry)
}

/// A `url` result to [`RctiPlus`] carrying the entry's fields in order.
fn url_entry_map(target: &str, entry: Map<String, Value>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": SERIES_NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for RctiPlusSeries {
    fn key(&self) -> &'static str {
        SERIES_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SERIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let series_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        let video_type = found.name("type").map(|kind| kind.as_str().to_owned());
        crate::screen::set_extractor(SERIES_NAME);
        crate::screen::screen(&format!(
            "[{SERIES_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        if let Some(kind) = &video_type {
            crate::screen::warning(&format!(
                "Only {kind} will be downloaded. To download everything from the series, remove \"/{kind}\" from the URL"
            ));
        }
        let auth = visitor_auth(client, &display_id)?;

        let (series_meta, _) = call_api_named(
            client,
            SERIES_NAME,
            &Url::parse(&format!(
                "https://api.rctiplus.com/api/v1/program/{series_id}/detail"
            ))?,
            &auth,
            &display_id,
            "Downloading series metadata",
        )?;
        let age_limit = series_meta
            .get("age_restriction")
            .and_then(|restrictions| restrictions.get(0))
            .and_then(|restriction| restriction.get("code"))
            .and_then(Value::as_str)
            .and_then(age_rating)
            .map_or(Value::Null, Value::from);
        let cast_sources: Vec<&Value> = ["starring", "creator", "writer"]
            .iter()
            .filter_map(|key| series_meta.get(*key).and_then(Value::as_array))
            .flatten()
            .filter_map(|person| person.get("name"))
            .collect();
        let tag_sources: Vec<&Value> = series_meta
            .get("tag")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| tag.get("name"))
            .collect();
        let metadata: Vec<(&'static str, Value)> = vec![
            ("age_limit", age_limit),
            ("cast", names_of(cast_sources)),
            ("tags", names_of(tag_sources)),
        ];

        let mut entries = Vec::new();
        if video_type.as_deref().is_none_or(|kind| kind == "episodes") {
            let seasons = match call_api_named(
                client,
                SERIES_NAME,
                &Url::parse(&format!(
                    "https://api.rctiplus.com/api/v1/program/{series_id}/season"
                ))?,
                &auth,
                &display_id,
                "Downloading seasons list JSON",
            ) {
                Ok((seasons, _)) => seasons,
                Err(Error::Reported(message)) if message.contains("not found") => Value::Null,
                Err(error) => return Err(error),
            };
            for season in seasons.as_array().into_iter().flatten() {
                let number = season.get("season").cloned().unwrap_or(Value::Null);
                let number_text = match &number {
                    Value::String(text) => text.clone(),
                    other => other.to_string(),
                };
                entries.extend(series_list_entries(
                    client,
                    &auth,
                    &format!(
                        "https://api.rctiplus.com/api/v2/program/{series_id}/episode?season={number_text}"
                    ),
                    &display_id,
                    &format!("Downloading season {number_text} episode entries"),
                    &metadata,
                )?);
            }
        }
        if video_type.as_deref().is_none_or(|kind| kind == "extras") {
            entries.extend(series_list_entries(
                client,
                &auth,
                &format!("https://api.rctiplus.com/api/v2/program/{series_id}/extra?content_id=0"),
                &display_id,
                "Downloading extra entries",
                &metadata,
            )?);
        }
        if video_type.as_deref().is_none_or(|kind| kind == "clips") {
            entries.extend(series_list_entries(
                client,
                &auth,
                &format!("https://api.rctiplus.com/api/v2/program/{series_id}/clip?content_id=0"),
                &display_id,
                "Downloading clip entries",
                &metadata,
            )?);
        }

        let title = series_meta
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let description = series_meta
            .get("summary")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut keys = vec!["display_id", "age_limit", "cast", "tags", "id"];
        let mut extra = Map::new();
        extra.insert("display_id".into(), display_id.clone().into());
        for (key, value) in &metadata {
            extra.insert((*key).into(), value.clone());
        }
        if title.is_some() {
            keys.push("title");
        }
        if description.is_some() {
            keys.push("description");
            extra.insert("description".into(), description.clone().into());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: series_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: SERIES_NAME.into(),
            extractor_key: SERIES_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `RCTIPlusSeriesIE.suitable`: a program page that is not an episode URL.
pub fn series_suitable(url: &str) -> bool {
    !VALID_URL.is_match(url) && SERIES_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.rctiplus.com/programs/616/inews-malam/extra/9438/diungkapkan-melalui-surat-terbuka-ceo-ruangguru-belva-devara-mundur-dari-staf-khusus-presiden"
        ));
        assert!(suitable(
            "https://www.rctiplus.com/missed-event/2507/mou-signing-ceremony-27-juli-2021-1400-wib"
        ));
        assert!(suitable("https://www.rctiplus.com/live-event/1/rcti"));
        assert!(!suitable(
            "https://www.rctiplus.com/programs/829/putri-untuk-pangeran"
        ));
    }

    #[test]
    fn upcoming_comparison_needs_comparable_fields() {
        let json = json!({"current_date": 5, "live_at": 9});
        assert_eq!(is_less(&json, "current_date", "live_at"), Some(true));
        assert_eq!(is_less(&json, "current_date", "start_date"), None);
    }
}
