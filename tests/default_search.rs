//! `--default-search` for inputs without a scheme (upstream
//! `GenericIE._real_extract`). Searches go to a dead proxy, so only the
//! dispatch is observed.
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(["--ignore-config", "--proxy", "http://127.0.0.1:9", "-s"])
        .args(args)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn auto_searches_youtube_and_auto_warning_says_so() {
    let output = run(&["--default-search", "auto", "some query"]);
    let text = stderr(&output);
    assert!(
        text.contains("[youtube:search] query \"some query\": "),
        "{text}"
    );
    assert!(!text.contains("Falling back"), "{text}");
    let output = run(&["--default-search", "auto_warning", "some query"]);
    let text = stderr(&output);
    assert!(
        text.contains("WARNING: [generic] Falling back to youtube search for  some query . Set --default-search \"auto\" to suppress this warning."),
        "{text}"
    );
    assert!(
        text.contains("[youtube:search] query \"some query\": "),
        "{text}"
    );
    let output = run(&["--default-search", "auto_warning", "URL"]);
    assert!(
        stderr(&output).contains("ERROR: [generic] Invalid URL:  'URL' . Call yt-dlp like this:"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_prefix_selects_its_search_and_errors_stay_upstreams() {
    let output = run(&["--default-search", "ytsearch3", "cats"]);
    assert!(
        stderr(&output).contains("[youtube:search] query \"cats\": "),
        "{}",
        stderr(&output)
    );
    for mode in ["error", "fixup_error"] {
        let output = run(&["--default-search", mode, "cats"]);
        assert_eq!(output.status.code(), Some(1));
        assert!(
            stderr(&output).contains("ERROR: [generic] 'cats' is not a valid URL"),
            "{}",
            stderr(&output)
        );
    }
    // A prefix without a search extractor is an unsupported URL.
    let output = run(&["--default-search", "nosuchsearch", "cats"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(!stderr(&output).contains("not supported natively"));
}
