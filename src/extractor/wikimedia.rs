//! Upstream `WikimediaIE` (`yt_dlp/extractor/wikimedia.py`): a Wikimedia
//! Commons file page's `videoinfo` from the MediaWiki API (requested with
//! upstream's robot-policy User-Agent): the source file, its transcoded
//! derivatives, timed text tracks and the extended metadata.
use super::{Extractor, dict_format, dict_info, download_json_headers};
use crate::{Error, Result, SubtitleFormat, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Wikimedia;

const NAME: &str = "wikimedia.org";
const KEY: &str = "Wikimedia";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://commons\.wikimedia\.org/wiki/File:(?P<id>[^/#?]+)").unwrap()
});
static TYPE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"^(?P<mime>[^;]+)(?:;\s*codecs="(?P<codecs>[^"]+)")?"#).unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Wikimedia {what} are not supported natively"))
}

/// `_HTTP_HEADERS`: the User-Agent of the tracked upstream release
/// (Wikimedia blocks faked browser User-Agents).
fn user_agent() -> String {
    format!(
        "yt-dlp/{} (https://github.com/yt-dlp/yt-dlp)",
        crate::cache::UPSTREAM_VERSION
    )
}

fn http_headers() -> Value {
    Value::Object(Map::from_iter([(
        "User-Agent".to_owned(),
        user_agent().into(),
    )]))
}

/// `mimetype2ext(mime)` (no default: an unknown type's subtype).
fn mimetype2ext(mime: Option<&Value>) -> Option<String> {
    mime.and_then(Value::as_str)
        .map(|mime| super::direct::mimetype2ext_or(mime, None))
}

/// `_parse_ext_and_codecs(type)`.
fn ext_and_codecs(text: Option<&str>) -> Vec<(&'static str, Value)> {
    let Some(found) = text
        .filter(|text| !text.is_empty())
        .and_then(|text| TYPE.captures(text))
    else {
        return Vec::new();
    };
    let mut fields = vec![(
        "ext",
        Value::from(super::direct::mimetype2ext_or(&found["mime"], None)),
    )];
    if let Some(codecs) = crate::codecs::parse(found.name("codecs").map(|codecs| codecs.as_str())) {
        fields.push(("vcodec", codecs.vcodec.into()));
        fields.push(("acodec", codecs.acodec.into()));
        fields.push((
            "dynamic_range",
            codecs.dynamic_range.map_or(Value::Null, Value::from),
        ));
        if let Some(scodec) = codecs.scodec {
            fields.push(("scodec", scodec.into()));
        }
    }
    fields
}

/// `traverse_obj(obj, (key, {int_or_none}))` as an optional field.
fn int(value: Option<&Value>) -> Option<Value> {
    Some(super::streamable::int_or_none(value)).filter(|value| !value.is_null())
}

/// An item of a list (or dict) that has a `src` URL.
fn with_src(value: Option<&Value>) -> Vec<&Value> {
    let items: Vec<&Value> = match value {
        Some(Value::Array(items)) => items.iter().collect(),
        Some(Value::Object(items)) => items.values().collect(),
        _ => Vec::new(),
    };
    items
        .into_iter()
        .filter(|item| {
            item.get("src")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .is_some()
        })
        .collect()
}

impl Extractor for Wikimedia {
    fn key(&self) -> &'static str {
        KEY
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
        // `urllib.parse.unquote` (invalid UTF-8 replaced).
        let title = percent_encoding::percent_decode_str(&display_id).decode_utf8_lossy();
        let query = super::login::urlencode(&[
            ("action", "query"),
            ("format", "json"),
            ("titles", &format!("File:{title}")),
            ("prop", "videoinfo"),
            (
                "viprop",
                "timestamp|user|url|size|derivatives|timedtext|extmetadata",
            ),
        ]);
        let api = Url::parse(&format!("https://commons.wikimedia.org/w/api.php?{query}"))?;
        let agent = user_agent();
        let response =
            download_json_headers(client, &api, NAME, &display_id, &[("User-Agent", &agent)])?;
        // `('query', 'pages', ..., {dict}, any)`.
        let page = match response.get("query").and_then(|query| query.get("pages")) {
            Some(Value::Object(pages)) => pages.values().find(|page| page.is_object()),
            Some(Value::Array(pages)) => pages.iter().find(|page| page.is_object()),
            _ => None,
        };
        let Some(video_info) = page
            .and_then(|page| page.get("videoinfo"))
            .and_then(|info| info.get(0))
            .filter(|info| info.is_object())
        else {
            return Err(Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to extract video info; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        };
        let page = page.expect("video info comes from a page");

        let mut formats = Vec::new();
        if let Some(source) = video_info
            .get("url")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
        {
            let mut fields = vec![
                ("url", Value::from(source)),
                ("format_id", "source".into()),
                ("quality", 10.into()),
            ];
            for (key, source) in [
                ("width", "width"),
                ("height", "height"),
                ("filesize", "size"),
            ] {
                if let Some(value) = int(video_info.get(source)) {
                    fields.push((key, value));
                }
            }
            fields.push(("http_headers", http_headers()));
            formats.push(dict_format(fields, NAME, &display_id)?);
        }
        for derivative in with_src(video_info.get("derivatives")) {
            let mut fields = vec![("url", derivative["src"].clone())];
            for key in ["width", "height"] {
                if let Some(value) = int(derivative.get(key)) {
                    fields.push((key, value));
                }
            }
            // `int_or_none(scale=1000)`.
            let tbr = match super::streamable::int_or_none(derivative.get("bandwidth")) {
                Value::Number(number) => {
                    number.as_i64().map(|bandwidth| bandwidth.div_euclid(1000))
                }
                _ => None,
            };
            if let Some(tbr) = tbr {
                fields.push(("tbr", tbr.into()));
            }
            fields.push(("http_headers", http_headers()));
            let kind = match derivative.get("type") {
                None | Some(Value::Null) => None,
                Some(Value::String(kind)) => Some(kind.as_str()),
                Some(_) => return Err(unsupported("derivative types that are not strings")),
            };
            fields.extend(ext_and_codecs(kind));
            let mut format = dict_format(fields, NAME, &display_id)?;
            if tbr.is_some() {
                crate::info_order::mark_int_rates(&mut format);
            }
            formats.push(format);
        }

        let mut subtitles = Subtitles::default();
        for track in with_src(video_info.get("timedtext")) {
            let language = track
                .get("srclang")
                .filter(|lang| super::nuxt::truthy(lang))
                .map(|lang| {
                    lang.as_str()
                        .map(str::to_owned)
                        .ok_or_else(|| unsupported("subtitle languages that are not strings"))
                })
                .transpose()?
                .unwrap_or_else(|| "unk".into());
            let src = track["src"].as_str().unwrap_or_default();
            let parsed =
                Url::parse(src).map_err(|_| unsupported("subtitle URLs that are not absolute"))?;
            let ext = mimetype2ext(track.get("type"))
                .ok_or_else(|| unsupported("subtitle tracks without a type"))?;
            let track = SubtitleFormat {
                name: None,
                url: parsed,
                ext,
                protocol: None,
                fragments: Vec::new(),
                extra: Map::from_iter([("http_headers".to_owned(), http_headers())]),
            };
            let language = Some(language);
            match subtitles
                .entries
                .iter_mut()
                .find(|(key, _)| *key == language)
            {
                Some((_, tracks)) => tracks.push(track),
                None => subtitles.entries.push((language, vec![track])),
            }
        }

        let id = match page.get("pageid") {
            Some(Value::Number(id)) => id.to_string(),
            Some(Value::String(id)) => id.clone(),
            None => {
                return Err(Error::Reported(format!(
                    "{display_id}: An extractor error has occurred. (caused by KeyError('pageid')); {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
            Some(_) => return Err(unsupported("page IDs that are not numbers")),
        };
        let metadata = |key: &str| {
            video_info
                .get("extmetadata")
                .and_then(|metadata| metadata.get(key))
                .and_then(|entry| entry.get("value"))
                .and_then(Value::as_str)
        };
        let duration = match video_info.get("duration") {
            Some(Value::Number(number)) => number.as_f64().map(Value::from),
            Some(Value::String(text)) => text.trim().parse::<f64>().ok().map(Value::from),
            _ => None,
        };
        // `{lambda x: x.split('|')}, ..., {str.strip}, filter`.
        let categories: Vec<Value> = metadata("Categories")
            .map(|categories| {
                categories
                    .split('|')
                    // Python's `str.strip()` whitespace.
                    .map(|category| {
                        category.trim_matches(|c: char| {
                            c.is_whitespace() || ('\x1c'..='\x1f').contains(&c)
                        })
                    })
                    .filter(|category| !category.is_empty())
                    .map(Value::from)
                    .collect()
            })
            .unwrap_or_default();
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", id.into()),
            ("display_id", display_id.as_str().into()),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        fields.extend(
            [
                ("title", metadata("ObjectName").map(Value::from)),
                (
                    "timestamp",
                    video_info
                        .get("timestamp")
                        .and_then(Value::as_str)
                        .and_then(crate::dates::parse_iso8601)
                        .map(Value::from),
                ),
                (
                    "description",
                    metadata("ImageDescription").map(|text| crate::html::clean_html(text).into()),
                ),
                (
                    "uploader",
                    video_info
                        .get("user")
                        .filter(|user| user.is_string())
                        .cloned(),
                ),
                ("duration", duration),
                ("license", metadata("UsageTerms").map(Value::from)),
                (
                    "categories",
                    (!categories.is_empty()).then_some(Value::Array(categories)),
                ),
            ]
            .into_iter()
            .filter_map(|(key, value)| value.map(|value| (key, value))),
        );
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://commons.wikimedia.org/wiki/File:Die_Temperaturkurve_der_Erde_(ZDF,_Terra_X)_720p_HD_50FPS.webm"
        ));
        assert!(suitable(
            "https://commons.wikimedia.org/wiki/File:Two-toed_sloth_rail_walking_%E4%BA%8C%E8%B6%BE%E6%A8%B9%E7%8D%BA%E7%88%AC%E8%A1%8C_(HD).webm"
        ));
        assert!(!suitable("https://commons.wikimedia.org/wiki/Main_Page"));
    }

    #[test]
    fn derivative_types_follow_parse_ext_and_codecs() {
        assert_eq!(
            ext_and_codecs(Some(r#"video/webm; codecs="vp9, opus""#)),
            vec![
                ("ext", Value::from("webm")),
                ("vcodec", "vp9".into()),
                ("acodec", "opus".into()),
                ("dynamic_range", Value::Null),
            ]
        );
        assert_eq!(
            ext_and_codecs(Some("application/vnd.apple.mpegurl")),
            vec![("ext", Value::from("m3u8"))]
        );
        assert!(ext_and_codecs(Some(";x")).is_empty());
        assert!(ext_and_codecs(None).is_empty());
    }
}
