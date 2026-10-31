//! Upstream `DFBIE` (`yt_dlp/extractor/dfb.py`, `tv.dfb.de`): the player
//! XML's video entry and, for its stream access URL and the `iphone`
//! variant, the token manifest as optional HDS or HLS formats.
use super::{Extractor, download_xml, extract_m3u8_optional};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Dfb;

const NAME: &str = "tv.dfb.de";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://tv\.dfb\.de/video/(?P<display_id>[^/]+)/(?P<id>\d+)")
        .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// Upstream's wrapped `KeyError` (reported with the video ID).
fn key_error(video_id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{video_id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// Upstream's bare (unwrapped) `AttributeError` on a `None` element or text.
fn attribute_error(attribute: &str) -> Error {
    Error::Reported(format!("'NoneType' object has no attribute '{attribute}'"))
}

fn parse_xml<'a>(text: &'a str, display_id: &str) -> Result<roxmltree::Document<'a>> {
    roxmltree::Document::parse(text.trim_start_matches('\u{feff}'))
        .map_err(|error| Error::Reported(format!("[{NAME}] {display_id}: {error}")))
}

/// `element.find(tag)` (a direct child).
fn find<'a, 'i>(node: roxmltree::Node<'a, 'i>, tag: &str) -> Result<roxmltree::Node<'a, 'i>> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == tag)
        .ok_or_else(|| attribute_error("text"))
}

/// `element.text`: `None` when the element starts with a child or is empty.
fn text(node: roxmltree::Node<'_, '_>) -> Option<String> {
    let text: String = node
        .children()
        .take_while(|child| !child.is_element())
        .filter_map(|child| child.text())
        .collect();
    (!text.is_empty()).then_some(text)
}

impl Extractor for Dfb {
    fn key(&self) -> &'static str {
        "DFB"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["display_id"].to_owned();
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let player = download_xml(
            client,
            &Url::parse(&format!(
                "http://tv.dfb.de/server/hd_video.php?play={video_id}"
            ))?,
            NAME,
            &display_id,
        )?;
        let player = parse_xml(&player, &display_id)?;
        let video_info = find(player.root_element(), "video")?;
        let stream_access =
            text(find(video_info, "url")?).ok_or_else(|| attribute_error("strip"))?;
        let stream_access = super::proto_relative(stream_access.trim());
        let mut formats = Vec::new();
        for sa_url in [
            stream_access.clone(),
            format!("{stream_access}&area=&format=iphone"),
        ] {
            let info = download_xml(client, &Url::parse(&sa_url)?, NAME, &display_id)?;
            let info = parse_xml(&info, &display_id)?;
            let token = info
                .root_element()
                .children()
                .find(|child| child.is_element() && child.tag_name().name() == "token")
                .ok_or_else(|| attribute_error("attrib"))?;
            let attribute = |key: &str| {
                token
                    .attribute(key)
                    .ok_or_else(|| key_error(&video_id, key))
            };
            let manifest_url = format!("{}?hdnea={}", attribute("url")?, attribute("auth")?);
            if manifest_url.contains(".f4m") {
                let manifest = Url::parse(&format!("{manifest_url}&hdcore=3.2.0"))?;
                formats.extend(super::extract_f4m_optional(
                    client,
                    NAME,
                    &manifest,
                    &display_id,
                    Some("hds"),
                )?);
            } else if let Some(stream) = extract_m3u8_optional(
                client,
                &Url::parse(&manifest_url)?,
                NAME,
                &display_id,
                Some("hls"),
                false,
            ) {
                formats.extend(stream.formats);
            }
        }
        let title = text(find(video_info, "title")?);
        let upload_date = text(find(video_info, "time_date")?)
            .and_then(|date| crate::dates::unified_strdate(&date, true));
        let mut info = super::direct::build_at(url.clone(), url, String::new());
        info.direct = None;
        info.format_id = None;
        info.formats = formats;
        if info.formats.is_empty() {
            info.extra
                .insert(crate::info::NO_FORMATS.into(), Value::Bool(true));
        }
        info.id = video_id.clone();
        match title {
            Some(title) => info.title = title,
            None => {
                info.title = String::new();
                info.extra
                    .insert(crate::info::TITLE_MISSING.into(), Value::Bool(true));
            }
        }
        info.webpage_url = url.clone();
        info.extractor = NAME.into();
        info.extractor_key = "DFB".into();
        crate::metadata_parser::set_field(&mut info, "display_id", Value::from(display_id));
        info.metadata.thumbnail = Some(format!("http://tv.dfb.de/images/{video_id}_640x360.jpg"));
        crate::metadata_parser::set_field(
            &mut info,
            "upload_date",
            upload_date.map_or(Value::Null, Value::from),
        );
        Ok(Some(crate::info_order::mark_info(
            info,
            "dict;id,display_id,title,thumbnail,upload_date,formats",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://tv.dfb.de/video/u-19-em-stimmen-zum-spiel-gegen-russland/11633/"
        ));
        assert!(!suitable("http://tv.dfb.de/video/11633/"));
    }
}
