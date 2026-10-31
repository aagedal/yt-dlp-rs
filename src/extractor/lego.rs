//! Upstream `LEGOIE` (`yt_dlp/extractor/lego.py`): LEGO videos from the
//! Slingshot media player API (geo bypass from the URL's locale, HTTP 451
//! as a geo restriction): HDS, HLS and progressive qualities, the locale's
//! SRT subtitles and the video metadata.
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, SubtitleFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Lego;

const NAME: &str = "LEGO";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?lego\.com/(?P<locale>[a-z]{2}-[a-z]{2})/(?:[^/]+/)*videos/(?:[^/]+/)*[^/?#]+-(?P<id>[0-9a-f]{32})",
    )
    .unwrap()
});

const QUALITIES: [&str; 5] = ["Lowest", "Low", "Medium", "High", "Highest"];
/// `_QUALITIES`: `(abr, height, width)`.
const SIZES: [(i64, i64, i64); 5] = [
    (64, 180, 320),
    (64, 270, 480),
    (96, 360, 640),
    (128, 540, 960),
    (128, 720, 1280),
];

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
    Error::UnsupportedHtml5(format!("LEGO {what} are not supported natively"))
}

/// `str(uuid.UUID(hex))`.
fn uuid(hex: &str) -> String {
    format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    )
}

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

impl Extractor for Lego {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let locale = found["locale"].to_owned();
        let url_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let country = locale
            .split_once('-')
            .map(|(_, country)| country.to_uppercase())
            .unwrap_or_default();
        crate::geo::initialize_countries(&[country.as_str()]);
        let api = super::update_url_query(
            &Url::parse("https://services.slingshot.lego.com/mediaplayer/v2")?,
            &[("videoId", format!("{}_{locale}", uuid(&url_id)).as_str())],
        );
        let item = match download_json(client, &api, NAME, &url_id) {
            Ok(item) => item,
            Err(Error::ExtractorRequest { source, .. })
                if crate::upstream_error_text(&source).starts_with("HTTP Error 451") =>
            {
                return Err(crate::geo::restricted(
                    &format!(
                        "[{NAME}] {url_id}: This video is not available from your location due to geo restriction"
                    ),
                    &[country.as_str()],
                ));
            }
            Err(error) => return Err(error),
        };
        let video = item
            .get("Video")
            .ok_or_else(|| key_error(&url_id, "Video"))?;
        let video_id = match video.get("Id") {
            Some(Value::String(id)) => id.clone(),
            Some(_) => return Err(unsupported("video IDs that are not strings")),
            None => return Err(key_error(&url_id, "Id")),
        };
        let title = video
            .get("Title")
            .cloned()
            .ok_or_else(|| key_error(&url_id, "Title"))?;
        let mut formats = Vec::new();
        let sources = match item.get("VideoFormats") {
            None => Vec::new(),
            Some(Value::Array(sources)) => sources.clone(),
            Some(_) => return Err(unsupported("format lists that are not lists")),
        };
        for source in &sources {
            let Some(source_url) = source
                .get("Url")
                .filter(|value| super::nuxt::truthy(value))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let source_format = source.get("Format").and_then(Value::as_str);
            match source_format {
                Some("F4M") => formats.extend(super::extract_f4m_optional(
                    client,
                    NAME,
                    &Url::parse(source_url)?,
                    &video_id,
                    Some("F4M"),
                )?),
                Some("M3U8") => {
                    if let Some(stream) = super::extract_m3u8_master_optional(
                        client,
                        &Url::parse(source_url)?,
                        NAME,
                        &video_id,
                        Some("M3U8"),
                        false,
                        false,
                    ) {
                        formats.extend(stream.formats);
                    }
                }
                _ => {
                    let quality = source.get("Quality").and_then(Value::as_str);
                    let format_id = [source_format, quality]
                        .into_iter()
                        .flatten()
                        .filter(|part| !part.is_empty())
                        .collect::<Vec<_>>()
                        .join("-");
                    let rank = quality
                        .and_then(|quality| QUALITIES.iter().position(|known| *known == quality));
                    let mut fields = vec![
                        ("format_id", Value::from(format_id)),
                        ("quality", Value::from(rank.map_or(-1, |rank| rank as i64))),
                        ("url", source_url.into()),
                    ];
                    if let Some((abr, height, width)) = rank.map(|rank| SIZES[rank]) {
                        fields.push(("abr", abr.into()));
                        fields.push(("height", height.into()));
                        fields.push(("width", width.into()));
                    }
                    let mut format = dict_format(fields, NAME, &video_id)?;
                    crate::info_order::mark_int_rates(&mut format);
                    formats.push(format);
                }
            }
        }
        let mut subtitles = crate::Subtitles::default();
        let text = |key: &str| {
            video
                .get(key)
                .filter(|value| super::nuxt::truthy(value))
                .and_then(|value| match value {
                    Value::String(text) => Some(text.clone()),
                    Value::Number(number) => Some(number.to_string()),
                    _ => None,
                })
        };
        if let Some(sub_file_id) = text("SubFileId")
            && sub_file_id != "00000000-0000-0000-0000-000000000000"
            && let (Some(path), Some(invariant), Some(file), Some(version)) = (
                text("NetstoragePath"),
                text("InvariantId"),
                text("VideoFileId"),
                text("VideoVersion"),
            )
        {
            let sub_url = Url::parse(&format!(
                "https://lc-mediaplayerns-live-s.legocdn.com/public/{path}/{invariant}_{file}_{locale}_{version}_sub.srt"
            ))?;
            subtitles.entries.push((
                Some(locale[..2].to_owned()),
                vec![SubtitleFormat {
                    name: None,
                    url: sub_url,
                    ext: "srt".into(),
                    protocol: None,
                    fragments: Vec::new(),
                    extra: Map::new(),
                }],
            ));
        }
        let get = |key: &str| video.get(key).cloned().unwrap_or(Value::Null);
        let opt = |value: Option<i64>| value.map_or(Value::Null, Value::from);
        let thumbnail = video
            .get("GeneratedCoverImage")
            .filter(|value| super::nuxt::truthy(value))
            .cloned()
            .unwrap_or_else(|| get("GeneratedThumbnail"));
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.into()),
                ("title", title),
                ("description", get("Description")),
                ("thumbnail", thumbnail),
                ("duration", opt(int_or_none(video.get("Length")))),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
                ("age_limit", opt(int_or_none(video.get("AgeFrom")))),
                ("season", get("SeasonTitle")),
                (
                    "season_number",
                    opt(int_or_none(video.get("Season")).filter(|value| *value != 0)),
                ),
                (
                    "episode_number",
                    opt(int_or_none(video.get("Episode")).filter(|value| *value != 0)),
                ),
            ],
            formats,
        );
        info.subtitles = subtitles;
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";subtitle=url");
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
            "http://www.lego.com/en-us/videos/themes/club/blocumentary-kawaguchi-55492d823b1b4d5e985787fa8c2973b1"
        ));
        assert!(!suitable("https://www.lego.com/en-us/videos"));
        assert_eq!(
            uuid("55492d823b1b4d5e985787fa8c2973b1"),
            "55492d82-3b1b-4d5e-9857-87fa8c2973b1"
        );
    }
}
