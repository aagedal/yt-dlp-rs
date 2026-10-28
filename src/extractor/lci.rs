//! Upstream `LCIIE` (`yt_dlp/extractor/lci.py`): LCI and TF1 Info videos as
//! `url` results of the first WAT ID in the page's Next.js `tms.videos`.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Lci;

const NAME: &str = "LCI";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?(?:lci|tf1info)\.fr/(?:[^/?#]+/)+[\w-]+-(?P<id>\d+)\.html")
        .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for Lci {
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
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let next_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        // `(..., 'videos', {dict.keys}, ..., {int_or_none}, any)`.
        let wat_id = match next_data.pointer("/props/pageProps/page/tms/videos") {
            Some(Value::Object(videos)) => {
                videos.keys().find_map(|key| key.trim().parse::<i64>().ok())
            }
            _ => None,
        };
        let Some(wat_id) = wat_id else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Could not find wat_id; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow
            .extra
            .insert(crate::FOLLOW_URL.into(), format!("wat:{wat_id}").into());
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.tf1info.fr/replay-lci/videos/video-24h-pujadas-du-vendredi-24-mai-6708-2300831.html"
        ));
        assert!(suitable("https://www.lci.fr/politique/x-2125611.html"));
        assert!(!suitable("https://www.lci.fr/politique/x.html"));
    }
}
