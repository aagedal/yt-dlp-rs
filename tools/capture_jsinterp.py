#!/usr/bin/env python3
"""Capture pinned upstream `JSInterpreter` results for tests/fixtures/jsinterp.

Development only: runs every case of tests/fixtures/jsinterp/cases.json
(`code`, optional `func` (default `f`) and JSON `args`) through the pinned
upstream checkout's `JSInterpreter(code).call_function(func, *args)` and
saves `repr(result)`, or the exception's class name and message
(`orig_msg` for `ExtractorError`s), in expected.json. src/jsinterp/tests.rs
runs this port's interpreter on the same cases and compares.
"""
import argparse
import copy
import json
from pathlib import Path
import sys

PINNED_UPSTREAM = 'f264a6cbd1f630894da0f368df00f8aae50140e0'
FIXTURES = Path(__file__).resolve().parents[1] / 'tests/fixtures/jsinterp'


def run(case):
    from yt_dlp.jsinterp import JSInterpreter
    from yt_dlp.utils import ExtractorError

    try:
        result = JSInterpreter(case['code']).call_function(
            case.get('func', 'f'), *copy.deepcopy(case.get('args', [])))
    except ExtractorError as error:
        return {'error': type(error).__name__, 'message': error.orig_msg}
    except Exception as error:
        return {'error': type(error).__name__, 'message': str(error)}
    return {'result': repr(result)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    sys.path.insert(0, str(args.upstream.resolve()))
    cases = json.loads((FIXTURES / 'cases.json').read_text(encoding='utf-8'))
    expected = {'upstream_commit': PINNED_UPSTREAM,
                'cases': [{**case, **run(case)} for case in cases]}
    text = json.dumps(expected, indent=1, ensure_ascii=False) + '\n'
    path = FIXTURES / 'expected.json'
    if args.write:
        path.write_text(text, encoding='utf-8')
    elif not path.is_file() or path.read_text(encoding='utf-8') != text:
        print('jsinterp: saved output differs from upstream')
        sys.exit(1)
    print(f'jsinterp: {len(cases)} cases')


if __name__ == '__main__':
    main()
