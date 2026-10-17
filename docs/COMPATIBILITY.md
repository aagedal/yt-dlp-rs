# Supported compatibility subset

This inventory describes tested behavior, not general yt-dlp compatibility.
All extraction and downloads run in Rust. Python is development tooling only.

## CLI

| Option | Current behavior |
| --- | --- |
| Positional URLs | One or more HTTP(S) URLs, processed sequentially; continue after errors, return failure if any URL fails |
| `-j`, `--dump-json` | One supported `VideoInfo` JSON object per successful input; extract without downloading |
| `-s`, `--simulate` | Extract metadata, write diagnostics to stderr, create no media files |
| `-o`, `--output` | `title`, `id`, `ext`, `extractor` string fields and literal `%%`; no overwrite |
| `-h`, `--help`, `-V`, `--version` | Clap help and crate version |
| All other flags | Rejected; aliases such as `--skip-download` and `--print` are not implemented |

Simulation and JSON still make network requests. This inventory does not claim
upstream equivalence for every option interaction or filename rule.

## HTML5 extraction

The generic fallback dispatches a single response to direct-media inspection or
HTML5 parsing. Site extractors registered through `Extractor` run first. The
standalone `DirectHttp` extractor remains available for direct-only callers.

For `text/html`, accept exactly one `video`/`audio` element with one nonempty
`src` or direct child `source[src]`. Resolve URLs against the final page URL and
first `base[href]`; decode HTML entities using an HTML5 parser. Prefer Open Graph
title, then document title, then page filename stem; collapse title whitespace.
Append `-1` to the ID and ` (1)` to the title, as upstream HTML5MediaEmbedIE does.
Metadata identifies `html5` / `HTML5MediaEmbed` with `direct: false` and the final
page URL as `webpage_url`. The selected media is probed and downloaded with the
page referrer. If the download response changes to HTML or a manifest, fail
without publishing it as media.

HTML bodies are limited to 2 MiB, including responses without Content-Length.
Only uncompressed UTF-8/ASCII HTML is supported. Other encodings are rejected
when declared in HTTP Content-Type; invalid UTF-8 is rejected. HTML meta charset
sniffing, compressed responses, XHTML, JavaScript, nonstandard lazy-loading
attributes, subtitles, posters, durations, codecs, dimensions, authentication,
and general embedded-player extraction are not implemented. Multiple sources
or media elements, HLS/DASH URLs/type hints, and non-HTTP sources fail explicitly.
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

The harness serves `tests/fixtures/html5/{video,audio,source}.html` and fixed media
bytes on localhost. It invokes upstream with only Generic and HTML5MediaEmbed
enabled and compares `id`, `title`, `ext`, `url`, `webpage_url`, `extractor`, and
`extractor_key` exactly. The sole URL normalization removes the fixture server's
exact origin. Query strings, fragments, paths, titles, and suffixes are preserved.
Other fields (including upstream timestamps and fields outside VideoInfo) are
excluded by this explicit allowlist. No volatile-value masking is applied within
the compared fields. Rust-specific boundary behavior is covered by `tests/http.rs`.

The comparison needs upstream source and is optional; required tests need no
upstream checkout, Python runtime integration, or live sites. Three fixtures do
not establish broad extractor parity. Format/playlist result types remain a
prerequisite for supporting multiple sources/elements.
