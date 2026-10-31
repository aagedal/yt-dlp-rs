//! Upstream `DiscoveryPlusBaseIE` (`yt_dlp/extractor/dplay.py`) and its
//! classes `CookingChannelIE`, `DiscoveryLifeIE`, `GoDiscoveryIE`,
//! `HGTVUsaIE`, `InvestigationDiscoveryIE` and `ScienceChannelIE`: the
//! disco-api video page (`token`, `content/videos` and
//! `playback/v3/videoPlaybackInfo`) with its HLS and DASH manifests. The
//! token's `st` cookie and cached token are honoured like upstream's
//! `_get_auth`. Not ported: `DPlayIE` and the other classes of the module,
//! `DiscoveryPlusIndiaIE` (its DASH manifest uses dependent representations,
//! which the native DASH parser rejects), the error-body messages for HTTP
//! 400/403 responses (they come out as the generic download error), and the
//! geo-bypass countries.
use super::{
    Extractor, dict_format, dict_info, download_json_headers, download_json_note,
    extract_m3u8_master_optional, extract_mpd_optional_note, post_json_note_headers,
};
use crate::{Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use url::Url;

/// `_DISCO_CLIENT_VER`.
const DISCO_CLIENT_VER: &str = "27.43.0";

/// `_PATH_REGEX` (`_match_id` is its `id` group).
const PATH_REGEX: &str = r"/(?P<id>[^/]+/[^/?#]+)";

/// `_auth_token_cache`: the anonymous token per (disco base, realm).
static TOKEN_CACHE: Mutex<Option<HashMap<(String, String), String>>> = Mutex::new(None);

/// One class's `_PRODUCT` and `_DISCO_API_PARAMS`.
struct Site {
    product: &'static str,
    disco_host: &'static str,
    realm: &'static str,
    /// `domain` (the `http_headers` Referer).
    domain: &'static str,
}

/// Upstream `DiscoveryPlusBaseIE._real_extract` for one class.
fn extract(
    client: &Client,
    url: &Url,
    name: &'static str,
    valid: &Regex,
    site: &Site,
) -> Result<Option<VideoInfo>> {
    let Some(found) = valid.captures(url.as_str()) else {
        return Ok(None);
    };
    let display_id = found["id"].to_owned();
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    get_disco_api_info(client, url, name, &display_id, site)
}

/// Upstream `_get_disco_api_info`.
fn get_disco_api_info(
    client: &Client,
    url: &Url,
    name: &'static str,
    display_id: &str,
    site: &Site,
) -> Result<Option<VideoInfo>> {
    let disco_base = format!("https://{}/", site.disco_host);
    let auth = get_auth(client, &disco_base, name, display_id, site.realm)?;
    let params = format!("realm={},siteLookupKey={}", site.realm, site.product);
    let client_header = format!("WEB:UNKNOWN:{}:{DISCO_CLIENT_VER}", site.product);
    let headers = [
        ("Referer", url.as_str()),
        ("x-disco-params", params.as_str()),
        ("x-disco-client", client_header.as_str()),
        ("Authorization", auth.as_str()),
    ];
    let content = super::update_url_query(
        &Url::parse(&format!("{disco_base}content/videos/{display_id}"))?,
        &[
            ("fields[channel]", "name"),
            ("fields[image]", "height,src,width"),
            ("fields[show]", "name"),
            ("fields[tag]", "name"),
            (
                "fields[video]",
                "description,episodeNumber,name,publishStart,seasonNumber,videoDuration",
            ),
            ("include", "images,primaryChannel,show,tags"),
        ],
    );
    let video = download_json_headers(client, &content, name, display_id, &headers)?;
    let data = video
        .get("data")
        .ok_or_else(|| key_error(display_id, "data"))?;
    let video_id = data
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(display_id, "id"))?
        .to_owned();
    let info = data
        .get("attributes")
        .ok_or_else(|| key_error(display_id, "attributes"))?;
    let title = info
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(display_id, "name"))?
        .trim()
        .to_owned();

    let mut formats = Vec::new();
    let mut subtitles = Subtitles::default();
    let playback = post_json_note_headers(
        client,
        &Url::parse(&format!("{disco_base}playback/v3/videoPlaybackInfo"))?,
        name,
        &video_id,
        "Downloading JSON metadata",
        &headers,
        json!({
            "deviceInfo": {"adBlocker": false, "drmSupported": false},
            "videoId": video_id,
            "wisteriaProperties": {},
        })
        .to_string(),
    )?;
    let streaming = playback
        .get("data")
        .and_then(|data| data.get("attributes"))
        .and_then(|attributes| attributes.get("streaming"))
        .ok_or_else(|| key_error(&video_id, "streaming"))?;
    for format_dict in streaming.as_array().into_iter().flatten() {
        let Some(format_dict) = format_dict.as_object() else {
            continue;
        };
        let Some(format_url) = format_dict
            .get("url")
            .and_then(Value::as_str)
            .filter(|text| !text.is_empty())
        else {
            continue;
        };
        let format_id = format_dict.get("type").cloned().unwrap_or(Value::Null);
        let ext = super::direct::determine_ext(format_url);
        let Ok(manifest) = Url::parse(format_url) else {
            continue;
        };
        if format_id.as_str() == Some("dash") || ext.as_deref() == Some("mpd") {
            if let Some(stream) = extract_mpd_optional_note(
                client,
                &manifest,
                name,
                display_id,
                Some("dash"),
                "Downloading MPD manifest",
            ) {
                formats.extend(stream.formats);
                super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else if format_id.as_str() == Some("hls") || ext.as_deref() == Some("m3u8") {
            if let Some(stream) = extract_m3u8_master_optional(
                client,
                &manifest,
                name,
                display_id,
                Some("hls"),
                true,
                false,
            ) {
                formats.extend(stream.formats);
                super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else {
            formats.push(dict_format(
                vec![("url", format_url.into()), ("format_id", format_id)],
                name,
                display_id,
            )?);
        }
    }

    let mut creator = Value::Null;
    let mut series = Value::Null;
    let mut tags = Vec::new();
    let mut thumbnails = Vec::new();
    for entry in video
        .get("included")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(attributes) = entry
            .get("attributes")
            .filter(|value| super::nuxt::truthy(value))
        else {
            continue;
        };
        match entry.get("type").and_then(Value::as_str) {
            Some("channel") => creator = attributes.get("name").cloned().unwrap_or(Value::Null),
            Some("image") => {
                if let Some(src) = attributes
                    .get("src")
                    .filter(|value| super::nuxt::truthy(value))
                {
                    thumbnails.push(json!({
                        "url": src,
                        "width": super::streamable::int_or_none(attributes.get("width")),
                        "height": super::streamable::int_or_none(attributes.get("height")),
                    }));
                }
            }
            _ => {}
        }
        if entry.get("type").and_then(Value::as_str) == Some("show") {
            series = attributes.get("name").cloned().unwrap_or(Value::Null);
        } else if entry.get("type").and_then(Value::as_str) == Some("tag")
            && let Some(name) = attributes
                .get("name")
                .filter(|value| super::nuxt::truthy(value))
        {
            tags.push(name.clone());
        }
    }

    // `strip_or_none`: an empty description is `None`.
    let description = match info.get("description") {
        Some(Value::String(text)) if !text.trim().is_empty() => Value::from(text.trim()),
        _ => Value::Null,
    };
    let mut result = dict_info(
        url,
        (name, name),
        vec![
            ("id", Value::from(video_id.clone())),
            ("display_id", Value::from(display_id)),
            ("title", Value::from(title)),
            ("description", description),
            (
                "duration",
                info.get("videoDuration")
                    .and_then(Value::as_f64)
                    .map_or(Value::Null, |milliseconds| {
                        Value::from(milliseconds / 1000.0)
                    }),
            ),
            (
                "timestamp",
                info.get("publishStart")
                    .and_then(Value::as_str)
                    .and_then(|date| crate::dates::unified_timestamp(date, false))
                    .map_or(Value::Null, Value::from),
            ),
            ("series", series),
            (
                "season_number",
                super::streamable::int_or_none(info.get("seasonNumber")),
            ),
            (
                "episode_number",
                super::streamable::int_or_none(info.get("episodeNumber")),
            ),
            ("creator", creator),
            ("tags", Value::Array(tags)),
            ("thumbnails", Value::Null),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("http_headers", json!({"Referer": site.domain})),
        ],
        formats,
    );
    result.subtitles = subtitles;
    // The raw dicts keep upstream's key order (`url`, `width`, `height`).
    result
        .extra
        .insert("thumbnails".into(), Value::Array(thumbnails));
    Ok(Some(result))
}

/// Upstream `DPlayBaseIE._get_auth` (`needs_device_id=True`).
fn get_auth(
    client: &Client,
    disco_base: &str,
    name: &'static str,
    display_id: &str,
    realm: &str,
) -> Result<String> {
    let key = (disco_base.to_owned(), realm.to_owned());
    let cookie = crate::cookies::get_cookie(disco_base, "st").filter(|value| !value.is_empty());
    let cached = TOKEN_CACHE
        .lock()
        .ok()
        .and_then(|cache| cache.as_ref().and_then(|cache| cache.get(&key).cloned()));
    let token = match cookie.or(cached) {
        Some(token) => token,
        None => {
            let device_id = device_id();
            let url = super::update_url_query(
                &Url::parse(&format!("{disco_base}token"))?,
                &[("realm", realm), ("deviceId", &device_id)],
            );
            let response = download_json_note(client, &url, name, display_id, "Downloading token")?;
            let token = response
                .get("data")
                .and_then(|data| data.get("attributes"))
                .and_then(|attributes| attributes.get("token"))
                .and_then(Value::as_str)
                .ok_or_else(|| key_error(display_id, "token"))?
                .to_owned();
            // Cached only when no `st` cookie is set.
            if crate::cookies::get_cookie(disco_base, "st").is_none()
                && let Ok(mut cache) = TOKEN_CACHE.lock()
            {
                cache
                    .get_or_insert_with(HashMap::new)
                    .insert(key, token.clone());
            }
            token
        }
    };
    Ok(format!("Bearer {token}"))
}

/// A random UUID-shaped device ID (upstream draws a fresh `uuid4()`).
fn device_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(std::process::id().into());
    let high = hasher.finish();
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(high);
    format!("{high:016x}{:016x}", hasher.finish())
}

fn key_error(id: &str, key: &str) -> crate::Error {
    super::clyp::key_error(id, key)
}

/// Declares an upstream class: its `_VALID_URL` (`prefix` + `_PATH_REGEX`)
/// and its [`Site`].
macro_rules! disco_class {
    ($struct:ident, $name:literal, $suitable:ident, $valid:ident, $prefix:literal, $site:expr) => {
        static $valid: LazyLock<Regex> =
            LazyLock::new(|| super::url_regex(&format!("^{}{}", $prefix, PATH_REGEX)).unwrap());

        /// `_match_valid_url`.
        pub fn $suitable(url: &str) -> bool {
            $valid.is_match(url)
        }

        pub struct $struct;

        impl Extractor for $struct {
            fn key(&self) -> &'static str {
                $name
            }

            fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
                extract(client, url, $name, &$valid, &$site)
            }
        }
    };
}

disco_class!(
    CookingChannel,
    "CookingChannel",
    cooking_channel_suitable,
    COOKING_CHANNEL_VALID,
    r"https?://(?:watch\.)?cookingchanneltv\.com/video",
    Site {
        product: "cook",
        disco_host: "us1-prod-direct.watch.cookingchanneltv.com",
        realm: "go",
        domain: "",
    }
);

disco_class!(
    DiscoveryLife,
    "DiscoveryLife",
    discovery_life_suitable,
    DISCOVERY_LIFE_VALID,
    r"https?://(?:www\.)?discoverylife\.com/video",
    Site {
        product: "dlf",
        disco_host: "us1-prod-direct.discoverylife.com",
        realm: "go",
        domain: "",
    }
);

disco_class!(
    GoDiscovery,
    "GoDiscovery",
    go_discovery_suitable,
    GO_DISCOVERY_VALID,
    r"https?://(?:go\.)?discovery\.com/video",
    Site {
        product: "dsc",
        disco_host: "us1-prod-direct.go.discovery.com",
        realm: "go",
        domain: "",
    }
);

disco_class!(
    HGTVUsa,
    "HGTVUsa",
    hgtv_usa_suitable,
    HGTV_USA_VALID,
    r"https?://(?:watch\.)?hgtv\.com/video",
    Site {
        product: "hgtv",
        disco_host: "us1-prod-direct.watch.hgtv.com",
        realm: "go",
        domain: "",
    }
);

disco_class!(
    InvestigationDiscovery,
    "InvestigationDiscovery",
    investigation_discovery_suitable,
    INVESTIGATION_DISCOVERY_VALID,
    r"https?://(?:www\.)?investigationdiscovery\.com/video",
    Site {
        product: "ids",
        disco_host: "us1-prod-direct.investigationdiscovery.com",
        realm: "go",
        domain: "",
    }
);

disco_class!(
    ScienceChannel,
    "ScienceChannel",
    science_channel_suitable,
    SCIENCE_CHANNEL_VALID,
    r"https?://(?:www\.)?sciencechannel\.com/video",
    Site {
        product: "sci",
        disco_host: "us1-prod-direct.sciencechannel.com",
        realm: "go",
        domain: "",
    }
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(cooking_channel_suitable(
            "https://watch.cookingchanneltv.com/video/bobbys-triple-threat-food-network-atve-us/titans-vs-marcus-samuelsson"
        ));
        assert!(discovery_life_suitable(
            "https://www.discoverylife.com/video/er-files-discovery-life-atve-us/sweet-charity"
        ));
        assert!(go_discovery_suitable(
            "https://discovery.com/video/dirty-jobs-discovery-atve-us/rodbuster-galvanizer"
        ));
        assert!(hgtv_usa_suitable(
            "https://watch.hgtv.com/video/home-inspector-joe-hgtv-atve-us/this-mold-house"
        ));
        assert!(investigation_discovery_suitable(
            "https://www.investigationdiscovery.com/video/unmasked-investigation-discovery/the-killer-clown"
        ));
        assert!(science_channel_suitable(
            "https://www.sciencechannel.com/video/strangest-things-science-atve-us/nazi-mystery-machine"
        ));
        assert!(!cooking_channel_suitable(
            "https://watch.cookingchanneltv.com/"
        ));
    }
}
