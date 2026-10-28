# Library API

The `yt_dlp_rs` crate exposes the extraction and download pipeline the CLI uses.
The API is unstable while the port is in beta. See the
[compatibility inventory](COMPATIBILITY.md) for the behavior behind each call.

The library exposes `YoutubeDl`, `ExtractResult`, `PlaylistInfo`, `VideoInfo`,
`VideoFormat`, `StreamKind`, `Fragment`, `ByteRange`, `HlsAes128`, `DownloadOptions`, `DownloadProgress`, `DownloadPhase`,
`ExtractionOptions`, `YoutubeOptions`, and an `Extractor` interface.
`YoutubeDl::with_youtube_options(YoutubeOptions { js_runtime: Some(path), ..Default::default() })`
configures optional YouTube signature/n solving; `YoutubeDl::new()` keeps it disabled.
`include_translated_subtitles: true` additionally exposes translated manual captions
in `automatic_captions`; the CLI enables this with `--write-auto-subs` or `--list-subs`.
ASR translations are discovered independently, without fetching their payloads.
`YoutubeDl::with_options(YoutubeOptions::default(), HttpOptions {
user_agent: Some("My media client".into()), ..Default::default()
})` configures HTTP defaults; `HttpOptions.headers` holds name/value pairs.
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
`download_with_options(info, path, DownloadOptions { retries: 0 })` disables
download retries for a library call; `download` uses the default policy.
`download_with_progress(info, path, options, callback)` reports one-based attempts,
stored bytes, optional total bytes, and the current phase. Fragment downloads also report one-based fragment indexes
and counts, with cumulative assembled bytes and a per-fragment attempt number. Return
`std::ops::ControlFlow::Continue(())` to continue or `Break(())` to cancel.
Cancellation is cooperative: it cannot interrupt an in-flight blocking request,
read, or filesystem operation. See the compatibility inventory for checkpoints.
`extract_result_with_options(url, None, ExtractionOptions { retries: 2 })` enables
whole-extraction retries; existing extraction methods keep retries disabled.
`YoutubeDl::with_growing_file(true)` makes `download_with_progress` record live
results as one growing QuickTime file (see `--growing-file` in the
[CLI table](COMPATIBILITY.md#cli)); `YoutubeDl::grows(&info)` tells whether a
result will be recorded that way.
