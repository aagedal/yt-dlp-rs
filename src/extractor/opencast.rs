//! Upstream `OpencastIE` and `OpencastPlaylistIE` (`yt_dlp/extractor/opencast.py`):
//! a Paella watch page's or an Engage series' `search/episode.json`
//! mediapackage, with its tracks as formats (direct files, DASH, HLS, HDS,
//! Smooth and RTMP). SMIL tracks are unsupported. Upstream's series lookup
//! expects a `search-results` wrapper that the live API no longer sends, so
//! the playlist reproduces upstream's KeyError.
use super::{Extractor, dict_format, dict_info, direct, download_json};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Opencast;
pub struct OpencastPlaylist;

const NAME: &str = "Opencast";
const PLAYLIST_NAME: &str = "OpencastPlaylist";

const INSTANCES: &str = r"(?:opencast\.informatik\.kit\.edu|electures\.uni-muenster\.de|oc-presentation\.ltcc\.tuwien\.ac\.at|medien\.ph-noe\.ac\.at|oc-video\.ruhr-uni-bochum\.de|oc-video1\.ruhr-uni-bochum\.de|opencast\.informatik\.uni-goettingen\.de|heicast\.uni-heidelberg\.de|opencast\.hawk\.de:8080|opencast\.hs-osnabrueck\.de|video[0-9]+\.virtuos\.uni-osnabrueck\.de|opencast\.uni-koeln\.de|media\.opencast\.hochschule-rhein-waal\.de|matterhorn\.dce\.harvard\.edu|hs-harz\.opencast\.uni-halle\.de|videocampus\.urz\.uni-leipzig\.de|media\.uct\.ac\.za|vid\.igb\.illinois\.edu|cursosabertos\.c3sl\.ufpr\.br|mcmedia\.missioncollege\.org|clases\.odon\.edu\.uy|oc-p\.uni-jena\.de)";
const UUID: &str = r"[\da-fA-F]{8}-[\da-fA-F]{4}-[\da-fA-F]{4}-[\da-fA-F]{4}-[\da-fA-F]{12}";

static EPISODE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^https?://(?P<host>{INSTANCES})/paella[0-9]*/ui/watch\.html\?(?:[^#]+&)?id=(?P<id>{UUID})"
    ))
    .unwrap()
});
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^https?://(?P<host>{INSTANCES})(?:/engage/ui/index\.html\?(?:[^#]+&)?epFrom=|/ltitools/index\.html\?(?:[^#]+&)?series=)(?P<id>{UUID})"
    ))
    .unwrap()
});
static RTMP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:rtmp://[^/]+/(?P<app>[^/]+))/(?P<ext>.+):(?P<playpath>.+)").unwrap()
});

/// `OpencastIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    EPISODE_URL.is_match(url)
}

/// `OpencastPlaylistIE._match_valid_url`.
pub fn playlist_suitable(url: &str) -> bool {
    SERIES_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `int_or_none(value, scale)`.
fn int_or_none(value: Option<&Value>, scale: i64) -> Option<i64> {
    let number = match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64))?,
        Value::String(text) => text.trim().parse::<i64>().ok()?,
        _ => return None,
    };
    Some(number / scale)
}

/// `variadic(value)`: a single object is a one-item list.
fn variadic(value: Option<&Value>) -> Vec<Value> {
    match value {
        Some(Value::Array(items)) => items.clone(),
        Some(Value::Null) | None => Vec::new(),
        Some(value) => vec![value.clone()],
    }
}

/// `traverse_obj(track, ('video', key))` or `None` when there is no `video`.
fn track_media(track: &Value, media: &str, key: &str) -> Option<Value> {
    track.get(media).and_then(|node| node.get(key)).cloned()
}

/// `_parse_mediapackage(video)`.
fn parse_mediapackage(
    client: &Client,
    video: &Value,
    page: &Url,
    display_id: &str,
) -> Result<VideoInfo> {
    let Some(video_id) = video.get("id").and_then(Value::as_str) else {
        return Err(Error::Reported(format!(
            "[{NAME}] {display_id}: Video id was not found; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    };
    let mut formats: Vec<VideoFormat> = Vec::new();
    for track in variadic(video.get("media").and_then(|media| media.get("track"))).iter() {
        let Some(href) = track.get("url").and_then(Value::as_str).map(str::to_owned) else {
            continue;
        };
        let manifest = Url::parse(&href)?;
        let ext = direct::determine_ext(&href);
        let transport = track.get("transport").and_then(Value::as_str);
        if transport == Some("DASH") || ext.as_deref() == Some("mpd") {
            if let Some(info) = super::extract_mpd_optional_note(
                client,
                &manifest,
                NAME,
                video_id,
                Some("dash"),
                "Downloading MPD manifest",
            ) {
                formats.extend(info.formats);
            }
        } else if transport == Some("HLS") || ext.as_deref() == Some("m3u8") {
            if let Some(info) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                video_id,
                Some("hls"),
                false,
                false,
            ) {
                formats.extend(info.formats);
            }
        } else if transport == Some("HDS") || ext.as_deref() == Some("f4m") {
            formats.extend(super::extract_f4m_optional(
                client,
                NAME,
                &manifest,
                video_id,
                Some("hds"),
            )?);
        } else if transport == Some("SMOOTH") {
            let (smooth, _subtitles) =
                super::extract_ism_optional(client, NAME, &manifest, video_id, Some("smooth"))?;
            formats.extend(smooth);
        } else if ext.as_deref() == Some("smil") {
            return Err(Error::UnsupportedHtml5(
                "Opencast SMIL tracks are not supported natively".into(),
            ));
        } else {
            let has_video = track.get("video").is_some_and(super::nuxt::truthy);
            let has_audio = track.get("audio").is_some_and(super::nuxt::truthy);
            let mut fields: Vec<(&str, Value)> = vec![
                ("url", href.clone().into()),
                ("ext", ext.clone().map_or(Value::Null, Value::from)),
                ("format_note", transport.map_or(Value::Null, Value::from)),
                (
                    "resolution",
                    track_media(track, "video", "resolution").unwrap_or(Value::Null),
                ),
                (
                    "fps",
                    int_or_none(track_media(track, "video", "framerate").as_ref(), 1)
                        .map_or(Value::Null, Value::from),
                ),
                (
                    "vbr",
                    int_or_none(track_media(track, "video", "bitrate").as_ref(), 1000)
                        .map_or(Value::Null, Value::from),
                ),
                (
                    "vcodec",
                    if has_video {
                        track_media(track, "video", "encoder")
                            .and_then(|encoder| encoder.get("type").cloned())
                            .unwrap_or(Value::Null)
                    } else {
                        "none".into()
                    },
                ),
                (
                    "abr",
                    int_or_none(track_media(track, "audio", "bitrate").as_ref(), 1000)
                        .map_or(Value::Null, Value::from),
                ),
                (
                    "asr",
                    int_or_none(track_media(track, "audio", "samplingrate").as_ref(), 1)
                        .map_or(Value::Null, Value::from),
                ),
                (
                    "acodec",
                    if has_audio {
                        track_media(track, "audio", "encoder")
                            .and_then(|encoder| encoder.get("type").cloned())
                            .unwrap_or(Value::Null)
                    } else {
                        "none".into()
                    },
                ),
            ];
            if transport == Some("RTMP") {
                // `continue`s the track when the URL has no `app/…:playpath`.
                let Some(found) = RTMP.captures(&href) else {
                    continue;
                };
                let play_path = format!("{}:{}", &found["ext"], &found["playpath"]);
                for (key, value) in fields.iter_mut() {
                    if *key == "ext" {
                        *value = found["ext"].to_owned().into();
                    }
                }
                fields.extend([
                    ("app", found["app"].to_owned().into()),
                    ("play_path", play_path.into()),
                    ("rtmp_live", true.into()),
                    ("preference", json!(-2)),
                ]);
            }
            let mut format = dict_format(fields, NAME, video_id)?;
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
    }

    // `traverse_obj(video, ('attachments', 'attachment', ..., 'url'), get_all=False)`.
    let thumbnail = variadic(
        video
            .get("attachments")
            .and_then(|attachments| attachments.get("attachment")),
    )
    .iter()
    .find_map(|attachment| {
        attachment
            .get("url")
            .and_then(Value::as_str)
            .map(str::to_owned)
    });
    let creator = video
        .get("creators")
        .and_then(|creators| creators.get("creator"))
        .cloned()
        .unwrap_or(Value::Null);
    let timestamp = video
        .get("start")
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map_or(Value::Null, Value::from);
    Ok(dict_info(
        page,
        (NAME, NAME),
        vec![
            ("id", video_id.into()),
            ("formats", Value::Null),
            ("title", video.get("title").cloned().unwrap_or(Value::Null)),
            (
                "series",
                video.get("seriestitle").cloned().unwrap_or(Value::Null),
            ),
            (
                "season_id",
                video.get("series").cloned().unwrap_or(Value::Null),
            ),
            ("creator", creator),
            ("timestamp", timestamp),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
        ],
        formats,
    ))
}

/// `_call_api(host, video_id, **kwargs)`.
fn call_api(
    client: &Client,
    template: &str,
    host: &str,
    id: &str,
    display_id: &str,
) -> Result<Value> {
    let api = Url::parse(&template.replacen("%s", host, 1).replacen("%s", id, 1))?;
    download_json(client, &api, NAME, display_id)
}

impl Extractor for Opencast {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = EPISODE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let host = found["host"].to_owned();
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let response = call_api(
            client,
            "https://%s/search/episode.json?id=%s",
            &host,
            &video_id,
            &video_id,
        )?;
        // `traverse_obj(response, (('search-results', 'result'), ('result', ...), 'mediapackage', {dict}, any))`.
        let package = response
            .get("search-results")
            .and_then(|results| results.get("result"))
            .and_then(|result| result.get("mediapackage"))
            .filter(|package| package.is_object())
            .cloned()
            .or_else(|| {
                variadic(response.get("result"))
                    .into_iter()
                    .find_map(|item| {
                        item.get("mediapackage")
                            .filter(|package| package.is_object())
                            .cloned()
                    })
            })
            .unwrap_or_else(|| json!({}));
        Ok(Some(parse_mediapackage(client, &package, url, &video_id)?))
    }
}

impl Extractor for OpencastPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SERIES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let host = found["host"].to_owned();
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(PLAYLIST_NAME);
        crate::screen::screen(&format!(
            "[{PLAYLIST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let response = call_api(
            client,
            "https://%s/search/episode.json?sid=%s",
            &host,
            &video_id,
            &video_id,
        )?;
        let results = response
            .get("search-results")
            .ok_or_else(|| key_error(&video_id, "search-results"))?;
        let episodes = variadic(results.get("result"));
        let mut entries = Vec::new();
        for episode in &episodes {
            let Some(package) = episode
                .get("mediapackage")
                .filter(|package| super::nuxt::truthy(package))
            else {
                continue;
            };
            entries.push(parse_mediapackage(client, package, url, &video_id)?);
        }
        // `traverse_obj(entries, (0, 'series'))`: the first entry's series title.
        let title = episodes
            .iter()
            .find_map(|episode| {
                episode
                    .get("mediapackage")
                    .filter(|package| super::nuxt::truthy(package))
            })
            .and_then(|package| package.get("seriestitle").and_then(Value::as_str))
            .map(str::to_owned);
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: video_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: PLAYLIST_NAME.into(),
            extractor_key: PLAYLIST_NAME.into(),
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
        assert!(suitable(
            "https://oc-video1.ruhr-uni-bochum.de/paella/ui/watch.html?id=ed063cd5-72c8-46b5-a60a-569243edcea8"
        ));
        assert!(playlist_suitable(
            "https://oc-video1.ruhr-uni-bochum.de/engage/ui/index.html?epFrom=cf68a4a1-36b1-4a53-a6ba-61af5705a0d0"
        ));
        assert!(playlist_suitable(
            "https://oc-video1.ruhr-uni-bochum.de/ltitools/index.html?subtool=series&series=cf68a4a1-36b1-4a53-a6ba-61af5705a0d0&lng=de"
        ));
        assert!(!suitable(
            "https://example.com/paella/ui/watch.html?id=ed063cd5-72c8-46b5-a60a-569243edcea8"
        ));
    }

    #[test]
    fn bitrates_scale_like_upstream() {
        assert_eq!(int_or_none(Some(&json!(1165000)), 1000), Some(1165));
        assert_eq!(int_or_none(Some(&json!("48000")), 1), Some(48000));
    }
}
