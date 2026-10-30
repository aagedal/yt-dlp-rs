//! Upstream `FC2IE` and `FC2EmbedIE` (`yt_dlp/extractor/fc2.py`): a content
//! page's title and thumbnail, and the `videoplaylist` API's stream URL (an
//! `m3u8_native` or direct URL as the result's own URL), plus the `flv2.swf`
//! embed as a `url_transparent` result of `fc2:ID`. FC2LiveIE (a WebSocket
//! session) and the login (`username`/`password`) path are not ported.
use super::{Extractor, download_json, download_webpage, search_warning};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Fc2;
pub struct Fc2Embed;

const NAME: &str = "fc2";
const KEY: &str = "FC2";
const EMBED_NAME: &str = "fc2:embed";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:https?://video\.fc2\.com/(?:[^/]+/)*content/|fc2:)(?P<id>[^/]+)").unwrap()
});
static EMBED_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://video\.fc2\.com/flv2\.swf\?(?P<query>.+)").unwrap());
static TITLE_PATTERNS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"<h2\s+class="videoCnt_title">([^<]+?)</h2>"#).unwrap(),
        Regex::new(r#"\s+href="[^"]+"\s*title="([^"]+?)"\s*rel="nofollow">\s*<img"#).unwrap(),
    ]
});
/// The third title pattern, whose `\1` back-reference the regex crate lacks:
/// the title must repeat right after the match.
static REPEATED_TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"\s+href="[^"]+"\s*title="([^"]+?)"\s*rel="nofollow">\s*"#).unwrap()
});

/// `FC2IE._match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `FC2EmbedIE._match_valid_url`.
pub fn embed_suitable(url: &str) -> bool {
    EMBED_URL.is_match(url)
}

/// `_html_search_regex((...), webpage, 'title', fatal=False)`.
fn search_title(webpage: &str) -> Option<String> {
    for pattern in TITLE_PATTERNS.iter() {
        if let Some(found) = pattern.captures(webpage) {
            return Some(crate::html::unescape_html(&found[1]));
        }
    }
    REPEATED_TITLE.captures_iter(webpage).find_map(|found| {
        let title = found[1].to_owned();
        let end = found.get(0)?.end();
        webpage[end..]
            .starts_with(&title)
            .then(|| crate::html::unescape_html(&title))
    })
}

impl Extractor for Fc2 {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // Login (`_login`) needs credentials this port does not read.
        let webpage = if url.as_str().starts_with("fc2:") {
            None
        } else {
            Some(download_webpage(client, url, NAME, &video_id)?)
        };

        let mut title = None;
        let mut thumbnail = None;
        let mut description = None;
        if let Some(webpage) = &webpage {
            title = search_title(webpage);
            if title.is_none() {
                search_warning(NAME, "title");
            }
            thumbnail = crate::html::og_search_property(webpage, "image");
            if thumbnail.is_none() {
                search_warning(NAME, "thumbnail URL");
            }
            description = crate::html::og_search_property(webpage, "description");
        }

        let playlist_url = Url::parse(&format!(
            "https://video.fc2.com/api/v3/videoplaylist/{video_id}?sh=1&fs=0"
        ))?;
        let vidplaylist = download_json(client, &playlist_url, NAME, &video_id)?;
        let stream = vidplaylist
            .get("playlist")
            .and_then(|playlist| playlist.get("nq"))
            .and_then(Value::as_str)
            .filter(|nq| !nq.is_empty());
        let Some(stream) = stream else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract video URL; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let base = Url::parse("https://video.fc2.com/")?;
        let stream_url = base.join(stream)?;
        let protocol = if vidplaylist.get("type").and_then(Value::as_i64) == Some(2) {
            "m3u8_native"
        } else {
            "https"
        };

        // A url-only result (upstream's top-level `url` with no formats): the
        // one media format carries the URL, and the info keeps its own keys.
        let mut info = super::direct::with_format(
            super::direct::build_at(stream_url.clone(), &stream_url, "mp4".into()),
            "",
        );
        for format in &mut info.formats {
            format.ext = "mp4".into();
            format.format_id = String::new();
            format.protocol = Some(protocol.into());
        }
        info.format_id = None;
        info.id = video_id;
        let mut keys = vec!["id"];
        let titled = title.is_some();
        match title {
            Some(title) => {
                info.title = title;
                keys.push("title");
            }
            None => {
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
                keys.push("title");
            }
        }
        keys.extend(["url", "ext", "protocol"]);
        crate::metadata_parser::set_field(&mut info, "ext", json!("mp4"));
        crate::metadata_parser::set_field(&mut info, "protocol", json!(protocol));
        // `None` values keep their keys (upstream's `"description": null`).
        keys.push("description");
        crate::metadata_parser::set_field(
            &mut info,
            "description",
            description.map_or(Value::Null, Value::from),
        );
        keys.push("thumbnail");
        crate::metadata_parser::set_field(
            &mut info,
            "thumbnail",
            thumbnail.map_or(Value::Null, Value::from),
        );
        info.direct = None;
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = KEY.into();
        info.extra.insert(
            crate::info_order::OWN_URL.into(),
            json!(stream_url.as_str()),
        );
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!(
                "untyped-dict;{}{}",
                keys.join(","),
                if titled { "" } else { "-untitled" }
            ),
        )))
    }
}

impl Extractor for Fc2Embed {
    fn key(&self) -> &'static str {
        "FC2Embed"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = EMBED_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(EMBED_NAME);
        // `urllib.parse.parse_qs`: the last `i` value is the video ID.
        let query: Vec<(String, String)> = url::form_urlencoded::parse(found["query"].as_bytes())
            .into_owned()
            .collect();
        let get = |key: &str| {
            query
                .iter()
                .filter(|(name, _)| name == key)
                .map(|(_, value)| value.clone())
                .next_back()
        };
        let Some(video_id) = query
            .iter()
            .filter(|(name, _)| name == "i")
            .map(|(_, value)| value.clone())
            .next_back()
        else {
            return Err(Error::Reported(format!(
                "[{EMBED_NAME}] Unable to extract video id; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let title = get("tl").unwrap_or_else(|| format!("FC2 video {video_id}"));
        // See `thumbnailImagePath()` in `ServerConst.as` of `flv2.swf`.
        let thumbnail = get("sj").filter(|sj| !sj.is_empty()).map(|sj| {
            let chars: Vec<char> = video_id.chars().collect();
            let slice = |from: usize, to: usize| -> String {
                chars[from.min(chars.len())..to.min(chars.len())]
                    .iter()
                    .collect()
            };
            let last = chars.len().saturating_sub(2);
            format!(
                "http://video{sj}-thumbnail.fc2.com/up/pic/{}/{}/{}/{}/{video_id}.jpg",
                slice(0, 6),
                slice(6, 8),
                slice(last, last + 1),
                slice(last + 1, last + 2),
            )
        });

        // The `url_transparent` result: `fc2:ID` (FC2IE) with these fields.
        let mut fields = serde_json::Map::new();
        fields.insert("title".into(), title.into());
        if let Some(thumbnail) = thumbnail {
            fields.insert("thumbnail".into(), thumbnail.into());
        }
        let target = format!("fc2:{video_id}");
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": target, "fields": Value::Object(fields)}),
        );
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://video.fc2.com/en/content/20121103kUan1KHs"));
        assert!(suitable("https://video.fc2.com/content/20121209FP73fxDx"));
        assert!(suitable(
            "http://video.fc2.com/en/a/content/20130926eZpARwsF"
        ));
        assert!(suitable("fc2:20121103kUan1KHs"));
        assert!(!suitable("https://video.fc2.com/"));
        assert!(embed_suitable(
            "http://video.fc2.com/flv2.swf?t=1&i=201403223kCqB3Ez&sj=11"
        ));
    }
}
