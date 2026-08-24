# 词汇表

Suprnova 专有的术语，只在这里定义一次。如果某一章用到一个词却没有解释它，这个定义就住在这里。条目按字母顺序排列；请跟随交叉链接，去看在上下文里实际使用这个术语的那一章。

阅读这份清单其余部分时，有几条约定要记在心里：

- **Trait** 指的是一个 Rust trait - 您在某个类型上实现的一份行为契约。**门面** 指的是一个零大小的结构体，它的静态方法就是某个子系统的入口点（`Cache`、`Mail`、`Auth`、`Storage`、`Bus`、`Notify`、`Vector`、`DB`、`Schedule`、`App`）。
- **驱动程序** 指的是门面或注册表背后一个可替换的后端 - `CacheStore`、`QueueDriver`、`VectorDriver`、`RateLimiterDriver`、`MailDriver`。驱动程序在启动时通过环境变量挑选，并经由容器绑定。
- **注册表** 指的是一个进程级全局的查找表，在编译期通过 `inventory` 填充，或者在启动时通过显式注册填充 - `ConnectionRegistry`、`MiddlewareRegistry`、`InertiaRegistry`、`ChannelRegistry`、`VectorRegistry`、`SupervisorRegistry`、`PaymentProviderRegistry`、`ScopeRegistry`。

## A

### 访问器

用 `#[accessor]` 宏在一个 Eloquent 模型上声明的读侧转换。每次这个属性被读取时都会运行，返回一个从一个或多个底层列派生出来的计算值（比如从 `first_name + last_name` 派生出 `full_name`）。是[修改器](#修改器)的对偶。参见 [Eloquent - 访问器与修改器](eloquent.md#accessors-and-mutators)。

### 操作

一个可注入的服务类，封装一件业务逻辑 - 一个单一的公开方法，依赖通过 `#[injectable]` 宏注入。是 Laravel 单动作可调用对象的 Suprnova 对应物。操作会自动作为单例绑定进容器，并被处理程序、作业和其他操作解析。参见[操作](actions.md)。

### 应用

`Application::new()` 里的那个链式构建器，注册您的 config、bootstrap、路由和迁移函数，然后调用 `.run()` 来分发这个二进制文件的 CLI 子命令（`serve`、`migrate`、`queue:work` 等）。每个二进制文件一个，住在 `src/app.rs` 里。参见[请求生命周期](lifecycle.md)。

### 原子计数器

一个缓存操作（`Cache::increment`、`Cache::decrement`），以单次往返修改一个数值，不会有读-改-写竞态。在 Redis 存储上由 Redis 的 `INCR`/`DECR` 支撑，在内存存储上由一个持有的守卫支撑。参见[缓存 - 原子计数器](cache.md#atomic-counters)。

### Authenticatable

一个已认证的用户类型要实现的 trait（`get_auth_identifier() -> String`、`get_auth_password()` 等），这样守卫和中间件就能和它对话，而不需要知道具体的用户结构体是什么。参见[认证](authentication.md)。

### Authorizable

给用户类型提供 policy 入口点（`can`、`can_any`、`cannot`）的那个 trait，供[门](#门)使用。参见[授权](authorization.md)。

## B

### 退避方案

一个队列工作进程在重试一个失败作业时，两次之间等待的那串延迟。`BackoffSchedule::linear`、`BackoffSchedule::exponential`，或者一个自定义的 `Vec<Duration>`。参见[队列 - 退避方案](queues.md#backoff-schedules)。

### 队列批次

一组一起被分发、并作为一个整体被追踪的作业 - `PendingBatch::new().add(job).add(other).dispatch()` 返回持久化后的批次 id。当您想要扇出工作、并在整个批次完成时运行一个回调时很有用。参见[队列 - 已排队的批次](queues.md#queued-batches)。

### `BelongsTo`

`HasOne`/`HasMany` 的反向关系种类 - 子级持有外键，父级在另一侧。十一种 Eloquent 关系种类之一。参见 [Eloquent - 关系](eloquent.md#relationships)。

### `BelongsToMany`

一种多对多关系，经由第三个、一等公民的[中间表](#中间表)模型。`BelongsToMany<Local, Related, Pivot>` - 中间表是在类型里指名的，不是靠字符串约定合成出来的。参见 [Eloquent - 关系](eloquent.md#relationships)。

### 应用启动

您在 `Application` 构建器上注册的那个 `bootstrap_fn`，在启动时运行一次（在 config 之后，在开始服务之前）。这是您把服务绑定进[容器](#容器)、注册观察者和事件监听器、配置默认请求头等等的地方。是 Laravel 服务提供者的 Suprnova 对应物，被塌缩成了一个函数。参见[应用启动](bootstrap.md)。

### Broadcastable

当一个[事件](#事件)应该被推送给 WebSocket 订阅者、而不只是（或者除了）本地进程内监听器时，它要实现的那个 trait。是事件分发器和[广播中枢](#broadcasthub)之间的桥梁。参见[广播](broadcasting.md)。

### `BroadcastHub`

这个 trait 命名的是“把一条消息扇出给一个频道的所有 WebSocket 订阅者的那个东西” - 内存实现（`InMemoryBroadcastHub`）是默认值；sea-streamer 实现（`SeaStreamerBroadcastHub`）是跨进程的生产部署形态。参见[广播 - 跨进程扇出](broadcasting.md#multi-process-fanout)。

### Eloquent 构造器

`Model::query()` 返回的那个链式查询对象 - 在调用 `.get()`、`.first()` 或 `.paginate(...)` 之前，您在这个可链式调用的表面上构建 `where`、`order_by`、`with`、`limit` 等等。双重命名：每一个过滤方法都同时存在 Laravel 名字（`db_where`、`db_or_where`）和 Rust 原生的同义词（`filter`、`or_filter`）两种形式。参见 [Eloquent - 查询构造器](eloquent.md#query-builder--dual-api)。

### 总线命令

一个通过 `Bus::dispatch(cmd)` 分发、路由给单个已注册 `Handler<C>` 的可序列化结构体。总线命令用于那种结果应该冒泡回调用方的进程内工作 - 队列[作业](#作业)用于那种应该被持久化、并在后台重试的工作。参见[命令总线](bus.md)。

## C

### 缓存驱动程序

`Cache` 门面背后被选中的那个后端（`memory` 或 `redis`）。在启动时通过 `CACHE_DRIVER` 挑选，并经由 [CacheStore](#cachestore) trait 呈现出来。参见[缓存](cache.md)。

### `CacheStore`

定义缓存驱动程序 SPI 的那个 trait - `get`、`put`、`forget`、`increment` 等等。`InMemoryCache` 和 `RedisCache` 是已实现的实现。参见[缓存 - 配置](cache.md#configuration)。

### Eloquent 转换

用 `casts!` 在一个 Eloquent 模型上声明的双向转换 - 数据库列类型 ↔ Rust 类型。有 22 种内置实现（`AsBool`、`AsDateTime`、`AsJson`、`AsEncrypted`、`AsArray` 等等）；其他任何情形，由一个用户实现的 `Cast` trait 覆盖。参见 [Eloquent - 转换](eloquent.md#casts)。

### 队列链

一串相互链接的[作业](#作业)，每一个只有在前一个成功之后才会运行。用 `PendingChain::dispatch` / `Queue::chain` 构建。参见[队列 - 已排队的链](queues.md#queued-chains)。

### 频道（广播）

一个事件所广播到的那个 trait - `PublicChannel`、`PrivateChannel` 或 `PresenceChannel`。这个频道结构体给自己命名（`fn name() -> String`），并给连接授权（`fn authorize(...)`）；私有频道和呈现频道会附加更强的 trait 约束。参见[广播 - 频道](broadcasting.md#channels)。

### 通道（通知）

把一个 [Notification](#notification) 路由到某种投递机制的那个 trait - 邮件、数据库、广播、Web 推送。一个通知在 `fn via(...)` 里指名自己的通道；每个通道解析目的地并发送。和同名的那个广播 trait 是两回事。参见[通知 - 频道](notifications.md#channels)。

### 容器

服务通过 `App` 门面绑定和解析的那个三层（任务本地 → 线程本地 → 全局）注册表。是 Laravel 服务容器的 Suprnova 对应物，额外带着逐请求和逐测试隔离的层。参见[服务容器](container.md)。

### 逐请求上下文

一个逐请求的、由有类型值组成的包，同一个异步任务里的任何代码都能拿到它 - `Context::set::<T>(value)`、`Context::get::<T>()`。当您显式传播它时，能在任务派生之后存活下来。和同名的那个功能标志上下文是两回事。参见[上下文](context.md)。

### CORS

Cross-Origin Resource Sharing（跨源资源共享）。给一次从来源 A 到来源 B 的 JavaScript fetch 把关的浏览器安全规则；Suprnova 提供 `CorsMiddleware`，用来发出那些标明允许哪些跨源请求的响应头。参见 [CORS](cors.md)。

### CSRF

Cross-Site Request Forgery（跨站请求伪造）。一个有状态会话必须防御的攻击；Suprnova 提供 `CsrfMiddleware`，在每一个改变状态的请求上要求一个匹配的令牌。参见 [CSRF 保护](csrf.md)。

## D

### `DB` 门面

数据库那个不经过模型的入口点 - `DB::table(...)`、`DB::transaction(...)`、`DB::raw(...)`。用于那些不适合 Eloquent 形态的查询（动态列、联结聚合、原生 SQL）。参见 [Eloquent - `DB` 门面](eloquent.md#db-facade--model-less-queries)。

### 磁盘

一个通过 `Storage` 门面注册的、具名的存储后端 - `Storage::disk("s3")`、`Storage::disk("local")`。每一个磁盘都实现 [DiskExt](#diskext)，并以它的注册名字为键。参见[文件存储](filesystem.md)。

### `DiskExt`

每一个存储后端都要实现的那个 trait - `put`、`get`、`delete`、`list`、`signed_url` 等等。底层由 `opendal` 支撑；提供本地文件系统、内存、S3、Azure Blob 和 GCS 的适配器。参见[文件存储](filesystem.md)。

## E

### Eloquent

整个 ORM 层 - `Model` trait、`Builder<M>`、关系、转换、作用域、观察者、事件、软删除、可修剪、工厂。这是 Laravel 给其他生态系统称为 ORM 的东西起的名字；在 Suprnova 里它架在 SeaORM 之上（用户不应该看到 SeaORM）。参见 [Eloquent](eloquent.md)。

### 队列信封

一个队列驱动程序实际序列化和存储的那个包装结构体（`Envelope { payload, attempts, max_attempts, delay, ... }`）。把[作业](#作业)的载荷和队列的管路隔离开来。参见[队列](queues.md)。

### 事件

一个通过 `EventDispatcher::dispatch(evt)` 分发、并投递给每一个已注册 `Listener<E>` 的可克隆结构体。Suprnova 提供这个 trait、这个门面（`EventFacade`）、`Subscriber` 聚合器，以及给[已入队的监听器](#已入队的监听器)用的钩子。参见[事件](events.md)。

### 事件监听器

参见[监听器](#监听器)。

## F

### 门面

一个零大小结构体的命名惯例，它的 `impl` 块承载着某个子系统的公开 API - `Cache`、`Mail`、`Auth`、`Storage`、`Bus`、`Notify`、`Vector`、`DB`、`Schedule`、`App`。继承自 Laravel；在 Suprnova 里，底层实现是经由[容器](#容器)解析的，而不是通过 PHP 的魔术调用。参见[服务容器](container.md)。

### Eloquent 工厂

`#[derive(Factory)]` 宏和 `Factory` trait，用由 `fake` 驱动的默认值产出逼真的测试行 - `UserFactory::times(5).create_many().await?`。是 Laravel 模型工厂的 Rust 对应物。参见[宏 - 工厂](macros.md#factories)。

### 失败关闭

一种驱动程序故障策略：后端中断会导致请求以 5xx 拒绝 - 在“宁可拒绝也不要泄漏”的场合被速率限制、会话和幂等性使用。与[失败开放](#失败开放)相反。通过 `BackendErrorPolicy::FailClosed` 配置。参见[速率限制](rate-limiting.md)。

### 失败开放

一种驱动程序故障策略：后端中断会放行请求（并记一条 warn 日志），而不是拒绝它 - 用于可用性比这个限制更重要的场合。通过 `BackendErrorPolicy::FailOpen` 配置。参见[速率限制](rate-limiting.md)。

### 功能标志

一个按名字为键、针对当前用户/上下文求值的布尔值（或有类型的值） - `feature!(MyFeature)`。由 `Evaluator` trait 支撑；提供一个数据库求值器，以及架在它之上的一个带 TTL 缓存的求值器。参见[功能标志](feature-flags.md)。

### Fillable

一份编译期的允许列表，说明哪些模型列可以从一份不可信属性的哈希表里被批量赋值 - 通过 `#[fillable]` 属性或者 `Fillable` trait 在模型结构体上声明。是 `#[guarded]` 的对偶。参见 [Eloquent - 批量赋值](eloquent.md#mass-assignment)。

### 文件系统

整个存储子系统 - `Storage` 门面、已注册的[磁盘](#磁盘)、[DiskExt](#diskext) trait、跨磁盘的流式复制。参见[文件存储](filesystem.md)。

### 表单请求

一个实现 `FormRequest`（或者通过 `#[request]` 派生）的结构体，在处理程序运行之前提取并验证一个请求体。是 Laravel 表单请求类的、可组合的、类型安全的对应物。参见[验证](validation.md)。

### `FrameworkError`

每一个框架内部失败都会转换成的那个单一枚举。携带它自己的 `HttpResponse` 投影（`From<FrameworkError> for HttpResponse`），会清理 5xx 响应体，并盖上一个请求 id。参见[错误模型](error-model.md)。

## G

### 门

授权的入口点 - `Gate::allows("update-post", user, post)`。对照已注册的 policy（通过 `#[policy]` 宏声明）解析，并在允许/拒绝时短路。返回一个 `GateResponse`（以授权用的 `Response` 重新导出）。参见[授权](authorization.md)。

### 全局作用域

一个应用在每一次 `Model::query()` 调用上的查询约束，直到被显式移除（`Builder::without_global_scope`）为止。通过 `GlobalScope` trait 实现，并在 bootstrap 里注册。参见 [Eloquent - 作用域](eloquent.md#scopes)。

### 认证守卫

附着在一个请求上的、具名的认证策略 - `session`（有状态，基于 cookie）、`token`（无状态，基于 bearer 令牌）。多个认证守卫可以共存；`Auth::guard("api")` 挑选其中一个。参见[认证](authentication.md)。

### Guarded

一份编译期的拒绝列表，说明哪些模型列*不能*被批量赋值。是 [Fillable](#fillable) 的对偶。参见 [Eloquent - 批量赋值](eloquent.md#mass-assignment)。

## H

### `HasMany`

一种一对多关系 - 父级持有本地键，子级持有外键。十一种 Eloquent 关系种类之一。参见 [Eloquent - 关系](eloquent.md#relationships)。

### `HasManyThrough`

一种经由第三个中间模型跳转来到达关联模型的关系 - `Country -> User -> Post`。参见 [Eloquent - 关系](eloquent.md#relationships)。

### `HasOne`

[HasMany](#hasmany) 的单行版本 - 父级持有本地键，子级带着外键，最多返回一行。参见 [Eloquent - 关系](eloquent.md#relationships)。

### Hash 门面

密码哈希的入口点 - `hash(password)`、`verify(password, hash)`。通过 `HASH_DRIVER` 挑选 bcrypt 或 argon2；`needs_rehash` 让您在登录时把用户从一种算法迁移到另一种。参见[哈希](hashing.md)。

### 处理程序

为一条匹配的路由返回一个 `Response` 的异步函数 - 由 `#[handler]` 宏转换成框架的类型化处理程序形态。组合在中间件链的最内侧。参见[路由](routing.md)、[控制器](controllers.md)。

### `HttpError`

一个用户自定义的错误类型要实现的 trait，用来指定它该如何渲染成一个 HTTP 响应 - 状态码、响应体、请求头。镜照 Laravel 的 `Renderable` 异常。参见[错误处理](errors.md)。

### `HttpResponse`

由处理程序和中间件产出的那个具体 HTTP 响应类型。包装着一个状态码、一组请求头和一个响应体 - 真正会发送给客户端的那个东西。参见[响应](responses.md)。

## I

### 幂等键

一个客户端提供的请求头（`Idempotency-Key`），意思是“如果您已经处理过带着这个键的一个请求，就重放同一个响应，不要再跑一次这个处理程序”。对于可安全重试的 POST/PUT/PATCH/DELETE 是必需的；Suprnova 提供 `Idempotency`、`Idempotent` 和 `Replay` 来包装处理程序。参见[幂等性](idempotency.md)。

### Inertia 响应

一个返回有类型的组件名加上序列化 props、而不是 HTML 的响应 - 是一个 Rust 处理程序和一个 Svelte / React / Vue 页面之间的桥梁。用 `Inertia::render(...)`，或者 `#[derive(InertiaProps)]` 宏加上 `inertia_response!` 构建。参见[前端](frontend.md)、[Inertia 响应](frontend-inertia-responses.md)。

### `InertiaProps`

这个派生宏，为一个被用作 Inertia 页面 props 的结构体，生成 `Serialize` 实现外加 TypeScript 类型元数据。驱动着 `suprnova generate-types` 命令。参见 [TypeScript 类型](frontend-typescript-types.md)。

## J

### 作业

一个实现 `Job` trait 的可序列化结构体 - 带着一个 `handle(self)` 方法，通过 `Queue::push(job)`（延迟分发用 `Queue::push_later(job, when)`）入队。被持久化进队列驱动程序的存储，并由一个工作进程运行。参见[队列](queues.md)。

### 作业中间件

包在一个作业的 `handle` 调用外面运行的那些可组合包装器（`WithoutOverlapping`、`RateLimited`、`ThrottlesExceptions`、`Skip`、`FailOnException`、`SkipIfBatchCancelled`）。是 HTTP 中间件在队列这一侧的对应物。参见[队列 - 作业中间件](queues.md#job-middleware)。

### `JobOutcome`

一个作业结算所产出的那个可判别枚举 - `Completed`、`Failed`、`Released`、`Deleted`、`Skipped` - 通过作业生命周期事件和队列指标计数器报告出来。参见[队列](queues.md)。

## L

### 惰性集合

[集合](#collection-eloquent)的流式对应物 - `Model::query().lazy().await` 返回一个 `LazyCollection<M>`，按块从数据库里取行，而不是把每一行都装进内存。参见 [Eloquent - 分块与惰性迭代](eloquent.md#chunking-and-lazy-iteration)。

### 长度感知分页器

经典的、带编号页码的分页器（`Builder::paginate(per_page)`），会运行这次查询外加一次 `COUNT(*)` - 知道总行数。参见 [Eloquent - 分页](eloquent.md#pagination)。

### 监听器

一个事件处理程序要实现的 trait - `Listener<E>::handle(evt)`。通过 `EventDispatcher::listen::<E, _>(arc_listener)`，或者经由 `Subscriber` 聚合器注册。参见[事件](events.md)。

### 缓存锁守卫

`Cache::lock(key, ttl).acquire()` 返回的那个句柄，代表跨进程的互斥 - `LockGuard`。释放这个守卫就会释放这个锁；如果任它落地不管，就要靠 TTL 兜底。参见[缓存](cache.md)。

### 锁策略

整个项目范围内、处理一个长期存活进程里 `std::sync::Mutex` / `std::sync::RwLock` 中毒的策略 - 两种被认可的模式（映射成错误，或者原地恢复）；永远不要裸用 `.lock().unwrap()`。参见[锁策略](lock-policy.md)。

## M

### `Mailable`

一封邮件消息要实现的 trait - `subject`、`to`、`cc`、`bcc`、`view`、附件。既可以手写，也可以通过 `#[derive(NotificationMailable)]` 宏派生；通过 `Mail::to(...).send(MyMail).await` 发送。参见[邮件](mail.md)。

### 维护模式

一个请求时的开关，让应用对除了一份允许列表之外的所有人下线 - `maintenance_mode().set(payload)`。由 `FileMaintenanceMode`（默认，一个哨兵文件）或 `CacheMaintenanceMode`（基于缓存，用于多实例部署）支撑；由 `MaintenanceMiddleware` 提供服务。在 crate 根部重新导出。

### 中间件

一个包在处理程序外面的可组合包装器 - 之前能看到请求，之后能看到响应，还可以通过返回 `Err(resp)` 来短路。可以全局注册、逐路由注册，或者逐分组注册；按一个固定的由外到内的顺序运行。参见[中间件](middleware.md)。

### 模型

一个标注了 `#[suprnova::model]`、指名一张数据库表的结构体。宏展开之后，这个结构体*就是* SeaORM 的 `Model` - Suprnova 不会包装它。经由 `Model` trait 携带 CRUD，经由 `Model::query()` 携带查询构建，还有工厂、转换、作用域、关系、观察者。参见 [Eloquent](eloquent.md)。

### 多态

“多态（polymorphic）”的简称。一个多态关系让单个关系可以指向若干个模型类型之一 - `MorphTo`（单个所有者，对应若干个可能的类型）、`MorphMany`/`MorphOne`（反过来，收集多态的子级）、`MorphToMany`/`MorphedByMany`（跨多态类型的多对多）。框架维护着一个运行时的 `MorphTypeEntry` [注册表](#注册表)，把判别字符串映射到 Rust 类型。参见 [Eloquent - 关系](eloquent.md#relationships)。

### 修改器

用 `#[mutator]` 宏声明的写侧转换 - 每次这个属性被设置时都会运行，在这个值被存到模型上之前。是[访问器](#访问器)的对偶。参见 [Eloquent - 访问器与修改器](eloquent.md#accessors-and-mutators)。

## N

### Notifiable

一个用户（或者任何能接收通知的对象）要实现的 trait - `route_for(channel)` 为这个具名通道返回地址（邮件地址、推送订阅、广播用户 id 等等），或者返回 `None` 来跳过。参见[通知 - `Notifiable` trait](notifications.md#the-notifiable-trait)。

### Notification

一条通知消息要实现的 trait - `channels()` 返回它应该扇出到的那些通道名字列表；每个通道会回调这条通知（经由逐通道的 trait，比如 `MailRendering` / `DatabaseChannel` 的载荷方法），取得这个通道专属的载荷。通过 `Notify::send(&user, &notif).await` 分发。参见[通知](notifications.md)。

## O

### 观察者

一个实现 `Observer<M>` 的结构体，监听一个 Eloquent 模型的生命周期事件 - `creating`、`created`、`updating`、`updated`、`deleting`、`deleted`、`saving`、`saved`、`retrieved`、`replicating` 等等。通过 `#[suprnova::observer(M)]` 宏注册；在启动时从 inventory 里排空。参见 [Eloquent - 观察者与生命周期事件](eloquent.md#observers-and-lifecycle-events)。

### `OriginPolicy`

CSRF 中间件对改变状态的请求上 `Origin` 请求头的强制执行选择 - `Strict`（必须匹配 host）、`AllowList`，或者 `None`。参见 [CSRF 保护](csrf.md)。

## P

### 分页器

一次 `.paginate(...)` 调用的结果 - 三种口味之一。`LengthAwarePaginator`（带 `COUNT(*)` 的编号页码）、`Paginator`（上一页/下一页，没有总数）、`CursorPaginator`（不透明的游标，用于在一个会变动的结果集上稳定迭代）。三者都会序列化成一份 Laravel 形态的 JSON 载荷。参见 [Eloquent - 分页](eloquent.md#pagination)。

### Panic 边界

包在中间件链外面（以及每一个后台工作进程处理程序外面）的那个 `AssertUnwindSafe(...).catch_unwind()` 包装器，把一次未处理的 panic 转换成一个经过清理的 500，外加一条记录下来的 `ErrorOccurred` 事件。是一张安全网，不是一份契约 - 公开 API 仍然应该返回 `Result`。参见[请求生命周期 - Panic 边界](lifecycle.md#5-panic-boundary--execute_chain_safely)。

### 支付提供商

一个实现 `PaymentProvider` 这个总括 trait（= `Checkout` + `Subscription` + `CustomerStore` + `WebhookHandler`）的类型。参考适配器：`suprnova-payments-stripe`（网关，完整的 `Payment` 实现）和 `suprnova-payments-paddle`（记录商户，没有 `Payment`）。参见[支付](payments.md)、[提供商指南](payments-provider-guide.md)。

### 中间表

[BelongsToMany](#belongstomany) 关系里的那个中间模型 - 一个一等公民的 `#[suprnova::model]`，有自己的结构体、转换和时间戳，作为第三个类型参数被显式指名（`BelongsToMany<L, R, P>`）。Suprnova 不会从一个表名隐式合成出一个中间表。参见 [Eloquent - 关系](eloquent.md#relationships)。

### 呈现频道

[频道](#频道-广播)的一个变体，服务器会追踪当前是谁订阅着，并带着每个成员的元数据发出加入/离开事件。适合用来做“谁在线”这类指示器。参见[广播 - 呈现频道](broadcasting.md#presence-channels)。

### 私有频道

[频道](#频道-广播)的一个变体，订阅时需要授权 - 对订阅的用户，`authorize(...)` 必须返回 true。适合用于逐用户的通知流。参见[广播 - 频道](broadcasting.md#channels)。

### 可修剪

把一个软删除（或者可查询）的模型标记为可以被 `model:prune` 清理的那个 trait - `Prunable::prunable_query()` 返回该删的那些行的构造器。`MassPrunable` 用单条 `DELETE WHERE` 删除；默认做法是逐行删除，好让观察者能触发。通过 `#[prunable]` 宏为注册表打上标记。参见 [Eloquent - 可修剪](eloquent.md#prunable)。

## Q

### 队列

整个后台工作子系统 - `Queue` 门面、[作业](#作业) trait、[信封](#队列信封)、驱动程序（memory、sync、redis、database、null）、工作进程、批次、链。参见[队列](queues.md)。

### 队列驱动程序

一个实现 `QueueDriver`（push、pop、release 等等）的类型 - 提供 `MemoryQueueDriver`、`SyncQueueDriver`（原地运行）、`RedisQueueDriver`、`DatabaseQueueDriver`、`NullQueueDriver`。在启动时通过 `QUEUE_DRIVER` 挑选。参见[队列 - 驱动程序](queues.md#drivers)。

### 队列工作进程

一个长期存活的循环，从队列驱动程序里拉取信封，在处理程序外面运行作业中间件，并报告结果。经由和 HTTP 服务器同样的生命周期启动，所以观察者和监听器会以完全一样的方式触发。由 `cargo run -- queue:work` 启动。参见[队列](queues.md)。

### 已入队的监听器

一个 `Listener<E>`，被调用时会把这个事件载荷持久化进队列，并在一个后台工作进程里运行 `handle`，而不是在进程内运行。适合用在一个事件监听器要做不该阻塞分发路径的 I/O 的场合。通过 `QueuedListener` 适配器包装。参见[事件](events.md)。

## R

### 限流器

整个速率限制子系统 - `RateLimiter`（基于缓存的门面）、`Limit` 构建器、`SlidingWindowConfig`（滑动窗口驱动程序）、`RateLimitMiddleware`（挂在路由上）、`ThrottleRequestsMiddleware`（Laravel 命名的别名）、`BackendErrorPolicy`（失败开放对失败关闭）。参见[速率限制](rate-limiting.md)。

### 重定向

一个包装着 `Location` 请求头的、专用的 [HttpResponse](#httpresponse) - 通过 `Redirect::to(...)`、`Redirect::route(...)`、`Redirect::back()` 构建，配上 `.with(...)`/`.with_input(...)` 链来携带 flash 数据。参见 [URL 生成](urls.md)、[响应](responses.md)。

### 注册表

一个进程级全局的查找表，要么在编译期由 `inventory` 填充（`ModelEntry`、`RelationEntry`、`MorphTypeEntry`、`ObserverEntry`、`PrunerEntry`、`TaskEntry`、`PaymentProviderEntry`、`CommandEntry`），要么在启动时由显式注册填充（`ConnectionRegistry`、`MiddlewareRegistry`、`InertiaRegistry`、`ChannelRegistry`、`VectorRegistry`、`SupervisorRegistry`）。全部都会在启动序列期间被排空或者查询。

### 关系

每一种关系种类都要实现的 trait - `BelongsTo`、`HasOne`、`HasMany`、`BelongsToMany`、`HasOneThrough`、`HasManyThrough`、`MorphTo`、`MorphOne`、`MorphMany`、`MorphToMany`、`MorphedByMany`。一个模型把它的关系声明为返回一个关系结构体的方法；框架从这个 trait 驱动预加载、`with(...)`、关系存在性查询，以及级联的 touch。参见 [Eloquent - 关系](eloquent.md#relationships)。

### 请求

框架那个有类型的请求结构体 - 包装底层的 hyper 请求，并暴露 `req.param("id")`、`req.json::<T>()`、`req.form_data()`、`req.flash()` 等等。以 `suprnova::Request` 重新导出。参见[请求](requests.md)。

### `Response`

Suprnova 把 `http::Response` 绑定到 `Result<HttpResponse, HttpResponse>` - 两个分支都携带一个 `HttpResponse`。处理程序体返回 `Response`，用 `?` 传播可能失败的工作，运行时用 `result.unwrap_or_else(|e| e)` 把两个分支折叠到一起。授权判定类型以 `GateResponse` 重新导出，避免和这个名字冲突。参见[响应](responses.md)、[请求生命周期](lifecycle.md#the-response-contract)。

### 资源

有两个不相关的东西共用这个名字；两者都已实现。

1. **JSON:API 资源** - 一个 `#[derive(Resource)]` 结构体，把一个模型序列化成带稀疏字段集和 include 的 JSON:API 形态。参见 [API 资源](eloquent-resources.md)。
2. **资源路由** - 一个路由辅助函数，针对一个 `ResourceController` 实现挂载一整套 CRUD 的 `index`/`show`/`store`/`update`/`destroy`。参见[路由](routing.md)。

### `routes!` 宏

把一个路由 DSL（`get!("/users", users::index)`、`group!`、`middleware!(Auth)`）展开成一个 `Router` 工厂函数的编译期宏。是一个应用里路由的唯一真相来源。参见[路由](routing.md)、[宏](macros.md)。

## S

### 本地作用域

用 `#[scopes(Model)]` 宏在一个 Eloquent 模型上声明的、可复用的查询片段 - `Post::query().published().recent().get()`。本地作用域默认是关闭的；只有被调用时才会运行。是[全局作用域](#全局作用域)的对应物。参见 [Eloquent - 作用域](eloquent.md#scopes)。

### 填充器

一个实现 `Seeder` trait、给数据库填充起始数据的类型 - 通过 `suprnova db:seed` 注册。通常由一个[工厂](#eloquent-工厂)支撑。参见 [Eloquent](eloquent.md)。

### 签名 URL

一个查询字符串里携带着 HMAC 签名（`?signature=...&expires=...`）的 URL，用来证明它是这个应用产出的、没有被篡改过。通过 `sign_url(...)` / `sign_route(...)` 构建；由中间件或者 `verify_signature(...)` 验证。参见 [URL 生成 - 签名 URL](urls.md#signed-urls)。

### 软删除

删除一个模型行时，设置一个 `deleted_at` 时间戳、而不是发出 `DELETE` 的那种模式。逐模型通过 `#[suprnova::model]` 属性上的 `soft_deletes = true` 选择加入；`Model::query()` 会自动过滤掉已回收的行；`with_trashed()` 和 `only_trashed()` 可以选择性地把它们找回来。参见 [Eloquent - 删除与软删除](eloquent.md#deleting-and-soft-deletes)。

### `Storage` 门面

文件系统子系统的入口点 - `Storage::disk("s3")`、`Storage::disk("local")` - 返回一个 [DiskExt](#diskext) 实现。参见[文件存储](filesystem.md)。

### 订阅者

一个用一次调用注册许多监听器的聚合器 - 实现 `Subscriber::subscribe(dispatcher)`，并经由 `EventDispatcher::subscribe(subscriber)` 注册。参见[事件](events.md)。

### 监督程序

一个长期存活的后台执行体要实现的 trait（`Supervisor::run`），好活在 `SupervisorRegistry` 之下。这个注册表会捕获运行循环里的 panic，应用一个 `RestartPolicy`，然后重新生成。是 Erlang `gen_server` 监督者模式的 Rust 对应物。参见[监督程序](supervisors.md)。

## T

### 任务

一个实现 `Task` trait 的结构体 - 声明一个 cron 表达式，或者一个更高层的频率（`daily()`、`every_minute()`），并在调度器上运行。在编译期经由 `TaskEntry` inventory 被发现。参见[任务调度](scheduling.md)。

### 可终止中间件

注册一个在响应已经写给客户端*之后*运行的钩子的中间件 - 通过 `Terminable` trait 实现，被捕获进一个 `TerminationSnapshot`，由 `dispatch_termination` 分发。适合用于日志记录、指标刷新、事后审计。参见[中间件 - 可终止中间件](middleware.md#terminable-middleware-post-response-hooks)。

### 穿透关系

一种经由第三个中间模型跳转的关系 - [HasManyThrough](#hasmanythrough) 和 `HasOneThrough`。参见 [Eloquent - 关系](eloquent.md#relationships)。

### 超时

给单次请求的墙钟时间设界、并在超出这个界限时返回 504 的中间件 - `TimeoutMiddleware`。和队列工作进程的超时（队列这一侧的 `TimeoutExceeded`）以及 HTTP 客户端的超时是两回事。参见[超时](timeout.md)。

### `TypedCommand`

console 那一侧的 trait - 由 `#[derive(Command)]` 结构体实现 - 给一个 console 命令提供有类型的参数（经由 `clap`）和一个异步的 `handle(self)` 方法。在编译期注册进 `CommandEntry` inventory。参见[控制台](console.md)。

## U

### `UserId`

`Auth::id()` 返回的不透明字符串标识符。框架的守卫/提供者路径携带已配置 `UserProvider` 使用的任何稳定键；对 `EloquentUserProvider<User>` 来说，这通常是字符串化的主键。Magnetar 门面公开一个 `UserId` newtype，但在写入框架会话状态之前，会把它的值绑定回应用的规范用户 ID。把请求边界保持为字符串形式，让数字 ID、UUID 和独立于提供者的不透明 ID 都能使用相同的中间件和事件契约。参见[认证](authentication.md)。

## V

### VAPID

Voluntary Application Server Identification（自愿应用服务器标识）- 用于标识一个 web push 发送方的 IETF 规范。Suprnova 提供 `VapidKey`、`VapidSigner`、`VapidClaims`，以及给每一次推送请求签名的 `WebPushClient`。参见 [Web 推送](web-push.md)。

### `Vector` 门面

向量搜索子系统的入口点 - `Vector::driver("qdrant").await?.upsert(...)`。由 `VectorDriver` 的一些实现支撑：内存、Qdrant、Pinecone（受 feature 把关）、MariaDB 原生。参见[向量搜索](vector.md)。

### `VectorDriver`

每一个向量后端都要实现的 trait - `upsert`、`search`、`delete`、`count`。让框架能支持多个向量数据库，而不强迫您用某一个。参见[向量搜索](vector.md)。

## W

### Web 推送

Web 平台的推送通知协议 - 加密的载荷经由用户代理的推送服务投递。Suprnova 提供 `WebPushClient`（VAPID 签名器、retry-after 解析、8 KiB 拒绝上限）和用于 [Notification](#notification) 投递的 `WebPushChannel`。参见 [Web 推送](web-push.md)。

### Webhook

一个由第三方（支付提供商、身份提供者……）发进您的应用、用来报告一个事件的 HTTP 请求。Suprnova 默认把每一个 webhook 都当作幂等的 - 提供商适配器实现 `WebhookHandler::verify(...)`，并把提供商的事件 id 存进一个会拒绝重放的 `UNIQUE` 约束里。参见[支付 - Webhook 处理](payments.md#webhook-handling)、[幂等性](idempotency.md)。

### 工作流

一段长时间运行的、由有类型步骤组成的有状态后台工作 - `#[workflow]` 和 `#[workflow_step]` 宏。每一步的返回值都会被持久化，所以一次工作流进行到一半时的工作进程重启，会从最后一个已完成的步骤恢复。这是 Suprnova 对那些装不进单个[作业](#作业)的多步骤后台流程给出的答案。参见[工作流](workflows.md)。

### `WsConfig`

逐路由的 WebSocket 配置 - 载荷大小上限（默认 1 MiB 文本 / 64 KiB 二进制）、最大帧大小、ping 间隔、空闲超时、来源策略。被 `ws!()` 路由使用。参见 [WebSocket](websockets.md)。

### `WsSocket`

框架那个交给 `ws!()` 处理程序的、有类型的 WebSocket 句柄。经由 `WsSocket::split()` 拆成一个 `Sink`（发送）半和一个 `Stream`（接收）半；ping/pong 由一个带 `AbortHandle` 的心跳任务管理，所以一个被丢弃的处理程序总会干净地拆除。参见 [WebSocket](websockets.md)。

## 下一步

- [Laravel 对等映射](parity.md) - 针对 Laravel 13 的逐项功能对比
- [环境变量](env-vars.md) - 框架读取的每一个 `env!`
- [文档索引](documentation.md) - 章节地图
