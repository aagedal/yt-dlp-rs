# Library API

The `yt_dlp_rs` crate exposes the extraction and download pipeline the CLI
uses. The CLI and its machine-readable output ([CLI_CONTRACT.md](CLI_CONTRACT.md))
are the primary contract; Aagedal Media Converter runs the binary. The library
is secondary and unstable until 1.0 (see [Stability](#stability)). See the
[compatibility inventory](COMPATIBILITY.md) for the behavior behind each call.

```rust,no_run
use std::ops::ControlFlow;
use yt_dlp_rs::{DownloadControl, DownloadOptions, Error, Interrupt, YoutubeDl};

fn main() -> yt_dlp_rs::Result<()> {
    let client = YoutubeDl::new()?.with_ffmpeg("ffmpeg");
    let result = client.extract_result_with_format("https://example.com/clip", Some("best"))?;
    for info in result.into_entries() {
        // Another thread may call `stop()` (keep the partial file) or `cancel()`.
        let interrupt = Interrupt::new();
        let mut control = DownloadControl::default();
        control.interrupt = Some(interrupt.clone());
        let mut options = DownloadOptions::default();
        options.retries = 1;
        let path = format!("{}.{}", info.id, info.ext);
        match client.download_with_control(&info, &path, options, &control, |progress| {
            eprintln!("{:?}: {} bytes", progress.phase, progress.downloaded_bytes);
            ControlFlow::Continue(())
        }) {
            Ok(bytes) => println!("{path}: {bytes} bytes"),
            Err(Error::DownloadStopped { path }) => println!("stopped: {}", path.display()),
            Err(error) => return Err(error),
        }
    }
    Ok(())
}
```

## Stability

The public API is what rustdoc shows for the crate root and `output`. Two
modules are `#[doc(hidden)]` and not part of it: `cli` (the binary's
implementation; `src/main.rs` only calls `cli::main`) and `__private`
(helpers the integration tests use, including the `Extractor` trait and
`YoutubeDl::register_extractor`, whose hooks follow upstream's extractor
internals and expose the reqwest client). Anything there can change in any
release.

- **Growing types are `#[non_exhaustive]`**: the option, control, progress,
  result and info structs (`HttpOptions`, `YoutubeOptions`,
  `ExtractionOptions`, `DownloadOptions`, `DownloadControl`,
  `PostprocessOptions`, `SubtitleEmbedding`, `SubtitleFile`, `Manifest`,
  `RetrySleep`, `DownloadProgress`, `FragmentBytes`, `PlaylistCounts`,
  `ProcessedDownload`, `AudioDownload`, `ConvertedSubtitle`,
  `SubtitleSelection`, `VideoInfo`, `PlaylistInfo`, `VideoFormat`,
  `Fragment`, `Metadata`, `Thumbnail`, `Chapter`, `DownloaderOptions`,
  `SubtitleFormat`) and the enums `Error`, `DownloadPhase`, `StreamKind`,
  `PartFiles`, `MergeFiles`, `ManifestKind` and `VideoMode`. Build a struct
  from `Default` (or its constructor: `Manifest::new`/`inline`,
  `Fragment::new`, `SubtitleFile::new`) and assign its public fields, and
  give a `match` on these enums a wildcard arm, so that a new field or
  variant is not a breaking change. `ExtractResult`, `ByteRange`,
  `ByteIndex`, `HlsAes128` and `PyNumber` are complete and stay exhaustive.
  Types with private fields (`AudioOptions`, `VideoOptions`,
  `SubtitleConversion`, `SubtitleOptions`, `TemplateOptions`, `CookieJar`,
  `Interrupt`, `RequestScope`, ...) are built through their methods.
- **One error type**: every call returns `yt_dlp_rs::Result<T>` with
  `yt_dlp_rs::Error`. Its `Display` text is the CLI's upstream message, so
  match variants instead of parsing text.
- **Public dependencies**: `url` 2 (`Url`), `serde_json` 1 (the info structs'
  `extra` maps and `http_headers`) and `reqwest` 0.12 (`Error::Http` and
  `Error::HttpRetryAfter` carry a `reqwest::Error`, for its status and
  kind). A major update of one of them is a breaking release.
- **Process-wide settings**: `no_playlist`, `age_limit`, `get_comments` and
  `skip_download` (`YoutubeOptions`) and `geo_verification_proxy`
  (`HttpOptions`) are stored process-wide when a `YoutubeDl` is built, so
  clients with different values interfere; the last one built wins.
- **`cargo-semver-checks`**: CI's `semver` job (`tools/semver_check.py`)
  compares the library with the latest `v*` release tag, default features
  only (`impersonate` adds no public items). From 1.0 on a breaking change
  fails the job unless `Cargo.toml`'s major version is above the tag's;
  before 1.0 the job only reports it (a warning), since a 0.x release may
  break the API. Run it locally with
  `cargo install cargo-semver-checks --locked` and
  `python3 tools/semver_check.py`.

## Reference

The crate root exposes `YoutubeDl`, `ExtractResult`, `PlaylistInfo`, `VideoInfo`,
`VideoFormat`, `StreamKind`, `Fragment`, `ByteRange`, `HlsAes128`, `DownloadOptions`, `DownloadProgress`, `DownloadPhase`,
`ExtractionOptions`, `YoutubeOptions`, `HttpOptions` and the types below.
Setting `YoutubeOptions::js_runtime` to `Some(path)` and passing the options to
`YoutubeDl::with_youtube_options` configures optional YouTube signature/n
solving; `YoutubeDl::new()` keeps it disabled.
`include_translated_subtitles = true` additionally exposes translated manual captions
in `automatic_captions`; the CLI enables this with `--write-auto-subs` or `--list-subs`.
ASR translations are discovered independently, without fetching their payloads.
`YoutubeDl::with_options(youtube, http)` also takes `HttpOptions` (for example
`http.user_agent = Some("My media client".into())`); `HttpOptions.headers` holds name/value pairs.
`HttpOptions::from_cli(user_agent, referer, &headers)` parses repeated FIELD:VALUE
strings with the same normalization and validation as the binary.
`CookieJar::load(path)` and `save(path)` expose explicit Netscape persistence;
`HttpOptions.cookies` accepts a shared `Arc<CookieJar>`, or the client creates a
fresh in-memory jar. `YoutubeDl::cookie_jar()` returns it for reuse or saving.
`HttpOptions.proxy` accepts an explicit URL or empty string for direct routing;
`None` preserves the existing reqwest environment/system routing policy.
`HttpOptions::validate()` checks configuration without network/DNS requests.
Cookie files are credential state: an explicit `--cookies` path is updated after
requests even in JSON/listing/simulation modes and after extraction failures.
`YoutubeDl::new()?.with_ffmpeg(path)` configures optional local merging.
`extract_result_unselected` / `extract_result_unselected_with_options` expose all
eligible alternatives without library selection; `select_result` applies configured
selection atomically before downloading.
`output::render_template_with_options` accepts `TemplateOptions` for the missing-value
placeholder; `render_template` keeps the default `NA`.
`output::render_formats` renders their inventory, while `validate_format_selector`
checks supported syntax without network or availability checks.
Use `extract_result` or
`extract_result_with_format(url, Some("1"))` for video/playlist results. A format
ID applies to every playlist entry independently; each must offer that ID.
`PlaylistSelection::new(items, start, end, reverse)` parses item selection;
`apply` filters/reorders fully extracted playlist entries and annotates their
original index/count and selected order/count. Single videos are unaffected.
Library extraction methods do not implicitly apply CLI playlist options.
`SubtitleOptions::new(languages, format)?.select(&info.subtitles)` returns selected
subtitle metadata and warnings. `select_sources(&info.subtitles,
&info.automatic_captions, write_normal, write_automatic)` selects enabled sources
with manual precedence. `subtitle_paths` plans sanitized sidecar names;
`download_subtitle_with_options` / `download_subtitle_with_progress` copy one
resource with the normal retry/cancellation policy. Selection, writing and archive
recording remain explicit library operations.
`DownloadArchive::load`, `contains` and `record` expose persistence separately;
library callers choose when to filter and record, after successful publication.
`archive_id` returns the supported lowercase extractor-key plus case-sensitive ID.
The existing `extract_info` methods accept single videos and reject playlists.
`--dump-json` emits one video object per entry.
`download_with_options(info, path, options)` with `options.retries = 0` disables
download retries for a library call; `download` uses the default policy.
`download_with_progress(info, path, options, callback)` reports one-based attempts,
stored bytes, optional total bytes, and the current phase. Fragment downloads also report one-based fragment indexes
and counts, with cumulative assembled bytes and a per-fragment attempt number. Return
`std::ops::ControlFlow::Continue(())` to continue or `Break(())` to cancel.
A callback's `Break` is cooperative: it is seen at the next checkpoint, not
during a blocking request, read, or filesystem operation; use an `Interrupt`
(below) to end a transfer blocked on the network. See the compatibility
inventory for checkpoints.
`extract_result_with_options(url, None, options)` with `options.retries = 2`
(`ExtractionOptions`) enables whole-extraction retries; the other extraction
methods keep retries disabled.
`YoutubeDl::with_growing_file(true)` makes `download_with_progress` record live
results as one growing QuickTime file (see `--growing-file` in the
[CLI table](COMPATIBILITY.md#cli)); `YoutubeDl::grows(&info)` tells whether a
result will be recorded that way.
`with_ffprobe`, `with_concurrent_fragments` (`-N`), `with_keep_fragments`
(`-k`), `with_downloader(protocol, ffmpeg)` and `with_live_dash_snapshot`
configure the downloads like their CLI options. `download_processed_with_options`
/ `_with_progress` (`PostprocessOptions`: audio extraction with `AudioOptions`,
remux/recode with `VideoOptions`, subtitle embedding with `SubtitleEmbedding`,
metadata, chapters and thumbnail embedding), `download_audio_with_options` /
`_with_progress` and `convert_subtitle_with_options` / `_with_progress`
(`SubtitleConversion`) run the local FFmpeg processors.

## Call controls, request scopes and inline manifests

`YoutubeDl::with_keep_partial_on_stop(true)` (`--keep-partial-on-stop`) makes
SIGINT/SIGTERM during a download act as `Interrupt::stop()`, and fetches a
merged download's audio-only streams first.

`Interrupt` is a shareable stop/cancel handle (for example a UI's Stop and
Cancel buttons):

- `interrupt.cancel()` aborts: a download returns `Error::DownloadCancelled`
  and removes its private temporary files.
- `interrupt.stop()` ends early but keeps what was downloaded: the data is
  published at the destination and the call returns
  `Error::DownloadStopped { path }`. A whole-file transfer keeps its received
  bytes; HLS/DASH fragment downloads keep whole fragments only (a native
  live HLS recording ends with the segments so far); a merged
  download merges the streams so far with FFmpeg (the last one partial),
  ending with the shortest stream (`-shortest`);
  FFmpeg-based downloads ask FFmpeg to finish (`q`). A stop before any data
  arrived is a cancel. A fragmented MP4 cut inside a fragment is trimmed to
  its whole fragments, and a progressive MP4 whose `moov` describes more
  than arrived is stream-copied to the samples present. A stopped MP4/MOV
  merge that does not read back (or fails) keeps the unmerged streams at
  their `NAME.fID.EXT` names instead, and `path` names the video stream.
  `download_processed_with_progress` skips the processors on a stop and
  publishes the partial file at the destination.
- During extraction either request ends the call with
  `Error::ExtractionCancelled` before its next request.

Checks run before each request and at each progress checkpoint. A request
blocked on the network (DNS, connect, TLS handshake, response headers or a
body read, including fragment workers) and retry or `--sleep-*` waits end
within about 100 ms, without shortening the socket timeout of a healthy
transfer: while an `Interrupt` is installed, requests are sent on a helper
thread and bodies stream through a small queue (see
[Interruptible transfers](COMPATIBILITY.md#interruptible-transfers)).
Filesystem operations are not interrupted.

`RequestScope::new(&url)` holds headers and cookies for one origin (scheme,
host and port). `.header(name, value)` values are sent only on requests to
that origin, so a manifest's authorization does not follow segment, key or
variant URLs on other hosts. `.cookie(name, value)` adds a host-only session
cookie and `.set_cookie("name=value; Domain=...; Path=...")` any cookie the
scope's URL could set; they follow RFC 6265 domain/path matching, are added to
the client's jar only for the duration of the call, never saved, and never
shown in info dicts. Values are redacted from `Debug`. Note that reqwest keeps
custom (non-`Authorization`) headers on a redirect to another host.
`CookieJar::set_cookie(&url, "name=value; ...")` stores a cookie permanently,
for copying another application's cookies into the shared jar.

Pass both through the `interrupt` and `scope` fields of `ExtractionOptions` to
`extract_result_with_options`, `extract_result_unselected_with_options` or
`extract_manifest`, and of `DownloadControl` to
`download_with_control(info, path, options, &control, callback)`, which
otherwise behaves like `download_with_progress`.

`YoutubeDl::extract_manifest(&Manifest, options)` turns an HLS or DASH manifest
into a processed, unselected video result for `select_result` and the download
calls. `Manifest::new(ManifestKind::Hls, url)` fetches it;
`Manifest::inline(ManifestKind::Dash, base_url, text)` parses the given text
with `base_url` resolving its relative URIs. `id`, `title` and `webpage_url`
override the defaults (the manifest file name's stem and the manifest URL). A
master playlist's variant playlists are fetched (with the scope). Inline
manifests download through the native downloaders; FFmpeg and external
downloaders read `url` themselves.
