# Global native HTTP header configuration

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against this exact official checkout and target:
no intervening commits, changed files, affected components or unmapped files.
This ports existing pinned behavior without advancing the global baseline.

Inspected `options.py:268–297,1186–1200` callback/option definitions,
`__init__.py:477–480,914` finalization/parameter mapping,
`utils/networking.py:24–105` HTTPHeaderDict, and
`YoutubeDL.py:746–748,2686–2720,2982–2989,4490–4491` global/format/subtitle header
merges and scoped-cookie handling. Relevant generic HTML5 extraction assigns a
page Referer per format. These mapped modules and networking handlers also cover
cookies, proxies, authentication, randomized browser defaults, impersonation,
legacy encoding and broader request/redirect semantics outside this slice.
No intervening unmapped changes require classification.

Native HttpOptions validates header defaults before creating clients. Repeated
CLI header names replace case-insensitively; values split at the first colon and
trim whitespace. Convenience User-Agent/Referer values win even when custom
headers appear later, preserving actual upstream normalization. Reqwest request
headers override defaults, retaining extractor page referrers/API headers and
transport identity/range validation. All normal request paths share one client;
explicit defaults additionally reach the isolated, canonical-origin player
client without relaxing origin/path/redirect checks. No runtime is launched for
challenge-free extraction. Validation diagnostics omit supplied values.

The actual-source offline worker calls parse_options, HTTPHeaderDict and
YoutubeDL._calc_headers with network/download/process calls forbidden. It compares
21 option configurations, seven merges and seven grammar errors; three source-only
inputs and one Cookie merge explain exclusions. Only case-insensitive field names
are projected to lowercase; values remain exact. Fixed request defaults avoid
claiming parity with upstream's randomized browser User-Agent. Native localhost
tests separately exercise actual headers for extraction/downloads, fragments,
keys, subtitles, retries and redirects. Player tests verify defaults on permitted
redirects and reject a changed-origin target before issuing a request.

Raw Cookie options are explicit unsupported errors rather than a global Cookie
header: source compatibility loading requires a scoped cookie jar, not unscoped
forwarding. Native grammar, control/size bounds, UTF-8 header values and reqwest
redirect forwarding are documented separately. Header defaults do not add
cookie files, browser cookie extraction, proxy/impersonation, broader request
configuration or general site authentication. Source parsing acceptance alone
is not treated as proof of transport validity for unsafe inputs.

```sh
python3 tools/compare_headers.py --upstream /path/to/yt-dlp
```

See [the compatibility contract](../../docs/COMPATIBILITY.md#http-request-defaults)
and [fixture source references](../../tests/fixtures/headers/README.md).
