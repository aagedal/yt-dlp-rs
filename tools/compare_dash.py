#!/usr/bin/env python3
"""Compare native DASH format dicts to the actual pinned upstream MPD parser.

Development/reference tooling only; Python is never a runtime fallback. Upstream
InfoExtractor._parse_mpd_formats_and_subtitles receives saved XML directly, with
sockets and urlopen blocked, and its formats are sorted by the actual FormatSorter
(as process_video_result does, which also fills audio_ext/video_ext/abr/vbr).
Generic passes no mpd_id for direct MPDs, and neither does the reference. The
native CLI may GET only the saved localhost MPD; segment, initialization, key and
external requests fail the comparison.

Native -j formats mirror upstream's info dict, so whole format dicts are compared
in sorted order: the native key set must equal the upstream parser's key set plus
only the fields YoutubeDL.process_video_result adds afterwards (format, resolution,
aspect_ratio, http_headers), which are excluded. Every other value must match,
including format_id, url, manifest_url, ext, codecs, dimensions, fps/asr,
tbr/abr/vbr, language, format_note, container, protocol, manifest_stream_number,
is_dash_periods, fragment_base_url and each fragment's path/url/duration. The one
value rule applied to upstream is process_video_result's own: dynamic_range None
becomes 'SDR' for formats with video. Only the exact ephemeral origin is replaced.

IMPORTANT: pinned upstream ignores SegmentList mediaRange and Initialization
range, while native emits fragment byte_range for them. For plan-list.mpd native
ranges are checked against explicit saved-fixture expectations and then removed
before the upstream comparison. This is NOT upstream byte-range parity. Nonnegative
repeats/PTO=0 are compared; pinned upstream does not expand negative repeats or
apply presentationTimeOffset. Native negative-repeat/PTO policy is outside this
comparison, not normalized away. media/periods.mpd covers _merge_mpd_periods
(two Periods with explicit durations). This does not establish media download,
decoding, merging, live, DRM or full DASH compatibility. No source URL, timestamp, query or
filename difference is ignored.
"""
import argparse
import contextlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import subprocess
import sys
import threading
from urllib.parse import urlsplit

PINNED_UPSTREAM = "f264a6cbd1f630894da0f368df00f8aae50140e0"
# plan-list.mpd in FormatSorter order: audio, then video. Exclusive end offsets.
NATIVE_LIST_RANGES = [
    [{"start": 0, "end": 8}, {"start": 8, "end": 24}, {"start": 24, "end": 48}],
    [{"start": 0, "end": 16}, {"start": 16, "end": 32}, {"start": 32, "end": 64}],
]
# Added by YoutubeDL.process_video_result after extraction; not parser output.
PROCESSED_FIELDS = ("format", "resolution", "aspect_ratio", "http_headers")
URL_FIELDS = ("url", "manifest_url", "fragment_base_url")
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


def normalize_format(source, origin, *, upstream=False):
    """Normalize one native or upstream format dict without mutating it."""
    result = {key: value for key, value in source.items()
              if upstream or key not in PROCESSED_FIELDS}
    if upstream and result.get("dynamic_range") is None and result.get("vcodec") != "none":
        result["dynamic_range"] = "SDR"  # YoutubeDL.process_video_result
    for key in URL_FIELDS:
        if isinstance(result.get(key), str):
            result[key] = normalize_url(result[key], origin)
    if "fragments" in result:
        result["fragments"] = [
            {**fragment, "url": normalize_url(fragment["url"], origin)} if "url" in fragment
            else dict(fragment) for fragment in result["fragments"]]
    return result


def normalize(formats, origin, *, upstream=False):
    return [normalize_format(source, origin, upstream=upstream) for source in formats]


def compare_formats(actual, expected, label):
    if actual != expected:
        raise ValueError(f"{label} mismatch: " + json.dumps(
            {"native": actual, "upstream": expected}, sort_keys=True))
    return actual


def compare(native, upstream, origin, *, native_only_ranges=None):
    actual, expected = normalize(native, origin), normalize(upstream, origin, upstream=True)
    if native_only_ranges is not None:
        if len(actual) != len(native_only_ranges):
            raise ValueError("native-only range fixture format count changed")
        for source, ranges in zip(actual, native_only_ranges):
            if [fragment.get("byte_range") for fragment in source.get("fragments", [])] != ranges:
                raise ValueError("native-only saved SegmentList ranges differ")
        # Upstream's range omission is a documented difference, not evidence of
        # range parity. Compare the remaining format dicts and fragment plan.
        actual = [{**source, "fragments": [
            {key: value for key, value in fragment.items() if key != "byte_range"}
            for fragment in source["fragments"]]} for source in actual]
    return compare_formats(actual, expected, "DASH format")


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
        for name in ("manifest.mpd", "periods.mpd"):
            manifests[f"/media/{name}"] = (root / "tests/fixtures/dash/media" / name).read_text(encoding="utf-8")
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
    print("Five saved DASH manifests' format dicts and fragment plans match the pinned upstream "
          "parser; SegmentList ranges checked independently (upstream ignores ranges; "
          "no range/download/remux parity claim)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
