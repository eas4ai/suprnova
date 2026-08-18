# Inertia 响应

Inertia 响应就是 Suprnova 处理程序把状态发布给一个 Svelte / React / Vue 页面组件的方式。每一个渲染 Inertia 页面的处理程序，都会返回一个这样的响应，要么通过 [`inertia_response!`](#inertia-response-宏) 这个宏构建（用于类型化、编译期校验的 eager props），要么通过 [`InertiaResponse`](#inertiaresponse-构建器) 这个构建器构建（用于其它一切 - lazy props、deferred props、merge、once、scroll、flash）。本章端到端地覆盖这整个响应接口：这个宏、这个构建器、v3 协议的那些特性（部分重新加载、历史加密、版本检测）、通过 `App::inertia_share*` 实现的共享数据，以及跨重定向携带的那个 flash bag。

如果您还没有选定一个前端，请先看[前端概览](frontend.md)和[页面组件](frontend-pages.md)；本章假定 SPA 桥梁已经接好，只专注于您的处理程序应该返回什么。

## `inertia_response!` 宏

这个宏是从一个处理程序到一个类型化 eager 页面的最短路径。它接受当前请求、一个组件名，以及一个 props 表达式：

```rust
use suprnova::{Request, Response, inertia_response, InertiaProps};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
    pub message: String,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        message: "Hello from Suprnova!".into(),
    })
}
```

有三件事需要知道：

- **开头那个 `&req` 是必需的。** 这个宏会从请求上读取 `X-Inertia` 请求头、URL，以及部分重新加载的过滤请求头，所以它需要这个请求值（或者一个引用）。没有它，部分重新加载就会悄无声息地坏掉。
- **组件是否存在会在编译期检查。** 这个宏会去找 `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`；如果没有文件匹配，构建就会失败，并给出一条“您是不是想写……？”的建议，建议内容取自磁盘上真实的文件名。嵌套路径的工作方式相同 - `inertia_response!(&req, "Admin/Dashboard", …)` 会解析到 `frontend/src/pages/Admin/Dashboard.svelte`（或者您那个前端对应的扩展名）。
- **这个宏展开成一个被 `await` 的 `Result`。** 您的处理程序必须返回 [`Response`](error-model.md)（也就是 `Result<HttpResponse, HttpResponse>`），或者另一种能通过 `?` / `From` 吸收 `FrameworkError` 的类型。prop 序列化或响应构建期间的失败会作为 `Err` 返回，而不是 panic。

### JSON 风格的 props

做原型和写小页面时，您可以跳过类型化的结构体：

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

这个宏仍然会校验组件文件。代价是您失去了类型化 props 那条链 - 没有 `#[derive(InertiaProps)]`，没有自动的 TypeScript 生成，也没有编译期检查来确认前端期待的形状是否吻合。

### 可选的配置覆盖

这个宏接受一个可选的、放在末尾的 `InertiaConfig`，用于逐响应的覆盖（不同的 SSR 设置，或者某一页专用的自定义默认标题）：

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

大多数应用会在启动时通过 [`Inertia::install`](#启动-inertia-install) 注册一份配置，然后就再也不碰这个参数了 - 那份被安装的配置本来就是每一个响应的起点。只有当您想为某一个页面覆盖这份已安装的配置时，才在这里传一份进来。

## `#[derive(InertiaProps)]`

`InertiaProps` 会发出一个 `Serialize` 实现，它的键名和您的字段名相匹配。它存在的意义，是让这条类型化 props 的路径保持简洁，也让这个 TypeScript 生成器（`suprnova generate-types`）有一个可以寻找的标记：

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
pub struct UserProps {
    pub name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
}
```

嵌套的类型会正常组合起来 - 字段可以是 `Vec<T>`、`Option<T>`、嵌套的结构体，任何能 `Serialize` 的东西。这些嵌套的类型本身不需要派生 `InertiaProps`；它们只需要 `Serialize`。在*顶层*的 props 结构体上用 `#[derive(InertiaProps)]`，您就会为整棵树拿到自动的 TypeScript 接口（参见[TypeScript 类型](frontend-typescript-types.md)）。

## `InertiaResponse` 构建器

这个宏覆盖的是类型化的 eager props。其他一切 - lazy、optional、deferred、可合并的、缓存在客户端的、flash、历史加密的覆盖 - 都直接使用这个构建器：

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy：只有在这个 prop 确实会被发送时，闭包才运行
        //（首次访问，或者一次请求了这个键的部分重新加载）。
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional：首次访问时绝不发送；客户端必须通过
        // X-Inertia-Partial-Data 显式索要这个键。
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer：首次渲染时跳过；客户端会发出一次后续的
        // XHR，闭包那时才运行。
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge：在部分重新加载时追加到已有内容里（“加载更多”）。
        .merge("rows", next_page().await?)
        // Once：在多次导航之间缓存在客户端；除非服务器强制刷新，
        // 否则后续访问会跳过解析器。
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash：一次性的 toast；出现在 `page.flash` 下，而不是 `props` 下。
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| 方法 | 用途 | 对应的 Laravel |
|---|---|---|
| `.with(k, v)` | eager prop，遵从部分重新加载的过滤 | 类型化 prop |
| `.always(k, v)` | eager prop，忽略部分重新加载的过滤 | `Inertia::always(…)` |
| `.lazy(k, ‖)` | 只有在这个 prop 会被发送时，解析器才运行 | `fn () => …` 闭包 |
| `.optional(k, ‖)` | 首次访问时绝不发送；必须被显式请求 | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | 首次访问时跳过；随后的 XHR 会触发解析 | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | 在部分重新加载时与已有的客户端状态合并 | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | 客户端会在多次导航之间缓存 | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate`（经由 `Inertia::paginate`） | 无限滚动分页 | `Inertia::scroll(…)` |
| `.flash(k, v)` | 放在 `page.flash` 下（而不是 `props` 下）的一次性值 | `session()->flash(…)` |
| `.title(…)` | HTML 外壳的默认 `<title>` | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | 逐响应的历史加密 | `Inertia::encryptHistory(…)` |
| `.clear_history()` | 强制在**这个**页面上轮换历史记录密钥 | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | 在 Inertia 访问之后保留 `#fragment` | `Inertia::preserveFragment()` |

eager 的那些构建器方法都有 `try_*` 对应函数（`try_with`、`try_always`、`try_merge_with`、`try_scroll`、`try_flash`），当某个值的 `Serialize` 实现可能在运行时失败时，它们会返回 `Result<Self, FrameworkError>` - 那些不可失败的方法会通过[那道 Panic 边界](error-model.md)把 panic 转换成一个 500，所以当您宁愿显式处理这次失败时，就伸手去拿 `try_*`。

`.clear_history()` 标记的是您正在构建的那个响应。一个登出处理程序会做重定向，而浏览器会丢掉这次重定向的响应 - 所以必须携带这个标志的是登录页面，而不是登出响应。`App::clear_history()` 正是为这种情况准备的修复 - 它是一个自由函数，不是构建器方法，所以不在上面那张表里。它会把一个一次性的会话标志 flash 进去，下一个 Inertia 页面对象会把它变成 `clearHistory: true`。它需要一个会话作用域，并且恰好只能挺过一跳。

请在 `Auth::logout()` / `Auth::logout_and_invalidate()` **之后**调用它，而不是之前 - 失效操作会清空整个会话，而这个标志就住在那个会话里，所以先 flash 只会被这次清空抹掉：

```rust
use suprnova::{App, Auth, Redirect, Response};

pub async fn logout() -> Response {
    Auth::logout_and_invalidate().await?;
    App::clear_history();
    Redirect::to("/login").into()
}
```

### 合并策略与无限滚动

`.merge`（追加）、`.merge_prepend` 和 `.deep_merge` 覆盖了常见的“加载更多”场景。要做差异合并 - 更新客户端已经持有的那些行，而不是把它们复制一份 - 请伸手去拿 `.merge_with`，并给它一个显式的、带 `match_on` 键的 `MergeStrategy`：

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // 新的这一页切片
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on` 点名的是客户端据以去重的那个字段（会以 `matchPropsOn` 的形式发到页面对象里），所以一次与当前窗口重叠的重新获取，会就地替换匹配上的行，而不是追加一堆副本。`Prepend` 和 `Deep` 接受同样的 `match_on`。

无限滚动就是同一套机制，外加上分页元数据。`.scroll` / `.scroll_with` - 或者 `.paginate`，它能直接适配一个 `LengthAwarePaginator` 或 `CursorPaginator` - 会在数据旁边发出 `scrollProps`，而客户端的 `<InfiniteScroll>` 组件会驱动下一页/上一页的获取：

```rust
// `posts` 是一个来自查询构造器的 CursorPaginator。
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

框架会从客户端发来的 `X-Inertia-Infinite-Scroll-Merge-Intent` 请求头里读取合并方向（向下滚动时是 `append`，向上滚动时是 `prepend`）。在一次全新的访问上 - 没有这个意图请求头 - `scrollProps["posts"].reset` 是 `true`，所以客户端会在渲染第一个窗口之前清空它的累加器。

## 部分重新加载

Inertia 3 客户端可以请求一个页面 props 的子集（或者通过带上一个 Optional 或 Defer 键来请求一个超集）。这个协议用到三个请求头：

| 请求头 | 含义 |
|---|---|
| `X-Inertia-Partial-Component` | 正在被部分重新加载的那个组件 - 必须与响应的组件一致，过滤才会生效。 |
| `X-Inertia-Partial-Data` | 白名单：以逗号分隔、要包含进来的 prop 键。 |
| `X-Inertia-Partial-Except` | 黑名单：以逗号分隔、要排除掉的 prop 键。键冲突时它胜过 `Partial-Data`。 |

过滤规则：

- `Eager`、`Lazy`、`Merge`、`Once`、`Scroll` 这几类 prop 遵循白名单 / 黑名单语义。
- `Always` prop 无论如何都会被发送。
- `Optional` 和 `Defer` prop 在一次标准访问上绝不出现，只会出现在一次显式列出了这个键、并且组件匹配的部分重新加载里。

处理程序不需要做任何特别的事 - 通过构建器把每一个 prop 注册好，框架在序列化页面对象时就会去查这些请求头。

一个 `once` prop 在客户端的缓存，只在一次**完整的** Inertia 访问上才会被尊重。在一次点名了这个键的部分重新加载上（`router.reload({ only: ['stats'] })`），解析器会运行，值也会被发送 - 客户端之所以来问，恰恰是因为它想要一份新的；在那里去尊重它那份过期缓存的说法，只会让它要的那个键什么都拿不到。

## 通过 `App::inertia_share*` 共享数据

有些 props 在每一个 Inertia 页面上都是一样的 - 认证状态、CSRF 令牌、当前的语言区域、应用全局的标志。在启动时把它们注册一次，它们就会合并进每一个响应里：

```rust
use suprnova::App;
use std::sync::Arc;

pub fn register() {
    // 同步的，在启动时计算一次并固定下来。
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // 异步的，逐响应解析（会被排除了这个键的
    // 部分重新加载跳过）。
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // 在多次导航之间缓存在客户端 - `share_once` 会在第一个
    // 需要它的页面上运行，此后客户端会通过
    // `X-Inertia-Except-Once-Props` 跳过重新解析，直到这个缓存键变化。
    App::inertia_share_once("plans", || async {
        Ok::<_, suprnova::FrameworkError>(load_plan_catalog().await?)
    });
}
```

对于逐请求的共享数据（已认证的用户、请求作用域的标志），实现 [`InertiaSharedData`](#逐请求的共享数据)，并注册这个单例 - 框架会在每一个 Inertia 响应上调用 `share(&req)`，并合并结果。

### 键冲突时的优先级

当同一个键出现在多个层里时，后写入的那个会赢：

1. 静态注册表（`App::inertia_share` / `App::inertia_share_lazy`）
2. 逐请求的 trait 提供者（`InertiaSharedData::share`）
3. 逐响应的构建器方法（`.with`、`.lazy`，等等）

这让一个处理程序可以为某一页覆盖一个全局共享的默认值，而不需要去注销任何东西。

### 逐请求的共享数据

这个 trait 会在每个 Inertia 响应上运行一次，能访问到这个请求。实现需要 `async_trait`（重导出为 `suprnova::__async_trait`）和 `IndexMap`（重导出为 `suprnova::indexmap`）：

```rust
use suprnova::{
    App, Auth, FrameworkError, InertiaRequestExt, InertiaSharedData, Prop,
    indexmap::IndexMap,
};
use std::sync::Arc;

pub struct AuthShare;

#[suprnova::__async_trait]
impl InertiaSharedData for AuthShare {
    async fn share(
        &self,
        _req: &dyn InertiaRequestExt,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::Eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        Ok(out)
    }
}

// 在 bootstrap 里：
App::register_inertia_shared(Arc::new(AuthShare));
```

## flash 与重定向

flash 数据是一种一次性的状态，它应该出现在下一次渲染里，之后就消失 - toast 消息、“刚刚创建”的 ID、验证结果摘要。Suprnova 会在每一个 Inertia 响应上，把它呈现在 `page.flash` 下面。有三种写入方式：

```rust
// 1. 压进当前请求的 flash bag 里。
App::flash("toast", "Saved");

// 2. 附加到某个特定的响应上（效果一样，但只作用于这个响应）。
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. 通过 Redirect 门面跨重定向携带。
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

`Redirect::with(key, value)` 这种写法是跨处理程序的路径：这个值会落进会话的 `_flash.new.*` 下面，下一个请求的 [`SessionMiddleware`](csrf.md) 会把它老化成 `_flash.old.*`，目的地的 `InertiaResponse` 再把它呈现在 `page.flash` 下面。

键冲突时，同一请求内的 flash（那个任务本地的包）会胜过继承下来的会话 flash，所以一个目的地处理程序只需要重新 flash 一次这个键，就能覆盖一个进来的值。

内部会话键（任何以 `_` 打头的）会被从 `page.flash` 里过滤掉 - 用于表单回填的 `_old_input` 和 `_inertia.*` 协议标志都不会泄漏给客户端。

### Redirect 辅助函数

`Redirect` 提供了完整的 Laravel 表面：

```rust
Redirect::to("/dashboard")                       // 302 到一个路径
Redirect::route("posts.show").with("id", "42")   // 具名路由，路由参数
Redirect::back("/")                              // 会话记录下来的上一个 URL
Redirect::refresh()                              // 同一个 URL，全新的 GET
Redirect::guest(&req, "/login")                  // 暂存目标 URL
Redirect::intended("/dashboard")                 // 弹出暂存的那个 URL
Redirect::signed_route("downloads.show", &[("id","42")])?  // 签名 URL
Redirect::to("/posts/42").preserve_fragment()    // 跨访问保留 #frag
```

所有 `Redirect` 变体都接受 `.with(k, v)`、`.with_input(map)`、`.with_errors(map)`、`.with_errors_bag(name, map)`、`.cookie(c)`、`.header(k, v)`、`.permanent()`、`.status(303)` 等等。整条链与 Laravel 的 `RedirectResponse` 一一对应。

对于非 GET 的 Inertia 访问，当 [`Inertia303Middleware`](#启动-inertia-install) 已安装时，框架会自动把响应转换成 `303 See Other`，这样浏览器就会发出一次干净的后续 GET，而不是把原来的 PUT/PATCH/DELETE 重新提交到重定向目标。

要把访客送**出** Inertia 应用 - 一个支付提供商、一个 OAuth 授权端点、一个托管的账单门户 - 请使用 `location_for`：

```rust
use suprnova::{InertiaResponse, Request, Response};

pub async fn checkout(req: Request) -> Response {
    Ok(InertiaResponse::location_for(&req, "https://billing.example/checkout"))
}
```

一次 Inertia XHR 会拿到 `409` + `X-Inertia-Location`（客户端会执行 `window.location = url`）；一次硬性导航则拿到一个普通的 `302` + `Location`。裸的 `InertiaResponse::location(url)` 总是返回 409 那种形式 - 只在已经确定这个请求就是一次 Inertia 访问的地方用它，因为一个浏览器追随一个没有 `Location` 请求头的 `409` 时，根本无处可去。

## 版本检测

Inertia 会给资产清单加上版本，这样一个长期存活的客户端就不会拿昨天那份 bundle 里的页面，去挂载到今天的服务器上。当客户端的 `X-Inertia-Version` 请求头与服务器已配置的版本对不上时，[`InertiaVersionMiddleware`](#启动-inertia-install) 会回答一个 `409 Conflict`，外加一个点名新 URL 的 `X-Inertia-Location` 请求头 - Inertia 客户端会接住它，做一次整页重新加载，从而拿到新的 bundle。

这次弹回会先重新 flash 会话。客户端会用一次整页 GET 来回应 409，而那次 GET 是一个全新的请求 - 没有这次重新 flash，上一个请求 flash 进去的验证错误或成功消息，就会在目的地页面读到它之前被老化掉，用户会仅仅因为一次部署正好落在提交中途，就丢掉自己的错误消息。这需要 `SessionMiddleware` 注册在版本中间件之前。

您通过 `InertiaConfig` 来设置这个版本：

```rust
use suprnova::InertiaConfig;

// 静态的 - 大多数应用都这样。烘焙进一个构建期的标识符。
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// 动态的 - 读一个清单哈希、一个容器部署 ID，什么都行。
// 这个闭包会在每一次版本检查时运行；如果它不便宜，就在内部做缓存。
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

如果版本解析是异步的或者可能失败（比如从 S3 读取一个清单哈希），请在启动时读一次，然后把缓存下来的 `String` 传给 `.version(...)`。

## 启动：`Inertia::install`

大多数应用会用一次调用装好这三个协议中间件：

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register() -> Result<(), suprnova::FrameworkError> {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)?;
    // ……其他共享数据、路由等等。
    Ok(())
}
```

`Inertia::install` 返回 `Result`，并按顺序做这些事：

1. 如果 `cfg` 解析成生产模式（`development == false` - 只要 `APP_ENV=production` 就是默认值），却没法从 `cfg.manifest_path` 加载出任何 Vite 清单，就失败即关闭。这就是 CFG-01 那道防护：一次前端尚未构建的生产启动，会醒目地报错，而不是悄悄回退到一条遗留的硬编码资产路径。
2. 注册 `InertiaHeadersMiddleware` - 在每一个响应上设置 `Vary: X-Inertia`，并把一次 Inertia 访问上的空 `200` 变成一个 `303` 回跳。
3. 注册 `InertiaVersionMiddleware` - 当客户端和服务器对资产版本意见不一致时，发出 `409` + `X-Inertia-Location`。
4. 注册 `Inertia303Middleware` - 在非 GET 的 Inertia 重定向上，把 `302` 升级成 `303`。

顺序很要紧：请求头中间件最先注册，所以它是最外层的，能看到每一个响应 - 包括版本中间件在处理程序还没运行之前就返回的那个 `409`。

`install` 还会**保留这份配置**。此后构建的每一个 `InertiaResponse` 都以它为起点，所以在这里设置的 `.frontend(...)`、`.version(...)`、`.default_title(...)`、`.ssr(...)` 和 `.encrypt_history(...)` 会到达每一个页面，而不需要哪个处理程序传任何东西。想为某一个页面用不同设置的处理程序，仍然可以用 `.with_config(...)` 覆盖；一个从不调用 `Inertia::install` 的应用拿到的是 `InertiaConfig::default()`；再调用一次 `install` 会替换掉被保留的那份配置。

`.with_config(...)` 会整份替换这份配置，`version` 也包括在内。`InertiaVersionMiddleware` 解析的仍然是当初交给 `Inertia::install` 的那个版本，所以这里一份没有带上同样 `.version(...)` 的配置，会让页面对象声明一个中间件将要弹回的版本 - 客户端在访问那个页面之后，会多经历一次整页加载。请在覆盖用的配置上把 `.version(...)` 设成一致。

如果您用到了 flash 数据，请把 `SessionMiddleware` 注册在 `Inertia::install` **之前**。版本中间件会在把客户端弹回之前重新 flash 会话，这样一条 flash 进去的错误消息才能挺过随后那次整页 GET；而它只有在一个会话作用域内部才做得到这件事。

只有当您确实不想要其中某一个中间件时，才跳过这次调用（这种情况很少；这三个都堵住了真实的失败模式 - 同一个 URL 的两种表示之间的缓存投毒、静默的过期 bundle，以及重定向上的表单重放）。

## 服务器驱动的 `<head>` 元素

Inertia 3.5 添加了一个客户端选项，让服务器决定 `<head>` 里放什么 - 当 meta 标签依赖于您刚加载出来的那条记录，而您又不想让标题和 OG 标签活在两个地方时，这就很有用。

这不需要任何框架支持。客户端会从一个**普通的 prop** 里读取这些元素，所以任何处理程序都可以提供它们：

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>) -> Response {
    Ok(inertia_response!("Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    }))
}
```

在客户端选择启用：

```js
createInertiaApp({
  serverHead: true,        // 读取 `head` 这个 prop
  // serverHead: 'meta',   // 或者读取一个名字不同的 prop
  // serverHead: (page) => [...],  // 或者从整个页面计算出来
})
```

每个字符串都是一个 HTML 元素。客户端会给任何缺少 `data-inertia` 属性的元素打上一个，这样它就能在多次导航之间对 head 元素做 diff；当您想要稳定的身份而不是按位置匹配时，就自己提供一个 `data-inertia="og-title"`。

对任何从用户数据插值进来的东西做转义 - 这些字符串是以 HTML 的形式注入的，所以通常的规则依然适用。

## SSR

Suprnova 通过 HTTP 环回，与一个进程外的 SSR 工作进程对话 - 通常就是跑在 Node / Bun / Deno 之下的 `@inertiajs/{svelte,react,vue}/server` `createServer()` bundle。请在您交给 [`Inertia::install`](#启动-inertia-install) 的那份配置上启用它 - 那份配置就是每一个响应的起点，所以不需要在您的处理程序里到处传递任何东西：

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // 工作进程的 URL
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

SSR 默认是关闭的，而且它是这份配置的一个属性：对每一个由已安装配置构建出来的响应它是开的，对任何用一份没有设置它的 `.with_config(...)` 覆盖过的响应它就是关的。启用之后，框架会把页面对象 POST 到 `<url>/render`，并把 `{ head, body }` 内联进 HTML 外壳。当工作进程出错或超时时，响应会回退到 CSR（一个空的 `<div id="app">`，由客户端去水合），同时 `on_ssr_error(...)` 钩子会触发；在 CI 里把 `ssr_throw_on_error(true)` 打开，就能让这些失败变成硬性的 500。

请单独启动这个工作进程 - 一旦您的项目提供了一个 SSR 入口，`suprnova ssr:start` 就是标准的运行器。

## 配置

Inertia 的行为是通过 `InertiaConfig` 以编程方式配置的，而您交给 [`Inertia::install`](#启动-inertia-install) 的那份配置，就是每一个响应的起点。框架直接读取的唯一一个环境变量是 `SUPRNOVA_FRONTEND`（`svelte` / `react` / `vue`），而且只有在配置没有明说时，它才提供默认的入口点文件名和页面组件扩展名 - 已安装配置上一个显式的 `.frontend(Frontend::React)` 会胜出，而这也正是 `suprnova new --frontend react` 脚手架出来的东西。其余一切都是构建器形态的：

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // 覆盖 SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // 给 lazy prop 的扇出设上限
    .url_resolver(|req| req.path_and_query()) // `page.url` 是怎么推导出来的
    .production();                            // false → 从 Vite 开发服务器加载
```

各前端专属的默认值：

| 前端 | 默认入口点 | 页面扩展名 |
|---|---|---|
| Svelte（默认） | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`、`.jsx` |
| Vue | `src/main.ts` | `.vue` |

### `url` 字段

`page.url` 是这个请求的路径**加上**查询字符串（`/users?page=2&sort=name`）。客户端会把它写进 `history.state`，所以前进/后退导航和 `router.reload()` 重放的正是它 - 丢掉查询，每一个分页过或过滤过的页面都会悄无声息地重置到第一页。`InertiaVersionMiddleware` 也是从请求的路径和查询推导出它的 `X-Inertia-Location` 的，所以默认情况下，一次 409 资产版本弹回会把浏览器带到页面对象点名的那个 URL 上，分毫不差。

当客户端应该记录的 URL 与实际到达的那个不同时 - 比如一个 SPA 并不据以路由的语言区域前缀，或者一条被反向代理重写过的路径 - 就用 `url_resolver` 覆盖这个推导过程：

```rust
use suprnova::InertiaConfig;

let cfg = InertiaConfig::new()
    .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
```

这个解析器通过 `InertiaRequestExt` 读取请求，并作用于每一个由您传给 [`Inertia::install`](#启动-inertia-install) 的那份配置构建出来的响应 - 一个应该全应用生效的解析器，通常就放在那里。要为单个响应覆盖它，请用 `InertiaResponse::with_config(cfg)`。一个解析器只会改变 `page.url`。那次 409 弹回仍然点名实际到达的那个 URL - 那才是浏览器必须去获取的 URL - 所以在装了解析器之后，两者是有意不同的。

`manifest_path` 处的 Vite 清单会在第一个请求时惰性加载，并在整个进程生命周期里缓存 - 每一个由已安装配置构建出来的响应共享这同一份缓存，所以这个文件只会被读取和解析一次。当它缺失时，生产环境的资产标签会回退到一条硬编码的遗留路径，并触发一条 `tracing::warn!`，好让这个缺口浮现在日志里。

### 为什么 Suprnova 有所不同

Laravel 的 Inertia 适配器有一个全局的“共享数据”注册表，外加一个逐请求的 `Inertia::share($k, $v)` 调用。PHP 那种“每个请求一个进程”的模型让这样做是安全的：每个请求都是一个全新的进程，意味着并发访客之间不会泄漏。

Rust 的进程模型恰恰相反 - 一个进程跨许多线程服务许多并发请求。所以这个注册表住在[服务容器](container.md)上（任务本地 → 线程本地 → 全局），而不在进程级的全局静态量里。`App::inertia_share*` 写入的是当前活跃容器的 `InertiaRegistry`，这让那些使用 `TestContainer::fake()` 的测试获得干净的隔离，而不必去注销任何东西。表面和 Laravel 一样；底下的机制不同，因为运行时不同。

另外还有五个 Rust 形态的选择值得点出来：

- **lazy prop 的解析器是并发运行的**，上限由 `max_concurrent_resolvers` 控制（默认 16）。一个有十二个 lazy prop 的页面，会在一个 Tokio 任务内部发出十二个并行查询 - 我们把框架建在 Tokio 之上，就是为了这个。如果一个页面有很多 lazy prop、每一个都要打到某个外部服务，请调一下这个上限。
- **编译期的组件检查**根本不是一个 Laravel 特性，因为 PHP 在编译期看不到您的前端文件。Suprnova 看得到，所以 `inertia_response!("Dashbaord", …)` 里的一个拼写错误，会让构建失败并给出一条“您是不是想写 Dashboard？”的建议，而不是稍后以一次运行时的“找不到组件”浮出水面。
- **一次 Inertia 访问上的空 `200` 会变成 `303`，而不是 `302`。** Laravel 的 `onEmptyResponse` 返回 `redirect()->back()`（一个 302），并且只对 PUT/PATCH/DELETE 依靠它后面那次 `302 → 303` 的转换。一个被替换掉的重定向，永远不是原方法的延续 - 客户端必须发出一次 GET - 所以 Suprnova 直接说 `303`，而不是把 GET 访问留在一个客户端会带着原动词去追随的 302 上。
- **`Inertia::location($url)` 在这里是两个方法，不是一个。** `location(url)` 保持 Laravel 那份始终为 `409` 的契约 - 它早于那个能感知请求的形式，而那些钉住标签的消费方依赖这个形态不变。`location_for(&req, url)` 是更新的、能感知请求的形式：对一次 Inertia XHR 是 `409`，对一次硬性导航是普通的 `302`。新代码请伸手去拿 `location_for`。
- **`Inertia::clearHistory()` 在这里同样是两个方法，不是一个。** 构建器上的 `.clear_history()` 标记的是单个响应；`App::clear_history()` 会把这个标志 flash 进会话，好让它挺过一次重定向。Laravel 之所以能用一个方法搞定，是因为它本来就有会话支撑 - Suprnova 把响应局部的那种形式保留为默认（不依赖会话），而把跨重定向的场景做成一次显式的选择启用。

## 下一步

- [页面组件](frontend-pages.md) - 前端是如何把一个组件名解析到一个 Svelte / React / Vue 模块的
- [TypeScript 类型](frontend-typescript-types.md) - `suprnova generate-types` 从您的 `#[derive(InertiaProps)]` 结构体发出 TS 定义
- [数据对象](data.md) - 用于 DTO 的 `#[derive(Data)]`，带着逐字段的 include/允许列表把关，能和部分重新加载组合起来
- [错误模型](error-model.md) - `Response`、Panic 边界，以及 `FrameworkError` 是如何穿过 Inertia 响应的
- [服务容器](container.md) - `App::inertia_share*` 和 `InertiaSharedData` 背后的那个查找模型
