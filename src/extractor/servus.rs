//! Upstream `ServusIE` (`yt_dlp/extractor/servus.py`): ServusTV, Servus and
//! P.M. Wissen videos from the Red Bull `servus-tv-playnet` player API (HLS,
//! playability errors) overlaid with the page's Next.js title, date,
//! duration, descriptions and categories.
use super::{
    Extractor, dict_info, download_json_note, download_webpage, extract_m3u8_fatal_master,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Servus;

const NAME: &str = "Servus";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?(?:servus\.com/(?:(?:at|de)/p/[^/]+|tv/videos)|(?:servustv|pm-wissen)\.com/(?:[^/]+/)?v(?:ideos)?)/(?P<id>[aA]{2}-?\w+|\d+-\d+)",
    )
    .unwrap()
});
static SEASON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Season (\d+)").unwrap());
static EPISODE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Episode (\d+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Servus {what} are not supported natively"))
}

fn truthy(value: Option<&Value>) -> bool {
    value.is_some_and(super::nuxt::truthy)
}

/// `float_or_none(value)`.
fn float_or_none(value: Option<&Value>) -> Option<f64> {
    match value? {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().parse().ok(),
        Value::Bool(value) => Some(f64::from(u8::from(*value))),
        _ => None,
    }
}

/// `format_field(obj, field, template)` of a `%02d`/`%d`/`%s` template.
fn format_field(value: Option<&Value>, template: &str) -> Result<String> {
    let Some(value) = value.filter(|value| !value.is_null() && value.as_str() != Some("")) else {
        return Ok(String::new());
    };
    let rendered = if template.contains("%02d") || template.contains("%d") {
        let number = match value {
            Value::Number(number) => number
                .as_i64()
                .or_else(|| number.as_f64().map(|value| value.trunc() as i64)),
            Value::Bool(value) => Some(i64::from(*value)),
            _ => None,
        }
        .ok_or_else(|| unsupported("availability values that are not numbers"))?;
        template
            .replace("%02d", &format!("{number:02}"))
            .replace("%d", &number.to_string())
    } else {
        let text = match value {
            Value::String(text) => text.clone(),
            Value::Number(number) => number.to_string(),
            _ => return Err(unsupported("availability values that are not text")),
        };
        template.replace("%s", &text)
    };
    Ok(rendered)
}

/// `_report_errors`.
fn report_errors(video: &Value, video_id: &str) -> Error {
    let errors: Vec<&Value> = match video.get("playabilityErrors") {
        Some(Value::Array(items)) => items.iter().filter(|item| !item.is_null()).collect(),
        Some(Value::Object(items)) => items.values().filter(|item| !item.is_null()).collect(),
        _ => Vec::new(),
    };
    if errors.is_empty() {
        return Error::Reported(format!(
            "[{NAME}] {video_id}: No videoUrl and no information about errors; {}",
            crate::BUG_REPORTS_MESSAGE
        ));
    }
    let has = |name: &str| errors.iter().any(|error| error.as_str() == Some(name));
    let message = if has("FSK_BLOCKED") {
        let details = video
            .pointer("/playabilityErrorDetails/FSK_BLOCKED")
            .filter(|details| details.is_object());
        let field = |key: &str, template: &str| {
            format_field(details.and_then(|details| details.get(key)), template)
        };
        let joined = match (
            field("minEveningHour", " from %02d:00"),
            field("maxMorningHour", " to %02d:00"),
            field("minAge", " (Minimum age %d)"),
        ) {
            (Ok(a), Ok(b), Ok(c)) => format!("{a}{b}{c}"),
            (Err(error), ..) | (_, Err(error), _) | (.., Err(error)) => return error,
        };
        if joined.is_empty() {
            "Blocked by FSK with unknown availability".to_owned()
        } else {
            format!("Only available{joined}")
        }
    } else if has("NOT_YET_AVAILABLE") {
        let available = video
            .pointer("/playabilityErrorDetails/NOT_YET_AVAILABLE/availableFrom")
            .filter(|value| !value.is_null())
            .or_else(|| video.get("currentSunrise"));
        match format_field(available, "Only available from %s") {
            Ok(text) if !text.is_empty() => text,
            Ok(_) => "Video not yet available with unknown availability".to_owned(),
            Err(error) => return error,
        }
    } else {
        let mut names = Vec::new();
        for error in &errors {
            match error.as_str() {
                Some(name) => names.push(name),
                None => return unsupported("playability errors that are not strings"),
            }
        }
        format!("Video unavailable: {}", names.join(", "))
    };
    Error::Reported(format!("[{NAME}] {video_id}: {message}"))
}

/// `_get_description`: the short then long description, `\n\n` collapsed
/// and HTML-unescaped, joined by blank lines.
fn description(next_data: &Value) -> Option<String> {
    let data = next_data.pointer("/props/pageProps/data")?;
    let parts: Vec<String> = ["stv_short_description", "stv_long_description"]
        .iter()
        .filter_map(|key| data.get(*key).and_then(Value::as_str))
        .map(|text| crate::html::unescape_html(&text.replace("\n\n", "\n")))
        .filter(|text| !text.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n\n"))
}

impl Extractor for Servus {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_uppercase();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, NAME, &video_id)?;
        // `_search_nextjs_data(..., fatal=False)`.
        let next_data = match super::search_nextjs_data(&webpage, NAME, &video_id) {
            Ok(data) => data,
            Err(Error::Reported(message))
                if message.contains("Unable to extract next.js data;") =>
            {
                crate::screen::warning(&format!(
                    "[{NAME}] unable to extract next.js data; {}",
                    crate::BUG_REPORTS_MESSAGE
                ));
                Value::Null
            }
            Err(Error::Reported(message)) if message.contains("Failed to parse JSON") => {
                return Err(unsupported("Next.js data that does not parse"));
            }
            Err(error) => return Err(error),
        };
        let mut api = Url::parse("https://api-player.redbull.com/stv/servus-tv-playnet")?;
        api.query_pairs_mut().append_pair("videoId", &video_id);
        let video = download_json_note(client, &api, NAME, &video_id, "Downloading video JSON")?;
        if !truthy(video.get("videoUrl")) {
            // Errors carry `get_temp_id`'s ID: the match before `upper()`.
            return Err(report_errors(&video, &found["id"]));
        }
        let video_url = video["videoUrl"]
            .as_str()
            .ok_or_else(|| unsupported("video URLs that are not strings"))?;
        let stream = extract_m3u8_fatal_master(
            client,
            &Url::parse(video_url).map_err(|_| unsupported("video URLs that are not URLs"))?,
            NAME,
            &video_id,
            Some("hls"),
            true,
        )?;
        let text = |key: &str| match video.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text.clone())),
            Some(_) => Err(unsupported(&format!("{key} values that are not strings"))),
        };
        let season = text("season")?;
        let season_number = season
            .as_deref()
            .and_then(|season| SEASON.captures(season))
            .and_then(|found| found[1].parse::<i64>().ok());
        let episode = text("chapter")?;
        let episode_number = episode
            .as_deref()
            .and_then(|episode| EPISODE.captures(episode))
            .and_then(|found| found[1].parse::<i64>().ok());
        let get = |key: &str| video.get(key).cloned().unwrap_or(Value::Null);
        let opt = |value: Option<String>| value.map_or(Value::Null, Value::from);
        let description = description(&next_data)
            .map(Value::from)
            .filter(super::nuxt::truthy)
            .unwrap_or_else(|| get("description"));
        let mut fields = vec![
            ("id", Value::from(video_id.clone())),
            ("title", get("title")),
            ("description", description),
            ("thumbnail", get("poster")),
            (
                "duration",
                float_or_none(video.get("duration")).map_or(Value::Null, |value| json!(value)),
            ),
            (
                "timestamp",
                video
                    .get("currentSunrise")
                    .and_then(Value::as_str)
                    .and_then(|date| crate::dates::unified_timestamp(date, false))
                    .map_or(Value::Null, Value::from),
            ),
            ("series", get("label")),
            ("season", opt(season)),
            (
                "season_number",
                season_number.map_or(Value::Null, Value::from),
            ),
            ("episode", opt(episode)),
            (
                "episode_number",
                episode_number.map_or(Value::Null, Value::from),
            ),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        // `**traverse_obj(next_data, ('props', 'pageProps', 'data', {...}))`:
        // present keys keep their position, new ones follow.
        if let Some(data) = next_data.pointer("/props/pageProps/data") {
            let mut set = |key: &'static str, value: Value| match fields
                .iter_mut()
                .find(|(name, _)| *name == key)
            {
                Some((_, old)) => *old = value,
                None => fields.push((key, value)),
            };
            if let Some(title) = data.pointer("/title/rendered").and_then(Value::as_str) {
                set("title", title.into());
            }
            if let Some(timestamp) = data
                .pointer("/stv_date/raw")
                .filter(|value| value.is_i64() || value.is_u64())
            {
                set("timestamp", timestamp.clone());
            }
            if let Some(duration) = float_or_none(data.get("stv_duration")) {
                set("duration", json!(duration));
            }
            if let Some(Value::Array(names)) = data.get("category_names") {
                let names: Vec<Value> = names
                    .iter()
                    .filter(|name| name.is_string())
                    .cloned()
                    .collect();
                if !names.is_empty() {
                    set("categories", Value::Array(names));
                }
            }
        }
        let mut info = dict_info(url, (NAME, NAME), fields, stream.formats);
        info.subtitles = stream.subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for url in [
            "https://www.servustv.com/natur/v/aa-28bycqnh92111/",
            "https://www.servustv.com/natur/v/aansszcx3yi9jmlmhdc1/",
            "https://www.servustv.com/videos/aa-1t6vbu5pw1w12/",
            "https://www.servus.com/de/p/Die-Gr%C3%BCnen-aus-Sicht-des-Volkes/AA-1T6VBU5PW1W12/",
            "https://www.servus.com/at/p/Wie-das-Leben-beginnt/1309984137314-381415152/",
            "https://www.servus.com/tv/videos/1380889096408-1235196658/",
            "https://www.pm-wissen.com/videos/aa-24mus4g2w2112/",
        ] {
            assert!(suitable(url), "{url}");
        }
        assert!(!suitable("https://www.servustv.com/natur/"));
    }

    #[test]
    fn availability_fields_follow_format_field() {
        assert_eq!(
            format_field(Some(&json!(6)), " to %02d:00").unwrap(),
            " to 06:00"
        );
        assert_eq!(format_field(Some(&json!("")), "%s").unwrap(), "");
        assert_eq!(format_field(None, "%s").unwrap(), "");
    }
}
