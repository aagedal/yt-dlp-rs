//! Upstream `BandlabIE` (`yt_dlp/extractor/bandlab.py`): a track, post or
//! revision URL (or an embed's `id`) resolved through the BandLab web API's
//! post and revision records to the revision's mixdown file, as an untyped
//! audio result. The albums/collections playlists (`BandlabPlaylistIE`) are
//! not ported; post types other than revisions are reported as unsupported.
use super::clyp::float_or_none;
use super::streamable::int_or_none;
use super::{Extractor, download_json_note_headers};
use crate::{Error, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::sync::LazyLock;
use url::Url;

pub struct Bandlab;

const NAME: &str = "Bandlab";
const API: &str = "https://www.bandlab.com/api/v1.3";
const HEADERS: &[(&str, &str)] = &[
    ("accept", "application/json"),
    ("referer", "https://www.bandlab.com/"),
    ("x-client-id", "BandLab-Web"),
    ("x-client-version", "10.1.124"),
];

static TRACK_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?bandlab.com/(?P<url_type>track|post|revision)/(?P<id>[\da-f_-]+)",
    )
    .unwrap()
});
static EMBED_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.)?bandlab.com/(?P<url_type>embed)/\?(?:[^#]*&)?id=(?P<id>[\da-f-]+)",
    )
    .unwrap()
});

/// `_match_valid_url` over both of upstream's patterns.
pub fn suitable(url: &str) -> bool {
    TRACK_URL.is_match(url) || EMBED_URL.is_match(url)
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Bandlab {what} are not supported natively"))
}

/// `traverse_obj(qs, (key, 0))`: the first non-blank value of a query key.
fn query_value(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, value)| name == key && !value.is_empty())
        .map(|(_, value)| value.into_owned())
}

/// `_call_api(endpoint, asset_id, query=...)`.
fn call_api(
    client: &Client,
    endpoint: &str,
    asset_id: &str,
    query: &[(&str, &str)],
    note: &str,
) -> Result<Value> {
    let mut url = Url::parse(&format!("{API}/{endpoint}/{asset_id}"))?;
    if !query.is_empty() {
        url = super::update_url_query(&url, query);
    }
    download_json_note_headers(client, &url, NAME, asset_id, note, HEADERS)
}

/// `_parse_revision(revision_data, url)` with the upstream dict's key order.
fn parse_revision(revision: &Value, url: &Url) -> Result<VideoInfo> {
    let get = |path: &[&str]| -> Option<&Value> {
        path.iter().try_fold(revision, |value, key| value.get(*key))
    };
    let string = |path: &[&str]| get(path).and_then(Value::as_str).map(str::to_owned);
    let url_string =
        |path: &[&str]| string(path).and_then(|text| super::json_ld::url_or_none(&text));

    let webpage_url = get(&["id"])
        .filter(|id| !id.is_null())
        .map(|_| url.to_string());
    let id = string(&["revisionId"]).or_else(|| string(&["id"]));
    let media = url_string(&["mixdown", "file"])
        .ok_or_else(|| unsupported("revisions without a mixdown file"))?;
    let media_url = Url::parse(&media)?;
    // The mixdown is the result's only format (`url` in upstream's dict).
    let mut info = super::direct::with_format(
        super::direct::build_at(media_url.clone(), &media_url, "m4a".into()),
        "",
    );
    for format in &mut info.formats {
        format.ext = "m4a".into();
        format.format_id = String::new();
        format.vcodec = Some("none".into());
    }
    info.format_id = None;
    info.direct = None;
    info.webpage_url = url.clone();
    info.extractor = NAME.into();
    info.extractor_key = NAME.into();

    let mut keys = vec![
        "vcodec",
        "media_type",
        "extractor_key",
        "extractor",
        "webpage_url",
    ];
    crate::metadata_parser::set_field(&mut info, "vcodec", "none".into());
    crate::metadata_parser::set_field(&mut info, "media_type", "revision".into());
    crate::metadata_parser::set_field(&mut info, "extractor_key", NAME.into());
    crate::metadata_parser::set_field(&mut info, "extractor", NAME.into());

    let mut fields: Vec<(&str, Value)> = Vec::new();
    if let Some(webpage_url) = webpage_url {
        fields.push(("webpage_url", webpage_url.into()));
    }
    if let Some(id) = id {
        fields.push(("id", id.into()));
    }
    if let Some(title) = string(&["song", "name"]) {
        fields.push(("title", title.clone().into()));
        fields.push(("track", title.into()));
    }
    fields.push(("url", media.into()));
    if let Some(thumbnail) = url_string(&["song", "picture", "url"]) {
        fields.push(("thumbnail", thumbnail.into()));
    }
    if let Some(description) = string(&["description"]) {
        fields.push(("description", description.into()));
    }
    if let Some(uploader) = string(&["creator", "name"]) {
        fields.push(("uploader", uploader.into()));
    }
    if let Some(uploader_id) = string(&["creator", "username"]) {
        fields.push(("uploader_id", uploader_id.into()));
    }
    if let Some(timestamp) =
        string(&["createdOn"]).and_then(|date| crate::dates::parse_iso8601(&date))
    {
        fields.push(("timestamp", timestamp.into()));
    }
    let duration = float_or_none(get(&["mixdown", "duration"]));
    if !duration.is_null() {
        fields.push(("duration", duration));
    }
    for (key, path) in [
        ("view_count", "plays"),
        ("like_count", "likes"),
        ("comment_count", "comments"),
    ] {
        let value = int_or_none(get(&["counters", path]));
        if !value.is_null() {
            fields.push((key, value));
        }
    }
    let genres: Vec<Value> = revision
        .get("genres")
        .and_then(Value::as_array)
        .map(|genres| {
            genres
                .iter()
                .filter_map(|genre| genre.get("name")?.as_str())
                .map(|name| name.into())
                .collect()
        })
        .unwrap_or_default();
    if !genres.is_empty() {
        fields.push(("genres", Value::Array(genres)));
    }

    for (key, value) in fields {
        if !keys.contains(&key) {
            keys.push(key);
        }
        crate::metadata_parser::set_field(&mut info, key, value);
    }
    Ok(crate::info_order::mark_info(
        info,
        &format!("untyped-dict;{}", keys.join(",")),
    ))
}

impl Extractor for Bandlab {
    fn key(&self) -> &'static str {
        NAME
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let (display_id, url_type) = if let Some(found) = TRACK_URL.captures(url.as_str()) {
            (found["id"].to_owned(), found["url_type"].to_owned())
        } else if let Some(found) = EMBED_URL.captures(url.as_str()) {
            (found["id"].to_owned(), found["url_type"].to_owned())
        } else {
            return Ok(None);
        };
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));

        // `traverse_obj(qs, (('revId', 'id'), 0, any))`.
        let mut revision_id = query_value(url, "revId").or_else(|| query_value(url, "id"));
        if url_type == "revision" {
            revision_id = Some(display_id.clone());
        }
        let shared_key = query_value(url, "sharedKey");
        let mut revision_data = None;
        if revision_id.is_none() {
            let query: Vec<(&str, &str)> = shared_key
                .as_deref()
                .map(|key| vec![("sharedKey", key)])
                .unwrap_or_default();
            let post = call_api(
                client,
                "posts",
                &display_id,
                &query,
                "Downloading post data",
            )?;
            revision_id = post
                .get("revisionId")
                .and_then(Value::as_str)
                .or_else(|| post.get("revision")?.get("id")?.as_str())
                .map(str::to_owned);
            revision_data = post
                .get("revision")
                .filter(|revision| revision.is_object())
                .cloned();
            if revision_data.is_none() && revision_id.is_none() {
                // Upstream returns `_parse_video` / `_parse_track` here.
                let post_type = post.get("type").and_then(Value::as_str).unwrap_or("");
                return Err(unsupported(&format!(
                    "post type {post_type:?} (only revisions are ported)"
                )));
            }
        }
        let revision_id = revision_id.unwrap_or_default();
        let revision = match revision_data {
            Some(revision) => revision,
            None => call_api(
                client,
                "revisions",
                &revision_id,
                &[("edit", "false")],
                "Downloading revision data",
            )?,
        };
        parse_revision(&revision, url).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://www.bandlab.com/track/04b37e88dba24967b9dac8eb8567ff39_07d7f906fc96ee11b75e000d3a428fff"
        ));
        assert!(suitable(
            "https://www.bandlab.com/post/07d7f906-fc96-ee11-b75e-000d3a428fff"
        ));
        assert!(suitable(
            "https://www.bandlab.com/embed/?blur=false&id=014de0a4-7d82-ea11-a94c-0003ffd19c0f"
        ));
        assert!(!suitable(
            "https://www.bandlab.com/davesnothome69/albums/89b79ea6-de42-ed11-b495-00224845aac7"
        ));
    }
}
