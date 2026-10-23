#!/usr/bin/env python3
"""Verify bounded eager playlist selection against actual pinned upstream.

Development-only offline tooling. Validate options through actual parse_options,
then call PlaylistEntries.get_requested_items on a materialized list, deduplicate
with actual orderedSet and reverse afterward as YoutubeDL does. Only subtract one
from source's one-based indices for the native zero-based API. Preserve request
order, slice phase, duplicate semantics, invalid grammar and warning presence.
Network, downloads, child processes and user configuration are not used.

Native ASCII/i64 bounds, 16 KiB specifications and 1024 segments are explicit
policies, not full upstream grammar parity. Unicode decimal cases are verified
separately as source-only acceptance. Empty explicit CLI items are rejected even
though low-level get_requested_items treats a falsy string as absent. Eager
selection does not claim paged/lazy behavior, archive breaks or entry acquisition.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = "6ccd977303412151ee23352c6257dd658ffc1987"
FIXTURE = Path(__file__).resolve().parents[1] / "tests/fixtures/playlist/cases.json"
UPSTREAM_WORKER = r'''
import json, socket, subprocess, sys
from optparse import OptParseError
from types import SimpleNamespace
sys.path.insert(0, sys.argv[1])
from yt_dlp import parse_options
from yt_dlp.utils import PlaylistEntries, orderedSet

def forbidden(*args, **kwargs):
    raise AssertionError('Network/download/process forbidden in playlist comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
fixture = json.load(sys.stdin)

def options(case):
    argv = ['--playlist-start', str(case['start'])]
    if case['end'] is not None:
        argv += ['--playlist-end', str(case['end'])]
    if case['items'] is not None:
        argv += ['--playlist-items=' + case['items']]
    if case['reverse']:
        argv += ['--playlist-reverse']
    # Explicit override argv makes upstream ignore user configuration files.
    return parse_options(argv).ydl_opts

def evaluate(case):
    params, warnings = options(case), []
    probe = SimpleNamespace(params=params,
        report_warning=lambda *args, **kwargs: warnings.append(args[0]),
        _match_entry=lambda *args, **kwargs: None)
    entries = PlaylistEntries(probe, {'entries': [{'id': str(i)} for i in range(case['length'])]})
    selected = list(orderedSet(entries.get_requested_items(), lazy=True))
    if params.get('playlistreverse'):
        selected.reverse()
    return {'name': case['name'], 'indices': [index - 1 for index, _ in selected],
            'warning': bool(warnings)}

result = {'matches': [evaluate(case) for case in fixture['cases']],
          'source_only_matches': [evaluate(case) for case in fixture['source_only_cases']],
          'invalid': []}
for case in fixture['invalid_cases']:
    try:
        options(case)
    except OptParseError:
        result['invalid'].append(case['name'])
    else:
        raise AssertionError('Expected invalid CLI playlist options: ' + case['name'])
print(json.dumps(result))
'''


def compare(result, fixture):
    def cases(key):
        return [{"name": case["name"], "indices": case["expected_indices"],
                 "warning": case["expected_warning"]} for case in fixture[key]]
    target = {"matches": cases("cases"), "source_only_matches": cases("source_only_cases"),
              "invalid": [case["name"] for case in fixture["invalid_cases"]]}
    if result != target:
        raise ValueError("playlist selection mismatch: " + json.dumps(
            {"upstream": result, "expected": target}, sort_keys=True))
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, type=Path)
    args = parser.parse_args()
    try:
        upstream = args.upstream.resolve()
        if not (upstream / "yt_dlp/utils/_utils.py").is_file():
            raise ValueError("--upstream must name an official yt-dlp source checkout")
        revision = subprocess.run(["git", "-C", str(upstream), "rev-parse", "HEAD"],
                                  capture_output=True, text=True, encoding="utf-8", timeout=15,
                                  check=True).stdout.strip()
        if revision != PINNED_UPSTREAM:
            raise ValueError(f"--upstream must be checked out at {PINNED_UPSTREAM}")
        fixture = json.loads(FIXTURE.read_text(encoding="utf-8"))
        if fixture["upstream_commit"] != PINNED_UPSTREAM:
            raise ValueError("playlist fixture does not match the pinned upstream commit")
        result = subprocess.run([sys.executable, "-B", "-c", UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding="utf-8", timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Playlist {len(fixture['cases'])} selections and {len(fixture['invalid_cases'])} "
          "invalid CLI cases match pinned upstream "
          "(materialized lists; phase/order/dedup preserved)")
    print(f"Verified {len(fixture['source_only_cases'])} source-only Unicode cases; "
          "native ASCII/resource bounds remain explicit policy")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
