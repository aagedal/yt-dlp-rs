//! Port extension: native live HLS. Upstream always records live HLS with
//! FFmpegFD (`_get_suitable_downloader`) and fails without ffmpeg; this port
//! then polls the media playlist instead, appending each new segment like
//! DashSegmentsFD's live generator (`live_dash`), until `#EXT-X-ENDLIST`,
//! an interrupt, or a playlist that stops advancing.
use crate::{Error, Fragment, Result, hls::Native, live_dash::LiveSource};
use reqwest::blocking::Client;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};
use url::Url;

/// Segments FFmpeg's HLS demuxer starts before the live edge
/// (`live_start_index` -3).
const LIVE_EDGE_SEGMENTS: usize = 3;
/// Consecutive reloads without a new segment before the stream is
/// considered ended.
const MAX_STALE_RELOADS: u32 = 10;

pub(crate) struct HlsLive {
    client: Client,
    url: Url,
    referer: Option<Url>,
    /// `--live-from-start`: every listed segment, not only the live edge.
    from_start: bool,
    /// The media sequence number of the next segment to emit.
    next_sequence: Option<u64>,
    map_sent: bool,
    pending: VecDeque<Fragment>,
    ended: bool,
    stale: u32,
    next_poll: Instant,
}

impl HlsLive {
    pub(crate) fn new(client: Client, url: Url, referer: Option<Url>, from_start: bool) -> Self {
        Self {
            client,
            url,
            referer,
            from_start,
            next_sequence: None,
            map_sent: false,
            pending: VecDeque::new(),
            ended: false,
            stale: 0,
            next_poll: Instant::now(),
        }
    }

    /// Reload the playlist and queue its new segments; the delay before the
    /// next reload.
    fn reload(&mut self) -> Result<Duration> {
        let response =
            crate::extractor::direct::probe(&self.client, &self.url, self.referer.as_ref())?;
        let (_, window) = crate::extractor::hls::read_window(response)?;
        match window.native {
            Native::Supported => {}
            Native::Drm => {
                return Err(Error::Reported(
                    "This format is DRM protected; Try selecting another format with --format or add --check-formats to automatically fallback to the next best format"
                        .into(),
                ));
            }
            Native::UnsupportedFeatures => {
                return Err(Error::Reported(
                    "m3u8 download detected but ffmpeg could not be found. Please install".into(),
                ));
            }
        }
        let mut fragments = window.fragments.into_iter();
        let map = if window.has_map {
            fragments.next()
        } else {
            None
        };
        if !self.map_sent {
            self.pending.extend(map);
            self.map_sent = true;
        }
        let segments: Vec<Fragment> = fragments.collect();
        let first = window.first_sequence;
        let end = first + segments.len() as u64;
        let start = match self.next_sequence {
            Some(next) => {
                if next < first {
                    // Segments left the playlist before they were reloaded.
                    crate::screen::warning(&format!(
                        "[hlsnative] Skipped {} live segment(s) that left the playlist",
                        first - next
                    ));
                }
                // A media sequence that restarted (a new stream) resumes
                // from its start.
                if next > end && end > 0 {
                    first
                } else {
                    next.max(first)
                }
            }
            None if self.from_start => first,
            None => end.saturating_sub(LIVE_EDGE_SEGMENTS as u64).max(first),
        };
        let fresh: Vec<Fragment> = segments
            .into_iter()
            .enumerate()
            .filter(|(index, _)| first + *index as u64 >= start)
            .map(|(_, fragment)| fragment)
            .collect();
        let target = Duration::from_secs(window.target_duration.max(1));
        let delay = if fresh.is_empty() {
            self.stale += 1;
            target / 2
        } else {
            self.stale = 0;
            target
        };
        self.next_sequence = Some(if fresh.is_empty() { start } else { end });
        self.pending.extend(fresh);
        if window.endlist {
            self.ended = true;
        } else if self.stale >= MAX_STALE_RELOADS {
            crate::screen::warning(
                "[hlsnative] The live playlist stopped advancing; ending the recording",
            );
            self.ended = true;
        }
        Ok(delay)
    }
}

impl LiveSource for HlsLive {
    fn next(&mut self, stop: &dyn Fn() -> bool) -> Result<Option<Fragment>> {
        loop {
            if let Some(fragment) = self.pending.pop_front() {
                return Ok(Some(fragment));
            }
            if self.ended || stop() {
                return Ok(None);
            }
            while let Some(remaining) = self.next_poll.checked_duration_since(Instant::now()) {
                if stop() {
                    return Ok(None);
                }
                std::thread::sleep(remaining.min(Duration::from_millis(100)));
            }
            let delay = self.reload()?;
            self.next_poll = Instant::now() + delay;
        }
    }
}
