#!/usr/bin/env python3
"""Compare saved audio extraction plans with actual pinned FFmpegExtractAudioPP.

Offline development reference: actual option validation, resolve_mapping,
_quality_args and run. A subclass captures ffmpeg calls and fixed probe codecs;
virtual os.replace calls are recorded. No network, subprocess or media I/O.
Quality argument numeric projection permits equivalent decimal spelling only.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM='fa9886678eb65103303c627b8e013f13f8999b41'
FIXTURE=Path(__file__).resolve().parents[1]/'tests/fixtures/audio/cases.json'
UPSTREAM_WORKER=r'''
import copy,json,math,os,socket,subprocess,sys
from optparse import OptParseError
sys.path.insert(0,sys.argv[1])
from yt_dlp import parse_options
from yt_dlp.postprocessor.ffmpeg import FFmpegExtractAudioPP,resolve_mapping

def forbidden(*args,**kwargs):
    raise AssertionError('Network/process forbidden in audio comparison')
socket.socket=socket.create_connection=forbidden
subprocess.Popen=forbidden
fixture=json.load(sys.stdin)
result={'cases':[],'source_only_cases':[],'invalid_cases':[]}
for key in ('cases','source_only_cases'):
    for case in fixture[key]:
        parsed=parse_options(case['args'])
        options=parsed.ydl_opts
        processor=next((p for p in options['postprocessors'] if p['key']=='FFmpegExtractAudio'),None)
        calls,replacements,probes=[],[],[]
        class FixturePP(FFmpegExtractAudioPP):
            def get_audio_codec(self,path):
                probes.append(path)
                return case['source']['codec']
            def run_ffmpeg(self,path,out_path,codec,more_opts):
                calls.append({'input':path,'output':out_path,'codec':codec,'more_opts':more_opts})
        pp=FixturePP(preferredcodec=processor['preferredcodec'],preferredquality=processor['preferredquality'])
        pp._features={'fdk':False}
        os.replace=lambda source,target:replacements.append([source,target])
        info={'filepath':case['source']['filepath'],'ext':case['source']['ext'],'vcodec':'avc1','acodec':case['source']['codec']}
        delete,updated=pp.run(copy.deepcopy(info))
        quality=None
        if calls and len(calls[0]['more_opts'])==2:
            flag,value=calls[0]['more_opts']
            if flag in ('-q:a','-b:a','-vbr'):
                unit='k' if value.endswith('k') else ''
                number=float(value[:-1] if unit else value)
                quality={'flag':flag,'value':number if math.isfinite(number) else str(number),'unit':unit}
        target,_=resolve_mapping(info['ext'],processor['preferredcodec'])
        result[key].append({'name':case['name'],'selector':options.get('format'),
            'mapping_target':target,'output_ext':updated['ext'],'output_path':updated['filepath'],
            'calls':calls,'delete':delete,'replacements':replacements,'probes':probes,'quality_args':quality})
for case in fixture['invalid_cases']:
    try:parse_options(case['args'])
    except OptParseError:result['invalid_cases'].append(case['name'])
    else:raise AssertionError('Expected invalid source audio options: '+case['name'])
print(json.dumps(result,ensure_ascii=False))
'''


def compare(result,fixture):
    target={key:[{'name':case['name'],**case['expected']} for case in fixture[key]]
            for key in ('cases','source_only_cases')}
    target['invalid_cases']=[case['name'] for case in fixture['invalid_cases']]
    if result != target:
        raise ValueError('audio plan mismatch: '+json.dumps({'actual':result,'expected':target},sort_keys=True))
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream',required=True,type=Path)
    args=parser.parse_args()
    try:
        upstream=args.upstream.resolve()
        if not (upstream/'yt_dlp/postprocessor/ffmpeg.py').is_file():
            raise ValueError('--upstream must name an official yt-dlp source checkout')
        revision=subprocess.run(['git','-C',str(upstream),'rev-parse','HEAD'],capture_output=True,text=True,
                                encoding='utf-8',timeout=15,check=True).stdout.strip()
        if revision != PINNED_UPSTREAM:raise ValueError(f'--upstream must be checked out at {PINNED_UPSTREAM}')
        fixture=json.loads(FIXTURE.read_text(encoding='utf-8'))
        if fixture['upstream_commit'] != PINNED_UPSTREAM:raise ValueError('audio fixture does not match pinned upstream commit')
        result=subprocess.run([sys.executable,'-B','-c',UPSTREAM_WORKER,str(upstream)],input=json.dumps(fixture),
                              capture_output=True,text=True,encoding='utf-8',timeout=30,check=True)
        compare(json.loads(result.stdout),fixture)
    except (OSError,ValueError,KeyError,subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}',file=sys.stderr)
        if isinstance(error,subprocess.CalledProcessError) and error.stderr:print(error.stderr,file=sys.stderr)
        return 1
    print(f"Audio {len(fixture['cases'])} plans, {len(fixture['source_only_cases'])} source-only and "
          f"{len(fixture['invalid_cases'])} invalid cases match actual pinned postprocessor (virtual files; no ffmpeg execution)")
    return 0


if __name__=='__main__':raise SystemExit(main())
