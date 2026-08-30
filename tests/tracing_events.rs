//! Framework tracing events mirror uber/fx's ConsoleLogger messages.

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
        .without_time()
        .with_target(false)
        .with_level(false)
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

    assert!(has_message(&logs, "SUPPLY"), "{logs}");
    assert!(has_message(&logs, "PROVIDE"), "{logs}");
    assert!(has_message(&logs, "INVOKE"), "{logs}");
    assert!(has_message(&logs, "BEFORE RUN"), "{logs}");
    assert!(has_message(&logs, "RUN"), "{logs}");
    assert!(has_message(&logs, "HOOK OnStart"), "{logs}");
    assert!(has_message(&logs, "ran successfully"), "{logs}");
    assert!(has_message(&logs, "RUNNING"), "{logs}");
    assert!(has_message(&logs, "HOOK OnStop"), "{logs}");
    assert!(has_message(&logs, "STOPPED"), "{logs}");
    assert!(logs.contains("[modrun]"), "{logs}");
}

#[tokio::test]
async fn sync_constructor_tracing_wraps_execution_and_reports_errors() {
    #[derive(Clone)]
    struct Service;

    fn fail_service() -> modrun::Result<Service> {
        Err(modrun::Error::hook("ctor-boom"))
    }

    let logs = with_logs(|| async {
        Modrun::builder()
            .provide_result(fail_service)
            .invoke(|_: Service| {})
            .start()
            .await
            .unwrap_err();
    })
    .await;

    let before = logs.find("BEFORE RUN").expect("before run event");
    let failed = logs.find("provide:").expect("constructor error event");
    assert!(before < failed, "{logs}");
    assert!(logs.contains("ctor-boom"), "{logs}");
    assert!(logs[failed..].contains("failed"), "{logs}");
}

#[tokio::test]
async fn build_timeout_emits_invoke_cancelled() {
    async fn hang() {
        tokio::time::sleep(Duration::from_secs(10)).await;
    }

    let logs = with_logs(|| async {
        Modrun::builder()
            .build_timeout(Duration::from_millis(20))
            .invoke_async(hang)
            .start()
            .await
            .unwrap_err();
    })
    .await;

    assert!(has_message(&logs, "invoke cancelled"), "{logs}");
}

#[tokio::test]
async fn build_timeout_emits_constructor_cancelled() {
    #[derive(Clone)]
    struct Pool;

    async fn hang() -> Pool {
        tokio::time::sleep(Duration::from_secs(10)).await;
        Pool
    }

    let logs = with_logs(|| async {
        Modrun::builder()
            .build_timeout(Duration::from_millis(20))
            .provide_async(hang)
            .invoke(|_: Pool| {})
            .start()
            .await
            .unwrap_err();
    })
    .await;

    let before = logs.find("BEFORE RUN").expect("before run event");
    let provide_cancel = logs
        .lines()
        .find(|line| line.contains("provide:") && line.contains(" cancelled"))
        .expect("constructor cancel event");
    let cancelled = logs.find(provide_cancel).expect("constructor cancel event");
    assert!(before < cancelled, "{logs}");
    assert!(has_message(&logs, "invoke cancelled"), "{logs}");
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

    assert!(has_message(&logs, "HOOK OnStart"), "{logs}");
    assert!(has_message(&logs, "rolling back"), "{logs}");
    assert!(has_message(&logs, "Failed to start"), "{logs}");
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

    assert!(has_message(&logs, "HOOK OnStart"), "{logs}");
    assert!(has_message(&logs, "cancelled"), "{logs}");
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
        .find(|l| l.contains("INVOKE") && l.contains("register_http"))
        .expect("invoking line");
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

    assert!(has_message(&logs, "SHUTDOWN REQUESTED"), "{logs}");
}

#[tokio::test]
async fn steady_shutdown_emits_one_requested_event_without_signal() {
    fn boot(lc: Lifecycle, shutdown: Shutdowner) {
        lc.append(hook().on_start(move || async move {
            shutdown.shutdown();
            Ok(())
        }))
        .unwrap();
    }

    let logs = with_logs(|| async {
        Modrun::builder().invoke(boot).run().await.unwrap();
    })
    .await;

    let requested = logs
        .lines()
        .filter(|line| line.contains("SHUTDOWN REQUESTED"))
        .count();
    assert_eq!(requested, 1, "{logs}");
    assert!(!logs.lines().any(|line| line.contains("SIG")), "{logs}");
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

#[tokio::test]
async fn constructor_panic_emits_panicked_not_cancelled() {
    #[derive(Clone)]
    struct Pool;

    fn boom() -> Pool {
        panic!("ctor-panic");
    }

    let logs = with_logs(|| async {
        let handle = tokio::spawn(async {
            let _ = Modrun::builder()
                .provide(boom)
                .invoke(|_: Pool| {})
                .start()
                .await;
        });
        assert!(handle.await.unwrap_err().is_panic());
    })
    .await;

    assert!(has_message(&logs, " panicked"), "{logs}");
    assert!(
        !logs.lines().any(|line| line.contains(" cancelled")),
        "{logs}"
    );
}

#[tokio::test]
async fn async_constructor_panic_emits_panicked_not_cancelled() {
    #[derive(Clone)]
    struct Pool;

    async fn boom() -> Pool {
        panic!("ctor-panic");
    }

    let logs = with_logs(|| async {
        let handle = tokio::spawn(async {
            let _ = Modrun::builder()
                .provide_async(boom)
                .invoke(|_: Pool| {})
                .start()
                .await;
        });
        assert!(handle.await.unwrap_err().is_panic());
    })
    .await;

    assert!(has_message(&logs, " panicked"), "{logs}");
    assert!(
        !logs.lines().any(|line| line.contains(" cancelled")),
        "{logs}"
    );
}

#[tokio::test]
async fn start_panic_emits_panicked_not_cancelled() {
    fn boot(lc: Lifecycle) {
        lc.append(
            hook()
                .name("boom")
                .on_start(|| async { panic!("hook-panic") }),
        )
        .unwrap();
    }

    let logs = with_logs(|| async {
        let handle = tokio::spawn(async {
            let _ = Modrun::builder().invoke(boot).start().await;
        });
        assert!(handle.await.unwrap_err().is_panic());
    })
    .await;

    assert!(has_message(&logs, "HOOK OnStart"), "{logs}");
    assert!(logs.contains("boom"), "{logs}");
    assert!(has_message(&logs, " panicked"), "{logs}");
    assert!(
        !logs.lines().any(|line| line.contains(" cancelled")),
        "{logs}"
    );
}

#[tokio::test]
async fn stop_panic_emits_panicked_not_cancelled() {
    fn boot(lc: Lifecycle) {
        lc.append(
            hook()
                .name("boom")
                .on_stop(|| async { panic!("hook-panic") }),
        )
        .unwrap();
    }

    let logs = with_logs(|| async {
        let handle = tokio::spawn(async {
            let app = Modrun::builder().invoke(boot).start().await.unwrap();
            let _ = app.stop().await;
        });
        assert!(handle.await.unwrap_err().is_panic());
    })
    .await;

    assert!(has_message(&logs, "HOOK OnStop"), "{logs}");
    assert!(logs.contains("boom"), "{logs}");
    assert!(has_message(&logs, " panicked"), "{logs}");
    assert!(
        !logs.lines().any(|line| line.contains(" cancelled")),
        "{logs}"
    );
}

#[tokio::test]
async fn dot_graph_emits_trace_event() {
    let dir = std::env::temp_dir().join(format!("modrun-dot-trace-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("graph.dot");

    let logs = with_logs(|| async {
        Modrun::builder()
            .no_banner()
            .dot_graph(&path)
            .provide(|| 1u32)
            .invoke(|_: u32| {})
            .start()
            .await
            .unwrap()
            .stop()
            .await
            .unwrap();
    })
    .await;

    assert!(
        logs.contains("GRAPH") && logs.contains("graph.dot"),
        "logs: {logs}"
    );
    let _ = std::fs::remove_dir_all(dir);
}
