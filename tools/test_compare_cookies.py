import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_cookies import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class CookieComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture=json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result={key:[{'name':case['name'],**case['expected']} for case in self.fixture[key]]
                     for key in ('file_cases','response_cases','source_only_cases')}
        self.files={c['name']:c['expected'] for c in self.fixture['file_cases']}
        self.responses={c['name']:c['expected'] for c in self.fixture['response_cases']}

    def test_exact_scope_values_multisets_records_and_order_checks_are_compared(self):
        before=copy.deepcopy((self.fixture,self.result))
        self.assertEqual(compare(self.result,self.fixture),self.result)
        self.assertEqual((self.fixture,self.result),before)
        for key in self.result:
            changed=copy.deepcopy(self.result);changed[key].pop()
            with self.assertRaisesRegex(ValueError,'cookie metadata mismatch'):compare(changed,self.fixture)
        changed=copy.deepcopy(self.result)
        changed['file_cases'][0]['requests'][0]['pairs'].pop()
        with self.assertRaises(ValueError):compare(changed,self.fixture)
        changed=copy.deepcopy(self.result)
        index=next(i for i,c in enumerate(changed['file_cases']) if c['name']=='duplicate-path-order')
        changed['file_cases'][index]['requests'][0]['ordered_parts'].reverse()
        with self.assertRaises(ValueError):compare(changed,self.fixture)

    def test_session_expiry_and_logical_roundtrip_are_distinct_from_header_expiry(self):
        case=self.files['expiry-session']
        self.assertEqual(case['requests'][0]['pairs'],[['future','future'],['session_blank','blank'],['session_zero','zero']])
        records={record['name']:record for record in case['saved_records']}
        self.assertEqual(records['past']['expires'],1)
        self.assertIsNone(records['session_zero']['expires'])
        self.assertIsNone(records['session_blank']['expires'])
        self.assertTrue(case['saved_sessions_zero'])
        self.assertEqual(self.files['nameless']['requests'][0]['pairs'],[[None,'nameless-value']])
        self.assertEqual(self.files['duplicate-path-order']['requests'][0]['ordered_parts'],
                         ['sid=deep','sid=app','sid=replaced-root'])

    def test_domain_secure_path_and_actual_response_cookie_policy(self):
        requests=self.files['domain-secure-path-http-only']['requests']
        self.assertIn(['secure','S'],requests[0]['pairs'])
        self.assertNotIn(['secure','S'],requests[1]['pairs'])
        self.assertNotIn(['path','P'],requests[2]['pairs'])
        self.assertEqual(requests[-1]['pairs'],[])
        for name in ['response-max-age-delete','response-expired-delete','response-domain-rejected']:
            self.assertTrue(all(not r['pairs'] for r in self.responses[name]['requests']),name)
        self.assertEqual(self.responses['response-default-path']['requests'][0]['pairs'],[['sid','session']])
        self.assertEqual(self.responses['response-default-path']['requests'][1]['pairs'],[])
        self.assertEqual(self.responses['response-replacement']['requests'][0]['pairs'],[['sid','two']])

    def test_source_only_differences_and_worker_isolation_are_explicit(self):
        cases={c['name']:c for c in self.fixture['source_only_cases']}
        self.assertEqual(cases['host-only-python-subdomain']['expected']['requests'][0]['pairs'],[['host','H']])
        self.assertTrue(cases['malformed-skipped']['expected']['warning'])
        self.assertTrue(all(c['native_policy'] for c in cases.values()))
        for text in ['jar.load','jar.save','reloaded.load','reloaded.get_cookie_header','jar.extract_cookies',
                     'socket.socket = socket.create_connection = forbidden','subprocess.Popen = forbidden']:
            self.assertIn(text,UPSTREAM_WORKER)
        self.assertEqual(self.fixture['upstream_commit'],PINNED_UPSTREAM)

    def run_main(self,revision,fixture_path=None):
        with tempfile.TemporaryDirectory() as directory:
            module=Path(directory)/'yt_dlp/cookies.py';module.parent.mkdir(parents=True);module.touch()
            outputs=[subprocess.CompletedProcess([],0,revision,''),subprocess.CompletedProcess([],0,json.dumps(self.result),'')]
            with mock.patch('sys.argv',['compare_cookies.py','--upstream',directory]), \
                    mock.patch('compare_cookies.subprocess.run',side_effect=outputs) as run, \
                    mock.patch('compare_cookies.FIXTURE',fixture_path or FIXTURE), \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status=main()
            return status,run,stdout.getvalue(),stderr.getvalue()

    def test_command_uses_exact_pin_and_shared_fixture(self):
        status,run,stdout,_=self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status,0);self.assertEqual(run.call_count,2)
        self.assertEqual(json.loads(run.call_args.kwargs['input']),self.fixture)
        self.assertIn('5 files, 6 response cases',stdout)

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
