# YouTube manual and automatic caption metadata

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against the official checkout at this exact target:
no intervening commits, changed files, affected components or unmapped files.
This ports existing pinned behavior without advancing the global baseline.

Inspected `_video.py:142` and the actual `_real_extract` caption loop at
4199–4343, subtitle-token diagnostics at 3208–3225, `_base.py:70–94` WEB token
policies and 1184–1204 text extraction, `YoutubeDL.py:2899–2919` sanitation and
3157–3220 normal/automatic selection, `_utils.py` query updates/URL sanitation,
`options.py` subtitle flags, and relevant existing subtitle tests in
`test/test_YoutubeDL.py`. These mapped modules also contain broader client/token
providers, site/media extraction, regex selection, conversion/embedding and option
behavior outside this slice. No intervening unmapped files require classification.

The pure Rust parser emits ordered URL/ext/name metadata for seven declared
server-side formats. It preserves actual query grouping, blank dropping,
replacement/key order, xosf removal, URL sanitation, manual/ASR language/name
rules, translation gating, suffixed manual translation queries, original aliases
and missing-name quirks. Eligible ASR language fills only missing audio-bearing
format language metadata. It does not fetch or decipher caption resources.
WEB subtitle PO-token policy is independent of media policy: protected exact
exp values skip remaining response tracks without provider token acquisition,
even if a pot query parameter already exists. Native diagnostics always warn
on this skip; actual upstream diagnostic level depends on options.

Orchestration adds independently reversible write-auto flags, normal-preferred
combined selection, optional names and automatic inventory listing. Normal
English prefixes precede automatic exact English by actual source order.
Read-only modes remain lazy; actual sidecar transfer shares existing native
HTTP retry/referrer/identity/cancellation/publication behavior and path planning.
No local format conversion, payload validation or embedding is implied.

The offline reference invokes actual pinned YoutubeIE._real_extract with saved
WEB player responses and unmodified caption/token policy. Video format generation
is stubbed to isolate captions, with network/download/process calls forbidden.
Twenty shared extraction cases and five actual process_subtitles cases compare
exact ordered URL/ext/name/language metadata; no volatile field or URL difference
is normalized away. One missing-manual-language case is recorded separately.
Ten native localhost pipeline cases cover watch/API extraction, unsupported
policy/metadata, no-runtime caption n, audio inference, normal-auto precedence and
actual sidecar bytes/referrers/progress. Additional CLI tests cover independent
flags/aliases/order and sidecar-only publication.

Resource bounds, strict malformed metadata rejection, absent captions, URL schemes
and generated-output limits are documented in the compatibility inventory. An
index map keeps duplicate query grouping linear while preserving declaration
order. Scope excludes additional clients/player responses, token acquisition,
authentication, caption URL challenges, manifest captions, regex languages,
compression, conversion/embedding, and any live-site reliability claim.

```sh
python3 tools/compare_youtube_captions.py --upstream /path/to/yt-dlp
```

See [the compatibility contract](../../docs/COMPATIBILITY.md#youtube-manual-and-automatic-captions)
and [fixture provenance](../../tests/fixtures/youtube/captions/README.md).
