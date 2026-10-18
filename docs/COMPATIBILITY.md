# Supported compatibility subset

This inventory describes tested behavior, not general yt-dlp compatibility.
All extraction and downloads run in Rust. Python is development tooling only.

## CLI

| Option | Current behavior |
| --- | --- |
| Positional URLs | One or more HTTP(S) URLs, processed sequentially; continue after errors, return failure if any URL fails |
| `-j`, `--dump-json` | One supported `VideoInfo` JSON object per video, including each HTML5 playlist entry; extract without downloading |
| `-s`, `--simulate` | Extract metadata, write diagnostics to stderr, create no media files |
| `-f`, `--format` | Bounded single-source ranking/IDs, audio/video selectors, two-stream `+` pairs and `/` fallbacks; see selection policy below; no filters or direct-URL format IDs |
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

WEB player requests need no PLAYER PO token at the pinned baseline. Its media
URLs require a GVS PO token except itag 18. API-derived formats therefore retain
only itag 18 before any media probe; no surviving format is an unsupported error.
This policy does not acquire tokens or assert that exempt URLs always work.
The original initial-response path retains its prior token/probe behavior.

Expose `streamingData.formats` followed by `adaptiveFormats` entries resolving
to direct HTTP(S) media, with MP4/M4A/WebM MIME types and recognized audio
and/or video codecs. Classify sources as standalone, video-only or audio-only. Preserve numeric itags as
format IDs and positive declared dimensions. Skip DRM, OTF/fragments, unknown
source types and live fragments. Encrypted `signatureCipher`/legacy `cipher` and
query-n URLs require explicit `--youtube-js-runtime PATH` or library configuration;
reject player responses with DRM license information. Manifest URLs are ignored.
If no supported direct format remains, return an explicit unsupported error.
Supported alternatives retain declaration order and use the existing resolution
ranking or literal itag selection, not upstream's full sorter. Every eligible
URL must pass a direct media probe; a failing probe fails the entire extraction,
including when a different format was requested. Filesizes and final URLs come
from those probes. Token-protected formats can therefore prevent extraction even
if another format might work. Metadata uses `youtube` / `Youtube`, `direct=false`,
and the canonical watch page. The shared downloader sends that page as referrer.

This is fixture-tested groundwork, not general live-site YouTube support. Modern
YouTube may require other API clients, PO tokens, SABR, manifest protocols, or fragmented adaptive delivery; those remain
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

The generic fallback dispatches a single response to direct-media inspection or
HTML5 parsing. Site extractors registered through `Extractor` run first. The
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
serve supported direct HTTP media. Redirected source URLs and response-derived
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
Filters, parentheses, comma selections, more than two merged streams, and
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
and general embedded-player extraction are not implemented. HLS/DASH URLs/type
hints and non-HTTP sources fail explicitly, even
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
The anonymous WEB API still accepts only itag 18; adaptive support does not bypass
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
callers must honor the plan. MP4 video + M4A audio yields MP4, WebM + WebM yields
WebM, and mixed containers yield Matroska (`mkv`). The selected video's dimensions
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
