# 认证流程

`suprnova::auth_flows` 是叠在[认证](authentication.md)之上的生命周期层。`auth::*` 回答“这次请求是谁”，`auth_flows::*` 覆盖邮箱证明、密码恢复、账户锁定和框架 TOTP 质询。

命名空间下提供五个表面：

- `EmailVerification` 铸造并消费框架 `auth_flow_tokens`，通过 [`Mail`](mail.md) 门面发送邮件，并通过配置的 `UserProvider` 将已认证的令牌所有者标记为已验证。
- `PasswordReset` 会在已安装的 Magnetar 引擎可用时使用该引擎。如果没有 Magnetar，已验证账户可以通过配置的 `UserProvider` 和框架 `auth_flow_tokens` 重置密码。未验证账户将按故障关闭原则被拒绝，因为通用提供程序无法执行 Magnetar 的原子化首次电子邮件证明策略。
- `BruteForce` 和 `LoginThrottleMiddleware` 将账户锁定状态委托给安装的 Magnetar 引擎。
- `TwoFactor` 是位于 `two_factor_credentials` 之上的框架拥有 TOTP 门面。它提供注册、确认、验证、恢复码、密钥轮换、质询提升和时间步重放保护。
- `remember_me` 为命名空间兼容性重导出 legacy 框架 remember 模块。安装 Magnetar 后，普通的 `Auth` 和 `SessionMiddleware` remember 流会改用 Magnetar 凭据。

同一个命名空间下还带着两个路由门中间件：

- `EnsureEmailVerifiedMiddleware` 组合在 `AuthMiddleware` 之后，根据 `email_verified_at` 给路由加门。
- `TwoFactorChallengeMiddleware` 组合在 `AuthMiddleware` 之前，把一个带着待定框架 TOTP 质询的会话重定向到质询表单。

每一条事务性消息都通过框架 [`Mail`](mail.md) 门面发送。Magnetar 提供安全引擎和存储契约；它不会安装第二套应用邮件传输。

### 状态住在哪里

电子邮件验证令牌住在框架的 `auth_flow_tokens` 表中，验证时间戳通过配置的 `UserProvider` 写入。验证绑定 actor：当前已认证用户必须拥有该令牌。

密码重置令牌、密码凭据、锁定行、不透明会话、remember 凭据、passkey ceremony、OAuth ceremony 和 auth epoch 属于安装的 Magnetar 主机引擎。密码重置、magic link 和 OAuth 经验证电子邮件完成共享 Magnetar 用于回收未验证账户的原子首次电子邮件证明边界。

本章的公共 `TwoFactor` 门面保留其框架拥有的 `two_factor_credentials` schema。Magnetar 也有一个供集成密码、magic-link、passkey、OAuth 和会话流使用的 factor 引擎。不要假设两个存储可以互换：一个给定应用应始终使用一个注册表面。

Suprnova 继续拥有 HTTP 中间件、cookie、出站邮件、事件和 `UserProvider` 桥接。应用代码使用框架门面，而不直接调用存储引擎。

## 跨流程的失败语义

每一个门面都遵循同一条排序规则：持久的状态变更先提交，通知类的副作用后触发。变更之后发生的一次监听者 panic、一次短暂的邮件传输故障，或者一次分发器错误，都不能把这次变更回滚。

- `EmailVerification::verify` 要求已认证的令牌所有者，在触发 `EmailVerified` 前消费令牌并将用户标记为已验证。
- `PasswordReset::complete` 会在已安装的 Magnetar 引擎可用时通过该引擎提交，包括首次证明策略、推进身份验证纪元以及原子化撤销。提供程序回退仅适用于已验证账户：它会使用框架令牌、轮换提供程序密码，然后报告框架会话和“记住我”的撤销结果。邮件和事件随后运行。
- `BruteForce::unlock_account` 会先提交这次解锁，然后才触发 `AccountUnlocked`。
- `TwoFactor::confirm` 会先盖上 `confirmed_at` 的戳，然后才触发 `TwoFactorEnrolled`；`TwoFactor::disable` 会先删除这一行，然后才触发 `TwoFactorDisabled`；`TwoFactor::complete_challenge` 会先把待定提升为已认证，然后才派发标准的 `auth::Login` + `auth::Authenticated` 这一对，紧接着是 `TwoFactorChallenged`。

一个需要持久性的监听者，应该自己缓冲这份工作（从监听者函数体里排一个任务进队列）；这个门面本身从不重试。

## 应用启动

在 `DB::init` 之后以及 `APP_KEY` 已初始化 `Crypt` 后初始化 Magnetar：

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config).await
}
```

`init_magnetar` 会在未禁用迁移时创建默认认证 schema，然后原子安装密码/会话和 passkey 适配器。第二次调用会返回错误。需要进程全局安装的测试应使用专用集成测试二进制文件，因为已安装引擎不可替换。

### 电子邮件验证

电子邮件验证需要：

1. 一个已注册的 `UserProvider`，能按电子邮件取回用户并标记验证时间戳。
2. 应用用户类型上的 `MustVerifyEmail`。
3. 可为空的 `email_verified_at` 列。
4. 框架 `auth_flow_tokens` 表。

```rust
use chrono::{DateTime, Utc};
use suprnova::MustVerifyEmail;

impl MustVerifyEmail for User {
    fn email(&self) -> &str {
        &self.email
    }

    fn email_verified_at(&self) -> Option<DateTime<Utc>> {
        self.email_verified_at
    }

    fn set_email_verified_at(&mut self, value: Option<DateTime<Utc>>) {
        self.email_verified_at = value;
    }
}
```

验证处理程序必须在已认证会话范围内运行。属于另一用户的有效令牌会被拒绝而不被消费。

### 密码重置和锁定

`BruteForce` 需要已安装的 Magnetar 密码引擎。密码重置优先使用该引擎，但当 `M` 实现 `MustVerifyEmail + CanResetPassword` 时，`EloquentUserProvider<M>` 支持已验证用户重置密码。未验证用户不会收到由提供程序支持的重置链接。要将重置用作首次邮箱原子化证明，请安装 Magnetar。

密码重置在发送时防枚举。完成会使用原子首次电子邮件证明存储，并为需要显式会话或 remember 吊销状态的调用方返回 `PasswordResetOutcome`。

### 注册 2FA 迁移

框架自带这套表结构；您的应用通过在自己的迁移器里列出这两个迁移来选择接入：

```rust
use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            // ……您自己的迁移……

            // 创建 `two_factor_credentials`。
            Box::new(suprnova::auth_flows::two_factor::migration::Migration),
            // 添加用于防止 TOTP 重放的 `last_used_timestep`。
            Box::new(suprnova::auth_flows::two_factor::migration_replay::Migration),
        ]
    }
}
```

两者对已经应用过的数据库都是幂等的（v1 使用 `CREATE TABLE IF NOT EXISTS`；v2 是一次加列）。对已经有此 schema 的生产数据库重新运行 `suprnova migrate` 是空操作。

### 环境变量

这些事务性的 mailable，在发送时会读取两个环境变量：

| 变量 | 默认值 | 用途 |
|---|---|---|
| `APP_NAME` | `"Suprnova"` | 主题品牌，以及身份验证器应用展示的那个 `otpauth://` issuer 标签。 |
| `MAIL_FROM` | 无 - **未设置时报错** | 每一条外发消息上的信封 `From`。请设成一个已验证的发件域名。 |

`MAIL_FROM` 故意没有默认值。回退到一个像 `noreply@example.com` 的占位符，会在生产环境里悄悄弄坏 DMARC / SPF，并从一个运维人员不掌控的域名发信，所以这个门面选择了失败关闭。`EmailVerification::send_link` 和 `PasswordReset::send_link`
会把这个错误以 `Err` 的形式暴露出来；`PasswordReset::complete` 则通过
`tracing::warn!` 记日志并继续（密码变更早已提交，所以这条通知路径没法把它回滚）。

应用还需要设置 `APP_URL`，好让控制器能推导出 `send_link` 调用所用的基础
URL；框架的门面本身，是把这个基础 URL 当作一个参数接收的。

邮件驱动程序是通过 `MAIL_DRIVER` 单独配置的 - 参见 [邮件](mail.md) 文档。

## 电子邮件验证

`EmailVerification` 会对照 `auth_flow_tokens` 表铸造、检查并消费验证令牌，并通过配置好的提供者把用户标记为已验证。四个操作覆盖了整个生命周期：

| 方法 | 签名 | 说明 |
|---|---|---|
| `send_link` | `send_link<U: MustVerifyEmail>(user: &U, base_url: &str) -> Result<()>` | 已经拿到一个用户在手时，铸造并寄出邮件。 |
| `resend` | `resend(email: &str, base_url: &str) -> Result<()>` | 防枚举：按邮箱查找用户；一个未知的地址会静默返回 `Ok(())`。 |
| `check` | `check(token: &str) -> Result<bool>` | 不消费令牌 - 在一个落地页上调用是安全的。 |
| `verify` | `verify(token: &str) -> Result<String>` | 绑定 actor 且一次性：已认证用户必须拥有令牌；成功会消费它、将用户标记为已验证，并返回该用户 ID。 |

```rust
use suprnova::auth_flows::EmailVerification;

// 在一次新注册之后，手上拿着刚创建出来的用户：
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// 可选的落地页检查 - 不消费令牌，所以刷新页面
// 不会烧掉这个令牌。
let valid: bool = EmailVerification::check(&token_str).await?;

// 点击跳转处理程序受认证保护。只有当 `Auth::id()`
// 与令牌所有者匹配时，`verify` 才会消费令牌。
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` 成功时会触发 `EmailVerified` - 监听者是解锁额外功能（欢迎邮件、默认关注、“完善您的资料”这类行动号召）的正确位置，不需要把它们和验证处理程序耦合在一起。这个事件携带的是提供者的那个用户 id。

### 重新发送端点（防枚举）

`resend` 只接受邮箱，并通过当前活跃的提供者查找用户。未知的提供者结果会规范化为 `Ok(())`；对于已知账户，门面会铸造令牌并发送邮件。`EmailVerification::resend` 同样会把未知提供者结果规范化为 `Ok(())`；但它不保证在令牌存储或邮件投递失败时具有相同的耗时或相同行为。处理程序仍然可以在任一成功结果之后返回一条中性消息：

```rust
use std::collections::HashMap;
use suprnova::auth_flows::EmailVerification;
use suprnova::{FrameworkError, HttpResponse, Request, Response};

pub async fn resend(req: Request) -> Response {
    resend_inner(req).await.map_err(HttpResponse::from)
}

async fn resend_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let email = params
        .get("email")
        .ok_or_else(|| FrameworkError::bad_request("missing email"))?;

    let base = format!(
        "{}/auth/verify",
        std::env::var("APP_URL").unwrap_or_else(|_| "http://localhost:8765".into()),
    );
    // `resend` 在内部完成查找 + 防枚举。
    EmailVerification::resend(email, &base).await?;

    Ok(HttpResponse::text(
        "If this email is on file, a verification link has been sent.",
    ))
}
```

`send_link` 和 `resend` 都会把 URL 构造成 `{base_url}?token={plaintext_token}`。
`base_url` 末尾的斜杠，会在查询字符串被追加之前被裁掉，所以
`https://app.example.com/verify/` 和 `https://app.example.com/verify` 都会产出一个干净的 URL。

点击跳转处理程序必须运行在 `AuthMiddleware` 之后。它从查询字符串取出令牌并调用 `verify`：

```rust
async fn verify_inner(req: Request) -> Result<HttpResponse, FrameworkError> {
    let raw = req.query().unwrap_or("");
    let params: HashMap<String, String> =
        url::form_urlencoded::parse(raw.as_bytes()).into_owned().collect();
    let token = params
        .get("token")
        .ok_or_else(|| FrameworkError::bad_request("missing token"))?;

    let _user_id = EmailVerification::verify(token).await?;

    Ok(HttpResponse::new().status(302).header("Location", "/"))
}
```

`verify` 会在消费前将 `Auth::id()` 与令牌所有者比较。属于另一账户的令牌会返回相同的无效令牌响应，并保持未使用。成功时，提供者会将已认证所有者标记为已验证，且门面触发 `EmailVerified`。

### 仅限已验证用户的路由：`EnsureEmailVerifiedMiddleware`

`EnsureEmailVerifiedMiddleware` 会根据已认证用户的 `email_verified_at`，给路由加门。把它组合在 `AuthMiddleware` 之后，这条链就会挡住任何一个用户还没完成验证步骤的请求。

**403 JSON** 和 **302 HTML 重定向** 之间的选择，是在路由注册时通过构造函数做出的 - 没有任何请求内容探测，这和 `AuthMiddleware::new` /
`AuthMiddleware::redirect_to` 定下的模式是一致的：

```rust
use suprnova::{AuthMiddleware, EnsureEmailVerifiedMiddleware, group, get};

// API 表面 - 带 JSON 正文的 403。
group!("/api")
    .middleware(AuthMiddleware::new())
    .middleware(EnsureEmailVerifiedMiddleware::new())
    .routes([
        get!("/me", profile::show),
    ]);

// Web 表面 - 302（对 Inertia 访问则是 409 + X-Inertia-Location）。
group!("/dashboard")
    .middleware(AuthMiddleware::redirect_to("/login"))
    .middleware(EnsureEmailVerifiedMiddleware::redirect_to("/email/verify"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

如果没有用户被认证，这个中间件会落进和“已认证但未验证”相同的响应分支 - 这和 Laravel `! $request->user() || ! hasVerifiedEmail()` 的形态是一致的。如果您想对未认证的请求单独给一个 `401`，就把 `AuthMiddleware` 组合在前面。

对于处理程序内部的分支（比如在不重定向的情况下，有条件地渲染一个“请验证”
的行动号召），通过会话认证守卫加载这个类型化的用户，并读取这个 trait 方法：

```rust
use suprnova::{Auth, MustVerifyEmail};
use crate::models::users::User;

if let Some(user) = Auth::user_as::<User>().await? {
    let verified: bool = user.is_email_verified();
    // 据此分支
}
```

## 密码重置

`PasswordReset` 有四个操作：

| 方法 | 签名 | 说明 |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | 防枚举的 Magnetar 签发；未知地址静默返回 `Ok(())`。 |
| `check` | `check(token: &str) -> Result<bool>` | 通过安装的 Magnetar 引擎进行不消费的验证。 |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | 原子消费令牌、应用首次证明策略、轮换凭据、吊销会话和 remember 状态，并返回用户 ID。 |
| `complete_with_outcome` | `complete_with_outcome(token, new_password) -> Result<PasswordResetOutcome>` | 运行相同事务，并返回已提交的吊销计数。 |

```rust
use suprnova::auth_flows::PasswordReset;

// 来自“忘记密码”表单。永远是 Ok(()) - 这个门面会查找
// 用户，只有在有账户在档时才会发送。
PasswordReset::send_link(&email, "https://app.example.com/reset").await?;

// 在渲染新密码表单之前，可选的落地页检查。
let valid: bool = PasswordReset::check(&token).await?;

// 这个点击跳转处理程序，在用户提交一个新密码之后：
// 消费令牌 + 轮换密码，返回用户 id。
let user_id: String = PasswordReset::complete(&token, &new_password).await?;
```

`complete` 会通过 `SecretString` 传递明文密码；Magnetar 在凭据引擎内对其哈希。不要预先哈希。空密码或仅空白密码会在调用引擎前返回 HTTP 400。

### 防枚举

`PasswordReset::send_link` 只会在滥用限流器、邮件配置、引擎和存储检查都成功之后，才会对未知地址返回 `Ok(())`。配置、限流器、存储和邮件失败仍然返回 `Err`。自用（dogfood）控制器让已知账户和未知账户的成功请求具有相同的 HTTP 状态码和正文，但实现不会让两者的执行时间相等。

### `complete` 的副作用

Magnetar 会在一个事务中提交密码重置：

1. 消费一次性重置令牌。
2. 当账户仍未验证时，应用首次电子邮件证明策略。
3. 哈希并替换密码。
4. 推进认证 epoch。
5. 吊销旧的不透明会话和 remember 凭据。
6. 当此次重置是账户的首次邮箱证明时，移除临时凭据。

提交后，框架发送 `PasswordChangedMail` 并分派 `PasswordResetCompleted`。邮件或监听器失败不能回滚重置。

对已经验证的账户，重置会保留合法 passkey、关联账户和已确认的双因素注册。对未验证的被抢占账户，首次证明会移除临时凭据，使先前注册者无法保留访问权。

## 暴力破解防护

暴力破解这一层有两个部分：记录并查询锁定状态的 `BruteForce` 门面，以及在处理程序被调用之前，就在 HTTP 层短路的 `LoginThrottleMiddleware`。

### `BruteForce` 门面

从您登录处理程序的认证失败分支里调用 `record_failed_attempt`，从成功分支里调用 `reset_attempts`：

```rust
use suprnova::auth_flows::BruteForce;

// 在认证失败路径里：
let status = BruteForce::record_failed_attempt(&email, Some(&peer_ip)).await?;
if status.is_locked {
    // 可以选择性地暴露一个自定义响应。中间件会在
    // *下一次*请求上替您做这件事 - 见下文。
}

// 在成功路径里：
BruteForce::reset_attempts(&email).await?;
```

`record_failed_attempt` 返回更新后的 `LockoutStatus`（`is_locked`、
`failed_attempts`，以及锁定时的 `locked_until`）。为审计日志传入这个可选的
`ip`；如果您的传输层没法干净地暴露客户端 IP，就传 `None`。

另外两个操作：

```rust
// 只读 - 对没有历史记录的邮箱是安全的。
let status = BruteForce::get_lockout_status(&email).await?;
let locked: bool = BruteForce::is_locked(&email).await?;

// 管理员 / 强制解锁。只在一次真实的状态迁移上才触发
// `AccountUnlocked`（对一个已经解锁的账户做空操作式的解锁不会触发）。
let was_locked: bool = BruteForce::unlock_account(&email).await?;
```

当这次调用发生时账户曾经处于锁定状态，`unlock_account` 返回 `true`，否则返回 `false`。`AccountUnlocked` 事件只在 `true` 时触发 - 一次 `false` 的返回就是它本来的样子：一次空操作，不是一次审计事件。

### `LoginThrottleMiddleware`

这个中间件会读取一次请求所针对的那个邮箱的锁定状态，并在账户被锁定时，用
`429 Too Many Requests` 短路。登录处理程序永远不会被调用，所以一个被锁定的账户，连尝试一次凭据检查的机会都没有：

```rust
use suprnova::auth_flows::LoginThrottleMiddleware;
use suprnova::Router;

// 这个邮箱提取器是一个包在 `&Request` 上的同步闭包。读取
// JSON/表单正文是异步的，并且会消费掉 `Request`，所以这个闭包
// 没法读取正文 - 请改从一个请求头、查询字符串或者路由参数里取。
let throttle = LoginThrottleMiddleware::new(|req| {
    req.header("X-Login-Email").map(str::to_string)
});

let router = Router::new()
    .post("/login", login_handler)
    .middleware(throttle);
```

实际可用的提取表面：

- 一个请求头（`X-Login-Email`），由前面的某个预处理程序设置 - 这是自用（dogfood）应用所用的模式。
- 一个查询字符串参数（`?email=…`）。
- 一个路由参数（`/login/{email}`）。

从提取器返回 `None`，是那个显式的“我没什么可检查的”信号 - 中间件会原样放行这个请求。这让这个中间件可以安全地安装在那些偶尔会看到匿名流量的路由上（比如同一个 `POST /login` 端点，也处理一个不带邮箱的“请求密码重置”子动作）。

锁定时，这个中间件会返回：

- 状态码 `429 Too Many Requests`。
- `Retry-After` 请求头 - 秒数，通过 `LockoutStatus::retry_after_seconds`，从这次锁定的 `locked_until` 算出来。如果这个时间戳不知何故缺失了，就回退到 `900`（15 分钟，Magnetar 的默认锁定周期）。
- 正文：`"Account locked due to too many failed login attempts. Try
  again later."`

### 后端错误（默认失败关闭）

如果 `get_lockout_status` 返回错误，`LoginThrottleMiddleware` 会记录这次失败，并默认返回 HTTP `503 Service Unavailable` 以及 `Retry-After: 1`，而不会调用登录处理程序。要在锁定后端中断期间保持登录可用，必须显式选择加入 `.on_backend_error(BackendErrorPolicy::FailOpen)`；只有这一策略会把请求传递给处理程序。

### 和 `RateLimitMiddleware` 叠加使用

`LoginThrottleMiddleware` 是按账户的 - 一旦越过阈值，它就会给单个邮箱加门。对于按 IP 的配额，把它和 [`RateLimitMiddleware`](rate-limiting.md) 叠加起来。这两者能很自然地组合：

```rust
let router = Router::new()
    .post("/login", login_handler)
    .middleware(LoginThrottleMiddleware::new(|req| { /* ... */ }))
    .middleware(RateLimitMiddleware::ip_based(20, std::time::Duration::from_secs(60)));
```

两者合在一起，覆盖了撞库攻击现实中的那些形状：分散式的（一个邮箱 × 许多
IP）是速率限制的活；集中式的（许多次尝试 × 一个邮箱）是这个限流中间件的活。

### 配置

`MagnetarConfig` 接受一个 `LockoutConfig`。默认值是五次失败尝试、15 分钟的计数和锁定周期、七天的尝试保留，以及 `BackendErrorPolicy::FailClosed`：

```rust,ignore
let config = MagnetarConfig::from_sea_orm(database)
    .lockout_config(lockout_policy);
```

只有在另一个故障关闭的身份控制措施取代账户锁定时，才使用 `LockoutConfig::disabled()`。

## 双因素认证（TOTP）

`TwoFactor` 覆盖的是基于 TOTP 的 2FA - 能和任何符合标准的身份验证器应用（Google Authenticator、1Password、Bitwarden、Authy）配对的那一种。这个流程是绑定 → 确认 → 持续的验证，再加上供用户丢失设备时使用的一次性恢复码，再加上把这一切缝进登录生命周期的质询流程。

### `TwoFactorUser` trait

框架没法伸手进您应用的用户存储里，所以调用方要实现一个小 trait，把自己的用户模型和 2FA 门面桥接起来：

```rust
use suprnova::auth_flows::TwoFactorUser;

pub trait TwoFactorUser: Send + Sync {
    fn user_id(&self) -> &str;
    fn email(&self) -> &str;
}
```

`user_id` 是不透明的存储键。它可以是渲染为文本的数字应用 ID、UUID 或 Magnetar `UserId`。框架 TOTP 表没有指向应用用户表的外键。

`email` 会被折进 `otpauth://` URL 的 `account_name` 段里，好让身份验证器应用显示可识别的账户标签。

```rust
use suprnova::auth_flows::TwoFactorUser;

struct AppUser2fa<'a> {
    user: &'a User,
}

impl TwoFactorUser for AppUser2fa<'_> {
    fn user_id(&self) -> &str {
        &self.user.auth_id
    }

    fn email(&self) -> &str {
        &self.user.email
    }
}
```

### 存储

2FA 状态住在框架拥有的 `two_factor_credentials` 表里。密钥和恢复码在落地时，会用 `crate::crypto::Crypt::encrypt_string` 加密，这需要一个进程全局的
`EncryptionKey`。应用通过在自己的 `Migrator::migrations()` 里列出这两个迁移，来选择接入这套表结构 - 参见[应用启动](#应用启动)。

### 绑定、确认、验证

```rust
use suprnova::auth_flows::{TwoFactor, EnrollmentResponse};

// 1. 绑定：生成一个新密钥 + 10 个恢复码，把它们加密
//    持久化，返回渲染二维码所需要的一切。
let response: EnrollmentResponse = TwoFactor::enroll(&user_2fa).await?;
// response.otpauth_url - `otpauth://totp/...` 深链接
// response.qr_code_svg - 包着一个 base64 PNG 的 <svg>，内联嵌入
// response.recovery_codes - Vec<String>，10 个明文恢复码 - 只展示一次

// 2. 确认：用户打开身份验证器应用，输入这个
//    6 位数的验证码。`confirm` 会校验它并盖上 `confirmed_at` 的戳。
TwoFactor::confirm(&user_2fa, &user_typed_code).await?;
// 触发 `TwoFactorEnrolled`

// 3. 在后续的登录上，用 `verify` 给会话加门：
let ok: bool = TwoFactor::verify(&user_2fa, &code_from_login_form).await?;
if !ok {
    return Err(suprnova::FrameworkError::domain("invalid 2FA code", 401));
}
```

`enroll` **恰好一次**返回明文恢复码。之后没有任何 API 可以再把它们取回来 -
从这一刻起，这个加密列就是单向的。把它们展示在绑定成功页上，鼓励用户保存它们，并且不要在别的任何地方存储这份明文。

`enroll` 拒绝覆盖一个**已确认**的绑定 - 它会返回一个 `409`，把调用方推向
`re_enroll`，而后者需要持有证明。对一个未确认（待定）的行重新绑定是允许的：之前那次绑定从未真正成为权威版本。

### 重放保护

`verify` 成功时，会把当前的 TOTP 时间步写进 `last_used_timestep`。之后
`current_timestep <= last_used_timestep` 的验证，即便验证码本身结构上是合法的，也会被拒绝，从而挫败一次在 30 秒窗口内对被盗验证码的重放。

这次时间步的认领是原子性的。这个戳是通过一次带条件的
`UPDATE … WHERE last_used_timestep IS NULL OR last_used_timestep < :current`
落地的，只有当这条语句正好影响一行时，这次验证才算成功。同一个时间步里的两次并发验证，不可能同时赢：第一个翻转了这一列，第二个的谓词就不再匹配了，第二个会被当作一次重放。一次朴素的读-改-写会是一次 TOCTOU 竞争 - 两次验证都读到盖戳之前的那一行，都校验同一个验证码，都盖戳，都成功。并发的竞争者也会被计入失败尝试，所以暴力破解计数器会把它们记下来。

### 恢复码

```rust
let consumed: bool = TwoFactor::consume_recovery_code(&user_2fa, &code).await?;
```

一次性：一个匹配的恢复码，会在这次调用返回之前，从这一行里移除，所以对同一个恢复码的第二次尝试会返回 `false`。恢复码是 12 位十进制数字，形状是
`NNNNNN-NNNNNN`（每个大约 40 比特的熵，匹配 Laravel Fortify 的格式）。

只有当 2FA 已经完全确认时，`consume_recovery_code` 才会接受恢复码 - 只要
`confirmed_at` 还是 NULL，它就会短路到 `Ok(false)`。没有这道门，一个在一个受害者账户上触发了绑定的攻击者（或者任何在不确认的情况下创建这一行的流程），就可以只用一个新鲜的恢复码来认证，彻底绕过 TOTP。这份契约，和 `verify`
那道“仅限已确认的绑定”的防护是对称的。

### 轮换恢复码和密钥

当一个用户用完了自己的恢复码，或者在怀疑遭到了入侵之后想轮换它们时：

```rust
let fresh: Vec<String> = TwoFactor::regenerate_recovery_codes(&user_2fa, &proof).await?;
```

`proof` 必须能验证为一个当前有效的 TOTP 验证码，或者一个未使用的恢复码。没有这道证明检查，一个会话被劫持的攻击者，就可以悄悄地把合法用户的恢复码全部清空（针对账户恢复的拒绝服务）。这些新恢复码会替换掉已持久化的那一套；既有的密钥和 `confirmed_at` 会被保留下来，所以用户的身份验证器应用不需要重新配对，照样能用。错误：

- `400` - 不存在已确认的绑定；请先调用 `enroll`/`confirm`。
- `401` - `proof` 既不能验证为一个 TOTP 验证码，也不能验证为一个未使用的恢复码。
- `429` - 账户被暴力破解限流锁定了。

要在不先停用 2FA 的情况下轮换**密钥**（重新配对到一个新设备）：

```rust
let response = TwoFactor::re_enroll(&user_2fa, &proof).await?;
```

和 `regenerate_recovery_codes` 一样的证明模型。这一行会被改写成一个新密钥 +
10 个新恢复码；`confirmed_at` 会重置为 NULL，所以用户必须先用来自新身份验证器的验证码 `confirm`，2FA 才会重新变为激活状态。

### 停用

```rust
TwoFactor::disable(&user_2fa).await?;
// 只有当一行被移除时，才触发 `TwoFactorDisabled`
```

幂等：对一个从未绑定过的用户做停用，不是一个错误。`TwoFactorDisabled` 事件只在一次真实的状态迁移上才触发，所以审计监听者看到的是每一次真正的停用对应一条记录，而不是每一次点击一个空操作按钮都对应一条。

### 质询流程（用第二因素给登录加门）

绑定 / 确认 / 验证这几个原语是构件；**质询流程**把它们缝进登录生命周期，让一个开启了 2FA 的用户，没法只靠密码就到达受保护的页面。

这个流程：

1. 密码登录解析出一个用户。
2. 如果 `TwoFactor::is_enabled_by_id(&user_id)` 返回 `true`，登录处理程序就会调用 `TwoFactor::start_challenge(user_id, remember)` - 它会把这个用户
   id 以**待定**状态存进会话，清空那个完全已认证的槽位，吊销任何由
   `Auth::attempt` 签发的记住我 cookie，并记住这个用户是否选择了记住我，好让这个 cookie 能在质询完成之后被重新签发。从这一刻起，直到这次质询完成之前，`Auth::id()` 都会返回 `None`。
3. 这个处理程序会重定向到一个展示验证码表单的 `/two-factor-challenge` 路由。
4. 这个质询 POST 处理程序会调用 `TwoFactor::complete_challenge(code)` - 验证这个验证码（TOTP **或者**一个未使用的恢复码，匹配 Fortify 的质询控制器），把待定提升为已认证，轮换会话 id（挫败会话固定攻击）和 CSRF 令牌，在用户选择了记住我时重新签发那个 cookie，并派发标准的 `auth::Login` +
   `auth::Authenticated` 生命周期事件，加上 2FA 专属的
   `TwoFactorChallenged`。

```rust
use suprnova::auth_flows::TwoFactor;
use suprnova::{Auth, Authenticatable, Credentials, redirect};

pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(&Credentials::password(&form.email, &form.password), form.remember).await? {
        Some(user) => {
            let user_id = user.get_auth_identifier();
            if TwoFactor::is_enabled_by_id(&user_id).await? {
                // 降级为“待定”：认证槽位被清空，待定被设置，
                // 记住我 cookie 被吊销。把表单的
                // remember 标志传下去，好让 `complete_challenge` 能在成功时
                // 重新签发这个 cookie。
                TwoFactor::start_challenge(user_id, form.remember).await?;
                redirect!("/two-factor-challenge").into()
            } else {
                redirect!("/dashboard").into()
            }
        }
        None => Err(invalid_credentials().into()),
    }
}

pub async fn complete(form: TwoFactorChallengeRequest) -> Response {
    let _user = TwoFactor::complete_challenge(&form.code).await?;
    // 会话 id + CSRF 都已经轮换过；如果原始的登录表单设置过它，
    // 记住我也已经被重新签发。挂在
    // `auth::Login` / `auth::Authenticated` 上的监听者，看到的是一次正常的登录。
    redirect!("/dashboard").into()
}
```

`complete_challenge` 会把轮换会话 id 和 CSRF 令牌，作为提升为已认证的一部分来做。这堵住了那个经典的会话固定攻击 - 攻击者在受害者登录之前，先在他们身上植入一个已知的会话 id；轮换之后，那个被植入的 id 已经死了，只有那个新生成的 id 才携带认证状态。这份契约和 `Auth::login_id` / `Auth::login_using_id`
是一致的，所以就会话状态和监听者可观测性而言，2FA 登录和没有 2FA 的登录是无法区分的。

用 `TwoFactorChallengeMiddleware`，在 `AuthMiddleware` **之前**给每一个受保护的路由分组加门，这样一个待定的会话，就会被弹回质询页，而不是登录页：

```rust
use suprnova::{AuthMiddleware, TwoFactorChallengeMiddleware, group, get};

group!("/dashboard")
    .middleware(TwoFactorChallengeMiddleware::redirect_to("/two-factor-challenge"))
    .middleware(AuthMiddleware::redirect_to("/login"))
    .routes([
        get!("/", dashboard::index),
    ]);
```

质询页本身（渲染表单的那个 GET，调用 `complete_challenge` 的那个 POST）**不能**安装 `TwoFactorChallengeMiddleware` - 它自己就是那个目的地。这个 POST
处理程序通常还会提前检查 `TwoFactor::pending_user_id().is_some()`，这样一个陈旧的链接，就不会带着一个空会话闯进验证逻辑里。

`TwoFactor::cancel_challenge()` 会清空两个待定槽位，而不认证任何人 - 把它接到质询页上的一个“返回登录”链接上。

**恢复码回退。** `complete_challenge(code)` 会先尝试 TOTP 这条路径，再回退到消费一个恢复码，所以一个丢失了身份验证器的用户，仍然能进来。每一个恢复码都是一次性的。

**暴力破解联动。** 失败的质询验证码，会通过 `BruteForce::record_failed_attempt`，喂给按账户计的暴力破解计数器，和裸的 `TwoFactor::verify` 做的一样。一个对质询表单做暴力尝试的攻击者，会在越过配置好的阈值之后触发 `AccountLocked`。即便 `complete_challenge` 在内部会尝试 TOTP 和恢复码两条路径，一次糟糕的提交也只算作**一次**失败尝试 - 内部那两条静默校验路径不会碰暴力破解计数器，所以外层只会把这一次尝试恰好记一次数。

**锁定门控。** `complete_challenge` 会提前检查 `BruteForce::is_locked`，如果账户已经被锁定，就返回 `429 Too Many Requests` - 即便提交的验证码是对的。没有这道方法内部的门，一个已经触发了锁定的攻击者，仍然可以在下一次请求上靠提交正确的验证码进来：暴力破解计数器是按用户邮箱建键的，但 `verify` 自己并不会去查它。密码路径上的 `LoginThrottleMiddleware`，在路由层强制执行同样的约束；把它组合在质询 POST 路由前面也没问题 - 两道门都是幂等的。

**失败事件。** `complete_challenge` 会在验证码错误（或者账户被锁定）时派发
`TwoFactorChallengeFailed { user_id }`，和密码路径上的 `auth::Failed` 是分开的。那些盯着“用户试了 2FA 但失败了”的监听者，订阅这个新事件；那些盯着
“密码没能通过认证”的监听者，留在 `auth::Failed` 上。这两个表面被刻意分开，所以一次 2FA 输错，在审计流水线看来不会像是一次密码失败。

### 为什么 Suprnova 有所不同

框架 TOTP 的 `user_id` 是一个 `String`。固定的 `i64`、UUID 或 Magnetar 标识符类型会把可复用门面绑定到一种应用 schema。这个字符串边界让应用可以选择任意稳定标识符，代价是调用点的一次转换。

Magnetar 的集成 factor gate 与这个保留门面分离。这种分离保留了使用 `two_factor_credentials` 的应用的兼容性，但应用不应通过两个存储为同一账户注册。

## 记住我

`suprnova::auth_flows::remember_me` 为兼容性重导出 legacy `suprnova::auth::remember` 模块。

安装 Magnetar 后，普通的 `Auth::attempt(..., true)`、`Auth::issue_remember_cookie` 和 `SessionMiddleware` 水合会使用 Magnetar 的用途绑定 remember 凭据。Magnetar 存储 verifier digest、检查 auth epoch、在成功使用时轮换凭据、随用户会话吊销它们，并在不暴露凭据秘密值的情况下报告重放或凭据格式异常。

面向浏览器的 cookie 仍由框架拥有。它使用逻辑 `remember_me` 名称加密，遵循 `SESSION_COOKIE_PREFIX`，并在后端吊销前清除，因此存储失败不会让浏览器继续发送旧凭据。

未安装 Magnetar 引擎时，legacy 数据库行实现仍可用。新应用应初始化 Magnetar，并将 legacy 重导出视为过渡表面。

## 事件

九个事件贯穿这些流程触发，每一次安全状态迁移对应一个：

| 事件 | 由谁触发 | 携带什么 |
|---|---|---|
| `EmailVerified` | `EmailVerification::verify` 成功时 | `user_id: String` |
| `PasswordResetLinkSent` | `PasswordReset::send_link` 成功时 - 对不存在的邮箱防枚举地静默 | `user_id: String`、`email: String` |
| `PasswordResetCompleted` | `PasswordReset::complete` 成功时 | `user_id: String` |
| `AccountLocked` | `BruteForce::record_failed_attempt` 在从未锁定 → 锁定的迁移上 | `email: String`、`failed_attempts: u32` |
| `AccountUnlocked` | `BruteForce::unlock_account` 在一次真实的解锁发生时 | `email: String` |
| `TwoFactorEnrolled` | `TwoFactor::confirm` 成功时 | `user_id: String` |
| `TwoFactorChallenged` | `TwoFactor::complete_challenge` 把待定提升为已认证时 | `user_id: String` |
| `TwoFactorChallengeFailed` | `TwoFactor::complete_challenge` 拒绝了一个错误的验证码，或者拒绝了一个被锁定的账户 | `user_id: String` |
| `TwoFactorDisabled` | `TwoFactor::disable` 在一行被真正移除时 | `user_id: String` |

每一个事件都是 `Debug + Clone + 'static` 的，不携带任何敏感数据（没有明文令牌，没有 IP），并且使用字符串形式的标识符，好让监听者能跨任务边界序列化它们，而不会从用户存储后端泄露类型信息。

### 监听

通过标准的事件 API 订阅 - 和其他任何进程内事件用的是同一套表面：

```rust
use std::sync::Arc;
use suprnova::async_trait;
use suprnova::auth_flows::events::AccountLocked;
use suprnova::{EventFacade, FrameworkError, Listener};

pub struct PageOpsOnLockout;

#[async_trait]
impl Listener<AccountLocked> for PageOpsOnLockout {
    async fn handle(&self, event: &AccountLocked) -> Result<(), FrameworkError> {
        tracing::warn!(
            email = %event.email,
            failed_attempts = event.failed_attempts,
            "account locked - paging ops",
        );
        // ……Slack 通知、追加审计表，等等。
        Ok(())
    }
}

// 在 bootstrap.rs 里：
EventFacade::listen::<AccountLocked, _>(Arc::new(PageOpsOnLockout)).await;
```

监听者在 Tokio 的运行时上运行，并按注册顺序被分发。完整的表面参见
[事件](events.md) 一章。

## 测试

三个伪造实现覆盖了认证流程的表面，并且它们可以组合使用。

### `Mail::fake()`

安装一个进程本地的捕获传输层。在这个守卫的生存期内，每一次发送都会落进一个内存缓冲区，而不会真正发出去：

```rust
use suprnova::mail::Mail;

#[tokio::test]
async fn send_link_dispatches_email() {
    let fake = Mail::fake();
    // ……驱动这个流程……
    EmailVerification::send_link(&user, "https://app.example.com/verify")
        .await
        .unwrap();
    fake.assert_sent(|m| {
        m.to.iter().any(|a| a.email == "alice@example.com")
            && m.subject.contains("Verify")
    });
    fake.assert_sent_count(1);
}
```

`MailFake` 暴露了 `assert_sent`、`assert_not_sent`、`assert_sent_count`，再加上原始的 `captured()` 和 `count()` 访问器。当这个守卫被丢弃时，之前绑定的传输层会被恢复 - 那些把伪造实现和显式的传输层绑定交错使用的测试，不会泄漏状态。

### `EventFacade::fake()`

同样的形态，只是换成事件：

```rust
use suprnova::auth_flows::events::EmailVerified;
use suprnova::events::testing::assert_dispatched;
use suprnova::EventFacade;

#[tokio::test]
async fn verify_fires_email_verified_event() {
    let _guard = EventFacade::fake();
    // ……驱动这个流程……
    EmailVerification::verify(&token).await.unwrap();
    assert_dispatched::<EmailVerified>(|e| !e.user_id.is_empty());
}
```

这个伪造实现会记录已分发的事件，而不去调用监听者，所以一个会和外部服务对话的监听者，不会在测试期间触发。配套的 `assert_not_dispatched::<E>(pred)` 断言相反的情况；`dispatched_count::<E>(pred)` 返回原始计数，用于更细粒度的断言。

### 电子邮件验证和密码重置的集成测试

电子邮件验证测试会创建 `auth_flow_tokens`、注册 `UserProvider`、确立已认证令牌所有者、设置 `MAIL_FROM`，并在 `Mail::fake()` 之下驱动门面。

密码重置测试会安装 `MagnetarPasswordAuthEngine` 测试适配器，并断言签发、不消费的检查、原子完成、会话吊销和一次性行为。

规范源码示例为：

- `framework/tests/email_verify.rs`：绑定 actor 的验证和一次性令牌。
- `framework/tests/password_reset.rs`：Magnetar 委托和完成结果。
- `framework/tests/magnetar_default_engine.rs`：真实默认引擎设置。
- `framework/tests/brute_force.rs`：锁定生命周期。
- `framework/tests/two_factor_challenge_flow.rs`：保留的框架 TOTP 质询流程。
- `framework/tests/magnetar_remember_middleware.rs`：remember 轮换和双会话绑定。

进程全局 Magnetar 安装刻意是一次性的。需要不同引擎的测试应放在单独的集成测试二进制文件中，或为整个二进制文件只安装一次测试适配器。

## 参考

| 符号 | 用途 |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`、`resend`、`check` 和绑定 actor 的 `verify`；`verify` 返回用户 ID。 |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | `new()` 用于 403 JSON，`redirect_to(path)` 用于浏览器或 Inertia 重定向。 |
| `suprnova::auth_flows::PasswordReset` | 优先使用 Magnetar 进行重置，并通过框架 `auth_flow_tokens` 为已验证账户提供 `UserProvider` 回退。 |
| `suprnova::MustVerifyEmail` | 框架验证门面的应用用户契约。 |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | 框架验证令牌的 SeaORM 表定义。 |
| `suprnova::auth_flows::BruteForce` | Magnetar 支撑的账户锁定门面。 |
| `suprnova::auth_flows::LoginThrottleMiddleware` | 账户锁定时，在登录处理程序之前返回 429 的 HTTP 中间件。 |
| `suprnova::auth_flows::TwoFactor` | 保留的框架 TOTP 注册、验证、恢复和质询门面。 |
| `suprnova::auth_flows::TwoFactorUser` | 框架 TOTP 门面的应用用户桥接。 |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | 等待框架 TOTP 质询的会话的门。 |
| `suprnova::auth_flows::remember_me` | 对 legacy 框架 remember 模块的兼容性重导出。 |
| `suprnova::MagnetarConfig` / `suprnova::init_magnetar` | 默认 Magnetar 引擎配置和一次性安装。 |
| `suprnova::auth_flows::events::*` | 认证生命周期事件。 |

## 下一步

- [认证](authentication.md) - 认证守卫、提供者、`Auth` 门面、`AuthMiddleware`。
- [邮件](mail.md) - `send_link` 调用借以派发的那个传输层。
- [事件](events.md) - 为这九个认证流程事件注册监听者。
- [速率限制](rate-limiting.md) - 把 `RateLimitMiddleware::ip_based` 和
  `LoginThrottleMiddleware` 搭配起来，做分层防御。
- [会话](session.md) - `start_challenge` / `complete_challenge` 在轮换会话
  id 时会碰到什么。
