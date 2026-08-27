//! Build / lifecycle microbenchmarks for cold-start tuning.
#![allow(missing_docs, dead_code)]

use std::time::Duration;

use criterion::{Criterion, criterion_group, criterion_main};
use modrun::{Lifecycle, Modrun, Module, hook};
use tokio::runtime::Runtime;

fn runtime() -> Runtime {
    Runtime::new().expect("runtime")
}

fn build_small(c: &mut Criterion) {
    let rt = runtime();
    c.bench_function("build_small_sync", |b| {
        b.to_async(&rt).iter(|| async {
            #[derive(Clone)]
            struct Cfg(u32);
            #[derive(Clone)]
            struct Svc(u32);

            Modrun::builder()
                .no_banner()
                .supply(Cfg(1))
                .provide(|c: Cfg| Svc(c.0 + 1))
                .invoke(|_: Svc| {})
                .start()
                .await
                .unwrap()
                .stop()
                .await
                .unwrap();
        });
    });
}

fn build_deep_modules(c: &mut Criterion) {
    let rt = runtime();
    c.bench_function("build_deep_modules", |b| {
        b.to_async(&rt).iter(|| async {
            #[derive(Clone)]
            struct Leaf;

            fn leaf() -> Leaf {
                Leaf
            }
            fn use_leaf(_: Leaf) {}

            let mut module = Module::new("m0").provide_private(leaf).invoke(use_leaf);
            for i in 1..12 {
                // Nested modules deepen the ancestor walk.
                let name: &'static str = match i {
                    1 => "m1",
                    2 => "m2",
                    3 => "m3",
                    4 => "m4",
                    5 => "m5",
                    6 => "m6",
                    7 => "m7",
                    8 => "m8",
                    9 => "m9",
                    10 => "m10",
                    _ => "m11",
                };
                module = Module::new(name).module(module);
            }

            Modrun::builder()
                .no_banner()
                .module(module)
                .start()
                .await
                .unwrap()
                .stop()
                .await
                .unwrap();
        });
    });
}

fn lifecycle_hooks(c: &mut Criterion) {
    let rt = runtime();
    c.bench_function("lifecycle_100_hooks", |b| {
        b.to_async(&rt).iter(|| async {
            fn boot(lc: Lifecycle) {
                for _ in 0..100 {
                    lc.append(
                        hook()
                            .on_start(|| async { Ok(()) })
                            .on_stop(|| async { Ok(()) }),
                    )
                    .unwrap();
                }
            }

            Modrun::builder()
                .no_banner()
                .invoke(boot)
                .start()
                .await
                .unwrap()
                .stop()
                .await
                .unwrap();
        });
    });
}

fn async_independent_ctors(c: &mut Criterion) {
    let rt = runtime();
    c.bench_function("async_independent_ctors", |b| {
        b.to_async(&rt).iter(|| async {
            #[derive(Clone)]
            struct A;
            #[derive(Clone)]
            struct B;
            #[derive(Clone)]
            struct C;
            #[derive(Clone)]
            struct D;

            async fn a() -> A {
                tokio::time::sleep(Duration::from_millis(1)).await;
                A
            }
            async fn b() -> B {
                tokio::time::sleep(Duration::from_millis(1)).await;
                B
            }
            async fn c() -> C {
                tokio::time::sleep(Duration::from_millis(1)).await;
                C
            }
            async fn d() -> D {
                tokio::time::sleep(Duration::from_millis(1)).await;
                D
            }

            Modrun::builder()
                .no_banner()
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
        });
    });
}

criterion_group!(
    benches,
    build_small,
    build_deep_modules,
    lifecycle_hooks,
    async_independent_ctors
);
criterion_main!(benches);
