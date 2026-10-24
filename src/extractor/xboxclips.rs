//! Upstream `XboxClipsIE` (`yt_dlp/extractor/xboxclips.py`): a gameclips.io
//! clip page's first HTML5 media entry with its title, recording date, size,
//! duration and views.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct XboxClips;

const NAME: &str = "XboxClips";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?:xboxclips\.com|gameclips\.io)/(?:video\.php\?.*vid=|[^/]+/)(?P<id>[\da-f]{8}-(?:[\da-f]{4}-){3}[\da-f]{12})",
    )
    .unwrap()
});
static RECORDED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r">Recorded: (\d{2})-(Jan|Feb|Mar|Apr|May|Ju[nl]|Aug|Sep|Oct|Nov|Dec)-(\d{4})")
        .unwrap()
});
static SIZE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r">Size: ([^<]+)<").unwrap());
static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r">Duration: (\d+) Seconds<").unwrap());
static VIEWS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r">Views: (\d+)<").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Upstream `parse_filesize` (`lookup_unit_table`, non-strict): a leading
/// number (`,` or `.` decimals) and a unit, rounded half to even.
pub(crate) fn parse_filesize(text: &str) -> Option<i64> {
    const UNITS: &[(&str, f64)] = &[
        ("B", 1.0),
        ("b", 1.0),
        ("bytes", 1.0),
        ("KiB", 1024.0),
        ("KB", 1e3),
        ("kB", 1024.0),
        ("Kb", 1e3),
        ("kb", 1e3),
        ("kilobytes", 1e3),
        ("kibibytes", 1024.0),
    ];
    const PREFIXES: &[(char, i32)] = &[
        ('M', 2),
        ('G', 3),
        ('T', 4),
        ('P', 5),
        ('E', 6),
        ('Z', 7),
        ('Y', 8),
    ];
    const NAMES: &[(&str, &str)] = &[
        ("mega", "mebi"),
        ("giga", "gibi"),
        ("tera", "tebi"),
        ("peta", "pebi"),
        ("exa", "exbi"),
        ("zetta", "zebi"),
        ("yotta", "yobi"),
    ];
    static PATTERN: LazyLock<(Regex, Vec<(String, f64)>)> = LazyLock::new(|| {
        let mut table: Vec<(String, f64)> = UNITS
            .iter()
            .map(|(unit, factor)| ((*unit).to_owned(), *factor))
            .collect();
        for (&(prefix, power), (decimal, binary)) in PREFIXES.iter().zip(NAMES) {
            let lower = prefix.to_ascii_lowercase();
            let (si, iec) = (1000f64.powi(power), 1024f64.powi(power));
            for (unit, factor) in [
                (format!("{prefix}iB"), iec),
                (format!("{prefix}B"), si),
                (format!("{lower}B"), iec),
                (format!("{prefix}b"), si),
                (format!("{lower}b"), si),
                (format!("{decimal}bytes"), si),
                (format!("{binary}bytes"), iec),
            ] {
                table.push((unit, factor));
            }
        }
        let units = table
            .iter()
            .map(|(unit, _)| regex::escape(unit))
            .collect::<Vec<_>>()
            .join("|");
        (
            Regex::new(&format!(r"^(\d+(?:[,.]\d+)?)\s*({units})\b")).unwrap(),
            table,
        )
    });
    let (pattern, table) = &*PATTERN;
    let found = pattern.captures(text)?;
    let number: f64 = found[1].replace(',', ".").parse().ok()?;
    let factor = table.iter().find(|(unit, _)| unit == &found[2])?.1;
    Some((number * factor).round_ties_even() as i64)
}

/// `month_by_abbreviation`.
fn month(abbreviation: &str) -> Option<u32> {
    [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|name| *name == abbreviation)
    .map(|index| index as u32 + 1)
}

/// A non-fatal `_html_search_regex`: the cleaned group, else a warning.
fn optional(pattern: &Regex, html: &str, name: &str) -> Option<String> {
    let value = pattern
        .captures(html)
        .map(|found| crate::html::clean_html(&found[1]).trim().to_owned());
    if value.is_none() {
        search_warning(NAME, name);
    }
    value
}

/// `parse_qs(url)[name][0]`.
fn query(url: &Url, name: &str) -> Result<String> {
    url.query_pairs()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "An extractor error has occurred. (caused by KeyError('{name}')); {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

impl Extractor for XboxClips {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = if url.as_str().contains("/video.php") {
            Url::parse(&format!(
                "https://gameclips.io/{}/{}",
                query(url, "gamertag")
                    .map_err(|error| Error::Reported(format!("{id}: {error}")))?,
                query(url, "vid").map_err(|error| Error::Reported(format!("{id}: {error}")))?
            ))?
        } else {
            url.clone()
        };
        let webpage = download_webpage(client, &page, NAME, &id)?;
        let entry = super::html5::parse_html5_media_entries(
            client, &page, &webpage, &id, NAME, None, None,
        )?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Reported("list index out of range".into()))?;
        let title = crate::html::html_search_meta(&webpage, "og:title")
            .or_else(|| crate::html::html_search_meta(&webpage, "twitter:title"));
        if title.is_none() {
            search_warning(NAME, "og:title");
        }
        let upload_date = RECORDED.captures(&webpage).and_then(|found| {
            Some(format!(
                "{}{:02}{}",
                &found[3],
                month(&found[2])?,
                &found[1]
            ))
        });
        let filesize = optional(&SIZE, &webpage, "file size")
            .as_deref()
            .and_then(parse_filesize);
        let duration =
            optional(&DURATION, &webpage, "duration").and_then(|value| value.parse::<i64>().ok());
        let view_count =
            optional(&VIEWS, &webpage, "view count").and_then(|value| value.parse::<i64>().ok());
        let first = entry
            .formats
            .first()
            .map_or_else(|| page.clone(), |format| format.url.clone());
        let mut info = super::direct::build_at(first.clone(), &first, String::new());
        if entry.formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.formats = entry.formats;
        info.subtitles = entry.subtitles;
        info.metadata.thumbnail = entry.thumbnail;
        info.direct = None;
        info.id = id;
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        // Processing drops a `None` duration.
        let has_duration = duration.is_some();
        for (name, value) in [
            ("upload_date", upload_date.map(Value::from)),
            ("filesize_approx", filesize.map(Value::from)),
            ("duration", duration.map(Value::from)),
            ("view_count", view_count.map(Value::from)),
        ] {
            match value {
                Some(value) => crate::metadata_parser::set_field(&mut info, name, value),
                None if name == "duration" => {}
                None => {
                    info.extra.insert(name.into(), Value::Null);
                }
            }
        }
        let shape = format!(
            "dict;formats,subtitles,thumbnail,id,title,upload_date,filesize_approx,{}view_count",
            if has_duration { "duration," } else { "" }
        );
        Ok(Some(crate::info_order::mark_info(info, &shape)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://xboxclips.com/video.php?uid=2533274823424419&gamertag=Iabdulelah&vid=074a69a9-5faf-46aa-b93b-9909c1720325"
        ));
        assert!(suitable(
            "https://gameclips.io/iAbdulElah/074a69a9-5faf-46aa-b93b-9909c1720325"
        ));
        assert!(!suitable("https://gameclips.io/iAbdulElah/"));
    }

    #[test]
    fn file_sizes_follow_upstream_parse_filesize() {
        assert_eq!(parse_filesize("26.8 MB"), Some(26_800_000));
        assert_eq!(parse_filesize("5 bytes"), Some(5));
        assert_eq!(parse_filesize("1,5 GiB"), Some(1_610_612_736));
        assert_eq!(parse_filesize("2 kB"), Some(2048));
        assert_eq!(parse_filesize("about 3 MB"), None);
    }
}
