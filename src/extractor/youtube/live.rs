//! Upstream `_prepare_live_from_start_formats` and `_live_adaptive_fragments`
//! for post-live (`isPostLiveDvr`) videos: each live adaptive format becomes
//! an `http_dash_segments` format whose fragments (`&sq=0` up to the
//! `X-Head-Seqnum` header's value minus two) are generated lazily, like
//! upstream's `LazyList`, when JSON output or a download first reads them
//! ([`prepare_fragments`]).
use crate::{Fragment, Result, VideoFormat, VideoInfo};
use reqwest::blocking::Client;
use serde_json::{Value, json};

/// The private format key holding the pending fragment generation (with the
/// stream's `live_start_time`); `__` keys never reach JSON output.
pub(crate) const PENDING: &str = "__yt_dlp_rs_post_live";

/// `_live_adaptive_fragments`' `FETCH_SPAN` and `MAX_DURATION` (120 hours).
const FETCH_SPAN: i64 = 5;
const MAX_DURATION: f64 = 432_000.0;

/// `fmt['preference'] = (fmt.get('preference') or -1) + adjustment` with
/// the note suffix (`adjust_incomplete_format`).
pub(super) fn adjust_incomplete(format: &mut VideoFormat, note: &str, adjustment: i64) {
    let preference = format
        .extra
        .get("preference")
        .and_then(Value::as_i64)
        .filter(|preference| *preference != 0)
        .unwrap_or(-1);
    format
        .extra
        .insert("preference".into(), (preference + adjustment).into());
    format.format_note = Some(match format.format_note.take().filter(|n| !n.is_empty()) {
        Some(existing) => format!("{existing} {note}"),
        None => note.to_owned(),
    });
}

/// `_prepare_live_from_start_formats` for a post-live video: the
/// `is_from_start` formats get `is_live`, pending fragments and the
/// `http_dash_segments` protocol.
pub(super) fn prepare_post_live(formats: &mut [VideoFormat], live_start_time: Option<i64>) {
    for format in formats {
        if format.extra.get("is_from_start") != Some(&Value::Bool(true)) {
            continue;
        }
        format.extra.insert("is_live".into(), false.into());
        format.extra.shift_remove("is_from_start");
        format.protocol = Some("http_dash_segments".into());
        format
            .extra
            .insert(PENDING.into(), json!({"live_start_time": live_start_time}));
        crate::info_order::mark_format(format, "youtube-post-live");
    }
}

/// `_live_adaptive_fragments`' `last_seq_cache`: the latest `X-Head-Seqnum`
/// by `int(time.time() // FETCH_SPAN)`, shared by the formats.
#[derive(Default)]
struct SeqCache(Option<(i64, Option<i64>)>);

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0.0, |elapsed| elapsed.as_secs_f64())
}

/// One HEAD request's `X-Head-Seqnum` (`None` on failure, like upstream's
/// non-fatal `_request_webpage`).
fn head_seqnum(client: &Client, url: &str) -> Option<i64> {
    match client.head(url).send().and_then(|r| r.error_for_status()) {
        Ok(response) => response
            .headers()
            .get("X-Head-Seqnum")?
            .to_str()
            .ok()?
            .trim()
            .parse()
            .ok(),
        Err(error) => {
            crate::screen::debug(&format!(
                "[youtube] Fragment request failed: {}",
                crate::retry::urllib_error(&error)
            ));
            None
        }
    }
}

/// The post-live fragments of one format (`url_feed` is `None`).
fn post_live_fragments(
    client: &Client,
    video_id: &str,
    itag: &str,
    base_url: &str,
    live_start_time: Option<i64>,
    target_duration: Option<f64>,
    cache: &mut SeqCache,
) -> Vec<Fragment> {
    let start = now();
    let lack_early_segments =
        start - live_start_time.map_or(start, |time| time as f64) > MAX_DURATION;
    if lack_early_segments {
        crate::screen::warning_once(&format!(
            "[youtube] Starting download from the last 120 hours of the live stream since YouTube does not have data before that. If you think this is wrong, {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    crate::screen::debug(&format!(
        "[youtube] [{video_id}] Generating fragments for format {itag}"
    ));
    let key = (now() / FETCH_SPAN as f64).floor() as i64;
    let last_seq = match cache.0 {
        Some((cached, seq)) if cached == key => seq,
        _ => {
            let seq = head_seqnum(client, base_url);
            if seq.is_some() {
                cache.0 = Some((key, seq));
            }
            seq
        }
    };
    let Some(last_seq) = last_seq else {
        return Vec::new();
    };
    // `last_seq += 1`, then `-= 2` without a URL feed.
    let last_seq = last_seq - 1;
    let mut known = 0_i64;
    if lack_early_segments && let Some(duration) = target_duration.filter(|d| *d > 0.0) {
        known = known.max(last_seq - (MAX_DURATION / duration).floor() as i64);
    }
    let Ok(base) = url::Url::parse(base_url) else {
        return Vec::new();
    };
    (known..last_seq)
        .map(|index| Fragment {
            encryption: None,
            url: crate::extractor::update_url_query(&base, &[("sq", &index.to_string())]),
            byte_range: None,
            path: None,
            duration: None,
            fragment_count: u64::try_from(last_seq).ok(),
        })
        .collect()
}

/// Generate one format's pending fragments, if it has any.
fn fill(
    client: &Client,
    video_id: &str,
    extra: &mut serde_json::Map<String, Value>,
    url: &url::Url,
    fragments: &mut Vec<Fragment>,
    cache: &mut SeqCache,
) {
    let Some(pending) = extra.shift_remove(PENDING) else {
        return;
    };
    let itag = extra
        .get("_itag")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let target_duration = extra.get("target_duration").and_then(Value::as_f64);
    *fragments = post_live_fragments(
        client,
        video_id,
        &itag,
        url.as_str(),
        pending["live_start_time"].as_i64(),
        target_duration,
        cache,
    );
}

/// Evaluate the pending post-live fragment lists of a result's formats,
/// requested formats and selected format.
pub(crate) fn prepare_fragments(client: &Client, info: &mut VideoInfo) -> Result<()> {
    let mut cache = SeqCache::default();
    let id = info.id.clone();
    for format in info.formats.iter_mut().chain(&mut info.requested_formats) {
        fill(
            client,
            &id,
            &mut format.extra,
            &format.url,
            &mut format.fragments,
            &mut cache,
        );
    }
    let url = info.url.clone();
    fill(
        client,
        &id,
        &mut info.extra,
        &url,
        &mut info.fragments,
        &mut cache,
    );
    Ok(())
}
