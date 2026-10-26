#!/usr/bin/env python3
"""Check saved format-sorting cases with pinned actual upstream FormatSorter.

Development only; no network/process access in the reference worker. Each
case's formats are filled by actual `FormatSorter._fill_sorting_fields` and
sorted with `formats.sort(key=FormatSorter(ydl, extractor_fields)
.calculate_preference)` under the case's `format_sort`, `format_sort_force`
and `prefer_free_formats` parameters, like `YoutubeDL.sort_formats`. The
filled dictionaries (what the Rust sorter reads) and the sorted format IDs are
compared exactly.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'fa9886678eb65103303c627b8e013f13f8999b41'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/format_sort/cases.json'
UPSTREAM_WORKER = r'''
import copy, json, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.utils import FormatSorter

def forbidden(*args, **kwargs):
    raise AssertionError('Network/process forbidden in format sort comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
fixture = json.load(sys.stdin)
result = []
for case in fixture['cases']:
    params = {'quiet': True, 'no_warnings': True, 'cachedir': False,
              'format_sort': case['sort'], 'format_sort_force': case['force'],
              'prefer_free_formats': case['free']}
    with YoutubeDL(params) as ydl:
        formats = copy.deepcopy(case['formats'])
        for fmt in formats:
            FormatSorter._fill_sorting_fields(fmt)
        filled = copy.deepcopy(formats)
        formats.sort(key=FormatSorter(ydl, case['extractor']).calculate_preference)
    result.append({'name': case['name'], 'filled': filled,
                   'sorted': [fmt['format_id'] for fmt in formats]})
print(json.dumps(result, ensure_ascii=False))
'''


def compare(result, fixture):
    target = [{key: case[key] for key in ('name', 'filled', 'sorted')} for case in fixture['cases']]
    if result != target:
        raise ValueError('format sort mismatch: ' + json.dumps(
            {'upstream': result, 'expected': target}, ensure_ascii=False))
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
            raise ValueError('format sort fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=60, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Format sorting {len(fixture['cases'])} cases match pinned upstream "
          '(actual FormatSorter._fill_sorting_fields/calculate_preference)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
