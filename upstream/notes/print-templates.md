# Video-stage print templates

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against this official checkout and exact target:
no intervening commits, changed files, affected components or unmapped files.
The global baseline is unchanged; no general catch-up claim is made.

Inspected `options.py` `when_prefix`/`_dict_from_options_callback` (268–310) and
the `-O`/`--print`, `--print-to-file` and hidden `--get-*` definitions
(1259–1305); `__init__.py` template validation (325–345), `print_only`,
`any_getting`, quiet and `simulate` wiring (755–817); `YoutubeDL.py`
`_forceprint` (3217–3255), `__forced_printings` (3257–3284), its `process_info`
call (3360–3367) and `prepare_outtmpl`/`evaluate_outtmpl` (1267–1500). These
mapped files also cover `--print-to-file`, playlist and post-processing stages,
`formats_table`/`thumbnails_table` fields, `--get-format`/`--get-duration`/
`--get-thumbnail`/`--get-description`, `--no-simulate`, `-J` and `--print-json`;
none is implied by this slice.

Implemented: `[video:]TEMPLATE` (case-insensitive recognized stage prefix only),
bare field-list shorthand and `field=` debug lines; unsanitized typed rendering
where empty strings stay available; Python-compatible scalar `j` output with
`+` disabling `ensure_ascii`; `filename` and newline-joined selected `urls`;
`-e`, `--get-id`, `-g` and `--get-filename` after all templates in upstream
order; and quiet simulation implied by any of them. Lines precede `-j` JSON.
Process order follows `process_info` with complete (`format` present) metadata.

Native policy differences, recorded as source-only fixture cases: unknown fields
and traversal are rejected at startup where upstream prints `NA`/traverses;
dictionary shorthand, unsupported conversions and every non-`video` stage are
rejected. All lines for one video are rendered before writing so a failure
prints nothing for it. Filename rendering failures only affect templates that
reference `filename`; upstream logs the error and substitutes `None`.

Five native unit tests and five localhost CLI cases cover parsing, ordering,
JSON escaping, playlist entries, listing interaction, filename failures and
startup rejections without media writes. `tools/compare_print.py` runs actual
pinned `parseOpts` and `__forced_printings` offline: 19 shared exact-line cases
and 5 source-only cases.

```sh
python3 tools/compare_print.py --upstream /path/to/yt-dlp
```
