//! Upstream `GoogleDriveIE` and `GoogleDriveFolderIE`
//! (`yt_dlp/extractor/googledrive.py`): a file's workspace-video playback
//! API (adaptive/progressive transcodes, title, duration, thumbnails and the
//! timed-text track list), the `drive.usercontent.google.com` source file
//! with its virus-scan confirmation form, the `<iframe>` embeds GenericIE
//! finds, and a folder's batch API listing as a playlist of file URLs.
use super::{Extractor, dict_format, dict_info};
use crate::{Error, ExtractResult, PlaylistInfo, Result, VideoInfo};
use regex::Regex;
use reqwest::blocking::{Client, Response};
use serde_json::{Map, Value, json};
use std::sync::LazyLock;
use url::Url;

pub struct GoogleDrive;
pub struct GoogleDriveFolder;

pub(crate) const NAME: &str = "GoogleDrive";
pub(crate) const KEY: &str = "GoogleDrive";
const FOLDER_NAME: &str = "GoogleDrive:Folder";
const FOLDER_KEY: &str = "GoogleDriveFolder";
const PLAYBACK_KEY: &str = "AIzaSyDVQw45DwoYh632gvsP5vPDqEKvb-Ywnb8";
/// `GoogleDriveFolderIE._BOUNDARY`.
const BOUNDARY: &str = "=====vc17a3rwnndj=====";
/// `GoogleDriveFolderIE._REQUEST` (`{folder_id}`, `{page_token}` and
/// `{key}` are formatted in unescaped).
const LIST_REQUEST: &str = "/drive/v2beta/files?openDrive=true&reason=102&syncType=0&errorRecovery=false&q=trashed%20%3D%20false%20and%20'{folder_id}'%20in%20parents&fields=kind%2CnextPageToken%2Citems(kind%2CmodifiedDate%2CmodifiedByMeDate%2ClastViewedByMeDate%2CfileSize%2Cowners(kind%2CpermissionId%2Cid)%2ClastModifyingUser(kind%2CpermissionId%2Cid)%2ChasThumbnail%2CthumbnailVersion%2Ctitle%2Cid%2CresourceKey%2Cshared%2CsharedWithMeDate%2CuserPermission(role)%2CexplicitlyTrashed%2CmimeType%2CquotaBytesUsed%2Ccopyable%2CfileExtension%2CsharingUser(kind%2CpermissionId%2Cid)%2Cspaces%2Cversion%2CteamDriveId%2ChasAugmentedPermissions%2CcreatedDate%2CtrashingUser(kind%2CpermissionId%2Cid)%2CtrashedDate%2Cparents(id)%2CshortcutDetails(targetId%2CtargetMimeType%2CtargetLookupStatus)%2Ccapabilities(canCopy%2CcanDownload%2CcanEdit%2CcanAddChildren%2CcanDelete%2CcanRemoveChildren%2CcanShare%2CcanTrash%2CcanRename%2CcanReadTeamDrive%2CcanMoveTeamDriveItem)%2Clabels(starred%2Ctrashed%2Crestricted%2Cviewed))%2CincompleteSearch&appDataFilter=NO_APP_DATA&spaces=drive&pageToken={page_token}&maxResults=50&supportsTeamDrives=true&includeItemsFromAllDrives=true&corpora=default&orderBy=folder%2Ctitle_natural%20asc&retryCount=0&key={key} HTTP/1.1";

static VALID_URL: LazyLock<Regex> = LazyLock::new(|| {
    crate::extractor::url_regex(
        r"^https?://(?:(?:docs|drive|drive\.usercontent)\.google\.com/(?:(?:uc|open|download)\?.*?id=|file/d/)|video\.google\.com/get_player\?.*?docid=)(?P<id>[a-zA-Z0-9_-]{28,})",
    )
    .unwrap()
});
static FOLDER_URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^https?://(?:docs|drive)\.google\.com/drive/folders/(?P<id>[\w-]{28,})").unwrap()
});
/// `_extract_embed_urls`' pattern (only its first match is used).
static EMBED: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"<iframe[^>]+src="https?://(?:video\.google\.com/get_player\?.*?docid=|(?:docs|drive)\.google\.com/file/d/)(?P<id>[a-zA-Z0-9_-]{28,})"#,
    )
    .unwrap()
});
static API_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#""(\w{39})""#).unwrap());

/// `GoogleDriveIE._match_id`.
pub fn suitable(url: &str) -> bool {
    VALID_URL.is_match(url)
}

/// `GoogleDriveFolderIE._match_id`.
pub fn folder_suitable(url: &str) -> bool {
    FOLDER_URL.is_match(url)
}

/// `_match_id`'s ID of a suitable URL.
pub(crate) fn temp_id(url: &str) -> Option<String> {
    VALID_URL.captures(url).map(|found| found["id"].to_owned())
}

/// `_extract_embed_urls`: the first Drive player or file `<iframe>` as a
/// `drive.google.com/file/d/ID` URL.
pub(crate) fn embed_urls(_url: &Url, webpage: &str) -> Vec<Url> {
    EMBED
        .captures(webpage)
        .and_then(|found| {
            Url::parse(&format!("https://drive.google.com/file/d/{}", &found["id"])).ok()
        })
        .into_iter()
        .collect()
}

fn unsupported(what: &str) -> Error {
    Error::UnsupportedHtml5(format!("Google Drive {what} are not supported natively"))
}

/// `traverse_obj(value, {int})`: Python `int` values only (`bool` included).
fn int_type(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Number(number) if number.is_i64() || number.is_u64() => {
            Some(Value::Number(number.clone()))
        }
        Value::Bool(flag) => Some(Value::Bool(*flag)),
        _ => None,
    }
}

/// `('itag', {int}, {str_or_none})`.
fn itag_id(value: Option<&Value>) -> Option<String> {
    match int_type(value)? {
        Value::Bool(true) => Some("True".into()),
        Value::Bool(false) => Some("False".into()),
        number => Some(number.to_string()),
    }
}

/// `int_or_none` (`int(v)`: integers, truncated floats, integer strings).
fn int_or_none(value: Option<&Value>) -> Option<Value> {
    match value? {
        Value::Bool(flag) => Some(i64::from(*flag).into()),
        other => match super::streamable::int_or_none(Some(other)) {
            Value::Null => None,
            number => Some(number),
        },
    }
}

/// `mimetype2ext(mime)` of a string.
fn mimetype2ext(value: Option<&Value>) -> Option<Value> {
    const MISSING: &str = "\u{0}";
    let ext = super::direct::mimetype2ext_or(value?.as_str()?, Some(MISSING));
    (ext != MISSING).then(|| ext.into())
}

/// `url_or_none(v['url'])` as the filter of a list item.
fn has_url(item: &Value) -> bool {
    item.get("url")
        .and_then(Value::as_str)
        .is_some_and(|url| super::json_ld::url_or_none(url).is_some())
}

/// One transcode's format dict.
fn transcode_format(transcode: &Value, video_id: &str) -> Result<crate::VideoFormat> {
    let mut fields: Vec<(&str, Value)> = vec![("url", transcode["url"].clone())];
    if let Some(itag) = itag_id(transcode.get("itag")) {
        fields.push(("format_id", itag.into()));
    }
    let metadata = transcode
        .get("transcodeMetadata")
        .filter(|value| value.is_object());
    if let Some(metadata) = metadata {
        let get = |key: &str| metadata.get(key);
        for (key, value) in [
            ("ext", mimetype2ext(get("mimeType"))),
            ("width", int_or_none(get("width"))),
            ("height", int_or_none(get("height"))),
            ("fps", int_or_none(get("videoFps"))),
            ("filesize", int_or_none(get("contentLength"))),
        ] {
            if let Some(value) = value {
                fields.push((key, value));
            }
        }
        for (key, source) in [
            ("vcodec", "videoCodecString"),
            ("acodec", "audioCodecString"),
        ] {
            let codec = match get(source) {
                Some(Value::String(codec)) => codec.clone(),
                _ => "none".into(),
            };
            fields.push((key, codec.into()));
        }
    }
    fields.push((
        "downloader_options",
        json!({"http_chunk_size": 10_i64 << 20}),
    ));
    dict_format(fields, NAME, video_id)
}

/// `thumbnails`: URL-valued items with their `ext`, `width` and `height`;
/// `None` when the key is not a list.
fn thumbnails(video_info: &Value) -> Option<Vec<Value>> {
    let items = video_info.get("thumbnails")?.as_array()?;
    let mut thumbnails = Vec::new();
    for item in items.iter().filter(|item| has_url(item)) {
        let mut thumbnail = Map::new();
        thumbnail.insert("url".into(), item["url"].clone());
        for (key, value) in [
            ("ext", mimetype2ext(item.get("mimeType"))),
            ("width", int_type(item.get("width"))),
            ("height", int_type(item.get("height"))),
        ] {
            if let Some(value) = value {
                thumbnail.insert(key.into(), value);
            }
        }
        thumbnails.push(Value::Object(thumbnail));
    }
    Some(thumbnails)
}

/// A non-fatal request's HTTP error as upstream's warning; other errors
/// stay errors.
fn request_warning(
    result: Result<Response>,
    video_id: &str,
    errnote: &str,
) -> Result<Option<Response>> {
    match result {
        Ok(response) => Ok(Some(response)),
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: {errnote}: {}",
                crate::upstream_error_text(&error)
            ));
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

/// `urlh.headers.get_content_disposition()`: a non-empty disposition.
fn has_content_disposition(response: &Response) -> bool {
    response
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .is_some_and(|value| {
            let text = String::from_utf8_lossy(value.as_bytes()).into_owned();
            !text.split(';').next().unwrap_or_default().trim().is_empty()
        })
}

/// `_get_subtitles(video_id, video_info)` (only when subtitles are
/// requested): the timed-text list's tracks in each listed format.
fn subtitles(client: &Client, video_id: &str, video_info: &Value) -> Result<crate::Subtitles> {
    use std::io::Read;
    let mut subtitles = crate::Subtitles::default();
    let Some(base) = video_info
        .pointer("/timedTextDetails/timedTextBaseUrl")
        .and_then(Value::as_str)
        .and_then(super::json_ld::url_or_none)
    else {
        return Ok(subtitles);
    };
    let Ok(base) = Url::parse(&base) else {
        return Err(unsupported("timed-text URLs that are not absolute"));
    };
    let list_url = super::update_url_query(
        &base,
        &[
            ("hl", "en-US"),
            ("type", "list"),
            ("tlangs", "1"),
            ("v", video_id),
            ("vssids", "1"),
        ],
    );
    crate::screen::screen(&format!("[{NAME}] {video_id}: Downloading subtitles XML"));
    let Some(mut response) = request_warning(
        super::direct::request(client, &list_url, None),
        video_id,
        "Unable to download XML",
    )?
    else {
        return Ok(subtitles);
    };
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    let text = super::decode_webpage(&body, response.headers());
    let document = match roxmltree::Document::parse(text.trim_start_matches('\u{feff}')) {
        Ok(document) => document,
        Err(error) => {
            crate::screen::warning(&format!(
                "[{NAME}] {video_id}: Failed to parse XML: {error}"
            ));
            return Ok(subtitles);
        }
    };
    let children: Vec<_> = document
        .root_element()
        .children()
        .filter(|node| node.is_element())
        .collect();
    let formats: Vec<&str> = children
        .iter()
        .filter(|node| node.tag_name().name() == "format")
        .filter_map(|node| node.attribute("fmt_code"))
        .collect();
    for track in children
        .iter()
        .filter(|node| node.tag_name().name() == "track")
    {
        let Some(language) = track.attribute("lang_code").filter(|code| !code.is_empty()) else {
            continue;
        };
        let mut tracks = Vec::new();
        for format in &formats {
            let mut query = vec![
                ("hl", "en-US"),
                ("v", video_id),
                ("type", "track"),
                ("lang", language),
                ("fmt", *format),
            ];
            if let Some(kind) = track.attribute("kind") {
                query.push(("kind", kind));
            }
            tracks.push(crate::SubtitleFormat {
                name: track.attribute("lang_original").map(str::to_owned),
                url: super::update_url_query(&base, &query),
                ext: (*format).to_owned(),
                protocol: None,
                fragments: Vec::new(),
                extra: Default::default(),
            });
        }
        match subtitles
            .entries
            .iter_mut()
            .find(|(key, _)| key.as_deref() == Some(language))
        {
            Some((_, existing)) => existing.extend(tracks),
            None => subtitles.entries.push((Some(language.to_owned()), tracks)),
        }
    }
    Ok(subtitles)
}

/// The source format of a response with a Content-Disposition (its final
/// URL, so redirect cookies apply), naming an untitled file after it.
fn source_format(
    response: &Response,
    title: &mut Option<String>,
    video_id: &str,
) -> Result<crate::VideoFormat> {
    if title.is_none() {
        *title = super::direct::disposition_filename(response).filter(|name| !name.is_empty());
    }
    let ext = title
        .as_deref()
        .and_then(super::direct::determine_ext)
        .unwrap_or_else(|| "mp4".into())
        .to_lowercase();
    dict_format(
        vec![
            ("url", response.url().as_str().into()),
            ("ext", ext.into()),
            ("format_id", "source".into()),
            ("quality", 1.into()),
        ],
        NAME,
        video_id,
    )
}

/// The source file: its download URL, else the virus-scan confirmation
/// page's `download-form` (posted empty), else the page's error caption as
/// a warning.
fn source(
    client: &Client,
    video_id: &str,
    title: &mut Option<String>,
) -> Result<Option<crate::VideoFormat>> {
    use std::io::Read;
    let source_url = Url::parse(&format!(
        "https://drive.usercontent.google.com/download?id={video_id}&export=download&confirm=t"
    ))?;
    crate::screen::screen(&format!("[{NAME}] {video_id}: Requesting source file"));
    let Some(mut response) = request_warning(
        super::direct::request(client, &source_url, None),
        video_id,
        "Unable to request source file",
    )?
    else {
        return Ok(None);
    };
    if has_content_disposition(&response) {
        return source_format(&response, title, video_id).map(Some);
    }
    let mut body = Vec::new();
    if let Err(error) = response.read_to_end(&mut body) {
        crate::screen::warning(&format!(
            "[{NAME}] {video_id}: Error reading response: {error}"
        ));
        return Ok(None);
    }
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_page(video_id, None, None);
    crate::screen::page_read(response.url(), &body)?;
    let page = super::decode_webpage(&body, response.headers());
    let form = crate::html_elements::element_html_by_id("download-form", &page)
        .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?;
    let action = form
        .as_deref()
        .map(crate::html_elements::extract_attributes)
        .and_then(|attributes| {
            attributes
                .get("action")
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .filter(|action| !action.is_empty());
    let Some(action) = action else {
        let caption = |class: &str| -> Result<Option<String>> {
            Ok(
                crate::html_elements::elements_text_and_html_by_class(class, &page)
                    .map_err(|error| Error::Reported(format!("[{NAME}] {video_id}: {error}")))?
                    .into_iter()
                    .next()
                    .map(|(text, _)| text)
                    .filter(|text| !text.is_empty()),
            )
        };
        let message = match caption("uc-error-subcaption")? {
            Some(text) => text,
            None => caption("uc-error-caption")?
                .unwrap_or_else(|| "unable to extract confirmation code".into()),
        };
        crate::screen::warning(&format!("[{NAME}] {message}"));
        return Ok(None);
    };
    let confirmed = Url::parse(&action)
        .map_err(|_| unsupported("confirmation forms without an absolute action URL"))?;
    crate::screen::screen(&format!(
        "[{NAME}] {video_id}: Requesting confirmed source file"
    ));
    let Some(response) = request_warning(
        super::direct::post(client, &confirmed, &[], String::new()),
        video_id,
        "Unable to request confirmed source file",
    )?
    else {
        return Ok(None);
    };
    if has_content_disposition(&response) {
        return source_format(&response, title, video_id).map(Some);
    }
    Ok(None)
}

impl Extractor for GoogleDrive {
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
        let playback = Url::parse(&format!(
            "https://content-workspacevideo-pa.googleapis.com/v1/drive/media/{video_id}/playback?key={PLAYBACK_KEY}"
        ))?;
        let video_info = super::download_json_note_headers(
            client,
            &playback,
            NAME,
            &video_id,
            "Downloading video webpage",
            &[("Referer", "https://drive.google.com/")],
        )?;
        let mut formats = Vec::new();
        for kind in ["adaptiveTranscodes", "progressiveTranscodes"] {
            let Some(transcodes) = video_info
                .pointer(&format!("/mediaStreamingData/formatStreamingData/{kind}"))
                .and_then(Value::as_array)
            else {
                continue;
            };
            for transcode in transcodes.iter().filter(|item| has_url(item)) {
                formats.push(transcode_format(transcode, &video_id)?);
            }
        }
        let mut title = video_info
            .pointer("/mediaMetadata/title")
            .and_then(Value::as_str)
            .map(str::to_owned);
        if let Some(format) = source(client, &video_id, &mut title)? {
            formats.push(format);
        }
        let mut fields: Vec<(&str, Value)> = vec![
            ("id", video_id.clone().into()),
            ("title", title.map_or(Value::Null, Value::from)),
        ];
        if let Some(duration) = video_info
            .pointer("/mediaMetadata/duration")
            .and_then(Value::as_str)
            .and_then(crate::parse_duration)
        {
            fields.push((
                "duration",
                match duration {
                    crate::PyNumber::Int(value) => value.into(),
                    crate::PyNumber::Float(value) => value.into(),
                },
            ));
        }
        // Processing drops an empty list.
        if let Some(thumbnails) = thumbnails(&video_info).filter(|list| !list.is_empty()) {
            fields.push(("thumbnails", Value::Array(thumbnails)));
        }
        fields.push(("formats", Value::Null));
        fields.push(("subtitles", Value::Null));
        let subtitles = if super::subtitles_requested() {
            subtitles(client, &video_id, &video_info)?
        } else {
            crate::Subtitles::default()
        };
        let mut info = dict_info(url, (NAME, KEY), fields, formats);
        if let Some(Value::String(shape)) = info.extra.get_mut(crate::info_order::INFO_SHAPE) {
            shape.push_str(";thumbnail=url,ext,width,height;subtitle=url,name,ext");
        }
        info.subtitles = subtitles;
        Ok(Some(info))
    }
}

/// `GoogleDriveFolderIE._DATA % request`.
fn batch_body(request: &str) -> String {
    format!(
        "--{BOUNDARY}\ncontent-type: application/http\ncontent-transfer-encoding: binary\n\nGET {request}\n\n--{BOUNDARY}\n"
    )
}

/// `_search_json('', response, 'api response', ...)`: the first `{` to the
/// last `}`, its first JSON value. `None` (after upstream's warning) when a
/// non-fatal response has none.
fn api_json(response: &str, folder_id: &str, fatal: bool) -> Result<Option<Value>> {
    let candidate = response.find('{').and_then(|start| {
        response
            .rfind('}')
            .filter(|end| *end > start)
            .map(|end| &response[start..=end])
    });
    let Some(candidate) = candidate else {
        if fatal {
            return Err(Error::Reported(format!(
                "[{FOLDER_NAME}] {folder_id}: Unable to extract api response; {}",
                crate::BUG_REPORTS_MESSAGE
            )));
        }
        crate::screen::warning(&format!(
            "[{FOLDER_NAME}] unable to extract api response; {}",
            crate::BUG_REPORTS_MESSAGE
        ));
        return Ok(None);
    };
    match serde_json::Deserializer::from_str(&crate::json::replace_lone_surrogates(candidate))
        .into_iter::<Value>()
        .next()
    {
        Some(Ok(value)) => Ok(Some(value)),
        _ if fatal => Err(Error::Reported(format!(
            "[{FOLDER_NAME}] {folder_id}: Unable to extract api response - Failed to parse JSON"
        ))),
        _ => {
            crate::screen::warning(&format!(
                "[{FOLDER_NAME}] {folder_id}: Unable to extract api response - Failed to parse JSON"
            ));
            Ok(None)
        }
    }
}

/// `_call_api(folder_id, key, data, fatal=...)`: the batch API's response
/// JSON (`{}` without one when not fatal).
fn call_api(
    client: &Client,
    folder_id: &str,
    key: &str,
    data: String,
    fatal: bool,
) -> Result<Value> {
    use std::io::Read;
    let url = super::update_url_query(
        &Url::parse("https://clients6.google.com/batch/drive/v2beta")?,
        &[
            ("$ct", &format!("multipart/mixed; boundary=\"{BOUNDARY}\"")),
            ("key", key),
        ],
    );
    crate::screen::screen(&format!("[{FOLDER_NAME}] {folder_id}: Downloading webpage"));
    let response = super::direct::post(
        client,
        &url,
        &[
            ("Content-Type", "text/plain;charset=UTF-8;"),
            ("Origin", "https://drive.google.com"),
        ],
        data.clone(),
    );
    let mut response = match response {
        Ok(response) => response,
        Err(error @ (Error::Http(_) | Error::HttpRetryAfter { .. })) => {
            if fatal {
                return Err(Error::ExtractorRequest {
                    extractor: FOLDER_NAME,
                    id: folder_id.to_owned(),
                    what: "webpage",
                    source: Box::new(error),
                });
            }
            // Upstream then searches `False` and fails with a TypeError.
            return Err(unsupported("folder info requests that fail"));
        }
        Err(error) => return Err(error),
    };
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(Error::ExtractionRead)?;
    let body = super::decode_content(response.headers(), body).map_err(Error::ExtractionRead)?;
    crate::screen::expect_post_page(folder_id, data.as_bytes());
    crate::screen::page_read(response.url(), &body)?;
    let text = super::decode_webpage(&body, response.headers());
    Ok(api_json(&text, folder_id, fatal)?.unwrap_or_else(|| Value::Object(Map::new())))
}

/// `url_result(f'https://drive.google.com/file/d/{id}', GoogleDriveIE)`.
fn file_entry(target: &str) -> Result<VideoInfo> {
    let url = Url::parse(target)?;
    let mut info = super::direct::build_at(url.clone(), &url, String::new());
    info.formats.clear();
    info.direct = None;
    info.extra.insert(
        crate::TRANSPARENT.into(),
        json!({
            "url": target,
            "force_videoid": null,
            "fields": {},
            "ie_key": KEY,
            "extractor": NAME,
            "entry": {"ie_key": KEY, "_type": "url", "url": target},
        }),
    );
    Ok(info)
}

/// Python `str()` of an item ID formatted into a URL.
fn id_text(value: &Value) -> Result<String> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Number(number) if number.is_i64() || number.is_u64() => Ok(number.to_string()),
        _ => Err(unsupported("folder items whose IDs are not strings")),
    }
}

impl Extractor for GoogleDriveFolder {
    fn key(&self) -> &'static str {
        FOLDER_KEY
    }

    fn extract(&self, _client: &Client, _url: &Url) -> Result<Option<VideoInfo>> {
        Ok(None)
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        let Some(found) = FOLDER_URL.captures(url.as_str()) else {
            return Ok(None);
        };
        let folder_id = found["id"].to_owned();
        crate::screen::set_extractor(FOLDER_NAME);
        crate::screen::screen(&format!(
            "[{FOLDER_NAME}] Extracting URL: {}",
            crate::screen::extracting_url(url.as_str())
        ));
        let webpage = super::download_webpage(client, url, FOLDER_NAME, &folder_id)?;
        let key = API_KEY
            .captures(&webpage)
            .map(|found| found[1].to_owned())
            .ok_or_else(|| {
                Error::Reported(format!(
                    "[{FOLDER_NAME}] {folder_id}: Unable to extract key; {}",
                    crate::BUG_REPORTS_MESSAGE
                ))
            })?;
        let folder_info = call_api(
            client,
            &folder_id,
            &key,
            batch_body(&format!("/drive/v2beta/files/{folder_id} HTTP/1.1")),
            false,
        )?;
        let title = folder_info.get("title").cloned().unwrap_or(Value::Null);
        let title = match title {
            Value::Null => None,
            Value::String(title) => Some(title),
            _ => return Err(unsupported("folder titles that are not strings")),
        };
        // `_get_folder_items` under `orderedSet`.
        let mut urls: Vec<String> = Vec::new();
        let mut page_token = Some(String::new());
        while let Some(token) = page_token {
            let request = LIST_REQUEST
                .replace("{folder_id}", &folder_id)
                .replace("{page_token}", &token)
                .replace("{key}", &key);
            let page = call_api(client, &folder_id, &key, batch_body(&request), true)?;
            let Some(items) = page.get("items") else {
                return Err(Error::Reported(format!(
                    "[{FOLDER_NAME}] {folder_id}: KeyError('items')"
                )));
            };
            let Some(items) = items.as_array() else {
                return Err(unsupported("folder listings whose items are not a list"));
            };
            for item in items {
                let Some(id) = item.get("id") else {
                    return Err(Error::Reported(format!(
                        "[{FOLDER_NAME}] {folder_id}: KeyError('id')"
                    )));
                };
                let target = format!("https://drive.google.com/file/d/{}", id_text(id)?);
                if !urls.contains(&target) {
                    urls.push(target);
                }
            }
            page_token = match page.get("nextPageToken") {
                None | Some(Value::Null) => None,
                Some(Value::String(token)) => Some(token.clone()),
                Some(_) => return Err(unsupported("page tokens that are not strings")),
            };
        }
        let entries = urls
            .iter()
            .map(|target| file_entry(target))
            .collect::<Result<Vec<_>>>()?;
        let mut extra = Map::new();
        extra.insert(
            crate::info_order::INFO_SHAPE.into(),
            if title.is_some() {
                "playlist-dict;id,title"
            } else {
                "playlist-dict;id"
            }
            .into(),
        );
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: folder_id,
            title: title.unwrap_or_default(),
            webpage_url: url.clone(),
            extractor: FOLDER_NAME.into(),
            extractor_key: FOLDER_KEY.into(),
            nested: Vec::new(),
            pending: None,
            entries,
            extra,
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_follow_upstream_valid_url() {
        assert!(suitable(
            "https://drive.google.com/file/d/0ByeS4oOUV-49Zzh4R1J6R09zazQ/edit?pli=1"
        ));
        assert!(suitable(
            "https://drive.google.com/uc?id=0B2fjwgkl1A_CX083Tkowdmt6d28"
        ));
        assert!(suitable(
            "https://drive.google.com/open?id=0B2fjwgkl1A_CX083Tkowdmt6d28"
        ));
        assert!(suitable(
            "https://drive.usercontent.google.com/download?id=0ByeS4oOUV-49Zzh4R1J6R09zazQ"
        ));
        assert!(suitable(
            "https://video.google.com/get_player?ps=docs&docid=0ByeS4oOUV-49Zzh4R1J6R09zazQ"
        ));
        assert_eq!(
            temp_id("https://drive.google.com/file/d/1ENcQ_jeCuj7y19s66_Ou9dRP4GKGsodiDQ/edit")
                .as_deref(),
            Some("1ENcQ_jeCuj7y19s66_Ou9dRP4GKGsodiDQ")
        );
        assert!(!suitable("https://drive.google.com/file/d/short/view"));
        assert!(folder_suitable(
            "https://drive.google.com/drive/folders/1dQ4sx0-__Nvg65rxTSgQrl7VyW_FZ9QI"
        ));
        assert!(!folder_suitable(
            "https://drive.google.com/drive/folders/short"
        ));
    }

    #[test]
    fn embeds_yield_the_first_drive_iframe() {
        let page = Url::parse("https://example.test/").unwrap();
        let html = r#"<iframe width="1" src="https://docs.google.com/file/d/0ByeS4oOUV-49Zzh4R1J6R09zazQ/preview"></iframe><iframe src="https://drive.google.com/file/d/1RAGWRgzn85TXCaCk4gxnwF6TGUaZatzE/preview"></iframe>"#;
        assert_eq!(
            embed_urls(&page, html),
            vec![
                Url::parse("https://drive.google.com/file/d/0ByeS4oOUV-49Zzh4R1J6R09zazQ").unwrap()
            ]
        );
        assert!(
            embed_urls(
                &page,
                "<iframe src='https://drive.google.com/file/d/0ByeS4oOUV-49Zzh4R1J6R09zazQ'>"
            )
            .is_empty()
        );
    }

    #[test]
    fn api_responses_take_the_first_json_object() {
        let response = "--b\n\nHTTP/1.1 200 OK\n\n{\"title\": \"Forrest\"}\n--b--\n";
        assert_eq!(
            api_json(response, "id", true).unwrap(),
            Some(json!({"title": "Forrest"}))
        );
    }
}
