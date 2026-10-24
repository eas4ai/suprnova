# 锁策略

Suprnova 是一个单一的、长期存活的 Tokio 进程，而不是一个由短命 PHP 工作进程组成的集群。您在启动时绑定的每一个进程级全局注册表、单例和共享缓存，都会比每一个触碰它的请求活得更久。这带来了一个不大但影响深远的变化，关系到您该如何使用 `std::sync::Mutex` 和 `std::sync::RwLock`：在持有守卫期间发生的一次 panic，会让这把锁在此后进程剩余的整个生命周期里都保持*被污染*的状态，而下一个调用者必须决定该拿它怎么办。本章就是针对这个决策的项目级统一策略 - 两种被认可的模式、该在什么情况下选择哪一种，以及为什么您永远不应该在框架或应用代码里使用裸的 `.lock().unwrap()`。

## 本章为何存在

在 Laravel 里，您从来不用去想被污染的锁，因为根本不存在这种东西。PHP 是 shared-nothing 的：一次致命错误会拆掉一个请求的进程，下一个请求会在一个全新的进程里启动，没有任何内存状态能存活下来被破坏。Suprnova 的运行方式正好相反。进程只启动一次，注册表被填充之后，会在二进制文件的整个生命周期里持续存活。一个处理程序，如果在持有某个进程级全局 `RwLock` 的写守卫期间发生 panic，就会让那把锁被*污染* - 此后每一次 `.read()` 和 `.write()` 都会永远返回 `Err(PoisonError)`，除非有人显式地恢复它。

默认的 Rust 习惯用法 - `.lock().unwrap()` - 会把那个 `Err` 转换成一次 panic。这次 panic 接着会在调用栈上游的某处变成另一把被污染的锁。这把锁又会拖垮下一个碰到它的子系统。一个糟糕的请求，就这样级联成一个半死不活的进程。

下面的策略正是为了防止这种级联。

> **范围。** 本策略适用于携带污染状态的 `std::sync::Mutex` 和 `std::sync::RwLock`。`tokio::sync` 里的异步表亲（`Mutex`、`RwLock`、`Semaphore`）*不会*被污染 - 在持有 `tokio::sync::Mutex` 守卫期间发生 panic，会干净地丢弃这个守卫，下一次 `.lock().await` 会成功。如果您的热路径是异步的，并且不需要从一个同步上下文（一个 `Drop` 实现、一个框架回调、一个 CLI 子命令）里获取这个守卫，就优先选用 Tokio 的版本，这个问题就不存在了。

## 两种被认可的模式

框架里每一个持有 `std::sync` 锁的地方，都恰好使用这两种模式之一。在您自己的代码里，也按同样的方式选择。

### 模式 1 - 将污染映射为返回的错误

当调用者本来就返回 `Result<_, E>`、再多一个 `?` 也不会改变它的形式时，就把污染作为一个错误暴露出来，让请求干净地失败。框架内部使用 `pub(crate)` 辅助函数（`lock::read`、`lock::write`、`lock::lock`），把一个被污染的守卫映射为 `FrameworkError::internal("<context> lock poisoned")`，并嵌入一个调用者提供的标签，这样日志就能分辨出是哪个子系统被污染了，而不需要每个调用点自己去包装错误。

这些辅助函数所体现的模式很简短，足以直接写在您的应用代码里：

```rust
use std::collections::HashMap;
use std::sync::RwLock;
use suprnova::FrameworkError;

static FEATURE_FLAGS: RwLock<HashMap<String, bool>> = RwLock::new(HashMap::new());

pub fn enable(flag: &str) -> Result<(), FrameworkError> {
    let mut guard = FEATURE_FLAGS
        .write()
        .map_err(|_| FrameworkError::internal("feature flags lock poisoned"))?;
    guard.insert(flag.to_string(), true);
    Ok(())
}

pub fn is_enabled(flag: &str) -> Result<bool, FrameworkError> {
    let guard = FEATURE_FLAGS
        .read()
        .map_err(|_| FrameworkError::internal("feature flags lock poisoned"))?;
    Ok(guard.get(flag).copied().unwrap_or(false))
}
```

在一个处理程序内部，`is_enabled(...)?` 会沿着与其他每一个框架错误相同的 `FrameworkError → HttpResponse` 路径收拢：客户端得到一个清理过的 500，响应体是 `{"message": "Internal Server Error"}`，结构化日志会捕获带标签的污染消息，request id 从头到尾都被保留，进程的其余部分继续提供服务。完整的转换路径参见[错误处理](errors.md)章节。

在以下情况下使用这种模式：

- 调用者本来就返回 `Result`（大多数可能失败的操作都是如此）。
- 被污染的锁代表该子系统一次真实的、不可恢复的故障 - 没有什么明智的“部分真相”可以退回去用。
- 您希望运维人员在下一次碰到该子系统时，能在日志里*看到*这次污染。带标签的消息就是您的取证线索。

框架的通知分发器、邮件传输、mailable 注册表、数据库事件监听器和具名连接注册表，全都使用这种模式。其中任何一个发生 panic，都会在下一个碰到该注册表的请求上表现为一个 500；其余的一切照常运行。

### 模式 2 - 用 `into_inner()` 就地恢复

当调用者的签名*不*是可能失败的（一次 `bool` 查找、一次热路径的路由检查、一条请求生命周期所依赖的路径），或者共享状态在结构上即使经历一次部分写入之后仍然可以安全使用时，就恢复这个守卫并继续：

```rust
use std::collections::HashMap;
use std::sync::RwLock;

static ALLOWED_INCLUDES: RwLock<HashMap<&'static str, Vec<&'static str>>> =
    RwLock::new(HashMap::new());

pub fn allows(dto: &str, field: &str) -> bool {
    ALLOWED_INCLUDES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(dto)
        .map(|fields| fields.contains(&field))
        .unwrap_or(false)
}

pub fn register(dto: &'static str, fields: &'static [&'static str]) {
    let mut guard = ALLOWED_INCLUDES
        .write()
        .unwrap_or_else(|e| e.into_inner());
    guard.insert(dto, fields.to_vec());
}
```

`PoisonError::into_inner()` 无视污染，直接返回这个守卫。之后的读和写都会正常进行 - 对 `is_poisoned()` 查询来说，这把锁仍然是被污染的，但数据流已经恢复。

框架在 `data::registry`（每次 JSON:API 响应都会读取的 include 集合允许列表）、`auth::manager`（具名认证提供者映射）、`app::paths`（已解析路径缓存）、mail 和 events 的测试用伪造实现，以及 config 里已加载环境变量键的映射中，都使用这种模式。这些地方无一例外，要么没有哪个调用者手上有 `Result` 可以返回，要么其状态是只追加的、在结构上可以安全地继续使用。

在以下情况下使用这种模式：

- 调用者的签名很朴素（`bool`、`&str`，或是某个存储值的克隆），把它改成 `Result` 会迫使每一个调用者 - 有时甚至是每一个框架子系统 - 都得跟着向上冒泡。
- 共享状态能够容忍一次部分写入。只追加的映射和缓存是典型的例子：最坏情况不过是一个缺失或过期的条目，而调用者本来就会处理这种情况（默认拒绝、回退到主数据源、重新计算）。
- 这条热路径运行得足够频繁，以至于让此后的每一个请求都返回一个错误，从运维角度看会比降级更糟。

## 如何在两者之间做选择

一句话概括这个决策规则：**如果使用污染后状态的最坏情况是一个会带来后果的错误答案，就映射为错误；如果只是一个调用者本来就会处理的缺失或过期条目，就就地恢复。**

逐步来看：

1. **调用者的签名是 `Result<_, E>` 吗？** 如果不是，您就必须就地恢复 - 为了一个污染边缘情况，就给一个 `bool` 加上 `Result`，通常意味着一次项目级的重构，不值得。
2. **如果观察到一个写了一半的值，应用会不会做出一个带有现实后果的错误决策？** 向错误的客户收费、允许一个未经授权的 include、把访问权限授予错误的租户 - 这些情况的答案是“是，映射为错误”。对“这个名字注册过吗？”返回 `false` 并回退到主连接池 - 这种情况的答案是“不，就地恢复”。
3. **这个状态是只追加的，还是在重新注册时天然幂等？** 如果是，就地恢复就是安全的。如果一次写入是依赖于先前值的状态机迁移，就优先选择映射为错误，这样您就不会让一次损坏进一步复合。

拿不准的时候，就映射为错误。一个返回 500 的请求是一个您能够修复的醒目信号；悄无声息的错误答案则不是。

## 永远不要使用 `.lock().unwrap()`

禁止使用的写法：

```rust
// 永远不要这样写 - 调用图里这行代码之下的任何一处 panic
// 都会污染这把锁，而此后的每一个调用者
// 都会把这次污染变成另一次 panic。
let mut guard = SOMETHING.lock().unwrap();
```

`.expect("…")` 是同一回事，只是带了一条更友好的消息。两者都会把一个被污染锁的 `Err` 转换成一次 panic，而请求生命周期里 `AssertUnwindSafe(...).catch_unwind()` 这张网会接住它，并转换成一个 500 - 但这张网是*最后一道防线*，不是跳过上面那个决策的许可证。公共框架 API 和应用代码必须从上面两种被认可的模式里选一种。

在 `std::sync` 锁上使用 `.unwrap()` 可以被接受的两个例外：

- ***就是想*断言污染确实发生的测试构建代码** - `framework/src/lock.rs` 自己的污染诱导辅助函数，就故意在那个会 panic 的线程里使用了 `.unwrap()`。
- **一次已经失败的污染操作的错误路径** - 到了您身处 `poison_rw(...)` 那个线程内部的时候，panic *本身就是*重点。

如果您不属于这两种情况，就从上面那一节里选一种模式。

## 如果我的函数返回 `bool` 该怎么办？

这正是 `ConnectionRegistry::has` 所处的情况。它是执行器只读副本路由热路径上的一次 `bool` 查找，以 `if ConnectionRegistry::has("read_replica").await { … }` 这样的方式内联调用。把它拓宽成 `Result<bool, FrameworkError>`，会迫使执行器里的每一个调用者都用 `?` 向上冒泡，把一条内部错误的代码路径，传播进那些只想要一个是或否的路由决策里。

就地恢复模式能处理这种情况 - 返回 `false`，让调用者的回退逻辑接管（在这里，执行器会退回主连接池，这本来就是安全的行为）。为了确保运维人员仍然能看到这个情况，在第一次观察到污染时，发出一条一次性的 `tracing::warn!`：

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::collections::HashMap;

static REGISTRY: RwLock<HashMap<String, ()>> = RwLock::new(HashMap::new());
static POISON_WARNED: AtomicBool = AtomicBool::new(false);

pub fn has(name: &str) -> bool {
    match REGISTRY.read() {
        Ok(g) => g.contains_key(name),
        Err(_) => {
            // 竞态安全：只有第一个观察者会记录日志。
            if !POISON_WARNED.swap(true, Ordering::SeqCst) {
                tracing::warn!(
                    target: "myapp::registry",
                    "registry lock poisoned - `has({name})` degrading to false",
                );
            }
            false
        }
    }
}
```

这个基于 `swap` 的门控很重要：`RwLock` 的污染是粘性的，没有这道门控，此后的每一次调用都会重新触发这条警告，淹没您的日志。有了这道门控，每个进程、每个注册表就只会得到恰好一条警告，而同一个注册表上对应的、返回 `Result` 的 getter（`get`、`register`）会在下一次真正*需要*这次查找成功时，把污染暴露出来。这就给了运维人员两种信号：一条“有些地方不对劲”的早期警告，以及在一个请求真正依赖该注册表时给出的一记硬性 500。

## 框架已经保护了哪些东西

框架自己拥有的任何状态，您都不需要再去套用这条策略 - 它已经就位了。具体来说：

- 具名连接注册表（`ConnectionRegistry::register`、`get`、`has`）在写入和返回 `Result` 的读取上都把污染映射为 `FrameworkError::internal`；`has` 借助只警告一次的门控降级为 `false`。
- 通知分发器和工厂注册表、mailable 注册表、邮件传输、邮件内存捕获，以及数据库事件监听器，在发生污染时全都返回 `FrameworkError::internal`。
- `data::registry` 的 include 允许列表、`auth::manager` 的提供者映射、`app::paths`、已加载环境变量键的缓存，以及内存中的测试伪造实现，全都就地恢复。

当您通过这些子系统的公共 API（`Notification::send`、`Mail::send`、`Auth::user`、`DB::connection`、JSON:API 响应路径）与它们打交道时，一把被污染的框架锁，表现出来的只会是一个干净的 500 - 绝不会是您调用点上的一次 panic。

## 为什么 Suprnova 有所不同

Laravel 没有锁策略，因为它没有长期存活的共享状态。每一个 PHP 请求都拥有自己的进程、自己的内存、自己的每一个单例副本。没有内存中的注册表可以被污染，也没有“下一个请求”从上一个请求那里继承损坏这种概念 - 运行时保证了一张干净的白纸。

Suprnova 建立在 Tokio 之上，这恰恰给了您 PHP 所排除的那种长期存活的共享状态。低成本的 WebSocket、内存缓存、不需要重新付出代价去重建的连接池 - 所有这些都需要比任何单个请求活得更久的进程级全局注册表。拥有这种能力，正是把这类应用迁移到 Rust 的全部意义所在（框架完整的动机参见[简介](introduction.md)）。拥有它的代价是，您现在必须去思考：当一个发生 panic 的线程把共享状态留在一个被守卫的状态里时会发生什么 - 因为现在*确实*有共享状态可以被留下。

这套两模式的策略，是在保留这种能力的同时消除代价的最小答案。在状态可以安全继续使用的地方就地恢复；在您宁愿要一个干净的 500 也不要一个错误答案的地方映射为错误。这两种选择都能让进程的其余部分继续提供服务。两者都不会留下一个发生 panic 的 unwrap，等着拖垮它上方的子系统。

这与框架应用在不可达的缓存和速率限制后端上的[失败开放与失败关闭决策](rate-limiting.md)是同一种模式：在调用点做出一个明确的策略选择，而不是套用一个默认值。无处不在的异步给了您长期存活的状态；框架则给了您让它保持诚实的操作手册。

## 下一步

- [错误处理](errors.md) - `FrameworkError::internal` 如何变成客户端收到的那个清理过的 500，同时把带标签的污染消息保留在您的结构化日志里。
- [服务容器](container.md) - 这条策略所保护的进程级全局注册表实际存放在哪里，以及为什么任务本地/线程本地作用域能防止测试之间互相继承对方的绑定。
- [请求生命周期](lifecycle.md) - 捕获*最后手段*的 unwrap 并将其转换为 500 的 panic 边界（`execute_chain_safely`），让您准确理解这张安全网做了什么，以及为什么它不是跳过上述策略的借口。
- [速率限制](rate-limiting.md) - 针对那些可能*不可达*而非被污染的后端，讲的是一套平行的 `BackendErrorPolicy` 方案；同样的显式选择原则，不同的故障模式。
- [测试](testing.md) - `TestContainer::fake` 和线程本地容器层如何防止并行测试污染彼此的注册表，这是污染处理方案在测试时刻的另一半。
