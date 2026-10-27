#!/usr/bin/env python3
"""Development-only encoding of RTVE media URLs for tests/fixtures/sites/rtve.

RTVE serves each asset's media URLs as a base64 PNG whose `tEXt` chunks hold
an alphabet and a digit-encoded URL (`RTVEBaseIE._decrypt_url` decodes them).
This writes the inverse encoding of the URLs below, deterministically.
"""
import base64
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1] / 'tests/fixtures/sites/rtve'
PNGS = {
    'video-3088905.png.txt': [
        ('HD_FULL', 'https://rtve-mediavod.example.test/resources/3088905_HD_FULL.mp4'),
        ('Alta', 'https://rtve-mediavod.example.test/resources/3088905_Alta.mp4'),
        ('', 'https://rtve-hls.example.test/resources/3088905/master.m3u8'),
        ('', 'https://rtve-dash.example.test/resources/3088905/manifest.mpd'),
    ],
    'audio-6766574.png.txt': [
        ('Media', 'https://rtve-mediavod.example.test/audios/6766574.mp3'),
    ],
    'video-1688877.png.txt': [
        ('', 'https://rtve-live.example.test/la1/master.m3u8'),
    ],
}


def encode_alphabet(chars):
    """Each character followed by the filler `_get_alphabet` skips."""
    out, skip = [], 0
    for char in chars:
        out.append(char)
        skip = (skip + 1) % 4
        out.append('x' * skip)
    return ''.join(out)


def encode_url(alphabet, url):
    """Each index's tens digit, `_get_url`'s skipped digits, its units digit."""
    out, skip, count = [], 3, 1
    for char in url:
        index = alphabet.index(char)
        out.append(f'{index // 10}{"7" * skip}{index % 10}')
        skip = (count + 3) % 4
        count += 1
    return ''.join(out)


def chunk(kind, data):
    return struct.pack('!I', len(data)) + kind + data + struct.pack('!I', zlib.crc32(kind + data))


def png(pairs):
    body = b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', bytes(13))
    for quality, url in pairs:
        alphabet = ''.join(sorted(set(url)))
        assert len(alphabet) <= 100 and '#' not in alphabet
        text = f'{encode_alphabet(alphabet)}#{quality}%%{encode_url(alphabet, url)}'
        body += chunk(b'tEXt', text.encode('iso-8859-1'))
    return base64.b64encode(body + chunk(b'IEND', b'')).decode()


def main():
    ROOT.mkdir(parents=True, exist_ok=True)
    for name, pairs in PNGS.items():
        (ROOT / name).write_text(png(pairs), encoding='utf-8')


if __name__ == '__main__':
    main()
