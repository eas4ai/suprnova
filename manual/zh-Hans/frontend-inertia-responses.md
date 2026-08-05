# Inertia 响应

Inertia 响应就是 Suprnova 处理程序把状态发布给一个 Svelte / React / Vue 页面组件的方式。每一个渲染 Inertia 页面的处理程序，都会返回一个这样的响应，要么通过 [`inertia_response!`](#inertia-response-宏) 这个宏构建（用于类型化、编译期校验的 eager props），要么通过 [`InertiaResponse`](#inertiaresponse-构建器) 这个构建器构建（用于其它一切 - lazy props、deferred props、merge、once、scroll、flash）。本章端到端地覆盖这整个响应接口：这个宏、这个构建器、v3 协议的那些特性（部分重新加载、历史加密、版本检测）、通过 `App::inertia_share*` 实现的共享数据，以及跨重定向携带的那个 flash bag。

如果您还没有选定一个前端，请先看[前端概览](frontend.md)和[页面组件](frontend-pages.md)；本章假定 SPA 桥梁已经接好，只专注于您的处理程序应该返回什么。

## `inertia_response!` 宏

这个宏是从一个处理程序到一个类型化 eager 页面之间最短的路径。它接受当前请求、一个组件名，以及一个 props 表达式：

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

有三件事要知道：

- **前面这个 `&req` 是必需的。** 这个宏会从请求上读取 `X-Inertia` 请求头、URL，以及那些部分重新加载过滤请求头，所以它需要这个请求的值（或者一个引用）。没有它，部分重新加载会静默地坏掉。
- **组件是否存在，是在编译期检查的。** 这个宏会去找 `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`；如果没有文件匹配，这次构建就会失败，并带上一条从磁盘上真实的文件名派生出来的 `did you mean…?` 建议。嵌套路径的工作方式是一样的 - `inertia_response!(&req, "Admin/Dashboard", …)` 会解析到 `frontend/src/pages/Admin/Dashboard.svelte`（或者您前端对应的扩展名）。
- **这个宏会展开成一个被 `await` 过的 `Result`。** 您的处理程序必须返回 [`Response`](error-model.md)（也就是 `Result<HttpResponse, HttpResponse>`），或者另一个能通过 `?` / `From` 吸收 `FrameworkError` 的类型。prop 序列化或者响应构建过程中的失败，会以 `Err` 的形式返回，而不是 panic。

### JSON 风格的 props

对于原型开发和小页面，您可以跳过这个类型化的结构体：

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

这个宏依然会校验这个组件文件。代价是您会失去这条类型化 prop 的链条 - 没有 `#[derive(InertiaProps)]`，没有自动的 TypeScript 生成，也没有对前端期望形态是否匹配的编译期检查。

### 可选的配置覆盖

这个宏接受一个可选的、放在末尾的 `InertiaConfig`，用于逐响应的覆盖（不同的 SSR 设置，某一页自定义的默认标题）：

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

大多数应用会在启动时，通过 [`Inertia::install`](#启动-inertia-install) 注册一份单一的配置，永远不会去碰这个参数。

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

这个宏覆盖了 eager 的类型化 props。其它一切 - lazy、optional、deferred、可合并、客户端缓存、flash、历史加密覆盖 - 都直接用这个构建器：

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy：这个闭包只在这个 prop 真的会被发送时才运行
        // （首次访问，或者请求了这个键的部分重新加载）。
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional：初次访问时永远不发送；客户端必须
        // 通过 X-Inertia-Partial-Data 显式请求这个键。
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer：在初次渲染时被跳过；客户端会发出一次
        // 后续的 XHR，这个闭包才会在那时运行。
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge：在部分重新加载（“加载更多”）时追加进已有的数据里。
        .merge("rows", next_page().await?)
        // Once：在多次导航之间被缓存在客户端；除非服务器
        // 强制刷新，否则后续访问会跳过这个解析器。
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash：一次性的 toast；出现在 `page.flash` 下面，不在 `props` 里。
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| 方法 | 用途 | 对应 Laravel 里的什么 |
|---|---|---|
| `.with(k, v)` | Eager prop，遵循部分重新加载过滤 | 类型化 prop |
| `.always(k, v)` | Eager prop，忽略部分重新加载过滤 | `Inertia::always(…)` |
| `.lazy(k, ‖)` | 只在 prop 会被发送时才运行的解析器 | `fn () => …` 闭包 |
| `.optional(k, ‖)` | 初次访问时不发送；必须被显式请求 | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | 初次访问时跳过；后续 XHR 触发解析 | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | 在部分重新加载时和客户端现有状态合并 | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | 客户端在多次导航之间缓存 | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate`（通过 `Inertia::paginate`） | 无限滚动分页 | `Inertia::scroll(…)` |
| `.flash(k, v)` | `page.flash` 下面（不是 `props`）的一次性值 | `session()->flash(…)` |
| `.title(…)` | HTML 壳的默认 `<title>` | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | 逐响应的历史加密 | `Inertia::encryptHistory(…)` |
| `.clear_history()` | 强制历史密钥轮换 | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | 在 Inertia 访问之后保留 `#fragment` | `Inertia::preserveFragment()` |

Eager 构建器方法有一组 `try_*` 的对应版本（`try_with`、`try_always`、`try_merge_with`、`try_scroll`、`try_flash`），当一个值的 `Serialize` 实现在运行时有可能失败时，它们会返回 `Result<Self, FrameworkError>` - 那些不可失败的方法会通过 [Panic 边界](error-model.md) 把这个 panic 转换成一个 500，所以当您更想显式处理这次失败时，就伸手去用 `try_*`。

### 合并策略与无限滚动

`.merge`（追加）、`.merge_prepend` 和 `.deep_merge` 覆盖了常见的“加载更多”场景。要做 diff 式合并 - 更新客户端已经持有的那些行，而不是把它们重复一遍 - 请伸手去用 `.merge_with`，带上一个携带着 `match_on` 键的显式 `MergeStrategy`：

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // 这一页新的切片
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on` 指名了客户端用来去重的那个字段（会以 `matchPropsOn` 的形式发到页面对象里），所以一次和当前窗口有重叠的重新获取，会就地替换掉匹配的行，而不是追加出重复的副本。`Prepend` 和 `Deep` 用的是同一个 `match_on`。

无限滚动用的是同一套机制，只是挂上了分页的元数据。`.scroll` / `.scroll_with` - 或者 `.paginate`，它直接适配一个 `LengthAwarePaginator` 或者 `CursorPaginator` - 会在数据旁边发出 `scrollProps`，而客户端的 `<InfiniteScroll>` 组件会驱动下一页/上一页的获取：

```rust
// `posts` 是来自查询构造器的一个 CursorPaginator。
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

框架会从客户端发送的 `X-Inertia-Infinite-Scroll-Merge-Intent` 这个请求头里，读出合并的方向（向下滚动时是 `append`，向上滚动时是 `prepend`）。在一次全新的访问上 - 没有 intent 请求头 - `scrollProps["posts"].reset` 是 `true`，所以客户端会在渲染第一个窗口之前，清空它的累积器。

## 部分重新加载

Inertia 3 的客户端可以请求一个页面 props 的一个子集（或者通过包含一个 Optional 或 Defer 键来请求一个超集）。这个协议用了三个请求头：

| 请求头 | 含义 |
|---|---|
| `X-Inertia-Partial-Component` | 正在被部分重新加载的组件 - 必须和响应的组件一致，过滤才会生效。 |
| `X-Inertia-Partial-Data` | 白名单：要包含的、逗号分隔的 prop 键。 |
| `X-Inertia-Partial-Except` | 黑名单：要排除的、逗号分隔的 prop 键。键冲突时胜过 `Partial-Data`。 |

过滤规则：

- `Eager`、`Lazy`、`Merge`、`Once`、`Scroll` 这些 props 遵循白名单 / 黑名单语义。
- `Always` 这些 props 无论如何都会被发送。
- `Optional` 和 `Defer` 这些 props 在一次标准访问上永远不会出现，只会出现在一次明确列出了这个键的、匹配的部分重新加载上。

处理程序不需要做任何特殊的事 - 通过这个构建器注册每一个 prop，框架会在序列化这个页面对象时，去查阅这些请求头。

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

## Flash 与重定向

Flash 数据是一种一次性的状态，应该出现在下一次渲染上，之后就消失 - toast 消息、“刚刚创建”的 ID、验证摘要。Suprnova 会在每一个 Inertia 响应上，把它呈现在 `page.flash` 下面。有三种写入方式：

```rust
// 1. 推入当前请求的 flash bag。
App::flash("toast", "Saved");

// 2. 附加到一个特定的响应上（效果只作用于这一个响应）。
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. 通过 Redirect 门面跨一次重定向携带过去。
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

`Redirect::with(key, value)` 这个形态是跨处理程序的路径：这个值会落在会话里的 `_flash.new.*` 下面，下一个请求的 [`SessionMiddleware`](csrf.md) 会把它老化成 `_flash.old.*`，而目的地的那个 `InertiaResponse` 会把它呈现在 `page.flash` 下面。

同请求的 flash（那个任务本地的 bag），在键冲突时会胜过继承来的会话 flash，所以一个目标处理程序，只需要重新 flash 这个键，就能覆盖一个进来的值。

内部的会话键（任何带 `_` 前缀的东西），会被从 `page.flash` 里过滤掉 - 用于表单回填的 `_old_input`，以及 `_inertia.*` 这些协议标志，都不会泄漏给客户端。

### 重定向辅助函数

`Redirect` 是完整的 Laravel 接口：

```rust
Redirect::to("/dashboard")                       // 302 到一个路径
Redirect::route("posts.show").with("id", "42")   // 具名路由，路由参数
Redirect::back("/")                              // 会话记录的上一个 URL
Redirect::refresh()                              // 同一个 URL，全新的 GET
Redirect::guest(&req, "/login")                  // 暂存原定的 URL
Redirect::intended("/dashboard")                 // 取出暂存的 URL
Redirect::signed_route("downloads.show", &[("id","42")])?  // 签名 URL
Redirect::to("/posts/42").preserve_fragment()    // 跨访问保留 #frag
```

所有 `Redirect` 的变体都接受 `.with(k, v)`、`.with_input(map)`、`.with_errors(map)`、`.with_errors_bag(name, map)`、`.cookie(c)`、`.header(k, v)`、`.permanent()`、`.status(303)`，等等。这整条链镜照了 Laravel 的 `RedirectResponse`。

对于非 GET 的 Inertia 访问，当 [`Inertia303Middleware`](#启动-inertia-install) 被安装时，框架会自动把这个响应转换成 `303 See Other`，这样浏览器就会发出一个干净的后续 GET，而不是把原始的 PUT/PATCH/DELETE 重新提交到这个重定向目标上。

## 版本检测

Inertia 会给这个资产 manifest 打版本号，这样一个长期存活的客户端就不会试图把昨天那个 bundle 里的页面，挂载到今天的服务器上。当客户端的 `X-Inertia-Version` 请求头和服务器配置的版本不一致时，[`InertiaVersionMiddleware`](#启动-inertia-install) 会用 `409 Conflict`，外加一个指名新 URL 的 `X-Inertia-Location` 请求头来响应 - Inertia 客户端会捡起这个信息，做一次整页重新加载，把这个新的 bundle 接过来。

您通过 `InertiaConfig` 来设置这个版本：

```rust
use suprnova::InertiaConfig;

// 静态的 - 大多数应用。把一个构建期的标识符固化进去。
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// 动态的 - 读取一个 manifest 哈希、容器部署 ID，任何东西。
// 这个闭包会在每一次版本检查时运行；如果它开销不小，就在内部做缓存。
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

对于异步的或者可能失败的版本解析（比如从 S3 读取一个 manifest 哈希），请在启动时做一次读取，并把缓存下来的这个 `String` 传给 `.version(...)`。

## 启动：`Inertia::install`

大多数应用会用一次调用，安装这两个协议中间件：

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register() -> Result<(), suprnova::FrameworkError> {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)?;
    // ……其它的共享数据、路由，等等。
    Ok(())
}
```

`Inertia::install` 返回 `Result`，并按顺序做这些事：

1. 如果 `cfg` 解析为生产模式（`development == false` - 只要 `APP_ENV=production`，这就是默认值），但从 `cfg.manifest_path` 加载不到 Vite manifest，就会故障关闭。这是 CFG-01 这道防护：一次带着未构建前端的生产环境启动，会明确地报错，而不是悄悄回退到一个遗留的硬编码资产路径。
2. 注册 `InertiaVersionMiddleware` - 当客户端和服务器在资产版本上意见不一致时，发出 `409` + `X-Inertia-Location`。
3. 注册 `Inertia303Middleware` - 在非 GET 的 Inertia 重定向上，把 `302` 升级成 `303`。

只有当您确实不想要这两个中间件之一时（很罕见；两者都堵上了真实的失败模式 - 静默的陈旧 bundle，以及重定向上的表单重放），才跳过这次调用。

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

Suprnova 会通过 HTTP 环回，和一个进程外的 SSR 工作进程通信 - 通常是在 Node / Bun / Deno 下运行的 `@inertiajs/{svelte,react,vue}/server` 这个 `createServer()` bundle。在配置上启用它：

```rust
InertiaConfig::new()
    .ssr("http://127.0.0.1:13714")  // 工作进程的 URL
    .ssr_timeout(std::time::Duration::from_millis(500))
    .ssr_exclude("/admin/**")
    .ssr_max_response_bytes(8 * 1024 * 1024)
```

SSR 默认是关闭的。启用之后，框架会把这个页面对象 POST 到 `<url>/render`，并把 `{ head, body }` 内联进这个 HTML 壳里。当工作进程出错或者超时时，这个响应会回退到 CSR（一个客户端会去水合的空 `<div id="app">`），并且 `on_ssr_error(...)` 这个钩子会被触发；在 CI 里把 `ssr_throw_on_error(true)` 打开，能让这些失败变成实打实的 500，而不是回退。

把这个工作进程单独启动起来 - 一旦您的项目发布了一个 SSR 入口，`suprnova ssr:start` 就是标准的运行器。

## 配置

Inertia 的行为是通过 `InertiaConfig` 以编程方式配置的。框架直接读取的唯一一个环境变量是 `SUPRNOVA_FRONTEND`（`svelte` / `react` / `vue`），它选择默认的入口文件名和页面组件的扩展名。其它一切都是构建器形态的：

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)              // 覆盖 SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)            // 给惰性 prop 的扇出封顶
    .production();                           // false → 从 Vite 开发服务器加载
```

按前端区分的默认值：

| 前端 | 默认入口 | 页面扩展名 |
|---|---|---|
| Svelte（默认） | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`、`.jsx` |
| Vue | `src/main.ts` | `.vue` |

`manifest_path` 处的这个 Vite manifest，会在首次请求时惰性加载，并在整个进程的生命周期内被缓存。当它缺失时，生产环境的资产标签会回退到一个硬编码的遗留路径，同时会触发一条 `tracing::warn!`，让这个缺口在日志里浮现出来。

### 为什么 Suprnova 有所不同

Laravel 的 Inertia 适配器有一个单一的全局“共享数据”注册表，外加一个逐请求的 `Inertia::share($k, $v)` 调用。PHP 每请求一个进程的模型，让这一点是安全的：每个请求一个全新的进程，意味着并发访客之间不会有泄漏。

Rust 的进程模型正好相反 - 一个进程要在许多线程上服务许多并发请求。所以这个注册表活在[服务容器](container.md)上（任务本地 → 线程本地 → 全局），不在进程全局的静态变量里。`App::inertia_share*` 会写入当前活跃容器的那个 `InertiaRegistry`，这让使用 `TestContainer::fake()` 的测试拿到干净的隔离，而不需要去注销任何东西。和 Laravel 一样的接口；底下是不同的机制，因为运行时是不同的。

还有两个值得指出的、Rust 形状的选择：

- **惰性 prop 解析器是并发运行的**，受 `max_concurrent_resolvers`（默认 16）封顶。一个带着十二个惰性 prop 的页面，会在一个 Tokio 任务内部，发出十二次并行查询 - 这正是我们把框架建在 Tokio 之上的意义所在。如果一个页面有很多惰性 prop，每一个都要打到一个外部服务，就去调整这个上限。
- **这个编译期的组件检查**根本不是一个 Laravel 特性，因为 PHP 在编译期看不到您的前端文件。Suprnova 看得到，所以 `inertia_response!("Dashbaord", …)` 里的一次拼写错误，会带着一条 `did you mean Dashboard?` 的建议让构建失败，而不是稍后才在运行时冒出一个“组件未找到”式的错误。

## 下一步

- [页面组件](frontend-pages.md) - 前端是如何把一个组件名解析到一个 Svelte / React / Vue 模块的
- [TypeScript 类型](frontend-typescript-types.md) - `suprnova generate-types` 从您的 `#[derive(InertiaProps)]` 结构体发出 TS 定义
- [数据对象](data.md) - 用于 DTO 的 `#[derive(Data)]`，带着逐字段的 include/允许列表把关，能和部分重新加载组合起来
- [错误模型](error-model.md) - `Response`、Panic 边界，以及 `FrameworkError` 是如何穿过 Inertia 响应的
- [服务容器](container.md) - `App::inertia_share*` 和 `InertiaSharedData` 背后的那个查找模型
