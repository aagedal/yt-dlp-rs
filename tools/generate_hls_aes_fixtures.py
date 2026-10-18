#!/usr/bin/env python3
"""Development-only reproducible encryption of original local HLS test media."""
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/hls'
KEYS = [bytes(range(16)), bytes(range(16, 32))]
IV = 0x112233445566778899aabbccddeeff00


def encrypt(source, target, key, iv):
    subprocess.run(['openssl', 'enc', '-aes-128-cbc', '-nosalt', '-K', key.hex(),
                    '-iv', f'{iv:032x}', '-in', str(source), '-out', str(target)],
                   check=True)


def main():
    key_directory = ROOT / 'aes-keys'
    key_directory.mkdir(exist_ok=True)
    for index, key in enumerate(KEYS):
        (key_directory / f'{index}.key').write_bytes(key)
    for kind in ('ts', 'fmp4'):
        directory = ROOT / f'aes-{kind}'
        directory.mkdir(exist_ok=True)
        lines = ['#EXTM3U', '#EXT-X-VERSION:7', '#EXT-X-TARGETDURATION:1',
                 '#EXT-X-MEDIA-SEQUENCE:7']
        if kind == 'fmp4':
            lines += ['#EXT-X-KEY:METHOD=AES-128,URI="../aes-keys/0.key",IV=0x2',
                      '#EXT-X-MAP:URI="init.mp4"']
            encrypt(ROOT / kind / 'init.mp4', directory / 'init.mp4', KEYS[0], 2)
        else:
            lines += ['#EXT-X-KEY:METHOD=AES-128,URI="../aes-keys/0.key"']
        suffix = 'ts' if kind == 'ts' else 'm4s'
        for index in range(3):
            iv = 7 + index if kind == 'ts' else 2
            key = KEYS[0]
            if index == 2:
                lines += [f'#EXT-X-KEY:METHOD=AES-128,URI="../aes-keys/1.key",IV=0x{IV:032x}']
                iv, key = IV, KEYS[1]
            name = f'segment-{index}.{suffix}'
            encrypt(ROOT / kind / name, directory / name, key, iv)
            lines += ['#EXTINF:0.6,', name]
        lines += ['#EXT-X-ENDLIST']
        (directory / 'index.m3u8').write_text('\n'.join(lines) + '\n', encoding='utf-8')
    directory = ROOT / 'aes-range'
    directory.mkdir(exist_ok=True)
    parts = [(ROOT / 'aes-fmp4' / name).read_bytes()
             for name in ('init.mp4', 'segment-0.m4s', 'segment-1.m4s', 'segment-2.m4s')]
    (directory / 'combined.mp4').write_bytes(b''.join(parts))
    lines = ['#EXTM3U', '#EXT-X-VERSION:7', '#EXT-X-TARGETDURATION:1',
             '#EXT-X-KEY:METHOD=AES-128,URI="../aes-keys/0.key",IV=0x2',
             f'#EXT-X-MAP:URI="combined.mp4",BYTERANGE="{len(parts[0])}@0"']
    for index, part in enumerate(parts[1:]):
        if index == 2:
            lines += [f'#EXT-X-KEY:METHOD=AES-128,URI="../aes-keys/1.key",IV=0x{IV:032x}']
        lines += ['#EXTINF:0.6,', f'#EXT-X-BYTERANGE:{len(part)}'
                  + (f'@{len(parts[0])}' if index == 0 else ''), 'combined.mp4']
    lines += ['#EXT-X-ENDLIST']
    (directory / 'index.m3u8').write_text('\n'.join(lines) + '\n', encoding='utf-8')


if __name__ == '__main__':
    main()
