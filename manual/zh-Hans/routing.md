# 路由

路由是 Suprnova 将一个入站 HTTP 请求转化为一次处理程序调用的方式。您使用 `routes!` 宏在 `src/routes.rs` 中声明路由（或者手动构建一个 `Router`），然后 `Server::from_config` 接过这个路由器，并在进程的整个生命周期内运行它。形态与 Laravel 的 `routes/web.php` 相同，只是用 Rust 类型取代了门面。

```rust
// src/routes.rs
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
    put!("/users/{id}", controllers::users::update).name("users.update"),
    delete!("/users/{id}", controllers::users::destroy).name("users.destroy"),
}
```

这个宏会展开为 `pub fn register() -> Router { ... }`。从您的 bootstrap 中调用它，并把结果交给服务器。

## HTTP 动词

每个动词一个宏。全部七个都接受一个“路径-处理程序”对，并返回一个构建器，您可以在其上链式调用 `.name(...)` 和 `.middleware(...)`。

| 宏 | 方法 | 用途 |
|---|---|---|
| `get!`     | GET     | 读取端点、静态页面 |
| `post!`    | POST    | 创建资源 |
| `put!`     | PUT     | 完整替换式更新 |
| `patch!`   | PATCH   | 部分更新（RFC 5789） |
| `delete!`  | DELETE  | 删除 |
| `head!`    | HEAD    | 仅头信息探测（未显式注册时，HEAD 会按照 RFC 9110 § 9.3.2 回退到 GET 注册表） |
| `options!` | OPTIONS | 能力发现、`Accept-Patch`。CORS 预检由 `CorsMiddleware` 在到达路由器之前处理，所以您通常不需要这一个 |

```rust
use suprnova::{routes, get, post, patch, delete};

routes! {
    get!("/articles", controllers::articles::index),
    post!("/articles", controllers::articles::store),
    patch!("/articles/{id}", controllers::articles::update),
    delete!("/articles/{id}", controllers::articles::destroy),
}
```

每个动词宏都会在编译期检查路径是否以 `/` 开头 - 缺少开头的斜杠会让构建失败，而不是让请求失败。

### 多方法与 `any!`

`any!` 会把一个处理程序注册到全部七个常见动词上。把它用于 webhook 接收端点，以及其他需要接受任意 HTTP 方法的端点。

```rust
use suprnova::{routes, any};

routes! {
    any!("/webhooks/inbound", controllers::webhooks::inbound)
        .name("webhooks.inbound")
        .middleware(SignatureCheck),
}
```

当您只想让一部分动词共享同一个处理程序时，改用构建器 API 和 `Router::methods`：

```rust
use suprnova::Router;
use hyper::Method;

let router = Router::new()
    .methods(&[Method::PUT, Method::PATCH], "/posts/{id}", update_post)
    .name("posts.update")
    .middleware(AuthMiddleware);
```

`.name(...)` 和 `.middleware(...)` 会作用到这个路由注册所涉及的每一个动词上，所以无论调用方按哪个方法做反向查找，得到的 URL 都是同一个。

### WebSocket 路由

`ws!` 注册一个长连接的升级处理程序。这个宏是同一个 `routes!` 主体的一部分 - 详见 [WebSocket](websockets.md)。

## 路由参数

动态段使用花括号（`{id}`）。为了照顾熟悉度，Suprnova 也接受 Express/Rails 风格的冒号（`:id`），并会在把模式交给 `matchit` 之前将其规范化为花括号。

```rust
routes! {
    get!("/users/{id}", controllers::users::show),       // matchit 原生写法
    get!("/users/:id", controllers::users::show),        // Express/Rails - 同一回事
    get!("/posts/{post_id}/comments/{comment_id}", controllers::comments::show),
}
```

冒号只有出现在路径段开头时才会被当作参数的起始符，所以段中间的字面冒号会原样保留（`/files/note:draft` 仍然是一个字面路由，而不是 `/files/{draft}`）。

在处理程序里从请求中读取参数：

```rust
use suprnova::{Request, Response, HttpResponse};

pub async fn show(req: Request) -> Response {
    let user_id = req.param("id").unwrap_or("0");
    Ok(HttpResponse::text(format!("User ID: {}", user_id)))
}
```

如果想要类型化提取而不必写 `unwrap_or` 这套操作，请参见下面的路由模型绑定，或者 [控制器](controllers.md) 里的 `#[handler]`。

## 路由模型绑定

当一个处理程序的参数是 SeaORM 的 `*::Model` 类型时，`#[handler]` 会提取匹配的路径参数，把它解析为主键类型，并从数据库中获取对应的行。缺失的行会产生 404；一个主键类型无法解析的参数会产生 400。

```rust
use suprnova::{handler, json_response, Response};
use crate::models::users;

// 路由：GET /users/{user}
#[handler]
pub async fn show(user: users::Model) -> Response {
    json_response!({ "name": user.name, "email": user.email })
}
```

参数名（`user`）就是 `#[handler]` 用来在匹配到的路由参数里查找的键 - 所以占位符必须一致（是 `/users/{user}`，而不是 `/users/{id}`）。

一个函数签名里绑定多个模型的方式相同；可以把它们与表单请求、原始类型或 `Request` 混用：

```rust
// 路由：PUT /posts/{post}/comments/{comment}
#[handler]
pub async fn update(
    post: posts::Model,
    comment: comments::Model,
    form: UpdateCommentRequest,
) -> Response {
    // post 和 comment 已经被取到；form 已经过验证。
    json_response!({ "post_id": post.id, "comment_id": comment.id })
}
```

### 要求

绑定对任何满足以下两个条件的 SeaORM 模型都是自动的：其 `Entity` 实现了 `suprnova::database::EntityExt`，且其主键类型实现了 `FromStr`。`EntityExt` 那一整套对通用实现友好的附加 trait，为您提供了 `Entity::find_by_pk(id)`、`::all()`、`::first()` 等方法；路由模型绑定，说到底就是由路径参数驱动的 `find_by_pk`。

```rust
// src/models/users.rs（旧版 SeaORM 风格布局）
pub use super::entities::users::*;
use sea_orm::entity::prelude::*;

impl ActiveModelBehavior for ActiveModel {}

// 启用路由模型绑定（以及 Laravel 风格的读取器接口）。
impl suprnova::database::EntityExt for Entity {}
impl suprnova::database::EntityExtMut for Entity {}
```

如果您的模型是用 `#[suprnova::model]` 宏声明的（[Eloquent](eloquent.md) 里的 Eloquent 接口），可以直接使用它：`User::find_by_pk(id).await?`。通过 `#[handler]` 做的路由模型绑定仍然期望 `*::Model` 这个形状 - 传入 SeaORM 模型类型，而不是包装结构体。

### 绑定是身份，不是授权

路由模型绑定回答的是“这一行存在吗？” - 它**不会**回答“当前用户是否被允许查看这一行？”。一个裸的绑定处理程序会让任何已认证用户通过猜测 `/posts/N` 来查看任意文章。请针对绑定到的模型使用 `Gate::authorize` 或 `#[policy]` 宏来做授权 - 参见 [授权](authorization.md)。

### 选择退出

不要使用 `*::Model` 参数类型。手动提取 ID 并查询：

```rust
use suprnova::{handler, json_response, Response, FrameworkError};
use crate::models::users;
use suprnova::database::EntityExt;

#[handler]
pub async fn show(id: i32) -> Response {
    let user = users::Entity::find_by_pk(id)
        .await?
        .ok_or(FrameworkError::not_found("User"))?;
    json_response!({ "id": user.id, "name": user.name })
}
```

## 命名路由

名称给您提供了用于生成 URL 的稳定标识符。用 `.name(...)` 附加一个：

```rust
routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users", controllers::users::index).name("users.index"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

名称遵循 Laravel 的 `<resource>.<action>` 约定 - `users.show`、`posts.destroy`、`admin.dashboard`。用顶层的 `route(name, &[...])` 辅助函数来查找它们：

```rust
use suprnova::route;

let home = route("home", &[]);
//   Some("/")

let profile = route("users.show", &[("id", "123")]);
//   Some("/users/123")
```

`route` 返回 `Option<String>`，并把参数值百分号编码为路径安全的形式（所以 `("slug", "a/b")` 会变成 `/posts/a%2Fb` - 对 `matchit` 是安全的，并且能通过 `req.param("slug")` 原样往返）。对于重定向目标和邮件链接，请使用更严格的兄弟函数 `suprnova::routing::try_route`，它返回 `Result<String, RouteUrlError>`，并且拒绝生成包含未填充的 `{placeholder}` 段的 URL。完整的 URL 接口（签名 URL、绝对 URL、`Redirect::route`）请参见 [URL 生成](urls.md)。

路由名称是全局唯一的，并且是进程级全局的。把同一个名称注册给两个不同的路径会在启动时 panic - 静默的遮蔽曾经是一个安全形状的 bug，因为重定向会路由到无论哪个注册碰巧获胜的那个。使用 `RouteBuilder::try_name`（或 `suprnova::routing::try_register_route_name`）来获得可失败的版本。

## 逐路由中间件

在任意路由构建器上链式调用 `.middleware(M)`：

```rust
use suprnova::{routes, get, post};
use crate::middleware::{AuthMiddleware, AdminMiddleware};

routes! {
    // 公开
    get!("/", controllers::home::index).name("home"),

    // 受保护
    get!("/dashboard", controllers::dashboard::index)
        .name("dashboard")
        .middleware(AuthMiddleware),

    // 多个中间件按从左到右的顺序组合（最外层的先写）
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware)
        .middleware(AdminMiddleware),
}
```

路由本地的中间件会在任何全局中间件（`Server::with_middleware`）和任何包裹该路由的组中间件之后运行。中间件映射是按 `(method, path)` 建立索引的，所以把认证附加到 `POST /api/posts` 上永远不会波及同一路径上公开的 `GET /api/posts`。关于中间件契约以及如何编写自己的中间件，请参见 [中间件](middleware.md)。

## 路由分组

`group!` 用于抽取共享的路径前缀和/或共享的中间件：

```rust
use suprnova::{routes, get, post, group};
use crate::middleware::{AuthMiddleware, ApiMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // 共享 /api 前缀 + 中间件
    group!("/api", {
        get!("/users", controllers::api::users::index).name("api.users.index"),
        post!("/users", controllers::api::users::store).name("api.users.store"),
        get!("/users/{id}", controllers::api::users::show).name("api.users.show"),
    }).middleware(ApiMiddleware),

    // 管理区域
    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard).name("admin.dashboard"),
        get!("/settings", controllers::admin::settings).name("admin.settings"),
    }).middleware(AuthMiddleware),
}
```

一个组前缀会和每个路由路径拼接在一起。组内路径为 `/` 的路由会精确解析为组前缀本身（`group!("/users", { get!("/", index) })` → `GET /users`）。

### 嵌套分组

分组可以嵌套到任意深度。前缀会拼接；中间件会从父级继承到子级：

```rust
routes! {
    group!("/api", {
        get!("/health", controllers::api::health),

        group!("/v1", {
            get!("/users", controllers::api::v1::users),

            group!("/admin", {
                get!("/stats", controllers::admin::stats),
            }).middleware(AdminMiddleware),
        }),
    }).middleware(AuthMiddleware),
}
```

| 路由 | 有效路径 | 中间件链 |
|---|---|---|
| `/api/health` | `/api/health` | `AuthMiddleware` |
| `/api/v1/users` | `/api/v1/users` | `AuthMiddleware` |
| `/api/v1/admin/stats` | `/api/v1/admin/stats` | `AuthMiddleware` → `AdminMiddleware` |

对于嵌套分组内的单个路由，执行顺序是**最外层的中间件先运行**：父分组 → 子分组 → 路由本地。逐路由的 `.middleware(...)` 运行在最内层。

## 兜底路由

`fallback!` 注册一个在没有其他路由匹配时运行的处理程序。把它用于自定义 404 页面。

```rust
use suprnova::{routes, get, fallback};

routes! {
    get!("/", controllers::home::index),

    fallback!(controllers::errors::not_found),
}
```

```rust
// src/controllers/errors.rs
use suprnova::{Request, Response, HttpResponse};

pub async fn not_found(req: Request) -> Response {
    Ok(HttpResponse::text(format!("Page not found: {}", req.path()))
        .status(404))
}
```

兜底路由支持自己的中间件链（`fallback!(handler).middleware(M)`）。如果没有注册兜底路由，框架会返回一个纯文本的 `404 Not Found`。

## 资源路由

对于一个标准的七动作 REST 接口，实现 `ResourceController` 并通过 `Router` 构建器注册这个资源。这是 Laravel `Route::resource()` 和 `Route::apiResource()` 的对应物。

```rust
use suprnova::{Router, ResourceController, ResourceAction, Request, Response, HttpResponse};
use std::pin::Pin;
use std::future::Future;

struct PostsCtl;

impl ResourceController for PostsCtl {
    fn index(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("list")) })
    }
    fn show(&self, _req: Request) -> Pin<Box<dyn Future<Output = Response> + Send>> {
        Box::pin(async { Ok(HttpResponse::text("one")) })
    }
    // store / update / destroy / create / edit 默认返回 404。
}

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .into();
```

您没有覆盖的方法会返回 404。使用 `api_resource` 来去掉 `create` 和 `edit` - 这两个路由的唯一作用是渲染表单。

### 默认路由与名称

| 动词 | 路径 | Trait 方法 | 名称 |
|---|---|---|---|
| GET    | `/posts`             | `index`   | `posts.index`   |
| GET    | `/posts/create`      | `create`  | `posts.create`  |
| POST   | `/posts`             | `store`   | `posts.store`   |
| GET    | `/posts/{post}`      | `show`    | `posts.show`    |
| GET    | `/posts/{post}/edit` | `edit`    | `posts.edit`    |
| PUT    | `/posts/{post}`      | `update`  | `posts.update`  |
| DELETE | `/posts/{post}`      | `destroy` | `posts.destroy` |

路径参数默认为资源名称的单数形式 - `posts` → `{post}`，`categories` → `{category}`。不规则复数会取字面上的最后一段；可以用 `.parameter(...)` 覆盖。

### 限制与重命名

```rust
use suprnova::{Router, ResourceAction};

Router::new()
    .resource("posts", PostsCtl)
    .only(&[ResourceAction::Index, ResourceAction::Show])      // 固定到两个动词
    .names([("index", "posts.list")])                          // 重命名一个默认值
    .parameter("post_id")                                      // {post} → {post_id}
    .into();
```

Rust 侧有一些在某些调用点读起来更顺的别名：`.keep(...)` 对应 `.only(...)`，`.drop(...)` 对应 `.except(...)`，`.rename(...)` 对应 `.names(...)`。

### 批量注册

```rust
Router::new()
    .resources([
        ("posts",    Box::new(PostsCtl)    as Box<dyn ResourceController>),
        ("comments", Box::new(CommentsCtl) as Box<dyn ResourceController>),
    ])
    .api_resources([("authors", Box::new(AuthorsCtl) as Box<dyn ResourceController>)]);
```

### 为整个资源做授权

`authorize_resource::<U, R>()` 会把常规的能力检查作为逐路由中间件附加到每一个生成的路由上 - 这是 Laravel `authorizeResource` 的对应物。没有它，一个资源接口就是不受控的，除非每一个控制器方法体都记得调用 `Gate::authorize`；哪怕只忘记一个 `destroy`，也会上线一个不受控的删除操作。

```rust
use suprnova::{Router, Gate};

// 能力是按 (ability, user type, resource marker type) 建立索引的。
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()
    .into();
```

动作 → 能力的映射沿用 Laravel 的方式：

| 动作 | 能力 |
|---|---|
| `index`、`show`     | `view`   |
| `create`、`store`   | `create` |
| `edit`、`update`    | `update` |
| `destroy`           | `delete` |

`PATCH` 共享 `update` 这个动作，所以它和 `PUT` 受到相同的门控。一个被拒绝的能力会在处理程序运行之前就以 `403` 短路；一个未认证的请求会失败关闭。资源标记类型 `R` 只需要 `Default` - 门是按它的*类型*来判别的，就像 Laravel 按模型类来判别一样。关于能力本身的定义，请参见[授权一章](authorization.md)。

## 路由器级别的重定向与视图

`Router` 上有三个语法糖方法，覆盖了不需要处理程序函数的路由声明：

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new()
    // 静态重定向：GET /old-pricing → 302 /pricing
    .redirect("/old-pricing", "/pricing", 302)
    // 301 的兄弟方法
    .permanent_redirect("/legacy", "/new")
    // Inertia 静态页面：GET /about 渲染 About 组件
    .inertia("/about", "About", json!({ "team_size": 4 }))
    .name("about");
```

`Router::inertia` 是 Suprnova 的 `Route::inertia($uri, $component, $props)`。它注册 `GET`；`HEAD` 请求会落到它上面，且其响应体会在服务器边界被剥离，因此无需额外注册。它返回一个 `RouteBuilder`，所以可以像其他路由一样在其后链式调用 `.name(...)` 和 `.middleware(...)`。

props 必须是 JSON 对象，或没有 props 时的 `null`。其他任何内容 - 数组、字符串 - 都是注册错误，而不是悄悄地得到空 props bag。`try_inertia` 是可失败形式。

`Router::view` 是这个方法的旧名称；它返回 `Router` 而非 `RouteBuilder`，因此用它声明的路由无法命名。请优先使用 `inertia`。

### 为什么 Suprnova 有所不同

Laravel 的 `Route::view` 渲染 Blade 模板；Suprnova 渲染 Inertia 组件，因为框架的模板系统是 Inertia，而非 Blade。一个结果是：这里的组件名是运行时字符串，因此它不会得到 `inertia_response!` 宏执行的编译期页面组件检查。当您希望组件名中的拼写错误在构建而非请求时失败，请用 `inertia_response!` 写出处理程序。

对于重定向*响应*（而不是路由声明） - `Redirect::route`、`Redirect::back`、`Redirect::intended`、签名重定向 - 请参见 [URL 生成](urls.md) 和 [响应](responses.md)。

## 签名 URL

HMAC 签名的路由和路由是相邻的概念（您针对一个命名路由铸造一个 URL，然后在入站请求上验证签名）。它们在 [URL 生成](urls.md) 里有完整的介绍；这里是简短版本：

```rust
use suprnova::url;

let reset = url::signed_route("password.reset", &[("user", "42")])?;
// /password/reset/42?signature=...

let expires_at = chrono::Utc::now().timestamp() + 3600;
let verify = url::temporary_signed_route("verify.email", &[("user", "42")], expires_at)?;
// /verify/email/42?expires=1748803600&signature=...
```

在处理程序内部用 `url::has_valid_signature(&request)`（布尔值）或 `url::signature_verdict(&request)`（`Valid`/`Expired`/`Invalid` 三路结果，让您可以渲染一个“请求一个新链接”的页面，而不是一个泛泛的 403）来验证。

## 可失败的注册

路由注册在启动时只运行一次，所以重复或格式错误的路由会被当作程序员错误处理：普通的辅助函数（`Router::get`、`post`、`put`、`delete`、`ws`、`RouteBuilder::name`，以及 `GroupBuilder` → `Router` 的 `From` 转换）会 **panic**，以便在启动时就明确地失败。对于在源码里声明的路由，这是正确的默认行为。

当模式或名称来自一个可能失败的来源 - 动态配置、插件系统、一个故意注册冲突路由的测试 - 请改用 `try_*` 系列。它们返回 `Result<_, FrameworkError>`（指明出问题的方法、路径或冲突的名称），而不是 panic：

| Panic 版本 | 可失败的对应版本 | 返回值 |
|---|---|---|
| `Router::get` / `post` / `put` / `patch` / `delete` / `head` / `options` | `try_get` / `try_post` / `try_put` / `try_patch` / `try_delete` / `try_head` / `try_options` | `Result<RouteBuilder, FrameworkError>` |
| `Router::ws`（以及每一个 `ws_*` 变体） | `try_ws`（以及每一个 `try_ws_*`） | `Result<Router, FrameworkError>` |
| `RouteBuilder::name` | `try_name` | `Result<Router, FrameworkError>` |
| `GroupBuilder` → `Router`（通过 `.into()`） | `GroupBuilder::try_finalize` | `Result<Router, FrameworkError>` |
| `ResourceRoutes::register` | `try_register` | `Result<Router, FrameworkError>` |

```rust
use suprnova::{FrameworkError, Router};

// `path` 来自动态配置；一个格式错误或重复的模式
// 是可恢复的，而不是启动时的 panic。
fn register_dynamic(router: Router, path: &str) -> Result<Router, FrameworkError> {
    Ok(router.try_get(path, health)?.into())
}
```

一个重复的组路由也可以用同样的方式恢复 - 因为 `From` 不能是可失败的，`.into()` 的可失败对应版本是固有方法 `try_finalize`：

```rust
let router: Router = Router::new()
    .group("/api", |r| r.get("/users", list).post("/users", create))
    .try_finalize()?;
```

panic 版本的辅助函数会作为符合人体工程学的脱围机制保留下来；`try_*` 系列纯粹是增量添加的。

## 为什么 Suprnova 有所不同

**双路径参数语法。** Laravel 使用 `{param}`；Express 使用 `:param`。Suprnova 两者都接受，并会在路径到达 `matchit` 之前把 `:param` 规范化为 `{param}`。两种风格都能和其他一切组合 - 分组、模型绑定、签名 URL。这么做的原因不是优柔寡断，而是我们无法预测您带来的是哪种背景，而路由语法是一个高频的摩擦点，不值得让人重新学习。

**两个平级的 API：宏与构建器。** Laravel 只提供一种 DSL（`Route::get(...)`）。Suprnova 同时提供声明式的 `routes! { ... }` 宏，以及可链式调用的 `Router::new().get(...).name(...)` 构建器。它们产生完全相同的注册结果。宏更适合用来写顶层路由表；当您在动态组合路由器时（插件、生成的路由、测试），构建器读起来更顺。选哪个取决于调用点 - 没有一个标准答案，因为两种形态都是一等公民。

**启动时 panic，而不是静默遮蔽。** 重复的路由名称或模式冲突会在启动时 panic。Laravel 那种以数组为键的注册表会静默地让后注册的那个获胜，当您的路由文件是唯一的注册者时这没问题，但一旦插件或生成的路由加入进来就不安全了。当您确实想要可失败性时，`try_*` 系列就是脱围机制。

## 下一步

- [控制器](controllers.md) - `#[handler]`、表单请求、返回 JSON/Inertia
- [中间件](middleware.md) - `Middleware` trait、执行顺序、编写您自己的中间件
- [URL 生成](urls.md) - 命名路由 URL、签名 URL、重定向、`RouteUrlError`
- [授权](authorization.md) - 针对绑定模型的门与策略
- [WebSocket](websockets.md) - `ws!`、`WebSocketHandler` trait、逐路由配置
