//! Upstream `AlJazeeraIE` (`yt_dlp/extractor/aljazeera.py`): Al Jazeera
//! articles, programmes and videos through the site's GraphQL
//! `ArchipelagoSingleArticleQuery` (else the page's Brightcove player URL)
//! as a `url_transparent` result of the Brightcove player.
use super::{Extractor, download_json_headers, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct AlJazeera;

const NAME: &str = "AlJazeera";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?P<base>\w+\.aljazeera\.\w+)/(?P<type>programs?/[^/]+|(?:feature|video|new)s)?/\d{4}/\d{1,2}/\d{1,2}/(?P<id>[^/?&#]+)",
    )
    .unwrap()
});
static BRIGHTCOVE_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"https?://players.brightcove.net/(?P<account>\d+)/(?P<player_id>[a-zA-Z0-9]+)_(?P<embed>[^/]+)/index.html\?videoId=(?P<id>\d+)",
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Al Jazeera {what} are not supported natively"))
}

/// `str(value)` in an f-string.
fn py_str(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        _ => Err(unsupported(
            "Brightcove IDs that are not strings or numbers",
        )),
    }
}

impl Extractor for AlJazeera {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let base = found["base"].to_owned();
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let wp = match base.as_str() {
            "balkans.aljazeera.net" => "ajb",
            "chinese.aljazeera.net" => "chinese",
            "mubasher.aljazeera.net" => "ajm",
            _ => "aje",
        };
        // `post_type.split('/')[0]` of a missing type is an uncaught
        // `AttributeError`.
        let Some(post_type) = found.name("type") else {
            return Err(Error::Reported(
                "'NoneType' object has no attribute 'split'".into(),
            ));
        };
        let post_type = match post_type.as_str().split('/').next().unwrap_or_default() {
            "features" => "post",
            "program" | "programs" => "episode",
            "videos" => "video",
            "news" => "news",
            other => {
                return Err(Error::Reported(format!(
                    "{display_id}: An extractor error has occurred. (caused by KeyError('{other}')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let mut api = Url::parse(&format!("https://{base}/graphql"))?;
        api.query_pairs_mut()
            .append_pair("wp-site", wp)
            .append_pair("operationName", "ArchipelagoSingleArticleQuery")
            .append_pair(
                "variables",
                &crate::output::python_json_dumps(
                    &json!({"name": display_id, "postType": post_type}),
                ),
            );
        let data = download_json_headers(client, &api, NAME, &display_id, &[("wp-site", wp)])?;
        let video = data
            .pointer("/data/article/video")
            .filter(|video| super::nuxt::truthy(video))
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let video_id = video.get("id").filter(|id| !id.is_null());
        let target = match video_id {
            Some(video_id) => {
                let text = |key: &str, default: &str| match video
                    .get(key)
                    .filter(|value| super::nuxt::truthy(value))
                {
                    Some(value) => py_str(value),
                    None => Ok(default.to_owned()),
                };
                format!(
                    "https://players.brightcove.net/{}/{}_default/index.html?videoId={}",
                    text("accountId", "911432371001")?,
                    text("playerId", "csvTfAlKW")?,
                    py_str(video_id)?
                )
            }
            None => {
                let webpage = download_webpage(client, url, NAME, &display_id)?;
                let Some(player) = BRIGHTCOVE_URL.captures(&webpage) else {
                    // Upstream hands the page to GenericIE.
                    return Err(unsupported(
                        "pages without a Brightcove player (handed to the generic extractor upstream)",
                    ));
                };
                format!(
                    "https://players.brightcove.net/{}/{}_{}/index.html?videoId={}",
                    &player["account"], &player["player_id"], &player["embed"], &player["id"]
                )
            }
        };
        // `{'_type': 'url_transparent', 'url': ..., 'ie_key': 'BrightcoveNew'}`.
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": {}}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://balkans.aljazeera.net/videos/2021/11/6/pojedini-domovi-u-sarajevu-jos-pod-vodom"
        ));
        assert!(suitable(
            "https://www.aljazeera.com/program/the-stream/2021/11/6/x"
        ));
        // Without a type upstream needs an empty path segment.
        assert!(!suitable("https://www.aljazeera.com/2021/11/6/x"));
        assert!(suitable("https://www.aljazeera.com//2021/11/6/x"));
        assert!(!suitable("https://www.aljazeera.com/videos/"));
    }
}
