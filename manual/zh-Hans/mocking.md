# 模拟和伪造

Suprnova 里每一个外部表面，都自带一个进程内的伪造实现，捕获您的代码本来会发送的东西 - 邮件、通知、排队的作业、被分发的命令、被触发的事件、被写入的文件、出站的 HTTP 调用 - 以及一套事后运行的匹配断言。形态总是一样的：装上这个伪造实现，运行被测试的代码，断言捕获到了什么。这一章是汇总性的概览；每个子系统章节（[邮件](mail.md)、[通知](notifications.md)、[队列](queues.md)、[总线](bus.md)、[事件](events.md)、[文件存储](filesystem.md)、[HTTP 客户端](http-client.md)）都会深入讲解它自己的伪造实现。

## 七个伪造实现

| 表面         | 入口点                                       | 断言风格                       | 并行安全性                                    | 章节                              |
|-----------------|---------------------------------------------------|---------------------------------------|----------------------------------------------------|--------------------------------------|
| 邮件            | `Mail::fake()` → `MailFake` 守卫                 | 守卫 上的方法                  | 需要 `#[serial]` - 全局传输层，没有序列化器 | [mail.md](mail.md)                   |
| 通知   | `Notify::fake()` → `NotifyFakeGuard`              | `notifications::testing` 里的自由函数 | 守卫 持有一个进程范围的序列化器            | [notifications.md](notifications.md) |
| 队列           | `suprnova::queue::testing::install_fake()`        | `queue::testing` 里的自由函数    | 守卫 持有一个进程范围的序列化器                | [queues.md](queues.md)               |
| 总线             | `suprnova::bus::testing::install_fake()`          | `bus::testing` 里的自由函数      | 守卫 持有一个进程范围的序列化器                | [bus.md](bus.md)                     |
| 事件          | `EventFacade::fake()` → `EventFakeGuard`          | `events` 里的自由函数            | 守卫 持有一个进程范围的序列化器                | [events.md](events.md)               |
| 存储         | `Storage::fake()` → `StorageFakeGuard`            | 一个 disk 上的 `DiskAssertExt` 方法     | 守卫 持有一个进程范围的序列化器                | [filesystem.md](filesystem.md)       |
| HTTP 客户端     | `Http::fake(\|\| async { … }).await`              | `assert_sent` / `assert_not_sent`     | 任务本地 - 测试之间真正并发         | [http-client.md](http-client.md)     |

有几条不变量在所有七个里都成立：

- **伪造实现会记录，真正的后端不会运行。** 邮件不会被发送，作业不会被推送到驱动程序，处理程序不会运行，事件会跳过它们的监听器，HTTP 不会触达网络，文件写入会进入一个内存磁盘。被捕获的那一侧，携带着足够断言本来会发生什么的信息。
- **这个 守卫 是 RAII 的。** 丢弃这个 守卫，会恢复之前就位的任何东西（之前的邮件传输层、一个干净的存储注册表、事件没有记录，等等）。测试不需要一个拆卸步骤。
- **这个伪造实现不会在错误上说谎。** 如果您的代码对一个未注册的命令调用 `Bus::dispatch`，这个伪造实现依然会返回 `Err(_)` - 只有成功的分发才会被捕获。

## 这些形态，以及它们为什么不同

有三种模式反复出现。知道一个伪造实现用的是哪种模式，就知道该导入一个自由函数、在 守卫 上调用一个方法，还是把测试主体包进一个闭包里。

### 守卫带方法（邮件）

`Mail::fake()` 返回一个 `MailFake`，它自己的方法就是那些断言。当断言者*就是*这个伪造实现本身时 - 您已经把它绑定到了一个局部变量上 - 这就很方便，但它是这种形态里唯一的一个伪造实现：

```rust,ignore
let fake = Mail::fake();
Mail::to("alice@example.org")
    .send(WelcomeEmail { name: "Alice".into() })
    .await?;
fake.assert_sent_count(1);
fake.assert_sent(|m| m.has_to("alice@example.org"));
```

### 守卫加自由函数（通知、队列、总线、事件）

这个 守卫 是一个什么都不做的令牌，它唯一的工作就是让这个伪造实现保持装着的状态；这些断言活在挨着伪造实现内部实现的一个 `testing` 子模块里。导入您需要的东西：

```rust,ignore
use suprnova::queue::testing::{install_fake, assert_pushed, pushed};

let _guard = install_fake();
schedule_welcome_email(user_id).await?;
assert_pushed::<WelcomeJob>(|j| j.user_id == user_id);
```

这是最常见的形态，因为它能干净地跨类型泛化 - 每一个断言都对 `J: Job` / `C: Command` / `E: Event` 是通用的，而不是被烤进一个 守卫 类型里。代价是多一次导入。
每一次被捕获的推送都携带伪造实现分配的信封 id，因此测试可以把它捕获的内容
与监听器看到的内容连接起来：

```rust,ignore
use suprnova::events::{EventFacade, dispatched};
use suprnova::queue::events::JobQueued;
use suprnova::queue::testing::{install_fake, pushed_with_id};

let _queue = install_fake();
let _events = EventFacade::fake();

Queue::push(SendInvoice { order_id: 7 }).await?;

let (job, id) = pushed_with_id::<SendInvoice>().remove(0);
assert_eq!(job.order_id, 7);
assert_eq!(dispatched::<JobQueued>(|_| true)[0].id, id);
```

在伪造实现下没有驱动程序，因此伪造实现本身会发出真实推送本来会发出的
`JobQueueing` / `JobQueued` 成对事件 - 使用它记录的 id。真实路径上的
`bulk` 和 `push_unique` 都不会发出事件，因此伪造实现也不会发出。

### 作用域带闭包（HTTP）

`Http::fake` 是那个特殊的例外。出站 HTTP 跑在不管哪个碰巧活着的 Tokio 任务上，所以这个伪造实现的状态活在一个 `tokio::task_local!` 里。您不能装一次就让它一直骑着走 - 您必须把调用这个客户端的主体包起来：

```rust,ignore
use suprnova::{Http, fake_response, assert_sent};

Http::fake(|| async {
    fake_response("POST", "/api/users", 201, serde_json::json!({"id": 1}));

    let resp = Http::post("https://example.com/api/users")
        .json(&serde_json::json!({"name": "Ada"}))
        .send()
        .await?;

    assert_eq!(resp.status(), 201);
    assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
})
.await;
```

回报是：其他每一个伪造实现都持有一个进程范围的序列化器，所以并行测试会一个接一个地运行，但 `Http::fake` 是真正并发的 - 每一个测试都得到自己的任务本地记录器，它们永远不会相撞。

### Storage 的扩展 trait

`Storage::fake()` 返回一个 守卫 *以及*一个默认的内存磁盘，但它的断言是通过 `DiskAssertExt` 这个扩展 trait，挂在这个磁盘本身上的：

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

let _guard = Storage::fake();
let disk = Storage::disk("default")?;

disk.put("invoices/42.pdf", b"...").await?;
disk.assert_exists("invoices/42.pdf").await;
disk.assert_count("invoices/", 1, false).await;
```

这个扩展 trait 是被 `#[cfg(any(test, feature = "testing"))]` 把关的，所以生产代码不会不小心调用 `disk.assert_exists(…)`。

## 并行安全性，一段话说完

七个伪造实现里，有六个守卫着一个进程全局的静态量。每一个的 守卫，在构造时都会拿走一个专用的 `FAKE_SERIAL` `std::sync::Mutex`，并一直持有它直到丢弃。效果是，任何两个装上同一个伪造实现的 `#[tokio::test]`，都会在一个进程下被串行化运行 - 不需要 [serial_test](https://crates.io/crates/serial_test) 这个 crate 里的 `#[serial]`。**Mail 是那个例外**：`MailFake` 这个 守卫 会交换全局的 `TRANSPORT`，却不拿走一个序列化器，所以并发的 `Mail::fake()` 测试*会*互相破坏。请给它们标上 `#[serial]`。**`Http::fake` 也是一个例外**：它是任务本地的，不是进程全局的，所以测试真正地并行运行，永远不需要 `#[serial]`。

如果您在同一个测试二进制文件里，对同一个表面交错使用真实分发和伪造分发，真实路径不会拿走这个序列化器，所以它可能和一个并行的伪造测试竞态。在这种情况下，请给真实分发的测试标上 `#[serial]` - 逐章节的文档会在适用的地方点出这一点（典范例子见[总线](bus.md)）。

## 邮件 - `Mail::fake()`

```rust,ignore
use serial_test::serial;
use suprnova::mail::{Mail, Address};

#[tokio::test]
#[serial]
async fn welcome_email_is_sent() {
    let fake = Mail::fake();

    register_user("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Welcome"));
    fake.assert_not_sent_to("eve@example.org");
}
```

| 断言                                  | 断言的是…                                            |
|--------------------------------------------|-----------------------------------------------------|
| `fake.assert_sent(\|m\| pred)`             | 至少一条被捕获的消息匹配               |
| `fake.assert_sent_to("…")`                 | 至少一条被捕获的消息被路由到了 email   |
| `fake.assert_not_sent(\|m\| pred)`         | 没有任何被捕获的消息匹配                         |
| `fake.assert_not_sent_to("…")`             | 没有任何消息去往 email                   |
| `fake.assert_sent_count(n)`                | 恰好 `n` 条被捕获的消息                       |
| `fake.assert_nothing_sent()`               | 什么都没有被捕获                             |
| `fake.assert_queued("MailableName")`       | 至少一个这个名字的排队 mailable           |
| `fake.assert_queued_with(name, \|q\| …)`   | 一个排队的 mailable 匹配这个谓词             |
| `fake.assert_queued_to("…")`               | 一个排队的 mailable 被路由到了 email               |
| `fake.assert_not_queued("MailableName")`   | 没有这个名字的排队 mailable   |
| `fake.assert_queued_count(n)`              | 恰好 `n` 个排队的 mailable                       |
| `fake.queued_on("…")`                      | 路由到某个队列的已排队邮件                  |
| `fake.assert_queued_on(name, "…")`         | 名称对应且路由到某个队列的已排队邮件    |
| `fake.queued_on_connection("…")`           | 路由到某个连接的已排队邮件             |
| `fake.assert_queued_on_connection(name, "…")` | 名称对应且路由到某个连接的已排队邮件 |
| `fake.assert_nothing_queued()`             | 什么都没有被排队                             |
| `fake.assert_outgoing_count(n)`            | 已发送 + 已排队总共 `n`                             |
| `fake.assert_nothing_outgoing()`           | 什么都没有被发送，也什么都没有被排队             |

`fake.captured()`、`fake.queued()`、`fake.sent(pred)`、`fake.sent_to(…)`、`fake.queued_named(…)`，以及 `fake.queued_to(…)`，会返回匹配的数据，这样您就能构建自定义断言。完整的表面，包括 `Mail::queue` 在 `Queue::fake` 都没有装上时，是如何被镜照进这个伪造实现的，请参见[邮件](mail.md)。
`queued_on_connection` / `assert_queued_on_connection` 读取
`QueuedSnapshot::connection` - `.on_connection(...)` 覆盖值（如果有）- 与下面普通作业路径中
`Queue::fake` 的 `assert_pushed_on_connection` 读取同一个字段，因此两个伪造实现保持对称。

## 通知 - `Notify::fake()`

```rust,ignore
use suprnova::notifications::{Notify, testing};

#[tokio::test]
async fn order_shipped_notifies_customer() {
    let _guard = Notify::fake();

    ship_order(order_id).await.unwrap();

    testing::assert_sent_to("alice@example.org", "OrderShipped");
    testing::assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    testing::assert_sent_times("OrderShipped", 1);
}
```

| 断言                                            | 断言的是…                                          |
|------------------------------------------------------|---------------------------------------------------|
| `assert_sent(\|r\| pred)`                            | 至少一个被分发的通知匹配      |
| `assert_sent_to(route, "Name")`                      | 具名的通知去到了这个逐通道的路由 |
| `assert_sent_to_on(route, channel, "Name")`          | 在这个通道上分发到了这个路由 |
| `assert_sent_named("Name")`                          | 具名的通知在任何通道上被分发了 |
| `assert_sent_times("Name", n)`                       | 恰好 `n` 个这个具名的通知             |
| `assert_nothing_sent()`                              | 没有任何通知被分发             |
| `assert_count(n)`                                    | 跨所有类型和通道恰好总共 `n` 个   |
| `assert_nothing_sent_to(route)`                      | 没有任何东西被分发到这个路由 |

`testing::recorded()` 会返回每一个 `FakeRecord`（通知名字、通道、路由、JSON 数据），用于更细粒度的断言。通知的收件人是按逐通道的 `route_for` 值建键的，所以 `assert_sent_to` 接受这个路由字符串（对 `"mail"` 是一个邮件地址，对 `"database"` 是字符串形式的 id，…） - 路由模型请参见[通知](notifications.md)。

## 队列 - `queue::testing::install_fake()`

```rust,ignore
use suprnova::Queue;
use suprnova::queue::testing::{
    install_fake, assert_pushed, assert_pushed_later, pushed,
};

#[tokio::test]
async fn order_placed_enqueues_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_pushed::<ChargeCustomerJob>(|j| j.order_id == 42);
}
```

| 断言                                      | 断言的是…                                                       |
|------------------------------------------------|----------------------------------------------------------------|
| `assert_pushed::<J>(\|j\| pred)`               | 至少一次 `J` 的推送匹配                               |
| `assert_pushed_later::<J>(\|j, at\| pred)`     | 一次 `J` 的推送被安排在了 `at`（延迟分发）         |
| `assert_pushed_on_queue::<J>(queue)`           | `J` 通过 [`EnvelopeOverrides`](queues.md#per-push-overrides-with-envelopeoverrides) 声明 `queue` 的推送 |
| `assert_pushed_on_connection::<J>(connection)` | `J` 通过 `EnvelopeOverrides` 声明 `connection` 的推送 |

数据那一侧会返回这些类型化的作业本身：

- `pushed::<J>() -> Vec<J>` - 每一次被捕获的 `J` 推送
- `pushed_with_available_at::<J>() -> Vec<(J, DateTime<Utc>)>` - 一样，但带着每个作业的计划时间戳
- `pushed_with_overrides::<J>() -> Vec<(J, EnvelopeOverrides)>` - 一样，但带着每个作业声明的逐推送覆盖值

只有 `Queue::push_with` 和 `Queue::later_with` 携带 `EnvelopeOverrides`，因此
`pushed_with_overrides` 对其他每一个入口点都会记录 `EnvelopeOverrides::default()` -
在伪造实现下，普通的 `Queue::push` 读取起来就像“没有声明覆盖值”，这与断言
`entries[0].1 == EnvelopeOverrides::default()` 相同。`assert_pushed_on_queue` /
`assert_pushed_on_connection` 检查的是声明的覆盖值，而不是解析后的队列或连接名称：
`Queue::route` 和 `Job::queue` / `Job::connection` 的解析不会在伪造实现下运行
（没有驱动器推送来触发解析），因此在生产环境中会落到某个路由或作业级默认值的作业，
在这里完全不会显示覆盖值。若要断言覆盖值携带的其他内容 - `timeout`、
`fail_on_timeout`、`max_tries`、`backoff` - 请直接使用 `pushed_with_overrides`。

每一个 `Queue::push`、`Queue::push_later`、`Queue::later`、`Queue::push_unique*`，以及链式/批处理分发器，都会汇聚进同一个记录器。伪造实现下 `push_unique` 的语义（它总是会记录并报告“已推送”），请参见[队列](queues.md)。

## 总线 - `bus::testing::install_fake()`

```rust,ignore
use suprnova::Bus;
use suprnova::bus::testing::{
    install_fake, assert_dispatched, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched,
};

#[tokio::test]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

| 断言                                           | 断言的是…                                                      |
|-----------------------------------------------------|-----------------------------------------------------------------|
| `assert_dispatched::<C>(\|c\| pred)`                | 至少一个被分发的 `C` 命令匹配                |
| `assert_not_dispatched::<C>(\|c\| pred)`            | 没有任何被分发的 `C` 命令匹配                          |
| `assert_dispatched_times::<C>(\|c\| pred, n)`       | 恰好 `n` 个被分发的 `C` 命令匹配                  |
| `assert_nothing_dispatched()`                       | 在这个活跃的伪造实现下，没有任何类型的命令被分发    |

在这个伪造实现下，`Bus::dispatch` 会返回 `Ok(Dispatched::Captured)`，而不是运行这个处理程序。真正的失败 - 编码/解码错误、在这个伪造实现被装上之前没有注册处理程序 - 依然会以 `Err(_)` 的形式暴露出来。参见[总线](bus.md)。

## 事件 - `EventFacade::fake()`

```rust,ignore
use suprnova::EventFacade;
use suprnova::events::{
    assert_dispatched, assert_dispatched_once, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched, dispatched,
    dispatched_count, dispatched_events, has_dispatched,
};

#[tokio::test]
async fn registration_dispatches_welcome_event() {
    let _guard = EventFacade::fake();

    register_user("ada@example.com").await.unwrap();

    assert_dispatched_once::<UserRegistered>();
    assert_dispatched::<UserRegistered>(|e| e.email == "ada@example.com");
}
```

| 断言                              | 断言的是…                                          |
|----------------------------------------|-----------------------------------------------------|
| `assert_dispatched::<E>(\|e\| pred)`   | 至少一个被分发的 `E` 匹配               |
| `assert_dispatched_once::<E>()`        | 恰好一个 `E` 被分发了                    |
| `assert_dispatched_times::<E>(n)`      | 恰好 `n` 个 `E` 被分发了                  |
| `assert_not_dispatched::<E>(\|e\| ..)` | 没有任何匹配的 `E` 被分发                    |
| `assert_nothing_dispatched()`          | 没有任何类型的事件被分发                     |
| `assert_listening::<E, L>()`           | 监听器 `L` 为 `E` 注册了               |
| `has_dispatched::<E>()`                | `bool`：任何 `E` 被记录了吗                          |
| `dispatched::<E>(\|e\| pred)`          | 匹配事件的 `Vec<E>` 克隆                |
| `dispatched_count::<E>(\|e\| pred)`    | 匹配事件的数量                    |
| `dispatched_events()`                  | 所有分发的 `HashMap<&'static str, usize>`  |

两个变体会缩小被伪造的范围：

```rust,ignore
// 只伪造这些 - 其他一切照常分发。
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// 伪造除了这些之外的每一个事件。
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

还有一个变体会抑制而不记录：

```rust,ignore
EventFacade::muted(async {
    // 没有监听器触发，没有事件被记录。
    run_bulk_import().await;
})
.await;
```

`muted` **不会**获取这个序列化器，所以 muted 作用域可以并行运行。完整的机制，包括 `assert_listening`（它只观察发生在这个伪造实现作用域*内部*的监听器注册），请参见[事件](events.md)。

## 存储 - `Storage::fake()`

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

#[tokio::test]
async fn invoice_upload_persists() {
    let _guard = Storage::fake();
    let disk = Storage::disk("default").unwrap();

    upload_invoice(b"%PDF-1.7 …").await.unwrap();

    disk.assert_exists("invoices/2026/05/30/inv-00042.pdf").await;
    disk.assert_contents("invoices/2026/05/30/inv-00042.pdf", b"%PDF-1.7 …").await;
}
```

这个 守卫 会预先注册一个 `"default"` 内存磁盘，所以简单的测试不需要任何磁盘设置。如果被测试的代码会伸手去拿一个非默认的磁盘，请在测试内部用 `Storage::register_memory("audit_logs")` 以自定义名字注册额外的磁盘。

| 断言                                        | 断言的是…                                          |
|--------------------------------------------------|---------------------------------------------------|
| `disk.assert_exists(path).await`                 | 这个路径存在                                   |
| `disk.assert_contents(path, &expected).await`    | 这个文件逐字节匹配 `expected`         |
| `disk.assert_missing(path).await`                | 这个路径不存在                            |
| `disk.assert_count(dir, n, recursive).await`     | `dir` 恰好包含 `n` 个条目                       |
| `disk.assert_directory_empty(dir).await`         | `dir` 没有任何条目（递归）                       |

这五个方法都会在不匹配时 panic，消息里带着磁盘路径。`Storage` 这个门面本身，以及驱动程序的故事（memory / fs / s3 / azblob / gcs），请参见[文件存储](filesystem.md)。

## HTTP 客户端 - `Http::fake`

```rust,ignore
use suprnova::{Http, fake_response, assert_sent, assert_not_sent};

#[tokio::test]
async fn payment_webhook_is_acked() {
    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 201, serde_json::json!({
            "id": "ch_42",
            "status": "succeeded",
        }));

        let result = charge_card(amount_cents).await;

        assert!(result.is_ok());
        assert_sent(|r| r.method == "POST" && r.url.contains("/v1/charges"));
        assert_not_sent(|r| r.method == "DELETE");
    })
    .await;
}
```

`fake_response(method, url_substring, status, body)` 会排一个预设响应进队列。方法 `"*"` 匹配任何方法。每一个预设条目会在第一次匹配的请求上被消耗掉；后续匹配的请求，或者落到下一个预设条目，或者返回一个空的 `200 {}`。

| 助手函数                                       | 用途                                                   |
|----------------------------------------------|-----------------------------------------------------------|
| `Http::fake(\|\| async { … }).await`         | 装上这个任务本地的伪造作用域                         |
| `fake_response(method, url_substring, …)`    | 把一个预设响应排进队列                            |
| `assert_sent(\|r\| pred)`                    | 断言至少一个被记录的请求匹配              |
| `assert_not_sent(\|r\| pred)`                | 断言没有任何被记录的请求匹配                |

### 被 spawn 出的任务默认不会继承这个伪造实现

`tokio::spawn` 不会把任务本地状态带进被 spawn 出的 future 里，所以逃出父任务的工作，也会逃出这个伪造实现。两个工具能处理这一点：

```rust,ignore
// 双重保险：把每一个没被伪造的出站调用，都变成一个硬性错误。
let _guard = suprnova::FailOnRealCallsGuard::install();

Http::fake(|| async {
    fake_response("GET", "/child", 204, serde_json::json!({}));

    // 显式选择加入：这个子任务能看到父任务的伪造状态。
    let handle = Http::spawn_with_fake_inheritance(async {
        Http::get("https://child.test").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);
})
.await;
```

`FailOnRealCallsGuard` 是 RAII 的 - 在一个测试的开头装上它，任何没有命中一个活跃伪造实现的出站调用，都会报错，而不是触达网络。`Http::spawn_with_fake_inheritance` 是给那些应该共享父任务伪造状态的任务用的显式选择加入。完整的讨论请参见[HTTP 客户端](http-client.md)。

## 广播

WebSocket 广播有一个并列的测试装置，但它的形态差异大到值得有自己的章节：`RecordingBroadcastHub` 是一个真正的 `BroadcastHub`，会记录每一个被发布的信封，同时依然投递给活跃的订阅者。把它绑定到 `InMemoryBroadcastHub` 的位置上，并调用 `hub.broadcasts()` / `hub.assert_broadcast(channel, event)`。广播模型和这个记录中枢的用法，请参见[广播](broadcasting.md)。

## 每一个伪造实现位于何处

| 表面       | 源码                                | 门面重导出                             |
|---------------|---------------------------------------|----------------------------------------------|
| 邮件          | `framework/src/mail/mod.rs`           | `suprnova::{Mail, MailFake}`                 |
| 通知 | `framework/src/notifications/testing.rs` | `suprnova::{Notify, NotifyFakeGuard}` + `suprnova::notifications::testing::*` |
| 队列         | `framework/src/queue/testing.rs`      | `suprnova::queue::testing::*`                |
| 总线           | `framework/src/bus/testing.rs`        | `suprnova::bus::testing::*`                  |
| 事件        | `framework/src/events/testing.rs`     | `suprnova::{EventFacade, EventFakeGuard}` + `suprnova::events::*` |
| 存储       | `framework/src/filesystem/testing.rs` | `suprnova::{Storage, DiskExt}` + `suprnova::filesystem::testing::DiskAssertExt` |
| HTTP          | `framework/src/http_client/fake.rs`   | `suprnova::{Http, fake_response, assert_sent, assert_not_sent, FailOnRealCallsGuard, RecordedRequest}` |

`testing` 和 `fake` 这两个模块，是被一个名为 `testing` 的 Cargo feature 把关的。它在默认 feature 集合里，所以任何依赖 `suprnova` 的测试都能免费拿到这些辅助函数。这些钩子自己在可能被应用代码不小心触达的地方是 `#[doc(hidden)]` 的；真正承重的那道防线，是 `Server::from_config` 的 `APP_KEY` 校验，它会在每一次启动时运行，不管编译进来了哪些测试辅助函数。生产构建的故事请参见[测试](testing.md)。

## 为什么是这些形态，而不是一种形态

一种统一的形态在文档页面上会更整洁，但在实践里会更糟。每一种形态存在，都是因为底层的状态有着不同的并发语义：

- **Mail 的**传输层是一个由 守卫 交换的全局 `Arc<dyn MailTransport>`。返回的 守卫 上的方法断言，把断言者绑定到了那个具体的安装上，这让在没有伪造实现活跃时调用断言变得不可能。
- **Notify / Queue / Bus / Events** 是在异构的类型化负载上做断言 - 每一个断言都对事件/作业/命令类型是通用的。一个 `testing` 模块里的自由函数，和类型参数组合起来，比 守卫 上一套手写的方法集更干净。
- **Storage** 的断言是逐磁盘的，不是逐伪造实现的 - 同一个 `disk.assert_exists(…)`，在一个集成测试套件里，既能对着一个伪造的内存磁盘工作，也能对着一个真正的 `s3` 磁盘工作。通过一个扩展 trait 把它们放在磁盘上，保持了这份对称性。
- **HTTP** 必须跟着任务走，而不是跟着调用栈。`Http::fake` 是唯一一个作用域没法表达成一个 守卫 的伪造实现 - spawn 的语义强制要求一个闭包。

如果您发现自己在伸手去拿一个不存在的辅助函数，请读相关的章节；公开的测试表面，是逐子系统被详尽记录下来的。

## 下一步

- [测试](testing.md) - `#[suprnova_test]` 这个宏、`TestDatabase`、`expect!`，以及 `TestContainer::fake`
- [HTTP 测试](http-tests.md) - 不打开一个套接字，直接驱动 `handle_request`
- [数据库测试](database-testing.md) - 逐测试的内存数据库故事
- [服务容器](container.md) - 用 `TestContainer::fake` 来交换注入的服务
