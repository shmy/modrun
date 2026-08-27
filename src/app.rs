use std::time::Duration;

use crate::error::{Error, Result};

use crate::container::{Container, seed_builtins};
use crate::error::combine_results;
use crate::invoke::ScopedInvoker;
use crate::lifecycle::Lifecycle;
use crate::module::Module;
use crate::option::ModOption;
use crate::scope::ScopeId;
use crate::shutdown::Shutdowner;
use crate::timeout::{DEFAULT_TIMEOUT, build_timeout, start_timeout, stop_timeout};
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
#[derive(Default)]
pub struct ModrunBuilder {
    options: Vec<Box<dyn ModOption>>,
    banner: crate::banner::Banner,
}

impl std::fmt::Debug for ModrunBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModrunBuilder")
            .field("options", &self.options.len())
            .field("banner", &self.banner)
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
    /// If set more than once, the last value wins. [`no_build_timeout`](Self::no_build_timeout)
    /// disables the budget.
    #[must_use]
    pub fn build_timeout(mut self, duration: Duration) -> Self {
        self.build_timeout_mut(duration);
        self
    }

    /// [`build_timeout`](Self::build_timeout) for `&mut self`.
    pub fn build_timeout_mut(&mut self, duration: Duration) -> &mut Self {
        self.push_option(build_timeout(Some(duration)));
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
        self.push_option(build_timeout(None));
        self
    }

    /// Total budget for all OnStart hooks.
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
        self.push_option(start_timeout(Some(duration)));
        self
    }

    /// Do not bound OnStart (or the start phase of a cancelled build).
    #[must_use]
    pub fn no_start_timeout(mut self) -> Self {
        self.no_start_timeout_mut();
        self
    }

    /// [`no_start_timeout`](Self::no_start_timeout) for `&mut self`.
    pub fn no_start_timeout_mut(&mut self) -> &mut Self {
        self.push_option(start_timeout(None));
        self
    }

    /// Total budget for all OnStop hooks (including unwind after a failed or
    /// cancelled start).
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
        self.push_option(stop_timeout(Some(duration)));
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
        self.push_option(stop_timeout(None));
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
    /// Ctrl-C / SIGTERM are wired from the beginning of this call when the
    /// `signal` crate feature is enabled (on by default). [`start`](Self::start)
    /// does not install signal handlers.
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
    /// than a crash. A timeout or hook failure still returns an error — a
    /// concurrent shutdown does not turn that failure into `Ok(())`.
    ///
    /// # Errors
    ///
    /// Returns an error when wiring/validation fails, a start or stop hook
    /// fails, a phase times out, or OS signal listeners cannot be installed.
    pub async fn run(self) -> Result<()> {
        self.print_banner();
        let lifecycle = Lifecycle::new();
        let shutdown = Shutdowner::new();
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
                return finish_run(
                    Ok(()),
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
                return finish_run(Ok(()), graceful_unwind(&app).await);
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
            }
        }
        app.stop().await
    }

    /// Build and start without waiting for a shutdown signal (useful in tests).
    ///
    /// Does not install Ctrl-C / SIGTERM handlers; call [`RunningApp::stop`]
    /// yourself (or use [`ModrunBuilder::run`](Self::run) for signal-driven stop).
    ///
    /// # Errors
    ///
    /// Returns an error when wiring/validation fails, graph construction times
    /// out, a start hook fails, or start times out (including cleanup failures
    /// while unwinding).
    pub async fn start(self) -> Result<RunningApp> {
        self.print_banner();
        let lifecycle = Lifecycle::new();
        let shutdown = Shutdowner::new();
        let mut state = self.into_build_state(lifecycle.clone(), shutdown)?;
        let stop_timeout = state.stop_timeout;
        if let Err(e) = run_invokers(&mut state).await {
            finish_run(Err(e), unwind_registered(&lifecycle, stop_timeout).await)?;
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
            build_timeout: Some(DEFAULT_TIMEOUT),
            start_timeout: Some(DEFAULT_TIMEOUT),
            stop_timeout: Some(DEFAULT_TIMEOUT),
            current_scope: ScopeId::ROOT,
            private_mode: false,
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
            let deps = invoker.dep_types().to_vec();
            let module = state.container.scopes().name(scope);
            crate::trace::invoking(function, &deps, module);
            let previous = state.container.enter_scope(scope);
            let result = invoker.call(&mut state.container).await;
            state.container.leave_scope(previous);
            if let Err(ref err) = result {
                crate::trace::invoke_failed(function, &deps, module, err);
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
        Err(e) => combine_results(Err(e), cleanup),
    }
}

async fn unwind_registered(lifecycle: &Lifecycle, stop_timeout: Option<Duration>) -> Result<()> {
    lifecycle.prepare_for_unwind();
    match with_timeout(
        stop_timeout,
        lifecycle.unwind_started(),
        Error::UnwindTimeout(stop_timeout.unwrap_or(Duration::ZERO)),
    )
    .await
    {
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
    pub(crate) private_mode: bool,
}

struct BuiltApp {
    lifecycle: Lifecycle,
    start_timeout: Option<Duration>,
    stop_timeout: Option<Duration>,
}

async fn graceful_unwind(app: &BuiltApp) -> Result<()> {
    match app.unwind_with_budget().await {
        Ok(()) => {
            crate::trace::rolled_back();
            Ok(())
        }
        Err(err) => {
            crate::trace::rollback_failed(&err);
            let leftover = app.lifecycle.pending_stops();
            if leftover > 0 {
                crate::trace::hooks_abandoned(leftover);
            }
            Err(err)
        }
    }
}

async fn with_timeout(
    budget: Option<Duration>,
    fut: impl std::future::Future<Output = Result<()>>,
    timed_out: Error,
) -> Result<()> {
    match budget {
        None => fut.await,
        Some(d) => match tokio::time::timeout(d, fut).await {
            Ok(result) => result,
            Err(_) => Err(timed_out),
        },
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
                let unwound = self.unwind_with_budget().await;
                if let Err(ref unwind_err) = unwound {
                    crate::trace::rollback_failed(unwind_err);
                    let leftover = self.lifecycle.pending_stops();
                    if leftover > 0 {
                        crate::trace::hooks_abandoned(leftover);
                    }
                } else {
                    crate::trace::rolled_back();
                }
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
        if let Err(ref err) = result {
            crate::trace::stop_failed(err);
            let leftover = self.lifecycle.pending_stops();
            if leftover > 0 {
                crate::trace::hooks_abandoned(leftover);
            }
        } else {
            crate::trace::stopped();
        }
        result
    }

    async fn unwind_with_budget(&self) -> Result<()> {
        with_timeout(
            self.stop_timeout,
            self.lifecycle.unwind_started(),
            Error::UnwindTimeout(self.stop_timeout.unwrap_or(Duration::ZERO)),
        )
        .await
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
                _ = self.sigint.recv() => "SIGINT",
                _ = self.sigterm.recv() => "SIGTERM",
            }
        }

        #[cfg(all(feature = "signal", windows))]
        {
            tokio::select! {
                _ = self.ctrl_c.recv() => "CTRL_C",
                _ = self.ctrl_break.recv() => "CTRL_BREAK",
                _ = self.ctrl_close.recv() => "CTRL_CLOSE",
                _ = self.ctrl_shutdown.recv() => "CTRL_SHUTDOWN",
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
