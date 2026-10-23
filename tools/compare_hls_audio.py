#!/usr/bin/env python3
"""Compare saved external-audio HLS format dicts with pinned upstream InfoExtractor.

Development tooling only. Native CLI JSON simulation uses the saved audio-video
master and its two child playlists; the localhost server serves manifests only.
Upstream _parse_m3u8_formats_and_subtitles receives saved master text directly,
with ext='mp4' as GenericIE passes for direct m3u8 URLs, and with sockets and
YoutubeDL.urlopen forbidden. No segment/key/site requests occur. Its formats are
sorted by the actual FormatSorter, as process_video_result does (which also fills
audio_ext/video_ext/abr/vbr), and compared to native formats in that order.

Native -j formats mirror upstream's info dict, so whole format dicts are compared:
the native key set must equal the upstream parser's key set plus only the fields
YoutubeDL.process_video_result adds afterwards (format, resolution, aspect_ratio,
http_headers), which are excluded. Every other value must match, including
format_id, url, manifest_url, ext, protocol, codecs (an audio rendition is vcodec
'none' with no acodec, the video variant acodec 'none'), language, format_note,
width, height, fps and tbr. The one value rule applied to upstream is
process_video_result's own: dynamic_range None becomes 'SDR' for formats with
video. Only the exact ephemeral localhost origin is normalized. The native
bestvideo+bestaudio selection must pick two formats; which ones is not compared.
Native manifest requests must equal the pinned NATIVE_REQUESTS list, which differs
from upstream's (see its comment); no segment or key request is permitted.
This does not establish upstream selector, download, merge or live-site parity.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

from compare_dash import compare_formats, normalize
from compare_hls import PINNED_UPSTREAM, manifest_server, verify_upstream

# Known request difference, pinned rather than normalized: native generic reads
# the master once, then every variant/rendition playlist eagerly, then the first
# m3u8_native format's playlist again for the live-status check. Upstream GenericIE
# reads the master twice (webpage, then _extract_m3u8_formats) and only that
# live-status playlist; it never reads the other child playlists while extracting.
NATIVE_REQUESTS = ["/audio-video/master.m3u8", "/audio-video/video/index.m3u8",
                   "/audio-video/audio/index.m3u8", "/audio-video/audio/index.m3u8"]
UPSTREAM_WORKER = r'''
import json, socket, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import FormatSorter

def forbidden(*args, **kwargs):
    raise AssertionError('Network forbidden in saved external-audio HLS comparison')
socket.create_connection = forbidden
socket.socket = forbidden
with YoutubeDL({'quiet': True, 'no_warnings': True, 'cachedir': False}) as ydl:
    ydl.urlopen = forbidden
    formats, subtitles = InfoExtractor(ydl)._parse_m3u8_formats_and_subtitles(
        sys.stdin.read(), sys.argv[2], ext='mp4')
    assert not subtitles, 'Unexpected subtitle results in audio-only rendition fixture'
    formats.sort(key=FormatSorter(ydl, []).calculate_preference)
    print(json.dumps(formats))
'''


def compare(native, upstream, origin):
    return compare_formats(normalize(native, origin), normalize(upstream, origin, upstream=True),
                           "external-audio format")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, type=Path)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/yt-dlp-rs"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        upstream = args.upstream.resolve()
        verify_upstream(upstream)
        binary = args.binary.resolve(strict=True)
        fixture = root / "tests/fixtures/hls/audio-video"
        manifests = {f"/audio-video/{path}": (fixture / path).read_text(encoding="utf-8")
                     for path in ("master.m3u8", "video/index.m3u8", "audio/index.m3u8")}
        with manifest_server(manifests) as (origin, requests):
            url = origin + "/audio-video/master.m3u8"
            native = subprocess.run(
                [str(binary), "--format", "bestvideo+bestaudio", "--dump-json", "--simulate", url],
                capture_output=True, text=True, encoding="utf-8", timeout=30, check=True)
            reference = subprocess.run(
                [sys.executable, "-B", "-c", UPSTREAM_WORKER, str(upstream), url],
                input=manifests["/audio-video/master.m3u8"], capture_output=True,
                text=True, encoding="utf-8", timeout=30, check=True)
            info = json.loads(native.stdout)
            compare(info["formats"], json.loads(reference.stdout), origin)
            if len(info.get("requested_formats", [])) != 2:
                raise ValueError("native explicit pair did not select two streams")
            if [path for path, _ in requests] != NATIVE_REQUESTS:
                raise ValueError(f"unexpected manifest/segment/key requests: {requests!r}")
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Saved HLS external-audio format dicts match pinned upstream {PINNED_UPSTREAM} "
          "(sorted order; no selector/download/merge parity claim)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
