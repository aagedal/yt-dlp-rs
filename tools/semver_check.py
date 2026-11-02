#!/usr/bin/env python3
"""Compare the library API with the latest release tag (cargo-semver-checks).

From 1.0 on, a breaking change fails unless Cargo.toml's major version is
above the release's. Before 1.0 the report is informational: every 0.x
release may break the API, and version bumps happen in release PRs, not in
the change that breaks. Needs `cargo-semver-checks` on PATH and the tags
fetched (`fetch-depth: 0` in CI). See docs/LIBRARY.md, "Stability".
"""
import argparse
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def major(version):
    return int(re.match(r'v?(\d+)\.', version).group(1))


def policy(current, baseline):
    """`skip`, `report` or `enforce` for a crate version against a release tag."""
    if major(current) > major(baseline):
        return 'skip'
    if major(current) == 0:
        return 'report'
    return 'enforce'


def crate_version(manifest):
    match = re.search(r'^version = "([^"]+)"', manifest, re.M)
    return match.group(1)


def latest_release():
    return subprocess.run(
        ['git', 'describe', '--tags', '--abbrev=0', '--match', 'v[0-9]*'],
        cwd=ROOT, check=True, capture_output=True, text=True).stdout.strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--baseline', help='release tag (default: the latest v* tag)')
    args = parser.parse_args()
    baseline = args.baseline or latest_release()
    current = crate_version((ROOT / 'Cargo.toml').read_text(encoding='utf-8'))
    mode = policy(current, baseline)
    print(f'yt-dlp-rs {current} against {baseline}: {mode}', flush=True)
    if mode == 'skip':
        return 0
    # Default features only: `impersonate` adds no public items, and
    # building BoringSSL here would only slow the check down.
    status = subprocess.run(
        ['cargo', 'semver-checks', '--baseline-rev', baseline,
         '--default-features', '--release-type', 'minor'], cwd=ROOT).returncode
    if status and mode == 'report':
        print(f'::warning::The library API changed incompatibly since {baseline}; '
              'allowed before 1.0.')
        return 0
    return status


if __name__ == '__main__':
    sys.exit(main())
