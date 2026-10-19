import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_codecs import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class CodecComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = {key: copy.deepcopy(self.fixture[key])
                       for key in ('parse_codecs', 'compatible_ext')}

    def test_results_warnings_and_order_are_compared(self):
        self.assertEqual(compare(self.result, self.fixture), self.result)
        for mutate in (
            lambda result: result['parse_codecs'][1]['result'].update(acodec='none'),
            lambda result: result['parse_codecs'][5]['warnings'].clear(),
            lambda result: result['compatible_ext'][0].update(ext='mkv'),
            lambda result: result['compatible_ext'].reverse(),
        ):
            changed = copy.deepcopy(self.result)
            mutate(changed)
            with self.assertRaisesRegex(ValueError, 'codec mismatch'):
                compare(changed, self.fixture)

    def test_saved_upstream_semantics_remain_explicit(self):
        cases = {case['input']: case for case in self.fixture['parse_codecs']}
        self.assertEqual(cases['vp9.2']['result']['dynamic_range'], 'HDR10')
        self.assertEqual(cases['unknownvcodec, unknownacodec']['result'],
                         {'vcodec': 'unknownvcodec', 'acodec': 'unknownacodec'})
        self.assertEqual(cases[', h264,,newcodec,aac']['warnings'],
                         ['WARNING: Unknown codec newcodec'])
        self.assertEqual(cases['']['result'], {})
        exts = [case['ext'] for case in self.fixture['compatible_ext']]
        self.assertIn('webm', exts)
        self.assertIn('mkv', exts)
        self.assertEqual(self.fixture['upstream_commit'], PINNED_UPSTREAM)

    def test_worker_uses_actual_helpers_without_network(self):
        self.assertIn('parse_codecs(case', UPSTREAM_WORKER)
        self.assertIn('get_compatible_ext(**entry)', UPSTREAM_WORKER)
        self.assertIn('subprocess.Popen = forbidden', UPSTREAM_WORKER)

    def run_main(self, revision):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'yt_dlp/__init__.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([], 0, revision, ''),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), '')]
            with mock.patch('sys.argv', ['compare_codecs.py', '--upstream', directory]), \
                    mock.patch('compare_codecs.subprocess.run', side_effect=outputs) as run, \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status, run, stdout.getvalue(), stderr.getvalue()

    def test_command_checks_pin_and_passes_exact_fixture(self):
        status, run, stdout, _ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status, 0)
        self.assertEqual(json.loads(run.call_args.kwargs['input']), self.fixture)
        self.assertIn('actual parse_codecs', stdout)

    def test_wrong_revision_stops_before_worker(self):
        status, run, _, stderr = self.run_main('0' * 40)
        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn(PINNED_UPSTREAM, stderr)


if __name__ == '__main__':
    unittest.main()
