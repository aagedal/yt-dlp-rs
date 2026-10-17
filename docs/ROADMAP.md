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

- `src/main.rs`: strict CLI subset, per-URL errors, NDJSON metadata output.
- `src/lib.rs`: ordered extractor registry and download orchestration.
- `src/info.rs`: typed subset of upstream video info fields.
- `src/extractor/`: independent extractors, generic fallback last.
- `src/extractor/html5.rs`: generic response dispatch and progressive HTML5 source parsing;
  direct-media response inspection is shared with `DirectHttp` to avoid duplicate probes.
- `src/download.rs`: streaming HTTP transfer and no-clobber publication.
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
sources; introduce typed playlist/result variants before adding those behaviors.

## Milestones

1. **Direct HTTP foundation (initial implementation).** Library and binary,
   metadata, templates, streaming downloads, integration tests, source ledger.
2. **Compatibility harness and generic web extraction.** Saved HTML/HTTP fixtures,
   normalized upstream-vs-Rust metadata comparison, HTML5 media extraction,
   format/playlist result types, and an explicit option compatibility inventory.
   HTML5 audio/video with typed progressive formats, explicit source-ID selection,
   a four-fixture comparison harness, and the option inventory are implemented.
   Multiple media elements are rejected; typed playlist results, automatic format
   ranking, and general selector expressions remain open.
3. **Resilient transport and media protocols.** Retry policy, range/resume with
   validator checks, HLS/DASH parsing and fragments, format selection, ffmpeg
   integration, and download progress/cancellation. Do not label manifest text
   as a successfully downloaded video.
4. **YouTube vertical slice.** Player/API request fixtures, client selection,
   progressive and adaptive formats, signature/JavaScript challenge strategy,
   playlist handling, and end-to-end validation. Split request/client machinery
   from site response parsing. Investigate upstream runtime dependencies before
   choosing a JavaScript engine or external runtime boundary.
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
