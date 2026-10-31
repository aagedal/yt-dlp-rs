//! Upstream `LSMLTVEmbedIE` (`yt_dlp/extractor/lsm.py`): an LTV embed's
//! `window.ltvEmbedPayload`, handed to its CloudyCDN or YouTube source; and
//! `LSMReplayIE`: a replay page's Nuxt data (HLS audio, or an embed).
//! `LSMLREmbedIE` (latvijasradio embeds) is not ported: its test URL no
//! longer serves the player JSON upstream.
use super::{
    Extractor, dict_info, direct, download_webpage, extract_m3u8_fatal_master, follow_url,
    nuxt::Step, search_json, streamable::int_or_none,
};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::{Captures, Regex};
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct LsmLtvEmbed;
pub struct LsmReplay;

const LTV_NAME: &str = "LSMLTVEmbed";
const REPLAY_NAME: &str = "LSMReplay";

static LTV_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://ltv\.lsm\.lv/embed\?(?:[^#]+&)?c=(?P<id>[^#&]+)")
        .unwrap()
});
static LTV_PAYLOAD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"window\.ltvEmbedPayload\s*=").unwrap());

static REPLAY_VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://replay\.lsm\.lv/[^/?#]+/(?:skaties/|klausies/)?(?:ieraksts|statja)/[^/?#]+/(?P<id>\d+)",
    )
    .unwrap()
});
/// `Object.create(null, {...})` (or `Object.create(null)`) in the Nuxt data.
static OBJECT_CREATE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Object\.create\(null(?:,(\{.+\}))?\)").unwrap());

/// `_match_valid_url`, for the LTV embed.
pub fn ltv_embed_suitable(url: &str) -> bool {
    LTV_VALID_URL.is_match(url)
}

/// `_match_valid_url`, for the replay page.
pub fn replay_suitable(url: &str) -> bool {
    REPLAY_VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("{LTV_NAME} {what} are not supported natively"))
}

/// Python's `repr` of an optional string, as an error message shows it.
fn python_repr(text: Option<&str>) -> String {
    match text {
        Some(text) => format!("'{text}'"),
        None => "None".into(),
    }
}

impl Extractor for LsmLtvEmbed {
    fn key(&self) -> &'static str {
        LTV_NAME
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = LTV_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = percent_encoding::percent_decode_str(&found["id"])
            .decode_utf8_lossy()
            .into_owned();
        crate::screen::set_extractor(LTV_NAME);
        crate::screen::screen(&format!(
            "[{LTV_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, LTV_NAME, &video_id)?;
        let data = match search_json(&LTV_PAYLOAD, &webpage, false) {
            Some(Some(data)) => data,
            _ => {
                return Err(Error::Reported(format!(
                    "[{LTV_NAME}] {video_id}: Unable to extract embed json; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        let embed_type = data.pointer("/source/name").and_then(Value::as_str);
        let target = match embed_type {
            // `url_result(embed_url, 'CloudyCDN', video_id, **fields)`.
            Some("backscreen" | "telia") => data
                .pointer("/source/embed_url")
                .and_then(Value::as_str)
                .and_then(super::json_ld::url_or_none)
                .ok_or_else(|| unsupported("embeds without a URL"))?,
            // `url_result(source.id, 'Youtube', ...)`: a bare video ID.
            Some("youtube") => {
                let id = data
                    .pointer("/source/id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| unsupported("YouTube embeds without an ID"))?;
                format!("https://www.youtube.com/watch?v={id}")
            }
            _ => {
                return Err(Error::Reported(format!(
                    "[{LTV_NAME}] {video_id}: Unsupported embed type {}; {}",
                    python_repr(embed_type),
                    crate::BUG_REPORTS_MESSAGE
                )));
            }
        };
        Ok(Some(follow_url(url, target)))
    }
}

/// `_fix_nuxt_data`: the `Object.create` calls, as their property objects.
fn fix_nuxt_data(webpage: &str) -> String {
    OBJECT_CREATE
        .replace_all(webpage, |caps: &Captures| {
            caps.get(1)
                .map_or("null", |group| group.as_str())
                .to_owned()
        })
        .into_owned()
}

impl Extractor for LsmReplay {
    fn key(&self) -> &'static str {
        REPLAY_NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = REPLAY_VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(REPLAY_NAME);
        crate::screen::screen(&format!(
            "[{REPLAY_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = download_webpage(client, url, REPLAY_NAME, &video_id)?;
        let data = super::nuxt::search_nuxt_data(
            &fix_nuxt_data(&webpage),
            REPLAY_NAME,
            Some(&video_id),
            "__REPLAY__",
            true,
            // Upstream's default `traverse=('data', 0)`.
            &[Step::Key("data"), Step::Index(0)],
        )?;
        let playback_type = data
            .pointer("/playback/type")
            .and_then(Value::as_str)
            .ok_or_else(|| super::clyp::key_error(&video_id, "type"))?;

        let media = |path: &str| {
            data.pointer(&format!("/mediaItem/{path}"))
                .filter(|v| !v.is_null())
        };
        let title = media("title").cloned();
        // `('mediaItem', ('lead', 'body'))` with `get_all=False`.
        let description = media("lead").or_else(|| media("body")).cloned();
        let duration = int_or_none(media("duration"));
        let timestamp = media("aired_at")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map(Value::from);
        let thumbnail = media("largeThumbnail")
            .and_then(Value::as_str)
            .and_then(super::json_ld::url_or_none)
            .map(Value::from);
        let optional = |key: &'static str, value: Option<Value>| value.map(|value| (key, value));

        match playback_type {
            "playable_audio_lr" => {
                let hls = data
                    .pointer("/playback/service/hls_url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| super::clyp::key_error(&video_id, "hls_url"))?;
                let manifest = Url::parse(hls)?;
                let mut stream = extract_m3u8_fatal_master(
                    client,
                    &manifest,
                    REPLAY_NAME,
                    &video_id,
                    None,
                    false,
                )?;
                // `_extract_m3u8_formats(url, video_id)`: `ext=None`.
                super::hls::without_ext(&mut stream);
                let mut fields = vec![
                    ("id", Value::from(video_id.clone())),
                    ("formats", Value::Null),
                ];
                fields.extend(
                    [
                        optional("title", title),
                        optional("description", description),
                        (!duration.is_null()).then_some(("duration", duration)),
                        optional("timestamp", timestamp),
                        optional("thumbnail", thumbnail),
                    ]
                    .into_iter()
                    .flatten(),
                );
                Ok(Some(dict_info(
                    url,
                    (REPLAY_NAME, REPLAY_NAME),
                    fields,
                    stream.formats,
                )))
            }
            "embed" => {
                let target = data
                    .pointer("/playback/service/url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| super::clyp::key_error(&video_id, "url"))?;
                // `{'_type': 'url_transparent', 'url': ...}` with the fields.
                let mut fields = Map::new();
                for (key, value) in [
                    ("title", title),
                    ("description", description),
                    ("duration", (!duration.is_null()).then_some(duration)),
                    ("timestamp", timestamp),
                    ("thumbnail", thumbnail),
                ] {
                    if let Some(value) = value {
                        fields.insert(key.into(), value);
                    }
                }
                let mut follow = direct::build_at(url.clone(), url, String::new());
                follow.formats.clear();
                follow.direct = None;
                follow.extra.insert(
                    crate::FOLLOW_TRANSPARENT.into(),
                    json!({"url": target, "fields": Value::Object(fields)}),
                );
                Ok(Some(follow))
            }
            other => Err(Error::Reported(format!(
                "[{REPLAY_NAME}] {video_id}: Unsupported playback type \"{other}\"; {}",
                crate::BUG_REPORTS_MESSAGE
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(ltv_embed_suitable(
            "https://ltv.lsm.lv/embed?enablesdkjs=1&c=eyJpdiI6IncwVzZmUFk2MU12enVWK1I3SUcwQ1E9PSJ9"
        ));
        assert!(ltv_embed_suitable("https://ltv.lsm.lv/embed?c=abc&x=1"));
        assert!(!ltv_embed_suitable("https://ltv.lsm.lv/embed?x=1"));
        assert!(replay_suitable(
            "https://replay.lsm.lv/lv/klausies/ieraksts/lr/183522/138-nepilniga-kompensejamo"
        ));
        assert!(replay_suitable(
            "https://replay.lsm.lv/lv/skaties/ieraksts/ltv/311130/4-studija"
        ));
        assert!(!replay_suitable(
            "https://replay.lsm.lv/lv/skaties/ieraksts/ltv/x/4-studija"
        ));
    }

    #[test]
    fn nuxt_object_create_becomes_its_property_object() {
        assert_eq!(
            fix_nuxt_data("a=Object.create(null,{x:1}); b=Object.create(null)"),
            "a={x:1}; b=null"
        );
    }
}
