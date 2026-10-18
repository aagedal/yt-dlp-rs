import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_proxies import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class ProxyComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture=json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result={key:[{'name':c['name'],**c['expected']} for c in self.fixture[key]]
                     for key in ('cases','source_only_cases')}
        self.cases={c['name']:c['expected'] for c in self.fixture['cases']}

    def test_exact_endpoint_auth_route_and_source_boundaries_are_compared(self):
        before=copy.deepcopy((self.fixture,self.result))
        self.assertEqual(compare(self.result,self.fixture),self.result)
        self.assertEqual((self.fixture,self.result),before)
        for key in self.result:
            changed=copy.deepcopy(self.result);changed[key].pop()
            with self.assertRaisesRegex(ValueError,'proxy decision mismatch'):compare(changed,self.fixture)
        index=next(i for i,c in enumerate(self.result['cases']) if c['name']=='socks5')
        for field,value in [('rdns',False),('port',1),('username','changed')]:
            changed=copy.deepcopy(self.result);changed['cases'][index]['selected'][0][field]=value
            with self.assertRaises(ValueError):compare(changed,self.fixture)

    def test_explicit_direct_precedence_and_controlled_environment(self):
        for name in ['no-explicit-no-environment','explicit-direct','sentinel-direct','last-empty-direct']:
            self.assertEqual(self.cases[name]['selected'],[None,None],name)
        self.assertEqual(self.cases['explicit-overrides-no-proxy']['selected'][0]['host'],'proxy.test')
        self.assertEqual(self.cases['repeated-last-wins']['selected'][0]['host'],'last.test')
        self.assertEqual(self.cases['environment-http-fallback']['selected'][0],self.cases['environment-http-fallback']['selected'][1])
        self.assertEqual(self.cases['environment-no-proxy']['selected'][:2],[None,None])
        self.assertEqual(self.cases['environment-no-proxy']['selected'][2]['host'],'env-all.test')

    def test_aliases_ports_ipv6_and_decoded_empty_credentials(self):
        self.assertEqual(self.cases['socks']['selected'][0]['scheme'],'socks4')
        self.assertFalse(self.cases['socks4']['selected'][0]['rdns'])
        self.assertTrue(self.cases['socks4a']['selected'][0]['rdns'])
        self.assertEqual(self.cases['socks5']['selected'][0]['scheme'],'socks5h')
        self.assertTrue(self.cases['socks5']['selected'][0]['rdns'])
        self.assertEqual(self.cases['socks5-default']['selected'][0]['port'],1080)
        self.assertEqual(self.cases['http-default']['selected'][0]['port'],80)
        self.assertEqual(self.cases['https-default']['selected'][0]['port'],443)
        self.assertEqual(self.cases['http-ipv6']['selected'][0]['host'],'::1')
        for scheme in ['http','https','socks4','socks5']:
            decoded=self.cases[scheme+'-auth-decoded']['selected'][0]
            self.assertEqual(decoded['username'],'user@name');self.assertEqual(decoded['password'],'pa:ss+word')
            self.assertIsNone(self.cases[scheme+'-auth-user-only']['selected'][0]['password'])
            self.assertEqual(self.cases[scheme+'-auth-empty-password']['selected'][0]['password'],'')
            self.assertEqual(self.cases[scheme+'-auth-empty-user']['selected'][0]['username'],'')

    def test_actual_source_calls_and_excluded_permissive_cleanup_are_explicit(self):
        self.assertTrue(all(c['native_policy'] for c in self.fixture['source_only_cases']))
        legacy=self.result['source_only_cases'][0]
        self.assertEqual(legacy['headers'],{})
        self.assertEqual(legacy['selected'][0]['host'],'request.test')
        for fragment in ['parse_options','probe.proxies','clean_proxies','select_proxy','make_socks_proxy_opts',
                         'compat_urllib.request.getproxies=lambda','urllib.request.proxy_bypass=lambda',
                         'socket.socket=socket.create_connection=forbidden','subprocess.Popen=forbidden']:
            self.assertIn(fragment,UPSTREAM_WORKER)
        self.assertEqual(self.fixture['upstream_commit'],PINNED_UPSTREAM)
        policy_cases=[c for c in self.fixture['cases'] if 'native_policy' in c]
        self.assertEqual(len(policy_cases),11)
        self.assertEqual(sum(c['expected']['proxy_option'] is None for c in policy_cases),6)
        self.assertEqual(sum(c['name'].startswith('socks4-auth-') for c in policy_cases),5)
        self.assertEqual(sum('native_policy' not in c for c in self.fixture['cases']),39)

    def run_main(self,revision,fixture_path=None):
        with tempfile.TemporaryDirectory() as directory:
            module=Path(directory)/'yt_dlp/utils/networking.py';module.parent.mkdir(parents=True);module.touch()
            outputs=[subprocess.CompletedProcess([],0,revision,''),subprocess.CompletedProcess([],0,json.dumps(self.result),'')]
            with mock.patch('sys.argv',['compare_proxies.py','--upstream',directory]), \
                    mock.patch('compare_proxies.subprocess.run',side_effect=outputs) as run, \
                    mock.patch('compare_proxies.FIXTURE',fixture_path or FIXTURE), \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status=main()
            return status,run,stdout.getvalue(),stderr.getvalue()

    def test_command_uses_exact_pin_and_fixture(self):
        status,run,stdout,_=self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status,0);self.assertEqual(run.call_count,2)
        self.assertEqual(json.loads(run.call_args.kwargs['input']),self.fixture)
        self.assertIn('50 source configurations',stdout)

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
