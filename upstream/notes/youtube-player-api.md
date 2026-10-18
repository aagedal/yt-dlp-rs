# YouTube anonymous WEB player API brief

Source and exact target: official yt-dlp commit
`897341241f2bfb2439fe0367f4c7c9ccd29a3828`, the existing pinned baseline.
Ran `tools/upstream_report.py` against the official local checkout at this target:
no intervening commits, changed files, affected components, or unmapped files.
This is a new baseline slice, not an upstream update; the global baseline stays
unchanged. The report establishes source availability, not parity.

## Sources reviewed and applicability

- `yt_dlp/extractor/youtube/_base.py`: `INNERTUBE_CLIENTS['web']` client/header
  identity and PLAYER/GVS PO token policy; `_extract_context`, `_call_api`,
  `extract_ytcfg`, `generate_api_headers`, and `_extract_response` for literal page
  config, JSON POST construction, context and headers.
- `yt_dlp/extractor/youtube/_video.py`: `_generate_player_context`,
  `_extract_player_response`, and `_extract_player_responses` for video ID,
  HTML5 preference, initial response reuse and API acquisition. Format extraction
  explicitly exempts itag 18 from HTTPS GVS token requirements.
- `test/test_youtube_misc.py` supplies URL tests, but no dedicated API construction
  or token fixtures were found. New Rust localhost fixtures assert the request.

These modules also implement authenticated clients, tokens, JavaScript challenges,
subtitles, manifests, adaptive formats and many other behaviors. Their source
mapping is partial and does not imply those features are covered. There is no
source delta or unmapped delta to classify at this target.

## Coherent behavior implemented

When a watch page has no initial player response, parse literal `ytcfg.set`
objects for a WEB client/version and optional visitor data. Build a fresh
allowlisted anonymous context and POST once to `/youtubei/v1/player` on the
canonical page origin with `prettyPrint=false`, JSON video ID and HTML5 playback
preference, numeric client header `1`, matching version, and Origin/Referer.
The shared reqwest redirect policy still applies; a fixed initial endpoint is
not a restriction on redirect destinations. Page host overrides, API keys,
user/session context and arbitrary fields are not copied.

The API JSON reader enforces identity encoding, JSON MIME, a 2 MiB read limit,
and explicit HTTP/partial/malformed response errors. Existing player parsing
validates playability, matching ID, title, live/DRM rejection, and direct muxed
challenge-free media. Only token-exempt itag 18 is retained before media probes;
no token acquisition or missing-token override is available. A malformed, gated,
or unsupported initial response never triggers fallback. Shared opt-in extraction
retries restart the entire page/API/probe sequence on transient network failures.

## Deliberate differences and deferred work

This is one WEB fallback, not upstream's client selection/fallback machinery.
It requires page client configuration instead of using a hardcoded default when
config is absent. Fresh context has `hl=en`, `gl=US`; upstream also handles
language/timezone/UTC offset and additional context data. Native requests omit
upstream's `contentCheckOk`/`racyCheckOk` overrides, signature timestamp, session
credentials, params, API keys and player tokens. Access/age/consent handling is
unsupported. The existing initial-response format/probe policy remains unchanged;
its token-protected alternatives can still fail extraction.

Additional clients, PO token acquisition, signature/`n` solving, JS execution,
SABR, adaptive merging, manifests, live content, and playlists remain unsupported.
No live-site reliability is claimed or exercised by required tests.

## Regression evidence

`tests/fixtures/youtube/player-api.html` contains synthetic literal WEB config,
including ignored host/key/user fields. `player-api.json` contains direct muxed
18 and 22 candidates; `player-api-expected.json` retains only 18. Rust local HTTP
tests assert method, path, headers, exact allowlisted body, media probes, selection,
metadata and download bytes. Failure cases exercise config, malformed/gated
initial responses, invalid API JSON/identity/status/encoding/size, token-only
formats, and whole-extraction retry after status or interrupted-body failures.

`tools/compare_youtube.py` injects the saved API response into upstream's real
response parser with ordinary anonymous WEB token policy and compares the same
expected metadata. Only the exact fixture origin is normalized. Acquisition is
bypassed: this verifies parser/token-policy subset parity, not API request parity
or current endpoint usability. The original watch-page comparison still uses its
documented missing-token override. Python remains development-only.

The subsequent [runtime decision](youtube-challenge-runtime.md) selects an
opt-in external Deno process with pinned offline EJS assets and records the
implementation gates. Challenge solving remains unimplemented. No Python runtime
extraction/download fallback is allowed.
