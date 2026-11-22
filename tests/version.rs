//! `--version` names this release and the upstream baseline it tracks.
use std::process::Command;

#[test]
fn version_names_the_upstream_baseline() {
    let manifest: serde_json::Value =
        serde_json::from_str(include_str!("../upstream/manifest.json")).unwrap();
    let upstream = manifest["upstream_version"].as_str().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .arg("--version")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "yt-dlp-rs {} (yt-dlp {upstream})\n",
            env!("CARGO_PKG_VERSION")
        )
    );
}
