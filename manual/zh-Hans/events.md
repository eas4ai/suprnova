# 事件

事件是 Suprnova 类型化的进程内 pub/sub。一个控制器触发
`UserRegistered { user_id }`；一个监听器给这个用户发邮件，另一个写一条审计行，第三个发布一次广播。这三个都会看到同一个载荷，按注册顺序运行，并且在编译期互相都不知道对方的存在。

面向用户的表面是 `EventFacade` 结构体（重新导出为 `suprnova::EventFacade`）。这个 crate 还把 `Event` 这个 *trait* 重新导出为 `suprnova::Event` - 和
Laravel 的门面同名，但在 Rust 里，这个 trait 是每一个载荷都实现的那个类型化契约。门面背后，是单一一个进程全局的 `EventDispatcher`（被持有在一个 `OnceLock` 里）：注册过的监听器，会比注册它们的那次请求活得更久，而分发既可以内联运行，也可以 spawn 进一个有界的、会重试的任务集合。

## 基础

```rust
use suprnova::{EventFacade, Event, Listener, FrameworkError, async_trait};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct UserRegistered {
    pub user_id: i64,
}

impl Event for UserRegistered {
    fn event_name() -> &'static str {
        "UserRegistered"
    }
}

pub struct SendWelcomeEmail;

#[async_trait]
impl Listener<UserRegistered> for SendWelcomeEmail {
    async fn handle(&self, e: &UserRegistered) -> Result<(), FrameworkError> {
        // 发送邮件……
        let _ = e.user_id;
        Ok(())
    }
}

// In bootstrap.rs:
EventFacade::listen::<UserRegistered, SendWelcomeEmail>(Arc::new(SendWelcomeEmail)).await;

// In a controller:
EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
```

`Event` 要求 `Send + Sync + Clone + 'static + Debug`，这样一个载荷才能跨越任务边界（已入队的监听器），分发器也才能记录它的日志。`Listener<E>`
是 `Send + Sync + 'static` 的，这样它才能比这次注册调用活得更久。没有
`#[derive(Event)]` - 这个 trait 只有两个方法（`event_name`，以及有默认实现的 `queued`），所以手写一个 impl 只需要两行。

## 分发模式

| 方法 | 语义 |
|---|---|
| `EventFacade::dispatch(event)` | 同步，快速失败 - 第一个监听器的 `Err` 会中止这条链 |
| `EventFacade::dispatch_best_effort(event)` | 同步，全部运行 - 在每一个监听器都运行完之后，返回第一个 `Err` |
| `EventFacade::dispatch(event)`，当 `Event::queued() = true` 时 | 每个监听器都会作为一个有界的、会重试的任务被 spawn；这次调用会在 spawn 完成之后返回 |

当一个下游副作用必须观察到一个成功的上游时，用 `dispatch`（快速失败） -
大多数模型生命周期钩子落在这里，所以一个否决保存操作的观察者，可以让它短路。当一次扇出中，一个失败的监听器不该压制掉其余的时，用
`dispatch_best_effort` - 大多数可观测性事件落在这里。

覆盖这个 trait 方法，来选择加入已入队的投递：

```rust
impl Event for ExpensiveAuditTrail {
    fn event_name() -> &'static str { "ExpensiveAuditTrail" }
    fn queued() -> bool { true }
}
```

已入队的监听器，受一个进程范围的信号量约束。默认上限是 256 个并发任务；可以逐分发器地用 `EventDispatcher::with_concurrency(n)` 覆盖，或者通过
`EVENT_MAX_CONCURRENCY` 环境变量全局覆盖。每个任务在放弃之前，最多重试
3 次，用 100ms→2s 的带抖动退避 - 这些是进程内针对暂态故障的重试，不是持久队列那种以分钟计的调度。

## 订阅者 - 捆绑相关的注册

当几个监听器都属于同一个功能时，一个 `Subscriber` 会把它们注册为一个单元。镜照的是 Laravel `EventServiceProvider` 的订阅者模式。

```rust
use suprnova::{EventFacade, EventDispatcher, Subscriber, async_trait};
use std::sync::Arc;

pub struct UserEventSubscriber {
    db: Arc<crate::Db>,
}

#[async_trait]
impl Subscriber for UserEventSubscriber {
    async fn subscribe(self: Arc<Self>, d: &EventDispatcher) {
        let db = self.db.clone();
        d.listen::<UserRegistered, _>(Arc::new(SendWelcomeEmail::new(db.clone()))).await;
        d.listen::<UserDeleted, _>(Arc::new(CleanupUserData::new(db.clone()))).await;
        d.listen::<UserPromoted, _>(Arc::new(NotifyAdmins::new(db))).await;
    }
}

// 在 bootstrap.rs 里 - 每个订阅者一行，而不是每个监听器三行：
EventFacade::subscribe(Arc::new(UserEventSubscriber { db: db.clone() })).await;
```

`subscribe` 接受 `Arc<S>`，这样需要和这个订阅者共享状态的监听器，就可以克隆这个 `Arc` 并捕获它。

## 检查与移除监听器

```rust
if EventFacade::has_listeners::<UserRegistered>() {
    EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
}

let removed: usize = EventFacade::forget::<UserRegistered>();
```

`has_listeners::<E>()` 镜照的是 Laravel 的
`Event::hasListeners($eventName)`。`forget::<E>()` 会丢弃为那个事件类型注册的每一个监听器，并返回被移除的数量。生产代码很少需要 `forget` -
监听器的注册通常是启动时一次性的 - 但热替换和测试代码会伸手去拿它。

当这个监听器注册表的锁被污染时，这两个方法都会返回安全的默认值（分别是 `false` 和 `0`），并记录一条 `tracing::error!`，让这次失败可以被观测到。

## Push 与 flush

`push` 会把一个事件捕获进一个逐事件名的桶里，而不触发它。
`flush::<E>()` 会排空这个桶，并按捕获顺序分发其中的一切。镜照的是
Laravel 的 `Event::push` / `Event::flush` 这一对。

```rust
// 在一个分两阶段做事的处理程序内部：
EventFacade::push(UserRegistered { user_id: 42 }).await;
// ……渲染、验证、更多工作……
EventFacade::flush::<UserRegistered>().await?;
```

被 push 过的事件会忽略 `defer` 作用域 - 它们已经被显式地推迟了。
`forget_pushed()` 会丢弃每一个被 push 过的事件，而不分发它们，并返回被丢弃的数量。镜照的是 `Event::forgetPushed()`。

## defer - 在一个回调内部缓冲每一次分发

`defer(only, async { … })` 会带着一个作用域内的任务本地缓冲区，运行这个回调。在这个回调内部发出的每一次 `dispatch` / `dispatch_best_effort`
调用，都会被捕获，并在这个回调返回之后被重放。镜照的是 Laravel 的
`Event::defer($callback, ?$events)`。

```rust
let ((), flush_err) = EventFacade::defer::<_, ()>(None, async {
    do_work_part_one().await?;
    EventFacade::dispatch(WorkStarted).await?; // 已缓冲
    do_work_part_two().await?;
    EventFacade::dispatch(WorkFinished).await?; // 已缓冲
    Ok(())
})
.await?;
// 到这一步，WorkStarted 和 WorkFinished 都已经按顺序触发了。
// `flush_err` 携带着这次重放里的第一个分发错误（如果有的话）。
```

传入 `Some(&["EventOne", "EventTwo"])`，来只推迟这些事件名；其他一切照常内联分发。一个回调错误会让它短路 - 被缓冲的事件会被丢弃，这个错误会传播出去。

这个 defer 缓冲区是逐 Tokio 任务的，所以两个并发的 `defer` 调用，不会踩到对方的状态。

## 已入队的监听器 - 进程内对比持久化

有两个不同的「已入队」层级，命名很重要：

| 需求 | 该伸手去拿 |
|---|---|
| 监听器应该脱离当前任务运行；崩溃时丢失也没关系 | 在这个事件 trait 上设置 `Event::queued() = true` |
| 监听器的工作必须在一次崩溃 + 重启中存活下来 | `QueuedListener<E, J>`（把事件桥接到一个持久化的作业） |

`Event::queued() = true` 会让分发器把每一个监听器都 spawn 成它自己的 Tokio 任务，受一个进程信号量约束，带着有界的重试（3 次尝试，带抖动的退避）。这个工作运行在这个进程上；一次崩溃会丢掉飞行中的监听器。[优雅关闭的排空](#关闭时排空)会等待飞行中的任务，直到一个截止时间。

`QueuedListener<E, J>` 是一个内置的监听器，它会从每一个事件构建一个 [`Job`](queues.md)，并把它推送到持久化的队列上。这个事件仍然同步地触发；这个监听器只是入队 - 这很快 - 所以请求延迟保持很低。这个作业本身会在崩溃中存活下来，因为这个队列是持久化的。

```rust
use suprnova::{EventFacade, QueuedListener};
use std::sync::Arc;

EventFacade::listen::<UserRegistered, _>(Arc::new(
    QueuedListener::<UserRegistered, SendWelcomeEmailJob>::new(|e| SendWelcomeEmailJob {
        user_id: e.user_id,
    }),
))
.await;
```

`QueuedListener` 只需要这个事件是一个常规的同步事件 - 持久性活在队列里，不活在分发器里。

### 给一个已入队的监听器做防抖

一个 `QueuedListener` 会汇入 `Queue::push`，所以只要它的**作业**声明了 `Job::debounce_for`，这个监听器立刻就被防抖了 - 不需要额外接线，而 `Job::debounce_id` 会给您一个逐实体的窗口。

当这个窗口属于这次注册、而不属于这个作业时，请用 `DebouncedListener`，并从事件派生出那个键：

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::events::{DebouncedListener, EventFacade};

EventFacade::listen::<OrderUpdated, _>(Arc::new(
    DebouncedListener::<OrderUpdated, ReindexOrder>::new(
        Duration::from_secs(30),
        |e| ReindexOrder { order_id: e.order_id },
    )
    .max_wait(Duration::from_secs(300))
    .keyed_by(|e| e.order_id.to_string()),
))
.await;
```

订单 55 的四个 `OrderUpdated` 事件会入队四个作业，然后只跑一个。完整的契约请参见[队列](queues.md)。
## 关闭时排空

已入队的进程内监听器，会 spawn 进一个由分发器跟踪的 `JoinSet`。服务器的优雅关闭流程，会调用 `EventFacade::drain_queued(timeout)` 来等待它们：

```rust
let still_running = EventFacade::drain_queued(Duration::from_secs(30)).await;
if still_running > 0 {
    tracing::warn!(still_running, "queued listeners abandoned at shutdown");
}
```

这次排空，会返回截止时间到达时仍在运行的数量（`0` = 完全排空）。超过截止时间的落后者会被中止，这样关闭就不会卡住。

## 把事件桥接到广播

`EventFacade::broadcast::<E>(hub)`，用一行接好一次从一个被分发的事件到一个 `BroadcastHub` 的桥接。任何实现了 `Broadcastable` 和 `Event` 的类型，都可以这样被广播；监听器会收到这个类型化的载荷，而这些具名频道上的订阅者，会收到这个广播信封。

```rust
use suprnova::EventFacade;
use std::sync::Arc;

let hub: Arc<dyn suprnova::BroadcastHub> = Arc::new(broadcast_hub);
EventFacade::broadcast::<OrderShipped>(hub).await;

// 任何之后的分发，也会被发布到 OrderShipped::broadcast_on()
// 声明的那些频道上：
EventFacade::dispatch(OrderShipped { order_id: 42, user_id: 99 }).await?;
```

关于频道模型（公开 / 私有 / 呈现）以及 `Broadcastable` trait，请参见
[广播](broadcasting.md)。

## 内置事件

框架会从它自己的子系统里，分发一组固定的事件。您通过注册监听器来选择加入；如果没有注册任何监听器，这些事件就是空操作。

| 子系统 | 事件 | 由谁分发 |
|---|---|---|
| 错误处理 | `ErrorOccurred` | 每一个 5xx 响应（返回的 `FrameworkError` 或恢复的 panic） |
| 认证（守卫） | `Auth\\Attempting`, `Auth\\Authenticated`, `Auth\\Login`, `Auth\\Logout`, `Auth\\Failed` | `StatefulGuard::attempt` / `login` / `logout` / `once` |
| 认证流程 | `EmailVerified`, `PasswordResetLinkSent`, `PasswordResetCompleted`, `AccountLocked`, `AccountUnlocked`, `TwoFactorEnrolled`, `TwoFactorChallenged`, `TwoFactorChallengeFailed`, `TwoFactorDisabled` | `auth_flows::{EmailVerification, PasswordReset, BruteForce, TwoFactor}` |
| 数据库 | `Database\\ConnectionEstablished`, `Database\\QueryExecuted`, `Database\\TransactionBeginning`, `Database\\TransactionCommitted`, `Database\\TransactionRolledBack`, `Database\\DatabaseBusy` | `DbConnection::connect`、`ExecutorChoice` 辅助函数、`DB::transaction` |
| 邮件 | `Suprnova\\Mail\\MessageSending`, `Suprnova\\Mail\\MessageSent` | `MailBuilder::send` 在传输前后 |
| 通知 | `Suprnova::Notifications::Sending`, `Suprnova::Notifications::Sent`, `Suprnova::Notifications::Failed` | 每一次通道投递 |
| 队列（工作进程） | `queue::JobQueueing`、`JobQueued`、`JobProcessing`、`JobProcessed`、`JobAttempted`、`JobExceptionOccurred`、`JobFailed`、`JobReleased`、`JobReleasedAfterException`、`JobTimedOut`、`Looping`、`WorkerStarting`、`WorkerStopping`、`WorkerInterrupted`、`UniqueJobSkipped`、`QueuePaused`、`QueueResumed`、`QueuesPaused`、`QueuesResumed` | `Queue::push` / `Queue::push_unique` / `run_worker` / `Queue::pause` / `resume` / `pause_all` / `resume_all` |
| 功能标志 | `FeatureUpdated`, `FeatureDeleted` | `features::admin` 的 CRUD |
| Eloquent（逐模型） | 16 个生命周期事件 - `Retrieved`、`Saving`、`Saved`、`Creating`、`Created`、`Updating`、`Updated`、`Deleting`、`Deleted`、`Restoring`、`Restored`、`ForceDeleting`、`ForceDeleted`、`Replicating`、`Pruning`、`Pruned` - 在每个模型的 `events::` 子模块下发出 | `#[suprnova::model]` 宏会把这些接入 save/update/delete |

`ErrorOccurred` 是把 5xx 异常发往 Sentry、Datadog、Slack 等等的专用钩子。这次分发是尽力而为的、并且是被 spawn 出来的，所以一个坏掉的 Sentry
监听器，压制不了其余的监听器，响应转换也永远不会被它阻塞。关于完整的
panic 恢复与转换契约，请参见[错误模型](error-model.md)。

模型生命周期事件以快速失败的方式触发：一个返回 `EventResult::Cancel`
（通过 `CancellableListener` trait）的 `Saving` 监听器，会中止这次保存。参见[Eloquent 观察者与生命周期事件](eloquent.md)。

## DB::listen - 观察查询

要做逐查询的可观测性，您可以通过分发器注册一个类型化的
`Listener<QueryExecuted>`，或者更常见地，注册一个镜照 Laravel
`DB::listen(function ($q) { ... })` 签名的 `DB::listen` 回调：

```rust
use suprnova::DB;
use std::sync::Arc;

DB::listen(Arc::new(|q| {
    tracing::debug!(
        sql = %q.sql,
        time_ms = q.time.as_millis(),
        connection = %q.connection_name,
        "query"
    );
}));
```

这个回调会收到一个携带 SQL、绑定值、挂钟时长、连接名、读 / 写分类，以及最终 `Result` 的 `QueryExecuted`（这样失败的查询也是可观测的）。
`QueryExecuted::to_raw_sql()` 会为了日志的方便而内联绑定值 - 这是
debug 格式，**不是** SQL 安全的。

两项关于重入和成本的保证：

- **重入守卫。** 一个自己会发出查询的监听器，不会从那个嵌套查询里再次触发 `QueryExecuted` - 分发器会在一个监听器运行期间设置一个任务本地标志，而执行器会在那个作用域内跳过发出事件。一个把日志写进数据库的监听器，不会因此循环。
- **没人在监听时零开销。** 执行器会在构建这个事件载荷之前，检查一个组合起来的 `query_observation_active()`（任何直接监听器，任何注册过的
  `Listener<QueryExecuted>`，或者启用了查询日志）。当这三者都关闭时，整条发出路径都会被短路掉。

## 测试 - `EventFacade::fake()`

`EventFacade::fake()` 会用一个记录器替换掉这个全局分发器。被分发的事件会进入这份记录，而不会运行监听器。这个伪造实现，会在这个守卫的生命周期内，持有一个进程范围的序列化器，所以用到它的并行 `#[tokio::test]`，会一次只跑一个 - 测试不再需要自己的 `serial_test` 互斥锁。

```rust
use suprnova::events::{
    EventFacade, assert_dispatched, assert_dispatched_once, assert_dispatched_times,
    assert_nothing_dispatched, has_dispatched, dispatched, dispatched_events,
};

#[tokio::test]
async fn registration_dispatches_welcome_event() {
    let _guard = EventFacade::fake();

    register_user("ada@example.com").await.unwrap();

    assert_dispatched_once::<UserRegistered>();
    assert_dispatched::<UserRegistered>(|e| e.email == "ada@example.com");
}
```

| 辅助函数 | 断言的内容 |
|---|---|
| `assert_dispatched::<E>(pred)` | 至少有一个匹配的 `E` 被分发过 |
| `assert_dispatched_once::<E>()` | 恰好有一个 `E` 被分发过 |
| `assert_dispatched_times::<E>(n)` | 恰好有 `n` 个 `E` 被分发过 |
| `assert_not_dispatched::<E>(pred)` | 没有匹配的 `E` 被分发过 |
| `assert_nothing_dispatched()` | 没有任何类型的事件被分发过 |
| `assert_listening::<E, L>()` | 一个监听器 `L` 已经为 `E` 注册过 |
| `has_dispatched::<E>()` | bool：是否记录了任何 `E` |
| `dispatched::<E>(pred)` | 匹配事件的 `Vec<E>` 克隆 |
| `dispatched_count::<E>(pred)` | 匹配事件的数量 |
| `dispatched_events()` | 所有分发的 `HashMap<&'static str, usize>` |

### 选择性的伪造

```rust
// 只伪造这些事件；其他一切照常分发。
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// 伪造除了这些之外的每一个事件。
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

镜照的是 Laravel 的 `Event::fake([…])` 和 `EventFake::except($events)`。

### Mute - 丢弃事件而不记录

`EventFacade::muted(async { … })` 会带着一个设置了的、任务本地的「静默分发器」标志，运行这个回调；在其内部分发的每一个事件，都会被丢弃，既不记录，也不调用监听器。这是 Suprnova 对 Laravel `NullDispatcher` 的对应物，作用域限定在一个回调上。

```rust
EventFacade::muted(async {
    // 没有监听器会触发，没有事件会被记录。
    run_bulk_import().await;
})
.await;
```

和 `fake()` 不同，`muted` **不会**获取这个进程序列化器 - 两个 muted 作用域可以并行运行。

### `assert_listening` - 验证一个监听器已经被接好

用来测试 bootstrap 的接线情况，而不必触发一个事件：

```rust
#[tokio::test]
async fn bootstrap_wires_welcome_listener() {
    let _guard = EventFacade::fake();
    bootstrap::register_listeners().await;
    suprnova::events::assert_listening::<UserRegistered, SendWelcomeEmail>();
}
```

这个伪造实现，是通过分发器的 `listen` 方法来观察注册的，所以这次注册必须发生在这个伪造实现的作用域**内部** - 在 `EventFacade::fake()` 之前注册的监听器，`assert_listening` 是看不到的。

## Laravel 对等参考

每一个在类型化 Rust 里有对应物的 Laravel 13 `Event` 门面和 `EventFake`
方法，都以最接近的匹配名字发布。Laravel 暴露的、不适合类型化 Rust 的方法，会被省略，并附上一句简短的说明。

| Laravel | Suprnova |
|---|---|
| `Event::dispatch($event)` | `EventFacade::dispatch(event).await` |
| `Event::dispatch($event)`（halt 参数） | 用 `dispatch`（在 `Err` 上快速失败） |
| `Event::until($event)` | `dispatch`（类型化的：第一个 `Err` 会停止） |
| `Event::listen($event, $listener)` | `EventFacade::listen::<E, L>(Arc::new(L))` |
| `Event::hasListeners($name)` | `EventFacade::has_listeners::<E>()` |
| `Event::forget($event)` | `EventFacade::forget::<E>()` |
| `Event::push($event)` | `EventFacade::push(event).await` |
| `Event::flush($event)` | `EventFacade::flush::<E>().await` |
| `Event::forgetPushed()` | `EventFacade::forget_pushed().await` |
| `Event::defer($callback, ?$events)` | `EventFacade::defer(only, async {…}).await` |
| `Event::subscribe($subscriber)` | `EventFacade::subscribe(Arc::new(S)).await` |
| `Event::fake()` | `EventFacade::fake()`（守卫） |
| `Event::fake([$names])` | `EventFacade::fake_only(&["…"])` |
| `EventFake::except($names)` | `EventFacade::fake_except(&["…"])` |
| `EventFake::assertDispatched` | `assert_dispatched` |
| `EventFake::assertDispatchedOnce` | `assert_dispatched_once` |
| `EventFake::assertDispatchedTimes` | `assert_dispatched_times` |
| `EventFake::assertNotDispatched` | `assert_not_dispatched` |
| `EventFake::assertNothingDispatched` | `assert_nothing_dispatched` |
| `EventFake::assertListening` | `assert_listening` |
| `EventFake::hasDispatched` | `has_dispatched` |
| `EventFake::dispatched` | `dispatched`（返回 `Vec<E>`） |
| `EventFake::dispatchedEvents` | `dispatched_events`（名字 → 数量的映射） |
| `NullDispatcher` | `EventFacade::muted(async {…}).await` |
| `Event::wildcards`（`User.*` 模式） | 未提供 - 使用类型化的监听器，或者用于逐模型生命周期钩子的 `Observer<M>` trait |
| `Event::subscribe`（字符串订阅者） | 使用类型化的 `Subscriber` trait |
| `DB::listen(function ($q) {…})` | `DB::listen(Arc::new(|q| {…}))` - 形状相同，接受一个 `&QueryExecuted` |

### 为什么 Suprnova 有所不同

Laravel 的分发器依赖 PHP 那种字符串化类型的运行时：事件是作为字符串传递的类名，监听器是通过容器查找的类名，而 `Event::listen('User.*', ...)`
能工作，是因为对类名字符串做通配符匹配，在 PHP 里是讲得通的。在 Rust
里，「这个监听器处理 `User.*`」的对应物，是「这个监听器对 `E: UserEvent`
是泛型的」 - 一个 trait，不是一次字符串匹配。所以 Suprnova 放弃了通配符，选择了类型系统，其结果是，一次破坏性的重构会变成编译错误，而不是运行时的错误路由。

另一处分歧是 `defer`：Laravel 的 defer 依赖每请求一个进程的模型，来限定这个推迟的作用域。Suprnova 在一个进程里服务许多并发请求，所以这个推迟缓冲区是任务本地的。两个并发的 `defer` 调用，各自拿到自己的缓冲区；这些调用不会互相踩到对方，也没有隐藏的全局状态会泄漏。

## 每一部分位于何处

| 部分 | 文件 |
|---|---|
| `Event` trait、`Listener<E>`、`Subscriber` | `framework/src/events/mod.rs` |
| `EventDispatcher`、`EventFacade`（门面结构体） | `framework/src/events/dispatcher.rs` |
| `ErrorOccurred` | `framework/src/events/builtins.rs` |
| `QueuedListener<E, J>` | `framework/src/events/queued_listener.rs` |
| `assert_dispatched*`、`EventFakeGuard`、`muted` | `framework/src/events/testing.rs` |
| 内置的事件载荷 | `framework/src/{database,auth,auth_flows,mail,notifications,queue,features}/events.rs` |
| 逐模型的生命周期事件 | 由宏生成到每个模型的 `events::` 子模块里 |

## 下一步

- [错误模型](error-model.md) - `ErrorOccurred` 与 5xx
  转换路径
- [队列](queues.md) - 持久化作业，那个能容忍崩溃的层级；
  `QueuedListener` 桥接到这里
- [广播](broadcasting.md) - 通过 `EventFacade::broadcast::<E>(hub)`，把被分发的事件接到 WebSocket 频道上
- [Eloquent](eloquent.md) - 模型生命周期事件与
  `Observer<M>` trait
- [数据库](database.md) - `DB::listen` 与
  `Database\\QueryExecuted` 事件
