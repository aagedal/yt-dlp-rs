# Supported compatibility subset

This inventory describes tested behavior, not general yt-dlp compatibility.
All extraction and downloads run in Rust. Python is development tooling only.

## CLI

| Option | Current behavior |
| --- | --- |
| Positional URLs | One or more HTTP(S) URLs (stripped), processed sequentially; continue after errors, return failure if any URL fails; optional with a batch file |
| `-a`, `--batch-file FILE`, `--no-batch-file` | URLs from FILE or `-` (stdin) precede positional URLs; upstream comment/BOM/newline/whitespace rules and `~`/`$VAR` path expansion; unreadable file fails before requests; last flag wins; see [batch files](../upstream/notes/batch-file.md) |
| `-j`, `--dump-json` | One selected `VideoInfo` JSON object per video, printed like upstream's `json.dumps` (`", "`/`": "` separators, `ensure_ascii`), including each HTML5 playlist entry, with upstream's `_filename`/`filename`, `__last_playlist_index`, `epoch` and `_type` (`-J` adds only `epoch` and `_type`, as upstream); no downloads; with `-F`, a table precedes JSON. Format dicts have upstream's shape where ported: HLS formats carry no fragments (HlsFD reads the playlist), with `has_drm` and, from a master playlist, `manifest_url`; DASH formats carry the MPD as `manifest_url`, `is_dash_periods`, `fragment_base_url` (the MPD's directory without a BaseURL) and fragments as `path` (relative locations) or `url` with upstream's `duration` (`d/timescale`, or the constant `duration` of a template or SegmentList; none for initialization) and, for this port's byte-range plans, `byte_range` as `start`/`end`; a missing stream's `vbr`/`abr` is the integer `0`; formats without a size or `manifest_url` get upstream's `filesize_approx` (`int(duration * tbr * 125)`); this port's internal stream-kind and audio-group keys are not written. DASH subtitle dicts keep this port's shape. For GenericIE's direct, untyped, HLS and DASH results and HTML5 embeds and playlist entries, keys follow upstream's insertion order (the extractor's dict, `add_extra_info`, `process_video_result`, then the selected format or `_merge` dict) with upstream's `null` keys, and an untyped direct link has no `formats`, like upstream; saved pinned upstream lines are compared exactly (`tests/fixtures/info_order/`, `tools/capture_info_order.py`; `epoch` and the origin normalized). `--load-info-json` results keep the loaded file's key order (and its values for keys this port does not model), with the keys processing adds following in upstream's order, as upstream processes the loaded dict in place (`tests/fixtures/info_order/loaded/`). YouTube watch results follow upstream `_real_extract`'s key order with its `null` keys (compared exactly in the `youtube*` site fixtures). `_version` (upstream's own release) is not written |
| `-J`, `--dump-single-json` | One JSON object per URL after processing: the `-j` object for videos (`null` when selection fails, or when extraction fails without `--abort-on-error`, after the error, as upstream dumps `extract_info`'s `None`); playlists add `_type`, `playlist_count`, `requested_entries` (omitted for the full playlist, like upstream) and the processed `entries` (`null` for failed entries); implies quiet simulation; like upstream's processed result, each video carries `requested_downloads` (each download's `-j` dict, with `__write_download_archive`, minus the keys equal to the video's, after `epoch`) before the selected format's keys, and an HTML5 playlist dict carries the page's `timestamp`/`upload_date` and upstream's empty `__files_to_move`; key order and `null` keys follow `-j` (saved pinned upstream `-J` lines are compared exactly). With `--no-simulate`, each download gets `process_info`'s `__postprocessors`, `__real_download`, `__files_to_merge` (merges), `__finaldir` and the absolute `filepath`; `__postprocessors` lists the merger as `<yt_dlp.postprocessor.ffmpeg.FFmpegMergerPP object>` (upstream's repr includes a memory address) and omits fixup processors |
| `-F`, `--list-formats` | List eligible alternatives without selection or downloads; combined `-j`/`-s` retain normal selection; upstream `render_formats_table` layout without colors (the `[info] Available formats for ID:` caption is a screen message, so quiet modes print only the table) |
| `--list-formats-old`, `--no-list-formats-as-table`, `--list-formats-as-table` | Upstream `listformats_table`: the old `format code  extension  resolution  note` layout (`_format_note`) |
| `-q`, `--quiet`, `--no-quiet` | Suppress informational `[...]` stdout lines (upstream `to_screen`); implied by printing/JSON unless `--no-quiet`; errors and warnings remain; last flag wins |
| `--no-warnings` | Suppress `WARNING:` lines |
| `--print-traffic` | Upstream `http.client` debug output on stdout (also when quiet): `send: b'…'` with the request bytes actually written (reqwest's connection tracing, so header names are lowercase and in this client's order, and no `Connection: close` is added), then `reply: '…'` and `header: NAME: VALUE` for each response's status line and headers. Lines are printed after each request, in order with the screen output (upstream's buffered `print()` often lands later in pipes); upstream's `director:` request-handler lines describe its Python handlers and are not printed |
| `--dump-pages`, `--write-pages` | Upstream `_webpage_read_content` for the bodies extraction reads: the generic HTML or DASH page (before `Extracting information`), its m3u8 manifest (after `Downloading m3u8 information`) and the `Checking m3u8 live status` playlist, and embedded HTML5 manifests. `--dump-pages` prints `[EXTRACTOR] Dumping request to URL` and the base64 body; `--write-pages` prints `[EXTRACTOR] Saving request to FILE` and writes the body to `_request_dump_filename`'s name in the working directory (`ID_URL.dump`, restricted-sanitized, cut to `--trim-filenames` or 240 characters with a `___MD5` suffix). Direct media and the downloader's own playlist refetches are not dumped, as upstream. YouTube extraction dumps the watch and client-config pages, every InnerTube POST (`player`, `next`, `browse`, ...; named `ID_MD5_URL.dump` with the MD5 of the request body, which is sent byte-identically to upstream's `json.dumps`), the player JS and the iframe API JS; generic `_download_json` bodies are dumped too (`youtube-dumps`, `bild`) |
| `-v`, `--verbose` | Upstream `write_debug` `[debug] ` lines on stderr: the configuration tree (`Command-line config: [...]`, nested `\| ` locations, `hide_login_info` `PRIVATE` values) and `Batch file urls`, then `print_debug_header`'s `Encodings` (utf-8, `No ANSI`/`dumb` per stream), `exe versions` (`ffmpeg -bsfs` version and `fdk`/`needs_adtstoasc`/`setts` features) and `Proxy map` (`--proxy` or `*_proxy` environment, Python dict) lines, then validation warnings, `Loaded 3 extractors` and `Loading archive file`; during processing `Identified a direct video link`/`M3U playlist`/`DASH manifest`, `Looking for embeds`/`Identified a html5 embed` (`N html5 embeds`), `Sort order given by user`/`by extractor` and `Formats sorted by` (with limits like `hdr:12(7)`), `Default format spec`, `Invoking http`/`hlsnative`/`dashsegments`/`ffmpeg downloader on "URL"` (also for subtitles), `ffmpeg`/`ffprobe command line` for every FFmpeg run (FFmpegFD also switches to `-loglevel verbose` without `-hide_banner`), MetadataParser `Searching for`/`Replacing all` and `Adding to archive`. `Extracting URL` lines are not truncated, quiet-mode screen lines go to stderr and `--no-warnings` warnings print as `[debug] WARNING:`. Verified against pinned upstream except environment lines: the version and system lines describe this port (`yt-dlp-rs version`, `Rust native build`), Python-only lines (lazy extractors, Git HEAD, optional libraries, JS runtimes, request handlers, plugins) are omitted, FFmpeg command lines show the resolved executable and this port's arguments, error tracebacks and failed-FFmpeg stderr dumps are not printed, and `Formats sorted by` is reported when a result is extracted (before, not between, a playlist's entry downloads) |
| `-s`, `--simulate`, `--no-simulate` | Extract metadata, print screen lines to stdout and diagnostics to stderr, create no media files; `--no-simulate` downloads even with printing/JSON/listing; last flag wins; resolution matches upstream (`tools/compare_simulate.py`) |
| `-O`, `--print [WHEN:]TEMPLATE` | Print a field list or unsanitized output template per selected video at the `video`, `before_dl`, `post_process`, `after_move` or `after_video` stage, or once per playlist at `playlist` (after its entries, over the playlist dict: `id`, `title`, `playlist_count`, `requested_entries` and the extracted entries); implies quiet, and simulation unless a later stage prints; repeatable; see [print templates](#print-templates) |
| `-e`/`--get-title`, `--get-id`, `-g`/`--get-url`, `--get-thumbnail`, `--get-description`, `--get-filename`, `--get-duration`, `--get-format` | Hidden upstream aliases printing, after `--print` lines and in upstream `__forced_printings` order, title, ID, selected URLs, thumbnail, description, prepared filename, `formatSeconds` duration and `format`; thumbnail, description and duration print only when present; imply quiet simulation |
| `-f`, `--format` | `-f -` is upstream's interactive selection: the formats table, then `Enter format selector (Press ENTER for default, or Ctrl+C to quit): ` read from stdin until a selector selects something (`ERROR: Requested format is not available` or the syntax error is reported without failing; an empty line is the default; end of input is `ERROR: EOF when reading a line`). A port of upstream `build_format_selector`: atoms, ranks, extensions, IDs, `all`/`mergeall`, filters, groups, `/`, `+` and `,`; see [Format selection](#format-selection); format IDs follow upstream for direct, HLS and DASH results (see below); exactly `-f best` or `-f mp4` prints upstream's common-mistake warning, ordered with the other `validate_options` conflict warnings |
| `-o`, `--output [TYPES:]TEMPLATE` | Upstream's template keys (traversal, maths, `>` dates, alternates, `&` replacements, defaults) and conversions (`diuoxXeEfFgGcrsa` plus `ljhqBUDS`), flags/width/precision and literal `%%`; upstream metadata sanitization; typed `subtitle`/`infojson`/`pl_infojson` templates (empty skips that file), `~`/`$VAR` template expansion; no overwrite; see [output paths](../upstream/notes/output-paths.md) |
| `-P`, `--paths [TYPES:]PATH` | `home` (default) and output-type directories joined like upstream `get_output_path`; absolute templates ignore them with a warning. `temp:` names the download beside `HOME/TEMP/NAME` (also checked for an existing download). Upstream's `MoveFilesAfterDownloadPP` runs after the `post_process` stage (and with `--skip-download`): subtitles and thumbnails are written beside the (temporary) media name and moved to their typed names (also without `temp:`, e.g. `-P subtitle:DIR`), converted subtitles/thumbnails follow their originals, `-k` sources go to the final directory, and the media moves to the absolute final directory with `[MoveFiles] Moving file "OLD" to "NEW"`. Existing targets print `Replacing existing file "NEW"` (removed first), or with `--no-overwrites` `Cannot move file "OLD" out of temporary directory since "NEW" already exists. ` and stay. Info JSON, descriptions, links and split chapters are written to their final names. Files deleted by earlier processors are skipped silently, so upstream's `File "OLD" cannot be found` warning is not printed |
| `--list-subs` | Upstream `render_subtitles_table` (`Language Name Formats`, formats in reverse order, `ID has no subtitles`); list-only unless combined with `-j`/`-s`, which retain normal format selection; no sidecar downloads |
| `--write-subs`, `--write-srt` | Select normal subtitles and write sidecars before actual media downloads; JSON/simulation/listing only expose selection; each video with a selection first prints upstream's `[info] ID: Downloading subtitles: LANGS` |
| `--write-auto-subs`, `--write-automatic-subs` | Select automatic captions independently; manual tracks win shared languages when both writing flags are enabled |
| `--no-write-auto-subs`, `--no-write-automatic-subs` | Disable automatic writing; last automatic write/no-write flag wins |
| `--no-write-subs`, `--no-write-srt` | Disable subtitle writing; last write/no-write flag wins |
| `--sub-langs`, `--srt-langs` | Repeated comma-separated literal tags, `all` and `-` exclusions; trimmed tokens, case-insensitive matching; no regex |
| `--embed-subs`, `--no-embed-subs` | Embed selected sidecars after media processing; implied acquisition and source retention rules below; last flag wins |
| `--convert-subs`, `--convert-sub`, `--convert-subtitles` | Convert written sidecars to one of srt/vtt/ass/lrc; `none` disables; no implied subtitle writing |
| `--sub-format` | Ordered `/` preferences, default `best`; last matching format or last available with a warning |
| `--skip-download`, `--no-download` | Skip media while allowing selected subtitle and info JSON sidecars; never record a completed media archive entry |
| `--write-info-json`, `--no-write-info-json` | Write cleaned video/playlist `.info.json` files before media/entries, in upstream's key order (see `-j`); a playlist file written before its entries is rewritten after them (`[info] Writing updated playlist metadata as JSON to: FILE`, before `playlist`-stage processing; with `--no-clean-info-json` it still lacks upstream's processed `entries`); existing files are replaced unless `-w`; never with JSON/simulation/listing; last flag wins; see [info JSON](../upstream/notes/info-json.md) |
| `--restrict-filenames`, `--no-restrict-filenames` | ASCII-only upstream sanitization and space-free default template; last flag wins |
| `--windows-filenames`, `--no-windows-filenames` | Force upstream path sanitization, or minimal `/`-only sanitization outside Windows; last flag wins |
| `--trim-filenames`, `--trim-file-names LENGTH` | Trim rendered names excluding the last two dot-separated parts; 0 disables |
| `-w`, `--no-overwrites`, `--force-overwrites`, `--yes-overwrites`, `--no-force-overwrites` | Upstream overwrite modes for media, info JSON and subtitles; last flag wins; see [overwrite options](#overwrite-options) |
| `--match-filters FILTER`, `--no-match-filters`, `--break-match-filters FILTER`, `--no-break-match-filters` | Upstream `match_str` over the JSON form of represented metadata: `&` conditions, repeated filters as OR, comparisons with `?`/`!`, string operators, regexes (Rust `regex`; no lookaround/backreferences), `parse_filesize`/`parse_duration` numbers; checked before selection (format fields incomplete) and again before processing; rejected videos are skipped with upstream's message, breaking rejections cancel with exit 101; filters are validated at startup; `-` (upstream's interactive filter) asks `Download "FILE"? (Y/n): ` on stdin before each download that passes the other filters (`y`/empty downloads, `n` prints `[download] Skipping TITLE`, other answers ask again, end of input fails the video with `ERROR: EOF when reading a line`); it is unsupported among `--break-match-filters`. A breaking rejection prints `[download] REASON`, then upstream's fixed `RejectedVideoReached` message |
| `--date`, `--datebefore`, `--dateafter DATE` | Upstream `DateRange` from strict `date_from_str` (`YYYYMMDD` or `now`/`today`/`yesterday` with `-N(day\|week\|month\|year)[s]`, UTC), `--date` overriding the others with upstream warnings, and upstream validation messages (out-of-range dates use the Python 3.13 wording); videos with an `upload_date` outside the range are skipped with `[download] YYYY-MM-DD upload date is not in range START to END` before the match filters; ASCII digits only |
| `-i`, `--ignore-errors`, `--no-abort-on-error`, `--abort-on-error`, `--no-ignore-errors` | Upstream `ignoreerrors` (last flag wins): by default (`--no-abort-on-error`) a failed input or video is reported, the queue continues and the exit status is 1; `--abort-on-error` stops the queue at the first reported error; `-i` additionally reports a failed postprocessor (`ERROR: …`) and continues the remaining processors with the latest completed file, so the video is published and archived (exit status 1). Merging is not skipped under `-i` |
| `--mtime`, `--no-mtime` | Upstream `try_utime`: with `--mtime` (off by default, last flag wins) whole-file media and subtitle downloads take their modification time from the final response's `Last-Modified` via ported `timeconvert` (`parsedate_tz` with zone offsets; the epoch is ignored), and fragmented downloads from the latest fragment carrying one; thumbnails are unaffected. Independently of `--mtime`, every FFmpeg output (merge, audio/video conversion, subtitle/metadata/thumbnail embedding) takes the oldest input modification time; timestamp failures are ignored |
| `--exec [WHEN:]CMD`, `--no-exec` | Upstream `ExecPP` on POSIX (`/bin/sh -c`; rejected on Windows): case-insensitive `WHEN:` prefixes `video`, `before_dl`, `post_process`, `after_move` (default) and `after_video` run at upstream's points (`video` and `after_video` also when simulating; `before_dl` also with `--skip-download`; `post_process`/`after_move` also for an already-downloaded file); `playlist` commands run once after a playlist's entries (also when simulating) over the playlist dict, unchanged when they have no fields; `pre_process` and `after_filter` are rejected. Templates use output-template fields with only `d`/`i`/`f`/`q` (shell-quoted) conversions plus `filepath` and `_filename`, validated at startup with upstream's unsafe-conversion/default/placeholder errors (exit 1); without fields, `{}` (appended when absent) becomes the quoted path, absolute after the move. `[Exec] Executing command: CMD` precedes each run; a failing `post_process`/`after_move` command fails the video before archiving (`ERROR: Postprocessing: …`, or reported and skipped under `-i`), and a failing `video`/`before_dl` command is reported as `ERROR: Preprocessing: …` and again after the video. `--no-exec` clears earlier commands; hidden `--exec-before-download` maps to `before_dl` with upstream's conflict warning |
| `--use-postprocessor NAME[:ARGS]` | Upstream `get_postprocessor` definitions for the built-in `Exec`/`ExecPP` only: `ARGS` is `;`-separated `NAME=VALUE` with `exec_cmd` and `when` (default `post_process`); the commands run like `--exec` ones and before the `--exec` commands of their stage. Other postprocessor names (plugins and built-ins) and other arguments are native errors |
| `--concat-playlist POLICY` | Upstream `FFmpegConcatPP` at the `playlist` stage: `always` joins the entries' downloaded files with the FFmpeg concat demuxer (stream copy) into the `pl_video` output template (`ext` from the entries, `mkv` when they differ; upstream's playlist fields), after an ffprobe check that every file has the same stream codecs (`The files have different streams/codecs and cannot be concatenated…`); `[Concat] Concatenating N files; Destination: FILE`, then the parts are deleted unless `-k` (`Deleting original file ABSPATH (pass -k to keep)`); a single entry is moved (`[Concat] Moving "A" to "B"`); a failed entry aborts it (`Aborting concatenation because some downloads failed`); nothing happens when simulating. The default `multi_video` and `never` do nothing, since this port's extractors produce no multi-video results. Runs before `--exec playlist:` commands |
| `--load-info-json FILE`, `--load-info` | Upstream `download_with_info_file`: FILE (with `~`/`$VAR` expansion) holds one dictionary or a list, each sanitized like the default `clean_infojson` (`None` values, `__` keys, `requested_*`, `entries`, `filepath`/`filename` and similar keys dropped) and processed without extraction: filters, archive checks and format selection run again over the saved `formats`. URLs are ignored with upstream's warning. Because sanitizing removes `entries`, a playlist prints its `Downloading playlist` line and falls back to re-extracting its `webpage_url` with upstream's `There are no entries` warning. Top-level keys this port does not model are kept (for templates, `--print`, JSON output and `stretched_ratio`). Upstream-written dictionaries load when they fit the represented subset (`direct` stays absent unless saved, stream kinds come from `vcodec`/`acodec` `none`, a missing top-level `url` comes from the last format); `m3u8_native` formats need no fragments (HlsFD reads the playlist when downloading: `[hlsnative] Downloading m3u8 manifest`), DASH fragment `path`s resolve against `fragment_base_url` (`byte_range` `start`/`end` too), and formats with other protocols and no saved fragments are rejected. A raised `DownloadError` (a failed subtitle download, or under `--abort-on-error` the first reported error) prints an empty `\r` line and `WARNING: The info failed to download: MESSAGE; trying with URL WEBPAGE_URL` (MESSAGE keeps its `ERROR: ` prefix when it was reported) and processes the webpage URL instead; the failure does not count toward the exit status |
| `--write-link`, `--write-url-link`, `--write-webloc-link`, `--write-desktop-link` | Upstream `_write_link_file` after info JSON (also with `--skip-download`, never when simulating): `--write-link` picks `.webloc` on macOS, `.desktop` on Linux and `.url` elsewhere; the `link` output template with the shortcut extension; `webpage_url` through ported `iri_to_uri` (http/https only, no IPv6; otherwise upstream's warning); `.url` files with CRLF and `.desktop` names escaped; an existing shortcut is kept (`Internet shortcut (.EXT) is already present`) unless `-w` is given, as upstream's overwrite check does |
| `--buffer-size SIZE`, `--resize-buffer`, `--no-resize-buffer` | HttpFD's reads: full blocks of `--buffer-size` (default 1024; positive, upstream's size syntax) resized after each block by `best_block_size` unless `--no-resize-buffer`, which sets when progress lines appear, as upstream (also each fragment's) |
| `--newline`, `--no-progress`, `--progress`, `--progress-delta SECONDS` | Upstream progress lines and printers (fragmented downloads like FragmentFD's hook: its size estimate over the running total, `ProgressCalculator`'s smoothed speed and ETA, and a line as each fragment finishes); see [download progress](#download-progress-and-cooperative-cancellation) |
| `--sleep-requests SECONDS` | Upstream `_request_webpage` delay: every extraction request after the run's first one waits, announced as `[EXTRACTOR] Sleeping SECONDS seconds ...` before its `Downloading …` line; this port's extra media/child-playlist probes (plain HTML5 sources, HLS variants, YouTube media) are not delayed, as upstream does not make them; validated with upstream's message |
| `-c`, `--continue`, `--no-continue`, `--part`, `--no-part` | Upstream `.part` files for whole-file media downloads: the transfer is written to `FILE.part` (kept after a failure) and renamed on success; an existing part file is continued with a plain `Range` request (`[download] Resuming download at byte N`; a full response restarts with `[download] Unable to resume`, and a complete part file fails with the server's 416 like upstream); `--no-continue` restarts it and `--no-part` writes the output directly. Merged, fragmented and postprocessed downloads keep private temporary files, and the library default remains a private temporary file removed on failure |
| `--age-limit YEARS` | Upstream `age_restricted`: skip videos whose `age_limit` exceeds YEARS (`Skipping "TITLE" because it is age restricted`); unknown ratings pass |
| `--min-views`, `--max-views`, `--match-title`, `--reject-title`, `--break-on-reject` (hidden upstream) | Upstream `_match_entry` order and messages: case-insensitive title regexes (Rust `regex` syntax, validated at startup), view counts when known, and `--break-on-reject` turning any rejection into the `RejectedVideoReached` cancellation (exit 101) |
| `--max-downloads NUMBER` | Stop after NUMBER processed videos (simulated, already-downloaded and size-skipped ones count), print upstream's message and exit 101; 0 means unlimited |
| `-r`, `--limit-rate`, `--rate-limit RATE` | Upstream `slow_down` per whole-file media transfer or fragment request: sleep after each read until the average rate is at most RATE (`parse_bytes`, strictly positive); subtitles are not limited |
| `--sleep-interval`/`--min-sleep-interval`, `--max-sleep-interval`, `--sleep-subtitles SECONDS` | Upstream validation messages; sleep a uniform random interval (time-seeded, not Python's RNG) before each media stream download after existing-file checks, and before each subtitle download; `--sleep-requests` is not implemented |
| `--min-filesize SIZE`, `--max-filesize SIZE` | Upstream `parse_bytes` sizes (`50k`, `44.6M`); whole-file HTTP media transfers with a declared length outside the limits are skipped with upstream's `[download]` message, no error and no archive record; fragment/HLS/DASH transfers are not checked |
| `--write-description`, `--no-write-description` | Write the represented description to the `description` output type (`.description`) before subtitles; upstream messages for missing/present descriptions; follows overwrite options; playlists report no description |
| `--list-thumbnails` | Upstream `[info] Available thumbnails for ID:` table (ID/Width/Height/URL) or `ID has no thumbnails`; implies list-only simulation like other listings |
| `--write-thumbnail`, `--no-write-thumbnail`, `--write-all-thumbnails` | Upstream flag folding; try thumbnails best-first until one is written (all with `--write-all-thumbnails`, named `ID.EXT` when several), before info JSON; upstream messages, 404 skip, overwrite options; raw bytes unless converted |
| `--convert-thumbnails FORMAT` | Upstream `FFmpegThumbnailsConvertorPP` as a `before_dl` processor (after SubtitlesConvertor, before `--exec before_dl:`): `jpg`/`png`/`webp` targets or `SRC>TGT/...` mappings (`none` disables) with upstream validation and "Not converting"/"could not find a mapping"/"already is in target format" messages, WebP extension fixup, `-f image2 -pattern_type none`, `-update 1`, `mjpeg2jpeg` for JPEG; originals deleted unless `-k`; requires `--ffmpeg-location` |
| `--embed-thumbnail`, `--no-embed-thumbnail` | Upstream `EmbedThumbnailPP` as the final stage after metadata: implies `--write-thumbnail` and deletes the written file after embedding unless it was requested explicitly; best on-disk thumbnail, WebP fixup, PNG conversion outside mkv/mka; mp3 ID3v2.3 cover, mkv/mka `-attach` with mimetype/filename (replacing an existing same-mimetype attachment), mp4/m4a/m4v/mov and ogg/opus/flac as upstream does with mutagen installed (its default dependency), natively: `[EmbedThumbnail] mutagen: Adding thumbnail to "FILE"`, an iTunes `covr` atom (`moov.udta.meta.ilst`, creating mutagen's `mdirappl` handler; chunk offsets follow a grown `moov`), a FLAC `PICTURE` block before any padding, or an Ogg Opus/Vorbis `METADATA_BLOCK_PICTURE` comment (header pages laced again, later pages renumbered), each picture a front cover with the image's MIME type and size; files are not byte-identical to mutagen's (no added padding). An MP4 that fails warns `unable to embed using mutagen; REASON` (`could not determine image type`, `not a MP4 file`) and falls back to the FFmpeg `attached_pic` method after `Neither mutagen nor AtomicParsley was found. Falling back to ffmpeg` (AtomicParsley is never used); upstream's `[ThumbnailsConvertor] Correcting thumbnail "FILE" extension to webp`/`Converting thumbnail "FILE" to png`, `[EmbedThumbnail] ffmpeg: Adding thumbnail to "FILE"`, and `There aren't any thumbnails to embed`/`There are no thumbnails on disk` when the processor runs; other containers are explicit errors; requires `--ffmpeg-location` (ffprobe for the FFmpeg methods) |
| `--write-playlist-metafiles`, `--no-write-playlist-metafiles` | Enable (default) or disable playlist info JSON/description files |
| `--output-na-placeholder TEXT` | Missing/failed numeric conversion placeholder, default `NA`; per-field defaults override it |
| `-R`, `--retries RETRIES` | Additional whole-file download attempts, default 10 like upstream; a non-negative integer or `inf`/`infinite`, with upstream's `parse_retries` usage errors; independent of extraction retries. Each retry prints upstream's `[download] Got error: TEXT. Retrying (N/M)...` line (HTTP statuses as `HTTP Error CODE: Reason` with the server's reason phrase, OS-level transport errors as urllib's `[Errno N] TEXT`), and the final error reads `ERROR: \r[download] Got error: TEXT. Giving up after M retries` (without the suffix under `-R 0`). Other media download HTTP errors print upstream's `ERROR: unable to download video data: HTTP Error CODE: Reason`; HttpFD names `[download] Destination: FILE` only once the server answers, so failed requests print none. A failed subtitle download is `Unable to download video subtitles for 'LANG': ERR` (a warning with `-i`, otherwise the video's error). Only the native transient failures listed under [Download retries](#download-retries) are retried |
| `--fragment-retries RETRIES` | Additional attempts per HLS/DASH fragment, default 10, same values and messages with `Retrying fragment N` |
| `--retry-sleep [TYPE:]EXPR` | Upstream `parse_sleep_func` delays for `http` (default type) and `fragment` retries: a number, `linear=START[:END[:STEP]]` or `exp=START[:END[:BASE]]`, announced with `Sleeping S.SS seconds ...`; invalid expressions fail with upstream's usage error. A `Retry-After` hint is still honored as a minimum. Without an expression the short native backoff is used instead of upstream's immediate retry. `extractor` and `file_access` types are accepted but unused |
| Fragment download resume | Upstream `FragmentFD` state from the CLI (HLS and DASH): fragments are appended to `FILE.part` (or `FILE` with `--no-part`) and `FILE.ytdl` records `{"downloader": {"current_fragment": {"index": N}}}` (with HLS's empty `extra_state`) after each downloaded fragment; a failed download keeps both, and with `--continue` (the default) the next run skips the recorded fragments, appends the rest, then removes the state and renames the part file. A corrupt state file or one that records fragments for an empty part prints upstream's `.ytdl file is corrupt`/`Inconsistent state of incomplete fragment download` `. Restarting from the beginning ...` warning. Unlike upstream, a restart (also with `--no-continue`) truncates the part file instead of appending to the old bytes, and concurrent fragments (`-N`) are assembled into the part file in order rather than left as `-FragN` files; the library default still assembles a private temporary file |
| `--skip-unavailable-fragments`, `--no-abort-on-unavailable-fragments`, `--abort-on-unavailable-fragments`, `--no-skip-unavailable-fragments` | Upstream fragment semantics from the CLI: every HTTP status error is retried for fragments, and with skipping (the default) a media fragment whose retries are exhausted is left out with `[download] fragment not found; Skipping fragment N ...` (never the first fragment). Aborting fails the download with upstream's `ERROR: \r[download] Got error: TEXT. Giving up after N retries` and `ERROR: fragment N not found, unable to continue`. The library default (`skip_unavailable_fragments: None`) keeps all-or-error assembly with transient-only retries |
| `--socket-timeout SECONDS` | Connect and per-operation timeout for every native client; 0 keeps native defaults (15 s connect, 300 s operations; upstream defaults to 20 s); negative/non-finite values fail before requests |
| `--source-address IP`, `-4`/`--force-ipv4`, `-6`/`--force-ipv6` | Bind every native client to an IP address; `-4`/`-6` store `0.0.0.0`/`::` like upstream; last of the three wins |
| `--no-check-certificates` | Accept invalid HTTPS certificates on every native client |
| `--proxy URL` | Explicit HTTP/HTTPS/SOCKS route; empty string forces direct connections, last value wins |
| `--cookies FILE`, `--no-cookies` | Load/save a scoped Netscape jar, including metadata modes; last flag wins, response cookies remain in memory |
| `--cookies-from-browser BROWSER[+KEYRING][:PROFILE][::CONTAINER]`, `--no-cookies-from-browser` | Upstream's specification parsing and usage errors (`unsupported browser specified for cookies: …`, `unsupported keyring …`, `invalid cookies from browser arguments: …`). `firefox` is read natively: profile directories per platform (or a profile name/path), the most recently modified `cookies.sqlite` of the root, `*/` and `Profiles/*/`, `containers.json` containers (by name or the whole `userContext….label` ID, as upstream matches) or `none`, schema-16 millisecond expiries, `Extracting cookies from firefox`/`Extracted N cookies from firefox` and the debug lines, through a native read-only SQLite reader (the write-ahead log is not read, as upstream's copy is opened without it). Like upstream the jar loads lazily: the lines (and a load error, which ends the run) appear at the first request, at the debug header with `-v`, or when `--cookies` is saved, where a failed load is retried and reported again. Browser cookies come first and `--cookies` file records replace them; the saved file then holds both. Chromium-based browsers (`brave`, `chrome`, `chromium`, `edge`, `opera`, `vivaldi`, `whale`) follow `_extract_chrome_cookies`: per-platform user data directories (or a profile name/path; Opera has no profiles), the newest `Cookies` below it, the `meta` version (24+ strips the SHA-256 prefix), `is_secure`/`secure`, upstream's `Extracted N cookies from BROWSER ( M could not be decrypted)` and `cookie version breakdown` lines, and decryption with PBKDF2-SHA1 and AES-128-CBC (upstream's lenient PKCS#7 removal): on macOS `v10` values with the `security find-generic-password` keychain password (other prefixes are upstream's plaintext "old data", whose `bytes` value becomes its `b'…'` text), on Linux `v10` (`peanuts`/empty key) and `v11` with the keyring chosen from `+KEYRING` or the desktop environment (`BASICTEXT`; KWallet through `dbus-send` and `kwallet-query`; GNOME keyring reports `secretstorage not available` like an upstream without that module and uses the empty password). Windows DPAPI decryption is an explicit error. Chromium's raw expiry timestamps (which upstream saves unchanged) are capped at 9999-12-31 in the jar. `safari` parses `Cookies.binarycookies` on macOS (the default or secondary location, or a file path) with upstream's `DataParser` errors and `skipping N bytes (…): b'…'` debug lines; other platforms report `unsupported platform: …`. Upstream's `logger.error` lines (such as `opera does not support profiles`) print `ERROR:` and fail the run's exit status without ending it. Python tracebacks are not printed, and upstream's crash when saving a bytes-valued cookie is not reproduced |
| `--add-headers FIELD:VALUE`, `--add-header` | Repeated custom HTTP defaults; case-insensitive last custom value wins |
| `--user-agent UA`, `--referer URL` | Convenience HTTP defaults override matching custom options; extractor/transport headers can override them |
| `--youtube-js-runtime PATH` | Opt into YouTube signature/query-n solving with an explicit Deno 2.3+ or Node.js 22+ executable; lazy validation, no discovery/installation; not general upstream runtime option parity |
| `--ffmpeg-location PATH` | Upstream `_determine_executables` for the CLI: without it `ffmpeg`/`ffprobe` are the first executable `PATH` entries; a directory holds both programs; a file selects the program its name contains (default `ffmpeg`) and finds the other by substituting the name, else beside it; a missing location warns `ffmpeg-location PATH does not exist! Continuing without ffmpeg` (once at startup) and disables ffmpeg. Availability (merge preference) does not run the executable for a version check like upstream; it is validated when used. The library keeps explicit `with_ffmpeg`/`with_ffprobe` configuration |
| `-x`, `--extract-audio`, `--audio-format`, `--audio-quality` | Ordered audio extraction mappings and bounded quality settings; see audio extraction below |
| `--remux-video`, `--recode-video` | Ordered video target mappings; stream copying or container-default re-encoding; recode wins |
| `--embed-metadata`, `--add-metadata`, `--no-embed-metadata`, `--no-add-metadata` | Final FFmpeg stage after subtitle embedding: upstream `FFmpegMetadataPP` stream-copy options, `-write_id3v1 1`, common tags from represented fields with upstream key precedence (title, date, description/synopsis, purl/comment, artist, genre, …) and per-stream `language`; requires `--ffmpeg-location` |
| `--embed-info-json`, `--no-embed-info-json` | Upstream `add_infojson` (`if_exists` with `--embed-metadata`): mkv/mka files get the written info JSON (or, with `--embed-info-json`, one generated for the purpose) as an `info.json` attachment with `mimetype=application/json`; other containers report `[Metadata] The info-json can only be attached to mkv/mka files`. Attachments are added to freshly downloaded files only (upstream also replaces an existing JSON attachment). Checked with ffprobe against pinned upstream |
| `--embed-chapters`, `--add-chapters`, `--no-embed-chapters`, `--no-add-chapters` | Upstream `addchapters` (implied by `--embed-metadata` unless disabled): `FFmpegMetadataPP` writes the chapters as an FFMETADATA second input (`_get_chapter_opts`: millisecond `START`/`END`, escaped titles) with `-map_metadata 1`; `[Metadata] There isn't any metadata to add` without chapters or metadata. Embedded chapters checked with ffprobe against pinned upstream |
| `chapters` (YouTube) | Upstream `_extract_chapters_from_json` (chaptered player bar), `_extract_chapters_from_engagement_panel` and `_extract_chapters_from_description`, via `_extract_chapters_helper` with its invalid/incomplete-chapter warnings, from the watch page's embedded `ytInitialData`, else (`Incomplete data received in embedded initial data; re-fetching using API.`) the `next` API's (`Downloading initial data API JSON`), like `_download_initial_data`. `process_video_result` fills `end_time` from the next start or the duration, inserts a chapter from 0 and names untitled ones `<Untitled Chapter N>`. Checked with three saved pages against pinned upstream |
| `--parse-metadata [WHEN:]FROM:TO`, `--replace-in-metadata [WHEN:]FIELDS REGEX REPLACE`, `--metadata-from-title` | Upstream `MetadataParserPP` (both options share one ordered action list per stage): `field_to_template`/`format_to_regex` interpretation of the unsanitized rendered template, `re.subn`-style replacement (a port of Python's `parse_template`, applied even without a match: `\1`/`\g<1>`/`\g<name>` groups checked against the regex with `invalid group reference`/`unknown group name`/`bad character in group name` errors, `\0`/three-digit octal escapes, `\a\b\f\n\r\t\v\\` escapes, `bad escape` errors for other letters, with upstream positions), and upstream's `[MetadataParser]` lines and warnings. Parsed fields this port does not model are kept as extra info-dict keys, available to templates, `--print`, JSON and `--embed-metadata`. Supported `WHEN`: `pre_process` (default), `after_filter`, `video`, `before_dl` (before the subtitle/thumbnail convertors; filenames are already chosen), `post_process` (after the fixups and merge, before the other processors, so embedded metadata and later stages see it), `after_move` and `after_video` (after that stage's `--print` lines, before its `--exec`; `after_video` sees only the earlier pre-download stages' changes, like upstream's per-format copies). `playlist` actions change the playlist dict after the playlist `--print` lines, before `Concat` (its `pl_video` filename) and `--exec playlist:`. Each stage's actions run as one processor with `MetadataParser` progress hooks. Regex syntax and error text follow the Rust `regex` crate (no look-around or backreferences); the match filter does not see `pre_process` changes, because it runs earlier |
| `--xattrs`, `--xattr` | Upstream `XAttrMetadataPP` after SplitChapters: `[XAttrMetadata] Writing metadata to file's xattrs`, then `user.xdg.referrer.url`, `user.dublincore.{title,date,contributor,format,description}` and (macOS) `com.apple.metadata:kMDItemWhereFroms` from the info dict (hyphenated `upload_date`), written with `setxattr` (alternate data streams on Windows); upstream's no-space and too-long warnings and unsupported-filesystem error. Checked with `xattr -l` against pinned upstream on macOS |
| `--split-chapters`, `--split-tracks`, `--no-split-chapters`, `--no-split-tracks` | Upstream `FFmpegSplitChaptersPP` as the last `post_process` processor (after metadata/thumbnail embedding, before `--exec`): `[SplitChapters] Splitting video by chapters; N chapters found`, then per chapter `Chapter NNN; Destination: FILE` from the `chapter` output template (default `%(title)s - %(section_number)03d %(section_title)s [%(id)s].%(ext)s`, with `section_*` fields) and an input-seeking `-ss`/`-t` stream copy; the original file is kept; `Chapter information is unavailable` without chapters. Split files match pinned upstream byte for byte on saved media. With `--force-keyframes-at-cuts` and more than one chapter, `[SplitChapters] Re-encoding "FILE" with appropriate keyframes` first re-encodes a temporary copy with `-force_key_frames` at the chapter starts (deleted afterwards); an already-downloaded file is split again like upstream |
| `--sponsorblock-mark CATS`, `--sponsorblock-remove CATS`, `--sponsorblock-chapter-title TEMPLATE`, `--no-sponsorblock`, `--sponsorblock-api URL` | Upstream `SponsorBlockPP` as the last `after_filter` processor for YouTube videos (`[SponsorBlock] SponsorBlock is not supported for EXTRACTOR` otherwise): `[SponsorBlock] Fetching SponsorBlock segments`, a GET of `API/api/skipSegments/HASH4` (first four hex digits of the ID's SHA-256, `service=YouTube`, JSON category and action-type lists; categories in upstream's table order where upstream's set order varies; 404 means no segments; `--extractor-retries` retries with `Unable to communicate with SponsorBlock API: … Retrying (N/M)...` warnings and urllib-style `[Errno N]` texts, a final failure being a deferred `ERROR: Preprocessing: …` reported again after the video), upstream's duration filter and its `different duration` warning, `Found N segments`/`No matching segments were found in the SponsorBlock database`, and the `sponsorblock_chapters` field. Marked categories become chapters and removed ones are cut by `ModifyChaptersPP` (titles from `--sponsorblock-chapter-title`, default `[SponsorBlock]: %(category_names)l`, over the chapter's `start_time`/`end_time`/`category`/`categories`/`name`/`category_names`; identically titled neighbours merge; `SponsorBlock information is unavailable`/`There are no matching SponsorBlock chapters`); sponsor chapters keep their `category`, `type`, `categories`, `name` and `category_names` keys (in this port's key order). Categories follow upstream's `_set_from_options_callback` (`all`, `default` = `all` or `all,-filler` for removal, `-CATEGORY`, `wrong CATS for OPTION: VALUE`); `poi_highlight` and `chapter` cannot be removed; `--sponsorblock-mark` implies `--embed-chapters`; `--no-sponsorblock` clears both; invalid title templates are usage errors with this port's template error text |
| `--remove-chapters REGEX`, `--no-remove-chapters`, `--force-keyframes-at-cuts`, `--no-force-keyframes-at-cuts` | Upstream `ModifyChaptersPP` after subtitle embedding and before metadata: chapters whose title matches a regex are cut with the FFmpeg concat demuxer (`inpoint`/`outpoint` from `_make_concat_opts`, stream copy, the spec deleted silently), `[ModifyChapters] Removing chapters from FILE` for the media and each remaining `srt`/`vtt`/`ass`/`lrc` subtitle file (other subtitle formats warn that they are now out of sync), originals renamed `NAME.uncut.EXT` and deleted unless `-k`; `--force-keyframes-at-cuts` re-encodes first (`Re-encoding "FILE" with appropriate keyframes`). The remaining chapters are re-timed by a port of `_remove_marked_arrange_sponsors`/`_remove_tiny_rename_sponsors` (checked against upstream's own `TestModifyChaptersPP` cases, `tools/capture_modify_chapters.py`) and replace the info dict's chapters and duration for later processors, `--split-chapters`, `after_move` prints and `--exec`; `Chapter information is unavailable`, `There are no chapters matching the regex`, the `remove the entire video` warning, the already-cut skip and the duration-mismatch error follow upstream (ffprobe supplies the real duration). Arguments are parsed by upstream's `parse_chapters` (no negative times, no `*from-url`); like pinned upstream, `*START-END` ranges alone remove nothing, and ranges combined with a regex (a pinned-upstream `TypeError`) are a native error. Native boundaries: the duration stays whole seconds, chapter dicts keep this port's key order, and SponsorBlock categories are not available |
| `--use-extractors NAMES`, `--ies NAMES`, `--force-generic-extractor` | Upstream `allowed_extractors` (`orderedSet_from_options`: comma lists, case-insensitive full-match regexes, `all`, `default`, `-` exclusions) over this port's names `html5`, `youtube`, `generic`, `end`, in matching order; `ERROR: No suitable extractor found for URL URL`, or `[UnsupportedURL] Extracting URL` / `Unsupported URL: URL` with `end`. `--force-generic-extractor` tries only `generic` (`No suitable extractor (Generic) found …`). An invalid regex is a usage error (upstream raises a traceback) |
| `--default-search PREFIX`, unqualified inputs | Upstream `GenericIE` for inputs without a scheme: `[generic] Extracting URL: INPUT`, then a domain-like input is retried with `https://` (`The url doesn't specify the protocol, trying with https`), otherwise `ERROR: [generic] 'INPUT' is not a valid URL` (`fixup_error`/`error`). `auto`, `auto_warning` and search prefixes need search extractors, which are an explicit native error |
| `--check-formats`, `--check-all-formats`, `--no-check-formats` | Upstream `_check_formats`: a quiet test download (10241 HTTP bytes or the first fragment, no retries) per format, cached per ID. `--check-formats` tests each selector atom's candidates in preference order until one works (`[info] Testing format ID`, `WARNING: Unable to download format ID. Skipping...`); `--check-all-formats` tests every format best-first before listing and selection (`[info] Unable to download format ID. Skipping...`). The test downloader's error line uses this port's error text |
| Option abbreviations and errors | optparse's `_match_long_opt`: an unambiguous prefix of a long option (including deprecated options and defined `--alias` names, also inside alias expansions) stands for it (`--match-filter`, `--restrict`); an ambiguous one is `ambiguous option: --no-ch (--no-check-certificates, --no-check-formats?)`, an unknown one `no such option: X`, and a switch given `=VALUE` `--X option does not take a value` (usage errors, exit 2); a short option's attached value is kept verbatim (`-f=best` selects `=best`). This port's own `--youtube-js-runtime` also takes part in matching |
| `--alias ALIASES OPTIONS` | Upstream option aliases: each name (prefixed with `--` unless it starts with `-`) takes as many arguments as the `str.format` fields in OPTIONS, which are shell-quoted into the template, shlex-split and parsed in place (aliases may chain, at most 100 triggers each); templates without fields are used as-is. Aliases are expanded before parsing, so they are not listed in `--help` |
| `-t NAME`, `--preset-alias NAME` | Upstream `_PRESET_ALIASES` (`mp3`, `aac`, `mp4`, `mkv`, `sleep`), expanded in place before parsing; `Unknown preset alias: NAME` usage error. Like optparse, every option that takes a value now accepts one starting with `-` |
| Format filter fields | Besides the original fields, `[...]` filters accept upstream's `acodec`, `vcodec`, `container`, `protocol`, `dynamic_range`, `resolution`, `video_ext`, `audio_ext`, `format` (string operators, `!`, `?`) and numeric `fps`, `vbr`, `asr`, `audio_channels`; other keys remain an explicit unsupported-field error (upstream looks up any key) |
| `--list-extractors`, `--extractor-descriptions` | Upstream `print_extractor_information` for this port's extractors only (`html5`, `youtube`, `generic`, with upstream's descriptions); command-line URLs are listed under every suitable extractor (`generic` takes the rest). Upstream lists about 1800 extractors |
| `--color [STREAM:]POLICY`, `--no-colors`, `--no-colours` | Validated like upstream (`"X" is not a valid color policy`); output is never colored, so `always` differs from upstream on capable terminals |
| `--write-comments`, `--get-comments`, `--no-write-comments`, `--no-get-comments` | Upstream `getcomments`: without `--write-info-json`/`--no-write-info-json` or `--embed-info-json`, and unless `-j` prints the JSON, it implies `--write-info-json`. The generic extractor has no comments, so nothing else changes; YouTube watch pages extract comments as described under [YouTube comments](#youtube-comments) |
| `--clean-info-json`, `--clean-infojson`, `--no-clean-info-json`, `--no-clean-infojson` | Upstream `clean_infojson`: the default drops `None` values, `__` keys and the private bookkeeping keys; `--no-clean-info-json` writes every represented field |
| `-N N`, `--concurrent-fragments N` | Upstream concurrent fragment downloads for HLS and DASH: N workers fetch fragments (each with its own `--fragment-retries` and skip handling) into private files beside the output, appended in order; progress reports completed fragments and in-flight bytes. Upstream's `concurrent fragments "N" must be positive` validation. Screen lines match sequential downloads apart from progress timing |
| `--downloader [PROTO:]NAME`, `--external-downloader`, `--downloader-args NAME:ARGS`, `--hls-prefer-native`, `--hls-prefer-ffmpeg` | Upstream `get_suitable_downloader` for `native` and `ffmpeg`/`avconv` (names or paths) per simplified protocol (`http`, `ftp`, `m3u8`, `dash`, `rtmp`, `default`): FFmpegFD downloads whole formats (merging all-FFmpeg pairs directly, without HlsFD's manifest line or fixups) when ffmpeg is available, otherwise the native downloaders run like upstream's `can_download` fallback. FFmpeg `--downloader-args` follow `_configuration_args` (`ffmpeg_iN`/`ffmpeg_i` before each input, `ffmpeg_oN`/`ffmpeg_o`/`ffmpeg`/`default` before the output); unnamed arguments print upstream's `External Downloader arguments given without specifying name` warning and move to the default downloader's key. `curl`, `wget`, `aria2c`, `axel` and `httpie` (names or paths, matched like `get_external_downloader`) run upstream's `CurlFD`/`WgetFD`/`Aria2cFD`/`AxelFD`/`HttpieFD` command lines for whole-file `http`/`https`/`ftp`/`ftps` formats without `hls_aes` or extractor query arguments, including merged formats' separate streams. That covers the headers, cookies (curl reads them from stdin; wget and aria2c load the `--cookies` file or an announced temporary file; axel and httpie get a `Cookie` header), `continuedl`, `noprogress`, `--limit-rate`, retries, `--max-filesize`, `--source-address`, `--proxy`, `--no-check-certificates`, aria2c's `--remote-time` from `--mtime`/`--no-mtime`, and `_configuration_args` keys. The tool writes the `.part` file, which is then renamed and reported like `ExternalFD.real_download`; a non-zero exit reports the captured stderr and `ERROR: NAME exited with code N`. An executable that cannot be run, other protocols and stdout output use the native downloaders, like upstream's `can_download`. As in the pinned upstream, no external downloader supports the `m3u8_frag_urls`/`dash_frag_urls` protocols, so HLS and DASH formats keep the native fragment downloaders. Unknown names are upstream's `No such [PROTO ]external downloader "NAME"` usage error. The HLS preference flags only affect the `m3u8` protocol, which no supported extractor produces |
| `--extractor-args generic:ARGS`, `--live-from-start` | Parsed like upstream (`IE_KEY:ARGS`, `;`-separated `KEY=VAL[,VAL]` with `\,` escapes, keys lowercased with `-` as `_`; a later value for the same extractor replaces it; malformed values are upstream's usage error). GenericIE's HLS/DASH arguments follow `_extra_manifest_info`: `fragment_query`/`key_query` (a URL's query, a query, or empty for the manifest's) become `extra_param_to_segment_url`/`extra_param_to_key_url`, `variant_query` updates every format URL, `hls_key=URI_OR_HEX_KEY[,HEX_IV]` becomes `hls_aes`, and `is_live` replaces the live check. The native HLS/DASH downloaders add the queries to fragment and key URLs and use the external key/IV. A live result gets `is_live`/`was_live`, upstream's default `best/bestvideo+bestaudio`, FFmpegFD and `--hls-use-mpegts` by default, and the title suffix ` YYYY-MM-DD HH:MM` (local time) unless `--live-from-start`. With `--live-from-start`, a live result keeps only its `is_from_start` formats (else upstream's `--live-from-start is passed, but there are no formats that can be downloaded from the start` error), and without it those formats are dropped; YouTube's are described under "YouTube formats". Live manifests are otherwise downloaded as finite VOD playlists. YouTube's `player_client`, `player_skip`, `formats`, `skip`, `po_token`, `fetch_pot`, `pot_trace`, `visitor_data`, `data_sync_id`, `innertube_host`, `innertube_key`, `player_params`, `use_ad_playback_context`, `webpage_client`, `webpage_skip`, `player_js_version`, `player_js_variant`, `raise_incomplete_data`, `lang`, `max_comments` and `comment_sort` are used (see [YouTube extractor arguments](#youtube-extractor-arguments)); and so are `youtubetab:skip=webpage,authcheck` and `youtubetab:approximate_date` (see "YouTube playlists"); other YouTube and `youtubetab` arguments warn individually; arguments for other extractors warn and are ignored |
| FFmpeg postprocessor failures | Like upstream's `real_run_ffmpeg`, postprocessor FFmpeg runs use `-loglevel repeat+info`, succeed on exit status 0 whatever stderr says, and fail with `ERROR: Postprocessing: LINE` from the last line of stderr (for example `Conversion failed!`) |
| `--postprocessor-args NAME:ARGS`, `--ppa` | Upstream `_configuration_args` for this port's FFmpeg postprocessors (`Merger`, `ExtractAudio`, `VideoRemuxer`, `VideoConvertor`, `Metadata`, `EmbedSubtitle`, `EmbedThumbnail`, `SubtitlesConvertor`, `ThumbnailsConvertor`, `SplitChapters`, `FixupM3u8`, `FixupM4a`): shlex-split values by case-insensitive `PP[+EXE][_i[N]\|_o[N]]` key (a later value replaces an earlier one); `PP+ffmpeg_iN`/`PP+ffmpeg_i` go before each input's `-i`, and `PP+ffmpeg_o1`/`PP+ffmpeg_o`/`PP+ffmpeg`, then `PP` and `ffmpeg` together, then `default` before the output file (a key given without arguments still ends the search). Unnamed values print `Post-Processor arguments given without specifying name. The arguments will be given to all post-processors` and become `default`; `wrong --ppa formatting; …` usage errors. The arguments join this port's own FFmpeg command lines, which differ from upstream's; ffprobe, AtomicParsley and unimplemented postprocessors (`ModifyChapters`, `FixupStretched`, `FixupTimestamp`, `FixupDuration`) take none |
| `--keep-fragments`, `--no-keep-fragments` | Upstream fragment files `TMPFILENAME-FragN` (`NAME.part-FragN`, or `NAME-FragN` with `--no-part`) are kept beside the output, byte-identical to the assembled fragments, for unprocessed fragmented downloads; processed downloads keep them in their private workspace (removed) |
| `--progress-template [TYPES:]TEMPLATE` | Upstream `_report_progress_status`: `download:` (default) templates render over `{'info': ..., 'progress': ...}` with `report_progress`'s fields (`status`, `downloaded_bytes`, `total_bytes`, `eta`, `speed`, `elapsed`, `fragment_index`/`fragment_count`, `filename`, `_percent_str`, `_speed_str`, `_eta_str`, `_total_bytes_str`, `_default_template`, ...) for downloading and finished statuses. `postprocess:` templates render for each postprocessor run's `started` and `finished` hooks (`progress.postprocessor` is upstream's `pp_key()`, e.g. `MetadataParser`, `SponsorBlock`, `SubtitlesConvertor`, `ThumbnailsConvertor`, `Exec`, `FixupM3u8`/`FixupM4a`/`FixupDuplicateMoov`/`FixupStretched`, `Merger`, `ExtractAudio`, `VideoRemuxer`/`VideoConvertor`, `EmbedSubtitle`, `ModifyChapters`, `Metadata`, `EmbedThumbnail`, `SplitChapters`, `XAttrMetadata`, `MoveFiles` and the playlist's `Concat`), in upstream's order and even when quiet (then on stderr), with no `finished` after a failure; `info` is the video's info dict as selected (or as parsed for `MetadataParser`/`SponsorBlock`), not each processor's updated copy. `download-title:`/`postprocess-title:` set the `--console-title` titles |
| `--compat-options OPTS` | Upstream `_set_from_options_callback`/`orderedSet_from_options` (`all`, `youtube-dl`, `youtube-dlc` and year aliases, `-NAME` removal, `wrong OPTS for --compat-options: …`) and `set_compat_opts`: `abort-on-error`, `no-playlist-metafiles`, `no-clean-infojson`, `no-attach-info-json`, `mtime-by-default`, `filename` (`%(title)s-%(id)s.%(ext)s`), `format-sort` (`FormatSorter.ytdl_default` appended) and `no-keep-subs` change defaults, and an explicitly given option marks the compatibility option `*NAME` in `-v`'s `Compatibility options:` line. The others take effect where upstream reads them: `filename-sanitization` (youtube-dl's `sanitize_filename` with `is_id` from the field name), `allow-unsafe-exec-expansion` (no `--exec` template validation), `allow-unsafe-ext` (upstream's warning, and no unusual-extension check; path separators in an extension stay a native error), `embed-metadata` (`comment` is the description and `synopsis` is dropped), `format-spec` (`bestvideo+bestaudio/best` merge default), `list-formats` (the legacy `-F` layout), `playlist-index` (an entry's index is the one selected at its position before `--playlist-reverse`/`--playlist-random`), `prefer-vp9-sort` (`vcodec:vp9.2` in the default sort unless `format-sort`, and in YouTube's sort fields) and `no-direct-merge` (a partial download without ffmpeg downloads the whole file natively instead of aborting; merges FFmpegFD would download per format are a native error). `no-youtube-channel-redirect` (a channel URL without a tab is read as given, without the `/videos` redirect, tab checks, home-page rejection or conditional regional redirect) and `no-youtube-unavailable-videos` (no playlist reload with unavailable videos) are read by `youtube:tab` (see "YouTube playlists"). `no-certifi`, `prefer-legacy-http-handler`, `no-live-chat`, `no-external-downloader-progress` and `embed-thumbnail-atomicparsley` concern features this port does not have; pinned upstream never reads `no-youtube-prefer-utc-upload-date` or `seperate-video-versions`; and `playlist-match-filter` only affects unresolved (`url`) entries, which this port's playlists do not have. `manifest-filesize-approx` gives manifest formats `filesize_approx` too; `multistreams` enables both multistream options unless given (`*multistreams` when both were). `all` lists the options in this port's table order (upstream's set order varies) |
| Accepted tuning and environment options | `--file-access-retries`, `--throttled-rate`, `--no-update`, `--cache-dir`/`--no-cache-dir`, `--no-plugin-dirs`, `--remote-components`, `--prefer-insecure`/`--prefer-unsecure`, `--ignore-no-formats-error`, `--allow-dynamic-mpd`/`--no-ignore-dynamic-mpd`/`--ignore-dynamic-mpd`/`--no-allow-dynamic-mpd`, the account (`-u`, `-p`, `-2`, `-n`, `--netrc-*`, `--video-password`, `--ap-*`) and geo (`--geo-verification-proxy`, `--xff`, `--geo-bypass*`) options are validated with upstream's messages (`buffer size "0" must be positive`, `invalid file access retry count "X" given`, `invalid time range to wait for video "X" given`, ...) and otherwise have no effect: the supported extractors use no password logins or geo bypass; `--wait-for-video` applies to YouTube (see below). `--mark-watched` applies to YouTube when cookies are passed (`--cookies`/`--cookies-from-browser`), like upstream `_mark_watched`: `Marking watched` and `Marking fully watched` requests to the player response's videostats URLs with upstream's query updates (a random `cpn`), `Unable to mark [fully ]watched` warnings otherwise (`youtube-watched`). `--rm-cache-dir` prints upstream's `Removing cache dir DIR ..` for this port's (unused) `yt-dlp-rs` cache directory. `-U`/`--update`/`--update-to` are errors (no self-update). `--js-runtimes deno[:PATH]` and `node[:PATH]` select the YouTube challenge runtime (a bare name is looked up on PATH; Deno is preferred like upstream's provider preference): the pinned Deno without permissions, or Node.js 22+ under its permission model (`--permission`, or `--experimental-permission --no-warnings=ExperimentalWarning` before 23.5.0) reading the solver from stdin, as upstream's `NodeJCP` runs it; `bun` and `quickjs`, `--extractor-args` for extractors other than `generic`, unported YouTube arguments and `youtubetab` arguments other than `skip`/`approximate_date`, `--plugin-dirs`, `--legacy-server-connect` and `--bidi-workaround` warn that they are not supported natively. `--encoding` follows upstream `write_string` for `utf-8`, `ascii`, `latin-1`, `cp1252`, `utf-16-le`/`-be`, `utf-32-le`/`-be` and the BOM codecs `utf-16`/`utf-32` (a little-endian BOM before every message, as CPython writes on little-endian hosts; with Python's aliases): every message, `--print`, JSON and listing line is encoded with `errors='ignore'`, while progress lines, the `[debug] Encodings:` line (whose `pref` names the encoding) and child processes' output are not; other codecs warn and keep UTF-8 (upstream raises `LookupError` for unknown names). `--impersonate` reports upstream's unavailable-target error and `--list-impersonate-targets` its table of unavailable `curl_cffi` targets; `--ap-list-mso` lists no providers |
| `--client-certificate CERTFILE`, `--client-certificate-key KEYFILE`, `--client-certificate-password PASSWORD` | TLS client authentication for every request: a PEM certificate (optionally containing its key) plus an optional separate PEM key. Encrypted keys and `--client-certificate-password` are an explicit native error, because the rustls transport needs unencrypted keys |
| `--enable-file-urls`, `file:` URLs | Upstream refuses `file:` URLs after `[generic] Extracting URL`/`Downloading webpage` with `file:// URLs are disabled by default in yt-dlp for security reasons. Use --enable-file-urls to enable at your own risk.`; with the option, local media files are direct links like urllib's file handler gives upstream (guessed Content-Type, mtime timestamp, protocol `file`) and are copied with progress. Local HTML pages and manifests remain an explicit native error |
| `--http-chunk-size SIZE`, `--test` | Upstream `HttpFD` chunking for whole-file HTTP media: each request asks for `bytes=START-END` of a randomized 95-100% chunk (`--http-chunk-size`, else the format's `downloader_options.http_chunk_size`, which YouTube HTTPS formats set to 10 MiB like upstream); a matching `206` continues with the next chunk, other responses restart as a whole download (`Unable to resume` past the start), and the size limits compare against each response's end like upstream. Hidden `--test` keeps the first 10241 bytes (one `bytes=0-10240` request, even if Range is ignored). Request patterns compared with pinned upstream; fragments and FFmpeg downloads ignore `--test` |
| `--download-sections REGEX` | Upstream `parse_chapters`/`download_range_func`: `*START-END` ranges (comma-separated, `parse_duration` timestamps, `inf`, negative values from the end when the duration is known), `*from-url` and chapter regexes, with upstream's usage errors (regex error text is this port's). Each range is processed as its own video with `section_start`/`section_end`/`section_title`/`section_number` (available to templates, `--print` and JSON) after `[info] ID: Downloading N time ranges: S-E`. Partial downloads use upstream's FFmpegFD: FFmpeg reads the format URLs with `-headers` (defaults, User-Agent, page Referer, cookies), `-ss`/`-t`, `-c copy`, `-map I:0` for merged pairs, `-f` from `EXT_TO_OUT_FORMATS` (`mp4`/`mpegts` for HLS) and `-loglevel quiet` in quiet modes, into `FILE.part`; its own output is inherited, then `[download] 100% of SIZE in TIME at SPEED`; no fixups follow. Without ffmpeg: `ERROR: You have requested downloading the video partially, but ffmpeg is not installed. Aborting`. Chapter regexes search the chapter titles (`section_title`, zero-based `section_number`); without matches upstream's `There are no chapters matching the regex` or, for videos without chapters, `Cannot match chapters since chapter information is unavailable` is printed and nothing is downloaded. `*from-url` downloads between the extracted `start_time` and `end_time` (YouTube's `t`/`start`/`end` URL parameters, fragment before query, parsed with `parse_duration` like upstream), or the whole video when neither is set. DASH formats pass the manifest URL with `-map I:manifest_stream_number` like upstream (numbers compared with upstream on the saved DASH fixtures; actual DASH input needs an FFmpeg build with its `dash` demuxer, as upstream does) |
| `--fixup POLICY`, `--hls-use-mpegts`, `--no-hls-use-mpegts` | Upstream `fixup()` for unmerged downloads (also before postprocessors that skip, such as `-x` of an m4a; not when a remux/recode converts to another container, upstream's `postprocessed_by_ffmpeg`, which leaves only FixupStretched): HLS (`hlsnative`, unless `--hls-use-mpegts`) MP4-labeled files that are MPEG-TS get `FFmpegFixupM3u8PP` (`[FixupM3u8] Fixing MPEG-TS in MP4 container of "FILE"`, stream copy `-map 0 -dn -ignore_unknown -c copy -f mp4`, `-bsf:a aac_adtstoasc` when ffprobe reports AAC, via `NAME.temp.EXT`); `m4a` downloads whose `container` is `m4a_dash` (DASH representations and YouTube audio-only formats, which carry upstream's `EXT_dash` containers) get `FFmpegFixupM4aPP` (`[FixupM4a] Correcting container of "FILE"`). A `stretched_ratio` other than 1 first gets `FFmpegFixupStretchedPP` for every download, merges and FFmpegFD included (`[FixupStretched] Fixing aspect ratio of "FILE"`, a stream copy with `-aspect RATIO`, `Skipping audio` for audio-only results, `ID: Non-uniform pixel ratio RATIO` warnings); the ratio is the info dict's (a single format's own value replaces it; a merge takes its video format's). Single DASH (`dashsegments`) downloads get `FFmpegFixupDuplicateMoovPP` (`[FixupDuplicateMoov] Fixing duplicate MOOV atoms of "FILE"`, a stream copy into the same container), since every MPD format is `is_dash_periods` upstream. Without ffmpeg upstream's `ID: MESSAGE. Install ffmpeg to fix this automatically` warning; `warn` only warns, `never`/`ignore` skip; `detect_or_warn` (default) and `force` fix new downloads (already-downloaded files are not re-fixed). MPEG-TS is detected from the file's bytes rather than ffprobe `format_name`. Merges and other FFmpeg inputs also choose demuxers from the bytes. Processed downloads (`-x`, embedding, recoding) are fixed up the same way before their processors run, like upstream's `fixup()` before `post_process`. Processed merges download upstream's `NAME.fID.EXT` intermediates beside the output and print the `[Merger]` and deletion lines before the processors run |
| `--video-multistreams`, `--audio-multistreams` (and `--no-…`) | Upstream `allow_multiple_video_streams`/`allow_multiple_audio_streams`: `_merge` keeps every selected video/audio stream (`-f bv+ba+ba.2`), the default merge spec becomes `bestvideo+bestaudio/best` with audio multistreams, and the merger maps each input like `FFmpegMergerPP` (`-map N:a:0` for an input with audio, then `-map N:v:0` for one with video). The merged `tbr` is CPython's compensated `sum()` |
| `--merge-output-format FORMAT` | Upstream `/`-separated container preferences passed to `get_compatible_ext` for merges (`mp4/mkv`); validated with upstream's prefix-matching `re.match` against `avi`/`flv`/`mkv`/`mov`/`mp4`/`webm` (`invalid merge output format "X" given` usage error); ignored without a merge |
| `--post-overwrites`, `--no-post-overwrites` | Upstream `nopostoverwrites`: processor outputs replace existing files by default; `--no-post-overwrites` makes audio extraction keep an existing output whose original is on disk (`[ExtractAudio] Post-process file FILE exists, skipping`) and keep the source; last flag wins; see audio extraction below |
| `-k`, `--keep-video`, `--no-keep-video` | Retain completed processing sources/intermediates and, for unprocessed merges, upstream's `NAME.fID.EXT` format files; accepted without other options like upstream; last flag wins |
| `--ffprobe-location PATH` | Native option: explicit audio probe executable, overriding the ffprobe found from `--ffmpeg-location` or `PATH` |
| `-I`, `--playlist-items ITEM_SPEC` | Ordered, deduplicated inclusive playlist indices/ranges, signed indices and steps; fully extracted HTML5 playlists only |
| `--playlist-start NUMBER`, `--playlist-end NUMBER` | Inclusive one-based defaults 1 through end, end -1 unbounded; items option overrides after validation |
| `--playlist-reverse`, `--no-playlist-reverse` | Reverse selected request order after deduplication; last flag wins |
| `--autonumber-start NUMBER`, `--autonumber-size NUMBER` | Hidden upstream options: `%(autonumber)s` counts processed videos from the start (default 1), zero-padded to the size (default 5); `%(video_autonumber)s` counts every entry; upstream's validation messages |
| `--id` | Hidden deprecated upstream option: `-o "%(id)s.%(ext)s"` with its deprecation warning, or `--id is ignored since --output was given` |
| `--print-json`, `--all-subs` | Hidden deprecated upstream aliases: quiet JSON printing without simulation; `--sub-langs all --write-subs` |
| Removed upstream switches | `--call-home`, `-C`, `--dump-user-agent`, `--sponskrub*`, `--prefer-ffmpeg`, `--write-annotations` and the other options in upstream's deprecated list are accepted and dropped (with their values) and reported once with upstream's `Deprecated Feature:` notice |
| `--no-playlist`, `--yes-playlist` | Upstream `noplaylist`: a YouTube `watch?v=ID&list=...` URL downloads just the video (`[youtube:tab] Downloading just the video ID because of --no-playlist`); without it the playlist is extracted (`Downloading playlist LIST - add --no-playlist to download just the video ID`, see "YouTube playlists"). HTML5 playlists are unaffected, as upstream's generic extractor ignores the option |
| `--flat-playlist`, `--no-flat-playlist` | Accepted; supported playlists contain fully extracted entries rather than URL results, which upstream processes either way |
| `--skip-playlist-after-errors N` | Upstream `__process_playlist` `failures`: after N entries fail extraction/selection, `ERROR: Skipping the remaining entries in playlist "TITLE" since N items failed extraction`; 0 is unlimited |
| `--playlist-random` | Upstream `random.shuffle` of the selected entries (clock-seeded; `playlist_autonumber` follows the new order); `--playlist-reverse is ignored since --playlist-random was given` |
| `--lazy-playlist`, `--no-lazy-playlist` | Upstream lazy semantics on the eagerly extracted playlist: entries keep their order, `n_entries` is unknown (`Downloading N/A items`, `item N of N/A`, `NA` in templates), `-J` has no `requested_entries`, and reverse/random are ignored with upstream's `is ignored since --lazy-playlist was given` warnings (printed after date conflicts, unlike upstream's order) |
| `--download-archive FILE` | Read recorded extractor/video IDs, skip before selection (`[download] ID: TITLE has already been recorded in the archive`), record after successful download/merge publication or an already-downloaded file; no writes in metadata/listing/simulation unless `--force-write-archive` |
| `--no-download-archive` | Disable the archive; last of it and `--download-archive` wins |
| `--break-on-existing`, `--no-break-on-existing` | Stop at the first archived video (earlier playlist entries are still processed), print upstream's cancellation message and exit 101 |
| `--break-per-input`, `--no-break-per-input` | `--max-downloads`/`--break-on-existing` only end the current input and reset the count |
| `--force-write-archive`, `--force-write-download-archive`, `--force-download-archive` | Record archive entries for successfully simulated or skip-download videos too |
| `--extractor-retries` | Additional whole-extraction attempts, integer 0–255, default 0; no `infinite` value |
| `-h`, `--help`, `-V`, `--version` | Clap help and crate version |
| All other flags | Rejected; general option parity is not implemented |

Upstream `__process_playlist` progress lines are printed for playlists
(`[download] Downloading playlist: TITLE`, `[EXTRACTOR] Playlist TITLE:
Downloading N items of COUNT`, `[download] Downloading item I of N` and
`[download] Finished downloading playlist: TITLE`), as is `[info] ID:
Downloading 1 format(s): FORMAT` after selection. Archived or filtered entries
are reported before the first item rather than interleaved, and the updated
playlist info JSON is not rewritten after the entries are processed.

Informational `to_screen` lines and progress go to stdout like upstream's
`_out_files`, or to stderr when the default output template is `-`
(`logtostderr`); `WARNING:`/`ERROR:` lines
and the STDIN batch prompt use stderr. Option validations that upstream reports through `parser.error`
(audio/remux/recode/subtitle/thumbnail formats, audio quality, playlist
start/end/items, sleep intervals, file sizes, rate limit, dates and a missing
URL) print upstream's blank line, `Usage: yt-dlp-rs [OPTIONS] URL [URL...]`,
`yt-dlp-rs: error: MESSAGE` with upstream's message, and exit 2. Malformed
option values rejected by clap (for example non-numeric `--retries`) keep
clap's error text, also with exit status 2; other startup failures print
`ERROR: MESSAGE` and exit 1.
Generic extraction prints upstream's `[generic] Extracting URL: URL` (truncated
like `truncate_string(url, 100, 20)`), `[generic] ID: Downloading webpage` with
`_generic_id`, the `Falling back on generic information extractor` warning when
the Content-Type is not a direct media type, then `Extracting information`
(HTML and MPD), `Downloading m3u8 information`/`Checking m3u8 live status`
(HLS; like upstream `_extra_manifest_info`, the first `m3u8_native` format's
playlist is fetched again and a missing `duration` becomes `int(sum(EXTINF))`;
live playlists stay unsupported) or the `URL could be a direct video link` warning (suffix-only media), and
`[html5] ID: Downloading m3u8 information`/`Downloading MPD manifest` for HTML5
manifest sources. Simulation prints no native per-video summary line.
Simulation and JSON still make network requests. This inventory does not claim
upstream equivalence for every option interaction or filename rule.

## YouTube watch-page subset

The built-in `Youtube` extractor runs before the generic HTTP fallback and
takes the URLs of pinned upstream's `YoutubeIE._VALID_URL` (ported with its
`_INVIDIOUS_SITES` list, `src/extractor/youtube/urls.rs`): watch URLs (also
`watch_popup`, `.php`, `movie`, `?v=`/`#v=`/`#!v=` on other paths and `#/`
redirects), `/v/`, `/e/`, `/embed/`, `/shorts/` and `/live/` paths, short links
(`youtu.be`, `vid.plus`, `zwearz.com/watch`, Invidious hosts), `gaming.`/other
subdomains, `youtubekids.com`, `youtube-nocookie.com`, `youtube.googleapis.com`,
`hooktube`/`yourepeat`/`pwnyoutube`/`deturl`/`cleanvideosearch` forms,
protocol-relative `//` URLs and naked 11-character IDs (`yt-dlp-rs BaW_jenozKc`),
except URLs with a `list` parameter, like upstream's `suitable`. Its `Extracting
URL` line shows the input; watch-page requests use the canonical HTTPS watch URL.

The other YouTube extractors follow upstream's patterns and order and return
upstream's `url` results: `youtube:playlist` (naked playlist IDs such as `PL…`,
`UU…`, `RD…`, `WL`, `LL`, `LM`, and other `?list=` URLs like
`/embed/videoseries?list=`, to `/playlist?…` with the URL's query),
`YoutubeYtBe` (`youtu.be/ID?list=…` to `watch?v=ID&list=…&feature=youtu.be`),
`YoutubeLivestreamEmbed` (`/embed/live_stream?channel=ID` to
`/channel/ID/live`), `youtube:user` (`ytuser:NAME`), the `:ytfav`,
`:ytwatchlater`, `:ytrec` (and the `youtube.com` home page), `:ytsubs` and
`:ythis` keywords (favorites, subscriptions and history first require account
cookies, with upstream's login message), `youtube:shorts:pivot:audio`
(`/source/ID/shorts`, with upstream's `sfv_audio_pivot` browse params) and
`youtube:consent` (`consent.youtube.com/m?continue=…` follows its `continue`
URL, else `Invalid cookie consent redirect URL`). `youtube:truncated_id` and
`youtube:truncated_url` raise upstream's `Incomplete YouTube ID` and `Did you
forget to quote the URL?` errors. A `youtube:playlist` result for a
`music.youtube.com` input targets the music playlist URL instead of upstream's
smuggled `is_music_url` (the tab's `Extracting URL` line differs).
Remaining URLs on the exact `youtube.com`, `www.youtube.com`, `m.youtube.com`,
`music.youtube.com`, `youtu.be`, `www.youtu.be`, `youtube-nocookie.com` and
`www.youtube-nocookie.com` hosts are still claimed with an unsupported-YouTube
error before any request (upstream's generic extractor would download them),
so unsupported site pages cannot appear to be supported videos.
`tests/fixtures/sites/youtube-urls/` compares these shapes with pinned
upstream. `/live/` is a URL shape; live, upcoming and post-live videos are
supported as described below.

The info dict follows upstream `_real_extract` after `_list_formats`, in its
key order, over every player response, the page's meta tags and `ytInitialData`:
- **From the player responses:** title, original (else translated, else
  initial-data) description, duration, view count, average rating, age limit,
  categories, keywords (else `og:video:tag` metas) as tags, playability, live
  status and media type.
- **Thumbnails:** every response's saved ones and the page's image, then the
  generated `i.ytimg.com` names with their preferences and URL deduplication.
  The best saved thumbnail is `thumbnail`.
- **Captions:** automatic captions (the manifests' subtitles first) and
  subtitles.
- **URL times:** the URL's `t`/`start`/`end` times.
- **Music metadata:** from YouTube Music's auto-generated descriptions.
- **From the initial data:** the comment count, a `live_chat` track,
  chapters and heatmap. `--sub-langs live_chat` downloads it like upstream's
  `YoutubeLiveChatFD`: the watch page's chat continuation, the chat page, then
  `live_chat/get_live_chat_replay` (or `get_live_chat` for a live stream,
  until it ends) InnerTube pages, written as one JSON line per action
  (`tests/fixtures/sites/youtube-live-chat/`, compared byte for byte).
- **From the primary info renderer:** location or series/season/episode,
  like counts and the view count.
- **From the secondary info renderer:** the channel, followers, collaborators
  as creators, license/album/artist/song rows and the verified badge.
- **Uploader, timestamp and availability:** uploader and handle, the
  `uploadDate` timestamp (only with a time and zone) and date (else the
  `dateText`), and availability from the badges.

Captions follow upstream's caption loop over the client responses with
streaming data:
- **Tracks:** each subtitle format's `fmt` URL (without `xosf`), carrying
  upstream's `impersonate` and client keys.
- **Translations:** `translationLanguages` from every response; translated
  manual captions only with `--write-auto-subs`/`--list-subs`.
- **`-orig` tracks:** for the original language, which also becomes the audio
  formats' language.
- **PO tokens:** tracks requiring a PO token (`exp=xpe`/`xpv`) are skipped
  unless `po_token=CLIENT.subs+TOKEN` gives one (`pot`, `potc=1` and `c` are
  added to every track URL), with upstream's skip and missing-languages
  messages (warnings for non-default clients when subtitles are wanted).

Storyboards become `sbN` `mhtml` formats with their fragments, downloaded like
upstream's `MhtmlFD` (`[mhtml] Total fragments: N`): the MIME headers with the
quoted-printable `Subject`, the HTML stub with upstream's stylesheet and slide
captions, one image part per fragment (type sniffed like `imghdr`, else
`jpeg`), a random `uuid4` hex boundary kept in the `.ytdl` `extra_state` for
resuming, and `--test`'s single fragment. Fragment requests use `--retries`
like upstream's HttpFD, and any other failure (such as a 404) ends the download
with `unable to download video data`. The generator meta names this port
(`yt-dlp-rs VERSION`) instead of upstream's version; otherwise the
`youtube-storyboard` fixture's file is byte-identical to pinned upstream's
(boundary aligned). This applies to Twitch's storyboards too. Relative `dateText` values are not resolved. HTML5 media `poster` attributes become thumbnails.
Generic results follow upstream `GenericIE`: the first response's
`Last-Modified` header becomes `timestamp` through a port of
`unified_timestamp` (pinned `DATE_FORMATS`, `extract_timezone`,
`email.utils.parsedate_tz`; checked against actual upstream for 115 inputs;
non-ASCII digits are not interpreted and inputs where upstream raises yield no
timestamp), and every video derives `upload_date` from `timestamp` like
`_fill_common_fields`. A single-entry HTML5 page also takes
`og:description` (unescaped) as `description`, `og:image` verbatim as the
thumbnail when no poster exists, and `_rta_search` markers as `age_limit`
(playlists do not represent these page-level fields).
An RSS document (`<rss>`, also served as XML) is upstream's `_extract_rss`
playlist: each `channel/item` is a `url_transparent` entry (its first
enclosure URL, else its link, smuggled with the `guid` as `force_videoid`)
extracted when the entry is processed and overlaid with the item's title,
description, `pubDate`, iTunes duration/image/title/episode/season/explicit
fields; `--flat-playlist` prints the raw entries unextracted, and numbered
episodes, seasons and chapters get upstream's `Episode N` titles.
A Smooth Streaming manifest (`SmoothStreamingMedia`) is the generic info dict
with `_parse_ism_formats_and_subtitles`: H264/AVC1 video and AACL/EC-3 audio
tracks as `protocol: ism` formats (`NAME-KBPS` IDs, int `tbr`, ISO 639-2
languages, `_download_params`) and TTML text tracks as `ismt` subtitles, each
with fragments from the `c` timeline (`t`/`d`/`r`, a missing duration taking
upstream's stream-end fallback); other codecs warn `FOURCC is not a supported
codec`, and live manifests have no formats. ISM formats download through a
port of upstream's `IsmFD` (`[ism] Total fragments: N`): the fragments
appended raw through the fragment downloader after the PIFF `ftyp`/`moov`
header built from `_download_params` and the first fragment's `tfhd` track ID
(byte-identical to upstream's `write_piff_header` apart from its `time.time()`
creation times), with `ism_track_written` in the `.ytdl` state; `--test` takes
the first fragment, and selected video+audio pairs merge with ffmpeg.
An F4M manifest (`{http://ns.adobe.com/f4m/1.0|2.0}manifest`, ampersands fixed
like `fix_xml_ampersands`) is the generic info dict with `_parse_f4m_formats`:
unencrypted `media` entries as `protocol: f4m` formats (`flv` for stream-level
manifests with `bootstrapInfo`; otherwise each `href`/`url` joined to the
`baseURL` or manifest directory), `bitrate` as the format ID and int `tbr`,
an audio `mimeType` as `vcodec: none`, and none for an Akamai player
verification challenge (upstream's `No video formats found!`). Nested
`.f4m`/`.m3u8` media URLs are unsupported errors. F4M formats download through
a port of upstream's `F4mFD`: `[f4m] Downloading f4m manifest`, the media of
the requested bitrate (else the highest), DRM checks, inline or URL bootstrap
info (`abst`/`asrt`/`afrt`), `SegN-FragM` fragment URLs with the media's query,
the Akamai `pv-2.0` token and `extra_param_to_segment_url`, then the FLV header,
metadata tag and each fragment's `mdat` payload through the fragment
downloader (`.part`/`.ytdl` resume, retries, `--keep-fragments`; `--test`
takes the first fragment). Live HDS streams and merged F4M formats are
unsupported errors.
An XSPF document (`{http://xspf.org/ns/0/}playlist`) is upstream's untitled
`_parse_xspf` playlist: each `trackList/track` is an entry with the page's ID,
its title (else the ID), annotation, image and millisecond duration, and its
`location`s (joined to the final URL, empty ones skipped) as formats with the
`s1:label`/`s1:width`/`s1:height` attributes and the document as
`manifest_url`. XSPF URLs found on HTML pages remain unsupported.
A SMIL document (root `smil`, any namespace) is upstream's `_parse_smil` result
for HTTP sources: `video`, then `audio`, then `media` elements (repeats
skipped) joined to the head `base`/`httpBase` (else the document URL) as
`http-BITRATE`/`http-N` formats with `system-bitrate`/1000 as `tbr`, size,
width and height; absolute sources get `_is_valid_url`'s `Checking video URL`
request and are skipped with its `URL is invalid` notice. Head metadata gives
the title (else the URL basename), description/abstract and `unified_strdate`
date; `image`s are `{id, url, width, height}` thumbnails and `textstream`s are
subtitles by language (default `en`), merged after the manifests' tracks. HLS
sources (`proto="m3u8"` or `.m3u8`, the `ext` attribute as the formats' `ext`)
and DASH sources add their manifests' formats (`hls-`/`dash-` IDs,
`fatal=False` with upstream's `Failed to download ...` warnings); a playlist
with one format takes `hls-BITRATE`/`hls-N`, `tbr`, width and height. RTMP,
F4M and ISM sources, imagestream storyboards, extension-less sources
(upstream's HEAD request) and relative text streams are unsupported errors.
A single SMIL URL found on an HTML page (`Downloading SMIL file`) is the same
result overlaid with the found entry's ID, page title, domain uploader, age
limit and Referer, then merged with the page fields without its `None`
values; several found SMIL URLs and found XSPF URLs (an upstream crash) are
unsupported errors.
A page without HTML5 media first tries upstream's JW Player setup data
(`_find_jwplayer_data` with a port of `js_to_json`, checked against pinned
upstream conversions in `tests/fixtures/js_to_json/`): one playlist item's
direct sources become formats (label-derived heights, `bitrate` as `tbr`,
audio types as audio-only), with `tracks` captions, title, description, image,
`pubdate`, int or float `duration` and the other `_parse_jwplayer_data` fields,
merged with the page fields; HLS and DASH sources add their manifests'
formats (`fatal=False`, warning that manifest subtitle tracks are ignored).
JW Player playlists, playlist URLs, SMIL, RTMP and YouTube sources are
unsupported errors. A Video.js embed
(`videojs(...)...VAR.src([...]);`) comes next: like upstream, only the first
direct source becomes a format (with the page as Referer), and
`VAR.addRemoteTextTrack({...}, true|false)` tracks become subtitles; HLS and
DASH sources add their manifests' formats and subtitles (`hls-`/`dash-` IDs)
with `_extra_manifest_info`'s live check; YouTube sources are unsupported. Then JSON-LD: `_search_json_ld`'s
`VideoObject`/`AudioObject`, episode, movie and article fields (checked against
saved pinned results in `tests/fixtures/json_ld/`) with an absolute media URL
other than the page make upstream's `url_transparent` result: the smuggled URL
(shown as upstream prints it) is extracted again by the generic extractor with
the page's ID and Referer, then overlaid with the JSON-LD and page fields;
media that is not direct HTTP media, or that has an `encodingFormat`, is an
unsupported error. Then come `_extract_embeds`' URL patterns in upstream order: SWFObject `flashvars`, JW Player `file:` options, `file=`/
`source=` parameters, the JW Player JS loader, Flow Player, Cinerama, Twitter
`player:stream` cards and `og:video`/`og:audio` (with `check_video`'s image and
subtitle exclusions), then `http-equiv="refresh"` redirects and `twitter:player`
iframes (`[redirect] Following redirect to URL`, extracted again). Found
direct media URLs become upstream's entries (basename ID, the domain as
`uploader`, the page title, a Referer header; one URL merged with the page's
`timestamp`/`description`/`thumbnail`, several as a playlist with numbered
titles); found HLS or DASH URLs are requested with the page as Referer and
get `_extra_manifest_info` (XSPF/F4M, RTMP and YouTube URLs are
unsupported errors). Before JSON-LD, a KVS player (`kt_player.js`/`.swf`
version, with upstream's untested-major-version warning) is `_extract_kvs`:
the `flashvars` object (`_search_json` with `js_to_json`), canonical-link
`display_id`, `<title>`/`<h1>` title, protocol-fixed preview thumbnail, and
`video_url`/`video_alt_urlN` `/get_file/` URLs (unscrambled with
`_kvs_get_real_url`'s license token) as `mp4` formats labeled by their
`_text`, with `parse_resolution` dimensions or `quality` 1 and the page as
Referer, merged with the page fields. Output templates render a JW
Player integer `tbr` as a float (JSON keeps upstream's int). Otherwise
the result is upstream's `ERROR: Unsupported URL: URL`.
All videos then get upstream `_sanitize_thumbnails` ordering, IDs, resolutions
and `thumbnail` finalization (no `--check-formats` URL testing). These
fields appear in JSON, info JSON, match filters and (scalar ones) output
templates; see [the source slice](../upstream/notes/youtube-metadata.md).

RTRFM shows (`rtrfm.com.au/shows|show-episode/ID`) follow upstream `RTRFMIE`:
the page's fatal `playShow[From]` show, date and title, the restream API's
`u` MP3 URL (`Downloading MP3 URL`) as an untyped result, upstream's expected
`Expired or no episode on this date` error for an MP4 URL, the `SHOW-DATE`
ID, `TITLE DATE` title, series, release date (and the `release_year`
processing derives from it) and OpenGraph description.

Rumble embeds (`rumble.com/embed/ID`) follow upstream `RumbleEmbedIE`: the
embed JS API's `ua` formats (progressive by height, audio, timeline; HLS
manifests non-fatally; tar archives skipped), live status, captions,
thumbnails and metadata, with the API's `sys.msg` warning. Rumble video
pages (`rumble.com/vID-SLUG.html`) follow `RumbleIE`: the page's embed (or
`Rumble("play", ...)` script) as a `url_transparent` result with the page's
release time, view, like and dislike counts and description. Channels are
not supported. `tests/fixtures/sites/rumble/` is compared with pinned
upstream output.

Rutube videos (`rutube.ru/[live/]video[/private]/ID`, `[play/]embed/ID`
with 32-character IDs) follow upstream `RutubeIE`: the video API (the URL's
query, such as a private `p` token, passed on) for metadata and description
chapters, and the play options' balancer HLS and progressive formats, live
HLS streams and captions (`ru` without a code). Numeric embeds
(`rutube:embed`) read the options' `effective_video` first and report
`extractor_key` `Rutube`. F4M balancer entries are an unsupported error;
channels, playlists, tags, people and movies are not supported.
`tests/fixtures/sites/rutube/` is compared with pinned upstream output.

ScreenRec shares (`screenrec.com/share/ID`) follow upstream `ScreenRecIE`: the
page's fatal `customUrl` manifest as HLS formats (subtitles ignored with
upstream's warning), the OpenGraph title (else the `<title>` text) and the
OpenGraph description and thumbnail with upstream's warnings.

Sen videos (`sen.com/video/ID`) follow upstream `SenIE`: the content API's
`player` node video URL (else the `vod.sen.com` manifest) as fatal HLS
formats (its subtitles ignored with upstream's warning), and the first
`details` node's title (else the generic title, after `fulltitle`), first
description and string badge tags.

Snapchat Spotlight snaps (`[www.]snapchat.com/spotlight/ID`) follow upstream
`SnapchatSpotlightIE` (2026.10.21): the page's Next.js data (`Unable to
extract next.js data` without it), the spotlight story whose `storyId` is the
ID, and its `videoMetadata` as an untyped `mp4` result (`contentUrl`, width,
height, millisecond upload date and duration as floats, name, description,
share and view counts with `-1` as `None`, thumbnail and creator), then the
story's description, string hashtags as `tags` and engagement counts. Without
a matching story the result has no formats (upstream's generic-title warning
and `No video formats found!`). `tests/fixtures/sites/snapchat/` is compared
with pinned upstream output.

SoundCloud tracks (`soundcloud.com/USER/TRACK[/SECRET]`,
`api[-v2].soundcloud.com/tracks/ID`) follow upstream `SoundcloudIE`: the web
client ID from the home page's JS assets (last first; refreshed after a
401/403, kept for the run), the resolved track JSON, each requested
transcoding's (`http`/`hls` × `aac`/`opus`/`mp3`) stream URL (`abr` and DRM
transcodings skipped, previews deprioritized, `hls-aes` as m3u8), artwork
thumbnails with the HEAD-checked original, and the track's metadata. DRM-only
and geo-blocked tracks are upstream's errors. Original downloads, logins,
comments, sets, users, stations and search are not supported.
`tests/fixtures/sites/soundcloud/` is compared with pinned upstream output.

Streamable (`https://streamable.com/[e/|s/]ID`) follows upstream
`StreamableIE`: the `ajax.streamable.com` video JSON (`Downloading JSON
metadata`), its unavailable-status error, `reddit_title`/`title`, every file
with a URL as a format (proto-relative URLs as `https:`, `parse_codecs` of the
input codec names, `bitrate`/1000 as `vbr`), owner, float `date_added` and
`duration`, and plays. Site extractors are compared with pinned upstream's
output through a fixture-only proxy (`tests/fixtures/sites/`).

TED talks (`www.ted.com/talks/[lang/L/]SLUG`) follow upstream `TedTalkIE`:
the page's Next.js `videoData` (`Unable to extract next.js data` without
it), HLS (`hls-`), H.264 (`h264-RATEk`), RTMP and audio-download formats,
an external video as a `url` result when there are none, and the title,
presenter, thumbnail (resize query trimmed), description, duration, view
count, dates and tags with OpenGraph fallbacks. Upstream's `http-` bitrate
variants never apply (they look for `m3u8` rather than `m3u8_native`
formats), so none are made. Playlists (`TedPlaylistIE`) and series, or one
`#season_N` (`TedSeriesIE`), are playlists of talk `url` results;
`embed[-ssl].ted.com` URLs (`TedEmbedIE`) are rewritten to `www`.
`tests/fixtures/sites/ted/` is compared with pinned upstream output.

Telegram public posts (`t.me/CHANNEL/ID`) follow upstream
`TelegramEmbedIE` (2026.10.21): the post's embed frame (`?embed=1`, any
`single` parameter dropped; `Downloading embed frame`), each video player's
`<video>` source as an `mp4` format, its link's basename as the ID and
`?single=True` link as `webpage_url`, the `M:SS` duration and thumbnail
(upstream's non-fatal warnings when missing), and the message's cleaned text
on one line as title and description (processing's generic title without
text), author as `channel`, channel name and `<time datetime>` timestamp.
Posts with several videos are playlists (`CHANNEL-ID`, titled `AUTHOR ID`,
with the text as description) unless the URL has `single` or
`--no-playlist` is given, which pick the post's own video. A post without a
video of its own (text-only posts, or `--no-playlist` on a group whose ID is
not a video) is an unsupported error here; upstream warns that the extractor
returned nothing. `tests/fixtures/sites/telegram/` is compared with pinned
upstream output.

The Intercept Field of Vision posts (`theintercept.com/fieldofvision/SLUG`)
follow upstream `TheInterceptIE`: the page's `initialStoreTree` post with the
slug, as a `url_transparent` result of `jwplatform:VIDEO` (extracted by the
JW Platform extractor) overlaid with the post's display ID, raw title,
excerpt, ISO 8601 date and comment count; no matching post is upstream's
`Unable to find the current post`.

This American Life episodes (`thisamericanlife.org/radio-archives/episode/ID`,
`play_full.php?play=ID`) follow upstream `ThisAmericanLifeIE`: the episode
page's fatal `twitter:title`, description meta and OpenGraph image, with the
`ID_64k.m3u8` stream as an untyped `m3u8_native` `m4a` AAC 64k result.

SzTV pages (`sztv.hu`/`tvszombathely.hu` `.../SLUG-ID`) follow upstream
`SztvHuIE`: the fatal player `file` (under `media.sztv.hu/vod/`) and meta
title, the meta description and OpenGraph image with upstream's non-fatal
warnings, as an untyped result.

Megaphone players (`https://player.megaphone.fm/ID`, `megaphone.fm`) follow
upstream `MegaphoneIE`: the page's fatal `og:audio:title`/`og:audio:artist`
(`creators`), non-fatal `og:image` (with upstream's warning), and the
`var episode = {...};` script object through `js_to_json` (`mediaUrl`
proto-relative as `https:` as the single format, raw `duration`; missing keys are
upstream's `KeyError` extractor errors).

Angel episodes (`angel.com/watch/SERIES/episode/ID/season-N/episode-N/SLUG`)
follow upstream `AngelIE`: the page's JSON-LD `url` as a fatal HLS manifest
(`Downloading HD m3u8 information`), the OpenGraph title, description and
image (Cloudinary transformations removed), then `merge_dicts` with the
remaining JSON-LD fields after the extractor's own.

Internet Archive items (`archive.org/details|embed/ID[/FILE]`) follow
upstream `ArchiveOrgIE`: the embed page's `<play-av>` playlist (one entry
when the URL names a file; subtitle tracks stored under their labels, as
upstream does), the metadata API's item fields (title, joined description,
uploader, creators, license, dates, venue, year) and per-file fields, formats
for files with known media extensions (originals preferred through
`source_preference`, private files skipped without a `logged-in-sig`
cookie), thumbnails and reviews as `comments`. One entry is merged into the
item's dict; several make a playlist. Wayback Machine YouTube captures
(`web.archive:youtube`) are not supported. `tests/fixtures/sites/archiveorg/`
is compared with pinned upstream output.

AudioBoom posts (`audioboom.com/posts|boos/ID`) follow upstream `AudioBoomIE`:
the post page's `V5DetailPagePlayer` React props (`_search_json` with
`unescapeHTML`) for the clip URL, title, description (else its cleaned
formatted description), duration, author and author URL, each falling back to
upstream's OpenGraph/meta/regex lookups with their non-fatal warnings (the
audio URL's is fatal), as an untyped result.

BehindKink posts (`behindkink.com/YYYY/MM/DD/SLUG`, marked broken upstream with
its warning) follow upstream `BehindKinkIE`: the fatal `<source>` URL (its
basename before `_` as the ID), OpenGraph title/image/description with
upstream's non-fatal warnings, the URL's date as `upload_date` and
`age_limit` 18, as an untyped result.

Bandcamp tracks (`ARTIST.bandcamp.com/track/SLUG`) follow upstream
`BandcampIE`: the page's `data-tralbum` and `data-embed` JSON (the attribute
search is fatal, as upstream), the track's streaming files (`mp3-128` and
the like), and for free downloads the download page's `data-blob` formats
through their stat URLs' retry URLs, with the track, artist, album artist,
dates, duration and tags. A page without a title warns like upstream's
`_html_extract_title`; a client challenge page (which needs impersonation)
is an explicit error. Albums, artist pages and weekly shows are not
supported. `tests/fixtures/sites/bandcamp/` is compared with pinned
upstream output.

Bild clips (`bild.de/.../SLUG-ID[,auto=true].bild.html`) follow upstream
`BildIE`: the `,view=json` clip data, the first clip's `application/x-mpegURL`
sources as non-fatal HLS formats, `video/mp4` sources as `http-mp4` and an
`Skipping unsupported format type` warning for any other type, with the
unescaped stripped title, description, poster thumbnail and integer
`durationSec` duration.

Bilibili videos (`bilibili.com/video/BV…|av…`, festival `?bvid=` pages)
follow upstream `BiliBiliIE`: the page's `__INITIAL_STATE__` (with the
rate-limit, `-403`/`-404` and missing-state errors), anthologies as
playlists of their `?p=N` parts (`_yes_playlist`), the WBI-signed `playurl`
API (nav key, `_dm_params` fingerprint values) with its DASH audio/video and
legacy `durl` formats (each missing `accept_quality` requested on its own,
the premium-format note and incompatible-FLV drop), chapters from the
player API, `_old_archive_ids` and, with `--write-subs`/`--list-subs`, the
danmaku XML track. Logged-in `__playinfo__` use follows the `SESSDATA`
cookie. Interactive videos, multi-part FLV results, inline SRT subtitle
tracks, comments and bangumi/cheese/space/audio pages are not supported.
`tests/fixtures/sites/bilibili/` is compared with pinned upstream output
(the random fingerprint, `wts` and `w_rid` query values are ignored by the
fixture proxy).

BitChute videos (`[www.|old.]bitchute.com/video|embed|torrent/.../ID`)
follow upstream `BitChuteIE`: the beta API's `video/media` URL (HLS as live
formats, or a progressive file HEAD-checked on its own and upstream's
alternative seed hosts, unchecked with `--no-check-formats`), the `video`
and `channel` metadata (non-fatal, with upstream's warnings) and the API's
403 geo-restriction errors. Channel and playlist pages are not supported.
`tests/fixtures/sites/bitchute/` is compared with pinned upstream output.

Bitmovin streams (`streams.bitmovin.com/ID`) follow upstream `BitmovinIE`:
the player config's `sources.hls` as fatal HLS formats and subtitles with the
string title (else the generic title, after `fulltitle`) and poster, with
upstream's `KeyError` messages; its `_EMBED_REGEX` iframes on generic pages
are found first among the embeds (one is followed, several form a playlist).

Blogger videos (`blogger.com/video.g?token=TOKEN`, `blogger.com`) follow
upstream `BloggerIE`: the page's `VIDEO_CONFIG` line decoded like Python's
`unicode_escape`, its streams as formats (`mime`-query extensions, string
format IDs), the iframe ID as ID and title, the thumbnail and the first
stream's `dur`; its `_EMBED_REGEX` iframes join the generic embed detection.

Bluesky posts (`bsky.app/profile/HANDLE/post/ID`, `main.bsky.dev`, and
`at://HANDLE/app.bsky.feed.post/ID`) follow upstream `BlueskyIE`: the
public API's post thread, each video embed's HLS playlist plus its PDS blob
(the service endpoint from the PLC directory or `did:web` document, else
bsky.social) with captions, external links followed as `url` results, media
with records and quoted posts, and the post's metadata, labels and age limit.
Several videos form a playlist; posts mixing videos and links are not
supported. `tests/fixtures/sites/bluesky/` is compared with pinned upstream
output.

Breitbart videos (`breitbart.com/videos/v/ID`) follow upstream `BreitBartIE`:
the JW Player `manifests/ID.m3u8` as fatal HLS formats (subtitles ignored with
upstream's warning), the page's generic title (else the generic video title),
OpenGraph description and thumbnail (with upstream's warnings) and RTA age
limit.

CGTN news videos (`news.cgtn.com/news/YYYY-MM-DD/SLUG-ID/index.html`) follow
upstream `CGTNIE`: the fatal `data-video` manifest as `hls` formats (subtitles
ignored with upstream's warning), the date span (UTC+8) as `timestamp`, the
section as `categories`, the author as `creators`, and the OpenGraph title,
description and thumbnail, with upstream's warnings for missing values.

CJSW episodes (`cjsw.com/program/PROGRAM/episode/ID`) follow upstream
`CJSWIE`: the fatal episode heading (else `data-audio-title`) and audio button
source, the source's UUID ID (else `PROGRAM/ID`), the first paragraph as the
description, `data-showname` (else the program) series and the episode ID.

Coub clips (`coub.com/view|embed|coubs/ID`, the Flash player's `coubID`,
`coub:ID`) follow upstream `CoubIE`: the v2 API (`Coub said: ERROR` for its
errors), HTML5 video and audio files with upstream's quality and source
preferences, the iPhone file and mobile audio, and the clip's metadata.
`tests/fixtures/sites/coub/` is compared with pinned upstream output.

CozyTV replays (`cozy.tv/USER/replays/ID`) follow upstream `CozyTVIE`: the
`USER-ID` ID, the cached replay JSON's title (else the generic title), `user`
(else the URL user) uploader, `unified_strdate` date, raw duration and
`was_live`, with the CDN `index.m3u8` as fatal HLS formats and subtitles.

Crooks and Liars embeds (`embed.crooksandliars.com/embed|v/ID`) follow upstream
`CrooksAndLiarsIE`: the `http:` embed page's fatal `var manifest` JSON, its
`video/` flavors as formats ranked by `qualities(webm_low, mp4_low, webm_high,
mp4_high)`, the title, description, protocol-relative poster, integer
`created`/duration and author, with upstream's `KeyError` messages and a
processing-time error without formats; its iframe/param embeds on generic
pages follow Blogger's in the embed order.

CRTVG programmes (`crtvg.es/tvg/a-carta/ID`) follow upstream `CrtvgIE`: the
page's fatal `var url` base, its `playlist.m3u8` (without an `ext`) and
`manifest.mpd` as non-fatal formats (their subtitles ignored with upstream's
warning), the `og:title`/`twitter:title` meta without ` | CRTVG`, the
description and image metas, and a seven-digit ID suffix's old archive ID.

Culture Unplugged films (`cultureunplugged.com/[documentary/watch-online/]play/ID[/SLUG]`)
follow upstream `CultureUnpluggedIE`: the movie data JSON as an untyped
result with its URL and title (upstream's `KeyError` messages), synopsis,
producer `creator` (and the derived `creators`), integer duration and views
and the truthy small/large thumbnails with their preferences.

Dailymotion videos (`dailymotion.TLD/video/ID`, `dai.ly/ID`, embed, swf and
player URLs, `lequipe.fr`) follow upstream `DailymotionIE`: the family-filter
cookie, a client-credentials token, the GraphQL media object (its errors are
upstream's), the player metadata (geo and other errors), the qualities' HLS
manifests requested with upstream's randomized headers (the impersonated
retries are unavailable, as upstream without `curl_cffi`), including
`PROGRESSIVE-URI` `http-` formats, and progressive H264 URLs with sizes and
`@60` frame rates, plus subtitles, posters and metadata. Playlist URLs (and
`?playlist=` without `--no-playlist`) are not supported.
`tests/fixtures/sites/dailymotion/` is compared with pinned upstream output.

EbaumsWorld videos (`ebaumsworld.com/videos/SLUG/ID`) follow upstream
`EbaumsWorldIE`: the player config XML (`Downloading XML`) as an untyped
result, empty elements as `None` (a generic title for an empty title) and a
missing element as upstream's bare `AttributeError` text. XML parse errors
carry this port's parser message rather than expat's.

eBay items (`ebay.com/itm/ID`) follow upstream `EbayIE`: the page's fatal
`"video":` JSON `playlistMap`, its `HLS` (without an `ext`) and `DASH`
manifests as non-fatal formats (subtitles ignored with upstream's warnings),
an `Unsupported format KEY` warning for other keys, and the `<title>` without
` | eBay`; a result without formats fails in processing after the generic
title, like upstream's `raise_no_formats`.

The Epoch Times videos (`www.theepochtimes.com/SLUG_ID.html`) follow upstream
`EpochIE`: the `videobox` element (upstream's `html.parser`-based
`get_element_html_by_id`, with its parse errors) `data-id` attribute as the
ID, its YouMaker `playlist.m3u8` as fatal `hls` formats and subtitles, and
the cleaned `<title>` text (with upstream's warning when missing).

Facebook videos (`facebook.com` watch, video, post, story, permalink,
event, group and watch-party URL shapes and `facebook:ID`) follow upstream
`FacebookIE`'s paths in its order. First the server JS (`handleServerJS`,
`s.handle`) and `bigPipe.onPageletArrive` pagelet (`js_to_json`)
`VideoConfig` instances: each `videoData` entry's `sd`/`hd` `src` and
`src_no_ratelimit` URLs (upstream's qualities and 720p height), inline DASH
manifest and `subtitles_src` English subtitles; several instances form a
playlist of their `video_url` pages. Then the page's relay prefetched data,
the path current pages use: `parse_graphql_video` over node attachments,
media sets, event cover videos and the page's `video` (legacy `playable_url`
fields, inline or linked DASH manifests from `videoDeliveryResponseFragment`,
progressive and HLS URLs, captions by locale); several videos form a
playlist. Login and interstitial pages are upstream's errors. Watch parties
(`/watchparty/ID`) POST upstream's `livingRoom` GraphQL query (with the
page's `lsd` token and `RelayAPIConfigDefaults` endpoint) and list the
recap's videos; other pages fall back to the tahoe player POST and its
`for (;;);` payload, else upstream's `Cannot parse data` error. Formats get
upstream's `facebookexternalhit` User-Agent and 250 MiB HTTP chunks, merged
with `extract_metadata`'s JSON-LD and webpage title, description, uploader,
timestamp, thumbnail and counts (an empty media title or message, as on
reels, falls back to the page's `og:title`/meta description, and the
uploader fallback is the raw `og:title`, like upstream). DASH
representations with `SegmentBase`
indexes (as Facebook's often are) are refused by the shared DASH parser
with a warning, and `BaseURL`-only ones become one-fragment DASH formats,
rather than upstream's unfragmented `https` formats; comments are not
supported. `facebook:reel` (`/reel/ID`),
`FacebookPluginsVideo` (`plugins/video.php?href=`) and `FacebookRedirectURL`
(`flx/warn/?u=`, upstream's error without a valid `u` URL) are followed like
upstream. `facebook:ads` (`ads/library/?id=ID`) follows upstream
`FacebookAdsIE`: the ad snapshot (current `ad_library_main` and older
`deeplinkAdCard` paths), its title (else `FORMAT by PAGE`), markup or body
description, page uploader fields and like count, and each video or card's
SD/HD (watermarked) URLs as one video or a playlist; upstream's 403 `Client
challenge` verification retry is not ported (the HTTP error is reported).
`tests/fixtures/sites/facebook/` is compared with pinned upstream output.

Fathom shares (`fathom.video/share/ID`) follow upstream `FathomIE`: the `app`
element's `data-page` JSON props (upstream's `TypeError` text without them),
the call's ID and fatal `video_url` HLS formats (subtitles ignored with
upstream's warning), the head title (else the generic title, after
`fulltitle`), float duration and ISO 8601 `started_at` timestamp, with
upstream's `KeyError` messages.

FC Zenit videos (`fc-zenit.ru/video/ID`) follow upstream `FczenitIE`: the
page's `config` MSI video ID (fatal), then the player's `msi/video?video=ID`
JSON: name, preview, float duration, int date, labeled qualities with URLs as
formats (`int_or_none` label heights) and labeled tags.

23video players (`*.twentythree.net`/`23video.com`/`filmweb.no`
`v.ihtml/player.html?...photo_id=ID`) follow upstream `TwentyThreeVideoIE`:
the `api/photo/list` JSON (the player query re-encoded like
`update_url_query` with `format=json`, the body's `{...}` taken like its
`transform_source`) with the audio and `mobile_high`/`medium`/`hd`/`1080p`/`4k`
downloads as formats and the named images as `{id, url, width, height,
filesize}` thumbnails. Filmweb articles (`filmweb.no/trailere|filmnytt/articleID.ece`,
`filmnytt` pages through their `data-videoid`) follow upstream `FilmwebIE`:
the trailer embed JSON's iframe as a `url_transparent` 23video result.

TikTok videos (`www.tiktok[v].com/@USER/video/ID`, `/embed/ID`,
`/share/video/ID`) follow upstream `TikTokIE`'s webpage path (no app API
without `app_info`): upstream's impersonation warning (this port cannot
impersonate a browser, like upstream without `curl_cffi`), the webpage's
`__UNIVERSAL_DATA_FOR_REHYDRATION__`, else the page's proof-of-work challenge
solved natively (SHA-256 over up to a million candidates, as
`_solve_challenge_and_set_cookies`) and the page refetched with its cookie.
Status codes 10216/10222 and 10204 and the `/login` redirect are upstream's
errors. `_parse_aweme_video_web` gives the bitrate renditions (URL-key IDs,
codecs, int bitrates, qualities and dimensions; `bytevc2` unplayable), the
`play` and watermarked `download` URLs, the music track as an audio format,
author, music, stats and cover thumbnails, with the page as every format's
Referer. Formats upstream marks `__needs_testing` (the watermarked download,
HEVC `media-video-hvc1` renditions) are tested when selected, as
`_check_formats` does by default, and `-F` lists them as `Untested`.
Subtitles, slideshows, user/sound/tag/collection pages and Douyin are not
supported. `vm.tiktok` short links (`vm.`/`vt.tiktok.com/ID`,
`tiktok.com/t/ID`) are resolved with a HEAD request and followed.
`tests/fixtures/sites/tiktok/` is compared with pinned upstream output.

Tweakers videos (`tweakers.net/video/ID`) follow upstream `TweakersIE`: the
`s1playlist/ID/1920/1080/playlist.json` item's progressive sources as formats
(labels as IDs, numbered when repeated, `mimetype2ext` or URL extensions),
with its description, poster, duration and account; an empty item list is
upstream's bare `IndexError`.

Twitch VODs (`twitch.tv/videos/ID`, `/CHANNEL/v/ID`, player and schedule
URLs) follow upstream `TwitchVodIE`: the persisted GraphQL metadata,
chapter-moment and storyboard operations, the video playback access token,
the usher HLS master (Twitch's `#EXT-X-TWITCH-*` tags are ignored, as
upstream does), the source rendition preferred, storyboard `mhtml` formats,
thumbnails, chapters and the URL's `t` start time. Twitch clips
(`clips.twitch.tv/SLUG`, `twitch.tv/[CHANNEL/]clip/SLUG`) follow
`TwitchClipsIE`: the clip GraphQL, signed default and portrait quality URLs,
thumbnails and metadata. Live streams, collections and channel video lists
are not supported. The usher URL's random `p` parameter makes VOD `-J`
output non-deterministic upstream too, so `tests/fixtures/sites/twitch/`
compares VODs through `--print` and `-F` (fixture routes may now ignore named
query parameters).

X/Twitter posts (`twitter.com`, `x.com`, `mobile.`/`m.`/`www.` and the onion
host: `NAME/status/ID`, `i/web/status/ID`, `statuses/ID`, optionally
`/video/N`) follow upstream `TwitterIE` with its default GraphQL API: a guest
token (`guest/activate.json`, unless an `auth_token` cookie is present, with
`ct0` as the CSRF token), then `TweetResultByRestId` and `_graphql_to_legacy`
(tombstones, unavailable/protected/NSFW reasons, visibility results,
retweets, quoted posts and card bindings). Each video's HLS variants (an
audio rendition's `tbr` from its `hls-audio-BITRATE` ID) and MP4 variants
(`http-KBPS` with dimensions from the URL), size-named thumbnails, duration
and upstream's `_format_sort_fields` are combined with the post's title
(`UPLOADER - TEXT`, links removed, 72 characters), description, timestamp,
counts, age limit and hashtags. Several videos form a playlist with `#N`
titles; `/video/N` selects one only with `--no-playlist`, as upstream's
`_yes_playlist`. Unified cards' videos are extracted and player/summary cards
and a post's only link are followed through the extractor registry;
broadcast, Spaces, Periscope and VMAP (amplify) cards, the legacy and
syndication APIs (and the rate-limit fallback to syndication) are not
supported. `twitter:card` (`i/cards/tfw/v1/ID`, `i/videos[/tweet]/ID`) is
followed to `statuses/ID`. `tests/fixtures/sites/twitter/` is compared with
pinned upstream output.

Vimeo videos (`vimeo.com/ID`, `player.vimeo.com/video/ID`, channel and
`user` clip URLs) follow upstream `VimeoIE` as it runs without login or
impersonation (with upstream's impersonation warning): player pages'
`playerConfig`, and vimeo.com pages' clip page config (or a channel page's
`data-config-url`) with its config JSON, through `_parse_config`
(progressive `http-QUALITY` formats, per-CDN HLS formats with audio
qualities from the group ID, per-CDN DASH formats including the `sep`
variants and `json=1` redirects, text tracks, thumbnails, chapters, live
status), merged with the page's description, timestamp, counts and license.
Unlisted-hash URLs and pages without a config need the logged-in API
client, which is upstream's login-required error; passwords, albums,
showcases, on-demand, reviews, events and original formats are not
supported. `tests/fixtures/sites/vimeo/` is compared with pinned upstream
output.

VK videos and clips (`vk.com`/`vkvideo.ru` `video|clip` IDs, `?z=` and
`list=` shapes, `video_ext.php` embeds) follow upstream `VKIE`: the
`al_video.php` `show` payload (`3` as login required, `8` as its message),
the embed page's error messages, security check and known error pages,
`playerParams`, the progressive `url*`/`cache*`, HLS (`hls*`), DASH
(`dash*`) and RTMP formats, subtitles, chapters (`time_codes` with int
times), the info page's date and view count and `_format_sort_fields`.
A YouTube embed is followed; Vimeo, Rutube, Dailymotion, Odnoklassniki and
Sibnet embeds and VK's JS challenge (which needs upstream's
`JSInterpreter`) are unsupported errors, and logins are not supported.

VK wall posts (`vk.com/wall-1_2`, `?w=wall-1_2`, `m.`/`new.`) follow
upstream `VKWallPostIE`: the `wkview.php` `show` payload's HTML as a
playlist (`UPLOADER - Wall post ID`, the `wall_post_text` description) of
its `data-audio` tracks (`m4a` formats, artist, track, cover thumbnails)
and the `wl_post_body` links to VK videos (`url` entries for `VKIE`; a post
without that element is upstream's `TypeError`). Upstream deduplicates the
links with a `set`, in hash order; this port keeps page order, so posts
linking several distinct videos may list them in another order.
VK user/club video sections (`vk.com/video/@NAME[/all][?section=S]`,
`vkvideo.ru/@NAME`) and playlists (`.../playlist/OWNER_ID`) follow upstream
`VKUserVideosIE`: the page's `newCur` cursor `oid` (or the playlist ID), the
`VideoInfoPanel__title` playlist title and `al_video.php`'s
`load_videos_silent` pages as `VKIE` `url` entries. Upstream fetches the
pages lazily while processing entries; this port fetches them all first.
VK Play Live records (`vkplay.live`/`live.vkplay.ru`/`live.vkvideo.ru`
`USER/record/ID`) and channels follow upstream `VKPlayIE` and
`VKPlayLiveIE`: `api.vkplay.live`'s `public_video_stream` JSON (a failure is
a warning) or the page's `initial-state` script, `playerUrls` HLS, DASH and
progressive formats (with the named resolutions), the common metadata and
`The channel is not currently live` for an offline channel. A live
stream's HLS master playlist is read without its media playlists.
`tests/fixtures/sites/vk/` is compared with pinned upstream output.

Weibo posts (`weibo.com/UID/ID`, `m.weibo.cn/status|detail/ID`) follow
upstream `WeiboIE`: the `ajax/statuses/show` JSON (a redirect to
`passport.weibo.com` runs the visitor `genvisitor`/`incarnate` handshake for
guest cookies, then retries), the playback list formats (or the media URLs'
`label=`/`template=` fallback, in document order where upstream iterates a
set), mixed-media posts as untitled playlists of their videos, and the
post's title (72-character truncation), alt title, uploader, counts, tags and
`_old_archive_ids`. `tv/show` and `video.weibo.com/show?fid=` pages follow
`WeiboVideoIE` through the TV component API to the post. User pages are not
supported. `tests/fixtures/sites/weibo/` is compared with pinned upstream
output.

XiaoHongShu (RedNote) notes (`www.xiaohongshu.com/explore/ID`,
`discovery/item/ID`) follow upstream `XiaoHongShuIE` (release 2026.10.21):
the page's fatal `window.__INITIAL_STATE__` (through `js_to_json`), every
codec's streams' `masterUrl` and `backupUrls` as formats with their FPS,
dimensions, codecs, scaled bitrates, channels, quality type (`format`), size
and duration, plus a `direct` format (`quality` 1) for the original upload
when `sns-video-bd.xhscdn.com/KEY` answers (otherwise upstream's
`Original video is not available` warning), the note's images as thumbnails,
the OpenGraph title overridden by the note's, description, tags and uploader
ID. Image-only notes have no formats and fail like upstream. Non-HTTP stream
URLs are refused; a malformed state fails without Python's `JSONDecodeError`
text. `tests/fixtures/sites/xiaohongshu/` is compared with upstream output.

Vocaroo recordings (`vocaroo.com`/`voca.ro` `[embed/]ID`) follow upstream
`VocarooIE`: the `media`/`media1` MP3 URL checked with a HEAD request
(`Downloading webpage`, the site as Referer), its `x-bz-upload-timestamp`
milliseconds as `timestamp`, and an empty title; an empty title gets
`_fill_common_fields`' generic title with its debug message (`fulltitle`
stays `''`).

Yorkshire Film Archive pages (`yfanefa.com/PATH`) follow upstream `YfanefaIE`
(`yfanefa`): the page's fatal `iwPlayer.options[...]` JSON, its URL joined
with the signature as fatal `hls` formats (subtitles ignored with upstream's
warning) or one `mp4` format, the `PATH` ID with slashes as dashes, the
OpenGraph title (else the `<title>` without ` | Yorkshire Film Archive`), and
the URL-valued preview and integer duration.

Zhihu videos (`zhihu.com/zvideo/ID`) follow upstream `ZhihuIE`: the zvideos
API's playlist qualities with a URL (`url` else `play_url`) as formats with
their sample rate, size, FPS, dimensions and bitrate, the fatal title, the
video thumbnail (else `image_url`), author name/ID and `people/` URL (`''`
without a token), timestamp, duration and counts.

Fuyin TV videos (`fuyin.tv/html/N/ID.html`) follow upstream `FuyinTVIE`: the
movie API's `data.url` as an untyped `mp4` result with its title (else the
generic title) and the non-fatal page's description meta (with its warning);
a failed page download warns and then fails with upstream's escaped
`TypeError` text.

Glide messages (`share.glide.me/ID`) follow upstream `GlideIE`: the page's
generic title, first `<source>` URL (else the fatal OpenGraph video URL) and
`video-thumbnail` image (else the OpenGraph image with its warning), each
protocol-relative URL as `https:`, as an untyped result.

GodResource videos (`new.godresource.com/video/ID`) follow upstream
`GodResourceIE`: the streams API's `streamUrl` as fatal HLS formats and
subtitles or one `mp4` format (other extensions are upstream's `Unexpected
video format`), the string title (else `''`, so processing's generic title),
`is_live`, URL-valued thumbnail, integer views, channel name and ID and
`unified_timestamp` creation/modification times; live streams are reported
as unsupported.

GodTube videos (`godtube.com/watch/?v=ID`, marked broken upstream with its
warning) follow upstream `GodTubeIE`: the player config XML (`Downloading
player config XML`: file, author, ISO 8601 date, duration, image) and media
XML title (`Downloading media XML`) as an untyped result.

Graspop concerts (`vod.graspop.be/LL/ID/`) follow upstream `GraspopIE`: the
Proximus festival stream's `source.assetUri` over `http:` as fatal HLS
formats, the string `name` title (else the generic title) and the URL-valued
`source.poster` thumbnail, with upstream's `KeyError` messages.

Harpodeon films (`harpodeon.com/video|preview/SLUG/ID`) follow upstream
`HarpodeonIE`: the fatal `hpBase` and `hpInjectVideo` MP4 as an untyped result
with the page as Referer, the `videoInfo` heading's title, creator (and the
derived `creators`) and year (one warning when absent), and the description
meta with its warning.

HistoricFilms tapes (`historicfilms.com/tapes/ID`, `/playID`) follow upstream
`HistoricFilmsIE`: the fatal tape ID, OpenGraph title/description, the
`thumbnailUrl` meta or OpenGraph image, the `duration` meta through
`parse_duration`, each non-fatal miss with upstream's `unable to extract`
warning, and the derived `.mov` URL as an untyped result. A missing title gets
`_fill_common_fields`' generic `EXTRACTOR video #ID` title with its warning
(`fulltitle` stays `null`).

Instagram posts and reels (`instagram.com[/USER]/{p,tv,reel,reels}/ID`, not
`/share/` links or reel audio) follow upstream `InstagramIE` as it runs
without browser impersonation (no `curl_cffi`): the session's LSD token from
the home page (`Setting up session`), the non-fatal `get_ruling_for_content`
check (with upstream's CSRF/access warnings and its restricted/error answers),
then, since the GraphQL request requires impersonation, the post page's
prefetched `xig_polaris_media` (a login redirect, private long shortcodes and
an empty response are upstream's errors). `_extract_product` gives the
`video_versions` formats, reversed image candidates, duration, the post's
user, caption, timestamp, counts and comments, and a carousel's videos as a
playlist (an entry without video formats is reported and the others
continue, like upstream). An inline `video_dash_manifest` adds upstream's
`dash-ID` formats when every representation has an absolute direct
`BaseURL` (its `SegmentBase` index is not read, as upstream; they are plain
HTTPS formats without a manifest URL); other inline manifests are an error
rather than a guess. With a `sessionid` cookie (`--cookies`), the media info
API (`Downloading video info`, with the `instagram:app_id` extractor
argument's API host) gives the product instead; a response redirected to the
login page is upstream's `cookies are no longer valid` warning, the cookie is
cleared and the logged-out path follows. Logged in, `--write-comments` reads
the comments API after the match filters (`Downloading comments info`,
non-fatal), replacing `comment_count`; comments of logged-in carousels
(upstream shares one generator between the entries) are an error.
`instagram://media?id=PK` (`InstagramIOS`) follows its `/tv/` post.
Stories and highlights (`instagram.com/stories/USER[/ID]`,
`instagram:story`) read the page's `"user":` JSON, then the
`feed/reels_media` API (a missing user or reel is upstream's login error);
each item with formats is an entry with the reel's uploader, and a user's
story ID is a playlist unless `--no-playlist` picks that item. Tag pages
(`explore/tags/TAG`, `instagram:tag`) and user pages (`instagram:user`,
marked broken upstream) read the page's `sharedData` and its GIS-signed
GraphQL timeline pages (trying upstream's signature templates on `403`) as
`url` entries of the video nodes; all pages are fetched before processing,
so a page error ends the extraction instead of upstream's partial playlist.
`tests/fixtures/sites/instagram/` is compared with pinned upstream output.

JoVE videos (`jove.com/video/ID`) follow upstream `JoveIE`: the page's fatal
chapters ID, the chapters XML's root `video` attribute (`Downloading chapters
XML`; none is upstream's `Failed to get the video URL`) as an untyped result,
the `citation_title`/`citation_publication_date` metas, OpenGraph thumbnail,
summary paragraph and comment count, each with upstream's warning, and the
escaped `int(None)` error without a comment count.

JW Platform media (`content.jwplatform.com`/`cdn.jwplayer.com`
`players|feeds|thumbs|previews|manifests|jw6|v2/media/ID`, and `jwplatform:ID`)
follow upstream `JWPlatformIE`: the `cdn.jwplayer.com/v2/media/ID` JSON through
the ported `_parse_jwplayer_data` with `require_title=True` (a missing title is
upstream's `KeyError`), the entry returned with its `None` fields, its HLS/DASH
sources requested under `[JWPlatform]`, and caption tracks as subtitles. JW
Player playlists are unsupported errors. On generic pages its
`_extract_embed_urls` (`<script>`/`<iframe>` `src` player URLs, else
`<input value>`, else a `data-video-jw-id` div as `jwplatform:ID`) joins the
embed detection, first in extractor order. A pseudo-URL result's
`webpage_url_domain` is `null`, like upstream's `get_domain`.

Kick (`kick.com`) follows upstream's five reworked extractors (yt-dlp
`fe84bef`, release 2026.10.21), each requesting the site APIs (`kick.com/api/v2`,
`web.kick.com/api/v1`, a `session_token` cookie as bearer token) with
upstream's per-extraction impersonation warning: `kick:vod`
(`CHANNEL/videos/UUID`: the playback endpoint's HLS URL, its `type: details`
error responses, metadata from the creator's video list; a subscriber-only VOD
without a playback URL checks the subscription and needs cookies),
`kick:clip` (`CHANNEL/clips/clip_ID`, `CHANNEL?clip=clip_ID`; HLS or direct
clip URLs, the API's 404 message as the error), `kick:live` (a channel's live
HLS stream, or `The channel is not currently live`), `kick:videos`
(`CHANNEL/videos`, `?sort=views`: `url_transparent` VOD entries smuggling
their availability) and `kick:clips` (`CHANNEL/clips`, `?sort=view|date`,
`&range=day|week|month|all`: every cursor page fetched up front, unlike
upstream's lazy generator). Live downloads have this port's live HLS limits.
`--use-extractors` matches every extractor by its upstream name.
`tests/fixtures/sites/kick/` is compared with upstream output.

Kommunetv archives (`SUBDOMAIN.kommunetv.no/archive/ID`) follow upstream
`KommunetvIE`: the Oslo streams API (requested with `Accept:
application/json`), its `stream.title` and first playlist's first file
without query or fragment as non-fatal `hls` formats, with upstream's
`KeyError`, bare `IndexError` and `No video formats found!` errors.

Ku6 shows (`v.ku6.com/show/ID[...].html`) follow upstream `Ku6IE`: the page's
fatal greedy `<h1 title=...>` heading (often empty, so processing's generic
title) and the player JSON's `data.f` URL as an untyped result, with
upstream's `KeyError` messages.

Imgur media (`imgur.com/[SLUG-]ID`, `i.imgur.com/ID.gifv|mp4`) follow
upstream `ImgurIE`: the post API's first media (`is not a video or animated
image` otherwise) and account fields, the `.gifv` page (non-fatal) with its
`video-elements` `<source>`s, OpenGraph sizes, `videoItem` GIF format and
Twitter player stream, duplicate URLs removed, `No sources found` errors and
`Accept: */*` headers. Galleries (`/gallery/`, `/t/TOPIC/`, `/r/SUB/`,
`imgur:gallery`) and albums (`/a/`, `imgur:album`) follow
`ImgurGalleryBaseIE`: the album API (a 404 answer is accepted), playlists of
animated/video items as `url` results, and a gallery's single video (or a
non-album ID) as a `url_transparent` result with the album's title and
description. `tests/fixtures/sites/imgur/` is compared with pinned upstream
output.

Tumblr posts (`BLOG.tumblr.com/post|video/ID` and `www.tumblr.com/BLOG/ID`
shapes) follow upstream `TumblrIE`: the anonymous API token from the login
page, the post page and its permalink API JSON, Tumblr-hosted video from the
iframe's `<source>` and `data-crt-options` HD URL (with upstream's JSON parse
warning and the post-media fallback), external provider embeds as `url`
results, and the post's title, description, uploader and tags. Posts with
several videos form a playlist and dashboard-only posts report upstream's
errors, but neither is covered by fixtures yet; logins are not supported.
`tests/fixtures/sites/tumblr/` is compared with pinned upstream output.

Odysee/LBRY streams (`odysee.com`, `lbry.tv` and `lbry://` claim URLs,
`$/embed/` and `$/download/` paths) follow upstream `LBRYIE`: the API
proxy's `resolve` and `get`, the v3 API's original file (when the answer is
not an m3u8), the streaming URL's redirect to its HLS master, livestreams
through `api.odysee.live` (`This stream is not live` otherwise), the
claim's metadata and channel URL, and upstream's `lbry said: CODE -
MESSAGE` errors. Channels and playlists are not supported.
`tests/fixtures/sites/lbry/` is compared with pinned upstream output.

LinkedIn posts (`linkedin.com/posts/SLUG-ID-XXXX`,
`feed/update/urn:li:activity:ID`) follow upstream `LinkedInIE`: the page's
first `<video>` `data-sources` formats (MIME type, `data-bitrate`/1000 as
`tbr`) and `data-captions-url` (`en`, `vtt`), the OpenGraph or `<title>`
title, reaction count, JSON-LD `SocialMediaPosting` author, thumbnail (with
upstream's warning when missing) and description. LinkedIn Learning and
events (login required) are not supported. `tests/fixtures/sites/linkedin/`
is compared with pinned upstream output.

Livestreamfails clips (`livestreamfails.com/clip|post/ID`) follow upstream
`LivestreamfailsIE`: the clip API JSON as an untyped result (`sourceId` as
`display_id`, `unified_timestamp` of `createdAt`, the streamer label as
`creator` with `creators` derived, `format_field`'s empty thumbnail without an
image ID). A `None` `display_id` becomes the ID in place, like upstream.

HitRecord records (`hitrecord.org/records/ID`) follow upstream `HitRecordIE`:
the records API's `source_url.mp4_url` as an untyped result with its title,
cleaned body description, millisecond duration, timestamp, uploader, Python
`str()` of the user ID, counts and string tag texts, with upstream's
`KeyError` messages.

Huajiao lives (`huajiao.com/l/ID`) follow upstream `HuajiaoIE`: the page's
fatal `var feed` JSON, its `formated_title` (upstream's `KeyError`), the
description meta (with its warning), `parse_duration`, image, space-delimited
ISO 8601 `creatime` timestamp, author nickname and raw `uid`, and the feed's
`m3u8` as fatal HLS formats (subtitles ignored with upstream's warning).
Output templates now read represented fields holding another type (such as an
integer `uploader_id`) from the info dict.

Hypem tracks (`hypem.com/track/ID`) follow upstream `HypemIE`: the page's
`displayList-data` script (cleaned like `_html_search_regex` before parsing),
its first track's `serve/source` JSON (`Downloading metadata`) URL as an untyped
`mp3` result with `song` as title and track, `artist`, `int_or_none`
`time`/`ts`, and upstream's missing-script, `KeyError` and bare `IndexError`
messages.

Loom videos (`loom.com/share|embed/ID`) follow upstream `LoomIE`: the
GraphQL `GetVideoSSR` metadata (password-protected and wrong-password
errors; `--video-password` is sent), `GetVideoSource` and `FetchChapters`
(description-style chapters), the campaign session `raw-url` and
`transcoded-url` answers and the CDN URL (when it is not a duplicate) as
pre-merged HLS (split video marked silent), DASH or progressive formats
with upstream's qualities and their query as `extra_param_to_segment_url`,
and the transcript subtitles with `--write-subs`/`--list-subs`. Folders are
not supported. `tests/fixtures/sites/loom/` is compared with pinned upstream
output.

Maariv players (`player.maariv.co.il/public/player.html?...media=ID`) follow
upstream `MaarivIE` (`maariv.co.il`): the Walla media API's URL-valued
`video.url` as non-fatal `hls` formats (subtitles ignored with upstream's
warning) and its stream URLs as `http` formats with `parse_resolution`'s
size, the title (else the generic title, after `fulltitle`), integer
duration and `unified_timestamp` upload date; a result without formats fails
in processing. Its iframe embeds on generic pages follow JW Platform's in the
embed order.

Masters videos (`masters.com/en_US/watch/YYYY-MM-DD/ID`) follow upstream
`MastersIE`: the related-content API's fatal HLS manifest (with its subtitle
renditions; no manifest is upstream's `Failed to obtain m3u8 URL`), the API
title (else the generic title), the URL date as `upload_date` and the first
image set's named images as thumbnails.

Mixcloud shows (`mixcloud.com/USER/SLUG`) follow upstream `MixcloudIE`: the
GraphQL cloudcast lookup (with upstream's impersonation warning), its
XOR-encrypted progressive (5 MiB chunks), HLS and DASH stream URLs, the
restriction and exclusive-content errors, comments and metadata. User
uploads, playlists and favorites are not supported.
`tests/fixtures/sites/mixcloud/` is compared with pinned upstream output.

Moving Image Archive films (`movingimage.nls.uk/film/ID`) follow upstream
`MovingImageIE`: the player's fatal `file` manifest as HLS formats (subtitles
ignored with upstream's warning), the fatal `Title` field without
surrounding brackets, the `Description` and `Running time` fields (each
warning as `title`, like upstream) and the player image.

Niconico videos (`[www.|sp.|embed.]nicovideo.jp/watch|shorts/ID`) follow
upstream `NiconicoIE`: the watch API (`v3_guest`, or `v3` with a
`user_session` cookie; 400/404 answers read) with upstream's error map,
geo-restriction, sensitive/scheduled/login-required messages and
availability, the domand `access-rights/hls` request (every available
video/audio pair) and its HLS formats (audio renditions with the API's
bitrate, sampling rate, ID and quality; variants sorted by rate, deduplicated
by URL, named after their playlists, with the lowest audio bitrate
subtracted), thumbnails with lenient resolutions, and the owner/channel,
genre, tag and count fields. The comment "subtitles" (inline JSON data)
are an unsupported error with `--write-subs`/`--list-subs`; logins,
playlists, series, search, live and user pages are not supported.
`tests/fixtures/sites/niconico/` is compared with pinned upstream output.

Nobel Prize media (`[www.|mediaplayer.]nobelprize.org/mediaplayer/?id|qid=ID`)
follow upstream `NobelPrizeIE`: the integer `id` (else `qid`) query value (no
ID is upstream's `Unsupported URL`), the `mediaplayer.nobelprize.org` page's
fatal JSON-LD as an untyped result overlaid with the `caption` meta title, the
`itemprop="description"` span and the `duration` meta.

OK.ru (Odnoklassniki) videos (`ok.ru`/`odnoklassniki.ru` `video`,
`videoembed`, `live`, `web-api/video/moviePlayer` and `dk?...st.mvId=`
shapes, `fromTime` as `start_time`) follow upstream `OdnoklassnikiIE`: the
page's stub error text and restricted-access login error, the player
`data-options` (external players as `url` results), flashvars metadata or the
POSTed metadata URL, progressive, HLS, DASH, live HLS and RTMP formats with
upstream's `type` qualities (CDN cookies cleared after each manifest),
subtitles, page meta upload date and age limit, and YouTube-hosted videos as
`url_transparent` results. Inline embedded DASH manifests are an unsupported
error; upstream's boosty branch calls a missing method and fails the same way.
`tests/fixtures/sites/odnoklassniki/` is compared with pinned upstream output.

OnePlace episodes (`oneplace.com/.../listen/SLUG-ID`) follow upstream
`OnePlacePodcastIE`: the fatal `mp3-url`/player `data-media-url`, the details
heading or `og:title`/`title` meta title and the `epDesc` description as an
untyped `mp3` audio result.

ParlView videos (`aph.gov.au/News_and_Events/Watch_Read_Listen/ParlView/video/ID`)
follow upstream `ParlviewIE`: the API's `videoDetails.files.file.url` as fatal
HLS formats and subtitles, the first string `parlViewTitle`/`title` (else the
generic title, after `fulltitle`), description, `H:M:S:F` duration, ISO 8601
`recordingFrom` timestamp and URL-valued thumbnail, with upstream's `KeyError`
messages.

Photobucket media (`photobucket.com/..._ID.mp4|flv`) follow upstream
`PhotobucketIE`: the page's `Pb.Data.Shared.MEDIA` JSON (`Extracting
information`) with the embed code's unquoted `file=` URL, username, creation
date, title and thumbnail as an untyped result with the URL's extension.

Pinterest pins (`pinterest.TLD/pin/[SLUG--]ID`) follow upstream
`PinterestIE`: the Pin resource API, the pin's video list (HLS with the
list key as `m3u8_id`, or progressive formats with their duration) or a
story pin's video, image thumbnails and metadata; a pin embedding another
site's video is a `url_transparent` result of the embed. Boards and
collections are not supported. `tests/fixtures/sites/pinterest/` is
compared with pinned upstream output.

Player.fm episodes (`player.fm/[series/]SERIES/SLUG`) follow upstream
`PlayerFmIE`: the episode's `.json` document as an untyped audio result with
the integer ID (else upstream's `Missing "id" field`), `clean_podcast_url`
URL (else `No video formats found!`), title (else the generic title, after
`fulltitle`), cleaned description, integer duration/size/timestamp, the first
URL-valued episode or series image and the series author as `creators`.

PopcornTV videos (`SUB.popcorntv.it/guarda/SLUG/ID`) follow upstream
`PopcornTVIE`: the fatal `contentUrl`/`embedUrl` link's `href` as `hls`
formats (subtitles ignored with upstream's warning), the `name` heading
(else the OpenGraph title), description article, OpenGraph thumbnail and
`uploadDate`/`duration` (minutes)/`interactionCount` metas, each with
upstream's warning, and upstream's `KeyError` messages.

Reddit posts (`SUB.reddit.com/[r/SUB/|user/NAME/]comments/ID`, also
`redditmedia.com`) follow upstream `RedditIE`. Without a `reddit_session`
cookie it first sets up an anonymous session from old Reddit (else the
shreddit fallback when no `loid` cookie was set), sets the `over18` and
`_options` (`pref_gated_sr_optin`) cookies, then reads the post's `.json`
listing (a 403 answer's `quarantined`/`private` reasons are upstream's
login-required errors). The info dict has the preview thumbnails (with
`Accept: */*` headers), age limit, 72-character title with the full
`alt_title`, description, float timestamp, uploader, subreddit as
`channel_id` and vote/comment counts. A Reddit-hosted video (the post's or a
crosspost's) gets the `fallback` MP4 plus non-fatal HLS (`f=…,subsAll`) and
`dash-` DASH formats with their subtitles; only with `--write-subs` or
`--list-subs` and no manifest subtitles is `wh_ben_en.vtt` checked, as
upstream's lazy `extract_subtitles`. A text post's embedded `RedditVideo`
media form a playlist; other links are `url_transparent` results followed
through the extractor registry with the post's fields. Format sorting now
sees `filesize_approx` (computed before sorting, as upstream), and playlist
entries get `playlist_uploader`/`playlist_channel` fields.
`tests/fixtures/sites/reddit/` is compared with pinned upstream output.

Premiership Rugby videos (`SUB.premiershiprugby.com/watch/SLUG`) follow
upstream `PremiershipRugbyIE`: the article CMS's hero media `videoLink` as fatal
HLS formats and subtitles without an `ext` (a media playlist keeps its URL's
extension), the `sourceSystemId` ID with the slug as `display_id`, hero title
(else the generic title), thumbnail, `int_or_none(msDuration, scale=1000)`
duration, raw tags and category texts, with upstream's `KeyError` messages.

QingTing programs (`qingting.fm`/`qtfm.cn` `[v]channels/C/programs/P`) follow
upstream `QingTingIE`: the mobile program page's `window.__initStores` JSON
(`_search_json`) with the program title and audio URL, channel title and
podcaster as an untyped `m4a` audio result; a missing audio URL is upstream's
`No video formats found!`.

ReverbNation songs (`reverbnation.com/.../song/ID`) follow upstream
`ReverbNationIE`: the `api.reverbnation.com/song/ID` JSON (`Downloading
information of song ID`) as an untyped `mp3` audio-only result with the artist
as `uploader`/`uploader_id`, and `thumbnail`/`image` thumbnails ranked like
`qualities`; a missing `name`/`url` is upstream's `KeyError` extractor error.

On generic pages, the embeds of Blogger, JW Platform, Megaphone, Streamable and YouTube
(`YoutubeIE._EMBED_REGEX`: iframe/embed/object/SWFObject/`data-video-url`
`youtube[-nocookie].com/embed|v|p/ID` URLs and lazy-load watch links, kept when
this port's YouTube extractor accepts them), in
upstream extractor order (each HTML-unescaped, joined to the page URL,
suitable and unique, with `Identified N NAME embed(s)` debug lines), are `url`
results like upstream `_extract_embeds`: one is followed (`original_url` stays
the page), several form the page's playlist (`--flat-playlist` keeps
`ie_key`/`extractor` entries with the `_match_id` temporary ID). A page with
both HTML5 media and these embeds is an unsupported error; other sites' embeds
are not detected.

`_fill_common_fields` derives the deprecated single-value fields
(`album_artist`, `artist`, `composer`, `creator`, `genre`) and their lists
from each other (`', '` split, or joined with commas as fullwidth commas); the
deprecation warning for extractors returning both is not represented, since no
supported extractor does.

YouTube acquisition follows upstream `_initial_extract` and
`_extract_player_responses` for an anonymous session. The watch page is
requested as `watch?v=ID&bpctr=9999999999&has_verified=1`, and its first
`ytcfg.set({...});` object is the page client's `ytcfg`. The page's
`ytInitialData` (kept when it has `contents`) and `ytInitialPlayerResponse` are
read too; the latter is kept, without streaming data, for metadata. Initial
data without `contents` is re-fetched from the `next` API. The
requested clients follow `_get_requested_clients`:
- **JS runtime available:** `tv_samsung`, `visionos`, `web`.
- **No runtime:** `visionos`, with upstream's deprecation warning.
- **`--extractor-args "youtube:player_client=..."`:** `default`, `all`,
  `-NAME` exclusions and names from upstream's `INNERTUBE_CLIENTS` table, which
  `src/extractor/youtube/innertube.rs` reproduces from the pinned checkout
  (`No player clients have been requested` when all are excluded).

The `web` client reuses the page's player response. Every other client makes
one `Downloading CLIENT player API JSON` POST to `/youtubei/v1/player`. The
request carries upstream's context (`hl` from `lang`, else `en`; `timeZone=UTC`), the page's `STS`
as `signatureTimestamp`, the content and racy check flags, and the
`X-YouTube-Client-Name`/`-Version`, `Origin`, `X-Goog-Visitor-Id` and
`User-Agent` headers. Without a player URL, a JS-requiring client takes
upstream's iframe API fallback. Responses for another video are skipped with
upstream's warning.

Formats follow `_extract_formats_and_subtitles`. First every `n` and
`signatureCipher` challenge across the responses is collected. Then each
response's HTTPS formats are built with `process_format_stream`: the format ID,
note, quality rank, source and language preferences, size, codecs, `EXT_dash`
container, `available_at`, and the 10 MiB `http_chunk_size`. Upstream's filters
apply:
- **Duplicates:** stream IDs already seen are skipped.
- **Live and OTF:** live adaptive formats (`targetDurationSec`) are skipped
  unless the video is post-live or `--live-from-start` is given (or
  `formats=incomplete`); OTF streams are skipped.
- **PO tokens:** formats whose client policy requires a GVS PO token are skipped
  with upstream's message unless `po_token` gives one (see
  [YouTube extractor arguments](#youtube-extractor-arguments)).
- **SABR:** formats missing a URL get upstream's SABR warning.
- **DRM:** upstream's DRM and damaged-format warnings apply.

After that come the response's HLS and DASH manifests, with upstream's `n`
path replacement, itag-based IDs, quality and preference adjustments.
Challenges are solved once through the configured runtime: the player JS
(`Downloading player ID-main`, `[jsc:deno] Solving JS challenges using deno`),
the pinned EJS batch, and signature permutations of `chr(0..LENGTH)` like
upstream's `sigfuncs`. Unsolved challenges get upstream's warnings, and their
formats are skipped. Like upstream, deno is enabled by default (`--js-runtimes`
adds Node.js, `--no-js-runtimes` clears them). The executable must be Deno
2.3+ or Node.js 22+ (upstream's minimums; CI exercises Deno 2.9.7).

Metadata still comes from the first player response with video details, as
described above. HLS formats come from the master playlist alone, as upstream
reads it (the chosen format's media playlist is fetched when downloading:
`[hlsnative] Downloading m3u8 manifest`); `youtube-hls` covers this. A live
video's formats are parsed like upstream's `live=True`, its title gets the local `YYYY-MM-DD HH:MM` suffix, and it downloads
through FFmpeg like upstream's FFmpegFD; on Unix an interrupt (SIGINT) asks
FFmpeg to quit, prints `[ffmpeg] Interrupted by user` and completes the
download (publication and post-processing) as upstream does for live
streams. An upcoming video reports its playability reason (`This live event
will begin in …`); with `--wait-for-video MIN[-MAX]` that reason is a
warning, and like `_wait_for_video` the extractor waits until the release time
(bounded by MIN/MAX; `Release time of video is not known` or `Video should
already be available according to extracted info` otherwise), showing `[wait]
Remaining time until next attempt: HH:MM:SS` (one line with `--no-progress`),
then extracts again (`[wait] Wait period ended; Re-extracting data`; on Unix
Ctrl+C retries at once). A live video's remaining HTTPS formats get upstream's
`(incomplete)` note and -20 preference.

A post-live video (`isPostLiveDvr`, `live_status` `post_live`, `was_live`)
follows `_needs_live_processing`: its HLS manifest (and its DASH manifest
beyond two hours) is skipped, other formats get the `(Last 2 hours)` note and
-10 preference, and `-v` shows `Video is in Post-Live Manifestless mode`. Each
live adaptive format keeps `target_duration`, `_itag` and `_client` and
becomes an `http_dash_segments` format (`dash` in `-F`) whose fragments, like
upstream's lazy list, are generated only when JSON output or a download reads
them: one HEAD request's `X-Head-Seqnum` (shared by the formats within five
seconds) gives `&sq=0` up to that number minus two, each with
`fragment_count`. A stream that started over 120 hours ago gets upstream's
`Starting download from the last 120 hours` warning and only the fragments of
those hours. A failed HEAD request leaves no fragments. The `youtube-post-live`
fixture compares this with pinned upstream.

With `--live-from-start`, a live video's adaptive formats follow the same
path as `is_from_start` formats with `is_live` and the
`http_dash_segments_generator` protocol (`dashG` in `-F`); the HLS manifest is
skipped and other formats get `(Last 2 hours)`. They download through a port
of `DashSegmentsFD`: `[dashsegments] Total fragments: unknown (live)`, no
`.ytdl` file, and fragments from `_live_adaptive_fragments`' generator, which
polls `X-Head-Seqnum` (one HEAD per five-second window, shared by the
formats), yields `&sq=` fragments up to it, refetches the webpage and player
responses (`_initial_extract`) for refreshed URLs once its no-fragment score
passes 15 (or after five hours), reports `Video is no longer live` /
`Cannot find refreshed url` through three retries, honors the 120-hour limit
and stops when the stream ends or the score passes 30. A merge's formats
(`FILE.fFORMAT.EXT`, named with the merged extension like upstream) download
in parallel and are then merged. On Unix an interrupt ends the fragment lists,
drops the fragment in flight and completes the download (publication, merge
and post-processing). Progress is one combined line rather than upstream's
per-format multiline status, and `-j` omits the generator (upstream prints its
`repr`). `youtube-live-from-start` compares extraction with pinned upstream;
`tests/youtube_live.rs` covers downloads, merging and interrupts against a
local server with a changing `X-Head-Seqnum`. The refetch does not reuse
upstream's `--extractor-retries` value (always 3).
Like upstream's anonymous `_extract_player_responses`, an age-gated response
adds the `web_embedded` client (and a "made for kids" video the
android_vr/visionos clients cannot play adds it given a JS runtime), with its
embed-page `ytcfg` (`_download_ytcfg`), `encryptedHostFlags` and the player
JS's signature timestamp when the `ytcfg` has no `STS`; responses for another
video are skipped, responses without details are kept. Without formats, the
playability status's reason is reported like upstream (DRM, sign-in hint,
captcha, rate limit); geo restrictions raise upstream's `GeoRestrictedError` text
with `This video is available in COUNTRIES.` (the microformat's
`availableCountries`, else the page's `regionsAllowed`, as `ISO3166Utils`
names; a warning with `--wait-for-video`).
Missing player-response metadata falls back like upstream: the title to the
microformat's, the page's meta tags, then the initial data's description
header or player overlay (`No title found in player responses; ...`); the
duration to the microformat's `lengthSeconds`, the `duration` meta, then the
live broadcast's end minus start; a missing saved thumbnail to the best
generated one. `yt:stretch=W:H` keywords set `stretched_ratio` on video
formats; m3u8 formats of videos uploaded within the last two days
(`today-2days`) get `__needs_testing` (`Untested`, tested when selected). A
watch page whose `ytInitialData` lacks `contents` warns and re-fetches it from
the `next` API (`Downloading initial data API JSON`, incomplete responses
retried three times; skipped with `player_skip=initial_data`). A paid video's
`trailerVideoId` is followed as `url_result(ID)` (`Extracting URL: ID`). A
multicamera `metadataList` makes a playlist of `url_transparent` feed
entries (`Downloading multifeed video (IDS) - add --no-playlist ...`, URLs
smuggling `force_singlefeed`, titles `TITLE (FEED)`); `--no-playlist`
downloads just the video. Premium sessions log `Detected YouTube Premium
subscription`. `youtube-fallbacks` and `youtube-multifeed` compare these with
pinned upstream.
Before extraction the `PREF` (`hl=en`, `tz=UTC`) and `SOCS` consent cookies are
set like `_initialize_pref`/`_initialize_consent`. With YouTube account
cookies (`--cookies`/`--cookies-from-browser` holding `LOGIN_INFO` and a
`SAPISID` cookie), the session is authenticated like upstream's: no consent
cookie, the authenticated (or, with the Premium logo, premium) default clients,
clients without cookie support skipped with upstream's warning, `web_music`
added for music URLs, `web_creator` tried for age verification, and InnerTube
requests carry `generate_api_headers`' session headers (`Authorization`
SAPISIDHASH values, `X-Origin`, `X-Goog-AuthUser`/`X-Goog-PageId` from the
session index and data-sync ID, `X-Youtube-Bootstrap-Logged-In`). The
`youtube-auth` fixture (with a `copy`-ed cookies file) compares these against
pinned upstream. A player response's pre-roll ads (`adPlacements` starting the
video and `adSlots` before content, by length or skip offset) set its formats'
`available_at` like `_get_available_at_timestamp` (`Detected a 15s ad skippable
after 5s for CLIENT` with `-v`), and the download of each stream first sleeps
until then (`Sleeping N seconds as required by the site...`), raising
`--sleep-interval` like upstream's forced sleep. InnerTube requests are retried
three times on network errors and HTTP errors other than 403/429 (`HTTP Error
500: … Retrying (1/3)...`), like `_extract_response`. The player and initial
data requests also report a failed response's JSON error message (`YouTube
said: ERROR - …`) and alerts (`YouTube said: TYPE - …`, `unknown error`s
retried), and retry an incomplete (empty, or initial data without `contents`)
response three times before giving up with a warning (`raise_incomplete_data`
raises instead).
A video left without formats or a playability reason fails with upstream's `No
video formats found!`. Cookie rotation warnings and PO token provider plugins are
not implemented.

`tests/fixtures/sites/youtube` (jsless `visionos`), `youtube-playability`
(age gate, unavailable and private videos), `youtube-consent` and `youtube-js` compare
these paths with pinned upstream output through the site-fixture proxy. The
`youtube-js` fixture uses the saved real player `74edf1a3` and its known
challenge vectors. Player API routes are matched by their
`X-YouTube-Client-Name` header. The capture tool gives upstream a temporary
cache seeded with the vendored EJS 0.8.0 release, and the volatile
`available_at` is normalized to 0. Cases that solve challenges are skipped when
`deno` is not on `PATH`.

## HTML5 extraction

The generic fallback dispatches a single response to DASH/HLS parsing,
direct-media inspection or HTML5 parsing. Site extractors registered through `Extractor` run first. The
standalone `DirectHttp` extractor remains available for direct-only callers.

For `text/html`, accept `video`/`audio` elements with nonempty
`src` and/or direct child `source[src]` declarations. Resolve URLs against the
final page URL and first `base[href]`; decode HTML entities using an HTML5 parser. Prefer Open Graph
title, then document title, then page filename stem; collapse title whitespace.
Skip elements without nonempty `src` or direct child `source[src]` declarations. One retained element is a
video; two or more form an HTML5 playlist in document order. Append `-1`, `-2`, …
to entry IDs and ` (1)`, ` (2)`, … to titles, as upstream HTML5MediaEmbedIE does.
Number retained entries contiguously, ignoring empty elements. All-empty pages
fail as unsupported. The playlist wrapper has the unsuffixed page ID/title,
`generic` / `Generic` extractor identity, and the final page `webpage_url`.
Without a page title the wrapper title is `video`, while entry titles use the
page filename stem. Subtitle-only elements remain outside the supported subset.
Metadata identifies `html5` / `HTML5MediaEmbed` with `direct: false` and the final
page URL as `webpage_url`. Every source is probed with the page referrer and must
serve supported direct HTTP media or a supported finite HLS/static DASH manifest. Redirected source URLs and response-derived
extensions/filesizes are recorded in typed `VideoFormat` entries. HTML5 JSON adds
`formats` and the selected `format_id`, including for a single source. IDs are
literal zero-based strings assigned to nonempty declarations: the media element's
`src` first, then direct child sources in document order. Duplicate URLs remain
separate declarations. Source IDs restart at `0` for each element. These IDs are
Rust-specific; upstream's label/title IDs,
and full upstream format sorting are not implemented.

Default selection and `-f best` choose the highest resolution source; `-f worst`
chooses the lowest. Child `<source>` attributes `width`, `height`, and fallback
`res` (for height) supply optional positive `u32` dimensions, with surrounding
whitespace allowed. Zero, signed, overflowing, unit-suffixed, and other malformed
values are ignored; an invalid height still allows `res` fallback. Media element
width/height describe the player and are not inherited, including by inline `src`.
Label/title resolution parsing is not implemented.

Resolution rank is the smaller known dimension, or the only known dimension;
unknown resolution ranks below known resolution. Portrait sources are therefore
ranked by their shorter side. On equal ranks best/default selects the last
declaration and worst the first. File size, extension, codecs, bitrate, frame
rate, and audio/video distinctions do not break ties. This is an explicit native
policy for the supported progressive subset, not full upstream best/worst parity
or a guarantee of perceptual quality. Format lists and source IDs retain declaration
order. Literal IDs override ranking; unavailable IDs return an error listing IDs.
Like upstream, `best`/`worst` need a complete (muxed or unknown-codec) format
unless every format is video-only or every one audio-only; `bv`/`ba` take
separate streams, `bv*`/`ba*` any format with video/audio and `b*`/`w*` any
format; extension selectors (`mp4`, `webm`, `m4a`, `mp3`, …) pick that
extension (video ones need a complete stream, falling back to video streams
when the result has no complete format); and a `+` merge drops a second format
that repeats the first's video or audio, leaving a single format. Comma
selections and more than two merged streams remain errors. See the bounded adaptive policy below.
Direct HTTP results carry upstream's single generic format: its ID is the
Content-Type subtype (`mp4`, `mpeg`, `webm`, …), or `0` when only the URL suffix
identified the media, and `audio/*` media is audio-only (`vcodec: none`), so
`bestaudio`, `ba/b` and the literal ID select it.
The library's `extract_info_with_format` performs the same selection; standalone
extractor callers must call `VideoInfo::select_format` to resolve candidates before
download. Selection updates top-level `url`, `ext`, `filesize`, `width`, `height`,
and `format_id` together. Optional dimension fields deserialize as absent for older
metadata. Each playlist entry is ranked independently.
The selected source is downloaded with the page referrer. If the download
response changes to HTML or a manifest, fail without publishing it as media.

HTML bodies are limited to 2 MiB, including responses without Content-Length.
Only uncompressed UTF-8/ASCII HTML is supported. Other encodings are rejected
when declared in HTTP Content-Type; invalid UTF-8 is rejected. HTML meta charset
sniffing, compressed responses, XHTML, JavaScript, nonstandard lazy-loading
attributes, durations, codecs, inferred dimensions, authentication,
and general embedded-player extraction are not implemented. Unsupported HLS/DASH
features and non-HTTP sources fail explicitly, even
when another supported source was explicitly selected. Unselected source probe
failures also fail extraction rather than hiding unsupported alternatives.
Every entry and alternative is validated before a result is returned, and a
failed probe fails the whole input before downloads or JSON output. The CLI
selects formats per entry like upstream `process_video_result`: a failed
selection reports `ERROR: [EXTRACTOR] ID: Requested format is not available. Use
--list-formats for a list of available formats` (after that entry's `-F` table)
and later entries continue; `-J` shows the entry as `null` (a single video prints
`null`). The library's `extract_result_with_format` still selects atomically and
names the entry in its error. A failed transfer or output-path collision
sets a nonzero CLI exit status while later entries and input URLs continue.
Fixed output paths may therefore collide; the default template includes each
entry's distinct ID. Existing files are never overwritten.

The library exposes `ExtractResult::Video(VideoInfo)` and
`ExtractResult::Playlist(PlaylistInfo)` through `extract_result` and
`extract_result_with_format`. Serde encodes the result enum using `_type: video`
or `_type: playlist`; playlist entries are ordinary `VideoInfo` objects.
`extract_info` / `extract_info_with_format` still return single videos. They
reject playlists rather than silently choosing the first entry; extraction and
format-selection errors can precede that boundary error. Existing custom
`Extractor::extract` implementations use the default `extract_result` adapter;
playlist-capable extractors override it. Standalone extractor callers must resolve
format selection themselves. The CLI flattens results into per-video NDJSON,
simulation diagnostics, or sequential downloads; it exposes no playlist wrapper
JSON option, playlist selection, nested playlists, or unresolved URL results.

MIME checks identify supported responses; they do not verify media containers.

## Download retries

The library default download policy permits three additional attempts after any
HTTP 5xx status (upstream HttpFD; other statuses fail at once), connection/timeouts
and other transport errors, response body read errors, or Content-Length
mismatches. Extraction requests keep retrying HTTP 408, 429, 500, 502, 503 and 504. `--retries 0` disables retries; values through 255
are accepted (the CLI default is 10, see `--retries`; `u32::MAX` retries
indefinitely). The library exposes `DownloadOptions { retries, fragment_retries,
retry_sleep, skip_unavailable_fragments }` through
`YoutubeDl::download_with_options`; `download` uses the default policy.
After the retry budget is exhausted, return `Error::RetriesExhausted`, which
wraps the final attempt's error with upstream's Giving-up text.

After a body interruption, a retry can reuse bytes already written in the same
private temporary file. This requires a known Content-Length and a valid strong
ETag from the **download** response, an identity-encoded body, and a nonzero
stored offset below the total. The retry sends `Range: bytes=N-` and `If-Range`
with that ETag. A 206 response is accepted only with the same final URL and ETag,
identity encoding, and a single valid Content-Range covering exactly `N` through
`total - 1` with the original total. Content-Length, when present, must match the
remaining length; the received body is also checked against that length. Further
interruptions advance the offset and consume the same retry budget. Transient
HTTP errors preserve the offset without appending their bodies.

A full response to a range request replaces the temporary contents from byte
zero, including when the representation changes. Missing/weak/malformed ETags,
unknown initial lengths, and encoded responses use whole-file retries instead.
Last-Modified alone is not used as a validator. Invalid/changed partial responses
and HTTP 416 fail explicitly, without publishing a file or guessing completion.
Temporary files are removed on final failure. Resume across invocations, saved
partial files, multipart ranges, and fixed-size range chunk downloading are not
implemented.
The offset/length checks also apply when a 206 body omits Content-Length.

Backoff is 100, 200, 400, 800, then 1000 milliseconds between attempts, capped
at one second thereafter. For the selected transient HTTP statuses, a single
`Retry-After` delay-seconds or preferred HTTP-date value extends this backoff,
capped at 60 seconds.
ASCII digits with optional surrounding spaces/tabs are accepted; oversized
integer values saturate to the cap. HTTP dates must use the preferred
IMF-fixdate form, such as `Wed, 21 Oct 2015 07:28:00 GMT`. The delay is measured
against the local wall clock when the response is checked, independently of
the server's `Date` header; past dates give a zero hint. Clock skew can therefore
affect the delay, which remains capped at 60 seconds. Missing, malformed,
duplicated, comma-list, and obsolete date values use ordinary backoff. A zero
hint cannot shorten backoff.
Headers on successful or permanent-error responses do not introduce delays;
exhausted/disabled retry budgets never sleep or gain extra attempts. The same
rules apply to HTML pages, source probes, and full/range download requests.
These are native subset policies, not upstream retry-default or HTTP-status parity.
Library failures with a supported hint use `Error::HttpRetryAfter { source,
retry_after }`, retaining the reqwest status error and capped duration; other
HTTP failures retain `Error::Http`. Neither error variant retries automatically
outside the orchestration APIs.

Every attempt repeats response validation and preserves the HTML5 page referrer.
Other HTTP statuses (including 401, 404, and 501), unsolicited 206 responses,
unsupported HTML5 media responses, output collisions, and filesystem failures
fail immediately. Network reads and filesystem writes are handled separately so
a local write failure cannot trigger another network transfer. The final file
is published only after the complete representation has been received and is
never overwritten.
Download retries do not affect extraction, page reads, or source probes.
The per-request timeouts still apply to each attempt; there is no overall
download deadline. Cooperative download cancellation is described below.

This slice is based on the same pinned revision
**897341241f2bfb2439fe0367f4c7c9ccd29a3828**, specifically
`yt_dlp/downloader/http.py`'s `RetryDownload`/`RetryManager` control flow,
transport-read failures, and Content-Length validation. Its broader 5xx retry
handling remains outside this slice.
The range slice additionally references `establish_connection`, `retry`, and
`download` in that file: range requests, Content-Range checks, retained byte
counts after interrupted reads, and short-response retries. Strong ETag/If-Range
and final-URL checks are stricter native policies; upstream does not implement
these guards. Upstream's heuristic HTTP 416 completion and persistent partial
files are deliberately excluded. The baseline's
`test/test_downloader_http.py` was reviewed for its localhost HTTP fixture,
known/unknown length, and chunk/range cases; `tests/retries.rs` adds independent
deterministic failure/recovery coverage for validated 206 responses, repeated
interruptions, range rejection, changed representations, unknown lengths, and
validator boundaries. This ports a transport slice from the
existing baseline and does not advance it.

## Extraction retries

`--extractor-retries N` opts into up to N additional whole-extraction attempts
(default 0, maximum 255). The library exposes the same policy through
`extract_result_with_options(url, format, ExtractionOptions { retries: N })`.
Existing extraction methods retain zero retries. Call `into_video()` on the
result when a single video is required; playlists still fail at that boundary.

A failed generic page request reports upstream's `ERROR: [generic] ID: Unable to
download webpage: ERR (caused by REPR)` (`HTTP Error CODE: Reason` with
`<HTTPError CODE: Reason>`, or urllib's `[Errno N] TEXT` with
`TransportError('…')`); library callers receive `Error::Webpage` wrapping the
HTTP error. Upstream never retries this request, as with the default of zero.

Retry HTTP 408, 429, 500, 502, 503, and 504, connection/timeouts, and HTML
response body read failures. Use the same bounded backoff and delay-seconds/HTTP-date
Retry-After policy as download retries. A new attempt runs the extractor registry from the start, re-fetching
pages and repeating earlier source probes, with the page referrer preserved.
This is a per-input budget, shared by all entries and sources; there is no
per-request budget and no reuse of partial HTML or extracted entries. Custom
extractors opting into this API must tolerate repeated calls. They can report
network body read failures with `Error::ExtractionRead`; ordinary `Error::Io`
remains a permanent failure so local filesystem errors cannot cause retries.

Format selection, unsupported content/schemes, invalid URLs, HTML size and encoding
limits, unsolicited 206 responses, and permanent HTTP statuses fail without
another attempt. After exhaustion return the final error. No partial playlist
metadata or downloads are emitted. The CLI continues to later input URLs and
returns a failing exit status if any input fails. The policy also applies under
`--simulate` and `--dump-json`; `--retries` controls downloads independently.
Standalone `Extractor` calls do not retry automatically.

This is a native transport policy extension of the existing supported subset,
not a port of upstream's extractor-specific retry semantics or defaults. It does
not advance the pinned baseline or claim general `--extractor-retries` parity.
`tests/retries.rs` covers status recovery/exhaustion, interrupted HTML, full
playlist restart after source-probe failures, referrers, CLI modes and budget
isolation, permanent error boundaries, and custom extractor filesystem errors.
The Retry-After extension is a native policy on this existing retry slice;
it does not advance the upstream baseline. Pure unit tests cover parsing and
delay bounds without sleeping; local HTTP tests cover recovery delays, exhausted
budgets, permanent failures, unsupported hints, and preserved range state.
The HTTP-date extension follows the preferred date form in
[RFC 9110 sections 5.6.7 and 10.2.3](https://www.rfc-editor.org/rfc/rfc9110.html#name-retry-after);
obsolete formats are explicitly outside this subset. Date parsing uses
`httpdate`, with a canonical-format check to reject obsolete or noncanonical
forms. Fixed-clock unit tests cover future/past dates, fractional delays, caps,
invalid dates, duplicate hints, and independence from the response Date header.
Local HTTP regressions cover expired-hint recovery and capped future hints on
exhausted budgets and permanent statuses for extraction and downloads.
Persistent resume, obsolete HTTP-date Retry-After formats, extraction cancellation,
and an overall extraction deadline remain unimplemented.

## Download progress and cooperative cancellation

`YoutubeDl::download_with_progress(info, path, options, callback)` extends the
existing download policy with a synchronous `FnMut(DownloadProgress) ->
ControlFlow<()>` observer. Existing `download` and `download_with_options` methods
keep their signatures and use a continuing observer. `DownloadProgress` contains
`attempt` (one-based, up to 256), `downloaded_bytes`, `total_bytes`, and `phase`.

Checkpoints run before each request (`Requesting`), after validated response
headers and each successfully written chunk (`Downloading`), during retry waits
(`WaitingToRetry`), and after flush/sync immediately before no-clobber publication
(`Publishing`). Retry waits sleep at most 100 ms between callbacks; scheduling
and callback runtime can add latency. Waiting events describe the failed attempt,
with the next `Requesting` event advancing the attempt number. Permanent or
exhausted failures do not emit waiting or publication events. Preflight output
collisions and filesystem errors may return before any callback.

Byte counts describe the current temporary file, not cumulative network traffic.
Totals come from download response headers, never extraction metadata; missing
lengths remain `None`. During `Requesting` and `WaitingToRetry`, the last known
count/total is retained (initially zero/`None`). A validated range response retains
the offset and original total. A full replacement response resets the byte count
to zero and updates the total before the next `Downloading` event. A transfer
with unknown length can still succeed; its final total remains `None`.

Return `ControlFlow::Break(())` at any checkpoint to get
`Error::DownloadCancelled`. Cancellation never retries, never publishes, and
removes the private temporary file, including when cancelling after the last
chunk or at `Publishing`. Created parent directories may remain. A callback can
inspect a caller-owned cancellation flag; there is no separate token type.
Callbacks execute on the caller's thread. Cancellation cannot interrupt blocking
requests, reads, flushes, or syncs; request timeouts still apply. Extraction has
no cancellation observer. `Publishing` does not guarantee success: only the
method's `Ok(bytes)` return confirms successful publication. No callback runs
after publication, and existing destinations remain protected even if created
by a callback immediately before publication.

The CLI renders upstream `FileDownloader.report_progress` lines (ported
`format_bytes`/`format_seconds`/`format_eta`/`format_speed`/`format_percent` and
templates, checked against actual upstream output in
`tests/fixtures/progress/cases.json`) on stdout through a port of
`minicurses`' single-line printer: each status starts with `\r` and is padded
over the previous one, the `100% of SIZE in TIME at SPEED` finished line ends
with a newline, `--newline` prints one line per status, and quiet output with
`--progress` erases the line instead. Whole-file downloads report after each
received block; fragmented downloads show the completed fragment count and a
size estimate from the average fragment so far (upstream also counts the current
fragment's announced length). `--no-progress` (implied by quiet unless
`--progress`) prints `[download] Download completed` instead; `--progress-delta
SECONDS` limits updates. Colors and other terminal control sequences are not
implemented. `--console-title` follows upstream: when stderr (else stdout) is a
terminal and `TERM` is set (on Windows, when `windows_enable_vt_mode`'s
`ENABLE_VIRTUAL_TERMINAL_PROCESSING` succeeds; otherwise titles fall back to
`SetConsoleTitleW`), the title is saved
(`ESC[22;0t`) and restored (`ESC[23;0t`) around the run unless simulating, and
each progress status sets the title (`yt-dlp %(progress._default_template)s`
by default) and ConEmu progress state (`ESC]9;4;STATE;PERCENT BEL`), including
with `--no-progress`. The Windows paths are not covered by local tests. JSON and simulation do not download
and emit no download progress. The CLI does not install signal handlers or
expose cancellation flags.

The library observer is a native extension of the transport milestone, not a
port of upstream's progress-hook contract or CLI cancellation behavior. The pinned baseline remains
**897341241f2bfb2439fe0367f4c7c9ccd29a3828**. Deterministic local HTTP tests in
`tests/progress.rs` cover observer events, response-derived/unknown totals,
resumed and restarted counters, cancellation cleanup, and retry-wait polling.

## Source and differential fixtures

This slice uses the existing baseline, **897341241f2bfb2439fe0367f4c7c9ccd29a3828**:

- `yt_dlp/extractor/generic.py`: response dispatch, HTML embed orchestration, and single-vs-playlist dispatch.
- `yt_dlp/extractor/genericembeds.py`, `HTML5MediaEmbedIE`: numbered entry metadata for single videos and playlists.
- `yt_dlp/extractor/common.py`, `_parse_html5_media_entries`: media/source parsing.
- `test/test_InfoExtractor.py`, `test_parse_html5_media_entries`: inline video,
  nested sources, multiple formats, relative URLs, and manifest examples reviewed.
- `yt_dlp/YoutubeDL.py`: playlist entry processing and per-entry JSON output
  reviewed for the bounded sequential orchestration subset.

The full upstream helpers support many behaviors outside this Rust subset.
This is a new port slice from the pinned revision, not an upstream update; the
global baseline has not advanced.

Run the optional differential harness against an official checkout at that SHA:

```sh
cargo build --locked
python3 tools/compare_html5.py --upstream .upstream/yt-dlp --binary target/debug/yt-dlp-rs
```

The harness serves `tests/fixtures/html5/{video,audio,source,formats,playlist,playlist-formats,ranked}.html` and fixed media
bytes on localhost. It invokes upstream with only Generic and HTML5MediaEmbed
enabled and compares `id`, `title`, `ext`, `url`, `webpage_url`, `extractor`, and
`extractor_key` exactly. The sole URL normalization removes the fixture server's
exact origin. Query strings, fragments, paths, titles, and suffixes are preserved.
Other fields (including upstream timestamps and fields outside VideoInfo) are
excluded by this explicit allowlist. No volatile-value masking is applied within
the compared fields. Rust-specific boundary behavior is covered by `tests/http.rs`.
For playlists the
harness compares every entry in order and requires the exact entry count. The
CLI emits per-entry NDJSON, so wrapper fields are covered by library integration
tests rather than this differential harness.

The multi-source fixtures have explicit upstream labels `0` and `1` so each
implementation can request the same source without equating their general ID
assignment rules. Both selections are compared, including for every playlist
entry. The format list, probe sizes,
and Rust-specific IDs are covered by deterministic Rust tests rather than the
upstream metadata allowlist. The multi-source fixture uses page-relative URLs;
`base[href]` resolution remains a separately tested Rust behavior because the
pinned upstream HTML5 embed path does not apply it in this fixture.

The comparison needs upstream source and is optional; required tests need no
upstream checkout, Python runtime integration, or live sites. Seven fixtures do
not establish broad extractor parity. Site playlists, playlist selection, and
recursive result processing remain unimplemented.

## Progressive automatic selection source slice

This slice uses the existing pinned source commit
**897341241f2bfb2439fe0367f4c7c9ccd29a3828** and leaves the global baseline unchanged.
`InfoExtractor._parse_html5_media_entries` in `yt_dlp/extractor/common.py` supplies
source width/height/res handling; its corresponding dimension examples in
`test/test_InfoExtractor.py` were reviewed. `FormatSorter` in
`yt_dlp/utils/_utils.py` supplies the shorter-known-dimension resolution concept,
and `YoutubeDL.build_format_selector` in `yt_dlp/YoutubeDL.py` supplies best/worst
selection direction. Other upstream ordering fields, stream eligibility, label
parsing remain outside this progressive ranking slice; bounded adaptive
expressions are described below. Declaration-order ties
are a native policy; upstream may choose differently for equal resolution.

The saved `ranked.html` fixture compares default, `best`, `worst`, and literal-ID
normalized selected metadata with upstream through `tools/compare_html5.py`.
All alternatives use the same progressive media type and distinct declared
resolutions to isolate the supported ranking criterion. Dimension metadata,
unknown/tie behavior, invalid attributes, explicit overrides, CLI transfers, and
independent playlist selection are covered by deterministic Rust tests.


## Codec fields

Formats carry upstream's `vcodec`, `acodec` and `dynamic_range` from a port of
`parse_codecs` (`src/codecs.rs`, checked against actual pinned upstream with
`tools/compare_codecs.py` and `tests/fixtures/codecs/cases.json`): HLS variant
`CODECS` (with `acodec` `none` when the variant's audio comes from an external
rendition group, and `vcodec` `none` for audio renditions), DASH `codecs`, YouTube
`mimeType` `codecs="..."`, and HTML5 `<source type>` codecs with upstream's quirk
that `vcodec` is then reset to unknown (`none` inside `<audio>`); the media
element's own `src` keeps no codecs. Direct `audio/*` responses get `vcodec`
`none`. Unknown codecs print upstream's `WARNING: Unknown codec X` directly on
stderr (not suppressed by `--no-warnings`/`-q`, like `write_string`). Processing
fills `video_ext`/`audio_ext`, `acodec` from `aac`/`opus`/`mp3`/`flac`/`vorbis`
extensions and `dynamic_range` `SDR` for formats with possible video, like
`process_video_result`; the selected result copies them, and merges take the
video's `vcodec`/`dynamic_range` and the audio's `acodec`. Upstream's explicit
`null` values are omitted. `stream_kind` follows the codec fields like upstream's
`'none'` checks (audio-only when `vcodec` is `none`, video-only when `acodec` is
`none`, otherwise complete, including formats without codecs); unknown codecs only
warn, so HLS variants, DASH representations and YouTube streams with uncommon
codecs are kept. DASH classifies non-video/audio content types by their codecs,
skips text and image representations, warns `Unknown MIME type X in DASH
manifest` for the rest, and labels formats with `mimetype2ext`; YouTube still
probes only `audio/*`/`video/*` streams. `tools/compare_youtube.py` also compares
non-null codec fields.

## Output to stdout (`-o -`)

Like upstream, a default output template of `-` writes the media to stdout:
screen, progress, JSON and `--print` output move to stderr (`logtostderr`), the
default format is `best/bestvideo+bestaudio` without the ffmpeg warning, the media
filename is `-` (sidecars derive from it, e.g. `-.info.json`), `[download]
Destination: -` is printed, and postprocessors, fixups, the archive record and
after-move hooks are skipped. A pair without ffmpeg prints upstream's two
warnings and streams the formats one after the other (upstream fails on the
second stream with `write to closed file`); a pair with ffmpeg is an explicit
error because upstream's FFmpegFD stdout merging is not ported. Each stream is
received into a private temporary file and then copied to stdout, so output
starts only after the transfer completes (upstream writes progressively).

## Generic direct links

Like `GenericIE`, a Content-Type matching `audio/*`, `video/*`, `application/ogg`
or an m3u8 type is a direct link: `ext` is `determine_ext` of the requested URL,
else `urlhandle_detect_ext` (Content-Disposition `filename*`/`filename`,
`x-amz-meta-name`, `x-amz-meta-file-type`, then the full `mimetype2ext` table with
its subtype fallback), and the format ID is the MIME subtype. An untyped or
`application/octet-stream` response is returned with upstream's "URL could be a
direct video link" warning and `ext` from the URL when it is in
`_UnsafeExtensionError.ALLOWED_EXTENSIONS`, else `unknown_video`. Downloads (not
`-j`/`-s`/`--get-filename`) refuse media or format extensions outside that list
with `The extracted extension ('EXT') is unusual and will be skipped for safety
reasons` (without upstream's bug-report link). Responses with other non-HTML
types keep the native MIME recognition, and upstream's HTML sniffing of the first
bytes and redirect re-extraction are not ported.

## Postprocessor messages

Processed downloads print upstream's postprocessor lines in chain order after the
transfer's progress ends: `[ExtractAudio] Destination: FILE` (or `Not converting
audio FILE; could not find a mapping for EXT` / `the file is already in a common
audio format` / `file is already in target format TARGET`),
`[VideoRemuxer] Remuxing video from SRC to TGT; Destination: FILE` and
`[VideoConvertor] Converting video ...` (or `Not remuxing/converting media file
"FILE"; already is in target format EXT` / `could not find a mapping for EXT`),
`[EmbedSubtitle] Embedding subtitles in "FILE"`, `[Metadata] Adding metadata to
"FILE"`, and `run_pp`'s `Deleting original file FILE (pass -k to keep)` after
each processor that replaces its input (not with `-k`; subtitles kept by
`--write-subs` are not deleted). Subtitle sidecars print `[info] Writing video
subtitles to: FILE` (or `[info] Video subtitle LANG.EXT is already present`) and
their own `[download] Destination` and progress lines. Checked against pinned
upstream with real media. Merges followed by processing still print no
per-format lines, and processed HLS downloads print no `[FixupM3u8]` line.

## Format descriptions

Like `process_video_result`, formats get `resolution` (`format_resolution`:
`audio only`, `WxH`, `Hp` or `Wx?`), `aspect_ratio` (`round(width / height, 2)`)
and, after IDs are assigned, `format` (`'{id} - {resolution or unknown}'` plus
` (format_note)`); results get `fulltitle` and `duration_string`
(`formatSeconds`). The selection copies them; merges join `format` with `+` and
take the video's resolution and aspect ratio. Templates can use `format`,
`resolution`, `protocol`, `vcodec`, `acodec`, `dynamic_range`, `video_ext`,
`audio_ext`, `duration_string`, `fulltitle`, `thumbnail` and the numbers
`aspect_ratio`, `fps`, `tbr`, `vbr`, `abr`, `asr` and `audio_channels`, rendered
like Python (`%s`/`%j` use `repr`, ints stay ints, so DASH/YouTube `fps` is `30`
and HLS `29.97`). Results copy the selected format's rates; merges sum `tbr`
(each format's first of tbr/vbr/abr) and take the video's `fps`/`vbr` and the
audio's `abr`/`asr`/`audio_channels`. Checked against pinned upstream on local
HLS, DASH and HTML5 pages and a saved info JSON.

## Format sorting

Every extracted or loaded video's formats are sorted like upstream
`process_video_result`: per-format fills (`protocol`, `video_ext`/`audio_ext`,
`acodec` from audio extensions, `dynamic_range` `SDR`, and `_fill_sorting_fields`
`vbr`/`abr`/`tbr`, with `0` for a missing stream), then a port of `FormatSorter`
(`src/format_sort.rs`) orders them worst first, and only then do formats without
an ID get their index (HTML5 `<source>` IDs come from `label`/`title`, otherwise
the sorted index). The sorter implements the default field order, field settings
(ordered regex lists, extractor/boolean/multiple/combined fields, aliases),
`-S` limits/`~` closest/`+` reverse, `--format-sort-force` and
`--prefer-free-formats` orders; 178 saved cases are checked against actual pinned
upstream with `tools/compare_format_sort.py`. YouTube results carry upstream's
`_format_sort_fields` and per-format `quality` (`qualities` over
`quality`/`audioQuality`, DRC −0.5), `source_preference` (−5 for itag 22, +100 for
Premium), `fps`, `asr` and `audio_channels` (included in `tools/compare_youtube.py`).
HTML5 sources parse `width`/`height`/`res` like Python `int()`, fall back to
`parse_resolution` of `label`/`title`, and take `tbr` from `parse_bitrate`.
Like upstream, direct and HTML5 progressive formats have no `filesize` at
extraction time (probes still validate the media; transfers learn Content-Length,
so `--min/max-filesize` act during download), YouTube takes it from
`contentLength`, and `direct` is present only (as true) for generic direct links,
so templates render `NA` and `[filesize...]` filters need `?` for such results.
Remaining sort-input differences: unrepresentable negative or larger-than-`u32`
dimensions are unknown natively (upstream keeps them); upstream
`filesize_approx`, `preference` and `language_preference` are not yet represented.
HLS `FRAME-RATE` gives `fps` (`float_or_none`); DASH `frameRate` and
`audioSamplingRate` give `fps`/`asr` (`int_or_none`, so fractional rates stay
unknown), DASH formats get upstream's
`DASH video`/`DASH audio` `format_note`, and the `mul`/`und`/`zxx`/`mis` languages
are dropped.

| Option | Behavior |
| --- | --- |
| `--ignore-config`, `--no-config`, `--config-locations PATH`, `--no-config-locations` | Upstream `parseOpts`/`Config` configuration files with this port's own `yt-dlp-rs` names: the command line, then a portable `yt-dlp-rs.conf` beside the executable, the home `yt-dlp-rs.conf` (`-P` home or the current directory), the first user file (`$XDG_CONFIG_HOME` or `~/.config`, `%APPDATA%`, `~/.yt-dlp-rs`, each trying `../yt-dlp-rs.conf`, `[../yt-dlp-rs.conf.txt]`, `config`, `config.txt`) and the system `/etc` file, stopping at `--ignore-config` (inside the system file it drops the user file). Files are BOM/`# coding:`-decoded and split with a port of `shlex.split(comments=True)` (checked by `tools/compare_shlex.py`); `--config-locations` (relative to the naming file, directories use `yt-dlp-rs.conf`, `-` reads stdin) nest before their file's arguments, earlier locations win, each file loads once; `config location X does not exist`/`Unable to parse` usage errors. Later configurations and the command line override earlier values like optparse (repeated single-value options keep the last). Verified case by case against pinned upstream with the upstream names; `--verbose` prints the configuration tree |
| `-S`, `--format-sort SORTORDER` | Comma-separated fields; later values are prepended (upstream `append=-1`); validated with `FormatSorter.regex` (`invalid format sorting "X" given`) |
| `--format-sort-reset` | Discards `-S` values given before it |
| `--format-sort-force`, `--S-force`, `--no-format-sort-force` | User fields precede the priority fields (`hasvid`, `ie_pref`); last flag wins |
| `--prefer-free-formats`, `--no-prefer-free-formats` | Free-container `vext`/`aext` orders; last flag wins |

## Direct adaptive selection and merging

`StreamKind` adds `standalone` (the default, omitted from JSON), `video_only`,
and `audio_only`. Generic sources retain standalone classification because their
track composition is unknown. Optional `tbr`/`abr` are positive finite declared
kilobits per second, not measured quality. YouTube uses averageBitrate then bitrate.
The anonymous WEB API direct-format path still accepts only itag 18; adaptive support does not bypass
that token policy.

`best`/`worst` choose standalone sources; an audio-only fallback is allowed only
when neither standalone nor video-only sources exist. `bestvideo`/`worstvideo`
choose video-only and `bestaudio`/`worstaudio` audio-only sources. Every selector
takes candidates in the sorted `formats` order described under
[Format sorting](#format-sorting): `best` is the last match and `worst` the first.
Literal IDs also select isolated streams.

See [Format selection](#format-selection) for the selector itself. Downloads
of a merge need one video-only and one audio-only stream; other merges (two
videos after `_merge`'s deduplication, for example) are explicit download
errors. Selection errors leave metadata unchanged, including across playlists. Without an explicit selector the default
is upstream's `_default_format_spec`: `bestvideo*+bestaudio/best` with FFmpeg,
otherwise `best/bestvideo+bestaudio`, warning `ffmpeg not found. The downloaded
format may not be the best available. ...` when that picks something other than
`bv*+ba/b`. Without FFmpeg a selected pair is downloaded to its `NAME.fID.EXT`
format files without merging after upstream's `You have requested merging of
multiple formats but ffmpeg is not installed. The formats won't be merged`
warning (an error under `--abort-on-error`, before downloading).
An explicit pair can be inspected with JSON/simulation without installing FFmpeg.

A merge plan carries `requested_formats` in expression order and a joined
`format_id`. Top-level `url` represents the first source, not a merged HTTP URL;
callers must honor the plan. Pairs need MP4/WebM/TS video and M4A/WebM/AAC/TS audio
(fixed local demuxers); the output container is upstream's `get_compatible_ext`
over the video's `vcodec`/`ext` and the audio's `acodec`/`ext` (for example
avc1+mp4a gives `mp4`, VP9+Opus `webm`, unknown codecs in `mp4`+`aac` `mkv`), with `--merge-output-format` / `YoutubeDl::with_merge_output_format`
preferences. The selected video's dimensions
are retained; merged filesize is unknown until download. Added serde fields have
defaults for older metadata, but Rust struct literals must supply the new fields.

`YoutubeDl::with_ffmpeg(path)` or `--ffmpeg-location PATH` permits downloading
both sources into a private destination-directory workspace, then stream copying
with explicit video/audio maps. Inputs use fixed local demuxers and the file-only
protocol whitelist. The executable is canonicalized; its version banner is checked.
No shell is used, environment is cleared except Windows system directories, stdin
is closed, and process output and elapsed time are bounded (64 KiB stdout,
256 KiB stderr, ten minutes shared by version check and merge). A process error,
nonempty error diagnostics, or empty output fails explicitly. No codec fallback
or transcoding occurs. A bad tool may fail after source downloads.

Each input receives the usual download retry budget. Progress includes one-based
`stream_index` and `stream_count=2`; input byte counts reset for the next source.
Input publication events are hidden. `Merging` reports the sum of stored input
bytes, with cooperative process cancellation polling. Final `Publishing` reports
the merged output size. Only that final file is published, using no-clobber
semantics; cancellation/failure removes both inputs and staging output. Single
stream downloads never inspect the configured FFmpeg path.

The CLI uses upstream's merge files (`MergeFiles::Upstream` in
`DownloadOptions`; the library default stays private) for merges without
postprocessing: each format goes to `correct_ext`/`prepend_extension`'s
`NAME.fFORMAT_ID.EXT` beside the destination (derived from the rendered
destination, so templates whose other fields depend on the format are not
re-rendered per format) with `.part` files as configured, announced with its
`[hlsnative]`/`[dashsegments] Total fragments` and `[download] Destination` lines
and finished status; an existing format file is reused with `[download] FILE has
already been downloaded`. Then `[Merger] Merging formats into "FILE"` and, unless
`-k`, `Deleting original file FILE (pass -k to keep)` for each format in selection
order (upstream's order comes from a set). Format files are kept after a failure.
Input publication events are reported with their `stream_index` in this mode.
Merges followed by postprocessing still use the private workspace and print the
final `Destination` line. The messages were compared by hand with pinned upstream
on a local DASH page; `tests/merge.rs` holds the regression cases.

Optional real-tool regression: set both `YT_DLP_RS_TEST_FFMPEG` and
`YT_DLP_RS_TEST_FFPROBE` to executable paths and run `cargo test --locked --test merge`.
The generated local media fixtures verify that copied output retains audio and
video tracks; this is not a claim that every FFmpeg version or codec pair works.
See [source and fixture provenance](../upstream/notes/youtube-adaptive-merge.md).


## Native HLS VOD and fragment transport

Direct HLS playlist URLs and HTML5 HLS sources support a finite VOD
subset, including identity AES-128 encryption. Recognize MPEGURL MIME types, final `.m3u8` URL paths, and explicit HTML5
HLS type hints. A conflicting media MIME must not cause a manifest to be saved as
one media file. Read at most 2 MiB of uncompressed UTF-8/ASCII manifest text;
ambiguous encoding/content-type headers fail. URLs resolve against each final
redirected manifest URL. HTML5 page referrers propagate to manifests and segments.

Format IDs follow upstream `_parse_m3u8_formats_and_subtitles`: a media
playlist is one format (`hls` from HTML5 sources, otherwise numbered `0`); master
variants are named by `NAME`, else their `VIDEO` group's rendition name or group
ID, else the bitrate in kbps (`AVERAGE-BANDWIDTH` preferred) or the format count,
prefixed `hls-` for HTML5 sources; audio renditions are `GROUP-NAME` and are
listed before the variants. Like upstream `process_video_result`, characters
used by format selectors become `_`, duplicate IDs get `-N` suffixes and IDs
that name another common extension get an `f` prefix. Variants keep positive
declared bandwidth in kbps, optional resolution and codec-derived stream kinds. Existing standalone selection retains resolution/declaration-order
ranking, not upstream's full bandwidth/codec sorter. Every variant manifest is
read before selection; segment bodies are not probed. One master level, at most
256 variants and 10000 retained fragments across a master are supported.

Media playlists require `EXT-X-ENDLIST`, nonempty positive-duration segments and
HTTP(S) URLs. A nonzero media sequence is permitted for a completed VOD. Without
an initialization map, `.aac` segments produce AAC and other ordinary/extensionless
segment URLs default to MPEG-TS (`ts`); clear nested manifests/HTML and MP4 media
without initialization are rejected. One initial `EXT-X-MAP` produces fragmented
MP4 (`mp4`). This is manifest interpretation, not a universal container probe.
Native assembly preserves the segment bytes; TS is not silently labeled MP4 or
remuxed. FFmpeg is unnecessary for this download path.

`EXT-X-BYTERANGE` supports explicit offsets and implicit continuation only after
an immediately preceding media range on the same URL. Map byte ranges require
an explicit offset. All ranges are positive with checked arithmetic. Unsupported
state is an error: live playlists without ENDLIST, SAMPLE-AES/DRM or unsupported
key formats, gaps, low-latency tags, variables, changing maps (also a map
after a discontinuity),
external video or unresolved rendition groups, mixed inband/external
audio groups, nested masters and unknown EXT tags.
`EXT-X-MEDIA:TYPE=SUBTITLES` renditions with a URI become subtitles like
upstream's `extract_media` (by `LANGUAGE`, default `und`; an m3u8 URI is `vtt`
with protocol `m3u8_native`), listed by `--list-subs` and written with
`--write-subs` through upstream's HlsFD flow (`[hlsnative] Downloading m3u8
manifest`, `Total fragments: N`): a single segment is copied as it is, several
are joined by a port of `yt_dlp/webvtt.py` and HlsFD's WebVTT packing
(`X-TIMESTAMP-MAP` offsets with 2^33 wraparound, cues continued across segments
merged, header blocks after the first segment discarded with a warning that,
unlike upstream's, does not point to yt-dlp's tracker). Encrypted or byte-range
subtitle segments are unsupported. DASH text representations (`contentType` `text`, or
a subtitle codec or TTML/XML/JSON MIME type) with a direct `BaseURL` become
subtitles by `lang` (default `und`) with `mimetype2ext` extensions, as upstream's
"direct URL to unfragmented media"; segmented text tracks keep the manifest URL
with protocol `http_dash_segments` and are written through DashSegmentsFD's flow
(`[dashsegments] Total fragments: N`, the fragments appended as they are). Fragment progress lines use this port's
renderer.
`EXT-X-DISCONTINUITY` (and, like upstream's prefix match,
`EXT-X-DISCONTINUITY-SEQUENCE`) lines are counted per fragment: downloads span
every section as HlsFD does, and `--hls-split-discontinuity` lists one format
per section (`format_index` 0..N, the index joined to the format ID unless 0,
for media playlists, variants and audio renditions), downloads only that
section after the manifest refetch, and still reports the playlist's total
`Total fragments`. `--no-hls-split-discontinuity` restores the default.
In-band rendition references without external resources are supported. Plain
comments and recognized informational metadata do not change assembly.

`VideoInfo.fragments` and each format's `fragments` contain ordered `Fragment`
values, including initialization when needed. Each has a URL and optional
`ByteRange { offset, length }`; absent fields deserialize to empty/default values.
Selection copies the chosen plan; a merge plan retains each child's fragments.
Extraction snapshots these plans, and downloads do not refetch manifests.

Formats carry upstream's `protocol`: `m3u8_native` for HLS, `http_dash_segments`
for DASH, and otherwise `determine_protocol` (`rtmp`/`mms`/`rtsp` prefixes, `.m3u8`
/`.f4m` URLs, else the URL scheme), filled for every extracted format like
`FormatSorter._fill_sorting_fields`; the selected result copies it, and merges join
both with `+`. A single fragmented CLI download prints upstream's
`[hlsnative] Total fragments: N` or `[dashsegments] Total fragments: N` line before
`Destination`. Like upstream HlsFD, each HLS download (including every
merged stream) first prints `[hlsnative] Downloading m3u8 manifest`, refetches
its media playlist and downloads the fragments planned from that response;
merges print each stream's downloader lines.

The fragment downloader retries each whole failed fragment using
`--fragment-retries`, retains earlier complete fragments, and truncates the
failed fragment before retrying. The library default never skips a failed
segment; the CLI follows upstream's `--skip-unavailable-fragments` default. Exact ranges require matching 206,
Content-Range, known resource bounds, validated length and identity encoding;
ignored ranges, unsolicited partials, HTML/manifests and empty fragments fail.
Private temporary output is removed on errors/cancellation; final publication
uses the same no-clobber race protection as direct downloads.

Progress adds `fragment_index` and `fragment_count` with one-based positions;
bytes count the assembled output so far, including initialization, and can reset
to the previous complete prefix on retry. Attempt numbers restart per fragment.
Total bytes remain unknown. Observers can cancel before requests, after writes,
during retry waits and before final publication. Blocking reads retain normal
request timeouts. These new public fields require updating Rust struct literals.

The saved TS, fMP4 and byte-range media tests verify exact bytes and optionally
FFprobe audio/video tracks using `YT_DLP_RS_TEST_FFPROBE`. The offline differential
harness compares actual pinned upstream HlsFD fragment order, URLs and ranges:

```sh
python3 tools/compare_hls.py --upstream /path/to/yt-dlp --binary target/debug/yt-dlp-rs
```

Only the exact ephemeral localhost origin and upstream exclusive range end are
normalized. Container naming/remux behavior, live/encrypted delivery and general
HLS parity are not inferred. YouTube HLS discovery uses this same bounded VOD
pipeline. See [the source slice](../upstream/notes/hls-vod.md).


### HLS AES-128

`EXT-X-KEY:METHOD=AES-128` supports raw 16-byte identity keys from HTTP(S) URLs,
optional `KEYFORMAT="identity"` / `KEYFORMATVERSIONS="1"`, and CBC with strict
PKCS#7 padding. `METHOD=NONE` clears encryption. Explicit IVs are hexadecimal
integers of up to 128 bits, left-padded to 16 bytes. Otherwise the IV is the
media segment sequence number in a 16-byte big-endian buffer. Initialization
maps require an explicit IV while encrypted, and do not consume media sequence
numbers. These rules follow [RFC 8216 sections 4.3.2.4, 4.3.2.5 and 5.2](https://www.rfc-editor.org/rfc/rfc8216.html).

Each fragment's optional `encryption` contains `HlsAes128 { key_url, iv }` only;
key bytes never appear in metadata. JSON and simulation request neither keys
nor segment bodies. Downloads retrieve keys lazily, preserving the page referrer
and identity encoding, with exact 16-byte length and response validation. A URL's
validated key is cached only for the current download; rotation to a different
URI fetches a new key. Key and segment requests share that fragment's retry
budget. Cancellation includes key request/read checkpoints.

RustCrypto `aes`/`cbc` performs bounded streaming decryption without an external
process. The last ciphertext block is held until EOF for padding validation.
Ciphertext lengths/ranges are validated against network headers; stored byte
counts, publication size and observer progress use plaintext bytes. Key bytes and
padding do not contribute to progress. Invalid keys/ciphertext/padding and
unsupported methods fail without publication. Earlier plaintext may exist only
in the private temporary file until all fragments pass validation.

Original OpenSSL-encrypted TS, fMP4/map and byte-range fixtures decrypt to the
exact original bytes and pass optional FFprobe track checks. Fixed NIST/RustCrypto
vectors cover CBC and padding independently of the fixture generator. Parser and
local HTTP tests cover rotation, NONE, sequence bounds, key caching, retries,
malformed responses and cancellation. The upstream plan comparison additionally
checks key URLs and IVs. Pinned upstream increments its counter for MAP; native
implicit-IV-after-MAP behavior follows RFC media sequence numbering, and that
specific upstream equivalence is explicitly excluded. Explicit-IV MAP plans
are compared without masking this difference.

SAMPLE-AES, DRM key formats, session keys, custom key overrides and live/key
refresh across playlist reloads remain unsupported. See the
[AES source slice](../upstream/notes/hls-aes128.md).


### External HLS audio renditions

`EXT-X-MEDIA:TYPE=AUDIO` declarations with HTTP(S) playlist URIs expose
upstream-named `GROUP-NAME` formats before the variants. Per-format metadata
contains `audio_group`, optional `language` and `format_note` (NAME), and true
`audio_default` / `audio_autoselect` flags. Missing flags are false. Empty/duplicate
names in a group, multiple default renditions, invalid flag values, and explicit
DEFAULT=YES with AUTOSELECT=NO fail. Audio-only rendition masters are supported.

A variant that references an external audio group with URIs and declares video
CODECS becomes video-only (`acodec` `none`) even when CODECS describes both tracks;
like upstream, a variant without CODECS (or with audio-only CODECS) is kept as a
complete (or audio-only) format, and an unknown AUDIO group is ignored. Inband
groups retain the existing standalone behavior. Both variant and rendition
declarations count toward the combined limit of 256; every child playlist is
read with the existing aggregate 10000-fragment cap. JSON/simulation fetch neither
segments nor encryption keys. Like upstream (`ext='mp4'`), every HLS format is
labeled `mp4`, including audio renditions and raw-AAC or TS segments.

Like upstream, rendition groups do not constrain pairs: `bestvideo+bestaudio`
takes the best video and the best audio in sorted order. Renditions with
`DEFAULT=NO` or `AUTOSELECT=NO` get `source_preference` -2; with several audio
groups, the others get their group's index among the groups ordered by their
videos' bandwidth and width, and the lowest/highest groups append `low`/`high`
to `format_note`. Expression order is preserved in the plan even for
`bestaudio+bestvideo`. Explicit language predicates are available through the
shared format-filter grammar. `audio_group` remains a native informational field.

HTML5 audio groups receive a source index prefix, so equal group names from
independent source masters cannot pair automatically.
Without FFmpeg configuration, implicit default selection (`best`) fails when
separate video and audio formats exist without a complete one (upstream would
select `bestvideo+bestaudio` and download the streams unmerged). An explicit
pair may still be inspected with JSON. With FFmpeg, implicit default is
upstream's `bestvideo*+bestaudio/best` and downloads both native
fragment plans before merging. TS video and AAC/TS audio use fixed local demuxers;
MP4 muxing relies on FFmpeg's AAC ADTS-to-ASC handling. No transcoding is introduced.

Local tests cover group/default selection, explicit overrides, audio-only masters,
validation, HTML namespaces, headers and no-segment simulation. Original separate
H.264 TS and AAC fixtures exercise the complete native-download/FFmpeg path and
verify both tracks and AAC codec metadata. The optional pinned-parser comparison:

```sh
python3 tools/compare_hls_audio.py --upstream /path/to/yt-dlp --binary target/debug/yt-dlp-rs
```

It compares URL-keyed stream kind, language/name, dimensions and total bitrate.
Format IDs/order, native group/flag fields, selection, ext/remuxing and download
behavior are explicitly excluded from that comparison. Only the exact local
origin is normalized. See [the source slice](../upstream/notes/hls-audio.md).


## Static DASH MPDs

Direct `.mpd` paths, `application/dash+xml`, and HTML5 DASH type hints support a
bounded static, single-period MPD subset. Original/final MPD paths and explicit
hints force parsing even with a conflicting media MIME, preventing MPD text from
being published as media. Read at most 2 MiB of identity-encoded UTF-8 XML;
ambiguous/invalid headers, DTDs, foreign element namespaces and external XML
references fail. No schema, xlink, initialization or media URL is fetched during
extraction or JSON/simulation. XML nodes/depth are bounded at 100000/64.

`BaseURL` resolves through MPD, Period, AdaptationSet and Representation, with one
base per level. SegmentTemplate attributes/timelines and SegmentList initialization
and segment lists inherit within the same addressing scheme. Supported template
tokens are RepresentationID, Bandwidth, Number and Time, dollar escaping and
bounded zero-padded decimal numeric forms. Initialization permits only RepresentationID
and Bandwidth; media cannot combine actual Number and Time tokens. Static duration
templates require a
finite Period or MPD duration. Exact day/hour/minute/second ISO durations use
nanosecond precision (up to nine fractional second digits), checked integer
arithmetic and ceiling division. Explicit Period and MPD duration bounds must agree.
No floating-point segment count is used.

SegmentTimeline supports explicit/implicit starts, positive durations, finite
repeats and bounded `r=-1` expansion to an aligned next start or finite period
end. Nonzero presentationTimeOffset is applied to period bounds and duration
Time templates; timelines that start before the offset require clipping and are
rejected. Like upstream, every listed S entry and SegmentURL is kept even past a
finite period end, and SegmentTemplate/SegmentList initialization is optional
(the plan then starts with the first media segment). Namespaced
addressing attributes fail rather than being interpreted as ordinary DASH fields.
The simple-addressing offset policy follows the
[DASH-IF restricted timing model](https://dashif.org/Guidelines-TimingModel/).
Unknown eptDelta and advanced timing attributes remain unsupported. Final segment
bytes are not cropped to presentation boundaries; no trimming/remux parity is
claimed.

SegmentList supports Initialization sourceURL/range and SegmentURL media/mediaRange.
Inclusive XML byte endpoints become exact ByteRange offset/length values. Explicit
resource BaseURLs may supply omitted range-resource URIs. Fragmented MP4/WebM
plans require initialization; an explicit whole-resource non-directory BaseURL
can instead form a single-fragment representation. Resolved URLs are limited to
16 KiB, formats to 256 and aggregate fragments (including initialization) to 10000.

Formats use upstream IDs (the representation `id`, else the content type; HTML5
sources add no prefix, as upstream passes no `mpd_id`), declared stream kind, MP4/M4A
or WebM extension, dimensions, bandwidth-derived kbps and inherited language.
Video/audio representations share the native `dash-period-0` group; HTML source
prefixes isolate independent masters. Their representative URL is the MPD URL;
the ordered fragments describe actual resources. Existing selectors, per-fragment
retries/cancellation, no-clobber publication and explicit FFmpeg merge policy apply.
Without a merger, separate-only presentations need explicit single-track selection.

Reject dynamic MPDs, multiple periods, ContentProtection/DRM, SegmentBase/SIDX and
index attributes, dependent/scalable representations, SubRepresentation,
EssentialProperty, remapping, multiple BaseURL alternatives, mixed inherited
addressing schemes and unsupported state. YouTube MPD discovery remains outside
its extractor. No live/site reliability or general DASH parity is implied.

Original local fMP4 fixtures verify exact single-track bytes and optional FFprobe
H.264/AAC tracks for timeline, duration-template, list and range forms. The real
merge test downloads both representations and checks the combined tracks. The
Linux real-tool job includes these checks. Pinned reference command:

```sh
python3 tools/compare_dash.py --upstream /path/to/yt-dlp --binary target/debug/yt-dlp-rs
```

The actual upstream parser receives saved XML with network access blocked; native
JSON reads only localhost MPDs. A documented allowlist compares declaration-order
metadata and fragment URLs, resolving upstream relative paths and normalizing
only the exact local origin. Source ID and audio bitrate representations have
explicit mappings. Pinned upstream ignores SegmentList ranges and does not expand
negative repeats or apply presentationTimeOffset: native range expectations are
checked independently, and those timing behaviors are excluded from upstream
parity claims. See [the source slice](../upstream/notes/dash-static.md).


## YouTube extractor arguments

`--extractor-args "youtube:..."` arguments of the single-video extractor
(`_configuration_arg`; values lowercased unless upstream reads them
case-sensitively):
- **`formats`:** `duplicate` keeps formats with an already seen stream ID and
  names manifest formats `ITAG-PROTO` (with the client's short name in the
  note, also for the deprecated `include_duplicate_formats`, which prints
  upstream's `Deprecated Feature:` line); `dashy` turns HTTPS formats with a
  size into 10 MiB-range `http_dash_segments` formats (both kinds, as
  `ID-dashy`, with `duplicate`); `incomplete` keeps live adaptive formats and
  manifests; `missing_pot` keeps formats lacking a required PO token (`MISSING
  POT` note; manifest formats lose 20 source preference).
- **`skip`:** `hls`, `dash` skip the manifests; `translated_subs` skips
  translated manual captions.
- **`player_skip`:** `configs` (no client config pages), `webpage` (no watch
  page; the default `ytcfg` and the `next` API's initial data are used), `js`
  (no player JS: signature/`n` formats are dropped and no iframe fallback),
  `initial_data` (no `next` API request).
- **`webpage_client`:** `web` (default) or `web_safari` (its User-Agent fetches
  the page, and its client reuses the page's player response); other values
  warn and fall back to `web`. **`webpage_skip`:** `player_response`,
  `initial_data` ignore the page's data.
- **`player_js_version`:** `STS@HASH` forces the signature timestamp and
  player ID (and skips the page's player response); invalid values warn.
  **`player_js_variant`:** one of upstream's variant names (`main` default,
  `actual` keeps the page's); the player JS cache key is `ID-VARIANT`.
- **`player_params`:** the player request's `params` and the page's `pp`.
  **`use_ad_playback_context`:** adds `adPlaybackContext` for clients
  supporting it.
- **`visitor_data`, `data_sync_id`:** replace the extracted values (the
  `X-Goog-Visitor-Id` header, session IDs and PO token checks).
  **`innertube_host`, `innertube_key`:** the API host (and `Origin`) and the
  `key=` query parameter.
- **`raise_incomplete_data`:** an incomplete response raises after its retries
  (a warning for player requests, as upstream catches it).
- **`lang`:** one of upstream's `_SUPPORTED_LANG_CODES` (case-sensitive;
  others fail with upstream's `Unsupported language code` error), used as the
  `PREF` cookie's and InnerTube context's `hl`; any value prefers the
  microformat's translated title and description, and values other than `en`
  print the `Preferring "LANG" translated fields` warning and silence
  `Cannot parse localized time text` warnings. (The playlist, search and clip
  extractors fall back to `en` instead of failing on an unsupported value.)
- **PO tokens:** `po_token=CLIENT.CONTEXT+TOKEN[,...]` (`gvs`, `player` or
  `subs`; a missing context is GVS) like `_get_config_po_token`: the token is
  URL-unquoted and re-encoded as padded base64url (as the Python 3.15 that
  captured the fixtures decodes it: unpadded input is accepted, characters
  outside the alphabet are dropped), with upstream's warnings for a missing
  `+` and undecodable tokens. `fetch_po_token`'s checks warn for a missing
  visitor data (anonymous, JS clients) or Data Sync ID (accounts). A player
  token goes into the player request's `serviceIntegrityDimensions.poToken`
  (not for the page's own response) and lifts GVS requirements marked
  `not_required_with_player_token`; a GVS token is appended to HTTPS URLs
  (`pot=`) and manifest paths (`/pot/TOKEN`); a subtitles token to caption
  URLs. Upstream ships no PO token providers: they are Python plugins
  (`yt_dlp_plugins` and `youtubepot-*` arguments), which this native port
  cannot load, so `fetch_pot=never|auto|always` and `pot_trace` are accepted
  without effect (no `[pot]` provider lines are printed).

`tests/fixtures/sites/youtube-args`, `youtube-pot`, `youtube-client-args` and
`youtube-lang` compare these with pinned upstream output (player requests
are matched by their JSON body where an argument changes it).

## YouTube playlists

`youtube:tab` ports upstream `YoutubeTabIE` (`yt_dlp/extractor/youtube/_tab.py`)
for `/playlist?list=ID` and `watch?v=ID&list=ID` URLs (the latter only without
`--no-playlist`; `watch?list=ID` warns and downloads the playlist) and channel
tabs (`/@HANDLE/TAB`, `/channel/UC…/TAB`, `/c/NAME`, `/user/NAME`), also on
upstream's Invidious hosts and never on `consent.youtube.com`; a tab URL with
a `v` but no `list` parameter is that video (`_yes_playlist`). A channel
URL without a tab reads its `/videos` tab (`Downloading all uploads of the
channel`) and, like upstream, extracts its live and shorts tabs too; a channel
with several of them is a playlist of the tabs' playlists (`Downloading as
multiple playlists, separated by tabs`). Such nested playlists are processed
like upstream's `__process_playlist` entries: `--playlist-items` and the other
selection options apply to the tabs and then to each tab's entries, every tab
prints its own `Downloading playlist`/`Finished downloading playlist` lines
between the outer playlist's, the `playlist` print/exec stage runs for each
level, and `-J` nests the processed tab playlists (with `__x_forwarded_for_ip`)
in the outer playlist's `entries`. With `--playlist-items 0` (exactly),
like upstream, the live and shorts tabs are not extracted but stay `url`
entries after the videos tab's playlist (one such tab alone is followed);
index `0` selects nothing and, as upstream's `slice(0, 0)`, reads no
continuation page. Archive and match filters apply to the
tabs' videos (upstream also matches the tab playlists themselves, which pass as
incomplete dicts). Topic channels without tabs are
redirected to their uploads (`UU…`) playlist; `/about` gives the empty
playlist; a missing tab is `This channel does not have a TAB tab`, a missing
live tab `The channel is not currently live`, and a home-page redirect or
conditional regional redirect is handled as upstream (all skipped with
`--compat-options no-youtube-channel-redirect`, which reads a channel URL
without a tab as given; a webpage redirected to the home page is rejected
unless the URL is `/feed/recommended`). It reads the
page's `ytInitialData` and `ytcfg`, makes upstream's web-client browse request
for unavailable videos (`Redownloading playlist API JSON with unavailable
videos`, `params: wgYCCAA=`; not with `--compat-options
no-youtube-unavailable-videos`), reports alerts like `_report_alerts`, and follows
continuations (`Downloading API JSON`, item `ID page N`) until none is left or a
token repeats. Like upstream's lazy entries generator, continuation pages are
fetched when the playlist is processed (after `Downloading playlist: TITLE`)
and only as far as `--playlist-items`/`--playlist-end` need with positive
bounds; open or negative bounds read every page. Until the entries were read
past their end, the playlist's count is its page's video count, if any, else
unknown (`Downloading N items` without `of M`, `playlist_count` `NA`). A page
without usable initial data is retried three times (`Incomplete yt initial
data received`) and then resolved through the `navigation/resolve_url` API, and
page alerts are reported before the incomplete-data check, as upstream's
`_extract_webpage` does; a failed page download is retried like upstream's
`RetryManager` (`HTTP Error 404: Not Found. Retrying (N/3)...`, then `…
Giving up after 3 retries`), except for HTTP 403 and 429.
`--extractor-args youtubetab:skip=webpage` skips the page and reads the
`navigation/resolve_url` endpoint directly (YouTube search also skips its web
client config). Without a page's `ytcfg`, an account's cookies make this
`_report_playlist_authcheck`'s error (`Playlists that require authentication
may not extract correctly without a successful webpage download. If you are
not downloading private content, …`), a warning with `skip=authcheck` and, for
search, always a warning. `youtubetab:approximate_date` (any value) gives
`_extract_video` and lockup entries `_parse_time_text`'s `timestamp` from their
relative or absolute time text (`upload_date` follows in `--flat-playlist`
output), warning `Cannot parse localized time text "TEXT"` for video
renderers. Its relative times depend on the clock, so the fixture prints them
only as set or unset. The first page's entries are extracted at once (upstream
extracts them as the playlist is processed), so such warnings also cover
first-page entries outside the selection. The playlist has `_extract_metadata_from_tabs`' fields
(title, availability, follower count, description, tags, thumbnails, modified
date, view and video counts, channel/uploader) in upstream's key order, and its
entries are `_extract_video`'s, `_extract_lockup_view_model`'s and the Shorts
lockups' `url` dicts (`ie_key: Youtube`), which
`--flat-playlist` and `-J` print as upstream does and normal runs resolve
through the `youtube` extractor. An inline watch-page playlist is followed to
its playlist page (a mix is extracted inline, see below). Playlist thumbnails are sorted and given IDs and resolutions like
`_sanitize_thumbnails`. The saved site fixtures `tests/fixtures/sites/youtube-playlist/`
(page, reload and continuation responses) and `tests/fixtures/sites/youtube-channel/`
(video, Shorts and lockup entries, `/about`, a missing tab, a channel root) and
`tests/fixtures/sites/youtube-tab-options/` (regional and home-page redirects,
unavailable-video reloads, with and without the compatibility options;
`youtubetab` arguments, cookies with and without the auth check, approximate
dates) are compared with pinned upstream output.

With `--lazy-playlist`, the requested pages are still fetched before the
first entry is processed. An inline watch-page playlist without a playlist page
(mixes, `RD…`) is `_extract_inline_playlist`'s: the page's videos, then `next`
API pages (`ID page N`) as needed until a page adds no new video; the fixture
`tests/fixtures/sites/youtube-mix/` covers it. YouTube Music URLs are handled
like upstream's: `music.youtube.com` playlists and channels are read on
`www.youtube.com` after `YouTube Music is not directly supported. Redirecting to
URL`, `/browse/VL…` becomes its playlist, `/browse/UC…` its channel and an album
(`/browse/MP…`) its canonical playlist (through the `web_music` client's
`navigation/resolve_url`), and a music playlist's URL entries stay
`music.youtube.com` URLs. Hashtag (`/hashtag/TAG`) and feed (`/feed/NAME`)
pages are read like other tab pages (`youtube-hashtag` fixture); like upstream,
a feed redirected to the home page is an error. `youtube:clip` ports `YoutubeClipIE`:
a `/clip/ID` page's video is extracted with the clip's ID, extractor,
`media_type: clip`, loop section (`section_start`/`section_end`, so the
duration is the section's and the download goes through FFmpeg) and an
https-first format order (`youtube-clip` fixture).

The other `_extract_entries` renderers are ported too, with continuation pages
dispatched by `_entries`' renderer table: a home/featured tab's shelves
(`_shelf_entries`: the shelf's own URL as an extractor-less `url` entry, then
its grid or expanded contents; horizontal lists are skipped like upstream's),
grid channels and playlists, community posts (`_post_thread_entries`: the
attached video and playlist, then the YouTube video links in the post text,
also on `backstagePostThreadRenderer` continuation pages; an
`itemSectionContinuation` page ends the entries, as upstream's lookup of its
missing `contents` does), YouTube Music list items
(`_music_reponsive_list_entry`), hashtag tiles and report history rows. With
`--flat-playlist`, an extractor-less entry shows the playlist's extractor, and
entries without an ID get `get_temp_id`'s. `tests/fixtures/sites/youtube-featured/`,
`youtube-posts/` and `youtube-tab-renderers/` compare these with pinned
upstream output.

## YouTube comments

With `--write-comments`, a YouTube watch page's comments are fetched after
extraction, before the `after_filter` metadata parsers, like upstream's
`_get_comments` (`youtube-comments` fixture):
- **Section:** the `comment-item-section` continuation, or upstream's
  generated continuation when the page has none; a `messageRenderer` before any
  comment warns `ID: Youtube said: MESSAGE` once and leaves `comments` and
  `comment_count` null.
- **Header:** `Downloading ~N comments` from the header's count and
  `Sorting comments by newest first` (or `top`) through the sort menu's
  continuation.
- **Pages:** `Downloading comment API JSON page N (COUNT/~TOTAL)` and indented
  reply thread pages, with upstream's retries for incomplete responses.
- **Comments:** both the `commentViewModel` entity format (from
  `frameworkUpdates` mutations) and the older `commentRenderer`, with
  upstream's fields and key order, `parent`, pinned/hearted/uploader/verified
  flags and the timestamp resolved from the relative time text.
- **Limits:** `youtube:max_comments=MAX,PARENTS,REPLIES,PER_THREAD` and
  `youtube:comment_sort=top|new`, ending with `Extracted N comments`.

Saved upstream output zeroes the `timestamp` that follows a comment's
`_time_text`, since it is relative to the capture time.

## YouTube search

`youtube:search` ports `YoutubeSearchIE` for `ytsearch:QUERY` (one result),
`ytsearchN:QUERY` and `ytsearchall:QUERY`, with upstream's videos-only
`params`; `youtube:search_url` ports `YoutubeSearchURLIE` for
`youtube.com/results?search_query=…` (or `q=`) URLs and their `sp` filter.
`youtube:music:search_url` ports `YoutubeMusicSearchURLIE` for
`music.youtube.com/search?q=…` URLs: the `web_music` client (`Downloading web
music client config` from `music.youtube.com`), a section from `sp` (named
when it is one of upstream's `_SECTIONS`) or from a `#section` fragment, and
the playlist titled `QUERY - SECTION`; its `musicShelfRenderer` results are
`musicResponsiveListItemRenderer` song, playlist and browse `url` entries.
Like upstream's generators, all three fetch lazily: `Downloading playlist`
comes first, then the client config (not with `youtubetab:skip=webpage`) and search API pages (`query "QUERY"
page N: Downloading API JSON`) with the previous page's visitor data and
continuation, stopping once the requested number of results (or
`--playlist-items`) is reached, as upstream's `islice` does. Results are
`_extract_entries`' video and channel `url` entries in a playlist titled by
the query. `tests/fixtures/sites/youtube-search/` and
`youtube-music-search/` are compared with pinned upstream output. The URL
parser percent-encodes non-ASCII query text, which is decoded again; a
non-ASCII query's `webpage_url` stays percent-encoded.
`--list-extractor-descriptions` draws the search example at random, as
upstream does. Pinned upstream has no other YouTube search prefix (no
`ytsearchdate`); other sites' search prefixes and `--default-search` are not
supported.

`youtube:notif` ports `YoutubeNotificationsIE` (`:ytnotif`, account cookies
required): the web client config, then lazily the
`notification/get_notification_menu` pages, as a `notifications` playlist of
video and community-post `url` entries (title, channel, thumbnails, and the
`youtubetab:approximate_date` timestamp). Without a client config it
fails with upstream's authcheck error (a warning with `youtubetab:skip=authcheck`). `tests/fixtures/sites/youtube-notifications/`
is compared with pinned upstream output.

## YouTube HLS discovery

Playable matching single-video initial responses and the anonymous WEB API
fallback may supply `streamingData.hlsManifestUrl`. The pinned WEB GVS policy makes
HLS tokens optional; it requires a token for DASH. DASH URLs are excluded before
preparation, solving and requests. This adds no token acquisition, authentication,
client selection or live/post-live support.

Direct URL query-n/signature and HLS `/n/{token}/` path challenges share one
complete offline solver batch before media probes or manifest requests. An explicit
Deno runtime is required for eligible challenged sources; without one those
sources are skipped. Challenge-free HLS never inspects runtime/player configuration.
Path tokens retain their raw spelling for upstream EJS matching; solved values
are encoded as a single URL segment. Strict URL preparation rejects ambiguous
or duplicate path markers, invalid escapes, fragments, query-n manifests and
legacy encrypted `/s/` paths. The latter two are unsupported native cases; the
pinned upstream manifest branch does not solve them.

HLS formats follow eligible direct formats, with native `hls-N` / `hls-audio-N`
IDs (upstream names YouTube HLS formats by itag; not yet ported) and
`youtube-hls:` audio group namespaces. Direct numeric itags are unchanged.
A media-only playlist forms `hls-0`; master formats retain dimensions, bandwidth,
language and rendition metadata from the existing native HLS parser. Selection
uses the shared format sorting (with YouTube's `_format_sort_fields`) and pair rules.
Representative fragments are retained even before library selection. Formats are
labeled `mp4` like upstream. No upstream format-ID parity is implied for YouTube HLS.

Only eligible direct formats are probed as media. HLS acquisition reads the master
and its media playlists; extraction/JSON/simulation never fetch initialization,
segments or AES keys. Downloads use the canonical watch-page referrer and existing
fragment transport. Every retained manifest must parse within the supported VOD
contract; an unsupported or failing retained manifest fails extraction even when
another direct format could work. Existing extraction retries can restart the
whole page/API/manifest sequence after a transient acquisition failure.

Offline source comparison:

```sh
python3 tools/compare_youtube_hls.py --upstream /path/to/yt-dlp
```

The actual pinned YoutubeIE and master parser run with network blocked and no
GVS token or missing_pot override. Plain and path-n saved responses compare
metadata and URLs against the same expected JSON consumed by Rust tests. A fixed
typed n solution checks path replacement; it does not execute EJS. Container,
fragment, transport and selection behavior have separate native tests.

See [the source slice](../upstream/notes/youtube-hls.md) for source applicability,
fixture provenance and offline comparison exclusions. Saved fixtures establish
this bounded extraction behavior, not current live-site availability.


## Format selection

`-f` is a port of upstream `build_format_selector` (`src/format_select.rs`):
the spec is tokenized like Python's `tokenize` with upstream's digit-prefix
workaround (whitespace between operands vanishes, `#` starts a comment,
operators other than `/ + , ( ) [` join the surrounding names, unbalanced
brackets or quotes give `Missing closing/opening brackets or parenthesis`),
then parsed by `_parse_format_selection` with upstream's `Invalid format
specification: NOTE` messages and caret (whose column, like upstream's,
counts the 32-letter prefix before each digit run). Filters are compiled at
startup by `_build_format_filter`: numeric `< <= > >= = !=` comparisons with
`?`, `parse_filesize` units (`[abr>100k]` is upstream's 102400) and
`Invalid value`/`Invalid filter specification` errors, and string `= ^= $= *=
~=` operators with `!` negation and quoting; any field name is accepted and
fields a format lacks match only with `?`. Evaluation follows
`_build_selector_function`: `best`/`worst`/`b`/`w` with `video`/`audio`/`v`/`a`,
`*` and `.N`, the audio/video/storyboard extension atoms (with the
`incomplete_formats` and no-merged-format fallbacks), literal IDs, `all`,
`mergeall`, `/` fallbacks, `+` merges over every pair of results, `( )` groups
whose filters narrow their context, and `,` lists. Streams are classified by
`vcodec`/`acodec` (`none`, or unknown counting as present) like upstream.
`_merge` keeps the first video and the first audio stream (including
upstream's skip after a dropped stream), builds `format_id`, `format`,
`protocol`, `tbr` and the single video's/audio's fields, and picks the
container with `get_compatible_ext` (`--merge-output-format`, or `webm`/`mkv`
with `--prefer-free-formats`). Python type errors from mistyped comparisons
(`'int' object has no attribute 'startswith'`, ...) are reported as errors.
`--check-formats` tests candidates lazily in selection order. A result without
formats is selected over itself, like `_get_formats`. Startup errors print
`ERROR: MESSAGE` where upstream raises (a traceback), and `~=` uses the Rust
regex dialect. Every saved spec of `tools/capture_format_selection.py` (1284
spec/format-list cases including errors) matches pinned upstream.

Several selected formats (`,`, `all`) are downloaded one after another, each
as its own video like upstream's `process_info` per format.

## Available format inventory

`-F` / `--list-formats` alone extracts all eligible alternatives and prints a
format table for each video. It bypasses library selection, so separate-only HLS,
DASH or supported direct adaptive presentations can be inspected without a merger.
A valid `-f` argument does not filter listing-only output or require that its ID
exists; its supported grammar is still validated before any requests.

As in pinned upstream control flow, adding `-j` or `-s` retains normal selection.
`-F -j` prints the complete table followed by selected JSON; this combined stdout
is human text plus JSON, rather than plain NDJSON. `-F -s` selects and prints the
table without downloading. Existing native selection/default/group policies apply,
so a separate-only combined mode may require an explicit pair selector or merger
configuration. With `--no-simulate`, listing keeps normal selection and then
downloads; upstream listing-style toggles remain unsupported.

Tables port upstream `render_formats_table` over the serialized info dict, in
sorted format order: `ID EXT RESOLUTION FPS HDR CH | FILESIZE TBR PROTO | VCODEC
VBR ACODEC ABR ASR MORE INFO`, with `format_field`/`round` (ties to even)
formatting, `format_bytes`/`format_decimal_suffix`, `≈`/`~` approximate sizes
(including `filesize_from_tbr`), `shorten_protocol_name`, `simplified_codec`,
`hide_empty` columns and the uncolored `|`/`-` delimiters (upstream uses them
whenever colors are off). An info dict without formats renders itself as one row.
Saved HLS, DASH and HTML5 fixtures render byte-identical tables and old-layout
notes. Unicode alignment counts characters like upstream; terminal controls and
line/bidirectional controls in text fields are visibly escaped (a native safety
difference). Each entry's table is printed before its format selection, so a
failed selection follows its table like upstream.

Extraction uses normal probes, bounded manifest reads, eligibility and optional
challenge solving. Listing does not fetch segment bodies, initialization resources
or AES keys, inspect FFmpeg, render output paths or create media files. Independent
`--extractor-retries` still restarts the full extraction after transient failures;
format/rendering failures are not extraction retries. Multiple inputs retain the
usual error continuation and nonzero final status on any failure.

The library exposes `extract_result_unselected` and
`extract_result_unselected_with_options` for inspection without selection. Their
metadata contains the extractor's representative fields; call selection before
download. Existing extraction methods still select every entry. The pure
`output::render_formats` has no acquisition or selection effects, and
`validate_format_selector` validates syntax without checking format availability.
See [the source slice](../upstream/notes/list-formats.md) for reference probes and
rendering comparison limits.


## Download archives

`--download-archive FILE` loads a regular UTF-8 archive before processing inputs.
Missing files are empty and are created only when a completed download is recorded.
Each record is lowercase `extractor_key`, one space and the case-sensitive `id`.
Both metadata fields must be nonempty, with no controls or surrounding whitespace;
Unicode and internal spaces are preserved. Loading strips surrounding whitespace
and recognizes LF, CRLF and lone CR. Existing unrelated records are preserved.
No URL disambiguation is added: filename-derived generic IDs can collide across
URLs, so archive identity is only as specific as the extractor's key and ID.

Filtering follows complete extraction, including normal page/API/manifest requests,
challenge solving and media probes, then precedes format selection. A recorded
entry can skip even when its requested format is unavailable. Extraction errors
still fail; no early URL-ID skip is implemented. Remaining playlist entries must
all select successfully before output/download begins. Metadata, listing and
simulation filter recorded entries too, but never append or create archives.
Malformed selectors still fail syntax preflight even if every entry is recorded.

Record only after successful final media publication, including merge success.
Extraction, selection, template, transport, merge, cancellation and existing-output
errors do not record. Duplicate inputs in one invocation skip after the first
successful record. If the media succeeds but recording fails, keep the completed
media, report the archive error, continue later entries/inputs and return failure.
Parent directories are never created; existing regular-file ancestors are rejected
consistently even when an OS reports the descendant as missing. A genuinely missing
archive parent fails on the first
record after media publication. Invalid UTF-8/nonregular existing paths fail before
any input request. Archives have no artificial size cap and are trusted local input.

Readers take a shared standard-library file lock; recording takes an exclusive
lock with a five-second deadline. Each append rereads the locked file and avoids
an existing ID, preserving previous bytes and inserting an LF separator if the
last record lacks CR/LF. Writes use LF, synchronize before success, and on write
or synchronization failure attempt to truncate only this append under the lock.
Rollback failure is explicitly reported. Crashes and noncooperating writers are
outside this transaction guarantee; compatibility with upstream Python locks on
every filesystem is not established.

Membership is cached at load and refreshed during this instance's recording.
Concurrent invocations can download the same media; only cooperating archive
updates are locked/deduplicated, not the entire download. Filesystem locking errors
are explicit. Upstream's blocking lock, blind append and missing-newline behavior
are intentionally not reproduced.

The library exposes `DownloadArchive::{load, contains, record}` and `archive_id`;
callers explicitly filter unselected results, call `YoutubeDl::select_result`,
download, then record after success. Extraction/download APIs do not implicitly
manage an archive. `ie_key`/URL-derived identity fallback, legacy alternate IDs,
`--force-write-archive`, `--no-download-archive`, in-memory set CLI input and general
archive option parity remain unsupported. The shared fixture compares twelve
identities/file-and-set membership checks, logical recording and four upstream
control flows. Native tests separately cover failure timing, concurrency and
append rollback. See [the source slice](../upstream/notes/download-archive.md).


## Playlist item selection

`-I` / `--playlist-items ITEM_SPEC` selects entries of fully extracted HTML5
playlists. Comma-separated segments preserve requested order and discard repeated
original indices after their first occurrence. `--playlist-reverse` reverses that
final order. Single video results are unaffected by valid playlist options.

Indices are one-based and ranges have inclusive endpoints. A negative index counts
from the end (`-1` is last); zero refers to no entry. `START:STOP:STEP` accepts
omitted bounds and signed nonzero steps: `::-1` reverses all entries, `1::2` takes
every other entry and `-5::2` starts five from the end. The endpoint may be `inf`
or `infinite`. Pinned upstream interprets `infinite` as an omitted endpoint,
so `:infinite:-1` reverses the list while `:inf:-1` selects nothing. The legacy `START-STOP` delimiter also supports omitted/negative
bounds and a colon step. Out-of-range positions are skipped while preserving the
range's phase, rather than resetting its first position to an in-range boundary.
Descending endpoints require a negative step; unavailable and empty selections
succeed with no output or download.

Without `-I`, `--playlist-start` (default 1) and `--playlist-end` select an inclusive
range; omitted end or end -1 means through the end. Start must be positive;
end must be at least start unless -1. These legacy bounds are validated even when
`-I` overrides them, and an override warning is emitted for effective nondefaults.
Empty item strings, empty comma segments, whitespace and zero steps are errors.
Native specs are bounded to 16 KiB and 1024 segments, with signed 64-bit numbers;
unsupported oversized syntax fails before HTTP instead of iterating huge ranges.
Format selector syntax is also checked before requests, including when a playlist
selection would be empty. Full Unicode numeric syntax is outside this contract.

Selection runs after complete extraction/probes and before archive filtering and
format selection. It cannot avoid requests, challenges, unsupported extraction
errors or media probe failures in excluded entries. Excluded entries need not
satisfy a requested format, while all retained entries must select successfully
before any output or downloads. No lazy/paged/nested or site playlist result is
introduced. Multiple inputs apply the same selection independently and retain
normal failure continuation. Listing, JSON, simulation and downloads use the same
selected order; empty selections do not create output or archive files.

Entry JSON adds `playlist_index` (original position), `playlist_count` (original
entry count), `playlist_autonumber` (selected order) and `n_entries` (selected
count). All are one-based/count integers where applicable and omitted on standalone
videos. IDs/titles keep original extractor numbering. Metadata is assigned before
archive filtering, so archive skips leave gaps in selected autonumbers and do not
change selected counts. This follows the materialized upstream ordering; no full
playlist metadata or lazy evaluation parity is claimed. Represented playlist fields
can be used in the typed templates described below.

Library callers explicitly use `PlaylistSelection::new(items, start, end, reverse)`
then `apply(&mut ExtractResult)` before format selection/download. `indices(len)`
returns zero-based positions in selected order. Library extraction APIs do not
implicitly apply CLI options or annotate playlist indices. Reapplying selection
treats the current entry vector as its input, so select once before archive filters.
Source evidence and shared comparison fixtures are recorded in
[the source slice](../upstream/notes/playlist-items.md).


## Typed output templates

`-o` accepts the represented root string fields `id`, `title`, `ext`, `extractor`,
`extractor_key`, `format_id`, `url` and `webpage_url`; integer fields `width`,
`height`, `filesize`, `playlist_index`, `playlist_autonumber`, `playlist_count` and
`n_entries`; and boolean `direct` (plus the other typed root fields listed in
`FIELDS`, including `__last_playlist_index`). The info dict also carries
upstream's derived keys: `original_url` (the input URL of the extracted result),
`webpage_url_basename` and `webpage_url_domain` (`add_default_extra_info`),
`display_id` (the ID unless set), playlist entries' `playlist`, `playlist_id`,
`playlist_title`, `playlist_webpage_url` and the playlist's basename/domain,
HTML5 entries' `_old_archive_ids` (`generic ID[-N]`, which download-archive checks
also match), keys of loaded info JSON this port does not model, and each format's
`http_headers` (upstream `_calc_headers`: the global headers in upstream order with
title-cased names, then the format's own headers, such as an HTML5 source's page
`Referer`; a single selected format's appear at the top, merges have none) and
`cookies` field (the jar's cookies for the format URL as `NAME=VALUE; Domain=…;
Path=…[; Secure][; Expires=…]`, values quoted like `SimpleCookie`, in jar order).
Formats also keep loaded keys this port does not model (for example
`stretched_ratio`), which a single selected format copies to the top. Every other field and any traversal use upstream `_traverse_infodict`
over the serialized info dict (`traverse_obj(..., traverse_string=True)`): dotted
keys, integer indices (negative from the end), Python slices such as
`%(title.0:20)s`, `%(formats.1:3.format_id)s` or `%(id.::-1)s`, `:` for every
item, and `{a,b.c}` dict subsets; branching results are lists with missing values
dropped. Unknown fields are missing values (`NA` or the `|default`), like
upstream. Lists and dicts render as Python `str()`/`repr()` text, `l` joins list
items with `, ` (newlines with `#`) and `j` emits Python `json.dumps` (indented
with `#`), sanitized in filenames. The serialized dict holds the represented
fields only (with this port's key order inside formats), so fields upstream
extractors would add are missing. As upstream's `prepare_outtmpl` does, every
template's dict also gets `epoch` (the current time) unless it has one.

Keys follow upstream's `INTERNAL_FORMAT_RE` (the regex crate's leftmost-first
captures, with its `(?<!\\),` lookbehind applied by hand): an optional `-`
negation, the field, `+`/`-`/`*` maths with numbers or other fields
(`%(epoch-3600)d`, `%(duration*1000)d`; floats, and a missing operand makes the
value missing), `>FORMAT` datetime formatting of timestamps or `YYYYMMDD`
strings (`%(timestamp>%Y-%m-%d)s`; `\,` escapes a comma), `,` alternates,
`&REPLACEMENT` (Python `str.format` with `{}`/`{0}`, `!r`/`!s`/`!a` and `{{`/`}}`;
a `ValueError` such as `{name}` makes the value missing with the global
placeholder) and `|DEFAULT`. A key that does not match is a missing value, like
upstream. Native boundaries: datetime formatting ports the C-locale directives
`aAbBcCdDeFfgGhHIjklmMnprRsStTuUVwWxXyYzZ%` and `%:z` (timestamps are UTC, so
`%Z` is `UTC`; other directives and `%s` of a `YYYYMMDD` date, which needs the
local time zone, are errors), replacement format specs (`{:>5}`) are errors, and
an operator without an operand is an error where upstream raises `IndexError`.

Named substitutions use `%(field)FORMAT` with upstream's conversions:
`d`/`i`/`u` (truncating), `f`/`F`, `e`/`E`, `g`/`G` (Python's rules), `s`, `r`
(Python `repr`), `a` (`ascii`), `c` (the first character, or `%s` of a falsy
value), and upstream's `l` (joined list items or dict keys), `j` (JSON), `h`
(HTML escaping), `q` (shell quoting; `#` quotes each list item), `B` (width and
precision in UTF-8 bytes, decoded with `errors='ignore'`), `U` (NFC; `+` for
NFKC, `#` for NFD; non-strings are an error where upstream raises
`TypeError`), `D` (`format_decimal_suffix`: `1.79G`, `#` for powers of 1024, a
negative value renders `None`) and `S` (`sanitize_filename`, `#` restricted).
`o`/`x`/`X` fail as upstream does (it passes a float: `%x requires an integer,
not float`). Formats support Python-style `#`, `0`, `-`, `+` and space flags,
ASCII minimum width, `.PRECISION` and one inert `h`/`l`/`L` length modifier. Width
is a minimum, string precision counts Unicode characters, integer precision
pads digits and floating precision defaults to six. Nonfinite integer
conversion is an explicit error. Naked precision dots, unknown conversions,
raw percent escapes and malformed named tokens fail; only `%%` escapes a percent.
These strict syntax limits differ from upstream's more permissive escaping.

`%(width,height|unknown)s` picks the first available known field. Empty strings
are unavailable. Only an unavailable value advances to the next alternative;
a chosen string that fails numeric casting uses the default instead. Explicit
`|TEXT` overrides `--output-na-placeholder` (default `NA`); either can be empty.
Unavailable and failed numeric conversions discard *all* formatting flags,
width and precision, so `%(width|0)04d` with missing width renders `0`, not `0000`.
Alternates after an available value are not evaluated.
Default text is literal, sanitized and never recursively evaluated.

For numeric conversions, pinned upstream first casts values through a float.
The native renderer follows that behavior, including rounding integer metadata
above 2^53 before `d/i/u`, numeric ASCII string casts, surrounding whitespace,
valid between-digit underscores and fixed-point rounding. A plain `s` on integer
metadata preserves its exact decimal value; booleans render `True`/`False` for
`s` and 1/0 numerically. Unicode numeric strings and full Python conversion
syntax are not claimed. No metadata values are invented for missing fields.

An exact bare `%(playlist_index)s` automatically zero-pads to the digit count of
the highest selected original index. Bare `%(playlist_autonumber)s` uses the
selected entry count. This context is assigned before archive filtering, so
archive skips do not shrink padding. Explicit formats (`03d`, `0s`, `hs`, `.0s`)
do not use this compatibility rule. Original playlist count is not a substitute
for selected maximum index. Library callers can apply `PlaylistSelection` before
rendering; its internal maximum-index context is accepted as
`__last_playlist_index` on input but omitted from JSON output. Explicit widths
avoid depending on that context after a JSON round trip.

String values/defaults use upstream `sanitize_filename` (default `is_id`; with
`--compat-options filename-sanitization`, youtube-dl's rules with `is_id` from
the field name) before
width or precision. By default `"*:<>?|` become full-width look-alikes, `/` and
`\` become `⧸`/`⧹`, newlines become spaces, other controls are removed and
`H:MM:SS`-style timestamps use `_`. `--restrict-filenames` applies NFKC, accent
transliteration, ASCII-only `_` substitutes with upstream collapsing/stripping
and removes spaces from the default template (`%(title)s-[%(id)s].%(ext)s`).
`--no-windows-filenames` (outside Windows) only replaces `/` and removes NUL.
`--windows-filenames` additionally applies upstream's forced non-Windows
`sanitize_path` to the whole rendered path (drop `.`/empty parts, fold `..`
lexically, `#` for reserved characters and a trailing space/dot).
`--trim-filenames`/`--trim-file-names LENGTH` slices the rendered text before
its last two dot-separated parts like Python `[:LENGTH]` (negative values cut
from the end). Native containment guard: a path component that is `.`, `..` or
otherwise dot/space-only and is not literally present at the same position in
the template becomes `_`, so metadata never traverses directories (upstream can
produce `../x.mp4`). Only literal template text can introduce directories.
Neither metadata nor defaults trigger environment, home-directory or template
expansion; the literal template and `--paths` values do (`~`, `$NAME`, `${NAME}`). On Windows hosts upstream's Windows `sanitize_path` is not ported.
See [filename sanitization](../upstream/notes/filename-sanitization.md).

Specs are limited to 16 KiB and 256 percent tokens, widths/precisions to 4096,
and selected text/default input and final expanded output to 64 KiB each. A
selected oversized string fails even if precision could truncate it. Bounds are
explicit errors before large formatting allocations; host filesystem filename
limits can still reject a rendered path. Empty final paths and stdout `-` remain
unsupported. Existing media follows the overwrite options below, failed renders
never record archives, and later entries/inputs still continue.

Rendering occurs only on actual download paths after extraction and selection,
or for `--print` lines that refer to `filename`. Other metadata, simulation and
listing paths leave output templates inert, including invalid ones; this
existing native timing is not general upstream option-normalization parity. Library `output::render_template` uses default options;
`render_template_with_options` accepts `TemplateOptions` with `na_placeholder`,
`filename_mode`, `windows_filenames` and `trim_file_name`.
Environment expansion of metadata and full filename/platform compatibility
remain open.

The shared 213-case comparison (including traversal, slices, maths, dates,
replacements, every conversion and `filename-sanitization`) uses actual pinned
`evaluate_outtmpl` in filename sanitization mode on portable fields
(`tools/compare_templates.py`). Native safety/resource cases are separate;
unsafe-character output is not normalized to hide policy differences. Three
source-only cases record unsupported behavior. See
[the source slice](../upstream/notes/typed-output-templates.md).


## Print templates

`-O`/`--print [WHEN:]TEMPLATE` prints at upstream's stages for each selected
video, including every selected HTML5 playlist entry. A WHEN prefix is
recognized case-insensitively only when it names an upstream stage before the
first `:`. `video` (the default) prints after `video`-stage `--exec` commands;
`before_dl` after sidecars and info JSON (also with `--skip-download`);
`post_process` after the download but before merging or postprocessing;
`after_move` with the absolute final path (also for an already-downloaded file);
and `after_video` after the archive record. `filepath` (from `post_process` on)
and `_filename` (the prepared media path, through `after_move`) are available
fields. `pre_process`, `after_filter` and `playlist` are rejected before any
request instead of printing at the wrong time. Without a recognized prefix the
whole value is the template.

A value fully matching upstream's shorthand pattern `([\w.:,]|-\d)+` is a
field list: each comma-separated field prints `%(field)s` on its own line, and a
trailing `=` prints `field = %(field)#j`. Dictionary shorthand (`{id,title}`) is
rejected. Other values are output templates using the typed template engine
above, with these print-mode differences from filenames: values and defaults
are not sanitized, empty strings remain available (so they do not advance
alternates), empty output and `-` are valid lines, and a `j` conversion prints
Python `json.dumps` text (`ensure_ascii` unless the `+` flag is given; `#`
indentation is inert for scalar fields) formatted with the remaining `%s` flags.
`filename` (the rendered `-o` path) and `urls` (the selected URL, or each
`requested_formats` URL on its own line) are added as upstream `_forceprint`
does. Unknown fields print `NA` and traversal works as in filenames; shorthand
follows upstream `format_tmpl` (`-O id.0`, `-O "{id,title}"` as `%(.{id,title})j`,
`-O field=` as `field = %(field)#j`). `_forceprint`'s `formats_table`
(the `-F` table, legacy with `--list-formats-old`), `thumbnails_table`,
`subtitles_table` and `automatic_captions_table` are available too.

Hidden `-e`/`--get-title`, `--get-id`, `-g`/`--get-url` and `--get-filename`
print after all `--print` lines in upstream order (title, ID, URLs, filename),
regardless of argument order. `--get-thumbnail`, `--get-description`,
`--get-duration` and `--get-format` are rejected because those fields are not
represented. With `-j`, printed lines precede each JSON object.

Printing and `--get-*` imply quiet simulation: no media, subtitle, info JSON or
archive writes, unless
`--no-simulate` is given, which prints and then downloads. Like upstream's
`print_only`, a `--print` at `before_dl` or a later stage does not imply
simulation (it still implies quiet). Listing options still print their tables and then keep
normal selection, as with `-j`/`-s`. All lines for a video are rendered before
any is written, so a render failure prints nothing for that video, reports an
error and continues with later inputs. A failed `-o` render fails only videos
whose print lines use `filename`.

`--print-to-file [WHEN:]TEMPLATE FILE` (repeatable) accepts the same video-stage
templates; FILE is a sanitized output template. Both are validated at startup.
For each processed video, every destination and line is rendered first, then
lines are appended (UTF-8, parent directories created) before any download.
Unlike `--print` it does not imply simulation; the `[info] Writing 'TEMPLATE'
to: FILE` notice (Python `repr`) is a screen line, suppressed while quiet.

A 19-case fixture compares exact printed lines with actual pinned upstream
`parseOpts` and `__forced_printings`, plus five source-only rejection cases.
See [the source slice](../upstream/notes/print-templates.md).

## HTML5 subtitles and sidecars

HTML5 media entries discover direct child `<track>` elements after validating the
media alternatives. Accept absent/empty `kind` or exact lowercase `subtitles` and
`captions`; ignore other kinds. Nonempty `src` is stripped and resolved against
the effective page/base URL with HTML entities decoded. Only HTTP(S) resources
are supported; an unsupported eligible track URL fails extraction even if another
language would be selected. No caption request or probe occurs during extraction.
Track-only elements without a supported media source remain unsupported.

Group formats by `srclang or lang or label`, preserving declaration order,
attribute case/whitespace and a final empty label. Missing all language attributes
remains unknown rather than inventing `und`. Metadata is a language-keyed
`subtitles` object with ordered URL/ext arrays. Python's unknown-language JSON key
is the string `null`; JSON round trips cannot distinguish it from a literal tag
named `null`. The library map retains `Option<String>` keys before serialization.
Empty subtitle objects are omitted from native metadata.

As in pinned postprocessing, protocol-relative track URLs acquire `http:` even
with an HTTPS page base, and `httpss://` is repaired to HTTPS. Extension completion
uses the URL suffix, lowercased, with `unknown_video` for unknown suffixes. It
retains source query/fragment and trailing-slash quirks; MIME hints do not infer
extensions and no WebVTT/SRT payload is parsed. This does not add subtitle
acquisition from HLS/DASH or other sites, embedded streams, conversion or embedding.
The separate YouTube caption subset is described below. Native direct-child DOM ownership and malformed
HTML behavior differ from upstream regex parsing.

Without explicit language options, writing prefers exact `en`, then the first
case-sensitive `en` prefix, then the first declared language. An unknown-language
entry encountered during the default prefix search produces an explicit error,
matching the source's unsupported path without propagating a Python exception.
An exact `en` shortcut can still succeed. `all` preserves unknown entries in
requested metadata; actual sidecar planning rejects unknown languages before I/O.
Listing and ordinary metadata remain available without writing or guessing a tag.

Explicit language flags append in command-line order. Trim comma-separated
tokens; literal matches are case-insensitive, `all` expands declaration order,
exclusions remove earlier matches and re-additions append again. Missing literals
select nothing without a warning. General regex syntax is rejected before HTTP;
simple Unicode lowercase matching is not full Python regex case-fold parity.
Option strings are bounded to 16 KiB and 1024 entries, with controls/empty language
tokens rejected. Format preferences are case-sensitive and slash-separated;
`best` chooses the last declaration. Choose the last matching extension for the
first available preference, or last available with a warning. Empty format queries
are valid fallback requests. Requested metadata is exposed with `--write-subs`
even in JSON/simulation, without caption downloads.

`--list-subs` renders upstream's subtitle table of declared languages/extensions,
with no URLs or resource inspection; text fields escape controls and
line/bidirectional controls. List-only mode bypasses media format availability;
adding `-j` or `-s` retains selection.
Without `--no-simulate`, listing, JSON and simulation never write sidecars or render output paths.
Actual downloads plan sanitized sidecars before I/O, replacing a final matching
media extension with `.LANG.EXT`, or appending that suffix otherwise. Language and
extension components use native filename sanitation. An ASCII case-insensitive
collision is an explicit error before any sidecar I/O, even on case-sensitive
filesystems. Unknown languages fail planning. Literal template directories remain
user-controlled; metadata cannot add directories. This is not upstream unsafe
filename/platform byte parity.

Existing regular sidecars are deleted and fetched again by default and with
`--force-overwrites`, and reused with `-w`. Nonregular or dangling sidecar
entries fail before fetching them. A nonregular or dangling media destination
fails before sidecar I/O unless media is skipped; an existing regular media file
follows the [overwrite options](#overwrite-options). Complete
sidecars are published before media. A sidecar failure prevents that entry's media
and archive record; earlier complete sidecars remain. Later entries/inputs continue.
Media/merge failure also leaves completed sidecars without an archive record.
These native preflight/partial-publication rules are explicit, rather than a claim
of identical upstream error timing or a transaction spanning all outputs.

Sidecars share native whole-file HTTP transport, page referrers, identity requests,
transient retries/Retry-After, guarded in-invocation range recovery, cancellation,
same-directory temporary cleanup and no-clobber publication. They accept raw
response bytes without a media MIME check, but reject compressed responses because
no caption decompression is implemented. Byte copying does not validate subtitle
syntax or encoding. `--skip-download --write-subs` writes only sidecars and never
records the video archive; plain skip-download creates no outputs. It still needs
normal extraction and format selection. Archive filtering precedes subtitle
selection; successful actual media publication remains the only archive-record
trigger. Native library extraction/download methods do not implicitly write
sidecars.

Library types are `SubtitleFormat`, ordered `SubtitleMap<T>`, `Subtitles`,
`RequestedSubtitles`, `SubtitleOptions` and `SubtitleSelection`. Call `select`,
assign `requested_subtitles`, then `subtitle_paths`; copy selected resources with
`YoutubeDl::download_subtitle_with_options` or `download_subtitle_with_progress`.
The latter has the same cooperative cancellation limits as media transfers.
`output::render_subtitles` is also re-exported at crate root. Shared comparisons
cover seven HTML metadata, 29 selection, five filenames, six option flows and
three source-only cases; payload/timing/cancellation tests use local HTTP fixtures.
See [the source slice](../upstream/notes/html5-subtitles.md).


## YouTube manual and automatic captions

The supported initial/anonymous WEB player response subset reads caption metadata
without fetching caption resources. Manual tracks populate `subtitles`; ASR tracks,
translations and original aliases populate `automatic_captions`. Formats preserve
upstream order (`json3`, `srv1`, `srv2`, `srv3`, `ttml`, `srt`, `vtt`), names and URL
query behavior, including duplicate grouping, blank-value dropping, `fmt` replacement
and `xosf` removal. Formats are server-side URLs; no local conversion is performed.
`vssId` supplies language keys before languageCode. Missing manual language is
skipped, without inventing a tag. Eligible ASR original language fills missing
language metadata on audio-bearing formats, leaving video-only formats untouched.

ASR translations are discovered independently. Translated manual tracks are added
to automatic metadata when `--write-auto-subs` or `--list-subs` is active; the
library equivalent is `YoutubeOptions.include_translated_subtitles`. The upstream
`und` special case and language-suffixed translated manual keys/queries are retained.
`--write-subs` and `--write-auto-subs` independently enable selection. Normal tracks
win shared language keys. Default selection searches normal exact/prefix English,
then all exact/prefix English, then first normal/available language. Existing
literal/all/exclusion and extension preference rules apply to the combined map.
`SubtitleOptions::select_sources` exposes this explicitly to library callers.

`--list-subs` prints automatic captions first when present, then normal subtitles;
a language shows one name when all its formats agree (none when all are unnamed),
otherwise every name with `unknown` for missing ones. Pure
`render_automatic_captions` is exported alongside `render_subtitles`. JSON retains
both maps and optional selected metadata. Listing, JSON and simulation do not
fetch caption bytes. The existing native subtitle transport and path planning
handle actual sidecar writes and `--skip-download` without media archive records.

WEB ordinarily has optional subtitle PO tokens, but exact decoded `exp=xpe` or
`exp=xpv` requires a token, which only `po_token=CLIENT.subs+TOKEN` can give
(no provider acquisition). Without one, earlier tracks are kept and the
remaining tracks of that response are skipped. Like
`_report_pot_subtitles_skipped`, the skip and the final missing-languages
message are warnings when subtitles are wanted (`--write-subs`,
`--write-auto-subs`, `--list-subs`) and (for the per-client skip) the client
is not a default one, debug messages otherwise. Existing URL `pot` values do not substitute for a token;
caption URL deciphering is not implemented. Caption query `n` is preserved
without launching Deno. Media token policies remain separate, including the
WEB direct itag18 exemption.

Native parser limits are 1000 tracks/translations each, 100000 generated formats,
64 MiB cumulative URL/name output, 16 KiB base URLs and 4 KiB names. Malformed
truthy metadata and unsupported URL schemes return explicit errors. Bounds and
strict URL/shape validation do not claim upstream malformed-input parity.
Twenty shared actual-source response cases and five selection cases compare exact
ordered metadata; one missing-language case is also recorded as source-only.
Ten local pipeline tests cover API/watch discovery, token gates, audio language,
malformed URL timing and actual sidecar bytes/referrers/progress without live sites.
This does not establish live-site reliability, token/authentication support,
compressed captions, regex selection, payload parsing, conversion or embedding.
See [the source slice](../upstream/notes/youtube-captions.md) and
[fixture reference](../tests/fixtures/youtube/captions/README.md).


## HTTP request defaults

`--add-headers FIELD:VALUE` (also `--add-header`) is repeatable. Split the first
colon only; trim surrounding value whitespace and retain internal spaces/tabs
and colons. Names are case-insensitive and the last custom value wins. Empty
values remain explicit. CLI field names accept ASCII letters/digits/underscore/
hyphen; arbitrary option abbreviations and Unicode field names are unsupported.
`--user-agent` and `--referer` use their last supplied values and override matching
custom headers regardless of argument order, preserving pinned normalization.
The CLI sends upstream's `std_headers` before custom headers (which override
them): `random_user_agent()`'s Chrome User-Agent (major version 145–151, drawn per
run), `Accept`, `Accept-Language: en-us,en;q=0.5` and `Sec-Fetch-Mode: navigate`.
The library's default User-Agent remains `yt-dlp-rs/VERSION`. FFmpegFD's
`-headers` lines use upstream's title-cased header names.

Global headers are defaults for the shared HTTP client across direct/HTML5/
YouTube pages, anonymous player API calls, media probes/downloads, HLS/DASH
manifests, fragment initialization/segments, AES keys, subtitle sidecars and
retries. Explicit per-request headers win: HTML5/YouTube media and sidecars retain
page Referer; transfers use identity encoding and transport-owned range headers;
player API Origin/client/version headers retain extractor values. Direct inputs
without an extractor Referer use the global Referer. Validation does not make
unsupported authentication or delivery protocols supported.

Explicit global defaults also reach the isolated YouTube player-script client.
The client still validates canonical origin/path and every redirect before
requesting it; custom headers do not authorize page-supplied origins or arbitrary
player redirects. Its identity encoding and bounded MIME/body validation remain
in force. Challenge-free extraction does not construct or launch the runtime.
General HTTP redirects use reqwest's existing ten-redirect policy: ordinary custom
headers persist; Authorization is removed on a change of host/port. This is a
native tested policy, not a claim of full upstream redirect/header parity.
Credentials placed in arbitrary custom header names follow that ordinary policy.

Construct `YoutubeDl::with_options(youtube_options, HttpOptions { headers,
user_agent, referer })` for library use. Existing constructors retain defaults.
Library header pairs allow valid HTTP token names; `HttpOptions::from_cli`
applies the CLI name grammar as well. Validation occurs before client creation,
network requests, archive loading or output writes. Errors omit supplied header
values. Names are bounded to 256 bytes; at most 256 custom entries, 16 KiB per
value and 64 KiB total supplied names/values, including overwritten entries.
CR/LF and other controls except horizontal tabs are rejected before trimming.
Header values are UTF-8 bytes; arbitrary upstream legacy-byte encoding is not
established by the fixtures. Raw Cookie headers remain explicitly unsupported; use the scoped jar described
below. They are never silently forwarded.

The actual pinned option/container/header-merge comparison covers 21 option
configurations, seven merges and seven grammar rejections, with three source-only
inputs and a separate Cookie-removal merge exclusion. Only header names are
lowercased for comparison; values stay exact. Native local tests establish actual
request bytes, precedence, redirects, retries, protocol/sidecar propagation and
preflight rejection independently of that pure source comparison.
See [source applicability](../upstream/notes/http-headers.md) and
[fixture provenance](../tests/fixtures/headers/README.md).


## Scoped HTTP cookies

Every downloader owns a fresh in-memory cookie jar by default. Response
Set-Cookie values update it during pages, redirects, probes, media, fragments,
keys and subtitle transfers. The explicit bounded player client shares the same
jar without relaxing its canonical-origin/path/redirect validation. Domain,
host-only, path, Secure and expiry rules are enforced by native cookie storage.
Secure cookies are sent only over HTTPS, including for loopback requests.
Cookie scope ignores ports, as cookies require; a change of port alone does not
isolate cookies. A different host only receives matching domain cookies. Native
host-only cookies require the exact host even where Python's legacy policy can
send them to subdomains. Canonical domain/path/name identity replaces earlier
host-only/domain variants for the same domain; Python can retain distinct
leading-dot records and send both. No cookie values are added to metadata or diagnostics.

`--cookies FILE` reads Netscape format before requests and saves the updated jar
after all inputs, including metadata/list/simulation and failed requests.
A missing input file starts empty. `--no-cookies` disables file loading/saving;
the last cookies/no-cookies option wins. It does not disable in-memory response
cookies. Cookie persistence is independent of media/archive publication; a save
failure returns a failing exit status while preserving completed media. Raw
Cookie custom headers are still unsupported: use a scoped jar or file instead.
Browser cookie extraction, JSON cookie files and browser encryption stores are
unsupported. Supplying cookies does not implement account-specific YouTube
request contexts, clients, token acquisition or prove live-site reliability.

Use `CookieJar::load`, `save` and `save_new` explicitly in the library. Set
`HttpOptions.cookies` to `Some(Arc<CookieJar>)` to share state across clients, or
use `YoutubeDl::cookie_jar()` to retrieve the client's jar. `save_new` refuses
replacement; the CLI uses it when the file did not exist initially, preventing
cookie persistence from overwriting a newly created media/output or another
process's newly created file. Existing regular cookie files are replaced through
a same-directory temporary file; symlinks and nonregular cookie paths are refused.
Parent directories must exist. Existing-file saves do not merge concurrent
process changes or acquire cross-process locks. Identical and existing canonical cookie/archive paths are rejected before requests. Library calls never save implicitly.

Saved-fixture comparisons invoke actual pinned YoutubeDLCookieJar loading,
request-header construction, response extraction and save/reload without network
or downloads. Five file cases and six response cases compare logical records and
sorted header pairs; duplicate-name path ordering is checked explicitly. Three
source-only cases document legacy host-only leakage, skipped malformed lines and
public-suffix acceptance. Python's standard-library policy is a reference-runtime
dependency, not a separately pinned source tree. HttpOnly marker preservation and
file bytes/order are explicit native policies rather than byte-level save parity.
Files are bounded to 4 MiB, 10000 records and 16 KiB per line. Stored response
cookies have the same count/aggregate bounds; outgoing Cookie headers are capped
at 16 KiB. Invalid or oversized response cookies are ignored with a deduplicated,
value-free warning, available through `CookieJar::take_warnings` for library users.
Malformed existing files fail before requests rather than skipping source lines.
Names use HTTP token syntax; controls, semicolon-bearing values, inconsistent
Netscape domain flags and invalid expiry/path/domain fields are unsupported.
Session expiry is saved as zero; expired records can persist but are never sent.
Headers use longest path then original creation order, retained on replacement.
HttpOnly file prefixes are preserved natively even where source saving loses them.
New and replaced cookie files use private mode 0600 on Unix; Windows permissions
follow platform temporary-file behavior. No public-suffix snapshot or browser
SameSite/partitioned/prefix policy is supplied. The local pipeline tests establish
request propagation and failure/persistence behavior separately. See [source applicability](../upstream/notes/http-cookies.md)
and [fixture provenance](../tests/fixtures/cookies/README.md).


## Explicit proxy routing

`--proxy URL` (last value wins) configures one route for all supported HTTP and
HTTPS requests. `--proxy ''` forces direct connections; the pinned
`__noproxy__` sentinel is also recognized. Explicit routes override ambient proxy
and NO_PROXY settings. Without an option, the existing reqwest environment/system
routing policy is preserved. This does not reproduce all Python platform discovery,
HTTP-to-HTTPS environment fallback, bypass suffix or PAC behavior. The six saved
omitted-option environment cases establish source behavior only; native tests use
isolated subprocess environments for basic inheritance and explicit direct bypass.

Proxy URLs support HTTP, HTTPS, SOCKS4, SOCKS4a, SOCKS5 and SOCKS5h. Missing schemes
and scheme-relative values become HTTP. HTTP/HTTPS default to ports 80/443; SOCKS
defaults to 1080. Pinned aliases map `socks` to SOCKS4 and `socks5` to SOCKS5h:
SOCKS4 resolves destination addresses locally, SOCKS4a and SOCKS5(h) ask the proxy.
Remote-DNS tests use unresolved synthetic hostnames and inspect SOCKS domain
fields, without contacting DNS or live origins. SOCKS4 user-ID credentials are
explicitly unsupported because the current backend cannot send them. SOCKS5
username/password authentication is offered only when both fields are nonempty,
matching source handshake behavior; incomplete credentials use no-auth negotiation.

Percent-encoded UTF-8 URL credentials are decoded for proxy authentication. HTTP
Basic authentication natively preserves user-only and empty fields, and applies
only to proxy requests/CONNECT. This is an explicit native policy: upstream
backends can omit authentication for incomplete or empty fields. Proxy credentials are omitted from configuration errors
and video metadata. The tests separately verify that authenticated CONNECT keeps
Proxy-Authorization outside the origin TLS HTTP stream. HTTPS proxy connections
and HTTPS origin tunnels retain normal certificate verification; custom trusted
roots are used only inside offline TLS fixtures. Proxy authentication does not add
origin login support or disable existing extraction/token eligibility rules.

The shared policy covers page/player API requests, media probes/downloads,
HLS/DASH manifests, keys/initialization/fragments, subtitle sidecars, retries and
redirects. Bounded player-script acquisition uses the same explicit route while
still validating every redirect target before requesting it. An off-origin player
redirect is rejected before another proxy request. Optional FFmpeg operations
remain local stream copies and do not perform network transfers.

Library `HttpOptions.proxy` has the same `None`/empty/URL meanings.
`HttpOptions::validate` validates without DNS/network requests; client construction
validates again. The CLI validates before cookie-file loading/archive/output I/O.
URLs are bounded to 16 KiB; controls, whitespace, backslashes, malformed percent
credentials, non-UTF-8 decoded credentials, zero ports, unsupported schemes,
non-root paths, queries and fragments are explicit errors. SOCKS credential fields
are capped at 255 decoded bytes. Legacy Ytdl-Request-Proxy/Ytdl-Socks-Proxy custom
headers are rejected; per-request and geo-verification proxy features remain open.
Untrusted environment settings use the pre-existing backend handling rather than
this explicit-URL validator. No fallback to a direct connection is configured when
an explicit proxy fails.

The offline pinned source comparison covers 50 source configurations and eight
source-only validation/legacy cases. Native unit tests compare 39 explicit endpoint
projections, reject five SOCKS4 credential cases and retain six omitted-option
cases only as environment-mode markers. Scheme/host/port/decoded credentials and
remote DNS flags are compared without claiming identical raw URL formatting.
Eleven local integration cases cover HTTP/SOCKS routing, credentials, empty fields,
headers/cookies, protocol resources, retries, redirects, direct override and
rejection before requests. Three saved-certificate TLS tests cover verified
CONNECT, HTTPS proxy transport and untrusted-origin rejection. A bounded-player
proxy test preserves redirect gates independently. See
[source applicability](../upstream/notes/http-proxies.md),
[configuration fixtures](../tests/fixtures/proxies/README.md) and
[TLS provenance](../tests/fixtures/proxies/tls/README.md).

## Audio extraction

`-x` / `--extract-audio` enables audio inspection and copy/conversion after a
successful native download or merge. Targets are `best` (default), `mp3`, `aac`,
`m4a`, `opus`, `vorbis`, `flac`, `alac` and `wav`. Ordered slash-separated
`SOURCE>TARGET/TARGET` rules match the source metadata extension; an unmatched
rule skips processing. CLI target names are lowercase. Without `-f` or `-k`,
`-x` selects `bestaudio/best` in sorted format order. With `-k` it retains
the normal default selection, potentially downloading/merging video first.

`--audio-quality` defaults to 5. Finite 0..10 values use the source MP3, Vorbis
and ordinary AAC encoder curves; values above 10 request kbit/s, e.g. `128K`.
The native limit is 1000000 and the option is at most 128 bytes. Opus/FLAC/WAV
have no invented VBR curve. Missing encoders fail. ALAC explicitly uses ALAC;
the pinned source's ALAC option-loss behavior is not reproduced. Format mappings
are bounded to 16 KiB/1024 branches and ASCII word source names.

`--ffmpeg-location PATH` configures the local executable; `--ffprobe-location`
may select a separate executable, otherwise FFprobe is inferred beside FFmpeg.
There is no PATH lookup or installation. Common audio extensions with `best`
(aiff, alac, flac, m4a, mka, mp3, ogg, opus, wav, wma) and unmatched rules skip
tools. Other paths inspect the first audio stream, then copy a matching codec or
encode the requested target. Unknown-codec `best` falls back to MP3. A same-path
codec copy skips FFmpeg. JSON, simulation, format/subtitle listing and
skip-download do not run audio tools.

Local input demuxers cover MP4/M4A/MOV, WebM/Matroska, MP3, AAC, Ogg/Opus, WAV,
FLAC, MPEG-TS, AVI, FLV and AIFF. Other containers fail explicitly. Tools receive
local regular files and a file-only protocol whitelist; remote I/O stays native.
Each probe has a 30-second budget and each conversion ten minutes including
version checks. Output capture, JSON, stderr and process cleanup are bounded.
Missing audio or tool diagnostics fail rather than guessing successful output.

Output replaces only a suffix matching the source extension; otherwise it
appends the target suffix. AAC target retains the pinned `.m4a` naming quirk:
AAC codec copy uses ADTS while AAC encoding uses the M4A container. Vorbis uses
`.ogg`, ALAC uses `.m4a`. Byte-identical FFmpeg output is not claimed.

Like upstream (`os.replace`, `ffmpeg -y`), processor outputs and kept originals replace existing regular files of the same name; `--no-post-overwrites` instead keeps an existing audio-extraction output (`[ExtractAudio] Post-process file FILE exists, skipping`, also keeping the source) and `--post-overwrites` restores the default. With a processing option, `-k` / `--keep-video` retains the source,
using `.orig` before the last suffix if the
conversion keeps its filename. `--no-keep-video` reverses that option. A completed
source is preserved after processing failure; cancellation cleans the private
workspace. If publishing audio fails after the retained source was published,
the source remains and the operation returns an error. Sidecars retain the
original media stem. Archive recording follows successful final audio publication
and rejects aliases of published media paths. There is no persistent processing
resume or transaction across source, sidecars, final audio and archive files.

Library callers use `AudioOptions`, `with_ffprobe` and
`download_audio_with_options` / `download_audio_with_progress`. The returned
`AudioDownload` reports the final path, byte count and optional retained path;
progress includes cooperative `Postprocessing` checkpoints. Callers select
formats before invoking this method, just as for normal downloads.

73 saved source plans, bounded model tests, portable subprocess regressions and
14 local integration tests cover this slice. Optional explicitly configured real
FFmpeg/FFprobe verify all target encoders and actual MP3/WAV/ALAC downloads.
See [source classification](../upstream/notes/audio-extraction.md) and
[reference rules](../tests/fixtures/audio/README.md). General postprocessor
chains, discovery, fixups, timestamp preservation and embedding remain open.


## Video remuxing and conversion

`--remux-video FORMAT` runs local stream copying; `--recode-video FORMAT` runs
local re-encoding. Each accepts an ordered `SOURCE>TARGET/TARGET` mapping.
Targets match the pinned option vocabulary: avi, flv, gif, mkv, mov, mp4, webm,
aac, aiff, alac, flac, m4a, mka, mp3, ogg, opus, vorbis and wav. Active `alac`
and `vorbis` targets fail explicitly: those literal suffixes do not identify an
output muxer in the pinned source path. Use `-x --audio-format alac/vorbis` for
those codecs and their proper container names. Same-extension or unmatched
mappings are successful skips, including these tags, and never inspect tools.
CLI targets are lowercase; source conditions are ASCII words. Library
`VideoOptions::new` case-folds mappings. Mapping limits are 16 KiB/1024 branches.

Both modes map all input streams, discard data streams and ignore unknown stream
types. Remuxing requests `-c copy`. Recoding leaves codec choice to FFmpeg's
container defaults; AVI explicitly requests `libxvid` and the `XVID` video tag.
No video quality/preset option is invented. Outputs receive `+faststart` as in
upstream. Native explicit muxer names replace filename-based FFmpeg inference;
codec/container compatibility and encoder availability still determine success.
Converting a mixed-stream input to an audio-only container can fail, just as a
remux to an incompatible container can fail. These are processing errors.

`--ffmpeg-location PATH` is required only when processing actually runs. Video
processing uses no FFprobe. Inputs are private local files with fixed demuxers
for MP4/M4A/MOV, Matroska/WebM, AVI, FLV, GIF, AAC, AIFF, FLAC, MP3, Ogg/Opus,
WAV, MPEG-TS, MPEG and ASF/WMA/WMV; other input containers fail. The runner uses
file-only protocols, closed stdin, bounded pipes, a ten-minute total execution
budget including version validation, cancellation checkpoints and child cleanup.
Read-only and skip-download modes run no processors.

Recode takes precedence over remux regardless of argument order and emits the
pinned warning. With `-x`, audio extraction runs first, then the video mapping
matches the updated extension. A same-extension video target skips re-encoding.
The existing format-selection policy still applies; the option does not add
formats or imply upstream sorter parity. A selected separate-stream pair merges
before this processing chain.

Final filename substitution uses the current stage's extension, lowercased for
video processing, and only replaces a matching literal suffix. For example,
`movie.WEBM` with metadata extension `WEBM` becomes `movie.WEBM.mp4`. Successful
processing publishes only the final result unless `-k` is enabled. With `-k`,
completed source/intermediate files are also published in processing order;
conflicting earlier stage filenames receive `.orig` before their final suffix.
Returning to an earlier container can therefore keep `out.orig.mp4`,
`out.m4a` and final `out.mp4`. Separate pre-merge input files are not retained.

A failed processor preserves its latest completed input; `-k` also preserves
earlier stages. Cancellation cleans the private workspace. Publication of
retained files and final output is not one transaction: earlier published files
remain after a later collision or failure. Existing destinations are never
overwritten. Existing sidecars retain their original stem; archives record only
successful final publication and cannot alias any published retained/final file.

Library callers use `PostprocessOptions` with optional `AudioOptions` and
`VideoOptions`, then `download_processed_with_options` or
`download_processed_with_progress`. `ProcessedDownload.path` and `.bytes`
describe the final result; `.kept_files` lists all retained media, and
`.kept_video` identifies the original source when retained. The global keep flag
or the contained audio keep flag enables retention. Existing audio-only APIs
continue to use this shared orchestration.

The saved comparison invokes actual pinned option parsing and video processor
runs for 70 shared plans, one source-only Unicode mapping and eight invalid
options. Portable subprocess tests and local HTTP pipeline tests cover actual
execution/publication separately. Optional real FFmpeg/FFprobe verify stream-copy
H.264 retention in MKV and AVI recoding to MPEG-4. This does not establish every
FFmpeg build/codec/container combination. See [source applicability and remaining
scope](../upstream/notes/video-processing.md) and [fixture provenance](../tests/fixtures/video/README.md).


## Subtitle conversion

`--convert-subs FORMAT` accepts one lowercase target: `srt`, `vtt`, `ass`, `lrc`
or `none` (disabled). `--convert-sub` and `--convert-subtitles` are aliases; the
last value wins. The option does not imply `--write-subs` or `--write-auto-subs`.
Conversion runs as upstream's `before_dl` `FFmpegSubtitlesConvertorPP`: after
subtitles, thumbnails, info JSON, links and `--print before_dl:`, before
ThumbnailsConvertor and `--exec before_dl:`, and therefore also with
`--skip-download`. It prints `[SubtitlesConvertor] Converting subtitles` (or
`There aren't any subtitles to convert` without requested subtitles), upstream's
already-in-format, JSON and TTML style-loss lines, then `Deleting original file
FILE (pass -k to keep)` for each converted original (and TTML's SRT intermediate)
unless `-k`. A failure is upstream's deferred `ERROR: Preprocessing: …`: the media
still downloads and the video then fails before postprocessing. JSON, simulation and format/subtitle listing fetch
no subtitle payloads and launch no conversion tools. Existing format/language
selection determines the input; conversion does not request another server format.

Same-extension and literal `json` subtitles skip conversion. The JSON skip keeps
its original bytes and reports that conversion is unavailable. `json3`, YouTube
XML server formats and other unrepresented input formats remain explicit errors
when active; the source's literal `json` branch does not include `json3`.

DFXP/TTML/TT first becomes SRT through a native port of `dfxp2srt`. It covers the
pinned timing grammar, duration fallback, skipped invalid timings/index gaps,
legacy/current/no namespaces, body/div/default and inherited styles, inline
spans/breaks, entities and whitespace. UTF-8/ASCII and UTF-16LE/BE are supported,
including the source's precise single-quoted UTF-8-mislabel workaround. All-invalid
paragraph times can produce a successful empty SRT; absent paragraphs error.
Style conversion is lossy, as upstream warns. Other targets pass this native SRT
to FFmpeg. It does not implement arbitrary TTML standards or additional encodings.

Native TTML limits are 8 MiB input, 16 MiB decoded XML/SRT, 100000 XML nodes,
64 nested elements, 10000 paragraphs and 1000000 style-resolution steps. DTDs,
custom entities, unresolved/cyclic style parents and unbounded/nonfinite numeric
times fail explicitly. Numeric timing syntax is ASCII; Unicode digits are not a
compatibility claim. The saved source's observable cross-cue style behavior and
millisecond truncation are preserved by exact text fixtures.

Ordinary local inputs support SRT, VTT, ASS/SSA and LRC through explicit
`--ffmpeg-location PATH`. No FFprobe or discovery is needed. The fixed local
demuxer and first-subtitle map feed explicit SRT/WebVTT/ASS/LRC muxers, with
source faststart output flags. The native first-subtitle map is deliberate;
these inputs are single subtitle sidecars. Tools use file-only protocols, closed
stdin, bounded output pipes, a 60-second overall version/conversion deadline and
cooperative cancellation. Inputs and produced UTF-8 files are capped at 16 MiB;
output growth is monitored while the child runs. Missing tools, diagnostics,
invalid UTF-8 and empty FFmpeg outputs fail.

Active conversions reject symlink/nonregular sources and an output equal to the
input. An existing converted or intermediate destination is replaced, as FFmpeg's
`-y` does upstream (a symlink there is replaced, not followed). Final names
replace the last suffix, even when it differs from metadata. The CLI preflights
all source/intermediate/converted paths, sanitation collisions and cookie/archive
aliases before subtitle I/O. Existing source-sidecar reuse remains available,
subject to active conversion's regular-file requirement.

Library conversion removes the source unless `keep_original` is set (the CLI
always keeps it and deletes originals itself, like upstream's `run_pp`). TTML to another
format can retain original TTML, intermediate SRT and the final file. Source files
remain after failures; a completed native SRT is also preserved if a later FFmpeg
step fails and its destination is available. Cancellation leaves public sources
untouched and removes private files. Source metadata/version checks detect ordinary
changes before publication/removal; a changed source is retained. Publishing,
source deletion, sidecars, media and archives are not one transaction. A converted
sidecar can remain after a later media failure; no media archive record is written.

Library `SubtitleConversion::new`, `YoutubeDl::convert_subtitle_with_options` and
`convert_subtitle_with_progress` expose this flow. `ConvertedSubtitle` reports the
final path/extension/bytes, whether conversion ran, warnings, retained original
and intermediates. Progress uses Postprocessing and Publishing checkpoints.

15 exact TTML text fixtures (including four upstream tests) and 42 saved option/
postprocessor plans are compared with actual pinned source, alongside one
source-only JSON3 plan and five invalid options. Native tests separately exercise
local files/processes and CLI behavior; real FFmpeg tests verify cues/timing and
representable styles across all four targets. See [source applicability](../upstream/notes/subtitle-conversion.md)
and [fixture provenance](../tests/fixtures/subtitle_conversion/README.md).


## Subtitle embedding

`--embed-subs` enables the last media processor, after audio extraction and video
remux/recode. `--no-embed-subs` disables it; the last flag wins. The supported
containers are exactly lowercase `mp4`, `mov`, `m4a`, `webm`, `mkv` and `mka`.
Other containers or no eligible captions produce a visible skip diagnostic and
publish the downloaded/processed media without launching embedding tools.

Embedding implies manual subtitle acquisition when automatic writing is disabled,
even with an explicit `--no-write-subs`. With `--write-auto-subs`, it does not
imply manual writing; explicit manual writing enables both with existing manual
precedence. Existing language/format selection applies before fetching. Metadata,
listing and simulation launch no tools or caption transfers. Skip-download fetches
and optionally converts selected sidecars but never embeds or removes them.

WebM accepts only VTT inputs and warns once when other caption formats are skipped.
Literal JSON and missing files are skipped with source warnings. MP4 ASS inputs
emit the pinned warning about imperfect embedding. Ordinary local SRT, VTT,
ASS/SSA and LRC are supported; other active inputs (including JSON3 and TTML)
require conversion first or fail explicitly. Converted sidecar paths/extensions
feed embedding. Active conversion drops the original caption name, matching
upstream metadata replacement; same-format/none/JSON skips retain that name.

The plan maps known media streams, discards data, removes existing subtitle
streams and maps each selected sidecar. MP4/MOV/M4A encode subtitles as
`mov_text`; WebM and Matroska copy their supported subtitle codecs. Video/audio
are copied. Language tags use the pinned 188-entry ISO639 first-two-character
lookup, case-sensitive, then fall back to the original tag. For example, `en-US`
becomes `eng`. Nonempty names become both `handler_name` and `title` where the
container represents them. All inputs have fixed local demuxers and file-only
protocols; output muxers and faststart are explicit. FFprobe is unnecessary.

`--ffmpeg-location PATH` is inspected only for an eligible embedding. The version
check and embedding share a ten-minute deadline, bounded process pipes and
cooperative cancellation. Track count is capped at 1000, language at 128 bytes,
name at 4096 bytes and extension at 1..32 ASCII alphanumerics; metadata controls
are rejected. Active sidecars must be regular nonsymlink files, at most 16 MiB
each and 256 MiB total. Private copies isolate FFmpeg from network acquisition.
Repeated canonical paths can create separate tracks but are cleaned up once.

Eligible embedded sidecars are removed only after successful final media
publication, unless explicit `--write-subs` or `-k` keeps them. This includes
preexisting selected sidecars. Upstream's automatic-only quirk is preserved:
`--write-auto-subs --embed-subs` without manual writing still removes eligible
automatic sidecars after success, unless `-k` is enabled. JSON, missing and
WebM-ineligible sidecars are not scheduled for removal. No caption is removed
on embedding failure, cancellation or failed final publication.

Embedding replaces the current private media stage. It does not create an extra
unembedded copy with `-k`; earlier audio/video processor inputs still follow
existing retention rules. No-clobber final publication and archive-after-success
remain in force. Cleanup checks original caption identity before removing it;
a changed caption is retained and returns an error after media publication.
A cleanup failure therefore can leave complete media and sidecars without an
archive record. Publication and cleanup are not one transaction.

Library callers put `SubtitleEmbedding` and local `SubtitleFile` entries in
`PostprocessOptions.embed`. `keep_subtitles` preserves eligible sidecars, as do
the global or contained audio keep flags. `ProcessedDownload.warnings` exposes
source warnings and skip diagnostics; `kept_files` continues to list retained
media, rather than sidecars supplied by the caller.

57 actual source plans, six documented source-only cases, 200 language projections
and a rejected alias are saved. The reference invokes actual option parsing,
embedding and `run_pp` deletion/keep bookkeeping. Eight native model/runner tests
and 16 local pipeline tests cover separate execution/publication behavior. Real
tools verify MP4/MKV/WebM codecs, language/name tags, cue text, copied media and
replacement of existing subtitle tracks. See [source applicability](../upstream/notes/subtitle-embedding.md)
and [fixture provenance](../tests/fixtures/subtitle_embedding/README.md). This does
not establish every FFmpeg codec/container combination, arbitrary postprocessor
arguments or broader subtitle acquisition.

## Overwrite options

Ported from upstream `existing_file`, `FileDownloader.download`,
`_write_info_json` and `_write_subtitles` (see
[the source slice](../upstream/notes/overwrites.md)). By default
(`--no-force-overwrites`) an existing regular media file is reported as
`[download] FILE has already been downloaded`, not downloaded again, and
recorded in the archive, while info JSON and subtitle sidecars are rewritten.
`-w/--no-overwrites` also keeps existing info JSON and subtitles.
`--force-overwrites`/`--yes-overwrites` deletes and replaces every existing
output (`Deleting existing file FILE`, upstream `report_file_delete`). Sidecars are still written
before the media check, as upstream does.

Like upstream's `existing_video_file`, the processed name (`final_ext`: a
plain `--recode-video`/`--remux-video` extension, or the `--audio-format` of
`-x`) is checked before the prepared name; `--force-overwrites` deletes each
existing candidate. A single file found under its prepared name is also
reported finished (`[download] Download completed`, or the untimed `100% of
SIZE` line), as `FileDownloader.download` does. When processing is requested,
the processors run again on the existing file, with its extension: same-named
results replace it in place (a same-named audio conversion reports
`Deleting original file NAME.orig.EXT`), other results are written beside it
and the original is deleted unless kept. Fixups are not repeated, like
upstream's default policy. Verified against pinned upstream for audio
extraction, remuxing, recoding and metadata embedding with and without `-k`.

A single file found under its prepared name is reused only with
`-w`/`--no-overwrites` or when continuing with part files (upstream
`FileDownloader.download`); with `--no-continue` or `--no-part` it is
downloaded again over the existing file (`--no-part` with `--continue` first
tries to resume the existing file itself), and processing then deletes it
unless kept. When a part file (or, with `--no-part`, the destination) is
resumed, `[download] Destination` follows the resume messages, as upstream
names it once data arrives.

Native boundaries: processed downloads use a private workspace, so they never
resume an existing part file or destination (upstream resumes them before
processing); nonregular and dangling-symlink destinations are refused instead
of written through.
