#!/usr/bin/env python3
"""Compare saved HTML5 subtitle metadata and selection with actual pinned upstream.

Development only: actual HTML parser, process_subtitles, subtitles_filename and
option evaluation. No network, download, subprocess or runtime Python fallback.
Ordered language maps become arrays to preserve None separately from string keys.
URL sanitation and ext completion use the actual process_video_result pipeline.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'fa9886678eb65103303c627b8e013f13f8999b41'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/subtitles/cases.json'
UPSTREAM_WORKER = r'''
import copy, json, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL, parse_options
from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import determine_ext, sanitize_url, subtitles_filename

def forbidden(*args, **kwargs):
    raise AssertionError('Network/download/process forbidden in subtitle comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
YoutubeDL.urlopen = YoutubeDL.download = forbidden
InfoExtractor._download_webpage = InfoExtractor._extract_m3u8_formats = InfoExtractor._extract_mpd_formats = forbidden
fixture = json.load(sys.stdin)
ydl = YoutubeDL({'quiet':True, 'no_warnings':True}, auto_init=False)
ie = InfoExtractor(ydl)
result = {'html_cases': [], 'selection_cases': [], 'filename_cases': [], 'source_only_cases': [], 'option_cases': []}
for case in fixture['html_cases']:
    entries = ie._parse_html5_media_entries(case['page_url'], case['html'], 'fixture')
    normalized = []
    for entry in entries:
        subtitles = [{'language':lang, 'formats':[{'url':sanitize_url(f['url']), 'ext':determine_ext(sanitize_url(f['url'])).lower()} for f in formats]}
                     for lang, formats in entry['subtitles'].items()]
        normalized.append({'subtitles': subtitles})
    result['html_cases'].append({'name':case['name'], 'entries':normalized})
for key in ('selection_cases', 'source_only_cases'):
    for case in fixture[key]:
        warnings = []
        options = {'writesubtitles':case.get('write',True), 'subtitleslangs':case['languages'].split(',') if case['languages'] else [],
                   'subtitlesformat':case['formats'], 'quiet':True}
        probe = YoutubeDL(options, auto_init=False)
        probe.to_screen = lambda *args, **kwargs: None
        probe.report_warning = lambda *args, **kwargs: warnings.append(args[0])
        subtitles = {entry['language']:entry['formats'] for entry in case['subtitles']}
        try:
            selected = probe.process_subtitles('fixture', subtitles, None)
            record = {'name':case['name'], 'selected':[{'language':lang, **f} for lang,f in (selected or {}).items()], 'warning':bool(warnings)}
        except (TypeError, AttributeError) as error:
            if key != 'source_only_cases': raise
            record = {'name':case['name'], 'error':type(error).__name__}
        result[key].append(record)
for case in fixture['filename_cases']:
    result['filename_cases'].append({'name':case['name'], 'filename':subtitles_filename(case['filename'],case['language'],case['ext'],case['media_ext'])})
for case in fixture['option_cases']:
    params = parse_options(case['argv']).ydl_opts
    probe = YoutubeDL(params, auto_init=False)
    result['option_cases'].append({'name':case['name'], 'simulate':probe.params.get('simulate'),
                                  'skip_download':probe.params.get('skip_download'), 'write':probe.params.get('writesubtitles'),
                                  'list':probe.params.get('listsubtitles')})
print(json.dumps(result, ensure_ascii=False))
'''

def compare(result, fixture):
    target = {key: [{'name':case['name'], **case['expected']} for case in fixture[key]]
              for key in ('html_cases', 'selection_cases', 'filename_cases', 'source_only_cases', 'option_cases')}
    if result != target:
        raise ValueError('subtitle rendering mismatch: ' + json.dumps(
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
            raise ValueError('subtitle fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Subtitles {len(fixture['html_cases'])} HTML metadata, {len(fixture['selection_cases'])} selection, "
          f"{len(fixture['filename_cases'])} filename, {len(fixture['option_cases'])} option and "
          f"{len(fixture['source_only_cases'])} source-only cases match actual pinned upstream")
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
