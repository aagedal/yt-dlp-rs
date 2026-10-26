# Upstream update to fa98866

Previous baseline: release tag `2026.10.21`,
`6ccd977303412151ee23352c6257dd658ffc1987`. Target: development commit
`fa9886678eb65103303c627b8e013f13f8999b41` (2026-10-23, embedded version
still `2026.10.21`). `tools/upstream_report.py` lists four intervening
commits; each is classified below.

| Commit | Subject | Classification |
| --- | --- | --- |
| `fa9886678` | [ie/odnoklassniki] Fix boosty direct link extraction (#17749) | Ported: a `videoembed` page whose stub says the author was not found or is blocked, requested without a smuggled `referrer`, is re-extracted as a `url` result smuggling `https://boosty.to` as the referrer (upstream previously called a missing `_extract_desktop`, which the port reproduced). The smuggled `referrer` is now unsmuggled and sent as the page request's `Referer` (`odnoklassniki.rs`). Fixtures: `odnoklassniki` `videoembed/4242` (header-routed retry) and `videoembed/4343` (the retry's stub error ends the loop). |
| `3d28cf478` | Remove Python 3.10 support (#17160) | One behavior change in ported code: `devalue.parse_iter` validates a `Date` with `datetime.fromisoformat` instead of `parse_iso8601`, so date-only, week-date, offset-only and CPython's lenient forms are accepted. Ported as `dates::is_py_isoformat_datetime`, a byte-level port of CPython 3.15's C parser (it tolerates junk before an offset, unlike the pure-Python one); 400,000 mutated strings matched CPython during development, and `dates`/`devalue` unit tests keep CPython-derived vectors. Everything else is equivalent on Python 3.11+: `dt.UTC`/`to_bytes` defaults/`re.NOFLAG`/`TimeoutError` aliases, `sys.exception()` in `trouble` (traceback text only), `aes_gcm_decrypt_and_verify` padding arithmetic, and the removed `<3.11` netrc `""` workaround (the port already parses empty tokens as 3.11+ does). The deleted `compat/urllib` Windows-registry proxy patch (Python <3.10.5) and `update.py` are not ported. Packaging, CI and documentation changes are not applicable. |
| `9b58cb33d` | [build] Always use latest Python version on Windows (#17901) | Build tooling. Not applicable. |
| `43a45ae37` | [docs] Add changelog entries for dropping Py3.10 & Win8 support (#17889) | Changelog only. Not applicable. |

Every saved site fixture was re-checked against the target checkout with
`capture_sites.py --upstream` (no `--write`); only the new odnoklassniki
cases were added. Every other `compare_*` tool and `capture_jsinterp.py`/
`capture_info_order.py` (without `--write`) pass against the target.
`compare_dates.py` needed its pinned-clock `dt` shim to expose `UTC`, which
upstream's date helpers now use instead of `timezone.utc`.

The global baseline, the tools' `PINNED_UPSTREAM`, the
generated tables' recorded commit and the fixtures' `upstream_commit` now
name the target commit; the files those tables are generated from
(`jsc/_builtin/vendor/_info.py`, `peertube.py`, `adobepass.py`,
`jsinterp.py`) are unchanged between the two commits. Only the YouTube
challenge vector file's recorded hash (`tools/test_ejs_assets.py`) changed
with it.

Caveat: `compare_ejs.py` and `compare_youtube.py` require the pinned Deno
2.9.7 and were not run (only 2.9.8 was installed); the EJS release, the
`jsc` package and `_info.py` are byte-identical between the two baselines.
