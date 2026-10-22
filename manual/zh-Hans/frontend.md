# 前端概览

Suprnova 通过 [Inertia.js](https://inertiajs.com/) 3.4.0 将 Rust 处理程序桥接到单页面前端。您在 Rust 中编写控制器，在 Svelte、React 或 Vue 中编写页面；框架在它们之间传递类型化的 props，中间不需要单独的 HTTP API。

## 三个一等起步套件

`suprnova new <name>` 脚手架生成一个可工作的项目。`--frontend` 标志选择 SPA 层：

```bash
suprnova new my-app                       # Svelte 5（默认）
suprnova new my-app --frontend svelte     # Svelte 5
suprnova new my-app --frontend react      # React 19
suprnova new my-app --frontend vue        # Vue 3.5
```

三个脚手架都共享相同的技术栈：

| 层 | 版本 |
|---|---|
| Inertia 客户端适配器 | `@inertiajs/{svelte,react,vue3}` 3.4.0 |
| 构建工具 | Vite 8 |
| 样式框架 | Tailwind v4 (`@tailwindcss/vite`) |
| TypeScript | 严格模式 |

选择是按项目进行的。服务器端没有“主要”框架 - `inertia_response!`
根据您选择的脚手架使用的扩展名 (`.svelte`, `.tsx`, `.vue`) 来解析，
`App::inertia_share`、部分重新加载和 TypeScript props 生成在三者之间都表现相同。

## 架构

```
                       Browser
   +-------------------------------------------------+
   |               SPA (Svelte / React / Vue)        |
   |   +---------------+ +---------------+           |
   |   | Home.svelte   | | Users/Show.tsx|  ...      |
   |   +-------+-------+ +-------+-------+           |
   |           |  typed props from Rust struct       |
   |   +-------v-------------------------------+     |
   |   |        Inertia client adapter         |     |
   +---+------------------+------------------+--+----+
                          |
                          |   HTTP (JSON on XHR, HTML on first load)
                          v
   +-------------------------------------------------+
   |                  Suprnova server                |
   |   +------------------------------------------+  |
   |   |          Controllers / handlers          |  |
   |   |   inertia_response!(&req, "Home",        |  |
   |   |                     HomeProps { ... })   |  |
   |   +------------------------------------------+  |
   +-------------------------------------------------+
```

首次请求返回一个 HTML 壳，其中包含嵌入在挂载节点的 `data-page` 属性中的初始页面对象。后续访问通过 `<Link>` / `router.visit` 进行，发送 `X-Inertia: true`，并获得返回一个 JSON 页面对象 - 适配器无需完整重新加载即可交换组件。

## 完整的页面往返

控制器将其 props 定义为 Rust 结构体，派生 `InertiaProps`，并将该值传递给 `inertia_response!` 宏：

```rust
use suprnova::{InertiaProps, Request, Response, inertia_response};

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

宏为您做了几件事。首先，它在编译时验证页面组件文件是否确实存在于
`frontend/src/pages/Home.{svelte,tsx,jsx,vue}` 下 - 拼写错误会导致构建错误，而不是浏览器中的 404。其次，它序列化 `HomeProps` 结构体，将其展开为每个顶级键一个 prop，以便部分重新加载可以过滤，并在返回前根据 `&req` 解析任何 lazy 或 deferred props。宏计算为 `Result<HttpResponse, FrameworkError>`，`Response` 返回类型直接接受它。

匹配的 Svelte 页面（默认脚手架）：

```svelte
<!-- frontend/src/pages/Home.svelte -->
<script lang="ts">
  import type { HomeProps } from '../types/inertia-props'

  let { title, message }: HomeProps = $props()
</script>

<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">{title}</h1>
  <p class="mt-2">{message}</p>
</div>
```

有关 React 和 Vue 对应项，请参阅 [页面组件](frontend-pages.md)。

## 生成 TypeScript 类型

您 `src/` 中的每个 `#[derive(InertiaProps)]` 结构体都会成为
`frontend/src/types/inertia-props.ts` 中的 TypeScript 接口：

```bash
suprnova generate-types
```

传递 `--routes`，同样的命令也会发出 `frontend/src/types/routes.ts` -
从您的 `routes!` 宏中提取的类型安全的 URL + 方法对，可直接用于 Inertia v2+ API。完整的类型映射表和路由辅助函数形状位于
[TypeScript 类型](frontend-typescript-types.md)。

## 共享数据

应该出现在每个页面上的任何东西（已认证用户、当前语言区域、应用元数据）都在启动时注册一次，并合并到每个 Inertia 响应中：

```rust
// 在 bootstrap.rs 中
App::inertia_share("appName", "Suprnova");
App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

// 异步 / 逐请求的共享数据通过 trait 提供。
App::register_inertia_shared(Arc::new(AppSharedData));
```

三种类型，按优先级顺序（后者在相同的键处优先）：

| API | 值何时初始化 |
|---|---|
| `App::inertia_share(k, v)` | 同步，在启动时设置一次 |
| `App::inertia_share_lazy(k, \|\| async { ... })` | 每个响应，重新计算 |
| `App::inertia_share_once(k, \|\| async { ... })` | 每个响应，然后客户端缓存 |
| `App::register_inertia_shared(Arc::new(impl))` | 每个请求，可访问 `&req` |

在响应构建器上附加的每页 props 总是在相同的键处覆盖共享数据。

## 部分重新加载和 lazy props

同一个 `InertiaResponse` 构建器暴露了 Inertia v3 的完整 prop 工具包 -
eager、lazy、optional、deferred、merge、once - 并且 Suprnova 自动响应
v3 部分重新加载头部（`X-Inertia-Partial-Data`、`X-Inertia-Partial-Except`、
`X-Inertia-Reset`、`X-Inertia-Except-Once-Props`）。下面的示例附加了三个具有不同评估规则的 props：

```rust
use suprnova::{InertiaResponse, FrameworkError, Request, Response};

pub async fn dashboard(req: Request) -> Response {
    let resp = InertiaResponse::new("Dashboard")
        .with("title", "Dashboard")
        .lazy("recent_orders", || async {
            Ok::<_, FrameworkError>(load_recent_orders().await?)
        })
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        .resolve(&req)
        .await?;
    Ok(resp)
}
```

`inertia_response!` 覆盖了 eager-props 情况；之后的所有内容都通过构建器进行。完整的表面 - `optional`、`merge`、`once`、`scroll`、
`flash`、`paginate`、SSR、版本不匹配、历史加密 - 记录在
[Inertia 响应](frontend-inertia-responses.md)中。

## 应用启动

脚手架应用在 `bootstrap.rs` 内的一个调用中安装四个协议关键的中间件：

```rust
use suprnova::{Inertia, InertiaConfig};

Inertia::install(&InertiaConfig::new().version(env!("CARGO_PKG_VERSION")))
    .expect("Inertia install failed");
```

`install` 返回 `Result` - 如果 `InertiaConfig` 解析为生产模式（`APP_ENV=production`
下的默认值）但找不到 Vite manifest，它会故障关闭，而不是悄悄地回退到传统的资产路径。见下面的 [开发与生产](#开发与生产)。

这会按顺序注册：`InertiaHeadersMiddleware`（在每个响应上设置 `Vary: X-Inertia`，并把 Inertia 访问中的空 `200` 转回 `303`）、`InertiaVersionMiddleware`（在资产版本不匹配时发出 409 + `X-Inertia-Location`，以便陈旧客户端重新加载）、`Inertia303Middleware`（在非 GET Inertia 访问时将 302 重写为 303，以确保后续请求明确是 GET），以及 `InertiaValidationRedirectMiddleware`（将 Inertia 访问上的 `422` 转为回到表单页的 `303`，并将错误 flash）。`InertiaVersionMiddleware` 和 `Inertia303Middleware` 曾需单独注册；`Inertia::install` 现在默认安装全部四个。完整的注册顺序以及每个中间件闭合的情形见 [Inertia 响应](frontend-inertia-responses.md#bootstrap-inertia-install)。

## 开发与生产

在开发中，Vite 开发服务器与后端并行运行，并提供启用 HMR 的资产：

```bash
suprnova serve
```

这会启动 Rust 服务器和 `vite` 一起。HTML 壳从 `http://localhost:5765` 加载模块。

对于生产，一次性构建前端并将后端指向 `public/assets/` 下的哈希 manifest：

```bash
cd frontend && npm run build
APP_ENV=production suprnova serve --backend-only
```

`InertiaConfig::default()` 从 `APP_ENV` 派生生产与开发模式（通过
`Environment::detect().is_production()`）- `APP_ENV=production` 是使 HTML 壳加载已构建的资产而不是 Vite 开发服务器的原因。`Inertia::install` 接着会在找不到支持该决定的 manifest 时明确地启动失败，而不是悄悄地回退到陈旧的硬编码路径。

Suprnova 读取 `public/assets/.vite/manifest.json` 来解析哈希入口点加上 `modulepreload` 的任何传递导入。SSR 是可选的 - 通过将 `InertiaConfig::ssr(...)` 指向运行的 `@inertiajs/{vue3,react,svelte}/server` 工作进程来选择加入。`suprnova new` 会为每个 starter 脚手架生成 SSR 入口点和构建脚本，而 `suprnova ssr:start` / `suprnova ssr:check` 会运行并验证该工作进程；完整设置（包括 bundle 存在性检查和 CSR 回退行为）见 [Inertia 响应](frontend-inertia-responses.md#ssr)。

### 为什么 Suprnova 有所不同

与典型 Inertia 设置在其他地方的外观不同的三个有意的偏差：

- **编译时组件验证。** `inertia_response!` 宏在构建时遍历
  `frontend/src/pages/`，如果组件文件缺失，拒绝展开并建议最接近的匹配。您无法提交指向已删除页面的控制器。
- **类型化 props 作为真实来源。** 页面 props 是带有
  `#[derive(InertiaProps)]` 的 Rust 结构体。`suprnova generate-types`
  读取它们并写入 TypeScript 接口 - 前端类型从后端派生，不是并行维护的。
- **Svelte 作为默认。** Inertia 的文档首先涉及 Vue 和 React；
  Suprnova 脚手架默认为 Svelte 5（runes-on）。React 19 和 Vue 3.5 是一等的，不是附带的想法 - 相同的协议、相同的 prop 管道、相同的生成器输出。

## 下一步

- [页面组件](frontend-pages.md)
- [Inertia 响应](frontend-inertia-responses.md)
- [TypeScript 类型](frontend-typescript-types.md)
- [路由](routing.md)
- [控制器](controllers.md)
