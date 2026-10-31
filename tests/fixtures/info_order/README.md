# Saved upstream `-j` lines (info dict key order)

Pinned official yt-dlp: `f264a6cbd1f630894da0f368df00f8aae50140e0`.

`site/` holds the served files; `cases.json` the exact `-j --user-agent fixture`
lines (`lines`) and `-J` lines (`single`) pinned upstream printed for each, with the `Content-Type` and
`Last-Modified` headers it was served with. The origin is `{ORIGIN}` (its host
`{HOST}`), and the volatile `epoch` and `_version` (upstream's release) are
removed; nothing else is normalized, except that a live result's local title
time (`YYYY-MM-DD HH:MM`, `HH_MM` in file names) becomes `{DATE} {HH}:{MM}`.

The cases cover GenericIE's typed direct link (`clip.mp4`), its untyped
"URL could be a direct video link" result (`clip.qqq`), a single HTML5 embed
(`one.html`), one with a poster, subtitle tracks and Open Graph metadata
(`rich.html`), an HTML5 playlist with a `<source>` (`three.html`), an HLS
master with subtitle renditions, a typed and an untyped (`#EXTM3U` body,
`text/plain`) media playlist, DASH manifests with segmented and direct text
tracks, multi-period and partly protected MPDs, and dynamic (live) MPDs.

Regenerate with `python3 tools/capture_info_order.py --upstream PATH --write`
(without `--write` it checks the saved lines). `tests/info_order.rs` serves the
same files and headers to the native CLI and compares its lines exactly.

`loaded/` holds info JSON written by the pinned upstream (`--skip-download
--write-info-json --user-agent UA`, host replaced by `media.invalid`,
`epoch` fixed) and upstream's `--load-info-json` `-j`/`-J` lines for it
(`epoch` and `_version` removed).
