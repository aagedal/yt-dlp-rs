import contextlib
import copy
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_hls_audio import NATIVE_REQUESTS, PINNED_UPSTREAM, compare, main, verify_upstream

ORIGIN = "http://127.0.0.1:43210"
HEADERS = {"User-Agent": "fixture"}


class HlsAudioComparisonTests(unittest.TestCase):
    def setUp(self):
        self.upstream = [
            {"format_id": "main-English", "format_note": "English", "format_index": None,
             "url": ORIGIN + "/audio.m3u8", "manifest_url": ORIGIN + "/master.m3u8",
             "language": "en", "ext": "mp4", "protocol": "m3u8_native", "vcodec": "none",
             "audio_ext": "mp4", "video_ext": "none", "vbr": 0, "abr": None, "tbr": None},
            {"format_id": "140", "url": ORIGIN + "/video.m3u8?token=x",
             "manifest_url": ORIGIN + "/master.m3u8", "tbr": 140.0, "ext": "mp4", "fps": None,
             "protocol": "m3u8_native", "width": 1280, "height": 720, "vcodec": "avc1.64000a",
             "acodec": "none", "dynamic_range": None, "video_ext": "mp4", "audio_ext": "none",
             "abr": 0, "vbr": 140.0},
        ]
        self.native = copy.deepcopy(self.upstream)
        self.native[0].update({"resolution": "audio only", "aspect_ratio": None,
                               "http_headers": HEADERS, "format": "main-English - audio only (English)"})
        self.native[1].update({"dynamic_range": "SDR", "resolution": "1280x720", "aspect_ratio": 1.78,
                               "http_headers": HEADERS, "format": "140 - 1280x720"})

    def test_format_dicts_match_in_order_without_mutation(self):
        before = copy.deepcopy((self.native, self.upstream))
        actual = compare(self.native, self.upstream, ORIGIN)
        self.assertEqual([source["url"] for source in actual],
                         ["{{ORIGIN}}/audio.m3u8", "{{ORIGIN}}/video.m3u8?token=x"])
        self.assertEqual(actual[1]["dynamic_range"], "SDR")
        self.assertEqual((self.native, self.upstream), before)
        with self.assertRaisesRegex(ValueError, "external-audio format mismatch"):
            compare(self.native[::-1], self.upstream, ORIGIN)

    def test_every_value_key_set_and_missing_format_are_significant(self):
        for index, field, value in (
                (0, "url", ORIGIN + "/changed.m3u8"), (0, "ext", "aac"), (0, "vcodec", None),
                (0, "language", "fr"), (0, "format_note", "French"), (0, "format_id", "audio"),
                (1, "width", 1920), (1, "height", 1080), (1, "tbr", 141.0), (1, "acodec", None),
                (1, "manifest_url", ORIGIN + "/other.m3u8"), (1, "protocol", "m3u8"),
                (0, "stream_kind", "audio_only")):
            changed = copy.deepcopy(self.native)
            changed[index][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "format mismatch"):
                compare(changed, self.upstream, ORIGIN)
        changed = copy.deepcopy(self.native)
        del changed[0]["language"]
        with self.assertRaisesRegex(ValueError, "format mismatch"):
            compare(changed, self.upstream, ORIGIN)
        with self.assertRaisesRegex(ValueError, "format mismatch"):
            compare(self.native[:-1], self.upstream, ORIGIN)

    def test_only_exact_origin_is_normalized(self):
        for url in ("https://127.0.0.1:43210/audio", "http://127.0.0.1:432100/audio"):
            native = copy.deepcopy(self.native)
            native[0]["url"] = url
            upstream = copy.deepcopy(self.upstream)
            upstream[0]["url"] = url
            self.assertEqual(compare(native, upstream, ORIGIN)[0]["url"], url)

    def test_native_requests_are_pinned_manifest_paths_only(self):
        self.assertTrue(all(path.endswith(".m3u8") for path in NATIVE_REQUESTS))
        self.assertEqual(NATIVE_REQUESTS[0], "/audio-video/master.m3u8")

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
