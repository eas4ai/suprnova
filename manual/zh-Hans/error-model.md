# 错误模型

本章是 Suprnova 错误处理背后的模型 - 类型、转换契约，以及框架免费为您提供的安全保证。关于日常的处理程序模式（`?`、返回错误、构建自定义领域错误），请参见[错误处理](errors.md)；本章解释的是这些模式*为什么*会以这种方式运作。

如果您只记得这一页的一件事：**Suprnova 中的错误是值，不是异常**。每一个错误最终都会通过一次单一的、完全的转换变成一个 `HttpResponse`。这里没有全局异常处理程序，因为这里根本没有全局异常。

## 结构

Suprnova 的错误模型由五个部分组成：

| 类型 | 角色 |
|---|---|
| `Response = Result<HttpResponse, HttpResponse>` | 每个处理程序都满足的契约 - 两个分支都已经是响应 |
| `FrameworkError` | 框架的规范错误枚举；每一条内部错误路径都会产生一个 |
| `AppError` | 无需专门类型、可内联使用的临时领域错误 |
| `HttpError`（trait） | 您自己的类型化领域错误所实现的接口，用以获得状态码 + 消息 |
| `ValidationErrors` | Laravel/Inertia 风格的错误包，用于逐字段的失败 |

这五者全都通过 `From` 实现收拢为单一的 `HttpResponse`。`?` 运算符在调用点完成这次转换；中间件链在请求边界完成它；panic 处理程序在发生栈展开时完成它。所有情况都使用同一种响应体结构，5xx 也都遵循同一条清理规则。

## `Response` 是 `Result<HttpResponse, HttpResponse>`

每个处理程序都返回这个类型：

```rust
pub type Response = Result<HttpResponse, HttpResponse>;
```

两个分支携带相同的负载类型，这正是关键所在。当中间件链执行完您的处理程序后，会用一行代码把结果收拢起来：

```rust
result.unwrap_or_else(|e| e)
```

框架不需要知道您的处理程序“成功”了还是“失败”了 - 两个分支都已经是渲染好的 HTTP 响应。这个区分存在，只是为了让 `?` 能够完成它的工作：

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    // `?` 在遇到 Err 时短路。下面的每一次转换都会通过一个
    // From 实现产生一个 HttpResponse - 这条链会收拢两个分支。
    let id: i64 = req.param("id")?.parse().map_err(|_| {
        suprnova::FrameworkError::param_parse("id", "i64")
    })?;
    let user = User::find_or_fail(id).await?;  // 缺失时返回 404
    Ok(json_response!({ "user": user }))
}
```

这一条单一契约 - 每一条错误路径都会通过 `From` 产生一个 `HttpResponse` - 就是这个模型的核心。本章接下来的内容，讲的都是各个 `From` 实现具体做了什么。

### 为什么 Suprnova 有所不同

Laravel 抛出异常，并将它们路由到注册在 `app/Exceptions/Handler.php` 里的一个全局 `Handler` 类。框架捕获一切，向这个处理程序询问“我该渲染什么？”，然后发出响应。PHP 的异常展开模型让这一切显得很自然。

Rust 在用户代码里没有异常展开。Suprnova 的对应做法是 `From<FrameworkError> for HttpResponse` 实现，加上 `ErrorOccurred` 事件。转换负责渲染；事件则是您接入可观测性（Sentry、PagerDuty、结构化日志转发器）的地方。您不需要注册一个处理程序类 - 转换是一个函数，监听 `ErrorOccurred` 就是扩展点。表面相同，机制不同。

## `FrameworkError` - 规范枚举

框架内部的每一条错误路径 - 提取器、路由绑定、容器、验证、数据库层、存储 - 都会产生一个 `FrameworkError`。它是一个有十四个变体的枚举，每个变体都标注了自己的 HTTP 状态码：

```rust
pub enum FrameworkError {
    ServiceNotFound { type_name: &'static str },        // 500
    ParamError { param_name: String },                   // 400
    ValidationError { field: String, message: String },  // 422
    Database(String),                                    // 500
    Internal { message: String },                        // 500
    Domain { message: String, status_code: u16 },        // *
    Validation(ValidationErrors),                        // 422
    Unauthorized,                                        // 403
    ModelNotFound { model_name: String },                // 404
    ParamParse { param: String, expected_type: &'static str }, // 400
    UnsupportedMediaType,                                // 415
    PrecognitionSuccess,                                 // 204
    PrecognitionFailure(ValidationErrors),               // 422
    AlreadyReported,                                     // 仅 CLI
}
```

您很少需要匹配这个变体。您通过一个便捷构造函数构造出一个变体，然后让 `?` 完成剩下的事：

```rust
use suprnova::FrameworkError;

// 下面这些全都会产生一个带有正确状态码的 FrameworkError：
FrameworkError::not_found("User");                    // → ModelNotFound, 404
FrameworkError::bad_request("Bad input");             // → Domain, 400
FrameworkError::param("user_id");                     // → ParamError, 400
FrameworkError::param_parse("user_id", "i64");        // → ParamParse, 400
FrameworkError::validation("email", "required");      // → ValidationError, 422
FrameworkError::domain("Conflict", 409);              // → Domain, 409
FrameworkError::internal("disk full");                // → Internal, 500
FrameworkError::database("timeout");                  // → Database, 500
```

`FrameworkError` 上没有 `unauthorized()` 或 `forbidden()` 构造函数 - `Unauthorized` 是一个固定的变体，携带 Laravel 的“This action is unauthorized.”消息，状态码为 403，而 401 的情况则要走 `AppError::unauthorized`（下一节）。注意：这个变体名为 `Unauthorized`，但状态码是 403，因为它建模的是 Laravel 的授权拒绝，而不是 HTTP 认证。

### 自动转换

`FrameworkError` 实现了 `From<sea_orm::DbErr>` 和 `From<opendal::Error>`，因此数据库和存储的错误无需包装就能流经 `?`：

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: ActiveModel) -> Result<Model, FrameworkError> {
    // 这里的两次 `?` 调用都会自动转换成 FrameworkError：
    // - DB::get 返回 Result<_, FrameworkError>
    // - insert 返回 Result<_, DbErr>，而它有 From<DbErr> for FrameworkError
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

如果您的代码返回 `Result<_, FrameworkError>`，那么您的依赖所产生的每一种常见错误，都已经说着正确的语言。控制器里的 `?` 除了把一种错误类型转换成另一种之外，不做任何其他工作。

### 包装上下文

当您需要带着操作上下文重新抛出一个错误时，使用 `.context()`：

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

消息会变成 `"creating new user: <original>"`。变体会在重要的地方被保留下来 - `Validation`、`ValidationError`、`PrecognitionFailure`、`Unauthorized`、`ModelNotFound` 和 `ParamParse` 会保留它们的结构，让响应渲染器依然能发出正确的形状。单纯携带消息的变体（`Internal`、`Database`、`Domain`）会被拉平成一个带有前缀消息的 `Domain`。

## `AppError` - 临时领域错误

对于那些您不想为其定义专门类型的一次性错误，使用 `AppError`。它实现了 `HttpError`，并有一个到 `FrameworkError` 的 `From`，所以 `?` 可以直接使用：

```rust
use suprnova::{AppError, Request, Response, json_response};

pub async fn transfer(req: Request) -> Response {
    let amount: i64 = req.param("amount")?.parse()
        .map_err(|_| AppError::bad_request("amount must be a number"))?;

    if amount <= 0 {
        return Err(AppError::unprocessable("amount must be positive").into());
    }

    if amount > 1_000_000 {
        return Err(AppError::forbidden("amount exceeds daily limit").into());
    }

    Ok(json_response!({ "transferred": amount }))
}
```

这些构造函数干净利落地对应到 Laravel 的 `abort($status, $msg)` 形式：

| `AppError::*` | 状态码 |
|---|---|
| `bad_request(msg)` | 400 |
| `unauthorized(msg)` | 401 |
| `forbidden(msg)` | 403 |
| `not_found(msg)` | 404 |
| `conflict(msg)` | 409 |
| `unprocessable(msg)` | 422 |
| `new(msg)` | 500 |
| `.status(code)` | 任意 |

注意 `AppError::unauthorized` 是 **401**（HTTP 认证缺失），而 `FrameworkError::Unauthorized` 是 **403**（授权被拒绝，对应 Laravel 的策略拒绝）。它们的含义不同；请选择与失败情况相匹配的那一个。

## `HttpError` - 自定义类型化错误

当同一个领域错误在很多地方都会出现时，把它建模成一个类型。实现 `HttpError`，转换方式就完全由您决定：

```rust
use suprnova::HttpError;

#[derive(Debug)]
pub struct InsufficientFunds {
    pub available: i64,
    pub requested: i64,
}

impl std::fmt::Display for InsufficientFunds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Insufficient funds: have {}, need {}",
            self.available, self.requested)
    }
}

impl std::error::Error for InsufficientFunds {}

impl HttpError for InsufficientFunds {
    fn status_code(&self) -> u16 { 402 }
    fn error_message(&self) -> String {
        format!("Need {} units, only {} available.",
            self.requested, self.available)
    }
}
```

`HttpError` 有两个方法，都带有默认实现：

```rust
pub trait HttpError: std::error::Error + Send + Sync + 'static {
    fn status_code(&self) -> u16 { 500 }
    fn error_message(&self) -> String { self.to_string() }
}
```

### 桥接到 `?`

一个朴素的 `impl<T: HttpError> From<T> for FrameworkError` 会和已有的 `From<AppError>` 实现冲突（因为 `AppError` 自己也实现了 `HttpError`）。Suprnova 没有这样做，而是用一个显式的桥接构造函数来解决这个孤儿规则问题：

```rust
use suprnova::{FrameworkError, HttpError};

pub async fn debit(account: &mut Account, amount: i64) -> Result<(), FrameworkError> {
    account.withdraw(amount)
        .map_err(FrameworkError::from_http_error)?;
    Ok(())
}
```

状态码和消息取自 `HttpError::status_code` 和 `HttpError::error_message`，并被存储进一个 `FrameworkError::Domain` 变体。之后响应渲染器就会走正常的 `Domain` 路径。

### `#[domain_error]`：无样板代码的类型

如果您想要类型化错误的模式，却不想手写 `Display`、`Error` 和 `HttpError` 的实现，使用 `#[domain_error]` 属性宏：

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFoundError;

#[domain_error(status = 402, message = "Insufficient funds")]
pub struct InsufficientFundsError {
    pub available: i64,
    pub requested: i64,
}
```

`#[domain_error]` 会生成完整的一套实现，*包括* `From<YourError> for FrameworkError`，所以 `?` 可以直接使用，不需要桥接调用：

```rust
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let user = User::find(id).await?
        .ok_or_else(|| FrameworkError::from(UserNotFoundError))?;
    Ok(json_response!({ "user": user }))
}
```

自定义错误方案分为三个层级 - 内联使用的 `AppError`、通过宏生成类型化错误的 `#[domain_error]`、追求完全控制的手写 `HttpError` - 无论您需要多正式的处理方式，都能找到合适的工具。

## `ValidationErrors` - Laravel 风格的错误包

当一个请求验证失败时，Suprnova 会发出 Laravel 和 Inertia 前端所期望的那种 JSON 形状：

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["The email field must be a valid email address."],
        "password": ["The password must be at least 8 characters."]
    },
    "request_id": "8f9e1a2b-c3d4-..."
}
```

您通常不需要手动构建它 - 表单请求上的 `#[derive(Validate)]`，加上它背后的 `validator` crate，会产生一个 `validator::ValidationErrors`，Suprnova 会通过 `ValidationErrors::from_validator` 把它转换过来。但当您需要的时候，这个类型是公开的：

```rust
use suprnova::{FrameworkError, ValidationErrors};

pub async fn after_validation(payload: &Signup) -> Result<(), FrameworkError> {
    let mut errs = ValidationErrors::new();

    if payload.email.ends_with("@example.com") {
        errs.add("email", "example.com addresses are not allowed");
    }
    if payload.password == payload.email {
        errs.add("password", "password must not match email");
    }

    errs.into_result().map_err(FrameworkError::Validation)
}
```

`add_to_bag` 会把错误归入一个具名的错误包（对应 Laravel 的 `withErrors($errors, 'profile')` 形式），做法是在字段名前面加上包名和一个 `.` 分隔符：

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// errors 映射：{ "profile.bio": [...], "billing.card": [...] }
```

`retain_fields` 只保留列出的条目 - 供 Precognition 的 `Precognition-Validate-Only` 请求头在内部使用，这样服务器会运行完整的验证，但只报告客户端所询问的那些字段的错误。

## 转换契约

当一个 `FrameworkError` 到达 HTTP 边界时，它会经过 `From<FrameworkError> for HttpResponse`。按顺序会发生三件事：

1. **状态路由**。变体的 `status_code()` 会被读取一次。
2. **日志记录与可观测性**。5xx 会触发 `tracing::error!` 并分派 `ErrorOccurred`；4xx 会触发 `tracing::warn!`。当作用域内存在 request id 时，两者都会携带它。
3. **响应体渲染**。一个 Laravel 形状的 JSON 响应体，5xx 会经过清理。

### 响应体结构

所有错误响应体都遵循同一个 JSON 骨架：

```json
{
    "message": "<human readable>",
    "errors": { "field": ["msg", ...] },
    "request_id": "<uuid>" | null,
    "debug_message": "<dev only>"
}
```

- `message` 总是存在。
- `errors` 只出现在验证类的错误里（`Validation`、`ValidationError`）- 两者渲染出相同的形状，让消费者只需要解析一条路径。
- `request_id` 总是出现（当处于请求作用域之外时为 `null` - 例如在早期启动阶段，或者在没有请求上下文的测试里）。
- `debug_message` 只在 `APP_DEBUG=true` 时才会出现在 5xx 里。它是纯附加的 - 生产环境的客户端绝不能依赖它。

### 5xx 清理规则

这是值得牢记的那条安全保证。对于任何状态码 ≥ 500 的错误，JSON 响应体的 `message` 都会被替换成这个字面字符串：

```json
{ "message": "Internal Server Error", "request_id": "..." }
```

原始的错误细节**不会**泄漏到响应体里。它会去往：

- `tracing::error!` 的日志条目，带有 request id 和状态码
- `ErrorOccurred` 事件，任何监听器都可以获取到它

当 `APP_DEBUG=true` 时（在 `local`/`dev`/`test` 之外默认为 false），响应还会携带一个带有原始细节的 `debug_message` 字段 - 但在两种模式下 `message` 都保持通用，所以前端和客户端都不会不小心依赖上仅供开发使用的数据。

正是这份契约，让您可以调用 `FrameworkError::internal("db connection refused: password mismatch on user 'app_rw'")`，而不会把密码泄漏到发给客户端的响应里。您传入的这个 `message` 是给阅读日志的运维人员看的；客户端看到的 `message` 是 `"Internal Server Error"`。

对于 4xx 错误，面向调用者的消息会被保留下来 - `404 User not found`、`400 Missing required parameter: user_id`。这些是客户端需要据此采取行动的领域错误，而不是内部故障。

### 契约位于何处

整个转换过程就是一个函数 - `framework/src/http/response.rs` 里的 `impl From<FrameworkError> for HttpResponse`。读一遍它，您就读完了 Suprnova 整个错误渲染表面。没有其他路径。

## Panic 边界

中间件或处理程序里的一次 panic，若非如此，本会沿着每个连接的任务向上传播，在响应过程中拆毁 hyper 服务，让客户端只得到一个 TCP 重置，而没有任何 HTTP 响应。Suprnova 会捕获它。

`framework/src/server.rs` 里的 `execute_chain_safely` 把中间件链包裹在 `AssertUnwindSafe(...).catch_unwind().await` 里。发生 panic 时，它会：

1. 提取 panic 载荷（处理 `&'static str` 和 `String` 载荷；其他任何类型都会表现为 `"panic with non-string payload"`）。
2. 用请求方法、路径和 id 记录一条 `tracing::error!`。
3. 构造 `FrameworkError::internal(format!("request handler panicked: {msg}"))`，并让它经过与其他每一个 5xx 相同的 `From<FrameworkError> for HttpResponse` 转换。
4. 把 request id 作为 `X-Request-Id` 回传回去。

panic 载荷留在日志条目里；客户端得到的是清理过的 `{"message": "Internal Server Error"}` 响应体。为返回的 5xx 错误触发的可观测性监听器，也会为 panic 触发 `ErrorOccurred` - 不需要另外接入一套单独的 panic 事件层面。

同样的 panic 恢复模式也被用于：

- WebSocket 处理程序（`framework/src/server.rs`）
- 计划任务（`framework/src/schedule/mod.rs`）
- 工作流（`framework/src/workflow/mod.rs`）
- `Supervisor` trait（广播）

这些子系统中任何一个发生 panic，都会被记录下来，并要么被转换成一个错误状态，要么被自动重启；它不会拖垮工作任务。

## 通过 `ErrorOccurred` 接入可观测性

`ErrorOccurred` 是框架内置的一个事件，会在每一次 5xx 响应上分派（包括从 panic 合成出来的那些）：

```rust
pub struct ErrorOccurred {
    pub error_message: String,
    pub status_code: u16,
    pub request_id: Option<String>,
}
```

监听它的方式和监听任何其他事件一样：

```rust
use std::sync::Arc;
use suprnova::{ErrorOccurred, EventFacade, FrameworkError, Listener};

pub struct SentryReporter;

#[suprnova::async_trait]
impl Listener<ErrorOccurred> for SentryReporter {
    async fn handle(&self, evt: &ErrorOccurred) -> Result<(), FrameworkError> {
        sentry::capture_message(&evt.error_message, sentry::Level::Error);
        Ok(())
    }
}

// 在 bootstrap.rs 中：
EventFacade::listen::<ErrorOccurred, _>(Arc::new(SentryReporter)).await;
```

这是 Suprnova 对应 Laravel 全局异常处理程序上 `report()` 回调的等价物。这个事件到达时带着原始的、未经清理的 `error_message`（客户端看到的响应体仍然是清理过的）、状态码，以及可用于关联的 request id。

## Abort 辅助函数

三个自由函数会在给定的状态码上让处理程序短路。它们对应 Laravel 的 `abort` / `abort_if` / `abort_unless`：

```rust
use suprnova::{abort_with, abort_if, abort_unless, Auth, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::check(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    Ok(json_response!({ "ok": true }))
}
```

每一个都返回 `Result<(), FrameworkError>`。用 `?` 来使用它们。底层的错误是 `FrameworkError::Domain { message, status_code }`，所以它会经过与其他每一个错误相同的响应体结构和清理规则来渲染。超出范围的状态码会被响应渲染器的状态校验强制转换为 500；您不需要在调用点自己防范错误的输入。

## CLI 哨兵：`AlreadyReported`

`FrameworkError` 的一个变体没有任何 HTTP 含义。`AlreadyReported` 通过 `FrameworkError::silent()` 构造，供控制台调度器在 clap 已经自行格式化并打印过一次参数解析错误时使用。二进制文件的 `main` 会把这个哨兵变体转换成一个非零退出码，而不用 `eprintln`，所以用户永远不会为同一次失败看到两条错误消息。

如果 `AlreadyReported` 真的到达了某个 HTTP 响应转换器，这说明某个请求处理程序意外地返回了 `silent()`。转换器会记录一条醒目的 `tracing::error!` 来指出这次泄漏，并返回一个通用的 500 - 这个变体本就不该出现在请求路径里，而这条醒目的日志能让这个 bug 变得可观测，而不是悄无声息。

您通常不会见到这个变体；之所以在这里记录它，是因为这个枚举是 `HTTP-flavoured` 的，而这个原本没有说明的变体会让任何阅读源码的人感到困惑。

## 安全保证总结

Suprnova 给您的契约：

- **完全转换**。每一个 `FrameworkError` 都会产生一个 `HttpResponse`。没有哪条错误路径会让服务器崩溃，或悄无声息地丢弃连接。
- **清理过的 5xx**。任何 5xx 发给客户端的响应体都是通用的 `{"message": "Internal Server Error", "request_id": "..."}`。细节流向日志和 `ErrorOccurred`。
- **可选的调试可见性**。`APP_DEBUG=true` 会为 5xx 添加一个 `debug_message` 字段，但绝不会影响 `message`。生产环境的客户端不会意外依赖上仅供开发使用的数据。
- **可关联的 request id**。每一个错误响应体都携带 request id（在没有请求作用域时为 `null`）；同一个 id 会同时出现在日志行和 `ErrorOccurred` 事件里。
- **Panic 恢复**。处理程序和中间件里的 panic 会被捕获、记录，并经过与返回错误相同的 `From` 实现路由。不会丢失连接，也不会有可观测性的空白。
- **万物同一种结构**。验证错误、参数错误、panic、自定义领域错误和存储故障，全都会收拢成同一个 JSON 骨架。前端代码只需要解析一种结构。

## 每一部分位于何处

| 部分 | 文件 |
|---|---|
| `FrameworkError`、`AppError`、`HttpError`、`ValidationErrors` | `framework/src/error.rs` |
| `From<FrameworkError> for HttpResponse`（转换 + 清理） | `framework/src/http/response.rs` |
| `abort`、`abort_if`、`abort_unless` | `framework/src/http/abort.rs` |
| `execute_chain_safely`（panic 边界） | `framework/src/server.rs` |
| `ErrorOccurred` 事件 | `framework/src/events/builtins.rs` |
| `#[domain_error]` 宏 | `suprnova-macros/src/domain_error.rs` |

## 下一步

- [错误处理](errors.md) - 使用这个模型的实际处理程序模式
- [请求生命周期](lifecycle.md) - 错误转换在请求流程的哪个环节运行
- [验证](validation.md) - `#[derive(Validate)]`、表单请求，以及 `ValidationErrors` 是如何被填充的
- [响应](responses.md) - `HttpResponse` 构建器、请求头、cookie、流式传输
- [事件](events.md) - 监听 `ErrorOccurred` 和其他内置事件
