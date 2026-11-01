#!/usr/bin/env python3
"""Compare this port's release binary with the pinned upstream on local servers.

Development only, never part of the required tests (timings depend on the
machine). Every request goes to 127.0.0.1: saved site fixtures are served by
tools/site_proxy.py, and generated HLS/DASH media by a local HTTP server that
can add a fixed delay before each response (per-request latency is what
concurrent fragment downloads, `-N`, hide).

Measured for both programs, as wall time and peak RSS (`/usr/bin/time -l` on
macOS, `-v` on Linux; upstream's is the Python interpreter's):

  startup     `--version`, and `--simulate` of a local direct media URL
  extract     `-J` of saved site fixtures (tests/fixtures/sites)
  hls, dash   downloading a generated VOD with `-N 1/4/8`, without and with
              per-request latency

Each case runs `--runs` times (after one warm-up run) and reports the median.
The result is written as JSON (`--output`); a summary table goes to stdout.

    cargo build --release --locked
    python3 tools/benchmark.py --upstream .upstream/yt-dlp --output bench.json
"""
import argparse
import http.server
import json
import os
from pathlib import Path
import platform
import re
import shutil
import statistics
import subprocess
import sys
import tempfile
import threading
import time

from capture_sites import PINNED_UPSTREAM, SITES, prepare, seeded_cache
from site_proxy import SiteProxy

ROOT = Path(__file__).resolve().parents[1]

# (fixture directory, case index): `-J` cases of the Media Converter sites
# that have one and succeed upstream.
EXTRACT_CASES = [
    ('youtube', 2),
    ('instagram', 0),
    ('tiktok', 0),
    ('vimeo', 0),
    ('dailymotion', 0),
    ('bandcamp', 0),
    ('soundcloud', 0),
    ('twitter', 0),
    ('facebook', 0),
    ('bluesky', 0),
]


class Media:
    """Generated media served from memory: `count` segments of `size` bytes."""

    def __init__(self, count, size):
        self.count = count
        self.size = size
        # MPEG-TS sync bytes keep sniffers quiet; the content is not decoded.
        packet = b'\x47' + bytes(187)
        self.segment = (packet * (size // 188 + 1))[:size]
        duration = 2
        lines = ['#EXTM3U', '#EXT-X-VERSION:3', f'#EXT-X-TARGETDURATION:{duration}',
                 '#EXT-X-MEDIA-SEQUENCE:0', '#EXT-X-PLAYLIST-TYPE:VOD']
        for index in range(count):
            lines += [f'#EXTINF:{duration}.0,', f'seg{index}.ts']
        lines.append('#EXT-X-ENDLIST')
        self.m3u8 = ('\n'.join(lines) + '\n').encode()
        total = count * duration
        self.mpd = f'''<?xml version="1.0" encoding="UTF-8"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT{total}S"
     minBufferTime="PT2S" profiles="urn:mpeg:dash:profile:isoff-live:2011">
  <Period id="0" start="PT0S">
    <AdaptationSet id="0" contentType="video" mimeType="video/mp4" segmentAlignment="true">
      <Representation id="video" codecs="avc1.64001f" width="1280" height="720" bandwidth="{size * 8 // duration}">
        <SegmentTemplate timescale="1" duration="{duration}" startNumber="0"
                         initialization="init.mp4" media="chunk$Number$.m4s"/>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>
'''.encode()


class MediaServer:
    """Serves `/<latency>/...` paths: the playlists, segments and a direct
    file, each response delayed by `latency` milliseconds."""

    def __init__(self, media):
        self.media = media
        self.requests = 0
        server = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = 'HTTP/1.1'

            def log_message(self, *args):
                pass

            def do_HEAD(self):
                self.respond(head=True)

            def do_GET(self):
                self.respond(head=False)

            def respond(self, head):
                server.requests += 1
                match = re.fullmatch(r'/(\d+)/([^?]+)(?:\?.*)?', self.path)
                if not match:
                    self.send_error(404)
                    return
                time.sleep(int(match[1]) / 1000)
                name = match[2]
                if name == 'index.m3u8':
                    body, kind = media.m3u8, 'application/vnd.apple.mpegurl'
                elif name == 'manifest.mpd':
                    body, kind = media.mpd, 'application/dash+xml'
                elif re.fullmatch(r'seg\d+\.ts', name):
                    body, kind = media.segment, 'video/mp2t'
                elif re.fullmatch(r'(chunk\d+\.m4s|init\.mp4)', name):
                    body, kind = media.segment, 'video/mp4'
                elif name == 'video.mp4':
                    body, kind = media.segment, 'video/mp4'
                else:
                    self.send_error(404)
                    return
                self.send_response(200)
                self.send_header('Content-Type', kind)
                self.send_header('Content-Length', str(len(body)))
                self.end_headers()
                if not head:
                    self.wfile.write(body)

        self.httpd = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
        self.httpd.daemon_threads = True
        threading.Thread(target=self.httpd.serve_forever, daemon=True).start()

    def url(self, latency, name):
        return f'http://127.0.0.1:{self.httpd.server_address[1]}/{latency}/{name}'


def peak_rss_bytes(stderr):
    """Peak RSS from `/usr/bin/time` output (macOS bytes, GNU kbytes)."""
    match = re.search(r'(\d+)\s+maximum resident set size', stderr)
    if match:
        return int(match[1])
    match = re.search(r'Maximum resident set size \(kbytes\): (\d+)', stderr)
    if match:
        return int(match[1]) * 1024
    return None


def time_flag():
    return '-l' if sys.platform == 'darwin' else '-v'


class Program:
    def __init__(self, name, command, env=None):
        self.name = name
        self.command = command
        self.env = env

    def run(self, args, cwd):
        """Wall seconds, peak RSS bytes and the exit code of one run."""
        started = time.perf_counter()
        result = subprocess.run(
            ['/usr/bin/time', time_flag(), *self.command, *args],
            capture_output=True, text=True, env=self.env, cwd=cwd, timeout=600)
        elapsed = time.perf_counter() - started
        return elapsed, peak_rss_bytes(result.stderr), result.returncode, result.stderr


def programs(binary, upstream):
    found = []
    if binary:
        found.append(Program('yt-dlp-rs', [str(binary)]))
    if upstream:
        env = dict(os.environ, PYTHONPATH=str(Path(upstream).resolve()),
                   PYTHONDONTWRITEBYTECODE='')
        found.append(Program('upstream', [sys.executable, '-m', 'yt_dlp'], env))
    return found


def measure(program, args, runs, setup=None, expect_bytes=None):
    """The median wall time and peak RSS over `runs` runs after a warm-up.
    With `expect_bytes`, a run must leave one file of that size."""
    times, peaks, failures = [], [], []
    for attempt in range(runs + 1):
        cwd = tempfile.mkdtemp(prefix='ytbench-')
        try:
            run_args = setup(cwd) if setup else args
            elapsed, peak, code, stderr = program.run(run_args, cwd)
            if code == 0 and expect_bytes is not None:
                sizes = [os.path.getsize(os.path.join(cwd, name)) for name in os.listdir(cwd)]
                if sizes != [expect_bytes]:
                    code, stderr = 1, f'expected one {expect_bytes}-byte file, found {sizes}'
        finally:
            shutil.rmtree(cwd, ignore_errors=True)
        if code != 0:
            failures.append(stderr.strip().splitlines()[-1:] or [f'exit {code}'])
            continue
        if attempt:
            times.append(elapsed)
            peaks.append(peak)
    if failures:
        return {'error': failures[0][0] if failures[0] else 'failed', 'failed_runs': len(failures)}
    peaks = [peak for peak in peaks if peak is not None]
    return {
        'runs': len(times),
        'median_s': round(statistics.median(times), 4),
        'min_s': round(min(times), 4),
        'max_s': round(max(times), 4),
        'peak_rss_mib': round(statistics.median(peaks) / 2**20, 1) if peaks else None,
    }


COMMON = ['--ignore-config', '--no-cache-dir']


def startup_cases(server):
    url = server.url(0, 'video.mp4')
    return [
        ('startup', '--version', ['--version']),
        ('startup', 'simulate direct URL', [*COMMON, '--simulate', url]),
    ]


def media_cases(server, latencies, workers):
    cases = []
    for kind, name in (('hls', 'index.m3u8'), ('dash', 'manifest.mpd')):
        for latency in latencies:
            for count in workers:
                label = f'-N {count}, {latency} ms latency'
                args = [*COMMON, '--no-progress', '-q', '--fixup', 'never', '-N', str(count),
                        '-o', 'out.%(ext)s', server.url(latency, name)]
                cases.append((kind, label, args))
    return cases


def extract_case(site, index):
    cases = json.loads((SITES / site / 'cases.json').read_text(encoding='utf-8'))
    return cases[index]


def run_extract(program, site, case, runs):
    proxy = SiteProxy(SITES / site)
    cache = seeded_cache()
    args = ['--ignore-config', '--proxy', proxy.url, '--no-check-certificate',
            '--user-agent', 'fixture', *case['args'], case['url']]
    if program.name == 'upstream':
        # The challenge solver cache capture_sites.py gives upstream.
        args = ['--cache-dir', str(cache), *args]
    else:
        args = ['--no-cache-dir', *args]

    def setup(cwd):
        prepare(SITES / site, case, cwd)
        return args
    try:
        return measure(program, args, runs, setup)
    finally:
        shutil.rmtree(cache, ignore_errors=True)



def version(program):
    result = subprocess.run([*program.command, '--version'], capture_output=True, text=True,
                            env=program.env)
    return result.stdout.strip()


def system_info():
    info = {
        'platform': platform.platform(),
        'machine': platform.machine(),
        'python': platform.python_version(),
        'cpus': os.cpu_count(),
    }
    if sys.platform == 'darwin':
        for key, name in (('cpu', 'machdep.cpu.brand_string'), ('memory_bytes', 'hw.memsize')):
            result = subprocess.run(['sysctl', '-n', name], capture_output=True, text=True)
            info[key] = result.stdout.strip()
    return info


def table(results):
    lines = []
    for group in results['groups']:
        lines.append(f"\n{group['name']}")
        for case in group['cases']:
            cells = []
            for name, value in case['results'].items():
                if 'error' in value:
                    cells.append(f"{name}: ERROR {value['error'][:60]}")
                else:
                    cells.append(f"{name}: {value['median_s']:.3f} s, {value['peak_rss_mib']} MiB")
            lines.append(f"  {case['label']:<36} " + ' | '.join(cells))
    return '\n'.join(lines)


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', type=Path, default=ROOT / 'target/release/yt-dlp-rs')
    parser.add_argument('--upstream', type=Path, help='pinned upstream checkout (omit to skip)')
    parser.add_argument('--runs', type=int, default=5)
    parser.add_argument('--groups', default='startup,extract,hls,dash')
    parser.add_argument('--segments', type=int, default=100)
    parser.add_argument('--segment-kib', type=int, default=256)
    parser.add_argument('--latencies', default='0,50', help='per-request delays in ms')
    parser.add_argument('--workers', default='1,4,8', help='`-N` values')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    if not args.binary.is_file():
        parser.error(f'{args.binary} not found; run `cargo build --release --locked`')
    progs = programs(args.binary, args.upstream)
    groups = args.groups.split(',')
    media = Media(args.segments, args.segment_kib * 1024)
    server = MediaServer(media)
    latencies = [int(value) for value in args.latencies.split(',')]
    workers = [int(value) for value in args.workers.split(',')]

    cases = []
    if 'startup' in groups:
        cases += startup_cases(server)
    if 'extract' in groups:
        cases += [('extract', f'{site} {" ".join(extract_case(site, index)["args"])}', (site, index))
                  for site, index in EXTRACT_CASES]
    cases += [case for case in media_cases(server, latencies, workers) if case[0] in groups]

    results = {
        'system': system_info(),
        'programs': {program.name: version(program) for program in progs},
        'upstream_commit': PINNED_UPSTREAM if args.upstream else None,
        'runs': args.runs,
        'media': {'segments': args.segments, 'segment_bytes': media.size,
                  'total_bytes': media.size * args.segments},
        'groups': [],
    }
    by_group = {}
    for group, label, case_args in cases:
        entry = {'label': label, 'results': {}}
        for program in progs:
            if group == 'extract':
                site, index = case_args
                value = run_extract(program, site, extract_case(site, index), args.runs)
            elif group in ('hls', 'dash'):
                # DASH also downloads the initialization segment.
                expect = media.size * (args.segments + (group == 'dash'))
                value = measure(program, case_args, args.runs, expect_bytes=expect)
            else:
                value = measure(program, case_args, args.runs)
            if group in ('hls', 'dash') and 'median_s' in value:
                value['mib_per_s'] = round(results['media']['total_bytes'] / 2**20 / value['median_s'], 1)
            entry['results'][program.name] = value
            print(f'{group}: {label}: {program.name}: {value}', file=sys.stderr)
        by_group.setdefault(group, []).append(entry)
    results['groups'] = [{'name': name, 'cases': entries} for name, entries in by_group.items()]
    if args.output:
        args.output.write_text(json.dumps(results, indent=1) + '\n', encoding='utf-8')
    print(table(results))


if __name__ == '__main__':
    main()
