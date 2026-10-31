//! Upstream `TeamcocoIE` (`yt_dlp/extractor/teamcoco.py`): a teamcoco.com
//! page's Next.js `pageData` blocks (`meta-tags`, `video-player`,
//! `video-info`) give the title, description, publish date and the player's
//! HLS sources, whose master playlists supply the formats. Upstream's
//! `ConanClassicIE` (`_WORKING = False`) is not ported.
use super::{Extractor, dict_format, dict_info, download_webpage, search_nextjs_data};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Teamcoco;

const NAME: &str = "Teamcoco";
const BASE: &str = "https://teamcoco.com/";

/// `_QUALITIES`: a progressive source's label to its (width, height).
const QUALITIES: &[(&str, (i64, i64))] = &[
    ("low", (480, 272)),
    ("sd", (640, 360)),
    ("hd", (1280, 720)),
    ("uhd", (1920, 1080)),
];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(?:www\.)?teamcoco\.com/(?P<id>([^/]+/)*[^/?#]+)")
        .unwrap()
});
static SCHEME_SLASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^https?:/[^/]").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Teamcoco {what} are not supported natively"))
}

/// `merge_dicts(*dicts)`: first non-`None` values, with empty strings
/// replaced by later non-empty strings.
fn merge_dicts(dicts: Vec<&Map<String, Value>>) -> Map<String, Value> {
    let mut merged = Map::new();
    for dict in dicts {
        for (key, value) in dict {
            let replace = match merged.get(key) {
                None => !value.is_null(),
                Some(Value::String(existing)) => existing.is_empty() && value.is_string(),
                Some(_) => false,
            };
            if replace {
                merged.insert(key.clone(), value.clone());
            }
        }
    }
    merged
}

/// `traverse_obj(info, (('image', 'poster'), {urljoin(BASE)}), get_all=False)`.
fn thumbnail_url(info: &Map<String, Value>) -> Option<Url> {
    let base = Url::parse(BASE).ok()?;
    ["image", "poster"]
        .iter()
        .filter_map(|key| info.get(*key)?.as_str())
        .find_map(|value| base.join(value).ok())
}

/// `parse_qs(thumbnail).get('id')[0]`.
fn query_id(thumbnail: &Url) -> Option<String> {
    thumbnail
        .query_pairs()
        .find(|(key, value)| key == "id" && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// `_get_formats_and_subtitles`.
fn formats_and_subtitles(
    client: &Client,
    info: &Map<String, Value>,
    video_id: &str,
) -> Result<(Vec<crate::VideoFormat>, Subtitles)> {
    let mut formats = Vec::new();
    let mut subtitles = Subtitles::default();
    let sources = match info.get("src") {
        Some(Value::Array(sources)) => sources.as_slice(),
        None | Some(Value::Null) => &[],
        Some(_) => return Err(unsupported("player sources that are not lists")),
    };
    for source in sources {
        let Some(source) = source.as_object() else {
            continue;
        };
        let Some(format_id) = source
            .get("label")
            .and_then(Value::as_str)
            .filter(|label| !label.is_empty())
        else {
            continue;
        };
        let Some(mut src_url) = source
            .get("src")
            .and_then(Value::as_str)
            .filter(|src| !src.is_empty())
            .map(str::to_owned)
        else {
            continue;
        };
        if SCHEME_SLASH.is_match(&src_url) {
            src_url = src_url.replacen(":/", "://", 1);
        }
        let mime = source.get("type");
        let ext = super::direct::determine_ext(&src_url).unwrap_or_else(|| {
            super::direct::mimetype2ext_or(mime.and_then(Value::as_str).unwrap_or_default(), None)
        });
        if format_id == "hls" || ext == "m3u8" {
            let manifest = Url::parse(&src_url)?;
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                video_id,
                Some(format_id),
                true,
                false,
            ) {
                for mut format in stream.formats {
                    if format.ext.is_empty() || format.ext == "unknown_video" {
                        format.ext = "mp4".into();
                    }
                    formats.push(format);
                }
                super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else if let Some((_, (width, height))) =
            QUALITIES.iter().find(|(label, _)| *label == format_id)
        {
            if src_url.starts_with("/mp4:protected/") {
                // Upstream's TODO: these protected files are skipped.
                continue;
            }
            formats.push(dict_format(
                vec![
                    ("url", src_url.into()),
                    ("ext", ext.into()),
                    ("format_id", format_id.into()),
                    ("width", (*width).into()),
                    ("height", (*height).into()),
                ],
                NAME,
                video_id,
            )?);
        }
    }
    Ok((formats, subtitles))
}

impl Extractor for Teamcoco {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].replace('/', "_");
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let next = search_nextjs_data(&webpage, NAME, &display_id)?;
        let page_data = next
            .get("props")
            .and_then(|props| props.get("pageProps"))
            .and_then(|page| page.get("pageData"))
            .and_then(Value::as_object)
            .ok_or_else(|| super::clyp::key_error(&display_id, "props"))?;
        let blocks = page_data
            .get("blocks")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        let props: Vec<&Map<String, Value>> = blocks
            .iter()
            .filter_map(Value::as_object)
            .filter(|block| {
                matches!(
                    block.get("name").and_then(Value::as_str),
                    Some("meta-tags" | "video-player" | "video-info")
                )
            })
            .filter_map(|block| block.get("props").and_then(Value::as_object))
            .collect();
        let info = merge_dicts(props);

        let thumbnail = thumbnail_url(&info);
        let video_id = thumbnail
            .as_ref()
            .and_then(query_id)
            .unwrap_or_else(|| display_id.clone());
        let (formats, subtitles) = formats_and_subtitles(client, &info, &video_id)?;

        let title = info.get("title").filter(|value| !value.is_null()).cloned();
        let description = ["descriptionHtml", "description"]
            .iter()
            .filter_map(|key| info.get(*key)?.as_str())
            .map(crate::html::clean_html)
            .next();
        let timestamp = match info.get("publishedOn").and_then(Value::as_str) {
            Some(published) => {
                crate::dates::unified_timestamp(&format!("{published} 12:00AM"), false)
            }
            None => None,
        };

        let mut fields: Vec<(&str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("display_id", display_id.clone().into()),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            (
                "thumbnail",
                thumbnail.map_or(Value::Null, |url| url.to_string().into()),
            ),
        ];
        if let Some(title) = title {
            fields.push(("title", title));
        }
        if let Some(description) = description {
            fields.push(("description", description.into()));
        }
        if let Some(timestamp) = timestamp {
            fields.push(("timestamp", timestamp.into()));
        }
        let mut result = dict_info(url, (NAME, NAME), fields, formats);
        result.subtitles = subtitles;
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("http://teamcoco.com/video/mary-kay-remote"));
        assert!(suitable(
            "http://teamcoco.com/video/full-episode-mon-6-1-joel-mchale?playlist=x;eyJ0eXBlIjoidGFnIiwiaWQiOjl9"
        ));
        assert!(suitable("http://teamcoco.com/italy/conan-jordan-schlansky"));
        assert!(!suitable("https://example.com/video/mary-kay-remote"));
    }

    #[test]
    fn thumbnail_id_comes_from_the_image_query() {
        let info = json!({"image": "/image/thumb?id=80187"});
        let thumbnail = thumbnail_url(info.as_object().unwrap()).unwrap();
        assert_eq!(
            thumbnail.as_str(),
            "https://teamcoco.com/image/thumb?id=80187"
        );
        assert_eq!(query_id(&thumbnail).as_deref(), Some("80187"));
    }
}
