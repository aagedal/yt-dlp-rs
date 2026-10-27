# Saved batch-file reference cases

Pinned official yt-dlp: `f264a6cbd1f630894da0f368df00f8aae50140e0`.

Each case in `cases.json` has hex-encoded batch-file bytes (`batch_hex`),
positional `urls` and the exact URL list returned by upstream `get_urls` for a
regular batch file (UTF-8 with `errors='ignore'`, universal newlines,
`read_batch_urls` comments/BOMs and final `strip`). The native unit test parses
the same bytes and strips the same positional URLs.

Run `python3 tools/compare_batch.py --upstream /path/to/pinned/yt-dlp`.
