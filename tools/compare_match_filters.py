#!/usr/bin/env python3
"""Check saved --match-filters cases with pinned actual upstream `match_str`.

Development only; no network/process access in the reference worker. Each
filter is evaluated by actual `yt_dlp.utils.match_str` on the saved dictionary;
results and error classes are compared exactly.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'f264a6cbd1f630894da0f368df00f8aae50140e0'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/match_filters/cases.json'
UPSTREAM_WORKER = r'''
import json, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp.utils import match_str

def forbidden(*args, **kwargs):
    raise AssertionError('Network/process forbidden in match filter comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
fixture = json.load(sys.stdin)
result = []
for case in fixture['cases']:
    entry = {'filter': case['filter'], 'dict': case['dict']}
    try:
        entry['result'] = bool(match_str(case['filter'], case['dict']))
    except Exception as error:
        entry['error'] = type(error).__name__
    result.append(entry)
print(json.dumps(result, ensure_ascii=False))
'''


def compare(result, fixture):
    target = fixture['cases']
    if result != target:
        raise ValueError('match filter mismatch: ' + json.dumps(
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
            raise ValueError('match filter fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Match filters {len(fixture['cases'])} cases match pinned upstream "
          '(actual match_str; results and error classes)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
