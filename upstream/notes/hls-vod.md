# Native HLS VOD and fragment transport

Exact source baseline and target: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`
(yt-dlp 2026.08.19). The official checkout at this exact revision was used with
`tools/upstream_report.py --target` that SHA: zero intervening commits, changed
files, affected components or unmapped files. This extends a mapped planned
component from the existing baseline; it does not advance the global baseline.

Reviewed `yt_dlp/downloader/hls.py` HlsFD.can_download/real_download, its segment
construction around MAP/KEY/BYTERANGE (233–297), `downloader/fragment.py` fragment
retry/append/publication flow, and `extractor/common.py`
`_parse_m3u8_formats_and_subtitles` master/rendition/bandwidth/codec handling.
`test/test_InfoExtractor.py::test_parse_m3u8_formats` (661 onward) supplies
master and rendition examples. Mapped HLS files also contain encryption,
WebVTT/live/discontinuity and external-downloader behavior outside this slice;
DASH remains unmapped implementation work, not silently caught up.

The port provides direct generic and HTML5 HLS acquisition, bounded finite VOD
parsing, one master level, selected fragment plans, initialization and exact
byte ranges, native sequential downloading with per-fragment retries and
cooperative cancellation. State is explicit in Fragment/ByteRange, separate from
extraction, selection and output. Every failed fragment fails the download after
its retry budget; earlier complete fragments are retained only during that call.
No Python, installed yt-dlp or FFmpeg is used by this native HLS path.

Native policies deliberately differ from upstream: ENDLIST is required, only a
single initial MAP is accepted, map ranges require explicit offsets, and no
AES-128, DRM, live, discontinuity, gap, low-latency, changing initialization,
external-rendition or nested-master behavior is claimed. TS output is labeled
`.ts` and concatenated without implicit remuxing; upstream may choose `.mp4`
and subsequently use FFmpeg. Generic existing resolution/declaration-order
selection remains in force rather than upstream's full sorter. YouTube manifests
are still excluded from its extractor and anonymous API token policy is unchanged.

`tools/compare_hls.py` injects saved playlist data into actual pinned
HlsFD.real_download with sockets/external downloaders disabled. It intercepts
constructed fragments before file preparation/network execution and compares
native ordered URLs/ranges for TS, fMP4 and explicit/implicit ranges. Only exact
localhost origin and exclusive start/end-to-offset/length representation are
normalized. This proves fragment-plan agreement for these fixtures, not full
HLS protocol or remuxing parity.

Tests cover pure parser state/ranges/limits, generic/HTML5 master selection and
JSON simulation without segment requests, fragment response validation/retries,
cancellation and publication races. Original local media fixtures independently
verify exact bytes and optionally inspect H.264/AAC tracks with FFprobe 9.0.2;
see [provenance](../../tests/fixtures/hls/README.md). The existing Linux real-tool
CI job includes these checks; the standard three-platform suite needs no external
media tools or live-site requests.
