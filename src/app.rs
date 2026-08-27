use std::time::Duration;

use crate::container::{Container, seed_builtins};
use crate::error::{Error, Result, combine_results, with_cleanup};
use crate::invoke::ScopedInvoker;
use crate::lifecycle::Lifecycle;
use crate::module::Module;
use crate::option::ModOption;
use crate::scope::ScopeId;
use crate::shutdown::Shutdowner;
use crate::timeout::DEFAULT_TIMEOUT;
use crate::wiring::impl_wiring_methods;

/// Entry point for configuring an application: [`Modrun::builder`].
#[derive(Debug)]
pub struct Modrun;

/// Fluent builder for a modrun application.
///
/// ```
/// # use modrun::Modrun;
/// #[derive(Clone)]
/// struct Config { port: u16 }
/// #[derive(Clone)]
/// struct Server;
///
/// fn new_server(_cfg: Config) -> Server { Server }
/// fn boot(_server: Server) {}
///
/// # #[tokio::main]
/// # async fn main() -> modrun::Result<()> {
/// Modrun::builder()
///     .supply(Config { port: 8080 })
///     .provide(new_server)
///     .invoke(boot)
///     .start()
///     .await?
///     .stop()
///     .await
/// # }
/// ```
pub struct ModrunBuilder {
    options: Vec<Box<dyn ModOption>>,
    banner: crate::banner::Banner,
    build_timeout: Option<Duration>,
    start_timeout: Option<Duration>,
    stop_timeout: Option<Duration>,
}

impl Default for ModrunBuilder {
    fn default() -> Self {
        Self {
            options: Vec::new(),
            banner: crate::banner::Banner::default(),
            build_timeout: Some(DEFAULT_TIMEOUT),
            start_timeout: Some(DEFAULT_TIMEOUT),
            stop_timeout: Some(DEFAULT_TIMEOUT),
        }
    }
}

impl std::fmt::Debug for ModrunBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModrunBuilder")
            .field("options", &self.options.len())
            .field("banner", &self.banner)
            .field("build_timeout", &self.build_timeout)
            .field("start_timeout", &self.start_timeout)
            .field("stop_timeout", &self.stop_timeout)
            .finish()
    }
}

impl Modrun {
    /// Start configuring an application.
    #[must_use]
    pub fn builder() -> ModrunBuilder {
        ModrunBuilder::default()
    }
}

impl ModrunBuilder {
    fn push_option(&mut self, option: Box<dyn ModOption>) {
        self.options.push(option);
    }

    /// Install a domain [`Module`].
    #[must_use]
    pub fn module(mut self, module: Module) -> Self {
        self.module_mut(module);
        self
    }

    /// [`module`](Self::module) for `&mut self`.
    pub fn module_mut(&mut self, module: Module) -> &mut Self {
        self.push_option(module.into_option());
        self
    }

    /// Total budget for graph construction (async constructors and invokers).
    ///
    /// Timeouts are cooperative: work that yields at `.await` is cancelled when
    /// the budget expires. Synchronous blocking (for example `std::thread::sleep`
    /// inside a sync invoker or constructor) cannot be preempted; after it
    /// returns, an over-budget success is still reported as
    /// [`Error::BuildTimeout`](crate::Error::BuildTimeout).
    ///
    /// [`Shutdowner`](crate::Shutdowner) and OS signals during [`run`](Self::run)
    /// use the same cooperative rule: they cancel the current phase at its next
    /// `.await`, so a shutdown from a synchronous OnStart does not skip later
    /// hooks that have not yet yielded.
    ///
    /// The cancellation timer follows Tokio's clock; the over-budget `Ok` check
    /// uses wall-clock [`std::time::Instant`]. Pausing Tokio time in tests
    /// (`tokio::time::pause`) can make those two disagree.
    ///
    /// If set more than once, the last value wins. [`no_build_timeout`](Self::no_build_timeout)
    /// disables the budget.
    #[must_use]
    pub fn build_timeout(mut self, duration: Duration) -> Self {
        self.build_timeout_mut(duration);
        self
    }

    /// [`build_timeout`](Self::build_timeout) for `&mut self`.
    pub fn build_timeout_mut(&mut self, duration: Duration) -> &mut Self {
        self.build_timeout = Some(duration);
        self
    }

    /// Do not bound graph construction.
    #[must_use]
    pub fn no_build_timeout(mut self) -> Self {
        self.no_build_timeout_mut();
        self
    }

    /// [`no_build_timeout`](Self::no_build_timeout) for `&mut self`.
    pub fn no_build_timeout_mut(&mut self) -> &mut Self {
        self.build_timeout = None;
        self
    }

    /// Total budget for all OnStart hooks.
    ///
    /// Same cooperative semantics as [`build_timeout`](Self::build_timeout):
    /// blocking work is not preempted, but an over-budget `Ok` is still turned
    /// into [`Error::StartTimeout`](crate::Error::StartTimeout). Unwind after a
    /// failed or cancelled start is budgeted by [`stop_timeout`](Self::stop_timeout).
    ///
    /// If set more than once, the last value wins. [`no_start_timeout`](Self::no_start_timeout)
    /// disables the budget.
    #[must_use]
    pub fn start_timeout(mut self, duration: Duration) -> Self {
        self.start_timeout_mut(duration);
        self
    }

    /// [`start_timeout`](Self::start_timeout) for `&mut self`.
    pub fn start_timeout_mut(&mut self, duration: Duration) -> &mut Self {
        self.start_timeout = Some(duration);
        self
    }

    /// Do not bound OnStart. Unwind after a failed/cancelled start still uses
    /// [`stop_timeout`](Self::stop_timeout).
    #[must_use]
    pub fn no_start_timeout(mut self) -> Self {
        self.no_start_timeout_mut();
        self
    }

    /// [`no_start_timeout`](Self::no_start_timeout) for `&mut self`.
    pub fn no_start_timeout_mut(&mut self) -> &mut Self {
        self.start_timeout = None;
        self
    }

    /// Total budget for all OnStop hooks (including unwind after a failed or
    /// cancelled start).
    ///
    /// Same cooperative semantics as [`build_timeout`](Self::build_timeout):
    /// blocking OnStop work is not preempted, but an over-budget `Ok` is still
    /// turned into [`Error::StopTimeout`](crate::Error::StopTimeout) /
    /// [`Error::UnwindTimeout`](crate::Error::UnwindTimeout).
    ///
    /// If set more than once, the last value wins. [`no_stop_timeout`](Self::no_stop_timeout)
    /// disables the budget.
    #[must_use]
    pub fn stop_timeout(mut self, duration: Duration) -> Self {
        self.stop_timeout_mut(duration);
        self
    }

    /// [`stop_timeout`](Self::stop_timeout) for `&mut self`.
    pub fn stop_timeout_mut(&mut self, duration: Duration) -> &mut Self {
        self.stop_timeout = Some(duration);
        self
    }

    /// Do not bound OnStop / unwind.
    #[must_use]
    pub fn no_stop_timeout(mut self) -> Self {
        self.no_stop_timeout_mut();
        self
    }

    /// [`no_stop_timeout`](Self::no_stop_timeout) for `&mut self`.
    pub fn no_stop_timeout_mut(&mut self) -> &mut Self {
        self.stop_timeout = None;
        self
    }

    /// Replace the default modrun banner with custom text (for example
    /// `include_str!("banner.txt")`).
    ///
    /// Printed to stdout once at the beginning of [`run`](Self::run) or
    /// [`start`](Self::start), before framework tracing events.
    #[must_use]
    pub fn banner(mut self, text: impl Into<std::borrow::Cow<'static, str>>) -> Self {
        self.banner_mut(text);
        self
    }

    /// [`banner`](Self::banner) for `&mut self`.
    pub fn banner_mut(&mut self, text: impl Into<std::borrow::Cow<'static, str>>) -> &mut Self {
        self.banner = crate::banner::Banner::Custom(text.into());
        self
    }

    /// Do not print a startup banner.
    #[must_use]
    pub fn no_banner(mut self) -> Self {
        self.no_banner_mut();
        self
    }

    /// [`no_banner`](Self::no_banner) for `&mut self`.
    pub fn no_banner_mut(&mut self) -> &mut Self {
        self.banner = crate::banner::Banner::Off;
        self
    }

    fn print_banner(&self) {
        crate::banner::emit(&self.banner);
    }

    /// Build → start hooks → wait for shutdown → stop.
    ///
    /// When the `signal` crate feature is enabled (on by default), OS signal
    /// listeners are installed from the beginning of this call on **Unix**
    /// (Ctrl-C / SIGTERM) and **Windows** (Ctrl-C / Ctrl-Break / Ctrl-Close /
    /// Ctrl-Shutdown). Other targets ignore the feature for OS signals and rely
    /// on [`Shutdowner`](crate::Shutdowner) only. [`start`](Self::start) does
    /// not install signal handlers.
    ///
    /// With `default-features = false`, only [`Shutdowner`](crate::Shutdowner)
    /// unblocks this future.
    ///
    /// Signal listeners are owned by this future and dropped when it returns —
    /// calling `run` repeatedly on the same runtime does not accumulate tasks.
    ///
    /// A shutdown request or OS signal during build or start cancels that phase
    /// and unwinds hooks that already started, plus any stop-only hooks already
    /// registered (even if OnStart never ran). If cleanup succeeds, `run`
    /// returns `Ok(())` so process managers treat it as a graceful stop rather
    /// than a crash. A timeout, hook failure, or background [`crate::task`]
    /// failure still returns an error — a concurrent graceful shutdown does not
    /// turn that failure into `Ok(())`.
    ///
    /// Cancellation is cooperative (same as [`build_timeout`](Self::build_timeout)):
    /// the in-flight future is dropped at its next `.await`. After start
    /// succeeds, this future waits until a signal or [`Shutdowner::shutdown`].
    /// A background [`crate::task`] that fails or panics requests shutdown on
    /// its own. Custom work spawned with [`tokio::spawn`] must still call
    /// [`Shutdowner::shutdown`] or `run` waits forever for an OS signal.
    ///
    /// # Errors
    ///
    /// Returns an error when wiring/validation fails, a start or stop hook
    /// fails, a phase times out, or OS signal listeners cannot be installed.
    ///
    /// # Panics
    ///
    /// A panic in a constructor, invoker, or hook is a programming error and is
    /// not converted into [`Error`]. Tracing records the in-flight phase as
    /// `panicked`, but lifecycle unwind may not run.
    pub async fn run(self) -> Result<()> {
        self.print_banner();
        let shutdown = Shutdowner::new();
        let lifecycle = Lifecycle::new(shutdown.clone());
        let mut signals = SignalWatch::install()?;

        let mut state = self.into_build_state(lifecycle.clone(), shutdown.clone())?;
        let stop_timeout = state.stop_timeout;

        let app = tokio::select! {
            biased;
            result = run_invokers(&mut state) => match result {
                Ok(()) => built_app_from_state(state),
                Err(e) => {
                    return finish_run(
                        Err(e),
                        unwind_registered(&lifecycle, stop_timeout).await,
                    );
                }
            },
            _ = shutdown.wait() => {
                crate::trace::shutdown_requested();
                return finish_interrupt(
                    &shutdown,
                    unwind_registered(&lifecycle, stop_timeout).await,
                );
            }
            signal = signals.recv() => {
                crate::trace::received_signal(signal);
                shutdown.shutdown();
                crate::trace::shutdown_requested();
                return finish_run(
                    Ok(()),
                    unwind_registered(&lifecycle, stop_timeout).await,
                );
            }
        };

        // Race only OnStart. Unwind runs afterwards so a shutdown cannot
        // swallow a start error that is already in hand.
        let started = tokio::select! {
            biased;
            result = app.start_hooks() => result,
            _ = shutdown.wait() => {
                crate::trace::shutdown_requested();
                crate::trace::rolling_back_after_shutdown();
                return finish_interrupt(&shutdown, graceful_unwind(&app).await);
            }
            signal = signals.recv() => {
                crate::trace::received_signal(signal);
                shutdown.shutdown();
                crate::trace::shutdown_requested();
                crate::trace::rolling_back_after_shutdown();
                return finish_run(Ok(()), graceful_unwind(&app).await);
            }
        };

        match started {
            Ok(()) => crate::trace::started(),
            Err(err) => {
                crate::trace::rolling_back(&err);
                let cleanup = graceful_unwind(&app).await;
                crate::trace::start_failed(&err);
                return finish_run(Err(err), cleanup);
            }
        }

        tokio::select! {
            _ = shutdown.wait() => {
                crate::trace::shutdown_requested();
            }
            signal = signals.recv() => {
                crate::trace::received_signal(signal);
                shutdown.shutdown();
                crate::trace::shutdown_requested();
            }
        }
        app.stop().await
    }

    /// Build and start without waiting for a shutdown signal (useful in tests).
    ///
    /// Does not install Ctrl-C / SIGTERM handlers; call [`RunningApp::stop`]
    /// yourself (or use [`ModrunBuilder::run`](Self::run) for signal-driven stop).
    ///
    /// A background [`crate::task`] that fails after its OnStart has returned
    /// does **not** cancel remaining start hooks or fail this call. Poll
    /// [`Shutdowner::is_requested`] / [`Shutdowner::wait`], or call
    /// [`RunningApp::stop`], to observe it. [`run`](Self::run) tears the
    /// process down when a worker fails.
    ///
    /// # Errors
    ///
    /// Returns an error when wiring/validation fails, graph construction times
    /// out, a start hook fails, or start times out (including cleanup failures
    /// while unwinding).
    ///
    /// # Panics
    ///
    /// Same as [`run`](Self::run): a panic in a constructor, invoker, or hook is
    /// not converted into [`Error`] and may skip lifecycle unwind.
    pub async fn start(self) -> Result<RunningApp> {
        self.print_banner();
        let shutdown = Shutdowner::new();
        let lifecycle = Lifecycle::new(shutdown.clone());
        let mut state = self.into_build_state(lifecycle.clone(), shutdown)?;
        let stop_timeout = state.stop_timeout;
        if let Err(e) = run_invokers(&mut state).await {
            return Err(with_cleanup(
                e,
                unwind_registered(&lifecycle, stop_timeout).await,
            ));
        }
        let app = built_app_from_state(state);
        app.start().await?;
        Ok(RunningApp { inner: Some(app) })
    }

    fn into_build_state(self, lifecycle: Lifecycle, shutdown: Shutdowner) -> Result<BuildState> {
        let mut state = BuildState {
            container: Container::new(),
            invokers: Vec::new(),
            lifecycle,
            build_timeout: self.build_timeout,
            start_timeout: self.start_timeout,
            stop_timeout: self.stop_timeout,
            current_scope: ScopeId::ROOT,
        };
        seed_builtins(&mut state.container, state.lifecycle.clone(), shutdown)?;

        for opt in self.options {
            opt.apply(&mut state)?;
        }

        let invoker_deps: Vec<_> = state
            .invokers
            .iter()
            .map(|scoped| (scoped.scope, scoped.invoker.dep_types()))
            .collect();
        state.container.validate(&invoker_deps)?;

        Ok(state)
    }
}

async fn run_invokers(state: &mut BuildState) -> Result<()> {
    let invokers = std::mem::take(&mut state.invokers);
    let budget = state.build_timeout;
    let invoke = async {
        for scoped in invokers {
            let ScopedInvoker { scope, invoker } = scoped;
            let function = invoker.name();
            let deps = invoker.dep_list();
            let module = state.container.scopes().name(scope);
            crate::trace::invoking(function, deps.as_slice(), module);
            let mut invoke_guard = InflightInvoke::new(function, module);
            let previous = state.container.enter_scope(scope);
            let result = async {
                if !deps.as_slice().is_empty() {
                    state.container.ensure_built(deps.as_slice()).await?;
                }
                match invoker.call(&state.container) {
                    crate::invoke::InvokeOut::Done(r) => r,
                    crate::invoke::InvokeOut::Fut(fut) => fut.await,
                }
            }
            .await;
            state.container.leave_scope(previous);
            invoke_guard.finish();
            if let Err(ref err) = result {
                crate::trace::invoke_failed(function, deps.as_slice(), module, err);
            }
            result?;
        }
        Ok(())
    };
    with_timeout(
        budget,
        invoke,
        Error::BuildTimeout(budget.unwrap_or(Duration::ZERO)),
    )
    .await
}

struct InflightInvoke {
    function: &'static str,
    module: &'static str,
    finished: bool,
}

impl InflightInvoke {
    fn new(function: &'static str, module: &'static str) -> Self {
        Self {
            function,
            module,
            finished: false,
        }
    }

    fn finish(&mut self) {
        self.finished = true;
    }
}

impl Drop for InflightInvoke {
    fn drop(&mut self) {
        crate::trace::emit_unfinished(
            self.finished,
            || crate::trace::invoke_panicked(self.function, self.module),
            || crate::trace::invoke_cancelled(self.function, self.module),
        );
    }
}

fn built_app_from_state(state: BuildState) -> BuiltApp {
    BuiltApp {
        lifecycle: state.lifecycle,
        start_timeout: state.start_timeout,
        stop_timeout: state.stop_timeout,
    }
}

fn finish_run(phase: Result<()>, cleanup: Result<()>) -> Result<()> {
    match phase {
        Ok(()) => cleanup,
        Err(e) => Err(with_cleanup(e, cleanup)),
    }
}

/// [`Shutdowner::shutdown`] during build/start is a graceful `Ok(())`.
/// A background [`crate::task`] failure is not: keep a specific join error,
/// or [`Error::TaskFailedDuringStart`]. An unwind timeout still retains that
/// failure as [`Error::CleanupAfterFailure`].
fn finish_interrupt(shutdown: &Shutdowner, cleanup: Result<()>) -> Result<()> {
    if shutdown.is_failure() {
        match cleanup {
            Ok(()) => Err(Error::TaskFailedDuringStart),
            Err(e) if matches!(e, Error::UnwindTimeout(_) | Error::StopTimeout(_)) => {
                Err(with_cleanup(Error::TaskFailedDuringStart, Err(e)))
            }
            Err(e) => Err(e),
        }
    } else {
        cleanup
    }
}

fn report_unwind_result(lifecycle: &Lifecycle, result: Result<()>) -> Result<()> {
    match result {
        Ok(()) => {
            crate::trace::rolled_back();
            Ok(())
        }
        Err(err) => {
            crate::trace::rollback_failed(&err);
            let leftover = lifecycle.pending_stops();
            if leftover > 0 {
                crate::trace::hooks_abandoned(leftover);
            }
            Err(err)
        }
    }
}

async fn unwind_lifecycle(lifecycle: &Lifecycle, stop_timeout: Option<Duration>) -> Result<()> {
    lifecycle.prepare_for_unwind();
    report_unwind_result(
        lifecycle,
        with_timeout(
            stop_timeout,
            lifecycle.unwind_started(),
            Error::UnwindTimeout(stop_timeout.unwrap_or(Duration::ZERO)),
        )
        .await,
    )
}

async fn unwind_registered(lifecycle: &Lifecycle, stop_timeout: Option<Duration>) -> Result<()> {
    unwind_lifecycle(lifecycle, stop_timeout).await
}

impl_wiring_methods!(ModrunBuilder);

/// Mutable state while options are applied. Formerly `AppBuilder`.
pub(crate) struct BuildState {
    pub(crate) container: Container,
    pub(crate) invokers: Vec<ScopedInvoker>,
    pub(crate) lifecycle: Lifecycle,
    pub(crate) build_timeout: Option<Duration>,
    pub(crate) start_timeout: Option<Duration>,
    pub(crate) stop_timeout: Option<Duration>,
    pub(crate) current_scope: ScopeId,
}

struct BuiltApp {
    lifecycle: Lifecycle,
    start_timeout: Option<Duration>,
    stop_timeout: Option<Duration>,
}

async fn graceful_unwind(app: &BuiltApp) -> Result<()> {
    unwind_lifecycle(&app.lifecycle, app.stop_timeout).await
}

async fn with_timeout(
    budget: Option<Duration>,
    fut: impl std::future::Future<Output = Result<()>>,
    timed_out: Error,
) -> Result<()> {
    match budget {
        None => fut.await,
        Some(d) => {
            let started = std::time::Instant::now();
            match tokio::time::timeout(d, fut).await {
                // Prefer a real phase error over a timeout.
                Ok(Err(err)) => Err(err),
                // Sync blocking can prevent the timer from firing until the
                // work returns; treat an over-budget Ok as a timeout.
                Ok(Ok(())) if started.elapsed() >= d => Err(timed_out),
                Ok(Ok(())) => Ok(()),
                Err(_) => Err(timed_out),
            }
        }
    }
}

impl BuiltApp {
    /// OnStart only. The caller is responsible for unwind on failure or cancel.
    async fn start_hooks(&self) -> Result<()> {
        with_timeout(
            self.start_timeout,
            self.lifecycle.start(),
            Error::StartTimeout(self.start_timeout.unwrap_or(Duration::ZERO)),
        )
        .await
    }

    async fn start(&self) -> Result<()> {
        match self.start_hooks().await {
            Ok(()) => {
                crate::trace::started();
                Ok(())
            }
            Err(err) => {
                crate::trace::rolling_back(&err);
                let unwound = graceful_unwind(self).await;
                crate::trace::start_failed(&err);
                combine_results(Err(err), unwound)
            }
        }
    }

    async fn stop(&self) -> Result<()> {
        let result = with_timeout(
            self.stop_timeout,
            self.lifecycle.stop(),
            Error::StopTimeout(self.stop_timeout.unwrap_or(Duration::ZERO)),
        )
        .await;
        match &result {
            Err(err) => {
                crate::trace::stop_failed(err);
                let leftover = self.lifecycle.pending_stops();
                if leftover > 0 {
                    crate::trace::hooks_abandoned(leftover);
                }
            }
            Ok(()) => crate::trace::stopped(),
        }
        result
    }
}

/// OS signal listener owned by [`ModrunBuilder::run`]. Dropping it uninstalls
/// the wait without leaving a detached task behind.
struct SignalWatch {
    #[cfg(all(feature = "signal", unix))]
    sigint: tokio::signal::unix::Signal,
    #[cfg(all(feature = "signal", unix))]
    sigterm: tokio::signal::unix::Signal,
    #[cfg(all(feature = "signal", windows))]
    ctrl_c: tokio::signal::windows::CtrlC,
    #[cfg(all(feature = "signal", windows))]
    ctrl_break: tokio::signal::windows::CtrlBreak,
    #[cfg(all(feature = "signal", windows))]
    ctrl_close: tokio::signal::windows::CtrlClose,
    #[cfg(all(feature = "signal", windows))]
    ctrl_shutdown: tokio::signal::windows::CtrlShutdown,
}

impl SignalWatch {
    fn install() -> Result<Self> {
        #[cfg(all(feature = "signal", unix))]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let sigint = signal(SignalKind::interrupt()).map_err(Error::SigintListen)?;
            let sigterm = signal(SignalKind::terminate()).map_err(Error::SigtermListen)?;
            Ok(Self { sigint, sigterm })
        }

        #[cfg(all(feature = "signal", windows))]
        {
            use tokio::signal::windows::{ctrl_break, ctrl_c, ctrl_close, ctrl_shutdown};
            Ok(Self {
                ctrl_c: ctrl_c().map_err(Error::SignalListen)?,
                ctrl_break: ctrl_break().map_err(Error::SignalListen)?,
                ctrl_close: ctrl_close().map_err(Error::SignalListen)?,
                ctrl_shutdown: ctrl_shutdown().map_err(Error::SignalListen)?,
            })
        }

        #[cfg(not(any(all(feature = "signal", unix), all(feature = "signal", windows))))]
        {
            Ok(Self {})
        }
    }

    async fn recv(&mut self) -> &'static str {
        #[cfg(all(feature = "signal", unix))]
        {
            tokio::select! {
                Some(()) = self.sigint.recv() => "SIGINT",
                Some(()) = self.sigterm.recv() => "SIGTERM",
                else => std::future::pending().await,
            }
        }

        #[cfg(all(feature = "signal", windows))]
        {
            tokio::select! {
                Some(()) = self.ctrl_c.recv() => "CTRL_C",
                Some(()) = self.ctrl_break.recv() => "CTRL_BREAK",
                Some(()) = self.ctrl_close.recv() => "CTRL_CLOSE",
                Some(()) = self.ctrl_shutdown.recv() => "CTRL_SHUTDOWN",
                else => std::future::pending().await,
            }
        }

        #[cfg(not(any(all(feature = "signal", unix), all(feature = "signal", windows))))]
        {
            let _ = self;
            std::future::pending::<&'static str>().await
        }
    }
}

/// A started application that can be stopped explicitly.
///
/// Dropping this without calling [`stop`](Self::stop) skips every OnStop hook,
/// so hold on to it for as long as the application should stay up.
///
/// The dependency container is dropped after build: singletons only stay alive
/// through values captured by hooks (or other `Clone` handles taken during
/// invoke). This is a wiring layer, not a live service locator. Build is not
/// transactional: a failed invoker may leave earlier constructors' side effects
/// applied for the duration of that failed `build` call.
#[must_use = "dropping a RunningApp skips all OnStop hooks; call stop() instead"]
pub struct RunningApp {
    inner: Option<BuiltApp>,
}

impl std::fmt::Debug for RunningApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunningApp").finish_non_exhaustive()
    }
}

impl RunningApp {
    /// Run every OnStop hook, in reverse registration order.
    ///
    /// # Errors
    ///
    /// Returns an error when one or more OnStop hooks fail, or when the stop
    /// budget expires (remaining hooks are then abandoned).
    pub async fn stop(mut self) -> Result<()> {
        let inner = self
            .inner
            .take()
            .expect("RunningApp always contains a built application");
        inner.stop().await
    }
}

impl Drop for RunningApp {
    fn drop(&mut self) {
        if self.inner.is_some() {
            crate::trace::running_app_dropped();
            #[cfg(debug_assertions)]
            eprintln!("modrun: dropping RunningApp without stop(); OnStop hooks will not run");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finish_interrupt_retains_failure_on_unwind_timeout() {
        let shutdown = Shutdowner::new();
        shutdown.fail();
        let err = finish_interrupt(
            &shutdown,
            Err(Error::UnwindTimeout(Duration::from_millis(30))),
        )
        .unwrap_err();
        match err {
            Error::CleanupAfterFailure { cleanup, earlier } => {
                assert!(
                    matches!(*cleanup, Error::UnwindTimeout(_)),
                    "cleanup was {cleanup}"
                );
                assert!(
                    matches!(*earlier, Error::TaskFailedDuringStart),
                    "earlier was {earlier}"
                );
            }
            other => panic!("expected CleanupAfterFailure, got {other}"),
        }
    }

    #[test]
    fn finish_interrupt_keeps_join_error() {
        let shutdown = Shutdowner::new();
        shutdown.fail();
        let err = finish_interrupt(&shutdown, Err(Error::hook("died"))).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("died"), "{msg}");
        assert!(!msg.contains("cleanup failed"), "{msg}");
    }

    #[test]
    fn finish_interrupt_graceful_ok_is_ok() {
        let shutdown = Shutdowner::new();
        shutdown.shutdown();
        finish_interrupt(&shutdown, Ok(())).unwrap();
    }
}
