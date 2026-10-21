import json
from pathlib import Path
import unittest

from capture_json_ld import PAGES


class JsonLdCaptureTests(unittest.TestCase):
    def test_saved_cases_cover_every_page_in_order(self):
        path = Path(__file__).resolve().parents[1] / 'tests/fixtures/json_ld/cases.json'
        cases = json.loads(path.read_text(encoding='utf-8'))
        self.assertEqual([case[0] for case in cases], PAGES)
        for _, info in cases:
            self.assertIsInstance(info, dict)


if __name__ == '__main__':
    unittest.main()
