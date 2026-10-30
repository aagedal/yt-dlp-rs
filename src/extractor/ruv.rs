//! Upstream `RuvIE` and `RuvSpilaIE` (`yt_dlp/extractor/ruv.py`): RÚV
//! Sarpurinn pages (the page's `video.src` HLS, MP3 or direct URL with its
//! OpenGraph metadata) and the RÚV Spila player's GraphQL episode (HLS or
//! direct file, VTT subtitles, clips as chapters).
use super::clyp::key_error;
use super::{Extractor, dict_format, dict_info, download_json, download_webpage, search_warning};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Ruv;
pub struct RuvSpila;

const NAME: &str = "Ruv";
const SPILA_NAME: &str = "ruv.is:spila";
const SPILA_KEY: &str = "RuvSpila";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?ruv\.is/(?:sarpurinn/[^/]+|node)/(?P<id>[^/]+(?:/\d+)?)",
    )
    .unwrap()
});
static SPILA_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?ruv\.is/(?:(?:sjon|ut)varp|(?:krakka|ung)ruv)/spila/.+/(?P<series_id>[0-9]+)/(?P<id>[a-z0-9]+)",
    )
    .unwrap()
});
/// `video\.FIELD\s*=\s*(["\'])(?P<url>(?:(?!\1).)+)\1` as one alternative
/// per quote; `quote` is the first group, which `_search_regex` returns
/// without a `group`.
fn field_regex(field: &str) -> Regex {
    Regex::new(&format!(
        r#"video\.{field}\s*=\s*(?:(?P<quote_a>")(?P<a>[^"\n]+)"|(?P<quote_b>')(?P<b>[^'\n]+)')"#
    ))
    .unwrap()
}
static SRC: LazyLock<Regex> = LazyLock::new(|| field_regex("src"));
static POSTER: LazyLock<Regex> = LazyLock::new(|| field_regex("poster"));
static NODE_ID: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"<link\b[^>]+\bhref=["']https?://www\.ruv\.is/node/(\d+)"#).unwrap()
});

/// `RuvIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `RuvSpilaIE._match_valid_url`.
pub fn spila_suitable(url: &str) -> bool {
    SPILA_URL.is_match(url)
}

fn announce(name: &'static str, url: &Url) {
    crate::screen::set_extractor(name);
    crate::screen::screen(&format!(
        "[{name}] Extracting URL: {}",
        crate::screen::extracting_url(url.as_str())
    ));
}

/// A Python exception escaping `_real_extract` unwrapped (`ERROR: TEXT`).
fn python_error(text: &str) -> Error {
    Error::Reported(text.to_owned())
}

impl Extractor for Ruv {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        announce(NAME, url);
        let webpage = download_webpage(client, url, NAME, &display_id)?;
        let unable = |what: &str| {
            Error::Reported(format!(
                "[{NAME}] {display_id}: Unable to extract {what}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        };
        let title = crate::html::og_search_property(&webpage, "title")
            .ok_or_else(|| unable("OpenGraph title"))?;
        // `_html_search_regex(..., group='url')`.
        let media_url = SRC
            .captures(&webpage)
            .and_then(|found| found.name("a").or_else(|| found.name("b")))
            .map(|found| crate::html::clean_html(found.as_str()))
            .ok_or_else(|| unable("video URL"))?;
        let video_id = NODE_ID
            .captures(&webpage)
            .map_or_else(|| display_id.clone(), |found| found[1].to_owned());

        let formats = match super::direct::determine_ext(&media_url).as_deref() {
            Some("m3u8") => {
                super::extract_m3u8_fatal_master(
                    client,
                    &Url::parse(&media_url)?,
                    NAME,
                    &video_id,
                    Some("hls"),
                    false,
                )?
                .formats
            }
            Some("mp3") => vec![dict_format(
                vec![
                    ("format_id", "mp3".into()),
                    ("url", media_url.as_str().into()),
                    ("vcodec", "none".into()),
                ],
                NAME,
                &video_id,
            )?],
            _ => vec![dict_format(
                vec![("url", media_url.as_str().into())],
                NAME,
                &video_id,
            )?],
        };

        let description = crate::html::og_search_property(&webpage, "description");
        // `_og_search_thumbnail(default=None) or _search_regex(poster,
        // fatal=False)`: the regex's first group, the opening quote.
        let thumbnail = crate::html::og_search_property(&webpage, "image")
            .filter(|thumbnail| !thumbnail.is_empty())
            .or_else(|| {
                let quote = POSTER
                    .captures(&webpage)
                    .and_then(|found| found.name("quote_a").or_else(|| found.name("quote_b")))
                    .map(|found| found.as_str().to_owned());
                if quote.is_none() {
                    search_warning(NAME, "thumbnail");
                }
                quote
            });
        let published = crate::html::html_search_meta(&webpage, "article:published_time");
        if published.is_none() {
            search_warning(NAME, "timestamp");
        }
        let timestamp = published.and_then(|date| crate::dates::unified_timestamp(&date, true));

        let optional = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let fields = vec![
            ("id", Value::from(video_id.as_str())),
            ("display_id", display_id.as_str().into()),
            ("title", title.into()),
            ("description", optional(description)),
            ("thumbnail", optional(thumbnail)),
            ("timestamp", timestamp.map_or(Value::Null, Value::from)),
            ("formats", Value::Null),
        ];
        Ok(Some(dict_info(url, (NAME, NAME), fields, formats)))
    }
}

/// The GraphQL query, formatted like upstream's triple-quoted string.
fn spila_query(series_id: &str, display_id: &str) -> String {
    format!(
        r#"{{
                Program(id: {series_id}){{
                    title image description short_description
                    episodes(id: {{value: "{display_id}"}}) {{
                        rating title duration file image firstrun description
                        clips {{
                            time text
                        }}
                        subtitles {{
                            name value
                        }}
                    }}
                }}
            }}"#
    )
}

/// Python truthiness of a JSON value.
fn truthy(value: Option<&Value>) -> bool {
    super::lnk::truthy(value)
}

impl Extractor for RuvSpila {
    fn key(&self) -> &'static str {
        SPILA_KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = SPILA_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["id"].to_owned();
        let series_id = found["series_id"].to_owned();
        announce(SPILA_NAME, url);
        let api = Url::parse_with_params(
            "https://www.ruv.is/gql/",
            [("query", spila_query(&series_id, &display_id))],
        )?;
        let response = download_json(client, &api, SPILA_NAME, &display_id)?;
        let not_subscriptable = || python_error("'NoneType' object is not subscriptable");
        let program = response
            .get("data")
            .ok_or_else(|| key_error(&display_id, "data"))?;
        if program.is_null() {
            return Err(not_subscriptable());
        }
        let program = program
            .get("Program")
            .ok_or_else(|| key_error(&display_id, "Program"))?;
        if program.is_null() {
            return Err(not_subscriptable());
        }
        let episodes = program
            .get("episodes")
            .ok_or_else(|| key_error(&display_id, "episodes"))?;
        let episode = match episodes {
            Value::Array(episodes) => episodes
                .first()
                .ok_or_else(|| python_error("list index out of range"))?,
            Value::Null => return Err(not_subscriptable()),
            _ => {
                return Err(Error::UnsupportedHtml5(
                    "RÚV Spila episodes that are not a list are not supported natively".into(),
                ));
            }
        };
        if !episode.is_object() {
            return Err(Error::UnsupportedHtml5(
                "RÚV Spila episodes that are not objects are not supported natively".into(),
            ));
        }

        let mut subtitles = Subtitles::default();
        match episode.get("subtitles") {
            Some(Value::Array(tracks)) => {
                for track in tracks {
                    let name = track.get("name");
                    let value = track.get("value");
                    if !truthy(name) || !truthy(value) {
                        continue;
                    }
                    let (Some(name), Some(value)) =
                        (name.and_then(Value::as_str), value.and_then(Value::as_str))
                    else {
                        return Err(Error::UnsupportedHtml5(
                            "RÚV Spila subtitles that are not strings are not supported natively"
                                .into(),
                        ));
                    };
                    let track: crate::SubtitleFormat = serde_json::from_value(
                        serde_json::json!({"url": Url::parse(value)?, "ext": "vtt"}),
                    )
                    .map_err(|error| Error::Reported(error.to_string()))?;
                    match subtitles
                        .entries
                        .iter_mut()
                        .find(|(language, _)| language.as_deref() == Some(name))
                    {
                        Some((_, tracks)) => tracks.push(track),
                        None => subtitles.entries.push((Some(name.to_owned()), vec![track])),
                    }
                }
            }
            None | Some(Value::Null) => {
                return Err(python_error("'NoneType' object is not iterable"));
            }
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "RÚV Spila subtitles that are not a list are not supported natively".into(),
                ));
            }
        }

        let media_url = episode
            .get("file")
            .ok_or_else(|| key_error(&display_id, "file"))?
            .as_str()
            .ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "RÚV Spila files that are not strings are not supported natively".into(),
                )
            })?;
        let formats = if super::direct::determine_ext(media_url).as_deref() == Some("m3u8") {
            let mut stream = super::extract_m3u8_fatal_master(
                client,
                &Url::parse(media_url)?,
                SPILA_NAME,
                &display_id,
                None,
                false,
            )?;
            super::hls::without_ext(&mut stream);
            stream.formats
        } else {
            vec![dict_format(
                vec![("url", media_url.into())],
                SPILA_NAME,
                &display_id,
            )?]
        };

        // `[{'start_time': parse_duration(c.get('time')), 'title':
        // c.get('text')} for c in episode.get('clips') or []]`.
        let mut chapters = Vec::new();
        for clip in episode
            .get("clips")
            .filter(|clips| truthy(Some(clips)))
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            // A `None` start time (filled from the previous chapter by
            // processing) is not representable natively.
            let start = clip
                .get("time")
                .and_then(Value::as_str)
                .and_then(crate::parse_duration)
                .and_then(|start| serde_json::to_value(start).ok())
                .ok_or_else(|| {
                    Error::UnsupportedHtml5(
                        "RÚV Spila clips without a parseable time are not supported natively"
                            .into(),
                    )
                })?;
            let mut chapter = Map::new();
            chapter.insert("start_time".into(), start);
            // A `None` title is processing's `<Untitled Chapter N>`.
            if let Some(title) = clip.get("text").filter(|title| !title.is_null()) {
                chapter.insert("title".into(), title.clone());
            }
            chapters.push(Value::Object(chapter));
        }

        // `traverse_obj(program, ('episodes', 0, 'title'), 'title')`.
        let title = [episode.get("title"), program.get("title")]
            .into_iter()
            .flatten()
            .find(|title| !title.is_null())
            .cloned()
            .unwrap_or(Value::Null);
        let description = [
            episode.get("description"),
            program.get("description"),
            program.get("short_description"),
        ]
        .into_iter()
        .find(|description| truthy(*description))
        .flatten()
        .cloned()
        .unwrap_or(Value::Null);
        // `episode.get('image', '').replace('$$IMAGESIZE$$', '1960') or None`.
        let thumbnail = match episode.get("image") {
            None => Value::Null,
            Some(Value::String(image)) => {
                let image = image.replace("$$IMAGESIZE$$", "1960");
                if image.is_empty() {
                    Value::Null
                } else {
                    image.into()
                }
            }
            Some(Value::Null) => {
                return Err(python_error("'NoneType' object has no attribute 'replace'"));
            }
            Some(_) => {
                return Err(Error::UnsupportedHtml5(
                    "RÚV Spila images that are not strings are not supported natively".into(),
                ));
            }
        };
        let timestamp = episode
            .get("firstrun")
            .and_then(Value::as_str)
            .and_then(|date| crate::dates::unified_timestamp(date, true))
            .map_or(Value::Null, Value::from);

        let fields = vec![
            ("id", Value::from(display_id.as_str())),
            ("title", title),
            ("description", description),
            ("subtitles", Value::Null),
            ("thumbnail", thumbnail),
            ("timestamp", timestamp),
            ("formats", Value::Null),
            (
                "age_limit",
                episode.get("rating").cloned().unwrap_or(Value::Null),
            ),
            ("chapters", Value::Array(chapters)),
        ];
        let keys: Vec<&str> = fields.iter().map(|(key, _)| *key).collect();
        let shape = format!(
            "dict;{};subtitle=url,ext;chapter!=start_time,title",
            keys.join(",")
        );
        let info = dict_info(url, (SPILA_NAME, SPILA_KEY), fields, formats);
        let mut info = crate::info_order::mark_info(info, &shape);
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
            "http://ruv.is/sarpurinn/ruv-aukaras/fh-valur/20170516"
        ));
        assert!(suitable("http://www.ruv.is/node/1151854"));
        assert!(suitable(
            "http://ruv.is/sarpurinn/klippa/secret-soltice-hefst-a-morgun"
        ));
        assert!(!suitable(
            "https://www.ruv.is/sjonvarp/spila/ithrottir/30657/9jcnd4"
        ));
        assert!(spila_suitable(
            "https://www.ruv.is/sjonvarp/spila/ithrottir/30657/9jcnd4"
        ));
        assert!(spila_suitable(
            "https://www.ruv.is/krakkaruv/spila/krakkafrettir/30712/9jbgb0"
        ));
        assert!(!spila_suitable("https://www.ruv.is/sjonvarp/spila/30657"));
    }
}
