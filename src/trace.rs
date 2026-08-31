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
    info_enabled().then(std::time::Instant::now)
}

pub(crate) fn info_enabled() -> bool {
    tracing::enabled!(target: TARGET, tracing::Level::INFO)
}

pub(crate) fn elapsed(started: Option<std::time::Instant>) -> Duration {
    started.map(|t| t.elapsed()).unwrap_or(Duration::ZERO)
}

fn module_suffix(module: &'static str) -> String {
    if module == "<root>" {
        String::new()
    } else {
        format!(" from module \"{module}\"")
    }
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

fn elapsed_ms(runtime: Duration) -> u64 {
    runtime.as_millis().min(u128::from(u64::MAX)) as u64
}

fn hook_label(name: &'static str, hook: usize) -> String {
    format!("{name} (#{hook})")
}

pub(crate) fn invoking(function: &str, _deps: &[(TypeId, &'static str)], scope_name: &'static str) {
    if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        return;
    }
    let suffix = module_suffix(scope_name);
    tracing::info!(
        target: TARGET,
        function,
        module = scope_name,
        "{PREFIX} INVOKE\t\t{function}{suffix}"
    );
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
    let suffix = module_suffix(scope_name);
    let user_deps = user_deps(deps);
    let deps_suffix = if user_deps.is_empty() {
        String::new()
    } else {
        format!(" (deps: {user_deps})")
    };
    tracing::error!(
        target: TARGET,
        function,
        module = scope_name,
        error = %err,
        "{PREFIX} ERROR\t\tinvoke failed: {err} ({function}{deps_suffix}){suffix}"
    );
}

pub(crate) fn invoke_cancelled(function: &str, scope_name: &'static str) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let suffix = module_suffix(scope_name);
    tracing::error!(
        target: TARGET,
        function,
        module = scope_name,
        "{PREFIX} ERROR\t\tinvoke cancelled ({function}){suffix}"
    );
}

pub(crate) fn invoke_panicked(function: &str, scope_name: &'static str) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let suffix = module_suffix(scope_name);
    tracing::error!(
        target: TARGET,
        function,
        module = scope_name,
        "{PREFIX} ERROR\t\tinvoke panicked ({function}){suffix}"
    );
}

/// Log panic vs cancel from a Drop that did not see a normal finish.
pub(crate) fn emit_unfinished(finished: bool, panicked: impl FnOnce(), cancelled: impl FnOnce()) {
    if finished {
        return;
    }
    if std::thread::panicking() {
        panicked();
    } else {
        cancelled();
    }
}

pub(crate) fn provided(
    type_name: &str,
    constructor: &str,
    scope_name: &'static str,
    private: bool,
) {
    if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        return;
    }
    let private_str = if private { " (PRIVATE)" } else { "" };
    let suffix = module_suffix(scope_name);
    tracing::info!(
        target: TARGET,
        type_name,
        constructor,
        module = scope_name,
        private,
        "{PREFIX} PROVIDE{private_str}\t{type_name} <= {constructor}{suffix}"
    );
}

pub(crate) fn supplied(type_name: &str, scope_name: &'static str, private: bool) {
    if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        return;
    }
    let private_str = if private { " (PRIVATE)" } else { "" };
    let suffix = module_suffix(scope_name);
    tracing::info!(
        target: TARGET,
        type_name,
        module = scope_name,
        private,
        "{PREFIX} SUPPLY{private_str}\t{type_name}{suffix}"
    );
}

pub(crate) fn provided_group(
    type_name: &str,
    constructor: &str,
    scope_name: &'static str,
    private: bool,
) {
    if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        return;
    }
    let private_str = if private { " (PRIVATE)" } else { "" };
    let suffix = module_suffix(scope_name);
    tracing::info!(
        target: TARGET,
        type_name,
        constructor,
        module = scope_name,
        private,
        "{PREFIX} PROVIDE GROUP{private_str}\t{type_name} <= {constructor}{suffix}"
    );
}

pub(crate) fn supplied_group(type_name: &str, scope_name: &'static str, private: bool) {
    if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        return;
    }
    let private_str = if private { " (PRIVATE)" } else { "" };
    let suffix = module_suffix(scope_name);
    tracing::info!(
        target: TARGET,
        type_name,
        module = scope_name,
        private,
        "{PREFIX} SUPPLY GROUP{private_str}\t{type_name}{suffix}"
    );
}

pub(crate) fn dot_graph_written(path: &str) {
    if tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        tracing::info!(
            target: TARGET,
            path,
            "{PREFIX} GRAPH\t\twrote dependency graph to {path}"
        );
    }
}

pub(crate) fn before_run(name: &str, scope_name: &'static str) {
    if !info_enabled() {
        return;
    }
    let suffix = module_suffix(scope_name);
    tracing::info!(
        target: TARGET,
        constructor = name,
        module = scope_name,
        "{PREFIX} BEFORE RUN\tprovide: {name}{suffix}"
    );
}

pub(crate) fn run_ok(name: &str, scope_name: &'static str, runtime: Duration) {
    if !info_enabled() {
        return;
    }
    let suffix = module_suffix(scope_name);
    tracing::info!(
        target: TARGET,
        constructor = name,
        module = scope_name,
        elapsed_ms = elapsed_ms(runtime),
        "{PREFIX} RUN\t\tprovide: {name} in {runtime:?}{suffix}"
    );
}

pub(crate) fn run_err(name: &str, scope_name: &'static str, err: &crate::Error) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let suffix = module_suffix(scope_name);
    tracing::error!(
        target: TARGET,
        constructor = name,
        module = scope_name,
        error = %err,
        "{PREFIX} ERROR\t\tprovide: {name} failed: {err}{suffix}"
    );
}

pub(crate) fn run_cancelled(name: &str, scope_name: &'static str) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let suffix = module_suffix(scope_name);
    tracing::error!(
        target: TARGET,
        constructor = name,
        module = scope_name,
        "{PREFIX} ERROR\t\tprovide: {name} cancelled{suffix}"
    );
}

pub(crate) fn run_panicked(name: &str, scope_name: &'static str) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let suffix = module_suffix(scope_name);
    tracing::error!(
        target: TARGET,
        constructor = name,
        module = scope_name,
        "{PREFIX} ERROR\t\tprovide: {name} panicked{suffix}"
    );
}

pub(crate) fn on_start_executing(hook: usize, name: &'static str) {
    if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        return;
    }
    let label = hook_label(name, hook);
    tracing::info!(
        target: TARGET,
        hook,
        hook_name = label.as_str(),
        "{PREFIX} HOOK OnStart\t\t{label} executing"
    );
}

pub(crate) fn on_start_executed(hook: usize, name: &'static str, runtime: Duration) {
    if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        return;
    }
    let label = hook_label(name, hook);
    tracing::info!(
        target: TARGET,
        hook,
        hook_name = label.as_str(),
        elapsed_ms = elapsed_ms(runtime),
        "{PREFIX} HOOK OnStart\t\t{label} ran successfully in {runtime:?}"
    );
}

pub(crate) fn on_start_failed(hook: usize, name: &'static str, err: &crate::Error) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let label = hook_label(name, hook);
    tracing::error!(
        target: TARGET,
        hook,
        hook_name = label.as_str(),
        error = %err,
        "{PREFIX} HOOK OnStart\t\t{label} failed: {err}"
    );
}

pub(crate) fn on_start_cancelled(hook: usize, name: &'static str) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let label = hook_label(name, hook);
    tracing::error!(
        target: TARGET,
        hook,
        hook_name = label.as_str(),
        "{PREFIX} HOOK OnStart\t\t{label} cancelled"
    );
}

pub(crate) fn on_start_panicked(hook: usize, name: &'static str) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let label = hook_label(name, hook);
    tracing::error!(
        target: TARGET,
        hook,
        hook_name = label.as_str(),
        "{PREFIX} HOOK OnStart\t\t{label} panicked"
    );
}

pub(crate) fn on_stop_executing(hook: usize, name: &'static str) {
    if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        return;
    }
    let label = hook_label(name, hook);
    tracing::info!(
        target: TARGET,
        hook,
        hook_name = label.as_str(),
        "{PREFIX} HOOK OnStop\t\t{label} executing"
    );
}

pub(crate) fn on_stop_executed(hook: usize, name: &'static str, runtime: Duration) {
    if !tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        return;
    }
    let label = hook_label(name, hook);
    tracing::info!(
        target: TARGET,
        hook,
        hook_name = label.as_str(),
        elapsed_ms = elapsed_ms(runtime),
        "{PREFIX} HOOK OnStop\t\t{label} ran successfully in {runtime:?}"
    );
}

pub(crate) fn on_stop_failed(hook: usize, name: &'static str, err: &crate::Error) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let label = hook_label(name, hook);
    tracing::error!(
        target: TARGET,
        hook,
        hook_name = label.as_str(),
        error = %err,
        "{PREFIX} HOOK OnStop\t\t{label} failed: {err}"
    );
}

pub(crate) fn on_stop_cancelled(hook: usize, name: &'static str) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let label = hook_label(name, hook);
    tracing::error!(
        target: TARGET,
        hook,
        hook_name = label.as_str(),
        "{PREFIX} HOOK OnStop\t\t{label} cancelled"
    );
}

pub(crate) fn on_stop_panicked(hook: usize, name: &'static str) {
    if !tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        return;
    }
    let label = hook_label(name, hook);
    tracing::error!(
        target: TARGET,
        hook,
        hook_name = label.as_str(),
        "{PREFIX} HOOK OnStop\t\t{label} panicked"
    );
}

pub(crate) fn started() {
    if tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        tracing::info!(target: TARGET, "{PREFIX} RUNNING");
    }
}

pub(crate) fn start_failed(err: &crate::Error) {
    if tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        tracing::error!(
            target: TARGET,
            error = %err,
            "{PREFIX} ERROR\t\tFailed to start: {err}"
        );
    }
}

pub(crate) fn rolling_back(err: &crate::Error) {
    if tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        tracing::error!(
            target: TARGET,
            error = %err,
            "{PREFIX} ERROR\t\tStart failed, rolling back: {err}"
        );
    }
}

pub(crate) fn rolling_back_after_shutdown() {
    if tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        tracing::error!(
            target: TARGET,
            "{PREFIX} ERROR\t\tStart interrupted by shutdown, rolling back"
        );
    }
}

pub(crate) fn rollback_failed(err: &crate::Error) {
    if tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        tracing::error!(
            target: TARGET,
            error = %err,
            "{PREFIX} ERROR\t\tCouldn't roll back cleanly: {err}"
        );
    }
}

pub(crate) fn rolled_back() {
    if tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        tracing::info!(target: TARGET, "{PREFIX} ROLLED BACK");
    }
}

pub(crate) fn received_signal(signal: &str) {
    if tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        let name = signal.to_ascii_uppercase();
        tracing::info!(target: TARGET, signal = name.as_str(), "{PREFIX} {name}");
    }
}

pub(crate) fn shutdown_requested() {
    if tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        tracing::info!(target: TARGET, "{PREFIX} SHUTDOWN REQUESTED");
    }
}

pub(crate) fn stop_failed(err: &crate::Error) {
    if tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        tracing::error!(
            target: TARGET,
            error = %err,
            "{PREFIX} ERROR\t\tFailed to stop cleanly: {err}"
        );
    }
}

pub(crate) fn stopped() {
    if tracing::enabled!(target: TARGET, tracing::Level::INFO) {
        tracing::info!(target: TARGET, "{PREFIX} STOPPED");
    }
}

pub(crate) fn hooks_abandoned(count: usize) {
    if tracing::enabled!(target: TARGET, tracing::Level::ERROR) {
        tracing::error!(
            target: TARGET,
            leftover = count,
            "{PREFIX} ERROR\t\tOnStop hooks abandoned after timeout (leftover: {count})"
        );
    }
}

pub(crate) fn running_app_dropped() {
    if tracing::enabled!(target: TARGET, tracing::Level::WARN) {
        tracing::warn!(
            target: TARGET,
            "{PREFIX} WARN\t\tdropping RunningApp without stop(); OnStop hooks will not run"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_label_includes_name_and_index() {
        assert_eq!(hook_label("http", 0), "http (#0)");
        assert_eq!(hook_label("unnamed", 3), "unnamed (#3)");
    }

    #[test]
    fn module_suffix_omits_root() {
        assert_eq!(module_suffix("<root>"), "");
        assert_eq!(module_suffix("user"), " from module \"user\"");
    }
}
