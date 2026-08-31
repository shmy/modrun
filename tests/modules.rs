//! Module scoping and private-provider tests.

use std::sync::Arc;

use modrun::{Modrun, Module};

#[tokio::test]
async fn domain_modules_with_private_deps() {
    #[derive(Clone)]
    struct UserRepo;
    #[derive(Clone)]
    struct UserService;
    #[derive(Clone)]
    struct OrderRepo;
    #[derive(Clone)]
    struct OrderService;

    fn new_user_repo() -> UserRepo {
        UserRepo
    }
    fn new_user_service(_repo: UserRepo) -> UserService {
        UserService
    }
    fn new_order_repo() -> OrderRepo {
        OrderRepo
    }
    fn new_order_service(_repo: OrderRepo) -> OrderService {
        OrderService
    }

    fn boot_user(_svc: UserService) {}
    fn boot_order(_svc: OrderService) {}

    Modrun::builder()
        .module(
            Module::builder("user")
                .provide_private(new_user_repo)
                .provide(new_user_service)
                .invoke(boot_user),
        )
        .module(
            Module::builder("order")
                .provide_private(new_order_repo)
                .provide(new_order_service)
                .invoke(boot_order),
        )
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn module_constructor_can_depend_on_arc_of_private_provider() {
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
        .module(
            Module::builder("domain")
                .provide_private(new_config)
                .provide(new_service)
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
async fn private_not_visible_outside_module() {
    #[derive(Clone)]
    struct Secret;

    fn new_secret() -> Secret {
        Secret
    }
    fn needs_secret(_s: Secret) {}

    let err = Modrun::builder()
        .module(Module::builder("user").provide_private(new_secret))
        .invoke(needs_secret)
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("missing") || msg.contains("Secret"),
        "unexpected: {msg}"
    );
}

#[tokio::test]
async fn sibling_private_providers_are_not_a_cycle() {
    #[derive(Clone)]
    struct Shared;
    #[derive(Clone)]
    struct Repo(&'static str);

    fn new_shared() -> Shared {
        Shared
    }
    fn new_repo_a(_s: Shared) -> Repo {
        Repo("a")
    }
    fn new_repo_b(_s: Shared) -> Repo {
        Repo("b")
    }

    fn check_a(repo: Repo) {
        assert_eq!(repo.0, "a");
    }
    fn check_b(repo: Repo) {
        assert_eq!(repo.0, "b");
    }

    Modrun::builder()
        .provide(new_shared)
        .module(
            Module::builder("a")
                .provide_private(new_repo_a)
                .invoke(check_a),
        )
        .module(
            Module::builder("b")
                .provide_private(new_repo_b)
                .invoke(check_b),
        )
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn sibling_domains_can_share_same_private_type() {
    #[derive(Clone, Debug, PartialEq)]
    struct LocalConfig {
        name: &'static str,
    }

    fn boot_a(cfg: LocalConfig) {
        assert_eq!(cfg.name, "a");
    }
    fn boot_b(cfg: LocalConfig) {
        assert_eq!(cfg.name, "b");
    }

    Modrun::builder()
        .module(
            Module::builder("a")
                .supply_private(LocalConfig { name: "a" })
                .invoke(boot_a),
        )
        .module(
            Module::builder("b")
                .supply_private(LocalConfig { name: "b" })
                .invoke(boot_b),
        )
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn private_provider_shadows_cached_public_value() {
    #[derive(Clone, Debug, PartialEq)]
    struct Config(&'static str);

    fn private_config() -> Config {
        Config("private")
    }

    fn check(cfg: Config) {
        assert_eq!(cfg, Config("private"));
    }

    Modrun::builder()
        .supply(Config("public"))
        .module(
            Module::builder("domain")
                .provide_private(private_config)
                .invoke(check),
        )
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn supplied_private_value_breaks_apparent_cycle() {
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

    fn use_a(_a: A) {}

    Modrun::builder()
        .provide(new_b)
        .module(
            Module::builder("domain")
                .supply_private(B)
                .provide(new_a)
                .invoke(use_a),
        )
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn same_scope_public_can_decorate_private() {
    #[derive(Clone, Debug, PartialEq)]
    struct Label(&'static str);

    fn private_label() -> Label {
        Label("inner")
    }

    fn public_label(inner: Label) -> Label {
        assert_eq!(inner, Label("inner"));
        Label("outer")
    }

    fn check(label: Label) {
        assert_eq!(label, Label("outer"));
    }

    // Invoke from outside the module so resolution picks the public binding;
    // the public constructor still sees the private one from its own scope.
    Modrun::builder()
        .module(
            Module::builder("domain")
                .provide_private(private_label)
                .provide(public_label),
        )
        .invoke(check)
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn nested_child_uses_parent_private_provider() {
    #[derive(Clone)]
    struct ParentSecret;
    #[derive(Clone)]
    struct ChildSvc;

    fn new_parent_secret() -> ParentSecret {
        ParentSecret
    }
    fn new_child_svc(_secret: ParentSecret) -> ChildSvc {
        ChildSvc
    }
    fn boot_child(_svc: ChildSvc) {}

    Modrun::builder()
        .module(
            Module::builder("parent")
                .provide_private(new_parent_secret)
                .module(
                    Module::builder("child")
                        .provide(new_child_svc)
                        .invoke(boot_child),
                ),
        )
        .start()
        .await
        .unwrap()
        .stop()
        .await
        .unwrap();
}

#[tokio::test]
async fn parent_invoke_cannot_use_child_private_provider() {
    #[derive(Clone)]
    struct ChildSecret;

    fn new_child_secret() -> ChildSecret {
        ChildSecret
    }
    fn needs_secret(_secret: ChildSecret) {}

    let err = Modrun::builder()
        .module(
            Module::builder("parent")
                .module(Module::builder("child").provide_private(new_child_secret))
                .invoke(needs_secret),
        )
        .start()
        .await
        .unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.contains("missing") || msg.contains("ChildSecret"),
        "unexpected: {msg}"
    );
}
