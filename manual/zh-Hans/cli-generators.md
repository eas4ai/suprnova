# 代码生成器

`suprnova make:*` 这个家族，会为项目的每一部分脚手架出那份约定俗成的文件 -
一个控制器、一个操作、一个中间件、一个 console 命令、一个领域错误、
一个计划任务、一个 Inertia 页面或者 props 结构体、一条数据库迁移 -
并把这个新模块接入它的父级 `mod.rs`（在需要的地方，还有 `src/lib.rs`
和 `cmd/main.rs`）。当您本来要重新敲一遍同样的样板代码
+ `pub mod x;` 这行导入时，就用它们 - 而这是大多数时候的情况。

## make:controller

脚手架出一个控制器 - 一个 `src/controllers/` 里的文件，带一个单独的、名为 `invoke` 的 `#[handler]` 异步 fn。

```bash
suprnova make:controller User
suprnova make:controller order_item
```

这个名字会被规范化成 `snake_case` 作为文件名，并原样用在响应里的那个 `controller:` 回显上。只接受 ASCII 字母、数字和 `_` - 像 `api/User` 这样的路径会被拒绝。

### 生成的文件

```rust
// src/controllers/user.rs
use suprnova::{handler, json_response, Request, Response};

#[handler]
pub async fn invoke(_req: Request) -> Response {
    json_response!({
        "controller": "User"
    })
}
```

### 它接好了什么

1. 写入 `src/controllers/<name>.rs`，带着这个 `#[handler]` fn。
2. 把 `pub mod <name>;` 加进 `src/controllers/mod.rs`（如果这个文件不存在就创建它）。
3. 打印一条提示，让您在 `src/routes.rs` 里加一条路由：`.get("/<name>", controllers::<name>::invoke)`。

处理程序契约、提取器，以及 `routes!` 宏，请参见[控制器](controllers.md)。

---

## make:action

脚手架出一个单一职责的操作 - 一个可从容器解析的结构体，带一个返回 `Result<String, FrameworkError>` 的异步 `execute` 方法，这样这个骨架在您填入方法体之前就能编译。

```bash
suprnova make:action CreateUser
suprnova make:action SendNotification
```

这个名字会被转成 PascalCase；如果缺了 `Action` 后缀就会追加上，文件名则是这个结构体名字的 snake-case 形式。

### 生成的文件

```rust
// src/actions/create_user_action.rs
use suprnova::{injectable, FrameworkError};

#[injectable]
pub struct CreateUserAction {
    // Add injected dependencies as fields here, e.g.
    // db: suprnova::DbConnection,
}

impl CreateUserAction {
    pub async fn execute(&self) -> Result<String, FrameworkError> {
        Ok("CreateUserAction executed".to_string())
    }
}
```

### 它接好了什么

1. 写入 `src/actions/<snake>.rs`。
2. 把 `pub mod <snake>;` 加进 `src/actions/mod.rs`。
3. `#[injectable]` 会在链接时把这个操作注册进容器，这样任何控制器都能通过 `App::get::<CreateUserAction>()` 解析它，并调用 `action.execute().await?`。

解析-调用这套模式，以及操作如何和容器组合，请参见[操作](actions.md)。

---

## make:middleware

脚手架出一个中间件 - 一个实现了 `suprnova::Middleware` 的单元结构体。默认的方法体会给内层处理程序计时，并带着逐请求 id 记录入站 + 出站事件，所以它第一次就能端到端地跑起来。

```bash
suprnova make:middleware Auth
suprnova make:middleware RateLimit
```

这个名字会被转成 PascalCase；如果缺了 `Middleware` 后缀就会追加上。文件用的是这个基础名字（不带后缀）的 snake-case 形式，例如 `Auth` → `src/middleware/auth.rs`，结构体 `AuthMiddleware`。

### 生成的文件

```rust
// src/middleware/auth.rs
use std::time::Instant;

use suprnova::{async_trait, current_request_id, Middleware, Next, Request, Response};

pub struct AuthMiddleware;

#[async_trait]
impl Middleware for AuthMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        let method = request.method().to_string();
        let path = request.path().to_string();
        let request_id = current_request_id()
            .map(|id| id.as_str().to_string())
            .unwrap_or_default();
        let started_at = Instant::now();

        println!(
            "[AuthMiddleware] --> {} {} (request_id={})",
            method, path, request_id,
        );

        let response = next(request).await;

        println!(
            "[AuthMiddleware] <-- {} {} ({} ms, request_id={})",
            method, path, started_at.elapsed().as_millis(), request_id,
        );

        response
    }
}
```

### 它接好了什么

1. 写入 `src/middleware/<snake>.rs`。
2. 把 `mod <snake>;` + `pub use <snake>::<StructName>;` 加进 `src/middleware/mod.rs`（如果需要就创建它）。
3. 同时打印逐路由的形态（`.get("/path", handler).middleware(AuthMiddleware)`）和全局的形态（`bootstrap.rs` 里的 `global_middleware!(middleware::AuthMiddleware)`）。

完整的链语义、排序，以及全局与逐路由的区分，请参见[中间件](middleware.md)。

---

## make:command

脚手架出一个 console 命令 - 一个 `#[derive(clap::Parser, Command)]` 结构体，会在链接时被逐项目的 `console` 二进制文件通过 `inventory` 捡起来。默认的方法体是一个 `println!("…: not yet implemented")`，这样这条命令立刻就能运行。

```bash
suprnova make:command CleanCache
suprnova make:command mail:send
suprnova make:command clean-cache
```

命名遵循三条规则：

- 包含 `:` 的输入，会被原样用作注册的命令名（Laravel 那种命名空间风格：`db:seed`、`mail:send`）。
- 否则，这个 snake-case 的 fn 名字会被转成 kebab 形式来注册（`CleanCache` → 命令 `clean-cache`）。
- 这个 Rust 文件和结构体，始终是同一个标识符的 snake-case / PascalCase 形式。

### 生成的文件

```rust
// src/commands/clean_cache.rs
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "clean-cache", description = "TODO: describe what clean-cache does")]
pub struct CleanCache {
    // Add clap-derive args here.
}

#[async_trait]
impl TypedCommand for CleanCache {
    async fn run(self) -> Result<(), FrameworkError> {
        println!("clean-cache: not yet implemented");
        Ok(())
    }
}
```

### 它接好了什么

1. 写入 `src/commands/<snake>.rs`。
2. 把 `pub mod <snake>;` 加进 `src/commands/mod.rs`（如果需要就创建它）。
3. 如果 `src/lib.rs` 缺了 `pub mod commands;` 就醒目地警告 - 没有它，这条命令就没法链接进这个 console 二进制文件。
4. 打印这条运行命令：`cargo run --bin console -- clean-cache`。

完整的类型化命令表面、给纯 argv 处理程序用的 `#[command]` 简写，以及逐项目 console 二进制文件的角色，请参见[控制台](console.md)。

---

## make:error

脚手架出一个领域错误 - 一个标注了 `#[domain_error]` 的单元结构体，这样它天生就带着一个 HTTP 状态、一条 `Display` 消息，以及一个 `From<…> for FrameworkError` 的实现。

```bash
suprnova make:error UserNotFound
suprnova make:error PaymentFailed
```

这个结构体名字是 PascalCase，文件名是 snake-case。默认状态是 500，消息则是这个结构体名字的句首大写形式 - 请在生成出来的文件里把这两个属性都改成符合实际情况的值。

### 生成的文件

```rust
// src/errors/user_not_found.rs
use suprnova::domain_error;

#[domain_error(status = 500, message = "User not found")]
pub struct UserNotFound;
```

把 `status = 500` 改成任何合适的值 - `404` 用于未找到，`402` 用于需要付款，`403` 用于禁止访问 - 并编辑这条消息字符串。要拿到更丰富的载荷，就给这个结构体加上具名字段，并在一个手写的 `Display` 实现里通过插值引用它们（到那时候就把 `#[domain_error]` 这个宏去掉）。

### 它接好了什么

1. 写入 `src/errors/<snake>.rs`。
2. 把 `pub mod <snake>;` 加进 `src/errors/mod.rs`（如果需要就创建它）。
3. 如果 `errors/` 目录是刚创建出来的，就会警告您要在 `src/lib.rs` 里声明 `mod errors;`。

### 使用方式

在一个返回 `Response` 的处理程序内部，把这个领域类型提升成一个 `FrameworkError`，这样 `?` 就能干净地短路：

```rust
use crate::errors::user_not_found::UserNotFound;
use suprnova::FrameworkError;

#[handler]
pub async fn show(req: Request) -> Response {
    let id = req.param("id")?;
    let user = find_user(id).await
        .ok_or_else(|| FrameworkError::from(UserNotFound))?;
    json_response!({ "user": user })
}
```

完整的自定义错误脉络，包括什么时候该用 `#[domain_error]`、什么时候该用 `AppError::bad_request(…)`、什么时候该用一个手写的 `HttpError` 实现，都在[错误处理](errors.md)这一章里。

---

## make:task

脚手架出一个计划任务 - 一个实现了 `suprnova::Task` 的单元结构体，会打印结构化的开始/结束行，这样在您填入真正的方法体之前，这个脚手架就已经会记录进度了。

```bash
suprnova make:task CleanupLogs
suprnova make:task SendReminders
```

这个名字会被转成 PascalCase；如果缺了 `Task` 后缀就会追加上。文件名是这个结构体名字的 snake-case 形式，例如 `CleanupLogs` → `src/tasks/cleanup_logs_task.rs`。

### 生成的文件

```rust
// src/tasks/cleanup_logs_task.rs
use std::time::Instant;

use async_trait::async_trait;
use suprnova::{Task, TaskResult};

pub struct CleanupLogsTask;

impl CleanupLogsTask {
    pub fn new() -> Self {
        Self
    }
}

impl Default for CleanupLogsTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        let started_at = Instant::now();
        println!("[CleanupLogsTask] task started");

        // Replace this with the real job.

        println!(
            "[CleanupLogsTask] task finished in {} ms",
            started_at.elapsed().as_millis(),
        );
        Ok(())
    }
}
```

### 它接好了什么

第一次调用 `make:task`，接线的工作比其他生成器都重 - 它会从零开始，在项目里创建出这个调度器的表面：

1. 如果缺失，就创建 `src/tasks/` 和 `src/tasks/mod.rs`。
2. 如果缺失，就创建 `src/schedule.rs`（那个 `register(schedule: &mut Schedule)` 入口点）。
3. 在 `src/lib.rs` 里声明 `pub mod schedule;` 和 `pub mod tasks;`。
4. 把 `.schedule(<crate>::schedule::register)` 插入 `cmd/main.rs` 或者 `src/main.rs` 里的 `Application::new()` 链上，就在 `.run()` 之前。
5. 写入 `src/tasks/<snake>.rs`，并把它加进 `src/tasks/mod.rs`。

后续的调用会跳过那些已经运行过的步骤。

### 注册这个任务

打开 `src/schedule.rs`，用这套流式的调度 API 加一条注册调用：

```rust
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes old log files daily"),
    );
}
```

然后运行这个调度器：

```bash
suprnova schedule:work   # 守护进程 - 每分钟检查一次
suprnova schedule:run    # 一次性 - 通常由 cron 调用
suprnova schedule:list   # 显示每一个已注册的任务
```

完整的任务表面（`hourly`、`weekly`、`cron(...)`、`between`、`when`、`without_overlapping`，时区处理）请参见[任务调度](scheduling.md)，以 cron 方式运行还是以守护进程方式运行的取舍，请参见[调度命令](cli-scheduling.md)。

---

## make:inertia

根据这个标志，脚手架出一个 Inertia 页面组件（默认），或者一个类型化的 Data 结构体（`--data`）。这个页面生成器会从 `.env` 里检测前端框架（Svelte 5、React 19、Vue 3.5），并产出匹配的文件扩展名。

### 页面模式（默认）

```bash
suprnova make:inertia About
suprnova make:inertia UserProfile
```

这个名字会被转成 PascalCase，如果缺了 `Page` 后缀就会追加上，所以 `About` → `AboutPage`。这个文件会落在 `frontend/src/pages/` 里，带着逐前端的扩展名：Svelte 是 `AboutPage.svelte`，React 是 `AboutPage.tsx`，Vue 是 `AboutPage.vue`。

示例（Svelte）：

```svelte
<!-- frontend/src/pages/AboutPage.svelte -->
<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">AboutPage</h1>
  <p class="mt-2">
    Edit <code class="bg-gray-100 px-1 rounded">frontend/src/pages/AboutPage.svelte</code> to get started.
  </p>
</div>
```

从一个控制器里渲染它：

```rust
inertia_response!(&req, "AboutPage", props)
```

控制器和页面之间的桥梁、部分重新加载，以及共享 props，请参见[页面组件](frontend-pages.md)和[Inertia 响应](frontend-inertia-responses.md)。

### Data 结构体模式（`--data`）

```bash
suprnova make:inertia UserProps --data
```

在 `app/src/props/` 里产出一个 `#[derive(Data, Validate)]` 结构体（不是 `src/props/` - 这个 `app/` 前缀是硬编码的，这样这个文件才会落在这个工作空间的示例/宿主应用里）：

```rust
// app/src/props/user_props.rs
use suprnova::Data;
use validator::Validate;

#[derive(Data, Validate)]
pub struct UserProps {
    pub id: i64,
    // Add fields here.
    //
    // Available field attributes:
    //   #[data(input_only)] - accepted on Deserialize, omitted from Serialize
    //   #[data(output_only)] - rejected on Deserialize, included in Serialize
    //   #[data(allow_include)] - registers as ?include=-eligible (default-deny)
    //
    // For PATCH endpoints, use suprnova::data::Field<T> to distinguish
    // absent from null. For lazy outbound fields, use suprnova::inertia::Prop<T>.
}
```

在一个控制器里用它来验证请求体：

```rust
let dto: UserProps = req.validate_json().await?;
```

---

## make:migration

脚手架出一个带时间戳的 SeaORM 迁移文件。详细内容在[CLI 迁移](cli-migrations.md)里讲了，那一章也会走一遍 `migrate` / `migrate:rollback` / `migrate:status` / `migrate:fresh` / `db:sync` 这些命令。这里是简版：

```bash
suprnova make:migration create_users_table
```

这个迁移名会被原样保留，并加上一个 `YYYYMMDDHHMMSS_` 的时间戳前缀，这样文件才能按时间顺序排列。生成出来的文件会落在 `migrations/` 里。

架构构造器表面请参见[迁移](migrations.md)，那个针对每个测试跑一个隔离数据库来运行迁移的 `TestDatabase::fresh` 模式，请参见[数据库测试](database-testing.md)。

---

## generate-types

从每一个标注了 `#[derive(InertiaProps)]` 的 Rust 结构体产出 TypeScript 接口。开发服务器会自动运行这个；这个独立的命令是给 CI 检查和一次性的重新生成用的。

```bash
suprnova generate-types [--output <PATH>] [--watch]
```

| 选项 | 默认值 | 描述 |
|---|---|---|
| `-o, --output <PATH>` | `frontend/src/types/inertia-props.ts` | 输出文件路径 |
| `-w, --watch` | off | 监视源文件，变化时重新生成 |

```bash
# 一次性
suprnova generate-types

# 监视模式（当您不想运行完整的开发服务器时有用）
suprnova generate-types --watch

# 自定义输出路径
suprnova generate-types --output frontend/src/types/props.ts
```

左边的 Rust 形态，产出右边的 TypeScript 接口：

```rust
#[derive(InertiaProps)]
pub struct UserPageProps {
    pub user: User,
    pub posts: Vec<Post>,
}
```

```typescript
export interface UserPageProps {
    user: User;
    posts: Post[];
}
```

完整的映射表（枚举、选项、日期、嵌套结构体）和覆盖钩子，请参见[TypeScript 类型](frontend-typescript-types.md)。

---

### 为什么 Suprnova 有所不同

Laravel 的 `php artisan make:*` 把一个文件丢进正确的目录里，就完事了 - PSR-4 自动加载会在框架下一次启动时把这个新类捡起来。Rust 没有对应的机制。`src/foo/bar.rs` 这样的一个文件，在 `src/foo/mod.rs` 声明 `pub mod bar;` 之前，不会被编译进这个 crate，而这个父级目录本身，也要用同样的方式在 `src/lib.rs` 里接好。

所以每一个 `suprnova make:*` 生成器做的是两件事，不是一件：它写入新文件，*同时*编辑最近的那个 `mod.rs`（对 `make:task` 和 `make:command` 来说，还要编辑 `src/lib.rs` 和 `cmd/main.rs`）。这就是为什么每一个生成器都会打印一行 `Created src/.../mod.rs` 或者 `Updated src/.../mod.rs` - 接线是这份工作的一部分，不是您要自己记住的一个后续步骤。

---

## 总结

| 命令 | 创建 | 接入 |
|---|---|---|
| `make:controller <name>` | `src/controllers/<snake>.rs` | `controllers/mod.rs` |
| `make:action <Name>` | `src/actions/<snake>_action.rs` | `actions/mod.rs` |
| `make:middleware <Name>` | `src/middleware/<snake>.rs` | `middleware/mod.rs` |
| `make:command <name>` | `src/commands/<snake>.rs` | `commands/mod.rs`（+ 警告关于 `lib.rs`） |
| `make:error <Name>` | `src/errors/<snake>.rs` | `errors/mod.rs` |
| `make:task <Name>` | `src/tasks/<snake>_task.rs` | `tasks/mod.rs`、`schedule.rs`、`lib.rs`、`main.rs` |
| `make:inertia <Name>` | `frontend/src/pages/<Name>Page.<ext>` | （没有模块接线） |
| `make:inertia <Name> --data` | `app/src/props/<snake>.rs` | （没有模块接线） |
| `make:migration <name>` | `migrations/YYYYMMDDHHMMSS_<name>.rs` | （没有模块接线） |
| `generate-types` | `frontend/src/types/inertia-props.ts` | 不适用 |

## 下一步

- [CLI 概览](cli.md) - 完整的子命令表
- [控制台](console.md) - `make:command` 会喂给它的那个逐项目 console 二进制文件
- [控制器](controllers.md) - `make:controller` 脚手架出来的那份处理程序契约
- [任务调度](scheduling.md) - 用来注册 `make:task` 生成出来的任务的那套流式调度 API
- [CLI 迁移](cli-migrations.md) - 和 `make:migration` 搭配的 migrate / db:sync 命令
