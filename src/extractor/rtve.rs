//! Upstream `RTVEALaCartaIE`, `RTVEAudioIE`, `RTVELiveIE` and
//! `RTVETelevisionIE` (`yt_dlp/extractor/rtve.py`): RTVE Play videos and
//! audios through the API metadata and `RTVEBaseIE`'s PNG-encoded media URLs
//! (`_decrypt_url`: each `tEXt` chunk's alphabet and digit-encoded URL), HLS,
//! DASH and progressive formats ranked by quality, subtitles, live channel
//! pages and television pages' `contentUrl`.
//!
//! Not ported: `RTVEProgramIE` (program pages).
use super::{
    Extractor, dict_format, dict_info, download_json, download_json_note, download_webpage,
    extract_m3u8_master_optional, extract_mpd_optional_note,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, SubtitleFormat, Subtitles, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct RtveALaCarta;
pub struct RtveAudio;
pub struct RtveLive;
pub struct RtveTelevision;
pub struct RtveProgram;

const PROGRAM_NAME: &str = "rtve.es:program";
const PROGRAM_KEY: &str = "RTVEProgram";
/// `RTVEProgramIE._PAGE_SIZE`.
const PROGRAM_PAGE_SIZE: usize = 60;
static PROGRAM_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?rtve\.es/play/videos/(?P<id>[\w-]+)/?(?:[?#]|$)",
    )
    .unwrap()
});

const ALACARTA_NAME: &str = "rtve.es:alacarta";
const ALACARTA_KEY: &str = "RTVEALaCarta";
const AUDIO_NAME: &str = "rtve.es:audio";
const AUDIO_KEY: &str = "RTVEAudio";
const LIVE_NAME: &str = "rtve.es:live";
const LIVE_KEY: &str = "RTVELive";
const TELEVISION_NAME: &str = "rtve.es:television";
const TELEVISION_KEY: &str = "RTVETelevision";

static ALACARTA_URL: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"^https?://(?:www\.)?rtve\.es/(?:m/)?(?:(?:alacarta|play)/videos|filmoteca)/(?:[^/?#]+/){2}(?P<id>\d+)").unwrap(),
        Regex::new(r"^https?://(?:www\.)?rtve\.es/infantil/serie/[^/?#]+/video/[^/?#]+/(?P<id>\d+)").unwrap(),
    ]
});
static AUDIO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?rtve\.es/(alacarta|play)/audios/(?:[^/?#]+/){2}(?P<id>\d+)")
        .unwrap()
});
static LIVE_URL: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r"^https?://(?:www\.)?rtve\.es/directo/(?P<id>[a-zA-Z0-9-]+)").unwrap(),
        Regex::new(
            r"^https?://(?:www\.)?rtve\.es/play/videos/directo/[^/?#]+/(?P<id>[a-zA-Z0-9-]+)",
        )
        .unwrap(),
    ]
});
static TELEVISION_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?rtve\.es/television/[^/?#]+/[^/?#]+/(?P<id>\d+).shtml")
        .unwrap()
});
static DATA_SETUP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<div[^>]+class="[^"]*videoPlayer[^"]*"[^>]*data-setup='"#).unwrap()
});

/// `RTVEALaCartaIE._match_valid_url`: its `(?!directo)` look-ahead as a
/// separate check (the segment after `videos/` is not `directo...`).
fn alacarta_id(url: &str) -> Option<String> {
    static DIRECTO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"^https?://(?:www\.)?rtve\.es/(?:m/)?(?:(?:alacarta|play)/videos|filmoteca)/directo",
        )
        .unwrap()
    });
    if DIRECTO.is_match(url) {
        return ALACARTA_URL[1]
            .captures(url)
            .map(|found| found["id"].to_owned());
    }
    ALACARTA_URL
        .iter()
        .find_map(|pattern| pattern.captures(url))
        .map(|found| found["id"].to_owned())
}

/// `RTVEALaCartaIE._match_valid_url`.
pub fn alacarta_suitable(url: &str) -> bool {
    alacarta_id(url).is_some()
}

/// `RTVEAudioIE._match_valid_url`.
pub fn audio_suitable(url: &str) -> bool {
    AUDIO_URL.is_match(url)
}

/// `RTVELiveIE._match_valid_url`.
pub fn live_suitable(url: &str) -> bool {
    LIVE_URL.iter().any(|pattern| pattern.is_match(url))
}

/// `RTVETelevisionIE._match_valid_url`.
pub fn television_suitable(url: &str) -> bool {
    TELEVISION_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("RTVE {what} are not supported natively"))
}

/// A `KeyError` reported as `ExtractorError(video_id=...)`.
fn lookup_error(id: &str, cause: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by {cause}); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `RTVEBaseIE._get_alphabet(alphabet_data)`.
fn alphabet(data: &[u8]) -> Vec<char> {
    let (mut alphabet, mut e, mut d) = (Vec::new(), 0, 0);
    for &byte in data {
        if d == 0 {
            alphabet.push(char::from(byte));
            e = (e + 1) % 4;
            d = e;
        } else {
            d -= 1;
        }
    }
    alphabet
}

/// `RTVEBaseIE._get_url(alphabet, url_data)`.
fn decode_url(alphabet: &[char], data: &[u8]) -> Result<String> {
    let digit = |byte: u8| {
        char::from(byte)
            .to_digit(10)
            .map(|digit| digit as usize)
            .ok_or_else(|| unsupported("media URL data that are not digits"))
    };
    let (mut url, mut f, mut e, mut b, mut l) = (String::new(), 0, 3, 1, 0);
    for &byte in data {
        if f == 0 {
            l = digit(byte)? * 10;
            f = 1;
        } else if e == 0 {
            l += digit(byte)?;
            url.push(
                *alphabet
                    .get(l)
                    .ok_or_else(|| unsupported("media URL indices outside the alphabet"))?,
            );
            e = (b + 3) % 4;
            f = 0;
            b += 1;
        } else {
            e -= 1;
        }
    }
    Ok(url)
}

/// `RTVEBaseIE._decrypt_url(png)`: each `tEXt` chunk's quality and URL.
fn decrypt_url(png: &str) -> Result<Vec<(String, String)>> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(png.trim())
        .map_err(|_| unsupported("media URL images that are not base64"))?;
    let truncated = || unsupported("truncated media URL images");
    let mut data = bytes.get(8..).ok_or_else(truncated)?;
    let mut found = Vec::new();
    loop {
        let length = data.get(..4).ok_or_else(truncated)?;
        let length = u32::from_be_bytes(length.try_into().expect("four bytes")) as usize;
        let kind = data.get(4..8).ok_or_else(truncated)?;
        if kind == b"IEND" {
            break;
        }
        // Like `BytesIO.read`, a short chunk reads what is left.
        let end = (8 + length).min(data.len());
        let chunk = &data[8..end];
        if kind == b"tEXt" {
            let text: Vec<u8> = chunk.iter().copied().filter(|byte| *byte != 0).collect();
            let (alphabet_data, url_data) = match text.iter().position(|byte| *byte == b'#') {
                Some(index) => (&text[..index], &text[index + 1..]),
                None => (&text[..], &[][..]),
            };
            let split = url_data.windows(2).rposition(|pair| pair == b"%%");
            let (quality, url_data) = match split {
                Some(index) => (&url_data[..index], &url_data[index + 2..]),
                None => (&[][..], url_data),
            };
            let quality = String::from_utf8(quality.to_vec())
                .map_err(|_| unsupported("media qualities that are not UTF-8"))?;
            found.push((quality, decode_url(&alphabet(alphabet_data), url_data)?));
        }
        data = data.get((end + 4).min(data.len())..).unwrap_or_default();
    }
    Ok(found)
}

/// `qualities(['Media', 'Alta', 'HQ', 'HD_READY', 'HD_FULL'])`.
fn quality(quality: &str) -> i64 {
    ["Media", "Alta", "HQ", "HD_READY", "HD_FULL"]
        .iter()
        .position(|known| *known == quality)
        .map_or(-1, |index| index as i64)
}

/// `_extract_png_formats_and_subtitles(video_id, media_type)`.
fn png_formats(
    client: &Client,
    name: &'static str,
    id: &str,
    media_type: &str,
) -> Result<(Vec<crate::VideoFormat>, Subtitles)> {
    let mut formats = Vec::new();
    let mut subtitles = Subtitles::default();
    for manager in ["rtveplayw", "default"] {
        let url = Url::parse(&format!(
            "http://www.rtve.es/ztnr/movil/thumbnail/{manager}/{media_type}/{id}.png?q=v2"
        ))?;
        // `_download_webpage(..., 'Downloading url information',
        // fatal=False)`.
        let png = match super::download_webpage_note(
            client,
            &url,
            name,
            id,
            "Downloading url information",
        ) {
            Ok(png) => png,
            Err(Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{name}] {id}: Unable to download webpage: {}",
                    crate::upstream_error_text(&source)
                ));
                continue;
            }
            Err(error) => return Err(error),
        };
        if png.is_empty() {
            continue;
        }
        for (format_quality, video_url) in decrypt_url(&png)? {
            match super::direct::determine_ext(&video_url).as_deref() {
                Some("m3u8") => {
                    let manifest = Url::parse(&video_url)?;
                    if let Some(stream) = extract_m3u8_master_optional(
                        client,
                        &manifest,
                        name,
                        id,
                        Some("hls"),
                        true,
                        false,
                    ) {
                        formats.extend(stream.formats);
                        super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                    }
                }
                Some("mpd") => {
                    let manifest = Url::parse(&video_url)?;
                    if let Some(stream) = extract_mpd_optional_note(
                        client,
                        &manifest,
                        name,
                        id,
                        Some("dash"),
                        "Downloading MPD manifest",
                    ) {
                        formats.extend(stream.formats);
                        super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                    }
                }
                _ => formats.push(dict_format(
                    vec![
                        ("format_id", format_quality.as_str().into()),
                        ("quality", quality(&format_quality).into()),
                        ("url", video_url.into()),
                    ],
                    name,
                    id,
                )?),
            }
        }
    }
    Ok((formats, subtitles))
}

fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// `_parse_metadata(metadata)`: the present fields in template order.
fn parse_metadata(metadata: &Value) -> Vec<(&'static str, Value)> {
    let text = |key: &str| metadata.get(key).and_then(Value::as_str);
    let date = |key: &str| {
        text(key)
            .and_then(crate::dates::parse_iso8601_space)
            .map(Value::from)
    };
    let thumbnail = ["thumbnail", "image", "imageSEO"]
        .iter()
        .find_map(|key| url_or_none(metadata.get(*key)))
        .map(Value::from);
    let duration = match metadata.get("duration") {
        Some(Value::Number(number)) => number.as_f64(),
        Some(Value::String(text)) => text.trim().parse::<f64>().ok(),
        _ => None,
    }
    .map(|duration| json!(duration / 1000.0));
    let series = [
        metadata.get("programTitle"),
        metadata
            .get("programInfo")
            .and_then(|info| info.get("title")),
    ]
    .into_iter()
    .flatten()
    .find_map(Value::as_str)
    .map(|series| Value::from(crate::html::clean_html(series)));
    [
        (
            "title",
            text("title").map(|title| Value::from(title.trim())),
        ),
        ("alt_title", text("alt").map(|alt| Value::from(alt.trim()))),
        (
            "description",
            text("description").map(|text| Value::from(crate::html::clean_html(text))),
        ),
        ("timestamp", date("dateOfEmission")),
        ("release_timestamp", date("publicationDate")),
        ("modified_timestamp", date("modificationDate")),
        ("thumbnail", thumbnail),
        ("duration", duration),
        (
            "is_live",
            metadata
                .get("live")
                .filter(|live| live.is_boolean())
                .cloned(),
        ),
        ("series", series),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|value| (key, value)))
    .collect()
}

/// `api_json['page']['items'][0]`.
fn first_item(api: &Value, id: &str) -> Result<Value> {
    let page = api
        .get("page")
        .ok_or_else(|| lookup_error(id, "KeyError('page')"))?;
    let items = page
        .get("items")
        .ok_or_else(|| lookup_error(id, "KeyError('items')"))?;
    match items {
        Value::Array(items) => items
            .first()
            .cloned()
            // An `IndexError` is not `extract`'s `ExtractorError`.
            .ok_or_else(|| Error::Reported("list index out of range".into())),
        _ => Err(unsupported("item lists that are not lists")),
    }
}

/// `traverse_obj(data, ('page', 'items', ..., {'id': ('lang', {str}),
/// 'url': ('src', {url_or_none})}, all, {subs_list_to_dict(lang='es')}))`.
fn api_subtitles(data: &Value) -> Result<Subtitles> {
    let mut subtitles = Subtitles::default();
    let items = data
        .get("page")
        .and_then(|page| page.get("items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for item in items {
        let Some(url) = url_or_none(item.get("src")) else {
            continue;
        };
        let language = item
            .get("lang")
            .and_then(Value::as_str)
            .unwrap_or("es")
            .to_owned();
        let parsed =
            Url::parse(&url).map_err(|_| unsupported("subtitle URLs that are not absolute"))?;
        let ext = super::direct::determine_ext(&url)
            .unwrap_or_else(|| "unknown_video".into())
            .to_lowercase();
        let mut track = Subtitles::default();
        track.entries.push((
            Some(language),
            vec![SubtitleFormat {
                name: None,
                url: parsed,
                ext,
                protocol: None,
                fragments: Vec::new(),
                extra: Map::new(),
            }],
        ));
        super::svt::merge_subtitles(&mut subtitles, track);
    }
    Ok(subtitles)
}

impl Extractor for RtveALaCarta {
    fn key(&self) -> &'static str {
        ALACARTA_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(id) = alacarta_id(url.as_str()) else {
            return Ok(None);
        };
        announce(ALACARTA_NAME, url);
        let api = Url::parse(&format!(
            "http://www.rtve.es/api/videos/{id}/config/alacarta_videos.json"
        ))?;
        let metadata = first_item(&download_json(client, &api, ALACARTA_NAME, &id)?, &id)?;
        match metadata.get("state") {
            None => return Err(lookup_error(&id, "KeyError('state')")),
            Some(Value::String(state)) if state == "DESPU" => {
                return Err(Error::Reported(format!(
                    "[{ALACARTA_NAME}] {id}: The video is no longer available"
                )));
            }
            Some(_) => {}
        }
        let (formats, mut subtitles) = png_formats(client, ALACARTA_NAME, &id, "videos")?;
        // `extract_subtitles(video_id)`.
        if super::subtitles_requested() {
            let api = Url::parse(&format!(
                "https://api2.rtve.es/api/videos/{id}/subtitulos.json"
            ))?;
            let data = download_json_note(
                client,
                &api,
                ALACARTA_NAME,
                &id,
                "Downloading subtitles info",
            )?;
            super::svt::merge_subtitles(&mut subtitles, api_subtitles(&data)?);
        }
        let infantil = url.path().starts_with("/infantil/");
        let mut fields = vec![
            ("id", Value::from(id.as_str())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        fields.extend(parse_metadata(&metadata));
        fields.push((
            "_old_archive_ids",
            if infantil {
                json!([format!("rtveinfantil {id}")])
            } else {
                Value::Null
            },
        ));
        let mut info = dict_info(url, (ALACARTA_NAME, ALACARTA_KEY), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

impl Extractor for RtveAudio {
    fn key(&self) -> &'static str {
        AUDIO_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = AUDIO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(AUDIO_NAME, url);
        let api = Url::parse(&format!("https://www.rtve.es/api/audios/{id}.json"))?;
        let metadata = first_item(&download_json(client, &api, AUDIO_NAME, &id)?, &id)?;
        let (formats, subtitles) = png_formats(client, AUDIO_NAME, &id, "audios")?;
        let mut fields = vec![
            ("id", Value::from(id.as_str())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        fields.extend(parse_metadata(&metadata));
        let mut info = dict_info(url, (AUDIO_NAME, AUDIO_KEY), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

impl Extractor for RtveLive {
    fn key(&self) -> &'static str {
        LIVE_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = LIVE_URL
            .iter()
            .find_map(|pattern| pattern.captures(url.as_str()))
        else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(LIVE_NAME, url);
        let webpage = download_webpage(client, url, LIVE_NAME, &id)?;
        let setup = match super::search_json(&DATA_SETUP, &webpage, false) {
            Some(Some(setup)) => setup,
            Some(None) => {
                return Err(Error::Reported(format!(
                    "[{LIVE_NAME}] {id}: data_setup: Failed to parse JSON"
                )));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{LIVE_NAME}] {id}: Unable to extract data_setup; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let asset = match setup.get("idAsset") {
            None => return Err(lookup_error(&id, "KeyError('idAsset')")),
            Some(Value::String(asset)) => asset.clone(),
            Some(Value::Number(asset)) => asset.to_string(),
            Some(_) => return Err(unsupported("asset IDs that are not scalars")),
        };
        let (formats, subtitles) = png_formats(client, LIVE_NAME, &asset, "videos")?;
        // `{'id': ..., **_search_json_ld(webpage, video_id, fatal=False),
        // 'title': ..., 'formats': ..., 'subtitles': ..., 'is_live': True}`.
        let json_ld = super::json_ld::search(&webpage);
        if json_ld.is_empty() {
            crate::screen::warning(&format!(
                "[{LIVE_NAME}] unable to extract JSON-LD ; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        if ["url", "formats", "subtitles"]
            .iter()
            .any(|key| json_ld.contains_key(*key))
        {
            return Err(unsupported("JSON-LD media URLs"));
        }
        let mut fields: Vec<(String, Value)> = vec![("id".into(), Value::from(id.as_str()))];
        for (key, value) in json_ld {
            match fields.iter_mut().find(|(known, _)| *known == key) {
                Some(field) => field.1 = value,
                None => fields.push((key, value)),
            }
        }
        let title = crate::html::html_extract_title(&webpage).map_or(Value::Null, Value::from);
        for (key, value) in [
            ("title", title),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("is_live", Value::Bool(true)),
        ] {
            match fields.iter_mut().find(|(known, _)| known == key) {
                Some(field) => field.1 = value,
                None => fields.push((key.into(), value)),
            }
        }
        let fields = fields
            .iter()
            .map(|(key, value)| (key.as_str(), value.clone()))
            .collect();
        let mut info = dict_info(url, (LIVE_NAME, LIVE_KEY), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

impl Extractor for RtveTelevision {
    fn key(&self) -> &'static str {
        TELEVISION_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = TELEVISION_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        announce(TELEVISION_NAME, url);
        let webpage = download_webpage(client, url, TELEVISION_NAME, &id)?;
        let Some(play_url) = crate::html::html_search_meta(&webpage, "contentUrl") else {
            // `_html_search_meta(...)` (`fatal=False`) warns first.
            super::search_warning(TELEVISION_NAME, "contentUrl");
            return Err(Error::Reported(format!(
                "[{TELEVISION_NAME}] {id}: The webpage doesn't contain any video"
            )));
        };
        // `url_result(play_url, ie=RTVEALaCartaIE.ie_key())`.
        if !alacarta_suitable(&play_url) {
            return Err(unsupported(
                "television pages linking to other than a la carta videos",
            ));
        }
        let target = Url::parse(&play_url)?;
        let mut info = super::direct::build_at(target.clone(), &target, String::new());
        info.formats.clear();
        info.direct = None;
        info.extra
            .insert(crate::FOLLOW_URL.into(), target.as_str().into());
        Ok(Some(ExtractResult::Video(info)))
    }
}

/// `RTVEProgramIE._VALID_URL`.
pub fn program_suitable(url: &str) -> bool {
    PROGRAM_URL.is_match(url)
}

/// `_fetch_page(program_id, page_num)`: one page of the program's videos API.
fn program_page(client: &Client, program_id: &str, page_num: usize) -> Result<Value> {
    let api = Url::parse_with_params(
        &format!("https://www.rtve.es/api/programas/{program_id}/videos"),
        &[
            ("type", "39816".to_owned()),
            ("page", page_num.to_string()),
            ("size", PROGRAM_PAGE_SIZE.to_string()),
        ],
    )?;
    download_json_note(
        client,
        &api,
        PROGRAM_NAME,
        program_id,
        &format!("Downloading page {page_num}"),
    )
}

/// `int_or_none` of a JSON value.
fn int_of(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|float| float.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `str` of a JSON scalar (`None` for a missing value).
fn str_of(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::String(text) => Some(Value::from(text.clone())),
        Value::Number(number) => Some(Value::from(number.to_string())),
        _ => None,
    }
}

/// `url_result(htmlUrl, RTVEALaCartaIE, url_transparent=True, **fields)`
/// as a playlist entry (its keys in upstream's order).
fn program_entry(target: &str, fields: Vec<(&str, Value)>) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    for (key, value) in fields {
        if !value.is_null() {
            entry.insert(key.into(), value);
        }
    }
    entry.insert("ie_key".into(), ALACARTA_KEY.into());
    entry.insert("_type".into(), "url_transparent".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": ALACARTA_KEY,
            "extractor": ALACARTA_NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for RtveProgram {
    fn key(&self) -> &'static str {
        PROGRAM_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PROGRAM_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let program_slug = found["id"].to_owned();
        crate::screen::set_extractor(PROGRAM_NAME);
        crate::screen::screen(&format!(
            "[{PROGRAM_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let program_page_html = download_webpage(client, url, PROGRAM_NAME, &program_slug)?;
        let Some(program_id) = crate::html::html_search_meta(&program_page_html, "DC.identifier")
        else {
            return Err(Error::Reported(format!(
                "[{PROGRAM_NAME}] {program_slug}: Unable to extract Program ID; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };

        let first_page = program_page(client, &program_id, 1)?;
        let page_count = first_page
            .pointer("/page/totalPages")
            .and_then(|value| int_of(Some(value)))
            .filter(|count| *count != 0)
            .unwrap_or(1);
        let mut entries = Vec::new();
        for page_num in 1..=page_count.max(0) as usize {
            let data = if page_num == 1 {
                first_page.clone()
            } else {
                program_page(client, &program_id, page_num)?
            };
            let items = data
                .pointer("/page/items")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for video in items {
                let Some(html_url) = video
                    .get("htmlUrl")
                    .and_then(Value::as_str)
                    .filter(|link| link.starts_with("http://") || link.starts_with("https://"))
                else {
                    continue;
                };
                let text = |key: &str| {
                    video
                        .get(key)
                        .and_then(Value::as_str)
                        .map(|t| Value::from(t.to_owned()))
                };
                let duration = video
                    .get("duration")
                    .and_then(Value::as_f64)
                    .map_or(Value::Null, |ms| Value::from(ms / 1000.0));
                let thumbnail = video
                    .get("thumbnail")
                    .and_then(Value::as_str)
                    .filter(|link| link.starts_with("http://") || link.starts_with("https://"))
                    .map_or(Value::Null, Value::from);
                // `(('programInfo', 'title'), {str}, any)`: a non-string
                // `programInfo` is skipped for the episode's own `title`.
                let series = [
                    video.get("programInfo").and_then(Value::as_str),
                    video.get("title").and_then(Value::as_str),
                ]
                .into_iter()
                .flatten()
                .next()
                .map(|text| Value::from(text.to_owned()));
                let fields = vec![
                    ("id", str_of(video.get("id")).unwrap_or(Value::Null)),
                    ("title", text("longTitle").unwrap_or(Value::Null)),
                    (
                        "description",
                        text("shortDescription").unwrap_or(Value::Null),
                    ),
                    ("duration", duration),
                    ("series", series.unwrap_or(Value::Null)),
                    (
                        "season_number",
                        int_of(video.get("temporadaOrden")).map_or(Value::Null, Value::from),
                    ),
                    (
                        "season_id",
                        str_of(video.get("temporadaId")).unwrap_or(Value::Null),
                    ),
                    (
                        "season",
                        str_of(video.get("temporada")).unwrap_or(Value::Null),
                    ),
                    (
                        "episode_number",
                        int_of(video.get("episode")).map_or(Value::Null, Value::from),
                    ),
                    ("episode", text("title").unwrap_or(Value::Null)),
                    ("thumbnail", thumbnail),
                ];
                entries.push(program_entry(html_url, fields)?);
            }
        }

        let title = crate::html::html_extract_title(&program_page_html);
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: program_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: PROGRAM_NAME.into(),
            extractor_key: PROGRAM_KEY.into(),
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
        assert_eq!(
            alacarta_id("http://www.rtve.es/alacarta/videos/la-aventura-del-saber/aventuraentornosilla/3088905/").as_deref(),
            Some("3088905")
        );
        assert!(alacarta_suitable(
            "https://www.rtve.es/infantil/serie/dino-ranch/video/pequeno-gran-ayudante/6868808/"
        ));
        assert!(!alacarta_suitable(
            "https://www.rtve.es/play/videos/directo/canales-lineales/la-1/"
        ));
        assert!(live_suitable(
            "https://www.rtve.es/play/videos/directo/canales-lineales/la-1/"
        ));
        assert!(live_suitable("http://www.rtve.es/directo/la-1/"));
        assert!(audio_suitable(
            "https://www.rtve.es/play/audios/crims/crims-ca-mercadal/6766574/"
        ));
        assert!(television_suitable(
            "https://www.rtve.es/television/20091103/video-inedito-del-8o-programa/299020.shtml"
        ));
    }

    #[test]
    fn png_urls_decode_like_upstream() {
        // Encoded by the inverse of `_decrypt_url` (checked with upstream).
        let alphabet_data = b"ax";
        assert_eq!(alphabet(alphabet_data), ['a']);
        assert_eq!(decode_url(&['a', 'b'], b"07770").unwrap(), "a");
    }

    #[test]
    fn program_urls_follow_upstream_valid_url() {
        assert!(program_suitable(
            "https://www.rtve.es/play/videos/saber-vivir/"
        ));
        assert!(!program_suitable(
            "https://www.rtve.es/play/videos/saber-vivir/25-01-26/16909076/"
        ));
    }
}
