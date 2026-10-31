//! Upstream `Ant1NewsGrEmbedIE` (`yt_dlp/extractor/antenna.py`): an
//! antenna.gr / ant1news.gr player URL, resolved by a HEAD request to its
//! canonical player host, whose `jsonPlayer` API gives the video's `url`
//! (HLS or a progressive file) and an optional `thumb`.
use super::{Extractor, dict_format, dict_info, download_json};
use crate::{Error, Result, VideoFormat, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Ant1NewsGrEmbed;

const NAME: &str = "ant1newsgr:embed";
const KEY: &str = "Ant1NewsGrEmbed";
const API_PATH: &str = "/templates/data/jsonPlayer";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:https?:)?//(?:[a-zA-Z0-9\-]+\.)?(?:antenna|ant1news)\.gr/templates/pages/player\?(?:[^#]+&)?cid=(?P<id>[^#&]+)",
    )
    .unwrap()
});

/// `_match_valid_url`'s `id` for the player URL.
pub fn embed_suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!(
        "Ant1NewsGr embeds with {what} are not supported natively"
    ))
}

/// `ExtractorError(msg)` (not expected): a bug-report error.
fn extractor_error(id: &str, message: &str) -> Error {
    Error::Reported(format!(
        "[{NAME}] {id}: {message}; {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

impl Extractor for Ant1NewsGrEmbed {
    fn key(&self) -> &'static str {
        KEY
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
        // `_request_webpage(HEADRequest(url))`: the canonical player URL.
        crate::screen::screen(&format!("[{NAME}] {id}: Resolve canonical player URL"));
        let canonical = super::direct::head_with_headers(client, url, &[])?
            .url()
            .clone();
        let host = canonical
            .host_str()
            .ok_or_else(|| unsupported("canonical URLs without a host"))?
            .to_owned();
        // `parse_qs(query)['cid'][0]`: blank values are dropped.
        let Some(cid) = canonical
            .query_pairs()
            .find(|(key, value)| key == "cid" && !value.is_empty())
            .map(|(_, value)| value.into_owned())
        else {
            return Err(super::clyp::key_error(&id, "cid"));
        };

        // `_download_and_extract_api_data`: `http_scheme()` is https.
        let api = super::update_url_query(
            &Url::parse(&format!("https://{host}{API_PATH}"))?,
            &[("cid", cid.as_str())],
        );
        let info = download_json(client, &api, NAME, &id)?;
        let media = match info.get("url") {
            Some(Value::String(media)) if !media.is_empty() => media.clone(),
            _ => {
                return Err(extractor_error(&id, &format!("No source found for {id}")));
            }
        };
        let media_url = Url::parse(&media)?;
        let (formats, subtitles) =
            if super::direct::determine_ext(&media).as_deref() == Some("m3u8") {
                let mut manifest =
                    super::extract_m3u8_fatal_master(client, &media_url, NAME, &id, None, true)?;
                for format in &mut manifest.formats {
                    if format.ext.is_empty() || format.ext == "unknown_video" {
                        format.ext = "mp4".into();
                    }
                }
                (manifest.formats, manifest.subtitles)
            } else {
                let ext =
                    super::direct::determine_ext(&media).unwrap_or_else(|| "unknown_video".into());
                let format: VideoFormat = dict_format(
                    vec![("url", media.clone().into()), ("format_id", ext.into())],
                    NAME,
                    &id,
                )?;
                (vec![format], Default::default())
            };

        // `scale_thumbnails_to_max_format_width` only rescales when a format
        // has a width; that case is not ported.
        let thumbnails = match info.get("thumb") {
            Some(Value::String(thumb)) if !thumb.is_empty() => {
                if formats.iter().any(|format| format.width.is_some()) {
                    return Err(unsupported("thumbnails scaled to a format width"));
                }
                json!([{ "url": thumb }])
            }
            _ => json!([]),
        };
        let title = info.get("title").cloned().unwrap_or(Value::Null);
        let mut result = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", id.clone().into()),
                ("title", title),
                ("thumbnails", thumbnails),
                ("formats", Value::Null),
                ("subtitles", Value::Null),
            ],
            formats,
        );
        result.subtitles = subtitles;
        Ok(Some(result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(embed_suitable(
            "https://www.antenna.gr/templates/pages/player?cid=3f_li_c_az_jw_y_u=&w=670&h=377"
        ));
        assert!(embed_suitable(
            "//ant1news.gr/templates/pages/player?w=1&cid=abc"
        ));
        assert!(!embed_suitable(
            "https://www.antenna.gr/watch/1643812/oi-prodotes-epeisodio-01/"
        ));
    }
}
