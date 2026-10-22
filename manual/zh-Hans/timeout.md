# 请求超时

`TimeoutMiddleware` 给每一个 HTTP 请求都套上一个硬性的截止时间。一个缓慢的处理程序 - 一次挂起的数据库查询、一个没有响应的上游 API、某个热路径里意外的无限循环 - 否则就会一直占着一个 hyper 连接，直到客户端放弃，或者操作系统杀掉这个进程。这个超时中间件给这次等待设了上限，丢弃这个飞行中的处理程序，并返回 `503 Service Unavailable`，这样运维人员看到的是这次失败，而不是应用在悄无声息地泄漏连接。

当您在构建任何会对接公共互联网的东西、任何会扇出到第三方 API 的东西，或者任何“数据库今天可能会很慢”是一个现实的、随时可能发生的日常情况时，就该伸手去拿它。

```rust
use suprnova::{global_middleware, TimeoutMiddleware};

pub async fn register() {
    // 每一条 HTTP 路由都会得到一个 30 秒的上限。
    global_middleware!(TimeoutMiddleware::default());
}
```

单独这一行，就给了整个应用和 Suprnova 数据库连接超时同样的默认上限 - 选定一次，处处应用。逐路由的覆盖也只需要一行。本章接下来会准确解释这个截止时间限定的是什么、它刻意不限定的是什么，以及它如何与 panic 边界、流式响应和 WebSocket 交互。

## 这个中间件

`TimeoutMiddleware` 位于 `suprnova::TimeoutMiddleware`。它暴露了三个构造函数和一个访问器：

```rust
use std::time::Duration;
use suprnova::TimeoutMiddleware;

let default_30s = TimeoutMiddleware::default();
let custom      = TimeoutMiddleware::new(Duration::from_millis(2_500));
let whole_secs  = TimeoutMiddleware::seconds(5);

assert_eq!(default_30s.duration(), Duration::from_secs(30));
assert_eq!(custom.duration(),      Duration::from_millis(2_500));
assert_eq!(whole_secs.duration(),  Duration::from_secs(5));
```

`TimeoutMiddleware::default()` 用的是一个 30 秒的截止时间。这个数字不是随意选的 - 它和 `DB_CONNECT_TIMEOUT`（同样是 30s）保持一致，这样一个卡在等待全新数据库连接上的请求，和一个卡在处理程序内部的请求，就共用同一个上限。如果您调高了一个，就把另一个也调高。

`TimeoutMiddleware::seconds(n)` 是常见的整数秒场景的简写。`TimeoutMiddleware::new(Duration::…)` 是您需要毫秒级精度时的脱围机制（一个永远不该超过 200ms 的内部健康检查；一次带着 50ms 预算的合成探测）。

## 全局安装它

一个全局超时是正确的起点：它给每一条路由都套上一个上限，不需要任何人记得去添加它。把它和您其他的全局中间件一起安装在 `bootstrap.rs` 里：

```rust
// src/bootstrap.rs
use suprnova::{
    global_middleware, CorsConfig, CorsMiddleware, DB, RequestIdMiddleware, TimeoutMiddleware,
};
use crate::middleware::LoggingMiddleware;

pub async fn register() {
    DB::init().await.expect("database connect");

    // 运行顺序很重要：request-id 在最前面（这样超时日志才能带上它），
    // 然后是日志记录（这样缓慢的请求仍然能被观察到），
    // 最后才是超时本身。
    global_middleware!(RequestIdMiddleware);
    global_middleware!(LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());

    global_middleware!(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"]),
    ));
}
```

顺序很重要，因为全局中间件是按注册顺序把链的其余部分包起来的：`RequestIdMiddleware` 在进入时最先运行，在退出时最后运行，所以当超时触发它的 `503` 时，request id 仍然在作用域内。把超时放在日志记录之前，会让那些最终确实完成了、但很缓慢的请求，从访问日志里被隐藏起来。

## 逐路由收紧

一个 30 秒的全局上限，是刻意定得宽松的 - 它在那里是为了拦住失控的处理程序，不是为了强制执行 SLA。当某个特定端点应该更快失败时，就给它接上一个逐路由的超时：

```rust
use suprnova::{Router, TimeoutMiddleware};

Router::new()
    // 公开的报表端点：必须在 5s 内响应，否则我们宁愿返回 503、
    // 让客户端重试，也不要一直阻塞着。
    .get("/report", controllers::report::show)
    .middleware(TimeoutMiddleware::seconds(5));
```

您也可以把一个更紧的超时接到一个路由分组上。这是一个公开 API 的典型形态：每个请求都应该很快，而应用的其余部分保持 30 秒的默认值：

```rust
use suprnova::Router;
use suprnova::TimeoutMiddleware;

Router::new()
    .group("/api", |r| {
        r.get("/users",       controllers::api::users::index)
         .post("/users",      controllers::api::users::create)
         .get("/users/{id}",  controllers::api::users::show)
    })
    .middleware(TimeoutMiddleware::seconds(3));
```

### 全局是一个上限；逐路由只能收紧

全局中间件运行在路由中间件的**外层**。这条链由外向内包裹：

```
全局超时（30s） → 路由超时（3s） → 处理程序
```

两个 `tokio::time::timeout` future 都已经上膛；内层的那个会先触发，因为它的截止时间更短。所以一个逐路由的超时只能让一条路由变得*更严格*，永远不会比全局的更宽松。

如果某个单一端点确实合理地需要比全局默认值运行*更久* - 一份缓慢的报表、一次大文件上传、一个长轮询的回退方案 - 您有两个选项：

1. 调高全局值。最简单，但它也会放宽所有其他路由的上限。
2. 把全局中间件的作用域限制到一个*排除*这个长耗时端点的路由分组，然后给这条缓慢的路由接上一个单独的超时（或者不接）。这样其余地方都能保持严格的默认值。

第二个选项对单个例外情况来说是正确的形态；当整整一类工作都需要更多余地时，第一个才是正确的。

## 这个截止时间实际限定的是什么

这个截止时间和 `next(request)` 返回的那个 future 竞态。这个 future 会在您的处理程序返回它的 `HttpResponse` 的那一刻解决 - 不是在响应体完成流式传输的时候。这个区分是承重的：

- **普通的处理程序**会在返回之前构建好完整的响应体，所以这个截止时间实际上限定的是处理程序的总耗时。一个序列化 JSON 列表、渲染一个 Inertia 页面，或者组装一个 HTML 响应的处理程序，会一直持有这个 future，直到工作完成。
- **流式响应**（`HttpResponse::sse(...)`、`HttpResponse::stream_bytes(...)`）会*立即*带着一个惰性的响应体返回。当 hyper 开始从这个流里拉取字节时，中间件链早就已经完成了，所以这个截止时间永远不会观察到响应体的生命周期。按设计，一个 SSE 事件流可以在一个 30 秒的超时之下，保持打开状态数小时之久 - 关于流式模型，请参见[Server-Sent 事件](sse.md)。
- **WebSocket 升级**会被显式跳过。见下一节。

这正是您几乎肯定想要的行为。如果您把一个长期存活的 SSE 流包在一个 30 秒的超时里，框架就会每 30 秒在流的中途强行断开这个连接一次，这个功能就会没法用。

## WebSocket 豁免

这个中间件会在给这个截止时间上膛之前检查请求：

```rust
if is_websocket_upgrade(request.headers()) {
    return next(request).await;
}
```

任何带着 `Upgrade: websocket` 的请求都会完全跳过这个超时。这个检查在令牌值上是不区分大小写的（`WebSocket`、`websocket`、`WEBSOCKET` 都能匹配），而一个没有 `Upgrade: websocket` 的裸 `Connection: upgrade`*不会*被当作一次 WS 升级 - 它会正常流经这个超时。

今天，WebSocket 升级走的是一条完全不运行全局中间件的独立服务器路径，所以这道守卫是一种深度防御 - 它确保了即使有一天这一点变了，这个超时也永远不会去限定一个长期存活的双向通道。关于升级是如何被分发的，以及一个已连接 socket 的生命周期，请参见 [WebSocket](websockets.md)。

## 到达截止时间时会发生什么

当 `tokio::time::timeout` 在处理程序完成之前流逝，这个中间件会按顺序做三件事：

1. **丢弃这个飞行中的处理程序 future。** 这个 future 本来正在 `timeout` 这个组合子内部被轮询；这个组合子会返回 `Err(Elapsed)`，这个 future 会在它最后一次被挂起的地方被丢弃。
2. **记录一条警告**，带着路由路径和以毫秒计的超时时长：

   ```
   WARN suprnova::timeout request exceeded its timeout; returning 503 Service Unavailable
       route=/report timeout_ms=5000
   ```

   这条日志是 `WARN` 级别的，所以默认情况下它会出现在运维人员的仪表盘上，和正常请求的 `INFO` 级访问日志分开。
3. **返回 `503 Service Unavailable`**，带一个纯文本的响应体：

   ```
   HTTP/1.1 503 Service Unavailable
   Content-Type: text/plain
   Content-Length: 42

   Service Unavailable: request timed out
   ```

这个 503 被包在 `Err(HttpResponse::…)` 里，所以它会像任何其他被中间件拒绝的请求一样，让链的其余部分短路。外层的中间件（日志记录、request-id、CORS）仍然会运行它们处理程序之后的那部分逻辑，所以这个响应会带着正确的响应头发出去。

### 为什么是 503 而不是 504

当*您*是网关、而*上游*超时了的时候，`504 Gateway Timeout` 才是正确的状态码。当*这个*服务自己没能及时产出响应时，`503 Service Unavailable` 才是正确的状态码。这个超时中间件限定的是*我们自己*的处理程序，所以它返回 503。如果您想要一种不同的形态 - 一个 JSON 响应体、一个不同的状态码、一个机器可读的代码 - 请在这个超时外面包一层您自己的中间件，翻译它的 503 响应。

## 取消安全

当这个截止时间流逝时，这个处理程序的 future 会在它当前的 `.await` 点上被**丢弃**。这是正常的 Tokio 取消；当一个客户端在请求中途关闭连接时，发生的也是同一件事。任何跨越这个 await 边界持有的东西，都会通过它的 `Drop` 实现被释放：

- **数据库事务**会回滚。一个 SeaORM 的 `DatabaseTransaction` 有一个 `Drop` 实现，会在底层连接上发出 `ROLLBACK`。
- **Mutex 和 RwLock 守卫**会释放。一个标准库或者 `parking_lot` 的守卫会在丢弃时释放，另一个等待者可以立即拿到它。
- **文件句柄**会关闭。当 `tokio::fs::File` 被丢弃时，操作系统级别的描述符会被释放。
- **网络连接**会归还回连接池，或者被关闭，取决于这个连接池丢弃时的行为。

其结果是，一个超时的处理程序不会留下任何悬空的东西 - 运维人员看到的是这个 503，数据库看到的是这次回滚，下一个请求看到的是一个干净的连接池。

### 什么*不会*被取消

任何您用 `tokio::spawn` 从这个请求上移出去的东西，都是**分离的**。被 spawn 出来的任务活在这个运行时上，而不是这个请求的 future 上，所以丢弃这个请求并不会停止它们。当您写出这样的代码时，这一点就很重要：

```rust
pub async fn webhook(req: Request) -> Response {
    let payload: WebhookPayload = req.json().await?;

    // 即发即忘的后台工作。即使请求超时也能存活下来。
    tokio::spawn(async move {
        if let Err(e) = process_webhook(payload).await {
            tracing::error!("webhook processing failed: {e}");
        }
    });

    Ok(HttpResponse::new().status(204))
}
```

如果请求在 `spawn` 这一行运行*之前*就超时了，这次 spawn 根本不会发生。如果请求在这次 spawn *之后*超时，这个后台任务会继续运行 - 它不会跟着这个请求一起被取消。对于 webhook 风格的工作来说，这几乎总是您想要的行为，但这也意味着，处理程序内部一次长 `.await` 之后的清理工作，是**不**保证会运行的：

```rust
pub async fn upload(req: Request) -> Response {
    let temp_path = save_to_temp(&req).await?;

    // 如果超时的是这一步，下面的清理逻辑不会运行。
    let processed = long_running_processing(&temp_path).await?;

    // 在超时之下不保证运行。
    tokio::fs::remove_file(&temp_path).await?;

    Ok(HttpResponse::json(serde_json::to_value(&processed)?))
}
```

修复办法是用 RAII。把这个临时文件包进一个结构体，让它的 `Drop` 实现去移除它；这样，无论处理程序是正常返回、返回一个错误，还是在中途的 `.await` 上被这个超时丢弃，这个清理逻辑都会运行。这和您为任何取消来源 - 客户端断开、运行时关闭、panic 恢复 - 所应用的纪律是一样的。

## 和 panic 边界的交互

Suprnova 服务器把整条中间件链包在 [`execute_chain_safely`](lifecycle.md) 里，它用 `AssertUnwindSafe(...).catch_unwind()` 把 panic 翻译成一个清理过的 `500 Internal Server Error`。一个超时的请求**不是**一次 panic - 这个 future 是被干净地丢弃的 - 所以这个超时的 `503` 会直接发出去，完全不涉及 panic 边界。

这两道边界处理的是不同的失败模式：

| 失败 | 边界 | 状态码 | 响应体 |
|---|---|---|---|
| 处理程序的 `.await` 超过了截止时间 | `TimeoutMiddleware` | `503` | `Service Unavailable: request timed out` |
| 处理程序发生 panic（对 `None` 调用 `.unwrap()` 等等） | `execute_chain_safely` | `500` | `{"message": "Internal Server Error"}` |
| 处理程序返回 `Err(HttpResponse)` | 正常的 `Response` 流程 | 由处理程序自行设定 | 由处理程序自行设定 |

您不需要二选一 - 这两道边界永远都是同时装好的。一个*先*超过了超时、*然后*才 panic 的处理程序，仍然会产出一个 503（这个 panic 还没来得及发生，future 就已经被丢弃了）。一个*在*超过超时*之前*就 panic 的处理程序，会产出一个 500。

## 运维调优

选择超时值时的三个考量：

1. **匹配您的数据库连接超时。** 如果 `DB_CONNECT_TIMEOUT=30`（默认值），一个比 30s 更短的请求超时，会在一次缓慢的连接真正完成之前就先触发 - 用户看到的是 `503`，而不是一次恢复的机会。要么调高连接超时，要么接受“30s”是这个下限。
2. **考虑最慢的合理处理程序。** 看一看您 `INFO` 级别请求时长的直方图。慢尾部的 p99 应该舒适地落在这个超时值之下，并留有余量应对时钟偏差和事件循环抖动。一个在健康流量上就经常触发的超时，是一个配置错误，不是一个特性。
3. **逐路由的超时是一种可观测性手段。** 在 `/api/*` 上收紧成 `TimeoutMiddleware::seconds(3)`，会把一个正在退化的 API 变成一个可见的告警（日志里满是 WARN，负载均衡器里满是 503），而不是一个悄悄恶化的延迟问题。在您有一个 SLA、并且想在错过它时得到一次硬性失败的地方使用它们。

框架自己的集成测试使用的是毫秒级的时长（`TimeoutMiddleware::new(Duration::from_millis(50))`），以确定性的方式练习这个截止时间。生产环境的截止时间几乎总是整数秒。

### 为什么 Suprnova 有所不同

在一个 Laravel + PHP-FPM 的部署里，请求超时活在应用之外：nginx 的 `proxy_read_timeout`、PHP-FPM 的 `request_terminate_timeout`、负载均衡器的空闲超时。当这个预算耗尽时，这个 PHP 进程会被杀掉，而任何打开的状态 - 数据库连接、文件句柄 - 都会一直泄漏，直到下一个请求复用这个工作进程。

Suprnova 在应用内部限定这个请求，因为它能做到。这个处理程序是一个 Tokio future，不是一个 PHP 进程，所以丢弃它会干净地运行 `Drop` 实现：事务回滚，锁释放，描述符关闭，连接池保持健康。这个 503 也会*作为一个真正的 HTTP 响应*发出去 - 客户端看到的是一个正规的状态码，而不是一次上游的连接重置。

这也是为什么这个中间件不去尝试成为一个 Tower 的 `Timeout` 层。Tower 的这一层对任何 Tokio 服务都是通用的，返回的是 `tower::timeout::error::Elapsed`，调用方随后必须把它映射成一个 HTTP 状态码。Suprnova 的这个中间件知道自己包裹的是一条 HTTP 请求管道；它直接返回 `503`，记录出问题的路由，并且遵守框架的 WebSocket 和流式豁免，而不需要调用方自己去操心这些。对于一个通用的 Tokio 服务来说，Tower 的这一层是正确的原语；对于一次 HTTP 请求，这才是正确的形态。

## 下一步

- [中间件](middleware.md) - 这个 trait、这条链、全局与逐路由的注册、可终止的钩子
- [请求生命周期](lifecycle.md) - 这个超时落在链的哪个位置，以及 `execute_chain_safely` 如何处理 panic
- [Server-Sent 事件](sse.md) - 这个超时刻意不去限定的那个流式响应模型
- [WebSocket](websockets.md) - 完全绕开这个超时的那条升级路径
- [错误处理](errors.md) - 5xx 响应是如何作为 `ErrorOccurred` 事件被分发、以供可观测性使用的
