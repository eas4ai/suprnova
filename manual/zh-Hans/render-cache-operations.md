# RenderCache 运维

一个你看不见的缓存，是一个你没法信任的缓存。RenderCache 直接回答运维人员的两个
问题，而且从不打印一个已存储的页面：**这个节点在这个键下持有什么，它还是最新的
吗？**以及**我怎么让一切停下来？**它还回答第三个问题 - “这个路由到底有没有在从
一份已存储的副本被服务？” - 但那是通过遥测和 `Age` 头，而不是通过一条命令，因为
那个问题问的是流量，而不是某一个条目。这里有两条控制台命令、七个遥测计数器、
一次有界的磁盘清扫，以及一根紧急拉杆。

本章讲的是这套运维界面：那些命令、它们究竟打印什么又能看见什么；那些计数器和
它们封闭的结果集合；文件层级如何回收磁盘；如何测试一个被缓存的路由，好让测试
证明的是缓存而不仅仅是有响应；出问题时该怎么办，包括一次数据库恢复所需要的
多节点流程；以及这个缓存自身的性能是怎么测的、那些数字老老实实地值多少。命令
示例就是 `the_operator_commands_inspect_without_a_body_and_advance_the_epoch`
在 `app/tests/live_render_cache.rs` 里，通过本仓库自己的控制台入口驱动的那些。

## 两条控制台命令

两条都是隐藏命令，由框架注册，可以像别的命令一样通过你项目的 `console` 二进制
够到。两条都绝不会打印一个已存储的正文或一个原始的依赖标识。

```bash
cargo run --bin console -- render-cache:inspect rk1.<43 base64url characters>
cargo run --bin console -- render-cache:epoch-advance
```

**`render-cache:inspect <key>`** 报告某一个已存储条目的形状：它的表示类别、它的
`body_bytes`、它其他的元数据，以及旁边的当前权威纪元，让你能判断自己正在看的这
个条目还是不是有效权威，还是早已在背后过期。当这个键指向它看不到的东西时，它会
打印 `no entry (current epoch: {epoch})`；而在键无法解析、或者没有安装运行时的
情况下，它会失败 - 而不是报告成功。

**它只读取本进程的进程内 L0，别的什么都不读。**
`RenderCache::inspect` 只在 L0 里查这个键；它从不去问 L1 层级。在 Database 或
Redis 配置档下这一点很要紧：一个在 `suprnova_render_entries` 里或在 Redis 里
活着的条目，无论是由另一个节点、还是由本节点在一次重启之前发布的，只要本进程
自启动以来没有服务过它，在这里就会打印 `no entry`。请把这份报告读成“这个节点
在内存里有什么”，而绝不要读成“这个部署存下了什么”。
`RenderCache::store_inspection` 也是一样，它报告的是 L0 的占用情况和当前纪元。

这个键就是查找本身所用的那段文本：`rk1.` 加上 43 个 base64url 字符，也就是你
应用的日志和遥测能够呈现出来的东西。它不是对任何东西的第二次哈希，所以运维人员
手里的一个键，恰好指名一个条目。

那条“不含正文”的主张是被检查过的，而不只是被声明。这个测试拿到实际被服务的那份
文档，把它切成行，并要求其中**每一**个非平凡的行，都不出现在 inspect 报告打印
出来的内容里。

**`render-cache:epoch-advance`** 是那次紧急失效。它推进权威纪元，并打印
`epoch advanced to {epoch}`。因为纪元被烘焙进了每一个查找键，这会让已存储的条目
变得触及不到，既没有什么要枚举的，也没有什么要删除的。这个测试断言了打印出来的
那一行，然后断言了真正要紧的那个后果：命令之后，这个路由会重新渲染。

**在运行它的那个节点上**，效果是立即的：这条命令会丢弃该进程的纪元租约并清空
它的进程内层级，所以它紧接着的下一个请求会在新纪元下派生键，然后什么也找不到。
（最后这一句成立的前提是纪元只往前走，那是通常的情形；在一次数据库恢复之后，
推进出来的那个值可能是这个部署用过的，所以请看下面的“恢复数据库”。）**在任何
别的节点上**，账本已经移动了，但那个进程仍然持有它旧的、租来的纪元和它自己的
L0，它会在下一次权威读取时跟上 - 在 `CoherenceMode::Authority` 下是立即的，
在 `CoherenceMode::Lease` 下最迟是 `max_age_ms` 之后。请在每个节点上都运行这条
命令，或者把其余节点重启。下面的“恢复数据库”给出了完整流程和它背后的测试。

当缓存内容出了问题、而你等不及各个条目自行过期时，就用它；在一个改变了已缓存
页面显示内容的作业之后也用它（参见
[RenderCache 世代](render-cache-generations.md)中的“已知的缺口”）。

## 权限变更

`RenderCache::bump_permission_version().await?` 是应用唯一一次要手工发起的失效
调用，而它其实算不上一条运维命令 - 它属于那条改变“一个已登录用户被允许做什么”
的代码路径。它会推进一个持久化的世代，每一个按主体建键的渲染都会关注这个世代；
它能挺过一次重启；并且当角色变更运行在某个事务里时，它会加入那个事务。没有它，
一个权限刚刚变化的用户，仍然会命中在其先前权限集合下缓存的内容。

## 遥测

七个封闭的计数器名称，而且它们当中没有任何一个会点名某个层级、某个提供者或某个
后端：

| 计数器 | 属性 |
|---|---|
| `suprnova.render_cache.lookups` | `outcome` |
| `suprnova.render_cache.hits` | `outcome` |
| `suprnova.render_cache.publications` | 无 |
| `suprnova.render_cache.rebuilds` | 无 |
| `suprnova.render_cache.stitch.assemblies` | `outcome` |
| `suprnova.render_cache.stitch.slots` | `outcome` |
| `suprnova.render_cache.epoch_rewinds` | 无 |

`lookups` 和 `hits` 携带同一个封闭的八种结果集合：

- `l0`、`l1` - 一个从进程内层级或共享层级被服务的新鲜条目。
- `conditional` - 一次 `If-None-Match` 匹配上的新鲜命中，应答了 `304`。
- `stale` - 一个被立即服务的陈旧可服务条目，或者一次前台重建失败之后的出错时
  陈旧回退。
- `miss` - 什么也没找到、一次出错时陈旧重建正在进行中，或者一个已死的条目。
- `bypass` - 一个未声明的查询参数、一个无法解析的已声明差异化维度，或者一份已
  耗尽的等待者名单。
- `moved` - 渲染之后的那次重读发现某项依赖或纪元变过了；候选被丢弃，从未发布。
- `declined` - 这次渲染不可存储：资格审查、一份溢出的观察报告、一个
  `Uncacheable` 分类、一条 Live 文档规则，或者某个上限。

`hits` 只在 `l0`、`l1`、`conditional` 和 `stale` 上递增。`publications` 只统计
一个存储回答了“已发布”的情况，绝不统计被栅栏挡下或被拒绝的尝试。`rebuilds` 每
派生一次后台重建统计一次。

那两个缝合计数器携带它们自己的集合：组装是 `assembled` 和 `fail_document`；
槽位是 `rendered`、`omitted`、`fallback` 和 `failed`。

`epoch_rewinds` 数的是检测次数，不是条目数：每当一个节点遇到一个盖着高于
权威自身纪元戳记的条目或租来的纪元，它就加一次，然后把账本的纪元推进到超过
那个戳记，并清空自己的 L0。数据库恢复之后出现一个非零值，是恢复被察觉到的
信号。在其他任何时候出现一个非零值，则意味着权威因为某个谁都没打算的理由
往回退了。

**一个偏高的 `declined` 比率，才是值得告警的信号。** 它意味着你接入的那些路由
在正确地渲染和服务，却从来没有被存储过，而响应两种情况下看起来一模一样。最快
的本地检查是连着发两个请求：如果第二个不带 `Age` 头，那就什么都没被存下来。

## 磁盘卫生

只有文件层级需要清扫，而且它多半会自己扫自己。

`FileRenderStore` 每个键存一个文件，平铺在 `RENDER_CACHE_L1_DIR` 之下。当一个
条目自发布以来的年龄达到它发布时所带的保留期，或者它的栅栏纪元比当前纪元更旧
时，它就是死的。保留期来自与在线新鲜度检查相同的那条按类别计算的死亡边界，所以
一个私有条目的文件会比一个公共条目的更早退休，而一次清扫也绝不会在“一个条目是
不是真的死了”这件事上与一次新鲜度检查产生分歧。

`sweep` 每次调用最多移除 64 个条目，按发布时间从旧到新，并返回是否还有剩余。它
在每第 256 次发布时自动运行，所以一个健康的目录不需要人操心。
`RenderCache::sweep()` 让你在想要的时候显式驱动它，而一份大于单次调用限额的积压
会分散到后续的触发里排空，而不是卡在一次漫长的扫描上。

有两件事清扫并不是：

- **一次纪元推进不会触碰 L1。** 它会把 L0 彻底清空，因为那是进程内内存，没有
  什么要去对账；而它会把每一个纪元之前的文件留在磁盘上，直到一次清扫把它回收。
  那属于磁盘卫生，不是正确性问题 - 那些文件本来就已经无法通过查找触及了。
- **数据库层级没有自动清扫**，只能通过 `RenderCache::sweep()` 回收。Redis 层级
  一个都不需要：它存的每一个条目都带着一个过期时间，Redis 会自己回收那些字节。

发布是崩溃安全的。它先写一个临时文件、fsync 它、把它 rename 覆盖到目标上，再
fsync 父目录，所以一个读者要么只会看到之前那个完整的文件，要么只会看到新的那个
完整文件。打开时，这个存储会移除任何遗留的临时文件，以及任何通不过帧检查的
文件，把一次撕裂的写入当作可自愈的，而不是一个被永久毒化的条目。

## 测试一个被缓存的路由

一个断言“被缓存的路由响应正确”的测试，无论这个响应出自存储还是出自一次新鲜渲染
都会通过。每一个主张都必须针对某种“只有一个已存储条目真的被服务了才会产生”的
东西来提出。有四种模式能做到这一点，而本仓库自己的自用（dogfood）测试四种都用
上了：`app/tests/live_render_cache.rs` 加上 `app/tests/live_support/mod.rs` 里的
测试装置。

**1. 在缓存靠处理程序那一侧统计渲染次数。** 在 `RenderCache::install` 之*后*
注册一个计数中间件。注册是追加式的，所以它落得比 `RenderCacheMiddleware` 更靠近
处理程序，而一个被缓存应答掉的请求会在调用它之前就返回：

```rust
let router = app::live::routes_with_render_cache_with_config(routes::register(), config)
    .await
    .expect("install the routes and the RenderCache middleware");
// After the install, so it only sees requests the cache forwarded.
render_counter::register();
```

于是两次读取 `render_counter::renders()` 之间的差值，就是缓存没有避免掉的渲染
次数，别的什么都不是 - 这不像完全相同的正文或者一个 `Age` 头，那两者都有诚实的
非缓存解释。`an_orm_write_invalidates_the_todos_document_through_generations` 里
每一条命中断言都倚在它上面。要等待一次不是你派发的渲染（一次后台重建），请用
计数器自己的屏障 `wait_until_renders_at_least`，绝不要用 sleep。

**例外是一个 `PublicShellStitched` 路由，而且这不是个小例外。** 一次缝合命中是
被刻意转发穿过这个路由整条链的 - 它的授权守卫必须再跑一遍，而且只有那条链末端
的 Live 完成中间件才会服务这次命中。一个在安装之后全局注册的计数中间件坐在这个
路由自己的链之外，所以在一次缝合命中上它被到达的方式，和在一次未命中上完全一样。
在这样一个路由上，计数器根本承载不了“没有处理程序运行过”这个主张。

请改为断言存储里装着什么、被服务的那份文档是由什么构成的，以及它有多旧，也就是
`the_dashboard_is_stitched_per_principal_from_one_shared_shell` 所做的：已存储的
条目是一个带预期槽位数的 `EntryKind::Composite`（`inspect_route_for_test`），
两个主体的文档只在它们的岛屿标签上不同，别处都一样，而发给第二个主体的响应
所报告的 `Age`，正是外壳被发布以来过去的整秒数。

最后这一条才是这个测试真正倚仗的证据，也是“这份响应出自存储”这一本地证明的
精确形态。`Age` 头本身是本章上面警告过的那种弱信号，因为一次渲染也会放一个：
放的是零。数字就不弱了：一次渲染在同一瞬间发布它的响应和它的条目，所以无论时钟
走了多远，一份渲染出来的响应报告的都是零；而一次组装报告的，是它据以组装的那个
外壳的年龄。这个测试为此驱动一个可调时钟，并且停在这个路由新鲜窗口的很里面，
好让它读到的东西是精确的，而不是碰巧的。

响应也携带 `Cache-Control: private, no-store`，但要按它本来的样子来读：那是这个
类别里一个带槽位的路由所携带的指令，在发布外壳的那次渲染上和之后每一次组装上
都同样被钉住，因为它跟着字节里装的是什么走，而不是跟着哪条路径产生了它们走。
“带槽位”在这里是关键词：一个零槽位的 `Composite`
保留的反而是这个类别的私有 `max-age`，所以这条指令说的是一个里面有岛屿的路由，
而不是一个没有岛屿的。那个测试在一次命中上断言 `renders() == before + 1`，
并在它自己的注释里说明了为什么那才是诚实的读法，而不是一次失败。

**2. 把条目读回来。** 有两个门面调用是普通的公开 API：
`RenderCache::store_inspection()` 报告 L0 的占用、字节数和当前纪元，而
`RenderCache::inspect(key_text)` 报告某一个条目的不含正文的元数据。在它们旁边，
框架还暴露了隐藏的测试接缝 - 标了 `#[doc(hidden)]`，并以 `_for_test` 命名，好让
没有人把它们误当成应用 API：

| 接缝 | 它给一个测试什么 |
|---|---|
| `RenderCache::key_for_route_for_test(pattern, params, login)` | 中间件**在纪元 1 上**派生出的那段键文本，也就是迁移填充的那个值 |
| `RenderCache::key_for_route_at_epoch_for_test(pattern, params, login, epoch)` | 同上，但在你指名的一个纪元之下 |
| `RenderCache::inspect_route_for_test(pattern)` | 那个纪元 1 的键在 L0 里的条目：类别、种类、状态、`body_bytes`、槽位 |
| `RenderCache::inspect_l1_for_test(pattern, params, login)` | 同上，但出自所配置的 L1 层级 |
| `RenderCache::clear_l0_for_test()` | 清空 L0，而不动 L1、纪元和协调器 |

纪元之所以要紧，是因为它是键的一部分。`key_for_route_for_test` 把纪元 1 写死了，
所以一个已经推进过纪元的测试 - 无论是在本节点上，还是通过账本在另一个节点上 -
必须用 `key_for_route_at_epoch_for_test` 指名那个新值，否则它查的会是一个根本
没有东西发布在其下的键。

`the_public_document_is_a_hit_whose_seed_still_promotes` 用 `store_inspection`
和 `inspect_route_for_test` 断言条目存在、并且存储在所声明的类别之下；
`the_database_profile_serves_a_hit_through_the_sql_stores` 用
`inspect_l1_for_test`，然后用 `clear_l0_for_test`，而那是唯一一种能证明后一个
请求出自 L1 而不是出自内存的办法。

**3. 拨时钟，而不是等。** 运行时读取的那个时钟可以在一个 `RenderCacheConfig`
上设置，而且永远不会由 `from_env` 设置，所以一个需要某个新鲜度区间的测试要装
自己的：

```rust
let clock = Arc::new(AdjustableTestClock::new(unix_now_ms()));
// Bound to its own name first: passing `Arc::clone(&clock)` inline leaves
// the compiler inferring the trait object as the clone's return type.
let for_runtime = Arc::clone(&clock);
let config = RenderCacheConfig::from_env()?.with_clock_for_test(for_runtime);
// ... install through the application's own configuration seam, then:
clock.advance_ms(300_001);
```

`AdjustableTestClock` 来自 `suprnova::live::testing`，而 `unix_now_ms` 是测试
装置自己读的墙上时钟，所以一个可调时钟起步的位置是系统时钟所在的位置，而不是
进程里其他部分都不会认同的某个时间原点。（那是一个时钟零点，而不是本章在别处
用“纪元”这个词所指的那个权威纪元。）测试装置把这一对封装成
`setup_app_with_clock` 和 `advance_clock_ms`，后者在启动取用了系统时钟时会
panic，而不是静默地什么都不做。
`stale_service_is_marked_and_rebuilt_in_the_background` 就是那个测试。

**4. 统计 SQL 语句数。** 一个跳过了处理程序、却在每一次命中上仍然去问数据库的
缓存，能满足每一个处理程序侧的计数器，却依然要花一次往返。
`DbConnection::observe_statements_for_test` 把 SeaORM 的指标回调指向一个你自己
的计数器，而且它看得到连接池上、以及从连接池开出的每一个事务上的语句：

```rust
// Immediately after connecting, before the connection is cloned or bound
// into the container: installing needs sole ownership of the pool, and the
// call reports `false` rather than counting nothing silently.
let installed = conn.observe_statements_for_test(|| {
    STATEMENTS.fetch_add(1, Ordering::SeqCst);
});
assert!(installed, "the statement observer needs an unshared connection");
```

这个回调对语句一无所知 - 没有 SQL 文本，没有绑定值 - 因为计数就是它的全部意义。
`framework/tests/render_cache/bypass.rs` 完全是按这个模式写的：
`a_lease_mode_hit_runs_nothing_and_issues_no_statement` 把一次租约模式命中限制
在零条语句，`an_authority_mode_hit_issues_exactly_one_statement` 把一次权威模式
命中限制在一条，而 `the_epoch_is_read_once_at_first_use` 用两次未命中互相对比，
说明纪元的代价是每个运行时读一次。

有两个习惯值得保持。请通过你自己应用的配置接缝来启动测试装置，而不是通过一个
手搭的路由器，这样测试装的就是与服务器相同的路由、策略和中间件顺序。还有，永远
不要加定时等待：上面每一道屏障都是计数器上的状态屏障，而这正是让这些测试可复现
而不是不稳定的原因。

## 出问题的时候

- **某个页面正在显示你知道已经过时的内容。** 先查这个路由到底有没有在存储（发
  两个请求，看有没有 `Age`）。如果它在存，而本该让它失效的那次写入来自一个队列
  工作进程、一个计划任务或一个控制台命令，那次写入什么都没推进：运行
  `render-cache:epoch-advance`（要按节点来 - 见最后一条）。
- **一个你本以为会缓存的页面从来不带 `Age` 头。** 它是被拒绝了，不是失败了。
  请对着 [RenderCache](render-cache.md) 里的分类清单逐条排查：一次会话读取、
  一次在没有 `Principal` 差异化维度的路由上的身份读取、一次没有 `Locale`
  差异化维度的语言环境读取、一次授权检查，或者一次原始 SQL 读取。
- **某个后端不可达。** 由 `RENDER_CACHE_FAILURE` 决定：`open`（默认）以不缓存
  的方式服务该路由，`closed` 应答一个光秃秃的 `503`。而一个在启动时就缺失的
  后端会直接让启动停下，并给出一句点名要修的迁移或变量的话。
- **Redis 被刷掉或重启了。** 条目会未命中并被重新渲染。没有任何陈旧的东西能被
  证明为最新：时效性是对着数据库世代账本证明的，而绝不是对着持有那些字节的
  层级。
- **一个重建领导者在重建途中挂了。** 一旦存储时间越过了到期时刻，它的租约就会
  被接管，而原来那个领导者自己的发布会被栅栏挡在外面，而不是和新的那次抢。它
  什么都不会发布；它那个请求的响应仍然会被服务。
- **一个 L1 文件被一次崩溃或一块写满的磁盘撕裂了。** 没有东西会服务它。每个
  文件都带着一份覆盖自身帧的摘要，所以一个被截断或被改动过的文件通不过那次
  检查，就是一次未命中；这个存储下一次打开时会把它、以及任何遗留的临时文件
  一起移除。一次撕裂的写入在这里是可自愈的，而不是一个被永久毒化的条目。
- **数据库是从一份备份里恢复出来的。** 这一条有一套流程，而不是一句话；见下面
  的“恢复数据库”。
- **你需要现在就让一切消失。** `render-cache:epoch-advance`。在不止一个节点上
  时，请在每个节点上都运行它，或者把你没在上面运行过的那些重启掉：这次推进会
  为整个部署移动账本的纪元，但它只在运行它的那个进程里清空 L0 并丢弃租来的
  纪元。下面的恢复流程会说清楚为什么。

## 恢复数据库

世代账本是每一次命中都要对照证明的那个权威，所以恢复数据库会改变“最新”对每一个
已经存下来的条目意味着什么。有两件事决定一个已存储的条目接下来会做什么，
而它们两个都不是“它被悄悄丢掉了”。

**这会自动替你处理好。** 恢复之后的第一次权威读取，只要遇到一个盖着高于
恢复后数值戳记的纪元或条目，就会彻底拒绝那个条目 - 哪怕只有一次，也不会带着
`Warning` 被服务，不论年龄多大，也不管路由的新鲜度策略怎么说 - 然后重建它，
把账本的纪元推进到比它见过的最高戳记还高一位，用推进后的值替换掉那个节点的
纪元租约，并清空那个节点的 L0。每一个其他节点都会在自己下一次权威读取时看到
被推进的纪元：在 `CoherenceMode::Authority` 下是立即看到，在
`CoherenceMode::Lease` 下则是在 `max_age_ms` 之内看到。
`suprnova.render_cache.epoch_rewinds` 会为每一次检测计一次数。

就这些了，而这正是运维人员的 `render-cache:epoch-advance` 所产生的那种收敛，
只是不需要运维人员出手就达成了。`framework/tests/render_cache/middleware.rs`
里那三个 `an_epoch_advanced_by_another_node_*` 测试量出了传播的上限，而
`a_rewound_epoch_refuses_the_entry_rebuilds_and_lifts` 量出了这次拒绝。

**还剩一步可选的操作。** 如果一个带陈旧可服务窗口的路由，在重建之前一次都不能
服务恢复之前的表示，那就清空共享的 L1 层级。正是这次推进让这一点变得可以做到：
一个盖着*低于*被推进纪元戳记的 L1 条目，会重新变成一个普通的已移动条目，而这样
一条已移动条目在这样的路由上，会在重建于请求背后运行的同时，被服务一次并带着
`Warning`。删掉文件层级目录里的内容、执行 `DELETE FROM suprnova_render_entries`，
或者删掉匹配 `<prefix>entry:*` 的 Redis 键 - 看这个配置档配的是哪一个层级。跳过
这一步，最坏的情况也不过是每个这样的键会有一份带 `Warning` 标记的恢复之前的
正文被服务。

## 度量它

RenderCache 带了两个基准测试，而它们是**按需工具，绝不是门禁步骤**：

```bash
crates/suprnova-live/scripts/run-render-cache-budget.sh
```

它会先跑引擎基准（`render_cache_budget`，用一个计数分配器做的热命中与复合组装
测量），然后跑框架工作负载基准（`render_cache_workloads`，同一个路由穿过整条
中间件），最后跑针对签入结果的契约测试。两者都被钉在
`SUPRNOVA_LIVE_S1_CPUSET` 上。

一次完整运行需要一个用完即弃的 PostgreSQL（`PG_TEST_URL`）和一个用完即弃的
Redis（`REDIS_TEST_URL`），因为签入结果的契约要求三个被记录的配置档全部到齐。
**一次部分运行必须把两个结果文件都重定向**到 `benchmarks/local/` 之下，用
`SUPRNOVA_LIVE_BENCH_RESULT` 和 `SUPRNOVA_LIVE_WORKLOADS_RESULT`；不这么做，它
就会用一份更短的文件覆盖掉签入的结果，然后让自己的契约失败。

签入的那些数字，来自
`crates/suprnova-live/benchmarks/render-cache-budget-v1.json` 和
`render-cache-workloads-v1.json`：

| 测量项 | 取值 |
|---|---|
| 一次新鲜 `Complete` L0 命中的引擎工作，p95 | 0.76 微秒 |
| 堆分配次数，新鲜命中 | 3 |
| 堆分配次数，条件式 `304` 命中 | 3 |
| 堆分配次数，受种子截止时间约束的命中 | 4 |
| 以上任意一种上的正文复制 | 没有；缓冲区是共享的 |
| 同一个路由穿过中间件、服务器一侧，p95 | 14.4 微秒 |
| 同一个请求经过一次回环 HTTP 往返，p95 | 109 微秒 |
| 每次热命中的 SQL 语句数（租约模式） | 0 |

中间件那几个数字是针对一份 65,536 字节的正文的，它那次渲染读了 12 行，记录为
14 个被观察到的依赖标识。

**请把它们读成探索性的，而不是合格证据。** 每一份签入的结果都带着
`"classification": "local_exploratory"` 和 `"s1_requirements_met": false`：它们
是在一台开发工作站上产生的，CPU 是共享的、调速器是 `powersave`、提供者走的是
回环。它们对于抓住整整一个数量级的回退有用，除此之外再细就没用了。一个数字只有
在专用运行机上、并带着它的认证声明产生时才算合格证据，而这些不是。

### 为什么 Suprnova 有所不同

Laravel 的响应缓存包把运维留给底下那个缓存存储。检视一个条目意味着手工找出它的
键、再读出那个值 - 而那个值就是渲染好的页面，所以看它就意味着把某个人的 HTML
打印到终端上；而让一切失效意味着刷掉一个同时还装着你的会话、你的限流和你的队列
的存储。可观测性则是那个存储驱动碰巧会发出的任何东西。

Suprnova 给这个缓存自己的一套运维界面，而且刻意做得很窄。检视在构造上就是不含
正文的，所以一个运维人员可以确认一个条目存在、它存在哪个类别之下、它有多大，
而完全不会被展示它的内容。失效是一次纪元推进，施加起来不花什么代价，而且只触及
这个缓存 - 你的会话和你的队列不在爆炸半径里。遥测是一个由七个计数器构成的封闭
集合，属性集合也是封闭的，而这正是让一块建在它们之上的看板能跨版本保持稳定、
而不是变成一堆会漂移的字符串的原因。这笔交易的代价是没有一条“删掉这一个键”的
命令：这些拉杆要么是按条目只读的，要么是整个纪元范围的。

## 下一步

- [RenderCache](render-cache.md) - 这些命令所作用的那些声明
- [可观测性](observability.md) - 上面那些计数器是在哪里被导出的
- [测试](testing.md) - 上面那些模式所处的周边测试约定
- [部署](deployment.md) - 围绕它们的生产环境检查清单
