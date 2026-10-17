# Supported compatibility subset

This inventory describes tested behavior, not general yt-dlp compatibility.
All extraction and downloads run in Rust. Python is development tooling only.

## CLI

| Option | Current behavior |
| --- | --- |
| Positional URLs | One or more HTTP(S) URLs, processed sequentially; continue after errors, return failure if any URL fails |
| `-j`, `--dump-json` | One supported `VideoInfo` JSON object per successful input; extract without downloading |
| `-s`, `--simulate` | Extract metadata, write diagnostics to stderr, create no media files |
| `-f`, `--format` | Literal zero-based HTML5 source ID; required for multiple sources; no expressions, ranking, merging, or direct-URL format IDs |
| `-o`, `--output` | `title`, `id`, `ext`, `extractor` string fields and literal `%%`; no overwrite |
| `-h`, `--help`, `-V`, `--version` | Clap help and crate version |
| All other flags | Rejected; aliases such as `--skip-download` and `--print` are not implemented |

Simulation and JSON still make network requests. This inventory does not claim
upstream equivalence for every option interaction or filename rule.

## HTML5 extraction

The generic fallback dispatches a single response to direct-media inspection or
HTML5 parsing. Site extractors registered through `Extractor` run first. The
standalone `DirectHttp` extractor remains available for direct-only callers.

For `text/html`, accept exactly one `video`/`audio` element with nonempty
`src` and/or direct child `source[src]` declarations. Resolve URLs against the
final page URL and first `base[href]`; decode HTML entities using an HTML5 parser. Prefer Open Graph
title, then document title, then page filename stem; collapse title whitespace.
Append `-1` to the ID and ` (1)` to the title, as upstream HTML5MediaEmbedIE does.
Metadata identifies `html5` / `HTML5MediaEmbed` with `direct: false` and the final
page URL as `webpage_url`. Every source is probed with the page referrer and must
serve supported direct HTTP media. Redirected source URLs and response-derived
extensions/filesizes are recorded in typed `VideoFormat` entries. HTML5 JSON adds
`formats` and the selected `format_id`, including for a single source. IDs are
literal zero-based strings assigned to nonempty declarations: the media element's
`src` first, then direct child sources in document order. Duplicate URLs remain
separate declarations. These IDs are Rust-specific; upstream's label/title IDs,
format sorting, and fallback expressions are not implemented.

A single source is selected automatically. Multiple sources require `-f ID`;
omitting it or requesting an unavailable ID returns an error listing available
IDs. `best`, `worst`, filters, merge and fallback expressions are errors. Direct
HTTP results expose no format IDs, so an explicit `-f` on them fails. The library's
`extract_info_with_format` performs the same selection; standalone extractor
callers must call `VideoInfo::select_format` to resolve candidates before download.
Selection updates top-level `url`, `ext`, `filesize`, and `format_id` together.
The selected source is downloaded with the page referrer. If the download
response changes to HTML or a manifest, fail without publishing it as media.

HTML bodies are limited to 2 MiB, including responses without Content-Length.
Only uncompressed UTF-8/ASCII HTML is supported. Other encodings are rejected
when declared in HTTP Content-Type; invalid UTF-8 is rejected. HTML meta charset
sniffing, compressed responses, XHTML, JavaScript, nonstandard lazy-loading
attributes, subtitles, posters, durations, codecs, dimensions, authentication,
and general embedded-player extraction are not implemented. Multiple media
elements, HLS/DASH URLs/type hints, and non-HTTP sources fail explicitly, even
when another supported source was explicitly selected. Unselected source probe
failures also fail extraction rather than hiding unsupported alternatives.
MIME checks identify supported responses; they do not verify media containers.

## Source and differential fixtures

This slice uses the existing baseline, **897341241f2bfb2439fe0367f4c7c9ccd29a3828**:

- `yt_dlp/extractor/generic.py`: response dispatch and HTML embed orchestration.
- `yt_dlp/extractor/genericembeds.py`, `HTML5MediaEmbedIE`: single-entry metadata.
- `yt_dlp/extractor/common.py`, `_parse_html5_media_entries`: media/source parsing.
- `test/test_InfoExtractor.py`, `test_parse_html5_media_entries`: inline video,
  nested sources, multiple formats, relative URLs, and manifest examples reviewed.

The full upstream helpers support many behaviors outside this Rust subset.
This is a new port slice from the pinned revision, not an upstream update; the
global baseline has not advanced.

Run the optional differential harness against an official checkout at that SHA:

```sh
cargo build --locked
python3 tools/compare_html5.py --upstream .upstream/yt-dlp --binary target/debug/yt-dlp-rs
```

The harness serves `tests/fixtures/html5/{video,audio,source,formats}.html` and fixed media
bytes on localhost. It invokes upstream with only Generic and HTML5MediaEmbed
enabled and compares `id`, `title`, `ext`, `url`, `webpage_url`, `extractor`, and
`extractor_key` exactly. The sole URL normalization removes the fixture server's
exact origin. Query strings, fragments, paths, titles, and suffixes are preserved.
Other fields (including upstream timestamps and fields outside VideoInfo) are
excluded by this explicit allowlist. No volatile-value masking is applied within
the compared fields. Rust-specific boundary behavior is covered by `tests/http.rs`.

The multi-source fixture has explicit upstream labels `0` and `1` so each
implementation can request the same source without equating their general ID
assignment rules. Both selections are compared. The format list, probe sizes,
and Rust-specific IDs are covered by deterministic Rust tests rather than the
upstream metadata allowlist. The multi-source fixture uses page-relative URLs;
`base[href]` resolution remains a separately tested Rust behavior because the
pinned upstream HTML5 embed path does not apply it in this fixture.

The comparison needs upstream source and is optional; required tests need no
upstream checkout, Python runtime integration, or live sites. Four fixtures do
not establish broad extractor parity. Playlist result types remain a prerequisite
for supporting multiple media elements.
