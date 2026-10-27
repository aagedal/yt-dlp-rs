# Upstream update to 2026.10.26

Previous baseline: development commit
`fa9886678eb65103303c627b8e013f13f8999b41` (2026-10-23, embedded version
`2026.10.21`). Target: release tag `2026.10.26`,
`f264a6cbd1f630894da0f368df00f8aae50140e0`. `tools/upstream_report.py`
lists eight intervening commits; each is classified below.

| Commit | Subject | Classification |
| --- | --- | --- |
| `7f7106cd3` | [rh:curl_cffi] Support `curl_cffi` 0.17.x (#17929) | Not applicable. Upstream accepts curl_cffi 0.17 and adds its Chrome 151/152, Firefox 153–155 and Safari 27.0 targets to the curl_cffi request handler's table. This port's impersonation backend is `wreq` (`src/impersonate.rs`, optional `impersonate` feature), whose targets come from its own emulation set rather than curl_cffi's table, so `--list-impersonate-targets` and target resolution are unchanged. |
| `15262b0b9` | [test:networking] Fix supported impersonate targets test (#17931) | Upstream test of the curl_cffi table. Not applicable. |
| `7e3644cfa` | [cleanup] Misc (#17892) | Changelog, maintainer list, `Makefile`, `pyproject.toml` and the generator comment of `jsc/_builtin/vendor/_info.py` (`VERSION = '0.8.0'` and the hashes are unchanged, so `vendor/ejs` is unchanged). Not applicable. |
| `7747073a2`, `c91f8de0b` | [build] Update dependencies | Python packaging pins and `uv.lock` only. Not applicable. |
| `fd26888eb` | [ci] Update 7 actions in 9 workflows (#17925) | CI only. Not applicable. |
| `755a01732` | [ci] Test with PyPy 3.12 (#17926) | CI only. Not applicable. |
| `f264a6cbd` | Release 2026.10.26 | Version, changelog and supported-sites list only. The cache envelope's recorded version (`src/cache.rs` `UPSTREAM_VERSION`) follows the new release. |

No extractor, downloader, postprocessor or core module changed, so no
behavior was ported. Every saved site fixture was re-checked against the
target checkout with `capture_sites.py --upstream` (no `--write`), including
the SRG SSR and TV4 fixtures added alongside this update, and every
`compare_*` tool and `capture_info_order.py`/`capture_jsinterp.py`/
`capture_js_to_json.py`/`capture_json_ld.py` (without `--write`) were run
against it; upstream output did not change.

The global baseline, the tools' `PINNED_UPSTREAM`, the generated tables'
recorded commit, `vendor/ejs/manifest.json` and the fixtures'
`upstream_commit` now name the target commit; only the YouTube challenge
vector file's recorded hash (`tools/test_ejs_assets.py`) changed with it.

Caveat: `compare_ejs.py` and `compare_youtube.py`'s signature step require
the pinned Deno 2.9.7 (only 2.9.8 was installed); the EJS release and the
`jsc` package's code are identical between the two baselines.
