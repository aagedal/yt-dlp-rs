#!/usr/bin/env python3
"""Generate src/extractor/peertube/instances.rs from the pinned upstream checkout.

Development only: reads `PeerTubeIE._INSTANCES_RE` from
`yt_dlp/extractor/peertube.py` without importing it, drops the verbose-mode
whitespace and comments, and writes the host alternatives (still regex
escaped, in upstream order) as a Rust array. `--check` fails when the saved
file differs instead of writing it.
"""
import argparse
import ast
from pathlib import Path
import re
import sys

PINNED_UPSTREAM = '6ccd977303412151ee23352c6257dd658ffc1987'
TARGET = Path(__file__).resolve().parents[1] / 'src/extractor/peertube/instances.rs'


def instances(source):
    for node in ast.walk(ast.parse(source)):
        if isinstance(node, ast.ClassDef) and node.name == 'PeerTubeIE':
            for statement in node.body:
                if (isinstance(statement, ast.Assign)
                        and any(getattr(target, 'id', None) == '_INSTANCES_RE'
                                for target in statement.targets)):
                    return ast.literal_eval(statement.value)
    raise SystemExit('PeerTubeIE._INSTANCES_RE not found')


def alternatives(pattern):
    # `(?x)`: whitespace is ignored and `#` starts a comment.
    text = ''.join(re.sub(r'#.*', '', line) for line in pattern.splitlines())
    text = re.sub(r'\s+', '', text)
    if not (text.startswith('(?:') and text.endswith(')')):
        raise SystemExit(f'unexpected _INSTANCES_RE shape: {text[:40]}...')
    hosts = text[3:-1].split('|')
    for host in hosts:
        if not re.fullmatch(r'(?:[A-Za-z0-9-]|\\\.)+', host):
            raise SystemExit(f'unexpected instance pattern: {host}')
    return hosts


def render(hosts):
    lines = [
        '//! Upstream `PeerTubeIE._INSTANCES_RE` (`yt_dlp/extractor/peertube.py`),',
        f'//! generated from the pinned upstream checkout ({PINNED_UPSTREAM})',
        '//! by `tools/generate_peertube_instances.py`; do not edit by hand.',
        '',
        '/// The instance host patterns (regex escaped), in upstream order.',
        'pub(super) const INSTANCES: &[&str] = &[',
    ]
    lines += [f'    r"{host}",' for host in hosts]
    lines.append('];')
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    source = (args.upstream / 'yt_dlp/extractor/peertube.py').read_text(encoding='utf-8')
    text = render(alternatives(instances(source)))
    if args.check:
        if not TARGET.is_file() or TARGET.read_text(encoding='utf-8') != text:
            print(f'{TARGET} differs from upstream')
            sys.exit(1)
        return
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(text, encoding='utf-8')


if __name__ == '__main__':
    main()
