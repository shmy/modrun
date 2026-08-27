//! Axum HTTP server assembled with modrun.
//!
//! Start and stop share the server task on the [`Hook`] value, so there is no
//! need to smuggle channel ends between two closures.
//!
//! ```bash
//! cargo run --example axum
//! curl http://127.0.0.1:3000/
//! curl http://127.0.0.1:3000/hello/modrun
//! # Ctrl-C for graceful shutdown
//! ```

use std::net::SocketAddr;

use axum::Router;
use axum::extract::{Path, State};
use axum::routing::get;
use modrun::{Hook, Lifecycle, Modrun, Module, Result, Shutdowner};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

#[derive(Clone)]
struct Config {
    addr: SocketAddr,
}

#[derive(Clone)]
struct Greeter {
    prefix: String,
}

#[derive(Clone)]
struct AppState {
    greeter: Greeter,
}

fn new_greeter() -> Greeter {
    Greeter {
        prefix: "hello".into(),
    }
}

fn new_state(greeter: Greeter) -> AppState {
    AppState { greeter }
}

async fn index(State(state): State<AppState>) -> String {
    format!("{} from modrun + axum\n", state.greeter.prefix)
}

async fn hello(State(state): State<AppState>, Path(name): Path<String>) -> String {
    format!("{}, {name}!\n", state.greeter.prefix)
}

struct HttpServer {
    cfg: Config,
    state: AppState,
    shutdown: Shutdowner,
    stop_tx: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<std::io::Result<()>>>,
}

impl Hook for HttpServer {
    fn name(&self) -> Option<&'static str> {
        Some("http.serve")
    }

    async fn on_start(&mut self) -> Result<()> {
        let app = Router::new()
            .route("/", get(index))
            .route("/hello/{name}", get(hello))
            .with_state(self.state.clone());

        let addr = self.cfg.addr;
        let listener = TcpListener::bind(addr).await?;
        println!("listening on http://{addr}");

        let (stop_tx, stop_rx) = oneshot::channel();
        let shutdown = self.shutdown.clone();
        self.task = Some(tokio::spawn(async move {
            let result = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = stop_rx.await;
                })
                .await;
            if result.is_err() {
                // Wake the app so OnStop can surface the server error
                // instead of waiting forever for an external signal.
                shutdown.shutdown();
            }
            result
        }));
        self.stop_tx = Some(stop_tx);
        Ok(())
    }

    async fn on_stop(&mut self) -> Result<()> {
        let _ = self.stop_tx.take().map(|tx| tx.send(()));
        if let Some(task) = self.task.take() {
            match task.await {
                Ok(result) => result?,
                Err(join) => return Err(modrun::Error::hook(join)),
            }
        }
        Ok(())
    }
}

fn register_http(lc: Lifecycle, cfg: Config, state: AppState, shutdown: Shutdowner) {
    lc.append(HttpServer {
        cfg,
        state,
        shutdown,
        stop_tx: None,
        task: None,
    })
    .expect("register http hooks");
}

fn http_domain() -> Module {
    Module::new("http")
        .provide_private(new_greeter)
        .provide(new_state)
        .invoke(register_http)
}

#[tokio::main]
async fn main() -> modrun::Result<()> {
    tracing_subscriber::fmt().init();

    Modrun::builder()
        .supply(Config {
            addr: SocketAddr::from(([127, 0, 0, 1], 3000)),
        })
        .module(http_domain())
        .run()
        .await
}
