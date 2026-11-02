#!/usr/bin/env python3
"""Report an upstream release the pinned baseline has not caught up with.

Road to 1.0 step 10: a baseline advance follows each upstream release within
a week. This tool finds the newest release tag in a local upstream checkout
(fetched with tags), and when the manifest's baseline does not contain it,
renders a tracking issue with the upstream_report.py brief: intervening
commits, affected components and unmapped files. It never fetches, edits the
manifest or ports anything; .github/workflows/upstream.yml runs it daily and
keeps one issue labelled `upstream` open while a release is pending.

Exit status: 0 when caught up or still within the deadline, 1 when the
release is older than --deadline-days, 2 on errors.
"""

import argparse
import datetime
import json
from pathlib import Path
import re
import subprocess
import sys

from upstream_report import MANIFEST, build_report, git, resolve

RELEASE_TAG = re.compile(r'^(\d{4})\.(\d{2})\.(\d{2})(?:\.(\d+))?$')
DEADLINE_DAYS = 7
# Lists in the issue body are capped so a long gap stays readable; the
# workflow's artifact keeps the whole report.
LIST_LIMIT = 40
EXTRACTOR_DIR = 'yt_dlp/extractor/'
# Shared extractor modules: a component touched only through these counts as
# core, since the change reaches every extractor.
SHARED_EXTRACTOR_FILES = {EXTRACTOR_DIR + name for name in (
    '__init__.py', '_extractors.py', 'common.py', 'extractors.py', 'generic.py')}


def site_file(path):
    return path.startswith(EXTRACTOR_DIR) and path not in SHARED_EXTRACTOR_FILES


def release_key(tag):
    match = RELEASE_TAG.match(tag)
    return tuple(int(part or 0) for part in match.groups()) if match else None


def latest_release(tags):
    """The newest `YYYY.MM.DD[.N]` tag, or None."""
    releases = [tag for tag in tags if release_key(tag)]
    return max(releases, key=release_key) if releases else None


def contains(checkout, ancestor, descendant):
    result = subprocess.run(
        ['git', '-C', str(checkout), 'merge-base', '--is-ancestor', ancestor, descendant],
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    if result.returncode not in (0, 1):
        raise subprocess.CalledProcessError(result.returncode, result.args, stderr=result.stderr)
    return result.returncode == 0


def commit_date(checkout, revision):
    stamp = git(checkout, 'log', '-1', '--format=%cI', revision, '--').strip()
    return datetime.datetime.fromisoformat(stamp)


def check(checkout, manifest, today):
    """The pending release (with its report), or a caught-up status."""
    tag = latest_release(git(checkout, 'tag', '--list').split())
    if tag is None:
        raise ValueError('no upstream release tags in the checkout (fetch with --tags)')
    target = resolve(checkout, tag)
    baseline = resolve(checkout, manifest['baseline'])
    status = {'release': tag, 'target': target, 'baseline': baseline,
              'baseline_version': manifest.get('upstream_version')}
    if contains(checkout, target, baseline):
        return {**status, 'status': 'current'}
    if not contains(checkout, baseline, target):
        raise ValueError(f'baseline {baseline} is not an ancestor of release {tag} ({target})')
    released = commit_date(checkout, target).date()
    due = released + datetime.timedelta(days=DEADLINE_DAYS)
    return {
        **status,
        'status': 'overdue' if today > due else 'pending',
        'released': released.isoformat(),
        'due': due.isoformat(),
        'report': build_report(checkout, manifest, target=target),
    }


def capped(lines, limit=LIST_LIMIT):
    if len(lines) <= limit:
        return lines
    return [*lines[:limit], f'- ... and {len(lines) - limit} more (see the workflow artifact)']


def render_issue(result, repository, run_url=None):
    report = result['report']
    tag = result['release']
    state = 'overdue' if result['status'] == 'overdue' else f'due {result["due"]}'
    title = f'Upstream {tag}: baseline advance {state}'
    components = report['affected_components']
    extractors = [c for c in components if all(map(site_file, c['changed_files']))]
    core = [c for c in components if c not in extractors]
    lines = [
        f'Upstream released [{tag}]({repository.removesuffix(".git")}/releases/tag/{tag}) '
        f'on {result["released"]}; the baseline is still {result["baseline_version"]} '
        f'(`{result["baseline"]}`). Road to 1.0 step 10 asks for a baseline advance '
        f'by {result["due"]}.',
        '',
        f'Target: `{result["target"]}`',
        '',
        f'{len(report["commits"])} commits, {len(report["changed_files"])} changed files, '
        f'{len(core)} core and {len(extractors)} extractor components affected, '
        f'{len(report["unmapped_files"])} unmapped files.',
        '',
        '### Commits',
        '',
        *capped([f'- `{c["commit"][:9]}` {c["subject"]}' for c in report['commits']]),
    ]
    for heading, group in (('Core components', core), ('Extractor components', extractors)):
        if group:
            lines += ['', f'### {heading}', '',
                      *capped([f'- `{c["name"]}` ({c["status"]}): '
                               + ', '.join(f'`{p}`' for p in c['changed_files'])
                               for c in group])]
    unmapped = report['unmapped_files']
    if unmapped:
        new_extractors = [p for p in unmapped if site_file(p)]
        lines += ['', '### Unmapped files', '',
                  f'{len(new_extractors)} under `{EXTRACTOR_DIR}`. Each needs a classification '
                  'in the update note, even when it is outside the current scope.', '',
                  *capped([f'- `{p}`' for p in sorted(unmapped, key=lambda p: not site_file(p))])]
    lines += [
        '', '### Next steps', '',
        'Follow "Future update agent" in `docs/UPSTREAM.md` and the upstream update '
        'tasks in `AGENTS.md`: classify every commit in '
        f'`upstream/notes/upstream-{tag}.md`, port what applies with fixtures, re-check '
        'saved fixtures against the target, then advance the baseline.',
        '',
        'This issue is updated by `.github/workflows/upstream.yml` and closed '
        'automatically once the baseline contains the release.',
    ]
    if run_url:
        lines[1:1] = ['', f'Run: {run_url}']
    return {'title': title, 'body': '\n'.join(lines) + '\n'}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('checkout', type=Path,
                        help='official yt-dlp checkout with tags and the baseline commit')
    parser.add_argument('--manifest', type=Path, default=MANIFEST)
    parser.add_argument('--report', type=Path, help='write the full result as JSON')
    parser.add_argument('--issue', type=Path,
                        help='write {title, body} when a release is pending (removed otherwise)')
    parser.add_argument('--run-url', help='workflow run linked from the issue')
    parser.add_argument('--today', type=datetime.date.fromisoformat,
                        default=datetime.date.today(), help=argparse.SUPPRESS)
    args = parser.parse_args(argv)
    try:
        manifest = json.loads(args.manifest.read_text())
        result = check(args.checkout, manifest, args.today)
    except (OSError, ValueError, KeyError, subprocess.CalledProcessError) as error:
        detail = (error.stderr.decode(errors='replace').strip()
                  if isinstance(error, subprocess.CalledProcessError) and error.stderr
                  else str(error))
        print(f'upstream watch failed: {detail}', file=sys.stderr)
        return 2
    if args.report:
        args.report.write_text(json.dumps(result, indent=2) + '\n')
    if args.issue:
        if result['status'] == 'current':
            args.issue.unlink(missing_ok=True)
        else:
            args.issue.write_text(json.dumps(
                render_issue(result, manifest['repository'], args.run_url), indent=2) + '\n')
    if result['status'] == 'current':
        print(f'baseline {result["baseline_version"]} contains the latest release {result["release"]}')
        return 0
    report = result['report']
    print(f'upstream {result["release"]} ({result["released"]}) is {result["status"]}: '
          f'{len(report["commits"])} commits, {len(report["affected_components"])} components, '
          f'{len(report["unmapped_files"])} unmapped files; due {result["due"]}')
    return 1 if result['status'] == 'overdue' else 0


if __name__ == '__main__':
    sys.exit(main())
