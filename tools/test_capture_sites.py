import json
import unittest

from capture_sites import PINNED_UPSTREAM, SITES, normalize
from site_proxy import load_routes, match_route, strip_query


class SiteCaptureTests(unittest.TestCase):
    def test_normalization_removes_only_epoch_and_version(self):
        line = '{"id": "a", "epoch": 1792000000, "_type": "video", "_version": {"version": "x"}}'
        self.assertEqual(normalize(line), '{"id": "a", "_type": "video"}')

    def test_normalization_zeroes_youtube_available_at(self):
        self.assertEqual(normalize('{"available_at": 1792534964}'), '{"available_at": 0}')

    def test_routes_can_ignore_random_query_parameters(self):
        self.assertEqual(strip_query('https://h/p.m3u8?a=1&p=42&b=2', ['p']), 'https://h/p.m3u8?a=1&b=2')
        self.assertEqual(strip_query('https://h/p?p=1', ['p']), 'https://h/p')
        routes = {'https://h/p?a=1': [({}, [], (200, {}, b'x'), ['p'])]}
        self.assertEqual(match_route(routes, 'https://h/p?a=1&p=99', {}), (200, {}, b'x'))
        self.assertIsNone(match_route(routes, 'https://h/p?a=2&p=99', {}))

    def test_every_site_has_routes_and_saved_cases(self):
        for site in SITES.iterdir():
            if not (site / 'cases.json').is_file():
                continue
            self.assertTrue(load_routes(site))
            cases = json.loads((site / 'cases.json').read_text(encoding='utf-8'))
            expected = json.loads((site / 'expected.json').read_text(encoding='utf-8'))
            self.assertEqual(expected['upstream_commit'], PINNED_UPSTREAM)
            self.assertEqual([(case['url'], case['args']) for case in expected['cases']],
                             [(case['url'], case['args']) for case in cases])


if __name__ == '__main__':
    unittest.main()
