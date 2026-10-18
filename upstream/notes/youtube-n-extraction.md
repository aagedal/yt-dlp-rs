# Opt-in YouTube n challenge extraction

Current status: the subsequent [signature extraction slice](youtube-signature-extraction.md)
adds encrypted signatures and combined signature/n URLs. The n-only descriptions
below record this first public integration step.

The public YouTube progressive extractor can now solve query `n` challenges when
an explicit Deno executable is configured. This connects the previously private
player/EJS pipeline to extraction. Encrypted signatures remain a separate port
step; adaptive media, manifests, additional clients, authentication and PO-token
acquisition remain unsupported. This is saved-fixture support, not a live-site
reliability or whole-project parity claim.

## Source and acceptance evidence

Exact official source/report target:
**897341241f2bfb2439fe0367f4c7c9ccd29a3828**. Ran:

```sh
python3 tools/upstream_report.py /private/tmp/yt-dlp-rs-challenge-upstream \
  --target 897341241f2bfb2439fe0367f4c7c9ccd29a3828
```

Baseline equals target: no intervening commits, changed files, affected
components, or unmapped files. The global baseline stays unchanged. Inspected
`yt_dlp/extractor/youtube/_video.py`'s `process_https_formats`, query `n` lookup,
`solve_js_challenges`, per-client GVS token eligibility, and player-source helpers;
reviewed the existing pinned `test/test_jsc/test_ejs_integration.py` vectors.
Signature permutation caching, manifests, other clients/providers and token
workflows in these mapped components are outside this n-only extraction step.

The prerequisite offline runtime gate passed at commit
`153182bc07ee0bffc5497acbe287bd29899f5085` in
[CI run 39130773103](https://github.com/aagedal/yt-dlp-rs/actions/runs/39130773103):
pinned Deno 2.9.7 saved n/signature vectors, permission denial tests, and the
private acquisition-to-media pipeline passed on Linux, macOS and Windows.
That gate validates the runtime boundary, not this new extractor integration;
the updated runtime CI job now runs all YouTube tests, including extraction.

## Public configuration and flow

CLI: `--youtube-js-runtime PATH`. Library:

```rust
use yt_dlp_rs::{YoutubeDl, YoutubeOptions};
let downloader = YoutubeDl::with_youtube_options(YoutubeOptions {
    js_runtime: Some("/absolute/path/to/deno".into()),
    ..Default::default()
})?;
# Ok::<(), yt_dlp_rs::Error>(())
```

The existing `Youtube` unit extractor and `YoutubeDl::new()` preserve default
challenge-free behavior. `Youtube::with_options(options)` exposes configured
extraction through the existing `Extractor` interface. There is no PATH search,
runtime installation, runtime argument override, or remote asset download.
Configuration alone performs no runtime path check and starts no process.
When an eligible URL first needs solving, canonicalize the path and validate
exact Deno 2.9.7 once per configured extractor. Cache that validation (including
failure) across whole-extraction retries and later URLs; player source and
solutions are not persistently cached. Rebuild the extractor to change runtime
configuration after a validation failure.

Validate player identity, playability/live/DRM restrictions and eligible combined
audio/video MP4/WebM candidates first. API-derived formats are still limited to
itag 18 before player/runtime/media requests. Without runtime configuration,
query-n candidates remain skipped as unsupported; existing unchallenged
alternatives can still be selected. With configuration, collect eligible query-n
candidates and challenge-free alternatives, discover/fetch one bounded player,
solve/validate the complete batch, then probe every retained resolved media URL.
Only after all probes and format selection succeed can metadata be returned or
downloads begin. Do not probe an unresolved URL or emit partial metadata on
player/runtime/protocol failures. Encrypted cipher formats remain excluded.

Malformed/ambiguous URL fields and unsupported paths are rejected as format
candidates. Declared source types must be absent/null or
`FORMAT_STREAM_TYPE_NORMAL`; unknown, malformed and OTF types are excluded. In particular credentials, malformed percent escapes, duplicate n
keys and decoded manifest n path components cannot reach transport. Failure to
process an otherwise supported retained challenge fails the whole extraction,
even if an unchallenged alternative exists. Resolution ranking, explicit itag
selection, probe-derived filesize/final URLs, canonical webpage metadata,
referrers and no-clobber downloads keep their existing contracts.

Player transient HTTP/read failures participate in opt-in whole-extraction
retries; each attempt restarts the page/API/player/probe sequence. Runtime
configuration, solver/protocol and unsupported errors are permanent. All bounded
process/pipe, player origin/redirect/MIME/encoding/body and URL-preservation
policies from the previous runner/player slices still apply. Script execution is
not a hard OS sandbox or a memory limit, and extraction remains synchronous
without a cancellation API.

## Deterministic evidence

`src/extractor/youtube/integration.rs` uses a native compiled fake runtime to test
metadata, selection, probe/download URLs and referrers, one batch with deduplicated
challenges, lazy runtime use, API token filtering, no runtime invocation for
gated/live/DRM/adaptive/OTF content, whole-extraction player retries with one-time
runtime validation, permanent malformed/missing/stderr/nonzero solver failures,
and no media probes after failure. `tests/http.rs` covers public library/CLI
configuration for direct/HTML5 JSON, simulation and downloads, and missing CLI
path arguments. Base tests require no Deno installation or live-site requests.

With `YT_DLP_RS_TEST_DENO` configured, a saved real player is served on localhost
and Deno solves the pinned n vector through the extractor's actual candidate,
player, URL-resolution and media-probe path. The test adapter changes only page
and player origins to loopback; production has no endpoint override. The existing
EJS comparison verifies exact saved challenge outputs against pinned upstream.

The saved n response/expected metadata comparison uses a fixed typed upstream
challenge response for the same independently verified vector, comparing the
existing normalized metadata allowlist exactly. It does not compare live player
acquisition, runtime invocation, or token acquisition. Only the exact fixture
origin is normalized; no timestamp/result/query masking is added.

Next: wire encrypted signature URL candidates through the same eligibility and
complete-batch boundary with signature/custom-sp/combined-n extraction fixtures.
Full feature parity remains open across the roadmap's other extractors,
protocols, options and postprocessors.
