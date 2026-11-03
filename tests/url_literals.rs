//! The generated `src/extractor/url_literals.rs`: every `UrlRegex` pattern
//! dispatch builds has an entry, and its literals are sound (every match
//! contains one). Regenerate with
//! `YT_DLP_RS_WRITE_URL_LITERALS=1 cargo test --test url_literals`.
use regex_syntax::hir::{Hir, HirKind};
use std::{collections::BTreeMap, path::Path};
use yt_dlp_rs::__private::{record_url_patterns, recorded_url_patterns, url_pattern_hash};

/// Larger sets (PeerTube's instance list has about 1000 hosts) cost more to
/// check than they save.
const MAX_LITERALS: usize = 4096;

/// Literals of which every match of `hir` contains one, or `None`.
fn required(hir: &Hir) -> Option<Vec<String>> {
    match hir.kind() {
        HirKind::Literal(literal) => Some(vec![String::from_utf8(literal.0.to_vec()).ok()?]),
        HirKind::Capture(capture) => required(&capture.sub),
        HirKind::Repetition(repetition) if repetition.min >= 1 => required(&repetition.sub),
        HirKind::Alternation(branches) => {
            let mut all = Vec::new();
            for branch in branches {
                all.extend(required(branch)?);
                if all.len() > MAX_LITERALS {
                    return None;
                }
            }
            all.sort();
            all.dedup();
            Some(all)
        }
        HirKind::Concat(parts) => {
            // A match contains every part's match; adjacent literals are one
            // longer literal.
            let mut candidates = Vec::new();
            let mut run = Vec::new();
            for part in parts {
                if let HirKind::Literal(literal) = part.kind() {
                    run.extend_from_slice(&literal.0);
                    continue;
                }
                if !run.is_empty() {
                    candidates.push(vec![String::from_utf8(std::mem::take(&mut run)).ok()?]);
                }
                candidates.extend(required(part));
            }
            if !run.is_empty() {
                candidates.push(vec![String::from_utf8(run).ok()?]);
            }
            candidates.into_iter().max_by_key(|set| score(set))
        }
        _ => None,
    }
}

/// A set's shortest literal, counting pieces of every URL (`https://www.`)
/// and literals under three bytes as 0, then fewer literals.
fn score(set: &[String]) -> (usize, std::cmp::Reverse<usize>) {
    let shortest = set
        .iter()
        .map(|literal| {
            if literal.len() < 3 || "https://www.".contains(literal.as_str()) {
                0
            } else {
                literal.len()
            }
        })
        .min()
        .unwrap_or(0);
    (shortest, std::cmp::Reverse(set.len()))
}

fn literals(pattern: &str) -> Vec<String> {
    let hir = regex_syntax::Parser::new()
        .parse(pattern)
        .unwrap_or_else(|error| panic!("{pattern}: {error}"));
    required(&hir)
        .filter(|set| score(set).0 > 0)
        .unwrap_or_default()
}

/// Every URL in the site fixtures: case inputs and saved routes.
fn fixture_urls() -> Vec<String> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sites");
    let mut urls = Vec::new();
    for site in std::fs::read_dir(root).unwrap() {
        let site = site.unwrap().path();
        if let Ok(text) = std::fs::read_to_string(site.join("cases.json")) {
            let cases: serde_json::Value = serde_json::from_str(&text).unwrap();
            for case in cases.as_array().into_iter().flatten() {
                urls.extend(case["url"].as_str().map(str::to_owned));
            }
        }
        if let Ok(text) = std::fs::read_to_string(site.join("routes.json")) {
            let routes: serde_json::Value = serde_json::from_str(&text).unwrap();
            for route in routes["routes"].as_array().into_iter().flatten() {
                urls.extend(route["url"].as_str().map(str::to_owned));
            }
        }
    }
    urls.sort();
    urls.dedup();
    urls
}

#[test]
fn url_literals_cover_every_dispatch_pattern() {
    record_url_patterns();
    // No site extractor takes this URL, so every one declines it (and
    // builds its patterns) before the generic extractor's request fails.
    let ydl = yt_dlp_rs::YoutubeDl::new().unwrap();
    assert!(ydl.extract_info("http://127.0.0.1:1/video.mp4").is_err());
    let patterns = recorded_url_patterns();
    assert!(patterns.len() > 800, "only {} patterns", patterns.len());

    let mut table: BTreeMap<u64, (String, Vec<String>)> = BTreeMap::new();
    for pattern in patterns {
        let hash = url_pattern_hash(&pattern);
        if let Some((other, _)) = table.get(&hash) {
            assert_eq!(*other, pattern, "hash collision");
            continue;
        }
        let literals = literals(&pattern);
        table.insert(hash, (pattern, literals));
    }

    // Sound on real inputs too: whenever a pattern matches a fixture URL,
    // the URL contains one of its literals.
    let urls = fixture_urls();
    assert!(urls.len() > 1000, "only {} fixture URLs", urls.len());
    for (pattern, literals) in table.values() {
        let regex = regex::Regex::new(pattern).unwrap();
        if literals.is_empty() {
            continue;
        }
        for url in &urls {
            if regex.is_match(url) {
                assert!(
                    literals
                        .iter()
                        .any(|literal| url.contains(literal.as_str())),
                    "{url} matches {pattern} without any of {literals:?}"
                );
            }
        }
    }

    let mut text = String::from(
        "// Generated by `YT_DLP_RS_WRITE_URL_LITERALS=1 cargo test --test url_literals`.\n\
         pub(super) static LITERALS: &[(u64, &[&str])] = &[\n",
    );
    for (hash, (_, literals)) in &table {
        let literals: Vec<String> = literals
            .iter()
            .map(|literal| format!("{literal:?}"))
            .collect();
        text.push_str(&format!(
            "    (0x{hash:016x}, &[{}]),\n",
            literals.join(", ")
        ));
    }
    text.push_str("];\n");
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/extractor/url_literals.rs");
    if std::env::var_os("YT_DLP_RS_WRITE_URL_LITERALS").is_some() {
        std::fs::write(&path, text).unwrap();
    } else {
        assert!(
            std::fs::read_to_string(&path).unwrap() == text,
            "src/extractor/url_literals.rs is stale; run \
             YT_DLP_RS_WRITE_URL_LITERALS=1 cargo test --test url_literals"
        );
    }
}
