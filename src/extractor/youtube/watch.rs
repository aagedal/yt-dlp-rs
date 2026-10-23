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
    let key = player_js_cache_key(player_url)?;
    if let Some(code) = CODE.lock().ok()?.get(&key) {
        return Some(code.clone());
    }
    crate::screen::screen(&format!("[youtube] {video_id}: Downloading player {key}"));
    let code = Url::parse(player_url)
        .map_err(Error::from)
        .and_then(|url| super::super::direct::request(client, &url, None))
        .and_then(|response| read_page(response, video_id, None));
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

/// `_extract_signature_timestamp(fatal=False)`: `player_js_version`'s STS,
/// else the `ytcfg`'s `STS`, else the player JS's.
fn signature_timestamp(
    client: &Client,
    video_id: &str,
    player_url: &str,
    ytcfg: &Map<String, Value>,
) -> Option<i64> {
    if let (Some(sts), _) = player_js_version() {
        return Some(sts);
    }
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
    construct_player_url(None, Some(found))
}

/// `_PLAYER_JS_VARIANT_MAP`.
const PLAYER_JS_VARIANTS: [(&str, &str); 11] = [
    ("main", MAIN_PLAYER_VARIANT),
    ("tcc", "player_ias_tcc.vflset/en_US/base.js"),
    ("tce", "player_ias_tce.vflset/en_US/base.js"),
    ("es5", "player_es5.vflset/en_US/base.js"),
    ("es6", "player_es6.vflset/en_US/base.js"),
    ("es6_tcc", "player_es6_tcc.vflset/en_US/base.js"),
    ("es6_tce", "player_es6_tce.vflset/en_US/base.js"),
    ("tv", "tv-player-ias.vflset/tv-player-ias.js"),
    ("tv_es6", "tv-player-es6.vflset/tv-player-es6.js"),
    ("phone", "player-plasma-ias-phone-en_US.vflset/base.js"),
    ("house", "house_brand_player.vflset/en_US/base.js"),
];

fn variant_path(variant: &str) -> &'static str {
    PLAYER_JS_VARIANTS
        .iter()
        .find(|(name, _)| *name == variant)
        .map_or(MAIN_PLAYER_VARIANT, |(_, path)| path)
}

/// `_player_js_version` (`player_js_version`, else `actual`).
pub(super) fn player_js_version_arg() -> String {
    super::args::non_empty("player_js_version").unwrap_or_else(|| "actual".into())
}

/// `_get_player_js_version`: the STS and player ID `STS@HASH` forces.
fn player_js_version() -> (Option<i64>, Option<String>) {
    let version = player_js_version_arg();
    if version == "actual" {
        return (None, None);
    }
    static VERSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^([0-9]{5,})@([0-9a-f]{8,})$").unwrap());
    match VERSION.captures(&version) {
        Some(found) => (found[1].parse().ok(), Some(found[2].to_owned())),
        None => {
            crate::screen::warning_once(&format!(
                "[youtube] Invalid player JS version \"{version}\" specified. It should be \"actual\" or in the format of STS@HASH"
            ));
            (None, None)
        }
    }
}

/// `_extract_player_info`.
fn player_id_of(player_url: &str) -> Option<String> {
    static PLAYER_ID: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"/s/player/(?P<id>[a-fA-F0-9]{8,})/").unwrap());
    Some(PLAYER_ID.captures(player_url)?["id"].to_owned())
}

/// `_get_player_id_variant_and_path`.
fn player_variant_and_path(player_url: &str) -> Option<(String, Option<&'static str>, String)> {
    let id = player_id_of(player_url)?;
    let path = Url::parse("https://www.youtube.com")
        .ok()?
        .join(player_url)
        .ok()?
        .path()
        .to_owned();
    let path = path
        .strip_prefix(&format!("/s/player/{id}/"))
        .map_or(path.clone(), str::to_owned);
    let variant = PLAYER_JS_VARIANTS
        .iter()
        .find(|(_, known)| *known == path)
        .or_else(|| {
            PLAYER_JS_VARIANTS.iter().find(|(_, known)| {
                let pattern = regex::escape(known).replace("en_US", "[a-zA-Z0-9_]+");
                Regex::new(&format!("^(?:{pattern})$")).is_ok_and(|regex| regex.is_match(&path))
            })
        })
        .map(|(name, _)| *name);
    if variant.is_none() {
        crate::screen::extractor_debug_once(&format!(
            "Unable to determine player JS variant\n        player = {player_url}"
        ));
    }
    Some((id, variant, path))
}

/// `_player_js_cache_key`: `PLAYER_ID-VARIANT`.
fn player_js_cache_key(player_url: &str) -> Option<String> {
    let (id, variant, path) = player_variant_and_path(player_url)?;
    let variant = match variant {
        Some(variant) => variant.to_owned(),
        None => path
            .strip_suffix(".js")
            .unwrap_or(&path)
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect(),
    };
    Some(format!("{id}-{variant}"))
}

/// `_construct_player_url`: the player of `player_js_variant` (`main` by
/// default, `actual` keeps the page's), with `player_js_version`'s ID.
fn construct_player_url(player_id: Option<&str>, player_url: Option<&str>) -> Option<String> {
    let mut id = match player_id {
        Some(id) => id.to_owned(),
        None => player_id_of(player_url?)?,
    };
    let mut force = None;
    if let (_, Some(forced)) = player_js_version()
        && forced != id
    {
        force = Some(format!("Forcing player {forced} in place of player {id}"));
        id = forced;
    }
    let mut variant = super::args::non_empty("player_js_variant").unwrap_or_else(|| "main".into());
    if variant != "actual" && !PLAYER_JS_VARIANTS.iter().any(|(name, _)| *name == variant) {
        let names: Vec<&str> = PLAYER_JS_VARIANTS.iter().map(|(name, _)| *name).collect();
        crate::screen::warning_once(&format!(
            "[youtube] Invalid player JS variant name \"{variant}\" requested. Valid choices are: {}",
            names.join(", ")
        ));
        variant = "main".into();
    }
    let base = Url::parse("https://www.youtube.com").ok()?;
    let Some(player_url) = player_url else {
        if let Some(force) = &force {
            crate::screen::extractor_debug_once(force);
        }
        return base
            .join(&format!("/s/player/{id}/{}", variant_path(&variant)))
            .ok()
            .map(String::from);
    };
    let actual = player_variant_and_path(player_url).and_then(|(_, variant, _)| variant);
    if force.is_none() && (variant == "actual" || Some(variant.as_str()) == actual) {
        return base.join(player_url).ok().map(String::from);
    }
    if variant == "actual" {
        variant = actual.unwrap_or("main").to_owned();
    }
    let mut lines: Vec<String> = force.into_iter().collect();
    if Some(variant.as_str()) != actual {
        lines.push(format!(
            "Forcing \"{variant}\" player JS variant for player {id}"
        ));
    }
    lines.push(format!("original url = {player_url}"));
    crate::screen::extractor_debug_once(&lines.join("\n        "));
    base.join(&format!("/s/player/{id}/{}", variant_path(&variant)))
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

/// `_extract_response(ep=ENDPOINT, default_client=CLIENT)` without a
/// visitor ID (`_extract_tab_endpoint`).
#[allow(clippy::too_many_arguments)]
pub(super) fn innertube_api_for(
    client: &Client,
    client_name: &str,
    endpoint: &str,
    extractor: &'static str,
    item_id: &str,
    ytcfg: &Map<String, Value>,
    query: Map<String, Value>,
    note: &str,
) -> Result<Value> {
    let config = innertube::client(client_name).expect("a defined client");
    call_api(
        client,
        config,
        endpoint,
        extractor,
        item_id,
        ytcfg,
        visitor_data(&[&Value::Object(ytcfg.clone())]).as_deref(),
        query,
        note,
        Session::default(),
    )
}

/// `_extract_visitor_data` of one source.
pub(super) fn visitor_from(source: &Value) -> Option<String> {
    visitor_data(&[source])
}

/// `_extract_visitor_data`: `youtube:visitor_data`, else the sources'.
fn visitor_data(sources: &[&Value]) -> Option<String> {
    if let Some(visitor) = super::args::non_empty_case("visitor_data") {
        return Some(visitor);
    }
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
        client.insert("hl".into(), super::request_lang().into());
        client.insert("timeZone".into(), "UTC".into());
        client.insert("utcOffsetMinutes".into(), 0.into());
    }
    context
}

/// `_extract_player_response` through `_extract_response`: one POST to
/// `/youtubei/v1/player` (`None` when an incomplete response was given up on).
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
    po_token: Option<&str>,
) -> Result<Option<Value>> {
    let mut query = Map::new();
    query.insert("videoId".into(), video_id.into());
    // `player_params`, else the client's `PLAYER_PARAMS`.
    let params = super::args::first_case("player_params")
        .or_else(|| config.player_params.map(str::to_owned))
        .filter(|params| !params.is_empty());
    if let Some(params) = params {
        query.insert("params".into(), params.into());
    }
    if let Some(token) = po_token {
        query.insert(
            "serviceIntegrityDimensions".into(),
            json!({"poToken": token}),
        );
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
    let mut playback_context = Map::new();
    playback_context.insert("contentPlaybackContext".into(), Value::Object(playback));
    // `use_ad_playback_context` for the clients supporting it.
    if super::args::first("use_ad_playback_context").is_some_and(|value| value != "false")
        && config.supports_ad_playback_context
    {
        playback_context.insert("adPlaybackContext".into(), json!({"pyv": true}));
    }
    query.insert("playbackContext".into(), Value::Object(playback_context));
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
    let default = default_ytcfg(config);
    let request = ApiRequest::new(
        config,
        "player",
        "youtube",
        video_id,
        api_context(player_ytcfg, &default),
        query,
        api_headers(config, player_ytcfg, visitor, session),
        &note,
    )?;
    extract_response(client, &request, None, true)
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

/// `_extract_data_sync_id`: `youtube:data_sync_id`, else the sources'.
fn data_sync_id(sources: &[&Value]) -> Option<String> {
    if let Some(id) = super::args::non_empty_case("data_sync_id") {
        return Some(id);
    }
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

/// `_webpage_read_content`: the body, dumped by `--dump-pages` and
/// `--write-pages` (named with the POST `data`'s MD5, if any).
fn read_page(
    mut response: reqwest::blocking::Response,
    item_id: &str,
    data: Option<&[u8]>,
) -> Result<String> {
    use std::io::Read;
    let url = response.url().clone();
    let mut bytes = Vec::new();
    response
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    match data {
        Some(data) => crate::screen::expect_post_page(item_id, data),
        None => crate::screen::expect_page(item_id, None, None),
    }
    crate::screen::page_read(&url, &bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Upstream's default `extractor_retries` for InnerTube requests.
const API_RETRIES: u32 = 3;

/// `_select_api_hostname`: `innertube_host`, else the client's host.
fn api_host(config: &innertube::ClientConfig) -> String {
    super::args::non_empty("innertube_host").unwrap_or_else(|| config.host.to_owned())
}

/// `generate_api_headers(ytcfg=YTCFG, default_client=CLIENT, ...)`.
fn api_headers(
    config: &innertube::ClientConfig,
    ytcfg: &Map<String, Value>,
    visitor: Option<&str>,
    session: Session,
) -> Vec<(&'static str, String)> {
    let default = default_ytcfg(config);
    let origin = format!("https://{}", api_host(config));
    let name_id = ytcfg_value(ytcfg, &default, &["INNERTUBE_CONTEXT_CLIENT_NAME"])
        .map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let version = ytcfg_value(ytcfg, &default, &["INNERTUBE_CLIENT_VERSION"])
        .or_else(|| {
            ytcfg_value(
                ytcfg,
                &default,
                &["INNERTUBE_CONTEXT", "client", "clientVersion"],
            )
        })
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let user_agent = ytcfg_value(
        ytcfg,
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
    // `visitor_data or self._extract_visitor_data(ytcfg)`.
    let visitor = visitor
        .map(str::to_owned)
        .or_else(|| visitor_data(&[&Value::Object(ytcfg.clone())]));
    if let Some(visitor) = visitor.filter(|visitor| !visitor.is_empty()) {
        headers.push(("X-Goog-Visitor-Id", visitor));
    }
    if let Some(user_agent) = user_agent {
        headers.push(("User-Agent", user_agent));
    }
    headers.extend(auth_headers(ytcfg, session, &origin));
    headers
}

/// [`api_headers`] for the web client and the page's own `ytcfg`.
pub(super) fn web_api_headers(
    ytcfg: &Map<String, Value>,
    visitor: Option<&str>,
) -> Vec<(&'static str, String)> {
    let config = innertube::client(WEBPAGE_CLIENT).expect("the web client is defined");
    api_headers(config, ytcfg, visitor, Session::default())
}

/// One `_call_api` request's parts.
struct ApiRequest<'a> {
    extractor: &'static str,
    item_id: &'a str,
    note: &'a str,
    url: Url,
    headers: Vec<(&'static str, String)>,
    body: String,
}

/// An attempt's failure: the error and whether `_extract_response` retries
/// it (network errors and HTTP errors other than 403 and 429).
struct Failure {
    error: Error,
    retry: bool,
}

impl<'a> ApiRequest<'a> {
    /// `_call_api`'s body (`{'context': CONTEXT, **query}`, `json.dumps`ed),
    /// URL (`innertube_host`, `key=innertube_key`) and headers.
    #[allow(clippy::too_many_arguments)]
    fn new(
        config: &innertube::ClientConfig,
        endpoint: &str,
        extractor: &'static str,
        item_id: &'a str,
        context: Value,
        query: Map<String, Value>,
        mut headers: Vec<(&'static str, String)>,
        note: &'a str,
    ) -> Result<Self> {
        let mut body = Map::new();
        body.insert("context".into(), context);
        body.extend(query);
        headers.push(("content-type", "application/json".into()));
        let mut url = Url::parse(&format!(
            "https://{}/youtubei/v1/{endpoint}",
            api_host(config)
        ))?;
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(key) = super::args::non_empty_case("innertube_key") {
                pairs.append_pair("key", &key);
            }
            pairs.append_pair("prettyPrint", "false");
        }
        Ok(ApiRequest {
            extractor,
            item_id,
            note,
            url,
            headers,
            body: crate::output::python_json_dumps(&Value::Object(body)),
        })
    }

    /// One `_download_json`: an HTTP error's JSON `error.message` is
    /// reported (`YouTube said: ERROR - ...`) before the error.
    fn attempt(&self, client: &Client) -> std::result::Result<Value, Failure> {
        let (extractor, item_id) = (self.extractor, self.item_id);
        // `_download_json(video_id=None)` names no item.
        if item_id.is_empty() {
            crate::screen::screen(&format!("[{extractor}] {}", self.note));
        } else {
            crate::screen::screen(&format!("[{extractor}] {item_id}: {}", self.note));
        }
        let headers: Vec<(&str, &str)> = self
            .headers
            .iter()
            .map(|(name, value)| (*name, value.as_str()))
            .collect();
        static ERROR_STATUSES: LazyLock<Vec<u16>> = LazyLock::new(|| (400..600).collect());
        let failure = |error: Error| {
            let status = match &error {
                Error::Http(source) | Error::HttpRetryAfter { source, .. } => source.status(),
                _ => None,
            };
            Failure {
                retry: status.is_none_or(|status| !matches!(status.as_u16(), 403 | 429)),
                error: Error::ExtractorRequest {
                    extractor,
                    id: item_id.to_owned(),
                    what: "API page",
                    source: Box::new(error),
                },
            }
        };
        let response = match super::super::direct::post_expecting(
            client,
            &self.url,
            &headers,
            self.body.clone(),
            &ERROR_STATUSES,
        ) {
            Ok(response) => response,
            Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
                return Err(failure(error));
            }
            Err(error) => {
                return Err(Failure {
                    error,
                    retry: false,
                });
            }
        };
        if let Err(source) = response.error_for_status_ref() {
            let body = response.bytes().unwrap_or_default();
            if !is_html(&body)
                && let Some(message) = serde_json::from_slice::<Value>(&body)
                    .ok()
                    .and_then(|value| value.pointer("/error/message")?.as_str().map(str::to_owned))
            {
                crate::screen::warning(&format!("[{extractor}] YouTube said: ERROR - {message}"));
            }
            return Err(failure(Error::Http(source)));
        }
        let text =
            read_page(response, item_id, Some(self.body.as_bytes())).map_err(|error| Failure {
                error,
                retry: false,
            })?;
        serde_json::from_str(&text).map_err(|error| Failure {
            error: Error::Reported(format!(
                "[{extractor}] {item_id}: Failed to parse JSON  (caused by {error})"
            )),
            retry: false,
        })
    }

    /// Report a retried failure like `RetryManager.report_retry`.
    fn report_retry(&self, text: &str, retry: u32) {
        crate::screen::warning(&format!(
            "[{}] {}. Retrying ({retry}/{API_RETRIES})...",
            self.extractor,
            text.strip_suffix('.').unwrap_or(text)
        ));
    }
}

/// `is_html`: the first bytes (after any BOM) start with `<`.
fn is_html(bytes: &[u8]) -> bool {
    let mut bytes = bytes;
    for bom in [
        &b"\xef\xbb\xbf"[..],
        b"\x00\x00\xfe\xff",
        b"\xff\xfe\x00\x00",
        b"\xff\xfe",
        b"\xfe\xff",
    ] {
        while let Some(rest) = bytes.strip_prefix(bom) {
            bytes = rest;
        }
    }
    String::from_utf8_lossy(&bytes[..bytes.len().min(512)])
        .trim_start()
        .starts_with('<')
}

/// `_call_api`: one POST to `/youtubei/v1/ENDPOINT` with the client's
/// `generate_api_headers`, retrying network and HTTP errors.
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
    let request = ApiRequest::new(
        config,
        endpoint,
        extractor,
        item_id,
        api_context(player_ytcfg, &default),
        query,
        api_headers(config, player_ytcfg, visitor, session),
        note,
    )?;
    let mut retries = 0;
    loop {
        match request.attempt(client) {
            Ok(value) => return Ok(value),
            Err(Failure { error, retry }) => {
                if retry && retries < API_RETRIES {
                    retries += 1;
                    request.report_retry(&retry_text(&error), retries);
                    continue;
                }
                return Err(error);
            }
        }
    }
}

/// The text `report_retry` shows for an error (`str(e.cause)`).
fn retry_text(error: &Error) -> String {
    match error {
        Error::ExtractorRequest { source, .. } => crate::upstream_error_text(source),
        other => crate::upstream_error_text(other),
    }
}

/// `_extract_response(..., check_get_keys=KEY, fatal=FATAL)`: [`call_api`]
/// whose alerts are reported (`unknown error`s retried) and whose
/// incomplete responses (without `KEY`, or empty) are retried, then given
/// up on with a warning, or with `raise_incomplete_data` raised.
fn extract_response(
    client: &Client,
    request: &ApiRequest<'_>,
    check_key: Option<&str>,
    fatal: bool,
) -> Result<Option<Value>> {
    let extractor = request.extractor;
    let raise_incomplete = !super::args::list("raise_incomplete_data").is_empty();
    let (mut main_retries, mut incomplete_retries) = (0, 0);
    loop {
        let response = match request.attempt(client) {
            Ok(response) => response,
            Err(Failure { error, retry }) => {
                if retry {
                    if main_retries < API_RETRIES {
                        main_retries += 1;
                        request.report_retry(&retry_text(&error), main_retries);
                        continue;
                    }
                    return Err(error);
                }
                return error_or_warning(error, fatal);
            }
        };
        if let Err(message) = report_alerts(&response, extractor) {
            if message.to_lowercase().contains("unknown error") {
                if main_retries < API_RETRIES {
                    main_retries += 1;
                    request.report_retry(&message, main_retries);
                    continue;
                }
                return Err(Error::Reported(format!("[{extractor}] {message}")));
            }
            return error_or_warning(Error::Reported(format!("[{extractor}] {message}")), fatal);
        }
        let complete = match check_key {
            Some(key) => super::tab::truthy(response.get(key)),
            None => super::tab::truthy(Some(&response)),
        };
        if complete {
            return Ok(Some(response));
        }
        if incomplete_retries < API_RETRIES {
            incomplete_retries += 1;
            request.report_retry("Incomplete data received", incomplete_retries);
            continue;
        }
        if raise_incomplete {
            return Err(Error::Reported(format!(
                "[{extractor}] Incomplete data received"
            )));
        }
        crate::screen::warning(&format!(
            "[{extractor}] Incomplete data received. Giving up after {API_RETRIES} retries"
        ));
        return Ok(None);
    }
}

/// `_error_or_warning(e, fatal=FATAL)`.
fn error_or_warning(error: Error, fatal: bool) -> Result<Option<Value>> {
    if fatal {
        return Err(error);
    }
    crate::screen::warning(&crate::upstream_error_text(&error));
    Ok(None)
}

/// `_extract_and_report_alerts(data, only_once=True)`: warnings, then the
/// last error's `YouTube said: MESSAGE`.
fn report_alerts(data: &Value, extractor: &str) -> std::result::Result<(), String> {
    let mut errors = Vec::new();
    for alert in data
        .get("alerts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
        .flat_map(|alert| alert.values())
    {
        let Some(kind) = alert
            .get("type")
            .and_then(Value::as_str)
            .filter(|kind| !kind.is_empty())
        else {
            continue;
        };
        let Some(message) = alert.get("text").and_then(super::metadata::text) else {
            continue;
        };
        if kind.eq_ignore_ascii_case("error") {
            errors.push((kind.to_owned(), message));
        } else if !matches!(
            message.as_str(),
            "Unavailable videos will be hidden during playback" | "Unavailable videos are hidden"
        ) {
            crate::screen::warning_once(&format!("[{extractor}] YouTube said: {kind} - {message}"));
        }
    }
    if let Some((_, last)) = errors.pop() {
        for (kind, message) in errors {
            crate::screen::warning_once(&format!("[{extractor}] YouTube said: {kind} - {message}"));
        }
        return Err(format!("YouTube said: {last}"));
    }
    Ok(())
}

/// `_download_player_url`: the player version in the iframe API JS.
fn iframe_player_url(client: &Client, video_id: &str) -> Option<String> {
    static VERSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"player\\?/([0-9a-fA-F]{8})\\?/").unwrap());
    if let (_, Some(forced)) = player_js_version() {
        crate::screen::extractor_debug_once(&format!("Forcing player {forced}"));
        return construct_player_url(Some(&forced), None);
    }
    crate::screen::screen(&format!("[youtube] {video_id}: Downloading iframe API JS"));
    let url = Url::parse("https://www.youtube.com/iframe_api").ok()?;
    let page = match super::super::direct::request(client, &url, None)
        .and_then(|response| read_page(response, video_id, None))
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
    construct_player_url(Some(&found[1]), None)
}

/// The session `_get_requested_clients` chooses clients for.
struct ClientSelection<'a> {
    js_available: bool,
    authenticated: bool,
    premium: bool,
    music_url: bool,
    video_id: &'a str,
}

/// `_get_requested_clients`.
fn requested_clients(session: &ClientSelection<'_>) -> Result<Vec<&'static str>> {
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
        crate::screen::extractor_warning_once(&format!(
            "[youtube] No supported JavaScript runtime could be found. Only deno is enabled by default; to use another runtime add  --js-runtimes RUNTIME[:PATH]  to your command/config. YouTube extraction without a JS runtime has been deprecated, and some formats may be missing. See  {EJS_WIKI_URL}  for details on installing one"
        ));
    }
    if requested.is_empty() {
        requested.extend(defaults);
    }
    requested.retain(|client| !excluded.iter().any(|excluded| excluded == client));
    if requested.is_empty() {
        return Err(Error::Reported(format!(
            "[youtube] {}: No player clients have been requested",
            session.video_id
        )));
    }
    if session.authenticated {
        if session.music_url && !requested.contains(&"web_music") {
            requested.push("web_music");
        }
        requested.retain(|name| {
            let supported = innertube::client(name).is_some_and(|config| config.supports_cookies);
            if !supported {
                crate::screen::extractor_warning_once(&format!(
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
    Ok(ordered)
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

/// `_WEBPAGE_CLIENTS` and `_DEFAULT_WEBPAGE_CLIENT`.
const WEBPAGE_CLIENTS: [&str; 2] = ["web", "web_safari"];

/// `_webpage_client`: `webpage_client` (`web` or `web_safari`), else `web`.
fn webpage_client() -> &'static str {
    let requested = super::args::first("webpage_client").unwrap_or_else(|| WEBPAGE_CLIENT.into());
    match WEBPAGE_CLIENTS.iter().find(|client| **client == requested) {
        Some(client) => client,
        None => {
            crate::screen::warning_once(&format!(
                "[youtube] Invalid webpage_client \"{requested}\" requested; falling back to {WEBPAGE_CLIENT}"
            ));
            WEBPAGE_CLIENT
        }
    }
}

/// `_skipped_webpage_data`: `webpage_skip`, and the webpage's player
/// response when `player_js_version` forces a player.
fn skipped_webpage_data() -> Vec<String> {
    let mut skipped = super::args::list("webpage_skip");
    if player_js_version_arg() != "actual" {
        skipped.push("player_response".into());
    }
    skipped
}

/// `_download_initial_webpage`: the watch page (`bpctr`, `has_verified` and
/// `pp`), unless `player_skip=webpage`.
fn download_initial_webpage(
    client: &Client,
    video_id: &str,
    webpage_client: &innertube::ClientConfig,
) -> Result<Option<String>> {
    if super::args::contains("player_skip", "webpage") {
        return Ok(None);
    }
    let mut url = Url::parse(&format!("https://www.youtube.com/watch?v={video_id}"))?;
    {
        let mut query = url.query_pairs_mut();
        query.append_pair("bpctr", "9999999999");
        query.append_pair("has_verified", "1");
        let pp = super::args::first_case("player_params")
            .filter(|pp| !pp.is_empty())
            .or_else(|| webpage_client.player_params.map(str::to_owned));
        if let Some(pp) = pp {
            query.append_pair("pp", &pp);
        }
    }
    let default = default_ytcfg(webpage_client);
    let user_agent = default
        .get("INNERTUBE_CONTEXT")
        .and_then(|context| context.pointer("/client/userAgent"))
        .and_then(Value::as_str);
    let headers: Vec<(&str, &str)> = user_agent
        .map(|user_agent| ("User-Agent", user_agent))
        .into_iter()
        .collect();
    super::super::download_webpage_note_headers(
        client,
        &url,
        "youtube",
        video_id,
        "Downloading webpage",
        &headers,
    )
    .map(Some)
}

/// `_download_initial_data`: the webpage's initial data (unless
/// `webpage_skip=initial_data`), else the `next` API's (unless
/// `player_skip=initial_data`).
fn download_initial_data(
    client: &Client,
    video_id: &str,
    webpage: Option<&str>,
    webpage_client: &innertube::ClientConfig,
    webpage_ytcfg: &Map<String, Value>,
    skipped: &[String],
) -> Result<Option<Value>> {
    static INITIAL_DATA: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?:window\s*\[\s*["']ytInitialData["']\s*\]|ytInitialData)\s*="#).unwrap()
    });
    let mut initial_data = None;
    if let Some(webpage) = webpage
        && !skipped.iter().any(|skip| skip == "initial_data")
    {
        initial_data = search_initial(webpage, &INITIAL_DATA, "yt initial data");
        if !super::tab::truthy(initial_data.as_ref().and_then(|data| data.get("contents"))) {
            crate::screen::warning(
                "[youtube] Incomplete data received in embedded initial data; re-fetching using API.",
            );
            initial_data = None;
        }
    }
    if initial_data.is_none() && !super::args::contains("player_skip", "initial_data") {
        let mut query = Map::new();
        query.insert("videoId".into(), video_id.into());
        query.insert("contentCheckOk".into(), true.into());
        query.insert("racyCheckOk".into(), true.into());
        let default = default_ytcfg(webpage_client);
        // `_call_api` generates its headers without the ytcfg.
        let request = ApiRequest::new(
            webpage_client,
            "next",
            "youtube",
            video_id,
            api_context(webpage_ytcfg, &default),
            query,
            api_headers(webpage_client, &Map::new(), None, Session::default()),
            "Downloading initial data API JSON",
        )?;
        initial_data =
            extract_response(client, &request, Some("contents"), false).map_err(|error| {
                match error {
                    // An `ExtractorError` raised to `extract` gets the video ID.
                    Error::Reported(message) => Error::Reported(
                        message
                            .strip_prefix("[youtube] ")
                            .map_or(message.clone(), |rest| {
                                format!("[youtube] {video_id}: {rest}")
                            }),
                    ),
                    other => other,
                }
            })?;
    }
    Ok(initial_data)
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
    /// The webpage's `ytcfg` (`master_ytcfg`).
    pub webpage_ytcfg: Map<String, Value>,
}

/// `_initial_extract`.
pub(super) fn acquire(
    client: &Client,
    video_id: &str,
    runtime: Option<&challenge::Runtime>,
    music_url: bool,
) -> Result<Acquired> {
    let webpage_client_name = webpage_client();
    let web = innertube::client(webpage_client_name).expect("webpage client");
    let webpage = download_initial_webpage(client, video_id, web)?;
    let mut webpage_ytcfg = webpage.as_deref().map(extract_ytcfg).unwrap_or_default();
    if webpage_ytcfg.is_empty() {
        webpage_ytcfg = default_ytcfg(web);
    }
    let skipped_webpage = skipped_webpage_data();
    static INITIAL_PLAYER_RESPONSE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"ytInitialPlayerResponse\s*=").unwrap());
    let initial_data = download_initial_data(
        client,
        video_id,
        webpage.as_deref(),
        web,
        &webpage_ytcfg,
        &skipped_webpage,
    )?;
    // `_get_requested_clients` runs before `_extract_player_responses`
    // searches the initial player response.
    let authenticated = super::has_auth_cookies();
    let premium = is_premium_subscriber(authenticated, initial_data.as_ref());
    if premium {
        crate::screen::extractor_debug("Detected YouTube Premium subscription");
    }
    let clients = requested_clients(&ClientSelection {
        js_available: runtime.is_some(),
        authenticated,
        premium,
        music_url,
        video_id,
    })?;
    let initial_pr = webpage.as_deref().and_then(|webpage| {
        search_initial(
            webpage,
            &INITIAL_PLAYER_RESPONSE,
            &format!("{webpage_client_name} client initial player response"),
        )
    });
    let webpage = webpage.unwrap_or_default();

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
    let skip_configs = super::args::contains("player_skip", "configs");
    let skip_js = super::args::contains("player_skip", "js");
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
        let mut player_ytcfg = if config.name == webpage_client_name {
            webpage_ytcfg.clone()
        } else {
            Map::new()
        };
        if !skip_configs && config.name != webpage_client_name {
            let downloaded = download_ytcfg(client, config, video_id);
            if !downloaded.is_empty() {
                player_ytcfg = downloaded;
            }
        }
        if player_url_found.is_none() {
            player_url_found = player_url(&[&webpage_ytcfg, &player_ytcfg]);
        }
        let mut require_js_player = config.require_js_player;
        if skip_js {
            require_js_player = false;
            player_url_found = None;
        }
        if player_url_found.is_none() && !tried_iframe_fallback && require_js_player {
            player_url_found = iframe_player_url(client, video_id);
            tried_iframe_fallback = true;
        }
        let reused = (config.name == webpage_client_name
            && !skipped_webpage.iter().any(|skip| skip == "player_response"))
        .then(|| initial_pr.clone())
        .flatten();
        let initial_value = initial_pr.clone().unwrap_or(Value::Null);
        let player_ytcfg_value = Value::Object(player_ytcfg.clone());
        if visitor.is_none() {
            visitor = visitor_data(&[&webpage_ytcfg_value, &initial_value, &player_ytcfg_value]);
        }
        if data_sync.is_none() {
            data_sync = data_sync_id(&[&webpage_ytcfg_value, &initial_value, &player_ytcfg_value]);
        }
        // `fetch_po_token_args`, with `player_ytcfg or _get_default_ytcfg(client)`.
        let client_ytcfg = if player_ytcfg.is_empty() {
            default_ytcfg(config)
        } else {
            player_ytcfg.clone()
        };
        let pot_request = super::pot::Request {
            client: config.name,
            video_id: video_id.to_owned(),
            visitor_data: visitor.clone(),
            data_sync_id: data_sync.clone().filter(|_| authenticated),
            player_url: player_url_found.clone().filter(|_| require_js_player),
            authenticated,
            bind_to_video_id: super::pot::binds_to_video_id(&client_ytcfg),
        };
        // A webpage player response needs no player PO token.
        let player_po_token = match reused {
            Some(_) => None,
            None => pot_request.fetch(
                super::pot::Context::Player,
                config.player.required || config.player.recommended,
            ),
        };
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
                player_po_token.as_deref(),
            ) {
                Ok(response) => response.unwrap_or(Value::Null),
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
            let innertube_client_name = client_ytcfg
                .get("INNERTUBE_CONTEXT")
                .and_then(|context| context.pointer("/client/clientName"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            player_responses.push(response.clone());
            responses.push(streams::ClientResponse {
                client: config.name,
                response: response.clone(),
                player_token_provided: player_po_token.is_some(),
                available_at: available_at(&response, video_id, config.name),
                pot: pot_request,
                innertube_client_name,
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
        webpage_ytcfg,
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
