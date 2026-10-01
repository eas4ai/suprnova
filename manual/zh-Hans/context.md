# 上下文

`Context` 是 Suprnova 的逐请求键值包。您把那些希望同一次请求里每一个下游调用方都能看到的数据塞进它 - 一个 request id、一个租户 slug、一个用户角色、一条审计轨迹 - 而不必让这个值穿过每一个函数签名。它是 Suprnova 对应 Laravel `Context` 门面的等价物。

```rust
use suprnova::Context;

Context::add("tenant_id", "acme");
Context::push("breadcrumbs", "checkout/start");
Context::hidden_add("api_key", secret);

let tenant: Option<String> = Context::get("tenant_id");
let page: Option<String> = Context::query_param("page");
```

在下面这些时候用它：

- 一行日志、一个排队的作业或者一条广播消息，需要请求作用域内的元数据（租户 id、关联 id、用户角色）
- 一个深层嵌套的辅助函数需要一个处理程序已经拿到的值，但这条调用链不该让一个参数穿过每一层
- 您想从并非处理程序的代码里，读取当前请求的查询字符串（`?page=3`、`?cursor=…`）

`Context` **不是**用来放跨请求状态的。它绑定在当前的 Tokio 任务上，请求结束时就消失。对于那些活得比一次请求更久的东西，请使用[服务容器](container.md)或者[缓存](cache.md)。

## 两个包

每一个活跃的 `Context` 作用域都携带两个键值映射和一个额外的槽位：

| 包 | 用什么读 | 是否出现在 `Context::all()` 里 |
|---|---|---|
| **可见** | `Context::get` | 是 |
| **隐藏** | `Context::hidden_get` | 否 |
| **查询** | `Context::query_param` | 否（URL 里那些 `?key=value` 对的一份独立快照） |

可见与隐藏之分，正是要两个包的全部意义所在：那些把 `Context::all()` 转储进结构化输出的日志序列化器，不会泄漏您有意隐藏的数据。审计元数据放进可见包；API 密钥、OAuth bearer 令牌，以及您不想出现在日志里的个人身份信息，放进隐藏包。

查询包由框架的请求中间件从 URL 的查询字符串自动填充（见下面的[分页会读取查询参数](#分页会读取查询参数)）。您通常只读它，从不写它。

## 活跃的作用域

框架会在每一个进来的 HTTP 请求上安装一个 `Context` 作用域。在处理程序、中间件、模型观察者、事件监听器，或者任何其他从请求任务可达的地方，这个作用域都是活的，`Context::*` 的读写不需要任何仪式就能工作。

在作用域之外 - 早期启动阶段的代码、一个不继承上下文的裸 `tokio::spawn`、一个没有安装作用域的单元测试 - 每一次修改都是一次**静默的空操作**，每一次读取都返回 `None`。契约是：绝不 panic，无论您从哪里调用。

```rust
// 在处理程序里 - 作用域是活的，一切正常工作：
Context::add("user_id", 42i64);
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, Some(42));

// 在作用域之外 - 静默的空操作 + None：
Context::add("user_id", 42i64);            // 被丢弃
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, None);
```

这条不 panic 的契约是刻意的。会碰到 `Context` 的库代码（一个自定义的日志订阅者、一个 SDK 扩展）不该需要知道自己是跑在一次请求里还是跑在启动阶段 - 它只管调用 `Context::get`，并把 `None` 当作“现在拿不到”来处理。

### 静默操作的可观测性

一次真正静默的空操作会藏起 bug（中间件顺序错了、上下文没有被传播进一个 spawn 出来的任务、启动期的一次误读）。框架的修改类操作依然不 panic，但每当它们丢弃东西时，都会在 `suprnova::context` 这个 target 上发出一个 `tracing::trace!` 事件：

```text
TRACE suprnova::context: Context mutation discarded: no active scope on this task op="add"
TRACE suprnova::context: Context mutation discarded: value failed to serialize op="push" key="bad"
TRACE suprnova::context: Context read returned None: value present but did not deserialize op="get" key="user_id" expected="String"
```

三类事件：

| 事件 | 何时触发 |
|---|---|
| `mutation discarded: no active scope` | 在任何作用域之外调用了 `add`、`push`、`hidden_add`、`forget` |
| `mutation discarded: value failed to serialize` | `add`/`push`/`hidden_add` 的值，它的 `Serialize` 实现报错了 |
| `read returned None: value present but did not deserialize` | `get`/`hidden_get` 找到了这个键，但存着的 JSON 与请求的 `T` 对不上 |

单纯的缺失 - 对一个从未设置过的键调用 `get` - 依然保持静默，这样“这个设过没有？”式的探测就不会把日志淹没。当您怀疑有传播方面的 bug 时，打开 `RUST_LOG=suprnova::context=trace`；那条静默的空操作路径就会变得可见，而生产代码的行为丝毫不变。

## 添加值

### `Context::add` - 在某个键上替换

```rust
use suprnova::Context;

Context::add("user_id", 42i64);
Context::add("tenant", "acme");
Context::add("plan", PlanTier::Pro);     // 任何可 Serialize 的值
```

键是 `Into<String>`；值是任何 `Serialize` 类型。值在写入时会被一次性转换成 `serde_json::Value`，并按那个形式存储。对同一个键后续的 `add` 会做替换。

### `Context::push` - 追加到一个栈上

```rust
Context::push("trail", "home");
Context::push("trail", "settings");
Context::push("trail", "billing");

let trail: Vec<String> = Context::get("trail").unwrap();
assert_eq!(trail, vec!["home", "settings", "billing"]);
```

`push` 会在第一次调用时初始化一个空数组，在后续调用时追加。如果这个键上已经有一个标量，它会被转换成一个 `[scalar, new_value]` 数组 - 对于同一个键上先前的 `add`，`push` 是宽容的。

### `Context::hidden_add` - 写入隐藏包

```rust
Context::hidden_add("api_key", os_env_secret);
Context::hidden_add("oauth_bearer", token);

// 可见包的转储（比如一个 JSON 日志发出器）看不到它们：
let all = Context::all();
assert!(!all.contains_key("api_key"));

// 但您仍然可以有意地读到它们：
let key: Option<String> = Context::hidden_get("api_key");
```

隐藏包的键与可见包的键是各自独立的 - 一次 `hidden_add("user_id", 99)` 和一次 `add("user_id", "alice")` 可以共存而不冲突。`Context::forget(key)` 一次调用就能从两个包里都移除。

## 读取值

### `Context::get` - 从可见包做类型化的读取

```rust
use suprnova::Context;

let user_id: Option<i64>       = Context::get("user_id");
let tenant:  Option<String>    = Context::get("tenant");
let trail:   Option<Vec<String>> = Context::get("trail");
```

`get` 在 `T: DeserializeOwned` 上是泛型的。存着的 JSON 值会在每一次读取时被反序列化。以下情况返回 `None`：

- 这个键没有被设置
- 当前任务上没有活跃的作用域
- 存着的值反序列化不成 `T`（比如您存进去一个 `i64`，却要一个 `String`）

最后这种情况会发出一条 `tracing::trace!`，好让这个类型不对的 bug 变得可观测 - 明明真相是“这个值的形状不对”，`Context::get` 看上去却像是“这个值没有被设置”，这类 bug 在没有一行日志指路的情况下要花上一小时才找得出来。

### `Context::hidden_get` - 从隐藏包做类型化的读取

与 `get` 形态相同，只是读隐藏包。类型不对时的 tracing 行为也一样。

### `Context::has` - 在可见包上做存在性检查

```rust
if Context::has("user_id") {
    // …
}
```

`has` 只检查可见包（如果您需要探测隐藏包，请用 `hidden_get(...).is_some()`）。

### `Context::all` - 可见包的快照

```rust
let snapshot: HashMap<String, serde_json::Value> = Context::all();
```

在作用域之外返回一个空的 `HashMap`。一个 JSON 日志发出器就该调用它，把请求作用域内的字段注入到每一行日志里 - 这也正是隐藏包要单独存在的原因。

### `Context::forget` - 从两个包里都移除一个键

```rust
Context::forget("trail");          // 可见包和隐藏包里都会被移除
```

这种双包移除是有意为之。如果您把相关的数据分别存进了两个包（比如 `user_id` 在可见包、`user_email` 在隐藏包），一次 `forget` 就把两边都清理干净。

## 读取查询参数

`Context::query_param` 读取的是在请求入口处捕获下来的、URL 里的那些 `?key=value` 对。请求中间件会把查询字符串一次性解析进这个作用域的查询包，之后每一个下游调用方都能按名字读取单个参数，而不必重新解析：

```rust
use suprnova::Context;

let page: Option<String>   = Context::query_param("page");
let cursor: Option<String> = Context::query_param("cursor");
let sort: Option<String>   = Context::query_param("sort");
```

当参数缺失、或者没有活跃的作用域时返回 `None`。重复的键遵循 Laravel 的“后者胜出”语义 - 和您从请求解析出来的那份查询映射里拿到的是同一个值。

### 分页会读取查询参数

这就是查询包存在的原因。Eloquent 的分页器直接从 `Context::query_param` 上读取 `?page=` 和 `?cursor=`，所以一个返回分页器的处理程序不需要手工把页码一路接下去：

```rust
use suprnova::{json_response, Request, Response};
use crate::models::Post;

pub async fn index(_req: Request) -> Response {
    // 通过 Context::query_param 从请求的 URL 里读取 ?page=N
    // - 不需要 req.query() 的样板代码，也不用把参数一层层穿下去。
    let posts = Post::query()
        .order_by_desc("created_at")
        .paginate(15)
        .await?;

    json_response!(posts)
}
```

有三个分页器入口点用到了它：

- `Builder::paginate(per_page)` - 读取 `?page=`
- `Builder::simple_paginate(per_page)` - 读取 `?page=`
- `Builder::cursor_paginate(per_page)` - 读取 `?cursor=`

完整的表面请参见[分页](pagination.md)。

## 传播进 spawn 出来的任务

`tokio::spawn` 会用一套全新的任务本地环境来启动子任务 - 父任务的 `Context` 作用域**不会**流进去。一次请求里的裸 `tokio::spawn` 看到的是一个空的 `Context`，每一次读取都返回 `None`。

要把作用域带进一次 spawn，请用 `Context::current()` 给它拍一张快照，再在子任务里用 `Context::scope` 重新进入它：

```rust
use suprnova::context::Context;

// 在一个请求处理程序里：
if let Some(store) = Context::current() {
    tokio::spawn(Context::scope(store, async move {
        // 现在 `Context::get`、`Context::query_param` 等等看到的
        // 就是父请求的那个包了。
        let request_id: Option<String> = Context::get("_request_id");
        do_background_work(request_id).await;
    }));
}
```

`Context::current()` 返回的那个 `ContextStore` 通过 `Arc` 共享着父任务底层的那些映射 - 只要子任务还持有这份克隆，它写入的东西对父任务就是可见的。这正是审计类和日志类的 spawn 想要的：子任务可以打上额外的键（`Context::add("audit.completed", true)`），而父任务最后那行日志就能看见它们。

如果您需要一份隔离的快照（子任务的写入不该回流到父任务），那就新建一个 `ContextStore`，只把您需要的那些键复制进去。

### 为什么裸的 `spawn` 不会传播

Tokio 的任务本地值（`tokio::task_local!`）刻意被限定在任务作用域内。跨 spawn 自动继承将意味着：

- 长期存活的后台任务会让父任务的上下文映射永远无法被释放
- 子任务里的一次 panic 可能会污染父任务的状态
- 运行时在每一次读取任务本地值时，都得沿着一条父指针链往上走

显式的 `Context::current()` + `Context::scope` 这套动作，让传播成为一个刻意的决定，而不是一个隐藏的默认行为。

## 测试

在 `#[tokio::test]` 或者 `#[suprnova_test]` 里，默认不会安装任何 `Context` 作用域。被测的、会碰到上下文的代码大多都能优雅地处理“没有作用域”这种情况（静默的空操作 + 读取返回 `None`），所以普通的单元测试不需要任何准备工作。

有两种情况测试需要帮一把：

### 当被测代码调用 `query_param` 时

分页辅助函数通过 `Context::query_param` 读取 `?page=`。一个针对“第 3 页返回正确的偏移量”的单元测试，需要 `query_param` 返回 `Some("3")`。有两种做法：

**`test_query_guard`（推荐）：**

```rust
use suprnova::Context;

#[tokio::test]
async fn paginate_reads_page_from_query() {
    let _q = Context::test_query_guard("page", "3");

    // 被测代码现在会看到 ?page=3
    assert_eq!(Context::query_param("page"), Some("3".into()));

    let posts = Post::query().paginate(15).await?;
    assert_eq!(posts.current_page(), 3);
}
// `_q` 在作用域结束时被丢弃 - 线程本地的覆盖会被抹掉。
```

`test_query_guard` 返回一个 RAII 守卫。即便测试的方法体发生了 panic，`Drop` 也会在这个操作系统线程被回收之前运行，把线程本地的覆盖清掉。这个守卫带着 `#[must_use]` - 把它绑定到 `_` 上会立刻清除，而那几乎从来都不是您想要的。

**裸用 `test_set_query` + `test_clear_query`：**

```rust
#[tokio::test]
async fn manual_pair() {
    Context::test_clear_query();        // 抹掉从任何相邻测试漏过来的残留
    Context::test_set_query("page", "5");

    // … 各种断言 …

    Context::test_clear_query();
}
```

请用守卫那种形式。手动的这一对之所以存在，是为了应付那些需要多个覆盖各自独立地设置和清除的场景，但带 `#[must_use]` 的守卫更难被用错。

这两个 API 都被 `#[cfg(any(test, feature = "testing"))]` 把住 - 它们会被编译进测试二进制文件，以及那些为了集成测试装置而选择启用 `testing` feature 的 release 构建。在普通的 release 构建里它们并不存在。

### 当被测代码从一个 `Context` 作用域读写时

请通过 `Context::scope` 显式地安装一个：

```rust
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_tenant_id() {
    Context::scope(ContextStore::default(), async {
        Context::add("tenant_id", "acme");

        let resolved = my_helper_that_reads_tenant().await;
        assert_eq!(resolved, "acme");
    })
    .await;
}
```

或者在创建作用域时就播种一个查询包：

```rust
use std::collections::HashMap;
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_query_from_scope() {
    let mut q = HashMap::new();
    q.insert("page".into(), "3".into());
    q.insert("sort".into(), "name".into());

    Context::scope(ContextStore::with_query(q), async {
        assert_eq!(Context::query_param("page"), Some("3".into()));
        assert_eq!(Context::query_param("sort"), Some("name".into()));
    })
    .await;
}
```

`ContextStore::with_query(HashMap)` 就是请求中间件用的那个构造函数，所以一个走着与生产相同代码路径的测试，看到的查询包形状也是一样的。

### 为什么会有那个线程本地的覆盖

查询参数的覆盖用的是 `thread_local!`，而不是任务本地值。这是刻意的：它让测试可以安装查询参数，**而不必把每一条断言都包进一次 `Context::scope` 调用里**。整套组合是这样的：

1. 读取时先检查线程本地的覆盖
2. 如果没有覆盖，就读取任务本地的 `CONTEXT` 作用域的查询包
3. 如果连作用域也没有，就返回 `None`

这次线程本地的查找在生产环境里实际上不花任何代价（在测试构建之外，这个覆盖永远是空的），而且省去了测试作者在每一条与分页相关的断言外面套上样板式 `Context::scope(...)` 的麻烦。

## 常见模式

### 在每一条日志上打上 request id

框架已经这么做了。请求中间件会把 `_request_id` 播种进可见包，这样下游的作业、广播和 `Context::all()` 的日志转储就能按名字读到这个 id。同一个中间件还会开启一个 `tracing` span，把这个 id 作为 span 的一个字段带上，正是这一点让它出现在请求内部发出的每一行日志上 - 订阅者那一侧请参见[日志](logging.md)。当您需要把这个值当作字符串来用时（比如把它接进一个出站的 HTTP 请求当作关联请求头），从 `Context` 里读取这个 id 就是正确的路径：

```rust
let request_id: Option<String> = Context::get("_request_id");
```

### 把租户上下文带进一个排队的作业

`Context` 不会自动跨越队列的序列化 / 反序列化边界传播 - 工作进程与派发方跑在不同的进程里，往往还在不同的机器上。请把您需要的任何东西塞进作业的载荷里：

```rust
use suprnova::{Context, FrameworkError, Queue};

// 在一个处理程序里：
let tenant_id: String = Context::get("tenant_id")
    .ok_or_else(|| FrameworkError::param("tenant_id missing"))?;

Queue::push(SendInvoice { tenant_id, invoice_id }).await?;
```

当工作进程处理 `SendInvoice` 时，请在 `Job::handle` 的开头安装一个全新的 `Context` 作用域，并从作业载荷里把您需要的那些键重新播种进去 - 用 `Context::scope(ContextStore::default(), async { ... })` 把方法体包起来。这样，这个作业调用的任何日志或者深层嵌套的辅助函数，看到的租户 id 就和它在一次请求里看到的一样。

这也是 `hidden_add` 挣得自己一席之地的地方 - 作业可以在进入作用域时一次性取到并藏好一个 API 密钥，之后作业内部每一次下游的 HTTP 调用都通过 `Context::hidden_get` 读取它，不必再去取一遍。`Job` trait 的形态请参见[队列](queues.md)。

### 贯穿一次请求的审计轨迹

```rust
Context::push("audit.steps", "validated_input");
// … 更多工作 …
Context::push("audit.steps", "charged_card");
// … 更多工作 …
Context::push("audit.steps", "sent_receipt");

// 在响应期的中间件里：
let steps: Vec<String> = Context::get("audit.steps").unwrap_or_default();
tracing::info!(?steps, "request audit trail");
```

一个在处理程序之后运行的响应期中间件，可以用一行日志把这条审计轨迹转储出来，而不是让每一步各自的 debug 行散落在请求日志的各处。

### 用隐藏包存放 SDK 扩展的凭据

```rust
// 在请求入口处，认证之后：
Context::hidden_add("sdk.api_key", load_api_key_for(user_id));

// 在一次 SDK 调用的深处：
let key = Context::hidden_get::<String>("sdk.api_key")
    .ok_or_else(|| FrameworkError::param("api key not stashed"))?;
```

那些转储 `Context::all()` 的日志不会显示这个密钥。对于任何处理程序需要往调用栈深处传递、又不想暴露给日志表面的凭据，隐藏包都是正确的存放位置。

## 为什么 Suprnova 有所不同

灵感来自 Laravel 的 `Context` 门面（在 Laravel 11 引入）- 相同的方法名、相同的可见 / 隐藏之分、相同的“请求之外保持静默”契约。有两处差别来自 Rust 的运行时：

**异步传播是显式的，不是魔法。** Laravel 的 `Context` 会自动流经排队的作业，因为 Laravel 在派发时就把上下文包序列化进了作业载荷。Rust 的异步模型里并没有一个单一的“当前请求”供线程本地值流入 - `tokio::spawn` 是从头开始的，而队列边界还牵涉到跨进程的序列化。Suprnova 把传播的原语（`Context::current()` + `Context::scope`）暴露出来，让您在边界处主动选择启用它，而不是假装任务继承了它们其实并没有继承的上下文。

**类型不对的读取是可观测的。** 在 Laravel 里，对一个以别的类型存储的值调用 `get::<T>` 会静默地返回 `None`（这是 PHP，反正写入的时候也没有强制类型）。在 Suprnova 里，这次读取会发出一条 `tracing::trace!`，因为类型不对这件事说明存在一个真正的 bug - 这个值确实在某处被写入过，只是没有用您正在读取的那个类型。这条 trace 让您能在带插桩的运行里找到它，同时又不改动那条不 panic 的契约。

第三处分歧是机械性的：Suprnova 的 `Context` 建立在 `tokio::task_local!` 之上，所以它的生命周期绑定在 Tokio 任务上，而不是绑定在任何全局状态上。跨线程的读取看到的是**当前跑在那个线程上的那个任务**的作用域，而不是最后一次被安装的那个作用域。正是这一点让同一个 `Context` 门面可以安全地从线程池、actor 或者一个 `spawn_blocking` 的方法体里调用 - 前提是您把作用域传播进了那次 spawn。

## 它位于何处

| 主题 | 文件 |
|---|---|
| `Context` 门面 + `ContextStore` | `framework/src/context/mod.rs` |
| HTTP 请求上的作用域安装 | `framework/src/logging/request_id.rs` |
| `Context::query_param` 的调用方（分页） | `framework/src/eloquent/builder.rs` |
| 重导出 | `framework/src/lib.rs`（`pub use context::{Context, ContextStore}`） |

## 下一步

- [请求生命周期](lifecycle.md) - `Context` 作用域在每一次请求上是在哪里被安装的
- [服务容器](container.md) - 用于那些活得比单个任务更久的跨请求状态
- [日志](logging.md) - `Context::all()` 是如何进到结构化日志行里的
- [分页](pagination.md) - `Context::query_param` 主要的下游读取方
- [测试](testing.md) - 单元测试里的 `test_query_guard` 和 `Context::scope` 模式
