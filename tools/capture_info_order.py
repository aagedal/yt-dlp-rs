#!/usr/bin/env python3
"""Capture pinned upstream `-j` lines for the info dict key-order fixture.

Development only: serves tests/fixtures/info_order/site on 127.0.0.1 with
fixed headers (HEADERS below) and runs the pinned upstream checkout's
`yt_dlp -j --user-agent fixture` (and `-J`) for each case. Lines are saved exactly,
except that the origin becomes {ORIGIN} (its host {HOST}) and the volatile `epoch` and
`_version` (upstream's release) are removed. The Rust test
tests/info_order.rs serves the same files with the same headers and compares
native output with these lines.
"""
import argparse
import json
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
import os
from pathlib import Path
import re
import subprocess
import sys
import threading

PINNED_UPSTREAM = '897341241f2bfb2439fe0367f4c7c9ccd29a3828'
ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/info_order'
LAST_MODIFIED = 'Mon, 01 Jan 2024 00:00:00 GMT'
CONTENT_TYPES = {
    'plain.m3u8': 'text/plain',
    '.mp4': 'video/mp4',
    '.qqq': 'application/octet-stream',
    '.webm': 'video/webm',
    '.html': 'text/html',
    '.m3u8': 'application/vnd.apple.mpegurl',
    '.mpd': 'application/dash+xml',
    '.vtt': 'text/vtt',
    '.jpg': 'image/jpeg',
}
CASES = ['clip.mp4', 'clip.qqq', 'one.html', 'rich.html', 'three.html', 'master.m3u8',
         'video.m3u8', 'plain.m3u8', 'segmented.mpd', 'text.mpd']


def content_type(name):
    return CONTENT_TYPES.get(name) or CONTENT_TYPES[os.path.splitext(name)[1]]


def normalize(line, origin):
    line = line.replace(origin, '{ORIGIN}').replace(origin.split('//', 1)[1], '{HOST}')
    line = re.sub(r'"epoch": \d+, ', '', line)
    return re.sub(r', "_version": \{[^{}]*\}', '', line)


class Handler(SimpleHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_GET(self):
        path = Path(self.directory) / self.path.lstrip('/').split('?')[0]
        if not path.is_file():
            self.send_error(404)
            return
        body = path.read_bytes()
        self.send_response(200)
        self.send_header('Content-Type', content_type(path.name))
        self.send_header('Content-Length', str(len(body)))
        self.send_header('Last-Modified', LAST_MODIFIED)
        self.end_headers()
        self.wfile.write(body)


def capture(upstream):
    server = ThreadingHTTPServer(('127.0.0.1', 0), partial(Handler, directory=str(ROOT / 'site')))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    origin = f'http://127.0.0.1:{server.server_port}'
    cases = []
    try:
        for name in CASES:
            case = {'name': name}
            for key, flag in (('lines', '-j'), ('single', '-J')):
                result = subprocess.run(
                    [sys.executable, '-m', 'yt_dlp', '--ignore-config', flag, '--user-agent', 'fixture',
                     f'{origin}/{name}'],
                    capture_output=True, text=True, timeout=60, check=True,
                    env=dict(os.environ, PYTHONPATH=str(upstream)))
                case[key] = [normalize(line, origin) for line in result.stdout.splitlines()]
            cases.append(case)
    finally:
        server.shutdown()
    return cases


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    parser.add_argument('--write', action='store_true', help='rewrite cases.json')
    args = parser.parse_args()
    upstream = args.upstream.resolve()
    revision = subprocess.run(['git', '-C', str(upstream), 'rev-parse', 'HEAD'],
                              capture_output=True, text=True, check=True).stdout.strip()
    if revision != PINNED_UPSTREAM:
        print(f'--upstream must be checked out at {PINNED_UPSTREAM}', file=sys.stderr)
        return 1
    cases = capture(upstream)
    fixture = {'upstream_commit': PINNED_UPSTREAM, 'last_modified': LAST_MODIFIED,
               'content_types': CONTENT_TYPES, 'cases': cases}
    path = ROOT / 'cases.json'
    if args.write:
        path.write_text(json.dumps(fixture, indent=1, ensure_ascii=False) + '\n', encoding='utf-8')
    elif json.loads(path.read_text(encoding='utf-8')) != fixture:
        print('captured upstream lines differ from cases.json', file=sys.stderr)
        return 1
    print(f'{len(cases)} info-order cases match pinned upstream')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
