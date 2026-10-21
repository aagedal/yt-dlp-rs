#!/usr/bin/env python3
"""Capture pinned upstream `js_to_json` outputs for tests/fixtures/js_to_json.

Development only: imports `yt_dlp.utils.js_to_json` from an upstream checkout
and saves `[input, output]` pairs for the inputs below; the Rust unit test in
src/js_to_json.rs compares the native conversion with them exactly.
"""
import argparse
import json
from pathlib import Path
import sys

INPUTS = [
    "{file: 'a.mp4', title: \"T\"}",
    "{'file': '//cdn.invalid/a.mp4', width: 640, height: 0x168, bitrate: 012}",
    "{sources: [{file: \"a.mp4\", label: '720p'},], tracks: [], }",
    "{a: undefined, b: void 0, c: true, d: null, e: !0, f: !!1}",
    "{/* comment */ a: 1, // line\n b: 'x\\'y', c: \"q\\\"r\", d: 'tab\\there'}",
    "{a: `tmpl ${'x'} and ${1}`, b: 'a\\x41b', c: 'line\\\ncont'}",
    "{10: 'n', 0x1F: 'h', 017: 'o', e5: 1e5, x: 1.5e-3, y: .5}",
    "{a: new Array(1, 2), b: Array(3), c: new Date('2020-01-01'), d: new Foo(1)}",
    "{a: parseInt('42'), b: (function(x){return x})('v'), c: new Map([['k', 1]])}",
    "{\"already\": \"json\", \"n\": [1, 2, {\"x\": null}]}",
    "{a: 'it\\'s', b: \"\\u00e9\", c: '\\/path', d: '\\n'}",
    "{title: 'Unicode: café ☃', image: \"//i.invalid/p.jpg\"}",
    "[1, 2, 3,]",
    "{a: {b: {c: 'd'}}, $x: 1, _y: 2, Infinity: NaN}",
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    sys.path.insert(0, str(args.upstream))
    from yt_dlp.utils import js_to_json
    cases = [[text, js_to_json(text)] for text in INPUTS]
    out = Path(__file__).resolve().parents[1] / 'tests/fixtures/js_to_json/cases.json'
    text = json.dumps(cases, indent=1, ensure_ascii=False) + '\n'
    if args.write:
        out.write_text(text, encoding='utf-8')
    elif out.read_text(encoding='utf-8') != text:
        sys.exit('saved js_to_json cases differ from upstream')
    print(f'{len(cases)} js_to_json cases match pinned upstream')


if __name__ == '__main__':
    main()
