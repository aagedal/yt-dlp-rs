#!/usr/bin/env python3
"""Check shared archive identity/membership fixtures against pinned upstream.

Development-only reference. Actual YoutubeDL archive preload, _make_archive_id,
in_download_archive and record_download_archive operate on a temporary regular
UTF-8 file and a set, with network, downloads and external processes forbidden.
Compare logical IDs/hits and appended records, not cross-platform newline bytes.
Actual process_video_result probes show archived items skip before selection,
plain simulation/listing don't write, and reference-only forced simulation can
write. Download success/failure/postprocess timing is outside these reference
probes; native end-to-end tests cover its publication contract separately.

Native identity validation rejects empty, outer-whitespace and control-bearing
components; upstream permissive construction, ie_key/URL fallback and old IDs
are excluded. Native extraction precedes archive filtering (no upstream early URL
ID skip), bounded locks/deduplicated recheck/missing-newline repair differ from
upstream blocking locks and append behavior. Those differences are not hidden.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys

PINNED_UPSTREAM = "6ccd977303412151ee23352c6257dd658ffc1987"
FIXTURE = Path(__file__).resolve().parents[1] / "tests/fixtures/archive/cases.json"
UPSTREAM_WORKER = r'''
import copy, json, socket, subprocess, sys, tempfile
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL

def forbidden(*args, **kwargs):
    raise AssertionError('Network/download/process forbidden in archive comparison')
socket.socket = socket.create_connection = forbidden
subprocess.Popen = forbidden
fixture = json.load(sys.stdin)
params = {'quiet': True, 'no_warnings': True, 'cachedir': False}
class Capture(YoutubeDL):
    dl = forbidden
    urlopen = forbidden
    def __init__(self, options):
        self.selected = self.listed = self.recorded = 0
        super().__init__(options)
    def to_screen(self, *args, **kwargs):
        pass
    def to_stdout(self, *args, **kwargs):
        pass
    def _select_formats(self, *args, **kwargs):
        self.selected += 1
        return super()._select_formats(*args, **kwargs)
    def list_formats(self, info):
        self.listed += 1
    def record_download_archive(self, info):
        self.recorded += 1
        return super().record_download_archive(info)

with tempfile.TemporaryDirectory(prefix='yt-dlp-rs-archive-reference-') as directory:
    path = Path(directory) / 'archive.txt'
    initial = fixture['archive_contents'].encode('utf-8')
    path.write_bytes(initial)
    with Capture(dict(params, download_archive=str(path))) as file_ydl:
        archive_set = set(file_ydl.archive)
        with Capture(dict(params, download_archive=archive_set)) as set_ydl:
            identities = [{
                'name': case['name'], 'archive_id': file_ydl._make_archive_id(case['info']),
                'file_hit': file_ydl.in_download_archive(case['info']),
                'set_hit': set_ydl.in_download_archive(case['info']),
            } for case in fixture['cases']]
            records = []
            for case in fixture['record_cases']:
                before = path.read_bytes()
                file_ydl.record_download_archive(case['info'])
                set_ydl.record_download_archive(case['info'])
                after = path.read_bytes()
                assert after.startswith(before), 'Recording did not append'
                records.append({
                    'name': case['name'], 'archive_id': file_ydl._make_archive_id(case['info']),
                    'last_record': after.decode('utf-8').splitlines()[-1],
                    'file_hit': file_ydl.in_download_archive(case['info']),
                    'set_hit': set_ydl.in_download_archive(case['info']),
                })
    flows = []
    for name, archived, listing, force_write in [
        ('archived-before-selection', True, False, False),
        ('plain-simulation-no-write', False, False, False),
        ('listing-no-write', False, True, False),
        ('reference-forced-simulation-write', False, False, True),
    ]:
        path.write_bytes(initial)
        info = copy.deepcopy(fixture['cases'][0]['info'])
        if not archived:
            info['id'] = 'Fresh reference item'
        info['formats'] = [{'format_id': 'single', 'ext': 'mp4', 'url': info['url'],
                            'acodec': 'mp4a.40.2', 'vcodec': 'avc1.42001e'}]
        options = dict(params, download_archive=str(path),
                       format='unavailable' if archived or listing else 'single',
                       check_formats=False, simulate=None if listing else True,
                       listformats=listing, force_write_download_archive=force_write)
        with Capture(options) as ydl:
            ydl.process_video_result(info, download=True)
            flows.append({'name': name, 'selection_calls': ydl.selected,
                          'listing_calls': ydl.listed, 'record_calls': ydl.recorded,
                          'archive_changed': path.read_bytes() != initial,
                          'result_hit': ydl.in_download_archive(info)})
print(json.dumps({'identities': identities, 'records': records, 'flows': flows}))
'''


def compare(result, fixture):
    target = {
        "identities": [{"name": case["name"], "archive_id": case["expected_archive_id"],
                        "file_hit": case["expected_hit"], "set_hit": case["expected_hit"]}
                       for case in fixture["cases"]],
        "records": [{"name": case["name"], "archive_id": case["expected_archive_id"],
                     "last_record": case["expected_archive_id"], "file_hit": True, "set_hit": True}
                    for case in fixture["record_cases"]],
        "flows": fixture["reference_flows"],
    }
    if result != target:
        raise ValueError("archive identity/membership/control-flow mismatch: " + json.dumps(
            {"upstream": result, "expected": target}, sort_keys=True))
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
            raise ValueError("archive fixture does not match the pinned upstream commit")
        result = subprocess.run([sys.executable, "-B", "-c", UPSTREAM_WORKER, str(upstream)],
                                input=json.dumps(fixture), capture_output=True, text=True,
                                encoding="utf-8", timeout=30, check=True)
        compare(json.loads(result.stdout), fixture)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print(f"Archive {len(fixture['cases'])} identities/hits, logical recording and "
          f"{len(fixture['reference_flows'])} control flows match pinned upstream "
          "(strict identity, extraction timing, lock/dedup/newline policies remain native)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
