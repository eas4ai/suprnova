# 任务调度

计划任务是框架按一个 cron 表达式运行的异步函数 - 每分钟、每小时、每天、每周，或者任何自定义的 5 字段 cron。任务活在您的应用二进制文件内部；`schedule:run` 会把到期的任务评估一次（从系统 cron 里调用它），而 `schedule:work` 会把同一个评估器作为一个长期存活的守护进程运行。

## 生成任务

创建一个新的计划任务，最快的方式是使用 suprnova CLI：

```bash
suprnova make:task CleanupLogs
```

这条命令会：
1. 创建 `src/tasks/cleanup_logs_task.rs`，带一个可运行的任务骨架
2. 如果 `src/tasks/mod.rs` 不存在就创建它，重新导出这个任务
3. 如果 `src/schedule.rs` 不存在就创建它，用来注册任务
4. 在 `src/lib.rs` 里声明 `pub mod schedule;` 和 `pub mod tasks;`
5. 把 `.schedule(<crate>::schedule::register)` 接入 `cmd/main.rs`（或者 API 起始模板里的 `src/main.rs`）中您的应用构建器

第 2 到 5 步都是幂等的，所以重新运行 `make:task` 能修复被手动移除的接线。这个调度器运行在您的应用二进制文件内部 - 没有一个需要单独构建或部署的调度器可执行文件。

```bash Examples
# 会在 src/tasks/cleanup_logs_task.rs 里创建 CleanupLogsTask
suprnova make:task CleanupLogs

# 会在 src/tasks/send_reminders_task.rs 里创建 SendRemindersTask
suprnova make:task SendReminders

# 也可以带上 "Task" 后缀（结果相同）
suprnova make:task BackupDatabaseTask
```

```rust Generated File
//! CleanupLogsTask 计划任务
//!
//! 用 `suprnova make:task cleanup_logs_task` 创建。

use std::time::Instant;

use async_trait::async_trait;
use suprnova::{Task, TaskResult};

/// CleanupLogsTask - 一个计划任务。
///
/// 用流式 API 在 `src/schedule.rs` 里注册这个任务；下面这个骨架会给自己的运行
/// 计时，并在每次调用时打印一条结构化的日志行，这样您第一次把它接好的时候，
/// 它就能端到端地工作。
pub struct CleanupLogsTask;

impl CleanupLogsTask {
    /// 创建这个任务的一个新实例。
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

        // 把这里替换成真正的作业。这个骨架发布时是一个空操作式的成功，
        // 这样在实现被填进去之前，这个任务也能被调度和观察。

        println!(
            "[CleanupLogsTask] task finished in {} ms",
            started_at.elapsed().as_millis(),
        );
        Ok(())
    }
}
```

## 定义计划

suprnova 支持两种定义计划任务的方式：

### 1. 基于 Trait 的任务（推荐）

对于需要依赖或可复用逻辑的复杂任务，实现 `Task` trait，并在注册时配置这个计划：

```rust
// src/tasks/cleanup_logs_task.rs
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::{Task, TaskResult};
use crate::models::Log;

pub struct CleanupLogsTask;

impl CleanupLogsTask {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        // Eloquent 的工作方式和它在一个控制器内部完全一样；任务看到的是
        // 和一个请求处理程序一样的容器绑定（`DB::connection()`、`App::get::<T>()`）
        // - 参见下面的应用启动。
        let cutoff = Utc::now() - Duration::days(30);
        Log::query()
            .filter_op("created_at", "<", cutoff)
            .delete_all()
            .await?;

        println!("Old logs cleaned up successfully");
        Ok(())
    }
}
```

然后用流式的调度 API，在 `src/schedule.rs` 里注册它：

```rust
// src/schedule.rs
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );
}
```

### 2. 基于闭包的任务

对于快速的、不需要单独文件的内联任务：

```rust
// src/schedule.rs
use suprnova::Schedule;

pub fn register(schedule: &mut Schedule) {
    // 简单的闭包任务
    schedule.add(
        schedule.call(|| async {
            println!("Ping! Running every minute");
            Ok(())
        })
        .every_minute()
        .name("heartbeat")
    );

    // 配置过的闭包任务
    schedule.add(
        schedule.call(|| async {
            // 您的任务逻辑
            Ok(())
        })
        .daily()
        .at("09:00")
        .name("morning-report")
        .description("Sends daily morning report")
    );
}
```

## 注册任务

在 `src/schedule.rs` 里注册您的任务：

```rust
// src/schedule.rs
use suprnova::Schedule;
use crate::tasks;

pub fn register(schedule: &mut Schedule) {
    // 基于 trait 的任务，用流式 API 配置计划
    schedule.add(
        schedule.task(tasks::CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );

    schedule.add(
        schedule.task(tasks::SendRemindersTask::new())
            .daily()
            .at("09:00")
            .name("send:reminders")
            .description("Sends daily reminder emails")
    );

    schedule.add(
        schedule.task(tasks::BackupDatabaseTask::new())
            .weekly()
            .at("00:00")
            .name("backup:database")
            .description("Weekly database backup")
            .without_overlapping()
    );

    // 基于闭包的任务
    schedule.add(
        schedule.call(|| async {
            println!("Quick task!");
            Ok(())
        })
        .hourly()
        .name("quick-task")
    );
}
```

## 计划频率选项

suprnova 提供了一套流式 API，用来定义任务该在什么时候运行：

### 常见间隔

| 方法 | 描述 |
|--------|-------------|
| `.every_minute()` | 每分钟运行一次 |
| `.every_two_minutes()` | 每 2 分钟运行一次 |
| `.every_five_minutes()` | 每 5 分钟运行一次 |
| `.every_ten_minutes()` | 每 10 分钟运行一次 |
| `.every_fifteen_minutes()` | 每 15 分钟运行一次 |
| `.every_thirty_minutes()` | 每 30 分钟运行一次 |
| `.hourly()` | 每小时的第 0 分钟运行 |
| `.hourly_at(30)` | 每小时的第 30 分钟运行 |
| `.every_two_hours()` / `.every_three_hours()` / `.every_four_hours()` / `.every_six_hours()` | 每 N 小时，在整点运行 |
| `.daily()` | 每天午夜运行 |
| `.daily_at("03:00")` | 每天凌晨 3:00 运行 |
| `.twice_daily(1, 13)` | 每天运行两次（例如凌晨 1:00 和下午 1:00） |
| `.weekly()` | 每周日午夜运行 |
| `.monthly()` | 每月 1 号午夜运行 |
| `.monthly_on(15)` | 每月的指定日期运行 |
| `.quarterly()` | 在 1/4/7/10 月的 1 号午夜运行 |
| `.yearly()` | 在 1 月 1 号午夜运行 |

### 指定日期的计划

```rust
use suprnova::DayOfWeek;

// 在指定日期运行
.weekly_on(DayOfWeek::Monday)
.weekly_on(DayOfWeek::Friday)

// 简写的星期方法
.sundays()
.mondays()
.tuesdays()
.wednesdays()
.thursdays()
.fridays()
.saturdays()

// 多个日期
.days(&[DayOfWeek::Monday, DayOfWeek::Wednesday, DayOfWeek::Friday])

// 工作日/周末
.weekdays()  // 周一至周五
.weekends()  // 周六至周日
```

### 时间修饰符

给任何计划链上 `.at()`，设定一个具体的时间：

```rust
.daily().at("14:30")           // 每天下午 2:30
.weekly().at("09:00")          // 每周上午 9:00
.mondays().at("08:00")         // 每周一上午 8:00
.monthly().at("00:00")         // 每月 1 号午夜
```

### 自定义 Cron 表达式

要获得完全的控制力，就用 cron 语法：

```rust
// 标准 cron 格式：分钟 小时 日 月 星期
.cron("0 */2 * * *")    // 每 2 小时
.cron("30 4 * * 1-5")   // 工作日的凌晨 4:30
.cron("0 0 1,15 * *")   // 每月的 1 号和 15 号
```

如果这个表达式格式错误（字段数量不对，无法解析的步进/范围/列表），`.cron(...)` 就会**panic**。当这个表达式是在运行时提供的（配置、用户输入），并且您更想让这个解析错误向上传播时，就用 `.try_cron(expr)`：

```rust
schedule.add(
    schedule.task(MyTask::new())
        .try_cron(env_expr)?   // 表达式错误时返回 Err(String)
        .name("from-config")
);
```

同样的 `panic` / `try_*` 配对，存在于每一个数值范围的构建器方法上：`try_hourly_at`、`try_daily_at`、`try_twice_daily`、`try_monthly_on`。不可能失败的那些变体，会在数值超出范围时 panic（例如 `daily_at("25:00")` 或者 `monthly_on(40)`）；可能失败的那些对应版本会返回 `Err(String)`。

## 任务配置

### 防止重叠

当同一个任务的前一次运行仍处于飞行中时，跳过一个节拍：

```rust
schedule.add(
    schedule.task(LongRunningTask::new())
        .daily()
        .name("long-task")
        .without_overlapping()
);
```

**这把锁如何工作。** 当这个标志被设置时，suprnova 会尝试通过已配置的 [`Cache`](cache.md) 后端（`schedule:lock:<task-name>`）获取一个分布式互斥锁。一次成功的获取会运行这个任务并释放这把锁；一次发生争用的获取，会被报告为一次成功的跳过 - `Ok(())`，同时这个任务的跳过计数器会递增一次，这样可观测性表面就能看到它，而不会污染 `schedule:run` 的退出码。

**跨进程保护需要 Cache。** 如果您运行多个进程来调度同一个任务（例如，几台机器都从系统 cron 调用 `suprnova schedule:run`，或者一个负载均衡器背后有多个 `schedule:work` 守护进程），Cache 后端正是协调它们的东西。**没有一个已配置的 Cache，`without_overlapping()` 会悄悄退化成一个逐进程的 `AtomicBool`** - 两个独立的进程不会看到彼此的锁。这个回退第一次触发时，框架会发出一条一次性的 `WARN`（`suprnova::schedule`），让运维人员注意到这个更弱的保证：

> `without_overlapping() falling back to in-process AtomicBool protection - Cache is not bootstrapped. Multi-process deployments will NOT see each other's locks. Configure Cache (CACHE_DRIVER=memory|redis) before relying on cross-process overlap protection.`

**自定义锁 TTL。** 这把锁的 TTL 默认是 30 分钟 - 足够让大多数任务完成，又足够短，让一个持有这把锁崩溃的任务，不需要运维人员介入就能解锁下一个节拍。可以用 `.without_overlapping_for(Duration)` 逐任务覆盖它。`Duration::ZERO` 在各个缓存后端之间是未定义的（Redis 会报错，内存后端会立即过期，Memcached 会把它当作“永不过期”），所以这个构建器会把它强制改成 30 分钟的默认值，并带一条一次性的 `WARN`，这样运维人员就能修复这个调用点。

```rust
use std::time::Duration;

schedule.add(
    schedule.task(SlowBackupTask::new())
        .daily()
        .name("backup:full")
        // 这个作业合理地会运行得比 30 分钟的默认值更久；
        // 给这把锁一个 2 小时的 TTL，这样一次缓慢的运行就不会被
        // 下一个节拍抢占。
        .without_overlapping_for(Duration::from_secs(2 * 3600))
);
```

### 只在一台服务器上运行

无论有多少个副本在运行这个调度器，都让一个任务在每一个到期的节拍上恰好运行一次：

```rust
schedule.add(
    schedule.task(NightlyBillingTask::new())
        .daily()
        .at("02:00")
        .name("billing:nightly")
        .on_one_server()
);
```

**没有它会出什么问题。** 每一个运行 `schedule:work` 的副本都会独立地评估这个计划，没有什么能阻止它们全都认定同一个节拍归自己所有。经过测量，三个副本每分钟都会产出同一个任务的三次执行，没有任何偏差。对于一个夜间的计费作业来说，这意味着每个客户都会被收费三次。

**为什么 `without_overlapping()` 覆盖不了这个问题。** 这两者看起来很像，解决的却是不同的问题：

| | 锁键 | 持有多久 | 防止什么 |
|---|---|---|---|
| `without_overlapping()` | 任务 | 这个任务的运行时长 | 一次缓慢的运行和它自己的下一个节拍重叠 |
| `on_one_server()` | 任务 **+ 这个节拍** | 这个节拍的窗口期 | 第二个副本运行同一个节拍 |

真正重要的区别在于这把锁什么时候被释放。`without_overlapping()` 会在处理程序一返回就释放 - 对于一个快速的任务，这甚至发生在第二个副本还没来得及看一眼之前，所以全部 N 个副本仍然都会运行。`on_one_server()` 则刻意让它的锁在处理程序之后继续持有，靠 TTL 让它过期，因为一个在同一个节拍里晚到的副本，必须发现这把锁已经被占用了。

它们可以组合使用。一个既长时间运行、又必须是单服务器的任务，两者都要。

**需要一个共享的缓存。** 这次选举是一把 [`Cache`](cache.md) 锁，所以“一台服务器”的意思是“共享同一个缓存后端的那些进程里的一个”。在 `CACHE_DRIVER=memory` 之下，这把锁活在单个进程的堆里，每个副本都会赢得自己的那次选举，而这个保证会悄无声息地缺失。

在生产环境里，这是一次**启动失败**，不是一条警告：

> `refusing to boot in production: 1 task(s) request single-server execution (billing:nightly) but CACHE_DRIVER is memory or unset, so the election lock lives in this process's heap. Every replica would win its own election and run the task, which is what on_one_server() exists to prevent. Set CACHE_DRIVER=redis with REDIS_URL, or set SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true to acknowledge per-process locking - which is only accurate if you run exactly one scheduler.`

如果您的部署确实只运行一个调度器，就设置 `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true`。在生产环境之外，内存驱动程序仍然可用，框架只会警告一次。

**自定义锁 TTL。** 默认是 60 秒 - 一个对齐到分钟的节拍。两个极端都要紧：太短的话，一个节拍晚到了几秒的副本会发现这把锁已经没了，于是再运行一次这个任务；太长的话，这把锁会活得比它的节拍还久，于是*下一次*到期的运行会发现它被占着，整个被跳过。对于更粗粒度的计划，请用 `.on_one_server_for(Duration)`。

```rust
use std::time::Duration;

schedule.add(
    schedule.task(HourlyRollupTask::new())
        .hourly()
        .name("rollup:hourly")
        // 一个按小时运行的任务，只需要这把锁活得比副本仍可能
        // 认为这个节拍到期的那个窗口更久。
        .on_one_server_for(Duration::from_secs(300))
);
```

**如果这个缓存无法访问**，这个节拍会被跳过，而不会运行。失去协调的那一刻，恰恰是最不该放行所有副本的时刻：一个被跳过的节拍在下一个节拍还能恢复，重复的副作用通常就不能了。

### 为什么 Suprnova 有所不同

Laravel 的 `onOneServer()` 是同样这种可选启用的机制，Suprnova 保留了它：逐服务器的任务 - 日志轮转、预热一个本地缓存 - 都是合理的，并且仍然可以表达。

不同之处在于失败模式。Laravel 会心甘情愿地在一个没法协调的缓存驱动程序上运行 `onOneServer()`。Suprnova 则会拒绝在生产环境启动，理由和内存速率限制器一样：一个悄悄做得比它声称的少得多的控制手段，比一个明显缺失的控制手段更糟。

### 在后台运行

把任务从每个节拍的关键路径里分离出来，这样它们就不会阻塞其他到期任务的启动：

```rust
schedule.add(
    schedule.task(BackgroundTask::new())
        .hourly()
        .name("background-task")
        .run_in_background()
);
```

**Panic 隔离。** 后台任务运行在一个带 `catch_unwind` 的 `tokio::task::JoinSet` 内部，所以一个发生 panic 的任务，会表现为一个记在这个任务名下的 `FrameworkError`，而不会拖垮整个调度器。`schedule:work` 这个守护进程会在关闭时（Ctrl-C / SIGTERM）排空这个 JoinSet，这样飞行中的后台任务会在退出之前完成。

**和 `without_overlapping` 组合使用。** 这两个标志可以组合 - 一个带 `without_overlapping()` 的后台任务，会 spawn 进这个 JoinSet，并从这个被 spawn 出来的 future 内部获取这把重叠锁，所以上面描述的锁语义仍然适用。

### 同分钟去重

Cron 的精度是分钟级的，suprnova 强制执行这一点：如果同一个任务，在同一个进程内，被要求在同一个挂钟分钟内运行两次，第二次调用就是一次空操作式的跳过 - `Ok(())`，同时这个任务的跳过计数器会递增。这堵住了一整类 bug：一个守护进程循环，或者一次紧凑的 `schedule:run` 调用，本来可能会在同一分钟里多次运行一个 `.every_minute()` 任务。

这道进程内的关卡是**始终开启的**，和 `without_overlapping` 无关。它**不会**跨越进程（每个进程都有自己逐任务的状态）。如果您需要跨进程的同分钟协调，请叠加 `without_overlapping`
+ 一个已配置的 Cache 后端 - 两者一起覆盖两个方向。

## 运行调度器

suprnova 提供了用于运行计划任务的 CLI 命令：

### 只运行一次

把所有到期的任务都执行一次（通常由系统 cron 每分钟调用一次）：

```bash
suprnova schedule:run
```

### 守护进程模式

持续运行，每分钟检查一次到期的任务：

```bash
suprnova schedule:work
```

这对开发环境来说是理想的，或者当您使用像 systemd 这样的进程管理器时也是。

### 列出任务

显示所有已注册的计划任务：

```bash
suprnova schedule:list
```

输出：
```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] - Removes logs older than 30 days
  send:reminders [0 9 * * *] - Sends daily reminder emails
  backup:database [0 0 * * 0] - Weekly database backup
```

## 生产环境设置

### 使用 Cron

添加一条 cron 记录，每分钟运行一次这个调度器：

```bash
* * * * * cd /path/to/your/project && suprnova schedule:run >> /dev/null 2>&1
```

**跨进程协调。** 如果您在多台主机上从系统 cron 运行 `schedule:run`（或者让它和一个 `schedule:work` 守护进程并存），带 `.without_overlapping()` 的任务就需要一个已配置的 **Cache** 后端（生产环境推荐 `CACHE_DRIVER=redis`）来跨进程协调。没有它，这个重叠标志就会退化成逐进程的保护，同一个任务可能会在同一分钟里在多台主机上运行。完整的锁语义请参见上文的[防止重叠](#防止重叠)。

### 使用 Systemd

为这个调度器守护进程创建一个 systemd 服务：

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/path/to/suprnova schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

## 访问应用上下文

计划任务和控制器一样，完全能访问应用上下文：

```rust
use async_trait::async_trait;
use suprnova::{App, Task, TaskResult};
use crate::actions::SendEmailAction;
use crate::models::User;

pub struct SendRemindersTask;

#[async_trait]
impl Task for SendRemindersTask {
    async fn handle(&self) -> TaskResult {
        // Eloquent：`.get()` 返回一个您可以遍历的 `Collection<User>`。
        let users = User::query()
            .filter("reminder_enabled", true)
            .get()
            .await?;

        // 任何在 `bootstrap.rs` 里绑定的东西，这里同样能触达。
        let send_email = App::get::<SendEmailAction>()
            .expect("SendEmailAction bound in bootstrap()");

        for user in users.iter() {
            send_email.execute(&user.email, "Daily Reminder").await?;
        }

        Ok(())
    }
}
```

## 文件组织

计划任务推荐的文件结构：

```
src/
├── tasks/
│   ├── mod.rs              # 重新导出所有任务（由 make:task 自动更新）
│   ├── cleanup_logs_task.rs
│   ├── send_reminders_task.rs
│   └── backup_database_task.rs
├── schedule.rs             # 注册任务（由 schedule:* 命令运行）
├── bootstrap.rs
├── routes.rs
└── lib.rs                  # 声明 `pub mod schedule;` + `pub mod tasks;`
cmd/
└── main.rs                 # 调用 `.schedule(<crate>::schedule::register)`
```

**src/tasks/mod.rs：**
```rust
pub mod cleanup_logs_task;
pub mod send_reminders_task;
pub mod backup_database_task;

pub use cleanup_logs_task::CleanupLogsTask;
pub use send_reminders_task::SendRemindersTask;
pub use backup_database_task::BackupDatabaseTask;
```

## 把调度器接入您的应用

`make:task` 会自动把 `.schedule(<crate>::schedule::register)` 接入您的 `Application` 构建器。如果您是手工搭建这条链的，相关的调用就在 `Application` 上：

```rust
// cmd/main.rs (or src/main.rs for the api starter)
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)        // <- 就是这一行
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await;
```

没有 `.schedule(...)`，所有的 `schedule:*` 子命令都会报告没有任务被注册。`schedule:work` 和 `schedule:run` 也会运行和 HTTP 服务器一样的运行时驱动程序和 `bootstrap_fn`，所以在启动时注册的观察者、监听器和容器绑定，对您的任务处理程序来说是可见的，就跟对控制器一样（参见[应用启动](bootstrap.md)）。

### 为什么 Suprnova 有所不同

Laravel 的调度器本身就是一个单一的 Artisan 命令（`schedule:run`），由 PHP-cron 每分钟触发一次。PHP 运行时会启动起来，评估到期的任务，在进程内运行它们或者 shell 出去执行，然后再把这个运行时拆掉。PHP 没有长期存活的进程，所以这个守护进程形态（`schedule:work`）是从 Lumen 反向移植回来的，作为一种应对没有 crontab 访问权限站点的变通方案，随 Laravel 自身一起发布。

在 Suprnova 里，这个守护进程是头等的。`schedule:work` 运行在一个本就长期存活的 Tokio 运行时内部，所以：

- **后台任务（`run_in_background`）能和节拍循环组合。** Laravel 为每一个后台任务都 spawn 一个子进程；我们则 spawn 进一个 `JoinSet`，并在下一个节拍或者关闭时，把完成情况呈现出来。
- **优雅关闭是一条 `tokio::select!` 的分支。** Ctrl-C / SIGTERM 会在退出之前排空飞行中的后台任务；进程内的任务会完成它们当前的这次调用。
- **同分钟去重是进程内状态。** 每个任务一个 `last_run_minute` 原子变量，保证单个进程不会重复触发一个对齐到分钟的任务，即便这个循环节拍走得很快。PHP 做不到这一点 - 每一次 cron 节拍都是一个全新的进程 - 这正是为什么 Laravel 把文件系统锁当作唯一的一道防线。

由 `Cache::lock` 支撑的 `without_overlapping` 仍然存在，用于多进程的情形（多台主机上的系统 cron，负载均衡器背后的多个 `schedule:work` 守护进程）。这是同一种机制，只是位于一个调度器并不总是需要的层次上。

## 总结

| 特性 | 用法 |
|---------|-------|
| 创建任务 | `suprnova make:task TaskName` |
| 基于 Trait | 实现 `Task` trait，在注册时配置计划 |
| 基于闭包 | `schedule.call(\|\| async { ... })` |
| 注册任务 | `schedule.add(schedule.task(...).daily().name("..."))` |
| 接入应用 | `Application::new().schedule(schedule::register)` |
| 运行一次 | `suprnova schedule:run` |
| 运行守护进程 | `suprnova schedule:work` |
| 列出任务 | `suprnova schedule:list` |
| 防止重叠 | `.without_overlapping()`（默认通过 Cache 后端提供 30 分钟的锁 TTL） |
| 自定义重叠 TTL | `.without_overlapping_for(Duration)` |
| 后台运行 | `.run_in_background()`（通过 JoinSet 实现 panic 隔离） |
| 同分钟去重 | 逐进程始终开启；被跳过的运行返回 `Ok(())` |
| 运行时校验 cron | `.try_cron(expr)` / `.try_daily_at(s)` / `.try_hourly_at(n)` |

## 下一步

- [调度命令](cli-scheduling.md) - `schedule:run` / `schedule:work` / `schedule:list` 的 CLI 参考
- [队列](queues.md) - 面向那些应该由一个工作进程接手、而不是靠时钟节拍触发的工作
- [控制台](console.md) - 面向一次性运维任务的 `#[command]`（不在计划之内）
- [缓存](cache.md) - 驱动跨进程 `without_overlapping` 的那个后端
- [应用启动](bootstrap.md) - `.schedule(...)` 是如何接入这个构建器的，以及任务能从容器里解析出什么
