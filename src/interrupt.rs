//! A temporary SIGINT handler: upstream's `KeyboardInterrupt` handling for
//! live FFmpeg downloads and `--wait-for-video`. Windows keeps the default
//! behavior, as upstream does not ask FFmpeg to quit there.
//!
//! Port extension: SIGTERM is handled like SIGINT while a handler is
//! installed, so a host that stops a live recording with SIGTERM gets the
//! finished (fixed up, post-processed and moved) file instead of a partial
//! one in a private workspace.
#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(unix)]
static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
const SIGINT: i32 = 2;

#[cfg(unix)]
const SIGTERM: i32 = 15;

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

/// The installed handler; dropping it restores the previous ones.
pub(crate) struct Guard {
    #[cfg(unix)]
    previous: [(i32, usize); 2],
}

impl Guard {
    pub(crate) fn install() -> Self {
        #[cfg(unix)]
        {
            INTERRUPTED.store(false, Ordering::SeqCst);
            let previous = [SIGINT, SIGTERM].map(|signum| {
                // SAFETY: the handler only stores to an atomic flag.
                let previous = unsafe { signal(signum, record as extern "C" fn(i32) as usize) };
                if previous == SIG_IGN {
                    // An ignored signal (SIGINT of a background job) stays
                    // ignored, as Python then installs no `KeyboardInterrupt`
                    // handler.
                    // SAFETY: restores the previous disposition.
                    unsafe { signal(signum, SIG_IGN) };
                }
                (signum, previous)
            });
            Self { previous }
        }
        #[cfg(not(unix))]
        Self {}
    }

    pub(crate) fn interrupted(&self) -> bool {
        interrupted()
    }
}

/// Whether SIGINT (or SIGTERM) arrived since a [`Guard`] was installed (for
/// threads that do not hold it).
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
        for (signum, previous) in self.previous {
            // SAFETY: restores the handlers `install` replaced.
            unsafe {
                signal(signum, previous);
            }
        }
    }
}

/// Ask a child process to terminate (SIGTERM; FFmpeg then finishes its
/// output like on `q`). Elsewhere the child is killed.
pub(crate) fn terminate(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        unsafe extern "C" {
            fn kill(pid: i32, sig: i32) -> i32;
        }
        if let Ok(pid) = i32::try_from(child.id()) {
            // SAFETY: signals the (not yet reaped) child's own process ID.
            unsafe { kill(pid, SIGTERM) };
        }
    }
    #[cfg(not(unix))]
    let _ = child.kill();
}
