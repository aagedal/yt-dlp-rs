//! Upstream `GlomexIE`, `GlomexEmbedIE` (`yt_dlp/extractor/glomex.py`): a
//! `video.glomex.com` page is followed to its player embed, whose integration
//! API lists the video's HLS and progressive sources. Playlists of several
//! videos and the webpage hand-off to embeds are not ported.
use super::{Extractor, dict_format, dict_info, extract_m3u8_master_optional};
use crate::{Error, ExtractResult, Result, Subtitles, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Glomex;
pub struct GlomexEmbed;

const NAME: &str = "glomex";
const EMBED_NAME: &str = "glomex:embed";
const INTEGRATION: &str = "19syy24xjn1oqlpc";
const DEFAULT_ORIGIN: &str = "https://player.glomex.com/";
const API_URL: &str = "https://integration-cloudfront-eu-west-1.mes.glomex.cloud/";
const PLAYER_PATH: &str = "//player.glomex.com/integration/1/iframe-player.html";

static PAGE_URL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^https?://video\.glomex\.com/[^/]+/(?P<id>v-[^-]+)").unwrap());
static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?:(?://player\.glomex\.com/integration/[^/]/iframe-player\.html)\?([^#]+&)?playlistId=(?P<id>[^#&]+)")
        .unwrap()
});

/// `_match_valid_url` for `video.glomex.com` pages.
pub fn suitable(url: &str) -> bool {
    PAGE_URL.is_match(url)
}

/// `_match_valid_url` for player embeds.
pub fn embed_suitable(url: &str) -> bool {
    EMBED_URL.is_match(url)
}

/// `GlomexEmbedIE.build_player_url`, smuggling the page as the origin.
fn player_url(video_id: &str, integration: &str, origin: Option<&str>) -> String {
    let mut query = Url::parse(&format!("https:{PLAYER_PATH}")).expect("valid player URL");
    query
        .query_pairs_mut()
        .append_pair("playlistId", video_id)
        .append_pair("integrationId", integration);
    match origin {
        Some(origin) => super::brightcove::smuggle(query.as_str(), &json!({"origin": origin})),
        None => query.to_string(),
    }
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Glomex {what} are not supported natively"))
}

/// The origin smuggled into an embed URL, or the default.
fn origin_of(url: &str) -> (String, Option<String>) {
    let (plain, data) = super::brightcove::unsmuggle(url);
    let origin = data
        .get("origin")
        .and_then(Value::as_str)
        .map(str::to_owned);
    (plain, origin)
}

impl Extractor for Glomex {
    fn key(&self) -> &'static str {
        "Glomex"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, _client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PAGE_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `url_result(GlomexEmbedIE.build_player_url(...), 'GlomexEmbed')`, followed.
        let target = player_url(&video_id, INTEGRATION, Some(url.as_str()));
        let mut follow = super::direct::build_at(url.clone(), url, String::new());
        follow.formats.clear();
        follow.direct = None;
        follow.extra.insert(crate::FOLLOW_URL.into(), target.into());
        Ok(Some(ExtractResult::Video(follow)))
    }
}

impl Extractor for GlomexEmbed {
    fn key(&self) -> &'static str {
        "GlomexEmbed"
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let (plain, origin) = origin_of(url.as_str());
        let Ok(parsed) = Url::parse(&plain) else {
            return Ok(None);
        };
        let Some(found) = EMBED_URL.captures(&plain) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(EMBED_NAME);
        crate::screen::screen(&format!(
            "[{EMBED_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let Some(integration) = parsed
            .query_pairs()
            .find(|(key, _)| key == "integrationId")
            .map(|(_, value)| value.into_owned())
        else {
            return Err(Error::Reported("No integrationId in URL".into()));
        };
        let origin = origin.unwrap_or_else(|| DEFAULT_ORIGIN.to_owned());
        download_and_extract(client, url, &playlist_id, &integration, &origin)
    }
}

/// `_download_and_extract_api_data` for one video.
fn download_and_extract(
    client: &Client,
    page: &Url,
    video_id: &str,
    integration: &str,
    current_url: &str,
) -> Result<Option<ExtractResult>> {
    let video_id_type = match video_id.split('-').next().unwrap_or_default() {
        "v" => "video",
        "pl" => "playlist",
        "rl" => "related videos playlist",
        "cl" => "curated playlist",
        _ => "unknown type",
    };
    let api = super::update_url_query(
        &Url::parse(API_URL)?,
        &[
            ("integration_id", integration),
            ("playlist_id", video_id),
            ("current_url", current_url),
        ],
    );
    let api_data = super::download_json_note(
        client,
        &api,
        EMBED_NAME,
        video_id,
        &format!("Downloading {video_id_type} JSON"),
    )?;
    let videos = api_data
        .get("videos")
        .ok_or_else(|| super::clyp::key_error(video_id, "videos"))?
        .as_array()
        .ok_or_else(|| unsupported("video lists that are not arrays"))?;
    if videos.is_empty() {
        return Err(Error::Reported(format!(
            "[{EMBED_NAME}] no videos found for {video_id}"
        )));
    }
    if videos.len() != 1 {
        return Err(unsupported("playlists of several videos"));
    }
    Ok(Some(ExtractResult::Video(extract_video(
        client, page, &videos[0], video_id,
    )?)))
}

/// `_extract_api_data`: one video's formats, subtitles and thumbnails.
fn extract_video(client: &Client, page: &Url, video: &Value, video_id: &str) -> Result<VideoInfo> {
    if video.get("error_code").and_then(Value::as_str) == Some("contentGeoblocked") {
        return Err(Error::Reported(format!(
            "[{EMBED_NAME}] {video_id}: This video is not available from your location due to geo restriction"
        )));
    }
    let language = video
        .get("language")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut formats = Vec::new();
    let mut subtitles = Subtitles::default();
    let sources = video
        .get("source")
        .and_then(Value::as_object)
        .ok_or_else(|| super::clyp::key_error(video_id, "source"))?;
    for (format_id, format_url) in sources {
        let format_url = format_url
            .as_str()
            .ok_or_else(|| unsupported("source URLs that are not strings"))?;
        if super::direct::determine_ext(format_url).as_deref() == Some("m3u8") {
            let manifest = Url::parse(format_url)?;
            if let Some(stream) = extract_m3u8_master_optional(
                client,
                &manifest,
                EMBED_NAME,
                video_id,
                Some(format_id),
                true,
                false,
            ) {
                formats.extend(stream.formats);
                super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
            }
        } else {
            formats.push(dict_format(
                vec![
                    ("url", format_url.into()),
                    ("format_id", format_id.as_str().into()),
                ],
                EMBED_NAME,
                video_id,
            )?);
        }
    }
    if let Some(language) = &language {
        for format in &mut formats {
            format.language = Some(language.clone());
        }
    }
    let thumbnails = thumbnails(video);
    let fields = vec![
        (
            "id",
            video
                .get("clip_id")
                .and_then(Value::as_str)
                .unwrap_or(video_id)
                .into(),
        ),
        ("title", video.get("title").cloned().unwrap_or(Value::Null)),
        (
            "description",
            video.get("description").cloned().unwrap_or(Value::Null),
        ),
        ("thumbnails", thumbnails),
        (
            "duration",
            super::streamable::int_or_none(video.get("clip_duration")),
        ),
        (
            "timestamp",
            video.get("created_at").cloned().unwrap_or(Value::Null),
        ),
        ("formats", Value::Null),
        ("subtitles", Value::Null),
    ];
    let mut info = dict_info(page, (EMBED_NAME, "GlomexEmbed"), fields, formats);
    info.subtitles = subtitles;
    // The thumbnails' `{'id', 'url', 'width', 'height'}` key order.
    if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
        shape.push_str(";thumbnail!=id,url,width,height");
    }
    Ok(info)
}

/// The `images` and `image` of a video as player thumbnails.
fn thumbnails(video: &Value) -> Value {
    let mut images: Vec<Value> = video
        .get("images")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    images.push(video.get("image").cloned().unwrap_or_else(|| json!({})));
    let mut thumbnails: Vec<Value> = Vec::new();
    for image in images {
        let Some(url) = image.get("url").and_then(Value::as_str) else {
            continue;
        };
        let thumbnail = json!({
            "id": image.get("id").cloned().unwrap_or(Value::Null),
            "url": format!("{url}/profile:player-960x540"),
            "width": 960,
            "height": 540,
        });
        if !thumbnails
            .iter()
            .any(|existing| existing["url"] == thumbnail["url"])
        {
            thumbnails.push(thumbnail);
        }
    }
    Value::Array(thumbnails)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://video.glomex.com/sport/v-cb24uwg77hgh-nach-2-0-sieg-guardiola-mit-mancity-vor-naechstem-titel"
        ));
        assert!(embed_suitable(
            "https://player.glomex.com/integration/1/iframe-player.html?integrationId=4059a013k56vb2yd&playlistId=v-cfa6lye0dkdd-sf"
        ));
        assert!(embed_suitable(
            "https://player.glomex.com/integration/1/iframe-player.html?origin=fullpage&integrationId=19syy24xjn1oqlpc&playlistId=rl-vcb49w1fb592p&playlistIndex=0"
        ));
        assert!(!embed_suitable(
            "https://video.glomex.com/sport/v-cb24uwg77hgh"
        ));
    }
}
