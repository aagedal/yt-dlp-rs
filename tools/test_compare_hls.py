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
                                         "byte_range": {"offset": 0, "length": 10}, "encryption": None})
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

    def test_aes_explicit_and_sequence_ivs_and_method_none(self):
        explicit = [0] * 15 + [171]
        implicit = list((258).to_bytes(16, "big"))
        native = [
            {"url": ORIGIN + "/first.ts", "encryption": {
                "key_url": ORIGIN + "/keys/first.key?fixture=1", "iv": implicit}},
            {"url": ORIGIN + "/second.ts", "encryption": {
                "key_url": ORIGIN + "/keys/rotated.key", "iv": explicit}},
            {"url": ORIGIN + "/plain.ts"},
            {"url": ORIGIN + "/zero.ts", "encryption": {
                "key_url": ORIGIN + "/keys/rotated.key", "iv": [0] * 16}},
        ]
        upstream = [
            {"url": ORIGIN + "/first.ts", "media_sequence": 258,
             "decrypt_info": {"METHOD": "AES-128", "URI": ORIGIN + "/keys/first.key?fixture=1"}},
            {"url": ORIGIN + "/second.ts", "media_sequence": 259,
             "decrypt_info": {"METHOD": "AES-128", "URI": ORIGIN + "/keys/rotated.key", "IV": explicit}},
            {"url": ORIGIN + "/plain.ts", "decrypt_info": {"METHOD": "NONE"}},
            {"url": ORIGIN + "/zero.ts", "media_sequence": 261,
             "decrypt_info": {"METHOD": "AES-128", "URI": ORIGIN + "/keys/rotated.key", "IV": [0] * 16}},
        ]
        original = copy.deepcopy((native, upstream))
        result = compare(native, upstream, ORIGIN)
        self.assertEqual(result[0]["encryption"], {
            "key_url": "{{ORIGIN}}/keys/first.key?fixture=1", "iv": implicit})
        self.assertEqual(result[1]["encryption"]["iv"], explicit)
        self.assertIsNone(result[2]["encryption"])
        self.assertEqual(result[3]["encryption"]["iv"], [0] * 16)
        self.assertEqual((native, upstream), original)
        for field, value in (("key_url", ORIGIN + "/wrong.key"), ("iv", [0] * 16)):
            changed = copy.deepcopy(native)
            changed[0]["encryption"][field] = value
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "fragment plan mismatch"):
                compare(changed, upstream, ORIGIN)

    def test_encryption_key_origin_is_exact_and_invalid_iv_is_rejected(self):
        url = "https://127.0.0.1:43210/key"
        encrypted = {"url": ORIGIN + "/first.ts", "encryption": {"key_url": url, "iv": [0] * 16}}
        self.assertEqual(normalize([encrypted], ORIGIN)[0]["encryption"]["key_url"], url)
        for iv in ([0] * 15, [256] * 16, [True] * 16, "00" * 16):
            encrypted["encryption"]["iv"] = iv
            with self.subTest(iv=iv), self.assertRaisesRegex(ValueError, "invalid AES-128 IV"):
                normalize([encrypted], ORIGIN)
        with self.assertRaisesRegex(ValueError, "unexpected upstream encryption"):
            normalize([{"url": ORIGIN + "/first.ts", "decrypt_info": {"METHOD": "SAMPLE-AES"}}],
                      ORIGIN, upstream=True)

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
