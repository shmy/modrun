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
#[derive(Clone)]
pub struct Shutdowner {
    inner: Arc<Inner>,
}

struct Inner {
    notify: Notify,
    requested: AtomicBool,
}

impl Shutdowner {
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                notify: Notify::new(),
                requested: AtomicBool::new(false),
            }),
        }
    }

    /// Request a graceful shutdown.
    pub fn shutdown(&self) {
        self.inner.requested.store(true, Ordering::Release);
        self.inner.notify.notify_waiters();
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
            .finish()
    }
}
