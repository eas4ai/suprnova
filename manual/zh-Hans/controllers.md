# 控制器

一个 Suprnova 控制器就是一个异步函数。它从请求里取走自己需要的东西 - 类型化的路径参数、一个已经加载好的模型、一份已经验证过的表单 - 然后返回一个 `Response`。这里没有控制器基类，也没有服务定位器的接线文件。函数就是基本单位，而 `#[handler]` 属性把它粘到路由宏上。

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

// GET /users/{user}
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
    })
}
```

这个处理程序的签名一次做了三件事：声明路由参数（`user`）、把对应的行从数据库里取出来，以及在这一行不存在时返回 404。这些都不是手写的。`#[handler]` 会读取参数类型，并生成相应的提取代码。

## 生成一个控制器

```bash
suprnova make:controller User
```

这会写出一个 `src/controllers/user.rs`，里面只有一个 `invoke` 存根，并把 `pub mod user;` 追加到 `src/controllers/mod.rs`。这个存根就是一个最小可用的处理程序：

```rust
//! User controller

use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn invoke(_req: Request) -> Response {
    json_response!({
        "controller": "User"
    })
}
```

您想往这个文件里加多少个函数都可以 - Suprnova 不追踪控制器“类”，只追踪函数。很多应用按资源来拆分（`controllers::user::{index, show, store, update, destroy}`），但框架里没有任何东西强制这么做。

名称会被转换成 `snake_case` 作为文件名：`OrderItem` 会变成 `order_item.rs`。

## `#[handler]` 属性

这个宏会给每个参数的类型分类，并生成与之匹配的提取器。一共四类：

| 参数类型 | 提取方式 | 失败时的行为 |
|---|---|---|
| `Request` | 原样把请求传递过去 | - |
| `i32`、`i64`、`u32`、`u64`、`usize`、`String` | `FromParam` - 解析同名的路由参数 | 解析失败返回 400，缺失也返回 400 |
| `T: AutoRouteBinding`（任何 Eloquent `Model`） | 把参数解析为模型的主键，然后加载对应的行 | 解析失败返回 400，找不到返回 404 |
| 其他任何东西（`T: FromRequest`） | 调用 `T::from_request(req)` - 通常是一个 `#[derive(FormRequest)]` 验证器 | `from_request` 返回什么就是什么；验证错误是 422 |

这个宏会按声明顺序运行这些提取，所以您函数的方法体看到的是完全类型化的值。如果任何一次提取失败，错误会通过 `?` 短路出去，处理程序的方法体根本不会运行。

### 路径参数

```rust
// 路由：get!("/users/{id}", controllers::user::show)
#[handler]
pub async fn show(id: i64) -> Response {
    json_response!({ "user_id": id })
}

// 路由：get!("/posts/{post_id}/comments/{comment_id}", show_comment)
#[handler]
pub async fn show_comment(post_id: i64, comment_id: i64) -> Response {
    json_response!({
        "post_id": post_id,
        "comment_id": comment_id,
    })
}
```

参数名必须与路由占位符一致：`{id}` 要求 `id: …`。参数类型通过 `FromParam` 解析。错误的输入（用 `/users/abc` 去匹配 `id: i64`）会返回 400，并附上一条指明参数名和目标类型的消息。

### 路由模型绑定

`Eloquent` 模型会自动实现 `AutoRouteBinding`。把模型声明为一个参数，框架就会去加载它：

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

// 路由：get!("/users/{user}", controllers::user::show)
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({
        "id": user.id,
        "name": user.name,
        "email": user.email,
    })
}
```

路由占位符的名字（`{user}`）和参数名（`user`）必须一致。框架会把参数字符串解析为模型的主键类型，调用 `Entity::find_by_pk`，并在这一行缺失时返回 404。任何 `#[suprnova::model]` 结构体都会自动绑定；对于不使用 `#[suprnova::model]` 的手写 SeaORM 实体，`route_binding!` 宏依然可用 - 参见[宏](macros.md#route_binding)。

### 表单请求

任何实现了 `FromRequest` 的东西都以同样的方式接进来。常见的情况是一个 `#[derive(FormRequest)]` 结构体，它会验证请求体，并在失败时给出一个以字段为键、携带错误的 422：

```rust
use suprnova::{attrs, handler, json_response, Response};
use crate::models::user;
use crate::requests::UpdateUserRequest;

// 路由：put!("/users/{user}", controllers::user::update)
#[handler]
pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response {
    let id = user.id;
    user.update(attrs! { name: form.name, email: form.email }).await?;
    json_response!({ "updated": id })
}
```

验证器的 derive 和完整的验证流程，请参见[表单请求](requests.md)。

### 当您想要原始的 `Request` 时

如果您更愿意手动去提取 - 或者您需要一个请求头、一个 cookie、一段查询字符串 - 就直接接收 `Request`：

```rust
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;             // 路由参数，缺失时 400
    let ua = req.header("User-Agent");      // Option<&str>
    let page: u32 = req.query_param("page") // Option<String>
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);

    json_response!({ "id": id, "ua": ua, "page": page })
}
```

您可以混着用：`pub async fn nested(category_id: i64, product: product::Model, req: Request)` 就是一个合法的签名。这个宏会按各自的规则去提取每一个参数。

## `Response` 契约

`Response` 是 `Result<HttpResponse, HttpResponse>` 的别名。两个分支承载的是同一个载荷类型，这就是 `?` 在任何地方都能用的原因。中间件链在边界处用一行代码把这个结果收拢：

```rust
result.unwrap_or_else(|e| e)
```

每一个 `?` 传播点依赖的都是这同一份契约。错误在到达这条链之前，会通过 `From<FrameworkError> for HttpResponse` 完成转换 - 完整的图景请参见[错误模型](error-model.md)。

一个处理程序的方法体自上而下地读下来，并用 `?` 提前退出：

```rust
use suprnova::{handler, json_response, Response};
use crate::models::user;

#[handler]
pub async fn show(id: i64) -> Response {
    let user = user::Model::find_or_fail(id).await?;
    let invoices = user.invoices().get().await?;
    json_response!({ "user": user, "invoices": invoices })
}
```

如果 `find_or_fail` 返回 `Err`，函数就以一个 404 退出。如果 `invoices().get()` 出错，您会得到一个 500。没有 `match` 语句，也没有异常处理程序。

## 创建响应

三个宏加一个构建器覆盖了常见的情况：

```rust
use suprnova::{handler, json_response, text_response, HttpResponse, Response, ResponseExt};

#[handler]
pub async fn json_handler() -> Response {
    json_response!({
        "users": [
            {"id": 1, "name": "John"},
            {"id": 2, "name": "Jane"},
        ]
    })
}

#[handler]
pub async fn health() -> Response {
    text_response!("OK")
}

#[handler]
pub async fn store() -> Response {
    // 通过 ResponseExt 提供的内置可链式状态码 / 响应头。
    json_response!({ "id": 1, "created": true }).status(201)
}

#[handler]
pub async fn page() -> Response {
    Ok(HttpResponse::html("<h1>Hello</h1>"))
}
```

`json_response!`、`text_response!` 和 `HttpResponse::*` 产出的都是同一个 `Response` 类型。`ResponseExt` trait 补上了 `.status(...)`、`.header(...)`、`.cookie(...)` 和 `.with_headers(...)`，这样您就可以在一个宏的结果上链式地做配置。

其余的一切 - 文件下载、流式响应体、Inertia 响应、重定向 - 请参见[响应](responses.md)。

## 重定向

`redirect!("route.name")` 会在编译期校验这个路由存在，并返回一个可以链式配置的构建器：

```rust
use suprnova::{handler, redirect, Response};

#[handler]
pub async fn store() -> Response {
    // 创建这个用户…
    redirect!("users.index").into()
}

#[handler]
pub async fn update(id: i64) -> Response {
    redirect!("users.show")
        .with("id", id.to_string())
        .into()
}

#[handler]
pub async fn search() -> Response {
    redirect!("users.index")
        .query("page", "1")
        .query("sort", "name")
        .into()
}
```

`.with(key, value)` 会填上一个路由占位符；`.query(key, value)` 会追加一个查询字符串参数；`.flash(key, value)` 会写入会话的 flash bag，供下一个请求使用。`.into()` 把构建器转换为一个 `Response`。

如果这个具名路由不存在，宏会让编译失败，并列出所有可用的路由名 - 拼写错误在进入预发布环境之前就会浮出水面。

## 由容器注入的服务

用 `App::resolve`（具体类型）或 `App::resolve_make`（trait 对象）从容器里解析服务。两者都返回 `Result<_, FrameworkError>`，所以它们能和 `?` 组合起来用：

```rust
use suprnova::{handler, json_response, App, Response};
use crate::services::UserService;

#[handler]
pub async fn index() -> Response {
    let user_service = App::resolve::<UserService>()?;
    let users = user_service.list_all().await?;
    json_response!({ "users": users })
}
```

如果您用 `#[injectable]` 绑定操作，控制器就是这样调用它们的。操作的形态请参见[操作](actions.md)；完整的容器接口 - 绑定、工厂，以及任务本地 / 线程本地 / 全局的查找级联 - 请参见[服务容器](container.md)。

## 一个完整的 RESTful 控制器实例

```rust
// src/controllers/user.rs
use suprnova::{attrs, handler, json_response, redirect, Response, ResponseExt};
use crate::models::user;
use crate::requests::{StoreUserRequest, UpdateUserRequest};

// GET /users
#[handler]
pub async fn index() -> Response {
    let users = user::Model::all().await?;
    json_response!({ "users": users })
}

// GET /users/{user}
#[handler]
pub async fn show(user: user::Model) -> Response {
    json_response!({ "user": user })
}

// POST /users
#[handler]
pub async fn store(form: StoreUserRequest) -> Response {
    let user = user::Model::create(attrs! {
        name: form.name,
        email: form.email,
    }).await?;
    json_response!({ "user": user }).status(201)
}

// PUT /users/{user}
#[handler]
pub async fn update(user: user::Model, form: UpdateUserRequest) -> Response {
    let id = user.id;
    user.update(attrs! {
        name: form.name,
        email: form.email,
    }).await?;
    json_response!({ "updated": id })
}

// DELETE /users/{user}
#[handler]
pub async fn destroy(user: user::Model) -> Response {
    user.delete().await?;
    redirect!("users.index").into()
}
```

用 `routes!` 宏来注册它们：

```rust
// src/routes.rs
use suprnova::{delete, get, post, put, routes};
use crate::controllers;

routes! {
    get!("/users",           controllers::user::index   ).name("users.index"),
    get!("/users/{user}",    controllers::user::show    ).name("users.show"),
    post!("/users",          controllers::user::store   ).name("users.store"),
    put!("/users/{user}",    controllers::user::update  ).name("users.update"),
    delete!("/users/{user}", controllers::user::destroy ).name("users.destroy"),
}
```

路由占位符 `{user}` 与参数名 `user: user::Model` 相对应，框架就是靠这一点知道该由哪个路径段来加载模型的。

## `Request` API

当您直接接收 `Request` 时，最常用到的方法是这些：

| 方法 | 返回值 | 说明 |
|---|---|---|
| `method()` | `&hyper::Method` | HTTP 方法 |
| `path()` | `&str` | URL 路径 |
| `param(name)` | `Result<&str, ParamError>` | 路由参数；用 `?` 提前退出 |
| `params()` | `&HashMap<String, String>` | 所有路由参数 |
| `query()` | `Option<&str>` | 原始查询字符串 |
| `query_param(key)` | `Option<String>` | 单个查询字符串的值 |
| `query_params()` | `HashMap<String, String>` | 所有查询参数 |
| `query_into::<T>()` | `Result<T, FrameworkError>` | 类型化反序列化 |
| `header(name)` | `Option<&str>` | 单个请求头 |
| `headers()` | `&hyper::HeaderMap` | 完整的请求头映射 |
| `has_header(name)` | `bool` | 存在性检查 |
| `bearer_token()` | `Option<String>` | 解析后的 `Authorization: Bearer …` |
| `cookie(name)` | `Option<String>` | 单个 cookie 的值 |
| `cookies()` | `HashMap<String, String>` | 所有 cookie |
| `ip()` | `Option<String>` | 对端 IP，会考虑 X-Forwarded-For |
| `secure()` | `bool` | HTTPS 检测（包括经过代理的情况） |
| `is_method(m)` | `bool` | 大小写不敏感 |
| `is_inertia()` | `bool` | Inertia 的 XHR 请求头 |
| `ajax()` | `bool` | `X-Requested-With: XMLHttpRequest` |
| `expects_json()` / `wants_json()` | `bool` | 检查 Accept 请求头 |
| `route_name()` | `Option<String>` | 匹配到的路由的 `.name(...)` |
| `json::<T>()` | `Result<T, FrameworkError>` | 把请求体解析为 JSON（会消费掉它） |
| `form::<T>()` | `Result<T, FrameworkError>` | 按 form-urlencoded 解析 |
| `input::<T>()` | `Result<T, FrameworkError>` | 按 content-type 分派的解析 |

这是一套 Laravel 形态的接口 - 这里的每一个方法都对应 Laravel `Request` 类上的一个方法。

## 文件布局

约定如下：

```
src/
├── controllers/
│   ├── mod.rs          # pub mod home; pub mod user; ...
│   ├── home.rs
│   ├── user.rs
│   └── api/
│       ├── mod.rs
│       └── user.rs
├── routes.rs           # routes! { ... }
└── main.rs
```

框架里没有任何东西强制这个布局 - 控制器可以放在任何能从 `routes.rs` 触达的地方。这个约定之所以存在，是因为它就是脚手架生成出来的样子，也因为路由和控制器天然是一对。

## 为什么 Suprnova 有所不同

Laravel 的控制器是继承 `Illuminate\Routing\Controller` 的类。方法是在容器逐请求解析出来的实例上调用的，构造函数注入也就发生在那里。这个模式在 PHP 上没什么问题 - 当整个进程在响应之后就被拆掉时，每个请求都 `new` 一次是很便宜的。

在 Rust 里，那个模式意味着要么 (a) 逐请求分配一个控制器结构体，这会付出一次您并不需要的 `Arc` 克隆，要么 (b) 通过一套并不划算的基类继承体系去重新实现依赖注入。

Suprnova 选了更简单的模型：控制器就是一个自由的异步函数，而“依赖”要么是容器解析（`App::resolve::<Service>()?`），要么是靠类型来完成提取的参数（`form: UpdateUserRequest`）。构造函数注入发生在[操作](actions.md)里的 `#[injectable]` 边界上，那才是它该待的地方。处理程序始终是一个从请求到响应的纯函数，这让隔离测试变得很简单：构造一个 `Request`，调用这个函数，然后对结果做断言。

## 下一步

- [路由](routing.md) - `routes!`、`get!`、`post!` 和 `.name()` 会展开成什么
- [表单请求](requests.md) - 通过 `#[derive(FormRequest)]` 做类型化验证
- [响应](responses.md) - JSON、HTML、文件、流、Inertia 页面、重定向
- [服务容器](container.md) - `App::resolve` 实际做了什么
- [操作](actions.md) - 业务逻辑住在控制器之外的什么地方
- [错误模型](error-model.md) - `?` 是如何把 `FrameworkError` 变成一个响应的
