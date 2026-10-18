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

from compare_video import FIXTURE, PINNED_UPSTREAM, UPSTREAM_WORKER, compare, main


class VideoComparisonTests(unittest.TestCase):
    def setUp(self):
        self.fixture = json.loads(FIXTURE.read_text(encoding='utf-8'))
        self.result = {key: [{'name': c['name'], **c['expected']} for c in self.fixture[key]]
                       for key in ('cases', 'source_only_cases')}
        self.result['invalid_cases'] = [c['name'] for c in self.fixture['invalid_cases']]
        self.cases = {c['name']: c['expected'] for c in self.fixture['cases']}

    def test_exact_options_paths_deletions_and_order_are_significant(self):
        before = copy.deepcopy((self.fixture, self.result))
        self.assertEqual(compare(self.result, self.fixture), self.result)
        self.assertEqual((self.fixture, self.result), before)
        for key in self.result:
            changed = copy.deepcopy(self.result)
            changed[key].pop()
            with self.assertRaisesRegex(ValueError, 'video plan mismatch'):
                compare(changed, self.fixture)
        for field, value in [('output_ext', 'bad'), ('delete', []), ('calls', []),
                             ('warnings', ['wrong']), ('output_path', 'other.mp4')]:
            changed = copy.deepcopy(self.result)
            changed['cases'][0][field] = value
            with self.assertRaises(ValueError):
                compare(changed, self.fixture)
        changed = copy.deepcopy(self.result)
        changed['cases'][0]['calls'][0]['args'].reverse()
        with self.assertRaises(ValueError):
            compare(changed, self.fixture)

    def test_all_source_targets_copy_and_recode_options_are_captured(self):
        targets = {'avi', 'flv', 'gif', 'mkv', 'mov', 'mp4', 'webm', 'aac', 'aiff',
                   'alac', 'flac', 'm4a', 'mka', 'mp3', 'ogg', 'opus', 'vorbis', 'wav'}
        common = ['-map', '0', '-dn', '-ignore_unknown']
        for mode in ('remux', 'recode'):
            plans = {name.removeprefix(mode + '-target-'): expected
                     for name, expected in self.cases.items() if name.startswith(mode + '-target-')}
            self.assertEqual(set(plans), targets)
            for target, expected in plans.items():
                args = common + (['-c', 'copy'] if mode == 'remux' else [])
                if mode == 'recode' and target == 'avi':
                    args += ['-c:v', 'libxvid', '-vtag', 'XVID']
                self.assertEqual(expected['calls'][0]['args'], args)
                self.assertEqual(expected['output_ext'], target)
                self.assertEqual(expected['delete'], [expected['calls'][0]['input']])

    def test_mapping_skips_casefold_and_filename_suffix_rules(self):
        for mode in ('remux', 'recode'):
            for suffix in ('same-extension', 'same-extension-casefold', 'map-unmatched'):
                expected = self.cases[mode + '-' + suffix]
                self.assertEqual(expected['calls'], [])
                self.assertEqual(expected['delete'], [])
                self.assertEqual(expected['output_format'], 'fixture')
            self.assertEqual(self.cases[mode + '-same-extension-casefold']['output_ext'], 'MP4')
            self.assertEqual(self.cases[mode + '-source-casefold']['output_path'], 'movie.WEBM.mp4')
            self.assertEqual(self.cases[mode + '-append-nonmatching']['output_path'], 'movie.final.mp4')
            self.assertEqual(self.cases[mode + '-map-first']['output_ext'], 'mkv')
            self.assertEqual(self.cases[mode + '-map-fallback']['output_ext'], 'mp4')
            self.assertEqual(self.cases[mode + '-map-source-case-trim']['output_path'], 'movie.mp4')

    def test_option_conflicts_selection_and_keep_flags_follow_actual_parser(self):
        for mode in ('remux', 'recode'):
            self.assertTrue(self.cases[mode + '-keep']['keep_video'])
            self.assertFalse(self.cases[mode + '-no-keep-last']['keep_video'])
            self.assertTrue(self.cases[mode + '-keep-last']['keep_video'])
            self.assertEqual(self.cases[mode + '-explicit-selector']['selector'], 'worst')
            self.assertEqual(self.cases[mode + '-audio-combination']['selector'], 'bestaudio/best')
            self.assertTrue(self.cases[mode + '-audio-combination']['extract_audio'])
            self.assertIsNone(self.cases[mode + '-audio-keep-combination']['selector'])
            self.assertEqual(self.cases[mode + '-repeated-last']['mapping'], 'mp4')
        for name in ('conflict-remux-first', 'conflict-recode-first'):
            self.assertEqual(self.cases[name]['mode'], 'recode')
            self.assertEqual(self.cases[name]['warnings'],
                             ['--remux-video is ignored since --recode-video was given'])
        self.assertTrue(all(c['native_policy'] for c in self.fixture['source_only_cases']))

    def test_worker_rejects_network_process_and_file_mutation_attempts(self):
        # Execute the real worker with a tiny hostile postprocessor module. Its
        # attempted effects must hit the worker guards before any actual I/O.
        for effect in ["__import__('socket').create_connection(('example.invalid', 80))",
                       "__import__('subprocess').Popen(['never-run'])",
                       "__import__('os').replace('never-read', 'never-written')"]:
            with self.subTest(effect=effect), tempfile.TemporaryDirectory() as directory:
                package = Path(directory) / 'yt_dlp'
                (package / 'postprocessor').mkdir(parents=True)
                (package / '__init__.py').write_text(
                    "from types import SimpleNamespace\n"
                    "def parse_options(args):\n"
                    " return SimpleNamespace(ydl_opts={'postprocessors':[{'key':'FFmpegVideoRemuxer','preferedformat':'mp4'}]})\n",
                    encoding='utf-8')
                (package / 'postprocessor/ffmpeg.py').write_text(
                    "class FFmpegVideoRemuxerPP:\n"
                    " def __init__(self, **kwargs): pass\n"
                    " def run(self, info):\n  " + effect + "\n"
                    "FFmpegVideoConvertorPP=FFmpegVideoRemuxerPP\n", encoding='utf-8')
                fixture = {'cases': [self.fixture['cases'][0]], 'source_only_cases': [], 'invalid_cases': []}
                result = subprocess.run([sys.executable, '-B', '-c', UPSTREAM_WORKER, directory],
                                        input=json.dumps(fixture), capture_output=True,
                                        text=True, encoding='utf-8', timeout=10)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn('Network/process/file mutation forbidden', result.stderr)

    def run_main(self, revision, fixture_path=None):
        with tempfile.TemporaryDirectory() as directory:
            module = Path(directory) / 'yt_dlp/postprocessor/ffmpeg.py'
            module.parent.mkdir(parents=True)
            module.touch()
            outputs = [subprocess.CompletedProcess([], 0, revision, ''),
                       subprocess.CompletedProcess([], 0, json.dumps(self.result), '')]
            with mock.patch('sys.argv', ['compare_video.py', '--upstream', directory]), \
                    mock.patch('compare_video.subprocess.run', side_effect=outputs) as run, \
                    mock.patch('compare_video.FIXTURE', fixture_path or FIXTURE), \
                    contextlib.redirect_stdout(io.StringIO()) as stdout, \
                    contextlib.redirect_stderr(io.StringIO()) as stderr:
                status = main()
            return status, run, stdout.getvalue(), stderr.getvalue()

    def test_command_pins_source_and_passes_exact_fixture_to_worker(self):
        status, run, stdout, _ = self.run_main(PINNED_UPSTREAM)
        self.assertEqual(status, 0)
        self.assertEqual(run.call_count, 2)
        self.assertEqual(json.loads(run.call_args.kwargs['input']), self.fixture)
        self.assertIn('70 plans', stdout)
        self.assertEqual(run.call_args.kwargs['encoding'], 'utf-8')

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
