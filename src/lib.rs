//! modrun — lightweight app wiring for Tokio services.
//!
//! Register constructors, pull the graph with invokers, and manage start/stop
//! via [`Lifecycle`].
//!
//! # Quick start
//!
//! ```no_run
//! use modrun::{Hook, Lifecycle, Modrun};
//!
//! #[derive(Clone)]
//! struct Config { port: u16 }
//!
//! #[derive(Clone)]
//! struct Server { cfg: Config }
//!
//! impl Hook for Server {
//!     async fn on_start(&mut self) -> modrun::Result<()> {
//!         println!("listening on {}", self.cfg.port);
//!         Ok(())
//!     }
//!
//!     async fn on_stop(&mut self) -> modrun::Result<()> {
//!         Ok(())
//!     }
//! }
//!
//! fn new_config() -> Config { Config { port: 8080 } }
//! fn new_server(cfg: Config) -> Server { Server { cfg } }
//!
//! fn boot(lc: Lifecycle, server: Server) -> modrun::Result<()> {
//!     lc.append(server)
//! }
//!
//! #[tokio::main]
//! async fn main() -> modrun::Result<()> {
//!     Modrun::builder()
//!         .provide(new_config)
//!         .provide(new_server)
//!         .invoke(boot)
//!         .run()
//!         .await
//! }
//! ```
//!
//! Dependencies must be `Clone` to inject by value. Singletons are cached by
//! type; `Arc<T>` is also stored, so you can inject `Arc<T>` even when `T`
//! itself is not `Clone`. Put shared mutable state behind `Arc` inside your
//! type, e.g. `#[derive(Clone)] struct Db(Arc<DbInner>)`.
//!
//! Because the cache key is the type, a type can only be provided once. To wire
//! two of the same thing — a primary and a replica pool, say — give each its own
//! newtype wrapper.
//!
//! # Async constructors
//!
//! Anything that needs to `await` before it exists (connection pools, clients
//! that handshake) can be provided directly:
//!
//! ```
//! use modrun::Modrun;
//!
//! #[derive(Clone)]
//! struct Pool;
//!
//! async fn connect() -> Result<Pool, std::io::Error> {
//!     Ok(Pool)
//! }
//!
//! #[tokio::main]
//! async fn main() -> modrun::Result<()> {
//!     Modrun::builder()
//!         .provide_result_async(connect)
//!         .invoke(|_pool: Pool| {})
//!         .start()
//!         .await?
//!         .stop()
//!         .await
//! }
//! ```
//!
//! # Domain modules
//!
//! A [`Module`] groups related wiring under a name and gives it a private scope.
//! `provide_private` keeps a type invisible outside the module, so two modules
//! can each hold their own `Repo` without colliding.
//!
//! ```
//! use modrun::{Modrun, Module};
//!
//! #[derive(Clone)]
//! struct UserRepo;
//! #[derive(Clone)]
//! struct UserService;
//!
//! fn new_user_repo() -> UserRepo { UserRepo }
//! fn new_user_service(_repo: UserRepo) -> UserService { UserService }
//! fn boot_user(_svc: UserService) {}
//!
//! fn user_domain() -> Module {
//!     Module::new("user")
//!         .provide_private(new_user_repo)
//!         .provide(new_user_service)
//!         .invoke(boot_user)
//! }
//!
//! #[tokio::main]
//! async fn main() -> modrun::Result<()> {
//!     Modrun::builder()
//!         .module(user_domain())
//!         .start()
//!         .await?
//!         .stop()
//!         .await
//! }
//! ```
//!
//! # Startup banner
//!
//! [`ModrunBuilder::run`] and [`ModrunBuilder::start`] print a modrun ASCII banner
//! to stdout before wiring begins (Spring Boot style). Override with
//! [`.banner(text)`](ModrunBuilder::banner) or disable with
//! [`.no_banner()`](ModrunBuilder::no_banner).
//!
//! # Logging
//!
//! Framework events (provide / supply / invoke / construct / lifecycle) are
//! emitted through [`tracing`] with target `modrun`, in the spirit of
//! [uber/fx](https://github.com/uber-go/fx)'s `fxevent` logger. Install a
//! subscriber in your binary, then filter with `RUST_LOG=modrun=info` (or an
//! equivalent `EnvFilter`). Without a subscriber the events are cheap no-ops.
//! Debug builds also print to stderr if a `RunningApp` is dropped without
//! [`RunningApp::stop`].
//!
//! ```no_run
//! tracing_subscriber::fmt::init();
//! ```
//!
//! # Crate features
//!
//! * **`signal`** *(enabled by default)* — Ctrl-C / SIGTERM listeners in
//!   [`ModrunBuilder::run`]. Disable with `default-features = false` when you
//!   only call [`ModrunBuilder::start`] or wait on [`Shutdowner`] yourself.

#![forbid(unsafe_code)]

mod app;
mod banner;
mod container;
mod deps;
mod error;
mod future;
mod invoke;
mod lifecycle;
mod module;
mod option;
mod provide;
mod scope;
mod shutdown;
mod supply;
mod timeout;
mod trace;
mod wiring;

pub use app::{Modrun, ModrunBuilder, RunningApp};
pub use banner::DEFAULT_BANNER;
pub use error::{BoxError, Error, MultipleStopError, Result};
pub use lifecycle::{Hook, HookFn, Lifecycle, hook};
pub use module::Module;
pub use shutdown::Shutdowner;
pub use timeout::DEFAULT_TIMEOUT;

/// Constructor and invoker bounds, for code that wraps modrun's wiring API.
/// Convert with [`ProviderFn::into_provider`] / [`InvokeFn::into_invoke`], then
/// register with [`ModrunBuilder::provide_dyn`] / [`ModrunBuilder::invoke_dyn`].
pub use invoke::{AsyncInvokeFn, DynInvoker, InvokeFn};
pub use provide::{
    AsyncProviderFn, DynProvider, FallibleAsyncProviderFn, FallibleProviderFn, ProviderFn,
};

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeDoctests;
