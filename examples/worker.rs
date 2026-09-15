//! Async constructors, newtype pools, and a background [`modrun::task`].
//!
//! Mirrors a typical SQL primary/replica + worker shape without extra crates.
//! The worker loop selects on [`modrun::Stopped`] so OnStop is graceful rather
//! than an abort. If this work returns `Err` or panics, [`modrun::task`]
//! requests shutdown on its own so `run()` does not wait forever for a signal.
//!
//! ```bash
//! cargo run --example worker
//! ```

use std::io;
use std::time::Duration;

use modrun::logging::init;
use modrun::{Lifecycle, Modrun, Module, task};
use tokio::time;
use tokio::time::sleep;

#[derive(Clone)]
struct Config {
    name: String,
}

#[derive(Clone)]
struct PrimaryPool(String);

#[derive(Clone)]
struct ReplicaPool(String);

async fn connect_primary(cfg: Config) -> Result<PrimaryPool, io::Error> {
    sleep(Duration::from_millis(5)).await;
    Ok(PrimaryPool(format!("{}-primary", cfg.name)))
}

async fn connect_replica(cfg: Config) -> Result<ReplicaPool, io::Error> {
    sleep(Duration::from_millis(5)).await;
    Ok(ReplicaPool(format!("{}-replica", cfg.name)))
}

fn boot(lc: Lifecycle, primary: PrimaryPool, replica: ReplicaPool) -> modrun::Result<()> {
    lc.append(task("worker", move |stopped| async move {
        println!("worker using {} / {}", primary.0, replica.0);
        let mut ticks = 0u32;
        let mut interval = time::interval(Duration::from_millis(40));
        tokio::pin!(stopped);
        loop {
            tokio::select! {
                _ = &mut stopped => {
                    println!("worker stopping after {ticks} ticks");
                    return Ok(());
                }
                _ = interval.tick() => {
                    ticks += 1;
                    println!("tick {ticks}");
                }
            }
        }
    }))
}

fn store() -> Module {
    Module::builder("store")
        .provide_result_async(connect_primary)
        .provide_result_async(connect_replica)
        .invoke(boot)
}

#[tokio::main]
async fn main() -> modrun::Result<()> {
    init();

    let app = Modrun::builder()
        .supply(Config {
            name: "modrun".into(),
        })
        .module(store())
        .start()
        .await?;
    sleep(Duration::from_millis(120)).await;
    app.stop().await
}
