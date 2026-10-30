//! The `impersonate` feature's transport (upstream `CurlCFFIRH`'s role):
//! `wreq` on BoringSSL with `wreq-util` browser emulations, driven by a
//! private Tokio runtime behind the blocking request API the extractors
//! use. Responses are buffered and handed back as blocking `reqwest`
//! responses, so status checks and body reads stay unchanged.
use super::{ImpersonateTarget, STD_HEADERS, Supported};
use crate::{CookieJar, Error, HttpOptions, Result};
use reqwest::cookie::CookieStore as _;
use std::sync::{Arc, LazyLock, RwLock};
use wreq_util::{Emulation, Platform, Profile as Browser};

/// A `wreq-util` browser profile on a platform.
#[derive(Clone, Copy)]
pub(crate) struct Profile {
    browser: Browser,
    platform: Platform,
}

#[cfg(test)]
impl Profile {
    pub(crate) fn placeholder() -> Self {
        Self {
            browser: Browser::Chrome149,
            platform: Platform::Windows,
        }
    }
}

/// The supported targets: recent `wreq-util` Chrome, Edge, Firefox and
/// Safari profiles on the platforms whose headers they emulate. `wreq-util`
/// models no OS versions beyond the user agent's (`Windows NT 10.0`, an iOS
/// version), so other OS versions are left open.
pub(super) fn targets() -> Vec<Supported> {
    use Browser::*;
    let desktop = [
        ("windows", Some("10"), Platform::Windows),
        ("macos", None, Platform::MacOS),
        ("linux", None, Platform::Linux),
    ];
    let mut targets = Vec::new();
    let mut push =
        |client: &str, version: &str, os: &str, os_version: Option<&str>, browser, platform| {
            targets.push(Supported {
                target: ImpersonateTarget {
                    client: Some(client.into()),
                    version: Some(version.into()),
                    os: Some(os.into()),
                    os_version: os_version.map(String::from),
                },
                emulation: Profile { browser, platform },
            });
        };
    for (version, browser) in [
        ("149", Chrome149),
        ("148", Chrome148),
        ("147", Chrome147),
        ("146", Chrome146),
        ("145", Chrome145),
    ] {
        for (os, os_version, platform) in desktop {
            push("chrome", version, os, os_version, browser, platform);
        }
        push(
            "chrome",
            version,
            "android",
            None,
            browser,
            Platform::Android,
        );
    }
    for (version, browser) in [("148", Edge148), ("147", Edge147), ("146", Edge146)] {
        for (os, os_version, platform) in &desktop[..2] {
            push("edge", version, os, *os_version, browser, *platform);
        }
    }
    for (version, browser) in [
        ("151", Firefox151),
        ("150", Firefox150),
        ("149", Firefox149),
    ] {
        for (os, os_version, platform) in desktop {
            push("firefox", version, os, os_version, browser, platform);
        }
    }
    for (version, browser) in [
        ("26.4", Safari26_4),
        ("26.3", Safari26_3),
        ("26.2", Safari26_2),
        ("26.1", Safari26_1),
        ("26.0", Safari26),
        ("18.5", Safari18_5),
    ] {
        push("safari", version, "macos", None, browser, Platform::MacOS);
    }
    // The iOS profiles' user agents name `iPhone OS 18_7` (26.2), `26_0`
    // and `18_1_1`.
    for (version, os_version, browser) in [
        ("26.2", "18.7", SafariIos26_2),
        ("26.0", "26.0", SafariIos26),
        ("18.1.1", "18.1.1", SafariIos18_1_1),
    ] {
        push(
            "safari",
            version,
            "ios",
            Some(os_version),
            browser,
            Platform::IOS,
        );
    }
    targets
}

struct Transport {
    client: wreq::Client,
    headers: reqwest::header::HeaderMap,
}

static RUNTIME: LazyLock<std::result::Result<tokio::runtime::Runtime, String>> =
    LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("yt-dlp-rs-impersonate")
            .enable_all()
            .build()
            .map_err(|error| error.to_string())
    });

static TRANSPORT: RwLock<Option<Arc<Transport>>> = RwLock::new(None);

fn error(message: impl std::fmt::Display) -> Error {
    Error::Impersonate(message.to_string())
}

/// `CookieJar` (this port's `YoutubeDL.cookiejar`) for `wreq`, so
/// impersonated requests share the session's cookies.
struct SharedCookies(Arc<CookieJar>);

impl wreq::cookie::CookieStore for SharedCookies {
    fn set_cookies(
        &self,
        cookie_headers: &mut dyn Iterator<Item = &wreq::header::HeaderValue>,
        uri: &wreq::Uri,
    ) {
        if let Ok(url) = url::Url::parse(&uri.to_string()) {
            self.0.set_cookies(cookie_headers, &url);
        }
    }

    fn cookies(&self, uri: &wreq::Uri, _version: wreq::Version) -> wreq::cookie::Cookies {
        url::Url::parse(&uri.to_string())
            .ok()
            .and_then(|url| self.0.cookies(&url))
            .map_or(
                wreq::cookie::Cookies::Empty,
                wreq::cookie::Cookies::Compressed,
            )
    }
}

/// Build the shared client from the global options (upstream's handler
/// `proxies`, `verify`, `source_address`, `timeout` and cookie jar).
pub(super) fn configure(options: &HttpOptions, cookies: &Arc<CookieJar>) -> Result<()> {
    let transport = options.transport_config()?;
    let (connect, operation) = match transport.timeout {
        Some(timeout) => (timeout, timeout),
        None => (
            std::time::Duration::from_secs(15),
            std::time::Duration::from_secs(300),
        ),
    };
    let mut builder = wreq::Client::builder()
        .cookie_provider(SharedCookies(cookies.clone()))
        // `CurlCFFIRH` follows at most 5 redirects.
        .redirect(wreq::redirect::Policy::limited(5))
        .referer(false)
        .connect_timeout(connect)
        .timeout(operation)
        .tls_cert_verification(!transport.insecure);
    if let Some(address) = transport.source_address {
        builder = builder.local_address(address);
    }
    match &transport.proxy {
        crate::http::ProxyConfig::Environment => {}
        crate::http::ProxyConfig::Direct => builder = builder.no_proxy(),
        crate::http::ProxyConfig::Explicit { url, .. } => {
            let proxy = wreq::Proxy::all(url.as_str())
                .map_err(|_| Error::HttpConfiguration("invalid or unsupported proxy URL".into()))?;
            builder = builder.no_proxy().proxy(proxy);
        }
    }
    let client = builder.build().map_err(error)?;
    let state = (options.client_certificate.is_none()).then(|| {
        Arc::new(Transport {
            client,
            headers: options.header_map().unwrap_or_default(),
        })
    });
    if let Ok(mut current) = TRANSPORT.write() {
        *current = state;
    }
    Ok(())
}

/// `_get_impersonate_headers`: the global headers merged with the
/// request's, without values equal to upstream's `std_headers` (nor this
/// port's own `Accept-Encoding` defaults), so the target's browser
/// headers apply.
fn impersonate_headers(
    global: &reqwest::header::HeaderMap,
    request: &reqwest::header::HeaderMap,
) -> reqwest::header::HeaderMap {
    let mut merged = global.clone();
    for name in request.keys() {
        merged.remove(name);
        for value in request.get_all(name) {
            merged.append(name.clone(), value.clone());
        }
    }
    let std = STD_HEADERS
        .read()
        .map(|std| std.clone())
        .unwrap_or_default();
    for (name, value) in std {
        if merged
            .get(name.as_str())
            .is_some_and(|current| current.as_bytes() == value.as_bytes())
        {
            merged.remove(name.as_str());
        }
    }
    if merged
        .get(reqwest::header::ACCEPT_ENCODING)
        .is_some_and(|value| value == "identity" || value == "gzip, deflate")
    {
        merged.remove(reqwest::header::ACCEPT_ENCODING);
    }
    merged
}

/// Send `request` impersonating `target`.
pub(super) fn send(
    request: reqwest::blocking::RequestBuilder,
    target: &Supported,
) -> Result<reqwest::blocking::Response> {
    let transport = TRANSPORT
        .read()
        .ok()
        .and_then(|transport| transport.clone())
        .ok_or_else(|| error("impersonation with client certificates is not supported natively"))?;
    let runtime = RUNTIME.as_ref().map_err(error)?;
    let request = request.build()?;
    let headers = impersonate_headers(&transport.headers, request.headers());
    let body = request
        .body()
        .and_then(|body| body.as_bytes())
        .map(<[u8]>::to_vec);
    let emulation = Emulation::builder()
        .profile(target.emulation.browser)
        .platform(target.emulation.platform)
        .build();
    let mut outgoing = transport
        .client
        .request(request.method().clone(), request.url().as_str())
        .emulation(emulation)
        .headers(headers);
    if let Some(body) = body {
        outgoing = outgoing.body(body);
    }
    let (status, version, headers, url, body) = runtime
        .block_on(async move {
            let response = outgoing.send().await?;
            let status = response.status();
            let version = response.version();
            let headers = response.headers().clone();
            let url = response.uri().to_string();
            let body = response.bytes().await?;
            Ok::<_, wreq::Error>((status, version, headers, url, body))
        })
        .map_err(error)?;
    let url = url::Url::parse(&url)?;
    let mut builder = http::Response::builder()
        .status(status.as_u16())
        .version(version);
    if let Some(map) = builder.headers_mut() {
        *map = headers;
    }
    let response = reqwest::ResponseBuilderExt::url(builder, url)
        .body(body.to_vec())
        .map_err(error)?;
    Ok(reqwest::blocking::Response::from(response))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn std_header_values_and_identity_encoding_are_dropped() {
        use reqwest::header::{HeaderMap, HeaderValue};
        set_std(vec![
            ("User-Agent".into(), "Mozilla/5.0 std".into()),
            ("Accept-Language".into(), "en-us,en;q=0.5".into()),
        ]);
        let mut global = HeaderMap::new();
        global.insert("user-agent", HeaderValue::from_static("Mozilla/5.0 std"));
        global.insert(
            "accept-language",
            HeaderValue::from_static("en-us,en;q=0.5"),
        );
        global.insert("x-global", HeaderValue::from_static("kept"));
        let mut request = HeaderMap::new();
        request.insert("accept-encoding", HeaderValue::from_static("identity"));
        request.insert("x-global", HeaderValue::from_static("request"));
        let merged = impersonate_headers(&global, &request);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged["x-global"], "request");
        // A user agent other than upstream's random one is kept.
        global.insert("user-agent", HeaderValue::from_static("custom"));
        assert_eq!(
            impersonate_headers(&global, &request)["user-agent"],
            "custom"
        );
    }

    fn set_std(headers: Vec<(String, String)>) {
        super::super::set_std_headers(headers);
    }
}
