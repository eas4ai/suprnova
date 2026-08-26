# 应用启动

`bootstrap.rs` 是您的应用在启动时完成自身装配的唯一地方。容器绑定、事件监听器、观察者、监督程序、全局中间件 - 任何应该在第一个请求到达服务器（或第一个作业从队列中弹出）之前就存在的东西，都在这里注册。这里没有需要组装的服务提供者脚手架。

这里有两个钩子，而不是一个。`register` 是进程范围的：每个子命令都会运行它，包括 `queue:work`、`schedule:work`、`workflow:work` 和您的控制台二进制文件，而不只是服务器。请在这里注册数据库连接、容器绑定、事件监听器、观察者、监督程序和工作进程作业。通过 `.http_bootstrap` 接线的 `register_http_stack` 只在服务器路径（`serve` / `web:run`）运行 - 全局中间件和 `Inertia::install` 属于这里。下方“bootstrap 在启动顺序中的位置”一节解释了为何要拆分。

## 结构

脚手架应用的入口点以链式方式构建一个 [`Application`](lifecycle.md) 并运行它。bootstrap 是构建器上的两个方法：

```rust
// cmd/main.rs
use app::{bootstrap, config, migrations, routes};
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

### `#[suprnova::main]`，而不是 `#[tokio::main]`

这个属性并不只是装饰性的，把它换回去会导致启动失败，并给出一条解释原因的消息。

加载 `.env` 会写入进程环境，而 `set_var` 只有在进程是单线程时才是安全的。`#[tokio::main]` 会在*整个* `main` 的外面构建运行时，所以在您的第一条语句运行之前，每一个工作线程都已经存在 - 而其中任何一个都可能通过 DNS 解析、时间格式化或某个 C 依赖间接调用 `getenv`。这种竞态一旦出错就是静默的，这是竞态能具备的最糟糕的属性。

`#[suprnova::main]` 保留了您本来就会编写的那个同样的 `async fn main`，只是重新排序了两件事：它先加载环境，然后构建运行时，再在其上运行您的函数体。它接受与 `#[tokio::main]` 相同的 `flavor` 和 `worker_threads` 参数。

如果 `Application::run` 发现环境从未在单线程上下文中被加载，它会拒绝启动，而不是仅仅发出警告 - 一个在 `#[tokio::main]` 下“正常”启动的应用，恰恰就是那种会在几周后破坏一次不相关的环境读取的应用。

框架会在启动序列中调用您的 `bootstrap_fn` 一次，这发生在环境已加载、运行时驱动程序（Cache、Queue、RateLimit、Mail）已就绪，但路由器尚未构建之前。同样的调用也会在后台工作进程（`queue:work`、`workflow:work`、`schedule:work`）中运行，因此在这里注册的观察者或监听器，对来自队列作业的插入操作和来自 HTTP 处理程序的插入操作会触发得完全一样。`http_bootstrap_fn` 紧接在 `bootstrap_fn` 后运行，但只在服务器路径上运行 - 后台工作进程和控制台二进制文件从不调用它。[请求生命周期](lifecycle.md)走一遍完整的顺序。

两个函数的签名由 `Application::bootstrap` 和 `Application::http_bootstrap` 固定：

```rust
// src/bootstrap.rs
pub async fn register() {
    // 数据库、绑定、观察者、监听器、监督程序、工作进程作业注册
}

pub fn register_http_stack() {
    // 全局中间件、Inertia::install
}
```

`register` 返回 `()`；`register_http_stack` 是同步的，不是 `async` - 两者在调用点都接为异步闭包（`.http_bootstrap(|| async { bootstrap::register_http_stack() })`），因为普通函数指针也能作为测试 harness 入口点，而无需将 `async` 引入测试。可能失败的设置使用 `.expect("…")`，并配一条说明补救措施的消息 - 启动阶段正是应该明确地失败的时候。示例应用的调用是 `DB::init().await.expect("Failed to connect to database");`，所以缺失的 `DATABASE_URL` 会在启动时中止进程并打印出真实的错误，而不是在第一个请求时表现为一条令人困惑的“connection refused”。

## bootstrap 里放什么

一个真正的 `bootstrap` 函数只做少数几件不同的事。下面的每个小节对应其中一件。示例应用的 `app/src/bootstrap.rs` 把它们全都用上了，是可用的参考实现。

### 数据库连接

```rust
use suprnova::DB;

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");
}
```

`DB::init` 读取 `DatabaseConfig`（由您的 `config_fn` 注册）并打开连接池。这个连接作为单例存储在[服务容器](container.md)中 - `DB::connection()` / `DB::get()` 可以在任何地方解析它。当您想指向环境变量派生的 URL 之外的某个地方时，`DB::init_with(config)` 就是面向测试和工具场景的脱围机制。

### Magnetar 认证引擎

使用内置密码、passkey、magic-link、bearer、锁定、记住我或 OAuth 门面的应用，会在数据库和 `APP_KEY` 就绪后初始化 Magnetar：

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");

    let database = DB::connection().expect("DB not initialized");
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config)
        .await
        .expect("Failed to initialize Magnetar");
}
```

默认的 `MagnetarConfig` 会把应用身份绑定到规范的 `app_users` 表。生成的全栈脚手架使用 `users` 模型，并不会初始化 Magnetar，因此不要把上面的默认初始化器原样添加到那个脚手架中。请使用 API 脚手架的 `app_users` 模型，或者为现有的 `users` 表构造自定义的 `MagnetarHostEngine` 和 `AuthSchema` 绑定。框架的 `UserProvider` 与 Magnetar 主机绑定必须指向同一个应用身份。当前默认 `MagnetarConfig` 初始化的可用参考是 API 脚手架，而不是 `app/src/bootstrap.rs`。

API 脚手架在应用 bootstrap 中读取 `PASSKEY_RP_ID` 和 `PASSKEY_RP_ORIGIN`。这些名称是脚手架约定，而非框架拥有的环境变量。

### 全局中间件

全局中间件只用于 HTTP，因此它属于 `register_http_stack`，而非 `register`：

```rust
use suprnova::{global_middleware, SessionMiddleware, SessionConfig, TimeoutMiddleware};
use crate::middleware;

pub fn register_http_stack() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));
}
```

`global_middleware!` 注册一个在每个请求上都会运行的层，包括未路由的请求（404、OPTIONS 预检）。您注册的顺序就是链运行的顺序 - 由外而内。框架会把自己的 `RequestIdMiddleware` 放在最外层；您添加的一切都位于它内部。[中间件](middleware.md)解释了完整的链结构，包括逐路由的层。

### 容器绑定

容器会接受您放进去的任何东西；这些宏是 [`App`](container.md) 门面之上的语法糖。

```rust
use std::sync::Arc;
use suprnova::{App, bind, singleton, factory};
use crate::providers::DatabaseUserProvider;

pub async fn register() {
    // trait → 单例（包装进 Arc）：
    bind!(dyn UserProvider, DatabaseUserProvider);

    // 具体类型单例：
    singleton!(MyConfig { max_uploads_per_user: 100 });

    // 工厂（每次解析时构造）：
    factory!(|| RequestLogger::new());

    // 或者直接调用门面，以获得更精细的控制：
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(hub);
}
```

trait 对象绑定是最常见的形式 - 绑定一个接口，让处理程序和测试去替换其实现。[服务容器](container.md)章节有完整的绑定 API，包括 `bind_factory!`、`_if_absent` 变体和三层查找模型。

### 事件监听器与观察者

分发器在 bootstrap 运行的那一刻就已经存活 - 在这里注册的监听器能看到之后的每一次分发。

```rust
use std::sync::Arc;
use suprnova::EventFacade;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;

pub async fn register() {
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
}
```

Eloquent 观察者（`#[suprnova::observer(M)]`）在编译期通过 `inventory::submit!` 自行收集。一次调用就会把 inventory 排空到分发器中：

```rust
suprnova::eloquent::observers::bootstrap_observers()
    .await
    .expect("observer install failed");
```

这次调用是幂等的 - 重新运行 bootstrap（一个第二次启动的工作进程）不会让监听器适配器被重复注册。[事件](events.md)涵盖了分发和监听器编写；[Eloquent](eloquent.md)涵盖了观察者。

### 监督程序

通过 `Supervisor` trait 和 `inventory::submit!` 声明的长时间运行的后台任务，通过一次调用启动：

```rust
use suprnova::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

每个监督程序都在自己的重启循环任务中运行，带有一个 panic 边界；发生 panic 的监督程序会被记录并重启，而不会被允许拖垮整个进程。参见[监督程序](supervisors.md)了解该 trait 和重启策略。

### 工作进程作业注册

工作进程需要按名称分发的队列作业和 mailable，会在启动时自行注册：

```rust
use suprnova::queue::worker::register_job;

pub async fn register() {
    register_job::<crate::jobs::welcome_log::WelcomeLog>();

    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();
}
```

没有这一步，工作进程就无法把一个已入队的信封映射回处理它的那个类型。

## 启动后钩子：`booted()`

bootstrap 负责*注册*；`booted()` 负责*解析*。构建器接受第二个回调，它会在服务器完成自身的服务启动之后、但在开始接受连接之前触发。当您需要读取框架自身在启动期间绑定的某个东西时，就用它：

```rust
Application::new()
    .config(config::register_all)
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
    .routes(routes::register)
    .booted(|| {
        let cfg: MyConfig = suprnova::App::get().unwrap();
        tracing::info!(?cfg, "services booted");
    })
    .run()
    .await;
```

`booted` 是同步的，运行在 `Server::from_config` 之后 - 驱动程序已就绪，加密密钥已加载，您的绑定已经存在。大多数应用不需要这个钩子；当一个一次性的启动后副作用需要看到一个完全构造好的容器时，才需要用到它。

## 一个完整的 `bootstrap.rs`

这个代表性组合并不是示例应用的逐字摘录。它把进程范围的注册放在 `register`，把只用于 HTTP 的设置放在 `register_http_stack`。上面的 Magnetar 初始化是单独展示的，因为它的应用用户 schema 必须和框架用户提供者匹配。

```rust
//! 应用 bootstrap - 注册服务、监听器、全局中间件，
//! 以及 Inertia 层。

use std::sync::Arc;
use std::time::Duration;

use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::features::{FeatureMiddleware, bootstrap_database_cached};
use suprnova::queue::worker::register_job;
use suprnova::{
    App, DB, EloquentUserProvider, EventFacade, FrameworkError, Inertia,
    InertiaConfig, SessionConfig, SessionMiddleware, Storage, SupervisorRegistry,
    UserProvider, bind, global_middleware,
};

use crate::broadcasting::ChatChannel;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;
use crate::middleware;
use crate::models::users::User;

pub async fn register() {
    // ── 数据库
    DB::init().await.expect("Failed to connect to database");

    // ── 认证提供者
    bind!(dyn UserProvider, EloquentUserProvider::<User>::new());


    // ── 广播中枢 + 频道注册表
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    let mut registry = ChannelRegistry::new();
    registry.register(ChatChannel);
    App::singleton(Arc::new(registry));

    // ── 事件监听器 + 桥接
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
    EventFacade::broadcast::<UserRegistered>(Arc::clone(&hub)).await;

    // ── 存储磁盘（生产环境中由环境变量控制的 S3）
    Storage::register_fs("public", "./storage/public")
        .expect("register public disk");

    // ── 工作进程作业注册
    register_job::<crate::jobs::welcome_log::WelcomeLog>();
    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();

    // ── 观察者 + 监督程序
    suprnova::eloquent::observers::bootstrap_observers()
        .await
        .expect("observer install failed");
    SupervisorRegistry::start_all().await;

    // ── 功能标志
    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature-flag chain wired");
}

pub fn register_http_stack() {
    // ── 全局中间件（按注册顺序由外向内）
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(suprnova::TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));

    // ── Inertia 协议层（不固定版本：默认实现会对 Vite 构建清单做哈希，
    // 所以一次前端构建会自行提升资源版本 - 参见
    // frontend-inertia-responses.md 中的“版本检测”）
    Inertia::install(&InertiaConfig::new()).expect("Inertia install failed");

    global_middleware!(FeatureMiddleware::new());
}
```

注意这种节奏：每个代码块只做一件事，调用一两个 API，然后要么成功，要么带着一条清晰的消息失败。这里没有什么取巧的地方；这些函数之所以长，是因为应用有很多活动的部件，而不是因为 bootstrap 模式本身复杂。

## 何时使用 bootstrap，何时使用 `#[injectable]`

`#[injectable]` 是一个宏，会在编译期把一个单例自动注册进容器的 `inventory`。对于那些只需要它们的 `#[inject]` 依赖就能构造完成的服务，它是正确的选择：

```rust
use suprnova::injectable;

#[injectable]
pub struct UserService;

#[injectable]
pub struct OrderService {
    #[inject]
    user_service: UserService,
}
```

这些服务会自行解析；bootstrap 不需要碰它们。

当构造过程需要其他任何东西时 - 一个环境变量、一个已构造好的配置结构体、一个 `dyn Trait` 绑定、一个运行时决策、一次异步的初始化调用，或者对某个本身并非服务的东西进行注册（一个监听器、一个观察者、一个队列作业映射、一个全局中间件层）- bootstrap 才是正确的地方。

| 用 `#[injectable]` 处理 | 用 `bootstrap` 处理 |
|---|---|
| 不需要运行时配置的具体类型单例 | 任何 `dyn Trait` |
| 由其他可注入对象构造出的服务 | 启动期的任何异步操作 |
| 默认的 DI 依赖图 | 由环境变量驱动的值 |
| | 事件监听器、观察者、监督程序 |
| | 全局中间件 |
| | 工作进程作业 + mailable 注册 |

您可以自由混用两者。到 `bootstrap` 运行时，`#[injectable]` 服务在容器里已经可见，所以 bootstrap 里的绑定可以读取它们。

## bootstrap 在启动顺序中的位置

完整的顺序（摘自[请求生命周期](lifecycle.md)）：

1. `Config::init(".")` - 加载 `.env`，探测环境
2. `init_policies()` - 将 `#[policy]` 的 inventory 排空
3. 您的 `config_fn` 运行（类型化配置注册）
4. 运行迁移（在 `serve` 上自动迁移）
5. **您的 `bootstrap_fn` 运行** ← `bootstrap::register`
6. **您的 `http_bootstrap_fn` 运行，仅服务器路径** ← `bootstrap::register_http_stack`
7. 从您的 `routes_fn` 组装路由
8. `Server::from_config` 启动驱动程序 + 容器
9. 您的 `booted_fn` 触发
10. 服务器开始接受连接

后台工作进程（`queue:work`、`workflow:work`、`schedule:work`）和控制台二进制文件共享第 1-5 步和第 8 步 - 它们运行 `bootstrap_fn`，但从不运行第 6 步，因为只有 `serve` / `web:run` 会运行 `http_bootstrap_fn`。这使您在 `register` 中注册的监听器或观察者能触达工作进程代码路径，就像触达 HTTP 处理程序一样，同时使 `register_http_stack` 的全局中间件和 `Inertia::install` 不会进入从不服务 HTTP 的进程。

### 为什么 Suprnova 有所不同

Laravel 也会为 `artisan` 命令和队列工作进程运行每个服务提供者的 `register()` 和 `boot()`，而不只是为 HTTP 请求运行 - 因为它的 Vite 集成会在渲染时从 `@vite` Blade 指令被要求渲染的内容中惰性解析资产 URL，所以它能这样做。一个从不渲染视图的工作进程从不触碰 manifest，所以缺失的构建不会出现。

Suprnova 的 `Inertia::install` 在启动时解析一次 manifest，并在生产中缺失时故障关闭 - 这是刻意的，以免配置错误的部署提供指向无人运行的 Vite 开发服务器的资产 URL。这一设计选择正是会弄坏一个（正确地）不携带 `public/assets` 的工作进程或控制台镜像的原因：Laravel 将失败推迟到请求时，而 Suprnova 若不作拆分，会在每个子命令的进程启动时遇到它。将启动表面拆为 `bootstrap` 和 `http_bootstrap` 可保留故障关闭检查，但只在它所属的会实际渲染 Inertia 页面服务器路径上执行。

Laravel 还会将启动本身拆分到多个服务提供者中：每个提供者实现 `register()` 和 `boot()`，它们收集在 `config/app.php`，Laravel 分两轮遍历它们（全部 `register`，然后全部 `boot`），这样服务可依赖另一个提供者的绑定，而无需在用户代码中安排顺序。当应用积累几十个不同子系统时，提供者类提供一个组织单元。

Suprnova 将此收拢为两个函数 - `register` 和 `register_http_stack` - 而非每个提供者都有一对 `register`/`boot`。原因如下：

- **两轮式的 `register`/`boot` 拆分解决的是一个 Rust 并不存在的排序问题。** `#[injectable]` 和容器的 `bootstrap_singletons` 已经能在不需要用户可见排序的情况下解析依赖图。绑定是内联注册的；查找机制会处理剩下的部分。
- **两个函数比十个函数更容易读懂。** 一个新贡献者打开 `bootstrap.rs`，就能在两个地方看到每一个绑定、每一个监听器、每一个观察者、每一个中间件层。提供者式的碎片化会把应用实际在做的事情藏起来。
- **inventory 风格的自动注册涵盖了其余部分。** 观察者、监督程序、计划任务、策略和队列处理程序，全都在编译期通过 `inventory::submit!` 自行收集。bootstrap 用单次调用（`bootstrap_observers`、`SupervisorRegistry::start_all`）排空这些 inventory，而不必逐一枚举。

Laravel 的提供者拆分真正物有所值的地方在于库的分发：一个自带绑定的 crate，会希望有一个注册入口点，让应用可以选择接入而不必编辑自己的 bootstrap。Suprnova 的对应做法，是在 crate 的根模块中提供一个公开的 `pub async fn register()`，再由应用的 `bootstrap` 里一行调用它。人体工程学上的代价是一行代码；可读性上的收益，是把一切都摆在同一个地方。

## 下一步

- [请求生命周期](lifecycle.md) - 完整的启动顺序，以及 `bootstrap_fn` 在哪里被触发
- [服务容器](container.md) - `App::bind` / `App::singleton` /
  `App::factory` 和三层查找
- [配置](configuration.md) - 在 bootstrap 之前运行的类型化配置注册
- [中间件](middleware.md) - 用 `global_middleware!` 注册的层的链组合
- [事件](events.md) - 监听器和观察者接入的分发器
