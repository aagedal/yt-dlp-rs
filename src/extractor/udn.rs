//! Upstream `UDNEmbedIE` (`yt_dlp/extractor/udn.py`): UDN video embeds from
//! the player page's `options`: a YouTube video as a `url` result, else each
//! video type's resolved URL (HLS, HDS or MP4 with height and bitrate), with
//! the title and poster; UDN embed iframes are embeds.
use super::{Extractor, dict_format, dict_info, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct UdnEmbed;

pub(crate) const NAME: &str = "UDNEmbed";
pub(crate) const KEY: &str = "UDNEmbed";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://video\.udn\.com/(?:embed|play)/news/(?P<id>\d+)").unwrap()
});
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+src="(?:https?:)?(?P<url>//video\.udn\.com/(?:embed|play)/news/\d+)""#,
    )
    .unwrap()
});
static OPTIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"var\s+options\s*=\s*([^;]+);").unwrap());
static MP4: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"_(?P<height>\d+)p_(?P<tbr>\d+)\.mp4").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls` under `orderedSet`.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Ok(embed) = url.join(&crate::html::unescape_html(&captures["url"])) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("UDN {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for UdnEmbed {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
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
        let page = download_webpage(client, url, NAME, &video_id)?;
        let options_str = OPTIONS
            .captures(&page)
            .map(|found| crate::html::clean_html(&found[1]))
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract options; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let options: Value = crate::json::from_str(&crate::js_to_json::js_to_json(&options_str))
            .map_err(|_| unsupported("player options that do not parse"))?;
        if !super::nuxt::truthy(&options) {
            return Err(unsupported("empty player options"));
        }
        let video_urls = match options.get("video") {
            Some(Value::Object(urls)) => urls.clone(),
            Some(_) => return Err(unsupported("video maps that are not objects")),
            None => return Err(key_error(&video_id, "video")),
        };
        let title = options
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "title"))?;
        let poster = options.get("poster").cloned().unwrap_or(Value::Null);
        if let Some(youtube) = video_urls
            .get("youtube")
            .filter(|value| super::nuxt::truthy(value))
        {
            let youtube = youtube
                .as_str()
                .ok_or_else(|| unsupported("YouTube URLs that are not strings"))?;
            return Ok(Some(super::follow_url(url, youtube.to_owned())));
        }
        let mut formats = Vec::new();
        for (video_type, api_url) in &video_urls {
            if !super::nuxt::truthy(api_url) {
                continue;
            }
            let api_url = api_url
                .as_str()
                .ok_or_else(|| unsupported("video API URLs that are not strings"))?;
            let api = url.join(api_url)?;
            let video_url = super::download_webpage_note(
                client,
                &api,
                NAME,
                &video_id,
                &format!("retrieve url for {video_type} video"),
            )?;
            match super::direct::determine_ext(&video_url).as_deref() {
                Some("m3u8") => {
                    let manifest = Url::parse(&video_url)
                        .map_err(|_| unsupported("playlist URLs that are not URLs"))?;
                    let stream = super::extract_m3u8_fatal_master(
                        client,
                        &manifest,
                        NAME,
                        &video_id,
                        Some("hls"),
                        false,
                    )?;
                    formats.extend(stream.formats);
                }
                Some("f4m") => {
                    let manifest = Url::parse(&video_url)
                        .map_err(|_| unsupported("HDS manifest URLs that are not URLs"))?;
                    formats.extend(super::extract_f4m_optional(
                        client,
                        NAME,
                        &manifest,
                        &video_id,
                        Some("hds"),
                    )?);
                }
                _ => {
                    let mut fields = vec![
                        ("url", Value::from(video_url.clone())),
                        ("format_id", format!("http-{video_type}").into()),
                    ];
                    let mut int_rates = false;
                    if let Some(found) = MP4.captures(&video_url) {
                        fields.push((
                            "height",
                            found["height"]
                                .parse::<i64>()
                                .map_or(Value::Null, Value::from),
                        ));
                        fields.push((
                            "tbr",
                            found["tbr"].parse::<i64>().map_or(Value::Null, Value::from),
                        ));
                        int_rates = true;
                    }
                    let mut format = dict_format(fields, NAME, &video_id)?;
                    if int_rates {
                        crate::info_order::mark_int_rates(&mut format);
                    }
                    formats.push(format);
                }
            }
        }
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("formats", Value::Null),
                ("title", title),
                ("thumbnail", poster),
            ],
            formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://video.udn.com/embed/news/300040"));
        assert!(suitable("https://video.udn.com/play/news/303776"));
        assert!(!suitable("https://video.udn.com/news/1308561"));
    }
}
