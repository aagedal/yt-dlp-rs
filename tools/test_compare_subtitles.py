import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_subtitles import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class SubtitleComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = {key: [{'name':c['name'], **c['expected']} for c in self.fixture[key]]
                       for key in ('html_cases','selection_cases','filename_cases','source_only_cases','option_cases')}

    def test_exact_metadata_selection_order_warnings_and_paths_are_compared(self):
        before = copy.deepcopy((self.fixture, self.result))
        self.assertEqual(compare(self.result, self.fixture), self.result)
        self.assertEqual((self.fixture, self.result), before)
        for key in self.result:
            changed = copy.deepcopy(self.result)
            changed[key].pop()
            with self.assertRaisesRegex(ValueError, 'subtitle rendering mismatch'):
                compare(changed, self.fixture)
        changed = copy.deepcopy(self.result)
        changed['selection_cases'][0]['warning'] = True
        with self.assertRaises(ValueError):
            compare(changed, self.fixture)

    def test_metadata_preserves_optional_languages_and_completes_extensions(self):
        html = {c['name']:c['expected']['entries'] for c in self.fixture['html_cases']}
        groups = html['language-preserved-or-none'][0]['subtitles']
        self.assertEqual([entry['language'] for entry in groups], [None, ' en ', '', 'English captions'])
        self.assertEqual(len(groups[0]['formats']), 2)
        entity = html['src-trim-entities-query'][0]['subtitles']
        self.assertEqual(entity[0]['formats'], [{'url':'https://example.test/subs/captions.VTT?x=1&y=2', 'ext':'vtt'}])
        self.assertEqual(entity[2]['formats'][0]['ext'], 'unknown_video')
        self.assertEqual(entity[1]['formats'][0]['url'], 'http://cdn.example.test/a.SRT#part')
        self.assertEqual(entity[3]['formats'], [{'url':'https://cdn.example.test/typo.VTT', 'ext':'vtt'}])
        self.assertEqual([s['language'] for s in html['kind-language-priority'][0]['subtitles']], ['en','fr','Label'])

    def test_selection_formats_language_case_and_default_rules(self):
        cases = {c['name']:c['expected'] for c in self.fixture['selection_cases']}
        self.assertEqual(cases['default-exact-en']['selected'][0]['language'], 'en')
        self.assertEqual(cases['default-prefix']['selected'][0]['language'], 'en-US')
        self.assertEqual(cases['default-case-sensitive']['selected'][0]['language'], 'fr')
        self.assertEqual(cases['literal-case']['selected'][0]['language'], 'en')
        self.assertEqual(cases['readd-order']['selected'][-1]['language'], 'en')
        self.assertEqual(cases['format-srt']['selected'][0]['url'], 'https://example.test/2-2.srt')
        self.assertTrue(cases['format-ass']['warning'])
        self.assertFalse(cases['missing']['warning'])
        self.assertEqual(cases['missing']['selected'], [])
        self.assertEqual(cases['write-disabled']['selected'], [])

    def test_option_combinations_and_unknown_language_boundaries(self):
        cases = {c['name']:c['expected'] for c in self.fixture['option_cases']}
        self.assertEqual(cases['list']['simulate'], 'list_only')
        self.assertIs(cases['list-json']['simulate'], True)
        self.assertIs(cases['list-simulate']['simulate'], True)
        self.assertTrue(cases['skip-write']['skip_download'])
        self.assertIsNone(cases['skip-write']['simulate'])
        self.assertTrue(cases['skip-write']['write'])
        self.assertEqual(self.fixture['source_only_cases'][0]['expected']['error'], 'AttributeError')
        self.assertTrue(all(c['native_policy'] for c in self.fixture['source_only_cases']))
        for actual in ['ie._parse_html5_media_entries','probe.process_subtitles','subtitles_filename','determine_ext','sanitize_url','parse_options','subprocess.Popen = forbidden','socket.socket = socket.create_connection = forbidden']:
            self.assertIn(actual, UPSTREAM_WORKER)

    def run_main(self, revision, fixture_path=None):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'yt_dlp/YoutubeDL.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([],0,revision,''),
                       subprocess.CompletedProcess([],0,json.dumps(self.result),'')]
            with mock.patch('sys.argv',['compare_subtitles.py','--upstream',directory]), \
                    mock.patch('compare_subtitles.subprocess.run',side_effect=outputs) as run, \
                    mock.patch('compare_subtitles.FIXTURE',fixture_path or FIXTURE), \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status,run,stdout.getvalue(),stderr.getvalue()

    def test_command_uses_exact_pin_and_fixture(self):
        status,run,stdout,_ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status,0)
        self.assertEqual(run.call_count,2)
        self.assertEqual(json.loads(run.call_args.kwargs['input']),self.fixture)
        self.assertIn('36 selection',stdout)

    def test_wrong_revision_stops_before_worker(self):
        status,run,_,stderr = self.run_main('0'*40)
        self.assertEqual(status,1)
        self.assertEqual(run.call_count,1)
        self.assertIn(PINNED_UPSTREAM,stderr)

    def test_wrong_fixture_pin_stops_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'wrong.json'
            self.fixture['upstream_commit']='0'*40
            path.write_text(json.dumps(self.fixture),encoding='utf-8')
            status,run,_,stderr=self.run_main(PINNED_UPSTREAM,path)
        self.assertEqual(status,1)
        self.assertEqual(run.call_count,1)
        self.assertIn('fixture',stderr)


if __name__ == '__main__':
    unittest.main()
