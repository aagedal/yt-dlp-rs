//! Upstream `BoxIE` (`yt_dlp/extractor/box.py`): Box shared files on
//! `app.box.com` and `ent.box.com` subdomains. The shared page gives the
//! file ID (the URL's, else the post-stream data's) and the request token;
//! an element access token then reads the file's metadata and its DASH
//! representations. HLS representations and shared folders are not supported.
use super::{
    Extractor, dict_info, download_json_note_headers, download_webpage, extract_mpd_fatal,
};
use super::{post_json_note_headers, update_url_query};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct BoxFile;

const NAME: &str = "Box";
const FILE_FIELDS: &str = "authenticated_download_url,created_at,created_by,description,extension,is_download_available,name,representations,size";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:[^.]+\.)?(?P<service>app|ent)\.box\.com/s/(?P<shared_name>[^/?#]+)(?:/file/(?P<id>\d+))?",
    )
    .unwrap()
});
static POST_STREAM_DATA: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Box\.postStreamData\s*=").unwrap());
static CONFIG: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"Box\.config\s*=").unwrap());

/// `_match_valid_url`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_search_json(start, webpage, name, video_id)` (fatal).
fn search_json_fatal(start: &Regex, webpage: &str, name: &str, id: &str) -> Result<Value> {
    super::search_json(start, webpage, false)
        .flatten()
        .ok_or_else(|| {
            Error::Reported(format!(
                "[{NAME}] {id}: Unable to extract {name}; {}",
                crate::BUG_REPORTS_MESSAGE
            ))
        })
}

/// `traverse_obj(..., ('/app-api/enduserapp/shared-item', {dict}))`'s
/// `itemType`, then `str(itemID)`: a file's ID (`None` for other items).
fn shared_file_id(post_stream_data: &Value, shared_name: &str) -> Result<Option<String>> {
    let shared_item = post_stream_data
        .get("/app-api/enduserapp/shared-item")
        .and_then(Value::as_object);
    let Some(shared_item) = shared_item else {
        return Ok(None);
    };
    if shared_item.get("itemType").and_then(Value::as_str) != Some("file") {
        return Ok(None);
    }
    Ok(Some(match shared_item.get("itemID") {
        Some(Value::String(id)) => id.clone(),
        Some(Value::Number(id)) => id.to_string(),
        Some(_) => return Err(unsupported("item IDs that are not strings or integers")),
        None => return Err(super::clyp::key_error(shared_name, "itemID")),
    }))
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Box {what} are not supported natively"))
}

/// The value of a JSON object's key, or `KeyError` for the file's ID.
fn field<'a>(object: &'a Map<String, Value>, file_id: &str, key: &str) -> Result<&'a Value> {
    object
        .get(key)
        .ok_or_else(|| super::clyp::key_error(file_id, key))
}

/// A string-or-null field value as Python's `or None` on a string.
fn nonempty_string(value: Option<&Value>) -> Value {
    match value {
        Some(Value::String(text)) if !text.is_empty() => Value::String(text.clone()),
        _ => Value::Null,
    }
}

impl Extractor for BoxFile {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let service = found["service"].to_owned();
        let shared_name = found["shared_name"].to_owned();
        let url_file_id = found.name("id").map(|id| id.as_str().to_owned());

        let webpage = download_webpage(
            client,
            url,
            NAME,
            url_file_id.as_deref().unwrap_or(&shared_name),
        )?;
        let file_id = match url_file_id {
            Some(file_id) => file_id,
            None => {
                let post_stream_data = search_json_fatal(
                    &POST_STREAM_DATA,
                    &webpage,
                    "Box post-stream data",
                    &shared_name,
                )?;
                shared_file_id(&post_stream_data, &shared_name)?.ok_or_else(|| {
                    Error::Reported(format!("[{NAME}] The requested resource is not a file"))
                })?
            }
        };

        let config = search_json_fatal(&CONFIG, &webpage, "Box config", &file_id)?;
        let request_token = config
            .get("requestToken")
            .ok_or_else(|| super::clyp::key_error(&file_id, "requestToken"))?
            .clone();
        let shared_link = format!("https://{service}.box.com/s/{shared_name}");
        let shared_name_header = format!("sharedName={shared_name}");
        let token_body = json!({ "fileIDs": [file_id] }).to_string();
        let tokens = post_json_note_headers(
            client,
            &Url::parse(&format!(
                "https://{service}.box.com/app-api/enduserapp/elements/tokens"
            ))?,
            NAME,
            &file_id,
            "Downloading token JSON metadata",
            &[
                ("Content-Type", "application/json"),
                (
                    "X-Request-Token",
                    request_token.as_str().unwrap_or_default(),
                ),
                ("X-Box-EndUser-API", &shared_name_header),
            ],
            token_body,
        )?;
        let access_token = tokens
            .get(&file_id)
            .ok_or_else(|| super::clyp::key_error(&file_id, &file_id))?
            .get("read")
            .and_then(Value::as_str)
            .ok_or_else(|| super::clyp::key_error(&file_id, "read"))?
            .to_owned();

        let file_url = update_url_query(
            &Url::parse(&format!("https://api.box.com/2.0/files/{file_id}"))?,
            &[("fields", FILE_FIELDS)],
        );
        let bearer = format!("Bearer {access_token}");
        let box_api = format!("shared_link={shared_link}");
        let file = download_json_note_headers(
            client,
            &file_url,
            NAME,
            &file_id,
            "Downloading file JSON metadata",
            &[
                ("Authorization", &bearer),
                ("BoxApi", &box_api),
                ("X-Rep-Hints", "[dash]"),
            ],
        )?;
        let file = file
            .as_object()
            .ok_or_else(|| unsupported("file metadata that is not an object"))?;
        let title = field(file, &file_id, "name")?.clone();

        let mut formats = Vec::new();
        let representations = file
            .get("representations")
            .and_then(|value| value.get("entries"))
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        for representation in representations {
            if representation.get("representation").and_then(Value::as_str) != Some("dash") {
                continue;
            }
            let Some(template) = representation
                .get("content")
                .and_then(|content| content.get("url_template"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            let Ok(manifest) = Url::parse(&template.replace("{+asset_path}", "manifest.mpd"))
            else {
                continue;
            };
            let manifest = update_url_query(
                &manifest,
                &[
                    ("access_token", &access_token),
                    ("shared_link", &shared_link),
                ],
            );
            let query = manifest.query().unwrap_or_default().to_owned();
            for mut format in extract_mpd_fatal(client, &manifest, NAME, &file_id, false)?.formats {
                format
                    .extra
                    .insert("extra_param_to_segment_url".into(), query.clone().into());
                crate::info_order::mark_format_update(&mut format, "extra_param_to_segment_url");
                formats.push(format);
            }
        }

        let creator = file
            .get("created_by")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let timestamp = file
            .get("created_at")
            .and_then(Value::as_str)
            .and_then(crate::dates::parse_iso8601)
            .map_or(Value::Null, Value::from);
        Ok(Some(dict_info(
            url,
            (NAME, NAME),
            vec![
                ("id", file_id.clone().into()),
                ("title", title),
                ("formats", Value::Null),
                ("description", nonempty_string(file.get("description"))),
                (
                    "uploader",
                    creator.get("name").cloned().unwrap_or(Value::Null),
                ),
                ("timestamp", timestamp),
                (
                    "uploader_id",
                    creator.get("id").cloned().unwrap_or(Value::Null),
                ),
            ],
            formats,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_match_upstream_valid_url() {
        assert!(suitable(
            "https://mlssoccer.app.box.com/s/0evd2o3e08l60lr4ygukepvnkord1o1x/file/510727257538"
        ));
        assert!(suitable(
            "https://thejacksonlaboratory.ent.box.com/s/2x09dm6vcg6y28o0oox1so4l0t8wzt6l/file/1536173056065"
        ));
        assert!(suitable(
            "https://utexas.app.box.com/s/2x6vanv85fdl8j2eqlcxmv0gp1wvps6e"
        ));
        assert!(suitable(
            "https://app.box.com/s/2x6vanv85fdl8j2eqlcxmv0gp1wvps6e"
        ));
        assert!(!suitable(
            "https://example.com/s/2x6vanv85fdl8j2eqlcxmv0gp1wvps6e"
        ));
    }
}
