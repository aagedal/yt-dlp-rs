import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest import mock

from compare_audio import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class AudioComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture=json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result={key:[{'name':c['name'],**c['expected']} for c in self.fixture[key]]
                     for key in ('cases','source_only_cases')}
        self.result['invalid_cases']=[c['name'] for c in self.fixture['invalid_cases']]
        self.cases={c['name']:c['expected'] for c in self.fixture['cases']}

    def test_exact_codec_options_paths_and_source_boundaries_are_compared(self):
        before=copy.deepcopy((self.fixture,self.result))
        self.assertEqual(compare(self.result,self.fixture),self.result)
        self.assertEqual((self.fixture,self.result),before)
        for key in self.result:
            changed=copy.deepcopy(self.result);changed[key].pop()
            with self.assertRaisesRegex(ValueError,'audio plan mismatch'):compare(changed,self.fixture)
        for field,value in [('output_ext','bad'),('delete',[]),('calls',[])]:
            changed=copy.deepcopy(self.result);changed['cases'][0][field]=value
            with self.assertRaises(ValueError):compare(changed,self.fixture)

    def test_quality_curves_and_bitrate_numeric_projection(self):
        for fmt,expected in [('mp3',[0,5,10]),('vorbis',[10,5,0]),('m4a',[4,2.05,0.1])]:
            for quality,value in zip(['0','5','10'],expected):
                args=self.cases['quality-'+fmt+'-'+quality]['quality_args']
                self.assertEqual(args['flag'],'-q:a')
                self.assertAlmostEqual(args['value'],value)
        self.assertEqual(self.cases['quality-mp3-128K']['quality_args'],{'flag':'-b:a','value':128,'unit':'k'})
        self.assertEqual(self.cases['quality-mp3-128K']['calls'][0]['more_opts'],['-b:a','128.0k'])
        self.assertEqual(self.cases['quality-mp3-128k']['quality_args'],self.cases['quality-mp3-128K']['quality_args'])
        self.assertEqual(self.cases['quality-mp3-10.5']['quality_args'],{'flag':'-b:a','value':10.5,'unit':'k'})
        self.assertIsNone(self.cases['quality-opus-5']['quality_args'])

    def test_copy_skip_mapping_and_original_temp_paths(self):
        self.assertEqual(self.cases['matching-aac']['calls'][0]['codec'],'copy')
        self.assertEqual(self.cases['matching-aac']['calls'][0]['more_opts'],['-f','adts'])
        self.assertEqual(self.cases['matching-aac']['output_ext'],'m4a')
        self.assertEqual(self.cases['unknown-best-mp3-fallback']['calls'][0]['codec'],'libmp3lame')
        for name in ['common-best-skip-mp3','common-best-skip-wav','map-no-match']:
            self.assertEqual(self.cases[name]['calls'],[])
            self.assertEqual(self.cases[name]['probes'],[])
        self.assertEqual(self.cases['same-extension-copy-skip']['calls'],[])
        self.assertEqual(self.cases['same-extension-transcode']['calls'][0]['output'],'movie.temp.mp3')
        self.assertEqual(self.cases['same-extension-transcode']['delete'],['movie.orig.mp3'])
        self.assertEqual(self.cases['append-extension']['output_path'],'movie.final.opus')
        self.assertEqual(self.cases['map-first']['mapping_target'],'mp3')
        self.assertEqual(self.cases['map-fallback']['mapping_target'],'mp3')
        self.assertEqual(self.cases['selector-default']['selector'],'bestaudio/best')
        self.assertIsNone(self.cases['selector-keep-video']['selector'])
        self.assertEqual(self.cases['selector-explicit']['selector'],'best')

    def test_actual_source_bugs_and_virtual_execution_are_explicit(self):
        cases={c['name']:c for c in self.fixture['source_only_cases']}
        self.assertIsNone(cases['alac-source-encode']['expected']['calls'][0]['codec'])
        self.assertEqual(cases['alac-source-encode']['expected']['calls'][0]['more_opts'],[])
        self.assertEqual(cases['alac-source-copy']['expected']['calls'][0]['more_opts'],['-acodec','alac'])
        self.assertEqual(cases['source-quality-inf']['expected']['quality_args']['value'],'inf')
        self.assertTrue(all(c['native_policy'] for c in cases.values()))
        for text in ['pp.run','resolve_mapping','parse_options','pp._features={\'fdk\':False}',
                     'def get_audio_codec','def run_ffmpeg','os.replace=lambda',
                     'socket.socket=socket.create_connection=forbidden','subprocess.Popen=forbidden']:
            self.assertIn(text,UPSTREAM_WORKER)
        self.assertEqual(self.fixture['upstream_commit'],PINNED_UPSTREAM)

    def run_main(self,revision,fixture_path=None):
        with tempfile.TemporaryDirectory() as directory:
            module=Path(directory)/'yt_dlp/postprocessor/ffmpeg.py';module.parent.mkdir(parents=True);module.touch()
            outputs=[subprocess.CompletedProcess([],0,revision,''),subprocess.CompletedProcess([],0,json.dumps(self.result),'')]
            with mock.patch('sys.argv',['compare_audio.py','--upstream',directory]), \
                    mock.patch('compare_audio.subprocess.run',side_effect=outputs) as run, \
                    mock.patch('compare_audio.FIXTURE',fixture_path or FIXTURE), \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status=main()
            return status,run,stdout.getvalue(),stderr.getvalue()

    def test_command_uses_exact_pin_and_fixture(self):
        status,run,stdout,_=self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status,0);self.assertEqual(run.call_count,2)
        self.assertEqual(json.loads(run.call_args.kwargs['input']),self.fixture)
        self.assertIn('73 plans',stdout)

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
