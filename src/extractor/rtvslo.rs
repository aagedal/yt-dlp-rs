//! Upstream `RTVSLOIE` and `RTVSLOShowIE` (`yt_dlp/extractor/rtvslo.py`):
//! RTV Slovenija 365 recordings from the `ava` API (`getRecordingDrm`
//! metadata, then `getMedia` with its JWT): the Wowza HLS master playlists
//! and `rtmp:` formats, sign-language variants, progressive media files,
//! upstream's geo and `Clip not available` errors, and shows as a playlist
//! of their `/arhiv/` links.
use super::{
    Extractor, dict_format, dict_info, download_json, download_webpage, extract_wowza_formats,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, SubtitleFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Rtvslo;
pub struct RtvsloShow;

const NAME: &str = "rtvslo.si";
const KEY: &str = "RTVSLO";
const SHOW_NAME: &str = "rtvslo.si:show";
const SHOW_KEY: &str = "RTVSLOShow";
const GEO_COUNTRIES: &[&str] = &["SI"];
const GEO_MESSAGE: &str = "This video is not available from your location due to geo restriction";
const SKIP_PROTOCOLS: &[&str] = &["smil", "f4m", "dash"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:365|4d)\.rtvslo.si/arhiv/[^/?#&;]+|(?:www\.)?rtvslo\.si/rtv365/arhiv)/(?P<id>\d+)",
    )
    .unwrap()
});
static SHOW_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:365|4d)\.rtvslo.si/oddaja/[^/?#&]+/(?P<id>\d+)").unwrap()
});
static ARCHIVE_LINK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<a [^>]*\bhref="(/arhiv/[^"]+)""#).unwrap());

/// `RTVSLOIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `RTVSLOShowIE._match_id`.
pub fn show_suitable(url: &str) -> bool {
    SHOW_URL.is_match(url)
}

fn start(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("RTV SLO {what} are not supported natively"))
}

/// `_API_BASE.format(method, id)`.
fn api(method: &str, video_id: &str, jwt: Option<&str>) -> Result<Url> {
    let mut url = Url::parse(&format!(
        "https://api.rtvslo.si/ava/{method}/{video_id}?client_id=82013fb3a531d5414f478747c1aca622"
    ))?;
    if let Some(jwt) = jwt {
        url.query_pairs_mut().append_pair("jwt", jwt);
    }
    Ok(url)
}

/// `int_or_none(value)`.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

/// Python truthiness.
fn truthy(value: &Value) -> bool {
    super::nuxt::truthy(value)
}

/// `traverse_obj(media, (KEY, 'hls_sec'), expected_type=url_or_none)`.
fn hls_sec(value: Option<&Value>) -> Option<String> {
    value
        .and_then(|value| value.get("hls_sec"))
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// `traverse_obj(meta, ('canonical', ('domain', 'path')))`, joined.
fn canonical(meta: &Value) -> Result<String> {
    let mut text = String::new();
    for key in ["domain", "path"] {
        match meta
            .get("canonical")
            .and_then(|canonical| canonical.get(key))
        {
            None | Some(Value::Null) => {}
            Some(Value::String(part)) => text.push_str(part),
            Some(_) => return Err(unsupported("canonical URL parts that are not strings")),
        }
    }
    Ok(text)
}

/// The `subs` (else `subtitles`) entries by language.
fn subtitles(meta: &Value) -> Result<crate::Subtitles> {
    let mut subtitles = crate::Subtitles::default();
    let list = match meta.get("subs").filter(|value| !value.is_null()) {
        Some(list) => list,
        None => match meta.get("subtitles").filter(|value| !value.is_null()) {
            Some(list) => list,
            None => return Ok(subtitles),
        },
    };
    let Value::Array(list) = list else {
        return Err(unsupported("subtitle lists that are not lists"));
    };
    for item in list {
        let language = match item.get("language") {
            Some(Value::String(language)) if language == "Slovenski" => "sl".to_owned(),
            Some(Value::String(language)) if !language.is_empty() => language.clone(),
            Some(value) if truthy(value) => {
                return Err(unsupported("subtitle languages that are not strings"));
            }
            _ => "und".to_owned(),
        };
        let url = item
            .get("file")
            .and_then(Value::as_str)
            .and_then(|file| Url::parse(file).ok())
            .ok_or_else(|| unsupported("subtitles without a file URL"))?;
        let ext = item
            .get("format")
            .and_then(Value::as_str)
            .map(str::to_lowercase)
            .ok_or_else(|| unsupported("subtitles without a format"))?;
        let track = SubtitleFormat {
            name: None,
            url,
            ext,
            protocol: None,
            fragments: Vec::new(),
            extra: Map::new(),
        };
        match subtitles
            .entries
            .iter_mut()
            .find(|(lang, _)| lang.as_deref() == Some(language.as_str()))
        {
            Some((_, tracks)) => tracks.push(track),
            None => subtitles.entries.push((Some(language), vec![track])),
        }
    }
    Ok(subtitles)
}

/// The `mediaFiles` entries whose `streams.https` is a URL, as
/// `traverse_obj` dicts.
fn media_file_formats(media: &Value, video_id: &str) -> Result<Vec<crate::VideoFormat>> {
    let mut formats = Vec::new();
    for file in media
        .get("mediaFiles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(url) = file
            .pointer("/streams/https")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        else {
            continue;
        };
        let mut fields = vec![("url", Value::from(url))];
        if let Some(ext) = file.get("mediaType").and_then(Value::as_str) {
            fields.push(("ext", ext.to_lowercase().into()));
        }
        for (key, source) in [
            ("width", "width"),
            ("height", "height"),
            ("tbr", "bitrate"),
            ("filesize", "filesize"),
        ] {
            if let Some(value) = int_or_none(file.get(source)) {
                fields.push((key, value.into()));
            }
        }
        let mut format = dict_format(fields, NAME, video_id)?;
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    Ok(formats)
}

impl Extractor for Rtvslo {
    fn key(&self) -> &'static str {
        KEY
    }

    /// `_GEO_COUNTRIES = ['SI']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(GEO_COUNTRIES)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(NAME, url);
        let meta = download_json(
            client,
            &api("getRecordingDrm", &video_id, None)?,
            NAME,
            &video_id,
        )?;
        let meta = meta
            .get("response")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "response"))?;

        let mut thumbnails = Vec::new();
        if let Some(images) = meta.get("images").filter(|images| truthy(images)) {
            let Value::Object(images) = images else {
                return Err(unsupported("image lists that are not objects"));
            };
            for (id, image) in images {
                thumbnails.push(json!({
                    "id": id,
                    "url": image,
                    "http_headers": {"Accept": "image/jpeg"},
                }));
            }
        }

        let subtitles = subtitles(&meta)?;

        let jwt = match meta.get("jwt") {
            Some(Value::String(jwt)) if !jwt.is_empty() => jwt.clone(),
            Some(value) if truthy(value) => {
                return Err(unsupported("authentication tokens that are not strings"));
            }
            _ => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Site did not provide an authentication token, cannot proceed.; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };

        let media = download_json(
            client,
            &api("getMedia", &video_id, Some(&jwt))?,
            NAME,
            &video_id,
        )?;
        let media = media
            .get("response")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "response"))?;

        let mut formats = Vec::new();
        if let Some(adaptive) = hls_sec(media.get("addaptiveMedia")) {
            formats = extract_wowza_formats(client, &adaptive, NAME, &video_id, SKIP_PROTOCOLS)?;
        }
        if let Some(adaptive) = hls_sec(media.get("addaptiveMedia_sl")) {
            for mut format in
                extract_wowza_formats(client, &adaptive, NAME, &video_id, SKIP_PROTOCOLS)?
            {
                format.format_id = format!("sign-{}", format.format_id);
                format.format_note = Some("Sign language interpretation".into());
                format.extra.insert("preference".into(), (-10).into());
                if format.language.as_deref() == Some("eng")
                    && format.acodec.as_deref() != Some("none")
                {
                    format.language = Some("slv".into());
                }
                for key in ["format_id", "format_note", "preference", "language"] {
                    crate::info_order::mark_format_update(&mut format, key);
                }
                formats.push(format);
            }
        }
        formats.extend(media_file_formats(&media, &video_id)?);
        for file in media
            .get("mediaFiles")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(stream) = file
                .pointer("/streams/hls_sec")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            formats.extend(extract_wowza_formats(
                client,
                &stream,
                NAME,
                &video_id,
                SKIP_PROTOCOLS,
            )?);
        }

        if formats
            .iter()
            .any(|format| format.url.as_str().contains("intermission.mp4"))
        {
            crate::geo::restricted_metadata_available(
                NAME,
                &format!("[{NAME}] {video_id}: "),
                GEO_MESSAGE,
                GEO_COUNTRIES,
            )?;
        }
        if formats.iter().any(|format| {
            format
                .extra
                .get("manifest_url")
                .and_then(Value::as_str)
                .is_some_and(|manifest| manifest.contains("dummy_720p.mp4"))
        }) && meta.get("stub").and_then(Value::as_str) == Some("error")
        {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: {NAME} said: Clip not available"
            )));
        }

        let webpage_url = canonical(&meta)?;
        let page = Url::parse(&webpage_url)
            .map_err(|_| unsupported("recordings without an absolute canonical URL"))?;
        let get = |key: &str| meta.get(key).cloned().unwrap_or(Value::Null);
        let timestamp = [meta.get("broadcastDate"), meta.pointer("/broadcastDates/0")]
            .into_iter()
            .flatten()
            .find(|value| !value.is_null())
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, true));
        let release_timestamp = meta
            .get("recordingDate")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, true));
        let duration = match meta.get("duration") {
            Some(duration) if truthy(duration) => duration.clone(),
            _ => meta
                .get("length")
                .and_then(Value::as_str)
                .and_then(crate::parse_duration)
                .map_or(Value::Null, |duration| json!(duration)),
        };
        let fields = vec![
            ("id", Value::from(video_id.clone())),
            ("webpage_url", webpage_url.into()),
            ("title", get("title")),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("thumbnails", Value::Array(thumbnails)),
            ("description", get("description")),
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
            (
                "release_timestamp",
                release_timestamp.map_or(Value::Null, Value::from),
            ),
            ("duration", duration),
            ("tags", get("genre")),
            ("series", get("showName")),
            ("series_id", get("showId")),
        ];
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
        info.webpage_url = page;
        info.subtitles = subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";subtitle=url,ext;thumbnail=id,url,http_headers");
        }
        Ok(Some(info))
    }
}

/// `url_result(url, ie=RTVSLOIE)`.
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), KEY.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for RtvsloShow {
    fn key(&self) -> &'static str {
        SHOW_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SHOW_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        start(SHOW_NAME, url);
        let webpage = download_webpage(client, url, SHOW_NAME, &playlist_id)?;
        let title = crate::html::html_extract_title(&webpage);
        if title.is_none() {
            super::search_warning(SHOW_NAME, "title");
        }
        let title = title.filter(|title| !title.is_empty());
        let description = crate::html::og_search_property(&webpage, "description");
        if description.is_none() {
            super::search_warning(SHOW_NAME, "OpenGraph description");
        }
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            super::search_warning(SHOW_NAME, "thumbnail URL");
        }
        // `playlist_from_matches(..., getter=urljoin('https://365.rtvslo.si'))`:
        // ordered, deduplicated targets.
        let base = Url::parse("https://365.rtvslo.si")?;
        let mut targets: Vec<String> = Vec::new();
        for found in ARCHIVE_LINK.captures_iter(&webpage) {
            let target = base.join(&found[1])?.to_string();
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        let entries = targets
            .iter()
            .map(|target| url_entry(target))
            .collect::<Result<Vec<_>>>()?;
        let mut keys = vec!["description", "thumbnail", "id"];
        if title.is_some() {
            keys.push("title");
        }
        let mut extra = Map::new();
        extra.insert(
            "description".into(),
            description.map_or(Value::Null, Value::from),
        );
        extra.insert(
            "thumbnail".into(),
            thumbnail.map_or(Value::Null, Value::from),
        );
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: SHOW_NAME.into(),
            extractor_key: SHOW_KEY.into(),
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
            "https://www.rtvslo.si/rtv365/arhiv/174842550?s=tv"
        ));
        assert!(suitable("https://365.rtvslo.si/arhiv/utrip/174843754"));
        assert!(suitable("https://4d.rtvslo.si/arhiv/dnevnik/174842550"));
        assert!(!suitable(
            "https://365.rtvslo.si/oddaja/ekipa-bled/173250997"
        ));
        assert!(show_suitable(
            "https://365.rtvslo.si/oddaja/ekipa-bled/173250997"
        ));
        assert!(!show_suitable("https://www.rtvslo.si/rtv365/arhiv/1"));
    }
}
