//! Upstream `TelegramEmbedIE` (`yt_dlp/extractor/telegram.py`): a public
//! `t.me` post's embed frame, its video players as formats and the message
//! text, author and date; posts with several videos are playlists unless the
//! URL has `single` or `--no-playlist` picks the post's own video.
use super::Extractor;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TelegramEmbed;

const NAME: &str = "telegram:embed";
const KEY: &str = "TelegramEmbed";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://t\.me/(?P<channel_id>[^/]+)/(?P<id>\d+)").unwrap()
});
static TIMESTAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<time[^>]*datetime="([^"]*)""#).unwrap());
static VIDEO_PLAYER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a class="tgme_widget_message_video_player(?s:.+?)</time>"#).unwrap()
});
static VIDEO_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<video[^>]+src="([^"]+)""#).unwrap());
static WEBPAGE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<a class="tgme_widget_message_video_player[^>]+href="([^"]+)""#).unwrap()
});
static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<time[^>]+duration[^>]*>([\d:]+)</time>").unwrap());
static THUMBNAIL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"tgme_widget_message_video_thumb"[^>]+background-image:url\('([^']+)'\)"#).unwrap()
});

/// `_VALID_URL`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_regex(pattern, text, name, fatal=False)`: the first group, or a
/// warning.
fn search(pattern: &Regex, text: &str, name: &str) -> Option<String> {
    let found = pattern
        .captures(text)
        .and_then(|found| found.get(1))
        .map(|found| found.as_str().to_owned());
    if found.is_none() {
        super::search_warning(NAME, name);
    }
    found
}

/// `clean_text(html_class, html)`: the first element's cleaned text on one
/// line, `None` when empty.
fn clean_text(class: &str, html: &str, id: &str) -> Result<Option<String>> {
    let found = crate::html_elements::elements_text_and_html_by_class(class, html)
        .map_err(|error| {
            Error::Reported(format!(
                "[{NAME}] {id}: An extractor error has occurred. (caused by HTMLParseError('{error}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?
        .into_iter()
        .next();
    Ok(found
        .map(|(content, _)| crate::html::clean_html(&content))
        .filter(|text| !text.is_empty())
        .map(|text| text.replace('\n', " ")))
}

/// `update_url_query(url, query)` for the embed frame: `embed=1` and no
/// `single` (`'single': []`), blank values dropped like `parse_qs`.
fn embed_url(url: &Url) -> Url {
    let mut without_single = url.clone();
    let pairs: Vec<(String, String)> = url
        .query_pairs()
        .filter(|(key, _)| key != "single")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    if pairs.is_empty() {
        without_single.set_query(None);
    } else {
        without_single
            .query_pairs_mut()
            .clear()
            .extend_pairs(&pairs);
    }
    super::update_url_query(&without_single, &[("embed", "1")])
}

/// `parse_qs(url, keep_blank_values=True)` has `single`.
fn has_single(url: &Url) -> bool {
    url.query_pairs().any(|(key, _)| key == "single")
}

fn video(
    dict: Map<String, Value>,
    format: crate::VideoFormat,
    webpage_url: Url,
) -> Result<VideoInfo> {
    let first = format.url.clone();
    let mut info = super::direct::build_at(first.clone(), &first, String::new());
    info.direct = None;
    info.formats = vec![format];
    info.webpage_url = webpage_url;
    info.extractor = NAME.into();
    info.extractor_key = KEY.into();
    let mut keys = Vec::new();
    for (key, value) in dict {
        keys.push(key.clone());
        match key.as_str() {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
            "formats" | "webpage_url" => {}
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    Ok(crate::info_order::mark_info(
        info,
        &format!("dict;{}", keys.join(",")),
    ))
}

impl Extractor for TelegramEmbed {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (channel_id, msg_id) = (found["channel_id"].to_owned(), found["id"].to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let embed = super::download_webpage_note(
            client,
            &embed_url(url),
            NAME,
            &msg_id,
            "Downloading embed frame",
        )?;
        let description = clean_text("tgme_widget_message_text", &embed, &msg_id)?;
        let channel = clean_text("tgme_widget_message_author", &embed, &msg_id)?;
        let timestamp = search(&TIMESTAMP, &embed, "timestamp")
            .and_then(|date| crate::dates::unified_timestamp(&date, true));
        let mut message = Map::new();
        message.insert(
            "title".into(),
            description.clone().unwrap_or_default().into(),
        );
        message.insert("description".into(), description.clone().into());
        message.insert("channel".into(), channel.clone().into());
        message.insert("channel_id".into(), channel_id.clone().into());
        message.insert("timestamp".into(), timestamp.into());

        let mut videos: Vec<(String, VideoInfo)> = Vec::new();
        for player in VIDEO_PLAYER.find_iter(&embed) {
            let player = player.as_str();
            let video_url = search(&VIDEO_URL, player, "video URL");
            let webpage_url = search(&WEBPAGE_URL, player, "webpage URL");
            let (Some(video_url), Some(webpage_url)) = (video_url, webpage_url) else {
                continue;
            };
            let video_url = Url::parse(&video_url)?;
            let webpage_url = Url::parse(&webpage_url)?;
            let id = crate::info::url_basename(&webpage_url);
            let mut format: crate::VideoFormat = serde_json::from_value(
                json!({"url": video_url.as_str(), "ext": "mp4", "format_id": ""}),
            )
            .map_err(|error| Error::Reported(format!("[{NAME}] {msg_id}: {error}")))?;
            crate::info_order::mark_format(&mut format, "dict:url,ext");
            let duration = search(&DURATION, player, "duration")
                .and_then(|duration| crate::parse_duration(&duration));
            let thumbnail = search(&THUMBNAIL, player, "thumbnail");
            // `update_url_query(webpage_url, {'single': True})`.
            let page = super::update_url_query(&webpage_url, &[("single", "True")]);
            let mut dict = Map::new();
            dict.insert("id".into(), id.clone().into());
            dict.insert("webpage_url".into(), page.as_str().into());
            // `process_video_result` pops a `None` (or non-positive) duration.
            if let Some(duration) = duration.filter(|duration| duration.as_f64() > 0.0) {
                dict.insert("duration".into(), json!(duration));
            }
            dict.insert("thumbnail".into(), thumbnail.into());
            dict.insert("formats".into(), json!([]));
            dict.extend(message.clone());
            videos.push((id, video(dict, format, page)?));
        }

        let playlist_id =
            (videos.len() > 1 && !has_single(url)).then(|| format!("{channel_id}-{msg_id}"));
        // `_yes_playlist(playlist_id, msg_id)`.
        let yes_playlist = match &playlist_id {
            None => false,
            Some(_) if super::no_playlist() => {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading just the video {msg_id} because of --no-playlist"
                ));
                false
            }
            Some(playlist_id) => {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading playlist {playlist_id} - add --no-playlist to download just the video {msg_id}"
                ));
                true
            }
        };
        if !yes_playlist {
            // `traverse_obj(videos, lambda _, x: x['id'] == msg_id, get_all=False)`;
            // upstream warns that the extractor returned nothing otherwise.
            return match videos.into_iter().find(|(id, _)| *id == msg_id) {
                Some((_, info)) => Ok(Some(ExtractResult::Video(info))),
                None => Err(Error::UnsupportedHtml5(format!(
                    "Telegram post {channel_id}/{msg_id} has no video of its own; upstream returns nothing for it"
                ))),
            };
        }
        // `playlist_result(videos, playlist_id, format_field(message,
        // 'channel', f'%s {msg_id}'), description)`.
        let title = channel
            .map(|channel| format!("{channel} {msg_id}"))
            .unwrap_or_default();
        let mut keys = vec!["id"];
        if !title.is_empty() {
            keys.push("title");
        }
        let mut extra = Map::new();
        if let Some(description) = description {
            keys.push("description");
            extra.insert("description".into(), description.into());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id.unwrap_or_default(),
            title,
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries: videos.into_iter().map(|(_, info)| info).collect(),
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://t.me/europa_press/613"));
        assert!(suitable("https://t.me/vorposte/29342?single"));
        assert!(!suitable("https://t.me/europa_press"));
        assert!(!suitable("https://t.me/s/europa_press"));
        assert!(!suitable("https://telegram.me/europa_press/613"));
    }

    #[test]
    fn embed_frame_query_drops_single() {
        let embed = |url: &str| embed_url(&Url::parse(url).unwrap()).to_string();
        assert_eq!(
            embed("https://t.me/vorposte/29342"),
            "https://t.me/vorposte/29342?embed=1"
        );
        assert_eq!(
            embed("https://t.me/vorposte/29342?single"),
            "https://t.me/vorposte/29342?embed=1"
        );
        assert_eq!(
            embed("https://t.me/vorposte/29342?single=1&embed=0&x=y"),
            "https://t.me/vorposte/29342?embed=1&x=y"
        );
    }
}
