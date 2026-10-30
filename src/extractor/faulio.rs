//! Upstream `FaulioIE` (`yt_dlp/extractor/faulio.py`): an on-demand episode
//! or media page of the Faulio-hosted Arabic/Persian channels. The API base
//! comes from the page's Nuxt config; the video's HLS and DASH manifests are
//! read with the page's Referer and Origin. Live channels are not ported.
use super::{
    Extractor, dict_info, download_json, download_json_note, download_webpage,
    extract_m3u8_master_optional_headers_id, extract_mpd_optional_note,
};
use crate::{ExtractResult, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value};
use std::sync::LazyLock;
use url::Url;

pub struct Faulio;

const NAME: &str = "Faulio";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:aloula\.sba\.sa|bahry\.com|maraya\.sba\.net\.ae|sat7plus\.org)/(?:(?:ar|en|fa)/)?(?:episode|media)/(?P<id>[a-zA-Z0-9-]+)",
    )
    .unwrap()
});
static NUXT_CONFIG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.__NUXT__\.config=").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_get_headers`: the page as Referer and its origin.
fn headers_for(url: &Url) -> [(String, String); 2] {
    let origin = format!("{}://{}", url.scheme(), url.host_str().unwrap_or_default());
    [
        ("Referer".into(), url.to_string()),
        ("Origin".into(), origin),
    ]
}

/// `_get_api_base`: `public.TRANSLATIONS_API_URL` of the page's Nuxt config.
fn api_base(client: &Client, url: &Url, video_id: &str) -> Result<String> {
    let webpage = download_webpage(client, url, NAME, video_id)?;
    let config = super::search_json(&NUXT_CONFIG, &webpage, true)
        .flatten()
        .ok_or_else(|| {
            crate::Error::Reported(format!(
                "[{NAME}] {video_id}: Unable to extract config; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })?;
    config
        .get("public")
        .and_then(|public| public.get("TRANSLATIONS_API_URL"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| super::clyp::key_error(video_id, "TRANSLATIONS_API_URL"))
}

/// `traverse_obj(video_info, ('blocks', 0))`'s fields in upstream's order.
fn block_fields(block: &Value) -> Vec<(&'static str, Value)> {
    let text = |key: &str| {
        block
            .get(key)
            .filter(|value| value.is_string())
            .cloned()
            .unwrap_or(Value::Null)
    };
    let int = |value: Option<&Value>| super::streamable::int_or_none(value);
    let mut fields = Vec::new();
    let slug = text("slug");
    fields.push(("display_id", slug));
    fields.push(("title", text("title")));
    fields.push(("episode", text("title")));
    fields.push(("description", text("description")));
    fields.push(("series", text("program_title")));
    fields.push(("season_number", int(block.get("season_number"))));
    fields.push(("episode_number", int(block.get("episode"))));
    fields.push((
        "thumbnail",
        block
            .get("image")
            .and_then(Value::as_str)
            .filter(|url| Url::parse(url).is_ok())
            .map_or(Value::Null, Value::from),
    ));
    fields.push((
        "duration",
        int(block
            .get("duration")
            .and_then(|duration| duration.get("total"))),
    ));
    fields.push(("age_limit", int(block.get("age_rating"))));
    fields.retain(|(_, value)| !value.is_null());
    fields
}

impl Extractor for Faulio {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let api = api_base(client, url, &video_id)?;
        let video_api = Url::parse(&format!("{api}/video/{video_id}"))?;
        let video_info = match download_json_note(
            client,
            &video_api,
            NAME,
            &video_id,
            "Downloading JSON metadata",
        ) {
            Ok(value) => Some(value),
            Err(crate::Error::ExtractorRequest { source, .. }) => {
                crate::screen::warning(&format!(
                    "[{NAME}] {video_id}: Unable to download JSON metadata: {}",
                    crate::upstream_error_text(&source)
                ));
                None
            }
            Err(error) => return Err(error),
        };
        let player_api = Url::parse(&format!("{api}/video/{video_id}/player"))?;
        let player_info = download_json(client, &player_api, NAME, &video_id)?;

        let headers = headers_for(url);
        let header_refs: Vec<(&str, &str)> = headers
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        let protocols = player_info
            .get("settings")
            .and_then(|settings| settings.get("protocols"));
        let hls = protocols.and_then(|p| p.get("hls")).and_then(Value::as_str);
        if let Some(hls) = hls
            && let Ok(manifest) = Url::parse(hls)
            && let Some(stream) = extract_m3u8_master_optional_headers_id(
                client,
                &manifest,
                NAME,
                &video_id,
                Some("hls"),
                &header_refs,
            )
        {
            formats.extend(stream.formats);
            super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
        }
        let dash = protocols
            .and_then(|p| p.get("dash"))
            .and_then(Value::as_str);
        if let Some(dash) = dash
            && let Ok(manifest) = Url::parse(dash)
            && let Some(stream) = extract_mpd_optional_note(
                client,
                &manifest,
                NAME,
                &video_id,
                Some("dash"),
                "Downloading MPD manifest",
            )
        {
            formats.extend(stream.formats);
            super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
        }

        let host = Url::parse(&api)?.host_str().unwrap_or_default().to_owned();
        let block = video_info
            .as_ref()
            .and_then(|info| info.get("blocks"))
            .and_then(Value::as_array)
            .and_then(|blocks| blocks.first())
            .cloned()
            .unwrap_or(Value::Null);
        let mut fields: Vec<(&str, Value)> =
            vec![("id", Value::from(format!("{host}_{video_id}")))];
        fields.extend(block_fields(&block));
        fields.push(("formats", Value::Null));
        fields.push(("subtitles", Value::Null));
        fields.push((
            "http_headers",
            Value::Object(
                headers
                    .iter()
                    .map(|(k, v)| (k.clone(), Value::from(v.as_str())))
                    .collect::<Map<String, Value>>(),
            ),
        ));
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(ExtractResult::Video(info)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://aloula.sba.sa/en/episode/29102"));
        assert!(suitable("https://bahry.com/en/media/1191"));
        assert!(suitable("https://maraya.sba.net.ae/episode/127735"));
        assert!(suitable("https://sat7plus.org/episode/18165"));
        assert!(suitable("https://aloula.sba.sa/episode/0"));
        assert!(!suitable("https://aloula.sba.sa/live/saudiatv"));
        assert!(!suitable("https://example.com/en/episode/1"));
    }
}
