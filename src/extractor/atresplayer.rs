//! Upstream `AtresPlayerIE` (`yt_dlp/extractor/atresplayer.py`): the
//! `api.atresplayer.com` client URL lookup for a clip's page path, the page's
//! `urlVideo` player document and its HLS and DASH sources. Anonymous access
//! only: upstream's `_perform_login` and the `required_registered` 403 body
//! parsing are not ported.
use super::json_ld::url_or_none;
use super::{
    Extractor, dict_info, download_json, download_json_note, extract_m3u8_master_optional,
    extract_mpd_optional_note, update_url_query,
};
use crate::{Result, Subtitles, VideoFormat};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct AtresPlayer;

const NAME: &str = "AtresPlayer";
const API_BASE: &str = "https://api.atresplayer.com/";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?atresplayer\.com/(?:[^/?#]+/){4}(?P<display_id>.+?)_(?P<id>[0-9a-f]{24})",
    )
    .unwrap()
});

/// `_match_valid_url`'s `display_id` and `id` groups.
fn match_ids(url: &str) -> Option<(String, String)> {
    VALID_URL
        .captures(url)
        .map(|found| (found["display_id"].to_owned(), found["id"].to_owned()))
}

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Upstream `traverse_obj(..., (key, {str}))`: a non-empty string value.
fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |node, key| node.get(*key))
        .and_then(Value::as_str)
}

/// Upstream `int_or_none(scale=...)` on a number: `int(value) // scale`.
fn scaled_int(value: Option<&Value>, scale: i64) -> Option<i64> {
    let number = match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64))?,
        _ => return None,
    };
    Some(number / scale)
}

/// Set (keeping its position) or append a `dict` key.
fn put(fields: &mut Vec<(&'static str, Value)>, key: &'static str, value: Value) {
    match fields.iter_mut().find(|(name, _)| *name == key) {
        Some(slot) => slot.1 = value,
        None => fields.push((key, value)),
    }
}

impl Extractor for AtresPlayer {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<crate::VideoInfo>> {
        let Some((display_id, video_id)) = match_ids(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        // `urllib.parse.urlparse(url).path` as the `href` query.
        let api = Url::parse(&format!("{API_BASE}client/v1/url"))?;
        let api = update_url_query(&api, &[("href", url.path())]);
        let lookup = download_json_note(
            client,
            &api,
            NAME,
            &video_id,
            "Downloading API endpoint data",
        )?;
        let metadata_url = match str_at(&lookup, &["href"]) {
            Some(href) => Url::parse(href)?,
            None => {
                return Err(crate::Error::Reported(format!(
                    "[{NAME}] {video_id}: An extractor error has occurred. (caused by KeyError('href')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let metadata = download_json(client, &metadata_url, NAME, &video_id)?;
        let video_url = match str_at(&metadata, &["urlVideo"]) {
            Some(href) => Url::parse(href)?,
            None => {
                return Err(crate::Error::Reported(format!(
                    "[{NAME}] {video_id}: An extractor error has occurred. (caused by KeyError('urlVideo')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let video_data = download_json_note(
            client,
            &video_url,
            NAME,
            &video_id,
            "Downloading video data",
        )?;

        let mut formats: Vec<VideoFormat> = Vec::new();
        let mut subtitles = Subtitles::default();
        let sources = video_data
            .get("sources")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        for source in sources {
            let Some(src_url) = source
                .get("src")
                .and_then(Value::as_str)
                .and_then(url_or_none)
            else {
                continue;
            };
            let src_type = source
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if matches!(
                src_type,
                "application/vnd.apple.mpegurl" | "application/hls+legacy" | "application/hls+hevc"
            ) {
                if let Some(stream) = extract_m3u8_master_optional(
                    client,
                    &Url::parse(&src_url)?,
                    NAME,
                    &video_id,
                    Some("hls"),
                    true,
                    false,
                ) {
                    formats.extend(stream.formats);
                    super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                }
            } else if matches!(src_type, "application/dash+xml" | "application/dash+hevc")
                && let Some(stream) = extract_mpd_optional_note(
                    client,
                    &Url::parse(&src_url)?,
                    NAME,
                    &video_id,
                    Some("dash"),
                    "Downloading MPD manifest",
                )
            {
                formats.extend(stream.formats);
                super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        }

        // The upstream dict literal: `video_data`'s keys first, then
        // `metadata`'s, each overriding values in place.
        let mut fields: Vec<(&'static str, Value)> = vec![
            ("display_id", display_id.clone().into()),
            ("id", video_id.clone().into()),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        let title = str_at(&metadata, &["title"]).or_else(|| str_at(&video_data, &["titulo"]));
        if let Some(title) = title {
            put(&mut fields, "title", title.into());
        }
        if let Some(description) =
            str_at(&metadata, &["description"]).or_else(|| str_at(&video_data, &["descripcion"]))
        {
            put(&mut fields, "description", description.into());
        }
        if let Some(duration) = scaled_int(metadata.get("duration"), 1)
            .or_else(|| scaled_int(video_data.get("duration"), 1))
        {
            put(&mut fields, "duration", duration.into());
        }
        if let Some(poster) = video_data
            .get("imgPoster")
            .and_then(Value::as_str)
            .and_then(url_or_none)
        {
            put(
                &mut fields,
                "thumbnail",
                format!("{poster}1920x1080.jpg").into(),
            );
        }
        let tags: Vec<Value> = metadata
            .get("tags")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tag| str_at(tag, &["title"]).map(Value::from))
            .collect();
        if !tags.is_empty() {
            put(&mut fields, "tags", Value::Array(tags));
        }
        if let Some(series) = str_at(&metadata, &["format", "title"]) {
            put(&mut fields, "series", series.into());
        }
        if let Some(season) = str_at(&metadata, &["currentSeason", "title"]) {
            put(&mut fields, "season", season.into());
        }
        if let Some(timestamp) = scaled_int(metadata.get("publicationDate"), 1000) {
            put(&mut fields, "timestamp", timestamp.into());
        }
        if let Some(channel) = str_at(&metadata, &["channel", "title"]) {
            put(&mut fields, "channel", channel.into());
        }

        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        let shape = info
            .extra
            .get(crate::info_order::INFO_SHAPE)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("{shape};subtitle=ext,url"),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.atresplayer.com/lasexta/programas/el-objetivo/clips/mbappe-describe-como-entrenador-a-carlo-ancelotti-sabe-cuando-tiene-que-ser-padre-jefe-amigo-entrenador_67f2dfb2fb6ab0e4c7203849/"
        ));
        assert!(suitable(
            "https://atresplayer.com/flooxer/series/biara-proyecto-lazarus/temporada-1/capitulo-3-supervivientes_67a6038b64ceca00070f4f69/"
        ));
        assert!(!suitable(
            "https://www.atresplayer.com/lasexta/programas/el-objetivo/"
        ));
        assert_eq!(
            match_ids(
                "https://www.atresplayer.com/flooxer/series/biara-proyecto-lazarus/temporada-1/capitulo-3-supervivientes_67a6038b64ceca00070f4f69/"
            ),
            Some((
                "capitulo-3-supervivientes".into(),
                "67a6038b64ceca00070f4f69".into()
            ))
        );
    }
}
