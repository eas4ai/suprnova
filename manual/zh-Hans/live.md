# Live

Suprnova Live 是框架的服务器驱动交互引擎。一个 Live 组件是一个 Rust 结构体：它的状态保存在服务器上，它的视图是一个 Askama 模板，它的动作通过签名协议从一个小型浏览器运行时执行，该运行时把重新渲染的 HTML 就地变形。没有需要保持同步的客户端状态模型，使用随附运行时无需安装任何构建工具，文档中也没有内联 JavaScript。

本章覆盖面向应用的表面：编写组件、注册组件、提供文档与岛屿、每个 Live 请求穿越的安全边界、上传、异步更新、资产、测试、诊断以及恢复。这里的一切只使用
`suprnova::live` 和 `suprnova::view`。

## 快速上手

由 `suprnova new` 创建的项目已经为 Live 做好准备：它附带带有空组件注册表和
`routes()` 函数的 `src/live/mod.rs`，其引导绑定注册表，`cmd/main.rs` 安装路由。生成一个组件，然后检查它：

```bash
suprnova live:make Counter
suprnova live:check
```

`live:make` 写入 `src/live/counter.rs` 与 `templates/live/counter.html`，在
`src/live/mod.rs` 中注册组件，并打印后续步骤。`live:check` 构建你的应用，并用集成检查器证明每一个已注册的视图。

## 编写组件

```rust
use suprnova::live::{LiveComponent, live};

/// A counter rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

- `name` 是注册的组件名。使用带点的 kebab-case 名称，例如 `app.counter`；CLI
  推导出 `<package>.<kebab>`。
- `view` 是相对于模板根目录的模板标识。
- `#[public]` 字段会被渲染并携带在签名快照中。`#[model]` 字段还通过 `live:model`
  接受来自浏览器的提议。
- `#[action]` 方法是浏览器唯一可以调用的入口。它们接收经过验证的参数，并可返回重定向或 flash 等类型化结果。

每个字段类型都必须实现 `Default`；除非挂载钩子另有指定，新岛屿从这些默认值开始。

## 视图

视图是 Askama 模板。除非 `askama.toml` 指定了其他目录，模板根目录是
`templates/`，因此 `live/counter.html` 位于 `templates/live/counter.html`：

```html
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

指令使用封闭的 `live:` 语法：`live:click`、`live:submit`、`live:model`、
`live:upload`、`live:key`、`live:loading` 以及文档记录的其余集合。检查器针对组件证明每一条指令：未知的动作、未知的模型字段、原始的 `safe` 过滤器或无障碍违规都会使 `live:check` 失败，并给出文件、行和列。

放置岛屿的文档是用 `#[suprnova::view]` 声明的普通视图；它们接受的唯一未转义值是通过 `trusted_html` 过滤器传入的 `TrustedHtml`。

## 注册与引导

`src/live/mod.rs` 拥有注册表和路由：

```rust
use suprnova::live::{LiveRegistry, RegistryError};

pub mod counter;

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .register::<counter::Counter>()?
        .build();
    Ok(registry)
}
```

在引导期间绑定它，使服务器、worker 以及 `suprnova live:*` 命令看到同一组组件：

```rust
suprnova::App::singleton(crate::live::registry().expect("Live component registry"));
```

运行时组装完成后，注册表即不可变。重复的组件名或视图，或者动作需要验证却没有验证端口的组件，都会以类型化的 `RegistryError` 使注册失败。

## 路由

`Router::try_live()` 恰好安装一次保留命名空间：`/__live/action`、
`/__live/upload`、`/__live/async/*` 的控制路由与 WebSocket 握手，以及不可变的
`/__live/assets/*` 路由。如果某条应用路由能够占据 `/__live`，启动将失败。

保留的请求路由带有严格策略：每个请求都需要会话、来源、CSRF、主体、租户和限流事实。框架记录会话和 CSRF 证明；你的应用通过路由守卫附加其余部分：

```rust
use std::sync::Arc;
use std::time::Duration;

use suprnova::live::{LiveTenantMiddleware, LiveTenantResolver};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::{AuthMiddleware, FrameworkError, RateLimitMiddleware, Request, Router, SlidingWindowConfig, async_trait};

pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let limiter = Arc::new(InMemoryRateLimiter::new());
    router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::optional())
            .middleware(LiveTenantMiddleware::new(Arc::new(SingleTenant)))
            .middleware(RateLimitMiddleware::new(
                limiter,
                SlidingWindowConfig { max_requests: 600, window: Duration::from_secs(60) },
                |request: &Request| format!("live:{}", request.ip().unwrap_or_else(|| "anon".into())),
            ))
    })
}

struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
```

从入口点安装路由，使运行时和挂载目录在第一个请求之前就绪：

```rust
Application::new()
    .bootstrap(bootstrap::register)
    .try_routes(|| live::routes(routes::register()))
    .run()
    .await;
```

## 文档与岛屿

文档路由声明一次其岛屿，通过 `LiveDocument` 渲染它们，并输出引导标签：

```rust
use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, Router, StatusCode};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "live/page.html")]
struct Page<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
}

pub fn install(router: Router) -> Result<Router, FrameworkError> {
    let mount = LiveMount::<Counter>::identity_bound("/dashboard", "counter", "dashboard-counter")?;
    let handler_mount = mount.clone();
    let router: Router = router
        .get("/dashboard", move |request: Request| {
            let mount = handler_mount.clone();
            async move { render(request, &mount).await }
        })
        .middleware(AuthMiddleware::redirect_to("/login"))
        .into();
    router.try_live_mount(&mount)
}

async fn render(request: Request, mount: &LiveMount<Counter>) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(mount, CanonicalValue::Object(BTreeMap::new()), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                ViewName::parse("live/page.html").map_err(|_| FrameworkError::internal("view"))?,
                &Page { bootstrap: bootstrap.html(), counter: counter.html() },
                DocumentResponseIntent::html(StatusCode::OK).map_err(|_| FrameworkError::internal("intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|_| HttpResponse::text("Live document failed").status(500))
}
```

- `LiveMount::public_seed` 声明任何访客都可以渲染的岛屿；其状态是一个可复用的种子，在第一次动作时提升为实例。
- `LiveMount::identity_bound` 声明属于当前会话和主体的岛屿；文档路由必须进行认证。
- 在 `bootstrap` 之前挂载每个岛屿，并且只调用一次 `bootstrap`。引导输出惰性的配置元素以及 ESM 或经典策略的脚本标签，在挂载的组件需要时添加上传与异步角色，并按需添加 Stimulus 桥接。
- 文档模板把 `{{ bootstrap|trusted_html }}` 放在 `<head>` 中，并把每个岛屿放在它所属的位置。

## 安全边界

Live 从不绕过框架的中间件。每个请求需要的内容：

| 事实 | 记录者 |
|---|---|
| 会话 | `SessionMiddleware` |
| 来源与 CSRF | 启用来源验证的 `CsrfMiddleware` |
| 主体 | 处于已认证分支的 `AuthMiddleware` |
| 租户 | 带有你的解析器的 `LiveTenantMiddleware` |
| 限流 | 处于放行分支的 `RateLimitMiddleware` |

随附的运行时发送 Live 媒体类型和浏览器自身的 `Sec-Fetch-Site` 头；它不携带会话令牌。无论你配置了哪种来源策略，CSRF 中间件都会自行为每个 Live 请求验证这一证明：同源的 Live 请求以无状态 CSRF 判定通过，而跨站或缺少该头的请求回退到令牌验证并被拒绝。普通路由在默认策略下保留令牌验证；使用 Live 不会放松其他任何东西：

```rust
global_middleware!(CsrfMiddleware::new());
```

匿名访客可以渲染公共种子，并且在守卫使用 `AuthMiddleware::optional()` 时可以对其执行动作：已登录的主体会被记录，匿名访客继续通行，由挂载类型决定。公共种子随后在首次动作时为访客自己的会话完成晋升，而绑定身份的岛屿仍然拒绝没有主体证据的请求。使用 `AuthMiddleware::new()` 时，守卫在任何引擎工作之前就对每个匿名请求以 `401`
应答。绑定身份的岛屿需要会话和主体；只要你的解析器指定了租户，租户就会绑定到岛屿的作用域中，而无法确定租户的解析器必须返回错误而不是 `None`。每一次拒绝都是封闭的：对过期或被篡改快照的 `409`
不携带正文，生产环境的消息从不包含快照、令牌、Cookie 或渲染后的 HTML。

## 上传

在模型字段上声明上传策略：

```rust
use suprnova::live::{LiveComponent, UploadPolicy, UploadReplacement, UploadScan, UploadType, live};

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(512 * 1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

#[derive(LiveComponent)]
#[live(name = "app.avatar-uploader", view = "live/avatar-uploader.html")]
pub struct AvatarUploader {
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl AvatarUploader {
    #[action]
    pub fn save_avatar(&mut self) {}
}
```

视图通过 `<input type="file" live:upload="avatar">` 绑定该字段。运行时通过
`/__live/upload` 创建、传输并完成上传；文件在隔离区等待，直到声明的最终化动作运行，此时框架把它交给你的 `UploadFinalizer`。在运行时组装之前绑定最终化器，以及任何扫描器或验证器：

```rust
App::singleton(LiveUploadHost::new().with_finalizer(Arc::new(AppUploadFinalizer::default())));
```

上传通过 gate 按字段和控制进行授权。为 `Create`、`Reacquire`、`Status`、`Queue`、
`BeginTransfer`、`PutChunk`、`Complete`、`Accept`、`BeginFinalize`、
`CommitFinalize`、`Cancel`、`Reject`、`Expire` 和 `Fail` 定义能力
`live:<component>.upload.<field>.<Control>`。

丢失传输授权的浏览器通过你的应用在保留命名空间之外拥有的一条路由重新获取它：

```rust
let router: Router = router
    .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")?
    .middleware(AuthMiddleware::new())
    .into();
```

该路由要求与动作相同的事实，只应答创建该上传的会话和主体，并返回带有当前传输状态的新授权。

## 异步更新

组件声明它监听的流；浏览器运行时通过 SSE 或 WebSocket 订阅，并回退到轮询：

```rust
use suprnova::live::{EventPayloadMetadata, LiveComponent, live};

pub struct ActivityPosted;

impl EventPayloadMetadata for ActivityPosted {
    const NAME: &'static str = "activity.posted";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "app.activity-feed",
    view = "live/activity-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "activity", topics("activity"), events(ActivityPosted)))
)]
pub struct ActivityFeed {
    #[public]
    headline: String,
}
```

为订阅者定义能力 `live:<component>.stream.<name>`，然后从应用的任何地方发布：

```rust
let streams = LiveStreams::resolve()?;
streams.event::<ActivityPosted>("activity", LiveEventTarget::Island, payload).await?;
streams.refresh("activity").await?;
```

刷新告诉已订阅的岛屿重新渲染；事件被投递到岛屿注册的处理程序。轮询就是普通的重新渲染：传输不可用时岛屿的状态会追平，但其间发布的事件负载不会重放给它们的处理程序，运行时会把该流报告为降级而非最新。恰好声明一个流的组件会让其岛屿根订阅该流；拥有多个流的组件通过运行时的已注册调用逐个订阅。

流会随着打开它的会话一起结束。当持有该流的节点上的会话被销毁时，无论是普通注销、失效、ID 重新生成还是“在所有设备上注销”，该会话在那里打开的每个成员资格都会立即退出，之后的事件不会再到达它。在另一个节点上被销毁的会话由投递本身捕获：每个成员资格的会话每十秒至多一次地向会话存储重新核对，因此事件会在这个间隔内停止。无论如何，流的 gate 在每次投递前都会被再次询问，所以策略变更会在每个节点上立即结束投递。

## 资产与免构建使用

框架在 `/__live/assets/<identity>/<file>` 提供经过审阅的精确运行时工件，带有不可变缓存、强验证器以及引导标签中的完整性属性。由于文档不包含内联脚本，严格的
`script-src 'self'` 策略得以成立。要把相同的字节发布到 CDN 或静态目录：

```bash
suprnova live:assets --out public/__live
```

发布是原子的，除非传入 `--replace`，否则拒绝替换字节不同的目录。

## 组件库

Suprnova 随附了面向 Live 的组件库基础：一份带基础层的令牌样式表，以及一组建立在原生控件和 `live:model`、`live:error`、`live:loading` 词汇之上的表单类展示型组件。基础层是一个运行时产物。文档一旦选择加入，它就会作为一个样式表链接送达，与运行时脚本处于同一套标识、完整性与缓存契约之下：

```rust
let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
```

其中的每一条规则都位于级联层 `suprnova-ui` 之内，因此你自己未分层的样式无需争夺优先级就能胜出。每一个视觉值都是一个 `--sn-` 自定义属性，覆盖颜色、字体、间距、圆角、阴影、动效、密度与状态，并同时具有浅色与深色取值：在 `:root` 上覆盖一个令牌即可换肤，或者去掉整个层而保留每一种行为、名称与状态属性，因为组件是根据检查器所证明的属性（`aria-invalid`、`aria-busy`、`aria-expanded`、`aria-pressed`、`aria-current`、`aria-selected`、`:disabled`）来呈现状态的，从不依赖类名。一份 Tailwind CSS 4 的 `@theme` 预设把这些令牌映射到 Tailwind 的命名空间；Tailwind 从来不是必需的。组件通过 `live:add` 安装，每个组件在保留根目录 `templates/suprnova-ui/` 下占一个目录：Askama 宏视图、样式表、组件若有则包含的 JavaScript，以及为它们命名的清单：

```bash
suprnova live:add field
suprnova live:add password-input
```

你编辑过的文件在后续运行中会被保留；`--force` 会替换它。第三方组件通过 `--manifest` 从自己的清单安装到自己的根目录下。在视图中调用这些宏，用 `try_live_ui_assets()` 提供打包进来的样式表与脚本，并在文档中链接它们：

```html
{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% call field::field("email", "Email", required=true) %}
{% call input::input("email", kind="email", required=true) %}{% endcall %}
{% endcall %}
```

检查器会展开这些宏，所以 `live:check` 能像证明任何其他视图一样证明库视图。目前的表单家族包括：字段、标签、输入框、文本域、数字输入、滑块、搜索输入、带显示切换的密码输入、复选框与复选框组、单选组、开关、下拉选择、按钮与链接按钮、按钮组、fieldset、表单操作栏、校验摘要以及文件输入。库组件命名为 `suprnova.*`，注册表会拒绝来自任何其他 crate 的该前缀；自定义元素位于 light DOM 并带有 `sn-` 前缀。

浮层一族建立在同样的基础之上：工具提示、可折叠块与手风琴、弹出层、单层下拉菜单、对话框、抽屉面板和侧边抽屉。每一个都在任何脚本运行之前，通过浏览器自身的原语保持打开状态：折叠内容用 `details`，弹出层和菜单用 `popover` 属性，三种模态用 `dialog`，由随组件安装的 `sn-dialog`、`sn-sheet` 和 `sn-drawer` 元素通过 `showModal()` 打开，关闭时把焦点交还给触发器。打开和关闭从不发出 Live 请求；只有你放进浮层里的动作才会。每个浮层根都带有稳定的键和 `live:preserve.self`，所以打开的浮层能够在没有替换其区域的 morph 之后继续保持打开：

```html
{% import "suprnova-ui/dialog/dialog.html" as dialog %}
{% call dialog::dialog_trigger("confirm", "Delete everything", variant="danger") %}{% endcall %}
{% call dialog::dialog("confirm", "confirm", "Delete everything?") %}
<p>This removes every note.</p>
{% call button::button("Delete", action="confirm_delete", variant="danger") %}{% endcall %}
{% call dialog::dialog_close("confirm", "Cancel") %}{% endcall %}
{% endcall %}
```

`popover` 属性把支持的基线定在 Chrome 与 Edge 114、Firefox 128 和 Safari 17。在存在 CSS 锚点定位的地方，弹出层和菜单位于触发器之下；否则由浏览器居中显示。手风琴的单项展开模式依赖 `details name`，较旧的受支持版本会把它们当作彼此独立的折叠块。

### 为什么 Suprnova 与众不同

Laravel 随附 Blade 组件和入门套件的标记；Suprnova 则通过框架本身、基于 Live 自己的词汇来提供这个库，不让任何客户端应用拥有页面。皮肤默认开启，去掉它也不会破坏任何东西，这正是这里“无头”的含义。

## 测试

`suprnova::live::testing` 为进程内测试准备路由器的运行时和挂载目录。
`app/tests/live_*.rs` 中的应用测试展示了完整模式：内存数据库、预置的会话
Cookie、真实的全局中间件栈，以及通过 `handle_request` 发出的请求：

```rust
let router = app::live::routes(app::routes::register())?;
let runtime = prepare_live_router_for_test(&router)?;
App::singleton(runtime.clone());
```

从岛屿的 `data-suprnova-live-snapshot` 属性解码其快照，带上会话 Cookie 和
`Sec-Fetch-Site: same-origin` 提交一个动作，然后断言被接受的渲染结果。过期快照以空正文应答 `409`；缺少主体则应答 `401`。

## 诊断与运维

- `suprnova live:check` 证明每一个已注册的视图；`--allow-unproved` 接受检查器刻意不做断言的动态结构。
- `suprnova live:inspect` 报告已绑定的注册表、配置上限、已安装的上传能力、已组装的运行时服务以及资产标识，而不暴露状态或秘密。
- `LiveConfig` 限制请求和响应字节数以及受信上下文的生命周期；在运行时组装之前绑定自定义配置。
- 错误携带封闭的种类，例如 `live_document_context_rejected` 和
  `invalid_live_bootstrap`；遥测标签是封闭的枚举。

## 恢复

- `409` 告诉运行时重新渲染岛屿；操作不会被重放。
- 已关闭的异步传输被退役，运行时以新的传输代际重新连接；过期的代际会被拒绝。
- 过期或轮换的会话使绑定身份的工作失效；应用展示其登录路径，访客从新文档继续。

Live 在没有 RenderCache 的情况下完整运行。缓存 Live 文档是 RenderCache 的职责；参见 [RenderCache](render-cache.md)。

## CLI 参考

| 命令 | 用途 |
|---|---|
| `suprnova live:make <name>` | 生成组件及其视图并注册 |
| `suprnova live:check` | 用集成检查器证明每一个已注册的视图 |
| `suprnova live:inspect` | 报告运行时、注册表、提供者和工件的安全状态 |
| `suprnova live:assets --out <dir>` | 原子地发布经过审阅的运行时工件 |
