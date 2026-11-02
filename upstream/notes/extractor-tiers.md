# Road to 1.0, step 9: extractor tiers

Survey of 2026-11-02 against the pinned baseline
`f264a6cbd1f630894da0f368df00f8aae50140e0` (2026.10.26), written before any
of the changes below. No baseline advance.

## How the counts were derived

`tools/extractor_tiers.py --survey --upstream .upstream/yt-dlp` (unit tests in
`tools/test_extractor_tiers.py`):

- The registry is the debug binary's own `--list-extractors` output: names in
  upstream's listing order, with upstream's `(CURRENTLY BROKEN)` suffix.
- Every case URL in `tests/fixtures/sites/*/cases.json` and every URL in
  `tools/canary_sites.json` is attributed to an extractor through
  `--list-extractors URL...`, upstream's own rule: the first extractor in
  listing order whose pattern matches, generic for the rest. Extraction picks
  the first match in `_ies` order instead; the two differ only where patterns
  overlap, which the listing and `--list-extractors` tests already pin.
- A fixture case also counts for an extractor it reaches through another one
  (embeds, `url_transparent`, playlists): the saved upstream output names it
  in a `[NAME] ` screen line or an `"extractor": "NAME"` field.
- Upstream flags come from the pinned checkout's
  `list_extractor_classes()`: `working()` (`_WORKING`), `_ENABLED`,
  `IE_DESC is not False` and `description()`.

## Results

| | Count |
| --- | ---: |
| Registered extractors (`--list-extractors`) | 904 of upstream's 1754 |
| Fixture directories / saved cases | 698 / 4039 |
| Handle a canary URL (11 canary sites, 19 URLs) | 12 |
| Own fixture cases (besides the 12) | 885 |
| No own case, reached through another extractor | 3 (DetikEmbed, Viously, WDR) |
| Tested outside the site fixtures | 1 (`html5`: generic's HTML5 step, `tests/http.rs`, `tools/compare_html5.py`) |
| No fixture coverage at all | 3 (audius:track, GettrStreaming, ximalaya:album) |
| Upstream `_WORKING = False` (all classes) | 136 |
| ... of them registered here | 9, all already marked and warning |
| Registered here, absent upstream (removed) | 0 |
| Upstream `_ENABLED = False` among ours | 0 |
| Upstream `IE_DESC = False` among ours | 6, all absent from our `--extractor-descriptions` |

The 12 canary extractors: Bandcamp, Bluesky, facebook, Instagram, Reddit,
soundcloud, TikTok, twitch:clips, twitch:vod, twitter, vimeo (the Vimeo
canary uses the embed player) and youtube. The canary covers these classes,
not the whole site family: `youtube:tab`, `twitch:stream` or
`instagram:story` are tier 2.

The 9 upstream-broken extractors here (BehindKink, GodTube, instagram:user,
KankaNews, ProjectVeritas, SkylineWebcams, TeleTask, Toypics, Yappy) already
print upstream's `(CURRENTLY BROKEN)` suffix in `--list-extractors`, are left
out of `--extractor-descriptions`, and warn `The program functionality for
this site has been marked as broken, and will probably not work.` before
extraction. Each of their fixture directories has a case whose saved
upstream output holds that warning, so the behavior is already compared with
the pinned upstream.

`--list-extractors` equals upstream's listing filtered to our names, in
order. `--extractor-descriptions` differs in one line: `video.arnes.si`
lacks upstream's `: Arnes Video`.

## Gaps found on the way

The three extractors without fixtures prompted a check of every registered
extractor's non-HTTP upstream test URL (98 URLs of 37 extractors, run
offline against a closed proxy port). `src/extractor/mod.rs`'s
`pseudo_url` allowlist decides which non-HTTP schemes reach an extractor,
and it lacked seven that upstream accepts:

- `audius:ID` (audius:track), `cbcplayer:ID` (cbc.ca:player),
  `facebook:ID`, `globo:ID`, `nexx:...`, `radiocanada:...` and
  `wyborcza:video:ID` all failed with `unsupported URL scheme`.
- `lbry://@channel#id` and Bluesky's `at://did:plc:.../...` fail because
  the `url` crate cannot represent them: `lbry://@TestChan#5` parses as
  `lbry://TestChan#5` (empty userinfo dropped, `#5` becomes a fragment), so
  even the allowed `lbry://` video form with an `@channel` part reaches the
  wrong extractor; `did:plc:...` is read as a host and an invalid port.
  Upstream matches these as plain text.
- `svt:ID` fails like upstream (`Unsupported url scheme: "svt" (urllib)`).

## Decisions

- **Tiers.** Tier 1 is the canary list (`tools/canary_sites.json`): the
  extractors that handle a canary URL are checked daily against the live
  site and the pinned upstream, and breakage opens a `canary` issue. Tier 2
  is every other extractor with at least one saved fixture case (its own or
  one that reaches it): compared with pinned upstream on saved responses,
  best effort on the live site. An extractor without any is `untested`; the
  generated list says so, and new extractors need a fixture anyway
  (AGENTS.md). Being in tier 1 or 2 is not a claim about the whole upstream
  extractor: `upstream/manifest.json` scopes say which parts are ported.
- **Dead sites follow upstream.** Upstream keeps a dead or broken site's
  extractor with `_WORKING = False` (its `(CURRENTLY BROKEN)` listing suffix
  and the warning) and removes extractors only in its own releases. The port
  matches that exactly and adds no marking of its own: at a baseline advance,
  an extractor upstream newly marks gets the suffix, the description
  omission and the warning with a fixture; one upstream removes is removed
  here with its fixtures and manifest mapping. The survey's "not marked
  broken", "marked broken but working" and "absent upstream" lines are the
  check (docs/UPSTREAM.md). Nothing needs doing at this baseline: no
  registered extractor is gone upstream, and the 9 broken ones match.
  Sites this port never ported because they were dead (most of the skipped
  candidates in earlier batches) stay unported.
- **Full list.** `docs/SITES.md` is generated by
  `tools/extractor_tiers.py --write` from the registry, the fixtures and the
  canary list; CI's Linux `rust` job runs `--check` after the tests so it
  cannot drift.
