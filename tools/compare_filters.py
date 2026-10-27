#!/usr/bin/env python3
"""Verify shared native predicate fixtures against actual pinned upstream filters.

Development-only offline tooling. Call YoutubeDL._build_format_filter directly
for each saved predicate and evaluate the saved typed format dictionaries in
input declaration order. No extraction, URL acquisition, downloads, sorting,
ranking or complete selector grammar is tested. Socket access is forbidden.

The shared Rust fixture covers only supported numeric/text fields, numeric six
comparisons and missing-value ?, decimal/binary units, text equality/prefix/
suffix/substring and their negation, quoting/escapes and selector delimiters.
Regex and unsupported fields are excluded. Exact-byte filesize inputs and
integer comparison literals are <= 2^53; upstream converts numeric literals to
float, so larger integer precision parity is explicitly outside this comparison.
The pinned legacy suffix distinction is preserved: k/kB use 1024 while K/KB
use 1000; Mi/MiB use 1024 squared and M/MB use 1000 squared. Numeric-looking
unquoted text comparisons take upstream's numeric-first branch even for string
fields; quotes force text comparison. JSON positive/negative zero rates are kept.
No values, ordering, matching IDs or predicates are normalized away.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = "f264a6cbd1f630894da0f368df00f8aae50140e0"
FIXTURE = Path(__file__).resolve().parents[1] / "tests/fixtures/selectors/filters.json"
UPSTREAM_WORKER = r'''
import json, socket, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL

def forbidden(*args, **kwargs):
    raise AssertionError('Network access forbidden in saved format predicate comparison')
socket.socket = socket.create_connection = forbidden
fixture = json.load(sys.stdin)
with YoutubeDL({'quiet': True, 'no_warnings': True, 'cachedir': False}) as ydl:
    ydl.urlopen = forbidden
    results = []
    for case in fixture['cases']:
        predicate = ydl._build_format_filter(case['predicate'])
        results.append({'name': case['name'], 'matching_ids': [
            fmt['format_id'] for fmt in fixture['formats'] if predicate(fmt)]})
    print(json.dumps(results))
'''


def compare(results, cases):
    expected = [{"name": case["name"], "matching_ids": case["expected_ids"]} for case in cases]
    if results != expected:
        raise ValueError("predicate match mismatch: " + json.dumps(
            {"upstream": results, "expected": expected}, sort_keys=True))
    return results


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
            raise ValueError("predicate fixture does not match the pinned upstream commit")
        result = subprocess.run([sys.executable, "-B", "-c", UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding="utf-8", timeout=30, check=True)
        compare(json.loads(result.stdout), fixture["cases"])
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"All {len(fixture['cases'])} saved format predicates match pinned upstream "
          "(matching IDs only; no sorting/full-selector/large-integer float parity claim)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
