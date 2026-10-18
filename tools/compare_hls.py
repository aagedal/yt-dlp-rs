#!/usr/bin/env python3
"""Compare native fragment plans with pinned upstream HlsFD, using saved VODs.

Development tooling only. The Rust CLI reads localhost manifests in JSON simulation
mode. Upstream receives the same saved manifest text through
hls_media_playlist_data; socket access and external downloader discovery are
blocked. HlsFD.real_download constructs its actual fragments, intercepted before
any fragment download or file preparation. Comparison preserves fragment order,
URL query/fragment, and ranges. Upstream exclusive start/end ranges map exactly to
native offset/length. AES-128 key URLs and resolved 16-byte IVs are compared;
upstream's explicit byte IV maps to a JSON byte list, and its implicit IV maps
from media_sequence to a 128-bit big-endian integer. Only the exact ephemeral
localhost origin is replaced (including key URLs). The fMP4/MAP case uses explicit
IVs: pinned upstream counts MAP as a media sequence increment, whereas the native
plan follows RFC 8216's media-segment counter for implicit IVs. This difference is
not normalized away and implicit-IV-after-MAP parity is not claimed.
No extension/container comparison is made: native TS concatenation emits .ts,
while upstream may label TS HLS .mp4 and subsequently remux it. This check does not
establish decryption, remuxing, live, master selection, or network acquisition
parity. No segment, key, or external site request is permitted.
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
AES_TS_MANIFEST = """#EXTM3U
#EXT-X-VERSION:3
#EXT-X-TARGETDURATION:1
#EXT-X-MEDIA-SEQUENCE:7
#EXT-X-KEY:METHOD=AES-128,URI="keys/first.key?fixture=1"
#EXTINF:1,
first.ts
#EXTINF:1,
second.ts
#EXT-X-KEY:METHOD=AES-128,URI="keys/rotated.key",IV=0x000000000000000000000000000000ab
#EXTINF:1,
third.ts
#EXT-X-KEY:METHOD=NONE
#EXTINF:1,
plain.ts
#EXT-X-KEY:METHOD=AES-128,URI="keys/first.key?fixture=1"
#EXTINF:1,
fifth.ts
#EXT-X-ENDLIST
"""
AES_MAP_MANIFEST = """#EXTM3U
#EXT-X-VERSION:7
#EXT-X-TARGETDURATION:1
#EXT-X-MEDIA-SEQUENCE:21
#EXT-X-KEY:METHOD=AES-128,URI="keys/map.key",IV=0x00000000000000000000000000000001
#EXT-X-MAP:URI="init.mp4"
#EXTINF:1,
first.m4s
#EXT-X-KEY:METHOD=AES-128,URI="keys/rotated.key",IV=0x00000000000000000000000000000002
#EXTINF:1,
second.m4s
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
    print(json.dumps(fd.captured, default=lambda value: list(value) if isinstance(value, bytes) else forbidden()))
'''


def normalize_url(url, origin):
    parsed = urlsplit(url)
    if f"{parsed.scheme}://{parsed.netloc}" == origin:
        return "{{ORIGIN}}" + url[len(origin):]
    return url


def normalize(fragments, origin, *, upstream=False):
    result = []
    for fragment in fragments:
        url = normalize_url(fragment["url"], origin)
        byte_range = fragment.get("byte_range")
        if upstream and byte_range:
            byte_range = {"offset": byte_range["start"],
                          "length": byte_range["end"] - byte_range["start"]}
        encryption = fragment.get("encryption")
        if upstream:
            decrypt_info = fragment.get("decrypt_info", {})
            method = decrypt_info.get("METHOD", "NONE")
            if method == "AES-128":
                iv = decrypt_info.get("IV")
                if iv is None:
                    iv = list(fragment["media_sequence"].to_bytes(16, "big"))
                encryption = {"key_url": decrypt_info["URI"], "iv": iv}
            elif method != "NONE":
                raise ValueError(f"unexpected upstream encryption method: {method}")
        if encryption is not None:
            iv = encryption["iv"]
            if not isinstance(iv, list) or len(iv) != 16 or any(
                    type(byte) is not int or not 0 <= byte <= 255 for byte in iv):
                raise ValueError("invalid AES-128 IV byte list")
            encryption = {"key_url": normalize_url(encryption["key_url"], origin), "iv": list(iv)}
        result.append({"url": url, "byte_range": byte_range or None, "encryption": encryption})
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
        manifests["/aes/sequence.m3u8"] = AES_TS_MANIFEST
        manifests["/aes/map.m3u8"] = AES_MAP_MANIFEST
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
                raise ValueError(f"unexpected manifest/segment/key requests: {requests!r}")
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print("TS, fMP4/init, byte-range, and AES-128 fragment plans match pinned upstream HlsFD "
          "(ordered URLs, ranges, key URLs and IVs; no decryption/container/remux parity claim)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
