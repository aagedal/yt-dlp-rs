//! Upstream `ACastIE` and `ACastChannelIE` (`yt_dlp/extractor/acast.py`):
//! the `feeder.acast.com` episode JSON (with its show) as an untyped audio
//! result, a show's episode list as a playlist of them, and `ACastIE`'s
//! iframe embeds.
use super::{Extractor, download_json};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct ACast;
pub struct ACastChannel;

pub(crate) const NAME: &str = "acast";
pub(crate) const KEY: &str = "ACast";
const CHANNEL_NAME: &str = "acast:channel";
const CHANNEL_KEY: &str = "ACastChannel";
const API: &str = "https://feeder.acast.com/api/v1/shows/";

const VALID_URL_BODY: &str = r#"https?://(?:(?:(?:embed|www|shows)\.)?acast\.com/|play\.acast\.com/s/)(?P<channel>[^/?#]+)/(?:episodes/)?(?P<id>[^/#?"]+)"#;

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| crate::extractor::url_regex(&format!("^{VALID_URL_BODY}")).unwrap());
static CHANNEL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:(?:www|shows)\.)?acast\.com/|play\.acast\.com/s/)(?P<id>[^/#?]+)")
        .unwrap()
});
/// `_EMBED_REGEX`: an iframe `src` matching `_VALID_URL`.
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r#"<iframe[^>]+\bsrc=['"](?P<url>{VALID_URL_BODY})"#
    ))
    .unwrap()
});

/// `ACastIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `ACastChannelIE.suitable`: not an episode URL.
pub fn channel_suitable(url: &str) -> bool {
    !suitable(url) && CHANNEL_URL.is_match(url)
}

/// `_match_id`'s ID of a suitable URL.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls` under `orderedSet`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Ok(embed) = url.join(&crate::html::unescape_html(&captures["url"])) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Acast {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// Python truthiness of a JSON value.
fn truthy(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => false,
        Value::String(text) => !text.is_empty(),
        Value::Number(number) => number.as_f64() != Some(0.0),
        Value::Array(items) => !items.is_empty(),
        Value::Object(items) => !items.is_empty(),
        Value::Bool(true) => true,
    }
}

/// `int_or_none`.
fn int_or_none(value: Option<&Value>) -> Value {
    let value = match value {
        Some(Value::Number(number)) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Some(Value::String(text)) => text.trim().parse().ok(),
        Some(Value::Bool(flag)) => Some(i64::from(*flag)),
        _ => None,
    };
    value.map_or(Value::Null, Value::from)
}

/// `_extract_show_info`.
fn show_info(show: &Map<String, Value>) -> [(&'static str, Value); 2] {
    let get = |key: &str| show.get(key).cloned().unwrap_or(Value::Null);
    [("creator", get("author")), ("series", get("title"))]
}

/// `ACastBaseIE._extract_episode`; `id` is the error's video ID.
fn extract_episode(
    episode: &Value,
    show_info: &[(&'static str, Value)],
    id: &str,
    webpage_url: &Url,
    extractor: (&'static str, &'static str),
) -> Result<VideoInfo> {
    let episode = episode
        .as_object()
        .ok_or_else(|| unsupported("episodes that are not objects"))?;
    let title = match episode.get("title") {
        Some(Value::String(title)) => title.clone(),
        Some(_) => return Err(unsupported("episode titles that are not strings")),
        None => return Err(key_error(id, "title")),
    };
    let episode_id = match episode.get("id") {
        Some(Value::String(episode_id)) => episode_id.clone(),
        Some(_) => return Err(unsupported("episode IDs that are not strings")),
        None => return Err(key_error(id, "id")),
    };
    let display_id = episode.get("episodeUrl").cloned().unwrap_or(Value::Null);
    let media = match episode.get("url") {
        Some(Value::String(media)) => super::playerfm::clean_podcast_url(media),
        Some(_) => return Err(unsupported("episode URLs that are not strings")),
        None => return Err(key_error(id, "url")),
    };
    // `clean_html(description or summary)`.
    let description = match episode.get("description") {
        Some(value) if truthy(value) => Some(value),
        _ => episode.get("summary"),
    };
    let description = match description {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(text)) => crate::html::clean_html(text).into(),
        Some(_) => return Err(unsupported("descriptions that are not strings")),
    };
    let timestamp = match episode.get("publishDate") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::String(date)) => {
            crate::dates::parse_iso8601(date).map_or(Value::Null, Value::from)
        }
        Some(_) => return Err(unsupported("publish dates that are not strings")),
    };
    let thumbnail = episode.get("image").cloned().unwrap_or(Value::Null);
    if !matches!(thumbnail, Value::Null | Value::String(_)) {
        return Err(unsupported("images that are not strings"));
    }

    let media = Url::parse(&media).map_err(|_| unsupported("media URLs that are not absolute"))?;
    let ext =
        super::direct::determine_ext(media.as_str()).unwrap_or_else(|| "unknown_video".into());
    let mut info = super::direct::with_format(
        super::direct::build_at(media.clone(), &media, ext.clone()),
        "",
    );
    for format in &mut info.formats {
        format.ext = ext.clone();
        format.format_id = String::new();
    }
    info.format_id = None;
    info.id = episode_id;
    info.title = title.clone();
    info.webpage_url = webpage_url.clone();
    info.direct = None;
    info.extractor = extractor.0.into();
    info.extractor_key = extractor.1.into();
    let mut keys = vec!["id", "display_id", "url", "title"];
    let fields = [
        ("display_id", display_id),
        ("description", description),
        ("thumbnail", thumbnail),
        ("timestamp", timestamp),
        ("duration", int_or_none(episode.get("duration"))),
        ("filesize", int_or_none(episode.get("contentLength"))),
        ("season_number", int_or_none(episode.get("season"))),
        ("episode", title.into()),
        ("episode_number", int_or_none(episode.get("episode"))),
    ];
    for (key, value) in fields.into_iter().chain(show_info.iter().cloned()) {
        // Processing drops a `None` `duration`.
        if key == "duration" && value.is_null() {
            continue;
        }
        if key != "display_id" {
            keys.push(key);
        }
        if key == "filesize" {
            // The untyped dict is its own format.
            for format in &mut info.formats {
                format.filesize = value.as_u64();
            }
        }
        crate::metadata_parser::set_field(&mut info, key, value);
    }
    Ok(crate::info_order::mark_info(
        info,
        &format!("untyped-dict;{}", keys.join(",")),
    ))
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for ACast {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let (channel, display_id) = (found["channel"].to_owned(), found["id"].to_owned());
        announce(NAME, url);
        let mut api = Url::parse(&format!("{API}{channel}/episodes/{display_id}"))?;
        api.query_pairs_mut().append_pair("showInfo", "true");
        let episode = download_json(client, &api, NAME, &display_id)?;
        // `episode.get('show') or {}`.
        let empty = Map::new();
        let show = match episode.get("show") {
            Some(Value::Object(show)) => show,
            Some(value) if truthy(value) => {
                return Err(unsupported("shows that are not objects"));
            }
            _ => &empty,
        };
        extract_episode(&episode, &show_info(show), &display_id, url, (NAME, KEY)).map(Some)
    }
}

impl Extractor for ACastChannel {
    fn key(&self) -> &'static str {
        CHANNEL_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !channel_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = CHANNEL_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let show_slug = found["id"].to_owned();
        announce(CHANNEL_NAME, url);
        let api = Url::parse(&format!("{API}{show_slug}"))?;
        let show = download_json(client, &api, CHANNEL_NAME, &show_slug)?;
        let show = show
            .as_object()
            .ok_or_else(|| unsupported("shows that are not objects"))?;
        let info = show_info(show);
        let mut entries = Vec::new();
        match show.get("episodes") {
            Some(Value::Array(episodes)) => {
                for episode in episodes {
                    entries.push(extract_episode(
                        episode,
                        &info,
                        &show_slug,
                        url,
                        (CHANNEL_NAME, CHANNEL_KEY),
                    )?);
                }
            }
            Some(value) if truthy(value) => {
                return Err(unsupported("episode lists that are not lists"));
            }
            _ => {}
        }
        // `playlist_result(entries, id, title, description)`.
        let id = match show.get("id") {
            Some(Value::String(id)) => Some(id.clone()).filter(|id| !id.is_empty()),
            Some(value) if truthy(value) => {
                return Err(unsupported("show IDs that are not strings"));
            }
            _ => None,
        };
        let title = match show.get("title") {
            Some(Value::String(title)) => Some(title.clone()).filter(|title| !title.is_empty()),
            Some(value) if truthy(value) => {
                return Err(unsupported("show titles that are not strings"));
            }
            _ => None,
        };
        let mut keys = Vec::new();
        let mut extra = Map::new();
        if id.is_some() {
            keys.push("id");
        } else {
            extra.insert("id".into(), Value::Null);
        }
        if title.is_some() {
            keys.push("title");
        }
        if let Some(description) = show.get("description").filter(|value| !value.is_null()) {
            extra.insert("description".into(), description.clone());
            keys.push("description");
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: id.unwrap_or_default(),
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: CHANNEL_NAME.into(),
            extractor_key: CHANNEL_KEY.into(),
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
        for url in [
            "https://shows.acast.com/sparpodcast/episodes/2.raggarmordet-rosterurdetforflutna",
            "http://embed.acast.com/adambuxton/ep.12-adam-joeschristmaspodcast2015",
            "https://play.acast.com/s/rattegangspodden/s04e09styckmordetihelenelund-del2-2",
            "https://www.acast.com/sparpodcast/2.raggarmordet-rosterurdetforflutna",
            "https://play.acast.com/s/sparpodcast/2a92b283-1a75-4ad8-8396-499c641de0d9",
        ] {
            assert!(suitable(url), "{url}");
            assert!(!channel_suitable(url), "{url}");
        }
        for url in [
            "https://www.acast.com/todayinfocus",
            "http://play.acast.com/s/ft-banking-weekly",
            "https://shows.acast.com/sparpodcast",
        ] {
            assert!(!suitable(url), "{url}");
            assert!(channel_suitable(url), "{url}");
        }
        assert!(!channel_suitable("https://embed.acast.com/adambuxton"));
    }

    #[test]
    fn embeds_follow_upstream_embed_regex() {
        let page = Url::parse("https://example.test/").unwrap();
        let html = r#"<iframe width="1" src="https://embed.acast.com/show/ep-1?theme=light"></iframe><iframe src="https://embed.acast.com/show/ep-1"><iframe src="https://embed.acast.com/show">"#;
        let found: Vec<String> = embed_urls(&page, html).iter().map(Url::to_string).collect();
        assert_eq!(found, ["https://embed.acast.com/show/ep-1"]);
    }
}
