use std::future::Future;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use crate::error::{Error, Result};

use crate::error::aggregate_errors;
use crate::future::BoxFuture;

type Callback = Box<dyn FnOnce() -> BoxFuture<'static, Result<()>> + Send>;

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
///     fn name(&self) -> Option<&'static str> {
///         Some("http")
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
/// Hook futures must be cancellation-safe: a timeout drops the in-progress
/// future. Any task spawned by a hook must be tracked and shut down explicitly.
/// Panicking is considered a fatal programming error and is not converted into
/// [`Error`]; in particular, a panic during start may bypass lifecycle unwind.
pub trait Hook: Send + 'static {
    /// Label used in framework logs. Does not affect execution order.
    fn name(&self) -> Option<&'static str> {
        None
    }

    /// `false` when [`on_start`](Self::on_start) is a no-op.
    ///
    /// Stop-only hooks are marked active as soon as they are reached, so they
    /// still run if a later start hook fails.
    #[doc(hidden)]
    fn has_start(&self) -> bool {
        true
    }

    /// `false` when [`on_stop`](Self::on_stop) is a no-op.
    #[doc(hidden)]
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
}

/// Ad-hoc start/stop callbacks. Prefer implementing [`Hook`] when the two
/// phases share state.
#[derive(Default)]
pub struct HookFn {
    name: Option<&'static str>,
    on_start: Option<Callback>,
    on_stop: Option<Callback>,
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

impl HookFn {
    /// An empty hook. Attach callbacks with
    /// [`on_start`](Self::on_start) and [`on_stop`](Self::on_stop).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Label used in framework logs. Does not affect execution order.
    #[must_use]
    pub fn name(mut self, name: &'static str) -> Self {
        self.name = Some(name);
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
    /// Calling this twice on the same [`HookFn`] keeps the last callback.
    #[must_use]
    pub fn on_stop<F, Fut>(mut self, f: F) -> Self
    where
        F: FnOnce() -> Fut + Send + 'static,
        Fut: Future<Output = Result<()>> + Send + 'static,
    {
        self.on_stop = Some(Box::new(move || Box::pin(f())));
        self
    }
}

impl Hook for HookFn {
    fn name(&self) -> Option<&'static str> {
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
        let f = self.on_stop.take();
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
    name: Option<&'static str>,
    inner: Option<Box<dyn ErasedHook>>,
}

struct InflightHook {
    lifecycle: Option<Lifecycle>,
    idx: usize,
    name: Option<&'static str>,
    start: bool,
    finished: bool,
}

impl InflightHook {
    fn start(lifecycle: Lifecycle, idx: usize, name: Option<&'static str>) -> Self {
        crate::trace::on_start_executing(idx, name);
        Self {
            lifecycle: Some(lifecycle),
            idx,
            name,
            start: true,
            finished: false,
        }
    }

    fn stop(idx: usize, name: Option<&'static str>) -> Self {
        crate::trace::on_stop_executing(idx, name);
        Self {
            lifecycle: None,
            idx,
            name,
            start: false,
            finished: false,
        }
    }

    fn ok(&mut self, runtime: std::time::Duration) {
        self.finished = true;
        if self.start {
            crate::trace::on_start_executed(self.idx, self.name, runtime);
        } else {
            crate::trace::on_stop_executed(self.idx, self.name, runtime);
        }
    }

    fn fail(&mut self, err: &Error) {
        self.finished = true;
        if self.start {
            crate::trace::on_start_failed(self.idx, self.name, err);
        } else {
            crate::trace::on_stop_failed(self.idx, self.name, err);
        }
    }
}

impl Drop for InflightHook {
    fn drop(&mut self) {
        if !self.finished {
            if self.start {
                crate::trace::on_start_cancelled(self.idx, self.name);
                if let Some(lc) = self.lifecycle.take() {
                    lc.reject_append_and_activate_trailing();
                }
            } else {
                crate::trace::on_stop_cancelled(self.idx, self.name);
            }
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
    pub(crate) fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(State {
                hooks: Vec::new(),
                started: 0,
                phase: Phase::Registering,
            })),
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
    pub fn append<H: Hook>(&self, hook: H) -> Result<()> {
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
            let started_at = Instant::now();
            match hook.on_start().await {
                Ok(()) => {
                    inflight.ok(started_at.elapsed());
                    let mut state = self.state();
                    state.hooks[idx].inner = Some(hook);
                    state.started += 1;
                }
                Err(err) => {
                    inflight.fail(&err);
                    // Drop `hook` without putting it back — its OnStop must not run.
                    self.reject_append_and_activate_trailing();
                    return Err(err);
                }
            }
        }
    }

    fn reject_append_and_activate_trailing(&self) {
        let mut state = self.state();
        state.phase = Phase::Stopping;
        while state.started < state.hooks.len() {
            let skip = match state.hooks[state.started].inner.as_ref() {
                // In-flight or failed start: skip this slot without running stop.
                None => true,
                Some(hook) => !hook.has_start(),
            };
            if skip {
                state.started += 1;
            } else {
                break;
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
        while let Some((idx, name, mut hook)) = self.take_next_stop() {
            let mut inflight = InflightHook::stop(idx, name);
            let started_at = Instant::now();
            match hook.on_stop().await {
                Ok(()) => inflight.ok(started_at.elapsed()),
                Err(err) => {
                    inflight.fail(&err);
                    errors.push(err);
                }
            }
        }
        aggregate_errors(errors)
    }

    async fn stop_first(&self, count: usize) -> Result<()> {
        let mut errors = Vec::new();
        for _ in 0..count {
            if let Some((idx, name, mut hook)) = self.take_next_stop() {
                let mut inflight = InflightHook::stop(idx, name);
                let started_at = Instant::now();
                match hook.on_stop().await {
                    Ok(()) => inflight.ok(started_at.elapsed()),
                    Err(err) => {
                        inflight.fail(&err);
                        errors.push(err);
                    }
                }
            } else {
                break;
            }
        }
        aggregate_errors(errors)
    }

    /// Take the next stoppable hook among those that have started, in reverse
    /// order. Taking one at a time means a cancelled `stop` future still leaves
    /// remaining hooks available for a follow-up best-effort pass. The hook is
    /// taken under the lock and `on_stop` is invoked after releasing it, so user
    /// code may re-enter [`Lifecycle`].
    fn take_next_stop(&self) -> Option<(usize, Option<&'static str>, Box<dyn ErasedHook>)> {
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
