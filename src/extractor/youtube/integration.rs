//! Deterministic extraction tests for the configured offline challenge path.
use super::*;
use serde_json::json;
use std::{
    collections::VecDeque,
    io::Write,
    net::TcpListener,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

const ID: &str = "BaW_jenozKc";
const WATCH: &str = "/watch?v=BaW_jenozKc";
const SCRIPT: &str = "/s/player/74edf1a3/player_ias.vflset/en_US/base.js";

struct Fake {
    _directory: tempfile::TempDir,
    path: PathBuf,
}
impl Fake {
    fn new(mode: &str) -> Self {
        static COMPILED: OnceLock<PathBuf> = OnceLock::new();
        let compiled = COMPILED.get_or_init(|| {
            let directory = tempfile::tempdir().unwrap().keep();
            let binary = directory.join(format!("solver{}", std::env::consts::EXE_SUFFIX));
            assert!(
                Command::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                    .arg(
                        Path::new(env!("CARGO_MANIFEST_DIR"))
                            .join("tests/helpers/youtube_runtime.rs")
                    )
                    .args(["--edition=2024", "-Dwarnings", "-o"])
                    .arg(&binary)
                    .status()
                    .unwrap()
                    .success()
            );
            binary
        });
        let directory = tempfile::tempdir().unwrap();
        let path = directory
            .path()
            .join(format!("{mode}{}", std::env::consts::EXE_SUFFIX));
        std::fs::copy(compiled, &path).unwrap();
        Self {
            _directory: directory,
            path,
        }
    }
    fn log(&self) -> String {
        std::fs::read_to_string(self.path.with_extension("log")).unwrap_or_default()
    }
}

struct Reply {
    path: String,
    status: u16,
    mime: &'static str,
    body: String,
    extra: &'static str,
    canonical_referrer: bool,
}
fn reply(path: &str, mime: &'static str, body: impl Into<String>) -> Reply {
    Reply {
        path: path.into(),
        status: 200,
        mime,
        body: body.into(),
        extra: "",
        canonical_referrer: false,
    }
}
fn format(itag: u64, url: &str) -> Value {
    json!({"itag":itag,"url":url,"mimeType":"video/mp4; codecs=\"avc1.42001E, mp4a.40.2\"","width":1280,"height":720})
}
fn cipher_format(itag: u64, url: &str, parameter: Option<&str>, legacy: bool) -> Value {
    let mut source = format(itag, url);
    source.as_object_mut().unwrap().remove("url");
    let mut cipher = url::form_urlencoded::Serializer::new(String::new());
    cipher.append_pair("url", url).append_pair("s", "secret");
    if let Some(parameter) = parameter {
        cipher.append_pair("sp", parameter);
    }
    source[if legacy { "cipher" } else { "signatureCipher" }] = cipher
        .finish()
        .replace("%7B%7BORIGIN%7D%7D", "{{ORIGIN}}")
        .into();
    source
}
fn response(formats: Vec<Value>) -> Value {
    json!({"playabilityStatus":{"status":"OK"},"videoDetails":{"videoId":ID,"title":"Challenge fixture"},"streamingData":{"formats":formats}})
}
fn page(response: Option<Value>) -> String {
    let config = json!({"PLAYER_JS_URL":SCRIPT,"INNERTUBE_CONTEXT":{"client":{"clientName":"WEB","clientVersion":"2.20260708.00.00"}}});
    let initial = response
        .map(|value| format!("<script>ytInitialPlayerResponse = {value};</script>"))
        .unwrap_or_default();
    format!("<script>ytcfg.set({config});</script>{initial}")
}

struct Server {
    origin: Url,
    stop: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Server {
    fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let origin_text = origin.as_str().trim_end_matches('/').to_owned();
        let worker = thread::spawn(move || {
            let mut replies: VecDeque<_> = replies.into();
            while !worker_stop.load(Ordering::Relaxed) {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => panic!("{error}"),
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
                let reply = replies
                    .pop_front()
                    .unwrap_or_else(|| panic!("unexpected request: {headers}"));
                let api = reply.path.starts_with("/youtubei/");
                let method = if api { "POST" } else { "GET" };
                assert!(
                    headers.starts_with(&format!("{method} {} HTTP/1.1\r\n", reply.path)),
                    "{headers}"
                );
                if api {
                    let length: usize = headers
                        .to_ascii_lowercase()
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length: "))
                        .unwrap()
                        .parse()
                        .unwrap();
                    let mut body = vec![0; length];
                    stream.read_exact(&mut body).unwrap();
                    assert_eq!(
                        serde_json::from_slice::<Value>(&body).unwrap()["videoId"],
                        ID
                    );
                }
                if reply.path.starts_with("/media") {
                    let referer = if reply.canonical_referrer {
                        format!("https://www.youtube.com{WATCH}")
                    } else {
                        format!("{origin_text}{WATCH}")
                    };
                    assert!(
                        headers
                            .to_ascii_lowercase()
                            .contains(&format!("referer: {referer}\r\n").to_ascii_lowercase()),
                        "{headers}"
                    );
                }
                let body = reply.body.replace("{{ORIGIN}}", &origin_text);
                write!(stream,"HTTP/1.1 {} Fixture\r\nContent-Type: {}\r\nContent-Length: {}\r\n{}Connection: close\r\n\r\n",reply.status,reply.mime,body.len(),reply.extra).unwrap();
                let _ = stream.write_all(body.as_bytes());
            }
            assert!(
                replies.is_empty(),
                "unrequested fixture replies: {}",
                replies.len()
            );
        });
        Self {
            origin,
            stop,
            worker: Some(worker),
        }
    }
    fn watch(&self) -> Url {
        self.origin.join(WATCH).unwrap()
    }
    fn source(&self) -> player::PlayerSource {
        player::PlayerSource::local(self.origin.clone()).unwrap()
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Err(error) = self.worker.take().unwrap().join()
            && !thread::panicking()
        {
            std::panic::resume_unwind(error);
        }
    }
}
fn http_client() -> Client {
    Client::builder()
        .no_proxy()
        .referer(false)
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}
fn extract(server: &Server, runtime: Option<&challenge::Runtime>) -> Result<VideoInfo> {
    extract_watch_page_with_runtime(
        &http_client(),
        &server.watch(),
        ID,
        runtime,
        Some(&server.source()),
    )
}

#[test]
fn configured_n_formats_solve_once_before_probes_and_download() {
    let fake = Fake::new("solver");
    let runtime = challenge::Runtime::new(fake.path.clone());
    let server = Server::new(vec![
        reply(
            WATCH,
            "text/html",
            page(Some(response(vec![
                format(18, "{{ORIGIN}}/media-low.mp4?n=challenge&keep=%2f"),
                format(22, "{{ORIGIN}}/media-high.mp4?n=challenge"),
            ]))),
        ),
        reply(SCRIPT, "text/javascript", "const fake = true;"),
        reply("/media-low.mp4?n=solved&keep=%2f", "video/mp4", "low"),
        reply("/media-high.mp4?n=solved", "video/mp4", "high"),
        {
            let mut download = reply("/media-high.mp4?n=solved", "video/mp4", "high");
            download.canonical_referrer = true;
            download
        },
    ]);
    let mut info = extract(&server, Some(&runtime)).unwrap();
    info.select_format(Some("18")).unwrap();
    assert_eq!(info.filesize, Some(3));
    info.select_format(None).unwrap();
    assert_eq!(info.format_id.as_deref(), Some("22"));
    assert!(info.url.as_str().ends_with("n=solved"));
    assert_eq!(
        info.webpage_url.as_str(),
        format!("https://www.youtube.com/watch?v={ID}")
    );
    assert_eq!(fake.log(), "version\nsolve\n");
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("video.mp4");
    assert_eq!(
        crate::YoutubeDl::new()
            .unwrap()
            .download(&info, &destination)
            .unwrap(),
        4
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"high");
}

#[test]
fn without_runtime_only_challenge_free_alternatives_are_probed() {
    let server = Server::new(vec![
        reply(
            WATCH,
            "text/html",
            page(Some(response(vec![
                format(18, "{{ORIGIN}}/media.mp4"),
                format(22, "{{ORIGIN}}/unresolved.mp4?n=challenge"),
            ]))),
        ),
        reply("/media.mp4", "video/mp4", "media"),
    ]);
    assert_eq!(extract(&server, None).unwrap().formats.len(), 1);
}

#[test]
fn api_token_filter_precedes_challenges_runtime_and_media_probes() {
    let fake = Fake::new("solver");
    let runtime = challenge::Runtime::new(fake.path.clone());
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(None)),
        reply(
            "/youtubei/v1/player?prettyPrint=false",
            "application/json",
            response(vec![
                format(18, "{{ORIGIN}}/media.mp4"),
                format(22, "{{ORIGIN}}/unresolved.mp4?n=challenge"),
            ])
            .to_string(),
        ),
        reply("/media.mp4", "video/mp4", "media"),
    ]);
    let info = extract(&server, Some(&runtime)).unwrap();
    assert_eq!(info.formats.len(), 1);
    assert_eq!(info.formats[0].format_id, "18");
    assert_eq!(fake.log(), "");
}

#[test]
fn api_token_exempt_n_format_is_solved_without_protected_formats() {
    let fake = Fake::new("solver");
    let runtime = challenge::Runtime::new(fake.path.clone());
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(None)),
        reply(
            "/youtubei/v1/player?prettyPrint=false",
            "application/json",
            response(vec![
                format(18, "{{ORIGIN}}/media.mp4?n=challenge"),
                format(22, "{{ORIGIN}}/protected.mp4?n=protected"),
            ])
            .to_string(),
        ),
        reply(SCRIPT, "text/javascript", "const fake=true;"),
        reply("/media.mp4?n=solved", "video/mp4", "media"),
    ]);
    let info = extract(&server, Some(&runtime)).unwrap();
    assert_eq!(info.formats.len(), 1);
    assert_eq!(info.formats[0].format_id, "18");
    assert_eq!(fake.log(), "version\nsolve\n");
}

#[test]
fn protocol_failures_are_permanent_and_never_probe_unresolved_media() {
    for mode in ["malformed", "missing", "stderr", "exit"] {
        let fake = Fake::new(mode);
        let runtime = challenge::Runtime::new(fake.path.clone());
        let server = Server::new(vec![
            reply(
                WATCH,
                "text/html",
                page(Some(response(vec![
                    format(18, "{{ORIGIN}}/direct.mp4"),
                    format(22, "{{ORIGIN}}/media.mp4?n=challenge"),
                ]))),
            ),
            reply(SCRIPT, "text/javascript", "const fake=true;"),
        ]);
        let error = extract(&server, Some(&runtime)).unwrap_err();
        assert!(matches!(error, Error::UnsupportedYoutube(_)));
        assert_eq!(crate::retry::failure_delay(&error), None);
        assert_eq!(fake.log(), "version\nsolve\n");
    }
}

#[test]
fn gated_live_drm_fragmented_adaptive_and_otf_sources_never_reach_runtime() {
    let fake = Fake::new("solver");
    let runtime = challenge::Runtime::new(fake.path.clone());
    let mut cases = Vec::new();
    let mut gated = response(vec![format(18, "{{ORIGIN}}/media.mp4?n=challenge")]);
    gated["playabilityStatus"]["status"] = "LOGIN_REQUIRED".into();
    cases.push(gated);
    let mut live = response(vec![format(18, "{{ORIGIN}}/media.mp4?n=challenge")]);
    live["videoDetails"]["isLiveContent"] = true.into();
    cases.push(live);
    let mut drm = response(vec![format(18, "{{ORIGIN}}/media.mp4?n=challenge")]);
    drm["streamingData"]["licenseInfos"] = json!([{"drmFamily":"WIDEVINE"}]);
    cases.push(drm);
    let mut adaptive = response(Vec::new());
    adaptive["streamingData"]["adaptiveFormats"] =
        json!([format(18, "{{ORIGIN}}/media.mp4?n=challenge")]);
    adaptive["streamingData"]["adaptiveFormats"][0]["targetDurationSec"] = 5.into();
    cases.push(adaptive);
    let mut otf = response(vec![format(18, "{{ORIGIN}}/media.mp4?n=challenge")]);
    otf["streamingData"]["formats"][0]["type"] = "FORMAT_STREAM_TYPE_OTF".into();
    cases.push(otf);
    for kind in [json!("FORMAT_STREAM_TYPE_UNKNOWN"), json!(7)] {
        let mut value = response(vec![format(18, "{{ORIGIN}}/media.mp4?n=challenge")]);
        value["streamingData"]["formats"][0]["type"] = kind;
        cases.push(value);
    }
    for value in cases {
        let server = Server::new(vec![reply(WATCH, "text/html", page(Some(value)))]);
        assert!(extract(&server, Some(&runtime)).is_err());
    }
    assert_eq!(fake.log(), "");
}

#[test]
fn player_network_retry_restarts_whole_extraction_and_reuses_runtime_validation() {
    struct LocalYoutube {
        runtime: challenge::Runtime,
        source: player::PlayerSource,
    }
    impl Extractor for LocalYoutube {
        fn key(&self) -> &'static str {
            "LocalYoutube"
        }
        fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
            extract_watch_page_with_runtime(
                client,
                url,
                ID,
                Some(&self.runtime),
                Some(&self.source),
            )
            .map(Some)
        }
    }
    let fake = Fake::new("solver");
    let html = page(Some(response(vec![format(
        18,
        "{{ORIGIN}}/media.mp4?n=challenge",
    )])));
    let mut failed = reply(SCRIPT, "text/javascript", "");
    failed.status = 503;
    failed.extra = "Retry-After: 0\r\n";
    let server = Server::new(vec![
        reply(WATCH, "text/html", html.clone()),
        failed,
        reply(WATCH, "text/html", html),
        reply(SCRIPT, "text/javascript", "const fake=true;"),
        reply("/media.mp4?n=solved", "video/mp4", "media"),
    ]);
    let mut ydl = crate::YoutubeDl::new().unwrap();
    ydl.client = http_client();
    ydl.register_extractor(Box::new(LocalYoutube {
        runtime: challenge::Runtime::new(fake.path.clone()),
        source: server.source(),
    }));
    let info = ydl
        .extract_result_with_options(
            server.watch().as_str(),
            Some("18"),
            crate::ExtractionOptions { retries: 1 },
        )
        .unwrap()
        .into_video()
        .unwrap();
    assert!(info.url.as_str().ends_with("n=solved"));
    assert_eq!(fake.log(), "version\nsolve\n");
}

#[test]
fn configured_challenge_free_youtube_bypasses_bad_runtime_and_missing_config() {
    let runtime = challenge::Runtime::new("/nonexistent-deno".into());
    let value = response(vec![format(18, "{{ORIGIN}}/media.mp4")]);
    let html = format!("<script>ytInitialPlayerResponse = {value};</script>");
    let server = Server::new(vec![
        reply(WATCH, "text/html", html),
        reply("/media.mp4", "video/mp4", "media"),
    ]);
    assert_eq!(extract(&server, Some(&runtime)).unwrap().formats.len(), 1);
}

#[test]
fn missing_or_invalid_runtime_and_player_config_do_not_probe_media() {
    for runtime in [
        None,
        Some(challenge::Runtime::new("/nonexistent-deno".into())),
    ] {
        let html = page(Some(response(vec![format(
            18,
            "{{ORIGIN}}/media.mp4?n=challenge",
        )])));
        let server = Server::new(vec![reply(WATCH, "text/html", html)]);
        let error = extract(&server, runtime.as_ref()).unwrap_err();
        assert!(matches!(error, Error::UnsupportedYoutube(_)));
        assert_eq!(crate::retry::failure_delay(&error), None);
    }
    let fake = Fake::new("solver");
    let runtime = challenge::Runtime::new(fake.path.clone());
    let value = response(vec![format(18, "{{ORIGIN}}/media.mp4?n=challenge")]);
    let server = Server::new(vec![reply(
        WATCH,
        "text/html",
        format!("<script>ytInitialPlayerResponse = {value};</script>"),
    )]);
    assert!(extract(&server, Some(&runtime)).is_err());
    assert_eq!(fake.log(), "");
}

#[test]
fn real_saved_player_extracts_challenged_progressive_metadata() {
    let Some(path) = std::env::var_os("YT_DLP_RS_TEST_DENO") else {
        return;
    };
    let runtime = challenge::Runtime::new(path.into());
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/youtube/n-extraction.json"
    ))
    .unwrap();
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(Some(fixture))),
        reply(
            SCRIPT,
            "text/javascript",
            include_str!("../../../tests/fixtures/youtube/challenges/player-74edf1a3.js"),
        ),
        reply("/media.mp4?n=9nRTxrbM1f0yHg", "video/mp4", "media"),
    ]);
    let mut info = extract(&server, Some(&runtime)).unwrap();
    info.select_format(None).unwrap();
    assert_eq!(info.format_id.as_deref(), Some("18"));
    assert_eq!(info.filesize, Some(5));
    assert!(info.url.as_str().ends_with("n=9nRTxrbM1f0yHg"));
    let expected: Value = serde_json::from_str(
        &include_str!("../../../tests/fixtures/youtube/n-extraction-expected.json")
            .replace("{{ORIGIN}}", server.origin.as_str().trim_end_matches('/')),
    )
    .unwrap();
    let actual = serde_json::to_value(&info).unwrap();
    for (key, value) in expected.as_object().unwrap() {
        assert_eq!(&actual[key], value, "{key}");
    }
}

#[test]
fn signatures_and_combined_n_challenges_use_the_existing_complete_batch_boundary() {
    for (legacy, parameter, url, target) in [
        (
            false,
            None,
            "{{ORIGIN}}/media.mp4",
            "/media.mp4?signature=signature-value",
        ),
        (
            true,
            None,
            "{{ORIGIN}}/media.mp4?keep=%2f",
            "/media.mp4?keep=%2f&signature=signature-value",
        ),
        (
            false,
            Some("lsig"),
            "{{ORIGIN}}/media.mp4?keep=%2f&n=challenge&bare#fragment",
            "/media.mp4?keep=%2f&n=solved&bare&lsig=signature-value",
        ),
    ] {
        let fake = Fake::new("solver");
        let runtime = challenge::Runtime::new(fake.path.clone());
        let server = Server::new(vec![
            reply(
                WATCH,
                "text/html",
                page(Some(response(vec![cipher_format(
                    18, url, parameter, legacy,
                )]))),
            ),
            reply(SCRIPT, "text/javascript", "const fake=true;"),
            reply(target, "video/mp4", "media"),
            {
                let mut download = reply(target, "video/mp4", "media");
                download.canonical_referrer = true;
                download
            },
        ]);
        let mut info = extract(&server, Some(&runtime)).unwrap();
        info.select_format(Some("18")).unwrap();
        // Metadata records the final probe URL; HTTP transport drops fragments.
        let expected = format!("{}{target}", server.origin.as_str().trim_end_matches('/'));
        assert_eq!(info.url.as_str(), expected);
        assert_eq!(info.filesize, Some(5));
        assert_eq!(fake.log(), "version\nsolve\n");
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("signed.mp4");
        assert_eq!(
            crate::YoutubeDl::new()
                .unwrap()
                .download(&info, &target)
                .unwrap(),
            5
        );
        assert_eq!(std::fs::read(target).unwrap(), b"media");
    }
}

#[test]
fn api_cipher_token_policy_and_default_runtime_boundary_are_preserved() {
    let fake = Fake::new("solver");
    let runtime = challenge::Runtime::new(fake.path.clone());
    let mut protected = cipher_format(22, "{{ORIGIN}}/protected.mp4", None, false);
    protected["signatureCipher"] = "invalid-protected-cipher".into();
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(None)),
        reply(
            "/youtubei/v1/player?prettyPrint=false",
            "application/json",
            response(vec![
                cipher_format(18, "{{ORIGIN}}/media.mp4", Some("sig"), false),
                protected,
            ])
            .to_string(),
        ),
        reply(SCRIPT, "text/javascript", "const fake=true;"),
        reply("/media.mp4?sig=signature-value", "video/mp4", "media"),
    ]);
    let info = extract(&server, Some(&runtime)).unwrap();
    assert_eq!(info.formats.len(), 1);
    assert_eq!(info.formats[0].format_id, "18");
    assert_eq!(fake.log(), "version\nsolve\n");
    let server = Server::new(vec![reply(
        WATCH,
        "text/html",
        page(Some(response(vec![cipher_format(
            18,
            "{{ORIGIN}}/media.mp4",
            None,
            false,
        )]))),
    )]);
    assert!(extract(&server, None).is_err());
}

#[test]
fn malformed_or_ambiguous_ciphers_never_reach_runtime_or_media() {
    let fake = Fake::new("solver");
    let runtime = challenge::Runtime::new(fake.path.clone());
    let mut ambiguous = cipher_format(18, "{{ORIGIN}}/media.mp4", None, false);
    ambiguous["url"] = "{{ORIGIN}}/direct.mp4".into();
    let mut missing = cipher_format(18, "{{ORIGIN}}/media.mp4", None, false);
    missing["signatureCipher"] = "url=https%3A%2F%2Fexample.invalid%2Fv".into();
    for source in [
        ambiguous,
        missing,
        cipher_format(18, "{{ORIGIN}}/media.mp4?sig=existing", Some("sig"), false),
        cipher_format(18, "{{ORIGIN}}/media.mp4", Some("n"), false),
        cipher_format(18, "{{ORIGIN}}/n/path/media.mp4", None, false),
    ] {
        let server = Server::new(vec![reply(
            WATCH,
            "text/html",
            page(Some(response(vec![source]))),
        )]);
        assert!(extract(&server, Some(&runtime)).is_err());
    }
    assert_eq!(fake.log(), "");
}

#[test]
fn incomplete_signature_batch_cannot_probe_unchallenged_alternatives() {
    let fake = Fake::new("missing");
    let runtime = challenge::Runtime::new(fake.path.clone());
    let server = Server::new(vec![
        reply(
            WATCH,
            "text/html",
            page(Some(response(vec![
                format(18, "{{ORIGIN}}/plain.mp4"),
                cipher_format(22, "{{ORIGIN}}/media.mp4?n=challenge", Some("sig"), false),
            ]))),
        ),
        reply(SCRIPT, "text/javascript", "const fake=true;"),
    ]);
    let error = extract(&server, Some(&runtime)).unwrap_err();
    assert!(matches!(error, Error::UnsupportedYoutube(_)));
    assert_eq!(crate::retry::failure_delay(&error), None);
}

#[test]
fn real_saved_player_signature_and_n_extraction_match_upstream_metadata_and_download() {
    let Some(path) = std::env::var_os("YT_DLP_RS_TEST_DENO") else {
        return;
    };
    let runtime = challenge::Runtime::new(path.into());
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/youtube/signature-extraction.json"
    ))
    .unwrap();
    let expected: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/youtube/signature-extraction-expected.json"
    ))
    .unwrap();
    let media_path = expected["formats"][0]["url"]
        .as_str()
        .unwrap()
        .strip_prefix("{{ORIGIN}}")
        .unwrap();
    let mut download = reply(media_path, "video/mp4", "media");
    download.canonical_referrer = true;
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(Some(fixture))),
        reply(
            SCRIPT,
            "text/javascript",
            include_str!("../../../tests/fixtures/youtube/challenges/player-74edf1a3.js"),
        ),
        reply(media_path, "video/mp4", "media"),
        download,
    ]);
    let mut info = extract(&server, Some(&runtime)).unwrap();
    info.select_format(Some("18")).unwrap();
    let expected: Value = serde_json::from_str(
        &expected
            .to_string()
            .replace("{{ORIGIN}}", server.origin.as_str().trim_end_matches('/')),
    )
    .unwrap();
    let actual = serde_json::to_value(&info).unwrap();
    for (key, value) in expected.as_object().unwrap() {
        assert_eq!(&actual[key], value, "{key}");
    }
    let directory = tempfile::tempdir().unwrap();
    let target = directory.path().join("signed.mp4");
    assert_eq!(
        crate::YoutubeDl::new()
            .unwrap()
            .download(&info, &target)
            .unwrap(),
        5
    );
    assert_eq!(std::fs::read(target).unwrap(), b"media");
}

#[test]
fn saved_adaptive_formats_probe_and_select_through_public_orchestration() {
    struct LocalYoutube;
    impl Extractor for LocalYoutube {
        fn key(&self) -> &'static str {
            "LocalYoutube"
        }
        fn extract(&self, client: &Client, url: &Url) -> Result<Option<VideoInfo>> {
            extract_watch_page(client, url, ID).map(Some)
        }
    }
    for (enable_merge, selector, selected) in [
        (false, None, "18"),
        (true, None, "137+140"),
        (false, Some("140+137"), "140+137"),
    ] {
        let saved: Value = serde_json::from_str(include_str!(
            "../../../tests/fixtures/youtube/adaptive.json"
        ))
        .unwrap();
        let server = Server::new(vec![
            reply(WATCH, "text/html", page(Some(saved))),
            reply("/media/muxed.mp4", "video/mp4", "muxed"),
            reply("/media/video.mp4", "video/mp4", "picture"),
            reply("/media/audio-low.m4a", "audio/mp4", "low"),
            reply("/media/audio.m4a", "audio/mp4", "high"),
            reply("/media/video.webm", "video/webm", "webpic"),
            reply("/media/audio.webm", "audio/webm", "wb"),
        ]);
        let mut ydl = crate::YoutubeDl::with_youtube_options(YoutubeOptions {
            include_translated_subtitles: false,
            js_runtime: Some(PathBuf::from("unused-adaptive-runtime")),
        })
        .unwrap();
        ydl.register_extractor(Box::new(LocalYoutube));
        if enable_merge {
            // Configuration changes selection; simulation never inspects the executable.
            ydl = ydl.with_ffmpeg("unused-adaptive-ffmpeg");
        }
        let info = ydl
            .extract_info_with_format(server.watch().as_str(), selector)
            .unwrap();
        assert_eq!(info.format_id.as_deref(), Some(selected));
        let expected: Value = serde_json::from_str(
            &include_str!("../../../tests/fixtures/youtube/adaptive-expected.json")
                .replace("{{ORIGIN}}", server.origin.as_str().trim_end_matches('/')),
        )
        .unwrap();
        let expected_formats: Vec<VideoFormat> =
            serde_json::from_value(expected["formats"].clone()).unwrap();
        assert_eq!(
            serde_json::to_value(&info.formats).unwrap(),
            serde_json::to_value(expected_formats).unwrap()
        );
        assert_eq!(info.title, expected["title"]);
        assert_eq!(info.webpage_url.as_str(), expected["webpage_url"]);
        if selected == "18" {
            assert!(info.requested_formats.is_empty());
            assert_eq!(info.filesize, Some(5));
            assert_eq!(info.height, Some(360));
        } else {
            assert_eq!(info.requested_formats.len(), 2);
            assert_eq!(info.height, Some(1080));
            assert_eq!(info.ext, "mp4");
            assert_eq!(info.filesize, None);
            assert_eq!(info.url, info.requested_formats[0].url);
        }
    }
}

#[test]
fn adaptive_without_standalone_requires_explicit_stream_or_configured_merge() {
    let mut saved: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/youtube/adaptive.json"
    ))
    .unwrap();
    saved["streamingData"]["formats"] = json!([]);
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(Some(saved))),
        reply("/media/video.mp4", "video/mp4", "picture"),
        reply("/media/audio-low.m4a", "audio/mp4", "low"),
        reply("/media/audio.m4a", "audio/mp4", "high"),
        reply("/media/video.webm", "video/webm", "webpic"),
        reply("/media/audio.webm", "audio/webm", "wb"),
    ]);
    let mut info = extract(&server, None).unwrap();
    assert!(matches!(
        info.select_format(None),
        Err(Error::FormatSelection(_))
    ));
    info.select_format(Some("bestaudio")).unwrap();
    assert_eq!(info.format_id.as_deref(), Some("140"));
    assert_eq!(info.stream_kind, StreamKind::AudioOnly);
    info.select_format_with_merge(None, true).unwrap();
    assert_eq!(info.format_id.as_deref(), Some("137+140"));
}

mod manifests;

mod captions;
