# 广播

广播，是构建在 Suprnova 的 [WebSocket 原语](websockets.md)之上的服务器到客户端通知层。您通过 `EventFacade` 分发一个 `Broadcastable` 事件；框架会把这个事件的 JSON 信封，扇出给这个事件命名的那些频道上的每一个 WebSocket
订阅者。您从不管理单个连接 - 您管理的是频道订阅，剩下的都交给这个中枢。

`BroadcastHub` 就是这条总线。默认的 `InMemoryBroadcastHub` 完全在进程内运行 - 对单副本部署和测试套件来说很合适。在 `broadcasting-fanout` 这个
Cargo feature 之后，`SeaStreamerBroadcastHub` 会把同样的这些事件，路由经过一个流代理（Redis Streams、Kafka、文件、stdio），这样一个进程里的一次发布，就能触达每一个其他进程里的订阅者。

[WebSocket](websockets.md) 那一章里的一切仍然适用 - 心跳 ping，
`max_missed_pings`，`WsConfig`，逐路由中间件，路径参数。广播只是在上面加了一个传输协议和一个频道注册表。

## 快速上手

四个文件，浏览器就能看到一个事件。

`src/channels/order_updates.rs`：

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
}
```

`src/events/order_placed.rs`：

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        vec!["order.updates".into()]
    }
}
```

`src/bootstrap.rs`：

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::container::App;
use suprnova::events::EventFacade;

pub async fn register() {
    // 1. 把这个中枢绑在这个 trait 背后 - 处理程序会统一地解析它。
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    // 2. 提前注册每一个频道；WS 处理程序会按名字解析。
    let mut registry = ChannelRegistry::new();
    registry.register(OrderUpdates);
    App::singleton(Arc::new(registry));

    // 3. 每个 Broadcastable 类型接好一次事件 → 中枢的桥接。
    EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
}
```

`src/routes.rs` - 通过从容器里解析出已经 bootstrap 好的中枢和注册表，为每条路由构建一个 `BroadcastingWsHandler`：

```rust
use std::sync::Arc;
use suprnova::broadcasting::{
    BroadcastHub, BroadcastingWsHandler, ChannelRegistry, InMemoryBroadcastHub,
};
use suprnova::container::App;
use suprnova::{routes, ws, AuthMiddleware};

fn broadcasting_handler() -> BroadcastingWsHandler {
    // 容器优先；回退到一个全新的进程内中枢 + 空注册表，
    // 这样那些不经过 bootstrap 就组装路由器的单元测试仍然能工作。
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .unwrap_or_else(|| Arc::new(InMemoryBroadcastHub::new()));
    let registry: Arc<ChannelRegistry> = App::get::<Arc<ChannelRegistry>>()
        .unwrap_or_else(|| Arc::new(ChannelRegistry::new()));
    BroadcastingWsHandler::new(hub, registry)
}

routes! {
    ws!("/ws/broadcast", broadcasting_handler())
        .middleware(AuthMiddleware::new()),
}
```

连接，并观察：

```bash
wscat -c ws://localhost:3000/ws/broadcast
> {"action":"connected","socket_id":"6f1a3c2e-…"}
> {"action":"subscribe","channel":"order.updates","data":{}}
< {"action":"subscribed","channel":"order.updates"}
```

从任何控制器、工作进程，或者调度任务分发：

```rust
EventFacade::dispatch(OrderPlaced { order_id: 99, user_id: 42 }).await?;
```

```
< {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}
```

## 频道

一个频道，是一个具名的订阅目标。客户端按名字订阅；这个中枢会把事件投递给那个名字下的每一个活跃订阅者。`Channel` trait 有着不对称的默认值，在写上失败关闭，在读上失败开放 - 见下面的
[为什么 Suprnova 有所不同](#为什么-suprnova-有所不同)。

### 公开频道

默认情况。任何客户端都可以订阅。

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
    // authorize() 默认是 true - 对所有订阅者开放。
}
```

### 私有频道

覆盖 `authorize`，来给订阅把关。一次被拒绝的订阅，会产出一个带
`reason: "unauthorized"` 的 `error` 帧；不会发送任何 `subscribed` 帧。

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct PrivateChat;

#[async_trait]
impl Channel for PrivateChat {
    fn name(&self) -> &'static str { "chat.private" }

    async fn authorize(
        &self,
        _req: &Request,
        _params: &ChannelParams,
        data: &Value,
    ) -> bool {
        data["token"].as_str().map(|t| t == "valid").unwrap_or(false)
    }
}

impl PrivateChannel for PrivateChat {}
```

`data` 是客户端在这个订阅帧的 `data` 字段里发送的任何东西 - 一个 bearer
令牌，一个签署过的频道绑定，任何应用自定义的东西。`Request` 是原始的
HTTP 升级请求（请求头和 cookie 都可以直接读取）。`params` 携带的是从一个参数化名字里捕获到的值，对固定名字来说是空的。

`PrivateChannel` 是一个标记 trait。框架不会在运行时检查它 - 它是一个类型层面的信号，表示这个频道覆盖了 `authorize`，是为将来的工具准备的（一个 clippy lint，一次审计遍历）。

### 参数化频道

在 `name()` 里嵌入 `{param}` 片段，一次注册就能服务每一个匹配这个模式的具体订阅 - 和 Laravel 的 `Broadcast::channel('orders.{id}', …)` 是同一个模型。被捕获的值，会作为一个 `ChannelParams` 映射，抵达每一个钩子。

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct OrderChannel;

#[async_trait]
impl Channel for OrderChannel {
    fn name(&self) -> &'static str { "orders.{id}" }

    async fn authorize(
        &self,
        _req: &Request,
        params: &ChannelParams,
        _data: &Value,
    ) -> bool {
        let order_id = params.get("id").unwrap_or_default();
        // 依据被捕获的 id 来把关 - 会话用户拥有这个订单吗？
        !order_id.is_empty()
    }
}

impl PrivateChannel for OrderChannel {}

// 一次注册，服务 orders.42、orders.99、orders.featured……
registry.register(OrderChannel);
```

每一个 `{param}` 恰好绑定一个点分片段：`orders.{id}` 匹配 `orders.42`，但不匹配 `orders` 或者 `orders.42.line`。解析会优先选择一个精确的固定名字注册，而不是任何模式（对那一个名字，`orders.featured` 会胜过
`orders.{id}`），然后是最具体的模式（字面片段最多的），用字典序最小的模式，作为一个确定性的平局判定。

### 呈现频道

呈现频道会跟踪成员情况。当一个客户端订阅时，这个中枢会给这个客户端投递一份 `presence.here` 快照，并向每一个其他订阅者广播 `presence.joined`。当一个客户端离开时，这个中枢会广播 `presence.left`。

这个两部分的契约，很容易只实现一半：您必须同时覆盖
`Channel::presence_info`（让它返回 `Some(self)`）**并且**实现
`PresenceChannel::member_info`。忘掉 `presence_info`，会把这个频道接成一个非呈现频道 - 订阅能工作，但 `presence.joined` / `presence.here` /
`presence.left` 永远不会触发。

```rust
use async_trait::async_trait;
use serde_json::{json, Value};
use suprnova::FrameworkError;
use suprnova::broadcasting::{Channel, ChannelParams, PresenceChannel};
use suprnova::http::Request;

pub struct PresenceLobby;

#[async_trait]
impl Channel for PresenceLobby {
    fn name(&self) -> &'static str { "presence.lobby" }

    // 必需的 - 没有这个覆盖，PresenceChannel 会被接上，但处于惰性状态。
    fn presence_info(&self) -> Option<&dyn PresenceChannel> {
        Some(self)
    }
}

#[async_trait]
impl PresenceChannel for PresenceLobby {
    async fn member_info(
        &self,
        _req: &Request,
        _params: &ChannelParams,
    ) -> Result<Value, FrameworkError> {
        // 返回其他订阅者需要用来识别这个成员的东西 -
        // 通常是一个用户 id。永远不要包含机密信息或私人 PII。
        Ok(json!({ "user_id": 42, "display_name": "Alice" }))
    }
}
```

关于完整的事件流程和自连接回显，请参见[呈现](#呈现)。

### 保留名字

以 `__` 开头的名字，是为框架的元频道保留的（`__presence__` 承载着跨进程的呈现复制）。对一个 `__` 前缀的名字调用 `registry.register(channel)`
会在注册时 panic，这样这个错误会在启动时被捕获，而不是在运行时。

### 为什么 Suprnova 有所不同

Laravel 把频道授权绑定到一个 `$user` 回调参数上，因为 PHP 会隐式地注入当前已认证的用户。Suprnova 的 `authorize` 则接受原始的 `Request`、被捕获的 `ChannelParams`，以及一个任意的 `data: Value` - 三个正交的输入，全都可用，没有任何隐式的上下文。您从 `Request` 里读取会话 cookie 或者
bearer 令牌，从 `ChannelParams` 里读取路由风格的参数；`data` 载荷是一个自由的位置，留给客户端在订阅时提供的令牌。

`Channel` trait 的默认值，是**故意不对称的**：`authorize` 默认是
`true`（订阅默认是公开的），`authorize_publish` 默认是 `false`（客户端发起的发布，默认是被拒绝的）。危险的操作失败关闭；安全的操作失败开放。如果没把握，就两个都别动。

## Broadcastable trait

`Broadcastable: Event + Serialize` - 每一个 `Broadcastable`，也都是一个
`Event`。通过 `EventFacade::dispatch(event)` 分发，会运行每一个进程内监听器，**并且**把这个 JSON 序列化的载荷，推送给这个事件命名的那些频道上的每一个 WebSocket 订阅者。

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        // 一个事件，多个频道。每个频道上的每一个订阅者，
        // 都会收到同一个信封。
        vec![
            format!("user.{}.orders", self.user_id),
            "orders.global".into(),
        ]
    }
}
```

在启动时，为每个 Broadcastable 类型接好一次这个桥接：

```rust
EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
```

在那之后，`EventFacade::dispatch(event).await?` 就是发送这一侧的全部了 -
不需要一次单独的 `publish` 调用。

默认情况下，这个事件通过 `serde_json::to_value(&event)` 序列化，并推送给每一个订阅者。在进程内中枢上，零订阅者的频道会被静默地跳过；跨进程的中枢仍然会发布它们，这样其他进程就有机会去投递。

四个可选的方法，会细化这个默认行为：

**`broadcast_event_name(&self) -> &'static str`** - 覆盖这个传输格式里的事件名。默认是 `Self::event_name()`。用它来把进程内的事件身份，和线上的名字解耦开。

**`broadcast_with(&self) -> Option<Value>`** - 返回 `Some(value)`，来推送一个精心挑选的载荷，而不是完整的事件序列化结果（Laravel 的
`broadcastWith()`）。在不改变事件类型的前提下，省略机密信息，或者为客户端重塑形状：

```rust
impl Broadcastable for AccountFunded {
    fn broadcast_on(&self) -> Vec<String> {
        vec![format!("account.{}", self.account_id)]
    }
    fn broadcast_with(&self) -> Option<serde_json::Value> {
        // 永远不要把这个余额放进发给客户端的响应里 - 只放公开 id。
        Some(serde_json::json!({ "account_id": self.account_id }))
    }
}
```

**`broadcast_when(&self) -> bool`** - 返回 `false`，来把这个事件分发给进程内的监听器，但跳过这次 WebSocket 推送（Laravel 的
`broadcastWhen()`）。只有这次广播会被把关；事件流水线的其余部分照常运行：

```rust
impl Broadcastable for DraftSaved {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("doc.{}", self.doc_id)] }
    fn broadcast_when(&self) -> bool { self.publish } // 只在发布时广播
}
```

**`broadcast_to_others(&self) -> bool`** - 返回 `true`，来排除触发这次广播的那个连接（Laravel 的 `toOthers()`）。框架会在连接时，为每一个广播连接分配一个 `socket_id`（在 `connected` 帧里发送）；浏览器会把它作为 HTTP 请求上的 `X-Socket-ID` 请求头回显；在处理那个请求期间分发的一个 `broadcast_to_others` 事件，会跳过发起请求的那个连接。在请求之外（一个工作进程或作业），或者没有 `X-Socket-ID` 存在时，它会退化成向所有人广播：

```rust
impl Broadcastable for MessagePosted {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("chat.{}", self.room)] }
    fn broadcast_to_others(&self) -> bool { true } // 发送者已经有了这个
}
```

这是一个逐事件类型的选择。要做逐分发的排除，就直接发布：

```rust
use suprnova::broadcasting::BroadcastEnvelope;

hub.publish(
    BroadcastEnvelope::new(channel, event, data).with_except(socket_id),
).await?;
```

### 与相邻监听器的分发顺序

`EventFacade::dispatch` 是**快速失败**的：如果一次中枢发布返回了
`Err`（比如跨进程中枢上的一次代理断连），`BroadcastListener` 就会返回
`Err`，而任何在它**之后**注册的相邻监听器都不会运行。有两种方式来处理这一点：

- 把这个广播桥接，注册在那些副作用（数据库写入，日志发出）必须无论广播结果如何都要运行的进程内监听器**之后**。
- 当每一个监听器都必须运行、无论其中一个是否返回 `Err` 时，切换到
  `EventFacade::dispatch_best_effort(event)`。

内存中枢永远不会返回 `Err` - 只有跨进程的变体，会暴露代理的失败。

## 传输协议

广播路由上的每一条消息，都是一个 UTF-8 JSON 帧。两种形状：
`ClientFrame`（客户端 → 服务器）和 `ServerFrame`（服务器 → 客户端）。

### 客户端帧

| `action` | 必需字段 | 可选字段 | 含义 |
|----------|-----------------|-----------------|---------|
| `subscribe` | `channel` | `data` | 订阅 `channel`。`data` 会被转发给 `Channel::authorize`。 |
| `unsubscribe` | `channel` | | 从 `channel` 上分离。 |
| `publish` | `channel`、`event`、`data` | | 把一个事件推送给 `channel` 上的每一个订阅者。受 `Channel::authorize_publish` 把关，**并且**要求一个活跃的订阅。 |

客户端发起的 `publish`，受**两项**检查把关：这个连接必须持有对目标频道的一个已授权订阅，**并且** `Channel::authorize_publish` 必须返回
`true`（它默认是 `false`）。这镜照的是 Pusher 的客户端事件契约 - 想要客户端发布的频道，通过覆盖这个钩子来显式地选择加入。大多数服务器端的广播频道，永远不想要客户端发起的事件，而这个默认拒绝的形状，正好匹配这个意图。

```json
{"action":"subscribe","channel":"chat.42","data":{"token":"abc"}}
{"action":"unsubscribe","channel":"chat.42"}
{"action":"publish","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
```

### 服务器帧

| `action` | 字段 | 含义 |
|----------|--------|---------|
| `connected` | `socket_id` | 只发送一次，最先发送。把 `socket_id` 作为 `X-Socket-ID` HTTP 请求头回显，这样服务器端的 `broadcast_to_others` 就能排除这个连接。 |
| `subscribed` | `channel` | 订阅被接受了。 |
| `unsubscribed` | `channel` | 取消订阅已确认。 |
| `event` | `channel`、`event`、`data` | 一个事件被广播到了 `channel` 上。 |
| `lagged` | `channel`、`skipped` | 这个订阅者落后于服务器逐频道的环形缓冲区，`skipped` 个信封在这个连接上被丢弃了。`channel` 上的客户端本地状态已经陈旧；在处理更多事件之前，先重新拉取。 |
| `error` | `channel`（可为 null）、`reason` | 上一个操作失败了。对于不绑定某个频道的信封层面的错误，`channel` 是 `null`。 |

```json
{"action":"connected","socket_id":"6f1a3c2e-…"}
{"action":"subscribed","channel":"chat.42"}
{"action":"unsubscribed","channel":"chat.42"}
{"action":"event","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
{"action":"lagged","channel":"chat.42","skipped":42}
{"action":"error","channel":"chat.42","reason":"unauthorized"}
{"action":"error","channel":null,"reason":"malformed envelope: …"}
```

#### 关于 `lagged`

每一个频道都有一个逐进程的环形缓冲区（256 个信封）。一个排空速度不够快的订阅者 - 一个慢客户端，一个卡住的转发器 - 会落后，而这个缓冲区会覆写最旧的那些事件。发生这种情况时，服务器会发送一个命名了这个频道和被丢弃事件数量的 `lagged` 帧，然后照常继续投递后续的帧。这个缺口，在服务器一侧**不可**恢复；客户端必须在处理这个频道上更多的事件之前，重新拉取或者重新同步。静默地丢弃事件，会让 bug 藏在「我们丢了一个 tick」这种说法背后，而不是「客户端的状态和服务器的分歧了」。

#### 发布失败

当一次客户端发起的 `publish` 被 `authorize_publish` 接受了，但这次中枢发布本身失败了（跨进程中枢上的一次代理断连）时，发起的客户端会收到一个带 `reason: "publish failed: …"` 的 `error` 帧，这样它就知道这个事件没能触达其他进程。其他订阅者不会被通知。

### 示例会话

```
S → C  {"action":"connected","socket_id":"6f1a3c2e-…"}
C → S  {"action":"subscribe","channel":"order.updates","data":{}}
S → C  {"action":"subscribed","channel":"order.updates"}

# 服务器分发 OrderPlaced：
S → C  {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}

C → S  {"action":"subscribe","channel":"chat.private","data":{"token":"bad"}}
S → C  {"action":"error","channel":"chat.private","reason":"unauthorized"}

C → S  {"action":"unsubscribe","channel":"order.updates"}
S → C  {"action":"unsubscribed","channel":"order.updates"}
```

## 逐路由中间件

广播路由支持和纯 WebSocket 路由一样的 `.middleware(M)` 链式调用：

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new()),
```

任何中间件给出的非 2xx 响应，都会让这次升级短路 - 客户端会收到这个 HTTP
错误响应，不会发生任何 WebSocket 握手。这是强制执行传输层面认证（会话有效性，来源检查，连接时的限流）的正确位置，不必在每一个频道的
`authorize` 里重复这个检查。

多个中间件从左到右组合：

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

这个拆分是刻意的：**传输层面**的（究竟谁可以打开这个连接）活在中间件里；**频道层面**的（谁可以订阅哪个频道）活在 `Channel::authorize` 里。

### 逐路由的 `WsConfig`

逐路由地覆盖这个进程范围的 WebSocket 默认值。在处理程序之后链式调用
`.config(WsConfig { ... })` - 在 `.middleware(M)` 之前或之后都行（顺序不重要）：

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

ws!("/ws/chat", broadcasting_handler())
    .config(WsConfig {
        ping_interval: Duration::from_secs(5),
        max_missed_pings: 1,
        ..Default::default()
    })
    .middleware(AuthMiddleware::new())
```

这五个可配置的字段，以及每一个各自在什么地方要紧：

| Field | 默认值 | 使用场景 |
|-------|---------|----------|
| `ping_interval` | 30s | 聊天 / 呈现：缩短到 5-10 秒，来快速侦测死掉的移动端连接。批量数据流：加长它，来减少开销。 |
| `max_missed_pings` | 2 | 对于一次缺失的 Pong 就该立刻关闭的聊天场景，设为 `1`。对不稳定的移动网络，设为 `3+`。设为 `usize::MAX`，来禁用无 pong 时关闭。 |
| `max_message_size` | 1 MiB | 对公开端点安全的默认值。对受信任的内部数据源，从 `WsConfig::generous()`（64 MiB）开始。 |
| `max_frame_size` | 64 KiB | 为带余量的聊天 / 通知帧定的大小。对大型的、不分片的帧，从 `WsConfig::generous()`（16 MiB）开始。 |
| `origin_policy` | `SameOrigin` | 默认值会拒绝跨来源的升级 - 这是浏览器 WS 握手拥有的唯一 CSRF 防护。对显式的跨来源前端，用 `AllowList(vec![...])`；只对非浏览器端点，用 `AllowAny`。 |

当没有提供 `.config(...)` 时，这条路由会继承 `WsConfig::default()`。显式的逐路由配置，总是会胜过这个默认值。

对于服务受信任的内部数据源（服务器到服务器的扇出，大型二进制传输）的路由，就从这个受信任数据源的工厂开始，再按需调整：

```rust
use suprnova::ws::WsConfig;
use std::time::Duration;

ws!("/ws/internal/firehose", FirehoseHandler::new())
    .config(WsConfig {
        ping_interval: Duration::from_secs(10),
        ..WsConfig::generous() // 64 MiB 消息 / 16 MiB 帧
    })
```

## 呈现

当一个客户端成功订阅一个呈现频道时，这个中枢会：

1. 用这个升级 `Request` 和被捕获的 `ChannelParams`，调用
   `PresenceChannel::member_info`，来收集这个正在加入的成员的数据。
2. 给这个新订阅者，发送一个带 `data: { "members": [...] }` 的
   `presence.here` 事件帧 - 所有当前被跟踪成员的一份快照（不包括这个刚加入的）。
3. 向这个频道发布一个带 `data: <member_info>` 的 `presence.joined` 事件。每一个订阅者 - 包括这个新的、通过它自己的转发器 - 都会收到它；客户端通过把这个加入成员的身份和自己的比较，来过滤掉自连接。

当一个订阅者断开连接，或者发送一个取消订阅帧时：

4. 这个中枢会发布一个带这个离开成员数据的 `presence.left` 事件。每一个剩下的订阅者都会收到它。

这三个帧，都会以带保留 `event` 名的 `event` action 帧的形式抵达：

```json
{"action":"event","channel":"presence.lobby","event":"presence.here","data":{"members":[{"user_id":1},{"user_id":2}]}}
{"action":"event","channel":"presence.lobby","event":"presence.joined","data":{"user_id":3}}
{"action":"event","channel":"presence.lobby","event":"presence.left","data":{"user_id":3}}
```

跨进程时，呈现状态是通过保留的 `__presence__` 元频道复制的（参见
[跨进程扇出](#跨进程扇出)）。任何进程上的跟踪和取消跟踪操作，都会传播给所有订阅者；`list_members` 返回的是合并后的视图（本地 + 远程）。那些
`untrack_member` 从未触发过的、已经死掉的进程，它们的成员会通过 TTL 被修剪掉 - 默认 60 秒。

## 跨进程扇出

默认的 `InMemoryBroadcastHub` 只会扇出给当前进程上的订阅者。对于多副本部署，请启用 `broadcasting-fanout` 这个 Cargo feature，并换上 `SeaStreamerBroadcastHub`：

`Cargo.toml`：

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.3", features = ["broadcasting-fanout"] }
```

`src/bootstrap.rs`：

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry};
use suprnova::broadcasting::fanout::SeaStreamerBroadcastHub;
use suprnova::container::App;

pub async fn register() {
    let hub: Arc<dyn BroadcastHub> = Arc::new(
        SeaStreamerBroadcastHub::new(
            "redis://broker:6379",   // streamer URI（后端按协议方案选出）
            "suprnova-broadcast",    // 流键（集群里每一个进程共用）
        )
        .await
        .expect("connect"),
    );
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));
    // ……bootstrap 的其余部分不变
}
```

这个构造函数接受两个参数：streamer URI（在运行时按协议方案选择后端）和流键（集群里每一个进程共享的那个主题名字）。请在每一个副本上使用同一个流键，否则它们看不到彼此的事件。

`new_with_presence_ttl(uri, key, ttl)` 会覆盖默认的 60 秒呈现 TTL - 这对那些需要快速演练崩溃恢复路径的测试很有用。`new_loopback(uri, key)` 会为单进程的集成测试启用 stdio 环回；那道重复防护会确保每一个应用事件在本地仍然恰好投递一次。

### 后端

后端是在运行时根据 URI 的协议方案选出来的：

| URI 协议方案 | 后端 | 可用于生产 | 备注 |
|------------|---------|------------------|-------|
| `redis://`、`rediss://` | Redis Streams | **是** | 默认推荐。`rediss://` 使用 TLS。默认构建里已启用。 |
| `kafka://`、`kafka+ssl://` | Kafka | **是** | 需要 `sea-streamer` 的 feature 集合里带上 `kafka`（`framework/Cargo.toml`）。 |
| `stdio://` | stdin/stdout 管道 | 否 - 仅供测试 | 单进程环回。 |
| `file://` | 本地文件 | 否 - 单主机 | 需要 `sea-streamer` 的 feature 集合里带上 `file`。 |

Suprnova 的默认构建启用了 `stdio` + `redis` + `socket`。要启用 Kafka 或 file，请编辑 `framework/Cargo.toml`，加上相应的 `sea-streamer` feature。

### 架构

每一次 `publish(envelope)` 会并行做两件事：

1. **本地扇出** - 内层的 `InMemoryBroadcastHub` 会立即投递给这个进程上的订阅者。本地订阅者从不等待网络。
2. **写入流** - 同一个信封会被序列化并推送到 sea-streamer 的流里，这样其他每一个进程的消费者泵都会取到它，并在本地投递。

一道重复投递防护会避免同一个应用数据事件被看到两次：这个中枢实例有一个随机 UUID，它产出的每一个信封都带着那个 UUID，而消费者泵会跳过那些实例 id 与本地中枢自己相同的入站信封。呈现元频道的消息是个例外 - 每个中枢都需要在跨进程视图里看到自己的事件，这样读取路径才是统一的。

后端分发是基于枚举的，不是 trait 对象：这个中枢存放的是一个来自 sea-streamer socket 适配器的具体 `SeaProducer` / `SeaConsumer`，它本身就是一个覆盖每一个已编译后端的枚举。发布调用点上没有 `dyn` 开销。

### 跨进程呈现

`SeaStreamerBroadcastHub` 会自动把呈现状态复制到各个进程。每个实例在构造时都有一个 UUID `instance_id`；`track_member` / `untrack_member` 会把 `PresenceEvent` 发布到保留的 `__presence__` 元频道上。每个进程都维护一份 `cross_process_view`，由它自己的消费者任务更新；`list_members` 返回合并后的视图（本地和远程一视同仁）。

存活性：每个进程都会每隔 `ttl / 6`（在默认的 60 秒 TTL 下就是 10 秒）把自己的成员重新发布一次，作为心跳。陈旧的条目 - 那些 `last_seen` 已经超出 TTL 的成员 - 会每隔 `ttl / 2` 被修剪掉。这处理的是那些还没来得及发布 `MemberRemoved` 就崩溃了的进程。

## 无 pong 时关闭

广播路由参与的，是和纯 `ws!` 路由一样的 WebSocket 心跳。框架每
`WsConfig::ping_interval`（默认 30 秒）发送一次 Ping。如果一个连接在
`max_missed_pings` 个连续的间隔内（默认 2 个），都没能用一个 Pong 响应，框架就会以码 1011 关闭。

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let config = WsConfig {
    ping_interval: Duration::from_secs(15),
    max_missed_pings: 3,
    ..WsConfig::default()
};
```

调低 `ping_interval`，能更快侦测到死连接，代价是更高的基准流量。
`max_missed_pings: 1`，会在第一次缺失的 Pong 之后就关闭 - 只有在网络故障很罕见、并且您想要尽可能快的死连接清理时，才用这个。
`max_missed_pings: usize::MAX`，会完全禁用无 pong 时关闭。

## 生产部署

广播路由，是您 HTTP 路由所在的同一个 hyper 监听器上，升级而来的 HTTP 连接。TLS 终结发生在上游，和
[WebSocket 那一章](websockets.md#production-deployment)里描述的完全一样。那一章里的 nginx 和 Caddy 配置，原样适用 - 扩展它们来覆盖
`/ws/broadcast` 这条路径。

活跃的 WebSocket 处理程序任务（包括广播连接），会在框架的 `WS_TASKS`
集合里被跟踪，并在优雅关闭时被排空，所以飞行中的事件投递，会在进程退出之前完成。

## 测试广播

`RecordingBroadcastHub` 是 Suprnova 对 Laravel `Broadcast::fake()` 的对应物 - 一个会记录每一个已发布信封、同时仍然投递给活跃订阅者的
`BroadcastHub`。在测试里把它绑在 `InMemoryBroadcastHub` 的位置上，就能在不必先订阅的情况下，断言广播过什么：

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, RecordingBroadcastHub};
use suprnova::container::App;

#[tokio::test]
async fn shipping_an_order_broadcasts_to_the_user_channel() {
    let hub = Arc::new(RecordingBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub) as Arc<dyn BroadcastHub>);

    // ……运行发布的代码（直接发布，或者通过一个被分发的 Broadcastable）……

    hub.assert_broadcast("orders.42", "OrderShipped");
    assert_eq!(hub.count(), 1);
}
```

| 辅助函数                        | 断言的内容                                                  |
|--------------------------------|----------------------------------------------------------|
| `assert_broadcast(ch, ev)`     | `ch` 上至少有一个事件名为 `ev` 的信封       |
| `assert_nothing_broadcast()`   | 没有任何东西被发布过                    |
| `broadcasts()`                 | `Vec<BroadcastEnvelope>` - 每一个被记录的信封       |
| `count()`                      | 记录的信封总数                                 |

要断言一个 `Broadcastable` *事件*到底有没有被分发过（而不是什么东西到达了传输格式），`EventFacade::fake()` 会记录这个事件本身 - 参见
[事件](events.md#testing--eventfacadefake)。

## Laravel 对等参考

| Laravel | Suprnova |
|---------|----------|
| `Broadcast::channel('name', fn(...))` | `Channel` trait impl + `registry.register(...)` |
| `Broadcast::channel('orders.{id}', ...)` | `fn name() -> "orders.{id}"`，参数在 `ChannelParams` 里 |
| `PrivateChannel`（接口） | `PrivateChannel` 标记 trait + 覆盖 `authorize` |
| `PresenceChannel`（接口） | `PresenceChannel` + 覆盖 `Channel::presence_info` |
| `ShouldBroadcast`（接口） | `Broadcastable` trait |
| `broadcastOn()` | `broadcast_on(&self) -> Vec<String>` |
| `broadcastAs()` | `broadcast_event_name(&self) -> &'static str` |
| `broadcastWith()` | `broadcast_with(&self) -> Option<Value>` |
| `broadcastWhen()` | `broadcast_when(&self) -> bool` |
| `toOthers()` | `broadcast_to_others(&self) -> bool` |
| `Broadcast::fake()` | `RecordingBroadcastHub` 绑定为 `dyn BroadcastHub` |
| `assertBroadcasted` | `RecordingBroadcastHub::assert_broadcast(channel, event)` |
| Pusher / Reverb / Ably 驱动程序 | `InMemoryBroadcastHub`（单进程）或者 `SeaStreamerBroadcastHub`（跨进程：Redis / Kafka / file / stdio） |
| Echo 客户端库 | 未提供 - 目前得自己动手，从浏览器接好这个 JSON 信封协议 |

## 参考

| 符号 | 用途 |
|--------|---------|
| `suprnova::broadcasting::Channel` | Channel trait。覆盖 `name()`（必需）、`authorize`、`authorize_publish`、`presence_info`。 |
| `suprnova::broadcasting::ChannelParams` | 从一个参数化的 `name()` 里捕获到的值。`get(key) -> Option<&str>`。对固定名字是空的。 |
| `suprnova::broadcasting::PrivateChannel` | 一个覆盖了 `authorize` 的 `Channel` 上的标记 trait。没有必需的方法。 |
| `suprnova::broadcasting::PresenceChannel` | `async fn member_info(req, params) -> Result<Value, FrameworkError>`。要求覆盖 `Channel::presence_info`。 |
| `suprnova::broadcasting::ChannelRegistry` | 持有每一个已注册的频道。在容器里绑定为 `Arc<ChannelRegistry>`；由 `BroadcastingWsHandler` 解析。 |
| `suprnova::broadcasting::Broadcastable` | 作用在 `Event + Serialize` 上的 trait。必需：`broadcast_on()`。可选：`broadcast_event_name`、`broadcast_with`、`broadcast_when`、`broadcast_to_others`。 |
| `suprnova::broadcasting::BroadcastHub` | 中枢 trait。`subscribe`、`publish`、`subscriber_count`，呈现的跟踪/取消跟踪/列举。 |
| `suprnova::broadcasting::InMemoryBroadcastHub` | 默认的进程内中枢。没有外部依赖。`publish` 无条件返回 `Ok`。 |
| `suprnova::broadcasting::RecordingBroadcastHub` | 测试替身。记录每一次发布；仍然投递给活跃订阅者。 |
| `suprnova::broadcasting::BroadcastEnvelope` | 一个已发布的事件：`channel`、`event`、`data`、`except`。`new(ch, ev, data)` 构建器；`.with_except(socket_id)` 用于逐分发的排除。 |
| `suprnova::broadcasting::ClientFrame` / `ServerFrame` | 这个 JSON 信封的传输格式类型。`ServerFrame::Lagged { channel, skipped }` 暴露逐频道的环形缓冲区溢出。 |
| `suprnova::broadcasting::BroadcastingWsHandler` | 框架可复用的 `WebSocketHandler`。构造函数：`BroadcastingWsHandler::new(hub, registry)`。传给 `ws!()`。 |
| `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub` | `broadcasting-fanout` 之后的跨进程中枢。`new(uri, stream_key)`、`new_with_presence_ttl(uri, key, ttl)`、`new_loopback(uri, key)`。 |
| `EventFacade::broadcast::<E>(hub)` | 为 `E` 注册这个事件 → 中枢的桥接。每个 `Broadcastable` 在启动时调用一次。 |
| `EventFacade::dispatch(event)` | 触发进程内的监听器，**并且**在 `E::broadcast_on()` 返回的每一个频道上，发布到这个中枢。 |
| `WsRouteDef::config(WsConfig)` | 逐路由的 WS 配置覆盖。以任意顺序和 `.middleware(M)` 组合。 |
| `WsRouteDef::middleware(M)` | 逐路由的中间件链。一个非 2xx 响应会让这次升级短路。 |
| `WsConfig::generous()` | 受信任数据源的工厂：64 MiB 消息 / 16 MiB 帧，其他字段不变。不要在公开路由上使用。 |

## 下一步

- [WebSocket](websockets.md) - 底层的原语，`WsSocket`，`OriginPolicy`
- [事件](events.md) - `EventFacade`，快速失败对比尽力而为的分发
- [Server-Sent 事件](sse.md) - 没有 Upgrade 握手的单向推送
- [通知](notifications.md) - `BroadcastChannel` 这个通知驱动程序
- [Web 推送](web-push.md) - 向离线用户发送的服务器推送通知
