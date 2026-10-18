import contextlib
import copy
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_hls_audio import PINNED_UPSTREAM, compare, main, normalize, verify_upstream

ORIGIN = "http://127.0.0.1:43210"


class HlsAudioComparisonTests(unittest.TestCase):
    def setUp(self):
        self.native = [
            {"url": ORIGIN + "/video.m3u8?token=x", "stream_kind": "video_only",
             "width": 1280, "height": 720, "tbr": 140.0, "audio_group": "main"},
            {"url": ORIGIN + "/audio.m3u8", "stream_kind": "audio_only",
             "language": "en", "format_note": "English", "audio_default": True,
             "audio_autoselect": True, "ext": "aac"},
        ]
        self.upstream = [
            {"url": ORIGIN + "/audio.m3u8", "vcodec": "none", "language": "en",
             "format_note": "English", "ext": None},
            {"url": ORIGIN + "/video.m3u8?token=x", "acodec": "none",
             "width": 1280, "height": 720, "tbr": 140.0, "_audio_group_id": "main"},
        ]

    def test_url_order_normalization_is_explicit_and_inputs_remain_unchanged(self):
        before = copy.deepcopy((self.native, self.upstream))
        actual = compare(self.native, self.upstream, ORIGIN)
        self.assertEqual([source["url"] for source in actual],
                         ["{{ORIGIN}}/audio.m3u8", "{{ORIGIN}}/video.m3u8?token=x"])
        self.assertEqual(actual[0]["stream_kind"], "audio_only")
        self.assertEqual(actual[1]["stream_kind"], "video_only")
        self.assertEqual(compare(self.native[::-1], self.upstream[::-1], ORIGIN), actual)
        self.assertEqual((self.native, self.upstream), before)
        actual[0]["language"] = "changed"
        self.assertEqual((self.native, self.upstream), before)

    def test_every_allowlisted_field_difference_and_missing_format_are_errors(self):
        for index, field, value in (
                (0, "url", ORIGIN + "/changed.m3u8"), (0, "stream_kind", "standalone"),
                (0, "width", 1920), (0, "height", 1080), (0, "tbr", 141.0),
                (1, "language", "fr"), (1, "format_note", "French")):
            changed = copy.deepcopy(self.native)
            changed[index][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "metadata mismatch"):
                compare(changed, self.upstream, ORIGIN)
        with self.assertRaisesRegex(ValueError, "metadata mismatch"):
            compare(self.native[:-1], self.upstream, ORIGIN)

    def test_duplicates_fail_in_either_list_instead_of_collapsing(self):
        for native, upstream in ((self.native + self.native, self.upstream),
                                 (self.native, self.upstream + self.upstream)):
            with self.assertRaisesRegex(ValueError, "duplicate format URL"):
                compare(native, upstream, ORIGIN)

    def test_optional_fields_ignored_fields_and_exact_origin_normalization(self):
        native = [{"url": ORIGIN + "/muxed", "format_id": "native", "audio_default": True}]
        upstream = [{"url": ORIGIN + "/muxed", "format_id": "upstream", "language": None}]
        self.assertEqual(compare(native, upstream, ORIGIN)[0]["stream_kind"], "standalone")
        for url in ("https://127.0.0.1:43210/audio", "http://127.0.0.1:432100/audio",
                    "https://example.invalid/audio"):
            self.assertEqual(normalize([{"url": url}], ORIGIN)[0]["url"], url)
        with self.assertRaises(KeyError):
            normalize([{}], ORIGIN)
        with self.assertRaisesRegex(ValueError, "unknown native stream kind"):
            normalize([{"url": ORIGIN + "/audio", "stream_kind": "unknown"}], ORIGIN)
        with self.assertRaisesRegex(ValueError, "neither audio nor video"):
            normalize([{"url": ORIGIN + "/audio", "vcodec": "none", "acodec": "none"}],
                      ORIGIN, upstream=True)

    def test_requires_exact_pinned_checkout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            module = root / "yt_dlp/downloader/hls.py"
            module.parent.mkdir(parents=True)
            module.touch()
            for revision, succeeds in ((PINNED_UPSTREAM, True), ("0" * 40, False)):
                with self.subTest(revision=revision), mock.patch(
                        "compare_hls.subprocess.run", return_value=subprocess.CompletedProcess(
                            [], 0, revision + "\n", "")):
                    if succeeds:
                        verify_upstream(root)
                    else:
                        with self.assertRaisesRegex(ValueError, PINNED_UPSTREAM):
                            verify_upstream(root)

    def test_bad_pin_precedes_binary_and_server_access(self):
        with mock.patch("sys.argv", ["compare_hls_audio.py", "--upstream", "/saved/upstream",
                                     "--binary", "/nonexistent-binary"]), \
                mock.patch("compare_hls_audio.verify_upstream", side_effect=ValueError(PINNED_UPSTREAM)), \
                mock.patch("compare_hls_audio.manifest_server") as server, \
                mock.patch("compare_hls_audio.subprocess.run") as run, \
                contextlib.redirect_stderr(io.StringIO()) as stderr:
            self.assertEqual(main(), 1)
        server.assert_not_called()
        run.assert_not_called()
        self.assertIn(PINNED_UPSTREAM, stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
