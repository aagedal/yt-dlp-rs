//! Option validation failures follow upstream `parser.error`: a blank line,
//! the usage line, `PROG: error: MESSAGE` and exit status 2, before any request.
use std::process::Command;

fn run(args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args(args)
        .output()
        .unwrap();
    assert!(output.stdout.is_empty());
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn validation_messages_and_layout_follow_upstream() {
    let url = "http://127.0.0.1:9/never";
    for (args, message) in [
        (
            vec!["--audio-format", "xyz"],
            r#"invalid audio format "xyz" given"#,
        ),
        (
            vec!["--audio-quality", "-1"],
            r#"audio quality "-1" must be positive or 0"#,
        ),
        (
            vec!["--remux-video", "xyz"],
            r#"invalid remux video format "xyz" given"#,
        ),
        (
            vec!["--recode-video", "a>b>c"],
            r#"invalid recode video format "a>b>c" given"#,
        ),
        (
            vec!["--convert-subs", "xyz"],
            r#"invalid subtitle format "xyz" given"#,
        ),
        (
            vec!["--convert-thumbnails", "gif"],
            r#"invalid thumbnail format "gif" given"#,
        ),
        (
            vec!["--playlist-start", "0"],
            r#"playlist start "0" must be positive"#,
        ),
        (
            vec!["--playlist-start", "5", "--playlist-end", "2"],
            r#"playlist end "2" must be must be greater than or equal to playlist start "5""#,
        ),
        (
            vec!["--playlist-items", "1,abc"],
            "Invalid playlist-items '1,abc': 'abc' is not a valid specification",
        ),
        (
            vec!["--playlist-items", "1,,2"],
            "Invalid playlist-items '1,,2': There is two or more consecutive commas",
        ),
        (
            vec!["--playlist-items", "1:5:0"],
            "Invalid playlist-items '1:5:0': Step in '1:5:0' cannot be zero",
        ),
        (
            vec!["--sleep-interval", "-1"],
            r#"sleep interval "-1.0" must be positive or 0"#,
        ),
        (
            vec!["--min-filesize", "abc"],
            r#"invalid min filesize "abc" given"#,
        ),
        (
            vec!["--limit-rate", "0"],
            r#"rate limit "0" must be positive"#,
        ),
    ] {
        let mut all = args.clone();
        all.push(url);
        let (code, stderr) = run(&all);
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
        assert_eq!(
            stderr,
            format!("\nUsage: yt-dlp-rs [OPTIONS] URL [URL...]\n\nyt-dlp-rs: error: {message}\n"),
            "{args:?}"
        );
    }
}

#[test]
fn a_missing_url_is_reported_after_option_validation() {
    let (code, stderr) = run(&[]);
    assert_eq!(code, Some(2));
    assert_eq!(
        stderr,
        "\nUsage: yt-dlp-rs [OPTIONS] URL [URL...]\n\nyt-dlp-rs: error: You must provide at least one URL.\nType yt-dlp-rs --help to see a list of all options.\n"
    );
    let (code, stderr) = run(&["--date", "bogus"]);
    assert_eq!(code, Some(2));
    assert!(stderr.ends_with("yt-dlp-rs: error: Invalid date format \"bogus\"\n"));
}
