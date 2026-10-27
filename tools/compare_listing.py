#!/usr/bin/env python3
"""Verify shared listing metadata and pinned upstream listing control flow.

Development-only offline reference. Actual YoutubeDL.list_formats executes its
renderer on an adaptive-only saved VideoInfo-shaped fixture. Compare declaration
IDs and typed metadata fields, not table column layout, whitespace, coloring,
resolution spelling, rate rounding or exact rendered cells. Reference codec
fields map to native stream_kind; no metadata values are otherwise normalized.

Separately run actual parse_options and process_video_result, intercepting only
listing/output/selection observations. Network, download and external processes
are forbidden. The source may sort formats during processing, so control-flow
listing IDs are compared as a set, never as native declaration-order parity.
-F/-F -f unavailable list without selection; -F -j and -F -s select. Explicit
pairs avoid querying default merge/FFmpeg policy. --print-json is a reference-only
probe demonstrating upstream's unselected JSON path. Native doesn't implement it.
Upstream prints the table before a later selection failure; native validates a
whole input before output. That atomic-output difference remains documented.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = "f264a6cbd1f630894da0f368df00f8aae50140e0"
FIXTURES = Path(__file__).resolve().parents[1] / "tests/fixtures/listing"
FIELDS = ("format_id", "stream_kind", "ext", "width", "height", "filesize", "tbr",
          "language", "format_note")
FLOW_CASES = (
    ("plain", ["-F"]),
    ("valid-list-only", ["-F", "-f", "v360"]),
    ("unavailable-list-only", ["-F", "-f", "unavailable"]),
    ("json-pair", ["-F", "-j", "-f", "v720+a128"]),
    ("simulate-pair", ["-F", "-s", "-f", "v720+a128"]),
    ("reference-print-json", ["-F", "--print-json"]),
    ("combined-unavailable", ["-F", "-j", "-f", "unavailable"]),
    ("malformed-before-listing", ["-F", "-f", "bv+"]),
)
UPSTREAM_WORKER = r'''
import copy, json, socket, subprocess, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL, parse_options
from yt_dlp.utils import ExtractorError

def forbidden(*args, **kwargs):
    raise AssertionError('Network/download/external process forbidden in listing comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
fixture = json.load(sys.stdin)
info, flow_cases = fixture['info'], fixture['flow_cases']

def selected_ids(info):
    if info.get('requested_formats'):
        return [fmt['format_id'] for fmt in info['requested_formats']]
    return [info['format_id']] if info.get('format_id') else []

class Capture(YoutubeDL):
    dl = forbidden
    urlopen = forbidden
    def __init__(self, params):
        self.events, self.tables, self.json_output, self.listed = [], [], [], []
        self.selection_calls = 0
        super().__init__(params)
    def to_screen(self, *args, **kwargs):
        pass
    def to_stdout(self, message, *args, **kwargs):
        if str(message).startswith('{'):
            self.events.append('json')
            self.json_output.append(json.loads(message))
        else:
            self.events.append('table')
            self.tables.append(str(message))
    def list_formats(self, info):
        self.events.append('list')
        self.listed.append(copy.deepcopy(info['formats']))
        return super().list_formats(info)
    def _select_formats(self, formats, selector):
        self.events.append('select')
        self.selection_calls += 1
        return super()._select_formats(formats, selector)

with Capture({'quiet': True, 'cachedir': False, 'listformats': True}) as ydl:
    ydl.list_formats(copy.deepcopy(info))
    assert len(ydl.tables) == 1 and len(ydl.listed) == 1
    assert all(fmt['format_id'] in ydl.tables[0] for fmt in info['formats'])
    direct = {'formats': ydl.listed[0], 'declaration_ids': [fmt['format_id'] for fmt in ydl.listed[0]]}

flows = []
for name, argv in flow_cases:
    ydl, result, status = None, None, 'ok'
    try:
        params = parse_options(argv).ydl_opts
        params.update(cachedir=False, no_warnings=True, check_formats=False)
        ydl = Capture(params)
        result = ydl.process_video_result(copy.deepcopy(info), download=True)
    except SyntaxError:
        status = 'syntax_error'
    except ExtractorError as error:
        assert error.expected
        status = 'unavailable'
    flows.append({
        'name': name, 'simulate': ydl.params.get('simulate') if ydl else None,
        'status': status, 'selection_calls': ydl.selection_calls if ydl else 0,
        'listed_ids': sorted(fmt['format_id'] for fmt in ydl.listed[0]) if ydl and ydl.listed else [],
        'selected_ids': selected_ids(result) if result else [],
        'json_selected_ids': selected_ids(ydl.json_output[0]) if ydl and ydl.json_output else None,
        'events': ydl.events if ydl else [],
    })
    if ydl:
        ydl.close()
print(json.dumps({'direct': direct, 'flows': flows}))
'''


def normalize_formats(formats):
    result = []
    for source in formats:
        entry = {field: source.get(field) for field in FIELDS}
        video, audio = source.get("vcodec"), source.get("acodec")
        if video == "none" and audio and audio != "none":
            entry["stream_kind"] = "audio_only"
        elif audio == "none" and video and video != "none":
            entry["stream_kind"] = "video_only"
        elif video and audio and video != "none" and audio != "none":
            entry["stream_kind"] = "standalone"
        elif entry["stream_kind"] is None:
            raise ValueError("missing listing stream classification")
        result.append(entry)
    return result


def compare(result, expected):
    actual = {"declaration_ids": result["direct"]["declaration_ids"],
              "formats": normalize_formats(result["direct"]["formats"]), "flows": result["flows"]}
    target = {key: expected[key] for key in actual}
    if actual != target:
        raise ValueError("listing metadata/control-flow mismatch: " + json.dumps(
            {"upstream": actual, "expected": target}, sort_keys=True))
    return actual


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
        info = json.loads((FIXTURES / "input.json").read_text(encoding="utf-8"))
        expected = json.loads((FIXTURES / "expected.json").read_text(encoding="utf-8"))
        if expected["upstream_commit"] != PINNED_UPSTREAM:
            raise ValueError("listing fixture does not match the pinned upstream commit")
        result = subprocess.run([sys.executable, "-B", "-c", UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps({"info": info, "flow_cases": FLOW_CASES}),
                                capture_output=True, text=True, encoding="utf-8", timeout=30,
                                check=True)
        compare(json.loads(result.stdout), expected)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Listing metadata and {len(FLOW_CASES)} pinned upstream control flows match "
          "(no exact table-byte, sorting, default-selection or atomic-output parity claim)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
