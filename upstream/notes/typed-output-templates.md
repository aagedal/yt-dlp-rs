# Typed filename template conversions

Exact official source target and unchanged global baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against the official checkout at this exact target;
there are no intervening commits, changed files, affected components or unmapped
files. This ports existing behavior without advancing the baseline.

Inspected `YoutubeDL.py` prepare_outtmpl/evaluate_outtmpl (1267–1522),
prepare_filename (1524 onward), exact-bare-s playlist compatibility widths,
float_or_none numeric casting, missing/default format reset, alternative fields,
and sanitation ordering; `utils/_utils.py` STR_FORMAT_RE_TMPL (2885);
`options.py` output-na-placeholder (1420); and
`test/test_YoutubeDL.py::test_prepare_outtmpl_and_filename` (622–758 and custom
conversion continuation). These mapped files also implement traversal,
arithmetic/dates, custom conversions, replacements, environment expansion,
platform sanitizers and other filename controls outside this slice. No
intervening unmapped changes require classification.

Native parsing/formatting is separate from CLI orchestration. It supports typed
represented fields with s/d/i/u/f/F conversions, Python-style flags/width/precision,
known-field alternatives and literal defaults. Missing/failed numeric values
reset all formatting; explicit empty defaults are preserved. Numeric conversions
follow source float casting rather than falsely claiming exact-integer d behavior
above 2^53. Plain s retains typed integer decimal strings and boolean spelling.
Bare playlist index/autonumber s widths use selected context assigned before
archive filtering. Original maximum index context is not exposed in JSON output.

Existing native component sanitization remains deliberate; only literal template
text can introduce directories. Sanitization precedes formatting, preserving
normal padding, with a final dot/space-only alias guard. Metadata/defaults do not
undergo recursive evaluation or environment expansion. Unknown fields and all
unsupported expressions/conversions remain errors rather than guessed values.
Specs, tokens, widths/precisions, selected text/default bytes and expanded bytes
have documented bounds. Metadata/listing/simulation keep output paths inert.

Shared fixtures compare actual pinned evaluate_outtmpl in filename mode on clean,
portable metadata, retaining Unicode precision, order, exact strings, float
rounding, flags, defaults and bare-s compatibility behavior. Native sanitation,
path containment, resource limits and malformed/unsupported cases are tested
separately without normalizing away the policy differences. Six source-only
cases record unsupported source behavior, not native conformance. CLI local
fixtures verify actual published filenames/bytes, selected-context padding,
archive filtering/no-clobber/failure continuation and read-only inertness.

```sh
python3 tools/compare_templates.py --upstream /path/to/yt-dlp
```

See [the compatibility contract](../../docs/COMPATIBILITY.md#typed-output-templates).
