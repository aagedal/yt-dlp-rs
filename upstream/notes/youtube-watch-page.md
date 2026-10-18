# YouTube watch-page implementation brief

Historical initial-slice brief. The subsequent [WEB API slice](youtube-player-api.md)
implements the missing-response fallback and API token policy described as next
steps below; the original initial-response path retains this brief's scope.

Source and target: official yt-dlp commit
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (the existing pinned baseline).
This introduces a scoped extractor at the baseline; it is not an upstream update.
The global baseline remains unchanged.

Ran `tools/upstream_report.py` against the official checkout with that exact
`--target`: no intervening commits, changed files, affected components, or unmapped
files. The empty report establishes reference availability only, not parity.

Read `yt_dlp/extractor/youtube/_video.py` (`_VALID_URL`, `_extract_player_responses`,
`_invalid_player_response`, `_extract_formats_and_subtitles`, `_list_formats`,
`_real_extract`), `_base.py`'s initial response pattern/client configuration, and
`test/test_youtube_misc.py` URL-ID tests. The source module also contains API
clients, token policies, JavaScript challenge solving, subtitles, manifests,
live processing, and many metadata fields outside this slice. These are deferred,
not covered by a partial source mapping. There is no source delta to classify.

## Coherent behavior ported

Single-video URL recognition, watch-page player JSON, matching identity,
playability checks, title, muxed progressive formats, numeric itags and declared
dimensions. Exclude signature/n challenges, DRM, OTF, and live fragments before
probing candidate media. Register before generic HTTP so unsupported YouTube
URLs cannot silently succeed through the generic HTML5 extractor. Shared
selection, output, retries, and download transport stay outside the site parser.

Regression fixtures: `tests/fixtures/youtube/progressive.html` and `expected.json`.
Rust tests in `src/extractor/youtube.rs` exercise local watch-page requests,
media probes, selection and bytes downloaded, plus malformed/unsupported cases.
`tools/compare_youtube.py` runs upstream's real response parser with saved player
responses instead of API acquisition and compares the explicit expected subset.
Rust independently verifies the same expected subset with local probe responses.
Only fixture origins are normalized. The synthetic fixture is not a captured
live-site response and does not demonstrate current endpoint usability.

## Deferred prerequisites and differences

- Canonical watch-page acquisition only. No Innertube API/client fallback, consent,
  authentication, cookies, age handling, or PO token acquisition. Upstream exempts
  itag 18 from its HTTPS GVS PO token requirement; others can still require tokens.
  All eligible probes must succeed in this Rust slice, so a protected alternative
  may block an otherwise usable format. No best-effort silent probe failures.
- Upstream's JavaScript challenge providers use EJS and supported JS runtimes.
  No runtime boundary has been chosen for Rust. Python is never a runtime fallback.
- Adaptive audio/video needs explicit stream types and muxing (e.g. ffmpeg).
  HLS/DASH, SABR, live, and playlists require further coherent implementations.
- Only recognized muxed MP4/WebM codecs are accepted. Full metadata, subtitles,
  upstream sorting, token policy and full URL matching are not ported.
- Media types/filesizes are validated by Rust HTTP probes, whereas the comparison
  reads declared fixture contentLength through upstream. The fixture deliberately
  makes them agree. This is parser parity for this subset, not transport parity.

Next bounded step: deterministic player API/client request fixtures and explicit
client/token policy, followed by a reviewed JavaScript challenge strategy. This
slice adds no live-site requests to CI and makes no broad YouTube support claim.
