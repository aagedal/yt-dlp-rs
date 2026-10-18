#!/usr/bin/env python3
"""Check saved info JSON cases with pinned actual upstream helpers, offline.

Development only; the reference worker forbids network/process access and only
writes inside its own temporary directory. It runs actual replace_extension,
prepare_filename(..., 'pl_infojson') over _playlist_infodict, sanitize_info
with clean_infojson and write_json_file. Text and dictionaries are compared
exactly, including key order. Documented rule: upstream's `_version` is
removed before comparison because it describes upstream's own release.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = '897341241f2bfb2439fe0367f4c7c9ccd29a3828'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/info_json/cases.json'
UPSTREAM_WORKER = r'''
import collections, copy, json, os, socket, subprocess, sys, tempfile
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.utils import replace_extension, write_json_file

def forbidden(*args, **kwargs):
    raise AssertionError('Network/download/process forbidden in info JSON comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
YoutubeDL.urlopen = YoutubeDL.download = forbidden
fixture = json.load(sys.stdin)
result = {'filenames': [], 'playlist_names': [], 'sanitize': [], 'json_text': [], 'source_only': []}
for case in fixture['filenames']:
    result['filenames'].append({'name': case['name'],
                                'expected': replace_extension(case['filename'], 'info.json', case['ext'])})
for case in fixture['playlist_names']:
    ydl = YoutubeDL({'quiet': True, 'no_warnings': True, 'outtmpl': case['template'],
                     'outtmpl_na_placeholder': case['na_placeholder']}, auto_init=False)
    requested = list(range(1, case['n_entries'])) + [case['last_playlist_index']] if case['n_entries'] else []
    playlist = dict(copy.deepcopy(case['playlist']), _type='playlist', entries=[],
                    requested_entries=requested, playlist_count=case['playlist_count'])
    extra = YoutubeDL._playlist_infodict(playlist, n_entries=case['n_entries'])
    result['playlist_names'].append({'name': case['name'], 'expected': ydl.prepare_filename(
        collections.ChainMap(playlist, extra), 'pl_infojson')})
for case in fixture['sanitize']:
    value = YoutubeDL.sanitize_info(copy.deepcopy(case['info']), True)
    value.pop('_version')
    result['sanitize'].append({'name': case['name'], 'expected': value})
with tempfile.TemporaryDirectory() as directory:
    path = os.path.join(directory, 'case.info.json')
    for case in fixture['json_text']:
        write_json_file(case['value'], path)
        with open(path, encoding='utf-8') as file:
            result['json_text'].append({'name': case['name'], 'expected': file.read()})
for case in fixture['source_only']:
    value = YoutubeDL.sanitize_info(copy.deepcopy(case['info']), True)
    result['source_only'].append({'name': case['name'],
                                  'upstream_extra_key': case['upstream_extra_key'] if case['upstream_extra_key'] in value else None})
print(json.dumps(result, ensure_ascii=False))
'''
SECTIONS = ('filenames', 'playlist_names', 'sanitize', 'json_text')


def expected(fixture):
    target = {key: [{'name': case['name'], 'expected': case['expected']} for case in fixture[key]]
              for key in SECTIONS}
    target['source_only'] = [{'name': case['name'], 'upstream_extra_key': case['upstream_extra_key']}
                             for case in fixture['source_only']]
    return target


def compare(result, fixture):
    target = expected(fixture)
    # Lists of dicts compare key order only through serialization.
    if json.dumps(result, ensure_ascii=False) != json.dumps(target, ensure_ascii=False):
        raise ValueError('info JSON mismatch: ' + json.dumps(
            {'upstream': result, 'expected': target}, ensure_ascii=False))
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
            raise ValueError('info JSON fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-I', '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    counts = ', '.join(f'{len(fixture[key])} {key}' for key in SECTIONS)
    print(f'Info JSON {counts} and {len(fixture["source_only"])} source-only cases match pinned '
          'upstream (actual replace_extension/prepare_filename/sanitize_info/write_json_file)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
