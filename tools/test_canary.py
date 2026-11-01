import copy
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import time
import unittest
from unittest import mock

import canary
from canary import (SITES, Runner, check_expectations, check_probe, classify_error,
                    compare_upstream, cookie_env_name, render_issue, render_markdown,
                    select_sites, summarize, validate_sites, worst)

TIER_1 = {'youtube', 'instagram', 'tiktok', 'twitter', 'facebook', 'reddit', 'vimeo',
          'twitch', 'bluesky', 'soundcloud', 'bandcamp'}


def load_sites():
    return json.loads(SITES.read_text(encoding='utf-8'))


class CanaryListTests(unittest.TestCase):
    def test_list_is_valid_and_covers_media_converter_sites(self):
        data = validate_sites(load_sites())
        self.assertEqual({s['site'] for s in data['sites']}, TIER_1)
        for site in data['sites']:
            self.assertTrue(any(e.get('download') for e in site['urls']), site['site'])
            for entry in site['urls']:
                self.assertTrue(entry['url'].startswith('https://'), entry['url'])

    def test_malformed_entries_are_rejected(self):
        base = load_sites()
        cases = [
            lambda d: d.update(schema_version=2),
            lambda d: d['sites'].append(copy.deepcopy(d['sites'][0])),
            lambda d: d['sites'][0].update(urls=[]),
            lambda d: d['sites'][0]['urls'][0]['expect'].pop('extractor_key'),
            lambda d: d['sites'][0]['urls'][0]['expect'].update(title_regex='x'),
            lambda d: d['sites'][0]['urls'][0]['expect'].update(duration=[5, 1]),
            lambda d: d['sites'][0]['urls'][0]['download'].pop('format'),
            lambda d: d['sites'][0]['urls'][0]['download'].update(streams=['subtitle']),
        ]
        for change in cases:
            data = copy.deepcopy(base)
            change(data)
            with self.assertRaises(ValueError):
                validate_sites(data)

    def test_site_filter(self):
        data = load_sites()
        self.assertEqual(len(select_sites(data, [''])), len(data['sites']))
        self.assertEqual([s['site'] for s in select_sites(data, [' Vimeo', 'youtube'])],
                         ['youtube', 'vimeo'])
        with self.assertRaisesRegex(ValueError, 'unknown canary sites: nope'):
            select_sites(data, ['nope'])

    def test_cookie_secret_names(self):
        self.assertEqual(cookie_env_name('reddit'), 'CANARY_COOKIES_REDDIT')
        self.assertEqual(cookie_env_name('x-twitter'), 'CANARY_COOKIES_X_TWITTER')


class ClassificationTests(unittest.TestCase):
    def test_blocked_errors_name_their_reason(self):
        cases = {
            "ERROR: [youtube] jNQXAC9IVRw: Sign in to confirm you’re not a bot. Use --cookies":
                'YouTube bot check',
            "ERROR: [youtube] x: Sign in to confirm you're not a bot": 'YouTube bot check',
            'WARNING: [youtube] x: ios client https formats require a GVS PO Token':
                'YouTube PO token required',
            'ERROR: [TikTok] 1: Your IP address is blocked from accessing this post': 'IP blocked',
            'ERROR: [Reddit] 6rrwyj: Account authentication is required. Use --cookies':
                'login wall',
            'ERROR: [Instagram] x: Instagram sent an empty media response. Check if':
                'login wall (empty media response)',
            'ERROR: [vimeo] 1: The web client only works when logged-in. Use --cookies':
                'login wall',
            'ERROR: Unable to download webpage: HTTP Error 429: Too Many Requests':
                'rate limited (HTTP 429)',
        }
        for text, reason in cases.items():
            self.assertEqual(classify_error(text), reason, text)

    def test_extraction_failures_stay_broken(self):
        for text in (
            'ERROR: https://www.instagram.com/p/x/: unsupported manifest: unsupported DASH fractional duration',
            'ERROR: [vimeo] 1: Unable to download webpage: HTTP Error 401: Unauthorized',
            'ERROR: unable to download video data: HTTP Error 403: Forbidden',
            'ERROR: [youtube] x: Unable to extract nsig function',
            '', None,
        ):
            self.assertIsNone(classify_error(text), text)

    def test_worst_status_wins(self):
        self.assertEqual(worst(['pass', 'blocked', 'broken']), 'broken')
        self.assertEqual(worst(['pass', 'diverged', 'blocked']), 'diverged')
        self.assertEqual(worst(['pass', 'skip', 'blocked']), 'blocked')
        self.assertEqual(worst(['pass', 'skip']), 'skip')
        self.assertEqual(worst(['pass']), 'pass')


class ComparisonTests(unittest.TestCase):
    INFO = {'id': 'a', 'extractor_key': 'Youtube', 'title': 'Me at the zoo', 'duration': 19,
            'uploader_id': '@jawed', 'live_status': 'not_live', 'formats': [{'format_id': '18'}]}

    def test_expectations(self):
        expect = {'id': 'a', 'extractor_key': 'Youtube', 'title': 'Me at the zoo',
                  'duration': [18, 20]}
        self.assertEqual(check_expectations(self.INFO, expect), [])
        self.assertEqual(check_expectations(self.INFO, {'id': 'a', 'title_regex': '^Me at'}), [])
        info = {**self.INFO, 'id': 'b', 'title': 'Other', 'duration': 25, 'formats': []}
        problems = check_expectations(info, {**expect, 'extractor_key': 'Vimeo'})
        self.assertEqual(problems, [
            "id: expected 'a', got 'b'",
            "extractor_key: expected 'Vimeo', got 'Youtube'",
            "title: expected 'Me at the zoo', got 'Other'",
            'duration: expected 18-20, got 25',
            'formats: none extracted',
        ])
        self.assertEqual(check_expectations({**self.INFO, 'title': None}, {'title_regex': 'x'}),
                         ['title: None does not match /x/'])
        self.assertEqual(check_expectations({**self.INFO, 'duration': None}, {'duration': [1, 2]}),
                         ['duration: expected 1-2, got None'])
        # A url-only result (no formats list) still counts as having media.
        self.assertEqual(check_expectations({'url': 'https://e/x.mp4'}, {}), [])
        self.assertEqual(check_expectations({}, {'formats': False}), [])

    def test_upstream_comparison(self):
        self.assertEqual(compare_upstream(self.INFO, {**self.INFO, 'duration': 19.9}), [])
        self.assertEqual(compare_upstream(self.INFO, {**self.INFO, 'duration': 21}),
                         ['duration: yt-dlp-rs 19, upstream 21'])
        self.assertEqual(compare_upstream(self.INFO, {**self.INFO, 'title': 'Me  at the zoo',
                                                      'formats': []}),
                         ["title: yt-dlp-rs 'Me at the zoo', upstream 'Me  at the zoo'",
                          'formats: yt-dlp-rs 1, upstream 0'])
        # Format counts may differ legitimately; only having formats is compared.
        self.assertEqual(compare_upstream(self.INFO, {**self.INFO, 'formats': [{}, {}]}), [])
        self.assertEqual(compare_upstream(self.INFO, {**self.INFO, 'title': 'x'}, ('id',)), [])

    def test_probe_streams(self):
        probe = {'streams': [{'codec_type': 'video'}, {'codec_type': 'audio'}]}
        self.assertEqual(check_probe(probe, ['video', 'audio']), [])
        self.assertEqual(check_probe({'streams': [{'codec_type': 'video'}]}, ['video', 'audio']),
                         ['ffprobe: no audio stream (found video)'])
        self.assertEqual(check_probe({}, []), ['ffprobe: no streams'])


def sample_report():
    sites = [
        {'site': 'youtube', 'name': 'YouTube', 'status': 'blocked', 'urls': [
            {'url': 'https://www.youtube.com/watch?v=a', 'status': 'blocked',
             'reason': 'YouTube bot check: ERROR: Sign in', 'attempts': 2}]},
        {'site': 'vimeo', 'name': 'Vimeo', 'status': 'broken', 'urls': [
            {'url': 'https://player.vimeo.com/video/1', 'status': 'broken',
             'reason': 'ERROR: [vimeo] 1: HTTP Error 401\nsecond line', 'attempts': 2,
             'upstream': {'status': 'pass'}},
            {'url': 'https://player.vimeo.com/video/2', 'status': 'pass', 'attempts': 2}]},
        {'site': 'bandcamp', 'name': 'Bandcamp', 'status': 'diverged', 'urls': [
            {'url': 'https://a.bandcamp.com/track/b', 'status': 'diverged',
             'reason': "title: yt-dlp-rs 'a|b', upstream 'a'", 'attempts': 2,
             'upstream': {'status': 'diverged'}}]},
        {'site': 'reddit', 'name': 'Reddit', 'status': 'skip', 'urls': [
            {'url': 'https://www.reddit.com/r/x', 'status': 'skip',
             'reason': 'needs cookies; CANARY_COOKIES_REDDIT is not set'}]},
        {'site': 'twitch', 'name': 'Twitch', 'status': 'pass', 'urls': [
            {'url': 'https://clips.twitch.tv/x', 'status': 'pass', 'attempts': 1,
             'upstream': {'status': 'blocked'}}]},
    ]
    return {'schema_version': 1, 'started': '2026-11-01T00:00:00Z', 'elapsed_seconds': 61.2,
            'binary_version': 'yt-dlp-rs 0.7.0', 'upstream_version': '2026.10.26',
            'sites': sites, 'summary': summarize(sites)}


class RenderingTests(unittest.TestCase):
    def test_summary_counts_sites(self):
        self.assertEqual(sample_report()['summary'],
                         {'broken': 1, 'diverged': 1, 'blocked': 1, 'skip': 1, 'pass': 1})

    def test_markdown(self):
        markdown = render_markdown(sample_report())
        self.assertIn('yt-dlp-rs `yt-dlp-rs 0.7.0`, upstream `2026.10.26`, '
                      '2026-11-01T00:00:00Z (61 s).', markdown)
        self.assertIn('❌ broken: 1 · ⚠️ diverged: 1 · 🚧 blocked: 1 · ⏭️ skip: 1 · ✅ pass: 1',
                      markdown)
        self.assertIn('| Vimeo | ❌ broken | https://player.vimeo.com/video/1 | '
                      'ERROR: [vimeo] 1: HTTP Error 401<br>second line (upstream works) |', markdown)
        self.assertIn('|  | ✅ pass | https://player.vimeo.com/video/2 | (passed on retry) |',
                      markdown)
        self.assertIn("| Bandcamp | ⚠️ diverged | https://a.bandcamp.com/track/b | "
                      "title: yt-dlp-rs 'a\\|b', upstream 'a' |", markdown)
        self.assertIn('| Twitch | ✅ pass | https://clips.twitch.tv/x | (upstream: blocked) |',
                      markdown)
        no_upstream = {**sample_report(), 'upstream_version': None}
        self.assertIn('upstream `not compared`', render_markdown(no_upstream))

    def test_issue_lists_only_broken_and_diverged_sites(self):
        issue = render_issue(sample_report(), 'https://github.com/o/r/actions/runs/1')
        self.assertEqual(issue['title'], 'Canary: Vimeo, Bandcamp failing')
        body = issue['body']
        self.assertIn('found 2 failing tier-1 sites: Vimeo, Bandcamp.', body)
        self.assertIn('Run: https://github.com/o/r/actions/runs/1', body)
        self.assertIn('### Vimeo (broken)\n\n- https://player.vimeo.com/video/1 (upstream works)\n\n  ```\n'
                      '  ERROR: [vimeo] 1: HTTP Error 401\n  second line\n  ```', body)
        self.assertNotIn('video/2', body)
        self.assertNotIn('YouTube', body)
        self.assertNotIn('Reddit', body)

    def test_no_issue_when_only_blocked_or_skipped(self):
        report = sample_report()
        report['sites'] = [s for s in report['sites'] if s['status'] not in ('broken', 'diverged')]
        self.assertIsNone(render_issue(report))


FAKE = r'''#!{python}
import json, os, sys
from pathlib import Path
args = sys.argv[1:]
state = Path(os.environ['FAKE_STATE'])
calls = json.loads(state.read_text()) if state.exists() else []
calls.append(args)
state.write_text(json.dumps(calls))
url = args[-1]
mode = os.environ['FAKE_MODE']
if '-j' in args:
    if mode == 'bot':
        sys.exit("ERROR: [youtube] a: Sign in to confirm you're not a bot")
    if mode == 'flaky' and len(calls) == 1:
        sys.exit('ERROR: [youtube] a: Unable to download webpage: timed out')
    if mode == 'broken':
        sys.exit('ERROR: [youtube] a: Unable to extract nsig function')
    print('[youtube] noise')
    title = os.environ.get('FAKE_UPSTREAM_TITLE', 'T') if 'upstream' in sys.argv[0] else 'T'
    print(json.dumps({{'id': 'a', 'extractor_key': 'Youtube', 'title': title, 'duration': 10,
                      'formats': [{{'format_id': '18'}}]}}))
else:
    path = Path(args[args.index('-P') + 1]) / 'a.mp4'
    path.write_bytes(b'' if mode == 'empty' else b'data')
    print(path)
'''


class RunnerTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.dir = Path(self.tmp.name)
        self.binary = self.dir / 'fake-yt-dlp-rs'
        self.binary.write_text(FAKE.format(python=sys.executable))
        self.binary.chmod(self.binary.stat().st_mode | stat.S_IEXEC)
        self.state = self.dir / 'calls.json'
        self.site = {'site': 'youtube', 'name': 'YouTube', 'urls': [{
            'url': 'https://www.youtube.com/watch?v=a',
            'expect': {'id': 'a', 'extractor_key': 'Youtube', 'duration': [9, 11]},
            'download': {'format': 'bestvideo+bestaudio/best', 'sections': '*0-4'},
        }]}

    def tearDown(self):
        self.tmp.cleanup()

    def check(self, mode, site=None, env=None, upstream=None):
        environ = {'FAKE_STATE': str(self.state), 'FAKE_MODE': mode, **(env or {})}
        with mock.patch.dict(os.environ, environ):
            runner = Runner(str(self.binary), upstream=upstream, ffmpeg=None, deno='/opt/deno',
                            log=lambda m: None)
            result = runner.check_site(site or self.site)
        calls = json.loads(self.state.read_text()) if self.state.exists() else []
        return result, calls

    def test_pass_runs_the_cli_like_media_converter(self):
        result, calls = self.check('ok')
        self.assertEqual(result['status'], 'pass')
        url = result['urls'][0]
        self.assertEqual((url['attempts'], url['download']['file'], url['download']['bytes']),
                         (1, 'a.mp4', 4))
        self.assertEqual(url['download']['probe'], 'skipped: no ffprobe')
        self.assertEqual(calls[0], ['--ignore-config', '--no-warnings', '--js-runtimes',
                                    'deno:/opt/deno', '-j', 'https://www.youtube.com/watch?v=a'])
        download = calls[1]
        for args in (['-f', 'bestvideo+bestaudio/best'], ['--print', 'after_move:filepath'],
                     ['--download-sections', '*0-4']):
            index = download.index(args[0])
            self.assertEqual(download[index:index + 2], args)

    def test_blocked_is_retried_once_and_reported_as_blocked(self):
        result, calls = self.check('bot')
        self.assertEqual(result['status'], 'blocked')
        self.assertEqual(result['urls'][0]['attempts'], 2)
        self.assertEqual(len(calls), 2)
        self.assertTrue(result['urls'][0]['reason'].startswith('YouTube bot check: ERROR:'))

    def test_flaky_failure_passes_on_retry(self):
        result, _ = self.check('flaky')
        self.assertEqual((result['status'], result['urls'][0]['attempts']), ('pass', 2))

    def test_broken_extraction_and_empty_download(self):
        result, _ = self.check('broken')
        self.assertEqual(result['status'], 'broken')
        self.assertEqual(result['urls'][0]['reason'],
                         'ERROR: [youtube] a: Unable to extract nsig function')
        self.state.unlink()
        result, _ = self.check('empty')
        self.assertEqual(result['urls'][0]['reason'], 'download: downloaded file is empty')

    def test_cookies_are_optional_secrets(self):
        site = {**self.site, 'requires_cookies': True}
        result, calls = self.check('ok', site)
        self.assertEqual(result['status'], 'skip')
        self.assertEqual(result['urls'][0]['reason'],
                         'needs cookies; CANARY_COOKIES_YOUTUBE is not set')
        self.assertEqual(calls, [])
        result, calls = self.check('ok', site, {'CANARY_COOKIES_YOUTUBE': '# Netscape HTTP Cookie File\n'})
        self.assertEqual(result['status'], 'pass')
        self.assertIn('--cookies', calls[0])
        self.assertFalse(Path(calls[0][calls[0].index('--cookies') + 1]).exists())

    def test_upstream_comparison_and_context(self):
        upstream = self.dir / 'fake-upstream'
        upstream.write_text(self.binary.read_text())
        upstream.chmod(self.binary.stat().st_mode)
        result, calls = self.check('ok', upstream=str(upstream))
        self.assertEqual(result['urls'][0]['upstream'], {'status': 'pass', 'diffs': []})
        self.assertEqual(calls[1], ['--js-runtimes', 'deno:/opt/deno', '--ignore-config',
                                    '--no-warnings', '-j', 'https://www.youtube.com/watch?v=a'])
        self.state.unlink()
        result, _ = self.check('ok', upstream=str(upstream), env={'FAKE_UPSTREAM_TITLE': 'U'})
        self.assertEqual((result['status'], result['urls'][0]['reason']),
                         ('diverged', "title: yt-dlp-rs 'T', upstream 'U'"))
        self.assertIn('download', result['urls'][0])
        self.state.unlink()
        result, _ = self.check('broken', upstream=str(upstream))
        self.assertEqual(result['urls'][0]['upstream']['status'], 'error')

    def test_site_timeout(self):
        with mock.patch.dict(os.environ, {'FAKE_STATE': str(self.state), 'FAKE_MODE': 'ok'}):
            runner = Runner(str(self.binary), site_timeout=0, log=lambda m: None)
            result = runner.check_site(self.site)
        self.assertEqual(result['status'], 'broken')
        self.assertEqual(result['urls'][0]['reason'], 'site timeout (0 s) reached')

    @unittest.skipUnless(os.name == 'posix', 'process groups')
    def test_command_timeout_kills_the_process_group(self):
        script = self.dir / 'sleeper'
        script.write_text(f'#!/bin/sh\nsleep 30 &\necho $! > {self.dir}/child\nwait\n')
        script.chmod(0o755)
        runner = Runner(str(self.binary), command_timeout=0.5, log=lambda m: None)
        self.assertIsNone(runner.run([str(script)], deadline=float('inf')))
        child = int((self.dir / 'child').read_text())
        with self.assertRaises(ProcessLookupError):
            for _ in range(50):
                os.kill(child, 0)
                time.sleep(0.05)

    def test_main_writes_reports_and_issue(self):
        sites = self.dir / 'sites.json'
        sites.write_text(json.dumps({'schema_version': 1, 'sites': [self.site]}))
        report, summary, issue = self.dir / 'r.json', self.dir / 's.md', self.dir / 'i.json'
        env = {'FAKE_STATE': str(self.state), 'FAKE_MODE': 'broken', 'GITHUB_STEP_SUMMARY': ''}
        with mock.patch.dict(os.environ, env), mock.patch('sys.stdout'), \
                mock.patch('sys.stderr'), mock.patch.object(canary, 'binary_version', return_value='yt-dlp-rs 0'):
            code = canary.main(['--binary', str(self.binary), '--list', str(sites),
                                '--report', str(report), '--summary', str(summary),
                                '--issue', str(issue), '--ffmpeg', '', '--deno', ''])
        self.assertEqual(code, 1)
        data = json.loads(report.read_text())
        self.assertEqual(data['summary']['broken'], 1)
        self.assertIn('❌ broken', summary.read_text())
        self.assertEqual(json.loads(issue.read_text())['title'], 'Canary: YouTube failing')


if __name__ == '__main__':
    unittest.main()
