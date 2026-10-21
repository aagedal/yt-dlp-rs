//! Upstream `YoutubeIE._real_extract`'s acquisition: the watch page, its
//! `ytcfg`, initial data and player response, then one player API request
//! per requested InnerTube client (`_extract_player_responses`), whose
//! responses feed [`super::streams`].
use super::{challenge, innertube, streams};
use crate::{Error, Result};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

/// `_DEFAULT_CLIENTS`, `_DEFAULT_JSLESS_CLIENTS` and `_DEFAULT_AUTHED_CLIENTS`.
const DEFAULT_CLIENTS: [&str; 3] = ["tv_samsung", "visionos", "web"];
const DEFAULT_JSLESS_CLIENTS: [&str; 1] = ["visionos"];
const DEFAULT_AUTHED_CLIENTS: [&str; 3] = ["web_embedded", "tv_samsung", "web"];
const DEFAULT_PREMIUM_CLIENTS: [&str; 3] = ["web_creator", "tv_samsung", "web"];
const WEBPAGE_CLIENT: &str = "web";
const EJS_WIKI_URL: &str = "https://github.com/yt-dlp/yt-dlp/wiki/EJS";
/// `_PLAYER_JS_VARIANT_MAP['main']`.
const MAIN_PLAYER_VARIANT: &str = "player_ias.vflset/en_US/base.js";

/// `_youtube_login_hint`.
pub(super) const LOGIN_HINT: &str = "Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies. Also see  https://github.com/yt-dlp/yt-dlp/wiki/Extractors#exporting-youtube-cookies  for tips on effectively exporting YouTube cookies";

/// `_split_innertube_client`: the client, its base client and variant
/// (`web_embedded.tv` is `web_embedded` for base `tv`).
fn split_client(name: &str) -> (&str, &str, Option<&str>) {
    if let Some((variant, base)) = name.rsplit_once('.') {
        return (variant, base, Some(variant));
    }
    match name.split_once('_') {
        Some((base, variant)) => (name, base, Some(variant)),
        None => (name, name, None),
    }
}

fn playability_status(response: &Value) -> Option<&str> {
    response
        .get("playabilityStatus")
        .and_then(|status| status.get("status"))
        .and_then(Value::as_str)
}

/// `_is_agegated`.
fn is_agegated(response: &Value) -> bool {
    let status = response.get("playabilityStatus");
    if status
        .and_then(|status| status.get("desktopLegacyAgeGateReason"))
        .is_some_and(|reason| super::tab::truthy(Some(reason)))
    {
        return true;
    }
    const REASONS: [&str; 5] = [
        "confirm your age",
        "age-restricted",
        "inappropriate",
        "age_verification_required",
        "age_check_required",
    ];
    ["status", "reason"].iter().any(|key| {
        status
            .and_then(|status| status.get(*key))
            .and_then(Value::as_str)
            .is_some_and(|reason| REASONS.iter().any(|expected| reason.contains(expected)))
    })
}

/// `_download_ytcfg`: the `ytcfg` of a client's own page, for the clients
/// that have one (`{}` otherwise or when the page fails).
fn download_ytcfg(
    client: &Client,
    config: &innertube::ClientConfig,
    video_id: &str,
) -> Map<String, Value> {
    let url = match config.name {
        "mweb" => "https://m.youtube.com".to_owned(),
        "web" | "web_safari" => "https://www.youtube.com".to_owned(),
        "web_music" => "https://music.youtube.com".to_owned(),
        "web_creator" => "https://studio.youtube.com".to_owned(),
        "web_embedded" => format!("https://www.youtube.com/embed/{video_id}?html5=1"),
        "tv" => "https://www.youtube.com/tv".to_owned(),
        _ => return Map::new(),
    };
    if config.require_auth && !super::has_auth_cookies() {
        return Map::new();
    }
    let Ok(url) = Url::parse(&url) else {
        return Map::new();
    };
    let default = default_ytcfg(config);
    let context = default.get("INNERTUBE_CONTEXT");
    let text = |path: &[&str]| {
        let mut value = context?;
        for key in path {
            value = value.get(*key)?;
        }
        value.as_str()
    };
    let mut headers = Vec::new();
    if let Some(user_agent) = text(&["client", "userAgent"]) {
        headers.push(("User-Agent", user_agent));
    }
    if let Some(referer) = text(&["thirdParty", "embedUrl"]) {
        headers.push(("Referer", referer));
    }
    let note = format!(
        "Downloading {} client config",
        config.name.replace('_', " ").trim()
    );
    // `_download_webpage_with_retries(fatal=False)`: a warning, no ytcfg.
    let webpage = match super::super::download_webpage_note_headers(
        client, &url, "youtube", video_id, &note, &headers,
    ) {
        Ok(webpage) => webpage,
        Err(error) => {
            crate::screen::warning(&crate::upstream_error_text(&error));
            return Map::new();
        }
    };
    let mut ytcfg = extract_ytcfg(&webpage);
    // `_fix_embedded_ytcfg`.
    if split_client(config.name).2 == Some("embedded")
        && let Some(Value::Object(context)) = ytcfg.get_mut("INNERTUBE_CONTEXT")
    {
        let third_party = context.entry("thirdParty").or_insert_with(|| json!({}));
        if let Some(third_party) = third_party.as_object_mut() {
            third_party.insert("embedUrl".into(), "https://www.reddit.com/".into());
        }
    }
    if config.name == "tv"
        && let Some(Value::Object(info)) = ytcfg
            .get_mut("INNERTUBE_CONTEXT")
            .and_then(|context| context.get_mut("client"))
            .and_then(|client| client.get_mut("configInfo"))
    {
        info.shift_remove("appInstallData");
    }
    ytcfg
}

/// `_load_player(fatal=False)`: the player JS, downloaded once per
/// `_player_js_cache_key` (`PLAYER_ID-VARIANT`) and process.
pub(super) fn load_player(client: &Client, video_id: &str, player_url: &str) -> Option<String> {
    static CODE: LazyLock<std::sync::Mutex<std::collections::HashMap<String, String>>> =
        LazyLock::new(Default::default);
    let key = player_url
        .split("/s/player/")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .map(|id| format!("{id}-main"))
        .unwrap_or_default();
    if let Some(code) = CODE.lock().ok()?.get(&key) {
        return Some(code.clone());
    }
    crate::screen::screen(&format!("[youtube] {video_id}: Downloading player {key}"));
    let code = Url::parse(player_url)
        .map_err(Error::from)
        .and_then(|url| super::super::direct::request(client, &url, None))
        .and_then(|response| response.text().map_err(Error::Http));
    match code {
        Ok(code) => {
            if let Ok(mut cache) = CODE.lock() {
                cache.insert(key, code.clone());
            }
            Some(code)
        }
        Err(_) => {
            crate::screen::warning(&format!("[youtube] {video_id}: Download of {key} failed"));
            None
        }
    }
}

/// `_extract_signature_timestamp(fatal=False)`: the `ytcfg`'s `STS`, else
/// the player JS's.
fn signature_timestamp(
    client: &Client,
    video_id: &str,
    player_url: &str,
    ytcfg: &Map<String, Value>,
) -> Option<i64> {
    if let Some(sts) = ytcfg
        .get("STS")
        .and_then(|sts| sts.as_i64().or_else(|| sts.as_str()?.parse().ok()))
        .filter(|sts| *sts != 0)
    {
        return Some(sts);
    }
    static STS: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?:signatureTimestamp|sts)\s*:\s*(?P<sts>[0-9]{5})").unwrap()
    });
    let code = load_player(client, video_id, player_url)?;
    match STS.captures(&code) {
        Some(found) => found["sts"].parse().ok(),
        None => {
            super::super::search_warning("youtube", "JS player signature timestamp");
            None
        }
    }
}

/// Whether `_report_pot_format_skipped` only writes a debug message.
pub(super) fn is_default_client(client: &str) -> bool {
    DEFAULT_CLIENTS.contains(&client) || DEFAULT_AUTHED_CLIENTS.contains(&client)
}

/// `extract_ytcfg`: `ytcfg\.set\s*\(\s*({.+?})\s*\)\s*;`.
pub(super) fn extract_ytcfg(webpage: &str) -> Map<String, Value> {
    static YTCFG: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?s)ytcfg\.set\s*\(\s*(\{.+?\})\s*\)\s*;").unwrap());
    YTCFG
        .captures(webpage)
        .and_then(|found| serde_json::from_str::<Value>(&found[1]).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

/// `_search_json(START, webpage, NAME, video_id, fatal=False)`.
fn search_initial(webpage: &str, start: &Regex, name: &str) -> Option<Value> {
    match super::super::search_json(start, webpage, false) {
        Some(Some(value)) => Some(value),
        Some(None) => {
            crate::screen::warning(&format!(
                "[youtube] Unable to extract {name} - Failed to parse JSON"
            ));
            None
        }
        None => {
            super::super::search_warning("youtube", name);
            None
        }
    }
}

/// The default `ytcfg` of a client (`_get_default_ytcfg`) as JSON.
fn default_ytcfg(client: &innertube::ClientConfig) -> Map<String, Value> {
    let mut ytcfg = Map::new();
    let context: Value = serde_json::from_str(client.context).unwrap_or(Value::Null);
    ytcfg.insert("INNERTUBE_CONTEXT".into(), context);
    ytcfg.insert(
        "INNERTUBE_CONTEXT_CLIENT_NAME".into(),
        client.client_name_id.into(),
    );
    ytcfg
}

/// `_ytcfg_get_safe`: a `ytcfg` value, else the client default's.
fn ytcfg_value<'a>(
    ytcfg: &'a Map<String, Value>,
    default: &'a Map<String, Value>,
    path: &[&str],
) -> Option<&'a Value> {
    let get = |map: &'a Map<String, Value>| {
        let mut value = map.get(path[0])?;
        for key in &path[1..] {
            value = value.get(*key)?;
        }
        Some(value).filter(|value| match value {
            Value::Null => false,
            Value::String(text) => !text.is_empty(),
            Value::Number(number) => number.as_f64() != Some(0.0),
            _ => true,
        })
    };
    get(ytcfg).or_else(|| get(default))
}

/// `_extract_player_url`: `PLAYER_JS_URL` (else a `jsUrl`) under
/// `_construct_player_url`'s `main` variant.
fn player_url(ytcfgs: &[&Map<String, Value>]) -> Option<String> {
    let found = ytcfgs
        .iter()
        .find_map(|ytcfg| ytcfg.get("PLAYER_JS_URL").and_then(Value::as_str))
        .or_else(|| {
            ytcfgs.iter().find_map(|ytcfg| {
                ytcfg
                    .get("WEB_PLAYER_CONTEXT_CONFIGS")?
                    .as_object()?
                    .values()
                    .find_map(|config| config.get("jsUrl")?.as_str())
            })
        })?;
    static PLAYER_ID: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"/s/player/(?P<id>[a-fA-F0-9]{8,})/").unwrap());
    let id = PLAYER_ID.captures(found)?["id"].to_owned();
    let base = Url::parse("https://www.youtube.com").ok()?;
    let path = found
        .split_once(&format!("/s/player/{id}/"))
        .map_or("", |(_, path)| path);
    if path == MAIN_PLAYER_VARIANT {
        return base.join(found).ok().map(String::from);
    }
    base.join(&format!("/s/player/{id}/{MAIN_PLAYER_VARIANT}"))
        .ok()
        .map(String::from)
}

/// `_extract_response(ep=ENDPOINT)` for the web client (tab pages).
#[allow(clippy::too_many_arguments)]
pub(super) fn innertube_api(
    client: &Client,
    endpoint: &str,
    extractor: &'static str,
    item_id: &str,
    ytcfg: &Map<String, Value>,
    visitor: Option<&str>,
    query: Map<String, Value>,
    note: &str,
) -> Result<Value> {
    let config = innertube::client(WEBPAGE_CLIENT).expect("the web client is defined");
    call_api(
        client,
        config,
        endpoint,
        extractor,
        item_id,
        ytcfg,
        visitor,
        query,
        note,
        Session::default(),
    )
}

/// `_extract_visitor_data` of one source.
pub(super) fn visitor_from(source: &Value) -> Option<String> {
    visitor_data(&[source])
}

/// `_extract_visitor_data`.
fn visitor_data(sources: &[&Value]) -> Option<String> {
    for source in sources {
        for path in [
            &["VISITOR_DATA"][..],
            &["INNERTUBE_CONTEXT", "client", "visitorData"][..],
            &["responseContext", "visitorData"][..],
        ] {
            let mut value = Some(*source);
            for key in path {
                value = value.and_then(|value| value.get(*key));
            }
            if let Some(Value::String(text)) = value {
                return Some(text.clone());
            }
        }
    }
    None
}

/// `_extract_context`: the client context with `hl`, `timeZone` and
/// `utcOffsetMinutes` enforced.
fn api_context(ytcfg: &Map<String, Value>, default: &Map<String, Value>) -> Value {
    let mut context = ytcfg
        .get("INNERTUBE_CONTEXT")
        .filter(|context| context.is_object())
        .or_else(|| default.get("INNERTUBE_CONTEXT"))
        .cloned()
        .unwrap_or_else(|| json!({}));
    if let Some(client) = context.get_mut("client").and_then(Value::as_object_mut) {
        client.insert("hl".into(), "en".into());
        client.insert("timeZone".into(), "UTC".into());
        client.insert("utcOffsetMinutes".into(), 0.into());
    }
    context
}

/// `_extract_player_response` through `_call_api`: one POST to
/// `/youtubei/v1/player`.
#[allow(clippy::too_many_arguments)]
fn player_response(
    client: &Client,
    config: &innertube::ClientConfig,
    video_id: &str,
    webpage_ytcfg: &Map<String, Value>,
    player_ytcfg: &Map<String, Value>,
    player_url: Option<&str>,
    visitor: Option<&str>,
    initial_pr: &Value,
    data_sync: Option<&str>,
) -> Result<Value> {
    let mut query = Map::new();
    query.insert("videoId".into(), video_id.into());
    if let Some(params) = config.player_params {
        query.insert("params".into(), params.into());
    }
    let sts = player_url.and_then(|url| signature_timestamp(client, video_id, url, webpage_ytcfg));
    let mut playback = Map::new();
    playback.insert("html5Preference".into(), "HTML5_PREF_WANTS".into());
    if let Some(sts) = sts {
        playback.insert("signatureTimestamp".into(), sts.into());
    }
    // Embedded player requests may need their page's `encryptedHostFlags`.
    if split_client(config.name).2 == Some("embedded")
        && let Some(flags) = player_ytcfg
            .get("WEB_PLAYER_CONTEXT_CONFIGS")
            .and_then(|configs| configs.get("WEB_PLAYER_CONTEXT_CONFIG_ID_EMBEDDED_PLAYER"))
            .and_then(|config| config.get("encryptedHostFlags"))
            .filter(|flags| super::tab::truthy(Some(flags)))
    {
        playback.insert("encryptedHostFlags".into(), flags.clone());
    }
    query.insert(
        "playbackContext".into(),
        json!({"contentPlaybackContext": Value::Object(playback)}),
    );
    query.insert("contentCheckOk".into(), true.into());
    query.insert("racyCheckOk".into(), true.into());
    let note = format!(
        "Downloading {} player API JSON",
        config.name.replace('_', " ").trim()
    );
    // The session index and IDs of `_extract_player_response`.
    let (webpage, player) = (
        Value::Object(webpage_ytcfg.clone()),
        Value::Object(player_ytcfg.clone()),
    );
    let (delegated, user) = parse_data_sync_id(data_sync);
    let session = Session {
        index: session_index(&[&webpage, &player]),
        delegated: delegated.or_else(|| delegated_session_id(&[&webpage, initial_pr, &player])),
        user: user.or_else(|| user_session_id(&[&webpage, initial_pr, &player])),
    };
    call_api(
        client,
        config,
        "player",
        "youtube",
        video_id,
        player_ytcfg,
        visitor,
        query,
        &note,
        session,
    )
}

/// An account session's request values (`generate_api_headers` arguments);
/// missing ones come from the request's `ytcfg`.
#[derive(Default)]
struct Session {
    index: Option<i64>,
    delegated: Option<String>,
    user: Option<String>,
}

fn first_text(sources: &[&Value], key: &str) -> Option<String> {
    sources
        .iter()
        .find_map(|source| source.get(key)?.as_str().map(str::to_owned))
}

/// `_extract_session_index`.
fn session_index(sources: &[&Value]) -> Option<i64> {
    sources
        .iter()
        .find_map(|source| match source.get("SESSION_INDEX")? {
            Value::Number(number) => number.as_i64(),
            Value::String(text) => text.trim().parse().ok(),
            _ => None,
        })
}

/// `_extract_data_sync_id`.
fn data_sync_id(sources: &[&Value]) -> Option<String> {
    sources.iter().find_map(|source| {
        source
            .get("DATASYNC_ID")
            .or_else(|| source.pointer("/responseContext/mainAppWebResponseContext/datasyncId"))?
            .as_str()
            .map(str::to_owned)
    })
}

/// `_parse_data_sync_id`: `DELEGATED||USER`, or `USER||`.
fn parse_data_sync_id(id: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(id) = id.filter(|id| !id.is_empty()) else {
        return (None, None);
    };
    match id.split_once("||") {
        Some((first, second)) if !second.is_empty() => {
            (Some(first.to_owned()), Some(second.to_owned()))
        }
        Some((first, _)) => (None, Some(first.to_owned())),
        None => (None, Some(id.to_owned())),
    }
}

/// `_extract_delegated_session_id`.
fn delegated_session_id(sources: &[&Value]) -> Option<String> {
    first_text(sources, "DELEGATED_SESSION_ID")
        .or_else(|| parse_data_sync_id(data_sync_id(sources).as_deref()).0)
}

/// `_extract_user_session_id`.
fn user_session_id(sources: &[&Value]) -> Option<String> {
    first_text(sources, "USER_SESSION_ID")
        .or_else(|| parse_data_sync_id(data_sync_id(sources).as_deref()).1)
}

/// `_generate_cookie_auth_headers`.
fn auth_headers(
    ytcfg: &Map<String, Value>,
    session: Session,
    origin: &str,
) -> Vec<(&'static str, String)> {
    let own = Value::Object(ytcfg.clone());
    let delegated = session.delegated.or_else(|| delegated_session_id(&[&own]));
    let index = session.index.or_else(|| session_index(&[&own]));
    let user = session.user.or_else(|| user_session_id(&[&own]));
    let mut headers = Vec::new();
    if let Some(delegated) = &delegated {
        headers.push(("X-Goog-PageId", delegated.clone()));
    }
    if delegated.is_some() || index.is_some() {
        headers.push(("X-Goog-AuthUser", index.unwrap_or(0).to_string()));
    }
    if let Some(authorization) = super::sid_authorization(origin, user.as_deref()) {
        headers.push(("Authorization", authorization));
        headers.push(("X-Origin", origin.to_owned()));
    }
    if ytcfg.get("LOGGED_IN").and_then(Value::as_bool) == Some(true) {
        headers.push(("X-Youtube-Bootstrap-Logged-In", "true".into()));
    }
    headers
}

/// `_call_api`: one POST to `/youtubei/v1/ENDPOINT` with the client's
/// `generate_api_headers`.
#[allow(clippy::too_many_arguments)]
fn call_api(
    client: &Client,
    config: &innertube::ClientConfig,
    endpoint: &str,
    extractor: &'static str,
    item_id: &str,
    player_ytcfg: &Map<String, Value>,
    visitor: Option<&str>,
    query: Map<String, Value>,
    note: &str,
    session: Session,
) -> Result<Value> {
    let default = default_ytcfg(config);
    let mut body = Map::new();
    body.insert("context".into(), api_context(player_ytcfg, &default));
    body.extend(query);
    let origin = format!("https://{}", config.host);
    let name_id = ytcfg_value(player_ytcfg, &default, &["INNERTUBE_CONTEXT_CLIENT_NAME"])
        .map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let version = ytcfg_value(player_ytcfg, &default, &["INNERTUBE_CLIENT_VERSION"])
        .or_else(|| {
            ytcfg_value(
                player_ytcfg,
                &default,
                &["INNERTUBE_CONTEXT", "client", "clientVersion"],
            )
        })
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let user_agent = ytcfg_value(
        player_ytcfg,
        &default,
        &["INNERTUBE_CONTEXT", "client", "userAgent"],
    )
    .and_then(Value::as_str)
    .map(str::to_owned);
    let mut headers: Vec<(&str, String)> = vec![
        ("X-YouTube-Client-Name", name_id),
        ("X-YouTube-Client-Version", version),
        ("Origin", origin.clone()),
    ];
    if let Some(visitor) = visitor {
        headers.push(("X-Goog-Visitor-Id", visitor.to_owned()));
    }
    if let Some(user_agent) = user_agent {
        headers.push(("User-Agent", user_agent));
    }
    headers.extend(auth_headers(player_ytcfg, session, &origin));
    headers.push(("content-type", "application/json".into()));
    let url = Url::parse(&format!(
        "https://{}/youtubei/v1/{endpoint}?prettyPrint=false",
        config.host
    ))?;
    crate::screen::screen(&format!("[{extractor}] {item_id}: {note}"));
    let headers: Vec<(&str, &str)> = headers
        .iter()
        .map(|(name, value)| (*name, value.as_str()))
        .collect();
    let body = serde_json::to_string(&Value::Object(body))
        .map_err(|error| Error::Reported(error.to_string()))?;
    let response =
        super::super::direct::post(client, &url, &headers, body).map_err(|error| match error {
            Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
                extractor,
                id: item_id.to_owned(),
                what: "API page",
                source: Box::new(error),
            },
            error => error,
        })?;
    let text = response.text().map_err(Error::Http)?;
    serde_json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{extractor}] {item_id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_download_player_url`: the player version in the iframe API JS.
fn iframe_player_url(client: &Client, video_id: &str) -> Option<String> {
    static VERSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"player\\?/([0-9a-fA-F]{8})\\?/").unwrap());
    crate::screen::screen(&format!("[youtube] {video_id}: Downloading iframe API JS"));
    let url = Url::parse("https://www.youtube.com/iframe_api").ok()?;
    let page = match super::super::direct::request(client, &url, None)
        .and_then(|response| response.text().map_err(Error::Http))
    {
        Ok(page) => page,
        Err(error) => {
            crate::screen::warning(&format!(
                "[youtube] {video_id}: Unable to download webpage: {}",
                crate::upstream_error_text(&error)
            ));
            return None;
        }
    };
    let Some(found) = VERSION.captures(&page) else {
        super::super::search_warning("youtube", "player version");
        return None;
    };
    Some(format!(
        "https://www.youtube.com/s/player/{}/{MAIN_PLAYER_VARIANT}",
        &found[1]
    ))
}

/// The session `_get_requested_clients` chooses clients for.
struct ClientSelection {
    js_available: bool,
    authenticated: bool,
    premium: bool,
    music_url: bool,
}

/// `_get_requested_clients`.
fn requested_clients(session: &ClientSelection) -> Vec<&'static str> {
    let defaults: &[&'static str] = if session.premium {
        &DEFAULT_PREMIUM_CLIENTS
    } else if session.authenticated {
        &DEFAULT_AUTHED_CLIENTS
    } else if !session.js_available {
        &DEFAULT_JSLESS_CLIENTS
    } else {
        &DEFAULT_CLIENTS
    };
    let mut requested: Vec<&'static str> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();
    let arguments = crate::manifest_args::configuration_arg("youtube", "player_client", false)
        .unwrap_or_default();
    let mut allowed: Vec<&'static innertube::ClientConfig> = innertube::CLIENTS
        .iter()
        .filter(|client| !client.name.starts_with('_'))
        .collect();
    allowed.sort_by_key(|client| std::cmp::Reverse(client.priority));
    for argument in &arguments {
        match argument.as_str() {
            "default" => requested.extend(defaults),
            "all" => requested.extend(allowed.iter().map(|client| client.name)),
            name if name.starts_with('-') => excluded.push(name[1..].to_owned()),
            name => match innertube::client(name) {
                Some(client) => requested.push(client.name),
                None => crate::screen::warning(&format!(
                    "[youtube] Skipping unsupported client \"{name}\""
                )),
            },
        }
    }
    if requested.is_empty() && excluded.is_empty() && defaults == DEFAULT_JSLESS_CLIENTS {
        crate::screen::warning_once(&format!(
            "[youtube] No supported JavaScript runtime could be found. Only deno is enabled by default; to use another runtime add  --js-runtimes RUNTIME[:PATH]  to your command/config. YouTube extraction without a JS runtime has been deprecated, and some formats may be missing. See  {EJS_WIKI_URL}  for details on installing one"
        ));
    }
    if requested.is_empty() {
        requested.extend(defaults);
    }
    requested.retain(|client| !excluded.iter().any(|excluded| excluded == client));
    if session.authenticated {
        if session.music_url && !requested.contains(&"web_music") {
            requested.push("web_music");
        }
        requested.retain(|name| {
            let supported = innertube::client(name).is_some_and(|config| config.supports_cookies);
            if !supported {
                crate::screen::warning_once(&format!(
                    "[youtube] Skipping client \"{name}\" since it does not support cookies"
                ));
            }
            supported
        });
    }
    let mut ordered = Vec::new();
    for client in requested {
        if !ordered.contains(&client) {
            ordered.push(client);
        }
    }
    ordered
}

/// `_get_available_at_timestamp`: now, plus the pre-roll ads' durations
/// (or their skip offsets), which must pass before the formats are served.
fn available_at(response: &Value, video_id: &str, client: &str) -> i64 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64());
    let mut renderers: Vec<&Value> = Vec::new();
    for placement in response
        .get("adPlacements")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if placement
            .pointer("/adPlacementRenderer/config/adPlacementConfig/kind")
            .and_then(Value::as_str)
            == Some("AD_PLACEMENT_KIND_START")
        {
            renderers
                .extend(placement.pointer("/adPlacementRenderer/renderer/instreamVideoAdRenderer"));
        }
    }
    for slot in response
        .get("adSlots")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if slot
            .pointer("/adSlotRenderer/adSlotMetadata/triggerEvent")
            .and_then(Value::as_str)
            != Some("SLOT_TRIGGER_EVENT_BEFORE_CONTENT")
        {
            continue;
        }
        let Some(content) = slot.pointer(
            "/adSlotRenderer/fulfillmentContent/fulfilledLayout/playerBytesAdLayoutRenderer/renderingContent",
        ) else {
            continue;
        };
        renderers.extend(content.get("instreamVideoAdRenderer"));
        for layout in content
            .pointer("/playerBytesSequentialLayoutRenderer/sequentialLayouts")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            renderers.extend(
                layout.pointer(
                    "/playerBytesAdLayoutRenderer/renderingContent/instreamVideoAdRenderer",
                ),
            );
        }
    }
    let mut wait = 0.0;
    for renderer in renderers
        .into_iter()
        .filter(|renderer| renderer.is_object())
    {
        // `parse_qs(playerVars)['length_seconds'][-1]`.
        let mut duration: Option<f64> = renderer
            .get("playerVars")
            .and_then(Value::as_str)
            .and_then(|vars| {
                url::form_urlencoded::parse(vars.as_bytes())
                    .filter(|(key, value)| key == "length_seconds" && !value.is_empty())
                    .last()
                    .and_then(|(_, value)| value.trim().parse::<i64>().ok())
            })
            .map(|seconds| seconds as f64);
        let mut ad = match duration {
            Some(seconds) => format!("a {seconds}s ad"),
            None => "an ad".to_owned(),
        };
        let skip = renderer
            .get("skipOffsetMilliseconds")
            .and_then(|value| match value {
                Value::Number(number) => number.as_f64(),
                Value::String(text) => text.trim().parse().ok(),
                _ => None,
            });
        if let Some(milliseconds) = skip {
            let seconds = milliseconds / 1000.0;
            // `skip_time if skip_time % 1 else int(skip_time)`.
            let shown = if seconds.fract() == 0.0 {
                format!("{}", seconds as i64)
            } else {
                crate::screen::py_float(seconds)
            };
            ad.push_str(&format!(" skippable after {shown}s"));
            duration = Some(seconds);
        }
        if let Some(duration) = duration {
            crate::screen::extractor_debug(&format!("{video_id}: Detected {ad} for {client}"));
            wait += duration;
        }
    }
    if wait != 0.0 {
        (now.ceil() + wait) as i64
    } else {
        now as i64
    }
}

/// `_is_premium_subscriber`: the topbar's YouTube Premium logo.
fn is_premium_subscriber(authenticated: bool, initial_data: Option<&Value>) -> bool {
    let Some(logo) = initial_data
        .filter(|_| authenticated)
        .and_then(|data| data.pointer("/topbar/desktopTopbarRenderer/logo/topbarLogoRenderer"))
    else {
        return false;
    };
    logo.pointer("/iconImage/iconType").and_then(Value::as_str) == Some("YOUTUBE_PREMIUM_LOGO")
        || logo
            .get("tooltipText")
            .and_then(super::metadata::text)
            .is_some_and(|text| text.to_lowercase().contains("premium"))
}

/// The acquisition result `_real_extract` continues with.
pub(super) struct Acquired {
    pub webpage: String,
    pub initial_data: Option<Value>,
    pub responses: Vec<streams::ClientResponse>,
    /// Every player response in upstream's order (the stripped initial one
    /// first), for metadata.
    pub player_responses: Vec<Value>,
    pub player_url: Option<String>,
    /// `is_premium_subscriber`.
    pub premium: bool,
}

/// `_initial_extract` for an anonymous session.
pub(super) fn acquire(
    client: &Client,
    video_id: &str,
    runtime: Option<&challenge::Runtime>,
    music_url: bool,
) -> Result<Acquired> {
    let webpage_url = Url::parse(&format!(
        "https://www.youtube.com/watch?v={video_id}&bpctr=9999999999&has_verified=1"
    ))?;
    let webpage = super::super::download_webpage(client, &webpage_url, "youtube", video_id)?;
    let web = innertube::client(WEBPAGE_CLIENT).expect("web client");
    let mut webpage_ytcfg = extract_ytcfg(&webpage);
    if webpage_ytcfg.is_empty() {
        webpage_ytcfg = default_ytcfg(web);
    }
    static INITIAL_DATA: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?:window\s*\[\s*["']ytInitialData["']\s*\]|ytInitialData)\s*="#).unwrap()
    });
    static INITIAL_PLAYER_RESPONSE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"ytInitialPlayerResponse\s*=").unwrap());
    let initial_data = search_initial(&webpage, &INITIAL_DATA, "yt initial data").filter(|data| {
        data.get("contents")
            .is_some_and(|contents| !contents.is_null())
    });
    // `_get_requested_clients` runs before `_extract_player_responses`
    // searches the initial player response.
    let authenticated = super::has_auth_cookies();
    let premium = is_premium_subscriber(authenticated, initial_data.as_ref());
    let clients = requested_clients(&ClientSelection {
        js_available: runtime.is_some(),
        authenticated,
        premium,
        music_url,
    });
    let initial_pr = search_initial(
        &webpage,
        &INITIAL_PLAYER_RESPONSE,
        &format!("{WEBPAGE_CLIENT} client initial player response"),
    );

    let mut player_responses: Vec<Value> = Vec::new();
    let mut responses: Vec<streams::ClientResponse> = Vec::new();
    // `not _invalid_player_response`: a response without video details is
    // kept (it may carry the playability error).
    let valid = |response: &Value| {
        response
            .get("videoDetails")
            .and_then(|details| details.get("videoId"))
            .is_none_or(|id| id.as_str() == Some(video_id))
    };
    if let Some(initial) = initial_pr
        .as_ref()
        .filter(|initial| initial.as_object().is_some_and(|object| !object.is_empty()))
        .filter(|initial| valid(initial))
    {
        let mut stripped = initial.clone();
        if let Some(object) = stripped.as_object_mut() {
            object.insert("streamingData".into(), Value::Null);
        }
        player_responses.push(stripped);
    }
    let mut player_url_found: Option<String> = None;
    let mut tried_iframe_fallback = false;
    let mut visitor: Option<String> = None;
    let mut data_sync: Option<String> = None;
    let mut skipped: Vec<(&'static str, String)> = Vec::new();
    let webpage_ytcfg_value = Value::Object(webpage_ytcfg.clone());
    let skip_configs = crate::manifest_args::configuration_arg("youtube", "player_skip", false)
        .is_some_and(|skip| skip.iter().any(|value| value == "configs"));
    // `append_client` adds the first named client not yet requested.
    let mut all_clients: Vec<&str> = clients.clone();
    let mut pending: Vec<String> = clients
        .iter()
        .rev()
        .map(|name| (*name).to_owned())
        .collect();
    let mut append_client = |pending: &mut Vec<String>, names: &[String]| {
        for name in names {
            let actual = split_client(name).0;
            if let Some(config) = innertube::client(actual) {
                if !all_clients.contains(&config.name) {
                    pending.push(name.clone());
                    all_clients.push(config.name);
                }
                return;
            }
        }
    };
    while let Some(requested) = pending.pop() {
        let (name, base, variant) = split_client(&requested);
        let Some(config) = innertube::client(name) else {
            continue;
        };
        let mut player_ytcfg = if config.name == WEBPAGE_CLIENT {
            webpage_ytcfg.clone()
        } else {
            Map::new()
        };
        if !skip_configs && config.name != WEBPAGE_CLIENT {
            let downloaded = download_ytcfg(client, config, video_id);
            if !downloaded.is_empty() {
                player_ytcfg = downloaded;
            }
        }
        if player_url_found.is_none() {
            player_url_found = player_url(&[&webpage_ytcfg, &player_ytcfg]);
        }
        if player_url_found.is_none() && !tried_iframe_fallback && config.require_js_player {
            player_url_found = iframe_player_url(client, video_id);
            tried_iframe_fallback = true;
        }
        let initial_value = initial_pr.clone().unwrap_or(Value::Null);
        if data_sync.is_none() {
            data_sync = data_sync_id(&[
                &webpage_ytcfg_value,
                &initial_value,
                &Value::Object(player_ytcfg.clone()),
            ]);
        }
        if visitor.is_none() {
            visitor = visitor_data(&[
                &webpage_ytcfg_value,
                &initial_value,
                &Value::Object(player_ytcfg.clone()),
            ]);
        }
        let reused = (config.name == WEBPAGE_CLIENT)
            .then(|| initial_pr.clone())
            .flatten();
        let response = match reused {
            Some(response) => response,
            None => match player_response(
                client,
                config,
                video_id,
                if player_ytcfg.is_empty() {
                    &webpage_ytcfg
                } else {
                    &player_ytcfg
                },
                &player_ytcfg,
                player_url_found.as_deref(),
                visitor.as_deref(),
                &initial_value,
                data_sync.as_deref(),
            ) {
                Ok(response) => response,
                Err(error) => {
                    crate::screen::warning(&crate::upstream_error_text(&error));
                    continue;
                }
            },
        };
        // `_invalid_player_response`: only another video's response is
        // skipped (one without details is kept).
        let id = response
            .get("videoDetails")
            .and_then(|details| details.get("videoId"))
            .map(|id| match id {
                Value::String(text) => text.clone(),
                other => other.to_string(),
            });
        let status = playability_status(&response).map(str::to_owned);
        if id.as_deref().is_some_and(|id| id != video_id) {
            skipped.push((config.name, id.unwrap_or_default()));
        } else if response
            .as_object()
            .is_some_and(|object| !object.is_empty())
        {
            player_responses.push(response.clone());
            responses.push(streams::ClientResponse {
                client: config.name,
                response: response.clone(),
                player_token_provided: false,
                available_at: available_at(&response, video_id, config.name),
            });
        }
        // A "made for kids" video android_vr/visionos cannot play may play
        // through web_embedded, given a JS runtime.
        if matches!(config.name, "android_vr" | "visionos")
            && matches!(status.as_deref(), Some("UNPLAYABLE" | "ERROR"))
            && webpage.contains("made for kids")
            && runtime.is_some()
        {
            append_client(&mut pending, &["web_embedded".into()]);
        }
        // web_embedded can work around the age gate of embeddable videos.
        let agegated = is_agegated(&response);
        if agegated && variant != Some("web_embedded") {
            append_client(&mut pending, &[format!("web_embedded.{base}")]);
        }
        if agegated && !authenticated {
            crate::screen::screen_once(&format!(
                "[youtube] {video_id}: This video is age-restricted; some formats may be missing without authentication. {LOGIN_HINT}"
            ));
        }
        // EU accounts need age verification; web_creator may work around it.
        let embedding_disabled =
            variant == Some("web_embedded") && status.as_deref() == Some("UNPLAYABLE");
        if authenticated && (agegated || embedding_disabled) {
            crate::screen::screen_once(&format!(
                "[youtube] {video_id}: This video is age-restricted and YouTube is requiring account age-verification; some formats may be missing"
            ));
            append_client(&mut pending, &["web_creator".into()]);
        }
        if !matches!(
            status.as_deref(),
            Some("OK" | "LIVE_STREAM_OFFLINE" | "AGE_CHECK_REQUIRED" | "AGE_VERIFICATION_REQUIRED")
        ) {
            crate::screen::extractor_debug(&format!(
                "{video_id}: {name} player response playability status: {}",
                status.as_deref().unwrap_or("None")
            ));
        }
    }
    if !skipped.is_empty() {
        let clients: Vec<&str> = skipped.iter().map(|(client, _)| *client).collect();
        let mut ids: Vec<&str> = skipped.iter().map(|(_, id)| id.as_str()).collect();
        ids.dedup();
        crate::screen::warning(&format!(
            "[youtube] Skipping player responses from {} clients (got player responses for video \"{}\" instead of \"{video_id}\")",
            clients.join("/"),
            ids.join("/")
        ));
        if player_responses.is_empty() {
            return Err(Error::Reported(format!(
                "[youtube] {video_id}: All player responses are invalid. Your IP is likely being blocked by Youtube"
            )));
        }
    } else if player_responses.is_empty() {
        return Err(Error::Reported(format!(
            "[youtube] {video_id}: Failed to extract any player response; {}",
            crate::BUG_REPORTS_MESSAGE
        )));
    }
    Ok(Acquired {
        webpage,
        initial_data,
        responses,
        player_responses,
        player_url: player_url_found,
        premium,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_at_adds_pre_roll_ads_like_upstream() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let ad = |vars: &str, skip: Option<i64>| {
            let mut renderer = json!({"playerVars": vars});
            if let Some(skip) = skip {
                renderer["skipOffsetMilliseconds"] = skip.into();
            }
            json!({"instreamVideoAdRenderer": renderer})
        };
        let layout =
            |content: Value| json!({"playerBytesAdLayoutRenderer": {"renderingContent": content}});
        let response = json!({
            "adPlacements": [
                {"adPlacementRenderer": {
                    "config": {"adPlacementConfig": {"kind": "AD_PLACEMENT_KIND_START"}},
                    "renderer": ad("length_seconds=15&x=1", Some(5000)),
                }},
                {"adPlacementRenderer": {
                    "config": {"adPlacementConfig": {"kind": "AD_PLACEMENT_KIND_MILLISECONDS"}},
                    "renderer": ad("length_seconds=99", None),
                }},
            ],
            "adSlots": [{"adSlotRenderer": {
                "adSlotMetadata": {"triggerEvent": "SLOT_TRIGGER_EVENT_BEFORE_CONTENT"},
                "fulfillmentContent": {"fulfilledLayout": layout(json!({
                    "playerBytesSequentialLayoutRenderer": {"sequentialLayouts": [
                        layout(ad("length_seconds=7", None)),
                        layout(ad("", None)),
                    ]},
                }))},
            }}],
        });
        // 5s (skippable) + 7s; the mid-roll and the ad without a length are ignored.
        let at = available_at(&response, "id", "tv_samsung");
        assert!((now + 12..=now + 14).contains(&at), "{at} {now}");
        let plain = available_at(&json!({}), "id", "tv_samsung");
        assert!((now..=now + 1).contains(&plain));
    }
}
