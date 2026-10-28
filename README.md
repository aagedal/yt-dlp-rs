# yt-dlp-rs

A native Rust port of [yt-dlp](https://github.com/yt-dlp/yt-dlp), with a few
extensions for video editing workflows. Extraction and downloads run entirely
in Rust: the binary never invokes Python or an installed yt-dlp.

This is a beta. The port follows upstream release **2026.10.26** (pinned in
[upstream/manifest.json](upstream/manifest.json)) and covers over 550 extractors
(`yt-dlp-rs --list-extractors`). Each one is checked against pinned upstream
output on saved responses. That is tested behavior, not a general promise of
yt-dlp compatibility: options this port does not implement are rejected with
an error rather than ignored.

## Install

Pre-release binaries for Linux, macOS and Windows are attached to the
[GitHub releases](https://github.com/aagedal/yt-dlp-rs/releases). To build
from source you need current stable Rust:

```sh
cargo install --locked --path .
```

The default build uses rustls. `cargo build --locked --features impersonate`
adds browser impersonation (TLS/HTTP2 fingerprints, as upstream's
`curl_cffi`) through `wreq`; it builds BoringSSL from source and needs CMake
and a C/C++ compiler.

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
  (which finish the file), but not if the process is killed. H.264/AAC formats
  are preferred after any `-S` fields. MOV cannot hold AV1, VP9, Opus or FLAC
  (FFmpeg refuses them: "av1 only supported in MP4"), so when those are
  selected the recording is a fragmented `.mp4` instead, which is also the
  container Resolve expects AV1 in.
- **`--keep-partial-on-stop`** makes SIGINT or SIGTERM stop a download and
  keep what it has as a usable file at the final name, instead of `.part`
  files. HLS/DASH downloads keep whole fragments, and separate video and
  audio streams are merged as far as they got (audio is fetched first, so a
  stop mid-video still has the whole audio). An app can offer Stop (keep)
  next to Cancel (delete) this way; `--print after_move:filepath` reports the
  kept file.
- **`--download-sections` for adaptive HTTPS formats** fetches only the
  segments covering the section, located through the format's MP4 `sidx` or
  WebM `Cues` index, in parallel 10 MiB ranges. Upstream instead has FFmpeg
  read the whole stream, which runs at about twice playback speed. The cut uses
  the same FFmpeg arguments and was checked packet-identical to upstream's
  output. Formats without an index, servers without range support and
  `--downloader ffmpeg` keep upstream's behavior.
- **`--section-timecode`** gives a section the timecode its first frame has in
  the whole video, so clips line up with full downloads in an editor. It needs
  ffprobe and an MP4/MOV result: use `--remux-video mp4` for WebM sections.

## Supported sites

YouTube (videos, Shorts, live streams, playlists, mixes, channels, search,
hashtags, clips, comments and YouTube Music) is extracted like upstream,
including PO-token policies, age-gate fallbacks and account cookies. It was
also spot-checked against the live site.

The largest social networks were compared live with upstream 2026.10.21 on
2026-10-23 and matched (same formats and metadata, or the same error):

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
Arte, France Télévisions) and several hundred smaller sites. Run
`yt-dlp-rs --list-extractors` for the full list. Other URLs go through the
generic extractor: direct media, HLS and DASH manifests, and HTML5
`<video>`/`<audio>` pages.

## Not supported

- DRM-protected media, and HLS SAMPLE-AES.
- Python plugins, including PO-token provider plugins (pass `po_token` with
  `--extractor-args` instead). `--use-postprocessor` supports only `Exec`.
- Generic-page features beyond HTML5 media elements and known embeds, such as
  JavaScript-built players.
- Chromium cookies on Windows (DPAPI) and from the GNOME keyring.
- Resuming partial downloads across separate runs: an interrupted transfer
  resumes only within the same run.
- Upstream options this port has not implemented: they are errors, never
  silently ignored.

## Development

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked
python3 -m unittest discover -s tools -p 'test_*.py' -v
```

Python 3.10+ and Git are needed only for the development tools. The HTTP
integration tests bind localhost sockets; no test contacts a live site. CI runs
on Linux, macOS and Windows. Pushing a `vX.Y.Z[-PRE]` tag runs
`.github/workflows/release.yml`, and a tag with a pre-release suffix publishes
a pre-release.

Upstream changes are tracked against the pinned baseline:

```sh
git clone https://github.com/yt-dlp/yt-dlp.git .upstream/yt-dlp
python3 tools/upstream_report.py .upstream/yt-dlp > upstream-report.json
```

Use a full clone so the baseline exists. The report lists changed files,
commits, affected Rust components and unmapped files. It does not port code or
advance the baseline; [the upstream workflow](docs/UPSTREAM.md) describes how
changes are ported.

More documentation:

- [Compatibility inventory](docs/COMPATIBILITY.md): every supported option and
  its exact behavior.
- [Library API](docs/LIBRARY.md): using the `yt_dlp_rs` crate.
- [Roadmap](docs/ROADMAP.md): architecture and upcoming work.

Licensed under the Unlicense, matching this repository's original license.
