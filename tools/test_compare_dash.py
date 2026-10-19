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


class DashComparisonTests(unittest.TestCase):
    def setUp(self):
        self.native = [{"format_id": "0", "audio_group": "dash-period-0",
                        "stream_kind": "video_only", "url": ORIGIN + "/manifest.mpd", "ext": "mp4", "tbr": 500.0,
                        "width": 640, "height": 360,
                        "fragments": [{"url": ORIGIN + "/base/init.mp4"},
                                      {"url": ORIGIN + "/base/segment.m4s?token=one"}]},
                       {"format_id": "1", "audio_group": "dash-period-0",
                        "stream_kind": "audio_only", "url": ORIGIN + "/manifest.mpd", "ext": "m4a", "tbr": 128.0,
                        "abr": 128.0, "language": "en",
                        "fragments": [{"url": ORIGIN + "/base/audio.m4s"}]}]
        self.upstream = [{"format_id": "0", "vcodec": "avc1", "acodec": "none",
                          "ext": "mp4", "url": ORIGIN + "/manifest.mpd", "tbr": 500.0, "width": 640, "height": 360,
                          "fragment_base_url": ORIGIN + "/base/",
                          "fragments": [{"path": "init.mp4"}, {"path": "segment.m4s?token=one"}]},
                         {"format_id": "1", "vcodec": "none", "acodec": "mp4a",
                          "ext": "m4a", "url": ORIGIN + "/manifest.mpd", "tbr": 128.0, "language": "en",
                          "fragment_base_url": ORIGIN + "/base/",
                          "fragments": [{"url": ORIGIN + "/base/audio.m4s"}]}]

    def test_actual_reference_paths_metadata_and_audio_rate_normalize_without_mutation(self):
        before = copy.deepcopy((self.native, self.upstream))
        result = compare(self.native, self.upstream, ORIGIN)
        self.assertEqual(result[0]["fragments"][1]["url"], "{{ORIGIN}}/base/segment.m4s?token=one")
        self.assertEqual(result[1]["abr"], 128.0)
        self.assertEqual((self.native, self.upstream), before)

    def test_compared_metadata_queries_fragment_order_and_count_are_significant(self):
        changes = []
        for field, value in (("width", 1280), ("height", 720), ("ext", "webm"),
                             ("tbr", 600.0), ("stream_kind", "standalone"),
                             ("url", ORIGIN + "/other.mpd")):
            changed = copy.deepcopy(self.native)
            changed[0][field] = value
            changes.append(changed)
        for field, value in (("abr", 96.0), ("language", "nb")):
            changed = copy.deepcopy(self.native)
            changed[1][field] = value
            changes.append(changed)
        for fragments in (self.native[0]["fragments"][::-1], self.native[0]["fragments"][:-1],
                          [{"url": ORIGIN + "/base/init.mp4"}, {"url": ORIGIN + "/base/segment.m4s?token=two"}]):
            changed = copy.deepcopy(self.native)
            changed[0]["fragments"] = fragments
            changes.append(changed)
        for changed in changes:
            with self.subTest(changed=changed), self.assertRaisesRegex(ValueError, "DASH plan mismatch"):
                compare(changed, self.upstream, ORIGIN)

    def test_sorted_ids_order_and_period_groups_are_asserted(self):
        for field, value in (("format_id", "representation-0"), ("audio_group", "dash-period-1")):
            changed = copy.deepcopy(self.native)
            changed[0][field] = value
            with self.subTest(field=field), self.assertRaises(ValueError):
                compare(changed, self.upstream, ORIGIN)
        with self.assertRaisesRegex(ValueError, "DASH plan mismatch"):
            compare(self.native[::-1], self.upstream, ORIGIN)
        with self.assertRaisesRegex(ValueError, "DASH plan mismatch"):
            compare(self.native, self.upstream[::-1], ORIGIN)

    def test_range_omission_is_never_implicitly_normalized(self):
        native = copy.deepcopy(self.native)
        native[0]["fragments"][0]["byte_range"] = {"offset": 0, "length": 16}
        with self.assertRaisesRegex(ValueError, "DASH plan mismatch"):
            compare(native, self.upstream, ORIGIN)
        ranges = [[{"offset": 0, "length": 16}, None], [None]]
        compare(native, self.upstream, ORIGIN, native_only_ranges=ranges)
        native[0]["fragments"][0]["byte_range"]["length"] = 17
        with self.assertRaisesRegex(ValueError, "saved SegmentList ranges"):
            compare(native, self.upstream, ORIGIN, native_only_ranges=ranges)
        with self.assertRaisesRegex(ValueError, "format count"):
            compare(native, self.upstream, ORIGIN, native_only_ranges=[])
        self.assertEqual(NATIVE_LIST_RANGES[0][-1], {"offset": 24, "length": 24})

    def test_exact_origin_only_and_reference_kind_must_be_unambiguous(self):
        for url in ("http://127.0.0.1:432100/path", "https://127.0.0.1:43210/path", "https://example.invalid/path"):
            self.assertEqual(normalize_url(url, ORIGIN), url)
        changed = copy.deepcopy(self.upstream)
        changed[0]["acodec"] = "aac"
        with self.assertRaisesRegex(ValueError, "separate, codec-classified"):
            normalize(changed, ORIGIN, upstream=True)

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
