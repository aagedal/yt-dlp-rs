//! Upstream `NiconicoIE` (`yt_dlp/extractor/niconico.py`): the watch API
//! (`v3_guest`, or `v3` with a `user_session` cookie) with upstream's error
//! and geo-restriction messages, the domand `access-rights/hls` request and
//! its HLS audio/video formats (audio fields from the API, video IDs from
//! the URLs and rates net of the lowest audio bitrate), thumbnails and
//! metadata. The comment "subtitles" (inline JSON data) are unsupported.
//!
//! Also the playlist-type extractors: `NiconicoPlaylistIE`,
//! `NiconicoSeriesIE`, `NiconicoHistoryIE` (`OnDemandPagedList` pages),
//! `NiconicoUserIE` and the `NicovideoSearchBaseIE` searches (generators).
use super::Extractor;
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Niconico;

const NAME: &str = "niconico";
const KEY: &str = "Niconico";
const API_BASE: &str = "https://nvapi.nicovideo.jp";
const BASE_URL: &str = "https://www.nicovideo.jp";
const HEADERS: [(&str, &str); 2] = [("X-Frontend-ID", "6"), ("X-Frontend-Version", "0")];
const LOGIN_HINT: &str = "Use --cookies, --cookies-from-browser, --username and --password, --netrc-cmd, or --netrc (niconico) to provide account credentials. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";
const THUMB_PREFS: [&str; 6] = ["url", "middleUrl", "largeUrl", "player", "ogp", "short"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:embed|sp|www)\.)?nicovideo\.jp/(?:shorts|watch)/(?P<id>(?:[a-z]{2})?\d+)",
    )
    .unwrap()
});

/// `NiconicoIE.suitable`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn fail(id: &str, message: &str) -> Error {
    Error::Reported(format!("[{NAME}] {id}: {message}"))
}

fn bug(id: &str, cause: &str) -> Error {
    fail(
        id,
        &format!(
            "An extractor error has occurred. (caused by {cause}); {}",
            crate::BUG_REPORTS_MESSAGE
        ),
    )
}

fn int(value: Option<&Value>) -> Option<i64> {
    let value = value?;
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// `_ERROR_MAP[err_code][reason_code or 'DEFAULT']`.
fn error_message(code: &str, reason: Option<&str>) -> Option<&'static str> {
    let map: &[(&str, &str)] = match code {
        "FORBIDDEN" => &[
            (
                "ADMINISTRATOR_DELETE_VIDEO",
                "Video unavailable, possibly removed by admins",
            ),
            ("CHANNEL_MEMBER_ONLY", "Channel members only"),
            (
                "DELETED_CHANNEL_VIDEO",
                "Video unavailable, channel was closed",
            ),
            (
                "DELETED_COMMUNITY_VIDEO",
                "Video unavailable, community deleted or missing",
            ),
            ("DEFAULT", "Page unavailable, check the URL"),
            ("HARMFUL_VIDEO", "Sensitive content, login required"),
            ("HIDDEN_VIDEO", "Video unavailable, set to private"),
            ("NOT_ALLOWED", "No permission"),
            ("PPV_VIDEO", "PPV video, payment information required"),
            ("PREMIUM_ONLY", "Premium members only"),
        ],
        "INVALID_PARAMETER" => &[("DEFAULT", "Video unavailable, may not exist or was deleted")],
        "MAINTENANCE" => &[("DEFAULT", "Maintenance is in progress")],
        "NOT_FOUND" => &[
            ("DEFAULT", "Video unavailable, may not exist or was deleted"),
            (
                "RIGHT_HOLDER_DELETE_VIDEO",
                "Removed by rights-holder request",
            ),
        ],
        "UNAUTHORIZED" => &[("DEFAULT", "Invalid session, re-login required")],
        _ => &[],
    };
    let find = |key: &str| {
        map.iter()
            .find(|(name, _)| *name == key)
            .map(|(_, message)| *message)
    };
    reason.and_then(find).or_else(|| find("DEFAULT"))
}

fn logged_in() -> bool {
    crate::cookies::get_cookie(BASE_URL, "user_session").is_some_and(|value| !value.is_empty())
}

/// `_download_json(url, data=json, headers)` for the access-rights request.
fn post_json(
    client: &Client,
    url: &Url,
    id: &str,
    headers: &[(&str, &str)],
    body: String,
) -> Result<Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading JSON metadata"));
    let mut response =
        super::direct::post(client, url, headers, body).map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor: NAME,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            },
            error => error,
        })?;
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    serde_json::from_slice(&bytes)
        .map_err(|error| fail(id, &format!("Failed to parse JSON  (caused by {error})")))
}

/// `_extract_formats(api_data, video_id)`.
fn extract_formats(
    client: &Client,
    api_data: &Value,
    video_id: &str,
) -> Result<Vec<crate::VideoFormat>> {
    let available = |path: &str| -> Vec<Value> {
        api_data
            .pointer(path)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter(|item| {
                item.get("isAvailable")
                    .is_some_and(|flag| truthy(Some(flag)))
                    && truthy(item.get("id"))
            })
            .cloned()
            .collect()
    };
    let videos = available("/media/domand/videos");
    let audios = available("/media/domand/audios");
    let access_key = api_data
        .pointer("/media/domand/accessRightKey")
        .and_then(Value::as_str)
        .filter(|key| !key.is_empty());
    let track_id = api_data
        .pointer("/client/watchTrackId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty());
    let (Some(access_key), Some(track_id)) = (access_key, track_id) else {
        return Ok(Vec::new());
    };
    if videos.is_empty() || audios.is_empty() {
        return Ok(Vec::new());
    }
    let mut outputs = Vec::new();
    for video in &videos {
        for audio in &audios {
            outputs.push(json!([video["id"], audio["id"]]));
        }
    }
    let mut url = Url::parse(&format!("{API_BASE}/v1/watch/{video_id}/access-rights/hls"))?;
    url.query_pairs_mut().append_pair("actionTrackId", track_id);
    let mut headers = vec![
        ("Accept", "application/json;charset=utf-8"),
        ("Content-Type", "application/json"),
        ("X-Access-Right-Key", access_key),
        ("X-Request-With", BASE_URL),
    ];
    headers.extend(HEADERS);
    let rights = post_json(
        client,
        &url,
        video_id,
        &headers,
        crate::output::python_json_dumps(&json!({"outputs": outputs})),
    )?;
    let Some(data) = rights.get("data") else {
        return Err(bug(video_id, "KeyError('data')"));
    };
    let Some(m3u8_url) = data.get("contentUrl") else {
        return Err(bug(video_id, "KeyError('contentUrl')"));
    };
    let manifest = Url::parse(m3u8_url.as_str().unwrap_or_default())?;
    let raw = super::extract_m3u8_fatal(client, &manifest, NAME, video_id, None, false)?.formats;

    let mut formats = Vec::new();
    for mut format in raw
        .iter()
        .filter(|format| format.vcodec.as_deref() == Some("none"))
        .cloned()
    {
        if let Some(audio) = audios.iter().find(|audio| {
            audio
                .get("id")
                .and_then(Value::as_str)
                .is_some_and(|id| format.format_id.starts_with(id))
        }) {
            if let Some(abr) = audio
                .get("bitRate")
                .and_then(Value::as_f64)
                .map(|rate| rate / 1000.0)
            {
                format.abr = Some(abr);
                crate::info_order::mark_format_update(&mut format, "abr");
            }
            if let Some(asr) = int(audio.get("samplingRate")) {
                format.asr = u32::try_from(asr).ok();
                crate::info_order::mark_format_update(&mut format, "asr");
            }
            if let Some(id) = audio.get("id").and_then(Value::as_str) {
                format.format_id = id.to_owned();
            }
            if let Some(quality) = int(audio.get("qualityLevel")) {
                format.quality = Some(quality as f64);
                crate::info_order::mark_int_quality(&mut format);
            }
        }
        format.acodec = Some("aac".into());
        crate::info_order::mark_format_update(&mut format, "acodec");
        formats.push(format);
    }
    // Sorted by `tbr` (stable), keeping each URL's lowest-`tbr` format.
    let mut video_formats: Vec<crate::VideoFormat> = raw
        .into_iter()
        .filter(|format| format.vcodec.as_deref() != Some("none"))
        .collect();
    video_formats.sort_by(|left, right| {
        left.tbr
            .unwrap_or(0.0)
            .partial_cmp(&right.tbr.unwrap_or(0.0))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut seen: Vec<Url> = Vec::new();
    video_formats.retain(|format| {
        if seen.contains(&format.url) {
            false
        } else {
            seen.push(format.url.clone());
            true
        }
    });
    let min_abr = audios
        .iter()
        .filter_map(|audio| audio.get("bitRate").and_then(Value::as_f64))
        .map(|rate| rate / 1000.0)
        .reduce(f64::min)
        .unwrap_or(0.0);
    for mut format in video_formats {
        let basename = format
            .url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .unwrap_or_default()
            .to_owned();
        format.format_id = basename
            .rsplit_once('.')
            .map_or(String::new(), |(stem, _)| stem.to_owned());
        let quality = videos
            .iter()
            .find(|video| {
                video.get("id").and_then(Value::as_str) == Some(format.format_id.as_str())
            })
            .and_then(|video| int(video.get("qualityLevel")))
            .filter(|quality| *quality != 0)
            .unwrap_or(-1);
        format.quality = Some(quality as f64);
        crate::info_order::mark_int_quality(&mut format);
        format.tbr = format.tbr.map(|tbr| tbr - min_abr);
        formats.push(format);
    }
    Ok(formats)
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(items)) => !items.is_empty(),
    }
}

/// `parse_resolution(url, lenient=True)`.
fn lenient_resolution(text: &str) -> (Option<u32>, Option<u32>) {
    static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)\s*[xX×,]\s*(\d+)").unwrap());
    static WIDTH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?:^|[^\d])(\d{2,5})w(?:$|[^a-zA-Z0-9])").unwrap());
    if let Some(found) = SIZE.captures(text) {
        return (found[1].parse().ok(), found[2].parse().ok());
    }
    let strict = super::html5::parse_resolution(text);
    if strict.1.is_some() {
        return strict;
    }
    match WIDTH.captures(text) {
        Some(found) => (found[1].parse().ok(), None),
        None => (None, None),
    }
}

/// `find_element(cls=CLASS)`: the first such element's content.
fn element_by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()?
        .into_iter()
        .next()
        .map(|(text, _)| text)
}

/// `traverse_obj(page, ({find_element(...)}, ..., {clean_html}))`: a
/// nonempty cleaned text.
fn cleaned(text: Option<String>) -> Option<String> {
    text.map(|text| crate::html::clean_html(&text))
        .filter(|text| !text.is_empty())
}

static POST_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<form[^>]+action=(?:"(.+?)"|'(.+?)')"#).unwrap());

impl Extractor for Niconico {
    fn key(&self) -> &'static str {
        NAME
    }

    fn netrc_machine(&self) -> Option<&'static str> {
        Some("niconico")
    }

    fn performs_login(&self) -> bool {
        true
    }

    /// `NiconicoBaseIE._perform_login`: the session cookies, the
    /// `mail_tel`/`password` form, and the MFA one-time password if asked.
    fn perform_login(
        &self,
        client: &Client,
        session: &super::login::Session,
        username: &str,
        password: &str,
    ) -> Result<()> {
        const LOGIN_BASE: &str = "https://account.nicovideo.jp";
        const FORM: (&str, &str) = ("Content-Type", "application/x-www-form-urlencoded");
        let login_error = |message: &str| session.error(&format!("Unable to login: {message}"));
        if logged_in() {
            return Ok(());
        }
        session.request(
            client,
            &Url::parse(&format!("{LOGIN_BASE}/login"))?,
            (
                Some("Requesting session cookies"),
                "Unable to download webpage",
            ),
            &[],
            None,
            &[],
        )?;
        let referer = format!("{LOGIN_BASE}/login");
        let data = super::login::urlencode(&[("mail_tel", username), ("password", password)]);
        let webpage = session.request(
            client,
            &Url::parse(&format!("{LOGIN_BASE}/login/redirector"))?,
            (Some("Logging in"), "Unable to log in"),
            &[FORM, ("Referer", &referer)],
            Some(data),
            &[],
        )?;
        if logged_in() {
            return Ok(());
        }
        let notice = element_by_class("notice error", &webpage.text)
            .and_then(|notice| element_by_class("notice__text", &notice));
        if let Some(message) = cleaned(notice) {
            return Err(login_error(&message));
        }
        if webpage.text.contains("oneTimePw") {
            let found = POST_URL
                .captures(&webpage.text)
                .ok_or_else(|| session.bug("Unable to extract post url"))?;
            let post_url = found.get(1).or_else(|| found.get(2)).unwrap().as_str();
            let otp = session.tfa_info("6 digit number shown on app")?;
            let data = super::login::urlencode(&[("otp", &otp)]);
            let mfa = session.request(
                client,
                &Url::parse(LOGIN_BASE)?.join(post_url)?,
                (Some("Performing MFA"), "Unable to complete MFA"),
                &[FORM],
                Some(data),
                &[],
            )?;
            if logged_in() {
                return Ok(());
            }
            if mfa
                .url
                .query_pairs()
                .any(|(key, value)| key == "error-code" && !value.is_empty())
            {
                let message = cleaned(element_by_class("pageMainMsg", &mfa.text));
                return Err(login_error(
                    message.as_deref().unwrap_or("MFA session expired"),
                ));
            }
            if mfa.text.contains("formError") {
                let message = cleaned(element_by_class("formError", &mfa.text).and_then(|error| {
                    crate::html_elements::element_text_and_html_by_tag("div", &error)
                        .ok()
                        .map(|(text, _)| text)
                }));
                return Err(login_error(
                    message.as_deref().unwrap_or("MFA challenge failed"),
                ));
            }
        }
        Err(session.bug("Unable to login: Unexpected login error"))
    }

    /// `_GEO_BYPASS = False` and `_GEO_COUNTRIES = ['JP']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(&["JP"])
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, page: &Url) -> Result<Option<ExtractResult>> {
        use std::io::Read;
        let Some(found) = VALID_URL.captures(page.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        self.initialize(client, NAME, &video_id)?;
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(page.as_str())
        ));
        let path = if logged_in() { "v3" } else { "v3_guest" };
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |time| time.as_millis());
        let mut api = Url::parse(&format!("{BASE_URL}/api/watch/{path}/{video_id}"))?;
        api.query_pairs_mut()
            .append_pair("actionTrackId", &format!("AAAAAAAAAA_{millis}"));
        crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading API JSON"));
        let mut response = super::direct::request_expecting(client, &api, &HEADERS, &[400, 404])
            .map_err(|error| match error {
                Error::Http(_) | Error::HttpRetryAfter { .. } => Error::Reported(
                    super::request_error_text(NAME, &video_id, "API JSON", &error).replacen(
                        ": Unable to download API JSON:",
                        ": Unable to fetch data:",
                        1,
                    ),
                ),
                error => error,
            })?;
        let mut bytes = Vec::new();
        response
            .read_to_end(&mut bytes)
            .map_err(Error::ExtractionRead)?;
        let api_resp: Value = serde_json::from_slice(&bytes).map_err(|error| {
            fail(
                &video_id,
                &format!("Failed to parse JSON  (caused by {error})"),
            )
        })?;
        let api_data = api_resp
            .get("data")
            .filter(|data| data.is_object())
            .cloned()
            .unwrap_or(Value::Null);
        let scheduled = api_data
            .get("publishScheduledAt")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let status = api_resp.pointer("/meta/status").and_then(Value::as_i64);
        if status != Some(200) {
            let code = api_resp
                .pointer("/meta/errorCode")
                .and_then(Value::as_str)
                .map(str::to_uppercase);
            let reason = str_or_none(api_data.get("reasonCode"));
            let mut message = code
                .as_deref()
                .and_then(|code| error_message(code, reason.as_deref()))
                .map(str::to_owned);
            match reason.as_deref() {
                Some("DOMESTIC_VIDEO" | "HIGH_RISK_COUNTRY_VIDEO") => {
                    // `raise_geo_restricted(countries=['JP'])`.
                    return Err(crate::geo::restricted(
                        &fail(
                            &video_id,
                            "This video is not available from your location due to geo restriction",
                        )
                        .to_string(),
                        &["JP"],
                    ));
                }
                Some("HARMFUL_VIDEO")
                    if api_data.pointer("/viewer/allowSensitiveContents")
                        == Some(&Value::Bool(false)) =>
                {
                    message = Some("Sensitive content, adjust display settings to watch".into());
                }
                Some("HIDDEN_VIDEO") if scheduled.is_some() => {
                    message = Some(format!(
                        "This content is scheduled to be released at {}",
                        scheduled.as_deref().unwrap_or_default()
                    ));
                }
                Some(
                    "CHANNEL_MEMBER_ONLY"
                    | "HARMFUL_VIDEO"
                    | "HIDDEN_VIDEO"
                    | "PPV_VIDEO"
                    | "PREMIUM_ONLY",
                ) => {
                    let Some(message) = message else {
                        return Err(bug(
                            &video_id,
                            "TypeError(\"unsupported operand type(s) for +=: 'NoneType' and 'str'\")",
                        ));
                    };
                    return Err(fail(&video_id, &format!("{message}. {LOGIN_HINT}")));
                }
                _ => {}
            }
            if let Some(message) = message {
                return Err(fail(&video_id, &message));
            }
            if status.is_some_and(|status| status >= 500) {
                return Err(fail(&video_id, "Service temporarily unavailable"));
            }
            return Err(fail(
                &video_id,
                &format!(
                    "API returned error status {}; {}",
                    status.map_or("None".into(), |status| status.to_string()),
                    crate::BUG_REPORTS_MESSAGE
                ),
            ));
        }
        // `_availability(needs_auth, needs_subscription, needs_premium)`.
        let payment = api_data.pointer("/payment/video");
        let flag = |key: &str| {
            payment
                .and_then(|payment| payment.get(key))
                .and_then(Value::as_bool)
        };
        let availability = if flag("isPremium") == Some(true) {
            "premium_only"
        } else if flag("isAdmission") == Some(true) {
            "subscriber_only"
        } else if flag("isContinuationBenefit")
            .or_else(|| flag("isPpv"))
            .unwrap_or(false)
        {
            "needs_auth"
        } else {
            "public"
        };
        let formats = extract_formats(client, &api_data, &video_id)?;
        let status_message = match availability {
            "needs_auth" => Some("PPV video, payment information required"),
            "premium_only" => Some("Premium members only"),
            "subscriber_only" => Some("Channel members only"),
            _ => None,
        };
        if formats.is_empty()
            && let Some(message) = status_message
        {
            return Err(fail(&video_id, &format!("{message}. {LOGIN_HINT}")));
        }

        let mut dict = Map::new();
        dict.insert("availability".into(), availability.into());
        dict.insert("display_id".into(), video_id.clone().into());
        dict.insert("formats".into(), json!([]));
        if let Some(genre) = api_data
            .pointer("/genre/label")
            .and_then(Value::as_str)
            .filter(|label| !label.is_empty())
        {
            dict.insert("genres".into(), json!([genre]));
        }
        dict.insert(
            "release_timestamp".into(),
            scheduled
                .as_deref()
                .and_then(crate::dates::parse_iso8601)
                .into(),
        );
        dict.insert(
            "subtitles".into(),
            if super::subtitles_requested() {
                if truthy(api_data.pointer("/comment/nvComment/server")) {
                    return Err(Error::UnsupportedHtml5(
                        "Niconico comments (inline subtitle data) are not supported natively"
                            .into(),
                    ));
                }
                Value::Null
            } else {
                json!({})
            },
        );
        let tags: Vec<Value> = api_data
            .pointer("/tag/items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| tag.get("name").and_then(Value::as_str))
            .filter(|name| !name.is_empty())
            .map(Value::from)
            .collect();
        if !tags.is_empty() {
            dict.insert("tags".into(), Value::Array(tags));
        }
        let thumbnails: Vec<Value> = api_data
            .pointer("/video/thumbnail")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter_map(|(key, url)| {
                let url = url.as_str().and_then(super::json_ld::url_or_none)?;
                let preference = THUMB_PREFS
                    .iter()
                    .position(|name| name == key)
                    .map_or(-1, |index| index as i64);
                let mut thumbnail = Map::new();
                thumbnail.insert("ext".into(), "jpg".into());
                thumbnail.insert("id".into(), key.clone().into());
                thumbnail.insert("preference".into(), preference.into());
                thumbnail.insert("url".into(), url.clone().into());
                let (width, height) = lenient_resolution(&url);
                if let Some(width) = width {
                    thumbnail.insert("width".into(), width.into());
                }
                if let Some(height) = height {
                    thumbnail.insert("height".into(), height.into());
                }
                Some(Value::Object(thumbnail))
            })
            .collect();
        dict.insert("thumbnails".into(), Value::Array(thumbnails));
        if let Some(owner) = ["channel", "owner"]
            .iter()
            .find_map(|key| api_data.get(*key).filter(|owner| !owner.is_null()))
        {
            let name = ["name", "nickname"]
                .iter()
                .find_map(|key| owner.get(*key).and_then(Value::as_str));
            let id = str_or_none(owner.get("id"));
            if let Some(name) = name {
                dict.insert("channel".into(), name.into());
            }
            if let Some(id) = &id {
                dict.insert("channel_id".into(), id.clone().into());
            }
            if let Some(name) = name {
                dict.insert("uploader".into(), name.into());
            }
            if let Some(id) = id {
                dict.insert("uploader_id".into(), id.into());
            }
        }
        let video = api_data.get("video").cloned().unwrap_or(Value::Null);
        if let Some(id) = str_or_none(video.get("id")) {
            dict.insert("id".into(), id.into());
        }
        if let Some(title) = video.get("title").and_then(Value::as_str) {
            dict.insert("title".into(), title.into());
        }
        if let Some(description) = video
            .get("description")
            .and_then(Value::as_str)
            .map(crate::html::clean_html)
            .filter(|text| !text.is_empty())
        {
            dict.insert("description".into(), description.into());
        }
        if let Some(duration) = int(video.get("duration")) {
            dict.insert("duration".into(), duration.into());
        }
        if let Some(timestamp) = video
            .get("registeredAt")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            dict.insert("timestamp".into(), timestamp.into());
        }
        for (key, source) in [
            ("comment_count", "comment"),
            ("like_count", "like"),
            ("view_count", "view"),
        ] {
            if let Some(count) = int(video.pointer(&format!("/count/{source}"))) {
                dict.insert(key.into(), count.into());
            }
        }
        if !dict.contains_key("id") {
            return Err(fail(&video_id, "Missing \"id\" field in extractor result"));
        }

        let first = formats
            .first()
            .map_or_else(|| page.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        info.direct = None;
        if formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.formats = formats;
        info.webpage_url = page.clone();
        info.extractor = NAME.into();
        info.extractor_key = KEY.into();
        let titled = dict.contains_key("title");
        if !titled {
            info.extra
                .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
        }
        let mut keys = Vec::new();
        for (key, value) in dict {
            keys.push(key.clone());
            match key.as_str() {
                "id" => info.id = value.as_str().unwrap_or_default().to_owned(),
                "title" => info.title = value.as_str().unwrap_or_default().to_owned(),
                "formats" => {}
                _ if value.is_null() => {
                    info.extra.insert(key, Value::Null);
                }
                _ => crate::metadata_parser::set_field(&mut info, &key, value),
            }
        }
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info,
            &format!(
                "dict{};{};thumbnail=ext,id,preference,url,width,height",
                if titled { "" } else { "-untitled" },
                keys.join(",")
            ),
        ))))
    }
}

// Upstream's playlist-type extractors: `NiconicoPlaylistBaseIE` (mylists,
// series, the logged-in user's history and likes) as `OnDemandPagedList`s,
// `NiconicoUserIE`'s generator, and the `NicovideoSearchBaseIE` page
// generators (searches, search and tag URLs, the date-bisecting search).

const PLAYLIST_NAME: &str = "niconico:playlist";
const SERIES_NAME: &str = "niconico:series";
const HISTORY_NAME: &str = "niconico:history";
const USER_NAME: &str = "NiconicoUser";
const SEARCH_NAME: &str = "nicovideo:search";
const SEARCH_URL_NAME: &str = "nicovideo:search_url";
const SEARCH_DATE_NAME: &str = "nicovideo:search:date";
const TAG_NAME: &str = "niconico:tag";
/// `NiconicoPlaylistBaseIE._PAGE_SIZE` (and `NiconicoUserIE._PAGE_SIZE`).
const PAGE_SIZE: usize = 100;
/// `NiconicoPlaylistBaseIE._API_HEADERS`.
const PLAYLIST_API_HEADERS: [(&str, &str); 3] = [
    ("X-Frontend-ID", "6"),
    ("X-Frontend-Version", "0"),
    ("X-Niconico-Language", "en-us"),
];
/// `NicovideoSearchDateIE`: `_START_DATE`, `_RESULTS_PER_PAGE`, `_MAX_PAGES`.
const SEARCH_START_DATE: (i64, i64, i64) = (2007, 1, 1);
const RESULTS_PER_PAGE: usize = 32;
const MAX_PAGES: u64 = 50;

static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:(?:www\.|sp\.)?nicovideo\.jp|nico\.ms)/(?:user/\d+/)?(?:my/)?mylist/(?:#/)?(?P<id>\d+)").unwrap()
});
static SERIES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:(?:www\.|sp\.)?nicovideo\.jp(?:/user/\d+)?|nico\.ms)/series/(?P<id>\d+)",
    )
    .unwrap()
});
static HISTORY_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.|sp\.)?nicovideo\.jp/my/(?P<id>history(?:/like)?)").unwrap()
});
static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?nicovideo\.jp/user/(?P<id>\d+)(?:/video)?/?(?:$|[#?])")
        .unwrap()
});
static SEARCH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^nicosearch(?P<prefix>|[1-9][0-9]*|all):(?P<query>[\s\S]+)").unwrap()
});
static SEARCH_DATE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^nicosearchdate(?P<prefix>|[1-9][0-9]*|all):(?P<query>[\s\S]+)").unwrap()
});
static SEARCH_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?nicovideo\.jp/search/(?P<id>[^?#&]+)?").unwrap()
});
static TAG_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?nicovideo\.jp/tag/(?P<id>[^?#&]+)?").unwrap()
});

/// `NiconicoPlaylistIE.suitable`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `NiconicoSeriesIE.suitable`.
pub fn series_suitable(url: &str) -> bool {
    SERIES_URL.is_match(url)
}

/// `NiconicoHistoryIE.suitable`.
pub fn history_suitable(url: &str) -> bool {
    HISTORY_URL.is_match(url)
}

/// `NiconicoUserIE.suitable`.
pub fn user_suitable(url: &str) -> bool {
    USER_URL.is_match(url)
}

/// `NicovideoSearchIE.suitable` (`nicosearch[N|all]:QUERY`).
pub fn search_suitable(url: &str) -> bool {
    SEARCH_RE.is_match(url)
}

/// `NicovideoSearchDateIE.suitable` (`nicosearchdate[N|all]:QUERY`).
pub fn search_date_suitable(url: &str) -> bool {
    SEARCH_DATE_RE.is_match(url)
}

/// `NicovideoSearchURLIE.suitable`.
pub fn search_url_suitable(url: &str) -> bool {
    SEARCH_URL.is_match(url)
}

/// `NicovideoTagURLIE.suitable`.
pub fn tag_suitable(url: &str) -> bool {
    TAG_URL.is_match(url)
}

fn announce(name: &'static str, text: &str) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(text)
    ));
}

/// An `[EXTRACTOR] ID: An extractor error has occurred.` error.
fn bug_as(name: &str, id: &str, cause: &str) -> Error {
    Error::Reported(format!(
        "[{name}] {id}: An extractor error has occurred. (caused by {cause}); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// A playlist entry: an entry dict (`_type: url`) for `NiconicoIE`, its ID
/// from the dict or else `NiconicoIE`'s `_VALID_URL` (`get_temp_id`).
fn url_entry(target: &str, entry: Map<String, Value>) -> Result<VideoInfo> {
    let url = super::target_url(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.id = match entry.get("id") {
        Some(id) => str_or_none(Some(id)).unwrap_or_default(),
        None => VALID_URL
            .captures(target)
            .map(|found| found["id"].to_owned())
            .unwrap_or_default(),
    };
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

/// `url_result(url, 'Niconico', video_id)` (`video_id=None`: no `id` key).
fn watch_result(video_id: Option<&str>, target: &str) -> Result<VideoInfo> {
    let mut entry = Map::new();
    entry.insert("ie_key".into(), KEY.into());
    if let Some(id) = video_id {
        entry.insert("id".into(), id.into());
    }
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    url_entry(target, entry)
}

/// `playlist_result(entries, playlist_id, title, description, **kwargs)`
/// whose entries are fetched while the playlist is processed (a generator
/// or an `OnDemandPagedList`, whose full count is unknown unless a
/// selection reads past its end).
fn pending_playlist(
    page: &Url,
    (extractor, key): (&'static str, &'static str),
    (id, title, description): (Option<String>, Option<String>, Option<Value>),
    kwargs: Vec<(&str, Value)>,
    state: Value,
) -> ExtractResult {
    let mut keys: Vec<&str> = kwargs.iter().map(|(key, _)| *key).collect();
    let mut extra = Map::new();
    for (key, value) in kwargs {
        extra.insert(key.into(), value);
    }
    if id.is_some() {
        keys.push("id");
    } else {
        extra.insert("id".into(), Value::Null);
    }
    let title = title.filter(|title| !title.is_empty());
    if title.is_some() {
        keys.push("title");
    }
    if let Some(description) = description {
        keys.push("description");
        extra.insert("description".into(), description);
    }
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!(
            "playlist-dict;{}{}",
            keys.join(","),
            crate::info_order::GENERATOR
        )
        .into(),
    );
    ExtractResult::Playlist(crate::PlaylistInfo {
        id: id.unwrap_or_default(),
        title: title.unwrap_or_default(),
        webpage_url: page.clone(),
        extractor: extractor.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: Some(crate::PendingEntries { state }),
        entries: Vec::new(),
        extra,
    })
}

/// The HTTP status of a failed request (`e.cause.status`).
fn http_status(error: &Error) -> Option<u16> {
    match error {
        Error::ExtractorRequest { source, .. } => http_status(source),
        Error::Http(source) | Error::HttpRetryAfter { source, .. } => {
            source.status().map(|status| status.as_u16())
        }
        _ => None,
    }
}

/// Which `NiconicoPlaylistBaseIE` subclass a list is.
#[derive(Clone, Copy, PartialEq)]
enum ListKind {
    Mylist,
    Series,
    History,
}

impl ListKind {
    fn from_name(name: &str) -> Self {
        match name {
            "series" => Self::Series,
            "history" => Self::History,
            _ => Self::Mylist,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::Mylist => "mylist",
            Self::Series => "series",
            Self::History => "history",
        }
    }

    fn extractor(self) -> (&'static str, &'static str) {
        match self {
            Self::Mylist => (PLAYLIST_NAME, "NiconicoPlaylist"),
            Self::Series => (SERIES_NAME, "NiconicoSeries"),
            Self::History => (HISTORY_NAME, "NiconicoHistory"),
        }
    }

    /// `_call_api(list_id, resource, query)`.
    fn call_api(
        self,
        client: &Client,
        list_id: &str,
        resource: &str,
        query: &[(&str, &str)],
    ) -> Result<Value> {
        let (name, _) = self.extractor();
        let base = match self {
            Self::Mylist => format!("{API_BASE}/v2/mylists/{list_id}"),
            Self::Series => format!("{API_BASE}/v2/series/{list_id}"),
            Self::History if list_id == "history/like" => format!("{API_BASE}/v1/users/me/likes"),
            Self::History => format!("{API_BASE}/v1/users/me/watch/history"),
        };
        let url = super::update_url_query(&Url::parse(&base)?, query);
        let response = super::download_json_note_headers(
            client,
            &url,
            name,
            list_id,
            &format!("Downloading {resource}"),
            &PLAYLIST_API_HEADERS,
        )?;
        let Some(data) = response.get("data") else {
            return Err(bug_as(name, list_id, "KeyError('data')"));
        };
        if self == Self::Mylist {
            return data
                .get("mylist")
                .cloned()
                .ok_or_else(|| bug_as(name, list_id, "KeyError('mylist')"));
        }
        Ok(data.clone())
    }
}

/// `NiconicoPlaylistBaseIE._parse_owner(item)`.
fn parse_owner(item: &Value) -> Vec<(&'static str, Value)> {
    let owner = item.get("owner");
    let uploader = owner
        .and_then(|owner| {
            [owner.get("name"), owner.pointer("/user/nickname")]
                .into_iter()
                .flatten()
                .find(|value| value.is_string())
        })
        .cloned()
        .unwrap_or(Value::Null);
    let uploader_id = owner
        .and_then(|owner| owner.get("id"))
        .filter(|id| id.is_string())
        .cloned()
        .unwrap_or(Value::Null);
    vec![("uploader", uploader), ("uploader_id", uploader_id)]
}

/// `NiconicoPlaylistBaseIE._fetch_page(list_id, page)`.
fn fetch_list_page(
    client: &Client,
    kind: ListKind,
    list_id: &str,
    page: usize,
) -> Result<Vec<VideoInfo>> {
    let page = (page + 1).to_string();
    let size = PAGE_SIZE.to_string();
    let resp = kind.call_api(
        client,
        list_id,
        &format!("page {page}"),
        &[("page", &page), ("pageSize", &size)],
    )?;
    // `traverse_obj(resp, ('items', ..., ('video', None)))`: each item's
    // `video`, then the item itself (mylists and users).
    let items: Vec<&Value> = match resp.get("items") {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    };
    let mut entries = Vec::new();
    for item in items {
        let candidates = [
            item.as_object().and_then(|item| item.get("video")),
            Some(item),
        ];
        for video in candidates.into_iter().flatten() {
            if video.is_null() {
                continue;
            }
            let Some(video) = video.as_object() else {
                return Err(Error::UnsupportedHtml5(
                    "Niconico list items that are not objects are not supported natively".into(),
                ));
            };
            let Some(id) = video.get("id").filter(|id| truthy(Some(id))) else {
                continue;
            };
            let Some(id_text) = str_or_none(Some(id)) else {
                return Err(Error::UnsupportedHtml5(
                    "Niconico video IDs that are not strings or numbers are not supported natively"
                        .into(),
                ));
            };
            let count = video
                .get("count")
                .filter(|count| truthy(Some(count)))
                .cloned()
                .unwrap_or(json!({}));
            let get_count = |key: &str| int(count.get(key)).map_or(Value::Null, Value::from);
            // A branching path: every non-`None` value, as a (maybe empty)
            // list.
            let thumbnail: Vec<Value> = ["nHdUrl", "largeUrl", "listingUrl", "url"]
                .iter()
                .filter_map(|key| video.get("thumbnail")?.get(*key))
                .filter(|value| !value.is_null())
                .cloned()
                .collect();
            let target = format!("{BASE_URL}/watch/{id_text}");
            let field = |key: &str| video.get(key).cloned().unwrap_or(Value::Null);
            let mut entry = Map::new();
            entry.insert("_type".into(), "url".into());
            entry.insert("id".into(), id.clone());
            entry.insert("title".into(), field("title"));
            entry.insert("url".into(), target.clone().into());
            entry.insert("description".into(), field("shortDescription"));
            entry.insert(
                "duration".into(),
                int(video.get("duration")).map_or(Value::Null, Value::from),
            );
            entry.insert("view_count".into(), get_count("view"));
            entry.insert("comment_count".into(), get_count("comment"));
            entry.insert("thumbnail".into(), Value::Array(thumbnail));
            entry.insert("ie_key".into(), KEY.into());
            for (key, value) in parse_owner(&Value::Object(video.clone())) {
                entry.insert(key.into(), value);
            }
            entries.push(url_entry(&target, entry)?);
        }
    }
    Ok(entries)
}

/// Whether a playlist's pending state is a Niconico list's or search's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    matches!(
        state["kind"].as_str(),
        Some("niconico_list" | "niconico_user" | "niconico_search" | "niconico_search_date")
    )
}

/// The pending entries of a Niconico list (`OnDemandPagedList(fetch_page,
/// 100)` read by upstream's `PlaylistEntries`, see [`super::paged`]), user
/// or search (generators, read in order until `limit` entries exist).
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let kind = ListKind::from_name(state["list"].as_str().unwrap_or_default());
    let (name, result) = match state["kind"].as_str() {
        Some("niconico_user") => (USER_NAME, fetch_user_pages(client, state, limit, entries)),
        Some("niconico_search") => (
            search_extractor(&state),
            fetch_search_pages(client, state, limit, entries),
        ),
        Some("niconico_search_date") => (
            SEARCH_DATE_NAME,
            fetch_search_date_pages(client, state, limit, entries),
        ),
        _ => {
            let list_id = state["list_id"].as_str().unwrap_or_default().to_owned();
            let mut fetch = |page| fetch_list_page(client, kind, &list_id, page);
            let placeholder = || watch_result(None, &format!("{BASE_URL}/watch/None"));
            let result = super::paged::fetch_requested(
                state,
                limit,
                entries,
                PAGE_SIZE,
                &mut fetch,
                &placeholder,
            );
            (kind.extractor().0, result)
        }
    };
    // An `ExtractorError` raised while the entries are read (outside
    // `extract()`) has no `[EXTRACTOR]` prefix.
    result.map_err(|error| match error {
        Error::ExtractorRequest { .. } | Error::Reported(_) => {
            let text = error.to_string();
            match text.strip_prefix(&format!("[{name}] ")) {
                Some(rest) => Error::Reported(rest.to_owned()),
                None => error,
            }
        }
        error => error,
    })
}

/// `NiconicoPlaylistIE`.
pub struct NiconicoPlaylist;
/// `NiconicoSeriesIE`.
pub struct NiconicoSeries;
/// `NiconicoHistoryIE` (cookies only: it has no password login).
pub struct NiconicoHistory;

/// `_real_extract` of a `NiconicoPlaylistBaseIE` subclass.
fn extract_list(
    client: &Client,
    page: &Url,
    kind: ListKind,
    pattern: &Regex,
) -> Result<Option<ExtractResult>> {
    let Some(found) = pattern.captures(page.as_str()) else {
        return Ok(None);
    };
    let list_id = found["id"].to_owned();
    let (name, _) = kind.extractor();
    announce(name, page.as_str());
    let info = match kind.call_api(client, &list_id, "list", &[("pageSize", "1")]) {
        Ok(info) => info,
        Err(error) if kind == ListKind::History && http_status(&error) == Some(401) => {
            // `raise_login_required(...)` of an extractor without logins.
            return Err(Error::Reported(format!(
                "[{name}] {list_id}: You have to be logged in to get your history. {}",
                super::login::login_hint(None, Some(super::login::Hint::Cookies))
            )));
        }
        Err(error) => return Err(error),
    };
    let (title, description, owner_of) = match kind {
        ListKind::Mylist => (info.get("name"), info.get("description"), &info),
        ListKind::Series => {
            let Some(detail) = info.get("detail") else {
                return Err(bug_as(name, &list_id, "KeyError('detail')"));
            };
            (detail.get("title"), detail.get("description"), detail)
        }
        ListKind::History => (None, None, &info),
    };
    let title = match title {
        None | Some(Value::Null) => None,
        Some(Value::String(title)) => Some(title.clone()),
        Some(_) => {
            return Err(Error::UnsupportedHtml5(
                "Niconico list titles that are not strings are not supported natively".into(),
            ));
        }
    };
    let description = description.filter(|value| !value.is_null()).cloned();
    let state = json!({
        "kind": "niconico_list",
        "list": kind.name(),
        "list_id": list_id,
        "random_access": true,
    });
    Ok(Some(pending_playlist(
        page,
        kind.extractor(),
        (Some(list_id), title, description),
        parse_owner(owner_of),
        state,
    )))
}

impl Extractor for NiconicoPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        extract_list(client, url, ListKind::Mylist, &PLAYLIST_URL)
    }
}

impl Extractor for NiconicoSeries {
    fn key(&self) -> &'static str {
        SERIES_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        extract_list(client, url, ListKind::Series, &SERIES_URL)
    }
}

impl Extractor for NiconicoHistory {
    fn key(&self) -> &'static str {
        HISTORY_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        extract_list(client, url, ListKind::History, &HISTORY_URL)
    }
}

/// `NiconicoUserIE`.
pub struct NiconicoUser;

impl Extractor for NiconicoUser {
    fn key(&self) -> &'static str {
        USER_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = USER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let list_id = found["id"].to_owned();
        announce(USER_NAME, url.as_str());
        let state = json!({
            "kind": "niconico_user",
            "list_id": list_id,
            "page_num": 0,
            "count": 0,
            "total": 1,
        });
        Ok(Some(pending_playlist(
            url,
            (USER_NAME, USER_NAME),
            (Some(list_id), None, None),
            Vec::new(),
            state,
        )))
    }
}

/// `NiconicoUserIE._entries(list_id)`: the `v2/users/ID/videos` pages, in
/// order, while fewer than `totalCount` (of the first page) were listed.
fn fetch_user_pages(
    client: &Client,
    mut state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let list_id = state["list_id"].as_str().unwrap_or_default().to_owned();
    loop {
        if limit.is_some_and(|limit| entries.len() >= limit) {
            return Ok(Some(state));
        }
        let page_num = state["page_num"].as_u64().unwrap_or(0);
        let mut count = state["count"].as_u64().unwrap_or(0);
        // `while count < total_count` (`int_or_none(totalCount)`).
        match state["total"].as_i64() {
            Some(total) if (count as i64) < total => {}
            Some(_) => return Ok(None),
            None => {
                return Err(Error::Reported(format!(
                    "[{USER_NAME}] {list_id}: TypeError(\"'<' not supported between instances of 'int' and 'NoneType'\")"
                )));
            }
        }
        let url = Url::parse(&format!(
            "{API_BASE}/v2/users/{list_id}/videos?sortKey=registeredAt&sortOrder=desc&pageSize={PAGE_SIZE}&page={}",
            page_num + 1
        ))?;
        let note = if page_num == 0 {
            "Downloading JSON metadata".to_owned()
        } else {
            format!("Downloading JSON metadata page {page_num}")
        };
        let parsed =
            super::download_json_note_headers(client, &url, USER_NAME, &list_id, &note, &HEADERS)?;
        let Some(data) = parsed.get("data") else {
            return Err(bug_as(USER_NAME, &list_id, "KeyError('data')"));
        };
        if page_num == 0 {
            state["total"] = int(data.get("totalCount")).into();
        }
        let items = match data.get("items") {
            Some(Value::Array(items)) => items.clone(),
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "Niconico user video lists that are not arrays are not supported natively"
                        .into(),
                ));
            }
            None => return Err(bug_as(USER_NAME, &list_id, "KeyError('items')")),
        };
        for item in &items {
            let Some(essential) = item.get("essential") else {
                return Err(bug_as(USER_NAME, &list_id, "KeyError('essential')"));
            };
            let Some(id) = essential.get("id") else {
                return Err(bug_as(USER_NAME, &list_id, "KeyError('id')"));
            };
            let Some(id) = str_or_none(Some(id)) else {
                return Err(Error::UnsupportedHtml5(
                    "Niconico video IDs that are not strings or numbers are not supported natively"
                        .into(),
                ));
            };
            count += 1;
            entries.push(watch_result(None, &format!("{BASE_URL}/watch/{id}"))?);
        }
        state["count"] = count.into();
        state["page_num"] = (page_num + 1).into();
    }
}

/// `re.findall(r'(?<=data-video-id=)["\']?(?P<videoid>.*?)(?=["\'])', page)`.
fn video_ids(page: &str) -> Vec<String> {
    const MARK: &[u8] = b"data-video-id=";
    let bytes = page.as_bytes();
    let quote = |byte: u8| byte == b'"' || byte == b'\'';
    // The first quote at or after `from` with no line break before it.
    let closing = |from: usize| {
        bytes[from.min(bytes.len())..]
            .iter()
            .position(|byte| quote(*byte) || *byte == b'\n')
            .map(|offset| from + offset)
            .filter(|index| quote(bytes[*index]))
    };
    let mut ids = Vec::new();
    let mut pos = 0;
    while pos <= bytes.len() {
        // The next position at or after `pos` that follows the mark.
        let start = pos.saturating_sub(MARK.len());
        let Some(offset) = bytes[start..]
            .windows(MARK.len())
            .position(|window| window == MARK)
        else {
            break;
        };
        let at = start + offset + MARK.len();
        if at < bytes.len() && quote(bytes[at]) {
            match closing(at + 1) {
                Some(end) => {
                    ids.push(String::from_utf8_lossy(&bytes[at + 1..end]).into_owned());
                    pos = end;
                }
                None => {
                    // The quote itself is the lookahead: an empty match.
                    ids.push(String::new());
                    pos = at + 1;
                }
            }
        } else {
            match closing(at) {
                Some(end) => {
                    ids.push(String::from_utf8_lossy(&bytes[at..end]).into_owned());
                    pos = end;
                }
                None => pos = at + 1,
            }
        }
    }
    ids
}

/// One `NicovideoSearchBaseIE._entries` page: `url` with the `query`
/// (`_download_webpage(url, item_id, query=query, note=note)`), its video
/// IDs.
fn search_page(
    client: &Client,
    extractor: &'static str,
    url: &Url,
    item_id: Option<&str>,
    query: &[(String, String)],
    note: &str,
) -> Result<Vec<String>> {
    let pairs: Vec<(&str, &str)> = query
        .iter()
        .map(|(key, value)| (key.as_str(), value.as_str()))
        .collect();
    let url = super::update_url_query(url, &pairs);
    crate::screen::screen(&match item_id {
        Some(id) => format!("[{extractor}] {id}: {note}"),
        None => format!("[{extractor}] {note}"),
    });
    let webpage = super::download_webpage_quiet(client, &url, extractor, item_id.unwrap_or(""))?;
    Ok(video_ids(&webpage))
}

fn search_extractor(state: &Value) -> &'static str {
    match state["extractor"].as_str() {
        Some(SEARCH_URL_NAME) => SEARCH_URL_NAME,
        Some(TAG_NAME) => TAG_NAME,
        _ => SEARCH_NAME,
    }
}

/// Whether the search's `max` results (`islice`) were read.
fn search_done(state: &Value) -> bool {
    state["max"]
        .as_u64()
        .is_some_and(|max| state["count"].as_u64().unwrap_or(0) >= max)
}

/// Pushes a page's results (`url_result(watch URL, 'Niconico', item)`),
/// up to the search's `max`.
fn push_results(ids: &[String], state: &mut Value, entries: &mut Vec<VideoInfo>) -> Result<()> {
    let max = state["max"].as_u64();
    let mut count = state["count"].as_u64().unwrap_or(0);
    for id in ids {
        if max.is_some_and(|max| count >= max) {
            break;
        }
        entries.push(watch_result(Some(id), &format!("{BASE_URL}/watch/{id}"))?);
        count += 1;
    }
    state["count"] = count.into();
    Ok(())
}

/// `NicovideoSearchBaseIE._entries(url, item_id)`: pages 1, 2, … until one
/// has no results (optionally `islice`d to `max` results).
fn fetch_search_pages(
    client: &Client,
    mut state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let extractor = search_extractor(&state);
    let url = Url::parse(state["url"].as_str().unwrap_or_default())?;
    let item_id = state["item_id"].as_str().map(str::to_owned);
    loop {
        if search_done(&state) {
            return Ok(None);
        }
        if limit.is_some_and(|limit| entries.len() >= limit) {
            return Ok(Some(state));
        }
        let page = state["page"].as_u64().unwrap_or(1);
        let ids = search_page(
            client,
            extractor,
            &url,
            item_id.as_deref(),
            &[("page".into(), page.to_string())],
            &format!("Downloading page {page}"),
        )?;
        if ids.is_empty() {
            return Ok(None);
        }
        push_results(&ids, &mut state, entries)?;
        state["page"] = (page + 1).into();
    }
}

fn date_text(days: i64) -> String {
    let (year, month, day) = crate::dates::civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// `_get_entries_for_date`'s query for `start`..`end`.
fn date_query(start: i64, end: i64, page: u64) -> Vec<(String, String)> {
    vec![
        ("start".into(), date_text(start)),
        ("end".into(), date_text(end)),
        ("sort".into(), "f".into()),
        ("order".into(), "d".into()),
        ("page".into(), page.to_string()),
    ]
}

/// `start_date + ((end_date - start_date) // 2)` (a date plus a
/// `timedelta` ignores its part of a day).
fn midpoint(start: i64, end: i64) -> i64 {
    start + (end - start).div_euclid(2)
}

/// `NicovideoSearchDateIE._entries(url, item_id)`: an interval whose last
/// page (50) is full is split in two (newer half first); the others list
/// their pages 1, 2, … until one has no results.
fn fetch_search_date_pages(
    client: &Client,
    mut state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let url = Url::parse(state["url"].as_str().unwrap_or_default())?;
    let item_id = state["item_id"].as_str().unwrap_or_default().to_owned();
    if state["intervals"].is_null() {
        // `start_date or self._START_DATE, end_date or now().date()`.
        let (year, month, day) = SEARCH_START_DATE;
        let start = crate::dates::days_from_civil(year, month, day);
        state["intervals"] = json!([[start, crate::dates::local_today()]]);
    }
    loop {
        if search_done(&state) {
            return Ok(None);
        }
        if limit.is_some_and(|limit| entries.len() >= limit) {
            return Ok(Some(state));
        }
        if let Some(current) = state["current"].as_array().cloned() {
            let start = current[0].as_i64().unwrap_or_default();
            let end = current[1].as_i64().unwrap_or_default();
            let page = current[2].as_u64().unwrap_or(1);
            let ids = search_page(
                client,
                SEARCH_DATE_NAME,
                &url,
                Some(&item_id),
                &date_query(start, end, page),
                &format!("    Downloading page {page}"),
            )?;
            push_results(&ids, &mut state, entries)?;
            state["current"] = if ids.is_empty() {
                Value::Null
            } else {
                json!([start, end, page + 1])
            };
            continue;
        }
        let mut intervals = state["intervals"].as_array().cloned().unwrap_or_default();
        if intervals.is_empty() {
            return Ok(None);
        }
        let interval = intervals.remove(0);
        let start = interval[0].as_i64().unwrap_or_default();
        let end = interval[1].as_i64().unwrap_or_default();
        let (start_text, end_text) = (date_text(start), date_text(end));
        let last_page = search_page(
            client,
            SEARCH_DATE_NAME,
            &url,
            Some(&item_id),
            &date_query(start, end, MAX_PAGES),
            &format!("Checking number of videos from {start_text} to {end_text}"),
        )?;
        if last_page.len() == RESULTS_PER_PAGE && start != end {
            let mid = midpoint(start, end);
            intervals.insert(0, json!([start, mid]));
            intervals.insert(0, json!([mid, end]));
        } else {
            crate::screen::screen(&format!(
                "[{SEARCH_DATE_NAME}] {item_id}: Downloading results from {start_text} to {end_text}"
            ));
            state["current"] = json!([start, end, 1]);
        }
        state["intervals"] = Value::Array(intervals);
    }
}

/// `NicovideoSearchIE` and `NicovideoSearchDateIE`
/// (`SearchInfoExtractor._real_extract`, `_MAX_RESULTS` unbounded).
fn extract_search(url: &Url, date: bool) -> Result<Option<ExtractResult>> {
    let text = super::youtube::search::input_text(url);
    let (pattern, name, key) = if date {
        (&*SEARCH_DATE_RE, SEARCH_DATE_NAME, "NicovideoSearchDate")
    } else {
        (&*SEARCH_RE, SEARCH_NAME, "NicovideoSearch")
    };
    let Some(found) = pattern.captures(&text) else {
        return Ok(None);
    };
    announce(name, &text);
    let query = found["query"].to_owned();
    let max = match &found["prefix"] {
        "" => Some(1),
        "all" => None,
        count => Some(count.parse::<u64>().unwrap_or(u64::MAX)),
    };
    // `_search_results(query)`: `//www.nicovideo.jp/search/QUERY`.
    let target = Url::parse(&format!("{BASE_URL}/search/{query}"))?;
    let state = json!({
        "kind": if date { "niconico_search_date" } else { "niconico_search" },
        "extractor": name,
        "url": target.as_str(),
        "item_id": query,
        "max": max,
        "count": 0,
        "page": 1,
        "intervals": null,
        "current": null,
    });
    Ok(Some(pending_playlist(
        url,
        (name, key),
        (Some(query.clone()), Some(query), None),
        Vec::new(),
        state,
    )))
}

/// `NicovideoSearchURLIE` and `NicovideoTagURLIE`: the URL's pages.
fn extract_search_url(url: &Url, tag: bool) -> Result<Option<ExtractResult>> {
    let (pattern, name, key) = if tag {
        (&*TAG_URL, TAG_NAME, "NicovideoTagURL")
    } else {
        (&*SEARCH_URL, SEARCH_URL_NAME, "NicovideoSearchURL")
    };
    // The URL as parsed: non-ASCII input text is percent-encoded (upstream
    // keeps it as given).
    let Some(found) = pattern.captures(url.as_str()) else {
        return Ok(None);
    };
    announce(name, url.as_str());
    let query = found.name("id").map(|id| id.as_str().to_owned());
    let state = json!({
        "kind": "niconico_search",
        "extractor": name,
        "url": url.as_str(),
        "item_id": query,
        "max": null,
        "count": 0,
        "page": 1,
    });
    Ok(Some(pending_playlist(
        url,
        (name, key),
        (query.clone(), query, None),
        Vec::new(),
        state,
    )))
}

/// `NicovideoSearchIE`.
pub struct NicovideoSearch;
/// `NicovideoSearchDateIE`.
pub struct NicovideoSearchDate;
/// `NicovideoSearchURLIE`.
pub struct NicovideoSearchUrl;
/// `NicovideoTagURLIE`.
pub struct NicovideoTagUrl;

impl Extractor for NicovideoSearch {
    fn key(&self) -> &'static str {
        SEARCH_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        extract_search(url, false)
    }
}

impl Extractor for NicovideoSearchDate {
    fn key(&self) -> &'static str {
        SEARCH_DATE_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        extract_search(url, true)
    }
}

impl Extractor for NicovideoSearchUrl {
    fn key(&self) -> &'static str {
        SEARCH_URL_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        extract_search_url(url, false)
    }
}

impl Extractor for NicovideoTagUrl {
    fn key(&self) -> &'static str {
        TAG_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        extract_search_url(url, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn playlist_urls_follow_upstream_valid_urls() {
        assert!(playlist_suitable("http://www.nicovideo.jp/mylist/27411728"));
        assert!(playlist_suitable(
            "https://www.nicovideo.jp/user/805442/mylist/27411728"
        ));
        assert!(playlist_suitable(
            "https://www.nicovideo.jp/my/mylist/#/68048635"
        ));
        assert!(series_suitable(
            "https://www.nicovideo.jp/user/44113208/series/110226"
        ));
        assert!(series_suitable("https://nico.ms/series/203559"));
        assert!(history_suitable("https://sp.nicovideo.jp/my/history/video"));
        assert!(history_suitable("https://www.nicovideo.jp/my/history/like"));
        assert!(user_suitable("https://www.nicovideo.jp/user/419948"));
        assert!(user_suitable(
            "https://www.nicovideo.jp/user/419948/video?x"
        ));
        assert!(!user_suitable(
            "https://www.nicovideo.jp/user/419948/mylist/1"
        ));
        assert!(search_suitable("nicosearch10:slithering pythons"));
        assert!(search_suitable("nicosearchall:a"));
        assert!(!search_suitable("nicosearch0:a"));
        assert!(!search_suitable("nicosearchdate:a"));
        assert!(search_date_suitable("nicosearchdateall:a"));
        assert!(search_url_suitable("http://www.nicovideo.jp/search/sm9"));
        assert!(search_url_suitable("https://www.nicovideo.jp/search/"));
        assert!(tag_suitable("https://www.nicovideo.jp/tag/a"));
    }

    #[test]
    fn video_ids_follow_upstream_findall() {
        assert_eq!(
            video_ids(r#"<a data-video-id="sm1"> <a data-video-id='so2'>"#),
            ["sm1", "so2"]
        );
        // Unquoted: up to the next quote on the line.
        assert_eq!(
            video_ids("data-video-id=sm1 data-video-id=\"sm2\""),
            ["sm1 data-video-id=", "sm2"]
        );
        // An opening quote without a closing one is an empty match.
        assert_eq!(video_ids("data-video-id=\"sm1\nx\""), [""]);
        assert_eq!(video_ids("data-video-id=sm1\n"), Vec::<String>::new());
        assert_eq!(video_ids(r#"data-video-id="""#), [""]);
    }

    #[test]
    fn date_search_splits_like_upstream() {
        let start = crate::dates::days_from_civil(2007, 1, 1);
        assert_eq!(date_text(start), "2007-01-01");
        assert_eq!(midpoint(start, start + 3), start + 1);
        assert_eq!(midpoint(start, start + 1), start);
        assert_eq!(
            date_query(start, start + 9, 50)
                .iter()
                .map(|(key, value)| format!("{key}={value}"))
                .collect::<Vec<_>>()
                .join("&"),
            "start=2007-01-01&end=2007-01-10&sort=f&order=d&page=50"
        );
    }

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://www.nicovideo.jp/watch/sm9"));
        assert!(suitable(
            "https://sp.nicovideo.jp/watch/sm28964488?ss_pos=1&cp_in=wt_tg"
        ));
        assert!(suitable("https://www.nicovideo.jp/shorts/sm45678901"));
        assert!(suitable("https://www.nicovideo.jp/watch/1173108780"));
        assert!(!suitable("https://www.nicovideo.jp/mylist/27411728"));
    }

    #[test]
    fn errors_follow_the_error_map() {
        assert_eq!(
            error_message("NOT_FOUND", Some("RIGHT_HOLDER_DELETE_VIDEO")),
            Some("Removed by rights-holder request")
        );
        assert_eq!(
            error_message("FORBIDDEN", Some("UNKNOWN")),
            Some("Page unavailable, check the URL")
        );
        assert_eq!(error_message("OTHER", None), None);
    }
}
