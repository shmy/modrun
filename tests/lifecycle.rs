//! Lifecycle, timeouts, and shutdown tests.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use modrun::{Error, Hook, Lifecycle, Modrun, Module, Shutdowner, hook};

/// Build a repeatable OnStop closure for [`hook`] (required after StopGuard retry).
macro_rules! on_stop_shared {
    ($shared:expr, |$var:ident| $body:block) => {{
        let __shared = Arc::clone(&$shared);
        move || {
            let $var = Arc::clone(&__shared);
            async move $body
        }
    }};
}

#[tokio::test]
async fn shared_counter_lifecycle() {
    #[derive(Clone)]
    struct Config;

    #[derive(Clone)]
    struct Counter(Arc<AtomicUsize>);

    fn boot(lc: Lifecycle, _cfg: Config, counter: Counter) {
        let c1 = Arc::clone(&counter.0);
        let c2 = Arc::clone(&counter.0);
        lc.append(
            hook()
                .on_start(move || {
                    let c = Arc::clone(&c1);
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                })
                .on_stop(move || {
                    let c = Arc::clone(&c2);
                    async move {
                        c.fetch_add(1, Ordering::SeqCst);
                        Ok(())
                    }
                }),
        )
        .unwrap();
    }

    let shared = Arc::new(AtomicUsize::new(0));
    let app = Modrun::builder()
        .supply(Config)
        .supply(Counter(Arc::clone(&shared)))
        .module(Module::new("app").invoke(boot))
        .start()
        .await
        .unwrap();
    assert_eq!(shared.load(Ordering::SeqCst), 1);
    app.stop().await.unwrap();
    assert_eq!(shared.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn start_failure_runs_stop_for_started_hooks() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    fn boot(lc: Lifecycle, log: Log) {
        let l1 = Arc::clone(&log.0);
        let l2 = Arc::clone(&log.0);
        lc.append(
            hook()
                .on_start(move || {
                    let l = Arc::clone(&l1);
                    async move {
                        l.lock().unwrap().push("start-ok");
                        Ok(())
                    }
                })
                .on_stop(move || {
                    let l = Arc::clone(&l2);
                    async move {
                        l.lock().unwrap().push("stop-ok");
                        Ok(())
                    }
                }),
        )
        .unwrap();

        lc.append(hook().on_start(|| async { Err(Error::hook("boom")) }))
            .unwrap();
    }

    let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
    let err = Modrun::builder()
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("boom"), "unexpected: {msg}");
    assert_eq!(shared.lock().unwrap().as_slice(), ["start-ok", "stop-ok"]);
}

#[tokio::test]
async fn hook_order_start_fifo_stop_lifo() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    fn boot(lc: Lifecycle, log: Log) {
        for (start, stop) in [("s1", "t1"), ("s2", "t2"), ("s3", "t3")] {
            let l1 = Arc::clone(&log.0);
            let l2 = Arc::clone(&log.0);
            lc.append(
                hook()
                    .on_start(move || {
                        let l = Arc::clone(&l1);
                        async move {
                            l.lock().unwrap().push(start);
                            Ok(())
                        }
                    })
                    .on_stop(move || {
                        let l = Arc::clone(&l2);
                        async move {
                            l.lock().unwrap().push(stop);
                            Ok(())
                        }
                    }),
            )
            .unwrap();
        }
    }

    let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
    let app = Modrun::builder()
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .start()
        .await
        .unwrap();
    assert_eq!(shared.lock().unwrap().as_slice(), ["s1", "s2", "s3"]);
    app.stop().await.unwrap();
    assert_eq!(
        shared.lock().unwrap().as_slice(),
        ["s1", "s2", "s3", "t3", "t2", "t1"]
    );
}

#[tokio::test]
async fn shutdowner_stops_run() {
    fn boot(lc: Lifecycle, shutdown: Shutdowner) {
        lc.append(hook().on_start(move || {
            let shutdown = shutdown.clone();
            async move {
                shutdown.shutdown();
                Ok(())
            }
        }))
        .unwrap();
    }

    Modrun::builder().invoke(boot).run().await.unwrap();
}

#[tokio::test]
async fn start_timeout_errors() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_start(|| async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        }))
        .unwrap();
    }

    let err = Modrun::builder()
        .start_timeout(Duration::from_millis(50))
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("timed out"), "unexpected: {msg}");
}

#[tokio::test]
async fn run_stops_hooks_after_programmatic_shutdown() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    fn boot(lc: Lifecycle, log: Log, shutdown: Shutdowner) {
        let started = Arc::clone(&log.0);
        let stopped = Arc::clone(&log.0);
        lc.append(
            hook()
                .on_start(move || async move {
                    started.lock().unwrap().push("start");
                    shutdown.shutdown();
                    Ok(())
                })
                .on_stop(on_stop_shared!(stopped, |stopped| {
                    stopped.lock().unwrap().push("stop");
                    Ok(())
                })),
        )
        .unwrap();
    }

    let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
    Modrun::builder()
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .run()
        .await
        .unwrap();
    assert_eq!(shared.lock().unwrap().as_slice(), ["start", "stop"]);
}

/// A failed start already unwinds the hooks it started, so an explicit stop
/// afterwards must not run them a second time.
#[tokio::test]
async fn stop_hooks_run_at_most_once() {
    #[derive(Clone)]
    struct Log(Arc<AtomicUsize>);

    fn boot(lc: Lifecycle, log: Log) {
        let counter = Arc::clone(&log.0);
        lc.append(hook().on_stop(on_stop_shared!(counter, |counter| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })))
        .unwrap();
        lc.append(hook().on_start(|| async { Err(Error::hook("boom")) }))
            .unwrap();
    }

    let shared = Arc::new(AtomicUsize::new(0));
    Modrun::builder()
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    assert_eq!(shared.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn start_timeout_unwinds_already_started_hooks() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    fn boot(lc: Lifecycle, log: Log) {
        let l1 = Arc::clone(&log.0);
        let l2 = Arc::clone(&log.0);
        lc.append(
            hook()
                .on_start(move || async move {
                    l1.lock().unwrap().push("start-ok");
                    Ok(())
                })
                .on_stop(on_stop_shared!(l2, |l2| {
                    l2.lock().unwrap().push("stop-ok");
                    Ok(())
                })),
        )
        .unwrap();
        lc.append(hook().on_start(|| async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        }))
        .unwrap();
    }

    let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
    let err = Modrun::builder()
        .start_timeout(Duration::from_millis(50))
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("timed out"), "unexpected: {msg}");
    assert_eq!(shared.lock().unwrap().as_slice(), ["start-ok", "stop-ok"]);
}

#[tokio::test]
async fn append_from_start_factory_does_not_deadlock() {
    fn boot(lc: Lifecycle) {
        let lc2 = lc.clone();
        lc.append(hook().on_start(move || {
            lc2.append(hook().on_stop(|| async { Ok(()) })).unwrap();
            async { Ok(()) }
        }))
        .unwrap();
    }

    Modrun::builder()
        .start_timeout(Duration::from_secs(2))
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn hook_without_on_stop_does_not_truncate_stop_chain() {
    fn boot(lc: Lifecycle, log: Arc<Mutex<Vec<&'static str>>>) {
        let first = Arc::clone(&log);
        lc.append(hook().on_stop(on_stop_shared!(first, |first| {
            first.lock().unwrap().push("first");
            Ok(())
        })))
        .unwrap();

        lc.append(hook().on_start(|| async { Ok(()) })).unwrap();

        lc.append(hook().on_stop(on_stop_shared!(log, |log| {
            log.lock().unwrap().push("last");
            Ok(())
        })))
        .unwrap();
    }

    let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    Modrun::builder()
        .supply(Arc::clone(&log))
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();

    assert_eq!(log.lock().unwrap().as_slice(), ["last", "first"]);
}

#[tokio::test]
async fn hook_without_on_stop_does_not_truncate_unwind() {
    fn boot(lc: Lifecycle, log: Arc<Mutex<Vec<&'static str>>>) {
        lc.append(
            hook()
                .on_start(|| async { Ok(()) })
                .on_stop(on_stop_shared!(log, |log| {
                    log.lock().unwrap().push("first");
                    Ok(())
                })),
        )
        .unwrap();

        lc.append(hook().on_start(|| async { Ok(()) })).unwrap();
        lc.append(hook().on_start(|| async { Err(Error::hook("start failed")) }))
            .unwrap();
    }

    let log: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let err = Modrun::builder()
        .supply(Arc::clone(&log))
        .invoke(boot)
        .start()
        .await
        .unwrap_err();

    assert!(err.to_string().contains("start failed"));
    assert_eq!(log.lock().unwrap().as_slice(), ["first"]);
}

#[tokio::test]
async fn stop_timeout_errors() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_stop(|| async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        }))
        .unwrap();
    }

    let err = Modrun::builder()
        .stop_timeout(Duration::from_millis(50))
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("timed out"), "unexpected: {msg}");
}

#[tokio::test]
async fn start_timeout_unwind_respects_stop_timeout() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    fn boot(lc: Lifecycle, log: Log) {
        let started = Arc::clone(&log.0);
        lc.append(
            hook()
                .on_start(move || async move {
                    started.lock().unwrap().push("start-ok");
                    Ok(())
                })
                .on_stop(|| async {
                    tokio::time::sleep(Duration::from_secs(10)).await;
                    Ok(())
                }),
        )
        .unwrap();
        lc.append(hook().on_start(|| async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        }))
        .unwrap();
    }

    let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
    let began = std::time::Instant::now();
    let err = Modrun::builder()
        .start_timeout(Duration::from_millis(50))
        .stop_timeout(Duration::from_millis(50))
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    let elapsed = began.elapsed();
    let msg = format!("{err}");
    assert!(
        msg.contains("timed out") && msg.contains("unwinding"),
        "unexpected: {msg}"
    );
    assert!(
        elapsed < Duration::from_secs(2),
        "unwind ignored stop_timeout: {elapsed:?}"
    );
    assert_eq!(shared.lock().unwrap().as_slice(), ["start-ok"]);
}

#[tokio::test]
async fn shutdown_during_start_unwinds_gracefully() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    fn boot(lc: Lifecycle, log: Log, shutdown: Shutdowner) {
        let s = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            s.shutdown();
        });

        let started = Arc::clone(&log.0);
        let stopped = Arc::clone(&log.0);
        lc.append(
            hook()
                .on_start(move || async move {
                    started.lock().unwrap().push("start-ok");
                    Ok(())
                })
                .on_stop(on_stop_shared!(stopped, |stopped| {
                    stopped.lock().unwrap().push("stop-ok");
                    Ok(())
                })),
        )
        .unwrap();
        lc.append(hook().on_start(|| async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        }))
        .unwrap();
    }

    let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
    Modrun::builder()
        .start_timeout(Duration::from_secs(5))
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .run()
        .await
        .unwrap();
    assert_eq!(shared.lock().unwrap().as_slice(), ["start-ok", "stop-ok"]);
}

#[tokio::test]
async fn shutdown_during_start_runs_stop_only_after_unstarted_hook() {
    #[derive(Clone)]
    struct Log(Arc<Mutex<Vec<&'static str>>>);

    fn boot(lc: Lifecycle, log: Log, shutdown: Shutdowner) {
        let s = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            s.shutdown();
        });

        lc.append(hook().on_start(|| async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        }))
        .unwrap();
        lc.append(hook().on_start(|| async {
            panic!("later start hook must not run after shutdown");
        }))
        .unwrap();

        let stopped = Arc::clone(&log.0);
        lc.append(hook().on_stop(on_stop_shared!(stopped, |stopped| {
            stopped.lock().unwrap().push("stop-only");
            Ok(())
        })))
        .unwrap();
    }

    let log = Log(Arc::new(Mutex::new(Vec::new())));
    Modrun::builder()
        .supply(log.clone())
        .invoke(boot)
        .run()
        .await
        .unwrap();
    assert_eq!(log.0.lock().unwrap().as_slice(), ["stop-only"]);
}

#[tokio::test]
async fn shutdown_during_build_is_ok() {
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

    Modrun::builder()
        .provide_async(connect)
        .invoke(|_p: Pool| {})
        .run()
        .await
        .unwrap();
}

#[tokio::test]
async fn shutdown_during_build_runs_stop_only_hooks() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    #[derive(Clone)]
    struct Pool;

    fn boot(lc: Lifecycle, shutdown: Shutdowner, log: Log) {
        let stopped = Arc::clone(&log.0);
        lc.append(hook().on_stop(on_stop_shared!(stopped, |stopped| {
            stopped.lock().unwrap().push("stop-ok");
            Ok(())
        })))
        .unwrap();

        let s = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            s.shutdown();
        });
    }

    async fn connect(_shutdown: Shutdowner) -> Pool {
        tokio::time::sleep(Duration::from_secs(10)).await;
        Pool
    }

    let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
    Modrun::builder()
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .provide_async(connect)
        .invoke(|_p: Pool| {})
        .run()
        .await
        .unwrap();
    assert_eq!(shared.lock().unwrap().as_slice(), ["stop-ok"]);
}

#[tokio::test]
async fn shutdown_during_build_runs_stop_only_after_unstarted_start_hook() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    #[derive(Clone)]
    struct Pool;

    fn boot(lc: Lifecycle, shutdown: Shutdowner, log: Log) {
        let started = Arc::clone(&log.0);
        let started_stop = Arc::clone(&log.0);
        lc.append(
            hook()
                .on_start(move || async move {
                    started.lock().unwrap().push("start");
                    Ok(())
                })
                .on_stop(on_stop_shared!(started_stop, |started_stop| {
                    started_stop.lock().unwrap().push("stop-started");
                    Ok(())
                })),
        )
        .unwrap();
        let only = Arc::clone(&log.0);
        lc.append(hook().on_stop(on_stop_shared!(only, |only| {
            only.lock().unwrap().push("stop-only");
            Ok(())
        })))
        .unwrap();

        let s = shutdown.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            s.shutdown();
        });
    }

    async fn connect(_shutdown: Shutdowner) -> Pool {
        tokio::time::sleep(Duration::from_secs(10)).await;
        Pool
    }

    let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
    Modrun::builder()
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .provide_async(connect)
        .invoke(|_p: Pool| {})
        .run()
        .await
        .unwrap();
    assert_eq!(shared.lock().unwrap().as_slice(), ["stop-only"]);
}

#[tokio::test]
async fn run_prefers_build_error_over_concurrent_shutdown() {
    #[derive(Clone)]
    struct Pool;

    async fn fail_build(shutdown: Shutdowner) -> Result<Pool, Error> {
        shutdown.shutdown();
        Err(Error::hook("build-boom"))
    }

    for _ in 0..32 {
        let err = Modrun::builder()
            .provide_result_async(fail_build)
            .invoke(|_p: Pool| {})
            .run()
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("build-boom"),
            "expected build error, not graceful shutdown: {msg}"
        );
    }
}

#[tokio::test]
async fn run_prefers_start_error_over_concurrent_shutdown() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    fn boot(lc: Lifecycle, shutdown: Shutdowner, log: Log) {
        let started = Arc::clone(&log.0);
        let stopped = Arc::clone(&log.0);
        lc.append(
            hook()
                .on_start(move || async move {
                    started.lock().unwrap().push("start-ok");
                    Ok(())
                })
                .on_stop(on_stop_shared!(stopped, |stopped| {
                    stopped.lock().unwrap().push("stop-ok");
                    Ok(())
                })),
        )
        .unwrap();
        lc.append(hook().on_start(move || async move {
            shutdown.shutdown();
            Err(Error::hook("start-boom"))
        }))
        .unwrap();
    }

    for _ in 0..32 {
        let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
        let err = Modrun::builder()
            .supply(Log(Arc::clone(&shared)))
            .invoke(boot)
            .run()
            .await
            .unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("start-boom"),
            "expected start error, not graceful shutdown: {msg}"
        );
        assert_eq!(shared.lock().unwrap().as_slice(), ["start-ok", "stop-ok"]);
    }
}

#[tokio::test]
async fn failed_start_stays_err_if_shutdown_during_unwind() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    fn boot(lc: Lifecycle, shutdown: Shutdowner, log: Log) {
        let stopped = Arc::clone(&log.0);
        let s = shutdown.clone();

        lc.append(hook().on_start(|| async { Ok(()) }).on_stop({
            let stopped = Arc::clone(&stopped);
            move || {
                let stopped = Arc::clone(&stopped);
                let s = s.clone();
                async move {
                    stopped.lock().unwrap().push("stop-ok");
                    s.shutdown();
                    Ok(())
                }
            }
        }))
        .unwrap();
        lc.append(hook().on_start(|| async { Err(Error::hook("boom")) }))
            .unwrap();
    }

    let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
    let err = Modrun::builder()
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .run()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("boom"),
        "start failure must not become Ok after shutdown during unwind: {msg}"
    );
    assert_eq!(shared.lock().unwrap().as_slice(), ["stop-ok"]);
}

#[tokio::test]
async fn build_timeout_after_invoke_unwinds_stop_only() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    #[derive(Clone)]
    struct Slow;

    fn boot(lc: Lifecycle, log: Log) {
        let stopped = Arc::clone(&log.0);
        lc.append(hook().on_stop(on_stop_shared!(stopped, |stopped| {
            stopped.lock().unwrap().push("stop-ok");
            Ok(())
        })))
        .unwrap();
    }

    async fn slow() -> Slow {
        tokio::time::sleep(Duration::from_secs(10)).await;
        Slow
    }

    let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
    let err = Modrun::builder()
        .build_timeout(Duration::from_millis(50))
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .provide_async(slow)
        .invoke(|_: Slow| {})
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("build timed out"), "unexpected: {msg}");
    assert_eq!(shared.lock().unwrap().as_slice(), ["stop-ok"]);
}

#[tokio::test]
async fn stop_timeout_abandons_inflight_and_remaining() {
    #[derive(Clone)]
    struct Log(Arc<std::sync::Mutex<Vec<&'static str>>>);

    fn boot(lc: Lifecycle, log: Log) {
        let l2 = Arc::clone(&log.0);
        let l1 = Arc::clone(&log.0);
        lc.append(hook().on_stop(on_stop_shared!(l2, |l2| {
            l2.lock().unwrap().push("never");
            Ok(())
        })))
        .unwrap();
        lc.append(hook().on_stop(on_stop_shared!(l1, |l1| {
            l1.lock().unwrap().push("hang");
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        })))
        .unwrap();
    }

    let shared = Arc::new(std::sync::Mutex::new(Vec::new()));
    let err = Modrun::builder()
        .stop_timeout(Duration::from_millis(50))
        .supply(Log(Arc::clone(&shared)))
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("timed out"), "unexpected: {msg}");
    assert_eq!(
        shared.lock().unwrap().as_slice(),
        ["hang"],
        "remaining OnStop after a timed-out hook must not run"
    );
}

#[tokio::test]
async fn combine_prefers_unwind_timeout_over_start_timeout() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_stop(|| async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        }))
        .unwrap();
        lc.append(hook().on_start(|| async {
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        }))
        .unwrap();
    }

    // First hook has no OnStart so it counts as started immediately; its OnStop
    // hangs. Second OnStart hangs until start_timeout. Unwind then hits stop_timeout.
    let err = Modrun::builder()
        .start_timeout(Duration::from_millis(50))
        .stop_timeout(Duration::from_millis(50))
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("unwinding"),
        "expected unwind timeout to win, got: {msg}"
    );
}

#[tokio::test]
async fn multiple_stop_errors_are_aggregated() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_stop(|| async { Err(Error::hook("stop-a")) }))
            .unwrap();
        lc.append(hook().on_stop(|| async { Err(Error::hook("stop-b")) }))
            .unwrap();
    }

    let err = Modrun::builder()
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("2 OnStop hooks failed") && msg.contains("stop-a") && msg.contains("stop-b"),
        "unexpected: {msg}"
    );
}

#[tokio::test]
async fn start_and_unwind_errors_are_both_retained() {
    fn boot(lc: Lifecycle) {
        lc.append(
            hook()
                .on_start(|| async { Ok(()) })
                .on_stop(|| async { Err(Error::hook("cleanup-boom")) }),
        )
        .unwrap();
        lc.append(hook().on_start(|| async { Err(Error::hook("start-boom")) }))
            .unwrap();
    }

    let err = Modrun::builder().invoke(boot).start().await.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("cleanup failed after an earlier phase error")
            && msg.contains("cleanup-boom")
            && msg.contains("start-boom"),
        "unexpected: {msg}"
    );
}

#[tokio::test]
async fn append_after_start_errors() {
    let lc_holder = std::sync::Arc::new(std::sync::Mutex::new(None::<Lifecycle>));
    let slot = std::sync::Arc::clone(&lc_holder);

    fn boot(lc: Lifecycle, slot: std::sync::Arc<std::sync::Mutex<Option<Lifecycle>>>) {
        *slot.lock().unwrap() = Some(lc);
    }

    // Can't easily pass Arc through provide without supply...
    let app = Modrun::builder()
        .supply(slot)
        .invoke(boot)
        .start()
        .await
        .unwrap();

    let lc = lc_holder.lock().unwrap().clone().unwrap();
    let err = lc.append(hook().on_stop(|| async { Ok(()) })).unwrap_err();
    assert!(
        format!("{err}").contains("after start has finished"),
        "unexpected: {err}"
    );
    app.stop().await.unwrap();
}

#[tokio::test]
async fn append_while_stopping_errors() {
    #[derive(Clone)]
    struct StopGate {
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Notify>,
    }

    let lc_holder = Arc::new(Mutex::new(None::<Lifecycle>));
    let slot = Arc::clone(&lc_holder);
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());

    fn boot(lc: Lifecycle, slot: Arc<Mutex<Option<Lifecycle>>>, gate: StopGate) {
        *slot.lock().unwrap() = Some(lc.clone());
        lc.append(hook().on_stop({
            let entered = Arc::clone(&gate.entered);
            let release = Arc::clone(&gate.release);
            move || {
                let entered = Arc::clone(&entered);
                let release = Arc::clone(&release);
                async move {
                    entered.notify_one();
                    release.notified().await;
                    Ok(())
                }
            }
        }))
        .unwrap();
    }

    let app = Modrun::builder()
        .supply(slot)
        .supply(StopGate {
            entered: Arc::clone(&entered),
            release: Arc::clone(&release),
        })
        .invoke(boot)
        .start()
        .await
        .unwrap();

    let stopping = tokio::spawn(app.stop());
    entered.notified().await;

    let lc = lc_holder.lock().unwrap().clone().unwrap();
    assert!(matches!(lc.append(hook()), Err(Error::AppendWhileStopping)));

    release.notify_one();
    stopping.await.unwrap().unwrap();
}

#[tokio::test]
async fn last_timeout_wins() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_start(|| async {
            tokio::time::sleep(Duration::from_millis(80)).await;
            Ok(())
        }))
        .unwrap();
    }

    // First timeout would fire; last one is long enough.
    Modrun::builder()
        .start_timeout(Duration::from_millis(10))
        .start_timeout(Duration::from_millis(500))
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn shutdowner_wait_and_is_requested() {
    fn boot(lc: Lifecycle, shutdown: Shutdowner) {
        lc.append(hook().on_start(move || async move {
            assert!(!shutdown.is_requested());
            shutdown.shutdown();
            shutdown.wait().await;
            assert!(shutdown.is_requested());
            Ok(())
        }))
        .unwrap();
    }

    Modrun::builder()
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn no_start_timeout_disables_budget() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_start(|| async {
            tokio::time::sleep(Duration::from_millis(80)).await;
            Ok(())
        }))
        .unwrap();
    }

    Modrun::builder()
        .start_timeout(Duration::from_millis(10))
        .no_start_timeout()
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn no_stop_timeout_disables_budget() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_stop(|| async {
            tokio::time::sleep(Duration::from_millis(80)).await;
            Ok(())
        }))
        .unwrap();
    }

    Modrun::builder()
        .stop_timeout(Duration::from_millis(10))
        .no_stop_timeout()
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[cfg(not(feature = "signal"))]
#[tokio::test]
async fn run_without_signal_feature_waits_for_shutdowner() {
    fn boot(lc: Lifecycle, shutdown: Shutdowner) {
        lc.append(hook().on_start(move || async move {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(10)).await;
                shutdown.shutdown();
            });
            Ok(())
        }))
        .unwrap();
    }

    Modrun::builder().invoke(boot).run().await.unwrap();
}

#[tokio::test]
async fn run_shutdown_during_start_is_ok() {
    fn boot(lc: Lifecycle, shutdown: Shutdowner) {
        lc.append(hook().on_start(move || async move {
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(20)).await;
                shutdown.shutdown();
            });
            tokio::time::sleep(Duration::from_secs(10)).await;
            Ok(())
        }))
        .unwrap();
    }

    Modrun::builder()
        .start_timeout(Duration::from_secs(2))
        .invoke(boot)
        .run()
        .await
        .unwrap();
}

#[tokio::test]
async fn build_timeout_errors() {
    #[derive(Clone)]
    struct Slow;

    async fn slow() -> Slow {
        tokio::time::sleep(Duration::from_secs(10)).await;
        Slow
    }

    let err = Modrun::builder()
        .build_timeout(Duration::from_millis(50))
        .provide_async(slow)
        .invoke(|_: Slow| {})
        .start()
        .await
        .unwrap_err();
    assert!(
        format!("{err}").contains("build timed out"),
        "unexpected: {err}"
    );
}

/// Sync invokers block the worker; `tokio::time::timeout` cannot preempt them,
/// but an over-budget success must still surface as BuildTimeout.
#[tokio::test]
async fn build_timeout_reports_sync_blocking_invoker() {
    let err = Modrun::builder()
        .no_banner()
        .build_timeout(Duration::from_millis(50))
        .invoke(|| std::thread::sleep(Duration::from_millis(200)))
        .start()
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::BuildTimeout(_)),
        "expected BuildTimeout, got: {err}"
    );
}

#[tokio::test]
async fn start_timeout_reports_sync_blocking_hook() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_start(|| async {
            std::thread::sleep(Duration::from_millis(200));
            Ok(())
        }))
        .unwrap();
    }

    let err = Modrun::builder()
        .no_banner()
        .start_timeout(Duration::from_millis(50))
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::StartTimeout(_)),
        "expected StartTimeout, got: {err}"
    );
}

#[tokio::test]
async fn stop_timeout_reports_sync_blocking_hook() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_stop(|| async {
            std::thread::sleep(Duration::from_millis(200));
            Ok(())
        }))
        .unwrap();
    }

    let err = Modrun::builder()
        .no_banner()
        .stop_timeout(Duration::from_millis(50))
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap_err();
    assert!(
        matches!(err, Error::StopTimeout(_)),
        "expected StopTimeout, got: {err}"
    );
}

#[tokio::test]
async fn no_build_timeout_disables_budget() {
    #[derive(Clone)]
    struct Slow;

    async fn slow() -> Slow {
        tokio::time::sleep(Duration::from_millis(80)).await;
        Slow
    }

    Modrun::builder()
        .build_timeout(Duration::from_millis(10))
        .no_build_timeout()
        .provide_async(slow)
        .invoke(|_: Slow| {})
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn append_after_failed_start_is_stopping() {
    let lc_holder = Arc::new(Mutex::new(None::<Lifecycle>));
    let slot = Arc::clone(&lc_holder);

    fn boot(lc: Lifecycle, slot: Arc<Mutex<Option<Lifecycle>>>) {
        *slot.lock().unwrap() = Some(lc.clone());
        lc.append(hook().on_start(|| async { Err(Error::hook("boom")) }))
            .unwrap();
    }

    let err = Modrun::builder()
        .supply(slot)
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("boom"), "unexpected: {err}");
    assert!(
        format!("{err}").contains("hook failed"),
        "unexpected: {err}"
    );

    let lc = lc_holder.lock().unwrap().clone().unwrap();
    assert!(matches!(lc.append(hook()), Err(Error::AppendWhileStopping)));
}

#[tokio::test]
async fn struct_hook_shares_start_stop_state() {
    #[derive(Clone)]
    struct Log(Arc<Mutex<Vec<u32>>>);

    struct Counter {
        log: Log,
        n: u32,
    }

    impl Hook for Counter {
        fn name(&self) -> Option<&'static str> {
            Some("counter")
        }

        async fn on_start(&mut self) -> modrun::Result<()> {
            self.n += 1;
            self.log.0.lock().unwrap().push(self.n);
            Ok(())
        }

        async fn on_stop(&mut self) -> modrun::Result<()> {
            self.n += 1;
            self.log.0.lock().unwrap().push(self.n);
            Ok(())
        }
    }

    fn boot(lc: Lifecycle, log: Log) {
        lc.append(Counter { log, n: 0 }).unwrap();
    }

    let log = Log(Arc::new(Mutex::new(Vec::new())));
    let app = Modrun::builder()
        .supply(log.clone())
        .invoke(boot)
        .start()
        .await
        .unwrap();
    assert_eq!(log.0.lock().unwrap().as_slice(), [1]);
    app.stop().await.unwrap();
    assert_eq!(log.0.lock().unwrap().as_slice(), [1, 2]);
}

#[tokio::test]
async fn failed_struct_start_does_not_run_stop() {
    #[derive(Clone)]
    struct Log(Arc<Mutex<Vec<&'static str>>>);

    struct Boom {
        log: Log,
    }

    impl Hook for Boom {
        async fn on_start(&mut self) -> modrun::Result<()> {
            self.log.0.lock().unwrap().push("start");
            Err(Error::hook("boom"))
        }

        async fn on_stop(&mut self) -> modrun::Result<()> {
            self.log.0.lock().unwrap().push("stop");
            Ok(())
        }
    }

    fn boot(lc: Lifecycle, log: Log) {
        lc.append(Boom { log }).unwrap();
    }

    let log = Log(Arc::new(Mutex::new(Vec::new())));
    let err = Modrun::builder()
        .supply(log.clone())
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("boom"), "unexpected: {err}");
    assert_eq!(log.0.lock().unwrap().as_slice(), ["start"]);
}

#[tokio::test]
async fn struct_stop_only_after_failed_start_still_runs() {
    #[derive(Clone)]
    struct Log(Arc<Mutex<Vec<&'static str>>>);

    struct StopOnly {
        log: Log,
    }

    impl Hook for StopOnly {
        fn has_start(&self) -> bool {
            false
        }

        async fn on_stop(&mut self) -> modrun::Result<()> {
            self.log.0.lock().unwrap().push("stop-ok");
            Ok(())
        }
    }

    fn boot(lc: Lifecycle, log: Log) {
        lc.append(hook().on_start(|| async { Err(Error::hook("boom")) }))
            .unwrap();
        lc.append(StopOnly { log }).unwrap();
    }

    let log = Log(Arc::new(Mutex::new(Vec::new())));
    let err = Modrun::builder()
        .supply(log.clone())
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("boom"), "unexpected: {err}");
    assert_eq!(log.0.lock().unwrap().as_slice(), ["stop-ok"]);
}

#[tokio::test]
async fn struct_stop_only_without_has_start_is_skipped_on_unwind() {
    #[derive(Clone)]
    struct Log(Arc<Mutex<Vec<&'static str>>>);

    struct StopOnly {
        log: Log,
    }

    impl Hook for StopOnly {
        async fn on_stop(&mut self) -> modrun::Result<()> {
            self.log.0.lock().unwrap().push("stop-ok");
            Ok(())
        }
    }

    fn boot(lc: Lifecycle, log: Log) {
        lc.append(hook().on_start(|| async { Err(Error::hook("boom")) }))
            .unwrap();
        lc.append(StopOnly { log }).unwrap();
    }

    let log = Log(Arc::new(Mutex::new(Vec::new())));
    let err = Modrun::builder()
        .supply(log.clone())
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    assert!(format!("{err}").contains("boom"), "unexpected: {err}");
    assert!(log.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn on_stop_join_panic_surfaces_as_error() {
    fn boot(lc: Lifecycle) {
        lc.append(hook().on_stop(|| async {
            let task = tokio::spawn(async {
                panic!("join-panic");
            });
            match task.await {
                Ok(()) => Ok(()),
                Err(join) => Err(Error::hook(join)),
            }
        }))
        .unwrap();
    }

    let err = Modrun::builder()
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("hook failed"), "unexpected: {msg}");
    assert!(
        msg.contains("panic") || msg.contains("join"),
        "unexpected: {msg}"
    );
}
