# Changelog

Notable changes in each release. A release tag publishes its section here
above GitHub's list of merged pull requests. Releases before 1.0 are
described only on the [GitHub releases](https://github.com/aagedal/yt-dlp-rs/releases)
page.

Upstream baseline: yt-dlp **2026.10.26** (`f264a6cb`).

## [Unreleased]

### Fixed since 1.0.0-rc.2

- `--section-timecode`: an MP4/MOV section whose requested start fell in
  the second half of a frame showed in DaVinci Resolve one frame later than
  its timecode (Resolve rounds the cut's start to the nearest frame), and
  its audio started up to a frame before its video. The cut now starts just
  before the first frame, so the timecode, the first frame and the audio
  agree; checked against full downloads in Resolve at 29.97 and 59.94 fps.

## [1.0.0-rc.2]

The second release candidate for 1.0, with the fixes from checking rc.1:
review of the last fixes before it, a live run of the release binaries
against upstream, and Aagedal Media Converter's growing-file recordings in
DaVinci Resolve.

### Release

- The Windows binary no longer needs the Visual C++ Redistributable.
- CI and the release workflow use the Node 24 versions of their GitHub
  Actions.

### Fixed since 1.0.0-rc.1

- `--growing-file` with `--embed-metadata`: the tags are written by the
  recording itself, so the file is no longer replaced by a rewritten,
  unfragmented one when it ends (DaVinci Resolve then showed a timeline
  clip with the wrong duration or timecode). Other postprocessors leave the
  recording at its name while they run, instead of moving it into a hidden
  workspace; their result then replaces it.
- A second signal while FFmpeg finishes a growing recording (a stop, then a
  cancel) waits for it, so the file is finished and its recording mark
  removed, instead of exiting at once. FFmpeg exiting 255 (stopped by a
  group signal this process ignored, as in a background job) ends the
  recording as a stop instead of an error.
- `--live-from-start --growing-file` on a YouTube stream older than 120
  hours: the timecode is where the download begins, not the stream's
  start. FFmpeg ended by a signal reports Python's negative return code.
- A fragment download stopped or failing right after a fragment was
  appended (a Ctrl+C at that moment among them) no longer lets `--continue`
  append that fragment a second time.
- `-x` with `--ffmpeg-location DIR` where `DIR` has no ffprobe uses
  `ffmpeg -i` like upstream instead of failing with a filesystem error.
- A chunked resume (`--http-chunk-size`, YouTube's default) that the server
  answers with the whole file reports a later read failure correctly
  (debug builds panicked).
- Bandcamp's album artist is HTML-unescaped like upstream.
- Response cookies set for another site's domain (Instagram sends
  Threads' cookies) are dropped silently like upstream, without the
  `Some cookies were ignored` warning.

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
