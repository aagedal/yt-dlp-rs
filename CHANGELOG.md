# Changelog

Notable changes in each release. A release tag publishes its section here
above GitHub's list of merged pull requests. Releases before 1.0 are
described only on the [GitHub releases](https://github.com/aagedal/yt-dlp-rs/releases)
page.

Upstream baseline: yt-dlp **2026.10.26** (`f264a6cb`).

## [1.0.0-rc.1]

The release candidate for 1.0, the first stable release. It runs against
the live canaries before 1.0.0 is tagged. From 1.0 releases follow semantic
versioning; the README's
[Stability](https://github.com/aagedal/yt-dlp-rs/blob/main/README.md#stability)
section says what that covers.

### Release

- Release binaries ship `THIRD-PARTY-NOTICES.txt` with the licenses of the
  statically linked dependencies (BoringSSL among them).
- Linux binaries need glibc 2.39 (Ubuntu 24.04, Debian 13 or later); they
  are built on Ubuntu 24.04 runners.
- The release workflow refuses a tag that does not match Cargo.toml's
  version or whose commit has no passing CI run.
- The crate is not published on crates.io; use it as a Git dependency
  (docs/LIBRARY.md). `YoutubeDl::register_extractor` is now
  `#[doc(hidden)]`, like the `__private::Extractor` it takes.
- The CLI output contract has a [versioning policy](https://github.com/aagedal/yt-dlp-rs/blob/main/docs/CLI_CONTRACT.md#versioning).

### Fixed since 0.10.0-beta.1

- Downloads: a run killed with SIGKILL mid-fragment no longer leaves a
  `.part` file that a later `--continue` resumes into a corrupt result
  (fragments are appended only when whole); a failed append truncates back to
  the last whole fragment.
- `-N` and indexed `--download-sections` no longer hang when a worker
  prints a retry or skip message.
- An empty `200` response is `ERROR: Did not get any data blocks` instead of
  an empty file, and HLS/ISM fragments are skipped like upstream.
- `--all-formats`, `--hls-prefer-ffmpeg`/`--hls-prefer-native` and
  `--compat-options no-live-chat` were accepted and ignored; they now work.
- `--min-filesize`/`--max-filesize` apply to each fragment, and a download
  whose fragments were all skipped fails with
  `The downloaded file is empty`.
- `--extractor-retries` has upstream's meaning (retries inside the
  YouTube, TikTok and SponsorBlock extractors; default 3, `infinite`
  accepted) instead of re-running whole extractions.
- `--write-comments`/`--get-comments` warn for SoundCloud, Bilibili,
  GameJolt and Rokfin, whose comment extraction is not ported, instead of
  writing nothing silently.
- `-x` works without ffprobe (through `ffmpeg -i`, like upstream).
- `--check-formats` tests loaded HLS formats through their playlists.
- An upcoming YouTube live stream with `--ignore-no-formats-error` is
  extracted without formats instead of failing.
- A `--keep-partial-on-stop` stop before any data arrived ends as
  `Interrupted by user` and leaves no files.
- Network read failures and filesystem errors use upstream's wording and
  name the path; `--verbose` reports the real extractor count.
- `file:` format URLs need `--enable-file-urls`; `--write-all-thumbnails`
  names stay in the output directory; response cookies for a whole
  top-level domain are rejected; the local relay for a `socks4://USER@`
  proxy no longer accepts connections from other local processes.
- Several parser panics found by fuzzing (WebVTT, remote numbers, the JS
  interpreter's unbounded work) fail cleanly.
- Dependencies were updated past RustSec advisories (rustls-webpki,
  webpki-roots, zstd), and CI checks advisories.
