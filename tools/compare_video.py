#!/usr/bin/env python3
"""Compare video remux/recode plans against actual pinned postprocessor runs.

Offline development reference only. Actual parse_options and both video run
methods execute with virtual files and captured FFmpeg calls. No network, media
reads, subprocesses, or FFmpeg execution are permitted inside the worker.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = '6ccd977303412151ee23352c6257dd658ffc1987'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/video/cases.json'
UPSTREAM_WORKER = r'''
import copy,json,os,socket,subprocess,sys
from optparse import OptParseError
sys.path.insert(0,sys.argv[1])
from yt_dlp import parse_options
from yt_dlp.postprocessor.ffmpeg import FFmpegVideoRemuxerPP,FFmpegVideoConvertorPP

def forbidden(*args,**kwargs):
    raise AssertionError('Network/process/file mutation forbidden in video comparison')
socket.socket=socket.create_connection=forbidden
subprocess.Popen=forbidden
os.replace=os.rename=os.remove=os.unlink=forbidden
fixture=json.load(sys.stdin)
result={'cases':[],'source_only_cases':[],'invalid_cases':[]}
for key in ('cases','source_only_cases'):
    for case in fixture[key]:
        parsed=parse_options(case['args'])
        options=parsed.ydl_opts
        processor=next(p for p in options['postprocessors'] if p['key'] in ('FFmpegVideoRemuxer','FFmpegVideoConvertor'))
        mode='remux' if processor['key']=='FFmpegVideoRemuxer' else 'recode'
        base=FFmpegVideoRemuxerPP if mode=='remux' else FFmpegVideoConvertorPP
        calls=[]
        class FixturePP(base):
            def _determine_executables(self):
                return {}
            def run_ffmpeg(self,path,out_path,opts):
                calls.append({'input':path,'output':out_path,'args':list(opts)})
        pp=FixturePP(preferedformat=processor['preferedformat'])
        info={'filepath':case['source']['filepath'],'ext':case['source']['ext'],'format':'fixture','vcodec':'avc1','acodec':'aac'}
        delete,updated=pp.run(copy.deepcopy(info))
        result[key].append({'name':case['name'],'mode':mode,'mapping':processor['preferedformat'],
            'selector':options.get('format'),'keep_video':options.get('keepvideo'),
            'extract_audio':any(p['key']=='FFmpegExtractAudio' for p in options['postprocessors']),
            'warnings':options.get('_warnings',[]),'output_ext':updated['ext'],
            'output_path':updated['filepath'],'output_format':updated['format'],
            'calls':calls,'delete':delete})
for case in fixture['invalid_cases']:
    try:parse_options(case['args'])
    except OptParseError:result['invalid_cases'].append(case['name'])
    else:raise AssertionError('Expected source option rejection: '+case['name'])
print(json.dumps(result,ensure_ascii=False))
'''


def compare(result, fixture):
    target = {key: [{'name': case['name'], **case['expected']} for case in fixture[key]]
              for key in ('cases', 'source_only_cases')}
    target['invalid_cases'] = [case['name'] for case in fixture['invalid_cases']]
    if result != target:
        raise ValueError('video plan mismatch: ' + json.dumps(
            {'actual': result, 'expected': target}, ensure_ascii=False, sort_keys=True))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    args = parser.parse_args()
    try:
        upstream = args.upstream.resolve()
        if not (upstream / 'yt_dlp/postprocessor/ffmpeg.py').is_file():
            raise ValueError('--upstream must name an official yt-dlp source checkout')
        revision = subprocess.run(['git', '-C', str(upstream), 'rev-parse', 'HEAD'],
                                  capture_output=True, text=True, encoding='utf-8',
                                  timeout=15, check=True).stdout.strip()
        if revision != PINNED_UPSTREAM:
            raise ValueError(f'--upstream must be checked out at {PINNED_UPSTREAM}')
        fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        if fixture['upstream_commit'] != PINNED_UPSTREAM:
            raise ValueError('video fixture does not match pinned upstream commit')
        completed = subprocess.run(
            [sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
            input=json.dumps(fixture), capture_output=True, text=True,
            encoding='utf-8', timeout=30, check=True)
        compare(json.loads(completed.stdout), fixture)
    except (OSError, ValueError, KeyError, StopIteration, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Video {len(fixture['cases'])} plans, {len(fixture['source_only_cases'])} source-only "
          f"and {len(fixture['invalid_cases'])} invalid cases match actual pinned postprocessors "
          '(exact options/deletions; virtual files; no FFmpeg execution)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
