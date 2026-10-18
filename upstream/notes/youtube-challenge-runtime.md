# YouTube JavaScript challenge runtime decision

Decision: use an opt-in external Deno process executing pinned, offline EJS
assets. Rust owns player acquisition, challenge batching, response validation,
URL rewriting, and media transport. Python remains development-only.

This completes the runtime-boundary investigation following the
[anonymous WEB API slice](youtube-player-api.md). It does not implement signature
or `n` solving, install a runtime, or expand the supported extraction subset.
Existing challenge rejection remains in place until the acceptance gates below
pass. This decision was independently reviewed against the same pinned sources.

## Source revision and applicability

Official yt-dlp source and exact target:
[`897341241f2bfb2439fe0367f4c7c9ccd29a3828`](https://github.com/yt-dlp/yt-dlp/tree/897341241f2bfb2439fe0367f4c7c9ccd29a3828).
Ran:

```sh
python3 tools/upstream_report.py /private/tmp/yt-dlp-rs-challenge-upstream \
  --target 897341241f2bfb2439fe0367f4c7c9ccd29a3828
```

The report has the same baseline and target, with empty commits, changed files,
affected components, and unmapped files. This is baseline research, not an
upstream update or a parity claim. The global baseline stays unchanged.

Sources inspected at that revision (paths relative to the official checkout):

- `yt_dlp/extractor/youtube/jsc/provider.py` and `jsc/README.md`: typed `n` and
  `sig` requests, player identity, challenge lists, keyed results, and provider
  errors. Only the Python provider API is public; EJS integration is internal.
- `yt_dlp/extractor/youtube/jsc/_builtin/ejs.py`: grouping by player URL,
  player acquisition, library/core assembly, JSON payloads, ordered responses,
  preprocessing cache, asset version checks, and SHA3-512 verification.
- `yt_dlp/extractor/youtube/jsc/_builtin/deno.py`, `node.py`, and `quickjs.py`:
  subprocess invocation, permissions, script variants, and temporary files.
- `yt_dlp/extractor/youtube/jsc/_builtin/vendor/_info.py`,
  `yt.solver.core.js`, and `yt.solver.deno.lib.js`: EJS version **0.8.0**,
  artifact hashes, solver structure, and dependency loading.
- `yt_dlp/utils/_jsruntime.py`: upstream Deno detection accepts versions starting
  at **2.3.0**. That is upstream's minimum, not a tested Rust support promise.
  `pyproject.toml` separately requests Deno >=2.6.6 in its optional dependency
  group and pins 2.9.5 for its pinned group.
- `yt_dlp/extractor/youtube/_video.py`: player URL/source acquisition,
  signature timestamp, batched challenge collection, signature permutation
  caching, and media URL substitution. Rust can initially solve actual signature
  strings rather than port upstream's permutation cache.
- `test/test_jsc/test_provider.py`, `test_ejs_integration.py`, and `conftest.py`:
  provider validation and real challenge vectors. Integration tests are download
  tests: missing player scripts are fetched into `test/testdata/sigs`. They are
  not an offline fixture suite ready to copy into this repository.

These sources also cover multiple runtimes/providers, remote asset acquisition,
plugin preferences, persistent preprocessing caches, manifests, and other client
workflows. Those remain outside this decision's implementation scope. EJS is a
separate dependency whose version and provenance must be tracked independently
of the yt-dlp baseline.

## Why an external Deno process

Deno already has an upstream EJS adapter, stdin script execution, and permissions
for denying script network, files, environment, and subprocess access. A separate
process can be terminated when a player loops or the solver fails. This keeps
engine integration out of the Rust extraction core and lets us reuse the solver
that upstream maintains.

An embedded engine would add bindings, engine/platform packaging, language
compatibility work, interrupt handling, and memory accounting. It would still
need EJS's parser and code generator. It is deferred until there is evidence
that the external boundary is insufficient. Node and QuickJS adapters are also
deferred: supporting several runtimes immediately would multiply the execution
and packaging test matrix. No Python interpreter, installed yt-dlp, browser, or
shell command is part of the runtime boundary.

The current [upstream EJS guide](https://github.com/yt-dlp/yt-dlp/wiki/EJS) also
describes external runtimes and separate solver assets. Implementation decisions
above are grounded in the pinned source, since the guide can change.

## Assets and execution contract

Vendor the matching **full library bundle and core**, with exact EJS release,
upstream artifact URLs, SHA3-512 hashes, and all applicable dependency licenses.
Verify artifacts during the development import, record their digests in a
separate asset ledger, and embed the reviewed bytes in the binary. Updating EJS
requires fixture comparisons and review; runtime execution performs no asset
downloads or package installation.

The pinned `yt.solver.deno.lib.js` is not that full library: it imports
`npm:meriyah@6.1.4` and `npm:astring@1.9.0`. Copying that file and the core alone
would require npm downloads or ambient caches. Obtain the full matching library
artifact and audit its licenses before vendoring; do not substitute the npm
stub or infer that the core's Unlicense covers every bundled dependency.

Use an explicitly configured Deno executable; an ordinary direct/HTML5 or
challenge-free YouTube extraction must not discover or launch it. Introduce CLI
and library configuration only with working solver integration. Resolve the
executable once, use argument arrays, and never invoke a shell. Reject missing
or unsupported runtime versions with an actionable error when solving is needed.
Initially pin a CI runtime version and test it on Linux, macOS, and Windows
before declaring a supported version range.

The starting invocation for the offline bundle follows the pinned adapter:

```text
deno run --ext=js --no-code-cache --no-prompt --no-remote --no-lock \
  --node-modules-dir=none --no-config --no-npm --cached-only -
```

Grant no script permissions. Use a private working/cache directory and clear
ambient environment variables, retaining only platform necessities through a
documented allowlist and explicitly set Deno controls. In particular, inherited
runtime flags, proxy settings, configuration, and npm caches must not affect the
result. Deno itself may write internal cache data despite script permissions;
the private directory must be cleaned after the process exits.

One process handles one bounded batch for one player. Compose reviewed library
and core source with a JSON-serialized data literal and a fixed invocation of
`jsc`; never interpolate player or challenge text as executable syntax. The EJS
payload is `type: player`, `player`, and `requests` containing `type: n|sig` and
`challenges`. Start without preprocessing output or persistent workers/caches.
The EJS envelope is version-specific and stays behind a Rust adapter rather
than becoming the public library API.

Initial runner budgets: 8 MiB player source, 16 MiB complete stdin, 128 unique
challenges per batch, 16 KiB per challenge, 1 MiB stdout, 64 KiB stderr, and a
15-second wall deadline covering spawn, writing, reading, and exit. Exceeding a
budget is an error, not truncation or partial success. Confirm these budgets
against saved real players before enabling solving; adjust them explicitly with
regression evidence if needed.

Drain stdout and stderr concurrently with writing stdin. The supervisor must
remain able to terminate a child that does not read input or close output.
On deadline, overflow, or any pipe/process failure, kill and wait for the child,
close pipes, and join helper threads before returning. Never block indefinitely
in `wait_with_output` or a writer thread. Nonzero exit and nonempty stderr are
errors under the initial strict policy; bound and sanitize diagnostics.

Deno permissions and process separation are not a hard OS sandbox or an RSS
limit. Heap flags do not bound total process memory. The first adapter makes no
hard-memory-limit claim. If hostile saved fixtures show an unacceptable gap,
add an OS-specific containment design before widening execution scope.

## Rust responsibilities and failure behavior

Keep private challenge data/validation separate from process execution under
`src/extractor/youtube/`. The runtime accepts player source and typed `n`/signature
requests and returns validated results; it performs no HTTP or media URL edits.
Player requests use the Rust transport and extraction retry policy, with an
explicit read cap and validated YouTube player URL/redirect policy. Add acquisition
only when its allowed URL shapes and local HTTP fixtures are defined.

Deduplicate challenge strings within each kind while preserving deterministic
order. Validate one complete UTF-8 JSON response: top-level success, exact response
count/order, supported response variants, exact requested challenge keys, and
bounded nonempty string results. Reject duplicate JSON object keys rather than
allowing last-value-wins ambiguity. Reject malformed/trailing output, missing or
unexpected keys, and whole-batch or per-request errors before any candidate URL
is changed. A result equal to its input can be valid; do not infer solver failure
from string equality.

Only after validation may extraction parse/transform supported cipher URLs and
replace signature or `n` query values. Test encoding, custom `sp`, and preservation
of unrelated query parameters explicitly. Manifest `/n/` paths stay unsupported.
Keep API itag/token eligibility and all existing live/DRM/adaptive restrictions
independent of solving. A solved challenge does not supply a PO token or imply
that an otherwise unsupported format is eligible. Complete extraction of every
entry before emitting metadata or starting downloads, as the current API requires.

Runtime/asset/protocol failures are deterministic extraction errors, not transient
HTTP errors and not candidates for automatic retries. Player HTTP acquisition
can use existing opt-in whole-extraction retries. Do not add extraction
cancellation claims: the library currently exposes cancellation for downloads
only. A future extraction cancellation API must also stop and reap the runner.

## Implementation sequence and acceptance gates

1. Import reviewed full EJS 0.8.0 assets with provenance/licenses/hashes, then
   establish checked-in synthetic and saved-player challenge vectors. Record
   player source URL, content hash, upstream vector provenance, and exact
   expected outputs. Neither asset download nor live player fetching belongs in
   required tests.
2. Implement the private typed protocol and bounded Deno runner. Pure tests
   cover JSON escaping/control characters, batching, duplicate/missing/extra
   results, malformed envelopes, and solver errors. A compiled Rust fake-runtime
   helper tests arguments, environment isolation, blocked stdin, output floods,
   nonzero exit, deadline, and process/thread cleanup on all CI platforms.
   Fake-runtime success establishes orchestration behavior only.
3. Run the real pinned Deno with the reviewed assets and checked-in players for
   both `n` and signature vectors. Compare exact results with upstream using
   those same saved players while network access is forbidden. A dedicated CI
   job must require the runtime rather than silently skip it; the base suite
   remains offline and does not require a user's Deno installation.
4. Integrate bounded player acquisition and one challenge kind at a time into
   the current progressive subset. Add local HTTP extraction/download fixtures
   for each behavior and its failure paths. Verify unchanged token filtering,
   no media probe of unresolved challenge URLs, selection, and output. Only then
   add runtime options and update compatibility claims/source mappings.

The decision itself introduced no runtime assets or extractor capabilities.
Steps 1–2 and local real-runtime vectors are now implemented by the
[offline runner slice](youtube-challenge-runner.md), with a dedicated CI matrix
added for step 3. Cross-platform results and extraction integration (step 4)
remain acceptance gates. Adaptive merging, signature timestamp API
changes, new clients, PO tokens, manifests, persistent caches, and live-site
reliability remain separate work.

## Investigation evidence

The pinned-source report above and an independent source review support this
decision. A local development-only smoke probe used Deno **2.9.7** on macOS arm64
with the exact offline flags above, a cleared environment, and a private directory.
It verified stdin execution, JSON round-trip of NUL/Unicode strings, dynamic
function evaluation, and denial of script environment reads, file reads/writes,
localhost fetch, and subprocess execution. No solver assets or YouTube player
were executed. This is invocation/permission evidence for that runtime only,
not challenge-solving, cross-platform, deadline, or hard-sandbox verification.
