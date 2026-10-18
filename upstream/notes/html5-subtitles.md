# HTML5 subtitle discovery and native sidecar downloads

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against this exact official checkout/target: no
intervening commits, changed files, affected components or unmapped files. This
ports existing behavior without advancing the baseline.

Inspected `extractor/common.py::_parse_html5_media_entries` (3351–3491), especially
track kind/src/language handling; `YoutubeDL.py` subtitle URL sanitation and
extension completion/selection (2899–2919), listing (3052–3062), process_subtitles
(3157–3220), simulation (3373–3376), sidecar writes (3389, 4447–4505), skip-download
(3460–3464), final-success flag (3674) and archive aggregation (3143–3146);
`utils/_utils.py` sanitize_url, determine_ext/KNOWN_EXTENSIONS, subtitles_filename
(1322) and orderedSet_from_options (5329–5360); `options.py` list callback (248),
subtitle flags (973–1008) and skip-download; and relevant subtitle/HTML5 references
in `test/test_YoutubeDL.py` and `test/test_InfoExtractor.py`. These mapped files
also implement automatic captions, regex selection, site/manifest acquisition,
conversion/embedding, full filename controls and unrelated downloader behavior
outside this slice. No intervening unmapped changes require classification.

Native extraction groups direct-child tracks by the actual optional language
value, preserving source order and duplicates per language. It fetches no subtitle
payload during extraction. URL sanitation is source processing, not comparison
normalization: protocol-relative captions become HTTP and the httpss typo is
repaired before extension completion. URL/query/fragment/trailing-slash quirks,
kind case sensitivity and empty/missing language distinctions are retained.
Malformed/nested DOM ownership and unsupported schemes have explicit native bounds.

Pure SubtitleOptions selection handles defaults, literal/all/exclusion languages
and ordered format preferences. Rendering lives separately in output/subtitles.rs;
transport uses a typed HTTP resource policy shared with media, retaining retries,
referrers, cancellation and publication while bypassing media MIME inspection for
raw sidecar bytes and rejecting unsupported compression. CLI sidecars precede
media, and skip-download writes no media archive record. Complete earlier sidecars
survive later failures. Existing destinations and sanitized/case-folded collisions
are rejected/reused through documented native rules; no full source failure timing
or unsafe filename parity is claimed. Unknown languages remain metadata, never
invented und tags; unsupported default/writing cases return explicit errors.

Seven HTML, 29 selection, five filename, six option and three source-only cases
use actual pinned methods with network/download/process calls blocked. Ordered
arrays only preserve dictionary order and distinguish None from string tags; no
URL, language, filename or order difference is ignored. Options/list/skip probes
and write-flag source inspection establish control flow separately from native
local HTTP byte/publishing tests. Those tests cover read-only laziness, selection,
retry/referrer/identity bytes, reuse/no-clobber, failures and continuation,
archive timing, path aliases/collisions, dangling symlinks and cancellation before
request/publication. Full project parity remains open.

```sh
python3 tools/compare_subtitles.py --upstream /path/to/yt-dlp
```

See [the compatibility contract](../../docs/COMPATIBILITY.md#html5-subtitles-and-sidecars)
and [fixture source references](../../tests/fixtures/subtitles/README.md).
