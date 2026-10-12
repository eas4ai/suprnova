# 通知

一个通知，是您想让一个用户（或者「任何有邮箱地址的人」）通过一个或多个通道收到的一条简短消息 - 邮件、应用内收件箱、浏览器推送、实时 WebSocket - 都来自同一个调用点。您写下 `Notify::send(&user, &OrderShipped { … })`；分发器会把这一个通知，扇出给这个通知声明的每一个通道，并通过这个收件人来给每一个通道定地址。

当*什么发生了*（一个订单发货了，一张发票付清了）比*怎么发生的*（究竟是哪种传输方式最终投递了它）更让您的代码关心时，就用通知。要拿到裸的传输访问权限 - 编写一个自定义的邮件正文，发布到一个特定的广播频道，发送一次一次性的 web 推送 - 就直接通过[邮件](mail.md)、[广播](broadcasting.md)，或者
[web 推送](web-push.md)。

## 快速上手

```rust
use serde::{Deserialize, Serialize};
use suprnova::FrameworkError;
use suprnova::NotificationMailable;          // derive 宏
use suprnova::notifications::channels::mail::MailRendering;
use suprnova::{Notifiable, Notification, Notify};

#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Order shipped - tracking {{ tracking }}",
    html    = "<p>Your order is on its way.</p><p>Tracking: <code>{{ tracking }}</code></p>",
    text    = "Tracking: {{ tracking }}",
    from    = "orders@example.com",
    from_name = "Acme Orders",
)]
pub struct OrderShipped {
    pub tracking: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str { "OrderShipped" }
    fn channels(&self) -> Vec<&'static str> { vec!["mail", "database"] }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "tracking": self.tracking })
    }
}

struct User { id: i64, email: String }
impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "mail"     => Some(self.email.clone()),
            "database" => Some(self.id.to_string()),
            _          => None,
        }
    }
}

async fn ship(user: &User, tracking: String) -> Result<(), FrameworkError> {
    Notify::send(user, &OrderShipped { tracking }).await
}
```

`Notify::send` 会在一次调用里，同时分发给邮件通道和数据库通道。收件人通过从 `route_for` 返回 `None` 来拒绝一个通道 - 这对「只用邮件」或者「只用推送」的用户很有用。

## 这三个 trait

| Trait | 代表什么 | 由谁实现 |
|---|---|---|
| `Notification` | 一条类型化的消息 + 它分发到的那些通道 | 您的通知结构体 |
| `Notifiable` | 一个收件人 - 暴露一个逐通道的 `route_for` | 您的 `User`、`Order`，任何可寻址的东西 |
| `Channel` | 一个传输方式 - 知道怎么投递到一个路由 | 内置：`MailChannel`、`DatabaseChannel`、`BroadcastChannel`、`WebPushChannel` |

### `Notifiable`

```rust
pub trait Notifiable: Send + Sync {
    fn route_for(&self, channel: &str) -> Option<String>;
}
```

收件人拥有逐通道的定址权。`route_for("mail")` 返回邮箱地址；
`route_for("database")` 把实体 id 作为字符串返回；`route_for("webpush")`
返回一份序列化的 `SubscriptionInfo` JSON；`route_for("broadcast")` 返回这个广播频道的名字。对这个收件人返回 `None`，来跳过一个通道。

### `Notification`

```rust
pub trait Notification: Serialize + DeserializeOwned + Send + Sync + 'static {
    fn notification_name() -> &'static str where Self: Sized;
    fn channels(&self) -> Vec<&'static str>;
    fn data(&self) -> serde_json::Value;

    fn should_send(&self, _channel: &str) -> bool { true }
    fn after_sending(&self, _channel: &str) -> Result<(), FrameworkError> { Ok(()) }

    fn queue(&self) -> Option<&'static str> { None }
    fn timeout(&self) -> Option<std::time::Duration> { None }
    fn fail_on_timeout(&self) -> bool { false }
    fn max_tries(&self) -> u32 { 3 }
    fn backoff(&self) -> BackoffSchedule { BackoffSchedule::default() }
}
```

| 方法 | 用途 |
|---|---|
| `notification_name()` | 由数据库通道持久化的稳定标识符，被用作队列信封的键，以及邮件渲染器注册表的查找键。 |
| `channels(&self)` | 这个通知分发到的那些通道名。顺序就是迭代顺序。 |
| `data(&self)` | 通道会投递 / 持久化的、可 JSON 序列化的载荷。通常是这些通道需要的那个字段子集的 `serde_json::to_value(self)`。 |
| `should_send(&self, channel)` | 在同步路径和已入队路径上都会被查询的逐通道否决权。返回 `false`，会为这次分发跳过那个通道。默认：总是发送。 |
| `after_sending(&self, channel)` | 每一个完成的通道各调用一次的成功后钩子，在同步路径和已入队路径上都会调用。返回 `Err`，会以和一个通道错误一样的方式传播。默认：空操作。 |
| `queue(&self)` | `Notify::queue` 分发解析到的队列。默认：`None`（驱动程序默认值，或者已注册的 `Queue::route`）。参见[队列调优](#队列调优)。 |
| `timeout(&self)` | 此通知的已入队作业每次尝试的超时。默认：`None`（无超时）。 |
| `fail_on_timeout(&self)` | 如果为 `true`，超时是永久失败（死信，不重试）。默认：`false`。 |
| `max_tries(&self)` | 此通知的已入队作业的最大尝试次数。默认：`3`。 |
| `backoff(&self)` | 此通知的已入队作业的退避计划。默认：框架默认值。 |

`should_send` 和 `after_sending`，在**这两条**路径上都会被遵守。
`Notify::send` 会在分发器里查询它们；`Notify::queue` 会在把每一个逐通道的作业入队之前检查 `should_send`，而工作进程会在投递之前重新检查一次
`should_send`（状态可能在入队和运行之间发生变化），并在一次成功的发送之后运行 `after_sending`。这三个生命周期*事件*（`NotificationSending` /
`NotificationSent` / `NotificationFailed`）仍然只会在同步路径上触发。

## 频道

### 邮件

邮件通道通过绑定好的邮件传输方式投递（参见[邮件](mail.md)）。一个通知通过实现 `NotificationMailable` 来选择加入：

```rust
pub trait NotificationMailable: Notification {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError>;
}
```

`MailRendering` 是这个渲染信封 - `subject`（必需），`html` 和/或 `text`
（至少需要一个），可选的 `from`、`cc`、`bcc`、`reply_to`，以及
`attachments`。邮件通道会从这份渲染结果，加上收件人的
`route_for("mail")`，组装出一条外发消息，应用配置好的发送方默认值（`Mail::always_from(...)`、`always_to(...)` 等等），并通过
`Mail::current_transport` 分发。

如果这个渲染器返回一份既没有 `html` 也没有 `text` 的渲染结果，投递会快速失败 - 一封空白的通知邮件永远不会被静默地发送出去。

#### `#[derive(NotificationMailable)]`

这个 derive，把逐 Notification 的 `to_mail` `impl`，折叠成一个
`#[mail(...)]` 属性。模板使用 [Tera](https://keats.github.io/tera/)；
`self` 序列化后的字段就是这个上下文。

```rust
#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Welcome {{ name }}",
    html_template = "templates/welcome.html",
    text_template = "templates/welcome.txt",
    from = "hello@example.com",
    from_name = "Acme",
    cc = "ops@example.com, support@example.com",
)]
pub struct Welcome { pub name: String }
```

支持的键：

| Key | Required? | Purpose |
|---|---|---|
| `subject` | 是 | Tera 模板 - 用 `self` 作为上下文渲染。 |
| `html` | 匕首 | 内联的 HTML 正文 Tera 模板。 |
| `html_template` | 匕首 | 一个 HTML 正文 Tera 模板的路径（通过 `include_str!` 嵌入）。 |
| `text` | 匕首 | 内联的纯文本正文 Tera 模板。 |
| `text_template` | 匕首 | 一个纯文本正文 Tera 模板的路径（通过 `include_str!` 嵌入）。 |
| `from` | 否 | 发送方邮箱 - 覆盖默认的 `noreply@localhost`。 |
| `from_name` | 否 | 显示名称。需要 `from`。 |
| `cc` | 否 | 逗号分隔的 CC 列表。空白和结尾的逗号会被忽略。 |
| `bcc` | 否 | 逗号分隔的 BCC 列表。 |
| `reply_to` | 否 | 逗号分隔的 Reply-To 列表。 |

（匕首）必须存在至少一个正文变体。`html` 和 `html_template` 互斥；
`text` 和 `text_template` 也是。

每一项不变式都在编译期被强制执行 - 缺失 `subject`、空正文、冲突的变体、没有 `from` 的 `from_name`，或者未知的键，都会让构建失败，而不是在分发时失败。

对于附件（二进制载荷）或者逐实例的动态收件人，就手写实现
`NotificationMailable`，并直接构建这个 `MailRendering`。

### 数据库

数据库通道会把每一个通知，持久化成 `notifications` 表里的一行：

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, NotificationDispatcher};

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")));
```

第二个参数，是收件人的多态类型标签（您存进 `notifiable_type` 里的东西，这样之后就能把收件箱行查回来）。收件人的 `route_for("database")` 会成为这个 `notifiable_id`。这份迁移随框架一起发布（`framework/migrations/20260516_create_notifications_table.sql`）；运行 `suprnova migrate`，这张表就会出现。

#### 读取收件箱

读取一侧的辅助函数，作为作用于 `(notifiable_type, notifiable_id)` 的自由函数，活在 `suprnova::notifications` 里：

```rust
use suprnova::notifications::{
    all_for, unread_for, read_for,
    mark_as_read, mark_as_unread, mark_all_as_read,
    delete_for, StoredNotification,
};

let unread: Vec<StoredNotification> = unread_for(&db, "users", "42").await?;
let count = mark_all_as_read(&db, "users", "42").await?;
let removed = delete_for(&db, "users", "42").await?;
```

`StoredNotification` 携带 `id`、`type_name`（即
`Notification::notification_name`）、`notifiable_type`、
`notifiable_id`、解码后的 JSON `data`、`read_at`、`created_at`、
`updated_at`。`mark_as_read` / `mark_as_unread` 是幂等的（匹配 Laravel
的契约）。

### Web 推送

Web 推送通道会加密这个载荷，并通过框架的 VAPID 签名客户端，把它 POST
到一个存储好的浏览器推送订阅端点：

```rust
use std::sync::Arc;
use suprnova::WebPushChannel;
use suprnova::web_push::{VapidKey, WebPushClient};

let client = WebPushClient::new(
    VapidKey::from_pem(b"-----BEGIN PRIVATE KEY-----\n…")?,
    "mailto:ops@example.com",
)?;
let push_channel = WebPushChannel::new(Arc::new(client), 86_400 /* TTL seconds */);
```

收件人的 `route_for("webpush")` 返回一份序列化的 `SubscriptionInfo`
JSON（和浏览器从 `PushSubscription.toJSON()` 交回来的形状一样 - 原样存储它，原样返回它）。这个 TTL 会被转发给推送服务。

当推送服务告诉这个通道一个订阅已经不在了（HTTP 404/410）时，这个通道会记录一条结构化的 WARN，并返回成功 - 这个通知已经到达一个终态，没有收件人可以重试。运维人员看到这条日志，并移除这个失效的订阅；投递不会报错。

关于完整的客户端，请参见[Web 推送](web-push.md)。

### 广播

广播通道会把每一个通知发布到应用的 `BroadcastHub` 上，这样 WebSocket
订阅者就能实时收到它。收件人的 `route_for("broadcast")` 就是这个频道名，这个通知类型就是这个事件，而 `data()` 就是这个载荷：

```rust
use std::sync::Arc;
use suprnova::BroadcastChannel;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;

// 在启动时 - 在任何广播分发之前绑定这个中枢。
App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

let dispatcher = suprnova::NotificationDispatcher::new()
    .register_channel(Arc::new(BroadcastChannel::new()));
```

这个通道会在投递时，从容器里解析出这个中枢。如果一个通知声明了
`"broadcast"`，但没有绑定任何 `BroadcastHub`，这个通道会返回一个错误 -
一个配置错误的应用，会把这个问题暴露出来，而不是静默地丢弃这条消息。发布到一个零活跃订阅者的频道，不算一个错误。

关于中枢搭建和 WebSocket 管路，请参见[广播](broadcasting.md)。

## 按需通知

有时候您想通知*一个不在您数据库里的人* - 一次给一个邮箱地址发送的一次性运维告警，一个 webhook 接收方，一个不属于任何用户的广播频道。
`AnonymousNotifiable` 就是那个「没有一行数据的用户」：

```rust
use suprnova::Notify;

let recipient = Notify::route("mail", "ops@example.com")?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;

// 在一个构建器里用多个通道：
let recipient = Notify::routes([
    ("mail", "ops@example.com"),
    ("broadcast", "ops-channel"),
])?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;
```

`Notify::route("database", …)` 和
`Notify::routes([..., ("database", …)])` 会返回 `Err` - 数据库通道持久化的是一对 `(notifiable_type, notifiable_id)`，而一个匿名收件人没法提供这个。

## 分发器

`NotificationDispatcher` 持有这个通道注册表。在启动时构建它一次，并全局地绑定它：

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, MailChannel, NotificationDispatcher, WebPushChannel};
use suprnova::notifications::set_dispatcher;

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(MailChannel::new()))
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")))
    .register_channel(Arc::new(WebPushChannel::new(push_client, 86_400)));

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` 在这个通道名上是后写入者胜出 - 注册两个都叫
`"mail"` 的通道，会静默地替换掉第一个。这让测试的搭建变得符合人体工程学。

一个通知声明了一个分发器没有注册的通道，会记录一条 WARN
（`no channel registered; skipping`），并继续到下一个通道 - 分发不会因为一个未知的通道名而报错。

`set_dispatcher` 返回 `Result<(), FrameworkError>`，因为这个分发器注册表活在一个 `RwLock` 后面；这条错误路径只会在这个锁被污染时触发（一个之前的写入者 panic 了）。在实践中，启动时的这个调用点会用 `?`。

### 生命周期事件

每一次同步的通道投递，周围都围绕着三个事件：

| 事件 | 什么时候 | 监听器错误的行为 |
|---|---|---|
| `NotificationSending` | 就在这个通道运行之前 | 监听器的 `Err` 会为这次分发**否决**这个通道 |
| `NotificationSent` | 在一次成功投递之后 | 尽力而为的分发 - 监听器的错误不会传播 |
| `NotificationFailed` | 当一个通道返回了一个错误时 | 尽力而为的分发；按照「第一次失败就停止」的契约，底层的通道错误仍然会传播 |

这三者都携带 `(notification, channel, route, data)`。`Failed` 会额外带上字符串化的 `error`。用 `EventFacade::listen::<E, L>` 来监听 - 参见
[事件](events.md)。

这些事件只会在同步的 `Notify::send` 路径上触发。已入队的工作进程会直接投递通道，而不分发这些事件。

### 遥测

`NotificationDispatcher::notify` 会把这次扇出，包在一个
`notification.dispatch` tracing span 里：

- `notification` - `Notification::notification_name()`
- `channel_count` - 声明的通道数量
- `duration_ms` - 完成时的扇出延迟
- 终态日志：`notification dispatched`（info）或者
  `notification dispatch failed`（warn）

邮件通道会在里面嵌套它自己的 `mail.send` span。

### 第一次失败就停止的契约

`Notify::send` 会在第一个通道错误上返回。已经成功的通道不会被回滚；还没运行的通道不会被尝试。同样的契约适用于已入队的工作进程。

要在多个通道上做到至少一次，就通过各自独立的 `Notify::queue` 调用，去分发每一个通道 - 队列信封的幂等键，能防止重试时的重复发送。

## 已入队的投递

`Notify::send` 在进程内运行。`Notify::queue` 会把一个
`SendNotificationJob` 推送到[队列](queues.md)上，预先从这个收件人解析出逐通道的路由，这样工作进程在执行时就不需要一个 `Notifiable` 句柄：

```rust
use suprnova::notifications::register_notification_factory;
use suprnova::Notify;

// 在启动时 - 每一个能通过 Notify::queue 到达的具体通知各一次。
register_notification_factory::<OrderShipped>()?;

// Anywhere:
Notify::queue(&user, OrderShipped { tracking }).await?;
```

在分发时，这个工作进程会：

1. 按 `notification_name` 查找这个通知工厂
2. 从这个 JSON 载荷重新构建出这个类型化的通知
3. 遍历在入队时记录下来的那些通道
4. 对每一个，重新检查 `should_send(channel)`（跳过被否决的通道），在绑定好的分发器上查找这个通道，调用 `deliver(route, &notification)`，然后运行 `after_sending(channel)`

在入队时声明了、但工作进程运行时却没有注册的通道，会记录一条 WARN 并被跳过 - 和同步路径一样的契约。没有预先解析出路由的通道，会被静默地跳过（收件人在入队时返回了 `None`）。

`Notify::queue` 也会在入队时求值 `should_send`，所以一个被否决的通道，压根就不会被入队；工作进程的重新检查，覆盖的是入队和运行之间发生变化的状态。已入队的路径**不会**触发这三个生命周期事件（`NotificationSending` / `NotificationSent` / `NotificationFailed`） -
那些仍然只在同步路径上有。如果您依赖这些事件，就通过 `Notify::send`
发送。

### 队列调优

另外五个 `Notification` 方法会把逐通知的队列策略带到 `Notify::queue` 的分发中，对应 `Job` 自身的调优方法：

| 方法 | 默认值 | 对应 |
|---|---|---|
| `queue(&self)` | `None` - 驱动程序默认值，或者已注册的 `Queue::route` | `Job::queue()` |
| `timeout(&self)` | `None` - 没有逐次尝试的超时 | `Job::timeout()` |
| `fail_on_timeout(&self)` | `false` - 超时像其他失败一样重试 | `Job::fail_on_timeout()` |
| `max_tries(&self)` | `3` | `Job::max_tries()` |
| `backoff(&self)` | 指数退避，2 秒基准、5 分钟上限、±25% 抖动 | `Job::backoff()` |

`Notify::queue` 会从通知实例中读取这五项一次，并把它们携带到每个逐通道的
`SendNotificationJob` 推送上。没有覆盖这五项中的任何一项的通知，会获得裸的
`Notify::queue` 调用一直产生的完全相同的信封。

```rust
struct WelcomeDigest;

impl Notification for WelcomeDigest {
    fn notification_name() -> &'static str { "WelcomeDigest" }
    fn channels(&self) -> Vec<&'static str> { vec!["mail"] }
    fn data(&self) -> serde_json::Value { serde_json::Value::Null }

    fn queue(&self) -> Option<&'static str> { Some("digests") }
    fn timeout(&self) -> Option<std::time::Duration> { Some(std::time::Duration::from_secs(10)) }
    fn fail_on_timeout(&self) -> bool { true }
}
```

当超时意味着无法恢复而不是暂时性故障时，请将 `fail_on_timeout(&self)` 设置为
`true`：工作进程会在第一次超时时将作业投入死信，而不是重试到 `max_tries`。

这五个方法只适用于 `Notify::queue` - `Notify::send` 在进程内运行，没有可供调优的队列信封。


### 为什么 Suprnova 有所不同

Laravel 依据 `ShouldQueue` 这个标记接口，来决定通知是否入队 - 同样的
`Notification::send($user, $notification)` 调用，如果这个通知实现了
`ShouldQueue`，就会入队，否则就内联发送。这个行为，取决于通知那一侧一个类型层面的标志，而这在调用点是不可见的。

Suprnova 让这个选择在每一次调用时都是显式的：`Notify::send` 总是同步的；`Notify::queue` 总是入队的。没有隐藏的模式切换。（这也是为什么没有 `send_now` - `send` 本身就已经是那个同步的了。）

收件人这一侧也有分歧。Laravel 的 `Notifiable` trait 是一个混入（mixin），它拉进了收件箱关系、`routeNotificationFor*` 方法，以及那个多态主键。Suprnova 的 `Notifiable` 刻意做得很精简 - 就是
`route_for(channel) -> Option<String>` - 因为 Rust 的 trait 不是通过混入来组合的。与 Laravel 等价的读取一侧，作为作用于
`(notifiable_type, notifiable_id)` 的自由函数发布（`unread_for`、
`mark_as_read`……），这样朴素的结构体就能是可通知的，而不必继承一个
ORM 关系。

## 测试

两个伪造实现表面，回答的是不同的问题。

### `Notify::fake()` - 「一个通知被分发了吗？」

```rust
use suprnova::Notify;
use suprnova::notifications::{
    assert_count, assert_nothing_sent, assert_sent_named,
    assert_sent_times, assert_sent_to, assert_sent_to_on,
    recorded_notifications,
};

#[tokio::test]
async fn ship_dispatches_order_shipped() {
    let _fake = Notify::fake();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    assert_sent_named("OrderShipped");
    assert_sent_to("alice@example.org", "OrderShipped");
    assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    assert_sent_times("OrderShipped", 1);
    assert_count(2); // 邮件 + 数据库
}
```

当这个伪造守卫存活时，`Notify::send` 和 `Notify::queue` 都会记录这次分发，而不运行通道，也不将一个作业入队 - 没有通道会运行，没有队列行会被写入。这个伪造实现，持有一个进程范围的序列化互斥锁，所以并行的测试不能交错捕获；让这个 `_fake` 守卫在测试结束时被丢弃，来清空这个记录器。

用 `recorded_notifications()` 来完全掌管被捕获的数据：

```rust
let records = recorded_notifications();
assert_eq!(records[0].notification, "OrderShipped");
assert_eq!(records[0].channel, "mail");
assert_eq!(records[0].data["tracking"], "1Z…");
```

### `Mail::fake()` + 真实的 `MailChannel` - 「这个通知*渲染*对了吗？」

`Notify::fake()` 会在到达这个通道之前就短路。要断言这个邮件正文确实按您期望的方式渲染了，就在 `Mail::fake()` 之下驱动这个真实的通道：

```rust
use serial_test::serial;
use std::sync::Arc;
use suprnova::mail::Mail;
use suprnova::notifications::{set_dispatcher, NotificationDispatcher};
use suprnova::{MailChannel, Notify, register_mail_renderer};

#[tokio::test]
#[serial]
async fn ordershipped_renders_tracking_in_subject() {
    let fake = Mail::fake();
    register_mail_renderer::<OrderShipped>().unwrap();
    set_dispatcher(Arc::new(
        NotificationDispatcher::new()
            .register_channel(Arc::new(MailChannel::new())),
    )).unwrap();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.subject.contains("1Z…"));
}
```

会碰到分发器、渲染器或者传输全局状态的测试，必须是
`#[serial_test::serial]` 的 - 那些都是进程全局的 static。

## 最佳实践

### 在启动时注册每一个工厂和渲染器

`Notify::queue` 会在工作进程上，通过这个工厂注册表重新构建这个通知，而 `MailChannel` 通过 `register_mail_renderer` 来渲染。提前注册每一个可入队 / 可邮寄的通知：

```rust
// bootstrap.rs
use suprnova::notifications::register_notification_factory;
use suprnova::register_mail_renderer;

pub fn register() -> Result<(), FrameworkError> {
    // 通知工厂（每一个能通过 Notify::queue 到达的 Notification 各一个）。
    register_notification_factory::<OrderShipped>()?;
    register_notification_factory::<InvoicePaid>()?;

    // 邮件渲染器（每一个 NotificationMailable 各一个）。
    register_mail_renderer::<OrderShipped>()?;
    register_mail_renderer::<InvoicePaid>()?;
    Ok(())
}
```

队列上一个未注册的通知，会在工作进程执行时，以
`unknown notification: {name}` 的形式暴露出来，并通过死信路径重试。一个针对未注册渲染器的 `MailChannel` 分发，会用同样的方式，暴露一个
`register via suprnova::register_mail_renderer::<N>()` 错误。

### 为多通道扇出使用队列

同步分发器按顺序访问这些通道，并在第一个错误上返回。通道 #2 上的一次失败，会让通道 #1 已经提交，而通道 #3 及之后的都没被尝试。对任何跨越多个通道的通知，优先选用 `Notify::queue`，这样工作进程就会带着退避去处理重试，这次分发也能在一次进程崩溃中存活下来。

### 让通道投递保持幂等

工作进程的重试，意味着同一个 `SendNotificationJob` 可能会执行超过一次。内置的通道对幂等性都很友好：`MailChannel` 会转发给那些通常按
message-id 去重的提供商；`DatabaseChannel` 会为每一次执行插入一个全新的 UUID（这对一条审计行来说是正确的行为）；`WebPushChannel` 会
POST 给一个会吞掉重复项的提供商。自定义通道应该以幂等操作为目标 - 带着稳定的客户端去重键的 HTTP POST，用 upsert 而不是盲目的插入，在投递路径上没有「递增一个计数器」这类副作用。

### 把这个分发器绑定在一个地方

`register_channel` 是后写入者胜出的，所以测试可以在搭建阶段，把一个真实的通道换成一个替身。把生产环境的绑定留在 `bootstrap.rs` 里，让测试用它们需要的任何替身，去构建自己的分发器。不要在请求处理程序内部惰性地调用 `register_channel` - 全局锁写入加上后写入者胜出的语义，在并发负载下会变得让人意外。

## 参考

| 符号 | 路径 |
|---|---|
| `Notifiable`, `Notification`, `Channel`, `DynNotification` | `suprnova::` |
| `Notify`（门面）, `NotifyFakeGuard` | `suprnova::` |
| `NotificationDispatcher`, `NotificationFactory` | `suprnova::` |
| `AnonymousNotifiable` | `suprnova::` |
| `MailChannel`, `MailRendering`, `NotificationMailable` | `suprnova::` |
| `register_mail_renderer::<N>()` | `suprnova::` |
| `DatabaseChannel`, `StoredNotification` | `suprnova::` |
| `WebPushChannel` | `suprnova::` |
| `BroadcastChannel` | `suprnova::` |
| `SendNotificationJob` | `suprnova::` |
| `NotificationSending`, `NotificationSent`, `NotificationFailed` | `suprnova::` |
| `set_dispatcher`, `register_notification_factory` | `suprnova::notifications::` |
| `all_for`, `unread_for`, `read_for`, `mark_as_read`, `mark_as_unread`, `mark_all_as_read`, `delete_for` | `suprnova::notifications::` |
| `assert_sent`, `assert_sent_named`, `assert_sent_times`, `assert_sent_to`, `assert_sent_to_on`, `assert_nothing_sent`, `assert_nothing_sent_to`, `assert_count`, `recorded_notifications` | `suprnova::notifications::` |
| `#[derive(NotificationMailable)]` | `suprnova::` |

## 下一步

- [邮件](mail.md) - 邮件通道搭乘的那个传输方式和 `Mailable` 表面
- [广播](broadcasting.md) - 广播通道用来发布的那个 `BroadcastHub`
- [Web 推送](web-push.md) - VAPID、加密、订阅存储
- [事件](events.md) - 监听 `NotificationSending` / `Sent` / `Failed`
- [队列](queues.md) - 驱动 `Notify::queue` 的那个工作进程
- [测试](testing.md) - 伪造实现表面与串行测试模式
