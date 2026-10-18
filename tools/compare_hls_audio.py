#!/usr/bin/env python3
"""Compare saved external-audio HLS metadata with pinned upstream InfoExtractor.

Development tooling only. Native CLI JSON simulation uses the saved audio-video
master and its two child playlists; the localhost server permits manifests only.
Upstream _parse_m3u8_formats_and_subtitles receives saved master text directly,
with sockets and YoutubeDL.urlopen forbidden. No segment/key/site requests occur.

The explicit allowlist is URL, stream kind, language, rendition name (format_note),
width, height, and tbr. Upstream codec 'none' markers map to native stream kinds.
Only the exact ephemeral localhost origin is normalized. Both lists are sorted by
unique URL because upstream emits audio first and native emits video first; this
is a documented ordering difference, not a claim of declaration-order parity.
Duplicate URLs fail instead of being collapsed. Missing optional fields become
None. IDs, extension, group/default/autoselect flags, protocol and other fields
are excluded: IDs differ; upstream does not infer the raw AAC extension from
child bodies, and its parser does not expose the native group/boolean fields.
This does not establish upstream selector, download, merge or live-site parity.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

from compare_hls import PINNED_UPSTREAM, manifest_server, normalize_url, verify_upstream

FIELDS = ("url", "stream_kind", "language", "format_note", "width", "height", "tbr")
UPSTREAM_WORKER = r'''
import json, socket, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.extractor.common import InfoExtractor

def forbidden(*args, **kwargs):
    raise AssertionError('Network forbidden in saved external-audio HLS comparison')
socket.create_connection = forbidden
socket.socket = forbidden
with YoutubeDL({'quiet': True, 'no_warnings': True, 'cachedir': False}) as ydl:
    ydl.urlopen = forbidden
    formats, subtitles = InfoExtractor(ydl)._parse_m3u8_formats_and_subtitles(
        sys.stdin.read(), sys.argv[2])
    assert not subtitles, 'Unexpected subtitle results in audio-only rendition fixture'
    print(json.dumps(formats))
'''


def normalize(formats, origin, *, upstream=False):
    normalized = []
    seen = set()
    for source in formats:
        url = normalize_url(source["url"], origin)
        if url in seen:
            raise ValueError(f"duplicate format URL: {url}")
        seen.add(url)
        if upstream:
            audio_only = source.get("vcodec") == "none"
            video_only = source.get("acodec") == "none"
            if audio_only and video_only:
                raise ValueError("upstream format contains neither audio nor video")
            kind = "audio_only" if audio_only else "video_only" if video_only else "standalone"
        else:
            kind = source.get("stream_kind", "standalone")
            if kind not in ("audio_only", "video_only", "standalone"):
                raise ValueError(f"unknown native stream kind: {kind}")
        normalized.append({
            "url": url, "stream_kind": kind,
            **{field: source.get(field) for field in FIELDS if field not in ("url", "stream_kind")},
        })
    return sorted(normalized, key=lambda source: source["url"])


def compare(native, upstream, origin):
    actual = normalize(native, origin)
    expected = normalize(upstream, origin, upstream=True)
    if actual != expected:
        raise ValueError("external-audio metadata mismatch: " + json.dumps(
            {"native": actual, "upstream": expected}, sort_keys=True))
    return actual


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
            if sorted(requests) != sorted(manifests):
                raise ValueError(f"unexpected manifest/segment/key requests: {requests!r}")
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Saved HLS external-audio metadata matches pinned upstream {PINNED_UPSTREAM} "
          "(URL-keyed allowlist; no selector/download/merge parity claim)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
