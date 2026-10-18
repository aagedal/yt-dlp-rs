# Direct adaptive selection and stream-copy merging

Source baseline and exact target: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`
(yt-dlp 2026.08.19). The official checkout at this commit was inspected; the
upstream report from baseline to target contains zero commits/files, affected
components or unmapped files. This is another bounded port from the existing
baseline, not an upstream-baseline advance or a claim of complete parity.

Reviewed sources include YouTube `_video.py` direct formats/adaptiveFormats and
codec/bitrate metadata, `YoutubeDL.py` format selector and merge result handling,
`test/test_YoutubeDL.py` format selection/audio/video and default selector tests,
and `postprocessor/ffmpeg.py` `FFmpegMergerPP.run`. In `_video.py`, the reviewed
sections at this commit include formats/adaptiveFormats (3393), bitrate (3429),
token gating (3439), metadata and codec parsing (3468–3482), fragment/OTF
exclusions (3498–3505), and WEB non-18 token policy (3517–3524). The applicable behavior is
explicit selection of separate video/audio tracks and local stream-copy merging.
Upstream maps selected track kinds and uses `-c copy`; this implementation does
the same for exactly two direct HTTP inputs. Upstream's full sorting, arbitrary
selectors, multistreams, discovery/version compatibility, manifest/AAC fixups,
remux options and other postprocessors remain outside this slice.

Native policy is deliberately narrower: typed standalone/audio-only/video-only
metadata, positive declared kbps, resolution then bitrate video ranking, audio
bitrate ranking, fixed bounded `+`/`/` syntax, no tool discovery, explicit container
choices and no-clobber final publication. Default with an explicit merger uses
`bestvideo+bestaudio/best`, not upstream's broader `bestvideo*` grammar. Without a
merger, selection does not silently drop video when only separate tracks exist.

The watch-page saved adaptive fixture is compared offline with pinned upstream.
The documented missing-PO-token comparison option permits parsing this synthetic
fixture only; media usability and live-site reliability are not inferred. The
anonymous WEB API's itag-18 exemption policy remains unchanged. Unknown codecs,
DRM/live/OTF/fragments and manifest URLs remain excluded before challenge solving.

Regression coverage includes pure selectors, atomic playlist selection, saved
adaptive extraction with local probes, native fake-tool process budgets and
arguments, download/merge cancellation and cleanup, and final publication races.
Optional real-tool fixtures are original generated color/sine media; see
[their provenance](../../tests/fixtures/merge/README.md). Local validation uses
FFmpeg/FFprobe 9.0.2. CI also runs a Linux real-tool job using its distribution
package; this does not define a general supported FFmpeg version range.

Python remains reference tooling only. No Python or installed yt-dlp process is
used by extraction, selection, download or merging at runtime.
