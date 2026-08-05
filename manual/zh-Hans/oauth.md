# OAuth、Apple 和魔法链接登录

Suprnova 在 `Auth` 门面背后提供三种由 torii 支撑的登录方式：**通用 OAuth**（GitHub、Google，或者任何 OIDC/OAuth2 提供者）、**使用 Apple 登录**，以及**无密码的魔法链接**。它们共享同一个前提条件（`init_torii` 加上那个握手迁移）和同一套门面形态 - `Auth::oauth(provider)` / `Auth::magic_link()` - 而且都不自带路由：您添加一个薄薄的控制器（起始 + 回调），框架就会处理 CSRF state、PKCE、令牌交换、身份验证、用户 upsert 和会话铸造。

整个表面都住在 `framework/src/torii_integration/` 里。这里面**没有**任何框架层面的环境变量约定 - 每一份凭据都是以编程方式传入的（从您自己的环境里取）；本章的示例用 `std::env::var(...)` 纯粹是为了示意您的密钥该放在哪儿。

## 前提条件

1. **在启动时初始化一次 torii** - 这是用户 upsert 和会话创建的后盾：

   ```rust
   use suprnova::{init_torii, ToriiConfig};

   // 在 bootstrap::register() 里，DB::init() 之后
   init_torii(ToriiConfig::from_sea_orm(db_conn)).await?;
   ```

2. **运行这个握手迁移。** OAuth 和 Apple 会把一个短命的（10 分钟）CSRF `state` + PKCE 握手，暂存进 `auth_ceremony_tokens` 表。请在您的 `Migrator` 里注册迁移 `m20251209_000000_create_auth_ceremony_tokens_table`（脚手架生成的起步模板已经包含它）。可以选择性地调度 `suprnova::torii_integration::ceremony::prune_expired()` 来回收过期的行。

3. **在 OAuth *起始*路由上安装 `SessionMiddleware`。** `begin()` 会把 `state` 写进会话；一次没有会话的调用会以 500 失败。

魔法链接只需要第 1 步。

## 通用 OAuth（GitHub、Google、自定义）

### 配置一个提供者

在启动时为每个提供者注册一次。这个注册表是进程全局的、幂等的，所以重复注册同一个提供者只会替换掉它的配置：

```rust
use suprnova::Auth;
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("github").configure(OAuthProviderConfig {
    client_id: std::env::var("GITHUB_CLIENT_ID")?,
    client_secret: std::env::var("GITHUB_CLIENT_SECRET")?,
    redirect_url: "https://app.example.com/auth/oauth/github/callback".into(),
    scopes: vec!["user:email".into()],
    endpoints_override: None,   // None → 内置的常见端点表
    apple_key_pair: None,       // 仅 Apple 需要；GitHub/Google 留 None
    apple_team_id: None,        // 仅 Apple 需要
});
```

`github`、`google` 和 `apple` 的常见 authorize/token/userinfo 端点是内置好的。对于任何其他提供者 - 或者一个自托管 / 测试服务器 - 请自己提供它们：

```rust
use suprnova::torii_integration::oauth::EndpointOverrides;

Auth::oauth("gitlab").configure(OAuthProviderConfig {
    client_id: /* … */,
    client_secret: /* … */,
    redirect_url: /* … */,
    scopes: vec!["read_user".into()],
    endpoints_override: Some(EndpointOverrides {
        authorize: "https://gitlab.com/oauth/authorize".into(),
        token: "https://gitlab.com/oauth/token".into(),
        userinfo: "https://gitlab.com/api/v4/user".into(),
        emails: None,   // GitHub 风格的 /emails 回退，用于一个私密的主邮箱
    }),
    apple_key_pair: None,
    apple_team_id: None,
});
```

### 启动这个流程（授权 URL）

```rust
// GET /auth/oauth/github/start（这个路由必须携带 SessionMiddleware）
let kickoff = Auth::oauth("github").begin().await?;
// kickoff.authorization_url - 把浏览器重定向到这里
// kickoff.state - CSRF state，已经替您存进了会话里
```

`begin()` 会铸造 CSRF `state`（UUID v4）和一个 RFC 7636 PKCE 验证值/S256 质询值，记录这次握手（10 分钟 TTL），并返回提供者的授权 URL。把用户重定向到 `authorization_url`。

### 完成这个流程 - `verify` 对比 `complete`

在回调上您有两个入口点（在 0.5.4 里拆开的）。按您的 `users` 表**是不是** torii 的表结构来选择：

| 方法 | 返回值 | 副作用 | 何时使用 |
|---|---|---|---|
| `verify_oauth_identity(code, state)` | `OAuthIdentity { provider, subject, email, name }` | **没有** - 验证这次握手，交换这个 code，取回用户信息，提取出一个已验证的邮箱 + 一个稳定的 `subject`。没有用户，没有会话。 | 您的应用自己拥有 `users` 表，并且您想自己查找 / 创建用户。 |
| `complete(code, state)` | `(User, Session)` | 把用户 upsert 进 torii（`get_or_create_user`），并铸造一个会话。 | 您的 `users` 表就是 torii 的表结构。 |

```rust
// 自定义 users 表：
let id = Auth::oauth("github").verify_oauth_identity(&code, &state).await?;
// id.subject 是这个提供者的稳定 id；id.email 是已验证的，或者是 None。
let user = my_users::upsert(id.provider, id.subject, id.email, id.name).await?;

// ……或者，torii 支撑的：
let (user, session) = Auth::oauth("github").complete(&code, &state).await?;
```

`verify` 返回的 `email` 总是一个*已验证*的地址（OIDC 的 `email_verified`、GitHub 视为已验证，或者 `/emails` 回退得到的）；一个未验证或缺失的邮箱会以 `None` 的形式回来，之后的重复登录会按 `subject` 来解析。

### 您要添加的路由

框架不提供任何 OAuth 路由 - 请接好两个薄薄的处理程序（镜照脚手架起步模板里已有的 `auth_verify` / `auth_reset` 控制器的形态）：

```rust
// 起始 - 重定向到提供者
get!("/auth/oauth/{provider}/start", controllers::oauth::start),
// 回调 - GitHub/Google 用 GET ?code&state
get!("/auth/oauth/{provider}/callback", controllers::oauth::callback),
```

把 `/start` 路由（至少这一个）放在 `SessionMiddleware` 后面。

## 使用 Apple 登录

Apple 用的是同一个门面 - `Auth::oauth("apple")` - 只是内置了几条 Apple 专属的规则：

- **回调是一个 `POST`。** Apple 用的是 `response_mode=form_post`，所以重定向是把 `code` + `state` 放进表单正文里投递，不是查询参数。请把 Apple 的回调注册成一个 `post!` 路由，并从表单字段里读取它们。
- **没有 PKCE。** Apple 会拒绝 `code_challenge`，所以授权 URL 省去了它（客户端密钥换成了一个签过名的 JWT）。
- **`client_secret` 不会被用到** - 留它 `String::new()` 就好。Suprnova 会在每一次令牌交换时，从您的 `.p8` 密钥铸造出那个短命的 JWT 客户端密钥。
- **自 0.5.6 起，ID 令牌会对照 Apple 的 JWKS（RS256）做验证**，而不是结构性地直接信任。

### 提供您的 Apple 密钥 - `AppleKeyPair`

`AppleKeyPair` 是唯一一个为应用重新导出的 Apple 类型（所以您不需要直接依赖 `apple` 这个 crate）。从您的 `.p8` 签名密钥构造它：

```rust
use suprnova::torii_integration::oauth::AppleKeyPair;

let key = AppleKeyPair::from_file(
    &std::env::var("APPLE_KEY_ID")?,   // Apple 的 *Key ID*（不是 Team ID）
    &std::env::var("APPLE_P8_PATH")?,  // AuthKey_XXXXXX.p8 的路径
)?;
// 或者：AppleKeyPair::from_base64(key_id, b64)  /  from_pem_bytes(key_id, bytes)
```

### 配置 Apple

```rust
use suprnova::torii_integration::oauth::OAuthProviderConfig;

Auth::oauth("apple").configure(OAuthProviderConfig {
    client_id: std::env::var("APPLE_CLIENT_ID")?,  // 您的 Services ID
    client_secret: String::new(),                  // 不会被用到 - 是从密钥铸造出来的
    redirect_url: "https://app.example.com/auth/apple/callback".into(),
    scopes: vec!["email".into(), "name".into()],
    endpoints_override: None,
    apple_key_pair: Some(key),
    apple_team_id: Some(std::env::var("APPLE_TEAM_ID")?),  // 10 个字符的 Team ID
});
```

### 完成 Apple 流程

和通用 OAuth 一样的拆分。`complete` 会做 upsert + 会话；verify 这条路径会为一张自定义的 users 表返回一个 `AppleIdentity`：

```rust
// POST /auth/apple/callback - 从表单正文里读取 code + state
let (user, session) = Auth::oauth("apple").complete(&code, &state).await?;

// ……或者自定义 users 表：
let id = Auth::oauth("apple").verify_apple_identity(&code, &state).await?;
// id: AppleIdentity { provider, subject, email, email_verified, is_private_email }
```

`AppleIdentity.email` 只在 Apple 断言它已验证时才是 `Some(_)`；一个未验证的邮箱会在这个身份构造出来*之前*就被拒绝（401）。`is_private_email` 会在用户选择了 Apple 的私密转发地址时被设置 - 请把 `subject` 当作那个稳定的键持久化下来，因为转发地址是您能拿到的唯一邮箱。

## 魔法链接登录

无密码的邮箱登录，由 torii 支撑，通过 `Auth::magic_link()`。框架负责签发和验证这个令牌；**您**负责把链接发出去（框架本身从不发邮件），这和 [邮件](mail.md) 一章能干净地组合起来。

```rust
use suprnova::Auth;

// POST /auth/magic - 请求一个链接
let token = Auth::magic_link()
    .send("alice@example.com", "https://app.example.com/auth/magic")
    .await?;
// 自己构造链接并把它发出去：
Mail::to("alice@example.com")
    .send(MagicLink { url: format!("https://app.example.com/auth/magic?token={token}") })
    .await?;

// GET /auth/magic?token=… - 消费它（一次性；第二次调用会失败）
let (user, session) = Auth::magic_link().consume(&token).await?;
```

用户会在第一次使用时被自动创建。`send` 返回**明文**令牌，好让您自己掌控 URL 的形态和投递方式。

> **说明 - `TokenPurpose::MagicLink`。** `auth_flows` 的
> `TokenPurpose` 枚举有一个 `MagicLink` 变体（在 0.5.5 中新增），但它是给通用的
> `TokenStore` 保留的一个*判别值* - 没有任何内置流程会消费它。
> 那条能工作、受支持的魔法链接路径，就是上面的 `Auth::magic_link()`。只有当您在
> `auth_flow_tokens` 表上手撸自己的流程时，才需要伸手去用 `TokenPurpose::MagicLink`。

## 关于配置的说明

这些方法都不读取框架的环境变量 - 提供者 ID、密钥、重定向 URL 和 Apple 密钥全都以编程方式传给 `configure(...)`。您可以用任何自己喜欢的方式加载它们（`std::env::var`、一个类型化的配置结构体、一个密钥管理器），并在 `bootstrap` 期间把提供者注册一次。这让多租户 / 按部署区分的提供者配置成为一等公民，而不是强推一套固定的环境变量命名方案。

## 参考

- 门面入口点：`Auth::oauth(provider)`、`Auth::magic_link()`
  （`suprnova::Auth`）
- 配置：`suprnova::torii_integration::oauth::{OAuthProviderConfig, EndpointOverrides, AppleKeyPair}`
- OAuth 结果：`OAuthKickoff { authorization_url, state }`、
  `OAuthIdentity { provider, subject, email, name }`、
  `AppleIdentity { provider, subject, email, email_verified, is_private_email }`
- 启动：`suprnova::{init_torii, ToriiConfig}`
- 握手存储：`auth_ceremony_tokens` 表 +
  `suprnova::torii_integration::ceremony::prune_expired()`

## 下一步

- [认证](authentication.md) - 认证守卫、提供者，以及这些流程为之创建会话的
  `Authenticatable` 用户模型
- [认证流程](auth-flows.md) - 电子邮件验证、密码重置和 2FA
- [邮件](mail.md) - 发送魔法链接邮件（以及 `MAIL_FROM` /
  `MAIL_FROM_NAME` 发信人配置）
- [会话](session.md) - 返回的那个 `Session` 是什么，以及它是如何被持久化的
