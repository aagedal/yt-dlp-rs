#!/usr/bin/env python3
"""Check saved batch-file cases with pinned actual upstream `get_urls`.

Development only; no network/process access in the reference worker. Each case's
bytes are written to a private temporary file and read by upstream's actual
`get_urls(urls, batchfile, 0)`. URL lists are compared exactly.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = '6ccd977303412151ee23352c6257dd658ffc1987'
FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/batch/cases.json'
UPSTREAM_WORKER = r'''
import json, os, socket, subprocess, sys, tempfile
sys.path.insert(0, sys.argv[1])
from yt_dlp import get_urls

def forbidden(*args, **kwargs):
    raise AssertionError('Network/process forbidden in batch comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
fixture = json.load(sys.stdin)
result = []
with tempfile.TemporaryDirectory() as directory:
    for index, case in enumerate(fixture['cases']):
        path = os.path.join(directory, f'{index}.txt')
        with open(path, 'wb') as file:
            file.write(bytes.fromhex(case['batch_hex']))
        result.append({'name': case['name'], 'urls': get_urls(list(case['urls']), path, 0)})
print(json.dumps(result, ensure_ascii=False))
'''


def compare(result, fixture):
    target = [{'name': case['name'], 'urls': case['expected']} for case in fixture['cases']]
    if result != target:
        raise ValueError('batch URL mismatch: ' + json.dumps(
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
            raise ValueError('batch fixture does not match the pinned upstream commit')
        result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding='utf-8', timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Batch {len(fixture['cases'])} cases match pinned upstream (actual get_urls; exact URLs)")
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
