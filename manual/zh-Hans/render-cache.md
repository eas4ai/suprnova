# RenderCache

RenderCache 会存储一份已证明安全的 GET 或 HEAD 路由响应副本，并用它来服务下一个
匹配的请求，完全不运行你的处理程序。你需要显式地将路由和分组接入其中；其余一切
都照旧运行，与今天完全一样。一个你从未接入的路由不受任何影响。一个你已经接入的
路由，即便这个特定请求最终证明没有什么地方是可以安全缓存的，仍然会正确地渲染并
提供服务 - 它只是永远不会被存储，而你可以查明原因。

本章覆盖启用缓存、接入路由与分组、声明差异化维度、读取它添加的响应头、渲染被
拒绝的原因、运维控制，以及它与 `suprnova::Cache` 的区别。

## 这几章

这是五章中的第一章。第一次请按顺序读完；此后，每一章都能自己回答一个问题。

| 章节 | 回答什么 |
|---|---|
| RenderCache（本章） | 我怎么把它打开，并把一个路由接入进来？ |
| [表示](render-cache-representations.md) | 实际被存储的到底是什么，又在什么键下？ |
| [世代](render-cache-generations.md) | 一份已存储的副本什么时候不再是最新的？ |
| [部署](render-cache-deployment.md) | 多个节点如何共享同一个缓存？ |
| [运维](render-cache-operations.md) | 我怎么检视、测试、度量它，以及把它关掉？ |

## 启用缓存

有两个环境变量在起步阶段很重要：

- `RENDER_CACHE_ENABLED` - 默认为 `true`，除非被设为 `false` 或 `0`。禁用后，
  每一个请求都会完全绕过 RenderCache；既不会查找任何东西，也不会存储任何东西。
- `RENDER_CACHE_L1_DIR` - 默认未设置，即没有磁盘层级。把它设为一个进程可以创建
  并写入的目录，存储的表示就会在一个文件支持的第二层级中挺过进程重启。

还有少数几个变量用于调整默认值：`RENDER_CACHE_L0_ENTRIES`（4,096）和
`RENDER_CACHE_L0_BYTES`（128 MiB）约束进程内层级；`RENDER_CACHE_L1_BYTES`
（1 GiB）约束文件层级；`RENDER_CACHE_FAILURE`（默认为 `open`，或者 `closed`）
决定存储或数据库出问题时，是让路由以不缓存的方式提供服务，还是直接拒绝该请求；
`APP_BUILD_ID` 把每一个缓存条目都限定在生成它的那次构建的命名空间下。请把它
显式设为每次部署都会变化的东西：它的默认值是一个编译进来的 crate 版本，而那个
版本并不会变。参见 [RenderCache 部署](render-cache-deployment.md)。

`RENDER_CACHE_PROFILE`（默认为 `embedded`，或者 `database`、`redis`）决定第二
层级和重建协调器是在本进程内，还是与其他每一个节点共享。一个共享的配置档还需要
一个由你的应用列出的迁移。这两件事，连同完整的变量表，都是
[部署](render-cache-deployment.md)那一章的主题。

## 接入一个路由或一个分组

在你明确表态之前，什么都不会被缓存。`Router::try_render_cache` 接入一个已注册
的路由模式；`Router::try_render_cache_group` 接入某个路径前缀下的每一个路由。
两者都接受一个用 `RenderCachePolicy::builder` 构建的策略：

```rust
use suprnova::{FrameworkError, Router};
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy,
};

fn add_render_cache(router: Router) -> Result<Router, FrameworkError> {
    router.try_render_cache_group(
        "/blog",
        RenderCachePolicy::builder(RepresentationClass::PublicShared)
            .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
            .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
            .build()?,
    )
}
```

`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)` 设定一个
表示保持新鲜的时长，以及从那条新鲜边界起算的两个窗口：在一次后台重建运行期间，
已存储的副本还可以越过它多远继续被服务；以及如果一次前台重建彻底失败，已存储的
副本还可以越过它多远被服务。这两个窗口不是叠加的；参见
[RenderCache 表示](render-cache-representations.md)。

`RepresentationClass` 按共享范围从最宽到最窄排列：`PublicShared`（为每一个匹配
已声明差异化维度的访客提供同一份表示）、`PublicShellStitched`（一份 Live 文档，
它共享的外壳只被存储一次，而它的岛屿会为每一个发起询问的人重新挂载；参见
[表示](render-cache-representations.md)）、`PrivateCached`（为每一个已登录访客
或租户各提供一份表示），以及 `Uncacheable`。

一个路由模式必须先被注册，然后才能接入；并且你必须在调用 `RenderCache::install`
（见下文）**之前**完成路由与分组的全部接入 - 安装这一步只会读取到那个时间点为止
已经注册的内容。

路由级别的策略也可以是其外层分组的一个收窄式补丁，用 `PolicyPatch` 而不是一个
完整的 `RenderCachePolicy`：它继承分组声明的一切，并且只能把它变窄（更短的
新鲜度窗口、更严格的类别），绝不能变宽。把一个路由整体从已缓存的分组中剔除，就
是一个把类别设为 `Uncacheable` 的 `PolicyPatch`。

用一行代码完成 RenderCache 的接入，放在每一个建立请求范围的语言环境、会话或
身份的中间件注册之后（RenderCache 会读取它们来构建自己的查找键，所以它需要在
设置这些内容的一切之后运行）：

```rust
use suprnova::RenderCache;
use suprnova::render_cache::RenderCacheConfig;

Application::new()
    // ...
    .try_routes_async(|| async {
        let router = add_render_cache(routes::register())?;
        RenderCache::install(router, RenderCacheConfig::from_env()).await
    });
```

## 声明差异化维度

默认情况下，一份缓存的表示只按路由模式、路径参数和应用构建进行区分。你的处理
程序输出实际依赖的其他任何东西，都需要通过两种机制来声明：

- **查询参数。** `.query(QueryPolicy::declared(["page", "sort"]))` 指名哪些
  查询参数会区分不同的表示；请求中出现的任何其他查询参数都会让该请求绕过缓存，
  而不是被悄悄忽略。
- **差异化维度**，通过 `.vary(dimension)` 逐个添加：
  - `VarianceDimension::Locale` 按协商出的语言环境分区。
  - `VarianceDimension::Host` 按请求的主机分区，适用于你的部署让不止一个主机
    具有意义的情况。
  - `VarianceDimension::Tenant` 把当前租户作为不透明的键材料来分区；任何处理
    程序会读取租户的路由都必须声明它。
  - `VarianceDimension::Principal` 把已登录访客作为不透明的键材料来分区，并
    绑定到一个权限版本（见下文“纪元、权限与检查”）；一个 `PrivateCached` 路由
    必须声明 `Principal` 或 `Tenant`（或两者都声明），否则根本无法构建成功。
- **`Media` 和 `Encoding`**，与各自专属的封闭集合一起声明：
  `.vary_media(NegotiatedPolicy::declared(["text/html", "application/json"], "text/html")?)`
  和
  `.vary_encoding(NegotiatedPolicy::declared(["identity", "gzip"], "identity")?)`。
  裸的 `.vary(VarianceDimension::Media)`（或 `::Encoding`）会在 `build`/
  `apply` 处被拒绝：和其他每一个维度都不同，这两个维度要对照一个只有
  路由才能指名的集合来协商，所以没有它就没有可以拿来作键的东西。

  协商会读取请求的 `Accept`（用于 `Media`）或 `Accept-Encoding`（用于
  `Encoding`）请求头，把它对照声明的集合做匹配，并把匹配到的请求头
  加入 `Vary`。它按 `q` 加权：声明的成员里质量最高的那个胜出，质量
  相同时保留请求头自身从左到右的顺序，所以排在前面的候选项赢得平局。
  通配符（`*/*`、`type/*`、单独的 `*`）会被当作字面 token 来比较，而
  不会展开去匹配集合，所以它实际上几乎不会命中任何一个真正声明过的值。
  一个缺失的请求头、一个没有指名声明集合里任何东西的值，或者一个让
  这套逻辑理解不了的请求头 - 比如 `q=0`、一个超出范围或者无法解析的
  质量值、乱七八糟的语法 - 都会解析成声明的默认值，而不是创建一个
  变体或者让请求失败。两个不同的协商结果值就是两个不同的键；同一个
  协商结果值，不管它在线上是怎么拼写或者加权的，永远都是同一份被
  存储的表示。

`VarianceDimension::FeatureVersion`、`VarianceDimension::ConfigVersion`，以及
自定义的 `VarianceDimension::Application(name)` 都存在于这个类型上，但在本
版本中没有解析器：一个声明了其中之一的路由会在每一个请求上悄悄绕过缓存，而不是
构建失败。目前请不要声明它们。

## 读取响应头

一次被服务的命中携带 `ETag`（一个强验证器，你的客户端可以把它作为
`If-None-Match` 送回来换取一个 `304`）、`Cache-Control`、`Vary`，以及 `Age`
（自该表示发布以来经过的整秒数，也是判断一个响应出自存储而不是出自你的处理程序
最快的本地迹象）。一个越过其新鲜区间之后才被服务的响应还会额外携带
`Warning: 110 - "Response is Stale"`。这五者中的每一个，连同自用（dogfood）路由
被断言会发出的取值，都定义在
[RenderCache 表示](render-cache-representations.md)。

## 为什么一次渲染永远不会被存储

被接入并不是保证。每一次渲染之后都会运行两项相互独立的检查，任何一项都可以拒绝
存储而不会使请求失败 - 无论哪种情况，你拿到的响应都是一样的，它只是永远不会
成为一个缓存条目：

**资格审查**会直接拒绝这样的响应：不是对 `GET` 或 `HEAD` 的一个朴素 `200`、
以流方式发送正文、设置了 Cookie，或者携带一个逐跳或追踪用的头。这些情况几乎
总是意外发生的（一次重定向、一个错误页面、一个碰巧触及 `Set-Cookie` 的
响应），而不是你需要专门去设计规避的东西。

**分类**基于你的处理程序在运行期间实际做了什么来拒绝存储，用的都是你能认出来的
说法：

- **你读取了一个会话值。** 对当前会话的任何读取（通过 `session()`、
  `session_mut`，或者一个会话 Cookie）都会把这次渲染永久性地强制变为
  `Uncacheable`，不论该路由声明了什么差异化维度。它*不*覆盖的唯一一样东西，是
  已登录访客自己的身份。当请求中更早的环节都没有解析出身份时，`Auth::id()` 会
  从会话里把它读出来，而那次读取被归类为一次身份读取，而不是一次会话读取 - 所以
  一次普通的、由 Cookie 支撑的登录，正是一个声明了 `Principal` 差异化维度的
  `PrivateCached` 路由所要服务的场景，去取访客的 id 并不会悄悄让页面变得不可
  缓存。会话中其他每一个值仍然会。有两个后果值得知道：一个发往这样一个路由的
  匿名请求会缓存在 `Anonymous` 键下，因为这次渲染没有解析出任何身份，没有观察到
  任何主体材料，而键也如实这么说 - 一个已登录访客派生出的是一个 `Private` 键，
  它永远触及不到那个条目；以及，一个具名 guard 自己的标识符，与默认 guard 的
  标识符在完全相同的意义上属于主体材料。
- **你在一个没有声明 `Principal` 的路由上读取了身份。** 读取已登录用户会把
  类别收窄为 `PrivateCached`；如果该路由声明的差异化维度中不包含 `Principal`，
  就没有办法按访客对条目分别建键，因此它会被拒绝存储，而不是被共享。
- **你（或者你的视图引擎）在没有声明 `Locale` 的情况下做了翻译。** 对协商出的
  语言环境的任何读取都需要一个已声明的 `Locale` 维度，否则该次渲染会被拒绝
  存储。每一个 Inertia 页面的文档外壳都会读取语言环境来设置 `<html lang>`，
  无论该页面自身的数据是否与语言有任何关系 - 所以一个 Inertia 路由要想被缓存，
  就需要声明 `Locale`，即便它自己完全没有翻译内容。
- **你检查了授权。** 一次判定是依据它自身的求值读取了什么来评判的。一个
  gate，如果它的主体只读取了租户 - 比如通过
  `suprnova::live::current_tenant()` - 就只按 `Tenant` 分类，并在一个
  按 `Tenant` 建键的路由上缓存。一个读取了某个按用户区分的事实的
  gate，或者一个没有读取任何 RenderCache 能看见的东西的 gate，仍然
  需要声明 `Principal`：一个未经过任何被计测的访问器、直接从其
  `user` 参数做出判定的主体，与一个从常量做出判定的主体是无法区分的，
  而对此保守的解读才是安全的解读。
- **页面背后的某个模型带有一个读取按请求状态的全局作用域。** 请声明该
  作用域依赖于什么。一个返回 `ScopeDependency::Constant` 的
  `GlobalScope` 什么都不记录，也不会损失任何缓存命中。默认值
  `ScopeDependency::PerRequest` 要求该作用域的 `apply` 通过一个被
  计测的访问器读取那份状态 - 例如 `suprnova::live::current_tenant()`、
  `Auth::id()`、`Lang::locale()`。一个按请求划分、但其求值没有读取
  这三者中任何一个的作用域，会把渲染收窄为 `Uncacheable`，并在拒绝
  存储时报出自己的名字，因此一个不可见的租户过滤器付出的代价是让
  你失去缓存，而不是让你的访客们彼此看到对方的行。
- **你读取了一个秘密配置值，或者一个未声明的请求上下文。** 两者都会强制变为
  `Uncacheable`。一个响应对普通请求头、或者对 `Config::get` 的依赖，对
  RenderCache 来说完全不可见 - 它无法拒绝它看不见的东西，所以声明匹配的
  差异化维度是你自己的责任。
- **你通过 `DB::select`、`DB::select_one`、`DB::scalar` 或
  `DB::select_on` 运行了原始 SQL。** 框架无法为一条原始语句
  读取的表命名，因此这次渲染永远不会被存储；它仍然会被正常
  提供服务。通过 `DB::table(..)` 进行的读取知道自己的表，
  会被正常缓存，`Auth::user()` 也是如此，因为它正是通过这条
  路径解析的。
  框架自身的 RBAC 角色与权限检查会指明它们所读取的五张表 - `roles`、
  `permissions`、`role_permissions`、`model_roles`，以及
  `model_permissions` - 因此一个执行了其中之一的已缓存路由会被精确地
  观测到，并按正常方式被缓存。
- **这次写入是由一个队列工作进程、一个计划任务或一个控制台
  命令做的。** 现在已经不再需要任何特殊处理。任何配置启用了
  RenderCache、且其数据库持有 RenderCache 迁移的进程都会推进
  世代，因此这样一次写入所使无效的内容，与同一次写入在服务器
  中所使无效的内容完全一致，`RenderCache::bump_permission_version()`
  在其中任何一个进程里都能正常工作。一个
  `RENDER_CACHE_ENABLED=false` 的进程，或者一个数据库没有持有
  该迁移的进程，则什么都不会推进，也完全不会发出任何 RenderCache
  SQL。

在 PostgreSQL 上，渲染运行在一个 `REPEATABLE READ` 事务
中，好让它读到的内容与它记录下的那些世代保持一致；一个已
缓存路由的处理程序如果更新了某一行，而这一行在渲染开始之
后被另一个事务改动过，就会遇到一次序列化失败。把被缓存的
路由设计成读路径。一个在渲染事务内部执行写入的处理程序仍
然会推进世代，但它会为同一些行与并发写入者竞争，并可能遇
到上面提到的那次序列化失败。

一次在任何事务之外完成的写入（单独调用 `model.save()`）会
先提交，再在紧随其后的一个事务中推进它的那些世代，所以两者
之间的那个瞬间是“新数据，旧世代”：多一次重建，但绝不会有
陈旧内容。

在实践中看到这一切发生并不需要任何特殊工具：隐藏命令 `render-cache:inspect`
（见下文）会显示一个路由的条目究竟存不存在，或者你也可以直接连续发出两个请求，
检查第二个请求是否携带一个 `Age` 头。

## 一个会缓存的路由

一个没有任何按访客区分内容的公共列表页面：

```rust
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    Ok(HttpResponse::html(render_post_list(&posts)))
}
```

注册并接入之后：

```rust
use suprnova::{get, routes};
use suprnova::render_cache::{FreshnessPolicy, RenderCachePolicy, RepresentationClass, SharedCachePolicy};

routes! {
    get!("/blog", controllers::blog::index),
}

router.try_render_cache(
    "/blog",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .shared(SharedCachePolicy::SMaxAge { seconds: 300 })
        .build()?,
)?;
```

`index` 从不触碰会话、已登录访客或语言环境，因此第一个请求会渲染并发布；接下来
五分钟内的每一个请求都会从那份存储的副本中获得服务，带着一个 `Age` 头，对已经
持有它的客户端返回一个 `304`，并为前面的任何 CDN 提供
`Cache-Control: public, max-age=300, s-maxage=300`。

## 一个被拒绝存储的路由

同样形态的页面，但处理程序读取会话来显示一条 flash 消息：

```rust
use suprnova::session::session;
use suprnova::{handler, HttpResponse, Response};

#[handler]
pub async fn index() -> Response {
    let posts = Post::query().order_by_desc("published_at").get().await?;
    let flash = session().and_then(|s| s.get::<String>("status"));
    Ok(HttpResponse::html(render_post_list_with_flash(&posts, flash.as_deref())))
}
```

接入方式与上面完全相同。每一个请求仍然会渲染并提供正确的页面 - flash 消息也
包含在内 - 但什么都不会被存储：这次会话读取会在 RenderCache 甚至还没到达资格
审查之前，就把类别收窄为 `Uncacheable`，因此对同一个 URL 的第二个请求会从头
重新渲染，而不是带着一个 `Age` 头回来。如果这个页面确实打算被缓存，修复方法是
不要在被缓存的路径里读取会话（改为从一个查询参数或者一个单独的小型响应中渲染
flash 消息）- 没有任何差异化维度声明能让一次会话读取变得可以缓存，因为一次
会话读取意味着这个响应依赖的东西，没有任何键能够安全地据以划分。

## 纪元、权限与检查

- **`RenderCache::bump_permission_version().await?`** - 每
  当应用中的某个动作改变了一个已登录用户被允许做什么（一次
  角色变更、一次权限授予或撤销）时，就调用它。它会推进一个
  持久化的世代，每一个按访客建键的渲染都会关注这个世代。这
  个世代能挺过一次重启，并且这次调用会在角色变更运行在某个
  事务中时，加入那个事务。不进行这次调用的话，一个权限刚刚
  发生变化的用户，仍然会命中在其先前权限集合下缓存的内容。
- **`RenderCache::advance_epoch()`**，或者隐藏命令
  `render-cache:epoch-advance` - 一次紧急失效操作。纪元本身就被烘焙进了查找
  键，所以推进它会让已存储的条目变得无法触及，既没有什么要枚举的，也没有什么
  要删除的。在运行这条命令的那个进程上，效果是立即的：它会丢弃该进程的纪元
  租约，并在同一时刻清空它的进程内层级。另一个节点会在它的下一次权威读取时
  跟上，而它那个文件支持的层级会把旧文件一直留着，直到一次清扫回收它们为止 -
  每第 256 次发布触发的那次自动清扫，或者一次显式的 `RenderCache::sweep()` -
  这属于磁盘卫生问题，而不是正确性问题。当缓存内容出了问题、而你等不及各个
  条目自行过期时，就用这个；在不止一个节点上的做法，参见
  [RenderCache 运维](render-cache-operations.md)。
- **隐藏命令 `render-cache:inspect <key>`** 通过你的应用日志或遥测能够呈现的
  那段键文本，报告某一个已存储条目的元数据（绝不是它的正文），并连同当前纪元
  一并给出，让你能判断自己正在查看的究竟还是有效权威，还是早已在背后过期。它
  只在运行中进程的进程内层级里查这个键，绝不去共享层级里查，所以在一个
  `database` 或 `redis` 配置档下，对于一个本节点自己没有服务过的键，它会报告
  没有条目。

## RenderCache 与 `suprnova::Cache` 的区别

`suprnova::Cache` 是一个你显式调用的键值存储：你选择键，你选择存储什么，你
选择何时使它失效（`Cache::put`、`Cache::get`、`Cache::remember`、
`Cache::forget`）。它适用于你的代码判定值得缓存的任何数据，运行在你配置的任何
后端上（内存或 Redis）。

RenderCache 不是一个通用存储，你也永远不会从处理程序里调用它。它缓存的是整个
HTTP 响应，键是从路由及其已声明的差异化维度自动派生出来的，而失效是基于世代的：
一次通过 ORM 或查询构造器完成的普通数据库写入，会推进该次渲染所依赖的那些世代，
条目会在下一次被请求时重新计算，而不是被手动删除；一次读取了原始 SQL 的渲染，
一开始就不会被存储，因此也没有什么可重新计算的。当你有一个想要计算一次并复用
的具体值时，用 `suprnova::Cache`；当你有一整个路由、其响应渲染代价高昂且可以
安全共享时，用 RenderCache。

### 为什么 Suprnova 有所不同

Laravel 框架本身没有对等物。响应缓存是一个你自己加进来的包，它用中间件把路由
包起来，按一个你自己拼出来的键存下已渲染的响应，此后的一切都归你：哪些路由可以
安全缓存、是什么让两个访客不同，以及一个已存储的页面什么时候不再为真。框架并不
知道某个页面被缓存过，所以它没法告诉你什么时候缓存它是个错误。

RenderCache 之所以是框架的一部分，正是因为这个。它看得见渲染的发生，所以它能
记录处理程序读了什么，把这些与路由声明的内容作对照，并拒绝存储一个它无法为其
安全性交代清楚的响应 - 而且是静默地拒绝，不改变访客被提供的东西。把一个路由
接入进来是一份声明，框架此后会照着它来要求你，而不是一个你对自己许下的承诺。
代价是有些你很想缓存的路由会被拒绝，你得去查明原因；好处是那些确实被存储下来
的，是由渲染它们的那个进程当场证明过可以安全存储的。

## 下一步

- [RenderCache 表示](render-cache-representations.md) - 实际被存储的是什么、
  在什么键下，以及在哪些层级里
- [RenderCache 世代](render-cache-generations.md) - 一份已存储的副本如何不再
  是最新的
- [缓存](cache.md) - 本章拿来作对比的那个显式键值存储
- [Live](live.md) - 一份缝合式表示是从哪些文档里裁下来的
