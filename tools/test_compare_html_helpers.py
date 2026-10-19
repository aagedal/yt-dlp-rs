import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_html_helpers import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class HtmlHelperComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = {key: copy.deepcopy(self.fixture[key]) for key in ('unescape', 'og', 'rta')}

    def test_results_and_order_are_compared(self):
        self.assertEqual(compare(self.result, self.fixture), self.result)
        changed = copy.deepcopy(self.result)
        changed['og'][0]['value'] = 'changed'
        with self.assertRaisesRegex(ValueError, 'HTML helper mismatch'):
            compare(changed, self.fixture)
        changed = copy.deepcopy(self.result)
        changed['rta'].reverse()
        with self.assertRaises(ValueError):
            compare(changed, self.fixture)

    def test_saved_upstream_semantics_remain_explicit(self):
        unescape = {case['input']: case['output'] for case in self.fixture['unescape']}
        self.assertEqual(unescape['&lang;&rang;'], '\u2329\u232a')
        og = {case['html']: case['value'] for case in self.fixture['og']}
        self.assertEqual(og['<meta property=og:description content=a/b/>'], 'a/b')
        rta = {case['html']: case['age_limit'] for case in self.fixture['rta']}
        self.assertEqual(rta['<p>By entering you acknowledge you are at least 21 years old</p>'], 21)
        self.assertEqual(self.fixture['upstream_commit'], PINNED_UPSTREAM)

    def test_worker_uses_actual_helpers_without_network(self):
        self.assertIn('ie._og_search_property(', UPSTREAM_WORKER)
        self.assertIn('ie._rta_search(', UPSTREAM_WORKER)
        self.assertIn('subprocess.Popen = forbidden', UPSTREAM_WORKER)

    def run_main(self, revision):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'yt_dlp/__init__.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([], 0, revision, ''),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), '')]
            with mock.patch('sys.argv', ['compare_html_helpers.py', '--upstream', directory]), \
                    mock.patch('compare_html_helpers.subprocess.run', side_effect=outputs) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status, run, stdout.getvalue(), stderr.getvalue()

    def test_command_checks_pin_and_passes_exact_fixture(self):
        status, run, stdout, _ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status, 0)
        self.assertEqual(json.loads(run.call_args.kwargs['input']), self.fixture)
        self.assertIn('actual unescapeHTML', stdout)

    def test_wrong_revision_stops_before_worker(self):
        status, run, _, stderr = self.run_main('0' * 40)
        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn(PINNED_UPSTREAM, stderr)


if __name__ == '__main__':
    unittest.main()
