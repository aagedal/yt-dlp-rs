# yt-dlp-rs

A native Rust port of [yt-dlp](https://github.com/yt-dlp/yt-dlp), with a few
extensions for video editing workflows. Extraction and downloads run entirely
in Rust: the binary never invokes Python or an installed yt-dlp.

The port follows upstream release **2026.10.26** (pinned in
[upstream/manifest.json](upstream/manifest.json)) and covers over 900 extractors
([docs/SITES.md](docs/SITES.md), or `yt-dlp-rs --list-extractors`). The
tier-1 sites (YouTube, Instagram, TikTok, X, Facebook, Reddit, Vimeo,
Twitch, Bluesky, SoundCloud, Bandcamp) are run daily against the live site
(a site behind a bot check or login is reported as blocked rather than
checked; see [Live canaries](#live-canaries)); the rest are checked against
pinned upstream output on saved responses and are best effort (see
[supported sites](docs/COMPATIBILITY.md#supported-sites)).
That is tested behavior, not a general promise of yt-dlp compatibility:
options this port does not implement are rejected with an error, or
accepted with a `WARNING: … is not supported natively and is ignored`
(such as `--plugin-dirs` or unported `--extractor-args`); the few accepted
without any effect are listed in the [CLI inventory](docs/COMPATIBILITY.md#cli).

## Install

Binaries are attached to the
[GitHub releases](https://github.com/aagedal/yt-dlp-rs/releases) as
`yt-dlp-rs-vX.Y.Z-TARGET.tar.gz` (`.zip` on Windows), with `SHA256SUMS`
and the third-party license notices:

| Platform | Target | Needs |
| --- | --- | --- |
| Linux x86-64 | `x86_64-unknown-linux-gnu` | glibc 2.39 (Ubuntu 24.04, Debian 13, Fedora 40 or later) |
| Linux ARM64 | `aarch64-unknown-linux-gnu` | glibc 2.39 |
| macOS Apple silicon | `aarch64-apple-darwin` | macOS 11 |
| macOS Intel | `x86_64-apple-darwin` | macOS 10.12 |
| Windows x64 | `x86_64-pc-windows-msvc` | Windows 10; on ARM64 under emulation |

The binaries are not signed or notarized. macOS blocks a binary downloaded
with a browser until its quarantine attribute is removed
(`xattr -d com.apple.quarantine yt-dlp-rs`), and Windows SmartScreen may
warn before the first run.

To build from source you need current stable Rust (there is no fixed
minimum version; the code follows the latest stable release):

```sh
cargo install --locked --git https://github.com/aagedal/yt-dlp-rs --tag vX.Y.Z
```

The default build uses rustls. `cargo build --locked --features impersonate`
adds browser impersonation (TLS/HTTP2 fingerprints, as upstream's
`curl_cffi`) through `wreq`; it builds BoringSSL from source and needs CMake
and a C/C++ compiler (on Windows, MSVC and LLVM's libclang). Every release
binary includes it. A native `aarch64-pc-windows-msvc` build of the feature
fails in `btls-sys` 0.5.6 (BoringSSL links x86-64 assembly); on Windows on
ARM use the x64 release, which runs under emulation.

Optional tools, found on `PATH` like upstream (or with `--ffmpeg-location`
and `--ffprobe-location`):

- **FFmpeg/FFprobe** for merging, live streams, sections, audio extraction,
  remuxing, subtitles, thumbnails and metadata embedding.
- **A JavaScript runtime** (deno by default, or `--js-runtimes node`,
  `quickjs` or `bun`) for YouTube's signature and `n` challenges. Without one,
  YouTube falls back to fewer formats, with upstream's warning.

Options can also come from configuration files like upstream's, named
`yt-dlp-rs` (for example `~/.config/yt-dlp-rs/config`); see `--ignore-config`
and `--config-locations`.

## Usage

The CLI follows upstream's options and output:

```sh
yt-dlp-rs 'https://www.youtube.com/watch?v=VIDEO_ID'
yt-dlp-rs -F 'https://www.youtube.com/watch?v=VIDEO_ID'
yt-dlp-rs -f 'bv[height<=1080]+ba/b' -o '%(title)s.%(ext)s' URL
yt-dlp-rs -x --audio-format mp3 URL
yt-dlp-rs --write-subs --sub-langs en --embed-subs URL
yt-dlp-rs --cookies-from-browser firefox URL
yt-dlp-rs --download-sections '*10:00-12:30' --section-timecode URL
yt-dlp-rs --live-from-start --growing-file 'https://www.youtube.com/watch?v=LIVE_ID'
```

The [compatibility inventory](docs/COMPATIBILITY.md) documents every
supported option and where it differs from upstream.

Programs that run the CLI should parse the machine-readable output in the
[CLI output contract](docs/CLI_CONTRACT.md) (`--progress-template` JSON,
`--print` stages, exit codes, stop signals), which works the same with
yt-dlp. The human-readable lines follow upstream but are not a stable
interface.

## Stability

From 1.0, releases follow semantic versioning. Within a major version:

- **Stable:** the machine-readable CLI output in the
  [CLI output contract](docs/CLI_CONTRACT.md#versioning) (what its tests
  cover), exit codes and stop signals; the names and meaning of the options
  under [Extensions in this fork](#extensions-in-this-fork); and the library
  API described in [docs/LIBRARY.md](docs/LIBRARY.md#stability), checked
  by `cargo-semver-checks` in CI.
- **Not covered:** human-readable and `--verbose` output; the full
  `-j`/`%(info)j` dict beyond named fields and its key order; site
  behavior (a site can break whenever it changes, and tier 2 is best
  effort); behavior that changes because the upstream baseline advances
  (each release names its baseline); platforms other than the release
  targets; the hidden `cli` and `__private` library modules.

Site fixes and upstream ports ship as minor or patch releases, as needed.
`-U` stays an error: install a newer release, or update through the
application that bundles yt-dlp-rs.

## Extensions in this fork

These options are not in upstream yt-dlp:

- **`--growing-file`** records a live stream into one fragmented QuickTime
  `.mov` at its final name, instead of `.part` files and a merge at the end.
  Editors can import the file while it is still being written. FFmpeg adds a
  time-of-day timecode track: the stream's start time with
  `--live-from-start`, when known. The progress line also shows how much has
  been recorded (`, recorded HH:MM:SS`, or `progress.recorded_seconds` in
  `--progress-template`). On macOS the file carries Blackmagic's
  `com.blackmagicdesign.metadata:recording` attribute while it grows, which
  DaVinci Resolve reads at import to treat it as a recording clip. The
  attribute is removed when the recording ends, including on SIGINT or SIGTERM
  (which finish the file; a second signal waits for that), but not if the
  process is killed. Postprocessors such as `--embed-metadata` then work on a
  link of the recording, which stays at its name until their result (a
  regular, unfragmented file) replaces it. H.264/AAC formats
  are preferred after any `-S` fields. MOV cannot hold AV1, VP9, Opus or FLAC
  (FFmpeg refuses them: "av1 only supported in MP4"), so when those are
  selected the recording is a fragmented `.mp4` instead, which is also the
  container Resolve expects AV1 in.
- **`--keep-partial-on-stop`** makes SIGINT or SIGTERM stop a download and
  keep what it has as a usable file at the final name, instead of `.part`
  files. HLS/DASH downloads keep whole fragments, and separate video and
  audio streams are merged as far as they got (audio is fetched first, so a
  stop mid-video still has audio, cut to the video's length). A fragmented MP4 cut inside a
  fragment is trimmed to whole fragments first, and a merge that does not read
  back keeps the unmerged streams instead. An app can offer Stop (keep)
  next to Cancel (delete) this way; `--print after_move:filepath` reports the
  kept file.
- **`--download-sections` for adaptive HTTPS formats** fetches only the
  segments covering the section, located through the format's MP4 `sidx` or
  WebM `Cues` index, in 10 MiB ranges (the video and audio streams in
  parallel). Upstream instead has FFmpeg read the whole stream, which runs at
  about twice playback speed. The cut uses
  the same FFmpeg arguments and was checked packet-identical to upstream's
  output. Formats without an index, servers without range support and
  `--downloader ffmpeg` keep upstream's behavior.
- **`--section-timecode`** gives a section the timecode its first frame has in
  the whole video, so clips line up with full downloads in an editor. It needs
  ffprobe and an MP4/MOV result: use `--remux-video mp4` for WebM sections.
- **`--ffprobe-location PATH`** picks the ffprobe executable separately
  (upstream takes it from `--ffmpeg-location` or `PATH` only).
- **`--youtube-js-runtime PATH`** names the Deno 2.3+ or Node.js 22+
  executable for YouTube's challenges, instead of the `--js-runtimes` list.

Two download paths also go beyond upstream without an option: live HLS is
recorded natively when FFmpeg is not available (upstream requires it), and
DASH formats with a single indexed file (SegmentBase) download as byte-range
fragments from its `sidx`/`Cues` index instead of in one request.

## Supported sites

YouTube (videos, Shorts, live streams, playlists, mixes, channels, search,
hashtags, clips, comments and YouTube Music) is extracted like upstream,
including PO-token policies, age-gate fallbacks and account cookies. It was
also spot-checked against the live site.

The largest social networks were last compared live with upstream
(2026.10.21, on 2026-10-23) and matched (same formats and metadata, or the
same error):

| Network | Covered | Notes |
| --- | --- | --- |
| YouTube | videos, live, Shorts, playlists, channels, search, Music | PO tokens and a JS runtime as upstream |
| Facebook | videos, reels, watch parties, ad library, redirects | comments not supported |
| Instagram | posts, reels, carousels, stories, highlights, tags, users | stories and most listings need `--cookies` |
| TikTok | videos, subtitles, users, collections, live | sound/effect/tag lists (broken upstream) not ported |
| Douyin | videos | needs fresh browser cookies, as upstream |
| X/Twitter | posts, cards, broadcasts, Spaces, `t.co`, Periscope | Periscope RTMP unsupported |
| Telegram | public post embeds | posts without their own video are an error |
| Snapchat | Spotlight | |
| Weibo | posts and `weibo.com/tv` videos | user pages not ported |
| Pinterest | pins | boards and collections not ported |
| Reddit | posts | some posts need cookies, as upstream |
| LinkedIn | posts | learning courses not ported |
| XiaoHongShu (RedNote) | video notes | |
| Bilibili | videos | bangumi, spaces and audio not ported |
| VK | videos, clips, wall posts, user videos, VK Play, logins | other embedded players are unsupported |

WhatsApp, WeChat, Messenger and Kuaishou have no upstream extractor.

Beyond these, the port includes Vimeo, Dailymotion, Twitch, Kick, SoundCloud,
Bandcamp, Tumblr, Bluesky, Niconico, PeerTube, Brightcove, Kaltura, JW Player,
Wistia, the major European public broadcasters (NRK, SVT, DR, Yle, ARD, ZDF,
Arte, France Télévisions) and several hundred smaller sites. The full list,
with each extractor's tier, is in [docs/SITES.md](docs/SITES.md). Other URLs go through the
generic extractor: direct media, HLS and DASH manifests, HTML5
`<video>`/`<audio>` pages, JSON-LD, JW Player, video.js and KVS players,
`og:video`/`twitter:player` metadata, RSS, SMIL and XSPF, and the embedded
players of supported sites.

## Not supported

- DRM-protected media, and native HLS SAMPLE-AES decryption (like upstream,
  such playlists are delegated to FFmpeg).
- Python plugins, including PO-token provider plugins (pass `po_token` with
  `--extractor-args` instead). `--use-postprocessor` supports only `Exec`.
- Generic-page features beyond HTML5 media elements and known embeds, such as
  JavaScript-built players.
- Chromium cookies on Windows (DPAPI) and from the GNOME keyring.
- The macOS and Windows system proxy settings (proxy environment variables
  such as `https_proxy` and `no_proxy` work as upstream).
- On Windows: password and two-factor prompts echo the input (upstream
  reads them without echo), and `--growing-file` with `--live-from-start`
  (it needs named pipes). Stopping a run there uses Ctrl+Break
  instead of SIGTERM (see [CLI contract](docs/CLI_CONTRACT.md#windows)).
- Upstream options this port has not implemented: they are errors, or
  warnings that say the option is ignored.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 -m unittest discover -s tools -p 'test_*.py' -v
```

Python 3.10+ and Git are needed only for the development tools. The HTTP
integration tests bind localhost sockets; no test contacts a live site. CI runs
the whole suite on Linux, macOS and Windows (GitHub's hosted runners). On
Windows the tests' FFmpeg and downloader stand-ins are shell scripts run by
Git for Windows' `sh.exe` (or `YT_DLP_RS_TEST_SH`); the few tests that need
POSIX pseudo-terminals, symlinks, named pipes or non-UTF-8 paths are Unix-only.
CI also fails on RustSec advisories for any feature's dependencies
(`cargo deny --all-features check advisories`) and on a dependency license
that `tools/notices/about.toml` does not accept.

To release, update `version` in Cargo.toml and the CHANGELOG, merge, and
once CI has passed on that commit push a `vX.Y.Z[-PRE]` tag for it. The tag
runs `.github/workflows/release.yml`, which refuses a tag that does not
match the version or a commit without a passing CI run, and publishes the
CHANGELOG section with the binaries. A tag with a pre-release suffix
publishes a pre-release.

Upstream changes are tracked against the pinned baseline:

```sh
git clone https://github.com/yt-dlp/yt-dlp.git .upstream/yt-dlp
python3 tools/upstream_report.py .upstream/yt-dlp > upstream-report.json
```

Use a full clone so the baseline exists. The report lists changed files,
commits, affected Rust components and unmapped files. It does not port code or
advance the baseline; [the upstream workflow](docs/UPSTREAM.md) describes how
changes are ported.

### Live canaries

`.github/workflows/canary.yml` runs daily (and by hand, optionally for some
sites) against the tier-1 sites in `tools/canary_sites.json`: the
Media Converter site list. For each URL, `tools/canary.py` runs `-j` and checks
the key fields (id, extractor key, title, duration, formats), compares them
with the pinned upstream release, downloads a few seconds with
`-f bestvideo+bestaudio/best` and probes the file with ffprobe. A failed URL
is retried once. Sites end up `pass`, `broken`, `diverged` (differs from
upstream), `blocked` (bot check, PO token, login wall, IP block or rate limit;
the patterns are listed in the script) or `skip` (needs a
`CANARY_COOKIES_<SITE>` secret holding a cookies file, and it is not set).
Broken or diverged sites open or update one issue labelled `canary`, which is
closed when none are left. The workflow is never a required check, and the
test suite never contacts live sites. To run it locally:

```sh
cargo build --release --features impersonate
python3 tools/canary.py --sites youtube,vimeo --upstream .upstream/yt-dlp
```

More documentation:

- [Compatibility inventory](docs/COMPATIBILITY.md): every supported option and
  its exact behavior.
- [CLI output contract](docs/CLI_CONTRACT.md): the machine-readable output
  programs can rely on.
- [Supported sites](docs/SITES.md): every extractor and its tier.
- [Library API](docs/LIBRARY.md): using the `yt_dlp_rs` crate as a Git
  dependency.
- [Changelog](CHANGELOG.md): what changed in each release.
- [Upstream workflow](docs/UPSTREAM.md): how upstream changes are ported.
- [Benchmarks](docs/BENCHMARKS.md): local comparisons with upstream and how to
  rerun them.
- [Roadmap](docs/ROADMAP.md): architecture and upcoming work.

Licensed under the Unlicense, matching this repository's original license.
