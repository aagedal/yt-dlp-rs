import copy
import json
import unittest

from compare_shlex import FIXTURE, compare, split


class ShlexComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))

    def test_saved_cases_match_and_changes_are_detected(self):
        self.assertEqual(compare(self.fixture), self.fixture['cases'])
        changed = copy.deepcopy(self.fixture)
        changed['cases'][1]['tokens'].reverse()
        with self.assertRaisesRegex(ValueError, 'shlex mismatch'):
            compare(changed)

    def test_comments_quotes_and_errors_remain_explicit(self):
        cases = {case['input']: case for case in self.fixture['cases']}
        self.assertEqual(cases['a#b c']['tokens'], ['a'])
        self.assertEqual(cases['"a#b" c']['tokens'], ['a#b', 'c'])
        self.assertEqual(cases['"unterminated']['error'], 'No closing quotation')
        self.assertEqual(split('x "" y'), {'input': 'x "" y', 'tokens': ['x', '', 'y']})


if __name__ == '__main__':
    unittest.main()
