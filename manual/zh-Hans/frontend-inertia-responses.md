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

对于完全没有逻辑的页面 - about、terms、privacy - 完全跳过处理程序，直接声明路由：

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new().inertia("/about", "About", json!({ "team_size": 4 }));
```

参见[路由](routing.md#router-level-redirects-and-views)。其中的组件是运行时字符串，因此不会获得此宏的编译期存在性检查 - 这是不写处理程序的取舍。

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

大多数应用会在启动时通过 [`Inertia::install`](#bootstrap-inertiainstall) 注册一份配置，然后就再也不碰这个参数了 - 那份被安装的配置本来就是每一个响应的起点。只有当您想为某一个页面覆盖这份已安装的配置时，才在这里传一份进来。

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
| `.always_with(k, ‖)` | 异步解析器，忽略部分重新加载过滤 | `Inertia::always(fn () => …)` |
| `.lazy(k, ‖)` | 只有在这个 prop 会被发送时，解析器才运行 | `fn () => …` 闭包 |
| `.optional(k, ‖)` | 首次访问时绝不发送；必须被显式请求 | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | 首次访问时跳过；随后的 XHR 会触发解析 | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | 在部分重新加载时与已有的客户端状态合并 | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | 客户端会在多次导航之间缓存 | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.scroll_with_wrapped` / `.paginate`（经由 `Inertia::paginate`） | 无限滚动分页 | `Inertia::scroll(…)` |
| `.flash(k, v)` | 放在 `page.flash` 下（而不是 `props` 下）的一次性值 | `session()->flash(…)` |
| `.title(…)` | HTML 外壳的默认 `<title>` | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | 逐响应的历史加密 | `Inertia::encryptHistory(…)` |
| `.clear_history()` | 强制在**这个**页面上轮换历史记录密钥 | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | 在 Inertia 访问之后保留 `#fragment` | `Inertia::preserveFragment()` |

eager 的那些构建器方法都有 `try_*` 对应函数（`try_with`、`try_always`、`try_merge_with`、`try_scroll`、`try_scroll_wrapped`、`try_flash`），当某个值的 `Serialize` 实现可能在运行时失败时，它们会返回 `Result<Self, FrameworkError>` - 那些不可失败的方法会通过[那道 Panic 边界](error-model.md)把 panic 转换成一个 500，所以当您宁愿显式处理这次失败时，就伸手去拿 `try_*`。

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

### 在一个 prop 上组合标志

上方的方法各自设置一个标志。一个 prop 可以携带多个标志，某些组合正是 Inertia 协议期待真实页面的工作方式：追加到客户端已渲染内容的 deferred 列表、客户端跨导航缓存的 merge prop、带自己缓存键的 optional prop。请用 `Prop` 构建 prop，再用 `.prop(key, prop)` 附加：

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::lazy(|| async { json!([{ "id": 1 }]) })
        .defer()
        .merge()
        .match_on("id"),
)
```

该 prop 在首次渲染时跳过，并在 `deferredProps` 下公布。客户端发出后续请求，解析器运行，值带着 `mergeProps` 指令到达，因此会追加到屏幕上已有的列表，而非替换它。

标志分为五组：

| 组 | 方法 | 效果 |
|---|---|---|
| 可见性 | `.always()`、`.optional()`、`.defer()` | 互斥；最后一次调用胜出 |
| Defer 细节 | `.group(name)`、`.rescue()` | 仅在 prop deferred 时读取 |
| Merge | `.merge()`、`.prepend()`、`.deep_merge()`、`.match_on(fields)`、`.merge_with_path(path)` | 客户端如何折叠值以及在何处折叠 |
| 客户端缓存 | `.once()`、`.as_key(key)`、`.until(ms)`、`.fresh()` | 客户端是否跨导航保留值 |
| Scroll | `.scroll(metadata)`、`.scroll_wrap(key)` | 无限滚动 `scrollProps` 条目以及无条件 merge 元数据；仅在 `.scroll` 设置时读取 `.scroll_wrap` |

来源包括 `Prop::eager(value)`、`Prop::lazy(closure)`、为自行构建的解析器使用的 `Prop::from_resolver(resolver)`，以及永远不会进入响应的 `Prop::absent()` - 即未加载关系时 `when_loaded!` 返回的内容。

组合前有两条规则值得了解：

- **可见性是一项设置，不是三面标志。** `.always().optional()` 是 optional prop，`.optional().always()` 是 always prop。两者都不是错误；先前调用会被擦除。
- **元数据遵循部分重新加载列表，而不是值。** 即使在值本身被保留的访问中，只要键通过 `X-Inertia-Partial-Data` 和 `X-Inertia-Partial-Except`，prop 的 `mergeProps`、`onceProps` 和 `scrollProps` 条目就会发出。这使 merge 指令能跨越 deferred prop 的两个请求。结果是：
  - 请求集之外的 `.always().merge()` prop 仍发送其值，却不会发送 merge 指令，因此客户端替换而不是追加。
  - `scrollProps` 除列表外还有一个额外条件：`.scroll().defer()` prop 会在非 partial 访问上公布 merge 指令，但那里不发送 cursor，因为屏幕上还没有可由 cursor 描述的内容。每个匹配的 partial reload 都获取 cursor，无论该请求是否也解析了值。
  - `deferredProps` 是列表永不控制的唯一块。它在任何匹配的 partial reload 上整体丢弃，无论列表怎么说 - Laravel 的 `resolveDeferredProps` 一旦请求是 partial 就返回 `[]`。partial reload 是客户端处理已经持有的公告，因此再次公布本轮遗漏键会让它再回来请求它们。面向**不同**组件的 partial reload 对所有 gate 都是标准访问，包括公告。

`.group(name)` 和 `.rescue()` 会存于任何 prop，但仅当 prop deferred 时读取，所以 `.rescue().defer()` 与 `.defer().rescue()` 含义相同。scroll prop 从客户端 `X-Inertia-Infinite-Scroll-Merge-Intent` 请求头获取其 merge 方向，所以 scroll prop 上的 `.merge()` 和 `.prepend()` 是多余且不会读取的。`.deep_merge()` 是例外：它会将 prop 路由到 `deepMergeProps` 而非 `mergeProps`，和 Laravel 的 `ScrollProp` 一样。

### 合并策略与无限滚动

`.merge`（追加）、`.merge_prepend` 和 `.deep_merge` 覆盖了常见的“加载更多”场景。要做差异合并 - 更新客户端已经持有的那些行，而不是把它们复制一份 - 请伸手去拿 `.merge_with`，并给它一个显式的、带 `match_on` 键的 `MergeStrategy`：

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // the new page slice
        MergeStrategy::Append { match_on: Some(vec!["id".into()]) },
    )
```

`match_on` 点名的是客户端据以去重的字段（会以 `matchPropsOn` 的形式发到页面对象里） - 一个字段或多个字段，与下方 `Prop::match_on` 相同 - 所以一次与当前窗口重叠的重新获取，会就地替换匹配上的行，而不是追加一堆副本。`Prepend` 和 `Deep` 接受同样的 `match_on`。

`MergeStrategy` 是单次调用形式。`Prop::merge()` / `.prepend()` / `.deep_merge()` / `.match_on(field)` 是同样设置的独立标志，适用于 prop 同时还需要可见性或缓存标志的情况 - 参见[在一个 prop 上组合标志](#composing-flags-on-one-prop)。

`.match_on` 一次可接收一个字段或多个字段 - `.match_on(["id", "slug"])` 和 `.match_on("id").match_on("slug")` 会发出相同的 `matchPropsOn`。

要只合并 prop 值的一部分，而非整个值，请用 `.merge_with_path` 点名嵌套字段：

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::eager(json!({ "data": next_page, "meta": meta }))
        .merge()
        .merge_with_path("data")
        .match_on("data.id"),
)
```

`mergeProps` 现在携带 `"posts.data"` 而非 `"posts"`，因此只有 `props.posts.data` 会折入客户端已有内容 - `props.posts.meta` 像任何非 merge prop 一样完全替换。调用会累积，所以有两个可合并字段的 prop 可独立点名它们。点名路径会完全关闭该 prop 的根级合并 - 路径合并 prop 永不同时合并整个值。`match_on` 可与路径组合，需在字段名中包含路径（`"data.id"`，而不是 `"id"`）；框架不会为您推断它。`.deep_merge()` 忽略 `.merge_with_path` - 深度合并已递归进入每个嵌套字段，路径无从缩小。

merge prop 的值也可来自解析器，通过 `.merge_lazy` / `.merge_lazy_with` - `.merge` / `.merge_with` 的解析器同级：

```rust
InertiaResponse::new("Feed/Index").merge_lazy("posts", || async {
    Ok::<_, FrameworkError>(load_next_page().await?)
})
```

解析器仅在 merge prop 实际会发送时运行 - 像其他 resolver-backed prop 一样，会被 partial-reload 过滤和 `.defer()` 跳过。

无限滚动就是同一套机制，外加上分页元数据。`.scroll` / `.scroll_with` - 或者 `.paginate`，它能直接适配一个 `LengthAwarePaginator` 或 `CursorPaginator` - 会在数据旁边发出 `scrollProps`，而客户端的 `<InfiniteScroll>` 组件会驱动下一页/上一页的获取：

```rust
// `posts` is a CursorPaginator from the query builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

scroll prop 总会携带 merge 元数据，而不只在后续获取上：它默认 append，只在客户端 `X-Inertia-Infinite-Scroll-Merge-Intent` 请求头如此声明时切换至 prepend（向下滚动时 `append`，向上滚动时 `prepend`）。`reset` 独立于该请求头 - 只有客户端在 `X-Inertia-Reset` 中点名键时才为 `true`，这也是普通 merge prop 所读取的请求头。新的未过滤访问不会发送两个请求头，因此它得到 `reset: false` 和 append 指令，与 Laravel 一致。

`.merge_with_path` 对 scroll prop 没有效果 - 计算其 merge 指令的 scroll 块读取 `Prop::scroll_wrap` 的单个 wrap 键，而非 `.merge_with_path` 的累积路径列表，因此 `.scroll(metadata).merge_with_path("data")` 会存储一个无人读取的路径。直接通过 `.prop(...)` 或下方 `.scroll_wrapped` 响应快捷方法访问的 `.scroll_wrap`，才是 scroll prop 的嵌套等价物。

scroll prop 也和其他 merge prop 一样遵从 `.match_on(...)` - 请通过 `.prop(...)` 访问它，因为 `.scroll` 和 `.match_on` 都没有组合的响应级快捷方法：

```rust
InertiaResponse::new("Users/Index").prop(
    "users",
    Prop::eager(rows)
        .scroll(ScrollMetadata::new("page").current(1).next(2))
        .match_on("id"),
)
```

match 字段以 prop 实际合并的位置为键：未包装时是裸键（`matchPropsOn: ["users.id"]`），或 `.scroll_wrap(...)` 下的 `key.wrap_key`（对包装在 `"data"` 下的 prop 为 `matchPropsOn: ["posts.data.id"]`） - 因此条目始终与客户端折叠的 merge 路径对齐，而不是悄无声息地永不匹配。

当 prop 值本身是包装结构 - `{ data: [...], meta: {...} }`，这是手工构建 API resource 通常返回的形状 - 合并整个对象会在每次获取时覆盖 `meta`。改用 `.scroll_wrapped` 将 merge 指向数组字段：

```rust
InertiaResponse::new("Feed/Index").scroll_wrapped(
    "posts",
    "data",
    ScrollMetadata::new("page").current(2).next(3),
    serde_json::json!({ "data": rows, "meta": { "total": total } }),
)
```

`mergeProps` 随后命名 `posts.data`，因此客户端将新行折入嵌套数组，而每次将 `meta` 整体替换。`.scroll_with_wrapped` 和 `try_scroll_wrapped` 是基于解析器和可失败的同级项，与 `.scroll_with` / `try_scroll` 对应。

这个 crate `pagination` 模块之外的类型 - 第三方 paginator、手写 cursor - 可通过实现 `ProvidesScrollMetadata` 向 `.scroll` 描述自己，而不是逐字段构造 `ScrollMetadata`：

```rust
use suprnova::{ProvidesScrollMetadata, ScrollMetadata};

impl ProvidesScrollMetadata for MyCursorPage {
    fn page_name(&self) -> String { "cursor".to_string() }
    fn previous_page(&self) -> Option<serde_json::Value> { self.prev.clone().map(Into::into) }
    fn next_page(&self) -> Option<serde_json::Value> { self.next.clone().map(Into::into) }
    fn current_page(&self) -> Option<serde_json::Value> { Some(self.current.clone().into()) }
}

InertiaResponse::new("Feed/Index").scroll("posts", page.scroll_metadata(), page.rows)
```

`LengthAwarePaginator`、`Paginator` 和 `CursorPaginator` 也实现它 - 参见[分页](pagination.md#inertia-integration---infinite-scroll-props)。

### 点记法嵌套

含有 `.` 的键会嵌套进响应，而不是作为文字字符串键发出 - Laravel 由 `Arr::set` 支撑的点记法（`Inertia::share('user.name', …)`、`resolveArrayableProperties`）：

```rust
InertiaResponse::new("Dashboard")
    .with("user.name", "Todd")
    .with("user.locale", "es")
```

发出为：

```json
{ "user": { "name": "Todd", "locale": "es" } }
```

而非两个文字 `"user.name"` / `"user.locale"` 键。共享前缀的两次调用会累积为一个对象；没有点的键不受影响。这适用于每一个附加 prop 的方法 - `.with`、`.always`、`.lazy`、shared-registry 键 - 以及没有其他内容：它绝不递归进入 prop 的**值**，所以 validation `errors` 对象保留其内部携带的任何带点字段名。没有可保留文字点的 escape hatch（`.with("config.json", …)` 仍会嵌套） - 这与 Laravel 一致，`Arr::set` 同样没有 escape 机制。

## 部分重新加载

Inertia 3 客户端可以请求一个页面 props 的子集（或者通过带上一个 Optional 或 Defer 键来请求一个超集）。这个协议用到三个请求头：

| 请求头 | 含义 |
|---|---|
| `X-Inertia-Partial-Component` | 正在被部分重新加载的那个组件 - 必须与响应的组件一致，过滤才会生效。 |
| `X-Inertia-Partial-Data` | 白名单：以逗号分隔、要包含进来的 prop 键。 |
| `X-Inertia-Partial-Except` | 黑名单：以逗号分隔、要排除掉的 prop 键。键冲突时它胜过 `Partial-Data`。 |

过滤只读取一项：prop 的可见性，取决于 `.always()`、`.optional()` 或 `.defer()`。没有这些设置的 prop 使用默认可见性。

- 默认可见性 prop 遵循白名单/黑名单语义。
- `.always()` prop 无论如何都发送。
- `.optional()` 和 `.defer()` prop 永不会在标准访问中发送，只会出现在显式列出该键的匹配部分重新加载中。

merge 和 scroll 标志不会参与：它们决定客户端如何折叠已接收的值，而非是否接收它，因此 `.defer().merge()` prop 的过滤与普通 `.defer()` 完全相同。`.once()` 也不参与，尽管它不只是折叠指令 - 在客户端报告该值已缓存的完整访问上，服务器跳过解析器且不发送值，如下方注释所述。三者改变的是随行的元数据块 - 见[在一个 prop 上组合标志](#composing-flags-on-one-prop)。
处理程序不需要做任何特别的事 - 通过构建器把每一个 prop 注册好，框架在序列化页面对象时就会去查这些请求头。

一个 `once` prop 在客户端的缓存，只在一次**完整的** Inertia 访问上才会被尊重。在一次点名了这个键的部分重新加载上（`router.reload({ only: ['stats'] })`），解析器会运行，值也会被发送 - 客户端之所以来问，恰恰是因为它想要一份新的；在那里去尊重它那份过期缓存的说法，只会让它要的那个键什么都拿不到。

### 嵌套 only/except（点记法）

`X-Inertia-Partial-Data` 和 `X-Inertia-Partial-Except` 条目可点名 prop 值内部路径，而不只点名 prop 自身的键。调用 `router.reload({ only: ['user.name'] })` 的客户端会发送 `X-Inertia-Partial-Data: user.name`，响应将 `user` prop 缩窄为该字段：

```json
{ "props": { "user": { "name": "Ada" } } }
```

`except` 以相同方式修剪而非缩窄 - `router.reload({ except: ['user.email'] })` 会使 `user` 的所有其他字段原样保留。

规则：

- 裸条目（`user`）仍表示整个 prop。若 `only` 同时点名 `user` 和 `user.name`，会发送整个值 - 裸条目胜出。
- 条目也可点名带点 prop 键的**祖先**。由 `.with("auth.user", …)` 或 `App::inertia_share("auth.user", …)` 注册在 `auth.user` 下的 prop，会参与 `only: ['auth']`，并完整发出，因为调用方请求了完整 `auth` 根。裸 `except: ['auth']` 会同样丢弃它。前缀必须在片段边界结束，因此无关的 `authAgent.user` prop 不受两者影响。
- 两个请求头点名同一路径时，`except` 胜出，与顶层相同。
- 无法在值中解析的路径 - 未知字段，或穿过 scalar 或数组而非对象的路径 - 不为该路径贡献内容，且不会丢弃同时请求的兄弟字段。
- `Always` prop 完全忽略 `only`/`except`，包括点记法 - 始终完整发送。
- `Optional` 和 `Defer` prop 仍需要显式请求才会解析。带点条目（`permissions.read`）可算作对顶层键的该请求，解析后的值会像 `Eager` prop 一样缩窄。
- 对当前值不是对象的 prop - 字符串、数字、数组 - 使用带点 `only` 会缩窄为 `{}`，而不是原始值。客户端只在缓存值和传入值**均为**对象时深度合并（`inertia-3.6.1/packages/core/src/response.ts` 的 `nestedTopKeys`）；空对象相对非对象缓存会与填充对象相同地未通过检查，因此空对象会完全替换缓存 scalar，而非合并到其上。请避免向非对象形状 prop 发送带点请求。
- 带点 `except` 不会在客户端删除字段 - 它阻止该字段在此次响应中刷新，客户端 merge 会从已有缓存恢复它。`deepMergeObjects` 会先克隆缓存值，然后仅覆盖服务器实际发送的键；服务器修剪的键从不触碰，因此保留旧值。客户端首次加载该 prop（尚无缓存）时，修剪字段确实不存在，因为没有可回退的缓存 - “从缓存恢复”行为仅适用于客户端已经见过的页面。
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

共享键在点上嵌套，方式与 `.with` 相同 - `"user.name"` / `"user.age"` 下的两个静态共享会在传输层落入同一个 `user` 对象。用 `App::inertia_shared` / `App::flush_inertia_shared` 读取共享值，或清空整个静态注册表 - 对应 Laravel 的 `Inertia::getShared` / `Inertia::flushShared`：

```rust
use suprnova::App;

App::inertia_share("user.name", "Todd");
assert_eq!(App::inertia_shared("user.name"), Some(serde_json::json!("Todd")));

App::flush_inertia_shared();
assert_eq!(App::inertia_shared("user.name"), None);
```

`inertia_shared` 仅读取静态注册表 - 对通过 `inertia_share_lazy` / `inertia_share_once` 注册的键会返回 `None`（没有请求可据以解析它，与 Laravel 的 `getShared` 相同，后者返回原始 closure 而不调用它），对逐请求 trait-provider share 也是如此。`flush_inertia_shared` 同样只清除静态注册表；通过 `register_inertia_shared` 注册的 provider 没有逐请求状态可清除。

对于逐请求的共享数据（已认证用户、请求作用域的标志），实现 [`InertiaSharedData`](#per-request-shared-data) 并注册单例 - 框架会在每一个 Inertia 响应中调用 `share(&req, component)` 并合并结果。`component` 是正在渲染的页面，因此 provider 可以按页面改变输出 - 如下所示。

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
        component: &str,
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
        // 按页面变化：只有管理仪表板需要导航计数。
        if component == "Admin/Dashboard" {
            out.insert("pendingReviews".into(), Prop::eager(serde_json::json!(12)));
        }
        Ok(out)
    }
}

// 在 bootstrap 里：
App::register_inertia_shared(Arc::new(AuthShare));
```

如果 provider 无需按页面变化，请忽略 `component`（`_component`）。

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

### 验证失败

处理程序在 Inertia 访问中验证失败时，框架会带着 flash 的错误，以 `303 See Other` 回到表单页，而不是返回 REST 客户端获得的 `422` JSON。这并非表面差异：Inertia 客户端会将任何没有 `X-Inertia` 响应头的响应视为非 Inertia，并在全屏错误模态框中渲染它，因此 `422` 永远到不了 `form.errors`。处理程序不需要改变 - 此桥接是 `Inertia::install` 注册的中间件之一。

所有四个分支共享**同一个**动作：删除原有的 `errors` flash，写入新错误，然后把原来的响应替换成一个 `303` 重定向。不会保留原有响应体、它的响应头或其中任何 `Set-Cookie` - 若一个自定义中间件在生成 `422` 后排队 cookie，它必须在验证桥接前运行，或者在 `303` 之后自己重新排队。框架不自动移动这些 cookie，正如 Laravel 的 `HandleInertiaRequests` 不会移走 controller 的 `422` 头。

标准错误对象仍显示为 `page.props.errors`：框架在下一次 Inertia 渲染时从 session flash 水合它。将验证器指向的每个 named bag（`validator.error_bag = Some("createUser")`）也会发为 `page.props.errors.createUser`，与 Laravel 的 `X-Inertia-Error-Bag` 行为对齐。没有 bag 的错误保留在顶层。一个消费 session flash 的非 Inertia 请求仍会消费同一份数据；不要假设它只会由 Inertia 使用。

目标依次是同源请求 `Referer`、会话记录的 previous URL，最后是失败请求自身的 URL。跨源 `Referer` 会被忽略；仅看似同源的也会被忽略：前导 `//` 或 `/\`（浏览器会在把反斜杠折叠为斜杠后将两者解析为 protocol-relative）以及值中任意位置的 ASCII 控制字节（URL 解析器会在比较源前从整个字符串剥离 tab 和换行，因此控制字节可将看似安全的路径在浏览器导航时变成另一源）均以相同方式回退。相同检查也用于最终 URL 回退，因此异常请求路径同样不能变成异源重定向。

字段值是其**第一条**消息，即普通字符串 - Inertia 自己的 `ErrorValue` 类型所描述的形状，也是 `$page.props.errors.email` 绑定的内容。设置 `InertiaConfig::with_all_errors(true)` 可改为以数组取得所有消息；客户端类型随后需要相应扩展：

```ts
// global.d.ts
import '@inertiajs/core'

declare module '@inertiajs/core' {
  export interface InertiaConfig {
    errorValueType: string[]
  }
}
```

一页上的多个表单保持隔离：在访问时发送 `X-Inertia-Error-Bag: <name>`，错误会在该 bag 下 flash 并从其下读回，以 `errors.<name>.<field>` 到达。

默认情况下 `errors` prop 始终可见，因此部分重新加载从不将其过滤或缩窄。`only: ['users']` 仍发送 bag，`except: ['errors']` 也一样；`only: ['errors.email']` 发送整个 bag 而非该字段。这与 Laravel 的形状相同 - 它的中间件将 bag 作为 `Inertia::always(...)` 共享，`resolveAlways` 在 `only`/`except` 重建后重新注入原始值。这很重要，因为客户端以 `{...current.props, ...response.props}` 折叠部分响应：空的 `errors` 对象会清除屏幕上已有消息，而未过滤的对象会正确保留它们。此规则覆盖两个来源 - 会话 flash bag 和处理程序自己的 `.with("errors", …)`。显式可见性标志仍优先，因此 `.prop("errors", Prop::eager(…).optional())` 表现为 optional。

它不做两件事。它不会重新 flash old input - 桥接运行时请求体已被消耗，Inertia `useForm` 会在失败提交中保留自己的状态，因此没有内容需重新填充。它也永不触及 Precognition 响应：dry-run `422` 正是客户端所要求的。

要把访客送**出** Inertia 应用 - 一个支付提供商、一个 OAuth 授权端点、一个托管的账单门户 - 请使用 `location_for`：

```rust
use suprnova::{InertiaResponse, Request, Response};

pub async fn checkout(req: Request) -> Response {
    Ok(InertiaResponse::location_for(&req, "https://billing.example/checkout"))
}
```

一次 Inertia XHR 会拿到 `409` + `X-Inertia-Location`（客户端会执行 `window.location = url`）；一次硬性导航则拿到一个普通的 `302` + `Location`。裸的 `InertiaResponse::location(url)` 总是返回 409 那种形式 - 只在已经确定这个请求就是一次 Inertia 访问的地方用它，因为一个浏览器追随一个没有 `Location` 请求头的 `409` 时，根本无处可去。

## 版本检测

Inertia 会给资产清单加上版本，这样一个长期存活的客户端就不会拿昨天那份 bundle 里的页面，去挂载到今天的服务器上。当客户端的 `X-Inertia-Version` 请求头与服务器已配置的版本对不上时，[`InertiaVersionMiddleware`](#bootstrap-inertiainstall) 会回答一个 `409 Conflict`，外加一个点名新 URL 的 `X-Inertia-Location` 请求头 - Inertia 客户端会接住它，做一次整页重新加载，从而拿到新的 bundle。

这次弹回会先重新 flash 会话。客户端会用一次整页 GET 来回应 409，而那次 GET 是一个全新的请求 - 没有这次重新 flash，上一个请求 flash 进去的验证错误或成功消息，就会在目的地页面读到它之前被老化掉，用户会仅仅因为一次部署正好落在提交中途，就丢掉自己的错误消息。这需要 `SessionMiddleware` 注册在版本中间件之前。

默认情况下无需设置任何内容：`InertiaConfig` 会对 Vite 构建清单（`manifest_path`，默认 `public/assets/.vite/manifest.json`）做 hash，并使用 SHA-256 的前 16 个字节经十六进制编码后的值。清单是每次构建都会变化、其他时候不会变化的唯一文件，因此版本会自行递增。没有清单可读时 - Vite 从内存提供内容的本地开发 - 它会回退到静态字符串 `"1.0"` 并以 `debug` 级别记录日志。

当您想使用其他来源时再覆盖：

```rust
use suprnova::{InertiaConfig, VersionResolver};

// 默认 - hash 构建清单。无需写任何内容。
let cfg = InertiaConfig::new();

// 不同的清单位置；版本随之变化。
let cfg = InertiaConfig::new().manifest_path("dist/.vite/manifest.json");

// 静态 - 烘焙构建时标识符。在后续 `.manifest_path(...)` 调用后依然存在：
// 显式版本是有意为之。
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// 动态 - 容器部署 ID 或任何其他值。闭包会在每次版本检查时运行；
// 若它成本不低，请在内部缓存。
let cfg = InertiaConfig::new().version_with(|| deployment_id());
```

清单会在每次版本检查时读取，这也是 Laravel `hash_file` 的做法 - 来自页面缓存的数 KB 内容，且构建会立刻被拾取。若已测量到此开销且希望消除它，请在启动时解析一次：

```rust
use suprnova::{InertiaConfig, VersionResolver};

let version = VersionResolver::from_manifest("public/assets/.vite/manifest.json").resolve();
let cfg = InertiaConfig::new().version(version);
```

对于异步或可失败的版本解析（如从 S3 读取清单 hash），请在启动时读取一次，并将缓存的 `String` 传给 `.version(...)`。

## 启动：`Inertia::install`

大多数应用会从仅 HTTP 的启动 hook `register_http_stack` 以一次调用安装四个协议中间件；服务器路径运行它，而队列、调度、工作流和 console 二进制跳过它（见[启动](bootstrap.md)）：

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    Inertia::install(&InertiaConfig::new())
        .expect("Inertia install failed");
    // add global middleware in the order you want it to wrap requests
}
```

```rust
// cmd/main.rs
Application::new()
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
```

不要将它放入 `bootstrap::register`。当构建前端清单缺失时，`Inertia::install` 会在生产环境失败即关闭；而 worker 或 console 镜像通常不含 `public/assets`，因此从进程范围 hook 安装会将这些二进制也一同终止。

`Inertia::install` 返回 `Result`，并按顺序：

1. 若 `cfg` 解析为生产模式（`development == false` - `APP_ENV=production` 时的默认值）却不能从 `cfg.manifest_path` 加载 Vite 清单，则失败即关闭。
2. 注册 `InertiaHeadersMiddleware` - 在每个响应设置 `Vary: X-Inertia`，并将 Inertia 访问的空 `200` 转为回跳 `303`。
3. 注册 `InertiaVersionMiddleware` - 客户端和服务器资产版本不一致时，发送 `409` + `X-Inertia-Location`。
4. 注册 `Inertia303Middleware` - 在非 GET Inertia 重定向上将 `302` 升级为 `303`。
5. 注册 `InertiaValidationRedirectMiddleware` - 将 Inertia 访问的 `422` 变为回到表单页、并 flash 错误的 `303`。参见[验证失败](#validation-failures)。

顺序很重要：headers middleware 最先注册，所以最外层且能看到每一个响应，包括版本 middleware 在处理程序尚未运行前返回的 `409`。验证重定向 middleware 最后注册，因此最内层、最接近处理程序，在另外三个 middleware 有机会触及它之前先看到 `422`。

`install` 还会**保留配置**。之后构建的每个 `InertiaResponse` 均以它为起点，因此 `.frontend(...)`、`.version(...)`、`.default_title(...)`、`.ssr(...)` 和 `.encrypt_history(...)` 到达每一个页面，无需处理程序传递它。使用 `.with_config(...)` 仍可为单个页面覆盖；从不调用 `Inertia::install` 的应用获得 `InertiaConfig::default()`；再次调用 `install` 会替换保留配置。

`Inertia::install` 只能调用一次；第二次调用失败，不会替换保留配置或叠加中间件。使用同一 `InertiaConfig` 的单一安装调用来设置配置。单页的 `.with_config(...)` 覆盖仍有效，但不能更改已安装版本中间件所使用的版本。

若使用 flash 数据，请在 `Inertia::install` **之前**注册 `SessionMiddleware`。版本 middleware 会在客户端弹回前重新 flash 会话，使 flash 错误经受后续完整页面 GET；它只能在会话作用域内完成此事。

仅当确实不想要某个 middleware 时才跳过此调用（很少；四者分别阻止 URL 两种表示之间的缓存投毒、静默的陈旧 bundle、重定向上的表单重放，以及验证 `422` 在客户端错误模态框中结束而无法到达 `form.errors`）。

## 服务器驱动的 `<head>` 元素

Inertia 3.5 添加了一个客户端选项，让服务器决定 `<head>` 里放什么 - 当 meta 标签依赖于您刚加载出来的那条记录，而您又不想让标题和 OG 标签活在两个地方时，这就很有用。

这不需要任何框架支持。客户端会从一个**普通的 prop** 里读取这些元素，所以任何处理程序都可以提供它们：

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>, req: Request) -> Response {
    inertia_response!(&req, "Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    })
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

在它进行任何分发前，网关可以检查已构建 SSR bundle 是否存在于磁盘 - 通过指向约定路径 `frontend/bootstrap/ssr/ssr.js` 的 `.ssr_bundle_path(...)` 选择加入（检查本身默认开启，即 `.ssr_ensure_bundle_exists(true)`，但设置路径前没有效果 - 这不会被自动检测，因此针对 test double 启用 SSR 无需同时在磁盘 stub bundle）。缺少 bundle 会立即回退到 CSR，不会为注定失败的连接付出 `ssr_timeout`。这映照 Laravel 的 `ensure_bundle_exists` 配置。

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")
        .ssr_bundle_path("frontend/bootstrap/ssr/ssr.js")
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

`suprnova new` 为每个 starter 脚手架化 `frontend/src/ssr.{ts,tsx}` 和 `build:ssr` npm script。构建它，然后启动工作进程：

```bash
cd frontend && npm run build:ssr
suprnova ssr:start
```

`suprnova ssr:check` 会验证工作进程确实在响应 - 它请求工作进程自己的 `GET /health` route，每个 `createServer()` bundle 都无需额外代码即可暴露该 route。

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
    .with_all_errors(false)                   // 每字段一条消息，或全部
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

另外还有九个 Rust 形态的选择值得点出来：

- **lazy prop 的解析器是并发运行的**，上限由 `max_concurrent_resolvers` 控制（默认 16）。一个有十二个 lazy prop 的页面，会在一个 Tokio 任务内部发出十二个并行查询 - 我们把框架建在 Tokio 之上，就是为了这个。如果一个页面有很多 lazy prop、每一个都要打到某个外部服务，请调一下这个上限。
- **编译期的组件检查**根本不是一个 Laravel 特性，因为 PHP 在编译期看不到您的前端文件。Suprnova 看得到，所以 `inertia_response!("Dashbaord", …)` 里的一个拼写错误，会让构建失败并给出一条“您是不是想写 Dashboard？”的建议，而不是稍后以一次运行时的“找不到组件”浮出水面。
- **一次 Inertia 访问上的空 `200` 会变成 `303`，而不是 `302`。** Laravel 的 `onEmptyResponse` 返回 `redirect()->back()`（一个 302），并且只对 PUT/PATCH/DELETE 依靠它后面那次 `302 → 303` 的转换。一个被替换掉的重定向，永远不是原方法的延续 - 客户端必须发出一次 GET - 所以 Suprnova 直接说 `303`，而不是把 GET 访问留在一个客户端会带着原动词去追随的 302 上。
- **`Inertia::location($url)` 在这里是两个方法，不是一个。** `location(url)` 保持 Laravel 那份始终为 `409` 的契约 - 它早于那个能感知请求的形式，而那些钉住标签的消费方依赖这个形态不变。`location_for(&req, url)` 是更新的、能感知请求的形式：对一次 Inertia XHR 是 `409`，对一次硬性导航是普通的 `302`。新代码请伸手去拿 `location_for`。
- **`Inertia::clearHistory()` 在这里同样是两个方法，不是一个。** 构建器上的 `.clear_history()` 标记的是单个响应；`App::clear_history()` 会把这个标志 flash 进会话，好让它挺过一次重定向。Laravel 之所以能用一个方法搞定，是因为它本来就有会话支撑 - Suprnova 把响应局部的那种形式保留为默认（不依赖会话），而把跨重定向的场景做成一次显式的选择启用。
- **`.lazy()` 不是 Laravel 的 `Inertia::lazy()`。** Laravel 的方法已弃用，且行为像 `optional()` - `LazyProp` 是 `OptionalProp` 的直接别名，首次访问时完全跳过（`ResponseFactory.php:174-181`）。Suprnova 的 `.lazy()` 是 Laravel 自身对不带 wrapper 的 callable prop 使用的普通 closure 约定 - 只要 partial-reload 过滤允许该键通过，它就会包含在内，标准访问也不例外。如果您来自 Laravel，想要名称“lazy”暗示的首次访问跳过行为，请使用 `.optional()`。
- **嵌套 `only`/`except` 在解析后而非解析前缩窄。** Laravel 的 `Response::resolvePartialProperties` 穿过原始、尚未解析的 prop 数组中的点路径，因此进入 `LazyProp` 或 `DeferProp` 的路径会退化为 `null` - 遍历碰到未解析 closure 后停止（`inertia-laravel-2.0.25/src/Response.php:273-297`）。Suprnova 先解析每个 prop 的值 - resolver 是 async，因此没有 Laravel 偶尔具备的全部 plain array 的同步时刻 - 再缩窄结果 JSON 值。未知或类型不匹配的嵌套路径会被丢弃而非回传 `null`，这符合客户端自己的 reconciliation 预期：它会将缩窄对象深度合并进已有内容（`inertia-3.6.1/packages/core/src/response.ts:414-425`），而异常 `null` 会覆盖客户端已有字段，而不是保留它。
- **`.scroll_wrapped` 是选择加入，而非自动。** Laravel 的 `Inertia::scroll($value, $wrapper = 'data', …)` 默认将每个 scroll prop 的 merge instruction 嵌在 `"data"` 下，因为 Laravel paginator resource 通常返回 `{ data: [...], links: {...}, meta: {...} }`，且只有数组应合并。Suprnova 内置 paginator 返回裸行数组（`Vec<T>`，无 envelope），所以 `.scroll` / `.paginate` 在 prop 根合并；需要嵌套路径的情形可用 `.scroll_wrapped`。
- **包装的 scroll prop 会自动为其 `match_on` 字段加前缀。** 在 `.scroll_wrapped("posts", "data")` prop 上，`match_on("id")` 会发出 `"posts.data.id"`。Laravel 发出未加前缀的 `"posts.id"`，其客户端无法将其与 merge target 对齐，故 match 悄然永不触发。这里嵌套点无歧义 - 一个 scroll prop 至多一个 wrapper - 因此 Suprnova 推导前缀，而非要求您键入它。请写裸字段名，不要写路径。

## 下一步

- [页面组件](frontend-pages.md) - 前端是如何把一个组件名解析到一个 Svelte / React / Vue 模块的
- [TypeScript 类型](frontend-typescript-types.md) - `suprnova generate-types` 从您的 `#[derive(InertiaProps)]` 结构体发出 TS 定义
- [数据对象](data.md) - 用于 DTO 的 `#[derive(Data)]`，带着逐字段的 include/允许列表把关，能和部分重新加载组合起来
- [错误模型](error-model.md) - `Response`、Panic 边界，以及 `FrameworkError` 是如何穿过 Inertia 响应的
- [服务容器](container.md) - `App::inertia_share*` 和 `InertiaSharedData` 背后的那个查找模型
