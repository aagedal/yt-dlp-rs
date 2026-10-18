# Grouped format selection

Exact official source target and unchanged global baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
`tools/upstream_report.py` with that target reports no intervening commits,
changed files, affected components or unmapped files. This is another behavior
slice from the pinned source, not a baseline advancement.

Inspected `YoutubeDL.py::build_format_selector`, particularly parsing/group and
merge/choice construction around 2338–2453, selector evaluation around 2535–2641,
the best/worst alias regex around 2590 and the final filter wrapper around 2641.
Relevant `test/test_YoutubeDL.py` format selection/filter tests cover the source
areas. These mapped files also implement comma/all selections, stream
deduplication, starred/indexed/extension selectors and full sorting outside this
slice. There are no intervening unmapped changes to classify.

The native parser now returns an explicit bounded expression tree. Groups,
ordered choices, two-input merges and filtered atoms remain separate; it does
not distribute grouped fallbacks into a list of possible merges. Group filters
narrow the input context before recursive evaluation, matching the upstream
wrapper's scope. Best/worst short forms canonicalize to existing native selectors;
bare predicates imply best. Syntax limits and unsupported modifiers fail before
selection, including unreachable later branches with more than two streams.

`VideoInfo` evaluates the tree before mutating selected metadata. Existing native
ranking, compatible rendition-group search, explicit two-ID group override,
single/pair output, merge policy and atomic playlist selection remain in force.
Literal-ID provenance survives grouped choices. Implicit direct results apply
group filters only to available metadata without inventing a format ID.

`tests/fixtures/selectors/expressions.json` is an original synthetic presentation
with codec and native kind annotations and an explicitly ordered format list.
`tools/compare_selectors.py` invokes actual pinned `build_format_selector` with
network blocked and explicit context flags, then compares IDs and request order.
Native selection tests consume the same expected cases. The saved order agrees
with native ranking for these cases; no upstream sorting or normalization hides
a difference. Unavailable and malformed late-fallback cases have explicit
expectations. This does not claim general sorter, incomplete-format fallback,
stream deduplication, group-policy or container parity.

Local CLI/HTTP tests separately verify grouped filters before ranking, aliases,
bare-filter downloads, HLS paired plans, direct metadata and playlist failure
atomicity. Native tests cover fallback scope and literal group overrides. Bounds
are 8 KiB, 16 global fallback choices, 16 group levels, 128 nodes and at most two
streams per possible branch, with 16 predicates per atom/group. See
[the compatibility contract](../../docs/COMPATIBILITY.md#grouped-selector-expressions).
