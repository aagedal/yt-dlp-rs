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
reuse the existing native pipeline. Unsupported dynamic, multiperiod, DRM, SIDX,
dependent streams and advanced addressing fail explicitly. See
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
