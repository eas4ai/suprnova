# 简介

Suprnova 是一个 Rust Web 框架，为您提供 Laravel 的开发者体验，构建在 Tokio 之上。您编写控制器和 Eloquent 风格的模型，框架为您提供并发、类型安全和单二进制部署。

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0");
    json_response!({ "id": id, "name": "Alice" })
}
```

```rust
use suprnova::{model, Model};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// 之后，在任意位置：
let user = User::find(42).await?;
let admins = User::query().db_where("role", "admin").get().await?;
let alice = User::create(attrs!{ name: "Alice", email: "alice@x.com" }).await?;
```

如果您上周还在用 Laravel 编写这样的代码，上面的 Rust 版本看起来会完全相同 - 相同的链式调用形式、相同的方法名称、相同的默认值。不同之处在于底层的运作方式：使用 Tokio 而不是 FPM，一个二进制文件而不是 PHP 运行时，每列都进行编译时类型检查。

## 为什么需要 Suprnova

Laravel 解决了后端 Web 开发的生产力问题。这些模式经过验证。经过十年的改进，在构建真实产品时，很少有东西会阻碍您前进。但是 PHP 的每进程模式使两件事无法实现：廉价的长连接（WebSocket、SSE、服务器推送通知，无需轮询）和在单个请求处理程序内部进行简单的并发 I/O。

Rust 通过 Tokio 为您免费提供这两者。问题在于 Rust Web 生态系统让您自己构建生产力层：选择一个 HTTP 库、选择一个 ORM、选择一个迁移工具、选择一个队列、将它们全部连接在一起、设计您自己的约定。每个应用都重新发明了 Laravel 已经标准化的东西。

Suprnova 就是将 Laravel 的约定复制到 Tokio 上所发生的情况。您获得：

- **相同的接口** - `routes!`、`Auth::user()`、`Cache::remember`、
  `Mail::send`、`Queue::push`、`Storage::disk("s3")`、`Notify::send`、
  `Schedule::call`、`Gate::allows`、Eloquent 查询构造器、软删除、工厂、观察者、广播，所有这些
- **不同的引擎** - 全异步、长连接作为一等公民、单个静态链接二进制文件、无预创建、无操作码缓存、无 FPM
- **类型安全** - 您的模型、路由和事件负载会在编译时接受检查；有问题的重构不会进入预发布环境
- **真正的前端方案** - Inertia.js 桥接到 Svelte 5、React 19 或 Vue 3.5 起步模板，无需维护单独的 API

## 设计原则

这些是框架作者对自己遵守的原则。它们解释了为什么一个章节会这样说。

**1. 对等源于 Laravel 变更日志。** 当 Laravel 发布一项功能时，Suprnova 会跟踪它。当今的基线是 Laravel 13.x，每个发布的子系统都已针对其进行审计。[Laravel 对等映射](parity.md)是明确的逐项功能表。

**2. 在 Rust 使事情变得更好的地方有意偏离。** 在 Laravel 做出了我们在 Rust 中不必做的 PHP 风格选择的地方，Suprnova 选择 Rust 风格的选择并说明这一点。最大的例子是并发：WebSocket、广播、后台工作进程和 HTTP/2 服务器推送是一等公民，而不是附加的。当您在章节中看到这被称出来时，寻找**“为什么 Suprnova 有所不同”**框。

**3. 无守门人。** Laravel 将某些功能限制在一个后端（例如通过 Postgres `pgvector` 进行向量搜索）。Suprnova 将后端视为驱动程序 - `Vector::driver("qdrant")`、`Vector::driver("pinecone")`、`Vector::driver("mariadb")`、`Cache::driver("redis")`、`Mail::driver("ses")`。您选择正确的工具；我们不为您选择。

**4. Suprnova 是 API 表面。** 在内部，我们使用 SeaORM、hyper、Tokio、serde、sqlx、validator、lettre 等等。您的代码中不应该出现任何这些。您依赖 `suprnova::*`。我们在框架根目录下重新导出您将使用的所有内容 - 包括 SeaORM 的 `Entity`、`Column`、`ActiveModel`、`QueryFilter` 等。脱围机制（`use suprnova::sea_orm;`）存在于精选表面不覆盖的罕见情况下，但您几乎不应该需要它。

## 包含的内容

非穷尽的映射。完整列表在 [`documentation.md`](documentation.md) 中。

| 领域 | 包含内容 |
|---|---|
| **HTTP** | `routes!` 宏、控制器、中间件、请求、响应、路由模型绑定、签名 URL、资源路由、重定向助手、CORS、CSRF、幂等键、超时、速率限制、带有 panic 恢复的结构化错误 |
| **数据库** | SeaORM 底层、多驱动程序（Postgres、MySQL、MariaDB、SQLite）、迁移、填充、查询构造器、具有保存点的事务、多连接读/写分离 |
| **Eloquent** | `#[suprnova::model]` 宏、所有 11 种关系类型、预加载、软删除、可修剪、作用域（本地+全局）、16 个生命周期事件、观察者、22 个内置转换、访问器/修改器、三个分页器、chunk/lazy/cursor 迭代、集合、复制 |
| **认证** | 框架守卫、中间件、提供者和浏览器会话；由 Magnetar 支撑的密码、passkey、魔法链接、OAuth、bearer 会话、锁定、记住我、auth-epoch 和迁移引擎；由提供者支撑的邮箱验证；框架 TOTP 兼容门面；策略宏和门 |
| **前端** | Inertia v3 桥接、Svelte 5 / React 19 / Vue 3.5 起步模板、类型化的 `#[derive(InertiaProps)]`、部分重新加载、自动 TypeScript 类型生成 |
| **后台** | 具有 memory/sync/redis/database/null 驱动程序的队列、批处理、链、作业中间件、失败作业存储、`#[command]`/`#[derive(Command)]` 控制台二进制文件、`Task` trait 调度程序、`#[workflow]` 长运行有状态工作、带有 panic 捕获自动重启的 `Supervisor` trait、命令总线、事件分发器 |
| **实时** | `ws!()` 宏用于类型化 WebSocket 处理程序、广播频道（公开、私有、呈现）、sea-streamer 分发、服务器发送事件、Web 推送（VAPID） |
| **缓存和存储** | Memory、Redis、Database 缓存驱动程序；原子操作；标记缓存；缓存锁；带有 fs/memory/s3/azblob/gcs 驱动程序的文件系统；路径遍历保护；具有多个后端的向量存储 |
| **邮件和通知** | `Mailable` trait、SMTP/SES/Mailgun/Postmark/SendGrid/Resend 驱动程序、RFC 5322 文件预览、内存/日志传输层，以及带有邮件/数据库/广播/webpush 通道的 `Notifiable` |
| **验证和数据** | `#[derive(Validate)]`、表单请求、异步验证、`#[derive(Data)]` 用于部分重新加载包含集、`#[derive(Resource)]` 用于 JSON:API |
| **支付** | 通用提供商接口（网关/MoR/重定向流）、Stripe 和 Paddle 参考适配器、具有 webhook 幂等性的镜像表、Inertia 结账组件 |
| **功能标志** | 数据库评估器、具有 TTL 的缓存评估器、功能中间件、通过同步 trait 的亚秒传播 |
| **测试** | `#[suprnova_test]`、`expect!`、`TestDatabase`、每个外部接口的伪造（Mail、Notify、Queue、Bus、Events、Storage、Http） |
| **CLI** | `suprnova new` 脚手架（Svelte/React/Vue）、`serve` 开发运行程序、`migrate*`、`db:sync`、`db:seed`、`make:*` 生成器、`model:prune`、每个项目的控制台二进制文件 |

## 生产就绪

该框架在范围和测试上都是生产级别的。截至当前 HEAD：

- 30 个文档化领域中 Laravel 13.x 的每个接口都已发布
- 独立代码审查提出的每个问题都已解决
- 工作区测试套件在每次更改时都通过
- `framework/src/lib.rs` 中的每个公开 API 都有文档 - 未记录的公开项会导致构建失败

截至 **v1.0.0**，公开 API 已稳定：应用固定到某个发布标签（`tag = "v<version>"` - 标签就是发布版本；不会发布到 crates.io），只有在版本号提升且对应的[变更日志](changelog.md)章节明确说明时，才会引入不兼容变更。

## 选择阅读路径

| 您是… | 开始于 |
|---|---|
| Laravel 开发者 | [从 Laravel 来](from-laravel.md) |
| 使用过 Axum/Actix/Rocket 的 Rust 开发者 | [从 Rust Web 来](from-rust-web.md) |
| 两者都是，或都不是，只想构建 | [安装](installation.md) → [快速上手](quickstart.md) |
| 寻找特定功能 | [`documentation.md`](documentation.md)（主目录） |
| 想知道“Suprnova 有 X 吗？” | [Laravel 对等映射](parity.md) |
