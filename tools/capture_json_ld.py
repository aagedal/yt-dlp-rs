#!/usr/bin/env python3
"""Capture pinned upstream `_search_json_ld(html, id, default={})` results.

Development only: runs the upstream checkout's InfoExtractor method (no
network) for the pages below and saves `[html, info]` pairs to
tests/fixtures/json_ld/cases.json; src/extractor/json_ld.rs compares the
native extraction with them exactly.
"""
import argparse
import json
from pathlib import Path
import sys

LD = '<script type="application/ld+json">{}</script>'
PAGES = [
    LD.format('{"@context":"https://schema.org","@type":"VideoObject","name":"LD &amp; Video","description":"D","contentUrl":"https://e.invalid/a.mp4","thumbnailUrl":["//e.invalid/t.jpg","bad"],"uploadDate":"2024-01-02T03:04:05Z","duration":"PT1M2.5S","author":{"@type":"Person","name":"Ann"},"contentSize":"123.9","bitrate":"500","width":640,"height":"360","keywords":"a, b","encodingFormat":"video/mp4"}'),
    LD.format('{"@context":"https://schema.org","@type":"AudioObject","name":"Song","contentUrl":"http://e.invalid/s.mp3","bitrate":128,"byArtist":{"name":"Band"},"interactionStatistic":[{"@type":"InteractionCounter","interactionType":{"@type":"ListenAction"},"userInteractionCount":"1.234"},{"@type":"InteractionCounter","interactionType":"http://schema.org/LikeAction","userInteractionCount":5}]}'),
    LD.format('{"@context":"https://schema.org","@graph":[{"@type":"WebPage","name":"Page"},{"@type":"VideoObject","name":"G","embedUrl":"https://e.invalid/e","aggregateRating":{"ratingValue":"4.5"}}]}'),
    LD.format('{"@context":"https://schema.org","@type":"TVEpisode","name":"Ep","episodeNumber":"3","partOfSeason":{"@type":"TVSeason","name":"S1","seasonNumber":1},"partOfSeries":{"@type":"TVSeries","name":"Show"},"video":{"@type":"VideoObject","contentUrl":"https://e.invalid/ep.mp4","name":"Ep video"}}'),
    LD.format('{"@context":"https://schema.org","@type":"NewsArticle","headline":"H","datePublished":"2024-03-04T05:06:07.123+02:00","articleBody":"","description":"Fallback","video":[{"@type":"VideoObject","contentUrl":"https://e.invalid/n.mp4"}]}'),
    LD.format('{"@context":"https://schema.org","@type":"Movie","name":"M","duration":"PT2H","dateCreated":"2020-01-01"}'),
    LD.format('[{"@context":"https://schema.org","@type":"Organization","name":"O"},{"@context":"https://schema.org","@type":"VideoObject","name":"Second","contentUrl":"rtmp://e.invalid/x","hasPart":[{"@type":"Clip","name":"One","startOffset":0,"endOffset":10},{"@type":"Clip","name":"Two","startOffset":10}],"duration":"PT20S"}]'),
    LD.format('{"@type":"VideoObject","name":"No context"}') + LD.format('{"@context":"x","@type":"VideoObject","name":"bad json",}'),
    '<script type=application/ld+json>{"@context":"x","@type":["Thing","VideoObject"],"name":"Line\nbreak","contentUrl":"a.mp4"}</script>',
]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--upstream', required=True, type=Path)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    sys.path.insert(0, str(args.upstream))
    from yt_dlp import YoutubeDL
    from yt_dlp.extractor.common import InfoExtractor
    with YoutubeDL({'quiet': True, 'no_warnings': True}) as ydl:
        ie = InfoExtractor(ydl)
        cases = [[page, ie._search_json_ld(page, 'id', default={})] for page in PAGES]
    out = Path(__file__).resolve().parents[1] / 'tests/fixtures/json_ld/cases.json'
    text = json.dumps(cases, indent=1, ensure_ascii=False) + '\n'
    if args.write:
        out.write_text(text, encoding='utf-8')
    elif out.read_text(encoding='utf-8') != text:
        sys.exit('saved JSON-LD cases differ from upstream')
    print(f'{len(cases)} JSON-LD cases match pinned upstream')


if __name__ == '__main__':
    main()
