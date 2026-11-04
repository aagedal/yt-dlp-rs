//! The `impersonate` feature's transport (upstream `CurlCFFIRH`'s role):
//! `wreq` on BoringSSL with `wreq-util` browser emulations, driven by a
//! private Tokio runtime behind the blocking request API the extractors
//! use. Responses are buffered and handed back as blocking `reqwest`
//! responses, so status checks and body reads stay unchanged.
use super::{ImpersonateTarget, STD_HEADERS, Supported};
use crate::{CookieJar, Error, HttpOptions, Result};
use std::sync::{Arc, LazyLock};
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
    /// Without `--proxy`, upstream's environment routes, chosen per request.
    environment: Option<crate::http::EnvironmentProxies>,
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

/// A client's transports.
#[derive(Default)]
pub(super) struct Transports {
    transport: Option<Arc<Transport>>,
    /// The transport through `--geo-verification-proxy` (see
    /// [`super::geo_scope`]).
    geo: Option<Arc<Transport>>,
}

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

/// Build a client's transports from its options (upstream's handler
/// `proxies`, `verify`, `source_address`, `timeout` and cookie jar).
pub(super) fn configure(options: &HttpOptions, cookies: &Arc<CookieJar>) -> Result<Transports> {
    let make = |transport: crate::http::TransportConfig| -> Result<Option<Arc<Transport>>> {
        let client = client(&transport, cookies)?;
        let environment = match transport.proxy {
            crate::http::ProxyConfig::Environment => {
                Some(crate::http::EnvironmentProxies::from_env()?)
            }
            _ => None,
        };
        Ok((options.client_certificate.is_none()).then(|| {
            Arc::new(Transport {
                client,
                headers: options.header_map().unwrap_or_default(),
                environment,
            })
        }))
    };
    Ok(Transports {
        transport: make(options.transport_config()?)?,
        geo: options
            .geo_transport_config()?
            .map(make)
            .transpose()?
            .flatten(),
    })
}

/// A `wreq` client for one transport configuration.
fn client(
    transport: &crate::http::TransportConfig,
    cookies: &Arc<CookieJar>,
) -> Result<wreq::Client> {
    // `CurlCFFIRH` uses the timeout for the connect and as a low-speed
    // limit (1 byte/s), so a slow but moving body is not cut off.
    let timeout = transport.socket_timeout();
    let mut builder = wreq::Client::builder()
        .cookie_provider(SharedCookies(cookies.clone()))
        // `CurlCFFIRH` follows at most 5 redirects.
        .redirect(wreq::redirect::Policy::limited(5))
        .referer(false)
        .connect_timeout(timeout)
        .read_timeout(timeout)
        .tls_cert_verification(!transport.insecure);
    if let Some(address) = transport.source_address {
        builder = builder.local_address(address);
    }
    match &transport.proxy {
        // Environment routes are set on each request (see `send`).
        crate::http::ProxyConfig::Environment | crate::http::ProxyConfig::Direct => {
            builder = builder.no_proxy();
        }
        crate::http::ProxyConfig::Explicit { url, .. } => {
            let proxy = wreq::Proxy::all(url.as_str())
                .map_err(|_| Error::HttpConfiguration("invalid or unsupported proxy URL".into()))?;
            builder = builder.no_proxy().proxy(proxy);
        }
    }
    builder.build().map_err(error)
}

/// `_get_impersonate_headers`: the global headers merged with the
/// request's, without values equal to upstream's `std_headers` (nor this
/// port's own `Accept-Encoding` defaults), so the target's browser
/// headers apply.
fn impersonate_headers(
    global: &reqwest::header::HeaderMap,
    request: &reqwest::header::HeaderMap,
    media: bool,
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
    // HttpFD's own `Accept-Encoding: identity` is not a `std_headers`
    // value, so upstream keeps it on media transfers.
    if !media
        && merged
            .get(reqwest::header::ACCEPT_ENCODING)
            .is_some_and(|value| value == "identity" || value == "gzip, deflate")
    {
        merged.remove(reqwest::header::ACCEPT_ENCODING);
    }
    merged
}

/// The transport of the client whose call is running on this thread:
/// inside a geo scope, the `--geo-verification-proxy` one, if any.
fn current() -> Result<Arc<Transport>> {
    let settings = crate::control::settings()
        .ok_or_else(|| error("impersonated requests need a YoutubeDl call"))?;
    let transports = &settings.impersonate.transports;
    let geo = super::geo_active()
        .then(|| transports.geo.clone())
        .flatten();
    geo.or_else(|| transports.transport.clone())
        .ok_or_else(|| error("impersonation with client certificates is not supported natively"))
}

/// `request` as a `wreq` request impersonating `target` (with HttpFD's
/// `Accept-Encoding` kept for `media`).
fn prepare(
    request: reqwest::blocking::RequestBuilder,
    target: &Supported,
    media: bool,
) -> Result<wreq::RequestBuilder> {
    let transport = current()?;
    let request = request.build().map_err(Error::http)?;
    let headers = impersonate_headers(&transport.headers, request.headers(), media);
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
    if let Some(target) = transport
        .environment
        .as_ref()
        .and_then(|environment| environment.target(request.url()))
    {
        let proxy = wreq::Proxy::all(target.as_str())
            .map_err(|_| Error::HttpConfiguration("invalid or unsupported proxy URL".into()))?;
        outgoing = outgoing.proxy(proxy);
    }
    if let Some(body) = body {
        outgoing = outgoing.body(body);
    }
    Ok(outgoing)
}

/// Send a media request impersonating `target`. Unlike [`send`], the body
/// streams through [`crate::http::interruptible::stream`]'s bounded queue,
/// so a transfer is never held in memory and a stop, cancel or signal ends
/// the connect, the header wait or any body read within its poll.
pub(super) fn send_media(
    request: reqwest::blocking::RequestBuilder,
    target: &Supported,
) -> Result<reqwest::blocking::Response> {
    let outgoing = prepare(request, target, true)?;
    let runtime = RUNTIME.as_ref().map_err(error)?;
    let transport = |error: wreq::Error| Error::MediaTransport(error.to_string());
    let opened = crate::http::interruptible::stream(move || {
        let response = runtime.block_on(outgoing.send()).map_err(transport)?;
        let status = reqwest::StatusCode::from_u16(response.status().as_u16()).map_err(error)?;
        Ok(crate::http::interruptible::Streamed {
            status,
            version: response.version(),
            headers: response.headers().clone(),
            url: url::Url::parse(&response.uri().to_string())?,
            content_length: response.content_length(),
            body: Box::new(MediaBody {
                runtime,
                response: Box::pin(response),
                chunk: bytes::Bytes::new(),
            }),
        })
    });
    match opened {
        Ok(response) => response,
        Err(crate::http::interruptible::Interrupted) => Err(Error::DownloadCancelled),
    }
}

/// A `wreq` response body as a blocking reader (on the queue's helper
/// thread); the client's read timeout bounds each wait.
struct MediaBody {
    runtime: &'static tokio::runtime::Runtime,
    response: std::pin::Pin<Box<wreq::Response>>,
    chunk: bytes::Bytes,
}

impl std::io::Read for MediaBody {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        use http_body::Body as _;
        while self.chunk.is_empty() {
            let response = &mut self.response;
            let frame = self.runtime.block_on(std::future::poll_fn(|context| {
                response.as_mut().poll_frame(context)
            }));
            match frame {
                None => return Ok(0),
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        self.chunk = data;
                    }
                }
                Some(Err(error)) => return Err(std::io::Error::other(error)),
            }
        }
        let count = buffer.len().min(self.chunk.len());
        buffer[..count].copy_from_slice(&self.chunk.split_to(count));
        Ok(count)
    }
}

/// Send `request` impersonating `target`.
pub(super) fn send(
    request: reqwest::blocking::RequestBuilder,
    target: &Supported,
) -> Result<reqwest::blocking::Response> {
    let outgoing = prepare(request, target, false)?;
    let runtime = RUNTIME.as_ref().map_err(error)?;
    let exchange = async move {
        let response = outgoing.send().await?;
        let status = response.status();
        let version = response.version();
        let headers = response.headers().clone();
        let url = response.uri().to_string();
        let body = response.bytes().await?;
        Ok::<_, wreq::Error>((status, version, headers, url, body))
    };
    // A stop, cancel or signal drops the exchange (closing its connection)
    // within `POLL`; the socket timeouts still bound a healthy one.
    let watch = crate::control::watch();
    let exchange = runtime.block_on(async move {
        let mut exchange = std::pin::pin!(exchange);
        let Some(watch) = watch else {
            return Some(exchange.await);
        };
        loop {
            match tokio::time::timeout(crate::http::interruptible::POLL, exchange.as_mut()).await {
                Ok(result) => return Some(result),
                Err(_) if watch.requested() => return None,
                Err(_) => {}
            }
        }
    });
    let (status, version, headers, url, body) = exchange
        .ok_or(crate::Error::ExtractionCancelled)?
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
        let merged = impersonate_headers(&global, &request, false);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged["x-global"], "request");
        // A user agent other than upstream's random one is kept.
        global.insert("user-agent", HeaderValue::from_static("custom"));
        assert_eq!(
            impersonate_headers(&global, &request, false)["user-agent"],
            "custom"
        );
    }

    #[test]
    fn media_requests_keep_identity_encoding() {
        use reqwest::header::{HeaderMap, HeaderValue};
        let mut request = HeaderMap::new();
        request.insert("accept-encoding", HeaderValue::from_static("identity"));
        assert_eq!(
            impersonate_headers(&HeaderMap::new(), &request, true)["accept-encoding"],
            "identity"
        );
    }

    fn set_std(headers: Vec<(String, String)>) {
        super::super::set_std_headers(headers);
    }
}
