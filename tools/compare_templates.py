#!/usr/bin/env python3
"""Check saved named output templates with pinned actual evaluate_outtmpl.

Development only; no network/process/download access in the reference worker.
Exact rendered strings are compared without normalization. Shared cases use
sanitization-neutral values and supported root fields; source-only cases make
native boundaries explicit. No filename sanitation or whole-template claim.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'fa9886678eb65103303c627b8e013f13f8999b41'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/templates/cases.json'
UPSTREAM_WORKER = r'''
import copy, json, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL

def forbidden(*args, **kwargs):
    raise AssertionError('Network/download/process forbidden in template comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
YoutubeDL.urlopen = YoutubeDL.download = forbidden
fixture = json.load(sys.stdin)
result = {}
for key in ('cases', 'source_only_cases'):
    result[key] = []
    for case in fixture[key]:
        ydl = YoutubeDL({'quiet': True, 'no_warnings': True,
                        'outtmpl_na_placeholder': case['na_placeholder'],
                        'compat_opts': case.get('compat_opts', [])}, auto_init=False)
        rendered = ydl.evaluate_outtmpl(case['template'], copy.deepcopy(case['info']), sanitize=True)
        result[key].append({'name': case['name'], 'rendered': rendered})
print(json.dumps(result, ensure_ascii=False))
'''


def compare(result, fixture):
    target = {key: [{'name': case['name'], 'rendered': case['expected']} for case in fixture[key]]
              for key in ('cases', 'source_only_cases')}
    if result != target:
        raise ValueError('template rendering mismatch: ' + json.dumps(
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
            raise ValueError('template fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Templates {len(fixture['cases'])} shared cases and "
          f"{len(fixture['source_only_cases'])} source-only cases match pinned upstream "
          '(actual evaluate_outtmpl; exact strings, no normalization)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
