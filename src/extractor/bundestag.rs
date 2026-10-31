//! Upstream `BundestagIE` (`yt_dlp/extractor/bundestag.py`): Bundestag
//! Mediathek and `dbtg.tv` videos from the archive HLS playlist and the
//! share API's audio and download files, with the overlay page's title and
//! description.
use super::{Extractor, dict_format, dict_info, download_json_note};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Bundestag;

const NAME: &str = "Bundestag";

static VALID_URLS: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        crate::extractor::url_regex(r"^https?://dbtg\.tv/[cf]vid/(?P<id>\d+)").unwrap(),
        crate::extractor::url_regex(
            r"^https?://www\.bundestag\.de/mediathek/?\?(?:[^#]+&)?videoid=(?P<id>\d+)",
        )
        .unwrap(),
    ]
});
static SHARE_AUDIO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"/\d+_(?P<codec>\w+)_(?P<bitrate>\d+)kb_(?P<channels>\w+)_\w+_\d+\.(?P<ext>\w+)")
        .unwrap()
});
static SHARE_VIDEO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"/\d+_(?P<codec>\w+)_(?P<width>\w+)_(?P<height>\w+)_(?P<bitrate>\d+)kb_\w+_\w+_\d+\.(?P<ext>\w+)",
    )
    .unwrap()
});
static SPAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<span[^>]*>[^<]+</span>").unwrap());

const OVERLAY_URL: &str = "https://www.bundestag.de/mediathekoverlay";
const SHARE_URL: &str =
    "https://webtv.bundestag.de/player/macros/_x_s-144277506/shareData.json?contentId=";

/// The `_VALID_URL` list's match.
fn video_id(url: &str) -> Option<String> {
    VALID_URLS
        .iter()
        .find_map(|regex| regex.captures(url))
        .map(|found| found["id"].to_owned())
}

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    video_id(url).is_some()
}

fn instance_url(video_id: &str) -> String {
    format!(
        "https://cldf-wzw-od.r53.cdn.tv1.eu/13014bundestagod/_definst_/13014bundestag/ondemand/3777parlamentsfernsehen/archiv/app144277506/145293313/{video_id}/{video_id}_playlist.smil/playlist.m3u8"
    )
}

/// `int_or_none` of a regex group.
fn group_int(found: &regex::Captures<'_>, name: &str) -> Option<i64> {
    found
        .name(name)
        .and_then(|value| value.as_str().parse().ok())
}

/// `_bt_extract_share_formats`.
fn share_formats(client: &Client, video_id: &str) -> Result<Vec<crate::VideoFormat>> {
    let api = Url::parse(&format!("{SHARE_URL}{video_id}"))?;
    let share_data = download_json_note(
        client,
        &api,
        NAME,
        video_id,
        "Downloading share format JSON",
    )?;
    if share_data.pointer("/status/code").and_then(Value::as_i64) != Some(1) {
        let message = share_data
            .pointer("/status/message")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .map_or_else(
                || "Unknown Share API Error".to_owned(),
                |message| format!("Share API response: {message}"),
            );
        crate::screen::warning(&format!(
            "[{NAME}] {message}; {}",
            crate::BUG_REPORTS_MESSAGE
        ));
        return Ok(Vec::new());
    }
    let mut formats = Vec::new();
    let Some(share_data) = share_data.as_object() else {
        return Ok(formats);
    };
    for (name, value) in share_data {
        let Some(url) = value.as_str().and_then(super::json_ld::url_or_none) else {
            continue;
        };
        let mut fields = vec![
            ("format_id", Value::from(name.clone())),
            ("url", Value::from(url.clone())),
        ];
        if name.starts_with("audio") {
            fields.push(("vcodec", "none".into()));
            if let Some(found) = SHARE_AUDIO.captures(&url) {
                fields.push(("acodec", found["codec"].into()));
                if let Some(channels) = match &found["channels"] {
                    "mono" => Some(1),
                    "stereo" => Some(2),
                    _ => None,
                } {
                    fields.push(("audio_channels", Value::from(channels)));
                }
                if let Some(abr) = group_int(&found, "bitrate") {
                    fields.push(("abr", abr.into()));
                }
                fields.push(("ext", found["ext"].into()));
            }
        } else if name.starts_with("download") {
            if let Some(found) = SHARE_VIDEO.captures(&url) {
                fields.push(("vcodec", found["codec"].into()));
                for (key, group) in [("tbr", "bitrate"), ("width", "width"), ("height", "height")] {
                    if let Some(value) = group_int(&found, group) {
                        fields.push((key, value.into()));
                    }
                }
                fields.push(("ext", found["ext"].into()));
            }
        } else {
            continue;
        }
        let mut format = dict_format(fields, NAME, video_id)?;
        crate::info_order::mark_int_rates(&mut format);
        formats.push(format);
    }
    Ok(formats)
}

impl Extractor for Bundestag {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(video_id) = video_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let mut formats = Vec::new();
        let instance = Url::parse(&instance_url(&video_id))?;
        match super::extract_m3u8_fatal_master(
            client,
            &instance,
            NAME,
            &video_id,
            Some("instance"),
            false,
        ) {
            Ok(stream) => formats.extend(stream.formats),
            Err(Error::Reported(message)) if message.contains("HTTP Error 404") => {
                return Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Could not find video id"
                )));
            }
            // `str(error)` before `extract` adds the extractor name.
            Err(Error::Reported(message)) => {
                let message = message
                    .strip_prefix(&format!("[{NAME}] "))
                    .unwrap_or(&message)
                    .to_owned();
                crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: Error extracting hls formats: {message}"
                ));
            }
            Err(error) => return Err(error),
        }
        formats.extend(share_formats(client, &video_id)?);
        if formats.is_empty() {
            // `raise_no_formats(msg, video_id=...)`: not expected, so raised
            // even with `--ignore-no-formats-error`.
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: Could not find suitable formats; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        let mut fields = vec![
            ("id", Value::from(video_id.clone())),
            ("formats", Value::Null),
        ];
        let overlay = super::update_url_query(
            &Url::parse(OVERLAY_URL)?,
            &[("videoid", video_id.as_str()), ("view", "main")],
        );
        // `_download_webpage(..., fatal=False)`.
        match super::download_webpage_note(
            client,
            &overlay,
            NAME,
            &video_id,
            "Downloading metadata overlay",
        ) {
            Ok(page) => {
                if let Ok((title, _)) =
                    crate::html_elements::element_text_and_html_by_tag("h3", &page)
                {
                    fields.push((
                        "title",
                        crate::html::clean_html(&SPAN.replace_all(&title, "")).into(),
                    ));
                }
                if let Ok((description, _)) =
                    crate::html_elements::element_text_and_html_by_tag("p", &page)
                {
                    fields.push(("description", crate::html::clean_html(&description).into()));
                }
            }
            Err(Error::ExtractorRequest { source, .. }) => crate::screen::warning(&format!(
                "[{NAME}] {video_id}: Unable to download webpage: {}",
                crate::upstream_error_text(&source)
            )),
            Err(error) => return Err(error),
        }
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert_eq!(
            video_id("https://dbtg.tv/cvid/7605304").as_deref(),
            Some("7605304")
        );
        assert_eq!(
            video_id("https://www.bundestag.de/mediathek?videoid=7602120&url=x").as_deref(),
            Some("7602120")
        );
        assert_eq!(
            video_id("https://www.bundestag.de/mediathek/?a=1&videoid=9").as_deref(),
            Some("9")
        );
        assert!(!suitable("https://www.bundestag.de/mediathek"));
    }
}
