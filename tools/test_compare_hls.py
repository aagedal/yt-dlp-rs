import contextlib
import copy
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_hls import PINNED_UPSTREAM, compare, main, normalize, verify_upstream

ORIGIN = "http://127.0.0.1:43210"


class HlsComparisonTests(unittest.TestCase):
    def setUp(self):
        self.native = [
            {"url": ORIGIN + "/init.mp4", "byte_range": {"offset": 0, "length": 10}},
            {"url": ORIGIN + "/combined.mp4?x=1#part", "byte_range": {"offset": 10, "length": 20}},
            {"url": ORIGIN + "/whole.ts"},
        ]
        self.upstream = [
            {"url": ORIGIN + "/init.mp4", "byte_range": {"start": 0, "end": 10}, "frag_index": 1},
            {"url": ORIGIN + "/combined.mp4?x=1#part", "byte_range": {"start": 10, "end": 30}},
            {"url": ORIGIN + "/whole.ts", "byte_range": {}},
        ]

    def test_actual_exclusive_ranges_map_without_mutating_input(self):
        before = copy.deepcopy((self.native, self.upstream))
        normalized = compare(self.native, self.upstream, ORIGIN)
        self.assertEqual(normalized[0], {"url": "{{ORIGIN}}/init.mp4",
                                         "byte_range": {"offset": 0, "length": 10}})
        self.assertEqual(normalized[1]["url"], "{{ORIGIN}}/combined.mp4?x=1#part")
        self.assertIsNone(normalized[2]["byte_range"])
        self.assertEqual((self.native, self.upstream), before)

    def test_order_count_range_and_url_are_significant(self):
        variants = [self.native[::-1], self.native[:-1]]
        for field, value in (("url", ORIGIN + "/changed.mp4"),
                             ("byte_range", {"offset": 10, "length": 21}),
                             ("byte_range", {"offset": 11, "length": 20})):
            changed = copy.deepcopy(self.native)
            changed[1][field] = value
            variants.append(changed)
        for changed in variants:
            with self.subTest(changed=changed), self.assertRaisesRegex(ValueError, "fragment plan mismatch"):
                compare(changed, self.upstream, ORIGIN)

    def test_only_exact_origin_is_normalized(self):
        for url in ("http://127.0.0.1:432100/init.mp4", "https://127.0.0.1:43210/init.mp4",
                    "http://example.invalid/init.mp4"):
            self.assertEqual(normalize([{"url": url}], ORIGIN)[0]["url"], url)

    def test_missing_fragment_url_is_error(self):
        with self.assertRaises(KeyError):
            compare([{}], [], ORIGIN)

    def test_requires_pinned_checkout(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            module = root / "yt_dlp/downloader/hls.py"
            module.parent.mkdir(parents=True)
            module.touch()
            for revision, succeeds in ((PINNED_UPSTREAM, True), ("0" * 40, False)):
                with self.subTest(revision=revision), mock.patch(
                        "compare_hls.subprocess.run", return_value=subprocess.CompletedProcess(
                            [], 0, revision + "\n", "")) as run:
                    if succeeds:
                        verify_upstream(root)
                    else:
                        with self.assertRaisesRegex(ValueError, PINNED_UPSTREAM):
                            verify_upstream(root)
                    self.assertEqual(run.call_args.args[0],
                                     ["git", "-C", str(root), "rev-parse", "HEAD"])
                    self.assertEqual(run.call_args.kwargs["encoding"], "utf-8")

    def test_bad_pin_fails_before_binary_or_server_access(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            module = root / "yt_dlp/downloader/hls.py"
            module.parent.mkdir(parents=True)
            module.touch()
            with mock.patch("sys.argv", ["compare_hls.py", "--upstream", directory,
                                         "--binary", "/nonexistent-explicit-binary"]), \
                    mock.patch("compare_hls.subprocess.run", return_value=subprocess.CompletedProcess(
                        [], 0, "0" * 40, "")) as run, \
                    mock.patch("compare_hls.manifest_server") as server, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                self.assertEqual(main(), 1)
            self.assertEqual(run.call_count, 1)
            server.assert_not_called()
            self.assertIn(PINNED_UPSTREAM, stderr.getvalue())


if __name__ == "__main__":
    unittest.main()
