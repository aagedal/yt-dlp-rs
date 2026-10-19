import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_dates import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class DateComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = {key: copy.deepcopy(self.fixture[key])
                       for key in ('unified_timestamp', 'strftime')}

    def test_results_and_order_are_compared(self):
        self.assertEqual(compare(self.result, self.fixture), self.result)
        changed = copy.deepcopy(self.result)
        changed['unified_timestamp'][0]['day_first'] += 1
        with self.assertRaisesRegex(ValueError, 'date mismatch'):
            compare(changed, self.fixture)
        changed = copy.deepcopy(self.result)
        changed['strftime'].reverse()
        with self.assertRaises(ValueError):
            compare(changed, self.fixture)

    def test_saved_upstream_semantics_remain_explicit(self):
        cases = {case['input']: case for case in self.fixture['unified_timestamp']}
        self.assertEqual(cases['Wed, 21 Oct 2015 07:28:00 GMT']['day_first'], 1445412480)
        self.assertNotEqual(cases['8/7/2009']['day_first'], cases['8/7/2009']['month_first'])
        self.assertEqual(cases['0001-01-01T00:30:00+01:00']['day_first_error'], 'OverflowError')
        self.assertEqual(self.fixture['upstream_commit'], PINNED_UPSTREAM)

    def test_worker_uses_actual_helpers_without_network(self):
        self.assertIn('unified_timestamp(case', UPSTREAM_WORKER)
        self.assertIn('strftime_or_none(case', UPSTREAM_WORKER)
        self.assertIn('subprocess.Popen = forbidden', UPSTREAM_WORKER)

    def run_main(self, revision):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'yt_dlp/__init__.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([], 0, revision, ''),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), '')]
            with mock.patch('sys.argv', ['compare_dates.py', '--upstream', directory]), \
                    mock.patch('compare_dates.subprocess.run', side_effect=outputs) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status, run, stdout.getvalue(), stderr.getvalue()

    def test_command_checks_pin_and_passes_exact_fixture(self):
        status, run, stdout, _ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status, 0)
        self.assertEqual(json.loads(run.call_args.kwargs['input']), self.fixture)
        self.assertIn('actual unified_timestamp', stdout)

    def test_wrong_revision_stops_before_worker(self):
        status, run, _, stderr = self.run_main('0' * 40)
        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn(PINNED_UPSTREAM, stderr)


if __name__ == '__main__':
    unittest.main()
