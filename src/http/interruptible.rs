//! Requests a stop, cancel or signal can end while they wait on the network.
//!
//! reqwest's blocking client waits for a connect, the response headers and
//! each body read on the calling thread until the socket timeout, with no way
//! to end the wait from outside. While something can interrupt the call (a
//! library [`crate::Interrupt`] or a SIGINT/SIGTERM handler, see
//! [`crate::control::watch`]), [`send`] sends on a helper thread and streams
//! the body through a small queue, so the caller observes an interrupt within
//! [`POLL`] of it. The helper keeps the real socket timeout: a healthy slow
//! transfer is not cut short, and an abandoned helper ends at its next read
//! or timeout, closing the connection. Without an interrupt source requests
//! are sent directly, as before.
use crate::control::Watch;
use bytes::Bytes;
use reqwest::blocking::{RequestBuilder, Response};
use std::{
    collections::VecDeque,
    io::Read,
    pin::Pin,
    sync::{Arc, Condvar, Mutex, Weak, mpsc},
    task::{Context, Poll, Waker},
    time::Duration,
};

/// The longest an interrupt waits to be noticed by a blocked transfer.
pub(crate) const POLL: Duration = Duration::from_millis(100);

/// Bytes read per body chunk, and chunks read ahead of the caller.
const CHUNK: usize = 64 * 1024;
const DEPTH: usize = 4;

/// Helper threads only wait on reqwest's own runtime thread.
const STACK: usize = 256 * 1024;
/// [`stream`]'s helper drives the other transport itself (a BoringSSL
/// handshake needs more than [`STACK`]).
#[cfg_attr(not(feature = "impersonate"), allow(dead_code))]
const STREAM_STACK: usize = 2 * 1024 * 1024;

/// A transfer ended by a stop, cancel or signal (in the source chain of the
/// I/O error a body read returns).
#[derive(Debug)]
pub(crate) struct Interrupted;

impl std::fmt::Display for Interrupted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("transfer interrupted")
    }
}

impl std::error::Error for Interrupted {}

/// Whether `error` (or an error it wraps) is [`Interrupted`].
pub(crate) fn is_interrupted(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(error);
    while let Some(error) = next {
        if error.is::<Interrupted>() {
            return true;
        }
        // `io::Error::source` skips the error it wraps.
        if let Some(inner) = error
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
            && is_interrupted(inner)
        {
            return true;
        }
        next = error.source();
    }
    false
}

/// Send `request`; `Err(Interrupted)` once an interrupt arrives first. The
/// response's body reads end with an [`Interrupted`] I/O error the same way.
pub(crate) fn send(
    request: RequestBuilder,
) -> std::result::Result<reqwest::Result<Response>, Interrupted> {
    let Some(watch) = crate::control::watch() else {
        return Ok(request.send());
    };
    if watch.requested() {
        return Err(Interrupted);
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let shared = Arc::new(Shared::new(Some(watch.clone())));
    let pump = Arc::clone(&shared);
    let spawned = std::thread::Builder::new()
        .name("yt-dlp-rs-http".into())
        .stack_size(STACK)
        .spawn(move || {
            let response = match request.send() {
                Ok(response) => response,
                Err(error) => {
                    let _ = sender.send(Err(error));
                    return;
                }
            };
            let head = Head::of(&response);
            if sender.send(Ok(head)).is_ok() {
                pump.fill(response);
            }
        });
    if let Err(error) = spawned {
        // No thread to wait on: send directly.
        crate::screen::debug(&format!("Unable to start an HTTP thread: {error}"));
        return Ok(Err(spawn_failed(error)));
    }
    let head = loop {
        match receiver.recv_timeout(POLL) {
            Ok(Ok(head)) => break head,
            Ok(Err(error)) => return Ok(Err(error)),
            Err(mpsc::RecvTimeoutError::Timeout) if watch.requested() => {
                return Err(Interrupted);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Ok(Err(spawn_failed(std::io::Error::other(
                    "the HTTP thread ended without a response",
                ))));
            }
        }
    };
    Ok(Ok(head.response(shared)))
}

/// A response from another transport: its head, and its body as a
/// blocking reader.
#[cfg_attr(not(feature = "impersonate"), allow(dead_code))]
pub(crate) struct Streamed {
    pub(crate) status: reqwest::StatusCode,
    pub(crate) version: reqwest::Version,
    pub(crate) headers: reqwest::header::HeaderMap,
    pub(crate) url: url::Url,
    pub(crate) content_length: Option<u64>,
    pub(crate) body: Box<dyn Read + Send>,
}

/// [`send`] for another transport: `open` sends the request on a helper
/// thread, whose body then streams through the same queue (also without an
/// interrupt source, so a body is never held in memory).
#[cfg_attr(not(feature = "impersonate"), allow(dead_code))]
pub(crate) fn stream(
    open: impl FnOnce() -> crate::Result<Streamed> + Send + 'static,
) -> std::result::Result<crate::Result<Response>, Interrupted> {
    let watch = crate::control::watch();
    if watch.as_ref().is_some_and(Watch::requested) {
        return Err(Interrupted);
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let shared = Arc::new(Shared::new(watch.clone()));
    let pump = Arc::clone(&shared);
    let spawned = std::thread::Builder::new()
        .name("yt-dlp-rs-http".into())
        .stack_size(STREAM_STACK)
        .spawn(move || match open() {
            Ok(streamed) => {
                let head = Head {
                    status: streamed.status,
                    version: streamed.version,
                    headers: streamed.headers,
                    extensions: http::Extensions::new(),
                    url: streamed.url,
                    content_length: streamed.content_length,
                };
                if sender.send(Ok(head)).is_ok() {
                    pump.fill(streamed.body);
                }
            }
            Err(error) => {
                let _ = sender.send(Err(error));
            }
        });
    if let Err(error) = spawned {
        return Ok(Err(crate::Error::Io(error)));
    }
    let head = loop {
        match receiver.recv_timeout(POLL) {
            Ok(Ok(head)) => break head,
            Ok(Err(error)) => return Ok(Err(error)),
            Err(mpsc::RecvTimeoutError::Timeout)
                if watch.as_ref().is_some_and(Watch::requested) =>
            {
                return Err(Interrupted);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Ok(Err(crate::Error::MediaTransport(
                    "the HTTP thread ended without a response".into(),
                )));
            }
        }
    };
    Ok(Ok(head.response(shared)))
}

/// A request error for a helper thread failure (built through reqwest so
/// callers keep handling one error type).
fn spawn_failed(error: std::io::Error) -> reqwest::Error {
    // `Response::error_for_status` is the only public constructor; a body
    // read error carries the cause instead.
    let response = Response::from(
        http::Response::builder()
            .body(reqwest::Body::wrap(Failed(Some(error))))
            .expect("a static response"),
    );
    response
        .bytes()
        .expect_err("the body fails on its first read")
}

/// A body whose first read fails with the given error.
struct Failed(Option<std::io::Error>);

impl http_body::Body for Failed {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, std::io::Error>>> {
        Poll::Ready(self.0.take().map(Err))
    }
}

/// What the caller sees of the response before its body.
struct Head {
    status: reqwest::StatusCode,
    version: reqwest::Version,
    headers: reqwest::header::HeaderMap,
    /// Hyper's reason phrase and the other response extensions.
    extensions: http::Extensions,
    url: url::Url,
    content_length: Option<u64>,
}

impl Head {
    fn of(response: &Response) -> Self {
        Self {
            status: response.status(),
            version: response.version(),
            headers: response.headers().clone(),
            extensions: response.extensions().clone(),
            url: response.url().clone(),
            content_length: response.content_length(),
        }
    }

    /// A response reading `shared`'s queue, otherwise like the original.
    fn response(self, shared: Arc<Shared>) -> Response {
        use reqwest::ResponseBuilderExt as _;
        let mut builder = http::Response::builder()
            .status(self.status)
            .version(self.version);
        if let Some(headers) = builder.headers_mut() {
            *headers = self.headers;
        }
        if let Some(extensions) = builder.extensions_mut() {
            *extensions = self.extensions;
        }
        if shared.watch.is_some() {
            watcher::register(&shared);
        }
        let body = Body {
            shared,
            content_length: self.content_length,
            done: false,
        };
        Response::from(
            builder
                .url(self.url)
                .body(reqwest::Body::wrap(body))
                .expect("parts of a valid response"),
        )
    }
}

struct Shared {
    /// `None`: nothing can interrupt the transfer.
    watch: Option<Watch>,
    state: Mutex<State>,
    /// Signals the helper that the queue has room or the reader left.
    space: Condvar,
}

#[derive(Default)]
struct State {
    chunks: VecDeque<Bytes>,
    /// The body's end: `Ok` at EOF, else the read error.
    end: Option<std::io::Result<()>>,
    /// The caller's task, woken by new data or an interrupt.
    waker: Option<Waker>,
    /// The response was dropped; the helper stops reading.
    closed: bool,
}

impl Shared {
    fn new(watch: Option<Watch>) -> Self {
        Self {
            watch,
            state: Mutex::new(State::default()),
            space: Condvar::new(),
        }
    }

    fn requested(&self) -> bool {
        self.watch.as_ref().is_some_and(Watch::requested)
    }

    /// The helper thread: read `response` into the queue until its end, an
    /// error or the reader leaving.
    fn fill(&self, mut response: impl Read) {
        let mut buffer = vec![0; CHUNK];
        loop {
            let read = loop {
                match response.read(&mut buffer) {
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    read => break read,
                }
            };
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            match read {
                Ok(0) => state.end = Some(Ok(())),
                Ok(count) => state
                    .chunks
                    .push_back(Bytes::copy_from_slice(&buffer[..count])),
                Err(error) => state.end = Some(Err(error)),
            }
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
            if state.end.is_some() {
                return;
            }
            while state.chunks.len() >= DEPTH && !state.closed {
                state = match self.space.wait(state) {
                    Ok(state) => state,
                    Err(_) => return,
                };
            }
            if state.closed {
                return;
            }
        }
    }

    /// Wake the reader if an interrupt arrived (the watcher's check).
    fn check(&self) {
        if self.requested()
            && let Some(waker) = self
                .state
                .lock()
                .ok()
                .and_then(|mut state| state.waker.take())
        {
            waker.wake();
        }
    }
}

/// The caller's end of the queue.
struct Body {
    shared: Arc<Shared>,
    content_length: Option<u64>,
    done: bool,
}

impl http_body::Body for Body {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<http_body::Frame<Bytes>, std::io::Error>>> {
        if self.done {
            return Poll::Ready(None);
        }
        let Ok(mut state) = self.shared.state.lock() else {
            return Poll::Ready(Some(Err(std::io::Error::other("HTTP body queue poisoned"))));
        };
        if let Some(chunk) = state.chunks.pop_front() {
            self.shared.space.notify_one();
            return Poll::Ready(Some(Ok(http_body::Frame::data(chunk))));
        }
        if let Some(end) = state.end.take() {
            drop(state);
            self.done = true;
            return Poll::Ready(end.err().map(Err));
        }
        if self.shared.requested() {
            drop(state);
            self.done = true;
            return Poll::Ready(Some(Err(std::io::Error::other(Interrupted))));
        }
        state.waker = Some(context.waker().clone());
        Poll::Pending
    }

    fn is_end_stream(&self) -> bool {
        self.done
    }

    fn size_hint(&self) -> http_body::SizeHint {
        self.content_length.map_or_else(
            http_body::SizeHint::default,
            http_body::SizeHint::with_exact,
        )
    }
}

impl Drop for Body {
    fn drop(&mut self) {
        if let Ok(mut state) = self.shared.state.lock() {
            state.closed = true;
            state.chunks.clear();
        }
        self.shared.space.notify_all();
    }
}

/// One thread checks the open bodies' interrupt sources every [`POLL`]
/// while any exists, waking a reader parked in reqwest's blocking read (a
/// signal handler or [`crate::Interrupt`] cannot wake it directly).
mod watcher {
    use super::*;

    static BODIES: Mutex<Vec<Weak<Shared>>> = Mutex::new(Vec::new());
    static THREAD: std::sync::OnceLock<Option<std::thread::Thread>> = std::sync::OnceLock::new();

    pub(super) fn register(shared: &Arc<Shared>) {
        if let Ok(mut bodies) = BODIES.lock() {
            bodies.push(Arc::downgrade(shared));
        }
        let thread = THREAD.get_or_init(|| {
            std::thread::Builder::new()
                .name("yt-dlp-rs-interrupt".into())
                .stack_size(STACK)
                .spawn(run)
                .ok()
                .map(|handle| handle.thread().clone())
        });
        if let Some(thread) = thread {
            thread.unpark();
        }
    }

    fn run() {
        loop {
            let live: Vec<Arc<Shared>> = match BODIES.lock() {
                Ok(mut bodies) => {
                    bodies.retain(|body| body.strong_count() > 0);
                    bodies.iter().filter_map(Weak::upgrade).collect()
                }
                Err(_) => return,
            };
            if live.is_empty() {
                std::thread::park();
                continue;
            }
            for shared in &live {
                shared.check();
            }
            drop(live);
            std::thread::sleep(POLL);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;
    use std::net::TcpListener;
    use std::time::Instant;

    fn client() -> reqwest::blocking::Client {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(20))
            .connect_timeout(Duration::from_secs(20))
            .build()
            .unwrap()
    }

    /// A server sending `head`, then `body` and then nothing until the
    /// connection closes.
    fn stalling(head: &'static [u8], body: &'static [u8]) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                std::thread::spawn(move || {
                    let mut request = [0; 4096];
                    let _ = stream.read(&mut request);
                    let _ = stream.write_all(head);
                    let _ = stream.write_all(body);
                    let _ = stream.flush();
                    std::thread::sleep(Duration::from_secs(30));
                });
            }
        });
        format!("http://{address}/")
    }

    fn within(started: Instant) {
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_stop_ends_a_wait_for_headers() {
        let url = stalling(b"HTTP/1.1 200 OK\r\nContent-Le", b"");
        let jar = Arc::new(crate::CookieJar::default());
        let interrupt = crate::Interrupt::new();
        let _context = crate::control::enter(Some(&interrupt), None, &jar).unwrap();
        let stopper = interrupt.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            stopper.stop();
        });
        let started = Instant::now();
        assert!(send(client().get(&url)).is_err());
        within(started);
    }

    #[test]
    fn a_cancel_ends_a_body_read_and_keeps_received_bytes() {
        let url = stalling(
            b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n",
            b"0123456789",
        );
        let jar = Arc::new(crate::CookieJar::default());
        let interrupt = crate::Interrupt::new();
        let _context = crate::control::enter(Some(&interrupt), None, &jar).unwrap();
        let mut response = send(client().get(&url)).unwrap().unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(response.content_length(), Some(100));
        assert_eq!(response.url().as_str(), url);

        let mut buffer = [0; 10];
        response.read_exact(&mut buffer).unwrap();
        assert_eq!(&buffer, b"0123456789");
        let canceller = interrupt.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            canceller.cancel();
        });
        let started = Instant::now();
        let error = response.read(&mut buffer).unwrap_err();
        within(started);
        assert!(is_interrupted(&error), "{error:?}");
        // Later requests end at once.
        assert!(send(client().get(&url)).is_err());
    }

    #[test]
    fn a_servers_reason_phrase_is_kept() {
        let url = stalling(
            b"HTTP/1.1 404 Gone Fishing\r\nContent-Length: 0\r\n\r\n",
            b"",
        );
        let jar = Arc::new(crate::CookieJar::default());
        let interrupt = crate::Interrupt::new();
        let _context = crate::control::enter(Some(&interrupt), None, &jar).unwrap();
        let response = send(client().get(&url)).unwrap().unwrap();
        let error = response.error_for_status().unwrap_err();
        assert!(error.to_string().contains("404 Gone Fishing"), "{error}");
    }

    #[test]
    fn whole_body_helpers_end_at_an_interrupt() {
        let url = stalling(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n", b"01234");
        let jar = Arc::new(crate::CookieJar::default());
        let interrupt = crate::Interrupt::new();
        let _context = crate::control::enter(Some(&interrupt), None, &jar).unwrap();
        let response = send(client().get(&url)).unwrap().unwrap();
        let stopper = interrupt.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            stopper.stop();
        });
        let started = Instant::now();
        let error = response.text().unwrap_err();
        within(started);
        assert!(is_interrupted(&error), "{error:?}");
    }

    #[test]
    fn bodies_stream_unchanged_and_errors_pass_through() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body: Vec<u8> = (0..1_000_000_u32).map(|value| value as u8).collect();
        let served = body.clone();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n",
                served.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&served).unwrap();
        });
        let jar = Arc::new(crate::CookieJar::default());
        let interrupt = crate::Interrupt::new();
        let _context = crate::control::enter(Some(&interrupt), None, &jar).unwrap();
        let response = send(client().get(format!("http://{address}/"))).unwrap();
        assert_eq!(response.unwrap().bytes().unwrap().as_ref(), body.as_slice());
        // A refused connection fails as a reqwest connect error.
        let closed = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let error = send(client().get(format!("http://127.0.0.1:{port}/")))
            .unwrap()
            .unwrap_err();
        assert!(error.is_connect(), "{error:?}");
    }
}
