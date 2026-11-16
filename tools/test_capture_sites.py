import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest

from capture_sites import PINNED_UPSTREAM, SITES, normalize, patched, prepare
from site_proxy import load_routes, match_route, strip_query


class SiteCaptureTests(unittest.TestCase):
    def test_normalization_removes_only_epoch_and_version(self):
        line = '{"id": "a", "epoch": 1792000000, "_type": "video", "_version": {"version": "x"}}'
        self.assertEqual(normalize(line), '{"id": "a", "_type": "video"}')

    def test_normalization_zeroes_youtube_available_at(self):
        self.assertEqual(normalize('{"available_at": 1792534964}'), '{"available_at": 0}')

    def test_normalization_zeroes_relative_comment_timestamps(self):
        line = '{"_time_text": "2 days ago", "timestamp": 1792454400, "x": {"timestamp": 5}}'
        self.assertEqual(normalize(line), '{"_time_text": "2 days ago", "timestamp": 0, "x": {"timestamp": 5}}')

    def test_routes_can_ignore_random_query_parameters(self):
        self.assertEqual(strip_query('https://h/p.m3u8?a=1&p=42&b=2', ['p']), 'https://h/p.m3u8?a=1&b=2')
        self.assertEqual(strip_query('https://h/p?p=1', ['p']), 'https://h/p')
        routes = {'https://h/p?a=1': [({}, [], (200, {}, b'x'), ['p'])]}
        self.assertEqual(match_route(routes, 'https://h/p?a=1&p=99', {}), (200, {}, b'x'))
        self.assertIsNone(match_route(routes, 'https://h/p?a=2&p=99', {}))

    def test_copied_inputs_are_placed_in_the_working_directory(self):
        with tempfile.TemporaryDirectory() as site, tempfile.TemporaryDirectory() as work:
            Path(site, 'cookies.txt').write_text('# Netscape HTTP Cookie File\n')
            self.assertEqual(prepare(Path(site), {'copy': ['cookies.txt']}, work), {'cookies.txt'})
            self.assertEqual(os.listdir(work), ['cookies.txt'])
            self.assertEqual(prepare(Path(site), {}, work), set())

    def test_site_patches_apply_to_a_copy_and_fail_once_merged(self):
        with tempfile.TemporaryDirectory() as upstream, tempfile.TemporaryDirectory() as site:
            Path(upstream, 'yt_dlp').mkdir()
            Path(upstream, 'yt_dlp', 'x.py').write_text('a = 1\n')
            patch = Path(site, 'upstream.patch')
            patch.write_text('--- a/yt_dlp/x.py\n+++ b/yt_dlp/x.py\n@@ -1 +1 @@\n-a = 1\n+a = 2\n')
            copy = patched(upstream, patch)
            try:
                self.assertEqual(Path(copy, 'yt_dlp', 'x.py').read_text(), 'a = 2\n')
                self.assertEqual(Path(upstream, 'yt_dlp', 'x.py').read_text(), 'a = 1\n')
                with self.assertRaises(SystemExit):
                    patched(copy, patch)
            finally:
                shutil.rmtree(copy)

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
