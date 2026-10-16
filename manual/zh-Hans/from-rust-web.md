# 从 Rust Web 来

您已经在 Axum、Actix、Rocket 或手写的 hyper 上部署过 Rust 服务。您了解这门语言和运行时。那么 Suprnova 到底能给您带来什么呢？

**生产力层。** 路由、控制器、ORM、迁移、队列、任务调度、认证、邮件、通知、广播、缓存、存储、验证和类型化的前端桥接 - 所有这些都相互连接，全部使用相同的约定，完全生产就绪。您编写控制器和模型；不需要您选择布局。

如果您已经在 Axum 上构建过一两个真实应用，您就知道其中有多少工作是布线而不是功能。Suprnova 就是这种布线，一次完成，在需要的地方有主见，在不需要的地方可插拔。

## 30 秒的总结

```bash
suprnova new myapp --frontend svelte    # 生成后端 + SPA + Vite
cd myapp
suprnova db:sync                        # 运行迁移，重新生成实体
suprnova serve                          # 后端 + Vite 开发服务器
```

现在您拥有了：

- 一个支持 HTTP/1.1 和 HTTP/2、WebSocket 升级、优雅关闭的 hyper 服务器
- 一个由 SeaORM 支持的 Eloquent 层，具有关系、预加载、软删除
- Inertia.js 使用类型化 `#[derive(InertiaProps)]` 桥接 Rust 和 Svelte 5
- 由框架认证守卫和中间件，加上 Magnetar 支撑的密码、passkey、魔法链接、OAuth、Bearer 会话、锁定和记住我引擎组成的认证
- 一个支持 memory/sync/redis/database/null 驱动程序的队列
- 由 `Task` trait 驱动的 cron 调度程序
- 每个项目的控制台二进制文件，用于 `cargo run --bin console <cmd>`
- 缓存、存储 (fs/s3/azblob/gcs)、邮件 （SMTP + 5 个提供商: SES、Mailgun、Postmark、SendGrid、Resend）、Web 推送
- 通过可插拔的中枢 （默认为 sea-streamer） 进行广播
- 验证、CSRF、CORS、速率限制、幂等性、请求超时、结构化错误

以及 `cargo build --release` 最后生成的一个静态链接二进制文件。

## 底层构成

| 事项 | Crate |
|---|---|
| HTTP 服务器 | `hyper` + tower 风格的中间件 （自有实现） |
| 异步运行时 | `tokio` |
| 路由器 | `matchit` |
| ORM | `sea-orm` （重新导出为 `suprnova::sea_orm`） |
| 迁移 | `sea-orm-migration` |
| 数据库驱动程序 | `sqlx` (postgres / mysql / mariadb / sqlite) |
| 序列化 | `serde` / `serde_json` |
| 验证 | `validator` |
| 浏览器会话 | 框架的 `SessionMiddleware` 和可插拔的会话存储 |
| 认证引擎 | 框架自有门面背后的 `suprnova-magnetar` |
| 模板 | `tera` （用于邮件正文；前端使用 Inertia） |
| 加密 | `aes-gcm`、`argon2`、`bcrypt` |
| WebSocket | `hyper-tungstenite` |
| 流处理 | `sea-streamer` （广播分发后端） |
| OAuth | Magnetar 提供商注册表和握手引擎 |
| 追踪 | `tracing` + `tracing-subscriber` |

您通常不会直接使用这些 crate 中的任何一个 - Suprnova 会重新导出您需要的内容。SeaORM 是最深的传递层：`Entity`、`Column`、`ActiveModel`、`ConnectionTrait`、查询构造器、迁移前导。如果您需要精选表面不覆盖的某些功能，脱围机制是 `use suprnova::sea_orm;`。

## Suprnova 相比原生 Axum 的优势

Axum 很出色。Actix 也是。Rocket 也是。Suprnova 存在的原因不是这些框架有问题 - 而是每个用它们构建真实产品的团队都会重新实现相同的生产力层。Suprnova 提供了这一层：

| 功能 | 在 Axum 上手写 | 在 Suprnova 中 |
|---|---|---|
| 能够扩展到数百条路由的路由宏 | Builder API，可能会变得冗长 | `routes!` 宏，支持分组、前缀、中间件、命名 |
| 路由模型绑定 （路径 id → 加载的模型） | 每个类型都需要自定义提取器 | `#[handler]` 从 `{id}` 自动解析 `post::Model` |
| Eloquent 风格的可链式查询构造器 | 直接使用 SeaORM | `Post::query().db_where(...).order_by(...).get().await?` |
| 软删除、观察者、生命周期事件 | 按模型构建 | `#[model(soft_deletes)] + impl Observer<Post>` |
| 迁移 + 实体生成 | 连接 sea-orm-cli + 脚本 | `suprnova db:sync` 运行迁移并重新生成实体 |
| 认证 （会话、提供商、认证守卫） | 拼接 tower-sessions + 自有逻辑 | `Auth::attempt`、`Auth::user`、每条路由的 `.middleware(AuthMiddleware)` |
| 电子邮件验证、密码重置、2FA、暴力防护 | 手动构建所有四个功能 | 全部内置，可配置，幂等 |
| 后台队列 | 选择驱动程序，编写工作进程 | `Queue::push` + `cargo run -- queue:work` |
| Cron 任务调度 | 用 `tokio_cron_scheduler` 编写 tokio 任务 | `impl Task` + `Schedule::task(...).daily().at("03:00")` |
| Inertia 桥接 | 构建提取器 + JS 适配器 | `inertia_response!(&req, "Page", props)` |
| 类型化前端 props (Rust → TS) | 编写生成器 | `#[derive(InertiaProps)]` + `suprnova generate-types` |
| 广播 （公开 / 私有 / 呈现频道） | 连接流处理后端 + 认证 | `BroadcastHub` + `Channel`/`PrivateChannel`/`PresenceChannel` trait |
| 多提供商邮件 | 选择一个，编写自己的抽象 | `Mail::driver("ses")` 等等，统一的 `Mailable` API |
| Web 推送 | 阅读规范，构建通知程序 | `WebPushChannel` 内置，VAPID 已集成 |
| 验证 + 表单请求 | 使用 `validator` + 自定义提取器 | `#[derive(Data, Validate)]` 表单请求，异步验证 |
| JSON:API 资源 | 手动格式化响应 | `#[derive(Resource)]` |
| 具有 fail-open/closed 策略的速率限制 | 构建它 | `RateLimiter` + `BackendErrorPolicy` |
| 幂等键 | 构建它 | `Idempotency::remember(key, ttl, body)` 带有 Stripe 风格的重放 |
| CSRF （具有 Laravel 风格的 glob 排除） | 构建它 | `CsrfMiddleware`，支持 `except` + `except_method` |
| 结构化错误，带有清理的 5xx | 构建它 | `FrameworkError` / `HttpError` trait，panic 恢复 |
| 容器，具有任务本地 → 线程本地 → 全局作用域 | 编写您自己的 | `App::bind` / `singleton` / `factory`，带有正确的隔离 |
| 健康检查端点、请求 id、结构化日志 | 粘合在一起 | 默认全部启用 |

权衡在于观点：Suprnova 选择布局、选择默认驱动程序、选择命名约定。您可以背离 （驱动程序是可插拔的、配置是可覆盖的、容器让您交换服务），但这些默认设计是“快速构建产品”的正确选择。

## 熟悉的 Rust 模式

您会认可这些形式：

```rust
// 处理程序返回 `Result<HttpResponse, HttpResponse>`（别名 Response）。
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    Ok(HttpResponse::json(serde_json::json!({ "post": post })))
}

// 中间件是一个 trait，不是闭包：
#[async_trait]
impl Middleware for RequireAdmin {
    async fn handle(&self, req: Request, next: Next) -> Response {
        let user = Auth::user_as::<User>().await?
            .ok_or_else(|| HttpResponse::text("Unauthorized").status(401))?;
        if !user.is_admin {
            return Err(HttpResponse::text("Forbidden").status(403));
        }
        next(req).await
    }
}

// 后台工作是 `Job` trait - `handle(self)` 运行该作业：
#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}
```

如果您习惯于 Tower 中间件：Suprnova 中间件在概念上是相同的 （围绕 `next` 的包装），但使用自有的 trait （不是 Tower 的 `Service`），因为当您开始嵌套特定于应用程序的提取器时，tower 的组合子类型会变得很复杂。形式更简单；心智模型是相同的。

如果您使用过 Axum 的提取器模式：Suprnova 的 `#[handler]` 宏扮演相同的角色，但通过服务容器解析而不是通过 trait，这让它可以注入应用服务以及请求数据。路由模型绑定 （`Post` 来自 `{id}`） 是内置的。

如果您直接使用过 `sqlx`：Suprnova 的 ORM 位于 SeaORM 之上，SeaORM 位于 sqlx 之上。您可以通过 `DB::select(...)` / `DB::select_one(...)` 下降到原始 SQL 或使用 `DB::table("name")` 进行可链式的动态查询；您也可以直接下降到 SeaORM 用于 Eloquent 表面不覆盖的东西 （例如带有自定义结果映射的原始 `Statement` 查询）。[Eloquent 章节](eloquent.md)覆盖了脱围机制。

## 生产力优势是什么？

挑一个您在原生 Axum 中之前构建过的功能。Suprnova 把它作为一个章节提供：

- **“我曾经构建过一个认证系统，花了两周时间。”** →
  [认证](authentication.md) + [认证流程](auth-flows.md)。设置迁移，配置认证守卫，完成。
- **“我编写了自己的队列工作进程，支持重试/退避。”** →
  [队列](queues.md)。`Queue::push` + `cargo run -- queue:work`。
- **“我曾经用 hyper-tungstenite 连接过 WebSocket。”** →
  [WebSocket](websockets.md)。`ws!()` 宏为处理程序提供类型；升级、ping/pong 心跳、关闭帧握手和反压都被处理了。
- **“我从头开始构建了一个 Inertia 适配器。”** →
  [Inertia](frontend.md)。`inertia_response!(&req, "Page", props)`，带有 `InertiaProps` 生成 TS 类型。
- **“我构建了一个每租户速率限制器。”** →
  [速率限制](rate-limiting.md)。可配置的键，可配置的 fail-open vs fail-closed 策略，fail-closed 返回 503。
- **“我实现了 Stripe webhook 签名验证 + 重放保护。”** →
  [支付: Stripe](payments-stripe.md)。内置在适配器中，webhook 进入具有 UNIQUE 幂等性的镜像表。

您需要用两周时间手动构建的东西，您可以用一行导入。

## 您仍然会认可的“属于您的”部分

某些事情仍然保持接近原生 Rust，因为这门语言为您提供了比框架抽象更好的东西：

- **并发原始工具。** `tokio::spawn`、`Arc`、`Mutex`、channel - 使用它们。框架不会包装它们。
- **错误类型。** 您定义您的域错误。在它们上实现 `HttpError` trait 以在网络响应中获得正确的状态码 + 消息。框架的 `FrameworkError` 和 `AppError` 分别是跨切和临时错误的脱围机制。
- **自定义驱动程序。** 缓存、队列、邮件、广播、向量、支付 - 每个“驱动程序注册表”子系统都接受自定义驱动程序。实现该 trait，在 `bootstrap.rs` 中注册，完成。
- **当您想要时的原始 SQL。** `DB::select(...)`、`DB::table(...).get()` 用于动态行，或完全下降到 SeaORM。ORM 不会妨碍您。
- **您自己的 Tower 中间件?** Suprnova 不提供 Tower 适配器 - 这里的中间件是 `impl Middleware`，不是 `tower::Service`。如果您需要引入仅限 Tower 的 crate，您需要手动适配它。实际上，内置中间件系统涵盖了您会使用的几乎所有东西。参见 [中间件](middleware.md)。

## 您放弃的东西

诚实比营销更重要：

- **约定。** 模型住在这里，控制器住在那里，迁移住在那里，观察者住在那里。脚手架作出选择。您可以反抗；您可能不应该这样做。这些约定是 Laravel 的，经过审计和实战测试。
- **请求流动方式的灵活性。** 中间件链有固定的最外层顺序 (request-id → globals → route middleware → handler)。您可以在其中任何地方插入中间件，但您不能移动 request-id 或 panic-recovery 层 - 它们是不变量。
- **PHP 风格的拐角。** 在 Laravel 因为 PHP 而做某事的地方，Suprnova 做 Rust 风格的事情 - 但我们会告诉您什么时候。在章节中寻找**“为什么 Suprnova 有所不同”**的说明。

## 为什么“Laravel 启发”应该对您很重要，即使您从未写过 PHP

Rust Web 生态系统大致处于 PHP 在 2009 年左右的位置。Crate 存在；模式不存在。Suprnova 从一个经历了 10 多年生产压力塑造的框架中移植了一套极其精炼的模式。您得到已经在现实中经受过考验的模式。

代价是 Suprnova *有主见*。如果您想要一个最小的“自己选择一切”框架，Axum 就在那里，它很出色。如果您想要一个“框架为您决定事情，这样您可以专注于产品”，那就是 Suprnova。

## 后续步骤

- [安装](installation.md) - `suprnova new`，生成什么
- [快速上手](quickstart.md) - 在 5 分钟内构建一个小应用
- [请求生命周期](lifecycle.md) - 请求如何流动，什么在哪里运行
- [服务容器](container.md) - 服务如何绑定和解析
- [Eloquent](eloquent.md) - 最长的章节；表面很广

或者通过 [`documentation.md`](documentation.md) 跳到任何地方。
