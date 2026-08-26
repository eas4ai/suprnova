# 环境变量

这是 Suprnova 框架在运行时读取的每一个环境变量的审计清单，按查阅它的子系统分组。每一条都已经对照框架源码核实过 - 默认值、类型和行为反映的是代码实际的所作所为，而不是起始 `.env` 恰好携带的东西。

这份清单也涵盖了 `suprnova` CLI 二进制文件读取的变量（开发服务器、SSR 工作进程），因为它们出现在起始 `.env` 里，读者也会到这里来找它们。

加载规则（`.env` → `.env.<environment>` → 进程环境）、`env*` 系列辅助函数（`env`、`env_required`、`env_optional`），以及类型化的 `Config::*` 注册模式，请参见[配置](configuration.md)。

## 约定

- **默认值** - 变量未设置时框架使用的值。`none` 表示没有默认值；框架会在启动时报错、回退到一个 feature 默认值（例如 `Memory` 驱动程序），或者把这个值当作 `None`。
- **类型** - 这个变量被解析成的 Rust 类型。`bool` 值接受 `true`/`false`/`1`/`0`/`yes`/`no`/`on`/`off`（不区分大小写）。对于类型化的框架旋钮，超出范围或无法解析的值会被夹紧（工作流），记一条 `warn!` 后取默认值（宽松的 `env()` / `env_optional()`），或者让启动失败（严格的 `try_from_env`）。
- **必需** - `boot` 意味着框架在列出的环境里，没有它就拒绝启动。`driver` 意味着只有在选中了父驱动程序时才需要它（例如除非 `MAIL_DRIVER=ses`，否则 `MAIL_SES_REGION` 与此无关）。其余的一切都是可选的。

如果起始 `.env` 携带了一个框架从不读取的键（`MAIL_FROM_ADDRESS`、`FILESYSTEM_DISK`），会在本章末尾单独点出来。

## 应用

`APP_*` 这一族是框架的身份标识和加密根。这些是每一个 Suprnova 应用都会设置的变量；本文件其余的部分会随着您选择接入各个子系统而变得相关。

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `APP_NAME` | `"Suprnova Application"` | `String` | 应用名称。用作 TOTP 签发者（2FA）、HTTP Basic 的 `WWW-Authenticate` realm、邮件主题的品牌署名，以及结构化日志字段。 |
| `APP_ENV` | `local` | `String` | 驱动 `Environment::detect()` 和 `.env.<suffix>` 查找。可识别的别名（不区分大小写）：`local`、`development`/`dev`、`staging`/`stage`/`stg`、`production`/`prod`、`testing`/`test`。任何其他值都会以原始大小写保留为 `Environment::Custom(...)`。 |
| `APP_DEBUG` | 能感知环境（见“必需”列） | `bool` | 详细的错误页面 + 额外的日志。在 `local`/`development`/`testing` 里默认为 `true`，其他所有地方（包括 `staging`、`production`，以及任何未识别的自定义环境）默认为 `false`。一个显式值总是胜出；一个无法解析的值会带着一条 `warn!` 回退到那个能感知环境的默认值。严格的 `try_from_env` 变体会在解析失败时中止启动。 |
| `APP_URL` | `"http://localhost:8765"`（AppConfig）/ `"http://localhost"`（URL 回退值） | `String` | 用于绝对 URL 生成、签名 URL，以及 Inertia 重定向的基础 URL。读取时会去掉结尾的斜杠。 |
| `APP_KEY` | 无 - 非开发环境必需 | `String`（base64-url-无填充，32 字节） | 供 `Crypt`、加密会话、分页游标、签名 URL，以及任何其他静态加密路径使用的 AES-256-GCM 密钥。在 `local`/`development`/`testing` 之外缺失或格式错误时，启动会**失败关闭**。用 `suprnova key:generate` 生成。 |
| `APP_KEY_PREVIOUS` | 无 | `String`（逗号分隔的 base64 密钥，最多 8 个） | 轮换期间使用的、逗号分隔的旧密钥。`Crypt::decrypt` 会先尝试当前的 `APP_KEY`，然后按顺序尝试每一项。硬性上限是 8 项 - `crypto::MAX_PREVIOUS_KEYS`。一个解码失败的、半轮换的条目会中止启动。参见[加密](encryption.md#key-rotation)中的“密钥轮换 - 密钥环”一节。 |
| `APP_PREVIOUS_KEYS` | 无 | `String`（`APP_KEY_PREVIOUS` 的别名） | 为兼容 Laravel 而接受的别名，这样一份被丢进 Suprnova 部署里的 Laravel `.env`，依然能优雅地解密遗留数据。当两者都被设置为不同的值时，`APP_KEY_PREVIOUS` 胜出，并带一条 `warn!` 把这个重复暴露出来；相同的值会被静默接受。 |
| `APP_BASE_PATH` | 当前工作目录 | `Path` | 路径解析器用来定位 `config/`、`database/`、`public/`、`storage/`、`resources/`、`lang/` 的根目录。在从一个不同于项目根目录的 CWD 运行这个二进制文件时很有用（例如一个 systemd unit，其 `WorkingDirectory=` 没有指向这个项目）。回退到 CWD，如果 CWD 不可用则回退到 `.`。 |
| `APP_TRUSTED_PROXIES` | 无 - 空的允许列表 | `String`（逗号分隔的 IP） | `Request::ip()`，以及 host / scheme / port 访问器可以信任其 `X-Forwarded-*` / `X-Real-IP` 请求头的那些 TCP 对端地址。**默认为空，所以代理请求头会被忽略，TCP 对端始终胜出** - 在部署到代理之后之前，请看下面的说明。一个无法解析的条目会让启动失败（`try_from_env`）。 |
| `AUTH_GUARD` | `"web"` | `String` | 由 `Auth::*` 读取的默认认证守卫的名字。镜照 Laravel - 只有默认值是可以通过环境变量选择的；具名的认证守卫通过 `AuthConfig::guard(name, …)` 活在代码里。 |

另外两个 `APP_*` 变量 - `APP_LOCALE` 和 `APP_FALLBACK_LOCALE` - 是由本地化子系统而不是 `AppConfig` 读取的，所以列在下面的**本地化**一节里。

### 在反向代理之后，请设置 `APP_TRUSTED_PROXIES`

忽略代理请求头是安全的默认值 - `X-Forwarded-For` 是调用方提供的，无条件信任它，就等于让任何人都能冒领任何地址。但一旦有一个终止连接的代理挡在您前面（nginx、Traefik、一个 ALB、Cloudflare），TCP 对端在每一个请求上就*是那个代理*，而不设置这个变量的后果，不仅仅是丢掉客户端的地址：

- **按 IP 的速率限制会塌缩进同一个桶。** `ThrottleRequestsMiddleware` 的默认键是 `request.ip()`，所以 `ThrottleRequestsMiddleware::with(20, 1, "login")` 不再意味着“每个客户端每分钟 20 次登录尝试”，而是变成*所有人加在一起总共* 20 次。这不仅更弱（没有按攻击者区分的预算），还很危险：任何单个调用方都能耗尽这个配额，把每一个合法用户都锁在登录表单之外。参见[速率限制](rate-limiting.md)。
- `Request::host()`、`scheme()` 和 `port()` 会回退到这条连接本身，而不是 `X-Forwarded-Host` / `-Proto` / `-Port`，所以生成出来的绝对 URL 可能会写着内部地址和协议，而不是公开的那一个。

列出代理跳转到达您这里所使用的地址 - 不是客户端的地址：

```bash
APP_TRUSTED_PROXIES=10.0.0.5,10.0.0.6
```

没有任何东西会替您检测这一点：一个在代理之后、却没有设置这个变量的应用看起来很健康，正常地提供服务，同时又悄悄地把所有人都当成同一个用户来做速率限制。

### `APP_KEY` 必需性矩阵

| 环境 | 启动时是否需要 `APP_KEY` |
|---|---|
| `local` | 不需要（缺失时会生成一个临时密钥） |
| `development` | 不需要 |
| `testing` | 不需要 |
| `staging` | 需要 - 启动会以非零状态退出，并带一条修复提示 |
| `production` | 需要 |
| `Custom(...)` | 需要 - 任何不在安全列表里的东西，在这项检查里都会被当作生产环境对待 |

## 服务器

HTTP 监听器和请求体的限制。

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `SERVER_HOST` | `"127.0.0.1"` | `String` | 绑定地址。设为 `0.0.0.0` 可以暴露到回环接口之外（例如在容器里）。 |
| `SERVER_PORT` | `8765` | `u16` | 绑定端口。宽松解析会警告并取默认值；严格的 `try_from_env` 会在打错字时中止启动。 |
| `SERVER_MAX_BODY_SIZE` | `8388608`（8 MiB） | `usize`（字节） | 进程全局的最大请求体大小。逐 `FormRequest::max_body_bytes` 的覆盖在单个端点上仍然适用。这个配置值会在 `Server::from_config` 期间被接入这个全局上限。 |
| `SERVER_MAX_CONNECTIONS` | 未设置（无界） | `usize` | 并发活跃 TCP 连接数的上限。未设置意味着没有上限。一个为零或者无法解析的值，会带着一条警告回退到一个有限值 `10000`，而不是悄悄地退回无界状态 - 一个搞砸了的限制仍然是想要一个限制的请求。 |
| `SERVER_HEADER_READ_TIMEOUT` | `30` | `u64`（秒） | 读取一个请求完整请求头的截止时间。这是应对 slowloris 攻击的缓解措施。零会被当作无效值而不是“禁用”，并会回退到默认值。不适用于已经建立的 WebSocket/SSE 连接。 |
| `SERVER_HEALTH_READINESS_TOKEN` | 未设置（就绪性为公开） | `String` | 到达 `/_suprnova/health/ready` 和 `/_suprnova/health?db=true` 所需的共享密钥，以 `X-Suprnova-Health-Token` 的形式发送。没有它，这些路径会回答 404，和任何未路由的路径没有区别；存活性探针则始终保持公开。参见[部署](deployment.md#health-check)。 |

## 数据库

连接 URL 和 sqlx 连接池调优。任何触碰数据库的子命令（`migrate*`、`db:sync`、`db:seed`、带 `QUEUE_DRIVER=database` 的 `queue:work`、`workflow:work`、会话的数据库存储），以及在应用注册了迁移时的 `serve`，都需要 `DATABASE_URL`。

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `DATABASE_URL` | 无 - 存在迁移时必需 | `String` | 连接 URL。协议头选择驱动程序：`sqlite://path`、`postgres://...` / `postgresql://...`、`mysql://...`、`mariadb://...`。框架会为 SQLite 路径自动创建父目录。当配置的 `Migrator` 没有任何迁移时，`serve` 会完全跳过数据库连接。 |
| `DB_MAX_CONNECTIONS` | `10` | `u32` | sqlx 连接池的上限。 |
| `DB_MIN_CONNECTIONS` | `1` | `u32` | sqlx 连接池的下限（保持热连接）。 |
| `DB_CONNECT_TIMEOUT` | `30`（秒） | `u32` | sqlx 在报错之前，会为一次初始连接等待多久。 |
| `DB_LOGGING` | `false` | `bool` | 为 true 时，sqlx 会记录每一条语句（在生产环境里请谨慎使用 - 很啰嗦）。 |
| `SUPRNOVA_AUTO_MIGRATE_BEST_EFFORT` | `false` | `bool` | 为 true 时，`serve` 启动期间一次失败的自动迁移会被记录下来，但不会中止启动。默认是失败关闭：启动会以非零状态退出，而不是针对一个只迁移了一部分的架构启动。传入 `--no-migrate` 可以完全跳过自动迁移。 |

## 会话

会话子系统的 cookie 属性和生存期。注意 `SESSION_SECURE` 默认为**`true`** - 默认就对生产环境安全；只有在本地 HTTP 开发时才把它关掉。

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `SESSION_LIFETIME` | `120`（分钟） | `u64` | 会话生存期，以分钟计。通过 `env_optional` 解析；无法解析时会静默回退。 |
| `SESSION_TOUCH_INTERVAL` | `300`（秒） | `u64` | 滑动过期持久化的最小节奏。运行时的强制措施会把它限制在会话生存期的一半以内。 |
| `SESSION_GC_INTERVAL` | `3600`（秒） | `u64` | 由 `SessionMiddleware::install` 装上的、受监督的过期会话收集器的运行节奏。 |
| `SESSION_COOKIE` | `"suprnova_session"` | `String` | 会话 cookie 的名字。 |
| `SESSION_PATH` | `"/"` | `String` | Cookie 的 `Path=` 属性。 |
| `SESSION_DOMAIN` | 未设置 | `String` | Cookie 的 `Domain=` 属性。留空以得到仅限主机的 cookie（对大多数应用来说更安全的默认值）。 |
| `SESSION_SECURE` | `true` | `bool` | Cookie 的 `Secure` 属性。默认为 `true`；只有在本地 HTTP 开发时才设为 `false`。`cookie_http_only` 始终为 `true`，且不能通过环境变量配置。 |
| `SESSION_SAME_SITE` | `"Lax"` | `String` | `SameSite` 属性。接受 `Strict`、`Lax`、`None`（不区分大小写）。 |
| `SESSION_COOKIE_PREFIX` | 未设置 | `String`（`__Host-` / `__Secure-`） | 应用于会话和记住我传输名称的前缀。`Config::init` 会在启动时校验该值以及 `SESSION_DOMAIN` / `SESSION_PATH` 约束；无效组合会在开始提供服务前失败。 |
| `SESSION_PARTITIONED` | `false` | `bool` | 为第三方隔离的 cookie 发出 `Partitioned` / CHIPS 这个 cookie 属性。 |
| `SESSION_EXPIRE_ON_CLOSE` | `false` | `bool` | 为 true 时，去掉 `Max-Age`，这样浏览器就会在关闭时删除这个 cookie（会话 cookie 语义）。 |
| `SESSION_CONNECTION` | 未设置 | `String` | 会话存储所用的具名数据库连接。未设置意味着使用默认连接。 |
| `REMEMBER_LIFETIME` | `43200`（30 天，以分钟计） | `u64` | “记住我” cookie/令牌的生存期，以分钟计。 |

## 本地化

本地化子系统读取的三个 `APP_*` 变量。关于它的其余一切 - 检测链、它查阅的会话键和 cookie 名字、Unicode 隔离标记 - 都是 `LocalizationConfig` 上的代码级配置，不是环境变量。参见[本地化](localization.md)。

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `APP_LOCALE` | `"en"` | `String`（BCP-47） | 当检测链（会话 → cookie → `Accept-Language`）什么都没找到时使用的语言区域。也是 `suprnova generate-types` 为 `lang-keys.ts` 提取消息键时所用的那个语言区域。一个不是有效 BCP-47 标识符的值会让启动失败，而不是静默地取默认值。 |
| `APP_FALLBACK_LOCALE` | `"en"` | `String`（BCP-47） | 当一个键在当前语言区域的语料表里缺失时，会去查阅的语言区域。一个在两者里都缺失的键，会连同一次性的 `warn!` 一起，渲染成这个键本身；`Lang::try_get` 则会返回 `Err`。和 `APP_LOCALE` 一样的严格解析。 |
| `APP_LOCALE_PARENTS` | 无 - 空映射 | `String`（逗号分隔的 `child=parent` 对，两侧都是 BCP-47） | 在 `APP_FALLBACK_LOCALE` 之前被查阅的、逐语言区域的回退父级，例如 `APP_LOCALE_PARENTS=pt-PT=pt-BR,en-AU=en-GB`。`Lang` 的回退链会传递地遍历这些父级，`FluentTranslator` 会把每个语言区域配置好的父级链，压平进它所服务的那份语料表里。一对格式错误的配对、一个无效的语言区域、一个被命名了不止一次的子级，或者一个环（包括一个语言区域把自己命名为自己的父级），都会让启动失败，而不是在请求时才退化。参见[回退链](localization.md#fallback-chains)。 |

语料表本身是文件，不是环境变量：`APP_BASE_PATH` 之下的 `lang/<locale>/*.ftl`。一个缺失的 `lang/` 目录不是一个错误 - 应用会带着框架内嵌的英语校验语料表启动。

## 缓存

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `CACHE_DRIVER` | `memory` | `String`（`memory`/`in-memory`/`inmemory`、`redis`） | 选择启动目标。Memory 会把一切都保留在进程内；Redis 需要 `REDIS_URL`，如果无法连通就会让启动失败。未知的值会带着一条清楚的错误让启动失败。 |
| `REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | Redis 连接 URL（只有在 `CACHE_DRIVER=redis` 时才会被查阅）。 |
| `REDIS_PREFIX` | `"suprnova_cache:"` | `String` | 缓存条目的键前缀（用于在共享 Redis 上避免冲突）。 |
| `CACHE_DEFAULT_TTL` | `3600`（秒） | `u64` | 默认 TTL，以秒计。`0` 意味着“永不过期”。应用于 `Cache::put(None)` / `Cache::tags_put(None)`；`Cache::forever` 和 `Cache::remember_forever` 总是绕过它。 |

## 队列

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `QUEUE_DRIVER` | `memory` | `String`（`memory`、`redis`、`database`、`failover`） | 当前生效的队列后端。未知的值会记一条 `warn!` 并回退到内存。`failover` 会包住其余几种的一个有序列表 - 参见 `QUEUE_FAILOVER_CONNECTIONS`。 |
| `QUEUE_FAILOVER_CONNECTIONS` | - | `String`（逗号分隔，例如 `redis,database`） | 供 `QUEUE_DRIVER=failover` 使用的、按优先级排序的连接列表。选中那个驱动程序时必须给出；缺失或为空白的值是一个启动错误，一个点名 `failover` 的条目（不允许嵌套）或者点名一个并不存在的驱动程序的条目也是。每一项都读它自己那个驱动程序的变量。只有推送会沿着这个列表往下穿；每一次读取和每一次确认都走第一个连接，所以每一个后备连接都需要它自己的工作进程。 |
| `QUEUE_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | Redis URL（当 `QUEUE_DRIVER=redis` 时，由驱动程序要求必需）。 |
| `QUEUE_REDIS_STREAM` | `"suprnova-queue"` | `String` | 用于扇出的 Redis Stream 键。 |
| `QUEUE_REDIS_GROUP` | `"default"` | `String` | 消费者组的名字。 |
| `QUEUE_REDIS_CONSUMER` | `"consumer-1"` | `String` | 组内的消费者名字。并行的工作进程请逐个设置。 |
| `QUEUE_VISIBILITY_TIMEOUT_SECS` | `60` | `u64` | 一个已被认领的作业在另一个消费者可以重新认领它之前，保持不可见的时长。请把它对齐到您最慢的那个作业。 |
| `QUEUE_DB_TABLE` | `"jobs"` | `String` | 数据库驱动程序所用的表名。会作为一个 SQL 标识符被校验 - 一个格式错误的值会在启动时失败，而不是在 SQL 拼装的时候。当 `QUEUE_DRIVER=database` 时，由驱动程序要求必需；这个驱动程序还要求 `DB::init()` 已经先跑过。 |
| `QUEUE_FAILED_DB_TABLE` | `"failed_jobs"` | `String` | 死信存储写入的那张表。当 `QUEUE_DRIVER=database` 时会自动绑定 - `queue:retry` 会读它，`Queue::retry_failed` 需要它，所以这张表是那个驱动程序契约的一部分。`memory`（按构造就是易失的）和 `redis`（没有表可写）都不用它。和 `QUEUE_DB_TABLE` 不同，这里一个格式错误的标识符**不会**让启动失败：它会以 `error!` 记进日志，并且不绑定任何存储，于是被死信的作业会被完整地记进日志，而不是被持久化。可以手工恢复，但没法通过 `queue:retry` 恢复。 |

## 调度

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION` | 未设置 | 类 `bool` | 确认您知道一个标记了 `on_one_server()` 的任务，是在通过一个**逐进程**的缓存来选举一个 leader。这次选举的共享程度，取决于它背后的缓存有多共享，所以在生产环境里，`CACHE_DRIVER=memory` 加上一个单服务器任务，是一次会点名违规任务的、硬性的启动失败，而不是悄悄退化成“每个副本都跑它”。只有在这个部署真的只跑一个调度器时，才设置这个变量；否则请设置 `CACHE_DRIVER=redis`。参见[任务调度](scheduling.md)。 |

## 工作流

`#[workflow]` 这个长时间运行的有状态工作进程。所有的值都会被夹紧到安全的最小值，而不是被盲目地照单全收 - 一个 `WORKFLOW_CONCURRENCY=0` 会让工作进程的信号量永远停摆，所以框架会发出警告并夹紧这个值，而不是接受一份明显破损的配置。

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `WORKFLOW_CONCURRENCY` | `4` | `usize` | 每个工作进程的最大并发工作流执行数。被夹紧到 `>= 1`。 |
| `WORKFLOW_POLL_INTERVAL_MS` | `1000`（毫秒） | `u64` | 工作进程轮询新到期工作流的频率。 |
| `WORKFLOW_LOCK_TIMEOUT_SECS` | `30`（秒） | `u64` | 一个工作进程已经死掉的、被认领的工作流行的回收超时。 |
| `WORKFLOW_MAX_ATTEMPTS` | `3` | `i32` | 每次工作流运行在被标记为失败之前的最大尝试次数。被夹紧到 `>= 1`。 |
| `WORKFLOW_RETRY_BACKOFF_SECS` | `5` | `i64` | 每次尝试之间的线性退避。被夹紧到 `>= 0` - 负的退避会把重试安排到过去，并产生一个紧密循环的回收。 |

## 邮件
`MAIL_DRIVER` 默认为**`log`** - 发出的邮件会打印到已配置的 tracing 订阅者，而不会触达网络。在测试里把它翻转成 `memory`，把 `file` 用于可在邮件客户端中打开的 `.eml` 预览，在生产环境里翻转成 `smtp`/`ses` 等等。特定于提供者的密钥/令牌只有在选中了那个驱动程序时才是必需的；一个未知的驱动程序值会记一条 `warn!`，并回退到 `log`。

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `MAIL_DRIVER` | `"log"` | `String`（`log`、`memory`、`file`、`smtp`、`ses`、`sendgrid`、`mailgun`、`postmark`、`resend`） | 选择启动目标。 |
| `MAIL_FROM` | 无 - 认证流程门面必需 | `String` | 认证流程门面（`EmailVerification`、`PasswordReset`、`TwoFactor`）所用的默认发件地址。这些路径都需要它；缺失时会在调用点报错，而不是静默地回退到一个会破坏 DMARC/SPF 的占位符。 |
| `MAIL_FROM_NAME` | 未设置 | `String` | 认证流程 `From` 的可选显示名（自 **0.5.9** 起）。设置了它时，这个请求头会渲染成 `Name <MAIL_FROM>`；`MAIL_FROM` 仍然是一个裸地址。在发送时读取，所以对排入队列的认证流程邮件也一样适用。 |

### File（`MAIL_DRIVER=file`）

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `MAIL_FILE_PATH` | `storage_path("mail")` | `String` | 每次发送写入一个 RFC 5322 `.eml` 文件的目录。永不自动清理。绝对路径按给定值使用；相对路径以应用基础目录为锚点（参见 `APP_BASE_PATH`）。 |

### SMTP（`MAIL_DRIVER=smtp`）

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `MAIL_SMTP_HOST` | `"127.0.0.1"` | `String` | SMTP 主机。 |
| `MAIL_SMTP_PORT` | `587` | `u16` | SMTP 端口。 |
| `MAIL_SMTP_USER` | 未设置 | `String` | SMTP 用户名。要得到一条加密的传输，`MAIL_SMTP_USER`**和** `MAIL_SMTP_PASS` 必须都被设置；两者都没有时，这条连接会默认走未加密的本地捕获模式。恰好只设置其中一个，会在启动时发出警告。 |
| `MAIL_SMTP_PASS` | 未设置 | `String` | SMTP 密码。部分凭据的行为，请参见 `MAIL_SMTP_USER`。 |
| `MAIL_SMTP_ENCRYPTION` | 推导得出 | `starttls` \| `tls` \| `none` | 这条连接是如何被加密的。未设置时，会从凭据推导：两者都设置了就是 `starttls`，两者都没有就是 `none`。`tls` 选择隐式 TLS（端口 465）。`ssl` 和 `null` 会被当作与 Laravel 兼容的别名接受。一个不认识的值，在**每一个**环境里都会让启动失败 - 一次打字错误绝不能退化成明文传输。 |
| `MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION` | 未设置 | 类 `bool` | 生产环境会拒绝在一条未加密的 SMTP 连接上启动。设为 `1`/`true`/`yes`/`on` 来确认接受明文传输 - 只有在这个中继只能通过一个私有网络到达时，这才是可以辩护的。 |

### Postmark（`MAIL_DRIVER=postmark`）

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `MAIL_POSTMARK_TOKEN` | 由驱动程序要求必需 | `String` | Postmark 服务器令牌。 |
| `MAIL_POSTMARK_ENDPOINT` | Postmark 默认值 | `String` | 覆盖这个 API 端点（区域性端点或者模拟服务器）。 |

### Amazon SES（`MAIL_DRIVER=ses`）

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `MAIL_SES_ACCESS_KEY` | 由驱动程序要求必需 | `String` | AWS 访问密钥。 |
| `MAIL_SES_SECRET_KEY` | 由驱动程序要求必需 | `String` | AWS 私有访问密钥。 |
| `MAIL_SES_REGION` | `"us-east-1"` | `String` | AWS 区域。 |
| `MAIL_SES_ENDPOINT` | 该区域的 AWS 默认值 | `String` | 覆盖这个 SES 端点（区域性端点或者模拟服务器）。 |

### SendGrid（`MAIL_DRIVER=sendgrid`）

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `MAIL_SENDGRID_API_KEY` | 由驱动程序要求必需 | `String` | SendGrid API 密钥。 |
| `MAIL_SENDGRID_ENDPOINT` | SendGrid 默认值 | `String` | 覆盖这个 API 端点。 |

### Mailgun（`MAIL_DRIVER=mailgun`）

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `MAIL_MAILGUN_API_KEY` | 由驱动程序要求必需 | `String` | Mailgun API 密钥。 |
| `MAIL_MAILGUN_DOMAIN` | 由驱动程序要求必需 | `String` | Mailgun 发信域名。 |
| `MAIL_MAILGUN_ENDPOINT` | Mailgun 默认值 | `String` | 覆盖这个 API 端点（例如欧盟对美国）。 |

### Resend（`MAIL_DRIVER=resend`）

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `MAIL_RESEND_API_KEY` | 由驱动程序要求必需 | `String` | Resend API 密钥。 |
| `MAIL_RESEND_ENDPOINT` | Resend 默认值 | `String` | 覆盖这个 API 端点。 |

## 速率限制

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `RATE_LIMIT_DRIVER` | `memory` | `String`（`memory`、`redis`） | 选择速率限制器的后端。在生产环境之外，一个未知的值会记一条 `warn!` 并回退到 memory；**在生产环境里，memory - 包括通过一个未知值得到的 memory - 都会让启动失败**，除非设置了 `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION`。 |
| `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION` | 未设置 | 类 `bool` | 确认您接受生产环境里逐进程的速率限制桶。只有在您刚好运行一个进程时才准确：在 N 个副本之后，每一份配额实际上都变成了 N 倍，并且在每一次部署时都会重置。 |
| `RATE_LIMIT_REDIS_URL` | `"redis://127.0.0.1:6379"` | `String` | Redis URL（当 `RATE_LIMIT_DRIVER=redis` 时，由驱动程序要求必需）。 |
| `RATE_LIMIT_PREFIX` | `"suprnova:"` | `String` | Redis 里的键前缀。 |

## 图像

图像驱动程序的选择，以及那些为敌意输入设界的解码限制。超出范围的限制会带一条 `warn!` 被夹紧，而不是让启动失败：一个为零的限制会拒绝掉这个应用里的每一张图像。一个未知的 `IMAGE_DRIVER` 会在第一次使用时失败，并点名那些合法的值。

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `IMAGE_DRIVER` | `oxideav` | `String`（`oxideav`、`magick`） | 选择图像后端。`oxideav` 是纯 Rust 的，没有宿主依赖；`magick` 会去调用一个宿主上安装好的 ImageMagick 7，以换取更宽的输入支持。不区分大小写。 |
| `IMAGE_MAX_DIMENSION` | `16384` | `u32` | 一张被解码图像的宽和高的上限，会在分配任何东西之前，对着输入自己的文件头做检查。它同样也给缩放目标设上限。最小值 `1`。 |
| `IMAGE_MAX_ALLOC_BYTES` | `268435456`（256 MiB） | `u64` | 解码后 RGBA 占用（`width * height * 4`）的上限。它同样也给源文件本身的大小设上限，不管它来自一个路径、一个磁盘，还是 `Image::from_stream`（后者会在收集的过程中就检查）。最小值 `4`。 |
| `IMAGE_MAGICK_BINARY` | `magick` | `String` | `magick` 驱动程序调用的那个二进制文件。只支持 ImageMagick 7；不接受 ImageMagick 6 的 `convert` 这个名字。二进制文件缺失会在第一次使用时给出一个明确的错误。 |
| `IMAGE_MAGICK_TIMEOUT_SECS` | `30` | `u32` | 一次 ImageMagick 调用的挂钟时间上限。它既是 ImageMagick 自己的 `-limit time` 参数，也是 Rust 这一侧的截止时间 - 后者会在两秒之后杀掉子进程的整个进程组，因为 `-limit time` 是由一个监视器来执行的，而一个卡死在某个委托里的子进程永远不会让它介入。它给一个卡住的委托设了界，否则那个委托会在整个进程的生命周期里一直占着一个阻塞工作线程。仅 `magick` 驱动程序。最小值 `1`。 |

关于这两层限制是怎么执行的，以及该如何在两个驱动程序之间做选择，请参见[图像](images.md)。

## 哈希

密码哈希驱动程序和逐算法的参数。无效的值会在第一次哈希时返回一个 `FrameworkError::param`，让配置错误立刻暴露出来，而不是静默地取默认值。

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `HASH_DRIVER` | `bcrypt` | `String`（`bcrypt`、`argon`/`argon2i`、`argon2id`） | 活跃的哈希算法。不区分大小写。 |
| `HASH_ROUNDS` | `12` | `u32` | Bcrypt 的成本（范围 `4..=31`）。超出范围的值会带着一条清楚的错误失败。 |
| `HASH_MEMORY` | `65536`（64 MiB，以 KiB 为单位） | `u32` | Argon2 的内存，以 KiB 计。最小值 `8`。仅限 Argon。 |
| `HASH_TIME` | `4` | `u32` | Argon2 的时间/迭代次数。最小值 `1`。仅限 Argon。 |
| `HASH_THREADS` | `1` | `u32` | Argon2 的并行度（匹配 OWASP / libsodium）。最小值 `1`。仅限 Argon。 |
| `HASH_VERIFY` | `false` | `bool` | 为 true 时，`verify()` 会拒绝来自一个与 `HASH_DRIVER` 不同算法的哈希（返回 `Ok(false)`）。默认为 `false`，这样遗留的 bcrypt 哈希在驱动程序翻转之后，直到被轮换之前，依然能通过校验。 |

## 验证

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `HIBP_TIMEOUT_SECS` | `30`（秒） | `u64` | `Password::uncompromised()` 那次 Have I Been Pwned range 检查的请求超时时间，每次构造一个默认的 `HibpVerifier` 时都会重新读取。一个缓慢或者不可达的 HIBP 仍然会失败开放 - 参见[验证](validation.md)。 |

## 认证流程

双因素认证用 `APP_NAME`（在“应用”一节里有覆盖）作为 TOTP 签发者字符串 - 没有一个专门的 `2FA_ISSUER` 环境变量。当 `APP_NAME` 未设置时，这个签发者会回退到 `"Suprnova"`。

## Inertia / 前端

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `SUPRNOVA_FRONTEND` | `svelte` | `String`（`svelte`、`react`、`vue`） | 活跃的前端。不区分大小写。驱动 `Frontend::detect_from_env()`、默认的 Vite 入口点，以及编译期的页面组件扩展名搜索顺序。未知或未设置的值会回退到 `svelte`。 |

## 维护模式

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `MAINTENANCE_DRIVER` | `file` | `String`（`file`、`cache`） | 选择 `down`/`up` 状态如何被存储。`file` 会写入框架的存储路径；`cache` 则搭在已配置的缓存驱动程序上（在许多应用实例必须协调维护状态时很有用）。任何其他值都会回退到 `file`。 |

## 事件

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `EVENT_MAX_CONCURRENCY` | `256` | `usize` | 并发排队监听器任务数的上限。`<= 0` 或无法解析的值会回退到默认值。适用于 `Event::queue` / 排队的监听器；同步监听器不受这个限制约束。 |

## 日志

`LOG_FORMAT` **能感知环境**：在生产环境里（`APP_ENV=production`），默认值是 `json`，对日志聚合器友好；其他所有地方，默认值都是 `pretty`，方便本地/开发环境的人类可读输出。显式值总是胜出。

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `LOG_LEVEL` | `"info"` | `String`（`error`、`warn`、`info`、`debug`、`trace` - 不区分大小写） | tracing-subscriber 的过滤级别。 |
| `LOG_FORMAT` | 能感知环境（生产环境为 `json`，其他地方为 `pretty`） | `String`（`json`、`pretty`） | tracing-subscriber 的输出格式。 |

## 可观测性（OpenTelemetry）

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | 未设置（遥测被禁用） | `String` | OTLP 采集器端点。未设置（或者是空白）时，导出器不会被安装，框架会继续使用标准的 `tracing` 订阅者。 |
| `OTEL_SERVICE_NAME` | `"suprnova"` | `String` | 每一个 span / 指标 / 日志记录上的 `service.name` 这个资源属性。 |
| `OTEL_SERVICE_VERSION` | 构建时的 `CARGO_PKG_VERSION` | `String` | `service.version` 这个资源属性。 |
| `OTEL_SDK_DISABLED` | `false` | `bool` | 标准的 OTel 总开关。为 true 时，不管 `OTEL_EXPORTER_OTLP_ENDPOINT` 是什么，导出器都不会被安装。 |

## CLI / 开发服务器

这些是由 `suprnova` CLI 二进制文件（开发服务器、SSR 工作进程）而不是运行时框架读取的 - 它们出现在起始 `.env` 里，或者由 `suprnova serve` / `suprnova ssr:*` 遵循。

| 变量 | 默认值 | 类型 | 用途 |
|---|---|---|---|
| `VITE_PORT` | `5765` | `u16` | Vite 在 `suprnova serve` 里绑定的端口。CLI 的 `--frontend-port` 会覆盖它。 |
| `SUPRNOVA_SSR_RUNTIME` | `"node"` | `String` | 用来启动这个 SSR 工作进程的运行时（`suprnova ssr:start`）。CLI 的 `--runtime` 会覆盖它。 |
| `SUPRNOVA_SSR_BUNDLE` | `frontend/bootstrap/ssr/ssr.js` | `Path` | 构建好的 SSR bundle 的路径。CLI 的 `--bundle` 会覆盖它。 |
| `SUPRNOVA_SSR_URL` | `"http://127.0.0.1:13714"` | `String` | `suprnova ssr:check` 所用的 SSR 工作进程 URL。CLI 的 `--url` 会覆盖它。 |

## 没有环境变量的子系统

有几个子系统完全是在 Rust 代码里，通过容器或者服务注册来配置的 - 框架为它们读取**零个**环境变量：

- **文件系统 / 存储。** 磁盘是在 `bootstrap()` 里用 `FilesystemRegistry::add_disk(name, driver)` 注册的。没有 `FILESYSTEM_DISK` 这个环境变量（这个名字出现在一些起始 `.env` 文件里，但框架并不会查阅它 - 见下面的“框架不读取的变量”）。
- **广播和 WebSocket。** 通道是用 `ws!()` 这个宏，以及代码里的 `BroadcastHub` 配置注册的。驱动程序本身搭在已配置的 `CACHE_DRIVER` 所选中的那个东西上。
- **CORS、CSRF、幂等性、超时。** 通过传给 `bootstrap()` 里中间件构造函数的构建器结构体来配置。默认值足够保守，一个典型的应用永远不需要去动它们。
- **Magnetar 和 OAuth。** `MagnetarConfig` 在应用 bootstrap 中构建。其 `session`、`lockout`、`passkey`、`two_factor` 和 OAuth 提供商策略，来自应用代码传给构建器的值。OAuth 客户端 ID 和密钥（`GITHUB_CLIENT_ID`、`GOOGLE_CLIENT_ID` 等）仍是*用户*配置 - 您的 bootstrap 通过 `std::env::var(...)` 读取它们，并交给已安装的 Magnetar OAuth 注册表。框架本身不会猜测或读取一组固定的 OAuth 密钥。
- **向量搜索、通知、支付、功能标志。** 每一个都在 `bootstrap()` 里通过 `App::bind` 注册具体的驱动程序。在 Rust 里选择您的驱动程序；把它需要的任何 URL/密钥，当作您自己的环境变量传进去。

## 框架不读取的变量

脚手架生成的起始 `.env` 出于人工作者的方便，列出了几个框架从不查阅的键。它们被记录在这里，这样一个搜索它们的读者就不会一头雾水：

- `MAIL_FROM_ADDRESS` - 一个 Laravel 风格的占位符，框架从不查阅它。认证流程门面实际使用的发件地址是 `MAIL_FROM`（在“邮件”一节里有覆盖）。如果您想保留这个 Laravel 名字，您自己的 `Mailable` 类型可以通过 `env_optional` 读取它，但 `suprnova::*` 里没有任何东西会这样做。（`MAIL_FROM_NAME` 自 0.5.9 起**确实**会被读取 - 见“邮件”这一章 - 所以它不再列在这里。）
- `FILESYSTEM_DISK` - 默认磁盘名字的占位符。请改用代码里的 `FilesystemRegistry::set_default(name)` 来设置默认值。

## 值是如何被解析的

三种环境变量辅助函数的简短参考 - 完整的讲解请参见[配置](configuration.md#direct-env-access)：

| 辅助函数 | 缺失时的行为 | 无法解析时的行为 |
|---|---|---|
| `env(key, default)` | 返回 `default` | `warn!` + 返回 `default` |
| `env_required(key)` | **panic** | **panic** |
| `env_optional(key)` | 返回 `None` | `warn!` + 返回 `None` |
| `env_strict(key)`（内部使用，由 `try_from_env` 调用） | 返回 `Ok(None)` | 返回 `Err(FrameworkError)` - 启动中止 |

严格的变体（`AppConfig::try_from_env`、`ServerConfig::try_from_env`）是 `Config::init` 会调用的东西，所以 `APP_DEBUG=tru` 或者 `SERVER_PORT=80a0` 这样的打字错误，会带着一个结构化的错误中止启动，而不是静默地退回默认值。宽松的变体是为那些更广泛的调用点群体（包括 `impl Default`）存在的，在那些地方一次解析失败绝不能 panic。

## 逐环境覆盖

加载器按这个顺序读取文件，每一个都会覆盖前一个：

1. `.env`
2. `.env.<environment>`（例如 `.env.production`、`.env.staging`、`.env.testing`，对于 `APP_ENV=<custom>` 则是 `.env.<custom>`）
3. 进程环境

这意味着一次容器化的生产部署，可以只携带一个精简的 `.env.production`，只覆盖那些和 `.env` 不同的键（驱动程序名字、URL、密钥材料），而真正的容器环境，会为那些绝不应该落进一个已提交文件里的密钥，同时覆盖两者。

加载器的确切行为，以及那个防止过期的 `.env` 值，在多次重新加载之间被提升进“真正的系统环境”这一层的 `LOADED_KEYS` 追踪机制，请参见[配置](configuration.md#how-env-loading-works)。

## 下一步

- [配置](configuration.md) - 类型化的 `Config::*` 注册、`env*` 系列辅助函数、环境探测
- [部署](deployment.md) - 生产环境里要设置什么
- [加密](encryption.md) - 通过 `APP_KEY_PREVIOUS` 做 `APP_KEY` 轮换
- [应用启动](bootstrap.md) - 环境驱动的启动顺序在哪里被建立
