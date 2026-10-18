import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_headers import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class HeaderComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture=json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result={key:[{'name':case['name'],'headers':case['expected_headers']} for case in self.fixture[key]]
                     for key in ('cases','request_cases','source_only_cases')}
        self.result['invalid_cases']=[case['name'] for case in self.fixture['invalid_cases']]

    def test_exact_values_and_all_case_categories_are_compared(self):
        before=copy.deepcopy((self.fixture,self.result))
        self.assertEqual(compare(self.result,self.fixture),self.result)
        self.assertEqual((self.fixture,self.result),before)
        for key in self.result:
            changed=copy.deepcopy(self.result);changed[key].pop()
            with self.assertRaisesRegex(ValueError,'request header mismatch'):
                compare(changed,self.fixture)
        changed=copy.deepcopy(self.result)
        changed['cases'][1]['headers']['x-test']='value '
        with self.assertRaises(ValueError):compare(changed,self.fixture)

    def test_actual_option_precedence_first_colon_and_empty_values(self):
        cases={case['name']:case['expected_headers'] for case in self.fixture['cases']}
        self.assertEqual(cases['case-insensitive-last'],{'x-test':'second'})
        self.assertEqual(cases['first-colon'],{'x-url':'https://example.test:8443/path:a'})
        self.assertEqual(cases['trim-value'],{'x-test':'value with spaces'})
        self.assertEqual(cases['empty-value'],{'x-empty':''})
        self.assertEqual(cases['whitespace-value'],{'x-empty':''})
        self.assertEqual(cases['convenience-before'],cases['convenience-after'])
        self.assertEqual(cases['convenience-before'],{'user-agent':'flag-UA','referer':'flag-ref'})
        self.assertEqual(cases['empty-convenience-beats-header'],{'user-agent':''})
        self.assertEqual(cases['abbreviated-option'],cases['custom'])

    def test_merge_precedence_and_cookie_scope_are_explicit(self):
        cases={case['name']:case for case in self.fixture['request_cases']}
        self.assertEqual(cases['extractor-overrides-global']['expected_headers'],
                         {'user-agent':'Fixture-Default-UA','accept':'*/*','x-test':'extractor','referer':'page-ref'})
        self.assertEqual(cases['extractor-empty-remains']['expected_headers']['x-test'],'')
        self.assertNotIn('cookie',cases['cookie-removed']['expected_headers'])
        self.assertTrue(cases['cookie-removed']['native_policy'])
        self.assertTrue(all(case['native_policy'] for case in self.fixture['source_only_cases']))
        for fragment in ['parse_options','HTTPHeaderDict','YoutubeDL._calc_headers','YoutubeDLCookieJar',
                         'socket.socket = socket.create_connection = forbidden','subprocess.Popen = forbidden']:
            self.assertIn(fragment,UPSTREAM_WORKER)
        self.assertEqual(self.fixture['upstream_commit'],PINNED_UPSTREAM)

    def run_main(self,revision,fixture_path=None):
        with tempfile.TemporaryDirectory() as directory:
            module=Path(directory)/'yt_dlp/YoutubeDL.py';module.parent.mkdir(parents=True);module.touch()
            outputs=[subprocess.CompletedProcess([],0,revision,''),subprocess.CompletedProcess([],0,json.dumps(self.result),'')]
            with mock.patch('sys.argv',['compare_headers.py','--upstream',directory]), \
                    mock.patch('compare_headers.subprocess.run',side_effect=outputs) as run, \
                    mock.patch('compare_headers.FIXTURE',fixture_path or FIXTURE), \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status=main()
            return status,run,stdout.getvalue(),stderr.getvalue()

    def test_command_uses_exact_pin_and_shared_fixture(self):
        status,run,stdout,_=self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status,0);self.assertEqual(run.call_count,2)
        self.assertEqual(json.loads(run.call_args.kwargs['input']),self.fixture)
        self.assertIn('21 options',stdout)

    def test_wrong_revision_stops_before_worker(self):
        status,run,_,stderr=self.run_main('0'*40)
        self.assertEqual(status,1);self.assertEqual(run.call_count,1)
        self.assertIn(PINNED_UPSTREAM,stderr)

    def test_wrong_fixture_pin_stops_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            path=Path(directory)/'wrong.json';self.fixture['upstream_commit']='0'*40
            path.write_text(json.dumps(self.fixture),encoding='utf-8')
            status,run,_,stderr=self.run_main(PINNED_UPSTREAM,path)
        self.assertEqual(status,1);self.assertEqual(run.call_count,1)
        self.assertIn('fixture',stderr)


if __name__=='__main__':unittest.main()
