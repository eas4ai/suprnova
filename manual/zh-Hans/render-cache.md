# RenderCache

RenderCache 会存储一份已证明安全的 GET 或 HEAD 路由响应副本，并用它来服务下一个
匹配的请求，完全不运行你的处理程序。你需要显式地将路由和分组接入其中；其余一切
都照旧运行，与今天完全一样。一个你从未接入的路由不受任何影响。一个你已经接入的
路由，即便这个特定请求最终证明没有什么地方是可以安全缓存的，仍然会正确地渲染并
提供服务 - 它只是永远不会被存储，而你可以查明原因。

本章覆盖启用缓存、接入路由与分组、声明差异化维度、读取它添加的响应头、渲染被
拒绝的原因、运维控制，以及它与 `suprnova::Cache` 的区别。

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
`APP_BUILD_ID`（默认是你的 crate 自身的版本）把每一个缓存条目都限定在生成它的
那次构建的命名空间下，因此一次部署永远不会返回旧构建的字节。

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
表示保持新鲜的时长、在后台重建运行期间它还可以继续被服务多久，以及如果那次重建
彻底失败，它还可以再被服务多久。`RepresentationClass` 按共享范围从最宽到最窄
排列：`PublicShared`（为每一个匹配已声明差异化维度的访客提供同一份表示）、
`PublicShellStitched`（为未来的组合式外壳表示保留，目前尚不可用）、
`PrivateCached`（为每一个已登录访客或租户各提供一份表示），以及 `Uncacheable`。

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

let router = add_render_cache(router)?;
let router = RenderCache::install(router, RenderCacheConfig::from_env()).await?;
```

## 声明差异化维度

默认情况下，一份缓存的表示只按路由模式、路径参数和应用构建进行区分。你的处理
程序输出实际依赖的其他任何东西，都需要通过两种机制来声明：

- **查询参数。** `.query(QueryPolicy::declared(["page", "sort"]))` 指名哪些
  查询参数会区分不同的表示；请求中出现的任何其他查询参数都会让该请求绕过缓存，
  而不是被悄悄忽略。
- **差异化维度**，通过 `.vary(dimension)` 逐个添加：
  - `VarianceDimension::Locale` 按协商出的语言环境分区。
  - `VarianceDimension::Media` 按协商出的媒体类型分区。
  - `VarianceDimension::Host` 按请求的主机分区，适用于你的部署让不止一个主机
    具有意义的情况。
  - `VarianceDimension::Tenant` 把当前租户作为不透明的键材料来分区；任何处理
    程序会读取租户的路由都必须声明它。
  - `VarianceDimension::Principal` 把已登录访客作为不透明的键材料来分区，并
    绑定到一个权限版本（见下文“纪元、权限与检查”）；一个 `PrivateCached` 路由
    必须声明 `Principal` 或 `Tenant`（或两者都声明），否则根本无法构建成功。

`VarianceDimension::FeatureVersion`、`VarianceDimension::ConfigVersion`，以及
自定义的 `VarianceDimension::Application(name)` 都存在于这个类型上，但在本
版本中没有解析器：一个声明了其中之一的路由会在每一个请求上悄悄绕过缓存，而不是
构建失败。目前请不要声明它们。

## 读取响应头

一次被服务的命中携带 `ETag`（一个强验证器，你的客户端可以把它作为
`If-None-Match` 送回来换取一个 `304`）、`Cache-Control`（默认是 `private`，
除非类别是 `PublicShared` 且你设置了 `SharedCachePolicy::SMaxAge`，此时它还
携带 `public` 和 `s-maxage`）、`Vary`（来自任何隐含它的已声明维度 - `Locale`
隐含 `Accept-Language`，`Media` 隐含 `Accept`），以及 `Age`（自该表示发布以来
经过的整秒数）。一个陈旧但仍可服务的响应还会额外携带
`Warning: 110 - "Response is Stale"`。

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
  `Uncacheable`，不论该路由声明了什么差异化维度。当一个匿名访客的身份是通过
  会话回退来解析的时候，这条规则同样会触发 - 这是一个常见的意外，因为该访客
  确实是匿名的，得到的键也正确地是 `Anonymous`，但这次读取本身仍然是一次会话
  读取。
- **你在一个没有声明 `Principal` 的路由上读取了身份。** 读取已登录用户会把
  类别收窄为 `PrivateCached`；如果该路由声明的差异化维度中不包含 `Principal`，
  就没有办法按访客对条目分别建键，因此它会被拒绝存储，而不是被共享。
- **你（或者你的视图引擎）在没有声明 `Locale` 的情况下做了翻译。** 对协商出的
  语言环境的任何读取都需要一个已声明的 `Locale` 维度，否则该次渲染会被拒绝
  存储。每一个 Inertia 页面的文档外壳都会读取语言环境来设置 `<html lang>`，
  无论该页面自身的数据是否与语言有任何关系 - 所以一个 Inertia 路由要想被缓存，
  就需要声明 `Locale`，即便它自己完全没有翻译内容。
- **你检查了授权。** `Gate` 总是把一次判定当作是按访客划分的，所以即便在一个
  只按 `Tenant` 建键的路由上，它也需要声明 `Principal`，除非该 gate 自身的
  检查能被证明是按租户划分的。RenderCache 自己无法分辨这种区别。
- **页面背后的某个模型带有一个按租户划分的全局作用域。** 一个从自己的请求本地
  状态中读取当前租户来过滤查询的全局作用域 - 也就是 Suprnova 自己的
  `GlobalScope` 文档所展示的那种模式 - 会在 RenderCache 完全看不到那次读取的
  情况下，改变查询返回的内容。请在任何由这样一个模型支撑的路由上声明 `Tenant`
  差异化维度；这里没有任何机制能替你捕捉这种遗漏。
- **你读取了一个秘密配置值，或者一个未声明的请求上下文。** 两者都会强制变为
  `Uncacheable`。一个响应对普通请求头、或者对 `Config::get` 的依赖，对
  RenderCache 来说完全不可见 - 它无法拒绝它看不见的东西，所以声明匹配的
  差异化维度是你自己的责任。

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

- **`RenderCache::bump_permission_version()`** - 每当应用中的某个动作改变了
  一个已登录用户被允许做什么（一次角色变更、一次权限授予或撤销）时，就调用它。
  不这样做的话，一个权限刚刚发生变化的用户，仍然会命中在其先前权限集合下缓存的
  内容。
- **`RenderCache::advance_epoch()`**，或者隐藏命令 `render-cache:epoch-advance` -
  一次紧急失效操作。每一个当前已存储的条目，会在它的下一次请求上立即变得
  无法通过普通查找触及，因为纪元本身就被烘焙进了查找键。进程内层级也会在同一
  时刻被彻底清空；一个文件支持的层级会把旧文件留在磁盘上，直到周期性或手动的
  清扫回收它们为止，这属于磁盘卫生问题，而不是正确性问题。当缓存内容出了问题、
  而你等不及各个条目自行过期时，就用这个。
- **隐藏命令 `render-cache:inspect <key>`** 通过你的应用日志或遥测能够呈现的
  那段键文本，报告某一个已存储条目的元数据（绝不是它的正文），并连同当前纪元
  一并给出，让你能判断自己正在查看的究竟还是有效权威，还是早已在背后过期。

## RenderCache 与 `suprnova::Cache` 的区别

`suprnova::Cache` 是一个你显式调用的键值存储：你选择键，你选择存储什么，你
选择何时使它失效（`Cache::put`、`Cache::get`、`Cache::remember`、
`Cache::forget`）。它适用于你的代码判定值得缓存的任何数据，运行在你配置的任何
后端上（内存或 Redis）。

RenderCache 不是一个通用存储，你也永远不会从处理程序里调用它。它缓存的是整个
HTTP 响应，键是从路由及其已声明的差异化维度自动派生出来的，而失效是基于世代的：
一次通过 ORM 或查询构造器完成的普通数据库写入，会推进该次渲染所依赖的那些世代，
条目会在下一次被请求时重新计算，而不是被手动删除。当你有一个想要计算一次并复用
的具体值时，用 `suprnova::Cache`；当你有一整个路由、其响应渲染代价高昂且可以
安全共享时，用 RenderCache。
