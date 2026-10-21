import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_format_sort import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class FormatSortComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = [{key: copy.deepcopy(case[key]) for key in ('name', 'filled', 'sorted')}
                       for case in self.fixture['cases']]

    def test_filled_formats_and_order_are_compared(self):
        self.assertEqual(compare(self.result, self.fixture), self.result)
        for mutate in (
            lambda result: result[0]['sorted'].reverse(),
            lambda result: result[1]['filled'][0].update(protocol='https'),
            lambda result: result.reverse(),
        ):
            changed = copy.deepcopy(self.result)
            mutate(changed)
            with self.assertRaisesRegex(ValueError, 'format sort mismatch'):
                compare(changed, self.fixture)

    def test_saved_upstream_semantics_remain_explicit(self):
        cases = {case['name']: case for case in self.fixture['cases']}
        self.assertEqual(cases['html5 default']['sorted'], ['h2', 'h0', 'h1', 'h3'])
        # `res` is min(width, height): tall 1080x1920 ties with high 1920x1080.
        self.assertEqual(cases['heights -S +res']['sorted'][:2], ['high', 'tall'])
        self.assertEqual(cases['heights -S +res']['sorted'][-1], 'unknown')
        self.assertTrue(any(case['force'] for case in self.fixture['cases']))
        self.assertTrue(any(case['free'] for case in self.fixture['cases']))
        self.assertEqual(self.fixture['upstream_commit'], PINNED_UPSTREAM)

    def test_worker_uses_actual_sorter_without_network(self):
        self.assertIn('FormatSorter._fill_sorting_fields(fmt)', UPSTREAM_WORKER)
        self.assertIn('calculate_preference', UPSTREAM_WORKER)
        self.assertIn('subprocess.Popen = forbidden', UPSTREAM_WORKER)

    def run_main(self, revision):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'yt_dlp/__init__.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([], 0, revision, ''),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), '')]
            with mock.patch('sys.argv', ['compare_format_sort.py', '--upstream', directory]), \
                    mock.patch('compare_format_sort.subprocess.run', side_effect=outputs) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status, run, stdout.getvalue(), stderr.getvalue()

    def test_command_checks_pin_and_passes_exact_fixture(self):
        status, run, stdout, _ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status, 0)
        self.assertEqual(json.loads(run.call_args.kwargs['input']), self.fixture)
        self.assertIn('actual FormatSorter', stdout)

    def test_wrong_revision_stops_before_worker(self):
        status, run, _, stderr = self.run_main('0' * 40)
        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn(PINNED_UPSTREAM, stderr)


if __name__ == '__main__':
    unittest.main()
