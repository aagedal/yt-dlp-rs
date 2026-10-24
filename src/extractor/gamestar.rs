//! Upstream `GameStarIE` (`yt_dlp/extractor/gamestar.py`): the page's first
//! `VideoObject` JSON-LD script and the portal's video URL.
use super::{Extractor, download_webpage};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct GameStar;

const NAME: &str = "GameStar";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?game(?P<site>pro|star)\.de/videos/.*,(?P<id>[0-9]+)\.html")
        .unwrap()
});
// `type=(["\'])application/ld\+json\1` without the backreference.
static JSON_LD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<script[^>]+type=(?:"application/ld\+json"|'application/ld\+json')[^>]*>(?P<json_ld>[^<]+VideoObject[^<]+)</script>"#,
    )
    .unwrap()
});
static COMMENT_COUNT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<span>Kommentare</span>\s*<span[^>]+class=["']count[^>]+>\s*\(\s*([0-9]+)"#)
        .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for GameStar {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let site = found["site"].to_owned();
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &id)?;
        let script = JSON_LD
            .captures(&webpage)
            .map(|found| found["json_ld"].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract JSON-LD; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let json_ld: Value = serde_json::from_str(&script).map_err(|_| {
            Error::UnsupportedHtml5(
                "GameStar JSON-LD that does not parse is not supported natively".into(),
            )
        })?;
        let fields = super::json_ld::json_ld(&json_ld);
        // `info_dict['title']`.
        let Some(title) = fields.get("title").cloned() else {
            return Err(Error::Reported(format!(
                "{id}: An extractor error has occurred. (caused by KeyError('title')); {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        // `remove_end(title, f' - Game{site.title()}')`.
        let suffix = format!(" - Game{}{}", site[..1].to_uppercase(), &site[1..]);
        let title = match title.as_str() {
            Some(text) => text.strip_suffix(&suffix).unwrap_or(text).to_owned(),
            None => {
                return Err(Error::UnsupportedHtml5(
                    "GameStar titles that are not strings are not supported natively".into(),
                ));
            }
        };
        let view_count = super::streamable::int_or_none(json_ld.get("interactionCount"));
        // `_html_search_regex(..., fatal=False)`.
        let comment_count = match COMMENT_COUNT.captures(&webpage) {
            Some(found) => crate::html::clean_html(&found[1])
                .trim()
                .parse::<i64>()
                .map_or(Value::Null, Value::from),
            None => {
                super::search_warning(NAME, "comment count");
                Value::Null
            }
        };
        let media = Url::parse(&format!(
            "http://gamestar.de/_misc/videos/portal/getVideoUrl.cfm?premium=0&videoId={id}"
        ))?;
        let mut info = super::direct::with_format(
            super::direct::build_at(media.clone(), &media, "mp4".into()),
            "",
        );
        for format in &mut info.formats {
            format.ext = "mp4".into();
            format.format_id = String::new();
        }
        info.format_id = None;
        info.id = id;
        info.title = title;
        info.webpage_url = url.clone();
        info.direct = None;
        info.extractor = NAME.into();
        info.extractor_key = NAME.into();
        let own = ["id", "url", "ext", "view_count", "comment_count"];
        let mut keys: Vec<String> = Vec::new();
        for (key, value) in fields {
            if !own.contains(&key.as_str()) && key != "title" {
                crate::metadata_parser::set_field(&mut info, &key, value);
            }
            keys.push(key);
        }
        for key in own {
            if !keys.iter().any(|existing| existing == key) {
                keys.push(key.to_owned());
            }
        }
        crate::metadata_parser::set_field(&mut info, "view_count", view_count);
        crate::metadata_parser::set_field(&mut info, "comment_count", comment_count);
        Ok(Some(crate::info_order::mark_info(
            info,
            &format!("untyped-dict;{}", keys.join(",")),
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.gamestar.de/videos/trailer,3/hobbit-3-die-schlacht-der-fuenf-heere,76110.html"
        ));
        assert!(suitable(
            "http://www.gamepro.de/videos/top-10-indie-spiele-fuer-nintendo-switch-video-tolle-nindies-games-zum-download,95316.html"
        ));
        assert!(!suitable("http://www.gamestar.de/videos/76110.html"));
    }
}
