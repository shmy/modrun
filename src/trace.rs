//! Structured framework events, emitted via [`tracing`].
//!
//! Messages and field names mirror [uber/fx](https://github.com/uber-go/fx)'s
//! `fxevent` logger so `RUST_LOG=modrun=info` reads similarly to Fx's default
//! output. Install a subscriber in your binary (for example
//! `tracing_subscriber::fmt::init()`); with no subscriber these calls are cheap
//! no-ops.

use std::any::TypeId;
use std::time::Duration;

use crate::lifecycle::Lifecycle;
use crate::shutdown::Shutdowner;

/// Log target for every modrun framework event.
pub(crate) const TARGET: &str = "modrun";

pub(crate) fn start_timer() -> Option<std::time::Instant> {
    tracing::enabled!(target: TARGET, tracing::Level::INFO).then(std::time::Instant::now)
}

pub(crate) fn elapsed(started: Option<std::time::Instant>) -> std::time::Duration {
    started
        .map(|t| t.elapsed())
        .unwrap_or(std::time::Duration::ZERO)
}

fn module_label(name: &'static str) -> Option<&'static str> {
    (name != "<root>").then_some(name)
}

fn is_framework_dep(id: TypeId) -> bool {
    id == TypeId::of::<Lifecycle>() || id == TypeId::of::<Shutdowner>()
}

fn user_deps(deps: &[(TypeId, &'static str)]) -> String {
    deps.iter()
        .filter(|(id, _)| !is_framework_dep(*id))
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(crate) fn invoking(function: &str, deps: &[(TypeId, &'static str)], scope_name: &'static str) {
    // `user_deps` allocates, so skip it entirely when nothing is listening.
    if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        return;
    }
    let user_deps = user_deps(deps);
    match (module_label(scope_name), user_deps.is_empty()) {
        (Some(module), true) => {
            tracing::info!(target: TARGET, function, module, "invoking");
        }
        (Some(module), false) => {
            tracing::info!(target: TARGET, function, deps = user_deps, module, "invoking");
        }
        (None, true) => {
            tracing::info!(target: TARGET, function, "invoking");
        }
        (None, false) => {
            tracing::info!(target: TARGET, function, deps = user_deps, "invoking");
        }
    }
}

pub(crate) fn invoke_failed(
    function: &str,
    deps: &[(TypeId, &'static str)],
    scope_name: &'static str,
    err: &crate::Error,
) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let user_deps = user_deps(deps);
    match module_label(scope_name) {
        Some(module) if user_deps.is_empty() => {
            tracing::error!(
                target: TARGET,
                error = %err,
                function,
                module,
                "invoke failed"
            );
        }
        Some(module) => {
            tracing::error!(
                target: TARGET,
                error = %err,
                function,
                deps = user_deps,
                module,
                "invoke failed"
            );
        }
        None if user_deps.is_empty() => {
            tracing::error!(
                target: TARGET,
                error = %err,
                function,
                "invoke failed"
            );
        }
        None => {
            tracing::error!(
                target: TARGET,
                error = %err,
                function,
                deps = user_deps,
                "invoke failed"
            );
        }
    }
}

pub(crate) fn provided(type_name: &str, scope_name: &'static str, private: bool) {
    match module_label(scope_name) {
        Some(module) if private => {
            tracing::info!(target: TARGET, type_name, module, private, "provided");
        }
        Some(module) => {
            tracing::info!(target: TARGET, type_name, module, "provided");
        }
        None if private => {
            tracing::info!(target: TARGET, type_name, private, "provided");
        }
        None => {
            tracing::info!(target: TARGET, type_name, "provided");
        }
    }
}

pub(crate) fn supplied(type_name: &str, scope_name: &'static str, private: bool) {
    match module_label(scope_name) {
        Some(module) if private => {
            tracing::info!(target: TARGET, type_name, module, private, "supplied");
        }
        Some(module) => {
            tracing::info!(target: TARGET, type_name, module, "supplied");
        }
        None if private => {
            tracing::info!(target: TARGET, type_name, private, "supplied");
        }
        None => {
            tracing::info!(target: TARGET, type_name, "supplied");
        }
    }
}

pub(crate) fn before_run(name: &str, scope_name: &'static str) {
    match module_label(scope_name) {
        Some(module) => {
            tracing::info!(
                target: TARGET,
                name,
                kind = "provide",
                module,
                "before run"
            );
        }
        None => {
            tracing::info!(target: TARGET, name, kind = "provide", "before run");
        }
    }
}

pub(crate) fn run_ok(name: &str, scope_name: &'static str, runtime: Duration) {
    match module_label(scope_name) {
        Some(module) => {
            tracing::info!(
                target: TARGET,
                name,
                kind = "provide",
                module,
                runtime = ?runtime,
                "run"
            );
        }
        None => {
            tracing::info!(
                target: TARGET,
                name,
                kind = "provide",
                runtime = ?runtime,
                "run"
            );
        }
    }
}

pub(crate) fn run_err(name: &str, scope_name: &'static str, err: &crate::Error) {
    match module_label(scope_name) {
        Some(module) => {
            tracing::error!(
                target: TARGET,
                name,
                kind = "provide",
                module,
                error = %err,
                "error returned"
            );
        }
        None => {
            tracing::error!(
                target: TARGET,
                name,
                kind = "provide",
                error = %err,
                "error returned"
            );
        }
    }
}

pub(crate) fn run_cancelled(name: &str, scope_name: &'static str) {
    match module_label(scope_name) {
        Some(module) => {
            tracing::error!(
                target: TARGET,
                name,
                kind = "provide",
                module,
                "run cancelled"
            );
        }
        None => {
            tracing::error!(
                target: TARGET,
                name,
                kind = "provide",
                "run cancelled"
            );
        }
    }
}

pub(crate) fn on_start_executing(hook: usize, name: Option<&str>) {
    match name {
        Some(name) => tracing::info!(target: TARGET, hook, name, "OnStart hook executing"),
        None => tracing::info!(target: TARGET, hook, "OnStart hook executing"),
    }
}

pub(crate) fn on_start_executed(hook: usize, name: Option<&str>, runtime: Duration) {
    match name {
        Some(name) => tracing::info!(
            target: TARGET,
            hook,
            name,
            runtime = ?runtime,
            "OnStart hook executed"
        ),
        None => tracing::info!(
            target: TARGET,
            hook,
            runtime = ?runtime,
            "OnStart hook executed"
        ),
    }
}

pub(crate) fn on_start_failed(hook: usize, name: Option<&str>, err: &crate::Error) {
    match name {
        Some(name) => {
            tracing::error!(target: TARGET, hook, name, error = %err, "OnStart hook failed")
        }
        None => tracing::error!(target: TARGET, hook, error = %err, "OnStart hook failed"),
    }
}

pub(crate) fn on_start_cancelled(hook: usize, name: Option<&str>) {
    match name {
        Some(name) => tracing::error!(target: TARGET, hook, name, "OnStart hook cancelled"),
        None => tracing::error!(target: TARGET, hook, "OnStart hook cancelled"),
    }
}

pub(crate) fn on_stop_executing(hook: usize, name: Option<&str>) {
    match name {
        Some(name) => tracing::info!(target: TARGET, hook, name, "OnStop hook executing"),
        None => tracing::info!(target: TARGET, hook, "OnStop hook executing"),
    }
}

pub(crate) fn on_stop_executed(hook: usize, name: Option<&str>, runtime: Duration) {
    match name {
        Some(name) => tracing::info!(
            target: TARGET,
            hook,
            name,
            runtime = ?runtime,
            "OnStop hook executed"
        ),
        None => tracing::info!(
            target: TARGET,
            hook,
            runtime = ?runtime,
            "OnStop hook executed"
        ),
    }
}

pub(crate) fn on_stop_failed(hook: usize, name: Option<&str>, err: &crate::Error) {
    match name {
        Some(name) => {
            tracing::error!(target: TARGET, hook, name, error = %err, "OnStop hook failed")
        }
        None => tracing::error!(target: TARGET, hook, error = %err, "OnStop hook failed"),
    }
}

pub(crate) fn on_stop_cancelled(hook: usize, name: Option<&str>) {
    match name {
        Some(name) => tracing::error!(target: TARGET, hook, name, "OnStop hook cancelled"),
        None => tracing::error!(target: TARGET, hook, "OnStop hook cancelled"),
    }
}

pub(crate) fn started() {
    tracing::info!(target: TARGET, "started");
}

pub(crate) fn start_failed(err: &crate::Error) {
    tracing::error!(target: TARGET, error = %err, "start failed");
}

pub(crate) fn rolling_back(err: &crate::Error) {
    tracing::error!(target: TARGET, error = %err, "start failed, rolling back");
}

pub(crate) fn rolling_back_after_shutdown() {
    tracing::error!(target: TARGET, "start interrupted by shutdown, rolling back");
}

pub(crate) fn rollback_failed(err: &crate::Error) {
    tracing::error!(target: TARGET, error = %err, "rollback failed");
}

pub(crate) fn rolled_back() {
    tracing::info!(target: TARGET, "rolled back");
}

pub(crate) fn received_signal(signal: &str) {
    tracing::info!(target: TARGET, signal, "received signal");
}

pub(crate) fn shutdown_requested() {
    tracing::info!(target: TARGET, "shutdown requested");
}

pub(crate) fn stop_failed(err: &crate::Error) {
    tracing::error!(target: TARGET, error = %err, "stop failed");
}

pub(crate) fn stopped() {
    tracing::info!(target: TARGET, "stopped");
}

pub(crate) fn hooks_abandoned(count: usize) {
    tracing::error!(
        target: TARGET,
        leftover = count,
        "OnStop hooks abandoned after timeout"
    );
}

pub(crate) fn running_app_dropped() {
    tracing::warn!(
        target: TARGET,
        "dropping RunningApp without stop(); OnStop hooks will not run"
    );
}
