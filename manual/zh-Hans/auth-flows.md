# 认证流程

`suprnova::auth_flows` 是叠在[会话认证](authentication.md)之上的生命周期层。
`auth::*` 回答的是“这次请求是谁”，`auth_flows::*` 回答的则是这个问题周围的一切 -
证明这个电子邮件地址是真的，在密码丢失时把它找回来，抵御针对它的撞库攻击，并用第二因素保护它。五个流程打包在同一个命名空间下：

- `EmailVerification` - 铸造、检查并消费一次性的验证令牌；`send_link` / `resend`
  通过 [`Mail`](mail.md) 门面派发验证邮件，`verify` 则通过配置好的用户提供者，把用户标记为已验证。
- `PasswordReset` - 防枚举的 `send_link`、不消费令牌的 `check`，以及 `complete`。
  `complete` 会通过配置好的用户提供者轮换密码，吊销这个用户的每一个会话和记住我行，并发送一条 `PasswordChangedMail` 安全通知。
- `BruteForce` + `LoginThrottleMiddleware` - 由 torii 支撑的锁定状态，加上一个会在登录处理程序被调用之前，就用 `429 Too Many Requests` 短路的 HTTP 中间件。
- `TwoFactor` - TOTP 绑定、确认、验证、恢复码、密钥轮换、用第二因素给密码登录加门的那整套质询流程，以及粒度为 30 秒时间步的重放保护。
- `remember_me` - 为了命名空间的凝聚性，对 `crate::auth::remember` 的重导出（数据库行 + bcrypt + 一次性轮换式的持久 cookie）。

同一个命名空间下还带着两个路由门中间件：

- `EnsureEmailVerifiedMiddleware` - 组合在 `AuthMiddleware` 之后，根据
  `email_verified_at` 给路由加门。
- `TwoFactorChallengeMiddleware` - 组合在 `AuthMiddleware` 之前，把一个带着待定 2FA 质询的会话，弹回质询表单，而不是登录页。

每一条事务性消息，都是通过 [`Mail`](mail.md) 门面投递的。torii 那个可选的
`mailer` feature，在 `framework/Cargo.toml` 里被故意禁用了：在 torii 内部再跑一套邮件栈，会拆分遥测数据、让传输配置的表面翻倍，还会强迫应用去接两个
“发件人”地址。

### 状态住在哪里

电子邮件验证和密码重置是**与提供者无关**的。验证和重置令牌，住在框架自己的
`auth_flow_tokens` 表里（一次性、经过 SHA-256 哈希）；用户查找 + 变更，则经过应用注册的那个 [`UserProvider`](authentication.md) - 和 `Auth::user` 所解析的是同一个提供者。这两个流程都不需要初始化任何全局认证实例：一个刚刚脚手架生成的应用，已经绑定好了 `EloquentUserProvider<User>`，而这就是 `EmailVerification`
和 `PasswordReset` 需要的一切。

对于那些真正依赖它的流程，torii 仍然拥有安全状态 - 按账户的暴力破解锁定计数器、OAuth / passkey / WebAuthn 握手，以及会话池。Suprnova 拥有跨越所有流程的横切关注点 - 出站邮件、事件分发、2FA 的 TOTP 表、记住我 cookie，以及 HTTP 中间件。应用代码永远只接触 `suprnova::auth_flows::*`。Laravel 把对应的表面折进了 Fortify；Suprnova 则把模型 trait（`MustVerifyEmail` / `CanResetPassword`）和令牌存储留在框架里，让这些流程能对任何用户后端工作。

## 跨流程的失败语义

每一个门面都遵循同一条排序规则：持久的状态变更先提交，通知类的副作用后触发。变更之后发生的一次监听者 panic、一次短暂的邮件传输故障，或者一次分发器错误，都不能把这次变更回滚。

- `EmailVerification::verify` 会先消费这个令牌、通过提供者把用户标记为已验证，然后才触发 `EmailVerified`。
- `PasswordReset::complete` 会先消费这个令牌、通过提供者轮换密码，然后吊销这个用户的每一个会话和记住我行（失败时只记日志，不会向上暴露），然后以发后不理的方式派发 `PasswordChangedMail`，最后触发 `PasswordResetCompleted`。
- `BruteForce::unlock_account` 会先提交这次解锁，然后才触发 `AccountUnlocked`。
- `TwoFactor::confirm` 会先盖上 `confirmed_at` 的戳，然后才触发
  `TwoFactorEnrolled`；`TwoFactor::disable` 会先删除这一行，然后才触发
  `TwoFactorDisabled`；`TwoFactor::complete_challenge` 会先把待定提升为已认证，然后才派发标准的 `auth::Login` + `auth::Authenticated` 这一对，紧接着是 `TwoFactorChallenged`。

一个需要持久性的监听者，应该自己缓冲这份工作（从监听者函数体里排一个任务进队列）；这个门面本身从不重试。

## 应用启动

电子邮件验证和密码重置是由提供者支撑的，**不需要** torii。暴力破解防护和 2FA
仍然需要 torii。您用到哪个流程，就接哪个流程要的线 - 它们彼此独立。

### 电子邮件验证 + 密码重置

三件事，一个脚手架生成的应用早就都有了：

1. **一个实现了认证流程表面的用户提供者。** 在 `bootstrap.rs::register()` 里，把 `EloquentUserProvider<User>`（和 `Auth::user` 所解析的是同一个提供者）注册为 `dyn UserProvider` 绑定。两个门面都会在内部解析这个当前活跃的提供者；调用点不需要传任何实例。

   ```rust
   use suprnova::{bind, EloquentUserProvider};
   use suprnova::auth::UserProvider;
   use crate::models::users::User;

   bind!(dyn UserProvider, EloquentUserProvider::<User>::new());
   ```

2. **您 `User` 上的这两个模型 trait。** `EloquentUserProvider<User>` 只有在
   `User` 同时实现了 `MustVerifyEmail` 和 `CanResetPassword` 时，才会实现认证流程的那些方法（`retrieve_by_email` / `mark_email_verified` /
   `set_password` / `is_email_verified`） - 这两个是 Suprnova 对 Laravel
   `MustVerifyEmail` / `CanResetPassword` 契约的类似物：

   ```rust
   use chrono::{DateTime, Utc};
   use suprnova::{Authenticatable, CanResetPassword, MustVerifyEmail};

   impl MustVerifyEmail for User {
       fn email(&self) -> &str {
           &self.email
       }
       fn email_verified_at(&self) -> Option<DateTime<Utc>> {
           self.email_verified_at
       }
       fn set_email_verified_at(&mut self, v: Option<DateTime<Utc>>) {
           self.email_verified_at = v;
       }
       fn name(&self) -> Option<&str> {
           Some(&self.name)
       }
   }

   impl CanResetPassword for User {
       fn email_for_reset(&self) -> &str {
           &self.email
       }
       fn set_password_hash(&mut self, hash: &str) {
           // 这个值送到时已经哈希过了 - 原样存储它。
           self.password = hash.to_string();
       }
   }
   ```

   `is_email_verified()` 有一个跟踪这个时间戳的默认实现（`email_verified_at().is_some()`），`name()` 默认是 `None` - 覆盖它，好在邮件里用名字问候用户。

3. **您迁移器里的两个列 / 表。** `users` 表需要一个可为空的 `email_verified_at`
   时间戳（提供者会在 `is_email_verified` 里读它，在 `mark_email_verified`
   里给它盖戳），而框架那张一次性的 `auth_flow_tokens` 表，存放验证 / 重置令牌。框架自带这张令牌表的 `CREATE`；把它列进您的迁移器里：

   ```rust
   use sea_orm_migration::prelude::*;

   #[async_trait::async_trait]
   impl MigrationTrait for AuthFlowTokens {
       async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .create_table(
                   suprnova::auth_flows::token_store::create_auth_flow_tokens_table(),
               )
               .await
       }

       async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
           manager
               .drop_table(Table::drop().table(Alias::new("auth_flow_tokens")).to_owned())
               .await
       }
   }
   ```

   在您自己的列迁移里，给 `users` 加上 `email_verified_at`（一个可为空的
   `timestamp_with_time_zone`）；`NULL` 意味着未验证，所以既有的行能正确地回填。

令牌是一次性的，落地时经过 SHA-256 哈希 - 一次数据库转储，永远不会产出一个可用的明文令牌。默认的 TTL，电子邮件验证是 **24 小时**，密码重置是 **15 分钟**。

### 暴力破解 + 2FA：给 torii 接线

`BruteForce` / `LoginThrottleMiddleware` 和 `TwoFactor` 都是由 torii 支撑的 - 它们需要那个全局的 torii 实例，在 `bootstrap.rs::register()` 里、`DB::init` 之后被初始化。（OAuth、passkey 和 WebAuthn 握手走的是同一个实例 - 参见[认证](authentication.md)。）

```rust
use suprnova::torii_integration::{init_torii, ToriiConfig};
use suprnova::DB;

pub async fn register() -> Result<(), suprnova::FrameworkError> {
    DB::init().await?;

    let conn = DB::connection()?.inner().clone();
    init_torii(ToriiConfig::from_sea_orm(conn)).await?;

    Ok(())
}
```

`init_torii` 是幂等的。这层 `OnceLock` 防护意味着第二次调用是一次空操作，所以那些按每个 fixture 都重新进入一次 `register()` 的测试装置，不会重复迁移。测试时，换上 `ToriiConfig::sqlite_in_memory()` - 它会拉起一个共享缓存的内存数据库，能在多个 runtime 之间存活：

```rust
let config = ToriiConfig::sqlite_in_memory()
    .await?
    .apply_migrations(true);
init_torii(config).await?;
```

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
            // 为 TOTP 重放保护添加 `last_used_timestep`。
            Box::new(suprnova::auth_flows::two_factor::migration_replay::Migration),
        ]
    }
}
```

两者对一个已经应用过的数据库都是幂等的（v1 用的是
`CREATE TABLE IF NOT EXISTS`；v2 是一次加列）。对一个已经有这套表结构的生产数据库重新跑一次 `suprnova migrate`，是一次空操作。

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
| `verify` | `verify(token: &str) -> Result<String>` | 一次性：消费这个令牌，把用户标记为已验证，返回用户 id。 |

```rust
use suprnova::auth_flows::EmailVerification;

// 在一次新注册之后，手上拿着刚创建出来的用户：
EmailVerification::send_link(&user, "https://app.example.com/verify-email").await?;

// 可选的落地页检查 - 不消费令牌，所以刷新页面
// 不会烧掉这个令牌。
let valid: bool = EmailVerification::check(&token_str).await?;

// 这个点击跳转处理程序会消费令牌并给用户盖戳，
// 返回这个已验证用户的 id。
let user_id: String = EmailVerification::verify(&token_str).await?;
```

`verify` 成功时会触发 `EmailVerified` - 监听者是解锁额外功能（欢迎邮件、默认关注、“完善您的资料”这类行动号召）的正确位置，不需要把它们和验证处理程序耦合在一起。这个事件携带的是提供者的那个用户 id。

### 重新发送端点（防枚举）

`resend` 只接受邮箱 - 这个门面会通过当前活跃的提供者去查找用户，当有账户在档时，就铸造一个令牌并发出邮件；一个未知的邮箱是一次静默的空操作，仍然返回
`Ok(())`。处理程序自己从不针对存在性分支，所以一个试探性的调用方，没法区分
“已发送”和“没有这个账户”：

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

这个点击跳转处理程序，会从查询字符串里取出令牌，并调用 `verify`：

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

这个处理程序不需要自己去查找用户 - `verify` 会消费令牌，通过提供者把用户标记为已验证，返回用户 id，并触发 `EmailVerified`。一次性：对同一个令牌的第二次 `verify` 会返回一个错误。

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

`PasswordReset` 有三个操作：

| 方法 | 签名 | 说明 |
|---|---|---|
| `send_link` | `send_link(email: &str, base_url: &str) -> Result<()>` | 防枚举：按邮箱查找用户；一个未知的地址会静默返回 `Ok(())`。 |
| `check` | `check(token: &str) -> Result<bool>` | 不消费令牌 - 在渲染新密码表单之前确认这个令牌。 |
| `complete` | `complete(token: &str, new_password: &str) -> Result<String>` | 一次性：消费这个令牌，轮换密码，吊销会话 + 记住我，发送变更通知，返回用户 id。 |

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

`complete` 会在把 `new_password` 交给提供者之前先哈希它 - 传明文，不要传一个预先哈希过的值。一个空的 / 全是空白字符的密码，会提前被一个 `400` 拒绝。

### 防枚举

`send_link` 的结构，让响应的形状永远不会泄露一个邮箱地址是否有账户：

- 它永远返回 `Ok(())`。当这个邮箱不存在时，没有令牌被铸造，没有邮件被派发，
  `PasswordResetLinkSent` 事件也不会触发 - 但这份缺席同样不会通过返回类型暴露出来，所以一个调用方（以及一个网络观察者）没法区分“没有这个账户”和
  “链接已发送”。
- 这个自用（dogfood）控制器，把 `send_link` 和一个固定的 200 响应正文搭配在一起，所以一个试探性的调用方，没法通过状态码、响应正文或者响应耗时来区分。

### `complete` 的副作用

`complete` 会按顺序运行四个步骤：

1. 消费这个令牌（一次性），并通过配置好的提供者轮换密码哈希（唯一一个能让这次调用失败的步骤）。
2. 通过 `crate::session::destroy_all_for_user`，吊销这个用户的每一个会话行（尽力而为：失败时 `tracing::warn!`）。
3. 通过 `crate::auth::remember::revoke_all_for_user`，吊销每一个记住我行（尽力而为）。
4. 以发后不理的方式派发 `PasswordChangedMail`，然后触发
   `PasswordResetCompleted`。

一个被偷走的会话，和一个被截获的记住我 cookie，都不能活得比它们所依赖的那份凭据更久。这些吊销会在每一次成功的重置上发生，不只是用户自己发起的那些，所以一次安全团队强制发起的重置，也会把一个正在活动的攻击者踢出去。

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
- `Retry-After` 请求头 - 秒数，通过 `LockoutStatus::retry_after_seconds`，从这次锁定的 `locked_until` 算出来。如果这个时间戳不知何故缺失了，就回退到 `900`（15 分钟 - torii 的默认锁定周期）。
- 正文：`"Account locked due to too many failed login attempts. Try
  again later."`

### 后端错误时失败开放

如果 `get_lockout_status` 返回一个 `Err`（一次短暂的数据库故障），这个中间件会放行这次请求。下游的登录处理程序接下来会自己发起这次调用，并可以决定要失败关闭还是失败开放。这个中间件在可用性这一侧犯错：只要认证数据库有一点风吹草动就拖垮登录端点，比让处理程序直接自己做这次调用更糟。

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

torii 的 `BruteForceProtectionConfig` 默认是**锁定前 5 次失败尝试**和**15
分钟的锁定周期**。这些就是 `init_torii` 今天接好的线；配置逐应用的值，需要伸手进 torii 自己的配置表面，Suprnova 的 `ToriiConfig` 构建器并没有把它暴露出来。这些默认值是故意做得保守的 - 在决定放宽它们之前，先想清楚“打错五次密码就把我锁 15 分钟”这件事本身是不是可以接受。

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

`user_id` 是那个不透明的存储键 - 通常是 `torii::UserId.as_str()`，但任何稳定的、逐用户的标识符都能用。2FA 表以它为索引；它和您的用户表之间没有外键。

`email` 会被折进 `otpauth://` URL 的 `account_name` 段里，好让身份验证器应用把这一行渲染成一个人类可读的标签（比如“MyCorp (alice@example.com)”）。

一个常见的模式，是用一个小小的 newtype 包一层您的用户模型：

```rust
use suprnova::auth_flows::TwoFactorUser;
use suprnova::torii_integration::User as ToriiUser;

struct AppUser2FA<'a> { user: &'a ToriiUser }

impl<'a> TwoFactorUser for AppUser2FA<'a> {
    fn user_id(&self) -> &str { self.user.id.as_str() }
    fn email(&self)   -> &str { &self.user.email }
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

2FA 的 `user_id` 被故意设计成一个 `String`。如果它被定成 `i64`、`Uuid` 或者
`torii::UserId` 类型，2FA 表就会被永久地绑死在框架最先选定的那个形状上 - 那些用不同形状存储用户的应用（UUID 对比自增整数，或者压根不用 torii、却想用
2FA 模块的应用）就会被排除在外。一个字符串形式的 `user_id`，让每个应用都能挑一个自己喜欢的、稳定的逐用户标识符；代价是在调用点多一次 `.to_string()`。
Laravel 的 Fortify，把对应的列绑死在 Eloquent 的 `User::id` 上 - Suprnova
把它解耦开，让 `TwoFactor` 成为一个可复用的生命周期原语，而不是一个 User
形状的附属品。

## 记住我

`suprnova::auth_flows::remember_me` 重导出了 `suprnova::auth::remember` - 这个持久 cookie 模块早就随会话认证一起发布了。这次重导出纯粹是组织上的：任何认证流程形状的东西，都住在 `auth_flows::*` 下面，即便它的实现早于这个命名空间就已经存在。

已发布的设计：

- **数据库行 + bcrypt 哈希** - 每一个签发出去的令牌，在 `remember_tokens`
  表里都有一行，只存储 bcrypt 哈希，永远不存明文。一次数据库转储，没法产出能重新认证的凭据。
- **一次性轮换** - 一次成功的验证会 DELETE 掉匹配的那一行，并签发一个新的。一个被截获的 cookie 没法被重用；如果攻击者和受害者竞相使用它，落后的那一个会发现这一行已经不见了，认证失败。
- **吊销** - `revoke_all_for_user` 用一次 DELETE 抹掉一个用户的每一行。
  `Auth::logout` 会串联上这一步，让一次真正的登出，确实清空持久状态；
  `PasswordReset::complete` 也做同样的事，让一次密码重置，让每一个既有的持久 cookie 都失效。
- **清理** - `prune_expired` 按计划清理过期的行。

实际上，框架的会话中间件做了大部分重活；典型的应用不需要直接调用
`remember_me` 这个模块。[认证](authentication.md) 文档覆盖了面向用户的那个表面 - `Auth::login` 上的 `remember` 标志、cookie 名字，以及生存期旋钮。

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

### 电子邮件验证 + 密码重置的集成测试

验证 / 重置测试不需要 torii - 在一个内存数据库上准备好 `auth_flow_tokens`
表，注册一个提供者，设置 `MAIL_FROM`，并在 `Mail::fake()` 之下驱动这个门面。框架自己的测试，会直接从 `create_auth_flow_tokens_table()` 建出这张表：

```rust
use sea_orm::ConnectionTrait;
use suprnova::auth_flows::token_store::create_auth_flow_tokens_table;
use suprnova::mail::Mail;
use suprnova::testing::TestDatabase;

#[tokio::test]
#[serial_test::serial]
async fn send_link_mails_a_token_link() {
    let db = TestDatabase::sqlite_memory().await.unwrap();
    let conn = db.conn();
    let stmt = create_auth_flow_tokens_table();
    conn.execute(conn.get_database_backend().build(&stmt))
        .await
        .unwrap();

    // 这些门面会读取 MAIL_FROM（失败关闭）；为测试设置它。
    // SAFETY：由 `#[serial]` 序列化 - 没有并行的观察者。
    unsafe { std::env::set_var("MAIL_FROM", "test-mailer@example.com"); }

    let fake = Mail::fake();
    // ……驱动 EmailVerification::send_link(&user, base) ……
    fake.assert_sent_to("ada@example.com");
}
```

由提供者支撑的那些路径（`resend` / `verify` / `complete`），还需要额外注册一个 `dyn UserProvider` 绑定，好让查找 + 变更能解析出来 - 参见
`framework/tests/email_verify.rs` 和 `framework/tests/password_reset.rs`。

### 面向暴力破解 + 2FA 测试的 `ToriiConfig::sqlite_in_memory()`

暴力破解和 2FA 测试，会在一个内存 SQLite 数据库上拉起一个新的 torii。
`framework/tests/` 里的示例测试文件，用一个共享 runtime + `once_cell::sync::Lazy<()>`
的模式，把这份代价摊薄到各个测试上，再加上 `#[serial]`，让那些交错使用
`Mail::fake()` 的测试之间，进程全局的邮件传输层保持稳定：

```rust
use once_cell::sync::Lazy;
use serial_test::serial;
use tokio::runtime::Runtime;
use suprnova::torii_integration::{init_torii, ToriiConfig};

static RT: Lazy<Runtime> = Lazy::new(|| Runtime::new().expect("tokio runtime"));

static SETUP: Lazy<()> = Lazy::new(|| {
    RT.block_on(async {
        let config = ToriiConfig::sqlite_in_memory()
            .await
            .expect("sqlite in-memory connection")
            .apply_migrations(true);
        init_torii(config).await.expect("init_torii");
    });
});

#[test]
#[serial]
fn my_test() {
    Lazy::force(&SETUP);
    RT.block_on(async {
        // ……在这里用 Mail::fake() / EventFacade::fake() ……
    });
}
```

标准示例 - 编写您自己的测试时，可以照抄这些：

- `framework/tests/email_verify.rs` - 验证令牌的往返、`send_link` 的末尾斜杠裁剪、针对主题/HTML 的 `Mail::fake()` 断言。
- `framework/tests/password_reset.rs` - 带新密码认证的重置往返、对未知邮箱的防枚举、`complete` 拒绝被重用的令牌。
- `framework/tests/brute_force.rs` - 完整的锁定生命周期、`AccountLocked`
  每次迁移触发一次、`unlock_account` 返回 `was_locked`。
- `framework/tests/two_factor.rs` - 完整的绑定 → 确认 → 验证，用一个从
  otpauth URL 算出来的真实 TOTP 验证码、恢复码的一次性、重新绑定会覆盖密钥、两次并发验证之间的重放拒绝。
- `framework/tests/two_factor_challenge_flow.rs` - 带会话轮换、记住我重新签发和事件分发的端到端质询流程。
- `framework/tests/email_verified_middleware.rs` 和
  `two_factor_challenge_middleware.rs` - 中间件的响应形态（403 JSON 对比
  302 对比 409 + X-Inertia-Location）。

## 参考

| 符号 | 用途 |
|---|---|
| `suprnova::auth_flows::EmailVerification` | `send_link`、`resend`、`check`、`verify` - 由提供者支撑；`verify` 返回用户 id。 |
| `suprnova::auth_flows::EnsureEmailVerifiedMiddleware` | `new()` 对应 403 JSON，`redirect_to(path)` 对应 302 / 409 + X-Inertia-Location。检查配置好的提供者的 `is_email_verified`（失败关闭）。 |
| `suprnova::auth_flows::PasswordReset` | `send_link`、`check`、`complete` - 由提供者支撑；`complete` 返回用户 id。 |
| `suprnova::MustVerifyEmail` / `suprnova::CanResetPassword` | `EloquentUserProvider` 背后的用户要实现的模型 trait，让验证 / 重置门面能读它的邮箱 + 写它的验证时间戳 / 密码哈希。 |
| `suprnova::auth_flows::token_store::create_auth_flow_tokens_table` | `auth_flow_tokens` 的 SeaORM `CREATE TABLE` - 列进您的迁移器里。 |
| `suprnova::auth_flows::BruteForce` | `record_failed_attempt`、`reset_attempts`、`get_lockout_status`、`is_locked`、`unlock_account`。 |
| `suprnova::auth_flows::LoginThrottleMiddleware` | 一个在被针对的账户被锁定时，会在处理程序之前就 429 的 HTTP 中间件。 |
| `suprnova::auth_flows::TwoFactor` | `enroll`、`re_enroll`、`confirm`、`verify`、`consume_recovery_code`、`regenerate_recovery_codes`、`is_enabled`、`is_enabled_by_id`、`start_challenge`、`pending_user_id`、`cancel_challenge`、`complete_challenge`、`disable`。 |
| `suprnova::auth_flows::TwoFactorUser` | 把应用的用户模型桥接到 2FA 门面的 trait。 |
| `suprnova::auth_flows::EnrollmentResponse` | `TwoFactor::enroll` 的返回值 - `otpauth_url`、`qr_code_svg`、`recovery_codes`。 |
| `suprnova::auth_flows::TwoFactorChallengeMiddleware` | `new()` 对应 403 JSON，`redirect_to(path)` 对应 302 / 409 + X-Inertia-Location。组合在 `AuthMiddleware` 之前。 |
| `suprnova::auth_flows::two_factor::migration::Migration` | `two_factor_credentials` 的 SeaORM 迁移。列进您的 `Migrator::migrations()` 里。 |
| `suprnova::auth_flows::two_factor::migration_replay::Migration` | 为 `last_used_timestep` 加列（TOTP 重放保护）。列在建表迁移之后。 |
| `suprnova::auth_flows::remember_me` | 对 `suprnova::auth::remember` 的重导出。 |
| `suprnova::auth_flows::events::*` | 九个事件 - 参见[事件](#事件)。 |
| `suprnova::auth_flows::EmailVerificationMail` | 事务性 Mailable。主题 `"Verify your email for {APP_NAME}"`。 |
| `suprnova::auth_flows::PasswordResetMail` | 事务性 Mailable。主题 `"Reset your {APP_NAME} password"`。 |
| `suprnova::auth_flows::PasswordChangedMail` | 安全通知 Mailable。主题 `"Your {APP_NAME} password was changed"`。 |
| `suprnova::torii_integration::ToriiConfig` | torii 的启动配置。生产环境用 `from_sea_orm(conn)`，测试用 `sqlite_in_memory()`。 |
| `suprnova::torii_integration::init_torii` | 幂等的全局初始化。从 `bootstrap.rs::register()` 里调用一次。 |

## 下一步

- [认证](authentication.md) - 认证守卫、提供者、`Auth` 门面、`AuthMiddleware`。
- [邮件](mail.md) - `send_link` 调用借以派发的那个传输层。
- [事件](events.md) - 为这九个认证流程事件注册监听者。
- [速率限制](rate-limiting.md) - 把 `RateLimitMiddleware::ip_based` 和
  `LoginThrottleMiddleware` 搭配起来，做分层防御。
- [会话](session.md) - `start_challenge` / `complete_challenge` 在轮换会话
  id 时会碰到什么。
