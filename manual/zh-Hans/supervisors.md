# 监督程序

监督程序是框架在启动时启动、并在退出时自动重启的一个长期存活的 Tokio 任务。监督程序面向的是“常驻”工作：后台心跳、指标采集器、连接预热器、周期性清理器，或者任何应该永不停止运行的异步循环。它们和[队列工作进程](queues.md)不同 - 后者消费的是队列里一个个离散的 `Job` 条目。监督程序没有作业队列 - 它拥有自己的循环，自行决定何时休眠、等待或行动。

`SupervisorRegistry` 会把每一个已注册的监督程序作为一个分离的 Tokio 任务 spawn 出来，监视每个任务的 `JoinHandle`，并在它退出时 - 无论是返回 `Err`、返回 `Ok`，还是 panic - 依据它的 `RestartPolicy` 重启它。重启之间由一次指数退避隔开，从 100ms 起步，上限 60 秒，这样一个不断崩溃的监督程序就不会陷入自旋循环、淹没日志。

## 快速上手

定义一个监督程序，通过 `inventory::submit!` 注册它，然后在 bootstrap 时调用 `SupervisorRegistry::start_all()`。

**`src/supervisors/heartbeat.rs`:**

```rust
use async_trait::async_trait;
use std::time::Duration;
use suprnova::supervisor::{RestartPolicy, Supervisor};
use suprnova::{FrameworkError, SupervisorEntry};
use tokio_util::sync::CancellationToken;

pub struct LogHeartbeat;

#[async_trait]
impl Supervisor for LogHeartbeat {
    fn name(&self) -> &'static str { "heartbeat" }

    async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(60)) => {
                    tracing::info!("supervisor heartbeat tick");
                }
            }
        }
    }

    fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }
}

// 使用重新导出的 `suprnova::inventory`，这样脚手架出来的应用就不需要
// 把 `inventory` 添加为一个直接依赖。
suprnova::inventory::submit!(SupervisorEntry {
    factory: || Box::new(LogHeartbeat),
});
```

**`src/bootstrap.rs`:**

```rust
use suprnova::supervisor::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

这就是全部的设置。`LogHeartbeat` 这个监督程序会在启动时启动，每 60 秒记录一次日志 - 并且因为 `RestartPolicy::Always` 在 `Ok` 和 `Err` 两种退出上都会重启，一旦这个循环因任何原因退出，它都会被立即重启。

## 重启策略

每个监督程序都通过这个 trait 方法声明自己的 `RestartPolicy`。默认值是 `OnError`。

| 策略 | 何时重启… | 使用场景 |
|--------|-----------------|----------|
| `RestartPolicy::OnError` | `run()` 返回 `Err` 或发生 panic | 应该在成功时运行至完成的任务（例如，一个包装成监督程序的一次性初始化作业）。 |
| `RestartPolicy::Always` | `run()` 返回 `Ok` 或 `Err`，或发生 panic | 真正的守护进程 - 那些永不应该返回的循环。如果这个循环因任何原因退出，那就是一个 bug，重启是正确的响应。 |
| `RestartPolicy::Never` | （从不） | 应该只运行一次、无论结果如何都不重启的一次性任务。 |

```rust
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::OnError }   // 默认
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }    // 守护进程循环
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Never }     // 一次性
```

**该在什么时候选 `Always`，什么时候选 `OnError`。** 一个无限循环的监督程序（`loop { ... }`）应该用 `Always` - 如果这个循环真的返回了 `Ok(())`，说明发生了意料之外的事情，重启才是正确的响应。一个做有限的工作、成功时返回 `Ok` 的监督程序（例如，刷新一次缓存）应该用 `OnError`，这样一次干净的结束就不会触发重启。

**一次性工作用 `Never`。** 对于按计划运行的工作，优先选用[队列工作进程](queues.md)或[计划任务](scheduling.md)。只有当监督程序这个模式恰好适合某个必须在启动时运行一次、此后再也不运行的东西时，才使用 `RestartPolicy::Never`。

## Panic 处理

`run()` 内部的 panic 会被注册表捕获并当作错误来处理 - 一个发生 panic 的监督程序会带着退避被重启，而不会让进程崩溃。注册表会监视每个监督程序的 `JoinHandle`，并通过标准的 Tokio join 机制来检测 panic。

从重启策略的角度看，无论策略是什么，一次 panic 永远都被当作一次 `Err` 退出：

- `OnError` - panic 之后会重启（panic 算作一次错误）。
- `Always` - panic 之后会重启（和其他任何退出一样）。
- `Never` - panic 之后不会重启（和其他任何退出一样）。

在重启退避开始之前，这次 panic 会连同监督程序的名字一起，以 `error!` 级别被记录下来。

## 退避

当一个监督程序退出、并且它的策略要求重启时，注册表会先等待，再 spawn 出替代任务：

| 连续重启次数 | 延迟 |
|---------|-------|
| 第 1 次 | 100ms |
| 第 2 次 | 200ms |
| 第 3 次 | 400ms |
| 第 4 次 | 800ms |
| … | 每次翻倍 |
| 上限 | 60s |

这个退避会在一次健康的运行之后重置。延迟在每一次*连续*重启时都会翻倍，直到 60 秒的上限为止，但一次至少存活了 60 秒（也就是这个上限时长）的运行会被当作健康的：下一次重启会回落到 100ms 的下限，而不会继承此前一轮故障中累积起来的退避。所以一个干净运行了数小时、随后才闪断重启一次的守护进程，会立刻重启，而不是等上它很早之前累积下来的那 60 秒。

这次重置是基于存活状态的，并且是刻意保守的：只有一次*活得比可能的最大退避还久*的运行才算健康。一次在这个门槛之前就退出的运行，会把当前的退避原样带到下一次，所以一个真正处于颠簸状态的监督程序 - 一个运行永远达不到这个门槛的监督程序 - 仍然会一路爬升到 60 秒的上限，并停留在那里。这次重置永远不会掩盖一个正在崩溃循环的监督程序。

这个 60 秒的上限，防止了一个永久损坏的监督程序无限期地休眠，或者在每次重试时都去锤打外部依赖。把它和 `error!` 级别的日志记录结合起来，就能在一个监督程序进入高退避区间时收到告警。

## 优雅关闭

监督程序会收到一个 `CancellationToken`，作为 `run()` 的一个参数。作为 `Server::run` 关闭流程的一部分，框架会在 Ctrl-C / SIGTERM 上取消这个令牌。想要刷新状态、完成飞行中的工作、或者以其他方式干净退出的监督程序，应该对 `cancel.cancelled()` 做一次 `tokio::select!`：

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(60)) => {
                tracing::info!("supervisor heartbeat tick");
            }
        }
    }
}
```

框架会在取消之后给监督程序的 JoinSet 一个 5 秒的宽限窗口来排空它。没有在这个窗口内响应这个令牌的监督程序，会通过 `JoinSet::abort_all` 被中止。这次排空运行在 WebSocket 处理程序排空之后（这样 WS 连接会先清理干净），并在遥测缓冲区刷新之前。

完全忽略这个令牌的监督程序，会一直运行到这个 5 秒的窗口耗尽，然后被强制中止。如果您的监督程序持有需要刷新的资源（打开的文件句柄、飞行中的 HTTP 请求、写了一半的记录），请始终对 `cancel.cancelled()` 做 select，并在返回之前清理干净。

### 嵌入方与集成测试

`Server::run` 会替您调用 `SupervisorRegistry::shutdown(...)`。在 `Server::run` 之外调用 `SupervisorRegistry::start_all()` 的代码（从一个自定义二进制文件里驱动框架的嵌入方，或者直接拉起监督程序的集成测试）也必须在拆卸时调用 `SupervisorRegistry::shutdown(timeout)`，否则监督程序任务会泄漏到测试的生命周期之外：

```rust
use std::time::Duration;
use suprnova::SupervisorRegistry;

// 测试设置
SupervisorRegistry::start_all().await;

// … 练习这个监督程序 …

// 测试拆卸 - 取消这个共享令牌，把 JoinSet 排空到
// `timeout` 为止，然后对剩下的顽固任务 `abort_all`。
SupervisorRegistry::shutdown(Duration::from_secs(1)).await;
```

如果从未调用过 `start_all`，`shutdown` 就是一次空操作，所以从拆卸逻辑里无条件调用它是安全的。

## 可观测性

每一次错误路径上的重启，都会带着结构化字段发出一条 `error!` 级别的日志条目：

- `supervisor` - 来自 `Supervisor::name()`。
- `error` - `run()` 的 `Err` 返回值里的错误消息，或者对于一次被捕获的 panic 是 `"panic: <payload>"`，或者对于一次异常的 join 失败是 `"join error: <detail>"`。
- `backoff_ms` - 距下一次 spawn 的退避延迟，以毫秒计。

Panic 是通过同一条错误日志上报的 - 没有单独的“已 panic”消息：

```
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error=connection refused backoff_ms=400
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error="panic: \"deliberate test panic\"" backoff_ms=800
```

`RestartPolicy::Always` 在返回 `Ok(())` 时会发出一条 `warn!`（不是 `error!`），带着同样的 `supervisor` / `backoff_ms` 字段，消息是 “supervisor returned Ok under Always policy; restarting” - 这对发现那些干净退出了、但本不该退出的守护进程循环很有用。

监督程序不会围绕 `run()` 自动获得一个 tracing span - 注册表只为生命周期（启动、重启）打 span，而不为任务内部打。如果您想为监督程序内部做的工作获得 span 上下文，请自行发出 `info_span!`，或者给您的循环体加上 `instrument`：

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = async {
                let span = tracing::info_span!("heartbeat.tick");
                let _guard = span.enter();
                do_work().await.ok();
                tokio::time::sleep(Duration::from_secs(60)).await;
            } => {}
        }
    }
}
```

### 为什么 Suprnova 有所不同

Laravel 没有直接的对应物。PHP 的每请求一个进程模型，让常驻的进程内守护进程根本不可能存在 - 长期存活的工作必须活在请求生命周期之外，通常是一个由 `supervisord` 管理的、消费队列的工作进程，或者一条 cron 调度的命令。Laravel 的队列工作进程（`php artisan queue:work`）是最接近的类比，但它仍然是一个一次性的 CLI 进程，由一个外部的监督者重启它。

Suprnova 运行在一个单一的、长期存活的进程内的 Tokio 之上。常驻的后台任务作为受监督的 Tokio 任务，自然而然地和 HTTP 服务器并存 - 没有额外的进程边界，没有外部监督者，没有单独的状态 IPC 通道。`Supervisor` trait 就是 `supervisord` 的进程内对应物，被限定在框架自己的任务树里，带着同样的“退出即重启 + 退避”保证。

（Laravel 也有的）`Queue` 工作进程仍然存在 - 参见[队列](queues.md) - 用于离散作业式的工作。监督程序覆盖的是 Laravel 完全推到框架边界之外的“持续节拍”场景。

## v1 范围之外

以下条目是刻意推迟的：

- **监督树（父子关系）。** 这里没有层级结构 - 所有监督程序都是单一 `SupervisorRegistry` 之下的平级对等体。结构化的监督（一个监督程序拥有并重启子监督程序）是编排器的地盘。

- **资源限制（cgroup、内存、CPU）。** 请通过 systemd 的 unit 文件（`MemoryMax=`、`CPUQuota=`）或者 Kubernetes 在 pod 级别的资源请求/限制来施加资源约束。框架不会对单个监督程序任务施加进程内部的资源限制。

- **跨机器的监督。** 监督程序运行在单台机器上的单个进程之内。把监督决策分布到多台机器上，是编排器的地盘（Kubernetes、Nomad、多台主机上的 systemd）。

## 参考

四个主要的类型 - `Supervisor`、`RestartPolicy`、`SupervisorEntry`、`SupervisorRegistry` - 除了在更长的 `suprnova::supervisor::*` 路径下之外，还在 crate 根部被重新导出（`suprnova::Supervisor` 等等）。两个自由访问器函数仍然留在 `suprnova::supervisor::*` 下。

| 符号 | 用途 |
|--------|---------|
| `Supervisor` | 要在您的监督程序结构体上实现的 trait。必需的方法：`name() -> &'static str`、`async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError>`。可选：`restart_policy() -> RestartPolicy`（默认为 `OnError`）。这个 `cancel` 令牌会在进程关闭时被触发；在 5 秒的中止窗口耗尽之前，对 `cancel.cancelled()` 做 select 来干净退出。 |
| `RestartPolicy` | 带有 `OnError`、`Always`、`Never` 三个变体的枚举。控制注册表在什么时候 spawn 出一个替代任务。 |
| `SupervisorEntry` | Inventory 条目。声明 `factory: fn() -> Box<dyn Supervisor>`。通过 `suprnova::inventory::submit!(SupervisorEntry { factory: || Box::new(MySupervisor) })`，每个监督程序提交一条条目。 |
| `SupervisorRegistry::start_all()` | 异步函数。遍历每一个已提交的 `SupervisorEntry` 值，把每个监督程序作为一个分离的 Tokio 任务 spawn 进这个进程级的 JoinSet，并开始监视重启。是幂等的 - 这些进程级的静态变量都是 `OnceLock`。从您 bootstrap 的 `register()` 里调用一次。 |
| `SupervisorRegistry::shutdown(timeout)` | 异步函数。取消这个共享的取消令牌，让每一个正在监视 `cancel.cancelled()` 的监督程序退出，把 JoinSet 排空到 `timeout` 为止，然后对剩下的顽固任务 `abort_all`。`Server::run` 会把这次调用作为其关闭流程的一部分；在 `Server::run` 之外调用 `start_all` 的嵌入方和集成测试，必须自行调用这个函数，以避免任务泄漏。如果从未调用过 `start_all`，这就是一次空操作。 |
| `suprnova::supervisor::supervisor_tasks()` / `supervisor_cancel_token()` | 返回 `Option<&'static …>`、指向底层 JoinSet 和取消令牌的访问器。被 `Server::run` 的关闭流程使用；以 `pub` 的形式公开，这样从一个自定义二进制文件里驱动框架的嵌入方就能接入。应用代码通常不需要用到它们。 |

## 下一步

- [队列](queues.md) - 监督程序与队列工作进程之间的决策，以及离散作业这个替代方案
- [任务调度](scheduling.md) - 面向那些不需要一个长期存活循环的周期性工作
- [工作流](workflows.md) - 面向那些需要持久化恢复能力的、有状态的长时间运行工作
- [广播](broadcasting.md) - 使用同一套关闭流程（排空顺序）
- [请求生命周期](lifecycle.md) - `Server::run` 和这个关闭排空落在哪个位置
