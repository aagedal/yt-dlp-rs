#!/usr/bin/env python3
"""Produce a read-only, machine-readable change brief for a future porting agent.

Uses a local upstream git checkout. No network access or third-party Python packages.
Unknown files remain visible as unmapped changes, never silently ignored.
"""

import argparse
import json
from pathlib import Path
import subprocess
import sys

MANIFEST = Path(__file__).resolve().parents[1] / "upstream" / "manifest.json"


def git(checkout, *args):
    return subprocess.run(
        ["git", "-C", str(checkout), *args],
        check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    ).stdout.decode("utf-8", errors="replace")


def resolve(checkout, revision):
    # Resolve user-provided revisions before using them in git ranges.
    return git(checkout, "rev-parse", "--verify", "--end-of-options", f"{revision}^{{commit}}").strip()


def matches(path, patterns):
    return any(path.startswith(pattern) if pattern.endswith("/") else path == pattern for pattern in patterns)


def build_report(checkout, manifest, target="HEAD", base=None):
    baseline = resolve(checkout, base or manifest["baseline"])
    target = resolve(checkout, target)
    # A baseline must be an ancestor: a report across unrelated histories is misleading.
    git(checkout, "merge-base", "--is-ancestor", baseline, target)
    paths = sorted(filter(None, git(checkout, "diff", "--name-only", "--no-renames", "-z", baseline, target, "--").split("\0")))
    components = []
    mapped = set()
    for component in manifest["components"]:
        changed = [p for p in paths if matches(p, component["upstream"])]
        if changed:
            mapped.update(changed)
            components.append({**component, "changed_files": changed})
    commits = []
    for line in git(checkout, "log", "--reverse", "--format=%H%x09%s", f"{baseline}..{target}", "--").splitlines():
        sha, subject = line.split("\t", 1)
        commits.append({"commit": sha, "subject": subject})
    return {
        "schema_version": 1,
        "repository": manifest["repository"],
        "baseline": baseline,
        "target": target,
        "commits": commits,
        "changed_files": paths,
        "affected_components": components,
        "unmapped_files": [p for p in paths if p not in mapped],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("checkout", type=Path, help="local official yt-dlp checkout containing the baseline commit")
    parser.add_argument("--manifest", type=Path, default=MANIFEST)
    parser.add_argument("--target", default="HEAD", help="upstream revision to compare")
    parser.add_argument("--base", help="explicit baseline override for investigation (does not edit the manifest)")
    args = parser.parse_args()
    try:
        manifest = json.loads(args.manifest.read_text())
        if manifest["schema_version"] != 1:
            raise ValueError("unsupported manifest schema version")
        report = build_report(args.checkout, manifest, args.target, args.base)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        detail = error.stderr.decode(errors="replace").strip() if isinstance(error, subprocess.CalledProcessError) else str(error)
        print(f"upstream report failed: {detail}\nEnsure the checkout contains both revisions and the baseline is an ancestor of the target.", file=sys.stderr)
        return 1
    print(json.dumps(report, indent=2, ensure_ascii=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
