//! Entry points for the `cargo fuzz` targets in `fuzz/`. Compiled only under
//! `--cfg fuzzing` (set by cargo-fuzz), so the parsers stay crate-private in
//! normal builds. Each function feeds untrusted bytes to one parser and
//! discards the result: only panics, hangs and runaway allocations matter.

use url::Url;

fn base() -> Url {
    Url::parse("https://example.com/path/manifest").expect("static URL")
}

fn text(data: &[u8]) -> &str {
    std::str::from_utf8(data).unwrap_or_default()
}

/// HLS playlists in every parse mode, and the DRM check.
pub fn hls(data: &[u8]) {
    let text = text(data);
    let base = base();
    let _ = crate::hls::has_drm(text);
    let _ = crate::hls::parse(text, &base);
    let _ = crate::hls::parse_window(text, &base);
    let _ = crate::hls::parse_live(text, &base);
}

/// DASH MPDs.
pub fn dash(data: &[u8]) {
    let _ = crate::dash::parse_manifest(text(data), &base());
}

/// MP4 `sidx` / WebM `Cues` indexes. The first 16 bytes choose the init and
/// index ranges; the rest is the file head starting at offset 0.
pub fn media_index(data: &[u8]) {
    let Some((header, head)) = data.split_first_chunk::<16>() else {
        return;
    };
    let word = |at: usize| u64::from(u32::from_be_bytes(header[at..at + 4].try_into().unwrap()));
    let ranges = crate::info::ByteIndex {
        init: (0, word(0)),
        index: (word(4), word(8)),
    };
    let total = (header[12] & 1 == 1).then(|| word(12) >> 1);
    let _ = crate::media_index::parse(head, 0, ranges, total);
}

/// `js_to_json` and the JS value parser built on it.
pub fn js_to_json(data: &[u8]) {
    let text = text(data);
    let _ = crate::js_to_json::js_to_json(text);
    let _ = crate::js_to_json::js_to_json_strict(text);
    let _ = crate::js_to_json::parse_js_value(text);
}

/// SvelteKit `devalue` payloads.
pub fn devalue(data: &[u8]) {
    if let Ok(value) = serde_json::from_slice(data) {
        let _ = crate::devalue::parse_iter(&value, &[]);
    }
}

/// TTML/DFXP to SRT conversion.
pub fn ttml(data: &[u8]) {
    let _ = crate::subtitle_convert::ttml::to_srt(data);
}

/// Netscape cookie files.
pub fn cookie_file(data: &[u8]) {
    let _ = crate::cookies::CookieJar::parse_for_fuzzing(text(data));
}

/// Browser cookie databases: every table the cookie readers query.
pub fn sqlite(data: &[u8]) {
    let Ok(db) = crate::sqlite::Database::from_bytes(data.to_vec()) else {
        return;
    };
    let _ = db.user_version();
    for name in ["cookies", "moz_cookies", "meta"] {
        let _ = db.table(name);
    }
}

/// HDS bootstrap boxes and `mdat` extraction.
pub fn f4m(data: &[u8]) {
    crate::f4m_fd::fuzz_bootstrap(data);
    let _ = crate::f4m_fd::mdat(data);
}

/// Smooth Streaming (ISM) fragment boxes.
pub fn ism(data: &[u8]) {
    let _ = crate::ism_fd::track_id(data);
}

/// `--load-info-json` input.
pub fn info_json(data: &[u8]) {
    let _ = crate::info_json::load_info_json_text(text(data));
}

/// `.netrc` files.
pub fn netrc(data: &[u8]) {
    let _ = crate::netrc::Netrc::from_content(text(data));
}

/// Webpage helpers every extractor runs on fetched HTML.
pub fn html(data: &[u8]) {
    use crate::html_elements as elements;
    let html = text(data);
    let _ = crate::html::unescape_html(html);
    let _ = crate::html::clean_html(html);
    let _ = crate::html::og_search_property(html, "title");
    let _ = crate::html::html_search_meta(html, "description");
    let _ = crate::html::html_extract_title(html);
    let _ = crate::html::rta_search(html);
    let _ = elements::element_text_and_html_by_tag("video", html);
    let _ = elements::elements_text_and_html_by_attribute("data-id", "x", html);
    let _ = elements::elements_text_and_html_by_class("player", html);
    let _ = elements::element_html_by_id("main", html);
    let _ = elements::extract_attributes(html);
    let _ = crate::extractor::yield_json_ld(html);
}

/// Date strings from pages and APIs.
pub fn dates(data: &[u8]) {
    let date = text(data);
    let _ = crate::dates::unified_timestamp(date, false);
    let _ = crate::dates::unified_timestamp(date, true);
    let _ = crate::dates::unified_strdate(date, false);
    let _ = crate::dates::parse_iso8601(date);
    let _ = crate::dates::parse_iso8601_space(date);
    let _ = crate::dates::timeconvert(date);
}

/// HLS WebVTT segments joined like HlsFD: NUL bytes separate the
/// fragments.
pub fn webvtt(data: &[u8]) {
    let mut packer = crate::webvtt::Packer::default();
    for (index, fragment) in data.split(|&byte| byte == 0).enumerate() {
        let _ = packer.pack(fragment, index + 1, &mut |_| {});
    }
    let _ = packer.finish();
}

/// Output filename sanitization of remote titles, both modes.
pub fn sanitize_filename(data: &[u8]) {
    let text = text(data);
    let _ = crate::output::sanitize_filename(text, false);
    let _ = crate::output::sanitize_filename(text, true);
}

/// YouTube live chat continuations, live and replayed.
pub fn live_chat(data: &[u8]) {
    let Ok(chat) = serde_json::from_slice::<serde_json::Value>(data) else {
        return;
    };
    let mut output = Vec::new();
    let _ = crate::extractor::youtube::live_chat::parse_actions_replay(&chat, &mut output);
    let mut offset = 0;
    let _ = crate::extractor::youtube::live_chat::parse_actions_live(
        &chat,
        0,
        &mut offset,
        &mut output,
    );
}

/// Native cover-art embedding into downloaded FLAC, Ogg and MP4 files, and
/// thumbnail sniffing. The first byte picks the container; a fixed PNG is
/// the image.
pub fn cover_art(data: &[u8]) {
    const PNG: &[u8] = &[
        0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0,
        0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
    ];
    let _ = crate::cover_art::image_type(data);
    let _ = crate::cover_art::image_size(data);
    let Some((kind, file)) = data.split_first() else {
        return;
    };
    let picture = crate::cover_art::picture(PNG, "image/png", Some((1, 1)));
    let _ = match kind % 3 {
        0 => crate::cover_art::embed_flac(file, &picture),
        1 => crate::cover_art::embed_ogg(file, &picture),
        _ => crate::cover_art::embed_mp4(file, PNG, 14),
    };
}
