#!/usr/bin/env python3
"""Compare saved YouTube WEB captions using actual pinned YoutubeIE._real_extract.

Offline development reference only. Saved player responses, actual caption loop,
WEB client/subtitle token policy, no token overrides; video format extraction is
stubbed for this isolated captions comparison. Network/process/download forbidden.
Compare ordered maps as arrays and exact URL/ext/name, including source URL
sanitation. No decipher, video policy or payload claim; selection compares explicit saved maps.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'fa9886678eb65103303c627b8e013f13f8999b41'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/youtube/captions/cases.json'
UPSTREAM_WORKER = r'''
import copy, json, socket, subprocess, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.extractor.youtube import YoutubeIE, _video
from yt_dlp.utils import sanitize_url

def forbidden(*args, **kwargs):
    raise AssertionError('Network/download/process forbidden in YouTube caption comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
YoutubeDL.urlopen = YoutubeDL.download = forbidden
root = Path(sys.argv[2])
fixture = json.load(sys.stdin)
result = {'cases':[], 'source_only_cases':[]}
for key in result:
    for case in fixture[key]:
        player = json.loads((root / case['player']).read_text(encoding='utf-8'))
        token_requests, warnings, skipped = [], [], []
        def token_provider(**kwargs):
            token_requests.append(kwargs)
            return None
        class FixtureYoutubeIE(YoutubeIE):
            _request_webpage = forbidden
            _extract_m3u8_formats_and_subtitles = _extract_mpd_formats_and_subtitles = forbidden
            @classmethod
            def ie_key(cls):
                return YoutubeIE.ie_key()
            def _initial_extract(self, *args):
                player['streamingData'].update({
                    _video.STREAMING_DATA_CLIENT_NAME:'web',
                    _video.STREAMING_DATA_INNERTUBE_CONTEXT:{'client':{'clientName':'WEB'}},
                    _video.STREAMING_DATA_FETCH_SUBS_PO_TOKEN:token_provider,
                    _video.STREAMING_DATA_PLAYER_TOKEN_PROVIDED:False,
                    _video.STREAMING_DATA_IS_PREMIUM_SUBSCRIBER:False,
                })
                return '', {}, {}, False, [player], None
            def _extract_formats_and_subtitles(self, *args):
                # Caption-only reference: no video policy, decipher or transport claim.
                yield {'format_id':'18', 'url':'https://example.test/media.mp4', 'ext':'mp4',
                       'vcodec':'avc1.42001E', 'acodec':'mp4a.40.2'}
                yield {}
            def _report_pot_subtitles_skipped(self, *args, **kwargs):
                skipped.append(True)
                return super()._report_pot_subtitles_skipped(*args, **kwargs)
        params = {'quiet':True,'no_warnings':True,'cachedir':False,'skip_download':True,
                  'writeautomaticsub':case['translated_manual'] and case.get('translated_via') != 'list',
                  'listsubtitles':case.get('translated_via') == 'list',
                  'extractor_args':{'youtube':{'player_skip':['js','initial_data']}}}
        with YoutubeDL(params) as ydl:
            ie = FixtureYoutubeIE(ydl)
            ie.report_warning = lambda *args,**kwargs:warnings.append(args[0])
            ie.initialize()
            info = ie._real_extract('https://www.youtube.com/watch?v=BaW_jenozKc')
        def ordered(container):
            return [{'language':lang,'formats':[{'url':sanitize_url(f['url']),'ext':f['ext'],'name':f.get('name')} for f in formats]}
                    for lang,formats in container.items()]
        result[key].append({'name':case['name'], 'subtitles':ordered(info['subtitles']),
            'automatic_captions':ordered(info['automatic_captions']),
            'audio_language':info['formats'][0].get('language'),
            'token_requests':token_requests, 'warning':bool(warnings), 'policy_skip':bool(skipped)})
result['selection_cases'] = []
for case in fixture.get('selection_cases', []):
    params = {'writesubtitles':case['write_manual'], 'writeautomaticsub':case['write_auto'],
              'subtitleslangs':case['languages'].split(',') if case['languages'] else [],
              'subtitlesformat':case['format'], 'quiet':True}
    probe = YoutubeDL(params, auto_init=False)
    warnings = []
    probe.to_screen = lambda *args, **kwargs: None
    probe.report_warning = lambda *args, **kwargs: warnings.append(args[0])
    maps = [{group['language']:group['formats'] for group in case[field]} for field in ('subtitles','automatic_captions')]
    selected = probe.process_subtitles('fixture', *maps)
    result['selection_cases'].append({'name':case['name'], 'selected':[{'language':lang, **fmt} for lang,fmt in (selected or {}).items()], 'warning':bool(warnings)})
print(json.dumps(result,ensure_ascii=False))
'''

def compare(result, fixture):
    target = {key: [{'name':case['name'], **case['expected']} for case in fixture[key]]
              for key in ('cases', 'source_only_cases', 'selection_cases')}
    if result != target:
        raise ValueError('YouTube caption mismatch: ' + json.dumps(
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
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream), str(FIXTURE.parent)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"YouTube captions {len(fixture['cases'])} shared and {len(fixture['source_only_cases'])} "
          f"source-only plus {len(fixture['selection_cases'])} selection cases match actual pinned caption loop (exact URLs/names/order)")
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
