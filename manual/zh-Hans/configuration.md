# 配置

Suprnova 从环境变量（开发时从 `.env` 加载，生产时从进程环境加载）读取配置，并以两种方式将其暴露给您的代码：

1. **直接环境访问** - `env::env`、`env_required`、`env_optional`
   用于一次性查询
2. **类型化配置结构体** - `Config::register` / `Config::get` 用于需要读取多次的配置，具有强类型支持

框架本身会读取一些环境变量（`APP_KEY`、
`APP_ENV`、`DATABASE_URL` 等）；其余的由您来决定。

## `.env` 文件

`suprnova new` 会生成一个起始 `.env` 文件，包含您的应用启动所需的值：

```env
APP_NAME="my-app"
APP_ENV=local                # local, development, staging, production, testing, …
APP_DEBUG=true               # detailed error pages + verbose logs
APP_URL=http://localhost:8765

# 32-byte AES-256 key (URL-safe base64, no padding). Encrypts session
# cookies, pagination cursors, and anything via `suprnova::Crypt`.
# Generated at scaffold time. Rotate with `suprnova key:generate`.
APP_KEY=<32-byte base64>

SERVER_HOST=127.0.0.1
SERVER_PORT=8765
VITE_PORT=5765

# Database - SQLite by default; swap to postgres://user:pass@host/db
DATABASE_URL=sqlite://./database.db
DB_MAX_CONNECTIONS=10
DB_MIN_CONNECTIONS=1
DB_CONNECT_TIMEOUT=30
DB_LOGGING=false

# Session
SESSION_LIFETIME=120         # minutes
SESSION_COOKIE=suprnova_session
SESSION_SECURE=false         # set true in production (HTTPS only)
SESSION_PATH=/
SESSION_SAME_SITE=Lax

# Mail - defaults to `log` driver (writes outgoing mail to the
# tracing log, good for dev). Set MAIL_DRIVER to one of
# smtp / ses / mailgun / postmark / sendgrid / resend / log / memory
# for production.
MAIL_DRIVER=log
# SMTP credentials (only read when MAIL_DRIVER=smtp):
MAIL_SMTP_HOST=127.0.0.1
MAIL_SMTP_PORT=587
MAIL_SMTP_USER=
MAIL_SMTP_PASS=
# starttls | tls | none. Left blank it derives from the credentials
# above - starttls with them, none without. Production refuses to boot
# unencrypted; see the Mail chapter.
MAIL_SMTP_ENCRYPTION=
```

同时提供一个 `.env.example` 文件，包含相同的键和占位符值 -
提交它；不要提交 `.env`。默认的 `.gitignore` 已经排除了 `.env`。

## `.env` 加载如何工作

启动时，框架会：

1. 从 `APP_ENV` 检测环境（不区分大小写，
   `prod`/`dev`/`stage`/`stg`/`test` 也可识别）。
2. 从项目根目录加载 `.env`。
3. 如果存在特定环境的文件（`.env.staging`、`.env.production`），在 `.env` 之上加载它 - 其值会覆盖 `.env`。
4. 真实的进程环境变量覆盖两者（这是容器编排依赖的）。

一句话总结顺序：**进程环境变量 > `.env.<environment>` > `.env`**。

```rust
use suprnova::Config;

let env = Config::environment();           // Environment::Local
let is_prod = Config::is_production();     // false
```

在 CI 运行中，使用 `APP_ENV=testing`，框架会在 `.env` 之上加载 `.env.testing`，这样您可以覆盖数据库 URL 和禁用邮件驱动程序，而无需修改开发用的 `.env`。

## 直接环境访问

对于一次性读取字符串、数字、布尔值 - 任何实现了
`std::str::FromStr` 的类型 - 使用 `env::*` 系列函数：

```rust
use suprnova::config::{env, env_required, env_optional};

let port: u16 = env("SERVER_PORT", 8765);                    // 带回退值
let url: String = env_required("APP_URL");                   // 缺失则触发 panic - 仅限启动时
let smtp_host: Option<String> = env_optional("MAIL_HOST");   // 缺失则返回 None
```

- `env(key, default)` - 类型强制读取，带回退值
- `env_required(key)` - 如果键缺失或解析失败则触发 panic。仅在启动时使用（在 `bootstrap()` 或 `config::register()` 中），缺少必需值时应该让进程立即崩溃
- `env_optional(key)` - 返回 `Option<T>`；缺失或无法解析的值返回 `None`

每个唯一的键在首次读取时会被记录一次，所以您可以审计您的应用触及的确切环境变量。

## 类型化配置结构体

对于您的应用需要读取多次的任何配置，定义一个类型化结构体并注册它。模式是：

```rust
// src/config/database.rs
use suprnova::Config;
use suprnova::config::{env, env_required, env_optional};

#[derive(Clone, Debug)]
pub struct DatabaseConfig {
    pub url: String,
    pub max_connections: u32,
    pub min_connections: u32,
    pub connect_timeout_secs: u32,
    pub logging: bool,
}

pub fn register() {
    Config::register(DatabaseConfig {
        url: env_required("DATABASE_URL"),
        max_connections: env("DB_MAX_CONNECTIONS", 10),
        min_connections: env("DB_MIN_CONNECTIONS", 1),
        connect_timeout_secs: env("DB_CONNECT_TIMEOUT", 30),
        logging: env("DB_LOGGING", false),
    });
}
```

然后在任何地方只用一行就能读取它：

```rust
let db = Config::get::<DatabaseConfig>().expect("DB config registered at boot");
println!("Pool size: {}", db.max_connections);
```

注册表由 `TypeId` 作为键，所以每个结构体只存储一次。再次使用相同类型调用 `Config::register` 会替换前一个条目 - 这对测试很方便。

### 将注册接入您的应用

脚手架的 `cmd/main.rs` 在流畅的启动管道中包含一个 `.config(…)` 步骤：

```rust
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(my_app::config::register)   // ← 这会调用您的注册函数
        .bootstrap(my_app::bootstrap::register)
        .routes(my_app::routes::register)
        .migrations::<my_app::migrations::Migrator>()
        .run()
        .await
}
```

`my_app::config::register` 通常代理到每个部分模块：

```rust
// src/config/mod.rs
pub mod database;
pub mod mail;

pub fn register() {
    database::register();
    mail::register();
}
```

### 从环境变量反序列化整个结构体

对于更大的配置，您可以通过 `serde` 直接从环境变量反序列化。
Suprnova 暴露了两个辅助函数：

```rust
use suprnova::Config;

#[derive(Clone, Debug, serde::Deserialize)]
pub struct ServerConfig {
    pub host: String,
    pub port: u16,
}

// 从环境变量读取 SERVER_HOST / SERVER_PORT
let cfg = Config::resolve_prefixed::<ServerConfig>("SERVER_")?;
```

- `Config::resolve::<T>()` - 从所有进程环境变量反序列化
- `Config::resolve_prefixed::<T>("PREFIX_")` - 仅反序列化具有给定前缀的变量（前缀在反序列化前被删除）

两者都返回 `Result<T, FrameworkError>`，所以缺少必需字段会显示为 `FrameworkError::Internal`，包含 envy 诊断而不是 panic。

## 特定环境的配置

`Environment` 枚举涵盖标准集合：

| 变体 | 识别的 `APP_ENV` 值 |
|---|---|
| `Local` | `local` |
| `Development` | `development`、`dev` |
| `Staging` | `staging`、`stage`、`stg` |
| `Production` | `production`、`prod` |
| `Testing` | `testing`、`test` |
| `Custom(String)` | 任何其他（保留您的大小写，用于 `.env.<custom>` 查询） |

常见的分支：

```rust
use suprnova::{Config, Environment};

if Config::is_production() {
    // 严格的 cookie、真实的邮件驱动程序，等等
}

if Config::is_debug() {
    // 详细的错误页面、查询日志
}

match Config::environment() {
    Environment::Production => { /* … */ },
    Environment::Staging    => { /* … */ },
    _ => { /* dev/test path */ },
}
```

`is_debug()` 在 `APP_DEBUG=true` 被显式设置时返回 `true`，或者 - 当 `APP_DEBUG` 未设置时 - 检测到的环境是
`Local`、`Development` 或 `Testing` 时返回 `true`。Production、staging 和任何未识别的自定义环境默认为 `false`。在生产环境中关闭它；它控制错误页面的详细程度和一些内部默认值。

### `APP_KEY` 在非开发环境中是必需的

在生产环境中（任何 `APP_ENV` 除了 `local`/`development`/
`testing`），Suprnova 要求 `APP_KEY` 被设置为一个有效的 32 字节
URL-safe base64 字符串。不设置就启动会失败并显示描述性错误消息 - 没有无声的回退。

如果您还没有 `APP_KEY`：

```bash
suprnova key:generate          # 打印密钥，并附一句提醒您把它加进 .env 的提示
suprnova key:generate --show   # 只打印密钥，适合 `APP_KEY=$(suprnova key:generate --show)` 这样用
```

这两种形式都不会为您编辑 `.env` - 请自己将打印的密钥复制到您的
`.env`（或您的密钥管理器）中。

关于密钥轮换（在迁移窗口期间旧加密数据仍需解密），请参阅 [加密](encryption.md#key-rotation)。

## 测试中的配置

在测试中，在测试设置中注册配置而不是依赖
`.env`：

```rust
use suprnova::suprnova_test;

#[suprnova_test]
async fn test_with_custom_db() {
    suprnova::Config::register(DatabaseConfig {
        url: "sqlite::memory:".to_string(),
        max_connections: 1,
        min_connections: 1,
        connect_timeout_secs: 5,
        logging: false,
    });

    // … 您的测试
}
```

`#[suprnova_test]` 属性还设置了隔离的容器状态，所以并发测试不会看到彼此的绑定 - 参见
[测试](testing.md)。

## Suprnova 读取的常见环境变量

一个非穷尽的列表 - 这些是框架本身查看的变量。您的应用在此基础上读取更多。

| 变量 | 默认值 | 作用 |
|---|---|---|
| `APP_NAME` | `"app"` | 在启动时记录，用于某些默认错误消息 |
| `APP_ENV` | `local` | 驱动 `Environment::detect` 和 `.env.<suffix>` 查询 |
| `APP_DEBUG` | 环境相关（生产环境为 `false`） | 详细错误页面+额外日志 |
| `APP_URL` | `http://localhost:8765` | 用于绝对 URL 生成、签名 URL 的基础 URL |
| `APP_KEY` | 无（生产环境必需） | `Crypt`、会话、游标的 AES-256 密钥 |
| `APP_KEY_PREVIOUS` | 无 | 逗号分隔的旧密钥用于轮换（最多 8 个） |
| `SERVER_HOST` | `127.0.0.1` | 绑定地址 |
| `SERVER_PORT` | `8765` | 绑定端口 |
| `DATABASE_URL` | 无 | 如果您的应用使用数据库则必需 |
| `DB_MAX_CONNECTIONS` | `10` | sqlx 池最大连接数 |
| `DB_MIN_CONNECTIONS` | `1` | sqlx 池最小连接数 |
| `DB_CONNECT_TIMEOUT` | `30`（秒） | sqlx 池连接超时 |
| `SESSION_LIFETIME` | `120`（分钟） | 会话过期时间 |
| `SESSION_TOUCH_INTERVAL` | `300`（秒） | 最小滑动过期写入间隔 |
| `SESSION_GC_INTERVAL` | `3600`（秒） | 监督过期会话清理间隔 |
| `SESSION_COOKIE` | `suprnova_session` | Cookie 名称 |
| `SESSION_SECURE` | `true` | 设置 `Secure` cookie 标志。在本地 HTTP 开发中覆盖为 `false`。 |
| `SESSION_SAME_SITE` | `Lax` | `Strict`、`Lax` 或 `None` |
| `MAIL_DRIVER` | `log` | `smtp`、`ses`、`mailgun`、`postmark`、`sendgrid`、`resend`、`log`、`memory` 之一 |
| `CACHE_DRIVER` | `memory` | `memory`、`redis`、`database` 之一 |
| `QUEUE_DRIVER` | `memory` | `memory`、`redis`、`database` 之一（未知值会警告并回退到 `memory`） |
| `RATE_LIMIT_DRIVER` | `memory` | `memory`、`redis` 之一 |
| `LOG_FORMAT` | 环境相关（开发/本地为 `pretty`，生产环境为 `json`） | `pretty` 或 `json` |
| `LOG_LEVEL` | `info` | `error`、`warn`、`info`、`debug`、`trace` 之一 |

完整的审计列表在 [环境变量](env-vars.md) 中。

## 下一步

- [应用启动](bootstrap.md) - 类型化配置注册的调用位置
- [服务容器](container.md) - 如何与绑定的服务一起读取注册的配置
- [环境变量](env-vars.md) - 完整参考列表
- [部署](deployment.md) - 生产环境设置
