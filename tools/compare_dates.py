#!/usr/bin/env python3
"""Check saved date parsing cases with pinned actual upstream helpers.

Development only; no network/process access in the reference worker. Each
input is parsed by actual `yt_dlp.utils.unified_timestamp` (day-first and
month-first) and each timestamp formatted by `strftime_or_none`; results and
the classes of exceptions upstream raises are compared exactly.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = '897341241f2bfb2439fe0367f4c7c9ccd29a3828'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/dates/cases.json'
UPSTREAM_WORKER = r'''
import json, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp.utils import strftime_or_none, unified_timestamp

def forbidden(*args, **kwargs):
    raise AssertionError('Network/process forbidden in date comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
fixture = json.load(sys.stdin)
result = {'unified_timestamp': [], 'strftime': []}
for case in fixture['unified_timestamp']:
    entry = {'input': case['input']}
    for key, day_first in (('day_first', True), ('month_first', False)):
        try:
            entry[key] = unified_timestamp(case['input'], day_first)
        except Exception as error:
            entry[key] = None
            entry[key + '_error'] = type(error).__name__
    result['unified_timestamp'].append(entry)
for case in fixture['strftime']:
    result['strftime'].append({'timestamp': case['timestamp'],
                               'date': strftime_or_none(case['timestamp'])})
print(json.dumps(result, ensure_ascii=False))
'''


def compare(result, fixture):
    target = {key: fixture[key] for key in ('unified_timestamp', 'strftime')}
    if result != target:
        raise ValueError('date mismatch: ' + json.dumps(
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
            raise ValueError('date fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Dates {len(fixture['unified_timestamp'])} inputs and {len(fixture['strftime'])} "
          'timestamps match pinned upstream (actual unified_timestamp/strftime_or_none)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
