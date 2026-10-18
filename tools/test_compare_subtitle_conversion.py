import contextlib
import copy
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

from compare_subtitle_conversion import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class SubtitleConversionComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = {'ttml_cases': [{'name': c['name'], 'srt': c['expected_srt']}
                                      for c in self.fixture['ttml_cases']]}
        self.result.update({key: [{'name': c['name'], **c['expected']} for c in self.fixture[key]]
                            for key in ('cases', 'source_only_cases')})
        self.result['invalid_cases'] = [c['name'] for c in self.fixture['invalid_cases']]
        self.cases = {c['name']: c['expected'] for c in self.fixture['cases']}

    def test_exact_text_timing_order_paths_and_deletions_are_compared(self):
        before = copy.deepcopy((self.fixture, self.result))
        self.assertEqual(compare(self.result, self.fixture), self.result)
        self.assertEqual((self.fixture, self.result), before)
        for key in self.result:
            changed = copy.deepcopy(self.result)
            changed[key].pop()
            with self.assertRaisesRegex(ValueError, 'subtitle conversion mismatch'):
                compare(changed, self.fixture)
        changed = copy.deepcopy(self.result)
        changed['ttml_cases'][0]['srt'] = changed['ttml_cases'][0]['srt'].rstrip()
        with self.assertRaises(ValueError):
            compare(changed, self.fixture)
        for field in ('calls', 'delete', 'writes', 'warnings'):
            changed = copy.deepcopy(self.result)
            case = next(c for c in changed['cases'] if c['name'] == 'ttml-to-vtt')
            case[field] = []
            with self.assertRaises(ValueError):
                compare(changed, self.fixture)

    def test_ordinary_formats_and_webvtt_muxer_match_captured_source_calls(self):
        for source in ('srt', 'vtt', 'ass', 'lrc'):
            for target in ('srt', 'vtt', 'ass', 'lrc'):
                expected = self.cases[source + '-to-' + target]
                self.assertEqual(expected['when'], 'before_dl')
                if source == target:
                    self.assertEqual(expected['calls'], [])
                    self.assertEqual(expected['delete'], [])
                    self.assertEqual(expected['subtitles'][0]['name'], 'English')
                else:
                    self.assertEqual(expected['calls'][0]['args'],
                                     ['-f', 'webvtt' if target == 'vtt' else target])
                    self.assertEqual(expected['delete'], ['temp/movie.en.' + source])
                    self.assertEqual(expected['subtitles'][0]['ext'], target)
                    self.assertNotIn('name', expected['subtitles'][0])
        self.assertEqual(self.cases['filename-last-suffix-replaced']['subtitles'][0]['filepath'],
                         'temp/movie.srt')

    def test_ttml_intermediate_native_text_and_move_map_quirk_are_explicit(self):
        corpus = {c['name']: c for c in self.fixture['ttml_cases']}
        for source in ('tt', 'ttml', 'dfxp'):
            srt = self.cases[source + '-to-srt']
            self.assertEqual(srt['calls'], [])
            self.assertEqual(srt['writes'], [{'path': 'temp/movie.en.srt',
                                               'data': corpus['upstream-basic']['expected_srt']}])
            self.assertEqual(srt['delete'], ['temp/movie.en.' + source])
            self.assertNotIn('temp/movie.en.srt', srt['files_to_move'])
            vtt = self.cases[source + '-to-vtt']
            self.assertEqual(vtt['calls'], [{'input': 'temp/movie.en.srt',
                                              'output': 'temp/movie.en.vtt', 'args': ['-f', 'webvtt']}])
            self.assertEqual(vtt['delete'], ['temp/movie.en.' + source, 'temp/movie.en.srt'])
            self.assertEqual(vtt['files_to_move']['temp/movie.en.vtt'], 'published/movie.en.vtt')
            self.assertIn('style information loss', vtt['warnings'][0])
        self.assertEqual(len(corpus), 15)
        self.assertEqual(corpus['timing-all-invalid-empty-output']['expected_srt'], '')
        self.assertTrue(corpus['timing-skips-index-gaps']['expected_srt'].startswith('3\n'))
        self.assertIn('02,000 --> 00:00:01,000', corpus['timing-skips-index-gaps']['expected_srt'])
        self.assertTrue((FIXTURE.parent / corpus['upstream-utf16']['input_file']).read_bytes().startswith(b'\xff\xfe'))
        self.assertTrue((FIXTURE.parent / corpus['utf16-be-bom']['input_file']).read_bytes().startswith(b'\xfe\xff'))
        self.assertIn('<font', corpus['upstream-styles-forward-inheritance']['expected_srt'])
        self.assertIn('第二行', corpus['upstream-basic']['expected_srt'])

    def test_options_aliases_none_repeat_json_and_missing_file_policies(self):
        for name in ('alias-short', 'alias-long'):
            self.assertEqual(self.cases[name]['target'], 'srt')
        for name in ('none', 'none-last'):
            self.assertIsNone(self.cases[name]['target'])
            self.assertEqual(self.cases[name]['calls'], [])
        self.assertEqual(self.cases['last-target']['target'], 'ass')
        self.assertTrue(self.cases['skip-download']['skip_download'])
        self.assertEqual(self.cases['skip-download']['when'], 'before_dl')
        self.assertEqual(self.cases['audio-and-video']['selector'], 'bestaudio/best')
        self.assertEqual(self.cases['json-special-skip']['calls'], [])
        self.assertEqual(self.cases['json-special-skip']['delete'], [])
        self.assertIn('currently not possible', self.cases['json-special-skip']['messages'][-1])
        self.assertIn('file is missing', self.cases['missing-file']['warnings'][0])
        self.assertIsNone(self.cases['no-requested-subtitles']['subtitles'])
        source = self.fixture['source_only_cases'][0]
        self.assertTrue(source['native_policy'])
        self.assertEqual(len(source['expected']['calls']), 1)
        self.assertEqual(source['subtitles'][0]['ext'], 'json3')

    def test_actual_worker_blocks_network_process_and_nonvirtual_io(self):
        for effect, diagnostic in [
            ("__import__('socket').create_connection(('example.invalid',80))", 'Network/process/nonvirtual'),
            ("__import__('subprocess').Popen(['never-run'])", 'Network/process/nonvirtual'),
            ("__import__('os').replace('never-read','never-written')", 'Network/process/nonvirtual'),
            ("open('never-read','rb')", 'Nonvirtual subtitle file access forbidden'),
        ]:
            with self.subTest(effect=effect), tempfile.TemporaryDirectory() as directory:
                package = Path(directory) / 'yt_dlp'
                (package / 'postprocessor').mkdir(parents=True)
                (package / '__init__.py').write_text(
                    "from types import SimpleNamespace\n"
                    "def parse_options(args):\n"
                    " return SimpleNamespace(ydl_opts={'postprocessors':[{'key':'FFmpegSubtitlesConvertor','format':'srt','when':'before_dl'}]})\n",
                    encoding='utf-8')
                (package / 'utils.py').write_text('def dfxp2srt(data): return ""\n', encoding='utf-8')
                (package / 'postprocessor/ffmpeg.py').write_text(
                    "class FFmpegSubtitlesConvertorPP:\n"
                    " def __init__(self, **kwargs): pass\n"
                    " def run(self, info):\n  " + effect + '\n', encoding='utf-8')
                fixture = {'ttml_cases': [], 'cases': [self.fixture['cases'][0]],
                           'source_only_cases': [], 'invalid_cases': []}
                result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, directory, directory],
                                        input=json.dumps(fixture), capture_output=True, text=True,
                                        encoding='utf-8', timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(diagnostic, result.stderr)

    def run_main(self, revision, fixture_path=None):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'yt_dlp/postprocessor/ffmpeg.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([], 0, revision, ''),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), '')]
            with mock.patch('sys.argv', ['compare_subtitle_conversion.py', '--upstream', directory]), \
                    mock.patch('compare_subtitle_conversion.subprocess.run', side_effect=outputs) as run, \
                    mock.patch('compare_subtitle_conversion.FIXTURE', fixture_path or FIXTURE), \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status, run, stdout.getvalue(), stderr.getvalue()

    def test_command_pins_source_and_transmits_exact_fixture(self):
        status, run, stdout, _ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status, 0)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(json.loads(run.call_args.kwargs['input']), self.fixture)
        self.assertIn('15 exact TTML texts', stdout)
        self.assertIn('42 plans', stdout)

    def test_wrong_source_revision_stops_before_worker(self):
        status, run, _, stderr = self.run_main('0' * 40)
        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn(PINNED_UPSTREAM, stderr)

    def test_wrong_fixture_revision_stops_before_worker(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = copy.deepcopy(self.fixture)
            fixture['upstream_commit'] = '0' * 40
            path = Path(directory) / 'wrong.json'
            path.write_text(json.dumps(fixture), encoding='utf-8')
            status, run, _, stderr = self.run_main(PINNED_UPSTREAM, path)
        self.assertEqual(status, 1)
        self.assertEqual(run.call_count, 1)
        self.assertIn('fixture', stderr)


if __name__ == '__main__':
    unittest.main()
