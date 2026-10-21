#!/usr/bin/env python3
"""Check saved HTML helper cases with pinned actual upstream functions.

Development only; no network/process access in the reference worker. Inputs
are evaluated by actual `yt_dlp.utils.unescapeHTML`,
`InfoExtractor._og_search_property(prop, html, fatal=False)` and
`InfoExtractor._rta_search(html)`; results are compared exactly.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = '897341241f2bfb2439fe0367f4c7c9ccd29a3828'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/html/cases.json'
UPSTREAM_WORKER = r'''
import json, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.extractor.common import InfoExtractor
from yt_dlp.utils import unescapeHTML

def forbidden(*args, **kwargs):
    raise AssertionError('Network/process forbidden in HTML helper comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
fixture = json.load(sys.stdin)
ie = InfoExtractor(YoutubeDL({'quiet': True, 'no_warnings': True}))
print(json.dumps({
    'unescape': [{'input': c['input'], 'output': unescapeHTML(c['input'])}
                 for c in fixture['unescape']],
    'og': [{'html': c['html'], 'prop': c['prop'],
            'value': ie._og_search_property(c['prop'], c['html'], fatal=False)}
           for c in fixture['og']],
    'rta': [{'html': c['html'], 'age_limit': ie._rta_search(c['html'])} for c in fixture['rta']],
}, ensure_ascii=False))
'''
SECTIONS = ('unescape', 'og', 'rta')


def compare(result, fixture):
    target = {key: fixture[key] for key in SECTIONS}
    if result != target:
        raise ValueError('HTML helper mismatch: ' + json.dumps(
            {'upstream': result, 'expected': target}, ensure_ascii=False, sort_keys=True))
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
            raise ValueError('HTML helper fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    counts = ', '.join(f'{len(fixture[key])} {key}' for key in SECTIONS)
    print(f'HTML helpers ({counts}) match pinned upstream '
          '(actual unescapeHTML/_og_search_property/_rta_search)')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
