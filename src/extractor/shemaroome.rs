//! Upstream `ShemarooMeIE` (`yt_dlp/extractor/shemaroome.py`): movies and
//! shows whose hidden form fields name the catalogue item. The
//! `user_all_lists` answer carries the HLS URL encrypted with AES-128-CBC
//! (zero IV); items it does not release are refused as premium. The master
//! playlist is read with the `stream_key` header, which every format keeps.
use super::{Extractor, dict_info, download_webpage};
use crate::subtitles::SubtitleFormat;
use crate::{Error, Result, Subtitles, VideoInfo};
use base64::Engine as _;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct ShemarooMe;

const NAME: &str = "ShemarooMe";
const USER_LISTS: &str = "https://www.shemaroome.com/users/user_all_lists";
const PREMIUM: &str = "Premium videos cannot be downloaded yet.";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?shemaroome\.com/(?:movies|shows)/(?P<id>[^?#]+)").unwrap()
});
static DESCRIPTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)>Synopsis(</.+?)</").unwrap());
static RELEASE_DATES: LazyLock<[Regex; 2]> = LazyLock::new(|| {
    [
        Regex::new(r#"itemprop="uploadDate">\s*([\d-]+)"#).unwrap(),
        Regex::new(r#"id="release_date" value="([\d-]+)"#).unwrap(),
    ]
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unable(id: &str, what: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: Unable to extract {what}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_search_regex(r'id="NAME" value="([^"]+)', ...)`.
fn hidden_value(webpage: &str, name: &str) -> Option<String> {
    let pattern = Regex::new(&format!(r#"id="{}" value="([^"]+)"#, regex::escape(name))).ok()?;
    pattern.captures(webpage).map(|found| found[1].to_owned())
}

/// A required hidden field (`what` is the name upstream reports).
fn required(webpage: &str, id: &str, name: &str, what: &str) -> Result<String> {
    hidden_value(webpage, name).ok_or_else(|| unable(id, what))
}

/// `base64.b64decode(data[key])`.
fn decoded_field(id: &str, data: &Value, key: &str) -> Result<Vec<u8>> {
    let text = data
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| super::clyp::key_error(id, key))?;
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .map_err(|_| Error::Reported(format!("[{NAME}] {id}: Invalid base64 in {key}")))
}

/// `unpad_pkcs7(aes_cbc_decrypt_bytes(data, key, 16 zero bytes)).decode('ascii')`.
fn decrypt_manifest_url(id: &str, encrypted: &[u8], key: &[u8]) -> Result<String> {
    let key: [u8; 16] = key
        .try_into()
        .map_err(|_| Error::Reported(format!("[{NAME}] {id}: Unsupported AES key length")))?;
    let mut decryptor = crate::crypto::Decryptor::new(&key, &[0; 16]);
    let mut plain = decryptor.update(encrypted);
    plain.extend(decryptor.finish()?);
    let text = String::from_utf8(plain)
        .ok()
        .filter(|text| text.is_ascii())
        .ok_or_else(|| Error::Reported(format!("[{NAME}] {id}: Decrypted URL is not ASCII")))?;
    Ok(text)
}

impl Extractor for ShemarooMe {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].replace('/', "_");
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        let title = required(&webpage, &video_id, "ma_title", "title")?;
        let thumbnail = crate::html::og_search_property(&webpage, "image").map_or_else(
            || {
                super::search_warning(NAME, "thumbnail");
                Value::Null
            },
            Value::from,
        );
        let content_def = required(&webpage, &video_id, "content_definition", "content_def")?;
        let catalog_id = required(&webpage, &video_id, "catalog_id", "catalog_id")?;
        let item_category = required(&webpage, &video_id, "item_category", "item_category")?;
        let content_id = required(&webpage, &video_id, "content_id", "content_id")?;

        let body = format!(
            "catalog_id={catalog_id}&content_id={content_id}&category={item_category}&content_def={content_def}"
        );
        let api = Url::parse(USER_LISTS)?;
        let data = super::post_json_note_headers(
            client,
            &api,
            NAME,
            &video_id,
            "Downloading JSON metadata",
            &[("Content-Type", "application/x-www-form-urlencoded")],
            body,
        )?;
        if !data.get("status").is_some_and(super::nuxt::truthy) {
            return Err(Error::Reported(format!("[{NAME}] {video_id}: {PREMIUM}")));
        }
        let url_data = decoded_field(&video_id, &data, "new_play_url")?;
        let key = decoded_field(&video_id, &data, "key")?;
        let manifest = decrypt_manifest_url(&video_id, &url_data, &key)?;
        let stream_key = data
            .get("stream_key")
            .and_then(Value::as_str)
            .ok_or_else(|| super::clyp::key_error(&video_id, "stream_key"))?;
        let manifest = Url::parse(&manifest)?;
        let headers = [("stream_key", stream_key)];
        let (mut formats, mut subtitles) = (Vec::new(), Subtitles::default());
        if let Some(mut stream) = super::extract_m3u8_master_optional_headers(
            client, &manifest, NAME, &video_id, &headers,
        ) {
            formats = std::mem::take(&mut stream.formats);
            subtitles = std::mem::take(&mut stream.subtitles);
        }
        let mut header_map = Map::new();
        header_map.insert("stream_key".into(), Value::from(stream_key));
        for format in &mut formats {
            format.http_headers = Some(header_map.clone());
            // `fmt['http_headers'] = headers` after the manifest's dict.
            crate::info_order::mark_format_update(format, "http_headers");
        }

        // `subtitles.setdefault('EN', []).append(...)`, then the manifest's.
        let mut declared = Subtitles::default();
        if let Some(sub_url) = data
            .get("subtitle")
            .and_then(Value::as_str)
            .filter(|sub_url| !sub_url.is_empty())
        {
            let sub_url = Url::parse(&super::proto_relative(sub_url))?;
            let ext = super::direct::determine_ext(sub_url.as_str()).unwrap_or_default();
            let track = SubtitleFormat {
                name: None,
                url: sub_url,
                ext,
                protocol: None,
                fragments: Vec::new(),
                extra: Map::new(),
            };
            declared.entries.push((Some("EN".into()), vec![track]));
        }
        super::html5::merge_subtitles(&mut declared, subtitles);

        let release_date = RELEASE_DATES
            .iter()
            .find_map(|pattern| pattern.captures(&webpage).map(|found| found[1].to_owned()));
        let release_date = release_date
            .and_then(|date| crate::dates::unified_strdate(&date, false))
            .map_or(Value::Null, Value::from);
        let description = DESCRIPTION
            .captures(&webpage)
            .map(|found| crate::html::clean_html(&found[1]))
            .map_or(Value::Null, Value::from);

        let fields = vec![
            ("id", Value::from(video_id)),
            ("formats", Value::Null),
            ("title", Value::from(title)),
            ("thumbnail", thumbnail),
            ("release_date", release_date),
            ("description", description),
            ("subtitles", Value::Null),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = declared;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.shemaroome.com/movies/dil-hai-tumhaara"
        ));
        assert!(suitable(
            "https://www.shemaroome.com/shows/jurm-aur-jazbaat/laalach"
        ));
        assert!(!suitable("https://www.shemaroome.com/"));
    }

    #[test]
    fn hidden_fields_are_matched_by_id_and_value() {
        let page = r#"<input type="hidden" id="ma_title" value="Dil Hai Tumhaara">"#;
        assert_eq!(
            hidden_value(page, "ma_title").as_deref(),
            Some("Dil Hai Tumhaara")
        );
        assert_eq!(hidden_value(page, "catalog_id"), None);
    }
}
