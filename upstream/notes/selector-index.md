# Indexed selector ranks

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
`tools/upstream_report.py` at this target reports zero intervening commits,
changed files, affected components and unmapped files. This adds a bounded slice
from the pinned source; it does not advance the global baseline.

Inspected `YoutubeDL.py::build_format_selector` around 2590–2640: the best/worst
alias regex captures positive `.N`, defaults to rank one, applies filters and
direction, then accesses `matches[format_idx - 1]`. Also inspected
`utils/_utils.py::int_or_none` (2021) and `LazyList.__getitem__` (around 2277),
plus the upstream format selection/filter tests in `test/test_YoutubeDL.py`.
The mapped files also implement starred selectors, extension selection, sorting,
multiselection and other behavior outside this slice. There are no intervening
unmapped changes to classify.

Native atoms retain an optional explicit rank after alias canonicalization.
Filtering and native kind/ranking run before a checked one-based lookup; a
missing rank produces no candidate and permits scoped fallback. Explicitly indexed
atoms expose only that candidate to existing pair/group compatibility validation.
Unindexed atoms retain native compatible-pair search. Indexing is not a literal
ID override. Direct metadata has one represented source and remains ID-less.
Dotted names that do not match the positive rank regex retain literal semantics.

The existing shared expression fixture/tool now checks indexed forms through
actual pinned `build_format_selector` using an explicitly ordered presentation.
Native selection consumes the same expected IDs. This verifies the supported
rank lookup, alias, predicate/group scope, order and unavailable cases; it does
not establish general ranking, incomplete-format fallback or rendition-group
policy parity. No upstream sorter or network requests are involved.

Reference investigation found that indexes beyond `sys.maxsize` raise an iterator
ValueError rather than returning an unavailable candidate. Extremely long Python
integers can instead trigger `int_or_none`'s default-one behavior. Native parsing
rejects indexes above the platform signed index maximum before any branch is
selected; neither behavior is silently normalized into success. This explicit
native resource bound also applies to unreachable later branches. Oversized
indexes are excluded from matching-ID parity claims and tested independently.

Local CLI/HTTP regressions verify rank-after-filter selection, exact downloaded
bytes, HLS audio/default ordering and pairs, unavailable fallbacks, direct metadata
and atomic playlist failures. Parser/native unit tests cover aliases, dotted IDs,
rank ties, group compatibility and numeric limits. See
[the compatibility contract](../../docs/COMPATIBILITY.md#indexed-selector-ranks).
