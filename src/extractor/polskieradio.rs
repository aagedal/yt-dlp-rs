//! Upstream `PolskieRadioLegacyIE` (legacy article pages with their `data-media`
//! players), `PolskieRadioIE` (Next.js article pages with their audio
//! attachments) and `PolskieRadioPodcastListIE` (a podcast's paged episode
//! list, read through the pending entries like `InAdvancePagedList`) in
//! `yt_dlp/extractor/polskieradio.py`. The audition, player and podcast-track
//! classes, and `PolskieRadioCategoryIE` (category pages with billennium or
//! post-back pagination), are not ported.
use super::{Extractor, dict_format, dict_info, direct, download_json, download_webpage};
use crate::info_order::INFO_SHAPE;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct PolskieRadioLegacy;
pub struct PolskieRadio;
pub struct PolskieRadioPodcastList;

const LEGACY_NAME: &str = "polskieradio:legacy";
const LEGACY_KEY: &str = "PolskieRadioLegacy";
const NAME: &str = "PolskieRadio";
const PODCAST_NAME: &str = "polskieradio:podcast:list";
const PODCAST_KEY: &str = "PolskieRadioPodcastList";
const PODCAST_API: &str = "https://apipodcasts.polskieradio.pl/api";
const PODCAST_PAGE_SIZE: usize = 10;

static LEGACY_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?polskieradio(?:24)?\.pl/\d+/\d+/[Aa]rtykul/(?P<id>\d+)",
    )
    .unwrap()
});
static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:[^/]+\.)?(?:polskieradio(?:24)?|radiokierowcow)\.pl/artykul/(?P<id>\d+)",
    )
    .unwrap()
});
static PODCAST_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://podcasty\.polskieradio\.pl/podcast/(?P<id>\d+)")
        .unwrap()
});
static CONTENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<div[^>]+class="\s*this-article\s*"[^>]*>(.+?)<div[^>]+class="tags"[^>]*>"#)
        .unwrap()
});
static TIMESTAMP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<span[^>]+id="datetime2"[^>]*>(.+?)</span>"#).unwrap());
static DATA_MEDIA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<[^>]+data-media="?(\{[^>]+\})"?"#).unwrap());
static ENTRY_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([a-f\d]{8}-(?:[a-f\d]{4}-){3}[a-f\d]{12})").unwrap());

/// `_match_id` for `PolskieRadioLegacyIE`.
pub fn legacy_suitable(url: &str) -> bool {
    LEGACY_VALID_URL.is_match(url)
}

/// `_match_id` for `PolskieRadioIE`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_id` for `PolskieRadioPodcastListIE`.
pub fn podcast_list_suitable(url: &str) -> bool {
    PODCAST_VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("PolskieRadio {what} are not supported natively"))
}

fn missing(id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `strip_or_none` for a JSON value.
fn strip_or_none(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

/// `_html_search_regex`'s cleaned and stripped match, when non-empty.
fn cleaned(found: Option<regex::Captures<'_>>) -> Option<String> {
    found
        .map(|found| crate::html::clean_html(&found[1]).trim().to_owned())
        .filter(|text| !text.is_empty())
}

/// `_search_json`-free `__NEXT_DATA__`-based article data: the first of
/// `pageProps.data.articleData`, `pageProps.post.data` and `pageProps.data`
/// that is not `None` (`traverse_obj(..., get_all=False)`).
fn article_data(next_data: &Value) -> Option<&Value> {
    let props = next_data.get("props")?.get("pageProps")?;
    let candidates = [
        props.get("data").and_then(|data| data.get("articleData")),
        props.get("post").and_then(|post| post.get("data")),
        props.get("data"),
    ];
    candidates
        .into_iter()
        .flatten()
        .find(|value| !value.is_null())
}

/// A URL-only media entry (upstream's dict with `url`, processed as a direct
/// URL): its keys in the dict's order, `vcodec: none` for audio.
fn media_entry(
    page: &Url,
    (extractor, key): (&'static str, &'static str),
    media: &Url,
    fields: Vec<(&str, Value)>,
    ext: &str,
) -> VideoInfo {
    let audio = fields
        .iter()
        .any(|(name, value)| *name == "vcodec" && value.as_str() == Some("none"));
    let mut info = direct::with_format(direct::build_at(media.clone(), media, ext.into()), "");
    for format in &mut info.formats {
        format.ext = ext.into();
        format.format_id = String::new();
        if audio {
            format.vcodec = Some("none".into());
        }
    }
    info.format_id = None;
    info.direct = None;
    info.webpage_url = page.clone();
    info.extractor = extractor.into();
    info.extractor_key = key.into();
    let mut keys = Vec::new();
    for (name, value) in fields {
        if value.is_null() {
            continue;
        }
        keys.push(name.to_owned());
        match name {
            "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
            "title" => {
                info.title = value.as_str().unwrap_or_default().to_owned();
                info.extra.shift_remove(crate::info::TITLE_MISSING);
            }
            "thumbnail" => info.metadata.thumbnail = value.as_str().map(str::to_owned),
            "url" => {}
            _ => crate::metadata_parser::set_field(&mut info, name, value),
        }
    }
    let untitled = if keys.iter().any(|key| key == "title") {
        ""
    } else {
        "-untitled"
    };
    crate::info_order::mark_info(info, &format!("untyped-dict;{}{untitled}", keys.join(",")))
}

/// `_extract_webpage_player_entries(webpage, playlist_id, base_data)`: the
/// `data-media` players of a page, one entry per distinct media file.
fn player_entries(
    page: &Url,
    (extractor, key): (&'static str, &'static str),
    webpage: &str,
    base: Vec<(&str, Value)>,
) -> Vec<VideoInfo> {
    let mut urls: Vec<String> = Vec::new();
    let mut entries = Vec::new();
    for found in DATA_MEDIA.captures_iter(webpage) {
        let raw = crate::html::unescape_html(&found[1]);
        let Ok(media) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        let file = media
            .get("file")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty());
        let desc = media
            .get("desc")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty());
        let (Some(file), Some(desc)) = (file, desc) else {
            continue;
        };
        let media_url = super::proto_relative(file);
        if urls.contains(&media_url) {
            continue;
        }
        urls.push(media_url.clone());
        let Ok(url) = Url::parse(&media_url) else {
            continue;
        };
        // `entry = base_data.copy(); entry.update({...})`.
        let mut fields: Vec<(&str, Value)> = base.clone();
        let id = match media.get("id") {
            Some(Value::String(text)) => text.clone(),
            Some(other) => other.to_string(),
            None => continue,
        };
        set_or_push(&mut fields, "id", Value::from(id));
        set_or_push(&mut fields, "url", Value::from(media_url.clone()));
        let duration = super::streamable::int_or_none(media.get("length"));
        if !duration.is_null() {
            set_or_push(&mut fields, "duration", duration);
        }
        if media.get("provider").and_then(Value::as_str) == Some("audio") {
            set_or_push(&mut fields, "vcodec", Value::from("none"));
        }
        let title = percent_encoding::percent_decode_str(desc)
            .decode_utf8_lossy()
            .into_owned();
        if !title.is_empty() {
            set_or_push(&mut fields, "title", Value::from(title));
        }
        entries.push(media_entry(page, (extractor, key), &url, fields, "mp3"));
    }
    entries
}

fn set_or_push<'a>(fields: &mut Vec<(&'a str, Value)>, key: &'a str, value: Value) {
    match fields.iter_mut().find(|(name, _)| *name == key) {
        Some((_, existing)) => *existing = value,
        None => fields.push((key, value)),
    }
}

fn playlist(
    url: &Url,
    (extractor, key): (&'static str, &'static str),
    id: String,
    title: String,
    description: Option<String>,
    entries: Vec<VideoInfo>,
) -> ExtractResult {
    let mut extra = Map::new();
    let mut keys = vec!["id", "title"];
    if let Some(description) = description {
        keys.push("description");
        extra.insert("description".into(), description.into());
    }
    extra.insert(
        INFO_SHAPE.into(),
        format!("playlist-dict;{}", keys.join(",")).into(),
    );
    ExtractResult::Playlist(PlaylistInfo {
        id,
        title,
        webpage_url: url.clone(),
        extractor: extractor.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })
}

fn legacy(client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
    let Some(found) = LEGACY_VALID_URL.captures(url.as_str()) else {
        return Ok(None);
    };
    let playlist_id = found["id"].to_owned();
    crate::screen::set_extractor(LEGACY_NAME);
    crate::screen::screen(&format!(
        "[{LEGACY_NAME}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    let (webpage, final_url) =
        super::download_webpage_handle(client, url, LEGACY_NAME, &playlist_id)?;
    if VALID_URL.is_match(final_url.as_str()) {
        return Ok(Some(super::follow_url(url, final_url.to_string())));
    }

    let content = CONTENT.captures(&webpage).map(|found| found[1].to_owned());
    let timestamp = cleaned(TIMESTAMP.captures(&webpage))
        .and_then(|text| crate::dates::unified_timestamp(&text, true));
    let thumbnail = crate::html::og_search_property(&webpage, "image");
    let title = crate::html::og_search_property(&webpage, "title")
        .map(|title| title.trim().to_owned())
        .ok_or_else(|| missing(&playlist_id, "og:title"))?;
    let description = crate::html::og_search_property(&webpage, "description")
        .map(|text| text.trim().replace('\u{a0}', " "))
        .filter(|text| !text.is_empty());

    let Some(content) = content else {
        return Err(unsupported("audition records without a player list"));
    };
    let base = vec![
        ("title", Value::from(title.clone())),
        ("timestamp", timestamp.map_or(Value::Null, Value::from)),
        ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
    ];
    let entries = player_entries(url, (LEGACY_NAME, LEGACY_KEY), &content, base);
    Ok(Some(playlist(
        url,
        (LEGACY_NAME, LEGACY_KEY),
        playlist_id,
        title,
        description,
        entries,
    )))
}

fn article(client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
    let Some(found) = VALID_URL.captures(url.as_str()) else {
        return Ok(None);
    };
    let playlist_id = found["id"].to_owned();
    crate::screen::set_extractor(NAME);
    crate::screen::screen(&format!(
        "[{NAME}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    let webpage = download_webpage(client, url, NAME, &playlist_id)?;
    let next_data = super::search_nextjs_data(&webpage, NAME, &playlist_id)?;
    let Some(article) = article_data(&next_data) else {
        return Err(missing(&playlist_id, "article data"));
    };
    let title = strip_or_none(article.get("title"));
    let description = strip_or_none(article.get("lead"));

    let mut entries = Vec::new();
    let attachments = article
        .get("attachments")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|entry| entry.get("fileType").and_then(Value::as_str) == Some("Audio"));
    for attachment in attachments {
        let Some(file) = attachment.get("file").and_then(Value::as_str) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {playlist_id}: An extractor error has occurred. (caused by KeyError('file')); {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let ext = attachment
            .get("fileName")
            .and_then(Value::as_str)
            .and_then(super::direct::determine_ext);
        let entry_id = ENTRY_ID
            .captures(file)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| missing(&playlist_id, "entry id"))?;
        let entry_title = strip_or_none(attachment.get("description")).or_else(|| title.clone());
        let media = Url::parse(file)?;
        let mut fields = vec![
            ("url", Value::from(file)),
            ("ext", ext.map_or(Value::Null, Value::from)),
        ];
        fields.push(("id", Value::from(entry_id)));
        fields.push(("title", entry_title.map_or(Value::Null, Value::from)));
        entries.push(media_entry(url, (NAME, NAME), &media, fields, "mp3"));
    }
    if entries.is_empty() {
        let content = article
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| missing(&playlist_id, "content"))?;
        let base = vec![("title", title.clone().map_or(Value::Null, Value::from))];
        entries = player_entries(url, (NAME, NAME), content, base);
    }
    Ok(Some(playlist(
        url,
        (NAME, NAME),
        playlist_id,
        title.unwrap_or_default(),
        description,
        entries,
    )))
}

/// `pending` state of a podcast's episode list.
pub(crate) fn is_podcast_pending_state(state: &Value) -> bool {
    state["kind"] == PODCAST_NAME
}

fn episode(page: &Url, data: &Value) -> Result<VideoInfo> {
    let id = data
        .get("guid")
        .and_then(Value::as_str)
        .ok_or_else(|| missing("", "episode guid"))?
        .to_owned();
    let url = data
        .get("url")
        .and_then(Value::as_str)
        .ok_or_else(|| missing(&id, "episode url"))?;
    let format = dict_format(
        vec![
            ("url", url.into()),
            (
                "filesize",
                super::streamable::int_or_none(data.get("fileSize")),
            ),
        ],
        NAME,
        &id,
    )?;
    let title = data.get("title").cloned().unwrap_or(Value::Null);
    let timestamp = data
        .get("publishDate")
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601);
    let thumbnail = data
        .get("image")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none);
    Ok(dict_info(
        page,
        (PODCAST_NAME, PODCAST_KEY),
        vec![
            ("id", id.clone().into()),
            ("formats", Value::Null),
            ("title", title.clone()),
            (
                "description",
                data.get("description").cloned().unwrap_or(Value::Null),
            ),
            (
                "duration",
                super::streamable::int_or_none(data.get("length")),
            ),
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            (
                "series",
                data.get("podcastTitle").cloned().unwrap_or(Value::Null),
            ),
            ("episode", title),
        ],
        vec![format],
    ))
}

fn podcast_page(client: &Client, podcast_id: &str, page: usize) -> Result<Value> {
    download_json(
        client,
        &Url::parse(&format!(
            "{PODCAST_API}/Podcasts/{podcast_id}/?pageSize={PODCAST_PAGE_SIZE}&page={page}"
        ))?,
        PODCAST_NAME,
        podcast_id,
    )
}

/// `InAdvancePagedList` pages of a podcast's episodes, read on request.
pub(crate) fn fetch_podcast_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let podcast_id = state["podcast_id"].as_str().unwrap_or_default().to_owned();
    let page = Url::parse(state["url"].as_str().unwrap_or_default())?;
    let first_page = state["first_page"].clone();
    let mut fetch = |pagenum: usize| -> Result<Vec<VideoInfo>> {
        let data = if pagenum == 0 {
            first_page.clone()
        } else {
            podcast_page(client, &podcast_id, pagenum + 1)?
        };
        let items = match data.get("items") {
            Some(Value::Array(items)) => items,
            _ => return Err(unsupported("podcast pages without an item list")),
        };
        items.iter().map(|item| episode(&page, item)).collect()
    };
    let placeholder = || Ok(direct::build_at(page.clone(), &page, String::new()));
    super::paged::fetch_requested(
        state,
        limit,
        entries,
        PODCAST_PAGE_SIZE,
        &mut fetch,
        &placeholder,
    )
}

fn podcast_list(client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
    let Some(found) = PODCAST_VALID_URL.captures(url.as_str()) else {
        return Ok(None);
    };
    let podcast_id = found["id"].to_owned();
    crate::screen::set_extractor(PODCAST_NAME);
    crate::screen::screen(&format!(
        "[{PODCAST_NAME}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    let data = podcast_page(client, &podcast_id, 1)?;
    let id = match data.get("id") {
        Some(Value::String(text)) => text.clone(),
        Some(other) => other.to_string(),
        None => return Err(missing(&podcast_id, "podcast id")),
    };
    let title = data.get("title").and_then(Value::as_str).map(str::to_owned);
    let description = data
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let uploader = data
        .get("announcer")
        .and_then(Value::as_str)
        .map(str::to_owned);

    let state = json!({
        "kind": PODCAST_NAME,
        "podcast_id": podcast_id,
        "url": url.as_str(),
        "first_page": data,
        "random_access": true,
    });
    let mut extra = Map::new();
    let mut keys = vec!["id"];
    if title.is_some() {
        keys.push("title");
    }
    if description.is_some() {
        keys.push("description");
        extra.insert(
            "description".into(),
            description.map_or(Value::Null, Value::from),
        );
    }
    if uploader.is_some() {
        keys.push("uploader");
        extra.insert("uploader".into(), uploader.map_or(Value::Null, Value::from));
    }
    // `{'_type': 'playlist', 'entries': ..., 'id', 'title', ...}`: the entries
    // come first, then the dict's keys.
    extra.insert(
        INFO_SHAPE.into(),
        format!(
            "playlist-dict;;after={}{}",
            keys.join(","),
            crate::info_order::GENERATOR
        )
        .into(),
    );
    Ok(Some(ExtractResult::Playlist(PlaylistInfo {
        id,
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: PODCAST_NAME.into(),
        extractor_key: PODCAST_KEY.into(),
        nested: Vec::new(),
        pending: Some(crate::PendingEntries { state }),
        entries: Vec::new(),
        extra,
    })))
}

impl Extractor for PolskieRadioLegacy {
    fn key(&self) -> &'static str {
        LEGACY_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        legacy(client, url)
    }
}

impl Extractor for PolskieRadio {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        article(client, url)
    }
}

impl Extractor for PolskieRadioPodcastList {
    fn key(&self) -> &'static str {
        PODCAST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        podcast_list(client, url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(legacy_suitable(
            "https://www.polskieradio.pl/8/2382/Artykul/2534482,Zagarysci-Poezja-jak-spoiwo"
        ));
        assert!(!legacy_suitable(
            "https://jedynka.polskieradio.pl/artykul/1587943"
        ));
        assert!(suitable("https://jedynka.polskieradio.pl/artykul/1587943"));
        assert!(suitable("https://radiokierowcow.pl/artykul/2694529"));
        assert!(!suitable("https://www.polskieradio.pl/10/4930"));
        assert!(podcast_list_suitable(
            "https://podcasty.polskieradio.pl/podcast/8/"
        ));
        assert!(!podcast_list_suitable(
            "https://podcasty.polskieradio.pl/track/6eafe403-cb8f-4756-b896-4455c3713c32"
        ));
    }
}
