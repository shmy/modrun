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

* [`logging::init`](https://docs.rs/modrun/latest/modrun/logging/fn.init.html) is a no-op when a tracing subscriber is already installed (it no longer panics).
* [`MultipleStopError`](https://docs.rs/modrun/latest/modrun/struct.MultipleStopError.html) is `#[non_exhaustive]` and exposes accessors alongside the existing public fields.
