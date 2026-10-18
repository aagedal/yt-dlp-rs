# Subtitle conversion and native TTML preprocessing

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against this official checkout and target:
no intervening commits, changed files, affected components or unmapped files.
The baseline remains unchanged; no general catch-up claim is made.

Inspected `postprocessor/ffmpeg.py:939–1018` and shared execution,
`utils/_utils.py:3404–3599`, `options.py:1790–1796`,
`__init__.py:251–259,644–650`, `YoutubeDL.py` sidecar/before_dl/skip_download
ordering, `test/test_utils.py:1611–1725` and relevant subtitle/postprocessor tests.
The mapped files also implement other postprocessors, generic XML/timing helpers,
metadata/timestamps and full orchestration outside this slice. No intervening
unmapped changes exist.

The target grammar, aliases, none disabling, JSON/same-format skips and conversion
before media downloads follow the source. Conversion alone does not select
subtitles for writing. Skip-download still permits sidecar conversion. Native
orchestration preflights names and reserved cookie/archive paths, stages local
files privately and publishes without overwriting. Public source removal follows
successful conversion; keep retains source and native-SRT intermediates. Failed
FFmpeg conversion preserves the original and available completed native SRT.
Metadata modes stay lazy and required tests make no live-site requests.

Native dfxp2srt ports the source timing/style/text behavior and supported encoding
paths. Saved tests preserve paragraph numbering gaps, invalid-time skips, empty
output, namespace rewrites, forward/inherited styles and observable cross-cue
state. Source timestamp/HTML-like text behavior is not a full TTML interpretation.
Bounds cover bytes, decoded/output size, XML nodes/depth, cue count and style
resolution. DTDs/custom entities and unresolved/cyclic parent styles error; the
source can loop indefinitely on unresolved parents. Numeric timing syntax is
ASCII with a finite integer-millisecond bound. Broader encoding/XML/TTML standards
support remains open rather than guessed.

Actual pinned parse_options, FFmpegSubtitlesConvertorPP.run and dfxp2srt are
invoked in the offline reference. Fifteen byte-preserving inputs compare exact
SRT, including four copied upstream vectors and UTF-16. Forty-two plans compare
exact captured options/warnings/messages/paths/deletions and move bookkeeping.
One JSON3 source-only case and five invalid targets are recorded. Virtual FFmpeg
output tests metadata propagation only; runtime media quality is separate.
The worker forbids network/process and nonvirtual filesystem mutation. Python
is development/reference tooling only, never a runtime fallback.

The bounded local runner uses fixed input/output formats and the first subtitle
stream, explicit file-only protocols, source faststart flags, 60-second overall
version/conversion deadlines, bounded pipes and monitored 16 MiB output growth.
This fixed stream map is a native policy for single-stream sidecars; upstream
supplies output format and FFmpeg auto-selection. Active unsupported formats fail.
Literal JSON skips; JSON3 is not included in the source's JSON special branch.

Portable process tests cover strict arguments, output validation, hanging/flooding
children, deadlines and cancellation. Fourteen local HTTP/library pipeline tests
cover writing flags/aliases, skip-download, native/lazy paths, retention,
intermediate recovery, no-clobber races, source changes and reserved state aliases.
Configured real tools verify all four target formats, cues/timing and representable
styles. Existing audio/video chains remain independently covered.

Native file publication/error timing, source identity checks, strict no-overwrite
and move bookkeeping differ from full upstream orchestration and are explicit.
Arbitrary postprocessor chains/arguments, embedding, timestamp preservation,
additional encodings/formats and full TTML standards remain open.

```sh
python3 tools/compare_subtitle_conversion.py --upstream /path/to/yt-dlp
```

See [compatibility details](../../docs/COMPATIBILITY.md#subtitle-conversion) and
[fixture provenance](../../tests/fixtures/subtitle_conversion/README.md).
