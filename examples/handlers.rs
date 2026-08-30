//! Multiple domain modules each contribute a handler; boot collects them via [`Group`].
//!
//! ```bash
//! cargo run --example handlers
//! ```

use modrun::{Group, Hook, Lifecycle, Modrun, Module, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Handler {
    name: &'static str,
}

#[derive(Clone)]
struct Registry {
    names: Vec<&'static str>,
}

impl Registry {
    fn register(&mut self, name: &'static str) {
        self.names.push(name);
    }
}

impl Hook for Registry {
    async fn on_start(&mut self) -> Result<()> {
        println!("registered handlers (in order): {}", self.names.join(", "));
        Ok(())
    }

    async fn on_stop(&mut self) -> Result<()> {
        println!("goodbye");
        Ok(())
    }
}

fn new_registry() -> Registry {
    Registry { names: Vec::new() }
}

#[derive(Clone)]
struct UserRepo;

fn new_user_repo() -> UserRepo {
    UserRepo
}

fn user_handler(_repo: UserRepo) -> Handler {
    Handler { name: "user" }
}

fn user_domain() -> Module {
    Module::new("user")
        .provide_private(new_user_repo)
        .provide_group(user_handler)
}

#[derive(Clone)]
struct OrderRepo;

async fn connect_order_repo() -> OrderRepo {
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    OrderRepo
}

fn order_handler(_repo: OrderRepo) -> Handler {
    Handler { name: "order" }
}

fn order_domain() -> Module {
    Module::new("order")
        .provide_async_private(connect_order_repo)
        .provide_group(order_handler)
}

fn boot(lc: Lifecycle, registry: Registry, handlers: Group<Handler>) -> Result<()> {
    let mut registry = registry;
    for handler in handlers {
        registry.register(handler.name);
    }
    lc.append(registry)
}

#[tokio::main]
async fn main() -> Result<()> {
    modrun::logging::init();

    Modrun::builder()
        .provide(new_registry)
        .module(user_domain())
        .module(order_domain())
        .invoke(boot)
        .start()
        .await?
        .stop()
        .await
}
