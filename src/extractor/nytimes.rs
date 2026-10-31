//! Upstream `NYTimesIE` (`yt_dlp/extractor/nytimes.py`): a video page's
//! GraphQL record (renditions, crops, byline) with HLS masters and direct
//! renditions as formats. `NYTimesArticleIE`, `NYTimesCookingIE` and
//! `NYTimesCookingRecipeIE` (article and recipe pages) are not ported.
use super::{Extractor, dict_format, dict_info, post_json_errnote_optional};
use crate::{ExtractResult, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct NYTimes;

const NAME: &str = "NYTimes";
const DNS_NAMESPACE: &str = "36dd619a-56dc-595b-9e09-37f4152c7b5d";
const TOKEN: &str = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAuNIzKBOFB77aT/jN/FQ+/QVKWq5V1ka1AYmCR9hstz1pGNPH5ajOU9gAqta0T89iPnhjwla+3oec/Z3kGjxbpv6miQXufHFq3u2RC6HyU458cLat5kVPSOQCe3VVB5NRpOlRuwKHqn0txfxnwSSj8mqzstR997d3gKB//RO9zE16y3PoWlDQXkASngNJEWvL19iob/xwAkfEWCjyRILWFY0JYX3AvLMSbq7wsqOCE5srJpo7rRU32zsByhsp1D5W9OYqqwDmflsgCEQy2vqTsJjrJohuNg+urMXNNZ7Y3naMoqttsGDrWVxtPBafKMI8pM2ReNZBbGQsQXRzQNo7+QIDAQAB";
const GRAPHQL_API: &str = "https://samizdat-graphql.nytimes.com/graphql/v2";
const GRAPHQL_QUERY: &str = "query VideoQuery($id: String!) {
  video(id: $id) {
    ... on Video {
      bylines {
        renderedRepresentation
      }
      duration
      firstPublished
      promotionalHeadline
      promotionalMedia {
        ... on Image {
          crops {
            name
            renditions {
              name
              width
              height
              url
            }
          }
        }
      }
      renditions {
        type
        width
        height
        url
        bitrate
      }
      summary
    }
  }
}";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:www\.)?nytimes\.com/video/(?:[^/]+/)+?|graphics8\.nytimes\.com/bcvideo/\d+(?:\.\d+)?/iframe/embed\.html\?videoId=)(?P<id>\d+)",
    )
    .unwrap()
});

/// `uuid.uuid5(namespace, name)` (RFC 4122 version 5, SHA-1).
fn uuid5(namespace: &[u8; 16], name: &str) -> [u8; 16] {
    let mut data = namespace.to_vec();
    data.extend_from_slice(name.as_bytes());
    let digest = crate::sha1::digest(&data);
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&digest[..16]);
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    bytes
}

fn uuid_bytes(text: &str) -> [u8; 16] {
    let hex: String = text.chars().filter(|c| *c != '-').collect();
    let mut bytes = [0u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).unwrap_or_default();
    }
    bytes
}

fn uuid_text(bytes: &[u8; 16]) -> String {
    let hex: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &hex[0..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..32]
    )
}

/// `_call_api(media_id)`: the video's GraphQL record (`{}` when it fails).
fn call_api(client: &Client, media_id: &str) -> Result<Value> {
    // reference: `id-to-uri.js`
    let video_uuid = uuid5(&uuid_bytes(DNS_NAMESPACE), "video");
    let media_uuid = uuid5(&video_uuid, media_id);
    let body = json!({
        "query": GRAPHQL_QUERY,
        "variables": {"id": format!("nyt://video/{}", uuid_text(&media_uuid))},
    });
    let response = post_json_errnote_optional(
        client,
        &Url::parse(GRAPHQL_API)?,
        (NAME, media_id),
        (
            "Downloading JSON from GraphQL API",
            "Unable to download JSON metadata",
        ),
        &[
            ("Content-Type", "application/json"),
            ("Nyt-App-Type", "vhs"),
            ("Nyt-App-Version", "v3.52.21"),
            ("Nyt-Token", TOKEN),
            ("Origin", "https://nytimes.com"),
        ],
        body.to_string(),
    )?;
    Ok(response
        .as_ref()
        .and_then(|json| json.get("data"))
        .and_then(|data| data.get("video"))
        .filter(|video| video.is_object())
        .cloned()
        .unwrap_or_else(|| json!({})))
}

fn int_value(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `_extract_thumbnails(thumbs)`: the crops' renditions with a URL.
fn thumbnails(data: &Value) -> Option<Value> {
    let mut list = Vec::new();
    let crops = data
        .get("promotionalMedia")
        .and_then(|media| media.get("crops"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten();
    for crop in crops {
        for rendition in crop
            .get("renditions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(url) = rendition
                .get("url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
            else {
                continue;
            };
            let mut thumb = serde_json::Map::new();
            thumb.insert("url".into(), json!(url));
            if let Some(width) = int_value(rendition.get("width")) {
                thumb.insert("width".into(), json!(width));
            }
            if let Some(height) = int_value(rendition.get("height")) {
                thumb.insert("height".into(), json!(height));
            }
            list.push(Value::Object(thumb));
        }
    }
    if list.is_empty() {
        None
    } else {
        Some(Value::Array(list))
    }
}

/// `_extract_formats_and_subtitles(video_id, content_media_json)`.
fn formats_and_subtitles(
    client: &Client,
    media_id: &str,
    data: &Value,
) -> Result<(Vec<VideoFormat>, crate::Subtitles)> {
    let mut urls: Vec<String> = Vec::new();
    let mut formats = Vec::new();
    let mut subtitles = crate::Subtitles::default();
    for video in data
        .get("renditions")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|video| video.is_object())
    {
        let Some(video_url) = video
            .get("url")
            .and_then(Value::as_str)
            .filter(|url| !url.is_empty())
        else {
            continue;
        };
        let format_id = video.get("type").and_then(Value::as_str).map(str::to_owned);
        if format_id.as_deref() == Some("thumbs") || urls.iter().any(|seen| seen == video_url) {
            continue;
        }
        urls.push(video_url.to_owned());
        let mime = video.get("mimetype").and_then(Value::as_str);
        let ext = mime
            .map(|mime| super::direct::mimetype2ext_or(mime, None))
            .filter(|ext| !ext.is_empty())
            .or_else(|| super::direct::determine_ext(video_url))
            .unwrap_or_default();
        if ext == "m3u8" {
            let m3u8_id = format_id.clone().unwrap_or_else(|| "hls".into());
            let manifest = Url::parse(video_url)?;
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                media_id,
                Some(&m3u8_id),
                true,
                false,
            ) {
                formats.extend(stream.formats);
                super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else if ext == "mpd" {
            // All mpd urls give 404 errors.
            continue;
        } else {
            let filesize = int_value(video.get("file_size").or_else(|| video.get("fileSize")));
            let bitrate = int_value(video.get("bitrate")).map(|rate| rate / 1000);
            let bitrate = bitrate.filter(|rate| *rate != 0);
            let mut format = dict_format(
                vec![
                    ("url", json!(video_url)),
                    ("format_id", json!(format_id)),
                    (
                        "vcodec",
                        video
                            .get("videoencoding")
                            .or_else(|| video.get("video_codec"))
                            .cloned()
                            .unwrap_or(Value::Null),
                    ),
                    ("width", json!(int_value(video.get("width")))),
                    ("height", json!(int_value(video.get("height")))),
                    ("filesize", json!(filesize)),
                    ("tbr", json!(bitrate)),
                    ("ext", json!(ext)),
                ],
                NAME,
                media_id,
            )?;
            crate::info_order::mark_int_rates(&mut format);
            formats.push(format);
        }
    }
    Ok((formats, subtitles))
}

impl Extractor for NYTimes {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let media_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let data = call_api(client, &media_id)?;
        let (formats, subtitles) = formats_and_subtitles(client, &media_id, &data)?;

        let duration = data
            .get("duration")
            .and_then(Value::as_f64)
            .map(|duration| duration / 1000.0);
        let timestamp = data
            .get("firstPublished")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601);
        // `', '.join(By-stripped bylines)`: empty when there are none.
        let creator = data
            .get("bylines")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|byline| byline.get("renderedRepresentation").and_then(Value::as_str))
            .map(|text| text.strip_prefix("By ").unwrap_or(text).to_owned())
            .collect::<Vec<_>>()
            .join(", ");
        let fields = vec![
            ("id", json!(media_id)),
            (
                "title",
                data.get("promotionalHeadline")
                    .cloned()
                    .unwrap_or(Value::Null),
            ),
            (
                "description",
                data.get("summary").cloned().unwrap_or(Value::Null),
            ),
            ("timestamp", json!(timestamp)),
            ("duration", json!(duration)),
            ("creator", json!(creator)),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("thumbnails", json!(thumbnails(&data))),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        // Upstream's thumbnail dicts keep `url`, `width`, `height` before the
        // `id` the sanitizer adds.
        if let Some(Value::String(shape)) = info.extra.get(crate::info_order::INFO_SHAPE).cloned() {
            info.extra.insert(
                crate::info_order::INFO_SHAPE.into(),
                format!("{shape};thumbnail=url,width,height,id").into(),
            );
        }
        Ok(Some(ExtractResult::Video(info)))
    }
}

/// URL matching for `--list-extractors`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "http://www.nytimes.com/video/opinion/100000002847155/verbatim-what-is-a-photocopier.html?playlistId=100000001150263"
        ));
        assert!(suitable(
            "https://graphics8.nytimes.com/bcvideo/1.0/iframe/embed.html?videoId=100000002847155"
        ));
        assert!(!suitable(
            "https://www.nytimes.com/2015/04/14/business/owner-of-gravity.html"
        ));
    }

    #[test]
    fn media_uuid_matches_the_graphql_id() {
        let video = uuid5(&uuid_bytes(DNS_NAMESPACE), "video");
        let media = uuid5(&video, "100000002847155");
        assert_eq!(uuid_text(&media), "88a5c80d-0382-575f-a377-8e6e7d63eb60");
    }
}
