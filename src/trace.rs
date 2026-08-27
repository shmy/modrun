//! Framework events, emitted via [`tracing`].
//!
//! Messages follow [uber/fx](https://github.com/uber-go/fx)'s `ConsoleLogger` layout.
//! Install `modrun::logging::init()` (feature `logging`) or an equivalent
//! subscriber without timestamps, then filter with `RUST_LOG=modrun=info`.

use std::any::TypeId;
use std::time::Duration;

use crate::lifecycle::Lifecycle;
use crate::shutdown::Shutdowner;

/// Log target for every modrun framework event.
pub(crate) const TARGET: &str = "modrun";

const PREFIX: &str = "[modrun]";

pub(crate) fn start_timer() -> Option<std::time::Instant> {
    tracing::enabled!(target: TARGET, tracing::Level::INFO).then(std::time::Instant::now)
}

pub(crate) fn elapsed(started: Option<std::time::Instant>) -> Duration {
    started.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
}

fn module_label(name: &'static str) -> Option<&'static str> {
    (name != "<root>").then_some(name)
}

fn module_suffix(module: Option<&'static str>) -> String {
    module
        .map(|name| format!(" from module \"{name}\""))
        .unwrap_or_default()
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

fn log_info(message: String) {
    if tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        tracing::info!(target: TARGET, "{message}");
    }
}

fn log_error(message: String) {
    if tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        tracing::error!(target: TARGET, "{message}");
    }
}

fn log_warn(message: String) {
    if tracing::enabled!(target: TARGET, tracing::Level::WARN) {
        tracing::warn!(target: TARGET, "{message}");
    }
}

fn hook_label(name: Option<&str>, hook: usize) -> String {
    name.map(str::to_owned)
        .unwrap_or_else(|| format!("#{hook}"))
}

pub(crate) fn invoking(function: &str, _deps: &[(TypeId, &'static str)], scope_name: &'static str) {
    let suffix = module_suffix(module_label(scope_name));
    log_info(format!("{PREFIX} INVOKE\t\t{function}{suffix}"));
}

pub(crate) fn invoke_failed(
    function: &str,
    deps: &[(TypeId, &'static str)],
    scope_name: &'static str,
    err: &crate::Error,
) {
    let suffix = module_suffix(module_label(scope_name));
    let user_deps = user_deps(deps);
    let deps_suffix = if user_deps.is_empty() {
        String::new()
    } else {
        format!(" (deps: {user_deps})")
    };
    log_error(format!(
        "{PREFIX} ERROR\t\tinvoke failed: {err} ({function}{deps_suffix}){suffix}"
    ));
}

pub(crate) fn invoke_cancelled(function: &str, scope_name: &'static str) {
    let suffix = module_suffix(module_label(scope_name));
    log_error(format!(
        "{PREFIX} ERROR\t\tinvoke cancelled ({function}){suffix}"
    ));
}

pub(crate) fn invoke_panicked(function: &str, scope_name: &'static str) {
    let suffix = module_suffix(module_label(scope_name));
    log_error(format!(
        "{PREFIX} ERROR\t\tinvoke panicked ({function}){suffix}"
    ));
}

pub(crate) fn provided(
    type_name: &str,
    constructor: &str,
    scope_name: &'static str,
    private: bool,
) {
    let private_str = if private { " (PRIVATE)" } else { "" };
    let suffix = module_suffix(module_label(scope_name));
    log_info(format!(
        "{PREFIX} PROVIDE{private_str}\t{type_name} <= {constructor}{suffix}"
    ));
}

pub(crate) fn supplied(type_name: &str, scope_name: &'static str, private: bool) {
    let private_str = if private { " (PRIVATE)" } else { "" };
    let suffix = module_suffix(module_label(scope_name));
    log_info(format!("{PREFIX} SUPPLY{private_str}\t{type_name}{suffix}"));
}

pub(crate) fn before_run(name: &str, scope_name: &'static str) {
    let suffix = module_suffix(module_label(scope_name));
    log_info(format!("{PREFIX} BEFORE RUN\tprovide: {name}{suffix}"));
}

pub(crate) fn run_ok(name: &str, scope_name: &'static str, runtime: Duration) {
    let suffix = module_suffix(module_label(scope_name));
    log_info(format!(
        "{PREFIX} RUN\t\tprovide: {name} in {runtime:?}{suffix}"
    ));
}

pub(crate) fn run_err(name: &str, scope_name: &'static str, err: &crate::Error) {
    let suffix = module_suffix(module_label(scope_name));
    log_error(format!(
        "{PREFIX} ERROR\t\tprovide: {name} failed: {err}{suffix}"
    ));
}

pub(crate) fn run_cancelled(name: &str, scope_name: &'static str) {
    let suffix = module_suffix(module_label(scope_name));
    log_error(format!(
        "{PREFIX} ERROR\t\tprovide: {name} cancelled{suffix}"
    ));
}

pub(crate) fn on_start_executing(hook: usize, name: Option<&str>) {
    let label = hook_label(name, hook);
    log_info(format!("{PREFIX} HOOK OnStart\t\t{label} executing"));
}

pub(crate) fn on_start_executed(hook: usize, name: Option<&str>, runtime: Duration) {
    let label = hook_label(name, hook);
    log_info(format!(
        "{PREFIX} HOOK OnStart\t\t{label} ran successfully in {runtime:?}"
    ));
}

pub(crate) fn on_start_failed(hook: usize, name: Option<&str>, err: &crate::Error) {
    let label = hook_label(name, hook);
    log_error(format!("{PREFIX} HOOK OnStart\t\t{label} failed: {err}"));
}

pub(crate) fn on_start_cancelled(hook: usize, name: Option<&str>) {
    let label = hook_label(name, hook);
    log_error(format!("{PREFIX} HOOK OnStart\t\t{label} cancelled"));
}

pub(crate) fn on_stop_executing(hook: usize, name: Option<&str>) {
    let label = hook_label(name, hook);
    log_info(format!("{PREFIX} HOOK OnStop\t\t{label} executing"));
}

pub(crate) fn on_stop_executed(hook: usize, name: Option<&str>, runtime: Duration) {
    let label = hook_label(name, hook);
    log_info(format!(
        "{PREFIX} HOOK OnStop\t\t{label} ran successfully in {runtime:?}"
    ));
}

pub(crate) fn on_stop_failed(hook: usize, name: Option<&str>, err: &crate::Error) {
    let label = hook_label(name, hook);
    log_error(format!("{PREFIX} HOOK OnStop\t\t{label} failed: {err}"));
}

pub(crate) fn on_stop_cancelled(hook: usize, name: Option<&str>) {
    let label = hook_label(name, hook);
    log_error(format!("{PREFIX} HOOK OnStop\t\t{label} cancelled"));
}

pub(crate) fn started() {
    log_info(format!("{PREFIX} RUNNING"));
}

pub(crate) fn start_failed(err: &crate::Error) {
    log_error(format!("{PREFIX} ERROR\t\tFailed to start: {err}"));
}

pub(crate) fn rolling_back(err: &crate::Error) {
    log_error(format!(
        "{PREFIX} ERROR\t\tStart failed, rolling back: {err}"
    ));
}

pub(crate) fn rolling_back_after_shutdown() {
    log_error(format!(
        "{PREFIX} ERROR\t\tStart interrupted by shutdown, rolling back"
    ));
}

pub(crate) fn rollback_failed(err: &crate::Error) {
    log_error(format!(
        "{PREFIX} ERROR\t\tCouldn't roll back cleanly: {err}"
    ));
}

pub(crate) fn rolled_back() {
    log_info(format!("{PREFIX} ROLLED BACK"));
}

pub(crate) fn received_signal(signal: &str) {
    log_info(format!("{PREFIX} {}", signal.to_ascii_uppercase()));
}

pub(crate) fn shutdown_requested() {
    log_info(format!("{PREFIX} SHUTDOWN REQUESTED"));
}

pub(crate) fn stop_failed(err: &crate::Error) {
    log_error(format!("{PREFIX} ERROR\t\tFailed to stop cleanly: {err}"));
}

pub(crate) fn stopped() {
    log_info(format!("{PREFIX} STOPPED"));
}

pub(crate) fn hooks_abandoned(count: usize) {
    log_error(format!(
        "{PREFIX} ERROR\t\tOnStop hooks abandoned after timeout (leftover: {count})"
    ));
}

pub(crate) fn running_app_dropped() {
    log_warn(format!(
        "{PREFIX} WARN\t\tdropping RunningApp without stop(); OnStop hooks will not run"
    ));
}
