//! SIGINT (and SIGTERM) handlers recording upstream's `KeyboardInterrupt`:
//! the CLI's handler for a whole run, and temporary ones for live FFmpeg
//! downloads, `--wait-for-video` and growing files, which consume the
//! signal themselves. On Windows the C runtime's `signal` maps Ctrl+C to
//! SIGINT, like Python's handler, and Ctrl+Break (`SIGBREAK`, the console
//! event a parent can send to one process group) stands in for SIGTERM.
#[cfg(any(unix, windows))]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[cfg(any(unix, windows))]
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Whether a signal arriving after an unhandled one ends the process at
/// once (the CLI's run-wide handler: a second Ctrl+C does not wait for the
/// next checkpoint).
#[cfg(any(unix, windows))]
static EXIT_ON_REPEAT: AtomicBool = AtomicBool::new(false);

/// Temporary guards installed over the run-wide handler: while any is, the
/// signal is theirs to handle.
#[cfg(any(unix, windows))]
static NESTED: AtomicUsize = AtomicUsize::new(0);

/// Installed guards of any kind: while there is none, a recorded signal is
/// stale (the last guard's owner handled it).
#[cfg(any(unix, windows))]
static GUARDS: AtomicUsize = AtomicUsize::new(0);

/// Whether a signal ends the process at once (a prompt waiting for input,
/// where upstream's `input()` raises `KeyboardInterrupt` immediately).
#[cfg(any(unix, windows))]
static EXIT_AT_ONCE: AtomicBool = AtomicBool::new(false);

#[cfg(any(unix, windows))]
const SIGINT: i32 = 2;

/// The signal standing in for a stop request: SIGTERM, or Ctrl+Break's
/// `SIGBREAK` on Windows (where SIGTERM is never raised).
#[cfg(unix)]
const SIGTERM: i32 = 15;
#[cfg(windows)]
const SIGTERM: i32 = 21;

#[cfg(any(unix, windows))]
const SIG_IGN: usize = 1;

#[cfg(unix)]
unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
    fn write(fd: i32, buffer: *const std::ffi::c_void, count: usize) -> isize;
    fn _exit(status: i32) -> !;
}

#[cfg(windows)]
unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
    fn _exit(status: i32) -> !;
}

#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn GetStdHandle(id: u32) -> *mut std::ffi::c_void;
    fn WriteFile(
        file: *mut std::ffi::c_void,
        buffer: *const std::ffi::c_void,
        count: u32,
        written: *mut u32,
        overlapped: *mut std::ffi::c_void,
    ) -> i32;
}

/// `write(2, ...)` from a signal handler.
#[cfg(any(unix, windows))]
fn write_stderr(message: &[u8]) {
    // SAFETY: `message` is valid for its length; `write` is async-signal-safe.
    #[cfg(unix)]
    unsafe {
        write(2, message.as_ptr().cast(), message.len());
    }
    // The C runtime's `_write(2, ...)` would translate newlines; the C
    // runtime runs console handlers on their own thread, where this is safe.
    // SAFETY: as above; `written` is a valid out-pointer.
    #[cfg(windows)]
    unsafe {
        const STD_ERROR_HANDLE: u32 = -12i32 as u32;
        let mut written = 0;
        WriteFile(
            GetStdHandle(STD_ERROR_HANDLE),
            message.as_ptr().cast(),
            message.len() as u32,
            &mut written,
            std::ptr::null_mut(),
        );
    }
}

#[cfg(any(unix, windows))]
extern "C" fn record(signum: i32) {
    // The C runtime resets a Windows handler before calling it.
    // SAFETY: re-installs this handler, which only stores to atomics.
    #[cfg(windows)]
    unsafe {
        signal(signum, record as extern "C" fn(i32) as usize);
    }
    #[cfg(unix)]
    let _ = signum;
    let repeated = INTERRUPTED.swap(true, Ordering::SeqCst);
    if EXIT_AT_ONCE.load(Ordering::SeqCst) || (repeated && EXIT_ON_REPEAT.load(Ordering::SeqCst)) {
        write_stderr(b"\nERROR: Interrupted by user\n");
        // SAFETY: `_exit` is async-signal-safe.
        unsafe { _exit(1) };
    }
}

/// The installed handler; dropping it restores the previous one.
pub(crate) struct Guard {
    #[cfg(any(unix, windows))]
    previous: usize,
    /// With the run-wide handler in place, whether a signal was already
    /// pending for it: a signal this guard handled is not left for it to see.
    #[cfg(any(unix, windows))]
    nested: Option<bool>,
    /// SIGTERM's previous handler, when it is handled too.
    #[cfg(any(unix, windows))]
    terminate: Option<usize>,
}

impl Guard {
    pub(crate) fn install() -> Self {
        #[cfg(any(unix, windows))]
        {
            // Over another guard, whose SIGINT handler is not the previous
            // one when SIGINT is ignored (`replace` keeps it ignored) but
            // which still handles SIGTERM.
            let nested =
                (GUARDS.load(Ordering::SeqCst) > 0).then(|| INTERRUPTED.load(Ordering::SeqCst));
            let previous = replace(SIGINT);
            if nested.is_some() {
                NESTED.fetch_add(1, Ordering::SeqCst);
            }
            if nested != Some(true) {
                INTERRUPTED.store(false, Ordering::SeqCst);
            }
            GUARDS.fetch_add(1, Ordering::SeqCst);
            Self {
                previous,
                nested,
                terminate: None,
            }
        }
        #[cfg(not(any(unix, windows)))]
        Self {}
    }

    /// Also end at SIGTERM, which stops a growing-file recording
    /// (`--growing-file`, a port extension) as an interrupt does, so its
    /// file is finished and its recording mark removed.
    pub(crate) fn install_with_terminate() -> Self {
        #[cfg(any(unix, windows))]
        {
            let mut guard = Self::install();
            guard.terminate = Some(replace(SIGTERM));
            guard
        }
        #[cfg(not(any(unix, windows)))]
        Self {}
    }

    pub(crate) fn interrupted(&self) -> bool {
        interrupted()
    }
}

/// Whether SIGINT arrived since a [`Guard`] was installed (for threads
/// that do not hold it).
pub(crate) fn interrupted() -> bool {
    #[cfg(any(unix, windows))]
    {
        INTERRUPTED.load(Ordering::SeqCst)
    }
    #[cfg(not(any(unix, windows)))]
    false
}

/// Whether a handler is installed and a signal arrived since: any guard's
/// signal ends transfers in flight (its owner then stops, finishes or ends
/// the run as it would at its next checkpoint).
pub(crate) fn pending() -> bool {
    #[cfg(any(unix, windows))]
    {
        GUARDS.load(Ordering::SeqCst) > 0 && INTERRUPTED.load(Ordering::SeqCst)
    }
    #[cfg(not(any(unix, windows)))]
    false
}

/// Whether a signal handler is installed.
pub(crate) fn guarded() -> bool {
    #[cfg(any(unix, windows))]
    {
        GUARDS.load(Ordering::SeqCst) > 0
    }
    #[cfg(not(any(unix, windows)))]
    false
}

/// Install `record` for `signum`, returning the previous handler.
#[cfg(any(unix, windows))]
fn replace(signum: i32) -> usize {
    // SAFETY: the handler only stores to an atomic flag.
    let previous = unsafe { signal(signum, record as extern "C" fn(i32) as usize) };
    if previous == SIG_IGN {
        // An ignored signal (SIGINT in a background job) stays ignored, as
        // Python then installs no `KeyboardInterrupt` handler.
        // SAFETY: restores the previous disposition.
        unsafe { signal(signum, SIG_IGN) };
    }
    previous
}

/// Whether a temporary guard over the run-wide handler handles signals.
pub(crate) fn handled_elsewhere() -> bool {
    #[cfg(any(unix, windows))]
    {
        NESTED.load(Ordering::SeqCst) > 0
    }
    #[cfg(not(any(unix, windows)))]
    false
}

/// Run `wait` (a blocking prompt) with signals exiting at once.
pub(crate) fn exit_at_once<T>(wait: impl FnOnce() -> T) -> T {
    #[cfg(any(unix, windows))]
    {
        struct Reset;
        impl Drop for Reset {
            fn drop(&mut self) {
                EXIT_AT_ONCE.store(false, Ordering::SeqCst);
            }
        }
        EXIT_AT_ONCE.store(true, Ordering::SeqCst);
        let _reset = Reset;
        wait()
    }
    #[cfg(not(any(unix, windows)))]
    wait()
}

/// The CLI's handler for a whole run (see `control::SignalInterrupt`):
/// SIGINT and SIGTERM are recorded, and a second one exits at once.
pub(crate) fn install_run_wide() -> Guard {
    let guard = Guard::install_with_terminate();
    #[cfg(any(unix, windows))]
    EXIT_ON_REPEAT.store(true, Ordering::SeqCst);
    guard
}

impl Drop for Guard {
    fn drop(&mut self) {
        #[cfg(any(unix, windows))]
        {
            GUARDS.fetch_sub(1, Ordering::SeqCst);
            // SAFETY: restores the handlers `install` replaced.
            unsafe {
                if let Some(previous) = self.terminate {
                    signal(SIGTERM, previous);
                }
                signal(SIGINT, self.previous);
            }
            match self.nested {
                // The live download, wait or recording handled a signal of
                // its own, as upstream catches its `KeyboardInterrupt`.
                Some(pending) => {
                    INTERRUPTED.store(pending, Ordering::SeqCst);
                    NESTED.fetch_sub(1, Ordering::SeqCst);
                }
                None => EXIT_ON_REPEAT.store(false, Ordering::SeqCst),
            }
        }
    }
}
