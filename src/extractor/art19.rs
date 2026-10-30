//! Upstream `Art19IE` (`yt_dlp/extractor/art19.py`): an episode's direct
//! `rss.art19.com` audio plus the RSS feed's media renditions, with metadata
//! merged from the player and RSS JSON (both optional). `Art19ShowIE` is not
//! ported.
use super::{Extractor, dict_format, dict_info, download_json_note_headers};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Art19;

const NAME: &str = "Art19";
const UUID: &str = r"[\da-f]{8}-?[\da-f]{4}-?[\da-f]{4}-?[\da-f]{4}-?[\da-f]{12}";

static SHOW_EPISODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^https?://(?:www\.)?art19\.com/shows/[^/#?]+/episodes/(?P<id>{UUID})"
    ))
    .unwrap()
});
static RSS_EPISODE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^https?://rss\.art19\.com/episodes/(?P<id>{UUID})\.mp3"
    ))
    .unwrap()
});

/// `_match_id` over both `_VALID_URL` patterns.
fn episode_id(url: &str) -> Option<String> {
    SHOW_EPISODE
        .captures(url)
        .or_else(|| RSS_EPISODE.captures(url))
        .map(|found| found["id"].to_owned())
}

/// `_match_valid_url` for either pattern.
pub fn suitable(url: &str) -> bool {
    episode_id(url).is_some()
}

/// `_download_json(..., fatal=False)`: an HTTP error is a warning and `None`.
fn optional_json(
    client: &Client,
    url: &Url,
    id: &str,
    note: &str,
    headers: &[(&str, &str)],
) -> Result<Option<Value>> {
    match download_json_note_headers(client, url, NAME, id, note, headers) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// `traverse_obj(data, path, {str})`.
fn string(data: &Value, path: &str) -> Option<Value> {
    data.pointer(path)
        .and_then(Value::as_str)
        .map(|text| text.to_owned().into())
}

/// `traverse_obj(data, path, {int_or_none})`.
fn int(data: &Value, path: &str) -> Option<Value> {
    match data.pointer(path)? {
        Value::Number(number) => number.as_i64().map(Value::from),
        Value::String(text) => text.trim().parse::<i64>().ok().map(Value::from),
        _ => None,
    }
}

/// `traverse_obj(data, path, {float_or_none})`.
fn float(data: &Value, path: &str) -> Option<Value> {
    match data.pointer(path)? {
        Value::Number(number) => number.as_f64().map(Value::from),
        Value::String(text) => text.trim().parse::<f64>().ok().map(Value::from),
        _ => None,
    }
}

/// `traverse_obj(data, path, {parse_iso8601})`.
fn timestamp(data: &Value, path: &str) -> Option<Value> {
    data.pointer(path)
        .and_then(Value::as_str)
        .and_then(crate::dates::parse_iso8601)
        .map(Value::from)
}

/// `traverse_obj(data, path, {url_or_none})`.
fn url_value(data: &Value, path: &str) -> Option<Value> {
    data.pointer(path)
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map(Value::from)
}

/// Python's `dict` update over the `**traverse_obj(...)` spreads: a key keeps
/// its first position and takes the latest non-`None` value.
fn merge(target: &mut Vec<(&'static str, Value)>, fields: Vec<(&'static str, Option<Value>)>) {
    for (key, value) in fields {
        let Some(value) = value else { continue };
        match target.iter_mut().find(|(name, _)| *name == key) {
            Some(slot) => slot.1 = value,
            None => target.push((key, value)),
        }
    }
}

impl Extractor for Art19 {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(episode_id) = episode_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        let player_url = Url::parse(&format!("https://art19.com/episodes/{episode_id}"))?;
        let player_metadata = optional_json(
            client,
            &player_url,
            &episode_id,
            "Downloading player metadata",
            &[("Accept", "application/vnd.art19.v0+json")],
        )?;
        let rss_url = Url::parse(&format!("https://rss.art19.com/episodes/{episode_id}.json"))?;
        let rss_metadata = optional_json(
            client,
            &rss_url,
            &episode_id,
            "Downloading RSS metadata",
            &[],
        )?;

        let direct_url = format!("https://rss.art19.com/episodes/{episode_id}.mp3");
        let mut formats = vec![dict_format(
            vec![
                ("format_id", "direct".into()),
                ("url", direct_url.into()),
                ("vcodec", "none".into()),
                ("acodec", "mp3".into()),
            ],
            NAME,
            &episode_id,
        )?];
        if let Some(media) = rss_metadata
            .as_ref()
            .and_then(|rss| rss.pointer("/content/media"))
            .and_then(Value::as_object)
        {
            for (fmt_id, fmt_data) in media {
                if fmt_id == "waveform_bin" {
                    continue;
                }
                let Some(fmt_url) = url_value(fmt_data, "/url") else {
                    continue;
                };
                let quality: i64 = if fmt_id == "ogg" { -2 } else { -1 };
                formats.push(dict_format(
                    vec![
                        ("format_id", fmt_id.as_str().into()),
                        ("url", fmt_url),
                        ("vcodec", "none".into()),
                        ("acodec", fmt_id.as_str().into()),
                        ("quality", quality.into()),
                    ],
                    NAME,
                    &episode_id,
                )?);
            }
        }

        let mut merged: Vec<(&'static str, Value)> = Vec::new();
        if let Some(player) = &player_metadata {
            merge(
                &mut merged,
                vec![
                    ("title", string(player, "/episode/title")),
                    ("description", string(player, "/episode/description_plain")),
                    ("episode_id", string(player, "/episode/id")),
                    ("episode_number", int(player, "/episode/episode_number")),
                    ("season_id", string(player, "/episode/season_id")),
                    ("series_id", string(player, "/episode/series_id")),
                    ("timestamp", timestamp(player, "/episode/created_at")),
                    (
                        "release_timestamp",
                        timestamp(player, "/episode/released_at"),
                    ),
                    (
                        "modified_timestamp",
                        timestamp(player, "/episode/updated_at"),
                    ),
                ],
            );
        }
        if let Some(rss) = &rss_metadata {
            merge(
                &mut merged,
                vec![
                    ("title", string(rss, "/content/episode_title")),
                    (
                        "description",
                        string(rss, "/content/episode_description_plain"),
                    ),
                    ("episode_id", string(rss, "/content/episode_id")),
                    ("episode_number", int(rss, "/content/episode_number")),
                    ("season", string(rss, "/content/season_title")),
                    ("season_id", string(rss, "/content/season_id")),
                    ("season_number", int(rss, "/content/season_number")),
                    ("series", string(rss, "/content/series_title")),
                    ("series_id", string(rss, "/content/series_id")),
                    ("thumbnail", url_value(rss, "/content/cover_image")),
                    ("duration", float(rss, "/content/duration")),
                ],
            );
        }

        let mut fields = vec![("id", Value::from(episode_id)), ("formats", Value::Null)];
        fields.extend(merged);
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        let id = "5ba1413c-48b8-472b-9cc3-cfd952340bdb";
        assert_eq!(
            episode_id(&format!("https://rss.art19.com/episodes/{id}.mp3")).as_deref(),
            Some(id)
        );
        assert_eq!(
            episode_id(&format!(
                "https://www.art19.com/shows/the-daily-briefing/episodes/{id}"
            ))
            .as_deref(),
            Some(id)
        );
        assert!(!suitable("https://www.art19.com/shows/the-daily-briefing/"));
    }

    #[test]
    fn merge_keeps_first_position_and_latest_value() {
        let mut merged = Vec::new();
        merge(
            &mut merged,
            vec![("title", Some("a".into())), ("series", None)],
        );
        merge(
            &mut merged,
            vec![("series", Some("s".into())), ("title", Some("b".into()))],
        );
        assert_eq!(merged, vec![("title", "b".into()), ("series", "s".into())]);
    }
}
