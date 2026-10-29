//! Upstream `DetikEmbedIE` (`yt_dlp/extractor/detik.py`): no URLs of its own
//! (`_VALID_URL = False`). The generic extractor's embed detection finds a
//! page's `flowplayer` or `detikVideo` player script and gives its HLS
//! formats with the page's JSON-LD and meta-tag metadata, as one entry.
use super::html5::WebpageEntry;
use crate::{Error, Result};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

/// The extractor's `IE_NAME`; errors from the generic extractor's embed scan
/// use `generic` instead, as upstream's `_search_regex` calls do.
pub(crate) const NAME: &str = "DetikEmbed";

static PLAYER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<script\s*[^>]+src="https?://(aws)?cdn\.detik\.net\.id/(?P<type>flowplayer|detikVideo)[^>]+>\s*(?P<video_data>\{[^}]+\})"#,
    )
    .unwrap()
});
static IDENTIFIER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"identifier\s*:\s*'([^']+)").unwrap());
static VIDEO_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"videoUrl\s*:\s*['"]?([^"']+)"#).unwrap());
static IMAGE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"imageUrl\s*:\s*['"]?([^"']+)"#).unwrap());
static AUTHOR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([^-]+)").unwrap());

/// The keys `json_ld::search` and this module can yield: a
/// [`WebpageEntry`]'s fields have static names.
const FIELD_KEYS: [&str; 34] = [
    "abr",
    "artist",
    "average_rating",
    "chapters",
    "description",
    "display_id",
    "duration",
    "end_time",
    "episode",
    "episode_number",
    "ext",
    "extractor",
    "extractor_key",
    "filesize",
    "formats",
    "height",
    "id",
    "release_timestamp",
    "season",
    "season_number",
    "series",
    "start_time",
    "subtitles",
    "tags",
    "tbr",
    "thumbnail",
    "thumbnails",
    "timestamp",
    "title",
    "uploader",
    "url",
    "vcodec",
    "view_count",
    "width",
];

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Detik {what} are not supported natively"))
}

fn static_key(key: &str) -> Result<&'static str> {
    FIELD_KEYS
        .iter()
        .copied()
        .find(|known| *known == key)
        .ok_or_else(|| unsupported(&format!("JSON-LD field {key}")))
}

/// `_search_regex(pattern, text, name)` (fatal, from the generic extractor).
fn required(pattern: &Regex, text: &str, name: &str) -> Result<String> {
    pattern
        .captures(text)
        .map(|found| found[1].to_owned())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[generic] Unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

/// `_html_search_meta(names, webpage)` without a warning.
fn meta(webpage: &str, names: &[&str]) -> Option<String> {
    names
        .iter()
        .find_map(|name| crate::html::html_search_meta(webpage, name))
}

/// [`meta`], warning about the first name when it is missing.
fn meta_warn(webpage: &str, names: &[&str]) -> Option<String> {
    let found = meta(webpage, names);
    if found.is_none() {
        super::search_warning(NAME, names[0]);
    }
    found
}

/// `int_or_none(value, scale)`: a whole number divided by `scale`.
fn int_or_none(text: Option<String>, scale: i64) -> Value {
    text.and_then(|text| text.trim().parse::<i64>().ok())
        .map_or(Value::Null, |value| Value::from(value.div_euclid(scale)))
}

/// `merge_dicts`: the first non-null value per key (an empty string yields
/// to a later non-empty one).
fn merge_into(merged: &mut Vec<(String, Value)>, dict: Vec<(String, Value)>) {
    for (key, value) in dict {
        if value.is_null() {
            continue;
        }
        match merged.iter_mut().find(|(known, _)| *known == key) {
            None => merged.push((key, value)),
            Some((_, existing)) => {
                if existing.as_str() == Some("") && value.is_string() {
                    *existing = value;
                }
            }
        }
    }
}

/// `_extract_from_webpage`: the page's player (if any) as one info dict.
pub(crate) fn webpage_entries(
    client: &Client,
    url: &Url,
    webpage: &str,
) -> Result<Vec<WebpageEntry>> {
    let Some(found) = PLAYER.captures(webpage) else {
        return Ok(Vec::new());
    };
    let player_type = found["type"].to_owned();
    let video_data = found["video_data"].to_owned();
    let display_id = crate::info::url_basename(url);

    let mut extra: Vec<(String, Value)> = Vec::new();
    let video_url = if player_type == "flowplayer" {
        let video: Value = serde_json::from_str(&video_data.replace('\'', "\"")).map_err(|_| {
            Error::Reported(format!("[generic] {display_id}: Failed to parse JSON"))
        })?;
        let video_url = video
            .get("videoUrl")
            .and_then(Value::as_str)
            .ok_or_else(|| super::clyp::key_error(&display_id, "videoUrl"))?
            .to_owned();
        let identifier = required(&IDENTIFIER, webpage, "identifier")?;
        extra.push(("id".into(), identifier.into()));
        extra.push((
            "thumbnail".into(),
            video.get("imageUrl").cloned().unwrap_or(Value::Null),
        ));
        video_url
    } else {
        let video_url = required(&VIDEO_URL, &video_data, "videoUrl")?;
        let video_id = meta_warn(webpage, &["video_id", "dtk:video_id"]);
        let thumbnail = required(&IMAGE_URL, &video_data, "videoUrl")?;
        let author = meta(webpage, &["dtk:author"]).unwrap_or_default();
        let uploader = AUTHOR.captures(&author).map(|found| found[1].to_owned());
        extra.push(("id".into(), video_id.map_or(Value::Null, Value::from)));
        extra.push(("thumbnail".into(), thumbnail.into()));
        extra.push((
            "duration".into(),
            int_or_none(meta(webpage, &["duration"]), 1),
        ));
        extra.push((
            "release_timestamp".into(),
            int_or_none(meta(webpage, &["dtk:publishdateunix"]), 1000),
        ));
        extra.push((
            "timestamp".into(),
            int_or_none(meta(webpage, &["dtk:createdateunix"]), 1000),
        ));
        extra.push(("uploader".into(), uploader.map_or(Value::Null, Value::from)));
        video_url
    };

    let info = super::extract_m3u8_fatal_master(
        client,
        &Url::parse(&video_url)?,
        NAME,
        &display_id,
        None,
        true,
    )?;
    if !info.subtitles.is_empty() {
        return Err(unsupported("subtitle tracks in player playlists"));
    }
    let formats = info.formats;

    let title = meta_warn(webpage, &["og:title", "originalTitle"])
        .or_else(|| crate::html::html_extract_title(webpage));
    let description = meta_warn(
        webpage,
        &["og:description", "twitter:description", "description"],
    );
    let tags = meta_warn(webpage, &["keywords", "keyword", "dtk:keywords"])
        .map(|keywords| Value::Array(keywords.split(',').map(Value::from).collect()));
    let json_ld = super::json_ld::search(webpage);

    let mut merged = Vec::new();
    merge_into(&mut merged, json_ld.into_iter().collect());
    merge_into(&mut merged, extra);
    // The final dict in its order; `formats` and `subtitles` keep their
    // places (the formats and subtitles are set on the info below).
    let final_dict = [
        ("display_id", Some(Value::from(display_id.as_str()))),
        ("title", title.map(Value::from)),
        ("description", description.map(Value::from)),
        ("formats", None),
        ("subtitles", None),
        ("tags", tags),
    ];
    for (key, value) in final_dict {
        match (key, value) {
            ("formats", _) | ("subtitles", _) => {
                if !merged.iter().any(|(known, _)| known == key) {
                    let placeholder = if key == "formats" {
                        Value::Null
                    } else {
                        Value::Object(Default::default())
                    };
                    merged.push((key.to_owned(), placeholder));
                }
            }
            (key, Some(value)) => merge_into(&mut merged, vec![(key.to_owned(), value)]),
            (_, None) => {}
        }
    }
    let mut fields = Vec::with_capacity(merged.len() + 2);
    for (key, value) in merged {
        fields.push((static_key(&key)?, value));
    }
    fields.push(("extractor", NAME.into()));
    fields.push(("extractor_key", NAME.into()));
    Ok(vec![WebpageEntry {
        extractor: (NAME, NAME),
        fields,
        formats,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn player_regex_finds_detik_video_scripts() {
        let html = "<script type=\"text/javascript\" src=\"https://awscdn.detik.net.id/detikVideo/detikVideo.core.js?v=1\">\n{ videoUrl : 'https://vod.example/x.m3u8' }</script>";
        let found = PLAYER.captures(html).unwrap();
        assert_eq!(&found["type"], "detikVideo");
        assert!(
            PLAYER
                .captures("<script src=\"https://example.com/x.js\">{}</script>")
                .is_none()
        );
    }

    #[test]
    fn merge_keeps_first_non_null_and_upgrades_empty_strings() {
        let mut merged = Vec::new();
        merge_into(
            &mut merged,
            vec![
                ("title".into(), Value::from("")),
                ("duration".into(), Value::Null),
            ],
        );
        merge_into(
            &mut merged,
            vec![
                ("title".into(), Value::from("Video")),
                ("duration".into(), Value::from(83)),
                ("duration".into(), Value::from(99)),
            ],
        );
        assert_eq!(
            merged,
            vec![
                ("title".to_owned(), Value::from("Video")),
                ("duration".to_owned(), Value::from(83)),
            ]
        );
    }
}
