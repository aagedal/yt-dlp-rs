#!/usr/bin/env python3
"""Compare saved request-header options with actual pinned upstream, offline.

Actual parse_options, HTTPHeaderDict and YoutubeDL._calc_headers are exercised.
Only header names are lowercased for case-insensitive comparison; values are
exact actual results. Request cases supply fixed defaults, not randomized UA.
Cookie compatibility and malformed/header-bound differences are source-only.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = '897341241f2bfb2439fe0367f4c7c9ccd29a3828'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/headers/cases.json'
UPSTREAM_WORKER = r'''
import json, socket, subprocess, sys
from optparse import OptParseError
from types import SimpleNamespace
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL, parse_options
from yt_dlp.cookies import YoutubeDLCookieJar
from yt_dlp.utils.networking import HTTPHeaderDict

def forbidden(*args, **kwargs):
    raise AssertionError('Network/download/process forbidden in header comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
YoutubeDL.urlopen = YoutubeDL.download = forbidden
fixture = json.load(sys.stdin)

def normalized(headers):
    return {name.lower():value for name,value in headers.items()}

def options(case):
    # Explicit argv disables user config and environmental header injection.
    params = parse_options(case['args']).ydl_opts
    return normalized(HTTPHeaderDict(params['http_headers']))

result = {'cases':[], 'request_cases':[], 'source_only_cases':[], 'invalid_cases':[]}
for key in ('cases','source_only_cases'):
    for case in fixture[key]:
        result[key].append({'name':case['name'],'headers':options(case)})
for case in fixture['request_cases']:
    defaults = HTTPHeaderDict(case['defaults'], case['global'])
    info = {'url':'https://example.test/media.mp4', 'http_headers':case['extractor']}
    probe = SimpleNamespace(params={'http_headers':defaults},cookiejar=YoutubeDLCookieJar())
    headers = YoutubeDL._calc_headers(probe, info)
    result['request_cases'].append({'name':case['name'],'headers':normalized(headers)})
for case in fixture['invalid_cases']:
    try:
        options(case)
    except OptParseError:
        result['invalid_cases'].append(case['name'])
    else:
        raise AssertionError('Expected source option rejection: '+case['name'])
print(json.dumps(result,ensure_ascii=False))
'''


def compare(result, fixture):
    target = {key:[{'name':case['name'],'headers':case['expected_headers']} for case in fixture[key]]
              for key in ('cases','request_cases','source_only_cases')}
    target['invalid_cases'] = [case['name'] for case in fixture['invalid_cases']]
    if result != target:
        raise ValueError('request header mismatch: '+json.dumps(
            {'actual':result,'expected':target},ensure_ascii=False,sort_keys=True))
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream',required=True,type=Path)
    args=parser.parse_args()
    try:
        upstream=args.upstream.resolve()
        if not (upstream/'yt_dlp/YoutubeDL.py').is_file():
            raise ValueError('--upstream must name an official yt-dlp source checkout')
        revision=subprocess.run(['git','-C',str(upstream),'rev-parse','HEAD'],capture_output=True,
                                text=True,encoding='utf-8',timeout=15,check=True).stdout.strip()
        if revision != PINNED_UPSTREAM:
            raise ValueError(f'--upstream must be checked out at {PINNED_UPSTREAM}')
        fixture=json.loads(FIXTURE.read_text(encoding='utf-8'))
        if fixture['upstream_commit'] != PINNED_UPSTREAM:
            raise ValueError('header fixture does not match pinned upstream commit')
        result=subprocess.run([sys.executable,'-B','-c',UPSTREAM_WORKER,str(upstream)],
                              input=json.dumps(fixture),capture_output=True,text=True,encoding='utf-8',
                              timeout=30,check=True)
        compare(json.loads(result.stdout),fixture)
    except (OSError,ValueError,KeyError,subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}',file=sys.stderr)
        if isinstance(error,subprocess.CalledProcessError) and error.stderr:
            print(error.stderr,file=sys.stderr)
        return 1
    print(f"Headers {len(fixture['cases'])} options, {len(fixture['request_cases'])} request merges, "
          f"{len(fixture['invalid_cases'])} invalid and {len(fixture['source_only_cases'])} source-only cases "
          "match actual pinned upstream (case-insensitive names; exact values)")
    return 0


if __name__=='__main__':
    raise SystemExit(main())
