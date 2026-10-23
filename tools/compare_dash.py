#!/usr/bin/env python3
"""Compare finite native DASH plans to the actual pinned upstream MPD parser.

Development/reference tooling only; Python is never a runtime fallback. Upstream
InfoExtractor._parse_mpd_formats_and_subtitles receives saved XML directly, with
sockets and urlopen blocked. The native CLI may GET only the saved localhost MPD;
segment, initialization, key and external requests fail the comparison.

Normalization is an explicit allowlist: both sides are compared in upstream
FormatSorter order (the reference is sorted by actual FormatSorter, as
process_video_result does), and format IDs (Representation@id; generic passes
no mpd_id for direct MPDs) must match at each position;
fragment relative paths resolve against upstream fragment_base_url; only the exact
ephemeral origin is replaced. Audio-only upstream tbr supplies normalized abr,
since upstream does not expose a separate abr. Compared fields are stream kind,
extension, dimensions, tbr/abr, language and ordered fragment URLs/ranges. Upstream
notes, codecs, fps/asr and stream counters have no native field
and are excluded. Native dash-period-0 grouping is asserted independently.

IMPORTANT: pinned upstream ignores SegmentList mediaRange and Initialization
range. For plan-list.mpd only URLs/metadata are compared with upstream; native
ranges are independently checked against explicit saved-fixture expectations.
This is NOT upstream byte-range parity. Nonnegative repeats/PTO=0 are compared;
pinned upstream does not expand negative repeats or apply presentationTimeOffset.
Native negative-repeat/PTO policy is outside this comparison, not normalized away.
This does not establish media download, decoding, merging, live, DRM or full DASH
compatibility. No source URL, timestamp, query or filename difference is ignored.
"""
import argparse
import contextlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import subprocess
import sys
import threading
from urllib.parse import urljoin, urlsplit

PINNED_UPSTREAM = "6ccd977303412151ee23352c6257dd658ffc1987"
# plan-list.mpd in FormatSorter order: audio, then video.
NATIVE_LIST_RANGES = [
    [{"offset": 0, "length": 8}, {"offset": 8, "length": 16}, {"offset": 24, "length": 24}],
    [{"offset": 0, "length": 16}, {"offset": 16, "length": 16}, {"offset": 32, "length": 32}],
]
UPSTREAM_WORKER = r'''
import json, socket, sys, xml.etree.ElementTree as ET
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.extractor.common import InfoExtractor

def forbidden(*args, **kwargs):
    raise AssertionError('Network forbidden in saved DASH parser comparison')
socket.create_connection = forbidden
socket.socket = forbidden
with YoutubeDL({'quiet': True, 'no_warnings': True, 'cachedir': False}) as ydl:
    ydl.urlopen = forbidden
    ie = InfoExtractor(ydl)
    ie._download_webpage = forbidden
    ie._download_xml = forbidden
    url = sys.argv[2]
    formats, subtitles = ie._parse_mpd_formats_and_subtitles(
        ET.fromstring(sys.stdin.read()), mpd_base_url=url.rsplit('/', 1)[0] + '/', mpd_url=url)
    assert not subtitles, 'unexpected subtitle results'
    from yt_dlp.utils import FormatSorter
    formats.sort(key=FormatSorter(ydl, []).calculate_preference)
    print(json.dumps(formats))
'''


def verify_upstream(upstream):
    if not (upstream / "yt_dlp/extractor/common.py").is_file():
        raise ValueError("--upstream must name an official yt-dlp source checkout")
    revision = subprocess.run(["git", "-C", str(upstream), "rev-parse", "HEAD"],
                              capture_output=True, text=True, encoding="utf-8",
                              timeout=15, check=True).stdout.strip()
    if revision != PINNED_UPSTREAM:
        raise ValueError(f"--upstream must be checked out at {PINNED_UPSTREAM}")


def normalize_url(url, origin):
    parsed = urlsplit(url)
    if f"{parsed.scheme}://{parsed.netloc}" == origin:
        return "{{ORIGIN}}" + url[len(origin):]
    return url


def normalize(formats, origin, *, upstream=False):
    result = []
    for source in formats:
        if upstream:
            video, audio = source.get("vcodec") != "none", source.get("acodec") != "none"
            if video == audio:
                raise ValueError("reference must expose separate, codec-classified audio/video")
            kind = "video_only" if video else "audio_only"
            format_id = source["format_id"]
        else:
            format_id = source["format_id"]
            if source.get("audio_group") != "dash-period-0":
                raise ValueError("native DASH period grouping changed")
            kind = source["stream_kind"]
        fragments = []
        for fragment in source["fragments"]:
            location = fragment.get("url") if upstream else fragment["url"]
            if upstream:
                location = location or fragment["path"]
                location = urljoin(source["fragment_base_url"], location)
            fragments.append({"url": normalize_url(location, origin),
                              "byte_range": fragment.get("byte_range") or None})
        result.append({"format_id": format_id, "stream_kind": kind,
                       "url": normalize_url(source["url"], origin),
                       "ext": source["ext"], "width": source.get("width"),
                       "height": source.get("height"), "tbr": source.get("tbr"),
                       "abr": source.get("tbr") if upstream and kind == "audio_only" else source.get("abr"),
                       "language": source.get("language"),
                       "container": source.get("container"),
                       "manifest_stream_number": source.get("manifest_stream_number"),
                       "fragments": fragments})
    return result


def compare(native, upstream, origin, *, native_only_ranges=None):
    actual, expected = normalize(native, origin), normalize(upstream, origin, upstream=True)
    if native_only_ranges is not None:
        if len(actual) != len(native_only_ranges):
            raise ValueError("native-only range fixture format count changed")
        for source, ranges in zip(actual, native_only_ranges):
            if [fragment["byte_range"] for fragment in source["fragments"]] != ranges:
                raise ValueError("native-only saved SegmentList ranges differ")
        # Upstream's range omission is a documented difference, not evidence of
        # range parity. Compare the remaining actual reference URL/metadata plan.
        actual = [{**source, "fragments": [{**fragment, "byte_range": None}
                   for fragment in source["fragments"]]} for source in actual]
    if actual != expected:
        raise ValueError("DASH plan mismatch: " + json.dumps({"native": actual, "upstream": expected}, sort_keys=True))
    return actual


@contextlib.contextmanager
def manifest_server(manifests):
    requests = []
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            body = manifests.get(self.path)
            self.send_response(200 if body is not None else 404)
            self.send_header("Content-Type", "application/dash+xml")
            body = (body if body is not None else "unexpected request").encode("utf-8")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)
        def log_message(self, *args):
            pass
    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}", requests
    finally:
        server.shutdown()
        server.server_close()
        worker.join(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, type=Path)
    parser.add_argument("--binary", type=Path, default=Path("target/debug/yt-dlp-rs"))
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    try:
        upstream = args.upstream.resolve()
        verify_upstream(upstream)  # Must precede binary inspection or server creation.
        binary = args.binary.resolve(strict=True)
        manifests = {f"/{name}": (root / "tests/fixtures/dash" / name).read_text(encoding="utf-8")
                     for name in ("plan-number.mpd", "plan-time.mpd", "plan-list.mpd")}
        manifests["/media/manifest.mpd"] = (root / "tests/fixtures/dash/media/manifest.mpd").read_text(encoding="utf-8")
        with manifest_server(manifests) as (origin, requests):
            for path, manifest in manifests.items():
                url = origin + path
                native = subprocess.run([str(binary), "-j", "-s", "-f", "bestvideo+bestaudio/best", url],
                                        capture_output=True, text=True, encoding="utf-8", timeout=30, check=True)
                reference = subprocess.run([sys.executable, "-B", "-c", UPSTREAM_WORKER,
                                           str(upstream), url], input=manifest,
                                          capture_output=True, text=True, encoding="utf-8", timeout=30, check=True)
                compare(json.loads(native.stdout)["formats"], json.loads(reference.stdout), origin,
                        native_only_ranges=NATIVE_LIST_RANGES if path == "/plan-list.mpd" else None)
            if requests != list(manifests):
                raise ValueError(f"unexpected MPD/segment/init requests: {requests!r}")
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print("Four saved DASH format/fragment plans match pinned upstream metadata and ordered URLs; "
          "SegmentList ranges checked independently (upstream ignores ranges; no range/download/remux parity claim)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
