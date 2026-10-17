import copy
import json
from pathlib import Path
import unittest

from compare_youtube import ORIGIN, compare, normalize


class YoutubeComparisonTest(unittest.TestCase):
    def setUp(self):
        fixture = Path(__file__).resolve().parents[1] / "tests/fixtures/youtube/expected.json"
        self.expected = json.loads(fixture.read_text())
        self.info = json.loads(fixture.read_text().replace("{{ORIGIN}}", ORIGIN))

    def test_exact_fixture_matches_without_mutating_input(self):
        original = copy.deepcopy(self.info)
        self.info["duration"] = 10  # Outside the explicit comparison subset.
        self.assertEqual(compare(self.info, self.expected), self.expected)
        self.assertEqual(self.info["formats"], original["formats"])

    def test_only_exact_origin_is_normalized(self):
        for url in (ORIGIN + "0/media/low.mp4", "https://127.0.0.1:43210/media/low.mp4"):
            with self.subTest(url=url):
                self.info["formats"][0]["url"] = url
                self.assertEqual(normalize(self.info)["formats"][0]["url"], url)
        self.info["formats"][0]["url"] = ORIGIN + "/media/low.mp4?a=1&b=2#part"
        self.assertEqual(normalize(self.info)["formats"][0]["url"],
                         "{{ORIGIN}}/media/low.mp4?a=1&b=2#part")

    def test_format_order_count_and_metadata_are_significant(self):
        for formats in (self.info["formats"][::-1], self.info["formats"][:1]):
            with self.subTest(formats=formats), self.assertRaisesRegex(ValueError, "metadata mismatch"):
                compare(dict(self.info, formats=formats), self.expected)
        self.info["formats"][0]["width"] = 641
        with self.assertRaisesRegex(ValueError, "metadata mismatch"):
            compare(self.info, self.expected)


if __name__ == "__main__":
    unittest.main()
