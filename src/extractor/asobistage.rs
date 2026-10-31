//! Upstream `AsobiStageIE` (`yt_dlp/extractor/asobistage.py`): an event's
//! archive page (`/event/EVENT/archive/SLUG`) as a playlist with one entry
//! per archive channel whose survapi stream is listed for the event. Anonymous
//! visitors only: `_owned_tickets` is always empty, so channels that need a
//! ticket are skipped with upstream's warning. Live (`player`) pages are
//! unsupported natively.
use super::{
    Extractor, dict_info, download_json_note, download_webpage, extract_m3u8_master_optional,
    search_nextjs_data, search_warning,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

pub struct AsobiStage;

const NAME: &str = "AsobiStage";
const API_HOST: &str = "https://asobistage-api.asobistore.jp";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://asobistage\.asobistore\.jp/event/(?P<id>(?P<event>\w+)/(?P<type>archive|player)/(?P<slug>\w+))(?:[?#]|$)",
    )
    .unwrap()
});

/// `_match_valid_url(url)`: the event, type and slug of an event page.
fn match_page(url: &str) -> Option<(String, String, String, String)> {
    let found = VALID_URL.captures(url)?;
    Some((
        found["id"].to_owned(),
        found["event"].to_owned(),
        found["type"].to_owned(),
        found["slug"].to_owned(),
    ))
}

fn non_empty_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// `_download_json(url, video_id, note, errnote, fatal=False, headers=...)`:
/// an HTTP error is a warning and no JSON.
fn download_json_optional(
    client: &Client,
    url: &Url,
    id: &str,
    note: &str,
    errnote: &str,
    headers: &[(&str, &str)],
) -> Result<Option<Value>> {
    crate::screen::screen(&format!("[{NAME}] {id}: {note}"));
    let mut response = match super::direct::request_expecting(client, url, headers, &[]) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: {errnote}: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = super::decode_webpage(&body, response.headers());
    match crate::json::from_str::<Value>(&text) {
        Ok(value) => Ok(Some(value)),
        Err(_) => {
            crate::screen::warning(&format!("[{NAME}] {id}: Unable to parse JSON"));
            Ok(None)
        }
    }
}

/// `_get_available_channel_id(channel)` for an anonymous visitor: the
/// channel's VSPF ID when it is free (`rights_type_id` 6) or when it has no
/// ticket to check against; a ticket-only channel is skipped with a warning.
fn available_channel_id(channel: &Value) -> Option<String> {
    let channel_id = non_empty_str(channel.get("chennel_vspf_id"))?.to_owned();
    let viewrights = channel
        .get("viewrights")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    if viewrights
        .iter()
        .any(|rights| rights.get("rights_type_id").and_then(Value::as_i64) == Some(6))
    {
        return Some(channel_id);
    }
    let name = non_empty_str(channel.get("channel_name")).unwrap_or(&channel_id);
    crate::screen::warning(&format!(
        "[{NAME}] You are not a ticketholder for \"{name}\""
    ));
    None
}

/// `traverse_obj(json, (video_type, lambda _, v: v['broadcast_slug'] == slug,
/// 'channels', lambda _, v: v['chennel_vspf_id'] != '00000'))`.
fn archive_channels<'a>(listing: &'a Value, video_type: &str, slug: &str) -> Vec<&'a Value> {
    listing
        .get(video_type)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|item| item.get("broadcast_slug").and_then(Value::as_str) == Some(slug))
        .filter_map(|item| item.get("channels").and_then(Value::as_array))
        .flatten()
        .filter(|channel| channel.get("chennel_vspf_id").and_then(Value::as_str) != Some("00000"))
        .collect()
}

/// `playlist_result(entries, video_id, **event_data)`.
fn playlist(
    url: &Url,
    video_id: &str,
    title: Option<String>,
    thumbnail: Option<String>,
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut keys = Vec::new();
    let mut extra = Map::new();
    if let Some(title) = &title {
        keys.push("title");
        extra.insert("title".into(), json!(title));
    }
    if let Some(thumbnail) = &thumbnail {
        keys.push("thumbnail");
        extra.insert("thumbnail".into(), json!(thumbnail));
    }
    keys.push("id");
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id: video_id.to_owned(),
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: NAME.into(),
        extractor_key: NAME.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

impl Extractor for AsobiStage {
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
        let Some((video_id, event, type_, slug)) = match_page(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_real_initialize`: the bearer token for the survapi calls.
        let token = download_json_note(
            client,
            &Url::parse(&format!("{API_HOST}/api/v1/vspf/token"))?,
            NAME,
            &video_id,
            "Getting token",
        )?;
        let survapi_header = format!("Bearer {}", token.as_str().unwrap_or_default());

        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `_search_nextjs_data(webpage, video_id, default={})`.
        let event_cms = match search_nextjs_data(&webpage, NAME, &video_id) {
            Ok(data) => data
                .get("props")
                .and_then(|props| props.get("pageProps"))
                .and_then(|page| page.get("eventCMSData"))
                .cloned()
                .unwrap_or(Value::Null),
            Err(_) => {
                search_warning(NAME, "next.js data");
                Value::Null
            }
        };
        let title = non_empty_str(event_cms.get("event_name")).map(str::to_owned);
        let thumbnail = non_empty_str(event_cms.get("event_thumbnail_image"))
            .and_then(super::json_ld::url_or_none);

        if type_ == "player" {
            return Err(Error::UnsupportedHtml5(
                "AsobiStage live (player) events are not supported natively".into(),
            ));
        }
        let video_type = "archives";
        let listing = download_json_note(
            client,
            &Url::parse(&format!(
                "https://asobistage.asobistore.jp/cdn/v101/events/{event}/{video_type}.json"
            ))?,
            NAME,
            &video_id,
            "Getting channel list",
        )?;

        let mut entries = Vec::new();
        for channel in archive_channels(&listing, video_type, &slug) {
            let Some(channel_id) = available_channel_id(channel) else {
                continue;
            };
            let channel_json = download_json_optional(
                client,
                &Url::parse(&format!(
                    "https://survapi.channel.or.jp/proxy/v1/contents/{channel_id}/get_by_cuid"
                ))?,
                &channel_id,
                "Getting archive channel info",
                "Unable to get archive channel info",
                &[("Authorization", survapi_header.as_str())],
            )?;
            let content = channel_json
                .as_ref()
                .and_then(|json| json.get("ex_content"))
                .cloned()
                .unwrap_or(Value::Null);
            let m3u8_url = non_empty_str(content.get("streaming_url"));
            let channel_title = non_empty_str(content.get("title")).map(str::to_owned);
            let channel_thumbnail = content
                .get("thumbnail")
                .and_then(|thumb| thumb.get("url"))
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none);
            let formats = match m3u8_url {
                Some(manifest) => extract_m3u8_master_optional(
                    client,
                    &Url::parse(manifest)?,
                    NAME,
                    &channel_id,
                    None,
                    false,
                    false,
                )
                .map(|info| info.formats)
                .unwrap_or_default(),
                None => Vec::new(),
            };
            let fields = vec![
                ("id", json!(channel_id)),
                ("title", title_value(channel_title)),
                ("formats", Value::Null),
                ("is_live", json!(false)),
                ("thumbnail", thumbnail_value(channel_thumbnail)),
            ];
            entries.push(dict_info(url, (NAME, NAME), fields, formats));
        }

        if entries.is_empty() {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: This video is only available for registered users. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies"
            )));
        }
        Ok(Some(playlist(url, &video_id, title, thumbnail, entries)))
    }
}

fn title_value(title: Option<String>) -> Value {
    title.map_or(Value::Null, Value::from)
}

fn thumbnail_value(thumbnail: Option<String>) -> Value {
    thumbnail.map_or(Value::Null, Value::from)
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://asobistage.asobistore.jp/event/idolmaster_idolworld2023_goods/archive/live"
        ));
        assert!(suitable(
            "https://asobistage.asobistore.jp/event/315passionhour_2022summer/archive/frame"
        ));
        assert!(suitable(
            "https://asobistage.asobistore.jp/event/ijigenfes_utagassen/player/day1"
        ));
        assert!(!suitable(
            "https://asobistage.asobistore.jp/event/x/other/y"
        ));
    }

    #[test]
    fn archive_channels_filter_by_slug_and_channel_id() {
        let listing = json!({"archives": [
            {"broadcast_slug": "live", "channels": [
                {"chennel_vspf_id": "00000"},
                {"chennel_vspf_id": "3aef7110"}
            ]},
            {"broadcast_slug": "other", "channels": [{"chennel_vspf_id": "1"}]}
        ]});
        let channels = archive_channels(&listing, "archives", "live");
        assert_eq!(channels.len(), 1);
        assert_eq!(channels[0]["chennel_vspf_id"], "3aef7110");
    }
}
