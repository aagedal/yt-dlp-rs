//! Upstream `OmnyfmIE`, `OmnyfmPlaylistIE` and `OmnyfmShowIE`
//! (`yt_dlp/extractor/omnyfm.py`): an episode's Next.js clip data as an
//! audio URL with chapters, a playlist's or show's clips as `url` entries
//! (paged through the omny API). The `asahi.com` embed pages upstream also
//! handles through `_WEBPAGE_TESTS` are reached only when the page links here.
use super::{Extractor, direct, download_webpage, nrk, search_nextjs_data};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Omnyfm;
pub struct OmnyfmPlaylist;
pub struct OmnyfmShow;

const NAME: &str = "Omnyfm";
const PLAYLIST_NAME: &str = "OmnyfmPlaylist";
const SHOW_NAME: &str = "OmnyfmShow";
const API_BASE: &str = "https://api.omny.fm";
const BASE_URL: &str = "https://omny.fm/shows";
const PAGE_SIZE: usize = 100;

// Upstream's episode pattern has a negative lookahead rejecting an ID of
// `playlists`; the `regex` crate has none, so that is checked in code.
static EPISODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"^https?://omny\.fm/shows/(?P<uploader_id>[\w-]+)/(?P<id>[\w-]+)(?:/embed)?(?:[?#"']|$)"#,
    )
    .unwrap()
});
static PLAYLIST: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"^https?://omny\.fm/shows/(?P<uploader_id>[\w-]+)/playlists(?:/(?P<id>[\w-]+))?(?:/embed)?/?(?:[?#"']|$)"#,
    )
    .unwrap()
});
static SHOW: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://omny\.fm/shows/(?P<id>[\w-]+)/?(?:[?#]|$)").unwrap());

/// An episode URL, not the `playlists` index of a show.
fn episode_match(url: &str) -> Option<(String, String)> {
    let found = EPISODE.captures(url)?;
    let id = found["id"].to_owned();
    (id != "playlists").then(|| (found["uploader_id"].to_owned(), id))
}

/// `clean_html(value)` as a non-empty string.
fn html_text(value: Option<&Value>) -> Option<String> {
    let text = crate::html::clean_html(value?.as_str()?);
    (!text.is_empty()).then_some(text)
}

fn str_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// `update_url(query=None)`.
fn without_query(text: &str) -> Option<String> {
    let mut url = Url::parse(text).ok()?;
    url.set_query(None);
    Some(url.to_string())
}

/// `int_or_none` of a JSON value.
fn int_value(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `parse_duration(value)` as a JSON number, if it parses.
fn duration_value(text: Option<&str>) -> Option<Value> {
    let number = crate::parse_duration(text?)?;
    serde_json::to_value(number).ok()
}

/// `_yield_clips`: a `url` entry per clip slug (`Omnyfm` episodes).
fn clip_entries(clips: &Value, uploader_id: &str, entries: &mut Vec<VideoInfo>) {
    for clip in clips
        .get("Clips")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(slug) = str_at(clip, "Slug") {
            entries.push(entry(&format!("{BASE_URL}/{uploader_id}/{slug}"), NAME));
        }
    }
}

fn entry(target: &str, ie_key: &'static str) -> VideoInfo {
    // `url_result(target, ie)`: the outer entry keeps its own key order.
    nrk::url_entry(target, ie_key, None, None, None).expect("omny entry URLs parse")
}

/// `_entries(uploader_id, playlist_id)`: the playlist's clips, paged by ID.
fn playlist_entries(
    client: &Client,
    uploader_id: &str,
    playlist_id: &str,
) -> Result<Vec<VideoInfo>> {
    let mut entries = Vec::new();
    let mut clip_id: Option<String> = None;
    for page in 1.. {
        let mut url = Url::parse(&format!(
            "{API_BASE}/programs/{uploader_id}/playlists/{playlist_id}/clips"
        ))?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(clip_id) = &clip_id {
                query.append_pair("clipId", clip_id);
            }
            query.append_pair("direction", "AfterExclusive");
            query.append_pair("pageSize", &PAGE_SIZE.to_string());
        }
        let clips = super::download_json_note(
            client,
            &url,
            NAME,
            playlist_id,
            &format!("Downloading page {page}"),
        )?;
        clip_entries(&clips, uploader_id, &mut entries);
        if clips.get("NextClipsAvailable").and_then(Value::as_bool) != Some(true) {
            break;
        }
        let last = clips
            .get("Clips")
            .and_then(Value::as_array)
            .and_then(|clips| clips.last())
            .and_then(|clip| clip.get("Id"))
            .and_then(Value::as_str);
        match last {
            Some(id) => clip_id = Some(id.to_owned()),
            None => break,
        }
    }
    Ok(entries)
}

impl Extractor for Omnyfm {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some((uploader_id, audio_id)) = episode_match(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &audio_id)?;
        let next = search_nextjs_data(&webpage, NAME, &audio_id)?;
        let clip = next
            .get("props")
            .and_then(|props| props.get("pageProps"))
            .and_then(|page| page.get("clip"))
            .filter(|clip| clip.is_object())
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let program = clip.get("Program").cloned().unwrap_or(Value::Null);

        let section_start = url
            .query_pairs()
            .filter(|(key, _)| key == "t")
            .last()
            .and_then(|(_, value)| duration_value(Some(&value)));
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", audio_id.clone().into()),
            ("section_start", section_start.unwrap_or(Value::Null)),
            ("uploader_id", uploader_id.into()),
            ("vcodec", "none".into()),
        ];
        if let Some(title) = html_text(clip.get("Title")) {
            fields.push(("title", title.into()));
        }
        if let Some(chapters) = clip.get("Chapters").and_then(Value::as_array) {
            let chapters: Vec<Value> = chapters
                .iter()
                .filter_map(|chapter| {
                    let position = duration_value(str_at(chapter, "Position"))?;
                    let mut map = Map::new();
                    if let Some(name) = html_text(chapter.get("Name")) {
                        map.insert("title".into(), name.into());
                    }
                    map.insert("start_time".into(), position);
                    Some(Value::Object(map))
                })
                .collect();
            if !chapters.is_empty() {
                fields.push(("chapters", chapters.into()));
            }
        }
        if let Some(description) = html_text(clip.get("Description")) {
            fields.push(("description", description.into()));
        }
        if let Some(duration) = clip.get("DurationSeconds").and_then(Value::as_f64) {
            fields.push(("duration", duration.into()));
        }
        if let Some(episode) = int_value(clip.get("Episode")) {
            fields.push(("episode_number", episode.into()));
        }
        if let Some(size) = int_value(clip.get("PublishedAudioSizeInBytes")) {
            fields.push(("filesize", size.into()));
        }
        if let Some(stamp) = str_at(&clip, "ModifiedAtUtc").and_then(crate::dates::parse_iso8601) {
            fields.push(("modified_timestamp", stamp.into()));
        }
        if let Some(season) = int_value(clip.get("Season")) {
            fields.push(("season_number", season.into()));
        }
        let tags: Vec<Value> = clip
            .get("Tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| html_text(Some(tag)).map(Value::from))
            .collect();
        if !tags.is_empty() {
            fields.push(("tags", tags.into()));
        }
        if let Some(image) = str_at(&clip, "ImageUrl").and_then(without_query) {
            fields.push(("thumbnail", image.into()));
        }
        if let Some(stamp) = str_at(&clip, "PublishedUtc").and_then(crate::dates::parse_iso8601) {
            fields.push(("timestamp", stamp.into()));
        }
        let audio = str_at(&clip, "AudioUrl")
            .filter(|audio| Url::parse(audio).is_ok())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Omny clips without an audio URL are not supported natively".into(),
                )
            })?;
        fields.push(("url", audio.into()));
        if let Some(page) = str_at(&clip, "PublishedUrl").filter(|page| Url::parse(page).is_ok()) {
            fields.push(("webpage_url", page.into()));
        }
        let categories: Vec<Value> = program
            .get("Categories")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|category| html_text(Some(category)).map(Value::from))
            .collect();
        if !categories.is_empty() {
            fields.push(("categories", categories.into()));
        }
        if let Some(name) = html_text(program.get("Name")) {
            fields.push(("uploader", name.into()));
        }
        // Upstream's bare `url` result: the audio URL is the one (audio-only)
        // format, and the dict's keys keep `fields`' order.
        Ok(Some(url_info(url, &Url::parse(audio)?, fields)))
    }
}

/// Upstream's bare `url` result: the audio URL is the one audio-only format,
/// and the dict's keys keep `fields`' order with no `formats` key.
fn url_info(page: &Url, media: &Url, fields: Vec<(&str, Value)>) -> VideoInfo {
    // `determine_ext` lowercases the extension.
    let ext = direct::determine_ext(media.as_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let keys: Vec<String> = fields.iter().map(|(key, _)| (*key).to_owned()).collect();
    let mut formats =
        direct::with_format(direct::build_at(media.clone(), media, ext.clone()), "").formats;
    // The dict's exact `filesize` is the format's size too.
    let size = fields
        .iter()
        .find(|(key, _)| *key == "filesize")
        .and_then(|(_, value)| value.as_u64());
    for format in &mut formats {
        format.ext = ext.clone();
        format.format_id = String::new();
        format.vcodec = Some("none".into());
        format.filesize = size;
    }
    let mut info = super::dict_info(page, (NAME, NAME), fields, formats);
    info.format_id = None;
    crate::info_order::mark_info(
        info,
        &format!(
            "untyped-dict;{};chapter=title,start_time,end_time",
            keys.join(",")
        ),
    )
}

impl Extractor for OmnyfmPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST.captures(url.as_str()) else {
            return Ok(None);
        };
        let uploader_id = found["uploader_id"].to_owned();
        let playlist_id = found.name("id").map(|id| id.as_str().to_owned());
        crate::screen::set_extractor(PLAYLIST_NAME);
        crate::screen::screen(&format!(
            "[{PLAYLIST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = download_webpage(
            client,
            url,
            PLAYLIST_NAME,
            playlist_id.as_deref().unwrap_or(&uploader_id),
        )?;
        let next = search_nextjs_data(
            &page,
            PLAYLIST_NAME,
            playlist_id.as_deref().unwrap_or(&uploader_id),
        )?;
        let page_props = next
            .get("props")
            .and_then(|props| props.get("pageProps"))
            .cloned()
            .unwrap_or(Value::Null);

        let Some(playlist_id) = playlist_id else {
            // The show's playlists, each an `OmnyfmPlaylist` URL.
            let entries: Vec<VideoInfo> = page_props
                .get("playlistsWithClips")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|item| str_at(item.get("playlist")?, "Slug"))
                .map(|slug| {
                    entry(
                        &format!("{BASE_URL}/{uploader_id}/playlists/{slug}"),
                        PLAYLIST_NAME,
                    )
                })
                .collect();
            return Ok(Some(ExtractResult::Playlist(playlist_info(
                url,
                PLAYLIST_NAME,
                uploader_id,
                String::new(),
                &[],
                entries,
            ))));
        };

        let entries = playlist_entries(client, &uploader_id, &playlist_id)?;
        let playlist = page_props.get("playlist").cloned().unwrap_or(Value::Null);
        let mut extra: Vec<(&str, Value)> = Vec::new();
        if let Some(title) = html_text(playlist.get("Title")) {
            extra.push(("title", title.into()));
        }
        if let Some(description) = html_text(playlist.get("Description")) {
            extra.push(("description", description.into()));
        }
        if let Some(thumbnail) = str_at(&playlist, "ArtworkUrl").and_then(without_query) {
            extra.push(("thumbnail", thumbnail.into()));
        }
        if let Some(embed) = str_at(&playlist, "EmbedUrl").filter(|embed| Url::parse(embed).is_ok())
        {
            extra.push(("webpage_url", embed.trim_end_matches("/embed").into()));
        }
        let title = extra
            .iter()
            .find(|(key, _)| *key == "title")
            .and_then(|(_, value)| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        Ok(Some(ExtractResult::Playlist(playlist_info(
            url,
            PLAYLIST_NAME,
            playlist_id,
            title,
            &extra,
            entries,
        ))))
    }
}

impl Extractor for OmnyfmShow {
    fn key(&self) -> &'static str {
        SHOW_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SHOW.captures(url.as_str()) else {
            return Ok(None);
        };
        let uploader_id = found["id"].to_owned();
        crate::screen::set_extractor(SHOW_NAME);
        crate::screen::screen(&format!(
            "[{SHOW_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = download_webpage(client, url, SHOW_NAME, &uploader_id)?;
        let next = search_nextjs_data(&page, SHOW_NAME, &uploader_id)?;
        let program = next
            .get("props")
            .and_then(|props| props.get("pageProps"))
            .and_then(|page| page.get("program"))
            .cloned()
            .unwrap_or(Value::Null);
        let organization = str_at(&program, "OrganizationId").ok_or_else(|| {
            Error::Reported(format!(
                "[{SHOW_NAME}] {uploader_id}: Unable to extract organization ID"
            ))
        })?;
        let program_id = str_at(&program, "Id").ok_or_else(|| {
            Error::Reported(format!(
                "[{SHOW_NAME}] {uploader_id}: Unable to extract program ID"
            ))
        })?;

        // `OnDemandPagedList`: pages by cursor until one is short.
        let mut entries = Vec::new();
        for page in 0.. {
            let mut url = Url::parse(&format!(
                "{API_BASE}/orgs/{organization}/programs/{program_id}/clips"
            ))?;
            url.query_pairs_mut()
                .append_pair("cursor", &page.to_string())
                .append_pair("pageSize", &PAGE_SIZE.to_string());
            let clips = super::download_json_note(
                client,
                &url,
                SHOW_NAME,
                &uploader_id,
                &format!("Downloading page {}", page + 1),
            )?;
            let before = entries.len();
            clip_entries(&clips, &uploader_id, &mut entries);
            if entries.len() - before < PAGE_SIZE {
                break;
            }
        }

        let mut extra: Vec<(&str, Value)> = Vec::new();
        if let Some(title) = html_text(program.get("Name")) {
            extra.push(("title", title.into()));
        }
        if let Some(description) = html_text(program.get("Description")) {
            extra.push(("description", description.into()));
        }
        if let Some(thumbnail) = str_at(&program, "ArtworkUrl").and_then(without_query) {
            extra.push(("thumbnail", thumbnail.into()));
        }
        let title = extra
            .iter()
            .find(|(key, _)| *key == "title")
            .and_then(|(_, value)| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        Ok(Some(ExtractResult::Playlist(playlist_info(
            url,
            SHOW_NAME,
            uploader_id,
            title,
            &extra,
            entries,
        ))))
    }
}

/// A `playlist_result(entries, id, **fields)` in its key order.
fn playlist_info(
    page: &Url,
    extractor: &'static str,
    id: String,
    title: String,
    fields: &[(&str, Value)],
    entries: Vec<VideoInfo>,
) -> PlaylistInfo {
    let mut extra = Map::new();
    let mut keys = Vec::new();
    for (key, value) in fields {
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
    PlaylistInfo {
        id,
        title,
        webpage_url: page.clone(),
        extractor: extractor.into(),
        extractor_key: extractor.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    }
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    EPISODE.is_match(url)
}

/// URL matching for `--list-extractors`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST.is_match(url)
}

/// URL matching for `--list-extractors`.
pub fn show_suitable(url: &str) -> bool {
    SHOW.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(episode_match("https://omny.fm/shows/sleep-hub/cannabinoids-and-sleep").is_some());
        assert!(
            episode_match("https://omny.fm/shows/the-origin-of-things/a-song-of-hope/embed")
                .is_some()
        );
        assert!(episode_match("https://omny.fm/shows/sleep-hub/playlists").is_none());
        assert!(PLAYLIST.is_match("https://omny.fm/shows/bayfm-program03/playlists"));
        assert!(SHOW.is_match("https://omny.fm/shows/the-origin-of-things"));
        assert!(!SHOW.is_match("https://omny.fm/shows/sleep-hub/cannabinoids-and-sleep"));
    }
}
