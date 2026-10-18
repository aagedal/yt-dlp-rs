# Bounded extract-audio processing

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against the official checkout and this exact target:
no intervening commits, changed files, affected components or unmapped files.
This ports existing behavior without advancing the baseline.

Inspected `postprocessor/ffmpeg.py:50–80,432–535`, audio option validation and
selector defaults in `__init__.py:258–266,594–596,656–662`,
`options.py:1617–1634,1667–1671`, filename extension helpers in
`utils/_utils.py`, and the postprocessor tests. These files also implement
unrelated postprocessors, discovery, fixups, metadata, timestamps and general
orchestration outside this slice. No intervening unmapped changes exist.

The pure model implements ordered mappings, common-audio best skips, first-audio
codec copy/fallback decisions, eight targets, filename changes and encoder quality
curves. The CLI selects bestaudio/best for -x without -k or an explicit selector.
Existing native ranking still applies; this is not upstream sorter parity.
The saved reference invokes actual parse_options and FFmpegExtractAudioPP.run,
substituting fixed probes/captured FFmpeg calls and virtual renames only.
73 supported plans, five source-only cases and four source rejections are retained.
Numeric quality projections and source AAC container quirks are documented in
[fixture provenance](../../tests/fixtures/audio/README.md).

Native deliberately rejects malformed/nonfinite/unbounded quality and selects
ALAC explicitly instead of reproducing source option loss that can encode AAC.
No FDK encoder selection is implemented. WAV uses the muxer's default encoder.
Required tests use portable Rust process fixtures; optional configured real tools
verify all eight target encoders and end-to-end MP3/WAV/ALAC outputs. Required
checks have no live-site dependency and Python is reference tooling only.

FFprobe and FFmpeg accept private regular local files with explicit demuxers,
file-only protocol and bounded execution/output. Probe timeout is 30 seconds;
conversion timeout is ten minutes, each including version validation. Tools are
explicit paths, with only sibling FFprobe inference. Read-only modes, unmapped
rules and common-audio best skips do not launch tools. Missing audio, unsupported
containers, missing tools/encoders or nonempty stderr remain errors.

Orchestration downloads or merges into a same-directory private workspace, then
publishes the chosen final path without overwriting. A completed source is retained
at its original destination after processing failure; cancellation cleans private
files. Keep-video publishes the source first (using .orig before the final suffix
when conversion retains the original filename), then converted audio. If final
publication subsequently fails, the retained source remains public and the call
returns an error. The two publications are not one transaction. Subtitle sidecars
remain under their original media stem. Archives record only full success, with
an explicit guard against appending archive records to final/retained media paths.

Remaining scope: arbitrary postprocessor chains, video conversion, metadata and
thumbnail embedding, subtitle conversion, fixups, timestamp preservation,
installation/discovery, additional containers/codecs, general retention options
and whole-project parity. `-k` currently requires `-x` rather than implying
unsupported retention of separate merge inputs.

```sh
python3 tools/compare_audio.py --upstream /path/to/yt-dlp
```
