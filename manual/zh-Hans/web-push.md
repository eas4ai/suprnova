# Web 推送

即便您的站点已经关闭，Web 推送也能把一条简短的消息送到浏览器 - Service
Worker 会被唤醒，解密这个载荷，并展示一个操作系统层面的通知。Suprnova 端到端地提供了这套协议：VAPID 密钥生成，AES128GCM 载荷加密，HTTP 传输，以及一个接入通知子系统的 `WebPushChannel`，这样您发给邮件或数据库的同一个
`Notification`，也会作为一次推送落地。

当您想在没有打开的 WebSocket 的情况下，实时提醒用户时，就伸手去拿它 - 订单已发货，好友请求，被提及，余额到账。如果用户用的是桌面浏览器，并且站点已经关闭，Web 推送是唯一能触达他们的机制；如果他们正在站点上，
[广播](broadcasting.md) 通常是更好的选择。

这个 API 位于 `web-push` 这个 Cargo feature 之后，它默认是启用的。使用
`default-features = false` 的应用，必须显式地启用 `web-push`。

## 这四个部分

Web 推送的活动部件比邮件或数据库更多，因为这份规范（[RFC 8030](https://datatracker.ietf.org/doc/html/rfc8030) +
[RFC 8291](https://datatracker.ietf.org/doc/html/rfc8291) +
[RFC 8292](https://datatracker.ietf.org/doc/html/rfc8292)）把身份、加密和传输拆到了三个契约里：

| 部分 | 是什么 |
|---|---|
| `VapidKey` / `VapidSigner` | 一个 P-256 ECDSA 密钥对，用来签署证明您的服务器确实是它所声称的那个身份的 JWT |
| `WebPushClient` | 加密一个载荷、签署一个 VAPID JWT，并把它 POST 到这个订阅的端点的那个 HTTP 客户端 |
| `WebPushChannel` | 把一个 `Notification` 转换成一次 `WebPushClient::send` 调用的通知子系统适配器 |
| `SubscriptionInfo` | 用户订阅时浏览器交给您的那个不透明的 (`endpoint`、`p256dh`、`auth`) 三元组 - 您存储它；您不生成它 |

最下面这三层 - `VapidKey`、`WebPushClient`、加密的 POST - 都从
`suprnova::web_push` 重新导出，所以应用永远不需要直接依赖底层的
`suprnova-web-push` crate。

## 生成一个 VAPID 密钥对

Web 推送用 VAPID（自愿应用服务器标识）来让推送服务能对行为不端的发送者做限流并联系到他们。每个应用您需要一个 P-256 密钥对；公钥进入您的前端，这样浏览器就能把订阅绑定到您的服务器上，私钥则留在服务器上签署 JWT。

生成一次，把它持久化，然后永远复用它：

```rust
use suprnova::VapidKey;

let key = VapidKey::generate();

// 把这个 PEM 保存在某个持久的地方 - 一个密钥管理器，一个部署流水线
// 挂载的文件，一个用环境变量充当文件的卷。您没法重新生成它，除非让
// 每一个现有的订阅都失效。
let pem = key.to_pem()?;
std::fs::write("vapid_private.pem", &pem)?;

// 前端需要的是 base64url、无填充、未压缩的公钥。
// 把它交给您的 JS，这样 `pushManager.subscribe()` 就能把它用作
// `applicationServerKey`。
println!("PUBLIC_VAPID_KEY={}", key.public_key_uncompressed_b64url());
```

在启动时，加载保存好的这份 PEM：

```rust
use suprnova::{VapidKey, VapidSigner};

let pem = std::fs::read_to_string("vapid_private.pem")?;
let key = VapidKey::from_pem(&pem)?;
let signer = VapidSigner::new(key);
```

一个 `VapidSigner` 会产出 JWT，但不发送任何东西 - 它纯粹是一个签名原语。下一层会包装它。

## 构建一个 WebPushClient

`WebPushClient` 是 HTTP 一侧的原语：喂给它一个签名器和一个联系 URI
（“推送服务在您行为不端时该怎么联系到您”），拿回一个对象，它的 `send`
方法会加密一个载荷，签署一个 JWT，并把它 POST 到这个订阅的端点。

```rust
use std::sync::Arc;
use suprnova::{VapidKey, VapidSigner, WebPushClient};

let signer = VapidSigner::new(VapidKey::from_pem(&pem)?);

// 按照 RFC 8292 §2.1，这个 subject 必须是一个 mailto: URI 或者一个
// https: URL。其他任何东西都会在构造时被拒绝，这样一次配置错误的部署
// 会在启动时就快速失败 - 而不是在第一次分发失败之后才悄无声息地暴露。
let client = WebPushClient::new(signer, "mailto:ops@example.org")?;

let client = Arc::new(client);
```

为什么是 `Arc<WebPushClient>`？`WebPushClient` 包装了一个 `VapidSigner`，它又包装了一个私有的 `ES256KeyPair`。这些都不是 `Clone` 的 - 私钥不该被随意复制 - 而给每一次通道注册都构造一个全新的签名器，就意味着同一个应用有 N 个独立的 VAPID 身份。包在 `Arc` 里，能让单一的一个已签署身份，为每一次注册和每一次并发投递撑腰。

### 端点策略

订阅端点是用户产生的数据：当一个用户订阅时，浏览器从一个远程推送服务那里收到这个 URL，而您的服务器会存储浏览器交回来的任何东西。一个被恶意存储的订阅，可以把这次 HTTP POST 指向任何可达的地方，把这个推送发送器变成一个 SSRF 工具。

`WebPushClient` 默认使用 `EndpointPolicy::Strict`：

- Scheme 必须是 `https`
- Host 必须是一个命名域名，不能是一个 IP 字面量
- 云元数据主机名，以及 RFC 2606 保留的 TLD（`.localhost`、`.local`、
  `.internal`、`.test`、`.example`、`.invalid`）都会被拒绝

这会挡住那些明显的 SSRF 探测，而不会破坏真实的推送服务（FCM、Mozilla
Autopush、苹果的 `web.push.apple.com`）。

对着一个 `wiremock` mock 服务器做本地集成测试时，您得选择退出：

```rust
use suprnova::{EndpointPolicy, WebPushClient};

let client = WebPushClient::new(signer, "mailto:test@example.org")?
    .with_endpoint_policy(EndpointPolicy::AllowAny);
```

不要在生产环境里使用 `AllowAny`。这些严格的检查存在的意义，就是防止一张被篡改的订阅表被武器化。

### 自定义传输

`WebPushClient::new` 会应用一个逐请求 30 秒的超时。如果您需要不同的传输策略 - 企业代理、固定的 TLS、更短的超时 - 请把一个 `reqwest::ClientBuilder` 传给 `WebPushClient::with_client_builder`。构建器的所有选项都会生效，但重定向策略会被强制禁用：已验证的端点即使返回 3xx，也绝不能把 POST 转发到未经验证的 URL，因此该库不会接受调用方的重定向设置。

```rust
use reqwest::Client;
use std::time::Duration;
use suprnova::WebPushClient;

let client = WebPushClient::with_client_builder(
    Client::builder().timeout(Duration::from_secs(10)),
    signer,
    "mailto:ops@example.org",
)?;
```

`WebPushClient::with_client` 接收一个已经构建好的客户端，该库无法检查它的重定向策略。在默认的 `Strict` 策略下，这类传输的发送会在任何 I/O 之前被拒绝。请切换到 `with_client_builder`；如果确认客户端不会跟随重定向，也可以用 `.allow_unconfined_redirects()` 显式接受该风险。

## 把 WebPushChannel 接入通知

裸的 `WebPushClient::send` 是可以工作的 - 但在 Suprnova 里，实际发送推送通知的方式，是通过[通知](notifications.md)子系统。一个 `Notification`
在它的 `channels()` 里声明 `vec!["webpush"]`，一个 `Notifiable` 收件人从 `route_for("webpush")` 返回一个 JSON 编码的 `SubscriptionInfo`，绑定好的 `NotificationDispatcher` 负责这次扇出。

```rust
use std::sync::Arc;
use suprnova::{
    NotificationDispatcher, WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};

let client: Arc<WebPushClient> = Arc::new(
    WebPushClient::new(signer, "mailto:ops@example.org")?
);

// ttl_secs：推送服务持有一条未投递消息的时长。
// 对于不紧急的通知，86_400（24 小时）是一个合理的默认值；
// 对于「立刻行动」这类提醒，把它降到 60 - 在这种场景下，
// 一条陈旧的消息比没有消息更糟。
let webpush = Arc::new(WebPushChannel::new(client, 86_400));

let dispatcher = NotificationDispatcher::new()
    .register_channel(webpush);

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` 在这个通道的 `name()` 上是后写入者胜出，所以测试可以换上一个替身，而不会影响生产环境的绑定。

## 定义一个通知

一个绑定推送的通知，形状和任何其他 Suprnova 通知一样 - 在 `channels()`
里声明 `"webpush"`，把您想投递的任何 JSON 放进 `data()`：

```rust
use serde::{Deserialize, Serialize};
use suprnova::Notification;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct OrderShipped {
    pub order_id: i64,
    pub tracking_url: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str {
        "OrderShipped"
    }

    fn channels(&self) -> Vec<&'static str> {
        vec!["webpush"]
    }

    fn data(&self) -> serde_json::Value {
        serde_json::json!({
            "title":   "Your order has shipped",
            "body":    format!("Track order #{}", self.order_id),
            "url":     self.tracking_url,
        })
    }
}
```

`data()` 里的 JSON，就是您的 Service Worker 会收到的东西。选定一个稳定的形状，并为前端把它记录下来 - Suprnova 不会强加一个，因为通知 UI 是前端的关切。

## 给收件人定路由

一个 `Notifiable` 会为它支持的每一个通道，返回对应的路由。对 Web 推送而言，这个路由就是那个 JSON 编码的 `SubscriptionInfo` - 恰好就是浏览器通过 `PushSubscription.toJSON()` 产出的东西，原样存储：

```rust
use suprnova::Notifiable;

pub struct User {
    pub id: i64,
    pub push_subscription_json: Option<String>,
}

impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "webpush" => self.push_subscription_json.clone(),
            _ => None,
        }
    }
}
```

返回 `None`，会让分发器静默地跳过这个通道 - 这对那些没有订阅推送、但仍然会收到邮件的用户很有用。

## 发送它

同步的：

```rust
use suprnova::Notify;

let user = User::find(42).await?.unwrap();
Notify::send(&user, &OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

已入队的 - 在入队时就预先解析出这个订阅的路由，这样工作进程就不需要重新加载这个用户：

```rust
Notify::queue(&user, OrderShipped {
    order_id: 1234,
    tracking_url: "https://ship.example.org/o/1234".into(),
}).await?;
```

要让 `Notify::queue` 能工作，就在启动时注册这个通知的工厂，这样工作进程就能把这个 JSON 载荷重新构建成这个类型化的通知：

```rust
suprnova::notifications::register_notification_factory::<OrderShipped>()?;
suprnova::queue::worker::register_job::<suprnova::SendNotificationJob>();
```

在幕后，已入队的分发会构建一个携带
`(notification_name, payload, per_channel_routes, channels)` 的
`SendNotificationJob`。工作进程会重新水化这个通知，在绑定好的分发器上按名字查找 `WebPushChannel`，并调用 `deliver(route, &notification)` -
和同步的 `Notify::send` 走的是同一条代码路径。

## 浏览器一侧

Suprnova 不提供一个 JavaScript SDK - 浏览器一侧就是纯粹的 Web 推送
API。您的前端需要实现的流程：

1. 注册一个 Service Worker。
2. 向用户请求权限。
3. 通过 `pushManager.subscribe({ userVisibleOnly: true,
   applicationServerKey: <your VAPID public key> })` 订阅。
4. 把 `subscription.toJSON()` POST 给一个把它存储在用户行上的
   Suprnova 端点。

```js
// Service Worker 注册（放在您应用入口的某个地方）
const registration = await navigator.serviceWorker.register('/sw.js');

if (Notification.permission === 'default') {
    await Notification.requestPermission();
}

if (Notification.permission === 'granted') {
    const subscription = await registration.pushManager.subscribe({
        userVisibleOnly: true,
        applicationServerKey: window.PUBLIC_VAPID_KEY,
    });

    await fetch('/api/push/subscribe', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(subscription.toJSON()),
    });
}
```

您的 Suprnova 端点接收这个 JSON，验证它的形状，并把它存储在这个用户上 -
这个字符串对您的服务器来说是不透明的，但它必须是浏览器产出的那份精确
JSON（`SubscriptionInfo` 类型之后会用 `Deserialize` 来解析它）：

```rust
use suprnova::{Auth, Request, Response, SubscriptionInfo, attrs, json_response};

pub async fn subscribe(req: Request) -> Response {
    let user_id = Auth::id().expect("auth middleware");

    let (_parts, bytes) = match req.body_bytes().await {
        Ok(b) => b,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };
    let raw = match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => return json_response!({ "error": "body not utf-8" }).map(|r| r.status(400)),
    };

    // 解析它，来验证这个形状 - endpoint、keys.p256dh、keys.auth。
    // 如果解析失败，浏览器交给我们的是某个格式错误的东西。
    let sub: SubscriptionInfo = match serde_json::from_str(&raw) {
        Ok(s) => s,
        Err(e) => return json_response!({ "error": e.to_string() }).map(|r| r.status(400)),
    };

    // 原样持久化 `raw` - 这正是 WebPushChannel 在分发时
    // 会交给 serde_json::from_str 的那个精确字符串。
    User::query()
        .db_where_op("id", "=", user_id)
        .update_all(attrs! { push_subscription_json: raw })
        .await
        .unwrap();

    json_response!({ "ok": true, "endpoint": sub.endpoint })
}
```

Service Worker 会解密这个推送载荷，并渲染这个通知：

```js
// /sw.js
self.addEventListener('push', (event) => {
    const data = event.data.json();
    event.waitUntil(
        self.registration.showNotification(data.title, {
            body: data.body,
            data: { url: data.url },
        }),
    );
});

self.addEventListener('notificationclick', (event) => {
    event.notification.close();
    event.waitUntil(clients.openWindow(event.notification.data.url));
});
```

## 载荷上限

Web 推送规范把每一个加密后的载荷，总共限制在 4096 字节以内。Suprnova
会在加密时拒绝大于 3992 字节的明文（这个上限减去 AES128GCM 大约 85 字节的加密开销），这样这个失败会在您的代码里暴露出来，而不是以推送服务给出的一个 413 出现。一个序列化后的 `data()` 超过这个上限的
`Notification`，会从这个通道的 `deliver` 里返回 `WebPushError::Encryption`。

对于任何更大的东西 - 一段长消息体，一张缩略图 - 就发送一条携带一个 URL
的简短通知，让 Service Worker 在点击时去拉取。这样做又快（不必对一个多
KB 的载荷做加密），又更灵活（这次拉取可以返回您想要的任何形状）。

## 失效的订阅

当推送服务返回 404 或 410 时，这个订阅就失效了 - 用户卸载了浏览器，撤销了权限，或者清空了存储。`WebPushChannel` 把这当作一次非致命的 WARN
来对待：

```text
WARN webpush subscription gone (404/410); caller should remove
     channel=webpush endpoint=https://fcm.googleapis.com/fcm/send/abc
```

分发返回 `Ok(())`，因为这个通知已经到达了一个终态 - 没有收件人可以重试。您的应用被期望依据这个 warn 采取行动：从日志里解析出 `endpoint`
（或者挂一个通过 `WebPushError` 分类的 `NotificationFailed` 监听器），并移除这条订阅行。Suprnova 提供这个 warn；它不会替您自动清理这张订阅表。

## 重试与 Retry-After

当推送服务返回一个暂时性的 5xx、408 或 429 时，底层的
`WebPushError::PushServiceRejected` 会携带解析出来的 `Retry-After` 提示（只支持 delta-seconds 形式 - HTTP-date 形式会返回 `None`）：

```rust
use suprnova::WebPushError;

match client.send(&sub, payload, ContentEncoding::Aes128Gcm, 60).await {
    Ok(_) => (),
    Err(e) if e.is_retryable() => {
        let wait = e.retry_after().unwrap_or(Duration::from_secs(30));
        tokio::time::sleep(wait).await;
        // ...重试一次，或者带着一个延迟把它推回队列
    }
    Err(WebPushError::SubscriptionGone) => {
        // 移除这个订阅
    }
    Err(e) => return Err(e.into()),
}
```

`Retry-After` 这个提示的上限是 24 小时，这样一个恶意的服务器，就不能把一个工作进程停放进一次持续数年的 sleep 里。

当使用 `Notify::queue` 时，适用的是队列自己的重试 / 退避 - 一个从
`WebPushChannel::deliver` 传播出来的 `WebPushError`，会以一个作业错误的形式暴露出来，而这个信封会按照这个作业的退避策略来处理重新入队。
`Retry-After` 这个提示会被记录日志，但（目前）还不会被反馈进队列的延迟计算里；如果您需要这一点，就挂一个用这个提示的延迟来重新入队的
`NotificationFailed` 监听器。

## 遥测

通知分发器会把这次扇出，包在一个用通知名和通道数量打了标签的
`notification.dispatch` info span 里。每一次成功的投递，都会发出一个
`NotificationSent` 事件；失败会发出携带通道名、路由和错误字符串的
`NotificationFailed`。用您接入其他框架事件的同样方式，把这些接入您的指标 / 日志流水线 - 参见[事件](events.md)。

一个失效的订阅，会发出一条带着 `channel="webpush"`、这个端点和这个通知名的结构化 WARN。这就是该去抓取、用来驱动一个自动化订阅清理作业的信号。

### 为什么 Suprnova 有所不同

Laravel 的 `WebPush` 驱动程序是一个社区包（`laravel-notification-channels/webpush`） - 不在核心里，单独发版，对 ORM 有自己的主张。Suprnova 把 Web 推送烤进了框架本体，因为这份协议定义得很清楚，而这个加密的 HTTP POST，是一个太小的契约，不值得包进一个第三方抽象里。通知子系统让这个表面保持统一：您发给邮件或数据库的同一个 `Notification`，也会作为一次推送落地，没有驱动程序矩阵，没有单独的配置树。

我们还默认暴露了这个严格端点策略。这个 Laravel 社区包，把 SSRF 防护留给了应用；我们的立场是，「这个端点来自用户数据」是每一个 Web 推送订阅的固有形状，而这个安全的默认值，应该属于框架，不该属于您的代码。

这个重试分类（`is_retryable`、`retry_after`），是作为 `WebPushError`
上的类型化方法暴露出来的，而不是作为队列层里的一张魔法常量表。队列仍然拥有重试策略 - 这个错误告诉您一次重试是否可能成功，以及要等多久；队列决定是否、以及何时再次出队。把这两者分开，意味着您自定义的重试策略（指数退避、带抖动、有上限）不必为 Web 推送特殊处理。

## 测试

搭起一个 `wiremock` 服务器，用 `EndpointPolicy::AllowAny` 把一个
`WebPushClient` 指向它，然后对它收到的请求做断言：

```rust
use std::sync::Arc;
use suprnova::{
    EndpointPolicy, NotificationDispatcher, Notify, VapidKey, VapidSigner,
    WebPushChannel, WebPushClient,
    notifications::set_dispatcher,
};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn order_shipped_pushes() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/push"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&server)
        .await;

    let signer = VapidSigner::new(VapidKey::generate());
    let client = Arc::new(
        WebPushClient::new(signer, "mailto:test@example.org")
            .unwrap()
            .with_endpoint_policy(EndpointPolicy::AllowAny),
    );
    let channel = Arc::new(WebPushChannel::new(client, 60));

    let dispatcher = NotificationDispatcher::new().register_channel(channel);
    set_dispatcher(Arc::new(dispatcher)).unwrap();

    let user = test_user_with_subscription(&server.uri()).await;
    Notify::send(&user, &OrderShipped {
        order_id: 1,
        tracking_url: "https://ship.example.org/o/1".into(),
    }).await.unwrap();
    // server.received_requests() 现在包含了这次加密的 POST。
}
```

对于不关心加密字节的端到端测试，`Notify::fake()`（在
[通知](notifications.md)里介绍过）会捕获这次分发，而不运行这个通道 -
更快，没有 mock 服务器，没有加密往返。

## 参考

- 原语：`suprnova::VapidKey`、`suprnova::VapidSigner`、
  `suprnova::VapidClaims`
- 客户端：`suprnova::WebPushClient`、`suprnova::EndpointPolicy`、
  `suprnova::PushResponse`、`suprnova::SubscriptionInfo`
- 错误：`suprnova::WebPushError` - `.is_retryable()`、`.retry_after()`、
  `WebPushError::SubscriptionGone`
- 编码：`suprnova::ContentEncoding`（Aes128Gcm；3992 字节的明文上限）
- 通道：`suprnova::WebPushChannel`
- 门面：`suprnova::Notify`
- 队列作业：`suprnova::SendNotificationJob`
- 工厂注册：
  `suprnova::notifications::register_notification_factory`

## 下一步

- [通知](notifications.md) - `WebPushChannel` 接入的那个多通道分发器
- [邮件](mail.md) - 给没有推送的用户用的邮件通道对应物
- [广播](broadcasting.md) - 给正在站点上的用户用的实时投递
- [队列](queues.md) - `Notify::queue` 是如何为 `SendNotificationJob` 撑腰的
- [事件](events.md) - 监听 `NotificationSent` /
  `NotificationFailed` 来驱动失效订阅的清理
