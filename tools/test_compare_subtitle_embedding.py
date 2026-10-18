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

from compare_subtitle_embedding import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class SubtitleEmbeddingComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = {key: [{'name': c['name'], **c['expected']} for c in self.fixture[key]]
                       for key in ('cases', 'source_only_cases')}
        self.result['language_cases'] = copy.deepcopy(self.fixture['language_cases'])
        self.result['invalid_cases'] = [c['name'] for c in self.fixture['invalid_cases']]
        self.cases = {c['name']: c['expected'] for c in self.fixture['cases']}

    def test_exact_argv_warnings_deletion_and_order_are_compared(self):
        before = copy.deepcopy((self.fixture, self.result))
        self.assertEqual(compare(self.result, self.fixture), self.result)
        self.assertEqual((self.fixture, self.result), before)
        for key in self.result:
            changed = copy.deepcopy(self.result)
            changed[key].pop()
            with self.assertRaisesRegex(ValueError, 'subtitle embedding mismatch'):
                compare(changed, self.fixture)
        for field in ('calls', 'delete', 'deleted_by_run_pp', 'replacements', 'messages'):
            changed = copy.deepcopy(self.result)
            changed['cases'][0][field] = []
            with self.assertRaises(ValueError):
                compare(changed, self.fixture)
        changed = copy.deepcopy(self.result)
        changed['cases'][0]['calls'][0]['args'].reverse()
        with self.assertRaises(ValueError):
            compare(changed, self.fixture)

    def test_all_six_containers_replace_existing_subtitle_maps(self):
        common = ['-map', '0', '-dn', '-ignore_unknown', '-c', 'copy']
        for ext in ('mp4', 'mov', 'm4a', 'webm', 'mkv', 'mka'):
            expected = self.cases['container-' + ext]
            prefix = common + (['-c:s', 'mov_text'] if ext in ('mp4', 'mov', 'm4a') else [])
            call = expected['calls'][0]
            self.assertEqual(call['args'], prefix + [
                '-map', '-0:s', '-map', '1:0', '-metadata:s:s:0', 'language=eng',
                '-metadata:s:s:0', 'handler_name=English', '-metadata:s:s:0', 'title=English'])
            self.assertEqual(call['inputs'], ['movie.' + ext, 'subs/movie.en.vtt'])
            self.assertEqual(call['output'], 'movie.temp.' + ext)
            self.assertEqual(expected['replacements'], [['movie.temp.' + ext, 'movie.' + ext]])
        self.assertEqual(self.cases['filename-no-extension']['calls'][0]['output'], 'movie.temp')
        self.assertEqual(self.cases['filename-nonmatching-extension']['calls'][0]['output'], 'movie.temp.final')

    def test_language_projection_uses_all_source_codes_and_case_sensitive_prefix(self):
        languages = {c['input']: c['expected'] for c in self.fixture['language_cases']}
        self.assertEqual(len(languages), 200)
        self.assertEqual(languages['en-US'], 'eng')
        self.assertEqual(languages['fra'], 'fra')
        self.assertEqual(languages['eng'], 'eng')
        self.assertEqual(languages['sco'], 'srd')
        self.assertEqual(languages['EN'], 'EN')
        self.assertEqual(languages['FR-ca'], 'FR-ca')
        self.assertEqual(languages[''], '')
        for value in ('en-US', 'fra', 'sco', 'EN', ''):
            case = self.cases['language-' + (value or 'empty')]
            args = case['calls'][0]['args']
            self.assertIn('language=' + languages[value], args)
        self.assertFalse(any('title=' in arg for arg in self.cases['name-empty']['calls'][0]['args']))
        self.assertIn('title=Français 第二行', self.cases['name-unicode']['calls'][0]['args'])

    def test_skips_warnings_and_eligible_deletions_are_not_guessed(self):
        for name in ('json-skipped', 'missing-skipped', 'webm-no-vtt', 'empty-subtitles',
                     'absent-subtitles', 'unsupported-container', 'uppercase-container-skip'):
            self.assertEqual(self.cases[name]['calls'], [])
            self.assertEqual(self.cases[name]['delete'], [])
        self.assertEqual(len(self.cases['webm-warning-once']['warnings']), 1)
        self.assertEqual(self.cases['webm-warning-once']['delete'], ['subs/movie.es.vtt'])
        self.assertEqual(len(self.cases['mp4-ass-warning-once']['warnings']), 1)
        self.assertEqual(len(self.cases['json-and-webm-warning']['warnings']), 2)
        self.assertEqual(len(self.cases['mp4-missing-ass-no-warning']['warnings']), 1)
        self.assertIn('file is missing', self.cases['mp4-missing-ass-no-warning']['warnings'][0])

    def test_parser_implied_write_and_actual_run_pp_keep_policy(self):
        implied = self.cases['container-mp4']
        self.assertTrue(implied['write_subtitles'])
        self.assertFalse(implied['already_have_subtitle'])
        self.assertEqual(implied['delete'], implied['deleted_by_run_pp'])
        manual = self.cases['flags-explicit-manual-write']
        self.assertTrue(manual['already_have_subtitle'])
        self.assertEqual(manual['delete'], [])
        auto = self.cases['flags-auto-only']
        self.assertFalse(auto['write_subtitles'])
        self.assertTrue(auto['write_automatic'])
        self.assertFalse(auto['already_have_subtitle'])
        self.assertEqual(len(auto['deleted_by_run_pp']), 1)
        self.assertTrue(self.cases['flags-explicit-no-manual-write']['write_subtitles'])
        self.assertFalse(self.cases['flags-explicit-no-manual-write']['already_have_subtitle'])
        kept = self.cases['flags-keep']
        self.assertEqual(kept['delete'], ['subs/movie.en.vtt'])
        self.assertEqual(kept['deleted_by_run_pp'], [])
        self.assertEqual(kept['files_to_move'], {'subs/movie.en.vtt': ''})
        self.assertEqual(len(self.cases['flags-keep-no-keep-last']['deleted_by_run_pp']), 1)
        self.assertEqual(self.cases['flags-no-keep-keep-last']['deleted_by_run_pp'], [])
        self.assertFalse(self.cases['flags-no-embed-last']['enabled'])
        self.assertFalse(self.cases['flags-no-embed-last']['write_subtitles'])
        self.assertTrue(self.cases['flags-embed-last']['enabled'])
        self.assertTrue(all(c['native_policy'] for c in self.fixture['source_only_cases']))

    def test_real_worker_blocks_network_process_and_file_effects(self):
        for effect in ["__import__('socket').create_connection(('example.invalid',80))",
                       "__import__('subprocess').Popen(['never-run'])",
                       "__import__('os').unlink('never-delete')", "open('never-read','rb')"]:
            with self.subTest(effect=effect), tempfile.TemporaryDirectory() as directory:
                package = Path(directory) / 'yt_dlp'
                (package / 'postprocessor').mkdir(parents=True)
                (package / '__init__.py').write_text(
                    "from types import SimpleNamespace\n"
                    "def parse_options(args):\n"
                    " return SimpleNamespace(ydl_opts={'postprocessors':[{'key':'FFmpegEmbedSubtitle','already_have_subtitle':False}]})\n"
                    "class YoutubeDL:\n"
                    " @staticmethod\n"
                    " def run_pp(probe,pp,info): return pp.run(info)\n", encoding='utf-8')
                (package / 'utils.py').write_text('class ISO639Utils: pass\n', encoding='utf-8')
                (package / 'postprocessor/ffmpeg.py').write_text(
                    'class FFmpegEmbedSubtitlePP:\n'
                    ' def __init__(self, **kwargs): pass\n'
                    ' def run(self, info):\n  ' + effect + '\n', encoding='utf-8')
                fixture = {'cases': [self.fixture['cases'][0]], 'source_only_cases': [],
                           'language_cases': [], 'invalid_cases': []}
                result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, directory],
                                        input=json.dumps(fixture), capture_output=True, text=True,
                                        encoding='utf-8', timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('Network/process/nonvirtual mutation forbidden', result.stderr)

    def run_main(self, revision, fixture_path=None):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'yt_dlp/postprocessor/ffmpeg.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([], 0, revision, ''),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), '')]
            with mock.patch('sys.argv', ['compare_subtitle_embedding.py', '--upstream', directory]), \
                    mock.patch('compare_subtitle_embedding.subprocess.run', side_effect=outputs) as run, \
                    mock.patch('compare_subtitle_embedding.FIXTURE', fixture_path or FIXTURE), \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status, run, stdout.getvalue(), stderr.getvalue()

    def test_command_requires_pin_and_transmits_exact_fixture(self):
        status, run, stdout, _ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status, 0)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(json.loads(run.call_args.kwargs['input']), self.fixture)
        self.assertIn('57 plans', stdout)
        self.assertIn('200 language projections', stdout)

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
