//! Upstream `CloudflareStreamIE` (`yt_dlp/extractor/cloudflarestream.py`):
//! a video's (or signed token's) `manifest/video.m3u8` and `video.mpd`
//! formats, both non-fatal.
use super::{Extractor, dict_info, extract_m3u8_optional, extract_mpd_optional_note};
use crate::{Error, Result, VideoInfo};
use base64::Engine;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct CloudflareStream;

const NAME: &str = "CloudflareStream";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    let subdomain = r"(?:(?:watch|iframe|customer-\w+)\.)?";
    let domain = r"(?:cloudflarestream\.com|(?:videodelivery|bytehighway)\.net)";
    let embed = format!(r"(?:embed\.|{subdomain}){domain}/embed/[^/?#]+\.js\?(?:[^#]+&)?video=");
    let id = r"[\da-f]{32}|eyJ[\w-]+\.[\w-]+\.[\w-]+";
    crate::extractor::url_regex(&format!(
        r"^https?://(?:{subdomain}(?P<domain>{domain})/|{embed})(?P<id>{id})"
    ))
    .unwrap()
});

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// A signed token's `sub`: `base64.urlsafe_b64decode(PAYLOAD + '===')`.
fn token_subject(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let engine = base64::engine::GeneralPurpose::new(
        &base64::alphabet::URL_SAFE,
        base64::engine::GeneralPurposeConfig::new()
            .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent),
    );
    let decoded = engine.decode(payload.trim_end_matches('=')).ok()?;
    let value: Value = serde_json::from_slice(&decoded).ok()?;
    value.get("sub")?.as_str().map(str::to_owned)
}

impl Extractor for CloudflareStream {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let mut video_id = found["id"].to_owned();
        let domain = match found.name("domain").map(|domain| domain.as_str()) {
            Some("bytehighway.net") => "bytehighway.net",
            _ => "cloudflarestream.com",
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let base_url = format!("https://{domain}/{video_id}/");
        if video_id.contains('.') {
            video_id = token_subject(&video_id).ok_or_else(|| {
                Error::UnsupportedHtml5(
                    "CloudflareStream tokens without a decodable string `sub` are not supported natively"
                        .into(),
                )
            })?;
        }
        let manifest_base_url = format!("{base_url}manifest/video.");
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        let m3u8 = Url::parse(&format!("{manifest_base_url}m3u8"))?;
        if let Some(stream) =
            extract_m3u8_optional(client, &m3u8, NAME, &video_id, Some("hls"), true)
        {
            formats.extend(stream.formats);
            subtitles = stream.subtitles;
        }
        let mpd = Url::parse(&format!("{manifest_base_url}mpd"))?;
        if let Some(stream) = extract_mpd_optional_note(
            client,
            &mpd,
            NAME,
            &video_id,
            Some("dash"),
            "Downloading MPD manifest",
        ) {
            formats.extend(stream.formats);
            super::html5::merge_subtitles(&mut subtitles, stream.subtitles);
        }
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("title", video_id.into()),
                (
                    "thumbnail",
                    format!("{base_url}thumbnails/thumbnail.jpg").into(),
                ),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
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
            "https://embed.cloudflarestream.com/embed/we4g.fla9.latest.js?video=31c9291ab41fac05471db4e73aa11717"
        ));
        assert!(suitable(
            "https://watch.cloudflarestream.com/9df17203414fd1db3e3ed74abbe936c1"
        ));
        assert!(suitable(
            "https://customer-aw5py76sw8wyqzmh.cloudflarestream.com/2463f6d3e06fa29710a337f5f5389fd8/iframe"
        ));
        assert!(suitable(
            "https://embed.videodelivery.net/embed/r4xu.fla9.latest.js?video=81d80727f3022488598f68d323c1ad5e"
        ));
        assert!(!suitable("https://cloudflarestream.com/abc"));
    }

    #[test]
    fn tokens_name_their_subject() {
        assert_eq!(
            token_subject("eyJhbGciOiJSUzI1NiJ9.eyJzdWIiOiJhYmMifQ.sig").as_deref(),
            Some("abc")
        );
    }
}
