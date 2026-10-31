# Static DASH source slice

Reference and exact target: official yt-dlp commit
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19), also the unchanged global
baseline. Ran `tools/upstream_report.py` against the official local checkout at
that target: zero intervening commits, changed files, affected components or
unmapped files. This is a new bounded source slice, not an upstream catch-up claim.

Inspected `yt_dlp/extractor/common.py` `_parse_mpd_periods` and
`_parse_mpd_formats_and_subtitles`, especially inherited multisegment information
around lines 2871–3225, and `test/test_InfoExtractor.py::test_parse_mpd_formats`
around lines 1084–1468. These mapped files also implement dynamic/multiperiod
delivery, subtitles, SegmentBase and many other behaviors outside this slice.
`yt_dlp/downloader/dash.py` and `fragment.py` inform the separation between parsed
plans and transport; existing native fragment transport handles download rather
than invoking Python or FFmpeg for extraction. No unmapped upstream change was
introduced by this baseline-to-itself report.

Native support is limited to static single-period MPDs, one BaseURL per level,
inherited SegmentTemplate/SegmentTimeline or SegmentList, explicit whole-resource
representations, MP4/WebM audio/video metadata and bounded ordered fragments.
Direct HTTP/HTML5 dispatch acquires a bounded MPD without probing fragments;
selection, byte ranges, retries, publication and explicit local FFmpeg merging
reuse the existing native pipeline. Unsupported dynamic, SIDX, dependent streams
and advanced addressing fail explicitly. See
[the compatibility contract](../../docs/COMPATIBILITY.md#static-dash-mpds).

`tools/compare_dash.py` invokes the actual pinned parser with saved XML and
network blocked, then compares native localhost JSON metadata and ordered URLs.
It documents declaration-order ID mapping, exact-origin normalization and inferred
audio bitrate. The four saved cases cover Number, Time, list and generated media
timeline plans. Pinned upstream ignores Initialization/SegmentURL byte ranges;
native range expectations are asserted independently. Upstream also does not
expand negative repeats or apply presentationTimeOffset. Those native behaviors
have parser tests and are excluded from reference parity claims. The bounded
simple-addressing offset policy uses the
[DASH-IF timing model](https://dashif.org/Guidelines-TimingModel/); timelines needing
pre-period clipping are rejected and media bytes are not trimmed.

Regression coverage includes inherited addresses, token escaping and invalid token
combinations, exact rational timing and overflow, finite/negative repeats, ranges,
XML/URL/resource limits, unsupported structures, redirects, HTML namespaces,
headers and simulation without segment requests. Original generated media covers
timeline, duration, list and range assembly; local FFmpeg/FFprobe 9.0.2 additionally
verified individual H.264/AAC tracks and merged MP4 for all four forms. The Linux
real-tool CI job includes these tests. Fixture provenance is in
[the fixture README](../../tests/fixtures/dash/README.md).

This adds no YouTube MPD discovery, general DASH compatibility, live-site guarantee
or global baseline advancement.

## Multi-period and ContentProtection (pinned 2026.10.26)

Against the pinned baseline `f264a6cbd1f630894da0f368df00f8aae50140e0`
(2026.10.26; no baseline change), ported from `yt_dlp/extractor/common.py`:
`_merge_mpd_periods` and the per-Period loop of `_parse_mpd_periods`
(`stream_numbers` shared across Periods) and `is_drm_protected`/`has_drm`.
Formats equal except for `format_id`, `fragments` and `manifest_stream_number`
merge by appending fragments (also within one Period); subtitles concatenate with
the once-per-extraction multiple-period warning. `has_drm` formats are then dropped
by the existing `process_video_result` port unless `--allow-unplayable-formats`
(`This video is DRM protected`). Deliberate difference: a Period without
`duration` spans to the next Period's start (or the presentation end), where
upstream reuses `mediaPresentationDuration`; this only changes duration-template
segment counts. RepresentationIndex stays rejected (upstream never reads it).

## Dynamic (live) MPDs (pinned 2026.10.26)

Against the same pinned baseline (no baseline change), ported from
`yt_dlp/extractor/common.py` `_parse_mpd_periods` (the `dynamic_mpd` check, the
`period_duration` fallback and the template/timeline/list loops as they behave
for live manifests), `yt_dlp/extractor/generic.py` (`live_status` of a direct
MPD), `yt_dlp/options.py` (`--allow-dynamic-mpd`/`--ignore-dynamic-mpd` and
aliases), `yt_dlp/downloader/__init__.py` `_get_suitable_downloader` (live
`http_dash_segments` → FFmpegFD unless `native`), `yt_dlp/downloader/external.py`
`FFmpegFD._call_downloader` (`-re` per live DASH input), `yt_dlp/downloader/dash.py`
(`Live DASH videos are not supported`) and the `YoutubeDL.process_info` fixup
conditions (no duplicate-MOOV/m4a fixup after FFmpegFD).

Upstream reads a dynamic MPD with its static code, so the port adds a
dynamic-only schedule (`dash::dynamic_times`) instead of relaxing the static
rules: Period duration from `duration` or `mediaPresentationDuration` only (no
Period start checks); `$Number$` duration templates counted from it, from an
earlier representation's summed fragment durations in the same Period (float, as
`try_get(..., float)`), or not at all; timelines without presentationTimeOffset,
`t="0"` continuing, any time order and `r="-1"` as one segment; SegmentList
timelines taking `r + 1` URLs and dropping extras. Live-only hints upstream never
reads (`Location`, `PatchLocation`, `availabilityTimeOffset`,
`availabilityTimeComplete`) are ignored for dynamic MPDs only. Static parsing and
output are unchanged.

Explicit errors instead of upstream's behavior: too few SegmentURLs (`IndexError`)
and templates without a timeline or `$Number$` (`KeyError`) fail like upstream but
with this port's unsupported-manifest error; timeline/list structures the static
parser already rejects (namespaced attributes, `r < -1`, ...) stay rejected;
`--downloader native` stops after DashSegmentsFD's error where upstream goes on
to download the snapshot's fragments. Storyboard (image) representations still
produce no formats, though their fragments count toward an inferred Period
duration as upstream's do.

Evidence: `tools/compare_dash.py` (now eight manifests) on
`tests/fixtures/dash/dynamic-{number,timeline,list}.mpd`; exact `-j`/`-J` lines
for `dynamic.mpd` and `dynamic-timeline.mpd` in `tests/fixtures/info_order/`
(the live title's local time normalized as `{DATE} {HH}:{MM}`); parser unit tests
whose manifests were run through the pinned parser; and `tests/dash_dynamic.rs`
comparing `--ignore-dynamic-mpd`/`--allow-dynamic-mpd`, `--ignore-no-formats-error`,
`--live-from-start` and download outputs with pinned upstream on the same
manifests. FFmpegFD arguments for merged and single-format live downloads equal
upstream's as recorded by the same fake FFmpeg, except this port's existing
non-direct `Referer` header. FFmpeg's DASH demuxer itself is not tested: the
default Homebrew FFmpeg 9.0.2 has none, and FFmpeg-full 9.0.2 loops re-reading a
fully available local dynamic manifest (and its first segment) rather than
ending, so a real-tool live test would be wall-clock dependent.

Evidence: exact `-j`/`-J` lines for merged, unmerged and direct multi-period and
partly protected manifests (`tests/fixtures/info_order/`, captured with
`tools/capture_info_order.py`), `tools/compare_dash.py` on
`media/periods.mpd`, byte-identical two-Period downloads, and the DRM error,
`--ignore-no-formats-error`, `--allow-unplayable-formats`, `-F` and warning
outputs compared with pinned upstream on the same local manifests.
