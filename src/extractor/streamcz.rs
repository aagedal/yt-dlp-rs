//! Upstream `StreamCZIE` (`yt_dlp/extractor/streamcz.py`): Stream.cz and
//! Televize Seznam episodes from the GraphQL `episode` (its `spl` playlist,
//! following a `Location` redirect), the playlist's TS and MP4 qualities
//! with their codecs, and its subtitles.
use super::{Extractor, dict_format, dict_info, download_json_note};
use crate::{Error, Result, SubtitleFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct StreamCz;

const NAME: &str = "StreamCZ";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?:stream|televizeseznam)\.cz/[^?#]+/(?P<display_id>[^?#]+)-(?P<id>[0-9]+)",
    )
    .unwrap()
});

const QUERY: &str = "
                    query LoadEpisode($urlName : String){ episode(urlName: $urlName){ ...VideoDetailFragmentOnEpisode } }
                    fragment VideoDetailFragmentOnEpisode on Episode {
                        id
                        spl
                        urlName
                        name
                        perex
                        duration
                        views
                    }";

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Stream.cz {what} are not supported natively"))
}

/// `float_or_none(value, scale=...)`.
fn float_or_none(value: Option<&Value>, scale: f64) -> Option<f64> {
    let value = match value? {
        Value::Number(number) => number.as_f64()?,
        Value::String(text) => text.trim().parse().ok()?,
        Value::Bool(value) => f64::from(u8::from(*value)),
        _ => return None,
    };
    Some(value / scale)
}

/// `urljoin(base, path)` of a string path.
fn join(base: &str, path: &Value) -> Option<String> {
    super::videopress::urljoin(Some(base), path.as_str()?)
}

/// The GraphQL POST (`data=json.dumps(...)`).
fn graphql(client: &Client, video_id: &str) -> Result<Value> {
    use std::io::Read;
    let url = Url::parse("https://www.televizeseznam.cz/api/graphql")?;
    crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading GraphQL result"));
    let body = crate::output::python_json_dumps(&json!({
        "variables": {"urlName": video_id},
        "query": QUERY,
    }));
    let mut response = super::direct::post(
        client,
        &url,
        &[("Content-Type", "application/json;charset=UTF-8")],
        body,
    )
    .map_err(|error| match error {
        Error::Http(_) | Error::HttpRetryAfter { .. } => Error::ExtractorRequest {
            extractor: NAME,
            id: video_id.to_owned(),
            what: "JSON metadata",
            source: Box::new(error),
        },
        error => error,
    })?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{NAME}] {video_id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_extract_formats`.
fn formats(spl_url: &str, video: &Value, video_id: &str) -> Result<Vec<crate::VideoFormat>> {
    let mut formats = Vec::new();
    let empty = Map::new();
    let groups = [
        (
            "ts",
            -1,
            video
                .pointer("/http_stream/qualities")
                .and_then(Value::as_object)
                .unwrap_or(&empty),
        ),
        (
            "mp4",
            1,
            video
                .get("mp4")
                .and_then(Value::as_object)
                .unwrap_or(&empty),
        ),
    ];
    for (ext, pref, streams) in groups {
        for (format_id, stream) in streams {
            let Some(stream_url) = stream.get("url").filter(|url| super::nuxt::truthy(url)) else {
                continue;
            };
            let url = join(spl_url, stream_url)
                .ok_or_else(|| unsupported("stream URLs that are not strings"))?;
            let resolution = stream.get("resolution");
            let height = resolution
                .and_then(|resolution| resolution.get(1))
                .filter(|height| super::nuxt::truthy(height))
                .cloned()
                .or_else(|| {
                    format_id
                        .replace('p', "")
                        .trim()
                        .parse::<i64>()
                        .ok()
                        .map(Value::from)
                });
            let mut fields = vec![
                ("format_id", Value::from(format!("{format_id}-{ext}"))),
                ("ext", ext.into()),
                ("source_preference", pref.into()),
                ("url", url.into()),
                (
                    "tbr",
                    float_or_none(stream.get("bandwidth"), 1000.0)
                        .map_or(Value::Null, |tbr| json!(tbr)),
                ),
                (
                    "duration",
                    float_or_none(stream.get("duration"), 1000.0)
                        .map_or(Value::Null, |duration| json!(duration)),
                ),
                (
                    "width",
                    resolution
                        .and_then(|resolution| resolution.get(0))
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                ("height", height.unwrap_or(Value::Null)),
            ];
            let codec = match stream.get("codec") {
                None | Some(Value::Null) => None,
                Some(Value::String(codec)) => Some(codec.as_str()),
                Some(_) => return Err(unsupported("codecs that are not strings")),
            };
            if let Some(codecs) = crate::codecs::parse(codec) {
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
            formats.push(dict_format(fields, NAME, video_id)?);
        }
    }
    Ok(formats)
}

impl Extractor for StreamCz {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let display_id = found["display_id"].to_owned();
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let response = graphql(client, &video_id)?;
        let data = response
            .get("data")
            .ok_or_else(|| key_error(&video_id, "data"))?
            .get("episode")
            .ok_or_else(|| key_error(&video_id, "episode"))?;
        let spl = match data.get("spl") {
            Some(Value::String(spl)) => spl.clone(),
            Some(_) => return Err(unsupported("playlist URLs that are not strings")),
            None => return Err(key_error(&video_id, "spl")),
        };
        let mut spl_url = format!("{spl}spl2,3");
        // Upstream's request escaping (`escape_url`) sends `|` as `%7C`.
        let mut metadata = download_json_note(
            client,
            &Url::parse(&spl_url.replace('|', "%7C"))
                .map_err(|_| unsupported("playlist URLs that are not URLs"))?,
            NAME,
            &video_id,
            "Downloading playlist",
        )?;
        if metadata.get("Location").is_some() && metadata.get("data").is_none() {
            spl_url = metadata["Location"]
                .as_str()
                .ok_or_else(|| unsupported("redirects that are not strings"))?
                .to_owned();
            metadata = download_json_note(
                client,
                &Url::parse(&spl_url.replace('|', "%7C"))
                    .map_err(|_| unsupported("redirects that are not URLs"))?,
                NAME,
                &video_id,
                "Downloading redirected playlist",
            )?;
        }
        let video = metadata
            .get("data")
            .ok_or_else(|| key_error(&video_id, "data"))?;
        let mut subtitles = crate::Subtitles::default();
        if let Some(Value::Object(groups)) = video.get("subtitles") {
            for subs in groups.values() {
                let Some(language) = subs
                    .get("language")
                    .filter(|value| super::nuxt::truthy(value))
                else {
                    continue;
                };
                let language = language
                    .as_str()
                    .ok_or_else(|| unsupported("subtitle languages that are not strings"))?
                    .to_owned();
                let Some(Value::Object(urls)) = subs.get("urls") else {
                    return Err(unsupported("subtitle URL maps that are not objects"));
                };
                for (ext, sub_url) in urls {
                    let sub_url = join(&spl_url, sub_url)
                        .and_then(|url| Url::parse(&url).ok())
                        .ok_or_else(|| unsupported("subtitle URLs that are not URLs"))?;
                    let track = SubtitleFormat {
                        name: None,
                        url: sub_url,
                        ext: ext.clone(),
                        protocol: None,
                        fragments: Vec::new(),
                        extra: Map::new(),
                    };
                    let language = Some(language.clone());
                    match subtitles
                        .entries
                        .iter_mut()
                        .find(|(key, _)| *key == language)
                    {
                        Some((_, tracks)) => tracks.push(track),
                        None => subtitles.entries.push((language, vec![track])),
                    }
                }
            }
        }
        let formats = formats(&spl_url, video, &video_id)?;
        let get = |key: &str| data.get(key).cloned().unwrap_or(Value::Null);
        let views = match data.get("views") {
            Some(Value::Number(number)) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
            Some(Value::String(text)) => text.trim().parse().ok(),
            _ => None,
        };
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("display_id", display_id.into()),
                ("title", get("name")),
                ("description", get("perex")),
                (
                    "duration",
                    float_or_none(data.get("duration"), 1.0)
                        .map_or(Value::Null, |duration| json!(duration)),
                ),
                ("view_count", views.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
        info.subtitles = subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";subtitle=ext,url");
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
            "https://www.televizeseznam.cz/video/lajna/buh-57953890"
        ));
        assert!(suitable(
            "https://www.stream.cz/kdo-to-mluvi/kdo-to-mluvi-velke-odhaleni-prinasi-novy-porad-uz-od-25-srpna-64087937"
        ));
        assert!(!suitable("https://www.stream.cz/porady"));
    }
}
