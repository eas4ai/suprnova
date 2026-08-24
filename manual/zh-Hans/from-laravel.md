# 从 Laravel 来

如果您已经发布过 Laravel 应用，您已经了解 Suprnova 的 80%。本章将您的编程习惯映射到 Rust 等价物，使您能够快速提高生产力。我们将展示您每天使用的模式、改变形状的模式，以及 Rust 提供但 PHP 无法提供的一些东西。

## 一览表

| 您在 Laravel 中写的 | 您在 Suprnova 中写的 |
|---|---|
| `composer create laravel/laravel my-app` | `suprnova new my-app --frontend svelte` |
| `php artisan serve` | `suprnova serve` |
| `php artisan migrate` | `suprnova migrate` |
| `php artisan make:controller PostController` | `suprnova make:controller post` |
| `Route::get('/posts/{id}', [PostController::class, 'show'])` | `get!("/posts/{id}", controllers::post::show)` (in `routes!`) |
| `class Post extends Model` | `#[suprnova::model] struct Post { … }` |
| `Post::find($id)` | `Post::find(id).await?` |
| `Post::where('status', 'published')->get()` | `Post::query().db_where("status", "published").get().await?` |
| `Auth::user()` | `Auth::user().await?` |
| `Cache::remember('key', 60, fn() => …)` | `Cache::remember("key", Some(Duration::from_secs(60)), \|\| async { … }).await?` |
| `Queue::push(new SendEmail($user))` | `Queue::push(SendEmail { user_id }).await?` |
| `Mail::to($u)->send(new Welcome($u))` | `Mail::to(&u.email).send(WelcomeMail { user: u }).await?` |
| `Storage::disk('s3')->put($path, $bytes)` | `Storage::disk("s3")?.put(&path, bytes).await?` |
| `Notification::send($u, new Invoice($i))` | `Notify::send(&u, &InvoiceNotification { invoice }).await?` |
| `Gate::allows('update', $post)` | `Gate::allows::<PostPolicy, _>("update", &user, &post).await?` |
| `request()->validate([...])` | `#[handler]` extracts an `#[derive(Data, Validate)]` arg directly |
| `event(new OrderShipped($order))` | `EventFacade::dispatch(OrderShipped { order }).await?` |
| `Bus::dispatch(new ProcessFoo($x))` | `Bus::dispatch(ProcessFoo { x }).await?` |
| `php artisan schedule:list` | `suprnova schedule:list` |
| `php artisan tinker` | （没有 REPL - 编写一次性的 `cargo run` 脚本或测试）|
| `composer require league/csv` | `cargo add csv` |

## 思维模式转变

### 异步无处不在

最大的变化是：每个数据库调用、HTTP 调用、文件 I/O、缓存调用、队列推送 - 任何跨越边界的操作 - 都是 `async` 的，您使用 `.await?` 来调用它。做了几个小时后，它就会融入节奏。在那之前，编译器会指出您忘记的每个地方。

```rust
// Laravel
$user = User::find($id);
$user->subscribe($plan);
Mail::to($user)->send(new Welcome($user));

// Suprnova
let user = User::find(id).await?;
user.subscribe(&plan).await?;
Mail::to(&user.email).send(WelcomeMail { user }).await?;
```

`?` 是 Rust 的“发生错误时提前返回”。处理程序返回 `Result<HttpResponse, HttpResponse>`（别名为 `Response`），所以数据库错误上的 `?` 会短路到您的错误转换器，客户端获得正确的 500（或 4xx，取决于错误类型）。您几乎不必编写 `try/catch` - `?` 会处理。

### 编译时模型

Eloquent 在运行时读取您的数据库架构，而 Suprnova 在编译时读取：

```rust
#[suprnova::model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

就是这样 - 这个结构体就是 Eloquent 模型。您得到 `Post::find`、`Post::query()`、`Post::create`、`post.update(...)`、`post.delete()`、软删除（通过 `#[model(soft_deletes)]`）、时间戳、观察者，所有这些。宏生成一个 SeaORM `Entity`、`Model`、`ActiveModel` 和 `Column` 枚举，并实现 Suprnova `Model` trait - 但您依赖的是 `Post`，而不是其他任何东西。

如果您在迁移中重命名一个列，该结构体不再与数据库架构匹配 - 根据您的配置，要么编译器在构建时捕获它，要么类型强制转换在第一次查询时失败。无论哪种方式，您在部署前就能发现问题，而不是之后。

### 单一二进制文件

没有 PHP-FPM，没有 nginx 配置读取 `index.php`，没有在部署时进行 `composer install`。`cargo build --release` 给您一个静态链接的二进制文件。使用 `scp` 将其传到服务器，用 `systemd` 运行，完成。或者构建一个容器 - `FROM scratch` 也可以工作。

我们有 [Railway、Digital Ocean 和 Hetzner 的部署方案](deployment.md)。通常的流程是：构建二进制文件、发送二进制文件、设置环境变量、运行。

## 框架映射

### 路由

`routes!` 结合了 `routes/web.php` 和 `routes/api.php` 的角色。

```rust
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),

    // 带共享前缀 + 中间件的路由组
    group("/admin")
        .middleware(crate::middleware::admin())
        .routes(routes! {
            get!("/users", controllers::admin::users::index).name("admin.users"),
            post!("/users", controllers::admin::users::store),
            put!("/users/{id}", controllers::admin::users::update),
            delete!("/users/{id}", controllers::admin::users::destroy),
        }),

    // 资源路由（Laravel 的 Route::resource）
    resource!("posts", controllers::post),
}
```

完整参考：[路由](routing.md)。值得了解的差异：

- 组中间件在注册时被 **压平** 到每个路由的中间件列表中（不作为单独的链层运行） - 这意味着分组不会产生额外的运行时成本。
- Laravel 的 `{id}` 和 Rails 风格的 `:id` 语法都可以工作；它们在内部被标准化。
- 命名路由通过 `route("posts.show", &[("id", "42")])` 解析，还有一个签名 URL 变体用于有时间限制的链接。

### 控制器

控制器只是一个返回 `Response` 的自由函数：

```rust
use suprnova::{Request, Response, json_response, HttpResponse};
use crate::models::Post;

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0").parse::<i64>()?;
    let post = Post::find_or_fail(id).await?;
    json_response!({ "post": post })
}
```

您也可以使用 `#[handler]` 宏在签名中提取类型化参数（路由参数、查询、正文、请求本身、容器服务）：

```rust
use suprnova::handler;

#[handler]
pub async fn show(post: post::Model) -> Response {
    // 路由模型绑定已自动运行；`post` 就是加载出来的那一行。
    json_response!({ "post": post })
}
```

`post::Model` 类型来自模型的生成模块 - 这是 `#[handler]` 用来选择路由模型绑定而不是默认表单请求提取的信号。如果该行不存在，绑定在您的代码运行前返回 404 - 与 Laravel 的隐式绑定行为相同。

还支持操作结构体（单方法“可调用”控制器，Laravel 风格）：参见 [操作](actions.md)。

### Eloquent

双 API 查询构造器接受 Laravel 名称或 Rust 习惯用法的名称 - 两者都可以，选择在调用点读起来最清晰的那个。

```rust
// Laravel 表面
let active = User::query()
    .db_where("status", "active")
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;

// Rust 表面（结果完全相同）
let active = User::query()
    .filter("status", "active")
    .order_by_desc("created_at")
    .take(20)
    .get()
    .await?;
```

`db_where` 是 Laravel 方的名称（裸 `where` 与 Rust 关键字冲突）。`filter` 是 Rust 习惯用法的别名。两者都存在；两者做同样的事情。对于非相等运算符，使用 `db_where_op`（或其 `filter_op` 别名）：`.db_where_op("status", "!=", "archived")`。参见 [Eloquent 参考](eloquent.md) - 这是最长的章节是有原因的，表面很广。

### Auth

```rust
use suprnova::{Auth, Credentials};

// 在处理程序中：
let user = Auth::user().await?;   // Option<Arc<dyn Authenticatable>>
let id = user.as_ref().map(|u| u.get_auth_identifier());

// 登录（例如在您的登录控制器内部）：
let creds = Credentials::password("alice@x.com", "secret");
Auth::attempt(&creds, false).await?;

// 退出登录：
Auth::logout().await?;
```

`Auth::attempt` 会通过默认的有状态守卫及其配置好的 `UserProvider` 校验凭据；生成的全栈脚手架使用的就是这条路径。`Auth::password()`、密码重置、`BruteForce`、passkey、魔法链接、OAuth、Bearer 会话和 Magnetar 会话管理都要求已安装 Magnetar 引擎。电子邮件验证和兼容性的 `TwoFactor` 门面仍由框架拥有。参见[认证](authentication.md)、[认证流程](auth-flows.md)和[OAuth 与无密码登录](oauth.md)。

### 迁移

您编写 SeaORM 迁移。即使语法是新的，形状看起来也会很熟悉：

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(
            Table::create()
                .table(Alias::new("posts"))
                .if_not_exists()
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key().auto_increment())
                .col(ColumnDef::new(Alias::new("title")).string().not_null())
                .col(ColumnDef::new(Alias::new("body")).text().not_null())
                .to_owned()
        ).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new("posts")).to_owned()).await
    }
}
```

`suprnova make:migration create_posts_table` 脚手架该文件。`suprnova migrate`、`migrate:rollback`、`migrate:status`、`migrate:fresh` 都按您期望的方式工作。`suprnova db:sync` 运行迁移并重新生成宏层编译的 SeaORM 实体。参见 [迁移](migrations.md)。

### 队列和调度

```rust
use suprnova::{FrameworkError, Job, Queue, async_trait};
use serde::{Deserialize, Serialize};

// 定义一个作业 - 数据放在结构体上，契约放在
// `impl Job` 上。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SendWelcomeEmail {
    pub user_id: i64,
}

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str {
        "SendWelcomeEmail"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}

// 把它推入队列：
Queue::push(SendWelcomeEmail { user_id: user.id }).await?;

// 或者带一个延迟：
Queue::later(
    std::time::Duration::from_secs(60),
    SendWelcomeEmail { user_id },
).await?;
```

工人使用 `cargo run -- queue:work` 运行。驱动程序包括内存和同步（进程内，用于测试）、数据库、redis 和 null。批处理、链接、独特作业、重试、退避、中间件、失败作业存储 - 全都有。参见 [队列](queues.md)。

调度使用 `Task` trait 和每个项目的调度器二进制文件：

```rust
use suprnova::{Task, TaskResult, async_trait};

pub struct DailyDigest;

#[async_trait]
impl Task for DailyDigest {
    async fn handle(&self) -> TaskResult {
        // …
        Ok(())
    }
}

// 在启动函数中注册（例如通过 Schedule::call / .task / .add）：
//   schedule.add(schedule.task(DailyDigest).daily().at("03:00").name("daily-digest"));
```

参见 [任务调度](scheduling.md)。

### 邮件、通知、广播

这些与 Laravel 一一对应。`Mailable` 是一个派生宏；`Notifiable` 是您用户模型上的一个 trait；通道是 `mail`/`database`/`broadcast`/`webpush`；广播支持公开、私有和呈现频道。参见 [邮件](mail.md)、[通知](notifications.md)、[广播](broadcasting.md)。

### 前端

没有 Blade。相反，前端是通过 Inertia.js 的真实 SPA，您从 Rust 传递类型化的 props：

```rust
use suprnova::{inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps, serde::Serialize)]
pub struct ShowProps {
    pub post: Post,
    pub comments: Vec<Comment>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    let comments = post.comments().get().await?;
    inertia_response!(&req, "Posts/Show", ShowProps { post, comments })
}
```

`Posts/Show` 是一个 Svelte 组件（或 React，或 Vue - 您的起步模板选择）。props 的 TypeScript 类型从 `InertiaProps` 派生自动生成 - 添加新的 prop 结构体后运行 `suprnova generate-types`，前端就会获得类型化绑定。

如果您曾在 Laravel 中通过 `inertia()` 使用过 Inertia，这是同样的事情 - 只是端到端类型化。参见 [前端概览](frontend.md)。

## 改变形状的事物

在 Suprnova 中，有些事物的运作方式不同。它们都不是障碍，但值得预先了解。

### 没有服务提供者

Laravel 有数十个服务提供者注册绑定、观察者、视图组合器等。Suprnova 在您的 `bootstrap.rs` 中有 **一个** 启动函数。您在那里按顺序注册所有东西。这不优雅，但是透明的 - 您可以在 30 行中准确看到您的应用启动的内容。

```rust
// bootstrap.rs
use std::sync::Arc;

pub async fn register() {
    suprnova::App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
    crate::observers::register();
}
```

[服务容器](container.md) 和 [应用启动](bootstrap.md) 章节有详细信息。

### 配置是类型化的

Laravel 使用 `config('app.timezone')` 返回数组说的任何东西，而 Suprnova 有类型化的配置结构体：

```rust
let cfg = suprnova::Config::get::<AppConfig>()?;
let tz = &cfg.timezone;   // &str，不是 mixed
```

您可以注册自己的类型化配置部分。参见 [配置](configuration.md)。

### 没有门面作为别名

Laravel 门面如 `DB::` 是在 `config/app.php` 中配置的类别名。Suprnova 门面是 crate 根目录中的真实模块：

```rust
use suprnova::{Auth, Cache, DB, Event, Gate, Mail, Notify, Queue, Schedule, Storage};
```

相同的表面，不需要全局别名。

### 编译时间是真实的

Rust 编译时间不是 PHP。清理构建一个新的 Suprnova 应用需要 1-2 分钟；开发期间的增量构建需要几秒钟。开发工作流程是相同的 - `suprnova serve` 监视更改并重新构建 - 但当您第一次更改宏并重新编译下游 crate 时，您会感到这一点。缓存很快就会收回成本。

### 借用检查器存在

大多数控制器和处理程序从不涉及生命周期注解 - 框架的签名隐藏它们。当借用检查器对您大喊时，通常是因为您尝试跨越 `.await` 保持引用，该 `.await` 穿过互斥体，或在跨越需要独占访问权限的等待调用的数据库事务中保持引用。错误很清楚，修复通常是 `.clone()` 或重组为较小的范围。

### 没有 `tinker` REPL

没有 REPL。最接近的等价物是 `examples/` 中的一次性 `cargo run` 脚本，或者一个 `#[suprnova_test]` 测试，用来练习您正在调试的东西。您在 tinker 中会做的大部分事情（戳一个模型、触发一个通知、分发一个作业）都是一个 5 行的测试。

## Laravel 章节的落脚点

快速查询，如果您知道您要什么但不知道在哪里：

| Laravel 主题 | Suprnova 章节 |
|---|---|
| 生命周期 | [请求生命周期](lifecycle.md) |
| 服务容器 | [服务容器](container.md) |
| 服务提供者 | [应用启动](bootstrap.md) |
| 门面 | [服务容器](container.md) |
| 路由 | [路由](routing.md) |
| 中间件 | [中间件](middleware.md) |
| CSRF 保护 | [CSRF](csrf.md) |
| 控制器 | [控制器](controllers.md) |
| 请求 | [请求](requests.md) |
| 响应 | [响应](responses.md) |
| URL 生成 | [URL 生成](urls.md) |
| 会话 | [会话](session.md) |
| 验证 | [验证](validation.md) |
| 错误处理 | [错误处理](errors.md) |
| 日志 | [日志](logging.md) |
| Artisan 控制台 | [控制台](console.md) + [CLI 参考](cli.md) |
| 广播 | [广播](broadcasting.md) |
| 缓存 | [缓存](cache.md) |
| 事件 | [事件](events.md) |
| 文件存储 | [文件系统和存储](filesystem.md) |
| HTTP 客户端 | [HTTP 客户端](http-client.md) |
| 本地化 | [本地化](localization.md) - Fluent `.ftl` 目录，不是 PHP 数组 |
| 邮件 | [邮件](mail.md) |
| 通知 | [通知](notifications.md) |
| 队列 | [队列](queues.md) |
| 速率限制 | [速率限制](rate-limiting.md) |
| 任务调度 | [任务调度](scheduling.md) |
| 认证 | [认证](authentication.md) |
| 授权 | [授权](authorization.md) |
| 电子邮件验证 | [认证流](auth-flows.md) |
| 密码重置 | [认证流](auth-flows.md) |
| 加密 | [加密](encryption.md) |
| 哈希 | [哈希](hashing.md) |
| 数据库 | [数据库](database.md) |
| 查询构造器 | [查询构造器](queries.md) |
| 分页 | [分页](pagination.md) |
| 迁移 | [迁移](migrations.md) |
| 填充 | [数据填充](seeding.md) |
| Eloquent | [Eloquent API](eloquent.md) |
| Eloquent: 关系 | [Eloquent 关系](eloquent-relationships.md) |
| Eloquent: 集合 | [Eloquent 集合](eloquent-collections.md) |
| Eloquent: 转换 / 强制类型 | [Eloquent 转换、访问器和修改器](eloquent-mutators.md) |
| Eloquent: API 资源 | [JSON:API 资源](eloquent-resources.md) |
| Eloquent: 序列化 | [Eloquent 序列化](eloquent-serialization.md) |
| Eloquent: 工厂 | [Eloquent 工厂](eloquent-factories.md) |
| 测试 | [测试](testing.md) |
| HTTP 测试 | [HTTP 测试](http-tests.md) |
| 数据库测试 | [数据库测试](database-testing.md) |
| 模拟 | [模拟和伪造](mocking.md) |
| Cashier (Stripe) | [支付 - Stripe 适配器](payments-stripe.md) |
| Cashier (Paddle) | [支付 - Paddle 适配器](payments-paddle.md) |
| Sanctum / Passport | 通过 `BearerTokenMiddleware` 使用 Magnetar Bearer 会话；没有独立的 Sanctum 或 Passport API |
| Horizon | 队列检查内置于框架；没有 Horizon 仪表板 |
| Telescope / Pulse | （延迟至 v2+） |

Laravel 有但 Suprnova 还没有的东西：

- Telescope / Pulse 仪表板。基础[可观测性](observability.md)已发布。
- Sanctum / Passport 软件包 API。Magnetar Bearer 会话和
  `BearerTokenMiddleware` 提供令牌认证，但不提供 Laravel 的令牌管理表面。
- Horizon 仪表板。队列检查内置于框架中。
- Blade - 按设计；Inertia 是前端方案
- `trans_choice` - [本地化](localization.md) 已发布，但复数是通过 CLDR 类别在消息内部选择的，而不是通过 `trans_choice` 采用的 `[1,19]` 风格的整数范围

## 下一步

- [安装](installation.md) - 让项目运行起来
- [快速上手](quickstart.md) - 在 5 分钟内构建一个小应用
- [路由](routing.md) - 从这里开始的自然下一章

或通过 [`documentation.md`](documentation.md) 随处跳转。
