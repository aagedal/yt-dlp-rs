#!/usr/bin/env python3
"""Check saved --print/--get-* cases with pinned actual upstream forced printing.

Development only; no network/process/download access in the reference worker.
The worker parses each case's arguments with upstream `parseOpts` and calls the
actual private `YoutubeDL.__forced_printings` (which uses `_forceprint`) with a
saved prepared filename. Printed lines are compared exactly, without
normalization. Source-only cases document behavior the native port rejects.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = '6ccd977303412151ee23352c6257dd658ffc1987'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/print/cases.json'
UPSTREAM_WORKER = r'''
import copy, json, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.options import parseOpts

def forbidden(*args, **kwargs):
    raise AssertionError('Network/download/process forbidden in print comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
YoutubeDL.urlopen = YoutubeDL.download = forbidden
fixture = json.load(sys.stdin)
result = {}
for key in ('cases', 'source_only_cases'):
    result[key] = []
    for case in fixture[key]:
        _, opts, _ = parseOpts([*case['args'], 'https://example.invalid/'])
        ydl = YoutubeDL({
            'quiet': True, 'no_warnings': True,
            'outtmpl_na_placeholder': case['na_placeholder'],
            'forceprint': opts.forceprint, 'print_to_file': opts.print_to_file,
            'forcetitle': opts.gettitle, 'forceid': opts.getid,
            'forceurl': opts.geturl, 'forcefilename': opts.getfilename,
        }, auto_init=False)
        lines = []
        ydl.to_stdout = lines.append
        info = copy.deepcopy(case['info'])
        info['format'] = info['ext']  # process_info: complete, not incomplete
        ydl._YoutubeDL__forced_printings(info, case['filename'], incomplete=False)
        result[key].append({'name': case['name'], 'lines': lines})
print(json.dumps(result, ensure_ascii=False))
'''


def compare(result, fixture):
    target = {key: [{'name': case['name'], 'lines': case['expected']} for case in fixture[key]]
              for key in ('cases', 'source_only_cases')}
    if result != target:
        raise ValueError('print output mismatch: ' + json.dumps(
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
            raise ValueError('print fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Print {len(fixture['cases'])} shared cases and "
          f"{len(fixture['source_only_cases'])} source-only cases match pinned upstream "
          '(actual forced printing; exact lines, no normalization)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
