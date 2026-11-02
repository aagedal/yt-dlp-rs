# Road to 1.0, step 8: library API

Survey of 2026-11-02 at `a9137cff` (step 7 merged), written before the
changes below. It is not an upstream port: upstream's Python `YoutubeDL`
API has no Rust equivalent, so this step only decides what the `yt_dlp_rs`
crate promises.

## What was public

`cargo +nightly doc` with `--output-format json` listed 2321 public paths
(not counting methods): 35 modules, 976 structs, 1071 functions, 157 enum
variants, 31 enums, 31 constants, 9 type aliases, 7 statics and 3 traits.
1819 of the paths were `extractor::*` (every site extractor struct and its
helpers), 121 `output::*` (template, progress, print, exec, colour and
encoding internals), 42 `error::*`, 32 `jsinterp::*`; the root re-exported
about 110 items from otherwise private modules. `YoutubeDl` had 91 public
methods. Public error types besides `Error`: `netrc::ParseError`,
`format_select::SelectorError` and `jsinterp::JsError` (`bidi::StartError`
belongs to the binary).

Nearly all of it was public only because `src/main.rs` is a separate crate
root: the binary named the library 1222 times (`yt_dlp_rs::extractor` 914,
`output` 57, `Error` 51, `VideoInfo` 32, `batch` 21, `screen` 18, ...; 86
distinct root items), so everything it touched had to be `pub`, and
whatever those items mention had to be public too.

## Who uses it

- **The CLI** (`src/main.rs`): 86 root items and 52 `YoutubeDl` methods
  that nothing else calls (`announce_download`, `entry_archive_identity`,
  `report_live_dash`, `uses_extractor`, `entry_title`, `fetch_pending`,
  `with_live_dash_snapshot`'s companions such as `live_dash_fails`, the
  `with_*` setters for CLI options like `with_default_search`,
  `with_dynamic_mpd`, `with_test`, ...), plus `total_fragments`,
  `trouble_reported`, `LIVE_DASH_UNSUPPORTED`, `stream_path`,
  `parse_duration`, `SignalInterrupt`, `upstream_shape`, the info-JSON
  writers, the step 7 `FragmentBytes::live` field and whole modules
  (`batch`, `config`, `screen`, `sections`, `metadata_parser`,
  `manifest_args`, `pp_args`, `sponsorblock`, `geo`, `cache`, `xattr`,
  `getpass`, `netrc`, `browser_cookies`, `dates`, `impersonate`, ...).
- **Integration tests** (27 of the 98 files in `tests/` link the library;
  the rest run the binary): `YoutubeDl` (extraction, selection, the
  download calls with options/progress/control, audio/video/subtitle
  processing, `cookie_jar`, `with_ffmpeg`/`with_ffprobe`,
  `with_concurrent_fragments`, `with_keep_fragments`, `with_downloader`,
  `register_extractor`), `Error` (21 variants matched), `ExtractResult`,
  `VideoInfo`, `Fragment`, `ByteRange`, `ByteIndex`, `PyNumber`,
  `StreamKind`, the option structs (`DownloadOptions` in 11 files,
  `ExtractionOptions`, `HttpOptions`, `YoutubeOptions`, `DownloadControl`,
  `PostprocessOptions`, `AudioOptions`, `VideoOptions`, `VideoMode`,
  `SubtitleConversion`, `SubtitleEmbedding`, `SubtitleFile`, `PartFiles`,
  `MergeFiles`), `DownloadProgress`/`DownloadPhase`, `Interrupt`,
  `RequestScope`, `Manifest`/`ManifestKind`, `CookieJar`, and a few
  helpers: `extractor::{Extractor, DirectHttp, GenericHttp}`,
  `output::{render_formats, render_template, host_path,
  shell_quote_command}`, `container::{sniff, sniff_bytes}`.
- **Fuzz targets** (`fuzz/`): only `fuzz::*`, compiled with
  `--cfg fuzzing` and already `#[doc(hidden)]`.
- **docs/LIBRARY.md**: prose, no compiled examples; there were no doctests.
  It documents roughly the test set above plus `PlaylistSelection`,
  `SubtitleOptions`/`select_sources`/`subtitle_paths`, `DownloadArchive`/
  `archive_id`, `output::render_template_with_options`/`TemplateOptions`
  and `validate_format_selector`.
- **Aagedal Media Converter** runs the binary, so it uses none of this.

Public items leaked crate dependencies: `reqwest::Error` in
`Error::Http`/`Error::HttpRetryAfter`, `reqwest::blocking::Client` and
`login::Session` in the `Extractor` trait, `regex::Regex` in
`ModifyChapters`, `serde_json` maps in the info structs and `url::Url`
throughout.

## Decisions

- **Move the binary into the library.** `src/main.rs` becomes a
  `#[doc(hidden)] pub mod cli` (`src/cli.rs`, the same file moved
  unsplit) and the binary only calls `yt_dlp_rs::cli::main()`. The CLI then
  reaches internals as `crate::...`, so every CLI-only module and item
  becomes crate-private instead of hidden-but-public, and the CLI keeps
  constructing `#[non_exhaustive]` structs with literals. A hidden
  `cli_support` re-export module would have kept all 2000+ items
  reachable and forced the binary off struct literals.
- **Keep a small documented surface**: `YoutubeDl` with its extraction,
  selection, manifest, download (options, progress, control) and local
  processing calls; the info structs a result carries; the option,
  progress and control types; `CookieJar`, `HttpOptions`,
  `YoutubeOptions`; `DownloadArchive`/`archive_id`; `PlaylistSelection`;
  subtitle selection and paths; `output::{render_template,
  render_template_with_options, TemplateOptions, render_formats}`,
  `validate_format_selector`; `Error`/`Result`.
- **Hide what tests need but should not be promised**, in a
  `#[doc(hidden)] pub mod __private` (excluded by `cargo-semver-checks`):
  the `Extractor` trait, `register_extractor`, `DirectHttp`/`GenericHttp`
  (the trait exposes the reqwest client and upstream's extractor hooks),
  `host_path`, `shell_quote_command` and the container sniffers.
- **`#[non_exhaustive]`** on option, progress, control, result and info
  structs that gain fields, and on `Error` and the enums that grow
  (`DownloadPhase`, `StreamKind`, `PartFiles`, `MergeFiles`,
  `ManifestKind`, `VideoMode`). External code builds them from `Default`
  (or their constructors) and assigns fields; CLI-only fields of a kept
  struct (`PostprocessOptions`' fixup, chapter, info-JSON and metadata
  parser plans) become `pub(crate)`.
- **One error type**: with their modules private, `netrc::ParseError`,
  `SelectorError` and `JsError` are internal; public calls return
  `yt_dlp_rs::Error`. `reqwest::Error` stays inside `Error::Http` and
  `Error::HttpRetryAfter` (tests read its status): reqwest 0.12 is a
  public dependency, documented as such, so a reqwest major update is a
  breaking release.
- **`cargo-semver-checks`** in CI compares the PR with the latest
  release tag's library.
