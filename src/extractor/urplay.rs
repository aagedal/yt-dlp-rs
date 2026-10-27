//! Upstream `URPlayIE` (`yt_dlp/extractor/urplay.py`): UR Play and UR Skola
//! programs from the page's Next.js `productData` (else the legacy
//! `ProgramContainer` React props' accessible episode), the media API's HLS
//! playlist and the streaming info's subtitle tracks, with the series and
//! episode metadata, image thumbnails and age limit.
use super::{
    Extractor, dict_info, download_json_note, download_webpage, extract_m3u8_fatal_master,
};
use crate::{Error, Result, SubtitleFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct UrPlay;

const NAME: &str = "URPlay";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?ur(?:play|skola)\.se/(?:program|Produkter)/(?P<id>[0-9]+)")
        .unwrap()
});
static REACT_PROPS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"data-react-class="routes/Product/components/ProgramContainer/ProgramContainer"[^>]+data-react-props="(\{.+?\})""#,
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn extractor_error(id: &str, cause: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by {cause}); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("URPlay {what} are not supported natively"))
}

/// `int_or_none(value)` of a JSON number or numeric string.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(i64::from(*value)),
        _ => None,
    }
}

/// `dict_get(d, keys)`: the first truthy value.
fn dict_get<'a>(value: &'a Value, keys: &[&str]) -> Option<&'a Value> {
    keys.iter()
        .filter_map(|key| value.get(*key))
        .find(|value| super::nuxt::truthy(value))
}

/// `parse_lang_code`: a three-letter code, else the two-letter code's.
fn parse_lang_code(code: &str) -> Option<String> {
    use crate::embed_subtitles::languages::short_to_long;
    let lang = code.to_lowercase();
    // `ISO639Utils.long2short(lang)`: whether a short code maps to it.
    let is_long = (b'a'..=b'z').any(|first| {
        (b'a'..=b'z').any(|second| {
            let short = [first, second];
            short_to_long(std::str::from_utf8(&short).unwrap_or_default()) == Some(lang.as_str())
        })
    });
    if is_long {
        return Some(lang).filter(|lang| !lang.is_empty());
    }
    short_to_long(&lang).map(str::to_owned)
}

/// The `__NEXT_DATA__` product data, or the legacy React props' episode.
fn product_data(webpage: &str, video_id: &str) -> Result<Value> {
    match super::search_nextjs_data(webpage, NAME, video_id) {
        Ok(data) => {
            return match data.pointer("/props/pageProps/productData") {
                Some(Value::Object(map)) if !map.is_empty() => Ok(Value::Object(map.clone())),
                _ => Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to parse __NEXT_DATA__; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))),
            };
        }
        Err(Error::Reported(message)) if message.contains("Unable to extract next.js data;") => {
            crate::screen::warning(&format!(
                "[{NAME}] unable to extract next.js data; {}",
                crate::BUG_REPORTS_MESSAGE
            ));
        }
        Err(Error::Reported(message)) if message.contains("Failed to parse JSON") => {
            return Err(unsupported("Next.js data that does not parse"));
        }
        Err(error) => return Err(error),
    }
    let props = REACT_PROPS
        .captures(webpage)
        .map(|found| crate::html::clean_html(&found[1]))
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract urplayer data; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    let props: Value = crate::json::from_str(&props)
        .map_err(|_| unsupported("legacy React props that do not parse"))?;
    let episodes = props
        .get("accessibleEpisodes")
        .ok_or_else(|| extractor_error(video_id, "KeyError('accessibleEpisodes')"))?;
    let wanted = video_id.parse::<i64>().ok();
    episodes
        .as_array()
        .into_iter()
        .flatten()
        .find(|episode| episode.get("id").and_then(Value::as_i64) == wanted && wanted.is_some())
        .cloned()
        .ok_or_else(|| extractor_error(video_id, "StopIteration()"))
}

impl Extractor for UrPlay {
    fn key(&self) -> &'static str {
        NAME
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
        let page = Url::parse(
            &url.as_str()
                .replace("skola.se/Produkter", "play.se/program"),
        )?;
        let webpage = download_webpage(client, &page, NAME, &video_id)?;
        let data = product_data(&webpage, &video_id)?;
        let episode = match data.get("title") {
            Some(Value::String(title)) => title.clone(),
            Some(_) => return Err(unsupported("titles that are not strings")),
            None => return Err(extractor_error(&video_id, "KeyError('title')")),
        };
        let api = Url::parse(&format!(
            "https://media-api.urplay.se/config-streaming/v1/urplay/sources/{video_id}"
        ))?;
        let sources = download_json_note(
            client,
            &api,
            NAME,
            &video_id,
            "Downloading streaming information",
        )?;
        let hls_url = sources
            .pointer("/sources/hls")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{NAME}] {video_id}: Unable to extract HLS URL; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let stream = extract_m3u8_fatal_master(
            client,
            &Url::parse(&hls_url)?,
            NAME,
            &video_id,
            Some("hls"),
            true,
        )?;
        let mut subtitles = stream.subtitles;
        let streaming = match data.get("streamingInfo") {
            Some(Value::Object(streaming)) => streaming,
            Some(_) => return Err(unsupported("streaming info that is not an object")),
            None => return Err(extractor_error(&video_id, "KeyError('streamingInfo')")),
        };
        for stream in streaming.values() {
            let Some(stream) = stream.as_object() else {
                return Err(unsupported("streams that are not objects"));
            };
            for (kind, value) in stream {
                if kind == "sd" || kind == "hd" || !value.is_object() {
                    continue;
                }
                let Some(location) = value
                    .get("location")
                    .filter(|location| super::nuxt::truthy(location))
                else {
                    continue;
                };
                let Some(location) = location.as_str() else {
                    return Err(unsupported("subtitle locations that are not strings"));
                };
                let lang = match value.get("language") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(code)) => parse_lang_code(code),
                    Some(_) => return Err(unsupported("subtitle languages that are not strings")),
                };
                let Some(lang) = lang else {
                    continue;
                };
                let track = SubtitleFormat {
                    name: None,
                    url: Url::parse(location)
                        .map_err(|_| unsupported("subtitle locations that are not URLs"))?,
                    ext: kind.clone(),
                    protocol: None,
                    fragments: Vec::new(),
                    extra: Map::new(),
                };
                let lang = Some(lang);
                match subtitles.entries.iter_mut().find(|(key, _)| *key == lang) {
                    Some((_, tracks)) => tracks.push(track),
                    None => subtitles.entries.push((lang, vec![track])),
                }
            }
        }
        let mut thumbnails = Vec::new();
        if let Some(Value::Object(image)) = data.get("image") {
            for (key, value) in image {
                let mut thumbnail = Map::new();
                thumbnail.insert("id".into(), key.clone().into());
                thumbnail.insert("url".into(), value.clone());
                let size: Vec<&str> = key.split('x').collect();
                if size.len() == 2 {
                    for (name, text) in [("width", size[0]), ("height", size[1])] {
                        thumbnail.insert(
                            name.into(),
                            text.trim().parse::<i64>().map_or(Value::Null, Value::from),
                        );
                    }
                }
                thumbnails.push(Value::Object(thumbnail));
            }
        }
        let series = data
            .get("series")
            .filter(|series| super::nuxt::truthy(series))
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let series_title = dict_get(&series, &["seriesTitle", "title"])
            .or_else(|| dict_get(&data, &["seriesTitle", "mainTitle"]))
            .cloned();
        let title = match &series_title {
            Some(Value::String(series)) => format!("{series} : {episode}"),
            Some(_) => return Err(unsupported("series titles that are not strings")),
            None => episode.clone(),
        };
        // `min(try_get(a, lambda x: x['from'], int) or 0 for a in ageRanges)`.
        let ranges = match data.get("ageRanges") {
            None => Vec::new(),
            Some(Value::Array(ranges)) => ranges.iter().collect(),
            Some(_) => return Err(unsupported("age ranges that are not lists")),
        };
        let age = ranges
            .iter()
            .map(|range| range.get("from").and_then(Value::as_i64).unwrap_or(0))
            .min()
            // An uncaught `ValueError` is reported as its message.
            .ok_or_else(|| Error::Reported("min() iterable argument is empty".into()))?;
        let get = |key: &str| data.get(key).cloned().unwrap_or(Value::Null);
        let fields = vec![
            ("id", Value::from(video_id.clone())),
            ("title", title.into()),
            ("description", get("description")),
            ("thumbnails", Value::Array(thumbnails)),
            (
                "timestamp",
                data.get("publishedAt")
                    .and_then(Value::as_str)
                    .and_then(|date| crate::dates::unified_timestamp(date, false))
                    .map_or(Value::Null, Value::from),
            ),
            ("series", series_title.unwrap_or(Value::Null)),
            ("formats", Value::Null),
            (
                "duration",
                int_or_none(data.get("duration")).map_or(Value::Null, Value::from),
            ),
            ("categories", get("categories")),
            ("tags", get("keywords")),
            (
                "season",
                series.get("label").cloned().unwrap_or(Value::Null),
            ),
            ("episode", episode.into()),
            (
                "episode_number",
                int_or_none(data.get("episodeNumber")).map_or(Value::Null, Value::from),
            ),
            (
                "age_limit",
                super::videopress::parse_age_limit(Some(&Value::from(age))),
            ),
            ("subtitles", Value::Null),
        ];
        let mut info = dict_info(url, (NAME, NAME), fields, stream.formats);
        info.subtitles = subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=id,url,width,height;subtitle=ext,url");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://urplay.se/program/203704-ur-samtiden-livet-universum-och-rymdens-markliga-musik-om-vetenskap-kritiskt-tankande-och-motstand"
        ));
        assert!(suitable(
            "https://urskola.se/Produkter/190031-Tripp-Trapp-Trad-Sovkudde"
        ));
        assert!(!suitable("https://urplay.se/serie/1"));
    }

    #[test]
    fn language_codes_follow_iso639_utils() {
        assert_eq!(parse_lang_code("SV").as_deref(), Some("swe"));
        assert_eq!(parse_lang_code("swe").as_deref(), Some("swe"));
        assert_eq!(parse_lang_code("xx"), None);
    }
}
