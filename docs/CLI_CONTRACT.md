# CLI output contract

This is the machine-readable interface yt-dlp-rs keeps stable from 1.0 on
(see [Versioning](#versioning) for what that promises).
It is built only from upstream yt-dlp's own options, so a program that runs
the CLI as a subprocess can use the same arguments and parsing with either
yt-dlp or yt-dlp-rs. Where the two differ, the difference is listed here.

The human-readable output (`[download] 45.2% of …`, `[download] Destination:`,
`[Merger] Merging formats into …`, extractor messages, `--verbose` lines)
keeps following upstream as closely as the port can, but it is **not** part
of the contract. It may change with upstream or between releases.

Covered by `tests/cli_contract.rs` on Linux, macOS and Windows (GitHub's
hosted runners), with local servers only. The test
compares the port with saved output of the pinned upstream
(`f264a6cbd1f630894da0f368df00f8aae50140e0`, 2026.10.26), which
`tools/capture_cli_contract.py` captures from
`tests/fixtures/cli_contract/cases.json`. The comparison is exact after the
normalization described in that tool: paths and the server origin are
replaced, a JSON line becomes its list of keys, and repeated lines are
collapsed. Values that depend on timing (speed, ETA, elapsed time, how many
`downloading` statuses a transfer reports) are not compared. Only what
those tests cover is promised. On Windows the shell-script FFmpeg stand-in
runs through Git for Windows' `sh`, both signal cases are a Ctrl+Break, and
`\` in paths is compared as `/`; see [Windows](#windows) for what differs.

## Streams

| Stream | Carries |
| --- | --- |
| stdout | `--progress-template` download lines, `--print` output, `-j`/`-J`/`--print-json` JSON, `--get-*` fields, and (unless quiet) the human messages |
| stderr | `WARNING:` and `ERROR:` lines, the `postprocess:` progress template while quiet, `--verbose` output, and the human messages while quiet with `--verbose` |

`--print`, `-j`, `-J`, `--print-json` and the `--get-*` options imply `--quiet`.
Quiet turns off download progress unless `--progress` is given. Each status
line ends in a newline only with `--newline`. Without it, statuses are
separated by `\r`, and the last one ends without a newline when a signal
stops the download.

## Progress: `--progress-template`

`--progress-template [TYPES:]TEMPLATE` is an output template over
`{"info": INFO, "progress": PROGRESS}`. `TYPES` is `download` (the default)
or `postprocess`, or several separated by commas. `download-title` and
`postprocess-title` set the terminal title instead (`--console-title`); they
are not part of the contract.

Pass `-q --progress --newline` with it. Then stdout holds one line per
status and nothing else except `--print` output.

### Download statuses

`progress.status` is `downloading` (repeated) and then one `finished` per
downloaded file. A failed or interrupted transfer has no `finished`. The
fields:

| Field | `downloading` | `finished` |
| --- | --- | --- |
| `status` | `"downloading"` | `"finished"` |
| `filename` | final name of the file being downloaded (relative to the working directory, as in `-o`) | same |
| `tmpfilename` | the file being written: `filename` + `.part` (with `--no-part`, or a `--growing-file` recording, `filename` itself) | absent |
| `downloaded_bytes` | bytes so far (all fragments so far for HLS/DASH) | final size (absent for a reused file) |
| `total_bytes` | `Content-Length` of a whole-file transfer, else `null`/absent | final size |
| `total_bytes_estimate` | HLS/DASH only: estimated from the fragments so far | absent |
| `fragment_index`, `fragment_count` | HLS/DASH only: completed fragments and their count (`null` count for live streams) | absent |
| `eta`, `speed`, `elapsed` | integer seconds, bytes per second, seconds; `null` until known | `speed`, `elapsed` (absent for a reused file) |
| `_percent` | number or `null` | `100.0` |
| `_percent_str`, `_speed_str`, `_eta_str`, `_total_bytes_str`, `_total_bytes_estimate_str`, `_downloaded_bytes_str`, `_elapsed_str` | upstream's padded human strings | subset |
| `_default_template` | the human `[download]` line without its prefix | same |
| `ctx_id`, `max_progress`, `progress_idx` | always `null` (upstream internals) | same |

Which keys a status has depends on the downloader, as upstream:

- HttpFD, for whole-file HTTP(S) downloads: `downloading` has `status, downloaded_bytes, total_bytes, tmpfilename, filename, eta, speed, elapsed, ctx_id` and then the `_…` strings. `finished` has `downloaded_bytes, total_bytes, filename, status, elapsed, ctx_id, speed` and then the strings.
- FragmentFD, for HLS, DASH, ISM and F4M (also with `-N`): `downloading` has `status, downloaded_bytes, fragment_index, fragment_count, filename, tmpfilename, max_progress, progress_idx, elapsed, total_bytes_estimate, speed, eta` and then the strings. `finished` adds `max_progress, progress_idx` after `ctx_id`.
- A merge input that already exists (reused): one `finished` with `filename, status, total_bytes` and then the strings.
- FFmpeg (`--download-sections`, live HLS through FFmpeg, `--downloader ffmpeg`) and other external downloaders: no `downloading` statuses. After success there is one `finished` with `filename, status, elapsed, downloaded_bytes, total_bytes` (this shape has unit tests only).
- The port's indexed `--download-sections` path for adaptive HTTPS formats (a [port extension](../README.md#extensions-in-this-fork)) reports `downloading` statuses with HttpFD's keys while it fetches the section's byte ranges. No contract test covers it, so its statuses are outside the contract; add `--downloader ffmpeg` for upstream's shape.

With `-N`, fragments are fetched in parallel. `fragment_index` counts the
fragments completed in any order, and its order relative to the byte counts
depends on thread timing, as upstream.

`info` is the video's info dict (the fields `-j` prints) as it is being
downloaded. For a merged download (`-f bv+ba`), each format is downloaded
separately, as `NAME.fFORMAT_ID.EXT`. For each one, `progress.filename` is
that file and `info` is the video narrowed to that format (`info.format_id`,
`info.ext`, `info.vcodec`/`info.acodec` of the stream). Only named fields
such as `%(info.id)s` are part of the contract. The full `%(info)j` dict has
more keys and a different key order than upstream (for example no
`_version`).

A template field whose value is `null` or missing renders as `NA`, also
with the `j` conversion (`%(progress.eta)j` prints `NA`, which is not valid
JSON). For JSON, use the whole dict (`%(progress)j`) or a subset, which
leaves out null and missing keys:

```
download:[progress] %(progress.{status,downloaded_bytes,total_bytes,total_bytes_estimate,fragment_index,fragment_count,eta,speed,filename,tmpfilename})j
```

`--growing-file` (a port extension) adds `recorded_seconds` and
`_recorded_str` to `downloading`.

### Postprocessor statuses

A `postprocess:` template renders for each postprocessor run's `started`
and `finished` hooks, over `{"info": …, "progress": {"status",
"postprocessor", "_default_template"}}`. `progress.postprocessor` is
upstream's name for the processor, for example `Merger`, `Metadata`,
`ExtractAudio`, `FixupM3u8` or `MoveFiles`. A failed run has no `finished`.
While quiet these lines go to **stderr**, otherwise to stdout.

## Stages: `--print` and `--print-to-file`

`--print [STAGE:]TEMPLATE` prints a template, or a field name such as
`filepath`, at a stage of each video. `--print-to-file [STAGE:]TEMPLATE FILE`
appends it to FILE instead. The default stage is `video`. Per video, the
stages print in this order:

| Stage | When | Useful fields |
| --- | --- | --- |
| `pre_process` | after extraction, before format selection | `id`, `title`, `webpage_url` |
| `after_filter` | after `--match-filters` | |
| `video` | after format selection | `format_id`, `duration`, `live_status`, `playlist_index` |
| `before_dl` | before the download | `filename` (the prepared output name, relative), `duration`, `live_status` |
| `post_process` | after the download, before postprocessing | `filepath` (relative) |
| `after_move` | after postprocessing and moving to the final directory | `filepath` (**absolute** path of the final file) |
| `after_video` | after everything for the video | |
| `playlist` | once after all entries of a playlist | `id`, `title`, `playlist_count` |

`--print` implies `--simulate` (no download) unless one of the templates
uses `before_dl`, `post_process`, `after_move`, `after_video` or `playlist`.
In that case the download happens and the stages run. `--no-simulate`
downloads in any case, and `-s` never downloads. In a simulation, stages
from `before_dl` to `after_move` do not print.

`-j` (`--dump-json`) prints each video's JSON after its `video` stage.
`-J` (`--dump-single-json`) prints one JSON object per URL at the end (a
playlist with its `entries`), or `null` when extraction fails. Both imply
`--simulate`. With `--no-simulate` they download too, and the stages from
`before_dl` on print as in a download. `--print-json` is `-j` that
downloads: it prints the JSON before `before_dl`. Not compared with
upstream: the JSON lacks upstream's `_version` key, and some internal `__`
keys differ (see `docs/COMPATIBILITY.md`).

A consumer that needs the final path should use
`--print after_move:filepath` and treat that line as the file. It prints
only for a file that was finished, or kept by `--keep-partial-on-stop`.

## Exit codes

| Code | Meaning |
| --- | --- |
| 0 | every requested video was processed without an error |
| 1 | an error was reported (`ERROR:` on stderr) for at least one input or video, the run was interrupted by a signal (see below), or the program crashed |
| 2 | usage error: unknown option, invalid option value or no URL. stderr has the usage line and `yt-dlp-rs: error: MESSAGE` |
| 101 | the run was cut short on purpose: `--max-downloads` reached, `--break-on-existing`, `--break-on-reject` or `--break-match-filters` |

With the default `--no-abort-on-error`, later URLs and playlist entries
still run after an error, and the exit code is 1. `--abort-on-error` stops at
the first error. An error does not always end its video: like upstream, a
live DASH format downloaded with `--downloader native` reports `ERROR: Live
DASH videos are not supported` and still downloads the manifest's snapshot
(with `after_move` and `finished` statuses), and the run exits 1. A few malformed values are rejected by the argument parser
itself, with its own message instead of upstream's, but they also exit 2
(`docs/COMPATIBILITY.md`).

## Signals: stop and cancel

On Unix, SIGINT and SIGTERM to the process (or its process group, which
also reaches FFmpeg) end the run like upstream's Ctrl+C. Downloads, a
blocked connect or read included, stop within about 100 ms. Later URLs and
playlist entries are not processed. stderr ends with a blank line and
`ERROR: Interrupted by user`, and the exit code is 1. A second signal exits
at once with the same message.

Upstream yt-dlp handles only SIGINT this way. On SIGTERM Python dies of the
signal: exit status −15, or 143 in a shell, no message and no cleanup. A
consumer that sends SIGTERM to both should treat "exit 1 with
`Interrupted by user`" and "killed by its signal" the same.

Without `--keep-partial-on-stop` (upstream behavior):

- stdout: no `after_move` line for the interrupted video. The last progress
  status is `downloading`, with no `finished`.
- Files: the download's `tmpfilename` stays (`NAME.EXT.part`; for a
  whole-file download only once data arrived, like upstream), along with
  the `NAME.EXT.ytdl` state of an HLS/DASH download and the
  `NAME.fFORMAT_ID.EXT` files of a merge, so a later run with the same
  options continues where it stopped (`--continue`, the default). A
  postprocessing interruption keeps the latest completed stage. The port's
  private temporary files are removed.

With `--keep-partial-on-stop` (a port extension, not in upstream):

- The received data becomes a usable file at the final name. HLS/DASH keep
  whole fragments, and a merge combines the streams so far (audio is
  downloaded first). The fixups run and the file is moved to its final
  directory.
- stdout: the `after_move` print runs, so `--print after_move:filepath`
  prints the kept file's absolute path. Other postprocessing and the
  download archive record are skipped.
- stderr and exit code as above (`ERROR: Interrupted by user`, exit 1), and
  later videos are not processed.
- A stop before any data arrived (before an HLS/DASH download's first
  whole fragment) leaves nothing, not even its `.part` and `.ytdl` files.
  A stop right after a resumed download continues keeps the earlier run's
  fragments.
- A merge of large streams takes time after the signal. Wait for the
  process to exit before killing it.

Cancel (stop and delete) is the consumer's job: send the signal without
`--keep-partial-on-stop`, wait for the exit, then delete the reported
`tmpfilename`, the `NAME.fFORMAT_ID.EXT` format files and any `.part` and
`.ytdl` files that share the reported name (and `-FragN` files with
`--keep-fragments`).

Growing-file recordings (`--growing-file`), live downloads through FFmpeg
and `--live-from-start` downloads handle the signal themselves, with or
without `--keep-partial-on-stop`: they finish the file (merging the
formats of a `--live-from-start` merge), print `after_move` and exit 0
(`docs/COMPATIBILITY.md`). The format files of a `--live-from-start` merge
are named with the merged extension (`NAME.fFORMAT_ID.mkv`), and their
progress statuses report those names, like upstream.

## Windows

Everything above holds on Windows, tested on GitHub's `windows-latest`
runners, except:

- Signals. Windows has no SIGTERM. Ctrl+C (SIGINT, as for Python) and
  Ctrl+Break (the port's stand-in for SIGTERM) both stop the run as
  described in [Signals](#signals-stop-and-cancel), `--keep-partial-on-stop`
  included. A console event reaches every process attached to the console
  in the target process group, FFmpeg included. To stop one run, start it
  with `CREATE_NEW_PROCESS_GROUP` and send
  `GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, PID)`; Ctrl+C cannot be sent
  to one group, and a new group ignores it. A process without a console
  (started with `DETACHED_PROCESS`) cannot receive either. Upstream on
  Windows handles Ctrl+C the same way; Python installs no Ctrl+Break
  handler, so Windows' default one ends it without a message.
- Exit codes are the same. A process ended with `TerminateProcess` exits
  with the code its killer chose; there is no "killed by a signal" status.
- Paths in `--print` output, `filename`/`tmpfilename` and messages use
  `\` separators and drive letters (`C:\Users\...`), as upstream's do;
  `--print after_move:filepath` is absolute. `%(...)q` in `--exec` quotes
  for `cmd.exe`, not a POSIX shell.
- Lines end with `\n`, not `\r\n`. Output of `--exec` commands is their
  own (`cmd`'s `echo` ends lines with `\r\n`).

## Versioning

From 1.0, within one major version:

- No key, `status` value, `--print` stage, exit code or signal behavior
  listed here is removed or renamed, and its meaning does not change.
- New keys and statuses may appear. Key order is not promised; parse the
  JSON.
- When an upstream baseline advance changes one of these upstream, the
  port keeps the old shape until its next major version and lists the
  difference here. A change to `tests/fixtures/cli_contract/expected.json`
  that removes a key or changes a value's meaning needs a major release.
- `--version` is not covered. It prints `yt-dlp-rs X.Y.Z`, where upstream
  prints its release date.

Human-readable output, `--verbose` output and the full `%(info)j` dict are
outside the contract (see the top of this page) and may change in any
release.

## Migration note for Aagedal Media Converter

Media Converter (`YTDLPService.swift`, `YTDLPProgressParser.swift`) passes
`--newline --progress --verbose` and three `--print` templates. Because
`--print` makes the run quiet, `--verbose` is what moves the human
`[download]` lines to stderr, where they are regex-parsed for progress, the
`Destination:` path and `[Merger] Merging formats into`. The same arguments
can instead produce lines that need no regexes, with both yt-dlp and
yt-dlp-rs:

1. Add a JSON progress template and keep the existing prints:

   ```
   -q --progress --newline
   --progress-template 'download:__AMC_PROGRESS__=%(progress.{status,downloaded_bytes,total_bytes,total_bytes_estimate,fragment_index,fragment_count,eta,speed,filename,tmpfilename})j'
   --print 'before_dl:__AMC_LIVE_STATUS__=%(live_status)s'
   --print 'before_dl:__AMC_SOURCE_DURATION__=%(duration)s'
   --print 'after_move:filepath'
   ```

   `--verbose` is then only needed for logs. It no longer has to be parsed.
2. Parse stdout line by line:
   - `__AMC_PROGRESS__=` is followed by a JSON object (null fields left out).
     Progress is `downloaded_bytes / (total_bytes or total_bytes_estimate)`.
     Without either, it is indeterminate, as for live streams. `fragment_index`/`fragment_count` give
     fragment progress for HLS/DASH, and `eta` and `speed` are numbers.
   - The first `tmpfilename` of a download replaces the
     `[download] Destination:` regex as the file to recover on Stop. During
     a merge, `filename` changes to each format's `NAME.fID.EXT` file.
   - `"status": "finished"` marks a file complete (one per format of a
     merge). The merge itself follows. Add
     `--progress-template 'postprocess:__AMC_PP__=%(progress.{status,postprocessor})j'`
     to see `Merger`/`Metadata` start and finish. While quiet these lines
     arrive on **stderr**.
   - Any other stdout line after `before_dl` is the `after_move` path of the
     final file (absolute).
3. Exit codes: 0 success, 1 failure or interrupted, 2 bad arguments (do not
   fall back to yt-dlp with the same arguments), 101 not used by the app's
   options.
4. Stop: pass `--keep-partial-on-stop` to yt-dlp-rs only (yt-dlp rejects it
   with exit 2), send SIGTERM to the process group, and wait for the exit.
   The kept file is the `after_move` path, so no `.part` renaming is needed.
   Give a stopped merge more than the current 2 s
   (`SubprocessRunner.terminationGracePeriod`) before SIGKILL. With yt-dlp,
   keep the current `.part` recovery, and remember that SIGTERM kills it
   without a message (exit 143).
5. Cancel: SIGTERM without `--keep-partial-on-stop`, then delete the files
   listed under "Signals" above.

The human `[download]` lines can stay as a fallback while both engines are
in use. They are not promised to stay byte-identical.
