//! Login behavior without an upstream comparison (the pinned upstream's
//! output for these is captured under `tests/fixtures/sites/*-login/`).
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::process::Command;

fn run(site: &str, args: &[&str]) -> (Option<i32>, String, Vec<String>) {
    let proxy = site_proxy::SiteProxy::start(site);
    let work = tempfile::tempdir().unwrap();
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
fn vimeo_logins_are_an_unsupported_error_before_any_request() {
    // Upstream logs in through the viewer token and then extracts as the
    // account, which is not ported.
    for credentials in [
        &["-u", "user@example.com", "-p", "pw"][..],
        &["--netrc-cmd", "printf 'machine vimeo login a password b'"],
    ] {
        let mut args = vec!["-s"];
        args.extend_from_slice(credentials);
        args.push("https://vimeo.com/76979871");
        let (code, stderr, requests) = run("vimeo", &args);
        assert_eq!(code, Some(1), "{stderr}");
        assert!(
            stderr.contains(
                "ERROR: https://vimeo.com/76979871: unsupported HTML5 media: Vimeo logins (--username/--netrc) are not supported natively"
            ),
            "{stderr}"
        );
        assert!(requests.is_empty(), "{requests:?}");
    }
    // Without credentials the extraction is unchanged.
    let (code, stderr, _) = run("vimeo", &["-s", "https://vimeo.com/76979871"]);
    assert_eq!(code, Some(0), "{stderr}");
}
