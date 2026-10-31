//! Upstream `UOLIE` (`yt_dlp/extractor/uol.py`): UOL videos from the
//! `apiuol/v3/media/detail` item and the croupier formats map: progressive
//! formats and HLS formats whose media playlist and segment requests carry
//! the format's `ver`/`r`/`token`/`sign` query (`extra_param_to_segment_url`).
use super::{Extractor, dict_format, dict_info, download_json, update_url_query};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Uol;

pub(crate) const NAME: &str = "uol.com.br";
pub(crate) const KEY: &str = "UOL";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:.+?\.)?uol\.com\.br/.*?(?:(?:mediaId|v)=|view/(?:[a-z0-9]+/)?|video(?:=|/(?:\d{4}/\d{2}/\d{2}/)?))(?P<id>\d+|[\w-]+-[A-Z0-9]+)",
    )
    .unwrap()
});

/// `qualities([...])`.
const QUALITIES: [&str; 5] = ["mobile", "WEBM", "360p", "720p", "1080p"];
const THUMBNAILS: [&str; 6] = ["Small", "Medium", "Wmedium", "Large", "Wlarge", "Xlarge"];

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("UOL {what} are not supported natively"))
}

fn key_error(id: &str, key: &str) -> Error {
    Error::Reported(format!(
        "{id}: An extractor error has occurred. (caused by KeyError('{key}')); {}",
        crate::BUG_REPORTS_MESSAGE
    ))
}

/// Python `str(value)` of a JSON scalar (as `urlencode` renders it).
fn py_str(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) => Ok(number.to_string()),
        Value::Bool(true) => Ok("True".into()),
        Value::Bool(false) => Ok("False".into()),
        Value::Null => Ok("None".into()),
        _ => Err(unsupported("query values that are not scalars")),
    }
}

/// An optional string field (`None` when missing or null).
fn text<'a>(object: &'a Value, key: &str) -> Result<Option<&'a str>> {
    match object.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(unsupported(&format!("{key} fields that are not strings"))),
    }
}

impl Extractor for Uol {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let detail = Url::parse(&format!(
            "https://api.mais.uol.com.br/apiuol/v3/media/detail/{video_id}"
        ))?;
        let response = download_json(client, &detail, NAME, &video_id)?;
        let video_data = response
            .get("item")
            .ok_or_else(|| key_error(&video_id, "item"))?;
        if !video_data.is_object() {
            return Err(unsupported("detail items that are not objects"));
        }
        let media_id = match video_data.get("mediaId") {
            None => return Err(key_error(&video_id, "mediaId")),
            Some(Value::String(id)) => id.clone(),
            Some(Value::Number(id)) => id.to_string(),
            Some(_) => return Err(unsupported("media IDs that are not strings or numbers")),
        };
        let title = video_data
            .get("title")
            .cloned()
            .ok_or_else(|| key_error(&video_id, "title"))?;
        let ver = video_data.get("revision").cloned().unwrap_or(json!(2));

        let croupier = Url::parse(&format!(
            "https://croupier.mais.uol.com.br/v3/formats/{media_id}/jsonp"
        ))?;
        let uol_formats = download_json(client, &croupier, NAME, &media_id)?;
        let uol_formats = uol_formats
            .as_object()
            .ok_or_else(|| unsupported("format maps that are not objects"))?;
        let mut formats = Vec::new();
        for (format_id, f) in uol_formats {
            if !f.is_object() {
                continue;
            }
            let f_url = [f.get("url"), f.get("secureUrl")]
                .into_iter()
                .flatten()
                .find(|value| super::nuxt::truthy(value));
            let Some(f_url) = f_url else {
                continue;
            };
            let f_url = f_url
                .as_str()
                .ok_or_else(|| unsupported("format URLs that are not strings"))?;
            let mut query = vec![
                ("ver", py_str(&ver)?),
                ("r", "http://mais.uol.com.br".to_owned()),
            ];
            for k in ["token", "sign"] {
                if let Some(v) = f.get(k).filter(|v| super::nuxt::truthy(v)) {
                    query.push((k, py_str(v)?));
                }
            }
            let pairs: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
            let f_url = update_url_query(&Url::parse(f_url)?, &pairs);
            if format_id == "HLS" {
                let encoded_query = super::login::urlencode(&pairs);
                if let Some(stream) = super::extract_m3u8_master_optional(
                    client,
                    &f_url,
                    NAME,
                    &media_id,
                    Some("hls"),
                    false,
                    false,
                ) {
                    for mut m3u8_f in stream.formats {
                        m3u8_f.extra.insert(
                            "extra_param_to_segment_url".into(),
                            encoded_query.clone().into(),
                        );
                        crate::info_order::mark_format_update(
                            &mut m3u8_f,
                            "extra_param_to_segment_url",
                        );
                        m3u8_f.url = update_url_query(&m3u8_f.url, &pairs);
                        formats.push(m3u8_f);
                    }
                }
                continue;
            }
            let quality = QUALITIES
                .iter()
                .position(|q| q == format_id)
                .map_or(-1, |index| index as i64);
            formats.push(dict_format(
                vec![
                    ("format_id", format_id.clone().into()),
                    ("url", f_url.as_str().into()),
                    ("quality", quality.into()),
                ],
                NAME,
                &media_id,
            )?);
        }

        let mut tags = Vec::new();
        match video_data.get("tags") {
            None => {}
            Some(Value::Array(items)) => {
                for tag in items {
                    if !tag.is_object() {
                        return Err(unsupported("tags that are not objects"));
                    }
                    if let Some(description) =
                        tag.get("description").filter(|d| super::nuxt::truthy(d))
                    {
                        tags.push(description.clone());
                    }
                }
            }
            Some(_) => return Err(unsupported("tag lists that are not lists")),
        }

        let mut thumbnails = Vec::new();
        for q in THUMBNAILS {
            let Some(q_url) = video_data
                .get(format!("thumb{q}"))
                .filter(|value| super::nuxt::truthy(value))
            else {
                continue;
            };
            thumbnails.push(json!({"id": q, "url": q_url}));
        }

        let description = text(video_data, "description")?.map(crate::html::clean_html);
        let duration = match video_data.get("duration") {
            Some(Value::String(duration)) => {
                crate::parse_duration(duration).map(|duration| match duration {
                    crate::info::PyNumber::Int(value) => Value::from(value),
                    crate::info::PyNumber::Float(value) => Value::from(value),
                })
            }
            _ => None,
        };
        let timestamp =
            text(video_data, "publishDate")?.and_then(crate::dates::parse_iso8601_space);
        let mut info = dict_info(
            url,
            (NAME, KEY),
            vec![
                ("id", media_id.into()),
                ("title", title),
                ("description", description.map_or(Value::Null, Value::from)),
                ("thumbnails", Value::Array(thumbnails)),
                ("duration", duration.unwrap_or(Value::Null)),
                ("tags", Value::Array(tags)),
                ("formats", Value::Null),
                ("timestamp", timestamp.map_or(Value::Null, Value::from)),
                (
                    "view_count",
                    super::streamable::int_or_none(video_data.get("viewsQtty")),
                ),
            ],
            formats,
        );
        // The thumbnails' `{'id', 'url'}` key order.
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=id,url");
        }
        Ok(Some(info))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        for url in [
            "http://player.mais.uol.com.br/player_video_v3.swf?mediaId=15951931",
            "http://tvuol.uol.com.br/video/incendio-destroi-uma-das-maiores-casas-noturnas-de-londres-04024E9A3268D4C95326",
            "http://mais.uol.com.br/static/uolplayer/index.html?mediaId=15951931",
            "http://mais.uol.com.br/view/15954259",
            "http://noticias.band.uol.com.br/brasilurgente/video/2016/08/05/15951931/miss-simpatia-e-encontrada-morta.html",
            "http://videos.band.uol.com.br/programa.asp?e=noticias&pr=brasil-urgente&v=15951931&t=Policia-desmonte-base-do-PCC-na-Cracolandia",
            "http://mais.uol.com.br/view/cphaa0gl2x8r/incendio-destroi-uma-das-maiores-casas-noturnas-de-londres-04024E9A3268D4C95326",
            "http://noticias.uol.com.br//videos/assistir.htm?video=rafaela-silva-inspira-criancas-no-judo-04024D983968D4C95326",
            "http://mais.uol.com.br/view/e0qbgxid79uv/15275470",
        ] {
            assert!(suitable(url), "{url}");
        }
        assert!(!suitable("https://www.uol.com.br/"));
    }
}
