# YouTube WEB HLS discovery

Exact source target and unchanged global baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19), from the official
yt-dlp checkout. `tools/upstream_report.py` at that target reports no intervening
commits, changed files, affected components or unmapped files. This ports another
coherent behavior from the pinned baseline; it does not advance that baseline.

Inspected `_base.py` WEB_PO_TOKEN_POLICIES (lines 72–89), `_video.py` initial
response/client policy attachment (3077–3133), `gvs_pot_required` (3267),
`get_manifest_n_challenge` (3292–3298), batch collection (3348–3381), and HLS/DASH
branches (3684–3759). Reviewed relevant source tests, including HLS duplicate-itag
cases and historical encrypted DASH examples. These mapped files also cover
authentication, clients, tokens, live/post-live, subtitles, DASH and ranking far
beyond this source slice. Historical encrypted-DASH examples do not demonstrate
a current manifest signature rewrite implementation.

The anonymous WEB policy makes HLS GVS tokens optional/recommended, but requires
them for DASH. Native extraction therefore admits HLS from initial responses and
the existing isolated WEB API fallback; it excludes DASH before preparation,
solving or acquisition. Direct API candidates retain the existing itag-18-only
restriction. All paths retain identity, playability, title, live and DRM gates.
There is no new client, token acquisition, authentication or live-site claim.

Manifest n challenges use raw `/n/{token}/` path values, unlike direct media query
values. Both join the same existing offline EJS batch, including signatures;
all input and output validation completes before media/manifest requests. Native
preparation rejects ambiguous paths, fragments, query-n manifests and legacy
`/s/` paths rather than treating them as solved. Pinned upstream passes the latter
two through without solving them in its manifest branch. Solved path values are
encoded as one segment and unrelated URL bytes are preserved.

The existing bounded native HLS parser builds VOD plans with initialization,
ranges, AES-128 metadata and external audio groups. JSON/extraction requests only
playlists; native fragment download and explicit FFmpeg merging handle media.
Native `hls-N` IDs and group namespaces avoid collisions with direct numeric itags.
Native ranking and TS/AAC/fMP4 extensions do not claim upstream sorter/remux parity.
Unlike upstream's nonfatal manifest parsing, a retained unsupported/failing native
manifest fails the whole extraction; this strict policy is documented explicitly.

`tests/fixtures/youtube/hls/` contains original synthetic player responses and
handwritten master/media playlists. They use the separately verified saved EJS
n vector; no third-party media is added. `tools/compare_youtube_hls.py` invokes
actual pinned YoutubeIE and its master parser with network blocked, ordinary WEB
policy, no token and no missing_pot override. It asserts HLS acquisition and DASH
exclusion for plain and path-n cases. A typed fixed n solver verifies path
replacement; it does not claim solver/acquisition parity. Rust extraction tests
consume the same expected metadata fixture. Exact-origin normalization and the
URL-checked 95/96-to-hls-0/hls-1 ID mapping are explicit; container labels,
selection, fragments and downloads are excluded from this comparison.

Local HTTP tests separately cover direct/master alternatives, API policy,
selection and grouped pairs, canonical download referrers, ordered fragment
assembly, no key/init/segment extraction requests, shared challenge execution,
malformed manifests and eligibility gates. The existing real offline Deno test
also resolves a saved-player manifest n token in the same batch as direct n/signature.
