# modrun

[English](README.md) | [简体中文](README_zh.md)

Lightweight application wiring for Tokio services: register constructors, pull the
dependency graph, and manage start/stop in one place.

Requires **Rust 1.85** or newer (edition 2024). This is not a general-purpose DI
container: there are no string qualifiers, no request-scoped objects, and no
`get<T>()` after the graph has been built. Two of the same type use newtypes;
swap test doubles with [`supply`](#concepts) at the composition root.

If you have written a `main` that builds a config, then a pool, then a repo, then a
service, then a server — and a shutdown path that has to unwind all of it in the
right order — modrun is that `main`, written once.

```rust,no_run
use modrun::{Hook, Lifecycle, Modrun};

#[derive(Clone)]
struct Config {
    port: u16,
}

#[derive(Clone)]
struct Server {
    cfg: Config,
}

impl Hook for Server {
    async fn on_start(&mut self) -> modrun::Result<()> {
        println!("listening on {}", self.cfg.port);
        Ok(())
    }

    async fn on_stop(&mut self) -> modrun::Result<()> {
        println!("goodbye");
        Ok(())
    }
}

fn new_config() -> Config {
    Config { port: 8080 }
}

fn new_server(cfg: Config) -> Server {
    Server { cfg }
}

fn boot(lc: Lifecycle, server: Server) -> modrun::Result<()> {
    lc.append(server)
}

#[tokio::main]
async fn main() -> modrun::Result<()> {
    Modrun::builder()
        .provide(new_config)
        .provide(new_server)
        .invoke(boot)
        .run()
        .await
}
```

`run()` builds the graph, runs every OnStart hook, waits for an OS signal or
[`Shutdowner`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html), then runs every OnStop hook in reverse.
With the default `signal` feature, handlers are installed at the start of
`run()` on **Unix** (Ctrl-C / SIGTERM) and **Windows** (Ctrl-C / Ctrl-Break /
Ctrl-Close / Ctrl-Shutdown); on other targets only [`Shutdowner`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html)
unblocks `run()`. A shutdown request during build or start cancels that phase
and unwinds hooks that already started, plus any stop-only hooks already
registered (even if OnStart never ran). A timeout or hook failure still returns
an error; a concurrent shutdown does not turn that into `Ok(())`.
Disable the default `signal` feature if you only use `start()` or wait on
[`Shutdowner::wait`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html#method.wait) yourself.

## Concepts

**`provide`** registers a constructor. Nothing is built until something asks for it,
and each type is built at most once. Constructors that return `Result<T, E>` must
use `provide_result` (or `provide_result_async`); handing one to plain `provide`
is a compile error. `provide_async` and `provide_result_async` take `async fn`s.
When several independent constructors are needed at once, modrun builds them by
DAG layer: **async** constructors in the same layer are polled concurrently on one
task; **sync** constructors run inside `construct()` and defer creation of later
futures in that layer. Shared dependencies still run first, then dependents
continue in dependency order.

**`supply`** hands the container a value you already have, skipping the constructor.

**`invoke`** pulls the graph. An invoker's parameters are the roots of what gets
built, so a type nobody invokes (directly or transitively) is never constructed.
A migrator or background consumer that is only `provide`d will stay inert until
something `invoke`s it (or a type that depends on it). Invokers run once, during
build, and are where you normally register lifecycle hooks.

**`Lifecycle`** collects start/stop hooks. It is injected automatically, so any
constructor or invoker can take it as a parameter. OnStart hooks run in registration
order; OnStop hooks run in reverse. If a start hook fails or start is cancelled,
hooks that already finished OnStart are stopped again before the error propagates —
and any OnStop failures are retained in the returned error. A start hook that fails
or is cancelled mid-flight does **not** run its own OnStop. A stop-only hook (one
without OnStart) is considered active immediately and is included in unwind.
Register hooks during `invoke` (or from an OnStart factory);
`append` returns an error if start has already finished or stop has begun. An
invoker may itself return `modrun::Result<()>`, so give `boot` that return type
and hand the `append` result straight back instead of unwrapping it.
Implement [`Hook`](https://docs.rs/modrun/latest/modrun/trait.Hook.html) on a
struct when start and stop share state (`&mut self`). For a struct that only
implements OnStop, override [`has_start`](https://docs.rs/modrun/latest/modrun/trait.Hook.html#method.has_start)
to return `false` so trailing activation still runs it after a failed start.
For one-off closures, use
[`hook()`](https://docs.rs/modrun/latest/modrun/fn.hook.html); OnStop callbacks
must be repeatable (`Fn`) when they capture shared state — clone an [`Arc`](std::sync::Arc)
inside the closure on each call. Hook and constructor
errors use [`Error`](https://docs.rs/modrun/latest/modrun/enum.Error.html) (`thiserror`); hooks should return
[`Error::hook`](https://docs.rs/modrun/latest/modrun/enum.Error.html#method.hook) or
[`Error::io`](https://docs.rs/modrun/latest/modrun/enum.Error.html#method.io) so the original
failure stays on [`std::error::Error::source`](std::error::Error::source). There is no
[`From<std::io::Error>`](std::convert::From); wrap I/O with [`Error::io`](https://docs.rs/modrun/latest/modrun/enum.Error.html#method.io)
(`bind(addr).await.map_err(|e| Error::io(format!("bind {addr}"), e))?`).

Override [`Hook::name`](https://docs.rs/modrun/latest/modrun/trait.Hook.html#method.name) from the default
`"unnamed"` for clearer logs and errors. Constructors and invokers accept at most eight parameters; group extra
dependencies in a struct rather than stretching arity.

Hook futures must be cancellation-safe. A start/stop timeout drops the in-progress
future, but it cannot cancel detached tasks created with `tokio::spawn`. Prefer
[`task()`](https://docs.rs/modrun/latest/modrun/fn.task.html) for workers, or
[`task_with()`](https://docs.rs/modrun/latest/modrun/fn.task_with.html) when
bind/listen must finish during OnStart (see the axum example). Both signal
[`Stopped`](https://docs.rs/modrun/latest/modrun/struct.Stopped.html) on OnStop,
join, and abort if the hook is dropped mid-flight. If that background work
returns `Err` or panics after start has succeeded, shutdown is requested
automatically so `run()` does not wait forever for a signal. Custom tasks
spawned with [`tokio::spawn`](https://docs.rs/tokio/latest/tokio/fn.spawn.html)
must still call
[`Shutdowner::shutdown`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html#method.shutdown).
Hook panics are treated as fatal programming errors and may bypass lifecycle
unwind (logged as `panicked`).

**`Shutdowner`** is also injected automatically. Calling `shutdown()` unblocks `run()`
from inside the app, which is how you shut down in response to something other than a
signal. During build/start that cancellation is cooperative (next `.await`).

The container is dropped when build finishes. Singletons stay alive only through
values you capture in hooks (or other `Clone` handles taken during invoke). modrun
wires startup; it is not a live service locator. Build is not transactional: if a
later invoker fails, earlier constructors may already have run their side effects
for that failed build call.

Dependencies must be `Clone` to inject by value, because singletons are cached
by type and cloned on inject. Prefer `Arc<T>` when a type is large, shared across
many constructors, or injected more than once — `Arc<T>` is registered as an alias,
so a constructor that returns `T` can be injected as `Arc<T>` without `T: Clone`.
Put shared mutable state behind an `Arc` inside your type:

```rust
use std::sync::Arc;

struct DbInner;

#[derive(Clone)]
struct Db(Arc<DbInner>);
```

Since the cache key is the type, a type can only be provided once. To wire two of the
same thing — a primary and a replica pool, say — give each its own newtype:

```rust
# use std::sync::Arc;
# struct PgPool;
#[derive(Clone)]
struct PrimaryDb(Arc<PgPool>);
#[derive(Clone)]
struct ReplicaDb(Arc<PgPool>);
# let _ = std::any::type_name::<(PrimaryDb, ReplicaDb)>();
```

Connect pools with `provide_result_async` at the composition root (so tests can
`supply` a fake). Do not open connections inside OnStart unless you want pool
failure to look like a start-hook failure rather than a constructor error.

## Modules

A `Module` groups related wiring under a name and gives it a private scope.
`provide_private` keeps a type invisible outside the module, so two domains can each
have their own `Repo` without colliding:

```rust
use modrun::{Modrun, Module};

# #[derive(Clone)] struct UserRepo;
# #[derive(Clone)] struct UserService;
# fn new_user_repo() -> UserRepo { UserRepo }
# fn new_user_service(_r: UserRepo) -> UserService { UserService }
# fn boot_user(_s: UserService) {}
fn user_domain() -> Module {
    Module::new("user")
        .provide_private(new_user_repo)
        .provide(new_user_service)
        .invoke(boot_user)
}

# #[tokio::main]
# async fn main() -> modrun::Result<()> {
Modrun::builder()
    .module(user_domain())
    .start()
    .await?
    .stop()
    .await
# }
```

Private types are visible to the module that declared them and to its nested modules.
Everything registered with plain `provide` or `supply` is visible everywhere, wherever
it was declared. `provide_private` / `supply_private` exist only on [`Module`](https://docs.rs/modrun/latest/modrun/struct.Module.html),
not on the root builder.

## Groups

Multiple modules can each contribute one value of the same type; a consumer receives
them as [`Group<T>`](https://docs.rs/modrun/latest/modrun/struct.Group.html) (not
`Vec<T>`). Register members with [`provide_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.provide_group)
(and `provide_group_async` / `provide_group_result` / `provide_group_result_async`
when needed). Members are aggregated in registration order and do **not** occupy the
singleton slot for `T`, so they can coexist with a separate `provide` of the same type.

```rust
use modrun::{Group, Modrun, Module};

# #[derive(Clone, PartialEq, Eq)] struct Handler(&'static str);
# fn user_handler() -> Handler { Handler("user") }
# fn order_handler() -> Handler { Handler("order") }
# fn boot(_: Group<Handler>) {}

Modrun::builder()
    .module(Module::new("user").provide_group(user_handler))
    .module(Module::new("order").provide_group(order_handler))
    .invoke(boot)
# ;
```

Inject `Group<T>` (or `Arc<Group<T>>`) in an invoker or constructor; iterate with
`for item in group`. Members and injected groups require `T: Clone`; prefer
`Arc<Group<T>>` when several consumers need the same collection, or return
`Arc<T>` / `Arc<dyn Trait>` from group member constructors when values are heavy.
With no members, register the empty group with
[`init_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.init_group)
or [`require_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.require_group)
(`require_group` also fails the build if the group stays empty; only the composition
root can call it). `T` is inferred from the constructor return type (no `provide_group::<T>`
turbofish); for a trait-object group, return `Arc<dyn Trait>`. A module-private
[`provide_private`](https://docs.rs/modrun/latest/modrun/struct.Module.html#method.provide_private)
of `Group<T>` shadows the aggregated group inside that module — use
[`provide_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.provide_group)
to contribute members instead. For two groups of the same element type, use
newtypes (same as duplicate singletons).

**Method matrix** (all have `_mut` on [`ModrunBuilder`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html)
and [`Module`](https://docs.rs/modrun/latest/modrun/struct.Module.html) where applicable):

| Register | Use when |
|----------|----------|
| `provide_group` | sync infallible member ctor |
| `provide_group_result` | sync fallible (`Result`) |
| `provide_group_async` | async infallible |
| `provide_group_result_async` | async fallible |
| `supply_group` | pre-built member value |
| `provide_group_dyn` | erased ctor from `into_provider()` |
| `init_group` / `require_group` | composition root only; empty vs non-empty policy |
| `init_group_mut` / `require_group_mut` | same, for `&mut self` builders |

## Dependency graph

Export the wiring graph as [Graphviz DOT](https://graphviz.org/doc/info/lang.html) for
documentation or debugging. Validation runs first (missing providers and cycles surface
as errors), but no constructor or invoker runs.

```rust
use modrun::Modrun;

# #[derive(Clone)] struct Config;
# #[derive(Clone)] struct Server;
# fn new_config() -> Config { Config }
# fn new_server(_: Config) -> Server { Server }
# fn boot(_: Server) {}

// Return DOT as a string (no file I/O)
let dot = Modrun::builder()
    .provide(new_config)
    .provide(new_server)
    .invoke(boot)
    .render_dot()?;
# Ok::<(), modrun::Error>(())
```

To write a file before graph construction when starting the app, chain
[`.dot_graph("modrun.dot")`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.dot_graph)
on the builder passed to [`run`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.run).

Nodes show the type, constructor name, and module scope (subgraph per module).
Solid arrows are constructor / invoker dependencies; dotted edges link group members
to their `Group<T>` aggregate. Built-in `Lifecycle` and `Shutdowner` nodes are
omitted for clarity. See [docs/graph-sample.dot](docs/graph-sample.dot) for sample
output. Render with `dot -Tpng modrun.dot -o modrun.png`.

## Logging

Framework events (provide / supply / invoke / construct / OnStart / OnStop) are
emitted through [`tracing`](https://docs.rs/tracing) with target `modrun`, using
[uber/fx](https://github.com/uber-go/fx)-style console lines such as
`[modrun] PROVIDE    my::Type <= my::new`. The same events carry structured
fields (`constructor`, `module`, `elapsed_ms`, `error`, …) so a JSON subscriber
can filter them in production.

[`modrun::logging::init()`](https://docs.rs/modrun/latest/modrun/logging/fn.init.html)
is for examples and local binaries (enabled by default via the `logging`
feature). It writes to stderr, enables ANSI only when stderr is a TTY, and is a
**no-op** if a subscriber is already installed (it will not panic). Production
services should install their own subscriber and skip this helper. Without a
subscriber the events are cheap no-ops:

```rust,no_run
fn main() {
    #[cfg(feature = "logging")]
    modrun::logging::init();
}
```

Set `RUST_LOG=modrun=info` with your own subscriber. Cancelled hooks and
constructors emit `ERROR` lines. A successful stop emits `STOPPED`. Leak
warnings on `RunningApp` go through tracing; debug builds also print to stderr.

## Startup banner

[`ModrunBuilder::run`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.run) and
[`start`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.start) print a modrun
ASCII banner to stderr before wiring (Spring Boot style), and only when stderr
is a TTY. Custom text (or `include_str!("banner.txt")` in your crate) is always
printed to stderr:

```rust,no_run
# use modrun::Modrun;
Modrun::builder()
    .banner("my service")
    // ...
# ;
```

Disable with [`.no_banner()`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.no_banner).
Piped processes and daemons without a TTY skip the default banner automatically.
Tests that run in a terminal should still call `.no_banner()` so captured stderr
stays quiet.

## Failure modes

The graph is checked before anything is constructed, so these are build-time errors
rather than surprises later:

* a provider whose dependency nothing provides, even if nobody uses that provider
* a dependency cycle
* the same type provided twice

At runtime, `build_timeout`, `start_timeout`, and `stop_timeout` (15s by default)
bound graph construction, OnStart, and OnStop respectively. Timeouts are
cooperative: work that yields at `.await` is cancelled when the budget expires.
Synchronous blocking (for example `std::thread::sleep` in a sync invoker,
constructor, or hook) cannot be preempted, but an over-budget success is still
reported as a timeout error rather than `Ok`. The cancellation timer follows
Tokio's clock; the over-budget `Ok` check uses wall-clock `Instant`, so
`tokio::time::pause` in tests can make those two disagree. If a timeout is set more than once
on the builder, the last value wins. `no_build_timeout` / `no_start_timeout` /
`no_stop_timeout` disable the budget. `no_start_timeout` only disables OnStart;
`stop_timeout` still budgets unwind after a failed or cancelled start. When the
budget expires, remaining OnStop hooks are abandoned and the timeout is reported
as an error rather than hanging — connection pools may not get a clean close.
For production start that runs migrations or cache warm-up, set an explicit
`start_timeout` (and `stop_timeout`) instead of relying on the 15s default.

`run()` treats Ctrl-C / SIGTERM / [`Shutdowner`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html)
during build or start as a graceful stop: it unwinds hooks that already started
and any registered stop-only hooks, and returns `Ok(())` if cleanup succeeds.
A background [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) that
fails or panics during start is **not** treated as graceful — `run()` returns
the join error (or `background task failed during start` if unwind reported
success). If unwind then times out, both are retained on
[`Error::CleanupAfterFailure`](https://docs.rs/modrun/latest/modrun/enum.Error.html).
If that phase had already failed, `run()` still returns the failure.
Shutdown and OS signals are cooperative in the same way as timeouts: they take
effect at the next `.await`, so a `shutdown()` from a synchronous OnStart does
not skip later hooks that have not yet yielded. After `RUNNING`, `run()` waits
until a signal or `Shutdowner::shutdown()`; a background [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html)
that fails or panics requests shutdown on its own. Work spawned with
[`tokio::spawn`](https://docs.rs/tokio/latest/tokio/fn.spawn.html) must still
call `shutdown()` or `run()` waits forever. A panic in a
constructor, invoker, or hook is not converted into [`Error`](https://docs.rs/modrun/latest/modrun/enum.Error.html)
and may skip lifecycle unwind (tracing records it as `panicked`).

## Testing

`start()` builds and starts without waiting for a signal, returning a `RunningApp` you
can `stop()` yourself. A background [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html)
that fails after its OnStart has returned does **not** fail `start()` or skip later
hooks; wait on [`Shutdowner`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html)
or call `stop()` to observe it. Use [`run`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.run)
when a failed worker should tear the process down:

```rust
# use std::sync::Arc;
# use std::sync::atomic::{AtomicUsize, Ordering};
# use modrun::{hook, Lifecycle, Modrun};
# #[derive(Clone)] struct Hits(Arc<AtomicUsize>);
# fn boot(lc: Lifecycle, hits: Hits) {
#     let n = Arc::clone(&hits.0);
#     lc.append(hook().on_start(move || async move {
#         n.fetch_add(1, Ordering::SeqCst);
#         Ok(())
#     })).unwrap();
# }
# #[tokio::main]
# async fn main() -> modrun::Result<()> {
let hits = Arc::new(AtomicUsize::new(0));
let app = Modrun::builder()
    .no_banner()
    .supply(Hits(Arc::clone(&hits)))
    .invoke(boot)
    .start()
    .await?;
assert_eq!(hits.load(Ordering::SeqCst), 1);
app.stop().await
# }
```

Replaceable types belong at the composition root (`provide` / `supply`), not
inside the domain [`Module`](https://docs.rs/modrun/latest/modrun/struct.Module.html).
If the module also `provide`s `Repo`, a test `supply(FakeRepo)` will hit
`already provided`. Runnable version: `cargo run --example swap`.

```rust
# use modrun::{Modrun, Module};
# #[derive(Clone)] struct Repo;
# #[derive(Clone)] struct Service;
# fn connect_repo() -> Repo { Repo }
# fn fake_repo() -> Repo { Repo }
# fn new_service(_: Repo) -> Service { Service }
# fn boot(_: Service) {}
fn user_domain() -> Module {
    Module::new("user")
        .provide(new_service)
        .invoke(boot)
}

# #[tokio::main]
# async fn main() -> modrun::Result<()> {
// production
Modrun::builder()
    .no_banner()
    .provide(connect_repo)
    .module(user_domain())
    .start()
    .await?
    .stop()
    .await?;

// test: same module, fake at the root
Modrun::builder()
    .no_banner()
    .supply(fake_repo())
    .module(user_domain())
    .start()
    .await?
    .stop()
    .await
# }
```

Tests that sleep in hooks should set an explicit timeout (or `no_start_timeout`);
the default budget is 15s. Prefer [`.no_banner()`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.no_banner)
so stderr stays quiet when tests run in a terminal.

## Errors

Graph problems fail before constructors run. Typical `Display` text:

* `type already provided: my::Config`
* `invoker in module '<root>' needs a dependency nothing provides: my::Db`
* `provider for my::Svc in module 'user' needs a dependency nothing provides: my::Repo`
* `dependency cycle detected involving: A -> B -> A`
* `application start timed out after 15s`
* `application stop timed out after 15s while unwinding`
* `invoker my::boot failed: …`
* `hook 'http.serve' failed: …`
* `background task failed during start`
* `required group is empty: modrun::Group<my::Route>`
* `provide_group_dyn type mismatch: expected my::Route, got alloc::string::String`
* `invoker in module '<root>' needs a dependency nothing provides: modrun::Group<my::Route>; register the group with init_group, provide_group, or require_group`

Constructor and hook failures keep the original error on
[`std::error::Error::source`](https://doc.rust-lang.org/std/error/trait.Error.html#tymethod.source).
Every hook has a [`Hook::name`](https://docs.rs/modrun/latest/modrun/trait.Hook.html#method.name)
in logs and errors (default `"unnamed"`; [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) sets its own).
Several OnStop failures become [`MultipleStopError`](https://docs.rs/modrun/latest/modrun/struct.MultipleStopError.html).
If unwind fails after an earlier phase error, both are retained on
[`Error::CleanupAfterFailure`](https://docs.rs/modrun/latest/modrun/enum.Error.html).

## When not to use this

* Request-scoped objects (one instance per HTTP request)
* Looking up a type from a live container after `start()`
* String-named bindings (`"primary"` vs `"replica"`) — use newtypes
* Runtime-agnostic libraries; modrun targets Tokio

## Examples

```bash
cargo run --example basic    # domain modules, private deps, an async constructor
cargo run --example handlers # multiple modules contribute to a Group
cargo run --example worker   # newtype pools + task that selects on Stopped
cargo run --example swap     # supply a fake at the composition root (tests)
cargo run --example axum     # HTTP server: task_with binds in OnStart, then serve
```

## License

MIT
