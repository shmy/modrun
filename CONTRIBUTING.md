# Contributing to modrun

## What modrun is

modrun is a **modular application composer for Tokio**: domain [`Module`](src/module.rs)s,
constructor injection, explicit module boundaries, and coordinated lifecycle — wired once
at the composition root. It is **not** a general-purpose DI container or a live runtime
with inter-module messaging.

## Public model (stability)

The stable surface is intentionally small:

| Concept | API |
|---------|-----|
| Register constructors | `provide` / `provide_result` / `provide_async` / `provide_result_async` |
| Pre-built values | `supply` |
| Graph roots | `invoke` / `invoke_async` |
| Domain modules | `Module` + `provide_private` |
| Value groups | `Group<T>` + `provide_group*` / `supply_group` / `init_group` / `require_group` |
| Lifecycle | `Lifecycle`, [`Hook`](src/lifecycle.rs), `task` / `task_with` |
| Run / stop | `ModrunBuilder::run` / `start`, `RunningApp::stop`, `Shutdowner` |

[`Hook`](src/lifecycle.rs) trait methods may only gain **default** implementations in
minor releases. Constructor and invoker arity stays capped at **eight** parameters; pack
extra deps in a struct.

MSRV is **1.85** (edition 2024), tracked in `Cargo.toml` and CI.

## Application code vs wrapper authors

**Application code** should register plain functions:

```rust
Modrun::builder()
    .provide(new_config)
    .provide(new_server)
    .invoke(boot)
```

**Wrapper libraries** that re-export modrun wiring may use:

- `ProviderFn` / `InvokeFn` marker traits
- `into_provider()` / `into_invoke()`
- `provide_dyn` / `invoke_dyn` / `provide_group_dyn`

These erase types for generic wrappers; they are not part of the application-facing model.

## Deliberately rejected

We will not add APIs for:

- Property or field injection
- Runtime string tokens / named bindings (`"primary"` vs `"replica"`) — use newtypes
- Annotations, derives, or auto-scanning
- Macro-generated entire containers
- Service locator (`get<T>()` after build)
- Global singleton registries
- `fx.Populate`, `fx.Annotate`, or `fx.Replace` equivalents
- First-class `decorate` — use [wrapper constructors](examples/wrap.rs) instead
- Module-to-module event buses (not in scope today)

Internal `Container::get` exists **only** during build (`pub(crate)`); it must never become public.

## Adding features

Before proposing new wiring APIs, ask:

1. Can a plain constructor or `Module` scope express it already?
2. Does it push modrun toward reflection-style DI instead of Rust constructor injection?
3. Does it expand the method matrix (sync/async/fallible × public/private × group × dyn)?

Prefer documentation and examples over new registration verbs. See [ROADMAP.md](ROADMAP.md)
for current priorities.

## Development

```bash
cargo test
cargo test --doc
cargo run --example basic
```

Please run tests before opening a PR. Commit messages use Conventional Commits in Chinese
(项目惯例).
