#!/usr/bin/env python3
"""Capture pinned upstream output for the site extractor fixtures.

Development only: for every tests/fixtures/sites/<site>/cases.json, runs the
pinned upstream checkout through tools/site_proxy.py (fixture routes only, no
network) as `yt_dlp --ignore-config --proxy PROXY --no-check-certificate
--user-agent fixture ARGS URL` in an empty directory, and saves the exit
code, stdout, the `ERROR:`/`WARNING:` stderr lines and the written files in
expected.json (`epoch` and `_version` removed). tests/sites.rs runs this port
the same way through tests/helpers/site_proxy.rs and compares.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile

from site_proxy import SiteProxy

PINNED_UPSTREAM = '897341241f2bfb2439fe0367f4c7c9ccd29a3828'
SITES = Path(__file__).resolve().parents[1] / 'tests/fixtures/sites'


def normalize(text):
    text = re.sub(r'"epoch": \d+, ', '', text)
    return re.sub(r', "_version": \{[^{}]*\}', '', text)


def run(upstream, proxy, case):
    directory = tempfile.mkdtemp()
    env = dict(os.environ, PYTHONPATH=str(upstream))
    result = subprocess.run(
        [sys.executable, '-m', 'yt_dlp', '--ignore-config', '--proxy', proxy.url,
         '--no-check-certificate', '--user-agent', 'fixture', *case['args'], case['url']],
        capture_output=True, text=True, env=env, cwd=directory, timeout=120)
    files = {}
    for name in sorted(os.listdir(directory)):
        files[name] = os.path.getsize(os.path.join(directory, name))
    return {
        'exit': result.returncode,
        'stdout': normalize(result.stdout).splitlines(),
        'stderr': [line for line in result.stderr.splitlines()
                   if line.startswith(('ERROR:', 'WARNING:'))],
        'files': files,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    parser.add_argument('--write', action='store_true')
    parser.add_argument('sites', nargs='*', help='only these fixture directories (default: all)')
    args = parser.parse_args()
    failed = False
    for site in sorted(path for path in SITES.iterdir() if (path / 'cases.json').is_file()):
        if args.sites and site.name not in args.sites:
            continue
        proxy = SiteProxy(site)
        cases = json.loads((site / 'cases.json').read_text(encoding='utf-8'))
        expected = {'upstream_commit': PINNED_UPSTREAM,
                    'cases': [{**case, **run(args.upstream, proxy, case)} for case in cases]}
        text = json.dumps(expected, indent=1, ensure_ascii=False) + '\n'
        path = site / 'expected.json'
        if args.write:
            path.write_text(text, encoding='utf-8')
        elif not path.is_file() or path.read_text(encoding='utf-8') != text:
            print(f'{site.name}: saved output differs from upstream')
            failed = True
        print(f'{site.name}: {len(cases)} cases')
    if failed:
        sys.exit(1)


if __name__ == '__main__':
    main()
