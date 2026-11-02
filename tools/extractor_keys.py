#!/usr/bin/env python3
"""Write src/data/extractor_keys.tsv from a pinned upstream checkout.

Offline development only. Each line is `IE_NAME<TAB>ie_key<TAB>enabled` for
every class of `gen_extractor_classes()`, in that order: upstream
`YoutubeDL._ies` order, which `_make_archive_id` searches for the first
extractor suitable for an entry's URL. Regenerate after a baseline advance.
"""
import argparse
from pathlib import Path
import subprocess
import sys

OUTPUT = Path(__file__).resolve().parents[1] / 'src/data/extractor_keys.tsv'
WORKER = r'''
import sys
sys.path.insert(0, sys.argv[1])
from yt_dlp.extractor import gen_extractor_classes
for ie in gen_extractor_classes():
    print(f'{ie.IE_NAME}\t{ie.ie_key()}\t{int(bool(ie._ENABLED))}')
'''


def render(upstream):
    return subprocess.run(
        [sys.executable, '-I', '-c', WORKER, str(upstream)],
        check=True, capture_output=True, text=True).stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', type=Path, required=True)
    parser.add_argument('--write', action='store_true',
                        help='write the table instead of checking it')
    args = parser.parse_args()
    table = render(args.upstream.resolve())
    if args.write:
        OUTPUT.write_text(table, encoding='utf-8')
        return 0
    if OUTPUT.read_text(encoding='utf-8') != table:
        print(f'{OUTPUT} differs from {args.upstream}; rerun with --write', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
