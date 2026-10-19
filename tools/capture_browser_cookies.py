#!/usr/bin/env python3
"""Build browser cookie database fixtures and record upstream's extraction.

Development/reference tooling only; Python is never a runtime fallback. Each
fixture profile is written with Python's sqlite3 (several b-tree pages, an
overflowing value, rows from before an ALTER TABLE, containers), then the
pinned upstream checkout's `extract_cookies_from_browser` reads it. The
cookies and logger messages it produced are saved as the native expectation.
No network access is involved.

Chromium profiles hold macOS-keychain (`v10`, 1003 PBKDF2 iterations) values
for the password in `chrome/password.txt`; upstream reads it through a fake
`security` command, as the native tests do.

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


CHROME_PASSWORD = b'fixture keychain password'


def chrome_profile(directory, meta_version, rows, secure_column='is_secure'):
    from yt_dlp.aes import aes_cbc_encrypt_bytes
    import hashlib
    os.makedirs(directory, exist_ok=True)
    path = os.path.join(directory, 'Cookies')
    if os.path.exists(path):
        os.remove(path)
    key = hashlib.pbkdf2_hmac('sha1', CHROME_PASSWORD, b'saltysalt', 1003, 16)
    connection = sqlite3.connect(path)
    connection.execute('CREATE TABLE meta(key LONGVARCHAR NOT NULL UNIQUE PRIMARY KEY, value LONGVARCHAR)')
    connection.execute("INSERT INTO meta VALUES ('version', ?)", (str(meta_version),))
    connection.execute(
        'CREATE TABLE cookies(creation_utc INTEGER NOT NULL, host_key TEXT NOT NULL, name TEXT NOT NULL, '
        'value TEXT NOT NULL, path TEXT NOT NULL, expires_utc INTEGER NOT NULL, '
        f'{secure_column} INTEGER NOT NULL, is_httponly INTEGER NOT NULL, encrypted_value BLOB DEFAULT \'\')')
    for host, name, value, kind, path_, expires, secure in rows:
        plain, encrypted = '', b''
        if kind == 'plain':
            plain = value
        elif kind == 'v10':
            data = value.encode()
            if meta_version >= 24:
                data = hashlib.sha256(host.encode()).digest() + data
            encrypted = b'v10' + aes_cbc_encrypt_bytes(data, key, b' ' * 16)
        elif kind == 'v10-bad':
            encrypted = b'v10' + aes_cbc_encrypt_bytes(b'\xff\xfe' * 8, key, b' ' * 16)
        else:
            encrypted = value.encode()
        connection.execute(
            'INSERT INTO cookies VALUES (0, ?, ?, ?, ?, ?, ?, 0, ?)',
            (host, name, plain, path_, expires, secure, encrypted))
    connection.commit()
    connection.close()


def main():
    checkout, fixtures = sys.argv[1:]
    sys.path.insert(0, checkout)
    from yt_dlp.cookies import extract_cookies_from_browser

    for generated in ('firefox', 'chrome'):
        shutil.rmtree(os.path.join(fixtures, generated), ignore_errors=True)
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

    chrome_profile(
        os.path.join(fixtures, 'chrome', 'Default'), 24,
        [('.example.test', 'sid', 'secret value', 'v10', '/', 13400000000000000, 1),
         ('example.test', 'plain', 'visible', 'plain', '/', 0, 0),
         ('old.test', 'legacy', 'stored', 'old', '/', 13400000000000000, 0),
         ('bad.test', 'garbled', '', 'v10-bad', '/', 0, 0)])
    chrome_profile(
        os.path.join(fixtures, 'chrome', 'Old'), 20,
        [('example.test', 'sid', 'pre-24', 'v10', '/', 0, 1)], secure_column='secure')
    with open(os.path.join(fixtures, 'chrome', 'password.txt'), 'wb') as password:
        password.write(CHROME_PASSWORD)

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
    if sys.platform == 'darwin':
        from yt_dlp import cookies as module
        module._get_mac_keyring_password = lambda name, logger: CHROME_PASSWORD
        for profile in ('Default', 'Old'):
            logger = Logger()
            root = os.path.join(fixtures, 'chrome', profile)
            jar = extract_cookies_from_browser('chrome', root, logger)
            # macOS 'old data' stays bytes upstream; str() gives its repr.
            cookies = sorted(
                [c.domain, c.path, c.name, c.value if isinstance(c.value, str) else repr(c.value),
                 c.expires, bool(c.secure)] for c in jar)
            messages = [[kind, text.replace(os.path.abspath(fixtures), 'FIXTURES')]
                        for kind, text in logger.messages]
            # The patched keychain lookup logs nothing.
            messages.insert(2, ['debug', 'using find-generic-password to obtain password from OSX keychain'])
            cases.append({'browser': 'chrome', 'profile': profile, 'cookies': cookies,
                          'messages': messages})
    with open(os.path.join(fixtures, 'expected.json'), 'w') as output:
        json.dump(cases, output, indent=1)
        output.write('\n')


if __name__ == '__main__':
    main()
