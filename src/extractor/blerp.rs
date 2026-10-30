//! Upstream `BlerpIE` (`yt_dlp/extractor/blerp.py`): a soundbite's GraphQL
//! `biteById` record as a direct MP3 result.
use super::{Extractor, direct, post_json_note_headers};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Blerp;

const NAME: &str = "blerp";
const KEY: &str = "Blerp";
const OPERATION: &str = "webBitePageGetBite";
const API: &str = "https://api.blerp.com/graphql";

/// `_GRAPHQL_QUERY` with insignificant whitespace removed.
const QUERY: &str = "query webBitePageGetBite($_id: MongoID!) { web { biteById(_id: $_id) { ...bitePageFrag __typename } __typename } } fragment bitePageFrag on Bite { _id title userKeywords keywords color visibility isPremium owned price extraReview isAudioExists image { filename original { url __typename } __typename } userReactions { _id reactions createdAt __typename } topReactions totalSaveCount saved blerpLibraryType license licenseMetaData playCount totalShareCount totalFavoriteCount totalAddedToBoardCount userCategory userAudioQuality audioCreationState transcription userTranscription description createdAt updatedAt author listingType ownerObject { _id username profileImage { filename original { url __typename } __typename } __typename } transcription favorited visibility isCurated sourceUrl audienceRating strictAudienceRating ownerId reportObject { reportedContentStatus __typename } giphy { mp4 gif __typename } audio { filename original { url __typename } mp3 { url __typename } __typename } __typename }";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?blerp\.com/soundbites/(?P<id>[0-9a-zA-Z]+)").unwrap()
});

/// `strip_or_none` of a string value (`expected_type=str`).
fn strip_or_none(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) => {
            let stripped = crate::batch::py_strip(text);
            if stripped.is_empty() {
                Value::Null
            } else {
                Value::from(stripped)
            }
        }
        _ => Value::Null,
    }
}

impl Extractor for Blerp {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let audio_id = found["id"].to_owned();
        crate::screen::set_extractor(KEY);
        crate::screen::screen(&format!(
            "[{KEY}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let body = json!({
            "operationName": OPERATION,
            "query": QUERY,
            "variables": {"_id": audio_id},
        })
        .to_string();
        let api = Url::parse(API)?;
        let result = post_json_note_headers(
            client,
            &api,
            NAME,
            &audio_id,
            "Downloading JSON metadata",
            &[("Content-Type", "application/json")],
            body,
        )?;
        let bite = result
            .get("data")
            .and_then(|data| data.get("web"))
            .and_then(|web| web.get("biteById"))
            .filter(|bite| bite.is_object())
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Blerp responses without a soundbite are not supported natively".into(),
                )
            })?;
        let id = bite.get("_id").cloned().unwrap_or(Value::Null);
        let media = bite
            .get("audio")
            .and_then(|audio| audio.get("mp3"))
            .and_then(|mp3| mp3.get("url"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "Blerp soundbites without an MP3 URL are not supported natively".into(),
                )
            })?;
        let media = Url::parse(media)?;
        let title = bite.get("title").cloned().unwrap_or(Value::Null);
        let owner = bite.get("ownerObject");
        let uploader = strip_or_none(owner.and_then(|owner| owner.get("username")));
        let uploader_id = strip_or_none(owner.and_then(|owner| owner.get("_id")));
        let tags: Vec<Value> = match bite.get("userKeywords") {
            Some(Value::Array(keywords)) => keywords
                .iter()
                .filter_map(|keyword| match strip_or_none(Some(keyword)) {
                    Value::Null => None,
                    stripped => Some(stripped),
                })
                .collect(),
            _ => Vec::new(),
        };
        let tags = if tags.is_empty() {
            Value::Null
        } else {
            Value::Array(tags)
        };

        // A bare direct URL: its one format is numbered `0` (`process_video_result`).
        let mut info =
            direct::with_format(direct::build_at(media.clone(), &media, "mp3".into()), "");
        info.direct = None;
        info.id = id.as_str().unwrap_or_default().to_owned();
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = KEY.into();
        for (key, value) in [
            ("uploader", uploader),
            ("uploader_id", uploader_id),
            ("tags", tags),
        ] {
            if !value.is_null() {
                crate::metadata_parser::set_field(&mut info, key, value);
            }
        }
        if let Some(title) = title.as_str() {
            info.title = title.to_owned();
        }
        Ok(Some(crate::info_order::mark_info(
            info,
            "untyped-dict;id,url,title,uploader,uploader_id,ext,tags",
        )))
    }
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn suitable(url: &str) -> bool {
        VALID_URL.is_match(url)
    }

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://blerp.com/soundbites/6320fe8745636cb4dd677a5a"
        ));
        assert!(suitable(
            "https://www.blerp.com/soundbites/5bc94ef4796001000498429f"
        ));
        assert!(!suitable("https://blerp.com/soundbites/"));
    }
}
