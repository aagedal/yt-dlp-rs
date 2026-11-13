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

/// Growing-file recordings in progress: a repeated signal waits for them
/// to finish their file and remove its recording mark instead of exiting.
#[cfg(any(unix, windows))]
static RECORDING: AtomicUsize = AtomicUsize::new(0);

/// Temporary guards installed over the run-wide handler: while any is, the
/// signal is theirs to handle.
#[cfg(any(unix, windows))]
static NESTED: AtomicUsize = AtomicUsize::new(0);

/// Installed guards of any kind: while there is none, a recorded signal is
/// stale (the last guard's owner handled it).
#[cfg(any(unix, windows))]
static GUARDS: AtomicUsize = AtomicUsize::new(0);

/// The handlers guards replaced, installed by the first guard and restored
/// by the last one: guards on different threads (library calls side by
/// side) drop in any order, so none may restore its own predecessor.
#[cfg(any(unix, windows))]
struct Installed {
    guards: usize,
    /// SIGINT's handler before the first guard.
    interrupt: usize,
    /// Guards also handling SIGTERM, and its handler before the first.
    terminating: usize,
    terminate: usize,
}

#[cfg(any(unix, windows))]
static INSTALLED: std::sync::Mutex<Installed> = std::sync::Mutex::new(Installed {
    guards: 0,
    interrupt: 0,
    terminating: 0,
    terminate: 0,
});

#[cfg(any(unix, windows))]
fn installed() -> std::sync::MutexGuard<'static, Installed> {
    INSTALLED
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

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
    if EXIT_AT_ONCE.load(Ordering::SeqCst)
        || (repeated
            && EXIT_ON_REPEAT.load(Ordering::SeqCst)
            && RECORDING.load(Ordering::SeqCst) == 0)
    {
        write_stderr(b"\nERROR: Interrupted by user\n");
        // SAFETY: `_exit` is async-signal-safe.
        unsafe { _exit(1) };
    }
}

/// The installed handler; dropping the last guard restores the previous
/// one.
pub(crate) struct Guard {
    /// With the run-wide handler in place, whether a signal was already
    /// pending for it: a signal this guard handled is not left for it to see.
    #[cfg(any(unix, windows))]
    nested: Option<bool>,
    /// Whether SIGTERM is handled too.
    #[cfg(any(unix, windows))]
    terminate: bool,
    /// Whether this guard holds a growing-file recording ([`RECORDING`]).
    #[cfg(any(unix, windows))]
    recording: bool,
}

impl Guard {
    pub(crate) fn install() -> Self {
        #[cfg(any(unix, windows))]
        {
            let mut installed = installed();
            // Over another guard, which already installed the handler.
            let nested = (installed.guards > 0).then(|| INTERRUPTED.load(Ordering::SeqCst));
            if installed.guards == 0 {
                installed.interrupt = replace(SIGINT);
            }
            installed.guards += 1;
            if nested.is_some() {
                NESTED.fetch_add(1, Ordering::SeqCst);
            }
            if nested != Some(true) {
                INTERRUPTED.store(false, Ordering::SeqCst);
            }
            GUARDS.fetch_add(1, Ordering::SeqCst);
            Self {
                nested,
                terminate: false,
                recording: false,
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
            let mut installed = installed();
            if installed.terminating == 0 {
                installed.terminate = replace(SIGTERM);
            }
            installed.terminating += 1;
            guard.terminate = true;
            guard
        }
        #[cfg(not(any(unix, windows)))]
        Self {}
    }

    /// [`Guard::install_with_terminate`] for a growing-file recording: a
    /// second signal does not end the process before FFmpeg has finished
    /// the file and its recording mark is removed (SIGKILL still does).
    pub(crate) fn install_recording() -> Self {
        #[cfg(any(unix, windows))]
        {
            let mut guard = Self::install_with_terminate();
            RECORDING.fetch_add(1, Ordering::SeqCst);
            guard.recording = true;
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
            if self.recording {
                RECORDING.fetch_sub(1, Ordering::SeqCst);
            }
            let mut installed = installed();
            GUARDS.fetch_sub(1, Ordering::SeqCst);
            installed.guards -= 1;
            if self.terminate {
                installed.terminating -= 1;
                if installed.terminating == 0 {
                    // SAFETY: restores the handler the first guard replaced.
                    unsafe { signal(SIGTERM, installed.terminate) };
                }
            }
            if installed.guards == 0 {
                // SAFETY: as above.
                unsafe { signal(SIGINT, installed.interrupt) };
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    /// SIGINT's current handler (set back at once).
    fn current() -> usize {
        // SAFETY: reinstalls the handler it read.
        unsafe {
            let handler = signal(SIGINT, SIG_IGN);
            signal(SIGINT, handler);
            handler
        }
    }

    #[test]
    fn guards_dropped_out_of_order_restore_the_original_handler_last() {
        let original = current();
        let handler = record as extern "C" fn(i32) as usize;
        // Two library calls' guards, the first dropped first.
        let first = Guard::install_with_terminate();
        let second = Guard::install();
        drop(first);
        assert_eq!(current(), handler);
        assert!(guarded());
        drop(second);
        assert_eq!(current(), original);
        assert!(!guarded());
    }
}
