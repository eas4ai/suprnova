# 错误处理

本章是在 Suprnova 的处理程序、服务和中间件里编写可失败代码的日常模式指南。至于底层的模型 - 转换契约、panic 边界、5xx 清理规则、可观测性钩子 - 请阅读[错误模型](error-model.md)。本章展示的是您实际要敲下的东西。

要记住的形态是：

- 处理程序返回 `Response = Result<HttpResponse, HttpResponse>`。
- `?` 运算符会把 `FrameworkError`、`AppError`、`DbErr`、`ParamError`、`ValidationErrors`，以及任何类型化的 `HttpError` 自动收拢成一个 `HttpResponse`。
- 三个自由辅助函数（`abort_with`、`abort_if`、`abort_unless`）让您可以在某个状态码上短路，而不必点名任何错误类型。

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;          // 缺失时返回 400
    let user = find_user(id).await?;    // DbErr 时返回 500，Option::None 时返回 404
    json_response!({ "user": user })
}
```

本章接下来的部分是一份错误产生方的清单 - 该构造什么、它返回什么状态码、客户端看到什么形状。

## `?` 就是那次转换

处理程序方法体里的每一个 `?` 都会运行 `From<E> for HttpResponse`。框架把这些实现接好了，所以您实际调用的那些东西返回的错误，本来就知道该怎么渲染自己。您不用写转换；您只写失败。

```rust
use suprnova::{DB, FrameworkError, Request, Response, json_response};
use sea_orm::EntityTrait;

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = users::Entity::find_by_id(id)
        .one(&*DB::get()?)
        .await?
        .ok_or_else(|| FrameworkError::not_found("User"))?;

    json_response!({ "user": user })
}
```

这段代码里发生了三件事 - 没有一件是看得见的：

1. `req.param("id")?` → `ParamError` → `FrameworkError::ParamError`（400）。
2. 一次 SeaORM 调用上的 `.await?` → `DbErr` → `FrameworkError::Database`（500，发给客户端的响应经过清理）。
3. `.ok_or_else(...)?` 会直接构造出一个 `FrameworkError::ModelNotFound`（404）。

这三者都会经过[错误模型](error-model.md)里描述的同一个 `From<FrameworkError> for HttpResponse` 实现。

## `AppError` - 内联的领域错误

对于那些不值得为其定义专门类型的一次性错误，使用 `AppError`。它的构造函数对应到 Laravel 的 `abort($status, $msg)` 形式：

| 构造函数 | 状态码 |
|---|---|
| `AppError::new(msg)` | 500 |
| `AppError::bad_request(msg)` | 400 |
| `AppError::unauthorized(msg)` | 401 |
| `AppError::forbidden(msg)` | 403 |
| `AppError::not_found(msg)` | 404 |
| `AppError::conflict(msg)` | 409 |
| `AppError::unprocessable(msg)` | 422 |
| `AppError::new(msg).status(code)` | 任意 |

`AppError` 有一个到 `FrameworkError` 的 `From`，所以 `?` 可以直接用，不需要任何仪式：

```rust
use suprnova::{AppError, Request, Response, json_response};

pub async fn transfer(req: Request) -> Response {
    let amount: i64 = req.param("amount")?.parse()
        .map_err(|_| AppError::bad_request("amount must be a number"))?;

    if amount <= 0 {
        return Err(AppError::unprocessable("amount must be positive").into());
    }

    if amount > balance() {
        return Err(AppError::forbidden("amount exceeds daily limit").into());
    }

    json_response!({ "transferred": amount })
}
```

注意这里的不对称：`AppError::unauthorized` 是 **401**（缺少认证凭据），而 `FrameworkError::Unauthorized` 是 **403**（策略拒绝了一个已认证的用户）。它们的含义不同；请选择与失败情况相匹配的那一个。

## `FrameworkError` - 规范枚举

内部提取器、容器、路由绑定、验证、数据库层和存储，全都会产生 `FrameworkError`。您通常通过一个便捷构造函数构造出一个，然后让 `?` 去路由它。

```rust
use suprnova::FrameworkError;

FrameworkError::not_found("User");                    // 404
FrameworkError::bad_request("Bad input");             // 400
FrameworkError::param("user_id");                     // 400
FrameworkError::param_parse("user_id", "i64");        // 400
FrameworkError::validation("email", "required");      // 422
FrameworkError::domain("Conflict", 409);              // 409（任意状态码）
FrameworkError::internal("disk full");                // 500
FrameworkError::database("timeout");                  // 500
FrameworkError::service_not_found::<MyService>();     // 500
FrameworkError::model_not_found("Post");              // 404
```

完整的变体集合，连同它们对响应形状的影响，都在[错误模型](error-model.md)里。上面这些构造函数覆盖了每一种常见情形；只有当您要对收到的错误做匹配时，才需要直接用到那些变体。

### 自动转换

`FrameworkError` 已经会说您的依赖所发出的那些方言了。下面这两个 `?` 都会自动转换：

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: users::ActiveModel)
    -> Result<users::Model, FrameworkError>
{
    // DB::get 返回 Result<_, FrameworkError>。
    // .insert 返回 Result<_, DbErr>，而它有 From<DbErr> for FrameworkError。
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

框架还为存储操作实现了 `From<opendal::Error>`，为路径参数的提取实现了 `From<ParamError>`。

### 带上下文重新抛出

当您想标注一个错误来自哪里、又不想丢掉状态码时，使用 `.context()`：

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

消息会变成 `"creating new user: <original>"`。结构化的变体（`Validation`、`ValidationError`、`ModelNotFound`、`ParamParse`、`PrecognitionFailure`、`Unauthorized`）会保留自己的变体，让响应渲染器依然发出正确的形状；单纯携带消息的扁平变体（`Internal`、`Database`、`Domain`）会被拉平成一个 `Domain`，带上加了前缀的消息，并保留原来的状态码。

### 把重复键错误变成 422

`Unique` 验证规则会在写入之前运行一次 `SELECT COUNT(*)`，所以它是建议性的 - 两个并发请求可以双双通过，然后双双尝试插入。落败的那个请求会拿到一次数据库唯一约束冲突，若不加处理就会以 500 的形式泄漏出去。`from_unique_violation` 会把它翻译成那条建议性规则本来会产生的同一个 422：

```rust
use suprnova::FrameworkError;

let user = new_user.insert(db).await.map_err(|e| {
    FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    )
})?;
```

如果底层的 `DbErr` 不是一次唯一约束冲突，它就会作为 500 类的 `Database` 错误原样透传。后端的覆盖范围就是 SeaORM 的 `DbErr::sql_err` 所能识别的范围 - Postgres、MySQL/MariaDB 和 SQLite 都会把各自的重复键错误映射过来。

## 自定义领域错误

分为三个层级，取决于这个错误需要多强的复用性。

### 类型化的情形用 `#[domain_error]`

大多数可复用的错误想要的是一个名字、一个固定的状态码和一个固定的消息模板 - 而不是每次调用都不同的消息。`#[domain_error]` 属性宏会一次性生成 `Display`、`std::error::Error`、`HttpError`，以及针对 `FrameworkError` 的 `From`：

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFound;

#[domain_error(status = 402, message = "Insufficient funds")]
pub struct InsufficientFunds {
    pub available: i64,
    pub requested: i64,
}
```

在调用点用 `?` 来使用它们：

```rust
use crate::errors::user_not_found::UserNotFound;

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = find_user(id).await
        .ok_or_else(|| FrameworkError::from(UserNotFound))?;

    json_response!({ "user": user })
}
```

这个宏会在编译期明确地拒绝格式错误的属性 - 溢出的状态码（`status = 70_000`）、错误的字面量类型（`message = 42`）、未知的键 - 所以您不会因为一个拼写错误就悄无声息地拿到错误的状态码。

#### 用 CLI 生成一份脚手架

```bash
suprnova make:error UserNotFound
```

会写出一个 `src/errors/user_not_found.rs`，带有默认的 `status = 500` 和一条推断出来的句首大写消息，并更新 `src/errors/mod.rs` 把它重导出。`status` 和 `message` 随您口味修改。

### 手写的情形用 `HttpError`

当一个领域错误需要在消息里带上运行时状态（比如这次失败牵涉到的那些 id）时，就直接实现 `HttpError`。这个 trait 有两个方法，都带有合理的默认实现：

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

要把一个手写的 `HttpError` 桥接到 `?` 上，请调用 `FrameworkError::from_http_error`。一个一揽子的 `From<T: HttpError> for FrameworkError` 会和已有的 `From<AppError>` 实现冲突，所以这个桥接是一个显式的构造函数：

```rust
account.withdraw(amount)
    .map_err(FrameworkError::from_http_error)?;
```

### 用错误枚举承载单个模块的失败

当一个服务有好几种相关的失败时，把它们归到一个枚举里，然后为整个枚举写一个 `From`：

```rust
use suprnova::FrameworkError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OrderError {
    #[error("Order {0} not found")]
    NotFound(i64),

    #[error("Insufficient stock for product {product_id}")]
    InsufficientStock { product_id: i64 },

    #[error("Payment failed: {0}")]
    PaymentFailed(String),

    #[error("Order already shipped")]
    AlreadyShipped,
}

impl From<OrderError> for FrameworkError {
    fn from(err: OrderError) -> Self {
        let status = match &err {
            OrderError::NotFound(_) => 404,
            OrderError::InsufficientStock { .. } => 422,
            OrderError::PaymentFailed(_) => 402,
            OrderError::AlreadyShipped => 409,
        };
        FrameworkError::Domain {
            message: err.to_string(),
            status_code: status,
        }
    }
}
```

只要这个 `From` 存在，这个枚举就能像其他任何错误类型一样穿过 `?`。

## `abort_with` / `abort_if` / `abort_unless`

三个辅助函数会在某个状态码上让处理程序短路。它们对应 Laravel 的 `abort` / `abort_if` / `abort_unless`。（这个自由函数以 `abort_with` 而不是 `abort` 的名字导出，好把 `abort` 留给用户类型当方法名用。）

```rust
use suprnova::{abort_if, abort_unless, abort_with, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;

    json_response!({ "ok": true })
}
```

每一个都返回 `Result<(), FrameworkError>`，所以由 `?` 来干活。底层的错误是 `FrameworkError::Domain { message, status_code }`，它会走和其他每一个错误相同的响应体结构来渲染。超出范围的状态码会被响应渲染器强制转换为 500；您不需要在调用点自己防范错误的输入。

## `ValidationErrors` - Laravel 风格的错误包

当验证失败时 - 无论是在 `#[derive(Validate)]` 那一步，还是在一个 `after_validation` 的方法体里 - 框架都会发出 Laravel 和 Inertia 前端所期望的那种 JSON 形状：

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["The email field must be a valid email address."],
        "password": ["The password field must be at least 8 characters."]
    },
    "request_id": "8f9e1a2b-c3d4-..."
}
```

大多数时候您不需要直接构造它 - `#[derive(Validate)]` 跑起来之后，框架会替您转换 `validator::ValidationErrors`。当您需要以命令式的方式添加错误时（跨字段规则，或者作为 `Unique` 补充的异步唯一性检查），构建一个 `ValidationErrors` 并把它返回：

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

`add_to_bag` 会把一个字段归入一个具名的错误包（对应 Laravel 的 `withErrors($errors, 'profile')` 形式），做法是在字段名前面加上包名和一个 `.` 分隔符。当一个响应要携带来自多个子表单、而这些子表单又无法共享一个扁平命名空间的错误时，它就很有用：

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// errors 映射：{ "profile.bio": [...], "billing.card": [...] }
```

`from_validator(ve)` 会转换一个 `validator::ValidationErrors`；`retain_fields(&keep)` 会返回一份只包含所列条目的副本（供 Precognition 的 `Precognition-Validate-Only` 请求头在内部使用）。

## 通过 `ErrorOccurred` 接入可观测性

每一个 5xx 响应都会触发一个 `ErrorOccurred` 事件 - 包括那些从 panic 合成出来的。监听它的方式和监听任何其他事件一样：

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
// `listen` 会从监听器类型推断出两个泛型参数。它返回
// `()`（注册不可能失败），所以既不需要 `?` 也不需要 Result。
EventFacade::listen::<ErrorOccurred, SentryReporter>(Arc::new(SentryReporter)).await;
```

这个事件携带着原始的错误消息（发给客户端的响应体仍然是清理过的 - 参见[错误模型](error-model.md)）、状态码，以及可用于关联的 request id。这是 Suprnova 对应 Laravel 异常处理程序上 `report()` 回调的等价物。

## 您会经常写到的模式

### 把路径参数解析成类型化的值

```rust
let id: i64 = req.param("id")?.parse()
    .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
```

`ParamError` 本来就会转换成 400；`param_parse` 是它在解析失败情形下的对应物，渲染出同样的形状。

### 按 ID 查找，缺失时返回 404

```rust
let user = users::Entity::find_by_id(id)
    .one(&*DB::get()?)
    .await
    .map_err(FrameworkError::from)?
    .ok_or_else(|| FrameworkError::not_found("User"))?;
```

`map_err(FrameworkError::from)?` 会把 SeaORM 的 `DbErr` 先经由 `From<DbErr> for FrameworkError`、再经由 `From<FrameworkError> for HttpResponse` 桥接过去。Rust 不会跨两跳自动串联 `From` 实现，所以这个显式的 `.map_err` 是必需的。

或者，用 Eloquent 那一层（它已经包装了 SeaORM，直接返回 `Result<_, FrameworkError>`）：

```rust
use suprnova::Model;

let user = User::find_or_fail(id).await?;
```

`find_or_fail` 就是把 `find(id).ok_or(ModelNotFound)` 打包好的样子。

### 对一次操作做授权

```rust
let user = Auth::user().await?
    .ok_or_else(|| AppError::unauthorized("login required"))?;
abort_unless(post.owner_id == user.id() || user.is_admin(), 403,
    "you don't own this post")?;
```

`abort_unless` 返回 `Result<(), FrameworkError>`；`?` 会把它收拢回您处理程序的错误分支。

### 返回类型化错误的服务

```rust
use suprnova::{App, FrameworkError, injectable};

#[injectable]
pub struct UserService;

impl UserService {
    pub async fn find_by_email(&self, email: &str)
        -> Result<users::Model, FrameworkError>
    {
        users::Entity::find()
            .filter(users::Column::Email.eq(email))
            .one(&*DB::get()?)
            .await?
            .ok_or_else(|| FrameworkError::not_found("User"))
    }
}

// 调用点：
pub async fn show(req: Request) -> Response {
    let email = req.param("email")?;
    let user = App::resolve::<UserService>()?
        .find_by_email(email)
        .await?;
    json_response!({ "user": user })
}
```

`App::resolve::<UserService>()?` 返回 `Result<Arc<UserService>, FrameworkError>`。链式的 `?` 会把解析失败和查找失败双双收拢成一个响应。

## 速查表

| 您想要… | 就用 |
|---|---|
| 带状态码的内联错误 | `AppError::bad_request("…")` 及其同类 |
| 可复用的类型化错误 | `#[domain_error(status = …, message = "…")]` |
| 生成出来的脚手架 | `suprnova make:error UserNotFound` |
| 带运行时状态的手写错误 | `impl HttpError for MyError` |
| 把手写的错误桥接到 `?` | `FrameworkError::from_http_error(e)` |
| 在某个状态码上短路 | `abort_with` / `abort_if` / `abort_unless` |
| 模型缺失时返回 404 | `FrameworkError::not_found("User")` / `Model::find_or_fail` |
| 路径参数解析失败 | `FrameworkError::param_parse("id", "i64")` |
| 字段级的验证错误 | `FrameworkError::validation("email", "…")` |
| 多字段的错误包 | `ValidationErrors::new().add(…)` + `Validation(errs)` |
| 重复键冲突 → 422 | `FrameworkError::from_unique_violation(field, msg, e)` |
| 标注一个已有的错误 | `err.context("creating user")` |
| 观测每一个 5xx | 监听 `ErrorOccurred` |

## 下一步

- [错误模型](error-model.md) - 变体、转换契约、5xx 清理、panic 边界
- [验证](validation.md) - `#[derive(Validate)]`、表单请求，以及 `after_validation`
- [响应](responses.md) - `HttpResponse` 构建器、状态码、响应头
- [事件](events.md) - 监听 `ErrorOccurred` 和其他内置事件
- [请求生命周期](lifecycle.md) - 错误转换在请求流程的哪个环节运行
