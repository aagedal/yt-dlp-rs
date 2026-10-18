# Explicit shared HTTP and SOCKS proxy routing

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against this exact official checkout/target:
no intervening commits, changed files, affected components or unmapped files.
This ports existing behavior without advancing the global baseline.

Inspected `options.py:609–613`, `YoutubeDL.py:4205–4218`,
`utils/networking.py:169–196,246–258`, `networking/_helper.py:53–90`,
`socks.py:154–210` and `test/test_networking.py:1687–1701,1837–1870`, plus request
handler scheme support and proxy tests. These mapped files also implement
platform/environment discovery, bypass behavior, per-request/geo-verification
proxies, additional backend policies and compatibility handling outside this
slice. No intervening unmapped changes require classification.

The pure native policy validates explicit endpoints without DNS/network requests,
then configures both the main and bounded player clients. An empty option forces
direct connections; an explicit endpoint overrides environment routes/bypass.
Missing schemes become HTTP; SOCKS aliases retain the source remote-DNS semantics.
Global headers, scoped cookies, transport validation, retries and extraction gates
remain independent of routing. No direct fallback is configured for failed proxies.
The omitted option deliberately retains existing reqwest environment/system
behavior rather than claiming the Python environment fallback/bypass algorithm.

HTTP proxy Basic credentials are decoded explicitly and preserve empty/user-only
fields as an explicit native policy; source backends can omit HTTP auth for
incomplete/empty fields, so this is not a universal wire parity claim. SOCKS5 offers user/password auth only with both nonempty values as source
does. Current SOCKS4 backend cannot send USERID, so native rejects such URLs
instead of silently discarding it. Strict URL bounds, unsupported paths/query/
fragments and legacy per-request proxy headers produce value-free errors.

The offline worker invokes actual parse_options, YoutubeDL.proxies, clean_proxies,
select_proxy and make_socks_proxy_opts with controlled discovered proxy mappings,
real environment bypass predicates and network/download/process calls forbidden.
Fifty source configurations and eight source-only cases preserve source results.
Native parser tests compare 39 explicit endpoint projections, reject five SOCKS4
credential cases and treat six omitted-option cases as environment-mode markers
only. Host casing/default ports/URL formatting projection is documented; credentials
and DNS flags stay explicit. Configuration comparison is not a transport proof.

Eleven deterministic local integration tests establish actual HTTP absolute-form
requests, proxy Basic auth, SOCKS4 IPv4, SOCKS4a/SOCKS5(h) remote hostname fields,
SOCKS5 auth/no-auth, direct/environment overrides, cookies/referrers, HLS/DASH
resources, retries and redirects. Three saved-certificate Rustls tests establish
verified HTTPS proxy and CONNECT origin transport, auth exclusion from the inner
HTTP stream and untrusted-certificate rejection. The test CA is trusted only by
private test clients, never installed in the OS or production client. Certificates
and deliberately public dummy key are saved fixtures; tests invoke no OpenSSL.
The bounded-player proxy regression also rejects an off-origin redirect before
another request. Required tests make no live-site requests.

Remaining scope includes SOCKS4 USERID, full upstream environment discovery and
bypass/PAC semantics, per-request/extractor proxies and geo-verification routing.
Python remains development/reference tooling only.

```sh
python3 tools/compare_proxies.py --upstream /path/to/yt-dlp
```

See [compatibility details](../../docs/COMPATIBILITY.md#explicit-proxy-routing) and
[fixture provenance](../../tests/fixtures/proxies/README.md).
