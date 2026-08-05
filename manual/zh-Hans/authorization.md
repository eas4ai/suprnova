# 授权

认证回答的是 _“您是谁？”_；授权回答的是 _“您是否被允许做这件事？”_ Suprnova 提供了一个 Laravel 形状的 `Gate` 门面，外加一个用于面向资源接线的 `#[policy]` 宏，每一种检查都有同步和异步两种变体，这样同一个表面，无论您的策略体需要一次数据库访问，还是只是一次结构体字段比较，都能用。

## 快速上手

```rust
use suprnova::{Authorizable, Gate};

#[derive(Debug)]
struct User { id: i64, is_admin: bool }
#[derive(Debug)]
struct Post { id: i64, author_id: i64, is_public: bool }

// 让用户可以选择接入 `user.can(action, &resource)` 这种人体工程学写法。
impl Authorizable for User {}

// 接入一个能力：
Gate::define::<User, Post>("update", |user, post| {
    user.is_admin || post.author_id == user.id
});

let alice = User { id: 1, is_admin: false };
let own_post = Post { id: 10, author_id: 1, is_public: false };
let foreign_post = Post { id: 11, author_id: 99, is_public: false };

assert!(alice.can("update", &own_post));
assert!(alice.cannot("update", &foreign_post));

// 直接从一个处理程序里返回 403：
alice.authorize("update", &foreign_post)?;
```

## `Gate` 表面

### 定义能力

```rust
// 同步闭包 - 直接被调用，没有装箱的 future。
Gate::define::<User, Post>("view", |user, post| post.is_public || user.id == post.author_id);

// 异步闭包 - 这个 future 必须是被拥有的（不能有跨越闭包返回的借用）。
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_is_admin = user.is_admin;
    let post_id = post.id;
    async move {
        // ……数据库查询、RPC 调用，等等。
        user_is_admin || check_publish_permission(post_id).await
    }
});
```

内部是类型擦除的；这个注册表按 `(action, TypeId<U>, TypeId<R>)` 建立索引。一个 `User` 动作门和一个同名的 `Comment` 动作门是彼此独立存在的 - `Gate::has::<User, Post>("publish")` 和 `Gate::has::<User, Comment>("publish")` 分别给出各自的答案。

### 检查能力

| 方法 | 返回值 | 用途 |
|---|---|---|
| `Gate::allows(action, &user, &resource)` | `bool` | 快速分支 |
| `Gate::denies(action, &user, &resource)` | `bool` | 取反 |
| `Gate::authorize(action, &user, &resource)` | `Result<(), FrameworkError>` | 对一个不带信息的拒绝返回 403；一个更丰富的拒绝会携带自己的状态码/消息（参见[更丰富的判定结果](#更丰富的判定结果-response-inspect-raw)） - 用 `?` 让处理程序短路 |
| `Gate::inspect(action, &user, &resource)` | `Response` | 完整的判定结果：`allowed` + `message` + `code` + HTTP `status` |
| `Gate::raw(action, &user, &resource)` | `Option<Response>` | 和 `inspect` 类似，但 `None` 表示没有定义规则（区别于一次明确的拒绝） |
| `Gate::any(&[...], &user, &resource)` | `bool` | 只要有一个允许就为 true |
| `Gate::none(&[...], &user, &resource)` | `bool` | 没有一个允许时为 true |
| `Gate::check(&[...], &user, &resource)` | `bool` | 全部允许时才为 true |

每个方法都有一个 `_async` 对应版本，对同步注册和异步注册的门都能用，所以处理程序不需要知道背后是哪种闭包在支撑这个动作。

### 内省

```rust
// 这个能力有没有被定义？
Gate::has::<User, Post>("publish");  // bool

// 有哪些能力存在？（按动作名排序并去重）
let all: Vec<String> = Gate::abilities();
```

`abilities()` 会跨资源类型去重：为 `User`-对-`Post` 和 `User`-对-`Comment` 都注册 `"view"`，只会产出一个 `"view"` 条目。这对管理端的选择器和 Inertia 的共享数据很有用。

### 门缺失时的语义

对一个从未被注册过的动作调用 `allows` / `denies` / `authorize`，**默认会拒绝**。对一个异步注册的门调用同步 API 时也是一样（同步路径没法 `.await` - 默认拒绝会通过 `tracing::warn!` 把这个问题暴露在日志里，而不是悄悄放过去）。异步注册的门，从 `_async` 路径调用会得到正确的响应。

## 用 `#[policy]` 实现的策略

当一个资源类型有好几个能力时，把它们归拢进一个策略结构体，然后让 `#[policy]` 把每个方法都注册成一个门：

```rust
use suprnova::policy;
use suprnova::authorization::Response;

struct User { id: i64, is_admin: bool }
struct Post { id: i64, author_id: i64, is_public: bool }
struct PostPolicy;

#[policy(User, Post)]
impl PostPolicy {
    // 一个 `-> bool` 方法就是一个朴素的允许/拒绝门。
    fn view_any(_user: &User, _post: &Post) -> bool {
        true // 任何人都可以列出文章
    }
    fn view(user: &User, post: &Post) -> bool {
        post.is_public || post.author_id == user.id || user.is_admin
    }

    // 一个 `-> Response` 方法在拒绝时可以携带一条消息 + HTTP 状态码。
    fn update(user: &User, post: &Post) -> Response {
        if post.author_id == user.id || user.is_admin {
            Response::allow()
        } else {
            Response::deny_with("You may only edit your own posts.")
        }
    }
    fn delete(user: &User, post: &Post) -> Response {
        if user.is_admin {
            Response::allow()
        } else {
            Response::deny_as_not_found() // 对非管理员隐藏这篇文章
        }
    }
}
```

每个方法都会变成一次 `inventory::submit!`。`Server::serve` 会在启动时通过 `init_policies()` 清空这个 inventory，所以第一个请求到达的时候，每一个动作都已经注册好了（这一步在启动流程里落在哪个位置，参见[应用启动](bootstrap.md)）。`init_policies()` 位于 `suprnova::authorization::init_policies`，并且是幂等的 - 在那些练习策略注册、但又不想拉起一个完整服务器的测试里，可以手动调用它。

策略方法是无状态的关联函数，接受 `(user, resource)` - 和 Laravel 的 `update(User $user, Post $post)` 是同一种形态，只是 Laravel 里 `$this` 是那个无状态的策略对象。每个方法都接受这两个参数，为的是让门的签名保持统一；`view_any` / `create` 只是忽略掉资源参数（`_post`）。您没写的方法不会被注册，一个未注册的动作默认会拒绝。

### 方法名 → 动作的映射

方法名会被直接用作这个动作的动词部分，资源名转换成 kebab-case 形式后缀在后面：

| 方法 | 动作 |
|---|---|
| `Post` 上的 `view` | `"view-post"` |
| `Post` 上的 `view_any` | `"view_any-post"` |
| `UserProfile` 上的 `force_delete` | `"force_delete-user-profile"` |

这和 Laravel 的 camelCase 动作名（`viewAny`、`forceDelete`）不一样，是为了让 Rust 这一侧的表面保持符合语言习惯 - 每一个动作字符串，都镜照着您在编辑器里会自动补全出来的那个方法标识符。

### 返回类型：`bool` 还是 `Response`

一个策略方法的返回类型决定了它要如何注册 - 以及一次拒绝能携带什么：

| 返回类型 | 通过什么注册 | 拒绝会以什么形式出现 |
|---|---|---|
| `bool` | `Gate::define` | 朴素的 `403`（`This action is unauthorized.`） |
| `Response` | `Gate::define_with` | `Response` 携带的那条消息、code 和 HTTP 状态码 |

对于简单的是/否，返回 `bool`。当一次拒绝应当携带一个理由，或者一个非 403 的状态码时，返回一个 `Response`（从 `suprnova::authorization::Response` 导入） - 用 `Response::deny_with("…")` 携带一条消息，或者用 `Response::deny_as_not_found()` 以 `404` 回应并隐藏这个资源的存在。两者都会被编译成同一个类型擦除后的门（一个 `bool` 会被包进一个朴素的允许/拒绝里）。任何其他返回类型 - 或者没写返回类型 - 都是一个编译错误。

## `Authorizable` trait

给 `Gate` 调用用的、开箱即用的用户侧语法糖：

```rust
use suprnova::Authorizable;

impl Authorizable for User {}

// 同步语法糖
if alice.can("update", &post)    { /* ... */ }
if alice.cannot("delete", &post) { /* ... */ }
alice.authorize("update", &post)?;  // 拒绝时返回 403

// 异步语法糖
if alice.can_async("publish", &post).await    { /* ... */ }
alice.authorize_async("publish", &post).await?;
```

每个方法都有一个默认实现体，代理给对应的 `Gate` 方法，所以 `impl Authorizable for User {}`（空实现体）就够了。这是可选接入，而不是一个万能实现：不是每一个能传给 `Gate::allows` 的类型，都适合作为 `.can` 的主体 - 最常见的情况下，它就是您应用里的 `User`。

## 组合模式

### 给路由分组做门控

```rust
use suprnova::{group, get, Auth, AuthMiddleware, FrameworkError, Request, Response};

// 中间件检查认证用户；处理程序对这个动作做授权。
group!("/posts")
    .middleware(AuthMiddleware::new())
    .routes([
        get!("/{id}/edit", edit_form),
    ]);

async fn edit_form(req: Request) -> Response {
    let user: User = Auth::user_as::<User>()
        .await?
        .ok_or(FrameworkError::Unauthorized)?;
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let post = Post::find(id).await?
        .ok_or_else(|| FrameworkError::not_found("Post"))?;
    user.authorize("update", &post)?;
    // ……渲染编辑表单
}
```

### 多动作检查

一个“列出这个用户能对这个资源做的所有事情”的页面：

```rust
let actions = ["view", "update", "delete", "restore", "force_delete"];
let mut allowed = Vec::new();
for action in &actions {
    if user.can(action, &post) {
        allowed.push(*action);
    }
}
// 或者让它短路：
let can_do_anything = Gate::any(&actions, &user, &post);
let is_locked_out   = Gate::none(&actions, &user, &post);
```

### 多门授权

```rust
// 只有当用户能对这个资源做*所有*这些动作时才允许。
Gate::authorize_async("publish", &user, &post).await?;
if Gate::check_async(&["update", "view"], &user, &post).await {
    // 组合检查。
}
```

### 给资源路由做门控

当存在一个 `Router::resource` 表面时，`authorize_resource::<U, R>()` 会把常规的能力检查一次性接到全部七个路由上，这样您就不用依赖每一个控制器方法都记得去做授权：

```rust
Gate::define::<User, Post>("view",   |u, _p| u.is_member);
Gate::define::<User, Post>("create", |u, _p| u.is_author);
Gate::define::<User, Post>("update", |u, _p| u.is_author);
Gate::define::<User, Post>("delete", |u, _p| u.is_admin);

let router: Router = Router::new()
    .resource("posts", PostsCtl)
    .authorize_resource::<User, Post>()   // index/show→view，store→create，……
    .into();
```

一个被拒绝的能力会在处理程序运行之前就返回 `403`；一个未认证的请求会失败关闭。完整的动作 → 能力对照表在[路由一章](routing.md)里。

## 异步语义

`Gate::define_async` 的闭包必须返回一个**被拥有的** future - 这个类型擦除后的注册表不能允许 `&user` 或 `&resource` 引用活得比闭包的返回还久。在返回之前，把您在 `async move {}` 块里需要的字段先拷贝或克隆出来：

```rust
Gate::define_async::<User, Post, _, _>("publish", |user, post| {
    let user_id = user.id;        // 拷贝原生类型
    let post_id = post.id;
    let admin   = user.is_admin;
    async move {
        // 这里没有 `user` / `post` 引用 - 只有被捕获的拷贝。
        admin || check_can_publish(user_id, post_id).await
    }
});
```

同步门可以从异步路径透明地工作（`Gate::allows_async` 会不带 `.await` 地分派它们），所以一个代码库今天可以先注册同步门，之后再把单个能力迁移到异步，而不需要改动调用点。

## 锁污染应对

`Gate` 注册表内部用的是一个 `RwLock`。如果这把锁曾经被污染过（一个线程在持有写守卫期间发生了 panic），这个注册表会**安全拒绝** - 此后每一次 `authorize` 调用都会返回 `Unauthorized`，而不会 panic。注册调用会记一条 `tracing::error!` 然后继续。这和框架更广泛的策略是一致的：一把被污染的锁永远不会中止进程。

## 更丰富的判定结果：`Response`、`inspect`、`raw`

一个朴素的 `bool` 门只能回答允许/拒绝。如果一次拒绝需要携带一条*消息*、一个机器可读的*code*，或者一个非 403 的 HTTP *状态码*，就用 `define_with`（或者 `define_async_with`）注册这个门，并返回一个 `Response`：

```rust
use suprnova::authorization::Response;  // 在 crate 根部重导出为 `GateResponse`

Gate::define_with::<User, Post>("update", |user, post| {
    if post.author_id == user.id {
        Response::allow()
    } else {
        Response::deny_with("You do not own this post.")
    }
});

// 隐藏一个资源的存在，而不是承认它存在：
Gate::define_with::<User, Secret>("view", |user, secret| {
    if user.can_see(secret) {
        Response::allow()
    } else {
        Response::deny_as_not_found()  // 是 404，不是 403
    }
});
```

用 `Gate::inspect`（同步）/ `Gate::inspect_async` 来查看完整的判定结果：

```rust
let decision = Gate::inspect("update", &user, &post);
decision.allowed();   // bool
decision.message();   // Option<&str> - Some("You do not own this post.")
decision.status();    // Option<u16> - 这里是 None；deny_as_not_found 之后是 Some(404)
```

`Response` 的构造函数镜照 Laravel：`allow()`、`deny()`、`deny_with(msg)`、`deny_with_status(status, msg)`、`deny_as_not_found()`，再加上 `with_message` / `with_code` / `with_status` / `as_not_found` 这几个构建器方法。

### 一次拒绝如何变成一个错误

`Gate::authorize` 会通过 `Response::authorize()` 把这个判定结果收拢起来：

| 判定结果 | `authorize` 的结果 |
|---|---|
| 允许 | `Ok(())` |
| 朴素的 `deny()`（没有 message/code/status） | `FrameworkError::Unauthorized`（403，`"This action is unauthorized."`） |
| 更丰富的拒绝（设置了 message 和/或 status） | `FrameworkError::Domain { message, status_code }` |

所以 `deny_as_not_found()` 会以 404 的形式出现，`deny_with_status(422, "…")` 是 422，而 `deny_with("…")` 是携带您那条消息的 403。这个 `code` 在被检视的 `Response` 上是可读的，但**不会**随着 `authorize` 一起传递过去 - `FrameworkError` 没有 code 字段；如果您需要它，请从 `inspect()` 里读。

### `raw`：“拒绝”对比“未定义”

`Gate::raw`（以及 `raw_async`）返回 `Option<Response>`：`None` 意味着*没有规则被应用* - 没有 `before` 钩子触发，没有门被注册，也没有 `after` 钩子填入结果 - 这和一次明确的 `Some(deny)` 是不同的。`inspect` 会把这个 `None` 规范化成默认拒绝；`raw` 则为诊断保留它原来的样子（“这个动作到底有没有被治理？”）。

## `before` / `after` 钩子

`Gate::before` 注册一个在任何门*之前*运行的检查；第一个返回 `Some(decision)` 的钩子会让其余一切都短路。典型的用法是一个全局的覆盖：

```rust
// 管理员可以做任何事。
Gate::before::<User>(|user, _action| user.is_admin.then_some(true));
```

`Gate::after` 在门*之后*运行。遵循 Laravel `??=` 那种语义，一个 after 钩子只能**填入**一个尚未决定的结果（没有门匹配上，也没有 before 钩子触发过） - 它永远不能覆盖一个已经产出的允许/拒绝。每一个 after 钩子都总是会运行，所以它同时也是审计日志的接缝：

```rust
Gate::after::<User>(|user, action, decided| {
    audit_log(user.id, action, decided);   // 观察每一次评估
    None                                    // 只记录；不改变结果
});
```

钩子是按**用户类型** `U` 建立索引的，不是按资源 - 一个钩子会为每一个 `(action, U, R)` 触发。请把资源相关的逻辑放进门里。钩子是同步的判定函数，但也适用于异步的评估路径；对于异步的授权逻辑，请用 `define_async` / `define_async_with`。

### 为什么 Suprnova 有所不同

Laravel 的 `Gate::forUser($user)->allows(...)` 会重新绑定这个门*隐式*的当前用户解析器，让下一次检查按那个用户来求值。Suprnova 的门在每一次调用上都**显式地**接受用户，所以“换一个用户来检查”，就只是 `Gate::allows(action, &other_user, &resource)`。这里没有一个隐式的解析器需要重新绑定 - 这个显式的 API 严格来说更通用，这让 `forUser` 变得多余，而不是缺失。

同样的道理也适用于 Laravel 按类名做的策略自动发现。Suprnova 在注册时就把策略方法绑定到了那个类型擦除后的 `(action, U, R)` 键上，所以一个 `Post` 策略和一个 `Comment` 策略即便有相同的方法名，也会注册成两个不同的门，不需要任何命名约定或者发现扫描。

## 下一步

- [认证](authentication.md) - 用户侧的另一半：认证守卫、`Auth::user()`、`Auth::user_as::<T>()`
- [应用启动](bootstrap.md) - `init_policies()` 在启动流程里的哪个位置运行，以及如何注册 before/after 钩子
- [中间件](middleware.md) - 把 `AuthMiddleware` 和路由级别的授权搭配起来
- [错误模型](error-model.md) - 一次门的拒绝是如何收拢成一个 403、一个 404，或者一个自定义状态码的 `FrameworkError::Domain` 的
- [事件](events.md) - 通过 `Gate::after` 监听策略结果，用于审计日志
