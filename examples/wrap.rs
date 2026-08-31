//! Wrapper constructors for cross-cutting concerns — the Rust alternative to a
//! `decorate` API.
//!
//! Each type can only be `provide`d once. Compose wrappers in one constructor,
//! or expose a newtype / service struct from a module (`provide_private` raw value
//! + public wrapper returning a different type).
//!
//! ```bash
//! cargo run --example wrap
//! ```

use modrun::{Modrun, Module, Result};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Logger {
    name: &'static str,
    metrics: bool,
}

impl Logger {
    fn with_name(self, name: &'static str) -> Self {
        Self { name, ..self }
    }

    fn with_metrics(self) -> Self {
        Self {
            metrics: true,
            ..self
        }
    }
}

/// Public face of the logging module — distinct from the private [`Logger`] binding.
#[derive(Clone, PartialEq, Eq)]
struct AppLogger(Logger);

fn new_logger() -> Logger {
    Logger {
        name: "default",
        metrics: false,
    }
}

fn named_logger(log: Logger) -> Logger {
    log.with_name("myapp")
}

fn metrics_logger(log: Logger) -> Logger {
    log.with_metrics()
}

fn new_app_logger(log: Logger) -> AppLogger {
    AppLogger(metrics_logger(named_logger(log)))
}

fn logging_domain() -> Module {
    Module::new("logging")
        .provide_private(new_logger)
        .provide(new_app_logger)
        .invoke(|log: AppLogger| {
            assert_eq!(log.0.name, "myapp");
            assert!(log.0.metrics);
        })
}

/// At the composition root, a single constructor is enough when you do not need a
/// private binding:
fn app_logger() -> Logger {
    metrics_logger(named_logger(new_logger()))
}

fn boot(log: Logger) {
    assert_eq!(log.name, "myapp");
    assert!(log.metrics);
}

#[tokio::main]
async fn main() -> Result<()> {
    modrun::logging::init();

    println!("-- module + newtype wrapper --");
    Modrun::builder()
        .module(logging_domain())
        .start()
        .await?
        .stop()
        .await?;

    println!("-- root + composed ctor --");
    Modrun::builder()
        .provide(app_logger)
        .invoke(boot)
        .start()
        .await?
        .stop()
        .await
}
