//! Upstream `CtsNewsIE` (`yt_dlp/extractor/ctsnews.py`): a 華視新聞 article's
//! CTSPlayer video (its `get_id` hidden input's MP4 feed `source_url`), else
//! the first YouTube embed of the page as a `url` result.
use super::{Extractor, download_webpage, follow_url, login, search_warning};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct CtsNews;

const NAME: &str = "CtsNews";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://news\.cts\.com\.tw/[a-z]+/[a-z]+/\d+/(?P<id>\d+)\.html").unwrap()
});
static DATETIME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{4}/\d{2}/\d{2} \d{2}:\d{2})").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_html_search_meta(name, page)` (non-fatal).
fn meta(page: &str, name: &str) -> Option<String> {
    let value = crate::html::html_search_meta(page, name);
    if value.is_none() {
        search_warning(NAME, name);
    }
    value
}

impl Extractor for CtsNews {
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
        let page_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let page = download_webpage(client, url, NAME, &page_id)?;

        // `self._hidden_inputs(page).get('get_id')`: the last value wins.
        let get_id = login::hidden_inputs(&page)
            .into_iter()
            .filter(|(name, _)| name == "get_id")
            .map(|(_, value)| value)
            .next_back()
            .filter(|value| !value.is_empty());
        let Some(news_id) = get_id else {
            crate::screen::screen(&format!("[{NAME}] Not CTSPlayer video, trying Youtube..."));
            let youtube_url = super::youtube::embed_urls(url, &page)
                .into_iter()
                .next()
                .ok_or_else(|| {
                    Error::UnsupportedHtml5(
                        "CtsNews pages without a CTSPlayer or YouTube embed are not supported natively"
                            .into(),
                    )
                })?;
            return Ok(Some(follow_url(url, youtube_url.to_string())));
        };

        let feed = super::update_url_query(
            &Url::parse("http://news.cts.com.tw/action/test_mp4feed.php")?,
            &[("news_id", news_id.as_str())],
        );
        let mp4_feed = super::download_json_note(client, &feed, NAME, &news_id, "Fetching feed")?;
        let video_url = match mp4_feed.get("source_url") {
            Some(Value::String(source)) => source.clone(),
            _ => {
                return Err(super::clyp::key_error(&news_id, "source_url"));
            }
        };

        let description = meta(&page, "description");
        let title = crate::html::html_search_meta(&page, "title").ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {news_id}: Unable to extract title; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let thumbnail = meta(&page, "image");

        let timestamp = match DATETIME.captures(&page) {
            Some(found) => crate::dates::unified_timestamp(&found[1], true).map(|t| t - 8 * 3600),
            None => {
                search_warning(NAME, "date and time");
                None
            }
        };

        // A bare `url` result (upstream's untyped dict): its one format is
        // the media URL itself, as for a generic direct link.
        let media = Url::parse(&video_url)?;
        let ext = super::direct::determine_ext(media.as_str()).unwrap_or_default();
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, ext.clone()),
            "",
        );
        for format in &mut info.formats {
            format.ext = ext.clone();
        }
        info.id = news_id.clone();
        info.title = title.clone();
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let mut keys = vec!["id", "url", "title"];
        let fields = [
            ("description", description.map_or(Value::Null, Value::from)),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
        ];
        for (key, value) in fields {
            keys.push(key);
            crate::metadata_parser::set_field(&mut info, key, value);
        }
        Ok(Some(ExtractResult::Video(crate::info_order::mark_info(
            info,
            &format!("untyped-dict;{}", keys.join(",")),
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://news.cts.com.tw/cts/international/201501/201501291578109.html"
        ));
        assert!(!suitable("http://news.cts.com.tw/"));
    }
}
