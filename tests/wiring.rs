//! Provide / invoke / supply / graph-validation tests.

use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use modrun::Modrun;

#[tokio::test]
async fn provide_result_and_deps() {
    #[derive(Clone)]
    struct Config {
        port: u16,
    }

    #[derive(Clone)]
    struct Server {
        cfg: Config,
    }

    fn new_config() -> std::result::Result<Config, &'static str> {
        Ok(Config { port: 9090 })
    }

    fn new_server(cfg: Config) -> Server {
        Server { cfg }
    }

    fn check(server: Server) {
        assert_eq!(server.cfg.port, 9090);
    }

    Modrun::builder()
        .provide_result(new_config)
        .provide(new_server)
        .invoke(check)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn missing_provider_errors() {
    #[derive(Clone)]
    struct Missing;
    fn needs(_m: Missing) {}

    let err = Modrun::builder().invoke(needs).start().await.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("Missing") || msg.contains("missing"),
        "unexpected error: {msg}"
    );
}

#[tokio::test]
async fn duplicate_provide_errors() {
    fn a() -> u32 {
        1
    }
    fn b() -> u32 {
        2
    }

    let err = Modrun::builder()
        .provide(a)
        .provide(b)
        .invoke(|_: u32| {})
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("already provided"), "unexpected error: {msg}");
}

#[tokio::test]
async fn async_constructors_are_awaited_in_dependency_order() {
    #[derive(Clone)]
    struct Pool(&'static str);
    #[derive(Clone)]
    struct Repo(&'static str);

    async fn connect() -> Pool {
        tokio::task::yield_now().await;
        Pool("connected")
    }

    async fn new_repo(pool: Pool) -> std::result::Result<Repo, &'static str> {
        tokio::task::yield_now().await;
        Ok(Repo(pool.0))
    }

    fn check(repo: Repo) {
        assert_eq!(repo.0, "connected");
    }

    Modrun::builder()
        .provide_async(connect)
        .provide_result_async(new_repo)
        .invoke(check)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn async_constructor_failure_surfaces() {
    #[derive(Clone)]
    struct Pool;

    async fn connect() -> std::result::Result<Pool, &'static str> {
        Err("connection refused")
    }

    let err = Modrun::builder()
        .provide_result_async(connect)
        .invoke(|_p: Pool| {})
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("connection refused"), "unexpected: {msg}");
    let src = std::error::Error::source(&err).expect("source");
    assert!(
        src.to_string().contains("connection refused"),
        "source was {src}"
    );
}

/// Construction is lazy, so without an up-front check a provider that nothing
/// depends on could carry a missing dependency forever without complaint.
#[tokio::test]
async fn unused_provider_with_missing_dependency_fails_build() {
    #[derive(Clone)]
    struct Absent;
    #[derive(Clone)]
    struct Unused;

    fn new_unused(_absent: Absent) -> Unused {
        Unused
    }

    let err = Modrun::builder()
        .provide(new_unused)
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("nothing provides") && msg.contains("Absent"),
        "unexpected: {msg}"
    );
}

#[tokio::test]
async fn validation_errors_follow_registration_order() {
    #[derive(Clone)]
    struct MissingA;
    #[derive(Clone)]
    struct MissingB;
    #[derive(Clone)]
    struct A;
    #[derive(Clone)]
    struct B;

    fn new_a(_: MissingA) -> A {
        A
    }
    fn new_b(_: MissingB) -> B {
        B
    }

    for _ in 0..16 {
        let err = Modrun::builder()
            .provide(new_a)
            .provide(new_b)
            .start()
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("MissingA") && !msg.contains("MissingB"),
            "unexpected validation order: {msg}"
        );
    }
}

#[tokio::test]
async fn dependency_cycle_fails_build() {
    #[derive(Clone)]
    struct A;
    #[derive(Clone)]
    struct B;

    fn new_a(_b: B) -> A {
        A
    }
    fn new_b(_a: A) -> B {
        B
    }

    let err = Modrun::builder()
        .provide(new_a)
        .provide(new_b)
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("cycle"), "unexpected: {msg}");
    assert!(
        msg.contains("->"),
        "cycle error should list the path, got: {msg}"
    );
}

/// The same type provided privately in two sibling modules is two distinct
/// providers, not a cycle.
#[tokio::test]
async fn invoker_missing_dependency_fails_before_construction() {
    #[derive(Clone)]
    struct Built;
    #[derive(Clone)]
    struct Missing;

    fn needs(_b: Built, _m: Missing) {}

    let counter = Arc::new(AtomicUsize::new(0));
    let err = Modrun::builder()
        .provide({
            let counter = Arc::clone(&counter);
            move || {
                counter.fetch_add(1, Ordering::SeqCst);
                Built
            }
        })
        .invoke(needs)
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("nothing provides") && msg.contains("Missing"),
        "unexpected: {msg}"
    );
    assert_eq!(
        counter.load(Ordering::SeqCst),
        0,
        "Built must not be constructed before validation fails"
    );
}

#[tokio::test]
async fn fallible_invoker_error_is_contextualized() {
    fn boom() -> std::result::Result<(), &'static str> {
        Err("nope")
    }

    let err = Modrun::builder().invoke(boom).start().await.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("invoker") && msg.contains("failed") && msg.contains("nope"),
        "unexpected: {msg}"
    );
    assert!(msg.contains("boom"), "unexpected: {msg}");
    let src = std::error::Error::source(&err).expect("source");
    assert!(src.to_string().contains("nope"), "source was {src}");
}

#[tokio::test]
async fn duplicate_supply_errors() {
    let err = Modrun::builder()
        .supply(1u32)
        .supply(2u32)
        .invoke(|_: u32| {})
        .start()
        .await
        .unwrap_err();
    assert!(
        format!("{err}").contains("already provided"),
        "unexpected: {err}"
    );
}

#[tokio::test]
async fn empty_builder_starts_and_stops() {
    Modrun::builder()
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn provide_dyn_and_invoke_dyn() {
    use std::any::TypeId;

    use modrun::__wiring::{InvokeFn, ProviderFn};

    #[derive(Clone)]
    struct N(u8);

    fn new_n() -> N {
        N(7)
    }
    fn check(n: N) {
        assert_eq!(n.0, 7);
    }

    let provider = new_n.into_provider();
    assert_eq!(provider.result_type(), TypeId::of::<N>());
    assert!(provider.result_name().ends_with("::N"));
    assert!(provider.dep_types().is_empty());

    Modrun::builder()
        .provide_dyn(provider)
        .invoke_dyn(check.into_invoke())
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn inject_arc_without_cloning_t() {
    struct Token;

    fn new_token() -> Token {
        Token
    }

    fn boot(token: Arc<Token>) {
        let _ = token;
    }

    Modrun::builder()
        .provide(new_token)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn constructor_can_depend_on_arc_of_provider_result() {
    struct Config;
    struct Service(Arc<Config>);

    fn new_config() -> Config {
        Config
    }
    fn new_service(config: Arc<Config>) -> Service {
        Service(config)
    }
    fn boot(service: Arc<Service>) {
        let _ = &service.0;
    }

    Modrun::builder()
        .provide(new_config)
        .provide(new_service)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn cycle_through_arc_dependency_is_reported_during_validation() {
    #[derive(Clone)]
    struct A;
    struct B;

    fn new_a(_b: Arc<B>) -> A {
        A
    }
    fn new_b(_a: A) -> B {
        B
    }

    let err = Modrun::builder()
        .provide(new_a)
        .provide(new_b)
        .start()
        .await
        .unwrap_err();
    let msg = err.to_string();
    assert!(
        msg.contains("cycle") && msg.contains("->"),
        "unexpected: {msg}"
    );
}

#[tokio::test]
async fn provide_mut_and_invoke_async() {
    struct N(u8);

    fn new_n() -> N {
        N(3)
    }

    async fn check(n: Arc<N>) {
        assert_eq!(n.0, 3);
    }

    let mut b = Modrun::builder();
    b.provide_mut(new_n);
    b.invoke_async_mut(check);
    b.start().await.unwrap().stop().await.unwrap();
}

#[tokio::test]
async fn three_node_cycle_is_reported() {
    #[derive(Clone)]
    struct NodeA;
    #[derive(Clone)]
    struct NodeB;
    #[derive(Clone)]
    struct NodeC;

    fn new_a(_b: NodeB) -> NodeA {
        NodeA
    }
    fn new_b(_c: NodeC) -> NodeB {
        NodeB
    }
    fn new_c(_a: NodeA) -> NodeC {
        NodeC
    }
    fn use_a(_a: NodeA) {}

    let err = Modrun::builder()
        .provide(new_a)
        .provide(new_b)
        .provide(new_c)
        .invoke(use_a)
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(msg.contains("cycle"), "unexpected: {msg}");
    assert!(msg.contains("->"), "unexpected: {msg}");
    assert!(
        msg.contains("NodeA") && msg.contains("NodeB") && msg.contains("NodeC"),
        "unexpected: {msg}"
    );
}

#[tokio::test]
async fn eight_parameter_provide_and_invoke() {
    #[derive(Clone)]
    struct P1(u8);
    #[derive(Clone)]
    struct P2(u8);
    #[derive(Clone)]
    struct P3(u8);
    #[derive(Clone)]
    struct P4(u8);
    #[derive(Clone)]
    struct P5(u8);
    #[derive(Clone)]
    struct P6(u8);
    #[derive(Clone)]
    struct P7(u8);
    #[derive(Clone)]
    struct P8(u8);
    #[derive(Clone)]
    struct Eight(u8);

    #[allow(clippy::too_many_arguments)]
    fn new_eight(p1: P1, p2: P2, p3: P3, p4: P4, p5: P5, p6: P6, p7: P7, p8: P8) -> Eight {
        Eight(p1.0 + p2.0 + p3.0 + p4.0 + p5.0 + p6.0 + p7.0 + p8.0)
    }

    fn boot(eight: Eight) {
        assert_eq!(eight.0, 36);
    }

    Modrun::builder()
        .supply(P1(1))
        .supply(P2(2))
        .supply(P3(3))
        .supply(P4(4))
        .supply(P5(5))
        .supply(P6(6))
        .supply(P7(7))
        .supply(P8(8))
        .provide(new_eight)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}
