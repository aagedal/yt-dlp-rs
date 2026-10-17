import unittest

from compare_html5 import compare, normalize


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


if __name__ == "__main__":
    unittest.main()
