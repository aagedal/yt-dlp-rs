import contextlib
import io
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock
from urllib.error import HTTPError
from urllib.request import Request, urlopen

from compare_hls import (PINNED_UPSTREAM, compare, main, manifest_server, normalize_requests,
                         parse_range, verify_upstream)

MANIFEST = "/hls/aes-ts/index.m3u8"


class HlsComparisonTests(unittest.TestCase):
    def setUp(self):
        self.requests = [
            (MANIFEST, None),
            ("/hls/aes-ts/segment-0.ts?part=0", None),
            ("/hls/aes-keys/0.key?fixture=1", None),
            ("/hls/range/combined.mp4", "bytes=10-29"),
            ("/hls/aes-keys/1.key", None),
        ]

    def test_playlist_excluded_and_keys_split_from_fragments_in_order(self):
        result = normalize_requests(self.requests + [(MANIFEST, None)], MANIFEST)
        self.assertEqual(result, {
            "fragments": [["/hls/aes-ts/segment-0.ts?part=0", None],
                          ["/hls/range/combined.mp4", "bytes=10-29"]],
            "keys": [["/hls/aes-keys/0.key?fixture=1", None], ["/hls/aes-keys/1.key", None]]})
        with self.assertRaisesRegex(ValueError, "no fragment requests"):
            normalize_requests([(MANIFEST, None)], MANIFEST)

    def test_key_fetch_timing_is_not_compared_but_key_order_is(self):
        native = [self.requests[2], self.requests[1], self.requests[3], self.requests[4]]
        self.assertEqual(compare(native, self.requests, b"x", b"x", MANIFEST),
                         normalize_requests(self.requests, MANIFEST))
        swapped = [self.requests[4], self.requests[1], self.requests[3], self.requests[2]]
        with self.assertRaisesRegex(ValueError, "fragment request mismatch"):
            compare(swapped, self.requests, b"x", b"x", MANIFEST)

    def test_order_count_query_and_range_are_significant(self):
        variants = [
            [self.requests[0], self.requests[3], self.requests[2], self.requests[1], self.requests[4]],
            self.requests[:-2] + self.requests[-1:],
            [self.requests[0], ("/hls/aes-ts/segment-0.ts", None)] + self.requests[2:],
            self.requests[:3] + [("/hls/range/combined.mp4", "bytes=10-30")] + self.requests[4:],
            self.requests[:3] + [("/hls/range/combined.mp4", None)] + self.requests[4:],
            self.requests + [("/hls/aes-keys/1.key", None)],
        ]
        for changed in variants:
            with self.subTest(changed=changed), self.assertRaisesRegex(ValueError, "fragment request mismatch"):
                compare(changed, self.requests, b"x", b"x", MANIFEST)

    def test_assembled_bytes_must_be_identical_and_nonempty(self):
        for native, upstream in ((b"abc", b"abd"), (b"abc", b"abcd"), (b"", b"")):
            with self.subTest(native=native), self.assertRaisesRegex(ValueError, "assembled output mismatch"):
                compare(self.requests, self.requests, native, upstream, MANIFEST)

    def test_range_parsing_is_inclusive_and_bounded(self):
        self.assertEqual(parse_range("bytes=10-29", 100), (10, 30))
        self.assertEqual(parse_range("bytes=10-", 100), (10, 100))
        for header in ("bytes=10-100", "bytes=30-10", "items=0-1"):
            with self.subTest(header=header), self.assertRaises(ValueError):
                parse_range(header, 100)

    def test_server_serves_files_ignoring_query_honours_ranges_and_logs(self):
        with tempfile.TemporaryDirectory() as directory:
            blob = Path(directory) / "blob.bin"
            blob.write_bytes(bytes(range(32)))
            with manifest_server({"/a.m3u8": "#EXTM3U\n"}, {"/hls/blob.bin": blob}) as (origin, requests):
                with urlopen(Request(origin + "/hls/blob.bin?q=1", headers={"Range": "bytes=4-7"})) as response:
                    self.assertEqual(response.status, 206)
                    self.assertEqual(response.read(), bytes(range(4, 8)))
                with urlopen(origin + "/a.m3u8") as response:
                    self.assertEqual(response.read(), b"#EXTM3U\n")
                with self.assertRaises(HTTPError) as missing:
                    urlopen(origin + "/hls/missing.bin")
                missing.exception.close()
                self.assertEqual(missing.exception.code, 404)
            self.assertEqual(requests, [("/hls/blob.bin?q=1", "bytes=4-7"), ("/a.m3u8", None),
                                        ("/hls/missing.bin", None)])

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
