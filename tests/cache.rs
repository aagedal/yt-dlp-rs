//! Upstream `Cache` (`--cache-dir`, `--no-cache-dir`, `--rm-cache-dir`):
//! extractors store data on the first run and read it on the second, with
//! the messages pinned upstream prints (compared by hand against
//! `python3 -m yt_dlp` through the same fixture routes).
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::path::Path;
use std::process::{Command, Output};

/// Run against `site`'s fixture routes with `cache` as `--cache-dir`.
fn run(proxy: &site_proxy::SiteProxy, cache: &Path, args: &[&str], url: &str) -> Output {
    let work = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--proxy",
            &proxy.url,
            "--no-check-certificate",
        ])
        .args(["--user-agent", "fixture", "--cache-dir"])
        .arg(cache)
        .args(args)
        .arg(url)
        .current_dir(work.path())
        .output()
        .unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Debug lines about the cache.
fn cache_debug(output: &Output) -> Vec<String> {
    text(&output.stderr)
        .lines()
        .filter(|line| line.contains(" cache") && !line.starts_with("[debug] Command-line"))
        .map(str::to_owned)
        .collect()
}

const SOUNDCLOUD: &str = "https://soundcloud.com/ethmusic/lostin-powers-she-so-heavy";

#[test]
fn soundcloud_client_id_is_read_from_the_cache_on_the_second_run() {
    let proxy = site_proxy::SiteProxy::start("soundcloud");
    let directory = tempfile::tempdir().unwrap();
    let cache = directory.path().join("cache");
    let first = run(&proxy, &cache, &["-v", "-s"], SOUNDCLOUD);
    assert!(first.status.success(), "{}", text(&first.stderr));
    assert!(text(&first.stdout).starts_with(
        "[soundcloud] Downloading main page\n[soundcloud] Downloading JS asset\n[soundcloud] Extracting URL: "
    ));
    assert_eq!(
        cache_debug(&first),
        ["[debug] Saving soundcloud.client_id to cache"]
    );
    // Upstream's file format: `json.dump` of the versioned envelope.
    assert_eq!(
        std::fs::read_to_string(cache.join("soundcloud/client_id.json")).unwrap(),
        r#"{"yt-dlp_version": "2026.10.26", "data": "aaaaaaaaaaaaaaaaBBBBBBBBBBBBBBBB"}"#
    );
    let second = run(&proxy, &cache, &["-v", "-s"], SOUNDCLOUD);
    assert!(second.status.success(), "{}", text(&second.stderr));
    assert!(
        text(&second.stdout).starts_with("[soundcloud] Extracting URL: "),
        "{}",
        text(&second.stdout)
    );
    assert_eq!(
        cache_debug(&second),
        ["[debug] Loading soundcloud.client_id from cache"]
    );
    // The rest of the output is unchanged.
    let rest = |output: &Output| {
        text(&output.stdout)
            .lines()
            .skip_while(|line| !line.contains("Extracting URL"))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    assert_eq!(rest(&first), rest(&second));
}

#[test]
fn corrupt_entries_warn_and_disabled_caches_are_untouched() {
    let proxy = site_proxy::SiteProxy::start("soundcloud");
    let directory = tempfile::tempdir().unwrap();
    let cache = directory.path().join("cache");
    std::fs::create_dir_all(cache.join("soundcloud")).unwrap();
    std::fs::write(cache.join("soundcloud/client_id.json"), "{").unwrap();
    let output = run(&proxy, &cache, &["-s"], SOUNDCLOUD);
    assert!(output.status.success());
    let warning = format!(
        "WARNING: Cache retrieval from {} failed (1)",
        cache.join("soundcloud").join("client_id.json").display()
    );
    assert!(
        text(&output.stderr).lines().any(|line| line == warning),
        "{}",
        text(&output.stderr)
    );
    assert!(text(&output.stdout).starts_with("[soundcloud] Downloading main page"));
    // The corrupt entry was replaced.
    assert!(
        std::fs::read_to_string(cache.join("soundcloud/client_id.json"))
            .unwrap()
            .contains("aaaaaaaaaaaaaaaaBBBBBBBBBBBBBBBB")
    );
    // `--no-cache-dir` (after `--cache-dir`) neither reads nor writes.
    let disabled = directory.path().join("disabled");
    let output = run(&proxy, &disabled, &["--no-cache-dir", "-s"], SOUNDCLOUD);
    assert!(output.status.success());
    assert!(text(&output.stdout).starts_with("[soundcloud] Downloading main page"));
    assert!(!disabled.exists());
}

const BRIGHTCOVE: &str =
    "https://players.brightcove.net/1111111111001/default_default/index.html?videoId=6300000000001";

fn metadata_requests(output: &Output) -> usize {
    text(&output.stdout)
        .lines()
        .filter(|line| line.ends_with("Downloading JSON metadata"))
        .count()
}

#[test]
fn brightcove_policy_key_is_cached_and_a_stale_key_is_replaced() {
    let proxy = site_proxy::SiteProxy::start("brightcove");
    let directory = tempfile::tempdir().unwrap();
    let cache = directory.path().join("cache");
    let entry = cache.join("brightcove/1111111111001_default.json");
    // `config.json`, then the Playback API.
    let first = run(&proxy, &cache, &["-v", "-s"], BRIGHTCOVE);
    assert!(first.status.success(), "{}", text(&first.stderr));
    assert_eq!(metadata_requests(&first), 2);
    assert_eq!(
        cache_debug(&first),
        ["[debug] Saving brightcove.1111111111001_default to cache"]
    );
    assert_eq!(
        std::fs::read_to_string(&entry).unwrap(),
        r#"{"yt-dlp_version": "2026.10.26", "data": "BCpkFIXTUREKEY1"}"#
    );
    // Only the Playback API, with the cached key.
    let second = run(&proxy, &cache, &["-v", "-s"], BRIGHTCOVE);
    assert!(second.status.success(), "{}", text(&second.stderr));
    assert_eq!(metadata_requests(&second), 1);
    assert_eq!(
        cache_debug(&second),
        ["[debug] Loading brightcove.1111111111001_default from cache"]
    );
    // `INVALID_POLICY_KEY` for a cached key: cleared, extracted, stored.
    std::fs::write(
        &entry,
        r#"{"yt-dlp_version": "2026.10.26", "data": "BCpkSTALEKEY"}"#,
    )
    .unwrap();
    let stale = run(&proxy, &cache, &["-v", "-s"], BRIGHTCOVE);
    assert!(stale.status.success(), "{}", text(&stale.stderr));
    assert_eq!(metadata_requests(&stale), 3);
    assert_eq!(
        cache_debug(&stale),
        [
            "[debug] Loading brightcove.1111111111001_default from cache",
            "[debug] Saving brightcove.1111111111001_default to cache",
            "[debug] Saving brightcove.1111111111001_default to cache",
        ]
    );
    assert!(
        std::fs::read_to_string(&entry)
            .unwrap()
            .contains("BCpkFIXTUREKEY1")
    );
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
fn youtube_signature_specs_are_read_from_the_cache_on_the_second_run() {
    if !deno_available() {
        return;
    }
    let proxy = site_proxy::SiteProxy::start("youtube-js");
    let directory = tempfile::tempdir().unwrap();
    let cache = directory.path().join("cache");
    let args = ["-v", "--print", "%(formats.:.url)j"];
    let url = "https://www.youtube.com/watch?v=BaW_jenozKc";
    let first = run(&proxy, &cache, &args, url);
    assert!(first.status.success(), "{}", text(&first.stderr));
    assert_eq!(
        cache_debug(&first),
        ["[debug] Saving youtube-sigfuncs.74edf1a3-main-110 to cache"]
    );
    let entry: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(cache.join("youtube-sigfuncs/74edf1a3-main-110.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(entry["yt-dlp_version"], "2026.10.26");
    assert_eq!(entry["data"].as_array().unwrap().len(), 110);
    let second = run(&proxy, &cache, &args, url);
    assert!(second.status.success(), "{}", text(&second.stderr));
    assert_eq!(
        cache_debug(&second),
        ["[debug] Loading youtube-sigfuncs.74edf1a3-main-110 from cache"]
    );
    assert_eq!(text(&first.stdout), text(&second.stdout));
    assert!(text(&second.stdout).contains("&sig=NJAJEij0EwRgIhAI0KExTgjfPk"));
    // Entries from before 2025.07.21 are discarded and solved again.
    std::fs::write(
        cache.join("youtube-sigfuncs/74edf1a3-main-110.json"),
        entry["data"].to_string(),
    )
    .unwrap();
    let old = run(&proxy, &cache, &args, url);
    assert_eq!(
        cache_debug(&old),
        [
            "[debug] Loading youtube-sigfuncs.74edf1a3-main-110 from cache",
            "[debug] Discarding old cache from version 2022.08.19 (needs 2025.07.21)",
            "[debug] Saving youtube-sigfuncs.74edf1a3-main-110 to cache",
        ]
    );
    assert_eq!(text(&first.stdout), text(&old.stdout));
}

#[test]
fn rm_cache_dir_follows_upstream_cache_remove() {
    let directory = tempfile::tempdir().unwrap();
    let cache = directory.path().join("cache");
    std::fs::create_dir_all(cache.join("soundcloud")).unwrap();
    let remove = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config"])
            .args(args)
            .output()
            .unwrap()
    };
    let cache_arg = cache.to_str().unwrap();
    let output = remove(&["--cache-dir", cache_arg, "--rm-cache-dir"]);
    assert!(output.status.success());
    assert_eq!(
        text(&output.stdout),
        format!("Removing cache dir {cache_arg} ...\n")
    );
    assert!(!cache.exists());
    let output = remove(&["--cache-dir", cache_arg, "--rm-cache-dir"]);
    assert_eq!(
        text(&output.stdout),
        format!("Removing cache dir {cache_arg} ..\n")
    );
    let output = remove(&["--cache-dir", cache_arg, "--no-cache-dir", "--rm-cache-dir"]);
    assert_eq!(
        text(&output.stdout),
        "Cache is disabled (Did you combine --no-cache-dir and --rm-cache-dir?)\n"
    );
    // A directory without `cache` or `tmp` in its path is refused.
    let output = remove(&["--cache-dir", "/nonexistent/yt-dlp-rs", "--rm-cache-dir"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(text(&output.stderr).contains(
        "ERROR: Not removing directory /nonexistent/yt-dlp-rs - this does not look like a cache dir"
    ));
    // The default is `${XDG_CACHE_HOME}/yt-dlp-rs`.
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--rm-cache-dir"])
        .env("XDG_CACHE_HOME", directory.path().join("xdg-cache"))
        .output()
        .unwrap();
    assert_eq!(
        text(&output.stdout),
        format!(
            "Removing cache dir {} ..\n",
            directory.path().join("xdg-cache/yt-dlp-rs").display()
        )
    );
}
