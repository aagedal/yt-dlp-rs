# yt-dlp-rs

An incremental, native Rust adaptation of [yt-dlp](https://github.com/yt-dlp/yt-dlp).
It downloads direct HTTP media and a limited subset of HTML5 media pages, with
a foundation for porting extractors and tracking upstream changes. This is an
early prototype; YouTube and other site-specific extractors are not implemented.

Requires stable Rust with edition 2024 support. Python 3.10+ and Git are needed
only for the upstream development tools. The Rust binary does not invoke Python
or an installed yt-dlp.

```sh
cargo build --locked
cargo run --locked -- 'https://example.com/video.mp4'
cargo run --locked -- --dump-json 'https://example.com/video.mp4'
cargo run --locked -- --simulate 'https://example.com/video.mp4'
cargo run --locked -- -o 'downloads/%(title)s.%(ext)s' 'https://example.com/audio.mp3'
```

The example URLs illustrate syntax; replace them with an actual direct media URL.
Install locally with `cargo install --locked --path .` to use `yt-dlp-rs` directly.

Supported today:

- HTTP(S) media identified by recognized audio/video MIME types, with a known
  file-extension fallback for absent or `application/octet-stream` content types.
- Redirects, percent-decoded URL filenames, query strings, and streamed downloads.
- UTF-8 HTML pages with exactly one `<video>` or `<audio>` element and one
  `src` or direct child `<source src>`. Relative sources, the first `<base href>`,
  HTML entities, page redirects, and page referrers are supported. The media
  response must pass the same MIME/extension checks as a direct URL.
- HTML5 IDs and titles use upstream's `-1` and ` (1)` suffixes. Titles prefer
  Open Graph `og:title`, then `<title>`, then the page URL filename stem.
- `-j/--dump-json`, `-s/--simulate`, `-o/--output`, multiple URLs, help and version.
- Output fields `%(title)s`, `%(id)s`, `%(ext)s`, `%(extractor)s`, and literal `%%`.
  Metadata cannot introduce directories; literal template paths can.
- Same-directory temporary files, cleanup on transfer failure, and no overwrite
  of existing destinations. A failed URL sets a nonzero exit status while later
  URLs still run. JSON goes to stdout; diagnostics go to stderr.

Other HTML pages, YouTube, HLS/DASH, playlists, format selection, cookies/authentication,
resume/retries, subtitles, and ffmpeg postprocessing remain unimplemented.
Unsupported CLI flags are errors. Filename handling is basic and does not yet
implement yt-dlp's platform-specific filename compatibility rules.

Direct extraction makes a streaming GET and inspects headers without consuming
the body. HTML5 extraction reads at most 2 MiB of uncompressed UTF-8/ASCII HTML,
then probes the selected media. Multiple media elements or sources are errors;
JavaScript players, lazy-loading attributes, and embedded site players are not
supported. A download makes another GET. There is a 15-second connection timeout
and a five-minute total timeout per request. Simulation and JSON extraction still make
network requests. MIME identification is not media-container verification.

The library exposes `YoutubeDl`, `VideoInfo`, and an `Extractor` interface. See
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
