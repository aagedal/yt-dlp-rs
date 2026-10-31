//! Upstream `AfreecaTVIE` (VODs), `AfreecaTVCatchStoryIE` and
//! `AfreecaTVUserIE` (`yt_dlp/extractor/afreecatv.py`, sooplive.com): a public
//! VOD's `api.m.sooplive.com` JSON and its HLS or HTTP files, a catch story's
//! clips from `api.m.sooplive.com/catchstory/a/view` as direct URL entries,
//! and a station's VOD list as `url` entries. The live stream
//! (`AfreecaTVLiveIE`) and the subscriber-only VOD path (CloudFront cookies)
//! are not ported; they are reported as unsupported.
use super::{
    Extractor, dict_format, dict_info, direct, download_json_note_headers, post_json_errnote,
    update_url_query,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct AfreecaTv;
pub struct AfreecaTvCatchStory;
pub struct AfreecaTvUser;

const VOD_NAME: &str = "soop";
const VOD_KEY: &str = "AfreecaTV";
const NAME: &str = "soop:catchstory";
const KEY: &str = "AfreecaTVCatchStory";
const USER_NAME: &str = "soop:user";
const USER_KEY: &str = "AfreecaTVUser";
const API: &str = "https://api.m.sooplive.com";
/// `AfreecaTVUserIE._PER_PAGE`.
const USER_PAGE_SIZE: usize = 60;

static CATCH_URL: LazyLock<Regex> = LazyLock::new(|| {
    super::url_regex(r"^https?://vod\.sooplive\.com/player/(?P<id>\d+)/catchstory").unwrap()
});
static VOD_URL: LazyLock<Regex> = LazyLock::new(|| {
    super::url_regex(
        r"^https?://vod\.sooplive\.com/(?:PLAYER/STATION|player)/(?P<id>\d+)/?(?:$|[?#&])",
    )
    .unwrap()
});
static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    super::url_regex(
        r"^https?://(?:www\.)?sooplive\.com/station/(?P<id>[^/?#]+)/vod/?(?P<slug_type>[^/?#]+)?",
    )
    .unwrap()
});
static ZONE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:Z|[+-]\d{2}:?\d{2})$").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    CATCH_URL.is_match(url)
}

/// `_fixup_thumb`: the thumbnail as a `jpg` (its URL ends in a PHP script).
fn fixup_thumb(thumb: Option<&Value>) -> Option<Value> {
    let url = thumb?.as_str()?;
    (!url.is_empty()).then(|| json!([{ "url": url, "ext": "jpg", "id": "0" }]))
}

/// `int_or_none(value, scale=1000)`.
fn scaled_int(value: Option<&Value>) -> Option<Value> {
    let number = match value? {
        Value::Number(number) => number.as_i64()?,
        Value::String(text) => text.trim().parse::<i64>().ok()?,
        _ => return None,
    };
    Some((number / 1000).into())
}

/// A string-valued field (`{str}`), else `None`.
fn text(value: Option<&Value>) -> Option<Value> {
    match value {
        Some(Value::String(text)) => Some(text.clone().into()),
        _ => None,
    }
}

/// A direct URL entry with `fields` (in order): the URL as its one format,
/// as upstream's `url` results of a playlist.
fn direct_entry(page: &Url, url: &str, fields: Vec<(&str, Value)>) -> Result<VideoInfo> {
    let media = Url::parse(url)?;
    let ext = direct::determine_ext(url).unwrap_or_default();
    let mut info = direct::with_format(direct::build_at(media.clone(), &media, ext.clone()), "");
    for format in &mut info.formats {
        format.ext = ext.clone();
        format.format_id = String::new();
    }
    info.format_id = None;
    info.webpage_url = page.clone();
    info.direct = None;
    info.extractor = NAME.into();
    info.extractor_key = KEY.into();
    let mut keys = Vec::new();
    for (name, value) in fields {
        keys.push(name.to_owned());
        match name {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            _ => crate::metadata_parser::set_field(&mut info, name, value),
        }
    }
    let shape = format!("untyped-dict;{};thumbnail=url,ext", keys.join(","));
    Ok(crate::info_order::mark_info(info, &shape))
}

impl Extractor for AfreecaTvCatchStory {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CATCH_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let api = Url::parse(&format!(
            "{API}/catchstory/a/view?aStoryListIdx=&nStoryIdx={video_id}"
        ))?;
        let data = download_json_note_headers(
            client,
            &api,
            "soop",
            &video_id,
            "Downloading API JSON",
            &[("Referer", url.as_str())],
        )?;

        let mut entries = Vec::new();
        for story in data
            .get("data")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if story.get("story_type").and_then(Value::as_str) != Some("catch") {
                continue;
            }
            for clip in story
                .get("catch_list")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                // `lambda _, v: v['files'][0]['file']`: a truthy file.
                let first = clip.get("files").and_then(|files| files.get(0));
                let Some(file) = first
                    .and_then(|first| first.get("file"))
                    .filter(|file| super::nuxt::truthy(file))
                else {
                    continue;
                };
                let file = file.as_str().unwrap_or_default();
                // Upstream's `url_or_none` drops anything else.
                if !(file.starts_with("https://") || file.starts_with("http://")) {
                    continue;
                }
                // Upstream's dict order: traverse_obj keeps the present keys.
                let mut fields: Vec<(&str, Value)> = Vec::new();
                if let Some(id) = text(first.and_then(|first| first.get("file_info_key"))) {
                    fields.push(("id", id));
                }
                fields.push(("url", file.into()));
                if let Some(duration) = scaled_int(first.and_then(|first| first.get("duration"))) {
                    fields.push(("duration", duration));
                }
                if let Some(title) = text(clip.get("title")) {
                    fields.push(("title", title));
                }
                if let Some(uploader) = text(clip.get("writer_nick")) {
                    fields.push(("uploader", uploader));
                }
                if let Some(uploader_id) = text(clip.get("writer_id")) {
                    fields.push(("uploader_id", uploader_id));
                }
                if let Some(thumbnails) = fixup_thumb(clip.get("thumb")) {
                    fields.push(("thumbnails", thumbnails));
                }
                if let Some(timestamp) = clip.get("write_timestamp").and_then(Value::as_i64) {
                    fields.push(("timestamp", timestamp.into()));
                }
                entries.push(direct_entry(url, file, fields)?);
            }
        }

        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: video_id,
            title: String::new(),
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

/// `_match_valid_url` of [`AfreecaTv`].
pub fn vod_suitable(url: &str) -> bool {
    VOD_URL.is_match(url)
}

/// `_match_valid_url` of [`AfreecaTvUser`].
pub fn user_suitable(url: &str) -> bool {
    USER_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("AfreecaTV {what} are not supported natively"))
}

/// A VOD part: its info fields and formats.
type PartEntry<'a> = (Vec<(&'a str, Value)>, Vec<crate::VideoFormat>);

/// `_fixup_thumb` of a VOD: the thumbnail as a `jpg` (its URL ends in a PHP
/// script, so the extension is given).
fn vod_thumbnail(thumb: Option<&Value>) -> Option<Value> {
    let url = thumb?.as_str()?;
    (!url.is_empty()).then(|| json!([{ "url": url, "ext": "jpg" }]))
}

/// `url_or_none` of a `file` value.
fn file_url(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (text.starts_with("https://") || text.starts_with("http://")).then(|| text.to_owned())
}

/// `_call_api(endpoint, id, data=..., headers=...)` of a form POST.
fn call_api_post(
    client: &Client,
    endpoint: &str,
    id: &str,
    body: String,
    referer: &Url,
) -> Result<Value> {
    let api = Url::parse(&format!("{API}/{endpoint}"))?;
    post_json_errnote(
        client,
        &api,
        (VOD_NAME, id),
        ("Downloading API JSON", "Unable to download API JSON"),
        &[
            ("Referer", referer.as_str()),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        body,
    )
}

/// `parse_iso8601(file_start, delimiter=' ', timezone=+9h)`: a date without
/// its own zone is in `+09:00`.
fn file_timestamp(start: &str) -> Option<i64> {
    let parsed = crate::dates::parse_iso8601_space(start)?;
    Some(if ZONE.is_match(start) {
        parsed
    } else {
        parsed - 9 * 3600
    })
}

/// Sets `key` in place (keeping its position) or appends it.
fn upsert<'a>(fields: &mut Vec<(&'a str, Value)>, key: &'a str, value: Value) {
    match fields.iter_mut().find(|(name, _)| *name == key) {
        Some((_, slot)) => *slot = value,
        None => fields.push((key, value)),
    }
}

/// A `url_result` of the VOD extractor (a user's VOD list entry): its IE key,
/// then the given fields, then the type and URL, in upstream's order.
fn vod_url_entry(target: &str, fields: Vec<(&str, Value)>) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), VOD_KEY.into());
    for (key, value) in fields {
        entry.insert(key.into(), value);
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": VOD_KEY,
            "extractor": VOD_NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for AfreecaTv {
    fn key(&self) -> &'static str {
        VOD_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VOD_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(VOD_NAME, url);

        let body = format!("nTitleNo={video_id}&nApiLevel=10");
        let response = call_api_post(client, "station/video/a/view", &video_id, body, url)?;
        let data = response
            .get("data")
            .ok_or_else(|| super::clyp::key_error(&video_id, "data"))?;

        // `needs_private_auth`: subscriber-only VODs (their HLS files need
        // the CloudFront cookies, which are not ported).
        let needs_private_auth = data
            .get("sub_upload_type")
            .and_then(Value::as_str)
            .is_some_and(|kind| !kind.is_empty());

        match data.get("code").and_then(Value::as_i64) {
            Some(-6221) => {
                return Err(Error::Reported(format!(
                    "[{VOD_NAME}] {video_id}: The VOD does not exist"
                )));
            }
            Some(-6205) => {
                return Err(Error::Reported(format!(
                    "[{VOD_NAME}] {video_id}: This VOD is private"
                )));
            }
            _ => {}
        }

        // `traverse_obj` mapping: only the present keys, in this order.
        let mut common: Vec<(&str, Value)> = Vec::new();
        if let Some(title) = text(data.get("title")) {
            common.push(("title", title));
        }
        if let Some(uploader) = text(data.get("writer_nick")) {
            common.push(("uploader", uploader));
        }
        if let Some(uploader_id) = text(data.get("bj_id")) {
            common.push(("uploader_id", uploader_id));
        }
        if let Some(duration) = scaled_int(data.get("total_file_duration")) {
            common.push(("duration", duration));
        }
        if let Some(thumbnails) = vod_thumbnail(data.get("thumb")) {
            common.push(("thumbnails", thumbnails));
        }
        let common_title = common
            .iter()
            .find(|(key, _)| *key == "title")
            .map_or(Value::Null, |(_, value)| value.clone());

        // Each part: its fields and formats.
        let mut entries: Vec<PartEntry> = Vec::new();
        let files = data.get("files").and_then(Value::as_array);
        for (index, element) in files.into_iter().flatten().enumerate() {
            let Some(file) = file_url(element.get("file")) else {
                continue;
            };
            let file_num = index + 1;
            let formats = if direct::determine_ext(&file).as_deref() == Some("m3u8") {
                if needs_private_auth {
                    return Err(unsupported("subscriber-only HLS files"));
                }
                let manifest = Url::parse(&file)?;
                super::extract_m3u8_fatal(
                    client,
                    &manifest,
                    VOD_NAME,
                    &video_id,
                    Some("hls"),
                    false,
                )?
                .formats
            } else {
                vec![dict_format(
                    vec![("url", file.clone().into()), ("format_id", "http".into())],
                    VOD_NAME,
                    &video_id,
                )?]
            };
            let id = match element.get("file_info_key") {
                Some(Value::String(key)) if !key.is_empty() => key.clone(),
                _ => format!("{video_id}_{file_num}"),
            };
            let mut fields: Vec<(&str, Value)> = common.clone();
            // `{**common_info, 'id', 'title', 'formats', **file_fields}`.
            let part_title = match &common_title {
                Value::String(title) if !title.is_empty() => title.clone(),
                _ => "Untitled".to_owned(),
            };
            upsert(
                &mut fields,
                "title",
                format!("{part_title} (part {file_num})").into(),
            );
            fields.push(("id", id.into()));
            fields.push(("formats", Value::Null));
            if let Some(duration) = scaled_int(element.get("duration")) {
                upsert(&mut fields, "duration", duration);
            }
            if let Some(timestamp) = element
                .get("file_start")
                .and_then(Value::as_str)
                .and_then(file_timestamp)
            {
                fields.push(("timestamp", timestamp.into()));
            }
            entries.push((fields, formats));
        }

        if data.get("adult_status").and_then(Value::as_str) == Some("notLogin") {
            if entries.is_empty() {
                return Err(Error::Reported(format!(
                    "[{VOD_NAME}] {video_id}: {}",
                    crate::extractor::login::login_required_message(
                        "Only users older than 19 are able to watch this video",
                        Some("afreecatv"),
                        Some(crate::extractor::login::Hint::Password),
                    )
                )));
            }
            return Err(unsupported("age-restricted VODs with some parts visible"));
        }

        if entries.is_empty() {
            if needs_private_auth {
                return Err(Error::Reported(format!(
                    "[{VOD_NAME}] {video_id}: {}",
                    crate::extractor::login::login_required_message(
                        "This VOD is for subscribers only",
                        Some("afreecatv"),
                        Some(crate::extractor::login::Hint::Password),
                    )
                )));
            }
            return Err(unsupported("VODs without any files"));
        }
        if entries.len() > 1 {
            return Err(unsupported("multi-part VODs"));
        }

        // `{**entries[0], 'title': common_info.get('title')}`.
        let (mut fields, formats) = entries.remove(0);
        let title = common
            .iter()
            .find(|(key, _)| *key == "title")
            .map_or(Value::Null, |(_, value)| value.clone());
        upsert(&mut fields, "title", title);
        let mut info = dict_info(url, (VOD_NAME, VOD_KEY), fields, formats);
        // The `_fixup_thumb` dict keeps `url`, `ext`, then the sanitizer's `id`.
        if let Some(Value::String(shape)) = info.extra.get(crate::info_order::INFO_SHAPE).cloned() {
            info.extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                format!("{shape};thumbnail=url,ext").into(),
            );
        }
        Ok(Some(info))
    }
}

/// `AfreecaTVUserIE._fetch_page`: one page of a station's VOD list as `url`
/// entries of the VOD extractor (`page` is 0-based).
fn user_page(
    client: &Client,
    user_id: &str,
    user_type: &str,
    page: usize,
) -> Result<Vec<VideoInfo>> {
    let page_number = (page + 1).to_string();
    let per_page = USER_PAGE_SIZE.to_string();
    let api = update_url_query(
        &Url::parse(&format!(
            "https://chapi.sooplive.com/api/{user_id}/vods/{user_type}"
        ))?,
        &[
            ("page", page_number.as_str()),
            ("per_page", per_page.as_str()),
            ("orderby", "reg_date"),
        ],
    );
    let info = download_json_note_headers(
        client,
        &api,
        USER_NAME,
        user_id,
        &format!("Downloading {user_type} video page {page_number}"),
        &[],
    )?;
    let items = info
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| super::clyp::key_error(user_id, "data"))?;
    let mut entries = Vec::new();
    for item in items {
        let title_no = match item.get("title_no") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Number(number)) => number.to_string(),
            _ => return Err(super::clyp::key_error(user_id, "title_no")),
        };
        let title = item.get("title_name").cloned().unwrap_or(Value::Null);
        entries.push(vod_url_entry(
            &format!("https://vod.sooplive.com/player/{title_no}/"),
            vec![("id", title_no.into()), ("title", title)],
        )?);
    }
    Ok(entries)
}

impl Extractor for AfreecaTvUser {
    fn key(&self) -> &'static str {
        USER_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = USER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let user_id = found["id"].to_owned();
        let user_type = found
            .name("slug_type")
            .map_or_else(|| "all".to_owned(), |kind| kind.as_str().to_owned());
        announce(USER_NAME, url);

        // `OnDemandPagedList` of pages of 60: a page that is not full ends it.
        let mut entries = Vec::new();
        for page in 0.. {
            let page_entries = user_page(client, &user_id, &user_type, page)?;
            let full = page_entries.len() >= USER_PAGE_SIZE;
            entries.extend(page_entries);
            if !full {
                break;
            }
        }

        let title = format!("{user_id} - {user_type}");
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: user_id,
            title,
            webpage_url: url.clone(),
            extractor: USER_NAME.into(),
            extractor_key: USER_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(vod_suitable("https://vod.sooplive.com/player/192805325"));
        assert!(vod_suitable(
            "https://vod.sooplive.com/PLAYER/STATION/20515605"
        ));
        assert!(!vod_suitable(
            "https://vod.sooplive.com/player/103247/catchstory"
        ));
        assert!(user_suitable(
            "https://www.sooplive.com/station/devil0108/vod/review"
        ));
        assert!(!user_suitable("https://vod.sooplive.com/player/192805325"));
        assert!(suitable(
            "https://vod.sooplive.com/player/103247/catchstory"
        ));
        assert!(!suitable("https://vod.sooplive.com/player/192805325"));
    }

    #[test]
    fn durations_scale_from_milliseconds() {
        assert_eq!(scaled_int(Some(&json!(16_999))), Some(json!(16)));
        assert_eq!(scaled_int(Some(&json!("13000"))), Some(json!(13)));
    }
}
