//! Domain-style modules with a fluent builder API.
//!
//! ```bash
//! cargo run --example basic
//! ```

use std::time::Duration;

use modrun::{Hook, Lifecycle, Modrun, Module, Result};

#[derive(Clone)]
struct AppConfig {
    name: String,
}

#[derive(Clone)]
struct GreeterRepo {
    prefix: String,
}

#[derive(Clone)]
struct Greeter {
    repo: GreeterRepo,
}

/// An async constructor: awaited while the graph is built, so `Greeter` below
/// receives a repo that is already connected.
async fn new_repo(cfg: AppConfig) -> GreeterRepo {
    tokio::time::sleep(Duration::from_millis(10)).await;
    GreeterRepo {
        prefix: format!("hello from {}", cfg.name),
    }
}

fn new_greeter(repo: GreeterRepo) -> Greeter {
    Greeter { repo }
}

impl Hook for Greeter {
    async fn on_start(&mut self) -> Result<()> {
        println!("{}", self.repo.prefix);
        Ok(())
    }

    async fn on_stop(&mut self) -> Result<()> {
        println!("goodbye");
        Ok(())
    }
}

fn register(lc: Lifecycle, greeter: Greeter) -> modrun::Result<()> {
    lc.append(greeter)
}

fn greeter_domain() -> Module {
    Module::new("greeter")
        .provide_async_private(new_repo)
        .provide(new_greeter)
        .invoke(register)
}

#[tokio::main]
async fn main() -> modrun::Result<()> {
    tracing_subscriber::fmt().init();

    Modrun::builder()
        .supply(AppConfig {
            name: "modrun".into(),
        })
        .module(greeter_domain())
        .start()
        .await?
        .stop()
        .await
}
