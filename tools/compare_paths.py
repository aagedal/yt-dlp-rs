#!/usr/bin/env python3
"""Check saved typed output template/path cases with pinned actual upstream.

Development only; no network/process/download access in the reference worker.
Each case's `-o`/`-P`/filename arguments are parsed by actual `parseOpts`, then
actual `YoutubeDL.prepare_filename` renders media, `subtitle`, `infojson` and
playlist `pl_infojson` names under the fixture's HOME and variables. Defined for
non-Windows upstream behavior; names are compared exactly.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'f264a6cbd1f630894da0f368df00f8aae50140e0'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/paths/cases.json'
UPSTREAM_WORKER = r'''
import json, os, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.options import parseOpts

def forbidden(*args, **kwargs):
    raise AssertionError('Network/download/process forbidden in paths comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
YoutubeDL.urlopen = YoutubeDL.download = forbidden
if sys.platform == 'win32':
    raise SystemExit('paths fixture is defined for non-Windows upstream behavior')
fixture = json.load(sys.stdin)
os.environ.update(fixture['env'])
result = []
for case in fixture['cases']:
    _, opts, _ = parseOpts([*case['args'], 'https://example.invalid/'])
    outtmpl = dict(opts.outtmpl)
    if outtmpl.get('default') == '':
        del outtmpl['default']  # __init__ treats an empty default as unset
    ydl = YoutubeDL({'quiet': True, 'no_warnings': True, 'outtmpl': outtmpl,
                     'paths': opts.paths, 'restrictfilenames': opts.restrictfilenames,
                     'windowsfilenames': opts.windowsfilenames,
                     'trim_file_name': opts.trim_file_name}, auto_init=False)
    names = {kind or 'media': ydl.prepare_filename(dict(fixture['info']), kind)
             for kind in ('', 'subtitle', 'infojson')}
    names['pl_infojson'] = ydl.prepare_filename(dict(fixture['playlist']), 'pl_infojson')
    result.append({'name': case['name'], 'names': names})
print(json.dumps(result, ensure_ascii=False))
'''


def compare(result, fixture):
    target = [{'name': case['name'], 'names': case['expected']} for case in fixture['cases']]
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
            raise ValueError('paths fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Paths {len(fixture['cases'])} cases x 4 output kinds match pinned upstream "
          '(actual parseOpts/prepare_filename; exact names)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
