//! Upstream `XinpianchangIE` (`yt_dlp/extractor/xinpianchang.py`):
//! Xinpianchang (新片场) videos from the page's Next.js video `vid`/`appKey`
//! and the media API's DASH, HLS and progressive resources.
use super::{
    Extractor, dict_format, dict_info, download_json, download_webpage_note_headers,
    extract_m3u8_fatal_master, extract_mpd_fatal,
};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct Xinpianchang;

const NAME: &str = "Xinpianchang";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(r"^https?://(www\.)?xinpianchang\.com/(?P<id>a\d+)").unwrap()
});

/// `_match_id`.
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
    Error::UnsupportedHtml5(format!("Xinpianchang {what} are not supported natively"))
}

fn get<'a>(value: &'a Value, key: &str, id: &str) -> Result<&'a Value> {
    value.get(key).ok_or_else(|| key_error(id, key))
}

/// `str(value)` of a string or integer (other types are not supported).
fn py_str(value: &Value, what: &str) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) if number.is_i64() || number.is_u64() => Ok(number.to_string()),
        _ => Err(unsupported(what)),
    }
}

/// `int_or_none(value)`.
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

impl Extractor for Xinpianchang {
    fn key(&self) -> &'static str {
        NAME
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
        let webpage = download_webpage_note_headers(
            client,
            url,
            NAME,
            &video_id,
            "Downloading webpage",
            &[("Referer", url.as_str())],
        )?;
        let next_data = super::search_nextjs_data(&webpage, NAME, &video_id)?;
        let mut video_data = &next_data;
        for key in ["props", "pageProps", "detail", "video"] {
            video_data = get(video_data, key, &video_id)?;
        }
        let vid = py_str(
            get(video_data, "vid", &video_id)?,
            "video IDs of other types",
        )?;
        let app_key = py_str(
            get(video_data, "appKey", &video_id)?,
            "app keys of other types",
        )?;
        let api = super::update_url_query(
            &Url::parse(&format!(
                "https://mod-api.xinpianchang.com/mod/api/v2/media/{vid}"
            ))?,
            &[("appKey", app_key.as_str())],
        );
        let response = download_json(client, &api, NAME, &video_id)?;
        let data = get(&response, "data", &video_id)?;
        // `data.get('resource').items()`.
        let resources = match data.get("resource") {
            Some(Value::Object(resources)) => resources,
            None | Some(Value::Null) => {
                return Err(Error::Reported(
                    "'NoneType' object has no attribute 'items'".into(),
                ));
            }
            Some(_) => return Err(unsupported("resources that are not objects")),
        };
        let mut formats = Vec::new();
        let mut subtitles = crate::Subtitles::default();
        for (kind, resource) in resources {
            match kind.as_str() {
                "dash" | "hls" => {
                    let Value::Object(resource) = resource else {
                        return Err(unsupported("stream resources that are not objects"));
                    };
                    let Some(manifest) = resource.get("url").filter(|url| super::nuxt::truthy(url))
                    else {
                        continue;
                    };
                    let Value::String(manifest) = manifest else {
                        return Err(unsupported("stream URLs that are not strings"));
                    };
                    let manifest = Url::parse(manifest)
                        .map_err(|_| unsupported("stream URLs that are not absolute"))?;
                    let stream = if kind == "dash" {
                        extract_mpd_fatal(client, &manifest, NAME, &video_id, true)?
                    } else {
                        extract_m3u8_fatal_master(client, &manifest, NAME, &video_id, None, true)?
                    };
                    formats.extend(stream.formats);
                    super::svt::merge_subtitles(&mut subtitles, stream.subtitles);
                }
                "progressive" => {
                    let Value::Array(progressive) = resource else {
                        return Err(unsupported("progressive resources that are not lists"));
                    };
                    for prog in progressive {
                        let Value::Object(prog) = prog else {
                            return Err(unsupported("progressive files that are not objects"));
                        };
                        let Some(media) = prog.get("url").filter(|url| super::nuxt::truthy(url))
                        else {
                            continue;
                        };
                        let media = media
                            .as_str()
                            .and_then(super::json_ld::url_or_none)
                            .ok_or_else(|| unsupported("progressive files without a valid URL"))?;
                        let opt =
                            |key: &str| int_or_none(prog.get(key)).map_or(Value::Null, Value::from);
                        formats.push(dict_format(
                            vec![
                                ("url", media.into()),
                                ("width", opt("width")),
                                ("height", opt("height")),
                                ("ext", "mp4".into()),
                                // NB: Server returns 403 without the Range header
                                ("http_headers", json!({"Range": "bytes=0-"})),
                            ],
                            NAME,
                            &video_id,
                        )?);
                    }
                }
                _ => {}
            }
        }
        let field = |key: &str| data.get(key).cloned().unwrap_or(Value::Null);
        let owner = data.get("owner");
        let uploader = owner
            .and_then(|owner| owner.get("username"))
            .cloned()
            .unwrap_or(Value::Null);
        let uploader_id = match owner.and_then(|owner| owner.get("id")) {
            None | Some(Value::Null) => Value::Null,
            Some(id) => py_str(id, "uploader IDs of other types")?.into(),
        };
        let mut info = dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", video_id.clone().into()),
                ("title", field("title")),
                ("description", field("description")),
                (
                    "duration",
                    int_or_none(data.get("duration")).map_or(Value::Null, Value::from),
                ),
                ("categories", field("categories")),
                ("tags", field("keywords")),
                ("thumbnail", field("cover")),
                ("uploader", uploader),
                ("uploader_id", uploader_id),
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
        assert!(suitable("https://www.xinpianchang.com/a11766551"));
        assert!(suitable(
            "https://www.xinpianchang.com/a11779743?from=IndexPick&part=%E7%BC%96%E8%BE%91%E7%B2%BE%E9%80%89&index=2"
        ));
        assert!(!suitable("https://www.xinpianchang.com/u10357277"));
    }
}
