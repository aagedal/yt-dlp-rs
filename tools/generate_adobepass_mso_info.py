#!/usr/bin/env python3
"""Generate src/extractor/adobepass/mso_info.rs from the pinned upstream checkout.

Development only: reads the module-level `MSO_INFO` dict of
`yt_dlp/extractor/adobepass.py` without importing it and writes its entries
(in upstream order: `name` and the optional `username_field`,
`password_field` and `login_hostname`) as a Rust array. `--check` fails when
the saved file differs instead of writing it.
"""
import argparse
import ast
from pathlib import Path
import sys

PINNED_UPSTREAM = 'fa9886678eb65103303c627b8e013f13f8999b41'
TARGET = Path(__file__).resolve().parents[1] / 'src/extractor/adobepass/mso_info.rs'
FIELDS = ('username_field', 'password_field', 'login_hostname')


def mso_info(source):
    for node in ast.parse(source).body:
        if (isinstance(node, ast.Assign)
                and any(getattr(target, 'id', None) == 'MSO_INFO' for target in node.targets)):
            return ast.literal_eval(node.value)
    raise SystemExit('MSO_INFO not found')


def rust_str(text):
    if not isinstance(text, str):
        raise SystemExit(f'unexpected MSO_INFO value: {text!r}')
    escaped = ''.join(
        '\\\\' if char == '\\' else '\\"' if char == '"'
        else f'\\u{{{ord(char):x}}}' if ord(char) < 0x20 else char
        for char in text)
    return f'"{escaped}"'


def render(table):
    lines = [
        '//! Upstream `MSO_INFO` (`yt_dlp/extractor/adobepass.py`), generated from',
        f'//! the pinned upstream checkout ({PINNED_UPSTREAM})',
        '//! by `tools/generate_adobepass_mso_info.py`; do not edit by hand.',
        'use super::MsoInfo;',
        '',
        '/// The TV providers by Adobe Pass MSO id, in upstream order.',
        'pub(crate) const MSO_INFO: &[(&str, MsoInfo)] = &[',
    ]
    for mso_id, info in table.items():
        unknown = set(info) - {'name', *FIELDS}
        if unknown:
            raise SystemExit(f'unexpected MSO_INFO keys for {mso_id}: {sorted(unknown)}')
        # rustfmt's layout of each entry.
        lines += ['    (', f'        {rust_str(mso_id)},', '        MsoInfo {',
                  f'            name: {rust_str(info["name"])},']
        for field in FIELDS:
            value = f'Some({rust_str(info[field])})' if field in info else 'None'
            lines.append(f'            {field}: {value},')
        lines += ['        },', '    ),']
    lines.append('];')
    return '\n'.join(lines) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    source = (args.upstream / 'yt_dlp/extractor/adobepass.py').read_text(encoding='utf-8')
    text = render(mso_info(source))
    if args.check:
        if not TARGET.is_file() or TARGET.read_text(encoding='utf-8') != text:
            print(f'{TARGET} differs from upstream')
            sys.exit(1)
        return
    TARGET.parent.mkdir(parents=True, exist_ok=True)
    TARGET.write_text(text, encoding='utf-8')


if __name__ == '__main__':
    main()
