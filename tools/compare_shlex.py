#!/usr/bin/env python3
"""Check saved configuration tokenizer cases with Python's actual shlex.

Development only. Upstream `Config.read_file` tokenizes configuration files
with `shlex.split(contents, comments=True)`; each saved input is split again
by this interpreter's shlex and the tokens (or ValueError messages) are
compared exactly with tests/fixtures/config/shlex.json.
"""
import json
from pathlib import Path
import shlex
import sys

FIXTURE = Path(__file__).resolve().parents[1] / 'tests/fixtures/config/shlex.json'


def split(text):
    try:
        return {'input': text, 'tokens': shlex.split(text, comments=True)}
    except ValueError as error:
        return {'input': text, 'error': str(error)}


def compare(fixture):
    result = [split(case['input']) for case in fixture['cases']]
    if result != fixture['cases']:
        raise ValueError('shlex mismatch: ' + json.dumps({'python': result, 'expected': fixture['cases']}))
    return result


def main():
    try:
        compare(json.loads(FIXTURE.read_text(encoding='utf-8')))
    except (OSError, ValueError, KeyError) as error:
        print(f'comparison failed: {error}', file=sys.stderr)
        return 1
    print(f'Config tokenizer cases match Python shlex.split(comments=True) {sys.version.split()[0]}')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
