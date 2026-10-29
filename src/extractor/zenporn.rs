//! Upstream `ZenPornIE` (`yt_dlp/extractor/zenporn.py`): the page's embed
//! host and video ID, the host's `api/json` metadata (optional), and the
//! `videofile.php` URL, which is base64-encoded after undoing the lookalike
//! characters the site substitutes (reverse engineered from the site's
//! JavaScript like upstream).
use super::{Extractor, dict_info, download_json_note, download_webpage, update_url_query};
use crate::{Error, Result, VideoInfo};
use base64::Engine as _;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct ZenPorn;

const NAME: &str = "ZenPorn";

static VALID_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://(?:www\.)?zenporn\.com/video/(?P<id>\d+)").unwrap());
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"https://(?P<ext_domain>[\w.-]+\.\w{3})/embed/(?P<extr_id>\d+)/").unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_gen_info_url`: the host's directory for the video's ID (the thousands
/// are truncated, like the site's script does).
fn info_url(ext_domain: &str, video_id: &str, lifetime: u32) -> Result<Url> {
    let id: u64 = video_id.parse().map_err(|_| {
        Error::UnsupportedHtml5("ZenPorn video IDs that are not numbers".to_owned())
    })?;
    let path = [1_000_000, 1_000, 1]
        .iter()
        .map(|divisor| (id / divisor * divisor).to_string())
        .collect::<Vec<_>>()
        .join("/");
    Ok(Url::parse(&format!(
        "https://{ext_domain}/api/json/video/{lifetime}/{path}.json"
    ))?)
}

/// `_decode_video_url`: lookalike characters are mapped back to base64 and
/// the text must decode to UTF-8 (`None` otherwise, like upstream).
fn decode_video_url(encoded: &str) -> Option<String> {
    let text: String = encoded
        .chars()
        .map(|character| match character {
            'А' => 'A',
            'В' => 'B',
            'С' => 'C',
            'Е' => 'E',
            'М' => 'M',
            '.' => '+',
            ',' => '/',
            '~' => '=',
            other => other,
        })
        .collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(text)
        .ok()?;
    String::from_utf8(bytes).ok()
}

/// `_download_json(url, video_id, fatal=False)`: HTTP errors are warnings.
fn download_json_optional(
    client: &Client,
    url: &Url,
    id: &str,
    note: &str,
) -> Result<Option<Value>> {
    match download_json_note(client, url, NAME, id, note) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// The single format of a top-level `url` result: the info dict itself
/// (`process_video_result`), marked as such.
fn url_only_format(url: &str, ext: &str) -> Result<crate::VideoFormat> {
    let mut fields = serde_json::Map::new();
    fields.insert("format_id".into(), "".into());
    fields.insert("url".into(), url.into());
    fields.insert("ext".into(), ext.into());
    let mut format: crate::VideoFormat = serde_json::from_value(Value::Object(fields))
        .map_err(|error| Error::Reported(format!("[{NAME}] {error}")))?;
    crate::info_order::mark_format(&mut format, "url-only");
    Ok(format)
}

impl Extractor for ZenPorn {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;

        let Some(embed) = EMBED.captures(&webpage) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to extract embed info; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let ext_domain = embed["ext_domain"].to_owned();
        let video_id = embed["extr_id"].to_owned();

        let info_json = download_json_optional(
            client,
            &info_url(&ext_domain, &video_id, 86_400)?,
            &video_id,
            "Downloading JSON metadata",
        )?;

        let video_file = update_url_query(
            &Url::parse(&format!("https://{ext_domain}/api/videofile.php"))?,
            &[("video_id", &video_id), ("lifetime", "8640000")],
        );
        let video_json = download_json_note(
            client,
            &video_file,
            NAME,
            &video_id,
            "Downloading video file JSON",
        )
        .map_err(|error| match error {
            Error::ExtractorRequest { source, .. } => Error::Reported(format!(
                "[{NAME}] {video_id}: Failed to download video file JSON: {}",
                crate::upstream_error_text(&source)
            )),
            error => error,
        })?;

        let encoded = video_json
            .get(0)
            .and_then(|entry| entry.get("video_url"))
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::UnsupportedHtml5("ZenPorn video files that are not expected".into())
            })?;
        let Some(decoded_url) = decode_video_url(encoded) else {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to decode the video url; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let extension = video_json
            .get(0)
            .and_then(|entry| entry.get("format"))
            .and_then(Value::as_str)
            .and_then(super::direct::determine_ext);

        let full_url = format!("https://{ext_domain}{decoded_url}");
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("display_id", display_id.clone().into()),
            ("ext", extension.clone().map_or(Value::Null, Value::from)),
            ("url", full_url.as_str().into()),
            ("age_limit", 18.into()),
        ];
        // `**traverse_obj(info_json, ('video', {...}))`.
        let video = info_json.as_ref().and_then(|info| info.get("video"));
        let text = |key: &str| {
            video
                .and_then(|video| video.get(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        let title = text("title");
        let description = text("description");
        let thumbnail = video
            .and_then(|video| video.get("thumb"))
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none);
        let upload_date =
            text("post_date").and_then(|date| crate::dates::unified_strdate(&date, true));
        let uploader = video
            .and_then(|video| video.get("user"))
            .and_then(|user| user.get("username"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        for (key, value) in [
            ("title", title),
            ("description", description),
            ("thumbnail", thumbnail),
            ("upload_date", upload_date),
            ("uploader", uploader),
        ] {
            if let Some(value) = value {
                fields.push((key, value.into()));
            }
        }

        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let format = url_only_format(&full_url, extension.as_deref().unwrap_or("unknown_video"))?;
        let shape = format!("untyped-dict;{}", keys.join(","));
        Ok(Some(crate::info_order::mark_info(
            dict_info(url, (NAME, NAME), fields, vec![format]),
            &shape,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://zenporn.com/video/15627016/desi-bhabi-ki-chudai"
        ));
        assert!(suitable("https://zenporn.com/video/15570701"));
        assert!(!suitable("https://zenporn.com/"));
    }

    #[test]
    fn info_url_and_decoding() {
        assert_eq!(
            info_url("txxx.com", "12791908", 86_400).unwrap().as_str(),
            "https://txxx.com/api/json/video/86400/12000000/12791000/12791908.json"
        );
        // "/get_file/1" in base64, with `.` `,` and lookalike `А` substitutions.
        assert_eq!(
            decode_video_url("L2dldF9maWxlLzE=").as_deref(),
            Some("/get_file/1")
        );
        assert_eq!(decode_video_url("!!!"), None);
    }
}
