# modrun 1.0.0

**modrun** is a modular application composer for Tokio: domain [`Module`](https://docs.rs/modrun/1.0.0/modrun/struct.Module.html)s, constructor injection, explicit module boundaries, and coordinated lifecycle — wired once at the composition root.

Requires **Rust 1.85+** (edition 2024).

```toml
[dependencies]
modrun = "1.0"
```

## Highlights

* **Five wiring verbs** — `provide`, `supply`, `invoke`, `module`, `provide_group`; application API stable since 1.0.0 ([CONTRIBUTING.md](https://github.com/shmy/modrun/blob/main/CONTRIBUTING.md)).
* **`Group<T>`** — multiple modules contribute values of the same type; inject `Group<T>` or `Arc<Group<T>>` at the composition root.
* **Lifecycle** — `Hook`, `task` / `task_with`, `Shutdowner`, cooperative shutdown and timeouts.
* **Dependency graph** — `render_dot()` and `.dot_graph(path)` for Graphviz DOT export (validation only, no constructors run).
* **Observability** — fx-style console lines plus structured `tracing` fields (`constructor`, `module`, `elapsed`, `error`, …).
* **Examples** — `basic`, `handlers`, `worker`, `swap`, `wrap`, `axum`.

## Documentation

* [README](https://github.com/shmy/modrun/blob/main/README.md) · [简体中文](https://github.com/shmy/modrun/blob/main/README_zh.md)
* [docs.rs](https://docs.rs/modrun/1.0.0)
* [CHANGELOG](https://github.com/shmy/modrun/blob/main/CHANGELOG.md)

## Install

```bash
cargo add modrun
```

Full release notes: [CHANGELOG.md#100---2026-09-02](https://github.com/shmy/modrun/blob/main/CHANGELOG.md#100---2026-09-02).
