# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Changed

* **Breaking:** structured trace field `elapsed_ms` (integer milliseconds, sub-ms values truncated to `0`) is replaced by `elapsed` (human-readable string matching `{runtime:?}`, e.g. `"806.417µs"`, `"2.174542ms"`). Affects `RUN`, `HOOK OnStart`, and `HOOK OnStop` success events.
* Group aggregation collects members without cloning the member key list.
* Constructor trace path checks `tracing` INFO once per provider when logging is off.
* Graph validation caches dependency resolution during `validate` / `freeze_layers`, and skips the DFS cycle pass on the success path (runs only when layer sorting detects a cycle, for a readable path).
* Scope ancestor walks use incrementally built chains (slice iteration instead of parent-pointer hops).

## \[1.0.0] - 2026-08-31

First stable release. Application-facing API is frozen; see [CONTRIBUTING.md](CONTRIBUTING.md)
for the stability promise and semver rules. Type-erased wiring in [`modrun::__wiring`](https://docs.rs/modrun/latest/modrun/__wiring/index.html)
remains intentionally unstable relative to typed `provide` / `invoke`.

### Added

* [`Group<T>`](https://docs.rs/modrun/latest/modrun/struct.Group.html): multiple modules contribute values of the same type; inject `Group<T>` or `Arc<Group<T>>`. Register members with `provide_group` / `provide_group_result` / `provide_group_async` / `provide_group_result_async`, `supply_group`, or hidden `provide_group_dyn`. Empty groups need [`init_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.init_group) or [`require_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.require_group) (`require_group` also rejects emptiness at build time). Members require `T: Clone`. `handlers` example.
* [`ModrunBuilder::render_dot`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.render_dot) and [`.dot_graph(path)`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.dot_graph): export the dependency graph as Graphviz DOT after validation, without running constructors or invokers.
* [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) / [`Stopped`](https://docs.rs/modrun/latest/modrun/struct.Stopped.html) for long-running OnStart work. OnStop signals the task and joins; drop aborts leftover work.
* [`task_with`](https://docs.rs/modrun/latest/modrun/fn.task_with.html) / [`PreparedTask`](https://docs.rs/modrun/latest/modrun/struct.PreparedTask.html): await setup (bind/listen) during OnStart so those failures fail start, then spawn the rest.
* [`logging::try_init`](https://docs.rs/modrun/latest/modrun/logging/fn.try_init.html) / `try_init_with_filter` so callers can detect an existing subscriber.
* Structured tracing fields (`constructor`, `module`, `elapsed_ms`, `error`, …) on framework events, while the `logging` helper still prints fx-style console lines.
* `worker` example: async constructors, primary/replica newtypes, and a `task` that selects on [`Stopped`](https://docs.rs/modrun/latest/modrun/struct.Stopped.html).
* `swap` example: `supply` a test double at the composition root instead of `provide` inside a module.
* Chinese README (`README_zh.md`).

### Changed

* **Breaking:** removed all public `*_mut` builder methods (`provide_mut`, `module_mut`, `build_timeout_mut`, …). Chain on consuming `self` instead.
* **Breaking:** removed `Module::new`; use `Module::builder("name")` (symmetric with `Modrun::builder()`).
* Positioning and docs: modular application **composer** (not generic DI); API matrix and three common patterns (domain module / group / wrapper) in README.
* Type-erased wiring moved to hidden [`modrun::__wiring`](https://docs.rs/modrun/latest/modrun/__wiring/index.html); `provide_dyn` / `invoke_dyn` / `provide_group_dyn` are `#[doc(hidden)]`.
* MSRV lowered from 1.88 to **1.85** (edition 2024 floor; let-chains are unused). CI checks the library with `cargo test` (not `--all-targets`) so examples/benches are not bound to that MSRV.
* [`logging::init`](https://docs.rs/modrun/latest/modrun/logging/fn.init.html) is a no-op when a tracing subscriber is already installed (it no longer panics). The helper writes to stderr; ANSI follows stderr (TTY only).
* Default startup banner is printed to stderr, and only when stderr is a TTY. [`.banner(text)`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.banner) still prints custom text to stderr even when not a TTY.
* [`MultipleStopError`](https://docs.rs/modrun/latest/modrun/struct.MultipleStopError.html) is `#[non_exhaustive]`; use accessors (`count()`, `summary()`, `errors()`).
* `std::io::Error` no longer converts with `From`. Use [`Error::io`](https://docs.rs/modrun/latest/modrun/enum.Error.html#method.io) with a context label.
* Panic during invoke / construct / OnStart / OnStop is traced as `panicked`, not `cancelled`.
* Background [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) join failures include the task name.
* [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) / [`task_with`](https://docs.rs/modrun/latest/modrun/fn.task_with.html) request [`Shutdowner::shutdown`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html) when the background future returns `Err` or panics, so `run()` does not wait forever for a signal.
* [`Hook::name`](https://docs.rs/modrun/latest/modrun/trait.Hook.html#method.name) now returns `&'static str` instead of `Option<&'static str>`. Unnamed hooks use the sentinel `"unnamed"`. Override the default (via the trait or [`hook().name(...)`](https://docs.rs/modrun/latest/modrun/fn.hook.html)) for clearer logs and errors. Hook trace labels are `{name} (#{index})` (e.g. `unnamed (#0)`, `http (#1)`).
* Named lifecycle hooks and fallible invokers include their name in [`Error`](https://docs.rs/modrun/latest/modrun/enum.Error.html) (`hook 'http' failed`, `invoker my::boot failed`). Unnamed hooks use `hook 'unnamed' failed`. [`Error::Io`](https://docs.rs/modrun/latest/modrun/enum.Error.html) and other non-`Hook` variants keep their identity.
* A [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) failure during start is an error from [`run`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.run), not a graceful `Ok(())`. If unwind then times out, both are retained on [`Error::CleanupAfterFailure`](https://docs.rs/modrun/latest/modrun/enum.Error.html). [`start`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.start) still succeeds; observe the failure from `stop()` or [`Shutdowner`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html).

### Fixed

* OS signal listeners no longer treat a closed signal stream (`recv() == None`) as Ctrl-C / SIGTERM.
* OnStart `Err` unwinds stop-only hooks that sit after a start hook that never ran, matching shutdown/timeout cancel.
