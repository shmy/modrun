//! Axum HTTP server assembled with modrun.
//!
//! [`modrun::task_with`] binds during OnStart so `AddrInUse` fails start, then
//! owns the server task so OnStop can shut it down without smuggling channel
//! ends between two closures.
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
use modrun::{Error, Lifecycle, Modrun, Module, task_with};

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

fn register_http(lc: Lifecycle, cfg: Config, state: AppState) -> modrun::Result<()> {
    let addr = cfg.addr;
    lc.append(task_with(
        "http.serve",
        move || async move {
            let listener = tokio::net::TcpListener::bind(addr)
                .await
                .map_err(|e| Error::io(format!("bind {addr}"), e))?;
            println!("listening on http://{addr}");
            Ok(listener)
        },
        move |listener, stopped| async move {
            let app = Router::new()
                .route("/", get(index))
                .route("/hello/{name}", get(hello))
                .with_state(state);
            axum::serve(listener, app)
                .with_graceful_shutdown(stopped)
                .await
                .map_err(|e| Error::io(format!("serve {addr}"), e))
        },
    ))
}

fn http_domain() -> Module {
    Module::builder("http")
        .provide_private(new_greeter)
        .provide(new_state)
        .invoke(register_http)
}

#[tokio::main]
async fn main() -> modrun::Result<()> {
    modrun::logging::init();

    Modrun::builder()
        .supply(Config {
            addr: SocketAddr::from(([127, 0, 0, 1], 3000)),
        })
        .module(http_domain())
        .run()
        .await
}
