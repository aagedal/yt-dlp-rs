//! Upstream `YoutubeIE._mark_watched` (`--mark-watched` with cookies): the
//! player responses' `videostats` playback and watchtime pings.
use reqwest::blocking::Client;
use serde_json::Value;
use url::Url;

const CPN_ALPHABET: &[u8; 64] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-_";

/// `CPN_ALPHABET[random.randint(0, 256) & 63]` sixteen times.
fn cpn() -> String {
    use std::hash::{BuildHasher, Hasher};
    (0..16)
        .map(|index| {
            let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
            hasher.write_usize(index);
            CPN_ALPHABET[(hasher.finish() % 257) as usize & 63] as char
        })
        .collect()
}

/// The ping URL for `base`: its query with `ver`, `cpn`, `cmt` (the `len`
/// parameter minus one second, as a Python float) and `el`, plus `st` and
/// `et` for the watchtime ping (`urlencode(qs, True)`).
fn ping_url(base: &Url, cpn: &str, full: bool) -> Url {
    let length = base
        .query_pairs()
        .find(|(key, value)| key == "len" && !value.is_empty())
        .and_then(|(_, value)| value.trim().parse::<f64>().ok())
        .unwrap_or(1.5)
        - 1.0;
    let length = crate::screen::py_float(length);
    let mut updates = vec![
        ("ver", "2"),
        ("cpn", cpn),
        ("cmt", length.as_str()),
        ("el", "detailpage"),
    ];
    if full {
        updates.extend([("st", "0"), ("et", length.as_str())]);
    }
    super::super::update_url_query(base, &updates)
}

/// `_mark_watched`: `Marking watched`, then `Marking fully watched`
/// (non-fatal requests); a missing URL warns and stops.
pub(super) fn mark(client: &Client, video_id: &str, player_responses: &[Value]) {
    let cpn = cpn();
    for (full, key) in [
        (false, "videostatsPlaybackUrl"),
        (true, "videostatsWatchtimeUrl"),
    ] {
        let label = if full { "fully " } else { "" };
        let base = player_responses.iter().find_map(|response| {
            response
                .get("playbackTracking")?
                .get(key)?
                .get("baseUrl")?
                .as_str()
                .and_then(super::super::json_ld::url_or_none)
        });
        let Some(base) = base.and_then(|base| Url::parse(&base).ok()) else {
            crate::screen::warning(&format!("[youtube] Unable to mark {label}watched"));
            return;
        };
        let url = ping_url(&base, &cpn, full);
        if let Err(error) = super::super::download_webpage_note(
            client,
            &url,
            "youtube",
            video_id,
            &format!("Marking {label}watched"),
        ) {
            // `errnote: err` with the request's own error.
            let error = match &error {
                crate::Error::ExtractorRequest { source, .. } => source,
                error => error,
            };
            crate::screen::warning(&format!(
                "[youtube] {video_id}: Unable to mark watched: {}",
                crate::upstream_error_text(error)
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ping_urls_follow_upstream_query_updates() {
        let base = Url::parse(
            "https://s.youtube.com/api/stats/playback?cl=1&docid=X&len=10&ns=yt&el=embedded",
        )
        .unwrap();
        assert_eq!(
            ping_url(&base, "CPN", false).as_str(),
            "https://s.youtube.com/api/stats/playback?cl=1&docid=X&len=10&ns=yt&el=detailpage&ver=2&cpn=CPN&cmt=9.0"
        );
        let base = Url::parse("https://s.youtube.com/api/stats/watchtime?docid=X").unwrap();
        assert_eq!(
            ping_url(&base, "CPN", true).as_str(),
            "https://s.youtube.com/api/stats/watchtime?docid=X&ver=2&cpn=CPN&cmt=0.5&el=detailpage&st=0&et=0.5"
        );
        let cpn = cpn();
        assert_eq!(cpn.len(), 16);
        assert!(cpn.bytes().all(|byte| CPN_ALPHABET.contains(&byte)));
    }
}
