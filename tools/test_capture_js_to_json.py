import json
from pathlib import Path
import unittest

from capture_js_to_json import INPUTS


class JsToJsonCaptureTests(unittest.TestCase):
    def test_saved_cases_cover_every_input_in_order(self):
        path = Path(__file__).resolve().parents[1] / 'tests/fixtures/js_to_json/cases.json'
        cases = json.loads(path.read_text(encoding='utf-8'))
        self.assertEqual([case[0] for case in cases], INPUTS)
        for _, output in cases:
            self.assertIsInstance(output, str)


if __name__ == '__main__':
    unittest.main()
