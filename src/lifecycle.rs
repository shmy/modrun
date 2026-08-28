use crate::error::{Error, Result};
use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::error::aggregate_errors;
use crate::future::BoxFuture;
use crate::shutdown::Shutdowner;

type Callback = Box<dyn FnOnce() -> BoxFuture<'static, Result<()>> + Send>;
type StopCallback = Arc<dyn Fn() -> BoxFuture<'static, Result<()>> + Send + Sync>;

/// Start/stop callbacks for a component.
///
/// Implement this on your own type and pass it to [`Lifecycle::append`].
/// Start and stop share the same instance, so runtime state (a listener, a
/// task handle) lives on `self` instead of being smuggled through channels.
///
/// ```
/// use modrun::{Hook, Result};
///
/// struct Server {
///     port: u16,
/// }
///
/// impl Hook for Server {
///     fn name(&self) -> &'static str {
///         "http"
///     }
///
///     async fn on_start(&mut self) -> Result<()> {
///         println!("listening on {}", self.port);
///         Ok(())
///     }
///
///     async fn on_stop(&mut self) -> Result<()> {
///         println!("goodbye");
///         Ok(())
///     }
/// }
/// ```
///
/// For one-off closures, use [`hook`]:
///
/// ```
/// # use modrun::hook;
/// let listener = String::from("127.0.0.1:3000");
/// let h = hook().on_start(move || async move {
///     println!("binding {listener}");
///     Ok(())
/// });
/// ```
///
/// For a background worker, use [`crate::task`]: OnStart spawns the work.
/// For a server that must bind before the app is running, use
/// [`crate::task_with`] so listen failures fail start. Both request
/// [`crate::Shutdowner::shutdown`] if the background future fails or panics,
/// so [`crate::ModrunBuilder::run`] does not wait forever. Custom hooks that
/// spawn their own tasks still need to call `shutdown()` themselves.
///
/// Hook futures must be cancellation-safe: a timeout drops the in-progress
/// future. Any task spawned by a hook must be tracked and shut down explicitly
/// — [`crate::task`] does this for you. Panicking is considered a fatal
/// programming error and is not converted into [`Error`]; in particular, a
/// panic during start may bypass lifecycle unwind. Tracing records the
/// in-flight phase as `panicked` rather than `cancelled`.
///
/// # Stop-only struct hooks
///
/// The default [`has_start`](Self::has_start) is `true`, so a struct that only
/// implements [`on_stop`](Self::on_stop) is still treated as needing OnStart.
/// Register it **after** a hook that fails OnStart and it will **not** be
/// activated for unwind unless you override `has_start`:
///
/// ```
/// # use modrun::{Hook, Result};
/// struct Metrics;
///
/// impl Hook for Metrics {
///     fn has_start(&self) -> bool {
///         false
///     }
///
///     async fn on_stop(&mut self) -> Result<()> {
///         println!("flush metrics");
///         Ok(())
///     }
/// }
/// ```
///
/// For one-off stop callbacks, [`hook().on_stop(...)`](crate::hook) already
/// behaves as stop-only.
pub trait Hook: Send + 'static {
    /// Label used in framework logs and hook failure messages. Does not affect
    /// execution order. Defaults to `"unnamed"`; override for clearer diagnostics.
    /// Avoid `"unnamed"` as an explicit override (indistinguishable from the
    /// default) and avoid empty strings.
    fn name(&self) -> &'static str {
        "unnamed"
    }

    /// Whether this hook runs an OnStart phase.
    ///
    /// Defaults to `true`. Override to `false` for stop-only struct hooks (see
    /// [trait-level notes](Self#stop-only-struct-hooks)).
    fn has_start(&self) -> bool {
        true
    }

    /// Whether this hook runs an OnStop phase.
    fn has_stop(&self) -> bool {
        true
    }

    /// Run while the application starts. Hooks start in registration order,
    /// and a failure unwinds the ones that already ran.
    fn on_start(&mut self) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    /// Run while the application stops, in reverse registration order.
    fn on_stop(&mut self) -> impl Future<Output = Result<()>> + Send {
        async { Ok(()) }
    }

    /// Receives the process [`Shutdowner`] when the hook is
    /// [`Lifecycle::append`]ed.
    ///
    /// [`crate::task`] uses this so a background failure unblocks
    /// [`crate::ModrunBuilder::run`]. Custom hooks that spawn their own work
    /// can store it the same way.
    fn attach_shutdown(&mut self, _shutdown: Shutdowner) {}
}

/// Ad-hoc start/stop callbacks. Prefer implementing [`Hook`] when the two
/// phases share state.
pub struct HookFn {
    name: &'static str,
    on_start: Option<Callback>,
    on_stop: Option<StopCallback>,
}

/// Build an ad-hoc [`Hook`] from closures.
#[must_use]
pub fn hook() -> HookFn {
    HookFn::new()
}

impl std::fmt::Debug for HookFn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HookFn")
            .field("name", &self.name)
            .field("on_start", &self.on_start.is_some())
            .field("on_stop", &self.on_stop.is_some())
            .finish()
    }
}

impl Default for HookFn {
    fn default() -> Self {
        Self::new()
    }
}

impl HookFn {
    /// An empty hook. Attach callbacks with
    /// [`on_start`](Self::on_start) and [`on_stop`](Self::on_stop).
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: "unnamed",
            on_start: None,
            on_stop: None,
        }
    }

    /// Label used in framework logs. Does not affect execution order.
    ///
    /// Must not be empty. Using `"unnamed"` is discouraged — it is the default
    /// when this method is not called.
    #[must_use]
    pub fn name(mut self, name: &'static str) -> Self {
        debug_assert!(!name.is_empty(), "hook name must not be empty");
        self.name = name;
        self
    }

    /// Run `f` while the application starts. Hooks start in registration order,
    /// and a failure unwinds the ones that already ran.
    ///
    /// Calling this twice on the same [`HookFn`] keeps the last callback.
    #[must_use]
    pub fn on_start<F, Fut>(mut self, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.on_start = Some(Box::new(move || Box::pin(f())));
        self
    }

    /// Run `f` while the application stops, in reverse registration order.
    ///
    /// A hook with OnStop but no OnStart is considered active immediately and
    /// also runs when a later start hook fails and the lifecycle unwinds.
    ///
    /// `f` must be callable more than once (`Fn`, not `FnOnce`) so an in-flight
    /// OnStop can be retried after cancellation. Capture shared state with
    /// [`Arc`](std::sync::Arc) and clone it inside `f` when needed.
    ///
    /// Calling this twice on the same [`HookFn`] keeps the last callback.
    #[must_use]
    pub fn on_stop<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        let f = Arc::new(f);
        self.on_stop = Some(Arc::new(move || Box::pin(f())));
        self
    }
}

impl Hook for HookFn {
    fn name(&self) -> &'static str {
        self.name
    }

    fn has_start(&self) -> bool {
        self.on_start.is_some()
    }

    fn has_stop(&self) -> bool {
        self.on_stop.is_some()
    }

    fn on_start(&mut self) -> impl Future<Output = Result<()>> + Send {
        let f = self.on_start.take();
        async move {
            match f {
                Some(f) => f().await,
                None => Ok(()),
            }
        }
    }

    fn on_stop(&mut self) -> impl Future<Output = Result<()>> + Send {
        let f = self.on_stop.clone();
        async move {
            match f {
                Some(f) => f().await,
                None => Ok(()),
            }
        }
    }
}

trait ErasedHook: Send {
    fn has_start(&self) -> bool;
    fn has_stop(&self) -> bool;
    fn on_start(&mut self) -> BoxFuture<'_, Result<()>>;
    fn on_stop(&mut self) -> BoxFuture<'_, Result<()>>;
}

impl<H: Hook> ErasedHook for H {
    fn has_start(&self) -> bool {
        Hook::has_start(self)
    }

    fn has_stop(&self) -> bool {
        Hook::has_stop(self)
    }

    fn on_start(&mut self) -> BoxFuture<'_, Result<()>> {
        Box::pin(Hook::on_start(self))
    }

    fn on_stop(&mut self) -> BoxFuture<'_, Result<()>> {
        Box::pin(Hook::on_stop(self))
    }
}

struct HookEntry {
    name: &'static str,
    inner: Option<Box<dyn ErasedHook>>,
}

struct InflightHook {
    lifecycle: Option<Lifecycle>,
    idx: usize,
    name: &'static str,
    finished: bool,
}

impl InflightHook {
    fn start(lifecycle: Lifecycle, idx: usize, name: &'static str) -> Self {
        crate::trace::on_start_executing(idx, name);
        Self {
            lifecycle: Some(lifecycle),
            idx,
            name,
            finished: false,
        }
    }

    fn ok(&mut self, runtime: std::time::Duration) {
        self.finished = true;
        crate::trace::on_start_executed(self.idx, self.name, runtime);
    }

    fn fail(&mut self, err: &Error) {
        self.finished = true;
        crate::trace::on_start_failed(self.idx, self.name, err);
    }
}

impl Drop for InflightHook {
    fn drop(&mut self) {
        crate::trace::emit_unfinished(
            self.finished,
            || crate::trace::on_start_panicked(self.idx, self.name),
            || crate::trace::on_start_cancelled(self.idx, self.name),
        );
        if !self.finished {
            if let Some(lc) = self.lifecycle.take() {
                lc.prepare_for_unwind();
            }
        }
    }
}

/// Holds a hook taken for OnStop; if the future is cancelled before completion,
/// the hook is written back so a follow-up unwind pass can retry it.
struct StopGuard {
    lifecycle: Lifecycle,
    idx: usize,
    name: &'static str,
    hook: Option<Box<dyn ErasedHook>>,
    finished: bool,
}

impl StopGuard {
    fn new(
        lifecycle: Lifecycle,
        idx: usize,
        name: &'static str,
        hook: Box<dyn ErasedHook>,
    ) -> Self {
        crate::trace::on_stop_executing(idx, name);
        Self {
            lifecycle,
            idx,
            name,
            hook: Some(hook),
            finished: false,
        }
    }

    async fn run(mut self) -> Result<()> {
        let started_at = crate::trace::start_timer();
        let result = {
            let hook = self.hook.as_mut().expect("hook present");
            match hook.on_stop().await {
                Ok(()) => Ok(()),
                Err(err) => Err(err.with_hook_name(self.name)),
            }
        };
        match &result {
            Ok(()) => crate::trace::on_stop_executed(
                self.idx,
                self.name,
                crate::trace::elapsed(started_at),
            ),
            Err(err) => crate::trace::on_stop_failed(self.idx, self.name, err),
        }
        self.finished = true;
        self.hook = None;
        result
    }
}

impl Drop for StopGuard {
    fn drop(&mut self) {
        if !self.finished {
            if let Some(hook) = self.hook.take() {
                let mut state = self.lifecycle.state();
                state.hooks[self.idx].inner = Some(hook);
                state.started += 1;
            }
            crate::trace::emit_unfinished(
                false,
                || crate::trace::on_stop_panicked(self.idx, self.name),
                || crate::trace::on_stop_cancelled(self.idx, self.name),
            );
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    /// Invoke / registration window.
    Registering,
    /// [`Lifecycle::start`] is running; OnStart factories may still append.
    Starting,
    /// Start finished successfully; further appends are rejected.
    Started,
    /// Stop or unwind is in progress (or finished); appends are rejected.
    Stopping,
}

struct State {
    hooks: Vec<HookEntry>,
    /// Hooks that have finished starting (including those with no OnStart).
    /// Stop runs only over this prefix, in reverse.
    started: usize,
    phase: Phase,
}

/// Application lifecycle registry.
///
/// Injected as a cloneable handle. Hooks run in registration order on start,
/// and in reverse order on stop.
#[derive(Clone)]
pub struct Lifecycle {
    inner: Arc<Mutex<State>>,
    shutdown: Shutdowner,
}

impl std::fmt::Debug for Lifecycle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state();
        f.debug_struct("Lifecycle")
            .field("phase", &state.phase)
            .field("hooks", &state.hooks.len())
            .field("started", &state.started)
            .finish()
    }
}

impl Lifecycle {
    pub(crate) fn new(shutdown: Shutdowner) -> Self {
        Self {
            inner: Arc::new(Mutex::new(State {
                hooks: Vec::new(),
                started: 0,
                phase: Phase::Registering,
            })),
            shutdown,
        }
    }

    /// A poisoned lifecycle means some other thread panicked while registering a
    /// hook. The hooks already recorded are still valid, so recover rather than
    /// turning an unrelated panic into a second one.
    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Append a lifecycle hook.
    ///
    /// Register hooks during invoke, or from an OnStart factory that runs before
    /// start finishes. Returns an error if start has already completed or stop
    /// has begun — those hooks would never run.
    ///
    /// # Errors
    ///
    /// Returns an error when start has already finished or stop/unwind has
    /// begun — appended hooks would never run.
    pub fn append<H: Hook>(&self, mut hook: H) -> Result<()> {
        hook.attach_shutdown(self.shutdown.clone());
        debug_assert!(!hook.name().is_empty(), "hook name must not be empty");
        let mut state = self.state();
        match state.phase {
            Phase::Registering | Phase::Starting => {
                state.hooks.push(HookEntry {
                    name: hook.name(),
                    inner: Some(Box::new(hook)),
                });
                Ok(())
            }
            Phase::Started => Err(Error::AppendAfterStart),
            Phase::Stopping => Err(Error::AppendWhileStopping),
        }
    }

    /// Run OnStart hooks in registration order.
    ///
    /// On failure or cancellation the caller is responsible for
    /// [`unwind_started`](Self::unwind_started) — this method does not stop
    /// hooks itself, so the outer timeout budget can cover cleanup.
    ///
    /// A start hook that returns `Err` (or is cancelled mid-flight) does **not**
    /// run its own OnStop; only hooks that finished OnStart are unwound.
    pub(crate) async fn start(&self) -> Result<()> {
        {
            let mut state = self.state();
            state.phase = Phase::Starting;
        }

        // `start_inner` changes Starting → Started while holding the same lock
        // used by `append`, so no hook can slip into the gap after the final
        // empty check. On error or cancellation the phase becomes Stopping so
        // concurrent appends cannot register hooks that would never run.
        self.start_inner().await
    }

    async fn start_inner(&self) -> Result<()> {
        loop {
            // Under one lock: skip empty OnStarts and take at most one real
            // callback so append can still re-enter between awaits.
            let start = {
                let mut state = self.state();
                loop {
                    if state.started >= state.hooks.len() {
                        state.phase = Phase::Started;
                        break None;
                    }
                    let idx = state.started;
                    let skip = match state.hooks[idx].inner.as_ref() {
                        Some(hook) => !hook.has_start(),
                        None => true,
                    };
                    if skip {
                        state.started += 1;
                        continue;
                    }
                    let hook = state.hooks[idx]
                        .inner
                        .take()
                        .expect("hook with OnStart present");
                    let name = state.hooks[idx].name;
                    break Some((idx, name, hook));
                }
            };

            let Some((idx, name, mut hook)) = start else {
                return Ok(());
            };

            let mut inflight = InflightHook::start(self.clone(), idx, name);
            let started_at = crate::trace::start_timer();
            match hook.on_start().await {
                Ok(()) => {
                    inflight.ok(crate::trace::elapsed(started_at));
                    let mut state = self.state();
                    state.hooks[idx].inner = Some(hook);
                    state.started += 1;
                }
                Err(err) => {
                    let err = err.with_hook_name(name);
                    inflight.fail(&err);
                    // Drop `hook` without putting it back — its OnStop must not run.
                    // Same activation as cancel/timeout: remaining stop-only
                    // hooks run even if a later start hook never ran.
                    self.prepare_for_unwind();
                    return Err(err);
                }
            }
        }
    }

    /// Mark every remaining stop-only hook as started and drop hooks that never
    /// ran OnStart, so a build/start cancel or failure can unwind stop-only
    /// hooks even when they sit after a start hook that did not run.
    pub(crate) fn prepare_for_unwind(&self) {
        let mut state = self.state();
        state.phase = Phase::Stopping;
        while state.started < state.hooks.len() {
            match state.hooks[state.started].inner.as_ref() {
                None => {
                    state.started += 1;
                }
                Some(hook) if !hook.has_start() => {
                    state.started += 1;
                }
                Some(_) => {
                    let idx = state.started;
                    state.hooks[idx].inner = None;
                    state.started += 1;
                }
            }
        }
    }

    pub(crate) fn pending_stops(&self) -> usize {
        self.state().started
    }

    /// Run OnStop for hooks that already started. Used when start fails or is
    /// cancelled mid-flight (timeout or shutdown).
    pub(crate) async fn unwind_started(&self) -> Result<()> {
        let started = {
            let mut state = self.state();
            state.phase = Phase::Stopping;
            state.started
        };
        self.stop_first(started).await
    }

    pub(crate) async fn stop(&self) -> Result<()> {
        {
            let mut state = self.state();
            state.phase = Phase::Stopping;
        }
        let mut errors = Vec::new();
        while let Some((idx, name, hook)) = self.take_next_stop() {
            match StopGuard::new(self.clone(), idx, name, hook).run().await {
                Ok(()) => {}
                Err(err) => errors.push(err),
            }
        }
        aggregate_errors(errors)
    }

    async fn stop_first(&self, count: usize) -> Result<()> {
        let mut errors = Vec::new();
        for _ in 0..count {
            if let Some((idx, name, hook)) = self.take_next_stop() {
                match StopGuard::new(self.clone(), idx, name, hook).run().await {
                    Ok(()) => {}
                    Err(err) => errors.push(err),
                }
            } else {
                break;
            }
        }
        aggregate_errors(errors)
    }

    /// Take the next stoppable hook among those that have started, in reverse
    /// order. The hook is taken under the lock and `on_stop` runs after releasing
    /// it. If that future is cancelled, [`StopGuard`] writes the hook back so a
    /// follow-up unwind can retry it; a timeout abandons the in-flight hook without
    /// a second budget (see caller).
    fn take_next_stop(&self) -> Option<(usize, &'static str, Box<dyn ErasedHook>)> {
        let mut state = self.state();
        loop {
            if state.started == 0 {
                return None;
            }
            state.started -= 1;
            let idx = state.started;
            let Some(hook) = state.hooks[idx].inner.take() else {
                continue;
            };
            if !hook.has_stop() {
                continue;
            }
            return Some((idx, state.hooks[idx].name, hook));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shutdown::Shutdowner;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    #[tokio::test]
    async fn stop_guard_writes_back_cancelled_on_stop() {
        let lc = Lifecycle::new(Shutdowner::new());
        let log = Arc::new(Mutex::new(Vec::new()));
        let l = Arc::clone(&log);
        let l2 = Arc::clone(&l);
        lc.append(hook().on_stop(move || {
            let l = Arc::clone(&l2);
            async move {
                l.lock().unwrap().push("begin");
                tokio::time::sleep(Duration::from_millis(200)).await;
                l.lock().unwrap().push("end");
                Ok(())
            }
        }))
        .unwrap();
        lc.prepare_for_unwind();

        let lc2 = lc.clone();
        tokio::select! {
            _ = lc.unwind_started() => {}
            _ = tokio::time::sleep(Duration::from_millis(20)) => {}
        }
        assert_eq!(log.lock().unwrap().as_slice(), ["begin"]);

        lc2.unwind_started().await.unwrap();
        assert_eq!(log.lock().unwrap().as_slice(), ["begin", "begin", "end"]);
    }

    #[tokio::test]
    async fn prepare_for_unwind_runs_stop_only_after_unstarted_hooks() {
        let lc = Lifecycle::new(Shutdowner::new());
        let log = Arc::new(Mutex::new(Vec::new()));
        let started = Arc::clone(&log);
        let stopped = Arc::clone(&log);
        lc.append(
            hook()
                .on_start(move || {
                    let started = Arc::clone(&started);
                    async move {
                        started.lock().unwrap().push("start");
                        Ok(())
                    }
                })
                .on_stop(move || {
                    let stopped = Arc::clone(&stopped);
                    async move {
                        stopped.lock().unwrap().push("stop-started");
                        Ok(())
                    }
                }),
        )
        .unwrap();
        let only = Arc::clone(&log);
        lc.append(hook().on_stop(move || {
            let only = Arc::clone(&only);
            async move {
                only.lock().unwrap().push("stop-only");
                Ok(())
            }
        }))
        .unwrap();

        lc.prepare_for_unwind();
        lc.unwind_started().await.unwrap();
        assert_eq!(log.lock().unwrap().as_slice(), ["stop-only"]);
    }

    #[tokio::test]
    async fn failed_start_runs_stop_only_after_unstarted_hooks() {
        let lc = Lifecycle::new(Shutdowner::new());
        let log = Arc::new(Mutex::new(Vec::new()));
        lc.append(hook().on_start(|| async { Err(Error::hook("boom")) }))
            .unwrap();
        lc.append(hook().on_start(|| async {
            panic!("later start hook must not run after failed start");
        }))
        .unwrap();
        let only = Arc::clone(&log);
        lc.append(hook().on_stop(move || {
            let only = Arc::clone(&only);
            async move {
                only.lock().unwrap().push("stop-only");
                Ok(())
            }
        }))
        .unwrap();

        let err = lc.start().await.unwrap_err();
        assert!(format!("{err}").contains("boom"), "unexpected: {err}");
        lc.unwind_started().await.unwrap();
        assert_eq!(log.lock().unwrap().as_slice(), ["stop-only"]);
    }
}
