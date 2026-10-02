# CORS

`CorsMiddleware` 会回应预检 `OPTIONS` 请求，并为普通的跨域响应加上 `Access-Control-Allow-*` 响应头。当一个位于不同来源的浏览器要调用您的 API 时 - 公开 API、托管在另一个域名下的 SPA、移动端 webview，或者一个单独托管的文档站点 - 您在 `bootstrap()` 里安装它一次。同源应用（Inertia 与后端由同一个主机提供服务，也就是 Suprnova 的默认形态）根本不需要 CORS。这个中间件镜照 Laravel 的 `HandleCors` 和 `config/cors.php`，只不过它的形式是 `CorsConfig` 上的一个类型化构建器。

## 全局安装它

```rust,ignore
use std::time::Duration;
use suprnova::{global_middleware, CorsConfig, CorsMiddleware};

pub fn register() {
    global_middleware!(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"])
            .allow_credentials(true)
            .max_age(Duration::from_secs(600)),
    ));
}
```

一次预检就是一个带有 `Access-Control-Request-Method` 请求头的 `OPTIONS` 请求。路由器里没有 `OPTIONS` 路由，所以一次预检永远*匹配*不到任何路由 - 但 Suprnova 的服务器会在未匹配的请求上运行全局中间件链（最终以一个 404 收尾），所以一个全局安装的 `CorsMiddleware` 会看到这次预检，并在那个 404 被产生出来之前就用 `204` 把它短路掉。**这就是为什么 CORS 必须全局安装，而不是逐路由安装。**

## 选择一个来源策略

`CorsConfig` 故意没有 `Default`。一个不假思索就放行的宽松策略是一个安全陷阱，所以您必须自己做出选择：

| 构建器 | 行为 |
| --- | --- |
| `CorsConfig::allow_origins([...])` | 固定的允许列表。只有当来源与其中某一项完全一致时，它才会被回显回去。 |
| `CorsConfig::any_origin()` | 通配符 `*`。在启用凭据的情况下，中间件会回显具体请求的来源，而不是 `*`（按照 Fetch 规范，`*` 与凭据的组合是无效的）。 |
| `.allow_origin_patterns([...])` | 在字面列表之上追加的正则表达式模式。适用于动态子域名。 |

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .allow_origin_patterns([r"^https://[a-z0-9-]+\.staging\.example$"])
```

模式会被自动锚定 - 缺少 `^` 和 `$` 时会自动在前后补上，这样一个针对 `https://evil.com/?u=https://app.example` 这类重定向 URL 的部分匹配就无法漏过去。

无效的正则表达式会在配置时（也就是启动时）panic，而不是在请求时 - 目的是把配置 bug 醒目地摆到台面上，而不是静默地失败开放。

`allowed_origins_patterns`（Laravel 风格命名的别名）同样可用。

## 限定哪些路径应用 CORS

Laravel 的 `cors.php` 配置里有一个 `paths` 数组（`['api/*', 'sanctum/csrf-cookie']`），把 CORS 的应用范围限制在特定的 URL 模式上。Suprnova 镜照了这一点：

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .paths(["api/*", "sanctum/csrf-cookie"])
```

没有设置 `paths` 时，CORS 会在每一个请求上运行（这是 Suprnova 的默认行为 - 因为这个中间件本来就是靠注册来选择启用的）。一旦设置了至少一个模式，就只有匹配的请求会得到 CORS 处理（预检**和**真实响应的装饰都算）；其余的一切都会原封不动地流过去。

模式使用 Laravel 的 `Str::is` 语义：`*` 是一个跨多个路径段的通配符，会贪婪地跨越 `/`。开头的 `/` 会被规范化，所以 `"api/*"` 和 `"/api/*"` 是等价的。

```rust,ignore
"api/*"             // 匹配 /api/users、/api/users/42
"api/*/posts"       // 匹配 /api/v2/posts、/api/v1/posts
"sanctum/csrf-cookie" // 精确匹配的字面量
"*"                 // 匹配一切
```

## 通过谓词跳过

对于那些无法用路径模式表达的、基于请求形态的判断（根据某个请求头跳过、只在生产环境运行 CORS、健康检查期间跳过），请使用 `skip_when`：

```rust,ignore
CorsConfig::any_origin()
    .skip_when(|req| req.header("X-Internal-Call").is_some())
    .skip_when(|req| req.path() == "/healthz")
```

它镜照 Laravel 的 `HandleCors::skipWhen(Closure)`，但挂在策略上，而不是作为一份全局可变状态存在。可以注册多个 `skip_when` 回调；其中任意一个返回 `true` 就会跳过 CORS。

## 方法、请求头、暴露的响应头

```rust,ignore
CorsConfig::allow_origins(["https://app.example"])
    .methods(["GET", "POST", "DELETE"])           // 默认 = GET/POST/PUT/PATCH/DELETE/OPTIONS/HEAD
    .allow_headers(["Content-Type", "X-CSRF-TOKEN"])  // 收紧范围；默认 = 回显请求所要求的
    .allow_any_headers()                          // 显式表示“回显请求方所要求的任何请求头”
    .expose_headers(["X-Total-Count", "Link"])    // JS 可以在响应上读取的响应头
```

Laravel 风格命名的别名（这样 `cors.php` 的使用者能找到他们预期的东西）：

- `allowed_methods(...)` ≡ `methods(...)`
- `allowed_headers(...)` ≡ `allow_headers(...)`
- `exposed_headers(...)` ≡ `expose_headers(...)`
- `allowed_origins_patterns(...)` ≡ `allow_origin_patterns(...)`
- `supports_credentials(...)` ≡ `allow_credentials(...)`

## 凭据与 `*`

按照 Fetch 规范，`Access-Control-Allow-Origin: *` 与凭据放在一起是无效的 - 浏览器会拒绝这个响应。当有一份显式的来源列表（`allow_origins([...])`）再加上 `allow_credentials(true)` 时，中间件会回显具体请求的 `Origin`，而不是 `*`，于是这个策略就能如预期地工作。

**`any_origin() + allow_credentials(true)` 会在构建时 panic。** 这个组合完全绕开了来源允许列表：任何攻击者页面都可以发起带凭据的跨域请求并读取响应。与其在运行时发出错误的响应头，策略的构造函数选择明确地失败，这样这个配置错误就永远到不了一个正在运行的部署里。请改用一份显式的允许列表：

```rust,ignore
// 正确 - 显式的允许列表配合凭据。
CorsConfig::allow_origins(["https://app.example"]).allow_credentials(true)
// → 请求带有 Origin: https://app.example
// → 响应：Access-Control-Allow-Origin: https://app.example
//         Access-Control-Allow-Credentials: true

// 在构建时被拒绝 - panic，并附上一条补救提示。
// CorsConfig::any_origin().allow_credentials(true)
```

## Max-age

```rust,ignore
.max_age(Duration::from_secs(600))   // 类型化
.max_age_secs(600)                   // Laravel 风格的整数秒
```

`Access-Control-Max-Age` 告诉浏览器，它可以把预检结果缓存多久。值越大 = 预检往返越少，策略变更传播得越慢。

## 中间件实际发出什么

### 预检（`OPTIONS` + `Access-Control-Request-Method`）

如果来源是被允许的：

```
HTTP/1.1 204 No Content
Access-Control-Allow-Origin: <origin>
Access-Control-Allow-Credentials: true        // 启用凭据时
Access-Control-Allow-Methods: GET, POST, ...
Access-Control-Allow-Headers: <reflected or fixed>
Access-Control-Max-Age: 600                   // 设置了才有
Vary: Origin, Access-Control-Request-Method, Access-Control-Request-Headers
```

如果来源不被允许：只有一个光秃秃的 `204` 加 `Vary`（没有任何 `Access-Control-*`）。是浏览器那边的“请求头缺失”检查产生了 CORS 错误 - 这与 `tower-http` 的惯例一致。

### 真实的跨域响应

当请求带有 `Origin` 请求头，并且该来源被允许时：

```
Access-Control-Allow-Origin: <origin or *>
Access-Control-Allow-Credentials: true        // 启用时
Access-Control-Expose-Headers: X-Total, Link  // 配置了才有
Vary: Origin                                  // 只在不是 "*" 时
```

`*` 形式的 ACAO 对每一个来源都是一样的，所以不需要 `Vary`；而一个具体的来源会随来源不同而变化，所以共享缓存必须把它算进缓存键里。

## 测试 CORS 处理程序

CORS 是在浏览器一侧强制执行的 - 即便来源不被允许，服务器仍然会运行处理程序，它只是不去装饰响应而已。这就是可以被测试的行为：

```rust,ignore
let (status, headers, body) = request_with_origin(
    "/api/data",
    "https://app.example",
).await;
assert_eq!(status, 200);
assert_eq!(
    headers.get("access-control-allow-origin"),
    Some(&"https://app.example".to_string()),
);
```

对于一个不被允许的来源，处理程序照样运行，响应体也照样返回，但正是 `Access-Control-Allow-Origin` 的缺失，挡住了浏览器去读取它。

## Laravel 对等映射

| Laravel 的 `cors.php` | Suprnova 构建器 |
| --- | --- |
| `paths` | `.paths([...])` |
| `allowed_methods` | `.methods([...])` / `.allowed_methods([...])` |
| `allowed_origins` | `CorsConfig::allow_origins([...])` |
| `allowed_origins_patterns` | `.allow_origin_patterns([...])` / `.allowed_origins_patterns([...])` |
| `allowed_headers` | `.allow_headers([...])` / `.allowed_headers([...])` |
| `exposed_headers` | `.expose_headers([...])` / `.exposed_headers([...])` |
| `max_age` | `.max_age(Duration)` / `.max_age_secs(u64)` |
| `supports_credentials` | `.allow_credentials(bool)` / `.supports_credentials(bool)` |
| `HandleCors::skipWhen(closure)` | `.skip_when(\|req\| ...)` |

这个中间件是全局注册的，而不是 Laravel 那种“对 `paths` 自动安装”的做法 - Suprnova 的中间件链是显式的，设计思路参见 [中间件](middleware.md)。

### 为什么 Suprnova 有所不同

Laravel 的 `HandleCors` 会自动挂到内核上，并从 `config/cors.php` 读取它的策略。这个形态对 PHP 是行得通的，因为对一个每进程一个请求的框架来说，配置数组是唯一一个既能共享配置、又不必在每个请求上重新求值的地方。Suprnova 把同样的这些选项暴露为一个类型化的 `CorsConfig` 构建器，由您用 `global_middleware!` 显式注册，这既让中间件链在 `bootstrap()` 里保持可见，也让编译器能够强制您在允许列表和通配符之间做出选择（`CorsConfig` 没有 `Default`，所以您不会因为忘了填某个配置值，就意外地把 `Access-Control-Allow-Origin: *` 发布上线）。

另一处分歧是：即便在没有路由的路径上，预检也能到达中间件。Laravel 会把 `OPTIONS` 交给它的路由器，所以预检会匹配到一个 `OPTIONS` 路由（这样的路由会为每一条 REST 路由自动注册）。Suprnova 的路由器没有 `OPTIONS` 路由；取而代之的是，服务器会在返回 404 之前，先在未匹配的请求上运行全局中间件链，于是一个全局安装的 `CorsMiddleware` 会在走上“未找到”那条路径之前，就用 `204` 把预检短路掉。这就是为什么 CORS *必须*全局安装 - 一个逐路由的注册永远看不到预检。

## 下一步

- [中间件](middleware.md) - 这个 trait、这条链、全局注册与逐路由注册、可终止钩子
- [CSRF](csrf.md) - 大多数应用会和 CORS 一起安装的另一个全局中间件
- [路由](routing.md) - 路由是如何匹配的（以及为什么预检匹配不上），还有全局链在没有兜底路由时所走的那条路径
- [请求生命周期](lifecycle.md) - 相对于会话、CSRF 和处理程序，CORS 在这条链里位于什么位置
- [配置](configuration.md) - 面向那些需要环境驱动设置的中间件的类型化配置模式
