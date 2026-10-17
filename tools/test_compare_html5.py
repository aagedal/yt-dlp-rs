import unittest
from unittest.mock import patch

from compare_html5 import compare, compare_entries, normalize, run_json


class Html5ComparisonTest(unittest.TestCase):
    def setUp(self):
        self.origin = "http://127.0.0.1:12345"
        self.info = {
            "id": "video", "title": "A & B", "ext": "mp4",
            "url": self.origin + "/media/clip.mp4?one=1&two=2#part",
            "webpage_url": self.origin + "/video.html",
            "extractor": "generic", "extractor_key": "Generic",
            "timestamp": 123,
        }

    def test_normalization_removes_only_local_origin_and_excluded_fields(self):
        normalized = normalize(self.info, self.origin)
        self.assertEqual(normalized["url"], "/media/clip.mp4?one=1&two=2#part")
        self.assertEqual(normalized["webpage_url"], "/video.html")
        self.assertEqual(normalized["title"], "A & B")
        self.assertNotIn("timestamp", normalized)
        self.assertTrue(self.info["url"].startswith(self.origin))

    def test_other_origins_are_preserved(self):
        for origin in (self.origin + "0", "https://127.0.0.1:12345", "http://elsewhere.invalid"):
            info = dict(self.info, url=origin + "/media/clip.mp4")
            self.assertEqual(normalize(info, self.origin)["url"], info["url"])

    def test_mismatched_fields_fail_without_erasing_significant_values(self):
        for field, value in (("title", "A  & B"), ("id", "clip"),
                             ("url", self.origin + "/media/clip.mp4?two=2&one=1")):
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, field):
                compare(self.info, dict(self.info, **{field: value}), self.origin)

    def test_missing_required_field_is_an_error(self):
        incomplete = dict(self.info)
        del incomplete["ext"]
        with self.assertRaises(KeyError):
            compare(self.info, incomplete, self.origin)

    def test_ignored_fields_do_not_cause_mismatches(self):
        actual = dict(self.info, timestamp=456)
        self.assertEqual(compare(self.info, actual, self.origin), normalize(self.info, self.origin))

    def test_single_video_matches_one_ndjson_entry(self):
        self.assertEqual(compare_entries(self.info, [self.info], self.origin),
                         [normalize(self.info, self.origin)])

    def test_playlist_compares_ordered_entries(self):
        second = dict(self.info, id="video-2", title="Second", ext="mp3",
                      url=self.origin + "/media/tone.mp3")
        playlist = {"_type": "playlist", "entries": [self.info, second]}
        self.assertEqual(compare_entries(playlist, [self.info, second], self.origin),
                         [normalize(self.info, self.origin), normalize(second, self.origin)])
        with self.assertRaisesRegex(ValueError, "entry 1:.*id"):
            compare_entries(playlist, [second, self.info], self.origin)

    def test_playlist_entry_count_must_match_exactly(self):
        playlist = {"_type": "playlist", "entries": [self.info, self.info]}
        for entries in ([], [self.info], [self.info] * 3):
            with self.subTest(count=len(entries)), self.assertRaisesRegex(ValueError, "entry count mismatch"):
                compare_entries(playlist, entries, self.origin)

    def test_missing_or_malformed_playlist_entries_fail(self):
        for entries in (None, {}, "entries"):
            with self.subTest(entries=entries), self.assertRaisesRegex(ValueError, "entries must be a list"):
                compare_entries({"_type": "playlist", "entries": entries}, [self.info], self.origin)
        with self.assertRaisesRegex(ValueError, "entries must be a list"):
            compare_entries({"_type": "playlist"}, [self.info], self.origin)
        for upstream_entry, rust_entry in ((None, self.info), (self.info, None), ([], self.info)):
            with self.subTest(upstream=upstream_entry, rust=rust_entry), self.assertRaisesRegex(ValueError, "entry 1 must be an object"):
                compare_entries({"_type": "playlist", "entries": [upstream_entry]},
                                [rust_entry], self.origin)

    def test_playlist_entries_require_allowlisted_fields(self):
        incomplete = dict(self.info)
        del incomplete["webpage_url"]
        with self.assertRaises(KeyError):
            compare_entries({"_type": "playlist", "entries": [self.info]},
                            [incomplete], self.origin)

    def test_ndjson_reader_preserves_order(self):
        with patch("compare_html5.subprocess.run") as run:
            run.return_value.stdout = '{"id":"first"}\n{"id":"second"}\n'
            self.assertEqual(run_json(["binary"], ndjson=True),
                             [{"id": "first"}, {"id": "second"}])


if __name__ == "__main__":
    unittest.main()
