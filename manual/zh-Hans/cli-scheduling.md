# 调度命令

面向这个逐分钟任务调度器的 CLI 表面。这三个 `schedule:*` 子命令，全都会委托进您应用二进制文件的 `Application::run()` 分发，所以它们看到的配置、服务、观察者和监听器，和一个请求处理程序看到的完全一样。完整的调度器模型 - `Task` trait、流式的 cron API、`without_overlapping`、`run_in_background` - 活在[任务调度](scheduling.md)里；这一章是这些命令本身的运维参考。

## 这些命令是怎么运行的

`suprnova schedule:run`、`suprnova schedule:work` 和 `suprnova schedule:list`，都是薄薄的外壳，针对当前目录下的项目调用 `cargo run -- schedule:<subcommand>`。在生产环境里，同样这些子命令也能直接在应用二进制文件上触达：

```bash
# 在开发环境里（从项目根目录，源码构建）：
suprnova schedule:run

# 在生产环境里（二进制文件在 PATH 上）：
/usr/local/bin/myapp schedule:run
```

运行时驱动程序（Cache、Queue、RateLimit、Mail）和您的 `bootstrap_fn`，会在任何任务运行之前就引导好，所以一个计划任务能从容器里解析服务，和一个控制器一模一样 - 参见[应用启动](bootstrap.md)。

您必须把调度器接入应用构建器，这些子命令才能找到任何任务：

```rust
// cmd/main.rs（backend 起步）或者 src/main.rs（API 起步）
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)   // <-- 这个调度器钩子
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await
```

`suprnova make:task <Name>` 会自动接好这个；如果您是手工搭建这条链的，就自己加上这个 `.schedule(...)` 调用。

## schedule:run

把每一个已注册的任务都评估一次，运行那些 cron 表达式匹配当前这一分钟的任务。设计成由系统 cron 每分钟调用一次。如果任何任务失败了，就以非零状态退出；如果这一分钟没有任何任务到期，就以零状态退出（附带 `No tasks were due.`）。

```bash
suprnova schedule:run
```

### 示例输出

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✓ send:reminders
```

当一个任务返回一个错误时，它这一行会带上 `✗` 前缀，并附上这条错误消息：

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✗ backup:database: connection refused
```

当这一分钟没有任何任务到期时：

```
Running due scheduled tasks...
No tasks were due.
```

### Crontab 条目

一条条目就能让这个调度器每分钟运行一次。这个应用二进制文件会自己评估所有到期的任务，所以这是一台生产主机唯一需要的 crontab 行：

```cron
* * * * * cd /path/to/your/project && /usr/local/bin/myapp schedule:run >> /var/log/myapp/schedule.log 2>&1
```

如果您是在多台主机上都从系统 cron 运行 `schedule:run`（或者让它和一个 `schedule:work` 守护进程并存），标记了 `.without_overlapping()` 的任务就需要一个已配置的 Cache 后端（`CACHE_DRIVER=redis` 是生产级的选择）来跨进程协调 - 锁的语义请参见[防止重叠](scheduling.md#preventing-overlapping)。

## schedule:work

把这个调度器作为一个长期存活的守护进程来运行。第一次节拍对齐到下一个分钟边界，之后这个循环每分钟评估一次到期的任务，直到它收到 `SIGINT`（Ctrl-C）或者 `SIGTERM`。关闭时，任何还在飞行中的 `run_in_background` 任务都会被等待完成，再退出，这样它们就不会在写入中途被拆掉。

```bash
suprnova schedule:work
```

### 示例输出

```
Starting scheduler daemon...
Press Ctrl+C to stop

==============================================
  suprnova Scheduler Daemon
==============================================
  3 task(s) registered. Press Ctrl+C to stop.
==============================================
```

每一次节拍都是安静的 - 只有失败才会被记录。关闭时：

```
suprnova: scheduler shutting down.
suprnova: waiting for 1 background task(s) to finish…

Scheduler daemon stopped.
```

### 使用场景

- **开发环境。** 不需要 crontab - 在一个终端里启动这个守护进程，看着它按节拍运行。
- **Docker。** 当您想让一个镜像扮演调度器这个角色时，就把它用作容器的主进程。
- **Systemd。** 把它当作一个长期运行的 unit 来管理（见下面的[systemd 服务单元](#systemd-服务单元)）。

### systemd 服务单元

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/usr/local/bin/myapp schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

`Restart=always` 会在这个守护进程崩溃时把它拉回来；`RestartSec=5` 给一个崩溃循环做防抖。因为这个框架的 Panic 边界会捕获发生 panic 的任务，把它们转换成 `FrameworkError`，一个坏任务不应该崩掉这个守护进程 - `Restart=always` 是留给那种罕见的、进程级的失败（OOM、父进程被杀）用的。

## schedule:list

打印每一个已注册的任务，带上它的 cron 表达式和描述。

```bash
suprnova schedule:list
```

### 示例输出

```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] - Removes logs older than 30 days
  send:reminders [0 9 * * *] - Sends daily reminder emails
  backup:database [0 0 * * 0] - Weekly database backup
  heartbeat [* * * * *]
```

在构造器上链了 `.description(...)` 的任务，会在 cron 表达式之后带上这条描述；没有描述的任务只显示 cron。

当什么都没注册时（`.schedule(...)` 这个构造器调用缺失了，或者 `schedule::register` 是个空操作）：

```
No scheduled tasks registered.
Define tasks in src/schedule.rs and wire it with `Application::schedule(schedule::register)`.
```

## 生成一个任务

这个框架发布了一个生成器，它会创建这个任务，把它接入项目，并把这个调度器调用加进您的 `main.rs`：

```bash
suprnova make:task CleanupLogs
```

这会：

1. 创建 `src/tasks/cleanup_logs_task.rs`（一个可运行的 `Task` 骨架，会记录自己的耗时）
2. 如果 `src/tasks/mod.rs` 还不存在，就创建它（重新导出 `CleanupLogsTask`）
3. 如果 `src/schedule.rs` 还不存在，就创建它（带一个 `register(&mut Schedule)` 函数）
4. 在 `src/lib.rs` 里声明 `pub mod schedule;` 和 `pub mod tasks;`
5. 把 `.schedule(<crate>::schedule::register)` 加进 `cmd/main.rs`（或者 API 起步下的 `src/main.rs`）里的 `Application` 链上

第 2 到 5 步都是幂等的，所以重新运行 `make:task` 能修复被手动移除的接线。更广泛的 `make:*` 家族请参见[生成器](cli-generators.md)。

生成之后，在 `src/schedule.rs` 里注册这个任务：

```rust
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

这套流式的构造器 API（`.daily()`、`.cron(...)`、`.without_overlapping()`、`.run_in_background()`，还有那些按星期的修饰符）在[任务调度](scheduling.md)里有完整的覆盖。

## 退出码

| 命令 | 以零状态退出 | 以非零状态退出 |
|---|---|---|
| `schedule:run` | 每一个到期的任务都返回了 `Ok(())`，或者没有任何任务到期 | 至少一个任务返回了 `Err(_)`，或者 panic 了 |
| `schedule:work` | 通过 `SIGINT` / `SIGTERM` 干净地关闭（这层包装把退出码 130 当作一次干净的 Ctrl-C） | bootstrap 失败，或者这个守护进程中止了 |
| `schedule:list` | 列出成功（包括那条「没有任务注册」的消息） | 应用启动失败 |

`schedule:work` 内部的后台任务失败，会被记录到 stderr，但不会让这个守护进程退出 - `JoinSet` 的 `catch_unwind` 边界会把它们呈现成 `FrameworkError`，节拍循环则继续下去。

### 为什么 Suprnova 有所不同

Laravel 的 `schedule:run` 是唯一的一等入口点；守护进程形态（`schedule:work`）是给没有 crontab 的主机反向移植回来的。PHP 没有长期存活的进程，所以每一分钟都是一个全新的运行时，必须重新引导框架、容器，以及每一个服务绑定。

在 Suprnova 里，这个守护进程是头等的。`schedule:work` 运行在那个提供 HTTP 服务的同一个 Tokio 运行时内部，所以：

- **后台任务能和这个节拍循环组合。** 一个 `.run_in_background()` 任务会被 spawn 进一个 `JoinSet`；这个循环会在下一次节拍之前轮询已完成的那些，并在关闭时把剩下的排空。Laravel 会为每一个后台任务 spawn 一个子进程。
- **优雅关闭会排空飞行中的工作。** Ctrl-C / SIGTERM 会让内联任务完成它们当前这次调用，并在退出之前等待每一个后台 spawn 完成。Laravel 依赖操作系统去杀掉那个 cron 子进程。
- **启动成本只需要付一次。** 容器、驱动程序，以及您的 `bootstrap_fn`，会在这个守护进程启动时引导，而不是在每一次节拍时。`schedule:run` 仍然是每次调用都要付一次启动成本（它是一个单次触发的子命令），但守护进程这条路径，才是这套运行时模型真正回本的地方。

`schedule:run` 依然能用（当系统 cron 已经是运维人员的权威来源时，它就是正确的选择）。挑哪一个适合您的部署形态就用哪一个 - 两者共享同一套任务定义。

## 下一步

- [任务调度](scheduling.md) - `Task` trait、流式的 cron API、`without_overlapping`、`run_in_background`，以及同分钟去重
- [生成器](cli-generators.md) - 完整的 `make:*` 家族，包括 `make:task`
- [控制台](console.md) - 标注了 `#[command]` 的一次性运维任务（不在计划之内）
- [队列](queues.md) - 面向那些应该由一个工作进程接手、而不是靠时钟节拍触发的工作
- [应用启动](bootstrap.md) - `.schedule(...)` 是如何接入这个构建器的，以及任务能从容器里解析出什么
