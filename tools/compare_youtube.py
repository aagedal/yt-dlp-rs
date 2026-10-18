#!/usr/bin/env python3
"""Compare saved YouTube player-response metadata with an upstream checkout.

Development-only parser parity check: no requests, player API acquisition,
JavaScript, token acquisition, or download occurs. Rust fixture tests independently
consume the same expected JSON files. This does not establish live-site or client
request parity: the saved API response is injected after acquisition.
Only FIELDS and FORMAT_FIELDS are compared, preserving ordered formats. The sole
normalization replaces the exact fixed fixture origin with {{ORIGIN}} in URLs.
For the original watch-page fixture only, upstream's missing_pot option permits
parsing itag 22 without acquiring a token; this does not claim URL usability.
The API fixture uses ordinary anonymous WEB token policy, retaining exempt itag 18
and rejecting itag 22 without a GVS token.
"""

import argparse
import json
from pathlib import Path
import subprocess
import sys
from urllib.parse import urlsplit


ORIGIN = "http://127.0.0.1:43210"
FIELDS = ("id", "title", "extractor", "extractor_key", "webpage_url")
FORMAT_FIELDS = ("format_id", "ext", "width", "height", "url", "filesize")
FIXTURE_CASES = (
    ("progressive.html", "expected.json", "watch-page"),
    ("player-api.json", "player-api-expected.json", "player-api"),
)
UPSTREAM_WORKER = r'''
import json, socket, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.extractor.youtube import YoutubeIE
from yt_dlp.extractor.youtube import _video

def forbidden(*args, **kwargs):
    raise AssertionError('Network access forbidden in saved YouTube comparison')

socket.create_connection = forbidden
socket.socket = forbidden
page = Path(sys.argv[2]).read_text().replace('{{ORIGIN}}', sys.argv[3])
is_api = sys.argv[4] == 'player-api'

class FixtureYoutubeIE(YoutubeIE):
    _request_webpage = forbidden

    @classmethod
    def ie_key(cls):
        return YoutubeIE.ie_key()

    def _initial_extract(self, url, smuggled_data, webpage_url, webpage_client, video_id):
        response = json.loads(page) if is_api else self._search_json(
            self._YT_INITIAL_PLAYER_RESPONSE_RE, page, 'saved player response', video_id)
        response['streamingData'].update({
            _video.STREAMING_DATA_CLIENT_NAME: 'web',
            _video.STREAMING_DATA_FETCH_GVS_PO_TOKEN: lambda **kwargs: None,
            _video.STREAMING_DATA_PLAYER_TOKEN_PROVIDED: False,
            _video.STREAMING_DATA_IS_PREMIUM_SUBSCRIBER: False,
            _video.STREAMING_DATA_AVAILABLE_AT_TIMESTAMP: None,
        })
        return '' if is_api else page, {}, {}, False, [response], None

with YoutubeDL({'quiet': True, 'no_warnings': True, 'cachedir': False,
                'skip_download': True, 'extractor_args': {'youtube': {
                    'player_skip': ['js'], 'formats': [] if is_api else ['missing_pot'],
                    'skip': ['hls', 'dash', 'translated_subs']}}}) as ydl:
    ie = FixtureYoutubeIE(ydl)
    info = ie._real_extract('https://www.youtube.com/watch?v=BaW_jenozKc')
    info.update(extractor=ie.IE_NAME, extractor_key=ie.ie_key())
    print(json.dumps(info))
'''


def normalize(info, origin=ORIGIN):
    """Select exact comparison fields and normalize only the fixture origin."""
    result = {field: info[field] for field in FIELDS}
    result["formats"] = [
        {field: entry[field] for field in FORMAT_FIELDS}
        for entry in info["formats"]
    ]
    for entry in [result, *result["formats"]]:
        for field in ("url", "webpage_url"):
            if field not in entry:
                continue
            value = entry[field]
            parsed = urlsplit(value)
            if f"{parsed.scheme}://{parsed.netloc}" == origin:
                entry[field] = "{{ORIGIN}}" + value[len(origin):]
    return result


def compare(actual, expected):
    normalized = normalize(actual)
    if normalized != expected:
        raise ValueError("metadata mismatch: " + json.dumps(
            {"upstream": normalized, "expected": expected}, sort_keys=True))
    return normalized


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, type=Path)
    args = parser.parse_args()
    upstream = args.upstream.resolve()
    if not (upstream / "yt_dlp" / "extractor" / "youtube" / "_video.py").is_file():
        parser.error("--upstream must name an official yt-dlp source checkout")
    fixtures = Path(__file__).resolve().parents[1] / "tests" / "fixtures" / "youtube"
    try:
        for response_file, expected_file, mode in FIXTURE_CASES:
            completed = subprocess.run(
                [sys.executable, "-B", "-c", UPSTREAM_WORKER, str(upstream),
                 str(fixtures / response_file), ORIGIN, mode],
                capture_output=True, text=True, timeout=30, check=True)
            compare(json.loads(completed.stdout), json.loads((fixtures / expected_file).read_text()))
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print("YouTube saved watch-page and API metadata match (parser/token policy parity only)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
