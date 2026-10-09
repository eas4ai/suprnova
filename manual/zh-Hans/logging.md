# 日志

Suprnova 通过 [`tracing`](https://docs.rs/tracing) 来记录日志 - 每一行日志都是一个带字段的结构化事件，而不是一个格式化好的字符串。启动时会安装一个订阅者，它从环境里读取 `LOG_LEVEL` 和 `LOG_FORMAT`，在开发环境里发出美观的多行输出、在生产环境里每行发出一个 JSON 对象，并把一个逐请求的 id 传播进处理程序发出的每一个事件。

本章讲的是日志表面本身：订阅者、格式、级别，以及那个让生产日志可以被检索的 request id 关联。关于 OpenTelemetry 桥接和查询日志，请参见[可观测性](observability.md)；关于发出方可以和这个 id 一起读取的那个请求 `Context` 包，请参见[上下文](context.md)。

## 什么被记录到哪里

默认有两路输出：

| 位置 | 格式 | 何时 |
|---|---|---|
| `stdout` | `LogFormat::Pretty` - 多行、带颜色、对人友好 | 开发环境（`APP_ENV` 是 `local`、`dev`、`testing`、…） |
| `stdout` | `LogFormat::Json` - 每行一个 JSON 对象 | 生产环境（`APP_ENV=production` / `prod`） |

开发 / 生产的默认值是通过 `Environment::detect()` 从 `APP_ENV` 算出来的。用 `LOG_FORMAT=pretty` 或者 `LOG_FORMAT=json` 覆盖它，就能显式强制其中一种。

```env
# .env（开发）
LOG_LEVEL=info,sqlx=warn
LOG_FORMAT=pretty   # 可选；这就是开发环境的默认值

# .env.production
LOG_LEVEL=info,sqlx=warn,suprnova::queue=debug
LOG_FORMAT=json     # 可选；这就是生产环境的默认值
```

框架只往 `stdout` 写。在生产环境里，把您的容器运行时、systemd journal 或者日志聚合器对准它（`docker logs`、`kubectl logs`、`journalctl -u my-app`，一个 Loki/Vector agent，等等）。这里没有轮转的文件写入器 - 让平台去掌管日志的持久化。

## 发出事件

在处理程序、作业、中间件里，随便哪里，都可以用 `tracing` 的宏：

```rust
use suprnova::{json_response, session, Request, Response};
use tracing::{debug, info, warn, error, instrument};

pub async fn checkout(_req: Request) -> Response {
    let user_id: i64 = session()
        .and_then(|s| s.get::<i64>("user_id"))
        .unwrap_or(0);

    info!(user_id, "checkout starting");

    let order = place_order(user_id).await.map_err(|e| {
        error!(user_id, error = %e, "checkout failed");
        e
    })?;

    info!(user_id, order_id = order.id, total = order.total_cents, "checkout succeeded");

    json_response!(order)
}
```

每个字段在 JSON 输出里都会成为一个顶层的键，在 pretty 输出里则是一对带颜色的 `field=value`。优先用字段而不是字符串插值 - 字段在 JSON 日志里是可检索的，而且格式化器会做类型感知的渲染。

要把一个函数包进一个 span，并给它内部的每一个事件都打上共享的字段，请使用 `#[instrument]`：

```rust
#[instrument(skip(db), fields(user_id = %user_id))]
pub async fn load_dashboard(
    db: &suprnova::DatabaseConnection,
    user_id: i64,
) -> Result<Dashboard, FrameworkError> {
    info!("loading"); // 自动从这个 span 里带上 user_id
    // … 各种查询 …
}
```

当启用了 `otel` 这个 feature 时，同一个 `#[instrument]` 会变成一个 OpenTelemetry span - 参见[可观测性](observability.md#opentelemetry)。

## 日志级别

`LOG_LEVEL` 是一条 [`tracing-subscriber` 的 env-filter 指令](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html)，而不是单独一个级别。它的语法是用逗号分隔的 `target=level` 对，其中不带 target 的裸值用来设定默认级别：

```env
LOG_LEVEL=info                                  # 一切都在 info 及以上
LOG_LEVEL=debug                                 # 一切都在 debug 及以上
LOG_LEVEL=info,sqlx=warn                        # 默认 info，sqlx 更安静
LOG_LEVEL=warn,suprnova::queue=debug,my_app=info  # 默认 warn，两个 target 更啰嗦
```

target 通常就是发出事件的 crate 或模块路径（`suprnova::queue`、`hyper::server`、`my_app::services::checkout`）。想找到某个 target，读一读 JSON 日志行 - 每个事件上的 `target` 字段就是它的过滤键。

按啰嗦程度递增排列的级别：`error` < `warn` < `info`（默认）< `debug` < `trace`。无论级别是什么，发给客户端的错误响应总是被清理成 `{"message": "Internal Server Error"}` - 细节只会去往结构化日志。

### 无效的指令不会让启动崩溃

一个格式错误的 `LOG_LEVEL`（比如 `LOG_LEVEL=app=notalevel`）会兜底回 `"info"`，并往 `stderr` 写一行警告：

```text
suprnova: invalid LOG_LEVEL directive "app=notalevel" (...); falling back to "info". Fix LOG_LEVEL to silence this.
```

这里用的是 `stderr` 而不是 `tracing::warn!`，因为那时订阅者还没有被安装 - 一次 `warn!` 会被悄无声息地丢掉。把这条指令改对，警告就消失了。

## Pretty 输出与 JSON 输出

同一句 `info!(user_id = 42, "saved")` 在两种格式下渲染得不一样。

**Pretty（开发）：**

```text
  2026-05-30T22:14:08.221341Z  INFO request{request_id=78a9...} my_app::handlers::checkout: saved
    at src/handlers/checkout.rs:48
    in checkout
    in request with request_id: 78a9..., method: POST, path: /checkout
```

**JSON（生产）：**

```json
{
  "timestamp": "2026-05-30T22:14:08.221341Z",
  "level": "INFO",
  "fields": { "message": "saved", "user_id": 42 },
  "target": "my_app::handlers::checkout",
  "span": { "name": "checkout" },
  "spans": [
    { "name": "request", "request_id": "78a9...", "method": "POST", "path": "/checkout" }
  ]
}
```

这种 JSON 形状正是生产环境的聚合器（Datadog、Loki、Honeycomb、CloudWatch、…）开箱即用就能解析的。`span.request_id` 就是那个关联键 - 见下文。

## 逐请求 id 的关联

每一个 HTTP 请求都会从 `RequestIdMiddleware` 拿到一个 `RequestId`，那是每条链上最外层的中间件。这个 id 会：

- 从一个安全的入站 `X-Request-Id` 请求头里被**复用**（字母数字外加 `- _ . :`，最长 128 字节），或者在它缺失 / 不安全时被**重新铸造**成一个 UUID v4。
- 作为 `X-Request-Id` 在响应上被**回传**（2xx 和 5xx 两种情形都有）。
- 被**纳入**一个名为 `request` 的 `tracing` span 的作用域，这样来自任何中间件、处理程序或下游库的每一个事件，都会自动在自己的 `spans` 数组里带上 `request_id`。
- 以 `_request_id` 的名字被**播种**进请求的 `Context` 包，这样那些想要裸字符串的发出方（作业、广播载荷、错误报告）就能按名字读到它。

在代码里用 `current_request_id()` 读取它：

```rust
use suprnova::current_request_id;
use tracing::info;

if let Some(id) = current_request_id() {
    info!(request_id = %id, "checkpoint reached");
}
```

`current_request_id()` 返回 `Option<RequestId>`，因为后台工作（作业、计划任务、没有安装这个中间件的测试）跑在任何请求作用域之外。

### 后台任务：带着这个 id 一起 spawn

`tokio::spawn` 会启动一个任务本地状态为空的全新任务 - 一个 spawn 出副作用工作的处理程序会丢掉 `current_request_id()`，它的日志事件也就成了孤儿。请改用 `spawn_with_request_id`：

```rust
use suprnova::spawn_with_request_id;
use tracing::info;

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    let order = place_order().await?;

    spawn_with_request_id(async move {
        // 这个任务仍然能观察到 current_request_id()。
        // 它的日志事件带着和处理程序相同的 request_id。
        info!(order_id = order.id, "post-checkout fanout running");
        send_receipt(order.id).await;
        update_analytics(order.id).await;
    });

    suprnova::Response::ok().json(&order)
}
```

这个辅助函数会同时传播 `RequestId` 这个任务本地值和当前的 `tracing::Span`，所以被 spawn 出来的 future，它的事件在日志里会嵌套在同一个 `request` span 之下。在活跃的请求作用域之外，它会退化成一个裸的 `tokio::spawn` - 可以无条件放心使用。

只有 request id 和 tracing span 会跟着这个任务走 - 请求的 `Context` 包故意不跟着，因为后台工作并不是在服务那个发起它的 HTTP 请求。

## 订阅者

框架会在启动时从 `Server::run()` 里安装一个全局的 `tracing` 订阅者。您几乎永远不需要自己调用它；之所以把它写进文档，是因为测试、嵌入方，以及一些不寻常的入口点有时候需要。

```rust
use suprnova::{LogConfig, init_subscriber};

// 从环境里读取 LOG_LEVEL / LOG_FORMAT：
init_subscriber(LogConfig::from_env());

// 或者用编程的方式：
init_subscriber(LogConfig {
    level: "info,sqlx=warn".to_string(),
    format: suprnova::LogFormat::Json,
});
```

`init_subscriber` 是**幂等的**。第二次调用会让已有的订阅者留在原处，并发出一条 `tracing::warn!`，好让运维人员看到新的 `LogConfig` 没有被应用。正是这一点让那些各自调用 `init_subscriber` 的测试不会互相竞争 - 第一个胜出，其余的都是空操作。

如果要那个能感知 OTel 的版本（同样的 `LogConfig`，再加上分布式追踪的导出），请使用 [`init_telemetry`](observability.md#opentelemetry)。

### 那些守护进程

`queue:work`、`schedule:work`、`schedule:run` 和 `workflow:work` 是您应用二进制文件的子命令，它们不经由 `Server::run()` 启动，所以会在起来的路上安装自己的订阅者。它们读取的 `LOG_LEVEL` 和 `LOG_FORMAT` 与服务器相同，而您自己什么都不用调用：

```bash
LOG_LEVEL=info,suprnova::queue=debug cargo run --bin my-app -- queue:work

# …或者，在容器里，针对已经构建好的二进制文件：
LOG_LEVEL=info my-app queue:work
```

在 0.9.1 之前，这条路径根本什么都没有安装。守护进程发出的每一行 `tracing::` 都无处可去，`LOG_LEVEL` 对它们也毫无作用，于是在容器里，启动横幅成了唯一的输出 - 一个正在把作业转入死信的工作进程、一个因为输掉选举而跳过一个节拍的调度器，以及一把它释放不掉的锁，看上去全都和一个闲置的进程一模一样。如果您跑的是一个早于 0.9.1 的固定版本，又在纳闷为什么工作进程一声不吭，那就是原因，而修复办法是升级，而不是改配置。

一个工作进程要说的大部分话，都是在 `warn!` 和 `error!` 上说的 - 一个耗尽了尝试次数的作业、一条它没能持久化的死信、一把它释放不掉的锁 - 所以默认的 `info` 级别就足以看见麻烦。当您还需要那些更安静的决策时，再降到 `debug`。

## 测试

测试不需要安装订阅者 - `#[suprnova_test]` 属性和 `TestContainer::fake` 搭起的机制，已经足够让处理程序的事件流动起来。如果您想对日志输出做断言，请通过 `tracing-subscriber` 的 [`tracing_subscriber::fmt::TestWriter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/struct.TestWriter.html) 或者一个自定义的层来捕获；框架故意没有提供一个“把这个测试里的所有日志都捕获下来”的假实现，因为标准的 `tracing-subscriber` 测试模式本来就能干净地工作。

## 为什么 Suprnova 有所不同

Laravel 用的是 [Monolog](https://github.com/Seldaek/monolog) - 消息字符串加上可选的上下文数组、日志通道，以及逐通道的处理程序（文件、syslog、Slack、…）。PHP 那种每请求一个进程的模型意味着，一个单一的全局静态 logger 是安全的：每个请求都有自己的进程和自己的上下文。

Rust 的进程模型恰好相反 - 一个进程在许多线程上服务着许多并发请求。一个全局的字符串格式化器会在上下文上产生竞争，并且要求在每一个调用点都显式地为 `request_id` 接线。`tracing` 用结构化字段和任务本地的 span 同时解决了这两点：不用接线，字段保持类型化，而且关联是自动的，因为在这条链发出的每一个事件处，那个 request span 都在作用域之内。

只输出到 `stdout` 同样是有意为之。在容器化的部署里（Suprnova 唯一的交付方式），掌管日志持久化的是运行时而不是应用 - 文件轮转、保留期和转发全都属于平台。

## 下一步

- [可观测性](observability.md) - OpenTelemetry、查询日志，以及完整的运维表面
- [上下文](context.md) - 那个逐请求的包，`_request_id` 和其他上下文字段就住在里面
- [错误处理](errors.md) - 框架的 panic 边界和 5xx 路径如何发出它们自己的结构化事件
- [环境变量](env-vars.md) - `LOG_LEVEL`、`LOG_FORMAT` 参考
