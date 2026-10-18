# Download archive persistence

Exact official source target and unchanged global baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19).
Ran `tools/upstream_report.py` against the official local checkout with this exact
target: no intervening commits, changed files, affected components or unmapped
files. This adds a bounded existing behavior, without advancing the baseline.

Inspected `YoutubeDL.py` preload (842), archive entry matching (1649), early URL-ID
check (1713), final write flags (3130–3146), simulation (3374), success after
postprocessing/hooks (3674), identity/membership/record methods (3855–3895);
`utils/_utils.py` locked_file (1619) and make_archive_id (5316); archive/force-write
options (781/1322); and searched the pinned test tree for archive coverage.
`test/test_YoutubeDL.py` has no focused archive cases; `test/parameters.json`
only sets force-write false. New saved probes cover the source methods. These mapped
files also cover forced simulation writes, alternate IDs, fallback identity,
postprocessor hooks, general download errors and options outside this slice.
There are no intervening unmapped changes to classify. Native partial coverage is
mapped explicitly as download-archive rather than treating all of YoutubeDL as
ported.

Native `DownloadArchive` separates persistence from extraction and transport.
CLI filtering is after complete extraction/probing but before selection, so
recorded separate-only/unavailable-format entries skip without requiring a merger.
Remaining entries retain atomic selection. Recording follows final publication,
including explicit FFmpeg merges; failures never claim successful archive writes.
A failed archive update keeps completed media and produces a nonzero result.
Metadata/listing/simulation are read-only. Missing files are not created on load;
parent directories are never created. Invalid existing archives fail before HTTP.

Identity matches the supported source fields: lowercase extractor_key, space,
case-sensitive ID. Native validation rejects empty/outer-whitespace/control
components instead of using upstream's permissive fallback/legacy-ID paths.
Membership uses UTF-8 stripped lines with universal CR/LF newline handling.
Standard-library file locks have a bounded five-second wait, shared on load and
exclusive for reread/deduplicated append. Existing bytes remain intact, a missing
final newline is repaired, and partial append/synchronization failures attempt
rollback under the lock. These are explicit native policies, not byte-for-byte
upstream append or lock interoperability claims. Cached membership does not
serialize whole downloads or continuously watch other writers. Generic filename
IDs retain their collision risk; no URL hash or invented identity is introduced.

The shared saved fixtures compare actual source `_make_archive_id`, file/set
preload/membership, one Unicode append and four `process_video_result` flows.
Sockets, downloads and external processes are forbidden in reference probes.
They prove archived-before-selection, ordinary simulation/listing no-write and
reference-only forced simulation write, not full source download timing. Native
local tests separately prove transport/no-clobber/merger failures never record,
media survives archive-write failure, successful merges record only after final
publication, playlist filtering/atomicity, concurrent dedup and injected partial
append rollback. The fixture README records detailed source references and
excluded behavior.

```sh
python3 tools/compare_archive.py --upstream /path/to/yt-dlp
```

See [the compatibility contract](../../docs/COMPATIBILITY.md#download-archives).


Lock-lifetime follow-up: concurrent offline runtime tests exposed a retained Unix
flock after the archive-local descriptor closed. A duplicated or briefly inherited
fork/exec descriptor shares the open file description. Archive load/record now
use an owning guard that explicitly unlocks before closing on success, duplicate
fast paths and errors. A deterministic Unix regression retains a duplicate
through guard drop and verifies an independent writer can immediately lock; this
covers the lifetime issue without depending on a subprocess scheduling race.


Windows CI also exposed that metadata lookup below a regular-file ancestor can
report NotFound instead of Unix NotADirectory. Native missing-path handling now
checks the nearest existing ancestor is a directory, giving a consistent error
for invalid/child and deeper paths. Genuinely missing directory trees remain lazy
until recording; they are never created. This is explicit native path validation,
not a cross-platform equivalence claim for all upstream filesystem errors.
