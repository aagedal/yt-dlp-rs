//! Descriptive metadata from one player response, following upstream
//! `YoutubeIE._real_extract` for fields derivable without `ytInitialData`,
//! watch-page meta tags or thumbnails.

use crate::Metadata;
use serde_json::Value;

/// Upstream `_get_text`: `simpleText`, else the joined `runs[].text`.
pub(super) fn text(value: &Value) -> Option<String> {
    if let Some(text) = value.get("simpleText").and_then(Value::as_str)
        && !text.is_empty()
    {
        return Some(text.to_owned());
    }
    let runs = value
        .get("runs")
        .and_then(Value::as_array)
        .or_else(|| value.as_array())?;
    let joined: String = runs
        .iter()
        .filter_map(|run| run.get("text").and_then(Value::as_str))
        .collect();
    (!joined.is_empty()).then_some(joined)
}

/// Python `int_or_none` for JSON integers and integer strings.
fn int(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number.as_i64().or_else(|| {
            number
                .as_f64()
                .filter(|v| v.is_finite())
                .map(|v| v.trunc() as i64)
        }),
        Value::String(text) => crate::batch::py_strip(text).parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

fn present(value: &Value) -> Option<&Value> {
    (!value.is_null()).then_some(value)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `^(UC[\w-]{22})$`.
fn ucid(value: &str) -> Option<String> {
    let rest = value.strip_prefix("UC")?;
    (rest.chars().count() == 22 && rest.chars().all(|c| is_word(c) || c == '-'))
        .then(|| value.to_owned())
}

/// `handle_from_url`: `^(?:https?://(?:www\.)?youtube\.com)?/(@[\w.-]{3,30})`.
fn handle(url: &str) -> Option<String> {
    let decoded = percent_encoding::percent_decode_str(url).decode_utf8_lossy();
    let mut rest = decoded.as_ref();
    for prefix in ["https://", "http://"] {
        if let Some(tail) = rest.strip_prefix(prefix) {
            let tail = tail.strip_prefix("www.").unwrap_or(tail);
            rest = tail.strip_prefix("youtube.com")?;
            break;
        }
    }
    let name = rest.strip_prefix("/@")?;
    let length = name
        .chars()
        .take_while(|&c| is_word(c) || c == '.' || c == '-')
        .take(30)
        .count();
    (length >= 3).then(|| format!("@{}", name.chars().take(length).collect::<String>()))
}

/// Days since 1970-01-01 for a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
}

fn valid_date(year: i64, month: i64, day: i64) -> bool {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    (1..=12).contains(&month) && day >= 1 && day <= days[(month - 1) as usize]
}

/// The microformat `uploadDate` subset: `YYYY-MM-DD[THH:MM:SS[.f+][Z|±HH[:]MM]]`.
/// A timestamp requires both time and zone (`parse_iso8601(timezone=NO_DEFAULT)`);
/// otherwise the date part is the upload date (`unified_strdate`).
fn upload_time(value: &str) -> (Option<String>, Option<i64>) {
    let bytes = value.as_bytes();
    let digits = |range: std::ops::Range<usize>| -> Option<i64> {
        let part = value.get(range)?;
        part.bytes()
            .all(|b| b.is_ascii_digit())
            .then(|| part.parse().ok())?
    };
    let (Some(year), Some(month), Some(day)) = (digits(0..4), digits(5..7), digits(8..10)) else {
        return (None, None);
    };
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') || !valid_date(year, month, day) {
        return (None, None);
    }
    let date = format!("{year:04}{month:02}{day:02}");
    if bytes.len() == 10 {
        return (Some(date), None);
    }
    let time = (bytes.get(10) == Some(&b'T'))
        .then(|| Some((digits(11..13)?, digits(14..16)?, digits(17..19)?)))
        .flatten()
        .filter(|_| bytes.get(13) == Some(&b':') && bytes.get(16) == Some(&b':'));
    let Some((hour, minute, second)) = time.filter(|(h, m, s)| *h < 24 && *m < 60 && *s < 60)
    else {
        return (None, None);
    };
    let mut rest = &value[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let length = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if length == 0 {
            return (None, None);
        }
        rest = &fraction[length..];
    }
    let offset = match rest {
        "" => None,
        "Z" => Some(0),
        zone => {
            let sign = match zone.as_bytes()[0] {
                b'+' => 1,
                b'-' => -1,
                _ => return (None, None),
            };
            let zone = &zone[1..];
            let (hours, minutes) = match zone.len() {
                5 if zone.as_bytes()[2] == b':' => (&zone[..2], &zone[3..]),
                4 => (&zone[..2], &zone[2..]),
                _ => return (None, None),
            };
            let (Ok(hours), Ok(minutes)) = (hours.parse::<i64>(), minutes.parse::<i64>()) else {
                return (None, None);
            };
            Some(sign * (hours * 3600 + minutes * 60))
        }
    };
    let Some(offset) = offset else {
        return (Some(date), None);
    };
    let timestamp =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset;
    let (year, month, day) = civil_from_days(timestamp.div_euclid(86_400));
    (
        Some(format!("{year:04}{month:02}{day:02}")),
        Some(timestamp),
    )
}

/// Upstream `url_or_none` scheme check.
fn url_or_none(url: &str) -> Option<&str> {
    let url = url.trim();
    let rest = &url[..url.find("//")?];
    let scheme = rest.strip_suffix(':').unwrap_or(rest);
    let known = [
        "", "http", "https", "rtmp", "rtmpt", "rtmpe", "rtmps", "rtmpte", "rtmpts", "rtmfp",
        "rtsp", "rtsps", "rtspu", "mms", "ftp", "ftps", "ws", "wss",
    ];
    (known.contains(&scheme.to_ascii_lowercase().as_str())
        && (rest.is_empty() || rest.ends_with(':')))
    .then_some(url)
}

const THUMBNAIL_NAMES: [&str; 19] = [
    "maxresdefault",
    "hq720",
    "sddefault",
    "hqdefault",
    "0",
    "mqdefault",
    "default",
    "sd1",
    "sd2",
    "sd3",
    "hq1",
    "hq2",
    "hq3",
    "mq1",
    "mq2",
    "mq3",
    "1",
    "2",
    "3",
];

/// Upstream thumbnail candidates: saved `videoDetails`/microformat thumbnails,
/// then generated `i.ytimg.com` names, with preferences and URL deduplication.
/// Returns the deduplicated list and the best saved thumbnail URL.
fn thumbnails(player: &Value, id: &str) -> (Vec<crate::Thumbnail>, Option<String>) {
    let mut saved = Vec::new();
    for root in [
        "/videoDetails/thumbnail/thumbnails",
        "/microformat/playerMicroformatRenderer/thumbnail/thumbnails",
    ] {
        for thumbnail in player
            .pointer(root)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(url) = thumbnail
                .get("url")
                .and_then(Value::as_str)
                .and_then(url_or_none)
            else {
                continue;
            };
            let url = if url.contains("maxresdefault") {
                url.split('?').next().unwrap_or(url)
            } else {
                url
            };
            let dimension = |key: &str| {
                thumbnail
                    .get(key)
                    .and_then(int)
                    .and_then(|v| u32::try_from(v).ok())
            };
            saved.push(crate::Thumbnail {
                url: url.to_owned(),
                width: dimension("width"),
                height: dimension("height"),
                ..Default::default()
            });
        }
    }
    let preference = |url: &str| {
        let index = THUMBNAIL_NAMES
            .iter()
            .position(|name| url.contains(&format!("/{id}/{name}")))
            .unwrap_or(THUMBNAIL_NAMES.len()) as i64;
        i64::from(if url.contains(".webp") { 0 } else { -1 }) - 2 * index
    };
    for thumbnail in &mut saved {
        thumbnail.preference = Some(preference(&thumbnail.url));
    }
    let mut best = saved.clone();
    // `_sort_thumbnails` on the saved copy: the last is the best known thumbnail.
    best.sort_by(|a, b| {
        let key = |t: &crate::Thumbnail| {
            (
                t.preference.unwrap_or(-1),
                t.width.map_or(-1, i64::from),
                t.height.map_or(-1, i64::from),
            )
        };
        key(a).cmp(&key(b)).then_with(|| a.url.cmp(&b.url))
    });
    let thumbnail = best.last().map(|t| t.url.clone());
    let mut all = saved;
    for name in THUMBNAIL_NAMES {
        for ext in ["webp", "jpg"] {
            let webp = if ext == "webp" { "_webp" } else { "" };
            let url = format!("https://i.ytimg.com/vi{webp}/{id}/{name}.{ext}");
            all.push(crate::Thumbnail {
                preference: Some(preference(&url)),
                url,
                ..Default::default()
            });
        }
    }
    let mut seen = std::collections::HashSet::new();
    all.retain(|thumbnail| seen.insert(thumbnail.url.clone()));
    (all, thumbnail)
}

/// Metadata for the supported (non-live) player responses.
pub(super) fn parse(player: &Value) -> Metadata {
    let details = &player["videoDetails"];
    let id = details["videoId"].as_str().unwrap_or_default();
    let (thumbnails, thumbnail) = thumbnails(player, id);
    let micro = player
        .pointer("/microformat/playerMicroformatRenderer")
        .unwrap_or(&Value::Null);
    let description = match present(&details["shortDescription"]) {
        Some(original) => original.as_str().map(str::to_owned),
        None => text(&micro["description"]),
    };
    let duration = [&details["lengthSeconds"], &micro["lengthSeconds"]]
        .into_iter()
        .filter_map(int)
        .find(|&seconds| seconds != 0)
        .and_then(|seconds| u64::try_from(seconds).ok());
    let channel_id = present(&details["channelId"])
        .or_else(|| present(&micro["externalChannelId"]))
        .and_then(Value::as_str)
        .and_then(ucid);
    let view_count = present(&details["viewCount"])
        .or_else(|| present(&micro["viewCount"]))
        .and_then(int)
        .and_then(|count| u64::try_from(count).ok());
    let age_limit = if micro["isFamilySafe"] == Value::Bool(false) {
        18
    } else {
        0
    };
    let category = micro["category"].as_str().filter(|c| !c.is_empty());
    let tags = details["keywords"]
        .as_array()
        .map(|keywords| {
            keywords
                .iter()
                .filter_map(|keyword| keyword.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let is_live = present(&details["isLive"]).or_else(|| {
        player
            .pointer("/microformat/playerMicroformatRenderer/liveBroadcastDetails/isLiveNow")
            .and_then(present)
    });
    let live_content = present(&details["isLiveContent"]);
    let live_status = [is_live, live_content]
        .into_iter()
        .any(|value| value == Some(&Value::Bool(false)))
        .then(|| "not_live".to_owned());
    let media_type = if micro["isShortsEligible"].as_bool() == Some(true) {
        "short"
    } else {
        "video"
    };
    let uploader_id = micro["ownerProfileUrl"].as_str().and_then(handle);
    let (upload_date, timestamp) = micro["uploadDate"]
        .as_str()
        .map_or((None, None), upload_time);
    // `_availability` without initial-data badges: only private, sign-in and
    // unlisted states can be known.
    let is_private = details["isPrivate"].as_bool();
    let availability = if is_private == Some(true) {
        Some("private")
    } else if age_limit >= 18 {
        Some("needs_auth")
    } else if is_private.is_some() && micro["isUnlisted"].as_bool() == Some(true) {
        Some("unlisted")
    } else {
        None
    };
    Metadata {
        description,
        duration: duration.map(|duration| crate::info::PyNumber::Int(duration as i64)),
        channel_url: channel_id
            .as_ref()
            .map(|id| format!("https://www.youtube.com/channel/{id}")),
        channel_id,
        uploader_url: uploader_id
            .as_ref()
            .map(|handle| format!("https://www.youtube.com/{handle}")),
        uploader_id,
        view_count,
        average_rating: details["averageRating"].as_f64(),
        age_limit: Some(age_limit),
        categories: category.map(|category| vec![category.to_owned()]),
        tags: Some(tags),
        playable_in_embed: player
            .pointer("/playabilityStatus/playableInEmbed")
            .and_then(Value::as_bool),
        live_status,
        media_type: Some(media_type.into()),
        upload_date,
        timestamp: timestamp.map(crate::info::PyNumber::Int),
        availability: availability.map(str::to_owned),
        thumbnail,
        thumbnails,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_dates_follow_timezone_and_precision_rules() {
        assert_eq!(
            upload_time("2008-05-29T00:00:00-07:00"),
            (Some("20080529".into()), Some(1_212_044_400))
        );
        assert_eq!(
            upload_time("2008-05-29T23:30:00.5-0100"),
            (Some("20080530".into()), Some(1_212_107_400))
        );
        assert_eq!(upload_time("2008-05-29"), (Some("20080529".into()), None));
        assert_eq!(
            upload_time("2008-05-29T10:00:00"),
            (Some("20080529".into()), None)
        );
        assert_eq!(upload_time("2008-02-30"), (None, None));
        assert_eq!(
            handle("http://www.youtube.com/@BlenderOfficial").as_deref(),
            Some("@BlenderOfficial")
        );
        assert_eq!(handle("/@ab"), None);
        assert_eq!(
            ucid("UCSMOQeBJ2RAnuFungnQOxLg").as_deref(),
            Some("UCSMOQeBJ2RAnuFungnQOxLg")
        );
        assert_eq!(ucid("UCshort"), None);
    }
}
