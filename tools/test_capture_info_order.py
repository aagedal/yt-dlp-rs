import json
import unittest

from capture_info_order import CASES, CONTENT_TYPES, PINNED_UPSTREAM, ROOT, content_type, normalize


class InfoOrderCaptureTests(unittest.TestCase):
    def test_normalization_removes_only_the_origin_epoch_and_version(self):
        origin = 'http://127.0.0.1:4321'
        line = ('{"url": "http://127.0.0.1:4321/a", "webpage_url_domain": "127.0.0.1:4321", '
                '"epoch": 1792000000, "_type": "video", "_version": {"version": "x", "release_git_head": "y"}}')
        self.assertEqual(normalize(line, origin),
                         '{"url": "{ORIGIN}/a", "webpage_url_domain": "{HOST}", "_type": "video"}')

    def test_headers_and_saved_cases_cover_every_served_case(self):
        self.assertEqual(content_type('plain.m3u8'), 'text/plain')
        self.assertEqual(content_type('master.m3u8'), 'application/vnd.apple.mpegurl')
        self.assertEqual(content_type('clip.qqq'), 'application/octet-stream')
        fixture = json.loads((ROOT / 'cases.json').read_text(encoding='utf-8'))
        self.assertEqual(fixture['upstream_commit'], PINNED_UPSTREAM)
        self.assertEqual(fixture['content_types'], CONTENT_TYPES)
        self.assertEqual([case['name'] for case in fixture['cases']], CASES)
        for case in fixture['cases']:
            self.assertTrue((ROOT / 'site' / case['name']).is_file())
            self.assertTrue(case['lines'] and case['single'])
            for line in case['lines'] + case['single']:
                self.assertNotIn('"epoch"', line)
                self.assertNotIn('"_version"', line)
                self.assertNotIn('127.0.0.1', line)


if __name__ == '__main__':
    unittest.main()
