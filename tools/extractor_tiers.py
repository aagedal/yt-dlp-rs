#!/usr/bin/env python3
"""Extractor tiers: write or check docs/SITES.md, and survey the pinned upstream.

Development and CI only. The registry is the built binary's own
`--list-extractors` output (upstream's order and `(CURRENTLY BROKEN)` marks).
Each extractor is attributed:

- the canary URLs of tools/canary_sites.json and the case URLs of
  tests/fixtures/sites/*/cases.json, through `--list-extractors URL...`
  (upstream's rule: the first extractor in listing order whose pattern
  matches the URL, generic for the rest);
- fixture cases that reach it through another extractor: its name in the
  saved upstream output, as a `[NAME] ` screen line or `"extractor": "NAME"`.

Tier 1 extractors handle a canary URL. Tier 2 extractors have at least one
saved fixture case (or, for the few in OTHER_TESTS, another upstream
comparison). The rest are `untested`. `--write` regenerates docs/SITES.md,
`--check` (CI) fails when it differs or when the supported-sites summary in
docs/COMPATIBILITY.md states other counts. `--survey --upstream PATH` prints the
counts recorded in upstream/notes/extractor-tiers.md, with the pinned
upstream's own flags (`_WORKING`, `_ENABLED`, `IE_DESC = False`) and a
comparison of both listings.
"""
import argparse
from collections import Counter
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
SITES = ROOT / 'tests/fixtures/sites'
CANARY = ROOT / 'tools/canary_sites.json'
OUTPUT = ROOT / 'docs/SITES.md'
SUMMARY = ROOT / 'docs/COMPATIBILITY.md'
BROKEN = ' (CURRENTLY BROKEN)'
# Extractors without a URL pattern of their own, tested outside the site fixtures.
OTHER_TESTS = {
    'html5': 'generic\'s HTML5 media step; `tests/http.rs` and `tools/compare_html5.py`',
}
# Registered only to claim their URLs with an error (upstream/manifest.json scopes).
NOT_PORTED = {
    'GettrStreaming': 'not ported, reports an error (fails upstream at the pin)',
    'ximalaya:album': 'not ported, reports an error',
}
URL_CHUNK = 400
UPSTREAM_WORKER = r'''
import json, sys
sys.path.insert(0, sys.argv[1])
from yt_dlp.extractor import list_extractor_classes
rows = []
for ie in list_extractor_classes():
    rows.append({
        'name': ie.IE_NAME, 'working': bool(ie.working()), 'enabled': bool(ie._ENABLED),
        'desc': ie.IE_DESC is not False,
        'description': ie.description(markdown=False, search_examples=('X',)),
    })
print(json.dumps(rows))
'''


def default_binary():
    suffix = '.exe' if sys.platform == 'win32' else ''
    return ROOT / f'target/debug/yt-dlp-rs{suffix}'


def run(binary, *args):
    return subprocess.run([str(binary), *args], check=True, capture_output=True,
                          text=True, encoding='utf-8').stdout


def parse_listing(text):
    """`--list-extractors [URL...]` output: [(name, broken)] and {url: name}."""
    names, owners, current = [], {}, None
    for line in text.splitlines():
        if line.startswith('  '):
            owners[line[2:]] = current
        elif line:
            current = line[:-len(BROKEN)] if line.endswith(BROKEN) else line
            names.append((current, line.endswith(BROKEN)))
    return names, owners


def attribute(binary, urls):
    owners = {}
    urls = sorted(set(urls))
    for start in range(0, len(urls), URL_CHUNK):
        owners.update(parse_listing(run(binary, '--list-extractors', *urls[start:start + URL_CHUNK]))[1])
    return owners


def fixture_cases(sites=SITES):
    """[(site directory, url)] for every saved case."""
    cases = []
    for path in sorted(sites.glob('*/cases.json')):
        cases.extend((path.parent.name, case['url'])
                     for case in json.loads(path.read_text(encoding='utf-8')))
    return cases


def reached_names(text):
    """Extractor names in saved upstream output: screen lines and `extractor` fields."""
    names = set(re.findall(r'"\[([^\]"]+)\] ', text))
    names.update(re.findall(r'\\"extractor\\": \\"((?:[^"\\]|\\\\.)+?)\\"', text))
    return names


def reached(sites=SITES):
    """{name: set of site directories whose saved output names it}."""
    found = {}
    for path in sorted(sites.glob('*/expected.json')):
        for name in reached_names(path.read_text(encoding='utf-8')):
            found.setdefault(name, set()).add(path.parent.name)
    return found


def canary_urls(path=CANARY):
    data = json.loads(path.read_text(encoding='utf-8'))
    return [(site['name'], entry['url']) for site in data['sites'] for entry in site['urls']]


def classify(names, owners, cases, reached_by, canaries):
    """One row per registered extractor, in listing order."""
    direct = Counter(owners[url] for _, url in cases)
    sites = {}
    for directory, url in cases:
        sites.setdefault(owners[url], set()).add(directory)
    watched = {}
    for site, url in canaries:
        watched.setdefault(owners[url], []).append(site)
    rows = []
    for name, broken in names:
        via = reached_by.get(name, set()) - sites.get(name, set())
        if name in watched:
            tier = '1'
        elif direct[name] or via or name in OTHER_TESTS:
            tier = '2'
        else:
            tier = 'untested'
        rows.append({
            'name': name, 'broken': broken, 'tier': tier, 'cases': direct[name],
            'via': sorted(via), 'canary': sorted(set(watched.get(name, []))),
            'other': OTHER_TESTS.get(name), 'not_ported': NOT_PORTED.get(name),
        })
    return rows


def render(rows):
    counts = Counter(row['tier'] for row in rows)
    broken = sum(row['broken'] for row in rows)
    lines = [
        '# Supported sites',
        '',
        '<!-- Generated by tools/extractor_tiers.py --write; CI checks it. Do not edit. -->',
        '',
        f'{len(rows)} extractors, in `--list-extractors` order. Tiers are defined in',
        '[COMPATIBILITY.md](COMPATIBILITY.md#supported-sites): tier 1 is watched by the',
        'daily live canaries, tier 2 is compared with pinned upstream on saved',
        'responses only (best effort on the live site), `untested` has no saved',
        f'case. Tier 1: {counts["1"]}, tier 2: {counts["2"]}, untested: {counts["untested"]};',
        f'{broken} are marked broken by upstream (`(CURRENTLY BROKEN)` in',
        '`--list-extractors`, a warning before extraction).',
        '',
        'Cases are the saved fixture cases whose URL this extractor handles;',
        'for an extractor without its own, "reached via" lists the fixture',
        'directories whose upstream output shows it handling a URL passed on by',
        'another extractor.',
        '',
        '| Extractor | Tier | Cases | Notes |',
        '| --- | --- | ---: | --- |',
    ]
    for row in rows:
        notes = []
        if row['canary']:
            notes.append('canary: ' + ', '.join(row['canary']))
        if row['broken']:
            notes.append('marked broken upstream')
        if row['via'] and not row['cases']:
            notes.append('reached via ' + ', '.join(f'`{site}`' for site in row['via']))
        if row['other']:
            notes.append(row['other'])
        if row['not_ported']:
            notes.append(row['not_ported'])
        name = row['name'].replace('|', '\\|')
        lines.append(f'| {name} | {row["tier"]} | {row["cases"]} | {"; ".join(notes)} |')
    return '\n'.join(lines) + '\n'


def summary_lines(rows):
    """The counts COMPATIBILITY.md's supported-sites summary must state."""
    counts = Counter(row['tier'] for row in rows)
    return [f'{len(rows)} of upstream\'s', f'| 1 | {counts["1"]} |',
            f'| 2 | {counts["2"]} |', f'| untested | {counts["untested"]} |',
            f'`_WORKING = False` ({sum(row["broken"] for row in rows)} here)']


def build(binary):
    names, _ = parse_listing(run(binary, '--list-extractors'))
    cases = fixture_cases()
    canaries = canary_urls()
    owners = attribute(binary, [url for _, url in cases] + [url for _, url in canaries])
    return classify(names, owners, cases, reached(), canaries)


def upstream_rows(upstream):
    return json.loads(subprocess.run(
        [sys.executable, '-I', '-c', UPSTREAM_WORKER, str(upstream)],
        check=True, capture_output=True, text=True, encoding='utf-8').stdout)


def without_examples(line):
    # Search extractors' descriptions end with a random example query upstream.
    return re.sub(r' \(e\.g\. "[^"]*"\)$', '', line)


def survey(rows, upstream, descriptions):
    """Markdown counts comparing the registry with the pinned upstream."""
    ours = {row['name']: row for row in rows}
    theirs = {row['name']: row for row in upstream}
    removed = [name for name in ours if name not in theirs]
    broken_up = [row['name'] for row in upstream if not row['working']]
    ported_broken = [name for name in broken_up if name in ours]
    unmarked = [name for name in ported_broken if not ours[name]['broken']]
    wrongly_marked = [name for name, row in ours.items()
                      if row['broken'] and name in theirs and theirs[name]['working']]
    disabled = [row['name'] for row in upstream if not row['enabled'] and row['name'] in ours]
    no_desc = [row['name'] for row in upstream if not row['desc'] and row['name'] in ours]
    order = [row['name'] for row in upstream if row['name'] in ours] == list(ours)
    expected = [without_examples(row['description']) for row in upstream
                if row['name'] in ours and row['working'] and row['desc']]
    ours_desc = [without_examples(line) for line in descriptions.splitlines()]
    desc_diff = sorted(set(ours_desc) ^ set(expected))
    tiers = Counter(row['tier'] for row in rows)
    tier2 = [row for row in rows if row['tier'] == '2']
    with_cases = sum(1 for row in tier2 if row['cases'])
    via_only = sum(1 for row in tier2 if not row['cases'] and row['via'])
    lines = [
        f'- Registered extractors: {len(rows)} of {len(upstream)} upstream listing entries',
        f'- Tier 1 (canary URL): {tiers["1"]}: ' + ', '.join(r['name'] for r in rows if r['tier'] == '1'),
        f'- Tier 2: {tiers["2"]} ({with_cases} with own cases, {via_only} reached via another extractor only, '
        f'{len(tier2) - with_cases - via_only} tested elsewhere)',
        f'- Untested: {tiers["untested"]}: ' + ', '.join(r['name'] for r in rows if r['tier'] == 'untested'),
        f'- Saved fixture cases: {sum(r["cases"] for r in rows)} in {len(list(SITES.glob("*/cases.json")))} directories',
        f'- Upstream `_WORKING = False`: {len(broken_up)}; registered here: {len(ported_broken)}: ' + ', '.join(ported_broken),
        f'- Registered but not marked broken here: {len(unmarked)} {unmarked}',
        f'- Marked broken here but working upstream: {len(wrongly_marked)} {wrongly_marked}',
        f'- Registered here but absent upstream (removed): {len(removed)} {removed}',
        f'- Upstream `_ENABLED = False` among ours: {len(disabled)} {disabled}',
        f'- Upstream `IE_DESC = False` among ours: {len(no_desc)}: ' + ', '.join(no_desc),
        f'- `--list-extractors` order equals upstream\'s (filtered to ours): {order}',
        f'- `--extractor-descriptions` lines differing from upstream (examples ignored): {desc_diff}',
    ]
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=default_binary())
    mode = parser.add_mutually_exclusive_group(required=True)
    mode.add_argument('--write', action='store_true', help='regenerate docs/SITES.md')
    mode.add_argument('--check', action='store_true', help='fail when docs/SITES.md is stale')
    mode.add_argument('--survey', action='store_true', help='print counts against --upstream')
    parser.add_argument('--upstream', type=Path, help='pinned upstream checkout (--survey)')
    args = parser.parse_args()
    rows = build(args.binary)
    if args.survey:
        if not args.upstream:
            parser.error('--survey needs --upstream')
        sys.stdout.write(survey(rows, upstream_rows(args.upstream.resolve()),
                                run(args.binary, '--extractor-descriptions')))
        return 0
    text = render(rows)
    if args.write:
        OUTPUT.write_text(text, encoding='utf-8', newline='\n')
        return 0
    status = 0
    if OUTPUT.read_text(encoding='utf-8') != text:
        print(f'{OUTPUT} is stale; run tools/extractor_tiers.py --write', file=sys.stderr)
        status = 1
    summary = SUMMARY.read_text(encoding='utf-8')
    for line in summary_lines(rows):
        if line not in summary:
            print(f'{SUMMARY}: supported-sites summary lacks {line!r}', file=sys.stderr)
            status = 1
    return status


if __name__ == '__main__':
    sys.exit(main())
