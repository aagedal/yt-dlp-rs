//! Upstream `TvwIE`, `TvwNewsIE` and `TvwTvChannelsIE`
//! (`yt_dlp/extractor/tvw.py`): TVW's Invintus events (the detailed event
//! API's HLS streams), the news pages' embedded events as playlists, and the
//! live channel pages' persistent HLS stream.
use super::{
    Extractor, dict_info, download_webpage, extract_m3u8_fatal_live, extract_m3u8_master_optional,
    post_json_note_headers,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Tvw;
pub struct TvwNews;
pub struct TvwTvChannels;

const NAME: &str = "tvw";
const NEWS_NAME: &str = "tvw:news";
const CHANNEL_NAME: &str = "tvw:tvchannels";
const API_KEY: &str = "7WhiEBzijpritypp8bqcU7pfU9uicDR";

static EVENT_URLS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"^https?://(?:www\.)?tvw\.org/video/(?P<id>[^/?#]+)").unwrap(),
        Regex::new(r"^https?://(?:www\.)?tvw\.org/watch/?\?(?:[^#]+&)?eventID=(?P<id>\d+)")
            .unwrap(),
    ]
});
static NEWS_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?tvw\.org/\d{4}/\d{2}/(?P<id>[^/?#]+)").unwrap()
});
static CHANNEL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?tvw\.org/tvchannels/(?P<id>[^/?#]+)").unwrap()
});
static NEWS_PLAYER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<[a-z]+\s[^>]*\bclass="[^"]*\binvintus-player\b[^"]*"[^>]*>"#).unwrap()
});
static CHANNEL_FRAME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<iframe\s[^>]*\bid="invintus-persistent-stream-frame"[^>]*>"#).unwrap()
});

/// `_match_id` of the event URLs (the first matching pattern).
fn event_id(url: &str) -> Option<String> {
    EVENT_URLS
        .iter()
        .find_map(|pattern| pattern.captures(url))
        .map(|found| found["id"].to_owned())
}

/// `_match_valid_url` for the event URLs.
pub fn suitable(url: &str) -> bool {
    event_id(url).is_some()
}

/// `_match_valid_url` for news pages.
pub fn news_suitable(url: &str) -> bool {
    NEWS_URL.is_match(url)
}

/// `_match_valid_url` for live channel pages.
pub fn channel_suitable(url: &str) -> bool {
    CHANNEL_URL.is_match(url)
}

/// `_html_search_meta(name, html, fatal=True)` of a `name` or `property`.
fn meta_required(webpage: &str, name: &str, id: &str) -> Result<String> {
    crate::html::html_search_meta(webpage, name).ok_or_else(|| {
        Error::Reported(format!(
            "[{NAME}] {id}: Unable to extract {name}; {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })
}

/// `remove_end(value, ' - TVW')`.
fn strip_site(title: Option<String>) -> Option<String> {
    title.map(|title| {
        title
            .strip_suffix(" - TVW")
            .map_or(title.clone(), str::to_owned)
    })
}

/// `_og_search_thumbnail`/`_og_search_description` without a fatal error.
fn og(webpage: &str, prop: &str) -> Option<String> {
    crate::html::og_search_property(webpage, prop)
}

/// The URL result for an event (`watch?eventID=`), as a playlist entry.
fn event_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let entry = json!({"ie_key": "Tvw", "_type": "url", "url": target});
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": "Tvw",
            "extractor": NAME,
            "entry": entry,
        }),
    );
    Ok(info)
}

impl Extractor for Tvw {
    fn key(&self) -> &'static str {
        "Tvw"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(display_id) = event_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_match_id` gives the URL's ID for `/video/` and `eventID=` pages.
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let client_id = meta_required(&webpage, "clientID", &display_id)?;
        let video_id = meta_required(&webpage, "eventID", &display_id)?;

        let request = json!({"clientID": client_id, "eventID": video_id, "showStreams": true});
        let api = Url::parse("https://api.v3.invintus.com/v2/Event/getDetailed")?;
        let response = post_json_note_headers(
            client,
            &api,
            NAME,
            &video_id,
            "Downloading JSON metadata",
            &[("authorization", "embedder"), ("wsc-api-key", API_KEY)],
            request.to_string(),
        )?;
        let video_data = response
            .get("data")
            .ok_or_else(|| super::clyp::key_error(&video_id, "data"))?;

        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        let streams = video_data
            .get("streamingURIs")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|uris| uris.values())
            .filter_map(Value::as_str)
            .filter_map(|text| Url::parse(text).ok());
        for stream in streams {
            if let Some(found) = extract_m3u8_master_optional(
                client,
                &stream,
                NAME,
                &video_id,
                Some("hls"),
                true,
                false,
            ) {
                formats.extend(found.formats);
                super::svt::merge_subtitles(&mut subtitles, found.subtitles);
            }
        }
        let caption_path = video_data
            .get("captionPath")
            .and_then(Value::as_str)
            .filter(|text| Url::parse(text).is_ok());
        if let Some(caption) = caption_path {
            let track = crate::SubtitleFormat {
                name: None,
                url: Url::parse(caption)?,
                ext: "vtt".into(),
                protocol: None,
                fragments: Vec::new(),
                extra: Map::new(),
            };
            subtitles.entries.push((Some("en".into()), vec![track]));
        }

        let fields = vec![
            ("id", Value::from(video_id.as_str())),
            ("display_id", Value::from(display_id.as_str())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            (
                "title",
                video_data
                    .get("title")
                    .filter(|value| value.is_string())
                    .cloned()
                    .or_else(|| strip_site(og(&webpage, "title")).map(Value::from))
                    .unwrap_or(Value::Null),
            ),
            (
                "description",
                video_data
                    .get("description")
                    .and_then(Value::as_str)
                    .map(crate::html::clean_html)
                    .map(Value::from)
                    .or_else(|| og(&webpage, "description").map(Value::from))
                    .unwrap_or(Value::Null),
            ),
            ("categories", string_list(video_data.get("categories"))),
            (
                "thumbnail",
                optional_url(video_data.get("videoThumbnail"))
                    .as_str()
                    .map(Value::from)
                    .or_else(|| og(&webpage, "image").map(Value::from))
                    .unwrap_or(Value::Null),
            ),
            (
                "timestamp",
                video_data
                    .get("startDateTime")
                    .and_then(Value::as_str)
                    .and_then(|date| crate::dates::unified_timestamp(date, false))
                    .map_or(Value::Null, Value::from),
            ),
            (
                "location",
                video_data
                    .get("locationName")
                    .and_then(Value::as_str)
                    .map_or(Value::Null, Value::from),
            ),
            (
                "is_live",
                Value::Bool(video_data.get("eventStatus").and_then(Value::as_str) == Some("live")),
            ),
        ];
        let mut info = dict_info(url, (NAME, "Tvw"), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(ExtractResult::Video(info)))
    }
}

/// A list of strings (`categories`), or the key's absence.
fn string_list(value: Option<&Value>) -> Value {
    match value.and_then(Value::as_array) {
        Some(items) => Value::Array(
            items
                .iter()
                .filter(|item| item.is_string())
                .cloned()
                .collect(),
        ),
        None => Value::Null,
    }
}

/// `url_or_none` of a value.
fn optional_url(value: Option<&Value>) -> Value {
    match value
        .and_then(Value::as_str)
        .filter(|text| Url::parse(text).is_ok())
    {
        Some(text) => Value::from(text),
        None => Value::Null,
    }
}

impl Extractor for TvwNews {
    fn key(&self) -> &'static str {
        "TvwNews"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = NEWS_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(NEWS_NAME);
        crate::screen::screen(&format!(
            "[{NEWS_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NEWS_NAME, &playlist_id)?;
        // `orderedSet` of the players' event IDs.
        let mut events: Vec<String> = Vec::new();
        for player in NEWS_PLAYER.find_iter(&webpage) {
            let attrs = crate::html_elements::extract_attributes(player.as_str());
            let Some(Value::String(event)) = attrs.get("data-eventid") else {
                continue;
            };
            if !events.contains(event) {
                events.push(event.clone());
            }
        }
        let entries = events
            .iter()
            .map(|event| event_entry(&format!("https://tvw.org/watch?eventID={event}")))
            .collect::<Result<Vec<_>>>()?;
        let mut extra = Map::new();
        let description = og(&webpage, "description");
        let mut keys = vec!["id", "title"];
        if description.is_some() {
            keys.push("description");
        }
        extra.insert(
            "description".into(),
            description.map_or(Value::Null, Value::from),
        );
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: strip_site(og(&webpage, "title")).unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: NEWS_NAME.into(),
            extractor_key: "TvwNews".into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

impl Extractor for TvwTvChannels {
    fn key(&self) -> &'static str {
        "TvwTvChannels"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CHANNEL_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(CHANNEL_NAME);
        crate::screen::screen(&format!(
            "[{CHANNEL_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, CHANNEL_NAME, &video_id)?;
        let stream = channel_stream(&webpage).ok_or_else(|| {
            Error::Reported(format!(
                "[{CHANNEL_NAME}] {video_id}: Unable to extract stream url"
            ))
        })?;
        let formats =
            extract_m3u8_fatal_live(client, &stream, CHANNEL_NAME, &video_id, Some("hls"), false)?;
        let fields = vec![
            ("id", Value::from(video_id.as_str())),
            ("formats", Value::Null),
            (
                "title",
                strip_site(og(&webpage, "title")).map_or(Value::Null, Value::from),
            ),
            (
                "thumbnail",
                og(&webpage, "image").map_or(Value::Null, Value::from),
            ),
            ("is_live", Value::Bool(true)),
        ];
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (CHANNEL_NAME, "TvwTvChannels"),
            fields,
            formats.formats,
        ))))
    }
}

/// The `live247URI` of the persistent stream's `encoder` parameter.
fn channel_stream(webpage: &str) -> Option<Url> {
    let frame = CHANNEL_FRAME.find(webpage)?;
    let attrs = crate::html_elements::extract_attributes(frame.as_str());
    let Value::String(src) = attrs.get("src")? else {
        return None;
    };
    let src = Url::parse(src).ok()?;
    let encoder = src
        .query_pairs()
        .find(|(key, _)| key == "encoder")?
        .1
        .into_owned();
    let encoder: Value = serde_json::from_str(&encoder).ok()?;
    let live = encoder.get("live247URI")?.as_str()?;
    Url::parse(live).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://tvw.org/video/billy-frank-jr-statue-maquette-unveiling-ceremony-2024011211/"
        ));
        assert!(suitable(
            "https://tvw.org/video/home-warranties-workgroup-2"
        ));
        assert!(suitable("https://tvw.org/watch?eventID=2025041235"));
        assert!(!suitable("https://tvw.org/"));
        assert!(news_suitable(
            "https://tvw.org/2024/06/the-impact-water-rights-and-the-skookumchuck-dam-debate/"
        ));
        assert!(channel_suitable("https://tvw.org/tvchannels/air/"));
        assert!(!channel_suitable("https://tvw.org/tvchannels/"));
    }
}
