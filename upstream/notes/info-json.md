# Video and playlist info JSON sidecars

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against this official checkout and exact target:
no intervening commits, changed files, affected components or unmapped files.
The global baseline is unchanged; no general catch-up claim is made.

Inspected `YoutubeDL.py` `_write_info_json` (4400–4420), `sanitize_info`
(3740–3775), `_prepare_filename` (1525–1553), `process_info` ordering
(3334–3410), `__process_playlist` playlist files (2040–2195) and
`_playlist_infodict`; `utils/_utils.py` `OUTTMPL_TYPES`, `_change_extension`/
`replace_extension` and `write_json_file`; `options.py:1498–1520` and
`__init__.py` parameter wiring. These mapped files also cover unsupported
description/thumbnail/link files, `--load-info-json`, `--no-clean-info-json`,
`--embed-info-json`, typed `-o infojson:` templates, comments and overwrite
options; none is implied by this slice.

`--write-info-json`/`--no-write-info-json` (last wins) write cleaned metadata.
Like upstream, simulation, `-j` and listing never write; `--skip-download` does.
Video files are written after subtitle sidecars and before media. Playlist files
are written before any entry is processed; a failed playlist write skips that
playlist, and a fully archived playlist still writes its file. Upstream's final
"updated playlist" rewrite is omitted because it produces identical cleaned
content here (requested entries are removed and no playlist processors run).

Names use `replace_extension(name, "info.json", ext)` with Python `splitext`
edges. Video names use the media template and selected `ext`; playlist names
render the media template over playlist id/title/extractor/webpage URL,
`playlist_index` 0, selected count, full count and largest selected index
(upstream `__last_playlist_index` padding); other represented fields are missing
and fall back to defaults. The names are planned with subtitle names before any
I/O; collisions with media, sidecars, cookies or archive files fail explicitly.

Content is the represented `VideoInfo`/`PlaylistInfo` subset after upstream
clean filtering: private/requested/filename keys, `__` keys and null dictionary
values are removed recursively; list nulls are kept; `epoch` and `_type` are
appended when absent. `playlist_count` is added for playlists. `_version` is
omitted because it describes upstream's own release (documented comparison rule).
Bytes follow `json.dump(..., ensure_ascii=False)` separators and escaping.
Existing destinations are kept with upstream's "already present" message, as the
default `overwrites=None` does; non-regular destinations fail. Writes create
parents, sync a sibling temporary file using the umask-reduced 0666 mode and
publish without replacing a concurrently created file.

Five native unit tests, six localhost CLI cases and a pinned fixture cover names,
cleaning/key order, bytes, retention, ordering, playlist counts, archive and
reserved-path behavior. `tools/compare_info_json.py` runs actual
`replace_extension`, `prepare_filename(..., "pl_infojson")`, `sanitize_info` and
`write_json_file` offline: 13 names, 5 playlist names, 4 sanitize, 4 byte cases
and one source-only `_version` case. Fields upstream adds that are not modeled
(`webpage_url_basename`, `format`, `display_id`, ...) remain absent, as in `-j`.

```sh
python3 tools/compare_info_json.py --upstream /path/to/yt-dlp
```
