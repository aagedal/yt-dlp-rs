//! Upstream `DreiSatIE` (`yt_dlp/extractor/dreisat.py`): a 3sat page's player
//! JSON, its content document (the player's API token) and the PTMD, built
//! like ZDF's base extraction (`merge_dicts` over the PTMD info).
use super::Extractor;
use super::zdf::{
    PtmdUrl, extract_chapters, extract_ptmd, extract_thumbnails, int_or_none, key_error,
    mark_shape, parse_aspect_ratio, path, request_json, text, unsupported,
};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct DreiSat;

const NAME: &str = "3sat";
const KEY: &str = "DreiSat";
const PLAYER_ID: &str = "android_native_6";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?3sat\.de/(?:[^/?#]+/)*(?P<id>[^/?#&]+)\.html")
        .unwrap()
});
static PLAYER_JSON: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"data-zdfplayer-jsb=(["'])"#).unwrap());

/// `DreiSatIE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `traverse_obj(video_target, ...)`'s first non-empty string among keys.
fn first_text(values: &[Option<&Value>]) -> Option<String> {
    values
        .iter()
        .find_map(|value| text(*value).filter(|text| !text.is_empty()))
}

impl Extractor for DreiSat {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = super::download_webpage(client, url, NAME, &video_id)?;
        let player = match super::search_json(&PLAYER_JSON, &webpage, false) {
            Some(Some(player)) => player,
            _ => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract player JSON; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let player_url = player
            .get("content")
            .ok_or_else(|| key_error(&video_id, "content"))?
            .as_str()
            .ok_or_else(|| unsupported("player content URLs that are not strings"))?
            .to_owned();
        let api_token = format!(
            "Bearer {}",
            player
                .get("apiToken")
                .ok_or_else(|| key_error(&video_id, "apiToken"))?
                .as_str()
                .ok_or_else(|| unsupported("API tokens that are not strings"))?
        );

        let content = request_json(
            client,
            &Url::parse(&player_url)?,
            NAME,
            Some(&video_id),
            "Downloading video metadata",
            "Failed to download video metadata",
            &[("Api-Auth", &api_token)],
            None,
        )?;
        let video_target = content
            .get("mainVideoContent")
            .and_then(|main| main.get("http://zdf.de/rels/target"))
            .ok_or_else(|| key_error(&video_id, "mainVideoContent"))?;

        // `((('streams', 'default'), None), ('ptmd', 'ptmd-template'), {str}, any)`.
        let ptmd_path = [
            video_target
                .get("streams")
                .and_then(|streams| streams.get("default")),
            Some(video_target),
        ]
        .into_iter()
        .flatten()
        .find_map(|source| {
            [
                "http://zdf.de/rels/streams/ptmd",
                "http://zdf.de/rels/streams/ptmd-template",
            ]
            .into_iter()
            .find_map(|key| text(source.get(key)).filter(|template| !template.is_empty()))
        })
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract ptmd path; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        // `_expand_ptmd_template`: `urljoin(player_url, template)`.
        let expanded = ptmd_path.replace("{playerId}", PLAYER_ID);
        let ptmd_url = Url::parse(&player_url)?.join(&expanded)?;
        let aspect_ratio = parse_aspect_ratio(video_target.get("aspectRatio"))?;
        let ptmd = extract_ptmd(
            client,
            NAME,
            &[PtmdUrl {
                url: ptmd_url.to_string(),
                is_dgs: false,
            }],
            &video_id,
            &api_token,
            aspect_ratio,
        )?;

        // `merge_dicts(info, {...})`: the PTMD info's keys first.
        let mut fields: Vec<(&str, Value)> = vec![
            (
                "id",
                ptmd.content_id
                    .clone()
                    .unwrap_or_else(|| video_id.clone())
                    .into(),
            ),
            (
                "duration",
                ptmd.duration
                    .map(Value::from)
                    .or_else(|| int_or_none(video_target.get("duration")).map(Value::from))
                    .unwrap_or(Value::Null),
            ),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        let teaser = [content.get("title"), content.get("teaserHeadline")];
        if let Some(title) = first_text(&teaser) {
            fields.push(("title", title.into()));
        }
        if let Some(episode) = first_text(&teaser) {
            fields.push(("episode", episode.into()));
        }
        let description = [content.get("leadParagraph"), content.get("teasertext")];
        if let Some(description) = first_text(&description) {
            fields.push(("description", description.into()));
        }
        if let Some(timestamp) = content
            .get("editorialDate")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
        {
            fields.push(("timestamp", timestamp.into()));
        }
        if let Some(chapters) = extract_chapters(video_target.get("streamAnchorTag")) {
            fields.push(("chapters", chapters));
        }
        fields.push((
            "thumbnails",
            extract_thumbnails(
                &path(&content, &["teaserImageRef", "layouts"])
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
        ));
        let target = path(&content, &["programmeItem"])
            .and_then(|items| items.get(0))
            .and_then(|item| item.get("http://zdf.de/rels/target"));
        let series = |key: &str| target.and_then(|target| target.get(key));
        let cmdm = |key: &str, field: &str| series(key).and_then(|value| value.get(field));
        let series_fields: [(&str, Option<Value>); 7] = [
            (
                "series_id",
                text(cmdm("http://zdf.de/rels/cmdm/series", "seriesUuid")).map(Value::from),
            ),
            (
                "series",
                text(cmdm("http://zdf.de/rels/cmdm/series", "seriesTitle")).map(Value::from),
            ),
            (
                "season",
                text(cmdm("http://zdf.de/rels/cmdm/season", "seasonTitle")).map(Value::from),
            ),
            (
                "season_number",
                int_or_none(cmdm("http://zdf.de/rels/cmdm/season", "seasonNumber"))
                    .map(Value::from),
            ),
            (
                "season_id",
                text(cmdm("http://zdf.de/rels/cmdm/season", "seasonUuid")).map(Value::from),
            ),
            (
                "episode_number",
                int_or_none(series("episodeNumber")).map(Value::from),
            ),
            ("episode_id", text(series("contentId")).map(Value::from)),
        ];
        for (key, value) in series_fields {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }

        let mut info = super::dict_info(url, (NAME, KEY), fields, ptmd.formats);
        info.subtitles = ptmd.subtitles;
        mark_shape(
            &mut info,
            ";chapter=start_time,title;thumbnail=id,url,preference,width,height",
        );
        Ok(Some(ExtractResult::Video(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.3sat.de/dokumentation/reise/traumziele-suedostasiens-die-philippinen-und-vietnam-102.html"
        ));
        assert!(!suitable("https://www.3sat.de/"));
    }
}
