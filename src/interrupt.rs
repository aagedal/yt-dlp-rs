//! A temporary SIGINT handler: upstream's `KeyboardInterrupt` handling for
//! live FFmpeg downloads and `--wait-for-video`. Windows keeps the default
//! behavior, as upstream does not ask FFmpeg to quit there.
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(unix)]
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
const SIGINT: i32 = 2;

#[cfg(unix)]
const SIG_IGN: usize = 1;

#[cfg(unix)]
unsafe extern "C" {
    fn signal(signum: i32, handler: usize) -> usize;
}

#[cfg(unix)]
extern "C" fn record(_: i32) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

/// The installed handler; dropping it restores the previous one.
pub(crate) struct Guard {
    #[cfg(unix)]
    previous: usize,
}

impl Guard {
    pub(crate) fn install() -> Self {
        #[cfg(unix)]
        {
            INTERRUPTED.store(false, Ordering::SeqCst);
            // SAFETY: the handler only stores to an atomic flag.
            let previous = unsafe { signal(SIGINT, record as extern "C" fn(i32) as usize) };
            if previous == SIG_IGN {
                // An ignored SIGINT (a background job) stays ignored, as
                // Python then installs no `KeyboardInterrupt` handler.
                // SAFETY: restores the previous disposition.
                unsafe { signal(SIGINT, SIG_IGN) };
            }
            Self { previous }
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

impl Drop for Guard {
    fn drop(&mut self) {
        #[cfg(unix)]
        // SAFETY: restores the handler `install` replaced.
        unsafe {
            signal(SIGINT, self.previous);
        }
    }
}
