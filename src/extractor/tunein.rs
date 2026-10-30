//! Upstream `TuneInStationIE`, `TuneInPodcastIE` and `TuneInPodcastEpisodeIE`
//! (`yt_dlp/extractor/tunein.py`): a station's or an episode's `opml`
//! stream list (direct MP3/AAC/Ogg URLs) with the `api.tunein.com` profile,
//! and a podcast's paged episode list as URL entries. HLS streams and the
//! `TuneInEmbedIE`/`TuneInShortenerIE` redirects are not supported natively.
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TuneInStation;
pub struct TuneInPodcast;
pub struct TuneInPodcastEpisode;

const STATION_NAME: &str = "tunein:station";
const PODCAST_NAME: &str = "tunein:podcast:program";
const EPISODE_NAME: &str = "tunein:podcast";
const PAGE_SIZE: usize = 20;

static STATION_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://tunein\.com/radio/[^/?#]+(?P<id>s\d+)").unwrap());
static PODCAST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://tunein\.com/podcasts(?:/[^/?#]+){1,2}(?P<id>p\d+)").unwrap()
});
static EPISODE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)^https?://tunein\.com/podcasts(?:/[^/?#]+){1,2}(?P<series_id>p\d+)/?\?(?:[^#]+&)?topicid=(?P<id>\d+)",
    )
    .unwrap()
});

/// `_match_valid_url` for `TuneInStationIE`.
pub fn station_suitable(url: &str) -> bool {
    STATION_URL.is_match(url)
}

/// `TuneInPodcastIE.suitable`: not an episode URL.
pub fn podcast_suitable(url: &str) -> bool {
    !EPISODE_URL.is_match(url) && PODCAST_URL.is_match(url)
}

/// `_match_valid_url` for `TuneInPodcastEpisodeIE`.
pub fn episode_suitable(url: &str) -> bool {
    EPISODE_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("TuneIn {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// `_call_api`: the `api.tunein.com` profile (or its contents), `{}` on failure.
fn call_api(
    client: &Client,
    item_id: &str,
    endpoint: Option<&str>,
    query: &[(&str, &str)],
) -> Value {
    let mut api = format!("https://api.tunein.com/profiles/{item_id}");
    if let Some(endpoint) = endpoint {
        api.push('/');
        api.push_str(endpoint);
    }
    let mut api = Url::parse(&api).expect("TuneIn API URL");
    if !query.is_empty() {
        api = super::update_url_query(&api, query);
    }
    download_json(client, &api, STATION_NAME, item_id).unwrap_or(Value::Null)
}

/// `_extract_formats_and_subtitles`: the `opml` stream list's direct formats.
fn extract_formats(client: &Client, content_id: &str) -> Result<Vec<crate::VideoFormat>> {
    let api = super::update_url_query(
        &Url::parse("https://opml.radiotime.com/Tune.ashx")?,
        &[
            ("formats", "mp3,aac,ogg,flash,hls"),
            ("id", content_id),
            ("render", "json"),
        ],
    );
    let streams = download_json(client, &api, STATION_NAME, content_id)?;
    let mut formats = Vec::new();
    for stream in streams
        .get("body")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(url) = stream
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| Url::parse(url).is_ok())
        else {
            continue;
        };
        if stream.get("media_type").and_then(Value::as_str) == Some("hls") {
            return Err(unsupported("HLS streams"));
        }
        let abr = super::streamable::int_or_none(stream.get("bitrate"));
        let ext = stream
            .get("media_type")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let mut fields = vec![("abr", abr)];
        if let Some(ext) = ext {
            fields.push(("ext", ext.into()));
        }
        fields.push(("url", url.into()));
        let mut format = dict_format(
            fields
                .into_iter()
                .filter(|(_, value)| !value.is_null())
                .collect(),
            STATION_NAME,
            content_id,
        )?;
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    Ok(formats)
}

/// `clean_html` of a profile's string, `None` when absent or empty.
fn clean_text(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .map(crate::html::clean_html)
        .filter(|text| !text.is_empty())
}

impl Extractor for TuneInStation {
    fn key(&self) -> &'static str {
        "TuneInStation"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = STATION_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let station_id = found["id"].to_owned();
        crate::screen::set_extractor(STATION_NAME);
        crate::screen::screen(&format!(
            "[{STATION_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let formats = extract_formats(client, &station_id)?;
        let profile = call_api(client, &station_id, None, &[]);
        let item = profile.get("Item");
        let alt_title = clean_text(item.and_then(|item| item.get("Subtitle")));
        let description = clean_text(item.and_then(|item| item.get("Description")));
        let follower_count = item
            .and_then(|item| item.pointer("/Actions/Follow/FollowerCount"))
            .map(|value| super::streamable::int_or_none(Some(value)))
            .filter(|value| !value.is_null());
        let is_live = item
            .and_then(|item| item.pointer("/Actions/Play/IsLive"))
            .and_then(Value::as_bool);
        let location = item
            .and_then(|item| item.pointer("/Properties/Location/DisplayName"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let thumbnail = item
            .and_then(|item| item.get("Image"))
            .and_then(Value::as_str)
            .filter(|image| Url::parse(image).is_ok())
            .map(str::to_owned);
        let title = clean_text(item.and_then(|item| item.get("Title")));

        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (STATION_NAME, "TuneInStation"),
            vec![
                ("id", station_id.clone().into()),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("title", title.map_or(Value::Null, Value::from)),
                ("alt_title", alt_title.map_or(Value::Null, Value::from)),
                (
                    "channel_follower_count",
                    follower_count.unwrap_or(Value::Null),
                ),
                ("description", description.map_or(Value::Null, Value::from)),
                ("is_live", is_live.map_or(Value::Null, Value::from)),
                ("location", location.map_or(Value::Null, Value::from)),
                ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            ],
            formats,
        ))))
    }
}

/// `url_result(update_url_query(url, {'topicId': ...}))` as a playlist entry.
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": null,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for TuneInPodcast {
    fn key(&self) -> &'static str {
        "TuneInPodcast"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        if !podcast_suitable(url.as_str()) {
            return Ok(None);
        }
        let Some(found) = PODCAST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let podcast_id = found["id"].to_owned();
        crate::screen::set_extractor(PODCAST_NAME);
        crate::screen::screen(&format!(
            "[{PODCAST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        // `OnDemandPagedList`: pages are read until one is not full.
        let mut entries = Vec::new();
        for page in 0.. {
            let offset = (page * PAGE_SIZE).to_string();
            let limit = PAGE_SIZE.to_string();
            let contents = call_api(
                client,
                &podcast_id,
                Some("contents"),
                &[("filter", "t:free"), ("limit", &limit), ("offset", &offset)],
            );
            let items = contents
                .get("Items")
                .and_then(Value::as_array)
                .ok_or_else(|| key_error(&podcast_id, "Items"))?;
            for item in items {
                let Some(guide_id) = item.get("GuideId").and_then(Value::as_str) else {
                    continue;
                };
                // `GuideId` filtered to a non-empty string; `item[1:]` is the topic ID.
                if guide_id.is_empty() {
                    continue;
                }
                let topic = guide_id.chars().skip(1).collect::<String>();
                let target = super::update_url_query(url, &[("topicId", &topic)]);
                entries.push(url_entry(target.as_str())?);
            }
            if items.len() < PAGE_SIZE {
                break;
            }
        }

        let title = call_api(client, &podcast_id, None, &[])
            .pointer("/Item/Title")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_default();
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: podcast_id,
            title,
            webpage_url: url.clone(),
            extractor: PODCAST_NAME.into(),
            extractor_key: "TuneInPodcast".into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

impl Extractor for TuneInPodcastEpisode {
    fn key(&self) -> &'static str {
        "TuneInPodcastEpisode"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = EPISODE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let series_id = found["series_id"].to_owned();
        let display_id = found["id"].to_owned();
        let episode_id = format!("t{display_id}");
        crate::screen::set_extractor(EPISODE_NAME);
        crate::screen::screen(&format!(
            "[{EPISODE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let formats = extract_formats(client, &episode_id)?;
        let series = call_api(client, &series_id, None, &[]);
        let series_title = clean_text(series.pointer("/Item/Title"));
        let episode = call_api(client, &episode_id, None, &[]);
        let item = episode.get("Item");

        let cast = item
            .and_then(|item| item.pointer("/Properties/ParentProgram/Hosts"))
            .and_then(Value::as_str)
            .map(crate::html::clean_html)
            .map(|hosts| {
                hosts
                    .split(';')
                    .map(|host| host.trim().to_owned())
                    .filter(|host| !host.is_empty())
                    .collect::<Vec<_>>()
            })
            .filter(|hosts| !hosts.is_empty());
        let duration = item
            .and_then(|item| item.pointer("/Actions/Play/Duration"))
            .map(|value| super::streamable::int_or_none(Some(value)))
            .filter(|value| !value.is_null());
        let timestamp = item
            .and_then(|item| item.pointer("/Actions/Play/PublishTime"))
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601);
        let thumbnail = item
            .and_then(|item| item.get("Image"))
            .and_then(Value::as_str)
            .filter(|image| Url::parse(image).is_ok())
            .map(str::to_owned);

        let mut fields = vec![
            ("id", episode_id.clone().into()),
            ("display_id", display_id.into()),
            ("formats", Value::Null),
            ("series", series_title.map_or(Value::Null, Value::from)),
            ("series_id", series_id.into()),
            ("subtitles", Value::Null),
            (
                "title",
                clean_text(item.and_then(|item| item.get("Title")))
                    .map_or(Value::Null, Value::from),
            ),
            (
                "alt_title",
                clean_text(item.and_then(|item| item.get("Subtitle")))
                    .map_or(Value::Null, Value::from),
            ),
        ];
        if let Some(hosts) = cast {
            fields.push(("cast", hosts.into()));
        }
        fields.extend([
            (
                "description",
                clean_text(item.and_then(|item| item.get("Description")))
                    .map_or(Value::Null, Value::from),
            ),
            ("duration", duration.unwrap_or(Value::Null)),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
        ]);
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (EPISODE_NAME, "TuneInPodcastEpisode"),
            fields,
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(station_suitable(
            "https://tunein.com/radio/Jazz24-885-s34682/"
        ));
        assert!(!station_suitable("https://tunein.com/radio/Jazz24-885/"));
        assert!(podcast_suitable(
            "https://tunein.com/podcasts/Technology-Podcasts/Artificial-Intelligence-p1153019/"
        ));
        assert!(!podcast_suitable(
            "https://tunein.com/podcasts/Technology-Podcasts/Artificial-Intelligence-p1153019/?topicId=236404354"
        ));
        assert!(episode_suitable(
            "https://tunein.com/podcasts/Technology-Podcasts/Artificial-Intelligence-p1153019/?topicId=236404354"
        ));
        assert!(!episode_suitable("https://tunein.com/embed/player/s6404/"));
    }
}
