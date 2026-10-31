//! Upstream `MicrosoftLearnEpisodeIE` and `MicrosoftLearnPlaylistIE`
//! (`yt_dlp/extractor/microsoftembed.py`): a show's or event's episode list
//! (paged `contentbrowser` search, as `url` entries), and
//! learn.microsoft.com show episodes through the page's `entryId` and the
//! public video entry API, with the non-fatal HLS and DASH manifests, the
//! `video-http-*` progressive qualities, the audio track, thumbnails and
//! captions. Smooth Streaming (`adaptiveVideoUrl`) manifests are not
//! supported natively.
use super::{
    Extractor, dict_format, dict_info, download_json, download_json_note, download_webpage,
    extract_m3u8_master_optional, extract_mpd_optional_note, html5::parse_resolution,
    search_warning, update_url_query,
};
use crate::{
    Error, ExtractResult, PlaylistInfo, Result, SubtitleFormat, Subtitles, VideoFormat, VideoInfo,
};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct MicrosoftLearnEpisode;
pub struct MicrosoftLearnPlaylist;

const NAME: &str = "MicrosoftLearnEpisode";
const PLAYLIST_NAME: &str = "MicrosoftLearnPlaylist";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://learn\.microsoft\.com/(?:[\w-]+/)?shows/[\w-]+/(?P<id>[^?#/]+)",
    )
    .unwrap()
});

static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://learn\.microsoft\.com/(?:[\w-]+/)?(?P<type>shows|events)/(?P<id>[\w-]+)/?(?:[?#]|$)",
    )
    .unwrap()
});

/// `_match_valid_url` for the episode pattern.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_valid_url` for the show and event playlist pattern.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!(
        "MicrosoftLearnEpisode {what} are not supported natively"
    ))
}

/// `url_or_none(publicVideo[key])`.
fn public_url(public: Option<&Value>, key: &str) -> Option<String> {
    public
        .and_then(|public| public.get(key))
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// `_sub_to_dict` of the `captions` list: grouped by language (`und` when
/// the language is missing), each track `{'url': ...}` with its processing
/// `ext`.
fn caption_tracks(public: Option<&Value>) -> Result<Subtitles> {
    let mut groups: Vec<(String, Vec<SubtitleFormat>)> = Vec::new();
    let captions = public
        .and_then(|public| public.get("captions"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for caption in captions {
        let Some(url) = caption
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let tag = caption
            .get("language")
            .and_then(Value::as_str)
            .unwrap_or("und")
            .to_owned();
        let track = SubtitleFormat {
            name: None,
            url: Url::parse(&url)?,
            ext: super::direct::determine_ext(&url).unwrap_or_else(|| "unknown_video".into()),
            protocol: None,
            fragments: Vec::new(),
            extra: Default::default(),
        };
        match groups.iter_mut().find(|(key, _)| *key == tag) {
            Some((_, tracks)) => tracks.push(track),
            None => groups.push((tag, vec![track])),
        }
    }
    let mut subtitles = Subtitles::default();
    for (tag, tracks) in groups {
        subtitles.insert(Some(tag), tracks);
    }
    Ok(subtitles)
}

impl Extractor for MicrosoftLearnEpisode {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `_html_search_meta('entryId', webpage, 'entryId', fatal=True)`.
        let entry_id = crate::html::html_search_meta(&webpage, "entryId").ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract entryId; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let api = Url::parse(&format!(
            "https://learn.microsoft.com/api/video/public/v1/entries/{entry_id}"
        ))?;
        let video_info = download_json(client, &api, NAME, &video_id)?;
        let public = video_info
            .get("publicVideo")
            .filter(|value| value.is_object());

        if public_url(public, "adaptiveVideoUrl").is_some() {
            return Err(unsupported("Smooth Streaming manifests"));
        }
        let mut formats: Vec<VideoFormat> = Vec::new();
        if let Some(hls) = public_url(public, "adaptiveVideoHLSUrl") {
            let manifest = Url::parse(&hls)?;
            if let Some(stream) = extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                &video_id,
                Some("hls"),
                false,
                false,
            ) {
                formats.extend(stream.formats);
            }
        }
        if let Some(dash) = public_url(public, "adaptiveVideoDashUrl") {
            let manifest = Url::parse(&dash)?;
            if let Some(stream) = extract_mpd_optional_note(
                client,
                &manifest,
                NAME,
                &video_id,
                Some("dash"),
                "Downloading MPD manifest",
            ) {
                formats.extend(stream.formats);
            }
        }
        for key in ["low", "medium", "high"] {
            let Some(video_url) = public_url(public, &format!("{key}QualityVideoUrl")) else {
                continue;
            };
            let (width, height) = parse_resolution(&video_url);
            let mut fields = vec![
                ("url", Value::from(video_url.as_str())),
                ("format_id", Value::from(format!("video-http-{key}"))),
                ("acodec", Value::from("none")),
            ];
            if let Some(width) = width {
                fields.push(("width", Value::from(width)));
            }
            if let Some(height) = height {
                fields.push(("height", Value::from(height)));
            }
            formats.push(dict_format(fields, NAME, &video_id)?);
        }
        if let Some(audio) = public_url(public, "audioUrl") {
            formats.push(dict_format(
                vec![
                    ("url", Value::from(audio.as_str())),
                    ("format_id", Value::from("audio-http")),
                    ("vcodec", Value::from("none")),
                ],
                NAME,
                &video_id,
            )?);
        }

        let subtitles = caption_tracks(public)?;
        let title = crate::html::og_search_property(&webpage, "title");
        if title.is_none() {
            search_warning(NAME, "OpenGraph title");
        }
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            search_warning(NAME, "OpenGraph description");
        }
        let timestamp = video_info
            .get("createTime")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601);
        let thumbnails: Vec<Value> = public
            .and_then(|public| public.get("thumbnailOtherSizes"))
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|sizes| sizes.values())
            .filter_map(Value::as_str)
            .filter_map(super::json_ld::url_or_none)
            .map(|url| json!({ "url": url }))
            .collect();

        let mut fields = vec![
            ("id", Value::from(entry_id.as_str())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            (
                "title",
                title.map_or(Value::Null, |title| Value::from(title.as_str())),
            ),
            (
                "description",
                description.map_or(Value::Null, |description| Value::from(description.as_str())),
            ),
        ];
        if let Some(timestamp) = timestamp {
            fields.push(("timestamp", Value::from(timestamp)));
        }
        if !thumbnails.is_empty() {
            fields.push(("thumbnails", Value::Array(thumbnails)));
        }
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

/// `url_result(f'https://learn.microsoft.com/en-us{url_path}')`: a plain
/// URL entry (no extractor named), as upstream's `url` results.
fn url_entry(target: &str) -> Result<VideoInfo> {
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
            "ie_key": null,
            "entry": {"_type": "url", "url": target},
        }),
    );
    Ok(info)
}

impl Extractor for MicrosoftLearnPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        let playlist_type = found["type"].to_owned();
        crate::screen::set_extractor(PLAYLIST_NAME);
        crate::screen::screen(&format!(
            "[{PLAYLIST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, PLAYLIST_NAME, &playlist_id)?;
        let title = crate::html::og_search_property(&webpage, "title");
        let description = crate::html::og_search_property(&webpage, "description");

        let sub_type = if playlist_type == "shows" {
            "episodes"
        } else {
            "sessions"
        };
        let url_base = format!(
            "https://learn.microsoft.com/api/contentbrowser/search/{playlist_type}/{playlist_id}/{sub_type}"
        );
        let entries = playlist_entries(client, &url_base, &playlist_id)?;

        // playlist_result(..., **metainfo): metainfo keys come before the id.
        let mut keys = Vec::new();
        let mut extra = Map::new();
        if title.is_some() {
            keys.push("title");
        }
        if let Some(description) = &description {
            keys.push("description");
            extra.insert("description".into(), description.clone().into());
        }
        keys.push("id");
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
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

/// `MicrosoftLearnPlaylistIE._entries`: the search API's pages of `$skip`
/// results until the reported `count` (or an empty page).
fn playlist_entries(client: &Client, url_base: &str, playlist_id: &str) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    let mut skip: usize = 0;
    loop {
        let api = update_url_query(
            &Url::parse(url_base)?,
            &[("locale", "en-us"), ("$skip", &skip.to_string())],
        );
        let info = download_json_note(
            client,
            &api,
            PLAYLIST_NAME,
            playlist_id,
            &format!("Downloading entries {skip}"),
        )?;
        let paths: Vec<&str> = info
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|result| result.get("url")?.as_str())
            .collect();
        for path in &paths {
            entries.push(url_entry(&format!(
                "https://learn.microsoft.com/en-us{path}"
            ))?);
        }
        skip += paths.len();
        let count = info.get("count").and_then(Value::as_i64).unwrap_or(0);
        if skip as i64 >= count || paths.is_empty() {
            break;
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://learn.microsoft.com/en-us/shows/bash-for-beginners/what-is-the-difference-between-a-terminal-and-a-shell-2-of-20-bash-for-beginners/"
        ));
        assert!(suitable(
            "https://learn.microsoft.com/en-gb/shows/on-demand-instructor-led-training-series/az-900-module-1"
        ));
        // A show page without an episode slug is the playlist extractor's.
        assert!(!suitable(
            "https://learn.microsoft.com/en-us/shows/bash-for-beginners"
        ));
        assert!(!suitable(
            "https://learn.microsoft.com/en-us/events/build-2022"
        ));
        assert!(playlist_suitable(
            "https://learn.microsoft.com/en-us/shows/bash-for-beginners"
        ));
        assert!(playlist_suitable(
            "https://learn.microsoft.com/en-us/events/build-2022"
        ));
        assert!(!playlist_suitable(
            "https://learn.microsoft.com/en-us/shows/bash-for-beginners/what-is-the-difference/"
        ));
    }
}
