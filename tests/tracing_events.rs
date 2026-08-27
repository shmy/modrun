//! Framework tracing events mirror uber/fx's fxevent messages.

use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use modrun::{Lifecycle, Modrun, Module, Shutdowner, hook};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

async fn with_logs<F, Fut>(f: F) -> String
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = ()>,
{
    let capture = Capture::default();
    let writer = capture.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(true)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();

    let _guard = tracing::subscriber::set_default(subscriber);
    f().await;
    capture.text()
}

#[tokio::test]
async fn emits_fx_style_lifecycle_events() {
    #[derive(Clone)]
    struct Config;

    #[derive(Clone)]
    struct Service;

    fn new_service(_cfg: Config) -> Service {
        Service
    }

    fn boot(lc: Lifecycle, _svc: Service) {
        lc.append(
            hook()
                .on_start(|| async { Ok(()) })
                .on_stop(|| async { Ok(()) }),
        )
        .unwrap();
    }

    let logs = with_logs(|| async {
        Modrun::builder()
            .supply(Config)
            .module(Module::new("app").provide(new_service).invoke(boot))
            .start()
            .await
            .unwrap()
            .stop()
            .await
            .unwrap();
    })
    .await;

    assert!(has_message(&logs, "supplied"), "{logs}");
    assert!(has_message(&logs, "provided"), "{logs}");
    assert!(has_message(&logs, "invoking"), "{logs}");
    assert!(has_message(&logs, "before run"), "{logs}");
    assert!(
        logs.lines()
            .any(|l| l.contains("run") && !l.contains("before run") && !l.contains("cancelled")),
        "{logs}"
    );
    assert!(has_message(&logs, "OnStart hook executing"), "{logs}");
    assert!(has_message(&logs, "OnStart hook executed"), "{logs}");
    assert!(has_message(&logs, "started"), "{logs}");
    assert!(has_message(&logs, "OnStop hook executing"), "{logs}");
    assert!(has_message(&logs, "OnStop hook executed"), "{logs}");
    assert!(has_message(&logs, "stopped"), "{logs}");
    assert!(logs.contains("modrun"), "{logs}");
}

fn has_message(logs: &str, message: &str) -> bool {
    logs.lines().any(|line| line.contains(message))
}

#[tokio::test]
async fn emits_rollback_events_on_start_failure() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_start(|| async { Err(modrun::Error::hook("boom")) }))
            .unwrap();
    }

    let logs = with_logs(|| async {
        let err = Modrun::builder().invoke(boot).start().await.unwrap_err();
        assert!(format!("{err}").contains("boom"));
    })
    .await;

    assert!(has_message(&logs, "OnStart hook failed"), "{logs}");
    assert!(has_message(&logs, "start failed, rolling back"), "{logs}");
    assert!(has_message(&logs, "start failed"), "{logs}");
}

#[tokio::test]
async fn start_timeout_emits_cancelled() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().name("hang").on_start(|| async {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            Ok(())
        }))
        .unwrap();
    }

    let logs = with_logs(|| async {
        let _ = Modrun::builder()
            .start_timeout(std::time::Duration::from_millis(50))
            .invoke(boot)
            .start()
            .await
            .unwrap_err();
    })
    .await;

    assert!(has_message(&logs, "OnStart hook cancelled"), "{logs}");
    assert!(logs.contains("hang"), "{logs}");
}

#[tokio::test]
async fn invoking_logs_function_and_filters_framework_deps() {
    #[derive(Clone)]
    struct Config;

    fn register_http(_lc: Lifecycle, _cfg: Config, _shutdown: modrun::Shutdowner) {}

    let logs = with_logs(|| async {
        Modrun::builder()
            .supply(Config)
            .module(Module::new("http").invoke(register_http))
            .start()
            .await
            .unwrap()
            .stop()
            .await
            .unwrap();
    })
    .await;

    let invoking_line = logs
        .lines()
        .find(|l| l.contains("modrun:") && l.contains(" invoking"))
        .expect("invoking line");
    assert!(invoking_line.contains("register_http"), "{invoking_line}");
    assert!(invoking_line.contains("function="), "{invoking_line}");
    assert!(invoking_line.contains("deps="), "{invoking_line}");
    assert!(invoking_line.contains("Config"), "{invoking_line}");
    assert!(
        !invoking_line.contains("modrun::lifecycle::Lifecycle"),
        "{invoking_line}"
    );
    assert!(
        !invoking_line.contains("modrun::shutdown::Shutdowner"),
        "{invoking_line}"
    );
}

#[tokio::test]
async fn named_hook_appears_in_logs() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().name("http.serve").on_start(|| async { Ok(()) }))
            .unwrap();
    }

    let logs = with_logs(|| async {
        Modrun::builder()
            .invoke(boot)
            .start()
            .await
            .unwrap()
            .stop()
            .await
            .unwrap();
    })
    .await;

    assert!(logs.contains("http.serve"), "{logs}");
}

#[tokio::test]
async fn build_cancel_emits_shutdown_requested() {
    #[derive(Clone)]
    struct Pool;

    async fn connect(shutdown: Shutdowner) -> Pool {
        let s = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            s.shutdown();
        });
        tokio::time::sleep(Duration::from_secs(10)).await;
        Pool
    }

    let logs = with_logs(|| async {
        Modrun::builder()
            .provide_async(connect)
            .invoke(|_p: Pool| {})
            .run()
            .await
            .unwrap();
    })
    .await;

    assert!(has_message(&logs, "shutdown requested"), "{logs}");
}

#[tokio::test]
async fn stop_timeout_emits_hooks_abandoned() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_stop(|| async {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            Ok(())
        }))
        .unwrap();
        lc.append(hook().on_stop(|| async { Ok(()) })).unwrap();
    }

    let logs = with_logs(|| async {
        let _ = Modrun::builder()
            .stop_timeout(std::time::Duration::from_millis(50))
            .invoke(boot)
            .start()
            .await
            .unwrap()
            .stop()
            .await
            .unwrap_err();
    })
    .await;

    assert!(has_message(&logs, "OnStop hooks abandoned"), "{logs}");
}
