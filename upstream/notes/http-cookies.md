# Shared scoped cookies and Netscape persistence

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against this exact official checkout/target:
no intervening commits, changed files, affected components or unmapped files.
This ports existing behavior without advancing the global baseline.

Inspected `cookies.py:93–113,1276–1410` loading, Netscape preprocessing,
session-zero handling, header construction and persistence; `options.py:1538–1545`
cookies/no-cookies flags; `YoutubeDL.py:1053–1066,4225` save/close and lazy jar
construction; `test/test_networking.py:520–580` response-cookie/shared-jar propagation; and relevant fixtures/tests in
`test/test_cookies.py`. These mapped files also implement browser databases,
encrypted credentials, cookie compatibility loading and browser/OS integration
outside this slice. Python standard-library cookie policy is used by the actual
reference runtime and is not separately pinned. No intervening unmapped changes
require classification.

The protocol store is [cookie_store 0.22.1](https://docs.rs/cookie_store/0.22.1/cookie_store/struct.CookieStore.html);
its version is recorded in Cargo.lock. Native CookieJar implements reqwest's cookie provider using cookie_store for
request scope and response updates. All native clients share an Arc jar, including
the bounded player client without relaxing canonical origin/path restrictions.
Secure cookies require HTTPS even for loopback HTTP. Native host-only scope
requires an exact host, rather than reproducing legacy Python subdomain leakage.
Canonical domain/path/name identity replaces host-only/domain variants where
legacy Python can retain both leading-dot records. Path/expiry/domain/response replacement/deletion and longest-path creation-order
headers are exercised with saved-source and local HTTP cases. Cookie values are
omitted from diagnostics, Debug and video metadata.

Netscape persistence supports named/nameless, session/zero/empty expiry, future
and expired cookies, leading-dot domain flags and HttpOnly prefixes. Missing
files start empty. Parsing and storage have explicit size/count/control bounds;
malformed existing files fail rather than source-style skipped lines. Atomic
same-directory saves preserve old bytes on failure and use private Unix file
permissions. The CLI saves state even in metadata modes and after failed inputs,
while no-cookies disables only file persistence. Initially missing paths use
no-clobber publication to protect media/new files from being overwritten.
Explicit library save/save_new and warning collection remain separate operations.

The offline source worker invokes actual YoutubeDLCookieJar.load,
extract_cookies/get_cookie_header and save/reload with sockets, downloads and
subprocesses forbidden. Five file cases and six response cases compare logical
records and sorted header multisets, with separate longest-path order assertions.
Three source-only cases describe legacy host-only subdomain scope, malformed-line
skipping and public-suffix acceptance. Save-byte/order and HttpOnly-prefix
preservation differences are not hidden by a file-byte parity claim. Native local
fixtures separately verify real authenticated header propagation, redirects,
failures, read-only persistence, shared clients, HLS/DASH fragments/keys and
no-overwrite behavior. The required suite makes no live-site requests.

Raw Cookie header compatibility remains unsupported; explicit scope is required.
No browser cookie import/decryption, public-suffix snapshot, browser context
SameSite/partitioned/prefix enforcement, account-specific YouTube authentication
or live-site reliability is claimed. Python remains development-only.

```sh
python3 tools/compare_cookies.py --upstream /path/to/yt-dlp
```

See [compatibility details](../../docs/COMPATIBILITY.md#scoped-http-cookies) and
[fixture provenance](../../tests/fixtures/cookies/README.md).
