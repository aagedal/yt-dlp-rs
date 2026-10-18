//! Bounded player-script discovery and transport, separate from EJS execution.
//! This private boundary is not called by extraction until runtime gates pass.
use super::{client::literal_config, direct, unsupported};
use crate::{Error, Result};
use reqwest::{
    blocking::Client,
    header::{CONTENT_ENCODING, CONTENT_TYPE, LOCATION},
    redirect::Policy,
};
use serde_json::Value;
use std::{io::Read, time::Duration};
use url::Url;

const MAX_PLAYER_BYTES: u64 = 8 * 1024 * 1024;
const MAX_REDIRECTS: usize = 10;

/// A fixed origin and transport with automatic redirects disabled. Page-supplied
/// hosts and the general media client's redirect policy are not used. Explicit
/// global HTTP defaults apply only within the validated canonical origin.
pub(super) struct PlayerSource {
    origin: Url,
    client: Client,
}

impl PlayerSource {
    pub(super) fn new(headers: reqwest::header::HeaderMap) -> Result<Self> {
        Self::with_origin(Url::parse("https://www.youtube.com/")?, false, headers)
    }

    #[cfg(test)]
    pub(super) fn local(origin: Url) -> Result<Self> {
        assert_eq!(origin.scheme(), "http");
        assert_eq!(origin.host_str(), Some("127.0.0.1"));
        Self::with_origin(origin, true, reqwest::header::HeaderMap::new())
    }

    // Origin substitution is private; only tests supply loopback HTTP origins.
    fn with_origin(
        origin: Url,
        no_proxy: bool,
        headers: reqwest::header::HeaderMap,
    ) -> Result<Self> {
        let mut builder = Client::builder()
            .user_agent(concat!("yt-dlp-rs/", env!("CARGO_PKG_VERSION")))
            .default_headers(headers)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(300))
            .redirect(Policy::none())
            .referer(false);
        if no_proxy {
            builder = builder.no_proxy();
        }
        Ok(Self {
            origin,
            client: builder.build()?,
        })
    }

    pub(super) fn discover(&self, html: &str) -> Result<Url> {
        let config = literal_config(html)?;
        let raw = if let Some(raw) = config.get("PLAYER_JS_URL") {
            raw.as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| unsupported("invalid PLAYER_JS_URL"))?
                .to_owned()
        } else {
            let contexts = config
                .get("WEB_PLAYER_CONTEXT_CONFIGS")
                .and_then(Value::as_object)
                .ok_or_else(|| unsupported("no literal YouTube player script configuration"))?;
            let mut found = None;
            for context in contexts.values() {
                let Some(raw) = context.get("jsUrl") else {
                    continue;
                };
                let raw = raw
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| unsupported("invalid player context jsUrl"))?;
                if found.is_some_and(|previous| previous != raw) {
                    return Err(unsupported("ambiguous player context script URLs"));
                }
                found = Some(raw);
            }
            found
                .ok_or_else(|| unsupported("no literal YouTube player script URL"))?
                .to_owned()
        };
        let url = self.resolve(&self.origin, &raw)?;
        self.validate(&url)?;
        Ok(url)
    }

    fn resolve(&self, base: &Url, raw: &str) -> Result<Url> {
        if raw.is_empty()
            || raw.len() > 4096
            || raw
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == '\\')
        {
            return Err(unsupported("invalid player script URL text"));
        }
        let authority = raw
            .strip_prefix("//")
            .or_else(|| raw.split_once("://").map(|(_, rest)| rest));
        if authority.is_some_and(|rest| {
            rest.split(['/', '?', '#'])
                .next()
                .is_some_and(|host| host.contains('@'))
        }) {
            return Err(unsupported(
                "player script URL must not contain credentials",
            ));
        }
        Ok(base.join(raw)?)
    }

    /// Validate before the first request and before following every redirect.
    /// Pin player identity across redirects, allowing only known path variants.
    fn validate<'a>(&self, url: &'a Url) -> Result<&'a str> {
        if url.origin() != self.origin.origin()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(unsupported(
                "player script URL must use the canonical origin without credentials, query, or fragment",
            ));
        }
        let parts: Vec<_> = url.path().split('/').collect();
        if parts.len() < 6 || parts[..3] != ["", "s", "player"] {
            return Err(unsupported("unrecognized player script path"));
        }
        let id = parts[3];
        if !(8..=64).contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(unsupported("invalid player script identity"));
        }
        let tail = &parts[4..];
        let supported = match tail {
            [variant, locale, "base.js"] => {
                matches!(
                    *variant,
                    "player_ias.vflset"
                        | "player_ias_tcc.vflset"
                        | "player_ias_tce.vflset"
                        | "player_es5.vflset"
                        | "player_es6.vflset"
                        | "player_es6_tcc.vflset"
                        | "player_es6_tce.vflset"
                        | "house_brand_player.vflset"
                ) && !locale.is_empty()
                    && locale.len() <= 32
                    && locale
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
            }
            ["tv-player-ias.vflset", "tv-player-ias.js"]
            | ["tv-player-es6.vflset", "tv-player-es6.js"]
            | ["player-plasma-ias-phone-en_US.vflset", "base.js"] => true,
            _ => false,
        };
        if !supported {
            return Err(unsupported("unrecognized player script variant"));
        }
        Ok(id)
    }

    pub(super) fn fetch(&self, url: &Url) -> Result<String> {
        let id = self.validate(url)?.to_owned();
        let mut current = url.clone();
        for redirects in 0..=MAX_REDIRECTS {
            if self.validate(&current)? != id {
                return Err(unsupported(
                    "player script redirect changed player identity",
                ));
            }
            let response = direct::request(&self.client, &current, None)?;
            if response.status().is_redirection() {
                if !matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308) {
                    return Err(unsupported("unsupported player script redirect status"));
                }
                if redirects == MAX_REDIRECTS {
                    return Err(unsupported("too many player script redirects"));
                }
                let mut locations = response.headers().get_all(LOCATION).iter();
                let location = locations
                    .next()
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| unsupported("missing or invalid player redirect location"))?;
                if locations.next().is_some() {
                    return Err(unsupported("multiple player redirect locations"));
                }
                current = self.resolve(&current, location)?;
                continue;
            }
            if !response.status().is_success() {
                return Err(unsupported("player script response is not successful"));
            }
            if response.headers().get_all(CONTENT_TYPE).iter().count() != 1
                || response.headers().get_all(CONTENT_ENCODING).iter().count() > 1
            {
                return Err(unsupported("ambiguous player script response headers"));
            }
            if !matches!(
                direct::response_mime(&response).as_str(),
                "text/javascript" | "application/javascript" | "application/x-javascript"
            ) {
                return Err(unsupported("player script response is not JavaScript"));
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
                        return Err(unsupported("only UTF-8/ASCII player scripts are supported"));
                    }
                }
            }
            if response
                .headers()
                .get(CONTENT_ENCODING)
                .is_some_and(|v| !v.as_bytes().eq_ignore_ascii_case(b"identity"))
            {
                return Err(unsupported("compressed player scripts are not supported"));
            }
            if response
                .content_length()
                .is_some_and(|n| n > MAX_PLAYER_BYTES)
            {
                return Err(unsupported("player script exceeds 8 MiB"));
            }
            let mut bytes = Vec::new();
            response
                .take(MAX_PLAYER_BYTES + 1)
                .read_to_end(&mut bytes)
                .map_err(Error::ExtractionRead)?;
            if bytes.len() as u64 > MAX_PLAYER_BYTES {
                return Err(unsupported("player script exceeds 8 MiB"));
            }
            if bytes.is_empty() {
                return Err(unsupported("player script is empty"));
            }
            return String::from_utf8(bytes).map_err(|_| unsupported("player script is not UTF-8"));
        }
        unreachable!("last redirect is rejected")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, net::TcpListener, thread, time::Instant};

    const PATH: &str = "/s/player/74edf1a3/player_ias.vflset/en_US/base.js";
    const SAVED: &str =
        include_str!("../../../tests/fixtures/youtube/challenges/player-74edf1a3.js");

    fn html(value: Value) -> String {
        format!("<script>ytcfg.set({value});</script>")
    }

    #[test]
    fn discovers_config_urls_with_precedence_and_literal_json_escapes() {
        let source = PlayerSource::new(reqwest::header::HeaderMap::new()).unwrap();
        for value in [
            serde_json::json!({"PLAYER_JS_URL": PATH}),
            serde_json::json!({"PLAYER_JS_URL": format!("https://www.youtube.com{PATH}")}),
            serde_json::json!({"PLAYER_JS_URL": format!("//www.youtube.com{PATH}")}),
            serde_json::json!({"WEB_PLAYER_CONTEXT_CONFIGS": {"main": {"jsUrl": PATH}}}),
            serde_json::json!({"WEB_PLAYER_CONTEXT_CONFIGS": {"a": {"jsUrl": PATH}, "b": {"jsUrl": PATH}}}),
            serde_json::json!({"PLAYER_JS_URL": PATH, "WEB_PLAYER_CONTEXT_CONFIGS": {"bad": {"jsUrl": "https://invalid.example/evil.js"}}}),
        ] {
            assert_eq!(
                source.discover(&html(value)).unwrap().as_str(),
                format!("https://www.youtube.com{PATH}")
            );
        }
        let escaped = r#"<script>ytcfg.set({"PLAYER_JS_URL":"\/s\/player\/74edf1a3\/player_ias.vflset\/en_US\/base.js"});</script>"#;
        assert!(source.discover(escaped).is_ok());
        let merged = format!(
            "{}{}",
            html(serde_json::json!({"PLAYER_JS_URL":"bad"})),
            html(serde_json::json!({"PLAYER_JS_URL":PATH}))
        );
        assert!(source.discover(&merged).is_ok());
        let fallback = html(serde_json::json!({"WEB_PLAYER_CONTEXT_CONFIGS":{"a":{"other":1}}}));
        assert!(source.discover(&fallback).is_err());
    }

    #[test]
    fn never_evaluates_or_guesses_missing_or_ambiguous_configuration() {
        let source = PlayerSource::new(reqwest::header::HeaderMap::new()).unwrap();
        for page in [
            "<p>ytcfg.set({\"PLAYER_JS_URL\":\"ignored\"})</p>".to_owned(),
            format!("<script src='{PATH}'></script>"),
            "<script>ytcfg.set(makeConfig());</script>".into(),
            "<script>ytcfg.set({broken});</script>".into(),
            "<script>ytcfg.set({});</script>".into(),
            html(serde_json::json!({"PLAYER_JS_URL": null})),
            html(serde_json::json!({"PLAYER_JS_URL": ""})),
            html(serde_json::json!({"WEB_PLAYER_CONTEXT_CONFIGS":[]})),
            html(serde_json::json!({"WEB_PLAYER_CONTEXT_CONFIGS":{"a":{"jsUrl":null}}})),
            html(
                serde_json::json!({"WEB_PLAYER_CONTEXT_CONFIGS":{"a":{"jsUrl":PATH},"b":{"jsUrl":"/s/player/12345678/player_ias.vflset/en_US/base.js"}}}),
            ),
        ] {
            assert!(
                matches!(source.discover(&page), Err(Error::UnsupportedYoutube(_))),
                "{page}"
            );
        }
    }

    #[test]
    fn recognized_variants_and_strict_player_url_boundaries() {
        let source = PlayerSource::new(reqwest::header::HeaderMap::new()).unwrap();
        for variant in [
            "player_ias.vflset/en_US/base.js",
            "player_ias_tcc.vflset/en_US/base.js",
            "player_ias_tce.vflset/en_US/base.js",
            "player_es5.vflset/en_US/base.js",
            "player_es6.vflset/en_US/base.js",
            "player_es6_tcc.vflset/en_US/base.js",
            "player_es6_tce.vflset/en_US/base.js",
            "tv-player-ias.vflset/tv-player-ias.js",
            "tv-player-es6.vflset/tv-player-es6.js",
            "player-plasma-ias-phone-en_US.vflset/base.js",
            "house_brand_player.vflset/en_US/base.js",
            "player_ias.vflset/nb_NO/base.js",
        ] {
            let url = Url::parse(&format!(
                "https://www.youtube.com/s/player/ABCdef0123/{variant}"
            ))
            .unwrap();
            assert_eq!(source.validate(&url).unwrap(), "ABCdef0123");
        }
        for raw in [
            format!("http://www.youtube.com{PATH}"),
            format!("https://www.youtube.com:444{PATH}"),
            format!("https://www.youtube.com.evil.invalid{PATH}"),
            format!("https://youtube.com{PATH}"),
            format!("https://user:pass@www.youtube.com{PATH}"),
            format!("https://@www.youtube.com{PATH}"),
            format!("https://www.youtube.com{PATH}?x=1"),
            format!("https://www.youtube.com{PATH}#fragment"),
            "/iframe_api".into(),
            PATH.replace("74edf1a3", "74edf1a"),
            PATH.replace("74edf1a3", "not_hex!"),
            PATH.replace("74edf1a3", &"a".repeat(65)),
            PATH.replace("en_US", "en-US"),
            PATH.replace("base.js", "other.js"),
            format!("{PATH}/extra"),
            PATH.replace("player_ias", "unknown"),
            PATH.replace("74edf1a3", "%37%34edf1a3"),
            format!(" {PATH}"),
            format!("{PATH}\n"),
            PATH.replace('/', "\\"),
            "data:text/javascript,alert(1)".into(),
            "javascript:alert(1)".into(),
        ] {
            assert!(
                source
                    .discover(&html(serde_json::json!({"PLAYER_JS_URL":raw})))
                    .is_err(),
                "{raw}"
            );
        }
    }

    struct Reply {
        path: String,
        status: u16,
        mime: &'static str,
        extra: String,
        body: Vec<u8>,
        length: Option<usize>,
    }

    impl Reply {
        fn script(body: &[u8]) -> Self {
            Self {
                path: PATH.into(),
                status: 200,
                mime: "text/javascript; charset=utf-8",
                extra: String::new(),
                body: body.to_vec(),
                length: Some(body.len()),
            }
        }
        fn redirect(path: &str, location: &str) -> Self {
            Self {
                path: path.into(),
                status: 302,
                mime: "text/plain",
                extra: format!("Location: {location}\r\n"),
                body: Vec::new(),
                length: Some(0),
            }
        }
    }

    fn serve(replies: Vec<Reply>) -> (PlayerSource, thread::JoinHandle<()>) {
        serve_with_headers(replies, reqwest::header::HeaderMap::new())
    }

    fn serve_with_headers(
        replies: Vec<Reply>,
        headers: reqwest::header::HeaderMap,
    ) -> (PlayerSource, thread::JoinHandle<()>) {
        let expected_headers = headers.clone();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let source = PlayerSource::with_origin(origin, true, headers).unwrap();
        let worker = thread::spawn(move || {
            for reply in replies {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "missing player request");
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(error) => panic!("{error}"),
                    }
                };
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
                assert!(
                    headers.starts_with(&format!("GET {} HTTP/1.1\r\n", reply.path)),
                    "{headers}"
                );
                let lower = headers.to_ascii_lowercase();
                assert!(lower.contains("accept-encoding: identity\r\n"));
                assert!(!lower.contains("cookie:"));
                for name in ["authorization", "referer"] {
                    assert_eq!(
                        lower.contains(&format!("{name}:")),
                        expected_headers.contains_key(name)
                    );
                }
                for (name, value) in &expected_headers {
                    if name.as_str() != "accept-encoding" {
                        assert!(headers.contains(&format!(
                            "{}: {}\r\n",
                            name.as_str(),
                            value.to_str().unwrap()
                        )));
                    }
                }
                let length = reply
                    .length
                    .map(|n| format!("Content-Length: {n}\r\n"))
                    .unwrap_or_default();
                write!(stream,"HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\n{}{length}Connection: close\r\n\r\n",reply.status,reply.mime,reply.extra).unwrap();
                let _ = stream.write_all(&reply.body);
            }
            // Give a wrongly followed redirect a chance to connect while listening.
            thread::sleep(Duration::from_millis(20));
            assert!(
                matches!(listener.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock),
                "unexpected player request"
            );
        });
        (source, worker)
    }

    fn url(source: &PlayerSource) -> Url {
        source
            .discover(&html(serde_json::json!({"PLAYER_JS_URL":PATH})))
            .unwrap()
    }

    #[test]
    fn acquires_exact_saved_player_without_media_or_runtime_requests() {
        let (source, worker) = serve(vec![Reply::script(SAVED.as_bytes())]);
        assert_eq!(source.fetch(&url(&source)).unwrap(), SAVED);
        worker.join().unwrap();
    }

    #[test]
    fn explicit_http_defaults_apply_within_the_validated_player_origin() {
        let headers = crate::HttpOptions {
            user_agent: Some("FixturePlayer/1".into()),
            referer: Some("https://example.test/explicit".into()),
            headers: vec![
                ("x-test".into(), "player".into()),
                ("authorization".into(), "Bearer fixture".into()),
                ("accept-encoding".into(), "gzip".into()),
            ],
        }
        .header_map()
        .unwrap();
        let (source, worker) = serve_with_headers(
            vec![
                Reply::redirect(PATH, PATH),
                Reply::script(b"const player = true;"),
            ],
            headers.clone(),
        );
        assert_eq!(source.fetch(&url(&source)).unwrap(), "const player = true;");
        worker.join().unwrap();
        let trap = TcpListener::bind("127.0.0.1:0").unwrap();
        trap.set_nonblocking(true).unwrap();
        let location = format!("http://{}{PATH}", trap.local_addr().unwrap());
        let (source, worker) = serve_with_headers(vec![Reply::redirect(PATH, &location)], headers);
        assert!(source.fetch(&url(&source)).is_err());
        worker.join().unwrap();
        assert!(
            matches!(trap.accept(), Err(error) if error.kind() == std::io::ErrorKind::WouldBlock)
        );
    }

    #[test]
    fn follows_only_valid_same_player_redirects_and_caps_loops() {
        let next = PATH.replace("en_US", "nb_NO");
        let mut script = Reply::script(b"const player = 'ok';");
        script.path = next.clone();
        let (source, worker) = serve(vec![Reply::redirect(PATH, &next), script]);
        assert_eq!(source.fetch(&url(&source)).unwrap(), "const player = 'ok';");
        worker.join().unwrap();
        let (source, worker) = serve(
            (0..=MAX_REDIRECTS)
                .map(|_| Reply::redirect(PATH, PATH))
                .collect(),
        );
        let error = source.fetch(&url(&source)).unwrap_err();
        assert!(error.to_string().contains("too many"));
        worker.join().unwrap();
    }

    #[test]
    fn rejects_unsafe_redirects_before_requesting_the_target() {
        let trap = TcpListener::bind("127.0.0.1:0").unwrap();
        trap.set_nonblocking(true).unwrap();
        for location in [
            format!("http://{}{PATH}", trap.local_addr().unwrap()),
            "https://evil.invalid/steal.js".into(),
            PATH.replace("74edf1a3", "12345678"),
            format!("{PATH}?v=1"),
            format!("{PATH}#f"),
            PATH.replace("base.js", "unknown.js"),
            "/iframe_api".into(),
            "file:///tmp/player.js".into(),
        ] {
            let (source, worker) = serve(vec![Reply::redirect(PATH, &location)]);
            assert!(
                matches!(
                    source.fetch(&url(&source)),
                    Err(Error::UnsupportedYoutube(_))
                ),
                "{location}"
            );
            worker.join().unwrap();
            assert!(
                matches!(trap.accept(),Err(error) if error.kind()==std::io::ErrorKind::WouldBlock)
            );
        }
        let mut redirect = Reply::redirect(PATH, PATH);
        redirect.extra.push_str(&format!("Location: {PATH}\r\n"));
        let (source, worker) = serve(vec![redirect]);
        assert!(
            source
                .fetch(&url(&source))
                .unwrap_err()
                .to_string()
                .contains("multiple")
        );
        worker.join().unwrap();
    }

    #[test]
    fn invalid_response_status_encoding_type_and_body_never_become_scripts() {
        let mut replies = Vec::new();
        for mime in [
            "text/html",
            "application/octet-stream",
            "text/plain",
            "text/javascript; charset=iso-8859-1",
        ] {
            let mut reply = Reply::script(b"script");
            reply.mime = mime;
            replies.push(reply);
        }
        let mut compressed = Reply::script(b"script");
        compressed.extra = "Content-Encoding: gzip\r\n".into();
        replies.push(compressed);
        for extra in [
            "Content-Encoding: identity\r\nContent-Encoding: gzip\r\n",
            "Content-Type: text/html\r\n",
        ] {
            let mut duplicate = Reply::script(b"not a complete player");
            duplicate.extra = extra.into();
            replies.push(duplicate);
        }
        let mut partial = Reply::script(b"const partial = true;");
        partial.status = 206;
        partial.extra = "Content-Range: bytes 0-20/100\r\n".into();
        replies.push(partial);
        for status in [204, 206, 300, 304, 404] {
            let mut reply = Reply::script(b"");
            reply.status = status;
            replies.push(reply);
        }
        replies.push(Reply::script(&[0xff]));
        replies.push(Reply::script(b""));
        let mut announced = Reply::script(b"short");
        announced.length = Some(MAX_PLAYER_BYTES as usize + 1);
        replies.push(announced);
        let mut oversized = Reply::script(&vec![b'x'; MAX_PLAYER_BYTES as usize + 1]);
        oversized.length = None;
        replies.push(oversized);
        for reply in replies {
            let (source, worker) = serve(vec![reply]);
            assert!(source.fetch(&url(&source)).is_err());
            worker.join().unwrap();
        }
    }

    #[test]
    fn transient_status_and_interrupted_body_preserve_extraction_retry_classification() {
        let mut transient = Reply::script(b"");
        transient.status = 503;
        transient.extra = "Retry-After: 5\r\n".into();
        let (source, worker) = serve(vec![transient]);
        let error = source.fetch(&url(&source)).unwrap_err();
        assert!(matches!(error, Error::HttpRetryAfter { .. }));
        assert_eq!(
            crate::retry::failure_delay(&error),
            Some(Duration::from_secs(5))
        );
        worker.join().unwrap();
        let mut truncated = Reply::script(b"part");
        truncated.length = Some(20);
        let (source, worker) = serve(vec![truncated]);
        let error = source.fetch(&url(&source)).unwrap_err();
        assert!(matches!(error, Error::ExtractionRead(_)));
        assert_eq!(crate::retry::failure_delay(&error), Some(Duration::ZERO));
        worker.join().unwrap();
    }
}
