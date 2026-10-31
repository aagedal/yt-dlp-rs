//! Upstream `TVPEmbedIE` (the TVPlayer2 `getTvpConfig` JSONP of a video's
//! files, posters and subtitles), `TVPVODVideoIE` (a VOD episode's API
//! metadata with its HLS and DASH playlists) and `TVPVODSeriesIE` (a serial's
//! seasons and their episodes as URL results) in `yt_dlp/extractor/tvp.py`.
//! The JSONP callback is fixed to the first name upstream picks at random.
//! Payment-protected content and `TVPIE`/`TVPStreamIE` are not ported.
use super::{
    Extractor, dict_format, dict_info, download_json, download_webpage, extract_ism_optional,
    extract_m3u8_master_optional, extract_mpd_optional_note, update_url_query,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TVPEmbed;
pub struct TVPVODVideo;
pub struct TVPVODSeries;

const EMBED_NAME: &str = "tvp:embed";
const EMBED_KEY: &str = "TVPEmbed";
const VIDEO_NAME: &str = "tvp:vod";
const VIDEO_KEY: &str = "TVPVODVideo";
const SERIES_NAME: &str = "tvp:vod:series";
const SERIES_KEY: &str = "TVPVODSeries";
const API_BASE: &str = "https://vod.tvp.pl/api/products";
/// The JSONP callback (upstream picks one of several names at random).
const CALLBACK: &str = "jebacpis";

static EMBED_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?:tvp:|https?://(?:[^/]+\.)?(?:tvp(?:parlament)?\.pl|tvp\.info|tvpworld\.com|swipeto\.pl)/(?:sess/(?:tvplayer\.php\?.*?object_id|TVPlayer2/(?:embed|api)\.php\?.*[Ii][Dd])|shared/details\.php\?.*?object_id)=)(?P<id>\d+)",
    )
    .unwrap()
});
/// `(?<!-odcinki)` is checked on the slug (the regex crate has no lookbehind).
static VIDEO_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://vod\.tvp\.pl/(?P<category>[a-z\d-]+,\d+)/(?P<slug>[a-z\d-]+)(?:-odcinki,\d+/odcinek--?\d+,S-?\d+E-?\d+)?,(?P<id>\d+)/?(?:[?#]|$)",
    )
    .unwrap()
});
static SERIES_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://vod\.tvp\.pl/[a-z\d-]+,\d+/[a-z\d-]+-odcinki,(?P<id>\d+)(?:\?[^#]+)?(?:#.+)?$",
    )
    .unwrap()
});

/// `_match_id` for `TVPEmbedIE`.
pub fn embed_suitable(url: &str) -> bool {
    EMBED_VALID_URL.is_match(url)
}

/// `_match_valid_url` for `TVPVODVideoIE` (without its `-odcinki` lookbehind).
pub fn video_suitable(url: &str) -> bool {
    VIDEO_VALID_URL
        .captures(url)
        .is_some_and(|found| !found["slug"].ends_with("-odcinki"))
}

/// `_match_id` for `TVPVODSeriesIE`.
pub fn series_suitable(url: &str) -> bool {
    SERIES_VALID_URL.is_match(url)
}

fn reported(id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{EMBED_NAME}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `str_or_none` for a JSON value.
fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

/// `int_or_none` for a JSON value.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `dict_get(d, keys)`: the first truthy value.
fn dict_get_truthy(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    })
}

/// `clean_html(dict_get(video, keys))`.
fn cleaned_text(value: &Value, keys: &[&str]) -> Option<String> {
    dict_get_truthy(value, keys).map(|text| crate::html::clean_html(&text))
}

fn call_api(
    client: &Client,
    resource: &str,
    video_id: &str,
    query: &[(&str, &str)],
) -> Result<Value> {
    let mut params = vec![("lang", "pl"), ("platform", "BROWSER")];
    params.extend_from_slice(query);
    let url = update_url_query(&Url::parse(&format!("{API_BASE}/{resource}"))?, &params);
    download_json(client, &url, VIDEO_NAME, video_id)
}

/// `_parse_video(video, with_url)`'s fields in the upstream dict's order.
fn parse_video(video: &Value) -> Vec<(&'static str, Value)> {
    let mut fields: Vec<(&'static str, Value)> = Vec::new();
    if let Some(id) = str_or_none(video.get("id")) {
        fields.push(("id", id.into()));
    }
    if let Some(title) = video.get("title").filter(|title| !title.is_null()) {
        fields.push(("title", title.clone()));
    }
    if let Some(age) = int_or_none(video.get("rating")) {
        fields.push(("age_limit", age.into()));
    }
    if let Some(duration) = int_or_none(video.get("duration")) {
        fields.push(("duration", duration.into()));
    }
    if let Some(number) = int_or_none(video.get("number")) {
        fields.push(("episode_number", number.into()));
    }
    if let Some(series) = video
        .get("season")
        .and_then(|season| season.get("serial"))
        .and_then(|serial| serial.get("title"))
        .and_then(|title| str_or_none(Some(title)))
    {
        fields.push(("series", series.into()));
    }
    let mut thumbnails = Vec::new();
    if let Some(images) = video.get("images").and_then(Value::as_object) {
        for list in images.values().filter_map(Value::as_array) {
            for image in list {
                if let Some(url) = image
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(super::json_ld::url_or_none)
                {
                    thumbnails.push(json!({ "url": url }));
                }
            }
        }
    }
    fields.push(("thumbnails", Value::Array(thumbnails)));
    fields.push((
        "description",
        cleaned_text(video, &["lead", "description"]).map_or(Value::Null, Value::from),
    ));
    fields
}

/// `TVPEmbedIE._real_extract`.
fn embed(client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
    let Some(found) = EMBED_VALID_URL.captures(url.as_str()) else {
        return Ok(None);
    };
    let video_id = found["id"].to_owned();
    crate::screen::set_extractor(EMBED_NAME);
    crate::screen::screen(&format!(
        "[{EMBED_NAME}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    let config_url = Url::parse(&format!(
        "https://www.tvp.pl/sess/TVPlayer2/api.php?id={video_id}&@method=getTvpConfig&@callback={CALLBACK}"
    ))?;
    let webpage = download_webpage(client, &config_url, EMBED_NAME, &video_id)?;

    // Stripping the JSONP padding: `webpage[15 + len(callback):-3]`.
    let padded = 15 + CALLBACK.len();
    let text: Vec<char> = webpage.chars().collect();
    let datastr: String = text
        .get(padded.min(text.len())..text.len().saturating_sub(3))
        .unwrap_or_default()
        .iter()
        .collect();
    if let Some(error) = datastr.strip_prefix("null,") {
        let error: Value = serde_json::from_str(error).unwrap_or(Value::Null);
        let desc = error
            .get(0)
            .and_then(|first| first.get("desc"))
            .and_then(Value::as_str)
            .unwrap_or("unexpected JSON error");
        if desc == "Obiekt wymaga płatności" {
            return Err(Error::UnsupportedHtml5(
                "TVP videos that require payment and log-in are not supported natively".into(),
            ));
        }
        return Err(Error::Reported(format!(
            "[{EMBED_NAME}] {desc}; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }
    let data: Value =
        crate::json::from_str(&datastr).map_err(|_| reported(&video_id, "Failed to parse JSON"))?;
    let content = data.get("content").ok_or_else(|| {
        reported(
            &video_id,
            "An extractor error has occurred. (caused by KeyError('content'))",
        )
    })?;
    let info = content.get("info").ok_or_else(|| {
        reported(
            &video_id,
            "An extractor error has occurred. (caused by KeyError('info'))",
        )
    })?;
    let is_live = info.get("isLive").and_then(Value::as_bool).unwrap_or(false);
    if info.get("isGeoBlocked").and_then(Value::as_bool) == Some(true) {
        return Err(Error::UnsupportedHtml5(
            "geo-restricted TVP videos are not supported natively".into(),
        ));
    }

    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for file in content
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(video_url) = file
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let manifest = Url::parse(&video_url)?;
        let ext = super::direct::determine_ext(&video_url).unwrap_or_default();
        if ext == "m3u8" {
            if let Some(stream) = extract_m3u8_master_optional(
                client,
                &manifest,
                EMBED_NAME,
                &video_id,
                Some("hls"),
                false,
                is_live,
            ) {
                formats.extend(stream.formats);
            }
        } else if ext == "mpd" {
            if is_live {
                // Upstream skips live DASH manifests (no client can play them).
                continue;
            }
            if let Some(stream) = extract_mpd_optional_note(
                client,
                &manifest,
                EMBED_NAME,
                &video_id,
                Some("dash"),
                "Downloading MPD manifest",
            ) {
                formats.extend(stream.formats);
            }
        } else if ext == "f4m" {
            return Err(Error::UnsupportedHtml5(
                "TVP HDS (F4M) formats are not supported natively".into(),
            ));
        } else if video_url.ends_with(".ism/manifest") {
            let (ism_formats, ism_subtitles) =
                extract_ism_optional(client, EMBED_NAME, &manifest, &video_id, Some("mss"))?;
            formats.extend(ism_formats);
            super::svt::merge_subtitles(&mut subtitles, ism_subtitles);
        } else {
            let quality = file.get("quality");
            let fps = int_or_none(quality.and_then(|q| q.get("fps")));
            let tbr =
                int_or_none(quality.and_then(|q| q.get("bitrate"))).map(|bitrate| bitrate / 1000);
            let width = int_or_none(quality.and_then(|q| q.get("width")));
            let height = int_or_none(quality.and_then(|q| q.get("height")));
            let ext_value = if ext.is_empty() {
                file.get("type").cloned().unwrap_or(Value::Null)
            } else {
                Value::from(ext.clone())
            };
            let mut format = dict_format(
                vec![
                    ("format_id", "direct".into()),
                    ("url", video_url.clone().into()),
                    ("ext", ext_value),
                    ("fps", fps.map_or(Value::Null, Value::from)),
                    ("tbr", tbr.map_or(Value::Null, Value::from)),
                    ("width", width.map_or(Value::Null, Value::from)),
                    ("height", height.map_or(Value::Null, Value::from)),
                ],
                EMBED_NAME,
                &video_id,
            )?;
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
    }

    let mut fields: Vec<(&str, Value)> = vec![
        ("id", video_id.clone().into()),
        (
            "title",
            dict_get_truthy(info, &["subtitle", "title", "seoTitle"])
                .map_or(Value::Null, Value::from),
        ),
        (
            "description",
            dict_get_truthy(info, &["description", "seoDescription"])
                .map_or(Value::Null, Value::from),
        ),
    ];
    let mut thumbnails = Vec::new();
    for thumb in content
        .get("posters")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(thumb_url) = thumb
            .get("src")
            .and_then(Value::as_str)
            .filter(|src| !src.is_empty())
        else {
            continue;
        };
        if thumb_url.contains("{width}") || thumb_url.contains("{height}") {
            continue;
        }
        // `_sanitize_thumbnails` numbers the posters that have no ID.
        thumbnails.push(json!({
            "url": thumb_url,
            "width": thumb.get("width").cloned().unwrap_or(Value::Null),
            "height": thumb.get("height").cloned().unwrap_or(Value::Null),
            "id": thumbnails.len().to_string(),
        }));
    }
    fields.push(("thumbnails", Value::Array(thumbnails)));
    let mut age_limit = int_or_none(info.get("ageGroup").and_then(|group| group.get("minAge")));
    if age_limit == Some(1) {
        age_limit = Some(0);
    }
    fields.push(("age_limit", age_limit.map_or(Value::Null, Value::from)));
    fields.push(("is_live", Value::Bool(is_live)));
    let duration = if is_live {
        None
    } else {
        int_or_none(info.get("duration"))
    };
    fields.push(("duration", duration.map_or(Value::Null, Value::from)));
    fields.push(("formats", Value::Null));
    fields.push(("subtitles", Value::Null));

    let mut info_fields: Vec<(&str, Value)> = fields;
    let is_vod = info.get("vortalName").and_then(Value::as_str) == Some("vod");
    if is_vod {
        let title = info.get("title").map_or("None".to_owned(), text_of);
        let subtitle = info.get("subtitle").map_or("None".to_owned(), text_of);
        for (key, value) in info_fields.iter_mut() {
            if *key == "title" {
                *value = format!("{title}, {subtitle}").into();
            }
        }
        info_fields.push(("series", info.get("title").cloned().unwrap_or(Value::Null)));
        info_fields.push(("season", info.get("season").cloned().unwrap_or(Value::Null)));
        info_fields.push((
            "episode_number",
            info.get("episode").cloned().unwrap_or(Value::Null),
        ));
    }
    let mut subtitle_tracks = Vec::new();
    for sub in content
        .get("subtitles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(sub_url) = sub
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        let lang = str_or_none(sub.get("lang")).unwrap_or_default();
        let parsed = Url::parse(sub_url).map_err(|_| {
            Error::UnsupportedHtml5(
                "TVP subtitle URLs that are not absolute are not supported natively".into(),
            )
        })?;
        let ext = str_or_none(sub.get("type")).unwrap_or_default();
        subtitle_tracks.push((Some(lang), track(parsed, ext)));
    }
    let keys: Vec<&str> = info_fields.iter().map(|(key, _)| *key).collect();
    let shape = format!("dict;{};thumbnail=url,width,height", keys.join(","));
    let mut info = crate::info_order::mark_info(
        dict_info(url, (EMBED_NAME, EMBED_KEY), info_fields, formats),
        &shape,
    );
    for (lang, format) in subtitle_tracks {
        match subtitles.entries.iter_mut().find(|(key, _)| *key == lang) {
            Some((_, existing)) => existing.push(format),
            None => subtitles.entries.push((lang, vec![format])),
        }
    }
    info.subtitles = subtitles;
    Ok(Some(info))
}

/// A subtitle track of `url` in `ext`.
fn track(url: Url, ext: String) -> crate::SubtitleFormat {
    crate::SubtitleFormat {
        name: None,
        url,
        ext,
        protocol: None,
        fragments: Vec::new(),
        extra: Map::new(),
    }
}

/// `str(value)` of a JSON text value.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => "None".to_owned(),
        other => other.to_string(),
    }
}

/// `TVPVODVideoIE._real_extract`.
fn vod_video(client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
    let Some(found) = VIDEO_VALID_URL.captures(url.as_str()) else {
        return Ok(None);
    };
    if found["slug"].ends_with("-odcinki") {
        return Ok(None);
    }
    let category = found["category"].to_owned();
    let video_id = found["id"].to_owned();
    crate::screen::set_extractor(VIDEO_NAME);
    crate::screen::screen(&format!(
        "[{VIDEO_NAME}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    let is_live = category == "live,1";
    let entity = if is_live { "lives" } else { "vods" };
    let video = call_api(client, &format!("{entity}/{video_id}"), &video_id, &[])?;
    let mut fields = parse_video(&video);
    let playlist = call_api(
        client,
        &format!("{video_id}/videos/playlist"),
        &video_id,
        &[("videoType", "MOVIE")],
    )?;

    let mut formats = Vec::new();
    for manifest_url in hls_sources(&playlist) {
        let manifest = Url::parse(&manifest_url)?;
        if let Some(stream) = super::extract_m3u8_master_optional(
            client, &manifest, VIDEO_NAME, &video_id, None, false, false,
        ) {
            formats.extend(stream.formats);
        }
    }
    for manifest_url in dash_sources(&playlist) {
        let manifest = Url::parse(&manifest_url)?;
        if let Some(stream) = super::extract_mpd_optional_note(
            client,
            &manifest,
            VIDEO_NAME,
            &video_id,
            None,
            "Downloading MPD manifest",
        ) {
            formats.extend(stream.formats);
        }
    }
    let mut subtitles = crate::Subtitles::default();
    for sub in playlist
        .get("subtitles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let lang = str_or_none(sub.get("language"))
            .filter(|lang| !lang.is_empty())
            .unwrap_or_else(|| "und".to_owned());
        let Some(sub_url) = sub.get("url").and_then(Value::as_str) else {
            continue;
        };
        let format = track(Url::parse(sub_url)?, "ttml".to_owned());
        match subtitles
            .entries
            .iter_mut()
            .find(|(key, _)| *key == Some(lang.clone()))
        {
            Some((_, existing)) => existing.push(format),
            None => subtitles.entries.push((Some(lang), vec![format])),
        }
    }
    fields.push(("formats", Value::Null));
    fields.push(("subtitles", Value::Null));
    fields.push(("is_live", Value::Bool(is_live)));
    let mut info = dict_info(url, (VIDEO_NAME, VIDEO_KEY), fields, formats);
    info.subtitles = subtitles;
    Ok(Some(info))
}

fn hls_sources(playlist: &Value) -> Vec<String> {
    sources(playlist, "HLS")
}

fn dash_sources(playlist: &Value) -> Vec<String> {
    sources(playlist, "DASH")
}

fn sources(playlist: &Value, kind: &str) -> Vec<String> {
    playlist
        .get("sources")
        .and_then(|sources| sources.get(kind))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|source| source.get("src").and_then(Value::as_str))
        .filter_map(super::json_ld::url_or_none)
        .collect()
}

/// A serial's episode as a `url` result of `TVPVODVideoIE`.
fn episode_entry(video: &Value) -> Result<VideoInfo> {
    let web_url = video.get("webUrl").and_then(Value::as_str).ok_or_else(|| {
        reported(
            "",
            "An extractor error has occurred. (caused by KeyError('webUrl'))",
        )
    })?;
    let target = Url::parse(web_url)?;
    let mut info = super::direct::build_at(target.clone(), &target, String::new());
    info.formats.clear();
    info.direct = None;
    // `_parse_video(video)`'s dict, then the URL result's `_type`, `url` and `ie_key`.
    let mut entry = Map::new();
    for (key, value) in parse_video(video) {
        if !value.is_null() || key == "description" {
            entry.insert(key.into(), value);
        }
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), web_url.into());
    entry.insert("ie_key".into(), VIDEO_KEY.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": web_url,
            "force_videoid": null,
            "fields": {},
            "ie_key": VIDEO_KEY,
            "extractor": VIDEO_KEY,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

/// `TVPVODSeriesIE._real_extract`: every season's episodes as URL results.
fn vod_series(client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
    let Some(found) = SERIES_VALID_URL.captures(url.as_str()) else {
        return Ok(None);
    };
    let playlist_id = found["id"].to_owned();
    crate::screen::set_extractor(SERIES_NAME);
    crate::screen::screen(&format!(
        "[{SERIES_NAME}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    let metadata = call_api(
        client,
        &format!("vods/serials/{playlist_id}"),
        &playlist_id,
        &[],
    )?;
    let seasons = call_api(
        client,
        &format!("vods/serials/{playlist_id}/seasons"),
        &playlist_id,
        &[],
    )?;
    let mut entries = Vec::new();
    for season in seasons.as_array().into_iter().flatten() {
        let season_id = str_or_none(season.get("id")).unwrap_or_default();
        let episodes = call_api(
            client,
            &format!("vods/serials/{playlist_id}/seasons/{season_id}/episodes"),
            &playlist_id,
            &[],
        )?;
        for episode in episodes.as_array().into_iter().flatten() {
            entries.push(episode_entry(episode)?);
        }
    }
    let title = metadata
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned);
    // `clean_html(traverse_obj(metadata, ('description', 'lead'), ...))`.
    let description = metadata
        .get("description")
        .and_then(|description| description.get("lead"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(crate::html::clean_html);
    let categories = Value::Array(vec![
        metadata
            .get("mainCategory")
            .and_then(|category| category.get("name"))
            .cloned()
            .unwrap_or(Value::Null),
    ]);
    let age_limit = int_or_none(metadata.get("rating"));
    let mut extra = Map::new();
    extra.insert("categories".into(), categories);
    extra.insert(
        "age_limit".into(),
        age_limit.map_or(Value::Null, Value::from),
    );
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!(
            "playlist-dict;categories,age_limit,id,title{}",
            crate::info_order::GENERATOR
        )
        .into(),
    );
    if let Some(description) = description {
        extra.insert("description".into(), description.into());
    }
    Ok(Some(ExtractResult::Playlist(PlaylistInfo {
        id: playlist_id,
        title: title.unwrap_or_default(),
        webpage_url: url.clone(),
        extractor: SERIES_NAME.into(),
        extractor_key: SERIES_KEY.into(),
        nested: Vec::new(),
        pending: None,
        entries,
        extra,
    })))
}

impl Extractor for TVPEmbed {
    fn key(&self) -> &'static str {
        EMBED_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(embed(client, url)?.map(ExtractResult::Video))
    }
}

impl Extractor for TVPVODVideo {
    fn key(&self) -> &'static str {
        VIDEO_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(vod_video(client, url)?.map(ExtractResult::Video))
    }
}

impl Extractor for TVPVODSeries {
    fn key(&self) -> &'static str {
        SERIES_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        vod_series(client, url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(embed_suitable("tvp:194536"));
        assert!(embed_suitable(
            "https://www.tvp.pl/sess/tvplayer.php?object_id=51247504&amp;autoplay=false"
        ));
        assert!(!embed_suitable("https://www.tvp.pl/"));
        assert!(video_suitable(
            "https://vod.tvp.pl/dla-dzieci,24/laboratorium-alchemika-odcinki,309338/odcinek-24,S01E24,311357"
        ));
        assert!(!video_suitable(
            "https://vod.tvp.pl/seriale,18/ranczo-odcinki,316445"
        ));
        assert!(series_suitable(
            "https://vod.tvp.pl/seriale,18/ranczo-odcinki,316445"
        ));
    }
}
