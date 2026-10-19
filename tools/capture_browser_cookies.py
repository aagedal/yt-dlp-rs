#!/usr/bin/env python3
"""Build browser cookie database fixtures and record upstream's extraction.

Development/reference tooling only; Python is never a runtime fallback. Each
fixture profile is written with Python's sqlite3 (several b-tree pages, an
overflowing value, rows from before an ALTER TABLE, containers), then the
pinned upstream checkout's `extract_cookies_from_browser` reads it. The
cookies and logger messages it produced are saved as the native expectation.
No network access is involved.

usage: capture_browser_cookies.py UPSTREAM_CHECKOUT FIXTURE_DIR
"""
import json
import os
import shutil
import sqlite3
import sys


def firefox_profile(directory, version, rows, legacy_rows=()):
    os.makedirs(directory, exist_ok=True)
    path = os.path.join(directory, 'cookies.sqlite')
    if os.path.exists(path):
        os.remove(path)
    connection = sqlite3.connect(path)
    # Small pages force interior b-tree pages and overflow chains.
    connection.execute('PRAGMA page_size=512')
    connection.execute(
        'CREATE TABLE moz_cookies (id INTEGER PRIMARY KEY, originAttributes TEXT NOT NULL '
        "DEFAULT '', name TEXT, value TEXT, host TEXT, path TEXT, expiry INTEGER, "
        'lastAccessed INTEGER, creationTime INTEGER, isSecure INTEGER, isHttpOnly INTEGER, '
        'CONSTRAINT moz_uniqueid UNIQUE (name, host, path, originAttributes))')
    insert = ('INSERT INTO moz_cookies (originAttributes, name, value, host, path, expiry, '
              'lastAccessed, creationTime, isSecure, isHttpOnly) VALUES (?, ?, ?, ?, ?, ?, 0, 0, ?, 0)')
    for row in legacy_rows:
        connection.execute(insert, row)
    connection.execute('ALTER TABLE moz_cookies ADD COLUMN sameSite INTEGER DEFAULT 0')
    for row in rows:
        connection.execute(insert, row)
    connection.execute(f'PRAGMA user_version={version}')
    connection.commit()
    connection.close()
    with open(os.path.join(directory, 'containers.json'), 'w') as containers:
        json.dump({'version': 5, 'identities': [
            {'userContextId': 2, 'public': True, 'l10nID': 'userContextWork.label'},
            {'userContextId': 7, 'public': True, 'name': 'Shopping'},
        ]}, containers)


def main():
    checkout, fixtures = sys.argv[1:]
    sys.path.insert(0, checkout)
    from yt_dlp.cookies import extract_cookies_from_browser

    shutil.rmtree(fixtures, ignore_errors=True)
    many = [('', f'n{index:03d}', f'v{index}', 'example.test', '/', 4102444800000, 0)
            for index in range(120)]
    firefox_profile(
        os.path.join(fixtures, 'firefox', 'v16'), 16,
        [*many,
         ('', 'long', 'x' * 3000, '.example.test', '/deep', 4102444800000, 1),
         ('userContextId=2', 'work', 'w', 'example.test', '/', 4102444800000, 0),
         ('userContextId=2&privateBrowsingId=0', 'work2', 'w2', 'example.test', '/', 4102444800000, 0),
         ('userContextId=7', 'shop', 's', 'shop.test', '/', 4102444800000, 1)],
        legacy_rows=[('', 'legacy', 'old', 'legacy.test', '/', 4102444800000, 0)])
    firefox_profile(
        os.path.join(fixtures, 'firefox', 'v12'), 12,
        [('', 'seconds', 'yes', 'example.test', '/', 4102444800, 0)])

    class Logger:
        def __init__(self):
            self.messages = []

        def info(self, message):
            self.messages.append(['info', message])

        def debug(self, message):
            self.messages.append(['debug', message])

        def warning(self, message, only_once=False):
            self.messages.append(['warning', message])

        def error(self, message):
            self.messages.append(['error', message])

    cases = []
    for profile, container in [('v16', None), ('v16', 'userContextWork.label'), ('v16', 'Shopping'),
                               ('v16', 'none'), ('v12', None)]:
        logger = Logger()
        root = os.path.join(fixtures, 'firefox', profile)
        jar = extract_cookies_from_browser('firefox', root, logger, container=container)
        cookies = sorted(
            [c.domain, c.path, c.name, c.value, c.expires, bool(c.secure)] for c in jar)
        messages = [[kind, text.replace(os.path.abspath(fixtures), 'FIXTURES')]
                    for kind, text in logger.messages]
        cases.append({'profile': profile, 'container': container,
                      'cookies': cookies, 'messages': messages})
    with open(os.path.join(fixtures, 'expected.json'), 'w') as output:
        json.dump(cases, output, indent=1)
        output.write('\n')


if __name__ == '__main__':
    main()
