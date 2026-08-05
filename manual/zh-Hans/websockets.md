# WebSocket

Suprnova 的 WebSocket 路由和 HTTP 路由并排存在于同一个路由器里。您注册一个路径和一个处理程序；框架会在那个路径上检测 `Upgrade: websocket` 请求，运行与一次 HTTP GET 到同一路径时一样的中间件链，完成 RFC 6455 握手，然后带着一个类型化的 `WsSocket` 加上原始的 `Request` 去调用您的处理程序。没有一个单独的 WebSocket 服务器 - 连接是从服务您 HTTP 流量的那同一个 hyper 监听器升级而来的。框架还会在一个逐服务器的 `JoinSet` 里跟踪每一个被 spawn 出来的处理程序，所以一次优雅关闭，会在监听器退出之前排空飞行中的连接。

## 快速上手

添加一个 `EchoHandler`，并把它注册进 `routes!`。

`src/ws/echo.rs`：

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct EchoHandler;

#[async_trait]
impl WebSocketHandler for EchoHandler {
    async fn handle(&self, mut socket: WsSocket, _req: Request) -> Result<(), FrameworkError> {
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("echo: {text}")).await?;
        }
        Ok(())
    }
}
```

`src/routes.rs`（在 `routes! { ... }` 里面）：

```rust
ws!("/ws/echo", app_ws::echo::EchoHandler),
```

启动应用，并用 `wscat` 连接：

```bash
cargo run --bin app
```

```text
$ wscat -c ws://localhost:3000/ws/echo
Connected (press CTRL+C to quit)
> hello
< echo: hello
> suprnova
< echo: suprnova
```

当 `recv_text()` 返回 `Ok(None)` 时，对端关闭了这个连接；这个循环退出，处理程序返回 `Ok(())`，框架发送一个干净的 Close(1000) 帧。

## 升级的生命周期

一次 WebSocket 握手，就是一个带 `Upgrade: websocket` 的 HTTP GET。框架会在任何帧流动之前，对它运行完整的请求流水线：

1. **路由匹配。** 路由器会在 WS 路由表里查找这个路径；未命中时，这个请求会落到 HTTP 的后备路径上。
2. **来源策略。** 配置好的 [`OriginPolicy`](#来源策略) 会被强制执行。违反它会返回 HTTP 403，不做升级。
3. **子协议协商。** 如果这条路由有 `accepted_protocols`，第一个与客户端提议重合的 token，会被回显在 101 响应上。
4. **中间件链。** `RequestIdMiddleware` 跑在最外层，然后是每一个全局注册的中间件，再然后是这条路由自己的逐路由中间件。任何中间件给出的非 2xx 响应，都会让这次升级短路 - 对端会收到这个 HTTP 错误，而这个 WebSocket 的 future 会干净地被丢弃。
5. **握手。** `hyper_tungstenite::upgrade` 产出那个会解出一个 `WebSocketStream` 的 future。
6. **处理程序分发。** 这个（可能已经被中间件重写过的）`Request`，加上一个新构建的 `WsSocket`，会被交给 `WebSocketHandler::handle`。
7. **心跳 + 处理程序。** 框架会 spawn 一个逐连接的心跳任务，并在一个带着请求 id 的 `ws.connection` tracing span 之下，等待这个处理程序的 future。
8. **关闭握手。** 在 `Ok(())` 上，框架发送 Close(1000)；在 `Err(_)` 上，它发送 Close(1011 "internal error")。转发器会被等待，这样这个关闭帧会在这个连接被追踪的任务上报完成之前，被刷写进传输格式里。

返回值的语义和 HTTP 是反过来的：没有响应体。`Ok(())` 意味着干净地断开连接；`Err(_)` 会被记录日志，对端会看到 Close(1011)。无论哪种情况，这个连接都会被拆除。

## `WsSocket` API

`WsSocket` 是框架传给您的处理程序的那个双向句柄。在内部，底层的 tungstenite 流被拆分成 Sink 和 Stream 两半：一个转发器任务拥有这个 sink，并排空一个 mpsc；面向处理程序的发送方法，会把内容排入这个 mpsc。处理程序直接从 stream 那一半读取。这个拆分意味着框架也可以推送帧（心跳 ping、广播器的扇出），而不必和处理程序的发送路径抢占。

### `send_text`

```rust
socket.send_text("hello").await?;
socket.send_text(format!("user {id} joined")).await?;
```

把一个 UTF-8 文本帧排入队列。只有当这个连接已经关闭时，才会返回 `Err`。

### `send_binary`

```rust
socket.send_binary(bytes).await?;
```

把一个二进制帧排入队列。接受任何 `Into<Vec<u8>>`。错误语义与 `send_text` 相同。

### `recv_text`

```rust
while let Some(text) = socket.recv_text().await? {
    // text: String
}
// Ok(None) 意味着对端关闭了连接。
```

返回下一条文本消息，静默地丢弃一个只处理文本的处理程序不该关心的那些帧类型：

- `Message::Binary` - 对端的二进制载荷
- `Message::Ping` - 由对端发起的 ping（tungstenite 会自动处理这个 pong）
- `Message::Pong` - 对端对框架心跳的 pong 回复（副作用是，未响应 ping 计数器会被重置为零）
- `Message::Frame` - 来自服务器端语境的原始帧变体；在这一层永远不会遇到

一个被吞掉的帧就没有了；没有办法回溯着去看到它。如果处理程序需要观察二进制帧或者关闭码，就从第一次读取开始，使用 [`recv`](#recv)。

### `recv`

```rust
use tokio_tungstenite::tungstenite::Message;

while let Some(msg) = socket.recv().await? {
    match msg {
        Message::Text(t)   => { /* ... */ }
        Message::Binary(b) => { /* ... */ }
        Message::Close(_)  => break,
        _                  => {}
    }
}
```

返回任意类型的下一条消息，包括 Binary、Ping、Pong 和 Close。在被返回之前，`Pong` 仍然会把未响应 ping 计数器重置为零，作为一个副作用。`Ok(None)` 意味着底层的流已经结束。

### `close`

```rust
socket.close(1008, "policy violation").await?;
return Ok(());
```

把一个关闭帧排入队列，然后返回。这个转发器会把这个帧写进 sink，在这个 sink 上调用 `close()`，然后终止。在同一个 socket 上后续的发送，都会返回 `Err`，因为这个转发器已经不在了。在调用 `close` 之后，总是立刻返回 `Ok(())`。

`close` 会依据 RFC 6455 §7.4 + §5.5.1，提前验证它的参数：

- `code` 必须满足 `CloseCode::is_allowed()`。保留的或者无效的码（1004、1005、1006、1015，任何低于 1000 的，任何高于 4999 的）都会被 `Err` 拒绝，并且**不会发送任何帧** - 这个连接保持打开，调用方可以带着一个有效的码重试。正常关闭用 1000，已定义的原因用 1001-1013，IANA 注册的码用 3000-3999，或者应用私有的码用 4000-4999。
- `reason` 的上限是 123 字节（125 字节的控制帧上限减去 2 字节的码）。更长的原因会被拒绝，什么都不会被排入队列。

### 为什么 Suprnova 有所不同

PHP 框架把 WebSocket 支持作为一个独立的进程外挂上去（ratchet、soketi、pusher）。Suprnova 的 WebSocket 路由，活在和您的 HTTP 路由同一个 `routes! { ... }` 里，由同一个 hyper 监听器服务，由同一条优雅关闭路径排空。只有一个二进制文件，一份配置，一次部署。长期存活的连接是一等公民，因为 Tokio 让它们变得低成本；框架不必为它们道歉。

## 路径参数

WebSocket 路由支持和 HTTP 路由一样的 `{param}` 捕获语法。被捕获的值，可以在传给处理程序的 `Request` 上取到。

```rust
// In routes!:
ws!("/ws/rooms/{id}", RoomHandler),
```

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct RoomHandler;

#[async_trait]
impl WebSocketHandler for RoomHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let room_id = req.param("id")?;
        socket.send_text(format!("joined room {room_id}")).await?;
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[{room_id}] {text}")).await?;
        }
        Ok(())
    }
}
```

`req.param("id")` 返回 `Result<&str, ParamError>`；如果这个片段缺失，`?` 会传播一个 `FrameworkError::ParamError`，这会导致处理程序返回 `Err`，框架发送 Close(1011)。在实践中，只要这条路由匹配上了，这个捕获值就总是存在的 - 这条错误路径，是针对参数名拼写错误的一张安全网。

Express 风格的 `:id` 片段也被接受（`ws!("/ws/rooms/:id", h)`），并会在内部转换成 matchit 形式。

关于完整的 `Request` API - 请求头、cookie、查询字符串、对端地址 - 请参见[请求文档](requests.md)。

## 逐路由中间件

在 `ws!` 条目上链式调用 `.middleware(M)`。多个中间件从左到右组合，并按照与一次 HTTP 请求到同一路径完全相同的固定顺序运行：`RequestIdMiddleware` 在最外层，然后是每一个全局注册的中间件，然后是这条逐路由的链，然后是处理程序。

```rust
ws!("/ws/private", PrivateHandler)
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

任何中间件给出的非 2xx 响应，都会让这次升级短路。对端会收到这个带着 `X-Request-Id` 的拒绝响应（比如 401、403），那个从未被唤醒的 WebSocket future 会干净地被丢弃，处理程序永远不会被调用。这是做传输层面检查的正确层次：究竟谁可以打开这个连接，这个连接是从哪里来的，每个身份有多少个并发连接。

中间件可以通过调用 `next(modified_req)`，用一个修改过的 `Request` 来替换。终结者会捕获这条链最终传出来的东西，而这就是处理程序会看到的那个 `Request` 参数。解析身份的中间件（一次会话查找，一次令牌检查），可以通过 `Request` 的扩展来附加结果；处理程序会用和 HTTP 控制器一样的方式把它读回来。

直接作用在 `Router` 上的变体（`Router::ws`、`Router::ws_with_middleware`、`Router::ws_with_config`、`Router::ws_with_middleware_and_config`），为那些在宏之外构建 `Router` 的代码，覆盖了同样的表面。每一个都有一个可失败的 `try_*` 对应函数，会在重复或者格式错误的模式上返回 `Err(FrameworkError)`，而不是 panic。

### 为什么 Suprnova 有所不同

大多数生态系统要么在 WebSocket 升级上跳过中间件（Node 的惯例），要么为「WebSocket 中间件」强加一套单独的注册仪式（.NET / Spring 的惯例）。Suprnova 把这次升级当作它本来就是的那个 HTTP GET 来对待：同一条链，以同一个顺序运行，带着同样的短路语义。没有第二个概念需要学习 - `AuthMiddleware`、`RateLimitMiddleware`、`RequestIdMiddleware`、`CorsMiddleware` 能在 WS 路由上工作，是因为它们能在任何路由上工作。来源的强制执行是唯一额外的一点复杂之处，而它是 `WsConfig` 的一个属性，不是一个单独的中间件。

## 连接时的认证

处理程序接收到的是经过中间件重写的 `Request`。三种模式效果都不错，按与框架其余部分的整合程度递增排列：

**模式一 - 处理程序里内联的 bearer 令牌。** 最简单。不需要任何认证中间件就能工作。`wscat`、浏览器客户端和负载均衡器，都会干净地传递请求头。

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct PrivateChatHandler;

#[async_trait]
impl WebSocketHandler for PrivateChatHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let Some(token) = req.header("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            socket.close(1008, "missing bearer token").await?;
            return Ok(());
        };
        let Some(user_id) = verify_token(token).await else {
            socket.close(1008, "invalid bearer token").await?;
            return Ok(());
        };
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[user {user_id}] {text}")).await?;
        }
        Ok(())
    }
}

async fn verify_token(_token: &str) -> Option<i64> { Some(42) }
```

**模式二 - 用一个路由中间件来给这次升级把关。** 在任何帧流动之前，拒绝未经授权的打开操作。关注点分离得更干净；处理程序只会看到已经认证过的连接。

```rust
ws!("/ws/private", PrivateChatHandler)
    .middleware(AuthMiddleware::new()),
```

`AuthMiddleware` 在未认证的请求上返回 401；这次升级会带着这个拒绝响应被中止，处理程序永远不会被调用。

**模式三 - 中间件把关，加上处理程序重新读取。** 中间件会让未经授权的打开操作短路；然后处理程序会重新读取它知道现在必定存在的那个凭据（令牌、cookie 等等），来识别刚刚连接上来的是哪个用户：

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    // 中间件已经审查过这个 bearer；只有在它有效时，我们才会走到这里。
    let token = req.bearer_token().expect("auth middleware vetted bearer presence");
    let user_id = lookup_user_by_token(&token).await?;
    // ...
}
```

**模式四 - 让中间件去认证，然后读取结果。** 当一个认证中间件已经在这次升级上运行时，这是首选做法。它解析出的身份，会被携带在这个请求本身上：

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    let Some(user_id) = req.auth_user_id() else {
        socket.close(1008, "unauthenticated").await?;
        return Ok(());
    };
    // `user_id` 来自会话 / 令牌中间件，不是来自客户端在某个帧里
    // 发送的任何东西。
    socket.send_text(format!("welcome, {user_id}")).await?;
    Ok(())
}
```

这正是让一个私有广播频道的 `authorize` 钩子变得有意义的地方：它接收到的是同一个 `Request`，所以它能依据服务器推导出的身份来把关，而不是依据客户端自己选择的一个值。在 `auth_user_id` 存在之前，一个频道没有任何可信的东西能去查询，而那个显而易见的占位方案 - 「接受任何订阅帧携带的令牌看起来对的订阅者」 - 根本不算一个门。

那些在 HTTP 控制器里能用的线程本地访问器 - `session()`、`Auth::user()`、逐请求的 `Context` 包 - 在一个 WebSocket 处理程序内部依然**不会**被填充。中间件链的任务本地作用域，会在这条链返回时展开；处理程序运行在一个刚被 spawn 出来的任务里，那个任务只继承了请求 id 和解析出来的认证 id。处理程序需要的其他一切，都直接从 `Request` 上读取（请求头，通过 `req.cookie("...")` 读取的 cookie，被捕获的参数，通过 `req.bearer_token()` 读取的 bearer 令牌）- 这些都会存活进这个处理程序的任务里。

### 为什么 Suprnova 有所不同

Laravel 通过一个单独的 HTTP 端点（`/broadcasting/auth`）来给广播频道授权，所以这个频道回调运行在一个拥有完整会话的普通请求里。Suprnova 则改成在升级期间就在进程内授权 - 一个连接，没有第二次往返 - 这意味着身份必须被显式地携带跨越这个 spawn 边界，而不是被重新查找。

## `WsConfig`

`WsConfig` 控制逐连接的行为。默认值面向公开的、面向浏览器的端点 - 每一个活跃连接都会预留一个按 `max_message_size` 定量的 tungstenite 缓冲区，所以框架默认取小值，让需要更多的路由自己显式地把上限调高。

| 字段                 | 默认值        | 类型            | 效果 |
|-----------------------|----------------|-----------------|--------|
| `ping_interval`       | 30s            | `Duration`      | 框架发送一次 Ping 帧来保持连接活跃的频率。 |
| `max_message_size`    | 1 MiB          | `usize`         | 重组后消息的最大字节数。更大的消息会被 tungstenite 拒绝。 |
| `max_frame_size`      | 64 KiB         | `usize`         | 单个 WebSocket 帧的最大字节数。 |
| `max_missed_pings`    | 2              | `usize`         | 心跳以码 1011 关闭连接之前，连续未响应的 Pong 次数。`usize::MAX` 会禁用这项强制检查。 |
| `origin_policy`       | `SameOrigin`   | `OriginPolicy`  | 在升级时强制执行的来源请求头检查。参见[来源策略](#来源策略)。 |
| `accepted_protocols`  | `vec![]`       | `Vec<String>`   | 服务器接受的 `Sec-WebSocket-Protocol` token。空表示不协商。参见[子协议](#子协议)。 |

按使用场景推荐的覆盖值：

- **聊天 / 通知 / 光标位置** - 默认值就可以。如果您的负载均衡器有一个激进的空闲超时，就把 `ping_interval` 降到 5 到 10 秒。
- **受信任的内部数据源**（服务器到服务器的扇出，批量导出，大型二进制传输）- 从 `WsConfig::generous()` 开始，它会把 `max_message_size` 提到 64 MiB，把 `max_frame_size` 提到 16 MiB，同时保留其他默认值。
- **特定的超大载荷**（一条上传 256 MiB 音频文件的路由）- 直接设置这些字段；不要把这个更大的上限，应用到不需要它的路由上。

这个配置结构体可以用 `Default` 构造，并且每一个字段都是公开的：

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let chat = WsConfig {
    ping_interval: Duration::from_secs(5),
    max_missed_pings: 1,
    ..Default::default()
};

let trusted = WsConfig::generous();
assert_eq!(trusted.max_message_size, 64 * 1024 * 1024);
assert_eq!(trusted.max_frame_size, 16 * 1024 * 1024);
```

在 `ws!` 条目上或者在 `Router::ws_with_config` 上，逐路由地应用这个覆盖：

```rust
ws!("/ws/chat", ChatHandler).config(chat),
```

`WsConfig` 会在路由注册时被验证。一个为零的 `ping_interval` 或者一个为零的 `max_missed_pings`，会破坏这个心跳任务；两者都会在启动时被拒绝，而不是在第一个连接上 panic。

### 心跳与无 pong 时关闭

对每一个升级后的连接，框架都会 spawn 一个心跳任务，每 `ping_interval` 发送一次 `Ping(b"")`。每一次 tick，未响应 ping 计数器都会递增；每一次对端的 Pong，它都会被重置为零。如果这个计数器达到了 `max_missed_pings`，这个心跳就会发送 Close(1011 "no pong response")，然后这个连接会被拆除。把 `max_missed_pings` 设为 `usize::MAX`，可以禁用这项强制检查（ping 仍然会照常发送，但这个连接永远不会因为缺失 pong 而被关闭）。

第一次 tick 会在任务启动时就被消耗掉，这样对端在第一次 ping 之前，至少能得到一整个完整间隔的宽限期。

## 来源策略

浏览器在 WebSocket 握手上总是会发送一个 `Origin` 请求头。和 `fetch()` / `XMLHttpRequest` 不同，WebSocket 升级不受 CSRF 令牌中间件的保护（这次握手不携带任何令牌），所以一次同源 `Origin` 检查，是站在一个恶意页面和一个登录用户会话上的特权 WS 端点之间的唯一屏障。框架会在 `hyper_tungstenite::upgrade` 被调用之前，强制执行这个配置好的策略；违反它会返回 HTTP 403，不做升级。

```rust
use suprnova::ws::{OriginPolicy, WsConfig};

let cfg = WsConfig {
    origin_policy: OriginPolicy::AllowList(vec![
        "https://app.example.com".into(),
        "https://admin.example.com".into(),
    ]),
    ..Default::default()
};
```

| 变体      | 行为 |
|--------------|----------|
| `SameOrigin`（默认） | 只有当 `Origin` 的 host（以及端口，如果有的话）与请求的 `Host` 请求头匹配时才允许。缺失 `Origin` 会被拒绝。不比较 scheme（TLS 在上游终结，所以服务器没法可靠地判断公开的 scheme 究竟是 https 还是 http）。 |
| `AllowAny`   | 跳过这个检查。只用于非浏览器端点（服务器到服务器，原生应用，测试用的 mock）。 |
| `AllowList(Vec<String>)` | 只有当 `Origin` 精确匹配（不区分大小写）所提供的来源之一时才允许。每一项都是浏览器会发送的完整 `scheme://host[:port]` 形式。 |

非浏览器客户端（CLI 工具、服务器、原生应用）通常不会发送 `Origin` 请求头。只服务这类客户端的路由，应该用 `AllowAny`；两者都服务的路由，应该用 `AllowList`，把每一个生产环境的前端来源都列举出来。

## 子协议

一个 WebSocket 子协议，是客户端和服务器在握手期间商定的一个应用层面的 token（比如 `graphql-transport-ws`、`jsonrpc-2.0`）。填充 `accepted_protocols` 来参与进来：

```rust
use suprnova::ws::WsConfig;

let cfg = WsConfig {
    accepted_protocols: vec![
        "graphql-transport-ws".into(),
        "graphql-ws".into(),
    ],
    ..Default::default()
};
```

当客户端提议 `Sec-WebSocket-Protocol` 时，框架会挑选（按 RFC 6455 §4.2.2 的客户端偏好顺序）第一个与 `accepted_protocols` 重合的、客户端提议的 token，不区分大小写地匹配，并把它回显在 101 响应上。如果客户端提议了协议，但没有一个匹配上，这次升级依然会成功，只是不带 `Sec-WebSocket-Protocol` 请求头 - RFC 6455 那时会要求浏览器在客户端这一侧让这个连接失败，这才是正确的行为（一个继续下去的服务器，会悄无声息地说着错误的协议）。

当 `accepted_protocols` 为空时，协商会被完全跳过 - 这个升级响应会省略 `Sec-WebSocket-Protocol`，客户端会退回到默认的协议处理。

## 生产部署

框架处理握手和帧 I/O。在生产环境里，您在框架这一侧不需要任何额外的配置。

**TLS 终结发生在上游。** 客户端在 nginx、Caddy，或者云负载均衡器上连接 `wss://`；这个代理会剥掉 TLS，把纯粹的 `ws://` 转发给框架。框架不需要一个 `rustls` feature 或者一份 TLS 证书。

### nginx

```nginx
location /ws/ {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "Upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
}
```

`proxy_read_timeout` 和 `proxy_send_timeout` 必须足够长，才能覆盖心跳之间的空闲间隙。在默认的 30 秒 `ping_interval` 下，3600 秒是一个舒适的上限。

### Caddy

```caddy
reverse_proxy /ws/* localhost:3000 {
    header_up Upgrade {http.request.header.Upgrade}
    header_up Connection "Upgrade"
}
```

Caddy 在代理时会自动处理 `Upgrade` / `Connection`；上面这些显式的 `header_up` 指令，是为了让意图更清楚。

### 云负载均衡器（AWS ALB、GCP GLB）

在监听器规则上启用 WebSocket 支持（当目标组的协议是 HTTP/1.1、且粘性会话关闭时，AWS ALB 会自动做到这一点）。确保负载均衡器的空闲超时至少和 `ping_interval` 一样长；框架的心跳会让这个连接保持活跃，但负载均衡器会丢弃在它看来是空闲的连接。

## 优雅关闭

每一个被 spawn 出来的 WebSocket 处理程序，都在服务器的 `WS_TASKS` `JoinSet` 里被跟踪。在 `Ctrl-C` 或者一个外部关闭信号上，这个监听器会停止接受新连接，`Server::run` 会在进程退出之前排空这个集合。这个处理程序的 future 要等到关闭握手被刷写完成才会解出：在用户的 `handle` 返回之后，框架会等待这个转发器，这样最终的 Close(1000) 或者 Close(1011) 帧，会在这个连接的任务上报完成之前，被写进传输格式里。在一次干净的关闭里，对端看到的是一次正常关闭，不是一次 TCP 重置。

已完成的句柄，会在服务器的生命周期内被随手收割，所以在长期运行下，这个 `JoinSet` 不会无限增长。

## 参考

| Symbol | Purpose |
|---|---|
| `suprnova::ws::WebSocketHandler` | Trait：`async fn handle(&self, socket: WsSocket, request: Request) -> Result<(), FrameworkError>`。`Send + Sync + 'static`。 |
| `suprnova::ws::WsSocket` | 双向句柄。方法：`send_text`、`send_binary`、`recv_text`、`recv`、`close`。`close` 会提前验证码和原因的长度。 |
| `suprnova::ws::WsConfig` | 逐连接配置。字段：`ping_interval`、`max_message_size`、`max_frame_size`、`max_missed_pings`、`origin_policy`、`accepted_protocols`。`Default` + `generous()` 构造函数。在注册时被验证。 |
| `suprnova::ws::OriginPolicy` | `SameOrigin`（默认）、`AllowAny`、`AllowList(Vec<String>)`。在升级时强制执行。 |
| `ws!(path, Handler)` | `routes! { ... }` 的宏形式。返回一个支持以任意顺序 `.config(WsConfig)` 和 `.middleware(M)` 的 `WsRouteDef`。 |
| `Router::ws(path, handler)` | 直接注册。返回 `Router`。 |
| `Router::ws_with_config(path, handler, cfg)` | 逐路由的 `WsConfig` 覆盖。 |
| `Router::ws_with_middleware(path, handler, mws)` | 逐路由的中间件列表。 |
| `Router::ws_with_middleware_and_config(...)` | 两者都有。 |
| `Router::try_ws*` 家族 | 可失败的对应函数 - 在重复或者格式错误的模式上返回 `Err(FrameworkError)`，而不是 panic。 |

## 下一步

- [广播](broadcasting.md) - 频道、呈现、构建在 `ws!` 之上的传输协议
- [Server-Sent 事件](sse.md) - 面向那些身处严格代理之后的浏览器的单向推送
- [路由](routing.md) - `routes!` 和 `ws!` 到底展开成了什么
- [中间件](middleware.md) - 编写能统一给 HTTP 和 WS 把关的中间件
- [请求](requests.md) - 您的处理程序收到的那个 `Request` 上的请求头、cookie、查询参数、扩展
