//! Value group wiring tests.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use modrun::__wiring::ProviderFn;
use modrun::{Group, Modrun, Module};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Route(&'static str);

#[derive(Clone)]
struct Hits(Arc<AtomicUsize>);

#[derive(Debug)]
struct CloneCounter(Arc<AtomicUsize>);

impl Clone for CloneCounter {
    fn clone(&self) -> Self {
        self.0.fetch_add(1, Ordering::SeqCst);
        Self(Arc::clone(&self.0))
    }
}

#[derive(Clone, Debug)]
struct TrackedItem {
    clones: CloneCounter,
}

#[tokio::test]
async fn multiple_modules_aggregate_in_registration_order() {
    fn user_route() -> Route {
        Route("user")
    }

    fn order_route() -> Route {
        Route("order")
    }

    fn boot(routes: Group<Route>) {
        let names: Vec<_> = routes.iter().map(|r| r.0).collect();
        assert_eq!(names, vec!["user", "order"]);
    }

    Modrun::builder()
        .no_banner()
        .module(Module::builder("user").provide_group(user_route))
        .module(Module::builder("order").provide_group(order_route))
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn empty_group_with_init_group() {
    fn boot(routes: Group<Route>) {
        assert!(routes.is_empty());
        assert_eq!(&routes[..], &[] as &[Route]);
    }

    Modrun::builder()
        .no_banner()
        .init_group::<Route>()
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn empty_group_without_registration_is_missing() {
    fn boot(_routes: Group<Route>) {}

    let err = Modrun::builder()
        .no_banner()
        .invoke(boot)
        .start()
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(
        msg.contains("needs a dependency nothing provides"),
        "unexpected: {msg}"
    );
    assert!(msg.contains("init_group"), "unexpected: {msg}");
}

#[tokio::test]
async fn constructor_can_depend_on_init_group() {
    #[derive(Clone)]
    struct Wrap(usize);

    fn wrap(routes: Group<Route>) -> Wrap {
        Wrap(routes.len())
    }

    fn boot(wrap: Wrap) {
        assert_eq!(wrap.0, 0);
    }

    Modrun::builder()
        .no_banner()
        .init_group::<Route>()
        .provide(wrap)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn group_member_can_use_private_dependencies() {
    #[derive(Clone)]
    struct Repo;

    fn new_repo() -> Repo {
        Repo
    }

    fn user_route(_repo: Repo) -> Route {
        Route("user")
    }

    fn boot(routes: Group<Route>) {
        assert_eq!(routes.len(), 1);
        assert_eq!(routes.as_slice()[0], Route("user"));
    }

    Modrun::builder()
        .no_banner()
        .module(
            Module::builder("user")
                .provide_private(new_repo)
                .provide_group(user_route),
        )
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn supply_group_registers_member() {
    fn boot(routes: Group<Route>) {
        assert_eq!(routes.as_slice(), &[Route("fake")]);
    }

    Modrun::builder()
        .no_banner()
        .supply_group(Route("fake"))
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn singleton_and_group_of_same_type_coexist() {
    #[derive(Clone, PartialEq, Eq, Debug)]
    struct Handler(&'static str);

    fn health() -> Handler {
        Handler("health")
    }

    fn api() -> Handler {
        Handler("api")
    }

    fn boot(single: Handler, many: Group<Handler>) {
        assert_eq!(single, Handler("health"));
        assert_eq!(many.as_slice(), &[Handler("api")]);
    }

    Modrun::builder()
        .no_banner()
        .provide(health)
        .provide_group(api)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn same_ctor_in_two_groups_runs_twice() {
    #[derive(Clone, PartialEq, Eq)]
    struct A(&'static str);

    #[derive(Clone, PartialEq, Eq)]
    struct B(&'static str);

    fn make() -> Hits {
        Hits(Arc::new(AtomicUsize::new(0)))
    }

    fn to_a(hits: Hits) -> A {
        hits.0.fetch_add(1, Ordering::SeqCst);
        A("a")
    }

    fn to_b(hits: Hits) -> B {
        hits.0.fetch_add(1, Ordering::SeqCst);
        B("b")
    }

    fn boot(hits: Hits, _a: Group<A>, _b: Group<B>) {
        assert_eq!(hits.0.load(Ordering::SeqCst), 2);
    }

    Modrun::builder()
        .no_banner()
        .provide(make)
        .provide_group(to_a)
        .provide_group(to_b)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn require_group_fails_when_empty() {
    fn boot(_routes: Group<Route>) {}

    let err = Modrun::builder()
        .no_banner()
        .require_group::<Route>()
        .invoke(boot)
        .start()
        .await
        .unwrap_err();

    assert!(format!("{err}").contains("required group is empty"));
    assert!(format!("{err}").contains("Group<"));
    assert!(format!("{err}").contains("Route"));
}

#[tokio::test]
async fn group_into_iter_works() {
    fn one() -> Route {
        Route("one")
    }
    fn two() -> Route {
        Route("two")
    }

    fn boot(routes: Group<Route>) {
        let collected: Vec<_> = routes.into_iter().collect();
        assert_eq!(collected, vec![Route("one"), Route("two")]);
    }

    Modrun::builder()
        .no_banner()
        .provide_group(one)
        .provide_group(two)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn provide_group_result_async_member() {
    async fn connect() -> Result<Route, std::io::Error> {
        Ok(Route("async"))
    }

    fn boot(routes: Group<Route>) {
        assert_eq!(routes.as_slice(), &[Route("async")]);
    }

    Modrun::builder()
        .no_banner()
        .provide_group_result_async(connect)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn require_group_alone_fails_when_empty() {
    let err = Modrun::builder()
        .no_banner()
        .require_group::<Route>()
        .invoke(|| {})
        .start()
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(msg.contains("required group is empty"), "unexpected: {msg}");
    assert!(msg.contains("Group<"), "unexpected: {msg}");
    assert!(msg.contains("Route"), "unexpected: {msg}");
}

#[tokio::test]
async fn group_member_missing_dep_fails_at_validate() {
    #[derive(Clone)]
    struct Missing;

    fn member(_missing: Missing) -> Route {
        Route("x")
    }

    fn boot(_routes: Group<Route>) {}

    let err = Modrun::builder()
        .no_banner()
        .provide_group(member)
        .invoke(boot)
        .start()
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(
        msg.contains("needs a dependency nothing provides"),
        "unexpected: {msg}"
    );
}

#[tokio::test]
async fn unused_group_member_missing_dep_still_fails() {
    #[derive(Clone)]
    struct Missing;

    fn member(_missing: Missing) -> Route {
        Route("x")
    }

    let err = Modrun::builder()
        .no_banner()
        .provide_group(member)
        .invoke(|| {})
        .start()
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(
        msg.contains("needs a dependency nothing provides"),
        "unexpected: {msg}"
    );
}

#[tokio::test]
async fn cycle_through_group_is_detected() {
    fn member(_routes: Group<Route>) -> Route {
        Route("x")
    }

    fn boot(_routes: Group<Route>) {}

    let err = Modrun::builder()
        .no_banner()
        .provide_group(member)
        .invoke(boot)
        .start()
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(msg.contains("dependency cycle"), "unexpected: {msg}");
    assert!(msg.contains("member"), "unexpected: {msg}");
}

#[tokio::test]
async fn provide_group_conflicts_with_group_singleton() {
    fn boot(_routes: Group<Route>) {}

    let err = Modrun::builder()
        .no_banner()
        .provide(Group::<Route>::new)
        .provide_group(|| Route("x"))
        .invoke(boot)
        .start()
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(msg.contains("already provided"), "unexpected: {msg}");
}

#[tokio::test]
async fn arc_group_is_injectable() {
    fn one() -> Route {
        Route("one")
    }

    fn boot(routes: Arc<Group<Route>>) {
        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0], Route("one"));
    }

    Modrun::builder()
        .no_banner()
        .provide_group(one)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn trait_object_group() {
    trait Named: Send + Sync {
        fn name(&self) -> &'static str;
    }

    struct User;
    impl Named for User {
        fn name(&self) -> &'static str {
            "user"
        }
    }

    fn user() -> Arc<dyn Named> {
        Arc::new(User)
    }

    fn boot(handlers: Group<Arc<dyn Named>>) {
        let names: Vec<_> = handlers.iter().map(|h| h.name()).collect();
        assert_eq!(names, vec!["user"]);
    }

    Modrun::builder()
        .no_banner()
        .provide_group(user)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn nested_modules_aggregate_in_dfs_order() {
    fn user() -> Route {
        Route("user")
    }
    fn order() -> Route {
        Route("order")
    }
    fn payment() -> Route {
        Route("payment")
    }

    fn boot(routes: Group<Route>) {
        let names: Vec<_> = routes.iter().map(|r| r.0).collect();
        assert_eq!(names, vec!["user", "order", "payment"]);
    }

    Modrun::builder()
        .no_banner()
        .module(
            Module::builder("api")
                .module(Module::builder("user").provide_group(user))
                .module(Module::builder("order").provide_group(order)),
        )
        .module(Module::builder("payment").provide_group(payment))
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn provide_group_result_member() {
    fn try_route() -> Result<Route, std::io::Error> {
        Ok(Route("ok"))
    }

    fn boot(routes: Group<Route>) {
        assert_eq!(&*routes, &[Route("ok")]);
    }

    Modrun::builder()
        .no_banner()
        .provide_group_result(try_route)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn provide_group_on_module_can_use_private_deps() {
    #[derive(Clone)]
    struct Repo;

    fn new_repo() -> Repo {
        Repo
    }

    fn user_route(_repo: Repo) -> Route {
        Route("user")
    }

    fn boot(routes: Group<Route>) {
        assert_eq!(routes.as_slice(), &[Route("user")]);
    }

    Modrun::builder()
        .no_banner()
        .module(
            Module::builder("user")
                .provide_private(new_repo)
                .provide_group(user_route),
        )
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn private_group_shadows_aggregate_registered_first() {
    fn public_route() -> Route {
        Route("public")
    }

    fn private_routes() -> Group<Route> {
        Group::from_vec(vec![Route("private")])
    }

    fn boot(routes: Group<Route>, shared: Arc<Group<Route>>) {
        assert_eq!(routes.as_slice(), &[Route("private")]);
        assert_eq!(shared.as_slice(), &[Route("private")]);
    }

    Modrun::builder()
        .no_banner()
        .provide_group(public_route)
        .module(
            Module::builder("private")
                .provide_private(private_routes)
                .invoke(boot),
        )
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn private_group_shadows_aggregate_registered_last() {
    fn public_route() -> Route {
        Route("public")
    }

    fn private_routes() -> Group<Route> {
        Group::from_vec(vec![Route("private")])
    }

    fn boot(routes: Group<Route>) {
        assert_eq!(routes.as_slice(), &[Route("private")]);
    }

    Modrun::builder()
        .no_banner()
        .module(
            Module::builder("private")
                .provide_private(private_routes)
                .invoke(boot),
        )
        .provide_group(public_route)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn provide_group_async_member() {
    async fn connect() -> Route {
        Route("async")
    }

    fn boot(routes: Group<Route>) {
        assert_eq!(routes.as_slice(), &[Route("async")]);
    }

    Modrun::builder()
        .no_banner()
        .provide_group_async(connect)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn provide_group_dyn_registers_member() {
    fn one() -> Route {
        Route("dyn")
    }

    fn boot(routes: Group<Route>) {
        assert_eq!(routes.as_slice(), &[Route("dyn")]);
    }

    Modrun::builder()
        .no_banner()
        .provide_group_dyn::<Route>(one.into_provider())
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn provide_group_result_failure_propagates() {
    fn fail() -> Result<Route, std::io::Error> {
        Err(std::io::Error::other("group member failed"))
    }

    fn boot(_routes: Group<Route>) {}

    let err = Modrun::builder()
        .no_banner()
        .provide_group_result(fail)
        .invoke(boot)
        .start()
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(msg.contains("group member failed"), "unexpected: {msg}");
}

#[tokio::test]
async fn provide_group_result_async_failure_propagates() {
    async fn fail() -> Result<Route, std::io::Error> {
        Err(std::io::Error::other("async group member failed"))
    }

    fn boot(_routes: Group<Route>) {}

    let err = Modrun::builder()
        .no_banner()
        .provide_group_result_async(fail)
        .invoke(boot)
        .start()
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(
        msg.contains("async group member failed"),
        "unexpected: {msg}"
    );
}

#[tokio::test]
async fn provide_group_with_require_group_starts() {
    fn user_route() -> Route {
        Route("user")
    }

    fn boot(routes: Group<Route>) {
        assert_eq!(routes.as_slice(), &[Route("user")]);
    }

    Modrun::builder()
        .no_banner()
        .provide_group(user_route)
        .require_group::<Route>()
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn module_supply_group_aggregates() {
    fn boot(routes: Group<Route>) {
        assert_eq!(routes.as_slice(), &[Route("a"), Route("b")]);
    }

    Modrun::builder()
        .no_banner()
        .module(
            Module::builder("handlers")
                .supply_group(Route("a"))
                .supply_group(Route("b")),
        )
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn root_invoker_sees_global_group_when_submodule_has_private_group() {
    fn public_route() -> Route {
        Route("public")
    }

    fn private_routes() -> Group<Route> {
        Group::from_vec(vec![Route("private")])
    }

    fn boot(routes: Group<Route>) {
        assert_eq!(routes.as_slice(), &[Route("public")]);
    }

    Modrun::builder()
        .no_banner()
        .provide_group(public_route)
        .module(Module::builder("private").provide_private(private_routes))
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn group_aggregation_moves_members_without_cloning() {
    fn clone_hits() -> Arc<AtomicUsize> {
        Arc::new(AtomicUsize::new(0))
    }

    fn a(h: Arc<AtomicUsize>) -> TrackedItem {
        TrackedItem {
            clones: CloneCounter(Arc::clone(&h)),
        }
    }

    fn b(h: Arc<AtomicUsize>) -> TrackedItem {
        TrackedItem {
            clones: CloneCounter(Arc::clone(&h)),
        }
    }

    fn c(h: Arc<AtomicUsize>) -> TrackedItem {
        TrackedItem {
            clones: CloneCounter(Arc::clone(&h)),
        }
    }

    fn boot(h: Arc<AtomicUsize>, items: Group<TrackedItem>) {
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|item| Arc::ptr_eq(&item.clones.0, &h)));
        assert_eq!(
            h.load(Ordering::SeqCst),
            3,
            "move aggregation should not clone members; only injecting Group<T> clones once per item"
        );
    }

    Modrun::builder()
        .no_banner()
        .provide(clone_hits)
        .provide_group(a)
        .provide_group(b)
        .provide_group(c)
        .invoke(boot)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn supply_group_aggregation_moves_members_without_cloning() {
    let hits = Arc::new(AtomicUsize::new(0));

    Modrun::builder()
        .no_banner()
        .supply_group(TrackedItem {
            clones: CloneCounter(Arc::clone(&hits)),
        })
        .supply_group(TrackedItem {
            clones: CloneCounter(Arc::clone(&hits)),
        })
        .supply_group(TrackedItem {
            clones: CloneCounter(Arc::clone(&hits)),
        })
        .invoke(move |items: Group<TrackedItem>| {
            assert_eq!(items.len(), 3);
            assert!(items.iter().all(|item| Arc::ptr_eq(&item.clones.0, &hits)));
            assert_eq!(
                hits.load(Ordering::SeqCst),
                3,
                "supply_group must move members; only injecting Group<T> clones once per item"
            );
        })
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn provide_group_dyn_type_mismatch_fails_at_build() {
    fn one() -> Route {
        Route("one")
    }

    let err = Modrun::builder()
        .no_banner()
        .provide_group_dyn::<String>(one.into_provider())
        .invoke(|| {})
        .start()
        .await
        .unwrap_err();

    let msg = format!("{err}");
    assert!(
        msg.contains("provide_group_dyn type mismatch"),
        "unexpected: {msg}"
    );
    assert!(msg.contains("String"), "unexpected: {msg}");
    assert!(msg.contains("Route"), "unexpected: {msg}");
}
