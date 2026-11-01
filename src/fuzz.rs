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
