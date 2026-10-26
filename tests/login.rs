//! Login behavior without an upstream comparison (the pinned upstream's
//! output for these is captured under `tests/fixtures/sites/*-login/`).
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::process::Command;

fn run(site: &str, args: &[&str]) -> (Option<i32>, String, Vec<String>) {
    run_with(site, args, &[])
}

/// [`run`] with fixture files copied into the working directory.
fn run_with(site: &str, args: &[&str], copy: &[&str]) -> (Option<i32>, String, Vec<String>) {
    let proxy = site_proxy::SiteProxy::start(site);
    let work = tempfile::tempdir().unwrap();
    for name in copy {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/sites")
            .join(site)
            .join(name);
        std::fs::copy(source, work.path().join(name)).unwrap();
    }
    let output = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"))
        .args([
            "--ignore-config",
            "--proxy",
            &proxy.url,
            "--no-check-certificate",
        ])
        .args(["--user-agent", "fixture"])
        .args(args)
        .current_dir(work.path())
        .output()
        .unwrap();
    let requests = proxy.requests.lock().unwrap().clone();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        requests,
    )
}

#[test]
fn vimeo_logins_post_the_viewer_token_unless_a_session_cookie_is_set() {
    const URL: &str = "https://vimeo.com/123456789/abcdef0123";
    const VIEWER: &str = "https://vimeo.com/_next/viewer";
    const LOGIN: &str = "https://vimeo.com/log_in";
    let count = |requests: &[String], url: &str| requests.iter().filter(|r| *r == url).count();
    // The logged-out viewer's XSRF token, the login form, then the
    // logged-in viewer's JWT for the web API.
    let (code, stderr, requests) = run(
        "vimeo-login",
        &[
            "--print",
            "id",
            "-u",
            "user@example.com",
            "-p",
            "s3cret&pw",
            URL,
        ],
    );
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(count(&requests, VIEWER), 2, "{requests:?}");
    assert_eq!(count(&requests, LOGIN), 1, "{requests:?}");
    let login = requests.iter().position(|r| r == LOGIN).unwrap();
    assert_eq!(requests[login - 1], VIEWER, "{requests:?}");
    // A `vimeo` cookie skips the login: only the logged-in viewer.
    let (code, stderr, requests) = run_with(
        "vimeo-login",
        &[
            "--print",
            "id",
            "--cookies",
            "cookies.txt",
            "-u",
            "user@example.com",
            "-p",
            "wrong",
            URL,
        ],
        &["cookies.txt"],
    );
    assert_eq!(code, Some(0), "{stderr}");
    assert_eq!(count(&requests, VIEWER), 1, "{requests:?}");
    assert_eq!(count(&requests, LOGIN), 0, "{requests:?}");
    // A failed login is retried by the next URL (`_ready` stays unset).
    let (code, stderr, requests) = run(
        "vimeo-login",
        &["-s", "-u", "user@example.com", "-p", "wrong", URL, URL],
    );
    assert_eq!(code, Some(1), "{stderr}");
    assert_eq!(count(&requests, LOGIN), 2, "{requests:?}");
    assert_eq!(
        stderr
            .matches("ERROR: [vimeo] 123456789: Unable to log in: bad username or password")
            .count(),
        2,
        "{stderr}"
    );
}
