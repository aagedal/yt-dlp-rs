//! Upstream `AmericasTestKitchenIE` and `AmericasTestKitchenSeasonIE`
//! (`yt_dlp/extractor/americastestkitchen.py`): an episode or video through
//! the ATK API as a Zype `url_transparent`, and a season (or the whole show,
//! until a season page is missing) as a playlist of episode URLs.
use super::{Extractor, download_json, download_webpage};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct AmericasTestKitchen;
pub struct AmericasTestKitchenSeason;

const NAME: &str = "AmericasTestKitchen";
const SEASON_NAME: &str = "AmericasTestKitchenSeason";
const ZYPE_KEY: &str = "jZ9GUhRmxcPvX7M3SlfejB6Hle9jyHTdk2jVxG7wOHPLODgncEKVdPYBhuz9iWXQ";

static EPISODE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?:americastestkitchen|cooks(?:country|illustrated))\.com/(?:cooks(?:country|illustrated)/)?(?P<resource_type>episode|videos)/(?P<id>\d+)",
    )
    .unwrap()
});

static SEASON_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?x)^https?://(?:www\.)?(?P<domain>americastestkitchen|cookscountry|cooksillustrated)\.com
        (?:/(?P<path>cookscountry|cooksillustrated))?
        (?:/episodes(?:/browse)?/season[-_](?P<season>\d+))?
        /?(?:[?\#]|$)",
    )
    .unwrap()
});

static EPISODE_LINK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<a [^>]*\bhref="(?P<path>/(?:cookscountry/|cooksillustrated/)?episode/(?P<id>\d+)-[^"]+)"[^>]*>\s*<h3[^>]*>(?P<title>[^<]+)</h3>"#,
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    EPISODE_URL.is_match(url)
}

/// `_match_valid_url` for seasons and shows.
pub fn season_suitable(url: &str) -> bool {
    SEASON_URL.is_match(url)
}

/// The show path and title of `_SHOWS`.
fn show(key: &str) -> (&'static str, &'static str) {
    match key {
        "cookscountry" => ("/cookscountry", "Cook's Country"),
        "cooksillustrated" => ("/cooksillustrated", "Cook's Illustrated"),
        _ => ("", "America's Test Kitchen"),
    }
}

fn is_not_found(error: &Error) -> bool {
    matches!(
        error,
        Error::ExtractorRequest { source, .. }
            if matches!(source.as_ref(), Error::Http(error)
                if error.status().is_some_and(|status| status.as_u16() == 404))
    )
}

/// `try_get(obj, lambda x: x[key][0][value_key])`-style lookups.
fn text_of(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => Value::from(text.as_str()),
        _ => Value::Null,
    }
}

/// The Zype `url_transparent` with the ATK metadata as overriding fields.
fn zype_follow(url: &Url, zype_id: &str, fields: Vec<(&str, Value)>) -> VideoInfo {
    let target = format!("https://player.zype.com/embed/{zype_id}.js?api_key={ZYPE_KEY}");
    let mut info = super::direct::build_at(url.clone(), url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut fields_map = Map::new();
    for (key, value) in fields {
        if !value.is_null() {
            fields_map.insert(key.into(), value);
        }
    }
    info.extra.insert(
        crate::FOLLOW_TRANSPARENT.into(),
        json!({"url": target, "fields": Value::Object(fields_map)}),
    );
    info
}

impl Extractor for AmericasTestKitchen {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = EPISODE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let video_id = found["id"].to_owned();
        let is_episode = &found["resource_type"] == "episode";
        let resource_type = if is_episode { "episodes" } else { "videos" };
        let api = Url::parse(&format!(
            "https://www.americastestkitchen.com/api/v6/{resource_type}/{video_id}"
        ))?;
        let resource = download_json(client, &api, NAME, &video_id)?;
        let video = if is_episode {
            resource.get("video").cloned().unwrap_or(Value::Null)
        } else {
            resource.clone()
        };
        let episode = if is_episode {
            resource.clone()
        } else {
            match resource.get("episode") {
                Some(value) if super::nuxt::truthy(value) => value.clone(),
                _ => json!({}),
            }
        };
        let zype_id = video
            .get("zypeId")
            .and_then(Value::as_str)
            .ok_or_else(|| super::clyp::key_error(&video_id, "zypeId"))?
            .to_owned();
        let publish = video
            .get("publishDate")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let timestamp = publish
            .as_deref()
            .and_then(|date| crate::dates::unified_timestamp(date, false))
            .map_or(Value::Null, Value::from);
        let release_date = publish
            .as_deref()
            .and_then(|date| crate::dates::unified_strdate(date, false))
            .map_or(Value::Null, Value::from);
        let description = match video.get("description") {
            Some(Value::String(text)) => Value::from(crate::html::clean_html(text)),
            _ => Value::Null,
        };
        let series = episode
            .get("show")
            .and_then(|show| show.get("title"))
            .and_then(Value::as_str)
            .map_or(Value::Null, Value::from);
        let fields = vec![
            ("description", description),
            ("timestamp", timestamp),
            ("release_date", release_date),
            (
                "episode_number",
                super::streamable::int_or_none(episode.get("number")),
            ),
            (
                "season_number",
                super::streamable::int_or_none(episode.get("season")),
            ),
            ("series", series),
            ("episode", text_of(episode.get("title"))),
        ];
        Ok(Some(ExtractResult::Video(zype_follow(
            url, &zype_id, fields,
        ))))
    }
}

/// A `url_result` for one season entry (`season_number` is its only extra).
fn season_entry(target: &str, id: &str, title: &str, season: i64) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let entry = json!({
        "season_number": season,
        "ie_key": NAME,
        "id": id,
        "title": title,
        "_type": "url",
        "url": target,
    });
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": NAME,
            "extractor": NAME,
            "entry": entry,
        }),
    );
    Ok(info)
}

/// `_season_entries`: each distinct episode card of one season page.
fn season_entries(client: &Client, show_path: &str, season: i64) -> Result<Vec<VideoInfo>> {
    let page = format!("https://www.americastestkitchen.com{show_path}/episodes/season-{season}");
    let url = Url::parse(&page)?;
    let webpage = download_webpage(client, &url, SEASON_NAME, &format!("season-{season}"))?;
    let mut seen = Vec::new();
    let mut entries = Vec::new();
    for episode in EPISODE_LINK.captures_iter(&webpage) {
        let path = episode["path"].to_owned();
        if seen.contains(&path) {
            continue;
        }
        seen.push(path.clone());
        let target = format!("https://www.americastestkitchen.com{path}");
        let title = crate::html::clean_html(&episode["title"]);
        entries.push(season_entry(&target, &episode["id"], &title, season)?);
    }
    Ok(entries)
}

impl Extractor for AmericasTestKitchenSeason {
    fn key(&self) -> &'static str {
        SEASON_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = SEASON_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(SEASON_NAME);
        crate::screen::screen(&format!(
            "[{SEASON_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let domain = found["domain"].to_owned();
        let path = found.name("path").map(|m| m.as_str().to_owned());
        let season = found
            .name("season")
            .and_then(|m| m.as_str().parse::<i64>().ok());
        let (show_path, title) = show(path.as_deref().unwrap_or(&domain));

        let (playlist_id, playlist_title, entries) = match season.filter(|n| *n != 0) {
            Some(season) => (
                format!("season_{season}"),
                format!("Season {season}"),
                season_entries(client, show_path, season)?,
            ),
            None => {
                let mut entries = Vec::new();
                for season_number in 1.. {
                    match season_entries(client, show_path, season_number) {
                        Ok(found) => entries.extend(found),
                        Err(error) if is_not_found(&error) => break,
                        Err(error) => return Err(error),
                    }
                }
                (
                    path.clone().unwrap_or(domain.clone()),
                    title.to_owned(),
                    entries,
                )
            }
        };

        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: playlist_id,
            title: playlist_title,
            webpage_url: url.clone(),
            extractor: SEASON_NAME.into(),
            extractor_key: SEASON_NAME.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.americastestkitchen.com/episode/582-weeknight-japanese-suppers"
        ));
        assert!(suitable(
            "https://www.cooksillustrated.com/videos/4478-beef-wellington"
        ));
        assert!(!suitable(
            "https://www.americastestkitchen.com/episodes/season-1"
        ));
        assert!(season_suitable(
            "https://www.americastestkitchen.com/episodes/season-26"
        ));
        assert!(season_suitable(
            "https://www.cookscountry.com/episodes/browse/season_12"
        ));
        assert!(season_suitable("https://www.americastestkitchen.com/"));
        assert!(!season_suitable(
            "https://www.americastestkitchen.com/episode/582-weeknight-japanese-suppers"
        ));
    }
}
