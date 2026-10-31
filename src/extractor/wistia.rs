//! Upstream `WistiaIE`, `WistiaPlaylistIE` and `WistiaChannelIE`
//! (`yt_dlp/extractor/wistia.py`): the `fast.wistia.net/embed` media,
//! playlist and channel JSON, the media's assets as formats and thumbnails
//! and its captions, and the embeds GenericIE finds on other pages.
use super::clyp::{float_or_none, key_error};
use super::streamable::int_or_none;
use super::{Extractor, dict_format, dict_info};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Wistia;
pub struct WistiaPlaylist;
pub struct WistiaChannel;

pub(crate) const NAME: &str = "Wistia";
pub(crate) const KEY: &str = "Wistia";
pub(crate) const CHANNEL_NAME: &str = "WistiaChannel";
pub(crate) const CHANNEL_KEY: &str = "WistiaChannel";
const PLAYLIST_NAME: &str = "WistiaPlaylist";
const EMBED_BASE_URL: &str = "http://fast.wistia.net/embed/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^(?:wistia:|https?://(?:\w+\.)?wistia\.(?:net|com)/(?:embed/)?(?:iframe|medias)/)(?P<id>[a-z0-9]{10})",
    )
    .unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:\w+\.)?wistia\.(?:net|com)/(?:embed/)?playlists/(?P<id>[a-z0-9]{10})")
        .unwrap()
});
static CHANNEL_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:wistiachannel:|https?://(?:\w+\.)?wistia\.(?:net|com)/(?:embed/)?channel/)(?P<id>[a-z0-9]{10})",
    )
    .unwrap()
});

/// `WistiaIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `WistiaPlaylistIE._match_id`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `WistiaChannelIE._match_id`.
pub fn channel_suitable(url: &str) -> bool {
    CHANNEL_URL.is_match(url)
}

/// `WistiaIE.get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `WistiaChannelIE.get_temp_id`.
pub(crate) fn channel_temp_id(url: &str) -> Option<String> {
    CHANNEL_URL
        .captures(url)
        .map(|found| found["id"].to_owned())
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Wistia {what} are not supported natively"))
}

/// Python `str(value)` of a JSON scalar this port renders like upstream.
fn py_str(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Bool(true) => Some("True".into()),
        Value::Bool(false) => Some("False".into()),
        Value::Number(number) if number.is_i64() || number.is_u64() => Some(number.to_string()),
        _ => None,
    }
}

/// Python `urllib.parse.unquote_plus`.
fn unquote_plus(text: &str) -> String {
    percent_encoding::percent_decode_str(&text.replace('+', " "))
        .decode_utf8_lossy()
        .into_owned()
}

/// `WistiaBaseIE._download_embed_config`: the config JSON (with the
/// `--video-password` query and the Referer some videos require), its
/// `error` and the password-protection plugin.
fn download_embed_config(
    client: &Client,
    (extractor, config_type): (&'static str, &str),
    config_id: &str,
    referer: &str,
) -> Result<Value> {
    let base = format!("{EMBED_BASE_URL}{config_type}/{config_id}");
    let password = super::video_password();
    let mut config_url = Url::parse(&format!("{base}.json"))?;
    if let Some(password) = &password {
        config_url = super::update_url_query(&config_url, &[("password", password)]);
    }
    let referer = if referer.starts_with("http") {
        referer
    } else {
        &base
    };
    let config = super::download_json_headers(
        client,
        &config_url,
        extractor,
        config_id,
        &[("Referer", referer)],
    )?;
    if let Some(error) = config
        .get("error")
        .filter(|error| super::lnk::truthy(Some(error)))
    {
        let error = py_str(error).ok_or_else(|| unsupported("config errors that are not text"))?;
        return Err(Error::Reported(format!(
            "[{extractor}] {config_id}: Error while getting the playlist: {error}"
        )));
    }
    // `('media', ('embed_options', 'embedOptions'), 'plugin',
    // 'passwordProtectedVideo', 'on', any)`.
    let protected = ["embed_options", "embedOptions"].iter().find_map(|key| {
        config
            .get("media")
            .and_then(|media| media.get(key))
            .and_then(|options| options.get("plugin"))
            .and_then(|plugin| plugin.get("passwordProtectedVideo"))
            .and_then(|plugin| plugin.get("on"))
            .filter(|on| !on.is_null())
    });
    if protected.and_then(Value::as_str) == Some("true") {
        let message = if password.is_some() {
            "Invalid video password"
        } else {
            "This content is password-protected. Use the --video-password option"
        };
        return Err(Error::Reported(format!(
            "[{extractor}] {config_id}: {message}"
        )));
    }
    Ok(config)
}

/// `WistiaBaseIE._get_real_ext`: the URL's extension, else (for `.bin`
/// URLs) the non-fatal HEAD response's, with `mov` as `mp4`.
fn real_ext(client: &Client, extractor: &str, url: &str) -> Result<String> {
    let mut ext = super::direct::determine_ext(url).unwrap_or_else(|| "bin".into());
    if ext == "bin" {
        crate::screen::screen(&format!("[{extractor}] Checking media extension"));
        let target =
            Url::parse(url).map_err(|_| unsupported("asset URLs that are not absolute URLs"))?;
        match super::direct::head_with_headers(client, &target, &[]) {
            Ok(response) => {
                ext = super::direct::urlhandle_detect_ext(&response, Some("bin"));
            }
            Err(error) => crate::screen::warning(&format!(
                "[{extractor}] HEAD request returned error: {}",
                crate::upstream_error_text(&error)
            )),
        }
    }
    Ok(if ext == "mov" { "mp4".into() } else { ext })
}

/// An optional string field (`a.get(key)`), `None` for other types.
fn text_field<'a>(asset: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match asset.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(unsupported(&format!(
            "asset {key} values that are not strings"
        ))),
    }
}

/// An asset's format dict; its `int_or_none` bitrate is an int `tbr`.
fn asset_format(
    fields: Vec<(&str, Value)>,
    extractor: &str,
    video_id: &str,
) -> Result<crate::VideoFormat> {
    let int_rate = fields
        .iter()
        .any(|(key, value)| *key == "tbr" && !value.is_null());
    let mut format = dict_format(fields, extractor, video_id)?;
    if int_rate {
        crate::info_order::mark_int_rates(&mut format);
    }
    Ok(format)
}

/// `WistiaBaseIE._extract_media`: the media's assets as formats (an HLS
/// asset also as its `.ts` file) and thumbnails, and its captions.
fn extract_media(
    client: &Client,
    page: &Url,
    embed_config: &Value,
    (extractor, key): (&'static str, &'static str),
    temp_id: &str,
) -> Result<VideoInfo> {
    let data = embed_config
        .get("media")
        .ok_or_else(|| key_error(temp_id, "media"))?;
    if !data.is_object() {
        return Err(unsupported("media configs that are not objects"));
    }
    let video_id = data
        .get("hashedId")
        .ok_or_else(|| key_error(temp_id, "hashedId"))?
        .as_str()
        .ok_or_else(|| unsupported("hashed IDs that are not strings"))?
        .to_owned();
    let title = data.get("name").ok_or_else(|| key_error(temp_id, "name"))?;
    if !title.is_string() {
        return Err(unsupported("media names that are not strings"));
    }
    let assets = data
        .get("assets")
        .ok_or_else(|| key_error(temp_id, "assets"))?
        .as_array()
        .ok_or_else(|| unsupported("asset lists that are not arrays"))?;
    let mut formats = Vec::new();
    let mut thumbnails = Vec::new();
    for asset in assets {
        if !asset.is_object() {
            return Err(unsupported("assets that are not objects"));
        }
        let aurl = match asset.get("url") {
            Some(Value::String(aurl)) if !aurl.is_empty() => aurl.as_str(),
            value if !super::lnk::truthy(value) => continue,
            _ => return Err(unsupported("asset URLs that are not strings")),
        };
        let status_ok = match asset.get("status") {
            None | Some(Value::Null) => true,
            Some(Value::Number(number)) => number.as_f64() == Some(2.0),
            Some(_) => false,
        };
        let atype = text_field(asset, "type")?;
        if !status_ok || matches!(atype, Some("preview" | "storyboard")) {
            continue;
        }
        if matches!(atype, Some("still" | "still_image")) {
            let ext = real_ext(client, extractor, aurl)?;
            thumbnails.push(json!({
                "url": aurl.replace(".bin", &format!(".{ext}")),
                "width": int_or_none(asset.get("width")),
                "height": int_or_none(asset.get("height")),
                "filesize": int_or_none(asset.get("size")),
            }));
            continue;
        }
        let aext = match asset.get("ext") {
            Some(Value::String(ext)) if !ext.is_empty() => ext.clone(),
            value if !super::lnk::truthy(value) => real_ext(client, extractor, aurl)?,
            _ => return Err(unsupported("asset extensions that are not strings")),
        };
        let display_name = text_field(asset, "display_name")?;
        let format_id = match (atype, display_name) {
            (Some(atype), Some(display_name)) if atype.ends_with("_video") => {
                Value::from(format!("{}-{display_name}", &atype[..atype.len() - 6]))
            }
            (atype, _) => atype.map_or(Value::Null, Value::from),
        };
        let tbr = match int_or_none(asset.get("bitrate")) {
            Value::Number(number) if number.as_i64() == Some(0) => Value::Null,
            tbr => tbr,
        };
        let mut fields: Vec<(&str, Value)> = vec![
            ("format_id", format_id),
            ("url", aurl.into()),
            ("tbr", tbr),
            (
                "quality",
                if atype == Some("original") {
                    1.into()
                } else {
                    Value::Null
                },
            ),
        ];
        if display_name == Some("Audio") {
            fields.push(("vcodec", "none".into()));
        } else {
            fields.push(("width", int_or_none(asset.get("width"))));
            fields.push(("height", int_or_none(asset.get("height"))));
            fields.push((
                "vcodec",
                text_field(asset, "codec")?.map_or(Value::Null, Value::from),
            ));
        }
        let container = text_field(asset, "container")?;
        if container == Some("m3u8") || aext == "m3u8" {
            let Some(Value::String(format_id)) = fields.first().map(|(_, value)| value.clone())
            else {
                return Err(unsupported("HLS assets without a type"));
            };
            let mut ts_fields = fields.clone();
            ts_fields[0].1 = format_id.replace("hls-", "ts-").into();
            ts_fields[1].1 = aurl.replace(".bin", ".ts").into();
            ts_fields.push(("ext", "ts".into()));
            formats.push(asset_format(ts_fields, extractor, &video_id)?);
            fields.push(("ext", "mp4".into()));
            fields.push(("protocol", "m3u8_native".into()));
        } else {
            fields.push(("container", container.map_or(Value::Null, Value::from)));
            fields.push(("ext", aext.into()));
            fields.push(("filesize", int_or_none(asset.get("size"))));
        }
        formats.push(asset_format(fields, extractor, &video_id)?);
    }
    // `subtitles[language] = [...]`: a repeated language keeps its place.
    let mut captions: Vec<(String, Url)> = Vec::new();
    let caption_list = match data.get("captions") {
        None => &[][..],
        Some(Value::Array(list)) => list.as_slice(),
        Some(_) => return Err(unsupported("caption lists that are not arrays")),
    };
    for caption in caption_list {
        if !caption.is_object() {
            return Err(unsupported("captions that are not objects"));
        }
        let language = match caption.get("language") {
            Some(Value::String(language)) if !language.is_empty() => language.clone(),
            value if !super::lnk::truthy(value) => continue,
            _ => return Err(unsupported("caption languages that are not strings")),
        };
        let track = Url::parse(&format!(
            "{EMBED_BASE_URL}captions/{video_id}.vtt?language={language}"
        ))?;
        match captions
            .iter_mut()
            .find(|(existing, _)| *existing == language)
        {
            Some((_, url)) => *url = track,
            None => captions.push((language, track)),
        }
    }
    let subtitles = crate::Subtitles {
        entries: captions
            .into_iter()
            .map(|(language, url)| {
                let ext = super::direct::determine_ext(url.as_str())
                    .unwrap_or_else(|| "unknown_video".into());
                (
                    Some(language),
                    vec![crate::SubtitleFormat {
                        name: None,
                        url,
                        ext,
                        protocol: None,
                        fragments: Vec::new(),
                        extra: Default::default(),
                    }],
                )
            })
            .collect(),
    };
    let mut info = dict_info(
        page,
        (extractor, key),
        vec![
            ("id", video_id.into()),
            ("title", title.clone()),
            (
                "description",
                data.get("seoDescription").cloned().unwrap_or(Value::Null),
            ),
            ("formats", Value::Null),
            ("thumbnails", Value::Array(thumbnails)),
            ("duration", float_or_none(data.get("duration"))),
            ("timestamp", int_or_none(data.get("createdAt"))),
            ("subtitles", Value::Null),
        ],
        formats,
    );
    info.subtitles = subtitles;
    if let Some(Value::String(shape)) = info.extra.get(crate::info_order::INFO_SHAPE).cloned() {
        info.extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("{shape};thumbnail=url,width,height,filesize;subtitle=url").into(),
        );
    }
    Ok(info)
}

fn announce(extractor: &'static str, url: &Url) {
    crate::screen::set_extractor(extractor);
    crate::screen::screen(&format!(
        "[{extractor}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

impl Extractor for Wistia {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(video_id) = temp_id(url.as_str()) else {
            return Ok(None);
        };
        announce(NAME, url);
        let config = download_embed_config(client, (NAME, "medias"), &video_id, url.as_str())?;
        extract_media(client, url, &config, (NAME, KEY), &video_id).map(Some)
    }
}

impl Extractor for WistiaPlaylist {
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
        announce(PLAYLIST_NAME, url);
        let playlist = download_embed_config(
            client,
            (PLAYLIST_NAME, "playlists"),
            &playlist_id,
            url.as_str(),
        )?;
        // `try_get(playlist, lambda x: x[0]['medias']) or []`.
        let medias = match playlist.get(0).and_then(|first| first.get("medias")) {
            Some(Value::Array(medias)) => medias.as_slice(),
            value if !super::lnk::truthy(value) => &[][..],
            _ => return Err(unsupported("playlist media lists that are not arrays")),
        };
        let mut entries = Vec::new();
        for media in medias {
            if !media.is_object() {
                return Err(unsupported("playlist medias that are not objects"));
            }
            let Some(embed_config) = media
                .get("embed_config")
                .filter(|config| super::lnk::truthy(Some(config)))
            else {
                continue;
            };
            entries.push(extract_media(
                client,
                url,
                embed_config,
                (PLAYLIST_NAME, PLAYLIST_NAME),
                &playlist_id,
            )?);
        }
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: String::new(),
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

/// `WistiaBaseIE._extract_url_media_id`.
fn url_media_id(url: &str) -> Option<String> {
    static MEDIA_ID: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:wmediaid|wvideo(?:id)?)\]?=(?P<id>[a-z0-9]{10})").unwrap()
    });
    MEDIA_ID
        .captures(&unquote_plus(url))
        .map(|found| found["id"].to_owned())
}

/// `url_result(target, 'Wistia')`: followed by the downloader.
fn follow(url: &Url, target: &str) -> ExtractResult {
    let mut info = super::direct::build_at(url.clone(), url, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(crate::FOLLOW_URL.into(), target.into());
    ExtractResult::Video(info)
}

/// `url_result(f'wistia:{id}', WistiaIE, title=name)`.
fn media_entry(video_id: &str, title: Value) -> Result<VideoInfo> {
    let target = format!("wistia:{video_id}");
    let url = Url::parse(&target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = video_id.to_owned();
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": {"title": title, "ie_key": KEY, "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

/// Python `base64.b64decode` (non-validating) of the page's `[A-Za-z0-9=/]*`
/// data: binascii's error text, or `None` for padding this port does not
/// model.
fn b64decode(data: &str) -> Option<std::result::Result<Vec<u8>, String>> {
    use base64::Engine;
    let core = data.trim_end_matches('=');
    if core.contains('=') {
        return None;
    }
    let padding = data.len() - core.len();
    let needed = match core.len() % 4 {
        0 => 0,
        1 => {
            return Some(Err(format!(
                "Invalid base64-encoded string: number of data characters ({}) cannot be 1 more than a multiple of 4",
                core.len()
            )));
        }
        2 => 2,
        _ => 1,
    };
    if padding < needed {
        return Some(Err("Incorrect padding".into()));
    }
    if needed == 0 && padding > 0 {
        return None;
    }
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::STANDARD,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
            .with_decode_allow_trailing_bits(true),
    );
    engine.decode(core).ok().map(Ok)
}

/// The channel page's `wchanneljsonp` data (`_search_regex` defaulting to
/// the channel ID), base64-decoded, URL-unquoted JSON.
fn channel_page_data(client: &Client, channel_id: &str) -> Result<Value> {
    let page = Url::parse(&format!(
        "https://fast.wistia.net/embed/channel/{channel_id}"
    ))?;
    let webpage = super::download_webpage(client, &page, CHANNEL_NAME, channel_id)?;
    let pattern = Regex::new(&format!(
        r#"wchanneljsonp-{channel_id}'\]\s*=[^"]*"([A-Za-z0-9=/]*)"#
    ))
    .map_err(|_| unsupported("channel IDs that are not valid patterns"))?;
    let data = pattern
        .captures(&webpage)
        .map_or_else(|| channel_id.to_owned(), |found| found[1].to_owned());
    let decoded = match b64decode(&data) {
        Some(Ok(decoded)) => decoded,
        Some(Err(message)) => {
            return Err(Error::Reported(format!(
                "[{CHANNEL_NAME}] {channel_id}: Failed to parse JSON (caused by Error('{message}')); {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        None => return Err(unsupported("channel page data with unusual base64 padding")),
    };
    let text = String::from_utf8(decoded)
        .map_err(|_| unsupported("channel page data that is not UTF-8"))?;
    crate::json::from_str(&unquote_plus(&text))
        .map_err(|_| unsupported("channel page data that is not valid JSON"))
}

/// Python iteration of a `traverse_obj` branch: list items or dict values.
fn branch_items(value: Option<&Value>) -> Vec<&Value> {
    match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(map)) => map.values().collect(),
        _ => Vec::new(),
    }
}

impl Extractor for WistiaChannel {
    fn key(&self) -> &'static str {
        CHANNEL_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(channel_id) = channel_temp_id(url.as_str()) else {
            return Ok(None);
        };
        announce(CHANNEL_NAME, url);
        // `_yes_playlist(channel_id, media_id, playlist_label='channel')`.
        if let Some(media_id) = url_media_id(url.as_str()) {
            if super::no_playlist() {
                crate::screen::screen(&format!(
                    "[{CHANNEL_NAME}] Downloading just the video {media_id} because of --no-playlist"
                ));
                return Ok(Some(follow(url, &format!("wistia:{media_id}"))));
            }
            crate::screen::screen(&format!(
                "[{CHANNEL_NAME}] Downloading channel {channel_id} - add --no-playlist to download just the video {media_id}"
            ));
        }
        let data = match download_embed_config(
            client,
            (CHANNEL_NAME, "channel"),
            &channel_id,
            url.as_str(),
        ) {
            Ok(data) => data,
            // Some channels give a 403 from the JSON API.
            Err(_) => {
                crate::screen::warning(&format!(
                    "[{CHANNEL_NAME}] Failed to download channel data from API, falling back to webpage."
                ));
                channel_page_data(client, &channel_id)?
            }
        };
        let empty = Value::Object(Map::new());
        let series = data
            .get("series")
            .and_then(|series| series.get(0))
            .unwrap_or(&empty);
        if !series.is_object() {
            return Err(unsupported("channel series that are not objects"));
        }
        let mut entries = Vec::new();
        for section in branch_items(series.get("sections")) {
            for key in ["videos", "episodes"] {
                for video in branch_items(section.get(key)) {
                    let Some(hashed_id) = video
                        .get("hashedId")
                        .filter(|id| video.is_object() && super::lnk::truthy(Some(id)))
                    else {
                        continue;
                    };
                    let hashed_id = py_str(hashed_id)
                        .ok_or_else(|| unsupported("channel hashed IDs that are not strings"))?;
                    entries.push(media_entry(
                        &hashed_id,
                        video.get("name").cloned().unwrap_or(Value::Null),
                    )?);
                }
            }
        }
        // `playlist_result(entries, channel_id, playlist_title=...,
        // playlist_description=...)`.
        let mut keys = vec!["id"];
        let mut extra = Map::new();
        let title = match series.get("title") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(title)) => title.clone(),
            Some(_) => return Err(unsupported("channel titles that are not strings")),
        };
        if !title.is_empty() {
            keys.push("title");
        }
        if let Some(description) = series.get("description").filter(|value| !value.is_null()) {
            keys.push("description");
            extra.insert("description".into(), description.clone());
        }
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: channel_id,
            title,
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

/// `WistiaBaseIE._extract_from_webpage`'s TeachableIE check: a Teachable
/// course page belongs to TeachableIE (not ported), and Wistia reports no
/// embeds there.
fn teachable_page(url: &Url, webpage: &str) -> bool {
    static LINK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"<link[^>]+href=["']https?://(?:process\.fs|assets)\.teachablecdn\.com"#)
            .unwrap()
    });
    static COURSE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^https?://[^/]+/(?:courses|p)").unwrap());
    webpage.contains("teachableTracker.linker:autoLink")
        && LINK.is_match(webpage)
        && COURSE.is_match(url.as_str())
}

/// `_extract_wistia_async_embed`: `(type, id)` of each `wistia_async_ID`
/// class (the quote backreference spelled out per quote).
fn async_embeds(webpage: &str) -> Vec<(String, String)> {
    static ASYNC: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?s)<(?:div|section)[^>]+class=(?:"[^"]*?(?P<t1>wistia[a-z_0-9]+)\s*\bwistia_async_(?P<i1>[a-z0-9]{10})\b[^"]*"|'[^']*?(?P<t2>wistia[a-z_0-9]+)\s*\bwistia_async_(?P<i2>[a-z0-9]{10})\b[^']*')"#,
        )
        .unwrap()
    });
    ASYNC
        .captures_iter(webpage)
        .filter_map(|found| {
            let kind = found.name("t1").or_else(|| found.name("t2"))?;
            let id = found.name("i1").or_else(|| found.name("i2"))?;
            Some((kind.as_str().to_owned(), id.as_str().to_owned()))
        })
        .collect()
}

fn push_unique(found: &mut Vec<Url>, url: Url) {
    if !found.contains(&url) {
        found.push(url);
    }
}

/// `WistiaIE._extract_embed_urls` under `orderedSet`: `_EMBED_REGEX`
/// iframe/script/meta URLs, non-channel async embeds, then `data-wistia-id`,
/// `Wistia.embed(...)` and `id="wistia_..."` IDs. (Upstream's media-ID
/// fallback never runs: it tests a generator's truthiness.)
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    static EMBED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"<(?:meta[^>]+?content|(?:iframe|script)[^>]+?src)=["'](?P<url>(?:https?:)?//(?:fast\.)?wistia\.(?:net|com)/embed/(?:iframe|medias)/[a-z0-9]{10})"#,
        )
        .unwrap()
    });
    static IDS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"(?:data-wistia-?id=["']|Wistia\.embed\(["']|id=["']wistia_)(?P<id>[a-z0-9]{10})"#,
        )
        .unwrap()
    });
    let mut found = Vec::new();
    if teachable_page(url, webpage) {
        return found;
    }
    for captures in EMBED.captures_iter(webpage) {
        let Ok(embed) = url.join(&crate::html::unescape_html(&captures["url"])) else {
            continue;
        };
        if suitable(embed.as_str()) {
            push_unique(&mut found, embed);
        }
    }
    let ids = async_embeds(webpage)
        .into_iter()
        .filter(|(kind, _)| kind != "wistia_channel")
        .map(|(_, id)| id)
        .chain(
            IDS.captures_iter(webpage)
                .map(|found| found["id"].to_owned()),
        );
    for id in ids {
        if let Ok(embed) = Url::parse(&format!("wistia:{id}")) {
            push_unique(&mut found, embed);
        }
    }
    found
}

/// `WistiaChannelIE._extract_embed_urls` under `orderedSet`: channel async
/// embeds as `wistiachannel:ID` with the page URL's query
/// (`update_url_query(..., parse_qs(url))`).
pub(crate) fn channel_embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found = Vec::new();
    if teachable_page(url, webpage) {
        return found;
    }
    // `parse_qs` (blank values dropped, repeated keys grouped), re-encoded.
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (key, value) in url.query_pairs() {
        if value.is_empty() {
            continue;
        }
        match groups.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, values)) => values.push(value.into_owned()),
            None => groups.push((key.into_owned(), vec![value.into_owned()])),
        }
    }
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(
            groups
                .iter()
                .flat_map(|(key, values)| values.iter().map(move |value| (key, value))),
        )
        .finish();
    for (kind, id) in async_embeds(webpage) {
        if kind != "wistia_channel" {
            continue;
        }
        let Ok(mut embed) = Url::parse(&format!("wistiachannel:{id}")) else {
            continue;
        };
        embed.set_query((!query.is_empty()).then_some(query.as_str()));
        push_unique(&mut found, embed);
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("wistia:sh7fpupwlt"));
        assert!(suitable("http://fast.wistia.net/embed/iframe/sh7fpupwlt"));
        assert!(suitable("http://fast.wistia.com/embed/iframe/sh7fpupwlt"));
        assert!(suitable(
            "http://fast.wistia.net/embed/medias/sh7fpupwlt.json"
        ));
        assert!(!suitable(
            "https://fast.wistia.net/embed/playlists/aodt9etokc"
        ));
        assert!(playlist_suitable(
            "https://fast.wistia.net/embed/playlists/aodt9etokc"
        ));
        assert!(channel_suitable(
            "https://fast.wistia.net/embed/channel/3802iirk0l"
        ));
        assert!(channel_suitable(
            "wistiachannel:3802iirk0l?wmediaid=sp5dqjzw3n"
        ));
        assert!(!channel_suitable(
            "https://fast.wistia.net/embed/iframe/sh7fpupwlt"
        ));
    }

    #[test]
    fn media_ids_come_from_unquoted_queries() {
        assert_eq!(
            url_media_id(
                "https://amplitude.com/amplify-sessions?amp%5Bwmediaid%5D=pz0m0l0if3&amp%5Bwvideo%5D=pz0m0l0if3&wchannelid=emyjmwjf79&wmediaid=i8um783bdt"
            )
            .as_deref(),
            Some("pz0m0l0if3")
        );
        assert_eq!(
            url_media_id("https://fast.wistia.net/embed/channel/x"),
            None
        );
    }

    #[test]
    fn base64_errors_follow_binascii() {
        assert_eq!(b64decode("QQ=="), Some(Ok(b"A".to_vec())));
        assert_eq!(
            b64decode("errchannel"),
            Some(Err("Incorrect padding".into()))
        );
        assert!(matches!(b64decode("Q"), Some(Err(message)) if message.contains("(1)")));
        assert_eq!(b64decode("Q=Q="), None);
    }

    #[test]
    fn teachable_course_pages_have_no_wistia_embeds() {
        let page = "<link href=\"https://assets.teachablecdn.com/a.css\"><script>teachableTracker.linker:autoLink</script><div id=\"wistia_abcdefghij\"></div>";
        let course = Url::parse("https://school.example/courses/x/lectures/1").unwrap();
        assert!(embed_urls(&course, page).is_empty());
        let blog = Url::parse("https://school.example/blog").unwrap();
        assert_eq!(embed_urls(&blog, page).len(), 1);
    }
}
