//! Adobe Pass (`AdobePassIE`): `--ap-list-mso`, the TV provider option
//! validation and the request sequences of full MVPD logins through the
//! fixture proxy (`tests/fixtures/sites/adobepass`, whose routes also
//! require each request's body and headers). tests/sites.rs compares the
//! same cases' output with the pinned upstream.
#[path = "helpers/site_proxy.rs"]
mod site_proxy;

use std::process::Command;

fn binary() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_yt-dlp-rs"));
    command.arg("--ignore-config");
    command
}

const VIDEO: &str = "https://players.brightcove.net/7777777777001/default_default/index.html?videoId=6400000000001#__youtubedl_smuggle=%7B%22source_url%22%3A+%22https%3A%2F%2Fwww.tvsite.example%2Fwatch%2Ffixture%22%2C+%22software_statement%22%3A+%22fixture-software-statement%22%7D";

#[test]
fn list_mso_prints_upstreams_table() {
    let output = binary().arg("--ap-list-mso").output().unwrap();
    assert!(output.status.success());
    let expected = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/adobepass/list_mso.txt"),
    )
    .unwrap();
    assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
}

#[test]
fn tv_provider_options_are_validated_like_upstream() {
    for (args, message) in [
        (
            &["--ap-mso", "Nope"][..],
            "yt-dlp-rs: error: Unsupported TV Provider \"Nope\", use --ap-list-mso to get a list of supported TV Providers",
        ),
        (
            &["--ap-password", "secret"][..],
            "yt-dlp-rs: error: TV Provider account username missing",
        ),
    ] {
        let output = binary()
            .args(args)
            .arg("https://example.com/")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.lines().any(|line| line == message), "{stderr}");
    }
    // Upstream would prompt for the password (getpass).
    let output = binary()
        .args(["--ap-username", "user", "https://example.com/"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8(output.stderr).unwrap().contains(
        "ERROR: --ap-username without --ap-password needs an interactive password prompt"
    ));
}

/// The requested URLs of a run of `VIDEO` with `--ap-mso MSO`.
fn requests(mso: &str) -> Vec<String> {
    let proxy = site_proxy::SiteProxy::start("adobepass");
    let output = binary()
        .args([
            "--proxy",
            &proxy.url,
            "--no-check-certificate",
            "--user-agent",
            "fixture",
        ])
        .args(["-j", "--ap-mso", mso, "--ap-username", "fixture-user"])
        .args(["--ap-password", "fixture-pass", VIDEO])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let info: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(info["title"], "TV Everywhere fixture");
    proxy.requests.lock().unwrap().clone()
}

const ADOBE_START: [&str; 6] = [
    "https://players.brightcove.net/7777777777001/default_default/config.json",
    "https://edge.api.brightcove.com/playback/v1/accounts/7777777777001/videos/6400000000001",
    "https://sp.auth.adobe.com/indiv/devices",
    "https://sp.auth.adobe.com/o/client/register",
    "https://sp.auth.adobe.com/o/client/token",
    "https://sp.auth.adobe.com/reggie/v1/fixturerequestor/regcode",
];

const ADOBE_END: [&str; 4] = [
    "https://sp.auth.adobe.com/adobe-services/session",
    "https://sp.auth.adobe.com/adobe-services/authorize",
    "https://sp.auth.adobe.com/adobe-services/shortAuthorize",
    // The short media token (`fixture+media/token==`) as `tveToken`.
    "https://edge.api.brightcove.com/playback/v1/accounts/7777777777001/videos/6400000000001?tveToken=fixture%2Bmedia%2Ftoken%3D%3D",
];

fn saml(mso: &str) -> String {
    format!(
        "https://sp.auth.adobe.com/adobe-services/authenticate/saml?noflash=true&mso_id={mso}&requestor_id=fixturerequestor&no_iframe=false&domain_name=adobe.com&redirect_url=https%3A%2F%2Fwww.tvsite.example%2Fwatch%2Ffixture&reg_code=FIXTURECODE"
    )
}

fn sequence(mso: &str, login: &[&str]) -> Vec<String> {
    ADOBE_START
        .iter()
        .map(|url| (*url).to_owned())
        .chain([saml(mso)])
        .chain(login.iter().map(|url| (*url).to_owned()))
        .chain(ADOBE_END.iter().map(|url| (*url).to_owned()))
        .collect()
}

#[test]
fn generic_provider_login_follows_the_meta_refresh_and_forms() {
    assert_eq!(
        requests("RCN"),
        sequence(
            "RCN",
            &[
                // `unescapeHTML` of the meta refresh URL.
                "https://idp.rcn.example/sso/start?a=1&b=2",
                // The relative form action, joined with the page URL.
                "https://idp.rcn.example/sso/saml",
                "https://idp.rcn.example/sso/login",
                "https://sp.auth.adobe.com/sp/saml/SAMLAssertionConsumer?rcn",
            ]
        )
    );
}

#[test]
fn cablevision_login_is_upgraded_to_https() {
    // The login form's `http://` action is posted over https (CONNECT).
    assert_eq!(
        requests("Cablevision"),
        sequence(
            "Cablevision",
            &[
                "https://idp.optimum.example/saml",
                "https://idp.optimum.example/login",
                "https://sp.auth.adobe.com/sp/saml/SAMLAssertionConsumer?cablevision",
            ]
        )
    );
}

#[test]
fn special_cased_providers_follow_their_flows() {
    assert_eq!(
        requests("Comcast_SSO"),
        sequence(
            "Comcast_SSO",
            &[
                "https://login.xfinity.com/login",
                "https://oauth.xfinity.com/resume"
            ]
        )
    );
    assert_eq!(
        requests("Spectrum"),
        sequence(
            "Spectrum",
            &[
                "https://idp.spectrum.example/saml",
                "https://tveauthn.spectrum.net/tveauthentication/api/v1/manualAuth",
                "https://sp.auth.adobe.com/sp/saml/SAMLAssertionConsumer?spectrum",
            ]
        )
    );
    assert_eq!(
        requests("Verizon"),
        sequence(
            "Verizon",
            &[
                "https://ssoauth.verizon.com/login",
                "https://ssoauth.verizon.com/saml/response",
                "https://sp.auth.adobe.com/sp/saml/SAMLAssertionConsumer?verizon",
            ]
        )
    );
    assert_eq!(
        requests("slingtv"),
        sequence(
            "slingtv",
            &[
                "https://identity.sling.com/bookend?start=1",
                "https://identity.sling.com/bookend?start=1&flow=first&history=1",
                "https://identity.sling.com/login",
                "https://identity.sling.com/associate?step=2",
                "https://identity.sling.com/associate?step=2&flow=last&history=3",
                "https://sp.auth.adobe.com/sp/saml/SAMLAssertionConsumer?sling",
            ]
        )
    );
}
