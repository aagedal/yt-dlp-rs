//! Upstream `MixchIE`, `MixchArchiveIE` and `MixchMovieIE`
//! (`yt_dlp/extractor/mixch.py`): a user's live HLS stream, an archive's
//! master playlist (login required for paid archives) and a movie's file.
//! Live comments are only fetched with `--get-comments`, which is not
//! supported natively yet.
use super::{Extractor, dict_format, dict_info, download_json, extract_m3u8_fatal_master, json_ld};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Mixch;
pub struct MixchArchive;
pub struct MixchMovie;

const NAME: &str = "mixch";
const KEY: &str = "Mixch";
const ARCHIVE_NAME: &str = "mixch:archive";
const ARCHIVE_KEY: &str = "MixchArchive";
const MOVIE_NAME: &str = "mixch:movie";
const MOVIE_KEY: &str = "MixchMovie";
const LOGIN_HINT: &str = "Use --cookies-from-browser or --cookies for the authentication. See  https://github.com/yt-dlp/yt-dlp/wiki/FAQ#how-do-i-pass-cookies-to-yt-dlp  for how to manually pass cookies";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| crate::extractor::url_regex(r"^https?://mixch\.tv/u/(?P<id>\d+)").unwrap());
static ARCHIVE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://mixch\.tv/archive/(?P<id>\d+)").unwrap());
static MOVIE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://mixch\.tv/m/(?P<id>\w+)").unwrap());

/// `_match_id` for the live channel URLs.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_match_valid_url` for the archive URLs.
pub fn archive_suitable(url: &str) -> bool {
    ARCHIVE_URL.is_match(url)
}

/// `_match_valid_url` for the movie URLs.
pub fn movie_suitable(url: &str) -> bool {
    MOVIE_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Mixch {what} are not supported natively"))
}

/// The HTTP status of a failed download, through its wrappers.
fn http_status(error: &Error) -> Option<u16> {
    match error {
        Error::ExtractorRequest { source, .. } => http_status(source),
        Error::Http(source) | Error::HttpRetryAfter { source, .. } => {
            source.status().map(|status| status.as_u16())
        }
        _ => None,
    }
}

/// `traverse_obj(data, (a, b, ...))` for a string value.
fn str_at<'a>(value: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(value, |value, key| value.get(*key))
        .and_then(Value::as_str)
}

/// `traverse_obj(data, (a, b, ...), {int_or_none})`.
fn int_at(value: &Value, path: &[&str]) -> Value {
    super::streamable::int_or_none(path.iter().try_fold(value, |value, key| value.get(*key)))
}

/// A required JSON field (`KeyError` when missing).
fn key_or(value: Option<&Value>, id: &str, key: &str) -> Result<Value> {
    value
        .cloned()
        .ok_or_else(|| super::clyp::key_error(id, key))
}

impl Extractor for Mixch {
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
        let data = download_json(
            client,
            &Url::parse(&format!("https://mixch.tv/api-web/users/{video_id}/live"))?,
            NAME,
            &video_id,
        )?;
        let Some(live_info) = data.get("liveInfo").filter(|info| info.is_object()) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: The channel is not currently live"
            )));
        };
        // `extract_comments` is only fetched with `--get-comments`.
        if super::get_comments() {
            return Err(unsupported("live comments"));
        }
        let hls = key_or(live_info.get("hls"), &video_id, "hls")?;
        let formats = vec![dict_format(
            vec![
                ("format_id", "hls".into()),
                ("url", hls),
                ("ext", "mp4".into()),
                ("protocol", "m3u8".into()),
            ],
            NAME,
            &video_id,
        )?];
        // `**traverse_obj(data, {...})`: `None` values are left out.
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("uploader_id", video_id.clone().into()),
        ];
        if let Some(title) = str_at(&data, &["liveInfo", "title"]) {
            fields.push(("title", title.into()));
        }
        for (key, path) in [
            ("comment_count", "comments"),
            ("view_count", "visitor"),
            ("timestamp", "created"),
        ] {
            let value = int_at(&data, &["liveInfo", path]);
            if !value.is_null() {
                fields.push((key, value));
            }
        }
        if let Some(uploader) = str_at(&data, &["broadcasterInfo", "name"]) {
            fields.push(("uploader", uploader.into()));
        }
        fields.push(("formats", Value::Null));
        fields.push(("is_live", true.into()));
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (NAME, KEY),
            fields,
            formats,
        ))))
    }
}

impl Extractor for MixchArchive {
    fn key(&self) -> &'static str {
        ARCHIVE_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ARCHIVE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(ARCHIVE_NAME);
        crate::screen::screen(&format!(
            "[{ARCHIVE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let data = match download_json(
            client,
            &Url::parse(&format!("https://mixch.tv/api-web/archive/{video_id}"))?,
            ARCHIVE_NAME,
            &video_id,
        ) {
            Ok(data) => data,
            Err(error) if http_status(&error) == Some(401) => {
                return Err(Error::Reported(format!(
                    "[{ARCHIVE_NAME}] {video_id}: This video is only available for registered users. {LOGIN_HINT}"
                )));
            }
            Err(error) => return Err(error),
        };
        let archive = data
            .get("archive")
            .ok_or_else(|| super::clyp::key_error(&video_id, "archive"))?;
        let archive_url = Url::parse(
            archive
                .get("archiveURL")
                .and_then(Value::as_str)
                .ok_or_else(|| super::clyp::key_error(&video_id, "archiveURL"))?,
        )?;
        let stream =
            extract_m3u8_fatal_master(client, &archive_url, ARCHIVE_NAME, &video_id, None, false)?;
        let title = str_at(archive, &["title"]).map_or(Value::Null, Value::from);
        let thumbnail = str_at(archive, &["thumbnailURL"])
            .and_then(json_ld::url_or_none)
            .map_or(Value::Null, Value::from);
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (ARCHIVE_NAME, ARCHIVE_KEY),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("formats", Value::Null),
                ("thumbnail", thumbnail),
            ],
            stream.formats,
        ))))
    }
}

impl Extractor for MixchMovie {
    fn key(&self) -> &'static str {
        MOVIE_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = MOVIE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(MOVIE_NAME);
        crate::screen::screen(&format!(
            "[{MOVIE_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let data = download_json(
            client,
            &Url::parse(&format!("https://mixch.tv/api-web/movies/{video_id}"))?,
            MOVIE_NAME,
            &video_id,
        )?;
        let file = data
            .get("movie")
            .and_then(|movie| movie.get("file"))
            .cloned()
            .ok_or_else(|| super::clyp::key_error(&video_id, "file"))?;
        let formats = vec![dict_format(
            vec![
                ("format_id", "mp4".into()),
                ("url", file),
                ("ext", "mp4".into()),
            ],
            MOVIE_NAME,
            &video_id,
        )?];

        // `**traverse_obj(data, {...})`: `None` values are left out.
        let mut fields: Vec<(&str, Value)> =
            vec![("id", video_id.clone().into()), ("formats", Value::Null)];
        let mut add = |key: &'static str, value: Value| {
            if !value.is_null() {
                fields.push((key, value));
            }
        };
        add(
            "title",
            str_at(&data, &["movie", "title"]).map_or(Value::Null, Value::from),
        );
        add(
            "thumbnail",
            str_at(&data, &["movie", "thumbnailURL"])
                .and_then(json_ld::url_or_none)
                .map_or(Value::Null, Value::from),
        );
        add(
            "uploader",
            str_at(&data, &["ownerInfo", "name"]).map_or(Value::Null, Value::from),
        );
        add(
            "uploader_id",
            match data.get("ownerInfo").and_then(|owner| owner.get("id")) {
                Some(Value::Number(number)) => number.to_string().into(),
                _ => Value::Null,
            },
        );
        add(
            "channel_follower_count",
            int_at(&data, &["ownerInfo", "fan"]),
        );
        add("view_count", int_at(&data, &["ownerInfo", "view"]));
        add("like_count", int_at(&data, &["movie", "favCount"]));
        add("comment_count", int_at(&data, &["movie", "commentCount"]));
        add("timestamp", int_at(&data, &["movie", "published"]));
        let owner_id = match data.get("ownerInfo").and_then(|owner| owner.get("id")) {
            Some(Value::Number(number)) if number.as_i64().is_some_and(|id| id != 0) => {
                Value::from(format!("https://mixch.tv/u/{number}"))
            }
            _ => Value::Null,
        };
        add("uploader_url", owner_id);
        fields.push(("live_status", "not_live".into()));
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (MOVIE_NAME, MOVIE_KEY),
            fields,
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://mixch.tv/u/16943797"));
        assert!(!suitable("https://mixch.tv/u/"));
        assert!(archive_suitable("https://mixch.tv/archive/421"));
        assert!(movie_suitable("https://mixch.tv/m/Ve8KNkJ5"));
        assert!(!movie_suitable("https://mixch.tv/m/"));
    }
}
