# 幂等性

当一个客户端重试一次 POST 时，您希望第二次调用是安全的。网络是不可靠的，客户端会重试 - 但 `POST /charges` 绝不应该给同一张卡扣两次款，`POST /orders` 也绝不应该为一次点击产生两个订单。幂等键就是那份契约，它说的是“如果您再次看到同一个键，把原来的答案给我；不要重做这份工作。”

Suprnova 的 `Idempotency` 是 `Cache::lock` 之上的一层薄门面，它给您三种逐级升级的保证：只去重、失败可重试的去重，以及 Stripe 风格的结果重放。这三者都会在函数体运行期间，一直让这把锁的租约保持存活，所以一个缓慢的函数体永远不会让这把锁过期，也永远不会让一个重复请求溜过去。

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome: Idempotent<OrderId> = Idempotency::once(
    "create-order:user-42:client-key-abc",
    Duration::from_secs(86_400),
    || async {
        // 在这个 24 小时窗口内，针对每一个键精确地运行一次。
        place_order(&user, &cart).await
    },
)
.await?;

match outcome {
    Idempotent::Fresh(id) => /* 第一次调用 - id 是这个新订单 */ {},
    Idempotent::FreshUnfenced(id) => {
        // 这个订单已经被下单了，但这把锁的租约在执行到一半时
        // 丢失了，所以另一个调用方可能也下了一个。请协调
        // 或者告警 - 见下文的“独占性丢失时”。
    },
    Idempotent::Duplicate => /* 同一个键已经被用过了 */ {},
}
```

## 三种基础原语

| 方法 | 函数体何时运行 | 重复请求看到什么 | 失败会释放锁吗？ | 何时使用 |
|---|---|---|---|---|
| `Idempotency::once` | 每个窗口精确地运行一次 | `Duplicate` 标记 | 否 | 副作用绝不能重复（邮件已发送、扣款已尝试） |
| `Idempotency::commit_on_success` | 每个窗口每次成功运行一次 | `Duplicate` 标记 | 是 | 瞬态失败应该可以重试，但一次成功要保持住 |
| `Idempotency::remember` | 每个窗口每次成功运行一次 | 原始的返回值 | 是 | 重复请求必须收到原始的载荷，而不是一个标记 |

这三者都活在 `suprnova::idempotency` 之下，并从 crate 根重新导出为 `Idempotency`、`Idempotent` 和 `Replay`。它们共享同样的键哈希、租约续约和锁语义 - 只有成功/失败策略不同。

### `Idempotency::once` - 至多一次

这是最严格的契约。TTL 窗口里的第一个调用方会运行这个函数体，并得到 `Fresh(value)`。窗口内的每一个后续调用方都会得到 `Duplicate`，函数体**不会**再次运行 - 即便第一个调用方的函数体返回了 `Err` 也一样。TTL 本身就是这个去重窗口。

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

// 每次注册精确地发送一封欢迎邮件，不管注册回调
// 重试了多少次。
let result = Idempotency::once(
    &format!("welcome-mail:{}", user.id),
    Duration::from_secs(7 * 24 * 3600),
    || async {
        Mail::to(&user.email).send(WelcomeMail { user: user.clone() }).await
    },
)
.await?;
```

当这个副作用属于“我已经尝试过了；哪怕我在这个副作用之后出了错，也不要再试一次”这一类时，就用 `once` - 比如发送一封邮件、向一个不遵守自己那套幂等键的外部 API 发起请求、写入一条重复写入会污染下游分析的审计日志条目。

### `Idempotency::commit_on_success` - 成功时至少一次，失败可重试

和 `once` 类似，但如果函数体返回 `Err`，这把去重锁就会被释放，这样 TTL 窗口内的下一个调用方就能重试。一个成功的函数体，则会在窗口剩下的时间里一直持有这把锁。

```rust
use std::time::Duration;
use suprnova::{Idempotency, Idempotent};

let outcome = Idempotency::commit_on_success(
    &format!("publish-post:{}", post.id),
    Duration::from_secs(300),
    || async {
        // 向一个上游服务发布一条消息。网络错误是
        // 瞬态的 - 下一次重试应该重新进入，而不是在什么都没
        // 真正发生时，被告知“已经完成了”。
        social_media_client.post(&post).await
    },
)
.await?;
```

当函数体带有可重试的失败模式时（瞬态网络错误、上游速率限制、一次刷新就能修好的过期凭据），并且您想要成功时至少一次，但失败时让这把锁让出来以便重试能重新进入，就用 `commit_on_success`。

### `Idempotency::remember` - Stripe 风格的结果重放

这就是 HTTP `Idempotency-Key` 请求头最初被发明出来要解决的那份契约。第一个调用方运行这个函数体，存储这个成功的值，并得到 `Replay::Fresh`。窗口内之后到来的调用方，会得到 `Replay::Replayed(<original value>)` - 也就是记录下来的那个返回值，而不是一个标记。一个在第一个调用方仍在运行*期间*到达的并发调用方，会得到 `Replay::InProgress`。

```rust
use std::time::Duration;
use suprnova::{
    handler, Auth, FrameworkError, HttpResponse, Idempotency, Replay, Request, Response,
};

#[handler]
pub async fn create_charge(req: Request) -> Response {
    // 在为了取这个请求体而消费掉 `req` 之前，先把这个请求头提取成一个拥有所有权的 String。
    let key = req
        .header("Idempotency-Key")
        .ok_or_else(|| FrameworkError::bad_request("Idempotency-Key header required"))?
        .to_string();

    let user = Auth::user_as::<User>()
        .await?
        .ok_or_else(|| FrameworkError::unauthorized("login required"))?;

    let form: ChargeForm = req.json().await?;

    let outcome = Idempotency::remember(
        &format!("charge:{}:{}", user.id, key),
        Duration::from_secs(24 * 3600),
        || async {
            let charge = StripeClient::charge(&form).await?;
            Ok(ChargeResponse {
                id: charge.id,
                amount: charge.amount,
                status: charge.status,
            })
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(body) | Replay::Replayed(body) => {
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::FreshUnfenced(body) => {
            // 给客户端的响应是一样的，但值得记一个指标：独占性
            // 没有在整个函数体期间都被保持住。
            tracing::warn!("idempotent body completed unfenced");
            let json = serde_json::to_value(&body)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

请注意，`Fresh` 和 `Replayed` 在面向客户端的响应里被同等对待 - `remember` 的整个意义就在于，第二个调用方没法分辨自己是运行了这个函数体的那一个，还是拿到了记录下来的结果。

`InProgress` 是那个值得多想一下的情形：一个重复请求在第一个调用方的函数体还在执行时到达，所以还没有记录下来的结果可以交回去。带着 `Retry-After: 1` 请求头的 `409 Conflict`，是那个规范的答案 - 客户端稍作退避，然后重试，而这第二次尝试要么和原始调用竞争那个 `Cache::get` 短路，要么命中 `Replayed`。

## 键材料

所有三个方法，都接受一个任意的 `&str` 作为键。在它触达缓存后端之前，这个键会被 SHA-256 哈希成一个 64 字符的十六进制摘要。这带给您三样东西：

1. **有界的后端键长度。** 一个 POST 了 10 KB `Idempotency-Key` 请求头的客户端，产出的依然是一个 64 字节的缓存键。
2. **原始标识符不会泄漏进缓存工具里。** 如果这个键包含一个邮箱地址、一个会话 id，或者一个内部用户 id，它们都不会出现在 `redis-cli KEYS idem:*` 里。
3. **没有字符类冲突。** 缓存后端会特殊解读的任何东西（冒号、glob 字符、控制字节），都已经没有了 - 这个哈希是纯十六进制的。

这个哈希，哈的是用户提供的那个键，而不是缓存键前缀 - 同一个进程里两个不同调用点的 `Idempotency::once("k", …)` 和 `Idempotency::once("k", …)`，是故意会冲突的。如果您不想要这样，就自己给您的键加上命名空间：

```rust
Idempotency::once(
    &format!("billing:charge:{}:{}", tenant_id, client_key),
    Duration::from_secs(86_400),
    || async { /* … */ },
)
.await?;
```

## 租约续约 - 慢函数体问题

一个天真的锁 + TTL 组合，带着一个窗口 bug：如果函数体运行的时间比 TTL 长，这把锁就会在函数体仍在运行时过期，而第二个调用方就能获取一把全新的锁，并发地再次运行这个函数体。这份去重契约，恰恰会在那些慢到需要它的操作上失效。

Suprnova 解决这个问题的办法，是在函数体运行的整个期间，spawn 出一个后台任务，以 TTL 三分之一的间隔（下限为 50 毫秒）来刷新这把锁。一个带着 `biased` 排序的 `tokio::select!`，保证了函数体这一支，是唯一会去解析这个 future 的那一支。

一次刷新*错误*不会被当作租约丢失。它只意味着没能问到后端，而不是别人拿走了这把锁，所以续约会在下一个间隔重试，只有在连续失败了好几次之后才会放弃。在第一次抖动时就放弃，会确保这个租约失效，即便后端几毫秒之后就恢复了也一样。

### 独占性丢失时

续约依然可能真的失败：这个令牌不再匹配了，因为这把锁过期了，被别人认领走了。在那一刻，可能有两个调用方在运行同一个函数体。

函数体**不会**被取消。到租约丢失的那一刻，它可能已经给一张卡扣了款，或者发出了一条消息，取消它会把这个半途而废的状态搁浅在那里，没有任何东西记录它。函数体会运行到完成，而这次丢失会被报告出来：

| 结果 | 含义 |
|---|---|
| `Fresh(v)` / `Replay::Fresh(v)` | 函数体运行了，独占性自始至终都被保持住了 |
| `FreshUnfenced(v)` | 函数体运行了并产出了 `v`，但另一个调用方可能也并发地运行过 |

`FreshUnfenced` 是一个单独的变体，而不是 `Fresh` 上的一个标志位，这么设计正是为了让一次穷尽式的 `match` 没法不小心忽略它。拿它怎么办，由您自己决定 - 协调、告警、补偿 - 但把它当成 `Fresh` 来处理，会丢掉您能得到的、关于这份保证没有成立的唯一信号。

丢失一个租约，需要后端在好几个刷新间隔里都不可达，或者一次比 TTL 更长的 stop-the-world 式停顿。这很罕见。但它并非不可能，而且它过去是不可见的。

实际的结论是：请根据您的去重窗口来选取 TTL（`一个重复请求应该被去重多久？`），而不是根据您函数体最坏情况下的耗时。一个 30 分钟的函数体配一个 1 分钟的 TTL 是没问题的 - 这把锁会在函数体运行期间被刷新大约九十次。

一个演练这一点的测试：一个 200 毫秒的 TTL，配一个阻塞 500 毫秒的函数体，第二个调用方在 400 毫秒时到达。没有续约的话，第二个调用方会重新执行这个函数体。有续约的话，它看到的是 `Duplicate`。这把锁保持住了。

## 共享后端

跨进程的去重，需要一个跨进程的缓存。内存后端把锁保存在一个逐进程的 `HashMap` 里，所以同一台机器上的两个 `cargo run` 实例，看不到彼此的幂等键。凡是这些情形中任何一个要紧的生产部署 - 多个应用进程、水平扩展、带着重叠流量窗口的蓝绿部署 - 都必须设置 `CACHE_DRIVER=redis`，并提供一个可达的 `REDIS_URL`。

这个启动引导是失败关闭的：如果 `CACHE_DRIVER=redis`，但 Redis 不可达，应用会拒绝启动，而不是静默地降级到逐进程内存。完整的缓存后端契约，请参见 [cache.md](cache.md)。

## 错误处理

函数体的 `FrameworkError` 会原样一路穿过 `Idempotency` 往上传播。一次锁获取失败（请求进行到一半 Redis 挂了、后端返回一个错误），会作为一个来自缓存层的 `FrameworkError` 传播上去 - 没有静默的回退。这个错误类型就是框架标准的 `FrameworkError`，所以处理程序可以用 `?` 把它一路传给它们控制器的错误转换器：

```rust
use std::time::Duration;
use suprnova::{handler, FrameworkError, HttpResponse, Idempotency, Replay, Response};

#[handler]
pub async fn handler(order_id: i64) -> Response {
    let outcome: Replay<MyDto> = Idempotency::remember(
        &format!("order:{order_id}"),
        Duration::from_secs(60),
        || async move {
            let row = MyRow::find(order_id)
                .await?
                .ok_or_else(|| FrameworkError::not_found("missing"))?;
            Ok(MyDto::from(row))
        },
    )
    .await?;

    match outcome {
        Replay::Fresh(dto) | Replay::Replayed(dto) | Replay::FreshUnfenced(dto) => {
            let json = serde_json::to_value(&dto)
                .map_err(|e| FrameworkError::internal(format!("serialize: {e}")))?;
            Ok(HttpResponse::json(json))
        }
        Replay::InProgress => Ok(HttpResponse::text("retry")
            .status(409)
            .header("Retry-After", "1")),
    }
}
```

`commit_on_success` 或 `remember` 的 `Err` 路径上，一次释放失败**只会被记录，绝不会被返回** - 函数体的错误是调用方在那条路径上能看到的唯一错误。一次失败的释放，意味着这把锁会一直持有到 TTL 失效为止；在那之前，窗口内的一次重试会看到 `Duplicate` 或者 `InProgress`。日志里包含的是哈希过的键（绝不是原始的键材料），这样运维人员就能做关联，而不会泄漏 PII。

## 取消

如果调用方在函数体完成之前丢弃了 `Idempotency::remember` 这个 future，这个函数体就会像任何其他 `tokio::select!` 分支一样被取消 - 这把锁**不会**被释放，在 TTL 失效之前到达的一个重复请求，会看到 `InProgress`（TTL 过后，则再次看到 `Fresh`）。这是安全的默认行为：一个您不知道其影响的半途而废的函数体，不应该被假定为可以安全重试。如果您需要让某个函数体变得不可取消，请把那些持有非托管副作用的函数体包进 `tokio::spawn`，并 join 这个句柄。

## 队列集成

队列层在内部使用 `Idempotency::commit_on_success` 来实现 `Queue::push_unique`。如果您想让一个作业，在每一个 `Job::unique_for()` 窗口、每一个 `Job::unique_id(&self)` 上，最多只被入队一次，您不需要自己去调用 `Idempotency::*`：

```rust
use suprnova::{Job, Queue};

let was_pushed = Queue::push_unique(SendReceipt { order_id: 42 }).await?;
if was_pushed {
    // 我们赢得了这场竞态；这个作业已经在队列上了。
} else {
    // 另一个调用方已经把它入队了；把这当作成功处理。
}
```

完整的作业唯一性契约，请参见 [queues.md](queues.md)。

## 支付 webhook 入口

支付 webhook 处理程序**不**使用 `Idempotency::*`。webhook 入口有一个更严格的要求 - 每一个事件都必须是可审计的，哪怕是第一次投递也一样，所以那条审计行才是事实来源，而去重键是数据库的 `UNIQUE(provider, provider_event_id)` 约束。`Idempotency::remember` 会把响应载荷存进缓存；而这个 webhook 处理程序，存进 `payments_webhook_events` 的，是*完整的事件信封，外加处理结果*，这意味着一个运维人员可以通过读这张表，离线地重放或者重新处理事件。

这两种模式是互补的。对于由客户端驱动、限定 TTL 范围的去重键，用 `Idempotency::*`；对于需要审计能力超出缓存 TTL 的、由提供商驱动的 webhook 入口，用一张带 `UNIQUE` 索引的审计表。webhook 契约请参见 [payments.md](payments.md)。

### 为什么 Suprnova 有所不同

Laravel 的 `Cache::lock` 是一个原语；Stripe 风格的幂等性契约（记录结果、重放它、把进行中和重复请求区分开）则被留作一份用户态的方案。每一个需要它的 Laravel 项目，最终都会写出同一套锁加缓存的舞步，通常还带着下面这三个 bug 之一：

1. **没有租约续约。** 一个比 TTL 活得更久的函数体，会在一个重复调用方那里并发地重新执行。锁是在那里的；它只是在错误的时刻过期了。
2. **在成功路径上释放。** 在函数体成功时释放这把锁，会在 `body() -> Ok` 和下一个调用方获取一把全新的锁之间，打开一个窗口 - 而这正是去重原本要关上的那个窗口。
3. **缓存后端里的原始键。** 客户端提供的 `Idempotency-Key` 请求头，会直接进入 Redis 的键，把 PII 泄漏进运维工具里，并产生无上限的键大小。

Suprnova 把这份方案作为一个一等原语来提供，这样每一个调用方都能得到同样的租约续约、同样的失败关闭释放语义、同样的哈希键安全性。这三个方法（`once`、`commit_on_success`、`remember`）指名了您实际必须在其中做选择的三种策略 - 挑一个匹配您函数体失败模型的，然后继续往前走。

## 测试

`Idempotency` 通过容器解析它的 `CacheStore`，所以那些绑定了一个 `InMemoryCache` 的测试，每次测试都会得到一份全新的、隔离的缓存：

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::cache::InMemoryCache;
use suprnova::cache::store::CacheStore;
use suprnova::container::testing::TestContainer;
use suprnova::idempotency::{Idempotency, Replay};

#[tokio::test]
async fn duplicate_remember_replays_the_first_result() {
    let _guard = TestContainer::fake();
    let store: Arc<dyn CacheStore> = Arc::new(InMemoryCache::with_prefix("idem:"));
    TestContainer::bind::<dyn CacheStore>(store);

    let r1: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(7) },
    )
    .await
    .unwrap();
    assert_eq!(r1, Replay::Fresh(7));

    let r2: Replay<i32> = Idempotency::remember(
        "k",
        Duration::from_secs(60),
        || async { Ok(999) },
    )
    .await
    .unwrap();
    assert_eq!(r2, Replay::Replayed(7));
}
```

框架自己的 `framework/tests/idempotency.rs`，覆盖了这份契约的整个表面：重复抑制、TTL 过期、错误对成功的释放策略、跨越比 TTL 活得更久的函数体时长的租约续约、`InProgress` 竞态，以及缓存自己的 `release_lock` 出错的情形。如果您想看到自己能依赖的确切行为，请去读那些测试。

## 常见陷阱

- **`Idempotency::once` 在出错时会消耗掉这个窗口。** 一个失败了的第一个调用方，依然会一直持有这把锁，直到 TTL 失效。如果您想要窗口内的重试，请用 `commit_on_success`。
- **`Idempotency::remember` 会把 `T` 存进缓存后端。** 这个键是哈希过的，但*载荷*是用 serde 序列化、写进后端的。不要把那些绝不能出现在您缓存存储里的秘密，放进一个会被重放的值里。
- **两个进程需要一个共享的缓存。** 内存去重是逐进程的。跨进程的正确性，需要 `CACHE_DRIVER=redis`（或者另一种跨进程存储）。
- **低于 150 毫秒的 TTL 没有经过租约测试。** 续约下限是 50 毫秒，所以一个 100 毫秒的 TTL，大约每 50 毫秒刷新一次 - 对这份契约来说没问题，但框架的租约测试跑在 `ttl >= 1s` 上。请使用现实的去重窗口；一个以毫秒计的幂等性窗口，通常意味着这份契约不太是正确的工具。
- **函数体被取消不会释放这把锁。** 一个被取消的函数体，会让这把锁一直持有到 TTL 失效。这是失败关闭的选择；请安排好您的超时，让这次取消匹配上一个重复调用方应该看到的东西。

## 下一步

- [cache.md](cache.md) - 底层的锁原语，以及
  `CACHE_DRIVER` 的选定。
- [queues.md](queues.md) - `Queue::push_unique` 如何构建在
  `Idempotency::commit_on_success` 之上，来做作业级别的去重。
- [payments.md](payments.md) - 使用数据库行幂等性、而不是缓存建键去重的
  webhook 入口，以及什么时候该用哪一个。
- [rate-limiting.md](rate-limiting.md) - 使用同一个 `Cache` 后端来做滑动窗口强制执行的相邻中间件。
- [middleware.md](middleware.md) - 如何把幂等键提取，重构成一个可在您的 POST/PUT 路由上复用的中间件。
