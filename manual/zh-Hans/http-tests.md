# HTTP 测试

这一章展示了如何通过 `suprnova::handle_request` 驱动框架的请求管道，来测试您的 HTTP 表面 - 路由、中间件、认证流程、错误响应。如果您写过用 `$this->get('/users')` 断言 `$response->status()` 的 Laravel feature 测试，这就是 Suprnova 的对应物：您在生产环境里挂载的那个同一个 `Router` 会在测试里运行，每一个中间件都会触发，panic 边界依然会捕获，这个响应逐字节地就是一个真实客户端会看到的东西。

## 测试表面

恰好有三个构造块：

| 组成部分 | 角色 |
|---|---|
| `Router` | 被测试的路由 - 构建方式和生产环境一样 |
| `MiddlewareRegistry` | 全局中间件栈 - 构建方式也一样 |
| `handle_request(router, registry, req) -> hyper::Response<…>` | 进程内驱动器 - 端到端地运行一个请求 |

`handle_request` 和 `Server::run` 逐请求调用的是同一个函数，为测试和嵌入方而暴露出来。任何在生产环境里能工作的东西，在这里都能工作 - panic 恢复包装器、请求 id 作用域、Inertia flash bag 作用域、认证请求状态作用域、HEAD 请求体剥离、响应后终止。没有一个会换上一条更安静管道的“测试模式”。

`handle_request_with_peer` 是同一个调用，但带着一个显式的、给连接对端用的 `Option<std::net::IpAddr>` - 在您想断言 `Request::ip()` 的解析结果，又不想设置代理请求头时很有用。

## hyper 请求体的问题

一个值得提前知道的小麻烦：`handle_request` 接受一个 `hyper::Request<hyper::body::Incoming>`。`Incoming` 是 hyper 内部的流式请求体类型；您没法用 `Full::new(bytes)` 或者任何内存里的请求体类型来构造一个。它只能来自一条 hyper 连接。

有两种干净的绕过方式：

1. **TCP 回环** - 绑定一个 `127.0.0.1:0` 监听器，在一个
   `service_fn` 里服务一次 accept，通过一个 hyper 客户端发送这个请求，让 `Incoming` 在服务器一侧自然地产生出来。框架里每一个集成测试已经都是这么做的。
2. **进程内的 Request 构建** - 对于只需要检视 `Request` 访问器（请求头、路由参数、IP、JSON 解析），却不需要经过路由的测试，请使用同样的 TCP 回环捕获模式，但用一个把 `Request` 拽进一个
   `oneshot::channel` 而不是运行它的服务。
   `framework/tests/http_request_accessors.rs` 文件里逐字带着这个
   `build_request()` 辅助函数。

两种模式都会产出真正的 `Incoming` 请求体。这个回环是本地的，在测试的挂钟时间上是同步的（微秒级），并且从不触达 `lo` 之外的网络。没有一种既更慢又更简单、还能保住这份契约的方式。

### 为什么 Suprnova 有所不同

Laravel 的 `$this->get('/users')` 能工作，是因为 PHP 的请求生命周期是“构建一个 `Request` 对象，把它分发经过内核”。这个内核直接拿走这个内存里的对象；没有一个请求体类型会强制要求一次传输。Suprnova 的服务器构建在 hyper 之上，hyper 的请求体类型出于好的理由是有主见的（流式、背压、零拷贝）。测试表面继承了这个约束。

您为这个约束换来的是忠实度。生产请求路径的每一个细节 - 请求头解析、请求体限制、连接升级 - 在测试里都以同样的方式运行。您永远不会因为测试装置跳过了一层真实服务器会运行的东西，而让一个测试通过。

## 第一个端到端测试

这是一个完整、可运行的测试，挂载一个单独的路由，对它发送一个 GET，并对状态和请求体做断言。

```rust
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::body::Incoming;
use hyper::service::service_fn;
use hyper_util::rt::TokioIo;

use suprnova::http::text;
use suprnova::{MiddlewareRegistry, Request, Router, handle_request};

async fn spawn_server(router: Router, accepts: usize) -> SocketAddr {
    let router = Arc::new(router);
    let middleware = Arc::new(MiddlewareRegistry::new());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind ephemeral listener");
    let addr = listener.local_addr().expect("local_addr");

    tokio::spawn(async move {
        for _ in 0..accepts {
            let Ok((stream, _)) = listener.accept().await else { return };
            let io = TokioIo::new(stream);
            let router = router.clone();
            let middleware = middleware.clone();
            tokio::spawn(async move {
                let svc = service_fn(move |req: hyper::Request<Incoming>| {
                    let router = router.clone();
                    let middleware = middleware.clone();
                    async move {
                        Ok::<_, Infallible>(handle_request(router, middleware, req).await)
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(io, svc)
                    .await;
            });
        }
    });

    addr
}

async fn send_get(addr: SocketAddr, path: &str) -> (u16, Bytes) {
    let stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let io = TokioIo::new(stream);
    let (mut sender, conn) =
        hyper::client::conn::http1::handshake::<_, Full<Bytes>>(io).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });

    let req = hyper::Request::builder()
        .method("GET")
        .uri(path)
        .header("Host", "localhost")
        .header("Content-Length", "0")
        .body(Full::new(Bytes::new()))
        .unwrap();

    let resp = tokio::time::timeout(Duration::from_secs(5), sender.send_request(req))
        .await
        .expect("send_get timeout")
        .expect("hyper send_request");
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    (parts.status.as_u16(), bytes)
}

#[tokio::test]
async fn get_root_returns_hello() {
    let router = Router::new().get("/", |_req: Request| async { text("hello") });
    let addr = spawn_server(router, 1).await;

    let (status, body) = send_get(addr, "/").await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"hello");
}
```

那就是整个形态。逐 crate 复制这两个辅助函数，为您的测试套件调整它们（多次 accept、请求头捕获、请求体捕获）。框架自己在 `framework/tests/cors_middleware.rs`、`framework/tests/middleware_panic_safety.rs`，以及 `framework/tests/email_verified_middleware.rs` 里，用的都是几乎一样的辅助函数。

`accepts` 这个参数限定了这个 accept 循环在退出之前会服务多少条连接。对一个单独的请求来说，一次就够了；当一个测试要练习 panic 之后的恢复时，请调到两次或更多（见[测试 panic 边界](#测试-panic-边界)）。

## 构建一个请求

在 `send_get` 内部，您看到了：

```rust
let req = hyper::Request::builder()
    .method("GET")
    .uri("/users/42")
    .header("Host", "localhost")
    .header("Content-Length", "0")
    .body(Full::new(Bytes::new()))
    .unwrap();
```

这是那个典范形态。有几件事值得知道：

- **`Host` 请求头。** Hyper 会拒绝没有它的 HTTP/1.1 请求。请始终带上它；除非您的处理程序按它的值来做判断，否则这个值本身并不重要。
- **`Content-Length: 0`。** 匹配这个请求体。Hyper 会用
  `Full::new(Bytes::new())` 替您算出这个值，但在测试里写得明确一些，读起来更干净。
- **请求体类型。** 客户端一侧发送 `Full<Bytes>`。服务器一侧接收
  `Incoming`。您在测试里始终只构建 `Full<Bytes>` 请求；框架在经过
  hyper 的逐连接转换之后，会把它们接收成 `Incoming`。

一个带 JSON 请求体的 POST：

```rust
let body_bytes = serde_json::to_vec(&serde_json::json!({
    "name": "Alice",
    "email": "alice@example.com"
})).unwrap();

let req = hyper::Request::builder()
    .method("POST")
    .uri("/users")
    .header("Host", "localhost")
    .header("content-type", "application/json")
    .header("content-length", body_bytes.len())
    .body(Full::new(Bytes::from(body_bytes)))
    .unwrap();
```

## 对响应做断言

从 `handle_request` 回来的响应，是一个 `hyper::Response<BoxBody<Bytes, Infallible>>`。您会从它上面读出三样东西：

```rust
let (parts, body) = resp.into_parts();

// 1. 状态。
assert_eq!(parts.status.as_u16(), 200);

// 2. 请求头 - 不区分大小写的查找。
let location = parts.headers.get("location").and_then(|v| v.to_str().ok());
assert_eq!(location, Some("/login"));

// 3. 请求体 - 收集成字节，然后解析。
use http_body_util::BodyExt;
let bytes = body.collect().await.unwrap().to_bytes();

// 作为文本：
let text = String::from_utf8_lossy(&bytes);

// 作为 JSON：
let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
assert_eq!(value["message"], "ok");
```

对于错误响应，这个请求体形态是固定的，并且记录在[错误模型](error-model.md)里 - `message`、`errors`、`request_id`，以及一个可选的 `debug_message`。`request_id` 这个键总是存在的（在请求作用域之外可能是 `null`），在检查请求 id 中间件是否运行过时，断言的就是它。

## 测试中间件

中间件测试和路由测试看起来一样；唯一的区别是您在 spawn 之前，给这个注册表 `.append()` 了什么。

### 测试全局中间件

把中间件传给 `MiddlewareRegistry::new().append(...)`，并使用这个注册表 - 多个中间件按追加的顺序运行，`prepend` 会把一个新的放到最前面。

```rust
use suprnova::{CorsConfig, CorsMiddleware, MiddlewareRegistry};

fn cors_registry() -> MiddlewareRegistry {
    MiddlewareRegistry::new().append(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .allow_credentials(true)
            .max_age(std::time::Duration::from_secs(600)),
    ))
}

#[tokio::test]
async fn cors_preflight_returns_204_with_headers() {
    let router = Router::new();
    // `spawn_server` 的三参数形态，让您能接上一个非空的
// MiddlewareRegistry - 从
// framework/tests/cors_middleware.rs 复制这个辅助函数（大约 30 行）。
let addr = spawn_server(router, cors_registry(), 1).await;

    let (status, headers, _) = options(
        addr,
        "/anything",
        &[
            ("Origin", "https://app.example"),
            ("Access-Control-Request-Method", "POST"),
        ],
    ).await;

    assert_eq!(status, 204);
    assert_eq!(
        headers.get("access-control-allow-origin").map(String::as_str),
        Some("https://app.example"),
    );
}
```

这个测试证明的不只是 CORS 逻辑本身：它证明了全局中间件在**未路由**的请求上也会运行，这是框架保证的契约（否则一个永远匹配不到路由的 OPTIONS 预检，就会跳过 CORS）。完整的测试套件请参见 `framework/tests/cors_middleware.rs`。

### 测试路由特定的中间件

用路由构建器上的 `.middleware(...)` 来附加，和生产环境一模一样。然后照常测试这个路由 - 这个中间件链是从同一次注册构建出来的。

```rust
let router = Router::new()
    .get("/admin/dashboard", |_req| async { text("admin") })
    .middleware(RequireRole::new("admin"));

let (status, _) = send_get(addr, "/admin/dashboard").await;
assert_eq!(status, 403); // 未认证的请求
```

### 为已认证用户设置存根

真实的认证流程测试需要一个已登录的用户。最干净的模式是一个微小的一次性中间件，在被测试的中间件之前调用 `Auth::set_user`。框架自己的 `framework/tests/email_verified_middleware.rs` 用的就是这个：

```rust
use std::any::Any;
use std::sync::Arc;
use suprnova::{Auth, Authenticatable, Middleware, Next, Request, Response};

struct UserById(String);

impl Authenticatable for UserById {
    fn get_auth_identifier(&self) -> String { self.0.clone() }
    fn as_any(&self) -> &dyn Any { self }
}

struct LoginAs(String);

#[async_trait::async_trait]
impl Middleware for LoginAs {
    async fn handle(&self, request: Request, next: Next) -> Response {
        Auth::set_user(Arc::new(UserById(self.0.clone())));
        next(request).await
    }
}
```

然后在测试里：

```rust
let registry = MiddlewareRegistry::new()
    .append(LoginAs("user-id-123".to_string()))
    .append(EnsureEmailVerifiedMiddleware::new());
```

`LoginAs` 先运行，把这个用户装进逐请求的认证状态里，被测试的中间件就会看到 `Auth::id() == Some(...)`，而完全不需要发出一次真实的登录。这个认证状态作用域是由 `handle_request` 自己搭起来的 - 和生产环境里运行的是同一个 - 所以这个用户对之后的每一个中间件和处理程序都是可见的。

## 测试路由模型绑定

路由模型绑定会把 `/users/{id}` 变成一个类型化的 `User` 参数。这个绑定是作为处理程序提取器链的一部分运行的，所以一个普通的端到端测试就能免费练到它：

```rust
#[suprnova::model(table = "users")]
pub struct User {
    pub id: i64,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[tokio::test]
async fn show_user_binds_from_route_param() {
    // 通过这个模型插入一个测试用户。数据库设置省略 -
    // `TestDatabase` 的模式请参见测试那一章。
    let user = User::create(suprnova::attrs! {
        email: "bound@example.com"
    }).await.unwrap();

    let router = Router::new().get("/users/{id}", |req: Request| async move {
        let id: i64 = req.param("id")?.parse()
            .map_err(|_| suprnova::FrameworkError::param_parse("id", "i64"))?;
        let user = User::find_or_fail(id).await?;
        suprnova::http::json(serde_json::json!({ "email": user.email }))
    });

    let addr = spawn_server(router, 1).await;
    let (status, body) = send_get(addr, &format!("/users/{}", user.id)).await;

    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["email"], "bound@example.com");
}
```

对于隔离测试绑定的场景 - 没有路由器，没有 TCP 循环 - 请用 `Request::with_params(...)` 自己合成路由参数（见下面的[`Request` 上的构建器钩子](#request-上的构建器钩子)）。这正是 `framework/tests/data_route_params.rs` 用来针对合成参数测试 `#[derive(Data)]` 提取器的模式。

## 端到端地测试认证流程

一个真实的认证流程测试会注册一个用户，驱动这个登录路由，从响应上拽下这个会话 cookie，并在一个受保护的路由上重新发送它。四个步骤，全都在线路层面：

```rust
#[tokio::test]
async fn login_flow_issues_session_cookie() {
    // 1. 启动：创建这个用户。
    Auth::password()
        .register("alice@example.com", "longpassword123")
        .await.expect("register");

    // 2. 挂载这些路由。
    let router = Router::new()
        .post("/login", login_handler)
        .get("/dashboard", |_req: Request| async { text("dashboard") });
    let addr = spawn_server(router, 2).await;

    // 3. 驱动登录；捕获这个 Set-Cookie 请求头。
    let login = post_json(addr, "/login", serde_json::json!({
        "email": "alice@example.com",
        "password": "longpassword123",
    })).await;
    assert_eq!(login.status, 200);
    let cookie = extract_session_cookie(&login.headers);

    // 4. 对着这个受保护的路由重放这个 cookie。
    let (status, body) = get_with_cookie(addr, "/dashboard", &cookie).await;
    assert_eq!(status, 200);
    assert_eq!(&body[..], b"dashboard");
}
```

`extract_session_cookie` 和 `get_with_cookie` 是直来直去的请求头和 cookie 接线活 - `framework/tests/auth_http_middleware.rs` 有一份完整的实现。要点在于：整条流程都经过真实的 `SessionMiddleware`、真实的 `Auth` 认证守卫、真实的 `Authenticatable` 解析来运行。这个测试验证的是发给客户端的响应这份契约本身，而不是对它的一次模拟。

## 测试 panic 边界

一个处理程序内部的 panic，绝不能让服务器崩溃。这个 panic 恢复包装器（`execute_chain_safely`）会捕获它，并通过和返回的错误流经的同一条路径，把它转换成一个 500。您不需要任何特殊的测试基础设施就能验证这一点 - 把 `accepts` 设成 `>= 2`，这样这个监听器就能在这次 panic 中存活下来：

```rust
#[tokio::test]
async fn panicking_handler_yields_500_and_server_survives() {
    let router = Router::new()
        .get("/panic", |_req: Request| async {
            panic!("intentional test panic");
            #[allow(unreachable_code)] text("unreachable")
        })
        .get("/ok", |_req: Request| async { text("ok") });

    let addr = spawn_server(router, 4).await;

    // 第一步：这次 panic 会转换成一个经过清理的 500。
    let (s1, body) = send_get(addr, "/panic").await;
    assert_eq!(s1, 500);
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["message"], "Internal Server Error");
    assert!(parsed.get("request_id").is_some());

    // 第二步：这个监听器存活了下来。下一个请求是正常的。
    let (s2, body2) = send_get(addr, "/ok").await;
    assert_eq!(s2, 200);
    assert_eq!(&body2[..], b"ok");
}
```

## 不经过路由测试访问器

有时候您想测试一个 `Request` 访问器（`bearer_token`、`is_method`、`ip`、`is_json` 等等），却完全不想拉起一个路由器。这个技巧是一个微小的装置，运行一个 hyper 服务，它唯一的工作就是构造这个 `Request`，并通过一个 `tokio::sync::oneshot::channel` 把它发送回来：

```rust
let (req_tx, req_rx) = tokio::sync::oneshot::channel::<suprnova::Request>();
// … 回环 hyper 服务，它的 service_fn 会做：
//     let req = suprnova::Request::new(hyper_req);
//     let _  = req_tx.send(req);
//     返回一个带空请求体的 200
let req = req_rx.await.unwrap();
```

`framework/tests/http_request_accessors.rs` 有完整的 `build_request(builder, body) -> Request` 辅助函数。每个 crate 复制一次，每一个访问器测试就都能读得干干净净：

```rust
#[tokio::test]
async fn bearer_token_extracts_simple_token() {
    let req = build_request(
        hyper::Request::builder()
            .method("GET")
            .uri("/api/users")
            .header("Authorization", "Bearer secret-token-123"),
        "",
    ).await;
    assert_eq!(req.bearer_token().as_deref(), Some("secret-token-123"));
}
```

这个 Request 是真实的（由 hyper 从一次真实的线路交换里产生出来的），但没有任何路由或中间件运行过 - 当被测试的单元就是这个访问器本身时，这正是您想要的。

## `Request` 上的构建器钩子

当您手上有一个 `Request`，需要伪造路由层的某一部分时，三个构建器方法能帮上忙：

```rust
impl Request {
    pub fn with_params(mut self, params: HashMap<String, String>) -> Self;
    pub fn with_route_pattern(mut self, pattern: String) -> Self;
    pub fn with_peer_addr(mut self, addr: std::net::IpAddr) -> Self;
}
```

这些和服务器在分发一个匹配到的路由时调用的方法是一样的 - `Router` 会在 `matchit` 返回之后调用 `with_params`，调用 `with_route_pattern` 让 `req.route_pattern()` 能解析，并在知道了这个被接受的 TCP 套接字的 IP 之后调用 `with_peer_addr`。在测试里，您自己调用它们，来抄近路完成同样的设置。

```rust
let req = Request::new(hyper_req)
    .with_params(HashMap::from([("id".into(), "42".into())]))
    .with_route_pattern("/users/{id}".into())
    .with_peer_addr("192.168.1.10".parse().unwrap());

assert_eq!(req.param("id").unwrap(), "42");
assert_eq!(req.ip(), Some("192.168.1.10".parse().unwrap()));
```

## 需要知道的事情

一份会绊倒第一次写这些的作者的简短陷阱清单：

- **`Incoming` 仅限服务器一侧。** 您没法在测试里构建一个。
  TCP 回环（或者进程内的服务捕获）是唯一的路径 - 没有一个
  “从一个 `Vec<u8>` 请求体构建一个 `Request`”的构造函数。
- **不要在测试之间共享状态。** 每一个 `#[tokio::test]` 都会得到自己的运行时；跨测试的污染通常意味着您在共享一个全局量（`once_cell`、`lazy_static`、环境变量）。数据库状态请参见
  [测试](testing.md)里的 `TestDatabase`。
- **Cookie 需要一个真实的客户端。** 没有自动的 cookie jar - 请把一个响应上的 `Set-Cookie`，穿线传进下一个响应的 `Cookie` 里。这个模式请参见 `framework/tests/auth_http_middleware.rs`。
- **响应后终止的 spawn 是非阻塞的。** 如果您想对经由 `Terminable`
  运行的副作用做断言，请轮询它们 - 这个响应会在这个钩子运行之前就返回给客户端。

## 每一部分位于何处

| 组成部分 | 文件 |
|---|---|
| `handle_request`、`handle_request_with_peer` | `framework/src/server.rs` |
| `Request::new`、`with_params`、`with_route_pattern`、`with_peer_addr` | `framework/src/http/request.rs` |
| `MiddlewareRegistry::new`、`append`、`prepend` | `framework/src/middleware/registry.rs` |
| 回环测试装置（典范） | `framework/tests/cors_middleware.rs` |
| 进程内的 `Request` 捕获装置 | `framework/tests/http_request_accessors.rs` |
| Panic 边界测试模式 | `framework/tests/middleware_panic_safety.rs` |
| 认证 + 中间件端到端模式 | `framework/tests/email_verified_middleware.rs` |

## 下一步

- [测试](testing.md) - `#[suprnova_test]`、`TestDatabase`、
  `describe!`/`test!`/`expect!` 这些宏，以及单元级别的表面
- [错误模型](error-model.md) - 每一个错误响应用的那个 JSON 形态、
  5xx 的清理规则，以及 `request_id` 在一个测试请求体里意味着什么
- [中间件](middleware.md) - 编写您在这里测试的中间件，以及全局对路由的生命周期
- [路由](routing.md) - 您在生产环境和测试里都会挂载的那个
  `Router`、路由参数、路由名字、签名 URL
- [认证](authentication.md) - `Auth` 这个门面、`Authenticatable`、认证守卫，以及 `Auth::set_user` 如何和 `handle_request` 装上的那个请求作用域交互
