//! Upstream `WDRIE` and `WDRElefantIE` (`yt_dlp/extractor/wdr.py`): WDR media
//! metadata (`deviceids-medp.wdr.de/ondemand/.../ID.js`) with its HLS
//! renditions, and the Elefantenseite fragment pages, whose table of contents
//! and XML metadata lead to the WDR media as a followed `url` result. The
//! `WDRPageIE` pages, direct (non-HLS) renditions, SMIL, captions and the
//! `wdr:` prefix are not ported; those fail with explicit errors.
use super::{
    Extractor, dict_info, direct, download_json, download_json_transformed, download_xml,
    extract_f4m_optional, extract_m3u8_fatal_master, update_url_query, weibo::strip_jsonp,
};
use crate::{Error, ExtractResult, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Wdr;

pub struct WdrElefant;

const NAME: &str = "WDR";
const ELEFANT_NAME: &str = "WDRElefant";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:deviceids-medp\.wdr\.de/ondemand/\d+/|kinder\.wdr\.de/[^#?]+-)(?P<id>\d+)\.(?:js|assetjsonp)",
    )
    .unwrap()
});
// `kinder\.wdr\.de/(?!mediathek/)`.
static KINDER_MEDIATHEK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://kinder\.wdr\.de/mediathek/").unwrap());
static ELEFANT_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)wdrmaus\.de/elefantenseite/#(?P<id>.+)")
        .unwrap()
});

const TABLE_OF_CONTENTS: &str = "https://www.wdrmaus.de/elefantenseite/data/tableOfContentsJS.php5";
const ELEFANT_BASE: &str = "https://www.wdrmaus.de/elefantenseite/";

/// `WDRIE._VALID_URL`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url) && !KINDER_MEDIATHEK.is_match(url)
}

/// `WDRElefantIE._VALID_URL`.
pub fn elefant_suitable(url: &str) -> bool {
    ELEFANT_VALID_URL.is_match(url)
}

/// Upstream `'//' + host` (a protocol-relative URL) requested as `http:`.
fn absolute(url: &str) -> String {
    if url.starts_with("//") {
        format!("http:{url}")
    } else {
        url.to_owned()
    }
}

fn truthy_str(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
}

/// `WDRIE._real_extract` for the `ID.js` metadata at `url`.
fn extract_media(client: &Client, url: &Url, video_id: &str) -> Result<VideoInfo> {
    let fail = |message: String| Error::Reported(format!("[{NAME}] {video_id}: {message}"));
    let metadata = download_json_transformed(
        client,
        url,
        NAME,
        video_id,
        "Downloading JSON metadata",
        &|code| Ok(strip_jsonp(code)),
    )?;
    let is_live = metadata.get("mediaType").and_then(Value::as_str) == Some("live");
    let tracker = metadata
        .get("trackerData")
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            fail("An extractor error has occurred. (caused by KeyError('trackerData'))".into())
        })?;
    let title = tracker
        .get("trackerClipTitle")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            fail("An extractor error has occurred. (caused by KeyError('trackerClipTitle'))".into())
        })?
        .to_owned();
    let media_resource = metadata
        .get("mediaResource")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            fail("An extractor error has occurred. (caused by KeyError('mediaResource'))".into())
        })?;

    let mut formats: Vec<VideoFormat> = Vec::new();
    let unsupported = |what: &str| {
        Error::Reported(format!(
            "[{NAME}] {video_id}: {what} is not supported natively yet"
        ))
    };
    for (kind, media) in media_resource {
        if kind == "captionsHash" {
            if media.as_object().is_some_and(|hash| !hash.is_empty()) {
                return Err(unsupported("WDR captions"));
            }
            continue;
        }
        if kind != "dflt" && kind != "alt" {
            continue;
        }
        let Some(media) = media.as_object() else {
            continue;
        };
        for (tag_name, medium_url) in media {
            if tag_name != "videoURL" && tag_name != "audioURL" {
                continue;
            }
            let Some(medium_url) = medium_url.as_str() else {
                continue;
            };
            let medium = absolute(medium_url);
            match direct::determine_ext(&medium).as_deref() {
                Some("m3u8") => {
                    let manifest = Url::parse(&medium)?;
                    let stream = extract_m3u8_fatal_master(
                        client,
                        &manifest,
                        NAME,
                        video_id,
                        Some("hls"),
                        false,
                    )?;
                    formats.extend(stream.formats);
                }
                Some("f4m") => {
                    let manifest = update_url_query(
                        &Url::parse(&medium)?,
                        &[("hdcore", "3.2.0"), ("plugin", "aasp-3.2.0.77.18")],
                    );
                    formats.extend(extract_f4m_optional(
                        client,
                        NAME,
                        &manifest,
                        video_id,
                        Some("hds"),
                    )?);
                }
                _ => return Err(unsupported("WDR direct or SMIL media")),
            }
        }
    }
    if truthy_str(media_resource.get("captionURL")).is_some() {
        return Err(unsupported("WDR captions"));
    }

    let optional = |value: Option<&str>| value.map_or(Value::Null, Value::from);
    let id = tracker
        .get("trackerClipId")
        .and_then(Value::as_str)
        .unwrap_or(video_id);
    let upload_date = tracker
        .get("trackerClipAirTime")
        .and_then(Value::as_str)
        .and_then(|date| crate::dates::unified_strdate(date, true));
    let fields = vec![
        ("id", id.into()),
        ("title", title.into()),
        (
            "alt_title",
            optional(truthy_str(tracker.get("trackerClipSubcategory"))),
        ),
        ("formats", Value::Null),
        ("subtitles", Value::Null),
        ("upload_date", optional(upload_date.as_deref())),
        ("is_live", json!(is_live)),
    ];
    Ok(dict_info(url, (NAME, NAME), fields, formats))
}

impl Extractor for Wdr {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        if !suitable(url.as_str()) {
            return Ok(None);
        }
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        extract_media(client, url, &video_id).map(Some)
    }
}

impl Extractor for WdrElefant {
    fn key(&self) -> &'static str {
        ELEFANT_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = ELEFANT_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(ELEFANT_NAME);
        crate::screen::screen(&format!(
            "[{ELEFANT_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // The table of contents is always at this address.
        let table = download_json(
            client,
            &Url::parse(TABLE_OF_CONTENTS)?,
            ELEFANT_NAME,
            &display_id,
        )?;
        let Some(entry) = table.get(display_id.as_str()) else {
            return Err(Error::Reported(format!(
                "[{ELEFANT_NAME}] {display_id}: No entry in site's table of contents for this URL. Is the fragment part of the URL (after the #) correct?"
            )));
        };
        let xml_path = entry
            .get("xmlPath")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{ELEFANT_NAME}] {display_id}: An extractor error has occurred. (caused by KeyError('xmlPath'))"
                ))
            })?;
        let xml = download_xml(
            client,
            &Url::parse(&format!("{ELEFANT_BASE}{xml_path}"))?,
            ELEFANT_NAME,
            &display_id,
        )?;
        let document = roxmltree::Document::parse(&xml)
            .map_err(|error| Error::Reported(format!("[{ELEFANT_NAME}] {display_id}: {error}")))?;
        let zmdb_url = document
            .root_element()
            .children()
            .filter(|node| node.has_tag_name("movie"))
            .flat_map(|movie| movie.children())
            .find(|node| node.has_tag_name("zmdb_url"))
            .and_then(|node| node.text())
            .map(str::to_owned);
        let Some(zmdb_url) = zmdb_url else {
            return Err(Error::Reported(format!(
                "[{ELEFANT_NAME}] {display_id}: {display_id} is not a video"
            )));
        };
        // `url_result(zmdb_url, ie=WDRIE.ie_key())`, followed when processed.
        let mut follow = direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow
            .extra
            .insert(crate::FOLLOW_URL.into(), zmdb_url.into());
        Ok(Some(ExtractResult::Video(follow)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://deviceids-medp.wdr.de/ondemand/119/1198320.js"
        ));
        assert!(suitable(
            "https://kinder.wdr.de/tv/die-sendung-mit-dem-elefanten/video-wippe-100.assetjsonp"
        ));
        assert!(!suitable(
            "https://kinder.wdr.de/mediathek/video-1.assetjsonp"
        ));
        assert!(elefant_suitable(
            "http://www.wdrmaus.de/elefantenseite/#elefantenkino_wippe"
        ));
        assert!(!elefant_suitable("https://www.wdrmaus.de/elefantenseite/"));
    }
}
