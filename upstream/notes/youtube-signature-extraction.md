# Opt-in YouTube encrypted signature extraction

This second public challenge step follows query-n integration. Eligible
progressive MP4/WebM sources can now use encrypted `signatureCipher` (and legacy
`cipher`) fields when an explicit Deno 2.9.7 runtime is configured. Both signature
and query-n challenges can be resolved in one complete player batch. Default
extraction still excludes challenged formats. This does not add clients, PO
tokens, authentication, adaptive streams, manifests, or live-site reliability.

## Source and applicability

Exact official source/report target remains
**897341241f2bfb2439fe0367f4c7c9ccd29a3828**. The report command in the
[n integration note](youtube-n-extraction.md) ran against that exact revision;
baseline equals target with empty intervening commits, changed files, affected
components and unmapped files. The global baseline is unchanged.

Inspected `_video.py`'s `process_https_formats`, encrypted-signature collection,
`solve_js_challenges`, artificial index/permutation signature requests,
`solve_sig`, default/custom `sp` construction, and subsequent n query rewriting.
Reviewed the pinned `test/test_jsc/test_ejs_integration.py` vectors and used the
same saved player. Upstream's signature-length permutation cache, other clients,
manifest transformations, SABR and adaptive transport remain outside this step.
Rust solves actual signature strings rather than caching an index permutation.

## Integrated behavior

The existing native `--youtube-js-runtime PATH`, `YoutubeOptions`, lazy one-time
runtime path/version validation, bounded offline assets/process execution,
player acquisition policy, and extraction retry behavior are unchanged.
Playability, identity, live/DRM/OTF/type, muxed-codec and WEB API itag-18 eligibility
filtering precede any player fetch, runtime solve or media probe. WEB API token
filtering now also precedes parsing a cipher field: an ineligible protected
format cannot trigger player work or make an exempt format fail.

Retain exactly one nonnull `url`, `signatureCipher`, or `cipher` source field.
Cipher form data requires a direct HTTP(S) `url` and nonempty encrypted `s`.
`sp` defaults to `signature`; a nonempty custom name is encoded as a query key.
Reject ambiguous source fields, duplicate decoded cipher keys, malformed percent
escapes, invalid/non-HTTP URL text, credentials, duplicate n parameters, `sp=n`,
and collision with an already-present signature query key. These are unsupported
format candidates and are never probed. A retained runtime/solver failure fails
the whole extraction, including when an unchallenged alternative exists.

The private rewriter validates the entire typed signature/n response before
changing candidate URLs. It replaces only n values and appends the encoded
signature key/value, preserving unrelated raw query components and the fragment
through preparation. HTTP probes do not transmit fragments; metadata records
the final response URL, following the existing extractor contract. Every retained
resolved URL must pass the existing direct-media probe before metadata/selection
can succeed. Downloads use the resolved final URL and canonical watch referrer.

No public signature permutation cache or persistent player/solution cache is
introduced. Solving does not acquire a PO token, supply a signature timestamp to
the WEB API, bypass access gates, or make unsupported streams eligible.

## Regression and differential evidence

Local HTTP tests in `youtube/integration.rs` cover default `signature`, custom
`lsig`/`sig`, legacy cipher, combined signature/n, exact raw query handling,
metadata, probe/download URLs and canonical download referrers, API token
filtering, missing runtime, ambiguous/malformed/colliding/path cipher rejection,
and complete-batch failure before any unchallenged or challenged media probe.
The required suite uses a compiled native fake runtime; it proves orchestration,
not real signature solving.

`signature-extraction.json` and `signature-extraction-expected.json` use the
saved player's exact long signature vector plus the existing n vector. With
`YT_DLP_RS_TEST_DENO` set, Rust serves that player locally, executes real offline
EJS, extracts all allowlisted metadata exactly, and downloads fixed media from
the resulting signed URL. No live requests or package downloads occur.

For the same fixture, the upstream comparator now optionally executes the actual
upstream EJS stdin constructor with vendored assets and saved player. It passes
upstream's own typed artificial signature-index and n requests to Deno and sends
the actual typed solutions back into upstream extraction. This exercises the
upstream permutation path and the Rust actual-string path against identical
expected metadata. Only the exact fixture origin is normalized; query/signature
values, format ordering, dimensions, filesize and identity fields remain exact.

```sh
python3 tools/compare_youtube.py --upstream /path/to/pinned/yt-dlp \
  --deno /absolute/path/to/deno
```

The comparator requires an exact pinned upstream HEAD before executing any case.
Without `--deno`, the previous three comparison cases still run and the signature
case is explicitly reported skipped. Required Python tests need no runtime.
Local full comparisons pass with Deno 2.9.7; the dedicated CI job now runs all
YouTube extraction and runtime tests on Linux/macOS/Windows. The earlier runtime
acceptance gate passed on all three platforms, as recorded in the n note; that
is not a claim that a later integration commit's CI has already finished.

Remaining parity work includes broader player/client and token behavior,
JavaScript/player variants beyond saved evidence, adaptive streams/merging,
manifest protocols, playlists, other extractors and the rest of the option and
postprocessing surface. No general YouTube/yt-dlp compatibility claim is made.
