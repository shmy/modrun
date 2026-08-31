//! Swap a replaceable dependency at the composition root.
//!
//! Domain modules depend on `Repo` but do not `provide` it. Production wires a
//! real constructor; tests `supply` a fake. Providing `Repo` inside the module
//! would make `supply(fake)` fail with "already provided".
//!
//! ```bash
//! cargo run --example swap
//! ```

use modrun::{Lifecycle, Modrun, Module, hook};

#[derive(Clone)]
struct Repo {
    label: &'static str,
}

#[derive(Clone)]
struct Service {
    repo: Repo,
}

fn connect_repo() -> Repo {
    Repo { label: "postgres" }
}

fn fake_repo() -> Repo {
    Repo { label: "fake" }
}

fn new_service(repo: Repo) -> Service {
    Service { repo }
}

fn boot(lc: Lifecycle, service: Service) -> modrun::Result<()> {
    let label = service.repo.label;
    lc.append(hook().name("svc").on_start(move || async move {
        println!("service using {label}");
        Ok(())
    }))
}

fn user_domain() -> Module {
    Module::builder("user").provide(new_service).invoke(boot)
}

#[tokio::main]
async fn main() -> modrun::Result<()> {
    modrun::logging::init();

    println!("-- production --");
    Modrun::builder()
        .no_banner()
        .provide(connect_repo)
        .module(user_domain())
        .start()
        .await?
        .stop()
        .await?;

    println!("-- test --");
    Modrun::builder()
        .no_banner()
        .supply(fake_repo())
        .module(user_domain())
        .start()
        .await?
        .stop()
        .await
}
