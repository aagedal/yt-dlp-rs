//! Site extractors against saved pinned-upstream output: every
//! `tests/fixtures/sites/SITE/expected.json` case (captured by
//! `tools/capture_sites.py`) runs through the fixture proxy, comparing the
//! exit code, stdout, `ERROR:`/`WARNING:` stderr lines and written files.
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::process::Command;

fn normalize(text: &str) -> Vec<String> {
    let epoch = regex::Regex::new(r#""epoch": \d+, "#).unwrap();
    // YouTube's `available_at` is the extraction time (plus any ad wait).
    let available = regex::Regex::new(r#""available_at": \d+"#).unwrap();
    // A comment's timestamp is estimated from its relative `_time_text`.
    let comment_time = regex::Regex::new(r#"("_time_text": "[^"]*", "timestamp": )\d+"#).unwrap();
    text.lines()
        .map(|line| {
            let line = epoch.replace(line, "");
            let line = available.replace_all(&line, r#""available_at": 0"#);
            comment_time.replace_all(&line, "${1}0").into_owned()
        })
        .collect()
}

/// Whether `deno` (upstream's default JavaScript runtime) is on `PATH`.
fn deno_available() -> bool {
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths).any(|dir| {
            dir.join(format!("deno{}", std::env::consts::EXE_SUFFIX))
                .is_file()
        })
    })
}

#[test]
fn site_extractors_match_saved_upstream_output() {
    let sites = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sites");
    let mut failures = Vec::new();
    for entry in std::fs::read_dir(&sites).unwrap() {
        let directory = entry.unwrap().path();
        let Ok(text) = std::fs::read_to_string(directory.join("expected.json")) else {
            continue;
        };
        let name = directory
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        // `SITES=a,b`: only these fixture directories (for local iteration).
        if let Ok(only) = std::env::var("SITES")
            && !only.split(',').any(|site| site == name)
        {
            continue;
        }
        let proxy = site_proxy::SiteProxy::start(&name);
        let expected: serde_json::Value = serde_json::from_str(&text).unwrap();
        for case in expected["cases"].as_array().unwrap() {
            // Cases solving YouTube challenges need upstream's default runtime.
            let requires_deno = case["requires"]
                .as_array()
                .is_some_and(|requires| requires.iter().any(|value| value == "deno"));
            if requires_deno && !deno_available() {
                continue;
            }
            let work = tempfile::tempdir().unwrap();
            let args: Vec<&str> = case["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap())
                .collect();
            // `--netrc-cmd` commands are captured as POSIX shell; upstream
            // runs them with cmd.exe on Windows, where they differ.
            if cfg!(windows) && args.contains(&"--netrc-cmd") {
                continue;
            }
            // `copy`: input files (such as a cookies file), not compared.
            let copied: Vec<&str> = case["copy"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|name| name.as_str())
                .collect();
            for name in &copied {
                std::fs::copy(directory.join(name), work.path().join(name)).unwrap();
            }
            // A fresh cache per case, like the capture's.
            let cache = tempfile::tempdir().unwrap();
            let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
                .args([
                    "--ignore-config",
                    "--proxy",
                    &proxy.url,
                    "--no-check-certificate",
                ])
                .args(["--user-agent", "fixture", "--cache-dir"])
                .arg(cache.path())
                .args(&args)
                .arg(case["url"].as_str().unwrap())
                .current_dir(work.path())
                .output()
                .unwrap();
            let stderr: Vec<String> = String::from_utf8_lossy(&output.stderr)
                .lines()
                .filter(|line| line.starts_with("ERROR:") || line.starts_with("WARNING:"))
                .map(str::to_owned)
                .collect();
            let mut files = serde_json::Map::new();
            let mut names: Vec<_> = std::fs::read_dir(work.path())
                .unwrap()
                .map(|entry| entry.unwrap())
                .collect();
            names.sort_by_key(|entry| entry.file_name());
            names.retain(|entry| !copied.iter().any(|name| entry.file_name() == **name));
            for entry in names {
                files.insert(
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.metadata().unwrap().len().into(),
                );
            }
            let actual = serde_json::json!({
                "exit": output.status.code(),
                "stdout": normalize(&String::from_utf8_lossy(&output.stdout)),
                "stderr": stderr,
                "files": files,
            });
            // Upstream was captured without impersonate targets; with the
            // `impersonate` feature the extractors impersonate instead of
            // warning (through the same fixture proxy).
            #[cfg(feature = "impersonate")]
            let case = &{
                let mut case = case.clone();
                if let Some(lines) = case["stderr"].as_array_mut() {
                    lines.retain(|line| {
                        !line.as_str().is_some_and(|line| {
                            line.contains("The extractor is attempting impersonation, but no impersonate target is available")
                        })
                    });
                }
                case
            };
            for key in ["exit", "stdout", "stderr", "files"] {
                if actual[key] != case[key] {
                    failures.push(format!(
                        "{name} {args:?} {key}:\n  upstream {}\n  native   {}",
                        case[key], actual[key]
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn hds_downloads_assemble_flv_like_upstream_f4mfd() {
    // The FLV header, the media's metadata tag, then each fragment's
    // `mdat` payload (md5-identical to pinned upstream's file).
    let proxy = site_proxy::SiteProxy::start("f4m");
    let work = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--proxy",
            &proxy.url,
            "--no-check-certificate",
        ])
        .args(["--no-progress", "-o", "o.%(ext)s"])
        .arg("https://media.example.test/hds/real.f4m")
        .current_dir(work.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata = b"\x02\x00\x0aonMetaData\x08\x00\x00\x00\x00\x00\x00\x09";
    let mut expected = b"FLV\x01\x05\x00\x00\x00\x09\x00\x00\x00\x00\x12".to_vec();
    expected.extend_from_slice(&(metadata.len() as u32).to_be_bytes()[1..]);
    expected.extend_from_slice(&[0; 7]);
    expected.extend_from_slice(metadata);
    expected.extend_from_slice(&(11 + metadata.len() as u32).to_be_bytes());
    expected.extend(b"TAG1".repeat(50));
    expected.extend(b"TAG2".repeat(50));
    assert_eq!(std::fs::read(work.path().join("o.flv")).unwrap(), expected);
}

/// The MIME boundary of an MHTML document.
fn mhtml_boundary(document: &[u8]) -> String {
    let text = String::from_utf8_lossy(document);
    let start = text.find("boundary=\"").unwrap() + "boundary=\"".len();
    text[start..start + text[start..].find('"').unwrap()].to_owned()
}

/// Replace every `from` in `bytes` with `to`.
fn replace_bytes(bytes: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at..].starts_with(from) {
            out.extend_from_slice(to);
            at += from.len();
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    out
}

#[test]
fn storyboards_download_as_mhtml_like_upstream_mhtmlfd() {
    // Pinned upstream's `-f sb0` file (upstream-sb0.mhtml): the same bytes
    // once the random boundary and the generator meta (this port names
    // itself) are aligned.
    let site = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sites/youtube-storyboard");
    let proxy = site_proxy::SiteProxy::start("youtube-storyboard");
    let work = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--proxy",
            &proxy.url,
            "--no-check-certificate",
        ])
        .args(["--user-agent", "fixture", "--no-js-runtimes", "-f", "sb0"])
        .args(["--no-progress", "-o", "%(id)s.%(ext)s"])
        .arg("https://www.youtube.com/watch?v=StoryBoard1")
        .current_dir(work.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains(
            "[mhtml] Total fragments: 8\n[download] Destination: StoryBoard1.mhtml\n[download] Download completed\n"
        ),
        "{stdout}"
    );
    let upstream = std::fs::read(site.join("upstream-sb0.mhtml")).unwrap();
    let native = std::fs::read(work.path().join("StoryBoard1.mhtml")).unwrap();
    let generator = concat!("yt-dlp-rs ", env!("CARGO_PKG_VERSION"));
    let shift = generator.len() - "yt-dlp 2026.08.19".len();
    let html = |document: &[u8]| {
        let text = String::from_utf8_lossy(document);
        let start = text.find("Content-Length: ").unwrap() + "Content-Length: ".len();
        let end = start + text[start..].find('\r').unwrap();
        text[start..end].parse::<usize>().unwrap()
    };
    let mut aligned = replace_bytes(
        &native,
        mhtml_boundary(&native).as_bytes(),
        mhtml_boundary(&upstream).as_bytes(),
    );
    aligned = replace_bytes(&aligned, generator.as_bytes(), b"yt-dlp 2026.08.19");
    aligned = replace_bytes(
        &aligned,
        format!("Content-Length: {}\r", html(&native)).as_bytes(),
        format!("Content-Length: {}\r", html(&native) - shift).as_bytes(),
    );
    assert_eq!(
        String::from_utf8_lossy(&aligned),
        String::from_utf8_lossy(&upstream)
    );
    assert_eq!(aligned, upstream);
    assert!(!work.path().join("StoryBoard1.mhtml.ytdl").exists());
}

#[test]
fn a_single_youtube_embed_is_followed_by_the_youtube_extractor() {
    // Upstream's `url_result(embed)`: the configured YouTube extractor
    // requests the watch page (which this fixture does not serve).
    let proxy = site_proxy::SiteProxy::start("youtube-embeds");
    let work = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--proxy",
            &proxy.url,
            "--no-check-certificate",
        ])
        .args(["-s", "https://blog.example.test/one"])
        .current_dir(work.path())
        .output()
        .unwrap();
    let requests = proxy.requests.lock().unwrap();
    assert!(
        requests.iter().any(|url| url
            == "https://www.youtube.com/watch?v=BaW_jenozKc&bpctr=9999999999&has_verified=1"),
        "{requests:?}"
    );
}

#[test]
fn youtube_live_chat_replays_are_written_like_upstream() {
    // Upstream `YoutubeLiveChatFD`'s JSON lines for the saved chat replay.
    let proxy = site_proxy::SiteProxy::start("youtube-live-chat");
    let work = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--proxy",
            &proxy.url,
            "--no-check-certificate",
        ])
        .args(["--no-js-runtimes", "--no-progress", "--skip-download"])
        .args(["--write-subs", "--sub-langs", "live_chat", "-o", "chat"])
        .arg("https://www.youtube.com/watch?v=ChatReplay1")
        .current_dir(work.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sites/youtube-live-chat/replay-upstream.live_chat.json");
    assert_eq!(
        std::fs::read(work.path().join("chat.live_chat.json")).unwrap(),
        std::fs::read(expected).unwrap()
    );
}
