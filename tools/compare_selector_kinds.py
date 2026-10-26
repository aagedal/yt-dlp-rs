#!/usr/bin/env python3
"""Compare selector kinds with actual pinned upstream selection per context.

Development-only offline reference. Each saved VideoInfo-shaped context (a
muxed format beside separate streams, only video streams, only audio streams)
is fed to YoutubeDL.build_format_selector with upstream's has_merged_format and
incomplete_formats flags computed from its codecs. Formats are listed in
ascending order consistent with native ranking; no sorter, network or download
is used. Covers best/worst complete-stream rules and the incomplete fallback,
bv/ba, starred bv*/ba*/b*/w*, extension selectors and duplicate-stream merges,
plus the merger default `bestvideo*+bestaudio/best`. Selected IDs only.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'fa9886678eb65103303c627b8e013f13f8999b41'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/selectors/kinds.json'
UPSTREAM_WORKER = r'''
import copy, json, socket, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL

def forbidden(*args, **kwargs):
    raise AssertionError('Network access forbidden in selector kind comparison')
socket.socket = socket.create_connection = forbidden
fixture = json.load(sys.stdin)
result = []
with YoutubeDL({'quiet': True, 'no_warnings': True, 'cachedir': False,
                'check_formats': False}) as ydl:
    ydl.urlopen = forbidden
    for context in fixture['contexts']:
        formats = context['info']['formats']
        ctx = {'formats': formats,
               'has_merged_format': any('none' not in (f.get('acodec'), f.get('vcodec')) for f in formats),
               'incomplete_formats': (all(f.get('vcodec') != 'none' and f.get('acodec') == 'none' for f in formats)
                                      or all(f.get('vcodec') == 'none' and f.get('acodec') != 'none' for f in formats))}
        selected = {}
        for selector in fixture['selectors']:
            results = list(ydl.build_format_selector(selector)(copy.deepcopy(ctx)))
            selected[selector] = [] if not results else [
                fmt['format_id'] for fmt in results[0].get('requested_formats', (results[0],))]
        result.append({'name': context['name'], 'selected': selected})
print(json.dumps(result))
'''


def compare(result, fixture):
    expected = [{'name': context['name'], 'selected': context['expected']}
                for context in fixture['contexts']]
    if result != expected:
        raise ValueError('selector kind mismatch: ' + json.dumps(
            {'upstream': result, 'expected': expected}, sort_keys=True))
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
            raise ValueError('selector kind fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    count = len(fixture['contexts']) * len(fixture['selectors'])
    print(f'Selector kinds: {count} context/selector cases match pinned upstream '
          '(actual build_format_selector; selected IDs only)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
