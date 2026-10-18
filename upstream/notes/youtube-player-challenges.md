# Player acquisition and progressive URL challenge preparation

Current status: the later [n extraction slice](youtube-n-extraction.md) connects
query-n solving to public extraction behind explicit runtime configuration.
The private-only descriptions below record this slice's original boundary.

This slice connects the private player transport, EJS runner, and URL challenge
rewriter in deterministic tests. It does not yet enable challenge solving in the
public extractor or add CLI/library runtime options. Public YouTube extraction
continues rejecting unresolved signatures and `n` challenges. The next acceptance
gate is the dedicated pinned-Deno CI matrix, followed by integration with the
existing format eligibility/token policy and extraction metadata tests.

## Source revision and applicability

Exact official source and report target:
**897341241f2bfb2439fe0367f4c7c9ccd29a3828**. Ran:

```sh
python3 tools/upstream_report.py /private/tmp/yt-dlp-rs-challenge-upstream \
  --target 897341241f2bfb2439fe0367f4c7c9ccd29a3828
```

Baseline equals target; commits, changed files, affected components and unmapped
files are empty. The baseline stays unchanged. This is a port from the pinned
revision, not a catch-up claim. Reviewed `yt_dlp/extractor/youtube/_video.py`:
`_PLAYER_JS_VARIANT_MAP`, `_extract_player_url`, `_construct_player_url`,
`_extract_player_info`, `_load_player`, and `process_https_formats`' signature
and `n` handling. Also reviewed `test/test_jsc/test_ejs_integration.py` and its
existing saved `74edf1a3` challenge vectors. Broader upstream behaviors—forced
player versions/variants, iframe API discovery, caches, clients, PO tokens,
manifest challenges, adaptive formats—remain outside this slice.

## Player discovery and transport

`src/extractor/youtube/player.rs` reads the shared literal `ytcfg.set({...})`
configuration parser. `PLAYER_JS_URL` has priority; otherwise accept the unique
`WEB_PLAYER_CONTEXT_CONFIGS.*.jsUrl`. Conflicting context URLs and malformed or
missing configuration fail explicitly. No page JavaScript is evaluated and no
script-tag/iframe API fallback guesses a player. Later literal configuration
objects retain the existing top-level merge behavior.

Resolve against `https://www.youtube.com/`. Require that exact origin, no
credentials (including empty userinfo), query or fragment, and a recognized
`/s/player/<hex-id>/...` path. IDs are 8–64 hexadecimal characters. Recognize all
11 pinned upstream variant path shapes, with 1–32 ASCII alphanumeric/underscore
locale characters where applicable. URL text is at most 4096 bytes and contains
no whitespace, control characters, or backslashes. Unlike upstream's default
main-variant override, this boundary retains the actual validated path.

The player client has automatic redirects disabled, no implicit cookie/auth state,
the existing user agent/timeouts, and `Accept-Encoding: identity`. Explicit global
HTTP defaults now apply within this validated boundary; see
[request configuration](http-headers.md). It manually follows
at most ten HTTP 301/302/303/307/308 redirects. Every target is validated before
requesting it and must retain origin and player identity; changing a locale or
known variant with the same ID is allowed. Reject missing/duplicate/invalid
Location headers and other redirect statuses. A test-only constructor supplies
a loopback HTTP origin for fixtures; production has no origin-override API.

Require JavaScript MIME (`text/javascript`, `application/javascript`, or
`application/x-javascript`), optional UTF-8/UTF8/US-ASCII charset, identity/no content
encoding, and a nonempty UTF-8 body capped at 8 MiB whether or not a length is
advertised. Duplicate Content-Type or Content-Encoding headers are rejected.
The shared request helper rejects partial 206 responses. Transient statuses and
interrupted reads preserve the existing whole-extraction retry classification,
including Retry-After; this transport does not retry by itself.

The stricter origin, redirect, MIME/encoding, ambiguity and size checks are native
boundary policies. They do not assert identical upstream networking behavior.

## URL preparation and complete-batch resolution

`challenge/urls.rs` accepts exactly one string `url`, `signatureCipher`, or legacy
`cipher` field. Cipher form data requires `url` and encrypted `s`; `sp` defaults
to `signature`. Collect query `n` and signature challenges as distinct typed
requests, including when both have the same input string. Validate HTTP(S)
authority, no credentials, valid UTF-8 percent escapes, no whitespace/control
characters/backslashes in the URL, and reject decoded `n` path components used
by manifests. Reject duplicate cipher keys, duplicate decoded `n` query keys,
`sp=n`, empty challenges, and an existing query key colliding with `sp`.

Bounds are 256 KiB per raw/rewritten URL, 1 MiB per cipher, 1 KiB per signature
parameter, and the runner's 16 KiB per challenge/result. Duplicate rejection is
stricter than upstream's parse_qs behavior. Preserve every unrelated raw query
component and fragment byte for byte, retaining `n` key spelling/order and
encoding only replacement values and an appended signature key/value. The final
URL type performs its ordinary URL canonicalization before HTTP transport.

The private `challenge::resolve_urls` prepares every supplied source before
returning any URL. Its caller must already have applied progressive format,
DRM/live/client/token eligibility. For a challenge-free batch, it does not inspect
page configuration, inspect the supplied runtime path, launch a process, or fetch
a player. Otherwise it requires an explicit pinned Deno path, discovers and
fetches one bounded player, solves one typed batch, validates the entire response,
and rewrites only after validation. Missing/bad runtime, protocol, and URL failures
are permanent errors. Player network errors retain their transport classification.
It never probes media itself and never returns partial URL results.

## Evidence and remaining work

Required tests are local/offline and do not depend on Deno. They cover discovery,
all known path shapes, malformed/ambiguous config, origin and identity pinning,
redirect loops, duplicate headers, MIME/encoding/body failures, bounded reads,
retry classification, exact query encoding, custom/default signature parameters,
raw query/fragment preservation, missing/invalid solutions, and challenge-free
runtime bypass.

With `YT_DLP_RS_TEST_DENO` set, the dedicated runtime suite additionally serves
the saved real player over localhost, solves a cipher containing both signature
and `n` challenges, checks the exact rewritten URL including unrelated raw query
bytes, and requests fixed media bytes at the solved URL. This demonstrates the
private pipeline; it is not a public extractor end-to-end or live-site test.
The existing saved-player/upstream EJS comparator remains applicable.

Next: confirm the dedicated CI matrix, integrate the pipeline into the existing
eligible progressive source path, and test token filtering before player/media
requests, no unresolved URL probes, metadata/selection, failure atomicity,
whole-extraction retries and public runtime configuration. Do not widen supported
claims before those tests pass. Broader feature parity remains open.
