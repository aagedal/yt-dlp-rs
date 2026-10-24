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

/// The private format key of a live format downloaded from the start: the
/// stream's `live_start_time` and the extraction time (`start_time`, when
/// `refetch_manifest` last ran).
pub(crate) const LIVE: &str = "__yt_dlp_rs_live_from_start";

/// `_prepare_live_from_start_formats` for a live video with
/// `--live-from-start`: the `is_from_start` formats get `is_live` and the
/// `http_dash_segments_generator` protocol; their fragments are generated
/// while downloading ([`LiveFragments`]).
pub(super) fn prepare_live(formats: &mut [VideoFormat], live_start_time: Option<i64>) {
    let start_time = now();
    for format in formats {
        if format.extra.get("is_from_start") != Some(&Value::Bool(true)) {
            continue;
        }
        format.extra.insert("is_live".into(), true.into());
        format.protocol = Some("http_dash_segments_generator".into());
        format.extra.insert(
            LIVE.into(),
            json!({"live_start_time": live_start_time, "start_time": start_time}),
        );
        crate::info_order::mark_format(format, "youtube-post-live");
    }
}

/// A refetched format downloaded from the start.
pub(crate) struct RefetchedFormat {
    pub itag: Option<String>,
    pub client: String,
    pub url: Option<String>,
}

/// `refetch_manifest`'s result.
pub(crate) struct Refetched {
    pub is_live: bool,
    pub formats: Vec<RefetchedFormat>,
}

/// `refetch_manifest`'s shared (locked) state.
struct FeedState {
    formats: Vec<RefetchedFormat>,
    start_time: f64,
    is_live: bool,
}

/// The state `_prepare_live_from_start_formats` shares between one
/// download's formats: the refetched formats behind `lock` and the
/// `adaptive_last_seq_cache`.
pub(crate) struct LiveFeed {
    client: Client,
    video_id: String,
    runtimes: Vec<super::JsRuntime>,
    live_start_time: Option<i64>,
    /// `extractor_retries` (upstream's default 3) for `url_feed`'s
    /// `RetryManager`.
    retries: u32,
    state: std::sync::Mutex<FeedState>,
    cache: std::sync::Mutex<Option<((bool, i64), i64)>>,
}

/// Whether a format (or selected result) is generated from the start of a
/// live stream.
pub(crate) fn is_live_from_start(extra: &serde_json::Map<String, Value>) -> bool {
    extra.contains_key(LIVE)
}

fn refetched_format(extra: &serde_json::Map<String, Value>, url: &url::Url) -> RefetchedFormat {
    RefetchedFormat {
        itag: extra
            .get("_itag")
            .and_then(Value::as_str)
            .map(str::to_owned),
        client: extra
            .get("_client")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        url: Some(url.to_string()),
    }
}

impl LiveFeed {
    /// The feed of a result's live-from-start formats (`None` without any).
    pub(crate) fn new(
        client: &Client,
        info: &VideoInfo,
        runtimes: &[super::JsRuntime],
    ) -> Option<std::sync::Arc<Self>> {
        let mut sources: Vec<(&serde_json::Map<String, Value>, &url::Url)> = info
            .formats
            .iter()
            .chain(&info.requested_formats)
            .filter(|format| is_live_from_start(&format.extra))
            .map(|format| (&format.extra, &format.url))
            .collect();
        if is_live_from_start(&info.extra) {
            sources.push((&info.extra, &info.url));
        }
        let marker = sources.first()?.0.get(LIVE)?.clone();
        let mut known: Vec<RefetchedFormat> = Vec::new();
        for (extra, url) in sources {
            let format = refetched_format(extra, url);
            if !known
                .iter()
                .any(|seen| seen.itag == format.itag && seen.client == format.client)
            {
                known.push(format);
            }
        }
        Some(std::sync::Arc::new(Self {
            client: client.clone(),
            video_id: info.id.clone(),
            runtimes: runtimes.to_vec(),
            live_start_time: marker["live_start_time"].as_i64(),
            retries: 3,
            state: std::sync::Mutex::new(FeedState {
                formats: known,
                start_time: marker["start_time"].as_f64().unwrap_or_else(now),
                is_live: true,
            }),
            cache: std::sync::Mutex::new(None),
        }))
    }

    /// `url_feed`: the format's current URL and whether the stream is still
    /// live, refetching the player responses once `delay` seconds passed
    /// since the last refetch.
    fn url_feed(
        &self,
        itag: Option<&str>,
        client: &str,
        delay: f64,
    ) -> Result<Option<(String, bool)>> {
        let mut attempt = 0;
        loop {
            attempt += 1;
            let error = {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                if now() > state.start_time + delay {
                    let refetched =
                        super::refetch_live_formats(&self.client, &self.video_id, &self.runtimes)?;
                    state.formats = refetched.formats;
                    state.is_live = refetched.is_live;
                    state.start_time = now();
                }
                match state
                    .formats
                    .iter()
                    .find(|format| format.itag.as_deref() == itag && format.client == client)
                {
                    Some(format) => {
                        return Ok(format.url.clone().map(|url| (url, state.is_live)));
                    }
                    None if !state.is_live => {
                        format!("{}: Video is no longer live", self.video_id)
                    }
                    None => format!(
                        "Cannot find refreshed url for format {}; {}",
                        itag.unwrap_or("None"),
                        crate::BUG_REPORTS_MESSAGE
                    ),
                }
            };
            // `RetryManager(fatal=False)`'s reports.
            if attempt > self.retries {
                crate::screen::warning(&if attempt > 1 {
                    format!("[youtube] {error}. Giving up after {} retries", attempt - 1)
                } else {
                    format!("[youtube] {error}")
                });
                return Ok(None);
            }
            crate::screen::warning(&format!(
                "[youtube] {error}. Retrying ({attempt}/{})...",
                self.retries
            ));
        }
    }

    /// The `X-Head-Seqnum` for `key`, from the cache or one HEAD request.
    fn last_seq(&self, key: (bool, i64), base_url: &str) -> Option<i64> {
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((cached, seq)) = *cache
            && cached == key
        {
            return Some(seq);
        }
        let seq = head_seqnum(&self.client, base_url)?;
        *cache = Some((key, seq));
        Some(seq)
    }
}

/// `_live_adaptive_fragments` with a URL feed: a lazy fragment list that
/// polls `X-Head-Seqnum` (at most every five seconds) until the stream ends
/// or no new fragment appears (`no_fragment_score` above 30).
pub(crate) struct LiveFragments {
    feed: std::sync::Arc<LiveFeed>,
    itag: Option<String>,
    client_name: String,
    target_duration: Option<f64>,
    base_url: Option<String>,
    known: i64,
    score: i64,
    should_iterate: bool,
    lack_early_segments: bool,
    pending: std::collections::VecDeque<Fragment>,
}

impl LiveFragments {
    /// The generator for one format (`extra`/`url` are the format's).
    pub(crate) fn new(
        feed: std::sync::Arc<LiveFeed>,
        extra: &serde_json::Map<String, Value>,
        url: &url::Url,
    ) -> Self {
        let start = now();
        let lack_early_segments =
            start - feed.live_start_time.map_or(start, |time| time as f64) > MAX_DURATION;
        if lack_early_segments {
            crate::screen::warning_once(&format!(
                "[youtube] Starting download from the last 120 hours of the live stream since YouTube does not have data before that. If you think this is wrong, {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        let source = refetched_format(extra, url);
        crate::screen::debug(&format!(
            "[youtube] [{}] Generating fragments for format {}",
            feed.video_id,
            source.itag.as_deref().unwrap_or("None")
        ));
        Self {
            itag: source.itag,
            client_name: source.client,
            target_duration: extra.get("target_duration").and_then(Value::as_f64),
            base_url: source.url,
            feed,
            known: 0,
            score: 0,
            should_iterate: true,
            lack_early_segments,
            pending: Default::default(),
        }
    }

    /// The next fragment, `None` once the generator returns. `stop` is
    /// polled while waiting (an interrupt ends the list).
    pub(crate) fn next(&mut self, stop: &dyn Fn() -> bool) -> Result<Option<Fragment>> {
        loop {
            if let Some(fragment) = self.pending.pop_front() {
                return Ok(Some(fragment));
            }
            if !self.should_iterate || stop() || self.score > 30 {
                return Ok(None);
            }
            let fetch_time = now();
            let delay = if self.score > 15 { 5.0 } else { 18_000.0 };
            match self
                .feed
                .url_feed(self.itag.as_deref(), &self.client_name, delay)?
            {
                Some((url, is_live)) => {
                    self.base_url = Some(url);
                    self.should_iterate = is_live;
                }
                None => self.should_iterate = false,
            }
            let Some(base_url) = self.base_url.clone() else {
                self.score += 2;
                continue;
            };
            let key = (
                self.should_iterate,
                (now() / FETCH_SPAN as f64).floor() as i64,
            );
            let Some(mut last_seq) = self.feed.last_seq(key, &base_url) else {
                self.score += 2;
                continue;
            };
            if self.known > last_seq {
                self.score += 5;
                if self.should_iterate {
                    let until = fetch_time + FETCH_SPAN as f64;
                    while now() < until && !stop() {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                }
                continue;
            }
            last_seq += 1;
            if self.lack_early_segments
                && let Some(duration) = self.target_duration.filter(|d| *d > 0.0)
            {
                self.known = self
                    .known
                    .max(last_seq - (MAX_DURATION / duration).floor() as i64);
            }
            let Ok(base) = url::Url::parse(&base_url) else {
                return Ok(None);
            };
            for index in self.known..last_seq {
                self.pending.push_back(Fragment {
                    encryption: None,
                    url: crate::extractor::update_url_query(&base, &[("sq", &index.to_string())]),
                    byte_range: None,
                    path: None,
                    duration: None,
                    fragment_count: u64::try_from(last_seq).ok(),
                });
            }
            if self.known == last_seq {
                self.score += 5;
            } else {
                self.score = 0;
            }
            self.known = last_seq;
        }
    }
}
