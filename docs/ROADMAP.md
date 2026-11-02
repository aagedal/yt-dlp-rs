# Rust port roadmap

## Approach

Port behavior in vertical slices: extraction metadata, selection, transport, and
CLI behavior tested together. Keep a usable binary at each milestone. Do not
translate Python files mechanically or depend on Python for runtime behavior.

Upstream has hundreds of extractor modules, shared extraction utilities, multiple
download protocols, and postprocessors. Most of the work is ongoing website
compatibility, rather than implementing an HTTP client. A small stable Rust core,
fixture-driven extractors, and explicit upstream source mappings make that work
tractable for future agents.

## Current architecture

- `src/cli.rs` (the hidden `cli` module; `src/main.rs` only calls it): strict CLI subset, per-URL errors, per-video NDJSON metadata output, sequential HTML5 playlist processing.
- `src/lib.rs`: ordered extractor registry and download orchestration.
- `src/info.rs`: typed subset of upstream video info fields and video/HTML5 playlist results.
- `src/extractor/`: independent extractors, generic fallback last.
- `src/extractor/html5.rs`: generic response dispatch and progressive HTML5 source parsing;
  direct-media response inspection is shared with `DirectHttp` to avoid duplicate probes.
- `src/extractor/youtube.rs`: options, session cookies and dispatch to the
  YouTube submodules; `urls.rs` ports `YoutubeIE._VALID_URL` (Invidious hosts,
  naked IDs) and `redirect.rs`/`notifications.rs` the small YouTube extractors
  (`youtube:playlist`, `YoutubeYtBe`, feed keywords, truncated URLs, consent,
  `youtube:notif`).
- `src/extractor/youtube/args.rs`, `pot.rs`: `--extractor-args youtube:...` and
  configured PO tokens (`po_token`, `visitor_data`, `data_sync_id`).
- `src/extractor/youtube/watch.rs`: watch page, `ytcfg` and per-client player API
  acquisition (`_extract_player_responses`), age-gate fallback and playability.
- `src/extractor/youtube/innertube.rs`: upstream `INNERTUBE_CLIENTS`, generated from the pinned checkout.
- `src/extractor/youtube/streams.rs`: HTTPS, HLS and DASH formats with PO token
  policies and signature/`n` challenges (`challenge.rs`, `challenge/runtime.rs`).
- `src/extractor/youtube/dict.rs`, `metadata.rs`, `subs.rs`, `chapters.rs`: the
  info dict in upstream key order, captions and chapters.
- `src/extractor/youtube/tab.rs`, `search.rs`, `clip.rs`, `comments.rs`: playlists,
  channel tabs, mixes, feeds, searches, clips and comments.
- `src/extractor/youtube/live.rs`, `live_chat.rs`, `watched.rs`: post-live and
  live-from-start formats, live chat (`YoutubeLiveChatFD`) and `--mark-watched`;
  `src/live_dash.rs` and `src/mhtml.rs` port `DashSegmentsFD`'s live generator
  and `MhtmlFD` storyboards.
- `src/cookies.rs`: scoped response cookies and bounded Netscape import/atomic persistence.
- `src/http/proxy.rs`: pure explicit proxy validation and shared client routing.
- `src/http.rs`: validated global request headers and User-Agent/Referer defaults, shared by normal and bounded player clients.
- `src/download.rs`: streaming HTTP transfer, byte-progress/cancellation checkpoints, and no-clobber publication.
- `src/hls.rs` and `src/extractor/hls.rs`: pure VOD HLS parsing and generic/HTML5 manifest acquisition.
- `src/dash.rs` and `src/extractor/dash.rs`: bounded static (multi-period) MPD parsing, upstream's snapshot parse of dynamic (live) MPDs, and generic/HTML5 acquisition.
- `src/fragment.rs`: ordered native fragment/range transport and atomic publication.
- `src/rtmp_fd.rs` and `src/websocket_fd.rs`: upstream's RtmpFD (`rtmpdump`) and
  WebSocketFragmentFD (a websocket client piped into FFmpegFD's stdin, reusable by
  the unported niconico/FC2 live downloaders).
- `src/media_index.rs`, `src/indexed_fd.rs`: port extension for `--download-sections`
  of indexed single-file streams (YouTube `initRange`/`indexRange`): parse the MP4
  `sidx`/WebM `Cues`, fetch only the section's segments, then run `FFmpegFD`'s
  arguments over local files (`ffmpeg_fd::LocalInput`, `-seek_timestamp 1`).
- `src/section_timecode.rs`: port extension `--section-timecode`: ffprobe's frame
  rate and the first shown frame (edit-list start for MP4/MOV cuts, the sought
  keyframe otherwise) become the cut's `-timecode`.
- `src/growing.rs`: port extension `--growing-file`: FFmpegFD writes live streams
  as one fragmented QuickTime file (MP4 for AV1/VP9/Opus/FLAC) at the final name (`ffmpeg_fd::download_growing`;
  `--live-from-start` fragments reach it through named pipes, `live_dash::Job::pipe`),
  with `-progress` as the recorded time and, on macOS, Blackmagic's recording
  attribute while it grows.
- `src/playlist.rs`: playlist item/range parsing, ordered index deduplication and fully extracted entry selection.
- `src/subtitles.rs`: ordered subtitle metadata, language/format selection and sidecar name planning; `src/output/subtitles.rs` renders inventories.
- `src/subtitle_convert.rs`: subtitle conversion publication/retention; nested
  `ttml.rs` ports bounded native text/timing/styles and `ffmpeg.rs` converts local sidecars.
- `src/embed_subtitles.rs`: local subtitle eligibility and language/stream metadata;
  its bounded FFmpeg runner feeds the final stage of `src/postprocess.rs`.
- `src/info_json.rs`: cleaned video/playlist info JSON values, `.info.json` naming and keep-existing or replacing atomic writes.
- `src/archive.rs`: UTF-8 archive identity cache, bounded file locks and deduplicated transactional append.
- `src/cache.rs`: upstream `Cache` (`--cache-dir`, own `yt-dlp-rs` root, upstream JSON envelope and `min_ver` checks).
- `src/merge.rs`: bounded explicit FFmpeg stream-copy execution.
- `src/audio.rs`, `src/audio/ffmpeg.rs`: pure audio plans and bounded local
  inspection/conversion; `src/audio_download.rs` adapts the audio-only API.
- `src/video.rs`, `src/video/ffmpeg.rs`: video mappings and bounded local remux/recode;
  `src/postprocess.rs` shares ordered processing, retention and publication.
- `src/retry.rs`: shared transient HTTP classification, response checks, bounded backoff and Retry-After delay-seconds/preferred HTTP-date hints.
- `src/output.rs`: template rendering and filename-component sanitization; `src/output/formats.rs` renders format inventories, `src/output/template.rs` handles bounded typed filename/print templates and `src/output/print.rs` parses `--print` values.
- `tests/http.rs`: deterministic local HTTP and binary integration tests.
- `upstream/manifest.json`: upstream revision and component coverage ledger.
- `tools/upstream_report.py`: machine-readable input for an update agent.
- `tools/compare_html5.py`: saved-fixture upstream/Rust metadata comparison.

The initial implementation uses blocking reqwest with Rustls. It runs one URL at
a time; the extractor trait is synchronous. Async/concurrent execution should be
introduced when playlist/fragment workflows need it, with explicit cancellation
and bounded concurrency. The initial info structure intentionally does not
pretend to model every upstream field. `VideoFormat` models probed progressive
sources and optional declared dimensions. Default/`best`/`worst` selection ranks
by the smaller known dimension, with declaration-order ties; it does not reproduce
upstream's full sorter. `ExtractResult` distinguishes a video from a fully extracted HTML5
`PlaylistInfo`; nested playlists (`PlaylistInfo::nested`, YouTube channel tabs) and
deferred entry pages (`PlaylistInfo::pending`) are processed by the CLI's input queue;
a `MULTI_VIDEO` marker makes a playlist upstream's `_type: multi_video`
(`PlaylistInfo::result_type`), which the default `--concat-playlist` joins;
a top-level `url_transparent` result resolving to a playlist takes the outer
fields like upstream, and a playlist entry resolving to a playlist is processed
inline as a nested playlist (the outer playlist's remaining entries are
suspended as a `PlaylistRun` and resumed through the input queue).

## Milestones

1. **Direct HTTP foundation (initial implementation).** Library and binary,
   metadata, templates, streaming downloads, integration tests, source ledger.
2. **Compatibility harness and generic web extraction.** Saved HTML/HTTP fixtures,
   normalized upstream-vs-Rust metadata comparison, HTML5 media extraction,
   format/playlist result types, and an explicit option compatibility inventory.
   HTML5 audio/video with typed progressive formats, explicit source-ID selection,
   a seven-fixture comparison harness, typed HTML5 playlist results for multiple media
   elements, sequential entry downloads, and the option inventory are implemented.
   Bounded automatic resolution ranking and `best`/`worst` selection are implemented;
   Chained numeric/text format filters now constrain existing atoms, pairs and
   fallbacks before upstream FormatSorter ordering (`-S` supported). Parenthesized groups, best/worst shorthand
   aliases, bare-filter selectors and positive indexed ranks are implemented.
   Format listing and unselected extraction expose eligible alternatives before
   selection, including separate-only presentations without a merger.
   Playlist item/range selection, legacy start/end, reverse ordering and original
   index/selected-order metadata are implemented for fully extracted HTML5 lists.
   Selection precedes archive filtering and format selection but does not make
   extraction lazy. Typed filename templates support represented root fields,
   string/integer/fixed-point conversions, width/precision/flags, alternatives and
   missing-value defaults with upstream filename sanitization, restricted/Windows/
   minimal modes, name trimming, typed `-o` templates and `-P` paths; upstream's full template expressions and
   conversions followed, as did upstream's Windows-host `sanitize_path`. `--print` video-stage field lists/templates and
   the title/ID/URL/filename `--get-*` aliases print unsanitized values with
   quiet simulation; `--print-to-file` appends video-stage lines to output-template
   destinations without implying simulation. All upstream print stages are implemented.
   `-a/--batch-file` URL lists follow upstream `get_urls` parsing, and the
   `-w`/`--force-overwrites` modes follow upstream existing-file decisions.
   Upstream FormatSorter ranking and the full selector grammar followed, each
   compared with pinned upstream on saved cases; site playlists, nested
   playlists and URL results have explicit result types.
3. **Resilient transport and media protocols.** Retry policy, range/resume with
   validator checks, HLS/DASH parsing and fragments, format selection, ffmpeg
   integration, and download progress/cancellation. Do not label manifest text
   as a successfully downloaded video.
   Bounded download retries are implemented for selected transient
   statuses, connection/timeouts, and body read/truncation failures, with CLI and
   library configuration (upstream `--retries`/`--fragment-retries` defaults,
   messages, `--retry-sleep` and unavailable-fragment skipping). Interrupted transfers resume within a download call using
   strong ETags, known lengths, and strictly validated ranges, with whole-file
   fallback when validators are unavailable or the server returns a full response.
   Opt-in whole-extraction retries cover transient page/source HTTP failures and
   interrupted HTML reads, rebuilding every entry before output; extraction and
   download retry budgets are independent. Both honor a single Retry-After
   delay-seconds or preferred HTTP-date value with a 60-second cap. Obsolete
   HTTP-date formats remain unsupported.
   Download byte-progress observers and cooperative cancellation are implemented,
   with polling during retry waits and cancellation before publication; the CLI
   emits throttled byte progress on stderr. Blocking requests/reads are not
   interruptible. Native HLS VOD master/media selection, MPEG-TS/AAC/fMP4
   assembly, explicit/implicit media ranges, fragment retries and cancellation are
   implemented. Identity AES-128 CBC keys, sequence/explicit IVs, rotation and
   streaming decryption are implemented with exact local media fixtures.
   External audio groups, metadata and compatible video/audio selection now
   feed native fragment downloads and explicit FFmpeg copy merging, including
   MPEG-TS video and AAC/TS audio. Static DASH templates/timelines/lists/ranges, multi-period merging, `has_drm` marking and native
   representation downloads are implemented; explicit FFmpeg merges separate
   tracks. Dynamic DASH follows upstream: generic live results,
   `--ignore-dynamic-mpd` and FFmpegFD live downloads (native live DASH stays an
   error). Unknown HLS tags are ignored and SAMPLE-AES playlists delegated to
   FFmpegFD like upstream's HlsFD; external HLS video renditions are formats.
   Without ffmpeg, live HLS is recorded natively (a port extension; upstream
   requires ffmpeg). DASH SegmentBase files download as SIDX/Cues byte-range
   fragments (a port extension; upstream reads the whole file). Out of scope
   by decision (2026-10-31): native SAMPLE-AES decryption (FFmpegFD decrypts
   it, as upstream), `--remote-components` (the challenge solver stays the
   bundled, pinned EJS copy) and Python plugins (no Python at runtime). On Unix, SIGINT/SIGTERM end a
   CLI run like upstream's `KeyboardInterrupt`, keeping `.part` files.
   `--continue`/`.part` whole-file resume (also for downloads to be
   postprocessed) and FragmentFD `.ytdl` fragment resume persist across
   invocations.
   Global custom headers and User-Agent/Referer options apply across extraction,
   transfers, fragments, keys, subtitles and validated player acquisition with
   request-specific precedence. Shared in-memory cookies and explicit Netscape
   file persistence now cover the native HTTP clients, and
   `--cookies-from-browser` reads browser cookie stores natively; broader request
   configuration remains open.
   The `InfoExtractor` login lifecycle (credentials, netrc files and commands,
   two-factor codes) is ported, with logins for the extractors listed in
   docs/COMPATIBILITY.md (including Vimeo's login and logged-in web API
   client), and Adobe Pass TV provider authentication is ported.
   Explicit HTTP/HTTPS/SOCKS proxies and direct overrides now route all native
   clients, including verified HTTPS proxy/CONNECT transport and SOCKS4 user
   IDs; `--geo-verification-proxy` routes upstream's geo verification requests.
   Environment proxies follow upstream's `urllib` discovery and `no_proxy`
   bypass (Road to 1.0 step 7); macOS and Windows system proxy settings are
   not read.
4. **YouTube vertical slice.** Player/API request fixtures, client selection,
   progressive and adaptive formats, signature/JavaScript challenge strategy,
   playlist handling, and end-to-end validation. Split request/client machinery
   from site response parsing. Investigate upstream runtime dependencies before
   choosing a JavaScript engine or external runtime boundary.
   The first watch-page slice is implemented: strict video URL recognition,
   initial player response parsing, playable/identity checks, direct muxed
   MP4/WebM formats, itag selection, media probes, and local fixture downloads.
   A saved-response upstream comparison covers the parsed metadata subset.
   This does not establish current live-site reliability. An isolated anonymous
   WEB API fallback is implemented for missing initial responses, with literal
   page client configuration and token-exempt itag 18 only; local request/response
   fixtures cover it. The
   [JavaScript runtime decision](../upstream/notes/youtube-challenge-runtime.md)
   selects an opt-in external Deno process with pinned offline EJS assets;
   query `n` and encrypted-signature solving are integrated behind explicit runtime
   configuration. The full EJS 0.8.0
   assets, saved player vectors, private typed protocol, bounded Deno 2.9.7 runner,
   and a dedicated offline runtime CI matrix are now implemented; see the
   [runner slice](../upstream/notes/youtube-challenge-runner.md). Local saved-player
   `n`/signature comparisons and the prerequisite runtime CI matrix pass on all
   three platforms. Query-n extraction, eligibility/token filtering before solving,
   lazy runtime configuration and local extraction/download fixtures are implemented;
   see the [n extraction slice](../upstream/notes/youtube-n-extraction.md). Strict bounded player acquisition,
   pure URL rewriting, and a saved-player-to-media private pipeline test are now
   implemented; see the [player challenge slice](../upstream/notes/youtube-player-challenges.md).
   Encrypted signature/default/custom-sp and combined-n extraction/download
   fixtures now pass, including an actual offline upstream EJS permutation-path
   comparison; see the [signature slice](../upstream/notes/youtube-signature-extraction.md).
   Direct adaptive audio/video types, bounded pair/fallback selection and explicit
   FFmpeg stream-copy merging are implemented with local fixtures; see the
   [adaptive slice](../upstream/notes/youtube-adaptive-merge.md). DASH manifests
   and `dashy` fragmented HTTPS formats followed. Initial/anonymous WEB HLS
   discovery now feeds native finite VOD plans, with path-n solving shared with
   direct URL challenges. Upstream's challenge
   providers use EJS and a supported JS runtime; Python remains development-only.
   A live comparison pass (2026-10-22) then ported the behavior real use hit:
   chunked HTTPS downloads for merged formats, EU consent/PREF cookies,
   age-gate `web_embedded` fallback, playability reasons, authenticated cookie
   sessions, pre-roll ad waits, InnerTube retries, master-only HLS, live
   streams (FFmpeg, interruptible), mixes, nested channel tabs with lazily
   fetched pages, YouTube Music URLs, and hashtag/feed pages, each with saved
   fixtures, plus clips, `--wait-for-video` and comments. A parity pass
   (2026-10-23) then ported the remaining `YoutubeIE`/`YoutubeTabIE` behavior:
   live chat, MHTML storyboards, post-live and `--live-from-start` downloads,
   every documented `youtube`/`youtubetab` extractor argument including
   configured PO tokens, title/duration/geo/trailer/stretch fallbacks, `next`
   API refetches, multifeed, `--mark-watched`, `--write-pages` for API requests,
   the full `_VALID_URL` with the redirect, feed and notification extractors,
   and shelf/post/music/hashtag/report-history tab entries, each compared with
   pinned upstream on saved fixtures. PO token *provider* plugins are Python
   plugins upstream (none ship by default) and stay out of scope; a configured
   `po_token` is the native equivalent. The remaining documented differences
   are listed under YouTube in `docs/COMPATIBILITY.md` (live progress display,
   eager first tab page, `lang` validation outside `youtube`).
5. **Extractor expansion and parity.** Port shared helpers based on real extractor
   needs. Add sites with deterministic fixtures and a source mapping. Expand
   authentication, cookies, subtitles, archives, and postprocessors explicitly.
   HTML5 subtitle/caption track discovery, listing, regex language/all/exclusion
   selection, format preferences and native HTTP sidecars are implemented.
   Skip-download permits sidecar-only writes without archive records. The supported
   YouTube WEB response subset also exposes manual/automatic captions, translations,
   original aliases, token gating and independent automatic sidecar selection.
   Subtitle conversion to SRT/VTT/ASS/LRC, including native TTML/DFXP preprocessing,
   now follows sidecar acquisition before media downloads, with bounded local
   tools, no-clobber publication and original/intermediate retention.
   Subtitle embedding now follows audio/video processing for MP4/MOV/M4A/WebM/
   Matroska, preserving source container/language policies and sidecar retention.
   HLS/DASH manifest subtitles and other sites' subtitles followed.
   Video/playlist `--write-info-json` sidecars follow upstream clean filtering,
   naming and ordering for the represented metadata subset, and
   `--load-info-json` reprocesses saved dictionaries; `--embed-info-json`
   attaches the info JSON to Matroska outputs like upstream.
   Basic download archives are implemented: cached extractor-key/ID membership,
   filtering before selection and recording only after successful final publication.
   Bounded cooperating-writer locks, append rollback and a pinned offline archive
   reference fixture are covered, as are break-on-existing, per-input breaks,
   force-write and archive disabling, as are early temp-ID skips and
   unresolved playlist entries matched by their own `ie_key`/`id` (a recorded
   YouTube playlist entry is not extracted; `--break-on-existing` stops
   fetching playlist pages). The URL-suitable extractor fallback for entries
   without an `ie_key` and broader archive option parity remain open.
   Audio extraction now supports bounded format mappings, quality plans and local
   FFprobe/FFmpeg copy/conversion for eight target formats, source retention,
   cancellation and archive-after-final-publication. Video remux/recode now follows
   audio extraction in the shared pipeline, with source/intermediate retention and
   latest-completed-stage preservation after failure. Metadata and thumbnail
   embedding (FFmpeg paths only) and thumbnail conversion now follow upstream.
   Cover art that upstream embeds through mutagen (MP4 `covr`, FLAC pictures,
   Ogg `METADATA_BLOCK_PICTURE`) is written natively (`src/cover_art.rs`).
   `--fixup` policies, `--exec`, `--concat-playlist`, chapter splitting and
   removal, and SponsorBlock follow upstream's postprocessor order; see
   docs/COMPATIBILITY.md for each one's covered subset.
6. **Agent-maintained updates.** Upstream delta reports, bounded implementation
   briefs, differential regression fixtures, reviewable PRs, and staged releases.
   Measure extraction coverage and compatibility before enabling unattended merges.

Each milestone may span many changes. A milestone is complete when its documented
behavior passes deterministic tests, compatibility gaps are recorded, and its
upstream mapping is maintained. No whole-project parity claim or completion date
is implied by this initial adaptation.

## Road to 1.0

1.0 is a stability promise, not an extractor count. Upstream-compatible CLI
behavior, the core download pipeline and a tier of actively watched sites must
hold up on real input; the rest of the extractors stay best effort. The main
consumer (Aagedal Media Converter) runs the binary as a subprocess and will
keep doing so, so the CLI and its output are the primary contract; the Rust
library is secondary. Media Converter can change how it calls the CLI.

Work in this order; each step lands with deterministic tests like any other change.

1. **Parser hardening.** `cargo-fuzz` targets for every parser that reads
   untrusted bytes: HLS and DASH manifests, SIDX/MP4 boxes and WebM Cues,
   `js_to_json`, `devalue`, subtitle conversion, cookie files and browser
   cookie databases, `--load-info-json`. Panics found become regression tests;
   the targets run briefly in CI. Download-side panics fail closed with an
   error instead of aborting the run.
   Status (2026-11-01): 15 targets in `fuzz/` (also webpage helpers, dates
   and cover-art embedding), run in CI. Found and fixed: a SQLite schema
   panic and unbounded DASH planning memory (fragment and URL-byte budgets).
   A panic outside an extractor still ends the run with exit 1, like an
   upstream crash. Not fuzzed yet: `jsinterp` (no step limit, like upstream).
2. **Live canary suite.** A scheduled, non-required workflow that extracts and
   downloads a few megabytes from the tier-1 sites (at least the Media Converter
   site list: YouTube, Instagram, TikTok, X, Facebook, Reddit, Vimeo, Twitch,
   Bluesky, SoundCloud, Bandcamp), compares key metadata with the pinned
   upstream, and reports breakage. It never gates merges.
   Status (2026-11-01): `.github/workflows/canary.yml` runs `tools/canary.py`
   daily on `tools/canary_sites.json` (two URLs for most sites, one partial or
   short download each), with the pinned upstream from PyPI for comparison
   and one `canary` issue for broken or diverged sites. Reddit needs login for
   every request (upstream too) and is skipped without a
   `CANARY_COOKIES_REDDIT` secret; `vimeo.com` pages need login, so the Vimeo
   canary uses the embed player. The first local run found four port bugs:
   Instagram posts (`unsupported DASH fractional duration`), the Vimeo player
   (HTTP 401 where upstream succeeds), Twitch clip downloads (`unsupported
   HTML5 media`) and a Bandcamp title keeping a double space upstream collapses.
3. **Interruptible transfers.** Stop/Cancel and SIGINT/SIGTERM interrupt
   blocking connects and reads promptly instead of waiting for the socket timeout.
   Status (2026-11-01): requests send on a helper thread and stream their body
   through a 256 KiB queue whenever an `Interrupt` or signal handler is
   installed, so connects, TLS handshakes, header waits, body reads (single
   file, ranges, fragments with `-N`, the `impersonate` transport) and retry
   or `--sleep-*` waits end within about 100 ms; tests in
   `tests/stalled_transfers.rs`. No throughput change measured on a local 1 GiB
   transfer. Not covered: WebSocket connects, filesystem operations; Windows
   Ctrl+C still ends the process at once.
4. **CLI output contract.** A machine-readable interface rather than frozen
   human-readable lines: `--progress-template` with JSON fields
   (`%(progress)j` and friends), `--print` stage templates (`before_dl`,
   `after_move:filepath`), exit codes, and the stop/cancel signals with
   `--keep-partial-on-stop`. Both use upstream's own options, so a consumer
   works the same against yt-dlp. Document these as stable, test them, and
   move Media Converter off regex-parsing `[download]` lines. The human
   output keeps following upstream but is not a contract.
   Status (2026-11-01): specified in `docs/CLI_CONTRACT.md` with a Media
   Converter migration note, and pinned by `tests/cli_contract.rs`: 35 cases
   compared with saved pinned-upstream output (`tools/capture_cli_contract.py`)
   plus the port's own SIGTERM and `--keep-partial-on-stop` stops. Fixed on
   the way: `tmpfilename` was the final name instead of the `.part` file;
   merged downloads reported the merged file and info instead of each
   format's; `-N` fragment downloads reported no `downloading` statuses;
   progress dict keys and order now follow each upstream downloader;
   `-J` playlist entries lacked `epoch`; a `--keep-partial-on-stop` stop went
   on to the next video and lacked the message's blank line. Not covered:
   Windows (no signals), the full `%(info)j` dict, the human output.
   Media Converter itself is not changed yet.
5. **Benchmarks.** A local-server harness comparing startup, extraction from
   saved fixtures, HLS/DASH throughput with `-N 1/4/8` and peak memory with
   upstream. Then lazy playlist extraction for item selection and a reviewed
   default for `-N`.
   Status (2026-11-01): `tools/benchmark.py` measures both programs on local
   servers (fixture proxy, generated HLS/DASH with and without 50 ms
   per-request latency); results and their limits are in `docs/BENCHMARKS.md`.
   On an M5 Pro the port starts and extracts fixtures 6–18× faster; with
   latency, `-N 4`/`-N 8` cut download time 3.5×/6× for both programs.
   Peak RSS is higher than upstream's whenever all extractor URL patterns
   are compiled (about 80 MiB), and `-N > 1` writes each fragment twice,
   so on localhost it is slower than `-N 1`; both are follow-ups. `-N`
   stays 1 like upstream. Item selection already read paged playlists
   lazily; `--lazy-playlist` now reads each entry (and its page) only when
   the previous one is processed, like upstream, so `--max-downloads` stops
   page fetches (`tests/lazy_playlist.rs`, new site fixture cases). Not
   matched yet: some lazy `-J` entry dicts.
6. **Platform parity.** Impersonation in the Windows release (or a documented
   tier-2 limitation), and test runs on real Windows hardware.
   Status (2026-11-02): the `impersonate` feature (BoringSSL through
   `btls-sys`, built without assembly by MSVC) builds and passes its tests
   on `windows-latest`, and the Windows release now includes it. The whole
   `cargo test` suite runs on GitHub's Windows runners: the tests' shell
   FFmpeg and downloader stand-ins run there through a compiled shim and
   Git for Windows' `sh` (`tests/support/mod.rs`), and signal tests send
   Ctrl+Break. Unix-only with a stated reason: pseudo-terminal, symlink,
   named-pipe and non-UTF-8-path tests. Fixed on the way: Ctrl+C ended the
   process at once (now SIGINT, with Ctrl+Break for SIGTERM, so stops,
   `--keep-partial-on-stop` and interruptible transfers work); `--exec`
   was rejected (now upstream's `cmd.exe` call and quoting, also for
   `--netrc-cmd`); `--ffmpeg-location DIR` missed `ffmpeg.exe`; FFmpeg
   arguments and messages used `\\?\` paths; a killed run's files were
   never recovered. Windows differences are in `docs/CLI_CONTRACT.md`.
   Also run on a Windows 11 ARM64 VM (UTM on Apple Silicon): the whole suite
   passes natively (it found an ARM64-only environment variable the test
   helpers rejected), and the x64 release build with impersonation runs
   under emulation; a native ARM64 build of the feature fails in `btls-sys`
   0.5.6. Not covered: x86-64 Windows hardware outside the hosted runners,
   a console Ctrl+C typed during a download and merge (signal tests send
   Ctrl+Break from a parent), a live `--impersonate` download from a site
   that needs it, Defender scanning `.part` renames, echo-free password
   prompts and named pipes for `--growing-file --live-from-start`.
7. **Core gaps.** Impersonation for media transfers, environment proxy
   discovery and per-extractor proxy policies, the archive URL-suitable
   fallback and native live DASH.
   Status (2026-11-02): surveyed against the pinned upstream first
   (`upstream/notes/core-gaps.md`), then ported one change each. Media
   transfers (whole files, ranges, fragments and keys, also with `-N`,
   subtitles, thumbnails) use the `--impersonate` target, and a format's or
   subtitle's own `impersonate` field applies to HttpFD transfers with
   upstream's warning; impersonated bodies stream through the interruptible
   queue (`tests/impersonate.rs` captures their TLS ClientHello and stops a
   stalled one). Without `--proxy`, both transports discover proxies like
   upstream's `getproxies()` and `select_proxy` (lowercase variables, CGI
   rule, `http`→`https` fallback, `no_proxy`), compared with the pinned
   source's own functions on 21 cases; the only per-extractor policy in
   ported code, `--geo-verification-proxy`, was already ported. Archive
   entries with an `id` but no `ie_key` take the first suitable extractor's
   key in upstream's `_ies` order (`tools/extractor_keys.py`). A live DASH
   format with `--downloader native` reports upstream's error and downloads
   the manifest snapshot (exit 1), as upstream does. Fixed on the way: Rokfin
   stack request notes and skip-line titles, and GoPro/GoToStage sharing
   one `--list-extractors` row. Not covered: recording live DASH at the live
   edge without FFmpeg (upstream has no native recorder either; the port
   does this only for HLS), macOS/Windows system proxy settings, archive
   keys of extractors this port lacks, impersonated FFmpeg or external
   downloader transfers (none upstream), and the native ARM64 Windows
   impersonation build.
8. **Library API.** Shrink the public surface to what consumers use, mark
   growing structs `#[non_exhaustive]` or give them builders, one public error
   type, and `cargo-semver-checks` in CI.
   Status (2026-11-02): surveyed first (`upstream/notes/library-api.md`):
   2321 public paths and 91 `YoutubeDl` methods, nearly all public only
   because the binary was its own crate. The CLI now lives in the library
   as the hidden `cli` module (`src/cli.rs`, moved unsplit; `src/main.rs`
   calls it), so CLI-only items are crate-private: about 140 paths and 35
   `YoutubeDl` methods remain, documented in `docs/LIBRARY.md`. Test-only
   helpers, including the `Extractor` trait, sit in the hidden `__private`
   module. Option, progress, result and info structs and the growing enums
   are `#[non_exhaustive]` (built from `Default` or a constructor), and
   `Error` is the only public error type. CI's `semver` job runs
   `cargo-semver-checks` against the latest release tag through
   `tools/semver_check.py`: a warning before 1.0, a failure afterwards
   unless the major version grew. LIBRARY.md is the crate documentation and
   its example compiles as a doctest. Removed on the way: unused wrappers
   (`fixup_m3u8`, `fixup_m4a`, `select_video`, `resolve_entry`,
   `target_available`, ...) the old surface hid from dead-code checks. Not
   covered: `reqwest::Error` stays inside `Error::Http`/`HttpRetryAfter`
   (reqwest 0.12 is a public dependency), and some `YoutubeOptions` and
   `HttpOptions` settings are still process-wide.
9. **Extractor tiers.** Tier 1 is the canary list; tier 2 is fixture-tested and
   best effort; extractors for dead sites are removed or marked. A short
   supported-sites summary goes at the top of `docs/COMPATIBILITY.md`.
10. **Upstream cadence.** At least one full baseline advance through the
    milestone 6 loop within a week of an upstream release.

Then a release candidate runs against the canaries for a few weeks before 1.0.
Not required: splitting `src/cli.rs`, self-update (`-U` stays an error;
distribution goes through package managers and Media Converter's bundle).
