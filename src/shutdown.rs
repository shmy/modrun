use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use tokio::sync::Notify;

/// Triggers graceful shutdown from inside the app.
///
/// Injected automatically. Calling [`shutdown`](Self::shutdown) unblocks
/// [`ModrunBuilder::run`](crate::ModrunBuilder::run) so stop hooks can run.
/// When using [`ModrunBuilder::start`](crate::ModrunBuilder::start), call
/// [`wait`](Self::wait) (or poll [`is_requested`](Self::is_requested)) from
/// your own event loop.
///
/// During build or OnStart, cancellation is cooperative: the current future is
/// dropped at its next `.await`. A call from a synchronous OnStart does not skip
/// later hooks that have not yet yielded.
///
/// After the app is running, a background [`crate::task`] that fails or panics
/// requests shutdown on its own. Custom work spawned with [`tokio::spawn`]
/// should still call [`shutdown`](Self::shutdown); otherwise
/// [`ModrunBuilder::run`](crate::ModrunBuilder::run) waits forever for an OS
/// signal.
#[derive(Clone)]
pub struct Shutdowner {
    inner: Arc<Inner>,
}

struct Inner {
    notify: Notify,
    requested: AtomicBool,
    from_failure: AtomicBool,
}

impl Shutdowner {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                notify: Notify::new(),
                requested: AtomicBool::new(false),
                from_failure: AtomicBool::new(false),
            }),
        }
    }

    /// Request a graceful shutdown.
    ///
    /// See the [type-level docs](Self) for cooperative cancellation and for
    /// unblocking [`ModrunBuilder::run`](crate::ModrunBuilder::run) after a
    /// background task fails.
    pub fn shutdown(&self) {
        self.inner.requested.store(true, Ordering::Release);
        self.inner.notify.notify_waiters();
    }

    /// Like [`shutdown`](Self::shutdown), but `run` treats a start-phase
    /// interrupt as a failure rather than a graceful `Ok(())`.
    pub(crate) fn fail(&self) {
        self.inner.from_failure.store(true, Ordering::Release);
        self.shutdown();
    }

    /// Whether a background [`crate::task`] requested shutdown after failing.
    #[must_use]
    pub(crate) fn is_failure(&self) -> bool {
        self.inner.from_failure.load(Ordering::Acquire)
    }

    /// Whether [`shutdown`](Self::shutdown) has already been called.
    #[must_use]
    pub fn is_requested(&self) -> bool {
        self.inner.requested.load(Ordering::Acquire)
    }

    /// Wait until shutdown is requested. Completes immediately if it already was.
    pub async fn wait(&self) {
        loop {
            // Register as a waiter *before* reading the flag, and enable so
            // `notify_waiters` is not lost between the check and the first poll.
            let mut notified = std::pin::pin!(self.inner.notify.notified());
            notified.as_mut().enable();

            if self.inner.requested.load(Ordering::Acquire) {
                return;
            }
            notified.await;
        }
    }
}

impl std::fmt::Debug for Shutdowner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shutdowner")
            .field("requested", &self.inner.requested.load(Ordering::Acquire))
            .field(
                "from_failure",
                &self.inner.from_failure.load(Ordering::Acquire),
            )
            .finish()
    }
}
