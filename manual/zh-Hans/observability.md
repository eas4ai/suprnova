# 可观测性

框架内置了三层运营者可见的信号：结构化日志（始终开启）、逐请求 id 关联（始终开启，并会传播进被 spawn 出的任务），以及一个可选启用的 OpenTelemetry 桥接，它会把您发出的每一个 `tracing` span 都转换成一个导出的 OTel span。您为本地日志写下的同一个 `#[tracing::instrument]`，在 OTel 这个 feature 打开时就会变成一个分布式追踪的 span - 不需要第二套埋点 API。

```rust
use suprnova::telemetry::{init_telemetry, OtelConfig};
use suprnova::logging::LogConfig;

#[suprnova::main]
async fn main() {
    let guard = init_telemetry(LogConfig::from_env(), OtelConfig::from_env());

    // … 运行应用 …

    // 在退出前刷写缓冲的遥测数据：OTel 的批处理器会把 span/指标/日志缓冲在内存里，不调用 `shutdown` 就丢弃这个 guard，会丢失还没有被导出的一切。
    guard.shutdown().await;
}
```

一个脚手架应用的 `Server` 已经替您调用了 `init_telemetry`，并在关闭信号到达时刷写这个 守卫 - 只有在把 Suprnova 嵌入您自己的运行时里时，您才需要手动接好这一步。

## 三层信号

| 层 | 始终开启 | 它给您什么 |
|---|---|---|
| 结构化日志（`tracing`） | 是 | 以 `pretty`（开发）或 `json`（生产）格式输出到 stdout 的日志，能感知环境 |
| 请求 id 关联 | 是 | 通过 `tokio::task_local!` 作用域化的逐请求 id，在 `X-Request-Id` 上回传，并传播进 `spawn_with_request_id` 任务 |
| OpenTelemetry 导出 | `otel` feature + 采集端点 | 追踪、指标和日志的 OTLP HTTP/proto 导出；双向的 W3C `traceparent` 传播 |

OTel 这一层是**编译期可选启用的**，所以默认构建不携带任何 OpenTelemetry 依赖，[`Metrics`](#指标) 这个门面也会编译成惰性的空操作。这个 feature 关闭时，“追踪”和“指标导出”会悄无声息地变成空操作 - 您的日志依然照常工作。

### 为什么 Suprnova 有所不同

Laravel 的可观测性故事分裂在框架内事件（`QueryExecuted`、`MessageSent`、`JobProcessed`）与委托给 PHP 扩展（OpenTelemetry、Sentry、New Relic）、在 FPM 层挂载的运行时关切之间。事件表面很丰富；运行时表面则是“安装您的 APM 供应商需要的那个扩展”。

Suprnova 是单个异步进程，所以两半它都拥有。事件表面是对等的（同样的 `QueryExecuted`/`NotificationSent`/`ErrorOccurred` 形态），运行时表面则是框架内部一条 `tracing` → OpenTelemetry 的桥接。您不需要安装扩展；您翻转一个 feature 标志，您已经在发出的那些 span 就会变成 OTel 导出的。

## 结构化日志

`LogConfig::from_env()` 会读取两个环境变量：

| 变量 | 默认值 | 说明 |
|---|---|---|
| `LOG_LEVEL` | `"info"` | `tracing-subscriber` 的 env-filter 语法（例如 `"debug,sqlx=warn,hyper=warn"`） |
| `LOG_FORMAT` | 能感知环境 | 生产环境下是 `"json"`，其他地方都是 `"pretty"`；显式值总是胜出 |

这个格式默认值是通过 `Environment::detect()` 从 `APP_ENV` 探测出来的：默认情况下，一次生产部署会得到每行一个 JSON 对象、面向日志聚合器的输出，本地/开发环境的运行会得到人类可读的多行输出。如果您想在生产环境里得到原始的 stdout，显式的 `LOG_FORMAT=pretty` 会覆盖生产默认值。

```bash
# 本地开发 - 显式覆盖胜出
LOG_LEVEL=debug,sqlx=warn,hyper=warn LOG_FORMAT=pretty cargo run

# 生产环境 - APP_ENV=production 把格式默认值翻转成 json
APP_ENV=production LOG_LEVEL=info cargo run --release
```

一条格式错误的 `LOG_LEVEL` 指令不会让启动崩溃 - 它会兜底回 `"info"`，并往 stderr 打印一行警告，让这个配置错误运营者可见。

### 每一行里的 span 上下文

每一个被路由的 HTTP 请求都跑在一个由框架最外层中间件创建的 `request` span 里面。这个 span 携带三个字段 - `request_id`、`method`、`path` - JSON 格式化器会把它们嵌套在请求内部发出的每一个事件的 `span` 键下面。您的应用代码不需要在每一行里读取或记录这个 id；这个 span 隐式地携带着它：

```rust
use tracing::info;

pub async fn show(req: suprnova::Request) -> suprnova::Response {
    info!(user_id = 42, "loaded dashboard");
    // JSON 行携带着 span.request_id / span.method / span.path，
    // 调用点不需要穿针引线地传入任何东西。
    Ok(suprnova::json_response!({ "ok": true }))
}
```

## 请求 id 关联

每一个请求都会得到一个 36 字符的小写 UUID v4 id，通过一个 `tokio::task_local!` 作用域化。当入站的 `X-Request-Id` 请求头的值通过一次严格的安全检查（ASCII 字母数字外加 `-_.:`，最长 128 字节）时，中间件会复用它；任何在这个字符集之外的值都会被拒绝并替换成一个全新的 UUID，这样攻击者就不能往日志输出里注入控制字符，也不能撑爆下游的处理管道。

同一个 id 会在**每一个**响应上 - 成功、错误，以及 panic 恢复 - 以 `X-Request-Id` 请求头的形式被回传，这样前端或者上游服务就能把它写进 bug 报告，运营者也能在结构化日志里 grep 到它。

### 读取这个 id

```rust
use suprnova::{current_request_id, spawn_with_request_id};

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    // 在一个请求内部，这个 id 总是存在的。
    let id = current_request_id().expect("inside a request");
    tracing::info!(request_id = %id, "checkout starting");

    // 从一个处理程序里 spawn 出来的后台工作。`tokio::spawn` 会启动一个
    // 任务本地状态为空的任务 - 被 spawn 出的 future 如果没有帮助就会丢掉
    // 这个请求 id。`spawn_with_request_id` 会捕获调用方的 id，把它重新
    // 作用域化给这个被 spawn 出的 future，并附上当前的 `tracing` span，
    // 这样这个任务的事件就会像请求内的事件一样继承 `request_id`。
    spawn_with_request_id(async move {
        // 这条日志行携带着发起方那个请求的 id。
        tracing::info!("post-checkout fanout running");
    });

    Ok(suprnova::ok!())
}
```

`current_request_id()` 在请求之外会返回 `None` - 后台作业、计划任务，以及没有装上这个中间件的测试都看不到 id，这个辅助函数也不会凭空造一个。在请求作用域之外，`spawn_with_request_id` 就完全等价于 `tokio::spawn`；不会发生任何神奇的事情。

### 这个 id 还能在哪些地方拿到

| 表面 | 方式 |
|---|---|
| `tracing` 事件 | 请求内部每一行上的 `span.request_id` |
| 响应头 | 成功、错误，以及 panic 恢复响应上的 `X-Request-Id` |
| `Context` 包 | `Context::get("_request_id")` - 可以从观察者、监听器，以及会查阅 `Context` 的作业里读取 |
| 被 spawn 出的任务 | `spawn_with_request_id` 之后的 `current_request_id()` |

## 面向可观测性的内置事件

框架会在运营者通常想要埋点的那些点上，分发带类型的事件。每一个都是一个 `suprnova::Event`，您可以通过 `EventFacade::listen::<E, _>(...)` 来 `listen`，并发往 Sentry、Datadog、Slack，或者您自己的指标管道。它们全都经过 `dispatch_best_effort` 运行，所以一个失败的监听器不会破坏触发它的那个请求。

| 事件 | 何时触发 | 携带什么 |
|---|---|---|
| `ErrorOccurred` | 任何 `FrameworkError` → 5xx 的转换（包括 panic 恢复） | 错误上下文 + 请求 id |
| `QueryExecuted` | 每一个经过装配了埋点的执行器助手方法路由的查询 | sql、绑定参数、耗时、连接、读/写分类、结果 |
| `ConnectionEstablished` | `DbConnection::connect` 成功 | 连接名 |
| `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` | 闭包形式的 `DB::transaction` + 手动句柄 | 连接名 |
| `NotificationSending` / `NotificationSent` / `NotificationFailed` | `Notification::send` 的逐通道前置/后置/错误 | 通知 + 通道 + 收件人 |

`ErrorOccurred` 是发往 5xx 异常上报的钩子；`QueryExecuted` 是慢查询告警的钩子；这三个通知事件是投递看板的钩子。监听器 API 参见[事件](events.md)，每个事件在请求路径里的哪个位置触发参见[生命周期](lifecycle.md)。

### 直接的数据库查询观测

`DB::listen` 是专门为 `QueryExecuted` 量身定制的第二个同步钩子。它在执行器内部原地触发，所以一个慢的监听器会拖慢这次查询 - 请让它保持轻量。分发器路径（`EventFacade::listen::<QueryExecuted, _>`）是逐个全跑、尽力而为，并且能容忍错误；对任何可能失败的东西，请优先选用它。

```rust
use suprnova::DB;

// 在 bootstrap.rs 里：
DB::listen(|q| {
    if q.time > std::time::Duration::from_millis(100) {
        tracing::warn!(
            sql = %q.sql,
            ms = q.time.as_millis(),
            "slow query"
        );
    }
})?;
```

一个自己会发出数据库查询的监听器，**不会**为这次嵌套调用重新触发 `QueryExecuted` - 一个任务本地的重入防护会阻止“记录到数据库的监听器 → 发出事件 → 记录到数据库 → …”这样的循环。

### 为测试/调试捕获查询日志

面向测试断言，或者一次性的“这个代码块里跑了什么？”式调试：

```rust
use suprnova::DB;

DB::enable_query_log()?;
// … 运行您想检视的代码 …
let queries = DB::get_query_log()?;
for q in &queries {
    println!("{:>4}ms  {}", q.time.as_millis(), q.to_raw_sql());
}
DB::disable_query_log()?;
DB::flush_query_log()?;
```

这个缓冲区是**无界的** - 每一次捕获到的查询都会让它变大。请把它用在测试和一次性调查上；如果您在生产环境里一直开着它，请定期刷写。

## 分布式追踪（OTel）

添加 `otel` 这个 feature 来选择启用：

```toml
[dependencies]
suprnova = { git = "...", features = ["otel"] }
```

通过标准的 OTel 环境变量来配置：

```bash
# 最低限度：采集器住在哪里。
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
OTEL_SERVICE_NAME=my-app          # 默认为 "suprnova"
OTEL_SERVICE_VERSION=1.4.2        # 默认为您 crate 的版本
```

遥测只有在设置了 `OTEL_EXPORTER_OTLP_ENDPOINT`**并且**总开关 `OTEL_SDK_DISABLED` 没有打开时才会被**启用**。没有端点时，只有日志层单独运行，返回的 守卫 也不持有任何 provider，所以不调用 `shutdown()` 就丢弃它是静默的（不会在每一个测试进程上都发出一条多余的“缓冲的遥测数据可能会丢失”警告）。

### 追踪上下文自动汇入

**入站方向。** 当一个请求携带着一个 W3C [`traceparent`](https://www.w3.org/TR/trace-context/) 请求头到达时 - 也就是说它是由另一个被追踪的服务发起的 - 中间件会提取这份上下文，并把这个请求 span 重新挂到调用方那个 span 之下作为父级。您的服务器 span 会作为子级出现在*同一个*分布式追踪里，而不是一个全新的根。一个没有 `traceparent` 的请求（浏览器的直接访问）会保持一个干净的根 span。

**出站方向。** 框架的 HTTP 客户端（[`Http`](http-client.md)）会把当前活跃的追踪上下文以 `traceparent` 的形式注入到每一次出站调用上，这样下游服务就能延续同一个追踪。

合在一起：`upstream service → your handler → downstream service` 是一个互相连接的追踪，您的处理程序里不需要任何手动的 span 接线。

**错误状态。** 当一个处理程序返回一个 5xx 时，这个请求 span 会被标记为出错，这样 OTel 后端就会显示 `Status::Error`。（一次处理程序*panic*会被捕获并转换成一个带 error 级别日志和一个 `ErrorOccurred` 事件的 500，但这条路径上不会设置 OTel 的 span 状态 - panic 会在那个标记运行之前就展开这个 span 的 future。）

### 添加您自己的 span

因为这条桥接会把每一个 `tracing` span 都转换成一个 OTel span，您只用普通的 `tracing` 打埋点 - 您的代码里不需要任何 OTel 专属的 API：

```rust
use suprnova::DatabaseConnection;

#[tracing::instrument(skip(db))]
async fn load_dashboard(db: &DatabaseConnection, user_id: i64) -> anyhow::Result<()> {
    // 这个 span 会自动嵌套在这个请求 span 之下，并在 `otel` 这个
    // feature 打开时导出到您的采集器。
    Ok(())
}
```

### Suprnova 读取的环境变量

| 变量 | 效果 |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | 采集器的基础 URL。未设置 → 遥测被禁用。 |
| `OTEL_SERVICE_NAME` | `service.name` 这个资源属性（默认为 `"suprnova"`）。 |
| `OTEL_SERVICE_VERSION` | `service.version` 这个资源属性（默认：crate 的版本）。 |
| `OTEL_SDK_DISABLED` | 总开关。不区分大小写的 `true` 或 `1` 会禁用导出，即便设置了端点也一样。 |

其余那些标准的 OTLP 旋钮由 SDK 自己读取，所以按通常的方式配置它们：

| 变量 | 由谁读取 |
|---|---|
| `OTEL_EXPORTER_OTLP_HEADERS` | 导出器（采集器鉴权，例如 `Authorization=Bearer ...`） |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | 导出器（`http/protobuf` 等） |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | 导出器 |
| `OTEL_EXPORTER_OTLP_COMPRESSION` | 导出器 |

按信号区分的端点覆盖项（`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`、`_METRICS_ENDPOINT`、`_LOGS_ENDPOINT`）目前会被基础端点遮蔽 - 全部三种信号都会发往 `OTEL_EXPORTER_OTLP_ENDPOINT`。如果您需要把不同的信号分流到不同的采集器，请运行一个负责路由它们的本地采集器。

## 指标

`Metrics` 是计数器、直方图和仪表的门面。这些 handle 克隆起来很轻，并且会在每次构造时解析全局的 meter：

```rust
use suprnova::telemetry::Metrics;

// 计数器 - 单调递增。
let signups = Metrics::counter("user.signups");
signups.inc();                                  // +1
signups.inc_by(3);                              // +3
signups.inc_with(&[("plan", "pro")]);           // +1，带一个标签

// 直方图 - 分布（延迟、大小）。
let latency = Metrics::histogram("request.latency_ms");
latency.record(42.0);
latency.record_with(42.0, &[("route", "/checkout")]);

// 仪表 - 某一时刻的值。
let queue_depth = Metrics::gauge("jobs.pending");
queue_depth.set(17.0);
queue_depth.set_with(17.0, &[("queue", "emails")]);
```

没有 `otel` 这个 feature 时，上面的每一次调用都是零分配的空操作 - 把埋点留在热路径里，在默认构建里不用付出任何代价。

指标 handle 会绑定到不管哪个在这个底层度量仪首次被解析时活跃的 meter provider。请在 `init_telemetry` 运行**之后**创建 handle（或者延迟到首次使用时才创建) - 一个在初始化之前构造的 handle 会解析到那个空操作 provider 上，并一直保持惰性。惯用的模式是一个在启动之后很久、首次发出时才解析的 `once_cell` / `LazyLock` handle。

属性值是字符串类型的（`&[(&'static str, &str)]`）。数值和布尔类型的属性是一项计划中的增强；现在请在调用点把它们格式化成字符串。

命名规范：稳定、ASCII、点号分隔（例如 `"http.requests.total"`、`"http.request.duration"`）。标准的 OTel 语义约定就活在 `opentelemetry-semantic-conventions::metric::*` 里。

## 关闭契约

`init_telemetry` 会返回一个拥有 SDK provider handle 的 `TelemetryGuard`。OTel 的批处理器会在内存里缓冲 span / 指标 / 日志，并异步刷写它们，所以您必须在进程退出之前调用 `guard.shutdown().await`，否则您会丢失仍然缓冲着的一切。

- 调用 `shutdown()` 会刷写，并且调用一次是安全的（它拿走了 `self`）。
- **不**调用 `shutdown()` 就丢弃这个 守卫 会记录一条警告 - 但只有在这个 守卫 确实持有 provider 时才会。一次遥测被禁用的运行（没有端点，或者 `OTEL_SDK_DISABLED`，或者一个非 `otel` 的构建）会拿回一个不持有 provider 的 守卫，它的丢弃是静默的，所以没有采集器的开发和测试运行不会被刷屏。

## 总结

| 任务 | API |
|---|---|
| 启用 OTel | `features = ["otel"]` + `OTEL_EXPORTER_OTLP_ENDPOINT` |
| 初始化 | `init_telemetry(LogConfig::from_env(), OtelConfig::from_env())` |
| 退出时刷写 | `guard.shutdown().await` |
| 运行时禁用 | `OTEL_SDK_DISABLED=true` |
| 自定义 span | `#[tracing::instrument]`（自动桥接到 OTel） |
| 计数器 / 直方图 / 仪表 | `Metrics::counter/histogram/gauge(name)` |
| 分布式追踪的汇入 | 自动完成 - 提取入站的 `traceparent`，注入出站的 |
| 读取当前请求 id | `current_request_id()` |
| 把 id 传播进 spawn | `spawn_with_request_id(future)` |
| 同步的查询观察者 | `DB::listen(|q| { ... })` |
| 尽力而为的查询观察者 | `EventFacade::listen::<QueryExecuted, _>(...)` |
| 为测试捕获查询 | `DB::enable_query_log()` → `DB::get_query_log()` |

## 下一步

- [事件](events.md) - 监听器 API、分发模式，测试用的 `EventFacade::fake()`
- [生命周期](lifecycle.md) - 每个事件在请求路径里的哪个位置触发，请求 span 又是在哪里构造的
- [错误处理](errors.md) - `ErrorOccurred`、`HttpError`，经过清理的 5xx 响应体
- [数据库](database.md) - `QueryExecuted`、`DB::transaction`，那些触发这些事件的执行器助手方法
- [HTTP 客户端](http-client.md) - 闭合这个分布式追踪循环的出站 `traceparent` 注入
