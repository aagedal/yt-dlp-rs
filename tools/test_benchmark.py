import urllib.request
import unittest

from benchmark import EXTRACT_CASES, Media, MediaServer, extract_case, peak_rss_bytes


class BenchmarkToolTests(unittest.TestCase):
    def test_peak_rss_is_read_from_both_time_formats(self):
        self.assertEqual(peak_rss_bytes('  104611840  maximum resident set size\n'), 104611840)
        self.assertEqual(peak_rss_bytes('\tMaximum resident set size (kbytes): 2048\n'), 2 * 2**20)
        self.assertIsNone(peak_rss_bytes('nothing here'))

    def test_generated_media_lists_every_segment(self):
        media = Media(3, 1000)
        self.assertEqual(len(media.segment), 1000)
        self.assertEqual(media.m3u8.count(b'.ts\n'), 3)
        self.assertIn(b'#EXT-X-ENDLIST', media.m3u8)
        self.assertIn(b'mediaPresentationDuration="PT6S"', media.mpd)

    def test_server_serves_media_after_its_latency(self):
        server = MediaServer(Media(2, 500))
        with urllib.request.urlopen(server.url(0, 'seg1.ts')) as response:
            self.assertEqual(len(response.read()), 500)
        with self.assertRaises(urllib.error.HTTPError) as raised:
            urllib.request.urlopen(server.url(0, 'missing'))
        raised.exception.close()
        server.httpd.shutdown()

    def test_extract_cases_are_successful_json_fixture_cases(self):
        for site, index in EXTRACT_CASES:
            case = extract_case(site, index)
            self.assertTrue({'-J', '-j'} & set(case['args']), site)


if __name__ == '__main__':
    unittest.main()
