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
| Domain modules | `Module` + `Module::builder("name")` + `provide_private` |
| Value groups | `Group<T>` + `provide_group*` / `supply_group` / `init_group` / `require_group` |
| Lifecycle | `Lifecycle`, [`Hook`](src/lifecycle.rs), `task` / `task_with` |
| Run / stop | `ModrunBuilder::run` / `start`, `RunningApp::stop`, `Shutdowner` |

[`Hook`](src/lifecycle.rs) trait methods may only gain **default** implementations in
minor releases. Constructor and invoker arity stays capped at **eight** parameters; pack
extra deps in a struct.

MSRV is **1.85** (edition 2024), tracked in `Cargo.toml` and CI.

**Application-facing API is stable since 1.0.0.**

## Semver (since 1.0.0)

| Release | Allowed changes |
|---------|-----------------|
| **Patch** | Docs, diagnostics, tracing fields, non-breaking bug fixes |
| **Minor** | New `Error` variants (`Error` is `#[non_exhaustive]`), new `Hook` methods with **default** impls only, new optional builder knobs that do not change existing call sites |
| **Major** | Remove or rename public items, change constructor/invoker arity cap (currently **eight**), expose service locator or string-keyed bindings |

**Not covered by stability:** `modrun::__wiring`, `#[doc(hidden)]` builder methods (`provide_dyn`, …),
and any `pub(crate)` internals.

## Performance expectations

Startup cost is dominated by **your** constructors (I/O, parsing), not the framework.
modrun validates the graph once, constructs singletons lazily along a precomputed wave
order, runs invokers, then lifecycle hooks — and drops the container. There is no
runtime lookup after build.

Documented patterns that keep overhead low: `Arc<T>` for shared heavy deps,
`Arc<Group<T>>` / `Group<Arc<T>>` for large groups, shallow modules, no `dot_graph()`
on hot paths, and `default-features = false` when you do not need `logging` / `signal`.
See README「Performance」/「性能」.

## Application code vs wrapper authors

**Application code** should register plain functions:

```rust
Modrun::builder()
    .provide(new_config)
    .provide(new_server)
    .invoke(boot)
```

**Wrapper libraries** that re-export modrun wiring use the hidden [`__wiring`](src/lib.rs)
module and `#[doc(hidden)]` builder methods. These are omitted from docs.rs navigation
and are not part of the application-facing stability promise:

```rust
use modrun::__wiring::{DynProvider, InvokeFn, ProviderFn};

let provider: DynProvider = my_ctor.into_provider();
builder.provide_dyn(provider).invoke_dyn(my_boot.into_invoke());
```

Available in `modrun::__wiring`:

* `ProviderFn`, `FallibleProviderFn`, `AsyncProviderFn`, `FallibleAsyncProviderFn`, `ProviderMarker`
* `InvokeFn`, `AsyncInvokeFn`
* `DynProvider`, `DynInvoker`

Builder: `provide_dyn`, `invoke_dyn`, `provide_group_dyn` (+ `_private` variants on `Module`),
all `#[doc(hidden)]`. There are no `*_mut` builder methods — chain on `self` instead.

## Deliberately rejected

We will not add APIs for:

* Property or field injection
* Runtime string tokens / named bindings (`"primary"` vs `"replica"`) — use newtypes
* Annotations, derives, or auto-scanning
* Macro-generated entire containers
* Service locator (`get<T>()` after build)
* Global singleton registries
* `fx.Populate`, `fx.Annotate`, or `fx.Replace` equivalents
* First-class `decorate` — use [wrapper constructors](examples/wrap.rs) instead
* Module-to-module event buses (not in scope today)

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

## Releasing

Publishing is automated by [`.github/workflows/release.yml`](.github/workflows/release.yml)
when a version tag is pushed.

**One-time setup:** add a crates.io API token as the repository secret
`CARGO_REGISTRY_TOKEN` (Settings → Secrets and variables → Actions).

**Per release:**

1. Update `CHANGELOG.md` (move notes from `Unreleased` into `## [x.y.z] - YYYY-MM-DD`).
2. Bump `version` in `Cargo.toml`.
3. Commit and push to `main`.
4. Tag and push:

   ```bash
   git tag v1.0.1
   git push origin v1.0.1
   ```

The workflow checks that the tag matches `Cargo.toml`, runs fmt/clippy/tests/doc, publishes
to crates.io, and opens a GitHub Release with the matching `CHANGELOG` section.
