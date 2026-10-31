//! Upstream `SBSIE` (`yt_dlp/extractor/sbs.py`): SBS On Demand and SBS News
//! videos from the player API's SMIL document (fatal; HLS, DASH and HTTP
//! sources with subtitles, forced `_fe.vtt` tracks renamed `LANG-forced`),
//! the HEAD geo-restriction check when it has no formats, and the non-fatal
//! `video_stream` JSON merged with the MPX catalogue JSON's metadata.
use super::{Extractor, dict_info, download_json, update_url_query};
use crate::{Error, Result, Subtitles, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Sbs;

pub(crate) const NAME: &str = "SBS";
pub(crate) const KEY: &str = "SBS";

const PLAYER_API: &str = "https://www.sbs.com.au/api/v3";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?sbs\.com\.au/(?:ondemand(?:/video/(?:single/)?|/(?:movie|tv-program)/[^/]+/|/(?:tv|news)-series/(?:[^/]+/){3}|.*?\bplay=|/watch/)|news/(?:embeds/)?video/)(?P<id>[0-9]+)",
    )
    .unwrap()
});
/// `_EMBED_REGEX`, including the literal `]` its verbose pattern starts
/// with (`(?x)]`), and its `(["\'])...\1` quotes as alternatives.
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"\](?:<meta\s+property="og:video"\s+content=|<iframe[^>]+?src=)(?:"(?P<dq>https?://(?:www\.)?sbs\.com\.au/ondemand/video/.+?)"|'(?P<sq>https?://(?:www\.)?sbs\.com\.au/ondemand/video/.+?)')"#,
    )
    .unwrap()
});

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `get_temp_id`.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls`: the matched URLs, unescaped and deduplicated.
pub(crate) fn embed_urls(url: &Url, webpage: &str) -> Vec<Url> {
    let mut found: Vec<Url> = Vec::new();
    for captures in EMBED.captures_iter(webpage) {
        let Some(embed) = captures.name("dq").or_else(|| captures.name("sq")) else {
            continue;
        };
        let Ok(embed) = url.join(&crate::html::unescape_html(embed.as_str())) else {
            continue;
        };
        if suitable(embed.as_str()) && !found.contains(&embed) {
            found.push(embed);
        }
    }
    found
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("SBS {what} are not supported natively"))
}

/// `_AUS_TV_PARENTAL_GUIDELINES.get`.
fn parental_guideline(rating: &str) -> Option<i64> {
    Some(match rating {
        "P" | "G" | "PG" => 0,
        "C" => 7,
        "M" => 14,
        "MA15+" | "MAV15+" => 15,
        "R18+" => 18,
        _ => return None,
    })
}

/// `_extract_smil_formats_and_subtitles(url, video_id)`: fatal unless
/// `--ignore-no-formats-error`.
fn smil_formats(client: &Client, url: &Url, id: &str) -> Result<(Vec<VideoFormat>, Subtitles)> {
    use std::io::Read;
    crate::screen::screen(&format!("[{NAME}] {id}: Downloading SMIL file"));
    let mut response = match super::direct::request(client, url, None) {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            if !crate::no_formats::ignore() {
                return Err(Error::ExtractorRequest {
                    extractor: NAME,
                    id: id.to_owned(),
                    what: "SMIL file",
                    source: Box::new(error),
                });
            }
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download SMIL file: {}",
                crate::upstream_error_text(&error)
            ));
            return Ok((Vec::new(), Subtitles::default()));
        }
        Err(error) => return Err(error),
    };
    let final_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    let smil =
        super::smil::parse(&text).ok_or_else(|| unsupported("SMIL files that are not SMIL"))?;
    super::html5::smil_formats_and_subtitles(client, &smil, &final_url, id, NAME)
}

/// `_download_json(url, video_id, fatal=False)`: HTTP errors are warnings.
fn optional_json(client: &Client, url: &Url, id: &str) -> Result<Option<Value>> {
    match download_json(client, url, NAME, id) {
        Ok(value) => Ok(Some(value)),
        Err(Error::ExtractorRequest { source, .. }) => {
            crate::screen::warning(&format!(
                "[{NAME}] {id}: Unable to download JSON metadata: {}",
                crate::upstream_error_text(&source)
            ));
            Ok(None)
        }
        // Upstream warns with the decoder's message instead.
        Err(Error::Reported(message)) if message.contains(": Failed to parse JSON ") => {
            Err(unsupported("metadata responses that are not JSON"))
        }
        Err(error) => Err(error),
    }
}

/// `int_or_none(value)`.
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

/// `traverse_obj(obj, (*path, {str}))`.
fn str_at<'a>(obj: &'a Value, path: &[&str]) -> Option<&'a str> {
    path.iter()
        .try_fold(obj, |obj, key| obj.get(*key))?
        .as_str()
}

/// `duration`'s `({float_or_none}, {parse_duration})` branches.
fn duration(value: Option<&Value>) -> Value {
    match value {
        Some(Value::Number(number)) => number.as_f64().map_or(Value::Null, |value| json!(value)),
        Some(Value::Bool(value)) => json!(f64::from(u8::from(*value))),
        Some(Value::String(text)) => match text.trim().parse::<f64>() {
            Ok(value) => json!(value),
            Err(_) => crate::parse_duration(text).map_or(Value::Null, |value| json!(value)),
        },
        _ => Value::Null,
    }
}

impl Extractor for Sbs {
    fn key(&self) -> &'static str {
        KEY
    }

    /// `_GEO_COUNTRIES = ['AU']`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::countries(&["AU"])
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(video_id) = temp_id(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let smil_url = update_url_query(
            &Url::parse(&format!("{PLAYER_API}/video_smil"))?,
            &[("id", &video_id)],
        );
        let (formats, subtitles) = smil_formats(client, &smil_url, &video_id)?;

        if formats.is_empty() {
            // `_request_webpage(HEADRequest(...), note='Checking
            // geo-restriction', fatal=False, expected_status=403)`.
            crate::screen::screen(&format!("[{NAME}] {video_id}: Checking geo-restriction"));
            let target = Url::parse("https://sbs-vod-prod-01.akamaized.net/")?;
            match super::direct::head_expecting(client, &target, &[], &[403]) {
                Ok(response) => {
                    let geo_blocked = response
                        .headers()
                        .get_all("x-error-reason")
                        .iter()
                        .any(|value| value.as_bytes() == b"geo-blocked");
                    if geo_blocked {
                        // No `video_id`: `get_temp_id`'s URL ID.
                        return Err(crate::geo::restricted(
                            &format!(
                                "[{NAME}] {video_id}: This video is not available from your location due to geo restriction"
                            ),
                            &["AU"],
                        ));
                    }
                }
                Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
                    crate::screen::warning(&format!(
                        "[{NAME}] {video_id}: Unable to download webpage: {}",
                        crate::upstream_error_text(&error)
                    ));
                }
                Err(error) => return Err(error),
            }
            // `raise_no_formats(msg, video_id=video_id)` is not `expected`:
            // an error even with `--ignore-no-formats-error`.
            return Err(Error::Reported(format!(
                "[{NAME}] {video_id}: No formats are available; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }

        let mut stream_url = Url::parse(&format!("{PLAYER_API}/video_stream"))?;
        stream_url
            .query_pairs_mut()
            .append_pair("id", &video_id)
            .append_pair("context", "tv");
        let mut media: Map<String, Value> = match optional_json(client, &stream_url, &video_id)? {
            Some(Value::Object(stream)) => match stream.get("video_object") {
                Some(Value::Object(object)) => object.clone(),
                _ => Map::new(),
            },
            _ => Map::new(),
        };

        let catalogue_url = Url::parse(&format!(
            "https://catalogue.pr.sbsod.com/mpx-media/{video_id}"
        ))?;
        let catalogue = if media.is_empty() {
            Some(download_json(client, &catalogue_url, NAME, &video_id)?)
        } else {
            optional_json(client, &catalogue_url, &video_id)?
        };
        match catalogue {
            Some(Value::Object(catalogue)) => media.extend(catalogue),
            // `media.update(... or {})`.
            None | Some(Value::Null) | Some(Value::Bool(false)) => {}
            Some(Value::Array(items)) if items.is_empty() => {}
            Some(_) => return Err(unsupported("catalogue responses that are not objects")),
        }
        let media = Value::Object(media);

        // For named episodes, use the catalogue's title to set episode, rather
        // than generic 'Episode N'.
        let episode = match media.get("partOfSeries") {
            Some(Value::Object(series)) if !series.is_empty() => {
                media.get("title").and_then(Value::as_str)
            }
            _ => media.get("epName").and_then(Value::as_str),
        };

        // Need to set different language for forced subs or else they have
        // priority over full subs.
        let mut fixed_subtitles = Subtitles::default();
        for (lang, subs) in subtitles.entries {
            for sub in subs {
                let fixed_lang = if sub.url.as_str().to_lowercase().ends_with("_fe.vtt") {
                    lang.as_ref().map(|lang| format!("{lang}-forced"))
                } else {
                    lang.clone()
                };
                match fixed_subtitles
                    .entries
                    .iter_mut()
                    .find(|(key, _)| *key == fixed_lang)
                {
                    Some((_, tracks)) => tracks.push(sub),
                    None => fixed_subtitles.entries.push((fixed_lang, vec![sub])),
                }
            }
        }

        let text = |path: &[&str]| str_at(&media, path).map_or(Value::Null, Value::from);
        let first_text = |paths: &[&[&str]]| {
            paths
                .iter()
                .find_map(|path| str_at(&media, path))
                .map_or(Value::Null, Value::from)
        };
        let int = |key: &str| int_or_none(media.get(key)).map_or(Value::Null, Value::from);
        let timestamp = [&["datePublished"][..], &["publication", "startDate"]]
            .iter()
            .find_map(|path| str_at(&media, path).and_then(crate::dates::parse_iso8601))
            .map_or(Value::Null, Value::from);
        let age_limit = ["classificationID", "contentRating"]
            .iter()
            .find_map(|key| {
                media
                    .get(*key)
                    .and_then(Value::as_str)
                    .and_then(|rating| parental_guideline(&rating.to_uppercase()))
            })
            .map_or(Value::Null, Value::from);
        let is_live = media
            .get("liveStream")
            .filter(|value| value.is_boolean())
            .cloned()
            .unwrap_or(Value::Null);

        // `(('genres', ...), ('taxonomy', ('genre', 'subgenre'), 'name'), {str})`:
        // the branches are paths, so this reads the `genres` value and every
        // value of `media`, then their `taxonomy`, `genre.subgenre` and `name`.
        let mut categories: Vec<Value> = Vec::new();
        let branches = media.get("genres").into_iter().chain(
            media
                .as_object()
                .into_iter()
                .flat_map(|object| object.values()),
        );
        for obj in branches {
            for path in [&["taxonomy"][..], &["genre", "subgenre"], &["name"]] {
                if let Some(name) = str_at(obj, path) {
                    categories.push(name.into());
                }
            }
        }
        let mut tags: Vec<Value> = Vec::new();
        for path in [
            &["consumerAdviceTexts"][..],
            &["sbsSubCertification", "consumerAdvice"],
        ] {
            let items = path.iter().try_fold(&media, |obj, key| obj.get(*key));
            match items {
                Some(Value::Array(items)) => {
                    tags.extend(items.iter().filter(|item| item.is_string()).cloned());
                }
                Some(Value::Object(items)) => {
                    tags.extend(items.values().filter(|item| item.is_string()).cloned());
                }
                _ => {}
            }
        }
        let thumbnail_items: Vec<&Value> = match media.get("thumbnails") {
            Some(Value::Array(items)) => items.iter().collect(),
            Some(Value::Object(items)) => items.values().collect(),
            _ => Vec::new(),
        };
        let thumbnails: Vec<Value> = thumbnail_items
            .into_iter()
            .filter_map(|thumbnail| {
                let url = thumbnail.get("contentUrl")?.as_str()?;
                super::json_ld::url_or_none(url)?;
                let mut entry = Map::new();
                if let Some(name) = thumbnail.get("name").and_then(Value::as_str) {
                    entry.insert("id".into(), name.into());
                }
                entry.insert("url".into(), url.into());
                for key in ["width", "height"] {
                    if let Some(value) = int_or_none(thumbnail.get(key)) {
                        entry.insert(key.into(), value.into());
                    }
                }
                Some(Value::Object(entry))
            })
            .collect();
        let non_empty = |items: Vec<Value>| {
            if items.is_empty() {
                Value::Null
            } else {
                Value::Array(items)
            }
        };

        let fields = vec![
            ("id", Value::from(video_id.clone())),
            ("title", text(&["name"])),
            ("description", text(&["description"])),
            ("channel", text(&["taxonomy", "channel", "name"])),
            (
                "series",
                first_text(&[&["partOfSeries", "name"], &["seriesTitle"]]),
            ),
            (
                "series_id",
                first_text(&[&["partOfSeries", "uuid"], &["seriesID"]]),
            ),
            ("season_number", int("seasonNumber")),
            ("episode", episode.map_or(Value::Null, Value::from)),
            ("episode_number", int("episodeNumber")),
            ("timestamp", timestamp),
            ("release_year", int("releaseYear")),
            ("duration", duration(media.get("duration"))),
            ("is_live", is_live),
            ("age_limit", age_limit),
            ("categories", non_empty(categories)),
            ("tags", non_empty(tags)),
            ("thumbnails", non_empty(thumbnails)),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
            ("uploader", Value::from("SBSC")),
        ];
        // `**traverse_obj(...)` only sets the keys it finds.
        let fields = fields
            .into_iter()
            .filter(|(key, value)| matches!(*key, "formats" | "subtitles") || !value.is_null())
            .collect();
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=id,url,width,height");
        }
        info.subtitles = fixed_subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for (url, id) in [
            (
                "http://www.sbs.com.au/ondemand/video/single/320403011771/?source=drupal&vertical=thefeed",
                "320403011771",
            ),
            (
                "http://www.sbs.com.au/ondemand/video/320403011771/Dingo-Conservation-The-Feed",
                "320403011771",
            ),
            (
                "http://www.sbs.com.au/news/video/471395907773/The-Feed-July-9",
                "471395907773",
            ),
            (
                "https://www.sbs.com.au/ondemand/?play=1836638787723",
                "1836638787723",
            ),
            (
                "https://www.sbs.com.au/ondemand/program/inside-windsor-castle?play=1283505731842",
                "1283505731842",
            ),
            (
                "https://www.sbs.com.au/news/embeds/video/1840778819866",
                "1840778819866",
            ),
            (
                "https://www.sbs.com.au/ondemand/watch/1698704451971",
                "1698704451971",
            ),
            (
                "https://www.sbs.com.au/ondemand/movie/coherence/1469404227931",
                "1469404227931",
            ),
            (
                "https://www.sbs.com.au/ondemand/news-series/dateline/dateline-2022/dateline-s2022-ep26/2072245827515",
                "2072245827515",
            ),
            (
                "https://www.sbs.com.au/ondemand/tv-series/the-handmaids-tale/season-5/the-handmaids-tale-s5-ep1/2065631811776",
                "2065631811776",
            ),
            (
                "https://www.sbs.com.au/ondemand/tv-program/autun-romes-forgotten-sister/2116212803602",
                "2116212803602",
            ),
        ] {
            assert_eq!(temp_id(url).as_deref(), Some(id), "{url}");
        }
        assert!(!suitable("https://www.sbs.com.au/ondemand/"));
    }

    #[test]
    fn embeds_need_the_upstream_patterns_leading_bracket() {
        let page = Url::parse("https://example.com/").unwrap();
        let html = r#"<iframe src="https://www.sbs.com.au/ondemand/video/1/x"></iframe>]<iframe width="1" src='https://www.sbs.com.au/ondemand/video/320403011771/x'></iframe>"#;
        let urls = embed_urls(&page, html);
        assert_eq!(urls.len(), 1);
        assert_eq!(
            urls[0].as_str(),
            "https://www.sbs.com.au/ondemand/video/320403011771/x"
        );
    }

    #[test]
    fn durations_try_float_then_parse_duration() {
        assert_eq!(duration(Some(&json!(308))), json!(308.0));
        assert_eq!(duration(Some(&json!("308"))), json!(308.0));
        assert_eq!(duration(Some(&json!("PT5M8S"))), json!(308));
        assert_eq!(duration(None), Value::Null);
    }
}
