# 目录结构

当您运行 `suprnova new my-app --frontend svelte` 时，脚手架会给您这样的结构：

```
my-app/
├── Cargo.toml                      # crate 清单 + 依赖，两个 [[bin]] 目标
├── .env                            # 本地配置 - 数据库 URL、应用密钥、端口
├── .env.example                    # 用于运维/CI 的模板
├── .gitignore                      # 排除 target/、.env、node_modules/、public/assets/
├── cmd/
│   └── main.rs                     # 二进制入口；调用 Application::new().run()
├── src/
│   ├── lib.rs                      # 模块连接（`pub mod controllers;` 等）
│   ├── bootstrap.rs                # 注册服务、观察者、监听器 - Laravel 服务提供者的
│   │                               # Suprnova 类似物
│   ├── routes.rs                   # `routes!` 宏树 - 应用服务的所有 URL
│   ├── bin/
│   │   └── console.rs              # `cargo run --bin console <subcommand>` 入口 -
│   │                               # `php artisan` 的 Suprnova 类似物
│   ├── actions/
│   │   ├── mod.rs
│   │   └── example_action.rs       # 单方法可调用控制器
│   ├── commands/
│   │   └── mod.rs                  # `#[command]` 注解的处理程序在此注册
│   ├── config/
│   │   ├── mod.rs
│   │   ├── database.rs             # 类型化数据库配置（驱动程序、URL、连接池）
│   │   └── mail.rs                 # 类型化邮件配置
│   ├── controllers/
│   │   ├── mod.rs
│   │   ├── home.rs                 # GET / 处理程序
│   │   ├── auth.rs                 # 登录 / 注册 / 登出
│   │   └── dashboard.rs            # 需要认证；示例保护路由
│   ├── middleware/
│   │   ├── mod.rs
│   │   ├── logging.rs              # 请求/响应日志
│   │   └── authenticate.rs         # 基于会话的认证守卫
│   ├── migrations/
│   │   ├── mod.rs
│   │   ├── m_*_create_users_table.rs
│   │   ├── m_*_create_sessions_table.rs
│   │   ├── m_*_create_remember_tokens_table.rs
│   │   ├── m_*_create_workflows_table.rs
│   │   └── m_*_create_workflow_steps_table.rs
│   └── models/
│       ├── mod.rs
│       └── user.rs                 # `#[suprnova::model]` User 模型
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html                  # Vite 入口；挂载 SPA
│   └── src/
│       ├── main.{tsx,ts}           # Inertia 客户端配置（per-framework）
│       ├── app.css                 # 全局样式 + Tailwind
│       ├── pages/
│       │   ├── Home.{tsx,svelte,vue}
│       │   ├── Dashboard.{tsx,svelte,vue}
│       │   └── auth/
│       │       ├── Login.{tsx,svelte,vue}
│       │       └── Register.{tsx,svelte,vue}
│       └── types/
│           └── inertia-props.ts    # 从 #[derive(InertiaProps)] 自动生成
└── public/
    └── assets/                     # Vite 生产构建输出放在这里
```

Svelte 添加 `frontend/svelte.config.js` 和 `frontend/src/app.d.ts`。
Vue 添加 `frontend/src/shims-vue.d.ts`。

API 起步（`suprnova new my-api --api`）更精简：没有
`frontend/`、没有认证控制器，`cmd/main.rs` 被替换为
`src/main.rs`。

## 每个目录的用途

### `cmd/main.rs`

二进制入口点。一个短文件 - 通常 10-20 行 - 调用标准启动流程：

```rust
use suprnova::Application;
use my_app::{bootstrap, config, migrations, routes};

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

`Application::run()` 解析二进制的 CLI（`serve` / `web:run` /
`migrate*` / `schedule:*` / `workflow:work` / `queue:work`），加载
`.env`，运行您的配置函数，然后分发子命令。serve 路径也会运行您的启动函数并启动 HTTP 服务器。

在初始脚手架后，您几乎不会编辑 `cmd/main.rs`。

### `src/lib.rs`

一个平坦的模块声明文件：

```rust
pub mod actions;
pub mod bootstrap;
pub mod commands;
pub mod config;
pub mod controllers;
pub mod middleware;
pub mod migrations;
pub mod models;
pub mod routes;
```

这使得 `crate::controllers::home::index` 能够从
`routes.rs` 访问。

### `src/bootstrap.rs`

连接您的应用的单一函数。您可以在这里注册服务容器绑定、观察者、事件监听器、自定义中间件和任何其他启动时设置。它是 Laravel 的
`AppServiceProvider`、`EventServiceProvider`、`BroadcastServiceProvider`
等的类似物，全部在一个文件中：

```rust
use std::sync::Arc;
use suprnova::App;

pub async fn register() {
    // 把一个服务绑定进容器
    App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));

    // 注册一个 Eloquent 观察者
    crate::models::user::register_observer();

    // 监听事件
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
}
```

`register()` 每个进程运行一次，在配置加载器之后但在
`serve` 接受第一个请求之前。工作进程（`queue:work`、
`schedule:run`、`workflow:work`）重用相同的启动，所以它们看到相同的服务。参见 [应用启动](bootstrap.md)。

### `src/routes.rs`

您的 URL 表面。模块顶级的 `routes!` 宏展开为一个 `pub fn register() -> Router`，`cmd/main.rs` 将其交给
`Application::routes(...)`：

```rust
use suprnova::{get, post, put, delete, routes};
use crate::{controllers, middleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // Auth（已注册 + 受保护）
    get!("/login", controllers::auth::show_login).name("login.show"),
    post!("/login", controllers::auth::login).name("login.attempt"),
    post!("/logout", controllers::auth::logout).name("logout"),
    get!("/register", controllers::auth::show_register).name("register.show"),
    post!("/register", controllers::auth::register).name("register"),

    // 仪表板需要 authenticate 中间件
    get!("/dashboard", controllers::dashboard::index)
        .middleware(middleware::authenticate::auth())
        .name("dashboard"),
}
```

参见 [路由](routing.md)。

### `src/bin/console.rs`

您的项目 console 二进制。运行为 `cargo run --bin console
<subcommand>` 并分发框架的 `db:seed` 内置以及
`src/commands/` 中每个 `#[command]` 注解的处理程序（或 `#[derive(Command)]` 类型结构体）- 两种形式都通过 inventory
在编译时注册：

```bash
cargo run --bin console db:seed           # 框架内置
cargo run --bin console report:daily      # 您的自定义命令
```

长时间运行的工作进程（`queue:work`、`schedule:run`、
`schedule:work`、`workflow:work`）在主应用二进制上生存，因为 `Application::run()` 分发它们 - 调用它们为
`cargo run -- queue:work`（或通过 `suprnova schedule:run` /
`suprnova workflow:work` 如果您偏好整体 CLI）。

参见 [控制台](console.md)。

### `src/commands/`

您的 console 处理程序所在的位置。两种形式：一个带有 clap 派生参数和
`impl TypedCommand` 的类型结构体，或一个在
`async fn(Vec<String>) -> Result<(), FrameworkError>` 上的原始
`#[command]`。脚手架生成类型形式：

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "report:daily", description = "Generate the daily report")]
pub struct DailyReport {
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for DailyReport {
    async fn run(self) -> Result<(), FrameworkError> {
        // …
        Ok(())
    }
}
```

`suprnova make:command report-daily` 脚手架该文件并将其添加到
`src/commands/mod.rs`。参见 [控制台](console.md)。

### `src/config/`

类型化配置结构体。脚手架提供 `database.rs` 和
`mail.rs`；为您的应用关心的任何子系统添加您自己的。每个配置结构体从环境读取其值，`config::register_all()` 将它们注册到框架中：

```rust
use suprnova::{env, env_required};

#[derive(Clone, Debug)]
pub struct AnalyticsConfig {
    pub api_key: String,
    pub max_batch: u32,
}

impl AnalyticsConfig {
    pub fn from_env() -> Self {
        Self {
            api_key: env_required::<String>("ANALYTICS_API_KEY"),
            max_batch: env("ANALYTICS_MAX_BATCH", 100u32),
        }
    }
}
```

在 `config/mod.rs` 中连接它：

```rust
use suprnova::Config;

pub fn register_all() {
    Config::register(AnalyticsConfig::from_env());
}
```

参见 [配置](configuration.md)。

### `src/controllers/`

HTTP 处理程序函数。每个资源一个模块。每个接受 `Request` 并返回
`Response` 的 `pub async fn` 都可以从路由调用。

### `src/middleware/`

中间件实现。脚手架提供 `logging` 和
`authenticate`；您可以添加自己的为 `pub struct Foo`
加 `impl Middleware for Foo`。在 `bootstrap.rs` 中全局注册它们或通过 `routes!` 树中的 `.middleware(…)` 应用到每个路由。参见
[中间件](middleware.md)。

### `src/migrations/`

SeaORM 迁移。脚手架为认证 + 工作流表提供了一些。
`suprnova make:migration <name>` 添加一个新的。`suprnova
migrate`、`migrate:rollback`、`migrate:status`、`migrate:fresh`、
`db:sync` 都在这个目录上操作。参见 [迁移](migrations.md)。

### `src/models/`

您的 Eloquent 模型。每个模型一个文件，每个都是一个 `#[suprnova::model]`
结构体。脚手架提供 `user.rs`；通过手工编写新文件或在模式迁移后运行 `suprnova db:sync --regenerate-models` 来添加新模型。参见 [Eloquent](eloquent.md)。

### `src/actions/`

单方法可调用控制器。可选模式 - 当控制器只有一个方法时使用它们，您宁愿叫它“Action”也不想包装它。脚手架提供一个您可以删除或改编的示例。参见 [操作](actions.md)。

### `frontend/`

Vite + Inertia SPA。这是一个正常的前端项目 - `package.json`、
`vite.config.ts`、`tsconfig.json`、一个 `index.html` Vite 入口、源代码在 `src/` 下。Inertia 客户端配置在 `src/main.{tsx,ts}` 中，页面组件在 `src/pages/` 中。您的 Rust
`#[derive(InertiaProps)]` props 的 TypeScript 类型被 `suprnova generate-types`
重新生成到 `src/types/inertia-props.ts` 中。

参见 [前端](frontend.md)。

### `public/assets/`

Vite 放置生产构建的地方（`npm run build`）。Suprnova
服务器在生产中将此目录作为静态资产在 `/assets/*` 处提供。

## 应用增长时添加的目录

脚手架给您最小限度的设置 - 足以提供欢迎流程和受保护的仪表板。真实的应用会增长更多子系统。常见的添加：

| 目录 | 何时添加 |
|---|---|
| `src/jobs/` | 第一次 `Queue::push(SomeJob)` 时。参见 [队列](queues.md)。 |
| `src/listeners/` | 第一次 `Event::listen` 时。参见 [事件](events.md)。 |
| `src/observers/` | 第一次实现 `Observer<MyModel>` 时。参见 [Eloquent](eloquent.md#observers)。 |
| `src/notifications/` | 第一次实现 `Notification` 时。参见 [通知](notifications.md)。 |
| `src/mail/` | 第一次实现 `Mailable` 时。参见 [邮件](mail.md)。 |
| `src/policies/` | 第一次写 `#[policy]` 时。参见 [授权](authorization.md)。 |
| `src/factories/` | 第一次为测试写 `Factory<Model>` 时。参见 [Eloquent 工厂](eloquent-factories.md)。 |
| `src/seeders/` | 第一次为 `db:seed` 写 `Seeder` 时。参见 [数据填充](seeding.md)。 |
| `src/events/` | 第一次为您自己的事件类型 `impl Event` 时。参见 [事件](events.md)。 |
| `src/broadcasting/` | 第一次定义私有/presence `Channel` 时。参见 [广播](broadcasting.md)。 |
| `src/ws/` | 第一次写 `ws!()` 处理程序时。参见 [WebSocket](websockets.md)。 |
| `src/supervisors/` | 第一次实现长时间运行的 `Supervisor` 时。参见 [监督程序](supervisors.md)。 |
| `src/payments/` | 第一次为应用接入 Stripe/Paddle 时。参见 [支付](payments.md)。 |
| `src/props/` | 当您想让 `#[derive(InertiaProps)]` 结构体与控制器分离时。 |
| `resources/views/` | 第一次为邮件主体添加 Tera 模板时。 |
| `storage/` | 第一次写文件到本地文件系统磁盘时（参见 [文件存储](filesystem.md)）。 |
| `tests/` | 第一次写集成测试时。 |

您不必请求权限 - `mkdir src/jobs` 并添加
`pub mod jobs;` 到 `src/lib.rs`，就完成了。框架不强制目录名称；这些约定存在是为了让其他
Suprnova 开发者能够快速找到东西。

## 这个项目中的 dogfood `app/`

如果您是从 Suprnova 项目本身内部读这篇文档的，您会看到一个在根目录的 `app/` 目录，它一起使用所有框架功能。那是我们内部测试床 - 它同时行使支付、广播、Web 推送、工作流、监督程序等等。它不是新应用的干净参考；上面的脚手架输出是故意更小和更容易学习的。当您想看到这些部分如何组合的最大化示例时再读 `app/`。

## 下一步

- [配置](configuration.md) - 如何 `.env` 变成类型化配置
- [应用启动](bootstrap.md) - `bootstrap.rs` 实际上做什么
- [路由](routing.md) - 您的第一条路由
- [服务容器](container.md) - 如何 `App::bind` 和 `App::get`
  工作
