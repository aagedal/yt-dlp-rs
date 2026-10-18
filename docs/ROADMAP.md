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

- `src/main.rs`: strict CLI subset, per-URL errors, per-video NDJSON metadata output, sequential HTML5 playlist processing.
- `src/lib.rs`: ordered extractor registry and download orchestration.
- `src/info.rs`: typed subset of upstream video info fields and video/HTML5 playlist results.
- `src/extractor/`: independent extractors, generic fallback last.
- `src/extractor/html5.rs`: generic response dispatch and progressive HTML5 source parsing;
  direct-media response inspection is shared with `DirectHttp` to avoid duplicate probes.
- `src/extractor/youtube.rs`: narrow watch-page/player response extraction,
  direct standalone/separate formats with opt-in offline signature/n solving.
- `src/extractor/youtube/client.rs`: isolated anonymous WEB player API request
  fallback and explicit token-exempt itag 18 policy.
- `src/download.rs`: streaming HTTP transfer, byte-progress/cancellation checkpoints, and no-clobber publication.
- `src/hls.rs` and `src/extractor/hls.rs`: pure VOD HLS parsing and generic/HTML5 manifest acquisition.
- `src/dash.rs` and `src/extractor/dash.rs`: bounded static single-period MPD parsing and generic/HTML5 acquisition.
- `src/fragment.rs`: ordered native fragment/range transport and atomic publication.
- `src/merge.rs`: bounded explicit FFmpeg stream-copy execution.
- `src/retry.rs`: shared transient HTTP classification, response checks, bounded backoff and Retry-After delay-seconds/preferred HTTP-date hints.
- `src/output.rs`: template rendering and filename-component sanitization.
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
`PlaylistInfo`; unresolved URL results, nested playlists, and site playlists
remain outside the current result contract.

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
   fallbacks before native ranking. Full upstream ranking and general selector
   expressions remain open. Site
   playlists and nested/unresolved results require further explicit result types.
3. **Resilient transport and media protocols.** Retry policy, range/resume with
   validator checks, HLS/DASH parsing and fragments, format selection, ffmpeg
   integration, and download progress/cancellation. Do not label manifest text
   as a successfully downloaded video.
   Bounded download retries are implemented for selected transient
   statuses, connection/timeouts, and body read/truncation failures, with CLI and
   library configuration. Interrupted transfers resume within a download call using
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
   MPEG-TS video and AAC/TS audio. Static single-period DASH templates/timelines/lists/ranges and native
   representation downloads are implemented; explicit FFmpeg merges separate
   tracks. Dynamic/multi-period/SIDX DASH, SAMPLE-AES/DRM and live HLS,
   external video/subtitle renditions, discontinuities,
   persistent resume across invocations, extraction cancellation, and CLI signal handling remain open.
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
   [adaptive slice](../upstream/notes/youtube-adaptive-merge.md). Fragmented direct
   adaptive delivery and DASH discovery remain open. Initial/anonymous WEB HLS
   discovery now feeds native finite VOD plans, with path-n solving shared with
   direct URL challenges. Upstream's challenge
   providers use EJS and a supported JS runtime; Python remains development-only.
5. **Extractor expansion and parity.** Port shared helpers based on real extractor
   needs. Add sites with deterministic fixtures and a source mapping. Expand
   authentication, cookies, subtitles, archives, and postprocessors explicitly.
6. **Agent-maintained updates.** Upstream delta reports, bounded implementation
   briefs, differential regression fixtures, reviewable PRs, and staged releases.
   Measure extraction coverage and compatibility before enabling unattended merges.

Each milestone may span many changes. A milestone is complete when its documented
behavior passes deterministic tests, compatibility gaps are recorded, and its
upstream mapping is maintained. No whole-project parity claim or completion date
is implied by this initial adaptation.
