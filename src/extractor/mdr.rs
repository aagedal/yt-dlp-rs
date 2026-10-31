//! Upstream `MDRIE` (`yt_dlp/extractor/mdr.py`): MDR.DE pages' `-avCustom.xml`
//! player document: its assets' download, progressive and streaming URLs as
//! HLS (`HLS`, quality 1), HDS (`HDS`, quality 1) or progressive formats,
//! with the title, broadcast description, timestamp, duration and rights.
use super::{Extractor, dict_format, dict_info, download_webpage, download_xml};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Mdr;

const NAME: &str = "MDR";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?mdr\.de/(?:.*)/[a-z-]+-?(?P<id>\d+)(?:_.+?)?\.html",
    )
    .unwrap()
});
static DATA_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?:dataURL|playerXml["']?)\s*:\s*(?:"(?P<dq>[^\n]+?-avCustom\.xml)"|'(?P<sq>[^\n]+?-avCustom\.xml)')"#,
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn regex_error(id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `element.find(path)` of a `./a/b` path of direct children.
fn find<'a, 'i>(node: roxmltree::Node<'a, 'i>, path: &str) -> Option<roxmltree::Node<'a, 'i>> {
    path.trim_start_matches("./")
        .split('/')
        .try_fold(node, |node, tag| {
            node.children()
                .find(|child| child.is_element() && child.tag_name().name() == tag)
        })
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

/// `xpath_text(node, paths)` (not fatal): the first found element's text.
fn xpath_text(node: roxmltree::Node<'_, '_>, paths: &[&str]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| find(node, path))
        .and_then(text)
}

/// `int_or_none(text, scale)`: Python `int(text) // scale`.
fn int_or_none(text: Option<String>, scale: i64) -> Option<i64> {
    let value: i64 = text?.trim().replace('_', "").parse().ok()?;
    Some(value.div_euclid(scale))
}

impl Extractor for Mdr {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_GEO_COUNTRIES = ['DE']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["DE"])
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
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
        let data_url = DATA_URL
            .captures(&webpage)
            .and_then(|found| found.name("dq").or_else(|| found.name("sq")))
            .map(|data_url| data_url.as_str().replace(r"\/", "/"))
            .ok_or_else(|| regex_error(&video_id, "data url"))?;
        let doc_url = url.join(&data_url)?;
        let xml = download_xml(client, &doc_url, NAME, &video_id)?;
        let document = roxmltree::Document::parse(xml.trim_start_matches('\u{feff}'))
            .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
        let doc = document.root_element();
        let title = match ["./title", "./broadcast/broadcastName"]
            .iter()
            .find_map(|path| find(doc, path))
        {
            None => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Could not find XML element title; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            Some(node) => text(node).ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Could not find XML element's text title; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?,
        };
        let type_ = xpath_text(doc, &["./type"]);
        let mut formats = Vec::new();
        let mut processed: Vec<String> = Vec::new();
        let assets = find(doc, "./assets").into_iter().flat_map(|assets| {
            assets
                .children()
                .filter(|child| child.is_element() && child.tag_name().name() == "asset")
        });
        for asset in assets {
            for source in [
                "download",
                "progressiveDownload",
                "dynamicHttpStreamingRedirector",
                "adaptiveHttpStreamingRedirector",
            ] {
                let Some(url_el) = find(asset, &format!("./{source}Url")) else {
                    continue;
                };
                let Some(video_url) = text(url_el)
                    .as_deref()
                    .and_then(super::json_ld::url_or_none)
                else {
                    continue;
                };
                if processed.contains(&video_url) {
                    continue;
                }
                processed.push(video_url.clone());
                let ext = super::direct::determine_ext(&video_url);
                if ext.as_deref() == Some("m3u8") {
                    if let Some(stream) = super::extract_m3u8_master_optional(
                        client,
                        &Url::parse(&video_url)?,
                        NAME,
                        &video_id,
                        Some("HLS"),
                        false,
                        false,
                    ) {
                        for mut format in stream.formats {
                            format.quality = Some(1.0);
                            crate::info_order::mark_int_quality(&mut format);
                            formats.push(format);
                        }
                    }
                } else if ext.as_deref() == Some("f4m") {
                    let manifest =
                        Url::parse(&format!("{video_url}?hdcore=3.7.0&plugin=aasp-3.7.0.39.44"))?;
                    for mut format in super::extract_f4m_optional(
                        client,
                        NAME,
                        &manifest,
                        &video_id,
                        Some("HDS"),
                    )? {
                        format.quality = Some(1.0);
                        crate::info_order::mark_int_quality(&mut format);
                        formats.push(format);
                    }
                } else {
                    let media_type =
                        xpath_text(asset, &["./mediaType"]).unwrap_or_else(|| "MP4".into());
                    let vbr = int_or_none(xpath_text(asset, &["./bitrateVideo"]), 1000);
                    let abr = int_or_none(xpath_text(asset, &["./bitrateAudio"]), 1000);
                    let filesize = int_or_none(xpath_text(asset, &["./fileSize"]), 1);
                    // `join_nonempty(media_type, vbr or abr)`.
                    let rate = vbr
                        .filter(|rate| *rate != 0)
                        .or(abr)
                        .filter(|rate| *rate != 0);
                    let format_id = [
                        Some(media_type).filter(|kind| !kind.is_empty()),
                        rate.map(|rate| rate.to_string()),
                    ]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
                    .join("-");
                    let opt = |value: Option<i64>| value.map_or(Value::Null, Value::from);
                    let mut fields = vec![
                        ("url", Value::from(video_url.clone())),
                        ("format_id", format_id.into()),
                        ("filesize", opt(filesize)),
                        ("abr", opt(abr)),
                        ("vbr", opt(vbr)),
                    ];
                    if vbr.is_some_and(|vbr| vbr != 0) {
                        fields.push((
                            "width",
                            opt(int_or_none(xpath_text(asset, &["./frameWidth"]), 1)),
                        ));
                        fields.push((
                            "height",
                            opt(int_or_none(xpath_text(asset, &["./frameHeight"]), 1)),
                        ));
                    }
                    if type_.as_deref() == Some("audio") {
                        fields.push(("vcodec", "none".into()));
                    }
                    let mut format = dict_format(fields, NAME, &video_id)?;
                    crate::info_order::mark_int_rates(&mut format);
                    formats.push(format);
                }
            }
        }
        let opt_text = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let description = xpath_text(doc, &["./broadcast/broadcastDescription"]);
        let timestamp = xpath_text(
            doc,
            &[
                "./broadcast/broadcastDate",
                "./broadcast/broadcastStartDate",
                "./broadcast/broadcastEndDate",
            ],
        )
        .and_then(|date| crate::dates::parse_iso8601(&date));
        let duration = xpath_text(doc, &["./duration"])
            .and_then(|duration| crate::parse_duration(&duration))
            .map_or(Value::Null, |duration| serde_json::json!(duration));
        let uploader = xpath_text(doc, &["./rights"]);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("description", opt_text(description)),
                ("timestamp", timestamp.map_or(Value::Null, Value::from)),
                ("duration", duration),
                ("uploader", opt_text(uploader)),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://www.mdr.de/fakt/video189002.html"));
        assert!(suitable(
            "http://www.mdr.de/kultur/audio1312272_zc-15948bad_zs-86171fdd.html"
        ));
        assert!(suitable(
            "http://www.mdr.de/mediathek/mdr-videos/a/video-1334.html"
        ));
        assert!(!suitable("http://www.mdr.de/fakt/index.html"));
    }

    #[test]
    fn scaled_ints_floor_like_python() {
        assert_eq!(int_or_none(Some("128000".into()), 1000), Some(128));
        assert_eq!(int_or_none(Some("1500".into()), 1000), Some(1));
        assert_eq!(int_or_none(Some("x".into()), 1000), None);
    }
}
