//! Library call context: [`Interrupt`] (stop or cancel an extraction or a
//! download from another thread), [`RequestScope`] (headers and cookies
//! that belong to one request origin, such as a manifest's authorization)
//! and the calling client's [`ClientSettings`]. They are installed for the
//! duration of one library call on the calling thread and checked before
//! every request the call makes.
use crate::{CookieJar, Error, Result};
use reqwest::header::{HeaderName, HeaderValue};
use std::{
    cell::RefCell,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
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
/// Either one ends an extraction with [`Error::ExtractionCancelled`].
/// Requests in flight (connecting, waiting for headers or reading the body)
/// and waits between requests end within about 100 ms of the call.
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
#[non_exhaustive]
pub struct DownloadControl {
    /// Stop (keep the partial file) or cancel from another thread.
    pub interrupt: Option<Interrupt>,
    /// Headers and cookies for requests to one origin.
    pub scope: Option<RequestScope>,
}

/// The settings of one `YoutubeDl` that code below it reads as upstream
/// reads `self.get_param(...)`, so clients with different options can run
/// side by side.
#[derive(Default)]
pub(crate) struct ClientSettings {
    /// `noplaylist`, for `_yes_playlist`.
    pub(crate) no_playlist: bool,
    /// `age_limit`, for `age_restricted`.
    pub(crate) age_limit: Option<i64>,
    /// `getcomments`, for `extract_comments`.
    pub(crate) get_comments: bool,
    /// `skip_download`.
    pub(crate) skip_download: bool,
    /// `params['http_headers']['User-Agent']`.
    pub(crate) user_agent: String,
    /// The jar extractors' `_set_cookie` and `_get_cookies` use.
    pub(crate) cookies: Option<Arc<CookieJar>>,
    /// The client for `--geo-verification-proxy` requests.
    pub(crate) geo_client: Option<reqwest::blocking::Client>,
    /// `--print-traffic`.
    pub(crate) print_traffic: bool,
    /// The impersonating transports.
    #[cfg_attr(not(feature = "impersonate"), allow(dead_code))]
    pub(crate) impersonate: crate::impersonate::Transports,
}

/// What one library call installed on its thread.
#[derive(Default)]
pub(crate) struct Context {
    interrupt: Option<Interrupt>,
    scope: Option<Arc<RequestScope>>,
    /// The scope's cookies; only this call's requests send them.
    scope_cookies: Option<Arc<ScopeCookies>>,
    settings: Option<Arc<ClientSettings>>,
}

/// A [`RequestScope`]'s parsed cookies and the jar of the client whose call
/// holds them.
pub(crate) struct ScopeCookies {
    jar: Arc<CookieJar>,
    pub(crate) cookies: Vec<cookie_store::Cookie<'static>>,
}

thread_local! {
    static ACTIVE: RefCell<Option<Arc<Context>>> = const { RefCell::new(None) };
}

/// `--keep-partial-on-stop`: nested downloads holding [`SignalStop`]; while
/// any does, SIGINT/SIGTERM count as an [`Interrupt::stop`].
static SIGNAL_STOP: std::sync::Mutex<(usize, Option<crate::interrupt::Guard>)> =
    std::sync::Mutex::new((0, None));
static SIGNAL_STOP_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Treats SIGINT/SIGTERM as a stop while held (the first holder installs
/// the signal guard, the last one removes it).
pub(crate) struct SignalStop;

impl SignalStop {
    pub(crate) fn enter() -> Self {
        if let Ok(mut state) = SIGNAL_STOP.lock() {
            if state.0 == 0 {
                state.1 = Some(crate::interrupt::Guard::install_with_terminate());
                SIGNAL_STOP_ACTIVE.store(true, Ordering::SeqCst);
            }
            state.0 += 1;
        }
        Self
    }
}

impl Drop for SignalStop {
    fn drop(&mut self) {
        if let Ok(mut state) = SIGNAL_STOP.lock() {
            state.0 = state.0.saturating_sub(1);
            if state.0 == 0 {
                SIGNAL_STOP_ACTIVE.store(false, Ordering::SeqCst);
                state.1 = None;
            }
        }
    }
}

/// Whether a signal arrived while [`SignalStop`] is held.
fn signalled() -> bool {
    SIGNAL_STOP_ACTIVE.load(Ordering::SeqCst) && crate::interrupt::interrupted()
}

static SIGNAL_INTERRUPT_ACTIVE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// The CLI's SIGINT/SIGTERM handling for a whole run, upstream's
/// `KeyboardInterrupt`: while held, a signal ends extraction and downloads
/// at their next checkpoint (a blocked connect or read within about
/// 100 ms) like a cancel, but keeps files as a failure does: upstream `.part` files
/// and completed postprocessing stages stay, private temporary files are
/// removed. A second signal exits at once. Not for library callers, which
/// use [`Interrupt`]; Windows keeps the default Ctrl+C behavior.
#[doc(hidden)]
pub struct SignalInterrupt {
    _guard: crate::interrupt::Guard,
}

impl SignalInterrupt {
    pub fn install() -> Self {
        let guard = crate::interrupt::install_run_wide();
        SIGNAL_INTERRUPT_ACTIVE.store(true, Ordering::SeqCst);
        Self { _guard: guard }
    }

    /// Run `wait`, a prompt blocked on input, with a signal ending the
    /// process at once.
    pub fn prompt<T>(wait: impl FnOnce() -> T) -> T {
        crate::interrupt::exit_at_once(wait)
    }

    /// Sleep for `duration`, or until a signal (`time.sleep` raising
    /// `KeyboardInterrupt`); `false` when a signal ended it.
    pub fn sleep(duration: std::time::Duration) -> bool {
        sleep(duration)
    }

    /// Whether a signal ended the run.
    pub fn interrupted() -> bool {
        SIGNAL_INTERRUPT_ACTIVE.load(Ordering::SeqCst)
            && crate::interrupt::interrupted()
            // A live download, wait, recording or `--keep-partial-on-stop`
            // download handles the signal itself.
            && !crate::interrupt::handled_elsewhere()
    }
}

impl Drop for SignalInterrupt {
    fn drop(&mut self) {
        SIGNAL_INTERRUPT_ACTIVE.store(false, Ordering::SeqCst);
    }
}

/// Restores the previous context when dropped.
pub(crate) struct Guard {
    previous: Option<Arc<Context>>,
}

impl Drop for Guard {
    fn drop(&mut self) {
        let previous = self.previous.take();
        ACTIVE.with(|active| *active.borrow_mut() = previous);
    }
}

/// Install a client's `settings` for one library call; the caller's
/// interrupt and scope (if any) stay in effect.
pub(crate) fn enter_client(settings: &Arc<ClientSettings>) -> Guard {
    let previous = current();
    let context = Context {
        interrupt: previous.as_ref().and_then(|p| p.interrupt.clone()),
        scope: previous.as_ref().and_then(|p| p.scope.clone()),
        scope_cookies: previous.as_ref().and_then(|p| p.scope_cookies.clone()),
        settings: Some(settings.clone()),
    };
    ACTIVE.with(|active| *active.borrow_mut() = Some(Arc::new(context)));
    Guard { previous }
}

/// Install a client's `settings`, `interrupt` and `scope` for one library
/// call. Without `interrupt` or `scope`, the caller's stays in effect.
pub(crate) fn enter(
    settings: &Arc<ClientSettings>,
    interrupt: Option<&Interrupt>,
    scope: Option<&RequestScope>,
    jar: &Arc<CookieJar>,
) -> Result<Guard> {
    if interrupt.is_none() && scope.is_none() {
        return Ok(enter_client(settings));
    }
    let previous = current();
    let scope_cookies = match scope {
        Some(scope) if !scope.cookies.is_empty() => Some(Arc::new(ScopeCookies {
            jar: jar.clone(),
            cookies: scope
                .cookies
                .iter()
                .map(|header| crate::cookies::parse_set_cookie(header, &scope.url))
                .collect::<Result<_>>()?,
        })),
        Some(_) => None,
        None => previous.as_ref().and_then(|p| p.scope_cookies.clone()),
    };
    let context = Context {
        interrupt: interrupt
            .cloned()
            .or_else(|| previous.as_ref().and_then(|p| p.interrupt.clone())),
        scope: scope
            .map(|scope| Arc::new(scope.clone()))
            .or_else(|| previous.as_ref().and_then(|p| p.scope.clone())),
        scope_cookies,
        settings: Some(settings.clone()),
    };
    ACTIVE.with(|active| *active.borrow_mut() = Some(Arc::new(context)));
    Ok(Guard { previous })
}

/// This thread's context, to [`resume`] on a worker thread.
pub(crate) fn current() -> Option<Arc<Context>> {
    ACTIVE.with(|active| active.borrow().clone())
}

/// Install a context taken with [`current`] on a worker thread.
pub(crate) fn resume(context: Option<Arc<Context>>) -> Guard {
    let previous = current();
    ACTIVE.with(|active| *active.borrow_mut() = context);
    Guard { previous }
}

/// The settings of the client whose call is running on this thread; `None`
/// outside a `YoutubeDl` call (defaults apply).
pub(crate) fn settings() -> Option<Arc<ClientSettings>> {
    ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .and_then(|context| context.settings.clone())
    })
}

/// Whether the active [`Interrupt`] asks to stop or cancel.
pub(crate) fn requested() -> bool {
    signalled()
        || SignalInterrupt::interrupted()
        || ACTIVE.with(|active| {
            active
                .borrow()
                .as_ref()
                .and_then(|context| context.interrupt.as_ref())
                .is_some_and(Interrupt::is_requested)
        })
}

/// Whether the active [`Interrupt`] is a stop that keeps partial data.
pub(crate) fn stopping() -> bool {
    signalled() || stop_requested()
}

/// Whether the active [`Interrupt`] is a stop (not a signal).
pub(crate) fn stop_requested() -> bool {
    ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .and_then(|context| context.interrupt.as_ref())
            .is_some_and(Interrupt::is_stop)
    })
}

/// What can interrupt the current call, checked from another thread while
/// a transfer or sleep waits (see [`crate::http::interruptible`]).
#[derive(Clone)]
pub(crate) struct Watch {
    interrupt: Option<Interrupt>,
}

impl Watch {
    /// The call's [`Interrupt`], or a signal any handler recorded: the
    /// run-wide one, `--keep-partial-on-stop`'s stop, or a live download's
    /// or recording's own (which then finishes instead of ending the run).
    pub(crate) fn requested(&self) -> bool {
        crate::interrupt::pending() || self.interrupt.as_ref().is_some_and(Interrupt::is_requested)
    }
}

/// The current call's interrupt sources; `None` when nothing can interrupt
/// it (no [`Interrupt`] and no signal handler), so waits need no watching.
pub(crate) fn watch() -> Option<Watch> {
    let interrupt = ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()
            .and_then(|context| context.interrupt.clone())
    });
    (interrupt.is_some() || crate::interrupt::guarded()).then_some(Watch { interrupt })
}

/// Whether the current call's [`watch`] sees an interrupt.
pub(crate) fn interrupted() -> bool {
    watch().is_some_and(|watch| watch.requested())
}

/// Sleep for `duration`, ending early (`false`) at an interrupt.
pub(crate) fn sleep(duration: std::time::Duration) -> bool {
    let Some(watch) = watch() else {
        std::thread::sleep(duration);
        return true;
    };
    let until = std::time::Instant::now() + duration;
    loop {
        if watch.requested() {
            return false;
        }
        let remaining = until.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return true;
        }
        std::thread::sleep(remaining.min(crate::http::interruptible::POLL));
    }
}

/// An extraction checkpoint: `Err` once an interrupt was requested.
pub(crate) fn check() -> Result<()> {
    if requested() {
        Err(Error::ExtractionCancelled)
    } else {
        Ok(())
    }
}

/// The active scope's cookies when they belong to `jar` (another client's
/// jar never sends them).
pub(crate) fn scope_cookies(jar: &CookieJar) -> Option<Arc<ScopeCookies>> {
    ACTIVE.with(|active| {
        active
            .borrow()
            .as_ref()?
            .scope_cookies
            .clone()
            .filter(|scoped| std::ptr::eq(&*scoped.jar, jar))
    })
}

/// Add the active scope's headers to `request` when its URL has the
/// scope's origin, and its cookies (with the jar's) when one matches the
/// URL. The cookies go in an explicit `Cookie` header, which the client's
/// jar does not replace: the jar itself is shared by every call.
pub(crate) fn apply(request: &mut reqwest::blocking::Request) {
    ACTIVE.with(|active| {
        let active = active.borrow();
        let Some(context) = active.as_ref() else {
            return;
        };
        if let Some(scope) = &context.scope
            && request.url().origin() == scope.url.origin()
        {
            for (name, value) in &scope.headers {
                request.headers_mut().insert(name.clone(), value.clone());
            }
        }
        if let Some(scoped) = &context.scope_cookies
            && !request.headers().contains_key(reqwest::header::COOKIE)
            && !crate::cookies::scope_matches(&scoped.cookies, request.url()).is_empty()
            && let Some(header) = scoped.jar.header(request.url(), &scoped.cookies)
        {
            request
                .headers_mut()
                .insert(reqwest::header::COOKIE, header);
        }
    });
}

/// A redirect hop to `request`'s new URL: the scope's headers stay with its
/// origin, and its cookies are matched again (the hop's `Cookie` header was
/// already removed).
pub(crate) fn rescope(request: &mut reqwest::blocking::Request) {
    ACTIVE.with(|active| {
        if let Some(scope) = active.borrow().as_ref().and_then(|c| c.scope.as_ref())
            && request.url().origin() != scope.url.origin()
        {
            for (name, _) in &scope.headers {
                request.headers_mut().remove(name);
            }
        }
    });
    apply(request);
}

/// An extraction request about to be sent: `Err` once an interrupt was
/// requested, otherwise the builder with the active scope's headers.
pub(crate) fn prepare(
    request: reqwest::blocking::RequestBuilder,
) -> Result<reqwest::blocking::RequestBuilder> {
    check()?;
    scoped(request).map_err(Error::http)
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
            let _guard = enter(&Default::default(), None, Some(&scope), &jar).unwrap();
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
        let _guard = enter(&Default::default(), Some(&interrupt), None, &jar).unwrap();
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
            let _guard = enter(&Default::default(), None, Some(&scope), &jar).unwrap();
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
    fn scoped_cookies_stay_with_their_call() {
        let jar = Arc::new(CookieJar::default());
        let url = Url::parse("https://media.example.test/a.ts").unwrap();
        let scope = RequestScope::new(&url)
            .unwrap()
            .cookie("session", "secret")
            .unwrap();
        let _guard = enter(&Default::default(), None, Some(&scope), &jar).unwrap();
        assert!(jar.cookies(&url).is_some());
        // Another call on the same client, on another thread, sends none.
        let (other, target) = (jar.clone(), url.clone());
        std::thread::spawn(move || {
            let _guard = enter(&Default::default(), None, None, &other).unwrap();
            assert!(other.cookies(&target).is_none());
            let mut request = reqwest::blocking::Client::new()
                .get(target.as_str())
                .build()
                .unwrap();
            apply(&mut request);
            assert!(request.headers().get("cookie").is_none());
        })
        .join()
        .unwrap();
        // Nor does another client's jar on this thread.
        assert!(CookieJar::default().cookies(&url).is_none());
    }

    #[test]
    fn redirect_hops_rescope_headers_and_cookies() {
        let jar = Arc::new(CookieJar::default());
        let origin = Url::parse("https://media.example.test/a.m3u8").unwrap();
        let scope = RequestScope::new(&origin)
            .unwrap()
            .header("X-Token", "secret")
            .unwrap()
            .set_cookie("cdn=1; Domain=example.test")
            .unwrap();
        let _guard = enter(&Default::default(), None, Some(&scope), &jar).unwrap();
        let mut request = prepare(reqwest::blocking::Client::new().get(origin.as_str()))
            .unwrap()
            .build()
            .unwrap();
        assert_eq!(request.headers()["x-token"], "secret");
        assert_eq!(request.headers()["cookie"], "cdn=1");
        // A hop elsewhere (whose `Cookie` the redirect removed).
        request.headers_mut().remove("cookie");
        *request.url_mut() = Url::parse("https://cdn.example.test/a.m3u8").unwrap();
        rescope(&mut request);
        assert!(request.headers().get("x-token").is_none());
        assert_eq!(request.headers()["cookie"], "cdn=1");
        request.headers_mut().remove("cookie");
        *request.url_mut() = Url::parse("https://other.test/a.m3u8").unwrap();
        rescope(&mut request);
        assert!(request.headers().get("cookie").is_none());
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
