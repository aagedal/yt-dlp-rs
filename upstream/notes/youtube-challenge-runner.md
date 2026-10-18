# Offline YouTube challenge runner

This slice implements the private offline EJS protocol and Deno process boundary
selected in [the runtime decision](youtube-challenge-runtime.md). It does **not**
connect that runner to extraction, fetch player scripts at runtime, rewrite media
URLs, expose runtime CLI/library options, or establish live-site reliability.
The existing YouTube extractor continues rejecting signatures and `n` challenges.

## Source and applicability

Exact official yt-dlp source/target:
**897341241f2bfb2439fe0367f4c7c9ccd29a3828**. Ran:

```sh
python3 tools/upstream_report.py /private/tmp/yt-dlp-rs-challenge-upstream \
  --target 897341241f2bfb2439fe0367f4c7c9ccd29a3828
```

Baseline equals target: commits, changed files, affected components, and unmapped
files are all empty. This ports a behavior slice from the existing baseline;
the global baseline has not advanced. Inspected `jsc/_builtin/ejs.py` for
`_construct_stdin` and response correlation, `jsc/_builtin/deno.py` for offline
invocation, `jsc/provider.py` for challenge types, the vendored `_info.py` and
solver core, and `test/test_jsc/test_ejs_integration.py` for exact test vectors.
Paths above are under `yt_dlp/extractor/youtube/` unless prefixed with `test/`.
Other runtime providers, player acquisition/caching, preprocessing caches,
plugins, and extraction URL transformations are outside this slice.

EJS **0.8.0** is a separate dependency. `vendor/ejs/manifest.json` records the
full offline library and core artifact URLs and SHA3-512 digests checked against
upstream `_info.py`. The core matches the pinned checkout byte for byte. The
Unlicense is included; the library retains meriyah 6.1.4 ISC and astring 1.9.0 MIT
notices. No npm stub, remote import, runtime asset download, or package install
is used by the Rust runner. `tools/test_ejs_assets.py` checks assets, notices,
player provenance, and exact vector bytes without network access.

## Implemented contract

`src/extractor/youtube/challenge.rs` owns private typed `n`/`sig` batches,
per-kind first-seen deduplication, JSON serialization, and complete result
validation. Limits are 8 MiB player text, 128 unique challenges across both
kinds, 16 KiB per nonempty challenge/result, 16 MiB complete stdin, 1 MiB stdout,
and 64 KiB stderr. JSON escaping can exceed the stdin cap even when raw player
text fits; that is an explicit failure. No preprocessing output is requested.
Player and challenge text enter the script only as serialized JSON data.

Validation rejects malformed/trailing/non-UTF-8 JSON, duplicate keys including
escaped aliases, unexpected fields, unsupported variants, wrong response count,
missing/extra challenge keys, null/nonstring/empty/oversized results, and any
whole-batch or per-request error. Positional response correlation follows
upstream; the envelope has no challenge-kind field in each response. Equal input
and output strings are permitted. No partial solution set is returned.
Diagnostics are escaped and bounded.

`challenge/runtime.rs` canonicalizes an explicitly provided executable path,
checks pinned Deno **2.9.7**, and runs offline stdin JavaScript with the decision's
exact flags and no script permissions. It clears the environment, retaining only
Windows `SystemRoot`/`WINDIR` where present, and sets `DENO_DIR`,
`DENO_NO_UPDATE_CHECK`, `DENO_NO_PACKAGE_JSON`, and `NO_COLOR`. A fresh private
directory is the working/cache directory and is removed after the process and
pipe threads finish. Version detection has a separate bounded invocation;
challenge-free extraction never calls either invocation.

The 15-second process deadline covers spawning, concurrent stdin writing,
stdout/stderr reading, and exit. Pipe overflow, write/read/thread-start errors,
nonzero exit, nonempty stderr, and deadline expiry fail explicitly. The supervisor
kills and waits on failed children before joining pipe workers. This relies on
the fixed Deno invocation denying subprocess execution; it is not a general
supervisor for arbitrary programs that leave descendants holding its pipes.
There is no OS sandbox, hard RSS bound, extraction cancellation API, or retry
of solver failures. OS scheduling and process termination can add cleanup time
beyond the deadline.

## Validation and remaining gates

The saved `74edf1a3` player and vectors in
`tests/fixtures/youtube/challenges/expected.json` retain the original player URL,
content hash, upstream vector source commit, three `n` pairs and two signature
pairs (including control characters). Expected output is exact; no URL,
timestamp, challenge, or result normalization is applied. Pure synthetic tests
cover serialization, grouping, protocol failures, and limits. A portable Rust
fake runtime covers arguments, environment, private-directory cleanup, blocked
stdin, a hung process after reading input, floods, nonzero exit, stderr, and
invalid output bytes. Required base tests need no Deno installation or live site.

Run real-runtime tests explicitly:

```sh
YT_DLP_RS_TEST_DENO=/absolute/path/to/deno \
  cargo test --locked --lib extractor::youtube::challenge
python3 tools/compare_ejs.py --upstream /path/to/pinned/yt-dlp \
  --deno /absolute/path/to/deno --check-rust
```

The first command checks exact saved-player results and denial of script network,
file, environment, and process access. A configured missing/wrong runtime fails.
The dedicated `youtube-challenges` CI job installs pinned Deno and requires these
tests on Linux, macOS, and Windows. Local evidence is macOS arm64 with Deno 2.9.7;
adding the CI matrix does not prove its remote jobs have passed.

The optional comparison invokes the actual pinned upstream stdin constructor and
solver with the same saved player, library, core, and challenges. It ignores only
upstream's `preprocessed_player` cache output, which Rust does not request, and
checks every remaining response/key/value exactly. `--check-rust` also runs the
Rust saved-vector test. All solver execution is offline; Python is reference
tooling only.

Before extraction integration: confirm the dedicated CI matrix, define strict
player URL/redirect acquisition with bounded local HTTP fixtures, then implement
one challenge kind at a time with exact URL encoding/custom signature-parameter
coverage. Preserve token filtering, live/DRM/adaptive restrictions, and rejection
of unresolved challenge URLs before any media probe. Only working extraction
integration should introduce public runtime options or expand supported claims.
