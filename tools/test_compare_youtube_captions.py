import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_youtube_captions import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class YoutubeCaptionComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = {key:[{'name':c['name'], **c['expected']} for c in self.fixture[key]]
                       for key in ('cases','source_only_cases','selection_cases')}
        self.cases = {c['name']:c['expected'] for c in self.fixture['cases']}

    def test_exact_urls_names_order_and_policy_are_compared_without_normalization(self):
        before=copy.deepcopy((self.fixture,self.result))
        self.assertEqual(compare(self.result,self.fixture),self.result)
        self.assertEqual((self.fixture,self.result),before)
        for field,value in [('audio_language','xx'),('warning',True),('token_requests',[]),('policy_skip',True)]:
            changed=copy.deepcopy(self.result)
            changed['cases'][0][field]=value
            with self.assertRaisesRegex(ValueError,'YouTube caption mismatch'):
                compare(changed,self.fixture)
        for key in self.result:
            changed=copy.deepcopy(self.result);changed[key].pop()
            with self.assertRaises(ValueError):compare(changed,self.fixture)
        changed=copy.deepcopy(self.result)
        changed['cases'][0]['subtitles'][0]['formats'][0]['url']+='&xosf=1'
        with self.assertRaises(ValueError):compare(changed,self.fixture)

    def test_seven_formats_query_grouping_blank_drop_and_encoding(self):
        formats=self.cases['manual-query']['subtitles'][0]['formats']
        self.assertEqual([f['ext'] for f in formats],['json3','srv1','srv2','srv3','ttml','srt','vtt'])
        self.assertEqual(formats[0]['url'],'https://www.youtube.com/api/timedtext?dup=one&dup=two&lang=en&fmt=json3&keep=a%2Fb&space=a+b&reserved=%26%3D%2B%3F')
        self.assertEqual(self.cases['manual-query']['subtitles'][0]['language'],'en-US')
        self.assertEqual(self.cases['names-runs']['subtitles'][0]['formats'][0]['name'],'First')
        self.assertEqual(self.cases['relative-base']['subtitles'][0]['formats'][0]['url'],'http://www.youtube.com/api/timedtext?lang=en&fmt=json3')

    def test_translations_orig_aliases_and_missing_names_remain_source_exact(self):
        before=self.cases['manual-translations']['automatic_captions']
        after=self.cases['manual-translations-translated']['automatic_captions']
        self.assertEqual([g['language'] for g in before],['und'])
        self.assertEqual([g['language'] for g in after],['en-en','fr-en','und'])
        self.assertIn('tlang=fr-en',after[1]['formats'][0]['url'])
        self.assertNotIn('tlang=',after[0]['formats'][0]['url'])
        self.assertEqual(after[1]['formats'][0]['name'],'French from English')
        self.assertEqual(after,self.cases['manual-translations-via-list']['automatic_captions'])
        asr=self.cases['asr-translations']
        self.assertEqual(asr['audio_language'],'en')
        self.assertEqual([g['language'] for g in asr['automatic_captions']],['en-orig','en','fr'])
        self.assertEqual(self.cases['missing-name-asr']['automatic_captions'][0]['formats'][0]['name'],'None (Original)')
        self.assertIsNone(self.cases['missing-name-asr']['automatic_captions'][1]['formats'][0]['name'])

    def test_po_token_policy_is_not_bypassed_by_existing_url_token(self):
        for exp in ['xpe','xpv']:
            case=self.cases['token-'+exp]
            self.assertEqual(case['token_requests'],[{'required':True}])
            self.assertTrue(case['policy_skip'])
            self.assertEqual(case['subtitles'],[])
            self.assertFalse(case['warning'])
            self.assertTrue(self.cases['token-'+exp+'-translated']['warning'])
        self.assertEqual(self.cases['token-after-first']['token_requests'],[{'required':False}])
        self.assertTrue(self.cases['token-after-first']['policy_skip'])
        self.assertEqual(len(self.cases['token-after-first']['subtitles']),1)
        self.assertFalse(self.cases['comma-exp']['policy_skip'])
        self.assertIn('exp=xpe%2Cxpv',self.cases['comma-exp']['subtitles'][0]['formats'][0]['url'])

    def test_selection_precedence_and_reference_isolation(self):
        selections={c['name']:c['expected']['selected'] for c in self.fixture['selection_cases']}
        self.assertEqual(selections['precedence-False-False'],[])
        self.assertEqual(selections['normal-prefix-before-auto-exact'][0]['language'],'en-US')
        self.assertEqual(selections['precedence-True-False'],selections['precedence-True-True'])
        self.assertNotEqual(selections['precedence-True-True'],selections['precedence-False-True'])
        for fragment in ['ie._real_extract','probe.process_subtitles','socket.socket = socket.create_connection = forbidden','subprocess.Popen = forbidden','STREAMING_DATA_FETCH_SUBS_PO_TOKEN','sanitize_url']:
            self.assertIn(fragment,UPSTREAM_WORKER)
        self.assertTrue(all(c['native_policy'] for c in self.fixture['source_only_cases']))

    def run_main(self,revision,fixture_path=None):
        with tempfile.TemporaryDirectory() as directory:
            module=Path(directory)/'yt_dlp/YoutubeDL.py';module.parent.mkdir(parents=True);module.touch()
            outputs=[subprocess.CompletedProcess([],0,revision,''),subprocess.CompletedProcess([],0,json.dumps(self.result),'')]
            with mock.patch('sys.argv',['compare_youtube_captions.py','--upstream',directory]), \
                    mock.patch('compare_youtube_captions.subprocess.run',side_effect=outputs) as run, \
                    mock.patch('compare_youtube_captions.FIXTURE',fixture_path or FIXTURE), \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status=main()
            return status,run,stdout.getvalue(),stderr.getvalue()

    def test_command_passes_exact_pin_and_fixture(self):
        status,run,stdout,_=self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status,0);self.assertEqual(run.call_count,2)
        self.assertEqual(json.loads(run.call_args.kwargs['input']),self.fixture)
        self.assertIn('20 shared',stdout)

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
