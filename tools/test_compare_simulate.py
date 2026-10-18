import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_simulate import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class SimulateComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = copy.deepcopy(self.fixture['cases'])

    def test_exact_urls_and_order_are_compared(self):
        self.assertEqual(compare(self.result, self.fixture), self.result)
        changed = copy.deepcopy(self.result)
        changed[0]['simulate'] = not changed[0]['simulate']
        with self.assertRaisesRegex(ValueError, 'simulate mismatch'):
            compare(changed, self.fixture)
        changed = copy.deepcopy(self.result)
        changed.reverse()
        with self.assertRaises(ValueError):
            compare(changed, self.fixture)

    def test_saved_upstream_semantics_remain_explicit(self):
        cases = {' '.join(c['args']): c for c in self.fixture['cases']}
        self.assertTrue(cases['-F']['list_only'])
        self.assertFalse(cases['-F -j']['list_only'])
        self.assertFalse(cases['-j --no-simulate']['simulate'])
        self.assertTrue(cases['-j --no-simulate']['quiet'])
        self.assertTrue(cases['--no-simulate -s']['simulate'])
        self.assertEqual(self.fixture['upstream_commit'], PINNED_UPSTREAM)

    def test_worker_uses_actual_option_resolution_without_network(self):
        self.assertIn('parse_options(', UPSTREAM_WORKER)
        self.assertIn('YoutubeDL(parsed.ydl_opts', UPSTREAM_WORKER)
        self.assertIn('subprocess.Popen = forbidden', UPSTREAM_WORKER)

    def run_main(self, revision):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'yt_dlp/__init__.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([], 0, revision, ''),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), '')]
            with mock.patch('sys.argv', ['compare_simulate.py', '--upstream', directory]), \
                    mock.patch('compare_simulate.subprocess.run', side_effect=outputs) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status, run, stdout.getvalue(), stderr.getvalue()

    def test_command_checks_pin_and_passes_exact_fixture(self):
        status, run, stdout, _ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status, 0)
        self.assertEqual(json.loads(run.call_args.kwargs['input']), self.fixture)
        self.assertIn('actual parse_options/YoutubeDL', stdout)

    def test_wrong_revision_stops_before_worker(self):
        status, run, _, stderr = self.run_main('0' * 40)
        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn(PINNED_UPSTREAM, stderr)


if __name__ == '__main__':
    unittest.main()
