//! Narrow watch-page slice of YoutubeIE at the pinned upstream revision.
//! One bounded WEB API fallback; no JavaScript or manifest fallback.
// The offline boundary is tested independently before extraction integration.
#[allow(dead_code)]
mod challenge;
mod client;
#[allow(dead_code)]
mod player;
use super::{Extractor, direct};
use crate::{Error, Result, VideoFormat, VideoInfo};
use reqwest::{
    blocking::Client,
    header::{CONTENT_ENCODING, CONTENT_TYPE},
};
use scraper::{Html, Selector};
use serde_json::Value;
use std::io::Read;
use url::Url;

pub struct Youtube;
const MAX_PAGE_BYTES: u64 = 2 * 1024 * 1024;

fn unsupported(reason: &str) -> Error {
    Error::UnsupportedYoutube(reason.into())
}

fn valid_id(id: &str) -> bool {
    id.len() == 11
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

// Claim all URLs on these exact hosts so unsupported site pages cannot fall
// through to generic HTML5 extraction and appear to be supported YouTube videos.
fn video_id(url: &Url) -> Result<Option<String>> {
    let host = url.host_str().unwrap_or("");
    if !matches!(
        host,
        "youtube.com"
            | "www.youtube.com"
            | "m.youtube.com"
            | "music.youtube.com"
            | "youtu.be"
            | "www.youtu.be"
            | "www.youtube-nocookie.com"
            | "youtube-nocookie.com"
    ) {
        return Ok(None);
    }
    if url.query_pairs().any(|(key, _)| key == "list") {
        return Err(unsupported(
            "playlists and video URLs with list= are not supported",
        ));
    }
    let id = if matches!(host, "youtu.be" | "www.youtu.be") {
        url.path()
            .strip_prefix('/')
            .filter(|s| !s.contains('/'))
            .map(str::to_owned)
    } else if url.path() == "/watch" {
        let ids: Vec<_> = url
            .query_pairs()
            .filter(|(key, _)| key == "v")
            .map(|(_, value)| value.into_owned())
            .collect();
        if ids.len() != 1 {
            None
        } else {
            ids.into_iter().next()
        }
    } else {
        ["/shorts/", "/embed/", "/live/"]
            .iter()
            .find_map(|prefix| url.path().strip_prefix(prefix))
            .map(str::to_owned)
    };
    id.filter(|id| valid_id(id))
        .map(Some)
        .ok_or_else(|| unsupported("expected a single video URL with an 11-character ID"))
}

fn initial_player_response(html: &str) -> Result<Option<Value>> {
    let document = Html::parse_document(html);
    let scripts = Selector::parse("script").expect("static selector");
    for script in document.select(&scripts) {
        let text = script.text().collect::<String>();
        for marker in [
            "ytInitialPlayerResponse",
            "window[\"ytInitialPlayerResponse\"]",
            "window['ytInitialPlayerResponse']",
        ] {
            for (offset, _) in text.match_indices(marker) {
                // Avoid matching a longer identifier or a quoted property name.
                if offset > 0
                    && (text.as_bytes()[offset - 1].is_ascii_alphanumeric()
                        || matches!(text.as_bytes()[offset - 1], b'_' | b'$'))
                {
                    continue;
                }
                let rest = text[offset + marker.len()..].trim_start();
                let Some(json) = rest.strip_prefix('=').map(str::trim_start) else {
                    continue;
                };
                if !json.starts_with('{') {
                    return Err(unsupported("initial player response is not a JSON object"));
                }
                let mut values = serde_json::Deserializer::from_str(json).into_iter::<Value>();
                return values
                    .next()
                    .and_then(std::result::Result::ok)
                    .map(Some)
                    .ok_or_else(|| unsupported("malformed initial player response"));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
fn player_response(html: &str) -> Result<Value> {
    initial_player_response(html)?.ok_or_else(|| unsupported("no initial player response"))
}

fn parse_formats(player: &Value, id: &str) -> Result<(String, Vec<VideoFormat>)> {
    if player
        .pointer("/playabilityStatus/status")
        .and_then(Value::as_str)
        != Some("OK")
    {
        return Err(unsupported(
            "video is not playable without authentication, consent, or other access handling",
        ));
    }
    let details = &player["videoDetails"];
    if details["videoId"].as_str() != Some(id) {
        return Err(unsupported(
            "player response video ID does not match the requested video",
        ));
    }
    for key in ["isLive", "isLiveContent", "isUpcoming", "isPostLiveDvr"] {
        if details[key].as_bool() == Some(true) {
            return Err(unsupported(
                "live, upcoming, and post-live videos are not supported",
            ));
        }
    }
    if player
        .pointer("/microformat/playerMicroformatRenderer/liveBroadcastDetails/isLiveNow")
        .and_then(Value::as_bool)
        == Some(true)
    {
        return Err(unsupported("live videos are not supported"));
    }
    let title = details["title"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| unsupported("player response has no title"))?
        .to_owned();
    let streaming = &player["streamingData"];
    if streaming
        .get("licenseInfos")
        .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|a| !a.is_empty()))
    {
        return Err(unsupported("DRM-protected player response"));
    }
    let mut formats = Vec::new();
    for source in streaming["formats"].as_array().into_iter().flatten() {
        if [
            "signatureCipher",
            "cipher",
            "drmFamilies",
            "drmTrackType",
            "targetDurationSec",
        ]
        .iter()
        .any(|key| source.get(*key).is_some_and(|v| !v.is_null()))
            || source["type"].as_str() == Some("FORMAT_STREAM_TYPE_OTF")
        {
            continue;
        }
        let Some(raw) = source["url"].as_str() else {
            continue;
        };
        let Ok(url) = Url::parse(raw) else {
            continue;
        };
        if !matches!(url.scheme(), "http" | "https")
            || url.query_pairs().any(|(key, _)| key == "n")
            || url.path().contains("/n/")
        {
            continue;
        }
        let Some(mime) = source["mimeType"].as_str() else {
            continue;
        };
        let (kind, parameters) = mime.split_once(';').unwrap_or((mime, ""));
        let ext = match kind.trim() {
            "video/mp4" => "mp4",
            "video/webm" => "webm",
            _ => continue,
        };
        let codecs = parameters
            .split(';')
            .find_map(|p| p.trim().strip_prefix("codecs="))
            .unwrap_or("")
            .trim_matches('"');
        let codecs: Vec<_> = codecs.split(',').map(str::trim).collect();
        let video = codecs.iter().any(|c| {
            ["avc1", "av01", "vp8", "vp9", "vp09"].iter().any(|prefix| {
                c.strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('.'))
            })
        });
        let audio = codecs.iter().any(|c| {
            ["mp4a", "opus", "vorbis"].iter().any(|prefix| {
                c.strip_prefix(prefix)
                    .is_some_and(|suffix| suffix.is_empty() || suffix.starts_with('.'))
            })
        });
        if !video || !audio {
            continue;
        }
        let Some(itag) = source["itag"].as_u64().filter(|i| *i > 0) else {
            continue;
        };
        if formats
            .iter()
            .any(|f: &VideoFormat| f.format_id == itag.to_string())
        {
            continue;
        }
        let dimension = |key: &str| {
            source[key]
                .as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
        };
        formats.push(VideoFormat {
            format_id: itag.to_string(),
            url,
            ext: ext.into(),
            filesize: None,
            width: dimension("width"),
            height: dimension("height"),
        });
    }
    if formats.is_empty() {
        return Err(unsupported(
            "no supported direct combined audio/video formats; signatures, n challenges, adaptive streams, manifests, SABR, and DRM are not implemented",
        ));
    }
    Ok((title, formats))
}

fn extract_watch_page(client: &Client, page: &Url, id: &str) -> Result<VideoInfo> {
    let response = direct::request(client, page, None)?;
    if direct::response_mime(&response) != "text/html" {
        return Err(unsupported("watch page is not HTML"));
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
                return Err(unsupported("only UTF-8/ASCII watch pages are supported"));
            }
        }
    }
    if response
        .headers()
        .get(CONTENT_ENCODING)
        .is_some_and(|v| !v.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(unsupported("compressed watch pages are not supported"));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_PAGE_BYTES)
    {
        return Err(Error::HtmlTooLarge(MAX_PAGE_BYTES));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_PAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    if bytes.len() as u64 > MAX_PAGE_BYTES {
        return Err(Error::HtmlTooLarge(MAX_PAGE_BYTES));
    }
    let html = std::str::from_utf8(&bytes).map_err(|_| unsupported("watch page is not UTF-8"))?;
    // Never use API fallback to bypass a malformed, gated, or unsupported
    // initial response. The missing-response path gets exactly one WEB request.
    let (player, api) = match initial_player_response(html)? {
        Some(player) => (player, false),
        None => (client::fetch_player(client, page, html, id)?, true),
    };
    let (title, mut formats) = parse_formats(&player, id)?;
    if api {
        // WEB HTTPS GVS requires a PO token except for itag 18 at the pinned
        // source revision. No token acquisition or missing-token override.
        formats.retain(|format| format.format_id == "18");
        if formats.is_empty() {
            return Err(unsupported(
                "WEB API formats require an unsupported GVS PO token; no token-free itag 18",
            ));
        }
    }
    for format in &mut formats {
        let response = direct::request(client, &format.url, Some(page))?;
        let media = direct::from_response(&response, &format.url)
            .ok_or_else(|| unsupported("format URL does not serve direct HTTP media"))?;
        if media.ext != format.ext {
            return Err(unsupported(
                "format MIME type disagrees with the media response",
            ));
        }
        format.url = media.url;
        format.filesize = media.filesize;
    }
    let first = &formats[0];
    Ok(VideoInfo {
        id: id.into(),
        title,
        ext: first.ext.clone(),
        url: first.url.clone(),
        webpage_url: Url::parse(&format!("https://www.youtube.com/watch?v={id}"))?,
        extractor: "youtube".into(),
        extractor_key: "Youtube".into(),
        direct: false,
        format_id: None,
        filesize: first.filesize,
        width: first.width,
        height: first.height,
        formats,
    })
}

impl Extractor for Youtube {
    fn key(&self) -> &'static str {
        "Youtube"
    }
    fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
        let Some(id) = video_id(url)? else {
            return Ok(None);
        };
        let page = Url::parse(&format!("https://www.youtube.com/watch?v={id}"))?;
        extract_watch_page(client, &page, &id).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread};

    const FIXTURE: &str = include_str!("../../tests/fixtures/youtube/progressive.html");
    fn player() -> Value {
        player_response(&FIXTURE.replace("{{ORIGIN}}", "https://media.example")).unwrap()
    }

    #[test]
    fn exact_hosts_and_supported_video_url_shapes() {
        for input in [
            "https://www.youtube.com/watch?v=BaW_jenozKc&t=3",
            "https://youtu.be/BaW_jenozKc",
            "https://m.youtube.com/shorts/BaW_jenozKc",
            "https://www.youtube-nocookie.com/embed/BaW_jenozKc",
            "https://music.youtube.com/watch?v=BaW_jenozKc",
            "https://youtube.com/live/BaW_jenozKc",
        ] {
            assert_eq!(
                video_id(&Url::parse(input).unwrap()).unwrap().as_deref(),
                Some("BaW_jenozKc")
            );
        }
        for input in [
            "https://youtube.com.evil.invalid/watch?v=BaW_jenozKc",
            "https://example.com/watch?v=BaW_jenozKc",
        ] {
            assert!(video_id(&Url::parse(input).unwrap()).unwrap().is_none());
        }
        for input in [
            "https://youtube.com/playlist?list=foo",
            "https://youtube.com/watch?v=BaW_jenozKc&list=foo",
            "https://youtu.be/BaW_jenozKc/extra",
            "https://youtube.com/watch?v=bad",
            "https://youtube.com/watch?v=BaW_jenozKc&v=BaW_jenozKc",
            "https://youtube.com/@channel",
        ] {
            assert!(
                matches!(
                    crate::YoutubeDl::new().unwrap().extract_info(input),
                    Err(Error::UnsupportedYoutube(_))
                ),
                "{input}"
            );
        }
    }

    #[test]
    fn json_assignment_handles_braces_in_strings_and_trailing_javascript() {
        assert_eq!(player()["videoDetails"]["title"], "Fixture } ; & video");
        for marker in [
            "ytInitialPlayerResponse",
            "window[\"ytInitialPlayerResponse\"]",
            "window['ytInitialPlayerResponse']",
        ] {
            assert!(
                player_response(&format!(
                    "<script>{marker} = {{\"x\": 1}}; other();</script>"
                ))
                .is_ok()
            );
        }
        for page in [
            "<p>ytInitialPlayerResponse = {}</p>",
            "<script>ytInitialPlayerResponse = {broken}</script>",
            "<script>ytInitialPlayerResponse = null</script>",
            "<script>_ytInitialPlayerResponse = {}</script>",
            "<script>$ytInitialPlayerResponse = {}</script>",
        ] {
            assert!(matches!(
                player_response(page),
                Err(Error::UnsupportedYoutube(_))
            ));
        }
    }

    #[test]
    fn rejects_access_identity_live_and_drm_responses() {
        for (pointer, value) in [
            ("/playabilityStatus/status", Value::from("LOGIN_REQUIRED")),
            ("/videoDetails/videoId", Value::from("wrong_video")),
            ("/videoDetails/title", Value::from("")),
            ("/videoDetails/isLiveContent", Value::from(true)),
        ] {
            let mut data = player();
            *data.pointer_mut(pointer).unwrap() = value;
            assert!(matches!(
                parse_formats(&data, "BaW_jenozKc"),
                Err(Error::UnsupportedYoutube(_))
            ));
        }
        for key in ["isLive", "isUpcoming", "isPostLiveDvr"] {
            let mut data = player();
            data["videoDetails"][key] = true.into();
            assert!(parse_formats(&data, "BaW_jenozKc").is_err());
        }
        let mut data = player();
        data["streamingData"]["licenseInfos"] = serde_json::json!([{"drmFamily": "WIDEVINE"}]);
        assert!(parse_formats(&data, "BaW_jenozKc").is_err());
    }

    #[test]
    fn supports_muxed_webm_and_ignores_malformed_dimensions_and_duplicate_itags() {
        let mut data = player();
        data["streamingData"]["formats"][0]["mimeType"] = "video/webm; codecs=\"vp9, opus\"".into();
        data["streamingData"]["formats"][0]["width"] = 0.into();
        data["streamingData"]["formats"][0]["height"] = u64::MAX.into();
        let duplicate = data["streamingData"]["formats"][0].clone();
        data["streamingData"]["formats"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        let (_, formats) = parse_formats(&data, "BaW_jenozKc").unwrap();
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].ext, "webm");
        assert_eq!((formats[0].width, formats[0].height), (None, None));
    }

    #[test]
    fn unsupported_formats_are_never_exposed_or_probed() {
        for (key, value) in [
            ("signatureCipher", Value::from("s=secret")),
            ("cipher", Value::from("s=secret")),
            (
                "url",
                Value::from("https://media.example/video?n=challenge"),
            ),
            (
                "url",
                Value::from("https://media.example/n/challenge/video"),
            ),
            ("url", Value::from("blob:https://youtube.com/id")),
            ("type", Value::from("FORMAT_STREAM_TYPE_OTF")),
            ("targetDurationSec", Value::from(5)),
            ("drmFamilies", serde_json::json!(["WIDEVINE"])),
            ("mimeType", Value::from("video/mp4; codecs=\"avc1.42001E\"")),
            ("mimeType", Value::from("application/x-mpegURL")),
            (
                "mimeType",
                Value::from("video/mp4; codecs=\"avc1unknown, mp4aunknown\""),
            ),
        ] {
            let mut data = player();
            data["streamingData"]["formats"][0][key] = value;
            let (_, formats) = parse_formats(&data, "BaW_jenozKc").unwrap();
            assert_eq!(formats.len(), 1, "{key}");
            assert_eq!(formats[0].format_id, "22");
            data["streamingData"]["formats"]
                .as_array_mut()
                .unwrap()
                .pop();
            assert!(matches!(
                parse_formats(&data, "BaW_jenozKc"),
                Err(Error::UnsupportedYoutube(_))
            ));
        }
        let mut data = player();
        let formats = data["streamingData"]
            .as_object_mut()
            .unwrap()
            .remove("formats")
            .unwrap();
        data["streamingData"]["adaptiveFormats"] = formats;
        data["streamingData"]["hlsManifestUrl"] = "https://media.example/live.m3u8".into();
        assert!(parse_formats(&data, "BaW_jenozKc").is_err());
    }

    #[test]
    fn rejects_invalid_watch_page_responses_and_media_probes() {
        let bad_source = FIXTURE.replace("{{ORIGIN}}", "blob:https://youtube.com");
        for (mime, extra, body, probe_html) in [
            ("text/plain", "", b"no page".to_vec(), false),
            (
                "text/html; charset=iso-8859-1",
                "",
                b"ASCII page".to_vec(),
                false,
            ),
            (
                "text/html",
                "Content-Encoding: gzip\r\n",
                b"compressed".to_vec(),
                false,
            ),
            ("text/html", "", vec![0xff], false),
            (
                "text/html",
                "",
                vec![b' '; MAX_PAGE_BYTES as usize + 1],
                false,
            ),
            ("text/html", "", bad_source.into_bytes(), false),
            ("text/html", "", FIXTURE.as_bytes().to_vec(), true),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let origin = format!("http://{}", listener.local_addr().unwrap());
            let page = Url::parse(&format!("{origin}/watch?v=BaW_jenozKc")).unwrap();
            let body = if probe_html {
                FIXTURE.replace("{{ORIGIN}}", &origin).into_bytes()
            } else {
                body
            };
            let worker = thread::spawn(move || {
                for index in 0..if probe_html { 2 } else { 1 } {
                    let (mut stream, _) = listener.accept().unwrap();
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                        .unwrap();
                    let mut request = Vec::new();
                    while !request.ends_with(b"\r\n\r\n") {
                        let mut byte = [0];
                        assert_eq!(stream.read(&mut byte).unwrap(), 1);
                        request.push(byte[0]);
                    }
                    let response_body = if index == 0 {
                        body.as_slice()
                    } else {
                        b"<html>not media</html>"
                    };
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n", response_body.len()).unwrap();
                    let _ = stream.write_all(response_body);
                }
            });
            let client = Client::builder()
                .no_proxy()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap();
            assert!(matches!(
                extract_watch_page(&client, &page, "BaW_jenozKc"),
                Err(Error::UnsupportedYoutube(_) | Error::HtmlTooLarge(_))
            ));
            worker.join().unwrap();
        }
    }

    #[test]
    fn local_watch_page_probe_select_and_download() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let page = Url::parse(&format!("{origin}/watch?v=BaW_jenozKc")).unwrap();
        let html = FIXTURE.replace("{{ORIGIN}}", &origin);
        let worker = thread::spawn(move || {
            // Page, two format probes, selected media download.
            for expected in [
                "/watch?v=BaW_jenozKc",
                "/media/low.mp4",
                "/media/high.mp4",
                "/media/high.mp4",
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    assert_eq!(stream.read(&mut byte).unwrap(), 1);
                    request.push(byte[0]);
                }
                let request = String::from_utf8(request).unwrap();
                assert!(request.starts_with(&format!("GET {expected} HTTP/1.1\r\n")));
                let (mime, body) = if expected.starts_with("/watch") {
                    ("text/html; charset=utf-8", html.as_bytes())
                } else {
                    ("video/mp4", b"media".as_slice())
                };
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
                let _ = stream.write_all(body);
            }
        });
        let client = Client::builder()
            .no_proxy()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .unwrap();
        let mut info = extract_watch_page(&client, &page, "BaW_jenozKc").unwrap();
        assert_eq!(info.formats.len(), 2);
        assert_eq!(info.title, "Fixture } ; & video");
        info.select_format(Some("18")).unwrap();
        assert_eq!(info.height, Some(360));
        info.select_format(Some("worst")).unwrap();
        assert_eq!(info.format_id.as_deref(), Some("18"));
        assert!(info.select_format(Some("137+140")).is_err());
        info.select_format(None).unwrap();
        assert_eq!(info.format_id.as_deref(), Some("22"));
        assert_eq!(info.filesize, Some(5));
        let expected: Value = serde_json::from_str(
            &include_str!("../../tests/fixtures/youtube/expected.json")
                .replace("{{ORIGIN}}", &origin),
        )
        .unwrap();
        let actual = serde_json::to_value(&info).unwrap();
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(&actual[key], value, "{key}");
        }
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("video.mp4");
        let ydl = crate::YoutubeDl::new().unwrap();
        assert_eq!(ydl.download(&info, &destination).unwrap(), 5);
        assert_eq!(std::fs::read(destination).unwrap(), b"media");
        worker.join().unwrap();
    }
}
