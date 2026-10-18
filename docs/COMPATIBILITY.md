# Supported compatibility subset

This inventory describes tested behavior, not general yt-dlp compatibility.
All extraction and downloads run in Rust. Python is development tooling only.

## CLI

| Option | Current behavior |
| --- | --- |
| Positional URLs | One or more HTTP(S) URLs, processed sequentially; continue after errors, return failure if any URL fails |
| `-j`, `--dump-json` | One selected `VideoInfo` JSON object per video, including each HTML5 playlist entry; no downloads; with `-F`, a table precedes JSON |
| `-F`, `--list-formats` | List eligible alternatives without selection or downloads; combined `-j`/`-s` retain normal selection; native table styling |
| `-s`, `--simulate` | Extract metadata, write diagnostics to stderr, create no media files |
| `-f`, `--format` | Bounded single-source ranking/IDs, audio/video selectors, two-stream `+` pairs, grouped `/` fallbacks, aliases and bare/chained numeric/text filters; see selection policy below; no direct-URL format IDs |
| `-o`, `--output` | `title`, `id`, `ext`, `extractor` string fields and literal `%%`; no overwrite |
| `--retries` | Additional download attempts, integer 0–255, default 3; independent of extraction retries; no `infinite` value |
| `--youtube-js-runtime PATH` | Opt into YouTube signature/query-n solving with an explicit Deno 2.9.7 executable; lazy validation, no discovery/installation; not general upstream runtime option parity |
| `--ffmpeg-location PATH` | Explicit executable file for two-stream copy merging; no PATH discovery, directory lookup, installation, transcoding or other postprocessors |
| `--extractor-retries` | Additional whole-extraction attempts, integer 0–255, default 0; no `infinite` value |
| `-h`, `--help`, `-V`, `--version` | Clap help and crate version |
| All other flags | Rejected; aliases such as `--skip-download` and `--print` are not implemented |

Simulation and JSON still make network requests. This inventory does not claim
upstream equivalence for every option interaction or filename rule.

## YouTube watch-page subset

The built-in `Youtube` extractor runs before the generic HTTP fallback. Exact
`youtube.com`, `www.youtube.com`, `m.youtube.com`, `music.youtube.com`, `youtu.be`,
`www.youtu.be`, `youtube-nocookie.com`, and `www.youtube-nocookie.com` hosts are
claimed. Accept watch URLs with one `v` parameter, short links, `/shorts/`,
`/embed/`, and `/live/` paths with exactly an 11-character video ID. Watch-page requests
use the canonical HTTPS watch URL. Reject channels, playlists, malformed IDs,
and URLs containing `list=` before making a request. `/live/` is a URL shape;
live, upcoming, and post-live player responses remain unsupported.

Watch pages must be uncompressed UTF-8/ASCII `text/html`, at most 2 MiB. Parse
JSON assigned to `ytInitialPlayerResponse` in a script, including quoted window
property assignments. Require `playabilityStatus.status=OK`, the requested video
ID, and a nonempty title. JavaScript runs only for eligible signature/query-n formats when an explicit
runtime is configured. Authentication, age/consent gates,
additional API clients, cookies, and token acquisition are unsupported.

Only when the initial response is absent, make one WEB player API POST to the
canonical page origin at `/youtubei/v1/player?prettyPrint=false`; the shared
HTTP redirect policy still applies. Require literal
`ytcfg.set({...})` script configuration with `INNERTUBE_CONTEXT.client` naming
`WEB` and a nonempty digits/dots client version (at most 128 bytes). Multiple
literal objects merge in declaration order. Construct a fresh context with that
name/version, `hl=en`, `gl=US`, and optional visitor data (at most 4096 bytes, no
control characters); send matching client/visitor headers and page Origin/Referer.
Page host overrides, API keys, user/session context, and credentials are ignored.
The body contains video ID, this client context, and HTML5 playback preference.
No signature timestamp, content/racy check overrides, player token, or auth is
sent. JSON responses must be uncompressed, at most 2 MiB, and pass the same
identity/playability/format validation. Malformed or unsupported initial
responses never invoke fallback. API errors fail explicitly; opt-in extraction
retries restart the whole page/API sequence after transient status/read failures.

WEB player requests need no PLAYER PO token at the pinned baseline. Its direct
HTTPS and DASH media require a GVS PO token except direct itag 18; HLS tokens are
optional. API-derived direct formats therefore retain only itag 18 before any
media probe; eligible HLS is considered independently. No surviving format is an
unsupported error.
This policy does not acquire tokens or assert that exempt URLs always work.
The original initial-response path retains its prior token/probe behavior.

Expose `streamingData.formats` followed by `adaptiveFormats` entries resolving
to direct HTTP(S) media, with MP4/M4A/WebM MIME types and recognized audio
and/or video codecs. Classify sources as standalone, video-only or audio-only. Preserve numeric itags as
format IDs and positive declared dimensions. Skip DRM, OTF/fragments, unknown
source types and live fragments. Encrypted `signatureCipher`/legacy `cipher` and
query-n URLs require explicit `--youtube-js-runtime PATH` or library configuration;
reject player responses with DRM license information. Eligible HLS manifests
are expanded through the native VOD pipeline described below. DASH discovery
remains disabled because the WEB GVS token is unavailable.
If no supported direct or HLS format remains, return an explicit unsupported error.
Supported alternatives retain declaration order and use the existing resolution
ranking or literal itag selection, not upstream's full sorter. Every eligible direct
URL must pass a direct media probe; a failing probe fails the entire extraction,
including when a different format was requested. Filesizes and final URLs come
from those probes. Token-protected formats can therefore prevent extraction even
if another format might work. Metadata uses `youtube` / `Youtube`, `direct=false`,
and the canonical watch page. The shared downloader sends that page as referrer.

This is fixture-tested groundwork, not general live-site YouTube support. Modern
YouTube may require other API clients, PO tokens, SABR, DASH discovery, or fragmented direct adaptive delivery; those remain
unimplemented.

An optional offline EJS/Deno pipeline solves encrypted signatures and query-n challenges during public
extraction with `--youtube-js-runtime PATH` or
`YoutubeDl::with_youtube_options(YoutubeOptions { js_runtime: Some(path) })`.
The explicit executable must be Deno 2.9.7. Configuration is inert until an
eligible challenge needs solving; canonical path/version validation is cached
once per configured extractor. Token/live/DRM/format filtering precedes solving,
and complete-batch validation precedes every media probe. Player network failures
retain whole-extraction retry policy; runtime/protocol failures are permanent.
The [n extraction slice](../upstream/notes/youtube-n-extraction.md) records exact
behavior, cross-platform runtime evidence and deterministic extraction tests.
The [signature slice](../upstream/notes/youtube-signature-extraction.md) adds
exact cipher/default/custom-sp/combined-n extraction and download tests. The
optional comparison also accepts `--deno PATH` to run upstream's actual offline
EJS signature-permutation path against the same saved fixture. Without that flag,
the signature case is explicitly skipped.

An unsupported page cannot fall through to generic HTML5 and appear successful.
The deterministic Rust tests cover URL dispatch, player response validation,
format rejection, source probes, selection, and a local download. The optional
upstream parser comparison uses the same saved response and expected metadata:

```sh
python3 tools/compare_youtube.py --upstream /path/to/yt-dlp
```

Only ID, title, extractor identity, canonical webpage URL, and ordered formats'
itag, extension, dimensions, URL, and filesize are compared. Normalize only the
exact fixture origin to `{{ORIGIN}}`; all other compared values remain exact.
The harness bypasses upstream network/client acquisition and allows missing PO
tokens for the original watch-page fixture only. The additional saved API
response uses ordinary upstream WEB token policy and compares the retained
itag 18 metadata. These comparisons verify parsing and that token-policy subset,
not API acquisition/request parity, full ranking, or live-site usability. Local
Rust HTTP tests separately assert API requests, rejected responses, fallback
boundaries, whole-extraction retries, and token filtering before probes. See
[the watch-page brief](../upstream/notes/youtube-watch-page.md) and
[the API brief](../upstream/notes/youtube-player-api.md).

## HTML5 extraction

The generic fallback dispatches a single response to DASH/HLS parsing,
direct-media inspection or HTML5 parsing. Site extractors registered through `Extractor` run first. The
standalone `DirectHttp` extractor remains available for direct-only callers.

For `text/html`, accept `video`/`audio` elements with nonempty
`src` and/or direct child `source[src]` declarations. Resolve URLs against the
final page URL and first `base[href]`; decode HTML entities using an HTML5 parser. Prefer Open Graph
title, then document title, then page filename stem; collapse title whitespace.
Skip elements without nonempty `src` or direct child `source[src]` declarations. One retained element is a
video; two or more form an HTML5 playlist in document order. Append `-1`, `-2`, …
to entry IDs and ` (1)`, ` (2)`, … to titles, as upstream HTML5MediaEmbedIE does.
Number retained entries contiguously, ignoring empty elements. All-empty pages
fail as unsupported. The playlist wrapper has the unsuffixed page ID/title,
`generic` / `Generic` extractor identity, and the final page `webpage_url`.
Without a page title the wrapper title is `video`, while entry titles use the
page filename stem. Subtitle-only elements remain outside the supported subset.
Metadata identifies `html5` / `HTML5MediaEmbed` with `direct: false` and the final
page URL as `webpage_url`. Every source is probed with the page referrer and must
serve supported direct HTTP media or a supported finite HLS/static DASH manifest. Redirected source URLs and response-derived
extensions/filesizes are recorded in typed `VideoFormat` entries. HTML5 JSON adds
`formats` and the selected `format_id`, including for a single source. IDs are
literal zero-based strings assigned to nonempty declarations: the media element's
`src` first, then direct child sources in document order. Duplicate URLs remain
separate declarations. Source IDs restart at `0` for each element. These IDs are
Rust-specific; upstream's label/title IDs,
and full upstream format sorting are not implemented.

Default selection and `-f best` choose the highest resolution source; `-f worst`
chooses the lowest. Child `<source>` attributes `width`, `height`, and fallback
`res` (for height) supply optional positive `u32` dimensions, with surrounding
whitespace allowed. Zero, signed, overflowing, unit-suffixed, and other malformed
values are ignored; an invalid height still allows `res` fallback. Media element
width/height describe the player and are not inherited, including by inline `src`.
Label/title resolution parsing is not implemented.

Resolution rank is the smaller known dimension, or the only known dimension;
unknown resolution ranks below known resolution. Portrait sources are therefore
ranked by their shorter side. On equal ranks best/default selects the last
declaration and worst the first. File size, extension, codecs, bitrate, frame
rate, and audio/video distinctions do not break ties. This is an explicit native
policy for the supported progressive subset, not full upstream best/worst parity
or a guarantee of perceptual quality. Format lists and source IDs retain declaration
order. Literal IDs override ranking; unavailable IDs return an error listing IDs.
Comma selections, more than two merged streams, and
upstream selector modifiers remain errors. See the bounded adaptive policy below.
Direct HTTP results accept default/`best`/`worst` as their sole representation,
without adding format IDs; explicit IDs fail.
The library's `extract_info_with_format` performs the same selection; standalone
extractor callers must call `VideoInfo::select_format` to resolve candidates before
download. Selection updates top-level `url`, `ext`, `filesize`, `width`, `height`,
and `format_id` together. Optional dimension fields deserialize as absent for older
metadata. Each playlist entry is ranked independently.
The selected source is downloaded with the page referrer. If the download
response changes to HTML or a manifest, fail without publishing it as media.

HTML bodies are limited to 2 MiB, including responses without Content-Length.
Only uncompressed UTF-8/ASCII HTML is supported. Other encodings are rejected
when declared in HTTP Content-Type; invalid UTF-8 is rejected. HTML meta charset
sniffing, compressed responses, XHTML, JavaScript, nonstandard lazy-loading
attributes, subtitles, posters, durations, codecs, inferred dimensions, authentication,
and general embedded-player extraction are not implemented. Unsupported HLS/DASH
features and non-HTTP sources fail explicitly, even
when another supported source was explicitly selected. Unselected source probe
failures also fail extraction rather than hiding unsupported alternatives.
Every entry and alternative is validated before a result is returned. A failed
probe or format selection fails the whole input before downloads or JSON output;
selection errors identify the entry. A failed transfer or output-path collision
sets a nonzero CLI exit status while later entries and input URLs continue.
Fixed output paths may therefore collide; the default template includes each
entry's distinct ID. Existing files are never overwritten.

The library exposes `ExtractResult::Video(VideoInfo)` and
`ExtractResult::Playlist(PlaylistInfo)` through `extract_result` and
`extract_result_with_format`. Serde encodes the result enum using `_type: video`
or `_type: playlist`; playlist entries are ordinary `VideoInfo` objects.
`extract_info` / `extract_info_with_format` still return single videos. They
reject playlists rather than silently choosing the first entry; extraction and
format-selection errors can precede that boundary error. Existing custom
`Extractor::extract` implementations use the default `extract_result` adapter;
playlist-capable extractors override it. Standalone extractor callers must resolve
format selection themselves. The CLI flattens results into per-video NDJSON,
simulation diagnostics, or sequential downloads; it exposes no playlist wrapper
JSON option, playlist selection, nested playlists, or unresolved URL results.

MIME checks identify supported responses; they do not verify media containers.

## Download retries

The default download policy permits three additional attempts after HTTP 408,
429, 500, 502, 503, or 504, connection/timeouts, response body read errors, or
Content-Length mismatches. `--retries 0` disables retries; values through 255
are accepted. The library exposes `DownloadOptions { retries }` through
`YoutubeDl::download_with_options`; `download` uses the default policy.
After the retry budget is exhausted, return the final attempt's error.

After a body interruption, a retry can reuse bytes already written in the same
private temporary file. This requires a known Content-Length and a valid strong
ETag from the **download** response, an identity-encoded body, and a nonzero
stored offset below the total. The retry sends `Range: bytes=N-` and `If-Range`
with that ETag. A 206 response is accepted only with the same final URL and ETag,
identity encoding, and a single valid Content-Range covering exactly `N` through
`total - 1` with the original total. Content-Length, when present, must match the
remaining length; the received body is also checked against that length. Further
interruptions advance the offset and consume the same retry budget. Transient
HTTP errors preserve the offset without appending their bodies.

A full response to a range request replaces the temporary contents from byte
zero, including when the representation changes. Missing/weak/malformed ETags,
unknown initial lengths, and encoded responses use whole-file retries instead.
Last-Modified alone is not used as a validator. Invalid/changed partial responses
and HTTP 416 fail explicitly, without publishing a file or guessing completion.
Temporary files are removed on final failure. Resume across invocations, saved
partial files, multipart ranges, and fixed-size range chunk downloading are not
implemented.
The offset/length checks also apply when a 206 body omits Content-Length.

Backoff is 100, 200, 400, 800, then 1000 milliseconds between attempts, capped
at one second thereafter. For the selected transient HTTP statuses, a single
`Retry-After` delay-seconds or preferred HTTP-date value extends this backoff,
capped at 60 seconds.
ASCII digits with optional surrounding spaces/tabs are accepted; oversized
integer values saturate to the cap. HTTP dates must use the preferred
IMF-fixdate form, such as `Wed, 21 Oct 2015 07:28:00 GMT`. The delay is measured
against the local wall clock when the response is checked, independently of
the server's `Date` header; past dates give a zero hint. Clock skew can therefore
affect the delay, which remains capped at 60 seconds. Missing, malformed,
duplicated, comma-list, and obsolete date values use ordinary backoff. A zero
hint cannot shorten backoff.
Headers on successful or permanent-error responses do not introduce delays;
exhausted/disabled retry budgets never sleep or gain extra attempts. The same
rules apply to HTML pages, source probes, and full/range download requests.
These are native subset policies, not upstream retry-default or HTTP-status parity.
Library failures with a supported hint use `Error::HttpRetryAfter { source,
retry_after }`, retaining the reqwest status error and capped duration; other
HTTP failures retain `Error::Http`. Neither error variant retries automatically
outside the orchestration APIs.

Every attempt repeats response validation and preserves the HTML5 page referrer.
Other HTTP statuses (including 401, 404, and 501), unsolicited 206 responses,
unsupported HTML5 media responses, output collisions, and filesystem failures
fail immediately. Network reads and filesystem writes are handled separately so
a local write failure cannot trigger another network transfer. The final file
is published only after the complete representation has been received and is
never overwritten.
Download retries do not affect extraction, page reads, or source probes.
The per-request timeouts still apply to each attempt; there is no overall
download deadline. Cooperative download cancellation is described below.

This slice is based on the same pinned revision
**897341241f2bfb2439fe0367f4c7c9ccd29a3828**, specifically
`yt_dlp/downloader/http.py`'s `RetryDownload`/`RetryManager` control flow,
transport-read failures, and Content-Length validation. Its broader 5xx retry
handling remains outside this slice.
The range slice additionally references `establish_connection`, `retry`, and
`download` in that file: range requests, Content-Range checks, retained byte
counts after interrupted reads, and short-response retries. Strong ETag/If-Range
and final-URL checks are stricter native policies; upstream does not implement
these guards. Upstream's heuristic HTTP 416 completion and persistent partial
files are deliberately excluded. The baseline's
`test/test_downloader_http.py` was reviewed for its localhost HTTP fixture,
known/unknown length, and chunk/range cases; `tests/retries.rs` adds independent
deterministic failure/recovery coverage for validated 206 responses, repeated
interruptions, range rejection, changed representations, unknown lengths, and
validator boundaries. This ports a transport slice from the
existing baseline and does not advance it.

## Extraction retries

`--extractor-retries N` opts into up to N additional whole-extraction attempts
(default 0, maximum 255). The library exposes the same policy through
`extract_result_with_options(url, format, ExtractionOptions { retries: N })`.
Existing extraction methods retain zero retries. Call `into_video()` on the
result when a single video is required; playlists still fail at that boundary.

Retry HTTP 408, 429, 500, 502, 503, and 504, connection/timeouts, and HTML
response body read failures. Use the same bounded backoff and delay-seconds/HTTP-date
Retry-After policy as download retries. A new attempt runs the extractor registry from the start, re-fetching
pages and repeating earlier source probes, with the page referrer preserved.
This is a per-input budget, shared by all entries and sources; there is no
per-request budget and no reuse of partial HTML or extracted entries. Custom
extractors opting into this API must tolerate repeated calls. They can report
network body read failures with `Error::ExtractionRead`; ordinary `Error::Io`
remains a permanent failure so local filesystem errors cannot cause retries.

Format selection, unsupported content/schemes, invalid URLs, HTML size and encoding
limits, unsolicited 206 responses, and permanent HTTP statuses fail without
another attempt. After exhaustion return the final error. No partial playlist
metadata or downloads are emitted. The CLI continues to later input URLs and
returns a failing exit status if any input fails. The policy also applies under
`--simulate` and `--dump-json`; `--retries` controls downloads independently.
Standalone `Extractor` calls do not retry automatically.

This is a native transport policy extension of the existing supported subset,
not a port of upstream's extractor-specific retry semantics or defaults. It does
not advance the pinned baseline or claim general `--extractor-retries` parity.
`tests/retries.rs` covers status recovery/exhaustion, interrupted HTML, full
playlist restart after source-probe failures, referrers, CLI modes and budget
isolation, permanent error boundaries, and custom extractor filesystem errors.
The Retry-After extension is a native policy on this existing retry slice;
it does not advance the upstream baseline. Pure unit tests cover parsing and
delay bounds without sleeping; local HTTP tests cover recovery delays, exhausted
budgets, permanent failures, unsupported hints, and preserved range state.
The HTTP-date extension follows the preferred date form in
[RFC 9110 sections 5.6.7 and 10.2.3](https://www.rfc-editor.org/rfc/rfc9110.html#name-retry-after);
obsolete formats are explicitly outside this subset. Date parsing uses
`httpdate`, with a canonical-format check to reject obsolete or noncanonical
forms. Fixed-clock unit tests cover future/past dates, fractional delays, caps,
invalid dates, duplicate hints, and independence from the response Date header.
Local HTTP regressions cover expired-hint recovery and capped future hints on
exhausted budgets and permanent statuses for extraction and downloads.
Persistent resume, obsolete HTTP-date Retry-After formats, extraction cancellation,
and an overall extraction deadline remain unimplemented.

## Download progress and cooperative cancellation

`YoutubeDl::download_with_progress(info, path, options, callback)` extends the
existing download policy with a synchronous `FnMut(DownloadProgress) ->
ControlFlow<()>` observer. Existing `download` and `download_with_options` methods
keep their signatures and use a continuing observer. `DownloadProgress` contains
`attempt` (one-based, up to 256), `downloaded_bytes`, `total_bytes`, and `phase`.

Checkpoints run before each request (`Requesting`), after validated response
headers and each successfully written chunk (`Downloading`), during retry waits
(`WaitingToRetry`), and after flush/sync immediately before no-clobber publication
(`Publishing`). Retry waits sleep at most 100 ms between callbacks; scheduling
and callback runtime can add latency. Waiting events describe the failed attempt,
with the next `Requesting` event advancing the attempt number. Permanent or
exhausted failures do not emit waiting or publication events. Preflight output
collisions and filesystem errors may return before any callback.

Byte counts describe the current temporary file, not cumulative network traffic.
Totals come from download response headers, never extraction metadata; missing
lengths remain `None`. During `Requesting` and `WaitingToRetry`, the last known
count/total is retained (initially zero/`None`). A validated range response retains
the offset and original total. A full replacement response resets the byte count
to zero and updates the total before the next `Downloading` event. A transfer
with unknown length can still succeed; its final total remains `None`.

Return `ControlFlow::Break(())` at any checkpoint to get
`Error::DownloadCancelled`. Cancellation never retries, never publishes, and
removes the private temporary file, including when cancelling after the last
chunk or at `Publishing`. Created parent directories may remain. A callback can
inspect a caller-owned cancellation flag; there is no separate token type.
Callbacks execute on the caller's thread. Cancellation cannot interrupt blocking
requests, reads, flushes, or syncs; request timeouts still apply. Extraction has
no cancellation observer. `Publishing` does not guarantee success: only the
method's `Ok(bytes)` return confirms successful publication. No callback runs
after publication, and existing destinations remain protected even if created
by a callback immediately before publication.

The CLI reports stored bytes and the known total, when available, plus the
attempt number on stderr at most once per second at downloading checkpoints.
Fast downloads can emit only the existing destination/completion diagnostics.
JSON and simulation do not download and emit no download progress. The CLI does
not install signal handlers or expose cancellation flags.

This is a native extension of the transport milestone, not a port of upstream's
progress-hook contract or CLI cancellation behavior. The pinned baseline remains
**897341241f2bfb2439fe0367f4c7c9ccd29a3828**. Deterministic local HTTP tests in
`tests/progress.rs` cover observer events, response-derived/unknown totals,
resumed and restarted counters, cancellation cleanup, and retry-wait polling.

## Source and differential fixtures

This slice uses the existing baseline, **897341241f2bfb2439fe0367f4c7c9ccd29a3828**:

- `yt_dlp/extractor/generic.py`: response dispatch, HTML embed orchestration, and single-vs-playlist dispatch.
- `yt_dlp/extractor/genericembeds.py`, `HTML5MediaEmbedIE`: numbered entry metadata for single videos and playlists.
- `yt_dlp/extractor/common.py`, `_parse_html5_media_entries`: media/source parsing.
- `test/test_InfoExtractor.py`, `test_parse_html5_media_entries`: inline video,
  nested sources, multiple formats, relative URLs, and manifest examples reviewed.
- `yt_dlp/YoutubeDL.py`: playlist entry processing and per-entry JSON output
  reviewed for the bounded sequential orchestration subset.

The full upstream helpers support many behaviors outside this Rust subset.
This is a new port slice from the pinned revision, not an upstream update; the
global baseline has not advanced.

Run the optional differential harness against an official checkout at that SHA:

```sh
cargo build --locked
python3 tools/compare_html5.py --upstream .upstream/yt-dlp --binary target/debug/yt-dlp-rs
```

The harness serves `tests/fixtures/html5/{video,audio,source,formats,playlist,playlist-formats,ranked}.html` and fixed media
bytes on localhost. It invokes upstream with only Generic and HTML5MediaEmbed
enabled and compares `id`, `title`, `ext`, `url`, `webpage_url`, `extractor`, and
`extractor_key` exactly. The sole URL normalization removes the fixture server's
exact origin. Query strings, fragments, paths, titles, and suffixes are preserved.
Other fields (including upstream timestamps and fields outside VideoInfo) are
excluded by this explicit allowlist. No volatile-value masking is applied within
the compared fields. Rust-specific boundary behavior is covered by `tests/http.rs`.
For playlists the
harness compares every entry in order and requires the exact entry count. The
CLI emits per-entry NDJSON, so wrapper fields are covered by library integration
tests rather than this differential harness.

The multi-source fixtures have explicit upstream labels `0` and `1` so each
implementation can request the same source without equating their general ID
assignment rules. Both selections are compared, including for every playlist
entry. The format list, probe sizes,
and Rust-specific IDs are covered by deterministic Rust tests rather than the
upstream metadata allowlist. The multi-source fixture uses page-relative URLs;
`base[href]` resolution remains a separately tested Rust behavior because the
pinned upstream HTML5 embed path does not apply it in this fixture.

The comparison needs upstream source and is optional; required tests need no
upstream checkout, Python runtime integration, or live sites. Seven fixtures do
not establish broad extractor parity. Site playlists, playlist selection, and
recursive result processing remain unimplemented.

## Progressive automatic selection source slice

This slice uses the existing pinned source commit
**897341241f2bfb2439fe0367f4c7c9ccd29a3828** and leaves the global baseline unchanged.
`InfoExtractor._parse_html5_media_entries` in `yt_dlp/extractor/common.py` supplies
source width/height/res handling; its corresponding dimension examples in
`test/test_InfoExtractor.py` were reviewed. `FormatSorter` in
`yt_dlp/utils/_utils.py` supplies the shorter-known-dimension resolution concept,
and `YoutubeDL.build_format_selector` in `yt_dlp/YoutubeDL.py` supplies best/worst
selection direction. Other upstream ordering fields, stream eligibility, label
parsing remain outside this progressive ranking slice; bounded adaptive
expressions are described below. Declaration-order ties
are a native policy; upstream may choose differently for equal resolution.

The saved `ranked.html` fixture compares default, `best`, `worst`, and literal-ID
normalized selected metadata with upstream through `tools/compare_html5.py`.
All alternatives use the same progressive media type and distinct declared
resolutions to isolate the supported ranking criterion. Dimension metadata,
unknown/tie behavior, invalid attributes, explicit overrides, CLI transfers, and
independent playlist selection are covered by deterministic Rust tests.


## Direct adaptive selection and merging

`StreamKind` adds `standalone` (the default, omitted from JSON), `video_only`,
and `audio_only`. Generic sources retain standalone classification because their
track composition is unknown. Optional `tbr`/`abr` are positive finite declared
kilobits per second, not measured quality. YouTube uses averageBitrate then bitrate.
The anonymous WEB API direct-format path still accepts only itag 18; adaptive support does not bypass
that token policy.

`best`/`worst` choose standalone sources; an audio-only fallback is allowed only
when neither standalone nor video-only sources exist. `bestvideo`/`worstvideo`
choose video-only sources by resolution then total bitrate; `bestaudio`/`worstaudio`
choose audio-only sources by audio then total bitrate. Unknown values rank lowest.
Standalone ranking retains resolution-only policy. Best ties choose the last
candidate, worst ties the first. Literal IDs also select isolated streams.

A `+` expression requires exactly one video-only and one audio-only source.
`/` tries alternatives in order, for example `bestvideo+bestaudio/best`.
Every branch must have valid bounded syntax. Selection errors leave metadata
unchanged, including across playlists. Without an explicit selector, configuring
FFmpeg prefers that pair/fallback expression; otherwise default equals `best`.
An explicit pair can be inspected with JSON/simulation without installing FFmpeg.

A merge plan carries `requested_formats` in expression order and a joined
`format_id`. Top-level `url` represents the first source, not a merged HTTP URL;
callers must honor the plan. MP4/TS video + M4A/AAC/TS audio yields MP4, WebM + WebM yields
WebM, and mixed supported containers yield Matroska (`mkv`). The selected video's dimensions
are retained; merged filesize is unknown until download. Added serde fields have
defaults for older metadata, but Rust struct literals must supply the new fields.

`YoutubeDl::with_ffmpeg(path)` or `--ffmpeg-location PATH` permits downloading
both sources into a private destination-directory workspace, then stream copying
with explicit video/audio maps. Inputs use fixed local demuxers and the file-only
protocol whitelist. The executable is canonicalized; its version banner is checked.
No shell is used, environment is cleared except Windows system directories, stdin
is closed, and process output and elapsed time are bounded (64 KiB stdout,
256 KiB stderr, ten minutes shared by version check and merge). A process error,
nonempty error diagnostics, or empty output fails explicitly. No codec fallback
or transcoding occurs. A bad tool may fail after source downloads.

Each input receives the usual download retry budget. Progress includes one-based
`stream_index` and `stream_count=2`; input byte counts reset for the next source.
Input publication events are hidden. `Merging` reports the sum of stored input
bytes, with cooperative process cancellation polling. Final `Publishing` reports
the merged output size. Only that final file is published, using no-clobber
semantics; cancellation/failure removes both inputs and staging output. Single
stream downloads never inspect the configured FFmpeg path.

Optional real-tool regression: set both `YT_DLP_RS_TEST_FFMPEG` and
`YT_DLP_RS_TEST_FFPROBE` to executable paths and run `cargo test --locked --test merge`.
The generated local media fixtures verify that copied output retains audio and
video tracks; this is not a claim that every FFmpeg version or codec pair works.
See [source and fixture provenance](../upstream/notes/youtube-adaptive-merge.md).


## Native HLS VOD and fragment transport

Direct HLS playlist URLs and HTML5 HLS sources support a finite VOD
subset, including identity AES-128 encryption. Recognize MPEGURL MIME types, final `.m3u8` URL paths, and explicit HTML5
HLS type hints. A conflicting media MIME must not cause a manifest to be saved as
one media file. Read at most 2 MiB of uncompressed UTF-8/ASCII manifest text;
ambiguous encoding/content-type headers fail. URLs resolve against each final
redirected manifest URL. HTML5 page referrers propagate to manifests and segments.

Master playlists expose declaration-ordered `hls-N` format IDs, positive declared
bandwidth in kbps, optional resolution and codec-derived stream kinds. HTML5 master
sources flatten to `sourceIndex-hls-N`; a media playlist retains its ordinary
source index. Existing standalone selection retains resolution/declaration-order
ranking, not upstream's full bandwidth/codec sorter. Every variant manifest is
read before selection; segment bodies are not probed. One master level, at most
256 variants and 10000 retained fragments across a master are supported.

Media playlists require `EXT-X-ENDLIST`, nonempty positive-duration segments and
HTTP(S) URLs. A nonzero media sequence is permitted for a completed VOD. Without
an initialization map, `.aac` segments produce AAC and other ordinary/extensionless
segment URLs default to MPEG-TS (`ts`); clear nested manifests/HTML and MP4 media
without initialization are rejected. One initial `EXT-X-MAP` produces fragmented
MP4 (`mp4`). This is manifest interpretation, not a universal container probe.
Native assembly preserves the segment bytes; TS is not silently labeled MP4 or
remuxed. FFmpeg is unnecessary for this download path.

`EXT-X-BYTERANGE` supports explicit offsets and implicit continuation only after
an immediately preceding media range on the same URL. Map byte ranges require
an explicit offset. All ranges are positive with checked arithmetic. Unsupported
state is an error: live playlists without ENDLIST, SAMPLE-AES/DRM or unsupported
key formats, discontinuities, gaps, low-latency tags, variables, changing maps,
external video/subtitle or unresolved rendition groups, mixed inband/external
audio groups, nested masters and unknown EXT tags.
In-band rendition references without external resources are supported. Plain
comments and recognized informational metadata do not change assembly.

`VideoInfo.fragments` and each format's `fragments` contain ordered `Fragment`
values, including initialization when needed. Each has a URL and optional
`ByteRange { offset, length }`; absent fields deserialize to empty/default values.
Selection copies the chosen plan; a merge plan retains each child's fragments.
Extraction snapshots these plans, and downloads do not refetch manifests.

The fragment downloader retries each whole failed fragment using `--retries`,
retains earlier complete fragments, and truncates the failed fragment before
retrying. It never skips a failed segment. Exact ranges require matching 206,
Content-Range, known resource bounds, validated length and identity encoding;
ignored ranges, unsolicited partials, HTML/manifests and empty fragments fail.
Private temporary output is removed on errors/cancellation; final publication
uses the same no-clobber race protection as direct downloads.

Progress adds `fragment_index` and `fragment_count` with one-based positions;
bytes count the assembled output so far, including initialization, and can reset
to the previous complete prefix on retry. Attempt numbers restart per fragment.
Total bytes remain unknown. Observers can cancel before requests, after writes,
during retry waits and before final publication. Blocking reads retain normal
request timeouts. These new public fields require updating Rust struct literals.

The saved TS, fMP4 and byte-range media tests verify exact bytes and optionally
FFprobe audio/video tracks using `YT_DLP_RS_TEST_FFPROBE`. The offline differential
harness compares actual pinned upstream HlsFD fragment order, URLs and ranges:

```sh
python3 tools/compare_hls.py --upstream /path/to/yt-dlp --binary target/debug/yt-dlp-rs
```

Only the exact ephemeral localhost origin and upstream exclusive range end are
normalized. Container naming/remux behavior, live/encrypted delivery and general
HLS parity are not inferred. YouTube HLS discovery uses this same bounded VOD
pipeline. See [the source slice](../upstream/notes/hls-vod.md).


### HLS AES-128

`EXT-X-KEY:METHOD=AES-128` supports raw 16-byte identity keys from HTTP(S) URLs,
optional `KEYFORMAT="identity"` / `KEYFORMATVERSIONS="1"`, and CBC with strict
PKCS#7 padding. `METHOD=NONE` clears encryption. Explicit IVs are hexadecimal
integers of up to 128 bits, left-padded to 16 bytes. Otherwise the IV is the
media segment sequence number in a 16-byte big-endian buffer. Initialization
maps require an explicit IV while encrypted, and do not consume media sequence
numbers. These rules follow [RFC 8216 sections 4.3.2.4, 4.3.2.5 and 5.2](https://www.rfc-editor.org/rfc/rfc8216.html).

Each fragment's optional `encryption` contains `HlsAes128 { key_url, iv }` only;
key bytes never appear in metadata. JSON and simulation request neither keys
nor segment bodies. Downloads retrieve keys lazily, preserving the page referrer
and identity encoding, with exact 16-byte length and response validation. A URL's
validated key is cached only for the current download; rotation to a different
URI fetches a new key. Key and segment requests share that fragment's retry
budget. Cancellation includes key request/read checkpoints.

RustCrypto `aes`/`cbc` performs bounded streaming decryption without an external
process. The last ciphertext block is held until EOF for padding validation.
Ciphertext lengths/ranges are validated against network headers; stored byte
counts, publication size and observer progress use plaintext bytes. Key bytes and
padding do not contribute to progress. Invalid keys/ciphertext/padding and
unsupported methods fail without publication. Earlier plaintext may exist only
in the private temporary file until all fragments pass validation.

Original OpenSSL-encrypted TS, fMP4/map and byte-range fixtures decrypt to the
exact original bytes and pass optional FFprobe track checks. Fixed NIST/RustCrypto
vectors cover CBC and padding independently of the fixture generator. Parser and
local HTTP tests cover rotation, NONE, sequence bounds, key caching, retries,
malformed responses and cancellation. The upstream plan comparison additionally
checks key URLs and IVs. Pinned upstream increments its counter for MAP; native
implicit-IV-after-MAP behavior follows RFC media sequence numbering, and that
specific upstream equivalence is explicitly excluded. Explicit-IV MAP plans
are compared without masking this difference.

SAMPLE-AES, DRM key formats, session keys, custom key overrides and live/key
refresh across playlist reloads remain unsupported. See the
[AES source slice](../upstream/notes/hls-aes128.md).


### External HLS audio renditions

`EXT-X-MEDIA:TYPE=AUDIO` declarations with HTTP(S) playlist URIs now expose
`hls-audio-N` formats after the existing `hls-N` variants. Per-format metadata
contains `audio_group`, optional `language` and `format_note` (NAME), and true
`audio_default` / `audio_autoselect` flags. Missing flags are false. Empty/duplicate
names in a group, multiple default renditions, invalid flag values, and explicit
DEFAULT=YES with AUTOSELECT=NO fail. Audio-only rendition masters are supported.

A variant that references an external audio group must declare recognized video
CODECS. It becomes video-only even when CODECS describes both tracks. Missing or
audio-only CODECS on such a variant fails instead of guessing a complete stream.
Inband groups retain the existing standalone behavior. Both variant and rendition
declarations count toward the combined limit of 256; every child playlist is
read with the existing aggregate 10000-fragment cap. JSON/simulation fetch neither
segments nor encryption keys. Audio-only fMP4 is labeled M4A; raw AAC and TS retain
their containers.

Automatic pair selectors choose a complete compatible pair in video rank order,
then rank audio within that video's group. Both absent groups also match.
Expression order is preserved in the plan even for `bestaudio+bestvideo`.
One literal ID constrains the other generic selector to its group. Two literal
IDs explicitly override group matching, for example `-f hls-0+hls-audio-1`.
Best audio ranks DEFAULT, then AUTOSELECT, then declared audio/total bitrate;
worst reverses that ordering. Declaration ties retain existing rules. There is
no full upstream preference/sorting parity. Explicit language predicates are
available through the shared format-filter grammar.

HTML5 master format IDs and audio groups both receive the source index prefix,
so equal group names from independent source masters cannot pair automatically.
Without FFmpeg configuration, implicit default selection fails when only separate
video/audio formats exist. An explicit pair may still be inspected with JSON.
With FFmpeg, implicit default prefers a compatible pair and downloads both native
fragment plans before merging. TS video and AAC/TS audio use fixed local demuxers;
MP4 muxing relies on FFmpeg's AAC ADTS-to-ASC handling. No transcoding is introduced.

Local tests cover group/default selection, explicit overrides, audio-only masters,
validation, HTML namespaces, headers and no-segment simulation. Original separate
H.264 TS and AAC fixtures exercise the complete native-download/FFmpeg path and
verify both tracks and AAC codec metadata. The optional pinned-parser comparison:

```sh
python3 tools/compare_hls_audio.py --upstream /path/to/yt-dlp --binary target/debug/yt-dlp-rs
```

It compares URL-keyed stream kind, language/name, dimensions and total bitrate.
Format IDs/order, native group/flag fields, selection, ext/remuxing and download
behavior are explicitly excluded from that comparison. Only the exact local
origin is normalized. See [the source slice](../upstream/notes/hls-audio.md).


## Static DASH MPDs

Direct `.mpd` paths, `application/dash+xml`, and HTML5 DASH type hints support a
bounded static, single-period MPD subset. Original/final MPD paths and explicit
hints force parsing even with a conflicting media MIME, preventing MPD text from
being published as media. Read at most 2 MiB of identity-encoded UTF-8 XML;
ambiguous/invalid headers, DTDs, foreign element namespaces and external XML
references fail. No schema, xlink, initialization or media URL is fetched during
extraction or JSON/simulation. XML nodes/depth are bounded at 100000/64.

`BaseURL` resolves through MPD, Period, AdaptationSet and Representation, with one
base per level. SegmentTemplate attributes/timelines and SegmentList initialization
and segment lists inherit within the same addressing scheme. Supported template
tokens are RepresentationID, Bandwidth, Number and Time, dollar escaping and
bounded zero-padded decimal numeric forms. Initialization permits only RepresentationID
and Bandwidth; media cannot combine actual Number and Time tokens. Static duration
templates require a
finite Period or MPD duration. Exact day/hour/minute/second ISO durations use
nanosecond precision (up to nine fractional second digits), checked integer
arithmetic and ceiling division. Explicit Period and MPD duration bounds must agree.
No floating-point segment count is used.

SegmentTimeline supports explicit/implicit starts, positive durations, finite
repeats and bounded `r=-1` expansion to an aligned next start or finite period
end. Nonzero presentationTimeOffset is applied to period bounds and duration
Time templates; timelines that start before the offset require clipping and are
rejected, as are segment starts at or beyond a known period end. Namespaced
addressing attributes fail rather than being interpreted as ordinary DASH fields.
The simple-addressing offset policy follows the
[DASH-IF restricted timing model](https://dashif.org/Guidelines-TimingModel/).
Unknown eptDelta and advanced timing attributes remain unsupported. Final segment
bytes are not cropped to presentation boundaries; no trimming/remux parity is
claimed.

SegmentList supports Initialization sourceURL/range and SegmentURL media/mediaRange.
Inclusive XML byte endpoints become exact ByteRange offset/length values. Explicit
resource BaseURLs may supply omitted range-resource URIs. Fragmented MP4/WebM
plans require initialization; an explicit whole-resource non-directory BaseURL
can instead form a single-fragment representation. Resolved URLs are limited to
16 KiB, formats to 256 and aggregate fragments (including initialization) to 10000.

Formats use native declaration-order `dash-N` IDs, declared stream kind, MP4/M4A
or WebM extension, dimensions, bandwidth-derived kbps and inherited language.
Video/audio representations share the native `dash-period-0` group; HTML source
prefixes isolate independent masters. Their representative URL is the MPD URL;
the ordered fragments describe actual resources. Existing selectors, per-fragment
retries/cancellation, no-clobber publication and explicit FFmpeg merge policy apply.
Without a merger, separate-only presentations need explicit single-track selection.

Reject dynamic MPDs, multiple periods, ContentProtection/DRM, SegmentBase/SIDX and
index attributes, dependent/scalable representations, SubRepresentation,
EssentialProperty, remapping, multiple BaseURL alternatives, mixed inherited
addressing schemes and unsupported state. YouTube MPD discovery remains outside
its extractor. No live/site reliability or general DASH parity is implied.

Original local fMP4 fixtures verify exact single-track bytes and optional FFprobe
H.264/AAC tracks for timeline, duration-template, list and range forms. The real
merge test downloads both representations and checks the combined tracks. The
Linux real-tool job includes these checks. Pinned reference command:

```sh
python3 tools/compare_dash.py --upstream /path/to/yt-dlp --binary target/debug/yt-dlp-rs
```

The actual upstream parser receives saved XML with network access blocked; native
JSON reads only localhost MPDs. A documented allowlist compares declaration-order
metadata and fragment URLs, resolving upstream relative paths and normalizing
only the exact local origin. Source ID and audio bitrate representations have
explicit mappings. Pinned upstream ignores SegmentList ranges and does not expand
negative repeats or apply presentationTimeOffset: native range expectations are
checked independently, and those timing behaviors are excluded from upstream
parity claims. See [the source slice](../upstream/notes/dash-static.md).


## YouTube HLS discovery

Playable matching single-video initial responses and the anonymous WEB API
fallback may supply `streamingData.hlsManifestUrl`. The pinned WEB GVS policy makes
HLS tokens optional; it requires a token for DASH. DASH URLs are excluded before
preparation, solving and requests. This adds no token acquisition, authentication,
client selection or live/post-live support.

Direct URL query-n/signature and HLS `/n/{token}/` path challenges share one
complete offline solver batch before media probes or manifest requests. An explicit
Deno runtime is required for eligible challenged sources; without one those
sources are skipped. Challenge-free HLS never inspects runtime/player configuration.
Path tokens retain their raw spelling for upstream EJS matching; solved values
are encoded as a single URL segment. Strict URL preparation rejects ambiguous
or duplicate path markers, invalid escapes, fragments, query-n manifests and
legacy encrypted `/s/` paths. The latter two are unsupported native cases; the
pinned upstream manifest branch does not solve them.

HLS formats follow eligible direct formats, with native `hls-N` / `hls-audio-N`
IDs and `youtube-hls:` audio group namespaces. Direct numeric itags are unchanged.
A media-only playlist forms `hls-0`; master formats retain dimensions, bandwidth,
language and rendition metadata from the existing native HLS parser. Selection
uses native ranking and pair rules, with explicit FFmpeg configuration for merging.
Representative fragments are retained even before library selection. Container
extensions follow native TS/AAC/fMP4 assembly rather than upstream's default MP4
remux label. No upstream format-ID or sorter parity is implied.

Only eligible direct formats are probed as media. HLS acquisition reads the master
and its media playlists; extraction/JSON/simulation never fetch initialization,
segments or AES keys. Downloads use the canonical watch-page referrer and existing
fragment transport. Every retained manifest must parse within the supported VOD
contract; an unsupported or failing retained manifest fails extraction even when
another direct format could work. Existing extraction retries can restart the
whole page/API/manifest sequence after a transient acquisition failure.

Offline source comparison:

```sh
python3 tools/compare_youtube_hls.py --upstream /path/to/yt-dlp
```

The actual pinned YoutubeIE and master parser run with network blocked and no
GVS token or missing_pot override. Plain and path-n saved responses compare
metadata and URLs against the same expected JSON consumed by Rust tests. A fixed
typed n solution checks path replacement; it does not execute EJS. Container,
fragment, transport and selection behavior have separate native tests.

See [the source slice](../upstream/notes/youtube-hls.md) for source applicability,
fixture provenance and offline comparison exclusions. Saved fixtures establish
this bounded extraction behavior, not current live-site availability.


## Format predicates

Selector atoms and literal format IDs accept chained `[field operator value]`
predicates before `+` pairing and `/` fallback. For example:

```sh
-f 'best[height<=720][ext=mp4]/best'
-f 'bestvideo[height<=1080]+bestaudio[abr>=128]/best'
-f 'best[filesize<10MiB]'
-f 'bestaudio[language^=en][format_note*="English / main"]'
```

Numeric fields are `width`, `height`, `filesize`, `tbr` and `abr`; bitrate values
are kbps. Operators are `<`, `<=`, `>`, `>=`, `=`, `!=`. Nonnegative decimals and
pinned upstream filesize-style decimal/binary unit suffixes are supported.
Unit suffixes preserve upstream case-dependent multipliers and rounding; bare
filesize comparisons retain exact integer/rational precision rather than casting
large byte counts to floating point. Native large-integer guarantees are tested
separately from upstream's floating-point thresholds.

Text fields are `ext`, `format_id`, `language`, `format_note` and `audio_group`.
Support equality `=`, prefix `^=`, suffix `$=`, substring `*=` and their negations
`!=`, `!^=`, `!$=`, `!*=`. Quoted strings can contain spaces, brackets and selector
delimiters; quote/backslash escapes follow the supported upstream string syntax.
Comparisons are case-sensitive. Preserve upstream numeric-first equality parsing:
unquoted numeric-looking text values compare as numbers against strings, so
`[format_id=18]` does not match the string ID `18`; use `[format_id="18"]` for
text equality. Quoted numeric values on numeric fields remain unsupported.
Numeric and text operators may append `?` to
include unknown values, such as `[height<=?720]`; unknown values otherwise fail,
including negated predicates. Numeric fields cannot use text operators or vice
versa. Unsupported fields fail explicitly rather than being treated as absent.

All predicates are ANDed and applied before stream-kind fallback and native
ranking. Literal IDs must also pass their predicates. Existing group matching
still applies to automatic pairs; two literal IDs retain their explicit override.
Direct results without format alternatives evaluate their available top-level
metadata without inventing a format ID or bitrate/language fields. Failed
selection preserves the result, including all playlist entries. Every branch is
parsed before selection, so a malformed later fallback cannot be hidden by an
earlier successful branch. Selection still follows extraction of all alternatives;
filters do not reduce extraction probes or manifest acquisition.

The grammar is bounded to 8 KiB, 16 global fallback choices, 16 group levels,
128 syntax nodes, two streams on any possible branch and 16 predicates per atom
or group. Regex predicates, comma/all selections and other unsupported upstream
modifiers remain errors. Predicate
support does not imply full upstream selection/sorting parity. See
[the source slice](../upstream/notes/format-filters.md) and the shared saved cases.


## Grouped selector expressions

Parentheses scope fallback and merge expressions, with `+` binding more tightly
than `/`. Supported aliases combine `best`, `worst`, `b` or `w` with an optional
`video`/`v` or `audio`/`a`: `b`, `bv`, `ba`, `w`, `wv`, `wa`, `bestv`, `worsta`
and their long equivalents use the same native selectors. Bare filters imply
`best`, so `[height<=720]` equals `best[height<=720]`.

```sh
-f '(bv[height<=720]+ba)/b'
-f '(b/w)[ext=mp4]'
-f 'bv+(ba[language=en]/ba)'
```

A filter after a group narrows its input formats before evaluating the group's
children. For example, `(bv+ba/b)[height<=720]` removes unknown-height audio before
trying the pair, while `bv[height<=720]+ba` filters only video. Use `<=?` to retain
unknown heights where intended. A group does not filter the combined output.

Fallbacks retain their scope. If an inner fallback selects an available branch,
a surrounding merge does not revisit later inner branches when the chosen source
cannot form a supported pair. Put the fallback outside the full merge when that
behavior is desired. Existing native compatible-group pair search still applies;
literal ID provenance survives groups, preserving the deliberate override for
two explicit IDs. Direct results evaluate grouped filters on known metadata and
retain their absent format ID. Selection failures remain atomic.

The entire expression is parsed before selection. Empty/unbalanced/adjacent
groups, malformed later branches and any possible branch with more than two
streams fail, even when an earlier branch is available. Stars, extension selectors, comma/all selections and multistream merging
remain outside the supported grammar. Literal extension-shaped IDs retain their
existing ID meaning; they do not request an extension automatically.

The shared expression fixtures compare selected IDs against actual pinned
`YoutubeDL.build_format_selector` with an explicitly ordered format list and
context flags. No upstream sorter runs. They cover groups, precedence, aliases,
bare/group filters and unavailable branches; they do not establish full ranking,
codec/modifier, incomplete-format fallback, stream deduplication or container parity.
Native pair/group policy remains documented separately. Source and command:

```sh
python3 tools/compare_selectors.py --upstream /path/to/yt-dlp
```

See [the source slice](../upstream/notes/selector-groups.md).


## Indexed selector ranks

Best/worst selectors and their aliases accept a positive one-based `.N` suffix:
`best.2`, `worst.2`, `bv.2`, `ba.2` and their long forms. Apply atom and enclosing
group predicates first, choose the represented stream kind, apply native ranking
and then select Nth from that order. Existing declaration ties still apply.

```sh
-f 'bv.2[height<=1080]+ba.2/b'
-f '(b.3/b.2/b)[ext=mp4]'
```

A rank beyond the available matching formats is unavailable, so the enclosing
fallback can try its next branch. Explicit indexing pins an atom to one candidate
before native rendition-group/container validation. It does not silently move to
a different ranked format to form a pair. Unindexed atoms retain the existing
compatible-pair search policy; this is a native group-policy distinction outside
the upstream comparison. An indexed selector remains a selector and cannot use
the two-literal-ID group override.

An implicit direct result has one represented source: `b.1` and `w.1` can match
known metadata, while `.2` is unavailable and no format ID is invented. Rank
syntax starts with ASCII 1–9 and continues with digits. Names such as `bv.0`,
`bv.01`, `bv.x` and non-selector dotted IDs retain literal-ID meaning, matching
the pinned alias regex rather than being reinterpreted as a rank.

Indexes above the platform signed index maximum fail explicitly during complete
expression parsing, including later branches; no integer wrapping, huge allocation
or implicit best fallback occurs. Pinned upstream's oversized iterator errors and
very-long-integer conversion fallback are not emulated. Group suffix ranks such
as `(bv/ba).2`, stars, multiselection and full sorting remain unsupported.

The saved expression comparison includes indexed aliases, filtered/grouped ranks,
ordered pairs and unavailable ranks within the supported range. Native tests
separately verify ties, direct metadata, dotted literal IDs, compatibility failure
and oversized-index errors. See [the source slice](../upstream/notes/selector-index.md).


## Available format inventory

`-F` / `--list-formats` alone extracts all eligible alternatives and prints a
format table for each video. It bypasses library selection, so separate-only HLS,
DASH or supported direct adaptive presentations can be inspected without a merger.
A valid `-f` argument does not filter listing-only output or require that its ID
exists; its supported grammar is still validated before any requests.

As in pinned upstream control flow, adding `-j` or `-s` retains normal selection.
`-F -j` prints the complete table followed by selected JSON; this combined stdout
is human text plus JSON, rather than plain NDJSON. `-F -s` selects and prints the
table without downloading. Existing native selection/default/group policies apply,
so a separate-only combined mode may require an explicit pair selector or merger
configuration. `--no-simulate` and upstream listing-style toggles remain unsupported;
listing never downloads in this CLI subset.

Tables retain extractor declaration order and show ID, extension, resolution,
stream kind, exact declared/probed filesize in bytes, declared kbps rates, language
and rendition notes/default/autoselect flags. Missing fields display `-`; sizes
and codecs are not inferred from fragments. Standalone means independently usable
or unknown composition, not a claim that both audio and video exist. An implicit
source without a formats array has one row with `-` as its non-selectable ID.
Resource URL and encryption fields are not rendered. Table layout, number/size
formatting and order follow native metadata policy, not upstream sorter or table
byte parity. Unicode is preserved with character-count alignment; terminal-width
alignment for all scripts is not claimed.

Controls and line/bidirectional control characters are visibly escaped before
alignment. Cells are limited to 128 characters and captions to 256, with an
ellipsis for truncation. Full metadata remains available in JSON. Listing output
is rendered only after every entry in the input extracts successfully (and, for
combined modes, selects successfully). This preserves native playlist atomicity;
upstream may print a table before a later selection failure.

Extraction uses normal probes, bounded manifest reads, eligibility and optional
challenge solving. Listing does not fetch segment bodies, initialization resources
or AES keys, inspect FFmpeg, render output paths or create media files. Independent
`--extractor-retries` still restarts the full extraction after transient failures;
format/rendering failures are not extraction retries. Multiple inputs retain the
usual error continuation and nonzero final status on any failure.

The library exposes `extract_result_unselected` and
`extract_result_unselected_with_options` for inspection without selection. Their
metadata contains the extractor's representative fields; call selection before
download. Existing extraction methods still select every entry. The pure
`output::render_formats` has no acquisition or selection effects, and
`validate_format_selector` validates syntax without checking format availability.
See [the source slice](../upstream/notes/list-formats.md) for reference probes and
rendering comparison limits.
