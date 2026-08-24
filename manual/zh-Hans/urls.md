# URL 生成

URL 是您的应用引用自身的方式 - 每一次重定向、每一条邮件里的链接、每一个 Inertia `<Link>` 的 href、每一个签名下载，都得有个出处。把路径硬编码进去会让重构变得痛苦，也会让路由改名变得不安全。Suprnova 提供了一个小小的 `url::` 命名空间，以及一个兄弟辅助函数 `route()`，它们接受一个名称加上一组参数，回给您一个字符串，其中百分号编码已经处理妥当，签名铸造随手可用，而验证与 Laravel 的传输格式逐字节一致。

本章是 URL 生成接口的参考。[路由](routing.md)那一章讲的是如何声明路由并给它们命名；本章讲的是之后您拿这些名称能做什么。

```rust
use suprnova::{route, url};

// 按名称查找 → URL
let profile = route("users.show", &[("id", "42")]).unwrap();
//   "/users/42"

// 相对 APP_URL 的绝对 URL
let absolute = url::to("/dashboard");
//   "https://app.test/dashboard"

// 用于密码重置的签名链接
let link = url::signed_route("password.reset", &[("token", reset_token)])?;
//   "/password/reset/xyz?signature=ab12..."

// 在入站请求上验证
if url::has_valid_signature(&request)? {
    // 据此行动
}
```

本章里的一切都在 `suprnova::url::*` 和 `suprnova::route` 之下被重导出，所以使用方代码永远不需要直接伸手进路由模块里去。

## 命名路由

名称是在注册时附加到一条路由上的字符串标签。一旦名称存在，`route(name, params)` 就会把它解析回一个 URL 模式，并把参数代入进去。名称存放在唯一一个进程级全局注册表里 - 每个运行中的二进制文件只有一张 `name → path` 表，而不是每个 `Router` 一张。

```rust
use suprnova::{routes, get, post};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users/{id}", controllers::users::show).name("users.show"),
    post!("/users", controllers::users::store).name("users.store"),
}
```

`.name(...)` 这次调用注册了 `"users.show" → "/users/{id}"`。从那一刻起，进程里的任何地方都可以解析这个名称：

```rust
use suprnova::route;

let url = route("users.show", &[("id", "42")]);
// Some("/users/42")

let missing = route("does.not.exist", &[]);
// None
```

重新注册同一个 `(name, path)` 对是幂等的 - 当路由注册在启动期间跑了不止一次时，这很有用。把一个名称注册到*不同的*路径下则会 panic；这种冲突是一个安全形状的 bug，因为像 `Redirect::route` 这样的辅助函数会静默地指向竞争中获胜的那一边。

### 查找辅助函数

| 函数 | 返回值 | 路由不存在时 |
|---|---|---|
| `route(name, params)` | `Option<String>` | `None` |
| `route_with_params(name, params_map)` | `Option<String>` | `None` |
| `try_route(name, params)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |
| `try_route_with_params(name, params_map)` | `Result<String, RouteUrlError>` | `Err(NameNotFound)` |

宽松的 `route` / `route_with_params` 这一对，会把任何未填充的 `{placeholder}` 段原样留在输出里 - 用在调试日志里没问题，发给浏览器则不安全。严格的 `try_route` / `try_route_with_params` 这一对，会返回 `RouteUrlError::MissingParams { name, missing }`，把那些未填充的占位符列出来，好让调用方明确地失败，而不是把用户重定向到 `/users/{id}`。

```rust
use suprnova::routing::{try_route, RouteUrlError};

match try_route("users.show", &[]) {
    Ok(url) => /* 可以安全地重定向 */,
    Err(RouteUrlError::MissingParams { name, missing }) => {
        // missing == vec!["id"]
        return Err(FrameworkError::internal(
            format!("cannot build URL for {name}: missing {missing:?}"),
        ));
    }
    Err(RouteUrlError::NameNotFound(name)) => {
        return Err(FrameworkError::internal(format!("unknown route: {name}")));
    }
}
```

`Redirect::route` 底下正是出于这个原因才使用 `try_route_with_params` - 一次在 `Location` 响应头里带着裸 `{id}` 的重定向，会比干脆失败更糟糕。

### 百分号编码是自动的

参数值在被代入之前，会按 RFC 3986 的路径段规则做编码。这覆盖了 gen-delims 和 sub-delims（`/ ? # [ ] @ ! $ & ' ( ) * + , ; =`）、控制字符、空格，以及 `%` 本身。未保留字符（`A-Z a-z 0-9 - _ . ~`）则原样通过。

```rust
use suprnova::route;

// 含有斜杠的 slug 会被约束在一个段之内：
route("posts.show", &[("slug", "hello/world")]);
// Some("/posts/hello%2Fworld")

// 路径穿越的尝试无法逃出这个段：
route("users.show", &[("id", "../../etc/passwd")]);
// Some("/users/..%2F..%2Fetc%2Fpasswd")

// 真正的 Unicode 会原封不动地通过：
route("users.show", &[("id", "user-é-42")]);
// Some("/users/user-%C3%A9-42")
```

匹配那一侧保持了这次往返 - 一个发往 `/posts/hello%2Fworld` 的请求会匹配 `/posts/{slug}` 这条路由，而读取 `req.param("slug")` 的处理程序看到的是已经解码好的 `"hello/world"`。在边界上编码，在边界上解码；处理程序代码里永远看不到原始字节。

### 反向查找

当您手上已有一个匹配到的路由模式、又想拿到那个已注册的名称时 - 比如为了记日志，或者为了做 `Request::route_is("users.show")` 这类检查 - 请使用 `route_name_for_pattern`：

```rust
use suprnova::routing::route_name_for_pattern;

let name = route_name_for_pattern("/users/{id}");
// Some("users.show")
```

这是在名称注册表上的一次 O(n) 扫描。n 是已注册名称的数量；即便路由数达到四位数，这点开销相对于周围的请求生命周期也可以忽略不计。这个函数是为工具和中间件暴露出来的 - 当您在处理程序里拿一条命名路由做比较时，`Request::route_is` 已经替您调用过它了。

## 绝对 URL

对于其余的一切 - 构建邮件、分享 URL、发送 Open Graph 元数据 - 您需要的是一个带着正确协议和主机的绝对 URL。`url::to` 会把一个路径拼接到 `APP_URL` 上：

```rust
use suprnova::url;

// 环境变量里：APP_URL=https://app.example.com
let url = url::to("/about");
// "https://app.example.com/about"

// 已经是绝对形式的 URL 会原样通过：
let cdn = url::to("https://cdn.example/asset.js");
// "https://cdn.example/asset.js"

let proto_relative = url::to("//cdn.example/asset.js");
// "//cdn.example/asset.js"
```

主机、协议和端口全都来自 `APP_URL`。如果 `APP_URL` 是 `http://localhost:8765`，那么 `url::to("/foo")` 产出的就是 `"http://localhost:8765/foo"`。`APP_URL` 末尾的斜杠会被规范化掉，所以您永远不会得到 `https://host//path`。

### 强制 HTTPS

`url::secure(path)` 构建的是同一个绝对 URL，但会把协议升级为 `https://`，即使 `APP_URL` 是 `http://` 也一样：

```rust
use suprnova::url;

// 环境变量里：APP_URL=http://app.example.com
url::secure("/login");
// "https://app.example.com/login"
```

在生产环境里，您通常把 `APP_URL` 一次性设成您的 HTTPS 主机，然后再也不直接调用 `secure` - 这个升级是给这样一类环境准备的：本地开发跑在 HTTP 上，但某个特定的链接必须是 HTTPS（比如嵌在支付会话里的一个回调 URL）。

### 读取当前 URL

在处理程序内部，请求本身就是事实来源：

```rust
use suprnova::url;

async fn breadcrumbs(req: Request) -> Response {
    let here = url::current(&req);       // "/posts/42?expand=author"
    let full = url::full(&req);          // "https://app.test/posts/42?expand=author"
    let back = url::previous("/");        // 会话记录下来的上一个 URL
    // ...
}
```

| 辅助函数 | 返回值 | 来源 |
|---|---|---|
| `url::current(&req)` | 本次请求的路径 + 查询串 | 当前的 `Request` |
| `url::full(&req)` | 本次请求的绝对 URL | `APP_URL` + `current(&req)` |
| `url::previous(fallback)` | 会话中间件记录下来的上一个 URL | 会话里的 `_previous.url`，或者 `fallback` |

`previous` 是 `Redirect::back` 背后的机制 - 会话中间件记录每一次成功的 HTML `GET` 的 URL，令表单 `POST` 能返回到提交页面。Inertia 局部请求、JSON-API 请求（没有 `text/html` 的 `Accept: application/json`）以及非 2xx/3xx 响应都会跳过，因此不会返回用户从未见过的中间端点。中间件也拒绝记录并非根相对且同源的 URL：形如 `//host` 或 `/\host` 的请求路径（浏览器都会将两者解析为 protocol-relative 而不是路径），或在任意位置带有 ASCII 控制字节的路径（浏览器的 URL 解析器会在比较源之前剥离 `TAB` 或换行符，使看似安全的路径变为上面两种形式之一），均不会被存储 - 而且每次读取时都会再次运行相同检查，所以旧版本存入的值也会持续不通过，而不是仅因已在会话中就被信任。无论过去还是现在，到达应用的异常请求路径都无法将 `previous` 或 `Redirect::back` 引向异源。

## 签名 URL

签名 URL 让您可以铸造一个能证明自己出自您服务器的 URL，而不必把这个 URL 存在任何地方。签名是用您的 `APP_KEY` 对该 URL 的规范形式做的 HMAC-SHA256；服务器会在入站请求上重新算一遍这个 HMAC，只接受能对上的签名。

在下面这些场景请用签名 URL：

- **通过邮件送达的链接** - 密码重置、邮箱验证、邮件邀请、magic-link 登录。这个 URL 必须能扛住在收件箱里走一趟往返，同时又不能作为不透明状态被存起来。
- **短暂的下载链接** - 那种“您的 CSV 导出好了”、24 小时后过期的链接；也可以替代 S3 的签名链接，当您希望这个 URL 留在自己的域名上时。
- **指回您自己的 webhook** - 第三方回调应当拒绝伪造的调用，同时又不要求每个请求都查一次数据库。

```rust
use suprnova::url;
use chrono::Utc;

// 永久的签名 URL - 永不过期。
let link = url::signed_route(
    "password.reset",
    &[("user", user_id), ("token", token)],
)?;
// "/password/reset/42/xyz?signature=ab12cd34..."

// 临时的签名 URL - 从现在起一小时后过期。
let expires_at = Utc::now().timestamp() + 3600;
let link = url::temporary_signed_route(
    "verify.email",
    &[("user", user_id)],
    expires_at,
)?;
// "/verify/email/42?expires=1748803600&signature=def012..."
```

注意，`expires_at_epoch_seconds` 是一个**绝对的 UNIX 时间戳**，而不是一段时长。请在调用点把它算出来：

```rust
let one_hour_from_now = chrono::Utc::now().timestamp() + 3600;
let one_day_from_now  = chrono::Utc::now().timestamp() + 86_400;
```

这让辅助函数的签名保持精简，也让您可以用同一个函数来表达“从现在起多久”和“明确的绝对时刻”这两种截止时间。

### 验证

在入站那一侧，您拿实时的请求来验证签名：

```rust
use suprnova::{url, FrameworkError, Request, Response, HttpResponse};

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    if !url::has_valid_signature(&req)? {
        return Err(FrameworkError::forbidden("Invalid or expired link"));
    }
    // 签名没问题且未过期 - 继续。
    let user_id = req.param("user").unwrap();
    // ...
    Ok(HttpResponse::text("ok"))
}
```

只有当 HMAC 对得上、并且这个 URL 尚未过期时，`has_valid_signature` 才返回 `true`。要在*无效*、*已过期*和*有效*之间做三路区分，请使用 `signature_verdict`：

```rust
use suprnova::{url, FrameworkError, HttpResponse, Request, Response};
use suprnova::routing::SignatureVerdict;

pub async fn reset(req: Request) -> Response {
    reset_inner(req).await.map_err(HttpResponse::from)
}

async fn reset_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    match url::signature_verdict(&req)? {
        SignatureVerdict::Valid => {
            // 继续。
        }
        SignatureVerdict::Expired => {
            // 把用户弹到一个页面，说明链接已经过期，
            // 并提供重新发一封的选项。
            return Ok(HttpResponse::new()
                .status(302)
                .header("Location", "/password/reset-expired"));
        }
        SignatureVerdict::Invalid => {
            // 渲染一个泛泛的 403 - 不要泄漏这个签名到底是格式错误、
            // 缺失，还是仅仅不对。
            return Err(FrameworkError::forbidden("Invalid link"));
        }
    }
    // ...
    Ok(HttpResponse::text("ok"))
}
```

`signature_has_not_expired(&req)` 已经废弃，现在它回答的和 `has_valid_signature` 回答的一模一样。请改用上面的 `signature_verdict`；一个没有 `expires` 查询参数的 URL，按定义就是“永不过期”的 - 在 Suprnova 里如此，在 Laravel 里也是如此。

### 为什么 Suprnova 有所不同

Laravel 的 `URL::signatureHasNotExpired($request)` 字面意思就是“未过期”，所以一个**伪造的**签名也会返回 `true` - 它从来就没有一个可以错过的过期时刻。Suprnova 的版本过去与之一致。现在不再是了：这个辅助函数要求先有一个有效的签名。

原因在于，在 HMAC 给出结论之前，`expires` 都还是攻击者提供的，所以在签名通过校验之前，任何由它推导出来的答案都毫无意义 - 而一个名字听起来像是在把关的函数，却让每一个伪造的 URL 都穿过了任何单独调用它的地方。

要求签名必须有效，就把它坍缩成了 `has_valid_signature`，这也正是它带的是一个废弃标记、而不是一个行为标志的原因。这次坍缩并不是损失：在三态结论之下，除了 `Valid` 之外，没有哪个“未过期”是一个 `bool` 能够诚实报告的。如果您想把*已过期*和*无效*区分开来 - 想说“请重新申请一个链接”而不是“禁止访问” - 那正是 `signature_verdict` 的用途，而且它把这一点写进了类型里。

### 给任意 URL 签名

如果您想签名的 URL 并非来自一条已注册的命名路由 - 比如第三方递给您的一个回调 URL，或者在运行时动态构造出来的一个路径 - 请直接使用 `signed_url`：

```rust
use suprnova::url;

let callback = url::signed_url(
    "/webhooks/stripe/callback?order=42",
    Some(chrono::Utc::now().timestamp() + 600),  // 10 分钟后过期
)?;
```

过期时间传 `None`，铸造出来的就是一个永久签名。验证那一侧是一样的 - `has_valid_signature(&req)` 并不关心这个 URL 是从一条命名路由铸造出来的，还是从一个裸路径铸造出来的。

### 传输格式

两个仅在查询参数顺序上不同的 URL 会产出完全相同的签名，因为规范形式会在做哈希之前，按字典序对查询参数对排序。这一点很重要，因为客户端有时会在传输途中重排查询参数（代理、链接预览器、手机上的邮件应用），而一个一旦被重排就失效的签名 URL 是没法用的。

| 组成部分 | 取值 |
|---|---|
| 算法 | HMAC-SHA256 |
| 密钥 | 当前生效的 `APP_KEY` 原始字节 |
| 载荷 | `path?<sorted-query>`（没有参数时省略 `?`） |
| 排序依据 | `(key, value)` - 每一对都算，重复的也算 |
| 编码 | 十六进制编码的 64 字符摘要 |
| 比较 | 通过 `subtle::ConstantTimeEq` 做常数时间比较 |
| 保留的键 | `signature`、`expires` |

**重复的键会被签名，而不会被折叠。** `?tag=a&tag=b` 会把两个值都带进载荷，所以其中任何一个都无法在不破坏签名的情况下被添加、移除或替换。按 `(key, value)` 而不是只按键来排序，正是让这个顺序成为全序的原因，所以当一个键出现不止一次时，上面那条重排保证依然成立。

这一点值得写下来，因为反过来的做法后果很严重。早先有一个版本把 URL 规范化进了一个映射里，而映射对重复的键只保留最后一个值。`Request::query_param` 返回的却是*第一个*。于是一个合法签名的 `?user=victim` 可以带着原来的签名，被重放成 `?user=attacker&user=victim`：验证看到的是 `victim` 于是放行了，而处理程序作用在了 `attacker` 上。被签名的和被执行的是两个不同的 URL。如今三个查询访问器 - `query_param`、`query_params` 和 `Context::query_param` - 都把重复的键解析为它的最后一个值，而规范形式什么也没丢。

重复的 `signature` 或 `expires` 会被直接拒绝。它们是控制参数；其中任何一个出现两次，“哪一个说了算？”就没有一个非任意的答案，而验证器不该是那个去猜的组件。

HMAC 载荷会排除任何已经存在的 `signature` 查询参数（所以对已签名的东西再签一次是空操作），并根据调用参数重新发出一个新的 `expires` 值。剥掉或改写 `expires` 的客户端会破坏签名；剥掉 `signature` 的客户端会以 `Invalid` 告终。两者都是失败即关闭。

片段（`#section`）会从规范形式里被剥掉，因为浏览器从不把片段传回服务器。把片段纳入签名，会让每一个链接在客户端追加锚点的那一刻就失效 - `?signature=...#docs` 在服务器端是验证不过的。

### 保留的查询参数

`signature` 和 `expires` 是保留的查询参数名。一条确实需要名为 `signature` 或 `expires` 的查询参数的路由，会和签名 URL 的机制撞车，验证器会把这个值张冠李戴。要么给这个参数改名，要么把这条路由的入参包到另一个命名空间之下。

```rust
// 不好 - `signature` 与保留名冲突。
get!("/api/check", check)  // 接受 ?signature=hash

// 好 - 给它加个命名空间。
get!("/api/check", check)  // 接受 ?body_signature=hash
```

为了与 Laravel 的传输格式保持对称，这两个常量也被暴露了出来：

```rust
use suprnova::routing::{SIGNATURE_KEY, EXPIRES_KEY};
// SIGNATURE_KEY == "signature"
// EXPIRES_KEY   == "expires"
```

### 密钥轮换

签名 URL 使用的 `APP_KEY`，和驱动 `Crypt::encrypt` 与会话 cookie 完整性的是同一把。轮换 `APP_KEY` 会让每一个此前铸造、仍在途中的签名失效 - 一封仍在途中的密码重置邮件，会在用户下次点击时变成 403。

对大多数应用来说这就是正确的行为。如果您需要带重叠期的平滑轮换（好让旧链接在整个部署窗口里继续可用），请用 `APP_KEY_PREVIOUS` 把上一把密钥带过来；验证时密钥环会把每一把已安装的密钥都试一遍。密钥环的完整介绍请参见[哈希](hashing.md)那一章。

## 错误与边界情况

有几种失败模式值得了解：

- **`route(name, ...)` 返回 `None`**，当这个名称没有被注册时。这是宽松的那套接口 - 静默失败是有意为之，好让调用方代码能回退到一个默认值。想要明确的失败，请用 `try_route`。
- **`try_route` 对未知的名称返回 `Err(NameNotFound)`**，而当某个必需的 `{placeholder}` 没有对应的值时返回 `Err(MissingParams { name, missing })`。
- **`url::signed_route` 及其同类返回 `FrameworkError`**，当加密密钥没有安装时（比如您在 `.env` 里忘了 `APP_KEY`）。在生产环境里这会在启动时就失败，因为 `Crypt::init` 是在 `Server::from_config` 期间运行的；这里的错误路径存在的意义，是把配置错误醒目地暴露出来，而不是产出无法验证的链接。
- **`has_valid_signature` 返回的是 `Ok(false)`** 而不是 `Err`，对于无效或已过期的签名而言。`FrameworkError` 那个变体留给“服务器根本无从检查”这一类失败（密钥缺失）。
- **一个 `expires` 被篡改过的签名 URL** 验证结果是 `Invalid`，而不是 `Expired`。HMAC 载荷包含 `expires` 的值，所以改动它首先破坏的是签名。

```rust
use suprnova::{routing::SignatureVerdict, url};

// 下面这些全都是 Invalid，而不是 Expired：
url::signature_verdict(&req)?;  // 缺少 signature 查询参数
url::signature_verdict(&req)?;  // signature 是非十六进制的垃圾
url::signature_verdict(&req)?;  // 路径被篡改（/orders/1 → /orders/2）
url::signature_verdict(&req)?;  // 任何一个查询参数的值被篡改
url::signature_verdict(&req)?;  // expires 的值被篡改

// 这个才是 Expired：
url::signature_verdict(&req)?;  // HMAC 有效，但当前时间 > expires
```

## 为什么 Suprnova 有所不同

Laravel 的 `URL` 门面带有 `asset()`、`secureAsset()`、`assetFrom()` 和 `action()`。Suprnova 一个都没有提供 - 这是有意为之。

**静态资源**。Suprnova 在前端这一侧的做法是 Vite 加上文件系统磁盘（[文件系统](filesystem.md)），而不是一个独立的资源辅助函数。Vite 的 `@vite('resources/app.ts')` 指令（或者 Inertia 适配器里的对应物）在生产环境里发出正确的带哈希 URL，在开发环境里发出开发服务器的 URL。再造一条平行的 `URL::asset()` 通道，会把静态资源这件事拆到两个系统上，而这两个系统必须在哈希、版本，以及哪一份 manifest 说了算这些问题上达成一致。Vite 那一侧已经赢下了这份责任。

**Action 路由**。Laravel 的 `action('UserController@show', ['id' => 1])` 依赖 PHP 的类字符串路由 - 控制器是带方法的类，框架可以反向查出一个 `action` 字符串对应什么。Rust 的处理程序是自由函数。最接近的对应物就是命名路由，而 `route("users.show", &[("id", "1")])` 已经是正确的接口了。在 Rust 的处理程序类型之上重新引入按 action 字符串路由，相比命名路由不会带来任何实质的东西。

**`URL::forceScheme()` / `URL::forceRootUrl()`**。Laravel 暴露这两个，是为了测试，也是为了那些位于不传 `X-Forwarded-Proto` 的反向代理之后的站点。Suprnova 用配置来处理这两种情况：`APP_URL` 携带规范的主机和协议；对于代理环境，可信代理中间件（[中间件](middleware.md)）会读取 `X-Forwarded-*` 请求头，并在请求到达您的处理程序之前更新请求 URL。没有什么东西留给 `forceScheme` 去覆盖 - `APP_URL` 已经说明了协议是什么。

真正落在这里的，是使用方真会去用的那套面向用户的形态，并且在能够干净对应的地方沿用了同样的 Laravel 式命名。这次精简是有意的，不是疏漏。

## 下一步

- [路由](routing.md) - 声明路由、给它们命名、路由分组、资源路由，以及完整的逐方法匹配接口
- [响应](responses.md) - `Redirect::route`、`Redirect::signed_route`、`Redirect::back`，以及其余那一族消费 URL 生成的重定向辅助函数
- [哈希](hashing.md) - `APP_KEY` 的生命周期、密钥轮换，以及在加密之外同时支撑 URL 签名的那个共享密钥环
- [认证流程](auth-flows.md) - 签名 URL 在生产中的使用者：密码重置、邮箱验证，以及记住我 cookie
- [请求](requests.md) - `Request::path`、`Request::query`、`Request::route_is`，以及本章每一个辅助函数的反面
