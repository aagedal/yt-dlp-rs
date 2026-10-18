# Playlist item selection

Exact official source target and unchanged global baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against this official checkout and exact target:
no intervening commits, changed files, affected components or unmapped files.
This ports an existing behavior without advancing the baseline.

Inspected `utils/_utils.py` PlaylistEntries (2418–2550), parse/get_requested_items,
one-based inclusive slicing and frange (2209); `YoutubeDL.py` orderedSet,
playlistreverse and index/autonumber metadata (2084–2158); `options.py` selection
flags (682–701); `__init__.py` legacy bound validation (221) and item syntax
preflight (451); and `test/test_YoutubeDL.py::test_playlist_items_selection`
(992–1100), including legacy ranges, duplicate suppression, reverse, negative
indices, infinity and out-of-range step phase. These mapped files also implement
lazy/paged/incomplete lists, filters/break limits, randomization, unresolved results
and full metadata/options outside this slice. No intervening unmapped changes
need classification.

Native PlaylistSelection separates parsing/index calculation from extraction and
format selection. The CLI applies it to fully extracted HTML5 entry vectors before
archive filtering and atomic remaining-entry format selection. Single videos are
unaffected. Entries keep original IDs/titles and receive original index/count and
selected order/count metadata. Archive skips do not renumber selection metadata.
The library exposes explicit selection; extraction methods do not implicitly apply
CLI item options. Format syntax is preflight-validated even with empty selections.

Index arithmetic uses bounded signed components and computes the first in-range
position without walking billions of skipped indices, preserving the original
step phase. The pinned `infinite` endpoint converts to an omitted bound, unlike
`inf`; their negative-step difference is retained and covered explicitly. Legacy
start must be positive, distinct from valid zero in item specs. Specs over 16 KiB/1024 segments or signed 64-bit numeric bounds fail
explicitly. Unicode numeric syntax, lazy/paged/nested/site lists, full playlist
metadata and template syntax remain outside the native contract. Ordinary
extraction still probes all entries; excluding an entry cannot hide its extraction
failure. Source reference comparison covers materialized lists, not lazy request
counts or site reliability.

Shared fixtures cover 79 selections and 24 invalid CLI cases using the actual
pinned source option parser and index iterator, ordered
first-occurrence deduplication and reversal. Two source-only Unicode numeric
cases explicitly document acceptance outside the native ASCII grammar. Native
unit tests also compare dense signed ranges with an independent naive iterator
and exercise signed-64-bit extremes without huge prefix iteration. Native local HTTP tests verify selected
bytes/order, excluded format availability versus retained atomic failure, archive
interaction, read-only modes, metadata and validation before HTTP. Network/download/
process execution is blocked in saved-reference probes.

```sh
python3 tools/compare_playlist.py --upstream /path/to/yt-dlp
```

See [the compatibility contract](../../docs/COMPATIBILITY.md#playlist-item-selection).
