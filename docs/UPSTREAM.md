# Tracking and adapting upstream

Reference repository: https://github.com/yt-dlp/yt-dlp

The source baseline is release `2026.10.26`,
`f264a6cbd1f630894da0f368df00f8aae50140e0`. The initial baseline was the
development commit `897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026-10-10,
embedded version `2026.08.19`); the commits up to release `2026.10.21`
(`6ccd977303412151ee23352c6257dd658ffc1987`) are classified in
[upstream/notes/upstream-2026.10.21.md](../upstream/notes/upstream-2026.10.21.md),
those up to the development commit `fa9886678eb65103303c627b8e013f13f8999b41`
in [upstream/notes/upstream-fa98866.md](../upstream/notes/upstream-fa98866.md),
and those up to release `2026.10.26` in
[upstream/notes/upstream-2026.10.26.md](../upstream/notes/upstream-2026.10.26.md). Upstream is reference material, not a
runtime dependency. The source clone is not vendored into this repository.

The first slice was informed by upstream's `GenericIE` direct-response detection,
`InfoExtractor` result conventions, `YoutubeDL` output behavior, and `HttpFD`'s
separation from extraction. Rust behavior is intentionally narrower. For example,
the current implementation neither probes arbitrary binary bodies nor supports
upstream's full format/result dictionaries, resumption, or retry policy.

## Change report

```sh
git clone https://github.com/yt-dlp/yt-dlp.git .upstream/yt-dlp
python3 tools/upstream_report.py .upstream/yt-dlp --target HEAD > /tmp/upstream-report.json
```

The tool reads the source mapping in `upstream/manifest.json`. The default baseline
is its immutable commit SHA. `--target` accepts a local Git revision; the report
resolves it to a SHA. `--base` overrides the comparison baseline for investigation,
without modifying the manifest. Both commits must exist locally, and the baseline
must be an ancestor of the target. The tool never fetches, checks out files,
executes upstream code, changes the working tree, or advances the baseline.

Report fields:

- `baseline` and `target`: exact source revisions.
- `commits`: intervening commit SHAs and subjects.
- `changed_files`: all changed paths, including deleted files. Renames are reported
  as deletion/addition to keep both old and new source mappings visible.
- `affected_components`: component status, mapped Rust files, current scope, and
  relevant upstream paths. `partial` does not imply full-file compatibility.
- `unmapped_files`: changes requiring triage rather than silently ignored work.

An empty report at the pinned baseline only verifies that the reporter can read
the checkout. It is not a parity check. Tests exercise real local Git history
with changed/deleted files and unmapped paths.

## Future update agent

A baseline advance is due within a week of each upstream release.
`.github/workflows/upstream.yml` checks daily: `tools/upstream_watch.py` finds
the newest `YYYY.MM.DD[.N]` release tag in a blobless upstream clone and, when
the manifest's baseline does not contain it, keeps one issue labelled
`upstream` open with the exact target SHA and this report's commits, affected
components (core and per-extractor) and unmapped files. The job fails once
the release is more than seven days old and closes the issue when the
baseline contains it. It never ports, edits the manifest or opens PRs; the
issue is the brief for an agent (or person) together with `AGENTS.md`.

```sh
python3 tools/upstream_watch.py .upstream/yt-dlp --report /tmp/upstream-watch.json
```

No agent service, credentials, or automatic PR/merge machinery is configured.

For each update, the agent should:

1. Read the actual upstream patches and classify each change as applicable to a
   supported feature, a prerequisite, a deferred feature, or unrelated maintenance.
2. Port applicable changes in small reviewable units. Include source commit links
   and saved regression fixtures. Changes to shared helpers may affect many sites;
   source mapping alone is not sufficient impact analysis.
3. Run deterministic Rust/CLI tests, tool tests, and, when available, differential
   upstream metadata tests against identical fixtures. Optional live smoke tests
   should remain separate from required CI.
4. Prepare a PR describing changed behavior, validation, and known gaps. Record
   deferrals and unresolved compatibility differences.
5. Advance the baseline only once all intervening changes are accounted for.
   Never advance it merely because a scheduled report or CI job succeeded.
   With the new baseline, regenerate the extractor order that the archive's
   URL fallback searches: `tools/extractor_keys.py --upstream DIR --write`
   (without `--write` it checks `src/data/extractor_keys.tsv`). Then run
   `tools/extractor_tiers.py --survey --upstream DIR`: an extractor upstream
   newly marks `_WORKING = False` gets the same mark and warning here (with
   a fixture), one upstream removed is removed here with its fixtures and
   manifest entry, and the descriptions must match
   ([extractor tiers](../upstream/notes/extractor-tiers.md)). Regenerate
   `docs/SITES.md` with `tools/extractor_tiers.py --write`.

Before enabling unattended updates, implement the differential harness, persistent
triage/update notes, PR/review integration, and release rollback/versioning. The
current report plus coverage ledger is the input layer for that system.
