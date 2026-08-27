# modrun

Lightweight application wiring for Tokio services: register constructors, pull the
dependency graph, and manage start/stop in one place.

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

fn boot(lc: Lifecycle, server: Server) {
    lc.append(server).unwrap();
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

`run()` builds the graph, runs every OnStart hook, waits for Ctrl-C, SIGTERM, or
[`Shutdowner`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html), then runs every OnStop hook in reverse. Signal
handlers are installed at the start of `run()`, so a shutdown request during
build or start cancels that phase and unwinds any hooks that already started.
Disable the default `signal` feature if you only use `start()` or wait on
[`Shutdowner::wait`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html#method.wait) yourself.

## Concepts

**`provide`** registers a constructor. Nothing is built until something asks for it,
and each type is built at most once. `provide_result` takes a constructor returning
`Result<T, E>`; `provide_async` and `provide_result_async` take `async fn`s. When
several independent constructors are needed at once, modrun builds them
concurrently by DAG layer (shared dependencies still run first), then continues
in dependency order. That last pair is what you want for connection pools and
clients that handshake on creation.

**`supply`** hands the container a value you already have, skipping the constructor.

**`invoke`** pulls the graph. An invoker's parameters are the roots of what gets
built, so a type nobody invokes (directly or transitively) is never constructed.
Invokers run once, during build, and are where you normally register lifecycle hooks.

**`Lifecycle`** collects start/stop hooks. It is injected automatically, so any
constructor or invoker can take it as a parameter. OnStart hooks run in registration
order; OnStop hooks run in reverse. If a start hook fails or start is cancelled,
hooks that already finished OnStart are stopped again before the error propagates —
and any OnStop failures are retained in the returned error. A start hook that fails
or is cancelled mid-flight does **not** run its own OnStop. A stop-only hook (one
without OnStart) is considered active immediately and is included in unwind.
Register hooks during `invoke` (or from an OnStart factory);
`append` returns an error if start has already finished or stop has begun.
Implement [`Hook`](https://docs.rs/modrun/latest/modrun/trait.Hook.html) on a
struct when start and stop share state (`&mut self`). For one-off closures, use
[`hook()`](https://docs.rs/modrun/latest/modrun/fn.hook.html); those callbacks
are `FnOnce` and may consume what they capture. Hook and constructor
errors use [`Error`](https://docs.rs/modrun/latest/modrun/enum.Error.html) (`thiserror`); hooks should return
[`Error::hook`](https://docs.rs/modrun/latest/modrun/enum.Error.html#method.hook) or
[`Error::io`](https://docs.rs/modrun/latest/modrun/enum.Error.html#method.io) so the original
failure stays on \[`std::error::Error::source`]. `std::io::Error` converts with
\[`From`], so `listener.bind().await?` works inside a hook; prefer \[`Error::io`]
when you want a context label.

Give hooks a [`Hook::name`](https://docs.rs/modrun/latest/modrun/trait.Hook.html#method.name)
for logs. Constructors and invokers accept at most eight parameters.

Hook futures must be cancellation-safe. A start/stop timeout drops the in-progress
future, but it cannot cancel detached tasks created with `tokio::spawn`; retain a
handle or cancellation channel and shut those tasks down in OnStop (see the axum
example). Hook panics are treated as fatal programming errors and may bypass
lifecycle unwind.

**`Shutdowner`** is also injected automatically. Calling `shutdown()` unblocks `run()`
from inside the app, which is how you shut down in response to something other than a
signal.

The container is dropped when build finishes. Singletons stay alive only through
values you capture in hooks (or other `Clone` handles taken during invoke). modrun
wires startup; it is not a live service locator. Build is not transactional: if a
later invoker fails, earlier constructors may already have run their side effects
for that failed build call.

Dependencies must be `Clone` to inject by value, because singletons are cached
by type and cloned on inject. `Arc<T>` is registered as an alias, so a constructor
that returns `T` can be injected as `Arc<T>` without `T: Clone`. Put shared
mutable state behind an `Arc` inside your type:

```rust
use std::sync::Arc;

struct DbInner;

#[derive(Clone)]
struct Db(Arc<DbInner>);
```

Since the cache key is the type, a type can only be provided once. To wire two of the
same thing — a primary and a replica pool, say — give each its own newtype.

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

## Logging

Framework events (provide / supply / invoke / construct / OnStart / OnStop) are
emitted through [`tracing`](https://docs.rs/tracing) with target `modrun`, in the
same spirit as [uber/fx](https://github.com/uber-go/fx)'s `fxevent` logger. Install
a subscriber in your binary; without one the events are cheap no-ops:

```rust,no_run
fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "modrun=info".into()),
        )
        .init();
}
```

Or set `RUST_LOG=modrun=info` and call `tracing_subscriber::fmt::init()`.
Cancelled hooks and constructors emit `cancelled` events. A successful stop emits
`stopped`. Leak warnings on `RunningApp` go through tracing; debug builds also
print to stderr.

## Startup banner

[`ModrunBuilder::run`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.run) and
[`start`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.start) print a modrun
ASCII banner to stdout before wiring (Spring Boot style). Custom text
(or `include_str!("banner.txt")` in your crate):

```rust,no_run
# use modrun::Modrun;
Modrun::builder()
    .banner("my service")
    // ...
# ;
```

Disable with [`.no_banner()`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.no_banner).

## Failure modes

The graph is checked before anything is constructed, so these are build-time errors
rather than surprises later:

* a provider whose dependency nothing provides, even if nobody uses that provider
* a dependency cycle
* the same type provided twice

At runtime, `build_timeout`, `start_timeout`, and `stop_timeout` (15s by default)
bound graph construction, OnStart, and OnStop respectively. If a timeout is set
more than once on the builder, the last value wins. `no_build_timeout` /
`no_start_timeout` / `no_stop_timeout` disable the budget. `stop_timeout` also
budgets unwind after a failed or cancelled start. When the budget expires,
remaining OnStop hooks are abandoned and the timeout is reported as an error
rather than hanging.

`run()` treats Ctrl-C / SIGTERM / [`Shutdowner`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html)
during build or start as a graceful stop: it unwinds hooks that already started
and returns `Ok(())` if cleanup succeeds.

## Testing

`start()` builds and starts without waiting for a signal, returning a `RunningApp` you
can `stop()` yourself:

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
    .supply(Hits(Arc::clone(&hits)))
    .invoke(boot)
    .start()
    .await?;
assert_eq!(hits.load(Ordering::SeqCst), 1);
app.stop().await
# }
```

Tests that sleep in hooks should set an explicit timeout (or `no_start_timeout`);
the default budget is 15s.

## Examples

```bash
cargo run --example basic   # domain modules, private deps, an async constructor
cargo run --example axum    # HTTP server with graceful shutdown
```

## License

MIT
