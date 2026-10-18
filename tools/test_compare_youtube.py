import copy
import contextlib
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock
from urllib.parse import parse_qs, urlencode, urlsplit

from compare_youtube import (
    FIXTURE_CASES, SIGNATURE_CASE, ORIGIN, PINNED_UPSTREAM, compare, main, normalize)


class YoutubeComparisonTest(unittest.TestCase):
    def setUp(self):
        fixture = Path(__file__).resolve().parents[1] / "tests/fixtures/youtube/expected.json"
        self.expected = json.loads(fixture.read_text(encoding="utf-8"))
        self.info = json.loads(fixture.read_text(encoding="utf-8").replace("{{ORIGIN}}", ORIGIN))

    def test_exact_fixture_matches_without_mutating_input(self):
        original = copy.deepcopy(self.info)
        self.info["duration"] = 10  # Outside the explicit comparison subset.
        self.assertEqual(compare(self.info, self.expected), self.expected)
        self.assertEqual(self.info["formats"], original["formats"])

    def test_only_exact_origin_is_normalized(self):
        for url in (ORIGIN + "0/media/low.mp4", "https://127.0.0.1:43210/media/low.mp4"):
            with self.subTest(url=url):
                self.info["formats"][0]["url"] = url
                self.assertEqual(normalize(self.info)["formats"][0]["url"], url)
        self.info["formats"][0]["url"] = ORIGIN + "/media/low.mp4?a=1&b=2#part"
        self.assertEqual(normalize(self.info)["formats"][0]["url"],
                         "{{ORIGIN}}/media/low.mp4?a=1&b=2#part")

    def test_format_order_count_and_metadata_are_significant(self):
        for formats in (self.info["formats"][::-1], self.info["formats"][:1]):
            with self.subTest(formats=formats), self.assertRaisesRegex(ValueError, "metadata mismatch"):
                compare(dict(self.info, formats=formats), self.expected)
        self.info["formats"][0]["width"] = 641
        with self.assertRaisesRegex(ValueError, "metadata mismatch"):
            compare(self.info, self.expected)

    def test_api_fixture_expects_only_token_exempt_format(self):
        fixtures = Path(__file__).resolve().parents[1] / "tests/fixtures/youtube"
        expected = json.loads((fixtures / "player-api-expected.json").read_text(encoding="utf-8"))
        response = json.loads((fixtures / "player-api.json").read_text(encoding="utf-8"))
        self.assertEqual([fmt["itag"] for fmt in response["streamingData"]["formats"]], [18, 22])
        self.assertEqual([fmt["format_id"] for fmt in expected["formats"]], ["18"])
        info = json.loads(json.dumps(expected).replace("{{ORIGIN}}", ORIGIN))
        self.assertEqual(compare(info, expected), expected)
        info["formats"].append(self.info["formats"][1])
        with self.assertRaisesRegex(ValueError, "metadata mismatch"):
            compare(info, expected)

    def test_n_fixture_expects_rewritten_challenge_and_preserves_metadata(self):
        fixtures = Path(__file__).resolve().parents[1] / "tests/fixtures/youtube"
        response = json.loads((fixtures / "n-extraction.json").read_text(encoding="utf-8"))
        expected = json.loads((fixtures / "n-extraction-expected.json").read_text(encoding="utf-8"))
        self.assertEqual(response["assets"]["js"],
                         "https://www.youtube.com/s/player/74edf1a3/player_ias.vflset/en_US/base.js")
        original, = response["streamingData"]["formats"]
        rewritten, = expected["formats"]
        self.assertEqual(original["itag"], 18)
        self.assertEqual(original["url"], "{{ORIGIN}}/media.mp4?n=IlLiA21ny7gqA2m4p37")
        self.assertEqual(rewritten["url"], "{{ORIGIN}}/media.mp4?n=9nRTxrbM1f0yHg")
        info = json.loads(json.dumps(expected).replace("{{ORIGIN}}", ORIGIN))
        self.assertEqual(compare(info, expected), expected)
        info["formats"][0]["url"] = original["url"].replace("{{ORIGIN}}", ORIGIN)
        with self.assertRaisesRegex(ValueError, "metadata mismatch"):
            compare(info, expected)

    def test_command_compares_all_saved_cases(self):
        fixtures = Path(__file__).resolve().parents[1] / "tests/fixtures/youtube"
        results = [subprocess.CompletedProcess([], 0, PINNED_UPSTREAM + "\n", "")]
        results.extend(subprocess.CompletedProcess([], 0, (fixtures / name).read_text(encoding="utf-8"), "")
                       for _, name, _ in FIXTURE_CASES)
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/extractor/youtube/_video.py"
            module.parent.mkdir(parents=True)
            module.touch()
            with mock.patch("sys.argv", ["compare_youtube.py", "--upstream", directory]), \
                    mock.patch("compare_youtube.subprocess.run", side_effect=results) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout:
                self.assertEqual(main(), 0)
            self.assertIn("Signature comparison skipped", stdout.getvalue())
            self.assertEqual(run.call_count, 4)
            self.assertEqual(run.call_args_list[0].args[0],
                             ["git", "-C", str(upstream.resolve()), "rev-parse", "HEAD"])
            self.assertEqual([call.args[0][-3] for call in run.call_args_list[1:]],
                             ["watch-page", "player-api", "n-extraction"])

    def test_signature_fixture_contains_real_cipher_and_exact_saved_solution(self):
        fixtures = Path(__file__).resolve().parents[1] / "tests/fixtures/youtube"
        response = json.loads((fixtures / SIGNATURE_CASE[0]).read_text(encoding="utf-8"))
        expected = json.loads((fixtures / SIGNATURE_CASE[1]).read_text(encoding="utf-8"))
        vectors = json.loads((fixtures / "challenges/expected.json").read_text(encoding="utf-8"))
        signature, solution = next(iter(vectors["requests"][1]["challenges"].items()))
        fmt, = response["streamingData"]["formats"]
        self.assertNotIn("url", fmt)
        self.assertEqual(fmt["itag"], 18)
        cipher = parse_qs(fmt["signatureCipher"], strict_parsing=True)
        self.assertEqual(cipher, {"s": [signature], "sp": ["lsig"],
                                 "url": ["{{ORIGIN}}/media.mp4?n=IlLiA21ny7gqA2m4p37&keep=fixture"]})
        self.assertEqual(expected["formats"][0]["url"],
                         "{{ORIGIN}}/media.mp4?n=9nRTxrbM1f0yHg&keep=fixture&"
                         + urlencode({"lsig": solution}))
        query = parse_qs(urlsplit(expected["formats"][0]["url"]).query)
        self.assertEqual(query, {"n": ["9nRTxrbM1f0yHg"], "keep": ["fixture"], "lsig": [solution]})

    def test_explicit_runtime_enables_signature_comparison(self):
        fixtures = Path(__file__).resolve().parents[1] / "tests/fixtures/youtube"
        results = [subprocess.CompletedProcess([], 0, PINNED_UPSTREAM, "")]
        results.extend(subprocess.CompletedProcess([], 0,
                       (fixtures / name).read_text(encoding="utf-8"), "")
                       for _, name, _ in (*FIXTURE_CASES, SIGNATURE_CASE))
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            module = root / "yt_dlp/extractor/youtube/_video.py"
            module.parent.mkdir(parents=True)
            module.touch()
            deno = root / "explicit-deno"
            deno.touch()
            with mock.patch("sys.argv", ["compare_youtube.py", "--upstream", directory,
                                          "--deno", str(deno)]), \
                    mock.patch("compare_youtube.subprocess.run", side_effect=results) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout:
                self.assertEqual(main(), 0)
            self.assertEqual(run.call_count, 5)
            self.assertEqual(run.call_args.args[0][-3], "signature-extraction")
            self.assertEqual(run.call_args.args[0][-1], str(deno.resolve()))
            self.assertEqual(run.call_args.kwargs["encoding"], "utf-8")
            self.assertIn("actual offline EJS permutation path", stdout.getvalue())
            self.assertNotIn("skipped", stdout.getvalue())

    def test_unpinned_upstream_fails_before_workers_or_runtime_resolution(self):
        for runtime_args in ([], ["--deno", "/nonexistent-explicit-runtime"]):
            with self.subTest(runtime_args=runtime_args), tempfile.TemporaryDirectory() as directory:
                upstream = Path(directory)
                module = upstream / "yt_dlp/extractor/youtube/_video.py"
                module.parent.mkdir(parents=True)
                module.touch()
                revision = subprocess.CompletedProcess([], 0, "0" * 40 + "\n", "")
                with mock.patch("sys.argv", ["compare_youtube.py", "--upstream", directory,
                                              *runtime_args]), \
                        mock.patch("compare_youtube.subprocess.run", return_value=revision) as run, \
                        contextlib.redirect_stderr(io.StringIO()) as stderr, \
                        contextlib.redirect_stdout(io.StringIO()) as stdout:
                    self.assertEqual(main(), 1)
                self.assertEqual(run.call_count, 1)
                self.assertEqual(run.call_args.args[0],
                                 ["git", "-C", str(upstream.resolve()), "rev-parse", "HEAD"])
                self.assertIn(f"--upstream must be checked out at {PINNED_UPSTREAM}", stderr.getvalue())
                self.assertEqual(stdout.getvalue(), "")


if __name__ == "__main__":
    unittest.main()
