//! Narrow watch-page slice of YoutubeIE at the pinned upstream revision.
//! Bounded WEB API fallback, native HLS VOD and explicit offline JS runtime.
mod challenge;
mod chapters;
pub(crate) use chapters::from_description as chapters_from_description;
pub(crate) mod dict;
mod innertube;
mod metadata;
pub(crate) mod search;
mod streams;
mod subs;
pub(crate) mod tab;
mod watch;
use super::{Extractor, direct};
use crate::{Error, Result, StreamKind, VideoInfo};
use reqwest::blocking::Client;
use serde_json::Value;
use std::path::PathBuf;
use url::Url;

pub struct Youtube;

/// Opt-in YouTube challenge runtime. Defaults to challenge-free extraction.
#[derive(Debug, Clone, Default)]
pub struct YoutubeOptions {
    /// Explicit Deno 2.3+ or Node.js 22+ executable path; never discovered or installed.
    /// Inspected only when an eligible format has a supported URL challenge.
    pub js_runtime: Option<PathBuf>,
    /// Include translated manual captions in automatic-caption metadata.
    /// Original ASR captions/translations are always discovered without fetching.
    pub include_translated_subtitles: bool,
    /// Upstream `noplaylist`: a watch URL with `list=` downloads just its video.
    pub no_playlist: bool,
}

struct ConfiguredYoutube {
    include_translated_subtitles: bool,
    no_playlist: bool,
    runtime: Option<challenge::Runtime>,
}

impl Youtube {
    pub fn with_options(options: YoutubeOptions) -> impl Extractor {
        // The player JS and player API requests use the extraction client,
        // with its headers, cookies and proxy.
        ConfiguredYoutube {
            include_translated_subtitles: options.include_translated_subtitles,
            no_playlist: options.no_playlist,
            runtime: options.js_runtime.map(challenge::Runtime::new),
        }
    }
}

/// Upstream `YoutubeBaseInfoExtractor._real_initialize`: `_initialize_pref`
/// (English, UTC) and `_initialize_consent` (accept all; otherwise EU
/// requests are redirected to consent.youtube.com).
pub(crate) fn initialize() {
    const HOME: &str = "https://www.youtube.com";
    let mut pref: Vec<(String, String)> = crate::cookies::get_cookie(HOME, "PREF")
        .map(|value| {
            url::form_urlencoded::parse(value.as_bytes())
                .filter(|(_, value)| !value.is_empty())
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect()
        })
        .unwrap_or_default();
    // `dict.update`: replaced keys keep their position.
    for (key, value) in [("hl", "en"), ("tz", "UTC")] {
        match pref.iter_mut().find(|(name, _)| name == key) {
            Some(entry) => entry.1 = value.into(),
            None => pref.push((key.into(), value.into())),
        }
    }
    let pref = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&pref)
        .finish();
    crate::cookies::set_cookie(".youtube.com", "PREF", &pref);
    if has_auth_cookies() {
        // `_initialize_cookie_auth`.
        crate::screen::extractor_debug("Found YouTube account cookies");
        return;
    }
    // A SOCS value not starting with `CAA` records a choice; keep it.
    if crate::cookies::get_cookie(HOME, "SOCS").is_some_and(|socs| !socs.starts_with("CAA")) {
        return;
    }
    crate::cookies::set_cookie(".youtube.com", "SOCS", "CAI");
}

const YOUTUBE_HOME: &str = "https://www.youtube.com";

/// `_get_sid_cookies`: SAPISID (else `__Secure-3PAPISID`), `__Secure-1PAPISID`
/// and `__Secure-3PAPISID`.
fn sid_cookies() -> [Option<String>; 3] {
    let cookie = |name: &str| {
        crate::cookies::get_cookie(YOUTUBE_HOME, name).filter(|value| !value.is_empty())
    };
    let third = cookie("__Secure-3PAPISID");
    [
        cookie("SAPISID").or_else(|| third.clone()),
        cookie("__Secure-1PAPISID"),
        third,
    ]
}

/// `_has_auth_cookies` (`is_authenticated`): `LOGIN_INFO` and a SID cookie.
pub(crate) fn has_auth_cookies() -> bool {
    crate::cookies::get_cookie(YOUTUBE_HOME, "LOGIN_INFO").is_some()
        && sid_cookies().iter().any(Option::is_some)
}

/// `_get_sid_authorization_header`: one `SCHEME TIMESTAMP_HASH[_u]` per SID
/// cookie, hashing `[USER_SESSION_ID ]TIMESTAMP SID ORIGIN` with SHA-1.
pub(super) fn sid_authorization(origin: &str, user_session_id: Option<&str>) -> Option<String> {
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs_f64().round() as u64)
        .to_string();
    sid_authorization_at(origin, user_session_id, &timestamp, sid_cookies())
}

fn sid_authorization_at(
    origin: &str,
    user_session_id: Option<&str>,
    timestamp: &str,
    sids: [Option<String>; 3],
) -> Option<String> {
    let timestamp = timestamp.to_owned();
    let authorizations: Vec<String> = ["SAPISIDHASH", "SAPISID1PHASH", "SAPISID3PHASH"]
        .into_iter()
        .zip(sids)
        .filter_map(|(scheme, sid)| {
            let sid = sid?;
            let mut hashed = Vec::new();
            if let Some(user) = user_session_id {
                hashed.push(user.to_owned());
            }
            hashed.extend([timestamp.clone(), sid, origin.to_owned()]);
            let hash: String = crate::sha1::digest(hashed.join(" ").as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let mut parts = vec![timestamp.clone(), hash];
            if user_session_id.is_some() {
                // `''.join(additional_parts)` joins the dict's keys.
                parts.push("u".into());
            }
            Some(format!("{scheme} {}", parts.join("_")))
        })
        .collect();
    (!authorizations.is_empty()).then(|| authorizations.join(" "))
}

fn unsupported(reason: &str) -> Error {
    Error::UnsupportedYoutube(reason.into())
}

fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// `YoutubeIE._EMBED_REGEX` through the default `_extract_embed_urls`:
/// matches unescaped, joined to `url`, kept when suitable, without repeats.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    static PATTERNS: std::sync::LazyLock<[regex::Regex; 2]> = std::sync::LazyLock::new(|| {
        let quoted = |q: &str| {
            format!(
                r#"{q}(?P<u{}>(?:https?:)?//(?:www\.)?youtube(?:-nocookie)?\.com/(?:embed|v|p)/[0-9A-Za-z_-]{{11}}[^\n]*?){q}"#,
                if q == "\"" { 1 } else { 2 }
            )
        };
        [
            regex::Regex::new(&format!(
                r#"(?:<(?:[0-9A-Za-z-]+?)?iframe[^>]+?src=|data-video-url=|<embed[^>]+?src=|embedSWF\(?:\s*|<object[^>]+data=|new\s+SWFObject\()(?:{}|{})"#,
                quoted("\""),
                quoted("'")
            ))
            .expect("static pattern"),
            regex::Regex::new(
                r#"(?s)<a\s[^>]*\bhref="(?P<u1>https://www\.youtube\.com/watch\?v=[0-9A-Za-z_-]{11})"\s[^>]*\bclass="[^"]*\blazy-load-youtube"#,
            )
            .expect("static pattern"),
        ]
    });
    let mut found: Vec<Url> = Vec::new();
    for regex in PATTERNS.iter() {
        for captures in regex.captures_iter(webpage) {
            let Some(group) = captures.name("u1").or_else(|| captures.name("u2")) else {
                continue;
            };
            let Ok(embed) = url.join(&crate::html::unescape_html(group.as_str())) else {
                continue;
            };
            if suitable(embed.as_str()) && !found.contains(&embed) {
                found.push(embed);
            }
        }
    }
    found
}

/// `_match_id`'s ID of a suitable URL.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    Url::parse(url)
        .ok()
        .and_then(|url| video_id(&url, false).ok().flatten())
}

/// Upstream `YoutubeIE.suitable`: a supported single-video URL.
pub fn suitable(url: &str) -> bool {
    Url::parse(url).is_ok_and(|url| matches!(video_id(&url, false), Ok(Some(_))))
}

// Claim all URLs on these exact hosts so unsupported site pages cannot fall
// through to generic HTML5 extraction and appear to be supported YouTube videos.
fn video_id(url: &Url, no_playlist: bool) -> Result<Option<String>> {
    let host = url.host_str().unwrap_or("");
    if !matches!(
        host,
        "youtube.com"
            | "www.youtube.com"
            | "m.youtube.com"
            | "music.youtube.com"
            | "youtu.be"
            | "www.youtu.be"
            | "www.youtube-nocookie.com"
            | "youtube-nocookie.com"
    ) {
        return Ok(None);
    }
    if url.query_pairs().any(|(key, _)| key == "list") {
        // Upstream `YoutubeTabIE._yes_playlist`: only `--no-playlist` resolves
        // a watch URL with both IDs to its single video.
        let ids: Vec<_> = url.query_pairs().filter(|(key, _)| key == "v").collect();
        match (no_playlist && url.path() == "/watch", ids.as_slice()) {
            (true, [(_, id)]) if valid_id(id) => {
                crate::screen::screen(&format!(
                    "[youtube:tab] Downloading just the video {id} because of --no-playlist"
                ));
                return Ok(Some(id.clone().into_owned()));
            }
            _ => {
                return Err(unsupported(
                    "playlists and video URLs with list= are not supported",
                ));
            }
        }
    }
    let id = if matches!(host, "youtu.be" | "www.youtu.be") {
        url.path()
            .strip_prefix('/')
            .filter(|s| !s.contains('/'))
            .map(str::to_owned)
    } else if url.path() == "/watch" {
        let ids: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| key == "v")
            .map(|(_, value)| value.into_owned())
            .collect();
        if ids.len() != 1 {
            None
        } else {
            ids.into_iter().next()
        }
    } else {
        ["/shorts/", "/embed/", "/live/"]
            .iter()
            .find_map(|prefix| url.path().strip_prefix(prefix))
            .map(str::to_owned)
    };
    id.filter(|id| valid_id(id))
        .map(Some)
        .ok_or_else(|| unsupported("expected a single video URL with an 11-character ID"))
}
/// `_real_extract`'s reasons for a video without formats: DRM, then the
/// playability status's error screen (`raise_no_formats(reason)`).
fn unplayable_reason(id: &str, player_responses: &[Value]) -> Option<Error> {
    let fail = |message: &str| Error::Reported(format!("[youtube] {id}: {message}"));
    if player_responses.iter().any(|response| {
        response
            .pointer("/streamingData/licenseInfos")
            .is_some_and(|infos| infos.as_array().is_some_and(|infos| !infos.is_empty()))
    }) {
        return Some(fail("This video is DRM protected"));
    }
    let statuses: Vec<&Value> = player_responses
        .iter()
        .filter_map(|response| response.get("playabilityStatus").filter(|s| s.is_object()))
        .collect();
    let pemr = statuses
        .iter()
        .find_map(|status| {
            status
                .pointer("/errorScreen/playerErrorMessageRenderer")
                .filter(|renderer| renderer.is_object())
        })
        .cloned()
        .unwrap_or(Value::Null);
    let mut reason = pemr.get("reason").and_then(metadata::text).or_else(|| {
        statuses
            .iter()
            .find_map(|status| status.get("reason").and_then(Value::as_str))
            .map(str::to_owned)
    })?;
    let subreason = pemr
        .get("subreason")
        .and_then(metadata::text)
        .map(|text| crate::html::clean_html(&text))
        .unwrap_or_default();
    if !subreason.is_empty() {
        if subreason.starts_with("The uploader has not made this video available in your country") {
            // `raise_geo_restricted` (the available countries are not listed).
            return Some(Error::Reported(format!(
                "{subreason}\nYou might want to use a VPN or a proxy server (with --proxy) to workaround."
            )));
        }
        reason.push_str(&format!(". {subreason}"));
    }
    if reason.is_empty() {
        return None;
    }
    if reason.to_lowercase().contains("sign in") {
        let trimmed = reason
            .strip_suffix("This helps protect our community. Learn more")
            .unwrap_or(&reason)
            .trim();
        reason = format!(
            "{}. {}",
            trimmed.strip_suffix('.').unwrap_or(trimmed),
            watch::LOGIN_HINT
        );
    } else if statuses.iter().any(|status| {
        status
            .pointer("/errorScreen/playerCaptchaViewModel")
            .is_some_and(Value::is_object)
    }) {
        reason.push_str(". YouTube is requiring a captcha challenge before playback");
    } else if reason.contains("This content isn't available, try again later") {
        let trimmed = reason.trim();
        reason = format!(
            "{}. The current session has been rate-limited by YouTube for up to an hour. It is recommended to use `-t sleep` to add a delay between video requests to avoid exceeding the rate limit. For more information, refer to  https://github.com/yt-dlp/yt-dlp/wiki/Extractors#this-content-isnt-available-try-again-later",
            trimmed.strip_suffix('.').unwrap_or(trimmed)
        );
    }
    Some(fail(&reason))
}

/// `process_video_result`'s `raise_no_formats` for a video without formats
/// or a reason (skipped formats were warned about while extracting).
fn no_formats(id: &str) -> Error {
    Error::Reported(format!(
        "[youtube] {id}: No video formats found!; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `solve_js_challenges` through the configured runtime: the player JS
/// (`Downloading player ID-VARIANT`) and one EJS batch.
struct JsSolver<'a> {
    client: &'a Client,
    runtime: Option<&'a challenge::Runtime>,
    player_url: Option<String>,
    video_id: &'a str,
}

impl streams::Solver for JsSolver<'_> {
    fn solve(
        &mut self,
        n: &std::collections::BTreeSet<String>,
        sig: &std::collections::BTreeSet<usize>,
    ) -> (
        std::collections::HashMap<String, String>,
        std::collections::HashMap<usize, Vec<usize>>,
    ) {
        let mut n_results = std::collections::HashMap::new();
        let mut sig_results = std::collections::HashMap::new();
        let (Some(runtime), Some(player_url)) = (self.runtime, self.player_url.as_deref()) else {
            return (n_results, sig_results);
        };
        let Ok(runtime) = runtime.checked() else {
            return (n_results, sig_results);
        };
        let Some(player) = watch::load_player(self.client, self.video_id, player_url) else {
            return (n_results, sig_results);
        };
        // Signature specs are solved as `chr(0..LENGTH)` permutations.
        let identities: Vec<(usize, String)> = sig
            .iter()
            .map(|length| {
                (
                    *length,
                    (0..*length)
                        .filter_map(|i| char::from_u32(i as u32))
                        .collect(),
                )
            })
            .collect();
        let mut inputs: Vec<(challenge::Kind, &str)> = n
            .iter()
            .map(|challenge| (challenge::Kind::N, challenge.as_str()))
            .collect();
        inputs.extend(
            identities
                .iter()
                .map(|(_, identity)| (challenge::Kind::Sig, identity.as_str())),
        );
        let Ok(batch) = challenge::Batch::new(&player, &inputs) else {
            return (n_results, sig_results);
        };
        let name = runtime.name();
        crate::screen::screen(&format!(
            "[youtube] [jsc:{name}] Solving JS challenges using {name}"
        ));
        let Ok(solutions) = runtime.solve(&batch) else {
            return (n_results, sig_results);
        };
        for challenge in n {
            if let Some(result) = solutions.get(challenge::Kind::N, challenge) {
                n_results.insert(challenge.clone(), result.to_owned());
            }
        }
        for (length, identity) in &identities {
            if let Some(result) = solutions.get(challenge::Kind::Sig, identity) {
                sig_results.insert(*length, result.chars().map(|c| c as usize).collect());
            }
        }
        (n_results, sig_results)
    }
}

/// Upstream `_real_extract` over every requested client's player response.
fn extract_clients(
    client: &Client,
    url: &Url,
    id: &str,
    runtime: Option<&challenge::Runtime>,
    translated_manual: bool,
) -> Result<VideoInfo> {
    crate::screen::screen(&format!(
        "[youtube] Extracting URL: {}",
        crate::screen::extracting_url(&format!("https://www.youtube.com/watch?v={id}"))
    ));
    // `is_music_url`.
    let music_url = url.host_str() == Some("music.youtube.com");
    let acquired = watch::acquire(client, id, runtime, music_url)?;
    let first = |key: &str| {
        acquired
            .player_responses
            .iter()
            .find_map(|response| response.get(key).filter(|value| value.is_object()))
    };
    let details = first("videoDetails").cloned().unwrap_or(Value::Null);
    // `get_first(video_details, 'title') or search_meta([...])`.
    let title = details
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            ["og:title", "twitter:title", "title"]
                .into_iter()
                .find_map(|name| crate::html::html_search_meta(&acquired.webpage, name))
        })
        .unwrap_or_default();
    let duration = details
        .get("lengthSeconds")
        .and_then(|value| {
            value
                .as_str()?
                .parse::<i64>()
                .ok()
                .or_else(|| value.as_i64())
        })
        .filter(|duration| *duration != 0);
    // `_list_formats`' live status from the video details.
    let flag = |key: &str| details.get(key).and_then(Value::as_bool);
    let live_status = if flag("isPostLiveDvr") == Some(true) {
        Some("post_live")
    } else if flag("isLive") == Some(true) {
        Some("is_live")
    } else if flag("isUpcoming") == Some(true) {
        Some("is_upcoming")
    } else if flag("isLiveContent") == Some(true) {
        Some("was_live")
    } else if flag("isLive") == Some(false) || flag("isLiveContent") == Some(false) {
        Some("not_live")
    } else {
        None
    };
    // Upcoming videos have no formats yet; their playability reason is
    // reported below.
    if live_status == Some("post_live") {
        return Err(unsupported("post-live videos"));
    }
    let context = streams::Context {
        client,
        video_id: id,
        player_url: acquired.player_url.as_deref(),
        live_status,
        duration,
        verbose: crate::screen::verbose(),
        all_formats: false,
        skip_bad_formats: true,
        dashy: false,
        missing_pot: false,
        skip_player_js: false,
        skip_manifests: Default::default(),
        premium: acquired.premium,
    };
    let mut solver = JsSolver {
        client,
        runtime,
        player_url: acquired.player_url.clone(),
        video_id: id,
    };
    let streams = streams::extract(&context, &acquired.responses, &mut solver)?;
    let mut formats = Vec::new();
    let mut manifest_formats = streams
        .manifest_formats
        .into_iter()
        .map(Some)
        .collect::<Vec<_>>();
    for (manifest, index) in &streams.order {
        if *manifest {
            if let Some(format) = manifest_formats[*index].take() {
                formats.push(format);
            }
        } else {
            let mut format = streams::to_format(&streams.formats[*index], id)?;
            format.stream_kind = match (format.vcodec.as_deref(), format.acodec.as_deref()) {
                (Some("none"), _) => StreamKind::AudioOnly,
                (_, Some("none")) => StreamKind::VideoOnly,
                _ => StreamKind::Standalone,
            };
            crate::info_order::mark_format(&mut format, "youtube-https");
            formats.push(format);
        }
    }
    for dict in streams::storyboards(&acquired.player_responses, duration, id) {
        let mut format = streams::to_format(&dict, id)?;
        format.extra.insert("rows".into(), dict["rows"].clone());
        format
            .extra
            .insert("columns".into(), dict["columns"].clone());
        crate::info_order::mark_format(&mut format, "youtube-storyboard");
        formats.push(format);
    }
    // Captions from the client responses, into `_list_formats`' manifest
    // subtitles (the automatic captions' container).
    let clients: Vec<(&str, &Value)> = acquired
        .responses
        .iter()
        .map(|response| (response.client, &response.response))
        .collect();
    let manifest_subtitles = match serde_json::to_value(&streams.subtitles) {
        Ok(Value::Object(map)) => map,
        _ => Default::default(),
    };
    let captions = subs::extract(
        &acquired.player_responses,
        &clients,
        manifest_subtitles,
        translated_manual,
    );
    // `set_audio_lang_from_orig_subs_lang`: the first original language.
    if let Some(language) = captions.audio_languages.first() {
        for format in &mut formats {
            if format.acodec.as_deref() != Some("none") && format.language.is_none() {
                format.language = Some(language.clone());
            }
        }
    }
    let Some(first_format) = formats.first() else {
        return Err(
            unplayable_reason(id, &acquired.player_responses).unwrap_or_else(|| no_formats(id))
        );
    };
    let microformats: Vec<&Value> = acquired
        .player_responses
        .iter()
        .filter_map(|response| response.pointer("/microformat/playerMicroformatRenderer"))
        .collect();
    // `original_description if original_description is not None else
    // translated_description`, else the initial data's.
    let description = acquired
        .player_responses
        .iter()
        .find_map(|response| {
            response
                .pointer("/videoDetails/shortDescription")
                .filter(|value| !value.is_null())
        })
        .map(|value| value.as_str().unwrap_or_default().to_owned())
        .or_else(|| {
            microformats
                .iter()
                .find_map(|micro| micro.get("description").and_then(metadata::text))
        })
        .or_else(|| {
            acquired.initial_data.as_ref().and_then(|data| {
                data.get("engagementPanels")?
                    .as_array()?
                    .iter()
                    .filter_map(|panel| {
                        panel.pointer(
                            "/engagementPanelSectionListRenderer/content/structuredDescriptionContentRenderer/items",
                        )
                    })
                    .filter_map(Value::as_array)
                    .flatten()
                    .find_map(|item| {
                        item.pointer(
                            "/expandableVideoDescriptionBodyRenderer/attributedDescriptionBodyText/content",
                        )?
                        .as_str()
                        .map(str::to_owned)
                    })
            })
        });
    // `get_first(video_details, 'keywords', list) or [og:video:tag...]`.
    let mut keywords: Vec<String> = details
        .get("keywords")
        .and_then(Value::as_array)
        .map(|keywords| {
            keywords
                .iter()
                .filter_map(|keyword| keyword.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if keywords.is_empty() {
        static TAG: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
            regex::Regex::new(
                r#"(?is)<meta[^>]+?(?:itemprop|name|property|id|http-equiv)=(?:"og:video:tag"|'og:video:tag')[^>]+?content=(?:"([^"]*)"|'([^']*)')"#,
            )
            .unwrap()
        });
        keywords = TAG
            .captures_iter(&acquired.webpage)
            .filter_map(|found| found.get(1).or_else(|| found.get(2)))
            .map(|value| crate::html::unescape_html(value.as_str()))
            .collect();
    }
    let chapters = chapters::extract(
        acquired.initial_data.as_ref(),
        description.as_deref(),
        duration.and_then(|duration| u64::try_from(duration).ok()),
    );
    let dict = dict::build(&dict::Input {
        video_id: id,
        url,
        webpage: &acquired.webpage,
        player_responses: &acquired.player_responses,
        initial_data: acquired.initial_data.as_ref(),
        title: &title,
        description: description.as_deref(),
        duration,
        live_status,
        keywords,
        chapters: if chapters.is_empty() {
            Value::Null
        } else {
            serde_json::to_value(&chapters).unwrap_or(Value::Null)
        },
        automatic_captions: Value::Object(captions.automatic),
        subtitles: Value::Object(captions.subtitles),
    });
    let page = Url::parse(&format!("https://www.youtube.com/watch?v={id}"))?;
    let mut info = direct::build_at(first_format.url.clone(), &page, first_format.ext.clone());
    info.direct = None;
    info.id = id.into();
    info.title = title;
    info.webpage_url = page;
    info.extractor = "youtube".into();
    info.extractor_key = "Youtube".into();
    info.format_sort_fields = [
        "quality",
        "res",
        "fps",
        "hdr:12",
        "source",
        if crate::format_sort::PREFER_VP9_COMPAT.load(std::sync::atomic::Ordering::Relaxed) {
            "vcodec:vp9.2"
        } else {
            "vcodec"
        },
        "channels",
        "acodec",
        "lang",
        "proto",
    ]
    .map(str::to_owned)
    .to_vec();
    info.formats = formats;
    let mut keys: Vec<String> = Vec::new();
    for (key, value) in dict {
        keys.push(key.clone());
        if key == "title" {
            keys.push("formats".into());
        }
        if key == "release_timestamp" {
            keys.push("_format_sort_fields".into());
        }
        match key.as_str() {
            "id" | "title" | "webpage_url" => {}
            "subtitles" | "automatic_captions" => {
                let parsed: crate::Subtitles = serde_json::from_value(value).unwrap_or_default();
                if key == "subtitles" {
                    info.subtitles = parsed;
                } else {
                    info.automatic_captions = parsed;
                }
            }
            _ if value.is_null() => {
                info.extra.insert(key, Value::Null);
            }
            _ => crate::metadata_parser::set_field(&mut info, &key, value),
        }
    }
    Ok(crate::info_order::mark_info(
        info,
        &format!("youtube;{}", keys.join(",")),
    ))
}

impl Extractor for Youtube {
    fn key(&self) -> &'static str {
        "Youtube"
    }
    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(id) = video_id(url, false)? else {
            return Ok(None);
        };
        crate::screen::set_extractor("youtube");
        initialize();
        let info = extract_clients(client, url, &id, None, false)?;
        Ok(Some(info))
    }
}

impl Extractor for ConfiguredYoutube {
    fn key(&self) -> &'static str {
        "Youtube"
    }
    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(id) = video_id(url, self.no_playlist)? else {
            return Ok(None);
        };
        crate::screen::set_extractor("youtube");
        initialize();
        let info = extract_clients(
            client,
            url,
            &id,
            self.runtime.as_ref(),
            self.include_translated_subtitles,
        )?;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sid_authorization_matches_pinned_upstream() {
        // Recorded from pinned upstream's request for the youtube-auth fixture.
        let sid = || Some("sapisid-fixture/AbCdEf".to_owned());
        let hash = "1792620452_3448a83fcbe06bc33db20790bfab51bca47748db_u";
        assert_eq!(
            sid_authorization_at(
                "https://www.youtube.com",
                Some("USER123"),
                "1792620452",
                [sid(), sid(), sid()]
            )
            .unwrap(),
            format!("SAPISIDHASH {hash} SAPISID1PHASH {hash} SAPISID3PHASH {hash}")
        );
        let anonymous =
            sid_authorization_at("https://www.youtube.com", None, "1", [sid(), None, None])
                .unwrap();
        assert!(anonymous.starts_with("SAPISIDHASH 1_") && !anonymous.ends_with("_u"));
        assert_eq!(
            sid_authorization_at("o", None, "1", [None, None, None]),
            None
        );
    }

    #[test]
    fn exact_hosts_and_supported_video_url_shapes() {
        for input in [
            "https://www.youtube.com/watch?v=BaW_jenozKc&t=3",
            "https://youtu.be/BaW_jenozKc",
            "https://m.youtube.com/shorts/BaW_jenozKc",
            "https://www.youtube-nocookie.com/embed/BaW_jenozKc",
            "https://music.youtube.com/watch?v=BaW_jenozKc",
            "https://youtube.com/live/BaW_jenozKc",
        ] {
            assert_eq!(
                video_id(&Url::parse(input).unwrap(), false)
                    .unwrap()
                    .as_deref(),
                Some("BaW_jenozKc")
            );
        }
        for input in [
            "https://youtube.com.evil.invalid/watch?v=BaW_jenozKc",
            "https://example.com/watch?v=BaW_jenozKc",
        ] {
            assert!(
                video_id(&Url::parse(input).unwrap(), false)
                    .unwrap()
                    .is_none()
            );
        }
        // Playlist URLs belong to `youtube:tab` (its site fixture).
        for input in [
            "https://youtu.be/BaW_jenozKc/extra",
            "https://youtube.com/watch?v=bad",
            "https://youtube.com/watch?v=BaW_jenozKc&v=BaW_jenozKc",
        ] {
            assert!(
                matches!(
                    crate::YoutubeDl::new().unwrap().extract_info(input),
                    Err(Error::UnsupportedYoutube(_))
                ),
                "{input}"
            );
        }
    }

    #[test]
    fn no_playlist_resolves_watch_urls_with_lists() {
        let url = Url::parse("https://www.youtube.com/watch?v=BaW_jenozKc&list=PLfoo").unwrap();
        assert_eq!(
            video_id(&url, true).unwrap().as_deref(),
            Some("BaW_jenozKc")
        );
        assert!(video_id(&url, false).is_err());
        for input in [
            "https://youtube.com/playlist?list=foo",
            "https://youtube.com/watch?list=foo",
            "https://youtube.com/watch?v=bad&list=foo",
        ] {
            assert!(
                video_id(&Url::parse(input).unwrap(), true).is_err(),
                "{input}"
            );
        }
    }
}
