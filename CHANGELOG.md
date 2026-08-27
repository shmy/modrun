# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## Unreleased

### Added

* [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) / [`Stopped`](https://docs.rs/modrun/latest/modrun/struct.Stopped.html) for long-running OnStart work. OnStop signals the task and joins; drop aborts leftover work.
* [`task_with`](https://docs.rs/modrun/latest/modrun/fn.task_with.html) / [`PreparedTask`](https://docs.rs/modrun/latest/modrun/struct.PreparedTask.html): await setup (bind/listen) during OnStart so those failures fail start, then spawn the rest.
* [`logging::try_init`](https://docs.rs/modrun/latest/modrun/logging/fn.try_init.html) / `try_init_with_filter` so callers can detect an existing subscriber.
* Structured tracing fields (`constructor`, `module`, `elapsed_ms`, `error`, …) on framework events, while the `logging` helper still prints fx-style console lines.
* `worker` example: async constructors, primary/replica newtypes, and a `task` that selects on [`Stopped`](https://docs.rs/modrun/latest/modrun/struct.Stopped.html).
* `swap` example: `supply` a test double at the composition root instead of `provide` inside a module.
* Chinese README (`README_zh.md`).

### Changed

* MSRV lowered from 1.88 to **1.85** (edition 2024 floor; let-chains are unused). CI checks the library with `cargo test` (not `--all-targets`) so examples/benches are not bound to that MSRV.
* [`logging::init`](https://docs.rs/modrun/latest/modrun/logging/fn.init.html) is a no-op when a tracing subscriber is already installed (it no longer panics). ANSI follows stderr (TTY only).
* [`MultipleStopError`](https://docs.rs/modrun/latest/modrun/struct.MultipleStopError.html) is `#[non_exhaustive]`; use accessors (`count()`, `summary()`, `errors()`).
* `std::io::Error` no longer converts with `From`. Use [`Error::io`](https://docs.rs/modrun/latest/modrun/enum.Error.html#method.io) with a context label.
* Panic during invoke / construct / OnStart / OnStop is traced as `panicked`, not `cancelled`.
* Background [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) join failures include the task name.
* [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) / [`task_with`](https://docs.rs/modrun/latest/modrun/fn.task_with.html) request [`Shutdowner::shutdown`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html) when the background future returns `Err` or panics, so `run()` does not wait forever for a signal.
* Named lifecycle hooks and fallible invokers include their name in [`Error`](https://docs.rs/modrun/latest/modrun/enum.Error.html) (`hook 'http' failed`, `invoker my::boot failed`).

### Fixed

* OS signal listeners no longer treat a closed signal stream (`recv() == None`) as Ctrl-C / SIGTERM.
