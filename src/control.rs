//! Library call context: [`Interrupt`] (stop or cancel an extraction or a
//! download from another thread) and [`RequestScope`] (headers and cookies
//! that belong to one request origin, such as a manifest's authorization).
//! Both are installed for the duration of one library call on the calling
//! thread and checked before every request the call makes.
use crate::{CookieJar, Error, Result};
use reqwest::header::{HeaderName, HeaderValue};
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};
use url::Url;

const RUNNING: u8 = 0;
const STOP: u8 = 1;
const CANCEL: u8 = 2;

/// A stop or cancel request for library calls, shared with another thread
/// (for example a UI's Stop and Cancel buttons).
///
/// - [`Interrupt::cancel`] aborts: a download removes its private temporary
///   files and returns [`Error::DownloadCancelled`].
/// - [`Interrupt::stop`] ends early but keeps what was downloaded: a
///   download publishes the data it has at the destination (whole
///   fragments only) and returns [`Error::DownloadStopped`] with its path.
///
/// Either one ends an extraction with [`Error::ExtractionCancelled`] before
/// its next request. Requests already in flight finish or time out first.
#[derive(Debug, Clone, Default)]
pub struct Interrupt {
    state: Arc<AtomicU8>,
}

impl Interrupt {
    pub fn new() -> Self {
        Self::default()
    }

    /// Stop and keep the partial download.
    pub fn stop(&self) {
        let _ = self
            .state
            .compare_exchange(RUNNING, STOP, Ordering::SeqCst, Ordering::SeqCst);
    }

    /// Abort and discard the partial download (overrides an earlier stop).
    pub fn cancel(&self) {
        self.state.store(CANCEL, Ordering::SeqCst);
    }

    /// Whether [`stop`](Self::stop) or [`cancel`](Self::cancel) was called.
    pub fn is_requested(&self) -> bool {
        self.state.load(Ordering::SeqCst) != RUNNING
    }

    /// Whether the request is a stop that keeps partial data.
    pub fn is_stop(&self) -> bool {
        self.state.load(Ordering::SeqCst) == STOP
    }
}

/// Headers and cookies for requests to one origin (scheme, host and port).
///
/// Headers are sent only on requests whose URL has the scope's origin, so a
/// manifest's authorization does not reach segment or key hosts elsewhere.
/// Cookies follow RFC 6265 domain and path matching (host-only on the
/// scope's host unless a `Domain` attribute says otherwise); they are added
/// to the client's jar only while the call runs and are never saved.
/// Values are redacted from `Debug`.
#[derive(Clone)]
pub struct RequestScope {
    url: Url,
    headers: Vec<(HeaderName, HeaderValue)>,
    cookies: Vec<String>,
}

impl std::fmt::Debug for RequestScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let names: Vec<&str> = self.headers.iter().map(|(name, _)| name.as_str()).collect();
        f.debug_struct("RequestScope")
            .field("origin", &self.url.origin().ascii_serialization())
            .field("headers", &names)
            .field("cookies", &self.cookies.len())
            .finish()
    }
}

impl RequestScope {
    /// A scope for `url`'s origin (HTTP or HTTPS).
    pub fn new(url: &Url) -> Result<Self> {
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err(Error::HttpConfiguration(
                "a request scope needs an HTTP(S) URL with a host".into(),
            ));
        }
        Ok(Self {
            url: url.clone(),
            headers: Vec::new(),
            cookies: Vec::new(),
        })
    }

    /// Send `name: value` on requests to the scope's origin, replacing a
    /// header of the same name. The value is marked sensitive.
    pub fn header(mut self, name: &str, value: &str) -> Result<Self> {
        let name = HeaderName::from_bytes(name.trim().as_bytes())
            .map_err(|_| Error::HttpConfiguration(format!("invalid header name {name:?}")))?;
        if name == reqwest::header::COOKIE {
            return Err(Error::HttpConfiguration(
                "pass scoped cookies with RequestScope::cookie, not a Cookie header".into(),
            ));
        }
        let mut value = HeaderValue::from_str(value.trim())
            .map_err(|_| Error::HttpConfiguration(format!("invalid value for header {name}")))?;
        value.set_sensitive(true);
        self.headers.retain(|(existing, _)| *existing != name);
        self.headers.push((name, value));
        Ok(self)
    }

    /// A session cookie for the scope's host only, path `/`.
    pub fn cookie(self, name: &str, value: &str) -> Result<Self> {
        self.set_cookie(&format!("{name}={value}; Path=/"))
    }

    /// A cookie in `Set-Cookie` syntax, as if the scope's URL had sent it
    /// (a `Domain` attribute must cover the scope's host).
    pub fn set_cookie(mut self, header: &str) -> Result<Self> {
        crate::cookies::parse_set_cookie(header, &self.url)?;
        self.cookies.push(header.to_owned());
        Ok(self)
    }

    /// The URL the scope was created for.
    pub fn url(&self) -> &Url {
        &self.url
    }
}

/// Library download controls (`YoutubeDl::download_with_control`).
#[derive(Debug, Clone, Default)]
pub struct DownloadControl {
    /// Stop (keep the partial file) or cancel from another thread.
    pub interrupt: Option<Interrupt>,
    /// Headers and cookies for requests to one origin.
    pub scope: Option<RequestScope>,
}

/// What one library call installed on its thread.
#[derive(Default)]
pub(crate) struct Context {
    interrupt: Option<Interrupt>,
    scope: Option<Arc<RequestScope>>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Arc<Context>>> = const { RefCell::new(None) };
}

static NEXT_SCOPE: AtomicU64 = AtomicU64::new(1);

/// Restores the previous context (and removes scoped cookies) when dropped.
pub(crate) struct Guard {
    previous: Option<Arc<Context>>,
    cookies: Option<(Arc<CookieJar>, u64)>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some((jar, id)) = self.cookies.take() {
            jar.remove_scoped(id);
        }
        let previous = self.previous.take();
        ACTIVE.with(|active| *active.borrow_mut() = previous);
    }
}

/// Install `interrupt` and `scope` for one library call. Without either,
/// the caller's context (if any) stays in effect.
pub(crate) fn enter(
    interrupt: Option<&Interrupt>,
    scope: Option<&RequestScope>,
    jar: &Arc<CookieJar>,
) -> Result<Guard> {
    let previous = current();
    if interrupt.is_none() && scope.is_none() {
        return Ok(Guard {
            previous: previous.clone(),
            cookies: None,
        });
    }
    let mut cookies = None;
    if let Some(scope) = scope.filter(|scope| !scope.cookies.is_empty()) {
        let id = NEXT_SCOPE.fetch_add(1, Ordering::Relaxed);
        jar.push_scoped(id, &scope.url, &scope.cookies)?;
        cookies = Some((jar.clone(), id));
    }
    let context = Context {
        interrupt: interrupt
            .cloned()
            .or_else(|| previous.as_ref().and_then(|p| p.interrupt.clone())),
        scope: scope
            .map(|scope| Arc::new(scope.clone()))
            .or_else(|| previous.as_ref().and_then(|p| p.scope.clone())),
    };
    ACTIVE.with(|active| *active.borrow_mut() = Some(Arc::new(context)));
    Ok(Guard { previous, cookies })
}

/// This thread's context, to [`resume`] on a worker thread.
pub(crate) fn current() -> Option<Arc<Context>> {
    ACTIVE.with(|active| active.borrow().clone())
}

/// Install a context taken with [`current`] on a worker thread.
pub(crate) fn resume(context: Option<Arc<Context>>) -> Guard {
    let previous = current();
    ACTIVE.with(|active| *active.borrow_mut() = context);
    Guard {
        previous,
        cookies: None,
    }
}

/// Whether the active [`Interrupt`] asks to stop or cancel.
pub(crate) fn requested() -> bool {
    ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .and_then(|context| context.interrupt.as_ref())
            .is_some_and(Interrupt::is_requested)
    })
}

/// Whether the active [`Interrupt`] is a stop that keeps partial data.
pub(crate) fn stopping() -> bool {
    ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .and_then(|context| context.interrupt.as_ref())
            .is_some_and(Interrupt::is_stop)
    })
}

/// An extraction checkpoint: `Err` once an interrupt was requested.
pub(crate) fn check() -> Result<()> {
    if requested() {
        Err(Error::ExtractionCancelled)
    } else {
        Ok(())
    }
}

/// Add the active scope's headers to `request` when its URL has the
/// scope's origin.
pub(crate) fn apply(request: &mut reqwest::blocking::Request) {
    ACTIVE.with(|active| {
        let active = active.borrow();
        let Some(scope) = active.as_ref().and_then(|context| context.scope.as_ref()) else {
            return;
        };
        if request.url().origin() != scope.url.origin() {
            return;
        }
        for (name, value) in &scope.headers {
            request.headers_mut().insert(name.clone(), value.clone());
        }
    });
}

/// An extraction request about to be sent: `Err` once an interrupt was
/// requested, otherwise the builder with the active scope's headers.
pub(crate) fn prepare(
    request: reqwest::blocking::RequestBuilder,
) -> Result<reqwest::blocking::RequestBuilder> {
    check()?;
    Ok(scoped(request)?)
}

/// `request` with the active scope's headers (downloads check interrupts
/// at their progress checkpoints instead).
pub(crate) fn scoped(
    request: reqwest::blocking::RequestBuilder,
) -> reqwest::Result<reqwest::blocking::RequestBuilder> {
    let has_scope = ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .is_some_and(|context| context.scope.is_some())
    });
    if !has_scope {
        return Ok(request);
    }
    let (client, built) = request.build_split();
    let mut built = built?;
    apply(&mut built);
    Ok(reqwest::blocking::RequestBuilder::from_parts(client, built))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_does_not_override_cancel() {
        let interrupt = Interrupt::new();
        assert!(!interrupt.is_requested());
        interrupt.cancel();
        interrupt.stop();
        assert!(interrupt.is_requested() && !interrupt.is_stop());
        let interrupt = Interrupt::new();
        interrupt.stop();
        assert!(interrupt.is_stop());
        interrupt.cancel();
        assert!(!interrupt.is_stop());
    }

    #[test]
    fn scoped_headers_reach_only_their_origin() {
        let jar = Arc::new(CookieJar::default());
        let origin = Url::parse("https://media.example/master.m3u8").unwrap();
        let scope = RequestScope::new(&origin)
            .unwrap()
            .header("Authorization", "Bearer secret")
            .unwrap();
        let client = reqwest::blocking::Client::new();
        let headers = |url: &str| {
            let _guard = enter(None, Some(&scope), &jar).unwrap();
            prepare(client.get(url))
                .unwrap()
                .build()
                .unwrap()
                .headers()
                .get("authorization")
                .cloned()
        };
        assert!(headers("https://media.example/seg1.ts").is_some());
        assert!(headers("https://cdn.example/seg1.ts").is_none());
        assert!(headers("http://media.example/seg1.ts").is_none());
        assert!(headers("https://media.example:8443/seg1.ts").is_none());
        // Outside the call nothing is added.
        let request = prepare(client.get("https://media.example/x")).unwrap();
        assert!(
            request
                .build()
                .unwrap()
                .headers()
                .get("authorization")
                .is_none()
        );
    }

    #[test]
    fn an_interrupt_fails_the_next_request() {
        let jar = Arc::new(CookieJar::default());
        let interrupt = Interrupt::new();
        let _guard = enter(Some(&interrupt), None, &jar).unwrap();
        let client = reqwest::blocking::Client::new();
        assert!(prepare(client.get("https://example.test/")).is_ok());
        interrupt.stop();
        assert!(matches!(
            prepare(client.get("https://example.test/")),
            Err(Error::ExtractionCancelled)
        ));
    }

    #[test]
    fn scoped_cookies_match_like_stored_ones_and_are_removed_after_the_call() {
        use reqwest::cookie::CookieStore as _;
        let jar = Arc::new(CookieJar::default());
        let url = |text: &str| Url::parse(text).unwrap();
        let header = |target: &str| {
            jar.cookies(&url(target))
                .map(|value| value.to_str().unwrap().to_owned())
        };
        jar.set_cookie(
            &url("https://www.example.test/"),
            "stored=1; Domain=example.test",
        )
        .unwrap();
        // A Domain must cover the URL's host.
        assert!(
            jar.set_cookie(&url("https://www.example.test/"), "x=1; Domain=other.test")
                .is_err()
        );
        let scope = RequestScope::new(&url("https://media.example.test/a.m3u8"))
            .unwrap()
            .cookie("session", "s")
            .unwrap()
            .set_cookie("stored=2; Domain=example.test; Path=/video")
            .unwrap();
        {
            let _guard = enter(None, Some(&scope), &jar).unwrap();
            assert_eq!(
                header("https://media.example.test/a.ts").as_deref(),
                Some("stored=1; session=s")
            );
            // The scope's cookie replaces the stored one of the same name.
            assert_eq!(
                header("https://cdn.example.test/video/a.ts").as_deref(),
                Some("stored=2")
            );
            // `session` is host-only.
            assert_eq!(
                header("https://cdn.example.test/a.ts").as_deref(),
                Some("stored=1")
            );
        }
        assert_eq!(
            header("https://media.example.test/a.ts").as_deref(),
            Some("stored=1")
        );
    }

    #[test]
    fn debug_redacts_values() {
        let url = Url::parse("https://example.test/").unwrap();
        let scope = RequestScope::new(&url)
            .unwrap()
            .header("X-Token", "secret")
            .unwrap()
            .cookie("session", "secret")
            .unwrap();
        let debug = format!("{scope:?}");
        assert!(!debug.contains("secret"), "{debug}");
        assert!(debug.contains("x-token"), "{debug}");
    }
}
