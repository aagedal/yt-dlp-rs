# Working on yt-dlp-rs

This is an incremental native Rust port, currently limited to direct HTTP media
and single-source HTML5 audio/video pages.
Read README.md, docs/ROADMAP.md, and upstream/manifest.json before expanding scope.
Python is allowed for development/reference tooling, never as a runtime extraction
or download fallback. Do not claim yt-dlp compatibility beyond tested behavior.

## Implementation

- Keep extraction, orchestration, output rendering, and transport separate.
- Add site extractors through `Extractor` and register them before `DirectHttp`.
- Add deterministic local HTTP/fixture tests for each newly supported behavior.
- Preserve errors for unsupported behavior rather than guessing success.
- Do not add live-site requests to the required test suite.
- Run `cargo fmt --all -- --check`, `cargo clippy --locked --all-targets -- -D warnings`,
  `cargo test --locked`, and `python3 -m unittest discover -s tools -p 'test_*.py' -v`.

## Upstream update tasks

1. Run `tools/upstream_report.py` against an official upstream checkout containing
   the pinned baseline and the target commit. Save the exact target SHA in the brief.
2. Inspect both affected components and unmapped files. A mapped file may contain
   features outside our current supported subset. Explain applicability explicitly.
3. Read the upstream diff and relevant tests. Port one coherent behavior change
   at a time, recording the source commit and a regression fixture/test.
4. For site behavior, compare normalized metadata with upstream using saved fixtures
   where possible. Ignore volatile URLs/timestamps only through documented rules.
5. Update scope and source mappings for new components. Advance the global baseline
   only after every intervening change has been ported, explicitly deferred, or
   classified as outside current scope in a committed update note.
6. Run all checks and present changes for review. A successful report alone does
   not mean the port has caught up. No automatic merge/release is configured.
