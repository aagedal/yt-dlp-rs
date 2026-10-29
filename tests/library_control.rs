//! Library call controls: inline manifests (`extract_manifest`), request
//! scopes whose headers and cookies reach only their origin, extraction
//! cancellation, and `Interrupt::stop` (keep the partial file) versus
//! `Interrupt::cancel` (discard it). Local HTTP servers only.
use std::{
    io::{BufRead, BufReader, Read, Write},
    net::TcpListener,
    ops::ControlFlow,
    sync::{Arc, Mutex},
    thread,
};
use url::Url;
use yt_dlp_rs::{
    DownloadControl, DownloadOptions, DownloadPhase, Error, ExtractResult, ExtractionOptions,
    Interrupt, Manifest, ManifestKind, RequestScope, VideoInfo, YoutubeDl,
};

/// Each request's path and lower-cased header names and values.
type Requests = Arc<Mutex<Vec<(String, Vec<(String, String)>)>>>;

/// A local server answering `routes(path)` with a body (404 when `None`),
/// recording each request's path and lower-cased headers.
struct Server {
    port: u16,
    requests: Requests,
}

impl Server {
    fn new(routes: impl Fn(&str) -> Option<Vec<u8>> + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let record = requests.clone();
        let routes = Arc::new(routes);
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let (record, routes) = (record.clone(), routes.clone());
                thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 {
                        return;
                    }
                    let path = line.split_whitespace().nth(1).unwrap_or("").to_owned();
                    let mut headers = Vec::new();
                    loop {
                        let mut header = String::new();
                        reader.read_line(&mut header).unwrap();
                        let header = header.trim_end();
                        if header.is_empty() {
                            break;
                        }
                        if let Some((name, value)) = header.split_once(':') {
                            headers.push((name.trim().to_ascii_lowercase(), value.trim().into()));
                        }
                    }
                    record.lock().unwrap().push((path.clone(), headers));
                    let response = match routes(&path) {
                        Some(body) => {
                            let kind = if path.ends_with(".m3u8") {
                                "application/vnd.apple.mpegurl"
                            } else if path.ends_with(".ts") {
                                "video/mp2t"
                            } else {
                                "video/mp4"
                            };
                            let mut response = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\n\
                                 Content-Length: {}\r\nConnection: close\r\n\r\n",
                                body.len()
                            )
                            .into_bytes();
                            response.extend(body);
                            response
                        }
                        None => b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec(),
                    };
                    let _ = stream.write_all(&response);
                    let _ = stream.flush();
                    let mut rest = Vec::new();
                    let _ = stream.read_to_end(&mut rest);
                });
            }
        });
        Self { port, requests }
    }

    /// `http://HOST:PORT/` for `host` (`127.0.0.1` or `localhost`).
    fn url(&self, host: &str, path: &str) -> Url {
        Url::parse(&format!("http://{host}:{}{path}", self.port)).unwrap()
    }

    /// The headers named `name` sent with the request for `path`.
    fn header(&self, path: &str, name: &str) -> Option<String> {
        let requests = self.requests.lock().unwrap();
        let (_, headers) = requests
            .iter()
            .find(|(requested, _)| requested == path)
            .unwrap_or_else(|| panic!("{path} was not requested: {requests:?}"));
        headers
            .iter()
            .find(|(header, _)| header == name)
            .map(|(_, value)| value.clone())
    }

    fn paths(&self) -> Vec<String> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .map(|(path, _)| path.clone())
            .collect()
    }
}

fn segment(index: usize) -> Vec<u8> {
    // MPEG-TS sync bytes so fragment validation accepts the data.
    let mut packet = vec![0x47; 188];
    packet[1] = index as u8;
    packet
}

fn playlist(segments: &[Url]) -> String {
    let mut text = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:2\n");
    for url in segments {
        text.push_str(&format!("#EXTINF:2.0,\n{url}\n"));
    }
    text.push_str("#EXT-X-ENDLIST\n");
    text
}

fn video(result: ExtractResult) -> VideoInfo {
    match result {
        ExtractResult::Video(info) => info,
        ExtractResult::Playlist(_) => panic!("expected a video"),
    }
}

fn no_temporaries(directory: &std::path::Path) {
    let names: Vec<String> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with(".yt-dlp-rs-"))
        .collect();
    assert!(names.is_empty(), "{names:?}");
}

#[test]
fn an_inline_playlist_downloads_with_headers_and_cookies_scoped_to_its_origin() {
    let server = Server::new(|path| match path {
        "/media/a0.ts" => Some(segment(0)),
        "/cdn/b1.ts" => Some(segment(1)),
        _ => None,
    });
    // Same server, two origins: 127.0.0.1 (the manifest's) and localhost.
    let base = server.url("127.0.0.1", "/media/index.m3u8");
    let text = format!(
        "#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\na0.ts\n#EXTINF:2.0,\n{}\n#EXT-X-ENDLIST\n",
        server.url("localhost", "/cdn/b1.ts")
    );
    let scope = RequestScope::new(&base)
        .unwrap()
        .header("Authorization", "Bearer secret")
        .unwrap()
        .cookie("session", "abc")
        .unwrap();
    let client = YoutubeDl::new().unwrap();
    let mut result = client
        .extract_manifest(
            &Manifest {
                id: Some("clip".into()),
                ..Manifest::inline(ManifestKind::Hls, base.clone(), text)
            },
            ExtractionOptions {
                scope: Some(scope.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    // Inline: the manifest itself is never requested.
    assert!(server.paths().is_empty(), "{:?}", server.paths());
    client.select_result(&mut result, None).unwrap();
    let info = video(result);
    assert_eq!((info.id.as_str(), info.title.as_str()), ("clip", "clip"));
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("clip.mp4");
    let control = DownloadControl {
        scope: Some(scope),
        ..Default::default()
    };
    client
        .download_with_control(
            &info,
            &destination,
            DownloadOptions::default(),
            &control,
            |_| ControlFlow::Continue(()),
        )
        .unwrap();
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        [segment(0), segment(1)].concat()
    );
    assert_eq!(
        server.header("/media/a0.ts", "authorization").as_deref(),
        Some("Bearer secret")
    );
    assert_eq!(
        server.header("/media/a0.ts", "cookie").as_deref(),
        Some("session=abc")
    );
    // Another origin gets neither the header nor the host-only cookie.
    assert_eq!(server.header("/cdn/b1.ts", "authorization"), None);
    assert_eq!(server.header("/cdn/b1.ts", "cookie"), None);
    // The scope's cookie was never stored in the jar.
    let saved = directory.path().join("cookies.txt");
    client.cookie_jar().save(&saved).unwrap();
    assert!(!std::fs::read_to_string(&saved).unwrap().contains("session"));
}

#[test]
fn a_fetched_master_playlist_requests_variants_with_the_scope() {
    let server = Server::new(|path| match path {
        "/master.m3u8" => Some(
            b"#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360\nlow/index.m3u8\n"
                .to_vec(),
        ),
        "/low/index.m3u8" => Some(
            b"#EXTM3U\n#EXT-X-TARGETDURATION:2\n#EXTINF:2.0,\ns0.ts\n#EXT-X-ENDLIST\n".to_vec(),
        ),
        _ => None,
    });
    let url = server.url("127.0.0.1", "/master.m3u8");
    let scope = RequestScope::new(&url)
        .unwrap()
        .header("X-Token", "t")
        .unwrap();
    let client = YoutubeDl::new().unwrap();
    let info = video(
        client
            .extract_manifest(
                &Manifest::new(ManifestKind::Hls, url),
                ExtractionOptions {
                    scope: Some(scope),
                    ..Default::default()
                },
            )
            .unwrap(),
    );
    assert_eq!(info.formats.len(), 1);
    assert_eq!(info.formats[0].height, Some(360));
    assert_eq!(
        server.header("/master.m3u8", "x-token").as_deref(),
        Some("t")
    );
    assert_eq!(
        server.header("/low/index.m3u8", "x-token").as_deref(),
        Some("t")
    );
}

#[test]
fn an_inline_mpd_resolves_against_its_base_url() {
    let base = Url::parse("https://media.example/dash/manifest.mpd").unwrap();
    let mpd = r#"<?xml version="1.0"?>
<MPD xmlns="urn:mpeg:dash:schema:mpd:2011" type="static" mediaPresentationDuration="PT4S">
  <Period>
    <AdaptationSet mimeType="video/mp4" codecs="avc1.64001f">
      <Representation id="v1" bandwidth="500000" width="640" height="360">
        <BaseURL>video/v1.mp4</BaseURL>
      </Representation>
    </AdaptationSet>
  </Period>
</MPD>"#;
    let client = YoutubeDl::new().unwrap();
    let info = video(
        client
            .extract_manifest(
                &Manifest::inline(ManifestKind::Dash, base, mpd),
                ExtractionOptions::default(),
            )
            .unwrap(),
    );
    assert_eq!(info.formats.len(), 1);
    assert_eq!(
        info.formats[0].url.as_str(),
        "https://media.example/dash/video/v1.mp4"
    );
}

#[test]
fn an_interrupted_extraction_makes_no_request() {
    let server = Server::new(|_| Some(b"#EXTM3U\n".to_vec()));
    let interrupt = Interrupt::new();
    interrupt.cancel();
    let client = YoutubeDl::new().unwrap();
    let error = client
        .extract_manifest(
            &Manifest::new(ManifestKind::Hls, server.url("127.0.0.1", "/index.m3u8")),
            ExtractionOptions {
                interrupt: Some(interrupt.clone()),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::ExtractionCancelled), "{error}");
    let error = client
        .extract_result_unselected_with_options(
            server.url("127.0.0.1", "/page").as_str(),
            ExtractionOptions {
                interrupt: Some(interrupt),
                ..Default::default()
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::ExtractionCancelled), "{error}");
    assert!(server.paths().is_empty(), "{:?}", server.paths());
}

/// Four fragments; the interrupt is triggered while the third downloads.
fn interrupted_fragments(stop: bool) -> (tempfile::TempDir, std::path::PathBuf, Error) {
    let server = Server::new(|path| {
        path.strip_prefix("/s")
            .and_then(|rest| rest.strip_suffix(".ts"))
            .and_then(|index| index.parse().ok())
            .map(segment)
    });
    let segments: Vec<Url> = (0..4)
        .map(|index| server.url("127.0.0.1", &format!("/s{index}.ts")))
        .collect();
    let base = server.url("127.0.0.1", "/index.m3u8");
    let client = YoutubeDl::new().unwrap();
    let mut result = client
        .extract_manifest(
            &Manifest::inline(ManifestKind::Hls, base, playlist(&segments)),
            ExtractionOptions::default(),
        )
        .unwrap();
    client.select_result(&mut result, None).unwrap();
    let info = video(result);
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("partial.mp4");
    let interrupt = Interrupt::new();
    let control = DownloadControl {
        interrupt: Some(interrupt.clone()),
        ..Default::default()
    };
    let error = client
        .download_with_control(
            &info,
            &destination,
            DownloadOptions::default(),
            &control,
            |progress| {
                if progress.fragment_index == Some(3) && progress.phase == DownloadPhase::Requesting
                {
                    if stop {
                        interrupt.stop();
                    } else {
                        interrupt.cancel();
                    }
                }
                ControlFlow::Continue(())
            },
        )
        .unwrap_err();
    (directory, destination, error)
}

#[test]
fn a_stopped_fragment_download_keeps_its_whole_fragments() {
    let (directory, destination, error) = interrupted_fragments(true);
    match error {
        Error::DownloadStopped { path } => assert_eq!(path, destination),
        error => panic!("{error}"),
    }
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        [segment(0), segment(1)].concat()
    );
    no_temporaries(directory.path());
}

#[test]
fn a_cancelled_fragment_download_leaves_nothing() {
    let (directory, destination, error) = interrupted_fragments(false);
    assert!(matches!(error, Error::DownloadCancelled), "{error}");
    assert!(!destination.exists());
    no_temporaries(directory.path());
}

#[test]
fn a_stopped_whole_file_download_keeps_the_received_bytes() {
    let body: Vec<u8> = (0..4 * 1024 * 1024)
        .map(|index| (index % 251) as u8)
        .collect();
    let expected = body.clone();
    let server = Server::new(move |path| (path == "/video.mp4").then(|| body.clone()));
    let mut info: VideoInfo = serde_json::from_value(serde_json::json!({
        "id": "v", "title": "V", "ext": "mp4", "protocol": "https",
        "url": server.url("127.0.0.1", "/video.mp4").as_str(),
        "webpage_url": server.url("127.0.0.1", "/video.mp4").as_str(),
        "extractor": "generic", "extractor_key": "Generic", "direct": true,
    }))
    .unwrap();
    info.formats.clear();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("v.mp4");
    let interrupt = Interrupt::new();
    let control = DownloadControl {
        interrupt: Some(interrupt.clone()),
        ..Default::default()
    };
    let client = YoutubeDl::new().unwrap();
    let error = client
        .download_with_control(
            &info,
            &destination,
            DownloadOptions::default(),
            &control,
            |progress| {
                if progress.phase == DownloadPhase::Downloading && progress.downloaded_bytes > 0 {
                    interrupt.stop();
                }
                ControlFlow::Continue(())
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::DownloadStopped { .. }), "{error}");
    let kept = std::fs::read(&destination).unwrap();
    assert!(
        !kept.is_empty() && kept.len() < expected.len(),
        "{}",
        kept.len()
    );
    assert_eq!(kept, expected[..kept.len()]);
    no_temporaries(directory.path());
}

/// A stop while the second stream downloads merges the first stream with
/// the second one's received bytes (a fake FFmpeg concatenates its inputs).
#[cfg(unix)]
#[test]
fn a_stopped_merged_download_merges_the_streams_so_far() {
    use std::os::unix::fs::PermissionsExt;
    let tools = tempfile::tempdir().unwrap();
    let ffmpeg = tools.path().join("ffmpeg");
    std::fs::write(
        &ffmpeg,
        "#!/bin/sh\n\
         case \"$1\" in -version) echo 'ffmpeg version control-fixture'; exit 0;; esac\n\
         prev=; set -- \"$@\"; inputs=\n\
         for a; do [ \"$prev\" = -i ] && inputs=\"$inputs ${a#file:}\"; prev=$a; out=$a; done\n\
         cat $inputs > \"${out#file:}\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&ffmpeg, std::fs::Permissions::from_mode(0o755)).unwrap();
    let audio: Vec<u8> = (0..4 * 1024 * 1024)
        .map(|index| (index % 251) as u8)
        .collect();
    let served = audio.clone();
    let server = Server::new(move |path| match path {
        "/video" => Some(b"video-bytes".to_vec()),
        "/audio" => Some(served.clone()),
        _ => None,
    });
    let info: VideoInfo = serde_json::from_value(serde_json::json!({
        "id": "m", "title": "M", "ext": "mp4",
        "url": server.url("127.0.0.1", "/video").as_str(),
        "webpage_url": server.url("127.0.0.1", "/").as_str(),
        "extractor": "fixture", "extractor_key": "Fixture", "direct": false,
        "requested_formats": [
            {"format_id": "v", "url": server.url("127.0.0.1", "/video").as_str(),
             "ext": "mp4", "stream_kind": "video_only"},
            {"format_id": "a", "url": server.url("127.0.0.1", "/audio").as_str(),
             "ext": "m4a", "stream_kind": "audio_only"},
        ],
    }))
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("m.mp4");
    let interrupt = Interrupt::new();
    let control = DownloadControl {
        interrupt: Some(interrupt.clone()),
        ..Default::default()
    };
    let error = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg(&ffmpeg)
        .download_with_control(
            &info,
            &destination,
            DownloadOptions::default(),
            &control,
            |progress| {
                if progress.stream_index == Some(2)
                    && progress.phase == DownloadPhase::Downloading
                    && progress.downloaded_bytes > 0
                {
                    interrupt.stop();
                }
                ControlFlow::Continue(())
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::DownloadStopped { .. }), "{error}");
    let merged = std::fs::read(&destination).unwrap();
    let partial = &merged[b"video-bytes".len()..];
    assert!(merged.starts_with(b"video-bytes"));
    assert!(
        !partial.is_empty() && partial.len() < audio.len(),
        "{}",
        partial.len()
    );
    assert_eq!(partial, &audio[..partial.len()]);
    let names: Vec<String> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["m.mp4"]);
}

/// `Interrupt::stop` with FFmpegFD asks FFmpeg to finish (`q`), so the
/// published file is a playable prefix. Real FFmpeg; opt in with
/// `YT_DLP_RS_TEST_FFMPEG`.
#[test]
fn a_stopped_ffmpeg_download_is_finished_and_kept() {
    let Some(ffmpeg) = std::env::var_os("YT_DLP_RS_TEST_FFMPEG") else {
        return;
    };
    let ts = include_bytes!("fixtures/hls/audio-video/video/segment-0.ts").to_vec();
    // FFmpeg reads the manifest itself, so it is served (not inline).
    let text = Arc::new(Mutex::new(String::new()));
    let served = text.clone();
    // Segments after the first arrive slowly, so the stop lands mid-stream.
    let server = Server::new(move |path| {
        if path == "/index.m3u8" {
            return Some(served.lock().unwrap().clone().into_bytes());
        }
        let index: usize = path.strip_prefix("/s")?.strip_suffix(".ts")?.parse().ok()?;
        if index > 0 {
            thread::sleep(std::time::Duration::from_millis(400));
        }
        Some(ts.clone())
    });
    let segments: Vec<Url> = (0..20)
        .map(|index| server.url("127.0.0.1", &format!("/s{index}.ts")))
        .collect();
    let client = YoutubeDl::new()
        .unwrap()
        .with_ffmpeg(&ffmpeg)
        .with_downloader("m3u8", true);
    *text.lock().unwrap() = playlist(&segments);
    let mut result = client
        .extract_manifest(
            &Manifest::new(ManifestKind::Hls, server.url("127.0.0.1", "/index.m3u8")),
            ExtractionOptions::default(),
        )
        .unwrap();
    client.select_result(&mut result, None).unwrap();
    let info = video(result);
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("f.mp4");
    let interrupt = Interrupt::new();
    let control = DownloadControl {
        interrupt: Some(interrupt.clone()),
        ..Default::default()
    };
    let started = std::time::Instant::now();
    let error = client
        .download_with_control(
            &info,
            &destination,
            DownloadOptions::default(),
            &control,
            |_| {
                if started.elapsed() > std::time::Duration::from_millis(1500) {
                    interrupt.stop();
                }
                ControlFlow::Continue(())
            },
        )
        .unwrap_err();
    assert!(matches!(error, Error::DownloadStopped { .. }), "{error}");
    let size = std::fs::metadata(&destination).unwrap().len();
    assert!(size > 0);
    // Not all 20 segments (8 s of delays) were fetched.
    let segments = server
        .paths()
        .iter()
        .filter(|path| path.ends_with(".ts"))
        .count();
    assert!(segments < 20, "{:?}", server.paths());
}
