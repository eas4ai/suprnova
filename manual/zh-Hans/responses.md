# 响应

每一个 Suprnova 处理程序都返回一个 `Response`，它是
`Result<HttpResponse, HttpResponse>` 的别名。`Ok` 分支携带成功的响应，`Err` 分支携带一个已经渲染好的错误响应，而 `?` 运算符会在沿途把任何拥有通向 `HttpResponse` 的 `From` 实现的错误类型都折叠进
`HttpResponse`。本章是构建 `Ok` 那一侧的实用参考 - `HttpResponse` 构建器、
`Redirect` 构建器、cookie 接口，以及 `abort_*` 短路函数。关于错误的处理方式，请参见[错误模型](error-model.md)和
[错误处理](errors.md)。

## `HttpResponse` 构建器

`HttpResponse` 是按网络层面的形态定义的响应类型。构造函数会设好合理的默认值；可链式调用的设值方法则用来覆盖它们。

### 响应体构造函数

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn examples() -> Response {
    // text/plain
    let _ = HttpResponse::text("OK");

    // application/json (any serde_json::Value)
    let _ = HttpResponse::json(json!({ "ok": true }));

    // text/html; charset=utf-8
    let _ = HttpResponse::html("<h1>Hello</h1>");

    // 带显式内容类型的原始字节 - 用于 JSON:API 序列化，
    // 以及任何其他非 JSON 的字节响应体。
    let _ = HttpResponse::bytes_body(b"PNG...".to_vec(), "image/png");

    Ok(HttpResponse::text("done"))
}
```

针对长期存活的响应，有两个流式构造函数：

- `HttpResponse::sse(stream)` - Server-Sent 事件。它包装一个由
  `SseEvent` 值构成的 `Stream`，设置四个必需的响应头（`Content-Type: text/event-stream`、`Cache-Control: no-cache`、
  `Connection: keep-alive`、`X-Accel-Buffering: no`），并且在生产数据的那个流结束之前一直保持连接打开。参见 [Server-Sent 事件](sse.md)。
- `HttpResponse::stream_bytes(stream)` - 通用的分块响应。它接受一个 `Stream<Item = Result<Bytes, Infallible>>`。错误类型是
  `Infallible`，这是有意为之：框架里的每一个生产者都会在流结束之前，把自己的错误变成流上的一条终止消息，因为在响应进行到一半时，根本没有办法把传输层面的错误呈现给客户端。
- `HttpResponse::event_stream(stream, end)` - Laravel 的
  `ResponseFactory::eventStream`。它包装一个由 `sse::StreamedEvent` 值构成的
  `Stream`，将每个事件构造成 `event: update`（或事件自身的名称），并附加一个可配置的终止帧。参见 [Server-Sent 事件](sse.md)。
- `HttpResponse::stream_json(stream)` - Laravel 的
  `ResponseFactory::streamJson`。它包装任意 `Serialize` 值的 `Stream`，并将其作为逐步构建的 JSON 数组刷新，而不是先缓冲整个集合。参见
  [Server-Sent 事件](sse.md#event-stream-and-stream-json)。

### 状态码、响应头、cookie

每一个构建器方法都返回 `Self`，所以可以放心地链式调用：

```rust
use suprnova::{Cookie, HttpResponse, Response};
use serde_json::json;

pub async fn created() -> Response {
    Ok(HttpResponse::json(json!({ "id": 42 }))
        .status(201)
        .header("X-Resource-Id", "42")
        .cookie(Cookie::new("last_id", "42")))
}
```

| 方法 | 行为 |
|---|---|
| `.status(code)` | 设置 HTTP 状态码。落在 `100..=599` 之外的码，会在网络边界上降级为 500，并留下一条警告日志。 |
| `.header(name, value)` | 追加一个响应头。允许重复（与 `Set-Cookie` 的语义一致）。 |
| `.replace_header(name, value)` | 丢掉此前所有的同名项，只设置一个。 |
| `.with_headers([(k, v), ...])` | 一次追加多个。接受任何 `IntoIterator<Item = (K, V)>`。 |
| `.without_header(name)` | 移除每一个同名项（大小写不敏感）。 |
| `.header_value(name)` | 读回最先设置的那个值。在测试里很有用。 |
| `.cookie(Cookie)` | 以 `Set-Cookie` 的形式附加一个 cookie。 |
| `.with_cookies([Cookie, ...])` | 附加多个。 |
| `.without_cookie(name)` | 安排一次删除（等价于 `Cookie::forget(name)`）。 |

同样这些可链式调用的设值方法，也通过 `ResponseExt` trait 提供给 `Response`（也就是那个 `Result`），这样那些宏用起来依然顺手：

```rust
use suprnova::{json_response, Cookie, Response, ResponseExt};

pub async fn list() -> Response {
    json_response!({ "ok": true })
        .status(200)
        .header("X-Total-Count", "42")
        .cookie(Cookie::new("last_query", "list"))
}
```

`ResponseExt` 暴露了 `.status`、`.header`、`.with_headers`、
`.without_header`、`.cookie`、`.with_cookies` 和 `.without_cookie`。

### 网络边界上的校验

`HttpResponse::into_hyper` 在把响应交给 hyper 之前，会跑两道安全过滤：

- **状态码范围。** 任何落在 `100..=599` 之外的码都会降级为 500，并附带一条
  `tracing::warn!`。这会在边界上抓住 `AppError::status(700)` 这类笔误，而不是让不合规的状态码真的发到网络上。
- **响应头的 CRLF 注入。** 每一个响应头的名字和值，都会经由 hyper 自己的
  `HeaderName::try_from` / `HeaderValue::try_from` 做校验。任何被拒绝的响应头都会被丢弃并留下一条 warn 日志，响应则在不带它的情况下构建出来。那些由攻击者控制、又被回显进某个响应头的值（CORS 的 allow-headers、`X-Forwarded-*`、自定义的调试响应头），无法把响应拆开。

这两道过滤在成功路径上是静默的 - 只有当有东西试图溜过去时，您才会在日志里看到它们。

## 响应宏

针对常见场景，有两个 `Response` 形态的宏：

```rust
use suprnova::{json_response, text_response, Response};

pub async fn json_handler() -> Response {
    json_response!({ "users": [{ "id": 1, "name": "Alice" }] })
}

pub async fn text_handler() -> Response {
    text_response!("OK")
}
```

两者都展开为 `Ok(HttpResponse::...)`。在其中任意一个上链式调用 `ResponseExt` 的设值方法，就能调整状态码、响应头或 cookie。

## Cookie

`Cookie::new(name, value)` 产出一个带安全默认值的 cookie - `HttpOnly`、`Secure`、
`SameSite=Lax`、`Path=/`。可以逐个 cookie 地覆盖：

```rust
use suprnova::Cookie;
use std::time::Duration;

let session = Cookie::new("session_id", "abc123")
    .http_only(true)
    .secure(true)
    .same_site(suprnova::SameSite::Strict)
    .path("/")
    .domain("example.com")
    .max_age(Duration::from_secs(3600))
    .partitioned(true);
```

四个便捷构造函数覆盖了常见的用法：

- `Cookie::forget(name)` - 空值，`Max-Age=0`，路径为 `/`，没有域名。登出时使用它，指示浏览器丢弃这个 cookie。
- `Cookie::forget_with(name, path, domain)` - 带作用域的形式。只有删除 cookie 的
  `Path` 和 `Domain` 与设置它时的值相匹配，浏览器才会丢弃原 cookie，因此用
  `Path=/admin` 或 `Domain=.example.com` 设置的 cookie 不会被普通的 `forget` 删除。任一参数都可以传入 `None`，以保留默认值。
- `Cookie::forever(name, value)` - 五年的 `Max-Age`。
- `Cookie::encrypted(name, plaintext)` - 写入 AES-256-GCM 密文，其 AAD 绑定到 cookie 的逻辑名称。使用相同名称，通过 `Cookie::read_encrypted_for(name, wire)` 读取。`Cookie::read_encrypted(wire)` 是已弃用的、无上下文的 v1 读取器；它无法解密当前 `Cookie::encrypted` 的输出，并计划在 1.4.0 与 v1 回退一起移除。要求启动时设置 `APP_KEY`。参见[加密](encryption.md)。

一次移除多个 cookie - 常见的登出形式 - 可以使用 `without_cookies`；它在
`HttpResponse`、通过 `ResponseExt` 提供的 `Response`，以及两个重定向构建器上都可用：

```rust
use suprnova::{HttpResponse, Redirect};

let _ = HttpResponse::text("bye").without_cookies(["session", "remember"]);
let _: suprnova::Response = Redirect::to("/login")
    .without_cookies(["session", "remember"])
    .into();
```

在重定向上，删除操作会随 302 响应本身发送，而不是随目标发送，因此浏览器在跟随
`Location` 之前就已经删除了这些 cookie。

响应头的序列化，会把每一个按 RFC 6265 不算合法 cookie-octet 的字节都做百分号编码，控制字符也全都包含在内。cookie 名字或值里的 CRLF 会被编码，而不会被原样传下去 -
通过 cookie 做响应头注入这条路，在序列化器那里就被堵死了。

### 稍后排队 cookie

有时，不负责构建响应的代码仍然需要设置 cookie - 例如响应事件的监听器、在处理程序之前运行的一段中间件，或者一个作用域中没有 `HttpResponse` 的 `App::bind` 服务。
`Cookie::queue` 就是 Laravel 的 `Cookie::queue()`：它把 cookie 暂存到每个请求独有的
jar 中，`SessionMiddleware` 会在输出响应时、紧跟在会话 cookie 之后将其排空到响应中。

```rust
use suprnova::Cookie;

Cookie::queue(Cookie::new("theme", "dark"));

// 查看排队中的内容。
let queued = Cookie::queued("theme");

// 在响应发出去之前把它移除。
Cookie::unqueue("theme");

// 排入一次删除，而不是一个值 - 可以和 `forget_with` 组合。
Cookie::expire("theme", Some("/app"), None);
```

这个 jar 是任务本地的，并且每个请求都会新建为空 - 在一个请求中排队的内容在下一个请求中不可见；如果某个值已排队但始终没有被排空（路由链中没有 `SessionMiddleware`），它会被丢弃，而不会触发 panic。排队的 cookie 会附加到处理程序返回的任何响应上，包括重定向：排队 cookie 后返回 `Redirect::to(...)` 的处理程序仍会在 3xx 响应上携带
`Set-Cookie` 响应头。它们也会附加到 `SessionMiddleware` 为请求中途的内部失败自行构建的
500 响应上 - 例如现有会话无法读取、会话写入失败，或会话 cookie 加密失败 - 因为排队的
cookie 可能已经代表在别处提交的副作用（例如 remember-me 令牌行已经写入），所以报告失败的响应仍会携带它。它们**不会**跨越 panic 存活 - `SessionMiddleware` 的排空代码会在处理程序正常返回后运行，而被捕获的 panic 会在整个中间件链之外转换为 500，这正是
Laravel 自己排队的 cookie 在未捕获异常中丢失的同一位置。

### 为什么 Suprnova 有所不同

Laravel 的 `CookieJar` 按名称*和*路径为队列建立键，因此同名但路径不同的两个 cookie
可以独立排队。

Suprnova 的 jar 只按名称建立键：如果某个名称已经排队，再为这个名称排队的 cookie 会替换前一个 cookie，而不是为它追加第二条 `Set-Cookie` 行。这覆盖了常见情况 - 一个调用位置负责一个给定的 cookie 名称 - 同时无需 Laravel 版本所需的额外按路径查找。

## 重定向

`Redirect` 覆盖了 Laravel 重定向器的整个接口。每一个变体都实现了 `From<Redirect> for Response`，所以惯用的写法是 `Redirect::...().into()`。

### 目标

```rust
use suprnova::{Redirect, redirect_to};

// 显式的 URL 或路径
let _ = Redirect::to("/dashboard");

// 同样的事情，一个稍短一点的自由函数
let _ = redirect_to("/dashboard");

// 具名路由（返回 RedirectRouteBuilder）
let _ = Redirect::route("users.show").with("id", "42");

// 显式的外部 URL - 和 `to` 相同，但这个名字向开放重定向审计
// 表明“这是要跳出站点的”
let _ = Redirect::away("https://external.example.com");

// 刷新页面（从会话里读取上一个 URL；没有活跃的会话作用域时，
// 回退到 "/"）
let _ = Redirect::refresh();

// 同上，但在没有活跃作用域时接收一个显式的 Request
// let _ = Redirect::refresh_for(&request);

// 会话里的 previous_url，作用域中没有会话时使用回退值
let _ = Redirect::back("/login");

// 会话里存下的原定 URL，读取时被消耗，并带一个回退值
let _ = Redirect::intended("/home");

// 访客重定向：把当前请求的 URL 存为“原定”目标，
// 并把用户送到一个登录页面
// let _ = Redirect::guest(&request, "/login");
```

`Redirect::back`、`Redirect::intended`、`Redirect::guest` 和 `Redirect::refresh` 都与会话集成。在没有会话作用域时，它们会静默地落到各自的默认值上 - 这对只搭了一半的测试环境很方便。参见[会话](session.md)。

`Redirect::back` 的目标（会话记录的上一个 URL）绝不会原样信任。会话中间件一开始就只记录根相对、同源的 URL（以 `//` 或 `/\` 开头的路径，或其中任何位置带有 ASCII 控制字节的路径，绝不会被存储），并且每次读取时都会再次执行相同的检查，因此无论是请求以异常路径到达应用，还是在这个防护措施存在之前写入的会话 cookie，`back` 都无法被引导到跨源位置。完整规则请参见[会话](session.md#other-operations)。

### 命名路由校验

`redirect!` 过程宏会在编译期校验路由名，并展开为 `Redirect::route(name)`：

```rust
use suprnova::{redirect, Response};

pub async fn store() -> Response {
    // 如果 "users.index" 不是一个已注册的路由名，编译就会失败；
    // 错误消息会列出可用的路由，并给出相近的候选。
    redirect!("users.index").into()
}
```

### 状态码

```rust
use suprnova::Redirect;

let _ = Redirect::to("/x").permanent();      // 301
let _ = Redirect::to("/x").status(303);      // 303, 307, 308, ...
```

默认是 302。

### flash 数据

重定向构建器带着自己的 flash bag。在转换为 `Response` 时，这个 bag 会被排空并写入当前活跃的会话，正好再多存活一个请求：

```rust
use suprnova::Redirect;

let _ = Redirect::back("/users/new")
    .with("status", "User created")            // 单个键/值
    .with_input([                              // 重新填充表单
        ("email", "shawn@example.com"),
        ("name", "Shawn"),
    ])
    .with_errors([                             // default 错误包
        ("email", "Must be unique"),
    ])
    .with_errors_bag("login", [                // named 错误包
        ("password", "Required"),
    ]);
```

接收页面通过 `session.get(...)`（对应 `with`）、`session.get_old_input(...)`（对应
`with_input`），以及由 `session.pull_errors_flash()` 排空的 bag 映射（对应
`with_errors` / `with_errors_bag`）把这些值读回来。Inertia 层会自动消费 errors flash -
每一个 Inertia 响应的 `errors` prop 都是从会话里播种出来的，所以
`Redirect::back().with_errors(...)` 不需要额外接线，就能把消息呈现在目的页面上。对于多表单的页面，`X-Inertia-Error-Bag` 请求头会把这个 prop 收拢到一个具名的 bag 之下。

注意，在 `RedirectRouteBuilder`（也就是 `Redirect::route` 和 `redirect!` 返回的东西）上，
`.with(key, value)` 设置的是一个**路由参数**，而不是一条 flash 记录 - 在那里请改用
`.flash(key, value)`：

```rust
use suprnova::redirect;

let _ = redirect!("users.show")
    .with("id", "42")                          // 路由参数
    .flash("status", "Updated");               // 会话 flash
```

### Cookie、响应头、片段

```rust
use suprnova::{Cookie, Redirect};

let _ = Redirect::route("billing.show")
    .with_cookies([Cookie::new("welcome", "yes")])
    .with_headers([("X-Trace", "abc")])
    .with_fragment("invoices")                 // 追加 #invoices
    .without_fragment();                       // 或者剥掉之前任何的片段
```

`with_fragment` 既接受带前导 `#` 的片段，也接受不带的。在 `without_fragment` 之后再调用 `with_fragment`，会重新挂上一个。

### 让片段跨越重定向保留下来

对于那些希望目的地保留*发起方* URL hash 的 Inertia 应用，请使用 `preserve_fragment`：

```rust
use suprnova::Redirect;

let _ = Redirect::route("dashboard.index").preserve_fragment();
```

转换时，这会把 `_inertia.preserve_fragment = true` 作为 flash 写进会话；下一个 Inertia
响应会读到这个标志，并在它的页面对象里发出 `preserveFragment: true`。没有会话作用域 -
这个标志就被静默丢弃。

### 签名重定向

有两个构建器把 URL 签名接口包了起来，用于一次性地重定向到命名路由（密码重置、邮箱验证、下载链接）：

```rust
use suprnova::Redirect;

let r = Redirect::signed_route("downloads.show", &[("id", "42")])?;
let r = Redirect::temporary_signed_route(
    "downloads.show",
    &[("id", "42")],
    1_700_000_000, // expires_at_epoch_seconds
)?;
```

两者都返回 `Result<Redirect, FrameworkError>` - 用 `?` 把错误传播出去即可，因为
`Redirect` 能干净地转换为 `Response`。签名那套接口请参见 [URL 生成](urls.md)。

### 存下原定的 URL

`Redirect::set_intended_url` 会写入会话里那个原定的目标，而不真的执行一次重定向 - 通常是在认证中间件里、重定向到 `/login` 之前调用它，这样后面的一次 `Redirect::intended` 就能取回最初被请求的那个 URL：

```rust
suprnova::Redirect::set_intended_url("/admin/users");
```

## 从处理程序中止

有三个自由函数可以在给定的状态码上把处理程序短路掉。它们返回 `Result<(), FrameworkError>`；配合 `?` 使用：

```rust
use suprnova::{abort_if, abort_unless, abort_with, json_response, Request, Response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    json_response!({ "ok": true })
}
```

底层的错误是 `FrameworkError::Domain { message, status_code }`，所以它会走和其他每一条错误路径相同的 JSON 错误包与 5xx 清理规则来渲染。超出范围的状态码会被响应渲染器强制转为 500。完整的转换契约请参见[错误模型](error-model.md)。

## 直接返回错误

因为 `Response` 就是 `Result<HttpResponse, HttpResponse>`，所以您可以直接返回一个 `Err` 分支 -
当响应形态本来就是某个特定的 JSON 响应体、而您想让它原样发到网络上时，这很有用：

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn legacy_lookup() -> Response {
    Err(HttpResponse::json(json!({
        "error": "deprecated endpoint",
    })).status(410))
}
```

想要更丰富的东西 - 类型化的领域错误、验证、可观测性 - 请使用[错误模型](error-model.md)
那套接口（`AppError`、`FrameworkError`、`#[domain_error]`）。

## 快速参考

| 需要什么 | 用什么 |
|---|---|
| JSON 响应 | `HttpResponse::json(v)` 或 `json_response!({...})` |
| 文本响应 | `HttpResponse::text(s)` 或 `text_response!(s)` |
| HTML 响应 | `HttpResponse::html(s)` |
| 原始字节 + 内容类型 | `HttpResponse::bytes_body(b, "image/png")` |
| Server-Sent 事件 | `HttpResponse::sse(stream)` - 参见 [SSE](sse.md) |
| 分块流 | `HttpResponse::stream_bytes(stream)` |
| 设置状态码 | `.status(code)` |
| 添加响应头 | `.header(k, v)` / `.with_headers([...])` |
| 移除响应头 | `.without_header(name)` |
| 附加 cookie | `.cookie(c)` / `.with_cookies([...])` |
| 遗忘 cookie | `.without_cookie(name)` / `.without_cookies([...])` |
| 遗忘路径/域名作用域的 cookie | `Cookie::forget_with(name, Some("/admin"), Some("example.com"))` |
| 为下一个响应排队一个 cookie | `Cookie::queue(c)` |
| 查询已排队的 cookie | `Cookie::queued(name)` |
| 从队列中移除 cookie | `Cookie::unqueue(name)` |
| 排队一个删除 cookie | `Cookie::expire(name, path, domain)` |
| 简单重定向 | `Redirect::to(path).into()` 或 `redirect_to(path).into()` |
| 重定向到命名路由 | `redirect!("name").into()` 或 `Redirect::route("name")` |
| 退回上一页的重定向 | `Redirect::back(fallback)` |
| 重定向到原定 URL | `Redirect::intended(default)` |
| 访客重定向（暂存原定 URL） | `Redirect::guest(&req, login)` |
| 设置原定目标 | `Redirect::set_intended_url(url)` |
| 外部 URL | `Redirect::away(url)` |
| 刷新当前页面 | `Redirect::refresh()` / `Redirect::refresh_for(&req)` |
| 重定向到签名路由 | `Redirect::signed_route(name, &[(k, v)])?` |
| 重定向上的路由参数 | `.with("key", "value")` |
| 重定向上的查询参数 | `.query("key", "value")` |
| flash 数据 | `.with(key, value)`（在 `RedirectRouteBuilder` 上则是 `.flash`） |
| flash 输入 | `.with_input([(k, v), ...])` |
| flash 错误 | `.with_errors([(k, msg), ...])` |
| 具名的错误包 | `.with_errors_bag(bag, [(k, msg)])` |
| 追加片段 | `.with_fragment("section")` |
| 剥掉片段 | `.without_fragment()` |
| 保留片段（Inertia） | `.preserve_fragment()` |
| 永久重定向 | `.permanent()`（301） |
| 自定义重定向状态码 | `.status(303)` |
| 提前中止 | `abort_with(code, msg)?`、`abort_if(cond, code, msg)?`、`abort_unless(cond, code, msg)?` |

## 下一步

- [错误模型](error-model.md) - `FrameworkError`、`AppError`、
  `HttpError`，以及把每一个错误渲染成 `HttpResponse` 的那唯一一次转换
- [错误处理](errors.md) - 针对 `?`、`AppError` 和自定义领域错误的实用处理程序模式
- [Server-Sent 事件](sse.md) - 构建并消费 `sse(...)` 响应
- [URL 生成](urls.md) - 签名 URL、命名路由解析，以及
  `Redirect::signed_route` 背后的那套接口
- [会话](session.md) - flash 数据、原定 URL，以及
  `Redirect::with`/`with_input`/`with_errors` 写入的那个 bag
