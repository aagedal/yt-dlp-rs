//! Upstream `NovaEmbedIE` (`yt_dlp/extractor/nova.py`): a `media.cms.nova.cz`
//! embed page's `player:` JSON sources (HLS and DASH manifests, or direct
//! files), with its title, poster and duration. DRM-only sources are
//! reported as DRM protected. The article pages (`NovaIE`) are not ported.
use super::{Extractor, dict_format, dict_info, download_webpage, extract_mpd_optional_note};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct NovaEmbed;

const NAME: &str = "NovaEmbed";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://media(?:tn)?\.cms\.nova\.cz/embed/(?P<id>[^/?#&]+)")
        .unwrap()
});
static PLAYER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"player:\s*").unwrap());
static PLAYER_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r";\s*</script>").unwrap());
static LEGACY_JSON: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?:(?:replacePlaceholders|processAdTagModifier).*?:\s*)?(?:replacePlaceholders|processAdTagModifier)\s*\(\s*(?P<json>\{.*?\})\s*\)(?:\s*\))?\s*,",
    )
    .unwrap()
});
static TITLE_VALUE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<value>(?P<value>[^<]+)").unwrap());
static TITLE_VIDEO: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"videoTitle\s*:\s*(?:"(?P<value>[^"]+)"|'(?P<single>[^']+)')"#).unwrap()
});
static POSTER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"poster\s*:\s*(?:"(?P<value>[^"]+)"|'(?P<single>[^']+)')"#).unwrap()
});
static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"videoDuration\s*:\s*(\d+)").unwrap());

/// `_match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `int_or_none` for a JSON value.
fn int_or_none(value: Option<&Value>) -> Option<i64> {
    match value? {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|f| f as i64)),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// `url_or_none` for a JSON value (a string that is an absolute URL).
fn url_or_none(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?.trim();
    (Url::parse(text).is_ok() && !text.contains(char::is_whitespace)).then(|| text.to_owned())
}

/// `determine_ext` of a manifest or file URL, lower-cased.
fn ext_of(url: &str) -> String {
    super::direct::determine_ext(url).unwrap_or_default()
}

/// Upstream `process_format_list`: one source (or a list of them) into
/// formats. A source with a DRM key system is skipped and sets `has_drm`.
fn process_format_list(
    client: &Client,
    id: &str,
    source: &Value,
    format_id: &str,
    formats: &mut Vec<crate::VideoFormat>,
    has_drm: &mut bool,
) -> Result<()> {
    let items: Vec<&Value> = match source {
        Value::Array(items) => items.iter().collect(),
        other => vec![other],
    };
    for item in items {
        let Value::Object(dict) = item else {
            continue;
        };
        if dict
            .get("drm")
            .and_then(|drm| drm.get("keySystem"))
            .is_some_and(super::nuxt::truthy)
        {
            *has_drm = true;
            continue;
        }
        let Some(format_url) = url_or_none(dict.get("src")) else {
            continue;
        };
        let format_type = dict.get("type").and_then(Value::as_str).unwrap_or("");
        let ext = ext_of(&format_url);
        let manifest = Url::parse(&format_url)?;
        if format_type == "application/x-mpegURL" || format_id == "HLS" || ext == "m3u8" {
            if let Some(stream) = super::extract_m3u8_master_optional(
                client,
                &manifest,
                NAME,
                id,
                Some("hls"),
                false,
                false,
            ) {
                formats.extend(stream.formats);
            }
        } else if format_type == "application/dash+xml" || format_id == "DASH" || ext == "mpd" {
            if let Some(stream) = extract_mpd_optional_note(
                client,
                &manifest,
                NAME,
                id,
                Some("dash"),
                "Downloading MPD manifest",
            ) {
                formats.extend(stream.formats);
            }
        } else {
            formats.push(dict_format(vec![("url", format_url.into())], NAME, id)?);
        }
    }
    Ok(())
}

impl Extractor for NovaEmbed {
    fn key(&self) -> &'static str {
        NAME
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
        let webpage = download_webpage(client, url, NAME, &id)?;

        let mut has_drm = false;
        let mut duration: Option<i64> = None;
        let mut formats = Vec::new();

        // `_search_json(r'player:', ..., end_pattern=r';\s*</script>', fatal=False)`.
        let player = match PLAYER.find(&webpage) {
            Some(start) => {
                let rest = &webpage[start.start()..];
                let end = PLAYER_END.find(rest).map_or(rest.len(), |end| end.start());
                super::search_json_with(&PLAYER, &rest[..end], &str::to_owned).flatten()
            }
            None => None,
        };
        match player {
            Some(player) => {
                let sources = player
                    .get("lib")
                    .and_then(|lib| lib.get("source"))
                    .and_then(|source| source.get("sources"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                for source in &sources {
                    process_format_list(client, &id, source, "", &mut formats, &mut has_drm)?;
                }
                duration = int_or_none(
                    player
                        .get("sourceInfo")
                        .and_then(|info| info.get("duration")),
                );
            }
            None => super::search_warning(NAME, "player"),
        }

        if formats.is_empty() && !has_drm {
            // The older code path, in use before August 2023.
            let Some(legacy) = LEGACY_JSON.captures(&webpage) else {
                if webpage.contains("Player.init") {
                    return Err(Error::UnsupportedHtml5(
                        "NovaEmbed's older Player.init pages are not supported natively".into(),
                    ));
                }
                return Err(Error::Reported(format!(
                    "[{NAME}] {id}: Unable to extract player; {}",
                    crate::BUG_REPORTS_MESSAGE
                )));
            };
            let player: Value = serde_json::from_str(&legacy["json"]).map_err(|_| {
                Error::UnsupportedHtml5(
                    "NovaEmbed legacy player JSON that does not parse is not supported natively"
                        .into(),
                )
            })?;
            if let Some(tracks) = player.get("tracks").and_then(Value::as_object) {
                for (format_id, format_list) in tracks {
                    process_format_list(
                        client,
                        &id,
                        format_list,
                        format_id,
                        &mut formats,
                        &mut has_drm,
                    )?;
                }
            }
            duration = int_or_none(player.get("duration"));
        }

        if formats.is_empty() && has_drm {
            crate::no_formats::raise_expected(
                NAME,
                Some(&id),
                None,
                "This video is DRM protected",
            )?;
        }

        // `_og_search_title(default=None)` or the fatal `<value>` / `videoTitle` search.
        let title = match crate::html::og_search_property(&webpage, "title") {
            Some(title) => title,
            None => TITLE_VALUE
                .captures(&webpage)
                .map(|found| found["value"].to_owned())
                .or_else(|| {
                    TITLE_VIDEO.captures(&webpage).map(|found| {
                        found
                            .name("value")
                            .or_else(|| found.name("single"))
                            .map(|value| value.as_str().to_owned())
                            .unwrap_or_default()
                    })
                })
                .ok_or_else(|| {
                    Error::Reported(format!(
                        "[{NAME}] {id}: Unable to extract title; {}",
                        crate::BUG_REPORTS_MESSAGE
                    ))
                })?,
        };
        let thumbnail = crate::html::og_search_property(&webpage, "image").or_else(|| {
            POSTER.captures(&webpage).map(|found| {
                found
                    .name("value")
                    .or_else(|| found.name("single"))
                    .map(|value| value.as_str().to_owned())
                    .unwrap_or_default()
            })
        });
        if let Some(found) = DURATION.captures(&webpage) {
            duration = found[1].parse().ok().or(duration);
        }

        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", id.clone().into()),
                ("title", title.into()),
                ("thumbnail", thumbnail.map_or(Value::Null, Value::from)),
                ("duration", duration.map_or(Value::Null, Value::from)),
                ("formats", Value::Null),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable("https://media.cms.nova.cz/embed/KybpWYvcgOa"));
        assert!(suitable(
            "https://mediatn.cms.nova.cz/embed/EU5ELEsmOHt?autoplay=1"
        ));
        assert!(!suitable("https://tn.nova.cz/clanek/example.html"));
    }
}
