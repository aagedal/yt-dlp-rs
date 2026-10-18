//! YouTube manifest discovery remains extraction-only until fragment download.
use super::*;

const SIMPLE: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:1\n#EXTINF:1,\nfirst.ts\n#EXTINF:1,\nsecond.ts\n#EXT-X-ENDLIST\n";
const MASTER: &str = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=100000,RESOLUTION=640x360,CODECS=\"avc1.4,mp4a.40.2\"\nlow.m3u8\n#EXT-X-STREAM-INF:BANDWIDTH=300000,RESOLUTION=1280x720,CODECS=\"avc1.4,mp4a.40.2\"\nhigh.m3u8\n";

fn hls_response(formats: Vec<Value>, url: &str) -> Value {
    let mut player = response(formats);
    player["streamingData"]["hlsManifestUrl"] = url.into();
    player
}
fn manifest(path: &str, text: &str) -> Reply {
    reply(path, "application/vnd.apple.mpegurl", text)
}

#[test]
fn hls_only_watch_extraction_selects_and_downloads_ordered_fragments() {
    let server = Server::new(vec![
        reply(
            WATCH,
            "text/html",
            page(Some(hls_response(Vec::new(), "{{ORIGIN}}/hls/index.m3u8"))),
        ),
        manifest("/hls/index.m3u8", SIMPLE),
        reply("/hls/first.ts", "video/mp2t", "first"),
        reply("/hls/second.ts", "video/mp2t", "second"),
    ]);
    let mut info = extract(&server, None).unwrap();
    assert_eq!(info.id, ID);
    assert_eq!(info.title, "Challenge fixture");
    assert_eq!(info.extractor, "youtube");
    assert_eq!(
        info.webpage_url.as_str(),
        format!("https://www.youtube.com{WATCH}")
    );
    assert_eq!(info.formats.len(), 1);
    assert_eq!(info.formats[0].format_id, "hls-0");
    assert_eq!(info.fragments, info.formats[0].fragments);
    assert_eq!(info.fragments.len(), 2);
    assert_eq!(info.ext, "ts");
    info.select_format(None).unwrap();
    assert_eq!(info.format_id.as_deref(), Some("hls-0"));
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("youtube.ts");
    assert_eq!(
        crate::YoutubeDl::new()
            .unwrap()
            .download(&info, &destination)
            .unwrap(),
        11
    );
    assert_eq!(std::fs::read(destination).unwrap(), b"firstsecond");
}

#[test]
fn mixed_direct_and_master_formats_probe_only_direct_sources_and_rank_dimensions() {
    let mut direct = format(18, "{{ORIGIN}}/direct.mp4");
    direct["width"] = 640.into();
    direct["height"] = 360.into();
    let server = Server::new(vec![
        reply(
            WATCH,
            "text/html",
            page(Some(hls_response(
                vec![direct],
                "{{ORIGIN}}/hls/master.m3u8",
            ))),
        ),
        reply("/direct.mp4", "video/mp4", "muxed"),
        manifest("/hls/master.m3u8", MASTER),
        manifest("/hls/low.m3u8", SIMPLE),
        manifest("/hls/high.m3u8", SIMPLE),
    ]);
    let mut info = extract(&server, None).unwrap();
    assert_eq!(
        info.formats
            .iter()
            .map(|f| f.format_id.as_str())
            .collect::<Vec<_>>(),
        ["18", "hls-0", "hls-1"]
    );
    assert!(info.formats[0].fragments.is_empty());
    assert!(info.fragments.is_empty());
    assert_eq!(info.filesize, Some(5));
    info.select_format(None).unwrap();
    assert_eq!(info.format_id.as_deref(), Some("hls-1"));
    assert_eq!(info.height, Some(720));
    assert_eq!(info.filesize, None);
    assert_eq!(info.fragments.len(), 2);
    info.select_format(Some("18")).unwrap();
    assert!(info.fragments.is_empty());
    assert_eq!(info.filesize, Some(5));
}

#[test]
fn external_hls_audio_groups_are_namespaced_and_pair_selections_preserve_plans() {
    let master = "#EXTM3U\n#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID=\"main\",NAME=\"English\",LANGUAGE=\"en\",DEFAULT=YES,AUTOSELECT=YES,URI=\"audio.m3u8\"\n#EXT-X-STREAM-INF:BANDWIDTH=300000,RESOLUTION=1280x720,CODECS=\"avc1.4,mp4a.40.2\",AUDIO=\"main\"\nvideo.m3u8\n";
    let audio = "#EXTM3U\n#EXTINF:1,\naudio.aac\n#EXT-X-ENDLIST\n";
    let server = Server::new(vec![
        reply(
            WATCH,
            "text/html",
            page(Some(hls_response(Vec::new(), "{{ORIGIN}}/hls/master.m3u8"))),
        ),
        manifest("/hls/master.m3u8", master),
        manifest("/hls/video.m3u8", SIMPLE),
        manifest("/hls/audio.m3u8", audio),
    ]);
    let mut info = extract(&server, None).unwrap();
    assert_eq!(info.formats.len(), 2);
    assert_eq!(info.formats[0].stream_kind, StreamKind::VideoOnly);
    assert_eq!(info.formats[1].stream_kind, StreamKind::AudioOnly);
    assert_eq!(
        info.formats[0].audio_group.as_deref(),
        Some("youtube-hls:main")
    );
    assert_eq!(info.formats[0].audio_group, info.formats[1].audio_group);
    assert_eq!(info.formats[1].language.as_deref(), Some("en"));
    assert!(info.select_format(None).is_err());
    info.select_format(Some("bestvideo+bestaudio")).unwrap();
    assert_eq!(info.format_id.as_deref(), Some("hls-0+hls-audio-0"));
    assert_eq!(info.ext, "mp4");
    assert!(info.fragments.is_empty());
    assert_eq!(info.requested_formats[0].fragments.len(), 2);
    assert_eq!(info.requested_formats[1].fragments.len(), 1);
}

#[test]
fn web_api_hls_is_token_optional_while_direct_itag_policy_and_dash_exclusion_remain() {
    for include_18 in [false, true] {
        let mut formats = vec![format(22, "{{ORIGIN}}/forbidden.mp4?n=protected")];
        if include_18 {
            formats.insert(0, format(18, "{{ORIGIN}}/direct.mp4"));
        }
        let mut player = hls_response(formats, "{{ORIGIN}}/hls/index.m3u8");
        player["streamingData"]["dashManifestUrl"] = "{{ORIGIN}}/forbidden.mpd?n=protected".into();
        let mut replies = vec![
            reply(WATCH, "text/html", page(None)),
            reply(
                "/youtubei/v1/player?prettyPrint=false",
                "application/json",
                player.to_string(),
            ),
        ];
        if include_18 {
            replies.push(reply("/direct.mp4", "video/mp4", "muxed"));
        }
        replies.push(manifest("/hls/index.m3u8", SIMPLE));
        let fake = Fake::new("solver");
        let runtime = challenge::Runtime::new(fake.path.clone());
        let server = Server::new(replies);
        let info = extract(&server, Some(&runtime)).unwrap();
        assert_eq!(info.formats.len(), if include_18 { 2 } else { 1 });
        assert!(info.formats.iter().all(|format| format.format_id != "22"));
        assert!(
            info.formats
                .iter()
                .any(|format| format.format_id == "hls-0")
        );
        assert_eq!(fake.log(), "");
    }
}

#[test]
fn direct_signature_query_n_and_manifest_path_n_share_one_complete_solve_batch() {
    let fake = Fake::new("solver");
    let runtime = challenge::Runtime::new(fake.path.clone());
    let player = hls_response(
        vec![
            format(18, "{{ORIGIN}}/direct.mp4?n=challenge"),
            cipher_format(22, "{{ORIGIN}}/signature.mp4", None, false),
        ],
        "{{ORIGIN}}/n/challenge/hls/index.m3u8?keep=%2f",
    );
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(Some(player))),
        reply(SCRIPT, "text/javascript", "const fake=true;"),
        reply("/direct.mp4?n=solved", "video/mp4", "muxed"),
        reply(
            "/signature.mp4?signature=signature-value",
            "video/mp4",
            "muxed",
        ),
        manifest("/n/solved/hls/index.m3u8?keep=%2f", SIMPLE),
    ]);
    let info = extract(&server, Some(&runtime)).unwrap();
    assert_eq!(info.formats.len(), 3);
    assert_eq!(fake.log(), "version\nsolve\n");
    assert!(
        info.formats[2]
            .url
            .as_str()
            .ends_with("/n/solved/hls/index.m3u8?keep=%2f")
    );
    assert!(
        info.formats[2].fragments[0]
            .url
            .as_str()
            .ends_with("/n/solved/hls/first.ts")
    );
}

#[test]
fn challenged_hls_without_runtime_is_skipped_and_missing_player_config_does_not_probe() {
    for manifest_url in [
        "{{ORIGIN}}/hls/index.m3u8?n=challenge",
        "{{ORIGIN}}/n/challenge/hls/index.m3u8",
    ] {
        let server = Server::new(vec![reply(
            WATCH,
            "text/html",
            page(Some(hls_response(Vec::new(), manifest_url))),
        )]);
        assert!(matches!(
            extract(&server, None),
            Err(Error::UnsupportedYoutube(_))
        ));
    }
    let player = hls_response(Vec::new(), "{{ORIGIN}}/n/challenge/hls/index.m3u8");
    let html = format!("<script>ytInitialPlayerResponse = {player};</script>");
    let fake = Fake::new("solver");
    let runtime = challenge::Runtime::new(fake.path.clone());
    let server = Server::new(vec![reply(WATCH, "text/html", html)]);
    assert!(matches!(
        extract(&server, Some(&runtime)),
        Err(Error::UnsupportedYoutube(_))
    ));
    assert_eq!(fake.log(), "");
}

#[test]
fn eligibility_gates_cover_hls_before_manifest_runtime_and_media_requests() {
    for gate in ["login", "identity", "title", "live", "drm"] {
        let mut player = hls_response(Vec::new(), "{{ORIGIN}}/n/challenge/hls/index.m3u8");
        match gate {
            "login" => player["playabilityStatus"]["status"] = "LOGIN_REQUIRED".into(),
            "identity" => player["videoDetails"]["videoId"] = "wrong_video".into(),
            "title" => player["videoDetails"]["title"] = "".into(),
            "live" => player["videoDetails"]["isLiveContent"] = true.into(),
            "drm" => player["streamingData"]["licenseInfos"] = json!([{"drmFamily":"WIDEVINE"}]),
            _ => unreachable!(),
        }
        let fake = Fake::new("solver");
        let runtime = challenge::Runtime::new(fake.path.clone());
        let server = Server::new(vec![reply(WATCH, "text/html", page(Some(player)))]);
        assert!(
            matches!(
                extract(&server, Some(&runtime)),
                Err(Error::UnsupportedYoutube(_))
            ),
            "{gate}"
        );
        assert_eq!(fake.log(), "");
    }
}

#[test]
fn eligible_invalid_manifest_fails_the_entire_extraction_instead_of_using_direct_alternative() {
    let server = Server::new(vec![
        reply(
            WATCH,
            "text/html",
            page(Some(hls_response(
                vec![format(18, "{{ORIGIN}}/direct.mp4")],
                "{{ORIGIN}}/hls/index.m3u8",
            ))),
        ),
        reply("/direct.mp4", "video/mp4", "muxed"),
        manifest("/hls/index.m3u8", "#EXTM3U\n#EXTINF:1,\nfirst.ts\n"),
    ]);
    assert!(matches!(
        extract(&server, None),
        Err(Error::UnsupportedManifest(_))
    ));
}

#[test]
fn extraction_saves_init_and_key_plans_without_fetching_those_resources() {
    let media = "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"key.bin\",IV=0x1\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:1,\npart.m4s\n#EXT-X-ENDLIST\n";
    let server = Server::new(vec![
        reply(
            WATCH,
            "text/html",
            page(Some(hls_response(Vec::new(), "{{ORIGIN}}/hls/index.m3u8"))),
        ),
        manifest("/hls/index.m3u8", media),
    ]);
    let info = extract(&server, None).unwrap();
    assert_eq!(info.ext, "mp4");
    assert_eq!(info.fragments.len(), 2);
    assert!(info.fragments[0].url.as_str().ends_with("/hls/init.mp4"));
    assert!(info.fragments.iter().all(|fragment| {
        fragment
            .encryption
            .as_ref()
            .unwrap()
            .key_url
            .as_str()
            .ends_with("/hls/key.bin")
    }));
}

#[test]
fn dash_only_urls_are_never_fetched_or_solved_for_initial_or_web_responses() {
    for api in [false, true] {
        let mut player = response(Vec::new());
        player["streamingData"]["dashManifestUrl"] = "{{ORIGIN}}/forbidden.mpd?n=challenge".into();
        let replies = if api {
            vec![
                reply(WATCH, "text/html", page(None)),
                reply(
                    "/youtubei/v1/player?prettyPrint=false",
                    "application/json",
                    player.to_string(),
                ),
            ]
        } else {
            vec![reply(WATCH, "text/html", page(Some(player)))]
        };
        let fake = Fake::new("solver");
        let runtime = challenge::Runtime::new(fake.path.clone());
        let server = Server::new(replies);
        assert!(matches!(
            extract(&server, Some(&runtime)),
            Err(Error::UnsupportedYoutube(_))
        ));
        assert_eq!(fake.log(), "");
    }
}

#[test]
fn invalid_runtime_prevents_all_probes_but_no_runtime_can_use_direct_alternative() {
    let player = hls_response(
        vec![format(18, "{{ORIGIN}}/direct.mp4")],
        "{{ORIGIN}}/n/challenge/hls/index.m3u8",
    );
    let directory = tempfile::tempdir().unwrap();
    let runtime = challenge::Runtime::new(directory.path().join("missing-runtime"));
    let server = Server::new(vec![reply(WATCH, "text/html", page(Some(player.clone())))]);
    assert!(matches!(
        extract(&server, Some(&runtime)),
        Err(Error::UnsupportedYoutube(_))
    ));
    drop(server);
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(Some(player))),
        reply("/direct.mp4", "video/mp4", "muxed"),
    ]);
    let info = extract(&server, None).unwrap();
    assert_eq!(info.formats.len(), 1);
    assert_eq!(info.formats[0].format_id, "18");
}

#[test]
fn saved_hls_player_fixture_matches_the_upstream_comparison_metadata_allowlist() {
    let player: Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/youtube/hls/player-response.json"
    ))
    .unwrap();
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(Some(player))),
        manifest(
            "/hls/master.m3u8",
            include_str!("../../../../tests/fixtures/youtube/hls/master.m3u8"),
        ),
        manifest(
            "/hls/itag/95/file/index.m3u8",
            include_str!("../../../../tests/fixtures/youtube/hls/media.m3u8"),
        ),
        manifest(
            "/hls/itag/96/file/index.m3u8",
            include_str!("../../../../tests/fixtures/youtube/hls/media.m3u8"),
        ),
    ]);
    let mut info = extract(&server, None).unwrap();
    let expected: Value = serde_json::from_str(
        &include_str!("../../../../tests/fixtures/youtube/hls/expected.json")
            .replace("{{ORIGIN}}", server.origin.as_str().trim_end_matches('/')),
    )
    .unwrap();
    let actual = serde_json::to_value(&info).unwrap();
    for key in ["id", "title", "extractor", "extractor_key", "webpage_url"] {
        assert_eq!(actual[key], expected[key], "{key}");
    }
    let expected_formats = expected["formats"].as_array().unwrap();
    assert_eq!(info.formats.len(), expected_formats.len());
    for (format, expected) in info.formats.iter().zip(expected_formats) {
        assert_eq!(format.format_id, expected["format_id"]);
        assert_eq!(format.url.as_str(), expected["url"]);
        assert_eq!(format.width.map(u64::from), expected["width"].as_u64());
        assert_eq!(format.height.map(u64::from), expected["height"].as_u64());
        assert_eq!(format.tbr, expected["tbr"].as_f64());
        assert_eq!(format.language.as_deref(), expected["language"].as_str());
        assert_eq!(
            serde_json::to_value(format.stream_kind).unwrap(),
            expected["stream_kind"]
        );
        assert_eq!(format.fragments.len(), 2);
    }
    info.select_format(None).unwrap();
    assert_eq!(info.format_id.as_deref(), Some("hls-1"));
    assert_eq!(info.height, Some(720));
}

#[test]
fn unsupported_manifest_query_n_never_reaches_solver_or_manifest_request() {
    let fake = Fake::new("solver");
    let runtime = challenge::Runtime::new(fake.path.clone());
    let player = hls_response(
        vec![format(18, "{{ORIGIN}}/direct.mp4")],
        "{{ORIGIN}}/hls/index.m3u8?n=challenge",
    );
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(Some(player))),
        reply("/direct.mp4", "video/mp4", "muxed"),
    ]);
    let info = extract(&server, Some(&runtime)).unwrap();
    assert_eq!(info.formats.len(), 1);
    assert_eq!(info.formats[0].format_id, "18");
    assert_eq!(fake.log(), "");
}
