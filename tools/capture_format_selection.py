#!/usr/bin/env python3
"""Capture upstream format selection for the native selector port.

Development/reference tooling only; Python is never a runtime fallback. Each
saved format list (already in sorter order, worst first, as
`process_video_result` passes them) is selected with every spec through the
pinned checkout's actual `YoutubeDL.build_format_selector` and the context
`process_video_result` builds. Results record each selected format's ID,
extension, requested format IDs and merge fields; errors record their
exception type and text. No network access is involved.

usage: capture_format_selection.py UPSTREAM_CHECKOUT OUTPUT_JSON
"""
import json
import sys

FORMAT_LISTS = {
    # A YouTube-like list: separate streams plus one combined format.
    'adaptive': [
        {'format_id': '139', 'ext': 'm4a', 'vcodec': 'none', 'acodec': 'mp4a.40.5', 'abr': 48, 'tbr': 48, 'language': 'en', 'format_note': 'low', 'protocol': 'https', 'filesize': 1000},
        {'format_id': '249', 'ext': 'webm', 'vcodec': 'none', 'acodec': 'opus', 'abr': 50, 'tbr': 50, 'language': 'en', 'format_note': 'low', 'protocol': 'https', 'filesize': 1200},
        {'format_id': '140', 'ext': 'm4a', 'vcodec': 'none', 'acodec': 'mp4a.40.2', 'abr': 129, 'tbr': 129, 'language': 'en', 'format_note': 'medium', 'protocol': 'https', 'filesize': 3000},
        {'format_id': '251', 'ext': 'webm', 'vcodec': 'none', 'acodec': 'opus', 'abr': 135, 'tbr': 135, 'language': 'en', 'format_note': 'medium', 'protocol': 'https'},
        {'format_id': '160', 'ext': 'mp4', 'vcodec': 'avc1.4d400c', 'acodec': 'none', 'height': 144, 'width': 256, 'fps': 30, 'vbr': 80, 'tbr': 80, 'format_note': '144p', 'protocol': 'https', 'filesize': 2000},
        {'format_id': '18', 'ext': 'mp4', 'vcodec': 'avc1.42001E', 'acodec': 'mp4a.40.2', 'height': 360, 'width': 640, 'fps': 30, 'tbr': 500, 'format_note': '360p', 'protocol': 'https', 'language': 'en'},
        {'format_id': '134', 'ext': 'mp4', 'vcodec': 'avc1.4d401e', 'acodec': 'none', 'height': 360, 'width': 640, 'fps': 30, 'vbr': 300, 'tbr': 300, 'format_note': '360p', 'protocol': 'https', 'filesize_approx': 5000},
        {'format_id': '243', 'ext': 'webm', 'vcodec': 'vp9', 'acodec': 'none', 'height': 360, 'width': 640, 'fps': 30, 'vbr': 280, 'tbr': 280, 'format_note': '360p', 'protocol': 'https'},
        {'format_id': '136', 'ext': 'mp4', 'vcodec': 'avc1.64001F', 'acodec': 'none', 'height': 720, 'width': 1280, 'fps': 30, 'vbr': 1500, 'tbr': 1500, 'format_note': '720p', 'protocol': 'https', 'dynamic_range': 'SDR'},
        {'format_id': '247', 'ext': 'webm', 'vcodec': 'vp9', 'acodec': 'none', 'height': 720, 'width': 1280, 'fps': 30, 'vbr': 1400, 'tbr': 1400, 'format_note': '720p', 'protocol': 'https'},
        {'format_id': '399', 'ext': 'mp4', 'vcodec': 'av01.0.08M.08', 'acodec': 'none', 'height': 1080, 'width': 1920, 'fps': 30, 'vbr': 2000, 'tbr': 2000, 'format_note': '1080p', 'protocol': 'https'},
        {'format_id': 'hls-1080', 'ext': 'mp4', 'vcodec': 'avc1.640028', 'acodec': 'mp4a.40.2', 'height': 1080, 'width': 1920, 'fps': 30, 'tbr': 3000, 'protocol': 'm3u8_native'},
    ],
    # HLS-like video-only variants with audio renditions in two groups.
    'groups': [
        {'format_id': 'hls-aac-English', 'ext': 'mp4', 'vcodec': 'none', 'acodec': None, 'language': 'en', 'format_note': 'English', 'protocol': 'm3u8_native'},
        {'format_id': 'hls-aac-Deutsch', 'ext': 'mp4', 'vcodec': 'none', 'acodec': None, 'language': 'de', 'format_note': 'Deutsch', 'protocol': 'm3u8_native'},
        {'format_id': 'hls-ac3-English', 'ext': 'mp4', 'vcodec': 'none', 'acodec': None, 'language': 'en', 'protocol': 'm3u8_native'},
        {'format_id': 'hls-500', 'ext': 'mp4', 'vcodec': 'avc1.4d401e', 'acodec': 'none', 'height': 360, 'tbr': 500, 'protocol': 'm3u8_native'},
        {'format_id': 'hls-1500', 'ext': 'mp4', 'vcodec': 'avc1.64001f', 'acodec': 'none', 'height': 720, 'tbr': 1500, 'protocol': 'm3u8_native'},
    ],
    # Unknown codecs count as having both video and audio.
    'unknown': [
        {'format_id': 'a', 'ext': 'mp4', 'height': 240},
        {'format_id': 'b', 'ext': 'webm', 'height': 480},
        {'format_id': 'c', 'ext': 'mp3', 'vcodec': 'none'},
    ],
    'video_only': [
        {'format_id': 'v1', 'ext': 'mp4', 'vcodec': 'avc1', 'acodec': 'none', 'height': 360},
        {'format_id': 'v2', 'ext': 'webm', 'vcodec': 'vp9', 'acodec': 'none', 'height': 720},
        {'format_id': 'v3', 'ext': 'mp4', 'vcodec': 'avc1', 'acodec': 'none', 'height': 1080},
    ],
    'audio_only': [
        {'format_id': 'a1', 'ext': 'm4a', 'vcodec': 'none', 'acodec': 'mp4a.40.2', 'abr': 64},
        {'format_id': 'a2', 'ext': 'opus', 'vcodec': 'none', 'acodec': 'opus', 'abr': 128},
        {'format_id': 'sb', 'ext': 'mhtml', 'vcodec': 'none', 'acodec': 'none'},
    ],
    'single': [
        {'format_id': 'mp4', 'ext': 'mp4', 'url': 'https://example.invalid/v.mp4'},
    ],
}

SPECS = [
    # Atoms, aliases, stars and ranks.
    'best', 'worst', 'b', 'w', 'bv', 'ba', 'wv', 'wa', 'bestvideo', 'bestaudio', 'worstvideo',
    'worstaudio', 'bv*', 'ba*', 'wv*', 'wa*', 'b*', 'w*', 'best*', 'worst*', 'bestvideo*',
    'bv.2', 'ba.2', 'b.2', 'bv*.3', 'w.2', 'wv.1', 'bv.10', 'b.1', 'bv.0', 'bv.01', 'bv.x',
    'bestvideo.2', 'best.2', 'b*.2',
    # IDs and extensions.
    '18', '140', '251', 'hls-1080', 'hls-500', 'a', 'v2', 'a2', 'sb', 'mp4', 'webm', 'm4a',
    'mp3', 'opus', '3gp', 'flv', 'mhtml', 'mkv', 'ogg', 'nonexistent', 'mp4/webm', 'hls-aac-English',
    # Fallbacks, merges and groups.
    'bv+ba', 'bv*+ba/b', 'bestvideo*+bestaudio/best', 'best/bestvideo+bestaudio',
    'bestvideo+bestaudio/best', 'ba+bv', 'bv+ba+ba', 'bv+bv', 'b+ba', 'bv+b', 'ba+b', 'ba+ba',
    '136+140', '136+251', '247+140', '247+251', '399+140', '18+140', '140+18', '134+160',
    'bv[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/b', '(bv+ba)', '(bv/ba)', '(bv/ba).2',
    '(bv+ba)[height>=720]', '(b/bv)[height<=360]', '((bv))', '(bv,ba)', 'bv/ba/b',
    'v1+v2', 'v1+v2+v3', 'a1+a2', 'a1+v1', 'v3+a1', 'a+b', 'a+c', 'c+a', 'sb+a1', 'a1+sb',
    'mergeall', 'all', 'all[ext=mp4]', 'mergeall[vcodec=none]', 'mergeall[height>=720]',
    # Multiple selections.
    'bv,ba', 'bv,ba,b', '18,140', 'bv+ba,b', 'b,b', '(bv,ba)/b', 'bv/ba,b', 'nonexistent,b',
    # Filters.
    'b[height<=360]', 'b[height<=?360]', 'bv[height<720]', 'bv[height>=720][fps=30]',
    'bv[height!=720]', 'bv[height=720]', '[height>=720]', '[ext=webm]', 'bv[ext=webm]',
    'ba[abr>100]', 'ba[abr>100k]', 'ba[filesize<2k]', 'ba[filesize<2kB]', 'bv[filesize<3KiB]',
    'bv[filesize>1M]', 'ba[filesize_approx>1]', 'bv[tbr>1000]', 'bv[vcodec^=avc1]',
    'bv[vcodec$=1F]', 'bv[vcodec*=4d40]', 'bv[vcodec~=^av01]', 'bv[vcodec~=\'^vp\\d\']',
    'bv[vcodec!^=avc]', 'bv[vcodec!*=4d]', 'ba[acodec=opus]', 'ba[language=en]',
    'ba[language=de]', 'ba[language!=en]', 'ba[language^=d]', 'b[format_id=18]',
    'b[format_id="18"]', "b[format_id='18']", 'b[format_note=360p]', 'bv[format_note*=p]',
    'bv[protocol=https]', 'b[protocol^=m3u8]', 'bv[dynamic_range=SDR]', 'bv[dynamic_range!=SDR]',
    'bv[unknown_field=x]', 'bv[unknown_field=?x]', 'bv[unknown_field>1]', 'bv[unknown_field>?1]',
    'b[height>360][ext=mp4]', 'b[ height <= 360 ]', 'b[format_note = "360p"]',
    "ba[format_note*='e d']", 'bv[height<=720]+ba[ext=m4a]', 'b[ext=mp4]/b[ext=webm]',
    'ba[abr>=48][abr<=50]', 'bv[fps>?20]', 'bv[width>1000]', 'b[height=360]',
    'bv[vcodec~=avc1|vp9]', 'b[language=en]', 'ba[language?=en]',
    # Quirks of Python's tokenizer and the operator joiner.
    'b est', 'best video', 'bv # comment', 'bv\t+ ba', 'bv++ba', 'bv//ba', 'b$t', 'bv?',
    '"bv"', "'bv'", 'bv"x"', 'bv;ba', 'bv:ba', 'b@t', 'b!t', 'bv*+ba*', 'bv+=ba',
    # Errors.
    'bv+', '+ba', '/b', ',b', 'bv/', 'bv,', '()', '(', ')', 'bv)', '(bv', '[', ']', 'bv[',
    'bv]', 'bv[height>]', 'bv[height>abc]', 'bv[height>1.2.3]', 'bv[height>1Q]',
    'bv[height>=5,6]', 'bv[=1]', 'bv[height]', 'bv(ba)', 'bv[height<720](ba)', 'bv{x}', '{',
    "bv'x", 'bv\\x', '',
    'bv[vcodec~=(]', 'b[format_id<a]', 'b[height^=3]', 'b[height~=3]',
]


def main():
    checkout, output = sys.argv[1:]
    sys.path.insert(0, checkout)
    from yt_dlp import YoutubeDL

    def selection(formats, spec):
        ydl = YoutubeDL({'quiet': True, 'no_warnings': True})
        incomplete = (all(f.get('vcodec') != 'none' and f.get('acodec') == 'none' for f in formats)
                      or all(f.get('vcodec') == 'none' and f.get('acodec') != 'none' for f in formats))
        ctx = {
            'formats': formats,
            'has_merged_format': any('none' not in (f.get('acodec'), f.get('vcodec')) for f in formats),
            'incomplete_formats': incomplete,
        }
        try:
            selected = list(ydl.build_format_selector(spec)(ctx))
        except Exception as error:  # noqa: BLE001 - every upstream failure is recorded
            return {'error': type(error).__name__, 'message': str(error)}
        keys = ('format_id', 'ext', 'format', 'protocol', 'language', 'format_note',
                'filesize_approx', 'tbr', 'width', 'height', 'vcodec', 'acodec', 'abr', 'vbr')
        return {'selected': [
            {
                **{key: f.get(key) for key in keys if key in f},
                'requested': [r['format_id'] for r in f.get('requested_formats', ())],
            } for f in selected
        ]}

    cases = []
    for name, formats in FORMAT_LISTS.items():
        for spec in SPECS:
            cases.append({'formats': name, 'spec': spec, **selection([dict(f) for f in formats], spec)})
    with open(output, 'w', encoding='utf-8') as file:
        json.dump({'format_lists': FORMAT_LISTS, 'cases': cases}, file, indent=1, ensure_ascii=False)
        file.write('\n')
    errors = sum('error' in case for case in cases)
    print(f'{len(cases)} cases ({errors} errors)')


if __name__ == '__main__':
    main()
