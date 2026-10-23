import contextlib
import copy
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_dash import (PINNED_UPSTREAM, NATIVE_LIST_RANGES,
                          compare, main, normalize, normalize_url, verify_upstream)

ORIGIN = "http://127.0.0.1:43210"
HEADERS = {"User-Agent": "fixture"}


class DashComparisonTests(unittest.TestCase):
    def setUp(self):
        self.upstream = [
            {"format_id": "0", "manifest_url": ORIGIN + "/manifest.mpd", "ext": "mp4",
             "width": 640, "height": 360, "tbr": 500.0, "fps": None, "language": None,
             "format_note": "DASH video", "container": "mp4_dash", "vcodec": "avc1",
             "acodec": "none", "dynamic_range": None, "url": ORIGIN + "/manifest.mpd",
             "fragment_base_url": ORIGIN + "/base/",
             "fragments": [{"path": "init.mp4"}, {"path": "segment.m4s?token=one", "duration": 1.0}],
             "protocol": "http_dash_segments", "manifest_stream_number": 0,
             "is_dash_periods": True, "video_ext": "mp4", "audio_ext": "none", "abr": 0, "vbr": 500.0},
            {"format_id": "1", "manifest_url": ORIGIN + "/manifest.mpd", "ext": "m4a",
             "tbr": 128.0, "language": "en", "format_note": "DASH audio", "container": "m4a_dash",
             "vcodec": "none", "acodec": "mp4a", "dynamic_range": None,
             "url": ORIGIN + "/manifest.mpd", "fragment_base_url": ORIGIN + "/base/",
             "fragments": [{"url": ORIGIN + "/base/audio.m4s"}],
             "protocol": "http_dash_segments", "manifest_stream_number": 1,
             "is_dash_periods": True, "audio_ext": "m4a", "video_ext": "none", "vbr": 0, "abr": 128.0}]
        self.native = copy.deepcopy(self.upstream)
        self.native[0].update({"dynamic_range": "SDR", "resolution": "640x360", "aspect_ratio": 1.78,
                               "http_headers": HEADERS, "format": "0 - 640x360 (DASH video)"})
        self.native[1].update({"resolution": "audio only", "aspect_ratio": None,
                               "http_headers": HEADERS, "format": "1 - audio only (DASH audio)"})

    def test_format_dicts_match_with_processed_fields_and_sdr_rule_without_mutation(self):
        before = copy.deepcopy((self.native, self.upstream))
        result = compare(self.native, self.upstream, ORIGIN)
        self.assertEqual(result[0]["fragment_base_url"], "{{ORIGIN}}/base/")
        self.assertEqual(result[0]["dynamic_range"], "SDR")
        self.assertIsNone(result[1]["dynamic_range"])
        self.assertEqual(result[1]["fragments"][0]["url"], "{{ORIGIN}}/base/audio.m4s")
        self.assertNotIn("http_headers", result[0])
        self.assertEqual((self.native, self.upstream), before)

    def test_every_compared_value_queries_fragment_order_and_count_are_significant(self):
        changes = []
        for field, value in (("width", 1280), ("height", 720), ("ext", "webm"), ("tbr", 600.0),
                             ("vcodec", "avc1.64001f"), ("acodec", "mp4a"), ("fps", 30),
                             ("format_note", "other"), ("container", "webm_dash"),
                             ("protocol", "m3u8_native"), ("manifest_stream_number", 1),
                             ("is_dash_periods", False), ("dynamic_range", "HDR10"),
                             ("url", ORIGIN + "/other.mpd"), ("manifest_url", ORIGIN + "/other.mpd"),
                             ("fragment_base_url", ORIGIN + "/other/"), ("format_id", "video-0")):
            changed = copy.deepcopy(self.native)
            changed[0][field] = value
            changes.append(changed)
        for field, value in (("abr", 96.0), ("vbr", None), ("language", "nb")):
            changed = copy.deepcopy(self.native)
            changed[1][field] = value
            changes.append(changed)
        for fragments in (self.native[0]["fragments"][::-1], self.native[0]["fragments"][:-1],
                          [{"path": "init.mp4"}, {"path": "segment.m4s?token=two", "duration": 1.0}],
                          [{"path": "init.mp4"}, {"path": "segment.m4s?token=one", "duration": 2.0}],
                          [{"path": "init.mp4"}, {"url": ORIGIN + "/base/segment.m4s?token=one",
                                                  "duration": 1.0}]):
            changed = copy.deepcopy(self.native)
            changed[0]["fragments"] = fragments
            changes.append(changed)
        for changed in changes:
            with self.subTest(changed=changed[0]), self.assertRaisesRegex(ValueError, "DASH format mismatch"):
                compare(changed, self.upstream, ORIGIN)

    def test_key_sets_must_match_except_processed_fields(self):
        extra = copy.deepcopy(self.native)
        extra[0]["stream_kind"] = "video_only"
        missing = copy.deepcopy(self.native)
        del missing[1]["container"]
        for changed in (extra, missing):
            with self.subTest(changed=changed), self.assertRaisesRegex(ValueError, "DASH format mismatch"):
                compare(changed, self.upstream, ORIGIN)
        upstream_extra = copy.deepcopy(self.upstream)
        upstream_extra[0]["format"] = "only native processing adds this"
        with self.assertRaisesRegex(ValueError, "DASH format mismatch"):
            compare(self.native, upstream_extra, ORIGIN)

    def test_sorted_order_is_significant(self):
        with self.assertRaisesRegex(ValueError, "DASH format mismatch"):
            compare(self.native[::-1], self.upstream, ORIGIN)
        with self.assertRaisesRegex(ValueError, "DASH format mismatch"):
            compare(self.native, self.upstream[::-1], ORIGIN)

    def test_range_omission_is_never_implicitly_normalized(self):
        native = copy.deepcopy(self.native)
        native[0]["fragments"][0]["byte_range"] = {"start": 0, "end": 16}
        with self.assertRaisesRegex(ValueError, "DASH format mismatch"):
            compare(native, self.upstream, ORIGIN)
        ranges = [[{"start": 0, "end": 16}, None], [None]]
        compare(native, self.upstream, ORIGIN, native_only_ranges=ranges)
        native[0]["fragments"][0]["byte_range"]["end"] = 17
        with self.assertRaisesRegex(ValueError, "saved SegmentList ranges"):
            compare(native, self.upstream, ORIGIN, native_only_ranges=ranges)
        with self.assertRaisesRegex(ValueError, "format count"):
            compare(native, self.upstream, ORIGIN, native_only_ranges=[])
        self.assertEqual(NATIVE_LIST_RANGES[0][-1], {"start": 24, "end": 48})

    def test_exact_origin_only(self):
        for url in ("http://127.0.0.1:432100/path", "https://127.0.0.1:43210/path", "https://example.invalid/path"):
            self.assertEqual(normalize_url(url, ORIGIN), url)
        self.assertEqual(normalize([{"url": ORIGIN + "/a"}], ORIGIN)[0]["url"], "{{ORIGIN}}/a")

    def test_pin_is_verified_before_binary_or_server(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "yt_dlp/extractor/common.py"
            source.parent.mkdir(parents=True)
            source.write_text("", encoding="utf-8")
            with mock.patch("compare_dash.subprocess.run", return_value=subprocess.CompletedProcess([], 0, stdout="wrong-pin\n")), self.assertRaisesRegex(ValueError, PINNED_UPSTREAM):
                verify_upstream(root)
        with mock.patch("sys.argv", ["compare_dash.py", "--upstream", "/absent", "--binary", "/absent-binary"]), mock.patch("compare_dash.verify_upstream", side_effect=ValueError("bad pin")), mock.patch("compare_dash.manifest_server") as server, contextlib.redirect_stderr(io.StringIO()) as errors:
            self.assertEqual(main(), 1)
            self.assertIn("bad pin", errors.getvalue())
            server.assert_not_called()


if __name__ == "__main__":
    unittest.main()
