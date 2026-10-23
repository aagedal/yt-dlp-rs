# Upstream update to 2026.10.21

Previous baseline: `897341241f2bfb2439fe0367f4c7c9ccd29a3828` (development
commit after 2026.08.19). Target: release tag `2026.10.21`,
`6ccd977303412151ee23352c6257dd658ffc1987`. `tools/upstream_report.py`
lists nine intervening commits; each is classified below.

| Commit | Subject | Classification |
| --- | --- | --- |
| `12ac9aca7` | [ie/youtube] Fix initial data extraction (#17703) | Ported: `extract_yt_initial_data` reads a dedicated `<script id="yt-initial-data">` element first, for watch pages, tabs and live chat (`youtube.rs` `initial_data_script`). Fixture: `youtube-tab-pages` `PLscripttag…`. |
| `a51ef56c0` | [ie/youtube] Improve error messages (#17884) | Ported: the error screen's `playerInterstitialRenderer` view model supplies the reason/subreason, and an identical subreason is not repeated (`youtube.rs` `unplayable_reason`). Fixtures: `youtube-playability` `Interstit01`/`Interstit02`. |
| `fe84bef39` | [ie/kick] Rework extractors (#17322) | Ported in `kick.rs` (see the kick component scope); fixtures in `tests/fixtures/sites/kick/`. |
| `36b92b4f1` | [cleanup] Misc (#17569) | `downloader/external.py`: curl reads `--cookie -` from stdin from exactly 7.59 (`>=`); ported in `external_fd.rs`. README/changelog changes are documentation only. |
| `70279f2f0`, `f59cd0ba0` | [build] Update dependencies | Python packaging pins only; `yt-dlp-ejs` stays 0.8.0, so `vendor/ejs` is unchanged. Not applicable. |
| `f82b26812` | [build] Bypass cooldown for `deno` | Build tooling. Not applicable. |
| `8cdf4fea4` | [build] Fix armv7l executable verification | Build tooling. Not applicable. |
| `6ccd97730` | Release 2026.10.21 | Version/changelog/supportedsites only. |

Every saved upstream fixture was re-checked against the target checkout
(`--upstream` pointing at `6ccd97730`): `capture_sites.py` for all site
fixtures, every `compare_*` tool, and the `capture_*` tools into temporary
files diffed with the saved ones. Outside the YouTube, Kick and curl changes
above, upstream output did not change. The global baseline, the tools'
`PINNED_UPSTREAM` and the fixtures' `upstream_commit` now name the target
commit (only the YouTube challenge vector file's recorded hash changed with
it).

Not re-verified here:

- `compare_ejs.py` and `compare_youtube.py`'s signature step require the pinned
  Deno 2.9.7; only 2.9.8 was installed. The EJS release (0.8.0) and
  `_info.py` are byte-identical between the two baselines.
- `compare_dash.py`, `compare_hls.py` and `compare_hls_audio.py` fail
  identically at the previous baseline and pre-update code: they read native
  `-j` fields (`fragments`, `stream_kind`, period grouping) that the
  upstream-shaped info dict no longer emits. The tools need updating; the
  behaviors stay covered by the Rust HLS/DASH tests and site fixtures.

## Live checks

Alongside the update, the binary was compared live with upstream 2026.10.21
on roughly fifty public URLs from upstream's `_TESTS` (manual, outside the
required suite). Differences found there were fixed with saved regression
fixtures: response charset decoding (`_guess_encoding_from_content`; VK sends
windows-1251), `Content-Encoding` decoding of extraction responses (Imgur's
API compresses regardless of `Accept-Encoding: identity`), dropping zero
durations in `process_video_result`, HLS/DASH extraction bounds that
rejected long Twitch VODs, title-case HTTP/1.1 header names (Cloudflare
answers kick.com's lowercase ones with 403), generic pages decoded like
`_download_webpage` instead of rejecting non-UTF-8 HTML, Twitch VODs that are
still recording (only the master playlist is read), HLS
`#EXT-X-I-FRAME-STREAM-INF` lines (TED), unsegmented DASH representations
and upper-case MPD namespaces (OK.ru) and signed Loom masters whose variants
are read only at download time.

The same release also brought new or extended extractors for the largest
social networks, each compared with 2026.10.21 output on saved fixtures:
Facebook (legacy pages, watch parties, ads, redirects), Instagram (inline
DASH, logged-in API, stories, tags, users), TikTok (subtitles, users,
collections, live) and Douyin, X/Twitter (API selection, cards, broadcasts,
Spaces, Periscope), Twitch (live streams, channel lists, collections), VK
(wall posts, user videos, VK Play), Telegram embeds, Snapchat Spotlight and
XiaoHongShu.
