#!/usr/bin/env python3
"""Compare bounded selector expressions with actual pinned upstream selection.

Development-only offline reference. Feed shared native VideoInfo-shaped fixtures
into YoutubeDL.build_format_selector with explicit context flags and format
order. The inputs are deliberately ordered consistently with native ranking; no
upstream sorter, extraction, format checks, FFmpeg, network or download is used.
Codec flags accompanying native stream_kind are reference-only fixture metadata.

Compare only a selected format ID or ordered requested_formats IDs, and empty
results for unavailable expressions. Group filters apply to context before child
selection. Group/fallback parsing and alias/bare-filter behavior are tested, not
native ranking, rendition-group compatibility, merge metadata/container policy,
multi-stream output or complete upstream grammar. Late malformed fallback syntax
must fail construction even when the first branch would succeed. Regex, stars,
.N, extension selectors, commas/all and merges above two streams are excluded.
No IDs, order, predicate values or error acceptance are normalized away.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = "897341241f2bfb2439fe0367f4c7c9ccd29a3828"
FIXTURE = Path(__file__).resolve().parents[1] / "tests/fixtures/selectors/expressions.json"
UPSTREAM_WORKER = r'''
import copy, json, socket, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL

def forbidden(*args, **kwargs):
    raise AssertionError('Network access forbidden in saved selector expression comparison')
socket.socket = socket.create_connection = forbidden
fixture = json.load(sys.stdin)
formats = fixture['info']['formats']
ctx = {'formats': formats,
       'has_merged_format': any('none' not in (f.get('acodec'), f.get('vcodec')) for f in formats),
       'incomplete_formats': (all(f.get('vcodec') == 'none' for f in formats)
                              or all(f.get('acodec') == 'none' for f in formats))}
assert ctx['has_merged_format'] is True and ctx['incomplete_formats'] is False
with YoutubeDL({'quiet': True, 'no_warnings': True, 'cachedir': False,
                'check_formats': False}) as ydl:
    ydl.urlopen = forbidden
    matches, errors = [], []
    for case in fixture['cases']:
        results = list(ydl.build_format_selector(case['selector'])(copy.deepcopy(ctx)))
        assert len(results) <= 1, 'Fixture unexpectedly selected multiple outputs'
        ids = [] if not results else [fmt['format_id'] for fmt in
                                     results[0].get('requested_formats', (results[0],))]
        assert len(ids) <= 2, 'Fixture unexpectedly selected more than two streams'
        matches.append({'name': case['name'], 'selected_ids': ids})
    for case in fixture['invalid_cases']:
        try:
            ydl.build_format_selector(case['selector'])
        except SyntaxError:
            errors.append(case['name'])
        else:
            raise AssertionError('Expected invalid selector syntax: ' + case['selector'])
    print(json.dumps({'matches': matches, 'syntax_errors': errors}))
'''


def compare(result, fixture):
    expected = {
        "matches": [{"name": case["name"], "selected_ids": case["expected_ids"]}
                    for case in fixture["cases"]],
        "syntax_errors": [case["name"] for case in fixture["invalid_cases"]],
    }
    if result != expected:
        raise ValueError("selector expression mismatch: " + json.dumps(
            {"upstream": result, "expected": expected}, sort_keys=True))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, type=Path)
    args = parser.parse_args()
    try:
        upstream = args.upstream.resolve()
        if not (upstream / "yt_dlp/YoutubeDL.py").is_file():
            raise ValueError("--upstream must name an official yt-dlp source checkout")
        revision = subprocess.run(["git", "-C", str(upstream), "rev-parse", "HEAD"],
                                  capture_output=True, text=True, encoding="utf-8", timeout=15,
                                  check=True).stdout.strip()
        if revision != PINNED_UPSTREAM:
            raise ValueError(f"--upstream must be checked out at {PINNED_UPSTREAM}")
        fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        if fixture["upstream_commit"] != PINNED_UPSTREAM:
            raise ValueError("selector fixture does not match the pinned upstream commit")
        result = subprocess.run([sys.executable, "-B", "-c", UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding="utf-8", timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"All {len(fixture['cases'])} saved selector expressions and "
          f"{len(fixture['invalid_cases'])} syntax failures match pinned upstream "
          "(selected IDs only; explicit input order; no ranking/full-grammar parity claim)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
