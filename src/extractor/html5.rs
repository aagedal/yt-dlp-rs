//! Deliberately narrow HTML5 subset of upstream GenericIE and
//! HTML5MediaEmbedIE / InfoExtractor._parse_html5_media_entries at the pinned revision.
use super::{Extractor, direct};
use crate::{Error, Result, VideoFormat, VideoInfo};
use percent_encoding::percent_decode_str;
use reqwest::{
    blocking::Client,
    header::{CONTENT_ENCODING, CONTENT_TYPE},
};
use scraper::{Html, Selector};
use std::io::Read;
use url::Url;

pub struct GenericHttp;

const MAX_HTML_BYTES: u64 = 2 * 1024 * 1024;

struct Html5Media {
    urls: Vec<Url>,
    id: String,
    title: String,
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

fn parse_page(page_url: &Url, html: &str) -> Result<Option<Html5Media>> {
    let document = Html::parse_document(html);
    let media_selector = selector("video, audio");
    let mut elements = document.select(&media_selector);
    let Some(element) = elements.next() else {
        return Ok(None);
    };
    if elements.next().is_some() {
        return Err(unsupported(
            "multiple media elements require playlist support",
        ));
    }

    // Validate every alternative, including unselected manifests.
    let mut sources = Vec::new();
    if let Some(src) = nonempty(element.value().attr("src")) {
        sources.push((src, element.value().attr("type")));
    }
    for child in element.children().filter_map(scraper::ElementRef::wrap) {
        if child.value().name() == "source"
            && let Some(src) = nonempty(child.value().attr("src"))
        {
            sources.push((src, child.value().attr("type")));
        }
    }
    if sources.is_empty() {
        return Ok(None);
    }
    let base = document
        .select(&selector("base[href]"))
        .next()
        .map(|base| page_url.join(base.value().attr("href").unwrap_or("")))
        .transpose()?
        .unwrap_or_else(|| page_url.clone());
    let mut urls = Vec::new();
    for (source, hint) in sources {
        let url = base.join(source)?;
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Error::UnsupportedScheme(url.scheme().into()));
        }
        let path = url.path().to_ascii_lowercase();
        let hint = hint.unwrap_or("").to_ascii_lowercase();
        if path.ends_with(".m3u8")
            || path.ends_with(".mpd")
            || hint.contains("mpegurl")
            || hint.contains("dash+xml")
        {
            return Err(unsupported(
                "HLS/DASH sources require a manifest downloader",
            ));
        }
        urls.push(url);
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
    let title = if title.is_empty() { &id } else { &title };
    Ok(Some(Html5Media {
        urls,
        id: format!("{id}-1"),
        title: format!("{title} (1)"),
    }))
}

impl Extractor for GenericHttp {
    fn key(&self) -> &'static str {
        "Generic"
    }

    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        // Dispatch a single response so direct media does not need two probes.
        let response = direct::request(client, url, None)?;
        if direct::response_mime(&response) != "text/html" {
            return Ok(direct::from_response(&response, url));
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
        response.take(MAX_HTML_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_HTML_BYTES {
            return Err(Error::HtmlTooLarge(MAX_HTML_BYTES));
        }
        let html = std::str::from_utf8(&bytes)
            .map_err(|_| unsupported("only UTF-8/ASCII HTML is supported"))?;
        let Some(media) = parse_page(&page_url, html)? else {
            return Ok(None);
        };
        let mut formats = Vec::new();
        let mut first_info = None;
        for (index, url) in media.urls.iter().enumerate() {
            let response = direct::request(client, url, Some(&page_url))?;
            let info = direct::from_response(&response, url)
                .ok_or_else(|| unsupported("source does not serve supported direct HTTP media"))?;
            formats.push(VideoFormat {
                format_id: index.to_string(),
                url: info.url.clone(),
                ext: info.ext.clone(),
                filesize: info.filesize,
            });
            first_info.get_or_insert(info);
        }
        let mut info = first_info.expect("parsed HTML5 media has at least one source");
        info.formats = formats;
        info.id = media.id;
        info.title = media.title;
        info.webpage_url = page_url;
        info.direct = false;
        info.extractor = "html5".into();
        info.extractor_key = "HTML5MediaEmbed".into();
        Ok(Some(info))
    }
}
