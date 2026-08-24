# 中间件

中间件包裹一个请求处理程序。它在处理程序看到请求之前运行一次，在处理程序返回响应之后再运行一次，所以这里是放置横切关注点的地方 - 认证、日志、CORS、节流、计时、转换请求或响应。Suprnova 的接口和 Laravel 用户已经熟悉的那一个相同：一个 `handle(request, next)` 方法，决定是转发请求、短路它，还是在返回的路上修改响应。

## 该 trait

一个中间件是一个实现了 `Middleware` 的结构体：

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct LoggingMiddleware;

#[async_trait]
impl Middleware for LoggingMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // 前置处理：在处理程序之前运行。
        println!("--> {} {}", request.method(), request.path());

        // 转发给下一个中间件（如果这是最后一层，就是处理程序）。
        let response = next(request).await;

        // 后置处理：在处理程序返回之后运行。
        println!("<-- complete");

        response
    }
}
```

`handle` 有三件事可以做，对任意一次给定的请求，您只需要做其中一件：

- **转发。** 调用 `next(request).await` 把控制权交给下一层。返回的 `Response` 就是它上面的每一层都会看到的东西。
- **短路。** 不调用 `next`，直接返回 `Err(HttpResponse::...)`。框架会把 `Response` 的两个分支（`Result<HttpResponse, HttpResponse>`）收拢为单一的响应 - 一个 `Err` 是一个响应，不是一次崩溃。参见 [错误模型](error-model.md)。
- **修改。** 在转发之前修改请求，或者在之后修改响应。

`Next` 是 `Arc<dyn Fn(Request) -> MiddlewareFuture + Send + Sync>` - 把它当作一个从 `Request` 到 `Response` 的异步函数。

## 生成脚手架

CLI 会为您生成一个可以工作的中间件文件：

```bash
suprnova make:middleware Auth         # → src/middleware/auth.rs (AuthMiddleware)
suprnova make:middleware RateLimit    # → src/middleware/rate_limit.rs
suprnova make:middleware CorsMiddleware  # "Middleware" 后缀也可以，结果相同
```

生成的文件不是一个 TODO 占位符 - 它是一个真正的中间件，会给被包裹的请求计时，并使用 `RequestIdMiddleware` 安装的逐请求 id 记录入站/出站事件。把方法体换成您实际需要的东西。

## 注册中间件

根据作用域的不同，有三个可以安装它的地方：

### 全局

在每一个请求上运行，按注册顺序。在 `bootstrap()` 内部使用 `global_middleware!` 宏：

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, FrameworkError};
use crate::middleware;

pub async fn bootstrap() -> Result<(), FrameworkError> {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
    Ok(())
}
```

`global_middleware!(M)` 会展开为 `register_global_middleware(M)`。注册是**按具体类型幂等的** - 把同一个结构体注册两次，会保留第一次注册并输出一条调试日志。这让重新运行启动过程（测试、热重载、一个进程里的多个 `Server` 实例）变得安全。要安装同一行为的多份副本、并配以不同配置，把每一份包进一个独立的 newtype 里。

### 逐路由

在 `routes!` 宏产出的一个路由定义上链式调用 `.middleware(M)`：

```rust
// src/routes.rs
use suprnova::{routes, get};
use crate::{controllers, middleware::AuthMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/public", controllers::home::public),

    get!("/protected", controllers::dashboard::index)
        .middleware(AuthMiddleware),
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware),
}
```

### 逐分组

把中间件应用到一个 `group(...)` 块里的每一个路由：

```rust
use suprnova::Router;
use crate::middleware::{ApiMiddleware, AuthMiddleware};
use crate::controllers::{user, admin};

Router::new()
    // 公开路由 - 没有中间件。
    .get("/", home_handler)
    .get("/login", login_handler)

    // /api 下的每一个路由都携带 ApiMiddleware。
    .group("/api", |r| {
        r.get("/users", user::index)
         .post("/users", user::store)
         .get("/users/{id}", user::show)
    })
    .middleware(ApiMiddleware)

    // 管理路由共享认证。
    .group("/admin", |r| {
        r.get("/dashboard", admin::dashboard)
         .get("/settings", admin::settings)
    })
    .middleware(AuthMiddleware);
```

## 执行顺序

在运行时，这条链是由外而内运行的：

```
请求  →  RequestId  →  全局  →  分组中间件  →  路由中间件  →  处理程序
                                                                  │
响应  ←  RequestId  ←  全局  ←  分组中间件  ←  路由中间件  ←  处理程序
```

最先添加的中间件最先运行。在返回的路上，顺序会反过来 - `MiddlewareChain::execute` 把每一层的后置处理嵌套在前一层的里面。

如果一个中间件用 `Err(response)` 短路，链会立即展开：短路点**上方**的每一层仍然会在返回的路上看到这个响应，但**下方**（更靠近处理程序）的层都不会运行。

### 组中间件是被展平的，而不是分层堆叠的

这一点很重要，值得单独说明。**路由组中间件不是一个独立的运行时层。** 当 `GroupBuilder::try_finalize` 运行时，它会把组的中间件复制进每一个分组路由的 `(method, pattern)` 中间件列表里。到执行的时候，组中间件已经和直接附加在路由上的中间件无法区分。

两个后果：

- 运行时的顺序仍然是正确的（组中间件先于路由中间件运行，因为它先被注册），但**内省无法把组中间件和路由中间件区分开**。
- 中间件是按匹配到的模式（`"/posts/{id}"`）建立索引的，而不是按原始路径（`/posts/42`），所以参数化路由上的组中间件能可靠地触发。

展平这一步请参见 `framework/src/routing/group.rs`，执行循环请参见 `framework/src/middleware/chain.rs`。

## 短路

提前返回，在请求到达处理程序之前拦截它：

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct RequireApiKey;

#[async_trait]
impl Middleware for RequireApiKey {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if request.header("X-Api-Key").is_none() {
            return Err(HttpResponse::text("Unauthorized").status(401));
        }
        next(request).await
    }
}
```

这条链会把 `Result<HttpResponse, HttpResponse>` 收拢为单一的响应，所以 `Err(...)` 只是一个角色不同的响应。这个中间件上方的层仍然会在返回的路上观察到它，并可以对它做后置处理。

## Panic 安全性

`MiddlewareChain::execute` **不会**捕获 panic - 任何中间件或处理程序里的一次 panic 都会像其他任何异步函数一样直接向外展开。请求路径上的安全网上移了一层，位于服务器边界的 `execute_chain_safely` 里，它把这条链包在 `catch_unwind` 里，把一次 panic 转换为一个带有 request id 的清理过的 500，并为任何可观测性监听器分派 `ErrorOccurred`。完整的 panic 恢复流程请参见 [请求生命周期](lifecycle.md)。

这种拆分是刻意的：标准化的 panic 处理只发生一次，归请求生命周期所有，而不是在这个与层无关的原语内部被重复实现。在这个边界之外驱动一条链的消费者，要自己负责 `catch_unwind`。

## 内置中间件

这是一份不完全的地图。每一个都开箱即可安装 - 大多数需要一个配置结构体，没有一个需要脚手架。

| 中间件 | 用途 |
|---|---|
| `RequestIdMiddleware` | 永远处于最外层；为每个请求分配一个 UUID，并把它贯穿到日志和 `X-Request-Id` 里 |
| `TimeoutMiddleware` | 给出响应耗时的上界；超出时返回 503（见下文） |
| `CorsMiddleware` | 处理 CORS 预检 + 装饰跨源响应（见下文） |
| `CsrfMiddleware` | cookie 双重提交式的 CSRF 保护，`OriginPolicy` 可配置 |
| `RateLimitMiddleware` / `ThrottleRequestsMiddleware` | 令牌桶与滑动窗口节流；参见[速率限制](rate-limiting.md) |
| `SessionMiddleware` | 通过 cookie 加载/持久化会话；`req.session()` 靠它撑起 |
| `AuthMiddleware` / `GuestMiddleware` / `BearerTokenMiddleware` | 认证守卫的成员资格检查；参见[认证](authentication.md) |
| `LoginThrottleMiddleware` / `EnsureEmailVerifiedMiddleware` / `TwoFactorChallengeMiddleware` | 认证流程上的门；参见[认证流程](auth-flows.md) |
| `MaintenanceMiddleware` | 当缓存或文件系统上的维护标志被设置时返回 503 |
| `InertiaHeadersMiddleware` / `InertiaVersionMiddleware` / `Inertia303Middleware` / `InertiaValidationRedirectMiddleware` / `EncryptHistoryMiddleware` | Inertia 协议：每一个响应上的 `Vary: X-Inertia` 以及空 200 回跳重定向；资产版本 409 弹回；非 GET 重定向上的 302→303；一次 Inertia 访问中的 422 会变成带着已闪存错误的 303 回跳；历史加密。`Inertia::install` 会注册前四个；`EncryptHistoryMiddleware` 则单独选择性加入。参见 [Inertia 响应](frontend-inertia-responses.md#bootstrap-inertiainstall) |
| `IncludeMiddleware` | 为 `#[derive(Data)]` 的部分重新加载提供逐字段的 include 集合 |

### 请求超时

`TimeoutMiddleware` 给一个处理程序*产出*响应所能花的时间划出上界。否则，一个缓慢的处理程序或者一个挂住的数据库查询，可能会无限期地占着一条连接；一旦超出这个期限，超时就会返回 `503 Service Unavailable`。

```rust
// src/bootstrap.rs - 给每一条 HTTP 路由 30 秒的上限。
use suprnova::{global_middleware, TimeoutMiddleware};

global_middleware!(TimeoutMiddleware::default()); // DEFAULT_TIMEOUT = 30s
```

```rust
// 把单个端点收紧到 5 秒。
use suprnova::{Router, TimeoutMiddleware};

Router::new()
    .get("/report", heavy_report_handler)
    .middleware(TimeoutMiddleware::seconds(5));
```

`TimeoutMiddleware::new(Duration)` 接受任意时长；`TimeoutMiddleware::seconds(n)` 是整秒的简写。

全局中间件运行在路由中间件**外面**，所以一个全局超时是外层的天花板，而逐路由的超时只能让某一条路由*更严格* - 更短的那个期限会先触发。要让某一条路由跑得比全局默认值更久，请调高全局值，或者把这个全局中间件的作用范围限定到一个排除了该端点的路由分组上。

流式响应（`HttpResponse::sse(...)`、`HttpResponse::stream_bytes(...)`）天然被豁免：处理程序会立即返回，带着一个惰性的响应体，由 hyper 在中间件链完成之后排空。WebSocket 升级也会被显式跳过。取消安全性的语义请参见[超时](timeout.md)。

### CORS

`CorsMiddleware` 会加上浏览器所需要的那些 `Access-Control-*` 请求头，好让一个跨源页面能读取您的响应；它还会回答浏览器在非简单跨源调用之前发出的那个预检 `OPTIONS` 请求。同源应用（也就是默认的 Inertia 搭建方式）用不着它 - 只有当一个*不同*源上的浏览器调用您的 API 时，它才要紧。

CORS 必须**全局**安装，预检才能到得了它（预检从来不匹配任何一条路由，所以一个逐路由的 CORS 中间件永远也见不到它）。这里刻意没有一个宽松的默认值 - 请显式挑一个源策略：

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, CorsConfig, CorsMiddleware};

global_middleware!(CorsMiddleware::new(
    CorsConfig::allow_origins(["https://app.example", "https://admin.example"])
        .allow_credentials(true)
        .max_age(std::time::Duration::from_secs(600)),
));
```

`CorsConfig::any_origin()` 会显式地选择启用 `Access-Control-Allow-Origin: *`。构建器方法有：`.methods([...])`、`.allow_headers([...])` / `.allow_any_headers()`、`.expose_headers([...])`、`.paths([...])`（把 CORS 限定到 URL 模式上）、`.allow_origin_patterns([regex...])`、`.skip_when(|req| bool)`、`.allow_credentials(bool)`、`.max_age(Duration)`。还一并提供了 Laravel 命名的别名（比如 `.supports_credentials`、`.allowed_methods`），这样一份 Laravel 配置可以直接映射过来。

`Access-Control-Allow-Origin: *` 和凭据一起使用是非法的 - 浏览器会拒绝它。当设置了 `.allow_credentials(true)` 时，这个中间件总是回显请求里那个具体的 `Origin`，而不是 `*`，所以这个非法组合永远不可能被发出。非通配的响应还会带上 `Vary: Origin`，好让共享缓存保持正确。参见 [CORS](cors.md)。

## Pipeline - Laravel 的 `Illuminate\Pipeline\Pipeline`

`Pipeline` 是 Suprnova 对 Laravel 管道类的类比 - 一个建立在 `MiddlewareChain` 之上的流式构建器，映照 Laravel 用户已经熟悉的 `send / through / pipe / then / then_return / finally_with` 形态。当您想在请求生命周期之外组装一条中间件链时（一个任务、一个 CLI 命令、一次性的集成测试），它很有用：

```rust
use suprnova::{Pipeline, Request};

let response = Pipeline::new()
    .send(request)
    .through([AuthMiddleware, LoggingMiddleware])
    .pipe(CorsMiddleware::new(cors_config))
    .finally_with(|| tracing::info!("pipeline complete"))
    .then(|req| async move { handler(req).await })
    .await;
```

Rust 侧的别名和 Laravel 的名称一起提供：`with_request` 对应 `send`，`with_middleware` 对应 `through`，`push` 对应 `pipe`，`on_finally` 对应 `finally_with`，`execute` 对应 `then`。在您的代码库里，哪个读起来更顺就用哪个。

| Pipeline 方法 | Laravel | Rust 别名 | 用途 |
|---|---|---|---|
| `send(request)` | `send($passable)` | `with_request(request)` | 设置要贯穿传递的请求 |
| `through(iter)` | `through($pipes)` | `with_middleware(iter)` | 替换管道列表 |
| `through_boxed(iter)` | - | - | 用预先装箱的中间件替换管道列表 |
| `pipe(M)` | `pipe($pipes)` | `push(M)` | 追加单个中间件 |
| `pipe_boxed(M)` | - | - | 追加一个预先装箱的中间件 |
| `then(destination)` | `then($destination)` | `execute(destination)` | 用目标处理程序运行这条链 |
| `then_with(req, dst)` | - | - | 内联覆盖被传递的对象 |
| `then_return()` | `thenReturn()` | - | 运行这条链，返回一个 204 No Content |
| `finally_with(F)` | `finally($callback)` | `on_finally(F)` | 在目标解析完成之后运行 |

## 可终止中间件 - 响应后钩子

可终止中间件在响应已经发送给客户端*之后*运行。把它用于那些不需要阻塞响应的慢速 IO：会话持久化、审计日志、指标刷新。

Suprnova 把这个做成一个独立于 `Middleware` 的专用 `Terminable` trait，这样请求路径和终止路径就能保持类型清晰。一个类型可以只实现其中一个，也可以两个都实现：

```rust
use suprnova::{Terminable, TerminationSnapshot, register_terminable, async_trait};

pub struct AuditLogTerminator;

#[async_trait]
impl Terminable for AuditLogTerminator {
    async fn terminate(&self, snapshot: &TerminationSnapshot) {
        tracing::info!(
            method = %snapshot.method,
            path = %snapshot.path,
            status = snapshot.status,
            "request handled",
        );
    }
}

// 在 bootstrap.rs 中
register_terminable(AuditLogTerminator);
```

服务器会在每一次响应之后（包括 4xx 和 5xx），按注册顺序遍历已注册的可终止对象，并逐个 await。错误会通过 `tracing::error!` 记录下来，然后被吞掉 - 响应早已经离开了大楼，已经没有人可以把它们呈现给了。

注册是按具体类型幂等的。`registered_terminables()`、`terminable_count()` 和 `has_terminable::<T>()` 为测试和启动期诊断提供内省能力。

## 具名别名与分组

对于偏好字符串键中间件的使用者（Laravel 的 `middlewareAliases` / `middlewareGroups`），Suprnova 提供了一个进程级全局的别名 + 分组注册表：

```rust
use suprnova::middleware::{
    register_middleware_alias, register_middleware_group,
    resolve_middleware_group,
};

// 别名是工厂闭包 - 每次解析都会重新调用一次，所以每个
// 路由注册都会产出一个独立的中间件实例。
register_middleware_alias("auth", || AuthMiddleware::new());
register_middleware_alias("throttle", || ThrottleRequestsMiddleware::default());

// 分组会打包别名。支持嵌套分组。
register_middleware_group("api", ["auth".into(), "throttle".into()]);
register_middleware_group("web", ["session".into(), "auth".into()]);

// 在启动时或逐路由地解析成一个 Vec<BoxedMiddleware>。
let api_mws = resolve_middleware_group("api")?;
```

`resolve_middleware_group` 在以下情况下会返回 `Err(MiddlewareResolveError)`：

- `UnknownGroup(name)` - 这个具名分组从未被注册过；
- `UnknownAlias { group, missing }` - 分组里的一个条目不是一个已知的别名；
- `UnknownNestedGroup { group, missing }` - 一个嵌套分组的引用无法解析；
- `CycleDetected { group }` - 这个分组定义是递归的。

对同一个名称，别名或分组的注册是**后写覆盖前写**的，这镜照了 Laravel 那个可重新赋值的内核数组。

## 中间件优先级

`prepend_middleware_priority::<M>()` / `append_middleware_priority::<M>()` 会把一个 `TypeId` 注册进进程级全局的优先级列表 - 这是 Suprnova 对 Laravel `Kernel::$middlewarePriority` 的类比。无论注册顺序如何，类型在这个列表里出现得越靠前，就越会被排到链的前面：

```rust
use suprnova::{append_middleware_priority};

// 无论 SessionMiddleware 和 AuthMiddleware 的注册顺序如何，
// SessionMiddleware 总是先于 AuthMiddleware 运行。
append_middleware_priority::<SessionMiddleware>();
append_middleware_priority::<AuthMiddleware>();
```

`middleware_priority()` 返回当前 `Vec<TypeId>` 的一份快照，供诊断使用，或者供想要驱动自己的排序器的嵌入方使用。

## 注册表内省

除了 `register_global_middleware`，这个注册表还暴露了：

| 接口 | Laravel | 用途 |
|---|---|---|
| `prepend_global_middleware(M)` | `prependMiddleware` | 插入到链的最前面 |
| `has_global_middleware::<M>()` | `hasMiddleware` | 类型 `M` 是否已注册 |
| `global_middleware_count()` | - | 当前已注册的全局中间件数量 |
| `MiddlewareRegistry::from_global()` | - | 把全局注册表快照进一个逐服务器的注册表 |
| `MiddlewareRegistry::prepend(M)` | - | 在一个注册表实例上以构建器风格前置插入 |
| `MiddlewareRegistry::append_boxed(M)` | - | 追加一个预先装箱的中间件 |
| `MiddlewareRegistry::prepend_boxed(M)` | - | 前置插入一个预先装箱的中间件 |
| `MiddlewareRegistry::len()` / `is_empty()` | - | 构建器内省 |

`MiddlewareRegistry::from_global()` 会在调用那一刻给全局注册表拍一份快照。请在构建服务器**之前**注册每一个全局中间件 - 在服务器构建**之后**发起的 `global_middleware!` 调用不会追溯性地生效，所以一个正在运行的服务器的中间件栈不会在它脚下发生变化。

## 文件布局

当您有了几个中间件之后，一个典型的布局是：

```
src/
├── middleware/
│   ├── mod.rs          # mod + pub use
│   ├── auth.rs         # AuthMiddleware
│   ├── logging.rs      # LoggingMiddleware
│   └── audit.rs        # AuditLogTerminator
├── bootstrap.rs        # global_middleware! + register_terminable
├── routes.rs           # .middleware(M) 逐路由
└── main.rs
```

`make:middleware` 会让 `src/middleware/mod.rs` 保持同步 - 在文件生成时，它会追加新的 `mod foo;` 声明，以及匹配的 `pub use foo::FooMiddleware;` 重导出。

## 为什么 Suprnova 有所不同

Laravel 在 `app/Http/Kernel.php` 里注册中间件类，并通过容器解析它们，容器会对构造函数的类型提示做反射，以注入依赖。PHP 的每进程一个请求模型意味着内核在每个请求上都会被重建，所以反射式解析的代价是每个请求付出一次，并在请求之间消失。

Suprnova 的进程模型是一个二进制文件在许多线程上服务许多并发请求。为每个请求构建一条全新的链，会在全局中间件列表上强加一个同步点，并在每个请求的每一层都重新分配 `Arc<dyn Middleware>`。取而代之的是：

- 全局中间件在启动时被注册进一个 `OnceLock<RwLock<Vec<...>>>`，按 `TypeId` 建立索引以支持幂等注册。
- `MiddlewareRegistry::from_global()` 在服务器构建时给全局列表拍一次快照；逐请求的链复用这份快照。
- 这条链本身是通过嵌套 `Arc<dyn Fn>` 闭包组成的，所以逐请求的工作是每层一次 `Arc::clone`，而不是一次全新的分配。

面向用户的接口 - `handle(request, next)`、`global_middleware!` 宏、具名别名、优先级列表、可终止钩子 - 和一个 Laravel 开发者会用的是同一套。底层的机制把 PHP 的逐请求重建换成了一个 Rust 风格的、启动时快照的模型，这样框架就可以在不争用注册表的情况下服务并发请求。

## 下一步

- [请求生命周期](lifecycle.md) - 这条链在哪里运行，panic 又是如何在服务器边界被捕获的
- [错误模型](error-model.md) - `Result<HttpResponse, HttpResponse>` 到底意味着什么，短路又是如何收拢的
- [超时](timeout.md) - `TimeoutMiddleware` 的取消安全语义详解
- [CORS](cors.md) - 预检处理、来源模式、路径作用域
- [速率限制](rate-limiting.md) - `RateLimitMiddleware` / `ThrottleRequestsMiddleware` 和 `BackendErrorPolicy`
- [路由](routing.md) - `routes!`、`Router` 和 `group(...)` 展开成什么
