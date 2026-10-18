#!/usr/bin/env python3
"""Compare saved YouTube WEB HLS metadata through actual pinned YoutubeIE.

Development-only, offline reference tooling. Inject saved player responses with
WEB client policy, anonymous/non-premium state and a token provider returning
None. Do not enable missing_pot or override policy: HLS must reach the real saved
master parser while DASH must be skipped before manifest acquisition. Network
access, DASH acquisition and fragment/key downloads are forbidden.

The path-n case supplies a typed fixed solver response with the separately
verified saved EJS vector. This verifies discovery, policy, path replacement and
normalized metadata, not EJS execution or player/API acquisition parity.

Compare only FIELDS and FORMAT_FIELDS, preserving declaration order. Upstream
itag IDs 95/96 map explicitly to native hls-0/hls-1; the itag in each URL must
agree. Codec flags map to stream_kind. Missing language becomes null. Replace
only the exact fixed fixture origin in URLs. Native Rust tests consume the same
expected JSON. Exclude extension/container (native TS versus upstream MP4 label
and possible remux), filesize, ranking, selection, subtitles, fragments and
transport: the upstream master parser does not construct segment download plans.
"""
import argparse
import json
from pathlib import Path
import subprocess
import sys
from urllib.parse import urlsplit

PINNED_UPSTREAM = "897341241f2bfb2439fe0367f4c7c9ccd29a3828"
ORIGIN = "http://127.0.0.1:43210"
FIELDS = ("id", "title", "extractor", "extractor_key", "webpage_url")
FORMAT_FIELDS = ("format_id", "stream_kind", "url", "width", "height", "tbr", "language")
ITAG_IDS = {"95": "hls-0", "96": "hls-1"}
CASES = ("player-response.json", "player-response-n.json")
UPSTREAM_WORKER = r'''
import json, socket, sys
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.extractor.youtube import YoutubeIE, _video
from yt_dlp.extractor.youtube.jsc.provider import (
    JsChallengeResponse, JsChallengeType, NChallengeInput, NChallengeOutput)

def forbidden(*args, **kwargs):
    raise AssertionError('Network, DASH or segment/key acquisition forbidden in saved YouTube HLS comparison')
socket.socket = socket.create_connection = forbidden
root, filename, origin = Path(sys.argv[2]), sys.argv[3], sys.argv[4]
is_n = filename == 'player-response-n.json'
response = json.loads((root / filename).read_text(encoding='utf-8').replace('{{ORIGIN}}', origin))
master = (root / 'master.m3u8').read_text(encoding='utf-8').replace('{{ORIGIN}}', origin)
player_url = 'https://www.youtube.com/s/player/74edf1a3/player_ias.vflset/en_US/base.js'
manifest_calls, solves, token_calls = [], [], []

def token_provider(**kwargs):
    token_calls.append(kwargs)
    return None

def fixed_solution(requests):
    assert is_n and len(requests) == 1
    request, = requests
    assert request.type == JsChallengeType.N and isinstance(request.input, NChallengeInput)
    assert request.video_id == 'BaW_jenozKc'
    assert request.input.player_url == player_url
    assert request.input.challenges == ['IlLiA21ny7gqA2m4p37']
    solves.append(request)
    yield request, JsChallengeResponse(type=JsChallengeType.N, output=NChallengeOutput(
        results={'IlLiA21ny7gqA2m4p37': '9nRTxrbM1f0yHg'}))

class FixtureYoutubeIE(YoutubeIE):
    _request_webpage = forbidden
    _extract_mpd_formats_and_subtitles = forbidden
    @classmethod
    def ie_key(cls):
        return YoutubeIE.ie_key()
    def _initial_extract(self, url, smuggled_data, webpage_url, webpage_client, video_id):
        response['streamingData'].update({
            _video.STREAMING_DATA_CLIENT_NAME: 'web',
            _video.STREAMING_DATA_FETCH_GVS_PO_TOKEN: token_provider,
            _video.STREAMING_DATA_PLAYER_TOKEN_PROVIDED: False,
            _video.STREAMING_DATA_IS_PREMIUM_SUBSCRIBER: False,
            _video.STREAMING_DATA_AVAILABLE_AT_TIMESTAMP: None,
        })
        return '', {}, {}, False, [response], player_url if is_n else None
    def _extract_m3u8_formats_and_subtitles(self, url, video_id, ext=None, **kwargs):
        expected = origin + ('/hls/n/9nRTxrbM1f0yHg/file/index.m3u8' if is_n else '/hls/master.m3u8')
        assert url == expected, (url, expected)
        manifest_calls.append(url)
        return self._parse_m3u8_formats_and_subtitles(master, url, ext=ext, live=False)

with YoutubeDL({'quiet': True, 'no_warnings': True, 'cachedir': False,
                'skip_download': True, 'extractor_args': {'youtube': {
                    'player_skip': [] if is_n else ['js'], 'skip': ['translated_subs']}}}) as ydl:
    ydl.urlopen = forbidden
    ie = FixtureYoutubeIE(ydl)
    ie.initialize()
    ie._jsc_director.bulk_solve = fixed_solution
    info = ie._real_extract('https://www.youtube.com/watch?v=BaW_jenozKc')
    assert len(manifest_calls) == 1 and len(solves) == int(is_n)
    assert token_calls and all(call == {'required': True} for call in token_calls)
    info.update(extractor=ie.IE_NAME, extractor_key=ie.ie_key())
    print(json.dumps(info))
'''


def normalize(info, *, upstream=False, origin=ORIGIN):
    result = {field: info[field] for field in FIELDS}
    result["formats"] = []
    for source in info["formats"]:
        entry = {field: source.get(field) for field in FORMAT_FIELDS}
        if upstream:
            itag = source["format_id"]
            if itag not in ITAG_IDS or f"/itag/{itag}/" not in urlsplit(source["url"]).path:
                raise ValueError("unexpected upstream HLS itag/URL identity")
            entry["format_id"] = ITAG_IDS[itag]
            video, audio = source.get("vcodec"), source.get("acodec")
            if video == "none" and audio and audio != "none":
                entry["stream_kind"] = "audio_only"
            elif audio == "none" and video and video != "none":
                entry["stream_kind"] = "video_only"
            elif video and audio and video != "none" and audio != "none":
                entry["stream_kind"] = "standalone"
            else:
                raise ValueError("missing upstream HLS codec classification")
        elif entry["stream_kind"] is None:
            entry["stream_kind"] = "standalone"
        result["formats"].append(entry)
    for entry in [result, *result["formats"]]:
        for field in ("url", "webpage_url"):
            if field not in entry:
                continue
            value = entry[field]
            parsed = urlsplit(value)
            if f"{parsed.scheme}://{parsed.netloc}" == origin:
                entry[field] = "{{ORIGIN}}" + value[len(origin):]
    return result


def compare(info, expected, *, upstream=False):
    actual = normalize(info, upstream=upstream)
    if actual != expected:
        raise ValueError("YouTube HLS metadata mismatch: " + json.dumps(
            {"actual": actual, "expected": expected}, sort_keys=True))
    return actual


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, type=Path)
    args = parser.parse_args()
    upstream = args.upstream.resolve()
    try:
        if not (upstream / "yt_dlp/extractor/youtube/_video.py").is_file():
            raise ValueError("--upstream must name an official yt-dlp source checkout")
        revision = subprocess.run(["git", "-C", str(upstream), "rev-parse", "HEAD"],
                                  capture_output=True, text=True, encoding="utf-8", timeout=15,
                                  check=True).stdout.strip()
        if revision != PINNED_UPSTREAM:
            raise ValueError(f"--upstream must be checked out at {PINNED_UPSTREAM}")
        fixtures = Path(__file__).resolve().parents[1] / "tests/fixtures/youtube/hls"
        expected = json.loads((fixtures / "expected.json").read_text(encoding="utf-8"))
        for filename in CASES:
            result = subprocess.run([sys.executable, "-B", "-c", UPSTREAM_WORKER,
                                     str(upstream), str(fixtures), filename, ORIGIN],
                                    capture_output=True, text=True, encoding="utf-8", timeout=30,
                                    check=True)
            compare(json.loads(result.stdout), expected, upstream=True)
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print("Saved YouTube WEB HLS/plain and fixed path-n metadata match pinned upstream "
          "(ordinary token policy; DASH skipped; no transport/container/EJS parity claim)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
