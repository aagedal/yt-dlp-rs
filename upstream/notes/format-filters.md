# Typed format predicates

Exact source target and unchanged global baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19), from the official
yt-dlp checkout. Ran `tools/upstream_report.py` with that exact target: no
intervening commits, changed files, affected components or unmapped files.
This is an additional behavior slice at the pinned baseline, not a catch-up claim.

Inspected `YoutubeDL.py::_build_format_filter` (2203–2267), filter application
in `build_format_selector` (around 2641),
`test/test_YoutubeDL.py::test_format_filtering` (395 onward), and
`utils/_utils.py::parse_filesize` (1774 onward). These mapped files also cover
regexes, arbitrary metadata keys, nested selector groups, multistream selections
and full sorting outside this slice. The source mapping now explicitly includes
the helper's utils file; there are no unmapped intervening changes to classify.

The native selector parser separates syntax/predicates from `VideoInfo` ranking
and selection. It adds chained numeric/text predicates to existing single atoms,
two-stream pairs and ordered fallbacks, preserving complete-expression validation
and atomic result/playlist updates. Filters run before kind fallback and ranking.
Direct results without alternative formats evaluate only known top-level metadata;
missing IDs, rates and rendition text remain unknown. Transport is unchanged.

Numeric predicates cover width, height, filesize, tbr and abr; text predicates
cover extension, ID, language, note and audio group. This predicate slice left unknown fields, regexes, filter-only selectors, groups
and comma/all selections as explicit errors. The subsequent
[grouped-selector slice](selector-groups.md) adds groups and bare predicates;
unknown fields, regexes and comma/all selections remain errors.
Numeric-first unquoted text equality, unknown inclusion, text negation,
quote/backslash handling and decimal/binary
unit conventions are recorded in
[the compatibility contract](../../docs/COMPATIBILITY.md#format-predicates).
Limits are 8 KiB per expression, 16 branches, two atoms per branch and 16 filters
per atom. These bounds and strict supported metadata types are native policy.

Integral comparisons use checked rational thresholds and exact u64 byte counts;
unit suffixes preserve pinned upstream case conventions and ties-to-even integer
rounding. This avoids precision loss for native large files. Upstream converts
numeric literals through floating point, so arbitrary values above 2^53 and
decimal rounding edge cases are not claimed as general floating-point parity.
Nonfinite/negative rate metadata is unknown; finite zero is a known value.

`tests/fixtures/selectors/filters.json` contains original synthetic format
dictionaries and expected matching IDs. Rust predicate tests and
`tools/compare_filters.py` consume the same cases. The latter invokes actual
pinned `_build_format_filter` with network blocked, without normalizing predicate
text, matching IDs or order. It verifies predicates only; ranking, complete
selector grammar, regexes and unsupported fields are excluded. Native exact
integer/overflow behavior has separate tests.

Local HTTP/CLI regressions verify JSON/simulation, selected media bytes, missing
metadata, quoted selector delimiters, filtered HLS pairs, direct metadata and
playlist selection failures without partial output. Existing native ranking,
group matching, publication and explicit merger policy remain in force. Predicate
coverage does not establish full upstream sorting or selector compatibility.
