# Local video remuxing and recoding

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against this exact checkout and target. The report
contains no intervening commits, changed files, affected components or unmapped
files. No baseline advancement or general catch-up claim is made.

Inspected `postprocessor/ffmpeg.py:67–80,213–223,326–368,538–579`,
`options.py:1632–1644,1667–1671`, `__init__.py:261–262,533–555,663–673`,
`utils/_utils.py` extension helpers and media-extension sets, and the
postprocessor tests. These files also cover arbitrary postprocessors, probing,
codec feature discovery, metadata, subtitles, fixups, timestamps and additional
CLI behavior outside this slice. No unmapped intervening changes exist.

The model ports ordered mappings, lowercased source extension matching,
same-target/unmapped skips, full pinned target validation and exact stream options.
Remux maps all known streams, drops data and copies codecs. Recode uses container
encoder defaults except the source AVI Xvid override. The bounded runner adds
source faststart flags, explicit local demuxer/muxer selection, file-only protocol,
closed stdin, version validation, process/pipe budgets and cancellation. Literal
alac/vorbis targets remain explicit errors when active because the source's
suffix-based muxer inference cannot name an output container; the audio processor
has the separate working ALAC/Vorbis codec paths. No alternate codec is guessed.

An offline subclass captures actual pinned `parse_options` and
`FFmpegVideoRemuxerPP` / `FFmpegVideoConvertorPP.run` results for 70 shared cases,
one source-only Unicode mapping and eight invalid options. All options, source
stream argv, paths, warnings and deletion arrays are compared without
normalization. This does not compare media bytes or establish output-codec
availability. Native fixed muxer/protocol/process choices are documented separately
from those pure plans. Python remains development/reference tooling only.

Shared orchestration keeps existing audio behavior and runs audio before video,
matching source processor order and updated-extension mapping. Recode wins over
remux with the pinned warning. `-k` retains each completed source/intermediate;
conflicting earlier names gain `.orig` before their suffix. It does not retain
separate pre-merge input files. Without keep, only final media is published.
Processing failure preserves the latest completed input and earlier inputs with
keep. Recovery attempts every available retained path even if another conflicts.
Cancellation cleans private files. Publication is no-clobber and can partially
succeed across retained files, final media, sidecars and archive state. Archives
record only full success and cannot append to any published media path.

Portable Rust process fixtures cover arguments, deadlines, pipe failures,
version/banner checks, cancellation and output validation. Local HTTP tests cover
CLI/library behavior, format selection, mapping skips, option conflicts,
continuation/archive timing, collisions/races, all-stage retention, source→audio→video
and merge→video composition, uppercase naming and extension cycles. Configured
real tools prove saved H.264 stream-copy to MKV and actual AVI MPEG-4 recoding;
required tests have no live-site dependency. Linux media CI runs both pipeline
and runner real-tool checks.

Remaining scope includes arbitrary postprocessor chains, separate merger-input
retention, encoder/preset/postprocessor arguments, metadata/thumbnail/subtitle
embedding, fixups, timestamp preservation, discovery/installation and all-container/
all-codec output guarantees. See [compatibility details](../../docs/COMPATIBILITY.md#video-remuxing-and-conversion)
and [reference provenance](../../tests/fixtures/video/README.md).

```sh
python3 tools/compare_video.py --upstream /path/to/yt-dlp
```
