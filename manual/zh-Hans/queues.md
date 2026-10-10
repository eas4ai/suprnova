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
    queues: Vec::new(),
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

`FailoverQueueDriver` 不是第六个后端。它包住上面这些驱动程序的一个有序列表，好让一个连接拒绝掉的推送能往下穿到下一个。参见[故障转移连接](#故障转移连接)。

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

## 故障转移连接

`FailoverQueueDriver` 包住一个有序的连接列表。第一个连接拒绝掉的推送会在下一个上重试，依此类推沿着这个列表往下走，这样一次 Redis 故障就不会把每一次分发都变成一个丢失的作业。

从环境变量配置它：

```bash
QUEUE_DRIVER=failover
QUEUE_FAILOVER_CONNECTIONS=redis,database

# 每一个连接都读它自己的变量，就和它单独作为
# QUEUE_DRIVER 时完全一样。
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_DB_TABLE=jobs
```

或者当这些连接需要环境变量表达不了的运行时配置时，自己把它接起来：

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{
    DatabaseQueueDriver, FailoverQueueDriver, Queue, QueueDriver, RedisQueueDriver,
};
use suprnova::{DB, FrameworkError};

pub async fn register() -> Result<(), FrameworkError> {
    let redis = RedisQueueDriver::connect(
        "redis://127.0.0.1:6379",
        "suprnova-queue",
        "default",
        "consumer-1",
        Duration::from_secs(60),
    )
    .await?;
    let database =
        DatabaseQueueDriver::new(DB::connection()?.inner().clone(), "jobs".to_string())?;

    let failover = FailoverQueueDriver::new(vec![
        ("redis".to_string(), Arc::new(redis) as Arc<dyn QueueDriver>),
        ("database".to_string(), Arc::new(database) as Arc<dyn QueueDriver>),
    ])?;
    Queue::set_driver(Arc::new(failover));
    Ok(())
}
```

每一项上的那个 `String` 是这个连接的标签，会在 `QueueFailedOver` 事件上被报告出来。它不是从驱动程序类型推导出来的，因为两个连接可以跑同一个驱动程序。

当 `QUEUE_DRIVER=failover` 时，`QUEUE_FAILOVER_CONNECTIONS` 是必需的，而且这个列表里不能包含 `failover` 自己。一个点名了并不存在的驱动程序的条目是一个启动错误，而不是 `QUEUE_DRIVER` 对它自己施加的那种“警告并改用内存”的回退：在一条故障转移链里，一个悄悄变成了内存连接的笔误，会把一个易失的后端塞进一份持久的列表里。

### 写操作会故障转移，读操作不会

只有 `push` 和 `bulk_push` 会走这个连接列表。其他每一个操作 - `pop`、`ack`、`nack`、`release`、`settle`、`clear`、那四个计数器和那三个检查列举 - 都走**第一个**连接，绝不走别的。

这种不对称是契约，不是遗漏。一个预留令牌只对签发它的那个驱动程序有意义，所以拿另一个连接去 ack，什么都结算不了，还会把两边都搞坏。计数器和列举遵循同样的规则，好让您检查到的东西，就是这个连接上的工作进程会排空的东西，而不是一个跨后端的、和任何工作进程的视角都对不上的总和。

**跑在故障转移连接上的工作进程只排空主连接。** 那些转移到了后备连接上的作业，需要一个直接针对那个后备连接运行的工作进程：

```bash
# 排空这条故障转移链的主连接。
QUEUE_DRIVER=failover QUEUE_FAILOVER_CONNECTIONS=redis,database ./app queue:work

# 排空那些转移到了数据库上的作业。这个也要跑。
QUEUE_DRIVER=database ./app queue:work
```

Laravel 的文档出于同样的原因带着同样的警告。

这件事会波及到链，但只经由一扇门。工作进程会在一次调用里结算一个作业、并把一条[已排队的链](#已排队的链)的下一环入队，那次调用就是 `settle`，而这个装饰器只把那次调用委托给主连接。所以在一个事务性的主连接（比如数据库驱动程序）上，一个宕机的主连接会让这次结算失败，什么都不会转移出去：工作进程会把预留原样留着，靠可见性过期来重投这个作业。往下穿只发生在主连接回答 `Settled::Unsupported` 的时候 - 内存驱动程序和 Redis 驱动程序就是这么答的 - 因为那时工作进程会像任何一次普通推送那样，通过绑定的驱动程序把下一环推出去，而那次推送会往下穿。那条链剩下的部分接着就要等一个跑在后备连接上的工作进程。没有这样一个工作进程，这条链就停住了 - 那一环是持久的，什么都没丢，但也没有任何东西去跑它。

### `QueueFailedOver` 事件

每一个拒绝了一次推送的连接都会分发 `queue::events::QueueFailedOver { connection, job_name, exception }`，但只在那次把这个连接推*入*失败状态的推送上分发。一个已经被认为正在失败的连接会保持安静，直到后来有一次推送在它上面成功、把它重新武装起来。一次四小时的故障只产生一个事件，而不是每分发一次就来一个，这正是它能被当作告警来用的原因。

`connection` 是那个失败了的连接的标签，不是那个接下了这个作业的连接的标签。

当每一个连接都拒绝一次推送时，这次推送返回最后一个连接的错误。`bulk_push` 会把每一个信封分别推送，所以每一个都各自往下穿：一批被主连接接受了一半的作业绝不会被整批重新推到后备连接上，而且每个信封都保住它被构建时带的那个 `available_at`。一批不是原子的。如果有一个信封被每一个连接都拒绝了，`bulk_push` 会返回那个信封的错误，而在它之前的那些信封已经入队了。

转移不是去重。这个装饰器绝不会对一个已被某个连接接受的信封再试一次，但一个写下了这个信封、*然后*才报告失败的连接，会在下一个连接上产生一个重复项，因为“写下了它但把确认弄丢了”和“根本没接下它”是没法区分的。两份副本携带同一个作业 id。这就是框架的至少一次投递契约，也正是那个在别处让处理程序幂等成为一项要求的契约 - 参见[幂等性是工作进程和您之间的契约](#幂等性是工作进程和您之间的契约)。

### 为什么 Suprnova 有所不同

Laravel 的故障转移连接，是 `config/queue.php` 里的一个 `connections` 数组，通过连接注册表来解析。Suprnova 没有逐连接的驱动程序注册表 - 只有一个驱动程序被绑定在进程范围内 - 所以这些标签来自 `QUEUE_FAILOVER_CONNECTIONS`（或者来自您传给 `FailoverQueueDriver::new` 的那个 `String`），而读操作委托给的是第一个*驱动程序*，而不是一个具名连接。

Laravel 的 `FailoverQueue::bulk` 会逐个循环这些作业，好让每一个的延迟都能存活下来。Suprnova 在任何驱动程序看到之前，就已经把延迟解析到了信封上，所以那个逐信封的循环白得了这一点 - 但这个循环仍然是让一批只落地一半的作业不被重复推送的关键，所以它留着。

## 推送的各种变体

每一个推送变体都接受一个类型化的 `J: Job` 值，并在信封被提交给驱动程序时返回 - 而不是在处理程序运行时返回。

| 方法 | 行为 |
| --- | --- |
| `Queue::push(job)` | 立即入队 |
| `Queue::push_later(job, at)` | 在某个特定的 `DateTime<Utc>` 时可用 |
| `Queue::later(delay, job)` | 在从现在起 `delay` 之后可用 |
| `Queue::push_with(job, overrides)` | 使用逐次推送 `EnvelopeOverrides` 立即入队 |
| `Queue::push_after_commit(job)` | 在外围的 `DB::transaction` 提交时入队 |
| `Queue::later_with(delay, job, overrides)` | 从现在起 `delay` 后可用，并使用逐次推送 `EnvelopeOverrides` |
| `Queue::push_unique(job)` | 在 `J::unique_for` 期间内按 `J::unique_id` 去重；信封被推送时返回 `Ok(true)`，被一个仍然生效的去重键压制时返回 `Ok(false)` |
| `Queue::push_unique_later(job, at)` | 唯一 + 定时 |
| `Queue::later_unique(delay, job)` | 唯一 + 延迟 |
| `Queue::bulk(vec![job1, job2, ...])` | 推送每一个作业（驱动程序可能会走一条原生的批量路径） |

`push_unique` 要求缓存层已经完成启动 - 这把去重锁住在 [`Cache`](cache.md) 里，由 [`Idempotency::commit_on_success`](idempotency.md) 实现。一次失败的推送会释放这个去重键，好让调用方重试；一次成功的推送则会把它持有 `J::unique_for` 秒。这个作业必须重写 `Job::unique_id(&self)` 让它返回 `Some(id)` - 返回 `None` 会得到一个内部错误。

这个布尔值回答的是一个问题 - “这个作业在队列上吗？” - 而它背后还有第三种情形。如果这把去重锁的租约在推送飞行途中丢失了，推送仍然会完成（幂等层从不取消一个可能已经产生了效果的主体），您拿到的仍然是 `Ok(true)`，同时附带一条 `warn` 级别、点名这个作业和它唯一键的日志。作业确实入队了；未被证明的是，没有别人在并发地把同一个作业也入了队。您的处理程序本来就必须容忍重新投递，所以这不需要额外处理 - 但这条日志之所以在那里，是因为一大批这样的日志意味着支撑您那把去重锁的缓存正在吃紧。

### 处理开始即释放唯一性

一把唯一性锁通常会持续整个 `unique_for` 窗口，哪怕作业已经跑完了。当这把锁的存在是为了合并*排队中的*重复项、而不是为了把执行串行化时，请选择加入，让它在处理开始的那一刻就被释放：

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct RebuildSearchIndex {
    index: String,
}

#[async_trait]
impl Job for RebuildSearchIndex {
    fn job_name() -> &'static str { "rebuild-search-index" }
    fn unique_id(&self) -> Option<String> { Some(self.index.clone()) }
    fn unique_until_processing() -> bool { true }
    fn unique_for() -> Duration { Duration::from_secs(3600) }

    async fn handle(self) -> Result<(), FrameworkError> {
        // 一次跑 20 分钟的重建，不会再把第 2 分钟到达的那次
        // 重新分发吞掉了。
        Ok(())
    }
}
```

工作进程会在这个作业的中间件走完之后、处理程序运行之前的那一刻释放这把锁。由此有四个后果：

- 一个被中间件释放回队列的作业会保住它的锁。它还没有开始处理，所以对一个重复项来说什么都没变。
- 一个被中间件以任何其他方式短路掉的作业会交出它的锁，因为它根本就不会去处理了。这涵盖了删除这个作业、把它送进死信，以及在从未调用处理程序的情况下报告它已完成。
- 一个失败的作业会释放它的锁，并且仍然会被重试。这把锁在处理开始的那一刻就没了，所以在这次失败的尝试等完它的退避之前，一个重复项可以入队，于是您就有了同一个唯一 id 的两个信封。这就是这项选择加入所做的取舍。如果一次重试必须继续占住这个位置，请让 `unique_until_processing` 保持关闭，让 `unique_for` 的 TTL 覆盖整条尝试链。
- 这次释放是按所有者范围来的。`push_unique` 会把这把锁的所有者令牌记在信封上，工作进程再用那个令牌来释放，所以一次被重投的尝试永远不可能释放掉一把此后由更新的一次分发获取到的锁。

`unique_until_processing` 需要的和 `push_unique` 需要的是同样两件东西：一个返回 `Some(id)` 的 `unique_id`，以及一个已经完成启动的缓存层。

在 `sync` 驱动程序下，处理程序是在那次取走了锁的 `push_unique` 调用内部内联运行的，所以这个作业释放的是一把名义上仍由它自己的调用方持有的锁。如果那个处理程序运行的时间超过了 `unique_for` 的三分之一，去重租约的续约器会注意到这把锁没了，并记一条租约丢失的警告，而 `push_unique` 还会在上面再叠一条它自己的“无法证明排他性”的警告。这两条在这里都是预期之中的，而不是故障：作业跑了，推送返回 `Ok(true)`，而锁没了是因为这个作业自己把它释放了。

### 为什么 Suprnova 有所不同

Laravel 会在处理程序返回之后，就释放一个*普通*唯一作业的锁。Suprnova 则让那把锁随着 `unique_for` 的 TTL 过期，这在一个工作进程死在作业中途时，能让去重窗口保持诚实：您配置的那个窗口就是您得到的那个窗口，不管处理程序有没有返回过。`unique_until_processing` 在两个框架里的行为是一样的。

Suprnova 还从不强行释放一把唯一性锁。对于一次不携带所有者令牌的首次尝试，Laravel 会回退到强行释放。在 Suprnova 里，唯一那些不带令牌就到达工作进程的信封，是在这个令牌存在之前就已入队的信封，而它们保持 TTL 过期，而不是去冒一个删掉更新那次分发的锁的风险。

### 防抖 - 保住最后一次分发，而不是第一次

`push_unique` 压制掉一个重复项，保住的是**第一次**分发。防抖正好相反：它保住的是**最后一次**。二十个“这个订单变了”的事件汇成一次重新索引，在第二十次之后的一个窗口时长处发生，带着最新的那份载荷。

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct ReindexOrder {
    order_id: u32,
}

#[async_trait]
impl Job for ReindexOrder {
    fn job_name() -> &'static str { "reindex-order" }
    fn debounce_for() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn max_debounce_wait() -> Option<Duration> { Some(Duration::from_secs(300)) }
    fn debounce_id(&self) -> Option<String> { Some(self.order_id.to_string()) }

    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}
```

- `debounce_for` 就是那个窗口：每一次分发都会把它重新武装起来，所以这次运行发生在*最近一次*分发之后 30 秒。
- `max_debounce_wait` 阻止一阵连绵不断的分发把这份工作永远推迟下去。一旦这一阵已经推迟了五分钟，下一次分发就会不带延迟地入队。这个窗口随后重新开始，所以每一阵都是从它自己的第一次分发开始计量它的最大等待时长的。
- `debounce_id` 给这个窗口划定范围。对订单 7 的二十次更新汇成一次运行；对订单 8 的一次更新不受它们影响。省略它，这个作业的每一次分发就共用一个窗口。

每一次分发仍然会入队。合并是在工作进程那边结算的：每一次推送都会覆盖掉一个缓存令牌，而工作进程会丢掉任何令牌已经被一次更新的分发替换掉的信封，把它确认掉，并发出 `JobDebounced`。正是这一点，让活下来的那次运行携带的是最新的载荷，而不是最旧的。如果这个令牌已经过期或者被淘汰了，这个作业就会运行 - 防抖是失败开放的，因为一个丢失的令牌，并不能证明这个窗口归别人所有。

[`sync` 驱动程序](#驱动程序)没有工作进程，所以它会内联地运行每一次分发，什么都不会被合并掉。Laravel 的 sync 驱动程序也是同样的行为。`Queue::bulk` 是在驱动程序层面推送的，同样不会武装任何窗口，所以一个以批量方式推送的防抖作业，每一份副本都会运行。Laravel 的 `Queue::bulk` 出于同样的理由，也跳过了它自己那次防抖锁的获取。

当这个窗口属于调用方时，请改在调用点设置它：

```rust
use suprnova::queue::DebounceOptions;

Queue::push_debounced(
    ReindexOrder { order_id: 7 },
    DebounceOptions::new(Duration::from_secs(30))
        .max_wait(Duration::from_secs(300))
        .id("7"),
)
.await?;
```

一个作业不能同时声明 `debounce_for` 和 `unique_id`：唯一性保住的是一阵分发里的第一次，而防抖保住的是最后一次，所以这次推送会返回一个把两者都点了名的错误。链和批次拒绝一个防抖的作业，理由与此相关 - 一个被取代的链环会被丢掉，那会把这条链的其余部分晾在那里；而一个被丢掉的批次作业，会让这个批次的待办计数永远停在零以上，于是它的回调永远不会触发。

### 使用 `EnvelopeOverrides` 的逐次推送覆盖

`Queue::push_with` 和 `Queue::later_with` 会连同作业接收一个 `EnvelopeOverrides`，供这一次分发使用与作业自身默认值不同的队列、连接、超时或重试行为：

```rust
use std::time::Duration;
use suprnova::queue::{EnvelopeOverrides, Queue};

let overrides = EnvelopeOverrides {
    queue: Some("priority".into()),
    timeout: Some(Duration::from_secs(10)),
    max_tries: Some(1),
    ..Default::default()
};

Queue::push_with(SendWelcomeEmail { user_id: 42 }, overrides.clone()).await?;

// 延迟对应物，映照 `Queue::later` 与 `Queue::push` 的关系。
Queue::later_with(Duration::from_secs(60), SendWelcomeEmail { user_id: 42 }, overrides).await?;
```

每个字段默认都是 `None`，并交由 `Queue::push` 已运行的普通解析处理；此次推送中 `Some` 字段胜过其他所有值，优先于通过 [`Queue::route`](#队列路由) 注册的路由和作业在该字段上自身的 `Job::*` 声明：

| 字段 | 优先于 |
| --- | --- |
| `queue` | `Queue::route`、`Job::queue()` |
| `connection` | `Queue::route`、`Job::connection()` |
| `timeout` | `Job::timeout()` |
| `fail_on_timeout` | `Job::fail_on_timeout()` |
| `max_tries` | `Job::max_tries()` |
| `backoff` | `Job::backoff()` |
| `after_commit` | `Job::after_commit()` |

`EnvelopeOverrides` 是 `Mail::on_queue`/`.on_connection()` 和 `Notify::queue` 的逐通知队列调优共同构建所基于的原语 - 参见[邮件](mail.md#queueing)和[通知](notifications.md)。

### 作业声明的延迟

作业可自行携带默认延迟，无需每个调用点重复 `Queue::later(Duration::from_secs(60), job)`：

```rust
impl Job for SendDigest {
    // ...
    fn delay() -> Option<Duration> { Some(Duration::from_secs(60)) }
}
```

`Queue::push(job)`、`Queue::push_with(job, overrides)`、`Queue::push_unique(job)` 和 `Queue::bulk(vec![job1, job2])` 都会遵守它 - `available_at` 会从 `now` 变为 `now + J::delay()`。`Queue::bulk` 每次调用只解析一次延迟，因为向量中的每个作业共享同一具体 `J`，因而具有相同的 `Job::delay()`。

显式调用点延迟始终优先：`Queue::push_later(job, at)`、`Queue::later(delay, job)`、`Queue::later_with(delay, job, overrides)`、`Queue::push_unique_later(job, at)` 和 `Queue::later_unique(delay, job)` 均逐字使用调用方传入的时间戳或延迟 - 它们都不会查询 `Job::delay()`。当某作业类型的每次分发默认都应延迟时，使用 trait 方法；当仅某一次分发需要延迟、而类型不应另行声明时，使用 `later`/`push_later` 变体之一。

批次和链也不会查询它：`Queue::batch()...add(job)` 与 `Queue::chain()...add(job)?` 都将信封的 `available_at` 设为调用 `add` 时刻，因此一个声明了 `Job::delay()` 的作业作为批次或链的一部分仍会立即分发，即使同一作业的裸 `Queue::push(job)` 会等待。若批次或链中的步骤需要延迟，请用其他显式方式提供 - 例如作业自身的字段，在 `handle()` 中应用。

### 为什么 Suprnova 有所不同

Laravel 的 `$job->delay` 是实例属性，逐次分发设置（`SendDigest::dispatch($user)->delay(60)`），因此同一类的两次分发可携带不同的延迟。这里的 `Job::delay()` 则是类级默认值，和 `Job::queue()` 或 `Job::max_tries()` 一样 - 需要依据自身数据计算延迟的分发使用 `Queue::later`/`push_later`，它们本来就优先于声明的默认值。

### 提交后分发

一个在 [`DB::transaction`](database.md#transactions) 内部推送的作业，正在和那个事务赛跑。另一个进程上的工作进程可能会弹出这个信封、去找那个事务还开着不放的那一行，然后失败 - 或者更糟：事务回滚了，而这个作业针对一份已经不存在的数据跑了起来。

让这个作业选择加入，去等待这次提交：

```rust
use suprnova::{DB, FrameworkError, Job, Queue, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct SendReceipt {
    order_id: i64,
}

#[async_trait]
impl Job for SendReceipt {
    fn job_name() -> &'static str { "send-receipt" }
    fn after_commit() -> bool { true }

    async fn handle(self) -> Result<(), FrameworkError> {
        // 等到这里运行的时候，订单那一行保证已经是持久的了。
        Ok(())
    }
}

DB::transaction(|_tx| {
    Box::pin(async move {
        let order = Order::create(suprnova::attrs! { total: 4999i64 }).await?;
        // 这里什么都不会到达驱动程序。
        Queue::push(SendReceipt { order_id: order.id }).await?;
        Ok::<(), FrameworkError>(())
    })
})
.await?;
// 信封现在才上队列，而且只在现在。
```

三条规则覆盖了每一种情况：

- **在一个事务内部，整个推送都会等待这次提交。** 不只是驱动程序那一次写：信封的构建、`JobQueueing` 事件和 `JobQueued` 事件也全都发生在提交时刻，所以绝不会有监听器被告知一个随后被回滚丢弃掉的作业。
- **一次回滚会把它丢弃。** 这次推送干脆就没发生过。如果它取走过一把唯一性锁，回滚会把那把锁还回去。
- **在事务之外，推送立即发生。** 正是这一点让这项选择加入可以安全地声明在作业类型上：一个分发点不必知道自己所处的那条代码路径是不是事务性的。

一次[保存点](database.md#savepoints)回滚，对登记在它内部的一切来说都算一次回滚。`tx.rollback_to("name")` 会丢弃自 `tx.savepoint("name")` 以来被推迟的那些推送，并且就在那一刻释放它们取走的锁，于是同一个事务里的一次重新分发能再次赢得这个键。在这个保存点之前做出的推送不受影响，而一个您从不回滚的保存点，会把登记在它内部的一切都保留下来。

如果要按单次分发、而不是按作业类型来控制，请用 `EnvelopeOverrides::after_commit`。`Some(true)` 就是 Laravel 的 `afterCommit()`，并带有简写 `Queue::push_after_commit(job)`；`Some(false)` 就是 Laravel 的 `beforeCommit()`，供那一次必须在提交落地之前就对工作进程可见的分发使用：

```rust
use suprnova::queue::{EnvelopeOverrides, Queue};

// 推迟一个类型上并没有选择加入的作业。
Queue::push_after_commit(SendWelcomeEmail { user_id: 42 }).await?;

// 即使作业类型选择了加入，也立即推送。
Queue::push_with(
    SendReceipt { order_id: 7 },
    EnvelopeOverrides { after_commit: Some(false), ..Default::default() },
)
.await?;
```

一次被推迟的 `Queue::push` 会以提交、而不是以推送为基准来重新解析 [`Job::delay()`](#作业声明的延迟)，因为这个延迟的意思是“在分发之后等这么久”，而对一个被推迟的作业来说，分发*就是*提交。一个显式的时间戳是调用方关于某个时刻的意图，所以 `Queue::push_later`、`Queue::later` 和 `Queue::later_with` 会原封不动地把自己那个时间戳带过这次推迟。

`Queue::push_unique` 在推迟时带着一处刻意的不对称：去重锁是立即取走的，所以同一个事务里对同一个唯一 id 的第二次 `push_unique` 仍然会被压制，也仍然报告 `Ok(false)`。等待的只有信封。赢家会报告 `Ok(true)`，哪怕它的推送还悬着，因为这次推送是一定会发生的。一次回滚会按所有者范围释放它取走的那把锁，所以 `unique_for` 窗口绝不会被一次根本没发生过的分发挡住 - 任何其他让提交没能落地的结局也一样，包括一次被拒绝的 `COMMIT`。这项保证唯一的界限就是 TTL 本身：一个开着的时间超过 `unique_for` 的事务，可能会让它的锁过期、并在飞行途中被另一次分发重新取走，所以如果去重要紧，请给 `unique_for` 留出超过您最长事务的余量。`push_unique*` 这一家子不接受 `EnvelopeOverrides`，所以决定一次唯一推送是否推迟的只有 `Job::after_commit()` - 它没有逐次推送的覆盖。

批次和链不会推迟，就像它们不会查询 `Job::delay()` 一样：`Queue::batch()` 和 `Queue::chain()` 会直接构建并推送它们的信封。如果一个批次必须等待一次提交，请把那次 `.dispatch()` 调用包起来，让它在事务返回之后再运行。

已排队的[邮件](mail.md#queueing)和[通知](notifications.md)也不会推迟。它们各自搭在一个共享的作业类型上（`SendMailJob` / `SendNotificationJob`），而 `Mailable` 或 `Notification` 上目前还没有 `ShouldQueueAfterCommit` 的对应物，所以一次在事务内部的 `Mail::queue` 或 `Notify::queue` 调用会立即到达驱动程序。请在事务返回之后再发送它们。

在 `Queue::fake()` 之下，一次推送会被立即记录下来，连同推迟与否一起，所以一个测试不必提交任何东西就能对它断言。这与 Laravel 的 `Bus::fake` 一致，也正是它让一个测试能够一边驱动一个事务性的处理程序，一边就地对它的那些分发做断言。

### 为什么 Suprnova 有所不同

`Queue::bulk` 是单态的 - 每一个元素共享同一个具体的 `J` - 所以它的提交后划分对这次调用来说是全有或全无的。Laravel 会把一个异构数组划分成推迟的和立即的两半；这里没有什么可划分的。

推迟是绑在闭包形式上的。一次在手动 [`DB::begin_transaction`](database.md#manual-form) 内部的推送会**立即**发生，因为手动模式不安装任何环境事务，因此也没有一次提交可以把回调挂上去。在那里推迟，等于排上一个永远不会有人去跑的回调，而一次悄无声息消失掉的分发，比一次发生得太早的分发更糟。当一次分发必须等待提交时，请伸手去拿 `DB::transaction`。

Laravel 还会把一个连接级的 `after_commit` 配置键，当作它优先级链上的最后一层回退来读。Suprnova 到逐次推送的覆盖、再到作业自己的 `Job::after_commit()` 就停下了：这里的队列连接不携带它们自己的分发策略。

## 作业配置

覆盖 `Job` 的关联函数，逐个实现地去调优行为：

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }

    fn delay() -> Option<Duration> { None }                // 默认值：不延迟
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
    fn unique_until_processing() -> bool { true }          // 默认值：false（TTL 就是那个窗口）
    fn middleware() -> Vec<std::sync::Arc<dyn JobMiddleware>> {
        vec![/* 参见下面的“作业中间件” */]
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

1. 传给 `Queue::push_with` / `Queue::later_with` 的逐次推送覆盖（参见
   [使用 `EnvelopeOverrides` 的逐次推送覆盖](#使用-envelopeoverrides-的逐次推送覆盖)）
2. 一条通过 `Queue::route` 注册的路由
3. 这个作业自己的 `Job::queue` / `Job::connection`
4. 驱动程序 / 全局默认值

给一个字段传 `None`，会让那个维度保持不变，所以路由一个作业的连接，不会打扰它已经声明过的那个队列。

目前这两个维度运行在不同的深度上。**队列**是端到端被遵守的 - 盖在信封上，由驱动程序存储，被 `--queue` 过滤。**连接**解析的是 `JobQueueing` / `JobQueued` 生命周期事件上携带的那个连接*名字*，这是监听器和仪表盘看到的东西；一个进程级全局的驱动程序，仍然会接收每一次推送，所以路由一个作业的连接，还不会选中一个不同的驱动程序。现在声明连接，是为了将来逐连接的驱动程序落地时保持向前兼容，而不是现在就有行为上的效果。

然后把一个工作进程专门分给它：

```bash
./app queue:work --queue=billing
./app queue:work --queue=exports,heavy
./app queue:work                       # 和之前一样，排空每一个队列
```

一个没有路由的作业属于 `default`，所以 `--queue=default` 排空的是未路由的工作，而不会把它搁置不管。

### 转发一整个队列

`Queue::route` 是按作业类型定键的。当您想把一个池子的工作通过另一个池子排空时 - 退役一个队列、吸收一批积压、把工作从一个您即将下线的池子上挪走 - 就改成按队列名字给这次重定向定键：

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::forward("default", "high");
Queue::forward_on("exports", "heavy", "redis");   // 只在 `redis` 连接上
```

`forward_on` 里的那个连接是用来把关的，它比较的对象是这个进程的连接名字 - 如果您设置过 `Queue::set_connection_name`，就是它，否则就是驱动程序自己的名字。它不会拿去和这个作业的 `Job::connection`、某条 `Queue::route` 的连接，或者一次逐次推送的 `EnvelopeOverrides` 连接相比：那些点名的是生命周期事件报告的东西，而一个工作进程手上只有进程名字这一个值，能拿来给它的认领列表把关。这次重定向的两半都是按同一个值把关的，所以一次转发永远不可能挪动了推送却不挪动认领。

这次重定向在两侧都生效，这正是它不会把工作搁置不管的原因：

- **在推送这一侧**，这个名字是在路由和作业自己的 `Job::queue` 都说完话之后被改写的，也在一次逐次推送的 `EnvelopeOverrides` 队列（如果您传了的话）之后。
- **在弹出这一侧**，一个以 `--queue=default` 启动的工作进程会去排空 `high`。没有这一半，目的队列就会攒下没有任何工作进程认领的作业。

一个完全不带 `--queue` 启动的工作进程本来就排空所有东西，所以一次转发对它什么都不改变。转发 `default` 会捕获那些没有点名任何队列的作业，因为一个未路由的作业属于 `default`。

一次转发是一次单一的查找，绝不成链。在注册了 `a -> b` 和 `b -> c` 的情况下，一次解析到了 `a` 的推送会落在 `b` 上。因此，在一条已有的 `a -> b` 之上再注册 `b -> a`，是一次说得通的池子对调，而不是一个环：推送到 `a` 的仍然落在 `b` 上，推送到 `b` 的现在落在 `a` 上，而一个以其中任一个名字启动的工作进程会去认领另一个 - 什么都不成链，所以什么都不会被搁置。在更多队列名字之间做一次更长的轮换，解析方式完全相同，一次一跳，各跳互不相干。Laravel 的 `Queue::forward` 同样没有环检测，理由也一样：它的解析器就是这同一次单一查找。把一个队列转发到它自己的名字上是一次恒等映射 - 根本没有重定向 - 这正是您用来让一条已经注册过的转发失效的办法。

只有将来的推送会挪动。已经躺在源队列上的信封会留在那里，而那个过去排空它们的工作进程，现在正在认领目的队列，所以请在转发一个池子之前先把源池子排空。同样的道理也适用于 `queue:retry`：一个失败的作业会被重新入队到它死在的那个队列上。

暂停是在这次重定向之前求值的，按的是这个工作进程启动时所用的那些名字。`Queue::pause(&connection, "default")` 仍然会停住一个以 `--queue=default` 启动的工作进程，哪怕 `default` 正被转发到 `high`。反过来也成立：暂停这次转发的*目的地* - `Queue::pause(&connection, "high")` - 并不会停住一个以 `--queue=default` 启动的工作进程，因为要够到那个工作进程，靠的是它的源名字，而不是那个被改写过的名字。这次转变所引发的 `WorkerQueuePaused` 事件携带的是 `queue: default`，也就是那个配置上的名字，绝不是 `high` - Laravel 给这个事件排的顺序、报告的内容，都和这里一样。

那几个检查调用刻意不跟随转发：`Queue::pending_jobs(Some("default"))` 列出的是字面上在 `default` 上的东西，而不是在 `high` 上的东西，您正是靠这一点才能看见一个刚刚被您转发掉的源队列上遗留下来的积压。Laravel 在那里也会解析这次转发；参见下方的分歧说明。

用 `Queue::forward_for("default")` 把一条注册过的转发读回来，它会在 `queue` 里返回目的地，在 `connection` 里返回那个把关用的连接。

### 为什么 Suprnova 有所不同

Laravel 的 `Queue::route(...)` 接受一个类字符串；Suprnova 把这个作业当作一个类型参数来接受，所以一个被重命名或删除的作业，会是一个编译错误，而不是一条静默地不再匹配的路由。

更大的分歧在于，当一个驱动程序不能过滤时会发生什么。`QueueDriver::pop_from` 会**拒绝**一个它无法遵守的队列过滤器，而不是回退成排空所有东西。一个被告知只排空 `billing` 却悄悄排空了所有队列的工作进程，看起来和一个正常工作的部署毫无区别，直到错的池子消费了错的作业 - 所以这个配置错误，会在第一次轮询时就变得醒目。内存和数据库驱动程序原生支持过滤；一个不支持的驱动程序 - Redis 驱动程序就是一个，因为单个流的消费者组没有逐队列的存储 - 会报错，而不会误导。

`Queue::forward` 把 Laravel 的 `Queue::forward` 里队列到队列的那一半完整地移植了过来，也只移植了那一半。Laravel 的第三个参数可以把一个被转发的队列挪到一个不同的*连接*上，因为它的队列管理器是按连接名字解析驱动程序的。Suprnova 只有一个进程级全局的驱动程序，而一个连接名字只是给生命周期事件贴标签，所以 `Queue::forward_on(from, to, connection)` 把这个连接当作一个**把关的条件**来对待 - 由它决定这次按队列名字的重定向是否生效 - 而绝不当作一个目的地。出于同样的理由，这里的 `to` 是必需的，而 Laravel 的那个是可选的：在 Laravel 里省略 `to` 意味着“只挪动连接”，而那恰恰是 Suprnova 无法遵守的那个维度，所以一次 `forward(from, None)` 会是一个装扮成配置变更的空操作。

Laravel 的那几个检查调用会跟随一次转发，因为 `pendingJobs($queue)` 及其兄弟方法走的是和推送、弹出同一个驱动程序层的 `getQueue()`。Suprnova 的 `Queue::pending_jobs` / `delayed_jobs` / `reserved_jobs` 报告的则是您点名的那个字面上的队列。在只有一个进程级全局驱动程序的情况下，这种字面上的视图是唯一的办法，能让您看见那些留在一个刚刚被您转发走的队列上的信封 - 也就是本节告诉您要先排空的那批积压。想看新的工作正落在哪里，就按名字去问目的队列。

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
| `UniqueJobSkipped` | `push_unique` 在 `unique_for` 窗口内压制了重复项 |
| `JobDebounced` | 工作进程丢掉了一个被更新的防抖分发所取代的信封 |
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
| `QueuePaused` | `Queue::pause` 设置一个队列自身的开关 |
| `QueueResumed` | `Queue::resume` 清除一个队列自身的开关 |
| `QueuesPaused` | `Queue::pause_all` 设置全局开关 |
| `QueuesResumed` | `Queue::resume_all` 清除全局开关 |
| `WorkerQueuePaused` | 一个正在运行的工作进程第一次观察到某个队列处于暂停状态 |
| `WorkerQueueResumed` | 一个正在运行的工作进程看到一个暂停的队列重新变得可认领 |

用普通的 `Event::listen` API 来订阅。这些事件是尽力而为的 - 没有监听器的 `Event::dispatch` 是一次空操作式的 `Ok(())`，所以在没有 `Event::init()` 的部署里，工作进程不会为此付出任何代价。

`UniqueJobSkipped` 是唯一在*推送端*而非工作端触发的事件，也是唯一报告非失败的事件。它携带 `job_name`、`unique_id` 和 `connection` - 去重决策发生在信封存在之前，因此没有要报告的信封 id。推送仍返回 `Ok(false)`；该事件让原本不可见的压制变得可观察。

`QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` 也以相同方式触发 - 来自 `Queue::pause` / `resume` / `pause_all` / `resume_all` 自身，而非工作循环。它们同样不携带信封身份；完整契约参见下文“暂停队列”。

`WorkerQueuePaused` / `WorkerQueueResumed` 是工作进程那一侧的一对，它们才是告诉您*某一个特定的工作进程为什么安静下来了*的那一对。它们在工作循环内部，为每一次状态转变各触发一次，携带这个工作进程正在排空的那个连接，也携带队列名字 - 或者是 `None`，当一个未加过滤的工作进程因为一次全局暂停而空闲、没有队列名字可以报告时。

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

`QueueDriver` trait 为 `size` / `pending_size` / `reserved_size` / `delayed_size` / `clear` 声明了默认实现；`MemoryQueueDriver`、`DatabaseQueueDriver` 和 `RedisQueueDriver` 全都原生实现了它们。

### 检查队列

计数告诉您排了多少东西；有时候您需要看到那些真正的信封 - 一块管理面板、一次调试会话，或者一个“到底是什么卡住了”的问题。`Queue::pending_jobs` / `delayed_jobs` / `reserved_jobs` 会把那些尺寸计数器所计的同一批信息返回给您，形式是一份 `InspectedJob` DTO 的列举：

```rust
use suprnova::queue::{InspectedJob, Queue};

let pending: Vec<InspectedJob> = Queue::pending_jobs(None).await?;
let billing_only: Vec<InspectedJob> = Queue::pending_jobs(Some("billing")).await?;
let delayed = Queue::delayed_jobs(None).await?;
let reserved = Queue::reserved_jobs(None).await?;

for job in &pending {
    println!(
        "{} attempts={} queue={:?} payload={}",
        job.name, job.attempts, job.queue, job.payload
    );
}
```

`InspectedJob` 携带 `id`、`queue`、`name`、`attempts`、`payload` 和 `created_at`。`id` 和 `created_at` 是 `Option`：数据库驱动程序的列举，对于一行 `envelope_json` 解码失败的记录仍然会报告出来 - 以 `id: None` 和 `payload: {"unparseable": true}` 的形式 - 而不是把它丢掉，从而把一个毒丸作业藏起来不让查看的人看见；`Queue::fake()` 的投影从不记录一个独立于 `available_at` 的分发时间戳，所以在那里 `created_at` 永远是 `None`。

在内存驱动程序上，`delayed_size()` 直接读延迟存储的长度，而 `delayed_jobs()` 和 `pending_jobs()` 会先把任何一个 `available_at` 已经过去的条目提升上来。在一个作业到期、和后台回收器下一次 50 毫秒节拍之间那个很窄的窗口里，`delayed_size()` 仍然可能数进一个 `delayed_jobs()` 已经提升进 `pending_jobs()` 的作业 - 这些列举是更当下的那份视图；那里对不上是预期之中的，不是缺陷。

一个可见性超时已经失效的预留，会一直出现在 `reserved_jobs()` 里，直到一次 `pop` 或者后台回收器把它收回。只有这两者会收回，而收回才是消耗一次尝试的动作，所以一次列举调用永远不会改变一个作业的尝试次数，不管您调用它多少次。

#### 为什么 Suprnova 有所不同

- **一个带 `Option<&str>` 的方法，而不是每种列举配一对方法。** Laravel 在 `pendingJobs($queue)` 之外还另发了一个 `allPendingJobs()`；在这里 `queue: None` 把这两者收拢成一次调用。`delayedJobs`/`allDelayedJobs` 和 `reservedJobs`/`allReservedJobs` 也是同样的形态。
- **trait 的默认实现是一个诚实的 `Err`，而不是一个空集合。** Laravel 的 Beanstalkd 和 SQS 驱动程序，即使对一个明明有作业的队列，也会从这些方法返回 `[]` - 这是一种隐瞒式的谎言，一个第三方驱动程序作者可能不知不觉就照抄了。一个还没实现检查的 Suprnova 驱动程序会明说；`sync` 和 `null` 用 `Ok(vec![])` 覆盖它，因为对它们来说，“永远没有什么可列举的”就是字面上的事实，而不是一个未实现的方法。
- **Redis 的 `reserved_jobs` 是逐消费者的。** 这个驱动程序只知道它自己在进程内亲手发出去的那些预留；另一个消费者飞行中的那些条目，只能通过 Redis 自己的 `XPENDING` 看到，而不是通过这个调用。
- **Redis 的 `pending_jobs` 意思是“从未被投递给这个组里的任何消费者”。** 它扫描的是 `XRANGE (<last-delivered-id> +` - 也就是这个组的投递游标（`XINFO GROUPS`）之后的一切 - 而不是整个流，因为 `ack` 只会对一个条目做 `XACK`（这个驱动程序从不对流做 `XDEL`/`XTRIM`），所以一次仅仅排除掉某个消费者内存中那些预留的扫描，会把每一个已确认的作业永远报告成待处理。一个被释放或被 nack 的作业，会以一个高于游标的全新 id 重新发布出来，所以一旦它的重试生效，它就会重新出现。和 `pending_size` 处在同样的“上界”这一档：这个游标只被读一次，所以一次并发的 `pop` 可以在那次读取和那次扫描之间认领掉一个条目。实践中，一个正在运行的消费者的后台预读任务，往往会在推送之后的几毫秒内就认领掉一个新推送的条目，远早于任何应用去调用 `pop` - 所以 `pending_jobs` 反映的多半是那些在没有消费者主动轮询这个流的时候推送进来的工作，而不是“任何还没有人显式弹出过的信封”。

## 工作进程重启信号

`php artisan queue:restart` 翻译过来就是：

```rust
Queue::restart().await?;
```

这个信号以一个毫秒级时间戳的形式，活在 `Cache` 里。工作进程每一轮循环轮询一次，当这个时间戳比它们自己的启动时间更新时，就干净地退出。搭配一个监督者（systemd、Kubernetes、`supervisor` 模块），这样一个全新的工作进程就能接上前一个停下的地方。

## 暂停队列

`php artisan queue:pause` / `queue:resume` 对应于：

```rust
Queue::pause(&connection, "billing").await?;
Queue::resume(&connection, "billing").await?;
Queue::pause_all().await?;
Queue::resume_all().await?;
```

或在 CLI 中：

```bash
./app queue:pause billing
./app queue:pause --all
./app queue:resume billing
./app queue:resume --all      # 别名：queue:continue
```

暂停的工作进程会完成已经弹出的内容 - 暂停从不打断飞行中的作业 - 然后停止认领新工作，直到恢复。`pause_all` / `resume_all` 是全局开关；暂停（或恢复）命名队列仅影响该队列。**`resume_all` 不会清除逐队列暂停** - 单独暂停的队列在全局恢复后仍保持暂停，这与 Laravel 一致。请通过 `Queue::resume(&connection, "billing")` 显式清除。

一个被暂停的工作进程也会把这件事说出来。`queue:work` 会为每一次状态转变打印一行：

```text
  2026-08-25 14:03:11 Queue billing PAUSED
  2026-08-25 14:07:44 Queue billing RESUMED
```

一个不带 `--queue` 启动的工作进程没有队列名字可以报告，所以一次全局暂停打印的是 `All queues PAUSED`。这两行都来自 `WorkerQueuePaused` / `WorkerQueueResumed` 事件，所以您也可以自己去监听它们，把它们路由到您的告警系统所在的地方。

两个信号位于 `Cache` 中，与上面的重启信号并列：

| 键 | 含义 |
| --- | --- |
| `suprnova:queues:paused` | 全局开关，由 `pause_all` 设置 |
| `suprnova:queue:paused:{connection}:{queue}` | 一个队列的开关，由 `pause` 设置 |

用 `Queue::is_paused(&connection, "billing").await?`（任一键已设置即为 true）或 `Queue::paused_queues(&connection, &queues).await?`（`queues` 中当前暂停的项）检查状态。

### 逐队列暂停需要命名 `--queue`

以 `--queue=billing,exports` 启动的工作进程只从这两个队列认领，因此暂停 `billing` 会在暂停持续期间将列表缩小为 `exports`。完全未带 `--queue` 启动的工作进程会排空驱动程序持有的每个队列，无法向它询问“只暂停 `billing`” - `QueueDriver::pop_from` 从不报告有哪些队列名存在，因此没有任何内容可据以检查逐队列暂停键。`pause_all` 仍会完全停止未过滤的工作进程；命名的逐队列暂停只有在您也命名该工作进程的队列时才生效。

### 禁用暂停轮询

设置 `QUEUE_PAUSABLE=false` 后，该进程中的每个工作进程都会完全忽略暂停信号，每次循环不会增加缓存读取成本。`queue:pause`（而非 `queue:resume`）也会拒绝运行并以非零状态退出，因此禁用暂停的操作员会立即知晓，而不是发出悄然无效的暂停。它对应 Laravel 的 `Worker::$pausable`。

### 为什么 Suprnova 有所不同

无法访问的缓存会**失败开放**：无法读取暂停键的工作进程表现为“未暂停”，并继续排空 - 与上面的工作进程重启信号已采用的失败开放契约相同。瞬时缓存中断应使工作进程群降级为“忽略暂停”，而绝不能成为“每个工作进程悄然冻结” - 暂停状态是显式选择加入的信号，它自身不可用不应成为隐藏的终止开关。

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

这个伪造实现的守卫通过一个进程级互斥锁将并行测试串行化；它为每次推送捕获 `(payload, available_at, overrides)`，并在 `Drop` 时清除。除 `push_with`/`later_with` 外，所有入口点的 `overrides` 字段均为 `EnvelopeOverrides::default()` - 有关其断言 `assert_pushed_on_queue`/`assert_pushed_on_connection` 和 `pushed_with_overrides`，参见[模拟](mocking.md#queue---queuetestinginstall_fake)。在伪造模式下，`push_unique` 始终将推送记录为新鲜项 - 未接入驱动程序时去重没有意义。

一次防抖的推送也是同样的行为：这个伪造实现什么都不往缓存里写，所以没有任何窗口被武装起来，记录下来的 `available_at` 也不带防抖延迟。`assert_pushed_later` 会把它看成没有延迟的。这个伪造实现仍然会抓住的，是一个同时声明了 `debounce_for` 和 `unique_id` 的作业 - 无论环境如何，那一对都不可能成立，所以在 `Queue::fake()` 之下这次推送会返回一个错误，和它在生产环境里的行为一模一样。

## 幂等性是工作进程和您之间的契约

由 Redis 支撑的队列驱动程序，没法让 `nack` 成为原子操作 - `XADD` 和 `XACK` 是两个分开的命令。它们之间的一次崩溃，会通过 `XAUTOCLAIM` 重新投递这条消息。内存和数据库驱动程序，在每次尝试的粒度上是精确一次的，但工作进程循环并不区分驱动程序，所以**在一个生产部署里，每一个作业处理程序都必须是幂等的**。

对于典型的命令式作业，把处理程序体包进 [`Idempotency::once`](idempotency.md) 或者 [`Idempotency::commit_on_success`](idempotency.md)，用一个稳定的、逐操作的键来做键（实体 id、调用方提供的请求 id，等等）。当一次重试必须返回*原始*的结果，而不是跳过重新执行时，就用 `Idempotency::remember`，它会记录下这个成功的值，并在之后的投递上重放它。

## 下一步

- [总线](bus.md) - 带类型化结果的同步分发器
- [事件](events.md) - pub/sub 扇出
- [幂等性](idempotency.md) - 处理程序为至少一次投递所遵守的契约
- [缓存](cache.md) - 支撑着 `push_unique`、`WithoutOverlapping`、`RateLimited`
- [模拟和伪造](mocking.md) - 每一个伪造实现的守卫，包括 `Queue::fake`
