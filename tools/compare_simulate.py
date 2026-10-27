#!/usr/bin/env python3
"""Check saved simulate/quiet resolution with pinned actual upstream options.

Development only; no network/process access in the reference worker. Each
case's arguments go through actual `yt_dlp.parse_options` and
`YoutubeDL.__init__` (which turns listing into `simulate='list_only'`).
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'f264a6cbd1f630894da0f368df00f8aae50140e0'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/simulate/cases.json'
UPSTREAM_WORKER = r'''
import json, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL, parse_options

def forbidden(*args, **kwargs):
    raise AssertionError('Network/process forbidden in simulate comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
fixture = json.load(sys.stdin)
result = []
for case in fixture['cases']:
    parsed = parse_options([*case['args'], 'https://example.invalid/'])
    ydl = YoutubeDL(parsed.ydl_opts, auto_init=False)
    simulate = ydl.params.get('simulate')
    result.append({'args': case['args'], 'simulate': bool(simulate),
                   'list_only': simulate == 'list_only', 'quiet': bool(ydl.params.get('quiet'))})
print(json.dumps(result))
'''


def compare(result, fixture):
    target = fixture['cases']
    if result != target:
        raise ValueError('simulate mismatch: ' + json.dumps(
            {'upstream': result, 'expected': target}, ensure_ascii=False, sort_keys=True))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    args = parser.parse_args()
    try:
        upstream = args.upstream.resolve()
        if not (upstream / 'yt_dlp/__init__.py').is_file():
            raise ValueError('--upstream must name an official yt-dlp source checkout')
        revision = subprocess.run(['git', '-C', str(upstream), 'rev-parse', 'HEAD'],
                                  capture_output=True, text=True, encoding='utf-8', timeout=15,
                                  check=True).stdout.strip()
        if revision != PINNED_UPSTREAM:
            raise ValueError(f'--upstream must be checked out at {PINNED_UPSTREAM}')
        fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        if fixture['upstream_commit'] != PINNED_UPSTREAM:
            raise ValueError('simulate fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Simulate {len(fixture['cases'])} cases match pinned upstream "
          '(actual parse_options/YoutubeDL; simulate, list_only and quiet)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
