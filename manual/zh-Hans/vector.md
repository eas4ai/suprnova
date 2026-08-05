# 向量

Suprnova 提供一个 Laravel 形状的 `Vector` 门面，背后由四种驱动程序之一支撑 - 进程内的 Memory、Qdrant、Pinecone，或者 MariaDB 原生的 `VECTOR(N)` - 在启动时通过 `Vector::register` 显式选定。这个门面是 `VectorDriver` trait 之上的一层薄层，所以自定义后端可以用和内置驱动程序一样的方式接入。

## 快速上手

```rust
use std::sync::Arc;
use suprnova::{MemoryVectorDriver, Vector, VectorItem};

// 启动引导（通常在应用启动时做一次）
Vector::register("documents", Arc::new(MemoryVectorDriver::new()));

// 使用它
let store = Vector::store("documents")?;
store
    .upsert(vec![
        VectorItem::new("doc-1", embedding_for("Hello"), serde_json::json!({ "title": "Hello" })),
        VectorItem::new("doc-2", embedding_for("World"), serde_json::json!({ "title": "World" })),
    ])
    .await?;

let hits = store.similar(query_embedding, 10).await?;
for hit in hits {
    println!("{}: {} (score {:.3})", hit.id, hit.metadata["title"], hit.score);
}
```

## 契约

```rust
#[async_trait]
pub trait VectorDriver: Send + Sync + 'static {
    async fn upsert(&self, store: &str, items: Vec<VectorItem>) -> Result<(), FrameworkError>;
    async fn similar(&self, store: &str, query: Vec<f32>, k: usize) -> Result<Vec<VectorMatch>, FrameworkError>;
    async fn delete(&self, store: &str, ids: Vec<String>) -> Result<(), FrameworkError>;
    async fn count(&self, store: &str) -> Result<usize, FrameworkError>;
}
```

`VectorItem` 携带一个任意的 `String` id、一个 `embedding: Vec<f32>`，以及自由形式的 `metadata: serde_json::Value`（必须是一个 JSON 对象或者 `null`）。`VectorMatch` 返回原始的 id、后端的相似度分数，以及同样形状的 metadata。

这个 trait 特意做得很小。当您需要搜索上的过滤表达式、稀疏向量、scroll/list、快照，或者量化参数时，请通过它公开的 `client()` 脱围机制，下沉到驱动程序底层的 SDK。

### 为什么 Suprnova 有所不同

Laravel 只通过 Postgres 的 `pgvector` 提供向量能力。那是 PHP 形状的答案：选定一个存储后端，把它藏在单一一个驱动程序背后，就算完事。Suprnova 把这个选择当作一个配置层面的关注点。同一个 trait 涵盖了：用于测试的进程内 `HashMap`，当嵌入数量证明其运维成本合理时用的专用向量数据库（Qdrant、Pinecone），以及当您更想把向量放在产生它们的那些行旁边时用的关系型后端（MariaDB 11.7+）。Weaviate、Milvus、LanceDB、pgvector 和 LibSQL 排在真实的用户需求后面 - 没有一个是被这个 trait 的形状挡住的。

当您应用的其余部分都能放进一个引擎时，MariaDB 11.7+ 会把向量和关系型表、JSON 文档，以及系统版本化的时态数据放在一起 - 比单独运行 Postgres + Redis + Qdrant 要少操心几个活动部件。关于这项建议的完整语境，请参见[部署](deployment.md)。

## 驱动程序

### Memory - `MemoryVectorDriver`

进程内驱动程序，背后是 `HashMap`。余弦相似度；维度不匹配的点在查询时会被静默跳过（这样混合维度的测试数据就不会炸掉），零向量查询会清楚地报错。

```rust
Vector::register("docs", Arc::new(MemoryVectorDriver::new()));
```

用在测试和开发中。每一个 `MemoryVectorDriver::new()` 实例都是彼此隔离的 - 两次 `new()` 之间没有共享状态。

### Qdrant - `QdrantVectorDriver`

通过官方的 `qdrant-client` SDK，以 gRPC（默认端口 6334）和 Qdrant 通信。

```rust
use suprnova::{QdrantDistance, QdrantVectorDriver};

let driver = QdrantVectorDriver::from_url("http://localhost:6334")?
    .with_distance(QdrantDistance::Cosine)  // 默认值
    .with_auto_create(true);                // 默认值

Vector::register("docs", Arc::new(driver));
```

对于 Qdrant Cloud：

```rust
let driver = QdrantVectorDriver::from_url_with_api_key(
    "https://xxxxxxxx.eu-central.aws.cloud.qdrant.io:6334",
    std::env::var("QDRANT_API_KEY")?,
)?;
```

**ID 映射。** Qdrant 要求点 ID 必须是 `u64` 或者一个合法的 UUID。框架用三条规则来桥接任意字符串：

1. 如果这个字符串能解析成 `u64`，就使用 `Num(u64)` 这个变体。
2. 如果这个字符串是一个合法的 UUID，就原样使用 `Uuid(String)` 这个变体。
3. 否则，从一个稳定的命名空间派生出一个确定性的 v5 UUID。

调用方的原始字符串，会被存放在这个点的载荷里，位于一个保留键 `__suprnova_id`（导出为 `SUPRNOVA_ID_PAYLOAD_KEY`）之下，并在读取时从 `VectorMatch.metadata` 里被去掉。通过 `driver.client()` 直接查询 Qdrant 的高级用户，可以对 `__suprnova_id` 做过滤，从而把框架写入和直接调用桥接起来。

**自动创建。** 对一个从未见过的集合执行第一次 `upsert` 时，驱动程序会创建它，维度从第一个条目推断，距离度量则使用已配置的那个（默认余弦）。对竞态是安全的 - 针对同一个全新集合的并发 upsert 不会失败；谁先创建谁赢，另一个则继续往下走。通过 `.with_auto_create(false)` 禁用，以要求显式创建。

**缓存失效。** 如果一个集合被外部删除了（或者 Qdrant 在持久化刷盘之前重启了），驱动程序会在 upsert 时检测到“未找到”这个错误，丢弃这条缓存条目，重新运行一次 `ensure_collection`，并重试一次。

**脱围机制。** `driver.client()` 返回底层的 `qdrant_client::Qdrant` - 用它来做搜索上的过滤表达式、scroll、快照，或者其他没有通过这个 trait 暴露出来的 API。`QdrantVectorDriver::resolve_point_id`、`build_point` 和 `decode_match` 让您可以混用直接调用和经过 trait 路由的调用，而不会丢失 id 转换。

**本地搭建。** 通过 Docker 运行 Qdrant：

```bash
docker run -p 6334:6334 -p 6333:6333 qdrant/qdrant
```

集成测试通过下面这样运行：

```bash
QDRANT_URL=http://localhost:6334 cargo test -p suprnova --test vector_qdrant -- --ignored
```

### Pinecone - `PineconeVectorDriver`

> **由 Cargo feature 把关 - 默认关闭。** 用 `cargo build --features vector-pinecone` 启用（或者在您 `Cargo.toml` 里 `suprnova` 依赖下加上 `features = ["vector-pinecone"]`）。这个 feature 不会带来额外的依赖 - 它只把关这个驱动程序的编译，仅此而已 - 之所以默认关闭，纯粹是因为大多数应用不用 Pinecone，不应该为编译它付出代价。

通过它的 REST API 和 Pinecone 通信，用的是框架本来就带着的这个 HTTP 客户端。

> **为什么不用官方 SDK？** 这个驱动程序过去包装的是 `pinecone-sdk`，它讲的是 gRPC。这个 crate 最新的发布版本（0.1.2，发布于 2024-09-06）锁定了 `tonic 0.11 → rustls 0.22 → rustls-webpki 0.102`，而 `rustls-webpki 0.102` 带着四条 RustSec 通告，它们都已经在 `>= 0.103.13` 的上游修复了。一个被放弃维护的 crate，拖住了整棵依赖树，而“等上游修复”这件事，没有一个版本是真会有尽头的。Pinecone 通过 HTTPS 暴露了这个驱动程序需要的每一个操作，所以这条 REST 路线一次性去掉了四条通告和两个依赖。

```rust
use suprnova::PineconeVectorDriver;

// 直接给 API 密钥
let driver = PineconeVectorDriver::from_api_key(std::env::var("PINECONE_API_KEY")?)?;

// 或者通过环境变量：PINECONE_API_KEY，外加可选的 PINECONE_CONTROLLER_HOST
// 和 PINECONE_API_VERSION
let driver = PineconeVectorDriver::from_env()?;

// 绑定到一个非默认的命名空间
let driver = driver.with_namespace("public");

Vector::register("docs", Arc::new(driver));
```

通过 `Vector::store(name)` 传入的这个存储名字，映射到一个 Pinecone 索引名字。驱动程序会在首次使用时，通过控制平面的 `GET /indexes/{name}`，惰性地解析出这个索引的主机，然后把它缓存下来。如果您已经知道这个主机，可以把它钉死，跳过这次往返：

```rust
let driver = PineconeVectorDriver::from_env()?
    .with_index_host("docs", "docs-abc123.svc.aped-1234.pinecone.io");
```

一个从控制平面学到的主机，无论响应里写了什么，总是通过 `https` 联系。一个通过 `with_index_host` 钉死的主机，会保留您给它的那个协议方案，所以一个跑在 `http://` 上的本地模拟器也能用。

**API 版本。** Pinecone 按日期对它的 REST API 做版本管理，并且要求把那个版本钉在一个请求头里。驱动程序钉死了 `2025-04` - 这是它的请求和响应形状编写并测试所针对的版本 - 并暴露出 `with_api_version`（或者 `PINECONE_API_VERSION`）供您有意去移动这个版本。它不会浮动：`describe_index_stats` 里的命名空间键约定，就是版本之间发生过变化的东西之一，而 `count()` 读的正是那份映射。

**不自动创建。** 创建一个 Pinecone 索引，需要选定云平台（AWS/GCP/Azure）、区域、向量维度、距离度量，以及删除保护 - 需要权衡的东西太多，没法给出一个好的默认值。请先通过 Pinecone 控制台、Pinecone CLI，或者一次 `control_plane_post` 调用来创建索引，再注册，然后让框架指向这个已有的名字。

这是和 Qdrant 驱动程序之间最主要的不对称之处：Qdrant 会在第一次 upsert 时自动创建集合。

**ID 与 metadata。** Pinecone 原生就接受任意的 `String` id，所以 `VectorItem::id` 会直接透传过去。metadata 从头到尾都以 JSON 形式携带 - `PineconeVectorDriver::metadata_from_json` / `metadata_to_json` 只强制执行框架自己的那条规则：metadata 必须是一个对象或者 `null`。Pinecone 自己则把 metadata 的*值*限制为字符串、数字、布尔值和字符串列表，并在服务端拒绝嵌套对象；驱动程序不会重新实现这条检查，因为 Pinecone 的规则是有版本的，本地抄一份只会跟着漂移掉。

**批量上限。** Pinecone 的文档规定，每次 upsert 最多 1000 个向量，每次 delete 最多 1000 个 id。驱动程序会把您给它的东西在一个请求里原样发出去，而不是静默地分块 - 一次部分成功的写入，比一次被拒绝的写入更难推理。如果您超过了这些上限，请自己在调用方那边分批。

**命名空间。** 一个驱动程序实例绑定到一个命名空间。要使用同一个索引下的多个命名空间，请给每个命名空间在不同的存储名字下各注册一个驱动程序：

```rust
Vector::register("docs-public", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("public")
));
Vector::register("docs-private", Arc::new(
    PineconeVectorDriver::from_env()?.with_namespace("private")
));
```

**吞吐量。** 没有任何东西会被串行化。驱动程序按索引缓存的是一个主机字符串，而不是一个连接句柄，请求之间共享 `reqwest` 的连接池 - 所以对同一个索引的并发调用会真正并发地进行。（这个驱动程序取代的那个 gRPC 版本，会把每个名字对应的一个 `Index` 按在一个 `tokio::Mutex` 后面，因为 `pinecone-sdk` 只在 `&mut self` 后面暴露 `Index`。）

**脱围机制。** `control_plane_get`、`control_plane_post` 和 `data_plane_post` 可以触达 Pinecone 发布的任何端点，用您自己的请求和响应类型，经由驱动程序那个已认证、已解析好主机的传输层 - 过滤表达式、稀疏向量、按 id 取值、`/vectors/list`、索引管理：

```rust
#[derive(serde::Deserialize)]
struct FetchResponse { vectors: Vec<suprnova::vector::PineconeVector> }

let hits: FetchResponse = driver.data_plane_post(
    "docs",
    "/vectors/fetch_by_metadata",
    &serde_json::json!({ "filter": { "genre": { "$eq": "comedy" } }, "limit": 2 }),
).await?;
```

**测试。** 传输格式契约测试会在这个 feature 下默认运行：它们针对一个本地伪造实现来驱动这个驱动程序，并断言它放到网络上的确切方法、路径、请求头和 JSON 请求体。这些测试把驱动程序钉死在 Pinecone *文档记载* 的那份契约上。要确认文档和线上服务是否一致，需要那些标了 `#[ignore]` 的集成测试，它们需要下面这两个环境变量：

```bash
PINECONE_API_KEY=... PINECONE_TEST_INDEX=my-test-index \
    cargo test -p suprnova --features vector-pinecone \
    --test vector_pinecone -- --ignored
```

### MariaDB - `MariaDbVectorDriver`

通过直接的 `sqlx::MySqlPool`，使用 MariaDB 原生的 `VECTOR(N)` 列类型和 HNSW 索引，与 MariaDB 11.7+ 通信。您第一次调用驱动程序的某个方法时，它会跑一次 `SELECT VERSION()`，并拒绝任何低于 11.7 的版本 - 更老的服务器没有向量函数。

```rust
use std::sync::Arc;
use suprnova::{MariaDbDistance, MariaDbVectorDriver, Vector};

let driver = MariaDbVectorDriver::from_url(
    "mysql://user:pass@localhost:3306/myapp",
)?
.with_distance(MariaDbDistance::Cosine);  // 默认值

Vector::register("documents", Arc::new(driver));
```

`from_url` 是惰性的 - 它会校验 URL 语法，但在首次使用之前**不会**打开一个连接，所以即便数据库还没就绪，在应用启动时调用它也是安全的。当您需要自定义连接池选项时，用 `MariaDbVectorDriver::from_pool(pool)` 包一个已有的连接池。

**架构由您掌控。** 驱动程序不会自动创建表 - 架构是迁移层面的关注点。推荐的做法是 `driver.ensure_table_sql_for(name, dim)`，它会继承驱动程序已配置的距离度量，这样迁移里的 `DISTANCE=` 子句，和 `similar` 用的那个查询函数，就保证能对得上：

```rust
let driver = MariaDbVectorDriver::from_url(url)?
    .with_distance(MariaDbDistance::Cosine);

let sql = driver.ensure_table_sql_for("documents", 1536)?;
// 结果：
// CREATE TABLE IF NOT EXISTS `documents` (
//   id VARCHAR(255) NOT NULL PRIMARY KEY,
//   embedding VECTOR(1536) NOT NULL,
//   metadata JSON NULL,
//   VECTOR INDEX (embedding) DISTANCE=cosine
// ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4
```

对于那些没有驱动程序在作用域里的迁移生成器（CLI 工具、构建脚本），请使用静态方法 `MariaDbVectorDriver::ensure_table_sql(name, dim, distance)`，并传入您之后会在驱动程序上配置的同一个 `MariaDbDistance`。

**两端的距离度量必须一致。** 当查询时使用的函数和索引的 `DISTANCE=` 子句不一致时，MariaDB 会静默地回退到全表扫描。驱动程序用两层来防范这一点：

1. **`ensure_table_sql_for(name, dim)`** 会为生成的迁移 SQL 和 `similar` 里的运行时函数都读取 `self.distance` - 从构造上它们就不可能彼此漂移。
2. **首次调用 `similar` 时的一次运行时检查**，会为每个存储跑一次 `SHOW CREATE TABLE`，从实际的架构里解析出真正的 `DISTANCE=` 子句，如果它和 `with_distance(...)` 不一致，就清楚地报错。结果会被缓存，所以后续调用零成本。这能捕捉到那些绕过了 `ensure_table_sql_for` 的手写迁移，或者 `from_pool` 的搭建方式。

**存储名字的安全性。** 存储名字会被内插进生成的 SQL 里（MySQL 不对标识符做参数化）。名字会按 `[A-Za-z_][A-Za-z0-9_]*`、长度 ≤ 64 来校验；校验通过的名字，随后会在每一条语句里都被反引号包起来。非法的名字，会在 `register`/`upsert`/`similar`/`delete`/`count` 这个边界上，用 `FrameworkError::param` 报错。

**ID 与 metadata。** `VARCHAR(255)` 接受任意的 `String` id - 不需要 UUID 派生，也没有保留的载荷键。metadata 通过 MariaDB 的 `JSON` 列类型往返；`null` 的 metadata 会作为 SQL 的 `NULL` 存储。非对象的 metadata（数组、基本类型）会被用 `FrameworkError::param` 拒绝，以便和 Qdrant、Pinecone 保持一致。

**分数归一化。** MariaDB 返回的是原始的*距离*（越低 = 越接近）。这个 trait 的契约是*分数*（越高 = 越相似） - 驱动程序会按度量做转换：

| 度量 | MariaDB 返回 | 暴露出的 `score` |
| --- | --- | --- |
| 余弦 | `[0, 2]`（`1 - cos`） | `1.0 - d / 2.0` → `[0, 1]` |
| 欧几里得 | `[0, ∞)` L2 范数 | `1.0 / (1.0 + d)` → `(0, 1]` |

在这两种情况下，排序都会被保留（最好的结果排在最前面），但绝对的分数值在驱动程序之间**不能**比较 - 只有顺序才能。每个后端都会落在“越高 = 越好”这个约定上，但取值范围不同：Memory 的余弦返回 `[-1, 1]`，MariaDB 归一化后的余弦返回 `[0, 1]`，Qdrant 用 `[-1, 1]` 给出它原生的余弦相似度，而 Pinecone 则按索引创建时所用的度量，返回原始的相似度。请用 `score` 在单个驱动程序的结果集内部排序；如果不自己重新归一化，就不要跨驱动程序比较分数数值。

**脱围机制。** `driver.pool()` 返回底层的 `sqlx::MySqlPool`，用于这个 trait 没有覆盖到的原始查询。`MariaDbVectorDriver::embedding_to_vec_text`、`score_from_distance` 和 `ensure_table_sql` 都是纯函数，当您把直接 SQL 和经过 trait 路由的调用混用时，可以独立调用它们。

**批量 upsert 的行为。** `upsert` 每 500 行分一个块，为每个块发出一条多行的 `INSERT ... VALUES (...), (...), ...` 语句，全部包在一个单一事务里。加载一份全新的语料时，网络往返次数比逐行插入少了大约 500 倍；这次调用在整个批次上都保持原子性。批次大小是内部实现细节 - 用您所有的条目调用一次 `upsert`，分块由驱动程序处理。

**HNSW 索引在提交时重建。** MariaDB 会在行写入的同时更新 HNSW 图，但索引工作集中发生在提交那一刻。一次 100 万行的 `upsert`，会让事务在整个索引构建期间都保持打开，这可能是几分钟。对于非常大的初始加载，请把语料切成 10k-100k 行一批，反复调用 `upsert`，这样每一批都会提交，并在轮次之间释放锁。（更小的 `upsert` 调用，并不会让每一行更慢 - 它们只是把索引工作分摊到了更多的提交点上。）

**维度在建表时就被钉死了。** `VECTOR(N)` 固定了维度；把嵌入模型从一个 768 维的模型换成一个 1536 维的模型，意味着一次完整的表迁移（新建表、重新嵌入、切换）。请像规划一次架构迁移一样规划模型升级 - 不存在一条 "ALTER COLUMN VECTOR(768) → VECTOR(1536)" 这样的路径。

**连接池大小。** `from_url` 使用 sqlx 默认的 `MySqlPoolOptions` - 在撰写本文时是 `max_connections = 10`。对于高 QPS 的工作负载（每秒几百次 `similar` 调用），请自己用 `MySqlPoolOptions::new().max_connections(N).connect_lazy(url)` 搭建连接池，再传给 `from_pool`。驱动程序不会强加它自己的连接上限。

**本地搭建。** 通过 Docker 运行 MariaDB 11.7+：

```bash
docker run -p 3306:3306 \
    -e MARIADB_ROOT_PASSWORD=secret \
    -e MARIADB_DATABASE=vectors \
    mariadb:11.7
```

集成测试通过下面这样运行：

```bash
MARIADB_URL='mysql://root:secret@localhost:3306/vectors' \
    cargo test -p suprnova --test vector_mariadb -- --ignored
```

## 驱动程序对比

| 方面 | Memory | Qdrant | Pinecone | MariaDB |
| --- | --- | --- | --- | --- |
| 底层存储 | `HashMap` | Qdrant gRPC | Pinecone REST | MariaDB SQL |
| 持久化 | 无 | 是 | 是 | 是 |
| 自动创建 | 不适用 | 是（可配置） | 否（由用户创建索引） | 否（迁移由您负责） |
| 字符串 ID | 原生 | 哈希成 UUID-5 | 原生 | 原生 |
| 保留的 metadata 键 | 无 | `__suprnova_id` | 无 | 无 |
| 吞吐量 | 逐进程 | 并发 | 并发（受连接池限制） | 并发（受连接池限制） |
| 距离度量 | 余弦 | 可配置 | 在创建索引时设定 | 余弦 / 欧几里得 |
| 版本要求 | - | 任意 | 任意 | **11.7+** |

## 运维说明

**存储名字的约定。** 传给 `Vector::register` 和 `Vector::store` 的存储名字是一个标签 - 可以是任意字符串。对 Qdrant，框架会把它当作集合名字来用；对 Pinecone，则当作索引名字。请让这个标签匹配后端已有的命名方案。

**重新注册**一个名字、换上一个新的驱动程序实例，按设计是一个后写入者获胜的操作 - 这在测试装置里替换驱动程序、而不重启进程时很有用。

**测试隔离。** Memory 和依赖注册表的驱动程序测试，都使用打了时间戳标记的唯一存储名字，以避免在并行测试运行时发生冲突。

**错误语义。** 对于未注册的名字，`Vector::store(name)` 会返回 `FrameworkError::not_found`。驱动程序层面的失败（网络、认证、维度不匹配）会以 `FrameworkError::internal` 或者 `FrameworkError::param` 的形式返回，原因字符串会带在 display 消息里。

## 扩展

要添加第五种后端（Weaviate、Milvus、LanceDB、pgvector、LibSQL，……）：

1. 新增一个实现了 `VectorDriver` 的 `framework/src/vector/<backend>.rs`。
2. 从 `framework/src/vector/mod.rs` 和 crate 根重新导出这个驱动程序类型。
3. 照搬 Pinecone 那套测试划分：纯函数测试和传输格式契约测试（针对一个本地的 `wiremock` 伪造实现）总是会跑；集成测试则被 `#[ignore]` 把关在需要凭据的环境变量后面。中间这一层才是真正物有所值的那个 - 一个 CI 触达不到的后端，仍然有一份能被一个笔误打破的传输格式。

这个 trait 特意做得很小，这样发布一个新驱动程序的门槛就能保持很低。如果某个后端需要放不进去的表面（过滤表达式、稀疏向量、混合搜索），就通过驱动程序上的一个脱围机制把它暴露出来 - 不要把这个 trait 撑肥。

## 下一步

- [部署](deployment.md) - MariaDB 作为默认生产建议的完整语境
- [数据库](database.md) - 多驱动程序的 SeaORM 搭建，包括把
  MariaDB 用作向量之外的关系型后端
- [环境变量](env-vars.md) - `QDRANT_URL`、
  `PINECONE_API_KEY`、`MARIADB_URL`，以及其他驱动程序的 env 契约
- [缓存](cache.md) - 形状相同的姊妹门面，用的是同一套驱动程序 trait
- [Laravel 对等映射](parity.md) - 向量搜索相对于 Scout
  所处的位置
