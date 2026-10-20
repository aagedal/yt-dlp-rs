# Supported compatibility subset

This inventory describes tested behavior, not general yt-dlp compatibility.
All extraction and downloads run in Rust. Python is development tooling only.

## CLI

| Option | Current behavior |
| --- | --- |
| Positional URLs | One or more HTTP(S) URLs (stripped), processed sequentially; continue after errors, return failure if any URL fails; optional with a batch file |
| `-a`, `--batch-file FILE`, `--no-batch-file` | URLs from FILE or `-` (stdin) precede positional URLs; upstream comment/BOM/newline/whitespace rules and `~`/`$VAR` path expansion; unreadable file fails before requests; last flag wins; see [batch files](../upstream/notes/batch-file.md) |
| `-j`, `--dump-json` | One selected `VideoInfo` JSON object per video, including each HTML5 playlist entry; no downloads; with `-F`, a table precedes JSON |
| `-J`, `--dump-single-json` | One JSON object per URL after processing: the `-j` object for videos (`null` when selection fails); playlists add `_type`, `playlist_count`, `requested_entries` (omitted for the full playlist, like upstream) and the processed `entries` (`null` for failed entries); implies quiet simulation; native field shape like `-j`, not upstream `sanitize_info` output |
| `-F`, `--list-formats` | List eligible alternatives without selection or downloads; combined `-j`/`-s` retain normal selection; upstream `render_formats_table` layout without colors (the `[info] Available formats for ID:` caption is a screen message, so quiet modes print only the table) |
| `--list-formats-old`, `--no-list-formats-as-table`, `--list-formats-as-table` | Upstream `listformats_table`: the old `format code  extension  resolution  note` layout (`_format_note`) |
| `-q`, `--quiet`, `--no-quiet` | Suppress informational `[...]` stdout lines (upstream `to_screen`); implied by printing/JSON unless `--no-quiet`; errors and warnings remain; last flag wins |
| `--no-warnings` | Suppress `WARNING:` lines |
| `--dump-pages`, `--write-pages` | Upstream `_webpage_read_content` for the bodies extraction reads: the generic HTML or DASH page (before `Extracting information`), its m3u8 manifest (after `Downloading m3u8 information`) and the `Checking m3u8 live status` playlist, and embedded HTML5 manifests. `--dump-pages` prints `[EXTRACTOR] Dumping request to URL` and the base64 body; `--write-pages` prints `[EXTRACTOR] Saving request to FILE` and writes the body to `_request_dump_filename`'s name in the working directory (`ID_URL.dump`, restricted-sanitized, cut to `--trim-filenames` or 240 characters with a `___MD5` suffix). Direct media and the downloader's own playlist refetches are not dumped, as upstream; YouTube requests are not dumped |
| `-v`, `--verbose` | Upstream `write_debug` `[debug] ` lines on stderr: the configuration tree (`Command-line config: [...]`, nested `\| ` locations, `hide_login_info` `PRIVATE` values) and `Batch file urls`, then `print_debug_header`'s `Encodings` (utf-8, `No ANSI`/`dumb` per stream), `exe versions` (`ffmpeg -bsfs` version and `fdk`/`needs_adtstoasc`/`setts` features) and `Proxy map` (`--proxy` or `*_proxy` environment, Python dict) lines, then validation warnings, `Loaded 3 extractors` and `Loading archive file`; during processing `Identified a direct video link`/`M3U playlist`/`DASH manifest`, `Looking for embeds`/`Identified a html5 embed` (`N html5 embeds`), `Sort order given by user`/`by extractor` and `Formats sorted by` (with limits like `hdr:12(7)`), `Default format spec`, `Invoking http`/`hlsnative`/`dashsegments`/`ffmpeg downloader on "URL"` (also for subtitles), `ffmpeg`/`ffprobe command line` for every FFmpeg run (FFmpegFD also switches to `-loglevel verbose` without `-hide_banner`), MetadataParser `Searching for`/`Replacing all` and `Adding to archive`. `Extracting URL` lines are not truncated, quiet-mode screen lines go to stderr and `--no-warnings` warnings print as `[debug] WARNING:`. Verified against pinned upstream except environment lines: the version and system lines describe this port (`yt-dlp-rs version`, `Rust native build`), Python-only lines (lazy extractors, Git HEAD, optional libraries, JS runtimes, request handlers, plugins) are omitted, FFmpeg command lines show the resolved executable and this port's arguments, error tracebacks and failed-FFmpeg stderr dumps are not printed, and `Formats sorted by` is reported when a result is extracted (before, not between, a playlist's entry downloads) |
| `-s`, `--simulate`, `--no-simulate` | Extract metadata, print screen lines to stdout and diagnostics to stderr, create no media files; `--no-simulate` downloads even with printing/JSON/listing; last flag wins; resolution matches upstream (`tools/compare_simulate.py`) |
| `-O`, `--print [WHEN:]TEMPLATE` | Print a field list or unsanitized output template per selected video at the `video`, `before_dl`, `post_process`, `after_move` or `after_video` stage, or once per playlist at `playlist` (after its entries, over the playlist dict: `id`, `title`, `playlist_count`, `requested_entries` and the extracted entries); implies quiet, and simulation unless a later stage prints; repeatable; see [print templates](#print-templates) |
| `-e`/`--get-title`, `--get-id`, `-g`/`--get-url`, `--get-thumbnail`, `--get-description`, `--get-filename`, `--get-duration`, `--get-format` | Hidden upstream aliases printing, after `--print` lines and in upstream `__forced_printings` order, title, ID, selected URLs, thumbnail, description, prepared filename, `formatSeconds` duration and `format`; thumbnail, description and duration print only when present; imply quiet simulation |
| `-f`, `--format` | A port of upstream `build_format_selector`: atoms, ranks, extensions, IDs, `all`/`mergeall`, filters, groups, `/`, `+` and `,`; see [Format selection](#format-selection); format IDs follow upstream for direct, HLS and DASH results (see below); exactly `-f best` or `-f mp4` prints upstream's common-mistake warning, ordered with the other `validate_options` conflict warnings |
| `-o`, `--output [TYPES:]TEMPLATE` | Represented root metadata, `s/d/i/u/f/F` conversions, flags/width/precision, known alternatives/defaults and literal `%%`; upstream metadata sanitization; typed `subtitle`/`infojson`/`pl_infojson` templates (empty skips that file), `~`/`$VAR` template expansion; no overwrite; see [output paths](../upstream/notes/output-paths.md) |
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
| `--write-info-json`, `--no-write-info-json` | Write cleaned video/playlist `.info.json` files before media/entries; existing files are replaced unless `-w`; never with JSON/simulation/listing; last flag wins; see [info JSON](../upstream/notes/info-json.md) |
| `--restrict-filenames`, `--no-restrict-filenames` | ASCII-only upstream sanitization and space-free default template; last flag wins |
| `--windows-filenames`, `--no-windows-filenames` | Force upstream path sanitization, or minimal `/`-only sanitization outside Windows; last flag wins |
| `--trim-filenames`, `--trim-file-names LENGTH` | Trim rendered names excluding the last two dot-separated parts; 0 disables |
| `-w`, `--no-overwrites`, `--force-overwrites`, `--yes-overwrites`, `--no-force-overwrites` | Upstream overwrite modes for media, info JSON and subtitles; last flag wins; see [overwrite options](#overwrite-options) |
| `--match-filters FILTER`, `--no-match-filters`, `--break-match-filters FILTER`, `--no-break-match-filters` | Upstream `match_str` over the JSON form of represented metadata: `&` conditions, repeated filters as OR, comparisons with `?`/`!`, string operators, regexes (Rust `regex`; no lookaround/backreferences), `parse_filesize`/`parse_duration` numbers; checked before selection (format fields incomplete) and again before processing; rejected videos are skipped with upstream's message, breaking rejections cancel with exit 101; filters are validated at startup; interactive `-` is unsupported. A breaking rejection prints `[download] REASON`, then upstream's fixed `RejectedVideoReached` message |
| `--date`, `--datebefore`, `--dateafter DATE` | Upstream `DateRange` from strict `date_from_str` (`YYYYMMDD` or `now`/`today`/`yesterday` with `-N(day\|week\|month\|year)[s]`, UTC), `--date` overriding the others with upstream warnings, and upstream validation messages (out-of-range dates use the Python 3.13 wording); videos with an `upload_date` outside the range are skipped with `[download] YYYY-MM-DD upload date is not in range START to END` before the match filters; ASCII digits only |
| `-i`, `--ignore-errors`, `--no-abort-on-error`, `--abort-on-error`, `--no-ignore-errors` | Upstream `ignoreerrors` (last flag wins): by default (`--no-abort-on-error`) a failed input or video is reported, the queue continues and the exit status is 1; `--abort-on-error` stops the queue at the first reported error; `-i` additionally reports a failed postprocessor (`ERROR: …`) and continues the remaining processors with the latest completed file, so the video is published and archived (exit status 1). Merging is not skipped under `-i` |
| `--mtime`, `--no-mtime` | Upstream `try_utime`: with `--mtime` (off by default, last flag wins) whole-file media and subtitle downloads take their modification time from the final response's `Last-Modified` via ported `timeconvert` (`parsedate_tz` with zone offsets; the epoch is ignored), and fragmented downloads from the latest fragment carrying one; thumbnails are unaffected. Independently of `--mtime`, every FFmpeg output (merge, audio/video conversion, subtitle/metadata/thumbnail embedding) takes the oldest input modification time; timestamp failures are ignored |
| `--exec [WHEN:]CMD`, `--no-exec` | Upstream `ExecPP` on POSIX (`/bin/sh -c`; rejected on Windows): case-insensitive `WHEN:` prefixes `video`, `before_dl`, `post_process`, `after_move` (default) and `after_video` run at upstream's points (`video` and `after_video` also when simulating; `before_dl` also with `--skip-download`; `post_process`/`after_move` also for an already-downloaded file); `playlist` commands run once after a playlist's entries (also when simulating) over the playlist dict, unchanged when they have no fields; `pre_process` and `after_filter` are rejected. Templates use output-template fields with only `d`/`i`/`f`/`q` (shell-quoted) conversions plus `filepath` and `_filename`, validated at startup with upstream's unsafe-conversion/default/placeholder errors (exit 1); without fields, `{}` (appended when absent) becomes the quoted path, absolute after the move. `[Exec] Executing command: CMD` precedes each run; a failing `post_process`/`after_move` command fails the video before archiving (`ERROR: Postprocessing: …`, or reported and skipped under `-i`), and a failing `video`/`before_dl` command is reported as `ERROR: Preprocessing: …` and again after the video. `--no-exec` clears earlier commands; hidden `--exec-before-download` maps to `before_dl` with upstream's conflict warning |
| `--concat-playlist POLICY` | Upstream `FFmpegConcatPP` at the `playlist` stage: `always` joins the entries' downloaded files with the FFmpeg concat demuxer (stream copy) into the `pl_video` output template (`ext` from the entries, `mkv` when they differ; upstream's playlist fields), after an ffprobe check that every file has the same stream codecs (`The files have different streams/codecs and cannot be concatenated…`); `[Concat] Concatenating N files; Destination: FILE`, then the parts are deleted unless `-k` (`Deleting original file ABSPATH (pass -k to keep)`); a single entry is moved (`[Concat] Moving "A" to "B"`); a failed entry aborts it (`Aborting concatenation because some downloads failed`); nothing happens when simulating. The default `multi_video` and `never` do nothing, since this port's extractors produce no multi-video results. Runs before `--exec playlist:` commands |
| `--load-info-json FILE`, `--load-info` | Upstream `download_with_info_file`: FILE (with `~`/`$VAR` expansion) holds one dictionary or a list, each sanitized like the default `clean_infojson` (`None` values, `__` keys, `requested_*`, `entries`, `filepath`/`filename` and similar keys dropped) and processed without extraction: filters, archive checks and format selection run again over the saved `formats`. URLs are ignored with upstream's warning. Because sanitizing removes `entries`, a playlist prints its `Downloading playlist` line and falls back to re-extracting its `webpage_url` with upstream's `There are no entries` warning. Top-level keys this port does not model are kept (for templates, `--print`, JSON output and `stretched_ratio`). Upstream-written dictionaries load when they fit the represented subset (`direct` stays absent unless saved, stream kinds come from `vcodec`/`acodec` `none`, a missing top-level `url` comes from the last format); formats with other protocols and no saved fragments are rejected. A raised `DownloadError` (a failed subtitle download, or under `--abort-on-error` the first reported error) prints an empty `\r` line and `WARNING: The info failed to download: MESSAGE; trying with URL WEBPAGE_URL` (MESSAGE keeps its `ERROR: ` prefix when it was reported) and processes the webpage URL instead; the failure does not count toward the exit status |
| `--write-link`, `--write-url-link`, `--write-webloc-link`, `--write-desktop-link` | Upstream `_write_link_file` after info JSON (also with `--skip-download`, never when simulating): `--write-link` picks `.webloc` on macOS, `.desktop` on Linux and `.url` elsewhere; the `link` output template with the shortcut extension; `webpage_url` through ported `iri_to_uri` (http/https only, no IPv6; otherwise upstream's warning); `.url` files with CRLF and `.desktop` names escaped; an existing shortcut is kept (`Internet shortcut (.EXT) is already present`) unless `-w` is given, as upstream's overwrite check does |
| `--newline`, `--no-progress`, `--progress`, `--progress-delta SECONDS` | Upstream progress lines and printers; see [download progress](#download-progress-and-cooperative-cancellation) |
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
| `--embed-thumbnail`, `--no-embed-thumbnail` | Upstream `EmbedThumbnailPP` as the final stage after metadata: implies `--write-thumbnail` and deletes the written file after embedding unless it was requested explicitly; best on-disk thumbnail, WebP fixup, PNG conversion outside mkv/mka; mp3 ID3v2.3 cover, mkv/mka `-attach` with mimetype/filename (replacing an existing same-mimetype attachment), mp4/m4a/m4v/mov FFmpeg `attached_pic` fallback (mutagen/AtomicParsley are never used, so upstream's `Neither mutagen nor AtomicParsley was found. Falling back to ffmpeg` is always printed); upstream's `[ThumbnailsConvertor] Correcting thumbnail "FILE" extension to webp`/`Converting thumbnail "FILE" to png`, `[EmbedThumbnail] ffmpeg: Adding thumbnail to "FILE"`, and `There aren't any thumbnails to embed`/`There are no thumbnails on disk` when the processor runs; ogg/opus/flac and other containers are explicit errors; requires `--ffmpeg-location` and ffprobe |
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
| `--youtube-js-runtime PATH` | Opt into YouTube signature/query-n solving with an explicit Deno 2.9.7 executable; lazy validation, no discovery/installation; not general upstream runtime option parity |
| `--ffmpeg-location PATH` | Upstream `_determine_executables` for the CLI: without it `ffmpeg`/`ffprobe` are the first executable `PATH` entries; a directory holds both programs; a file selects the program its name contains (default `ffmpeg`) and finds the other by substituting the name, else beside it; a missing location warns `ffmpeg-location PATH does not exist! Continuing without ffmpeg` (once at startup) and disables ffmpeg. Availability (merge preference) does not run the executable for a version check like upstream; it is validated when used. The library keeps explicit `with_ffmpeg`/`with_ffprobe` configuration |
| `-x`, `--extract-audio`, `--audio-format`, `--audio-quality` | Ordered audio extraction mappings and bounded quality settings; see audio extraction below |
| `--remux-video`, `--recode-video` | Ordered video target mappings; stream copying or container-default re-encoding; recode wins |
| `--embed-metadata`, `--add-metadata`, `--no-embed-metadata`, `--no-add-metadata` | Final FFmpeg stage after subtitle embedding: upstream `FFmpegMetadataPP` stream-copy options, `-write_id3v1 1`, common tags from represented fields with upstream key precedence (title, date, description/synopsis, purl/comment, artist, genre, …) and per-stream `language`; requires `--ffmpeg-location` |
| `--embed-info-json`, `--no-embed-info-json` | Upstream `add_infojson` (`if_exists` with `--embed-metadata`): mkv/mka files get the written info JSON (or, with `--embed-info-json`, one generated for the purpose) as an `info.json` attachment with `mimetype=application/json`; other containers report `[Metadata] The info-json can only be attached to mkv/mka files`. Attachments are added to freshly downloaded files only (upstream also replaces an existing JSON attachment). Checked with ffprobe against pinned upstream |
| `--embed-chapters`, `--add-chapters`, `--no-embed-chapters`, `--no-add-chapters` | Upstream `addchapters` (implied by `--embed-metadata` unless disabled): `FFmpegMetadataPP` writes the chapters as an FFMETADATA second input (`_get_chapter_opts`: millisecond `START`/`END`, escaped titles) with `-map_metadata 1`; `[Metadata] There isn't any metadata to add` without chapters or metadata. Embedded chapters checked with ffprobe against pinned upstream |
| `chapters` (YouTube) | Upstream `_extract_chapters_from_json` (chaptered player bar), `_extract_chapters_from_engagement_panel` and `_extract_chapters_from_description`, via `_extract_chapters_helper` with its invalid/incomplete-chapter warnings, from the watch page's embedded `ytInitialData` (only when it has `contents`; upstream's `next` API refetch is not used). `process_video_result` fills `end_time` from the next start or the duration, inserts a chapter from 0 and names untitled ones `<Untitled Chapter N>`. Checked with three saved pages against pinned upstream |
| `--parse-metadata [WHEN:]FROM:TO`, `--replace-in-metadata [WHEN:]FIELDS REGEX REPLACE`, `--metadata-from-title` | Upstream `MetadataParserPP` (both options share one ordered action list per stage): `field_to_template`/`format_to_regex` interpretation of the unsanitized rendered template, `re.subn`-style replacement (a port of Python's `parse_template`, applied even without a match: `\1`/`\g<1>`/`\g<name>` groups checked against the regex with `invalid group reference`/`unknown group name`/`bad character in group name` errors, `\0`/three-digit octal escapes, `\a\b\f\n\r\t\v\\` escapes, `bad escape` errors for other letters, with upstream positions), and upstream's `[MetadataParser]` lines and warnings. Parsed fields this port does not model are kept as extra info-dict keys, available to templates, `--print`, JSON and `--embed-metadata`. Supported `WHEN`: `pre_process` (default), `after_filter`, `video`; later stages are a native usage error. Regex syntax and error text follow the Rust `regex` crate (no look-around or backreferences); the match filter does not see `pre_process` changes, because it runs earlier |
| `--xattrs`, `--xattr` | Upstream `XAttrMetadataPP` after SplitChapters: `[XAttrMetadata] Writing metadata to file's xattrs`, then `user.xdg.referrer.url`, `user.dublincore.{title,date,contributor,format,description}` and (macOS) `com.apple.metadata:kMDItemWhereFroms` from the info dict (hyphenated `upload_date`), written with `setxattr` (alternate data streams on Windows); upstream's no-space and too-long warnings and unsupported-filesystem error. Checked with `xattr -l` against pinned upstream on macOS |
| `--split-chapters`, `--split-tracks`, `--no-split-chapters`, `--no-split-tracks` | Upstream `FFmpegSplitChaptersPP` as the last `post_process` processor (after metadata/thumbnail embedding, before `--exec`): `[SplitChapters] Splitting video by chapters; N chapters found`, then per chapter `Chapter NNN; Destination: FILE` from the `chapter` output template (default `%(title)s - %(section_number)03d %(section_title)s [%(id)s].%(ext)s`, with `section_*` fields) and an input-seeking `-ss`/`-t` stream copy; the original file is kept; `Chapter information is unavailable` without chapters. Split files match pinned upstream byte for byte on saved media. With `--force-keyframes-at-cuts` and more than one chapter, `[SplitChapters] Re-encoding "FILE" with appropriate keyframes` first re-encodes a temporary copy with `-force_key_frames` at the chapter starts (deleted afterwards); an already-downloaded file is split again like upstream |
| `--sponsorblock-mark CATS`, `--sponsorblock-remove CATS`, `--sponsorblock-chapter-title TEMPLATE`, `--no-sponsorblock`, `--sponsorblock-api URL` | Upstream `SponsorBlockPP` as the last `after_filter` processor for YouTube videos (`[SponsorBlock] SponsorBlock is not supported for EXTRACTOR` otherwise): `[SponsorBlock] Fetching SponsorBlock segments`, a GET of `API/api/skipSegments/HASH4` (first four hex digits of the ID's SHA-256, `service=YouTube`, JSON category and action-type lists; categories in upstream's table order where upstream's set order varies; 404 means no segments; `--extractor-retries` retries with `Unable to communicate with SponsorBlock API: … Retrying (N/M)...` warnings and urllib-style `[Errno N]` texts, a final failure being a deferred `ERROR: Preprocessing: …` reported again after the video), upstream's duration filter and its `different duration` warning, `Found N segments`/`No matching segments were found in the SponsorBlock database`, and the `sponsorblock_chapters` field. Marked categories become chapters and removed ones are cut by `ModifyChaptersPP` (titles from `--sponsorblock-chapter-title`, default `[SponsorBlock]: %(category_names)l`, over the chapter's `start_time`/`end_time`/`category`/`categories`/`name`/`category_names`; identically titled neighbours merge; `SponsorBlock information is unavailable`/`There are no matching SponsorBlock chapters`); sponsor chapters keep their `category`, `type`, `categories`, `name` and `category_names` keys (in this port's key order). Categories follow upstream's `_set_from_options_callback` (`all`, `default` = `all` or `all,-filler` for removal, `-CATEGORY`, `wrong CATS for OPTION: VALUE`); `poi_highlight` and `chapter` cannot be removed; `--sponsorblock-mark` implies `--embed-chapters`; `--no-sponsorblock` clears both; invalid title templates are usage errors with this port's template error text |
| `--remove-chapters REGEX`, `--no-remove-chapters`, `--force-keyframes-at-cuts`, `--no-force-keyframes-at-cuts` | Upstream `ModifyChaptersPP` after subtitle embedding and before metadata: chapters whose title matches a regex are cut with the FFmpeg concat demuxer (`inpoint`/`outpoint` from `_make_concat_opts`, stream copy, the spec deleted silently), `[ModifyChapters] Removing chapters from FILE` for the media and each remaining `srt`/`vtt`/`ass`/`lrc` subtitle file (other subtitle formats warn that they are now out of sync), originals renamed `NAME.uncut.EXT` and deleted unless `-k`; `--force-keyframes-at-cuts` re-encodes first (`Re-encoding "FILE" with appropriate keyframes`). The remaining chapters are re-timed by a port of `_remove_marked_arrange_sponsors`/`_remove_tiny_rename_sponsors` (checked against upstream's own `TestModifyChaptersPP` cases, `tools/capture_modify_chapters.py`) and replace the info dict's chapters and duration for later processors, `--split-chapters`, `after_move` prints and `--exec`; `Chapter information is unavailable`, `There are no chapters matching the regex`, the `remove the entire video` warning, the already-cut skip and the duration-mismatch error follow upstream (ffprobe supplies the real duration). Arguments are parsed by upstream's `parse_chapters` (no negative times, no `*from-url`); like pinned upstream, `*START-END` ranges alone remove nothing, and ranges combined with a regex (a pinned-upstream `TypeError`) are a native error. Native boundaries: the duration stays whole seconds, chapter dicts keep this port's key order, and SponsorBlock categories are not available |
| `--use-extractors NAMES`, `--ies NAMES`, `--force-generic-extractor` | Upstream `allowed_extractors` (`orderedSet_from_options`: comma lists, case-insensitive full-match regexes, `all`, `default`, `-` exclusions) over this port's names `html5`, `youtube`, `generic`, `end`, in matching order; `ERROR: No suitable extractor found for URL URL`, or `[UnsupportedURL] Extracting URL` / `Unsupported URL: URL` with `end`. `--force-generic-extractor` tries only `generic` (`No suitable extractor (Generic) found …`). An invalid regex is a usage error (upstream raises a traceback) |
| `--default-search PREFIX`, unqualified inputs | Upstream `GenericIE` for inputs without a scheme: `[generic] Extracting URL: INPUT`, then a domain-like input is retried with `https://` (`The url doesn't specify the protocol, trying with https`), otherwise `ERROR: [generic] 'INPUT' is not a valid URL` (`fixup_error`/`error`). `auto`, `auto_warning` and search prefixes need search extractors, which are an explicit native error |
| `--check-formats`, `--check-all-formats`, `--no-check-formats` | Upstream `_check_formats`: a quiet test download (10241 HTTP bytes or the first fragment, no retries) per format, cached per ID. `--check-formats` tests each selector atom's candidates in preference order until one works (`[info] Testing format ID`, `WARNING: Unable to download format ID. Skipping...`); `--check-all-formats` tests every format best-first before listing and selection (`[info] Unable to download format ID. Skipping...`). The test downloader's error line uses this port's error text |
| `--alias ALIASES OPTIONS` | Upstream option aliases: each name (prefixed with `--` unless it starts with `-`) takes as many arguments as the `str.format` fields in OPTIONS, which are shell-quoted into the template, shlex-split and parsed in place (aliases may chain, at most 100 triggers each); templates without fields are used as-is. Aliases are expanded before parsing, so they are not listed in `--help` |
| `-t NAME`, `--preset-alias NAME` | Upstream `_PRESET_ALIASES` (`mp3`, `aac`, `mp4`, `mkv`, `sleep`), expanded in place before parsing; `Unknown preset alias: NAME` usage error. Like optparse, every option that takes a value now accepts one starting with `-` |
| Format filter fields | Besides the original fields, `[...]` filters accept upstream's `acodec`, `vcodec`, `container`, `protocol`, `dynamic_range`, `resolution`, `video_ext`, `audio_ext`, `format` (string operators, `!`, `?`) and numeric `fps`, `vbr`, `asr`, `audio_channels`; other keys remain an explicit unsupported-field error (upstream looks up any key) |
| `--list-extractors`, `--extractor-descriptions` | Upstream `print_extractor_information` for this port's extractors only (`html5`, `youtube`, `generic`, with upstream's descriptions); command-line URLs are listed under the extractor that would take them (`generic` takes the rest). Upstream lists about 1800 extractors |
| `--color [STREAM:]POLICY`, `--no-colors`, `--no-colours` | Validated like upstream (`"X" is not a valid color policy`); output is never colored, so `always` differs from upstream on capable terminals |
| `--write-comments`, `--get-comments`, `--no-write-comments`, `--no-get-comments` | Upstream `getcomments`: without `--write-info-json`/`--no-write-info-json` or `--embed-info-json`, and unless `-j` prints the JSON, it implies `--write-info-json`. The generic extractor has no comments, so nothing else changes; YouTube comment extraction (its comment API) is an explicit error |
| `--clean-info-json`, `--clean-infojson`, `--no-clean-info-json`, `--no-clean-infojson` | Upstream `clean_infojson`: the default drops `None` values, `__` keys and the private bookkeeping keys; `--no-clean-info-json` writes every represented field |
| `-N N`, `--concurrent-fragments N` | Upstream concurrent fragment downloads for HLS and DASH: N workers fetch fragments (each with its own `--fragment-retries` and skip handling) into private files beside the output, appended in order; progress reports completed fragments and in-flight bytes. Upstream's `concurrent fragments "N" must be positive` validation. Screen lines match sequential downloads apart from progress timing |
| `--downloader [PROTO:]NAME`, `--external-downloader`, `--downloader-args NAME:ARGS`, `--hls-prefer-native`, `--hls-prefer-ffmpeg` | Upstream `get_suitable_downloader` for `native` and `ffmpeg`/`avconv` (names or paths) per simplified protocol (`http`, `ftp`, `m3u8`, `dash`, `rtmp`, `default`): FFmpegFD downloads whole formats (merging all-FFmpeg pairs directly, without HlsFD's manifest line or fixups) when ffmpeg is available, otherwise the native downloaders run like upstream's `can_download` fallback. FFmpeg `--downloader-args` follow `_configuration_args` (`ffmpeg_iN`/`ffmpeg_i` before each input, `ffmpeg_oN`/`ffmpeg_o`/`ffmpeg`/`default` before the output); unnamed arguments print upstream's `External Downloader arguments given without specifying name` warning and move to the default downloader's key. `aria2c`, `axel`, `curl`, `httpie` and `wget` warn and use the native downloaders; unknown names are upstream's `No such [PROTO ]external downloader "NAME"` usage error. The HLS preference flags only affect the `m3u8` protocol, which no supported extractor produces |
| FFmpeg postprocessor failures | Like upstream's `real_run_ffmpeg`, postprocessor FFmpeg runs use `-loglevel repeat+info`, succeed on exit status 0 whatever stderr says, and fail with `ERROR: Postprocessing: LINE` from the last line of stderr (for example `Conversion failed!`) |
| `--postprocessor-args NAME:ARGS`, `--ppa` | Upstream `_configuration_args` for this port's FFmpeg postprocessors (`Merger`, `ExtractAudio`, `VideoRemuxer`, `VideoConvertor`, `Metadata`, `EmbedSubtitle`, `EmbedThumbnail`, `SubtitlesConvertor`, `ThumbnailsConvertor`, `SplitChapters`, `FixupM3u8`, `FixupM4a`): shlex-split values by case-insensitive `PP[+EXE][_i[N]\|_o[N]]` key (a later value replaces an earlier one); `PP+ffmpeg_iN`/`PP+ffmpeg_i` go before each input's `-i`, and `PP+ffmpeg_o1`/`PP+ffmpeg_o`/`PP+ffmpeg`, then `PP` and `ffmpeg` together, then `default` before the output file (a key given without arguments still ends the search). Unnamed values print `Post-Processor arguments given without specifying name. The arguments will be given to all post-processors` and become `default`; `wrong --ppa formatting; …` usage errors. The arguments join this port's own FFmpeg command lines, which differ from upstream's; ffprobe, AtomicParsley and unimplemented postprocessors (`ModifyChapters`, `FixupStretched`, `FixupTimestamp`, `FixupDuration`) take none |
| `--keep-fragments`, `--no-keep-fragments` | Upstream fragment files `TMPFILENAME-FragN` (`NAME.part-FragN`, or `NAME-FragN` with `--no-part`) are kept beside the output, byte-identical to the assembled fragments, for unprocessed fragmented downloads; processed downloads keep them in their private workspace (removed) |
| `--progress-template [TYPES:]TEMPLATE` | Upstream `_report_progress_status`: `download:` (default) templates render over `{'info': ..., 'progress': ...}` with `report_progress`'s fields (`status`, `downloaded_bytes`, `total_bytes`, `eta`, `speed`, `elapsed`, `fragment_index`/`fragment_count`, `filename`, `_percent_str`, `_speed_str`, `_eta_str`, `_total_bytes_str`, `_default_template`, ...) for downloading and finished statuses. `postprocess:` and the `-title` types are accepted but not rendered, because postprocessor progress hooks and console titles are not implemented |
| `--compat-options OPTS` | Upstream `_set_from_options_callback`/`orderedSet_from_options` (`all`, `youtube-dl`, `youtube-dlc` and year aliases, `-NAME` removal, `wrong OPTS for --compat-options: …`) and `set_compat_opts` for the defaults this port has: `abort-on-error`, `no-playlist-metafiles`, `no-clean-infojson`, `no-attach-info-json`, `mtime-by-default`, `filename` (`%(title)s-%(id)s.%(ext)s`), `format-sort` (`FormatSorter.ytdl_default` appended) and `no-keep-subs`; an explicitly given option marks the compatibility option `*NAME` in `-v`'s `Compatibility options:` line. `no-certifi`, `prefer-legacy-http-handler`, `no-live-chat`, `no-external-downloader-progress` and `embed-thumbnail-atomicparsley` concern features this port does not have; the others warn that they are not supported natively. `all` lists the options in this port's table order (upstream's set order varies) |
| Accepted tuning and environment options | `--buffer-size`, `--resize-buffer`/`--no-resize-buffer`, `--file-access-retries`, `--throttled-rate`, `--wait-for-video`/`--no-wait-for-video`, `--live-from-start`, `--mark-watched`, `--no-update`, `--cache-dir`/`--no-cache-dir`, `--no-plugin-dirs`, `--remote-components`, `--prefer-insecure`/`--prefer-unsecure`, `--ignore-no-formats-error`, `--allow-dynamic-mpd`/`--no-ignore-dynamic-mpd`/`--ignore-dynamic-mpd`/`--no-allow-dynamic-mpd`, the account (`-u`, `-p`, `-2`, `-n`, `--netrc-*`, `--video-password`, `--ap-*`) and geo (`--geo-verification-proxy`, `--xff`, `--geo-bypass*`) options are validated with upstream's messages (`buffer size "0" must be positive`, `invalid file access retry count "X" given`, `invalid time range to wait for video "X" given`, ...) and otherwise have no effect: the supported extractors use no logins, geo bypass or live streams, and reads use a fixed buffer. `--rm-cache-dir` prints upstream's `Removing cache dir DIR ..` for this port's (unused) `yt-dlp-rs` cache directory. `-U`/`--update`/`--update-to` are errors (no self-update). `--js-runtimes deno[:PATH]` selects the Deno runtime (a bare `deno` is looked up on PATH); other runtimes, `--extractor-args`, `--plugin-dirs`, multistreams, `--console-title`, `--legacy-server-connect`, `--bidi-workaround` and non-UTF-8 `--encoding` warn that they are not supported natively. `--impersonate` reports upstream's unavailable-target error and `--list-impersonate-targets` its table of unavailable `curl_cffi` targets; `--ap-list-mso` lists no providers |
| `--client-certificate CERTFILE`, `--client-certificate-key KEYFILE`, `--client-certificate-password PASSWORD` | TLS client authentication for every request: a PEM certificate (optionally containing its key) plus an optional separate PEM key. Encrypted keys and `--client-certificate-password` are an explicit native error, because the rustls transport needs unencrypted keys |
| `--enable-file-urls`, `file:` URLs | Upstream refuses `file:` URLs after `[generic] Extracting URL`/`Downloading webpage` with `file:// URLs are disabled by default in yt-dlp for security reasons. Use --enable-file-urls to enable at your own risk.`; with the option, local media files are direct links like urllib's file handler gives upstream (guessed Content-Type, mtime timestamp, protocol `file`) and are copied with progress. Local HTML pages and manifests remain an explicit native error |
| `--http-chunk-size SIZE`, `--test` | Upstream `HttpFD` chunking for whole-file HTTP media: each request asks for `bytes=START-END` of a randomized 95-100% chunk (`--http-chunk-size`, else the format's `downloader_options.http_chunk_size`, which YouTube HTTPS formats set to 10 MiB like upstream); a matching `206` continues with the next chunk, other responses restart as a whole download (`Unable to resume` past the start), and the size limits compare against each response's end like upstream. Hidden `--test` keeps the first 10241 bytes (one `bytes=0-10240` request, even if Range is ignored). Request patterns compared with pinned upstream; fragments and FFmpeg downloads ignore `--test` |
| `--download-sections REGEX` | Upstream `parse_chapters`/`download_range_func`: `*START-END` ranges (comma-separated, `parse_duration` timestamps, `inf`, negative values from the end when the duration is known), `*from-url` and chapter regexes, with upstream's usage errors (regex error text is this port's). Each range is processed as its own video with `section_start`/`section_end`/`section_title`/`section_number` (available to templates, `--print` and JSON) after `[info] ID: Downloading N time ranges: S-E`. Partial downloads use upstream's FFmpegFD: FFmpeg reads the format URLs with `-headers` (defaults, User-Agent, page Referer, cookies), `-ss`/`-t`, `-c copy`, `-map I:0` for merged pairs, `-f` from `EXT_TO_OUT_FORMATS` (`mp4`/`mpegts` for HLS) and `-loglevel quiet` in quiet modes, into `FILE.part`; its own output is inherited, then `[download] 100% of SIZE in TIME at SPEED`; no fixups follow. Without ffmpeg: `ERROR: You have requested downloading the video partially, but ffmpeg is not installed. Aborting`. Chapter regexes search the chapter titles (`section_title`, zero-based `section_number`); without matches upstream's `There are no chapters matching the regex` or, for videos without chapters, `Cannot match chapters since chapter information is unavailable` is printed and nothing is downloaded. `*from-url` downloads between the extracted `start_time` and `end_time` (YouTube's `t`/`start`/`end` URL parameters, fragment before query, parsed with `parse_duration` like upstream), or the whole video when neither is set. DASH formats pass the manifest URL with `-map I:manifest_stream_number` like upstream (numbers compared with upstream on the saved DASH fixtures; actual DASH input needs an FFmpeg build with its `dash` demuxer, as upstream does) |
| `--fixup POLICY`, `--hls-use-mpegts`, `--no-hls-use-mpegts` | Upstream `fixup()` for unmerged, unprocessed downloads: HLS (`hlsnative`, unless `--hls-use-mpegts`) MP4-labeled files that are MPEG-TS get `FFmpegFixupM3u8PP` (`[FixupM3u8] Fixing MPEG-TS in MP4 container of "FILE"`, stream copy `-map 0 -dn -ignore_unknown -c copy -f mp4`, `-bsf:a aac_adtstoasc` when ffprobe reports AAC, via `NAME.temp.EXT`); `m4a` downloads whose `container` is `m4a_dash` (DASH representations and YouTube audio-only formats, which carry upstream's `EXT_dash` containers) get `FFmpegFixupM4aPP` (`[FixupM4a] Correcting container of "FILE"`). A `stretched_ratio` other than 1 first gets `FFmpegFixupStretchedPP` for every download, merges and FFmpegFD included (`[FixupStretched] Fixing aspect ratio of "FILE"`, a stream copy with `-aspect RATIO`, `Skipping audio` for audio-only results, `ID: Non-uniform pixel ratio RATIO` warnings); the ratio is the info dict's (a single format's own value replaces it; a merge takes its video format's). Single DASH (`dashsegments`) downloads get `FFmpegFixupDuplicateMoovPP` (`[FixupDuplicateMoov] Fixing duplicate MOOV atoms of "FILE"`, a stream copy into the same container), since every MPD format is `is_dash_periods` upstream. Without ffmpeg upstream's `ID: MESSAGE. Install ffmpeg to fix this automatically` warning; `warn` only warns, `never`/`ignore` skip; `detect_or_warn` (default) and `force` fix new downloads (already-downloaded files are not re-fixed). MPEG-TS is detected from the file's bytes rather than ffprobe `format_name`. Merges and other FFmpeg inputs also choose demuxers from the bytes. Processed downloads (`-x`, embedding, recoding) are fixed up the same way before their processors run, like upstream's `fixup()` before `post_process`. Processed merges download upstream's `NAME.fID.EXT` intermediates beside the output and print the `[Merger]` and deletion lines before the processors run |
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
| `--no-playlist`, `--yes-playlist` | Upstream `noplaylist`: a YouTube `watch?v=ID&list=...` URL downloads just the video (`[youtube:tab] Downloading just the video ID because of --no-playlist`); without it such URLs remain an explicit unsupported-playlist error. HTML5 playlists are unaffected, as upstream's generic extractor ignores the option |
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

The built-in `Youtube` extractor runs before the generic HTTP fallback. Exact
`youtube.com`, `www.youtube.com`, `m.youtube.com`, `music.youtube.com`, `youtu.be`,
`www.youtu.be`, `youtube-nocookie.com`, and `www.youtube-nocookie.com` hosts are
claimed. Accept watch URLs with one `v` parameter, short links, `/shorts/`,
`/embed/`, and `/live/` paths with exactly an 11-character video ID. Watch-page requests
use the canonical HTTPS watch URL. Reject channels, playlists, malformed IDs,
and URLs containing `list=` before making a request. `/live/` is a URL shape;
live, upcoming, and post-live player responses remain unsupported.

Descriptive metadata follows upstream `_real_extract` for fields derivable from
the player response alone: `description` (original before microformat text),
`duration`, `channel_id`/`channel_url`, `uploader_id`/`uploader_url` (owner
handle), `view_count`, `average_rating`, `age_limit`, `categories`, `tags`,
`playable_in_embed`, `live_status`, `media_type`, `upload_date`/`timestamp`
(microformat `uploadDate` with zone) and `availability` (without initial-data
badges). `ytInitialData`-derived fields (`uploader`, `channel`, likes, comments,
chapters) and watch-page meta fallbacks are not extracted. Thumbnails follow
upstream: saved `videoDetails`/microformat thumbnails, 38 generated `i.ytimg.com`
candidates with name/WebP preferences, URL deduplication and the best saved
thumbnail as `thumbnail`. HTML5 media `poster` attributes become thumbnails.
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
All videos then get upstream `_sanitize_thumbnails` ordering, IDs, resolutions
and `thumbnail` finalization (no `--check-formats` URL testing). These
fields appear in JSON, info JSON, match filters and (scalar ones) output
templates; see [the source slice](../upstream/notes/youtube-metadata.md).

Watch pages must be uncompressed UTF-8/ASCII `text/html`, at most 2 MiB. Parse
JSON assigned to `ytInitialPlayerResponse` in a script, including quoted window
property assignments. Require `playabilityStatus.status=OK`, the requested video
ID, and a nonempty title. JavaScript runs only for eligible signature/query-n formats when an explicit
runtime is configured. Authentication, age/consent gates,
additional API clients, cookies, and token acquisition are unsupported.

Only when the initial response is absent, make one WEB player API POST to the
canonical page origin at `/youtubei/v1/player?prettyPrint=false`; the shared
HTTP redirect policy still applies. Require literal
`ytcfg.set({...})` script configuration with `INNERTUBE_CONTEXT.client` naming
`WEB` and a nonempty digits/dots client version (at most 128 bytes). Multiple
literal objects merge in declaration order. Construct a fresh context with that
name/version, `hl=en`, `gl=US`, and optional visitor data (at most 4096 bytes, no
control characters); send matching client/visitor headers and page Origin/Referer.
Page host overrides, API keys, user/session context, and credentials are ignored.
The body contains video ID, this client context, and HTML5 playback preference.
No signature timestamp, content/racy check overrides, player token, or auth is
sent. JSON responses must be uncompressed, at most 2 MiB, and pass the same
identity/playability/format validation. Malformed or unsupported initial
responses never invoke fallback. API errors fail explicitly; opt-in extraction
retries restart the whole page/API sequence after transient status/read failures.

WEB player requests need no PLAYER PO token at the pinned baseline. Its direct
HTTPS and DASH media require a GVS PO token except direct itag 18; HLS tokens are
optional. API-derived direct formats therefore retain only itag 18 before any
media probe; eligible HLS is considered independently. No surviving format is an
unsupported error.
This policy does not acquire tokens or assert that exempt URLs always work.
The original initial-response path retains its prior token/probe behavior.

Expose `streamingData.formats` followed by `adaptiveFormats` entries resolving
to direct HTTP(S) media, with MP4/M4A/WebM MIME types and recognized audio
and/or video codecs. Classify sources as standalone, video-only or audio-only. Preserve numeric itags as
format IDs and positive declared dimensions. Skip DRM, OTF/fragments, unknown
source types and live fragments. Encrypted `signatureCipher`/legacy `cipher` and
query-n URLs require explicit `--youtube-js-runtime PATH` or library configuration;
reject player responses with DRM license information. Eligible HLS manifests
are expanded through the native VOD pipeline described below. DASH discovery
remains disabled because the WEB GVS token is unavailable.
If no supported direct or HLS format remains, return an explicit unsupported error.
Supported alternatives retain declaration order and use the existing resolution
ranking or literal itag selection, not upstream's full sorter. Every eligible direct
URL must pass a direct media probe; a failing probe fails the entire extraction,
including when a different format was requested. Filesizes and final URLs come
from those probes. Token-protected formats can therefore prevent extraction even
if another format might work. Metadata uses `youtube` / `Youtube`, `direct=false`,
and the canonical watch page. The shared downloader sends that page as referrer.

This is fixture-tested groundwork, not general live-site YouTube support. Modern
YouTube may require other API clients, PO tokens, SABR, DASH discovery, or fragmented direct adaptive delivery; those remain
unimplemented.

An optional offline EJS/Deno pipeline solves encrypted signatures and query-n challenges during public
extraction with `--youtube-js-runtime PATH` or
`YoutubeDl::with_youtube_options(YoutubeOptions { js_runtime: Some(path), ..Default::default() })`.
The explicit executable must be Deno 2.9.7. Configuration is inert until an
eligible challenge needs solving; canonical path/version validation is cached
once per configured extractor. Token/live/DRM/format filtering precedes solving,
and complete-batch validation precedes every media probe. Player network failures
retain whole-extraction retry policy; runtime/protocol failures are permanent.
The [n extraction slice](../upstream/notes/youtube-n-extraction.md) records exact
behavior, cross-platform runtime evidence and deterministic extraction tests.
The [signature slice](../upstream/notes/youtube-signature-extraction.md) adds
exact cipher/default/custom-sp/combined-n extraction and download tests. The
optional comparison also accepts `--deno PATH` to run upstream's actual offline
EJS signature-permutation path against the same saved fixture. Without that flag,
the signature case is explicitly skipped.

An unsupported page cannot fall through to generic HTML5 and appear successful.
The deterministic Rust tests cover URL dispatch, player response validation,
format rejection, source probes, selection, and a local download. The optional
upstream parser comparison uses the same saved response and expected metadata:

```sh
python3 tools/compare_youtube.py --upstream /path/to/yt-dlp
```

Only ID, title, extractor identity, canonical webpage URL, and ordered formats'
itag, extension, dimensions, URL, and filesize are compared. Normalize only the
exact fixture origin to `{{ORIGIN}}`; all other compared values remain exact.
The harness bypasses upstream network/client acquisition and allows missing PO
tokens for the original watch-page fixture only. The additional saved API
response uses ordinary upstream WEB token policy and compares the retained
itag 18 metadata. These comparisons verify parsing and that token-policy subset,
not API acquisition/request parity, full ranking, or live-site usability. Local
Rust HTTP tests separately assert API requests, rejected responses, fallback
boundaries, whole-extraction retries, and token filtering before probes. See
[the watch-page brief](../upstream/notes/youtube-watch-page.md) and
[the API brief](../upstream/notes/youtube-player-api.md).

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
SECONDS` limits updates. Colors, terminal control sequences, console titles and
`--progress-template` are not implemented. JSON and simulation do not download
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
"direct URL to unfragmented media"; segmented DASH text tracks are not represented. Fragment progress lines use this port's
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
extractors would add are missing. Malformed traversal syntax remains an error.

Named substitutions use `%(field)FORMAT`, where conversions are `s`, `d`, `i`,
`u`, `f` and `F`. Formats support Python-style `#`, `0`, `-`, `+` and space flags,
ASCII minimum width, `.PRECISION` and one inert `h`/`l`/`L` length modifier. Width
is a minimum, string precision counts Unicode characters, integer precision
pads digits and floating precision defaults to six. Integer aliases truncate
numeric values toward zero; `F` uppercases nonfinite output. Nonfinite integer
conversion is an explicit error. Naked precision dots, unsupported conversions,
raw percent escapes and malformed named tokens fail; only `%%` escapes a percent.
These strict syntax limits differ from upstream's more permissive escaping.

`%(width,height|unknown)s` picks the first available known field. Empty strings
are unavailable. Only an unavailable value advances to the next alternative;
a chosen string that fails numeric casting uses the default instead. Explicit
`|TEXT` overrides `--output-na-placeholder` (default `NA`); either can be empty.
Unavailable and failed numeric conversions discard *all* formatting flags,
width and precision, so `%(width|0)04d` with missing width renders `0`, not `0000`.
All alternate field names are validated even after an available first candidate.
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

String values/defaults use upstream `sanitize_filename` (default `is_id`) before
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
Arithmetic/dates, replacements, scientific/general/hex/octal/repr and other custom
conversions (`h`, `B`, `U`, `D`, `S`, `c`), environment expansion
and full filename/platform compatibility remain open.

The shared 182-case comparison (including traversal and slice ranges) uses actual pinned `evaluate_outtmpl` in filename
sanitization mode on portable fields. Native safety/resource cases are separate;
unsafe-character output is not normalized to hide policy differences. Six
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
`-O field=` as `field = %(field)#j`). Arithmetic, `q`/other custom conversions
and `formats_table`-style fields are rejected before extraction, whereas
upstream evaluates them.

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
`exp=xpv` requires an unavailable token. Preserve earlier tracks, warn and skip
remaining tracks for that response. Existing URL `pot` values do not substitute
for provider acquisition. No token acquisition, additional client policies or
caption URL deciphering is implemented. Caption query `n` is preserved without
launching Deno. Native skip diagnostics consistently warn; upstream can report
this as a client notice depending on enabled options. Media token policies remain
separate, including the anonymous WEB direct itag18 restriction.

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
