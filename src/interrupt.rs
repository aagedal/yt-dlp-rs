//! SIGINT (and SIGTERM) handlers recording upstream's `KeyboardInterrupt`:
//! the CLI's handler for a whole run, and temporary ones for live FFmpeg
//! downloads, `--wait-for-video` and growing files, which consume the
//! signal themselves. Windows keeps the default behavior, as upstream does
//! not ask FFmpeg to quit there.
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[cfg(unix)]
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Whether a signal arriving after an unhandled one ends the process at
/// once (the CLI's run-wide handler: a second Ctrl+C does not wait for the
/// next checkpoint).
#[cfg(unix)]
static EXIT_ON_REPEAT: AtomicBool = AtomicBool::new(false);

/// Temporary guards installed over the run-wide handler: while any is, the
/// signal is theirs to handle.
#[cfg(unix)]
static NESTED: AtomicUsize = AtomicUsize::new(0);

/// Whether a signal ends the process at once (a prompt waiting for input,
/// where upstream's `input()` raises `KeyboardInterrupt` immediately).
#[cfg(unix)]
static EXIT_AT_ONCE: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
const SIGINT: i32 = 2;

#[cfg(unix)]
const SIGTERM: i32 = 15;

#[cfg(unix)]
const SIG_IGN: usize = 1;

#[cfg(unix)]
unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
    fn write(fd: i32, buffer: *const std::ffi::c_void, count: usize) -> isize;
    fn _exit(status: i32) -> !;
}

#[cfg(unix)]
extern "C" fn record(_: i32) {
    let repeated = INTERRUPTED.swap(true, Ordering::SeqCst);
    if EXIT_AT_ONCE.load(Ordering::SeqCst) || (repeated && EXIT_ON_REPEAT.load(Ordering::SeqCst)) {
        const MESSAGE: &[u8] = b"\nERROR: Interrupted by user\n";
        // SAFETY: `write` and `_exit` are async-signal-safe.
        unsafe {
            write(2, MESSAGE.as_ptr().cast(), MESSAGE.len());
            _exit(1);
        }
    }
}

/// The installed handler; dropping it restores the previous one.
pub(crate) struct Guard {
    #[cfg(unix)]
    previous: usize,
    /// With the run-wide handler in place, whether a signal was already
    /// pending for it: a signal this guard handled is not left for it to see.
    #[cfg(unix)]
    nested: Option<bool>,
    /// SIGTERM's previous handler, when it is handled too.
    #[cfg(unix)]
    terminate: Option<usize>,
}

impl Guard {
    pub(crate) fn install() -> Self {
        #[cfg(unix)]
        {
            let previous = replace(SIGINT);
            let nested = (previous == record as extern "C" fn(i32) as usize)
                .then(|| INTERRUPTED.load(Ordering::SeqCst));
            if nested.is_some() {
                NESTED.fetch_add(1, Ordering::SeqCst);
            }
            if nested != Some(true) {
                INTERRUPTED.store(false, Ordering::SeqCst);
            }
            Self {
                previous,
                nested,
                terminate: None,
            }
        }
        #[cfg(not(unix))]
        Self {}
    }

    /// Also end at SIGTERM, which stops a growing-file recording
    /// (`--growing-file`, a port extension) as an interrupt does, so its
    /// file is finished and its recording mark removed.
    pub(crate) fn install_with_terminate() -> Self {
        #[cfg(unix)]
        {
            let mut guard = Self::install();
            guard.terminate = Some(replace(SIGTERM));
            guard
        }
        #[cfg(not(unix))]
        Self {}
    }

    pub(crate) fn interrupted(&self) -> bool {
        interrupted()
    }
}

/// Whether SIGINT arrived since a [`Guard`] was installed (for threads
/// that do not hold it).
pub(crate) fn interrupted() -> bool {
    #[cfg(unix)]
    {
        INTERRUPTED.load(Ordering::SeqCst)
    }
    #[cfg(not(unix))]
    false
}

/// Install `record` for `signum`, returning the previous handler.
#[cfg(unix)]
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
    #[cfg(unix)]
    {
        NESTED.load(Ordering::SeqCst) > 0
    }
    #[cfg(not(unix))]
    false
}

/// Run `wait` (a blocking prompt) with signals exiting at once.
pub(crate) fn exit_at_once<T>(wait: impl FnOnce() -> T) -> T {
    #[cfg(unix)]
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
    #[cfg(not(unix))]
    wait()
}

/// The CLI's handler for a whole run (see `control::SignalInterrupt`):
/// SIGINT and SIGTERM are recorded, and a second one exits at once.
pub(crate) fn install_run_wide() -> Guard {
    let guard = Guard::install_with_terminate();
    #[cfg(unix)]
    EXIT_ON_REPEAT.store(true, Ordering::SeqCst);
    guard
}

impl Drop for Guard {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
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
