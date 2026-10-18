#!/usr/bin/env python3
"""Compare saved subtitle embedding plans with actual pinned upstream, offline.

Actual parse_options, FFmpegEmbedSubtitlePP.run, ISO639Utils.short2long and
YoutubeDL.run_pp execute. FFmpeg calls, replacement and deletion are captured;
source existence is virtual. No network, subprocess or media mutation occurs.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = '897341241f2bfb2439fe0367f4c7c9ccd29a3828'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/subtitle_embedding/cases.json'
UPSTREAM_WORKER = r'''
import builtins,copy,json,os,socket,subprocess,sys
from types import SimpleNamespace
from optparse import OptParseError
sys.path.insert(0,sys.argv[1])
from yt_dlp import YoutubeDL,parse_options
from yt_dlp.postprocessor.ffmpeg import FFmpegEmbedSubtitlePP
from yt_dlp.utils import ISO639Utils

def forbidden(*args,**kwargs):
    raise AssertionError('Network/process/nonvirtual mutation forbidden in subtitle embedding comparison')
socket.socket=socket.create_connection=forbidden
subprocess.Popen=forbidden
os.rename=os.remove=os.unlink=forbidden
fixture=json.load(sys.stdin)
result={'cases':[],'source_only_cases':[],'language_cases':[],'invalid_cases':[]}
for case in fixture['language_cases']:
    result['language_cases'].append({'input':case['input'],'expected':ISO639Utils.short2long(case['input']) or case['input']})
real_exists=os.path.exists
real_open=builtins.open
for key in ('cases','source_only_cases'):
    for case in fixture[key]:
        parsed=parse_options(case['args']);options=parsed.ydl_opts
        processor=next((p for p in options['postprocessors'] if p['key']=='FFmpegEmbedSubtitle'),None)
        requested=None if case['subtitles'] is None else {}
        existing={case['source']['filepath']}
        for sub in case['subtitles'] or []:
            requested[sub['language']]={k:v for k,v in sub.items() if k in ('ext','filepath','name')}
            if not sub.get('missing'):existing.add(sub['filepath'])
        calls,warnings,messages,replacements,deletions,deleted=[],[],[],[],[],[]
        def replace(source,destination):
            assert source in existing
            existing.remove(source);existing.add(destination);replacements.append([source,destination])
        def delete(*paths,**kwargs):
            deleted.extend(paths)
            for path in paths:existing.discard(path)
        class FixturePP(FFmpegEmbedSubtitlePP):
            def _determine_executables(self):return {}
            def report_warning(self,message,*args,**kwargs):warnings.append(message)
            def to_screen(self,message,*args,**kwargs):messages.append(message)
            def run_ffmpeg_multiple_files(self,inputs,output,args):
                assert all(path in existing for path in inputs)
                calls.append({'inputs':list(inputs),'output':output,'args':list(args)})
                existing.add(output)
            def run(self,info):
                delete,updated=super().run(info);deletions.extend(delete)
                return delete,updated
        info={'filepath':case['source']['filepath'],'ext':case['source']['ext'],
              'requested_subtitles':requested,'__files_to_move':{}}
        already=case.get('already_have_subtitle',processor['already_have_subtitle'] if processor else None)
        os.path.exists=lambda path:os.fspath(path) in existing
        os.replace=replace
        builtins.open=forbidden
        try:
            if processor:
                pp=FixturePP(already_have_subtitle=already)
                probe=SimpleNamespace(params=options,_delete_downloaded_files=delete)
                updated=YoutubeDL.run_pp(probe,pp,copy.deepcopy(info))
            else:updated=info
        finally:
            os.path.exists=real_exists;os.replace=forbidden;builtins.open=real_open
        result[key].append({'name':case['name'],'enabled':bool(processor),
            'write_subtitles':options.get('writesubtitles'),'write_automatic':options.get('writeautomaticsub'),
            'already_have_subtitle':already,'keep_video':options.get('keepvideo'),
            'calls':calls,'warnings':warnings,'messages':messages,'delete':deletions,
            'deleted_by_run_pp':deleted,'replacements':replacements,'files_to_move':updated['__files_to_move']})
for case in fixture['invalid_cases']:
    try:parse_options(case['args'])
    except OptParseError:result['invalid_cases'].append(case['name'])
    else:raise AssertionError('Expected source option rejection: '+case['name'])
print(json.dumps(result,ensure_ascii=False))
'''


def compare(result, fixture):
    target = {key: [{'name': case['name'], **case['expected']} for case in fixture[key]]
              for key in ('cases', 'source_only_cases')}
    target['language_cases'] = fixture['language_cases']
    target['invalid_cases'] = [case['name'] for case in fixture['invalid_cases']]
    if result != target:
        raise ValueError('subtitle embedding mismatch: ' + json.dumps(
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
            raise ValueError('subtitle embedding fixture does not match pinned upstream commit')
        completed = subprocess.run(
            [sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
            input=json.dumps(fixture), capture_output=True, text=True,
            encoding='utf-8', timeout=30, check=True)
        compare(json.loads(completed.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Subtitle embedding {len(fixture['cases'])} plans, "
          f"{len(fixture['source_only_cases'])} source-only and {len(fixture['language_cases'])} "
          'language projections match actual pinned source (virtual files; no FFmpeg execution)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
