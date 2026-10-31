//! Upstream `VolejTVIE` and `VolejTVCategoryPlaylistIE`
//! (`yt_dlp/extractor/volejtv.py`): a match's `api-volejtv-prod` JSON (its
//! CloudFront MP4 qualities and the teams/date title), and a category's
//! paged match list as `url` entries. `VolejTVClubPlaylistIE` is not ported.
use super::{Extractor, dict_format, dict_info, download_json, update_url_query};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct VolejTv;
pub struct VolejTvCategory;

const NAME: &str = "volejtv:match";
const KEY: &str = "VolejTV";
const CATEGORY_NAME: &str = "volejtv:category";
const CATEGORY_KEY: &str = "VolejTVCategoryPlaylist";
const API: &str = "https://api-volejtv-prod.apps.okd4.devopsie.cloud/api";
const CATEGORY_PAGE_SIZE: usize = 10;

static MATCH_URL: LazyLock<Regex> =
    LazyLock::new(|| super::url_regex(r"^https?://volej\.tv/match/(?P<id>\d+)").unwrap());
static CATEGORY_URL: LazyLock<Regex> =
    LazyLock::new(|| super::url_regex(r"^https?://volej\.tv/kategorie/(?P<id>[^/$?]+)").unwrap());

/// `int_or_none` rates are integers (`tbr`).
fn rated(mut format: crate::VideoFormat) -> crate::VideoFormat {
    crate::info_order::mark_int_rates(&mut format);
    format
}

/// `TBR_HEIGHT_MAPPING`.
fn tbr_height(tbr: &str) -> Option<i64> {
    match tbr {
        "6000" => Some(1080),
        "2400" => Some(720),
        "1500" => Some(480),
        "800" => Some(360),
        _ => None,
    }
}

/// `_match_valid_url` of [`VolejTv`].
pub fn suitable(url: &str) -> bool {
    MATCH_URL.is_match(url)
}

/// `_match_valid_url` of [`VolejTvCategory`].
pub fn category_suitable(url: &str) -> bool {
    CATEGORY_URL.is_match(url)
}

fn api(endpoint: &str) -> Result<Url> {
    Ok(Url::parse(&format!("{API}/{endpoint}"))?)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// A `str` or `None` JSON value (`{str}` in upstream's traversal).
fn text(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(text)) => Some(text.clone()),
        _ => None,
    }
}

/// `url_or_none` for the qualities' `cloud_front_path`.
fn url_or_none(value: &Value) -> Option<String> {
    let text = value.as_str()?.trim();
    let text = if let Some(rest) = text.strip_prefix("//") {
        format!("https://{rest}")
    } else {
        text.to_owned()
    };
    (text.starts_with("https://") || text.starts_with("http://")).then_some(text)
}

/// `url_result(url, ie=VolejTVIE)`: a `url` playlist entry.
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    let mut entry = Map::new();
    entry.insert("ie_key".into(), KEY.into());
    entry.insert("_type".into(), "url".into());
    entry.insert("url".into(), target.into());
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": Value::Object(entry),
        }),
    );
    Ok(info)
}

impl Extractor for VolejTv {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = MATCH_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        announce(NAME, url);
        let json_data =
            download_json(client, &api(&format!("match/{video_id}"))?, NAME, &video_id)?;

        let mut formats = Vec::new();
        let qualities = json_data
            .get("videos")
            .and_then(|videos| videos.get(0))
            .and_then(|video| video.get("qualities"))
            .and_then(Value::as_array);
        for video in qualities.into_iter().flatten() {
            let Some(cloud_front_path) = video.get("cloud_front_path").and_then(url_or_none) else {
                continue;
            };
            // `('quality', {int_or_none})` and the mapping's string keys.
            let quality = match video.get("quality") {
                Some(Value::String(text)) => Some(text.trim().to_owned()),
                Some(Value::Number(number)) => Some(number.to_string()),
                _ => None,
            };
            let tbr = quality
                .as_deref()
                .and_then(|text| text.parse::<i64>().ok())
                .map_or(Value::Null, Value::from);
            let format_id = match video.get("id") {
                Some(Value::String(id)) => Value::from(id.clone()),
                Some(Value::Number(id)) => Value::from(id.to_string()),
                _ => Value::Null,
            };
            let height = quality
                .as_deref()
                .and_then(tbr_height)
                .map_or(Value::Null, Value::from);
            formats.push(rated(dict_format(
                vec![
                    ("url", cloud_front_path.into()),
                    ("tbr", tbr),
                    ("format_id", format_id),
                    ("height", height),
                ],
                NAME,
                &video_id,
            )?));
        }

        let timestamp = match json_data.get("match_time") {
            Some(Value::String(time)) => {
                crate::dates::unified_timestamp(time, false).map_or(Value::Null, Value::from)
            }
            _ => Value::Null,
        };
        let category_title = json_data
            .get("category")
            .and_then(|category| text(category.get("title")))
            .filter(|title| !title.is_empty());
        let categories = category_title.map_or(Value::Null, |title| vec![title].into());
        let thumbnail = match json_data.get("poster") {
            Some(poster @ Value::String(_)) => url_or_none(poster).map_or(Value::Null, Value::from),
            _ => Value::Null,
        };

        // `join_nonempty(join_nonempty(*teams, delim='-'), strftime(...))`.
        let mut teams: Vec<String> = Vec::new();
        for team in json_data
            .get("teams")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if let Some(shortcut) = text(team.get("shortcut"))
                && !teams.contains(&shortcut)
            {
                teams.push(shortcut);
            }
        }
        if teams.len() > 2 {
            teams.retain(|team| team != "FIN");
        }
        let date = match timestamp.as_i64() {
            Some(seconds) => crate::dates::strftime_or_none(
                crate::dates::StrftimeInput::Integer(seconds),
                "(%Y-%m-%d)",
            )
            .ok()
            .flatten()
            .unwrap_or_default(),
            None => String::new(),
        };
        let teams = teams.join("-");
        let title = [teams, date]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ");

        // `traverse_obj` leaves out the `None` values.
        let mut fields: Vec<(&str, Value)> = vec![("id", video_id.clone().into())];
        for (key, value) in [
            (
                "series",
                text(json_data.get("competition_name")).map_or(Value::Null, Value::from),
            ),
            (
                "season",
                text(json_data.get("season")).map_or(Value::Null, Value::from),
            ),
            ("timestamp", timestamp),
            ("categories", categories),
            ("thumbnail", thumbnail),
        ] {
            if !value.is_null() {
                fields.push((key, value));
            }
        }
        fields.push(("formats", Value::Null));
        fields.push(("title", title.into()));

        Ok(Some(dict_info(url, (NAME, KEY), fields, formats)))
    }
}

impl Extractor for VolejTvCategory {
    fn key(&self) -> &'static str {
        CATEGORY_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = CATEGORY_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let slug = found["id"].to_owned();
        announce(CATEGORY_NAME, url);

        let categories = download_json(client, &api("category")?, CATEGORY_NAME, &slug)?;
        let mut category = None;
        for entry in categories.as_array().into_iter().flatten() {
            let (Some(id), Some(entry_slug), Some(title)) = (
                entry.get("id").filter(|v| super::nuxt::truthy(v)),
                text(entry.get("slug")).filter(|v| !v.is_empty()),
                text(entry.get("title")).filter(|v| !v.is_empty()),
            ) else {
                continue;
            };
            if entry_slug == slug {
                category = Some((id.clone(), title));
                break;
            }
        }
        let Some((category_id, title)) = category else {
            return Err(Error::UnsupportedHtml5(
                "Volej.tv categories that are not listed by the API are not supported natively"
                    .into(),
            ));
        };

        let page_url = |page: usize| -> Result<Url> {
            let base = api(&format!(
                "match/by-category-id-paginated/{}",
                value_text(&category_id)
            ))?;
            Ok(update_url_query(
                &base,
                &[
                    ("page", &page.to_string()),
                    ("take", &CATEGORY_PAGE_SIZE.to_string()),
                    ("order", "DESC"),
                ],
            ))
        };
        let first_page = download_json(client, &page_url(1)?, CATEGORY_NAME, &slug)?;
        let page_count = first_page
            .get("meta")
            .and_then(|meta| meta.get("pageCount"))
            .and_then(Value::as_i64)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{CATEGORY_NAME}] {slug}: Unable to extract page count"
                ))
            })?;

        let mut entries = Vec::new();
        for page in 1..=page_count.max(0) {
            let data = if page == 1 {
                first_page.clone()
            } else {
                download_json(client, &page_url(page as usize)?, CATEGORY_NAME, &slug)?
            };
            for item in data
                .get("data")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let Some(match_id) = item.get("id") else {
                    continue;
                };
                entries.push(url_entry(&format!(
                    "https://volej.tv/match/{}",
                    value_text(match_id)
                ))?);
            }
        }

        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            "playlist-dict;id,title".into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: slug,
            title,
            webpage_url: url.clone(),
            extractor: CATEGORY_NAME.into(),
            extractor_key: CATEGORY_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

/// `str(value)` of an ID JSON value.
fn value_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://volej.tv/match/270579"));
        assert!(!suitable("https://volej.tv/klub/1173"));
        assert!(category_suitable(
            "https://volej.tv/kategorie/chance-cesky-pohar"
        ));
        assert!(!category_suitable("https://volej.tv/match/270579"));
    }

    #[test]
    fn tbr_maps_to_upstream_heights() {
        assert_eq!(tbr_height("1500"), Some(480));
        assert_eq!(tbr_height("999"), None);
    }
}
