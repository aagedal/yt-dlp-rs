import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_info_json import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, expected, main


class InfoJsonComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = expected(self.fixture)

    def test_exact_values_order_and_key_order_are_compared(self):
        before = copy.deepcopy((self.result, self.fixture))
        self.assertEqual(compare(self.result, self.fixture), self.result)
        self.assertEqual((self.result, self.fixture), before)
        for key in ('filenames', 'playlist_names', 'json_text'):
            changed = copy.deepcopy(self.result)
            changed[key][0]['expected'] += ' '
            with self.assertRaisesRegex(ValueError, 'info JSON mismatch'):
                compare(changed, self.fixture)
        changed = copy.deepcopy(self.result)
        changed['sanitize'][0]['expected'] = dict(reversed(changed['sanitize'][0]['expected'].items()))
        with self.assertRaises(ValueError):
            compare(changed, self.fixture)
        changed = copy.deepcopy(self.result)
        changed['source_only'][0]['upstream_extra_key'] = None
        with self.assertRaises(ValueError):
            compare(changed, self.fixture)

    def test_fixture_records_upstream_edges(self):
        names = {case['name']: case['expected'] for case in self.fixture['filenames']}
        self.assertEqual(names['other-real-ext'], 'Title.webm.info.json')
        self.assertEqual(names['leading-dot-only'], '.hidden.info.json')
        playlists = {case['name']: case['expected'] for case in self.fixture['playlist_names']}
        self.assertEqual(playlists['counts-and-missing'], 'Plist-00-2-14-NA-NA.info.json')
        sanitized = {case['name']: case['expected'] for case in self.fixture['sanitize']}
        self.assertEqual(list(sanitized['defaults-appended']), ['id', 'epoch', 'ext', '_type'])
        self.assertEqual(sanitized['nested-cleaning-keeps-list-nulls']['list'], [None, {}])
        self.assertTrue(all(case['native_policy'] for case in self.fixture['source_only']))
        self.assertEqual(self.fixture['upstream_commit'], PINNED_UPSTREAM)

    def test_worker_uses_actual_helpers_and_forbids_network_processes(self):
        for needle in ('replace_extension(case', "'pl_infojson'", '_playlist_infodict',
                       'sanitize_info(copy.deepcopy', 'write_json_file(case',
                       'subprocess.Popen = forbidden',
                       'socket.socket = socket.create_connection = forbidden'):
            self.assertIn(needle, UPSTREAM_WORKER)

    def run_main(self, revision, fixture_path=None):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'yt_dlp/YoutubeDL.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([], 0, revision, ''),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), '')]
            with mock.patch('sys.argv', ['compare_info_json.py', '--upstream', directory]), \
                    mock.patch('compare_info_json.subprocess.run', side_effect=outputs) as run, \
                    mock.patch('compare_info_json.FIXTURE', fixture_path or FIXTURE), \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status, run, stdout.getvalue(), stderr.getvalue()

    def test_command_checks_pin_and_passes_fixture(self):
        status, run, stdout, _ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status, 0)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(json.loads(run.call_args.kwargs['input']), self.fixture)
        self.assertIn('-I', run.call_args.args[0])
        self.assertIn('sanitize_info', stdout)

    def test_wrong_revisions_stop_before_worker(self):
        status, run, _, stderr = self.run_main('0' * 40)
        self.assertEqual((status, run.call_count), (1, 1))
        self.assertIn(PINNED_UPSTREAM, stderr)
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'wrong.json'
            self.fixture['upstream_commit'] = '0' * 40
            path.write_text(json.dumps(self.fixture), encoding='utf-8')
            status, run, _, stderr = self.run_main(PINNED_UPSTREAM, path)
        self.assertEqual((status, run.call_count), (1, 1))
        self.assertIn('fixture', stderr)


if __name__ == '__main__':
    unittest.main()
