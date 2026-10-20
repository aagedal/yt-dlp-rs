//! Bounded anonymous WEB player acquisition, separate from response parsing.
use super::{MAX_PAGE_BYTES, unsupported};
use crate::{Error, Result};
use reqwest::{
    blocking::Client,
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_TYPE, ORIGIN, REFERER},
};
use scraper::{Html, Selector};
use serde_json::{Value, json};
use std::io::Read;
use url::Url;

// Only read literal configuration objects; never evaluate page JavaScript or
// accept a page-supplied API host, client family, credentials, or request body.
pub(super) fn literal_config(html: &str) -> Result<serde_json::Map<String, Value>> {
    let document = Html::parse_document(html);
    let scripts = Selector::parse("script").expect("static selector");
    let mut config = serde_json::Map::new();
    for script in document.select(&scripts) {
        let text = script.text().collect::<String>();
        for (offset, _) in text.match_indices("ytcfg.set(") {
            if offset > 0
                && (text.as_bytes()[offset - 1].is_ascii_alphanumeric()
                    || matches!(text.as_bytes()[offset - 1], b'_' | b'$' | b'.'))
            {
                continue;
            }
            let input = text[offset + "ytcfg.set(".len()..].trim_start();
            let mut values = serde_json::Deserializer::from_str(input).into_iter::<Value>();
            let Some(Ok(Value::Object(value))) = values.next() else {
                return Err(unsupported("malformed literal ytcfg configuration"));
            };
            config.extend(value);
        }
    }
    Ok(config)
}

fn web_context(html: &str) -> Result<Value> {
    let config = literal_config(html)?;
    let client = &config.get("INNERTUBE_CONTEXT").unwrap_or(&Value::Null)["client"];
    if client["clientName"].as_str() != Some("WEB") {
        return Err(unsupported(
            "player API fallback requires a WEB page client context",
        ));
    }
    let version = client["clientVersion"]
        .as_str()
        .filter(|s| {
            !s.is_empty() && s.len() <= 128 && s.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        })
        .ok_or_else(|| unsupported("missing or invalid WEB client version"))?;
    let mut context =
        json!({"client": {"clientName": "WEB", "clientVersion": version, "hl": "en", "gl": "US"}});
    if let Some(visitor) = client["visitorData"].as_str() {
        if visitor.len() > 4096 || visitor.chars().any(char::is_control) {
            return Err(unsupported("invalid WEB visitor data"));
        }
        context["client"]["visitorData"] = visitor.into();
    }
    Ok(context)
}

pub(super) fn fetch_player(client: &Client, page: &Url, html: &str, id: &str) -> Result<Value> {
    let context = web_context(html)?;
    let version = context["client"]["clientVersion"].as_str().unwrap();
    let endpoint = page.join("/youtubei/v1/player?prettyPrint=false")?;
    let body = json!({
        "context": context,
        "videoId": id,
        "playbackContext": {"contentPlaybackContext": {"html5Preference": "HTML5_PREF_WANTS"}}
    });
    let mut request = client
        .post(endpoint)
        .header(CONTENT_TYPE, "application/json")
        .header(ACCEPT_ENCODING, "identity")
        .header(ORIGIN, page.origin().ascii_serialization())
        .header(REFERER, page.as_str())
        .header("X-Youtube-Client-Name", "1")
        .header("X-Youtube-Client-Version", version);
    if let Some(visitor) = context["client"]["visitorData"].as_str() {
        request = request.header("X-Goog-Visitor-Id", visitor);
    }
    crate::screen::before_extraction_request();
    crate::screen::load_cookies()?;
    let response = request.body(body.to_string()).send();
    crate::screen::flush_traffic();
    let response = crate::retry::check_response(response?)?;
    if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(Error::PartialResponse);
    }
    if super::direct::response_mime(&response) != "application/json" {
        return Err(unsupported("player API response is not JSON"));
    }
    if response
        .headers()
        .get(CONTENT_ENCODING)
        .is_some_and(|v| !v.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(unsupported(
            "compressed player API responses are not supported",
        ));
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_PAGE_BYTES)
    {
        return Err(unsupported("player API response exceeds 2 MiB"));
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_PAGE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(Error::ExtractionRead)?;
    if bytes.len() as u64 > MAX_PAGE_BYTES {
        return Err(unsupported("player API response exceeds 2 MiB"));
    }
    serde_json::from_slice(&bytes).map_err(|_| unsupported("malformed player API JSON"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Write,
        net::TcpListener,
        thread,
        time::{Duration, Instant},
    };

    const PAGE: &str = include_str!("../../../tests/fixtures/youtube/player-api.html");
    const PLAYER: &str = include_str!("../../../tests/fixtures/youtube/player-api.json");
    const ID: &str = "BaW_jenozKc";

    struct Reply {
        path: &'static str,
        status: u16,
        mime: &'static str,
        extra: &'static str,
        body: Vec<u8>,
        missing_bytes: usize,
        omit_length: bool,
    }

    fn serve(replies: Vec<Reply>) -> (Url, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let page = Url::parse(&format!("{origin}/watch?v={ID}")).unwrap();
        let worker = thread::spawn(move || {
            for reply in replies {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "missing request: {}", reply.path);
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
                // Windows accepted sockets can inherit the listener's mode.
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut headers = Vec::new();
                while !headers.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    assert_eq!(stream.read(&mut byte).unwrap(), 1);
                    headers.push(byte[0]);
                }
                let headers = String::from_utf8(headers).unwrap();
                let api = reply.path.starts_with("/youtubei/");
                let method = if api { "POST" } else { "GET" };
                assert!(
                    headers.starts_with(&format!("{method} {} HTTP/1.1\r\n", reply.path)),
                    "{headers}"
                );
                if api {
                    let lower = headers.to_ascii_lowercase();
                    for expected in [
                        "content-type: application/json".to_owned(),
                        "x-youtube-client-name: 1".to_owned(),
                        "x-youtube-client-version: 2.20260708.00.00".to_owned(),
                        "x-goog-visitor-id: fixture-visitor".to_owned(),
                        format!("origin: {origin}"),
                        format!("referer: {origin}/watch?v={ID}"),
                        "accept-encoding: identity".to_owned(),
                    ] {
                        assert!(
                            lower.contains(&expected.to_ascii_lowercase()),
                            "missing {expected}: {headers}"
                        );
                    }
                    assert!(!lower.contains("cookie:") && !lower.contains("authorization:"));
                    let length: usize = lower
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    let mut body = vec![0; length];
                    stream.read_exact(&mut body).unwrap();
                    let actual: Value = serde_json::from_slice(&body).unwrap();
                    assert_eq!(
                        actual,
                        json!({
                            "context": {"client": {"clientName": "WEB", "clientVersion": "2.20260708.00.00", "hl": "en", "gl": "US", "visitorData": "fixture-visitor"}},
                            "videoId": ID,
                            "playbackContext": {"contentPlaybackContext": {"html5Preference": "HTML5_PREF_WANTS"}}
                        })
                    );
                }
                let body = String::from_utf8_lossy(&reply.body)
                    .replace("{{ORIGIN}}", &origin)
                    .into_bytes();
                let length = if reply.omit_length {
                    String::new()
                } else {
                    format!("Content-Length: {}\r\n", body.len() + reply.missing_bytes)
                };
                write!(stream, "HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\n{}{length}Connection: close\r\n\r\n", reply.status, reply.mime, reply.extra).unwrap();
                let _ = stream.write_all(&body);
            }
        });
        (page, worker)
    }

    fn reply(path: &'static str, mime: &'static str, body: impl Into<Vec<u8>>) -> Reply {
        Reply {
            path,
            mime,
            body: body.into(),
            status: 200,
            extra: "",
            missing_bytes: 0,
            omit_length: false,
        }
    }

    fn http_client() -> Client {
        Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap()
    }

    #[test]
    fn web_context_is_allowlisted_and_requires_literal_web_config() {
        let context = web_context(PAGE).unwrap();
        assert_eq!(context["client"]["clientVersion"], "2.20260708.00.00");
        assert!(context.get("user").is_none());
        for page in [
            "<p>ytcfg.set({})</p>".to_owned(),
            "<script>ytcfg.set({broken});</script>".to_owned(),
            PAGE.replace("WEB", "ANDROID"),
            PAGE.replace("2.20260708.00.00", "invalid-version"),
            PAGE.replace("fixture-visitor", "bad\\nvisitor"),
            PAGE.replace("ytcfg.set(", "other.ytcfg.set("),
        ] {
            assert!(web_context(&page).is_err(), "{page}");
        }
        let no_visitor = PAGE.replace(", \"visitorData\": \"fixture-visitor\"", "");
        assert!(
            web_context(&no_visitor).unwrap()["client"]
                .get("visitorData")
                .is_none()
        );
    }

    #[test]
    fn missing_initial_response_posts_web_request_filters_tokens_and_downloads() {
        let (page, worker) = serve(vec![
            reply("/watch?v=BaW_jenozKc", "text/html", PAGE),
            reply(
                "/youtubei/v1/player?prettyPrint=false",
                "application/json",
                PLAYER,
            ),
            reply("/media/low.mp4", "video/mp4", "media"),
            reply("/media/low.mp4", "video/mp4", "media"),
        ]);
        let mut info = super::super::extract_watch_page(&http_client(), &page, ID).unwrap();
        info.select_format(None).unwrap();
        assert_eq!(info.format_id.as_deref(), Some("18"));
        assert!(info.select_format(Some("22")).is_err());
        let expected: Value = serde_json::from_str(
            &include_str!("../../../tests/fixtures/youtube/player-api-expected.json")
                .replace("{{ORIGIN}}", &page.origin().ascii_serialization()),
        )
        .unwrap();
        let actual = super::super::comparable_info(&info);
        for (key, value) in expected.as_object().unwrap() {
            assert_eq!(&actual[key], value, "{key}");
        }
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("api.mp4");
        assert_eq!(
            crate::YoutubeDl::new()
                .unwrap()
                .download(&info, &path)
                .unwrap(),
            5
        );
        assert_eq!(std::fs::read(path).unwrap(), b"media");
        worker.join().unwrap();
    }

    #[test]
    fn malformed_or_unsupported_initial_response_never_uses_api() {
        for initial in [
            "{broken}".to_owned(),
            "null".to_owned(),
            "[]".to_owned(),
            "undefined".to_owned(),
            "{\"playabilityStatus\":{\"status\":\"LOGIN_REQUIRED\"}}".to_owned(),
            PLAYER.replace("BaW_jenozKc", "wrong_video"),
        ] {
            let html = format!("{PAGE}<script>ytInitialPlayerResponse = {initial};</script>");
            let (page, worker) = serve(vec![reply("/watch?v=BaW_jenozKc", "text/html", html)]);
            assert!(matches!(
                super::super::extract_watch_page(&http_client(), &page, ID),
                Err(Error::UnsupportedYoutube(_))
            ));
            worker.join().unwrap();
        }
    }

    #[test]
    fn rejects_api_status_encoding_size_and_invalid_json_before_probes() {
        let path = "/youtubei/v1/player?prettyPrint=false";
        let mut responses = vec![
            reply(path, "text/html", "<html>consent</html>"),
            reply(path, "application/json", "{broken}"),
            reply(path, "application/json", "null"),
            reply(
                path,
                "application/json",
                PLAYER.replace("BaW_jenozKc", "wrong_video"),
            ),
            reply(
                path,
                "application/json",
                PLAYER.replace("\"OK\"", "\"LOGIN_REQUIRED\""),
            ),
            reply(
                path,
                "application/json",
                PLAYER.replace("\"itag\": 18", "\"itag\": 37"),
            ),
            reply(
                path,
                "application/json",
                vec![b' '; MAX_PAGE_BYTES as usize + 1],
            ),
        ];
        let mut compressed = reply(path, "application/json", PLAYER);
        compressed.extra = "Content-Encoding: gzip\r\n";
        responses.push(compressed);
        let mut unbounded = reply(
            path,
            "application/json",
            vec![b' '; MAX_PAGE_BYTES as usize + 1],
        );
        unbounded.omit_length = true;
        responses.push(unbounded);
        for status in [206, 403, 503] {
            let mut response = reply(path, "application/json", PLAYER);
            response.status = status;
            responses.push(response);
        }
        for response in responses {
            let status = response.status;
            let (page, worker) = serve(vec![
                reply("/watch?v=BaW_jenozKc", "text/html", PAGE),
                response,
            ]);
            let error = super::super::extract_watch_page(&http_client(), &page, ID).unwrap_err();
            match status {
                206 => assert!(matches!(error, Error::PartialResponse)),
                403 => assert!(crate::retry::failure_delay(&error).is_none()),
                503 => assert!(crate::retry::failure_delay(&error).is_some()),
                _ => assert!(matches!(error, Error::UnsupportedYoutube(_))),
            }
            worker.join().unwrap();
        }
    }

    #[test]
    fn transient_api_failures_restart_the_entire_extraction() {
        struct LocalYoutube;
        impl crate::extractor::Extractor for LocalYoutube {
            fn key(&self) -> &'static str {
                "LocalYoutube"
            }
            fn extract(&self, client: &Client, url: &Url) -> Result<Option<crate::VideoInfo>> {
                super::super::extract_watch_page(client, url, ID).map(Some)
            }
        }
        for truncated in [false, true] {
            let mut failed = reply(
                "/youtubei/v1/player?prettyPrint=false",
                "application/json",
                "{}",
            );
            if truncated {
                failed.missing_bytes = 10;
            } else {
                failed.status = 503;
                failed.extra = "Retry-After: 0\r\n";
            }
            let (page, worker) = serve(vec![
                reply("/watch?v=BaW_jenozKc", "text/html", PAGE),
                failed,
                reply("/watch?v=BaW_jenozKc", "text/html", PAGE),
                reply(
                    "/youtubei/v1/player?prettyPrint=false",
                    "application/json",
                    PLAYER,
                ),
                reply("/media/low.mp4", "video/mp4", "media"),
            ]);
            let mut ydl = crate::YoutubeDl::new().unwrap();
            ydl.client = http_client();
            ydl.register_extractor(Box::new(LocalYoutube));
            let info = ydl
                .extract_result_with_options(
                    page.as_str(),
                    None,
                    crate::ExtractionOptions { retries: 1 },
                )
                .unwrap()
                .into_video()
                .unwrap();
            assert_eq!(info.format_id.as_deref(), Some("18"));
            worker.join().unwrap();
        }
    }
}
