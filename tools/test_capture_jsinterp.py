import json
from pathlib import Path
import unittest

from capture_jsinterp import FIXTURES, PINNED_UPSTREAM


class JsInterpCaptureTests(unittest.TestCase):
    def test_saved_results_cover_every_case_in_order(self):
        cases = json.loads((FIXTURES / 'cases.json').read_text(encoding='utf-8'))
        expected = json.loads((FIXTURES / 'expected.json').read_text(encoding='utf-8'))
        self.assertEqual(expected['upstream_commit'], PINNED_UPSTREAM)
        self.assertEqual(len(expected['cases']), len(cases))
        for case, saved in zip(cases, expected['cases']):
            self.assertEqual({key: saved[key] for key in case}, case)
            self.assertTrue(('result' in saved) != ('error' in saved), saved)


if __name__ == '__main__':
    unittest.main()
