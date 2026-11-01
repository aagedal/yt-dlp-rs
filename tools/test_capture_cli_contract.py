import json
import unittest

from capture_cli_contract import FIXTURE, PINNED_UPSTREAM, body, normalize


class CliContractCaptureTests(unittest.TestCase):
    def test_normalization_keeps_json_keys_and_collapses_repeats(self):
        text = ('{"status": "downloading", "_version": {}, "eta": 1}\n'
                '{"status": "downloading", "eta": null}\n'
                'null\n/tmp/w/a.mp4\n/tmp/w/a.mp4\nyt-dlp-rs: error\n')
        self.assertEqual(normalize(text, [('/tmp/w', 'DIR')]), [
            'JSON ["status", "eta"]', 'null', 'DIR/a.mp4', 'yt-dlp: error', ''])

    def test_route_bodies(self):
        self.assertEqual(body({'text': 'é'}), 'é'.encode())
        self.assertEqual(body({'size': 3, 'fill': 71}), b'GGG')
        self.assertEqual(body({'size': 2}), b'\0\0')

    def test_every_case_has_saved_upstream_output(self):
        cases = json.loads((FIXTURE / 'cases.json').read_text(encoding='utf-8'))['cases']
        expected = json.loads((FIXTURE / 'expected.json').read_text(encoding='utf-8'))
        self.assertEqual(expected['upstream_commit'], PINNED_UPSTREAM)
        self.assertEqual([(case['name'], case['args']) for case in expected['cases']],
                         [(case['name'], case['args']) for case in cases])


if __name__ == '__main__':
    unittest.main()
