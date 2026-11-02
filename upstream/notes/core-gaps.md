# Road to 1.0, step 7: core gaps

Survey of 2026-11-02 against the pinned baseline
`f264a6cbd1f630894da0f368df00f8aae50140e0` (2026.10.26), written before any
of the changes below. No baseline advance: this ports existing behavior.

## Impersonation for media transfers

Upstream: `--impersonate` is passed to every request handler
(`build_request_director`), and `ImpersonateRequestHandler._get_request_target`
uses it for any request without an `impersonate` extension, so `urlopen`
impersonates media too: HttpFD whole-file and `--http-chunk-size` ranges,
fragments (`HttpQuietDownloader` is an HttpFD), HLS keys and the media
playlist HlsFD refetches. A format's or subtitle's own `impersonate` field
(`_format_fields`) reaches only HttpFD (`_get_impersonate_target`: the first
available target, else `report_warning(_unavailable_targets_message(...))`
with the note `The extractor specified to use impersonation for this
download`), not fragments (`_download_fragment` builds a new info dict
without it). Such a format never goes to an external downloader
(`get_suitable_downloader`: `info_dict.get('impersonate') is None`). Of the
ported extractors only YouTube subtitles carry it (`impersonate: True`).

Port before: the `wreq` transport (`src/impersonate/transport.rs`) served
extraction requests only and buffered whole bodies; `send_media`
(`src/http.rs`) always used the plain reqwest client. The `impersonate`
field was kept in subtitle and format `extra` maps but ignored. Tests:
`tests/impersonate.rs` (ClientHello/SETTINGS capture through a local
`CONNECT` proxy, the feature-gated part runs in the `impersonate` CI job).

## Environment proxies and per-extractor proxy policies

Upstream: without `--proxy`, `YoutubeDL.proxies` is
`urllib.request.getproxies()` (environment `*_proxy` variables, lowercase
winning over uppercase, an empty lowercase value removing the scheme,
uppercase `HTTP_PROXY` ignored under CGI `REQUEST_METHOD`; without any, the
macOS System Configuration or the Windows registry) plus `https` copied from
`http` when missing. `clean_proxies` maps `__noproxy__` to direct, adds
`http://` to scheme-less values, renames `socks5`→`socks5h` and
`socks`→`socks4`, and skips unparsable values. `select_proxy` picks
`proxies[scheme]`, else `proxies['all']`, unless the `no` entry
(`NO_PROXY`) matches through `proxy_bypass_environment` (`*`, or a host or
`host:port` equal to or ending in `.NAME`, leading dots ignored,
case-insensitive) or the platform bypass. An explicit `--proxy` is
`{'all': ...}`, so it ignores `NO_PROXY`.

Per-extractor proxies upstream are the `Ytdl-request-proxy` header from
`geo_verification_headers()` (already ported as `--geo-verification-proxy`),
proxies passed to external downloaders and FFmpeg (already ported in
`src/external_fd.rs`/`src/ffmpeg_fd.rs`), the JavaScript runtimes' proxy
environment (the port runs runtimes with a cleared environment; see
COMPATIBILITY) and YouTube PO-token providers' `Request(proxies=...)`
(no PO-token providers here). No ported extractor sets per-request proxies.

Port before: `ProxyConfig::Environment` left reqwest's own environment
matcher in place (`HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`, uppercase first,
reqwest's `NO_PROXY` syntax, no `http`→`https` fallback, no other schemes,
no system settings since reqwest's `system-proxy` feature is off); the
`wreq` transport used `wreq`'s own. Tests: `tests/proxies.rs`
(`tests/fixtures/proxies/`, six omitted-option cases used as markers only).

## Archive URL-suitable fallback

Upstream `_make_archive_id`: an entry with an `id` but no `extractor_key`
or `ie_key` takes the key of the first extractor in `_ies` order (the
allowed extractors in class-name order, generic last) whose `suitable(url)`
accepts its `url`; no `url`, or no suitable extractor, gives no archive ID.

Port before: `YoutubeDl::entry_archive_identity` fell back to the port's own
marker `ie_key` (the extractor the port uses to resolve the entry), which is
the same key for every current case (Rokfin stacks), but not with
`--use-extractors` excluding it. Tests: `tests/fixtures/sites/archive-entries/`.

## Native live DASH

Upstream: a live `http_dash_segments` format uses FFmpegFD unless the
downloader is `native`. DashSegmentsFD then calls `report_error('Live DASH
videos are not supported')` and continues with the fragments of the
extraction's manifest snapshot. The CLI's default `ignoreerrors` is
`only_download`, so `trouble` only sets the exit code to 1; with
`--abort-on-error` (`ignoreerrors=False`) it raises and the download stops.
The snapshot is upstream's `_parse_mpd_periods` list (`src/dash.rs`
already reproduces it for dynamic MPDs). Upstream has no live-edge DASH
recorder of its own.

Port before: `--downloader native`/`dash:native` failed with the error and
downloaded nothing. FFmpegFD handling (recorded arguments, missing FFmpeg)
and the native failure are tested in `tests/dash_dynamic.rs` on the saved
`tests/fixtures/dash/dynamic-*.mpd` manifests.
