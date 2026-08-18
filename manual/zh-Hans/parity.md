# Laravel 对等映射

这是 Laravel 13.x 与 Suprnova 之间诚实的逐项功能映射。当您想问“Suprnova 有没有 X？”并想要一行拿到是/否/在哪里的答案时，请查这份表。

各节的顺序对照 Laravel 文档索引的顺序，好让一位 Laravel 开发者可以从头到尾一路扫下来。在每一节内部，列始终是同一套：

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|

**状态**列使用四个取值：

| 符号 | 含义 |
|---|---|
| **已实现** | 同样的接口，同样的行为（方法名往往也相同） |
| **路径不同** | 目标相同，形态不同 - 因为 Rust 让更好的选择成为可能 |
| **尚未实现** | 确实已经规划，只是还没有落地 |
| **刻意不做** | 不会实现 - 原因见备注列 |

相关的章节（如果存在的话）会从**备注**列链接过去。

这是一份持续更新的地图。Suprnova 覆盖了 30 个已文档化领域里 Laravel 13.x 的每一处表面；下面列出的空白，是这个已发布框架目前真实存在的空白。

## 架构概念

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| 请求生命周期 | `Application` → `Server` → `handle_request` 这条链 | 已实现 | [请求生命周期](lifecycle.md) |
| 服务容器 | `Container` + `App` 门面，三层（任务 / 线程 / 全局） | 路径不同 | 逐请求用任务本地，测试用线程本地 - [服务容器](container.md) |
| 上下文绑定（`when()->needs()->give()`） | 没有上下文绑定 - 每个容器层里，每个 trait 只有一个绑定 | 刻意不做 | 容器以 `TypeId` 为键，没有运行时反射可以按“谁在发问”来给一个绑定建键。请显式地组合：把依赖传进去，或者为每个消费方绑定一个不同的 newtype。[服务容器](container.md) |
| 服务提供者 | `bootstrap()` 函数 + `#[service]`、`#[policy]`、`#[command]`、观察者宏 | 路径不同 | 没有注册类 - bootstrap 就是一个函数；这些宏用 `inventory` 做编译期注册。[应用启动](bootstrap.md) |
| 门面 | 静态的 `App::get`、`Cache::*`、`Mail::*`、`Auth::*`、`Storage::*`、`Queue::*`、`Bus::*`、`Event::*`、`Notification::*`、`Gate::*`、`Schedule::*`、`DB::*`、`Vector::*` | 已实现 | 调用形态相同；这些门面是真实的类型，不是别名 |
| 契约 | trait - `Mailer`、`KeyValueStore`、`Hasher`、`Channel`、`VectorDriver`、`Evaluator`、`PaymentProvider` 等等 | 已实现 | 所有公开接缝都落在 trait 上；按 trait 绑定，实现可以随意替换 |

## 开始使用

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| 安装 | `cargo install --git …suprnova-cli`，然后 `suprnova new <name>` | 已实现 | [安装](installation.md) |
| 配置 | 通过 `#[derive(Config)]` + `Config::register` 实现的有类型配置 | 路径不同 | 编译期有类型，而不是数组包。[配置](configuration.md) |
| Agentic Development（AI 辅助开发） | 框架里没有一等公民的 AI SDK | 刻意不做 | 用您本来就会用的那些 crate（`async-openai`、`anthropic-rs`、`tokenizers` 等），挂在 `App::bind(Arc<dyn YourLlm>)` 下面 |
| 目录结构 | `src/{actions,bootstrap,controllers,middleware,models,routes}` | 已实现 | 意图相同，布局是 Rust 惯用风格。[目录结构](structure.md) |
| 前端 | Inertia v3，跑在 Svelte 5 / React 19 / Vue 3.5 之上 | 已实现 | [前端](frontend.md)、[页面组件](frontend-pages.md)、[TypeScript 类型](frontend-typescript-types.md) |
| 起步套件 | **Nebula**（认证）和 **Pulsar**（完整的产品站点），外加朴素的 `suprnova new` 脚手架 | 已实现 | 目前有两个起步套件 - Nebula 是 Breeze 的对应物；Pulsar 加上了文档、博客、社区和 RBAC。[起步套件](starter-kits.md) |
| 部署 | 单一二进制文件；Docker / Railway / DO / Hetzner 部署方案 | 路径不同 | 一份构建产物，而不是 PHP 运行时 + opcache + FPM。[部署](deployment.md) |

## 基础

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| 路由定义 | `routes!` 宏 + `get!` / `post!` / `put!` / `patch!` / `delete!` / `any!` / `head!` / `options!` / `fallback!` / `ws!` | 已实现 | [路由](routing.md) |
| 路由参数 | `{id}` 路径参数 + `req.param("id")` | 已实现 | 可选参数通过 `{id?}`；约束通过 `where!()` |
| 路由名字 | 路由上的 `.name("posts.show")` + `url("posts.show", &[("id", "42")])` | 已实现 | [URL 生成](urls.md) |
| 路由分组 | 带 `.prefix()` / `.middleware()` / `.name()` / `.controller()` 的 `group!` 宏 | 已实现 | 分组中间件会在注册时被展平到每一条路由上 |
| 资源路由 | `resource!("posts", PostController)` 会注册那 7 条标准路由 | 已实现 | `apiResource!`、`only(...)`、`except(...)` 全都支持 |
| 签名 URL | `sign_url(...)`、`sign_route(...)`、`verify_signature(...)` | 已实现 | 用 `APP_KEY` 做 HMAC-SHA256 |
| 路由模型绑定 | `#[handler]` 通过 `RouteBinding` 实现，从 `{post}` 里提取出 `Post` | 已实现 | `AutoRouteBinding` 派生宏会为 `#[suprnova::model]` 类型自动实现它 |
| 速率限制 | `throttle:60,1` 中间件 + `RateLimiter::for_signature` | 已实现 | [速率限制](rate-limiting.md) |
| 中间件 | `impl Middleware` trait；可以全局注册，也可以逐路由注册 | 已实现 | [中间件](middleware.md) |
| 中间件分组 + 别名 | `register_middleware_group`、`register_middleware_alias` | 已实现 | 在路由里按字符串名字查找 |
| CSRF 保护 | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | 已实现 | Origin 策略会强制 POST 同源。[CSRF](csrf.md) |
| 控制器 | `#[handler] pub async fn show(req: Request) -> Response` | 已实现 | 控制器是由自由函数组成的模块，不是类。[控制器](controllers.md) |
| 单操作控制器 | 一个处理程序本来就是一个单独的函数；按模块归类即可 | 已实现 | 这是 Rust 的惯例 - 不需要 `__invoke` 那套仪式 |
| 请求 | 带 `.input()`、`.param()`、`.query()`、`.header()`、`.cookie()`、`.json()`、`.file()` 等方法的 `Request` 结构体 | 已实现 | [请求](requests.md) |
| 表单请求 | `#[derive(Data, Validate, FormRequest)]` | 已实现 | 验证会在您提取的同时运行 |
| 文件上传 | `req.file("avatar")?` 返回一个 `UploadedFile`；带大小上限和分段数上限的流式 multipart | 已实现 | 超过阈值会自动溢写到临时文件 |
| 响应 | `HttpResponse` 构建器 + `json!()` / `text!()` / `Redirect::to` / `view` | 已实现 | [响应](responses.md) |
| 视图（Blade） | 服务器端渲染的 Inertia 页面（Svelte/React/Vue） - 没有 Blade 的对应物 | 路径不同 | Inertia 就是视图层。请用[页面组件](frontend-pages.md)代替 Blade |
| 资产打包（Vite） | 每个脚手架都自带 Vite 8；`suprnova serve` 会把 Vite 和后端一起跑起来 | 已实现 | 清单读取 + HMR 自动接好 |
| 静态资产（`public/`，在 Laravel 里由 Web 服务器提供） | `StaticFiles::public()` 这个进程内的兜底处理程序，会在网站根路径上提供 `public/` | 已实现 | `StaticFiles::from_dir(...)` + `cache_control(...)`；不需要单独的 Web 服务器 |
| URL 生成 | `url("posts.show", &[…])`、`route("posts.show", …)`、`redirect(...)`、`redirect_to(...)` | 已实现 | [URL 生成](urls.md) |
| 会话 | `session()`、`session_mut()`，flash bag 通过 `req.flash()` | 已实现 | 通过 `DatabaseSessionDriver` 由数据库支撑；默认由 cookie 支撑。[会话](session.md) |
| Cookie 队列（`Cookie::queue`） | Cookie 会被附加到您返回的那个响应上（`HttpResponse::cookie`、`Redirect::cookie`） | 尚未实现 | 一个请求作用域的 cookie 罐、在出站响应上被排空，是计划中的；今天请把 cookie 交给您构建的那个响应 |
| 验证 | `#[derive(Validate)]` + 18 条内置规则 + `Rule`/`AsyncRule` trait | 已实现 | `Url` 使用 Laravel 的协议方案允许列表，`Url::protocols([...])` 对应 `url:http,https`。异步规则（比如 `Unique`）会打到数据库。[验证](validation.md) |
| `Password` 规则（`Password::defaults()`、`uncompromised()`） | 没有密码强度规则家族；请把 `Min`、`Regex` 和一条自定义 `Rule` 组合起来 | 尚未实现 | 它包含 Have I Been Pwned 的 `uncompromised()` 检查，今天还没有对应物 |
| 错误处理 | `FrameworkError`、`AppError`、`HttpError` trait，`execute_chain_safely` 里的 panic 边界 | 已实现 | [错误处理](errors.md)、[错误模型](error-model.md) |
| 日志 | 带结构化字段的 `tracing` 订阅者，`LogFormat`（json / pretty / compact） | 路径不同 | 一行日志就是一个 JSON 文档；`request_id` 始终存在。[日志](logging.md) |
| 日志通道 / 文件驱动程序（`single`、`daily`、`monthly`、`stack`） | `tracing` 把结构化的行写到 stdout；由平台去轮转和转运它们 | 刻意不做 | 容器、systemd 以及每一个日志转运工具都已经在做轮转和留存了。在进程内重新实现一遍，既重复了平台的工作，又把日志藏了起来。[日志](logging.md) |
| abort 辅助函数 | `abort_if(cond, status, msg)`、`abort_unless(...)`、`abort_with(status, msg)` | 已实现 | 与 Laravel 的 `abort_if` 家族形态相同 |

## 深入探索

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| Artisan 控制台 | 由 `#[command]` + `#[derive(Command)]` 构建的、逐应用的 `console` 二进制文件 | 已实现 | [控制台](console.md)。`cargo run --bin console <subcommand>` |
| Tinker（REPL） | 没有 REPL | 刻意不做 | 写一个一次性的 `cargo run --bin xxx` 脚本，或者一个 `#[suprnova_test]` |
| 广播 | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | 已实现 | 面向多节点的 sea-streamer 扇出。[广播](broadcasting.md) |
| 缓存 | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`、`RedisCache` | 已实现 | 原子操作 + 带标签的缓存 + 缓存锁（`LockGuard`）。[缓存](cache.md) |
| 集合 | 带 Laravel 形态方法的 `eloquent::Collection<M>` | 已实现 | `Deref<Target = Vec<M>>`，所以现有的 Vec 惯用法照样能用。[集合](eloquent-collections.md) |
| 并发 | 处处都是 Tokio - `tokio::spawn`、`tokio::join!`、`tokio::select!` | 已实现 | 整个框架都是异步的。Laravel 的 `Concurrency::run([...])` 门面没有实现；Tokio 就是答案 |
| 上下文 | `Context::put` / `Context::get` / `ContextStore` + 自动注入到队列 / 邮件 / 事件里 | 已实现 | [上下文](context.md) |
| 契约 | 所有公开接缝都是 trait | 已实现 | 参见上面“架构 / 契约”那一行 |
| 事件 | `EventFacade::dispatch(e).await?`、`#[derive(Event)]`、`EventDispatcher`、排队的监听器、订阅者 | 已实现 | [事件](events.md) |
| 文件存储 | 架在 OpenDAL 之上的 `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` | 已实现 | 同样的 `put/get/delete/copy/move/exists/url` 表面。内置路径穿越防护。[文件存储](filesystem.md) |
| 辅助函数 | 对应物都在各自的所属模块里（没有厨房水槽式的 `helpers.md`） | 路径不同 | 比如 URL 辅助函数在 [urls.md](urls.md) 里，字符串辅助函数在 `std`/`heck` 里，数组辅助函数在 `std::collections` 里 - Rust 是用 crate 而不是一个全局命名空间来做这件事的 |
| HTTP 客户端 | `Http::get/post/...` 构建器 + 供测试用的 `Http::fake(...)` | 已实现 | 自动记录请求；`assert_sent` / `assert_not_sent`。[HTTP 客户端](http-client.md) |
| 图像（`Illuminate\Image`） | 没有图像处理表面 | 尚未实现 | 一个架在 `image` crate 之上的 `ImageDriver` trait（缩放 / 裁剪 / 转换 / 主色调）已在计划中；在它实现之前请直接使用 `image` crate |
| 本地化 | `Lang::get` / `get_with` / `try_get` / `has`，以及架在 `lang/<locale>/` 里 Fluent `.ftl` 语料表之上的 `__!("key", name: value)` 宏、`LocaleMiddleware` 检测、翻译过的验证消息、ICU4X 格式化 | 已实现 | 同一份语料表会在 `/_suprnova/lang/<locale>.ftl` 提供给浏览器，并由 `generate-types` 赋予类型。[本地化](localization.md) |
| 邮件 | `Mail::to(...).send(MyMail { ... }).await?` + `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory` 这些驱动程序 | 已实现 | `Mailable` trait + 由 Tera 渲染的 HTML/文本正文。[邮件](mail.md) |
| 通知 | `Notify::send(&user, notif).await?` + `mail/database/broadcast/webpush` 这些通道 | 已实现 | `Notifiable` trait + 逐通道的 `Notification`。[通知](notifications.md)、[Web 推送](web-push.md) |
| 包开发 | workspace 里的适配器 crate（比如 `suprnova-payments-stripe`） | 已实现 | 与 Laravel 包形态相同：依赖框架、绑定进容器、需要时暴露宏 |
| 进程（运行 shell 命令） | 标准库里的 `tokio::process::Command` | 刻意不做 | 没有门面 - Tokio 的 API 形态本来就是对的 |
| 队列 | `Queue::push(job).await?` + `sync/memory/database/redis/null` 这些驱动程序、批次、链、`JobMiddleware`、`FailedJobStore` | 已实现 | [队列](queues.md) |
| 队列暂停（`queue:pause` / `queue:resume`） | 没有暂停开关；要停止消费就停掉工作进程 | 尚未实现 | 由缓存支撑的全局与逐队列暂停，外加 `QueuesPaused` / `QueuesResumed` 事件，已在计划中 |
| 提交后派发（`afterCommit()`） | 在一个事务内部推送的作业，对驱动程序立即可见 | 尚未实现 | 今天一次回滚会把作业留在队列里。在事务作用域的派发实现之前，请把推送放到事务外面 |
| 故障转移队列连接 | 没有 `failover` 驱动程序 | 尚未实现 | 在 `FailoverQueueDriver` 实现之前，请逐次推送时显式挑选连接，或者绑定您自己的、包住两个驱动程序的 `QueueDriver` |
| `ShouldBeUniqueUntilProcessing` | `Queue::push_unique` 会在整个作业期间持有这把锁 | 尚未实现 | 在认领时（而不是完成时）释放唯一性锁是另一套语义，目前还没有接上 |
| 队列检查（`pendingJobs` / `delayedJobs` / `reservedJobs`） | 没有驱动程序层面的检查 API | 尚未实现 | 在检查表面实现之前，请直接查询驱动程序背后的存储（`jobs` 表、Redis 键） |
| 逐任务时区的调度 | 计划是在一个进程级的时区里被求值的 | 尚未实现 | 逐任务的 `timezone(...)`，外加一个能感知时区的 `schedule:list`，已在计划中。[任务调度](scheduling.md) |
| 速率限制 | `RateLimiter::for_signature(...)`、`ThrottleRequestsMiddleware`、`RateLimitMiddleware` | 已实现 | 通过 `SlidingWindowConfig` 实现的滑动窗口。[速率限制](rate-limiting.md) |
| 搜索（Scout） | 没有官方的全文搜索适配器 | 尚未实现 | 向量搜索今天已经通过[向量搜索](vector.md)实现了；关键词版的 Scout 对应物已在计划中 |
| 字符串（辅助函数） | `heck` crate（大小写转换）、`std::str`、`regex` | 路径不同 | 和 Rust 生态其余部分用的是同一批 crate；没有 `Str::camel($x)` 这种全局函数 |
| 任务调度 | `Schedule::call/command/task` + `#[derive(Task)]` + cron 语法 + `schedule:run` 工作进程 | 已实现 | [任务调度](scheduling.md) |
| 幂等键 | `Idempotency::remember(key, ttl, body)` - Stripe 风格的重放防护 | 已实现 | 由调用方用路由 + 用户 / 业务身份给这个键加上命名空间。[幂等性](idempotency.md) |
| 请求超时 | 可以逐路由配置的 `TimeoutMiddleware` | 已实现 | Rust 原生的做法 - 中止飞行中的 future，把工作线程腾出来。[请求超时](timeout.md) |
| 功能标志（Pennant） | `Feature` + `Evaluator` + `FeatureMiddleware` + 管理端 CRUD | 已实现 | 通过 `FeatureSync` trait 实现的亚秒级传播。[功能标志](feature-flags.md) |
| 可观测性（Pulse） | 通过 `init_telemetry`、`Metrics` 以及处处都在的 `tracing` 实现的 OpenTelemetry | 路径不同 | OTel 是 Rust 可观测性的通用语 - 把您的收集器对准这个二进制文件就行。[可观测性](observability.md) |
| Telescope（调试面板） | 目前没有对应物 | 尚未实现 | 推迟到 v2+；框架的 tracing + OTel 输出已经覆盖了大部分诊断需求 |
| Pulse（性能面板） | 目前没有对应物 | 尚未实现 | 与 Telescope 相同 - 在面板上线之前，请用您现有的可观测性技术栈来呈现指标 |
| 向量搜索 | `Vector::driver("memory"\|"qdrant"\|"pinecone"\|"mariadb")` | 已实现 | 不搞“只支持 Postgres pgvector”那种把关。[向量搜索](vector.md) |

### Suprnova 独有（没有 Laravel 对应物）

| Suprnova | 它是什么 | 备注 / 链接 |
|---|---|---|
| `ws!()` 宏 + WebSocket 处理程序 | 与路由器 + 中间件栈共享的有类型 WS 路由 | [WebSocket](websockets.md) |
| Server-Sent 事件 | `SseEvent` + `HttpResponse::sse(...)` | [SSE](sse.md) |
| 工作流 | 带重试、睡眠和步骤边界的长时间运行有状态工作 | [工作流](workflows.md) |
| 监督程序 | 带 panic 捕获自动重启、面向长期存活 tokio 任务的 `Supervisor` trait | [监督程序](supervisors.md) |
| Web 推送（VAPID） | 作为一等公民通道的浏览器推送通知 | [Web 推送](web-push.md) |
| 多连接读写分离 | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [数据库](database.md) |
| 同一个套接字上的 HTTP/2 + WebSocket | `Server::run` 里的 `hyper.with_upgrades()` | [请求生命周期](lifecycle.md) |
| Markdown 内容 + 文档流水线 | `MarkdownRenderer`（净化过的 comrak → syntect → ammonia）+ `build_docs(DocsBuildConfig)` → 由 `DocsChapter` 组成的、可搜索的 `DocsCatalog` | 标题提取 + `slugify_heading`；不需要单独的静态站点生成器就能撑起 Markdown 文档 / 博客 |

## 安全

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| 认证 | `Auth::user/check/login/logout/attempt`、`Authenticatable` trait、按名字区分的 `Guard` | 已实现 | [认证](authentication.md) |
| 多个认证守卫 | 通过 `AuthManager` 按名字（`web`、`api`、……）注册的 `Guard` | 已实现 | `SessionGuard`、`TokenGuard`、自定义实现 |
| 用户提供者 | `EloquentUserProvider<U>`、`DatabaseUserProvider`，或通过 `UserProvider` trait 自定义 | 已实现 | [认证流程](auth-flows.md) |
| 邮箱验证 | `EmailVerification` + `EnsureEmailVerifiedMiddleware` + `EmailVerificationMail`；用户模型上的 `MustVerifyEmail` 契约 | 已实现 | 由提供者支撑（不经过 torii） - [认证流程](auth-flows.md) |
| 密码重置 | `PasswordReset` + `PasswordResetMail` + `PasswordChangedMail`；用户模型上的 `CanResetPassword` 契约 | 已实现 | 由提供者支撑（不经过 torii） - [认证流程](auth-flows.md) |
| 暴力破解节流 | `BruteForce` + `LoginThrottleMiddleware` | 已实现 | 按 IP + 按用户计数 |
| 双因素认证（TOTP） | `TwoFactor` + `TwoFactorChallengeMiddleware` + `TwoFactorUser` trait | 已实现 | 恢复码 + 重放防护 |
| 记住我 | 通过 `SessionGuard` 实现的长期存活签名 cookie | 已实现 | 框架自有的 `auth::remember`：数据库行 + bcrypt + 一次性轮换 |
| OAuth（Socialite） | 通过内嵌的 `torii_integration` fork（Google / GitHub / Apple 等） | 已实现 | [认证](authentication.md) |
| Sanctum（API 令牌） | `TokenGuard` + 通过 torii 实现的数据库存储令牌 | 路径不同 | 令牌模型 + bearer 中间件已实现；没有单独的 Sanctum API 表面 |
| Passport（OAuth 服务端） | 尚无 | 尚未实现 | 如果需要一个 OAuth 提供者，请在 Suprnova 背后跑一个专门的身份服务（Keycloak、Hydra） |
| Fortify（认证后端） | 由 `auth_flows` 模块 + `auth_flows::*` 类型取代 | 已实现 | 同样的工作；不需要区分无头/有头，因为前端就是 Inertia |
| 授权（Policies / Gates） | `Gate::allows/denies` + `#[policy] impl PostPolicy` + `Authorizable` trait + 宏注册 | 已实现 | [授权](authorization.md) |
| 角色与权限（spatie/laravel-permission） | `HasRoles` trait + `roles` / `permissions` / `role_has_permissions` 表（`CreateRbacTables`）+ `RoleMiddleware` / `PermissionMiddleware`（失败即关闭） | 已实现 | 官方自带，不是社区包。`create_role` / `give_permission_to_role` / `assign_role_to_model` 这些辅助函数；叠加在 Gate/Policy 之上。[授权](authorization.md) |
| 加密 | `Crypt::encrypt/decrypt` + `CryptPurpose` 的 AAD 绑定 | 已实现 | AES-256-GCM，通过 `APP_KEY_PREVIOUS` 实现密钥轮换。[加密](encryption.md) |
| 哈希 | `hash::*` + `BcryptHasher`、`Argon2idHasher`、`Argon2iHasher`、`needs_rehash`、`is_hashed`、`verify` | 已实现 | 默认 Bcrypt；argon2id 可选。[哈希](hashing.md) |

## 数据库

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| DB::table('users')->where(...)->get() | `DB::table("users").db_where("id", "=", 1).get().await?` | 已实现 | [数据库](database.md)、[查询构造器](queries.md) |
| 多连接 | `DB::on("read")` + `ConnectionRegistry` | 已实现 | 读写分离是一等公民 |
| 事务 | `DB::transaction(\|tx\| async move { ... }).await?` | 已实现 | 保存点 + 死锁重试 |
| 查询事件 | `QueryListener` + `QueryExecuted` 事件 | 已实现 | `DB::listen(\|q\| { ... })` |
| 原生表达式 | `DB::raw("...")`、`DB::select("...", &[...])` | 已实现 | 必须走参数绑定（不支持字符串插值） |
| Postgres / MySQL / SQLite | 三者都通过 SeaORM 一等支持 | 已实现 | URL 检测在 `database::config::database_type()` 里 |
| MariaDB | 作为独立的一等选项（向量 + JSON + 时态） | 路径不同 | 因为存在 Laravel 只在 Postgres 上才提供的多范式特性，所以单独处理 |
| Redis | 由驱动使用（cache/queue/rate-limit） - 没有单独的 `Redis::*` 门面 | 路径不同 | 需要临时命令时直接拿 `redis` crate；cache/queue/rate-limit 已经覆盖了 95% 的常见用法 |
| MongoDB | 目前没有官方适配器 | 尚未实现 | 通过 `App::bind` 直接使用 `mongodb` crate |
| 查询构造器 | 带 `db_where` / `or_where` / `where_in` / `where_between` / `where_null` / `where_has` / `with` / `with_count` / `order_by` / `group_by` / `having` / `paginate` 等方法的 `Builder<M>` | 已实现 | [查询构造器](queries.md) |
| 分页 | `LengthAwarePaginator`、`Paginator`（简单分页）、`CursorPaginator` | 已实现 | 三者都会序列化成 Laravel 形态的 JSON。[分页](pagination.md) |
| 迁移 | `#[derive(DeriveMigrationName)] struct M;` + `up`/`down` + `Migrator` | 已实现 | 通过 `suprnova migrate`/`migrate:rollback`/`migrate:status`/`migrate:fresh` 运行。[迁移](migrations.md)、[CLI 迁移](cli-migrations.md) |
| 填充器 | `Seeder` trait + `db:seed` 子命令 | 已实现 | 逐模型工厂。[数据填充](seeding.md) |

## Eloquent ORM

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| `class User extends Model` | `#[suprnova::model(table = "users")] struct User { ... }` | 已实现 | 这个结构体本身就是 SeaORM 的 `Model`。[Eloquent](eloquent.md) |
| Find / first / get | `User::find(id)`、`User::query().first()`、`User::all()`、`Builder::get` | 已实现 | 全部都是异步的 |
| Create / update / delete | `User::create(attrs)`、`user.update(attrs)`、`user.delete()` | 已实现 | 用于部分属性的 `attrs! { name: "...", email: "..." }` 宏 |
| 批量赋值防护 | `#[model(fillable = [...])]` / `#[model(guarded = [...])]` + `unguarded \|\| { ... }` 作用域 | 已实现 | 严格模式用 `prevent_silently_discarding_attributes()` |
| 软删除 | `#[model(soft_deletes)]` 自动注入 `deleted_at` + `SoftDeletes` trait | 已实现 | `with_trashed()`、`only_trashed()`、`restore()`、`force_delete()` |
| `Prunable` / `MassPrunable` | `#[prunable] impl Prunable for User { ... }` + `model:prune` 工作进程 | 已实现 | 级联锁定到关系上 |
| 时间戳 | 有对应列时自动填充 `created_at`/`updated_at` | 已实现 | 通过 `#[model(timestamps = false)]` 禁用 |
| 主键类型 | 默认 i64；通过 `#[model(unique_id = "uuid")]` 或 `unique_id = "ulid"` 使用 UUID / ULID | 已实现 | 插入时自动生成 id |
| 本地作用域 | `#[scopes(User)] impl User { fn active(b: &mut Builder<User>) { ... } }` | 已实现 | 在 `Builder<M>` 上做方法分发 |
| 全局作用域 | `impl GlobalScope for ActiveOnly { ... }` + 注册 | 已实现 | 通过 `Builder::without_global_scope` 剥离 |
| 关系（11 种） | `HasOne`、`HasMany`、`BelongsTo`、`BelongsToMany`、`HasOneThrough`、`HasManyThrough`、`MorphOne`、`MorphMany`、`MorphTo`、`MorphToMany`、`MorphedByMany` | 已实现 | 按族分组的 morph 枚举。[关系](eloquent-relationships.md) |
| 预加载 | `User::query().with(&["posts", "posts.comments"]).get()` | 已实现 | `EagerLoadDispatch` 是密封的；只有宏生成的关系才能实现它 |
| 延迟加载预防 | `prevent_silently_discarding_attributes(true)` | 已实现 | 与 Laravel 的 `preventLazyLoading` 形态相同 |
| 关系上的聚合 | `with_count("posts")`、`with_sum("orders", "total")`、`with_avg`、`with_min`、`with_max` | 已实现 | 每个聚合一条子查询 |
| `whereHas` / `whereDoesntHave` | `where_has("posts", \|q\| q.db_where("published", "=", true))` | 已实现 | 关联 EXISTS 引擎 |
| `loadMissing` | `user.load_missing(&["posts"]).await?` | 已实现 | 作用于整个集合 |
| 克隆一条记录 | `user.replicate()` / `user.replicate_into::<OtherType>()` | 已实现 | 会派发 `Replicating` 事件 |
| 触碰父级时间戳 | `#[model(touches = ["post"])]` | 已实现 | 用 `without_touching \|\| { ... }` 跳过 |
| 观察者 | `impl Observer<User>` + `#[suprnova::observer(User)]` | 已实现 | 16 个生命周期事件 |
| 16 个生命周期事件 | `Created`、`Creating`、`Saving`、`Saved`、`Updating`、`Updated`、`Deleting`、`Deleted`、`Trashed`、`Restoring`、`Restored`、`Retrieved`、`Replicating`、`ForceDeleting`、`ForceDeleted`、`Pruning` | 已实现 | 逐模型的 `events::*` 子模块。`EventResult::cancel(_)` 会以 400 短路 |
| 修改器 / 访问器 | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, v: String)` | 已实现 | [修改器](eloquent-mutators.md) |
| 转换（22 种内置） | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | 已实现 | 自定义的话实现 `Cast` |
| 集合 | 带 `pluck`、`filter`、`map`、`each`、`chunk`、`groupBy`、`keyBy`、`sort_by`、`where_`、`first`、`last`、`count`、`is_empty`、`to_array` 及其 Laravel 同伴的 `Collection<M>`；`Deref<Target = Vec<M>>`，所以所有 `Vec` 惯用法照样能用 | 已实现 | [集合](eloquent-collections.md) |
| API 资源 | `#[derive(Resource)]` + `IntoJsonResource` + `JsonApiResponse` + 字段集 + include | 已实现 | JSON:API 形态和 Laravel 风格的资源形态两者皆可用。[API 资源](eloquent-resources.md) |
| 序列化 | `#[model(hidden = [...], visible = [...], appends = [...])]` | 已实现 | 对哪些属性会被序列化拥有同样的控制力。[序列化](eloquent-serialization.md) |
| 工厂 | `#[derive(Factory)] struct UserFactory` + `UserFactory::new().count(5).create().await?`（或 `UserFactory::times(5).create_many().await?`） | 已实现 | 用于循环取值的 `Sequence`。[工厂](eloquent-factories.md) |
| 生命周期：分块 / 惰性 / 游标 | `Builder::chunk(n, \|page\| async { ... })`、`lazy()`、`cursor()` | 已实现 | 对大表做内存受限的迭代 |
| 悲观锁 | `Builder::lock_for_update()`、`shared_lock()` | 已实现 | 在一个事务内部 |
| `whereJsonContains` 家族 | 通过 SeaORM 的列表达式提供（因后端而异） | 已实现 | 具体写法因后端而不同；常见场景已经提供了辅助函数 |

## 分页

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| `LengthAwarePaginator` | `LengthAwarePaginator`（page + total + per_page + last_page） | 已实现 | `Builder::paginate(n).await?` |
| `Paginator`（simple） | `Paginator`（page + per_page + has_more，不计总数） | 已实现 | `Builder::simple_paginate(n).await?` |
| `CursorPaginator` | `CursorPaginator`（不透明的游标令牌 + 方向） | 已实现 | `Builder::cursor_paginate(n).await?`；对无限滚动是确定性的 |
| Inertia 集成 | `IntoInertiaScroll` trait + `ScrollMetadata` | 已实现 | 直接接入 Inertia 的 `WhenVisible` / `merge` |

## AI（Laravel 如今原生自带；我们不把关）

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| AI SDK | 没有官方 AI SDK | 刻意不做 | 带上您本来就在用的 crate（`async-openai`、`anthropic-sdk`、`ollama-rs`、`tokenizers` 等），绑定到 `App` 下面 |
| MCP（Model Context Protocol） | 没有官方 MCP 服务端适配器 | 刻意不做 | Rust 的 MCP crate（`mcp-rs`、`mcp-sdk-rust`）可以干净地架在现有的路由 / 监督程序表面之下 |
| Boost（Laravel 的编码 agent） | 不适用 | 刻意不做 | 超出框架范围 |

## 测试

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| `php artisan test` | `cargo test` | 已实现 | [测试](testing.md) |
| Pest / PHPUnit 风格 | `#[suprnova_test]`（能感知异步）+ `expect!()` 这种 Jest 风格的断言 + `describe!()` / `test!()` 这两个 BDD 宏 | 已实现 | 三者可以互换使用 |
| 功能测试（HTTP） | 在进程内驱动 `handle_request(router, registry, req)` - 不打开任何套接字 | 已实现 | [HTTP 测试](http-tests.md) |
| `TestResponse` 包装器 | 直接对 `HttpResponse` 做断言（`status_code()`、`body()`、`header_value()`） | 尚未实现 | 一个链式的 `assert_status` / `assert_json_path` / `assert_cookie` 包装器已在计划中；今天测试会把响应解码一次，然后对取出的值做断言 |
| 控制台测试 | 运行 `dispatch_argv(["console", "..."])` 然后做断言 | 已实现 | 对 console 二进制文件来说，形态与 HTTP 测试相同 |
| 浏览器测试（Dusk） | 框架里不适用 - 请用 Playwright / WebdriverIO / `gstack` agent 浏览器 | 刻意不做 | 跨语言的工具已经存在；我们不重新发明它 |
| 数据库测试 | `TestDatabase::fresh::<Migrator>()` + 逐测试回滚 | 已实现 | [数据库测试](database-testing.md) |
| 模拟与伪造 | 逐门面的伪造实现：`MailFake`、`NotifyFakeGuard`、`EventFakeGuard`、`Queue::fake`、`Bus::fake`、`Http::fake`、`Storage::fake` | 已实现 | 记录下来的调用 + 断言辅助函数。[模拟和伪造](mocking.md) |
| 时间旅行 | 标准库运行时里的 `tokio::time::{pause, advance, resume}` | 已实现 | 不提供我们自己的那一套 - Tokio 的 API 已经做到了 |
| 容器隔离 | `TestContainer::fake(\|tc\| tc.bind(...))` - 线程本地 | 路径不同 | 从构造上就对并行安全。[服务容器](container.md) |

## 支付（Laravel 的 Cashier；我们这边是与提供商无关的通用方案）

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| Cashier（Stripe） | `suprnova-payments-stripe` 适配器 crate，架在通用的 `Payment` / `Subscription` / `CustomerStore` / `WebhookHandler` trait 之后 | 路径不同 | 通用表面，具体适配器。[支付](payments.md)、[Stripe 适配器](payments-stripe.md) |
| Cashier（Paddle） | `suprnova-payments-paddle` 适配器 | 路径不同 | 记录商户（Merchant of Record，MoR）流程 + 没有直接的 `Payment` 实现（网关由 Paddle 掌控）。[Paddle 适配器](payments-paddle.md) |
| 自定义提供商 | 实现 `PaymentProvider` + `SessionPayload` + `WebhookHandler` | 已实现 | [提供商指南](payments-provider-guide.md) |
| Inertia 结账组件 | 针对 `SessionPayload.flow`、已写好文档的 Svelte / React / Vue 分发循环 | 已实现 | [支付前端](payments-frontend.md)。现成的账单页面是一项计划中的起步套件加项（[起步套件](starter-kits.md)） |
| 订阅生命周期 | `Subscription::subscribe / update / cancel / get`（在提供商支持的范围内） | 已实现 | 提供商不支持的地方会返回 `NotSupported`（比如 Paddle 的 `subscribe` 和价格集替换） |
| Webhook 幂等性 | 带 `UNIQUE(provider, provider_event_id)` 的 `payments_webhook_events` 镜像表 | 已实现 | Stripe 风格的重放防护 |
| 镜像表 | `payments_customers`、`payments_payment_methods`、`payments_subscriptions`、`payments_subscription_items`、`payments_transactions`、`payments_webhook_events` | 已实现 | 每张表上都有一个 `provider_metadata` JSONB 列，用于适配器专属字段 |

## 前端（Laravel 有 Blade + 起步套件；我们有 Inertia）

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| Blade | 不适用 - Inertia 就是视图层 | 路径不同 | [前端](frontend.md) |
| Inertia.js | 一等公民：架在 Svelte 5 / React 19 / Vue 3.5 之上的 v3 | 已实现 | [Inertia 响应](frontend-inertia-responses.md)、[页面组件](frontend-pages.md) |
| 页面 URL 解析（`Inertia::resolveUrlUsing`） | `page.url` 是路径 + 查询；用 `InertiaConfig::url_resolver` 覆盖 | 已实现 | 默认的推导方式与版本中间件的 `X-Inertia-Location` 逐字节一致；一个 `url_resolver` 只会改变 `page.url` |
| Inertia 协议中间件（`Vary`、空响应、版本弹回） | `InertiaHeadersMiddleware` + `InertiaVersionMiddleware` + `Inertia303Middleware`，全都由 `Inertia::install` 接好 | 已实现 | 每一个响应上都带 `Vary: X-Inertia`；一次 Inertia 访问上的空 `200` 会变成一个 `303` 回跳；那次 409 弹回会重新 flash 会话 |
| 外部重定向 + 清除历史记录 | `InertiaResponse::location_for(&req, url)`、`App::clear_history()` | 已实现 | `location_for` 对 XHR 是 `409`，对一次硬性导航是 `302`；`App::clear_history()` 能挺过登出重定向 |
| 部分重新加载 | `#[derive(Data)]` + `req.includes("subset")` + Inertia 的部分重新加载协议 | 已实现 | 类型安全的 include 集合 |
| 延迟 prop | `Prop::deferred(...)` + `DeferConfig` | 已实现 | Inertia v3 的 deferred-props 协议 |
| 合并 prop | `MergeConfig` + `MergeStrategy::{Append, Prepend, Replace}` | 已实现 | Inertia v3 的合并协议 |
| 加密历史记录 | `EncryptHistoryMiddleware` | 已实现 | 历史记录在客户端静态加密 |
| 滚动位置 | `ScrollConfig` + `ScrollMetadata` | 已实现 | 导航时自动恢复 |
| TypeScript 类型 | `suprnova generate-types` 会读取 `#[derive(InertiaProps)]` 并产出 `.d.ts` | 已实现 | [TypeScript 类型](frontend-typescript-types.md) |
| 读取 Vite 清单 | 通过 `InertiaConfig::manifest_path` 自动接好 | 已实现 | 开发时是 HMR，生产时是带哈希的资产。清单缺失时 `Inertia::install` 会在生产环境失败即关闭 |
| Inertia SSR（`inertia:start-ssr`） | 传给 `Inertia::install` 的那份配置上的 `InertiaConfig::ssr(...)`，工作进程由 `suprnova ssr:start` 启动 | 已实现 | 通过 HTTP 环回连接的进程外工作进程；除非设置了 `ssr_throw_on_error(true)`，否则出错或超时会回退到 CSR。[Inertia 响应](frontend-inertia-responses.md) |

## CLI

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| `php artisan` | 由 `#[command]` 宏构建、逐应用的 `console` 二进制文件 | 已实现 | [控制台](console.md)、[CLI 概览](cli.md) |
| `make:controller` / `make:model` 等 | `suprnova make:controller / make:middleware / make:action / make:error / make:inertia / make:migration / make:task` | 已实现 | [生成器](cli-generators.md) |
| `serve` | `suprnova serve`（后端 + Vite 开发服务器一起跑） | 已实现 | [Serve](cli-serve.md) |
| `migrate` 家族 | `suprnova migrate / migrate:rollback / migrate:status / migrate:fresh` | 已实现 | [CLI 迁移](cli-migrations.md) |
| `db:seed` | `cargo run --bin console db:seed`（经由逐应用的 console） | 已实现 | 填充器通过 `Seeder` trait 注册 |
| `schedule:run` / `schedule:work` / `schedule:list` | 经由逐应用的 console 二进制文件，名字相同 | 已实现 | [调度命令](cli-scheduling.md) |
| `queue:work` | 经由逐应用的 console 二进制文件，名字相同 | 已实现 | 在 SIGTERM/SIGINT 上优雅关闭 |
| `tinker` | 没有 REPL | 刻意不做 | 参见“深入探索”里的那一行 |

## 部署

| Laravel | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| `php artisan optimize` | `cargo build --release` | 路径不同 | 一份二进制文件，没有 opcache 这一步 |
| `php artisan config:cache` | 有类型的配置本来就是编译期检查的 | 路径不同 | 没有需要失效的运行时缓存 |
| `php artisan route:cache` | 路由在编译期就被宏展开了 | 路径不同 | 路由器是在启动时，由已经有类型的路由构建出来的 |
| Envoy（SSH 部署） | 用任何编排工具都行 - Docker、systemd、Kubernetes、fly.io、Railway | 刻意不做 | 二进制文件就是部署产物 |
| Forge / Vapor | 不是我们该提供的东西 - 但 Railway、DO 和 Hetzner 的方案覆盖了同样的工作 | 路径不同 | [部署](deployment.md)、[Railway](deployment-railway.md)、[Digital Ocean](deployment-digital-ocean.md)、[Hetzner](deployment-hetzner.md) |
| 维护模式（`php artisan down` / `up`） | `./app down` / `./app up` - 绕过密钥、自定义的 retry/message/except 路径、`file` 或 `cache` 驱动程序 | 已实现 | [部署](deployment.md) |
| Horizon（队列面板） | 目前还没有面板 | 尚未实现 | 在那之前，失败作业的检查通过 `cargo run --bin console queue:failed` 进行 |

## 包（Laravel 的官方包 - 我们这边要么内置在核心里，要么以适配器形式提供，要么是刻意留下的空白）

| Laravel 包 | Suprnova | 状态 | 备注 / 链接 |
|---|---|---|---|
| Cashier（Stripe） | `suprnova-payments-stripe` | 已实现 | 通用 + 适配器。[支付](payments.md) |
| Cashier（Paddle） | `suprnova-payments-paddle` | 已实现 | MoR 流程。[支付](payments.md) |
| Dusk | 不适用 | 刻意不做 | 跨语言的浏览器工具已经存在（Playwright 等） |
| Envoy | 不适用 | 刻意不做 | 容器 / systemd / 编排工具就能做到 |
| Fortify | 由 `auth_flows` 取代 | 已实现 | 同样的工作，已经集成好。[认证流程](auth-flows.md) |
| Folio | 不适用 - 基于页面的路由不是 Rust 的惯用做法 | 刻意不做 | 用 `routes!` 做显式路由 |
| Homestead | 不适用 - 用 Docker / DevContainers | 刻意不做 | [Docker 方案](cli-docker.md) |
| Horizon | 目前不适用 | 尚未实现 | 失败作业通过逐应用的 console 呈现 |
| Mix | 由 Vite 取代 | 路径不同 | 每个脚手架都自带 Vite |
| Octane | 不适用 - 我们本来就是长期存活的 Tokio | 刻意不做 | 单一二进制文件，始终热着，没有 FPM 需要换出 |
| Passport | 目前不适用 | 尚未实现 | 在它实现之前，请在 Suprnova 背后跑一个专门的身份提供者 |
| Pennant（功能标志） | 以 `features::*` 重新实现 | 已实现 | [功能标志](feature-flags.md) |
| Pint（PHP 代码风格） | `cargo fmt` + `cargo clippy` | 路径不同 | 标准 Rust 工具链 |
| Precognition | 通过部分重新加载 + 同一套 `#[derive(Data, Validate, FormRequest)]` 类型实现的 Inertia 预知请求 | 已实现 | Precog 的两半（提前验证 + 轻量重新加载）都是 Inertia v3 + 表单请求自然而然带出来的 |
| Prompts（CLI UI） | 需要时用 `dialoguer` / `inquire` crate | 刻意不做 | Rust 生态已经覆盖了这个需求 |
| Pulse | 目前不适用 | 尚未实现 | 现在是 OTel，面板以后再说 |
| Reverb（WebSocket 服务端） | 内置在 Suprnova 里（`ws!()` + `BroadcastHub`） | 路径不同 | 不需要单独的服务端 - 就是同一个进程 |
| Sail（Docker 开发环境） | `suprnova-cli` 内置了 Docker 方案 | 已实现 | [CLI Docker](cli-docker.md) |
| Sanctum | `TokenGuard` + bearer 中间件 | 路径不同 | 令牌模型已实现；没有单独的包表面 |
| Scout（全文搜索） | 目前不适用 | 尚未实现 | 向量搜索已实现（[向量](vector.md)）；关键词版的 Scout 对应物以后再说 |
| Socialite | 通过内嵌的 torii fork | 已实现 | [认证](authentication.md) |
| Telescope | 目前不适用 | 尚未实现 | 在面板上线之前，由 Tracing + OTel 覆盖诊断这块空白 |
| Valet | 不适用 - Rust 应用直接运行 | 刻意不做 | `suprnova serve` 就是开发运行器 |

## 宏（Rust 特有的表面；上下文中给出最接近的 Laravel 类比）

Suprnova 提供了一大批 Laravel 没有对应物的 proc-macro，因为 Laravel 没有宏 - 它有的是运行时反射。把它们列在这里，好让您不会漏掉。

| 宏 | 最接近的 Laravel 概念 | 它做了什么 |
|---|---|---|
| `#[suprnova::model]` | `extends Model` | 生成 SeaORM 实体 + 实现 `Model` trait |
| `#[suprnova::observer(M)]` | `User::observe(UserObserver::class)` | 通过 `inventory` 注册一个 `Observer<M>` 实现 |
| `#[scopes(M)]` | 模型上的本地作用域 | 给 `Builder<M>` 添加方法 |
| `#[accessor]` / `#[mutator]` | Eloquent 的访问器 / 修改器 | 字段级的取值/赋值钩子 |
| `#[handler]` | 控制器的 `__invoke` | 从 `Request` 里自动提取有类型的参数 |
| `#[command]` / `#[derive(Command)]` | Artisan 命令类 | 注册一个 console 子命令 |
| `#[policy]` | Policy 类 | 通过 `inventory` 注册一个 `Policy` 实现 |
| `#[service(T)]` | 服务提供者的 `register` | 把 `T` 绑定进容器 |
| `#[injectable]` | 构造函数注入 | 生成一个由 `App::make` 支撑的构造函数 |
| `#[derive(InertiaProps)]` | Inertia props | TypeScript 代码生成 + Inertia 序列化 |
| `#[derive(Data)]` | 请求 DTO | 可以带着 include 集合支持从 `Request` 里提取 |
| `#[derive(FormRequest)]` | `FormRequest` 类 | 验证 + 认证门 + 转换 |
| `#[derive(Factory)]` | 模型工厂 | 由 Faker 支撑的测试数据生成 |
| `#[derive(Resource)]` | API 资源 | JSON:API + Laravel 形态的序列化 |
| `#[workflow]` / `#[workflow_step]` | Laravel 里不适用 | 长时间运行的有状态工作 |
| `routes!` + `get!` / `post!` / `ws!` 等 | `Route::get` / `Route::post` | 编译期路由注册 |
| `casts!` | `protected $casts = [...]` | 逐模型的 cast 声明 |
| `attrs!` | 批量赋值数组 | 部分属性构建器 |
| `json_response!` / `text_response!` | `response()->json(...)` | 快速写出 `Ok(HttpResponse::...)` |

完整参考请参见[宏](macros.md)。

## 辅助函数（Laravel 的全局辅助函数；我们的是有类型的）

Laravel 提供了数百个小型全局函数（`str_replace_first`、`array_flatten`、`now()`、`tap()`、`optional()` ……）。它们中的大多数在 `std` 或某个小型标准 crate 里都有直接的 Rust 对应物，所以 Suprnova 不会把它们重新塞进一个单一命名空间。那些*确实*值得起个别名的，会在各自的所属模块下提供。

| Laravel 辅助函数 | Suprnova / Rust 对应物 | 位置 |
|---|---|---|
| `auth()` | `Auth::user().await?` | [认证](authentication.md) |
| `cache()` | `Cache::get/put/...` | [缓存](cache.md) |
| `config('app.name')` | `Config::get::<AppConfig>()?.name` | [配置](configuration.md) |
| `csrf_token()` | `csrf_token()`（同名） | [CSRF](csrf.md) |
| `dd()` | `Builder::dd()`（Eloquent 查询的 dump-and-die）/ 标准库的 `dbg!()` | `Builder::dump()` / `Builder::dd()` 用于查询检查；一般的值用 `dbg!()` |
| `env('APP_KEY')` | `env("APP_KEY")` / `env_required("APP_KEY")` / `env_optional("APP_KEY")` | [配置](configuration.md)、[环境变量](env-vars.md) |
| `now()` | `chrono::Utc::now()`（以 `suprnova::chrono` 重新导出） | - |
| `optional($x)->y` | `x.as_ref().map(\|x\| x.y)` | Rust 直接用 `Option<T>` 处理这件事 |
| `redirect('/')` | `redirect("/")`（同名） | [路由](routing.md) |
| `request()` | `Request` 会被传入您的处理程序 | [请求](requests.md) |
| `response()` | `HttpResponse::json/text/redirect/...` | [响应](responses.md) |
| `route('posts.show', ['post' => 1])` | `url("posts.show", &[("post", "1")])` | [URL 生成](urls.md) |
| `session('key')` | `session().get("key")` | [会话](session.md) |
| `str()` / `Str::camel($x)` | `heck` crate 的方法（`ToUpperCamelCase` 等） | - |
| `tap($x, fn) → $x` | `tap` crate 里的 `tap`，或者用 `dbg!` 快速检查 | 按惯用方式使用 `tap` crate |
| `today()` | `chrono::Utc::now().date_naive()` | - |
| `value($x)` | 直接调用这个闭包：`x()` | 不适用 - Rust 闭包不需要辅助函数 |
| `view('home', $data)` | Inertia 响应：`Inertia::render("Home", data)` | [Inertia 响应](frontend-inertia-responses.md) |

## 我们确实还没有的东西

把上面每一个**尚未实现**汇总成一份清单，好让您在一个地方就能看清这块空白的形状：

| 领域 | 缺了什么 | 实现之前的变通办法 |
|---|---|---|
| 搜索（Scout - 关键词） | Algolia / Meilisearch / Elastic 适配器 | 在它实现之前，请用 `meilisearch-sdk` / `elasticsearch` 自己搭一套；语义搜索今天由[向量搜索](vector.md)处理 |
| Passport（OAuth 服务端） | 官方的 OAuth 身份提供者 | 在 Suprnova 背后跑一个 Hydra / Keycloak |
| Telescope（调试面板） | 呈现请求 / 查询 / 事件 / 缓存命中的 Web 界面 | 使用 OTel + tracing 的输出（[可观测性](observability.md)） |
| Pulse（性能面板） | 呈现慢查询 / 错误 / 热点路由的 Web 界面 | 同上：今天是 OTel 表面，面板以后再说 |
| Horizon（队列面板） | 呈现队列深度 / 失败作业 / 吞吐量的 Web 界面 | `cargo run --bin console queue:failed` 加上 OTel 指标 |
| 图像处理 | `Illuminate\Image` 的对应物（缩放 / 裁剪 / 转换） | 在您自己的 `App::bind` 背后直接使用 `image` crate |
| Cookie 队列 | `Cookie::queue` 这个请求作用域的罐子 | 把 cookie 附加到您返回的那个响应上 |
| `Password` 验证规则 | 强度规则 + `uncompromised()` 的 HIBP 检查 | 把 `Min` + `Regex` + 一条自定义 `Rule` 组合起来 |
| 队列暂停 | `queue:pause` / `queue:resume`，全局 + 逐队列 | 停掉这个工作进程 |
| 提交后派发 | 事务作用域的作业派发 | 在事务返回之后再推送 |
| 故障转移队列连接 | 架在一个有序驱动程序列表之上的 `failover` 驱动程序 | 逐次推送时挑选连接 |
| `ShouldBeUniqueUntilProcessing` | 在认领时就释放的锁 | `push_unique` 会在整个作业期间持有这把锁 |
| 队列检查 | `pendingJobs` / `delayedJobs` / `reservedJobs` | 查询驱动程序背后的存储 |
| 逐任务时区的调度 | 逐个计划任务的 `timezone(...)` | 每个时区跑一个调度器进程 |
| `TestResponse` 包装器 | 链式的 HTTP 断言 | 直接对 `HttpResponse` 做断言 |

## 我们不会做的功能（以及原因）

| Laravel 功能 | 为什么 Suprnova 没有它 |
|---|---|
| Tinker（REPL） | 对编译型二进制文件来说，Rust 没有一套行之有效的 REPL 方案。一个简短的 `#[suprnova_test]`，或者一个一次性的 `cargo run --bin <thing>` 脚本就能完成这件事 |
| Blade 模板 | Inertia 就是视图层；我们不会再提供一套并行的服务器端渲染模板引擎 |
| `helpers.md` 厨房水槽式集合 | Rust 提供 `std` + 一些小型专用 crate（`heck`、`chrono`、`regex`）；我们不会把它们重新塞进一个单一全局命名空间 |
| Mix | Vite 覆盖了它的功能，并且每个脚手架都自带 |
| Octane | Suprnova 本来就是长期存活的 Tokio；没有 FPM 模式需要优化掉 |
| Dusk（浏览器测试） | 跨语言工具（Playwright、WebdriverIO、`gstack` agent 浏览器）已经解决了这个问题 |
| Sail（Docker 开发环境） | Docker 方案已经内置（[CLI Docker](cli-docker.md)）；不需要单独的包 |
| Valet | `suprnova serve` 就是开发服务器 |
| Envoy（SSH 部署） | 容器 / systemd / 编排工具就能做到；我们不需要一套定制的 SSH DSL |
| Concurrency 门面（`Concurrency::run`） | Tokio（`tokio::join!` / `tokio::spawn` / `tokio::select!`）就是答案；不需要门面 |
| Processes 门面 | `tokio::process::Command` 本身的形态就已经对了 |
| 官方 AI SDK / MCP / Boost | 选您本来就在用的 Rust crate；我们不把关 |
| 专门的 Redis 门面 | Cache/queue/rate-limit 已经覆盖了 95% 的常见用法；需要临时命令时直接拿 `redis` crate |
| Strings 门面 | `heck`、`regex`、`std::str` 已经覆盖了这个需求；没有 `Str::camel($x)` 这种全局函数 |
| Prompts（CLI UI 库） | `dialoguer` / `inquire` 已经存在；我们不重新发明 |
| Laravel 风格的 PHP/JSON 翻译文件 | 本地化已经实现，但目录格式是 Fluent `.ftl` - 服务器和浏览器都能解析的同一种格式。`trans_choice` 也没有对应物：Fluent 会在消息内部挑选 CLDR 复数类别。[本地化](localization.md) |

## 这份清单如何保持诚实

每一行标着**已实现**的，都可以通过下面的方式核实：

1. 在 `framework/src/lib.rs` 里 grep 那个具名导出
2. 运行框架的测试套件（`cargo test --workspace`）
3. 阅读链接过去的那一章

每一行标着**尚未实现**的，都是有意要做的工作，而不是一次拒绝。每一行标着**刻意不做**的，都在备注列里有一句话的理由；那些理由，就是[简介](introduction.md)里的设计原则被应用到某个具体功能上的结果。

最近一次是对照 Laravel 13.25.0 评审的。

如果您发现某个自己会去用的 Laravel 功能不在这张地图上，请提一个 issue - 它要么是有一个 Suprnova 的答案却缺了一行，要么就是一处真实的空白，而我们想知道。

## 下一步

- [从 Laravel 来](from-laravel.md) - 同一份地图，以并排叙述的方式呈现
- [简介](introduction.md) - 这份对等工作所遵循的设计原则
- [`documentation.md`](documentation.md) - 横跨每一章的主目录
