# Subtitle embedding after native media processing

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against this official checkout and exact target:
no intervening commits, changed files, affected components or unmapped files.
The global baseline is unchanged; no general catch-up claim is made.

Inspected `postprocessor/ffmpeg.py:213–225,580–668`, shared FFmpeg execution,
`utils/_utils.py:3644–3842` ISO639, `options.py:1683–1689`,
`__init__.py:674–683`, and `YoutubeDL.py:3810–3828` run_pp deletion policy,
plus related postprocessor tests. These mapped components also cover unsupported
metadata, chapter/thumbnail processing, arbitrary arguments, fixups, timestamps
and broader orchestration. No intervening unmapped changes require classification.

The model implements all six source containers, missing/JSON/WebM eligibility,
MP4 ASS warnings, exact stream-copy maps and MP4/MOV/M4A mov_text override,
ISO639 first-two-character lookup and name/title metadata. It removes old subtitle
streams before mapping selected files. The complete 188-entry pinned table and
12 prefixed/case/unknown edges are compared, not inferred from a small locale set.
Source control-bearing names remain recorded as source-only native-boundary cases.

Actual parse_options, FFmpegEmbedSubtitlePP.run and YoutubeDL.run_pp execute in
an offline virtual-files worker. Fifty-seven shared plans and six source-only
cases preserve exact calls, warnings/messages, replacements, deletion arrays and
keep/file-move bookkeeping. Two hundred language projections and the actual
rejected --embed-subtitles alias are also compared. Python is development/
reference tooling only; the worker forbids network/process/nonvirtual mutation.

CLI behavior preserves implied manual acquisition unless automatic writing is
enabled. already_have_subtitle is determined from the original manual write flag,
before implication; auto-only explicit writing does not imply caption retention.
Global keep suppresses deletion. Conversion runs before media, then audio/video
processors, then embedding; converted metadata drops the original name as source
does. Skip-download retains acquired sidecars without embedding/deletion.

Native orchestration snapshots eligible local sidecars into the private workspace,
with 16 MiB per-file/256 MiB total bounds. The FFmpeg runner uses explicit local
media/sidecar demuxers and output muxers, file-only protocols, faststart, closed
stdin, bounded pipes, version validation and a 600-second shared deadline.
Metadata/track limits and unsupported active formats return explicit errors.
Missing/filtered/unsupported-container skips remain visible and never inspect tools.

Embedding replaces the current private media stage, as source replaces in place;
no redundant unembedded media copy is retained. Earlier audio/video inputs retain
existing keep behavior. Eligible sidecars are removed only after successful final
no-clobber publication. Keep flags suppress removal; skipped files are untouched.
Canonical duplicate inputs are cleaned once and source identity checks avoid
removing ordinarily changed captions. Cleanup failure returns an error after
media publication, leaving the changed source and suppressing archive recording.
This ordering and native no-clobber/source-check policies are not a transaction
across media, sidecars and archive state.

Eight native model/runner tests and sixteen localhost/API/CLI cases cover paths,
laziness, manual/automatic option precedence, conversion/remux composition,
retention/failure continuation, archive aliases, cancellation, source changes,
input budgets and publication races. Configured real FFmpeg/FFprobe verify
MP4/MKV/WebM media/subtitle codecs, language/name metadata, cue text and replacement
of existing subtitle tracks. Required tests have no live-site dependency; Linux
real-media CI runs these checks.

Remaining scope includes manifest/other-site caption acquisition,
additional active input formats, arbitrary postprocessor arguments, broader
codec/container guarantees and general postprocessing parity. See
[compatibility details](../../docs/COMPATIBILITY.md#subtitle-embedding) and
[fixture provenance](../../tests/fixtures/subtitle_embedding/README.md).

```sh
python3 tools/compare_subtitle_embedding.py --upstream /path/to/yt-dlp
```
