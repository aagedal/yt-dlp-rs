//! Upstream `FranceTVIE`, `FranceTVSiteIE` and `FranceTVInfoIE`
//! (`yt_dlp/extractor/francetv.py`): `francetv:ID` videos through the k7
//! player API (desktop and mobile device JSON, 422 error codes for geo
//! restriction, DRM and other messages, signed token URLs, DASH, HLS with
//! the audio bitrate fixup, RTMP and checked plain URLs, the audio
//! description preference, the spritesheet storyboard and the HEAD geo
//! check), france.tv pages' Next.js flight data video IDs and franceinfo
//! pages' player IDs as smuggled `francetv:` URL results, and franceinfo's
//! Dailymotion embeds as a playlist of `url` results.
use super::{
    Extractor, dict_format, dict_info, extract_m3u8_master_optional, extract_mpd_optional_note,
};
use crate::{Error, ExtractResult, PlaylistInfo, Result, Subtitles, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct FranceTv;
pub struct FranceTvSite;
pub struct FranceTvInfo;

const TV_NAME: &str = "francetv";
const TV_KEY: &str = "FranceTV";
const SITE_NAME: &str = "francetv:site";
const SITE_KEY: &str = "FranceTVSite";
const INFO_NAME: &str = "franceinfo";
const INFO_KEY: &str = "FranceTVInfo";
const GEO_COUNTRIES: &[&str] = &["FR"];
const GEO_MESSAGE: &str = "This video is not available from your location due to geo restriction";

static TV_URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^francetv:(?P<id>[^@#]+)").unwrap());
static SITE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www\.)?france\.tv|mobile\.france\.tv)/(?:[^/]+/)*(?P<id>[^/]+)\.html",
    )
    .unwrap()
});
static INFO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www|mobile|france3-regions)\.france(?:tv)?info.fr/(?:[^/?#]+/)*(?P<id>[^/?#&.]+)",
    )
    .unwrap()
});
static EPISODE_INFO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"S(\d+)\s*E(\d+)").unwrap());

/// `FranceTVIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    TV_URL.is_match(url)
}

/// `FranceTVSiteIE._match_valid_url`.
pub fn site_suitable(url: &str) -> bool {
    SITE_URL.is_match(url)
}

/// `FranceTVInfoIE._match_valid_url`.
pub fn info_suitable(url: &str) -> bool {
    INFO_URL.is_match(url)
}

fn announce(name: &'static str, url: &str) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url)
    ));
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!(
        "France Télévisions {what} are not supported natively"
    ))
}

/// A non-expected `ExtractorError` (`_search_regex` miss or bare raise).
fn bug(name: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{name}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// A string-or-`None` API value; other types are unsupported.
fn text(value: Option<&Value>, what: &str) -> Result<Option<String>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(_) => Err(unsupported(&format!("{what} that are not strings"))),
    }
}

/// `url_or_none(value)`.
fn url_or_none(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
}

/// `_download_json(url, video_id, note, fatal=False, expected_status=...)`:
/// the parsed body, or `None` after the upstream warning.
fn download_json_optional(
    client: &Client,
    url: &Url,
    id: &str,
    note: &str,
    expected: &[u16],
) -> Result<Option<Value>> {
    use std::io::Read;
    crate::screen::screen(&format!("[{TV_NAME}] {id}: {note}"));
    let mut response = match super::direct::request_expecting(client, url, &[], expected) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{TV_NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = super::decode_webpage(&body, response.headers());
    match crate::json::from_str(&text) {
        Ok(value) => Ok(Some(value)),
        Err(error) => {
            crate::screen::warning(&format!(
                "[{TV_NAME}] {id}: Failed to parse JSON  (caused by {error})"
            ));
            Ok(None)
        }
    }
}

/// `_is_valid_url(url, video_id, item)`: a GET request, or a skipping note.
fn is_valid_url(client: &Client, url: &str, id: &str, item: &str) -> Result<bool> {
    let checked = super::proto_relative_http(url);
    if !checked.starts_with("http://") && !checked.starts_with("https://") {
        return Ok(true);
    }
    crate::screen::screen(&format!("[{TV_NAME}] {id}: Checking {item} URL"));
    let target = Url::parse(&checked)?;
    match super::direct::request(client, &target, None) {
        Ok(_) => Ok(true),
        Err(error) => {
            crate::screen::screen(&format!(
                "[{TV_NAME}] {id}: {item} URL is invalid, skipping: {}",
                crate::upstream_error_text(&error)
            ));
            Ok(false)
        }
    }
}

/// `format_id` of a video (`None` printed as Python does).
fn shown(value: Option<&str>) -> &str {
    value.unwrap_or("None")
}

/// A k7 `video` object's format ID (`video.get('format')`).
fn video_format_id(video: &Value) -> Result<Option<String>> {
    text(video.get("format"), "video formats")
}

/// `traverse_obj(video, ('token', (None, 'akamai'), {url_or_none}, any))`.
fn token_url(video: &Value) -> Option<String> {
    let token = video.get("token")?;
    url_or_none(Some(token)).or_else(|| url_or_none(token.get("akamai")))
}

/// `FranceTVIE._extract_video(video_id, hostname)`.
fn extract_video(client: &Client, page: &Url, video_id: &str, hostname: &str) -> Result<VideoInfo> {
    let mut is_live: Option<Value> = None;
    let mut videos: Vec<Value> = Vec::new();
    let mut drm_formats = false;
    let mut title: Option<String> = None;
    let mut subtitle: Option<String> = None;
    let mut episode_number: Option<String> = None;
    let mut season_number: Option<String> = None;
    let mut image: Option<String> = None;
    let mut duration: Option<Value> = None;
    let mut timestamp: Option<i64> = None;
    let mut spritesheets: Option<Value> = None;
    let present = |value: Option<&Value>| value.filter(|value| !value.is_null()).cloned();

    // Desktop Chrome returns DASH; mobile Safari returns HLS.
    for (device_type, browser) in [("desktop", "chrome"), ("mobile", "safari")] {
        let mut api = Url::parse(&format!("https://k7.ftven.fr/videos/{video_id}"))?;
        api.query_pairs_mut()
            .append_pair("device_type", device_type)
            .append_pair("browser", browser)
            .append_pair("domain", hostname);
        let note = format!("Downloading {device_type} {browser} video JSON");
        let Some(dinfo) = download_json_optional(client, &api, video_id, &note, &[422])? else {
            continue;
        };
        if !super::nuxt::truthy(&dinfo) {
            continue;
        }
        if let Some(video) = dinfo.get("video").filter(|video| video.is_object()) {
            videos.push(video.clone());
            if duration.is_none() {
                duration = present(video.get("duration"));
            }
            if is_live.is_none() {
                is_live = present(video.get("is_live"));
            }
            if spritesheets.is_none() {
                spritesheets = present(video.get("spritesheets"));
            }
        } else if let Some(code) = dinfo
            .get("code")
            .filter(|code| code.is_i64() || code.is_u64())
            .and_then(Value::as_i64)
        {
            match code {
                2009 => {
                    return Err(crate::geo::restricted(
                        &format!("[{TV_NAME}] {video_id}: {GEO_MESSAGE}"),
                        GEO_COUNTRIES,
                    ));
                }
                // 2015, 2019: DRM-only; 2017: unavailable on the mobile site
                // (DRM).
                2015 | 2017 | 2019 => {
                    drm_formats = true;
                    continue;
                }
                _ => {
                    let message = text(dinfo.get("message"), "error messages")?
                        .map_or_else(|| "None".to_owned(), |text| crate::html::clean_html(&text));
                    crate::screen::warning(&format!(
                        "[{TV_NAME}] {TV_NAME} said: {code} \"{message}\""
                    ));
                    continue;
                }
            }
        }

        if let Some(meta) = dinfo.get("meta").filter(|meta| meta.is_object()) {
            if title.is_none() {
                title = text(meta.get("title"), "titles")?;
            }
            // `pre_title` holds a series' "S<ID> E<ID>".
            let pre_title = text(meta.get("pre_title"), "pre-titles")?;
            (season_number, episode_number) = pre_title
                .as_deref()
                .and_then(|pre_title| EPISODE_INFO.captures(pre_title))
                .map_or((None, None), |found| {
                    (Some(found[1].to_owned()), Some(found[2].to_owned()))
                });
            if subtitle.is_none() {
                subtitle = text(meta.get("additional_title"), "additional titles")?;
            }
            if image.is_none() {
                image = text(meta.get("image_url"), "image URLs")?;
            }
            if timestamp.is_none() {
                timestamp = text(meta.get("broadcasted_at"), "broadcast dates")?
                    .as_deref()
                    .and_then(crate::dates::parse_iso8601);
            }
        }
    }

    if videos.is_empty() && drm_formats {
        // `report_drm`.
        crate::no_formats::raise_expected(
            TV_NAME,
            Some(video_id),
            None,
            "This video is DRM protected",
        )?;
    }

    let mut formats: Vec<VideoFormat> = Vec::new();
    let mut subtitles = Subtitles::default();
    let mut last_url: Option<String> = None;
    for video in &videos {
        if url_or_none(video.get("url")).is_none() {
            continue;
        }
        let mut video_url = video["url"].as_str().unwrap_or_default().to_owned();
        let format_id = video_format_id(video)?;

        if let Some(token_url) = token_url(video) {
            let token = super::update_url_query(
                &Url::parse(&token_url)?,
                &[("format", "json"), ("url", &video_url)],
            );
            let note = format!(
                "Downloading signed {} manifest URL",
                shown(format_id.as_deref())
            );
            let signed = download_json_optional(client, &token, video_id, &note, &[])?;
            if let Some(signed) = url_or_none(signed.as_ref().and_then(|signed| signed.get("url")))
            {
                video_url = signed;
            }
        }
        last_url = Some(video_url.clone());
        let parsed =
            || Url::parse(&video_url).map_err(|_| unsupported("video URLs that are not absolute"));
        // `format_id or DEFAULT` (an empty ID is falsy).
        let id_or = |default: &str| {
            format_id
                .clone()
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| default.to_owned())
        };

        let ext = super::direct::determine_ext(&video_url).unwrap_or_default();
        if ext == "f4m" {
            let f4m_id = id_or(&ext);
            formats.extend(super::extract_f4m_optional(
                client,
                TV_NAME,
                &parsed()?,
                video_id,
                Some(&f4m_id),
            )?);
        } else if ext == "m3u8" {
            let m3u8_id = id_or("hls");
            if let Some(stream) = extract_m3u8_master_optional(
                client,
                &parsed()?,
                TV_NAME,
                video_id,
                Some(&m3u8_id),
                true,
                false,
            ) {
                let audio = Regex::new(&format!(
                    r"^{}-[Aa]udio-\w+-(?P<bitrate>\d+)",
                    regex::escape(&m3u8_id)
                ))
                .map_err(|_| unsupported("format IDs"))?;
                let mut fmts = stream.formats;
                for format in &mut fmts {
                    if format.vcodec.as_deref() != Some("none") || format.tbr.is_some() {
                        continue;
                    }
                    let Some(bitrate) = audio
                        .captures(&format.format_id)
                        .and_then(|found| found["bitrate"].parse::<i64>().ok())
                    else {
                        continue;
                    };
                    format.tbr = Some(bitrate as f64);
                    format.acodec = Some("mp4a".into());
                    crate::info_order::mark_format_update(format, "tbr");
                    crate::info_order::mark_format_update(format, "acodec");
                    crate::info_order::mark_int_rates(format);
                }
                formats.extend(fmts);
                super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else if ext == "mpd" {
            let mpd_id = id_or("dash");
            if let Some(stream) = extract_mpd_optional_note(
                client,
                &parsed()?,
                TV_NAME,
                video_id,
                Some(&mpd_id),
                "Downloading MPD manifest",
            ) {
                formats.extend(stream.formats);
                super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else if video_url.starts_with("rtmp") {
            let rtmp_id = match format_id.as_deref() {
                Some(id) if !id.is_empty() => format!("rtmp-{id}"),
                _ => "rtmp".to_owned(),
            };
            parsed()?;
            formats.push(dict_format(
                vec![
                    ("url", video_url.clone().into()),
                    ("format_id", rtmp_id.into()),
                    ("ext", "flv".into()),
                ],
                TV_NAME,
                video_id,
            )?);
        } else if is_valid_url(client, &video_url, video_id, shown(format_id.as_deref()))? {
            parsed()?;
            formats.push(dict_format(
                vec![
                    ("url", video_url.clone().into()),
                    (
                        "format_id",
                        format_id.clone().map_or(Value::Null, Value::from),
                    ),
                ],
                TV_NAME,
                video_id,
            )?);
        }
    }

    if formats.is_empty()
        && let Some(video_url) = &last_url
    {
        // `_request_webpage(HEADRequest(video_url), ..., fatal=False,
        // expected_status=403)`.
        crate::screen::screen(&format!(
            "[{TV_NAME}] {video_id}: Checking for geo-restriction"
        ));
        let target = Url::parse(video_url)?;
        match super::direct::head_expecting(client, &target, &[], &[403]) {
            Ok(response) => {
                let geo = response
                    .headers()
                    .get("x-errortype")
                    .is_some_and(|value| value.as_bytes() == b"geo");
                if geo {
                    crate::geo::restricted_metadata_available(
                        TV_NAME,
                        &format!("[{TV_NAME}] {video_id}: "),
                        GEO_MESSAGE,
                        GEO_COUNTRIES,
                    )?;
                }
            }
            Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
                crate::screen::warning(&format!(
                    "[{TV_NAME}] {video_id}: Unable to download webpage: {}",
                    crate::upstream_error_text(&error)
                ));
            }
            Err(error) => return Err(error),
        }
    }

    for format in &mut formats {
        if format.acodec.as_deref() == Some("none")
            || !matches!(format.language.as_deref(), Some("qtz" | "qad"))
        {
            continue;
        }
        format
            .extra
            .insert("language_preference".into(), Value::from(-10));
        crate::info_order::mark_format_update(format, "language_preference");
        let note = match format.format_note.as_deref() {
            Some(note) if !note.is_empty() => format!("audio description, {note}"),
            _ => "audio description".to_owned(),
        };
        if format.format_note.is_none() {
            crate::info_order::mark_format_update(format, "format_note");
        }
        format.format_note = Some(note);
    }

    if let Some(sheets) = spritesheets.filter(super::nuxt::truthy) {
        let urls: Vec<&Value> = match &sheets {
            Value::Array(items) => items.iter().collect(),
            Value::Object(map) => map.values().collect(),
            _ => Vec::new(),
        };
        // Each spritesheet is roughly a 10x10 grid of 2-second thumbnails.
        let fragments: Vec<Value> = urls
            .into_iter()
            .filter_map(|sheet| url_or_none(Some(sheet)))
            .map(|sheet| json!({"url": sheet, "duration": 200}))
            .collect();
        let mut storyboard = dict_format(
            vec![
                ("format_id", "spritesheets".into()),
                ("format_note", "storyboard".into()),
                ("acodec", "none".into()),
                ("vcodec", "none".into()),
                ("ext", "mhtml".into()),
                ("protocol", "mhtml".into()),
                ("url", "about:invalid".into()),
                ("fragments", Value::Array(fragments)),
            ],
            TV_NAME,
            video_id,
        )?;
        crate::info_order::mark_int_fragment_durations(&mut storyboard);
        formats.push(storyboard);
    }

    // `join_nonempty(title, subtitle, delim=' - ').strip()`.
    let full_title = [title.as_deref(), subtitle.as_deref()]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" - ")
        .trim()
        .to_owned();
    let numbered = episode_number.is_some();
    let int = |value: &Option<String>| {
        value
            .as_deref()
            .and_then(|value| value.parse::<i64>().ok())
            .map_or(Value::Null, Value::from)
    };
    let fields = vec![
        ("id", Value::from(video_id)),
        ("title", Value::from(full_title)),
        ("thumbnail", image.map_or(Value::Null, Value::from)),
        ("duration", duration.unwrap_or(Value::Null)),
        ("timestamp", timestamp.map_or(Value::Null, Value::from)),
        ("is_live", is_live.unwrap_or(Value::Null)),
        ("formats", Value::Null),
        ("subtitles", Value::Null),
        (
            "episode",
            if numbered {
                subtitle.clone().map_or(Value::Null, Value::from)
            } else {
                Value::Null
            },
        ),
        (
            "series",
            if numbered {
                title.clone().map_or(Value::Null, Value::from)
            } else {
                Value::Null
            },
        ),
        ("episode_number", int(&episode_number)),
        ("season_number", int(&season_number)),
        // Prioritize m3u8 over DASH.
        ("_format_sort_fields", json!(["res", "tbr", "proto"])),
    ];
    let mut info = dict_info(page, (TV_NAME, TV_KEY), fields, formats);
    info.subtitles = subtitles;
    Ok(info)
}

impl Extractor for FranceTv {
    fn key(&self) -> &'static str {
        TV_KEY
    }

    /// `_GEO_BYPASS = False` and `_GEO_COUNTRIES = ['FR']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(GEO_COUNTRIES)
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let original = url.as_str();
        if !TV_URL.is_match(original) {
            return Ok(None);
        }
        announce(TV_NAME, original);
        let (bare, smuggled) = super::brightcove::unsmuggle(original);
        let Some(found) = TV_URL.captures(&bare) else {
            return Err(unsupported("smuggled URLs that do not match"));
        };
        let video_id = found["id"].to_owned();
        // `smuggled_data.get('hostname') or 'www.france.tv'`.
        let hostname = match smuggled.get("hostname") {
            None | Some(Value::Null) => "www.france.tv".to_owned(),
            Some(Value::String(host)) if host.is_empty() => "www.france.tv".to_owned(),
            Some(Value::String(host)) => host.clone(),
            Some(_) => return Err(unsupported("smuggled hostnames that are not strings")),
        };
        extract_video(client, url, &video_id, &hostname).map(Some)
    }
}

/// `FranceTVBaseInfoExtractor._make_url_result(video_id, url)`: a
/// `francetv:ID` `url` result smuggling the page's hostname.
fn make_url_result(page: &Url, video_id: &str) -> ExtractResult {
    // Compat with old `@catalog` IDs.
    let video_id = video_id.split('@').next().unwrap_or_default();
    let mut target = format!("francetv:{video_id}");
    if let Some(host) = page.host_str() {
        target = super::brightcove::smuggle(&target, &json!({ "hostname": host }));
    }
    super::kaltura::follow(page, target)
}

impl Extractor for FranceTvSite {
    fn key(&self) -> &'static str {
        SITE_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SITE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        announce(SITE_NAME, url.as_str());
        let webpage = super::download_webpage(client, url, SITE_NAME, &display_id)?;
        let nextjs_data = super::search_nextjs_v13_data(&webpage, SITE_NAME, &display_id)?;
        // `get_first(nextjs_data, ('options', 'id', {str}))`.
        let video_id = nextjs_data.values().find_map(|data| {
            data.get("options")
                .and_then(|options| options.get("id"))
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
        });
        let Some(video_id) = video_id else {
            return Err(bug(SITE_NAME, &display_id, "Unable to extract video ID"));
        };
        Ok(Some(make_url_result(url, video_id)))
    }
}

/// `DailymotionIE._EMBED_REGEX` (the `swf/(?!video)` alternative last, as
/// `DailymotionIE.suitable` checks it).
static DAILYMOTION_EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)<(?:(?:embed|iframe)[^>]+?src=|input[^>]+id=['"]dmcloudUrlEmissionSelect['"][^>]+value=)["'](?P<url>(?:https?:)?//(?:dai\.ly/|(?:(?:(?:www|touch|geo)\.)?dailymotion\.[a-z]{2,3}|(?:www\.)?lequipe\.fr)/(?:(?:(?:crawler|embed|swf)/)?video/|swf/|player(?:/[\da-z]+)?\.html\?(?:video|playlist)=))[^/?_&#]+(?:[\w-]*\?playlist=x[0-9a-z]+)?)"#,
    )
    .unwrap()
});
static DAILYMOTION_PLAYER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)DM\.player\([^,]+,\s*\{.*?video['"]?\s*:\s*["']?(?P<id>[0-9a-zA-Z]+).+?\}\s*\);"#,
    )
    .unwrap()
});
static DAILYMOTION_SCRIPT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<script [^>]*\bsrc=(?:"(?:https?:)?//[\w-]+\.dailymotion\.com/player/[^"]+"|'(?:https?:)?//[\w-]+\.dailymotion\.com/player/[^']+')[^>]*>"#,
    )
    .unwrap()
});

/// `DailymotionIE._extract_embed_urls(url, webpage)`.
fn dailymotion_embed_urls(page: &Url, webpage: &str) -> Vec<String> {
    let mut urls = Vec::new();
    for found in DAILYMOTION_EMBED.captures_iter(webpage) {
        let embed = crate::html::unescape_html(&found["url"]);
        if let Ok(joined) = page.join(&embed)
            && super::dailymotion_suitable(joined.as_str())
        {
            urls.push(joined.to_string());
        }
    }
    for found in DAILYMOTION_PLAYER.captures_iter(webpage) {
        urls.push(format!(
            "https://www.dailymotion.com/embed/video/{}",
            &found["id"]
        ));
    }
    for found in DAILYMOTION_SCRIPT.find_iter(webpage) {
        let attrs = crate::html_elements::extract_attributes(found.as_str());
        let Some(src) = url_or_none(attrs.get("src")) else {
            continue;
        };
        let mut player = src.replace(".js", ".html");
        if player.starts_with("//") {
            player = format!("https:{player}");
        }
        let query = if let Some(video) = attrs.get("data-video").and_then(Value::as_str) {
            format!("video={video}")
        } else if let Some(playlist) = attrs.get("data-playlist").and_then(Value::as_str) {
            format!("playlist={playlist}")
        } else {
            continue;
        };
        let Ok(mut player) = Url::parse(&player) else {
            continue;
        };
        player.set_query(Some(&query));
        urls.push(player.to_string());
    }
    urls
}

/// `url_result(url, DailymotionIE.ie_key())`.
fn dailymotion_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), "Dailymotion".into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": "Dailymotion",
            "extractor": "dailymotion",
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

static INFO_VIDEO_ID: LazyLock<[Regex; 4]> = LazyLock::new(|| {
    [
        Regex::new(r#"player\.load[^;]+src:\s*["']([^"']+)"#).unwrap(),
        Regex::new(r#"id-video=([^@]+@[^"]+)"#).unwrap(),
        Regex::new(r#"<a[^>]+href="(?:https?:)?//videos\.francetv\.fr/video/([^@]+@[^"]+)""#)
            .unwrap(),
        Regex::new(
            r#"(?:data-id|<figure[^<]+\bid)=["']([\da-f]{8}-[\da-f]{4}-[\da-f]{4}-[\da-f]{4}-[\da-f]{12})"#,
        )
        .unwrap(),
    ]
});

impl Extractor for FranceTvInfo {
    fn key(&self) -> &'static str {
        INFO_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = INFO_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        announce(INFO_NAME, url.as_str());
        let webpage = super::download_webpage(client, url, INFO_NAME, &display_id)?;

        let embeds = dailymotion_embed_urls(url, &webpage);
        if !embeds.is_empty() {
            let entries = embeds
                .iter()
                .map(|embed| dailymotion_entry(embed))
                .collect::<Result<Vec<_>>>()?;
            let mut extra = Map::new();
            extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                "playlist-dict;".into(),
            );
            // `playlist_result(entries)`: no `id` key.
            extra.insert("id".into(), Value::Null);
            return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                id: String::new(),
                title: String::new(),
                webpage_url: url.clone(),
                extractor: INFO_NAME.into(),
                extractor_key: INFO_KEY.into(),
                nested: Vec::new(),
                pending: None,
                entries,
                extra,
            })));
        }

        // The player wrapper's `id`, else the older embed patterns.
        let wrapper = crate::html_elements::elements_text_and_html_by_attribute_regex(
            "(button|div)",
            "data-cy",
            "francetv-player-wrapper",
            &webpage,
        )
        .map_err(|error| unsupported(&format!("player wrappers that fail to parse ({error})")))?;
        let wrapper_id = wrapper.first().and_then(|(_, whole)| {
            crate::html_elements::extract_attributes(whole)
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
        });
        let video_id = match wrapper_id {
            Some(id) => id,
            None => INFO_VIDEO_ID
                .iter()
                .find_map(|pattern| pattern.captures(&webpage).map(|found| found[1].to_owned()))
                .ok_or_else(|| bug(INFO_NAME, &display_id, "Unable to extract video id"))?,
        };
        Ok(Some(make_url_result(url, &video_id)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for url in [
            "francetv:ec217ecc-0733-48cf-ac06-af1347b849d1",
            "francetv:162311093",
            "francetv:NI_1004933@Zouzous",
            "francetv:SIM_France3",
        ] {
            assert!(suitable(url), "{url}");
        }
        let found = TV_URL.captures("francetv:NI_983319@Info-web").unwrap();
        assert_eq!(&found["id"], "NI_983319");
        for url in [
            "https://www.france.tv/france-2/13h15-le-dimanche/140921-les-mysteres-de-jesus.html",
            "https://mobile.france.tv/france-5/c-dans-l-air/137347-emission-du-vendredi-12-mai-2017.html",
            "https://www.france.tv/142749-rouge-sang.html",
            "https://www.france.tv/france-3/direct.html",
        ] {
            assert!(site_suitable(url), "{url}");
            assert!(!info_suitable(url), "{url}");
        }
        let found = SITE_URL
            .captures(
                "https://www.france.tv/france-4/hero-corp/saison-1/134151-apres-le-calme.html",
            )
            .unwrap();
        assert_eq!(&found["id"], "134151-apres-le-calme");
        for url in [
            "https://www.francetvinfo.fr/replay-jt/france-3/soir-3/jt-grand-soir-3-jeudi-22-aout-2019_3561461.html",
            "http://france3-regions.francetvinfo.fr/limousin/emissions/jt-1213-limousin",
            "https://www.franceinfo.fr/replay-jt/france-2/20-heures/robert-de-niro-portrait-d-un-monument-du-cinema_7245456.html",
        ] {
            assert!(info_suitable(url), "{url}");
            assert!(!site_suitable(url), "{url}");
        }
        let found = INFO_URL
            .captures("http://www.francetvinfo.fr/economie/entreprises/les-entreprises-familiales-le-secret-de-la-reussite_933271.html")
            .unwrap();
        assert_eq!(
            &found["id"],
            "les-entreprises-familiales-le-secret-de-la-reussite_933271"
        );
    }

    #[test]
    fn results_smuggle_the_page_hostname() {
        let page = Url::parse("https://mobile.france.tv/x/1-y.html").unwrap();
        let ExtractResult::Video(info) = make_url_result(&page, "abc@Regions") else {
            panic!("a url result");
        };
        assert_eq!(
            info.extra[crate::FOLLOW_URL],
            "francetv:abc#__youtubedl_smuggle=%7B%22hostname%22%3A+%22mobile.france.tv%22%7D"
        );
    }

    #[test]
    fn dailymotion_embeds_follow_upstream_patterns() {
        let page = Url::parse("https://www.francetvinfo.fr/a_1.html").unwrap();
        let html = r#"<iframe src="//www.dailymotion.com/embed/video/x4iiko0?autoplay=0"></iframe>
            <script>DM.player(el, { video: "x5kesuj" });</script>
            <script src="//geo.dailymotion.com/player/xa.js" data-playlist="x6"></script>"#;
        assert_eq!(
            dailymotion_embed_urls(&page, html),
            [
                "https://www.dailymotion.com/embed/video/x4iiko0",
                "https://www.dailymotion.com/embed/video/x5kesuj",
                "https://geo.dailymotion.com/player/xa.html?playlist=x6",
            ]
        );
    }
}
