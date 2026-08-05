# Server-Sent 事件

Server-Sent 事件（SSE）是从服务器到浏览器的最简单向推送通道：浏览器打开一个
`EventSource(url)`，服务器让一个 `text/event-stream` 响应保持打开，并在事件发生时推送成帧的事件。没有 WebSocket 握手，没有 permessage-deflate，没有成帧库 - 只有以一个空行结束的 `data:`、`event:`、`id:`、`retry:` 行，遵循
[WHATWG `EventSource`](https://html.spec.whatwg.org/multipage/server-sent-events.html)
规范。

Suprnova 的 SSE 原语接入的是流式响应体这条路径：构建一个
`Stream<Item = SseEvent>`，把它交给 `HttpResponse::sse(...)`，连接管理、成帧、响应头和 panic 隔离都由框架负责。这个连接会一直保持打开，直到生产者流结束，或者客户端断开连接。

## 什么时候用 SSE，什么时候用 WebSocket

| 属性 | SSE | WebSocket |
|----------|-----|------------|
| 方向 | 服务器 → 浏览器 | 双向 |
| 传输 | 纯 HTTP/1.1 或 HTTP/2 | 仅升级 |
| 重连 | 自动，使用 `retry:` 和 `Last-Event-ID` | 手动 |
| 代理 / CDN | 能穿过任何允许长时间 HTTP 响应的东西 | 往往需要显式的 Upgrade 支持 |
| 浏览器 API | `EventSource`（内置） | `WebSocket`（内置） |
| 二进制帧 | 仅文本（UTF-8） | 文本或二进制 |
| 每标签页连接上限 | 6（HTTP/1.1）/ 无限制（HTTP/2） | 无限制 |

当您只需要从服务器到客户端的推送时（活动流、通知、日志追踪、AI 流式输出），就伸手去拿 SSE。当您需要双向流量或二进制帧时，就伸手去拿
[WebSocket](websockets.md)。

## 快速上手

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub async fn stream_ticks(_req: Request) -> Response {
    let (tx, rx) = mpsc::channel::<SseEvent>(16);
    tokio::spawn(async move {
        for i in 0..10 {
            let evt = SseEvent::data(format!("tick {i}"))
                .with_event("tick")
                .with_id(i.to_string());
            if tx.send(evt).await.is_err() {
                break; // 客户端已断开连接
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    Ok(HttpResponse::sse(ReceiverStream::new(rx)))
}
```

一次 tick 的传输格式输出：

```text
event: tick
id: 0
data: tick 0

```

浏览器会解析这个输出，并触发一个 `evt.data === "tick 0"`、
`evt.lastEventId === "0"` 的 `tick` 事件。

## `SseEvent` API

`SseEvent` 是您推送到流上的类型。它有两种：

* **Frame** - 一个普通事件，带有可选的 `event` / `id` / `retry`，以及一个多行的 `data` 载荷。通过 [`SseEvent::data`](#构造函数)、
  `SseEvent::json`，或 `SseEvent::error` 构建。
* **Comment** - 一个只存在于传输格式里的保活消息（`:\n\n` 或
  `: <text>\n\n`）。通过 `SseEvent::comment(text)` 或
  `SseEvent::keep_alive()` 构建。浏览器按规范会忽略注释；穿越这个连接的这些字节，是让空闲的代理和负载均衡器不去关闭它的东西。

### 构造函数

| 构造函数 | 产出 | 用途 |
|-------------|----------|-----|
| `SseEvent::data(text)` | 只有 `data:` 行的 Frame | 最小化的事件 |
| `SseEvent::json(event, &payload)` | 带 `event:` + JSON `data:` 的 Frame | 95% 的情形 - 客户端对 `evt.data` 调用 `JSON.parse()` |
| `SseEvent::error(message)` | 带 `event: error` 的 Frame | 领域层面的错误事件，与浏览器在传输失败时触发的连接层面的 `error` 不同 |
| `SseEvent::comment(text)` | Comment | 带一个运维人员能在日志里认出的标记的保活消息 |
| `SseEvent::keep_alive()` | 空 Comment（`:\n\n`） | 规范意义上字节数最少的心跳 |

### 构建器

| 构建器 | 效果 | 对 `Comment` 而言 |
|---------|--------|--------------|
| `.with_event(name)` | 设置 `event:` 字段 | 静默空操作 |
| `.with_id(id)` | 设置 `id:` 字段 - 恢复语义所必需 | 静默空操作 |
| `.with_retry(Duration)` | 设置 `retry:` 字段（毫秒）；规范规定 `Duration::ZERO` 意味着“立即重连” | 静默空操作 |
| `.try_with_event(name)` | 可失败的变体 - 参见[安全契约](#安全契约) | `Ok(self)`，不变 |
| `.try_with_id(id)` | `with_id` 的可失败变体 | `Ok(self)`，不变 |

`Comment` 上的构建器故意是空操作 - 这个传输格式没有办法表达“带一个事件名的注释”。误用会保持静默，而不是把这个事件转换成一个 Frame 来让生产者感到意外。

### 访问器

| 方法 | 返回值 |
|--------|---------|
| `.event()` | `Option<&str>` - 事件名，如果已设置 |
| `.id()` | `Option<&str>` - 最后一次事件的 id，如果已设置 |
| `.retry()` | `Option<Duration>` - 重连延迟，如果已设置 |
| `.payload()` | `&str` - `data:` 载荷（对 `Comment` 而言是 `""`） |
| `.is_comment()` | `bool` |
| `.comment_text()` | `Option<&str>` - 注释文本，如果这是一个 `Comment` |

### 传输格式编码

`SseEvent::to_wire()` 把这个事件序列化成可以直接进入响应体流的 `Bytes`：

**Frame：**

```text
event: <event>\n   (只在 Some 时)
id: <id>\n         (只在 Some 时)
retry: <ms>\n      (只在 Some 时)
data: <line>\n     (载荷里的每一行各一条，在 \r/\r\n 归一化之后)
\n                 (终止符 - 规范要求)
```

**Comment：**

```text
: <line>\n         (注释文本里的每一行各一条；空行是 `:\n`)
\n                 (刷写边界)
```

## 安全契约

SSE 的传输格式用 CR / LF / NUL 作为字段终止符，没有任何转义机制。一个让用户输入未经净化就抵达 `event:` 或 `id:` 的生产者，会暴露一个字段注入漏洞 - 一个 `"legit\ndata: injected"` 的值会在传输格式里产出两个 `data:` 字段，而
`"legit\n\nevent: spoofed"` 会终止当前事件并开启一个新的。

Suprnova 的 `to_wire()` 用两层来防御：

* **`event:` 和 `id:` 字段的值** - 每一个 CR / LF / NUL，都会在序列化时被剥除。每一次剥除都会触发一次结构化的 `WARN`：`target: "suprnova::sse"`，
  `field = "event"|"id"`。这个 warn 永远不记录那个值本身 - 从构造上来说，那些字节是攻击者可控的。
* **`data:` 和注释文本** - `\r\n` 和裸的 `\r`，会在切分之前被归一化为
  `\n`，所以一个在载荷里嵌入 `\r` 的生产者，没法让接收端的解析器在解析时合成出一个 `data:` / `event:` / `id:` 字段。NUL 会从注释文本里被剥除，并触发一条相配的 `WARN`。

如果您想在坏输入上**快速失败**，而不是静默地剥除，就伸手去拿
`try_with_*` 这几个对应函数：

```rust
use suprnova::{Response, sse::SseEvent};

let evt = SseEvent::data("hello")
    .try_with_event(&user_supplied_event)?     // 在 CR/LF/NUL 上返回 Err
    .try_with_id(&user_supplied_id)?;
```

返回的 `FrameworkError::validation(field, ...)` 会点名这个字段；它**不会**
把这个值回显出来，所以一个被暴露给客户端的 400，是可以安全记录日志的。

## 保活与代理的空闲超时

长期存活的 SSE 连接默认是沉默的。大多数生产环境的部署，都坐在一个会关闭空闲连接以释放资源的代理 / 负载均衡器 / CDN 后面：

* nginx 默认值：60 秒
* AWS ALB 默认值：60 秒
* Cloudflare 默认值：100 秒

每 15 到 30 秒发一次 `keep_alive()` 注释，能让这个连接穿过以上所有这些场景存活下来，而不必向浏览器分发一个 `message` 事件。这个字节数最少的形式（`:\n\n`）已经足够刷写代理的写缓冲区，而不需要发送任何载荷。

```rust
use std::time::Duration;
use futures::StreamExt;
use suprnova::sse::SseEvent;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

let (tx, rx) = mpsc::channel::<SseEvent>(16);

// 心跳任务 - 独立于事件生产者。
let hb_tx = tx.clone();
tokio::spawn(async move {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        if hb_tx.send(SseEvent::keep_alive()).await.is_err() {
            break; // 客户端已经不在了
        }
    }
});

// 事件生产者……在事件发生时把 frame 发送进 `tx`。
```

## 断开后恢复（`Last-Event-ID`）

当浏览器的 `EventSource` 断开这个连接时，它会自动重连，并把它见过的最新
`id:` 作为新请求上的 `Last-Event-ID` 请求头发送。用 `.with_id(...)` 给每一个事件打上标记，并在恢复请求上读取这个请求头：

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::{self, SseEvent}};

pub async fn stream_from_resume(req: Request) -> Response {
    let resume_from: u64 = sse::last_event_id(&req)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // 从 `resume_from + 1` 开始构建生产者流。这个闭包拥有自己的运行中计
    // 数器，所以这个变更就留在了流的内部。
    let stream = futures::stream::iter(events_since(resume_from))
        .scan(resume_from + 1, |next_id, payload| {
            let id = *next_id;
            *next_id += 1;
            futures::future::ready(Some((id, payload)))
        })
        .map(|(id, payload)| {
            SseEvent::json("activity", &payload)
                .expect("payload is a Serialize value")
                .with_id(id.to_string())
        });

    Ok(HttpResponse::sse(stream))
}
```

`sse::last_event_id(&Request) -> Option<String>` 会在请求头缺失**或者**这个值包含一个 NUL 字节时返回 `None`（按照 WHATWG 规范，NUL 会让一个
last-event-id 失效，浏览器的解析器会丢弃它）。返回的这个 `String` 在其他方面就是不透明的用户输入 - 在使用它之前，把它解析成您自己的游标 / 序号 /
偏移量。

## 领域层面的错误

`SseEvent::error("...")` 产出常规的 `event: error\ndata: <msg>\n\n` 形状。订阅者可以把它和浏览器在传输失败时触发的连接层面的 `error` 分开来监听：

```js
const es = new EventSource("/stream");

// 连接 / 传输错误（没有 `data`）。
es.onerror = (evt) => console.warn("transport error", evt);

// 由 SseEvent::error(...) 发出的领域层面的错误。
es.addEventListener("error", (evt) => console.error("server-side:", evt.data));
```

在把一个 `Stream<Item = Result<T, E>>` 映射到 `Stream<Item = SseEvent>` 时，惯用的模式是
`map(|r| match r { Ok(x) => SseEvent::json(...), Err(e) => SseEvent::error(...) })` -
面向消费者一侧的错误映射，留在生产者自己手里，框架永远不必去发明一个默认形状。

## 把一条流广播给多个订阅者

对多个 SSE 订阅者的扇出，已经由[广播子系统](broadcasting.md)覆盖了：订阅一个 `BroadcastHub` 频道，再用
`tokio_stream::wrappers::BroadcastStream` + `.map(...)`，把这个
`broadcast::Receiver` 适配成 `SseEvent` 流。每个连接都拿到自己的接收端；这个中枢处理慢消费者策略（当一个订阅者落后时的 `Lagged(n)` 错误），而您决定要怎么把这一点呈现给客户端。

`app/src/controllers/sse_example.rs` 里那个实际可运行的自用示例，用大约
25 行实现了这一点：

```rust
use futures::StreamExt;
use std::sync::Arc;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio_stream::wrappers::BroadcastStream;

pub async fn stream(_req: Request) -> Response {
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .expect("BroadcastHub not bootstrapped");
    let rx = hub.subscribe("user_registered");

    let stream = BroadcastStream::new(rx).map(|result| match result {
        Ok(envelope) => SseEvent::json("user.registered", &envelope.data)
            .unwrap_or_else(|_| {
                SseEvent::data(envelope.data.to_string())
                    .with_event("user.registered")
            }),
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
            SseEvent::data(n.to_string()).with_event("lagged")
        }
    });

    Ok(HttpResponse::sse(stream))
}
```

`lagged` 事件让客户端可以触发一次完整的重新拉取和恢复 - 这个连接会在这段延迟期间一直保持打开。

## 生产环境搭建

### 响应头

`HttpResponse::sse(...)` 会替您设置好所需的响应头：

| 响应头 | 值 | 原因 |
|--------|-------|-----|
| `Content-Type` | `text/event-stream` | 规范定义的；浏览器的 `EventSource` 要求它 |
| `Cache-Control` | `no-cache` | 阻止中间环节缓存这条流 |
| `Connection` | `keep-alive` | HTTP/1.1 的长期存活响应 |
| `X-Accel-Buffering` | `no` | 禁用 nginx 的代理缓冲 - 事件立刻刷写。在非 nginx 环境下是空操作 |

### 调整重连行为

浏览器默认的重连延迟是 3 秒。在流的开头发送一次 `retry:` 字段来覆盖它：

```rust
let preamble = SseEvent::data("ready").with_retry(Duration::from_secs(5));
```

按照规范，`Duration::ZERO` 是合法的（“立即重连”），并且会被原样发出 - 不做任何强制转换。对生产环境的流来说，5 到 15 秒的重连延迟，在快速恢复和不在区域性故障期间锤炸服务器之间取得了平衡。

### 为什么 Suprnova 有所不同

Laravel 把 SSE 发布成 `Response` 上的一个一次性辅助函数：
`Response::eventStream(fn () => ...)` 接受一个生成器风格的、会 yield 值的闭包，并把每一个被 yield 的值成帧为一个 `data:` 行。它没有把 `event:` /
`id:` / `retry:` 建模成一等字段，没有内置的保活原语，也不会对会在传输格式里注入额外字段的值做净化。

Suprnova 把 SSE 当作一个真正的子系统，而不是一个一次性的辅助函数：

- `SseEvent` 是一个带有可失败（`try_with_*`）和不可失败（`with_*`）构建器的类型化值，有明确区分的 `Frame` 和 `Comment` 两种，以及每一个单行字段上都有文档记录的净化契约。
- `HttpResponse::sse(stream)` 接入的是任何其他长期存活响应都会用到的那同一条 `stream_bytes` 响应体流水线，所以 SSE 和框架的其余部分共享同一条取消、响应头和 panic 隔离的路径。
- 生产者可以组合任何 `Stream<Item = SseEvent>` - `tokio::sync::mpsc`、
  `tokio::sync::broadcast`、`futures::stream::iter`，或者
  [BroadcastHub](broadcasting.md) 的扇出适配器。这些都不需要一个框架层面的脱围机制。
- 一个 `Last-Event-ID` 读取器（`sse::last_event_id`）和 WHATWG 的 NUL 丢弃规则都在箱子里，所以断开后恢复只是一次解析调用之遥，而不是每个应用各自一个自定义的请求头工具函数。

## 参考

| 符号 | 用途 |
|--------|------|
| `suprnova::sse::SseEvent` | SSE 流上一段可发出的内容。两种：`Frame`（带可选 `event` / `id` / `retry` + `data` 的事件）和 `Comment`（保活）。 |
| `SseEvent::data(text)` | 构建一个只有 `data:` 行的 frame。 |
| `SseEvent::json(event, &payload)` | 构建一个载荷是经过 `serde_json` 序列化的 `payload` 的 frame；把 `event:` 设为 `event`。返回 `Result<Self, serde_json::Error>`。 |
| `SseEvent::error(message)` | 构建一个带 `event: error`、以提供的消息为 `data` 的 frame。 |
| `SseEvent::comment(text)` | 构建一个仅含注释的事件（`: <text>\n\n`）。浏览器不可见；让代理保持清醒。 |
| `SseEvent::keep_alive()` | 空注释 `:\n\n` 的简写。字节数最少的心跳。 |
| `.with_event(name)` / `.with_id(id)` / `.with_retry(Duration)` | `Frame` 上不可失败的构建器；在 `Comment` 上是静默空操作。在 `to_wire()` 时剥除 CR / LF / NUL，并触发一次结构化的 WARN。 |
| `.try_with_event(name)` / `.try_with_id(id)` | 可失败的对应函数 - 在 CR / LF / NUL 上返回 `Err(FrameworkError::validation(...))`。当这个值来自用户输入、并且您想要一个 4xx 而不是静默剥除时使用。 |
| `.event()` / `.id()` / `.retry()` / `.payload()` / `.is_comment()` / `.comment_text()` | 访问器。`payload()` 这样命名，是为了避免和 `data` 构造函数冲突。 |
| `SseEvent::to_wire()` | 序列化成 SSE 传输格式的 `Bytes`。公开的，这样测试和适配器就能编码，而不必跨越响应构建器。 |
| `suprnova::sse::last_event_id(&Request) -> Option<String>` | 读取 `Last-Event-ID` 请求头。在缺失**或者**这个值包含一个 NUL 字节时返回 `None`（WHATWG 会丢弃无效的 id）。 |
| `suprnova::sse::last_event_id_from_value(Option<&str>)` | 暴露同一套验证契约的纯函数辅助 - 不必构建一个 `Request` 就能做单元测试。 |
| `HttpResponse::sse(stream)` | 从任何 `Stream<Item = SseEvent> + Send + Sync + 'static` 构建一个流式响应。设置 `Content-Type`、`Cache-Control`、`Connection`、`X-Accel-Buffering`。 |

## 下一步

- [WebSocket](websockets.md) - 另一种长期存活的连接，当您需要双向或二进制帧时。
- [广播](broadcasting.md) - 与 WebSocket 订阅者共享的 `BroadcastHub` 扇出。
- [通知](notifications.md) - 非流式推送投递的通道驱动程序（邮件、数据库、广播）。
- [Web 推送](web-push.md) - 在没有打开的 `EventSource` 时，也能触达客户端的服务器推送通知。
- [响应](responses.md) - `HttpResponse` 构建器表面的其余部分。
