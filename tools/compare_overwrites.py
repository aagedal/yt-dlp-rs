#!/usr/bin/env python3
"""Check saved overwrite cases with pinned actual upstream `process_info`.

Development only; no network/process access in the reference worker. Each case's
flags are parsed by actual `parseOpts` (with `__init__`'s --force-overwrites
implies --no-continue rule); existing files are created in a temporary directory
and actual `YoutubeDL.process_info` runs with only `HttpFD.real_download`
stubbed, so upstream's existing-file, continue, info JSON and subtitle overwrite
decisions apply. Download calls, old/new file states and archive records are
compared exactly.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'fa9886678eb65103303c627b8e013f13f8999b41'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/overwrites/cases.json'
UPSTREAM_WORKER = r'''
import json, os, socket, subprocess, sys, tempfile
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.options import parseOpts
from yt_dlp.downloader.http import HttpFD

def forbidden(*args, **kwargs):
    raise AssertionError('Network/process forbidden in overwrite comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
calls = []
def real_download(self, filename, info_dict):
    calls.append(os.path.basename(filename))
    with open(filename, 'w') as file:
        file.write('new')
    return True
HttpFD.real_download = real_download
fixture = json.load(sys.stdin)
real_stdout, sys.stdout = sys.stdout, sys.stderr
result = []
for case in fixture['cases']:
    _, opts, _ = parseOpts([*case['args'], 'https://example.invalid/'])
    if opts.overwrites:  # __init__: --force-overwrites implies --no-continue
        opts.continue_dl = False
    calls.clear()
    with tempfile.TemporaryDirectory() as directory:
        for name in case['existing']:
            with open(os.path.join(directory, name), 'w') as file:
                file.write('old')
        archive = os.path.join(directory, 'archive.txt')
        params = {'quiet': True, 'no_warnings': True, 'overwrites': opts.overwrites,
                  'continuedl': opts.continue_dl,
                  'outtmpl': os.path.join(directory, 'video.%(ext)s'), 'writeinfojson': True,
                  'writesubtitles': True, 'download_archive': archive}
        if params['overwrites'] is None:
            del params['overwrites']
        ydl = YoutubeDL(params, auto_init=False)
        ydl.sanitize_info = lambda info, *args: {'id': info['id']}
        info = {'id': 'v', 'title': 't', 'ext': 'mp4', 'url': 'https://example.invalid/v.mp4',
                'extractor': 'generic', 'extractor_key': 'Generic',
                'webpage_url': 'https://example.invalid/', 'format_id': '0', 'protocol': 'https',
                'requested_subtitles': {'nb': {'ext': 'vtt', 'data': 'WEBVTT\n'}}}
        ydl.process_info(info)
        ydl.record_download_archive(info)  # recorded after a successful process_info
        files = {}
        for name in sorted(os.listdir(directory)):
            if name != 'archive.txt':
                with open(os.path.join(directory, name)) as file:
                    files[name] = 'old' if file.read() == 'old' else 'new'
        result.append({'name': case['name'], 'media_downloaded': calls == ['video.mp4'],
                       'files': files, 'archived': os.path.exists(archive)})
real_stdout.write(json.dumps(result) + "\n")
'''


def compare(result, fixture):
    target = [{'name': case['name'], 'media_downloaded': case['media_downloaded'],
               'files': case['files'], 'archived': case['archived']} for case in fixture['cases']]
    if result != target:
        raise ValueError('overwrite mismatch: ' + json.dumps(
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
            raise ValueError('overwrite fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Overwrite {len(fixture['cases'])} cases match pinned upstream "
          '(actual parseOpts/process_info; exact file states)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
