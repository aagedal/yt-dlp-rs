//! Upstream `BioBioChileTVIE` (`yt_dlp/extractor/biobiochiletv.py`): BioBio
//! Chile TV article pages whose embedded Rudo player (`rudo.video/vod/...`)
//! supplies the video, as a `url_transparent` result that keeps the page's
//! title, thumbnail and uploader.
use super::{Extractor, download_webpage};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct BioBioChileTv;

const NAME: &str = "BioBioChileTV";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:tv|www)\.biobiochile\.cl/(?:notas|noticias)/(?:[^/]+/)+(?P<id>[^/]+)\.shtml",
    )
    .unwrap()
});
/// `_EMBED`-style search for the Rudo iframe (the `(?P=q1)` quote backreference
/// is written as two alternatives).
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+src=(?:"((?:https?:)?//rudo\.video/vod/[0-9a-zA-Z]+)"|'((?:https?:)?//rudo\.video/vod/[0-9a-zA-Z]+)')"#,
    )
    .unwrap()
});
static UPLOADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<a[^>]+href=["'](?:https?://(?:busca|www)\.biobiochile\.cl)?/(?:lista/)?(?:author|autor)[^>]+>(.+?)</a>"#,
    )
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

impl Extractor for BioBioChileTv {
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

        let embed = EMBED
            .captures(&webpage)
            .and_then(|found| found.get(1).or_else(|| found.get(2)))
            .map(|raw| raw.as_str().to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: No videos found; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let rudo_url = url.join(&embed)?;

        // `remove_end(_og_search_title(webpage), ' - BioBioChile TV')`.
        let title = crate::html::og_search_property(&webpage, "title").map(|title| {
            title
                .strip_suffix(" - BioBioChile TV")
                .unwrap_or(&title)
                .to_owned()
        });
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        let uploader = UPLOADER
            .captures(&webpage)
            .and_then(|found| found.get(1))
            .map(|name| crate::html::clean_html(name.as_str()))
            .filter(|name| !name.is_empty());

        // `url_transparent` to the Rudo embed with the page's fields.
        let mut fields = Map::new();
        fields.insert("title".into(), title.map_or(Value::Null, Value::from));
        fields.insert(
            "thumbnail".into(),
            thumbnail.map_or(Value::Null, Value::from),
        );
        fields.insert("uploader".into(), uploader.map_or(Value::Null, Value::from));
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(
            crate::FOLLOW_TRANSPARENT.into(),
            json!({"url": rudo_url.as_str(), "fields": Value::Object(fields)}),
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
            "http://tv.biobiochile.cl/notas/2015/10/21/sobre-camaras-y-camarillas-parlamentarias.shtml"
        ));
        assert!(suitable(
            "http://www.biobiochile.cl/noticias/bbtv/comentarios-bio-bio/2016/07/08/edecanes-del-congreso-figuras-decorativas-que-le-cuestan-muy-caro-a-los-chilenos.shtml"
        ));
        assert!(!suitable(
            "http://www.biobiochile.cl/portada/bbtv/index.html"
        ));
    }

    #[test]
    fn embed_accepts_both_quote_styles() {
        let double = r#"<iframe id="v" src="http://rudo.video/vod/abc123" width="7">"#;
        let single = "<iframe src='//rudo.video/vod/xyz'>";
        assert_eq!(
            EMBED
                .captures(double)
                .and_then(|f| f.get(1))
                .map(|m| m.as_str()),
            Some("http://rudo.video/vod/abc123")
        );
        assert_eq!(
            EMBED
                .captures(single)
                .and_then(|f| f.get(2))
                .map(|m| m.as_str()),
            Some("//rudo.video/vod/xyz")
        );
    }
}
