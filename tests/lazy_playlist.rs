//! `--lazy-playlist` reads a paged playlist's entries while processing them
//! (upstream's lazy `PlaylistEntries`), so pages after the last processed
//! entry are never fetched. The request counts and line order were checked
//! with pinned upstream (`python3 -m yt_dlp`) through the same fixture routes.
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::process::{Command, Output};

fn run(proxy: &site_proxy::SiteProxy, args: &[&str], url: &str) -> Output {
    let work = tempfile::tempdir().unwrap();
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--proxy", &proxy.url])
        .args(["--no-check-certificate", "--user-agent", "fixture"])
        .args(args)
        .arg(url)
        .current_dir(work.path())
        .output()
        .unwrap()
}

fn count(proxy: &site_proxy::SiteProxy, url: &str) -> usize {
    proxy
        .requests
        .lock()
        .unwrap()
        .iter()
        .filter(|request| *request == url)
        .count()
}

const CHANNEL: &str = "https://www.bitchute.com/channel/testchan/";
const EXTEND: &str = "https://old.bitchute.com/channel/testchan/extend/";

#[test]
fn max_downloads_stops_a_lazy_playlist_before_its_next_page() {
    // Two pages of 25 entries (27 in all) behind the channel page.
    let proxy = site_proxy::SiteProxy::start("bitchute");
    let output = run(&proxy, &["--max-downloads", "1", "--print", "id"], CHANNEL);
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "UGlrF9o9b-Q\n");
    // Without --lazy-playlist every requested entry is read first.
    assert_eq!(count(&proxy, EXTEND), 2);

    let proxy = site_proxy::SiteProxy::start("bitchute");
    let output = run(
        &proxy,
        &["--lazy-playlist", "--max-downloads", "1", "--print", "id"],
        CHANNEL,
    );
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(String::from_utf8_lossy(&output.stdout), "UGlrF9o9b-Q\n");
    assert_eq!(count(&proxy, EXTEND), 1);
}

#[test]
fn lazy_playlist_pages_are_fetched_when_their_entries_are_reached() {
    // A random-access paged list (`OnDemandPagedList`) of 102 entries in
    // pages of 100: items 99-102 span both pages.
    let proxy = site_proxy::SiteProxy::start("dailymotion");
    let output = run(
        &proxy,
        &["--lazy-playlist", "--flat-playlist", "-I", "99:"],
        "https://www.dailymotion.com/playlist/x6hynp",
    );
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<_> = stdout.lines().skip(1).collect();
    assert_eq!(
        lines,
        [
            "[download] Downloading playlist: x6hynp",
            // No count is known before the pages are read.
            "[dailymotion:playlist] Playlist x6hynp: Downloading N/A items",
            "[dailymotion:playlist] Downloading Access Token",
            "[dailymotion:playlist] x6hynp: Downloading page 1",
            "[download] Downloading item 1 of N/A",
            "[download] Downloading item 2 of N/A",
            "[dailymotion:playlist] x6hynp: Downloading page 2",
            "[download] Downloading item 3 of N/A",
            "[download] Downloading item 4 of N/A",
            "[download] Finished downloading playlist: x6hynp",
        ]
    );
    assert_eq!(count(&proxy, "https://graphql.api.dailymotion.com/"), 2);

    // Item 1 never needs the second page; neither does the playlist count.
    let proxy = site_proxy::SiteProxy::start("dailymotion");
    let output = run(
        &proxy,
        &[
            "--lazy-playlist",
            "--flat-playlist",
            "-I",
            "1",
            "--print",
            "%(playlist_index)s|%(id)s|%(playlist_count)s",
        ],
        "https://www.dailymotion.com/playlist/x6hynp",
    );
    assert!(output.status.success());
    // `__last_playlist_index` is 0 for a lazy playlist: no index padding.
    assert_eq!(String::from_utf8_lossy(&output.stdout), "1|x7p001|NA\n");
    assert_eq!(count(&proxy, "https://graphql.api.dailymotion.com/"), 1);
}
