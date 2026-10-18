# External HLS audio renditions and copy merging

Exact pinned source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
`tools/upstream_report.py` against the official checkout at that target reports
zero intervening commits/files, affected components or unmapped files. This is
a port slice from the existing baseline, not an assertion that HLS is complete.

Reviewed `InfoExtractor._parse_m3u8_formats_and_subtitles` in
`yt_dlp/extractor/common.py`: extract_media around 2283–2335 preserves AUDIO
rendition NAME/language and audio-only classification; 2420–2435 handles known
CODECS with an external AUDIO group and clears acodec on the video format.
`test/test_InfoExtractor.py::test_parse_m3u8_formats` includes multi-rendition
examples. Also reviewed `FFmpegMergerPP.run` for track maps and stream copy,
including upstream's m3u8 AAC `aac_adtstoasc` handling. The applicable behavior
is separate audio rendition metadata/selection and final two-track merging.

Native parser/extraction retains external AUDIO groups and flags, validates names,
references/defaults, snapshots each child media plan, and preserves source/page
referrers. Inband groups retain existing behavior. Combined variants/renditions
are limited to 256 and their retained fragments to 10000. Mixed inband/external
audio groups, external video/subtitle groups, nested masters and unresolved
references remain unsupported. Unlike upstream's missing-CODECS complete-format
heuristic, native extraction rejects an external-group variant without recognized
video CODECS; it does not silently drop its declared audio.

Native automatic pairing checks group association and searches video rank order
for complete pairs. Best audio ranks default/autoselect then bitrate; explicit
IDs remain available, and two literal IDs deliberately override group matching.
No claim of general upstream selection/sorting/language preference parity is made.
HTML source indices namespace both format IDs and group associations. Added
format fields deserialize with defaults and are omitted when absent/false.

A shared container policy now permits TS video and AAC/TS audio, with fixed local
FFmpeg mpegts/aac demuxers. MP4/TS video with M4A/AAC/TS audio produces MP4;
WebM/WebM remains WebM and mixed supported containers produce Matroska. Stream
copy, file-only protocols, explicit tool paths, process budgets and no-clobber
publication are unchanged. Actual FFmpeg 9.0.2 inserted aac_adtstoasc automatically
for AAC-to-MP4; real tests confirm AAC codec extradata and H.264/AAC output tracks.
No codec conversion or installed yt-dlp/Python runtime extraction is used.

The original saved fixtures contain separate video-only H.264 TS and AAC audio.
`tests/hls_media.rs` verifies the whole path from master acquisition, grouped
selection and two native fragment downloads through real merging, stream/fragment
progress and final publication. Additional tests cover metadata/defaults, group
mismatch/fallback/overrides, audio-only masters and fMP4/M4A, namespace boundaries,
HTTP headers, invalid groups/flags, and simulation without segment/key requests.
See [fixture generation](../../tests/fixtures/hls/README.md).

`tools/compare_hls_audio.py` invokes the actual pinned upstream parser with the
saved master injected and sockets/urlopen blocked. Native CLI extraction uses
only localhost master/child manifests. URL-keyed metadata compares stream kind,
language, format note, dimensions and total bitrate. Only the exact local origin
is normalized. Format IDs/order, container extension, native group/default flag
fields and selection/merging are excluded explicitly; this is parser metadata
evidence, not general acquisition/download/remux or live-site parity.
