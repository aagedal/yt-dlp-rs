//! Saved-shape YouTube caption discovery and sidecar transport on loopback only.
use super::*;
use crate::{SubtitleOptions, Subtitles};

const CAPTION: &str = "WEBVTT\n\n00:00.000 --> 00:01.000\nSaved caption é\n";
const EXTENSIONS: [&str; 7] = ["json3", "srv1", "srv2", "srv3", "ttml", "srt", "vtt"];

fn track(url: &str, vss: &str, kind: Option<&str>, name: &str) -> Value {
    let mut value =
        json!({"baseUrl":url,"vssId":vss,"name":{"simpleText":name},"isTranslatable":true});
    if let Some(kind) = kind {
        value["kind"] = kind.into();
    }
    value
}
fn captions(tracks: Vec<Value>, translations: Vec<Value>) -> Value {
    let mut value = response(vec![format(18, "{{ORIGIN}}/media.mp4")]);
    value["captions"] = json!({"playerCaptionsTracklistRenderer":{"captionTracks":tracks,"translationLanguages":translations}});
    value
}
fn translations() -> Vec<Value> {
    vec![
        json!({"languageCode":"en","languageName":{"simpleText":"English"}}),
        json!({"languageCode":"fr","languageName":{"simpleText":"French"}}),
    ]
}
fn formats<'a>(subtitles: &'a Subtitles, language: &str) -> &'a Vec<crate::SubtitleFormat> {
    &subtitles
        .entries
        .iter()
        .find(|(key, _)| key.as_deref() == Some(language))
        .unwrap_or_else(|| panic!("missing language {language}: {subtitles:?}"))
        .1
}
fn watch_server(value: Value) -> Server {
    Server::new(vec![
        reply(WATCH, "text/html", page(Some(value))),
        reply("/media.mp4", "video/mp4", "media"),
    ])
}
fn query(url: &Url, key: &str) -> Option<String> {
    url.query_pairs()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.into_owned())
}

#[test]
fn watch_manual_caption_metadata_is_lazy_and_keeps_formats_names_and_caption_n_without_runtime() {
    let value = captions(
        vec![track(
            "{{ORIGIN}}/caption?lang=en&xosf=1&n=caption-only&keep=%2f",
            ".en",
            None,
            "English",
        )],
        vec![],
    );
    let server = watch_server(value);
    let runtime = challenge::Runtime::new("/nonexistent-caption-test-deno".into());
    let info = extract(&server, Some(&runtime)).unwrap();
    let subs = formats(&info.subtitles, "en");
    assert_eq!(
        subs.iter().map(|s| s.ext.as_str()).collect::<Vec<_>>(),
        EXTENSIONS
    );
    for subtitle in subs {
        assert_eq!(subtitle.name.as_deref(), Some("English"));
        assert_eq!(query(&subtitle.url, "fmt"), Some(subtitle.ext.clone()));
        assert_eq!(query(&subtitle.url, "n").as_deref(), Some("caption-only"));
        assert!(query(&subtitle.url, "xosf").is_none());
        assert_eq!(query(&subtitle.url, "keep").as_deref(), Some("/"));
    }
    assert!(info.automatic_captions.is_empty());
    assert!(info.requested_subtitles.is_none());
}

#[test]
fn asr_original_aliases_translations_and_names_set_audio_language_without_touching_video_only() {
    let mut value = captions(
        vec![track(
            "{{ORIGIN}}/caption?lang=en",
            "a.en",
            Some("asr"),
            "English (auto-generated)",
        )],
        translations(),
    );
    value["streamingData"]["adaptiveFormats"] = json!([
        {"itag":137,"url":"{{ORIGIN}}/video.mp4","mimeType":"video/mp4; codecs=\"avc1.640028\""},
        {"itag":140,"url":"{{ORIGIN}}/audio.m4a","mimeType":"audio/mp4; codecs=\"mp4a.40.2\""}
    ]);
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(Some(value))),
        reply("/media.mp4", "video/mp4", "muxed"),
        reply("/video.mp4", "video/mp4", "picture"),
        reply("/audio.m4a", "audio/mp4", "sound"),
    ]);
    let info = extract(&server, None).unwrap();
    assert!(info.subtitles.is_empty());
    assert_eq!(
        formats(&info.automatic_captions, "en-orig")[0]
            .name
            .as_deref(),
        Some("English (Original)")
    );
    assert_eq!(
        formats(&info.automatic_captions, "en")[0].name.as_deref(),
        Some("English")
    );
    let french = &formats(&info.automatic_captions, "fr")[0];
    assert_eq!(french.name.as_deref(), Some("French"));
    assert_eq!(query(&french.url, "tlang").as_deref(), Some("fr"));
    assert!(query(&formats(&info.automatic_captions, "en")[0].url, "tlang").is_none());
    assert_eq!(info.formats[0].language.as_deref(), Some("en"));
    assert_eq!(info.formats[1].language, None);
    assert_eq!(info.formats[2].language.as_deref(), Some("en"));
}

#[test]
fn asr_language_missing_from_translations_retains_fallback_original_track_and_name() {
    let mut value = track(
        "{{ORIGIN}}/caption?lang=yue",
        "a.yue",
        Some("asr"),
        "Cantonese (auto-generated)",
    );
    value["isTranslatable"] = false.into();
    let server = watch_server(captions(vec![value], vec![]));
    let info = extract(&server, None).unwrap();
    assert_eq!(info.automatic_captions.entries.len(), 1);
    assert_eq!(
        formats(&info.automatic_captions, "yue")[0].name.as_deref(),
        Some("Cantonese")
    );
    assert_eq!(info.formats[0].language, None);
}

#[test]
fn translated_manual_captions_require_the_explicit_extraction_mode() {
    let value = captions(
        vec![track("{{ORIGIN}}/caption?lang=en", ".en", None, "English")],
        vec![json!({"languageCode":"fr","languageName":{"simpleText":"French"}})],
    );
    let normal = watch_server(value.clone());
    assert!(
        extract(&normal, None)
            .unwrap()
            .automatic_captions
            .is_empty()
    );
    let translated = watch_server(value);
    let info = extract_watch_page_with_caption_options(
        &http_client(),
        &translated.watch(),
        ID,
        None,
        Some(&translated.source()),
        true,
    )
    .unwrap();
    assert_eq!(formats(&info.subtitles, "en").len(), 7);
    let french = &formats(&info.automatic_captions, "fr-en")[0];
    assert_eq!(french.name.as_deref(), Some("French from English"));
    assert_eq!(query(&french.url, "tlang").as_deref(), Some("fr-en"));
}

#[test]
fn subtitle_source_preference_and_actual_sidecar_preserve_raw_caption_n_and_canonical_page_referrer()
 {
    let value = captions(
        vec![
            track(
                "{{ORIGIN}}/media-caption?lang=en&n=caption-only",
                ".en",
                None,
                "Manual English",
            ),
            track(
                "{{ORIGIN}}/auto?lang=en",
                "a.en",
                Some("asr"),
                "English (auto-generated)",
            ),
        ],
        translations(),
    );
    let mut caption_reply = reply(
        "/media-caption?lang=en&n=caption-only&fmt=vtt",
        "text/vtt",
        CAPTION,
    );
    caption_reply.canonical_referrer = true;
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(Some(value))),
        reply("/media.mp4", "video/mp4", "media"),
        caption_reply,
    ]);
    let info = extract(
        &server,
        Some(&challenge::Runtime::new("/nonexistent-caption-deno".into())),
    )
    .unwrap();
    let selection = SubtitleOptions::new(Some("en"), "vtt")
        .unwrap()
        .select_sources(&info.subtitles, &info.automatic_captions, true, true)
        .unwrap();
    let selected = &selection.requested.entries[0].1;
    assert_eq!(selected.name.as_deref(), Some("Manual English"));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("caption.en.vtt");
    let mut publication_seen = false;
    let bytes = crate::YoutubeDl::new()
        .unwrap()
        .download_subtitle_with_progress(
            &info,
            selected,
            &path,
            crate::DownloadOptions {
                retries: 0,
                ..Default::default()
            },
            |progress| {
                publication_seen |= progress.phase == crate::DownloadPhase::Publishing;
                std::ops::ControlFlow::Continue(())
            },
        )
        .unwrap();
    assert!(publication_seen);
    assert_eq!(bytes, CAPTION.len() as u64);
    assert_eq!(std::fs::read(path).unwrap(), CAPTION.as_bytes());
}

#[test]
fn normal_english_prefix_precedes_automatic_exact_english_and_auto_only_can_be_selected() {
    let value = captions(
        vec![
            track(
                "{{ORIGIN}}/manual?lang=en-GB",
                ".en.GB",
                None,
                "British English",
            ),
            track(
                "{{ORIGIN}}/auto?lang=en",
                "a.en",
                Some("asr"),
                "English (auto-generated)",
            ),
        ],
        translations(),
    );
    let server = watch_server(value);
    let info = extract(&server, None).unwrap();
    let selected = SubtitleOptions::default()
        .select_sources(&info.subtitles, &info.automatic_captions, true, true)
        .unwrap();
    assert_eq!(selected.requested.entries[0].0.as_deref(), Some("en-GB"));
    let selected = SubtitleOptions::new(Some("en"), "srt")
        .unwrap()
        .select_sources(&info.subtitles, &info.automatic_captions, false, true)
        .unwrap();
    assert_eq!(selected.requested.entries[0].1.ext, "srt");
    assert!(selected.requested.entries[0].1.url.path().contains("auto"));
}

#[test]
fn caption_token_experiment_stops_remaining_tracks_without_requests_or_guessing_success() {
    for experiment in ["xpe", "xpv"] {
        let value = captions(
            vec![
                track("{{ORIGIN}}/first?lang=en", ".en", None, "English"),
                track(
                    &format!("{{{{ORIGIN}}}}/blocked?lang=nb&exp={experiment}"),
                    ".nb",
                    None,
                    "Norwegian",
                ),
                track("blob:https://example.invalid/never", ".fr", None, "French"),
            ],
            vec![],
        );
        let server = watch_server(value);
        let info = extract(&server, None).unwrap();
        assert_eq!(info.subtitles.entries.len(), 1);
        assert_eq!(formats(&info.subtitles, "en").len(), 7);
        assert!(info.automatic_captions.is_empty());
    }
}

#[test]
fn unsupported_caption_urls_fail_before_media_probes_while_empty_or_missing_language_tracks_are_skipped()
 {
    let invalid = captions(
        vec![track(
            "ftp://example.invalid/caption",
            ".en",
            None,
            "English",
        )],
        vec![],
    );
    let server = Server::new(vec![reply(WATCH, "text/html", page(Some(invalid)))]);
    assert!(matches!(
        extract(&server, None),
        Err(Error::UnsupportedYoutube(_))
    ));
    for tracks in [
        vec![],
        vec![
            json!({"baseUrl":"{{ORIGIN}}/missing-language?lang=en","name":{"simpleText":"Unnamed"}}),
        ],
        vec![json!({"baseUrl":"","vssId":".en"})],
    ] {
        let server = watch_server(captions(tracks, vec![]));
        let info = extract(&server, None).unwrap();
        assert!(info.subtitles.is_empty());
        assert!(info.automatic_captions.is_empty());
    }
}

#[test]
fn web_api_caption_metadata_uses_the_same_policy_without_unprotecting_direct_formats_or_requesting_js()
 {
    let mut value = captions(
        vec![track(
            "{{ORIGIN}}/caption?lang=en&n=caption-only",
            ".en",
            None,
            "English",
        )],
        vec![],
    );
    value["streamingData"]["formats"]
        .as_array_mut()
        .unwrap()
        .push(format(22, "{{ORIGIN}}/protected.mp4?n=protected"));
    let server = Server::new(vec![
        reply(WATCH, "text/html", page(None)),
        reply(
            "/youtubei/v1/player?prettyPrint=false",
            "application/json",
            value.to_string(),
        ),
        reply("/media.mp4", "video/mp4", "media"),
    ]);
    let info = extract(
        &server,
        Some(&challenge::Runtime::new(
            "/nonexistent-api-caption-deno".into(),
        )),
    )
    .unwrap();
    assert_eq!(info.formats.len(), 1);
    assert_eq!(info.formats[0].format_id, "18");
    assert_eq!(formats(&info.subtitles, "en").len(), 7);
}

#[test]
fn caption_discovery_does_not_bypass_playability_or_non_web_api_client_gates() {
    let mut value = captions(
        vec![track(
            "ftp://example.invalid/caption",
            ".en",
            None,
            "English",
        )],
        vec![],
    );
    value["playabilityStatus"]["status"] = "LOGIN_REQUIRED".into();
    let server = Server::new(vec![reply(WATCH, "text/html", page(Some(value)))]);
    let error = extract(&server, None).unwrap_err();
    assert!(error.to_string().contains("not playable"));
    let html = page(None).replace("\"clientName\":\"WEB\"", "\"clientName\":\"ANDROID\"");
    let server = Server::new(vec![reply(WATCH, "text/html", html)]);
    assert!(extract(&server, None).is_err());
}
