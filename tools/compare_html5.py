#!/usr/bin/env python3
"""Compare the supported HTML5 subset with an explicit upstream checkout.

Development tooling only: neither executable uses the other as a runtime fallback.
Only saved video/audio/source HTML fixtures and fixed media bytes are served, on
localhost. Upstream is restricted to GenericIE and its HTML5MediaEmbedIE helper.
Compare id, title, ext, url, webpage_url, extractor, extractor_key
exactly. The sole normalization removes this server's origin from url and
webpage_url; query strings, fragments, paths, titles and all other fields remain
unchanged. Volatile and unsupported fields are excluded by this explicit allowlist.
"""

import argparse
from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import subprocess
import sys
import threading
from urllib.parse import urlsplit


FIELDS = ("id", "title", "ext", "url", "webpage_url", "extractor", "extractor_key")
FIXTURES = ("video", "audio", "source")
UPSTREAM_WORKER = """
import json, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
with YoutubeDL({'quiet': True, 'no_warnings': True, 'skip_download': True,
                'allowed_extractors': ['generic', 'html5'], 'cachedir': False}) as ydl:
    print(json.dumps(ydl.sanitize_info(ydl.extract_info(sys.argv[2], download=False))))
"""


def normalize(info, origin):
    """Select comparison fields, removing only the exact local fixture origin."""
    result = {field: info[field] for field in FIELDS}
    for field in ("url", "webpage_url"):
        value = result[field]
        parsed = urlsplit(value)
        if f"{parsed.scheme}://{parsed.netloc}" == origin:
            result[field] = value[len(origin):]
    return result


def compare(upstream, rust, origin):
    expected = normalize(upstream, origin)
    actual = normalize(rust, origin)
    if expected != actual:
        differences = {
            field: {"upstream": expected[field], "rust": actual[field]}
            for field in FIELDS if expected[field] != actual[field]
        }
        raise ValueError("metadata mismatch: " + json.dumps(differences, sort_keys=True))
    return actual


@contextmanager
def fixture_server(directory):
    pages = {
        f"/{name}.html": ("text/html; charset=utf-8", (directory / f"{name}.html").read_bytes())
        for name in FIXTURES
    }
    pages.update({
        "/media/clip.mp4": ("video/mp4", b"media bytes"),
        "/media/tone.mp3": ("audio/mpeg", b"audio"),
    })

    class Handler(BaseHTTPRequestHandler):
        def do_GET(self):
            entry = pages.get(urlsplit(self.path).path)
            if entry is None:
                self.send_error(404)
                return
            mime, body = entry
            self.send_response(200)
            self.send_header("Content-Type", mime)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            try:
                self.wfile.write(body)
            except (BrokenPipeError, ConnectionResetError):
                pass  # Header-only media probing may close the connection.

        def log_message(self, *_args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    worker = threading.Thread(target=server.serve_forever, daemon=True)
    worker.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}"
    finally:
        server.shutdown()
        server.server_close()
        worker.join()


def run_json(command):
    completed = subprocess.run(command, capture_output=True, text=True, timeout=30, check=True)
    return json.loads(completed.stdout)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, type=Path)
    parser.add_argument("--binary", required=True, type=Path)
    args = parser.parse_args()
    upstream = args.upstream.resolve()
    binary = args.binary.resolve()
    if not (upstream / "yt_dlp" / "extractor" / "generic.py").is_file():
        parser.error("--upstream must name an official yt-dlp source checkout")
    if not binary.is_file():
        parser.error("--binary must name a built Rust executable")
    fixtures = Path(__file__).resolve().parents[1] / "tests" / "fixtures" / "html5"
    mismatches = False
    try:
        with fixture_server(fixtures) as origin:
            for name in FIXTURES:
                url = f"{origin}/{name}.html"
                expected = run_json([sys.executable, "-c", UPSTREAM_WORKER, str(upstream), url])
                actual = run_json([str(binary), "-j", url])
                try:
                    compare(expected, actual, origin)
                except ValueError as error:
                    mismatches = True
                    print(f"{name}: {error}", file=sys.stderr)
                else:
                    print(f"{name}: metadata matches")
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    return int(mismatches)


if __name__ == "__main__":
    raise SystemExit(main())
