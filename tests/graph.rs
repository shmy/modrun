//! Dependency graph DOT export tests.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use modrun::{Group, Modrun, Module};

thread_local! {
    static CTOR_RAN: AtomicBool = const { AtomicBool::new(false) };
    static DOT_PATH: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

fn node_ids_with_label(dot: &str, needle: &str) -> Vec<String> {
    dot.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            let rest = trimmed.strip_prefix('n')?;
            let id = rest.split([' ', '[']).next()?;
            trimmed.contains(needle).then(|| format!("n{id}"))
        })
        .collect()
}

fn has_edge(dot: &str, from: &str, to: &str) -> bool {
    dot.lines()
        .any(|line| line.contains(&format!("{from} -> {to}")))
}

#[derive(Clone)]
struct Config;

#[derive(Clone)]
struct Service;

fn new_config() -> Config {
    Config
}

fn new_service(_cfg: Config) -> Service {
    Service
}

fn boot(_svc: Service) {}

#[test]
fn render_dot_simple_chain() {
    let dot = Modrun::builder()
        .no_banner()
        .provide(new_config)
        .provide(new_service)
        .invoke(boot)
        .render_dot()
        .unwrap();

    assert!(dot.starts_with("digraph modrun"));
    assert!(dot.contains(r#"label="Config\nctor=new_config""#));
    assert!(dot.contains(r#"label="Service\nctor=new_service""#));

    let config = node_ids_with_label(&dot, r"Config\nctor=new_config");
    let service = node_ids_with_label(&dot, r"Service\nctor=new_service");
    let boot = node_ids_with_label(&dot, r"\n(invoker)");
    assert_eq!(config.len(), 1);
    assert_eq!(service.len(), 1);
    assert!(!boot.is_empty());
    assert!(has_edge(&dot, &config[0], &service[0]));
    assert!(has_edge(&dot, &service[0], &boot[0]));
    assert!(
        !dot.contains("Lifecycle") && !dot.contains("Shutdowner"),
        "framework nodes should be omitted: {dot}"
    );
}

#[test]
fn render_dot_module_private_nodes() {
    #[derive(Clone)]
    struct Repo;
    #[derive(Clone)]
    struct Svc;

    fn new_repo() -> Repo {
        Repo
    }
    fn new_svc(_repo: Repo) -> Svc {
        Svc
    }
    fn boot_user(_svc: Svc) {}

    let dot = Modrun::builder()
        .no_banner()
        .module(
            Module::builder("user")
                .provide_private(new_repo)
                .provide(new_svc)
                .invoke(boot_user),
        )
        .render_dot()
        .unwrap();

    assert!(dot.contains("subgraph cluster_"));
    assert!(dot.contains("label=\"user\";"));
    assert!(dot.contains(r"Repo\nctor=new_repo\n(private)"));
    assert!(dot.contains(r"Svc\nctor=new_svc"));
    assert!(dot.contains(r"boot_user\n(invoker)"));

    let repo = node_ids_with_label(&dot, r"Repo\nctor=new_repo\n(private)");
    let svc = node_ids_with_label(&dot, r"Svc\nctor=new_svc");
    assert_eq!(repo.len(), 1);
    assert_eq!(svc.len(), 1);
    assert!(has_edge(&dot, &repo[0], &svc[0]));
}

#[test]
fn render_dot_does_not_run_constructors() {
    #[derive(Clone)]
    struct Boom;

    fn boom_ctor() -> Boom {
        CTOR_RAN.with(|ran| ran.store(true, Ordering::SeqCst));
        Boom
    }

    CTOR_RAN.with(|ran| ran.store(false, Ordering::SeqCst));
    let dot = Modrun::builder()
        .no_banner()
        .provide(boom_ctor)
        .invoke(|_: Boom| {})
        .render_dot()
        .unwrap();

    assert!(dot.contains("Boom"));
    assert!(!CTOR_RAN.with(|ran| ran.load(Ordering::SeqCst)));
}

#[test]
fn render_dot_group_membership_edges() {
    #[derive(Clone)]
    #[allow(dead_code)]
    struct Route(&'static str);

    fn user_route() -> Route {
        Route("user")
    }
    fn order_route() -> Route {
        Route("order")
    }
    fn boot(_routes: Group<Route>) {}

    let dot = Modrun::builder()
        .no_banner()
        .module(Module::builder("user").provide_group(user_route))
        .module(Module::builder("order").provide_group(order_route))
        .invoke(boot)
        .render_dot()
        .unwrap();

    assert!(dot.contains("Group<"));
    assert!(dot.contains("style=dotted"));
    assert!(dot.contains("user_route"));
    assert!(dot.contains("order_route"));

    let group = node_ids_with_label(&dot, "group aggregate");
    let user = node_ids_with_label(&dot, "user_route");
    assert_eq!(group.len(), 1);
    assert_eq!(user.len(), 1);
    assert!(
        dot.lines().any(|line| {
            line.contains("style=dotted") && line.contains(&group[0]) && line.contains(&user[0])
        }),
        "expected dotted edge from member to group: {dot}"
    );
}

#[test]
fn render_dot_supply_and_arc_dependency() {
    #[derive(Clone)]
    struct Config {
        port: u16,
    }

    #[derive(Clone)]
    struct Service;

    fn new_service(cfg: Arc<Config>) -> Service {
        assert_eq!(cfg.port, 8080);
        Service
    }

    let dot = Modrun::builder()
        .no_banner()
        .supply(Config { port: 8080 })
        .provide(new_service)
        .invoke(|_: Service| {})
        .render_dot()
        .unwrap();

    assert!(dot.contains(r"Config\n(supplied)"));
    assert!(dot.contains("Arc<"));
    assert!(dot.contains(r"Service\nctor=new_service"));

    let arc_config = node_ids_with_label(&dot, "Arc<graph::");
    let service = node_ids_with_label(&dot, r"Service\nctor=new_service");
    assert_eq!(arc_config.len(), 1, "dot:\n{dot}");
    assert_eq!(service.len(), 1);
    assert!(has_edge(&dot, &arc_config[0], &service[0]));
}

#[test]
fn render_dot_module_public_supply_edge() {
    #[derive(Clone)]
    struct Settings {
        port: u16,
    }

    #[derive(Clone)]
    struct Service;

    fn new_service(cfg: Settings) -> Service {
        assert_eq!(cfg.port, 9000);
        Service
    }

    let dot = Modrun::builder()
        .no_banner()
        .module(
            Module::builder("settings")
                .supply(Settings { port: 9000 })
                .provide(new_service),
        )
        .invoke(|_: Service| {})
        .render_dot()
        .unwrap();

    assert!(dot.contains("label=\"settings\";"));
    assert!(dot.contains(r"Settings\n(supplied)"));

    let settings = node_ids_with_label(&dot, r"Settings\n(supplied)");
    let service = node_ids_with_label(&dot, r"Service\nctor=new_service");
    assert_eq!(settings.len(), 1, "dot:\n{dot}");
    assert_eq!(service.len(), 1);
    assert!(has_edge(&dot, &settings[0], &service[0]));
}

#[test]
fn render_dot_module_private_supply_edge() {
    #[derive(Clone)]
    struct Token;

    #[derive(Clone)]
    struct Service;

    fn new_service(token: Token) -> Service {
        let _ = token;
        Service
    }

    let dot = Modrun::builder()
        .no_banner()
        .module(
            Module::builder("auth")
                .supply_private(Token)
                .provide(new_service),
        )
        .invoke(|_: Service| {})
        .render_dot()
        .unwrap();

    assert!(dot.contains("label=\"auth\";"));
    assert!(dot.contains(r"Token\n(supplied)"));

    let token = node_ids_with_label(&dot, r"Token\n(supplied)");
    let service = node_ids_with_label(&dot, r"Service\nctor=new_service");
    assert_eq!(token.len(), 1, "dot:\n{dot}");
    assert_eq!(service.len(), 1);
    assert!(has_edge(&dot, &token[0], &service[0]));
}

#[test]
fn render_dot_provide_private_with_arc_dependency() {
    #[derive(Clone)]
    struct Config;
    #[derive(Clone)]
    struct Service(#[allow(dead_code)] Arc<Config>);

    fn new_config() -> Config {
        Config
    }
    fn new_service(config: Arc<Config>) -> Service {
        Service(config)
    }

    let dot = Modrun::builder()
        .no_banner()
        .module(
            Module::builder("domain")
                .provide_private(new_config)
                .provide(new_service),
        )
        .invoke(|_: Service| {})
        .render_dot()
        .unwrap();

    let config = node_ids_with_label(&dot, r"Config\nctor=new_config\n(private)");
    let service = node_ids_with_label(&dot, r"Service\nctor=new_service");
    assert_eq!(config.len(), 1);
    assert_eq!(service.len(), 1);
    assert!(has_edge(&dot, &config[0], &service[0]));
}

#[test]
fn render_dot_escapes_special_module_names() {
    #[derive(Clone)]
    struct Widget;

    fn make_widget() -> Widget {
        Widget
    }

    let dot = Modrun::builder()
        .no_banner()
        .module(Module::builder("we\"ird").provide(make_widget))
        .render_dot()
        .unwrap();

    assert!(dot.contains(r#"label="we\"ird";"#));
    assert!(dot.contains(r"Widget\nctor=make_widget"));
}

#[tokio::test]
async fn dot_graph_writes_file_before_constructors_run() {
    #[derive(Clone)]
    struct Boom;

    fn boom_ctor() -> Boom {
        let path = DOT_PATH.with(|slot| slot.borrow().clone().expect("path set"));
        let dot =
            std::fs::read_to_string(&path).expect("dot must be written before constructor runs");
        assert!(dot.contains("digraph modrun"));
        assert!(dot.contains("Boom"));
        CTOR_RAN.with(|ran| ran.store(true, Ordering::SeqCst));
        Boom
    }

    let dir = std::env::temp_dir().join(format!("modrun-dot-before-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("graph.dot");
    DOT_PATH.with(|slot| *slot.borrow_mut() = Some(path.clone()));

    CTOR_RAN.with(|ran| ran.store(false, Ordering::SeqCst));

    let app = Modrun::builder()
        .no_banner()
        .dot_graph(&path)
        .provide(boom_ctor)
        .invoke(|_: Boom| {})
        .start()
        .await
        .unwrap();

    assert!(CTOR_RAN.with(|ran| ran.load(Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(dir);
    app.stop().await.unwrap();
}

#[test]
fn render_dot_distinct_clusters_for_dash_and_underscore_modules() {
    #[derive(Clone)]
    struct A;
    #[derive(Clone)]
    struct B;

    fn make_a() -> A {
        A
    }
    fn make_b() -> B {
        B
    }

    let dot = Modrun::builder()
        .no_banner()
        .module(Module::builder("a-b").provide(make_a))
        .module(Module::builder("a_b").provide(make_b))
        .render_dot()
        .unwrap();

    let cluster_lines: Vec<_> = dot
        .lines()
        .filter(|line| line.contains("subgraph cluster_"))
        .collect();
    assert_eq!(cluster_lines.len(), 2, "expected two clusters: {dot}");
    assert!(dot.contains("label=\"a-b\";"));
    assert!(dot.contains("label=\"a_b\";"));
}

#[test]
fn render_dot_reports_cycle_as_error() {
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
        .no_banner()
        .provide(new_a)
        .provide(new_b)
        .render_dot()
        .unwrap_err();

    let msg = err.to_string();
    assert!(msg.contains("cycle"), "unexpected: {msg}");
}

#[test]
fn render_dot_reports_missing_provider_as_error() {
    #[derive(Clone)]
    struct NeedsConfig;

    fn needs(_cfg: Config) -> NeedsConfig {
        NeedsConfig
    }

    let err = Modrun::builder()
        .no_banner()
        .provide(needs)
        .render_dot()
        .unwrap_err();

    let msg = err.to_string();
    assert!(
        msg.contains("needs a dependency nothing provides"),
        "unexpected: {msg}"
    );
}

#[tokio::test]
async fn dot_graph_writes_file() {
    #[derive(Clone)]
    struct Boom;

    fn boom_ctor() -> Boom {
        Boom
    }

    let dir = std::env::temp_dir().join(format!("modrun-dot-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("graph.dot");

    let app = Modrun::builder()
        .no_banner()
        .dot_graph(&path)
        .provide(boom_ctor)
        .invoke(|_: Boom| {})
        .start()
        .await
        .unwrap();

    assert!(path.is_file(), "dot file missing at {}", path.display());
    let dot = std::fs::read_to_string(&path).unwrap();
    assert!(dot.contains("digraph modrun"));
    assert!(dot.contains("Boom"));
    let _ = std::fs::remove_dir_all(dir);
    app.stop().await.unwrap();
}

#[test]
fn dot_graph_write_failure_surfaces_io_error() {
    let dir = std::env::temp_dir().join(format!("modrun-dot-fail-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();

    let err = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async {
            Modrun::builder()
                .no_banner()
                .dot_graph(&dir)
                .provide(|| 1u32)
                .invoke(|_: u32| {})
                .start()
                .await
        })
        .unwrap_err();

    let msg = err.to_string();
    assert!(msg.contains("write dot graph"), "unexpected: {msg}");
    let _ = std::fs::remove_dir_all(dir);
}
