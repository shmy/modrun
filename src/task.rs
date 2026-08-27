use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::error::{Error, Result};
use crate::lifecycle::Hook;

/// Completes when the matching [`Task`] / [`PreparedTask`] begins OnStop (or is
/// dropped).
///
/// Pass this to [`tokio::select!`] or Axum's `with_graceful_shutdown`.
///
/// ```
/// use modrun::{Lifecycle, task};
///
/// fn boot(lc: Lifecycle) -> modrun::Result<()> {
///     lc.append(task("consumer", |stopped| async move {
///         tokio::select! {
///             _ = stopped => Ok(()),
///             _ = consume() => Ok(()),
///         }
///     }))
/// }
///
/// # async fn consume() {}
/// # fn _unused() { let _ = boot; }
/// ```
pub struct Stopped {
    rx: oneshot::Receiver<()>,
}

impl Future for Stopped {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        match Pin::new(&mut self.rx).poll(cx) {
            Poll::Ready(_) => Poll::Ready(()),
            Poll::Pending => Poll::Pending,
        }
    }
}

struct LiveTask {
    name: &'static str,
    stop_tx: Option<oneshot::Sender<()>>,
    handle: Option<JoinHandle<Result<()>>>,
}

impl LiveTask {
    fn new(name: &'static str) -> Self {
        Self {
            name,
            stop_tx: None,
            handle: None,
        }
    }

    fn spawn_with_stop<F, Fut>(&mut self, run: F)
    where
        F: FnOnce(Stopped) -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let (stop_tx, stop_rx) = oneshot::channel();
        self.stop_tx = Some(stop_tx);
        self.handle = Some(tokio::spawn(run(Stopped { rx: stop_rx })));
    }

    async fn on_stop(&mut self) -> Result<()> {
        let _ = self.stop_tx.take().map(|tx| tx.send(()));
        let Some(handle) = self.handle.as_mut() else {
            return Ok(());
        };
        let joined = handle.await;
        self.handle = None;
        match joined {
            Ok(result) => result,
            Err(err) if err.is_cancelled() => Ok(()),
            Err(err) => Err(Error::hook(err)),
        }
    }
}

impl Drop for LiveTask {
    fn drop(&mut self) {
        if let Some(tx) = self.stop_tx.take() {
            let _ = tx.send(());
        }
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
    }
}

/// Long-running work spawned in OnStart and joined in OnStop.
///
/// Created with [`task`]. OnStart returns as soon as the task is spawned. For
/// bind/listen that must fail start, use [`task_with`].
pub struct Task<F> {
    live: LiveTask,
    start: Option<F>,
}

/// Await `prepare` during OnStart, then spawn `run` in the background.
///
/// Created with [`task_with`]. A `prepare` error (for example `AddrInUse`) fails
/// start instead of printing `RUNNING` and surfacing later on stop.
pub struct PreparedTask<P, R> {
    live: LiveTask,
    prepare: Option<P>,
    run: Option<R>,
}

/// Spawn `run` as a background task from OnStart, signal it on OnStop, then join.
///
/// `run` receives a [`Stopped`] future that completes when this hook stops (or
/// is dropped). The task is aborted if the hook is dropped without a finished
/// OnStop (timeout, unwind abandon, or `RunningApp` leak).
///
/// OnStart returns as soon as the work is spawned. Do **not** bind a server
/// port inside `run` — that failure would only show up on stop. Use
/// [`task_with`] so listen happens during OnStart.
///
/// If `run` can fail after start has already succeeded, call
/// [`crate::Shutdowner::shutdown`] so [`crate::ModrunBuilder::run`] does not wait
/// forever for a signal.
///
/// ```
/// use modrun::{Lifecycle, Modrun, task};
///
/// fn boot(lc: Lifecycle) -> modrun::Result<()> {
///     lc.append(task("worker", |stopped| async move {
///         stopped.await;
///         Ok(())
///     }))
/// }
///
/// # #[tokio::main]
/// # async fn main() -> modrun::Result<()> {
/// Modrun::builder()
///     .no_banner()
///     .invoke(boot)
///     .start()
///     .await?
///     .stop()
///     .await
/// # }
/// ```
#[must_use]
pub fn task<F, Fut>(name: &'static str, run: F) -> Task<F>
where
    F: FnOnce(Stopped) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    Task {
        live: LiveTask::new(name),
        start: Some(run),
    }
}

/// Like [`task`], but `prepare` is awaited during OnStart.
///
/// Use this for servers: bind/listen in `prepare` so `AddrInUse` fails start.
/// `run` then receives the prepared value and a [`Stopped`] future.
///
/// ```
/// use modrun::{Lifecycle, Modrun, Shutdowner, task_with};
///
/// fn boot(lc: Lifecycle, shutdown: Shutdowner) -> modrun::Result<()> {
///     lc.append(task_with(
///         "http.serve",
///         || async { Ok(()) },
///         move |(), stopped| async move {
///             tokio::select! {
///                 _ = stopped => Ok(()),
///                 result = serve() => {
///                     if result.is_err() {
///                         shutdown.shutdown();
///                     }
///                     result
///                 }
///             }
///         },
///     ))
/// }
///
/// # async fn serve() -> modrun::Result<()> { Ok(()) }
/// # #[tokio::main]
/// # async fn main() -> modrun::Result<()> {
/// Modrun::builder()
///     .no_banner()
///     .invoke(boot)
///     .start()
///     .await?
///     .stop()
///     .await
/// # }
/// ```
#[must_use]
pub fn task_with<P, T, PF, R, RF>(name: &'static str, prepare: P, run: R) -> PreparedTask<P, R>
where
    P: FnOnce() -> PF + Send + 'static,
    PF: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
    R: FnOnce(T, Stopped) -> RF + Send + 'static,
    RF: Future<Output = Result<()>> + Send + 'static,
{
    PreparedTask {
        live: LiveTask::new(name),
        prepare: Some(prepare),
        run: Some(run),
    }
}

impl<F> std::fmt::Debug for Task<F> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Task")
            .field("name", &self.live.name)
            .field("running", &self.live.handle.is_some())
            .finish()
    }
}

impl<P, R> std::fmt::Debug for PreparedTask<P, R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PreparedTask")
            .field("name", &self.live.name)
            .field("running", &self.live.handle.is_some())
            .finish()
    }
}

impl<F, Fut> Hook for Task<F>
where
    F: FnOnce(Stopped) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    fn name(&self) -> Option<&'static str> {
        Some(self.live.name)
    }

    fn on_start(&mut self) -> impl Future<Output = Result<()>> + Send {
        let run = self.start.take().expect("task OnStart ran twice");
        // Spawn before returning so a cancelled OnStart still drops `self`
        // with the handle set, and [`LiveTask::drop`] aborts the work.
        self.live.spawn_with_stop(run);
        async { Ok(()) }
    }

    async fn on_stop(&mut self) -> Result<()> {
        self.live.on_stop().await
    }
}

impl<P, T, PF, R, RF> Hook for PreparedTask<P, R>
where
    P: FnOnce() -> PF + Send + 'static,
    PF: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
    R: FnOnce(T, Stopped) -> RF + Send + 'static,
    RF: Future<Output = Result<()>> + Send + 'static,
{
    fn name(&self) -> Option<&'static str> {
        Some(self.live.name)
    }

    async fn on_start(&mut self) -> Result<()> {
        let prepare = self.prepare.take().expect("task_with OnStart ran twice");
        let run = self.run.take().expect("task_with OnStart ran twice");
        let ready = prepare().await?;
        self.live
            .spawn_with_stop(move |stopped| run(ready, stopped));
        Ok(())
    }

    async fn on_stop(&mut self) -> Result<()> {
        self.live.on_stop().await
    }
}
