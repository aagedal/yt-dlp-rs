import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_batch import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class BatchComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = [{'name': c['name'], 'urls': c['expected']} for c in self.fixture['cases']]

    def test_exact_urls_and_order_are_compared(self):
        self.assertEqual(compare(self.result, self.fixture), self.result)
        changed = copy.deepcopy(self.result)
        changed[0]['urls'][0] += ' '
        with self.assertRaisesRegex(ValueError, 'batch URL mismatch'):
            compare(changed, self.fixture)
        changed = copy.deepcopy(self.result)
        changed.reverse()
        with self.assertRaises(ValueError):
            compare(changed, self.fixture)

    def test_saved_upstream_semantics_remain_explicit(self):
        cases = {c['name']: c['expected'] for c in self.fixture['cases']}
        self.assertEqual(cases['upstream-vector'], ['foo', 'bar', 'baz', 'bam'])
        self.assertEqual(cases['fragment-vs-comment'], ['http://a/#frag', 'http://b/', 'http://c/#'])
        self.assertEqual(cases['positional-stripped-after-batch'],
                         ['http://batch/', 'http://pos/', 'http://pos2/'])
        self.assertEqual(self.fixture['upstream_commit'], PINNED_UPSTREAM)

    def test_worker_uses_actual_get_urls_without_network(self):
        self.assertIn('get_urls(list(case', UPSTREAM_WORKER)
        self.assertIn('subprocess.Popen = forbidden', UPSTREAM_WORKER)
        self.assertIn('socket.socket = socket.create_connection = forbidden', UPSTREAM_WORKER)

    def run_main(self, revision):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'yt_dlp/__init__.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([], 0, revision, ''),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), '')]
            with mock.patch('sys.argv', ['compare_batch.py', '--upstream', directory]), \
                    mock.patch('compare_batch.subprocess.run', side_effect=outputs) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status, run, stdout.getvalue(), stderr.getvalue()

    def test_command_checks_pin_and_passes_exact_fixture(self):
        status, run, stdout, _ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status, 0)
        self.assertEqual(json.loads(run.call_args.kwargs['input']), self.fixture)
        self.assertIn('actual get_urls', stdout)

    def test_wrong_revision_stops_before_worker(self):
        status, run, _, stderr = self.run_main('0' * 40)
        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn(PINNED_UPSTREAM, stderr)


if __name__ == '__main__':
    unittest.main()
