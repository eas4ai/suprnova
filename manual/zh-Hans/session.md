# 会话

会话是那个逐用户的键值对集合，它能在同一个浏览器的多次请求之间存活下来。Suprnova 开箱就带一个由数据库支撑的驱动程序，通过 `SessionMiddleware` 把它接进去，并用两个自由函数暴露当前活跃的会话 - `session()` 用于读，`session_mut()` 用于写。每当一个值应当活得比一次请求更久、但又不该由 URL 或 JWT 来携带时，就用它。

## 一个请求如何看到会话

`SessionMiddleware` 在每一个请求上运行，按顺序做五件事：

1. 从 `suprnova_session` cookie（AES-256-GCM 加密）里读出会话 id，以及上一次成功的活动触碰时间戳。被篡改的、无法解密的或者格式不对的 cookie，一律当作不存在。
2. 只有当一个有效的 cookie 指名了某个会话时，才从存储里加载 `SessionData`。没有 cookie 的请求会以一个干净的内存会话开始，不会必然产生一次数据库未命中。如果一个 cookie 对应的行已经不在了，就把它清掉，而不重新创建一个空行。存储读取出错会记一条 `warn!`，并让一个不带状态的请求继续走下去，但此后处理程序里的任何修改都会失败即关闭，而不是去覆盖未知的已存状态。
3. 让 flash 数据老化：`_flash.old.*` 被丢弃，`_flash.new.*` 被改名为 `_flash.old.*`。这一步之后，上一个请求 flash 进去的东西都是可读的；这个请求 flash 进去的东西则要到下一次才可读。
4. 在处理程序的整个执行期间，把会话绑定到一个任务本地槽位里。`session()` 和 `session_mut()` 查的就是这个槽位。
5. 在处理程序返回之后，持久化脏掉的会话状态、或者做一次受最小间隔约束的滑动过期触碰；只有在写入成功之后才附加一个替换用的加密 cookie；并且排空那些待发的带外 cookie（比如一个刚刚轮换过的记住我 cookie）。一个干净的、没有 cookie 的请求不会做任何会话存储 I/O，也不会收到会话 cookie。

第 5 步有一条安全保证值得单独拎出来：**如果这个请求修改了会话，而存储写入失败了，那么响应会被替换成一个 500。** 返回处理程序的成功结果，就意味着把一个 cookie 交给客户端，而它所对应的状态数据库从未记录下来 - 下一个请求会加载到一个空会话，那次修改（登录、CSRF 轮换、flash）就悄无声息地消失了。那些只读的请求，如果只是在一次到期的 `last_activity` 触碰上失败，就记一条 `warn!`，保留现有的 cookie，然后放行。

## 读取会话

```rust
use suprnova::session::session;

if let Some(s) = session() {
    let user_id: Option<String> = s.get("preferred_username");
    if s.has("cart") {
        // ...
    }
    if s.missing("locale") {
        // 第一次访问
    }
}
```

`session()` 会克隆当前的 `SessionData`。在请求作用域之外它返回 `None`（一个没有安装中间件的单元测试，一个 CLI 子命令）。对于类型化的值，`get::<T>` 会从底层的 JSON 反序列化出来；遇到缺失的键或者类型不对，您拿到的是 `None`，而不会 panic。

## 写入会话

`session_mut` 接受一个闭包，这个闭包收到的是 `&mut SessionData`：

```rust
use suprnova::session::session_mut;

session_mut(|s| {
    s.put("locale", "en");
    s.put("preferences", serde_json::json!({
        "theme": "dark",
        "notifications": true,
    }));
    s.forget("legacy_key");
});
```

这个闭包是同步的 - 底层锁上的守卫会在任何 `.await` 之前被释放，所以它能在异步处理程序内部组合使用，而不会跨越挂起点持有这把锁。您序列化的任何东西都必须实现 `Serialize`；`get` 上的反序列化则要求 `DeserializeOwned`。

采用闭包形式（而不是返回一个守卫）是有意的。Tokio 里的 future 可以在与它启动时不同的工作线程上恢复执行，所以会话必须住在一个 `task_local!` 槽位里，并通过一段绑定到作用域的临界区来借用。`|s|` 这个形态让那条边界变得显式，也阻止您不小心跨越一次 `.await` 持有 mutex 守卫。

## flash 数据

flash 值只在接下来的**一个**请求里可见，然后就消失。常见的用法是：一个控制器写入一条 flash，返回一个重定向，下一个页面把这条 flash 渲染出来。

```rust
use suprnova::session::session_mut;

session_mut(|s| s.flash("status", "Profile updated."));
```

在下一个请求里：

```rust
use suprnova::session::session_mut;

let status: Option<String> = session_mut(|s| s.get_flash("status"));
```

`get_flash` 在返回这个值的同时就把它移除了。想要读取而不消费的那个版本，请用 `get::<String>("_flash.old.status")`，不过控制器通常想要的是会消费掉的那种形式。

Laravel 那套完整的 flash 接口都在：

- `flash(key, value)` - 为下一个请求写入
- `now(key, value)` - 只为当前请求写入
- `reflash()` - 把当前可见的一切重新 flash 一轮
- `keep(&["k1", "k2"])` - 重新 flash 指定的一个子集
- `flash_input(map)` / `old_input()` / `get_old_input(key)` - `Redirect::with_input` / `old()` 这些辅助函数所用的表单输入 bag

## 重新生成与作废

在凭据发生变化之后（登录、密码重置、通过 2FA），您要轮换会话 id，这样变化之前被固定下来的那个 id 就不再有效：

```rust
use suprnova::session::{regenerate_session_id, regenerate_csrf_token};

regenerate_session_id();        // 新 id，数据不变
regenerate_csrf_token();        // 新的 CSRF 令牌，id 和数据都不变
```

要彻底清空会话（登出）：

```rust
use suprnova::session::invalidate_session;

invalidate_session();           // 清空数据 + 铸造一个新的 CSRF 令牌
```

对于那种需要吊销某个用户全部会话的安全事件（在别处重置了密码、账号找回、管理员强制登出）：

```rust
use suprnova::session::destroy_all_for_user;

let rows = destroy_all_for_user("user-42").await?;
tracing::info!(revoked = rows, "all sessions destroyed");
```

`destroy_all_for_user` 会解析由 `SessionMiddleware::new` 或 `with_store` 注册的 `SessionStore`，并在那个已配置的存储上调用 `destroy_for_user`。只有在没有注册任何会话存储时（例如从未构造中间件的测试或嵌入方），它才会回退到一个新建的 `DatabaseSessionDriver`。

## 认证辅助函数

`auth_user_id()` 返回当前已认证的用户 id（先查请求作用域内的认证状态，再回退到持久化的会话字段）：

```rust
use suprnova::session::{auth_user_id, is_authenticated};

if is_authenticated() {
    let uid = auth_user_id().expect("just checked");
    // ...
}
```

您通常是通过 [Auth](authentication.md) 门面来驱动认证的 - `Auth::login`、`Auth::logout`、`Auth::user()`。这些会话辅助函数是那些门面所依托的底层；当您需要检视原始会话，或者在实现自己的认证守卫时，才伸手去用它们。

## 其他操作

`SessionData` 的 API 镜照了 Laravel `Store` 的接口：

| 方法 | 作用 |
|---|---|
| `get::<T>(key)` | 类型化读取 |
| `put(key, value)` | 类型化写入 |
| `forget(key)` | 移除单个键 |
| `forget_many(&[..])` | 移除多个键 |
| `flush()` | 清空全部数据（保留 id） |
| `has(key)` / `missing(key)` | 存在性检查 |
| `has_any(&[..])` / `has_all(&[..])` | 批量存在性检查 |
| `all()` | 借用底层的那个映射 |
| `only(&[..])` / `except(&[..])` | 过滤之后的克隆 |
| `pull::<T>(key)` | 一步完成读取并遗忘 |
| `push(key, value)` | 追加到一个数组值上 |
| `increment(key, n)` / `decrement(key, n)` | 整数计数器 |
| `remember::<T>(key, \|\| default())` | 读取，若无则计算并写入 |
| `replace(&[(k, v), ..])` | 先清空再批量写入 |
| `put_many(&[(k, v), ..])` | 合并式的批量写入 |
| `previous_url()` / `set_previous_url(url)` | `Redirect::back` 读取的东西 |
| `password_confirmed()` / `password_confirmed_at()` | “用户刚刚确认过密码”的时间戳 |

修改类操作请在 `session_mut` 内部使用，读取则用 `session()`。中间件会在成功的 HTML `GET` 响应中自动填充 `previous_url` 槽位，所以 `redirect()->back()` 无需额外操作即可工作。中间件仅记录根相对且同源的 URL：以 `//` 或 `/\` 开头的请求路径（浏览器会将两者都视作 protocol-relative），或在任意位置携带 ASCII 控制字节的路径（`TAB` 或换行能令仅看似根相对的值在浏览器 URL 解析器剥离它后变成上述两种形式），永远不会被存储。`previous_url()` 也会在每次读取时重新检查相同规则，因此在此写入时防护措施出现前由旧版本写入的值会读作不存在，而不会被信任。无论如何，`Redirect::back()`、`Redirect::refresh()` 和 `url::previous()` 都不能将该槽位存过的值解析为应用外的 `Location`。

## 配置

通过环境变量来配置会话 - `SessionConfig::from_env` 会在启动时读取它们：

```env
# 以分钟计的生存期。它同时驱动数据库行的 TTL 和 cookie 的 Max-Age。
SESSION_LIFETIME=120

# 两次滑动过期写入之间的最小秒数（默认 5 分钟）。
# 运行时会强制把它压在会话生存期之下。
SESSION_TOUCH_INTERVAL=300

# 受监督的过期行回收节奏，以秒计（默认 1 小时）。
SESSION_GC_INTERVAL=3600

# 客户端上的 cookie 名字。
SESSION_COOKIE=suprnova_session

# Cookie 属性
SESSION_SECURE=true          # 要求 HTTPS；默认就是 true
SESSION_PATH=/
SESSION_DOMAIN=.example.com  # 可选；不设置 = 仅限本主机
SESSION_SAME_SITE=Lax        # Lax | Strict | None
SESSION_COOKIE_PREFIX=       # 空 | __Secure- | __Host-
SESSION_PARTITIONED=false    # 选择启用 CHIPS
SESSION_EXPIRE_ON_CLOSE=false # true → 省略 Max-Age，浏览器关闭时丢弃

# 会话存储所用的具名数据库连接（可选）
SESSION_CONNECTION=sessions

# 记住我令牌 / cookie 的生存期，以分钟计（默认 30 天）
REMEMBER_LIFETIME=43200
```

有几个默认值值得点出来：

- **`SESSION_SECURE` 默认是 `true`。** 通过明文 HTTP 传输的会话会构成凭据泄漏隐患，所以这个 secure 标志默认就是开着的。对于跑在 HTTP 上的本地开发，请在您本地的 `.env` 里设置 `SESSION_SECURE=false`。
- **`HttpOnly` 始终是开着的。** 没有旋钮可以关掉它 - 把会话 cookie 暴露给 JavaScript，就等于放弃了最主要的那道 XSS 防护，而在今天也没有哪个正当理由需要这样做。
- **`SameSite` 默认是 `Lax`。** `Strict` 会在大多数跨站 GET 导航上挡掉会话（包括从邮件里点回来的链接）；`Lax` 通常才是对的那个答案。

### Cookie 名称前缀加固

`SESSION_COOKIE_PREFIX=__Host-` 会使浏览器把会话和 remember-me cookie 锁定到主机。`__Host-` cookie 必须是 `Secure`、使用 `Path=/` 并省略 `Domain`；`__Secure-` cookie 必须是 `Secure`。Suprnova 在渲染时根据最终 cookie 名称执行这些规则，因此构建器调用顺序和排队的 cookie 都得到相同保护。

`Config::init` 会在启动时验证前缀、`SESSION_DOMAIN` 和 `SESSION_PATH`，若组合无效则在开始服务前失败。渲染时的执行仍会为任一种前缀强制启用 `Secure`，并将 `__Host-` 路径重写为 `/`；对于 `__Host-` 会丢弃 `Domain` 并记录警告，因为这缩小了请求的作用域。浏览器会静默丢弃无效的带前缀 cookie，因此部署前请检查启动诊断。

在本地 HTTP 开发中，请保持前缀为空，并仅在本地环境设置 `SESSION_SECURE=false`。在生产环境，请部署 HTTPS，保持 `SESSION_SECURE=true`，使用 `SESSION_COOKIE_PREFIX=__Host-`，保持 `SESSION_PATH=/`，并将 `SESSION_DOMAIN` 留空。

部署检查清单：

1. 确认公开源为 HTTPS，包括健康检查和第一次重定向。
2. 设置 `SESSION_COOKIE_PREFIX=__Host-`、`SESSION_SECURE=true` 和 `SESSION_PATH=/`。
3. 移除 `SESSION_DOMAIN`；启动验证器会在使用 `__Host-` 时拒绝它。
4. 检查第一个 `Set-Cookie` 响应，确认有 `__Host-suprnova_session`、`Secure` 和 `Path=/`，且没有 `Domain`。

### 为什么 Suprnova 有所不同

Laravel 不会在会话配置中公开一等的 cookie 前缀开关。Suprnova 将前缀设为带启动验证的配置值，因为其失败模式在浏览器端是静默的：无效 cookie 会在应用代码报告会话失败前被丢弃。

要以编程方式配置，请用流式构建器：

```rust
use std::time::Duration;
use suprnova::SessionConfig;

let config = SessionConfig::new()
    .lifetime(Duration::from_secs(60 * 60))      // 1 小时
    .touch_interval(Duration::from_secs(5 * 60))
    .gc_interval(Duration::from_secs(60 * 60))
    .cookie_name("myapp_session")
    .secure(true)
    .domain(".example.com")
    .remember_lifetime(Duration::from_secs(30 * 24 * 60 * 60));
```
`SessionConfig` 是 `#[non_exhaustive]`；在以编程方式配置需要前缀时，请使用默认值并为公开字段赋值：

```rust
use suprnova::{CookiePrefix, SessionConfig};

let mut config = SessionConfig::default();
config.cookie_prefix = CookiePrefix::Host;
```


## 把它接进去

`SessionMiddleware` 是在您应用的启动流程里作为全局中间件安装的。中间件的顺序很重要：会话必须排在 [CSRF](csrf.md) 之前，因为 CSRF 要读取那个逐会话的令牌。

```rust
use std::sync::Arc;
use suprnova::{global_middleware, CsrfMiddleware, SessionConfig, SessionMiddleware};

pub async fn bootstrap() {
    let config = SessionConfig::from_env();

    // `install` 还会顺带把配置好的 GC 监督程序注册进去。
    // 如果您更愿意通过 `Schedule` 自己调度 GC，
    // 请改用 `SessionMiddleware::new(config)`。
    global_middleware!(SessionMiddleware::install(config).await);

    global_middleware!(CsrfMiddleware::new());
}
```

`SessionMiddleware::install` 会注册一个[受监督的](supervisors.md) gc 任务，它按 `SESSION_GC_INTERVAL` 调用 `gc()`（默认每小时一次）。变体 `install_with_gc(config, interval).await` 接受一个自定义的间隔；`new(config)` 则跳过这个 gc 任务（如果您更愿意从一条[任务调度](scheduling.md)条目里调用 `gc()`，这会很有用）。这个受监督的任务参与框架的关停排空流程，所以 gc 循环会在 `Ctrl-C` / `SIGTERM` 时干净地退出，而不是被强行中止。

受保护的运维端点可以在不查询 sessions 表的情况下暴露回收器的状态：

```rust
use suprnova::session::session_gc_metrics;

let metrics = session_gc_metrics();
tracing::info!(
    runs = metrics.runs,
    failures = metrics.failures,
    removed_rows = metrics.removed_rows,
    last_success = metrics.last_success_unix_seconds,
    "session collector status"
);
```

要使用非数据库的存储 - 用于测试，或者用于一个您自己写的、由 Redis 支撑的驱动程序 - 请实现 `SessionStore`，并通过 `with_store` 把它传进去：

```rust
use std::sync::Arc;
use suprnova::{SessionConfig, SessionMiddleware, SessionStore};

let store: Arc<dyn SessionStore> = Arc::new(MyRedisStore::new());
let mw = SessionMiddleware::with_store(SessionConfig::from_env(), store);
```

## sessions 表

默认的驱动程序期望有一张这种形态的 `sessions` 表（`framework/src/session/driver/database.rs` 里的那个 SeaORM 实体才是事实来源）：

| 列 | 类型 | 说明 |
|---|---|---|
| `id` | VARCHAR PK | 40 个字符的小写字母数字会话 id |
| `user_id` | VARCHAR NULL | 已认证的用户 id（字符串，支持不透明的 id） |
| `payload` | TEXT | JSON 序列化的会话数据映射 |
| `csrf_token` | VARCHAR | 逐会话的 CSRF 令牌 |
| `last_activity` | TIMESTAMP | 最后一次访问；驱动过期 + GC |

随这张表一起提供的还有两个索引：`idx_sessions_user_id`（给 `destroy_for_user` 用）和 `idx_sessions_last_activity`（给 `gc()` 用）。

用脚手架生成的应用里包含一个与这个形态相符的 `create_sessions_table` 迁移。如果您自带迁移，请把列名一字不差地照搬 - SeaORM 是按位置解析它们的，改过名的列对不上。

### 为什么 Suprnova 有所不同

有两个地方，Laravel 做了一个 PHP 形状的选择，而 Tokio 让我们可以做得不一样：

**垃圾回收。** Laravel 在每个请求上跑一次 2/100 的抽签：每个请求有 2% 的概率就地触发一次会话 GC。这在 PHP 上行得通，因为每个请求本来就会派生一个全新的进程。在 Tokio 上我们有的是长期存活的工作进程，所以 `SessionMiddleware::install` 会注册一个[受监督的](supervisors.md)任务，按固定的间隔调用 `gc()`。没有逐请求的开销，也没有概率性的意外 - 用显式的调度取代抽签，而且监督程序的重启循环会接住 panic，所以一次糟糕的 gc 不会弄死这个守护进程。

**闭包形式的 `session_mut`。** Laravel 把 `$request->session()` 递给您，让您在它上面调用方法。我们不这么做，因为 Suprnova 里的处理程序是 future，而一个 future 可以在与它启动时不同的工作线程上恢复执行。会话住在一个 Tokio `task_local!` 槽位里，这意味着借用式的访问必须发生在一个作用域内部。闭包形式让这个作用域变得显式，并在静态层面上杜绝了跨越 `.await` 持有 mutex 守卫这个错误。

**脏写入时失败即关闭。** 一次受约束的活动触碰写失败，会记一条 `warn!`，并让请求带着它现有的 cookie 通过（用户可见的状态是完好的）。而一次*被修改过*的会话的写入失败 - 登录、flash、CSRF 轮换 - 会返回 500。悄悄把一个 cookie 交给客户端、而它所对应的状态存储从未记录下来，会让一次“成功的”登录在紧接着的下一个请求里就消失；不如把这次失败醒目地暴露出来。

## 下一步

- [认证](authentication.md) - `Auth::login`、认证守卫、用户提供者链
- [认证流程](auth-flows.md) - 密码重置、2FA、暴力破解限流、记住我
- [CSRF](csrf.md) - 会话的 CSRF 令牌在写操作上是如何被检查的
- [中间件](middleware.md) - 编写您自己的、读写会话的中间件
- [请求生命周期](lifecycle.md) - `SessionMiddleware` 在这条链上处于什么位置
