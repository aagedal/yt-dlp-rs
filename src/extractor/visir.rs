//! Upstream `VisirIE` (`yt_dlp/extractor/visir.py`): Vísir clips from the
//! page's `App.Player.Init(...)` options (an HLS playlist on
//! `vod.visir.is`), the article date, description and view count; player
//! pages follow their canonical clip URL, and player iframes on other pages
//! are followed.
use super::{Extractor, dict_info, download_webpage, extract_m3u8_fatal_master};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Visir;

pub(crate) const NAME: &str = "Visir";
pub(crate) const KEY: &str = "Visir";

const VALID_PATTERN: &str =
    r"https?://(?:www\.)?visir\.is/(?P<type>k|player)/(?P<id>[\da-f-]+)(?:/(?P<slug>[\w.-]+))?";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| crate::extractor::url_regex(&format!("^{VALID_PATTERN}")).unwrap());
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(r#"<iframe[^>]+src=["'](?P<url>{VALID_PATTERN})"#)).unwrap()
});
static PLAYER_INIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"App\.Player\.Init\(").unwrap());
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[0-9]{4}$").unwrap());

const MONTHS: [&str; 12] = [
    "janúar",
    "febrúar",
    "mars",
    "apríl",
    "maí",
    "júní",
    "júlí",
    "ágúst",
    "september",
    "október",
    "nóvember",
    "desember",
];

/// `_match_valid_url`.
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
    Error::UnsupportedHtml5(format!("Vísir {what} are not supported natively"))
}

/// `find_element(cls=...)`: the first element's content.
fn by_class(class: &str, html: &str) -> Option<String> {
    crate::html_elements::elements_text_and_html_by_class(class, html)
        .ok()
        .and_then(|found| found.into_iter().next())
        .map(|(text, _)| text)
}

/// The `article-item__date` element's `D. MONTH YYYY` (Icelandic months).
fn upload_date(webpage: &str) -> Option<String> {
    let text = crate::html::clean_html(&by_class("article-item__date", webpage)?);
    let parts: Vec<&str> = text.split_whitespace().collect();
    let [day, month, year] = parts[..] else {
        return None;
    };
    let day: i64 = day.trim_end_matches('.').parse().ok()?;
    let month = MONTHS.iter().position(|name| *name == month)? + 1;
    (day != 0 && YEAR.is_match(year)).then(|| format!("{year}{month:02}{day:02}"))
}

impl Extractor for Visir {
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
        let video_type = found["type"].to_owned();
        let video_id = found["id"].to_owned();
        let display_id = found.name("slug").map(|slug| slug.as_str().to_owned());
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        if video_type == "player" {
            let real_url = crate::html::og_search_property(&webpage, "url");
            let Some(real_url) = real_url else {
                return Err(unsupported("player pages without an OpenGraph URL"));
            };
            let target = VALID_URL.captures(&real_url);
            if target
                .as_ref()
                .is_none_or(|target| &target["type"] == "player")
            {
                // `UnsupportedError(real_url)` keeps its plain message.
                return Err(Error::Reported(format!("Unsupported URL: {real_url}")));
            }
            // `url_result(real_url, self.ie_key())`.
            let mut follow = super::direct::build_at(url.clone(), url, String::new());
            follow.formats.clear();
            follow.direct = None;
            follow
                .extra
                .insert(crate::FOLLOW_URL.into(), real_url.into());
            return Ok(Some(ExtractResult::Video(follow)));
        }
        let upload_date = upload_date(&webpage);
        // `_search_json(r'App\.Player\.Init\(', webpage, video_id, 'player',
        // ...)`: upstream passes the ID as the name.
        let player = match super::search_json(&PLAYER_INIT, &webpage, true) {
            Some(Some(player)) => player,
            Some(None) => {
                return Err(Error::Reported(format!(
                    "[{NAME}] player: Unable to extract {video_id} - Failed to parse JSON"
                )));
            }
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract {video_id}; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        // `traverse_obj(player, ('File', {urljoin('https://vod.visir.is/')}))`.
        let m3u8_url = player
            .get("File")
            .and_then(Value::as_str)
            .and_then(|file| super::videopress::urljoin(Some("https://vod.visir.is/"), file));
        let Some(m3u8_url) = m3u8_url else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Failed to obtain m3u8 URL; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let stream = extract_m3u8_fatal_master(
            client,
            &Url::parse(&m3u8_url).map_err(|_| unsupported("player files that are not URLs"))?,
            NAME,
            &video_id,
            None,
            false,
        )?;
        let mut fields = vec![
            ("id", Value::from(video_id.clone())),
            ("display_id", display_id.map_or(Value::Null, Value::from)),
            ("formats", Value::Null),
            ("upload_date", upload_date.map_or(Value::Null, Value::from)),
        ];
        if let Some(article) = by_class("article-item press-ads", &webpage) {
            if let Some(description) = by_class("-large", &article)
                .map(|text| crate::html::clean_html(&text))
                .filter(|text| !text.is_empty())
            {
                fields.push(("description", description.into()));
            }
            if let Some(views) = by_class("article-item__viewcount", &article)
                .and_then(|text| crate::html::clean_html(&text).parse::<i64>().ok())
            {
                fields.push(("view_count", views.into()));
            }
        }
        if let Some(title) = player.get("Title").and_then(Value::as_str) {
            fields.push(("title", crate::html::clean_html(title).into()));
        }
        if let Some(category) = player
            .get("Categoryname")
            .and_then(Value::as_str)
            .map(crate::html::clean_html)
            .filter(|category| !category.is_empty())
        {
            fields.push(("categories", Value::Array(vec![category.into()])));
        }
        let duration = match player.get("MediaDuration") {
            Some(Value::Number(number)) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
            Some(Value::String(text)) => text.trim().parse().ok(),
            _ => None,
        };
        if let Some(duration) = duration {
            fields.push(("duration", duration.into()));
        }
        if let Some(image) = player
            .get("Image")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            fields.push(("thumbnail", image.into()));
        }
        Ok(Some(ExtractResult::Video(dict_info(
            url,
            (NAME, KEY),
            fields,
            stream.formats,
        ))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.visir.is/k/eabb8f7f-ad87-46fb-9469-a0f1dc0fc4bc-1769022963988"
        ));
        assert!(suitable(
            "https://www.visir.is/player/0cd5709e-6870-46d0-aaaf-0ae637de94f1-1770060083580"
        ));
        assert!(!suitable("https://www.visir.is/g/20262837896d/x"));
    }

    #[test]
    fn dates_use_icelandic_month_names() {
        let page = r#"<span class="article-item__date">21. janúar 2026</span>"#;
        assert_eq!(upload_date(page).as_deref(), Some("20260121"));
        assert_eq!(
            upload_date(r#"<span class="article-item__date">x</span>"#),
            None
        );
    }
}
