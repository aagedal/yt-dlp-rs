import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_youtube_hls import CASES, ORIGIN, PINNED_UPSTREAM, compare, main, normalize

FIXTURES = Path(__file__).resolve().parents[1] / "tests/fixtures/youtube/hls"


class YoutubeHlsComparisonTests(unittest.TestCase):
    def setUp(self):
        self.expected = json.loads((FIXTURES / "expected.json").read_text(encoding="utf-8"))
        self.native = json.loads(json.dumps(self.expected).replace("{{ORIGIN}}", ORIGIN))
        self.upstream = copy.deepcopy(self.native)
        for fmt, itag in zip(self.upstream["formats"], ("95", "96"), strict=True):
            fmt.pop("stream_kind")
            fmt.update(format_id=itag, vcodec="avc1.42001e", acodec="mp4a.40.2", ext="mp4")

    def test_native_and_upstream_explicit_fields_match_without_mutation(self):
        original = copy.deepcopy((self.native, self.upstream))
        self.assertEqual(compare(self.native, self.expected), self.expected)
        self.assertEqual(compare(self.upstream, self.expected, upstream=True), self.expected)
        self.assertEqual((self.native, self.upstream), original)
        self.native["formats"][0].pop("stream_kind")
        self.assertEqual(compare(self.native, self.expected), self.expected)
        self.native["formats"][0]["ext"] = "ts"
        self.native["formats"][0]["fragments"] = [{"url": "ignored-by-metadata-only-check"}]
        self.assertEqual(compare(self.native, self.expected), self.expected)

    def test_order_count_urls_and_metadata_remain_significant(self):
        variants = [self.upstream["formats"][::-1], self.upstream["formats"][:1]]
        for field, value in (("width", 641), ("tbr", 401), ("language", "en"),
                             ("acodec", "none"), ("url", ORIGIN + "/changed/itag/95/index.m3u8")):
            changed = copy.deepcopy(self.upstream["formats"])
            changed[0][field] = value
            variants.append(changed)
        for formats in variants:
            with self.subTest(formats=formats), self.assertRaisesRegex(ValueError, "metadata mismatch"):
                compare(dict(self.upstream, formats=formats), self.expected, upstream=True)

    def test_itag_mapping_requires_known_id_and_matching_url(self):
        for itag in ("97", "96"):
            changed = copy.deepcopy(self.upstream)
            changed["formats"][0]["format_id"] = itag
            with self.subTest(itag=itag), self.assertRaisesRegex(ValueError, "itag/URL identity"):
                normalize(changed, upstream=True)
        self.upstream["formats"][0].pop("vcodec")
        with self.assertRaisesRegex(ValueError, "codec classification"):
            normalize(self.upstream, upstream=True)

    def test_only_exact_fixture_origin_is_normalized(self):
        for url in ("https://127.0.0.1:43210/itag/95/index.m3u8",
                    "http://127.0.0.1:432100/itag/95/index.m3u8"):
            self.native["formats"][0]["url"] = url
            self.assertEqual(normalize(self.native)["formats"][0]["url"], url)

    def test_saved_fixtures_exercise_policy_and_separately_verified_n_vector(self):
        plain = json.loads((FIXTURES / CASES[0]).read_text(encoding="utf-8"))
        challenged = json.loads((FIXTURES / CASES[1]).read_text(encoding="utf-8"))
        self.assertEqual(plain["streamingData"]["hlsManifestUrl"], "{{ORIGIN}}/hls/master.m3u8")
        self.assertEqual(plain["streamingData"]["dashManifestUrl"], "{{ORIGIN}}/forbidden.mpd")
        self.assertNotIn("formats", plain["streamingData"])
        self.assertEqual(challenged["streamingData"]["hlsManifestUrl"],
                         "{{ORIGIN}}/hls/n/IlLiA21ny7gqA2m4p37/file/index.m3u8")
        vector = json.loads((FIXTURES.parent / "challenges/expected.json").read_text(encoding="utf-8"))
        self.assertEqual(vector["requests"][0]["challenges"]["IlLiA21ny7gqA2m4p37"], "9nRTxrbM1f0yHg")

    def test_command_compares_both_cases_at_exact_pinned_revision(self):
        results = [subprocess.CompletedProcess([], 0, PINNED_UPSTREAM + "\n", "")]
        results += [subprocess.CompletedProcess([], 0, json.dumps(self.upstream), "") for _ in CASES]
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/extractor/youtube/_video.py"
            module.parent.mkdir(parents=True)
            module.touch()
            with mock.patch("sys.argv", ["compare_youtube_hls.py", "--upstream", directory]), \
                    mock.patch("compare_youtube_hls.subprocess.run", side_effect=results) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout:
                self.assertEqual(main(), 0)
            self.assertEqual(run.call_count, 3)
            self.assertEqual([call.args[0][-2] for call in run.call_args_list[1:]], list(CASES))
            self.assertIn("DASH skipped", stdout.getvalue())

    def test_bad_revision_fails_before_any_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / "yt_dlp/extractor/youtube/_video.py"
            module.parent.mkdir(parents=True)
            module.touch()
            with mock.patch("sys.argv", ["compare_youtube_hls.py", "--upstream", directory]), \
                    mock.patch("compare_youtube_hls.subprocess.run", return_value=subprocess.CompletedProcess(
                        [], 0, "0" * 40, "")) as run, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                self.assertEqual(main(), 1)
            self.assertEqual(run.call_count, 1)
            self.assertIn(PINNED_UPSTREAM, stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
