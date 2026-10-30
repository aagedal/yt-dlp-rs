//! Upstream `VideaIE` (`yt_dlp/extractor/videa.py`): videa.hu videos from the
//! page's player iframe. The player's `player/xml` answer is plain XML or
//! RC4-encrypted base64, keyed by the player's nonce, a random seed and the
//! response's `x-videa-xs` header; its `video_source` entries give the formats.
use super::{Extractor, dict_format, dict_info, download_webpage, download_webpage_with_headers};
use crate::{Error, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::hash::{BuildHasher, Hasher};
use std::sync::LazyLock;
use url::Url;

pub struct Videa;

const NAME: &str = "Videa";
const KEY: &str = "Videa";
const STATIC_SECRET: &str = "xHb0ZvME5q8CBcoQi6AngerDu3FGO9fkUlwPmLVY_RTzj2hJIS4NasXWKy1td7p";
const XML_URL: &str = "http://videa.hu/player/xml";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"^https?://videa(?:kid)?\.hu/",
        r"(?:videok/(?:[^/]+/)*[^?#&]+-|",
        r"(?:videojs_)?player\?.*?\bv=|",
        r"player/v/)",
        r"(?P<id>[^?#&]+)",
    ))
    .unwrap()
});

static PLAYER_IFRAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<iframe.*?src="(/player\?[^"]+)""#).unwrap());

static NONCE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"_xt\s*=\s*"([^"]+)""#).unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("videa {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    super::clyp::key_error(id, key)
}

/// `RC4` from `videa.py`: the keystream XORed over `cipher_text`, decoded as
/// UTF-8 (`res.decode()`).
pub(crate) fn rc4(cipher_text: &[u8], key: &str) -> Result<String> {
    let key: Vec<u32> = key.chars().map(u32::from).collect();
    let mut state: Vec<u8> = (0..=255).collect();
    let mut j: u32 = 0;
    for i in 0..256 {
        let key_char = key[i % key.len()];
        j = (j + u32::from(state[i]) + key_char) % 256;
        state.swap(i, j as usize);
    }
    let mut i: usize = 0;
    let mut j: usize = 0;
    let mut out = Vec::with_capacity(cipher_text.len());
    for &byte in cipher_text {
        i = (i + 1) % 256;
        j = (j + usize::from(state[i])) % 256;
        state.swap(i, j);
        let k = state[(usize::from(state[i]) + usize::from(state[j])) % 256];
        out.push(k ^ byte);
    }
    String::from_utf8(out).map_err(|_| unsupported("RC4 output that is not UTF-8"))
}

/// `parse_qs(url)`: the query's pairs in order, blank values dropped and
/// percent-decoded (`+` is a space).
fn parse_qs(url: &Url) -> Vec<(String, String)> {
    url.query()
        .unwrap_or_default()
        .split('&')
        .filter_map(|pair| {
            let (name, value) = pair.split_once('=')?;
            let decode = |text: &str| {
                percent_encoding::percent_decode_str(&text.replace('+', " "))
                    .decode_utf8_lossy()
                    .into_owned()
            };
            let value = decode(value);
            (!value.is_empty()).then(|| (decode(name), value))
        })
        .collect()
}

/// `random.choices(ascii_letters + digits, k=8)`.
fn random_seed() -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    let mut seed = String::new();
    for index in 0..8u64 {
        hasher.write_u64(index);
        let value = hasher.finish();
        seed.push(char::from(CHARS[(value % CHARS.len() as u64) as usize]));
    }
    seed
}

/// `xpath_element(node, path)`: the first direct child with `tag`.
fn child<'a, 'input>(
    node: roxmltree::Node<'a, 'input>,
    tag: &str,
) -> Option<roxmltree::Node<'a, 'input>> {
    node.children()
        .find(|item| item.is_element() && item.tag_name().name() == tag)
}

/// `ExtractorError(message)` from an extractor: `[IE] ID: message`, with the
/// bug report note (upstream does not expect these).
fn fatal(video_id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {video_id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `xpath_text(node, path, fatal=True)`: the element's text.
fn fatal_text(video_id: &str, node: roxmltree::Node<'_, '_>, tag: &str) -> Result<String> {
    let element = child(node, tag)
        .ok_or_else(|| fatal(video_id, &format!("Could not find XML element ./{tag}")))?;
    element.text().map(str::to_owned).ok_or_else(|| {
        fatal(
            video_id,
            &format!("Could not find XML element's text ./{tag}"),
        )
    })
}

/// `xpath_text(node, path)`: the element's text, `None` when it is missing.
fn optional_text(node: roxmltree::Node<'_, '_>, tag: &str) -> Option<String> {
    child(node, tag).and_then(|element| element.text().map(str::to_owned))
}

/// `int_or_none` of an XML text.
fn int_text(text: Option<&str>) -> Value {
    text.and_then(|text| text.trim().parse::<i64>().ok())
        .map_or(Value::Null, Value::from)
}

/// The `nonce`-derived `_t` value (the first half of `result`).
fn derive_result(nonce: &str) -> Result<String> {
    let chars: Vec<char> = nonce.chars().collect();
    if chars.len() < 64 {
        return Err(unsupported("nonces shorter than 64 characters"));
    }
    let (first, second) = chars.split_at(32);
    let secret: Vec<char> = STATIC_SECRET.chars().collect();
    let mut result = String::new();
    for (i, letter) in first.iter().enumerate() {
        let index = secret
            .iter()
            .position(|candidate| candidate == letter)
            .ok_or_else(|| unsupported("nonces with characters outside the static secret"))?
            as i64;
        // Python's negative indexing wraps around the second half.
        let mut position = i as i64 - (index - 31);
        if position < 0 {
            position += second.len() as i64;
        }
        let character = second
            .get(usize::try_from(position).map_err(|_| unsupported("nonce indexes"))?)
            .ok_or_else(|| unsupported("nonce indexes out of range"))?;
        result.push(*character);
    }
    Ok(result)
}

impl Extractor for Videa {
    fn key(&self) -> &'static str {
        KEY
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

        let video_page = download_webpage(client, url, NAME, &video_id)?;
        let (player_url, player_page) = if url.as_str().contains("videa.hu/player") {
            (url.clone(), video_page)
        } else {
            let src = PLAYER_IFRAME.captures(&video_page).ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract player url; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
            let player_url = url.join(&src[1])?;
            let player_page = download_webpage(client, &player_url, NAME, &video_id)?;
            (player_url, player_page)
        };

        let nonce = NONCE.captures(&player_page).ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract nonce; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
        let result = derive_result(&nonce[1])?;

        let mut query = parse_qs(&player_url);
        let seed = random_seed();
        query.push(("_s".into(), seed.clone()));
        query.push(("_t".into(), result.chars().take(16).collect()));
        let mut xml_url = Url::parse(XML_URL)?;
        {
            let mut pairs = xml_url.query_pairs_mut();
            for (name, value) in &query {
                pairs.append_pair(name, value);
            }
        }

        let (b64_info, headers) =
            download_webpage_with_headers(client, &xml_url, NAME, &video_id, &[])?;
        let info_text = if b64_info.starts_with("<?xml") {
            b64_info
        } else {
            let xs = headers
                .get("x-videa-xs")
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| key_error(&video_id, "x-videa-xs"))?;
            let key: String = result.chars().skip(16).collect::<String>() + &seed + xs;
            // `base64.b64decode` ignores characters outside the alphabet.
            let payload: String = b64_info
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '+' || *c == '/')
                .collect();
            let cipher = base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(payload)
                .map_err(|_| unsupported("encrypted payloads that are not base64"))?;
            rc4(&cipher, &key)?
        };

        let document = roxmltree::Document::parse(info_text.trim_start_matches('\u{feff}'))
            .map_err(|_| Error::Reported(format!("[{NAME}] {video_id}: Failed to parse XML")))?;
        let info = document.root_element();

        let Some(video) = child(info, "video") else {
            // Upstream prints the `<error>` element object here.
            let error = child(info, "error")
                .ok_or_else(|| fatal(&video_id, "Could not find XML element ./error"))?;
            return Err(Error::Reported(error.text().unwrap_or_default().to_owned()));
        };
        let sources = child(info, "video_sources")
            .ok_or_else(|| fatal(&video_id, "Could not find XML element sources"))?;
        let hash_values = child(info, "hash_values");

        let title = fatal_text(&video_id, video, "title")?;

        let mut formats = Vec::new();
        for source in sources
            .children()
            .filter(|node| node.is_element() && node.tag_name().name() == "video_source")
        {
            let Some(source_url) = source.text() else {
                continue;
            };
            let Some(source_name) = source.attribute("name") else {
                continue;
            };
            let source_exp = source.attribute("exp");
            let mut source_url = Url::parse(&super::proto_relative(source_url))
                .map_err(|_| unsupported("video source URLs that are not URLs"))?;
            let hash_value = hash_values
                .and_then(|hashes| optional_text(hashes, &format!("hash_value_{source_name}")));
            if let (Some(hash_value), Some(source_exp)) = (hash_value, source_exp)
                && !hash_value.is_empty()
                && !source_exp.is_empty()
            {
                source_url = super::update_url_query(
                    &source_url,
                    &[("md5", &hash_value), ("expires", source_exp)],
                );
            }
            let mut fields =
                super::zdf::codec_fields(source.attribute("codecs").map(Value::from).as_ref());
            let mime = source.attribute("mimetype").map(Value::from);
            let ext = match super::vidyard::mimetype2ext(mime.as_ref()) {
                Value::Null => "mp4".to_owned(),
                ext => ext.as_str().unwrap_or("mp4").to_owned(),
            };
            fields.extend([
                ("url", source_url.as_str().into()),
                ("ext", ext.into()),
                ("format_id", source_name.into()),
                ("width", int_text(source.attribute("width"))),
                ("height", int_text(source.attribute("height"))),
            ]);
            formats.push(dict_format(fields, NAME, &video_id)?);
        }

        let thumbnail = optional_text(video, "poster_src")
            .map_or(Value::Null, |poster| super::proto_relative(&poster).into());
        let age_limit = match optional_text(video, "is_adult_content").as_deref() {
            Some(text) if !text.is_empty() => Value::from(if text == "1" { 18 } else { 0 }),
            _ => Value::Null,
        };

        Ok(Some(dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", video_id.into()),
                ("title", title.into()),
                ("thumbnail", thumbnail),
                (
                    "duration",
                    int_text(optional_text(video, "duration").as_deref()),
                ),
                ("age_limit", age_limit),
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
        assert!(suitable(
            "http://videa.hu/videok/allatok/az-orult-kigyasz-285-kigyot-kigyo-8YfIAjxwWGwT8HVQ"
        ));
        assert!(suitable("http://videa.hu/player?v=8YfIAjxwWGwT8HVQ"));
        assert!(suitable(
            "https://videakid.hu/player/v/8YfIAjxwWGwT8HVQ?autoplay=1"
        ));
        assert!(!suitable("https://videa.hu/"));
    }

    #[test]
    fn rc4_matches_the_standard_vector() {
        // RC4("Key", "Plaintext") = BBF316E8D940AF0AD3 (hex).
        let cipher = [0xBB, 0xF3, 0x16, 0xE8, 0xD9, 0x40, 0xAF, 0x0A, 0xD3];
        assert_eq!(rc4(&cipher, "Key").unwrap(), "Plaintext");
    }
}
