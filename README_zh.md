# modrun

[English](README.md) | 简体中文

**面向 Rust 的模块化应用组合器。**

把大型 Tokio 服务拆成面向领域的模块：构造函数注入、显式模块边界、统一生命周期编排——在组合根一次性接线。

需要 **Rust 1.85** 或更高版本（edition 2024）。这不是通用 DI 容器：没有字符串 qualifier、没有请求级对象、没有注解、没有自动扫描，图构建完成之后也不能 `get<T>()`。两个同类依赖用 newtype 区分；测试替身在组合根用 [`supply`](#概念) 替换。**自 1.0.0 起 API 稳定** — 稳定性承诺与 semver 规则见 [CONTRIBUTING.md](CONTRIBUTING.md)。

如果你写过这样的 `main`——先造 config，再造连接池，再造 repo，再造 service，再造 server，停机时还得按相反顺序拆掉——modrun 就是把这段 `main` 写一次。

```rust,no_run
use modrun::{Hook, Lifecycle, Modrun};

#[derive(Clone)]
struct Config {
    port: u16,
}

#[derive(Clone)]
struct Server {
    cfg: Config,
}

impl Hook for Server {
    async fn on_start(&mut self) -> modrun::Result<()> {
        println!("listening on {}", self.cfg.port);
        Ok(())
    }

    async fn on_stop(&mut self) -> modrun::Result<()> {
        println!("goodbye");
        Ok(())
    }
}

fn new_config() -> Config {
    Config { port: 8080 }
}

fn new_server(cfg: Config) -> Server {
    Server { cfg }
}

fn boot(lc: Lifecycle, server: Server) -> modrun::Result<()> {
    lc.append(server)
}

#[tokio::main]
async fn main() -> modrun::Result<()> {
    Modrun::builder()
        .provide(new_config)
        .provide(new_server)
        .invoke(boot)
        .run()
        .await
}
```

`run()` 会构建依赖图、按序跑完所有 OnStart、等待操作系统信号或
[`Shutdowner`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html)，再按相反顺序跑 OnStop。
默认开启 `signal` feature 时，监听器在 `run()` 一开始就安装：**Unix** 上是 Ctrl-C / SIGTERM，**Windows** 上是 Ctrl-C / Ctrl-Break / Ctrl-Close / Ctrl-Shutdown；其他目标上只有
[`Shutdowner`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html)
能解开 `run()`。构建或启动阶段收到停机请求会取消当前阶段，并 unwind 已经启动过的 hook，以及已经注册的 stop-only hook（即使 OnStart 从未跑过）。超时或 hook 失败仍返回错误；并发的 shutdown 不会把失败变成 `Ok(())`。
如果只用 `start()`，或自己在 [`Shutdowner::wait`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html#method.wait) 上等待，可以关掉默认的 `signal` feature。

## 概念

### 五个接线动词

大多数应用只需要五个注册概念。按函数签名（sync / async / 可失败）选方法——见下方 [API 矩阵](#api-矩阵)。

**入口**（两者都是 fluent wiring builder，不是 runtime service locator）：

| 层级 | 起点 | 之后 |
|------|------|------|
| 组合根 | `Modrun::builder()` | `.module(...)`、`.invoke(boot)`、`.run()` / `.start()` |
| 领域模块 | `Module::builder("name")` | `.provide*`、`.invoke(...)`，经 `.module(...)` 挂到根 |

| 动词 | 作用 |
|------|------|
| [`provide`](#provide-变体) | 注册构造函数；依赖来自函数参数 |
| [`supply`](#supply) | 注入已有值，跳过构造函数 |
| [`invoke`](#invoke) | 拉取依赖图；invoker 参数决定哪些类型会被构造 |
| [`module`](#模块) | 把领域装配收在命名模块与私有作用域下 |
| [`provide_group`](#值组groups) | 向 [`Group<T>`](https://docs.rs/modrun/latest/modrun/struct.Group.html) 贡献一个成员 |

### API 矩阵

应用代码如何选注册方法（不含 `*_dyn`）：

| | Sync | `Result` | Async | Async `Result` |
|---|------|----------|-------|----------------|
| **单例** [`provide`](#provide) | `provide` | `provide_result` | `provide_async` | `provide_result_async` |
| **单例，模块私有** | `provide_private` | `provide_result_private` | `provide_async_private` | `provide_result_async_private` |
| **组成员** [`provide_group`](#值组groups) | `provide_group` | `provide_group_result` | `provide_group_async` | `provide_group_result_async` |
| **预构建值** | `supply` / `supply_private` | — | — | — |
| **图根** [`invoke`](#invoke) | `invoke` *(也可返回 `Result`)* | *(同上)* | `invoke_async` | *(同上)* |

[`invoke`](#invoke) **没有** `_result` 后缀：可失败 invoker 直接从 `invoke` / `invoke_async` 返回 `Result<(), E>`。只有构造函数在编译期区分 `provide` 与 `provide_result`。

仅 [`ModrunBuilder`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html)：
[`init_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.init_group)、
[`require_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.require_group)、
[`supply_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.supply_group)。
只要至少有一个模块 `provide_group`，依赖 `Group<T>` 的 invoker 通常**不必**调用 `init_group`；
`init_group` / `require_group` 仅用于空组，或必须在构建期拒绝「零成员」时。

### 三种常见模式

大多数应用是下面三种之一（均有示例）：

| 模式 | 场景 | 示例 |
|------|------|------|
| **领域模块** | 隐藏内部类型，只暴露服务 | [`basic`](examples/basic.rs) — `provide_private` + 公开 `provide` + `invoke` |
| **值组 Group** | 多模块贡献同类型元素 | [`handlers`](examples/handlers.rs) — `provide_group` → 根上注入 `Group<T>` |
| **Wrapper** | 指标、命名、超时（无 `decorate` API） | [`wrap`](examples/wrap.rs) — 单 ctor 组合，或 `provide_private` + newtype |

**领域模块**：

```text
fn user_domain() -> Module {
    Module::builder("user")
        .provide_private(new_user_repo)
        .provide(new_user_service)
        .invoke(register_user_hooks)
}

Modrun::builder().module(user_domain()).invoke(boot).run().await
```

**Group** — 插件、路由、handler：

```text
Modrun::builder()
    .module(Module::builder("user").provide_group(user_routes))
    .module(Module::builder("order").provide_group(order_routes))
    .invoke(|routes: Group<Route>| mount(routes))
```

**Wrapper** — 见 [横切关注点](#横切关注点wrapper-构造函数) 与 [`wrap.rs`](examples/wrap.rs)。

可替换依赖与测试替身放在 **组合根** 的 [`supply`](#supply)，不要放进模块——见 [`swap`](examples/swap.rs)。

### Provide 变体

按构造函数签名选方法——不是新的 DI 概念：

| 方法 | 构造函数 |
|------|----------|
| `provide` | `fn(...) -> T` |
| `provide_result` | `fn(...) -> Result<T, E>` |
| `provide_async` | `async fn(...) -> T` |
| `provide_result_async` | `async fn(...) -> Result<T, E>` |

返回 `Result` 的构造函数交给普通 `provide` 会 **编译失败**（编译器会指向 `provide_result`）。组成员用 `provide_group_*` 前缀，同样四选一。类型擦除的 `provide_dyn` / `invoke_dyn` / `provide_group_dyn` 在 [`modrun::__wiring`](https://docs.rs/modrun/latest/modrun/__wiring/index.html) 下，仅供 wrapper 库使用（`#[doc(hidden)]`，不出现在应用文档里）。

### 横切关注点（wrapper 构造函数）

modrun **不会** 增加 `decorate` API。横切逻辑用普通 Rust 函数表达。每种类型只能 `provide` 一次——在 **一个** 构造函数里组合 wrapper，或在模块里返回 **newtype / 服务结构体**（`provide_private` 放原始值，公开 `provide` 返回包装后的不同类型）：

```rust
use modrun::{Modrun, Module};

#[derive(Clone, PartialEq, Eq)]
struct Logger { name: &'static str }

#[derive(Clone, PartialEq, Eq)]
struct AppLogger(Logger); // 公开 binding ≠ 私有原始类型

fn new_logger() -> Logger { Logger { name: "default" } }
fn named(log: Logger) -> Logger { /* ... */ log }
fn new_app_logger(log: Logger) -> AppLogger { AppLogger(named(log)) }

fn logging() -> Module {
    Module::builder("logging")
        .provide_private(new_logger)
        .provide(new_app_logger)
        .invoke(|log: AppLogger| { /* ... */ })
}

Modrun::builder().module(logging());
```

组合根只需一个合成构造函数：`fn app_logger() -> Logger { ... }` 即可。详见 [`examples/wrap.rs`](examples/wrap.rs)。

### Provide

**`provide`** 注册构造函数。没有人要这个类型之前不会构造，每种类型最多构造一次。返回 `Result<T, E>` 的构造函数必须用 `provide_result`（或 `provide_result_async`）；交给普通 `provide` 是编译错误。`provide_async` 和 `provide_result_async` 接受 `async fn`。
若干彼此独立的构造函数需要同时出现时，modrun 按 DAG 层构建：**同一层的 async** 构造函数在一个任务上并发 poll；**sync** 构造函数在 `construct()` 里跑完，并推迟该层后续 future 的创建。共享依赖仍然先跑，再按依赖顺序继续。

### Supply

**`supply`** 把已经有的值交给容器，跳过构造函数。

### Invoke

**`invoke`** 拉取依赖图。invoker 的参数是构建的根：没有人直接或间接 invoke 的类型永远不会被构造。
只 `provide`、没有被 invoke 到的迁移任务或后台消费者会一直不跑，直到有东西 `invoke` 它（或依赖它的类型）。Invoker 在 build 期间只跑一次，通常也是注册生命周期 hook 的地方。

**`Lifecycle`** 收集 start/stop hook。它会自动注入，任何构造函数或 invoker 都可以把它当参数。OnStart 按注册顺序执行；OnStop 按相反顺序。某个 start hook 失败或 start 被取消时，已经跑完 OnStart 的 hook 会先被 stop，再把错误向上传——OnStop 的失败会留在返回的错误里。正在失败或被取消的那个 start hook **不会**跑自己的 OnStop。没有 OnStart 的 stop-only hook 一经注册就视为已激活，会参与 unwind。
在 `invoke` 里（或 OnStart 工厂里）注册 hook；start 已经结束或 stop 已经开始时，`append` 会返回错误。Invoker 本身可以返回 `modrun::Result<()>`，所以给 `boot` 这个返回类型，把 `append` 的结果直接往回传，而不要 `unwrap`。
start 和 stop 共享状态（`&mut self`）时，在结构体上实现 [`Hook`](https://docs.rs/modrun/latest/modrun/trait.Hook.html)。只实现 OnStop 的结构体要覆盖 [`has_start`](https://docs.rs/modrun/latest/modrun/trait.Hook.html#method.has_start)
为 `false`，这样失败 start 之后的 trailing activation 仍会跑它。
一次性闭包用 [`hook()`](https://docs.rs/modrun/latest/modrun/fn.hook.html)；捕获共享状态的 OnStop 回调必须可重复调用（`Fn`）——每次调用时在闭包里 clone 一份 [`Arc`](std::sync::Arc)。
Hook 和构造函数错误使用 [`Error`](https://docs.rs/modrun/latest/modrun/enum.Error.html)（`thiserror`）；hook 应返回
[`Error::hook`](https://docs.rs/modrun/latest/modrun/enum.Error.html#method.hook) 或
[`Error::io`](https://docs.rs/modrun/latest/modrun/enum.Error.html#method.io)，这样原始失败会留在 [`std::error::Error::source`](std::error::Error::source) 上。没有 `From<std::io::Error>`，I/O 要用 [`Error::io`](https://docs.rs/modrun/latest/modrun/enum.Error.html#method.io) 包一层（`bind(addr).await.map_err(|e| Error::io(format!("bind {addr}"), e))?`）。

用 [`Hook::name`](https://docs.rs/modrun/latest/modrun/trait.Hook.html#method.name)
覆盖默认的 `"unnamed"`，让日志和错误信息更清晰。构造函数和 invoker 最多八个参数；多出来的依赖收进一个结构体，不要把参数个数拉长。

Hook 的 future 必须是 cancellation-safe 的。start/stop 超时会 drop 正在进行的 future，但 **不能** 取消 `tokio::spawn` 出去的脱离任务。Worker 用 [`task()`](https://docs.rs/modrun/latest/modrun/fn.task.html)；bind/listen 必须在 OnStart 里完成时用 [`task_with()`](https://docs.rs/modrun/latest/modrun/fn.task_with.html)（见 axum 示例）。两者都会在 OnStop 时打出 [`Stopped`](https://docs.rs/modrun/latest/modrun/struct.Stopped.html)、join，并在 hook 中途被 drop 时 abort。后台工作在 start 已经成功之后返回 `Err` 或 panic 时会自动请求 shutdown，这样 `run()` 不会一直等信号。用 `tokio::spawn` 自己拉起的任务仍须调用
[`Shutdowner::shutdown`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html#method.shutdown)。
Hook 里 panic 视为致命的编程错误，可能跳过生命周期 unwind（日志为 `panicked`）。

**`Shutdowner`** 同样自动注入。在应用内部调用 `shutdown()` 会解开 `run()`，用来响应信号以外的停机原因。构建/启动阶段的取消同样是协作式的（下一个 `.await`）。

容器在 build 结束时被 drop。单例只通过你在 hook 里捕获的值（或 invoke 时拿走的其他 `Clone` 句柄）活下来。modrun 负责启动装配，不是活的 service locator。Build 不是事务：后面的 invoker 失败时，前面的构造函数可能已经产生了副作用。

按值注入要求依赖 `Clone`，因为单例按类型缓存，注入时 clone。类型很大、被很多构造函数共享、或会被注入多次时，优先 `Arc<T>`——`Arc<T>` 会登记为别名，返回 `T` 的构造函数也可以注入 `Arc<T>`，此时不要求 `T: Clone`。可变共享状态放在类型内部的 `Arc` 后面：

```rust
use std::sync::Arc;

struct DbInner;

#[derive(Clone)]
struct Db(Arc<DbInner>);
```

缓存键是类型，所以一种类型只能 provide 一次。要装配两份同类的东西（例如主库和从库连接池），各自包一层 newtype：

```rust
# use std::sync::Arc;
# struct PgPool;
#[derive(Clone)]
struct PrimaryDb(Arc<PgPool>);
#[derive(Clone)]
struct ReplicaDb(Arc<PgPool>);
# let _ = std::any::type_name::<(PrimaryDb, ReplicaDb)>();
```

连接池用 `provide_result_async` 放在组合根（这样测试才能 `supply` 假实现）。不要在 OnStart 里建连，除非你希望连接失败看起来像 start-hook 失败，而不是构造函数错误。

## 性能

modrun 的开销几乎全在 **冷启动**：校验图、一次性跑完构造函数与 invoker、再跑 lifecycle hook。`start()` 之后没有 service locator，也没有后台接线——容器在 build 结束时会被 drop。

和连数据库、绑端口、读配置相比，框架本身通常可以忽略。建议：

* **大对象或广泛共享的单例用 `Arc<T>`** — 按值注入会 clone 缓存；`Arc<T>` 只增引用计数（见上文）。
* **Group 成员较重时用 `Arc<Group<T>>` 或 `Group<Arc<T>>`** — 聚合 `Group<T>` 可能逐个 clone 成员（要求 `T: Clone`）。
* **模块嵌套尽量浅** — 私有绑定解析会沿 scope 祖先链查找（通常几次 HashMap 查找）。
* **生产环境关闭框架日志** — `default-features = false` 不装可选 subscriber；无 subscriber 时 `tracing` 事件几乎无成本。测试与 benchmark 用 [`.no_banner()`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.no_banner)。
* **不要在每次启动都开 `dot_graph()`** — DOT 会校验并渲染整图；仅调试或 CI 使用。

同一 wave 内独立的 async 构造函数会在当前 task 上并发 poll（不会 `tokio::spawn` 风暴）。本地可跑 `cargo bench --bench build` 对比 group 规模与 module 深度。

## 模块

`Module` 把相关装配收在一个名字下，并给出私有作用域。
`provide_private` 让类型在模块外不可见，两个领域就可以各自有一份 `Repo` 而不冲突：

```rust
use modrun::{Modrun, Module};

# #[derive(Clone)] struct UserRepo;
# #[derive(Clone)] struct UserService;
# fn new_user_repo() -> UserRepo { UserRepo }
# fn new_user_service(_r: UserRepo) -> UserService { UserService }
# fn boot_user(_s: UserService) {}
fn user_domain() -> Module {
    Module::builder("user")
        .provide_private(new_user_repo)
        .provide(new_user_service)
        .invoke(boot_user)
}

# #[tokio::main]
# async fn main() -> modrun::Result<()> {
Modrun::builder()
    .module(user_domain())
    .start()
    .await?
    .stop()
    .await
# }
```

私有类型对声明它的模块及其嵌套模块可见。
用普通 `provide` / `supply` 注册的类型在任何地方都可见，不论在哪声明。
`provide_private` / `supply_private` 只存在于 [`Module`](https://docs.rs/modrun/latest/modrun/struct.Module.html) 上，根 builder 没有。

## 值组（Groups）

多个模块可以各自贡献同一个类型的实例；消费者在注入点拿到的是
[`Group<T>`](https://docs.rs/modrun/latest/modrun/struct.Group.html)（不是 `Vec<T>`）。
用 [`provide_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.provide_group)
注册成员（需要时用 `provide_group_async` / `provide_group_result` /
`provide_group_result_async`）。成员按注册顺序聚合，**不占用** `T` 的单例槽位，
因此可以和同类型的 `provide` 单例并存。

```rust
use modrun::{Group, Modrun, Module};

# #[derive(Clone, PartialEq, Eq)] struct Handler(&'static str);
# fn user_handler() -> Handler { Handler("user") }
# fn order_handler() -> Handler { Handler("order") }
# fn boot(_: Group<Handler>) {}

Modrun::builder()
    .module(Module::builder("user").provide_group(user_handler))
    .module(Module::builder("order").provide_group(order_handler))
    .invoke(boot)
# ;
```

在 invoker 或构造函数里注入 `Group<T>`（或 `Arc<Group<T>>`），用 `for item in group` 遍历。
组成员与注入的组都要求 `T: Clone`；多个消费者共享同一集合时优先用 `Arc<Group<T>>`，
值较重时让组成员构造函数返回 `Arc<T>` / `Arc<dyn Trait>`。
已有模块 `provide_group` 时，组合根上依赖 `Group<T>` 的 invoker 会自动聚合成员，无需
`init_group`（见 [`handlers`](examples/handlers.rs)）。
**没有任何成员**时，用
[`init_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.init_group)
或
[`require_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.require_group)
注册空组（`require_group` 还会在组仍为空时让构建失败，且只能由组合根调用）。
`T` 由构造函数返回类型推断（没有 `provide_group::<T>` turbofish）；
trait object 组让构造函数返回 `Arc<dyn Trait>`。模块内
[`provide_private`](https://docs.rs/modrun/latest/modrun/struct.Module.html#method.provide_private)
的 `Group<T>` 会在该模块内遮蔽聚合结果——贡献成员请用
[`provide_group`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.provide_group)。
同一元素类型要分多组时，用 newtype（与单例重复时一样）。

**方法矩阵**（组成员与上面四种 `provide_*` 变体对称）：

| 注册 | 场景 |
|------|------|
| `provide_group` | 同步、不可失败 |
| `provide_group_result` | 同步、`Result` |
| `provide_group_async` | 异步、不可失败 |
| `provide_group_result_async` | 异步、`Result` |
| `supply_group` | 已有实例 |
| `init_group` / `require_group` | 仅组合根；空组 / 非空策略 |

## 依赖图

将接线图导出为 [Graphviz DOT](https://graphviz.org/doc/info/lang.html)，便于文档或排查问题。
会先执行与构建相同的校验（缺失 provider、循环依赖会报错），但不会运行任何构造函数或 invoker。

```rust
use modrun::Modrun;

# #[derive(Clone)] struct Config;
# #[derive(Clone)] struct Server;
# fn new_config() -> Config { Config }
# fn new_server(_: Config) -> Server { Server }
# fn boot(_: Server) {}

// 返回 DOT 字符串（不写文件）
let dot = Modrun::builder()
    .provide(new_config)
    .provide(new_server)
    .invoke(boot)
    .render_dot()?;
# Ok::<(), modrun::Error>(())
```

启动应用前写出文件时，在传给 [`run`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.run) 的 builder 上链式调用
[`.dot_graph("modrun.dot")`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.dot_graph)。

节点标注类型、构造函数名与模块作用域（每个 module 一个子图）。实线箭头表示构造 / invoker 依赖；点线连接组成员与其 `Group<T>` 聚合节点。内置的 `Lifecycle`、`Shutdowner` 节点会被省略以保持图面简洁。示例输出见 [docs/graph-sample.dot](docs/graph-sample.dot)。用 `dot -Tpng modrun.dot -o modrun.png` 渲染。

## 日志

框架事件（provide / supply / invoke / construct / OnStart / OnStop）通过 [`tracing`](https://docs.rs/tracing) 发出，target 为 `modrun`，控制台行是 [uber/fx](https://github.com/uber-go/fx) 风格，例如
`[modrun] PROVIDE    my::Type <= my::new`。同一批事件还带结构化字段（`constructor`、`module`、`elapsed`、`error` 等），生产环境的 JSON subscriber 可以按字段过滤。

[`modrun::logging::init()`](https://docs.rs/modrun/latest/modrun/logging/fn.init.html)
给示例和本地二进制用（默认 feature `logging`）。日志打到 stderr，仅在 stderr 是 TTY 时开 ANSI；如果已经安装了 subscriber，它是 **no-op**，不会 panic。生产服务应自己装 subscriber，跳过这个助手。没有 subscriber 时，这些事件是廉价空操作：

```rust,no_run
fn main() {
    #[cfg(feature = "logging")]
    modrun::logging::init();
}
```

自带 subscriber 时设 `RUST_LOG=modrun=info`。被取消的 hook 和构造函数打 `ERROR`。成功 stop 打 `STOPPED`。`RunningApp` 泄漏警告走 tracing；debug 构建还会打到 stderr。

## 启动 Banner

[`ModrunBuilder::run`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.run) 和
[`start`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.start) 在装配开始前会往 stderr 打一份 modrun ASCII banner（Spring Boot 风格），且仅当 stderr 是 TTY。自定义文本（或在自己的 crate 里 `include_str!("banner.txt")`）总会打到 stderr：

```rust,no_run
# use modrun::Modrun;
Modrun::builder()
    .banner("my service")
    // ...
# ;
```

用 [`.no_banner()`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.no_banner) 关掉。
管道和没有 TTY 的守护进程会自动跳过默认 banner。在终端里跑的测试仍应调用 `.no_banner()`，避免捕获到的 stderr 被 ASCII 画弄乱。

## 失败模式

图会在任何东西被构造之前检查，所以这些是构建期错误，而不是跑到一半才发现：

* 某个 provider 依赖了没有任何东西提供的类型，即使没人用这个 provider
* 依赖环
* 同一类型提供了两次

运行时，`build_timeout`、`start_timeout`、`stop_timeout`（默认 15s）分别限制图构建、OnStart、OnStop。超时是协作式的：在 `.await` 上让出的工作会在预算耗尽时被取消。
同步阻塞（例如 sync invoker、构造函数或 hook 里的 `std::thread::sleep`）无法被抢占，但超时后的成功仍会报成超时错误而不是 `Ok`。取消计时跟 Tokio 时钟；超时后的 `Ok` 检查用墙钟 `Instant`，测试里 `tokio::time::pause` 可能让两者不一致。同一项超时设多次，以最后一次为准。`no_build_timeout` / `no_start_timeout` / `no_stop_timeout` 关掉预算。`no_start_timeout` 只关掉 OnStart；失败或取消的 start 之后的 unwind 仍受 `stop_timeout` 限制。预算耗尽时，剩余 OnStop 会被放弃，并以错误上报而不是挂死——连接池可能来不及干净关闭。
生产启动若要跑迁移或预热缓存，请显式设置 `start_timeout`（以及 `stop_timeout`），不要依赖默认的 15s。

`run()` 把构建或启动阶段的 Ctrl-C / SIGTERM / [`Shutdowner`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html)
当成优雅退出：unwind 已经启动的 hook 以及已注册的 stop-only hook，清理成功则返回 `Ok(())`。
后台 [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) 在 start 阶段失败或 panic **不算**优雅退出——`run()` 返回 join 错误（若 unwind 上报成功，则为 `background task failed during start`）。
若随后 unwind 超时，两者都留在 [`Error::CleanupAfterFailure`](https://docs.rs/modrun/latest/modrun/enum.Error.html) 上。
若该阶段已经失败，`run()` 仍返回那次失败。
Shutdown 和 OS 信号与超时一样是协作式的：在下一个 `.await` 才生效，所以同步 OnStart 里调用 `shutdown()` 不会跳过后面尚未让出的 hook。进入 `RUNNING` 之后，`run()` 会等到信号或 `Shutdowner::shutdown()`；后台 [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) 失败或 panic 会自动请求 shutdown。用 `tokio::spawn` 自己拉起的任务仍须调用 `shutdown()`，否则会一直等。构造函数、invoker 或 hook 里的 panic 不会变成 [`Error`](https://docs.rs/modrun/latest/modrun/enum.Error.html)，并可能跳过生命周期 unwind（tracing 记为 `panicked`）。

## 测试

`start()` 构建并启动，不等待信号，返回一个你可以自己 `stop()` 的 `RunningApp`。后台 [`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) 在 OnStart 返回之后失败时，**不会**让 `start()` 失败，也不会跳过后续 hook；在 [`Shutdowner`](https://docs.rs/modrun/latest/modrun/struct.Shutdowner.html) 上等待或调用 `stop()` 才能看到。需要失败 worker 拆掉整个进程时用 [`run`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.run)：

```rust
# use std::sync::Arc;
# use std::sync::atomic::{AtomicUsize, Ordering};
# use modrun::{hook, Lifecycle, Modrun};
# #[derive(Clone)] struct Hits(Arc<AtomicUsize>);
# fn boot(lc: Lifecycle, hits: Hits) {
#     let n = Arc::clone(&hits.0);
#     lc.append(hook().on_start(move || async move {
#         n.fetch_add(1, Ordering::SeqCst);
#         Ok(())
#     })).unwrap();
# }
# #[tokio::main]
# async fn main() -> modrun::Result<()> {
let hits = Arc::new(AtomicUsize::new(0));
let app = Modrun::builder()
    .no_banner()
    .supply(Hits(Arc::clone(&hits)))
    .invoke(boot)
    .start()
    .await?;
assert_eq!(hits.load(Ordering::SeqCst), 1);
app.stop().await
# }
```

可替换的类型放在组合根（`provide` / `supply`），不要放进领域 [`Module`](https://docs.rs/modrun/latest/modrun/struct.Module.html) 里。
如果模块自己也 `provide` 了 `Repo`，测试里 `supply(FakeRepo)` 会碰到 `already provided`。可运行示例：`cargo run --example swap`。

```rust
# use modrun::{Modrun, Module};
# #[derive(Clone)] struct Repo;
# #[derive(Clone)] struct Service;
# fn connect_repo() -> Repo { Repo }
# fn fake_repo() -> Repo { Repo }
# fn new_service(_: Repo) -> Service { Service }
# fn boot(_: Service) {}
fn user_domain() -> Module {
    Module::builder("user")
        .provide(new_service)
        .invoke(boot)
}

# #[tokio::main]
# async fn main() -> modrun::Result<()> {
// 生产
Modrun::builder()
    .no_banner()
    .provide(connect_repo)
    .module(user_domain())
    .start()
    .await?
    .stop()
    .await?;

// 测试：同一个模块，在根上供给假实现
Modrun::builder()
    .no_banner()
    .supply(fake_repo())
    .module(user_domain())
    .start()
    .await?
    .stop()
    .await
# }
```

hook 里 sleep 的测试应显式设置超时（或 `no_start_timeout`）；默认预算是 15s。在终端里跑的测试请用 [`.no_banner()`](https://docs.rs/modrun/latest/modrun/struct.ModrunBuilder.html#method.no_banner)，避免 stderr 被 banner 弄乱。

## 错误

图的问题在构造函数运行之前失败。常见的 `Display` 文本：

* `type already provided: my::Config`
* `invoker in module '<root>' needs a dependency nothing provides: my::Db`
* `provider for my::Svc in module 'user' needs a dependency nothing provides: my::Repo`
* `dependency cycle detected involving: A -> B -> A`
* `application start timed out after 15s`
* `application stop timed out after 15s while unwinding`
* `invoker my::boot failed: …`
* `hook 'http.serve' failed: …`
* `background task failed during start`
* `required group is empty: modrun::Group<my::Route>`
* `provide_group_dyn type mismatch: expected my::Route, got alloc::string::String`
* `invoker in module '<root>' needs a dependency nothing provides: modrun::Group<my::Route>; register the group with init_group, provide_group, or require_group`

构造函数和 hook 失败会把原始错误留在
[`std::error::Error::source`](https://doc.rust-lang.org/std/error/trait.Error.html#tymethod.source)。
每个 hook 在日志和错误里都有 [`Hook::name`](https://docs.rs/modrun/latest/modrun/trait.Hook.html#method.name)
（默认 `"unnamed"`；[`task`](https://docs.rs/modrun/latest/modrun/fn.task.html) 会设自己的名字）。
多个 OnStop 失败会聚合成 [`MultipleStopError`](https://docs.rs/modrun/latest/modrun/struct.MultipleStopError.html)。
若更早阶段已经失败、unwind 又失败，两者都保留在
[`Error::CleanupAfterFailure`](https://docs.rs/modrun/latest/modrun/enum.Error.html) 上。

## 什么时候不该用

* 请求级对象（每个 HTTP 请求一份）
* `start()` 之后还想从活的容器里查找类型
* 字符串命名绑定（`"primary"` vs `"replica"`）——用 newtype
* 与运行时无关的库；modrun 面向 Tokio

## 示例

```bash
cargo run --example basic    # 领域模块、私有依赖、async 构造函数
cargo run --example handlers # 多模块向 Group 贡献成员
cargo run --example worker   # newtype 连接池 + 在 Stopped 上 select 的 task
cargo run --example swap     # 在组合根 supply 假实现（测试）
cargo run --example wrap     # wrapper 构造函数处理横切关注点
cargo run --example axum     # HTTP 服务：task_with 在 OnStart 里 bind，再 serve
```

## License

MIT
