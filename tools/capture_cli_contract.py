#!/usr/bin/env python3
"""Capture pinned upstream output for the CLI output contract tests.

Development only. Reads tests/fixtures/cli_contract/cases.json, serves its
`routes` from a local HTTP server, and runs the pinned upstream checkout as
`yt_dlp --ignore-config ARGS` in a directory holding only the case's
`files` for every case (with a fake FFmpeg, `FFMPEG` in the arguments, that
concatenates its inputs; `signal` cases get that signal on their process
group a second after a route's `stall_after` stall begins). The exit code,
the normalized stdout and stderr, the file names and the normalized
contents of the `read` files are saved in expected.json. tests/cli_contract.rs runs this port the same way and
compares.

Normalization (see `normalize`): the server origin becomes `BASE`, the
working directory `DIR`, the fake FFmpeg's directory `TOOLS` and the program
name `yt-dlp`; a line holding a JSON object becomes `JSON ` and its
top-level keys (without `_version`), JSON `null` stays `null`; consecutive
duplicate lines are collapsed, because how many progress statuses a
download reports depends on read timing. Values that depend on timing
(speed, ETA, elapsed time) must not appear in the templates of the cases.
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import signal
import socketserver
import subprocess
import sys
import tempfile
import threading
import time

PINNED_UPSTREAM = 'f264a6cbd1f630894da0f368df00f8aae50140e0'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/cli_contract'

FAKE_FFMPEG = '''#!/bin/sh
case "$1" in -version|-bsfs) echo 'ffmpeg version 7.1-contract'; exit 0;; esac
for a; do out=${a#file:}; done
: > "$out"
prev=
for a; do [ "$prev" = -i ] && cat "${a#file:}" >> "$out"; prev=$a; done
exit 0
'''


def body(route):
    """The bytes a route serves: `text`, or `size` bytes of `fill`."""
    if 'text' in route:
        return route['text'].encode()
    return bytes([route.get('fill', 0)]) * route['size']


class Server:
    """A local server for `routes`; `stalled` is set when a route that
    `stall_after` bytes stops sending."""

    def __init__(self, routes):
        self.stalled = threading.Event()
        self.release = threading.Event()
        server = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *args):
                pass

            def do_GET(self):
                route = routes.get(self.path.split('?')[0])
                if route is None:
                    self.send_response(404)
                    self.send_header('Content-Length', '0')
                    self.send_header('Connection', 'close')
                    self.end_headers()
                    return
                data = body(route)
                self.send_response(200)
                self.send_header('Content-Type', route['type'])
                self.send_header('Content-Length', str(route.get('length', len(data))))
                self.send_header('Connection', 'close')
                self.end_headers()
                chunk = route.get('chunk') or len(data) or 1
                sent = 0
                try:
                    for start in range(0, len(data), chunk):
                        self.wfile.write(data[start:start + chunk])
                        self.wfile.flush()
                        sent += chunk
                        if route.get('stall_after') and sent >= route['stall_after']:
                            server.stalled.set()
                            server.release.wait(60)
                            return
                        time.sleep(route.get('delay_ms', 0) / 1000)
                except (BrokenPipeError, ConnectionResetError):
                    pass

        class Threaded(socketserver.ThreadingMixIn, http.server.HTTPServer):
            daemon_threads = True

        self.httpd = Threaded(('127.0.0.1', 0), Handler)
        self.base = f'http://127.0.0.1:{self.httpd.server_address[1]}'
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def close(self):
        self.release.set()
        self.httpd.shutdown()


def normalize(text, replacements):
    for old, new in replacements:
        text = text.replace(old, new)
    text = text.replace('yt-dlp-rs', 'yt-dlp')
    lines = []
    for line in text.split('\n'):
        try:
            value = json.loads(line)
        except ValueError:
            value = line
        if isinstance(value, dict):
            line = 'JSON ' + json.dumps([key for key in value if key != '_version'])
        if not lines or lines[-1] != line:
            lines.append(line)
    return lines


def run_case(command, case, server, tools, env=None):
    with tempfile.TemporaryDirectory() as work:
        for name, content in case.get('files', {}).items():
            Path(work, name).write_text(content.replace('BASE', server.base), encoding='utf-8')
        args = [arg.replace('BASE', server.base).replace('FFMPEG', str(Path(tools, 'ffmpeg')))
                for arg in case['args']]
        server.stalled.clear()
        server.release.clear()
        process = subprocess.Popen(
            [*command, '--ignore-config', *args], cwd=work, env=env, start_new_session=True,
            # A SIGINT ignored on entry (a background job) would stay ignored.
            preexec_fn=lambda: signal.signal(signal.SIGINT, signal.SIG_DFL),
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        if 'signal' in case:
            if not server.stalled.wait(60):
                process.kill()
                raise RuntimeError(f'{case["name"]}: the download never stalled')
            # The client reads the data sent before the stall first.
            time.sleep(1)
            os.killpg(process.pid, getattr(signal, case['signal']))
        stdout, stderr = process.communicate(timeout=120)
        server.release.set()
        replacements = [(os.path.realpath(work), 'DIR'), (work, 'DIR'),
                        (os.path.realpath(tools), 'TOOLS'), (tools, 'TOOLS'), (server.base, 'BASE')]
        result = {
            'name': case['name'],
            'args': case['args'],
            'code': process.returncode,
            'stdout': normalize(stdout.decode(), replacements),
            'stderr': normalize(stderr.decode(), replacements),
            'files': sorted(os.listdir(work)),
        }
        if 'read' in case:
            result['contents'] = {
                name: normalize(Path(work, name).read_text(encoding='utf-8'), replacements)
                for name in case['read']}
        return result


def write_tools(directory):
    ffmpeg = Path(directory, 'ffmpeg')
    ffmpeg.write_text(FAKE_FFMPEG)
    ffmpeg.chmod(0o755)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    parser.add_argument('--write', action='store_true')
    parser.add_argument('cases', nargs='*', help='only these case names (default: all)')
    options = parser.parse_args()
    head = subprocess.run(['git', '-C', str(options.upstream), 'rev-parse', 'HEAD'],
                          capture_output=True, text=True, check=True).stdout.strip()
    if head != PINNED_UPSTREAM:
        sys.exit(f'upstream checkout is at {head}, not the pinned {PINNED_UPSTREAM}')
    fixture = json.loads((FIXTURE / 'cases.json').read_text(encoding='utf-8'))
    env = dict(os.environ, PYTHONPATH=str(options.upstream.resolve()))
    server = Server(fixture['routes'])
    results = []
    try:
        with tempfile.TemporaryDirectory() as tools:
            write_tools(tools)
            for case in fixture['cases']:
                if options.cases and case['name'] not in options.cases:
                    continue
                result = run_case([sys.executable, '-m', 'yt_dlp'], case, server, tools, env)
                results.append(result)
                print(json.dumps(result, indent=1))
    finally:
        server.close()
    if options.write:
        path = FIXTURE / 'expected.json'
        saved = {}
        if options.cases and path.exists():
            saved = {case['name']: case for case in json.loads(path.read_text(encoding='utf-8'))['cases']}
        saved.update({result['name']: result for result in results})
        cases = [saved[case['name']] for case in fixture['cases'] if case['name'] in saved]
        path.write_text(json.dumps({'upstream_commit': PINNED_UPSTREAM, 'cases': cases},
                                   indent=1, ensure_ascii=False) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
