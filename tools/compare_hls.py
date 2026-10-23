#!/usr/bin/env python3
"""Compare native HLS fragment downloads with pinned upstream HlsFD, using saved VODs.

Development tooling only; Python is never a runtime fallback. Native -j output
mirrors upstream's info dict, which carries no HLS fragment list (HlsFD builds
fragments at download time), so this compares what both downloaders actually do.
A localhost server serves only the saved tests/fixtures/hls tree plus one
synthetic playlist built from those saved resources. For each playlist, the native
CLI downloads it (--fixup never) and upstream HlsFD.real_download downloads it with
external downloaders/FFmpeg disabled and name resolution restricted to 127.0.0.1.

Compared: the ordered segment/initialization request sequence (path, query and
Range header), the ordered key request sequence, and the exact assembled output
bytes. Key requests are compared as their own sequence, not interleaved with
fragments: upstream fetches a key lazily while decrypting the first fragment that
uses it (after that fragment's request), whereas native fetches it before the
fragment request. That timing difference is documented, not claimed as parity.
Byte-identical output of the AES fixtures implies the same keys, IVs (implicit
media-sequence IVs and explicit IVs) and PKCS#7 unpadding. Playlist fetches are
excluded: the native CLI fetches the playlist during extraction and again in its
downloader, while HlsFD alone fetches it once. Covered: TS, fMP4 with MAP, MAP and
segment byte ranges with implicit offsets, AES-128 TS/fMP4/byte-range with key
rotation, and a mixed playlist with a key-URI query, METHOD=NONE and a URI query.

Not covered: the fMP4/MAP fixtures use explicit IVs; pinned upstream counts MAP
as a media sequence increment for implicit IVs, whereas the native plan follows
RFC 8216's media-segment counter. Implicit-IV-after-MAP parity is not claimed.
No extension/container comparison is made: upstream remuxes TS HLS in
YoutubeDL's fixup stage, which neither side runs here. This does not establish live, master
selection, retry, concurrency or network acquisition parity.
"""
import argparse
import contextlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
from urllib.parse import urlsplit

PINNED_UPSTREAM = "6ccd977303412151ee23352c6257dd658ffc1987"
KEY_PREFIX = "/hls/aes-keys/"
PLAYLISTS = ("ts", "fmp4", "range", "aes-ts", "aes-fmp4", "aes-range")
# Built only from saved resources: implicit IVs 7 and 8 match aes-ts generation.
MIXED_PATH = "/hls/aes-ts/mixed.m3u8"
MIXED_MANIFEST = """#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:1
#EXT-X-MEDIA-SEQUENCE:7
#EXT-X-KEY:METHOD=AES-128,URI="../aes-keys/0.key?fixture=1"
#EXTINF:0.6,
segment-0.ts?part=0
#EXTINF:0.6,
segment-1.ts
#EXT-X-KEY:METHOD=NONE
#EXTINF:0.6,
../ts/segment-2.ts
#EXT-X-ENDLIST
"""
UPSTREAM_WORKER = r'''
import socket, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.downloader import hls

_getaddrinfo = socket.getaddrinfo
def local_only(host, *args, **kwargs):
    if host != '127.0.0.1':
        raise AssertionError(f'Non-local network forbidden in saved HLS comparison: {host!r}')
    return _getaddrinfo(host, *args, **kwargs)
socket.getaddrinfo = local_only
hls.FFmpegFD.available = lambda *args, **kwargs: False
hls.get_suitable_downloader = lambda *args, **kwargs: None
with YoutubeDL({'quiet': True, 'no_warnings': True, 'noprogress': True,
                'cachedir': False, 'proxy': ''}) as ydl:
    fd = hls.HlsFD(ydl, {'noprogress': True})
    assert fd.real_download(sys.argv[3], {
        'id': 'fixture', 'ext': 'mp4', 'url': sys.argv[2], 'is_live': False})
'''


def normalize_requests(requests, manifest_path):
    """Split non-playlist requests into ordered fragment and key (path, Range) lists."""
    result = {"fragments": [], "keys": []}
    for path, byte_range in requests:
        location = urlsplit(path).path
        if location == manifest_path:
            continue
        kind = "keys" if location.startswith(KEY_PREFIX) else "fragments"
        result[kind].append([path, byte_range])
    if not result["fragments"]:
        raise ValueError(f"no fragment requests recorded for {manifest_path}")
    return result


def compare(native_requests, upstream_requests, native_bytes, upstream_bytes, manifest_path):
    actual = normalize_requests(native_requests, manifest_path)
    expected = normalize_requests(upstream_requests, manifest_path)
    if actual != expected:
        raise ValueError(f"fragment request mismatch for {manifest_path}: " + json.dumps(
            {"native": actual, "upstream": expected}))
    if not upstream_bytes or native_bytes != upstream_bytes:
        raise ValueError(f"assembled output mismatch for {manifest_path}: native "
                         f"{len(native_bytes)} bytes, upstream {len(upstream_bytes)} bytes")
    return actual


def verify_upstream(upstream):
    if not (upstream / "yt_dlp/downloader/hls.py").is_file():
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


def parse_range(header, size):
    """Parse a single 'bytes=start-end' (inclusive) header into a slice."""
    if not header.startswith("bytes="):
        raise ValueError(f"unsupported Range header: {header!r}")
    start, _, end = header[len("bytes="):].partition("-")
    start = int(start)
    end = int(end) + 1 if end else size
    if not 0 <= start < end <= size:
        raise ValueError(f"unsatisfiable Range header: {header!r}")
    return start, end


@contextlib.contextmanager
def manifest_server(manifests, files=None):
    """Serve exact manifest paths and, optionally, a saved file tree.

    Each request is logged as (path with query, Range header or None). File lookup
    ignores the query string; anything unknown is a logged 404.
    """
    requests = []
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            byte_range = self.headers.get("Range")
            requests.append((self.path, byte_range))
            path = urlsplit(self.path).path
            body = manifests.get(path)
            if body is not None:
                body = body.encode("utf-8")
            elif files is not None and ".." not in path.split("/"):
                candidate = files.get(path)
                body = candidate.read_bytes() if candidate is not None else None
            status = 200 if body is not None else 404
            body = body if body is not None else b"unexpected request"
            if status == 200 and byte_range:
                try:
                    start, end = parse_range(byte_range, len(body))
                except ValueError:
                    status, body = 416, b"bad range"
                else:
                    self.send_response(206)
                    self.send_header("Content-Range", f"bytes {start}-{end - 1}/{len(body)}")
                    body = body[start:end]
                    status = None
            if status is not None:
                self.send_response(status)
            self.send_header("Content-Type", "application/vnd.apple.mpegurl"
                             if path.endswith(".m3u8") else "application/octet-stream")
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


def fixture_files(root):
    base = root / "tests/fixtures/hls"
    return {"/hls/" + path.relative_to(base).as_posix(): path
            for path in base.rglob("*") if path.is_file()}


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
        paths = [f"/hls/{name}/index.m3u8" for name in PLAYLISTS] + [MIXED_PATH]
        with manifest_server({MIXED_PATH: MIXED_MANIFEST}, fixture_files(root)) as (origin, requests), \
                tempfile.TemporaryDirectory() as directory:
            for index, path in enumerate(paths):
                url = origin + path
                output = Path(directory) / str(index)
                output.mkdir()
                requests.clear()
                subprocess.run([str(binary), "--fixup", "never", "--no-progress", "-q",
                                "-o", str(output / "native.%(ext)s"), url],
                               capture_output=True, text=True, encoding="utf-8", timeout=60, check=True)
                native_requests = list(requests)
                native_files = list(output.glob("native.*"))
                if len(native_files) != 1:
                    raise ValueError(f"expected one native output for {path}: {native_files!r}")
                requests.clear()
                subprocess.run([sys.executable, "-B", "-c", UPSTREAM_WORKER, str(upstream), url,
                                str(output / "upstream.bin")],
                               capture_output=True, text=True, encoding="utf-8", timeout=60, check=True)
                compare(native_requests, list(requests), native_files[0].read_bytes(),
                        (output / "upstream.bin").read_bytes(), path)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print("TS, fMP4/init, byte-range, AES-128 and mixed-key HLS downloads match pinned upstream "
          "HlsFD (ordered fragment/key requests with ranges, identical assembled bytes; "
          "no remux/live/master-selection parity claim)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
