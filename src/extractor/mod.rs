mod behindkink;
mod dash;
pub(crate) mod direct;
mod ebaumsworld;
mod f4m;
mod fczenit;
mod filmweb;
mod generic_urls;
mod glide;
mod godtube;
mod historicfilms;
pub(crate) mod hls;
pub(crate) mod html5;
mod hypem;
pub(crate) mod ism;
mod json_ld;
mod jwplatform;
mod jwplayer;
mod kvs;
mod livestreamfails;
mod megaphone;
mod oneplace;
mod photobucket;
mod qingting;
mod reverbnation;
mod rss;
mod smil;
mod streamable;
mod sztvhu;
mod theintercept;
mod thisamericanlife;
mod twentythreevideo;
mod xspf;
mod youtube;

pub use behindkink::{BehindKink, suitable as behindkink_suitable};
pub use direct::DirectHttp;
pub(crate) use direct::from_response as direct_media_info;
pub use ebaumsworld::{EbaumsWorld, suitable as ebaumsworld_suitable};
pub(crate) use f4m::fix_ampersands as fix_xml_ampersands;
pub use fczenit::{Fczenit, suitable as fczenit_suitable};
pub use filmweb::{Filmweb, suitable as filmweb_suitable};
pub use glide::{Glide, suitable as glide_suitable};
pub use godtube::{GodTube, suitable as godtube_suitable};
pub use historicfilms::{HistoricFilms, suitable as historicfilms_suitable};
pub use hls::set_split_discontinuity as hls_split_discontinuity;
pub use html5::GenericHttp;
pub use hypem::{Hypem, suitable as hypem_suitable};
pub use jwplatform::{JwPlatform, suitable as jwplatform_suitable};
pub use livestreamfails::{Livestreamfails, suitable as livestreamfails_suitable};
pub use megaphone::{Megaphone, suitable as megaphone_suitable};
pub use oneplace::{OnePlacePodcast, suitable as oneplace_suitable};
pub use photobucket::{Photobucket, suitable as photobucket_suitable};
pub use qingting::{QingTing, suitable as qingting_suitable};
pub use reverbnation::{ReverbNation, suitable as reverbnation_suitable};
pub use streamable::{Streamable, suitable as streamable_suitable};
pub use sztvhu::{SztvHu, suitable as sztvhu_suitable};
pub use theintercept::{TheIntercept, suitable as theintercept_suitable};
pub use thisamericanlife::{ThisAmericanLife, suitable as thisamericanlife_suitable};
pub use twentythreevideo::{TwentyThreeVideo, suitable as twentythreevideo_suitable};
pub use youtube::{Youtube, YoutubeOptions, suitable as youtube_suitable};

use crate::{ExtractResult, Result, VideoInfo};
use reqwest::blocking::Client;
use url::Url;

/// Extractors return None when they do not recognize a URL/response.
/// A result may expose candidate formats; YoutubeDl resolves selection before returning it.
/// Register site extractors before the generic direct-media fallback.
pub trait Extractor: Send + Sync {
    fn key(&self) -> &'static str;
    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>>;

    /// Override to support playlists. Existing video-only extractors need no changes.
    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        Ok(self.extract(client, url)?.map(ExtractResult::Video))
    }
}

/// `[EXTRACTOR] ID: Unable to download WHAT: ERR (caused by REPR)`.
pub(crate) fn request_error_text(
    extractor: &str,
    id: &str,
    what: &str,
    error: &crate::Error,
) -> String {
    let text = crate::upstream_error_text(error);
    let status = match error {
        crate::Error::Http(source) | crate::Error::HttpRetryAfter { source, .. } => source.status(),
        _ => None,
    };
    let cause = match status {
        Some(_) => format!("<{}>", text.replacen("HTTP Error ", "HTTPError ", 1)),
        None => format!("TransportError({})", crate::batch::py_repr(&text)),
    };
    format!("[{extractor}] {id}: Unable to download {what}: {text} (caused by {cause})")
}

/// Upstream `_download_json(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// JSON metadata`, the request, and the parsed body.
pub(crate) fn download_json(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<serde_json::Value> {
    download_json_note(client, url, extractor, id, "Downloading JSON metadata")
}

/// [`download_json`] with its `note=` (the error note stays the default).
pub(crate) fn download_json_note(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
) -> Result<serde_json::Value> {
    download_json_transformed(
        client,
        url,
        extractor,
        id,
        note,
        &|text| Ok(text.to_owned()),
    )
}

/// [`download_json_note`] with `transform_source` applied to the body.
pub(crate) fn download_json_transformed(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
    transform: &dyn Fn(&str) -> Result<String>,
) -> Result<serde_json::Value> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "JSON metadata",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request(client, url, None).map_err(wrap)?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    let text = transform(&String::from_utf8_lossy(&body))?;
    serde_json::from_str(&text).map_err(|error| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse JSON  (caused by {error})"
        ))
    })
}

/// Upstream `update_url_query(url, query)`: the query decoded, updated and
/// re-encoded like `urlencode`.
pub(crate) fn update_url_query(url: &Url, updates: &[(&str, &str)]) -> Url {
    let mut pairs: Vec<(String, String)> = url
        .query_pairs()
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    for (key, value) in updates {
        pairs.retain(|(existing, _)| existing != key);
        pairs.push(((*key).to_owned(), (*value).to_owned()));
    }
    let mut url = url.clone();
    let query = url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(&pairs)
        .finish();
    url.set_query((!query.is_empty()).then_some(query.as_str()));
    url
}

/// Upstream `_download_webpage(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// webpage`, the request (`--dump-pages`/`--write-pages` like other
/// pages), and the body as UTF-8.
pub(crate) fn download_webpage(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<String> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: Downloading webpage"));
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "webpage",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request(client, url, None).map_err(wrap)?;
    let page_url = response.url().clone();
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    crate::screen::expect_page(id, None, None);
    crate::screen::page_read(&page_url, &body)?;
    Ok(String::from_utf8_lossy(&body).into_owned())
}

/// Upstream `_download_xml(url, video_id)`: `[EXTRACTOR] ID: Downloading
/// XML`, the request, and the body. Parse errors are this port's text
/// (upstream reports expat's).
pub(crate) fn download_xml(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
) -> Result<String> {
    download_xml_note(client, url, extractor, id, "Downloading XML")
}

/// [`download_xml`] with its `note=` (the error note stays the default).
pub(crate) fn download_xml_note(
    client: &Client,
    url: &Url,
    extractor: &'static str,
    id: &str,
    note: &str,
) -> Result<String> {
    use std::io::Read;
    crate::screen::screen(&format!("[{extractor}] {id}: {note}"));
    let wrap = |error: crate::Error| match error {
        crate::Error::Http(_) | crate::Error::HttpRetryAfter { .. } => {
            crate::Error::ExtractorRequest {
                extractor,
                id: id.to_owned(),
                what: "XML",
                source: Box::new(error),
            }
        }
        error => error,
    };
    let mut response = direct::request(client, url, None).map_err(wrap)?;
    let mut body = Vec::new();
    response
        .read_to_end(&mut body)
        .map_err(crate::Error::ExtractionRead)?;
    let text = String::from_utf8(body).map_err(|_| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse XML (not UTF-8)"
        ))
    })?;
    roxmltree::Document::parse(text.trim_start_matches('\u{feff}')).map_err(|error| {
        crate::Error::Reported(format!(
            "[{extractor}] {id}: Failed to parse XML ({error}); {}",
            crate::BUG_REPORTS_MESSAGE
        ))
    })?;
    Ok(text)
}

/// Upstream `_WORKING = False`: the warning before extraction.
pub(crate) fn broken_warning() {
    crate::screen::warning(
        "The program functionality for this site has been marked as broken, and will probably not work.",
    );
}

/// `root.find(tag).text` of an XML document: `Err` (upstream's bare
/// `AttributeError`) for a missing child, `None` for an empty one.
pub(crate) fn find_text(root: roxmltree::Node<'_, '_>, tag: &str) -> Result<Option<String>> {
    let child = root
        .children()
        .find(|child| child.is_element() && child.tag_name().name() == tag)
        .ok_or_else(|| {
            crate::Error::Reported("'NoneType' object has no attribute 'text'".into())
        })?;
    let text: String = child
        .children()
        .take_while(|node| !node.is_element())
        .filter_map(|node| node.text())
        .collect();
    Ok((!text.is_empty()).then_some(text))
}

/// `_search_json(start_pattern, string, ...)`: the greedy `{...}` after the
/// start (`contains_pattern=r'{(?s:.+)}'`), optionally converted with
/// `js_to_json`, and its first JSON value (`ignore_extra=True`); `None`
/// without a match, `Some(None)` when it does not parse.
pub(crate) fn search_json(
    start: &regex::Regex,
    html: &str,
    js: bool,
) -> Option<Option<serde_json::Value>> {
    let found = start.find(html)?;
    let rest = &html[found.end()..];
    let trimmed = rest.trim_start();
    if !trimmed.starts_with('{') {
        return None;
    }
    let end = trimmed.rfind('}')?;
    let candidate = &trimmed[..=end];
    let text = if js {
        crate::js_to_json::js_to_json(candidate)
    } else {
        candidate.to_owned()
    };
    Some(
        serde_json::Deserializer::from_str(&text)
            .into_iter::<serde_json::Value>()
            .next()
            .and_then(std::result::Result::ok),
    )
}

/// Whether an extractor's `_VALID_URL` takes this non-HTTP pseudo-URL.
pub fn pseudo_url(url: &str) -> bool {
    url.starts_with("jwplatform:") && jwplatform::suitable(url)
}

/// A non-fatal `_search_regex` miss: `[EXTRACTOR] unable to extract NAME`.
pub(crate) fn search_warning(extractor: &str, name: &str) {
    crate::screen::warning(&format!(
        "[{extractor}] unable to extract {name}; {}",
        crate::BUG_REPORTS_MESSAGE
    ));
}

/// `self._proto_relative_url(url, scheme='http:')`.
pub(crate) fn proto_relative_http(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("http://{rest}"),
        None => url.to_owned(),
    }
}

/// `self._proto_relative_url(url)`: `//host/...` becomes `https://host/...`.
pub(crate) fn proto_relative(url: &str) -> String {
    match url.strip_prefix("//") {
        Some(rest) => format!("https://{rest}"),
        None => url.to_owned(),
    }
}
