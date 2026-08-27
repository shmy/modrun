//! Concurrent constructor tests.

use std::sync::Arc;
use std::time::Duration;

use modrun::Modrun;
use tokio::sync::Barrier;

#[tokio::test]
async fn independent_async_ctors_run_concurrently() {
    #[derive(Clone)]
    struct A;
    #[derive(Clone)]
    struct B;
    #[derive(Clone)]
    struct C;
    #[derive(Clone)]
    struct D;

    async fn a(barrier: Arc<Barrier>) -> A {
        barrier.wait().await;
        A
    }
    async fn b(barrier: Arc<Barrier>) -> B {
        barrier.wait().await;
        B
    }
    async fn c(barrier: Arc<Barrier>) -> C {
        barrier.wait().await;
        C
    }
    async fn d(barrier: Arc<Barrier>) -> D {
        barrier.wait().await;
        D
    }

    let barrier = Arc::new(Barrier::new(4));
    Modrun::builder()
        .start_timeout(Duration::from_secs(2))
        .supply(Arc::clone(&barrier))
        .provide_async(a)
        .provide_async(b)
        .provide_async(c)
        .provide_async(d)
        .invoke(|_: A, _: B, _: C, _: D| {})
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}
