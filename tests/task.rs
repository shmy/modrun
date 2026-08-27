//! Background [`modrun::task`] lifecycle tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use modrun::{Error, Lifecycle, Modrun, task, task_with};

#[tokio::test]
async fn task_runs_until_stop() {
    let hits = Arc::new(AtomicUsize::new(0));
    let n = Arc::clone(&hits);

    fn boot(lc: Lifecycle, hits: Arc<AtomicUsize>) -> modrun::Result<()> {
        lc.append(task("worker", move |stopped| async move {
            hits.fetch_add(1, Ordering::SeqCst);
            stopped.await;
            hits.fetch_add(10, Ordering::SeqCst);
            Ok(())
        }))
    }

    Modrun::builder()
        .no_banner()
        .supply(n)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();

    assert_eq!(hits.load(Ordering::SeqCst), 11);
}

#[tokio::test]
async fn task_error_surfaces_on_stop() {
    fn boot(lc: Lifecycle) -> modrun::Result<()> {
        lc.append(task("boom", |_stopped| async {
            Err(Error::hook("task-boom"))
        }))
    }

    let err = Modrun::builder()
        .no_banner()
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("task-boom"), "{err}");
}

#[tokio::test]
async fn task_abort_on_stop_timeout() {
    let dropped = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&dropped);

    struct Guard(Arc<AtomicBool>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn boot(lc: Lifecycle, dropped: Arc<AtomicBool>) -> modrun::Result<()> {
        lc.append(task("hang", move |_stopped| async move {
            let _guard = Guard(dropped);
            std::future::pending::<()>().await;
            Ok(())
        }))
    }

    let err = Modrun::builder()
        .no_banner()
        .stop_timeout(Duration::from_millis(30))
        .supply(flag)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("timed out"), "{err}");
    tokio::time::timeout(Duration::from_secs(1), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("aborted task should drop");
}

#[tokio::test]
async fn task_failure_can_request_shutdown() {
    fn boot(lc: Lifecycle, shutdown: modrun::Shutdowner) -> modrun::Result<()> {
        lc.append(task("die", move |_stopped| async move {
            shutdown.shutdown();
            Err(Error::hook("died"))
        }))
    }

    let err = Modrun::builder()
        .no_banner()
        .invoke(boot)
        .run()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("died"), "{err}");
}

#[tokio::test]
async fn task_with_prepare_error_fails_start() {
    fn boot(lc: Lifecycle) -> modrun::Result<()> {
        lc.append(task_with(
            "listen",
            || async { Err(Error::hook("addr in use")) },
            |(), _stopped| async { Ok(()) },
        ))
    }

    let err = Modrun::builder()
        .no_banner()
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    assert!(err.to_string().contains("addr in use"), "{err}");
}

#[tokio::test]
async fn task_with_bind_conflict_fails_start() {
    let held = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = held.local_addr().unwrap();

    fn boot(lc: Lifecycle, addr: std::net::SocketAddr) -> modrun::Result<()> {
        lc.append(task_with(
            "http.serve",
            move || async move {
                tokio::net::TcpListener::bind(addr)
                    .await
                    .map(|_| ())
                    .map_err(Into::into)
            },
            |(), stopped| async move {
                stopped.await;
                Ok(())
            },
        ))
    }

    let err = Modrun::builder()
        .no_banner()
        .supply(addr)
        .invoke(boot)
        .start()
        .await
        .unwrap_err();
    let _ = held;
    let msg = err.to_string();
    assert!(
        msg.contains("Address already in use")
            || msg.contains("addr in use")
            || msg.contains("os error"),
        "{msg}"
    );
}

#[tokio::test]
async fn task_with_prepare_value_is_passed_to_run() {
    let hits = Arc::new(AtomicUsize::new(0));
    let n = Arc::clone(&hits);

    fn boot(lc: Lifecycle, hits: Arc<AtomicUsize>) -> modrun::Result<()> {
        let prepared = Arc::clone(&hits);
        let running = Arc::clone(&hits);
        lc.append(task_with(
            "worker",
            move || {
                let prepared = Arc::clone(&prepared);
                async move {
                    prepared.fetch_add(1, Ordering::SeqCst);
                    Ok(7u8)
                }
            },
            move |value, stopped| {
                let running = Arc::clone(&running);
                async move {
                    running.fetch_add(usize::from(value), Ordering::SeqCst);
                    stopped.await;
                    Ok(())
                }
            },
        ))
    }

    Modrun::builder()
        .no_banner()
        .supply(n)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();

    assert_eq!(hits.load(Ordering::SeqCst), 8);
}
