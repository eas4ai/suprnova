# 总线

总线是 Suprnova **同步**的命令分发器。您定义一个类型化的 `Command`（`{ 输入，Output 类型 }`），在启动时为它注册一个 `Handler`，然后进程里的任何代码都可以调用 `Bus::dispatch(cmd).await`，拿回一个携带处理程序类型化结果的 `Dispatched<T>`。

总线与[`Queue`](queues.md)配对 - 后者是它异步的兄弟。它们是两个刻意分开的门面，而不是一个统一路由的分发器：

| 您想要的是… | 该用 |
|-------------------------------------------------------|----------------|
| 立刻*就*在当前任务里运行这个工作，并拿回结果 | `Bus` |
| 把这个工作推给一个工作进程，失败时重试，持久化 | `Queue` |

由调用者显式选择。Suprnova 不发布一个 `ShouldQueue` 标记 - 在 Tokio 上，两条路径都是非阻塞的，所以显式选择比隐式路由更清晰、也更快。

## 快速上手

从命令到分发，十行代码：

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::bus::command::{Command, Handler};
use suprnova::bus::Bus;
use suprnova::error::FrameworkError;

#[derive(Serialize, Deserialize)]
pub struct ChargeCustomer { pub customer_id: i64, pub cents: i64 }

#[async_trait]
impl Command for ChargeCustomer {
    type Output = String; // 我们拿回来的那个 charge id
    fn command_name() -> &'static str { "ChargeCustomer" }
}

pub struct ChargeCustomerHandler;

#[async_trait]
impl Handler<ChargeCustomer> for ChargeCustomerHandler {
    async fn handle(&self, cmd: ChargeCustomer) -> Result<String, FrameworkError> {
        Ok(format!("charge-{}-{}", cmd.customer_id, cmd.cents))
    }
}

// 在启动时（一次性）：
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);

// 在一个请求处理程序里：
let charge_id = Bus::dispatch(ChargeCustomer { customer_id: 42, cents: 1999 })
    .await?
    .unwrap_executed();
```

## 定义命令

一个 `Command` 是任何带有一个关联的 `Output` 类型和一个唯一的 `command_name()` 的可序列化结构体：

```rust
#[async_trait]
pub trait Command: Serialize + DeserializeOwned + Send + Sync + 'static {
    type Output: Send + 'static;
    fn command_name() -> &'static str;
}
```

`Output` 是处理程序返回的东西。它只需要是 `Send + 'static` - 真正的分发路径会通过 `Box<dyn Any>` 让值保持原生形态，没有 serde 往返。这意味着像 `Bytes`、不透明句柄，或者 `Arc<Mutex<…>>` 这样的非 serde 输出，会以活的值的形式原样回到调用方。`Command` 自身上 `Serialize + DeserializeOwned` 这条约束，是为伪造捕获路径准备的：`Bus::fake()` 会把每一个被分发的命令记录成一个 `serde_json::Value`，这样基于谓词的断言（`assert_dispatched`、`assert_dispatched_times`）就能解码并检视它们。

`command_name()` 应该是一个每个具体的 `Command` 实现里都唯一的、稳定的字符串。它会出现在 `assert_dispatched`/`assert_dispatched_times` 的失败消息里，以及没有已注册处理程序时的错误返回中。

## 注册处理程序

一个 `Handler<C>` 是一个接受命令、返回 `Result<C::Output, FrameworkError>` 的类型化异步函数：

```rust
#[async_trait]
pub trait Handler<C: Command>: Send + Sync + 'static {
    async fn handle(&self, cmd: C) -> Result<C::Output, FrameworkError>;
}
```

在启动时，每种命令类型调用一次 `Bus::register::<C, H>(handler)`。这个注册表是全局的；重新注册同一个 `C` 会覆盖之前的处理程序（测试正是依赖这一点来替换实现），并发出一条 `tracing::warn!`，这样两次启动期服务注册之间的一次重复绑定就能在日志里被看到。

```rust
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);
Bus::register::<RefundCustomer, _>(RefundCustomerHandler);
```

## 分发

`Bus::dispatch::<C>(cmd)` 会在进程内运行已注册的处理程序，并返回一个 `Dispatched<C::Output>` 枚举：

```rust
pub enum Dispatched<T> {
    Executed(T),  // 处理程序运行了，这是结果
    Captured,    // Bus::fake() 是活跃的，处理程序没有运行
}
```

`Dispatched<T>` 有四个辅助方法：

- `.unwrap_executed()` - 返回这个值，在 `Captured` 上 panic
- `.executed() -> Option<T>` - 转换成 `Option`
- `.is_executed()` - 布尔谓词
- `.is_captured()` - 布尔谓词

对于真实模式下的调用点，`.unwrap_executed()` 是符合习惯的形态。

### `Bus::chain` - 顺序执行

`Bus::chain(Vec<C>)` 会一次运行一个命令，在第一个错误上（包括这个错误本身）停止。所有命令必须是同一种类型。返回 `Vec<Result<Dispatched<C::Output>, FrameworkError>>` - 每个被尝试过的命令对应一条记录。

```rust
let results = Bus::chain(vec![
    ChargeCustomer { customer_id: 1, cents: 100 },
    ChargeCustomer { customer_id: 2, cents: 200 },
    ChargeCustomer { customer_id: 3, cents: 300 },
]).await;

// 收集直到第一次失败为止、每一个成功的 charge id：
let charge_ids: Vec<String> = results
    .into_iter()
    .filter_map(|r| r.ok().and_then(|d| d.executed()))
    .collect();
```

`Bus::chain` 在设计上只接受单一类型 - 这个分发器返回的 `Dispatched<C::Output>`，只有在每一个输入共享同一个 `Output` 时才是类型良好的。对于 Laravel 风格的混合类型链（不同的作业类型混在一起，每一步启动下一步），请用 [`Queue::chain`](queues.md) - 队列会把每个作业装进一个类型化的信封里，所以不受同样的约束。

### `Bus::batch` - 并发执行

`Bus::batch(Vec<C>)` 通过 `futures::join_all` 并发运行命令，并按输入顺序收集结果。和 `chain` 一样，只接受单一类型。

```rust
let results = Bus::batch(vec![
    SendWelcomeEmail { user_id: 1 },
    SendWelcomeEmail { user_id: 2 },
    SendWelcomeEmail { user_id: 3 },
]).await;
```

`Bus::batch` 只接受单一类型，原因和 `chain` 一样。对于混合类型、带持久化、带进度回调、生命周期事件和 `BatchRepository` 的批次，请用 [`Queue::batch`](queues.md)。

## 测试

在测试的最前面安装这个伪造实现。`install_fake()` 会在这个守卫的生命周期内持有一个进程级的 `FAKE_SERIAL` 互斥锁，这样两个并行的 `Bus::fake()` 测试就不会互相破坏对方的已捕获存储 - 第二个会阻塞，直到第一个守卫被丢弃。如果同一个二进制文件里的一个兄弟测试调用了真实的 `Bus::dispatch`，您仍然要给这个测试标上 `#[serial]`：一个真实分发的调用方不会获取 `FAKE_SERIAL`，所以没有 `#[serial]` 的话，它可能和一个并行的伪造测试竞态，观察到 `is_active() == true`。`FAKE_SERIAL` 消除了伪造对伪造的风险，`#[serial]` 消除了真实对伪造的风险。

```rust
use serial_test::serial;
use suprnova::bus::Bus;
use suprnova::bus::testing::{
    assert_dispatched,
    assert_dispatched_times,
    assert_not_dispatched,
    assert_nothing_dispatched,
    install_fake,
};

#[tokio::test]
#[serial]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(/* … */).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

这个伪造实现会捕获被分发的命令，而不运行它们的处理程序。此时 `Bus::dispatch` 调用返回的是 `Ok(Dispatched::Captured)`（没有处理程序的输出），而不是 `Executed`。真正的错误 - 编码/解码失败，一个在这个伪造实现安装之前就缺失的已注册处理程序 - 仍然会以 `Err(_)` 的形式出现。

`install_fake()` 返回一个 `BusFakeGuard`。把它丢弃（它是 RAII 的），这个伪造实现就会被清除，`FAKE_SERIAL` 互斥锁也会被释放。典型的写法是在测试最前面写 `let _guard = install_fake();`。

### 断言表面

| 断言 | 断言的是… |
|------------------------------------------------------|------------------------------------------------------------|
| `assert_dispatched::<C>(pred)` | 至少有一个类型为 `C` 且匹配 `pred` 的命令 |
| `assert_not_dispatched::<C>(pred)` | 没有任何类型为 `C` 且匹配 `pred` 的命令 |
| `assert_dispatched_times::<C>(pred, count)` | 恰好有 `count` 个类型为 `C` 且匹配 `pred` 的命令 |
| `assert_nothing_dispatched()` | 在当前活跃的这个伪造实现下，没有分发过任何类型的命令 |

如果没有安装伪造实现，这四个都会带着 `Bus::fake() must be active` panic。类型限定的那几个，在数量不匹配时会带着 `expected … dispatched <command_name> …` panic。`assert_nothing_dispatched` 会带着 `expected no dispatched commands but found <n>` panic。

## 什么时候该用 `Queue` 代替

当您想要以下任何一项时，就伸手去拿 [`Queue`](queues.md)：

- **跨重启的持久性。** 如果驱动程序是 `database` 或 `redis`，一个排队的作业能在进程崩溃后存活下来。
- **带退避的重试。** 队列工作进程会在每次失败时应用 `Job::max_tries` + `Job::backoff`（指数 / 固定 / 序列）。
- **逐作业的超时。** `Job::timeout` + `Job::fail_on_timeout` 会被工作进程循环遵守。
- **延迟执行。** `Queue::later(duration, job)` 或 `Queue::push_later(job, at)`。
- **去重 / 幂等性。** `Job::unique_id` + `Queue::push_unique`，在一个可配置的 TTL 内为重复提交把关。
- **把调用方和工作进程解耦。** 在一个单独的 `cargo run --bin app -- queue:work` 工作进程集群上运行作业。

当您想要以下任何一项时，就伸手去拿 `Bus`：

- **进程内、立刻运行。** 没有跨进程的序列化。
- **把类型化的结果带回给调用方。** `Dispatched<C::Output>` 把处理程序的类型化返回值带到调用点。
- **同步的组合。** 一个把工作拆解成更小的 `Command` 调用、并按顺序读取每一个结果的请求处理程序。

一个典型的应用会同时用到两者：同步的请求路径通过 `Bus` 分发会返回结果的操作，而“即发即忘”式的、需要持久化的工作则通过 `Queue` 推送。

## 下一步

- [队列](queues.md) - 异步的兄弟、驱动程序、工作进程、重试策略、混合类型的链和批次
- [事件](events.md) - pub/sub 分发器（一个事件 → 多个监听器）
- [工作流](workflows.md) - 一条链不够用时，能在重启后存活的、有状态的长时间运行工作
- [测试](testing.md) - `#[suprnova_test]`、容器伪造实现，以及 `Bus::fake()` 用到的那个进程级序列化模式
