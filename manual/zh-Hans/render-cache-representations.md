# RenderCache 表示

一个被缓存的路由存的不是“一个页面”。它存的是一份**表示**：一个具体的响应，处在
一个查找键之下、一个或多个存储层级里，旁边还带着足够的元数据，用来应答一次条件
请求，并在此后证明它仍然是最新的。只有当两个访客派生出的键是同一个键时，他们才
会拿到同样的已存储字节；而这个键是从路由声明的内容派生出来的 - 绝不是从处理程序
碰巧做了什么派生出来的。

本章讲的就是那个被存下来的东西。一份表示可以采取哪些形态（`Complete` 和
`Composite`）、它的键里放了什么、它被写入哪些层级、一次被服务的命中携带的
`ETag`、`Cache-Control`、`Vary`、`Age` 和 `Warning`、它可能处在的四种新鲜度
状态、它如何应答 `If-None-Match` 和 `HEAD`，以及 `PrivateCached` 和
`PublicShellStitched` 实际存了什么。一份表示*为什么*会离开新鲜区间 - 一次写入、
一次纪元推进 - 是下一章的主题；在这里，只要知道这些区间存在、并且有一份表示落
在其中之一里就够了。下面每一个例子都是本仓库自用（dogfood）应用
（`app/src/live/mod.rs`）里的一个路由，并由 `app/tests/live_render_cache.rs`
中一个具名测试来证明。

## 两种条目形态

一个已存储的条目是两种类别之一。

- **`Complete`** 是一个完成了的答案：一个状态码、一组可重放的响应头，以及一个
  正文缓冲区。服务它不复制任何东西，也不运行任何东西。每一个 `PublicShared` 和
  `PrivateCached` 路由存的都是这种形态。
- **`Composite`** 是一个共享的**外壳**，上面开了带类型的洞，外加一张分段图，
  说明每个洞里该放回去什么。只有
  `RepresentationClass::PublicShellStitched` 存这种形态，而且只有一份 Live
  文档才会产生一个。

你在策略里声明的类别，决定了哪一种形态才是可达的。`/live/public` 和
`/live/todos` 都声明了 `PublicShared`；
`the_database_profile_serves_a_hit_through_the_sql_stores` 会把 `/live/todos`
已发布的条目从存储里读回来，断言它是一个 `EntryKind::Complete` 条目；而
`the_public_document_is_a_hit_whose_seed_still_promotes` 则通过
`RenderCache::inspect_route_for_test` 把 `/live/public` 的条目读回来，断言它被
存储在哪个类别之下。这一点很重要，因为“它被存下来了”和“它被静默拒绝了”产生的是
同一个响应：这个主张必须针对条目来提出，而不是针对访客看到的东西。

## 查找键

一个请求派生出的键，是由路由模式、它的路径参数、策略声明的那些查询参数、每一个
已声明差异化维度解析出的值、应用构建 id（`APP_BUILD_ID`），以及当前的权威纪元
构成的。没有别的了。一个随请求到来、但没有被 `QueryPolicy::declared` 指名的
查询参数，会让该请求绕过缓存，而不是被悄悄从键里丢掉，因为丢掉它就意味着给发出
它的人提供一个错误的页面。

这个键是运维人员可以拿在手里的文本：
`the_operator_commands_inspect_without_a_body_and_advance_the_epoch` 里的
`RenderCache::key_for_route_for_test` 断言它以 `rk1.` 开头，而
`render-cache:inspect` 接受的正是那段文本。

因为纪元是键的一部分，一次纪元推进不必去找出并删除任何东西。此前存储的每一个
条目，只是在下一次请求时不再能通过普通查找触及而已。
[运维](render-cache-operations.md)那一章的紧急失效，靠的就是这个机制。

## 一个策略会写入哪些层级

存储层级有两个。**L0** 是进程内内存，受 `RENDER_CACHE_L0_ENTRIES` 和
`RENDER_CACHE_L0_BYTES` 约束。**L1** 是部署配置档所配置的任何东西 - 一个文件
目录、一张数据库表，或者 Redis - 并且由每一个指向它的进程共享。

除非你另行表态，否则策略构建器**只**存入 **L0**：`StorageLayers::l0_only()` 是
默认值。一个值得放进共享层级的路由要自己声明：

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, StorageLayers,
};

let router = router.try_render_cache(
    "/live/todos",
    RenderCachePolicy::builder(RepresentationClass::PublicShared)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .layers(StorageLayers::l0_and_l1())
        .build()?,
)?;
```

这就是 `app/src/live/mod.rs` 里 `/live/todos` 的声明。它是那个应用中唯一一份
字节可以被每个节点共享的文档，所以它也是唯一一个声明了 `l0_and_l1()` 的。在
`embedded` 配置档下，除非 `RENDER_CACHE_L1_DIR` 指定了一个目录，否则 L1 处于
禁用状态，此时声明这个层级什么也不会改变；在 Database 配置档下，条目会落进
`suprnova_render_entries`，第二个进程能在那里找到它。

`the_database_profile_serves_a_hit_through_the_sql_stores` 就是证明。它在
Database 配置档的提供者上启动应用，用中间件派生出的那个键，直接从 L1 里读出已
发布的条目，然后清空 L0 再问一次 - 而第二个请求依然是在处理程序没有运行的情况
下被应答的。从客户端一侧看，一次内存命中长得一模一样，这正是这个测试要伸手去够
存储的原因。

请按路由而不是全局地挑选层级。在一次未命中上，L1 要付出单靠 L0 不必付出的一次
往返；而一个只会有一个节点去问的条目，不值得放到每个节点都看得见的地方。

## 一次被服务的命中携带的元数据

有五个响应字段描述一份被服务的表示，它们就定义在这里；其他章节使用它们时不会
再重述一遍。

| 字段 | 它说明什么 |
|---|---|
| `ETag` | 一个正好覆盖所发送字节的强验证器。客户端可以把它作为 `If-None-Match` 送回来。 |
| `Cache-Control` | 默认对每一个类别都是 `private`。一个设置了 `SharedCachePolicy::SMaxAge` 的 `PublicShared` 路由还会拿到 `public` 和 `s-maxage`，这是共享代理唯一一次被邀请保留这些字节的方式。一份至少含有一个岛屿的、已组装的 `Composite` 文档则是 `private, no-store`。 |
| `Vary` | 由那些隐含某个请求头的已声明差异化维度推导而来：`Locale` 隐含 `Accept-Language`，`Media` 隐含 `Accept`，`Encoding` 隐含 `Accept-Encoding`。一个不隐含任何请求头的维度什么也不会添加。这些名字是按头名称排序发出的，而不是按你声明维度的顺序。 |
| `Age` | 自该表示发布以来经过的整秒数。它的出现是“一个响应出自存储”最简单的本地证据。 |
| `Warning` | `110 - "Response is Stale"`，并且只出现在越过其新鲜区间之后才被服务的响应上。 |

从维度到请求头的映射是 `crates/suprnova-live/src/render_cache/variance.rs` 里的
`VarianceDimension::vary_header`。两个引擎测试证明了其中 `Locale` 和 `Encoding`
那两半，以及拼接后的头取值：`a_descriptor_orders_dimensions_and_bounds_values`
（`crates/suprnova-live/tests/render_cache_variance.rs`）断言一个同时携带两者的
描述符会报告 `["Accept-Encoding", "Accept-Language"]`；而
`cache_control_and_vary_agree_with_class_variance_and_seed_deadline`
（`crates/suprnova-live/tests/render_cache_coherence.rs`）断言同样这一对会发出
`Accept-Encoding, Accept-Language`，并且一个不含任何隐含请求头维度的描述符根本
不会发出 `Vary`。`Media` 隐含 `Accept` 是从代码里记录下来的；这里没有测试把这
一对配上。

其中三个响应取值是针对运行中的应用断言的：
`the_public_document_is_a_hit_whose_seed_still_promotes` 从 `/live/public` 上
读到 `private, max-age=300`，并要求第二个请求带有一个 `Age` 头；
`the_private_document_is_cached_per_principal_and_never_crosses` 从 `/live/me`
上读到 `private, max-age=60`；
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` 从一份组装出的
仪表盘上读到 `private, no-store`，而这个取值除了复合响应器之外没有别的东西会写。

## 四种新鲜度状态

在任何东西被服务之前，每一次命中都会精确地落到四种状态中的一种。
`FreshnessPolicy::new(fresh_ms, stale_servable_ms, stale_on_error_ms)` 设定
它们。**两个陈旧窗口都是从新鲜区间的末端起算的，而不是一个接一个地叠加** - 这
就是最容易把人绊倒的细节：

| 状态 | 自发布以来的年龄 | 访客拿到什么 |
|---|---|---|
| 新鲜 | 小于 `fresh_ms` | 已存储的字节，没有 `Warning` |
| 陈旧可服务 | 超出 `fresh_ms` 但不足 `stale_servable_ms` | 立刻拿到已存储的字节，带 `Warning`，并在请求背后派生一次有界的重建 |
| 出错时陈旧可服务 | 超出 `fresh_ms` 至少 `stale_servable_ms`，且不足 `stale_on_error_ms` | 一次前台重建；只有当那次重建自己失败时，才拿到带 `Warning` 的已存储字节 |
| 已死 | 超出 `fresh_ms` 达到或超过两个窗口中较大的那个 | 什么都没有；该请求会渲染 |

`/live/todos` 声明了 `FreshnessPolicy::new(300_000, 60_000, 300_000)`，所以它
在五分钟内是新鲜的，第六分钟里是陈旧可服务的，直到十分钟为止是出错时陈旧可服务
的，再之后就是已死的。

有两条规则会盖过这些区间。一份 `PrivateCached` 表示**绝不会**以陈旧状态被服务：
越过它的新鲜区间之后它就是已死的，这正是 `/live/me` 声明
`FreshnessPolicy::new(60_000, 0, 0)` 的原因 - 在那里放一个陈旧区间，会读起来像
是一个缓存并不遵守的承诺。而一份已存储的公共种子文档，只要它的晋升截止时间已过
就是已死的，无论它的区间怎么说，因为一个过了截止时间的种子再也无法被晋升。

`stale_service_is_marked_and_rebuilt_in_the_background` 在一个受控时钟上把
`/live/todos` 推过第一条边界，并断言被服务的正文、
`Warning: 110 - "Response is Stale"` 以及 `Age: 300`。是什么*导致*一份表示提前
离开新鲜区间 - 一次写入、一次纪元推进 - 是
[RenderCache 世代](render-cache-generations.md)的主题。

## 条件请求与 HEAD

一个把收到的 `ETag` 作为 `If-None-Match` 送回来的客户端会拿到一个没有正文的
`304`，而一次 `HEAD` 会拿到那些响应头、没有正文。两者都不会到达你的处理程序：

```
GET  /live/todos                          -> 200, ETag: "..."
GET  /live/todos  If-None-Match: "..."    -> 304, empty body
HEAD /live/todos                          -> 200, same ETag, empty body
```

`conditional_and_head_requests_are_answered_from_the_stored_entry` 针对运行中
的应用断言了这三者，其中包括在后两者上渲染计数不会移动。

有一个例外，而且是刻意的：一个 `Composite` 响应从不应答 `304`。每一次组装都是
一份不同的表示 - 新的岛屿标识，以及在文档带有引导 nonce 时一个新的引导 nonce -
所以一个 `304` 会让客户端把它已经持有的正文，与为这次请求现铸的响应头配到一起。
一个已组装响应上的 `ETag` 仍然是正好覆盖所发送字节的强验证器；它只是永远不会和
后来的请求匹配上。
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` 的第 7 步把一个
收到的验证器原样送回，并断言得到一个 `200` 和一个不同的 `ETag`。

## 一份属于某一个人的表示

`RepresentationClass::PrivateCached` 为每一个已登录访客各存一份表示。除非策略
同时声明了 `Principal` 或 `Tenant` 差异化维度，否则它在构建期就会被拒绝，这样
这一对搭配就不会意外地各走各的：

```rust
use suprnova::render_cache::{
    FreshnessPolicy, RenderCachePolicy, RepresentationClass, VarianceDimension,
};

let router = router.try_render_cache(
    "/live/me",
    RenderCachePolicy::builder(RepresentationClass::PrivateCached)
        .freshness(FreshnessPolicy::new(60_000, 0, 0)?)
        .vary(VarianceDimension::Principal)
        .build()?,
)?;
```

它背后的处理程序是一个普通的处理程序。它解析出已登录访客并渲染出对方的名字：

```rust
pub async fn me(_request: Request) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let user = Auth::user_as::<User>()
            .await?
            .ok_or_else(|| FrameworkError::internal("Live account document without a principal"))?;
        html(&MeView { name: user.name })
    }
    .await;
    result.map_err(failed)
}
```

没有为了让它能缓存而额外接线的东西。这个路由带着与仪表盘相同的
`AuthMiddleware::redirect_to("/login")`，所以一个匿名访客会在处理程序运行之前
就被重定向，而主体本身是在渲染内部解析出来的。从会话里读取已登录访客的身份被
归类为一次**身份读取**，而不是一次会话读取，所以这次渲染收窄为 `PrivateCached`，
键携带按主体划分的不透明材料，两者一致。

`the_private_document_is_cached_per_principal_and_never_crosses` 让两个访客各自
登录，每人命中两次且没有渲染，并断言每份正文写的都是自己那个人、而不是另一个人；
第三个访客会触发渲染，因为他和前两位都不共享任何东西。被服务的 `Cache-Control`
是 `private, max-age=60`，所以任何共享代理都拿不到这些字节。同一个测试还展示了
这笔交易的另一半：这次渲染是通过读取 `users` 表的那个提供者解析出主体的，所以
填充第三个访客会使每一个已存储的 `/live/me` 条目失效，而每个条目的下一次请求都
会重建。这正是[世代](render-cache-generations.md)所描述的、按表粒度的失效在
如实地做它该做的事。

在你声明这个类别之前，那种归类方式有两个后果值得知道：

- 一个发往带 `Principal` 差异化维度的 `PrivateCached` 路由的**匿名**请求，会
  缓存在 `Anonymous` 键下。这次渲染没有解析出任何身份，所以没有观察到任何主体
  材料，键上写的是 `Anonymous`，两者一致。一个已登录访客派生出的是一个
  `Private` 键，永远触及不到那个条目。这一点适用于这样的请求确实渲染出一个
  `200` 的场景，而 `/live/me` 从不会这样：它的登录重定向应答的是一个 `302`，
  而一个 `302` 在这一切被查阅之前就已经被资格审查拒绝了。
  `framework/tests/render_cache/middleware.rs` 里的
  `an_anonymous_render_resolving_identity_through_the_session_caches_anonymously`
  才是真正会走到这一步的框架测试。
- 一个**具名 guard** 的标识符，与默认 guard 的标识符在完全相同的意义上属于主体
  材料。读取它会记录一次主体读取，并且在存在 id 时记录那个值。

还有一条没有变过的规则：一个读取了主体、却*没有*声明 `Principal` 差异化维度的
路由会被拒绝存储。没有办法按访客给这样一个条目建键，所以它永远不会被存储，而
不是被共享。会话中其他每一个值仍然会强制变为 `Uncacheable`；参见
[RenderCache](render-cache.md) 中的分类清单。

## 一个开了洞的外壳

`RepresentationClass::PublicShellStitched` 是给这样一份 Live 文档用的：它的框架
对所有人都一样，而它的岛屿不是。已存储的条目只装那个外壳。任何绑定身份的岛屿的
标记，以及任何已签名的快照，都绝不会出现在已存储的字节里；每一次命中都会为发起
询问的那个人重新挂载每一个岛屿，所用的权限是为那次请求派生出来的。

本仓库的仪表盘就是那样一个路由：

```rust
router.try_render_cache(
    "/live",
    RenderCachePolicy::builder(RepresentationClass::PublicShellStitched)
        .freshness(FreshnessPolicy::new(300_000, 60_000, 300_000)?)
        .build()?,
)
```

`the_dashboard_is_stitched_per_principal_from_one_shared_shell` 断言了这样做买
到了什么、又付出了什么。已存储的条目是一个带三个槽位的 `EntryKind::Composite`，
每个绑定身份的岛屿各占一个。第二个主体是从那个外壳被应答的，而两份文档**只**在
它们的岛屿标签上不同 - 这个测试从每份文档里剥掉那三个岛屿标签，再逐字节比较剩
下的部分。这个路由自己的登录重定向在每一次命中上仍然会运行：一次缝合命中在任何
东西被服务之前，都会被转发穿过这个路由的整条中间件链，所以一个匿名访客拿到的是
重定向，而绝不是一份组装好的文档。

一份至少带有一个槽位的、已组装的文档会被发以
`Cache-Control: private, no-store`。它在为某一次请求重新派生出的权限之下装着
某一个主体的岛屿，而一个 `max-age` 会让一个共享的浏览器配置把它们重放给下一个
坐下来的人。一个零槽位的 `Composite` 根本不携带任何按主体划分的字节，只带一个
按请求生成的 nonce，所以它像别的私有表示一样，保留这个类别的私有 `max-age`；
`framework/tests/render_cache/stitch.rs` 里的
`a_zero_slot_composite_is_assembled_with_a_fresh_nonce_on_every_hit` 断言了
这一点。无论哪种情况，这个类别都会在策略构建期拒绝
`SharedCachePolicy::SMaxAge`，所以任何共享代理都拿不到这些字节。

有两个限制要知道：这个类别只在中间件链以 Live 完成中间件收尾的路由上才有意义，
所以只把它和 `LiveDocument::render` 一起用，别的都不行；另外，一个缝合条目绝不
会被出错时陈旧回退所服务，也绝不会触发一次后台重建。
[世代](render-cache-generations.md)那一章会说明第二点在实践中意味着什么。

### 为什么 Suprnova 有所不同

Laravel 根本没有服务端的表示模型。它的响应缓存包把一个路由的渲染输出，存在一个
你自己拼出来的键之下 - 典型的是 URL，有时是 URL 再加上一个为已登录用户手写的
后缀 - 并在下一次请求时把它交回来。被存下来的东西只有一种形态，它永远是一份
完成了的正文，而两个访客会不会共享它，是你拼出来的那个字符串的属性。

Suprnova 让键成为一份声明，让形态成为一个后果。你指名类别和差异化维度；框架来
派生键，拒绝一个没有划分维度的 `PrivateCached`，把渲染实际观察到的东西与键实际
说的东西作对照，并在两者不一致时拒绝存储这次渲染。而且因为有
`PublicShellStitched`，一个 95% 共享、5% 私有的页面不必在“什么都不缓存”和
“缓存一些不该缓存的东西”之间二选一：共享的那部分被存储一次，私有的那部分按请求
重新渲染，私有的字节从不进入存储。

## 下一步

- [RenderCache 世代](render-cache-generations.md) - 一份已存储的表示如何不再是
  最新的，以及接下来会发生什么
- [RenderCache](render-cache.md) - 声明策略、差异化维度，以及一次渲染永远不会
  被存储的那些原因
- [Live](live.md) - 一个缝合外壳为之开洞的那些岛屿
