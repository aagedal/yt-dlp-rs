#!/usr/bin/env python3
"""Compare subtitle plans and TTML text with pinned actual upstream, offline.

Actual parse_options, FFmpegSubtitlesConvertorPP.run and dfxp2srt execute. Media
files and FFmpeg output are virtual; captured calls establish planning only.
TTML expected SRT is compared exactly, without whitespace or timing normalization.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'f264a6cbd1f630894da0f368df00f8aae50140e0'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/subtitle_conversion/cases.json'
UPSTREAM_WORKER = r'''
import builtins,copy,io,json,os,socket,subprocess,sys
from pathlib import Path
from optparse import OptParseError
sys.path.insert(0,sys.argv[1])
from yt_dlp import parse_options
from yt_dlp.postprocessor.ffmpeg import FFmpegSubtitlesConvertorPP
from yt_dlp.utils import dfxp2srt

def forbidden(*args,**kwargs):
    raise AssertionError('Network/process/nonvirtual file mutation forbidden in subtitle comparison')
socket.socket=socket.create_connection=forbidden
subprocess.Popen=forbidden
os.replace=os.rename=os.remove=os.unlink=forbidden
fixture=json.load(sys.stdin)
root=Path(sys.argv[2])
inputs={case['input_file']:(root/case['input_file']).read_bytes() for case in fixture['ttml_cases']}
result={'ttml_cases':[],'cases':[],'source_only_cases':[],'invalid_cases':[]}
for case in fixture['ttml_cases']:
    result['ttml_cases'].append({'name':case['name'],'srt':dfxp2srt(inputs[case['input_file']])})
real_open=builtins.open
real_exists=os.path.exists
for key in ('cases','source_only_cases'):
    for case in fixture[key]:
        parsed=parse_options(case['args'])
        options=parsed.ydl_opts
        processor=next((p for p in options['postprocessors'] if p['key']=='FFmpegSubtitlesConvertor'),None)
        files,calls,writes,warnings,messages={ },[],[],[],[]
        requested=None if case['subtitles'] is None else {}
        move={}
        for sub in case['subtitles'] or []:
            if not sub.get('missing'):
                files[sub['filepath']]=inputs[sub['input_file']] if sub.get('input_file') else sub.get('file_text','SOURCE SUBTITLE\n').encode('utf-8')
            requested[sub['language']]={k:v for k,v in sub.items() if k in ('ext','filepath','data','name')}
            move[sub['filepath']]='published/'+os.path.basename(sub['filepath'])
        class VirtualText(io.StringIO):
            def __init__(self,path):
                super().__init__();self.path=path
            def close(self):
                if not self.closed:
                    data=self.getvalue();files[self.path]=data.encode('utf-8');writes.append({'path':self.path,'data':data})
                super().close()
        def virtual_open(path,mode='r',encoding=None,**kwargs):
            path=os.fspath(path)
            if mode=='rb' and path in files:return io.BytesIO(files[path])
            if mode in ('r','rt') and path in files:return io.StringIO(files[path].decode(encoding or 'utf-8'))
            if mode=='w' and encoding=='utf-8':return VirtualText(path)
            raise AssertionError('Nonvirtual subtitle file access forbidden')
        class FixturePP(FFmpegSubtitlesConvertorPP):
            def _determine_executables(self):return {}
            def report_warning(self,message,*args,**kwargs):warnings.append(message)
            def to_screen(self,message,*args,**kwargs):messages.append(message)
            def run_ffmpeg(self,path,out_path,args):
                assert path in files
                calls.append({'input':path,'output':out_path,'args':list(args)})
                # Synthetic output establishes metadata propagation, not codec/text correctness.
                files[out_path]=('FFMPEG:'+args[-1]+'\n').encode('utf-8')
        info={'requested_subtitles':requested,'__files_to_move':move}
        builtins.open=virtual_open
        os.path.exists=lambda path:os.fspath(path) in files
        try:
            if processor:
                delete,updated=FixturePP(format=processor['format']).run(copy.deepcopy(info))
            else:delete,updated=[],info
        finally:
            builtins.open=real_open;os.path.exists=real_exists
        subs=updated['requested_subtitles']
        result[key].append({'name':case['name'],'target':processor['format'] if processor else None,
            'when':processor['when'] if processor else None,'selector':options.get('format'),
            'skip_download':options.get('skip_download'),'calls':calls,'warnings':warnings,'messages':messages,
            'subtitles':None if subs is None else [{'language':lang,**sub} for lang,sub in subs.items()],
            'delete':delete,'writes':writes,'files_to_move':updated['__files_to_move']})
for case in fixture['invalid_cases']:
    try:parse_options(case['args'])
    except OptParseError:result['invalid_cases'].append(case['name'])
    else:raise AssertionError('Expected source option rejection: '+case['name'])
print(json.dumps(result,ensure_ascii=False))
'''


def compare(result, fixture):
    target = {'ttml_cases': [{'name': case['name'], 'srt': case['expected_srt']}
                             for case in fixture['ttml_cases']]}
    target.update({key: [{'name': case['name'], **case['expected']} for case in fixture[key]]
                   for key in ('cases', 'source_only_cases')})
    target['invalid_cases'] = [case['name'] for case in fixture['invalid_cases']]
    if result != target:
        raise ValueError('subtitle conversion mismatch: ' + json.dumps(
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
            raise ValueError('subtitle fixture does not match pinned upstream commit')
        completed = subprocess.run(
            [sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream), str(FIXTURE.parent)],
            input=json.dumps(fixture), capture_output=True, text=True,
            encoding='utf-8', timeout=30, check=True)
        compare(json.loads(completed.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Subtitle conversion {len(fixture['ttml_cases'])} exact TTML texts, "
          f"{len(fixture['cases'])} plans, {len(fixture['source_only_cases'])} source-only "
          f"and {len(fixture['invalid_cases'])} invalid cases match actual pinned source "
          '(virtual files; no FFmpeg execution)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
