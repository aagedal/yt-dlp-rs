#!/usr/bin/env python3
"""Compare native fragment plans with pinned upstream HlsFD, using saved VODs.

Development tooling only. The Rust CLI reads localhost manifests in JSON simulation
mode. Upstream receives the same saved manifest text through
hls_media_playlist_data; socket access and external downloader discovery are
blocked. HlsFD.real_download constructs its actual fragments, intercepted before
any fragment download or file preparation. Comparison preserves fragment order,
URL query/fragment, and ranges. Upstream exclusive start/end ranges map exactly to
native offset/length; only the exact ephemeral localhost origin is replaced.
No extension/container comparison is made: native TS concatenation emits .ts,
while upstream may label TS HLS .mp4 and subsequently remux it. This check does not
establish remuxing, encryption, live, master selection, or network acquisition
parity. No segment or external site request is permitted.
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

PINNED_UPSTREAM = "897341241f2bfb2439fe0367f4c7c9ccd29a3828"
RANGE_MANIFEST = """#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:1
#EXT-X-MAP:URI="combined.mp4",BYTERANGE="10@0"
#EXTINF:1,
#EXT-X-BYTERANGE:20@10
combined.mp4
#EXTINF:1,
#EXT-X-BYTERANGE:30
combined.mp4
#EXT-X-ENDLIST
"""
UPSTREAM_WORKER = r'''
import json, socket, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.downloader import hls

def forbidden(*args, **kwargs):
    raise AssertionError('Network or external downloader forbidden in saved HLS comparison')
socket.create_connection = forbidden
socket.socket = forbidden
hls.FFmpegFD.available = lambda *args, **kwargs: False
hls.get_suitable_downloader = lambda *args, **kwargs: None
class Capture(hls.HlsFD):
    def _prepare_and_start_frag_download(self, ctx, info):
        ctx['fragment_index'] = 0
    def download_and_append_fragments(self, ctx, fragments, info, **kwargs):
        self.captured = fragments
        return True
with YoutubeDL({'quiet': True, 'no_warnings': True, 'cachedir': False}) as ydl:
    ydl.urlopen = forbidden
    fd = Capture(ydl, {})
    assert fd.real_download('forbidden-output', {
        'id': 'fixture', 'ext': 'mp4', 'url': sys.argv[2],
        'is_live': False, 'hls_media_playlist_data': sys.stdin.read()})
    print(json.dumps(fd.captured))
'''


def normalize(fragments, origin, *, upstream=False):
    result = []
    for fragment in fragments:
        url = fragment["url"]
        parsed = urlsplit(url)
        if f"{parsed.scheme}://{parsed.netloc}" == origin:
            url = "{{ORIGIN}}" + url[len(origin):]
        byte_range = fragment.get("byte_range")
        if upstream and byte_range:
            byte_range = {"offset": byte_range["start"],
                          "length": byte_range["end"] - byte_range["start"]}
        result.append({"url": url, "byte_range": byte_range or None})
    return result


def compare(native, upstream, origin):
    actual = normalize(native, origin)
    expected = normalize(upstream, origin, upstream=True)
    if actual != expected:
        raise ValueError("fragment plan mismatch: " + json.dumps(
            {"native": actual, "upstream": expected}, sort_keys=True))
    return actual


def verify_upstream(upstream):
    if not (upstream / "yt_dlp/downloader/hls.py").is_file():
        raise ValueError("--upstream must name an official yt-dlp source checkout")
    revision = subprocess.run(["git", "-C", str(upstream), "rev-parse", "HEAD"],
                              capture_output=True, text=True, encoding="utf-8",
                              timeout=15, check=True).stdout.strip()
    if revision != PINNED_UPSTREAM:
        raise ValueError(f"--upstream must be checked out at {PINNED_UPSTREAM}")


@contextlib.contextmanager
def manifest_server(manifests):
    requests = []
    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            requests.append(self.path)
            body = manifests.get(self.path)
            self.send_response(200 if body is not None else 404)
            self.send_header("Content-Type", "application/vnd.apple.mpegurl")
            body = (body or "unexpected request").encode("utf-8")
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
        verify_upstream(upstream)
        binary = args.binary.resolve(strict=True)
        manifests = {
            f"/{kind}/index.m3u8": (root / "tests/fixtures/hls" / kind / "index.m3u8").read_text(encoding="utf-8")
            for kind in ("ts", "fmp4")}
        manifests["/range.m3u8"] = RANGE_MANIFEST
        with manifest_server(manifests) as (origin, requests):
            for path, manifest in manifests.items():
                url = origin + path
                native = subprocess.run([str(binary), "--dump-json", "--simulate", url],
                                        capture_output=True, text=True, encoding="utf-8",
                                        timeout=30, check=True)
                reference = subprocess.run([sys.executable, "-B", "-c", UPSTREAM_WORKER,
                                            str(upstream), url], input=manifest,
                                           capture_output=True, text=True, encoding="utf-8",
                                           timeout=30, check=True)
                compare(json.loads(native.stdout)["fragments"], json.loads(reference.stdout), origin)
            if requests != list(manifests):
                raise ValueError(f"unexpected manifest/segment requests: {requests!r}")
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print("TS, fMP4/init, and byte-range fragment plans match pinned upstream HlsFD "
          "(ordered URLs and exact ranges only; no container/remux parity claim)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
