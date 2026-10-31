//! Upstream `FranceCultureIE` (`yt_dlp/extractor/radiofrance.py`): a podcast
//! episode page's `AudioObject` JSON as a direct audio result with the page's
//! title, description, image, author and date. The other `RadioFrance*`
//! classes (radiovisions, live streams, playlists and schedules) are not
//! ported.
use super::{Extractor, download_webpage, search_warning};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct FranceCulture;

const NAME: &str = "FranceCulture";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?radiofrance\.fr/(?:franceculture|franceinfo|franceinter|francemusique|fip|mouv)/podcasts/(?:[^?#]+/)?(?P<display_id>[^?#]+)-(?P<id>\d{6,})(?:$|[?#])",
    )
    .unwrap()
});
// `_search_json('', webpage, 'audio data', ...,
// contains_pattern=r'{\s*"@type"\s*:\s*"AudioObject".+}')`.
static AUDIO_OBJECT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\{\s*"@type"\s*:\s*"AudioObject".+\}"#).unwrap());
static TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?s)<h1[^>]*itemprop="[^"]*name[^"]*"[^>]*>(.+?)</h1>"#).unwrap()
});
static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<meta name="description"\s*content="([^"]+)"#).unwrap());
static UPLOADER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?s)<span class="author">(.*?)</span>"#).unwrap());
static DATE_PUBLISHED: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#""datePublished"\s*:\s*"([^"]+)"#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_html_search_regex(pattern, webpage, name, default=...)`: the first
/// group, tags stripped and entities unescaped.
fn html_search(pattern: &Regex, webpage: &str) -> Option<String> {
    pattern
        .captures(webpage)
        .map(|found| crate::html::clean_html(&found[1]))
}

/// `_html_search_regex(...)` where `None` is a raw `_search_regex` match.
fn raw_search(pattern: &Regex, webpage: &str) -> Option<String> {
    pattern.captures(webpage).map(|found| found[1].to_owned())
}

impl Extractor for FranceCulture {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        let display_id = found["display_id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;

        let Some(audio) = AUDIO_OBJECT.find(&webpage) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract audio data; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        // `_parse_json(ignore_extra=True)`: the first JSON value only.
        let audio: Map<String, Value> = match serde_json::Deserializer::from_str(audio.as_str())
            .into_iter::<Value>()
            .next()
        {
            Some(Ok(Value::Object(audio))) => audio,
            _ => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {display_id}: Unable to extract audio data - Failed to parse JSON"
                )));
            }
        };
        let content_url = match audio.get("contentUrl") {
            Some(Value::String(content_url)) => content_url.clone(),
            _ => {
                return Err(Error::Reported(format!(
                    "{display_id}: An extractor error has occurred. (caused by KeyError('contentUrl')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let media = Url::parse(&super::proto_relative_http(&content_url))?;

        // `default=self._og_search_title(webpage)` is evaluated eagerly.
        let og_title = crate::html::og_search_property(&webpage, "title");
        if og_title.is_none() {
            search_warning(NAME, "OpenGraph title");
        }
        let title = html_search(&TITLE, &webpage).or(og_title);
        let description = html_search(&DESCRIPTION, &webpage);
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        if thumbnail.is_none() {
            search_warning(NAME, "OpenGraph thumbnail");
        }
        let uploader = html_search(&UPLOADER, &webpage);
        let upload_date = raw_search(&DATE_PUBLISHED, &webpage)
            .and_then(|date| crate::dates::unified_strdate(&date, false));

        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "mp3".into()),
            "",
        );
        info.direct = None;
        info.id = video_id.clone();
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        crate::metadata_parser::set_field(&mut info, "display_id", Value::String(display_id));
        // `'vcodec': 'none' if encodingFormat == 'mp3' else None`.
        let vcodec = match audio.get("encodingFormat") {
            Some(Value::String(format)) if format == "mp3" => Value::String("none".into()),
            _ => Value::Null,
        };
        crate::metadata_parser::set_field(&mut info, "vcodec", vcodec);
        let duration = audio
            .get("duration")
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
            .map_or(Value::Null, |duration| {
                serde_json::to_value(duration).unwrap_or(Value::Null)
            });
        crate::metadata_parser::set_field(&mut info, "duration", duration);
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        crate::metadata_parser::set_field(
            &mut info,
            "description",
            description.map_or(Value::Null, Value::String),
        );
        info.metadata.thumbnail = thumbnail;
        crate::metadata_parser::set_field(
            &mut info,
            "uploader",
            uploader.map_or(Value::Null, Value::String),
        );
        crate::metadata_parser::set_field(
            &mut info,
            "upload_date",
            upload_date.map_or(Value::Null, Value::String),
        );
        Ok(Some(crate::info_order::mark_info(
            info,
            "untyped-dict;id,display_id,url,vcodec,duration,title,description,thumbnail,uploader,upload_date",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.radiofrance.fr/franceculture/podcasts/science-en-questions/la-physique-d-einstein-aiderait-elle-a-comprendre-le-cerveau-8440487"
        ));
        assert!(suitable(
            "https://www.radiofrance.fr/franceinter/podcasts/le-7-9-30/le-7-9-30-du-vendredi-10-mars-2023-2107675"
        ));
        assert!(!suitable(
            "https://www.radiofrance.fr/franceculture/podcasts/le-billet"
        ));
        assert!(!suitable("https://www.radiofrance.fr/franceinter/"));
    }
}
