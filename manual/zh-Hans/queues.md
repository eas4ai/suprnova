# 队列

`Queue` 门面会把后台工作分发给一个驱动程序，并让一个独立的工作进程把它排空：HTTP 处理程序快速返回，繁重的活儿在幕后运行。当一个请求本来会因为某件可以稍后再做的事而阻塞时 - 发送邮件、打一个 webhook、生成一份报表 - 就该伸手去拿它。当您想要这份工作*立刻*在当前任务里运行、并返回一个类型化的结果时，搭配 [`Bus`](bus.md)；当您想要一个信号扇出给多个监听器时，搭配 [`Events`](events.md)。

## 快速上手

定义一个作业，在启动时注册它一次，推送它：

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use suprnova::{error::FrameworkError, queue::{Job, Queue}};

#[derive(Serialize, Deserialize)]
struct SendWelcomeEmail { user_id: i64 }

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        // …实际发送这封邮件
        Ok(())
    }
}

// 启动时做一次（工作进程和分发进程都需要这个）。
Queue::set_driver(std::sync::Arc::new(suprnova::queue::MemoryQueueDriver::new()));
suprnova::queue::worker::register_job::<SendWelcomeEmail>();

// 从一个处理程序里推送：
Queue::push(SendWelcomeEmail { user_id: 42 }).await?;
```

一个工作进程会持续排空这个已配置的驱动程序，直到被取消为止：

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use suprnova::queue::{Queue, worker::{WorkerConfig, run_worker}};

let driver = Queue::driver()?;
let cfg = WorkerConfig {
    visibility_timeout: Duration::from_secs(60),
    poll_interval: Duration::from_millis(100),
    max_jobs: None,
};
let shutdown = CancellationToken::new();
run_worker(driver, cfg, shutdown).await;
```

在一个脚手架出来的应用里，这个工作进程由这个二进制文件的 `queue:work` 子命令启动 - `cargo run -- queue:work` - 它运行的是和您的 HTTP 服务器一样的 bootstrap，所以在 `bootstrap()` 里注册的观察者和监听器，对来自一个队列处理程序的插入操作，触发方式是完全一样的。

## 驱动程序

框架内置发布了五个驱动程序。通过 `QUEUE_DRIVER` 环境变量配置，或者通过调用 `Queue::set_driver(...)` 以编程方式配置。

| 驱动程序 | 用于 | 优势 |
| --- | --- | --- |
| `MemoryQueueDriver` | 测试、单进程应用 | 用 `tokio::time::DelayQueue` 实现 `available_at`，兼容虚拟时钟 |
| `RedisQueueDriver` | 生产环境扇出 | 消费者组 + `XAUTOCLAIM` + 基于 ZSET 的延迟作业 |
| `DatabaseQueueDriver` | 单数据库应用 | 在 Postgres/MySQL 上用 `FOR UPDATE SKIP LOCKED`，在 SQLite 上用 `BEGIN` 串行化 |
| `SyncQueueDriver` | 开发、CI | 在 `push` 时内联运行这个处理程序，没有工作进程 |
| `NullQueueDriver` | 测试用的包装器 | 丢弃每一次推送，不运行 |

`Queue::bootstrap_from_env()` 会读取 `QUEUE_DRIVER`，接上匹配的驱动程序；`Queue::bootstrap_default()` 总是接上内存驱动程序。服务器的启动路径会替您调用这两者之一 - 大多数应用只需要通过环境变量来配置。

### 环境配置

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# 数据库驱动程序 - DB::init() 必须先运行
QUEUE_DRIVER=database
QUEUE_DB_TABLE=jobs
```

这个数据库驱动程序会在构造时把 `QUEUE_DB_TABLE` 验证成一个 SQL 标识符，所以一个格式错误的环境变量值，会让启动失败，而不会一路走到 SQL 组装那一步。Redis 底层用的是带 `AutoCommit::Disabled` 的 sea-streamer-redis；这个可见性超时是在消费者组构造时就固定下来的，所以逐次弹出时传入的 `visibility_timeout` 参数在 Redis 上会被忽略（这是 Redis Streams 强加的一处已记录在案的、对这个 trait 契约的背离）。

### 为什么 Suprnova 有所不同

Laravel 把每一个可排队的东西都路由经过总线，在分发时区分 `ShouldQueue` 作业。Suprnova 把两者拆开了：`Bus` 用于会返回一个类型化结果的同步工作，`Queue` 用于能在进程崩溃后存活下来的异步工作。PHP 需要这种隐式路由，因为它的每请求一个进程模型，让“晚一点、在另一个进程里做这件事”这种事很难用别的方式建模。Tokio 不需要 - 显式的 `Bus::dispatch` 对 `Queue::push`，更清晰、更快，并且在调用点就把持久性的选择摆出来了。并排对比请参见 [`bus.md`](bus.md)。

## 推送变体

每一种推送变体都接受一个类型化的 `J: Job` 值，并在这个信封被提交给驱动程序时返回 - 不是在处理程序运行时。

| 方法 | 行为 |
| --- | --- |
| `Queue::push(job)` | 立即入队 |
| `Queue::push_later(job, at)` | 在一个具体的 `DateTime<Utc>` 时刻变为可用 |
| `Queue::later(delay, job)` | 从现在起，过了 `delay` 之后变为可用 |
| `Queue::push_unique(job)` | 按 `J::unique_id`，在 `J::unique_for` 之内去重，全新的返回 `Ok(true)`，重复的返回 `Ok(false)` |
| `Queue::push_unique_later(job, at)` | 唯一 + 定时 |
| `Queue::later_unique(delay, job)` | 唯一 + 延迟 |
| `Queue::bulk(vec![job1, job2, ...])` | 推送每一个作业（驱动程序可能会使用一条原生的批量路径） |

`push_unique` 需要这个缓存层已经启动完毕 - 这把去重锁活在 [`Cache`](cache.md) 里，通过 [`Idempotency::commit_on_success`](idempotency.md) 实现。一次失败的推送会释放这个去重键，这样调用方就能重试；一次成功的推送会持有它 `J::unique_for` 秒。这个作业必须覆盖 `Job::unique_id(&self)`，让它返回 `Some(id)` - `None` 会返回一个内部错误。

## 作业配置

覆盖 `Job` 的关联函数，逐个实现地去调优行为：

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }

    fn max_tries() -> u32 { 5 }                            // 默认值：3
    fn timeout() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn fail_on_timeout() -> bool { false }                 // 默认值：false（超时会重试）
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Sequence { secs: vec![5, 15, 60, 300] }
    }
    fn unique_id(&self) -> Option<String> {
        Some(format!("welcome:{}", self.user_id))
    }
    fn unique_for() -> Duration { Duration::from_secs(600) }  // 默认值：5 分钟
    fn middleware() -> Vec<std::sync::Arc<dyn JobMiddleware>> {
        vec![/* 参见下面的"作业中间件" */]
    }
}
```

## 队列路由

默认情况下，每一个作业都进同一个队列，每个工作进程都会排空全部队列。一旦某些作业比另一些更慢或更重要，您就会想要专用的工作进程池：一次长时间运行的导出，不应该排在一千封欢迎邮件后面等着。

一个作业可以声明自己归属于哪里：

```rust
#[async_trait]
impl Job for GenerateExport {
    fn job_name() -> &'static str { "GenerateExport" }
    async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }

    fn queue() -> Option<&'static str> { Some("exports") }
    fn connection() -> Option<&'static str> { None }   // 默认连接
}
```

……而一个运维人员可以集中地覆盖它，不需要碰这个作业：

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::route::<GenerateExport>(None, Some("heavy"));
Queue::route::<SendInvoice>(Some("redis"), Some("billing"));
```

解析按优先级从高到低运行：

1. 一条通过 `Queue::route` 注册的路由
2. 这个作业自己的 `Job::queue` / `Job::connection`
3. 驱动程序 / 全局默认值

给一个字段传 `None`，会让那个维度保持不变，所以路由一个作业的连接，不会打扰它已经声明过的那个队列。

目前这两个维度运行在不同的深度上。**队列**是端到端被遵守的 - 盖在信封上，由驱动程序存储，被 `--queue` 过滤。**连接**解析的是 `JobQueueing` / `JobQueued` 生命周期事件上携带的那个连接*名字*，这是监听器和仪表盘看到的东西；一个进程级全局的驱动程序，仍然会接收每一次推送，所以路由一个作业的连接，还不会选中一个不同的驱动程序。现在声明连接，是为了将来逐连接的驱动程序落地时保持向前兼容，而不是现在就有行为上的效果。

然后把一个工作进程专门分给它：

```bash
./app queue:work --queue=billing
./app queue:work --queue=exports,heavy
./app queue:work                       # 和之前一样，排空每一个队列
```

一个没有路由的作业属于 `default`，所以 `--queue=default` 排空的是未路由的工作，而不会把它搁置不管。

### 为什么 Suprnova 有所不同

Laravel 的 `Queue::route(...)` 接受一个类字符串；Suprnova 把这个作业当作一个类型参数来接受，所以一个被重命名或删除的作业，会是一个编译错误，而不是一条静默地不再匹配的路由。

更大的分歧在于，当一个驱动程序不能过滤时会发生什么。`QueueDriver::pop_from` 会**拒绝**一个它无法遵守的队列过滤器，而不是回退成排空所有东西。一个被告知只排空 `billing` 却悄悄排空了所有队列的工作进程，看起来和一个正常工作的部署毫无区别，直到错的池子消费了错的作业 - 所以这个配置错误，会在第一次轮询时就变得醒目。内存和数据库驱动程序原生支持过滤；一个不支持的驱动程序 - Redis 驱动程序就是一个，因为单个流的消费者组没有逐队列的存储 - 会报错，而不会误导。

### `jobs` 表

`DatabaseQueueDriver` 期望的是这份模式。`queue` 这一列，正是让 `--queue` 过滤变得可能的东西：

```sql
CREATE TABLE jobs (
    id              TEXT PRIMARY KEY,
    job_name        TEXT NOT NULL,
    queue           TEXT NULL,
    envelope_json   TEXT NOT NULL,
    available_at    BIGINT NOT NULL,
    reserved_until  BIGINT NULL,
    reserved_token  TEXT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    created_at      BIGINT NOT NULL
);
CREATE INDEX idx_jobs_available_at ON jobs(available_at);
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

`queue` 是可空的，一个未路由的作业存的是 `NULL`，而不是 `'default'`。这是刻意的：一个由旧版本二进制文件写入的行，和一个由新版本写入的未路由行，是没法区分的，所以在一次滚动升级期间，一个混合版本的集群排空的是同一份工作。

把这一列添加到一张已有的表上是**必须的**，不只是为了过滤：无论这个作业是否被路由，`push` 都会在它的 `INSERT` 里点名 `queue` 这一列，所以一个 0.7.0+ 的二进制文件，针对一张缺少这一列的表，每一次推送都会失败。先运行这次迁移，再滚动升级二进制文件 - 更旧的二进制文件会显式地列出它们自己的列，忽略这个新的，所以这个顺序是安全的：

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

### 退避方案

| 变体 | 行为 |
| --- | --- |
| `Fixed { secs }` | 每次尝试之间恒定的延迟 |
| `Exponential { base_secs, cap_secs, jitter_ratio }` | `min(base * 2^(attempts-1), cap)` × `[1±jitter]` 区间内的一个随机数 |
| `Sequence { secs }` | 每次尝试对应一条记录；耗尽之后，最后一条记录会重复使用 |

默认值是 `Exponential { base_secs: 2, cap_secs: 300, jitter_ratio: 0.25 }` - 2 秒到 5 分钟，带 ±25% 的抖动。

## 作业中间件

框架内置发布了六个中间件，全都对应 `Illuminate\Queue\Middleware\*`：

| 中间件 | 行为 |
| --- | --- |
| `WithoutOverlapping` | 在这段时长内持有一把 `Cache::lock`；发生争用时带延迟释放 |
| `RateLimited` | 在 `RateLimiter` 的预算上把关；直到这个窗口重置为止都释放 |
| `ThrottlesExceptions` | 对连续的*失败*做速率限制，而不是对请求 |
| `Skip::when(cond)` / `Skip::unless(cond)` | 当条件满足时丢弃这个作业 |
| `FailOnException` | 把匹配的错误提升为永久失败（不重试） |
| `SkipIfBatchCancelled` | 如果这个作业所属的批次被取消了，就丢弃这个作业 |

把它们接到这个 `Job` 实现上：

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{JobMiddleware, RateLimited, WithoutOverlapping};

fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
    vec![
        Arc::new(
            WithoutOverlapping::new("user-42")
                .expire_after(Duration::from_secs(120))
        ),
        Arc::new(
            RateLimited::new(10, Duration::from_secs(60))
                .by("send-mail")
        ),
    ]
}
```

`WithoutOverlapping` 和 `RateLimited` 都需要缓存子系统已经启动完毕（在启动时调用 `Cache::init`，或者 `App::bind::<dyn CacheStore>(...)`）。

### 一把释放不掉的锁，不会让这个作业失败

如果 `WithoutOverlapping` 在处理程序运行之后没法释放它的锁 - 缓存后端抖了一下，连接断了 - 它会在 `warn` 级别记录日志，并仍然照旧返回处理程序自己的结果。这把锁随后会在 `expire_after` 上过期失效。

这是刻意的。到这次释放运行的时候，这个处理程序已经提交了它的副作用：行已经写入，邮件已经发出，扣款已经完成。把这次释放失败报告成一次作业失败，会让工作进程重试，把所有这些事再做一遍，这比一个锁键被持有到它的 TTL 结束，是一个更糟的结果。一个真正失败了的处理程序，仍然会报告它自己的失败 - 压下这次释放错误，并不会压下处理程序自己的错误。

### 释放而不消耗尝试次数的契约

中间件返回的是一个 `JobOutcome`，而不是 `Result<()>`。四个变体：

- `JobOutcome::Completed` - 处理程序运行了，确认。
- `JobOutcome::Released { delay }` - 在 `delay` 之后重新入队，**不**递增 `attempts`。被 `WithoutOverlapping`、`RateLimited` 使用。工作进程会把整个操作交给 `QueueDriver::release`，每一个内置驱动程序都会原地重新入队自己存储的那份副本，所以这条消息永远不会同时既被预留又可见，也永远不会两者都不是。尝试次数被保留了下来，工作进程里没有任何算术运算需要某个驱动程序去反驳 - 存储的那份副本，这一轮压根就没有被递增过。
- `JobOutcome::Failed { reason }` - 立即转入死信，持久化到失败作业存储，不重试。
- `JobOutcome::Deleted` - 丢弃这次预留，不转入死信。被 `Skip` 使用。如果这个作业属于一个批次，这个批次的 `pending_jobs` 仍然会递减，这样回调才能触发。

正是这份契约，让“因为这个桶满了而被限流”，在重试记账、指标和生命周期事件里，读起来和“因为处理程序出错而失败”是不一样的。

### 什么算作一次尝试

一个作业离开一个工作进程却没有完成，有两种方式，两种都会消耗一次尝试：

- **处理程序失败了** - 返回了 `Err`，或者 panic 进了框架的边界。工作进程会做否定确认；驱动程序会带着 `attempts + 1` 重新入队。
- **这个工作进程死了** - OOM 杀掉，`abort()`，一次段错误，`docker kill`，或者一个监督者在停止超时时发出的 SIGKILL。没有任何东西被结算；这次预留只是单纯地失效。无论哪个工作进程重新认领了这个作业，都会在那一刻记上这次尝试。

第二种情况，以前是不计费的，这是一个漏洞，不是一种善意：一个可靠地杀死自己工作进程的作业，永远没法耗尽 `max_tries`，所以永远没法被转入死信。只要还有什么东西在不断重启工作进程，它就会杀死每一个认领了它的工作进程，然后一字不差地原样回来，再杀死下一个。

三个内置驱动程序全都会计上这一次，因为切换 `QUEUE_DRIVER` 不应该改变一个毒丸作业能不能被拦下来。`database` 会检测一个已经失效的 `reserved_until`；`memory` 会在收割者把这次预留挪回可见状态时计上它；`redis` 会从 `XPENDING` 里读取这个条目的投递次数，因为一个 Redis 流条目是不可变的，它自己的计数器是唯一的记录。

`JobOutcome::Released` 是刻意的例外 - 参见上面的契约。一个被 `RateLimited` 限流的作业根本没有运行过，所以它不欠账。

**在 Redis 上，重新认领用的是两个时钟。** `--visibility-timeout` 设定一个条目必须处于未确认状态多久，才有资格被重新认领；第二个间隔控制的是一个消费者多久看一次。这个驱动程序把第二个绑在了第一个上，所以一个丢失的作业，会在大约两倍于配置的超时时长之内回来，而不是这个超时加上一个固定的 30 秒。

**这个预算会在处理程序运行之前就被检查，不只是在结算时才检查。** 其他每一个死信决策，都发生在一个处理程序返回之后，这个前提假设了处理程序会返回。一个会杀死自己工作进程的作业，根本走不到那个检查点，所以工作进程也会拒绝分发一个尝试次数已经耗尽的作业 - 会先把它转入死信，然后才轮到它拖垮下一个工作进程。没有这一点，计一次尝试就只会让一个数字往上爬，而这个作业还在不断循环。

**这对您意味着什么。** `attempts` 计的是*投递给一个工作进程的次数*，不是*处理程序失败的次数*。一个因为和这个作业无关的原因而丢失的工作进程 - 一次主机重启，一次由吵闹的邻居引发的 OOM - 也会从这个作业的预算里烧掉一次尝试。Laravel 的行为方式是一样的。带着这一点去设定 `max_tries` 的大小，并且优先选用幂等的处理程序：至少一次投递从来都是这份契约，而这一点只是让重新投递这条路径，老老实实地计数，而不是悄无声息地计数。

## 生命周期事件

工作进程会通过 [`Event`](events.md) 门面，发出 Laravel 形状的生命周期事件。监听器拿到的是这个信封的身份信息（`id`、`job_name`、`attempts`、`max_tries`、`connection`），不是这个类型化的作业实例 - 这个工作进程在 JSON 载荷上是类型抹除的。错误是以一个 `String` 的形式传递的，因为 `FrameworkError` 没有派生 `Clone`。

| 事件 | 什么时候触发 |
| --- | --- |
| `JobQueueing` | 在这个信封到达驱动程序之前 |
| `JobQueued` | 在驱动程序接受之后 |
| `JobProcessing` | 工作进程弹出了它，即将分发 |
| `JobProcessed` | 处理程序返回了 `Ok` |
| `JobAttempted` | 每一次终态结算（成功、失败、超时） |
| `JobExceptionOccurred` | 处理程序返回了 `Err`，将会重试 |
| `JobReleasedAfterException` | 出错后重试的重新入队发生了 |
| `JobReleased` | 由中间件驱动的释放（不是失败） |
| `JobFailed` | 已转入死信 |
| `JobTimedOut` | 超过了单次尝试的超时 |
| `Looping` | 每一次循环迭代（在弹出之前） |
| `WorkerStarting` / `WorkerStopping` | 每个工作进程的生命周期各一次 |
| `WorkerInterrupted` | 观察到了 `Queue::restart()` 信号 |

用普通的 `Event::listen` API 来订阅。这些事件是尽力而为的 - 没有监听器的 `Event::dispatch` 是一次空操作式的 `Ok(())`，所以在没有 `Event::init()` 的部署里，工作进程不会为此付出任何代价。

## 失败作业存储

被转入死信的作业，会落进已配置的 `FailedJobStore` 里：

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryFailedJobStore};

Queue::set_failed_store(Arc::new(MemoryFailedJobStore::new()));

// 在管理工具里：
let store = Queue::failed_store().unwrap();
for record in store.all().await? {
    println!("{} failed: {}", record.job_name, record.exception);
}
store.forget(some_id).await?;
store.flush(None).await?;
```

三种后端：

- `MemoryFailedJobStore` - 进程内的 `Vec`，重启后丢失。
- `DatabaseFailedJobStore` - 通过 SeaORM 持久化到一张 `failed_jobs` 表。
- `NullFailedJobStore` - 丢弃每一条记录。对应 Laravel 的 `NullFailedJobProvider`。

### 当这个存储拒绝一条记录时

如果这个已配置的存储返回一个错误，工作进程会在 `error` 级别记录日志，并**让这次预留保持原样**，而不是去确认它。这个作业会在可见性过期时回来，被重试 - 它不会被静默地丢弃。

这是刻意的。另一种做法 - 照样确认它 - 会丢弃一个已经耗尽了尝试次数、*并且*没能被记录在任何地方的作业，这是不可恢复的。一个不断回来的作业是可以恢复的：修好这个存储，下一次投递就会成功落地。

实际发生的情形是，一个 `DatabaseFailedJobStore` 指向了一张还没迁移的 `failed_jobs` 表。在您完成迁移之前，正在被转入死信的作业，会以每个可见性超时一次重新投递的节奏循环，每一次都记录下这个存储的错误。如果您真的想要失败被丢弃，就配置 `NullFailedJobStore` - 它会成功，所以这个作业会被确认，然后消失。

### 重试

```rust
use uuid::Uuid;

// 单条记录 - 如果这个 id 不在这个存储里，就是 false。
Queue::retry_failed(some_id).await?;

// 批量 - 一个可选的截止点（只重试比 before 更早的记录）。
let count = Queue::retry_all_failed(None).await?;
```

`retry_failed` 会加载这个信封，重置 `attempts`、`available_at` 和 `idempotency_key`，通过已配置的驱动程序推送，然后删除这条失败作业记录。对应的是 `php artisan queue:retry <id>` 加上 `queue:flush` 的语义（每一个被重试的信封都会被推送，*并且*从这个存储里移除）。

### `failed_jobs` 模式

`DatabaseFailedJobStore` 期望的是这张表（由您的迁移管理）：

```sql
CREATE TABLE failed_jobs (
    id              TEXT PRIMARY KEY,
    connection      TEXT NOT NULL,
    queue           TEXT NOT NULL,
    job_name        TEXT NOT NULL,
    envelope_json   TEXT NOT NULL,
    exception       TEXT NOT NULL,
    failed_at       BIGINT NOT NULL
);
CREATE INDEX idx_failed_jobs_failed_at ON failed_jobs(failed_at);
```

传给 `DatabaseFailedJobStore::new` 的 `table` 参数，会在构造时被验证为一个 SQL 标识符。

## 已排队的批次

分发一组带进度追踪和完成回调的作业：

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryBatchRepository, batch::register_callback};

Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));

// 在启动时注册具名的回调。
register_callback(Arc::new(SendSummary));
register_callback(Arc::new(PageOnFail));

let id = Queue::batch()
    .name("import-users")
    .add(ImportUser { id: 1 })
    .add(ImportUser { id: 2 })
    .add(ImportUser { id: 3 })
    .then("send-summary-email")
    .catch("page-on-fail")
    .finally("cleanup-temp-tables")
    .dispatch()
    .await?;

// 之后检查进度：
let repo = Queue::batch_repository().unwrap();
let snap = repo.find(&id).await?.unwrap();
println!("{}/{} jobs done ({}%)", snap.processed_jobs(), snap.total_jobs, snap.progress());
```

每个工作进程都会针对这个批次结算自己的作业，当 `pending_jobs` 抵达零时，这个工作进程就会触发已注册的 `then`/`catch`/`finally` 回调。默认情况下，第一次失败就会取消这个批次；`.allow_failures()` 会让剩下的作业继续进行。

### 持久化的批次

`MemoryBatchRepository` 会在重启后丢失，这会让每一个飞行中的批次都被搁置：它的计数器没了，`pending_jobs` 再也没法抵达零，回调也永远不会触发。请在生产环境里使用 `DatabaseBatchRepository`：

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, DatabaseBatchRepository};

Queue::set_batch_repository(Arc::new(DatabaseBatchRepository::new(db.clone())));
```

两张表，框架不会创建它们 - 请把它们添加到您的迁移里，和 `jobs`、`failed_jobs` 的工作方式一样：

```sql
CREATE TABLE job_batches (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    total_jobs    INTEGER NOT NULL,
    options_json  TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    cancelled_at  INTEGER NULL,
    finished_at   INTEGER NULL
);

CREATE TABLE job_batch_settlements (
    batch_id   TEXT NOT NULL,
    job_id     TEXT NOT NULL,
    failed     INTEGER NOT NULL,
    settled_at INTEGER NOT NULL,
    PRIMARY KEY (batch_id, job_id)
);
```

`DatabaseBatchRepository::with_tables(db, batches, settlements)` 让您自己给它们命名；这两个名字都会在构造时被验证为 SQL 标识符。

请注意 `pending_jobs` 和 `failed_jobs` **不是**什么：它们不是列。它们是在每一次读取时，从结算行里派生出来的 -

```text
pending_jobs = max(0, total_jobs - COUNT(settlements))
failed_jobs  = COUNT(settlements WHERE failed)
```
 -
因为队列是至少一次的，所以同一个作业会在以下情况下结算超过一次：发生了一次重新投递，一次确认被重复了，或者一个工作进程在完成工作和记录它之间死掉了。一个逐次结算递减的计数器，会在每一种情况下都产生偏移，而这个偏移不是表面上的：`pending_jobs` 把关着这些回调，所以一次过早的零，会在批次里其他作业仍在运行时就触发 `then`。有了这些派生出来的计数，以及 `(batch_id, job_id)` 上的主键，一次重复的结算什么都不会插入，也就没有计数器会出错 - 这跨越多个进程都成立，不只是在单个进程内。

### 当一次分发在中途失败时

如果一次 `driver.push` 在 `dispatch()` 进行到一半时失败了，那些已经到达队列的作业是真实存在的，并且已经盖上了这个批次的 id。所以这个批次会被结算，而不是被移除：每一个*没有*被推送的信封，都会被记录为一个失败的作业，这个批次会被取消。

`total_jobs` 仍然计的是您原本要求的数量，`failed_job_ids` 会准确地点名那些从未成功的作业，那些已经入队的会正常结算，而 `SkipIfBatchCancelled` 会丢弃剩下的 - 所以 `pending_jobs` 仍然会抵达零，您的 `catch`/`finally` 回调仍然会运行。如果压根什么都没被推送出去，`dispatch` 会亲自触发它们，因为已经没有工作进程能来做这件事了。无论哪种情况，您都会拿回原始的那个推送错误。

### 批次选项

| 选项 | 构建器方法 | 效果 |
| --- | --- | --- |
| 允许失败 | `.allow_failures()` | 一个作业失败之后仍然继续调度 |
| Then 回调 | `.then(name)` | 在每一个作业都成功时运行 |
| Catch 回调 | `.catch(name)` | 在第一次失败时运行 |
| Finally 回调 | `.finally(name)` | 无论批次以哪种方式结算，之后都运行 |
| 跳过已取消 | 作业上的 `SkipIfBatchCancelled` 中间件 | 批次被取消时丢弃剩下的作业 |

### `BatchCallback` 实现

```rust
use async_trait::async_trait;
use suprnova::queue::{Batch, BatchCallback};
use suprnova::error::FrameworkError;

pub struct SendSummary;

#[async_trait]
impl BatchCallback for SendSummary {
    fn name(&self) -> &'static str { "send-summary-email" }

    async fn handle(&self, batch: Batch, error: Option<String>) -> Result<(), FrameworkError> {
        let subject = match error {
            Some(_) => format!("Batch {} failed", batch.name),
            None    => format!("Batch {} done - {} jobs", batch.name, batch.total_jobs),
        };
        // … 发送邮件
        Ok(())
    }
}
```

用 `batch::register_callback(Arc::new(SendSummary))` 在启动时注册。回调是以 `name()` 为键的 - 这个批次的选项存储的是回调的名字，所以一次进程重启，是靠查找来找到已注册的回调，而不是尝试反序列化一个闭包（Rust 的闭包不能序列化）。

## 已排队的链

一种顺序的工作流：每一个环节，只有在前一个环节的处理程序确认之后才会运行：

```rust
Queue::chain()
    .add(GenerateReport { id: 99 })?
    .add(UploadToBucket { id: 99 })?
    .add(NotifyOwner { id: 99 })?
    .dispatch()
    .await?;
```

第一个信封会立刻被推送；剩下的都搭载在它的 `chain_remaining` 载荷字段上传递。每一次成功的结算，工作进程都会弹出下一个条目并分发它。一次失败会打断这条链 - 后续的环节永远不会被入队。

### 终态结算

结束一个链上的作业意味着两件事：把后继者入队，以及释放刚刚完成的这个作业。作为两个分开的操作，不存在一个安全的顺序。先确认，这两步之间的一次崩溃，会永久性地丢失这条链剩下的部分 - 队列里没有留下任何东西可以用来重试。先推送，同样的崩溃会让这个已完成的作业被重新投递，于是它的处理程序会再运行一次，而它的后继者会被入队两次。

所以工作进程会通过 `QueueDriver::settle(token, follow_ups)`，把两者一次性都交给驱动程序：

| 结果 | 含义 |
| --- | --- |
| `Settled::Atomically` | 后继者已入队，预留已在同一个事务里被丢弃 |
| `Settled::Stale` | 这次预留被另一个消费者重新认领了；**没有**任何东西被入队或者被丢弃 |
| `Settled::Unsupported` | 这个驱动程序不能以事务方式结算 |

`DatabaseQueueDriver` 实现了它：两个效果是同一个事务，以预留为键的那个 `DELETE`，同时也充当一道栅栏。如果您的可见性超时在处理程序运行期间过期了，另一个工作进程把这个作业接了过去，这个 delete 就什么都匹配不上，这个事务会回滚，您会得到 `Stale` - 什么都没有入队。两步式的结算根本没法表达这种情况：您的推送成功了，新主人的推送也成功了，于是这条链分叉了。

Redis 和内存驱动程序会回答 `Unsupported`，并保持先推送再确认的顺序，用一次至少一次的重复，换掉永久性的丢失。这是框架记录在案的契约，也正是为什么链上的信封 id，是从它的前驱者派生出来的，而不是随机的 - 一个被重新投递的步骤，会重新推送它之前推送过的那个 id，所以这个重复能被识别成同一个逻辑步骤。

如果您编写的驱动程序，它的后续写入和确认共享同一个事务域，就实现 `settle`。它的默认实现返回 `Unsupported`，所以在这个机制出现之前写的驱动程序，能保持不变地继续工作。

## 内省

```rust
Queue::size().await?;            // 总数
Queue::pending_size().await?;    // available_at <= now，尚未被预留
Queue::delayed_size().await?;    // available_at > now
Queue::reserved_size().await?;   // 当前已弹出，尚未确认
Queue::clear().await?;           // 丢弃每一个信封，返回这个数量
Queue::driver_name()?;           // 已配置的驱动程序名字，用于日志/管理
```

`QueueDriver` trait 为 `size` / `pending_size` / `reserved_size` / `delayed_size` / `clear` 声明了默认实现；`MemoryQueueDriver` 和 `DatabaseQueueDriver` 原生实现了它们。`RedisQueueDriver` 对 `size` / `clear` 会返回一个“unsupported”错误 - 这些请使用管理用的 redis-cli。

## 工作进程重启信号

`php artisan queue:restart` 翻译过来就是：

```rust
Queue::restart().await?;
```

这个信号以一个毫秒级时间戳的形式，活在 `Cache` 里。工作进程每一轮循环轮询一次，当这个时间戳比它们自己的启动时间更新时，就干净地退出。搭配一个监督者（systemd、Kubernetes、`supervisor` 模块），这样一个全新的工作进程就能接上前一个停下的地方。

## 优雅关闭

工作进程的 `CancellationToken` 会在下一个弹出边界上触发，永远不会在分发的中途。一个已经被弹出的处理程序，会在工作进程退出之前运行至完成（如果设置了它自己的 `Job::timeout()`，就受它约束）。这意味着飞行中的副作用不会在半路被扯断，但一次 SIGTERM 可能需要等到单个作业的超时时长，才能排空完成。给长期存活的工作进程设置 `WorkerConfig::max_jobs`，实现一种周期性重启策略；工作进程会在那么多次结算之后干净地退出，无论结果如何。

## 结算指标

工作进程会在每一次确认/否定确认失败时，通过 [`Metrics`](observability.md) 发出一个 `queue.settlement.failures` 计数器。属性：`operation`（`"ack"` | `"nack"`）、`driver`（已配置驱动程序的名字）、`job`（这个 job_name）、`outcome`（`"success"`、`"dead_letter"`、`"retry"`、`"deleted"`、`"timeout_dead_letter"`、`"timeout_retry"`、`"released"`）。

这里出现非零的速率，意味着至少一次投递，可能会重新投递一次已经成功的副作用，或者丢失尝试记账 - 请针对它显式地设置告警。

## 类型化的错误

`MaxAttemptsExceeded`、`TimeoutExceeded` 和 `ManuallyFailed`，对应的是 Laravel 的 `MaxAttemptsExceededException` / `TimeoutExceededException` / `ManuallyFailedException`。工作进程会把相关的原因附加到转入死信的那个 `JobFailed` 事件上，这样监听器就能做模式匹配，而不必对错误消息做子字符串搜索。

## 连接命名

工作进程会给每一个生命周期事件都打上一个连接名字。默认情况下，这就是这个驱动程序的 `name()`（例如 `"memory"`、`"redis"`、`"database"`）。同时运行多个连接的应用可以覆盖它：

```rust
Queue::set_connection_name("orders-redis");
```

## 测试

`Queue::fake()` 的语义活在 `queue::testing` 里：

```rust
let _guard = suprnova::queue::testing::install_fake();
my_code_that_dispatches_jobs().await;

suprnova::queue::testing::assert_pushed::<SendWelcomeEmail>(|j| j.user_id == 42);

// 对于延迟的分发，把这个计划好的时间戳固定下来：
suprnova::queue::testing::assert_pushed_later::<SendWelcomeEmail>(|j, at| {
    j.user_id == 42 && at > chrono::Utc::now()
});
```

这个伪造实现的守卫，通过一个进程级的互斥锁，把并行的测试串行化；它会为每一次推送捕获 `(payload, available_at)`，并在 `Drop` 时清除。在伪造模式下，`push_unique` 总是把这次推送记录为全新的 - 当没有接上任何驱动程序时，去重是无关紧要的。

## 幂等性是工作进程和您之间的契约

由 Redis 支撑的队列驱动程序，没法让 `nack` 成为原子操作 - `XADD` 和 `XACK` 是两个分开的命令。它们之间的一次崩溃，会通过 `XAUTOCLAIM` 重新投递这条消息。内存和数据库驱动程序，在每次尝试的粒度上是精确一次的，但工作进程循环并不区分驱动程序，所以**在一个生产部署里，每一个作业处理程序都必须是幂等的**。

对于典型的命令式作业，把处理程序体包进 [`Idempotency::once`](idempotency.md) 或者 [`Idempotency::commit_on_success`](idempotency.md)，用一个稳定的、逐操作的键来做键（实体 id、调用方提供的请求 id，等等）。当一次重试必须返回*原始*的结果，而不是跳过重新执行时，就用 `Idempotency::remember`，它会记录下这个成功的值，并在之后的投递上重放它。

## 下一步

- [总线](bus.md) - 带类型化结果的同步分发器
- [事件](events.md) - pub/sub 扇出
- [幂等性](idempotency.md) - 处理程序为至少一次投递所遵守的契约
- [缓存](cache.md) - 支撑着 `push_unique`、`WithoutOverlapping`、`RateLimited`
- [模拟和伪造](mocking.md) - 每一个伪造实现的守卫，包括 `Queue::fake`
