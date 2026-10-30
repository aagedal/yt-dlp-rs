//! Upstream `MediaStreamIE` (`yt_dlp/extractor/mediastream.py`): an
//! `mdstrm.com` embed's `window.MDSTRM.OPTIONS` sources (HLS with the
//! session IDs from the page, DASH, and direct files). The `WinSports` page
//! extractor and the webpage hand-off to embeds are not ported.
use super::{
    Extractor, dict_format, dict_info, download_webpage, extract_m3u8_fatal_master,
    extract_mpd_fatal,
};
use crate::{Error, ExtractResult, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct MediaStream;

const NAME: &str = "MediaStream";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://mdstrm\.com/(?:embed|live-stream)/(?P<id>\w+)").unwrap()
});
static OPTIONS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.MDSTRM\.OPTIONS\s*=").unwrap());

const GEO_MESSAGES: [&str; 4] = [
    "Debido a tu ubicación no puedes ver el contenido",
    "You are not allowed to watch this video: Geo Fencing Restriction",
    "Este contenido no está disponible en tu zona geográfica.",
    "El contenido sólo está disponible dentro de",
];

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("MediaStream {what} are not supported natively"))
}

/// `window.<name> = '...';` on the page (the session IDs of the HLS URL).
fn page_variable(webpage: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(r#"window\.{name}\s*=\s*["']([^"']+)["'];"#)).ok()?;
    pattern.captures(webpage).map(|found| found[1].to_owned())
}

impl Extractor for MediaStream {
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
        if GEO_MESSAGES.iter().any(|message| webpage.contains(message)) {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: This video is not available from your location due to geo restriction"
            )));
        }

        let Some(options) = super::search_json(&OPTIONS, &webpage, true).flatten() else {
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract metadata; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let sources = options
            .get("src")
            .and_then(Value::as_object)
            .ok_or_else(|| unsupported("players without a source map"))?;

        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        for (format_name, source) in sources {
            let source_url = source
                .as_str()
                .ok_or_else(|| unsupported("source URLs that are not strings"))?;
            match format_name.as_str() {
                "hls" => {
                    let mut params: Vec<(&str, String)> = vec![("at", "web-app".into())];
                    for (name, key) in [
                        ("MDSTRMUID", "uid"),
                        ("MDSTRMSID", "sid"),
                        ("MDSTRMPID", "pid"),
                        ("VERSION", "av"),
                    ] {
                        if let Some(value) = page_variable(&webpage, name) {
                            params.push((key, value));
                        }
                    }
                    let borrowed: Vec<(&str, &str)> = params
                        .iter()
                        .map(|(key, value)| (*key, value.as_str()))
                        .collect();
                    let manifest = super::update_url_query(&Url::parse(source_url)?, &borrowed);
                    let stream =
                        extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, true)?;
                    formats.extend(stream.formats);
                    super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                }
                "mpd" => {
                    let manifest = Url::parse(source_url)?;
                    let stream = extract_mpd_fatal(client, &manifest, NAME, &video_id, true)?;
                    formats.extend(stream.formats);
                    super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                }
                _ => formats.push(dict_format(
                    vec![("url", source_url.into())],
                    NAME,
                    &video_id,
                )?),
            }
        }

        let title = crate::html::og_search_property(&webpage, "title").map_or_else(
            || options.get("title").cloned().unwrap_or(Value::Null),
            Value::from,
        );
        let description = crate::html::og_search_property(&webpage, "description");
        let thumbnail = crate::html::og_search_property(&webpage, "image");
        let is_live = options.get("type").and_then(Value::as_str) == Some("live");
        let fields = vec![
            ("id", Value::from(video_id.as_str())),
            ("title", title),
            ("description", description.map_or(Value::Null, Value::from)),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("is_live", Value::Bool(is_live)),
            ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(ExtractResult::Video(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://mdstrm.com/embed/6318e3f1d1d316083ae48831"
        ));
        assert!(suitable(
            "https://mdstrm.com/live-stream/6318e3f1d1d316083ae48831"
        ));
        assert!(!suitable("https://mdstrm.com/"));
        assert!(!suitable(
            "https://www.multimedios.com/video/costa-rica-tv-en-vivo/v2616"
        ));
    }
}
