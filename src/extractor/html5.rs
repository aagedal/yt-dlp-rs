//! Deliberately narrow HTML5 subset of upstream GenericIE and
//! HTML5MediaEmbedIE / InfoExtractor._parse_html5_media_entries at the pinned revision.
use super::{Extractor, dash, direct, hls};
use crate::{
    Error, ExtractResult, PlaylistInfo, Result, SubtitleFormat, Subtitles, VideoFormat, VideoInfo,
};
use percent_encoding::percent_decode_str;
use reqwest::{
    blocking::Client,
    header::{CONTENT_ENCODING, CONTENT_TYPE},
};
use scraper::{Html, Selector};
use std::{collections::HashMap, io::Read};
use url::Url;

pub struct GenericHttp;

const MAX_HTML_BYTES: u64 = 2 * 1024 * 1024;

struct Html5Page {
    entries: Vec<Html5Entry>,
    id: String,
    title: String,
    playlist_title: String,
}

struct Html5Entry {
    sources: Vec<Html5Source>,
    subtitles: Subtitles,
    /// `absolute_url(poster)`.
    thumbnail: Option<String>,
}

// Match the pinned determine_ext URL suffix rules, without MIME requests.
fn subtitle_extension(url: &Url) -> String {
    let url = url.as_str().split('?').next().unwrap_or("");
    let Some((_, guess)) = url.rsplit_once('.') else {
        return "unknown_video".into();
    };
    if !guess.is_empty() && guess.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return guess.to_ascii_lowercase();
    }
    let trimmed = guess.trim_end_matches('/');
    if matches!(
        trimmed,
        "3g2"
            | "3gp"
            | "f4v"
            | "mk3d"
            | "divx"
            | "mpg"
            | "ogv"
            | "m4v"
            | "wmv"
            | "avi"
            | "flv"
            | "mkv"
            | "mov"
            | "mp4"
            | "webm"
            | "aac"
            | "ape"
            | "asf"
            | "f4a"
            | "f4b"
            | "m4b"
            | "m4r"
            | "oga"
            | "ogx"
            | "spx"
            | "vorbis"
            | "wma"
            | "weba"
            | "aiff"
            | "alac"
            | "flac"
            | "m4a"
            | "mka"
            | "mp3"
            | "ogg"
            | "opus"
            | "wav"
            | "f4f"
            | "f4m"
            | "m3u8"
            | "smil"
            | "mpd"
    ) {
        trimmed.into()
    } else {
        "unknown_video".into()
    }
}

struct Html5Source {
    url: Url,
    hls_hint: bool,
    dash_hint: bool,
    width: Option<u32>,
    height: Option<u32>,
}

fn dimension(value: Option<&str>) -> Option<u32> {
    let value = value?.trim();
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    value.parse().ok().filter(|&value| value > 0)
}

fn unsupported(reason: &str) -> Error {
    Error::UnsupportedHtml5(reason.into())
}

fn selector(value: &str) -> Selector {
    Selector::parse(value).expect("static HTML selector")
}

/// `(?:[a-zA-Z][a-zA-Z0-9+-.]*:)?//` at the start (upstream `urljoin` keeps these).
fn scheme_relative(value: &str) -> bool {
    let rest = match value.find(':') {
        Some(colon)
            if value[..colon]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic())
                && value[..colon]
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || ('+'..='.').contains(&c)) =>
        {
            &value[colon + 1..]
        }
        _ => value,
    };
    rest.starts_with("//")
}

fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn parse_page(page_url: &Url, html: &str) -> Result<Option<Html5Page>> {
    let document = Html::parse_document(html);
    let base = document
        .select(&selector("base[href]"))
        .next()
        .map(|base| page_url.join(base.value().attr("href").unwrap_or("")))
        .transpose()?
        .unwrap_or_else(|| page_url.clone());
    let mut entries = Vec::new();
    for element in document.select(&selector("video, audio")) {
        // Validate every alternative, including unselected manifests.
        let mut sources = Vec::new();
        if let Some(src) = nonempty(element.value().attr("src")) {
            // Media element dimensions describe the player, not its source.
            sources.push((src, element.value().attr("type"), None, None));
        }
        for child in element.children().filter_map(scraper::ElementRef::wrap) {
            if child.value().name() == "source"
                && let Some(src) = nonempty(child.value().attr("src"))
            {
                sources.push((
                    src,
                    child.value().attr("type"),
                    dimension(child.value().attr("width")),
                    dimension(child.value().attr("height"))
                        .or_else(|| dimension(child.value().attr("res"))),
                ));
            }
        }
        if sources.is_empty() {
            continue;
        }
        let mut urls = Vec::new();
        for (source, hint, width, height) in sources {
            let url = base.join(source)?;
            if !matches!(url.scheme(), "http" | "https") {
                return Err(Error::UnsupportedScheme(url.scheme().into()));
            }
            let hint = hint.unwrap_or("").to_ascii_lowercase();
            urls.push(Html5Source {
                url,
                width,
                height,
                hls_hint: hint.contains("mpegurl"),
                dash_hint: hint.contains("dash+xml"),
            });
        }
        let mut subtitles = Subtitles::default();
        let mut language_indices = HashMap::<Option<String>, usize>::new();
        for track in element.children().filter_map(scraper::ElementRef::wrap) {
            if track.value().name() != "track"
                || !matches!(
                    track.value().attr("kind"),
                    None | Some("" | "subtitles" | "captions")
                )
            {
                continue;
            }
            let Some(src) = nonempty(track.value().attr("src")) else {
                continue;
            };
            // Source postprocessing sanitizes protocol-relative subtitle URLs to
            // HTTP and repairs the httpss typo before completing extensions.
            let url = if src.starts_with("//") {
                Url::parse(&format!("http:{src}"))?
            } else if let Some(rest) = src.strip_prefix("httpss://") {
                Url::parse(&format!("https://{rest}"))?
            } else {
                base.join(src)?
            };
            if !matches!(url.scheme(), "http" | "https") {
                return Err(Error::UnsupportedScheme(url.scheme().into()));
            }
            let language = track
                .value()
                .attr("srclang")
                .filter(|lang| !lang.is_empty())
                .or_else(|| track.value().attr("lang").filter(|lang| !lang.is_empty()))
                .or_else(|| track.value().attr("label"))
                .map(str::to_owned);
            let format = SubtitleFormat {
                name: None,
                ext: subtitle_extension(&url),
                url,
            };
            if let Some(&index) = language_indices.get(&language) {
                subtitles.entries[index].1.push(format);
            } else {
                language_indices.insert(language.clone(), subtitles.entries.len());
                subtitles.entries.push((language, vec![format]));
            }
        }
        // Upstream `urljoin`: empty values are absent, scheme/protocol-relative
        // URLs are kept verbatim, and others resolve against the base.
        let thumbnail = match element.value().attr("poster") {
            None | Some("") => None,
            Some(poster) if scheme_relative(poster) => Some(poster.to_owned()),
            Some(poster) => base.join(poster).ok().map(String::from),
        };
        entries.push(Html5Entry {
            sources: urls,
            subtitles,
            thumbnail,
        });
    }
    if entries.is_empty() {
        return Ok(None);
    }
    let title = document
        .select(&selector("meta[property='og:title']"))
        .find_map(|meta| nonempty(meta.value().attr("content")))
        .map(str::to_owned)
        .or_else(|| {
            document
                .select(&selector("title"))
                .next()
                .map(|title| title.text().collect::<String>())
        });
    let title = title
        .unwrap_or_default()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let filename = page_url
        .path()
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or("");
    let stem = filename.rsplit_once('.').map_or(filename, |(stem, _)| stem);
    let id = percent_decode_str(stem).decode_utf8_lossy().into_owned();
    let id = if id.is_empty() { "video".into() } else { id };
    let playlist_title = if title.is_empty() { "video" } else { &title }.to_owned();
    let title = if title.is_empty() { &id } else { &title };
    Ok(Some(Html5Page {
        entries,
        title: title.to_owned(),
        playlist_title,
        id,
    }))
}

impl Extractor for GenericHttp {
    fn key(&self) -> &'static str {
        "Generic"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        self.extract_result(client, url)?
            .map(ExtractResult::into_video)
            .transpose()
    }

    fn extract_result(&self, client: &Client, url: &Url) -> Result<Option<ExtractResult>> {
        use crate::screen::{generic_id, screen, truncate_string, warning};
        crate::screen::set_extractor("generic");
        // Upstream `InfoExtractor.extract` and `GenericIE._real_extract` lines.
        screen(&format!(
            "[generic] Extracting URL: {}",
            truncate_string(url.as_str(), 100, 20)
        ));
        let id = generic_id(url.as_str());
        // `_request_webpage` sleeps before announcing the download.
        crate::screen::before_extraction_request();
        screen(&format!("[generic] {id}: Downloading webpage"));
        // Dispatch a single response so direct media does not need two probes.
        let response = direct::probe(client, url, None)?;
        // Upstream's direct-link Content-Type pattern; anything else falls
        // back to sniffing the body.
        let content_type = response
            .headers()
            .get(CONTENT_TYPE)
            .map(|value| String::from_utf8_lossy(value.as_bytes()).to_lowercase())
            .unwrap_or_default();
        let media_type = {
            static MEDIA: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| {
                regex::Regex::new(
                    r"^(?:audio|video|application/(?:ogg$|(?:vnd\.apple\.|x-)?mpegurl))",
                )
                .unwrap()
            });
            MEDIA.is_match(&content_type)
        };
        let fallback = || warning("[generic] Falling back on generic information extractor");
        let m3u8 = || {
            screen(&format!("[generic] {id}: Downloading m3u8 information"));
            screen(&format!("[generic] {id}: Checking m3u8 live status"));
        };
        // Upstream sets the timestamp before inspecting the response.
        let timestamp = direct::last_modified(&response);
        let stamp = |mut info: VideoInfo| {
            info.metadata.timestamp = timestamp;
            ExtractResult::Video(info)
        };
        let dash_path = url.path().to_ascii_lowercase().ends_with(".mpd");
        if dash_path || dash::recognized(&response) {
            if !media_type {
                fallback();
            }
            screen(&format!("[generic] {id}: Extracting information"));
            return Ok(dash::from_response(response, url, dash_path)?.map(stamp));
        }
        let hls_path = url.path().to_ascii_lowercase().ends_with(".m3u8")
            || response
                .url()
                .path()
                .to_ascii_lowercase()
                .ends_with(".m3u8");
        if hls_path || direct::response_mime(&response) != "text/html" {
            let mime = direct::response_mime(&response);
            let direct =
                direct::from_response(&response, url).map(|info| direct::with_format(info, &mime));
            if !media_type {
                fallback();
            }
            let hls = hls::from_response(client, response, url, None, hls_path, None)?;
            if hls.is_some() {
                m3u8();
            } else if direct.is_some() && !media_type {
                warning("[generic] URL could be a direct video link, returning it as such.");
            }
            return Ok(hls.or(direct).map(stamp));
        }
        fallback();
        screen(&format!("[generic] {id}: Extracting information"));
        if response
            .content_length()
            .is_some_and(|size| size > MAX_HTML_BYTES)
        {
            return Err(Error::HtmlTooLarge(MAX_HTML_BYTES));
        }
        if response
            .headers()
            .get(CONTENT_ENCODING)
            .is_some_and(|value| !value.as_bytes().eq_ignore_ascii_case(b"identity"))
        {
            return Err(unsupported("compressed HTML is not supported"));
        }
        if let Some(content_type) = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
        {
            for parameter in content_type.split(';').skip(1) {
                if let Some((name, value)) = parameter.trim().split_once('=')
                    && name.trim().eq_ignore_ascii_case("charset")
                    && !matches!(
                        value
                            .trim()
                            .trim_matches(['\'', '"'])
                            .to_ascii_lowercase()
                            .as_str(),
                        "utf-8" | "utf8" | "us-ascii"
                    )
                {
                    return Err(unsupported("only UTF-8/ASCII HTML is supported"));
                }
            }
        }
        let page_url = response.url().clone();
        let mut bytes = Vec::new();
        response
            .take(MAX_HTML_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(Error::ExtractionRead)?;
        if bytes.len() as u64 > MAX_HTML_BYTES {
            return Err(Error::HtmlTooLarge(MAX_HTML_BYTES));
        }
        let html = std::str::from_utf8(&bytes)
            .map_err(|_| unsupported("only UTF-8/ASCII HTML is supported"))?;
        let Some(media) = parse_page(&page_url, html)? else {
            return Ok(None);
        };
        let mut entries = Vec::new();
        for (index, entry) in media.entries.iter().enumerate() {
            let mut formats = Vec::new();
            let mut first_info = None;
            for (index, source) in entry.sources.iter().enumerate() {
                let url = &source.url;
                // Upstream requests only manifest sources it recognizes by
                // type or extension; other sources are probed natively.
                let manifest_hint = source.hls_hint
                    || source.dash_hint
                    || [".m3u8", ".mpd"]
                        .iter()
                        .any(|suffix| url.path().to_ascii_lowercase().ends_with(suffix));
                let response = if manifest_hint {
                    direct::request(client, url, Some(&page_url))?
                } else {
                    direct::probe(client, url, Some(&page_url))?
                };
                let direct = direct::from_response(&response, url);
                let dash_hint =
                    source.dash_hint || url.path().to_ascii_lowercase().ends_with(".mpd");
                let manifest = if dash_hint || dash::recognized(&response) {
                    let manifest = dash::from_response(response, url, dash_hint)?;
                    if manifest.is_some() {
                        screen(&format!("[html5] {id}: Downloading MPD manifest"));
                    }
                    manifest
                } else {
                    let manifest = hls::from_response(
                        client,
                        response,
                        url,
                        Some(&page_url),
                        source.hls_hint || url.path().to_ascii_lowercase().ends_with(".m3u8"),
                        Some("hls"),
                    )?;
                    if manifest.is_some() {
                        screen(&format!("[html5] {id}: Downloading m3u8 information"));
                    }
                    manifest
                };
                let info = manifest
                    .or(direct)
                    .ok_or_else(|| unsupported("source does not serve supported HTTP media"))?;
                if !info.formats.is_empty() {
                    // Manifest IDs stay as upstream names them (`hls-…`, bare DASH
                    // representation IDs); duplicates are numbered after extraction.
                    formats.extend(info.formats.iter().cloned().map(|mut format| {
                        format.audio_group =
                            format.audio_group.map(|group| format!("{index}-{group}"));
                        format
                    }));
                    first_info.get_or_insert(info);
                    continue;
                }
                formats.push(VideoFormat {
                    format_id: index.to_string(),
                    stream_kind: info.stream_kind,
                    audio_group: None,
                    language: None,
                    format_note: None,
                    audio_default: false,
                    audio_autoselect: false,
                    abr: None,
                    tbr: None,
                    url: info.url.clone(),
                    ext: info.ext.clone(),
                    filesize: info.filesize,
                    fragments: info.fragments.clone(),
                    width: source.width,
                    height: source.height,
                });
                first_info.get_or_insert(info);
            }
            let mut info = first_info.expect("parsed HTML5 media has at least one source");
            info.formats = formats;
            info.subtitles = entry.subtitles.clone();
            info.metadata.thumbnail = entry.thumbnail.clone();
            info.id = format!("{}-{}", media.id, index + 1);
            info.title = format!("{} ({})", media.title, index + 1);
            info.webpage_url = page_url.clone();
            info.direct = false;
            info.extractor = "html5".into();
            info.extractor_key = "HTML5MediaEmbed".into();
            entries.push(info);
        }
        if entries.len() == 1 {
            // `merge_dicts(embed, info_dict)`: page fields fill missing values.
            let mut info = entries.pop().expect("one entry");
            info.metadata.timestamp = info.metadata.timestamp.or(timestamp);
            info.metadata.description = crate::html::og_search_property(html, "description");
            if info.metadata.thumbnail.is_none() {
                info.metadata.thumbnail = crate::html::og_search_property(html, "image");
            }
            info.metadata.age_limit = crate::html::rta_search(html);
            return Ok(Some(ExtractResult::Video(info)));
        }
        Ok(Some(ExtractResult::Playlist(PlaylistInfo {
            id: media.id,
            title: media.playlist_title,
            webpage_url: page_url,
            extractor: "generic".into(),
            extractor_key: "Generic".into(),
            entries,
        })))
    }
}

#[cfg(test)]
mod subtitle_tests {
    use super::*;
    #[test]
    fn saved_html_tracks_match_actual_upstream_parser_and_extension_rules() {
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../../tests/fixtures/subtitles/cases.json"))
                .unwrap();
        for case in fixture["html_cases"].as_array().unwrap() {
            let page = parse_page(
                &Url::parse(case["page_url"].as_str().unwrap()).unwrap(),
                case["html"].as_str().unwrap(),
            )
            .unwrap()
            .unwrap();
            let actual: Vec<_> = page.entries.iter().map(|entry| {
                let groups: Vec<_> = entry.subtitles.entries.iter().map(|(language, formats)| serde_json::json!({"language":language,"formats":formats})).collect();
                serde_json::json!({"subtitles":groups})
            }).collect();
            assert_eq!(
                serde_json::json!({"entries":actual}),
                case["expected"],
                "{}",
                case["name"]
            );
        }
    }
    #[test]
    fn subtitle_extension_keeps_pinned_fragment_query_and_trailing_slash_quirks() {
        for (url, ext) in [
            ("https://example.test/a.VTT?token=1#f", "vtt"),
            ("https://example.test/a.vtt#f", "unknown_video"),
            ("https://example.test/a.vtt/", "unknown_video"),
            ("https://example.test/a.mp4/", "mp4"),
            ("https://example.test/a.MP4/", "unknown_video"),
            ("https://example.test/no-ext", "unknown_video"),
        ] {
            assert_eq!(subtitle_extension(&Url::parse(url).unwrap()), ext);
        }
    }
}
