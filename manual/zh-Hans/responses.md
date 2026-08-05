# 响应

每一个 Suprnova 处理程序都返回一个 `Response`，它是 `Result<HttpResponse, HttpResponse>` 的别名。`Ok` 分支携带成功的响应，`Err` 分支携带一个已经渲染好的错误响应，而 `?` 运算符会在沿途把任何拥有通向 `HttpResponse` 的 `From` 实现的错误类型都折叠进来。本章是构建 `Ok` 那一侧的实用参考 - `HttpResponse` 构建器、`Redirect` 构建器、cookie 接口，以及 `abort_*` 短路函数。错误那一侧的处理方式请参见[错误模型](error-model.md)和[错误处理](errors.md)。

## `HttpResponse` 构建器

`HttpResponse` 是按网络层面的形态定义的响应类型。构造函数会设好合理的默认值；可链式调用的设值方法则用来覆盖它们。

### 响应体构造函数

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn examples() -> Response {
    // text/plain
    let _ = HttpResponse::text("OK");

    // application/json（任何 serde_json::Value）
    let _ = HttpResponse::json(json!({ "ok": true }));

    // text/html; charset=utf-8
    let _ = HttpResponse::html("<h1>Hello</h1>");

    // 带显式内容类型的原始字节 - 供 JSON:API 序列化
    // 以及任何其他非 JSON 的字节响应体使用。
    let _ = HttpResponse::bytes_body(b"PNG...".to_vec(), "image/png");

    Ok(HttpResponse::text("done"))
}
```

针对长期存活的响应，还有两个流式构造函数：

- `HttpResponse::sse(stream)` - Server-Sent 事件。它包装一个由 `SseEvent` 值构成的 `Stream`，设置四个必需的响应头（`Content-Type: text/event-stream`、`Cache-Control: no-cache`、`Connection: keep-alive`、`X-Accel-Buffering: no`），并且在生产数据的那个流结束之前一直保持连接打开。参见 [Server-Sent 事件](sse.md)。
- `HttpResponse::stream_bytes(stream)` - 通用的分块响应。它接受一个 `Stream<Item = Result<Bytes, Infallible>>`。错误类型是 `Infallible`，这是有意为之：框架里的每一个生产者都会在流结束之前，把自己的错误变成流上的一条终止消息，因为在响应进行到一半时，根本没有办法把传输层面的错误呈现给客户端。

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

`ResponseExt` 暴露了 `.status`、`.header`、`.with_headers`、`.without_header`、`.cookie`、`.with_cookies` 和 `.without_cookie`。

### 网络边界上的校验

`HttpResponse::into_hyper` 在把响应交给 hyper 之前，会跑两道安全过滤：

- **状态码范围。** 任何落在 `100..=599` 之外的码都会降级为 500，并附带一条 `tracing::warn!`。这会在边界上抓住 `AppError::status(700)` 这类笔误，而不是让不合规的状态码真的发到网络上。
- **响应头的 CRLF 注入。** 每一个响应头的名字和值，都会经由 hyper 自己的 `HeaderName::try_from` / `HeaderValue::try_from` 做校验。任何被拒绝的响应头都会被丢弃并留下一条 warn 日志，响应则在不带它的情况下构建出来。那些由攻击者控制、又被回显进某个响应头的值（CORS 的 allow-headers、`X-Forwarded-*`、自定义的调试响应头），无法把响应拆开。

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

`Cookie::new(name, value)` 产出一个带安全默认值的 cookie - `HttpOnly`、`Secure`、`SameSite=Lax`、`Path=/`。可以逐个 cookie 地覆盖：

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

三个便捷构造函数覆盖了常见的用法：

- `Cookie::forget(name)` - 空值，`Max-Age=0`。在登出时用它来指示浏览器丢弃这个 cookie。
- `Cookie::forever(name, value)` - 五年的 `Max-Age`。
- `Cookie::encrypted(name, plaintext)` - AES-256-GCM 密文，绑定在 `CryptPurpose::Cookie` 这个 AAD 上，因此 cookie 的密文无法被重放到框架的另一个接口上（游标、2FA 密钥、模型属性转换）。要求启动时设置了 `APP_KEY`。配套的 `Cookie::read_encrypted(wire)` 会解密由同一条路径产出的值。参见[加密](encryption.md)。

响应头的序列化，会把每一个按 RFC 6265 不算合法 cookie-octet 的字节都做百分号编码，控制字符也全都包含在内。cookie 名字或值里的 CRLF 会被编码，而不会被原样传下去 - 通过 cookie 做响应头注入这条路，在序列化器那里就被堵死了。

## 重定向

`Redirect` 覆盖了 Laravel 重定向器的整个接口。每一个变体都实现了 `From<Redirect> for Response`，所以惯用的写法是 `Redirect::...().into()`。

### 目标

```rust
use suprnova::{Redirect, redirect_to};

// 显式的 URL 或路径
let _ = Redirect::to("/dashboard");

// 同一件事，写法稍短的自由函数
let _ = redirect_to("/dashboard");

// 命名路由（返回 RedirectRouteBuilder）
let _ = Redirect::route("users.show").with("id", "42");

// 显式的外部 URL - 与 `to` 相同，但这个名字是在向开放重定向审计
// 表明“这是要去站外的”
let _ = Redirect::away("https://external.example.com");

// 刷新页面（从会话里读取上一个 URL；如果没有活跃的会话作用域，
// 就回退到 "/"）
let _ = Redirect::refresh();

// 同样的功能，但在没有活跃作用域时接受一个显式的 Request
// let _ = Redirect::refresh_for(&request);

// 会话里的 previous_url，在作用域中没有会话时使用兜底值
let _ = Redirect::back("/login");

// 会话里存着的原定 URL，读取时被消费掉，带兜底值
let _ = Redirect::intended("/home");

// 访客重定向：把当前请求的 URL 暂存为“原定”目标，
// 并把用户送到登录页
// let _ = Redirect::guest(&request, "/login");
```

`Redirect::back`、`Redirect::intended`、`Redirect::guest` 和 `Redirect::refresh` 都与会话集成。在没有会话作用域时，它们会静默地落到各自的默认值上 - 这对只搭了一半的测试环境很方便。参见[会话](session.md)。

### 命名路由校验

`redirect!` 过程宏会在编译期校验路由名，并展开为 `Redirect::route(name)`：

```rust
use suprnova::{redirect, Response};

pub async fn store() -> Response {
    // 如果 "users.index" 不是一个已注册的路由名，编译就会失败；
    // 错误信息会列出可用的路由，并给出相近的候选。
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
    .with("status", "User created")            // 单个键 / 值
    .with_input([                              // 回填表单
        ("email", "shawn@example.com"),
        ("name", "Shawn"),
    ])
    .with_errors([                             // 默认的错误包
        ("email", "Must be unique"),
    ])
    .with_errors_bag("login", [                // 具名的错误包
        ("password", "Required"),
    ]);
```

接收页面通过 `session.get(...)`（对应 `with`）、`session.get_old_input(...)`（对应 `with_input`），以及由 `session.pull_errors_flash()` 排空的那个 bag 映射（对应 `with_errors` / `with_errors_bag`）把这些值读回来。Inertia 层会自动消费 errors flash - 每一个 Inertia 响应的 `errors` prop 都是从会话里播种出来的，所以 `Redirect::back().with_errors(...)` 不需要额外接线，就能把消息呈现在目的页面上。对于多表单的页面，`X-Inertia-Error-Bag` 请求头会把这个 prop 收拢到一个具名的 bag 之下。

注意，在 `RedirectRouteBuilder`（也就是 `Redirect::route` 和 `redirect!` 返回的东西）上，`.with(key, value)` 设置的是一个**路由参数**，而不是一条 flash 记录 - 在那里请改用 `.flash(key, value)`：

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
    .without_fragment();                       // 或者：剥掉此前的任何片段
```

`with_fragment` 既接受带前导 `#` 的片段，也接受不带的。在 `without_fragment` 之后再调用 `with_fragment`，会重新挂上一个。

### 让片段跨越重定向保留下来

对于那些希望目的地保留*发起方* URL hash 的 Inertia 应用，请使用 `preserve_fragment`：

```rust
use suprnova::Redirect;

let _ = Redirect::route("dashboard.index").preserve_fragment();
```

转换时，这会把 `_inertia.preserve_fragment = true` 作为 flash 写进会话；下一个 Inertia 响应会读到这个标志，并在它的页面对象里发出 `preserveFragment: true`。没有会话作用域 - 这个标志就被静默丢弃。

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

两者都返回 `Result<Redirect, FrameworkError>` - 用 `?` 把错误传播出去即可，因为 `Redirect` 能干净地转换为 `Response`。签名那套接口请参见 [URL 生成](urls.md)。

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

因为 `Response` 就是 `Result<HttpResponse, HttpResponse>`，所以您可以直接返回一个 `Err` 分支 - 当响应形态本来就是某个特定的 JSON 响应体、而您想让它原样发到网络上时，这很有用：

```rust
use suprnova::{HttpResponse, Response};
use serde_json::json;

pub async fn legacy_lookup() -> Response {
    Err(HttpResponse::json(json!({
        "error": "deprecated endpoint",
    })).status(410))
}
```

想要更丰富的东西 - 类型化的领域错误、验证、可观测性 - 请使用[错误模型](error-model.md)那套接口（`AppError`、`FrameworkError`、`#[domain_error]`）。

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
| 遗忘 cookie | `.without_cookie(name)` |
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

- [错误模型](error-model.md) - `FrameworkError`、`AppError`、`HttpError`，以及把每一个错误渲染成 `HttpResponse` 的那唯一一次转换
- [错误处理](errors.md) - 针对 `?`、`AppError` 和自定义领域错误的实用处理程序模式
- [Server-Sent 事件](sse.md) - 构建并消费 `sse(...)` 响应
- [URL 生成](urls.md) - 签名 URL、命名路由解析，以及 `Redirect::signed_route` 背后的那套接口
- [会话](session.md) - flash 数据、原定 URL，以及 `Redirect::with`/`with_input`/`with_errors` 写入的那个 bag
