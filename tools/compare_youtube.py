#!/usr/bin/env python3
"""Compare saved YouTube player-response metadata with an upstream checkout.

Development-only metadata comparison: no requests, player API acquisition,
token acquisition, or download occurs. With explicit --deno, the signature case
runs pinned offline EJS with saved player/assets through upstream's actual
signature permutation requests. For the n-only challenge case, a
typed fixed solver response supplies the separately verified saved EJS vector;
this checks normalized metadata with that solved vector, not solver or player
acquisition parity. Rust fixture tests independently
consume the same expected JSON files. This does not establish live-site or client
request parity: the saved API response is injected after acquisition.
Only FIELDS and FORMAT_FIELDS are compared, preserving ordered formats. Adaptive
metadata additionally compares stream_kind, tbr and abr: upstream acodec/vcodec
flags map to stream kind, and audio-only total bitrate maps to audio bitrate
(as upstream format processing does). The URL normalization replaces the exact
fixed fixture origin with {{ORIGIN}} in URLs. For the original watch-page and
saved adaptive fixtures only, upstream's missing_pot option permits parsing
token-requiring itags without acquiring a token; this does not claim URL usability.
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
PINNED_UPSTREAM = "897341241f2bfb2439fe0367f4c7c9ccd29a3828"
FIELDS = ("id", "title", "extractor", "extractor_key", "webpage_url")
FORMAT_FIELDS = ("format_id", "ext", "width", "height", "url", "filesize")
FIXTURE_CASES = (
    ("progressive.html", "expected.json", "watch-page"),
    ("player-api.json", "player-api-expected.json", "player-api"),
    ("n-extraction.json", "n-extraction-expected.json", "n-extraction"),
    ("adaptive.json", "adaptive-expected.json", "adaptive"),
)
SIGNATURE_CASE = ("signature-extraction.json", "signature-extraction-expected.json",
                  "signature-extraction")
UPSTREAM_WORKER = r'''
import json, socket, sys
sys.stdout.reconfigure(encoding='utf-8')
from pathlib import Path
from types import SimpleNamespace
sys.path.insert(0, sys.argv[1])
from yt_dlp import YoutubeDL
from yt_dlp.extractor.youtube import YoutubeIE
from yt_dlp.extractor.youtube import _video
from yt_dlp.extractor.youtube.jsc.provider import (
    JsChallengeResponse, JsChallengeType, NChallengeInput, NChallengeOutput,
    SigChallengeInput, SigChallengeOutput)

def forbidden(*args, **kwargs):
    raise AssertionError('Network access forbidden in saved YouTube comparison')

socket.create_connection = forbidden
socket.socket = forbidden
page = Path(sys.argv[2]).read_text(encoding='utf-8').replace('{{ORIGIN}}', sys.argv[3])
is_api = sys.argv[4] == 'player-api'
is_n = sys.argv[4] == 'n-extraction'
is_signature = sys.argv[4] == 'signature-extraction'
is_adaptive = sys.argv[4] == 'adaptive'
is_json = is_api or is_n or is_signature or is_adaptive
has_player = is_n or is_signature
player_url = 'https://www.youtube.com/s/player/74edf1a3/player_ias.vflset/en_US/base.js'
solves = []

def fixed_n_solution(requests):
    assert is_n, 'Unexpected JavaScript challenge in non-challenge fixture'
    assert len(requests) == 1
    request, = requests
    assert request.type == JsChallengeType.N
    assert isinstance(request.input, NChallengeInput)
    assert request.video_id == 'BaW_jenozKc'
    assert request.input.player_url == player_url
    assert request.input.challenges == ['IlLiA21ny7gqA2m4p37']
    solves.append(request)
    yield request, JsChallengeResponse(
        type=JsChallengeType.N,
        output=NChallengeOutput(results={'IlLiA21ny7gqA2m4p37': '9nRTxrbM1f0yHg'}))

def real_signature_solution(requests):
    assert is_signature and len(requests) == 2
    assert {request.type for request in requests} == {JsChallengeType.N, JsChallengeType.SIG}
    root = Path(sys.argv[5])
    sys.path.insert(0, str(root / 'tools'))
    from compare_ejs import run_deno, verify_inputs
    from yt_dlp.extractor.youtube.jsc._builtin.ejs import EJSBaseJCP
    fixture = verify_inputs(root)
    signature = next(iter(fixture['requests'][1]['challenges']))
    for request in requests:
        assert request.video_id == 'BaW_jenozKc'
        assert request.input.player_url == player_url
        if request.type is JsChallengeType.N:
            assert isinstance(request.input, NChallengeInput)
            assert request.input.challenges == ['IlLiA21ny7gqA2m4p37']
        else:
            assert isinstance(request.input, SigChallengeInput)
            assert request.input.challenges == [''.join(map(chr, range(len(signature))))]
        solves.append(request)
    provider = SimpleNamespace(
        _lib_script=SimpleNamespace(code=(root / 'vendor/ejs/yt.solver.lib.js').read_text(encoding='utf-8')),
        _core_script=SimpleNamespace(code=(root / 'vendor/ejs/yt.solver.core.js').read_text(encoding='utf-8')))
    player = (root / 'tests/fixtures/youtube/challenges' / fixture['player_file']).read_text(encoding='utf-8')
    output = run_deno(Path(sys.argv[6]), EJSBaseJCP._construct_stdin(provider, player, False, requests))
    assert output['type'] == 'result'
    for request, response in zip(requests, output['responses'], strict=True):
        assert response['type'] == 'result'
        assert set(response['data']) == set(request.input.challenges)
        output_type = NChallengeOutput if request.type is JsChallengeType.N else SigChallengeOutput
        yield request, JsChallengeResponse(type=request.type, output=output_type(results=response['data']))

class FixtureYoutubeIE(YoutubeIE):
    _request_webpage = forbidden

    @classmethod
    def ie_key(cls):
        return YoutubeIE.ie_key()

    def _initial_extract(self, url, smuggled_data, webpage_url, webpage_client, video_id):
        response = json.loads(page) if is_json else self._search_json(
            self._YT_INITIAL_PLAYER_RESPONSE_RE, page, 'saved player response', video_id)
        response['streamingData'].update({
            _video.STREAMING_DATA_CLIENT_NAME: 'web',
            _video.STREAMING_DATA_FETCH_GVS_PO_TOKEN: lambda **kwargs: None,
            _video.STREAMING_DATA_PLAYER_TOKEN_PROVIDED: False,
            _video.STREAMING_DATA_IS_PREMIUM_SUBSCRIBER: False,
            _video.STREAMING_DATA_AVAILABLE_AT_TIMESTAMP: None,
        })
        return '' if is_json else page, {}, {}, False, [response], player_url if has_player else None

with YoutubeDL({'quiet': True, 'no_warnings': True, 'cachedir': False,
                'skip_download': True, 'extractor_args': {'youtube': {
                    'player_skip': [] if has_player else ['js'],
                    'formats': ['missing_pot'] if is_adaptive or not is_json else [],
                    'skip': ['hls', 'dash', 'translated_subs']}}}) as ydl:
    ie = FixtureYoutubeIE(ydl)
    ie.initialize()
    ie._jsc_director.bulk_solve = real_signature_solution if is_signature else fixed_n_solution
    info = ie._real_extract('https://www.youtube.com/watch?v=BaW_jenozKc')
    assert len(solves) == (2 if is_signature else int(is_n)), 'Unexpected challenge request count'
    info.update(extractor=ie.IE_NAME, extractor_key=ie.ie_key())
    print(json.dumps(info))
'''


def normalize(info, origin=ORIGIN, *, adaptive=False):
    """Select explicit fields; map adaptive codec flags/rates and fixture origin."""
    result = {field: info[field] for field in FIELDS}
    result["formats"] = [
        {field: entry[field] for field in FORMAT_FIELDS}
        for entry in info["formats"]
    ]
    if adaptive:
        for entry, source in zip(result["formats"], info["formats"], strict=True):
            kind = source.get("stream_kind")
            if kind is None:
                video, audio = source.get("vcodec"), source.get("acodec")
                if video == "none" and audio and audio != "none":
                    kind = "audio_only"
                elif audio == "none" and video and video != "none":
                    kind = "video_only"
                elif video and audio and video != "none" and audio != "none":
                    kind = "standalone"
                else:
                    raise ValueError("adaptive format lacks stream codec classification")
            entry.update(stream_kind=kind, tbr=source.get("tbr"),
                         abr=source.get("abr", source.get("tbr") if kind == "audio_only" else None))
    for entry in [result, *result["formats"]]:
        for field in ("url", "webpage_url"):
            if field not in entry:
                continue
            value = entry[field]
            parsed = urlsplit(value)
            if f"{parsed.scheme}://{parsed.netloc}" == origin:
                entry[field] = "{{ORIGIN}}" + value[len(origin):]
    return result


def compare(actual, expected, *, adaptive=False):
    normalized = normalize(actual, adaptive=adaptive)
    if normalized != expected:
        raise ValueError("metadata mismatch: " + json.dumps(
            {"upstream": normalized, "expected": expected}, sort_keys=True))
    return normalized


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--upstream", required=True, type=Path)
    parser.add_argument("--deno", type=Path,
                        help="Explicit Deno 2.9.7 path; enables offline EJS signature comparison")
    args = parser.parse_args()
    upstream = args.upstream.resolve()
    if not (upstream / "yt_dlp" / "extractor" / "youtube" / "_video.py").is_file():
        parser.error("--upstream must name an official yt-dlp source checkout")
    root = Path(__file__).resolve().parents[1]
    fixtures = root / "tests" / "fixtures" / "youtube"
    try:
        revision = subprocess.run(
            ["git", "-C", str(upstream), "rev-parse", "HEAD"],
            capture_output=True, text=True, encoding="utf-8", timeout=15, check=True,
        ).stdout.strip()
        if revision != PINNED_UPSTREAM:
            raise ValueError(f"--upstream must be checked out at {PINNED_UPSTREAM}")
        deno = args.deno.resolve(strict=True) if args.deno else None
        cases = (*FIXTURE_CASES, SIGNATURE_CASE) if deno else FIXTURE_CASES
        for response_file, expected_file, mode in cases:
            completed = subprocess.run(
                [sys.executable, "-B", "-c", UPSTREAM_WORKER, str(upstream),
                 str(fixtures / response_file), ORIGIN, mode, str(root), str(deno or '')],
                capture_output=True, text=True, encoding="utf-8", timeout=60, check=True)
            compare(json.loads(completed.stdout),
                    json.loads((fixtures / expected_file).read_text(encoding="utf-8")),
                    adaptive=mode == "adaptive")
    except (OSError, ValueError, KeyError, subprocess.SubprocessError) as error:
        print(f"comparison failed: {error}", file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        return 1
    print("YouTube saved watch-page, API, adaptive, and fixed n-vector metadata match "
          "(parser/token policy and normalized solved metadata only)")
    if deno:
        print("Signature/n metadata match using upstream's actual offline EJS permutation path")
    else:
        print("Signature comparison skipped: pass --deno PATH to enable pinned offline EJS")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
