//! Upstream `TNAFlixNetworkEmbedIE` and `TNAFlixIE` (`yt_dlp/extractor/tnaflix.py`):
//! `player.tnaflix.com` embeds as a url result to the TNAFlix page, and the
//! page's AJAX video player `<source>` MP4s. Pages with a config XML (the
//! `flashvars`/`config`/`vkey`+`nkey` branch) are not supported natively.
use super::{Extractor, dict_format, dict_info, download_json_headers, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct TNAFlixNetworkEmbed;
pub struct TNAFlix;

const NAME: &str = "TNAFlix";

static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://player\.(?P<host>tnaflix)\.com/video/(?P<id>\d+)").unwrap()
});
static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?P<host>tnaflix)\.com/[^/]+/(?P<display_id>[^/]+)/video(?P<id>\d+)",
    )
    .unwrap()
});
static CONFIG: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"flashvars\.config\s*=\s*escape\("[^"]+"|<input[^>]+name="config\d?" value="[^"]+"|config\s*=\s*["'](?:https?:)?//"#,
    )
    .unwrap()
});
static TITLE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<title>(.+?) - (?:TNAFlix Porn Videos|TNAFlix\.com)</title>").unwrap()
});
static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)>Description:</[^>]+>(.+?)<").unwrap());
static UPLOADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<span>by\s*<a[^>]+\bhref=["']/profile/[^>]+>([^<]+)<"#).unwrap()
});
static CATEGORIES: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<span[^>]*>Categories:</span>(.+?)</div>").unwrap());
static SOURCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"<source src="([^"]+)""#).unwrap());
static HEIGHT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"-(\d+)p\.").unwrap());
static POSTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"data-poster="([^"]+)""#).unwrap());

/// `_match_valid_url` for the embed (TNAFlix hosts only; EMPFlix is not ported).
pub fn embed_suitable(url: &str) -> bool {
    EMBED_URL.is_match(url)
}

/// `_match_valid_url` for the TNAFlix page.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("TNAFlix {what} are not supported natively"))
}

/// `int_or_none` of a digits string.
fn int_or_none(text: &str) -> Option<i64> {
    text.parse().ok()
}

/// `_html_search_regex`: the first group, with tags stripped and entities
/// unescaped.
fn search_clean(pattern: &LazyLock<Regex>, html: &str) -> Option<String> {
    pattern
        .captures(html)
        .map(|found| crate::html::clean_html(&found[1]))
}

impl Extractor for TNAFlixNetworkEmbed {
    fn key(&self) -> &'static str {
        "TNAFlixNetworkEmbed"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = EMBED_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // Upstream `url_result('http://www.{host}.com/category/{id}/video{id}')`.
        let target = format!(
            "http://www.{}.com/category/{id}/video{id}",
            &found["host"],
            id = &found["id"]
        );
        Ok(Some(super::follow_url(url, target)))
    }
}

/// `TNAFlixIE._real_extract` for pages without a config XML.
fn extract_page(client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
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
    let hidden = super::login::hidden_inputs(&webpage);
    let has_hidden = |name: &str| hidden.iter().any(|(key, _)| key == name);
    if CONFIG.is_match(&webpage)
        || has_hidden("config")
        || (has_hidden("vkey") && has_hidden("nkey"))
    {
        return Err(unsupported("pages with a config XML"));
    }

    // Upstream's EMPFlix-style branch: the AJAX video player's `<source>`s.
    let player_url = Url::parse(&format!(
        "http://www.{}.com/ajax/video-player/{video_id}",
        &found["host"]
    ))?;
    let player = download_json_headers(
        client,
        &player_url,
        NAME,
        &display_id,
        &[("Referer", url.as_str())],
    )?;
    let player = player
        .get("html")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();

    let mut formats = Vec::new();
    for source in SOURCE.captures_iter(&player) {
        let video_url = super::proto_relative_http(&source[1]);
        let basename = crate::info::url_basename(&Url::parse(&video_url)?);
        let height = HEIGHT
            .captures(&basename)
            .and_then(|found| int_or_none(&found[1]));
        let ext = basename.rsplit('.').next().unwrap_or_default().to_owned();
        let format_id = match height {
            Some(height) => format!("{height}p"),
            None => basename.split('.').next().unwrap_or_default().to_owned(),
        };
        formats.push(dict_format(
            vec![
                ("url", video_url.into()),
                ("ext", ext.into()),
                ("height", height.map_or(Value::Null, Value::from)),
                ("format_id", format_id.into()),
            ],
            NAME,
            &video_id,
        )?);
    }

    // `_search_json_ld(webpage, display_id, default={})`.
    let json_ld = super::json_ld::search(&webpage);
    let json_string = |key: &str| json_ld.get(key).cloned().unwrap_or(Value::Null);

    let title = search_clean(&TITLE, &webpage)
        .or_else(|| crate::html::og_search_property(&webpage, "title"))
        .map_or_else(|| json_string("title"), Value::from);
    let description = search_clean(&DESCRIPTION, &webpage)
        .map_or_else(|| json_string("description"), Value::from);
    let duration = match crate::html::html_search_meta(&webpage, "duration")
        .and_then(|text| crate::parse_duration(&text))
    {
        Some(number) => json!(number),
        None => json_string("duration"),
    };
    let uploader =
        search_clean(&UPLOADER, &webpage).map_or_else(|| json_string("uploader"), Value::from);
    let age_limit = crate::html::rta_search(&webpage)
        .filter(|age| *age != 0)
        .unwrap_or(18);
    let categories: Vec<String> = search_clean(&CATEGORIES, &webpage)
        .unwrap_or_default()
        .split(',')
        .map(|category| category.trim().to_owned())
        .collect();
    let thumbnails = match POSTER.captures(&player) {
        Some(found) => json!([{"url": super::proto_relative_http(&found[1])}]),
        None => Value::Null,
    };

    Ok(Some(dict_info(
        url,
        (NAME, NAME),
        vec![
            ("id", video_id.clone().into()),
            ("display_id", display_id.clone().into()),
            ("title", title),
            ("description", description),
            ("thumbnails", thumbnails),
            ("duration", duration),
            ("age_limit", age_limit.into()),
            ("uploader", uploader),
            ("view_count", Value::Null),
            ("comment_count", Value::Null),
            ("average_rating", Value::Null),
            ("categories", categories.into()),
        ],
        formats,
    )))
}

impl Extractor for TNAFlix {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        extract_page(client, url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.tnaflix.com/teen-porn/Educational-xxx-video/video6538"
        ));
        assert!(suitable(
            "http://www.tnaflix.com/porn-stars/Carmella-Decesare-striptease/video553878"
        ));
        assert!(!suitable(
            "http://www.empflix.com/videos/Amateur-Finger-Fuck-33051.html"
        ));
        assert!(embed_suitable("https://player.tnaflix.com/video/6538"));
        assert!(!embed_suitable("http://player.empflix.com/video/33051"));
    }
}
