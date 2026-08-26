# OAuth、Apple 和魔法链接登录

Suprnova 通过框架自有的 `Auth` 门面，提供 OAuth、使用 Apple 登录以及无密码的魔法链接。Magnetar 为该门面提供凭据、认证握手、身份、因子门控和会话引擎。

公开入口点是：

- `Auth::oauth(provider)` 用于 OAuth 和 Apple。
- `Auth::magic_link()` 用于无密码的邮箱登录。

Suprnova 不会为这些流程安装路由。应用提供简短的起始和回调处理程序，并决定如何投递魔法链接邮件。

## 使用 OAuth 初始化 Magnetar

在同一个 `MagnetarConfig` 上配置 OAuth，该配置会初始化密码、通行密钥、会话、锁定和双因素服务。提供方注册表会与这些服务一起原子发布：如果任何服务无法构建，则它们都不会变为可见。

```rust,no_run
use std::sync::Arc;

use suprnova::{
    AbuseLimiter, App, AutoLinkPolicy, DB, DatabaseConnection, EndpointOverrides,
    FrameworkAbuseLimiter, GoogleOAuthProvider, GoogleProviderConfig, MagnetarConfig,
    MagnetarOAuthHostConfig, MagnetarOAuthProviderConfig, OAuthAuthorizationConfig,
    OAuthHttpTransport, PasskeyConfig, RateLimiterDriver, ReqwestOAuthTransport,
    RevocationTransport, SecretString, init_magnetar,
};

fn auth_config(
    database: DatabaseConnection,
    transport: Arc<dyn OAuthHttpTransport>,
    revocation: Arc<dyn RevocationTransport>,
    limiter: Arc<dyn AbuseLimiter>,
) -> MagnetarConfig {
    let provider = Arc::new(GoogleOAuthProvider::new(
        GoogleProviderConfig {
            client_id: "google-client".to_owned(),
            client_secret: SecretString::from("google-secret".to_owned()),
            redirect_uri: Some("https://app.example.com/auth/google/callback".to_owned()),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
            endpoints: EndpointOverrides::default(),
        },
        revocation,
    ));
    let oauth = MagnetarOAuthHostConfig::new(
        vec![MagnetarOAuthProviderConfig {
            provider,
            redirect_uri: "https://app.example.com/auth/google/callback".to_owned(),
            scopes: vec!["openid".to_owned(), "email".to_owned()],
        }],
        transport,
        limiter,
        OAuthAuthorizationConfig::default(),
        AutoLinkPolicy::default(),
    )
    .expect("valid OAuth host configuration");

    MagnetarConfig::from_sea_orm(database)
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_owned(),
            rp_origin: "https://app.example.com".to_owned(),
        })
        .oauth(oauth)
}

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let transport = Arc::new(ReqwestOAuthTransport::try_default()?);
    let limiter = Arc::new(FrameworkAbuseLimiter::new(
        App::resolve_make::<dyn RateLimiterDriver>()?,
    ));
    init_magnetar(auth_config(
        database.inner().clone(),
        transport.clone(),
        transport,
        limiter,
    ))
    .await
}
```

该框架重新导出 `OAuthProvider` 契约、五个第一方提供方和配置类型，以及实现自定义提供方所需的所有类型。`ReqwestOAuthTransport` 提供生产用的令牌、userinfo 和撤销 I/O。`FrameworkAbuseLimiter` 使用应用程序配置的 `RateLimiterDriver`。应用程序既不需要直接依赖 `suprnova-magnetar`，也不需要手写传输和限制器适配器。

当启用 `apply_migrations` 时，`MagnetarConfig` 会创建其架构，这是默认设置。仅当部署会单独准备相同架构时，才使用 `.apply_migrations(false)`。第二次初始化会返回错误，而不是替换任何已安装的引擎。

### 保留现有用户和会话体系

应用可以只使用 Magnetar 处理 OAuth 认证握手和提供方证明，而不让 Magnetar
接管密码、passkey、框架会话或记住我状态。构建相同的
`MagnetarOAuthHostConfig`，然后通过仅 OAuth 初始化器进行安装：

```rust,no_run
use suprnova::{
    MagnetarOAuthOnlyConfig, init_magnetar_oauth_only,
};

let database = DB::connection()?;
init_magnetar_oauth_only(
    MagnetarOAuthOnlyConfig::from_sea_orm(
        database.inner().clone(),
        oauth,
    ),
)
.await?;
```

照常使用 `Auth::oauth(provider).begin()` 启动认证握手。在回调中调用
`verify_oauth_identity(code, state)`，把已验证的提供方 subject 映射到应用自己的用户表，然后通过 `Auth::login` 建立现有的框架会话。此模式下不要调用
`complete`：`complete` 会应用 Magnetar 默认的账户和会话映射，而仅 OAuth
初始化的目的正是把这些决定留给应用。

仅 OAuth 初始化与完整的默认初始化是二选一的方案。第二个初始化器会失败，而不会混合两个会话权威来源。

### GitHub 提供方要求

GitHub 的 REST 用户端点需要 `User-Agent`；社区提供方会通过 `OAuthProvider::userinfo_headers` 添加它，以及所需的任何媒体类型 `Accept` 值。Suprnova 会单独添加 bearer `Authorization` 标头，并拒绝提供方覆盖它的尝试。

只有当用户将电子邮件公开时，GitHub 的 `/user` 响应才会包含电子邮件。已验证的主地址需要第二次 `/user/emails` 请求，而 `resolve_identity` 会刻意不执行 I/O，并接收一个 userinfo 响应。GitHub 提供方可以返回 `email: None` 并使用 Suprnova 的电子邮件补全握手，或者将 `userinfo_endpoint` 指向一个主机适配器，由它把 `/user` 与已验证的主电子邮件结合起来。不要将未验证或仅仅公开的地址视为账户所有权。

## 绑定会话

OAuth begin 需要 `SessionMiddleware`。Magnetar 会把认证握手绑定到发起方框架会话的摘要，因此回调不能被移动到另一个浏览器会话。

成功的密码、魔法链接、passkey 和 OAuth 登录会轮换框架会话 ID 与 CSRF 令牌，记录应用用户 ID，并存储不透明的 Magnetar Web 绑定。记住我功能的 hydration 会同时轮换 Magnetar 凭据和框架会话绑定。

## 开始 OAuth 流程

在提供者的起始处理程序中使用 `begin`：

```rust,ignore
use suprnova::Auth;

let kickoff = Auth::oauth("google").begin().await?;
// 返回一个重定向到 kickoff.authorization_url 的 HTTP 响应。
```

返回的 `OAuthKickoff` 包含：

- `authorization_url`，要发送给浏览器的 URL。
- `state`，绑定到发起会话的一次性选择器。

Magnetar 负责 state 生成、PKCE 策略、认证握手持久化、提供者交换、身份验证和滥用限制。宿主控制器负责 HTTP 重定向和回调路由。

## 验证或完成回调

回调有两个入口点：

| 方法 | 结果 | 副作用 |
|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity` | 验证提供者证明，并返回提供者、subject、已验证邮箱和显示名称，但不会创建应用会话。 |
| `complete(code, state)` | `(User, Session)` | 通过已安装的宿主引擎解析身份，应用账户链接策略和因子门控，轮换框架会话，并返回框架自有的用户和 Magnetar 会话值。 |

```rust,ignore
let identity = Auth::oauth("google")
    .verify_oauth_identity(&code, &state)
    .await?;

let (user, session) = Auth::oauth("google")
    .complete(&code, &state)
    .await?;
```

`OAuthIdentity.email` 仅在提供者提供了已验证邮箱时存在。请将提供者和 subject 持久化为稳定的外部身份。邮箱不是稳定的提供者标识符。

## 账户链接策略

OAuth 完成不会把拥有一个未经验证的邮箱字符串视为调用方拥有现有应用账户的证明。

完成结果可能要求先做更多工作，而不是签发会话：

- **需要完成邮箱**：当提供者身份需要单独的已验证邮箱认证握手时，返回 HTTP 409。
- **需要显式链接**：当现有的已验证账户必须授权链接时，返回 HTTP 409。
- **需要因子**：当账户策略要求在签发会话前提供第二个因子时，返回 HTTP 401。

成功完成已验证邮箱且赢得首次邮箱证明边界时，会以原子方式收回一个被抢注的未验证账户。事务会推进认证 epoch，移除临时凭据，吊销旧会话和记住我凭据，并附加已验证的提供者账户。已验证账户绝不会仅凭邮箱自动关联。

## 使用 Apple 登录

Apple 使用相同的 `Auth::oauth("apple")` 门面，但其回调通常使用 `response_mode=form_post`。请把回调注册为 `POST` 路由，并通过 Apple 专用方法传递可选的 Apple `user` 表单字段：

```rust,ignore
let identity = Auth::oauth("apple")
    .verify_apple_identity(&code, &state, form_post_user.clone())
    .await?;

let (user, session) = Auth::oauth("apple")
    .complete_with_apple_form_post(&code, &state, form_post_user)
    .await?;
```

`AppleIdentity` 包含稳定 subject、可选的已验证邮箱、`email_verified` 和 `is_private_email`。请将 subject 作为稳定键持久化。Apple 可能只在第一次授权期间提供显示名称，因此提供者适配器必须保留第一次 `form_post` 的值。

Apple 令牌和身份验证属于已安装的提供者实现。当前 Magnetar 提供者要求检查签名、issuer、audience、expiry 和 nonce，而不是信任 ID 令牌解码后的 JSON。

## 魔法链接登录

魔法链接登录使用已安装的 Magnetar 密码/会话引擎。框架返回明文的一次性令牌，而应用负责邮件组合和 URL 形态：

```rust,ignore
use suprnova::{Auth, Mail};

let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;

let url = format!("https://app.example.com/auth/magic?token={token}");
Mail::to("alice@example.com")
    .send(MagicLinkMail { url })
    .await?;

let (user, session) = Auth::magic_link().consume(&token).await?;
```

`send` 会在签发令牌前应用认证滥用预算。`consume` 是一次性的，会应用因子门控，将生成的会话绑定到框架请求会话，并返回用户和 Magnetar 会话。

对于未验证的既有账户，成功消费魔法链接是首个邮箱证明。事务会收回账户，并移除临时密码、passkey、链接账户、双因素、会话和记住我状态，使先前占用账户的人无法保留访问权限。

## 要添加的路由

典型应用会添加这些路由：

```rust,ignore
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
post!("/auth/apple/callback", controllers::oauth::apple_callback),
post!("/auth/magic", controllers::magic_link::send),
get!("/auth/magic/callback", controllers::magic_link::consume),
```

为每一条 OAuth 和 passkey 起始/回调路由应用 `SessionMiddleware`。会话携带认证握手选择器，并把整个往返绑定到发起它的浏览器。

## 认证迁移

`suprnova-magnetar` crate 包含一个可感知架构形态的迁移引擎，用于 Torii、Suprnova Web、Suprnova API 以及现有 Magnetar 架构。它是一个库接口和示例，而不是 `suprnova` CLI 子命令。

启用 `migration` feature 以及源数据库驱动程序，并在应用之前运行一次 dry plan。对于 PostgreSQL：

```text
cargo run -p suprnova-magnetar \
  --features migration,seaorm-postgres \
  --example migrate -- \
  --source-shape torii \
  --database-url "$SOURCE_DATABASE_URL" \
  --app-database-url "$DATABASE_URL"
```

当源数据库和应用数据库使用其他驱动程序时，请改用 `seaorm-mysql` 或 `seaorm-sqlite`。

加入 `--apply` 应用已审核的计划。运行器会在导入前重新检查源和架构指纹，记录重试状态，拒绝身份冲突，并使用事务导入。MySQL 同数据库迁移使用由写屏障保护的影子交换，并带有可恢复的还原和中止路径。

请将生成的计划和报告保存在部署记录中。不要应用一个在审核后源指纹已经改变的计划。

## 参考

- 默认启动：`MagnetarConfig`、`PasskeyConfig` 和 `init_magnetar`。
- 门面：`Auth::oauth(provider)` 和 `Auth::magic_link()`。
- OAuth 安装：`MagnetarConfig::oauth`、`ReqwestOAuthTransport` 和 `FrameworkAbuseLimiter`。
- 迁移库：`suprnova-magnetar` crate 中的 `magnetar::migration`。
- Bearer 认证：`BearerTokenMiddleware`。

## 下一步

- [认证](authentication.md)涵盖密码、passkey、守卫、框架会话和引擎初始化。
- [认证流程](auth-flows.md)涵盖邮箱验证、密码重置、锁定和双因素认证。
- [邮件](mail.md)涵盖由应用负责的魔法链接投递。
- [会话](session.md)涵盖绑定 OAuth 和 passkey 认证握手的浏览器会话。
