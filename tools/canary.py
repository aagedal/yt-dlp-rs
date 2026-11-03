#!/usr/bin/env python3
"""Live canary run against the tier-1 sites (tools/canary_sites.json).

Development/monitoring only: this contacts live sites and is never part of the
required test suite. It runs the yt-dlp-rs CLI the way Aagedal Media Converter
does (`-j`, `-f bestvideo+bestaudio/best`, `--print after_move:filepath`,
`--ffmpeg-location`, `--js-runtimes deno:PATH`), checks each URL's key
metadata against the canary list, optionally compares it with the pinned
upstream yt-dlp, downloads a few seconds and probes the result with ffprobe.

Every URL ends in one status:

- pass: metadata matched, the download (if any) produced a probe-able file.
- broken: yt-dlp-rs failed or produced unexpected metadata or media.
- diverged: yt-dlp-rs matched the canary list but differs from upstream in a
  key field (only when upstream succeeded too).
- blocked: the error text matches BLOCKED_PATTERNS (bot checks, PO tokens,
  login walls, IP blocks, rate limits), or the pinned upstream fails on the
  URL too (the site or this network refuses both). Not a port bug; reported
  separately.
- skip: the entry needs cookies (CANARY_COOKIES_<SITE>) that are absent.

A site takes its URLs' worst status (broken > diverged > blocked > skip > pass).
A failed URL is retried once. The exit status is 1 when any site is broken or
diverged, else 0.
"""
import argparse
import datetime
import json
import os
from pathlib import Path
import re
import shutil
import signal
import subprocess
import sys
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
SITES = Path(__file__).resolve().parent / 'canary_sites.json'
MANIFEST = ROOT / 'upstream/manifest.json'

STATUS_ORDER = ('broken', 'diverged', 'blocked', 'skip', 'pass')
FAILING = ('broken', 'diverged')
ISSUE_LABEL = 'canary'

# Error text that means the site refused this client or network, not that
# extraction is broken. Matched case-insensitively against the error output;
# the first match names the reason. Keep these specific: a pattern that
# matches a real extraction failure hides breakage. HTTP 403 is deliberately
# absent: on YouTube media it usually means a wrong challenge solution.
BLOCKED_PATTERNS = (
    (r"Sign in to confirm you(?:'|’)re not a bot", 'YouTube bot check'),
    (r'Sign in to confirm your age', 'YouTube age gate'),
    (r'\bPO[ -]?Token\b', 'YouTube PO token required'),
    (r'This content isn(?:\'|’)t available, try again later', 'YouTube rate limit'),
    (r'HTTP Error 429', 'rate limited (HTTP 429)'),
    (r'Too Many Requests', 'rate limited (HTTP 429)'),
    (r'Your IP address is blocked', 'IP blocked'),
    (r'not available in your (?:country|region)', 'geo blocked'),
    (r'geo[- ]?restrict', 'geo blocked'),
    (r'empty media response', 'login wall (empty media response)'),
    (r'login required|log in to|logged[- ]in|requires? (?:a )?login', 'login wall'),
    (r'Account authentication is required', 'login wall'),
    (r'You need to log in', 'login wall'),
    (r'rate-?limit', 'rate limited'),
)

# Fields compared with upstream when both succeed. Durations may differ by
# DURATION_TOLERANCE seconds (container vs. API rounding).
UPSTREAM_FIELDS = ('id', 'extractor_key', 'title', 'duration', 'uploader_id', 'live_status')
DURATION_TOLERANCE = 1.0


def classify_error(text):
    """Return a blocked reason for error text, or None when it looks broken."""
    for pattern, reason in BLOCKED_PATTERNS:
        if re.search(pattern, text or '', re.IGNORECASE):
            return reason
    return None


def last_json_line(stdout):
    for line in reversed((stdout or '').splitlines()):
        line = line.strip()
        if line.startswith('{'):
            return json.loads(line)
    raise ValueError('no JSON object in output')


def error_text(stderr, stdout=''):
    """The ERROR lines of a failed run, else its last output line."""
    lines = [l.strip() for l in (stderr or '').splitlines() if l.strip()]
    errors = [l for l in lines if l.startswith('ERROR:')]
    if errors:
        return '\n'.join(errors)[:2000]
    if lines:
        return lines[-1][:2000]
    out = [l.strip() for l in (stdout or '').splitlines() if l.strip()]
    return (out[-1] if out else 'no output')[:2000]


def check_expectations(info, expect):
    """List human-readable mismatches between an info dict and an entry's expect."""
    problems = []
    for key in ('id', 'extractor_key', 'title', 'live_status'):
        if key in expect and info.get(key) != expect[key]:
            problems.append(f'{key}: expected {expect[key]!r}, got {info.get(key)!r}')
    if 'title_regex' in expect:
        title = info.get('title')
        if not isinstance(title, str) or not re.search(expect['title_regex'], title):
            problems.append(f'title: {title!r} does not match /{expect["title_regex"]}/')
    if 'duration' in expect:
        low, high = expect['duration']
        duration = info.get('duration')
        if not isinstance(duration, (int, float)) or isinstance(duration, bool) \
                or not low <= duration <= high:
            problems.append(f'duration: expected {low}-{high}, got {duration!r}')
    if expect.get('formats', True):
        formats = info.get('formats')
        if not formats and not info.get('url'):
            problems.append('formats: none extracted')
    return problems


def compare_upstream(ours, theirs, fields=UPSTREAM_FIELDS):
    """List key-field differences from upstream's info dict."""
    diffs = []
    for key in fields:
        a, b = ours.get(key), theirs.get(key)
        if key == 'duration' and isinstance(a, (int, float)) and isinstance(b, (int, float)):
            if abs(a - b) <= DURATION_TOLERANCE:
                continue
        elif a == b:
            continue
        diffs.append(f'{key}: yt-dlp-rs {a!r}, upstream {b!r}')
    if bool(ours.get('formats')) != bool(theirs.get('formats')):
        diffs.append(f'formats: yt-dlp-rs {len(ours.get("formats") or [])}, '
                     f'upstream {len(theirs.get("formats") or [])}')
    return diffs


def check_probe(probe, streams):
    """List problems with ffprobe JSON output given required stream types."""
    found = {s.get('codec_type') for s in probe.get('streams') or []}
    if not found:
        return ['ffprobe: no streams']
    return [f'ffprobe: no {kind} stream (found {", ".join(sorted(t for t in found if t))})'
            for kind in streams if kind not in found]


def worst(statuses):
    statuses = list(statuses)
    for status in STATUS_ORDER:
        if status in statuses:
            return status
    return 'skip'


def summarize(sites):
    counts = {status: 0 for status in STATUS_ORDER}
    for site in sites:
        counts[site['status']] += 1
    return counts


def cookie_env_name(site):
    return 'CANARY_COOKIES_' + re.sub(r'[^A-Z0-9]', '_', site.upper())


def validate_sites(data):
    """Raise ValueError for a malformed canary list."""
    if data.get('schema_version') != 1:
        raise ValueError('canary list schema_version must be 1')
    seen = set()
    for site in data['sites']:
        for key in ('site', 'name', 'urls'):
            if key not in site:
                raise ValueError(f'canary site entry lacks {key!r}: {site}')
        if site['site'] in seen:
            raise ValueError(f'duplicate canary site {site["site"]!r}')
        seen.add(site['site'])
        if not site['urls']:
            raise ValueError(f'canary site {site["site"]!r} has no URLs')
        for entry in site['urls']:
            expect = entry.get('expect')
            if not isinstance(expect, dict) or 'id' not in expect or 'extractor_key' not in expect:
                raise ValueError(f'{entry.get("url")}: expect needs at least id and extractor_key')
            if 'title' in expect and 'title_regex' in expect:
                raise ValueError(f'{entry["url"]}: use title or title_regex, not both')
            if 'title_regex' in expect:
                re.compile(expect['title_regex'])
            if 'duration' in expect:
                low, high = expect['duration']
                if not low <= high:
                    raise ValueError(f'{entry["url"]}: duration range is reversed')
            download = entry.get('download')
            if download is not None:
                if not isinstance(download, dict) or 'format' not in download:
                    raise ValueError(f'{entry["url"]}: download needs a format')
                unknown = set(download.get('streams', [])) - {'video', 'audio'}
                if unknown:
                    raise ValueError(f'{entry["url"]}: unknown stream types {sorted(unknown)}')
    return data


def select_sites(data, names):
    wanted = {n.strip().lower() for n in names if n.strip()}
    if not wanted:
        return list(data['sites'])
    known = {s['site'] for s in data['sites']}
    unknown = wanted - known
    if unknown:
        raise ValueError(f'unknown canary sites: {", ".join(sorted(unknown))} '
                         f'(known: {", ".join(sorted(known))})')
    return [s for s in data['sites'] if s['site'] in wanted]


def md_escape(text):
    return str(text).replace('|', '\\|').replace('\n', '<br>')


STATUS_LABEL = {
    'pass': '✅ pass', 'broken': '❌ broken', 'diverged': '⚠️ diverged',
    'blocked': '🚧 blocked', 'skip': '⏭️ skip',
}


def upstream_note(url):
    status = url.get('upstream', {}).get('status')
    if status in ('blocked', 'error'):
        return f' (upstream: {status})'
    if status == 'pass' and url['status'] == 'broken':
        return ' (upstream works)'
    return ''


def render_markdown(report):
    counts = report['summary']
    lines = [
        '## yt-dlp-rs live canaries',
        '',
        f'yt-dlp-rs `{report["binary_version"]}`, upstream '
        f'`{report.get("upstream_version") or "not compared"}`, '
        f'{report["started"]} ({report["elapsed_seconds"]:.0f} s).',
        '',
        ' · '.join(f'{STATUS_LABEL[s]}: {counts[s]}' for s in STATUS_ORDER),
        '',
        '| Site | Status | URL | Details |',
        '| --- | --- | --- | --- |',
    ]
    for site in report['sites']:
        for index, url in enumerate(site['urls']):
            details = url.get('reason') or ''
            details += upstream_note(url)
            if url.get('attempts', 1) > 1 and url['status'] == 'pass':
                details += ' (passed on retry)'
            lines.append('| {} | {} | {} | {} |'.format(
                md_escape(site['name']) if index == 0 else '',
                STATUS_LABEL[url['status']], md_escape(url['url']), md_escape(details.strip())))
    lines += ['', 'Blocked means the site refused this network or needs login '
              '(see `BLOCKED_PATTERNS` in `tools/canary.py`); it does not open an issue.']
    return '\n'.join(lines) + '\n'


def render_issue(report, run_url=None):
    """Title and body of the canary issue, or None when nothing is failing."""
    failing = [s for s in report['sites'] if s['status'] in FAILING]
    if not failing:
        return None
    names = ', '.join(s['name'] for s in failing)
    lines = [
        f'The scheduled canary run found {len(failing)} failing tier-1 '
        f'site{"s" if len(failing) != 1 else ""}: {names}.',
        '',
    ]
    if run_url:
        lines += [f'Run: {run_url}', '']
    for site in failing:
        lines.append(f'### {site["name"]} ({site["status"]})')
        lines.append('')
        for url in site['urls']:
            if url['status'] in FAILING:
                lines.append(f'- {url["url"]}{upstream_note(url)}')
                lines.append('')
                lines.append('  ```')
                lines += ['  ' + l for l in (url.get('reason') or '').splitlines()]
                lines.append('  ```')
        lines.append('')
    lines.append('This issue is updated by `.github/workflows/canary.yml` and closed '
                 'automatically when no site is broken or diverged.')
    return {'title': f'Canary: {names} failing', 'body': '\n'.join(lines) + '\n'}


class Runner:
    def __init__(self, binary, upstream=None, ffmpeg=None, deno=None, site_timeout=600,
                 command_timeout=180, retries=1, log=None):
        self.binary = binary
        self.upstream = upstream
        self.ffmpeg = ffmpeg
        self.ffprobe = None
        if ffmpeg:
            candidate = Path(ffmpeg).with_name(Path(ffmpeg).name.replace('ffmpeg', 'ffprobe'))
            self.ffprobe = str(candidate) if candidate.exists() else shutil.which('ffprobe')
        self.deno = deno
        self.site_timeout = site_timeout
        self.command_timeout = command_timeout
        self.retries = retries
        self.log = log or (lambda message: print(message, file=sys.stderr, flush=True))

    def common_args(self, cookies):
        args = ['--ignore-config', '--no-warnings']
        if self.ffmpeg:
            args += ['--ffmpeg-location', self.ffmpeg]
        if self.deno:
            args += ['--js-runtimes', f'deno:{self.deno}']
        if cookies:
            args += ['--cookies', cookies]
        return args

    def run(self, command, deadline, cwd=None, env=None):
        timeout = min(self.command_timeout, max(1.0, deadline - time.monotonic()))
        # A new session, so a timeout also stops the FFmpeg children.
        process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                   text=True, encoding='utf-8', errors='replace', cwd=cwd,
                                   env=env, start_new_session=os.name == 'posix')
        try:
            stdout, stderr = process.communicate(timeout=timeout)
        except subprocess.TimeoutExpired:
            if os.name == 'posix':
                os.killpg(process.pid, signal.SIGKILL)
            else:
                process.kill()
            process.communicate()
            return None
        return subprocess.CompletedProcess(command, process.returncode, stdout, stderr)

    def upstream_command(self):
        path = Path(self.upstream)
        if path.is_dir():
            return [sys.executable, '-m', 'yt_dlp'], {**os.environ, 'PYTHONPATH': str(path)}
        return [self.upstream], None

    def extract(self, url, cookies, deadline):
        process = self.run([self.binary, *self.common_args(cookies), '-j', url], deadline)
        if process is None:
            return None, 'timed out extracting metadata'
        if process.returncode != 0:
            return None, error_text(process.stderr, process.stdout)
        try:
            return last_json_line(process.stdout), None
        except ValueError as error:
            return None, f'{error}: {error_text(process.stderr, process.stdout)}'

    def extract_upstream(self, url, cookies, deadline):
        command, env = self.upstream_command()
        args = ['--ignore-config', '--no-warnings', '-j', url]
        if cookies:
            args[:0] = ['--cookies', cookies]
        if self.deno:
            args[:0] = ['--js-runtimes', f'deno:{self.deno}']
        process = self.run([*command, *args], deadline, env=env)
        if process is None:
            return {'status': 'error', 'reason': 'timed out'}, None
        if process.returncode != 0:
            text = error_text(process.stderr, process.stdout)
            return {'status': 'blocked' if classify_error(text) else 'error', 'reason': text}, None
        try:
            return {'status': 'pass'}, last_json_line(process.stdout)
        except ValueError as error:
            return {'status': 'error', 'reason': str(error)}, None

    def download(self, url, spec, cookies, deadline):
        with tempfile.TemporaryDirectory(prefix='canary-') as directory:
            args = [self.binary, *self.common_args(cookies), '--newline', '--no-progress',
                    '-f', spec['format'], '-P', directory, '-o', '%(id)s.%(ext)s',
                    '--print', 'after_move:filepath', '--no-simulate']
            if 'sections' in spec:
                args += ['--download-sections', spec['sections']]
            process = self.run([*args, url], deadline)
            if process is None:
                return {'status': 'broken', 'reason': 'timed out downloading'}
            if process.returncode != 0:
                return {'status': 'broken', 'reason': error_text(process.stderr, process.stdout)}
            paths = [Path(l.strip()) for l in process.stdout.splitlines() if l.strip()]
            if not paths or not paths[-1].is_file():
                return {'status': 'broken',
                        'reason': f'no output file reported (stdout: {process.stdout.strip()[:300]!r})'}
            path = paths[-1]
            result = {'status': 'pass', 'file': path.name, 'bytes': path.stat().st_size}
            if result['bytes'] == 0:
                return {**result, 'status': 'broken', 'reason': 'downloaded file is empty'}
            if not self.ffprobe:
                result['probe'] = 'skipped: no ffprobe'
                return result
            probe = self.run([self.ffprobe, '-v', 'error', '-show_entries',
                              'stream=codec_type,codec_name:format=duration', '-of', 'json',
                              str(path)], deadline)
            if probe is None or probe.returncode != 0:
                return {**result, 'status': 'broken',
                        'reason': 'ffprobe failed: ' + (probe.stderr.strip() if probe else 'timed out')}
            data = json.loads(probe.stdout or '{}')
            result['streams'] = [s.get('codec_name') for s in data.get('streams') or []]
            problems = check_probe(data, spec.get('streams', []))
            if problems:
                return {**result, 'status': 'broken', 'reason': '; '.join(problems)}
            return result

    def check_url(self, entry, cookies, deadline):
        url = entry['url']
        result = {'url': url}
        info, error = self.extract(url, cookies, deadline)
        if info is None:
            blocked = classify_error(error)
            result.update(status='blocked' if blocked else 'broken',
                          reason=f'{blocked}: {error}' if blocked else error)
            if self.upstream and not blocked:
                # Does upstream fail too? Then the site or this network refuses
                # both (a timeout says nothing either way).
                upstream = self.extract_upstream(url, cookies, deadline)[0]
                result['upstream'] = upstream
                if upstream['status'] == 'error' and upstream['reason'] != 'timed out':
                    result.update(status='blocked', reason=f'upstream fails too: {error}')
            return result
        result['metadata'] = {k: info.get(k) for k in (*UPSTREAM_FIELDS, 'ext', 'format_id')}
        result['metadata']['format_count'] = len(info.get('formats') or [])
        problems = check_expectations(info, entry['expect'])
        if problems:
            result.update(status='broken', reason='; '.join(problems))
            return result
        result['status'] = 'pass'
        if self.upstream:
            upstream, theirs = self.extract_upstream(url, cookies, deadline)
            if theirs is not None:
                diffs = compare_upstream(info, theirs, entry.get('compare', UPSTREAM_FIELDS))
                upstream = {'status': 'diverged' if diffs else 'pass', 'diffs': diffs}
                if diffs:
                    result.update(status='diverged', reason='; '.join(diffs))
            result['upstream'] = upstream
        if entry.get('download'):
            download = self.download(url, entry['download'], cookies, deadline)
            result['download'] = download
            if download['status'] != 'pass':
                blocked = classify_error(download['reason'])
                result.update(status='blocked' if blocked else 'broken',
                              reason='download: ' + (f'{blocked}: ' if blocked else '')
                              + download['reason'])
        return result

    def check_site(self, site):
        started = time.monotonic()
        deadline = started + self.site_timeout
        env_name = cookie_env_name(site['site'])
        cookies_text = os.environ.get(env_name)
        result = {'site': site['site'], 'name': site['name'], 'urls': []}
        if site.get('requires_cookies') and not cookies_text:
            for entry in site['urls']:
                result['urls'].append({'url': entry['url'], 'status': 'skip',
                                       'reason': f'needs cookies; {env_name} is not set'})
            result['status'] = 'skip'
            return result
        with tempfile.TemporaryDirectory(prefix='canary-cookies-') as directory:
            cookies = None
            if cookies_text:
                cookies = str(Path(directory) / 'cookies.txt')
                Path(cookies).write_text(cookies_text, encoding='utf-8')
            for entry in site['urls']:
                attempts = 0
                while True:
                    attempts += 1
                    if time.monotonic() >= deadline:
                        url = {'url': entry['url'], 'status': 'broken',
                               'reason': f'site timeout ({self.site_timeout} s) reached'}
                        break
                    url = self.check_url(entry, cookies, deadline)
                    self.log(f'[canary] {site["site"]}: {entry["url"]} -> {url["status"]}'
                             + (f' ({url.get("reason")})' if url['status'] != 'pass' else ''))
                    if url['status'] == 'pass' or attempts > self.retries:
                        break
                url['attempts'] = attempts
                result['urls'].append(url)
        result['status'] = worst(u['status'] for u in result['urls'])
        result['seconds'] = round(time.monotonic() - started, 1)
        return result


def binary_version(command):
    try:
        out = subprocess.run([*command, '--version'], capture_output=True, text=True, timeout=60)
        return out.stdout.strip().splitlines()[0] if out.returncode == 0 and out.stdout.strip() else None
    except (OSError, subprocess.TimeoutExpired):
        return None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--binary', default=str(ROOT / 'target/release/yt-dlp-rs'),
                        help='yt-dlp-rs executable (default: target/release/yt-dlp-rs)')
    parser.add_argument('--upstream', help='upstream yt-dlp executable or source checkout '
                        '(for example .upstream/yt-dlp); it must be the pinned version')
    parser.add_argument('--sites', default='', help='comma-separated site keys (default: all)')
    parser.add_argument('--list', default=str(SITES), help='canary list JSON')
    parser.add_argument('--ffmpeg', default=shutil.which('ffmpeg'), help='ffmpeg executable')
    parser.add_argument('--deno', default=shutil.which('deno'), help='deno executable')
    parser.add_argument('--site-timeout', type=float, default=600,
                        help='seconds per site, all URLs and retries included')
    parser.add_argument('--command-timeout', type=float, default=180,
                        help='seconds per yt-dlp-rs/upstream/ffprobe invocation')
    parser.add_argument('--retries', type=int, default=1)
    parser.add_argument('--report', default='canary-report.json', help='JSON report path')
    parser.add_argument('--summary', help='markdown summary path (also appended to '
                        '$GITHUB_STEP_SUMMARY when set)')
    parser.add_argument('--issue', help='write the issue title/body JSON here when failing')
    parser.add_argument('--run-url', help='link to this run, for the issue body')
    args = parser.parse_args(argv)

    try:
        data = validate_sites(json.loads(Path(args.list).read_text(encoding='utf-8')))
        sites = select_sites(data, args.sites.split(','))
    except (OSError, ValueError, KeyError) as error:
        print(f'canary: {error}', file=sys.stderr)
        return 2
    version = binary_version([args.binary])
    if version is None:
        print(f'canary: cannot run {args.binary} --version', file=sys.stderr)
        return 2
    upstream_version = None
    if args.upstream:
        runner_cmd, env = Runner(args.binary, upstream=args.upstream).upstream_command()
        try:
            out = subprocess.run([*runner_cmd, '--version'], capture_output=True, text=True,
                                 timeout=60, env=env)
            upstream_version = out.stdout.strip() if out.returncode == 0 else None
        except (OSError, subprocess.TimeoutExpired):
            upstream_version = None
        pinned = json.loads(MANIFEST.read_text(encoding='utf-8'))['upstream_version']
        if upstream_version != pinned:
            print(f'canary: upstream {args.upstream} is {upstream_version!r}, not the pinned '
                  f'{pinned}; not comparing with upstream', file=sys.stderr)
            args.upstream = upstream_version = None

    runner = Runner(args.binary, upstream=args.upstream, ffmpeg=args.ffmpeg, deno=args.deno,
                    site_timeout=args.site_timeout, command_timeout=args.command_timeout,
                    retries=args.retries)
    started = time.monotonic()
    report = {
        'schema_version': 1,
        'started': datetime.datetime.now(datetime.timezone.utc).strftime('%Y-%m-%dT%H:%M:%SZ'),
        'binary_version': version,
        'upstream_version': upstream_version,
        'sites': [runner.check_site(site) for site in sites],
    }
    report['elapsed_seconds'] = round(time.monotonic() - started, 1)
    report['summary'] = summarize(report['sites'])
    Path(args.report).write_text(json.dumps(report, indent=2, ensure_ascii=False) + '\n',
                                 encoding='utf-8')
    markdown = render_markdown(report)
    if args.summary:
        Path(args.summary).write_text(markdown, encoding='utf-8')
    if os.environ.get('GITHUB_STEP_SUMMARY'):
        with open(os.environ['GITHUB_STEP_SUMMARY'], 'a', encoding='utf-8') as summary:
            summary.write(markdown)
    print(markdown)
    issue = render_issue(report, args.run_url)
    if args.issue and issue:
        Path(args.issue).write_text(json.dumps(issue, ensure_ascii=False) + '\n',
                                    encoding='utf-8')
    return 1 if issue else 0


if __name__ == '__main__':
    sys.exit(main())
