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
        entries.push(Html5Entry {
            sources: urls,
            subtitles,
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
        // Dispatch a single response so direct media does not need two probes.
        let response = direct::request(client, url, None)?;
        let dash_path = url.path().to_ascii_lowercase().ends_with(".mpd");
        if dash_path || dash::recognized(&response) {
            return Ok(dash::from_response(response, url, dash_path)?.map(ExtractResult::Video));
        }
        let hls_path = url.path().to_ascii_lowercase().ends_with(".m3u8")
            || response
                .url()
                .path()
                .to_ascii_lowercase()
                .ends_with(".m3u8");
        if hls_path || direct::response_mime(&response) != "text/html" {
            let direct = direct::from_response(&response, url);
            return Ok(hls::from_response(client, response, url, None, hls_path)?
                .or(direct)
                .map(ExtractResult::Video));
        }
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
                let response = direct::request(client, url, Some(&page_url))?;
                let direct = direct::from_response(&response, url);
                let dash_hint =
                    source.dash_hint || url.path().to_ascii_lowercase().ends_with(".mpd");
                let manifest = if dash_hint || dash::recognized(&response) {
                    dash::from_response(response, url, dash_hint)?
                } else {
                    hls::from_response(
                        client,
                        response,
                        url,
                        Some(&page_url),
                        source.hls_hint || url.path().to_ascii_lowercase().ends_with(".m3u8"),
                    )?
                };
                let info = manifest
                    .or(direct)
                    .ok_or_else(|| unsupported("source does not serve supported HTTP media"))?;
                if !info.formats.is_empty() {
                    formats.extend(info.formats.iter().cloned().map(|mut format| {
                        format.format_id = format!("{index}-{}", format.format_id);
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
            info.id = format!("{}-{}", media.id, index + 1);
            info.title = format!("{} ({})", media.title, index + 1);
            info.webpage_url = page_url.clone();
            info.direct = false;
            info.extractor = "html5".into();
            info.extractor_key = "HTML5MediaEmbed".into();
            entries.push(info);
        }
        if entries.len() == 1 {
            return Ok(entries.pop().map(ExtractResult::Video));
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
