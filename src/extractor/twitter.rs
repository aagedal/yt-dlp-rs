//! Upstream `yt_dlp/extractor/twitter.py`: `TwitterIE` (a status through
//! the GraphQL API with a guest token or the logged-in `auth_token`/`ct0`
//! cookies, the legacy API or the syndication endpoint as selected with
//! `--extractor-args twitter:api=...`, and the syndication fallback after a
//! rate limit), its videos' HLS and MP4 variants, unified, player, VMAP and
//! linked cards and the post's metadata; `TwitterCardIE` redirects,
//! `TwitterAmplifyIE`, `TwitterBroadcastIE` (broadcasts and live events),
//! `TwitterSpacesIE` and `TwitterShortenerIE` (`t.co`).
use super::Extractor;
use super::periscope;
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Twitter;
pub struct TwitterCard;
pub struct TwitterAmplify;
pub struct TwitterBroadcast;
pub struct TwitterSpaces;
pub struct TwitterShortener;

const NAME: &str = "twitter";
const KEY: &str = "Twitter";
const CARD_NAME: &str = "twitter:card";
const AMPLIFY_NAME: &str = "twitter:amplify";
const BROADCAST_NAME: &str = "twitter:broadcast";
const SPACES_NAME: &str = "twitter:spaces";
const SHORTENER_NAME: &str = "twitter:shortener";
const API_BASE: &str = "https://api.x.com/1.1/";
const GRAPHQL_API_BASE: &str = "https://x.com/i/api/graphql/";
const GRAPHQL_ENDPOINT: &str = "2ICDjqPd81tulZcYrtpTuQ/TweetResultByRestId";
const SPACES_ENDPOINT: &str = "HPEisOmj1epUNLCWTYhUWw/AudioSpaceById";
const AUTH: &str = "AAAAAAAAAAAAAAAAAAAAANRILgAAAAAAnNwIzUejRCOuH5E6I8xnZz4puTs%3D1Zv7ttfk8LF81IUq16cHjhLTvJu4FA33AGWWjCpTnA";
const LEGACY_AUTH: &str = "AAAAAAAAAAAAAAAAAAAAAIK1zgAAAAAA2tUWuhGZ2JceoId5GwYWU5GspY4%3DUq7gzFoCZs1QfwGoVdvSac3IniczZEYXIcDyumCauIXpcAPorE";
const BASE_REGEX: &str = r"https?://(?:(?:www|m(?:obile)?)\.)?(?:(?:twitter|x)\.com|twitter3e4tixl4xyajtrzo62zg5vztmjuricljdp2c5kshju4avyoid\.onion)/";
const VMAP_NAMESPACE: &str = "http://twitter.com/schema/videoVMapV2.xsd";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(&format!(
        r"^{BASE_REGEX}(?:(?:i/web|[^/]+)/status|statuses)/(?P<id>\d+)(?:/(?:video|photo)/(?P<index>\d+))?"
    ))
    .unwrap()
});
static CARD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^{BASE_REGEX}i/(?:cards/tfw/v1|videos(?:/tweet)?)/(?P<id>\d+)"
    ))
    .unwrap()
});
static AMPLIFY_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://amp\.twimg\.com/v/(?P<id>[0-9a-f\-]{36})").unwrap());
static BROADCAST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^{BASE_REGEX}i/(?P<type>broadcasts|events)/(?P<id>\w+)"
    ))
    .unwrap()
});
static SPACES_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r"^{BASE_REGEX}i/spaces/(?P<id>[0-9a-zA-Z]{{13}})")).unwrap()
});
static SHORTENER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:https?://t\.co/(?P<id>[^?#]+)|tco:(?P<eid>[^?#]+))").unwrap()
});
static DIMENSIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/(?P<width>\d+)x(?P<height>\d+)/").unwrap());
static AUDIO_BITRATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^hls-[Aa]udio-(?P<bitrate>\d{4,})").unwrap());
static URL_JUNK: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+(https?://[^ ]+)").unwrap());
/// `TwitterIE._MEDIA_ID_RE`.
static MEDIA_ID: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"_video/(\d+)/").unwrap());

/// `TwitterIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `TwitterCardIE._match_valid_url`.
pub fn card_suitable(url: &str) -> bool {
    CARD_URL.is_match(url)
}

/// `TwitterAmplifyIE._match_valid_url`.
pub fn amplify_suitable(url: &str) -> bool {
    AMPLIFY_URL.is_match(url)
}

/// `TwitterBroadcastIE._match_valid_url`.
pub fn broadcast_suitable(url: &str) -> bool {
    BROADCAST_URL.is_match(url)
}

/// `TwitterSpacesIE._match_valid_url`.
pub fn spaces_suitable(url: &str) -> bool {
    SPACES_URL.is_match(url)
}

/// `TwitterShortenerIE._match_valid_url` (also the `tco:` pseudo-URL).
pub fn shortener_suitable(url: &str) -> bool {
    SHORTENER_URL.is_match(url)
}

fn int(value: Option<&Value>, scale: i64) -> Option<i64> {
    let value = value?;
    let number = value
        .as_i64()
        .map(|number| number as f64)
        .or_else(|| value.as_f64())
        .or_else(|| value.as_str()?.trim().parse::<f64>().ok())?;
    Some((number / scale as f64) as i64)
}

fn str_or_none(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    }
}

/// An expected `ExtractorError`.
fn fail_as(extractor: &str, id: &str, message: &str) -> Error {
    Error::Reported(format!("[{extractor}] {id}: {message}"))
}

fn fail(id: &str, message: &str) -> Error {
    fail_as(NAME, id, message)
}

/// An unexpected `ExtractorError` (with the bug report request).
fn bug(extractor: &str, id: &str, message: &str) -> Error {
    fail_as(
        extractor,
        id,
        &format!("{message}; {}", crate::BUG_REPORTS_MESSAGE),
    )
}

/// A `KeyError` in `_real_extract`.
fn key_error(extractor: &str, id: &str, key: &str) -> Error {
    bug(
        extractor,
        id,
        &format!("An extractor error has occurred. (caused by KeyError('{key}'))"),
    )
}

fn login_required(extractor: &str, id: &str, message: &str) -> Error {
    fail_as(
        extractor,
        id,
        &format!(
            "{message}. Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies"
        ),
    )
}

fn logged_in() -> bool {
    crate::cookies::get_cookie(API_BASE, "auth_token").is_some()
}

/// `_selected_api`: `--extractor-args twitter:api=...` (`graphql` by default).
fn selected_api() -> String {
    crate::manifest_args::configuration_arg(KEY, "api", false)
        .and_then(|values| values.into_iter().next())
        .unwrap_or_else(|| "graphql".into())
}

/// `_set_base_headers(legacy)`: the bearer token and any `ct0` CSRF token.
fn base_headers(legacy: bool) -> Vec<(&'static str, String)> {
    let bearer = if legacy && !logged_in() {
        LEGACY_AUTH
    } else {
        AUTH
    };
    let mut headers = vec![("Authorization", format!("Bearer {bearer}"))];
    if let Some(csrf) = crate::cookies::get_cookie(API_BASE, "ct0") {
        headers.push(("x-csrf-token", csrf));
    }
    headers
}

fn read_json(response: reqwest::blocking::Response, extractor: &str, id: &str) -> Result<Value> {
    use std::io::Read;
    let mut response = response;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    serde_json::from_slice(&body).map_err(|error| {
        Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

fn request_error<'a>(
    extractor: &'static str,
    id: &'a str,
    what: &'static str,
) -> impl Fn(Error) -> Error + 'a {
    move |error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor,
            id: id.to_owned(),
            what,
            source: Box::new(error),
        },
        error => error,
    }
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

fn header_refs<'a>(headers: &'a [(&'static str, String)]) -> Vec<(&'a str, &'a str)> {
    headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect()
}

/// `_fetch_guest_token(display_id)`: a POST to `guest/activate.json`.
fn guest_token(client: &Client, extractor: &'static str, id: &str) -> Result<String> {
    let url = Url::parse(&format!("{API_BASE}guest/activate.json"))?;
    crate::screen::screen(&format!("[{extractor}] {id}: Downloading guest token"));
    let headers = base_headers(selected_api() == "legacy");
    let response = super::direct::post(client, &url, &header_refs(&headers), String::new())
        .map_err(request_error(extractor, id, "JSON metadata"))?;
    read_json(response, extractor, id)?
        .get("guest_token")
        .and_then(Value::as_str)
        .filter(|token| !token.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| bug(extractor, id, "Could not retrieve guest token"))
}

/// `_call_api(path, video_id, query, graphql)`: the response, after its
/// `errors` are checked.
fn call_api(
    client: &Client,
    extractor: &'static str,
    path: &str,
    id: &str,
    query: &[(&str, String)],
    graphql: bool,
) -> Result<Value> {
    let mut headers = base_headers(!graphql && selected_api() == "legacy");
    if logged_in() {
        headers.push(("x-twitter-auth-type", "OAuth2Session".into()));
        headers.push(("x-twitter-client-language", "en".into()));
        headers.push(("x-twitter-active-user", "yes".into()));
    } else {
        headers.push(("x-guest-token", guest_token(client, extractor, id)?));
    }
    let base = if graphql { GRAPHQL_API_BASE } else { API_BASE };
    let mut url = Url::parse(&format!("{base}{path}"))?;
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (key, value) in query {
            pairs.append_pair(key, value);
        }
    }
    crate::screen::screen(&format!(
        "[{extractor}] {id}: Downloading {} JSON",
        if graphql { "GraphQL" } else { "legacy API" }
    ));
    let allowed: &[u16] = if graphql {
        &[400, 401, 403, 404]
    } else {
        &[403]
    };
    let response = super::direct::request_expecting(client, &url, &header_refs(&headers), allowed)
        .map_err(request_error(extractor, id, "JSON metadata"))?;
    let result = read_json(response, extractor, id)?;
    // `', '.join(set(...))`: one message, or this port's order for several.
    let mut messages: Vec<String> = Vec::new();
    for message in result
        .get("errors")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|error| error.get("message").and_then(Value::as_str))
    {
        if !messages.iter().any(|known| known == message) {
            messages.push(message.to_owned());
        }
    }
    let error = messages.join(", ");
    if !error.is_empty() {
        if error.to_lowercase() == "dependency: unspecified" {
            crate::screen::debug(&format!("Ignoring Twitter API error: \"{error}\""));
        } else if error.to_lowercase().contains("not authorized") {
            return Err(login_required(
                extractor,
                id,
                error.strip_suffix('.').unwrap_or(&error),
            ));
        } else {
            return Err(bug(
                extractor,
                id,
                &format!("Error(s) while querying API: {error}"),
            ));
        }
    }
    Ok(result)
}

/// `_call_graphql_api(endpoint, media_id)`: the compact JSON query's `data`.
fn call_graphql_api(
    client: &Client,
    extractor: &'static str,
    endpoint: &str,
    id: &str,
    query: Vec<(&'static str, Value)>,
) -> Result<Value> {
    let query: Vec<(&str, String)> = query
        .into_iter()
        .map(|(key, value)| (key, value.to_string()))
        .collect();
    let result = call_api(client, extractor, endpoint, id, &query, true)?;
    Ok(result.get("data").cloned().unwrap_or(Value::Null))
}

/// `TwitterIE._build_graphql_query`.
fn graphql_query(id: &str) -> Vec<(&'static str, Value)> {
    let variables = json!({
        "tweetId": id,
        "withCommunity": false,
        "includePromotedContent": false,
        "withVoice": false,
    });
    let features = json!({
        "creator_subscriptions_tweet_preview_api_enabled": true,
        "tweetypie_unmention_optimization_enabled": true,
        "responsive_web_edit_tweet_api_enabled": true,
        "graphql_is_translatable_rweb_tweet_is_translatable_enabled": true,
        "view_counts_everywhere_api_enabled": true,
        "longform_notetweets_consumption_enabled": true,
        "responsive_web_twitter_article_tweet_consumption_enabled": false,
        "tweet_awards_web_tipping_enabled": false,
        "freedom_of_speech_not_reach_fetch_enabled": true,
        "standardized_nudges_misinfo": true,
        "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": true,
        "longform_notetweets_rich_text_read_enabled": true,
        "longform_notetweets_inline_media_enabled": true,
        "responsive_web_graphql_exclude_directive_enabled": true,
        "verified_phone_label_enabled": false,
        "responsive_web_media_download_video_enabled": false,
        "responsive_web_graphql_skip_user_profile_image_extensions_enabled": false,
        "responsive_web_graphql_timeline_navigation_enabled": true,
        "responsive_web_enhance_cards_enabled": false,
    });
    let toggles = json!({"withArticleRichContentState": false});
    vec![
        ("variables", variables),
        ("features", features),
        ("fieldToggles", toggles),
    ]
}

/// `TwitterSpacesIE._build_graphql_query`.
fn spaces_query(id: &str) -> Vec<(&'static str, Value)> {
    vec![
        (
            "variables",
            json!({
                "id": id,
                "isMetatagsQuery": true,
                "withDownvotePerspective": false,
                "withReactionsMetadata": false,
                "withReactionsPerspective": false,
                "withReplays": true,
                "withSuperFollowsUserFields": true,
                "withSuperFollowsTweetFields": true,
            }),
        ),
        (
            "features",
            json!({
                "dont_mention_me_view_api_enabled": true,
                "interactive_text_enabled": true,
                "responsive_web_edit_tweet_api_enabled": true,
                "responsive_web_enhance_cards_enabled": true,
                "responsive_web_uc_gql_enabled": true,
                "spaces_2022_h2_clipping": true,
                "spaces_2022_h2_spaces_communities": false,
                "standardized_nudges_misinfo": true,
                "tweet_with_visibility_results_prefer_gql_limited_actions_policy_enabled": false,
                "vibe_api_enabled": true,
            }),
        ),
    ]
}

/// `_graphql_to_legacy`: the legacy status dict of a GraphQL result.
fn graphql_to_legacy(data: &Value, id: &str) -> Result<Map<String, Value>> {
    let mut result = data
        .pointer("/tweetResult/result")
        .filter(|value| value.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}));
    let typename = result
        .get("__typename")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if !matches!(
        typename.as_deref(),
        None | Some("Tweet" | "TweetWithVisibilityResults" | "TweetTombstone" | "TweetUnavailable")
    ) {
        crate::screen::warning_once(&format!(
            "[{NAME}] {id}: Unknown typename: {}",
            typename.as_deref().unwrap_or_default()
        ));
    }
    if result.get("tombstone").is_some() {
        let cause = result
            .pointer("/tombstone/text/text")
            .and_then(Value::as_str)
            .map(|text| text.strip_suffix(". Learn more").unwrap_or(text).to_owned());
        return Err(fail(
            id,
            &format!(
                "Twitter API says: {}",
                cause.as_deref().unwrap_or("Unknown error")
            ),
        ));
    }
    match typename.as_deref() {
        Some("TweetUnavailable") => {
            let reason = result.get("reason").and_then(Value::as_str);
            return Err(match reason {
                Some("NsfwLoggedOut" | "NsfwViewerHasNoStatedAge") => {
                    login_required(NAME, id, "NSFW tweet requires authentication")
                }
                Some("Protected") => login_required(
                    NAME,
                    id,
                    "You are not authorized to view this protected tweet",
                ),
                _ => fail(id, reason.unwrap_or("Requested tweet is unavailable")),
            });
        }
        Some("TweetWithVisibilityResults") => {
            result = result
                .get("tweet")
                .filter(|value| value.is_object())
                .cloned()
                .unwrap_or_else(|| json!({}));
        }
        _ => {}
    }
    let mut status = result
        .get("legacy")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for (key, path) in [
        ("user", "/core/user_results/result/legacy"),
        ("card", "/card/legacy"),
        ("quoted_status", "/quoted_status_result/result/legacy"),
        (
            "retweeted_status",
            "/legacy/retweeted_status_result/result/legacy",
        ),
    ] {
        if let Some(value) = result.pointer(path).filter(|value| value.is_object()) {
            status.insert(key.into(), value.clone());
        }
    }
    if status
        .get("retweeted_status")
        .is_some_and(|value| truthy(Some(value)))
    {
        let user = status
            .get("retweeted_status_result")
            .and_then(|value| value.pointer("/result/core/user_results/result/legacy"))
            .filter(|value| value.is_object())
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let Some(retweeted) = status
            .get_mut("retweeted_status")
            .and_then(Value::as_object_mut)
        {
            retweeted.insert("user".into(), user);
        }
    }
    let bindings: Map<String, Value> = status
        .get("card")
        .and_then(|card| card.get("binding_values"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|value| value.is_object())
        .map(|binding| {
            (
                binding
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or("None")
                    .to_owned(),
                binding.get("value").cloned().unwrap_or(Value::Null),
            )
        })
        .collect();
    if !bindings.is_empty()
        && let Some(card) = status.get_mut("card").and_then(Value::as_object_mut)
    {
        card.insert("binding_values".into(), Value::Object(bindings));
    }
    if let Some(views) = int(result.pointer("/views/count"), 1) {
        status.insert("view_count".into(), views.into());
    }
    Ok(status)
}

/// `_generate_syndication_token(twid)`:
/// `((Number(twid) / 1e15) * Math.PI).toString(36).replace(/(0+|\.)/g, '')`.
fn syndication_token(twid: &str) -> String {
    let number = twid.parse::<u128>().unwrap_or_default() as f64;
    crate::jsinterp::js_number_to_string((number / 1e15) * std::f64::consts::PI, Some(36))
        .chars()
        .filter(|c| !matches!(c, '0' | '.'))
        .collect()
}

/// `_call_syndication_api(twid)`: the status transformed to the legacy shape.
fn call_syndication_api(client: &Client, twid: &str) -> Result<Value> {
    crate::screen::extractor_warning_once(&format!(
        "[{NAME}] {twid}: Not all metadata or media is available via syndication endpoint"
    ));
    let mut url = Url::parse("https://cdn.syndication.twimg.com/tweet-result")?;
    url.query_pairs_mut()
        .append_pair("id", twid)
        .append_pair("token", &syndication_token(twid));
    let mut status = super::download_json_note_headers(
        client,
        &url,
        NAME,
        twid,
        "Downloading syndication JSON",
        &[("User-Agent", "Googlebot")],
    )?;
    if !truthy(Some(&status)) {
        return Err(bug(
            NAME,
            twid,
            "Syndication endpoint returned empty JSON response",
        ));
    }
    let mut media = Vec::new();
    for source in [Some(&status), status.get("quoted_tweet")]
        .into_iter()
        .flatten()
    {
        for detail in source
            .get("mediaDetails")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
        {
            let mut detail = detail.clone();
            let media_id = detail
                .get("video_info")
                .and_then(|info| info.get("variants"))
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|variant| variant.get("url")?.as_str())
                .find_map(|url| Some(MEDIA_ID.captures(url)?[1].to_owned()))
                .unwrap_or_else(|| twid.to_owned());
            detail.insert("id_str".into(), media_id.into());
            media.push(Value::Object(detail));
        }
    }
    let Some(object) = status.as_object_mut() else {
        return Err(bug(
            NAME,
            twid,
            "An extractor error has occurred. (caused by TypeError('list indices must be integers or slices, not str'))",
        ));
    };
    object.insert("extended_entities".into(), json!({"media": media}));
    Ok(status)
}

fn truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(number)) => number.as_f64() != Some(0.0),
        Some(Value::String(text)) => !text.is_empty(),
        Some(Value::Array(items)) => !items.is_empty(),
        Some(Value::Object(object)) => !object.is_empty(),
    }
}

/// `_extract_variant_formats`.
fn variant_formats(
    client: &Client,
    extractor: &'static str,
    variant: &Value,
    id: &str,
    subtitles: &mut crate::Subtitles,
) -> Result<Vec<crate::VideoFormat>> {
    let Some(url) = variant
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
    else {
        return Ok(Vec::new());
    };
    if url.contains(".m3u8") {
        let Ok(manifest) = Url::parse(url) else {
            return Ok(Vec::new());
        };
        let Some(mut stream) =
            super::extract_m3u8_optional(client, &manifest, extractor, id, Some("hls"), true)
        else {
            return Ok(Vec::new());
        };
        for format in &mut stream.formats {
            if format.vcodec.as_deref() == Some("none")
                && format.tbr.is_none()
                && let Some(found) = AUDIO_BITRATE.captures(&format.format_id)
            {
                format.tbr = found["bitrate"]
                    .parse::<i64>()
                    .ok()
                    .map(|rate| (rate / 1000) as f64);
                crate::info_order::mark_format_update(format, "tbr");
                crate::info_order::mark_int_rates(format);
            }
        }
        super::html5::merge_subtitles(subtitles, stream.subtitles);
        return Ok(stream.formats);
    }
    let tbr = int(
        variant.get("bitrate").or_else(|| variant.get("bit_rate")),
        1000,
    )
    .filter(|tbr| *tbr != 0);
    let mut format = Map::new();
    format.insert("url".into(), url.into());
    format.insert(
        "format_id".into(),
        match tbr {
            Some(tbr) => format!("http-{tbr}"),
            None => "http".into(),
        }
        .into(),
    );
    if let Some(tbr) = tbr {
        format.insert("tbr".into(), tbr.into());
    }
    // Processing's `determine_ext(url)` (after the extractor's keys).
    format.insert(
        "ext".into(),
        super::direct::determine_ext(url)
            .unwrap_or_else(|| "unknown_video".into())
            .into(),
    );
    if let Some(found) = DIMENSIONS.captures(url) {
        format.insert(
            "width".into(),
            found["width"].parse::<i64>().unwrap_or(0).into(),
        );
        format.insert(
            "height".into(),
            found["height"].parse::<i64>().unwrap_or(0).into(),
        );
    }
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(format))
        .map_err(|error| fail_as(extractor, id, &error.to_string()))?;
    crate::info_order::mark_format(&mut format, "twitter-http");
    crate::info_order::mark_int_rates(&mut format);
    Ok(vec![format])
}

/// `_extract_formats_from_vmap_url(vmap_url, video_id)`.
fn vmap_formats(
    client: &Client,
    extractor: &'static str,
    vmap_url: Option<&str>,
    id: &str,
) -> Result<(Vec<crate::VideoFormat>, crate::Subtitles)> {
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    let Some(vmap_url) = vmap_url.and_then(super::json_ld::url_or_none) else {
        return Ok((formats, subtitles));
    };
    let text = super::download_xml(client, &Url::parse(&vmap_url)?, extractor, id)?;
    let document = roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| bug(extractor, id, &format!("Failed to parse XML ({error})")))?;
    let mut urls = Vec::new();
    for variant in document.descendants().filter(|node| {
        node.is_element()
            && node.tag_name().name() == "videoVariant"
            && node.tag_name().namespace() == Some(VMAP_NAMESPACE)
    }) {
        let mut attributes = Map::new();
        for attribute in variant.attributes() {
            attributes.insert(attribute.name().into(), attribute.value().into());
        }
        let url = percent_encoding::percent_decode_str(
            attributes
                .get("url")
                .and_then(Value::as_str)
                .ok_or_else(|| key_error(extractor, id, "url"))?,
        )
        .decode_utf8_lossy()
        .into_owned();
        attributes.insert("url".into(), url.clone().into());
        urls.push(url);
        formats.extend(variant_formats(
            client,
            extractor,
            &Value::Object(attributes),
            id,
            &mut subtitles,
        )?);
    }
    // `strip_or_none(xpath_text(vmap_data, './/MediaFile'))`.
    let video_url = document
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "MediaFile"
                && node.tag_name().namespace().is_none()
        })
        .and_then(|node| node.text())
        .map(|text| text.trim().to_owned());
    if let Some(video_url) = video_url
        && !urls.contains(&video_url)
    {
        formats.extend(variant_formats(
            client,
            extractor,
            &json!({"url": video_url}),
            id,
            &mut subtitles,
        )?);
    }
    Ok((formats, subtitles))
}

/// `extract_from_video_info`.
fn video_info(client: &Client, media: &Value, twid: &str) -> Result<Media> {
    let media_id = str_or_none(media.get("id_str")).or_else(|| str_or_none(media.get("id")));
    crate::screen::debug(&format!(
        "Extracting from video info: {}",
        media_id.as_deref().unwrap_or("None")
    ));
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for variant in media
        .pointer("/video_info/variants")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        formats.extend(variant_formats(
            client,
            NAME,
            variant,
            twid,
            &mut subtitles,
        )?);
    }
    let mut thumbnails = Vec::new();
    let media_url = media
        .get("media_url_https")
        .and_then(Value::as_str)
        .filter(|url| !url.is_empty())
        .or_else(|| {
            media
                .get("media_url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
        });
    if let Some(media_url) = media_url.and_then(|url| Url::parse(url).ok()) {
        let mut add = |name: &str, size: &Value| {
            let dimension = |short: &str, long: &str| {
                int(
                    size.get(short)
                        .filter(|value| truthy(Some(value)))
                        .or_else(|| size.get(long)),
                    1,
                )
            };
            thumbnails.push(json!({
                "id": name,
                "url": super::update_url_query(&media_url, &[("name", name)]).as_str(),
                "width": dimension("w", "width"),
                "height": dimension("h", "height"),
            }));
        };
        for (name, size) in media
            .get("sizes")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            add(name, size);
        }
        add("orig", media.get("original_info").unwrap_or(&json!({})));
    }
    let duration = media
        .pointer("/video_info/duration_millis")
        .and_then(|value| value.as_f64().or_else(|| value.as_str()?.parse().ok()))
        .map(|millis| millis / 1000.0);
    let mut fields = Map::new();
    fields.insert("id".into(), media_id.into());
    fields.insert("formats".into(), Value::Null);
    fields.insert("subtitles".into(), Value::Null);
    fields.insert("thumbnails".into(), Value::Array(thumbnails));
    // A `None` duration does not reach the output (processing drops it).
    if let Some(duration) = duration {
        fields.insert("duration".into(), json!(duration));
    }
    fields.insert(
        "_format_sort_fields".into(),
        json!(["res", "proto:m3u8", "br", "size"]),
    );
    Ok(Media::Video {
        fields,
        formats,
        subtitles,
    })
}

/// One of `selected_entries`.
enum Media {
    Video {
        fields: Map<String, Value>,
        formats: Vec<crate::VideoFormat>,
        subtitles: crate::Subtitles,
    },
    /// A card's `{'_type': 'url', 'url': ...}` (its `ie_key` is the
    /// extractor the registry finds for the URL).
    Url(String),
}

/// `get_binding_value(k)`.
fn binding(bindings: &Value, key: &str) -> Option<Value> {
    let value = bindings.get(key)?;
    let kind = value.get("type")?.as_str()?.to_lowercase();
    value.get(format!("{kind}_value")).cloned()
}

/// `extract_from_card_info`.
fn card_info(client: &Client, card: &Value, twid: &str) -> Result<Vec<Media>> {
    if !truthy(Some(card)) {
        return Ok(Vec::new());
    }
    let name = card.get("name").and_then(Value::as_str).unwrap_or_default();
    let name = name.rsplit(':').next().unwrap_or(name);
    crate::screen::debug(&format!(
        "Extracting from {name} card info: {}",
        card.get("url").and_then(Value::as_str).unwrap_or("None")
    ));
    let bindings = card.get("binding_values").cloned().unwrap_or(Value::Null);
    let text = |key: &str| {
        binding(&bindings, key)
            .and_then(|value| value.as_str().map(str::to_owned))
            .filter(|text| !text.is_empty())
    };
    match name {
        "player" => Ok(text("player_url").map(Media::Url).into_iter().collect()),
        "periscope_broadcast" => Ok(text("url")
            .or_else(|| text("player_url"))
            .map(Media::Url)
            .into_iter()
            .collect()),
        "broadcast" => Ok(text("broadcast_url").map(Media::Url).into_iter().collect()),
        "audiospace" => Ok(vec![Media::Url(format!(
            "https://twitter.com/i/spaces/{}",
            binding(&bindings, "id")
                .map(|value| match value {
                    Value::String(text) => text,
                    Value::Null => "None".into(),
                    other => other.to_string(),
                })
                .unwrap_or_else(|| "None".into())
        ))]),
        "summary" => Ok(text("card_url").map(Media::Url).into_iter().collect()),
        "unified_card" => {
            let unified: Value = text("unified_card")
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or(Value::Null);
            let mut found = Vec::new();
            for media in unified
                .get("media_entities")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|entities| entities.values())
                .filter(|media| media.get("type").and_then(Value::as_str) == Some("video"))
            {
                found.push(video_info(client, media, twid)?);
            }
            Ok(found)
        }
        // amplify, promo_video_website, promo_video_convo, appplayer, ...
        _ => {
            let amplify = name == "amplify";
            let vmap_url = if amplify {
                text("amplify_url_vmap")
            } else {
                text("player_stream_url")
            };
            let content_id = text(&format!(
                "{}_content_id",
                if amplify { name } else { "player" }
            ));
            let (formats, subtitles) = vmap_formats(
                client,
                NAME,
                vmap_url.as_deref(),
                content_id.as_deref().unwrap_or(twid),
            )?;
            let mut thumbnails = Vec::new();
            for suffix in ["_small", "", "_large", "_x_large", "_original"] {
                let image = binding(&bindings, &format!("player_image{suffix}"))
                    .filter(|image| truthy(Some(image)))
                    .unwrap_or_else(|| json!({}));
                let Some(image_url) = image
                    .get("url")
                    .and_then(Value::as_str)
                    .filter(|url| !url.is_empty() && !url.contains("/player-placeholder"))
                else {
                    continue;
                };
                thumbnails.push(json!({
                    "id": if suffix.is_empty() { "medium" } else { &suffix[1..] },
                    "url": image_url,
                    "width": periscope::int(image.get("width"), 1),
                    "height": periscope::int(image.get("height"), 1),
                }));
            }
            let mut fields = Map::new();
            fields.insert("formats".into(), Value::Null);
            fields.insert("subtitles".into(), Value::Null);
            fields.insert("thumbnails".into(), Value::Array(thumbnails));
            if let Some(duration) =
                periscope::int(binding(&bindings, "content_duration_seconds").as_ref(), 1)
            {
                fields.insert("duration".into(), duration.into());
            }
            Ok(vec![Media::Video {
                fields,
                formats,
                subtitles,
            }])
        }
    }
}

/// A video result in `{**info, **data, 'display_id': twid}` order.
fn video(
    url: &Url,
    dict: Map<String, Value>,
    formats: Vec<crate::VideoFormat>,
    subtitles: crate::Subtitles,
) -> VideoInfo {
    let mut info = periscope::build(url, (NAME, KEY), dict, formats, "id,url,width,height");
    info.subtitles = subtitles;
    info
}

/// `url_result(url)`, followed by the registry.
fn follow(target: &str) -> Result<ExtractResult> {
    let target = super::target_url(target)?;
    let mut info = super::direct::build_at(target.clone(), &target, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    Ok(ExtractResult::Video(info))
}

fn start(extractor: &'static str, url: &Url) {
    crate::screen::set_extractor(extractor);
    crate::screen::screen(&format!(
        "[{extractor}] Extracting URL: {}",
        crate::screen::extracting_url(&super::input_text(url))
    ));
}

/// `_extract_status(twid)`.
fn extract_status(client: &Client, twid: &str) -> Result<Value> {
    let api = selected_api();
    if !matches!(api.as_str(), "graphql" | "legacy" | "syndication") {
        return Err(fail(
            twid,
            &format!(
                "{} is not a valid API selection",
                crate::batch::py_repr(&api)
            ),
        ));
    }
    let attempt = || -> Result<Option<Value>> {
        if logged_in() || api == "graphql" {
            let data = call_graphql_api(client, NAME, GRAPHQL_ENDPOINT, twid, graphql_query(twid))?;
            Ok(Some(Value::Object(graphql_to_legacy(&data, twid)?)))
        } else if api == "legacy" {
            let query: Vec<(&str, String)> = vec![
                ("cards_platform", "Web-12".into()),
                ("include_cards", "1".into()),
                ("include_reply_count", "1".into()),
                ("include_user_entities", "0".into()),
                ("tweet_mode", "extended".into()),
            ];
            call_api(
                client,
                NAME,
                &format!("statuses/show/{twid}.json"),
                twid,
                &query,
                false,
            )
            .map(Some)
        } else {
            Ok(None)
        }
    };
    let mut status = match attempt() {
        Ok(status) => status,
        Err(error) if http_status(&error) == Some(429) => {
            crate::screen::warning(&format!(
                "[{NAME}] Rate-limit exceeded; falling back to syndication endpoint"
            ));
            Some(call_syndication_api(client, twid)?)
        }
        Err(error) => return Err(error),
    };
    if api == "syndication" {
        status = Some(call_syndication_api(client, twid)?);
    }
    let status = status.unwrap_or(Value::Null);
    Ok(status
        .get("retweeted_status")
        .filter(|value| value.is_object())
        .cloned()
        .or_else(|| status.is_object().then(|| status.clone()))
        .unwrap_or_else(|| json!({})))
}

impl Extractor for Twitter {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let twid = found["id"].to_owned();
        let selected = found.name("index").map(|index| index.as_str().to_owned());
        start(NAME, url);
        let status = extract_status(client, &twid)?;
        let description = ["full_text", "text"]
            .iter()
            .find_map(|key| status.get(*key).and_then(Value::as_str))
            .map(|text| text.replace('\n', " "))
            .filter(|text| !text.is_empty())
            .unwrap_or_default();
        let stripped = URL_JUNK.replace_all(&description, "");
        let chars: Vec<char> = stripped.chars().collect();
        let mut title = if chars.len() <= 72 {
            stripped.into_owned()
        } else {
            format!("{}...", chars[..69].iter().collect::<String>())
        };
        let user = status.get("user").cloned().unwrap_or_else(|| json!({}));
        let uploader = user
            .get("name")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty());
        if let Some(uploader) = uploader {
            title = format!("{uploader} - {title}");
        }
        let uploader_id = user.get("screen_name").and_then(Value::as_str);
        let mut info = Map::new();
        info.insert("id".into(), twid.clone().into());
        info.insert("title".into(), title.clone().into());
        info.insert("description".into(), description.into());
        info.insert(
            "uploader".into(),
            user.get("name").cloned().unwrap_or(Value::Null),
        );
        info.insert(
            "timestamp".into(),
            status
                .get("created_at")
                .and_then(Value::as_str)
                .and_then(|date| crate::dates::unified_timestamp(date, false))
                .into(),
        );
        info.insert(
            "channel_id".into(),
            str_or_none(status.get("user_id_str"))
                .or_else(|| str_or_none(user.get("id_str")))
                .into(),
        );
        info.insert("uploader_id".into(), uploader_id.into());
        info.insert(
            "uploader_url".into(),
            uploader_id
                .map(|id| format!("https://twitter.com/{id}"))
                .into(),
        );
        for (key, source) in [
            ("view_count", "view_count"),
            ("like_count", "favorite_count"),
            ("repost_count", "retweet_count"),
            ("comment_count", "reply_count"),
        ] {
            info.insert(key.into(), int(status.get(source), 1).into());
        }
        info.insert(
            "age_limit".into(),
            if truthy(status.get("possibly_sensitive")) {
                18
            } else {
                0
            }
            .into(),
        );
        let tags: Vec<Value> = status
            .pointer("/entities/hashtags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| tag.get("text").cloned())
            .collect();
        info.insert("tags".into(), Value::Array(tags));

        let videos: Vec<Value> = [Some(&status), status.get("quoted_status")]
            .into_iter()
            .flatten()
            .filter_map(|source| source.pointer("/extended_entities/media"))
            .filter_map(Value::as_array)
            .flatten()
            .filter(|media| {
                media.is_object() && media.get("type").and_then(Value::as_str) != Some("photo")
            })
            .cloned()
            .collect();
        let archive_id = json!([format!("twitter {twid}")]);

        // `_yes_playlist(twid, selected_index, video_label=...)`.
        let all = match &selected {
            None => true,
            Some(index) if super::no_playlist() => {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading just the URL-specified video number {index} because of --no-playlist"
                ));
                false
            }
            Some(index) => {
                crate::screen::screen(&format!(
                    "[{NAME}] Downloading playlist {twid} - add --no-playlist to download just the URL-specified video number {index}"
                ));
                true
            }
        };
        if !all {
            let index: usize = selected.as_deref().unwrap_or("1").parse().unwrap_or(0);
            let all_media: Vec<&Value> = [Some(&status), status.get("quoted_status")]
                .into_iter()
                .flatten()
                .filter_map(|source| source.pointer("/extended_entities/media"))
                .filter_map(Value::as_array)
                .filter_map(|media| media.get(index.wrapping_sub(1)))
                .filter(|media| media.is_object())
                .collect();
            let Some(desired) = all_media.first().copied() else {
                return Err(fail(&twid, &format!("Video #{index} is unavailable")));
            };
            if desired.get("type").and_then(Value::as_str) != Some("video") {
                return Err(fail(&twid, &format!("Media #{index} is not a video")));
            }
            for (position, entry) in videos.iter().enumerate() {
                if entry.get("id") != desired.get("id") {
                    continue;
                }
                if position == 0 {
                    info.insert("_old_archive_ids".into(), archive_id.clone());
                }
                if videos.len() != 1 {
                    title = format!("{title} #{}", position + 1);
                    info.insert("title".into(), title.clone().into());
                }
                break;
            }
            let Media::Video {
                fields,
                formats,
                subtitles,
            } = video_info(client, desired, &twid)?
            else {
                unreachable!("video info is a video");
            };
            let mut dict = info;
            dict.extend(fields);
            dict.insert("display_id".into(), twid.clone().into());
            return Ok(Some(ExtractResult::Video(video(
                url, dict, formats, subtitles,
            ))));
        }

        let mut selected_entries = Vec::new();
        for media in &videos {
            selected_entries.push(video_info(client, media, &twid)?);
        }
        if let Some(card) = status.get("card") {
            selected_entries.extend(card_info(client, card, &twid)?);
        }
        if selected_entries.is_empty() {
            let expanded = status
                .pointer("/entities/urls/0/expanded_url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none);
            return match expanded {
                // `url_result(expanded_url, display_id=twid, **info)`: a
                // `url` result, whose fields processing does not keep.
                Some(expanded) if expanded != url.as_str() => follow(&expanded).map(Some),
                // `raise_no_formats(..., expected=True)`, then `return info`.
                _ => {
                    crate::no_formats::raise_expected(
                        NAME,
                        None,
                        Some(&twid),
                        "No video could be found in this tweet",
                    )?;
                    Ok(Some(ExtractResult::Video(video(
                        url,
                        info,
                        Vec::new(),
                        Default::default(),
                    ))))
                }
            };
        }
        if selected_entries.len() == 1
            && let Some(Media::Url(target)) = selected_entries.first()
        {
            return follow(target).map(Some);
        }
        if selected_entries
            .iter()
            .any(|entry| matches!(entry, Media::Url(_)))
        {
            return Err(Error::UnsupportedHtml5(
                "tweets mixing videos and linked cards are not supported natively".into(),
            ));
        }
        let several = selected_entries.len() > 1;
        let mut entries = Vec::new();
        for (index, entry) in selected_entries.into_iter().enumerate() {
            let Media::Video {
                fields,
                formats,
                subtitles,
            } = entry
            else {
                continue;
            };
            let mut dict = info.clone();
            dict.extend(fields);
            dict.insert("display_id".into(), twid.clone().into());
            if index == 0 {
                dict.insert("_old_archive_ids".into(), archive_id.clone());
            }
            if several {
                dict.insert("title".into(), format!("{title} #{}", index + 1).into());
            }
            entries.push(video(url, dict, formats, subtitles));
        }
        if !several {
            return Ok(entries.pop().map(ExtractResult::Video));
        }
        // `playlist_result(entries, **info)`.
        let keys: Vec<String> = info.keys().cloned().collect();
        let mut extra = info;
        extra.shift_remove("id");
        extra.shift_remove("title");
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            format!("playlist-dict;{}", keys.join(",")).into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: twid,
            title,
            webpage_url: url.clone(),
            extractor: NAME.into(),
            extractor_key: KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

impl Extractor for TwitterCard {
    fn key(&self) -> &'static str {
        CARD_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CARD_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        start(CARD_NAME, url);
        follow(&format!("https://twitter.com/statuses/{}", &found["id"])).map(Some)
    }
}

impl Extractor for TwitterAmplify {
    fn key(&self) -> &'static str {
        AMPLIFY_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = AMPLIFY_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        start(AMPLIFY_NAME, url);
        let webpage = super::download_webpage(client, url, AMPLIFY_NAME, &video_id)?;
        // `_html_search_meta(name, webpage, display_name, fatal=False)`.
        let meta = |name: &str, display: &str| {
            let found = crate::html::html_search_meta(&webpage, name);
            if found.is_none() {
                crate::screen::warning(&format!(
                    "[{AMPLIFY_NAME}] {video_id}: unable to extract {display}; {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
            }
            found
        };
        let vmap_url = meta("twitter:amplify:vmap", "vmap url");
        let (mut formats, _) = vmap_formats(client, AMPLIFY_NAME, vmap_url.as_deref(), &video_id)?;
        let thumbnail = meta("twitter:image:src", "thumbnail");
        let dimension = |target: &str| {
            let get = |side: &str| {
                let name = format!("twitter:{target}:{side}");
                meta(&name, &name).and_then(|value| value.trim().parse::<i64>().ok())
            };
            (get("width"), get("height"))
        };
        let mut thumbnails = Vec::new();
        if let Some(thumbnail) = thumbnail.filter(|thumbnail| !thumbnail.is_empty()) {
            let (width, height) = dimension("image");
            thumbnails.push(json!({"url": thumbnail, "width": width, "height": height}));
        }
        let (width, height) = dimension("player");
        let Some(first) = formats.first_mut() else {
            return Err(bug(
                AMPLIFY_NAME,
                &video_id,
                "An extractor error has occurred. (caused by IndexError('list index out of range'))",
            ));
        };
        first.width = width.and_then(|value| u32::try_from(value).ok());
        first.height = height.and_then(|value| u32::try_from(value).ok());
        crate::info_order::mark_format_update(first, "width");
        crate::info_order::mark_format_update(first, "height");
        let mut dict = Map::new();
        dict.insert("id".into(), video_id.into());
        dict.insert("title".into(), "Twitter Video".into());
        dict.insert("formats".into(), Value::Null);
        dict.insert("thumbnails".into(), Value::Array(thumbnails));
        Ok(Some(ExtractResult::Video(periscope::build(
            url,
            (AMPLIFY_NAME, "TwitterAmplify"),
            dict,
            formats,
            "url,width,height",
        ))))
    }
}

/// `url_result(url, TwitterIE)` as a playlist entry.
fn status_entry(target: &str) -> Result<VideoInfo> {
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
            "ie_key": KEY,
            "extractor": NAME,
            "entry": {"ie_key": KEY, "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

impl Extractor for TwitterBroadcast {
    fn key(&self) -> &'static str {
        BROADCAST_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = BROADCAST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        start(BROADCAST_NAME, url);
        let broadcast_id = if &found["type"] == "events" {
            let timeline = call_api(
                client,
                BROADCAST_NAME,
                &format!("live_event/1/{display_id}/timeline.json"),
                &display_id,
                &[],
                false,
            )?;
            let objects = timeline
                .get("twitter_objects")
                .filter(|value| value.is_object())
                .cloned()
                .unwrap_or(Value::Null);
            let tweet_ids: Vec<String> = objects
                .get("tweets")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|tweets| tweets.values())
                .filter_map(|tweet| tweet.get("id_str")?.as_str().map(str::to_owned))
                .collect();
            if !tweet_ids.is_empty() {
                let title = objects
                    .get("live_events")
                    .and_then(|events| events.get(&display_id))
                    .and_then(|event| event.get("title"))
                    .and_then(Value::as_str)
                    .map(crate::html::clean_html)
                    .filter(|title| !title.is_empty());
                let entries = tweet_ids
                    .iter()
                    .map(|id| status_entry(&format!("https://twitter.com/i/web/status/{id}")))
                    .collect::<Result<Vec<_>>>()?;
                let mut extra = Map::new();
                extra.insert(
                    crate::info_order::INFO_SHAPE.into(),
                    format!(
                        "playlist-dict;{}",
                        if title.is_some() { "id,title" } else { "id" }
                    )
                    .into(),
                );
                if title.is_none() {
                    extra.insert("title".into(), Value::Null);
                }
                return Ok(Some(ExtractResult::Playlist(PlaylistInfo {
                    id: display_id,
                    title: title.unwrap_or_default(),
                    webpage_url: url.clone(),
                    extractor: BROADCAST_NAME.into(),
                    extractor_key: "TwitterBroadcast".into(),
                    nested: Vec::new(),
                    pending: None,
                    entries,
                    extra,
                })));
            }
            objects
                .get("broadcasts")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|broadcasts| broadcasts.values())
                .flat_map(|broadcast| ["id", "broadcast_id"].map(|key| broadcast.get(key)))
                .find_map(|value| value?.as_str().map(str::to_owned))
                .ok_or_else(|| {
                    bug(
                        BROADCAST_NAME,
                        &display_id,
                        "Unable to extract broadcast ID",
                    )
                })?
        } else {
            display_id.clone()
        };
        let response = call_api(
            client,
            BROADCAST_NAME,
            "broadcasts/show.json",
            &broadcast_id,
            &[("ids", broadcast_id.clone())],
            false,
        )?;
        let broadcast = response
            .get("broadcasts")
            .ok_or_else(|| key_error(BROADCAST_NAME, &display_id, "broadcasts"))?
            .get(&broadcast_id)
            .ok_or_else(|| key_error(BROADCAST_NAME, &display_id, &broadcast_id))?;
        if !truthy(Some(broadcast)) {
            return Err(fail_as(
                BROADCAST_NAME,
                &display_id,
                "Broadcast no longer exists",
            ));
        }
        let mut info = periscope::parse_broadcast_data(broadcast, &broadcast_id);
        info.insert("display_id".into(), display_id.clone().into());
        let title = broadcast
            .get("status")
            .and_then(Value::as_str)
            .map(crate::html::clean_html)
            .filter(|title| !title.is_empty())
            .unwrap_or_else(|| {
                [
                    info.get("uploader")
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty()),
                    Some("Twitter Broadcast"),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" - ")
            });
        info.insert("title".into(), title.into());
        let twitter_username = broadcast
            .get("twitter_username")
            .and_then(Value::as_str)
            .filter(|name| !name.is_empty());
        let uploader_id = twitter_username
            .map(Value::from)
            .or_else(|| info.get("uploader_id").cloned())
            .unwrap_or(Value::Null);
        info.insert("uploader_id".into(), uploader_id);
        info.insert(
            "uploader_url".into(),
            twitter_username
                .map(|name| format!("https://twitter.com/{name}"))
                .into(),
        );
        if info.get("live_status").and_then(Value::as_str) == Some("is_upcoming") {
            // `raise_no_formats(..., expected=True)`, then `return info`.
            crate::no_formats::raise_expected(
                BROADCAST_NAME,
                None,
                Some(&display_id),
                "This live broadcast has not yet started",
            )?;
            return Ok(Some(ExtractResult::Video(periscope::build(
                url,
                (BROADCAST_NAME, "TwitterBroadcast"),
                info,
                Vec::new(),
                "url",
            ))));
        }
        let media_key = broadcast
            .get("media_key")
            .ok_or_else(|| key_error(BROADCAST_NAME, &display_id, "media_key"))?;
        let media_key = str_or_none(Some(media_key)).unwrap_or_default();
        let stream = call_api(
            client,
            BROADCAST_NAME,
            &format!("live_video_stream/status/{media_key}"),
            &media_key,
            &[],
            false,
        )?;
        let source = stream
            .get("source")
            .ok_or_else(|| key_error(BROADCAST_NAME, &display_id, "source"))?;
        let m3u8_url = source
            .get("noRedirectPlaybackUrl")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
            .map(str::to_owned)
            .or_else(|| source.get("location")?.as_str().map(str::to_owned))
            .ok_or_else(|| key_error(BROADCAST_NAME, &display_id, "location"))?;
        if m3u8_url.contains("/live_video_stream/geoblocked/") {
            return Err(crate::geo::restricted(
                &fail_as(
                    BROADCAST_NAME,
                    &display_id,
                    "This video is not available from your location due to geo restriction",
                )
                .to_string(),
                &[],
            ));
        }
        let manifest = Url::parse(&m3u8_url)?;
        let m3u8_id = manifest
            .query_pairs()
            .find(|(key, value)| key == "type" && !value.is_empty())
            .map(|(_, value)| value.into_owned());
        let common = periscope::common_format_info(broadcast, BROADCAST_NAME, &display_id)?;
        let formats = periscope::pscp_m3u8_formats(
            client,
            &manifest,
            BROADCAST_NAME,
            &broadcast_id,
            m3u8_id.as_deref(),
            (&common.0, common.1, common.2),
            true,
        )?;
        info.insert("formats".into(), Value::Null);
        Ok(Some(ExtractResult::Video(periscope::build(
            url,
            (BROADCAST_NAME, "TwitterBroadcast"),
            info,
            formats,
            "url",
        ))))
    }
}

impl Extractor for TwitterSpaces {
    fn key(&self) -> &'static str {
        SPACES_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SPACES_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let space_id = found["id"].to_owned();
        start(SPACES_NAME, url);
        let data = call_graphql_api(
            client,
            SPACES_NAME,
            SPACES_ENDPOINT,
            &space_id,
            spaces_query(&space_id),
        )?;
        let space = data
            .get("audioSpace")
            .ok_or_else(|| key_error(SPACES_NAME, &space_id, "audioSpace"))?;
        if !truthy(Some(space)) {
            return Err(fail_as(SPACES_NAME, &space_id, "Twitter Space not found"));
        }
        let metadata = space
            .get("metadata")
            .ok_or_else(|| key_error(SPACES_NAME, &space_id, "metadata"))?;
        let live_status =
            metadata
                .get("state")
                .and_then(Value::as_str)
                .and_then(|state| match state.to_lowercase().as_str() {
                    "notstarted" => Some("is_upcoming"),
                    "ended" => Some("was_live"),
                    "running" => Some("is_live"),
                    "timedout" => Some("post_live"),
                    _ => None,
                });
        let is_live = live_status == Some("is_live");
        const REFERER: &str = "https://twitter.com/";
        let mut formats = Vec::new();
        if live_status == Some("is_upcoming") {
            crate::no_formats::raise_expected(
                SPACES_NAME,
                None,
                Some(&space_id),
                "Twitter Space not started yet",
            )?;
        } else if !is_live && !truthy(metadata.get("is_space_available_for_replay")) {
            crate::no_formats::raise_expected(
                SPACES_NAME,
                None,
                Some(&space_id),
                "Twitter Space ended and replay is disabled",
            )?;
        } else if let Some(media_key) = metadata.get("media_key").filter(|key| truthy(Some(key))) {
            let media_key = str_or_none(Some(media_key)).unwrap_or_default();
            let stream = call_api(
                client,
                SPACES_NAME,
                &format!("live_video_stream/status/{media_key}"),
                &media_key,
                &[],
                false,
            )?;
            let source = ["noRedirectPlaybackUrl", "location"]
                .iter()
                .filter_map(|key| stream.get("source")?.get(*key)?.as_str())
                .find_map(super::json_ld::url_or_none);
            if let Some(source) = source {
                let audio = source.contains("audio-space");
                formats = periscope::m3u8_formats(
                    client,
                    &Url::parse(&source)?,
                    SPACES_NAME,
                    &media_key,
                    if audio {
                        ("m4a", "m3u8")
                    } else {
                        ("mp4", "m3u8_native")
                    },
                    None,
                    is_live,
                    false,
                    REFERER,
                )?;
                if audio {
                    for format in &mut formats {
                        format.vcodec = Some("none".into());
                        format.acodec = Some("aac".into());
                        crate::info_order::mark_format_update(format, "vcodec");
                        crate::info_order::mark_format_update(format, "acodec");
                        if !is_live {
                            format.container = Some("m4a_dash".into());
                            crate::info_order::mark_format_update(format, "container");
                        }
                    }
                }
            }
        }
        let participants: Vec<&str> = space
            .pointer("/participants/speakers")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|speaker| speaker.get("display_name")?.as_str())
            .collect();
        let participants = if participants.is_empty() {
            "nobody yet".to_owned()
        } else {
            participants.join(", ")
        };
        if formats.is_empty() && live_status == Some("post_live") {
            crate::no_formats::raise_expected(
                SPACES_NAME,
                None,
                Some(&space_id),
                "Twitter Space ended but not downloadable yet",
            )?;
        }
        let mut dict = Map::new();
        dict.insert("id".into(), space_id.clone().into());
        dict.insert(
            "description".into(),
            format!("Twitter Space participated by {participants}").into(),
        );
        dict.insert("formats".into(), Value::Null);
        dict.insert("http_headers".into(), json!({"Referer": REFERER}));
        dict.insert("live_status".into(), live_status.into());
        if let Some(title) = metadata.get("title").and_then(Value::as_str) {
            dict.insert("title".into(), title.into());
        }
        if let Some(release) = ["started_at", "scheduled_start"]
            .iter()
            .find_map(|key| periscope::int(metadata.get(*key), 1000))
        {
            dict.insert("release_timestamp".into(), release.into());
        }
        if let Some(timestamp) = periscope::int(metadata.get("created_at"), 1000) {
            dict.insert("timestamp".into(), timestamp.into());
        }
        let creator = metadata.pointer("/creator_results/result/legacy");
        if let Some(creator) = creator.filter(|value| value.is_object()) {
            if let Some(name) = creator.get("name").and_then(Value::as_str) {
                dict.insert("uploader".into(), name.into());
            }
            if let Some(screen_name) = str_or_none(creator.get("screen_name")) {
                dict.insert("uploader_id".into(), screen_name.into());
            }
            if let Some(thumbnail) = creator
                .get("profile_image_url_https")
                .and_then(Value::as_str)
                .map(|url| url.replace("_normal", "_400x400"))
                .and_then(|url| super::json_ld::url_or_none(&url))
            {
                dict.insert("thumbnail".into(), thumbnail.into());
            }
        }
        Ok(Some(ExtractResult::Video(periscope::build(
            url,
            (SPACES_NAME, "TwitterSpaces"),
            dict,
            formats,
            "url",
        ))))
    }
}

impl Extractor for TwitterShortener {
    fn key(&self) -> &'static str {
        SHORTENER_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let input = super::input_text(url);
        let Some(found) = SHORTENER_URL.captures(&input) else {
            return Ok(None);
        };
        start(SHORTENER_NAME, url);
        let (shortcode, target) = match found.name("eid") {
            Some(eid) => (
                eid.as_str().to_owned(),
                Url::parse(&format!("https://t.co/{}", eid.as_str()))?,
            ),
            None => (found["id"].to_owned(), url.clone()),
        };
        crate::screen::screen(&format!(
            "[{SHORTENER_NAME}] {shortcode}: Downloading webpage"
        ));
        let response =
            super::direct::request_with_headers(client, &target, None, &[("User-Agent", "curl")])
                .map_err(|error| match error {
                Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                    extractor: SHORTENER_NAME,
                    id: shortcode.clone(),
                    what: "webpage",
                    source: Box::new(error),
                },
                error => error,
            })?;
        const UNSAFE_LINK: &str = "https://twitter.com/safety/unsafe_link_warning?unsafe_link=";
        let new_url = response.url().as_str().replace(UNSAFE_LINK, "");
        follow(&new_url).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://twitter.com/freethenipple/status/643211948184596480"
        ));
        assert!(suitable("https://x.com/i/web/status/910031516746514432"));
        assert!(suitable("https://mobile.twitter.com/a/status/1/video/2"));
        assert!(!suitable("https://twitter.com/freethenipple"));
        assert!(card_suitable(
            "https://twitter.com/i/cards/tfw/v1/560070183650213889"
        ));
        assert!(card_suitable(
            "https://twitter.com/i/videos/tweet/705235433198714880"
        ));
        assert!(amplify_suitable(
            "https://amp.twimg.com/v/0ba0c3c7-0af3-4c0a-bed5-7efd1ffa2951"
        ));
        assert!(broadcast_suitable(
            "https://twitter.com/i/broadcasts/1yNGaQLWpejGj"
        ));
        assert!(broadcast_suitable(
            "https://x.com/i/events/1910629646300762112"
        ));
        assert!(spaces_suitable(
            "https://twitter.com/i/spaces/1OwxWwQOPlNxQ"
        ));
        assert!(!spaces_suitable("https://twitter.com/i/spaces/short"));
        assert!(shortener_suitable("https://t.co/AbC123"));
        assert!(shortener_suitable("tco:AbC123"));
        assert!(!shortener_suitable("https://t.co/"));
    }

    /// Values from upstream `TwitterIE._generate_syndication_token`.
    #[test]
    fn syndication_tokens_follow_upstream() {
        for (twid, token) in [
            ("1001", "8vdna2e11tp"),
            ("20", "6dq1a2xwd93"),
            ("1577719286659006464", "3tojuiek1l"),
            ("1910629646300762112", "4mqf4ecf3un"),
            ("1006", "8wz2sdwriwr"),
        ] {
            assert_eq!(syndication_token(twid), token, "{twid}");
        }
        assert_eq!(
            crate::jsinterp::js_number_to_string(255.5, Some(16)),
            "ff.8"
        );
        assert_eq!(
            crate::jsinterp::js_number_to_string(-0.25, Some(2)),
            "-0.01"
        );
    }
}
