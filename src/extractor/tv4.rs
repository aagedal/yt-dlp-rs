//! Upstream `TV4IE` (`yt_dlp/extractor/tv4.py`): tv4.se clips and
//! tv4play.se programmes, films and sport through the `playback2.a2d.tv`
//! asset metadata and play manifest URL, with the manifest's non-fatal HLS,
//! DASH (`.mpd`), HDS (`.f4m`) and Smooth Streaming (`.ism/Manifest`)
//! siblings and the Sweden-only geo error.
use super::{
    Extractor, dict_info, download_json_note, extract_f4m_optional, extract_ism_optional,
    extract_m3u8_master_optional, extract_mpd_optional_note,
};
use crate::{Error, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Tv4;

const NAME: &str = "TV4";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^https?://(?:www\.)?(?:tv4\.se/(?:[^/]+)/klipp/(?:.*)-|tv4play\.se/(?:(?:program|barn)/(?:(?:[^/]+/){1,2}|(?:[^\?]+)\?video_id=)|iframe/video/|film/|sport/|))(?P<id>[0-9]+)",
    )
    .unwrap()
});
static ISM_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\.ism/.*?\.m3u8").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("TV4 {what} are not supported natively"))
}

/// A `KeyError` reported as `ExtractorError(video_id=get_temp_id(url))`.
fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// `_call_api(endpoint, video_id, query=query)`.
fn call_api(client: &Client, endpoint: &str, id: &str, query: &[(&str, &str)]) -> Result<Value> {
    let mut url = Url::parse(&format!("https://playback2.a2d.tv/{endpoint}/{id}"))?;
    // `{'service': 'tv4', 'device': 'browser', 'protocol': 'hls', **query}`.
    let mut pairs = vec![
        ("service", "tv4"),
        ("device", "browser"),
        ("protocol", "hls"),
    ];
    for &(key, value) in query {
        match pairs.iter_mut().find(|(known, _)| *known == key) {
            Some(pair) => pair.1 = value,
            None => pairs.push((key, value)),
        }
    }
    url.query_pairs_mut().extend_pairs(pairs);
    download_json_note(
        client,
        &url,
        NAME,
        id,
        &format!("Downloading {endpoint} API JSON"),
    )
}

/// The `traverse_obj(info, {...}, get_all=False)` fields that are present.
fn metadata_fields(info: &Value) -> Vec<(&'static str, Value)> {
    let text = |key: &str| info.get(key).filter(|value| value.is_string()).cloned();
    let int = |key: &str| {
        Some(super::streamable::int_or_none(info.get(key))).filter(|value| !value.is_null())
    };
    let timestamp = ["broadcast_date_time", "broadcastDateTime"]
        .iter()
        .filter_map(|key| info.get(*key).and_then(Value::as_str))
        .find_map(crate::dates::parse_iso8601)
        .map(Value::from);
    let thumbnail = info
        .get("image")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
        .map(Value::from);
    let is_live = info
        .get("isLive")
        .filter(|value| value.is_boolean())
        .cloned();
    [
        ("title", text("title")),
        ("description", text("description")),
        ("timestamp", timestamp),
        ("duration", int("duration")),
        ("thumbnail", thumbnail),
        ("is_live", is_live),
        ("series", text("seriesTitle")),
        ("season_number", int("seasonNumber")),
        ("episode", text("episodeTitle")),
        ("episode_number", int("episodeNumber")),
    ]
    .into_iter()
    .filter_map(|(key, value)| value.map(|value| (key, value)))
    .collect()
}

impl Extractor for Tv4 {
    fn key(&self) -> &'static str {
        NAME
    }

    /// `_GEO_BYPASS = False`.
    fn geo(&self) -> crate::geo::GeoContext {
        crate::geo::GeoContext::no_bypass(&[])
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let asset = call_api(
            client,
            "asset",
            &id,
            &[("protocol", "hls,dash"), ("drm", "widevine")],
        )?;
        let info = asset
            .get("metadata")
            .filter(|metadata| metadata.is_object())
            .cloned()
            .unwrap_or_else(|| Value::Object(Default::default()));
        // `headers=self.geo_verification_headers()`: empty without
        // `--geo-verification-proxy`, which this port does not apply.
        let play = call_api(client, "play", &id, &[])?;
        let item = play
            .get("playbackItem")
            .ok_or_else(|| key_error(&id, "playbackItem"))?;
        if !item.is_object() {
            return Err(unsupported("playback items that are not objects"));
        }
        let manifest_url = match item.get("manifestUrl") {
            None => return Err(key_error(&id, "manifestUrl")),
            Some(Value::String(manifest_url)) => manifest_url.clone(),
            Some(_) => return Err(unsupported("manifest URLs that are not strings")),
        };

        let mut formats = Vec::new();
        let mut subtitles = Subtitles::default();
        let manifest = Url::parse(&manifest_url)?;
        if let Some(stream) =
            extract_m3u8_master_optional(client, &manifest, NAME, &id, Some("hls"), true, false)
        {
            formats.extend(stream.formats);
            super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
        }
        let mpd = Url::parse(&manifest_url.replace(".m3u8", ".mpd"))?;
        if let Some(stream) = extract_mpd_optional_note(
            client,
            &mpd,
            NAME,
            &id,
            Some("dash"),
            "Downloading MPD manifest",
        ) {
            formats.extend(stream.formats);
            super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
        }
        let f4m = Url::parse(&manifest_url.replace(".m3u8", ".f4m"))?;
        formats.extend(extract_f4m_optional(client, NAME, &f4m, &id, Some("hds"))?);
        let ism = Url::parse(&ISM_PATH.replace(&manifest_url, ".ism/Manifest"))?;
        let (ism_formats, ism_subtitles) =
            extract_ism_optional(client, NAME, &ism, &id, Some("mss"))?;
        formats.extend(ism_formats);
        super::svt::merge_subtitles(&mut subtitles, ism_subtitles);

        if formats.is_empty()
            && info
                .get("is_geo_restricted")
                .is_some_and(super::nuxt::truthy)
        {
            return Err(crate::geo::restricted(
                &format!(
                    "[{NAME}] {id}: This video is not available from your location due to geo-restriction, or not being authenticated"
                ),
                &["SE"],
            ));
        }

        let mut fields = vec![
            ("id", Value::from(id.as_str())),
            ("formats", Value::Null),
            ("subtitles", Value::Null),
        ];
        fields.extend(metadata_fields(&info));
        let mut info = dict_info(url, (NAME, NAME), fields, formats);
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for url in [
            "http://www.tv4.se/kalla-fakta/klipp/kalla-fakta-5-english-subtitles-2491650",
            "http://www.tv4play.se/iframe/video/3054113",
            "http://www.tv4play.se/sport/3060959",
            "http://www.tv4play.se/film/2378136",
            "http://www.tv4play.se/barn/looney-tunes?video_id=3062412",
            "http://www.tv4play.se/program/farang/3922081",
            "https://www.tv4play.se/program/nyheterna/avsnitt/13315940",
            "https://www.tv4play.se/13315940",
        ] {
            assert!(suitable(url), "{url}");
        }
        assert_eq!(
            &VALID_URL
                .captures(
                    "http://www.tv4.se/kalla-fakta/klipp/kalla-fakta-5-english-subtitles-2491650"
                )
                .unwrap()["id"],
            "2491650"
        );
        assert!(!suitable("https://www.tv4play.se/program/nyheterna"));
        assert!(!suitable("https://www.tv4.se/nyheter/123"));
    }

    #[test]
    fn ism_manifest_urls_replace_the_hls_path() {
        assert_eq!(
            ISM_PATH.replace("https://x.test/a.ism/b/c.m3u8?t=1", ".ism/Manifest"),
            "https://x.test/a.ism/Manifest?t=1"
        );
    }
}
