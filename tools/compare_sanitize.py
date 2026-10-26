#!/usr/bin/env python3
"""Check saved filename sanitization cases with pinned actual `prepare_filename`.

Development only; no network/process/download access in the reference worker.
Each case sets upstream `restrictfilenames`, `windowsfilenames`, `trim_file_name`
and `outtmpl` (or the default template) and calls actual
`YoutubeDL.prepare_filename`. Generated on a non-Windows host; names are
compared exactly.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'fa9886678eb65103303c627b8e013f13f8999b41'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/sanitize/cases.json'
UPSTREAM_WORKER = r'''
import json, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL

def forbidden(*args, **kwargs):
    raise AssertionError('Network/download/process forbidden in sanitize comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
YoutubeDL.urlopen = YoutubeDL.download = forbidden
if sys.platform == 'win32':
    raise SystemExit('sanitize fixture is defined for non-Windows upstream behavior')
fixture = json.load(sys.stdin)
result = []
for case in fixture['cases']:
    params = {'quiet': True, 'no_warnings': True, **case['params']}
    if case['output'] is not None:
        params['outtmpl'] = case['output']
    ydl = YoutubeDL(params, auto_init=False)
    info = dict(fixture['info'], title=case['title'])
    result.append({'name': case['name'], 'filename': ydl.prepare_filename(info)})
print(json.dumps(result, ensure_ascii=False))
'''


def compare(result, fixture):
    target = [{'name': case['name'], 'filename': case['expected']} for case in fixture['cases']]
    if result != target:
        raise ValueError('filename mismatch: ' + json.dumps(
            {'upstream': result, 'expected': target}, ensure_ascii=False, sort_keys=True))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    args = parser.parse_args()
    try:
        upstream = args.upstream.resolve()
        if not (upstream / 'yt_dlp/YoutubeDL.py').is_file():
            raise ValueError('--upstream must name an official yt-dlp source checkout')
        revision = subprocess.run(['git', '-C', str(upstream), 'rev-parse', 'HEAD'],
                                  capture_output=True, text=True, encoding='utf-8', timeout=15,
                                  check=True).stdout.strip()
        if revision != PINNED_UPSTREAM:
            raise ValueError(f'--upstream must be checked out at {PINNED_UPSTREAM}')
        fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        if fixture['upstream_commit'] != PINNED_UPSTREAM:
            raise ValueError('sanitize fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Sanitize {len(fixture['cases'])} cases match pinned upstream "
          '(actual prepare_filename; exact names)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
