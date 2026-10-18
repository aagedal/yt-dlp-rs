# YouTube player-response metadata

Exact official source target and unchanged baseline:
`897341241f2bfb2439fe0367f4c7c9ccd29a3828` (2026.08.19). No intervening
upstream commits; the global baseline is unchanged.

Inspected `extractor/youtube/_video.py` `_real_extract` description/duration
selection (3983–4038), live status (`_list_formats`, 3863–3874), the info
dictionary (4166–4196), uploader/handle, upload date/timestamp and availability
updates (4523–4585); `_base.py` `_get_text`, `ucid_or_none`, `handle_from_url`,
`_YT_HANDLE_RE`/`_YT_CHANNEL_UCID_RE`; `common.py` `_availability`; and
`utils/_utils.py` `parse_iso8601`/`extract_timezone`. These also derive fields
from `ytInitialData` (channel/uploader names, likes, comments, chapters,
badges, heatmap), watch-page meta tags and generated/sorted thumbnails, none of
which is implied here.

Implemented: the listed fields from one player response (`videoDetails`,
`microformat.playerMicroformatRenderer`, `playabilityStatus`), with upstream's
`or` fallbacks, integer-string coercion, `UC` channel ID validation, owner
handle extraction, `not_live` only when an explicit false live flag exists,
`short` media type, `age_limit` 18 for `isFamilySafe: false`, and availability
without initial-data badges (so `public` is never inferred). `uploadDate` with
time and zone gives a timestamp and its UTC date; date-only or zone-less values
give only the date. Other date spellings are not parsed.

`tests/fixtures/youtube/metadata.json` is a rich saved response; all six saved
cases now compare these fields against actual upstream extraction via
`tools/compare_youtube.py` (missing values as null), and native fixture tests
consume the same expected files.
