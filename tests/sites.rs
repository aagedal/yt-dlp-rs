//! Site extractors against saved pinned-upstream output: every
//! `tests/fixtures/sites/SITE/expected.json` case (captured by
//! `tools/capture_sites.py`) runs through the fixture proxy, comparing the
//! exit code, stdout, `ERROR:`/`WARNING:` stderr lines and written files.
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::process::Command;

fn normalize(text: &str) -> Vec<String> {
    let epoch = regex::Regex::new(r#""epoch": \d+, "#).unwrap();
    text.lines()
        .map(|line| epoch.replace(line, "").into_owned())
        .collect()
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
        let proxy = site_proxy::SiteProxy::start(&name);
        let expected: serde_json::Value = serde_json::from_str(&text).unwrap();
        for case in expected["cases"].as_array().unwrap() {
            let work = tempfile::tempdir().unwrap();
            let args: Vec<&str> = case["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap())
                .collect();
            let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
                .args([
                    "--ignore-config",
                    "--proxy",
                    &proxy.url,
                    "--no-check-certificate",
                ])
                .args(["--user-agent", "fixture"])
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
