# Available format inventory

Exact official source target and unchanged global baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
`tools/upstream_report.py` at that target reports no intervening commits, changed
files, affected components or unmapped files. This adds a new bounded behavior
slice; it does not advance the baseline.

Inspected `options.py` list-formats flags (around 945), `YoutubeDL.py` list-only
simulation setup (around 788), `process_video_result` listing before selection
(3050–3090), `render_formats_table` (3982 onward), and `list_formats` (4080).
Also inspected `__init__.py` option normalization: `-F` alone becomes list-only,
but `-F -j` and `-F -s` set ordinary simulation and continue through selection.
`--print-json` is a distinct upstream option and is not implemented here. These
mapped files also cover interactive selection, thumbnails, subtitles, old/table
styles, forced output, sorting and codec metadata outside this slice. There are
no intervening unmapped changes to classify.

The native library separates unselected extraction from selection. Existing
selected APIs delegate to the same whole-extraction retry loop, then select every
entry atomically. New unselected APIs return all eligible alternatives without
requiring a complete default selection or configured merger. CLI list-only mode
uses that path. Combined JSON/simulation modes retain existing native selection;
tables and selected JSON are rendered after the entire input succeeds. This
native atomic timing differs from upstream's early table output on later failure.
Supported selector syntax is preflight-validated for listing arguments; a valid
but unavailable selector does not constrain list-only output.

The pure renderer prints native metadata in declaration order, including exact
known size, dimensions, stream kind, rates, language and rendition notes. Unknown
values remain unknown and direct implicit sources receive no invented selectable
ID. Controls are escaped and cell/caption output bounded. Resource URL/encryption
fields are omitted. This is not an upstream sorter, Unicode terminal-width,
number-format or exact table-rendering compatibility claim.

Local fixtures cover progressive alternatives, direct implicit metadata,
separate-only HLS/DASH inventory without FFmpeg, combined selected JSON, playlist
atomicity, multiple-input continuation, syntax preflight and transient retries.
Extraction retains existing media probes/manifest reads and challenge policy;
there are no media fragment/init/key downloads or merger/output-path side effects.
The saved listing fixture/reference probes separately document metadata and
upstream list-only/combined-mode behavior, with network and tool execution blocked.
See [the compatibility contract](../../docs/COMPATIBILITY.md#available-format-inventory).


Offline reference command:

```sh
python3 tools/compare_listing.py --upstream /path/to/yt-dlp
```

The tool invokes the actual pinned renderer/list method on saved metadata and
runs eight actual option-normalization/video-processing flow probes. Socket,
download and external-process calls are blocked. Direct metadata comparison
preserves declaration order and typed known fields. Processing-flow IDs are
compared as sets because upstream sorts them; explicit pairs avoid default-rank
or FFmpeg-discovery claims. Native tests consume the same saved input/expected
metadata and separately verify the documented native output timing.
