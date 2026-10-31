//! Upstream `IwaraIE` (`yt_dlp/extractor/iwara.py`): the anonymous
//! `api.iwara.tv/video/ID` metadata, its `fileUrl` file list (requested with
//! the `X-Version` SHA-1 of the file name, its `expires` and upstream's
//! secret) or its `embedUrl` hand-off. Account logins (`IwaraBaseIE`) and the
//! `IwaraUserIE`/`IwaraPlaylistIE` listings are not ported: credentials are an
//! error rather than an anonymous guess.
use super::{Extractor, direct, download_json_headers, follow_url};
use crate::{Error, ExtractResult, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::io::Read;
use std::sync::LazyLock;
use url::Url;

pub struct Iwara;

const NAME: &str = "iwara";
const KEY: &str = "Iwara";
const API: &str = "https://api.iwara.tv";
/// Upstream's `X-Version` secret for the file list.
const FILE_SECRET: &str = "mSvL05GfEmeEmsEYfGCnVpEjYgTJraJN";
/// `qualities(['preview', '360', '540', 'Source'])`.
const QUALITIES: [&str; 4] = ["preview", "360", "540", "Source"];

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:www\.|ecchi\.)?iwara\.tv/videos?/(?P<id>[a-zA-Z0-9]+)",
    )
    .unwrap()
});

/// `_match_valid_url` for the video URLs.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `_call_api(path, video_id, expected_status=lambda x: True)`: the JSON
/// body whatever its status, with upstream's impersonation attempt.
fn call_api(client: &Client, name: &'static str, path: &str, id: &str) -> Result<Value> {
    let url = Url::parse(&format!("{API}/{path}"))?;
    crate::screen::screen(&format!("[{name}] {id}: Downloading JSON metadata"));
    let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
        crate::screen::warning_once(&format!("[{name}] {message}"));
    });
    let statuses: Vec<u16> = (100..600).collect();
    let mut response = direct::request_expecting(client, &url, &[], &statuses)?;
    let page_url = response.url().clone();
    let headers = response.headers().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(&headers, body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    let text = super::decode_webpage(&body, &headers);
    crate::json::from_str(&text).map_err(|error| {
        Error::Reported(format!(
            "[{name}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// `_extract_formats(video_id, fileurl)`.
fn extract_formats(
    client: &Client,
    video_id: &str,
    file_url: &str,
) -> Result<Vec<crate::VideoFormat>> {
    let url = Url::parse(file_url)?;
    let Some(expires) = url
        .query_pairs()
        .find(|(key, _)| key == "expires")
        .map(|(_, value)| value.into_owned())
    else {
        return Err(super::clyp::key_error(video_id, "expires"));
    };
    let last = url
        .path()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    // https://github.com/yt-dlp/yt-dlp/issues/6549#issuecomment-1473771047
    let x_version: String =
        crate::sha1::digest(format!("{last}_{expires}_{FILE_SECRET}").as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
    let _impersonate = crate::impersonate::attempt(crate::impersonate::ANY, |message| {
        crate::screen::warning_once(&format!("[{NAME}] {message}"));
    });
    let files = download_json_headers(
        client,
        &url,
        NAME,
        video_id,
        &[("X-Version", x_version.as_str())],
    )?;
    let mut formats = Vec::new();
    for file in files.as_array().into_iter().flatten() {
        let name = file.get("name").and_then(Value::as_str);
        // `traverse_obj(..., ('src', ('view', 'download'), {proto_relative}))`.
        let Some(src) = file.get("src").and_then(Value::as_object) else {
            continue;
        };
        let Some(format_url) = ["view", "download"]
            .iter()
            .find_map(|key| src.get(*key).and_then(Value::as_str))
        else {
            continue;
        };
        let quality = name.map_or(-1, |name| {
            QUALITIES
                .iter()
                .position(|known| *known == name)
                .map_or(-1, |index| index as i64)
        });
        // The spec's key order: format_id, url, ext, quality, height.
        let mut fields = Vec::new();
        if let Some(name) = name {
            fields.push(("format_id", Value::from(name)));
        }
        fields.push(("url", Value::from(super::proto_relative(format_url))));
        if let Some(mime) = file.get("type").and_then(Value::as_str) {
            let ext = direct::mimetype2ext_or(mime, None);
            if !ext.is_empty() {
                fields.push(("ext", Value::from(ext)));
            }
        }
        fields.push(("quality", Value::from(quality)));
        if let Some(height) = name.and_then(|name| name.parse::<i64>().ok()) {
            fields.push(("height", Value::from(height)));
        }
        formats.push(super::dict_format(fields, NAME, video_id)?);
    }
    Ok(formats)
}

fn text_field(value: Option<&Value>) -> Option<Value> {
    value.filter(|value| !value.is_null()).cloned()
}

impl Extractor for Iwara {
    fn key(&self) -> &'static str {
        KEY
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        match self.extract_result(client, url)? {
            Some(ExtractResult::Video(info)) => Ok(Some(info)),
            _ => Ok(None),
        }
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = VALID_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let video_id = found["id"].to_owned();
        crate::screen::set_extractor(NAME);
        crate::screen::screen(&format!(
            "[{NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        // `_get_login_info()`: the login flow is not ported.
        if super::login::get_login_info(NAME, Some(NAME))?.is_some() {
            return Err(Error::UnsupportedHtml5(
                "Iwara account logins are not supported natively".into(),
            ));
        }
        let video = call_api(client, NAME, &format!("video/{video_id}"), &video_id)?;
        if let Some(message) = video.get("message").and_then(Value::as_str) {
            return Err(match message {
                "errors.privateVideo" => Error::Reported(format!(
                    "[{NAME}] {video_id}: {}",
                    super::login::login_required_message(
                        "Private video. Login if you have permissions to watch",
                        Some(NAME),
                        Some(super::login::Hint::Password),
                    )
                )),
                "errors.notFound" => Error::Reported(format!(
                    "[{NAME}] {video_id}: {}",
                    super::login::login_required_message(
                        "Video may need login to view",
                        Some(NAME),
                        Some(super::login::Hint::Password),
                    )
                )),
                other => Error::Reported(format!(
                    "[{NAME}] {video_id}: Iwara says: {other}; {}",
                    crate::BUG_REPORTS_MESSAGE
                )),
            });
        }

        let file_url = video
            .get("fileUrl")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty());
        let Some(file_url) = file_url else {
            return match video
                .get("embedUrl")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            {
                Some(embed) => Ok(Some(follow_url(url, embed.to_owned()))),
                None => Err(Error::Reported(format!(
                    "[{NAME}] {video_id}: This video is unplayable"
                ))),
            };
        };
        let formats = extract_formats(client, &video_id, file_url)?;

        let rating = video.get("rating").and_then(Value::as_str);
        let age_limit = if rating == Some("ecchi") { 18 } else { 0 };
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", Value::from(video_id.as_str())),
            ("age_limit", Value::from(age_limit)),
        ];
        let optional = [
            ("title", text_field(video.get("title"))),
            ("description", text_field(video.get("body"))),
            ("uploader", text_field(video.pointer("/user/name"))),
            ("uploader_id", text_field(video.pointer("/user/username"))),
            ("tags", {
                let tags: Vec<Value> = video
                    .get("tags")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|tag| tag.get("id").cloned())
                    .collect();
                (!tags.is_empty()).then(|| Value::from(tags))
            }),
            ("like_count", text_field(video.get("numLikes"))),
            ("view_count", text_field(video.get("numViews"))),
            ("comment_count", text_field(video.get("numComments"))),
            (
                "timestamp",
                video
                    .get("createdAt")
                    .and_then(Value::as_str)
                    .and_then(|date| crate::dates::unified_timestamp(date, false))
                    .map(Value::from),
            ),
            (
                "modified_timestamp",
                video
                    .get("updatedAt")
                    .and_then(Value::as_str)
                    .and_then(|date| crate::dates::unified_timestamp(date, false))
                    .map(Value::from),
            ),
            (
                "thumbnail",
                video.pointer("/file/id").and_then(Value::as_str).map(|id| {
                    Value::from(format!(
                        "https://files.iwara.tv/image/thumbnail/{id}/thumbnail-00.jpg"
                    ))
                }),
            ),
        ];
        for (key, value) in optional {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        fields.push(("formats", Value::Null));
        Ok(Some(ExtractResult::Video(super::dict_info(
            url,
            (NAME, KEY),
            fields,
            formats,
        ))))
    }
}

/// `IwaraUserIE` and `IwaraPlaylistIE`: videos per page.
const PER_PAGE: usize = 32;
const USER_KIND: &str = "iwara-user";
const PLAYLIST_KIND: &str = "iwara-playlist";
const USER_NAME: &str = "iwara:user";
const USER_KEY: &str = "IwaraUser";
const PLAYLIST_NAME: &str = "iwara:playlist";
const PLAYLIST_KEY: &str = "IwaraPlaylist";

static USER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?iwara\.tv/profile/(?P<id>[^/?#&]+)").unwrap()
});
static PLAYLIST_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:www\.)?iwara\.tv/playlist/(?P<id>[0-9a-f-]+)").unwrap()
});

/// `IwaraUserIE._VALID_URL`.
pub fn user_suitable(url: &str) -> bool {
    USER_URL.is_match(url)
}

/// `IwaraPlaylistIE._VALID_URL`.
pub fn playlist_suitable(url: &str) -> bool {
    PLAYLIST_URL.is_match(url)
}

/// `url_result('https://iwara.tv/video/ID')`.
fn url_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(
        crate::TRANSPARENT.into(),
        serde_json::json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": null,
            "entry": {"_type": "url", "url": target},
        }),
    );
    Ok(info)
}

/// `traverse_obj(videos, ('results', ..., 'id'))` as video entries.
fn video_entries(videos: &Value) -> Result<Vec<VideoInfo>> {
    videos
        .get("results")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|video| video.get("id").and_then(Value::as_str))
        .map(|id| url_entry(&format!("https://iwara.tv/video/{id}")))
        .collect()
}

/// Whether a playlist's pending state is an Iwara user's or playlist's.
pub(crate) fn is_pending_state(state: &Value) -> bool {
    state["kind"] == USER_KIND || state["kind"] == PLAYLIST_KIND
}

/// `OnDemandPagedList(functools.partial(self._entries, ...), 32)`: a page
/// fetch per requested page, page 0 of a playlist being the extraction's.
pub(crate) fn fetch_pages(
    client: &Client,
    state: Value,
    limit: Option<usize>,
    entries: &mut Vec<VideoInfo>,
) -> Result<Option<Value>> {
    let is_user = state["kind"] == USER_KIND;
    let playlist_id = state["playlist_id"].as_str().unwrap_or_default().to_owned();
    let user_id = state["user_id"].as_str().unwrap_or_default().to_owned();
    let first_page = state["first_page"].clone();
    let page_url = format!(
        "https://iwara.tv/{}/{playlist_id}",
        if is_user { "profile" } else { "playlist" }
    );
    let mut fetch = |pagenum: usize| -> Result<Vec<VideoInfo>> {
        let videos = if is_user {
            call_api(
                client,
                USER_NAME,
                &format!("videos?page={pagenum}&sort=date&user={user_id}&limit={PER_PAGE}"),
                &playlist_id,
            )?
        } else if pagenum == 0 {
            first_page.clone()
        } else {
            call_api(
                client,
                PLAYLIST_NAME,
                &format!("playlist/{playlist_id}?page={pagenum}&limit={PER_PAGE}"),
                &playlist_id,
            )?
        };
        video_entries(&videos)
    };
    let placeholder = || url_entry(&page_url);
    super::paged::fetch_requested(state, limit, entries, PER_PAGE, &mut fetch, &placeholder)
}

/// A playlist whose entries are pending (`playlist-dict`, lazily paged).
fn pending_playlist(
    page: &Url,
    (extractor, key): (&'static str, &'static str),
    id: &str,
    title: Option<String>,
    state: Value,
) -> ExtractResult {
    let mut keys = vec!["id"];
    if title.is_some() {
        keys.push("title");
    }
    let mut extra = serde_json::Map::new();
    extra.insert(
        crate::info_order::INFO_SHAPE.into(),
        format!(
            "playlist-dict;{}{}",
            keys.join(","),
            crate::info_order::GENERATOR
        )
        .into(),
    );
    ExtractResult::Playlist(crate::PlaylistInfo {
        id: id.to_owned(),
        title: title.unwrap_or_default(),
        webpage_url: page.clone(),
        extractor: extractor.into(),
        extractor_key: key.into(),
        nested: Vec::new(),
        pending: Some(crate::PendingEntries { state }),
        entries: Vec::new(),
        extra,
    })
}

pub struct IwaraUser;
pub struct IwaraPlaylist;

impl Extractor for IwaraUser {
    fn key(&self) -> &'static str {
        USER_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = USER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(USER_NAME);
        crate::screen::screen(&format!(
            "[{USER_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let user_info = call_api(
            client,
            USER_NAME,
            &format!("profile/{playlist_id}"),
            &playlist_id,
        )?;
        let user_id = user_info
            .pointer("/user/id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let title = user_info
            .pointer("/user/name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let state = serde_json::json!({
            "kind": USER_KIND,
            "playlist_id": playlist_id,
            "user_id": user_id,
            "random_access": true,
        });
        Ok(Some(pending_playlist(
            url,
            (USER_NAME, USER_KEY),
            &playlist_id,
            title,
            state,
        )))
    }
}

impl Extractor for IwaraPlaylist {
    fn key(&self) -> &'static str {
        PLAYLIST_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = PLAYLIST_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let playlist_id = found["id"].to_owned();
        crate::screen::set_extractor(PLAYLIST_NAME);
        crate::screen::screen(&format!(
            "[{PLAYLIST_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let first_page = call_api(
            client,
            PLAYLIST_NAME,
            &format!("playlist/{playlist_id}?page=0&limit={PER_PAGE}"),
            &playlist_id,
        )?;
        let title = first_page
            .pointer("/title/name")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let state = serde_json::json!({
            "kind": PLAYLIST_KIND,
            "playlist_id": playlist_id,
            "first_page": first_page,
            "random_access": true,
        });
        Ok(Some(pending_playlist(
            url,
            (PLAYLIST_NAME, PLAYLIST_KEY),
            &playlist_id,
            title,
            state,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn video_urls_match_upstream_valid_url() {
        assert!(suitable("https://www.iwara.tv/video/k2ayoueezfkx6gvq"));
        assert!(suitable("https://iwara.tv/video/1ywe1sbkqwumpdxz5/"));
        assert!(suitable("https://ecchi.iwara.tv/videos/blggmfno8ghl725bg"));
        assert!(!suitable("https://iwara.tv/profile/user792540/videos"));
        assert!(user_suitable("https://iwara.tv/profile/user792540/videos"));
        assert!(playlist_suitable(
            "https://iwara.tv/playlist/458e5486-36a4-4ac0-b233-7e9eef01025f"
        ));
    }
}
