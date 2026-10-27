#!/usr/bin/env python3
"""Verify saved Netscape cookies and response scoping using pinned actual jar.

Development-only offline reference: actual YoutubeDLCookieJar.load,
get_cookie_header, extract_cookies and save/reload. Header pairs compare as a
sorted multiset except explicit same-name path ordering checks. Logical saved
records exclude wire/file order and HttpOnly loss, not cookie scope or values.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = 'f264a6cbd1f630894da0f368df00f8aae50140e0'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/cookies/cases.json'
UPSTREAM_WORKER = r'''
import contextlib, io, json, socket, subprocess, sys, tempfile
from email.message import Message
from pathlib import Path
from urllib.request import Request
sys.path.insert(0, sys.argv[1])
from yt_dlp.cookies import YoutubeDLCookieJar

def forbidden(*args, **kwargs):
    raise AssertionError('Network/process forbidden in cookie comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
fixture=json.load(sys.stdin)
root=Path(sys.argv[2])

def parts(header):
    return [part.strip() for part in (header or '').split(';') if part.strip()]

def pairs(header):
    return sorted([part.split('=',1) if '=' in part else [None,part] for part in parts(header)],
                  key=lambda pair:json.dumps(pair,ensure_ascii=False))

def records(jar):
    return sorted([{'domain':c.domain,'path':c.path,'secure':c.secure,'expires':c.expires,
                    'name':c.name if c.value is not None else None,
                    'value':c.value if c.value is not None else c.name}
                   for c in jar],key=lambda item:json.dumps(item,sort_keys=True))

result={'file_cases':[],'response_cases':[],'source_only_cases':[]}
for key in result:
    for case in fixture[key]:
        jar=YoutubeDLCookieJar()
        warning=io.StringIO()
        if case.get('file'):
            with contextlib.redirect_stderr(warning):
                jar.load(root/case['file'])
        for event in case.get('events',[]):
            message=Message()
            for value in event['set_cookie']:
                message.add_header('Set-Cookie',value)
            class Response:
                def info(self):return message
            jar.extract_cookies(Response(),Request(event['url']))
        with tempfile.TemporaryDirectory() as directory:
            saved=Path(directory)/'cookies.txt'
            jar.save(saved)
            # Evaluate physical session encoding before reloading restores None.
            saved_sessions_zero=all(line.split('\t')[4]=='0' for line in saved.read_text(encoding='utf-8').splitlines()
                if line and not line.startswith('#') and line.split('\t')[5] in case.get('session_names',[]))
            reloaded=YoutubeDLCookieJar()
            reloaded.load(saved)
        saved_records=records(reloaded)
        requests=[]
        for request in case['requests']:
            header=reloaded.get_cookie_header(request['url'])
            record={'pairs':pairs(header)}
            if request.get('check_order'):
                record['ordered_parts']=parts(header)
            requests.append(record)
        result[key].append({'name':case['name'],'requests':requests,'saved_records':saved_records,
                            'saved_sessions_zero':saved_sessions_zero,'warning':bool(warning.getvalue())})
print(json.dumps(result,ensure_ascii=False))
'''


def compare(result, fixture):
    target={key:[{'name':case['name'],**case['expected']} for case in fixture[key]]
            for key in ('file_cases','response_cases','source_only_cases')}
    if result != target:
        raise ValueError('cookie metadata mismatch: '+json.dumps({'actual':result,'expected':target},
                                                              sort_keys=True,ensure_ascii=False))
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream',required=True,type=Path)
    args=parser.parse_args()
    try:
        upstream=args.upstream.resolve()
        if not (upstream/'yt_dlp/cookies.py').is_file():
            raise ValueError('--upstream must name an official yt-dlp source checkout')
        revision=subprocess.run(['git','-C',str(upstream),'rev-parse','HEAD'],capture_output=True,text=True,
                                encoding='utf-8',timeout=15,check=True).stdout.strip()
        if revision != PINNED_UPSTREAM:
            raise ValueError(f'--upstream must be checked out at {PINNED_UPSTREAM}')
        fixture=json.loads(FIXTURE.read_text(encoding='utf-8'))
        if fixture['upstream_commit'] != PINNED_UPSTREAM:
            raise ValueError('cookie fixture does not match pinned upstream commit')
        result=subprocess.run([sys.executable,'-B','-c',UPSTREAM_WORKER,str(upstream),str(FIXTURE.parent)],
                              input=json.dumps(fixture),capture_output=True,text=True,encoding='utf-8',timeout=30,check=True)
        compare(json.loads(result.stdout),fixture)
    except (OSError,ValueError,KeyError,subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}',file=sys.stderr)
        if isinstance(error,subprocess.CalledProcessError) and error.stderr:print(error.stderr,file=sys.stderr)
        return 1
    print(f"Cookies {len(fixture['file_cases'])} files, {len(fixture['response_cases'])} response cases and "
          f"{len(fixture['source_only_cases'])} source-only cases match actual pinned jar "
          '(sorted header multiset; explicit path-order checks; logical save/reload)')
    return 0


if __name__=='__main__':raise SystemExit(main())
