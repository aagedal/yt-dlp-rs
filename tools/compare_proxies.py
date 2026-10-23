#!/usr/bin/env python3
"""Compare saved explicit/environment proxy decisions with actual pinned source.

Offline development only: actual parse_options, YoutubeDL.proxies, clean_proxies,
select_proxy and make_socks_proxy_opts. Controlled getproxies and system-bypass
stubs avoid ambient configuration. Endpoint projection preserves credentials,
DNS policy and selected routes, not raw proxy URL spelling or transport claims.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM='6ccd977303412151ee23352c6257dd658ffc1987'
FIXTURE=Path(__file__).resolve().parents[1]/'tests/fixtures/proxies/cases.json'
UPSTREAM_WORKER=r'''
import copy, http.client, json, socket, subprocess, sys, urllib.parse, urllib.request
sys.path.insert(0,sys.argv[1])
from yt_dlp import YoutubeDL, parse_options
from yt_dlp.compat import urllib as compat_urllib
from yt_dlp.networking._helper import make_socks_proxy_opts
from yt_dlp.utils.networking import HTTPHeaderDict, clean_proxies, select_proxy

def forbidden(*args,**kwargs):
    raise AssertionError('Network/process/download forbidden in proxy comparison')
socket.socket=socket.create_connection=forbidden
subprocess.Popen=forbidden
YoutubeDL.urlopen=YoutubeDL.download=forbidden
# Environment proxy discovery is replaced per case; no ambient OS bypass checks.
urllib.request.proxy_bypass=lambda *args:False
fixture=json.load(sys.stdin)

def endpoint(proxy):
    if proxy is None:return None
    parsed=urllib.parse.urlsplit(proxy)
    if parsed.scheme.startswith('socks'):
        opts=make_socks_proxy_opts(proxy)
        return {'scheme':parsed.scheme,'host':opts['addr'],'port':opts['port'],
                'username':opts['username'],'password':opts['password'],'rdns':opts['rdns']}
    if parsed.scheme not in ('http','https'):
        return {'unsupported_scheme':parsed.scheme,'raw':proxy}
    # Canonical endpoint projection only: HTTPConnection/HTTPSConnection defaults.
    default=(http.client.HTTPSConnection if parsed.scheme=='https' else http.client.HTTPConnection).default_port
    return {'scheme':parsed.scheme,'host':parsed.hostname,'port':parsed.port if parsed.port is not None else default,
            'username':urllib.parse.unquote(parsed.username) if parsed.username is not None else None,
            'password':urllib.parse.unquote(parsed.password) if parsed.password is not None else None,'rdns':None}

result={'cases':[],'source_only_cases':[]}
for key in result:
    for case in fixture[key]:
        params=parse_options(case['args']).ydl_opts
        urllib.request.getproxies=compat_urllib.request.getproxies=lambda:copy.deepcopy(case['environment'])
        probe=YoutubeDL(params,auto_init=False)
        proxies=probe.proxies.copy()
        headers=HTTPHeaderDict(case.get('headers',{}))
        clean_proxies(proxies,headers)
        selected=[endpoint(select_proxy(url,proxies)) for url in case['requests']]
        result[key].append({'name':case['name'],'proxy_option':params.get('proxy'),'selected':selected,
                            'headers':{name.lower():value for name,value in headers.items()}})
print(json.dumps(result,ensure_ascii=False))
'''


def compare(result,fixture):
    target={key:[{'name':case['name'],**case['expected']} for case in fixture[key]]
            for key in ('cases','source_only_cases')}
    if result != target:
        raise ValueError('proxy decision mismatch: '+json.dumps({'actual':result,'expected':target},sort_keys=True))
    return result


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream',required=True,type=Path)
    args=parser.parse_args()
    try:
        upstream=args.upstream.resolve()
        if not (upstream/'yt_dlp/utils/networking.py').is_file():
            raise ValueError('--upstream must name an official yt-dlp source checkout')
        revision=subprocess.run(['git','-C',str(upstream),'rev-parse','HEAD'],capture_output=True,text=True,
                                encoding='utf-8',timeout=15,check=True).stdout.strip()
        if revision != PINNED_UPSTREAM:raise ValueError(f'--upstream must be checked out at {PINNED_UPSTREAM}')
        fixture=json.loads(FIXTURE.read_text(encoding='utf-8'))
        if fixture['upstream_commit'] != PINNED_UPSTREAM:raise ValueError('proxy fixture does not match pinned upstream commit')
        result=subprocess.run([sys.executable,'-B','-c',UPSTREAM_WORKER,str(upstream)],input=json.dumps(fixture),
                              capture_output=True,text=True,encoding='utf-8',timeout=30,check=True)
        compare(json.loads(result.stdout),fixture)
    except (OSError,ValueError,KeyError,subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}',file=sys.stderr)
        if isinstance(error,subprocess.CalledProcessError) and error.stderr:print(error.stderr,file=sys.stderr)
        return 1
    print(f"Proxies {len(fixture['cases'])} source configurations and {len(fixture['source_only_cases'])} source-only cases "
          'match actual pinned source (canonical endpoints; explicit environment; no transport claim)')
    return 0


if __name__=='__main__':raise SystemExit(main())
