#!/usr/bin/env python3
"""Check saved codec cases with pinned actual upstream helpers.

Development only; no network/process access in the reference worker. Each
input is parsed by actual `yt_dlp.utils.parse_codecs` (its "Unknown codec"
warnings are captured), and each container case is resolved by actual
`get_compatible_ext`. Results are compared exactly, including key order of
the returned dictionaries.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = '6ccd977303412151ee23352c6257dd658ffc1987'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/codecs/cases.json'
UPSTREAM_WORKER = r'''
import contextlib, io, json, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp.utils import _utils, get_compatible_ext, parse_codecs

def forbidden(*args, **kwargs):
    raise AssertionError('Network/process forbidden in codec comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
fixture = json.load(sys.stdin)
result = {'parse_codecs': [], 'compatible_ext': []}
for case in fixture['parse_codecs']:
    warnings = io.StringIO()
    with contextlib.redirect_stderr(warnings):
        parsed = parse_codecs(case['input'])
    result['parse_codecs'].append({'input': case['input'], 'result': parsed,
                                   'warnings': warnings.getvalue().splitlines()})
for case in fixture['compatible_ext']:
    entry = {key: case[key] for key in ('vcodecs', 'acodecs', 'vexts', 'aexts', 'preferences')}
    entry['ext'] = get_compatible_ext(**entry)
    result['compatible_ext'].append(entry)
print(json.dumps(result, ensure_ascii=False))
'''


def compare(result, fixture):
    target = {key: fixture[key] for key in ('parse_codecs', 'compatible_ext')}
    if result != target:
        raise ValueError('codec mismatch: ' + json.dumps(
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
            raise ValueError('codec fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Codecs {len(fixture['parse_codecs'])} inputs and {len(fixture['compatible_ext'])} "
          'container cases match pinned upstream (actual parse_codecs/get_compatible_ext)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
