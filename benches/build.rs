//! Build / lifecycle microbenchmarks for cold-start tuning.
#![allow(missing_docs, dead_code)]

use std::sync::Arc;
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use modrun::{Group, Lifecycle, Modrun, ModrunBuilder, Module, hook};
use tokio::runtime::Runtime;
use tokio::time::sleep;

const GROUP_MEMBER_COUNTS: [usize; 4] = [8, 32, 128, 512];
const MEMBER_PAYLOAD_BYTES: usize = 512;

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

            let mut module = Module::builder("m0").provide_private(leaf).invoke(use_leaf);
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
                module = Module::builder(name).module(module);
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
                sleep(Duration::from_millis(1)).await;
                A
            }
            async fn b() -> B {
                sleep(Duration::from_millis(1)).await;
                B
            }
            async fn c() -> C {
                sleep(Duration::from_millis(1)).await;
                C
            }
            async fn d() -> D {
                sleep(Duration::from_millis(1)).await;
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

/// Group member with a fixed payload so `Clone` cost is visible in benchmarks.
#[derive(Clone, PartialEq, Eq)]
struct BenchMember {
    id: u32,
    payload: Vec<u8>,
}

fn make_member(id: u32) -> BenchMember {
    BenchMember {
        id,
        payload: vec![id as u8; MEMBER_PAYLOAD_BYTES],
    }
}

#[derive(Clone, Copy)]
enum GroupSource {
    Provide,
    Supply,
}

#[derive(Clone, Copy)]
enum GroupInject {
    Value,
    Arc,
}

fn register_group_members(
    builder: ModrunBuilder,
    count: usize,
    source: GroupSource,
) -> ModrunBuilder {
    let mut builder = builder;
    match source {
        GroupSource::Provide => {
            for i in 0..count {
                let id = i as u32;
                builder = builder.provide_group(move || make_member(id));
            }
        }
        GroupSource::Supply => {
            for i in 0..count {
                builder = builder.supply_group(make_member(i as u32));
            }
        }
    }
    builder
}

async fn cold_start_with_group(count: usize, source: GroupSource, inject: GroupInject) {
    let builder = register_group_members(Modrun::builder().no_banner(), count, source);
    match inject {
        GroupInject::Value => {
            builder
                .invoke(|_: Group<BenchMember>| {})
                .start()
                .await
                .unwrap()
                .stop()
                .await
                .unwrap();
        }
        GroupInject::Arc => {
            builder
                .invoke(|_: Arc<Group<BenchMember>>| {})
                .start()
                .await
                .unwrap()
                .stop()
                .await
                .unwrap();
        }
    }
}

fn bench_groups(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("build_groups");

    for count in GROUP_MEMBER_COUNTS {
        for source in [GroupSource::Provide, GroupSource::Supply] {
            for inject in [GroupInject::Value, GroupInject::Arc] {
                let source_label = match source {
                    GroupSource::Provide => "provide",
                    GroupSource::Supply => "supply",
                };
                let inject_label = match inject {
                    GroupInject::Value => "value",
                    GroupInject::Arc => "arc",
                };
                let id = BenchmarkId::new(format!("{source_label}/{inject_label}"), count);
                group.bench_with_input(id, &count, |b, &count| {
                    b.to_async(&rt)
                        .iter(|| cold_start_with_group(count, source, inject));
                });
            }
        }
    }

    group.finish();
}

// --- Ancestor-chain resolution stress -------------------------------------
//
// Every public dependency lookup walks the whole scope chain from the current
// module up to root (checking private bindings at each level) before falling
// through to `values_public`. This benchmark nests `DEPTH` modules and, at the
// deepest one, constructs 8 services that each depend on 3 root-level public
// values — so each construction walks the full chain three times.

#[derive(Clone)]
struct Root0(u32);
#[derive(Clone)]
struct Root1(u32);
#[derive(Clone)]
struct Root2(u32);

#[derive(Clone)]
struct Deep<const N: usize>(u32);

fn make_deep<const N: usize>(a: Root0, b: Root1, c: Root2) -> Deep<N> {
    Deep(a.0 + b.0 + c.0 + N as u32)
}

const DEEP_DEPTHS: [usize; 3] = [8, 24, 48];

fn nest_to_depth(leaf: Module, depth: usize) -> Module {
    // Module names are `&'static str`; a fixed pool keeps names static while
    // still deepening the ancestor chain to `depth`.
    const NAMES: [&str; 48] = [
        "d00", "d01", "d02", "d03", "d04", "d05", "d06", "d07", "d08", "d09", "d10", "d11", "d12",
        "d13", "d14", "d15", "d16", "d17", "d18", "d19", "d20", "d21", "d22", "d23", "d24", "d25",
        "d26", "d27", "d28", "d29", "d30", "d31", "d32", "d33", "d34", "d35", "d36", "d37", "d38",
        "d39", "d40", "d41", "d42", "d43", "d44", "d45", "d46", "d47",
    ];
    let mut module = leaf;
    for level in 0..depth {
        module = Module::builder(NAMES[level % NAMES.len()]).module(module);
    }
    module
}

async fn cold_start_deep_resolution(depth: usize) {
    let leaf = Module::builder("leaf")
        .provide(make_deep::<0>)
        .provide(make_deep::<1>)
        .provide(make_deep::<2>)
        .provide(make_deep::<3>)
        .provide(make_deep::<4>)
        .provide(make_deep::<5>)
        .provide(make_deep::<6>)
        .provide(make_deep::<7>)
        .invoke(
            |_: Deep<0>,
             _: Deep<1>,
             _: Deep<2>,
             _: Deep<3>,
             _: Deep<4>,
             _: Deep<5>,
             _: Deep<6>,
             _: Deep<7>| {},
        );

    Modrun::builder()
        .no_banner()
        .supply(Root0(1))
        .supply(Root1(2))
        .supply(Root2(3))
        .module(nest_to_depth(leaf, depth))
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

fn bench_deep_resolution(c: &mut Criterion) {
    let rt = runtime();
    let mut group = c.benchmark_group("deep_resolution");
    for depth in DEEP_DEPTHS {
        group.bench_with_input(BenchmarkId::from_parameter(depth), &depth, |b, &depth| {
            b.to_async(&rt).iter(|| cold_start_deep_resolution(depth));
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    build_small,
    build_deep_modules,
    bench_groups,
    lifecycle_hooks,
    async_independent_ctors,
    bench_deep_resolution
);
criterion_main!(benches);
