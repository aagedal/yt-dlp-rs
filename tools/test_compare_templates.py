import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_templates import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class TemplateComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = {key: [{'name': c['name'], 'rendered': c['expected']} for c in self.fixture[key]]
                       for key in ('cases', 'source_only_cases')}

    def test_exact_strings_order_and_source_boundaries_are_compared(self):
        before = copy.deepcopy((self.result, self.fixture))
        self.assertEqual(compare(self.result, self.fixture), self.result)
        self.assertEqual((self.result, self.fixture), before)
        for key in ('cases', 'source_only_cases'):
            changed = copy.deepcopy(self.result)
            changed[key][0]['rendered'] += ' '
            with self.assertRaisesRegex(ValueError, 'template rendering mismatch'):
                compare(changed, self.fixture)
            changed = copy.deepcopy(self.result)
            changed[key].reverse()
            with self.assertRaises(ValueError):
                compare(changed, self.fixture)

    def test_missing_defaults_casting_and_automatic_padding_remain_exact(self):
        cases = {c['name']: c['expected'] for c in self.fixture['cases']}
        for name, expected in {
            'missing-default': 'xNA-end', 'missing-custom': 'xmissing-end',
            'missing-empty-global': 'x-end', 'missing-empty-explicit': 'x-end',
            'missing-zero-default': 'x0-end', 'nonnumeric-default': 'xfallback-end',
            'field-playlist_index': 'value-001-end',
            'field-playlist_autonumber': 'value-02-end',
            'filesize-9007199254740993': '9007199254740992',
            'filesize-18446744073709551615': '18446744073709551616',
            'no-alternate-after-nonnumeric': 'fallback',
            'empty-source-alternate': 'Café 𝐀 demo',
        }.items():
            self.assertEqual(cases[name], expected, name)
        self.assertEqual(self.fixture['upstream_commit'], PINNED_UPSTREAM)
        self.assertTrue(all(c.get('native_policy') for c in self.fixture['source_only_cases']))

    def test_worker_uses_actual_sanitized_evaluator_and_forbids_network_processes(self):
        self.assertIn('ydl.evaluate_outtmpl', UPSTREAM_WORKER)
        self.assertIn('sanitize=True', UPSTREAM_WORKER)
        self.assertIn('subprocess.Popen = forbidden', UPSTREAM_WORKER)
        self.assertIn('socket.socket = socket.create_connection = forbidden', UPSTREAM_WORKER)
        self.assertIn('YoutubeDL.urlopen = YoutubeDL.download = forbidden', UPSTREAM_WORKER)

    def run_main(self, revision, fixture_path=None):
        with tempfile.TemporaryDirectory() as directory:
            upstream = Path(directory)
            module = upstream / 'yt_dlp/YoutubeDL.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([], 0, revision, ''),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), '')]
            with mock.patch('sys.argv', ['compare_templates.py', '--upstream', directory]), \
                    mock.patch('compare_templates.subprocess.run', side_effect=outputs) as run, \
                    mock.patch('compare_templates.FIXTURE', fixture_path or FIXTURE), \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status, run, stdout.getvalue(), stderr.getvalue()

    def test_command_checks_pin_and_passes_exact_shared_fixture(self):
        status, run, stdout, _ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status, 0)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(run.call_args_list[0].args[0][-2:], ['rev-parse', 'HEAD'])
        self.assertEqual(json.loads(run.call_args.kwargs['input']), self.fixture)
        self.assertIn('actual evaluate_outtmpl', stdout)

    def test_wrong_revision_stops_before_worker(self):
        status, run, _, stderr = self.run_main('0' * 40)
        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn(PINNED_UPSTREAM, stderr)

    def test_wrong_fixture_revision_stops_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'wrong.json'
            self.fixture['upstream_commit'] = '0' * 40
            path.write_text(json.dumps(self.fixture), encoding='utf-8')
            status, run, _, stderr = self.run_main(PINNED_UPSTREAM, path)
        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn('fixture', stderr)


if __name__ == '__main__':
    unittest.main()
