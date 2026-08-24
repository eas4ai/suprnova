# 认证

Suprnova 提供一套 Laravel 形状的认证系统：一个静态的 `Auth` 门面、通过
`AuthManager` 解析出来的具名认证守卫、可插拔的用户提供者、您 User 模型上的一个 `Authenticatable` trait，以及用来给路由加门的中间件。一个脚手架生成的项目，启动时就已经带着一个会话认证守卫（`web`）和一个令牌认证守卫（`api`），针对您那个类型化的 `User` 接好了线，所以从您运行 `suprnova new` 那天起，登录、注册和受保护的路由就都能用。

## 组成部分

| 类型 | 角色 |
|---|---|
| `Auth` | 用于守卫以及 Magnetar 支撑的密码、magic-link、passkey 和 OAuth 操作的框架门面 |
| `MagnetarConfig` / `init_magnetar` | 组合并原子安装默认的密码、会话、锁定、passkey 和 factor 引擎 |
| `Authenticatable` | 您的应用模型实现的 trait；暴露出 `get_auth_identifier() -> String` 和密码哈希 |
| `UserProvider` | 取回应用用户的 trait；`EloquentUserProvider<M>` 和 `DatabaseUserProvider` 是内置的 |
| `AuthManager` | 持有 `AuthConfig` 和已注册的提供者；按需解析具名认证守卫 |
| `SessionGuard` / `TokenGuard` | 框架的有状态与无状态守卫契约 |
| `BearerTokenMiddleware` | 将 Magnetar bearer 会话解析为框架请求认证状态 |
| `AuthMiddleware` / `GuestMiddleware` / `BasicAuthMiddleware` | 路由认证守卫 |
| `Credentials` | JSON 形状的凭据映射，通常是 `{ "email", "password" }` |

框架守卫/提供者代码位于 `framework/src/auth/`。Magnetar 主机适配器和门面位于 `framework/src/magnetar_integration/`；引擎 crate 位于 `crates/suprnova-magnetar/`。高层的电子邮件验证、密码重置、锁定和 TOTP 流程位于 `framework/src/auth_flows/`，并在[认证流程](auth-flows.md)中说明。OAuth、Apple 和 magic-link 登录在[OAuth 和无密码登录](oauth.md)中说明。

## 标识符模型

这个已认证用户的 id，会作为一个 `String`，从头到尾贯穿 Suprnova - 会话存储、[`UserProvider::retrieve_by_id`]、记住我表、每一个认证事件。这个规范的表面是 `Authenticatable::get_auth_identifier() -> String`
（对应 Laravel 的 `getAuthIdentifier`）。数字型主键可以轻易地字符串化；UUID、ULID，以及不透明的 OAuth 提供者 id，都原样流过。

```rust
use std::any::Any;
use suprnova::Authenticatable;

impl Authenticatable for User {
    fn get_auth_identifier(&self) -> String {
        self.id.to_string()
    }

    fn get_auth_password(&self) -> Option<&str> {
        Some(&self.password)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
```

`get_auth_password` 就是内置的提供者用来通过 `hashing::verify_async`
校验一个明文密码的地方。对那些用其他方式认证的用户（OAuth、passkey、魔法链接），返回 `None`。`auth_identifier_name() -> &'static str` 方法（默认 `"id"`）给出 id 所在的那个列名。便捷方法
`auth_identifier() -> i64` 默认会解析这个字符串 id，遇到非数字的 id
就回退到 `0` - Suprnova 自身从不调用它；只有当您的模型是整数主键、并且想跳过这次解析时，才需要覆盖它。

### 为什么 Suprnova 有所不同

Laravel 的 `getAuthIdentifier()` 返回 `mixed`。PHP 不在乎这个 id 是一个 int、一个 UUID 字符串，还是一个来自遗留表的字符串型主键。Rust
需要一个单一的具体类型，让会话、提供者和事件都能对它达成一致。
`String` 是唯一一个能容纳所有 id 形状、又不强迫框架去了解您的应用用的是哪一种的选择。`auth_identifier()` 这个整数便捷方法，是为您的列是一个 `BIGINT` 的常见情形而存在的，但框架自身从不依赖它 - 明天把您的
`User` 换成 ULID，认证栈里没有任何东西会察觉到。

## 在启动时给认证接线

`config/auth.php` 在 Rust 这一侧的类似物，是一个注册为容器上
`AuthManager` 单例的 `AuthConfig`，外加一个注册在某个名字下的
`UserProvider`。`bootstrap.rs` 通常用两行就能把这两件事都做完：

```rust
use std::sync::Arc;
use suprnova::{App, Auth, AuthConfig, AuthManager, EloquentUserProvider};

use crate::models::user::User;

pub async fn bootstrap() -> Result<(), suprnova::FrameworkError> {
    // ……DB::init、安装 SessionMiddleware，等等。

    App::singleton(AuthManager::new(AuthConfig::from_env()));
    Auth::register_provider("users", Arc::new(EloquentUserProvider::<User>::new()))
        .expect("register users provider");

    Ok(())
}
```

`AuthConfig::from_env()` 会从 `AUTH_GUARD`（默认 `"web"`）读取默认的认证守卫，并且开箱就带两个具名认证守卫：一个 `web` 会话认证守卫和一个 `api` 令牌认证守卫，两者背后都是 `"users"` 这个提供者。需要更多认证守卫的应用（一个独立的 `admins` 提供者，有区别的有状态和无状态认证守卫），可以显式构建这份配置：

```rust
use suprnova::{AuthConfig, GuardConfig};

let config = AuthConfig::new("web")
    .guard("web", GuardConfig::session("users"))
    .guard("admin", GuardConfig::session("admins"))
    .guard("api", GuardConfig::token("users"));
```

## 初始化 Magnetar 引擎

API starter 会在数据库和 `APP_KEY` 就绪后初始化 Magnetar：

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register_auth() -> Result<(), suprnova::FrameworkError> {
    let database = DB::connection()?;
    let magnetar = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(magnetar).await
}
```

默认引擎共享应用的 SeaORM 连接，并会创建其 schema，除非选择 `.apply_migrations(false)`。它会原子安装密码/会话和 passkey 适配器。重新初始化会返回错误，而不是在另一请求仍使用旧存储时替换一个适配器。

`MagnetarConfig` 也接受会话、锁定和双因素策略值：

```rust,ignore
let magnetar = MagnetarConfig::from_sea_orm(database)
    .session_config(session_policy)
    .lockout_config(lockout_policy)
    .two_factor_config(factor_policy)
    .passkey_config(passkey_policy);
```

默认主机绑定使用带 `i64` 应用 ID 的规范 `app_users` 表。Magnetar 的公共 `UserId` 在门面边界保持不透明；默认绑定只在跨入应用表时解析存储的标识符。

### Magnetar 支撑的门面方法

已安装的引擎为这些框架拥有的方法提供动力：

- `Auth::password().register(...)`。
- `Auth::password().authenticate(...)`。
- `Auth::magic_link().send(...)` 和 `.consume(...)`。
- `Auth::passkey().begin_registration(...)` 和 `.finish_registration(...)`。
- `Auth::passkey().begin_authentication(...)` 和 `.finish_authentication(...)`。
- 安装 OAuth delegate 时的 `Auth::oauth(provider)`。
- 记住我的签发、轮换和吊销。
- 通过 `BearerTokenMiddleware` 查找 bearer 会话。
- `suprnova::magnetar_integration` 中的 `list_sessions`、`revoke_session` 和 `revoke_all_sessions`。

成功登录会轮换框架会话 ID 和 CSRF 令牌，存储应用用户 ID，并记录一个不透明的 Magnetar web 绑定。框架继续拥有 HTTP 中间件、cookie、邮件、事件以及其守卫/提供者契约。

### 密码认证

当应用需要集成的凭据、锁定、factor gate 和会话路径时，请使用 Magnetar 密码门面：

```rust,ignore
let user = Auth::password()
    .register("alice@example.com", password)
    .await?;

let (user, session) = Auth::password()
    .authenticate(
        "alice@example.com",
        password,
        request.header("User-Agent").map(str::to_string),
        request.peer_ip().map(str::to_string),
    )
    .await?;
```

`authenticate` 对无效凭据、锁定或所需的第二因素返回 HTTP 401 错误。存储和引擎失败仍是服务器错误。此方法绝不返回密码材料。

### Passkey

Passkey 的 begin 和 finish 调用需要 `SessionMiddleware`，因为一次性 ceremony selector 存在框架会话中：

```rust,ignore
let challenge = Auth::passkey()
    .begin_authentication("alice@example.com")
    .await?;

let (user, session) = Auth::passkey()
    .finish_authentication("alice@example.com", browser_credential)
    .await?;
```

注册遵循相应的 `begin_registration` 和 `finish_registration` 配对。现有账户的注册需要经验证的请求 actor 以及通过插件路径进行的最近重新认证；legacy 会话中的裸用户 ID 不会被提升为凭据 actor。

### 首次电子邮件证明和认证 epoch

Magnetar 将未验证账户上的第一次成功邮箱证明视为原子凭据边界。密码重置、magic-link 消费和 OAuth 经验证电子邮件完成都可能赢得这一边界。

事务会推进账户的认证 epoch、吊销旧会话和记住我凭据，并移除邮箱所有者到来前抢占者可能注册的临时凭据。密码、passkey、关联账户和双因素写入都携带 actor 快照，若操作进行期间账户 epoch 已改变则失败。

对已验证的账户，密码重置会保留合法 passkey、关联账户和双因素注册，同时仍轮换密码并使会话失效。OAuth 永不会仅凭电子邮件自动关联未验证的现有账户；它需要依主机策略完成已验证电子邮件或显式关联。

### 直接使用 Magnetar crate 表面

大多数应用会停留在框架门面。构建自定义身份主机的应用可直接依赖 `suprnova-magnetar`，以获得：

- 框架无关的 plugin route 和 effect handler。
- 密码及密码管理 plugin。
- Passkey 和双因素引擎。
- OAuth 授权、grant、provider plugin、设备授权和 token-broker 服务。
- 不透明、JWT、remember 和 grant 会话引擎。
- 自定义存储绑定和默认 SeaORM schema。
- 形状感知的 auth-data 迁移。

直接使用不会把 HTTP 或应用用户所有权转移给 Magnetar。主机仍会把 wire 请求、邮件 effect、应用 ID、速率限制 driver 和会话绑定映射进它自己的框架。

## `Auth` 门面

静态的 `Auth` 门面，是您从控制器和中间件里调用的那个 Laravel 形状的表面。基于凭据和基于用户的方法，都会代理给**默认认证守卫**
（`AuthConfig::default_guard` 指向的那个，默认是 `"web"`）；同步的
`check`/`guest`/`id` 读取是基于会话的快速路径，不需要任何 manager。

```rust
use suprnova::{Auth, Credentials};

// 校验凭据并把用户登录进去。会触发 Attempting → （Login + Authenticated），
// 并遵从记住我。返回解析出来的用户，凭据不对则返回 None。
if let Some(user) = Auth::attempt(&Credentials::password(&email, &password), remember).await? {
    println!("Welcome, user {}", user.get_auth_identifier());
}

// 直接把一个已知的用户登录进去。
Auth::login(user, remember).await?;

// 按 id 登录，不重新检查凭据（比如刚完成注册）。
Auth::login_using_id(&id, remember).await?;

// 校验凭据但不持久化会话（密码确认对话框）。
let ok: bool = Auth::validate(&Credentials::password(&email, &password)).await?;

// 只为这一次请求做认证 - 不写会话。对应 Laravel 的 `once`。
let ok: bool = Auth::once(&Credentials::password(&email, &password)).await?;
Auth::once_using_id(&id).await?;

// 基于会话的快速路径（不需要 AuthManager）。
if Auth::check()    { /* 已认证 */ }
if Auth::guest()    { /* 未认证 */ }
if let Some(id) = Auth::id() { /* 字符串 id */ }

// 当前用户在这次请求里，是否是通过记住我 cookie 完成认证的。
// 对应 Laravel 的 `viaRemember()`。
if Auth::via_remember() { /* … */ }

// 解析当前用户（通过已注册的提供者）。
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
if let Some(user) = Auth::user_as::<User>().await? {
    println!("Welcome, {}!", user.name);
}

// 拆除认证状态 + 吊销记住我 + 轮换 CSRF + 触发 Logout。
Auth::logout().await?;

// 完整地销毁会话（重新生成 id + 清空 + 吊销记住我 + 触发 Logout）。
Auth::logout_and_invalidate().await?;
```

`Auth::attempt` 在成功时返回解析出来的用户，而不是一个朴素的 `bool` - 比 Laravel 的 API 更丰富，也省掉了紧跟着的那次 `Auth::user()` 调用。`Ok(None)` 意味着这份凭据没能解析出一个用户；`Err` 意味着一次需要向上冒泡的数据库 / 哈希 / 配置失败。

如果您已经自己验证过一个用户的身份，只是想建立会话 - 比如在一次
OAuth 回调完成之后 - 就伸手去用这个同步原语：

```rust
// 同步，没有提供者，没有 AuthManager，没有事件。在请求作用域之外调用时
// 会返回 Err（没有安装 SessionMiddleware），这样一次被悄悄丢弃的登录，
// 就永远不会看起来像是成功了。
Auth::login_id(user.id.to_string())?;
```

`login_id` 会重新生成会话 id（防止会话固定攻击）并轮换 CSRF 令牌，然后把这个 id 写进会话。它是刻意做成一遇到问题就明确失败的：早前的版本在会话作用域之外会悄悄地什么也不做，审计修正了这一点 - 一次从未真正落地的“成功登录”，是那种没有别的机制能抓住的 bug。

## `Auth::user()` 和 `user_as<T>`

`Auth::user()` 会返回这个 trait 背后的用户：

```rust
if let Some(user) = Auth::user().await? {
    println!("user id: {}", user.get_auth_identifier());
}
```

这个 trait 对象涵盖了任何实现了 `Authenticatable` 的类型。要拿回您那个具体的 `User`，就通过 `user_as::<T>()` 向下转型：

```rust
use suprnova::Auth;
use crate::models::user::User;

if let Some(user) = Auth::user_as::<User>().await? {
    // 直接在模型上访问字段。
    println!("Welcome, {}!", user.name);
}
```

`user_as` 会在没有用户被认证时返回 `Ok(None)`，*同时*在解析出来的用户不是一个 `T` 时也返回 `Ok(None)`（比如栈里别处调用了一次不同类型的 `Auth::set_user(...)`）。在一次请求内部，用户是按请求缓存的，所以反复调用 `Auth::user()` 只会命中提供者一次。

## 具名认证守卫

裸的 `Auth::*` 方法只和默认认证守卫对话。要针对一个特定的认证守卫做操作，就按名字把它解析出来：

```rust
use suprnova::Auth;

// 只读操作在每一种驱动程序上都能用。
if Auth::guard("api")?.check().await? { /* … */ }

// Login/logout/attempt 需要一个有状态的认证守卫。令牌认证守卫在这里会明确地失败。
let user = Auth::stateful_guard("web")?
    .attempt(&credentials, false)
    .await?;
```

`Auth::guard("name")` 返回 `Arc<dyn Guard>`（只读契约），
`Auth::stateful_guard("name")` 返回 `Arc<dyn StatefulGuard>`（加上了
`attempt`/`login`/`logout`）。对一个令牌认证守卫索要有状态契约，会返回一个带修复提示的错误，而不是悄悄地限制这套 API。

## 用户提供者

一个 `UserProvider` 告诉认证栈该如何取用户、验证用户。两个提供者是内置的，所以常见情形都不需要自定义实现：

- **`EloquentUserProvider<M>`** - 通过一个类型化的、同时也是
  `Authenticatable` 的 `#[suprnova::model]` `User` 来解析。按主键查
  id，按 `email`（默认）查凭据。
- **`DatabaseUserProvider`** - 按名字把一张原始表解析成一个
  `GenericUser`（id + 属性映射）。当您没有、或者不想要一个类型化模型时，就用它。

两者都会拿凭据查找去对照一份允许列表（默认 `["email"]`）做过滤 - 一份心怀恶意的凭据映射，没法注入额外的 `WHERE` 谓词。用
`.credential_columns([...])` 自定义这份允许列表，用
`.identifier_column("uuid")` 自定义查找列，或者用 `.with_id_parser(...)`
自定义 id 绑定策略。

要接入一个自定义源（LDAP、一个外部 API），直接实现 `UserProvider`。
`retrieve_by_id` 把标识符当作一个 `&str` 接收：

```rust
use async_trait::async_trait;
use std::sync::Arc;
use suprnova::{Authenticatable, FrameworkError, UserProvider};

struct LdapProvider;

#[async_trait]
impl UserProvider for LdapProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        // ……从 LDAP 取数据，作为 Arc<dyn Authenticatable> 返回
        Ok(None)
    }

    // retrieve_by_credentials + validate_credentials 都有返回 None / false 的
    // trait 默认实现。要针对您的数据源支持 `Auth::attempt` 和
    // `Auth::validate`，就覆盖它们。
}
```

把它注册到 manager 上：

```rust
Auth::register_provider("ldap", Arc::new(LdapProvider))?;
```

## 保护路由

### `AuthMiddleware`

给仅限已认证用户的路由加门。未认证的请求会被重定向到一个登录页，或者收到 `401`：

```rust
use suprnova::{AuthMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/dashboard", controllers::dashboard::index)
        .post("/logout", controllers::auth::logout)
        .middleware(AuthMiddleware::redirect_to("/login"))
}
```

`AuthMiddleware::new()` 则会返回 `401 Unauthorized` - 最适合 JSON
API。`AuthMiddleware::redirect_to("/login")` 对常规请求发出一个
`302`，对 Inertia 请求发出一个 `409 X-Inertia-Location`（Inertia 客户端会把它变成一次整页访问）。要针对一个特定的认证守卫来做门控，就串联 `for_guard`：

```rust
// 除非 api 这个认证守卫已经通过认证，否则就是 401。
.middleware(AuthMiddleware::new().for_guard("api"))
```

一个令牌认证守卫（`for_guard("api")`）依赖链条里更早运行的某个
Bearer 令牌中间件，去填充这次请求的认证 id；没有它，这个认证守卫永远会报告未认证。

### `GuestMiddleware`

反过来的情形 - 用于那些已认证用户不该看到的登录页和注册页：

```rust
use suprnova::{GuestMiddleware, Router};

pub fn routes() -> Router {
    Router::new()
        .get("/login", controllers::auth::show_login)
        .post("/login", controllers::auth::login)
        .get("/register", controllers::auth::show_register)
        .post("/register", controllers::auth::register)
        .middleware(GuestMiddleware::redirect_to("/dashboard"))
}
```

`GuestMiddleware::for_guard("name")` 的工作方式和
`AuthMiddleware::for_guard` 一样。

### `BasicAuthMiddleware`

根据一个认证守卫的提供者，对 `Authorization: Basic` 请求头做 HTTP
Basic 认证：

```rust
use suprnova::BasicAuthMiddleware;

// 有状态 - 成功时把用户登录进会话（对应 Laravel 的 `basic`）。
.middleware(BasicAuthMiddleware::new())

// 无状态 - 只为这一次请求做认证（对应 Laravel 的 `onceBasic`）。
.middleware(BasicAuthMiddleware::once())
```

解码出来的用户名，会对照 `field` 这个凭据字段做匹配（默认是
`"email"`）；一个缺失、格式错误或者无效的请求头，会返回 `401`，并带一个 `WWW-Authenticate: Basic realm="..."` 质询。用 `.field(...)`、
`.realm(...)` 和 `.for_guard(...)` 来配置。

## 生命周期事件

这些认证守卫会派发五个生命周期事件。通过 [`EventFacade`](events.md)
监听它们：

| 事件 | 时机 |
|---|---|
| `Attempting` | 一次凭据尝试开始时（`attempt`/`once`） |
| `Authenticated` | 一个用户在这次请求里被主动认证时（`login`/`once`/`once_using_id`） |
| `Login` | 一个用户被持久化到会话时（`login`/成功的 `attempt`） |
| `Logout` | 一个用户被登出时 |
| `Failed` | 一次凭据尝试失败时（密码不对或 id 未知） |

每一个事件都携带认证守卫的名字和一个字符串形式的用户 id - 永远不带明文密码，也永远不带原始的凭据映射。`Authenticated` 只在一个用户被主动建立时触发，而不是在从一个既有会话上被动解析出 `Auth::user()`
时触发，所以监听者不会在每一次已认证请求上收到一连串重复事件。

## 脚手架生成的登录流程

`suprnova new` 会生成一个认证控制器，它对已注册的提供者使用
`Auth::attempt`。`FormRequest` 和 `Validate` 会产出 `{ message, errors }` 校验信封。对于 Inertia 请求，已安装的校验重定向中间件会把这次失败变成 HTTP `303 See Other` 重定向，重定向回发起页面并闪存这些错误。非 Inertia 客户端会收到 HTTP `422 Unprocessable Entity` JSON 信封：

```rust
use serde::Deserialize;
use suprnova::{
    handler, inertia_response, redirect, serde_json, Auth, Credentials,
    FormRequest, InertiaProps, Request, Response, Validate, ValidationErrors,
};

#[derive(InertiaProps)]
pub struct LoginProps {
    pub errors: Option<serde_json::Value>,
}

#[handler]
pub async fn show_login(req: Request) -> Response {
    inertia_response!(&req, "auth/Login", LoginProps { errors: None })
}

#[derive(Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email(message = "Please enter a valid email address"))]
    pub email: String,
    #[validate(length(min = 1, message = "Password is required"))]
    pub password: String,
    #[serde(default)]
    pub remember: bool,
}

impl FormRequest for LoginRequest {}

fn invalid_credentials() -> suprnova::FrameworkError {
    let mut errs = ValidationErrors::new();
    errs.add("email", "These credentials do not match our records.");
    suprnova::FrameworkError::Validation(errs)
}

#[handler]
pub async fn login(form: LoginRequest) -> Response {
    match Auth::attempt(
        &Credentials::password(&form.email, &form.password),
        form.remember,
    )
    .await?
    {
        Some(_user) => redirect!("/dashboard").into(),
        None => Err(invalid_credentials().into()),
    }
}

#[handler]
pub async fn logout(_req: Request) -> Response {
    Auth::logout().await?;
    redirect!("/").into()
}
```

注册遵循同样的形态：校验表单，创建用户，然后
`Auth::login(Arc::new(user), false).await?` 把这个刚创建出来的用户登录进会话，并触发 `Login` 事件。

## 脚手架生成的 `User` 模型

生成出来的 `User` 是一个实现了 `Authenticatable` 的
`#[suprnova::model]`。它还包含 `email_verified_at: Option<DateTime<Utc>>`，并实现
`MustVerifyEmail` 和 `CanResetPassword`。这些桥接让
`EloquentUserProvider<User>` 能标记电子邮件验证状态，并提供密码重置所需的身份数据。下方摘录只展示守卫登录所需的字段和辅助函数，是一个不完整的片段；完整的认证流程实现请使用生成的模型模板。密码辅助函数由 [`hashing`](hashing.md) 模块支撑：

```rust
use chrono::{DateTime, Utc};
use suprnova::{attrs, hashing, model, Authenticatable, FrameworkError};

#[model(
    table = "users",
    fillable = ["name", "email", "password"],
    hidden = ["password", "remember_token"],
    timestamps,
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub remember_token: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl User {
    pub async fn find_by_email(email: &str) -> Result<Option<Self>, FrameworkError> {
        <Self as suprnova::eloquent::Model>::query()
            .filter("email", email)
            .first()
            .await
    }

    pub fn verify_password(&self, password: &str) -> Result<bool, FrameworkError> {
        hashing::verify(password, &self.password)
    }

    pub async fn create(
        name: impl Into<String>,
        email: impl Into<String>,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        let hashed = hashing::hash(password)?;
        <Self as suprnova::eloquent::Model>::create(attrs! {
            name: name.into(),
            email: email.into(),
            password: hashed,
        })
        .await
    }
}
```

`hidden = ["password", "remember_token"]` 这个属性，会让模型在为传输而序列化成 JSON 时跳过这些列 - 它们存在于这个结构体上，但永远不会从一个 Inertia 响应里泄漏出去。

## 记住我

安装 Magnetar 引擎后，`Auth::attempt(credentials, true)` 和 `Auth::issue_remember_cookie` 会签发绑定用途的 Magnetar 记住我凭据。浏览器仍接收框架加密的 `remember_me` cookie，而 Magnetar 拥有验证器存储、auth-epoch 检查、一次性轮换、异常处理和吊销。

在没有活动框架登录的请求上，`SessionMiddleware` 通过已安装引擎消费 cookie，轮换记住我凭据、签发新的 Magnetar 会话，并绑定两层会话。陈旧 auth epoch、已吊销的账户会话、格式错误凭据或重放都不会认证该请求。

`Auth::revoke_remember_tokens()` 会让当前用户的每一个记住我凭据都失效。清除 cookie 会在后端吊销之前排队，因此即使存储操作失败，浏览器也会丢弃其凭据。

没有安装 Magnetar 引擎时，框架为兼容性保留 legacy `remember_tokens` 回退。新应用应初始化 Magnetar，而不是依赖该回退。

## 安全保证

一份认证栈所确立的不变量的简短清单：

- **`Auth::login_id` 在请求作用域之外会明确地失败。** 早前的版本会悄悄地丢掉这次会话写入；一次从未真正落地的“成功登录”，是那种没有别的机制能抓住的 bug。
- **会话 id 和 CSRF 令牌在每一次登录时都会重新生成。** `login_id`，以及由认证守卫支撑的 `login`/`attempt`，都会轮换它们，以防止会话固定攻击。
- **登出会在吊销记住我之前先清空认证状态。** 如果数据库那次吊销失败了，会话早已经处在登出状态，所以一个陈旧的认证槽位，不可能在一次不完整的登出中存活下来。清除记住我 cookie 的动作，排在数据库删除*之前*，所以即便那一行的删除失败了，浏览器也会把这个 cookie 丢掉（之后的清理扫荡会收尾）。
- **凭据允许列表挡住注入。** 两个内置的提供者，都会拿
  `retrieve_by_credentials` 去对照 `credential_columns` 做过滤，所以一个被攻击者影响的凭据映射里多出来的键，没法变成额外的 `WHERE`
  谓词。
- **凭据写入受 actor 围栏保护。** 密码、passkey、关联账户、双因素、会话和 remember 变更都携带经验证认证所确立的用户 ID 和 auth epoch。吊销或首次证明 epoch 变更会让进行中的陈旧写入失败。
- **首次邮箱证明是原子的。** 在未验证账户上，密码重置、magic-link 消费或 OAuth 经验证电子邮件完成会在同一事务中推进 auth epoch 并移除临时凭据。并发抢占者写入无法在提交后恢复访问。
- **电子邮件验证绑定 actor。** 框架验证门面需要一个 ID 与令牌所有者相匹配的已认证用户。另一账户的令牌会被拒绝且不会消费。
- **OAuth 电子邮件不是账户所有权。** 未验证的现有账户绝不会仅根据提供者电子邮件自动关联。已验证账户需要显式关联；未验证账户需要首次邮箱证明完成路径。
- **认证事件永远不携带明文。** 只有认证守卫的名字 + 字符串用户 id，没有别的。失败尝试的追踪（按邮箱建键的锁定），属于
  [认证流程](auth-flows.md) 里的 `BruteForce`，不属于这些生命周期事件。

[会话](session.md) 一章覆盖了那些基于会话的认证守卫所继承的 cookie 配置（`SESSION_LIFETIME`、`SESSION_COOKIE`、`SESSION_SECURE`、`SESSION_SAME_SITE` 和 `SESSION_COOKIE_PREFIX`）。

## 下一步

- [认证流程](auth-flows.md) - 电子邮件验证、密码重置、Magnetar 支撑的账户锁定、框架 TOTP 2FA 和认证流程事件
- [OAuth 和无密码登录](oauth.md) - Magnetar OAuth、Apple、magic link、provider 策略和 auth-data 迁移
- [授权](authorization.md) - `Gate`、策略和 `Authorizable`
- [会话](session.md) - 浏览器会话和 cookie 层
- [CSRF 保护](csrf.md) - 状态变更请求保护
- [哈希](hashing.md) - bcrypt 和 Argon2 辅助函数
