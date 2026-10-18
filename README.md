# yt-dlp-rs

An incremental, native Rust adaptation of [yt-dlp](https://github.com/yt-dlp/yt-dlp).
It downloads direct HTTP media and a limited subset of HTML5 media pages, with
a foundation for porting extractors and tracking upstream changes. This is an
early prototype; YouTube has a narrow watch-page subset, described below.

Requires stable Rust with edition 2024 support. Python 3.10+ and Git are needed
only for the upstream development tools. The Rust binary does not invoke Python
or an installed yt-dlp.

```sh
cargo build --locked
cargo run --locked -- 'https://example.com/video.mp4'
cargo run --locked -- --dump-json 'https://example.com/video.mp4'
cargo run --locked -- -f 1 'https://example.com/page-with-multiple-sources.html'
cargo run --locked -- --ffmpeg-location /path/to/ffmpeg -f 'bestvideo+bestaudio/best' 'https://www.youtube.com/watch?v=VIDEO_ID'
cargo run --locked -- --simulate 'https://example.com/video.mp4'
cargo run --locked -- -o 'downloads/%(title)s.%(ext)s' 'https://example.com/audio.mp3'
```

The example URLs illustrate syntax; replace them with actual media or supported HTML5 page URLs.
Install locally with `cargo install --locked --path .` to use `yt-dlp-rs` directly.

Supported today:

- HTTP(S) media identified by recognized audio/video MIME types, with a known
  file-extension fallback for absent or `application/octet-stream` content types.
- Redirects, percent-decoded URL filenames, query strings, and streamed downloads.
- UTF-8 HTML pages with `<video>`/`<audio>` elements and nonempty
  `src` or direct child `<source src>` elements. Progressive sources default to
  the highest declared resolution;
  `-f/--format` accepts `best`, `worst`, or a source ID (`0`, `1`, … in declaration order per element). Relative
  sources, the first `<base href>`, HTML entities, page redirects, and page referrers are supported. The media
  response must pass the same MIME/extension checks as a direct URL.
- Native HLS VOD downloads from direct playlist URLs or HTML5 sources:
  master selection, MPEG-TS/AAC segments, one fMP4 initialization map, and exact
  byte ranges. JSON/simulation fetch manifests but never segment bodies.
  Native AES-128 CBC decryption supports identity keys, explicit/sequence IVs and
  key rotation. External audio renditions retain group, language, name and
  default/autoselect metadata; automatic merging stays within the declared audio
  group. Live, SAMPLE-AES/DRM, discontinuous and external video/subtitle
  renditions remain unsupported.
- Static single-period DASH MPDs from direct URLs and HTML5 sources: inherited
  BaseURLs, numbered/time templates, finite timelines, SegmentLists and exact byte
  ranges. Native fragment downloads feed explicit audio/video merging. Dynamic,
  DRM, multi-period and SegmentBase/SIDX presentations remain unsupported.
- Multiple nonempty media elements form an HTML5 playlist, processed in document
  order. Empty elements are skipped. IDs and titles use upstream's numbered
  `-1` / ` (1)`, `-2` / ` (2)` suffixes. Titles prefer
  Open Graph `og:title`, then `<title>`, then the page URL filename stem.
- Typed progressive source metadata (`formats`, selected `format_id`, optional
  `width`/`height`) for HTML5. Child source dimensions come from positive integer
  `width`, `height`, and fallback `res` attributes. Ranking uses the smaller known
  dimension, or the only known one; unknown resolution sorts lowest. Ties select
  the last declaration for best/default and the first for worst.
- YouTube single-video watch, short-link, Shorts, embed, and live URL shapes
  canonicalized to a watch page (actual live content is rejected). Direct combined or separate audio/video MP4/M4A/WebM formats in
  `ytInitialPlayerResponse` are eligible;
  when that response is absent, one anonymous WEB player API request can supply
  token-exempt itag 18 using literal page client configuration.
  Format IDs are YouTube itags. This is fixture-tested groundwork, not general
  live-site support. `--youtube-js-runtime PATH` opts into bounded offline solving
  of encrypted signatures and query `n` challenges using explicitly installed
  Deno 2.9.7 and pinned bundled EJS assets. Challenge-free extraction never inspects
  or launches that runtime. Additional clients, token acquisition, fragmented adaptive streams,
  and playlists remain unsupported; many current videos will fail.
- Separate streams support `-f bestvideo+bestaudio` or literal pairs such as
  `-f 137+140`, with `/` fallbacks such as `bestvideo+bestaudio/best`.
  `--ffmpeg-location PATH` configures an explicit FFmpeg executable for local
  stream copying; default selection then prefers a video/audio pair. Without it,
  default selection requires an independently playable source. `-f best` always
  selects an independently playable source. This is a bounded selector subset.
- `-j/--dump-json`, `-s/--simulate`, `-o/--output`, multiple URLs, help and version.
- Output fields `%(title)s`, `%(id)s`, `%(ext)s`, `%(extractor)s`, and literal `%%`.
  Metadata cannot introduce directories; literal template paths can.
- Same-directory temporary files, cleanup on transfer failure, and no overwrite
  of existing destinations. A failed entry or URL sets a nonzero exit status while later
  entries and URLs still run. JSON goes to stdout; diagnostics go to stderr.
- Bounded download retries for selected transient HTTP statuses,
  connection/timeouts, and interrupted bodies. Interrupted transfers can resume
  within the same invocation with a strong ETag, known length, and validated range.
  `--retries N` sets additional
  attempts (default 3, 0 disables, maximum 255). Transient HTTP failures honor
  a single `Retry-After` delay-seconds or preferred HTTP-date value, capped at
  60 seconds.
- Download byte progress on stderr, at most once per second during transfers.
  Library observers can cancel at transfer checkpoints and during retry waits,
  cleaning up the private temporary file before publication.
- Opt-in whole-extraction retries with `--extractor-retries N` (default 0, maximum
  255), independent of download retries. Transient page/source request failures
  and interrupted HTML reads restart extraction before any output is emitted,
  using the same bounded `Retry-After` policy.

Other HTML pages, general YouTube support, broader DASH/HLS delivery, site playlists, full upstream format ranking, general selector
expressions, cookies/authentication, persistent resume, subtitles, and other
postprocessing remain unimplemented.
Unsupported CLI flags are errors. Filename handling is basic and does not yet
implement yt-dlp's platform-specific filename compatibility rules.

Direct extraction makes a streaming GET and inspects headers without consuming
the body. HTML5 extraction reads at most 2 MiB of uncompressed UTF-8/ASCII HTML,
then probes every declared media source. Extraction and format selection must
succeed for every entry before any media is downloaded or JSON emitted. JavaScript
players, lazy-loading attributes, and embedded site players are not supported by the generic extractor. A download makes another GET. There is a 15-second connection timeout
and a five-minute total timeout per request. Simulation and JSON extraction still make
network requests. MIME identification is not media-container verification.

The library exposes `YoutubeDl`, `ExtractResult`, `PlaylistInfo`, `VideoInfo`,
`VideoFormat`, `StreamKind`, `Fragment`, `ByteRange`, `HlsAes128`, `DownloadOptions`, `DownloadProgress`, `DownloadPhase`,
`ExtractionOptions`, `YoutubeOptions`, and an `Extractor` interface.
`YoutubeDl::with_youtube_options(YoutubeOptions { js_runtime: Some(path) })`
configures optional YouTube signature/n solving; `YoutubeDl::new()` keeps it disabled.
`YoutubeDl::new()?.with_ffmpeg(path)` configures optional local merging.
Use `extract_result` or
`extract_result_with_format(url, Some("1"))` for video/playlist results. A format
ID applies to every playlist entry independently; each must offer that ID.
The existing `extract_info` methods accept single videos and reject playlists.
`--dump-json` emits one video object per entry.
`download_with_options(info, path, DownloadOptions { retries: 0 })` disables
download retries for a library call; `download` uses the default policy.
`download_with_progress(info, path, options, callback)` reports one-based attempts,
stored bytes, optional total bytes, and the current phase. Fragment downloads also report one-based fragment indexes
and counts, with cumulative assembled bytes and a per-fragment attempt number. Return
`std::ops::ControlFlow::Continue(())` to continue or `Break(())` to cancel.
Cancellation is cooperative: it cannot interrupt an in-flight blocking request,
read, or filesystem operation. See the compatibility inventory for checkpoints.
`extract_result_with_options(url, None, ExtractionOptions { retries: 2 })` enables
whole-extraction retries; existing extraction methods keep retries disabled. See
[the roadmap](docs/ROADMAP.md) for architecture and upcoming milestones.
The [compatibility inventory](docs/COMPATIBILITY.md) records the exact subset and
the saved-fixture upstream comparison command.

To verify locally:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 -m unittest discover -s tools -p 'test_*.py' -v
```

The HTTP integration tests bind localhost sockets and need an environment that
allows them. CI is configured for Linux, macOS, and Windows; tests use no live sites.

Upstream tracking is pinned in [upstream/manifest.json](upstream/manifest.json).
To produce an agent-readable change report:

```sh
git clone https://github.com/yt-dlp/yt-dlp.git .upstream/yt-dlp
python3 tools/upstream_report.py .upstream/yt-dlp > upstream-report.json
```

Use a full clone so the baseline exists. Fetch newer commits explicitly before
subsequent reports. The report is read-only and includes changed files, commits,
affected Rust components, and unmapped files. It does not port code or advance
the baseline. [The upstream workflow](docs/UPSTREAM.md) describes the future
agent update loop. No recurring agent or automatic merge is enabled yet.

Licensed under the Unlicense, matching this repository's original license.
