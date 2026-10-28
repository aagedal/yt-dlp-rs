//! Upstream `DamtomoVideoIE` and `DamtomoRecordIE`
//! (`yt_dlp/extractor/damtomo.py`, `DamtomoBaseIE`): DAM★とも karaoke
//! movies and posts from their Shift JIS page (the song, singer and counts)
//! and the stream XML's HLS playlist.
use super::{Extractor, dict_info};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct DamtomoVideo;
pub struct DamtomoRecord;

/// One of the two extractors' URLs and names.
struct Site {
    name: &'static str,
    key: &'static str,
    valid_url: &'static LazyLock<Regex>,
    webpage_url_tmpl: &'static str,
    dkml_xml_url: &'static str,
    dkml_xml_ns: &'static str,
}

static VIDEO_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?clubdam\.com/app/damtomo/(?:SP/)?karaokeMovie/StreamingDkm\.do\?karaokeMovieId=(?P<id>\d+)",
    )
    .unwrap()
});
static RECORD_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?clubdam\.com/app/damtomo/(?:SP/)?karaokePost/StreamingKrk\.do\?karaokeContributeId=(?P<id>\d+)",
    )
    .unwrap()
});

static VIDEO: Site = Site {
    name: "damtomo:video",
    key: "DamtomoVideo",
    valid_url: &VIDEO_URL,
    webpage_url_tmpl: "https://www.clubdam.com/app/damtomo/karaokeMovie/StreamingDkm.do?karaokeMovieId=",
    dkml_xml_url: "https://www.clubdam.com/app/damtomo/karaokeMovie/GetStreamingDkmUrlXML.do?movieSelectFlg=2&karaokeMovieId=",
    dkml_xml_ns: "https://www.clubdam.com/app/damtomo/karaokeMovie/GetStreamingDkmUrlXML",
};
static RECORD: Site = Site {
    name: "damtomo:record",
    key: "DamtomoRecord",
    valid_url: &RECORD_URL,
    webpage_url_tmpl: "https://www.clubdam.com/app/damtomo/karaokePost/StreamingKrk.do?karaokeContributeId=",
    dkml_xml_url: "https://www.clubdam.com/app/damtomo/karaokePost/GetStreamingKrkUrlXML.do?karaokeContributeId=",
    dkml_xml_ns: "https://www.clubdam.com/app/damtomo/karaokePost/GetStreamingKrkUrlXML",
};

static DESCRIPTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?m)<div id="public_comment">\s*<p>\s*([^<]*?)\s*</p>"#).unwrap()
});
static UPLOADER_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<a href="https://www\.clubdam\.com/app/damtomo/member/info/Profile\.do\?damtomoId=([^"]+)""#,
    )
    .unwrap()
});
/// The start of `(?s)<(p|div)\s+class="(?P<class>[^" ]+?)">(?P<value>.+?)</\1>`.
static DATA_START: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"<(p|div)\s+class="([^" ]+)">"#).unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
static SAN: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*さん\s*$").unwrap());
static DATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{4}/\d{2}/\d{2})").unwrap());
static DIGITS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d+)").unwrap());
static XML_ENCODING: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\s*encoding="[^"]+?""#).unwrap());

/// `DamtomoVideoIE._match_valid_url`.
pub fn video_suitable(url: &str) -> bool {
    VIDEO_URL.is_match(url)
}

/// `DamtomoRecordIE._match_valid_url`.
pub fn record_suitable(url: &str) -> bool {
    RECORD_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// A request decoded with `encoding='sjis'`: the text and final URL.
fn download_sjis(
    client: &Client,
    url: &Url,
    (extractor, id): (&'static str, &str),
    (note, what): (&str, &'static str),
) -> Result<(String, Url)> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    let mut response = super::direct::request(client, url, None).map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor,
            id: id.to_owned(),
            what,
            source: Box::new(error),
        },
        error => error,
    })?;
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    Ok((decode_sjis(&body), page_url))
}

/// Python's `bytes.decode('sjis', 'replace')`: JIS X 0208 only (no NEC,
/// IBM or user-defined rows), whose six symbols differ from the WHATWG
/// decoder; an undecodable byte is U+FFFD and decoding resumes after it.
fn decode_sjis(bytes: &[u8]) -> String {
    let mut out = String::new();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte < 0x80 {
            out.push(char::from(byte));
            index += 1;
            continue;
        }
        if (0xa1..=0xdf).contains(&byte) {
            out.push(char::from_u32(0xff61 + u32::from(byte - 0xa1)).unwrap_or('\u{fffd}'));
            index += 1;
            continue;
        }
        let lead = matches!(byte, 0x81..=0x86 | 0x88..=0x9f | 0xe0..=0xea);
        let pair = bytes
            .get(index..index + 2)
            .filter(|_| lead)
            .and_then(|pair| {
                let remapped = match pair {
                    [0x81, 0x60] => Some('\u{301c}'),
                    [0x81, 0x61] => Some('\u{2016}'),
                    [0x81, 0x7c] => Some('\u{2212}'),
                    [0x81, 0x91] => Some('\u{a2}'),
                    [0x81, 0x92] => Some('\u{a3}'),
                    [0x81, 0xca] => Some('\u{ac}'),
                    _ => None,
                };
                remapped.or_else(|| {
                    let text = encoding_rs::SHIFT_JIS
                        .decode_without_bom_handling_and_without_replacement(pair)?;
                    let mut chars = text.chars();
                    let found = chars.next().filter(|found| !found.is_ascii())?;
                    chars.next().is_none().then_some(found)
                })
            });
        match pair {
            Some(found) => {
                out.push(found);
                index += 2;
            }
            None => {
                out.push('\u{fffd}');
                index += 1;
            }
        }
    }
    out
}

/// The page's `<p class=...>`/`<div class=...>` texts (`re.finditer` with
/// the closing tag's back reference), whitespace-collapsed.
fn data_dict(webpage: &str) -> Map<String, Value> {
    let mut data = Map::new();
    let mut position = 0;
    while let Some(found) = DATA_START.captures_at(webpage, position) {
        let whole = found.get(0).unwrap();
        let close = format!("</{}>", &found[1]);
        // `.+?`: at least one character before the closing tag.
        let value_start = whole.end();
        let first = webpage[value_start..]
            .chars()
            .next()
            .map_or(0, char::len_utf8);
        match (first > 0)
            .then(|| webpage[value_start + first..].find(&close))
            .flatten()
        {
            Some(offset) => {
                let value_end = value_start + first + offset;
                let value = crate::html::clean_html(&webpage[value_start..value_end]);
                data.insert(
                    found[2].to_owned(),
                    SPACES.replace_all(&value, " ").into_owned().into(),
                );
                position = value_end + close.len();
            }
            None => {
                // The search moves on from the next character.
                position = whole.start()
                    + webpage[whole.start()..]
                        .chars()
                        .next()
                        .map_or(1, char::len_utf8);
            }
        }
    }
    data
}

/// `int_or_none(self._search_regex(r'(\d+)', TEXT, NAME, default=None))`.
fn count(text: &str) -> Value {
    DIGITS
        .captures(text)
        .and_then(|found| found[1].parse::<i64>().ok())
        .map_or(Value::Null, Value::from)
}

fn extract(site: &Site, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
    let Some(found) = site.valid_url.captures(url.as_str()) else {
        return Ok(None);
    };
    let video_id = found["id"].to_owned();
    let name = site.name;
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
    let (webpage, handle) = download_sjis(
        client,
        &Url::parse(&format!("{}{video_id}", site.webpage_url_tmpl))?,
        (name, &video_id),
        ("Downloading webpage", "webpage"),
    )?;
    if handle.as_str() == "https://www.clubdam.com/sorry/" {
        return Err(Error::Reported(format!(
            "[{name}] {video_id}: You are rate-limited. Try again later."
        )));
    }
    if webpage.contains("<h2>予期せぬエラーが発生しました。</h2>") {
        return Err(Error::Reported(format!(
            "[{name}] {video_id}: There is an error on server-side. Try again later."
        )));
    }
    let description = DESCRIPTION
        .captures(&webpage)
        .map(|found| found[1].to_owned());
    let uploader_id = UPLOADER_ID
        .captures(&webpage)
        .map(|found| found[1].to_owned());
    let mut data = data_dict(&webpage);
    // Videos have no title: the song's name is used instead.
    let user_name = data
        .get("user_name")
        .and_then(Value::as_str)
        .ok_or_else(|| key_error(&video_id, "user_name"))?;
    let user_name = SAN.replace(user_name, "").into_owned();
    data.insert("user_name".into(), user_name.into());
    let title = data.get("song_title").cloned().unwrap_or(Value::Null);
    let (xml, _) = download_sjis(
        client,
        &Url::parse(&format!("{}{video_id}", site.dkml_xml_url))?,
        (name, &video_id),
        ("Requesting stream information", "XML"),
    )?;
    // No character outside ASCII is expected, so dropping the declared
    // encoding is safe.
    let xml = XML_ENCODING.replace_all(&xml, "").into_owned();
    let document = roxmltree::Document::parse(&xml).map_err(|error| {
        Error::Reported(format!(
            "[{name}] {video_id}: Failed to parse XML ({error}); {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    // `.find('.//d:streamingUrl', ...).text.strip()` under `try_get`.
    let m3u8_url = document
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "streamingUrl"
                && node.tag_name().namespace() == Some(site.dkml_xml_ns)
                && node != &document.root_element()
        })
        .and_then(|node| node.first_child().filter(|child| child.is_text()))
        .and_then(|text| text.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{name}] {video_id}: Failed to obtain m3u8 URL; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    let formats = super::extract_m3u8_fatal_master(
        client,
        &Url::parse(m3u8_url)?,
        name,
        &video_id,
        None,
        false,
    )?
    .formats;
    let upload_date = data
        .get("date")
        .and_then(Value::as_str)
        .and_then(|date| DATE.captures(date))
        .and_then(|found| crate::dates::unified_strdate(&found[1], true));
    let view_count = count(
        data.get("audience")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&video_id, "audience"))?,
    );
    let like_count = count(
        data.get("nice")
            .and_then(Value::as_str)
            .ok_or_else(|| key_error(&video_id, "nice"))?,
    );
    let field = |value: Option<String>| value.map_or(Value::Null, Value::from);
    Ok(Some(dict_info(
        url,
        (name, site.key),
        vec![
            ("id", video_id.clone().into()),
            ("title", title.clone()),
            ("uploader_id", field(uploader_id)),
            ("description", field(description)),
            (
                "uploader",
                data.get("user_name").cloned().unwrap_or(Value::Null),
            ),
            ("upload_date", field(upload_date)),
            ("view_count", view_count),
            ("like_count", like_count),
            ("track", title),
            (
                "artist",
                data.get("song_artist").cloned().unwrap_or(Value::Null),
            ),
            ("formats", Value::Null),
        ],
        formats,
    )))
}

impl Extractor for DamtomoVideo {
    fn key(&self) -> &'static str {
        VIDEO.key
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        extract(&VIDEO, client, url)
    }
}

impl Extractor for DamtomoRecord {
    fn key(&self) -> &'static str {
        RECORD.key
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        extract(&RECORD, client, url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(video_suitable(
            "https://www.clubdam.com/app/damtomo/karaokeMovie/StreamingDkm.do?karaokeMovieId=2414316"
        ));
        assert!(record_suitable(
            "https://www.clubdam.com/app/damtomo/SP/karaokePost/StreamingKrk.do?karaokeContributeId=27376862"
        ));
        assert!(!video_suitable(
            "https://www.clubdam.com/app/damtomo/karaokePost/StreamingKrk.do?karaokeContributeId=27376862"
        ));
    }

    #[test]
    fn shift_jis_follows_python() {
        // Values from Python's `bytes.decode('sjis', 'replace')`.
        assert_eq!(
            decode_sjis(b"a\x81\x60\x82\xa0\xb1\x87\x40\x81\xfd\x80~\x81\x7c"),
            "a\u{301c}\u{3042}\u{ff71}\u{fffd}@\u{fffd}\u{fffd}\u{fffd}~\u{2212}"
        );
        assert_eq!(decode_sjis(b"\x88"), "\u{fffd}");
    }

    #[test]
    fn data_follows_the_closing_tag_back_reference() {
        let data = data_dict(
            r#"<p class="a"></p><div class="b">x <p>y</p>
            z</div><p class="c">"#,
        );
        assert_eq!(Value::Object(data), serde_json::json!({"a": "x y"}));
    }
}
