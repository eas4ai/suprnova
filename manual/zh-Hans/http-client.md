# HTTP 客户端

`Http` 这个门面是 HTTP 的出站一侧 - Laravel 的 `Http::` 助手在 Rust 里的对应物。当您的处理程序、作业，或者计划任务需要调用别人的 API 时 - 一个支付网关、一个地理编码器、一个 webhook 目标、一条 Slack 消息 - 就该伸手去拿它。Fluent 构建器、JSON 进出、带抖动的重试、会记录您发送了什么的确定性测试伪造实现。和您在 Laravel 里用过的是同一套表面，外加任务本地的隔离，这样并行测试就不会看到彼此的伪造实现。

```rust
use suprnova::Http;
use serde_json::json;

let resp = Http::post("https://api.stripe.com/v1/charges")
    .bearer_token(secret_key)
    .json(&json!({ "amount": 1000, "currency": "usd" }))
    .send()
    .await?;

let body: serde_json::Value = resp.json().await?;
```

这就是那个形态：`Http::<verb>(url)` 返回一个 `RequestBuilder`；您把配置链接在它上面；`.send().await` 返回一个 `ClientResponse`。背后支撑的客户端是一个共享的 `reqwest::Client`，带着 rustls TLS、一个 30 秒的默认超时，以及一个 `suprnova/<version>` 用户代理 - 在首次调用时惰性构建。

## 各个动词

```rust
Http::get("https://api.example.com/users/42")
Http::post("https://api.example.com/users")
Http::put("https://api.example.com/users/42")
Http::patch("https://api.example.com/users/42")
Http::delete("https://api.example.com/users/42")
```

每一个动词都返回一个 `RequestBuilder`。这个 URL 可以是任何 `impl Into<String>` - 一个 `&str`、一个 `String`，或者一个 `Cow<str>`。这个门面里不自带 URL 构建的辅助函数；请自己格式化这个 URL，或者伸手去拿一个 query-string crate。

## 请求体

三种附上一个请求体的方式。每一种都会替换掉任何之前设置过的请求体。

### JSON

```rust
use serde::Serialize;

#[derive(Serialize)]
struct CreateUser {
    name: String,
    email: String,
}

Http::post("https://api.example.com/users")
    .json(&CreateUser {
        name: "Ada".into(),
        email: "ada@example.com".into(),
    })
    .send()
    .await?;
```

`.json(&value)` 接受任何实现了 `serde::Serialize` 的东西。这个请求实际发出的 `Content-Type` 会自动被设成 `application/json`。如果序列化失败（例如一个键不是字符串的 map），这个构建器会记录这个错误，`send()` 会把它暴露出来，而不是静默地发送一个 `null` 请求体。

### Form

```rust
Http::post("https://login.example.com/oauth/token")
    .form(&serde_json::json!({
        "grant_type": "client_credentials",
        "client_id": id,
        "client_secret": secret,
    }))
    .send()
    .await?;
```

`.form(&value)` 会把这个值序列化成 `application/x-www-form-urlencoded`。这个值必须序列化成一个 JSON 对象；这些键会变成表单字段。和 `.json` 一样的请求体错误语义 - 一次序列化失败会通过 `send().await?` 暴露出来，绝不会变成一个静默的空请求体。

### 裸字节

```rust
use bytes::Bytes;

let payload: Bytes = compress(report)?;
Http::post("https://collector.example.com/ingest")
    .header("Content-Type", "application/octet-stream")
    .body(payload)
    .send()
    .await?;
```

`.body(bytes)` 接受任何 `impl Into<Bytes>`。`Content-Type` 请求头由您自己负责 - `.body` 不会设置它。

## 请求头和认证

```rust
Http::get("https://api.example.com/private")
    .header("X-Request-Id", request_id)
    .header("Accept", "application/vnd.api+json")
    .bearer_token(api_key)
    .send()
    .await?;
```

`.header(name, value)` 是追加的；框架不会去重，所以两次用同一个名字调用，会发送两个请求头，reqwest 会按 HTTP 语义把它们接合起来。两个面向常见认证方案的快捷方式：

- `.bearer_token(token)` - 设置 `Authorization: Bearer <token>`
- `.basic_auth(user, password)` - 设置 `Authorization: Basic <b64>`；
  `password` 是 `Option<&str>`，所以 `.basic_auth("api-key", None)`
  会编码出一些提供者想要的那种 `api-key:` 形式

## 超时

这个共享客户端有一个 30 秒的默认超时。需要时可以逐请求覆盖它：

```rust
use std::time::Duration;

Http::get("https://slow.example.com/report")
    .timeout(Duration::from_secs(120))
    .send()
    .await?;
```

`.timeout(dur)` 会为这一次调用同时覆盖连接超时和请求总超时。这个构建器上没有一个单独的 `connect_timeout` 旋钮；底层的 reqwest 客户端用的是一个合并起来的超时。

## 重定向

这个共享客户端默认会跟随重定向（最多到 reqwest 的上限
10) - 当您调用的是一个受信任的端点，它用 `http → https` 来应答，或者把您交给一个 CDN URL 时，这就是正确的行为。

当这个请求 URL 受到不受信任的输入影响时，这个默认行为就会变成一个服务端请求伪造（SSRF）的攻击向量：一个恶意端点可以用一个 `Location` 指向内部服务或者云元数据地址（`http://169.254.169.254/…`）的 `3xx` 来应答，一个会跟随的客户端就会追过去。请用 `.no_redirects()` 为这些请求禁用重定向跟随：

```rust
let resp = Http::get(user_supplied_url)
    .no_redirects()
    .send()
    .await?;

// 这个 3xx 会被原样返回，而不是被跟随 - 请检视它并
// 拒绝它，而不要让客户端去追这个 Location 请求头。
if (300..400).contains(&resp.status()) {
    return Err(AppError::bad_request("refusing to follow a redirect"));
}
```

`.no_redirects()` 会把这个请求路由经过一个单独的、不跟随重定向的客户端；默认客户端 - 以及每一个不调用它的请求 - 都不受影响。这是 web-push 发送方已经对攻击者可控的推送端点应用过的那种重定向封锁，在通用客户端上的对应物。

## 重试

`Http` 自带完全抖动的指数退避重试 - AWS 的那份配方，和 Laravel 用的是同一份。两种重试模式都会对每一种 HTTP 方法处理传输失败；区别在于收到 5xx 响应时，是否允许重放 `POST` 和 `PATCH`。

### `.retry(max_attempts, base_backoff)` - 每一种方法的传输重试

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .send()
    .await?;
```

`max_attempts` 包括第一次尝试，所以 `retry(4, ...)` 在最初那次尝试之后，最多再重试三次。第 `n+1` 次尝试之前的延迟，是 `[0, base_backoff * 2^(n-1)]` 里的一个均匀随机时长，上限 30 秒。是完全抖动，不是指数退避加固定睡眠，这样许多因为同一次故障而重试的工作进程，就不会同步成一次惊群效应。

`.retry()` 会对每一种方法重试传输失败。如果收到响应，除非方法是 `POST` 或 `PATCH`，否则它会重试 5xx 状态。4xx 以及 2xx/3xx 响应会被原样返回。耗尽重试之后，最后一个响应（或者最后一个传输错误）会被返回给调用方。

这个区别对写入很重要。`POST` 或 `PATCH` 的传输失败可能意味着服务器已经提交写入、但响应丢失了，不过当前契约仍会重试这类失败。这些方法收到 5xx 响应时，会在一次尝试之后返回，除非调用方使用 `.retry_non_idempotent(...)`。

### `.retry_non_idempotent(...)` - POST/PATCH 的选择性加入

```rust
Http::post("https://api.example.com/charges")
    .header("Idempotency-Key", idem_key)
    .retry_non_idempotent(3, Duration::from_millis(200))
    .send()
    .await?;
```

当您提供了上游会遵守的幂等键，或者已经用别的办法让请求可以安全地重放时，请切换到 `.retry_non_idempotent(...)`。它保留对每一种方法的传输错误重试，并额外允许对 `POST` 和 `PATCH` 重试 5xx 响应。4xx 以及 2xx/3xx 响应仍会直接通过。

### 503 上会遵守 Retry-After

对于一个 `503 Service Unavailable`，框架会遵守一个 `Retry-After` 请求头 - 无论是增量秒数（`Retry-After: 30`）还是 HTTP 日期（`Retry-After: Tue, 15 Nov 1994 08:12:31 GMT`）形式。实际的等待时间，是那个带抖动的退避和这个 `Retry-After` 提示两者里较大的一个，依然上限 30 秒。一个恶意或者配置错误的服务器，返回一个 `Retry-After: 86400`，也不会把您的任务停摆一整天。

### `.retry_when(predicate)` - 进一步收紧策略

```rust
use std::time::Duration;

let resp = Http::get("https://flaky.example.com/health")
    .retry(4, Duration::from_millis(200))
    .retry_when(|ctx| ctx.method == "GET")
    .send()
    .await?;
```

`retry_when` 注册一个谓词，在上面的策略本来会进行的每次重试之前咨询。它可以否决一个本来符合条件的重试，但不能凭空制造一次重试。尤其是，它不能把 2xx、3xx 或 4xx 响应变成重试，也不能在没有 `.retry_non_idempotent(...)` 时让 `POST` 或 `PATCH` 收到的 5xx 响应变得可重试。对于每一种方法的传输错误重试（包括使用普通 `.retry()` 配置的 `POST` 和 `PATCH`），它都会在重试前被咨询。没有 `.retry(...)` 或 `.retry_non_idempotent(...)` 策略时，单独的 `retry_when` 没有可否决的重试。

该谓词接收 `RetryContext { attempt, method, url, outcome }`，其中 `outcome` 是 `RetryOutcome::TransportError`（响应到达之前发送失败）或 `RetryOutcome::Status(n)`（一个 5xx 响应）。

## 读取响应

`ClientResponse` 暴露状态、请求头，以及三个读取请求体的方法。每一个请求体方法都会消耗这个响应。

```rust
let resp = Http::get("https://api.example.com/users/42").send().await?;

let status: u16 = resp.status();
let etag: Option<String> = resp.header("ETag");

// 挑一个 - 每一个都会消耗这个响应。
let user: User = resp.json().await?;
// let text: String = resp.text().await?;
// let bytes: Bytes = resp.bytes().await?;
```

`.header(name)` 不区分大小写。`.json::<T>()` 返回 `Result<T, FrameworkError>`，并使用 `serde_json` 来解码。`.text()` 会强制要求 UTF-8，如果这个请求体不是有效的 UTF-8，就会暴露一个 `FrameworkError`。

### 响应体上限

否则，一个缓慢或者恶意的上游，就能把一个无界的请求体流进内存里。为了防范这一点，每一次缓冲的请求体读取都有一个上限 - 默认 25 MiB。在启动时全局覆盖它：

```rust
use suprnova::Http;

// 只做一次，在 bootstrap 里的某个地方。
Http::set_max_response_bytes(100 * 1024 * 1024); // 100 MiB
```

或者当某一次调用确实合理地要处理一个更大的载荷时，逐请求覆盖它：

```rust
let bytes = Http::get("https://example.com/big-export.json")
    .max_response_bytes(500 * 1024 * 1024) // 500 MiB
    .send()
    .await?
    .bytes()
    .await?;
```

一个声明的 `Content-Length` 超过这个上限的响应，会在读取任何请求体之前就被拒绝；这个流式循环也会针对实际的字节数强制这个上限，以防 `Content-Length` 缺失或者说了假话。

## 脱围机制 - 原始的 reqwest

框架覆盖了常见的情况。当您需要一些我们没有暴露的东西时 - 流式请求体、multipart 上传、重定向策略检视、websocket 升级 - 调用 `.into_inner()` 来解开底层的 `reqwest::Response`：

```rust
let resp = Http::get("https://example.com/big-stream").send().await?;
let raw: reqwest::Response = resp.into_inner()?;
let mut stream = raw.bytes_stream();
while let Some(chunk) = stream.next().await {
    process(chunk?);
}
```

`into_inner()` 在一个伪造响应上被调用时，会返回 `Err(FrameworkError::internal(...))` - 在那种情况下没有底层的 `reqwest::Response`。一旦您拿到了这个原始响应，响应体上限也就不再适用了；从那时起，读取由您自己掌握。

目前对于出站的 multipart 上传，请通过同样的脱围路线，直接落到 `reqwest::Client`。等需求模式自己成形之后，未来的版本可能会加一个 `.multipart(...)` 构建器。

## 用 `Http::fake` 测试

这是您每天都会用到的部分。`Http::fake` 会在一个 `tokio::task_local!` 作用域里运行您的测试主体，在那里每一个出站调用都会被拦截、捕获，并用您排进队列的东西来回答。

```rust
use suprnova::{Http, fake_response, assert_sent};

#[tokio::test]
async fn creates_a_user_via_api() {
    Http::fake(|| async {
        fake_response(
            "POST",
            "/api/users",
            201,
            serde_json::json!({ "id": 42, "name": "Ada" }),
        );

        let resp = Http::post("https://example.com/api/users")
            .json(&serde_json::json!({ "name": "Ada" }))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 201);
        let body: serde_json::Value = resp.json().await.unwrap();
        assert_eq!(body["id"], 42);

        assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
    })
    .await;
}
```

### 匹配预设响应

`fake_response(method, url_substring, status, body)` 会排一个预设响应进队列。第一个方法匹配（不区分大小写）、URL 又包含 `url_substring` 的出站请求，会消耗掉这个预设条目，并返回那个响应。用方法 `"*"` 来匹配任何方法。

后续匹配的请求，会落到同一形态的下一个预设条目上，或者 - 如果没有一个匹配 - 返回一个空的 `200 {}`。为每一次预期的调用排一个预设响应进队列：

```rust
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_1" }));
fake_response("GET", "/v1/customer", 200, json!({ "id": "cus_2" }));
// 两次对 /v1/customer 的 GET 会得到不同的响应；第三次会得到 200 {}。
```

### 断言

```rust
// 如果至少一个被记录的请求匹配，就通过。
assert_sent(|r| r.method == "POST" && r.url.contains("/charges"));

// 如果没有任何被记录的请求匹配，就通过。
assert_not_sent(|r| r.url.contains("/refunds"));
```

`RecordedRequest` 暴露 `method: String`、`url: String`、`headers: Vec<(String, String)>`，以及 `body: Option<Vec<u8>>`。这个谓词会针对每一个被记录的请求运行；断言失败时，会打印这个记录列表，请求头的值和请求体都会被涂黑（一个由 `Content-Type`、`Accept` 和 `User-Agent` 组成的小允许列表会完整显示；其余的一切都是 `<redacted>`）。这让 bearer 令牌和 webhook 负载，即便在一次断言炸掉时，也不会出现在 CI 日志里。

### 测试能安全地并行运行

这个伪造实现的状态活在一个 `tokio::task_local!` 里 - 每一个伪造作用域，都是限定在运行这个测试的任务上的，不是限定在进程上的。两个在不同任务上并发运行的测试，各自会得到自己的已记录请求 vec，和自己的预设响应队列。没有共享的 mutex，没有测试顺序，没有 `#[serial]`。

```rust
#[tokio::test]
async fn first_test() {
    Http::fake(|| async {
        fake_response("GET", "/a", 200, json!({"who": "first"}));
        let _ = Http::get("https://x.test/a").send().await.unwrap();
        assert_sent(|r| r.url.contains("/a"));
        // 兄弟测试对 /b 的请求在这里是不可见的。
    })
    .await;
}

#[tokio::test]
async fn second_test() {
    Http::fake(|| async {
        fake_response("GET", "/b", 200, json!({"who": "second"}));
        let _ = Http::get("https://x.test/b").send().await.unwrap();
        assert_sent(|r| r.url.contains("/b"));
    })
    .await;
}
```

## 生成任务的陷阱

`tokio::task_local!` 是限定在当前任务上的。经过 `tokio::spawn` 的工作，会落在一个全新的任务上，并**不会**继承这个伪造实现 - 默认情况下，来自这个被 spawn 出的 future 的出站调用，会触达真实的网络。两个辅助工具能解决这一点。

### `Http::fail_on_real_calls()` 和 `FailOnRealCallsGuard`

翻转一个进程全局的标志，把任何不匹配的出站调用都变成一个 `FrameworkError::internal(...)`，而不是让它触达网络。这是 Suprnova 对 Laravel `Http::preventStrayRequests()` 的对应物 - 它抓住的正是这个陷阱制造出来的那个 bug。

请使用这个 RAII 守卫，这样当测试结束时，这个标志会被重置，即便发生了 panic 也一样：

```rust
use suprnova::FailOnRealCallsGuard;

#[tokio::test]
async fn no_test_makes_a_real_call() {
    let _guard = FailOnRealCallsGuard::install();

    // 这个测试内部任何地方发出的、没被伪造的出站 HTTP 调用
    // - 包括来自一个被 `tokio::spawn` 出的任务 - 都会带着一条
    // 点名这个 URL 的消息报错。没有任何真正的网络 IO 会发生。
}
```

嵌套的 守卫 能正确地组合：内层 守卫 的 `Drop` 会恢复之前的状态，不会无条件地恢复成“允许”。所以一个在外层被守卫的作用域内部，装上自己 守卫 的内层测试辅助函数，在退出的路上不会解除外层 守卫 的武装。

这个标志按设计就是进程全局的。它的要点在于抓住一个被 `tokio::spawn` 出的 future 悄悄逃出一个伪造作用域，从 CI 里去 ping 一个真正的第三方。一个逐任务的标志会错过这一点。

### `Http::spawn_with_fake_inheritance(future)`

当被测试的代码合理地 spawn 出一个任务时 - 一个队列工作进程、一个后台同步器、一个子任务 - 如果您想让它的出站调用经过父任务的伪造实现，请把 `tokio::spawn` 换成 `Http::spawn_with_fake_inheritance`：

```rust
Http::fake(|| async {
    fake_response("GET", "/child", 204, json!({}));

    let handle = Http::spawn_with_fake_inheritance(async {
        // 跑在一个全新的任务上，但父任务的伪造状态
        // 会被重新装进这个任务的任务本地作用域里。这次发送
        // 会被拦截；响应就是上面那个 204。
        Http::get("https://child.example.com/child").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);

    // 来自这个子任务的已记录请求会在这里出现 - 这个
    // `Arc<Mutex<FakeState>>` 是共享的，不是拍了快照的。
    assert_sent(|r| r.url.contains("/child"));
})
.await;
```

如果在您调用 `spawn_with_fake_inheritance` 时没有任何伪造作用域是活跃的，它就等价于 `tokio::spawn` - 这个子任务会在没有任何伪造上下文的情况下运行。所以您可以在那些有时用 `Http::fake` 测试、有时不用的代码里，无条件地使用它。

### 测试设置里的双重保险

这两者可以组合起来。一个想要明确地安全的测试，会把它们搭配在一起：

```rust
#[tokio::test]
async fn pays_the_invoice() {
    let _guard = FailOnRealCallsGuard::install();

    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 200, json!({ "id": "ch_1" }));

        // 如果 URL 或方法上的一个打字错误，让它偏离了这个伪造实现，
        // 这个请求就会落到这个 guard 上，它会报错，
        // 带着一条点名这个 URL 的消息 - 而不是静默地
        // 返回一个把这次不匹配藏起来的空 200。
        pay_invoice(&invoice).await.unwrap();

        assert_sent(|r| r.url.contains("/v1/charges"));
    })
    .await;
}
```

没有这个 守卫 时，一个偏离了这个伪造实现的 URL 或方法，会静默地落到一个默认的 `200 {}` 上，即便生产代码调用的是一个不同的端点，您的测试也会通过。有了这个 守卫，您会在第一次不匹配时就明确地失败。

## OpenTelemetry 追踪传播

当框架带着 `otel` 这个 feature 构建，并且装上了一个 W3C TraceContext 传播器时，每一个出站的 `Http::*` 请求，都会把 `traceparent`（以及非空时的 `tracestate`）注入到它的请求头里 - 这样下游服务就能延续这个追踪。调用点不需要任何配置；这个传播器会在发送时读取 `opentelemetry::Context::current()`。

没有一个活跃的 OTel 上下文时，不会注入任何请求头，出站请求看起来和之前完全一样。传播器的设置请参见[可观测性](observability.md)。

## 为什么 Suprnova 有所不同

三处和 Laravel 的 `Http::` 门面之间的小分歧值得点出来。

**任务本地的伪造实现，而不是一个进程全局的模拟存储。** Laravel 的 `Http::fake()` 会改动一个进程范围的注册表；测试要在它上面串行化，或者您接受并行的运行器可能会竞态。Suprnova 的 `Http::fake` 用的是 `tokio::task_local!`，所以两个跑在两个任务上的测试，各自看到自己的伪造实现 - 没有测试顺序，没有共享的 mutex。代价是被 `tokio::spawn` 出的工作默认不会继承这个伪造实现，这就是为什么会有 `Http::spawn_with_fake_inheritance` 和 `FailOnRealCallsGuard`。合在一起，它们给您的是和 Laravel 的 `Http::preventStrayRequests()` 一样的“不会意外触达生产环境”保证，但作用域更严格。

**收到的 5xx 默认不重试 POST/PATCH。** Laravel 的 HTTP 客户端默认会重试任何方法。Suprnova 的 `.retry(...)` 仍会对 `POST` 和 `PATCH` 重试传输失败，但不会对这些方法收到的 5xx 响应重试。只有在让写入可以安全重放（通常使用上游会遵守的幂等键）之后，才使用 `.retry_non_idempotent(...)` 选择加入 5xx 响应重试。

**`retry_when` 只能收窄，绝不能扩大。** Laravel 的 `retry()` `$when` 回调会完全替换“是否应重试”的决定，因此它可以重试框架本来不会触及的状态（比如 404）。Suprnova 的 `retry_when` 只会否决 `.retry(...)` / `.retry_non_idempotent(...)` 已决定进行的一次重试；它会对每一种方法（包括 `POST` 和 `PATCH`）的传输错误重试进行咨询，但不能把 2xx、3xx 或 4xx 响应变为重试，也不能让 `POST` 或 `PATCH` 收到的 5xx 响应在普通 `.retry()` 下变得可重试。

## 边界情况与细则

- **`Http::*` 在 v1 里是封闭的。** 我们刻意不暴露底层的 `reqwest::Client`。要扩大这个表面，请往这个门面上加一个方法，而不是直接伸手去拿 `reqwest` - 除了通过一个真实响应上那个有文档说明的 `into_inner()` 脱围机制。
- **这个共享客户端只构建一次，永远存活。** 在首次调用任何 `Http::*` 动词时惰性构建，保存在一个 `OnceLock` 里。rustls 这套 TLS 栈和 30 秒的默认超时，都是烤进去的。
- **JSON/form 的序列化失败会明确地失败。** 一个 `.json(&unserializable)` 构建器会记录这个错误，`send()` 会把它作为 `FrameworkError::internal(...)` 返回。这个请求永远不会发出去 - 我们不会退化成一个 `null` 请求体。
- **30 秒的重试上限是硬性的。** 退避的数学计算上限是 30 秒；`Retry-After` 的解读上限是 30 秒；没有任何一次重试睡眠，会让一个任务停摆更久。
- **进程全局的上限是一次性的。** `Http::set_max_response_bytes` 是对一个进程全局原子量的一次写入 - 在启动时设置一次，之后按需逐请求覆盖它。没有一个“重置为默认值”的调用。

## 下一步

- [邮件](mail.md) - 出站邮件，测试用的是类似的伪造实现/驱动程序模式
- [通知](notifications.md) - 包括 web push 的通知通道，都共享同一套测试伪造实现的哲学
- [队列](queues.md) - 发出出站 HTTP 调用的作业，加上测试工作进程用的
  `spawn_with_fake_inheritance` 模式
- [测试](testing.md) - `#[suprnova_test]`、`TestContainer`，以及其余的伪造实现表面
- [可观测性](observability.md) - 让 `traceparent` 注入亮起来的 OTel 传播器设置
