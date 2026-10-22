//! Narrow watch-page slice of YoutubeIE at the pinned upstream revision.
//! Bounded WEB API fallback, native HLS VOD and explicit offline JS runtime.
mod challenge;
mod chapters;
pub(crate) mod clip;
pub(crate) mod comments;
pub(crate) use chapters::from_description as chapters_from_description;
pub(crate) mod dict;
mod innertube;
mod metadata;
pub(crate) mod search;
mod streams;
mod subs;
pub(crate) mod tab;
mod watch;
mod watched;
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
    /// Upstream `wait_for_video`: wait for a video without formats.
    pub wait_for_video: Option<WaitForVideo>,
    /// Upstream `getcomments`: extract comments in [`crate::YoutubeDl::post_extract`].
    pub get_comments: bool,
    /// Upstream `mark_watched` (only when cookies were passed).
    pub mark_watched: bool,
}

/// `--wait-for-video MIN[-MAX]` seconds, and whether progress lines are shown.
#[derive(Debug, Clone, Copy, Default)]
pub struct WaitForVideo {
    pub min: f64,
    pub max: Option<f64>,
    pub progress: bool,
}

struct ConfiguredYoutube {
    include_translated_subtitles: bool,
    get_comments: bool,
    mark_watched: bool,
    no_playlist: bool,
    wait_for_video: Option<WaitForVideo>,
    runtime: Option<challenge::Runtime>,
}

impl Youtube {
    pub fn with_options(options: YoutubeOptions) -> impl Extractor {
        // The player JS and player API requests use the extraction client,
        // with its headers, cookies and proxy.
        ConfiguredYoutube {
            include_translated_subtitles: options.include_translated_subtitles,
            no_playlist: options.no_playlist,
            wait_for_video: options.wait_for_video,
            get_comments: options.get_comments,
            mark_watched: options.mark_watched,
            runtime: options.js_runtime.map(challenge::Runtime::new),
        }
    }
}

/// Upstream `YoutubeBaseInfoExtractor._real_initialize`: `_initialize_pref`
/// (English, UTC) and `_initialize_consent` (accept all; otherwise EU
/// requests are redirected to consent.youtube.com).
pub(crate) fn initialize() {
    crate::screen::reset_extractor_warnings();
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
/// `_wait_for_video`: sleep until the release time (within the requested
/// bounds), showing the remaining time; an interrupt ends it early. Returns
/// the `ReExtractInfo` message.
fn wait_for_video(wait: &WaitForVideo, release_timestamp: Option<i64>, upcoming: bool) -> String {
    let now = || {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0.0, |elapsed| elapsed.as_secs_f64())
    };
    // `'%02d:%02d:%02d' % timetuple_from_msec(dur * 1000)[:-1]`.
    let format_duration = |seconds: f64| {
        let seconds = seconds.max(0.0);
        let hours = (seconds / 3600.0).floor();
        let minutes = ((seconds - hours * 3600.0) / 60.0).floor();
        let rest = (seconds - hours * 3600.0 - minutes * 60.0).floor();
        format!(
            "{:02}:{:02}:{:02}",
            hours as i64, minutes as i64, rest as i64
        )
    };
    let (min, max) = (wait.min, wait.max.filter(|max| *max != 0.0));
    let mut diff = release_timestamp.map(|release| release as f64 - now());
    if diff.is_none() && upcoming {
        diff = Some(match max {
            Some(max) if min != 0.0 => {
                let unit = f64::from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map_or(0, |elapsed| elapsed.subsec_nanos()),
                ) / 1e9;
                (min + (max - min) * unit).round()
            }
            Some(max) => max,
            None => min,
        });
        crate::screen::warning("Release time of video is not known");
    } else if diff.unwrap_or(0.0) <= 0.0 {
        crate::screen::warning("Video should already be available according to extracted info");
    }
    let diff = diff
        .unwrap_or(0.0)
        .max(min)
        .min(max.unwrap_or(f64::INFINITY));
    crate::screen::screen(&format!(
        "[wait] Waiting for {} - Press Ctrl+C to try now",
        format_duration(diff)
    ));
    let mut last = String::new();
    let mut progress = |message: &str| {
        let line = if wait.progress {
            let padding = " ".repeat(last.chars().count().saturating_sub(message.chars().count()));
            format!("{message}{padding}\r")
        } else if last.is_empty() {
            format!("{message}\n")
        } else {
            return;
        };
        crate::screen::screen_raw(&line);
        last = message.to_owned();
    };
    let interrupt = crate::interrupt::Guard::install();
    let until = now() + diff;
    loop {
        if interrupt.interrupted() {
            progress("");
            return "[wait] Interrupted by user".into();
        }
        let remaining = until - now();
        if remaining <= 0.0 {
            progress("");
            return "[wait] Wait period ended".into();
        }
        progress(&format!(
            "[wait] Remaining time until next attempt: {}",
            format_duration(remaining)
        ));
        // A second's sleep, polling the interrupt.
        for _ in 0..20 {
            if interrupt.interrupted() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

thread_local! {
    /// The URL a followed `url_result(ID)` names in `Extracting URL:` (its
    /// bare video ID).
    static EXTRACTING_AS: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// `url_result(VIDEO_ID, YoutubeIE)`: the downloader extracts the bare ID
/// (shown as such) through the watch URL.
fn follow_video(video_id: &str) -> Result<VideoInfo> {
    let target = Url::parse(&format!("https://www.youtube.com/watch?v={video_id}"))?;
    let mut info = direct::build_at(target.clone(), &target, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra
        .insert(crate::FOLLOW_URL.into(), target.as_str().into());
    EXTRACTING_AS.with(|shown| *shown.borrow_mut() = Some(video_id.to_owned()));
    Ok(info)
}

/// `datetime_from_str(upload_date) >= datetime_from_str('today-2days')`:
/// `today` is now rounded to the nearest UTC day.
fn recently_uploaded(upload_date: &str) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    recently_uploaded_at(upload_date, now)
}

fn recently_uploaded_at(upload_date: &str, now: i64) -> bool {
    let part = |range: std::ops::Range<usize>| upload_date.get(range)?.parse::<i64>().ok();
    let (Some(year), Some(month), Some(day)) = (part(0..4), part(4..6), part(6..8)) else {
        return false;
    };
    let today = (now + 43_200).div_euclid(86_400);
    metadata::days_from_civil(year, month, day) >= today - 2
}

/// `get_first(values, key)`: the first value that is not `None`.
fn get_first<'a>(values: &[&'a Value], key: &str) -> Option<&'a Value> {
    values
        .iter()
        .find_map(|value| value.get(key).filter(|value| !value.is_null()))
}

/// `initial_title`: the description header's title, else the player
/// overlay's.
fn initial_title(initial_data: Option<&Value>) -> Option<String> {
    let data = initial_data?;
    let header = data
        .get("engagementPanels")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|panel| {
            panel
                .pointer(
                    "/engagementPanelSectionListRenderer/content/structuredDescriptionContentRenderer",
                )
                .filter(|value| value.is_object())
        })
        .and_then(|sdcr| sdcr.get("items"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|item| {
            item.get("videoDescriptionHeaderRenderer")
                .filter(|value| value.is_object())
        });
    header
        .and_then(|header| header.get("title"))
        .and_then(metadata::text)
        .or_else(|| {
            data.pointer(
                "/playerOverlays/playerOverlayRenderer/videoDetails/playerOverlayVideoDetailsRenderer/title",
            )
            .and_then(metadata::text)
        })
}

/// `ISO3166Utils.short2full`.
fn country_name(code: &str) -> Option<&'static str> {
    let code = code.to_uppercase();
    include_str!("../data/iso3166.txt")
        .lines()
        .find_map(|line| line.split_once('\t').filter(|(short, _)| *short == code))
        .map(|(_, name)| name)
}

/// `raise_geo_restricted(subreason, countries)` as `YoutubeDL` reports its
/// `GeoRestrictedError`: the microformat's `availableCountries`, else the
/// page's `regionsAllowed` meta, as full names.
fn geo_restricted(id: &str, message: &str, player_responses: &[Value], webpage: &str) -> Error {
    let mut countries: Vec<String> = player_responses
        .iter()
        .find_map(|response| {
            response
                .pointer("/microformat/playerMicroformatRenderer/availableCountries")
                .filter(|countries| !countries.is_null())
        })
        .and_then(Value::as_array)
        .map(|countries| {
            countries
                .iter()
                .filter_map(|code| code.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    if countries.is_empty()
        && let Some(regions) =
            crate::html::html_search_meta(webpage, "regionsAllowed").filter(|r| !r.is_empty())
    {
        countries = regions.split(',').map(str::to_owned).collect();
    }
    let mut text = format!("[youtube] {id}: {message}");
    if !countries.is_empty() {
        // Unknown codes (a `TypeError` upstream) are shown as given.
        let names: Vec<&str> = countries
            .iter()
            .map(|code| country_name(code).unwrap_or(code))
            .collect();
        text.push_str(&format!(
            "\nThis video is available in {}.",
            names.join(", ")
        ));
    }
    text.push_str("\nYou might want to use a VPN or a proxy server (with --proxy) to workaround.");
    Error::Reported(text)
}

/// `_real_extract`'s reasons for a video without formats: DRM, then the
/// playability status's error screen (`raise_no_formats(reason)`).
fn unplayable_reason(
    id: &str,
    player_responses: &[Value],
    webpage: &str,
    wait: bool,
) -> Option<Error> {
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
    let reason = pemr.get("reason").and_then(metadata::text).or_else(|| {
        statuses
            .iter()
            .find_map(|status| status.get("reason").and_then(Value::as_str))
            .map(str::to_owned)
    });
    let subreason = pemr
        .get("subreason")
        .and_then(metadata::text)
        .map(|text| crate::html::clean_html(&text))
        .unwrap_or_default();
    if subreason.starts_with("The uploader has not made this video available in your country") {
        // `raise_geo_restricted(subreason, countries, metadata_available=True)`:
        // a warning while waiting for videos, else `GeoRestrictedError`.
        if wait {
            crate::screen::warning(&format!("[youtube] {subreason}"));
        } else {
            return Some(geo_restricted(id, &subreason, player_responses, webpage));
        }
    }
    let mut reason = reason?;
    if !subreason.is_empty() {
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
/// An extraction's outcome: the video, or (with `--wait-for-video`) a video
/// without formats to wait for.
enum Extraction {
    Video(Box<VideoInfo>),
    /// A paid video's trailer (`url_result(TRAILER_ID, 'Youtube')`).
    Trailer(String),
    Wait {
        release_timestamp: Option<i64>,
        upcoming: bool,
    },
}

#[allow(clippy::too_many_arguments)]
fn extract_clients(
    client: &Client,
    url: &Url,
    id: &str,
    runtime: Option<&challenge::Runtime>,
    translated_manual: bool,
    wait: bool,
    get_comments: bool,
    mark_watched: bool,
) -> Result<Extraction> {
    let shown = EXTRACTING_AS
        .with(|shown| shown.borrow_mut().take())
        .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={id}"));
    crate::screen::screen(&format!(
        "[youtube] Extracting URL: {}",
        crate::screen::extracting_url(&shown)
    ));
    // `is_music_url`.
    let music_url = url.host_str() == Some("music.youtube.com");
    let acquired = watch::acquire(client, id, runtime, music_url)?;
    // `info['__post_extractor'] = self.extract_comments(...)`.
    if get_comments {
        let contents = acquired
            .initial_data
            .as_ref()
            .and_then(|data| {
                data.pointer("/contents/twoColumnWatchNextResults/results/results/contents")
            })
            .filter(|contents| contents.is_array())
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new()));
        comments::defer(id, acquired.webpage_ytcfg.clone(), contents);
    }
    let first = |key: &str| {
        acquired
            .player_responses
            .iter()
            .find_map(|response| response.get(key).filter(|value| value.is_object()))
    };
    // `playabilityStatus.errorScreen...trailerVideoId`: `url_result(ID)`.
    if let Some(trailer) = acquired.player_responses.iter().find_map(|response| {
        response
            .pointer("/playabilityStatus/errorScreen/playerLegacyDesktopYpcTrailerRenderer/trailerVideoId")
            .and_then(Value::as_str)
            .filter(|trailer| !trailer.is_empty())
    }) {
        return Ok(Extraction::Trailer(trailer.to_owned()));
    }
    let details = first("videoDetails").cloned().unwrap_or(Value::Null);
    let all_details: Vec<&Value> = acquired
        .player_responses
        .iter()
        .filter_map(|response| {
            response
                .get("videoDetails")
                .filter(|value| value.is_object())
        })
        .collect();
    let all_microformats: Vec<&Value> = acquired
        .player_responses
        .iter()
        .filter_map(|response| {
            response
                .pointer("/microformat/playerMicroformatRenderer")
                .filter(|value| value.is_object())
        })
        .collect();
    let search_meta = |name: &str| crate::html::html_search_meta(&acquired.webpage, name);
    // `get_first(video_details, 'title') or translated_title or
    // search_meta([...])`, else the initial data's title (with a warning).
    let mut title = get_first(&all_details, "title")
        .and_then(Value::as_str)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            all_microformats
                .iter()
                .find_map(|micro| micro.get("title").and_then(metadata::text))
        })
        .or_else(|| {
            ["og:title", "twitter:title", "title"]
                .into_iter()
                .find_map(search_meta)
                .filter(|title| !title.is_empty())
        });
    if title.is_none()
        && let Some(initial) = initial_title(acquired.initial_data.as_ref())
    {
        crate::screen::warning(
            "[youtube] No title found in player responses; falling back to title from initial data. Other metadata may also be missing",
        );
        title = Some(initial);
    }
    let title = title.unwrap_or_default();
    // `int_or_none(lengthSeconds)` of the details, then the microformats,
    // then `parse_duration(search_meta('duration'))`.
    let length = |values: &[&Value]| {
        get_first(values, "lengthSeconds")
            .and_then(|value| match value {
                Value::String(text) => text.trim().parse::<i64>().ok(),
                other => other.as_i64(),
            })
            .filter(|duration| *duration != 0)
    };
    let duration = length(&all_details)
        .or_else(|| length(&all_microformats))
        .or_else(|| {
            match search_meta("duration").and_then(|meta| crate::parse_duration(&meta))? {
                crate::info::PyNumber::Int(value) => Some(value),
                // A fractional duration is not represented.
                crate::info::PyNumber::Float(value) => {
                    (value.fract() == 0.0).then_some(value as i64)
                }
            }
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
    // `if not duration and live_end_time and live_start_time`.
    let broadcast = |key: &str| {
        all_microformats.iter().find_map(|micro| {
            micro
                .get("liveBroadcastDetails")
                .and_then(|details| details.get(key))
                .filter(|value| !value.is_null())
        })
    };
    let duration = duration.or_else(|| {
        let time = |key: &str| {
            broadcast(key)
                .and_then(Value::as_str)
                .and_then(crate::dates::parse_iso8601)
                .filter(|time| *time != 0)
        };
        Some(time("endTimestamp")? - time("startTimestamp")?)
    });
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
    if formats.is_empty() {
        let reason = unplayable_reason(id, &acquired.player_responses, &acquired.webpage, wait);
        if wait {
            // `raise_no_formats(expected=True)` warns while waiting for videos
            // (only `report_drm` passes the video ID).
            if let Some(reason) = &reason {
                let text = reason.to_string();
                let prefix = format!("[youtube] {id}: ");
                match text.strip_prefix(&prefix) {
                    Some(message) if message != "This video is DRM protected" => {
                        crate::screen::warning(&format!("[youtube] {message}"));
                    }
                    _ => crate::screen::warning(&text),
                }
            }
            let release_timestamp = acquired.player_responses.iter().find_map(|response| {
                response
                    .pointer("/microformat/playerMicroformatRenderer/liveBroadcastDetails/startTimestamp")
                    .and_then(Value::as_str)
                    .and_then(crate::dates::parse_iso8601)
            });
            return Ok(Extraction::Wait {
                release_timestamp,
                upcoming: live_status == Some("is_upcoming"),
            });
        }
        return Err(reason.unwrap_or_else(|| no_formats(id)));
    }
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
    let mut keywords: Vec<String> = all_details
        .iter()
        .find_map(|details| details.get("keywords").and_then(Value::as_array))
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
    // `yt:stretch=W:H`: the first valid ratio, on every video format.
    static STRETCH: std::sync::LazyLock<regex::Regex> =
        std::sync::LazyLock::new(|| regex::Regex::new(r"(\d+)\s*:\s*(\d+)").unwrap());
    if let Some(ratio) = keywords
        .iter()
        .filter(|keyword| keyword.starts_with("yt:stretch="))
        .filter_map(|keyword| STRETCH.captures(keyword))
        .find_map(|found| {
            let (width, height): (f64, f64) = (found[1].parse().ok()?, found[2].parse().ok()?);
            (width > 0.0 && height > 0.0).then(|| width / height)
        })
    {
        for format in formats
            .iter_mut()
            .filter(|format| format.vcodec.as_deref() != Some("none"))
        {
            format.extra.insert("stretched_ratio".into(), ratio.into());
            crate::info_order::mark_format_update(format, "stretched_ratio");
        }
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
    // Newly uploaded (within two days) videos' HLS formats need testing.
    if !matches!(live_status, Some("is_live" | "post_live" | "is_upcoming"))
        && let Some(upload_date) = dict.get("upload_date").and_then(Value::as_str)
        && recently_uploaded(upload_date)
    {
        for format in &mut formats {
            if format.protocol.as_deref() == Some("m3u8_native") {
                format.extra.insert("__needs_testing".into(), true.into());
            }
        }
    }
    let page = Url::parse(&format!("https://www.youtube.com/watch?v={id}"))?;
    let first_format = &formats[0];
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
    // `self.mark_watched(video_id, player_responses)`.
    if mark_watched {
        watched::mark(client, id, &acquired.player_responses);
    }
    Ok(Extraction::Video(Box::new(crate::info_order::mark_info(
        info,
        &format!("youtube;{}", keys.join(",")),
    ))))
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
        match extract_clients(client, url, &id, None, false, false, false, false)? {
            Extraction::Video(info) => Ok(Some(*info)),
            Extraction::Trailer(trailer) => Ok(Some(follow_video(&trailer)?)),
            Extraction::Wait { .. } => unreachable!("waiting is not configured"),
        }
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
        loop {
            match extract_clients(
                client,
                url,
                &id,
                self.runtime.as_ref(),
                self.include_translated_subtitles,
                self.wait_for_video.is_some(),
                self.get_comments,
                self.mark_watched,
            )? {
                Extraction::Video(info) => return Ok(Some(*info)),
                Extraction::Trailer(trailer) => return Ok(Some(follow_video(&trailer)?)),
                Extraction::Wait {
                    release_timestamp,
                    upcoming,
                } => {
                    let wait = self.wait_for_video.unwrap_or_default();
                    let message = wait_for_video(&wait, release_timestamp, upcoming);
                    // `ReExtractInfo(expected=True)`.
                    crate::screen::screen(&format!("{message}; Re-extracting data"));
                }
            }
        }
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
    fn recent_uploads_follow_datetime_from_str_today_minus_two_days() {
        // 2026-10-22 11:00 UTC rounds to the 22nd; 13:00 UTC to the 23rd.
        let morning = 1_792_666_800;
        assert!(recently_uploaded_at("20261020", morning));
        assert!(!recently_uploaded_at("20261019", morning));
        let afternoon = morning + 2 * 3600;
        assert!(!recently_uploaded_at("20261020", afternoon));
        assert!(recently_uploaded_at("20261021", afternoon));
        assert!(!recently_uploaded_at("bad", morning));
    }

    #[test]
    fn country_names_follow_iso3166_utils() {
        assert_eq!(country_name("no"), Some("Norway"));
        assert_eq!(country_name("AX"), Some("Åland Islands"));
        assert_eq!(country_name("XX"), None);
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
