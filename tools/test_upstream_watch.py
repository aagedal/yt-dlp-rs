import datetime
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

from upstream_watch import latest_release, main, site_file


class UpstreamWatchTest(unittest.TestCase):
    def test_release_tags_sort_by_date_and_revision(self):
        self.assertEqual(latest_release(['2026.10.21', '2026.10.26', '2026.09.30.1']), '2026.10.26')
        self.assertEqual(latest_release(['2026.10.26', '2026.10.26.1', 'nightly']), '2026.10.26.1')
        self.assertIsNone(latest_release(['nightly', 'v1.0']))

    def test_shared_extractor_modules_are_core(self):
        self.assertTrue(site_file('yt_dlp/extractor/vimeo.py'))
        self.assertTrue(site_file('yt_dlp/extractor/youtube/_video.py'))
        self.assertFalse(site_file('yt_dlp/extractor/common.py'))
        self.assertFalse(site_file('yt_dlp/YoutubeDL.py'))

    def test_pending_overdue_and_current_releases(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            checkout = root / 'upstream'
            checkout.mkdir()

            def git(*args, date='2026-10-20T12:00:00+00:00'):
                env = {**os.environ, 'GIT_COMMITTER_DATE': date, 'GIT_AUTHOR_DATE': date}
                return subprocess.check_output(['git', '-C', str(checkout), *args],
                                               stderr=subprocess.DEVNULL, env=env).decode().strip()

            def commit(message, path, date):
                (checkout / path).parent.mkdir(parents=True, exist_ok=True)
                (checkout / path).write_text(message + '\n')
                git('add', '-A')
                git('commit', '-m', message, date=date)
                return git('rev-parse', 'HEAD')

            git('init')
            git('config', 'user.name', 'Fixture')
            git('config', 'user.email', 'fixture@example.invalid')
            baseline = commit('baseline', 'yt_dlp/YoutubeDL.py', '2026-10-20T12:00:00+00:00')
            git('tag', '2026.10.20')
            commit('[ie/vimeo] Fix', 'yt_dlp/extractor/vimeo.py', '2026-10-25T12:00:00+00:00')
            commit('[ie/newsite] Add', 'yt_dlp/extractor/newsite.py', '2026-10-25T13:00:00+00:00')
            commit('[core] Fix', 'yt_dlp/YoutubeDL.py', '2026-10-25T14:00:00+00:00')
            release = commit('Release 2026.10.26', 'yt_dlp/version.py', '2026-10-26T12:00:00+00:00')
            git('tag', '2026.10.26')
            manifest = root / 'manifest.json'
            manifest.write_text(json.dumps({
                'schema_version': 1,
                'repository': 'https://github.com/yt-dlp/yt-dlp.git',
                'baseline': baseline, 'upstream_version': '2026.10.20',
                'components': [
                    {'name': 'core', 'status': 'partial', 'upstream': ['yt_dlp/YoutubeDL.py'], 'rust': []},
                    {'name': 'vimeo', 'status': 'partial', 'upstream': ['yt_dlp/extractor/vimeo.py'], 'rust': []},
                ],
            }))
            issue, report = root / 'issue.json', root / 'report.json'

            def run(today):
                return main([str(checkout), '--manifest', str(manifest), '--issue', str(issue),
                             '--report', str(report), '--today', today, '--run-url', 'https://run.invalid/1'])

            self.assertEqual(run('2026-11-02'), 0)
            result = json.loads(report.read_text())
            self.assertEqual(result['status'], 'pending')
            self.assertEqual((result['target'], result['due']), (release, '2026-11-02'))
            rendered = json.loads(issue.read_text())
            self.assertEqual(rendered['title'], 'Upstream 2026.10.26: baseline advance due 2026-11-02')
            body = rendered['body']
            self.assertIn('Run: https://run.invalid/1', body)
            self.assertIn('4 commits, 4 changed files, 1 core and 1 extractor components affected, '
                          '2 unmapped files.', body)
            self.assertIn('- `vimeo` (partial): `yt_dlp/extractor/vimeo.py`', body)
            self.assertIn('1 under `yt_dlp/extractor/`', body)
            # New extractor modules come first among the unmapped files.
            self.assertLess(body.index('newsite.py'), body.index('version.py'))
            self.assertIn('upstream/notes/upstream-2026.10.26.md', body)

            self.assertEqual(run('2026-11-03'), 1)
            self.assertEqual(json.loads(issue.read_text())['title'],
                             'Upstream 2026.10.26: baseline advance overdue')

            # A baseline at or past the release is caught up and drops the issue.
            for caught_up in (release, commit('[ci] Later', '.github/ci.yml', '2026-10-29T12:00:00+00:00')):
                data = json.loads(manifest.read_text())
                data['baseline'] = caught_up
                manifest.write_text(json.dumps(data))
                self.assertEqual(run('2026-12-01'), 0)
                self.assertEqual(json.loads(report.read_text())['status'], 'current')
                self.assertFalse(issue.exists())

            # A baseline outside the release's history is an error, not a report.
            git('checkout', '-q', '--orphan', 'other')
            data['baseline'] = commit('unrelated', 'x.py', '2026-10-30T12:00:00+00:00')
            manifest.write_text(json.dumps(data))
            self.assertEqual(run('2026-12-01'), 2)


if __name__ == '__main__':
    unittest.main()
