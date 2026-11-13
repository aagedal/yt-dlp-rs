//! `--write-comments`/`--get-comments` for an extractor whose upstream
//! `_get_comments` is not ported (SoundCloud here): a warning says so,
//! instead of the comments being silently absent.
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::process::Command;

const SOUNDCLOUD: &str = "https://soundcloud.com/ethmusic/lostin-powers-she-so-heavy";

#[test]
fn unported_comment_extraction_warns() {
    let proxy = site_proxy::SiteProxy::start("soundcloud");
    let directory = tempfile::tempdir().unwrap();
    for (comments, warned) in [(true, true), (false, false)] {
        let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
            .args(["--ignore-config", "--proxy", &proxy.url])
            .args([
                "--no-check-certificate",
                "--user-agent",
                "fixture",
                "--cache-dir",
            ])
            .arg(directory.path().join("cache"))
            .args(["-s", "--print", "id"])
            .args(comments.then_some("--get-comments"))
            .arg(SOUNDCLOUD)
            .current_dir(directory.path())
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(output.status.success(), "{stderr}");
        let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        assert_eq!(
            stderr.contains(&format!(
                "WARNING: [soundcloud] {id}: Comments are not supported natively and are not extracted\n"
            )),
            warned,
            "{stderr}"
        );
    }
}
