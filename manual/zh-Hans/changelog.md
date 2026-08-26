# 更新日志

一份可读的、逐版本记录 Suprnova 变更内容的日志。每个版本小节都是该版本的发布记录。当一个版本的版本提交与匹配的 `v<version>` 标签被原子性地推送时，这个版本就算发布了。按最新到最旧排列。

## 1.3.6 - 2026-08-26

### 新增

- **框架错误现在可以渲染您自己的 Inertia 页面，而不是客户端那个崩溃模态框。** 一个没有某项权限的用户，点了一条通往受保护路由的导航链接，结果拿到的是 Inertia 那块 “All Inertia requests must receive a valid Inertia response, however a plain JSON response was received” 界面：那个 `403` 携带的是框架的 JSON 错误响应体，并且没有 `X-Inertia` 响应头，所以客户端拒绝了它。一条未路由路径的 `404`、一个被限流的 `429`，以及一个失败的处理程序给出的 `500`，情况都一样。用 `InertiaConfig::error_page("Error")` 点名一个页面组件，这些响应就会以它们原本的状态码渲染那个页面，并带上 `status`、`message`，以及 - 当那个错误携带了它的时候 - `request_id` 这几个 prop。错误响应设置的每一个响应头都会挺过这次替换，只有两类除外：仅仅描述了正被替换掉的那份响应体的（`Content-*`、`Transfer-Encoding`），以及管着它可以怎样被存储的（`Cache-Control`、`Expires`、`Age`、`ETag`、`Last-Modified`）；所以 `429` 上的 `Retry-After`、`401` 上的 `WWW-Authenticate`、`Vary` 和 `Set-Cookie`，全都仍然能到达客户端。这个页面转而为自己设置 `Cache-Control: no-cache, private`：它携带着您的共享 props，所以不管它替换掉的那个响应许可了什么，它都绝不能被一个共享缓存存下来、再提供给另一个访客。一次 Inertia 访问拿到的是 JSON 页面对象；一次硬性导航拿到的是完整的 HTML 外壳，所以把这个 URL 粘进地址栏也行得通。凡是已经有主的东西都不去动：验证 `422` 仍然重定向回表单，`X-Inertia-Location` 弹回以及本来就已经是 Inertia 页面的响应会原样通过，而一个 `Accept` 更偏好 JSON 的客户端，拿到的响应体和它此前拿到的分毫不差。`suprnova new` 会脚手架出 `frontend/src/pages/Error.*` 并设置 `.error_page("Error")`，所以新项目什么都不用做就已经就绪了。

### 修复

- **一个本地磁盘不会再因为另一个任务碰过某条路径，就拒绝这条合法的路径。** 路径防护此前用两次探测来解析一条路径的每一个组成部分，再把它们合并成一个判定，于是普通的并发活动可能被读成一次符号链接逃逸：一个 `canonicalize` 刚刚报告不存在、随后被另一个任务创建成普通文件的组成部分，回来时成了一个 `PermissionDenied`，点名了一个从来就不存在的符号链接。这在写入方按设计就会互相争抢的地方咬得最狠 - 只要赢家在那两次探测之间发布了这个键，一个在 `write_with(..).if_not_exists(true)` 上竞态落败的调用方拿到的就是这个拒绝，而不是 `ConditionNotMatch`；在一个满负载的测试套件下，这大约占了三分之一的运行。现在每一个组成部分都由单独一趟来分类，`symlink_metadata` 优先：那里什么都没有就是可用的空位，一个普通文件或目录会像此前一样被解析并限制在范围内，而只有一个仍然解析不出来的符号链接才会被拒绝。一个在分类过程中消失的组成部分，会被再看一次，而不是被拒绝。每一处符号链接拒绝都没有变化。

### 升级

- 在一个既有的应用选择启用它之前，什么都不会变。`InertiaConfig::error_page` 默认是 `None`，而 `Inertia::install` 只在点名了一个组件时才注册那个错误页面中间件，所以错误响应保持它们原本的响应体分毫不差。要采用它，请在您其他页面旁边加一个名为 `Error` 的页面组件（它会收到 `status`、`message`，以及一个可选的 `request_id`），并在您传给 `Inertia::install` 的那份 `InertiaConfig` 上链式调用 `.error_page("Error")`。一个会 **panic** 的处理程序仍然在范围之外：panic 安全网包住的是整条中间件链，所以它合成出来的那个 `500`，是在每一个中间件都已经展开之后才构建的。返回 `Err(...)` 而不是 panic，错误页面就管得着它了。请注意，判定的依据是响应体的**形状**，而不是它的作者：在一个错误状态码上，一个空的响应体、一个 `message` 是字符串的 JSON 对象，以及路由器自己那句 `404 Not Found` 文本，都会被改写，不管它们是哪个中间件构建的；而且只有 `message` 和 `request_id` 会活着进入 props。一个必须保住自己那份 JSON 响应体的响应，应该把它的文本放在 `message` 之外的某个键上，或者自己给自己设置 `X-Inertia: true`。另外，请在 `Inertia::install` **之前**注册 `LocaleMiddleware`：错误页面是在返回的路上渲染的，那时候在 Inertia 这一层内部注册的每一个中间件都已经返回，所以在它内部打开的语言区域作用域早就没了，于是每一个错误页面都会以应用的默认语言区域渲染。脚手架出来的 `bootstrap.rs` 现在会这么做，而同样的道理适用于您自己的任何一个请求作用域中间件，只要这个页面的共享 props 会读取它的状态。

## 1.3.5 - 2026-08-26

### 变更

- **每一个更新日志小节在全部六份手册翻译里都读得通。** de、es、fr、ja、pt-BR 和 zh-Hans 这几份手册，过去把 1.3.0 到 1.3.2 这几个小节留在一条译者说明后面、保持英文原样，更早的小节里也散落着没译的英文行；从 1.3.5 一直回到 0.1.0 的每一个小节现在都翻译好了，那些说明也去掉了。

### 修复

- **本地文件系统磁盘会一步发布每一个对象。** `Storage::register_fs` 和 `register_fs_with` 现在会把 `disk.write(...)`、`disk.writer(...)` 和 `disk.copy(...)` 先暂存成 `<root>/.suprnova-atomic/` 底下的一个临时文件，再用一次 `rename(2)` 把它发布到目标上，所以它们当中没有任何一个会在写了一半的长度上被观察到。在此之前，这个驱动程序是用 `create + truncate` 打开目标、再就地往里流式写入的：在整个写入期间，一个并发的读取方拿到的是一个空的、或者写了一半的对象，而一次写到一半的崩溃，会在活着的那条路径上留下一个被截断的对象。写入器上的 `abort()` 现在会把暂存文件丢弃掉，而不是带着 `Unsupported` 失败。
- **`write_with(..).if_not_exists(true)` 在本地磁盘上是一次真正的独占创建。** 它是用 `link(2)` 发布的，当目标已经存在时，这个调用会在内核里原子地失败，所以在任意多个相互竞态的调用方当中，恰好有一个会成功，其余每一个都会拿到 `ConditionNotMatch`，而且什么都没写。一次由朴素 rename 发布出去的暂存写入，会把这个条件劣化成“先检查、再覆盖”，悄悄丢掉除最后一个之外的所有写入方 - 而那恰恰与人们伸手去用这个原语的目的相反。
- **一次创建出对象的 `append` 仍然是一次 append。** append 是本地磁盘上唯一一个原地进行的操作，而这一点现在对第一次 append 同样成立，所以两个向同一个尚不存在的对象追加内容的写入方都会落地，而不是其中一个暂存出自己的一份副本、再把另一个覆盖掉。

- **`suprnova serve` 不会再去重新构建一个没人碰过的项目，`suprnova generate-types --watch` 也不会了。** 这两个监视器都只按路径来给一个文件系统事件分类，而这个生成器每一次运行都会读遍它们正在监视的同一棵 `src/` 目录树底下的每一个 `.rs` 文件 - 所以在 Linux 上，内核会把这些读取报告出来，于是每一次重新生成都排定了下一次。一个刚脚手架出来的项目会每半秒重新生成一次它的类型、重启一次它的后端，永远这样下去，而源码一个字都没改过。现在只有那些意味着磁盘上的字节确实变了的事件才算数。`generate-types --watch` 此前根本没有任何防抖，所以它是对着一阵变更里的第一个文件动手，而不是最后一个；它现在与 `serve` 共用同一个 500 毫秒的后沿窗口，而且这两个监视器共用一份实现，这样下一次修复就不可能只落到其中一个身上。生成器会先比较再写，所以一次输出逐字节相同的重新生成，会原封不动地留下这个文件以及它的 mtime。

- **后端监视器的范围被收窄到构建服务器真正要用到的那些路径上。** `cargo watch` 此前跑起来一个 `-w` 都不带，所以它监视的是整个没有被 gitignore 掉的项目：保存一个 Svelte 组件，或者重新生成 `frontend/src/types/inertia-props.ts`，都会重新构建框架并重启服务器。它现在监视的是 `src/`、`cmd/`、`Cargo.toml`、`Cargo.lock`、`.env` 和 `lang/` - 也就是那些构建输入，加上启动时各读一次的那两棵目录树 - 而且每一个只在它确实存在时才被带上，因为 cargo-watch 会拒绝一个并不存在的 `-w` 路径。`cmd/` 是全栈脚手架放服务器二进制文件那个 `main.rs` 的地方。这次调用还会传 `--no-vcs-ignores`，因为 cargo-watch 会把 `.gitignore` 施加到明确点名的 `-w` 根目录上，而脚手架会忽略掉 `.env`，否则 `-w .env` 就什么都监视不到；`-w` 已经把这个面收窄过了，所以这个标志不可能再把它放宽。前端的编辑，以及生成出来的那些 `.ts` 文件，不会再重启后端了。

- **`serde_json::Value` 现在生成为 `JsonValue`，而不是 `unknown`。** 它过去会劣化成 `unknown`，并且警告说它“不是这个项目定义的一个结构体”，而对一份 JSON 文档来说，这条建议是错的 - 而且脚手架自己的登录页和注册页每一次重新生成都会各触发它两次，所以每一个新建的项目开箱就在警告。它现在会发出一个递归的 `JsonValue` 别名，在生成出来的文件顶部只声明一次，而且只在确实有东西引用它的时候才声明。裸写的 `Value` 也映射到那里，除非项目自己定义了一个 `Value` 结构体。

- **`generate-types` 和 `serve` 都不会再把一个自己没写过的文件报告成已生成。** 因为现在一趟只在发出的内容确实不同时才写，`Generated <path>` 就成了一个关于文件系统的断言，而在一个没有变化的项目上每一次重跑，这个断言都是假的。`generate-types` 现在改说 `<path> is up to date`，一次性运行和 `--watch` 都一样，而 `serve` 启动时的那一趟会说 `N type(s) up to date → <path>`，把计数保留下来。`serve` 的文件监视器现在对一次什么都没写的重新生成保持沉默，文本输出和 `--json` 下都是如此：一个 `types_regenerated` 事件意味着磁盘上生成出来的那个文件现在不一样了，所以保存之后的沉默告诉您的是，您这次编辑并没有改变任何 prop 形态。

### 升级

- **`.suprnova-atomic` 在每一个本地磁盘的根目录下都是保留名。** 这个暂存目录必须待在根目录里面 - 当根目录是一个挂载点时，根目录的兄弟目录可能落在另一个文件系统上，那样每一次 rename 都会带着 `EXDEV` 失败 - 所以这个名字是保留的，而不只是一种约定。任何第一段是 `.suprnova-atomic` 的路径，现在都会被以一个权限错误拒绝（读、写、删除、stat、列举一视同仁），任何通过符号链接*解析*进这个目录的路径也一样，而且这个条目会从 `files`、`directories`、`all_files` 和 `all_directories` 里被过滤掉。如果某个磁盘根目录里已经有一个属于您自己的 `.suprnova-atomic` 条目，那么它通过这个磁盘就再也够不着了：请在升级之前把它挪到别处。一个叫这个名字的普通文件会在注册时就被拒绝，并附上一条说明原因的消息，而不是稍后在驱动程序内部才失败。这个名字以 `suprnova::ATOMIC_STAGING_DIR` 导出，好让备份和同步工具能把它排除掉。
- **以 rename 发布会替换掉目标的 inode。** 在本地磁盘上重写一个对象，不再保留它原先的权限模式、属主或者硬链接，而一个持有已打开描述符的读取方，会继续读到旧内容，而不是看到新字节。这是原子发布的标准代价，但如果您原先依赖这两者中的任何一个，那它就是一次行为变更。
- **一次条件写入需要一个支持硬链接的文件系统。** `if_not_exists` 是用 `link(2)` 发布的，而 FAT、exFAT 以及某些网络文件系统并不支持它。在那些文件系统上，它会干脆失败，而不是回落成“先检查、再覆盖”，因为一次回落会递给您一个并不成立的独占性保证。这个磁盘上别的一切都不受影响。
- **第一次 `append` 如果失败，会留下一个空对象。** append 是唯一一个不以单独一步发布出去的操作，所以对象是在字节落地之前就被创建出来的；一次失败或者被中止的首次 append 会把它留在那里，这与向一个已经存在的对象追加内容时一直以来的情形完全一样。
- **磁盘根目录里一个悬空的符号链接会被拒绝，而不是被覆盖。** 一条符号链接目标并不存在的路径，现在再也不能通过这个磁盘被写入、被追加、被复制到、被移动到或者被删除。`1.3.4` 会把这样一条链接替换成一个普通文件；这道防护无法证明一条解析不了的链接通向哪里，而通过这样一条链接去创建，创建出来的是这条链接的目标，位置可以是宿主机上的任何地方，所以它现在会拒绝。如果您本来就想写到那边去，请在磁盘之外把这条链接删掉。
- **没有任何东西会去清扫这个暂存目录。** 它装着正在飞行中的临时文件，以及一个发布到一半就死掉的进程留下来的那些东西，所以一台处在崩溃循环里的宿主机会让它无限地长大。在没有任何东西正在写这个磁盘的时候把它清空是安全的；建议把它从备份里排除掉。

## 1.3.4 - 2026-08-25

### 新增

- **读穿透磁盘接受一个 `copy` 标志，并且能跨后备磁盘完成 `copy` / `rename`。** 在 `ReadThroughConfig` 上设置 `copy: false`，就能在应答命中后备磁盘的读取时不把它们写穿过去，这会把这个磁盘变成一层透明的覆盖层，并把每一次取回都收窄到您所要的那个范围。`copy` 和 `rename` 现在会把一个只存在于后备磁盘上的源对象，流式传到主磁盘上的目的地；一次 `rename` 还会删掉后备磁盘上的源对象，这样之后的读取就不可能把这个被移走的对象复活。那些条件会跟着这条流式路径一起走：`if_not_exists` 仍然会拒绝一个已经存在的目的地，一次复制的源版本决定后备磁盘交出哪一个对象，而一次复制的 `if_match` 会被以 `Unsupported` 拒绝，而不是被悄悄丢掉。一次在中途失败的传输，只会移走它自己创建出来的那个目的地，所以它不可能毁掉一个本来就在那里的对象。
- **防抖作业与防抖的已入队监听器。** `Job::debounce_for()` 会把一阵分发合并成一次运行，时间落在最近一次分发之后的一个窗口时长处，并且携带最新的那份载荷。它是 `push_unique` 的镜像 - 后者保住第一次分发，把其余的都压制掉。`Job::max_debounce_wait()` 阻止一阵连绵不断的分发把这份工作永远推迟下去，而 `Job::debounce_id(&self)` 把这个窗口按实体划定范围，所以对某一个订单的二十次更新会合并起来，而不会碰到另一个订单的。`Queue::push_debounced(job, DebounceOptions)` 在调用点设置这个窗口，而 `DebouncedListener::new(window, build).keyed_by(...)` 会给一个事件监听器做防抖，键从事件派生出来 - 一个朴素的 `QueuedListener` 本来就已经遵从作业自己声明的窗口。每一次分发仍然会入队；合并是在工作进程那边结算的，它会把一个被取代的信封确认掉，并发出 `JobDebounced`。防抖是失败开放的：一个过期或被淘汰的窗口会让这个作业跑起来，而不是把它丢掉。每一次真正的运行都会开启一个全新的最大等待窗口，所以一阵分发总是从它自己的第一次分发开始计量它的最大等待时长，而不会继承上一阵的。一个作业不能同时声明 `debounce_for` 和 `unique_id`，而链和批次会拒绝一个防抖的作业 - 一个被取代的链环会把这条链的其余部分晾在那里，而一个被取代的批次作业，会让这个批次的待办计数永远停在零以上。信封为此携带了两个附加字段，而对每一次非防抖的推送，它在传输格式上仍然逐字节相同。

- **`Storage::register_read_through` 把两个磁盘组合成一个读穿透磁盘。** 读取和元数据先对着主磁盘解析，然后回退到第二个磁盘；任何在后备磁盘上找到的东西都会被写穿到主磁盘上，所以一次存储迁移能在真实流量之下完成。写入和列举留在主磁盘上，而一次删除会把这个对象从两个磁盘上都移走。当一次失败的提升必须浮现出来、而不是退化成一次后备读取时，请设置 `throw_on_promotion_failure`。一次提升是原子地发布出去的，所以没有任何读取方能看到一个写了一半的对象，而且它会把后备对象的内容类型、缓存控制、内容处置、内容编码和用户元数据都带过去。一次带版本的或者条件的读取，会带着它的条件原封不动地被传递下去，被应答，但不会被提升。
- **`Queue::forward` 按名字重定向一整个队列。** `Queue::route` 是按作业类型定键的，而 `Queue::forward("default", "high")` 是按队列名字定键的 - 这是那根用来退役一个池子、吸收一批积压，或者把工作从一个您即将下线的池子上挪走的杠杆，不需要碰任何一个作业或者路由。它在两侧都生效：新的、解析到了 `default` 的推送会落在 `high` 上，*并且*一个以 `--queue=default` 启动的工作进程会去排空 `high`，所以目的队列不会攒下没人认领的工作。转发 `default` 会捕获那些没有点名任何队列的作业。一次转发是一次单一的查找，绝不成链，所以一次对调（在 `a -> b` 之上还注册了 `b -> a`）或者一次更长的轮换，是一次说得通的池子交换，而不是一个环 - 和 Laravel 完全一样，它的解析器就是这同一次单一查找。暂停仍然是按一个工作进程启动时所用的那些名字来求值的，所以 `Queue::pause(&connection, "default")` 会停住那个工作进程，哪怕 `default` 正被转发。`Queue::forward_on(from, to, connection)` 把一次转发限制到一个连接名字上，比较的对象是这个进程的连接名字，而不是某个作业声明的连接，所以这次重定向的两半是按同一个值把关的。`Queue::forward_for(from)` 把一条转发读回来，而 `Queue::try_forward` 是那个可失败的对应方法。那几个检查调用（`Queue::pending_jobs` 及其兄弟方法）刻意不跟随转发，所以一个被转发的队列上遗留下来的积压仍然保持可见。

- **读形状的 Redis 命令会重试一次瞬时故障，而不是把它浮现出来。** 连接管理器本来就已经在后台重连了，但撞上那个死套接字的命令，仍然让您这次调用失败了。`GET`、`EXISTS`、`Cache::flush` / `Cache::flush_tags` 背后的那些 `SCAN` 和 `SSCAN` 分页、队列驱动程序的 `XLEN` / `ZCARD` / `XPENDING` 读取，以及限流器的 `Retry-After` 计算，现在都会在一小段停顿之后重试一次。`REDIS_COMMAND_RETRIES` 在此之上追加重试，上限夹在 10。请按秒而不是按毫秒来给这次重试做预算：第二次尝试要等待替换连接就位，所以它要付掉这个驱动程序的整个连接预算和响应预算，而且一条超时的命令和一个被断开的套接字一样都算瞬时故障。写操作在任何设置下都绝不重试：一个瞬时错误意味着连接失败了，而不是服务器拒绝了这条命令，所以重复一次 `SET`、一次 `INCR`、一次锁获取、一次限流计数，或者一次队列弹出，都可能让它跑两遍。错误消息没有变化，所以任何基于它们做匹配的东西都照常工作。
- **一个被暂停的工作进程现在会告诉您它被暂停了。** `queue:work` 会为每一次状态转变打印一行 - `2026-08-25 14:03:11 Queue billing PAUSED`，回来的时候则是 `RESUMED` - 而且这个工作进程会发出 `WorkerQueuePaused` / `WorkerQueueResumed`，好让您把同样的信号路由进您自己的告警系统。这是工作进程那一侧的一对；已有的 `QueuePaused` / `QueueResumed` 是在跑 `queue:pause` 的那个进程里触发的，而那绝不会是工作进程，所以在此之前，一个因为有人暂停了它的队列而安静下来的工作进程，和一个卡死了的工作进程是分辨不出来的。每一个事件为每一次状态转变各触发一次，而不是每轮询一次就触发一次。它们的 `queue` 字段是可选的：一个不带 `--queue` 启动的工作进程会排空一切，在 `pause_all` 之下它没有队列名字可以报告，所以它报告的是 `None`，而不是编出一个监听器可能拿去匹配的名字。
- **`?include=` 的路径被限制在五段之内，而 `max_relationship_depth` 可以挪动这个上限。** 一张有环的关系图，会把 `?include=author.posts.author.posts...` 变成一场由客户端控制的扇出，唯一的界限就是查询字符串。路径现在会在解析的过程中就被截断；在 `bootstrap::register()` 里调用 `suprnova::max_relationship_depth(n)` 可以改这个上限，或者传 `0` 把 include 整个关掉。
- **`Gt`、`Gte`、`Lt` 和 `Lte` 把一个字段与一个数字、或者与另一个字段做比较。** `CompareWith` 用一个值同时点出操作数和度量方式：`Number` 用于一个字面量，`NumericField` 用于一个数值型的兄弟字段，而 `LengthField` 用于一个按字符数比较的兄弟字段。一个这条规则量不出来的操作数，会让这个字段失败，而不是 panic。
- **有三条成员资格规则加入了内置集合：`InArray`、`Contains` 和 `DoesntContain`。** `InArray` 拿一个值去对另一个字段的列表做检查，而且您是直接把列表传进去的，不是在一个规则字符串里点这个字段的名字。`Contains` 和 `DoesntContain` 作用在一个 JSON 数组上，并且只把一个参数与字符串元素相匹配，所以 `1` 和 `"1"` 仍然是两回事。
- **数据库连接池现在有了存活性旋钮。** `DB_IDLE_TIMEOUT`、`DB_MAX_LIFETIME`、`DB_ACQUIRE_TIMEOUT`、`DB_TEST_BEFORE_ACQUIRE` 和 `DB_PING_AFTER_IDLE` 控制着连接池什么时候关闭、回收和 ping 一条连接，并配有对应的 `DatabaseConfig::builder()` 设置方法。每一个默认都是未设置的，所以一个既有部署的连接池行为和以前完全一样。当一个 NAT 网关或者防火墙会丢弃空闲连接时，请用它们：sqlx 没有暴露任何与 libpq `keepalives_*` 等价的东西，所以连接池回收就是那个机制。
- **`db:seed <Class>` 会报告它的进度。** 一次有针对性的运行会在这个填充器之前打印一行 `RUNNING`，在它之后打印一行带耗时毫秒数的 `DONE`。一个光秃秃的 `db:seed` 保持沉默。那个格式化函数 `suprnova::two_column_detail`，您自己的 `#[command]` 处理程序也能用。
- **多对多关系现在可以按中间表的列过滤了。** `where_pivot`、`where_pivot_op`、`where_pivot_in`、`where_pivot_not_in`、`where_pivot_null`、`where_pivot_not_null`、`where_pivot_between`、`where_pivot_not_between`、`where_pivot_group` 以及它们的 `or_` 孪生方法，会约束 `BelongsToMany`、`MorphToMany` 和 `MorphedByMany` 上的 `get`、`first` 和 `count`。`where_pivot_group` 接受一个闭包，并渲染成一个带括号的分组，所以它在紧随其后的一个 `or_where_pivot` 里仍然保持为一个整体。中间表过滤器只作用于读取：只要设了一个，`attach`、`attach_with`、`detach` 和 `sync` 就会返回一个错误，而预加载也不会把它们带上。
- **`where_binary` 逐字节地比较列的值。** 这一家子（`where_binary`、`or_where_binary`、`where_not_binary`、`or_where_not_binary`）发布在 `Builder<M>` 上，而 `where_binary` 和 `where_not_binary` 还发布在 `DB::table(...)` 上。MySQL 和 MariaDB 发出 `= binary`；Postgres 和 SQLite 会在这条查询渲染时返回一个错误，而不是回退成一次取决于排序规则的匹配。
- **`Builder::try_to_sql_with_bindings_for` 为某个方言渲染 SQL，而不会 panic。** 它是 `to_sql_with_bindings_for` 那个可失败的对应方法，用于一个构造器确实没法为某个后端渲染出来的场合。
- **`Model::refresh_for_update` 会在一把 `FOR UPDATE` 锁之下重新加载一行。** 当您需要在一条语句里同时拿到这一行的当前状态和那把排他锁时，请在一个事务内部调用它。SQLite 没有行级锁，所以那个锁子句在那里是一次空操作。
- **`Builder::or_where_key` 和 `Builder::or_where_key_not` 以“或”的关系加上主键过滤。** 两者都会像 `or_where` 那样折进前面那个 `WHERE` 子句，而且两者都带有 `or_filter_key` 和 `or_filter_key_not` 别名。
- **`Builder::in_order_of` 把行排进一个明确的序列。** 传一个列，以及您想要的那个顺序的那些值；值不在列表里的行排在最后。这些值是作为参数绑定的，所以从请求数据里取也是安全的。

### 修复

- **维护模式的绕过 cookie 现在在服务端过期。** 那个 12 小时的 TTL 原本是一个由浏览器执行的 `max-age`，所以一个被截获的 cookie 会一直有效，直到您轮换密钥为止。加密的载荷现在携带着这个截止时间，而且每一个请求都会重新检查它。
- **`suprnova serve` 能跑一个没有前端的项目。** 一个用 `suprnova new --api` 脚手架出来的项目没有 `frontend/` 目录，而 `serve` 以前会用“No frontend directory found. Are you in a Suprnova project directory?”拒绝它，除非您传了 `--backend-only`。现在它会跳过 Vite 那一格，以及喂给它的那次 TypeScript 生成，然后把后端跑起来。在这样的项目上，`--frontend-only` 仍然会失败，并带着一条说明原因的消息。

### 升级

- **本次发布之前签发的绕过 cookie 会失效。** 这个 cookie 的载荷从光秃秃的密钥，变成了一个封好的 `{ secret, expires_at }` 对象，而一个没有截止时间的载荷会被拒绝。升级之后请访问一次那个密钥 URL，拿一个新的 cookie。别的都没变：`down`、`up`、`--secret` 和 `--with-secret` 的行为都和以前一样。
- **一条长于五段的 include 路径，现在返回它前五个关系，而不是全部。** 资源允许列表之外的东西从来就够不着，所以没有哪个响应会因此多出数据；只是一条很深的路径会丢掉它的尾巴。有一个状态码会随之改变：一条过深的尾巴点了一个这个资源并不允许的关系的路径，会在任何东西开始校验之前就被截断，所以它现在会带着活下来的那几段返回 `200`，而完整路径以前返回的是 `400` - 请调整任何对那次拒绝做断言的客户端或者测试。如果您的 API 有文档写明的路径比那还长，请用 `suprnova::max_relationship_depth(n)` 把上限抬高。
- **`DatabaseConfig` 多了五个公开字段。** 用结构体字面量来构建它的代码不再能编译。请用 `DatabaseConfig::from_env()` 或者 `DatabaseConfig::builder()`，两者都会用那些保持今天连接池行为的默认值把新字段填上。

## 1.3.3 - 2026-08-25

### 新增

- **故障转移队列连接。** `FailoverQueueDriver` 包住一个有序的连接列表：第一个连接拒绝掉的推送，会在下一个上重试，依此类推沿着列表往下走。用 `QUEUE_DRIVER=failover` 加上 `QUEUE_FAILOVER_CONNECTIONS=redis,database` 从环境变量把它接起来（每一项都读它自己那个驱动程序的变量，所以一个 `database` 项仍然需要先有 `DB::init()`，也仍然会带上它的失败作业存储），或者用 `FailoverQueueDriver::new(vec![(label, driver), ...])` 直接把它构建出来。只有写操作会往下穿：`push` 和 `bulk_push` 会走这个列表，而 `pop`、`pop_from`、`ack`、`nack`、`release`、`settle`、`clear`、全部四个计数器以及全部三个检查列举，都只委托给第一个连接、绝不给别的，因为一个预留令牌只对签发它的那个驱动程序有意义。运维上的后果是被写进文档、而不是被糊过去的：一个跑在故障转移连接上的工作进程只会排空主连接，所以任何转移到了后备连接上的东西，都需要它自己的工作进程。`bulk_push` 会把每一个信封分别推送，而不是转发一整批，这既保住了每个信封自己的 `available_at`（Laravel #60950），又避免了一批被主连接接受了一半的作业被整批重新推到后备连接上。一次拒绝会分发 `queue::events::QueueFailedOver { connection, job_name, exception }`，并且是边沿触发的：一个连接在进入失败状态时报告自己一次，之后保持安静，直到后来有一次推送在它上面成功、把它重新武装起来为止，所以一次故障只产生一条告警，而不是每分发一次就来一条。当每一个连接都拒绝时，这次推送返回最后一个连接的错误。空的连接列表、缺失或为空白的 `QUEUE_FAILOVER_CONNECTIONS`、一个嵌套的 `failover` 项，以及一个点名了并不存在的驱动程序的项，全都是启动错误 - 那种“警告并回退到内存”的行为留在 `QUEUE_DRIVER` 本身上，在那里一个笔误没法把一个易失的后端接进一条持久的链里。
- **队列检查 API。** `Queue::pending_jobs(queue)` / `delayed_jobs` / `reserved_jobs` 会把现有的 `pending_size`/`delayed_size`/`reserved_size` 这几个计数器背后真正的那些信封列举出来，形式是 `InspectedJob` DTO（`id`、`queue`、`name`、`attempts`、`payload`、`created_at`）- 对应 Laravel 的 `InspectedJob`。一个 `Option<&str>` 的队列过滤器，把 Laravel 的 `pendingJobs($queue)` / `allPendingJobs()` 这一对（以及 `delayedJobs`/`reservedJobs` 的对应物）各自收拢成一次调用。`QueueDriver` trait 的默认实现是一个诚实的 `Err` - 而不是 Laravel 给 Beanstalkd/SQS 的那个空集合默认值，那个读起来像是“队列里什么都没有”，哪怕明明有 - 所以一个还没实现检查的驱动程序会明说；`sync`/`null` 用 `Ok(vec![])` 覆盖它，因为对它们来说那确实就是事实。内存、数据库和 Redis 这三个驱动程序都实现了完整的列举：内存驱动程序的延迟存储从一个光秃秃的 `DelayQueue<Envelope>`（没法迭代）换成了一个 `DelayQueue<Uuid>` 加一张以 id 为键的映射；数据库驱动程序复用了那几个尺寸计数器一模一样的谓词，再加上 `ORDER BY available_at`，而一行 `envelope_json` 解码失败的记录仍然会被列出来（`id: None`，`payload: {"unparseable": true}`）而不是被丢掉，所以一行毒丸数据没法让运营者对队列的其余部分视而不见；Redis 的 `reserved_jobs` 的范围限定在这个消费者进程内的那些预留上（有文档说明），而 `pending_jobs` 会通过 `XRANGE` 分批扫描这个流。`Queue::fake()` 获得了对应的 `pending_jobs()`/`delayed_jobs()` 辅助函数，它们会把记录下来的推送投影出来，其中 `attempts` 永远是 `0`，`created_at` 永远是 `None`。
- **提交后分发。** `Job::after_commit()` 会把一次推送按住，直到外围的 `DB::transaction` 提交为止，这样另一个进程上的工作进程就永远不可能取到一个描述着事务尚未落成持久的那些行的信封。等待的是整个推送，而不只是驱动程序那一次写：信封的构建、`JobQueueing` 和 `JobQueued` 全都发生在提交时刻，所以绝不会有监听器被告知一个随后被回滚丢弃掉的作业。一次回滚会把这次推送整个丢弃；在事务之外，推送立即发生 - 正是这一点，让一个作业类型可以声明这项选择加入，而不需要每一个分发点都知道自己那条代码路径是不是事务性的。就单次分发而言，`EnvelopeOverrides::after_commit` 的优先级高于作业：`Some(true)`（带一个简写 `Queue::push_after_commit(job)`）会把一个并没有选择加入的作业也推迟掉，而 `Some(false)` 就是 Laravel 的 `beforeCommit()`。一次被推迟的 `Queue::push` 会以提交时刻、而不是推送时刻来重新解析 `Job::delay()`，而 `Queue::push_later` / `later` / `later_with` 则原封不动地把调用方那个绝对时间戳带过去。`Queue::push_unique` 会立即取走它的去重锁，哪怕这个信封被推迟了，所以同一个事务里的一个重复项仍然会被压制掉，而一次回滚会按所有者范围释放那把锁。`Queue::bulk` 作为一个整体被推迟。`Queue::fake()` 会立即记录一次推送，连同推迟与否一起，与 Laravel 的 `Bus::fake` 一致。手写的 `DB::begin_transaction` 从不推迟 - 它不安装任何环境事务，所以没有一次提交可以把回调挂上去。每一种让提交没能落地的结局，补偿方式都完全相同，包括一次被数据库拒绝的 `COMMIT`，以及一个泄漏出去、把提交挡住了的 `TxHandle`；而 `Transaction::rollback_to` 对它所回退的那个范围来说也算其中一种：一次在保存点内部被推迟的推送，会在那个保存点回滚时被丢弃，它的锁也就在那时被释放，而在这个保存点之前登记的一切都不受影响。已入队的邮件、通知、批次和链条目前还不会推迟。
- **处理开始即释放唯一性的作业。** `Job::unique_until_processing()` 会在处理开始时释放那把唯一性锁 - 在这个作业的中间件走完之后、处理程序运行之前 - 而不是把它按住整个 `unique_for` 窗口；当这把锁的存在是为了合并排队中的重复项、而不是为了把执行串行化时，这正是您想要的。一个被中间件释放回队列的作业会保住它的锁，因为它还没有开始处理；一个被中间件删除或者送进死信的作业则会交出它的锁。释放是按所有者范围来的：`Queue::push_unique` 会把缓存锁的所有者令牌记在信封上（`Envelope::unique_lock_owner`，一个附加字段，它让每一次非唯一推送的冻结传输格式保持逐字节相同），工作进程再用那个令牌来释放，所以一次被重投的尝试永远不可能强行释放一把如今由更新的一次分发持有的锁。配套的幂等表面也是公开的：`Idempotency::commit_on_success_owned` 会把锁的所有者交给函数体并把它返回，而 `Idempotency::release_owned(key, owner)` 会按所有者范围释放，在锁不存在或者被别人持有时报告 `Ok(false)` 而不是一个错误。朴素的 `unique_id` 作业没有变化，仍然让 `unique_for` 的 TTL 充当去重窗口。
- **`Gate::default_denial_response` 定制一次朴素拒绝的默认形状。** 对应 Laravel 的 `Gate::defaultDenialResponse($response)`。设置一次 - 通常在 `bootstrap::register()` 里 - 它就会重塑恰好两种结果：一个朴素的 `false`（一个 bool 门 - `Gate::define` / `Gate::define_async`，包括一个返回 `bool` 的 `#[policy]` 方法 - 或者一个判定为 `false` 的 `before`/`after` 钩子），以及一次根本没有任何东西做出过判定的求值（一个未定义的能力，钩子也没有意见）。这些以前全都会坍缩成一个朴素的 `Response::deny()`（一个 403）；现在它们会浮现为这个默认值所携带的那个 `Response`，例如用 `Response::deny_as_not_found()` 得到一个 404，从而在整个应用范围内隐藏一个资源的存在，而不是一个门一个门地改。这个默认值只作用于朴素的 `false` - 一个用 `define_with` / `define_async_with` 注册的门已经返回了它想要的那个 `Response`，而那个 `Response` 总是原封不动地穿过 `Gate::inspect`，这与 Laravel 自己的规则一致：默认值绝不替代一个被返回的 `Response` 对象。一个形状为 `Response::allow()` 的默认值会被拒绝（记日志、忽略），而不是悄悄把每一个 bool 门都反转成允许 - 关于这一处刻意与 Laravel 分歧的地方（Laravel 没有这样一道防护），请参见 `Gate::default_denial_response` 的文档注释。
- **`Password` 校验规则家族随本版发布，包括 Have I Been Pwned 的 `uncompromised()` 检查。** `Password::min(n)` 加上那几个强度构建器（`.max()`、`.letters()`、`.mixed_case()`、`.numbers()`、`.symbols()`）逐字移植了 Laravel `Password` 规则的那些正则 - 一个普通空格就能满足 `.symbols()`，与 Laravel 的 `\p{Z}` 分隔符类一致。`.uncompromised()`（或者 `.uncompromised_with_threshold(n)`）会拿这个密码去对 Have I Been Pwned 的 k-匿名 range API 做检查：只有密码 SHA-1 哈希的前 5 个字符会离开进程，而一次网络故障、超时或者非 2xx 响应会失败开放，而不是把注册挡住，与 Laravel 的 `NotPwnedVerifier` 完全一样。因为那次检查是一次 HTTP 往返，`Password` 是唯一一个同时实现 `Rule`（只做强度，供同步的 `validate!` 行使用）和 `AsyncRule`（先强度、再 HIBP 检查，供 `after_validation_async` 使用）的内置规则 - 对一个配置了 `uncompromised()` 的 `Password` 调用同步那条路径，会得到一个醒目的、面向开发者的错误，而不是被悄悄跳过。`Password::defaults_with(...)` 设置 `Password::defaults()` 所返回的那个进程级默认值。新增 `HIBP_TIMEOUT_SECS` 环境变量（默认 30 秒）。`Http::fake_response_text(...)` 是 `fake_response(...)` 的原始响应体兄弟方法，用于针对像 HIBP 这样的 `text/plain` 上游 API 做测试。
- **一个已调度的任务现在可以点名它的 cron 表达式是在哪个时区里读的，而且 `schedule:list` 可以用任意时区渲染整份调度表。** `.timezone(chrono_tz::Tz)` 钉住一个任务，`.try_timezone("Area/City")` 是那个可失败的兄弟方法，用于一个只在运行时才存在的时区名，而 `Schedule::timezone(tz)` 为它之后注册的每一个任务设定一个默认值。对一个没有钉住时区的任务来说什么都没变：它仍然是按进程的本地时区来求值的。钉住的时区只影响是否到期 - 调度器仍然每个进程分钟节拍一次，同一分钟的那道去重关卡也没有动过。请注意，一个实行夏令时的时区会让某些挂钟分钟发生两次、另一些一次都不发生，所以一个钉在这种分钟上的任务可能跑两次、也可能被跳过；调度那一章带有完整的警告。`schedule:list` 获得了一个 `--timezone` 选项和两个新列：打印出来的表达式是写在哪个时区里的，以及这个任务下一次触发的分钟。一个钉住了时区的任务，它的表达式会被改写进这份列举所用的时区里，当它在那个时区里跨越午夜时会拆成好几行；而当一次忠实的改写不可能做到时，它会被原封不动地保留下来 - 跨越一次夏令时切换时、当一次跨日翻转不得不把一个受限的“月中某日”和“周中某日”一起挪动时，或者当它不得不判定二月有多长时。`chrono_tz::Tz` 从 crate 根部被重导出，所以消费它的应用不必往自己的 `Cargo.toml` 里加 `chrono-tz`。
- **一套 Laravel 形状的图像子系统，位于 `suprnova::media` 里、默认开启的 `media` feature 之后。** `Image::from_bytes/from_path/from_disk/from_upload/from_stream` 构建出一条惰性管道 - `resize`、`scale`、`crop`、`cover`、`contain`、任意角度的 `rotate`、`flip_vertically`/`flip_horizontally`、`blur`、`sharpen`、`grayscale`、`to_format`、`quality` - 最后用 `to_bytes`、`to_response`、`save`、`store`、`dimensions`、`mime_type` 或者 `dominant_color` 收尾。它读写 PNG、JPEG、WebP、GIF 和 BMP；AVIF 输出推迟到那个自研的 AV1 编码器发布之后，到那时它就是一个新的 `OutputFormat` 变体，除此之外别无改动。和 Laravel 的 `gd`/`imagick` 分野一样，这里有两个驱动程序：`IMAGE_DRIVER=oxideav`（默认）跑在纯 Rust 的 [OxideAV](https://github.com/OxideAV) 编解码器家族之上，没有原生库，也没有东西要装；而 `IMAGE_DRIVER=magick` 会去调用一个宿主上安装好的 ImageMagick 7，以换取更宽的输入支持，包括 HEIC。解码限制（`IMAGE_MAX_DIMENSION`、`IMAGE_MAX_ALLOC_BYTES`）会在分配任何东西之前，对着输入自己的文件头做检查 - 包括一个扩展 WebP 的内层比特流，它那个仅供参考的画布尺寸没法被用来把一个更大的帧偷运过这道关卡 - 而且所有像素工作都跑在一个阻塞线程上。`magick` 驱动程序会按名字钉死输入的编解码器，而不是让 ImageMagick 从字节里自己挑一个，并且用 `IMAGE_MAGICK_TIMEOUT_SECS` 给每一次调用设界。`ImageDriver` 是通往其他一切的 trait 边界。这个模块之所以叫 `media`，是因为由 OxideAV 支撑的音频和视频表面将来会挨着它住。[图像](images.md)
- **WebP 那道关卡带着一个固定的、不可配置的界限。** 一个 WebP 会把它真正的解码尺寸声明在最内层的比特流 chunk 里，所以框架会走一遍这个容器去把它找出来；那次遍历每层最多访问 4096 个 chunk，并且只跟进两层嵌套，超出其中任何一条的文件都会被拒绝，而不是被测量。从一次没走完的遍历里报出一个数字，会造就一道只要堆上足够多的填充 chunk 就能绕过去的关卡。没有任何 `IMAGE_MAX_*` 变量会影响它，错误信息里也是这么说的。一段 300 帧的动画不受影响；一段 4100 帧的会被拒绝。[图像](images.md#one-bound-is-not-configurable)

- **现在可以安装 OAuth，而不必取代一个应用已有的密码与会话权威。** `MagnetarOAuthOnlyConfig` 和 `init_magnetar_oauth_only` 会安装默认的认证握手引擎和提供方引擎，同时把密码和 passkey 的槽位留空。已经有一张 `users` 表的应用，可以调用 `verify_oauth_identity`，自己把已验证的提供方 subject 映射过去，然后建立它自己平常的那个框架会话。

### 变更

- **`DB::transaction` 现在可以在一次成功的提交之后返回 `Err`**，也就是当一个提交后回调失败的时候：消息读作 `after-commit callback failed (the transaction itself committed): …`，闭包的返回值丢了，而它写下的东西没丢。`DB::transaction_with_attempts` 绝不会重试那个错误，不管回调自己那条消息读起来多像死锁 - 重新运行一个写入已经持久化的闭包，会把那些写入施加两次。
- **新增一个校验语料表键：`validation-password-unverifiable`。** 一个返回 `Err` 的自定义 `UncompromisedVerifier`，不再把它自己的错误文本原封不动地放进 422 响应体里。那段文本改为以 `error` 级别记进日志，而响应携带这个键，渲染成 “The { $field } could not be checked against known data leaks. Please try again.” - 这次检查没有跑起来，这和密码本身有问题不是一回事，而且基础设施细节不该出现在一个面向客户端的响应里。一个自带校验语料表的应用必须把这个键加上，否则它的用户会看到内置的英文回退。
- **`Image` 这个上传校验器现在叫 `ImageFile`。** `suprnova::Image` 是新的那个操作图像的管道类型，对应 `Illuminate\Image\Image`，而那条魔数字节上传规则则取用了 Laravel 给同一个规则类起的名字 `Illuminate\Validation\Rules\ImageFile`。迁移是每个使用点改一行：`UploadedFile<(Image, MaxSize<N>)>` 变成 `UploadedFile<(ImageFile, MaxSize<N>)>`。这是 1.0 之前的变动，由 git 标签的分发模型吸收掉。

### 移除

- **那个没被用到的直接 `image` 依赖没有了。** 它一直是一个基础依赖，而整个工作空间里没有任何一处用到它，白白把 JPEG、PNG、WebP 和 GIF 的编解码器拉了进来；把它去掉，就从依赖树里移走了 `gif`、`image-webp`、`zune-jpeg`、`color_quant` 和 `weezl`。这个 crate 本身仍然会传递性地出现，只带它的 `png` feature，藏在 `totp-rs` 的二维码渲染背后。新的图像子系统改为构建在 `media` feature 背后的那几个 OxideAV crate 之上。

### 修复

- **现在安装 OAuth，不再强迫那些由提供方支撑的应用去走 Magnetar 的 web 绑定校验。** 完整的 `init_magnetar` 那条路径仍然是原子的，也没有变化。仅 OAuth 那条路径会在构建期间就把那些引擎槽位预留下来，只发布 OAuth，并且宁可失败，也不把两个认证权威来源混在一起。

### 升级

- **`Image` 现在是一个不同的类型了；上传校验器叫 `ImageFile`。** 对任何在用那条魔数字节上传规则的人来说，这是源码级的破坏性变更。请在每一个使用点重命名：`UploadedFile<(Image, MaxSize<N>)>` 变成 `UploadedFile<(ImageFile, MaxSize<N>)>`。`suprnova::Image` 仍然能解析得到，但它现在是那个操作图像的管道类型，所以一次漏掉的重命名会编译失败，而不是悄悄改变行为。
- **`EnvelopeOverrides` 获得了一个公开的 `after_commit: Option<bool>` 字段。** 本仓库和脚手架模板里的每一处构造都用了 `..Default::default()`，不需要任何改动。用穷尽式结构体字面量来构建一个 `EnvelopeOverrides` 的代码，必须把这个新字段点出来；`after_commit: None` 保持今天的行为，也就是听从 `Job::after_commit()`。别的都没变：`after_commit()` 默认为 `false`，所以没有哪个既有作业会开始等待一次它以前并不等待的提交。
- **`Envelope` 获得了一个公开的 `unique_lock_owner: Option<String>` 字段。** 传输格式没有变化 - 这个字段是 `#[serde(default)]` 的，并且在为 `None` 时被跳过，所以信封在两个方向上都能逐字节地往返，`schema_version` 也仍然停在 2 - 但任何用结构体字面量来构建一个 `Envelope` 的代码，现在都必须把它点出来。请加上 `unique_lock_owner: None`，除非您是有意要把一把唯一性锁带过这次推送。只读取信封、或者通过 `Queue::push` 及其兄弟方法来构建信封的代码，不需要任何改动。

- 当应用已经自己拥有用户、密码、框架会话和记住我状态时，请使用 `init_magnetar_oauth_only`，而不是 `init_magnetar`。仅 OAuth 的回调使用 `verify_oauth_identity`；完整的 Magnetar 应用继续使用 `complete`。

## 1.3.2 - 2026-08-25

### 新增

- **现在可以通过 `MagnetarConfig::oauth` 注册 OAuth 提供者。** Suprnova 重新导出了 `OAuthProvider` 契约、全部五个第一方提供者与配置类型，以及一个应用所需要的 HTTP、撤销、滥用限流器、授权和自动关联类型。自定义提供者不再需要直接依赖 `suprnova-magnetar`，也不再需要手工保留一个 `MagnetarHostEngine`。

- **一个生产可用的 OAuth 传输和一个框架限流器适配器现在随框架发布，并从 crate 根部导出。** `ReqwestOAuthTransport` 实现了令牌、userinfo 和撤销 I/O：默认禁用重定向，超时 30 秒，带一个默认的 `User-Agent`，响应体上限 1 MiB。`FrameworkAbuseLimiter` 复用应用配置好的 `RateLimiterDriver`；应用不必再手写这两个适配器中的任何一个。

### 修复

- **`init_magnetar` 现在会把 OAuth 与密码、passkey 服务作为一次预留的安装一起发布。** OAuth 服务在发布之前就构建好，而在这次预留生效期间，三个引擎槽位全都保持隐藏。一次失败的或者重复的 OAuth 配置，不可能在缺少配置好的 OAuth 注册表的情况下，就让密码和 passkey 状态可见。

- **自定义提供者可以提供 userinfo 请求头。** `OAuthProvider::userinfo_headers` 会与宿主拥有的 bearer 请求头合并，从而满足像 GitHub 的 `User-Agent` 和媒体类型 `Accept` 请求头这样的要求，同时又不让一个提供者替换掉 `Authorization`。

### 升级

- **`4faaa933` 那次切换到 Magnetar，移除了 Torii 的 OAuth 安装路径，却没有把它的替代品接进默认初始化器里。** 旧的变通做法要求构造一个自定义宿主引擎、调用 `oauth_service`，再单独安装那个适配器。请把这个变通做法换成 `MagnetarConfig::from_sea_orm(database).oauth(oauth_config)` 加上一次 `init_magnetar` 调用。

- **GitHub 社区提供者必须显式处理已验证邮箱。** GitHub 的 `/user` 通常不会给出非公开的电子邮件，而已验证的主地址需要 `/user/emails`。请返回 `email: None` 以使用电子邮件补全握手，或者把 `userinfo_endpoint` 指向一个把两个响应合并起来的宿主适配器；绝不要把一个公开但未经验证的地址当作所有权。

## 1.3.1 - 2026-08-24

### 修复

- **由提供者支撑的应用又可以重置已验证用户了。** 在没有安装 Magnetar 引擎时，`PasswordReset` 会对已经验证过的账户，使用一个明确具备重置能力的 `UserProvider` 和框架的 `auth_flow_tokens`。当 `M` 实现了 `MustVerifyEmail + CanResetPassword` 时，`EloquentUserProvider<M>` 就选择加入；不需要任何 `app_users` 迁移。
- **已发布的那条框架版本线现在包含了两套发布后的修复。** 翻译版 1.3.0 更新日志的排版与标题、CJK 换行、本地化锚点、术语表词条和正文标点都已经对齐，而不再分散在互相分叉的本地分支和远程分支上。
- **打标签之后的 CLI 与 Magnetar 加固也包含在内。** 开发进程的清理用的是那套已经补全的进程组回退，而本地的资格校验契约覆盖了已发布的 ref 以及 plugin-SDK 的 SQLite 通道。

### 安全

- **提供者回退绝不会把密码重置当作首次邮箱证明。** 未知地址和未验证地址收到的是同一个不发邮件的响应。当一个未验证账户必须通过重置来证明邮箱所有权时，请安装 Magnetar，这样凭据清理、认证 epoch 的推进和吊销才会保持原子。提供者回退的完成会通过 `PasswordResetOutcome` 报告框架会话和记住我凭据的吊销失败。

### 升级

- **请把每一处 `v1.3.0` 的 Git 依赖挪到 `v1.3.1`。** 有自己 `users` 表的应用保留它们配置好的 `UserProvider`；它们不会仅仅为了重置一个已经验证过的账户，就去初始化默认的 `app_users` 引擎。使用 Magnetar 凭据或者未验证账户首次证明的应用，继续初始化 Magnetar。

## 1.3.0 - 2026-08-24

### 安全

- **Magnetar 现在会将凭据和会话变更限制在已认证 actor 及账户 auth epoch 之内。** 密码、passkey、关联账户、双因素、opaque-session、JWT、remember、OAuth 和设备授权写入都会拒绝陈旧或已撤销的 actor。未验证账户首次成功的密码重置、magic-link 或 OAuth 已验证邮箱证明会推进 epoch，并以原子方式移除临时凭据、会话、remember 状态及抢占者 TOTP 注册。已验证账户会在密码重置期间保留合法凭据。邮箱验证要求已认证的 token 所有者，且 OAuth 绝不会仅根据邮箱自动关联未验证的现有账户。

- **协议相对的 `_previous.url` 现在无法在写入侧或读取侧通过 `Redirect::back()` 产生跨源开放重定向。** `SessionMiddleware` 不再持久化协议相对的当前 URL：写入会经过 `InertiaValidationRedirectMiddleware` 对 `Referer` 检查所用的同一个清理器，形如 `//host`（或携带 ASCII 控制字节）的请求路径永远不会被记录 - 否则应用的 `fallback!` 路由（标准的 Inertia/SPA 应用外壳模式，任何未匹配路径都回答 `200`）可能会让 `GET //evil.test/anything` 原样持久化这个路径。`SessionData::previous_url()` 现在也会在每次**读取**时应用相同的检查，所以一个从此修复之前的版本升级而来的会话 cookie - 已经携带了当前进程不会写出的原始、未经清理的值 - 会自愈为“没有记录任何内容”，而不是被信任。这样，无论是旧的中毒 cookie 还是新的恶意请求，都不能把一个跨源 `Location` 交给 `Redirect::back()`、`Redirect::refresh()` 或 `url::previous()`。值未通过任一检查时会被视为不存在，而不是被替换成合成值，因此一个真正良好的 previous URL 永远不会被覆盖。
- **Inertia 验证重定向桥接的 `Referer` 检查又关闭了两个同源绕过。** `InertiaValidationRedirectMiddleware` 的 `303` 目标此前只拒绝以字面 `//` 或 `/\` 前缀开头的 `Referer` - 类似 `Referer: /<TAB>/evil.test` 的值会漏过去，因为 WHATWG URL 解析器会在比较 origin 之前从整个字符串中剥除 ASCII tab 和换行，所以浏览器会把它读成 `//evil.test` 并跟随 `303` 跨源跳转。现在检查会拒绝候选值中任何位置的 ASCII 控制字节（C0 或 DEL），而不仅是两个列出的前缀内部。另一方面，最后的后备路径 - 当 `Referer` 和会话的 previous URL 都不可用时使用的失败请求自身路径 - 此前从未清理：origin-form HTTP request-target 在语法上可以以 `//` 开头，因此原始客户端或不做规范化的代理也能把这个“安全的最后手段”变成跨源重定向。两条路径现在共享同一个根相对检查；即使请求自身路径也未通过检查，也会回退到 `/`。
- **Cookie 密文现在通过带上下文的 v2 AAD 绑定到其逻辑 cookie 名称。** `Cookie::encrypted` / `Cookie::read_encrypted_for` 会阻止为一个 cookie 槽铸造的值在另一个槽中解密，而逻辑名称绑定也会让之后的 `__Host-` / `__Secure-` 线缆前缀切换保持安全。无版本兼容窗口会先在整个密钥环上尝试 v2，再在整个密钥环上尝试 v1，因此现有 cookie 能熬过这次推出；v1 回退会保留旧的重放弱点，直到计划中的 1.4.0 移除。
- **会话和 remember-me cookie 前缀现在会在启动时验证，并在渲染时强制执行。** `SESSION_COOKIE_PREFIX=__Host-` 要求 `Secure`、`Path=/` 且没有 `Domain`；`__Secure-` 要求 `Secure`。无效的启动组合会在开始提供服务前失败，渲染器会重写无效的带前缀请求头，而不是让浏览器静默丢弃它们。

### 新增

- **Suprnova 身份验证现在运行在内部 Magnetar 引擎上。** 框架拥有的 `Auth` 门面在移除 Torii 依赖的同时，保留现有的密码、magic-link、passkey、OAuth、bearer、锁定、会话和双因素调用点。默认引擎以原子方式安装密码/会话和 passkey 适配器，将生命周期交付租约存储在应用程序数据库中，并共享应用程序的规范 `i64` `app_users` 身份。
- **一个感知形状的身份验证迁移运行器现在覆盖 Torii、Suprnova web 和 Suprnova API 来源。** 试运行会将稳定的计划 ID 绑定到持久的行和架构指纹，以及目标身份决策。应用执行使用事务性导入、重试账本、形状拥有的清理和冲突拒绝。MySQL 使用受写入屏障保护的影子交换，并配有预复制日志、行和架构一致性、可恢复重命名，以及保留清理的还原。
- **`MAIL_DRIVER=file` 现在会为每条消息向 `MAIL_FILE_PATH` 写入一个 RFC 5322 `.eml` 文件**（默认值为 `storage_path("mail")`；相对值锚定在应用程序基目录，而不是进程 CWD），这样本地邮件可以在邮件客户端中打开，而不必从日志行中读取。该文件携带 SMTP 发出的同一组完整请求头，包括 `X-Priority`、`Importance`、`X-Tag`、`X-Metadata-*` 和 `Return-Path`。和 `log`、`memory` 一样，它不会投递：除非设置 `MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`，生产启动会拒绝它。
- **`FrameworkError::External` 现在携带它所包装的错误。** `FrameworkError::from_external(e)` 和 `FrameworkError::from_external_with("saving user", e)` 会让原始错误作为 `std::error::Error` source 保持可访问，而不是将其熔化成字符串。`FrameworkError::external_source()` 会为 downcast 返回它 - 请使用它，而不是会产生共享 `Arc` 句柄的 `source()`。两个构造器都映射到 HTTP 500。
- **5xx 日志现在会渲染完整的错误 source 链。** `render_error_chain` 会遍历 `source()`，并接入框架错误日志行、`ErrorOccurred` 事件载荷，以及在 `APP_DEBUG=true` 下发出的 `debug_message` 字段。面向客户端的响应体保持不变，5xx 响应体仍会被清理。
- **`InertiaResponse::scroll_wrapped` / `scroll_with_wrapped` / `try_scroll_wrapped`。** 对于自身就是 envelope（`{ data: [...], meta: {...} }`）的值，将 scroll prop 的合并指令嵌套在 `<key>.<wrap_key>` 下，而不是裸键下 - 使用 `mergeProps: ["users.data"]` 而不是 `["users"]`。Laravel 的 `ScrollProp` 无条件地在 `"data"` 下包装；Suprnova 的内置分页器返回裸行数组，所以这是选择启用，而不是每个调用方都要绕开的默认行为。新的 `ProvidesScrollMetadata` trait（`page_name` / `previous_page` / `next_page` / `current_page`，带默认的 `scroll_metadata()`）为这个 crate 不知道的分页器复刻 Laravel 的同名接口；`LengthAwarePaginator`、`Paginator` 和 `CursorPaginator` 现在实现它，而不是手工构建 `ScrollMetadata`。scroll prop 的 `.match_on(...)` 字段现在也会输出到 `matchPropsOn`，与 Laravel 的 `resolveMergeMatchingKeys`（`Response.php:641-652`）一致；它会像其他 merge prop 一样合并 `ScrollProp` 的 `matchesOn()`，匹配项以 prop 实际合并的位置为键，即未包装的 `<key>`，或者 `.scroll_wrap(...)` 下的 `<key>.<wrap_key>`。
- **`Prop::merge_with_path`、多字段 `match_on` 以及基于 resolver 的 merge prop。** `Prop::merge_with_path(path)` 会合并 prop 值内部的嵌套字段，而不是整个 prop - `Prop::eager(v).merge().merge_with_path("data")` 会输出 `mergeProps: ["<key>.data"]`，带路径的合并 prop 永远不会同时合并根；`.deep_merge()` 会忽略它，因为 deep merge 本来就会递归遍历每个字段。`Prop::match_on` 现在可以在一次调用中接收一个或多个字段（`match_on(["id", "slug"])`），并在已有的 `match_on("id").match_on("slug")` 链式 `Prop` 组合之上提供它。`InertiaResponse::merge_lazy` / `merge_lazy_with` 增加 `.merge` / `.merge_with` 的 resolver 兄弟方法，与 Laravel 的 `Inertia::merge(fn () => ...)` 对应。
- **部分重新加载的 `only`/`except` 现在理解点号记法。** `X-Inertia-Partial-Data: user.name` 会把 `user` prop 缩小为 `{ name: ... }`，而不是要求整个值或什么都不要求；`X-Inertia-Partial-Except: user.email` 只会删除该字段，保留 `user` 的其余部分。如果两个请求头都列出一条路径，`except` 获胜；裸条目仍然表示整个 prop；未知或类型不匹配的嵌套路径会静默丢弃，不会影响它的兄弟项。`Always` prop 不受影响 - 它们总是完整发送。
- **点号键 prop 嵌套。** `.with("user.name", value)`（以及任何其他 prop 附加方法，无论 eager 还是 resolved）现在会嵌入 `props.user`，而不是发送字面量 `"user.name"` 键，符合 Laravel 基于 `Arr::set` 的 `resolveArrayableProperties` 解包。共享前缀的两次调用 - 先 `.with("user.name", …)` 再 `.with("user.age", …)` - 会累积到一个对象中；没有点号的键不受影响。`App::inertia_share*` 共享注册表键在 wire 上也以同样方式嵌套。解包只会处理顶层 prop *键* - 永远不会递归进入 prop 的值，所以验证 `errors` 包会保留它内部携带的点号字段名。
- **`App::inertia_shared(key)` / `App::flush_inertia_shared()`。** 这是 Laravel 的 `Inertia::getShared` / `Inertia::flushShared`，用于读取和清空静态 share 注册表（`App::inertia_share` / `_lazy` / `_once`）。`inertia_shared` 在读取侧支持与 `inertia_share` 相同的点号记法；对于 lazy 或 once share（没有可供解析的请求）以及未注册的键，它都会返回 `None`。`flush_inertia_shared` 只会清空静态注册表 - 通过 `App::register_inertia_shared` 注册的 trait provider 不受影响，与 Laravel 一致（那里没有需要清空的每请求状态）。
- **`InertiaResponse::always_with(key, resolver)`。** 这是 `.always(key, value)` 的异步 resolver 兄弟方法，用于一个始终包含、且昂贵到值得惰性解析的 prop - 对应 Laravel 的 `Inertia::always(fn () => …)`（`AlwaysProp` 接受任何值，包括闭包）。
- **`InertiaSharedData::share` 现在会收到页面组件名称**，因此 provider 可以按页面改变其输出 - 对应 Laravel 的 `RenderContext`。参见升级。
- **Inertia prop 组合。** `Prop` 现在携带正交标志，而不是九种封闭变体中的一种，因此一个 prop 可以同时 deferred *和* mergeable、mergeable *和* cached，或者 optional *和* cached - 这些是 Inertia 3 协议预期、而封闭 enum 无法表达的组合。使用 `Prop::eager` / `Prop::lazy` / `Prop::from_resolver` / `Prop::absent` 构建，链式调用 `.always()`、`.optional()`、`.defer()`、`.group()`、`.rescue()`、`.merge()`、`.prepend()`、`.deep_merge()`、`.match_on()`、`.once()`、`.as_key()`、`.until()`、`.fresh()`、`.scroll()`，并用新的 `InertiaResponse::prop(key, prop)` 附加。一个 `defer().merge()` prop 会在首次渲染中以 `deferredProps` 公告，并在后续请求中以 `mergeProps` 到达。新的 `MergeMode` 和 `Visibility` 类型描述这些标志；现有每一个构建器快捷方式（`.with`、`.always`、`.lazy`、`.optional`、`.defer`、`.merge*`、`.once*`）都不变。
- **队列暂停/恢复。** `Queue::pause(connection, queue)` / `resume` / `pause_all()` / `resume_all()` / `is_paused(connection, queue)` / `paused_queues(connection, &queues)`，像重启信号一样由 `Cache` 支撑 - `resume_all` 不会清除逐队列暂停，与 Laravel 的行为一致。worker 的 claim gate 紧挨着每次 pop 之前，因此正在处理的作业总会完成；全局暂停会像 Laravel 的 `pausedQueues` 一样短路 `--queue=...` 过滤，而逐队列暂停只对使用显式 `--queue=...` 列表启动的 worker 生效。新增 CLI 命令 `queue:pause [queue] [--all]` / `queue:resume [queue] [--all]`（别名 `queue:continue`），以及供运维人员禁用功能的 `QUEUE_PAUSABLE=false` - 不可暂停的 worker 会忽略暂停信号，而 `queue:pause` 自身会拒绝运行。新增事件：`QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed`。
- **`suprnova::testing::TestResponse`** - 一个流畅的、按 Laravel `TestResponse` 形状包装 `(status, headers, body)` 三元组的封装器，这是每个 HTTP 测试 harness 已经产生的结果：`assert_status`、`assert_ok`、`assert_redirect`、`assert_json`、`assert_json_path`、`assert_json_count`、`assert_see`、`assert_header`、`assert_cookie`，以及（给定 `.with_session_store(...)` 时的）`assert_session_has`。每个断言都返回 `&Self`，失败时 panic，与 `expect!` 契约相同。测试驱动请求的方式无需任何改变。
- **`suprnova new` 现在会脚手架出 SSR 入口。** 每个起始套件（Svelte、React、Vue）现在都提供 `frontend/src/ssr.{ts,tsx}` 和 `build:ssr` npm 脚本（`vite build --ssr`），并接入自己的输出目录（`frontend/bootstrap/ssr/`），因此 SSR bundle 永远不会与 `public/assets/` 中的客户端构建冲突。
- **`InertiaConfig::ssr_bundle_path(path)` / `.ssr_ensure_bundle_exists(bool)`。** SSR gateway 现在可以在派发渲染之前检查磁盘上是否存在已构建的 bundle，对应 Laravel 的 `ensure_bundle_exists` 配置 - 从未启动的 worker 或从未构建的 bundle 会快速失败，而不是为一个注定无法成功的连接支付 `ssr_timeout`。用 `.ssr_bundle_path(...)` 选择启用；与 Laravel 的 `BundleDetector` 不同，该路径永远不会自动检测，因此未设置路径的既有 SSR 配置（以及测试）不受影响。
- **Inertia 访问上的验证失败现在会重定向返回，而不是返回 `422` JSON。** `Inertia::install` 会注册第四个中间件 `InertiaValidationRedirectMiddleware`，将 `X-Inertia` 请求上的验证 `422` 转换成带 flash 错误、指向表单页的 `303` - 因此 `useForm().errors` 无需处理程序代码就会填充。Inertia 客户端会把缺少 `X-Inertia` 请求头的响应视为非 Inertia，并显示错误模态框，所以旧的 `422` 永远无法到达 `form.errors`。非 Inertia 请求保留 `422` envelope，Precognition 试运行不受影响，`X-Inertia-Error-Bag` 会限定 flash 的 bag。重定向目标依次是同源 `Referer`、会话的 previous URL、经过同一清理器的请求自身路径；如果连它也失败就回退到 `/`，绝不会原样信任。
- **`InertiaConfig::with_all_errors(bool)`** - 保留每个字段的全部验证消息，而不是折叠成第一条。对应 Laravel 的 `Inertia\Middleware::$withAllErrors`。
- **`suprnova::testing::AssertableInertia`** - 在 Inertia 页面对象之上提供、形状类似 Laravel `AssertableInertia` 的流畅断言；页面对象既可以从 `X-Inertia` JSON 响应解析，也可以从硬导航 HTML 外壳内嵌的 `<script data-page="app">` 元素解析：`component`、`url`、`version`、`prop`、`has`、`missing`、`where_`、`count`、`has_flash`。使用 `AssertableInertia::from_response` 从 `HttpResponse` 构建，或者使用新的 `TestResponse::assert_inertia()` 从 `TestResponse` 构建。`reload_only`、`reload_except` 和 `load_deferred_props` 会针对调用方提供的 `with_reload(...)` 闭包重放部分重新加载 - Suprnova HTTP 测试会跨越真实 socket，所以没有可硬编码的单一进程内测试客户端。
- **`Cookie::queue`/`queued`/`unqueue`/`expire`。** 一个任务本地的 cookie jar - Laravel 的 `CookieJar` - 允许任何代码为下一次出站响应排队一个 cookie，而不必持有可以附加它的 `HttpResponse`：事件监听器、容器绑定服务、处理程序之前的中间件都可以使用。它由 `Auth::login_remember` 已经使用的同一个每请求槽支撑，以便把 remember-me cookie 带过处理程序边界；`SessionMiddleware` 会把它和 session cookie 一起排到响应上。`Cookie::expire(name, path, domain)` 会排队一个用 `Cookie::forget_with` 构建的删除 cookie。路由的 middleware 链必须有 `SessionMiddleware` - 在其之外四个调用都是静默 no-op，与在 flash scope 外使用 `App::flash` 的行为一致。
- **`HttpResponse::event_stream(stream, end)` 和 `HttpResponse::stream_json(stream)`。** 这是 Laravel 的 `ResponseFactory::eventStream` / `streamJson`，以及 `@laravel/stream-{react,vue,svelte}` 的 `useEventStream` / `useJsonStream` 所期望的精确 wire 形状。`event_stream` 默认将 `Stream<Item = sse::StreamedEvent>` 的每个 item 帧化为 `event: update`，除非 item 自己命名了事件；它会对任何非字符串载荷做 JSON 编码，并附加可配置的终止帧（`EndSignal::default()` 是 `data: </stream>`；`EndSignal::None` 会省略它）。`stream_json` 会将每个 `Stream<Item = impl Serialize>` 作为一个增量刷新的 JSON 数组流出。两者都构建在既有的 `sse`/`stream_bytes` body pipeline 之上，因此与框架其余部分共享它的取消和 panic 隔离行为。
- **`suprnova serve` 现在会重新生成崩溃的开发进程，而不是拆掉整个会话。** 尝试之间使用指数退避 - 200ms，每次连续崩溃都翻倍，上限 5s；进程持续运行 30s 后重置到下限。`--no-restart` 选择退出并恢复之前的行为。`--restart-tries <N>`（默认 `5`，与 Laravel 的 `--restart-tries=5` 一致）会在进程连续崩溃达到该次数后放弃重试，而不是无限尝试；它会打印一条可操作消息，同时让其他进程和会话本身继续运行。`--timestamps` 会在每一行转发文本前加上 `HH:MM:SS`。新的 `Suprnova.toml` `[[serve.process]]` 数组允许项目声明自己的开发进程（Laravel 的 `DevCommands::register`），与后端和前端一起运行；每个进程都有自己的 `[name]` 前缀和可选颜色。现在条目中的未知键或空白 `name`/`command` 会是硬解析错误，而不是静默忽略或稍后才发生不透明的 spawn 失败。`--json` 会改为在 stdout 上每行输出一个 JSON 对象（NDJSON） - 进程启动、输出、退出、计划重启、重启成功、放弃、类型重新生成和关闭事件都会输出；文件 watcher 自身的重新生成通知及 `Ctrl+C` 处理程序的关闭通知在 `--json` 下也不会出现在 stdout - 以便脚本和日志管道使用。将它与 `--timestamps` 组合是安全的但重复，因为每个事件已经携带自己的时间戳。
- **`RequestBuilder::retry_when(predicate)`。** 在内置策略（`.retry(...)` / `.retry_non_idempotent(...)`）本来会执行每次重试之前咨询一个谓词，并接收 `RetryContext { attempt, method, url, outcome: RetryOutcome::TransportError | Status(u16) }`。它与策略组合而不是替换策略：`false` 会否决策略本来会执行的重试；它永远不能强制超过 `max_attempts` 的重试，或强制执行策略本来不会尝试的重试（4xx 状态，或没有 `retry_non_idempotent` 的非幂等方法）。
- **`#[model(touches = [...])]` 现在确实会执行 touch。** 子级被创建、保存、更新或删除后，列表中点名的每个 `BelongsTo` owner 都会在触发写入的同一个 executor 上获得一次 `UPDATE <owner> SET updated_at = ? WHERE <key> = ?` - 因此在 `DB::transaction` 内，touch 会加入该事务并随之回滚。其模型设置了 `timestamps = false` 的 owner 会被跳过，不会写入，也不会报错（Laravel 13.25 关闭了同一个缺口）。通过 `NULL` foreign key 找到的 owner 和软删除的 owner 也会被跳过。没有指名已声明 `BelongsTo` 关系的 `touches` 条目现在是编译错误；多态 owner 目前还不支持。
- **`without_touching_on::<M, _, _>(fut)`** - Laravel 的 `Model::withoutTouchingOn([M::class], $cb)`。它会抑制 `m.touch()` 以及任何指向 `M` 的 owner 级联，但其他类型的 owner 仍会递增。scope 可以嵌套，现有的 `without_touching` 现在除了直接的 `touch()` 调用外，也会抑制 owner 级联。
- **`Model::touch_owners()` / `touch_owners_with_tx(tx)`** - Laravel 的 `touchOwners()`，用于您通过框架不拥有的路径写入 child 行的情况。
- **值形状的验证规则：`ArrayKeys` 和 `Distinct`。** 新的 `ValueRule` trait（`passes(&self, value: &serde_json::Value)`）与 `Rule` 并列，共享同一个带键消息的契约。`rules::ArrayKeys(&[...])` 会拒绝携带允许列表之外任何键的 JSON 对象（Laravel 的 `array:keys`，#60918）；`rules::Distinct { ignore_case, strict }` 会拒绝含有重复元素的 JSON 数组（Laravel 的 `distinct`）。`validate!` 行可以在同一字段列表中接受任一类型的规则 - 分发是自动的，根据规则实现了哪个 trait 选择，而不是通过新的行语法选择。
- **`Job::delay()`** - 作业可以声明默认延迟（`fn delay() -> Option<Duration>`，默认 `None`），由 `Queue::push` 和 `Queue::bulk` 遵循：`available_at` 变为 `now + delay`，而不是 `now`。调用点上的显式延迟仍然优先 - `Queue::push_later(job, at)` 和 `Queue::later(delay, job)` 使用调用方的时间戳原样执行，永远不会咨询 `Job::delay()`。
- **`Notification::{queue, timeout, fail_on_timeout, max_tries, backoff}`。** 排队的 notification（`Notify::queue`）现在会通过 `Mail::on_queue` 使用的 `EnvelopeOverrides` 原语，把自己的队列调优默认值带到每次逐通道 `SendNotificationJob` 推送中。`fail_on_timeout(&self) == true` 会在第一次超时时将其转入死信而不是重试，与 Laravel 的 `#[FailOnTimeout]` notification 属性一致（#61072）。这五项默认值都与 `SendNotificationJob` 现有的 `Job` 默认值相同，因此不覆盖任何值的 notification 不受影响。
- **`Mail::on_queue` / `Mail::on_connection` + `Queue::push_with`/`later_with`。** 排队的 mailable 现在可以用 `Mail::to(..).on_queue("emails").queue(mailable)` 路由自己，也可以通过 `Mailable::queue(&self)` 提供默认值。两者都优先于为该作业注册的任何 `Queue::route` 以及作业自身的 `Job::queue()`/`Job::connection()` - 它们背后的新 `EnvelopeOverrides` 原语（`Queue::push_with(job, overrides)` / `Queue::later_with(delay, job, overrides)`）也会覆盖一次推送的 timeout、fail-on-timeout、max-tries 和 backoff。`MailFake` 的排队快照现在携带解析后的 `queue`，并提供 `queued_on(...)` / `assert_queued_on(name, queue)` 进行断言。
- **`Application::http_bootstrap(f)`** - 仅 HTTP 的启动钩子。它在 `bootstrap` 之后运行，并且只在 `serve` / `web:run` 路径上运行，因此 queue、schedule、workflow worker 以及 console binary 永远不会运行它。worker 和 console 容器镜像不再需要已构建的前端 manifest 才能启动：生产环境缺少 manifest 时 `Inertia::install` 会失败关闭，并且该检查现在只会在真正提供 HTTP 的进程上运行。
- **`Router::inertia(path, component, props)`** - Laravel 的 `Route::inertia`，用于其 handler 本来只有一行的静态页面。它注册 `GET`（HEAD 会落到它上面）并返回一个 `RouteBuilder`，因此路由可以命名并赋予 middleware。`Router::view` 保留为别名。
- **SES v2 发送选项。** SES transport 现在会在 `SendEmail` 上输出 `TenantName`、`ConfigurationSetName` 和 `ListManagementOptions`。每一项都有 transport 层默认值（`SesMailTransport::tenant_name` / `configuration_set_name` / `list_management`）以及逐消息请求头覆盖（`X-SES-TENANT-NAME`、`X-SES-CONFIGURATION-SET`、`X-SES-LIST-MANAGEMENT-OPTIONS`），请求头优先。请求构建时会消费这些请求头，永远不会把它们渲染进消息中。
- **每一个响应构建器现在都有 `without_cookies`。** `HttpResponse`、`Response`（通过 `ResponseExt`）、`Redirect` 和 `RedirectRouteBuilder` 都可以在一次调用中让一列 cookie 过期；`Redirect` / `RedirectRouteBuilder` 还补上了原本缺失的单名称 `without_cookie`。新的 `Cookie::forget_with(name, path, domain)` 会构建一个限定到原 cookie 设置所用 path 和 domain 的删除 cookie - 普通 `forget` 永远不会清除在 `/` 之外设置的 cookie。
- **`Queue::fake()` 会给每一次捕获的 push 加盖 envelope id。** `pushed_with_id::<J>()` 返回 `(job, id)` 对，fake 现在也会分发真实 driver push 所分发的同一对 `JobQueueing` / `JobQueued` 事件（携带该 id），这样测试可以将捕获的 push 与监听器看到的 push 对上。现有 fake 辅助函数不变。
- **`UniqueJobSkipped` queue event。** `Queue::push_unique` 抑制重复项时现在会分发 `queue::events::UniqueJobSkipped { job_name, unique_id, connection }`，因此去重从静默变为可观察。调用返回值保持不变（`Ok(false)`）。
- **查询构建器和集合上的 `model_keys()`。** `User::query().model_keys().await?` 会返回每一行匹配记录的主键，而不 hydrate 单个模型，并投影带表限定的键（`users.id`），所以查询经过 join 仍然有效。`Collection::model_keys()` 是已 hydrate 集合的对应方法。`#[suprnova::model]` 现在也会将键的 Rust 类型声明为 `EloquentModel::Key`，因此两者返回 `key_type` 命名的类型，而不是调用方选择的 turbofish。

### 修复

- **PostgreSQL 软删除现在使用后端感知的占位符，生成的时间戳写入也会遵循声明的转换。** `delete()` 和 `restore()` 会呈现 PostgreSQL 序号占位符，而不是 MySQL 和 SQLite 的 `?` 占位符。生成的创建、更新、保存、touch 和软删除写入也会通过每个字段声明的 `Cast` 存储类型转换时间戳，因此原生 `TIMESTAMPTZ` 列不再接收文本值。感谢 [@i-am-v-alexander-v](https://github.com/i-am-v-alexander-v) 报告这两个缺陷，并在 [PR #3](https://github.com/eas4ai/suprnova/pull/3) 中提交修复。
- **默认 workspace 和 Magnetar gate 运行不再需要实时 PostgreSQL 或 MySQL 服务。** 后端特定行为套件是显式且被忽略的资格测试；如果故意在没有其已配置数据库的情况下调用，这些测试仍会失败。仅测试可达性的测试和永久性 gate 环境要求已被移除，因此无关更改不必在每次验证运行时承担外部数据库设置成本。

- **`PartialFilter::narrow` 现在是 `pub`。** 它的四个同级谓词（`should_include`、`should_include_eager`、`should_include_optional` 以及该类型本身）此前已经公开，但使 `should_include_eager` 的 `true` 回答正确的 narrowing pass - 将 resolved value 缩小到 `only`/`except` 条目实际请求的点号路径 - 仍然是 `pub(crate)`。基于 `PartialFilter` 构建自定义部分重新加载处理的调用方没有公开方式复现这次 narrowing，因此即使 `should_include_eager` 报告键已包含，也会在点号 `only` 条目下完整发送该值。
- **`MailFake` 的 `QueuedSnapshot` 现在可以断言 `.on_connection(...)`。** `Queue::fake()` 在 Wave 3 中随 `assert_pushed_on_queue` 增加了 `assert_pushed_on_connection`；`Mail::fake()` 只获得了 queue 部分，所以用 connection override 排队的 mailable 虽然已被解析并应用于真实 dispatch，却无法通过 fake 断言。新增 `QueuedSnapshot::connection`、`MailFake::queued_on_connection` 和 `MailFake::assert_queued_on_connection` 来补上缺口，形状与 `assert_queued_on` 对应。
- **裸 `only` 条目无法访问点号 shared prop。** `App::inertia_share("auth.user", …)` 后跟 `router.reload({ only: ['auth'] })` 时会直接返回 `props: {"errors":{}}` - share 会彻底消失。注册表将 `auth.user` 存成一个字面键，而 `Arr::set` 解包 pass 只有在每个 prop 都解析之后才会嵌套它，所以 partial-reload gate 既不将仍然扁平的键匹配到 `auth`，也不匹配到其他任何项。现在 `only`/`except` 条目是对称的：条目可以精确指名 prop 的键、指名其内部的路径（`user.name`，会进行 narrowing），或者指名其祖先（对键 `auth.user` 使用 `auth`，因为调用方请求整个根，所以会完整发送 prop）。对一个裸 `except: ['auth']` 来说，它会像 Laravel 已经嵌套的 bag 中的 `Arr::forget` 一样，丢弃其下的每个 prop 键。前缀必须在 segment 边界结束，因此不相关的 `authAgent.user` prop 不会被任一列表触碰。Laravel 不会遇到这点，因为 `Inertia::share` 在 share 时就运行 `Arr::set`；Suprnova 的注册表做不到，因为 lazy share 在请求解析之前没有要嵌套的值。
- **`#[data(lazy(deferred))]` 字段绕过了 `?include=` allowlist。** `resolve_props` 中带 owner 标签的解析路径选择了 `Prop::is_lazy()` 的 prop，而带任何标志的值都不是 lazy，deferred 字段是 `Visibility::Deferred`。因此该字段会在普通 prop 路径之外解析，而那里没有 include-set 检查；任何发送 deferred follow-up 的客户端都会收到它，不论请求是否选择加入。现在 `Prop::resolve_with_owner` 会对所有带 resolver 的 owner-tagged prop 做 gate，而 `resolve_props` 会在其他 block 之前运行这个 gate：`?include=` 之外的字段会整体丢弃（没有值，也不公告 `deferredProps`），而被 `?include=` 点名但不在 DTO allowlist 中的字段会在 `X-Inertia-Partial-Data` 吸收它之前引发 `400`。这不是回归 - Wave 4 之前的代码按 `Prop::Lazy` enum 变体做 gate，而 `Prop::Defer` 也会失败 - 但不管怎样都是一个真实缺口。
- **匹配的 partial reload 会重新公告 `deferredProps`。** 只点名一个 deferred 键的 partial 仍会把每一个其他 deferred 键公告回客户端，客户端随后会再次获取它们，并在下一个 partial 上再次获取。Laravel 的 `resolveDeferredProps` 在请求为 partial 时会立即返回 `[]`，甚至不会检查单个 prop（`Response.php:661-663`）；现在这整个 block 会在任何匹配的 partial 上被丢弃。针对不同 component 的 partial reload 对这个 gate 来说是一次标准 visit，就像其他 visit 一样，因此其公告不受影响。
- **`errors` 包会根据错误来源不同而采取不同过滤。** session-flashed 包在 resolve loop 之前植入，任何 partial-reload filter 都无法触达；而处理程序自己的 `.with("errors", …)` 则经过普通 gate - 所以 `only: ['errors.email']` 会发送完整的 seeded bag，却只发送一个字段的 handler bag；`only: ['users']` 还会用 seeded bag 替换 handler 的 bag，而不是留下该键。两条路径现在都把 `errors` 视为始终可见，与 Laravel 的 middleware 一致；Laravel 将其作为 `Inertia::always(...)` share，并在 `only`/`except` 重建之后通过 `resolveAlways` 重新注入原始值。这是客户端所需的形状：它用 `{...current.props, ...response.props}` 把 partial 响应折叠进去，因此未过滤的空 `errors` 对象会擦除屏幕上已有的消息，而不加过滤的响应会保持正确。键上的显式 visibility flag 仍然优先，所以 `.prop("errors", Prop::eager(…).optional())` 仍按 optional 行为。
- **`Queue::fake()` 现在可以观察每次 push 的 `EnvelopeOverrides`。** 通过 `Queue::push_with`/`Queue::later_with` 推送的作业在 fake 下此前无法与普通 `Queue::push` 区分 - `FakePush` 只携带 payload 和 `available_at`，因此 override 从未离开门面，也没有任何方式断言测试调度到了正确的 queue 或 connection。新的 `queue::testing::pushed_with_overrides::<J>() -> Vec<(J, EnvelopeOverrides)>` 会返回每次捕获的 push 及其声明的值；`assert_pushed_on_queue::<J>(queue)` 和 `assert_pushed_on_connection::<J>(connection)` 覆盖常见的单字段情况，对应 `MailFake::assert_queued_on`。每个其他入口（`push`、`push_later`、`bulk`、`push_unique`、chain/batch dispatcher）仍然不带 override，并记录 `EnvelopeOverrides::default()`，因此普通 push 在 fake 下读取起来仍然等于“没有声明 override”。
- **中途停在响应体的 SSR worker 可能让一次渲染永久挂起。** `SsrConfig::timeout` 只限制等待响应头；响应头到达之后，读取响应体本身没有超时，因此一个接受连接、发送头部、随后停止发送数据的 worker，会让请求超过配置的 timeout 仍然挂起，而不是回退到 CSR（或在 `ssr_throw_on_error` 下报错）。现在两个阶段共享一个 deadline，因此配置的 timeout 会限制整个 SSR 调用，正如其自身文档已经承诺的那样。
- **排队的 cookie - 包括 `Auth::login_remember` 设置的 remember-me cookie - 会在 `SessionMiddleware` 的三条内部 fail-closed 路径上被静默丢弃。** session read 失败、session write 失败和 session-cookie 加密失败都会直接返回合成的 `500`，绕过 `handle` 结尾运行的 pending-cookie drain。该请求通过 `Cookie::queue` 排队的任何内容 - 包括已经提交到数据库的 remember-me token 行 - 都不会以 `Set-Cookie` 请求头到达客户端。三条路径现在都会在返回前 drain pending cookies，与 handler 返回错误或重定向的行为相同。这不覆盖未捕获的 panic，符合 Laravel 自身排队 cookie 会因 panic 丢失的行为。
- **`Queue::push_unique` 现在会遵循 `Job::delay()`，与 `Queue::push`、`Queue::push_with` 和 `Queue::bulk` 一致。** 它此前直接从 `Utc::now()` 计算 `available_at`，所以声明默认延迟（`fn delay() -> Option<Duration>`）的作业通过 `push_unique` 推送时会立即执行，而不是等待该延迟。`Queue::push_unique_later` 和 `Queue::later_unique` 不受影响 - 它们已经接收调用方的显式 timestamp 或 delay，永远不会咨询 `Job::delay()`，与 `push_later`/`later` 遵循相同规则。

### 变更

- **当前开发分支使用 SeaORM 2.0，并要求 Rust 1.94.0。** Suprnova 保留其 Eloquent、`#[model]`、迁移和数据库门面的源代码结构。直接调用 SeaORM 的应用程序必须导入 `ExprTrait` 以使用 SeaQuery 表达式方法，并对预构建的 `Statement` 值使用显式 `*_raw` 连接方法。SeaQuery 现为 1.0，直接 MariaDB 向量驱动程序使用 SQLx 0.9。现有数据库不需要迁移应用程序数据；全新的 PostgreSQL schema 保留基于 serial 的主键。
- **又移除了三个未使用的依赖。** `pretty_assertions` 和 `qrcode` 离开 framework crate（`totp-rs` 已经携带 `qr` feature，因此双因素注册的二维码 provisioning 不受影响），`notify-debouncer-mini` 离开 CLI（`notify` 自身保留 - `serve` 和 `generate-types` watcher 直接使用它）。三者都由 `cargo-udeps` 加上覆盖 doc tests 的全源搜索确认未使用。
- **`suprnova-macros` 不再依赖 `serde` 或 `serde_derive_internals`。** 两者都没有被使用：宏输出的 `::serde::Serialize` 路径会在下游 crate 中解析，而不是在宏 crate 自身中解析。对生成的代码没有影响。
- **`MergeStrategy` 的 `match_on` 现在可以携带多个字段名。** `Append`、`Prepend` 和 `Deep` 都从 `match_on: Option<String>` 扩展为 `match_on: Option<Vec<String>>`，因此 `InertiaResponse::merge_with` / `merge_lazy_with` 可以像 `.prop(key, Prop::eager(v).match_on([...]))` 一样按多个字段去重 - 在此之前，response-builder 快捷方式严格不如直接构建 `Prop` 灵活。参见升级。
- **Scroll prop 现在发出与 Laravel 相同的 `reset` 和 merge 语义。** `scrollProps[key].reset` 只有在客户端通过 `X-Inertia-Reset` 点名 `key` 时才是 `true`，与 Laravel 的 `resolveScrollProps` 一致 - 而不是像以前那样，在没有 `X-Inertia-Infinite-Scroll-Merge-Intent` 请求头的每次 visit 上都为 `true`。scroll prop 现在也会无条件携带 merge metadata，默认为 append：一次全新 visit（完全没有请求头）会输出 `reset: false` 加 `mergeProps` 条目，而以前会输出 `reset: true` 且不带 merge metadata。`X-Inertia-Reset` 中的键会从该响应的 `mergeProps` / `prependProps` 中排除，与普通 merge prop 的既有排除规则相同。
- **`ssr:check` 现在会验证 SSR worker 的 `GET /health` 路由回答 2xx**，而不只是确认某个东西接受了 TCP 连接。每个 `@inertiajs/{vue3,react,svelte}/server` worker 都会开箱回答 `/health`，因此 worker 侧不需要改动 - 与 Laravel 的 `Inertia\Ssr\HttpGateway::isHealthy()` 一致。
- **Inertia `errors` prop 现在每个字段携带一个字符串，而不是数组。** session-flashed validation bag 会渲染为 `{ email: "The email field is required." }`，而不是 `{ email: ["The email field is required."] }`，与 Laravel 默认值和 Inertia 自身的 `ErrorValue = string` 一致。`InertiaConfig::with_all_errors(true)` 会恢复数组形状。处理程序自己设置的 `errors` prop 会原样传递，session flash（`Redirect::with_errors`、`session.pull_errors_flash()`）仍存储数组 - 只有渲染的页面 prop 改变。
- **`Model::TOUCHES` 从 inherent const 移到了 `EloquentModel`。** parent-touch cascade 位于 `Model` trait default 上，而 trait default 无法读取 inherent const。`Comment::TOUCHES` 仍然可解析 - 现在需要在作用域中 `use suprnova::EloquentModel;`。没有 `touches` 属性的模型会获得 trait 的空默认值。
- **`RelationEntry` 增加了 `related_updated_at_column`。** 手工构造 `RelationEntry` 的任何代码都需要这个额外字段；树内没有这样的代码，宏会生成全部字段。
- **`Router::view` 现在拒绝不是 JSON object 的 props。** 它此前会静默忽略这些值，注册一个渲染空 prop bag 且没有诊断的路由。`null` 仍被接受为“没有 props”；`Router::try_inertia` 是可失败的形式。
- **Inertia asset version 现在默认为 Vite build manifest 的 hash**，而不是字面量 `"1.0"`，因此部署会让长期连接的客户端失效，而无需有人记得递增字符串。`InertiaConfig::manifest_path(...)` 会用它重新指向 resolver；显式 `.version(...)` / `.version_with(...)` 仍然优先。磁盘上没有 manifest 时（本地开发），版本会回退到 `"1.0"`，也就是此前每个应用看到的值，所以在构建之前一切不变。新的 `VersionResolver::from_manifest(path)` 会直接暴露该 resolver。

### 已弃用

- **`Cookie::read_encrypted` 现在是仅 v1 的遗留 reader。** 使用 `Cookie::encrypted` 铸造并用 `read_encrypted` 读取的代码，会在本版本写入第一个值后于运行时失败；请切换到 `read_encrypted_for(name, wire)`。无上下文的 `CryptPurpose::Cookie` 入口也被取代。两者都计划在 1.4.0 移除。

### 升级

- **Cookie 解密警告现在有两个独立维度。** `KeyOrigin::Previous(index)` 警告表示应在当前 `APP_KEY` 下重新加密该值，并且只有在 rotation tail 消失后才移除该 previous key；`AadVersion::Legacy` 警告表示应在 1.4.0 回退移除之前，通过名称绑定 API 重新签发 cookie。一个值可能同时报告两者。
- **`SESSION_COOKIE_PREFIX` 是选择启用的。** 只有在 HTTPS、`SESSION_SECURE=true`、`SESSION_PATH=/` 且没有 `SESSION_DOMAIN` 时才部署 `__Host-`；本地 HTTP 脚手架让它为空。`CsrfMiddleware` 的 `with_session_config` 保留字面量 `XSRF-TOKEN` 名称；当客户端使用那个独立名称配置时，请使用 `.xsrf_cookie_name("__Host-XSRF-TOKEN")`。
- **`DecryptOrigin` 现在是一个双轴 `#[non_exhaustive]` 结构体。** 独立读取其 `key` 和 `aad` 字段，并为 `KeyOrigin` / `AadVersion` enum 保留兼容 wildcard 的匹配策略。
- **`SessionConfig` 和 `CookieOptions` 现在是 `#[non_exhaustive]`。** 应用代码中的结构体字面量和函数式 record 更新必须改为 `Type::default()`，再进行公开字段赋值或调用 builder 方法。

- **`FrameworkError` 现在是 `#[non_exhaustive]`。** 您自己代码中对它的 `match` 需要 wildcard arm。这是加入变体仍会构成 breaking change 的最后一个版本。
- **`MergeStrategy::Append`/`Prepend`/`Deep` 的 `match_on` 字段现在是 `Option<Vec<String>>`，不再是 `Option<String>`。** 直接构造结构体字面量形式的调用点 - `MergeStrategy::Append { match_on: Some("id".into()) }` - 将不再编译；请将字段名包进一个 `Vec`：`Some(vec!["id".into()])`。`match_on: None` 不受影响，无需修改。
- **匹配的 partial reload 不再输出 `deferredProps`。** 从 partial-reload 响应读取 `page.deferredProps` 的代码 - 自定义 deferred-loading component、测试快照或端到端断言 - 现在会发现该键不存在，而不再列出请求未点名的 deferred prop。请从初始的（非 partial）visit 中读取公告；Laravel 将公告放在那里，官方客户端也在那里读取。
- **裸 `except` 条目现在会丢弃其下的点号 prop 键。** `X-Inertia-Partial-Except: auth` 以前会留下注册在 `auth.user` 下的 prop，因为 gate 比较的是完整键；现在它会被丢弃。如果页面依赖裸 `except` 只裁剪精确键，请改为指名精确键（`except: ['auth.user']`），或改用点号路径 narrowing。
- **`errors` 忽略 `only`/`except`。** 过滤掉处理程序提供的 `.with("errors", …)` prop，或用点号条目缩小它的 partial reload，现在会完整发送它。需要在 partial reload 中有意排除这个包的测试，应更新为显式标志它 - 使用 `.prop("errors", Prop::eager(…).optional())`，而不是依赖 partial-reload 列表。
- **`Prop::resolve_with_owner` 也会 gate 带标志的 prop。** 它此前会解析任何不是 `Prop::is_lazy()` 的 prop - eager value 或携带 flag 的 resolver - 而不咨询 include set。现在它会 gate 每一个带 resolver 的 prop，只有已经 materialized 的值才不经过 gate。因此 `#[data(lazy(deferred))]` 字段需要请求中的 `?include=<field>` 才会解析或公告，与其他每种 lazy 形态相同。将字段加入请求的 `?include=` 列表，或者如果它本来就不应选择启用，则删除 `lazy(...)` 属性。
- **Scroll prop 的 `reset` 不再跟随 merge-intent 请求头。** 直接读取 `page.scrollProps[key].reset` 的代码 - 自定义 infinite-scroll component 或测试快照 - 在普通 revisit 上会看到 `reset: false`（以及 `mergeProps` 条目），而此前会看到 `reset: true` 且没有 merge metadata。官方 `<InfiniteScroll>` component 只在普通 revisit 上表现不同：它会在每个 `router` `success` 事件上监听 `reset`，而不仅是显式 `router.reload()`，所以普通 revisit 不会再清除已累积状态，除非 server 真正通过 `X-Inertia-Reset` 点名该键，与 Laravel 一致。在依赖旧的“任何非 append/prepend visit 都会 reset”行为的地方，请显式发送 `X-Inertia-Reset: <key>`。
- **`Prop::match_on` 接收 `impl MatchOnFields`，不再接收 `impl Into<String>`。** 新 bound 使一次调用可以命名多个字段（`match_on(["id", "slug"])`），其 impl 列表刻意保持封闭 - 只包含 `&str`、`String`、`[T; N]` 和 `Vec<T>`。没有覆盖 `IntoIterator` 的 blanket impl：coherence 会拒绝它与 `&str` 和 `String` 的实现，因为没有任何东西阻止这些类型以后获得 `IntoIterator` 实现。以前能编译的三个参数类型现在不行：`&String`、`Cow<'_, str>` 和 `Box<str>`。请在调用点传入 `&str` - 对 `&String` 使用 `match_on(name.as_str())`，对 `Cow<'_, str>` 使用 `match_on(name.as_ref())`，对 `Box<str>` 使用 `match_on(&*name)`。
- **点号 `only`/`except` 条目现在会缩小顶层 prop，而不是完全排除它。** 在此修复之前，`X-Inertia-Partial-Data: user.name` 会让 `should_include_eager` 查找精确匹配的 `"user"` 条目，找不到后静默丢弃整个 `user` prop - 请求一个字段的客户端什么也得不到。现在任何碰巧依赖这个缺口（把带点号的 `router.reload({ only: [...] })` 当成省略该键）的前端页面组件都会收到 `{ user: { name: ... } }`。无需修改代码 - 这是 Inertia v3 协议已经规定的请求/响应契约。相同修复也应用于 `should_include_optional`，并且其运行影响更大：一个点号 `only` 条目（`permissions.read`）现在算作对 `Optional` 或 `Defer` prop 顶层键的显式请求，而此前必须使用裸条目（`permissions`）才会触发。过去完全跳过该 prop resolver 的请求现在会运行它 - 如果 resolver 命中数据库或外部服务，已经发送点号 partial-reload 请求的客户端会开始在以前不做这项工作的请求上执行它。若应用有带点号 partial-reload 流量的 `Optional`/`Defer` prop，请在升级后关注 resolver 调用量。
- **`InertiaSharedData::share` 现在接收页面组件名称。** 在 `req` 后增加一个 `component: &str` 参数：
  ```diff
  -async fn share(&self, req: &dyn InertiaRequestExt) -> Result<IndexMap<String, Prop>, FrameworkError>
  +async fn share(&self, req: &dyn InertiaRequestExt, component: &str) -> Result<IndexMap<String, Prop>, FrameworkError>
  ```

  如果 provider 不需要按页面变化，请忽略它（`_component`） - Laravel 的 `RenderContext` 会为 `ProvidesInertiaProperties::toInertiaProperties` 携带同样的 `(component, request)` 配对。
- **`Prop` 是结构体，不是 enum。** 它的变体已移除；通过方法构建和读取 prop：
  - `Prop::Eager(v)` -> `Prop::eager(v)`
  - `Prop::EagerNone` -> `Prop::absent()`
  - `Prop::Always(v)` -> `Prop::eager(v).always()`
  - `Prop::Lazy(r)` -> `Prop::from_resolver(r)`（`Prop::lazy(closure)` 不变）
  - `Prop::Optional(r)` -> `Prop::from_resolver(r).optional()`
  - `match prop { Prop::Eager(v) => … }` -> `prop.as_value()`
  - `matches!(prop, Prop::Lazy(_))` -> `prop.is_lazy()`；`matches!(prop, Prop::EagerNone)` -> `prop.is_absent()`
  `DeferConfig`、`MergeConfig`、`OnceConfig` 和 `ScrollConfig` payload 结构体已移除；它们的字段现在是 `Prop` 上的标志。`Prop::is_deferred()` 改名为 `Prop::has_resolver()`，这才是它一直代表的含义。`DeferOptions`、`OnceOptions`、`MergeStrategy`、`ScrollMetadata` 以及每一个 `InertiaResponse` 构建器方法都不变，因此只使用 response builder 的应用无需编辑。手工构建 prop 的应用 - 通常是 `InertiaSharedData` 实现 - 需要上述重命名。

- **这次修复保护已有的会话，而不仅是此后的请求。** 只升级就足够：早期版本写出的 session cookie 可能携带从未清理的 `_previous.url`，而 `SessionData::previous_url()` 现在会在该 session 升级后第一次使用时丢弃它，不会因为它已被存储就信任它。无需使现有 session 失效、迁移 session 表或强制重新登录。路径形如协议相对（`//host`）的请求以后也不会更新记录的 previous URL - 如果应用的 `fallback!` 路由（或一条能在异常路径上回答 200 的其他路由）曾经合法依赖这个路径成为 `Redirect::back()` 目标，那么它不再会如此。无论怎样，session 中原本安全的值会保留（或者如果从未记录过安全值，则 `Redirect::back(fallback)` 自己的 fallback 获胜）。除非您依赖这个修复已经关闭的精确边界情况，否则无需更改代码；而那本来就是一次开放重定向风险。
- **从页面中每一个 `errors.<field>` 绑定移除 `[0]`。** 新的默认形状中，`errors.email` 是字符串，因此 `errors.email[0]` 会渲染它的第一个字符，而不是消息。同时将 TypeScript 类型从 `string[]` 改为 `string`。如果不想修改页面，请在传给 `Inertia::install` 的配置上设置 `InertiaConfig::with_all_errors(true)`，并为 `@inertiajs/core` 添加 `errorValueType: string[]` 模块扩展。起始前端已经提供新形状。
- **手工编写验证失败返回重定向的处理程序现在可能删除它。** 现在桥接是自动的；仍然自行重定向的处理程序继续工作，因为中间件只会处理携带非空 `errors` 对象的 `422`。
- **崩溃的 `suprnova serve` 子进程现在会重新生成，而不是结束会话。** 如果依赖崩溃直接停止 `suprnova serve`（CI smoke check，或把退出视为“出错了”的脚本），请传递 `--no-restart` 以精确恢复该行为。默认也会限制重试：连续崩溃 5 次的进程不再重试（用 `--restart-tries` 提高上限，或用 `--no-restart` 恢复原本一次崩溃即结束的行为）。
- **`Model::TOUCHES` 不再是 inherent const。** 直接读取 `Comment::TOUCHES` 的代码需要让 `use suprnova::EloquentModel;`（或 `suprnova::eloquent::EloquentModel`）进入作用域 - const 被移到那里，以便 parent-touch cascade（一个 `Model` trait default）能够读取它。对应用做 `grep -rn TOUCHES` 就能找到每个调用点；多数应用没有，因为该 const 以前在运行时什么也不做。
- **`RelationEntry` 增加了一个字段。** 只有手工构造 `RelationEntry` 的代码需要修改 - 向字面量增加 `related_updated_at_column`。框架提供的宏生成 relation registration 时已经发出它，因此普通应用只通过 `#[suprnova::model]` 声明关系不会受影响。
- **带非对象 props 的 `Router::view` 现在会在启动时 panic。** 它此前会静默注册一个空 prop bag；`view` 委托给 `Router::inertia`，后者要求 object（或 `null`），否则会 panic。如果 `view` 调用可能携带非对象 props，请切换到 `Router::try_inertia` 并处理 `Err`；除此之外无需改变。
- **Inertia version manifest 默认值现在会在 build 存在的瞬间改变版本字符串。** 将 `X-Inertia-Version: 1.0` 硬编码的应用或测试，只能一直工作到磁盘上出现 Vite manifest；一旦出现，版本会变成 manifest hash。如果需要旧的常量，请自己从 `VersionResolver::from_manifest(path)` 读取，或显式固定 `.version(...)`。预期升级后的第一次部署会迫使已连接客户端经历一次完整页面 reload cycle - 只发生一次，这正是改动的目的。无 manifest 回退值导出为 `suprnova::MANIFEST_VERSION_FALLBACK`，因此无需再次硬编码 `"1.0"`。
- **将 `Inertia::install` 和 `global_middleware!` 注册移出 `bootstrap::register`。** 将它们放进一个新函数，并改为把该函数传给 `.http_bootstrap(...)` - scaffold 的新形状是一个同步的 `register_http_stack()`，以 `.http_bootstrap(|| async { bootstrap::register_http_stack() })` 调用。跳过这一步的应用保留今天的行为，包括缺少前端 manifest 时 worker 启动失败。

## 1.2.4 - 2026-08-18

### 安全

- **维护模式的绕过密钥现在以常数时间比较。** `MaintenanceMiddleware` 此前用普通的字符串比较来匹配这个密钥 URL，而普通比较会在第一个不同的字节处返回。由于这个密钥是一个随请求路径携带的 bearer 凭据，这个耗时差异会告诉攻击者，他们已经猜对了多长的前缀。这次比较现在会通过 `subtle::ConstantTimeEq` 跑完完整的字节长度，只在长度不匹配时短路 - 与它旁边那个绕过 cookie 的比较是同一个形状。

- **`rules::Url` 现在会拒绝脚本 URI。** 这条规则此前接受任何 `url::Url` 能解析的协议方案，`javascript:` 和 `vbscript:` 也在其中，所以一个通过了验证的 URL，被渲染进一个 `href` 之后仍然可能是一个脚本执行的落点。它现在采用 Laravel 的 `url` 规则形状（`Illuminate\Support\Str::isUrl` 的 `^(PROTOCOLS)://HOST` 模式）：协议方案必须在 Laravel 的允许列表上、必须后跟 `://`，**并且**后面必须跟一个非空的主机 - Laravel 的主机分组没有 `?`，所以即使协议方案在列表上，一个缺失或为空的主机也永远不会匹配。协议方案列表以及“`://` 加主机”这条要求都逐字取自 Laravel；主机本身由 `url` crate 解析，而不是由 Laravel 的正则解析，所以少数几个边界情况仍然不同 - 一个超出范围的端口在这里被拒绝，在那边则被接受，IDN 主机的归一化方式也不一样。新的 `Url::protocols(&[...])` 对应 Laravel 的 `url:http,https`；`HttpUrl` 现在就是它的字面语法糖，并保留自己的消息。**行为变更：**一个协议方案不在列表上、此前能通过验证的 URL 现在会失败 - 如果您本来就打算接受它，请用 `Url::protocols(&["myapp"])` 点名这个协议方案。另有两处行为变更：`mailto:`、`data:` 和 `tel:` 按名字在 Laravel 的允许列表上，但不携带 authority 组成部分，所以它们现在会失败；而 `file:///etc/passwd` 这类路径 - `scheme://` 后面最后两个斜杠之间什么都没有 - 现在同样会失败，因为空字符串也不是一个主机。两者都是从 Laravel 自己那条“`://` 加主机”的规则推出来的。

- **Inertia 响应现在处处都会声明 `Vary: X-Inertia`。** 这个响应头此前只设置在页面对象响应本身上。重定向、404、422 和静态响应都不带它，所以一个仅以 URL 为键的共享缓存，可能会把 JSON 页面对象提供给一次硬性的浏览器导航，或者把 HTML 外壳提供给一次 Inertia XHR。新的 `InertiaHeadersMiddleware` - 由 `Inertia::install` 注册为三者中最外层的那个 - 会在每一个响应上设置它，并且会把一次 Inertia 访问上的空 `200` 变成一个 `303` 回跳，而不是一个被客户端当作非 Inertia 而拒绝的响应。`InertiaVersionMiddleware` 现在会在它的 `409` 之前重新 flash 会话，所以一条被 flash 进去的错误消息，能挺过客户端随后那次整页 GET。

- **三处 Inertia 响应修复。** `InertiaResponse::location_for(&req, url)` 对一次 Inertia XHR 返回 `409` + `X-Inertia-Location`，对一次硬性导航则返回一个普通的 `302` + `Location`，所以一次在 SPA 之外发起的 OAuth 或 SSO 弹回，不再会死在一个没有响应体的 `409` 上。既有的 `location(url)` 保持它始终为 `409` 的形状。新的 `App::clear_history()` 会把清除历史记录的标志 flash 进会话，让它挺过登出重定向，落到那个真正会被渲染的页面上 - 而逐响应的 `.clear_history()` 只标记了那个被浏览器丢掉的重定向，于是上一个会话的加密历史记录仍然可以被解密。另外，一个 `once` prop 现在只在一次完整的 Inertia 访问上才会被跳过：一次显式的 `router.reload({ only: ['stats'] })` 会重新解析它，而不是什么都不返回。

- **SES 传输现在会发送自定义的消息头。** 在 `MAIL_DRIVER=ses` 之下，`Mail::to(..).header("List-Unsubscribe", ...)` 和 `Mailable::headers()` 此前会被静默丢弃：`Content.Simple` 请求体里没有 `Headers` 字段，而那个原始 MIME 构建器从来没有读过 `OutgoingMessage::headers`，尽管其他每一个传输都会转发它们。SES 的两条路径现在都会携带它们 - `Headers` 采用 SES v2 的 `{Name, Value}` 列表形式，原始 MIME 则写成真正的请求头行 - 所以退订链接、会话串联请求头和路由提示都能挺过一次驱动程序切换。请求头名字在两条路径上都会被提前校验 - CR、LF 和 NUL（注入用的那几个字节，Mailgun 传输早已拒绝它们），以及任何不是合法 RFC 5322 字段名的东西（空格、冒号、非 ASCII 字符） - 所以附上一个文件永远不会改变一封消息会不会被接受。

### 修复


- **嵌套的验证失败现在会到达 422 响应体。** 嵌套结构体上的、或者被验证的 `Vec<T>` 中某个元素上的 `#[validate(nested)]` 失败，此前会在验证器和响应之间丢失：请求确实被正确地以 422 拒绝了，但 `errors` 映射回来是空的，所以没有任何消息被渲染出来，客户端也没法分辨是哪个字段出了问题。嵌套的失败现在会和顶层的那些一起，被展平成 Laravel 的点分记法 - `address.street`、`items.1.name`、`order.items.2.sku`。

- **Inertia 页面对象的 `url` 现在保留查询字符串。** `page.url` 此前只有请求路径，所以对 `/users?page=2&sort=name` 的一次访问，客户端记录下来的是 `/users`。此后每一次前进/后退导航、每一次 `router.reload()`，都会在丢掉分页游标、排序和过滤条件的情况下重放这个页面。它现在是路径加查询 - 和 `InertiaVersionMiddleware` 早已用于 `X-Inertia-Location` 的推导方式相同，所以默认情况下两者逐字节一致。新的 `InertiaConfig::url_resolver(...)` 可以覆盖*页面对象*怎样给这个页面命名（Laravel 的 `Inertia::resolveUrlUsing`）；版本弹回仍然点名那个到达的 URL，因为那才是浏览器必须去获取的 URL。

- **`Inertia::install` 现在会把它的配置应用到每一个响应上。** 交给 `Inertia::install` 的那份配置此前只被读了三个字段，然后就被丢弃了，所以每一个没有显式 `.with_config(...)` 构建出来的 `InertiaResponse`，渲染时用的都是 `InertiaConfig::default()`。一个用 `--frontend react` 脚手架出来的应用，除非环境里设置了 `SUPRNOVA_FRONTEND`，否则提供的是 Svelte 的入口点，而且没有 React 的 refresh 前导脚本；在这份配置上启用的 SSR 从来到不了任何响应；页面对象的资产版本，也来自一份与版本中间件的解析器不同的配置。这份被安装的配置现在会保留在容器的 Inertia 注册表里，并且正是 `InertiaResponse::new` 的起点。逐响应的 `.with_config(...)` 仍然会覆盖它，从不调用 `Inertia::install` 的应用不受影响，而一次失败（失败即关闭）的安装什么都不会保留。附带的一个效果是，生产环境的 Vite 清单现在每个进程解析一次，而不是每个响应解析一次。

- **脚手架出来的应用现在会安装 Inertia 的协议中间件。** `suprnova new` 写出来的 `bootstrap.rs` 注册了会话、语言区域、CSRF 和 include 这几个中间件，却从来没有调用 `Inertia::install`，所以一个生成出来的应用既没有 `InertiaVersionMiddleware` 也没有 `Inertia303Middleware`：一个仍然跑着上一份 bundle 的浏览器，在部署之后从来不会被告知去重新加载；而一个做了重定向的 `PUT`/`PATCH`/`DELETE` 会停在一个 `302` 上，客户端可能带着原来的动词去追随它。这次调用现在落在 `SessionMiddleware` 之后 - 版本中间件的会话重新 flash 正是在那里才起作用 - 并带着一个具名的 `INERTIA_VERSION` 常量，供资产变化时递增；它还会钉住这个项目生成时所用的前端（`--frontend react` 对应 `.frontend(Frontend::React)`），这样 HTML 外壳加载的就是那个框架的 Vite 入口点，而不是回退到 Svelte 的那个。生成出来的 `.env` 现在也会相应地设置 `SUPRNOVA_FRONTEND`。`--api` 起始套件不受影响；它没有前端。

- **`Queue::push_unique` 不再把一个已入队的作业报告为被跳过。** 它的返回值此前是用 `matches!(outcome, Idempotent::Fresh(()))` 算出来的，这会把 `Idempotent::FreshUnfenced` 折叠成 `false` - 而那正是信封*确实*被推送了、但去重租约在推送途中丢失的那个结果。根据这个布尔值分支的调用方，会被告知一个即将运行的作业已经作为重复项被压制了。三个结果现在都会被穷尽匹配：租约丢失返回 `true`，并附带一条点名这个作业和它的唯一键的 `warn`，只有真正的重复项才返回 `false`。`push_unique_later` 和 `later_unique` 共用这条路径，也随之被修复。

### 变更

- **对等基线已挪到 Laravel 13.25.0。** 13.23.0、13.24.0 和 13.25.0 的发布说明被逐条追溯到了框架自己的接口上。每一件触及了 Suprnova 代码路径的事情，要么已经在这个版本里修复，要么在 [`manual/parity.md`](parity.md) 里有一行标着 `not yet` 或 `by design no`。

### 升级

有两处变更，可以在您这边不改任何代码的情况下改变一个正在运行的应用。

- **您传给 `Inertia::install` 的那份配置上的设置，现在会生效了。** 它们此前只被读了三个字段，然后就被丢弃了。如果您的安装配置设置了 `.ssr(...)`，那么 SSR 现在是开着的：请在部署之前启动那个工作进程（`suprnova ssr:start`），或者去掉这次 `.ssr(...)` 调用。在那里设置的 `.entry_point`、`.assets_base_url`、`.default_title` 和 `.encrypt_history(...)` 现在也会到达页面。

- **`rules::Url` 拒绝得更多了。** 此前能通过、现在不再能通过的值有：任何在 Laravel 允许列表之外的协议方案，`javascript:` 和 `vbscript:` 都在其中；`mailto:`、`data:` 和 `tel:`，它们在允许列表上，但不携带 `://` 主机；以及主机为空的 `scheme://`，例如 `file:///path`。如果您本来就打算接受某个协议方案，请点名它：`Url::protocols(&["myapp"])`。

## 1.2.3 - 2026-08-16

### 修复

- **日期时间转换现在可以读取数据库原生的`CURRENT_TIMESTAMP`文本。** `AsDateTime`、`AsImmutableDateTime`和`AsOptionalDateTime`仍会写入规范的RFC-3339；读取时也接受带时区的PostgreSQL文本以及不带时区的SQLite/MySQL值。不带时区的值按UTC解释。

## 1.2.2 - 2026-08-14

### 修复

- **在 PostgreSQL 上，所有基于属性的写入现在都能正确处理可为空的非文本值。** 类型化的 `Builder::update_all` 和 `Builder::upsert`、无模型的 `DB::table().insert/update`，以及多对多中间表的额外属性，会将显式 JSON 空值作为 SQL `NULL` 发出，同时继续绑定每一个非空值。这样会保留目标列的类型，而不是发送被标为文本类型的空参数；PostgreSQL 会拒绝将这种参数用于 bigint、integer、boolean、timestamp 和其他非文本列。多行 upsert 现在也会拒绝缺少或多出的列，而不会悄悄把形状错误的行转换为空值。多对多中间表的自动时间戳会以类型化 UTC 日期时间而非文本的形式绑定。

### 安全

- **发布门现在会在整个 workspace 中区分休眠的 lockfile 元数据与已编译的依赖项。** Cargo 会在 `Cargo.lock` 中记录 rust_decimal 未使用的可选 rkyv 0.7 兼容依赖；该门现在会证明，从任何 workspace 成员、feature、target 或依赖边都无法到达 rkyv 及其 derive crate。对应的 RustSec 例外由项目负责，期限至 2026-11-14，并且必须在 rust_decimal 不再记录这个遗留可选依赖时移除。

## 1.2.1 - 2026-08-09

### 变更

- **Suprnova 已从 GitHub 的 `entrepeneur4lyf` 组织迁移到 `eas4ai`。** 软件包元数据、文档、依赖示例和 scaffold 模板中的仓库 URL 现在使用 `github.com/eas4ai`。新项目也使用受监控的作者邮箱 `shawn@eas4ai.com`。此版本没有改变任何运行时行为。

## 1.2.0 - 2026-08-05

### 新增

- **手册现以七种语言发布。** `manual/es/`、`manual/fr/`、`manual/de/`、
  `manual/pt-BR/`、`manual/ja/` 和 `manual/zh-Hans/` 各自收录了完整的
  104 章手册 - 每一章、目录以及这份更新日志 - 均译自英文源文本。英文仍然是规范版本: 章节结构、代码块、标识符、CLI 命令和环境变量与源文本保持逐字节一致，因此译文章节在框架行为的表述上永远不可能与英文相左 - 它只是用读者的语言来讲述。

  这些翻译是为 suprnova.app 制作并评审的，该站点将本手册渲染为其 `/docs`。每个小节在那里都有一份评审台账: 裁定针对英文与译文双方的内容哈希记录，一个小节要计为已通过，必须有两位独立评审者对完全相同的字节予以通过；各语言的术语表则固定了术语裁定（哪些术语保留英文、哪些采用本族语词，以及理由）。欢迎在任一仓库提交更正 - 在这里的修复会在下一次同步时到达站点。

## 1.1.0 - 2026-08-02

### 新增

- **逐语言区域的回退链。** `LocalizationConfig` 新增了 `parents` 字段（`APP_LOCALE_PARENTS`，逗号分隔的 `child=parent` 对，或者可链式调用的 `.parent(child, parent)` 构建器）：一个语言区域可以先继承一个已配置的同级语言区域，再进一步回退到全局的 `fallback_locale` - `pt-PT` 继承自 `pt-BR`，`en-AU` 继承自 `en-GB`，依此类推，且具有传递性。`Lang::get`/`try_get`/`get_with`/`try_get_with`/`has` 全都会沿着这条链走，当前语言区域优先，所以这对任何 `Translator` 驱动程序都有效，不只是内置的那个。一个格式错误的配对、一个无效的语言区域、一个被命名两次的子项，或者一个环（包括一个语言区域把自己列为自己的父级），都会在配置加载时明确地失败，而不是在请求时才劣化。

  已提供的语料表会提前按链展平：`FluentTranslator` 现在会把每个语言区域的 `/_suprnova/lang/<locale>.ftl` 语料表构建成一次折叠 - 底部是给 `en`/`en-*` 语言区域用的内嵌框架语料表，然后是该语言区域已配置的父级链，最后才是它自己的 `*.ftl` 文件 - 所以一个链式语言区域仍然是浏览器只需获取一次的单个自包含文件，不需要客户端感知这条链。展平只覆盖已配置的父级；末端的 `fallback_locale` 仍然只是 `Lang` 门面层面的回退，不会被烘焙进已提供的字节里。

  这让增量式的语料表变得可行：一个 `lang/pt-PT/` 目录可以只保存真正与 `lang/pt-BR/` 不同的那少数几个字符串，而不必是一份完整的重复语料表。让这一切成为可能的合并，是在 Fluent AST 层面进行的 - 子项的值会替换父项的值，属性按名字合并（一个没有提到某个属性的覆盖不会再丢失那个属性），选择表达式整体替换（CLDR 复数类别是与语言区域相关的，所以逐变体合并并不连贯），子项独有的条目则会追加进去。完整的契约参见 `manual/localization.md` 新增的“回退链”小节。

### 变更

- **`LocalizationConfig` 新增了 `parents` 字段。** `from_env()` 和这个构建器不受影响；手写的结构体字面量构造（测试里手动构建一个 `LocalizationConfig`）需要多写一个字段。
- **已提供的语料表文本现在对每个语言区域都做了序列化器归一化**，并且同一语言区域内的多文件合并（一个语言区域目录里有好几个 `.ftl` 文件）现在会走和父级链一样的 AST 层面合并，而不是简单的 bundle 覆盖。已解析出来的翻译结果保持不变，除了下面这两处严格意义上的改进；不管怎样，底层字节都会发生变化 - `ETag`/`?v=<hash>` 会在升级时轮换一次。这两处改进是：一个覆盖不再会静默丢弃它没有提到的那些属性，一个仅覆盖属性的条目不再会剥离消息本身的值（此前这要么是一个错误，要么会解析成一次回退；现在它会解析成更早那次覆盖的值）。

## 1.0.0 - 2026-08-02

### 新增

- **本地化。** `lang/<locale>/*.ftl` 里的消息语料表（[Fluent](https://projectfluent.org)）、一个带 `__!("key", name: value)` 宏的 `Lang` 门面、逐请求的语言区域检测（`LocaleMiddleware`：会话 → cookie → `Accept-Language` → `APP_LOCALE`），以及基于 ICU4X、能感知语言区域的数字、货币、日期、时间、列表和相对时间格式化。这一章是 `manual/localization.md`。

  内置的验证规则不再硬编码英文。每一条规则返回一个带键的消息（`validation-min` 加上它的参数和一个英文回退），只在序列化边界处翻译一次 - 所以一个西班牙语应用只需要放进 `lang/es/validation.ftl`，就能得到西班牙语的验证错误，不需要包装任何规则，也不需要为框架的消息 fork 一份副本。字段名通过一次 `field-<name>` 查找来人性化。`Rule::passes`（以及 `ContextualRule` / `AsyncRule`）现在返回 `Result<(), ValidationMessage>`；一个自定义规则里的 `Err("…".into())` 主体仍然能编译、仍然会原样渲染，但您 `impl` 里的签名需要改成这个新类型。

  浏览器拿到的，是和服务器解析出来的完全一样的字节：合并后的语料表以 `/_suprnova/lang/<locale>.ftl` 提供，带着一个 ETag 和一个不可变的 `?v=<hash>` 形式，三个起始套件都用 `@fluent/bundle` 解析它，`suprnova generate-types` 会产出一个 `MessageKey` 联合类型，这样重命名一条消息就会让 TypeScript 编译器指向每一个调用点。

  之所以用 Fluent 而不是 Laravel 风格的 PHP 数组，是因为同一种格式必须同时服务服务器和浏览器，也因为让俄语、波兰语和阿拉伯语正确的，正是 CLDR 复数类别 - `trans_choice` 的整数区间做不到这一点，这也是这里没有 `trans_choice` 的原因。位于一个默认开启的 `localization` feature 之后；`--no-default-features` 仍然能编译、仍然会做验证，使用内嵌的英文回退。

- **`Paginator` 的 `IntoInertiaScroll`。** 这个 trait 此前给 `LengthAwarePaginator` 和 `CursorPaginator` 都实现了，唯独没给简单分页器实现，所以 `simple_paginate` 的结果完全没法喂给 `Inertia::paginate` - 尽管 `simple.rs` 自己的模块文档还把它指为 URL 生成路径。这让偏移分页的 Inertia 集合只能在“每个请求一次 `COUNT(*)`”和“手写滚动元数据”之间二选一。`next_page` 来自 `LIMIT n+1` 的溢出探测，而不是一个算出来的末页，因为根本没有总数可供计算。

### 修复

- **`suprnova generate-types` 每次运行都会产出不同的文件。** 拓扑排序通过遍历一个 `HashMap` 来给它的工作队列播种，而 Rust 会按进程随机化哈希遍历顺序，所以连续几次运行会把同样的一批接口排出不同的顺序。这份输出是一个提交进版本库的产物，所以每次运行都会产生一个 diff - 而一个无缘无故就反复变动的生成文件，会让人们不再重新生成它，此后它就会悄悄地不再描述它自称描述的那份 Rust 代码。目录遍历现在也排序了，所以输出不再依赖文件系统顺序。同一份源码运行两次，现在会得到字节级相同的结果。

- **`topological_sort` 做的事和它的文档注释正好相反**，把依赖方排在了被依赖方前面。这是无害的 - 一个 TypeScript 接口可以引用同一文件里稍后才声明的另一个接口 - 所以被修正的是这条注释，而不是这个顺序，因为改动顺序只会打乱一个已被跟踪的文件，却没有带来任何好处。

## 0.9.1 - 2026-08-01

三个缺陷，全都是通过在一个容器化的测试装置下运行 dogfood 应用发现的，而不是靠读代码发现的。它们每一个，对于一个从不像生产环境那样真正停掉一个进程的测试套件来说都是不可见的。

它们会按一个特定的顺序复合发生：一次滚动部署 SIGKILL 掉一个正在处理作业的工作进程（第一个缺陷），而这个作业接下来会走上一条从未计入这次尝试的重新认领路径（第二个缺陷）。

### 修复

- **`schedule:work`、`queue:work` 和 `workflow:work` 都忽略了 SIGTERM。** 三者都只在 `tokio::signal::ctrl_c()` 上做 select，而这只会安装一个 SIGINT 处理程序 - 所以进程里的任何地方都没有 SIGTERM 的处理程序，而 SIGTERM 正是 `docker stop`、Coolify、systemd 和 Kubernetes 发送的信号。三者背后都已经在那个 `select!` 之后精心写好了一段有边界的排空逻辑；但在一个监督程序之下，它从未被执行过。修复前的实测：对一个 `queue:work` 容器执行 `docker stop`，会烧光它整整 40 秒的宽限窗口，然后带着被摧毁的飞行中作业以 137 退出。作为 PID 1 - 这正是一个容器里运行的东西 - 内核会直接丢弃一个未被处理的 SIGTERM，所以这个进程不是死得难看；它根本没有死，直到 SIGKILL 出现。`Server::run` 已经正确处理了这两个信号，它的监听器现在也被共享了，这同时也关上了调度器循环里一个漏掉信号的窗口。

- **一个杀死了自己工作进程的作业，永远没法被转入死信。** 一个*处理程序*失败的作业会被 nack，它的尝试次数会被计入，所以它会在 `max_tries` 之后转入死信。而一个*杀死自己工作进程*的作业 - OOM、abort、段错误，或者上面那个 SIGKILL - 什么都不会结算；它的预留只是单纯地失效，而过去每一个驱动程序都会把它字节级原样地重新投递。这样的作业是不死的：它杀死每一个认领它的工作进程，原封不动地回来，再杀死下一个，只要还有什么东西在不断重启工作进程，这个循环就不会停。三个驱动程序现在都会在得知一个工作进程死亡时就计入这次尝试，因为切换 `QUEUE_DRIVER` 不应该改变一个毒丸作业能不能被拦下来。`attempts` 现在的含义是“投递给一个工作进程的次数”，而不是“处理程序失败的次数” - 记录在 `manual/queues.md` 里，因为一个因不相关原因而丢失的工作进程，同样会烧掉一次尝试。

- **……而这个耗尽了尝试次数的作业，现在会在被派发之前就转入死信。** 只计入这次尝试是必要的，但还不够。此前每一个死信决策都活在工作进程的结算路径里，而那条路径假定处理程序会返回 - 所以它恰恰对那些没法返回的作业从未运行过。只做驱动程序的修复时，计数器确实会往上爬（实测：三个被杀死的工作进程分别让它经历了 0 → 1 → 2），但没有任何东西会据此采取行动。现在这个预算会在处理程序运行之前就被花掉。这一点，只有在第一个修复看起来已经正确之后，重新跑一遍这个容器实验，才捕捉到。

- **守护进程完全没有 tracing 订阅者。** `serve` 会从 `init_telemetry` 拿到一个；而 `queue:work`、`schedule:work`、`schedule:run` 和 `workflow:work` 走的是另一条启动路径，什么都没拿到，所以它们发出的每一行 `tracing::` 都石沉大海，`LOG_LEVEL` 对它们来说形同虚设。而这恰恰是它们大部分要说的话 - 一个工作进程把某个作业转入死信、一个调度器跳过了一次它错过的 tick、一把它释放不掉的锁。在一个容器里，唯一可见的输出就是启动横幅，而这个进程看起来无所事事，实际上却在做这一切。这次发布里的两个缺陷，在这个问题被修好之前都是不可见的。

- **没有绑定失败作业存储时，一次死信就是一次静默删除。** 持久化这一步坐落在 `if let Some(store) = ..` 里面，所以在没有存储的情况下，这个分支根本不匹配，执行会直接落到 ack 上 - 这比它正上方的失败路径还要安静，因为那条路径至少还保留了预留。一个缺失的存储被当成了比一个坏掉的存储更成功。它现在会在 ERROR 级别记录整个信封，因为那正是 `queue:retry` 用来重新推送的东西：能靠人手恢复的工作，和已经不复存在的工作之间的差别。

- **`QUEUE_DRIVER=database` 现在会绑定一个失败作业存储。** `failed_jobs` 是这个驱动程序契约的一部分 - `queue:retry` 会读它，`Queue::retry_failed` 离了它没法工作 - 但 `bootstrap_from_env` 接上了驱动程序，却把存储留成了未设置，所以一个数据库支持的队列，除非应用手动绑定了一个，否则会把死信转进虚无。可以通过 `QUEUE_FAILED_DB_TABLE` 配置。只有这个驱动程序需要它：`memory` 天生就是短暂的，而 `redis` 根本没有表可写。

- **Redis 的重新认领延迟现在跟随 `--visibility-timeout`。** 这个标志设置的是 XAUTOCLAIM 的空闲阈值，但另有一个独立的时钟决定消费者多久看一次，而驱动程序把它留在了 sea-streamer 的 30 秒默认值上 - 所以 `--visibility-timeout 5` 实际的意思是“最多 35 秒”。这个间隔现在会跟踪已配置的超时，并被夹在 1 秒到 30 秒之间，这样一个很短的超时就没法变成一场 XAUTOCLAIM 风暴，而一个很长的超时也只会让重新认领比以前更快，不会更慢。

### 新增

- **`TaskBuilder::on_one_server()` / `on_one_server_for(ttl)`** - 让一个计划任务在多个副本之间、每个到期 tick 恰好只运行一次。没有它，就没有任何东西会为一个 tick 选出一个领导者：每个 `schedule:work` 进程都会独立地评估这份计划，实测三个副本会把每一个到期任务每分钟都跑三次，分毫不差。一个跑在三个副本上的夜间账单作业，会把每一位客户都扣三次款。

  `without_overlapping()` 覆盖不了这种情况，也没法覆盖：它的锁以任务为键，并在处理程序返回时释放，所以一个很快的任务会在第二个副本查看之前就把锁腾出来了。`on_one_server` 同时以任务*和这次 tick* 为键，并且会把锁一直持有到处理程序返回之后，让它靠 TTL 过期。这两者可以组合使用。

  这是可选启用的，与 Laravel 一致。但在失败关闭这一点上偏离了 Laravel：这次选举的共享程度，取决于它背后的缓存有多共享，所以在 `CACHE_DRIVER=memory` 且存在一个单服务器任务的情况下，一次生产环境启动会被拒绝，并点出违规的任务名字，除非设置了 `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true` - 留给那些确实只跑一个调度器的部署。

### 变更

- `manual/deployment.md` 不再把“只运行恰好一个 `schedule:work` 进程”写成唯一的选项，并新增了一个**优雅停止**小节，讲述每个子系统各自的排空窗口、如何把一个平台的终止宽限期设置得高于这些窗口，以及为什么 PID 1 会让一个缺失的信号处理程序，比听起来还要糟糕。

## 0.9.0 - 2026-07-31

### 安全

- **认证签发此前只能按调用方节流，没法按收件人节流。** 一个以地址为键的限制，回答的是“某一个客户端是不是太吵”；它回答不了“某一个邮箱是不是正在被灌爆”这个问题。一个分散在一个僵尸网络上、或者单个 IPv6 `/64` 里的攻击者，可以待在每一个按 IP 的预算之下，同时用密码重置邮件把某一个受害者的收件箱灌满，而框架里没有任何东西能表达出本该拦住它的那个限制 - 一个键函数能读到路径、请求头和查询字符串，却读不到一个表单编码的请求体，所以恰恰在承载这个地址的那条路由上，这个地址是不可见的。

  `identity_key` 会以被操作的这个账号为键，给一个桶建键。它先读查询字符串，再读一个已缓冲的表单请求体，所以一个键函数就能覆盖这两种形状；这个值会被裁剪空白并转成小写，因为 `Alice@Example.com` 和 `alice@example.com` 送达的是同一个邮箱，而一个靠按住 shift 键就能绕开的限制，算不上限制；它还会被哈希，因为一个限流后端往往是一个共享的 Redis，访问控制比主数据库要弱。

  两个新的中间件构建器为它提供支持。`key_reads_body(cap)` 会在建键之前缓冲请求体 - 这是可选启用的，因为缓冲是一件未认证的调用方能强迫您去做的工作，超过上限的请求体会被以 413 拒绝，而不是不建键就放行。`only_when(pred)` 会对那些它根本管不着的请求，整个跳过某个限流器，这正是防止一个叠加的按收件人预算，在那些根本没有指名收件人的路由上，悄悄变成生效限制的关键。

  dogfood 应用现在会在它的签发分组上把两者叠加起来：每个地址每 5 分钟 10 次，每个收件人每 15 分钟 3 次。

一次针对 Torii 的会话、密码、OAuth 和 passkey 路径的审查，发现了八个缺陷，全都已经在这个钉住版本的 fork（`suprnova-torii-rs` `968b0be`）里修复。

- **已过期的会话可以被刷新，重新活过来。** SeaORM 会话仓储的 `refresh` 没有过期谓词，会无条件地延长 `expires_at`，而 `OpaqueSessionProvider::refresh_session` 跳过了 `get_session` 会执行的那个 `is_expired()` 检查。一个持有到过期之后的令牌，可以被无限期地续期。已在两层都修复。无法通过 Suprnova 自己的接口触达 - `Torii` 和框架都没有暴露会话刷新 - 但它是这两个 crate 的公开 API。
- **登录表单会通过计时泄露哪些账号存在。** 只要邮箱匹配不上，认证就会立刻返回，完全跳过 Argon2：实测一个未知地址是 54 微秒，而一个错误密码是 719 毫秒，差出约 13000 倍，这在网络上是可读出来的。两条失败路径现在都会对着一个哑哈希做校验，所以耗时一样。这一个*确实*能通过 Suprnova 的密码登录触达。
- **JWT 的 `iss` 声明会被写入，但从未被校验过。** 算法钉定此前就已经是正确的 - `alg: none` 和 HS/RS 混淆从来都不可能发生 - 但签发者一直只是装饰，所以两个共享同一个签名密钥的服务，会互相接受对方的会话。现在在配置了一个签发者时会强制校验。
- **一个一次性的 PKCE 校验值可以被认领两次。** 消费它的方式此前是先读后删，所以对同一个 `csrf_state` 的两次 OAuth 回调可以都先读到它，然后才有任意一个删除真正落地。现在会在一次操作里完成认领 - 在 Postgres 上是 `DELETE ... RETURNING`，在 SeaORM 上则是一次主键删除，靠受影响的行数来挑出胜者。
- **已过期的会话被列成了活跃状态。** `find_by_user_id` 没有过期过滤条件，而过期的行会一直存活到清理任务运行为止，所以一个“您已登录的设备”界面，会把已经失效的会话提供给用户去撤销，却对那个真正存活的会话只字不提。
- **一个 passkey 查找被命名成了 `authenticate`。** Torii 的 `PasskeyService::authenticate_credential` 接受一个凭据 ID，返回拥有它的用户，而 `PasskeyAuth::authenticate` 会据此铸造一个会话。Torii 存的是 passkey - 它不带任何 WebAuthn 依赖，也没法校验一个断言，所以这些调用能证明的唯一一件事，就是调用方知道一个凭据 ID：这是一个浏览器会明文发送、`allowCredentials` 会交给任何能发起一次握手的人的值。已重命名为 `find_user_by_credential` 和 `create_session_for_verified_credential`，两个名字都点明了校验是调用方的职责。无法通过 Suprnova 触达，因为 Suprnova 自己驱动 `webauthn-rs`（参见 `torii_integration::passkey`），只在凭据存储这一件事上才会用到 Torii。
- **一个 WebAuthn 质询在它整个 TTL 期间都可以被重放。** 两个后端都不会在读取时消费掉一个质询，SeaORM 的 `get_challenge` 还完全忽略了 `expires_at`，把已过期的质询当作存活的返回。现在两个后端的读取都会排除已过期的行，一个新的 `take_challenge` 会让一个质询恰好只被认领一次 - 和 PKCE 修复同样的“删除决定胜者”形状。

### 破坏性变更

- **Azure Blob Storage 和 Google Cloud Storage 被挪到了新的 `filesystem-azure` 和 `filesystem-gcs` feature 后面。** 除非您启用了对应的 feature，否则 `Storage::register_azblob`、`register_azblob_with`、`register_gcs`、`register_gcs_with`、`AzBlobConfig` 和 `GcsConfig` 都不再存在。如果您用到了这两个后端中的任何一个，请把它加进您的依赖：

  ```toml
  suprnova = { git = "…", tag = "v…", features = ["filesystem-gcs"] }
  ```

  您得到的是一个点名缺失项的编译错误，而不是一次运行时失败。

  这两个 opendal 服务 crate 都会拉入 `rsa`，它携带着 RUSTSEC-2023-0071（Marvin 计时攻击），上游还没有修复版本。它们是仅有的两个开启了 `reqsign-core/jwt` 的 crate，而 `reqsign-core` 那个可选的 `rsa` 正是藏在这个 feature 后面，所以把它们挡在 feature 后面，就一次性切断了通向它的全部三条 opendal 路径。`rsa` 现在是*可以避开*的：`--no-default-features --features filesystem,database-postgres` 不依赖它就能解析成功，并且仍然拥有存储子系统。此前没有任何 feature 组合能在保留存储的同时甩掉它。

  一次开箱即用的默认构建仍然携带着 `rsa` - `database-mysql` 是一个默认 feature，`sqlx-mysql 0.8.6` 非可选地依赖它 - 所以这条审计例外仍然敞开着。S3 是刻意**没有**被挡在 feature 后面的：`reqsign-aws-v4` 拿到的是不带 `jwt` 的 `reqsign-core`，所以 S3 驱动程序从来没有贡献过这样一条路径，把它挡起来只会破坏用得最多的那个云后端，却什么都清除不了。

### 新增

- **`suprnova --version`**，同时支持 `-v` 以及 clap 默认的 `-V`。用其他每一个 CLI 都在用的那个标志去问一个 CLI 的版本，不应该打印出一条用法错误。

### 修复

- **两个 Redis 操作此前都没有上限。** 缓存的标签清空操作会用 `SMEMBERS` 读出一个标签的整个成员集合，再逐个键删除，所以一个成员很多的标签会拖住这个连接，一次并发写入还可能在读和删之间丢失；标签现在是基于世代的，会被原子性地清空，并用一个有边界的 `SSCAN` 来扫描。延迟队列的晋升流程，此前会用一次不设边界的 `ZRANGEBYSCORE` 搬动每一个到期作业，所以一批一起到期的积压作业，会产生一个单独的、庞大的脚本；它现在会分批晋升。
- **两处关闭时的排空操作此前会永远等下去。** `schedule:work` 在 Ctrl-C 时，以及工作流工作进程在取消之后，都会不设期限地等待每一个飞行中的任务，所以一个永远不返回的任务，会让进程一直开着，直到 `SIGKILL` 出现 - 运维人员看到的是一个“停不下来”的守护进程。两处现在都会等待一段有边界的宽限期，然后中止剩下的部分，并报告数量。
- **发布版本钉定的清扫此前只认识两种钉定写法里的一种**，所以每一个带着一行 `cargo install --tag vX.Y.Z`、却没有依赖片段的文件，从未被发现过。`suprnova-cli/README.md` 已经连续三个发布都在告诉读者去安装 v0.6.0；`manual/cli.md` 和 `manual/cli-new.md` 停在了 v0.7.2；`manual/installation.md` 两种写法都有，其中一种被提升了，另一种却冻结不动。发现和重写现在都从同一张模式表里读取，一个文件适用哪些规则，由它的内容本身决定。
- **任何带 `filesystem` 却不带 `testing` 的构建，`cargo doc` 都会失败** - 七个 `Storage::fake` 的文档内链接无法解析，而 `lib.rs` 禁止出现失效链接。`testing` 是一个默认 feature，所以此前从来没有任何关卡步骤构建过这种组合；`check-feature-matrix.sh` 现在会构建它。
- **Torii 自己的迁移，此前没法在它自己的架构之上被重放**，所以一个持有这份架构、却没有 `torii_migrations` 跟踪表的数据库 - 从一份跳过了它的转储恢复的，或者手动迁移过的 - 就没法被纳入管理。每一个 `Table::create()` 都带着 `.if_not_exists()`；19 个 `Index::create()` 调用里没有一个带，那条 `ADD COLUMN locked_at` 的 alter 也没带，所以重放会顺利地滑过那些表，然后死在第一个 `CREATE INDEX` 上。已经在这个钉住版本的 fork（`suprnova-torii-rs` `a0f956d`）里，通过 `has_index` / `has_column`，而不是 `IF NOT EXISTS`（sea-query 会在 MySQL 上静默丢弃它）来修复 - 单纯的语法修复本会让一个默认 feature 的构建仍然是坏的。
- **一次失败的 Torii 迁移，此前会中止整个进程，而不是返回一个错误。** `SeaORMStorage::migrate` 对这个迁移器做了 unwrap，并无条件地返回 `Ok(())`，所以 `init_torii` 把这个失败映射成 `FrameworkError` 的那段代码，根本是死代码，永远走不到。
- **一个应用自己的 `users` 表，此前会静默地压制 Torii 的那张表**，因为 `.if_not_exists()` 分不清“已经是我的了”和“已经是别人的了”。这次迁移报告成功，认证却在之后因为缺一列而失败 - 这正是 `--api` 起始套件把自己的表命名为 `app_users` 的原因。Torii 的迁移现在会在迁移时发出警告，如果一张既有的 `users` 表缺少它需要的列，就点出这些列和补救办法。它仍然只是一条警告，而不是一次硬失败，这样既有的部署才能继续启动。
- **Railway 和 DigitalOcean 的部署指南，此前把平台健康检查指向了一条可能探测 Postgres 的路径。** 这两个平台都会在那项检查失败时重启容器，所以照着这份建议做，会把一次数据库的短暂抖动，变成每一个副本上的一场重启循环。两份指南现在都改用 `/_suprnova/health/live`，数据库改由控制台手动探测。旧路径仍然可以解析；任何已经部署好的东西都不需要改动。

## 0.8.0 - 2026-07-30

对一次外部红队审计的补救。这次审计给出了 19 个 P1 级发现，以及对 1.0 的一个 NO-GO 裁定；这次发布关掉了**全部十九个**，外加若干在修复它们的过程中发现、审计本身没有点名的缺陷。

有几处修复，是刻意把一种静默的错误配置，变成了一次被拒绝的启动。部署之前请先读**升级**这一节 - 一个此前运行得好好的生产应用，可能会启动不起来。

### 升级

三种此前会带着一条警告（或者干脆悄无声息）启动的配置，现在在生产环境里会失败关闭。每一条错误都会点出解除它所需要的那个变量，每一种情况也都有一个显式的开关，留给那些真正不存在这个风险的部署。

- **一个不投递的邮件驱动程序。** `MAIL_DRIVER` 未设置、`log`、`memory`，或者一个无法识别的值，都会解析成一种渲染邮件、然后直接丢弃它的传输方式 - 所以密码重置会报告成功，实际上什么都没发出去。开关：`MAIL_ALLOW_NON_DELIVERING_IN_PRODUCTION=true`。
- **明文 SMTP。** 四种凭据组合里有三种会落在一个未加密的传输上，而两者都未设置的那种情况，此前只会记一条警告，照样发送。开关：`MAIL_ALLOW_INSECURE_SMTP_IN_PRODUCTION=true`。
- **内存限流器。** 它的桶活在单个进程的堆上，所以在 N 个副本背后，每一份配额实际上都是 N 倍，而且每次部署都会把它们重置。请把 `RATE_LIMIT_DRIVER` 指向 `redis`，或者，如果您确实只跑一个进程，就设置 `RATE_LIMIT_ALLOW_MEMORY_IN_PRODUCTION=true`。一个*无法识别*的驱动程序值，会因为同样的原因失败，因为它会回退到内存 - 大写的 `RATE_LIMIT_DRIVER=Redis` 是最有可能触达生产环境的情形，因为它看起来像是配置过的。

这三种情况在开发、测试和预发布环境里都不受影响。预发布环境是刻意没有被把关的：在那里硬性失败，只会逼着团队把这个开关全局打开，反而在真正要紧的地方解除了这项检查。

两处不属于启动失败的行为变化：

- **`fill` 和 `first_or_new` 会拒绝格式错误的值。** 一个没法解码成其字段类型的值，此前会变成那个字段的 `Default`，并返回 `Ok` - `fill(attrs!{ age: "abc" })` 会把 `age` 设成 `0`，并报告成功。它现在会返回一个点名该字段的 `ValidationError`，并让模型保持不变。未知的列仍然会被静默跳过（与 Laravel 保持一致），数值类型的放宽转换仍然照常工作。
- **`/_suprnova/health?db=true` 不再返回驱动程序错误。** 细节挪到了日志里；响应体仍然保留 `"database": "error"`。调试构建仍然会包含它。解析 `status` / `database` 的仪表盘不受影响。
- **`url::signature_has_not_expired` 现在要求一个有效的签名**，并且已被弃用。它此前会对一个伪造的 URL 回答 `true` - 一个坏签名并不是“已过期”，因为它从来就没有一个可以错过的过期时间 - 所以任何单靠它来把关的处理程序，都会接受伪造的链接。它现在和 `has_valid_signature` 完全等价。如果您此前是用它来区分*已过期*和*无效*（好去渲染“请重新申请一个链接”，而不是一个 403），请改用会返回全部三种状态的 `url::signature_verdict`。这是刻意偏离 Laravel 的 `URL::signatureHasNotExpired` 的地方。

两处新增功能，只有在您选择启用时才需要您做点什么：

- **`QueueDriver` 新增了 `settle` 和 `release`**，两者都带有默认实现，所以既有的驱动程序实现无需改动就能继续编译。如果您的后端能在一个事务里同时提交一次后续写入和一次确认，就实现 `settle`；如果它能原地把一条已预留的消息重新入队，就实现 `release`。
- **批次记账现在可以是持久化的了。** `DatabaseBatchRepository` 需要两张新表，`job_batches` 和 `job_batch_settlements` - 请把它们加进您的迁移，就像 `jobs` 和 `failed_jobs` 那样。架构在 `manual/queues.md` 里。如果您继续用 `MemoryBatchRepository`，什么都不会改变。

### 安全

- **Slowloris（SEC-07）。** hyper 的请求头读取超时，文档上写的是 30 秒，实际上却是不生效的 - 它只有在连接构建器上装了一个计时器时才会启动，而此前根本没有装。一个客户端可以无限期地持有一个连接、以及一个 `SERVER_MAX_CONNECTIONS` 名额。现在已经启动，并可以通过 `SERVER_HEADER_READ_TIMEOUT` 配置。
- **Multipart 上传（SEC-05）。** 这个上限此前只作用于单个部分的载荷，而不作用于原始流，所以一个请求体在总量上可以超出这个限制。现在会在流这一层设上限。
- **带一个空密钥的 Webhook HMAC（SEC-08）。** 两个支付适配器此前都接受一个空白密钥，而一个空密钥能验证过任何东西。现在两者都会拒绝它。
- **Paddle 签名解析（P2-11）。** 一个长度为奇数、或者不是十六进制的 `paddle-signature`，此前会一路传到那个钉住版本的 SDK 里，并在其内部 panic。现在会先做校验：一个格式错误的签名会得到一个 401。
- **Passkey 绑定与重置令牌（SEC-01、SEC-02）。** 针对一个既有邮箱的匿名绑定、非本人绑定，以及没有最近重新认证的本人绑定，现在都会分别以不同的状态码被拒绝。一次密码登录现在会盖上重新认证窗口的时间戳。
- **`dev:tls`（SEC-10）。** 此前一个项目可以自行选择这个命令信任哪个 CA。
- **生成出来的 Docker Compose（P2-12）。** 此前会在所有网络接口上发布 Postgres 和 Redis，凭据还被提交进了这个版本库。现在绑定在回环地址上，密码逐次脚手架生成，`.env` 以 0600 权限写入，并且会拒绝符号链接目标。
- **健康检查端点（P2-01、CI-05）。** 它此前是用 `query.contains("db=true")` 来决定要不要查询数据库 - 一个子串测试，所以 `?nodb=true` 也会触发这次探测。现在会被正确地解析。这个 503 不再内嵌那个会点出主机、端口、架构和版本的驱动程序错误。
- **凭据签发节流（P2-02）。** 参考应用里的四条认证签发路由此前完全没有速率限制，而唯一有限制的那条路由，把它的桶建在了原始的 `x-forwarded-for` 请求头上 - 而任何客户端都可以逐请求地改变它，来换来一个全新的桶。两者都已修复；签发预算现在由这四条路由共享，所以在它们之间轮换并不会让预算翻倍。
- **一个被重新投递的链上步骤，此前会用一个新 id 重新推送它的后继者（DATA-02b，部分修复）。** 结算会*在* ack 之前推送下一个链环节，这是刻意的：先 ack 意味着这段窗口内的一次崩溃，会永久性地丢失这条链剩下的部分，而一个重复是可以恢复的，静默丢失却不行。但这个后继者的信封，此前每次推送都会拿到一个全新的 `Uuid::new_v4()`，所以这笔交易产生的重复，无论对驱动程序、对一个发件箱，还是对处理程序来说，都和一个合法的新步骤没法区分。

  最后这一点才是真正的代价。框架的投递契约是至少一次，它对重复的回答是“处理程序必须是幂等的” - 但一个以 `env.id`（它收到的唯一标识符）为键的处理程序，没法为一个链式作业满足这份契约，因为这个重复每次到来时都带着一个新 id。这份契约从结构上就是没法被满足的。

  后继者的 id，现在是从它前驱者的 id 派生出来的一个 UUIDv5，这个值在前驱者自身的多次重新投递之间是稳定的。一个被重新投递的步骤，会重新推送它之前推送过的那个 id。没有架构变更，没有新字段，没有新依赖。

  这让这个重复变得**可检测**，而这正是 DATA-02b 剩下的部分所缺少的那个原语。它并没有让这次推送和这次 ack 变成原子的（那需要一个发件箱），也还没有任何东西会在进来的路上拒绝这个重复。这两点都还悬而未决。
- **签名 URL 校验的是一个 URL，执行的却是另一个（SEC-04）。** 这个规范形式此前会把查询参数对折叠进一个 map，所以一个重复的键只会保留它的**最后**一个值 - 而 `Request::query_param` 返回的却是**第一**个。因此，一个合法签名过的 `?user=victim`，可以在原始签名原封不动的情况下，被重放成 `?user=attacker&user=victim`：校验会针对 `victim` 做规范化并通过，而处理程序实际处理的却是 `attacker`。

  这个规范形式现在会携带每一个参数对，按 `(key, value)` 排序，所以签名覆盖的是参数的精确多重集合 - 增加、删除或替换任何一个值，都会破坏这个 HMAC。一个重复的 `signature` 或 `expires` 会被直接拒绝，因为两份中的任何一份，都没法给出一个不武断的答案，来说明该由哪一个说了算。

  `Request::query_param` 现在会把一个重复的键解析成它的最后一个值，和 `query_params` 以及 `Context::query_param` 保持一致；它此前是三者之中唯一意见不合的那一个，而这个分歧正是这个缺陷的另一半。**既有的签名链接仍然照常工作** - 在没有重复键的情况下，载荷字节保持不变，这一点由一个测试钉住，因为一次悄悄让每一个未过期的密码重置链接全部失效的规范形式变更，会比这个 bug 本身还要糟糕。

  六个回归测试，涵盖两种攻击顺序、一个必须仍然能签名并通过校验的合法重复键，以及这个重新排序的保证。*没有*改变的是：`signature_has_not_expired` 仍然会把一个伪造的签名报告成“未过期”。那是 Laravel 的行为，是被刻意作为一次文档修复而定下来的，并且有它自己的测试，钉住它不被一次好心的“纠正”改掉。
- **Postgres 之下的 RBAC。** 现在会针对一个真实的 Postgres 做校验，而不只是 SQLite。
- **四条 RustSec 公告被彻底消除，而不是续期。** Pinecone 驱动程序被针对 Pinecone 的 REST API 重写了，甩掉了 `pinecone-sdk 0.1.2` - 它最新的一次发布还停留在 2024-09-06 - 连带甩掉的还有 `tonic 0.11 → rustls 0.22 → rustls-webpki 0.102`，以及 RUSTSEC-2026-0049 / -0098 / -0099 / -0104。这四条此前都已经在 `rustls-webpki >= 0.103.13` 里于上游修复，这个工作空间的其他 TLS 使用者也早就解析到了这个版本；是一个被放弃维护的 crate，把这棵依赖树钉在了那条有漏洞的线上。`.cargo/audit.toml` 里的忽略项，从五条降到了一条。这对这个驱动程序的 API 意味着什么，参见**变更**。
- **审计例外现在会过期。** `.cargo/audit.toml` 里的每一条记录都带着一个 `OWNER` 和一个 `EXPIRES` 日期，`scripts/check-audit.sh` 会在所有者缺失、日期缺失或无法解析、又或者日期已经过了的情况下，让这个发布关卡失败。`cargo audit` 本身没有“会过期的忽略项”这个概念，所以一条“临时”加进去的忽略项，会一直留在那儿，直到有人重新读一遍这份文件。剩下的这一条（RUSTSEC-2023-0071，`rsa`，它根本没有修复版本）已经有了所有者和日期。
- **可达性主张是被检查的，而不是被断言的。** `scripts/check-feature-matrix.sh` 会解析真实的依赖树，并断言没有任何一种构建 - 包括 `cargo audit` 实际读取的那个 `--all-features` - 会包含 `pinecone-sdk`、`rustls-webpki 0.102.x` 或者 `tonic 0.11.x`。一个仅靠一条没有任何东西去验证的注释来证明合理性的例外，只要有人加一个依赖，就会立刻不再成立。

### 修复

- **数据库支持的队列上，每一次 release 此前都会静默地变成一次空操作。** `JobOutcome::Released` - 一把繁忙的 `WithoutOverlapping` 锁、一次限流器退避 - 此前的实现方式是“推送一份副本，然后 ack 原件”。信封 id 正是 `jobs` 表的主键，所以这份副本会和那一行仍然持有活跃预留的记录冲突，推送会以 `UNIQUE constraint failed: jobs.id` 失败。工作进程于是正确地拒绝了 ack，所以请求的延迟从未生效，`JobReleased` 事件也没有触发，这个作业就只是停在那儿，直到可见性超时才把它重新投递。现在，release 是原地完成的一次驱动程序调用。
- **一次部分成功的批次派发，会让它已经入队的那些作业变成孤儿（DATA-02）。** 当一次 `driver.push` 在循环中途失败时，`PendingBatch::dispatch` 会删掉这个批次行 - 但已经进了队列的那些信封，仍然盖着那个批次 id，所以它们每一个结算时面对的都是一个已经不存在的批次，每次投递都会返回 `Err(batch not found)`，永远如此。现在这个批次会被结算，而不是被删除：没能派发出去的作业会被记录为失败，这个批次会被取消，这样已经入队的那些作业能正常结算，终态回调也仍然会触发。
- **此前没有任何测试验证过 `url::has_valid_signature` 会拒绝一个伪造的 URL。** 是在校验 SEC-04 的修复时发现的：即使把这个主要的签名 URL 守卫改写成接受任何签名，整个框架测试套件依然能通过。
- **一个脚手架生成的应用，此前没法迁移它的数据库，也没法构建它的镜像（REL-01b）。** 两个脚手架都没有声明 `default-run`，所以全部九个会 shell 出去执行 `cargo run` 的 CLI 包装命令，在一个全新的项目上都会失败。生成出来的 Dockerfile 有五处相互独立的缺陷 - 缺一个锁文件的 COPY、不带锁的 `npm ci`、一个缓存阶段只 stub 了两个已声明二进制文件里的一个、前端构建从一个 vite 从不创建的路径复制，以及缺一份 `frontend/src/pages` 的复制，而 `inertia_response!` 恰恰会在编译期校验它。一个开箱即用的脚手架的镜像，此前根本构建不出来。
- **`docker:init` 此前给每一种项目类型都发出同一份 Dockerfile。** 在一个 `--api` 项目上，它的第一条指令 `COPY frontend/package.json` 就会直接失败。API 项目现在会拿到一份不带前端的 Dockerfile。
- **SQL 占位符（DATA-01）。** 现在会按后端各自渲染，而不是假定只有一种方言。
- **队列结算（DATA-02a、P2-06c）。** 后续写入现在会在预留被 ack 之前完成结算，一次释放锁时的错误，也不会再把一个已经成功的作业变成一次重试。
- **一个被取消的批次此前只会触发 `Catch`，从不触发 `Then`。**
- **`Builder::clone` 此前会静默丢弃预加载计划（P2-09a）。** `User::query().with("posts")` 无论在哪里被克隆 - 分页、`count()`，或者任何会克隆的作用域 - 都会返回不带任何关系、也不报错的行。
- **Presence 花名册此前会丢失成员（P2-08）。** 这份花名册此前会在订阅之前就被快照，所以任何在这段窗口内加入的人，会在两边都不出现，而且是永久性的。
- **Pinecone 此前会把每一次索引获取都串行化（P2-14）。** 这把写锁此前会横跨两次网络往返一直持有，而 `tokio` 这把公平的 `RwLock`，意味着一个冷索引会拖住每一个热索引。
- **类型监听器此前会丢弃突发的一批变更（P2-13）。** 前沿防抖此前会在一批变更里的第一个文件上就重新生成，然后丢弃剩下的，也没有一次收尾运行，所以最后一次保存永远不会生效。
- **`ssr:check` 此前可能会挂起，并且只会尝试一个地址（P2-13）。** DNS 完全跑在超时范围之外，而且只会尝试第一个解析出来的地址 - 所以一个带 AAAA 记录、却没有 IPv6 路由的主机，会在工作进程明明在监听 v4 的情况下，被报告为已下线。
- **`suprnova serve` 此前安装的 `cargo-watch` 没有钉定版本（P2-13）。** 现在会带着一个主版本号边界，以 `--locked` 方式安装。
- **发布版本提升脚本此前只改写五份 README，别的什么都不碰。** 四个手册章节和一条公开的文档注释里，钉着的标签从来没有被任何一次发布更新过 - 那条文档注释已经落后了两个发布版本。发现逻辑现在取代了那份手工维护的清单，冒烟测试也会独立地对已提升的目录树做 grep，而不是相信提升脚本自己那一步校验。
- **`db:sync` 此前把数据库架构当作可信输入来对待（CLI-01）。**
- **`migrate:fresh` 现在被挡在 `--force` 加一次类型化确认（CLI-02）的后面**，在应用二进制文件里和在 CLI 里都是如此。
- **`log` 邮件驱动程序现在会记录整条消息**，和 Laravel 一样，并且不再在生产环境里把持有者链接写进日志。

### 新增

- **原子性的终态结算（`QueueDriver::settle`，DATA-02）。** 链上的后继者和这次确认，现在会在 `DatabaseQueueDriver` 上一起提交，关上了那扇窗口 - 此前介于两者之间的一次崩溃，要么会永久丢失一条链剩下的部分，要么会把它的下一步跑两遍。这个以预留为键的删除同时还充当一道栅栏：一个可见性在运行途中过期的工作进程，什么都不会提交，只会报告 `Settled::Stale`，所以它没法为一条现在归另一个消费者所有的消息入队工作。没法做到这一点的驱动程序，会回答 `Settled::Unsupported`，并保持文档记载的“先推送再确认”顺序。
- **`DatabaseBatchRepository`（DATA-02）。** 批次记账现在扛得住一次重启，`pending_jobs`/`failed_jobs` 现在是从以 `(batch_id, job_id)` 为键的结算行派生出来的，而不是被存储起来再递减 - 所以一个被重新投递的作业，没法在它其他的作业还在运行时，就把一个批次推向“已完成”，这道防护跨越多个进程都成立，而不只是在单个进程内。
- **`/_suprnova/health/live` 和 `/_suprnova/health/ready`。** 存活探测什么都不碰；就绪探测则会探测依赖项。把一次数据库检查接进一个存活探测，会把一次数据库的短暂抖动，变成每一个副本的一场滚动重启，而此前那个单一的端点，恰恰会招来这种情况。`/_suprnova/health` 仍然完全按文档记载的方式工作。
- **`SERVER_HEALTH_READINESS_TOKEN`。** 就绪探测的一个可选共享密钥，以固定时间比较。没有它时，就绪探测会回答 404 - 和一条未路由的路径没法区分，因为它*本来就是*路由器自己的那个 404。默认未设置，这样既有的探测才能继续工作。
- **`MAIL_SMTP_ENCRYPTION`** - `starttls` | `tls` | `none`，`ssl` 和 `null` 作为与 Laravel 兼容的别名被接受。未设置时会从凭据推导，完全复现此前的行为。这同时也让端口 465 上的隐式 TLS 变得可达：这个传输此前就支持它，只是没有任何一种环境变量组合能选中它。
- **`SERVER_MAX_CONNECTIONS` 和 `SERVER_HEADER_READ_TIMEOUT`** 已经写进了 `manual/env-vars.md`，此前它们在那里完全是缺失的。

### 变更

这次审计自己的结论是，这个关卡在 470 秒内通过，却一个 19 个 P1 都没抓住。这次发布的大部分测试工作，瞄准的正是这一点。

- **Postgres 现在会在这个关卡里跑起来。** 分布在六个文件里的十二个测试此前从未执行过。其中两个，结果发现会把 `DROP TABLE` 对准默认情况下 `localhost:5432` 上碰巧存在的任何 Postgres，而且两者都从未初始化过 `Crypt`，所以它们第一次运行就都失败了。
- **脚手架断言现在读取的是一个用户实际收到的字节**，是替换之后的，而不是模板源码。这发现了一个 API 项目会带着一条把数据库字面地命名为 `{package_name}` 的文档注释一起发布，还有一份 `.env.example` 打广告似的列了五个框架从来不会读的邮件键。
- **队列故障注入。** ACK 丢失、重新投递、租约失效和部分派发，现在都由一个装饰器驱动，它会在指定的调用上让指定的操作失败，所以每一种情况都是确定性的，而不是一场靠 sleep 赌运气的竞态。
- **支付适配器现在有了反向测试。** Stripe 的 `verify()` 此前从未被一个*有效*签名实际演练过，所以每一条依赖“走到 HMAC 比较那一步”的拒绝路径，都是未经证明的。
- **Pinecone 驱动程序现在讲 REST 了。** *这是破坏性变更，藏在默认关闭的 `vector-pinecone` feature 后面。* 动机记在**安全**那一节；接口层面的变化是：
  - `client()` 没了 - 不再有 `PineconeClient` 这回事。取而代之的是 `control_plane_get`、`control_plane_post` 和 `data_plane_post`，它们能带着您自己的请求和响应类型，通过这个驱动程序已认证、已解析主机的传输，触达*任意* Pinecone 端点。这比旧的那个脱围机制能触达的范围严格地更大。
  - `json_to_metadata` → `metadata_from_json`，元数据现在是 `serde_json::Map`，而不是 `prost_types::Struct`。`decode_match_fields` → `decode_match`，接受一个 `PineconeMatch`。`namespace()` 返回 `&str`。
  - 新增：`with_control_plane`、`with_api_version`、`with_index_host`（钉定一个已知主机，跳过控制平面这一趟往返）、`index_host`，以及 `PineconeVector` / `PineconeMatch` 这两个线上传输类型。
  - `from_env` 仍然会读 `PINECONE_API_KEY` 和 `PINECONE_CONTROLLER_HOST`，现在还会读 `PINECONE_API_VERSION`。
  - 这个 REST API 版本是钉死的，不是浮动的 - `2025-04`，也就是这个驱动程序的请求和响应形状当初是照着哪个版本写的。
  - 不再有任何东西会被串行化了。旧的驱动程序此前会在一个 `tokio::Mutex` 背后为每个名字缓存一个 `Index`，因为 `pinecone-sdk` 只在 `&mut self` 背后暴露它；新的驱动程序缓存的是一个主机字符串，共享 `reqwest` 的连接池。
  - 从控制平面获知的一个主机，无论响应里携带的是什么协议，永远都会通过 `https` 联系。
  - `Debug` 是手写实现的，API 密钥会被掩去，所以一个持有这个驱动程序的结构体上的 `#[derive(Debug)]`，没法把它打印出来。
- **针对 Pinecone 的线上契约测试。** 那些实时集成测试需要一个 `PINECONE_API_KEY`，所以没法在这个关卡里运行 - 这让一次 REST 重写的字段名（`topK`、`includeMetadata`、`vectorCount`）此前没有任何东西撑腰。现在有十三个测试，会针对一个本地的 `wiremock` 伪造实现来驱动这个驱动程序，并断言它放上线路的确切方法、路径、请求头和 JSON 请求体，外加一个非 2xx 永远不会被解码成一个结果、一条错误消息永远不会携带 API 密钥。它们把这个驱动程序钉在 Pinecone *文档记载*的契约上；只有那些标了 `#[ignore]` 的测试，才能确认文档是不是真的和线上服务一致。

## 0.7.2 - 2026-07-28

### 修复

- **`generate-types` 现在能解析没有派生宏的嵌套 prop 结构体。** 0.7.1 的生成器此前会把任何类型没有派生 `InertiaProps`/`Data` 的 prop 字段降级成 `unknown` - 所以对一个带着已提交类型文件的项目重新运行这个生成器（或者 `suprnova serve` 的监听器），会把 `Array<AdminArticleRow>` 这样的真实接口替换成 `unknown`，并让整个应用的类型检查失灵。现在，`src/` 里任何地方定义的普通结构体，都会解析成它们真实的接口，从 prop 根节点开始传递地解析；`unknown`（带一条警告）现在只留给项目确实没有定义的那些类型 - 外部 crate 的类型、枚举、元组结构体。

### 变更

- **`routes.ts` 的生成现在是可选启用的。** `generate-types` 不再不由分说地把 `frontend/src/types/routes.ts` 塞进每一个项目；传入 `--routes` 来生成它。

- **前端起始套件的依赖已经刷新。** 从 `suprnova new` 生成的新脚手架，现在会钉定当前的版本：Vite ^8.1.5、Tailwind CSS ^4.3.3、Svelte ^5.56.8（vite-plugin-svelte ^7.2.0、svelte-check ^4.7.4）、React ^19.2.8（plugin-react ^6.0.4）、Vue ^3.5.40（plugin-vue ^6.0.8、vue-tsc ^3.3.8），以及 `@types/node` ^24（Node 24 LTS 的类型线）。TypeScript 刻意停留在 ^6.0.3：它是最新的 6.x，而 svelte-check 的对等依赖范围（`^5 || ^6`）还不接受 TypeScript 7。三个起始套件都针对刷新后的这套版本，做了端到端的校验（`npm install` 加 `npm run build`）。

## 0.7.1 - 2026-07-27

一次对 0.7.0 队列路由的缺陷修复，来自一次完整的发布后复查。

### 修复

- **链式作业不再会丢失它们已声明的队列。** `ChainLink` 此前会在建链时捕获一个作业的 `max_tries`、`timeout` 和 `backoff`，却唯独不捕获它的 `Job::queue()`，所以一个直接推送时会落在它已声明队列上的作业，在作为一条链的一部分被派发时，却会落在 `default` 上 - 路由 → 作业 → 默认这个解析顺序里，“作业”这一层，对链来说会悄无声息地消失。已声明的队列现在会被捕获在这个链环节上，解析方式和直接推送完全一样。在这次发布之前写下的链载荷，解码时不受影响（`serde(default)`），一个没有声明队列的链环节，序列化出来的字节和 0.7.0 写下的完全一致。
- **失败作业记录现在会携带这个作业死在哪个队列上。** 工作进程的死信路径此前会把 `queue = "default"` 硬编码进每一条 `FailedJob` 记录，所以一个已路由作业的失败，对一个按所属池筛选失败存储的运维人员来说是不可见的。这条记录现在会携带这个信封的队列（未路由作业则是 `default`）。
- **0.7.0 的升级说明，低估了 `jobs` 迁移的必要性。** 它此前写的是“未做过滤的工作进程不受影响，不需要迁移”，但 `DatabaseQueueDriver::push` 无论这个作业是否被路由，都会在它的 `INSERT` 里点名 `queue` 这一列 - 一个 0.7.0 的二进制文件对着一张没有迁移过的表，每一次推送都会失败，不管有没有过滤。下面的 0.7.0 小节和 `manual/queues.md` 已经更正：在数据库驱动程序上，这条 `ALTER TABLE` 对每一次部署都是必需的，而且必须在二进制文件滚动升级之前运行（更旧的二进制文件会显式列出自己的列，所以先迁移是安全的）。

- **README 不再宣传一个 `#[job]` 宏。** 根本不存在这样一个宏 - 作业实现的是 `Job` trait。队列那一行现在描述的是真实的接口，包括 0.7.0 的队列路由。

### 变更

- **发布流程现在会提升 README 里的版本引用。** `bump-workspace-version.py` 会和清单文件原子性地一起，改写 README 里钉定的安装标签、分发模型示例，以及 MSRV 那一行；一份被改写过、不再匹配某个模式的 README，会让发布明确地失败。README 此前从 v0.7.0 发布起就一直在宣传 v0.6.0，因为发布流程里没有任何东西碰过它。
- **连接路由的文档现在写明只是名字解析。** `Job::connection()` 以及 `Queue::route` 的连接字段，解析的是携带在 `JobQueueing` / `JobQueued` 生命周期事件上的连接*名字*；一个单一的、进程全局的驱动程序仍然会接收每一次推送，所以它们并不会选中一个不同的驱动程序。rustdoc 和 `manual/queues.md` 此前暗示了一种并不存在的驱动程序选择能力。队列这个维度不受影响 - 它是被端到端地遵守的。逐连接的驱动程序仍然是未来的工作。
- `ChainLink` 新增了一个公开的 `queue: Option<String>` 字段，这会破坏链环节的结构体字面量构造。通过 `ChainLink::from_job` 构建的链环节 - 这也是正常路径 - 不受影响。

### 升级

如果您在数据库队列驱动程序上，是从 ≤ 0.6.x 升级过来的，请在滚动二进制文件**之前**，先应用下面的 0.7.0 迁移；这对该驱动程序上的每一次部署都是必需的，不只是那些用了 `--queue` 的部署。0.7.1 本身不需要迁移。

## 0.7.0 - 2026-07-26

### 安全

- **把 `ammonia` 升级到了 4.1.4（RUSTSEC-2026-0213）。** 4.1.3 及之前的版本，允许通过 SVG 的 `animate` 和 `set` 动画标签发起 XSS。`ammonia` 是 Suprnova markdown 流水线末端的净化器（`comrak` → `syntect` → `ammonia`），所以任何通过 `content` 渲染用户提供的 Markdown 的应用都暴露在外。这条公告发布于 2026-07-21 - 在 v0.6.5 发布之后 - 所以**截至并包括 v0.6.5 的每一个发布都受影响**。升级框架就是这个修复；不需要任何应用层代码改动。

### 新增

- **队列路由。** 作业现在可以被派发到一个指定的队列和连接，工作进程也可以被专门指定给特定的队列 - 这是 Laravel 13 的 `Queue::route(...)` 接口，类型化之后的版本。一个作业用 `Job::queue()` / `Job::connection()` 声明自己的归属；一个运维人员可以在 `bootstrap::register()` 里用 `Queue::route::<SendInvoice>(Some("redis"), Some("billing"))` 集中覆盖它，而不需要编辑这个作业。解析顺序是路由、然后作业、然后全局默认，一个路由里的 `None` 字段是顺延，而不是清空。`queue:work --queue=billing,default` 只会排空这些队列。未路由的作业属于 `default`，所以它们永远不会被搁浅。链式作业按名字解析路由，因为一个链环节存储的是它被擦除类型之后的作业。
- **`QueueDriver::pop_from`。** 带过滤条件的 pop，它的默认实现会**拒绝**一个自己没法遵守的过滤条件，而不是静默地排空每一个队列 - 一个被告知去排空 `billing` 的工作进程，却悄悄排空了一切，这在错误的池子吃掉错误的作业之前，和一次正常工作的部署没法区分。内存和数据库驱动程序都原生支持过滤。自定义驱动程序仍然能编译，并继承这个明确报错的默认行为。
- **写下了 `jobs` 表的架构文档。** `manual/queues.md` 现在携带着 `DatabaseQueueDriver` 实际期望的那份 DDL，此前只能靠读驱动程序的 SQL 才能发现它。
- **写下了 Inertia 的 `serverHead` 选项的文档。** 服务器驱动的 `<head>` 元素（Inertia 3.5.0）不需要任何框架层面的支持：客户端会从一个普通的 prop 里读取它们，所以任何处理程序都已经可以提供它们了。参见 `manual/frontend-inertia-responses.md`。

### 变更

- `Envelope` 新增了一个 `queue: Option<String>` 字段。它是 `serde(default)`，缺失时会被跳过，所以一个未路由的信封，序列化出来的字节和更早版本写下的完全一致 - 那个冻结的线上格式测试原样通过，没有 `schema_version` 的提升，混合版本的集群在一次滚动升级期间也能互操作。
- `WorkerConfig` 新增了一个 `queues: Vec<String>` 字段（为空 = 排空一切，也就是此前的行为）。
- 移除了 `ROADMAP.md`。它的设计原则活在 `manual/introduction.md` 里，工作约定活在 `manual/contributions.md` 里，部署和横向扩展的材料活在 `manual/deployment.md` 里；那份已发布/计划中的清单已经过时了。`README.md` 里指向它、用来说明“与上游的关系”的那个指针，此前就已经是悬空的了 - 那份归属声明活在 `LICENSE` 里。
- 脚手架前端现在把 `@inertiajs/{svelte,react,vue3}` 钉在 `^3.6.1`（此前是 `^3.4.0`）。3.4.0 → 3.6.1 这个区间只涉及客户端 - 对照上游的更新日志，以及 `packages/core/src/types.ts` 里的 `Page` 契约审查过，3.6.1 客户端会发送的每一个 `X-Inertia-*` 请求头，都已经被处理了。
- `scripts/release.sh` 现在会自己发布 GitHub release，说明取自这个版本 `CHANGELOG.md` 里的那个小节。此前这是一个会被漏掉的手动“下一步”，这正是 v0.5.10 和 v0.6.1-v0.6.3 只有标签、Releases 页面停在一个过时版本上的原因。预检会在这个关卡之前运行，所以一个缺失的 `gh` 或者缺失的更新日志小节，会在几秒内就失败，而且除非 `origin` 是 GitHub，否则发布会被自动跳过。

### 升级

数据库队列驱动程序上既有的 `jobs` 表**必须**添加这一新列 - `push` 无论这个作业是否被路由，都会在它的 `INSERT` 里点名它，所以一张没有迁移过的表，每一次推送都会失败。请先迁移，再滚动二进制文件（更旧的二进制文件会显式列出自己的列，忽略这个新列，所以这个顺序是安全的）：

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

*（已在 0.7.1 中更正 - 这条说明原本声称未做过滤的部署不需要迁移。）*

## 0.6.5 - 2026-07-21

### 新增

- **Stripe 适配器里托管的一次性 Checkout。** 带着 `SessionMode::OneOff` 和非空 `price_refs` 的 `Checkout::start_session`，现在会创建一个托管的 Checkout Session（`mode=payment`，每个价格引用一个行项目，`allow_promotion_codes=true`），并返回 `SessionPayload::StripeCheckoutRedirect`。仅用 `amount_hint` 的 Elements 路径不受影响；两种形状按请求各自选择。
- **Stripe Managed Payments（记录商户）支持。** `StripeProvider::with_managed_payments(true)` - 或者在 `from_env()` 里设置 `STRIPE_MANAGED_PAYMENTS=true` - 会在创建托管的一次性 session 时发送 `managed_payments[enabled]=true`。默认关闭；这个字段会被整个省略，所以未开通的账号不受影响。
- **`Checkout::session_status`。** 新的 trait 方法（默认：`PaymentError::NotSupported`），以新的中性类型 `CheckoutSessionState`（`Open` / `Complete { paid, payment_ref, amount_total }` / `Expired`）报告一个 session 在提供商那一侧的状态。Stripe 的实现映射的是 `GET /v1/checkout/sessions/{id}`；`payment_ref` 携带这个 session 的 PaymentIntent id，用于和镜像表关联。这是重定向返回页面和对账扫描所需要的服务器端校验原语。
- **`Promotions` 能力 trait。** `create_promotion_code` 会基于一张预先创建好的优惠券，铸造一个限定客户、可选带过期时间、有兑换次数上限的优惠码。通过新的 `PaymentProvider::as_promotions()`（默认 `None`）查询。Stripe（`POST /v1/promotion_codes`）和 mock 都已实现。
- **`MockPaymentProvider` 为上面这些功能做了升级。** 记录每一次 `start_session` 请求（`recorded_sessions()`），按 session id 编排 `session_status` 的脚本（`script_session_status()` - 没被编排脚本的已知 session 会报告 `Open`，未知 id 则是 `NotFound`），并带着已记录的请求实现了 `Promotions`（`recorded_promotion_requests()`）。

## 0.6.4 - 2026-07-17

### 修复

- **Eloquent 聚合在各个数据库后端上现在解码一致。** 生成出来的 `count`、`sum`、`avg`、`min` 和 `max` 表达式，现在使用同一个稳定的内部结果别名。PostgreSQL 不再返回虚假的零或者 `None`，因为它的驱动程序给聚合列打标签的方式和 SQLite 不一样，而列缺失或类型不兼容的错误现在会传播出来，而不是被静默地设成默认值。
- **批量删除没法使用调用方提供的表表达式。** 可执行的删除 SQL，永远从模型已校验的静态 `M::TABLE` 派生它的目标。这个遗留的公开渲染器参数在源码层面仍然兼容，但没法重定向或者注入删除目标。

## 0.6.3 - 2026-07-15

### 新增

- **类型化的原始读取，现在可以留在一个事务已钉定的连接上。** `Transaction::backend()` 会暴露当前活跃的后端，`Transaction::query_all(Statement)` 会在这个事务内执行类型化的聚合查询或自定义 SQL，同时保留 `QueryExecuted` 的插桩。当一个受锁作用域限定的决策依赖于计算出来的结果列时，应用不再需要一个池级别的查询，也不再需要访问私有的执行器。

## 0.6.2 - 2026-07-15

### 修复

- **带绑定参数的原始谓词现在与后端无关。** Eloquent 的 `filter_raw` 和 `where_raw`，现在在每一个数据库后端上都接受可移植的 `?` 绑定标记；PostgreSQL 渲染时，会把它们在此前的谓词、关系子查询、HAVING 子句和 UNION 分支之间，重新定位到单调递增的 `$N` 位置上。既有的、已编号的 PostgreSQL 片段，会按它们各自局部的标记顺序被归一化，而混用不同风格、或者绑定数量不匹配的情况，会在做任何 I/O 之前就校验失败。这个感知 SQL 的扫描器，会保留引号字符串、标识符、注释和美元符引用体内部的问号；`??` 会在一个带绑定的原始片段里，发出一个字面的问号运算符。

## 0.6.1 - 2026-07-15

### 新增

- **可观测的、受监督的会话清理。** `SessionMiddleware::install` 使用可配置的 `SESSION_GC_INTERVAL` 节奏（默认一小时），而 `session_gc_metrics()` 会为受保护的运维接口，暴露进程本地的运行、成功、失败、已删除行数，以及上一次结果的时间戳。
- **有边界的滑动会话触碰。** `SESSION_TOUCH_INTERVAL` 控制着最小的活动写入节奏（默认五分钟），并被夹在会话生命周期的一半以内，这样活跃的会话就没法在两次触碰之间过期。

### 修复

- **无状态请求不再创建持久化会话。** 没有携带有效会话 cookie 的请求，不会执行任何会话存储的读或写，除非处理过程真的创建了状态，否则也不会收到会话 cookie。既有的干净会话，会避免无条件的 upsert 和 cookie 变动，遗留的 cookie 会在它们下一次请求时迁移，而那些背后行已经过期的 cookie，会被清理掉，且不会重新创建空会话。

## 0.6.0 - 2026-07-10

### 新增

- **可选启用的框架子系统，带向后兼容的默认值。** 文件系统存储、SQLite/Postgres/MySQL 数据库驱动程序、MariaDB 向量驱动程序，以及 Web Push，现在都有了显式的 Cargo feature。既有的默认构建会保留全部这些能力，而 `default-features = false` 的使用者，可以选择零驱动程序，或者只选自己用到的存储/数据库/向量/推送接口。这份可执行的 feature 矩阵，会校验零驱动程序、单个驱动程序、Nation X 最小化、默认，以及全部 feature 这几种配置。
- **原始的 P-256 VAPID 私钥导入。** `VapidKey::from_bytes` 现在除了既有的 PKCS#8 PEM 导入/导出路径之外，还接受一个经校验的、32 字节大端序的 P-256 标量。

### 变更

- **VAPID JWT 现在直接用 P-256 签名。** Web Push 现在会序列化 RFC 8292 的 ES256 请求头/声明，并用 `p256` 给它们签名，移除了那个通用的 JWT 依赖，同时保留了已生成的密钥、PEM 往返、公钥编码，以及 24 小时的生命周期边界。
- **安全依赖刷新。** 更新了有漏洞的框架依赖，包括 bcrypt 和 ammonia，并在保留语法高亮的同时，收窄了 Comrak 启用的 feature。
- **Rust 1.91.1 是这次发布的 MSRV。** 每一个工作空间成员包都声明同一个 `rust-version`，生成出来的 Dockerfile 会钉定匹配的构建器镜像，完整的发布关卡会用精确的 Rust 1.91.1 工具链，编译受支持的文件系统配置。
- **OpenDAL 0.58 安全钉定。** 这个 filesystem feature 钉定了 `eas4ai/opendal` 的提交 `88717391eb72c9839d3f8e79fccad9f22fc3a1b4`，一个恰好基于官方 Apache OpenDAL 提交 `ae99a3b016e354a1b2bb2baf0c70f9f9e134970a` 的最小化 fork。这个 fork 只改动了 OpenDAL 核心加上 S3、GCS 和 Azure Blob 所使用的 Reqsign 声明，这样下游使用者才能解析到官方 Apache Reqsign 的提交 `b49cd2996b9d2d9944e84481f8835ff55b188b97` 和 `quick-xml` 0.41.0。需要一个 fork 的原因是，一个依赖仓库根目录的 Cargo patch 不会传播给使用者；不这样做，已发布的依赖图仍可能恢复出有漏洞的 `quick-xml` 0.38/0.40。

### 修复

- **原子性的发布版本元数据。** 这次版本提升，现在会在一次已校验的操作里，同时更新 `workspace.package.version` 和每一个带版本号的内部路径依赖，暂存每一份受影响的清单文件，并在发布之前，用 `cargo check --workspace` 证明一个临时的 `0.6.0` 工作空间是可行的。发布版本号会按严格的 SemVer 2.0 校验，包括数字预发布段不能有前导零这条规则。与版本无关的一次性裸远程冒烟测试，会同时从当前源码和一个已经是 `0.6.0` 的源码，派生出一个更晚的补丁发布，会在这个关卡之前拒绝有暂存/未暂存/未跟踪改动的发布目录树，会证明原子性的提交/标签发布，在一个标签被拒绝时会把两个引用都回滚，并且会证明正常的发布流程不会碰到真实的远程仓库。发布版本号必须按 SemVer 优先级递增，包括预发布阶段之间的过渡。冒烟测试构建产物永远留在它们自己的临时工作空间内，忽略调用方的任何 `CARGO_TARGET_DIR`。
- **Rustdoc 覆盖了每一个受支持的 feature 边界。** OAuth 模块链接到公开的 `OAuthAuth::complete`，这份可执行的矩阵，会在没有任何依赖的情况下，构建零驱动程序、默认，以及全部 feature 的 rustdoc。
- **文件系统流校验现在是会话作用域的。** 本地文件系统的写入器、列举器和复制器，现在会在第一次 I/O 之前解析并限定它们的路径一次，而不是每个分块/条目都做一次，与此同时，已激活的关闭/中止操作，永远会触达后端去做清理。既有的遍历和符号链接限制，对一个可信的文件系统仍然生效；先规范化再打开的检查，并不能消除针对一个正在并发修改这棵目录树的主体的竞态。

### 安全

- **发布关卡现在会失败关闭。** `release.sh` 会在改写清单文件、或者创建提交/标签之前，先委托给这个规范的完整关卡；这个关卡永远会运行 `cargo audit`，把一个缺失的 `cargo-audit` 二进制文件当作一个错误，并在任何审计失败时停下来。它还会构建并审计一个隔离出来的下游文件系统使用者，断言精确的 OpenDAL/Reqsign 源码版本，并且没有低于 0.41 的 `quick-xml`。没有新增任何公告忽略项。

## 0.5.10 - 2026-07-03

### 修复

- **`generate-types` 不再丢弃自引用结构体。** 一个带有引用自身类型的字段的结构体（一个带 `children: Vec<Self>` 的树节点，比如一个带层级的评论视图），会在类型依赖图里产生一条自环边，把它的入度钉在零以上，所以 Kahn 的拓扑排序永远不会把它输出出来 - 让每一个引用它的接口，都带着一个失效的类型名，导致 `svelte-check`/`tsc` 失败。自环边现在会在排序之前被剥离，任何困在一个引用环里（相互递归）的结构体，现在会以任意顺序被输出，而不是被丢弃，因为 TS 接口本来就可以不分声明顺序地互相引用。

## 0.5.9 - 2026-07-01

### 新增

- **`MAIL_FROM_NAME` - 认证流程邮件上的可选显示名。** 邮箱验证、密码重置和密码已修改这几个 mailable，现在会在设置了 `MAIL_FROM_NAME` 时，把它们的 `From` 请求头渲染成 `"Name <address>"`（在发送时读取，这样它才能撑过队列的 serde 往返）。`MAIL_FROM` 仍然只是一个裸地址；把 `MAIL_FROM_NAME` 留空或不设置，会保持此前那种裸地址的行为。没有任何调用点需要改动 - 这些 mailable 会自己读取这个环境变量。

## 0.5.8 - 2026-06-30

### 修复

- **`generate-types` 的路由辅助函数现在永远是合法的 TypeScript。** 当一个模块里的好几条路由共享同一个处理程序时（比如一个 `static_files::serve` 的白名单，映射着一大堆 favicon/资源 URL），第一条会保留处理程序的名字，其余的则会拿到一个从路由路径派生出来的键 - 但这个路径此前只被部分净化过（`/ { } -` → `_`），所以一个文件扩展名会把一个 `.` 泄漏进这个键：`favicon_16x16.png: (...) => ...`。这是成员访问，不是一个属性名，所以 `tsc`/`svelte-check` 会拒绝生成出来的 `routes.ts`。派生出来的键现在会被净化成合法的标识符 - 每一个非字母数字字符都变成 `_`，一个前导数字会被加上前缀 - 所以 `favicon-16x16.png` → `favicon_16x16_png`，`2fa.json` → `_2fa_json`。唯一的处理程序名不受影响。

## 0.5.7 - 2026-06-30

### 修复

- **`generate-types` 不再产出悬空的类型引用。** 一个类型是某个没有派生 `InertiaProps`/`Data` 的结构体（或者一个生成器看不到的外部类型）的 prop 字段，此前会被产出成一个裸标识符 - 比如 `user: UserInfo` - 产出一份因为那个接口从未被写出来而让 `tsc`/`svelte-check` 失败的 TypeScript。这样的引用，现在会降级成 `unknown`（`user: unknown`；`Vec<T>` → `Array<unknown>`；`Option<T>` → `unknown | null`），所以生成出来的输出永远能通过类型检查，`generate-types` 也会打印一条警告，点出那个没能解析的类型，以及引用它的那个字段，并给出修复办法（给它派生 `InertiaProps`/`Data`）。泛型参数和已解析的嵌套 InertiaProps/Data 类型不受影响。

## 0.5.6 - 2026-06-29

### 变更

- **用 Apple 登录：RS256 JWKS 校验。** 把 `suprnova-apple-rs` 提升到 v0.3.1 - Apple 的 ID 令牌现在会针对 Apple 已发布的 JWKS（RS256）来校验，而不是在结构上被直接信任。

## 0.5.5 - 2026-06-28

### 新增

- **`MagicLink` 令牌用途。** 认证流程的 `TokenPurpose` 枚举上新增了 `MagicLink` 这个变体，用于无密码的魔法链接登录令牌。

## 0.5.4 - 2026-06-28

### 变更

- **可组合的 OAuth 完成流程。** 把通用的 OAuth 完成流程拆分成 `verify_oauth_identity`（校验并解析身份）和一个薄薄的 `complete`，这样应用就可以在不触发完整会话完成副作用的情况下，校验一个 OAuth 身份。

## 0.5.3 - 2026-06-28

### 修复

- **更正工作空间版本元数据。** v0.5.2 在它的 `Cargo.toml` 版本提升被暂存之前，就已经被打了标签并推送，所以推送出去的 v0.5.2 标签，读到的仍然是 `version = "0.5.1"`。v0.5.3 用正确的工作空间版本重新切出这次发布 - 没有代码改动（v0.5.2 的 OAuth 拆分不受影响）。

## 0.5.2 - 2026-06-28

### 变更

- **可组合的 Apple 完成流程。** 把 Apple Sign-In 的完成流程拆分成 `verify_apple_identity` 加一个薄薄的 `complete_apple`，与通用的 OAuth 拆分保持一致。（说明：推送出去的 v0.5.2 标签携带着一个过时的 `0.5.1` 版本字段 - 已在 v0.5.3 中修复。）

## 0.5.1 - 2026-06-28

### 变更

- **重命名了 Apple crate。** 把 Apple 依赖重新指向改名后的 `suprnova-apple-rs` 仓库。

## 0.5.0 - 2026-06-28

### 新增

- **用 Apple 登录。** 针对 Apple 的 OAuth 令牌交换 + ID 令牌校验 + 用户 upsert；Apple 的知名端点和 `form_post` 响应模式；`OAuthProviderConfig` 上特定于 Apple 的字段；重新导出的 `AppleKeyPair`，让应用不需要一个直接的 `apple` 依赖就能配置 Apple Sign-In。

### 修复

- 从 Apple 的授权 URL 里省略 PKCE 参数（Apple 在它们存在时会拒绝这个请求）。

### 依赖

- 采纳了 `torii` 的魔法认证修复；新增 `apple-rs` v0.3.0。

## 0.4.1 - 2026-06-26

### 性能

- 预先给 `MiddlewareChain` 分配大小，消除每请求一次的 `Vec` 重新分配。

### 修复

- 让维护模式的停机文件路径，在并行测试运行下也不会冲突。

### 文档

- 对框架的文档示例做编译检查（`ignore` → `no_run`）；把分发说明和已打标签的 GitHub Releases 对齐；忽略整个 `docs/` 目录树。

## 0.4.0 - 2026-06-22

### 变更

- **分发现在是 git 跟踪的；您不需要钉在标签上。** 脚手架生成的应用依赖 `suprnova = { git = "…/suprnova.git" }`，并跟踪默认分支；用 `cargo update -p suprnova` 拉取更新。版本会作为已打标签的 GitHub Releases（`v0.4.0`……）发布，供更新日志使用，但 `Cargo.lock` 已经钉定了精确解析出来的那个提交 - 所以构建在不手动钉定一个 `tag` 或 `rev` 的情况下，也能保持可复现。安装文档不再把钉定提交呈现为更新路径。

## 0.3.0 - 2026-06-21

### 新增

- **面向 Eloquent 读取的查询插桩** - `Builder::get`、`Model::find`、`find_many` 和 `all` 现在都会发出 `QueryExecuted`，所以模型的 SELECT 和预加载查询，现在会和写入、原始查询一起，出现在 `DB::listen` 和内存查询日志里。新增了带插桩的 `ExecutorChoice::statement_all` 读取终端。
- **资源路由授权** - `ResourceRoutes::authorize_resource::<U, R>()` 会把这个约定俗成的能力检查，作为逐路由中间件，挂到每一条生成出来的资源路由上（与 Laravel 的 `authorizeResource` 保持一致）。动作到能力的映射是：`index`/`show` → `view`，`create`/`store` → `create`，`edit`/`update` → `update`，`destroy` → `delete`。一次调用就能给整个七个动作的接口加上门，而不需要依赖每一个控制器方法体自己记得写一个 `Gate::authorize`。
- **原子性的限流命中** - `RateLimiter::hit_and_check(key, max, decay)` 会在一次往返里，同时递增一个固定窗口并测试它，返回这个桶现在是否已经超出限制（`i64::MAX` 表示不限）。
- **固定时间比较辅助函数** - `constant_time_eq(a, b)`（由 subtle 支撑），用于 webhook 签名校验；`WebhookHandler::verify` 的文档现在强制要求固定时间的摘要比较。
- **Inertia 客户端提升到 3.4.0** - Svelte/React/Vue 脚手架现在会把 `@inertiajs/{svelte,react,vue3}` 钉在 `^3.4.0`（此前是 `3.1.1`），带来了 `router.poll` 模式、动态的 `usePoll`、`Inertia.once`、InfiniteScroll 的取消修复，以及可等待的 Form `onSuccess`。服务器端已经在发出完整的 3.4.0 页面对象和请求头接口（一次性 prop、前置/深合并这一族滚动选项、`matchPropsOn`、被救回的/共享的 prop），所以这只是一次客户端版本追平，没有协议变化。
- **可选的连接上限** - `SERVER_MAX_CONNECTIONS`（以及编程方式的 `Server::max_connections(n)`），会用接受循环上的一个信号量，限定并发活跃连接的数量，在 TCP 这一层施加背压。未设置 - 或者设成 `0` - 会让连接保持不设上限（默认行为，未改变）。这是一道配合反向代理和 `LimitNOFILE` 使用的后盾，不是上游速率限制的替代品。
- **可以选择退出重定向跟随** - `RequestBuilder::no_redirects()` 会让一个请求走一个不跟随重定向的 HTTP 客户端，这样一个 `3xx` 会被原样返回，而不是被追着走。当请求 URL 受不受信任的输入影响时使用它，用来关闭一个基于重定向的 SSRF 向量（一个恶意端点把请求重定向到一个内部或云元数据主机）。默认客户端仍然会跟随重定向，与通用客户端的惯例保持一致。

### 安全

- **资源路由** 现在会在授权注册表那次类型擦除的向下转型上失败关闭，而不是 panic，`authorize_resource` 的拒绝 / 未认证请求，都会在处理程序运行之前就被拒绝。
- **限流器** 通过原子性地递增并比较（`hit_and_check`），关闭了一个固定窗口的“先检查后命中”竞态。
- **队列的 `RateLimited` 中间件** 现在通过那个原子性的 `hit_and_check` 来放行作业，而不是用一对分开的 `too_many_attempts` + `hit`，所以并发的工作进程不会再全部先通过预算检查，再由其中某一个去递增，从而超出 `max_attempts` 放行。
- **上传校验器**（`mimetypes` / `mime`）现在会对上传的字节做内容嗅探，而不是信任客户端提供的 `Content-Type`。
- **文件系统路径守卫** 现在会对路径做规范化，以捕获超出存储根目录的符号链接遍历，超出了此前那种词法层面的 `../` / 绝对路径 / UNC 检查。
- **认证** 关闭了一个无密码登录的计时预言机 - 一个匹配到了、但没有设密码的账号，被给了一个密码时，现在无论是 Eloquent 还是数据库用户提供者，都会跑一次固定成本的校验 - 而 `dummy_verify` 会驱动已配置的哈希器，让不匹配用户的路径也是固定时间的。
- **Eloquent** 现在会在 `pluck` / `value` / `pluck_keyed` / `sole_value` 以及 `sum` / `avg` / `min` / `max` 这些投影路径上，校验列标识符。
- **支付** - 这个 mock 提供者的校验器，在开发环境之外会失败关闭，webhook 的来源 IP，现在通过 `TrustedProxiesConfig`（`req.ip()`）解析，而不是一个原始的 `X-Forwarded-For` 请求头。
- **文件系统路径守卫** 现在会在一个写入目标还不存在时，一路走到最近的一个*确实存在*的祖先目录，关闭了一个符号链接逃逸 - 此前一个种在半路、紧邻父目录缺失的符号链接，能溜过这道守卫。
- **`DB::init_with`** 现在会在连接之前校验环境（与 `DB::init` 保持一致），所以那个开发环境的 SQLite 回退，没法再通过这个入口在生产环境里静默启动了。
- **静态文件服务** 现在会拒绝点文件（`.env`、`.git/config`、`.htpasswd`，任何以 `.` 开头的路径段），不只是拒绝 `.`/`..` 遍历。
- **支付 webhook** 现在会用一把 `FOR UPDATE` 锁加一次重新检查，把对同一个未处理事件的并发重试串行化，并把镜像表的唯一性冲突当作良性的“已经应用过”来对待；`payments_subscription_items` 新增了一个 `UNIQUE(subscription_id, provider_item_id)`。
- **RBAC** 现在会把模型判别符默认成完全限定的类型名，所以两个共享同一个叶子名字的可认证类型，没法再继承对方的角色/权限了。
- **`invalidate_session()`** 现在会轮换会话 id（而不只是清空），关闭了一个会话固定漏洞；队列的 `WithoutOverlapping` 中间件，现在即使在这个作业 panic 时，也会释放它的缓存锁。
- **邮件提供者** 现在会给错误响应体的读取设上限（8 KiB），与 web push 客户端保持一致，这样一个恶意端点就没法拖垮发送方的内存。
- **Web push** 现在会在默认客户端上禁用 HTTP 重定向跟随，这样一个被攻击者操纵的推送端点，就没法再把一次通知 POST 用 `3xx` 重定向到一个内部或云元数据主机（SSRF）。一次重定向现在会表现为一次被拒绝的推送，而不是一次被静默跟随的请求。
- **Stripe 适配器** 的 `Debug` 现在会掩去 webhook 签名密钥，*并且*会为 `stripe::Client`（它在自己的认证请求头里携带着这个 API 密钥）打印一个占位符，所以无论上游客户端自己的 `Debug` 怎么实现，`StripeProvider` 的一次 `{:?}` 都没法把任何一个密钥泄漏进日志。
- **Stripe 适配器** 的 `from_env` 现在会拒绝存在但为空的凭据，失败关闭，而不是构造出一个带着空（因此可伪造）webhook HMAC 密钥的客户端。
- **OAuth 邮箱校验** 现在对无法识别的提供商会失败关闭：一个携带 `email`、却没有 `email_verified` 标志的 userinfo 载荷，不再被当作已校验。一个未知的提供商现在必须断言 `email_verified: true`，或者暴露一个已校验邮箱端点，这关闭了一个针对以邮箱为账号键的应用的账号关联/劫持向量。Google（只认显式的 `true`）和 GitHub（由 `/user` 契约本身校验）不受影响。

### 修复

- **嵌套预加载**（`with(["posts.comments"])`）现在的查询数量是常数级的 - 尾段会用一次跨越所有父级的批量 IN 查询来加载，而不是每个父级一次查询（N+1）。
- **`where_has`/`where_doesnt_have`** 现在会用目标表来限定闭包里的列，所以一个在中间表和目标表上都存在的列，在多对多关系上不会再产生一个歧义列错误。
- **软删除的 `delete`/`force_delete`/`touch` 以及工厂的 `persist`** 现在会遵守模型的 `#[model(connection = "…")]` 路由（与 `restore` 和其他写入路径保持一致），而不是回退到主连接池。
- **JSON:API 的 `Maybe::Missing`** 现在使用一个不会冲突的线上哨兵值，所以形如 `{"__missing__": true}` 的用户数据不会再被静默剥离。
- **已入队的通知** 现在会遵守 `should_send`（逐渠道否决）和 `after_sending`，并在工作进程上重新检查它们 - 此前只有同步路径会这样做。
- **被 release 的作业** 现在会在 ack 原件之前先推送这份重试副本，所以一次瞬时的驱动程序推送错误，不会再丢掉这个作业。
- **Paddle adjustment（退款）webhook** 现在会以被引用的交易 id 为键来更新镜像，并从 `data.totals` 读取金额，而不是在 adjustment id 下插入一行零金额的记录。
- **携带查询字符串的 SQLite URL**（`sqlite://db.sqlite?mode=rwc`）现在会构建出一个有效的单查询连接 URL，以及一个干净的磁盘文件名。
- **HTTP** 现在会把 `Accept` 的 `q` 值夹在 `[0,1]` 之间，并且即便请求体已经被预先缓冲过，也会强制执行一个 `FormRequest` 的 `max_body_bytes`；**WebSocket** 配置现在会拒绝 `max_missed_pings < 2`（此前设成 1 会在每个连接的第一次 ping 时就把它关掉）。
- **Cron** 的月中日和周中日，在两者都受限制时使用 OR 语义（与 Vixie/POSIX 保持一致）；Markdown 的 `plain_text`/摘要会保留刻意留白的空格标点；`CachedEvaluator` 会限定自己缓存的增长；`SupervisorRegistry::start_all` 第二次调用时不会再重复 spawn；测试容器现在能从一把已中毒的锁原地恢复。
- **监督程序重启退避** 现在会在一次运行保持存活至少 60 秒这个上限之后，重置回 100 毫秒这个下限，所以一个健康运行了很长一段时间才退出的守护进程，会立刻重启，而不是继承此前一次失败爆发期间攀升上去的退避时间。一个运行时长从未达到这个阈值的崩溃循环，仍然会爬升到这个上限，所以这次重置永远不会掩盖一个正在抽搐的监督程序。
- 更正了关于 `filter_op`（运算符是按允许列表校验的）、签名 URL（与 Laravel 默认的绝对签名不是字节兼容的）、`UniqueIdKind::is_valid`（一个调用方辅助函数，并没有自动接入 `find`），以及标识符长度上限（是 128，不是 64）这几处过时的文档。

### 文档说明

- 在路由和授权章节里，写下了资源路由授权（`authorize_resource`）的文档；在速率限制章节里，写下了这个原子性的 `hit_and_check` 计数器的文档。

## 0.2.0 - 2026-06-21

新增基于角色的访问控制、一条 Markdown 内容/文档渲染流水线，以及原生的静态文件服务。

### 新增

- **二级 RBAC** - `HasRoles` trait；带一张 `role_has_permissions` 连接表的角色 + 权限；`PermissionMiddleware` / `RoleMiddleware`（两者都失败关闭 / 默认拒绝）；`CreateRbacTables` 迁移；以及 `create_role` / `create_permission` / `give_permission_to_role` 这几个辅助函数。
- **内容渲染** - Markdown 渲染和一条文档构建流水线：`MarkdownRenderer`、`build_docs`、`DocsCatalog` / `DocsChapter`、标题提取，以及 `slugify_heading`。渲染出来的 HTML 会被净化（comrak + syntect + ammonia）。
- **原生静态文件服务** - `StaticFiles::public()` 这个后备处理程序，会在网站根路径上提供一个 `public/` 目录，取代了应用里手写的逐资源白名单控制器。

### 修复

- 新生成的应用会继承一个框架层面的 `time = 0.3.47` 兼容性钉定，避免新脚手架的依赖解析中，`time 0.3.48` 带来的 Rust 1.96 一致性冲突。

### 文档说明

- 在整本手册、README 和路线图里，写下了两个已发布起始套件的文档 - **Nebula**（Breeze 级别的认证）和 **Pulsar**（产品网站 + 社区） - 围绕已发布的这部分接口重构了路线图；并在文档全篇统一了版本引用。

## 0.1.0 - 2026-06-10

首次发布的 Suprnova。Suprnova 是一个受 Laravel 启发的 Rust web 框架，从 Kit fork 而来，走上了自己的方向。今天的对齐目标是 Laravel 13.x。

这次发布采用 git 分发模型：框架的使用者依赖 `suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`，CLI 用 `cargo install --git` 安装。

### 新增

#### HTTP、路由与中间件

- 带路由分组、前缀、参数约束、命名路由的 `Router`
- 通过 `routes!` 宏做编译期校验的路由注册
- 资源路由（`Router::resource`），生成七条标准路由
- 签名 URL（`url::signed_route` / `url::temporary_signed_route` 自由函数，加上 `Redirect::signed_route` / `Redirect::temporary_signed_route`）
- 重定向辅助函数 - `Redirect::to`、`Redirect::back`、`Redirect::route`、`Redirect::with_input`、`Redirect::with_errors`、`with_flash`
- 带全局、分组和逐路由层级的 Middleware trait
- 内置中间件 - CORS、CSRF、会话、请求超时、请求 ID、节流 / 登录节流、签名 URL 校验、已认证、邮箱已验证、暴力破解
- Abort 辅助函数（`abort`、`abort_unless`、`abort_if`）
- `suprnova::handle_request(...)` - 用于针对一个路由器 + 中间件链，服务单个 hyper 请求的公开适配器

#### Inertia.js 前端桥接

- 带 TypeScript 类型产出的 `#[derive(InertiaProps)]`
- 带编译期组件校验的 `inertia_response!` 宏
- 三个一等公民起始前端 - **Svelte 5**（启用 runes）、**React 19**、**Vue 3.5** - 全都基于 Inertia 3.1.1 + Vite 8 + Tailwind v4
- 部分重新加载（`only` / `except`）、延迟 prop、持久布局、加密历史、滚动位置保留
- `Inertia::paginate(component, key, paginator)`，用于分页器 → Inertia prop 接线

#### Eloquent 风格 ORM（基于 SeaORM）

- `#[suprnova::model]` 属性宏，一次性产出一个 SeaORM 实体，以及面向用户的 Eloquent 结构体
- 完整的 `Model` trait - `create`、`find`、`find_or_fail`、`find_many`、`all`、`query`、`save`、`update`、`delete`、`force_delete`、`refresh`、`fresh`、`replicate`、`replicate_into`、`increment`/`decrement`、`destroy`、`is`/`is_not`、`to_array`/`to_json`
- 带 `Attrs` 信封的可填充 / 受保护批量赋值
- 22 种属性转换 - 布尔值、整数、浮点数、日期、枚举、已哈希、已加密、JSON、集合、金额、带时区的日期时间
- 通过 `#[suprnova::model]` 实现的访问器 / 修改器
- 自动时间戳（`created_at`、`updated_at`）
- 带 `force_delete`、`restore`、`trashed`、`only_trashed`、`with_trashed` 的软删除（`deleted_at`）
- 十一种关系类型 - `HasOne`、`HasMany`、`BelongsTo`、`BelongsToMany`、`HasOneThrough`、`HasManyThrough`、`MorphOne`、`MorphMany`、`MorphTo`、`MorphToMany`、`MorphedByMany`
- 逐家族的 morph 枚举 + 带 `APP_KEY_PREVIOUS` 轮换的 morph 注册表
- 通过 `.with(...)`、`.with_count(...)`、`.load_missing(...)` 实现的预加载
- 面向 `has` / `where_has` 的相关 EXISTS 引擎
- 十六个生命周期事件（retrieving、retrieved、creating、created、updating、updated、saving、saved、deleting、deleted、restoring、restored、force-deleting、force-deleted、replicating、trashed）
- 带按方法通过 inventory 自动注册的 `Observer<M>` trait
- 通过 `#[scopes(M)]` 实现的本地作用域，通过 `GlobalScope` 实现的全局作用域
- `Collection<M>` 的 Laravel 接口 - `pluck`、`key_by`、`group_by`、`where_in`、`first_where`、`contains_where`、`partition` 等等
- 三种分页器 - `paginate`（长度感知）、`simple_paginate`、`cursor_paginate` - 全都序列化成 Laravel 形状的 JSON
- 用于批量行迭代、且不会 OOM 的 `chunk` / `lazy` / `cursor`
- `lock_for_update` / `shared_lock` 行级锁
- 带 `DynamicRow`（用于临时查询）的 `DB::table(...)` 查询构造器
- 带保存点、死锁重试、多连接读写分离的 `DB::transaction(...)`
- `DB::listen(...)` 加 `QueryExecuted` / `TransactionBegan` / `TransactionCommitted` / `TransactionRolledBack` 事件
- `Prunable` trait 加 `model:prune` 控制台命令
- `dump` / `dd` 查询辅助方法
- 用于 UUID / ULID 主键的 `#[model(unique_id="...")]`

#### Auth

- `Authenticatable` trait 加 `EloquentUserProvider<M>`
- `Auth::attempt`、`Auth::login`、`Auth::user`、`Auth::user_or_fail`、`Auth::user_as<T>`、`Auth::logout`、`Auth::check`
- 多个具名守卫（web 会话、API 令牌）
- 邮箱验证流程 - `EmailVerification`、`EnsureEmailVerifiedMiddleware`、签名验证 URL、`EmailVerificationMail`
- 密码重置流程 - `PasswordReset`、有节流的令牌、`PasswordChangedMail`、`PasswordResetLinkSent` 事件
- 双因素 TOTP - 绑定、校验、恢复码、重放防护
- 暴力破解 / 登录节流 - 按 IP + 标识符建键，`LoginThrottleMiddleware`
- 带稳定不透明令牌的记住我 cookie
- 六个认证事件 - `LoginAttempted`、`LoggedIn`、`Authenticated`、`LoggedOut`、`PasswordResetLinkSent`、`EmailVerified`
- 由 `github.com/eas4ai/suprnova-torii-rs` 这个 Torii fork 支撑的浏览器会话

#### 授权

- `Gate` 门面 - `define`、`allows`、`denies`、`authorize`、`any`、`none`、`check`（同步 + 异步两种变体）
- 用于策略注册的 `#[policy(Model)]` 宏
- 资源路由自动授权

#### 支付

- 与提供商无关的五 trait 接口 - `Checkout`、`Payment`、`Subscription`、`CustomerStore`、`WebhookHandler`
- `PaymentProvider` 这个总括 trait，加上通过 `as_payment()` 实现的能力查询
- 数据库镜像 - `customers`、`subscriptions`、`subscription_items`、`payments`、`refunds`、`payment_webhook_events`（带 UNIQUE 以实现幂等性）
- 带流程标记的 `SessionPayload` 枚举（一次性 vs 订阅）
- 两个作为工作空间 crate 的参考适配器 - `suprnova-payments-stripe`（网关，完整的 `Payment` 实现），`suprnova-payments-paddle`（记录商户，没有 `Payment` 实现）
- 面向测试的 mock 提供者

#### 队列、作业、批次与链

- `Job` trait - `handle`、`max_tries`、`backoff`、`timeout`、`fail_on_timeout`
- `Queue::push`、`Queue::push_later`、`Queue::push_unique`、`Queue::push_unique_later`
- 驱动程序 - `sync`、`null`、`redis`、`database`
- `JobMiddleware` trait - 六个内置中间件
- 批次和链 - `Queue::batch(jobs).dispatch()`、fluent 链构建器、取消、进度跟踪
- 带重放的失败作业存储
- 带优雅停机、可配置并发度、通过 `catch_unwind` 实现 panic 恢复、结算指标的工作进程
- 十二个覆盖排队、处理、失败、release、工作进程生命周期的队列事件

#### 广播与 WebSocket

- `ws!()` 宏 + `Router::ws`，用于类型化的 WebSocket 端点
- `WsSocket` 的 Sink/Stream 拆分
- 通过 `Supervisor` trait 实现的自动重启监督程序
- 带 `Channel`、`Private`、`Presence` 频道的 `BroadcastHub`
- JSON 信封协议、presence 的 join/leave/here，带崩溃恢复的可配置 presence TTL
- 桥接到 `EventDispatcher` 的 `Broadcastable`
- 带可配置 WS_TASKS 排空的、无 pong 即关闭心跳
- 逐路由的 WebSocket 中间件
- 1 MiB / 64 KiB 更安全的默认值 + `WsConfig::generous()` 工厂
- 来源策略 + 违反协议时以 1011 关闭

#### 通知与邮件

- `Notification` trait + `Notify::send(recipient, notification).await`
- Mailable + Markdown 模板渲染
- 数据库 / 邮件 / 广播 / web push 渠道
- VAPID 签名 + RFC 8291 ECE 载荷加密（通过 `suprnova-web-push`）
- VAPID 主体校验、retry-after 解析、8 KiB 拒绝响应体上限
- 用于收件人类型化的 Notifiable trait

#### 事件

- 类型化的事件分发器 - `EventFacade::dispatch`、`EventFacade::listen<E, L>`、`EventFacade::forget`
- 可取消的 saving/updating 事件（返回 `EventResult::cancel`）
- 可入队的监听器

#### 文件系统

- 带多驱动程序支持的 `Storage::disk("name")` - 通过 OpenDAL 实现的本地、S3、Azure、GCS
- 移动、复制、是否存在、大小、mime、最后修改时间、前置/追加
- 流式上传和下载

#### 缓存

- `Cache::store("name")` + 驱动程序注册
- 驱动程序 - memory、redis（带有边界的连接超时）、database、file
- `remember`、`forever`、`tags`、原子递增/递减、锁

#### 向量数据库

- 带四种驱动程序的 `VectorDriver` trait - 内存、Qdrant（UUID-5 id 映射）、Pinecone（原生字符串 id）、MariaDB 原生 `VECTOR(N)` + HNSW 索引（11.7+）
- 余弦 / 点积 / 欧几里得距离

#### 控制台二进制文件与 CLI

- 逐项目的 `console` 二进制文件 - `php artisan` 的 Rust 对应物，通过 `#[suprnova::console::command]` 运行用户定义的命令
- 用于类型化参数的 `#[derive(Command)]`
- `suprnova` CLI - `new`、`serve`、`migrate`、`db:sync`、`generate-types`、`key:generate`、`make:{controller,middleware,action,error,inertia,migration,task,command}`、`db:seed`、`model:prune`
- `--version` 标志
- 面向三种前端的后端 + API 起始套件的脚手架模板

#### 功能标志

- 带快照加载的 `DatabaseEvaluator`
- 带 TTL 的 `CachedEvaluator`
- `FeatureMiddleware` 提取器
- 管理端 CRUD 接口
- 用于跨进程亚秒级传播的 `FeatureSync` trait

#### 调度

- Cron 表达式解析器
- 带可组合谓词的 `Schedule::task(...)`
- 单服务器锁、防重叠、派发跟踪
- `schedule:run` 控制台命令

#### 验证

- `validator` 0.20 集成
- `#[request]` + `#[derive(FormRequest)]` 宏
- 逐表单大小上限的 `#[form_request(max_body_bytes = N)]`
- 面向用户自写 `impl FormRequest` 的可选退出项 `#[form_request(custom_hooks)]`
- 生命周期钩子 - `authorize`、`after_validation`、`after_validation_async`

#### 数据库驱动程序

- 由 SeaORM 支撑的 SQLite、Postgres、MySQL、MariaDB 支持
- 基于 URL 的驱动程序检测
- 迁移系统 + `migrate`、`migrate:rollback`、`migrate:status`、`migrate:fresh`、`migrate:refresh`

#### HTTP 客户端

- `Http` 门面 - `get` / `post` / `put` / `patch` / `delete`，返回一个 `RequestBuilder`；`.send().await` 产出一个 `ClientResponse`
- rustls TLS、30 秒默认超时、`suprnova/<version>` user-agent
- `json` / `form` / `body` / `header` / `bearer_token` / `basic_auth` / `timeout` 这几个可链式调用的方法
- `RequestBuilder::retry(max_attempts, base_backoff)` - 面向瞬时失败和 5xx 的指数退避；遵守 `Retry-After`
- `Http::fake(|| async { ... }).await` 测试守卫，带 `fake_response(method, url_substring, status, body)` + `assert_sent` / `assert_not_sent`

#### 加密

- `Crypt` 静态门面 + `EncryptionKey`（`crypto::*`）；带 12 字节随机 nonce 的 AES-256-GCM
- `encrypt_string` / `decrypt_string` / `encrypt<T>` / `decrypt<T>`
- 防止跨协议重放的 `CryptPurpose` AAD 绑定
- `APP_KEY_PREVIOUS` 轮换
- 用于铸造新密钥的 `suprnova key:generate` CLI 命令

#### 测试

- `#[suprnova_test]` 异步测试宏
- 带并行安全实例的 `TestDatabase::fresh::<Migrator>()`
- 用于逐测试 mock 的 `TestContainer::bind`
- HTTP 测试辅助函数 - `Test::get`、`Test::post`、JSON / form / multipart
- Queue / Mail / Notification / Event 伪造实现
- `assert_emitted`、`assert_dispatched`、`assert_dispatched_times`

### 变更

- 认证校验和密码重置流程，现在通过已配置的用户提供者运行，而不是 Torii 内部机制。
- 生成出来的应用必须实现 `get_auth_password`；脚手架生成的示例现在会明确地失败，而不是让登录永远静默失败。
- 本地发布关卡现在接入了 `scripts/release.sh`，这个仓库也带上了一个强制执行的 pre-push 钩子，用于 fmt、clippy、测试、文档和 feature 构建。
- 脚手架生成的开发端口文档，改成了当前的后端/前端默认值（`8765` / `5765`），并写下了 `dev:tls` 和 `--with-portless` 的文档。
- `MAIL_FROM` 现在会在验证或重置令牌被签发之前先校验，避免在邮件配置无效时留下孤立的认证流程行。

### 修复

- React 脚手架模板与已发布起始套件之间的偏差。
- 根路由分组不再生成重复的 `//` 路径。
- 字面路径重定向现在会通过预期的路由路径派发。
- 广播扇出测试现在能处理 `track` / `untrack` 的结果。
- 邮件 log 驱动程序现在会发出渲染后的文本正文，所以验证和密码重置链接会出现在本地开发日志里。
- 密码重置的测试覆盖，钉住了会话和记住我的撤销行为。

### 说明

- **分发模型**：端到端基于 git。`suprnova = { git = "https://github.com/eas4ai/suprnova.git" }`；CLI 通过 `cargo install --git` 安装。没有任何东西发布到 crates.io。
