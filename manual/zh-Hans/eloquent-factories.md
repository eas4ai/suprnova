# Eloquent 工厂

工厂为测试和填充器产出随机化的模型实例。这个形态是 Laravel 的：`UserFactory::new().count(10).create_many().await?`。契约是一个 trait 加一个流式构造器，外加一个 `#[derive(Factory)]` 捷径，用于模型本身已经有一份合理的随机化表示这种常见情况。

本章覆盖手写和 derive 两种方式来定义工厂、把覆盖组合成可复用的“状态”、通过 `Sequence` 生成确定性的 ID、撑起 `create` 的那个 `Persistable` 接缝，以及 `make`（存在内存里）和 `create`（持久化）之间的区别。关于工厂最有用的那个测试编写场景，请参见[测试](testing.md)。

## `Factory` trait

这个 trait 恰好只有一个必须实现的方法：

```rust
pub trait Factory {
    type Model;

    fn definition() -> Self::Model
    where
        Self: Sized;
}
```

`definition()` 返回一个完全填充好的模型，每个字段都随机化成某个说得通的默认值。这个 trait 不携带任何逐实例的状态 - 实现者通常是零大小的标记（`struct UserFactory;`），这样调用者就能按名字找到这个工厂，而不必持有一个句柄。

这个 trait 还提供了两个带默认实现的构造器入口：

```rust
fn new() -> FactoryBuilder<Self::Model>;       // count = 1，没有覆盖
fn times(n: usize) -> FactoryBuilder<Self::Model>;  // new().count(n) 的糖
```

您会调用的其他每一个方法（`with`、`count`、`make`、`create`、`create_many`、……）都活在 `FactoryBuilder<M>` 上。

## 手写一个工厂

最小的手写形态，是把一个标记结构体，和一个知道怎么构建单个实例的 `Factory` 实现配成一对。当模型没有 derive `fake::Dummy` 时，您通常会伸手用这个 - 可能是因为某些字段需要确定性的播种（一个已知范围内的关系 ID），或者是因为这份随机化表示需要业务规则的意识：

```rust
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        let now = chrono::Utc::now();
        User {
            // `0` 只是一个占位符 - `persist_via_seaorm` 会在插入之前，
            // 把主键列翻成 `NotSet`，这样数据库才会指派真正的 id。
            id: 0,
            name: format!("Factory User #{}", next_seq()),
            email: format!("factory-{}@example.test", next_seq()),
            password: "factory-placeholder".into(),
            remember_token: None,
            active: true,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

`__eager` 和 `__pivot` 这两个字段，是 `#[suprnova::model]` 宏在每一个 Eloquent 结构体上都注入的预加载和中间表暂存状态。永远用默认值填它们 - 它们是由查询构造器填充的，不是由工厂填充的。

`next_seq()` 可以是您想要的任何东西 - 一个 `static AtomicU64`、一个 `Sequence`（下文会讲），或者一个线程本地的计数器。关键在于，`definition()` 在 `make_many` / `create_many` 内部每次调用都会重新运行，所以您需要的任何唯一性，都必须来自这个函数能够触及的一个计数器。

## 常见情况下的 `#[derive(Factory)]`

当模型本身实现了 `fake::Dummy` 时 - 无论是通过 `#[derive(Dummy)]`，还是手写的 `impl Dummy<Faker> for Model` - 这个 derive 都会把标记结构体 + 实现，收拢成模型上的一行：

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub author_id: i64,
    pub is_public: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

这个 derive 会发出一个作为姊妹类型的 `pub struct PostFactory;`，以及一个 `impl Factory for PostFactory`，它的 `definition()` 会调用 `Faker.fake::<Post>()`。工厂的可见性会照搬模型的可见性 - 一个 `pub` 的模型得到一个 `pub` 的工厂，一个 `pub(crate)` 的模型得到一个 `pub(crate)` 的工厂。

### 覆盖生成出来的名字

默认情况下，`#[derive(Factory)]` 会发出 `<Model>Factory`。通过 `name` 属性来覆盖它：

```rust
#[derive(Dummy, Factory)]
#[factory(name = "AccountFactory")]
pub struct User { /* … */ }
```

这个值必须能解析成一个 Rust 标识符 - `name = "User Factory"` 或者 `name = "user-factory"` 都会编译失败，报出一条清楚的、指着具体位置的错误。这个宏会照字面发出 `pub struct <Name>;`，所以任何不能当类型名的东西，也不能当工厂名。

### 手写 `Dummy` 实现更丰富的随机化

`#[derive(Dummy)]` 对基元类型的结构体管用，但不会给您任何对分布或者跨字段不变式的控制权。对任何非琐碎的场景，请手写 `Dummy` 实现，再配上 `#[derive(Factory)]`：

```rust
use suprnova::__fake::rand::Rng;
use suprnova::__fake::{Dummy, Fake, Faker, faker::lorem::en::{Paragraph, Sentence}};
use suprnova::Factory;

#[derive(Factory)]
pub struct Post { /* fields … */ }

impl Dummy<Faker> for Post {
    fn dummy_with_rng<R: Rng + ?Sized>(_: &Faker, rng: &mut R) -> Self {
        let title: String = Sentence(3..7).fake_with_rng(rng);
        let body: String = Paragraph(3..6).fake_with_rng(rng);
        let author_id: i64 = (1..=50i64).fake_with_rng(rng);
        let now = chrono::Utc::now();

        Post {
            id: 0,
            author_id,
            title,
            body,
            is_public: Faker.fake_with_rng::<bool, _>(rng),
            created_at: now,
            updated_at: now,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

`fake` 这个 crate 被重导出成了 `suprnova::__fake`，这样消费者就不需要在 `Cargo.toml` 里再单独写一行 `fake = "…"`。常用的类型也在 crate 根下被重导出了：`suprnova::{Dummy, Fake, Faker}`。

### 为什么 `#[derive(Factory)]` 只接受朴素结构体

这个 derive 会用一条清楚的编译错误，拒绝枚举、联合体，以及泛型模型。枚举和联合体没有一个有意义的默认表示。泛型会强迫您决定，工厂类型该怎么给它的模型套上类型参数 - 而这没有一个好的默认值，所以这个 derive 拒绝去猜。对这些情况，请手写 `impl Factory`。

## 流式构造器

`Factory::new()` / `Factory::times(n)` 返回一个 `FactoryBuilder<M>`。每一个操作都是可链式的；在您调用一个终结方法（`make`、`make_one`、`make_many`、`create`、`create_one`、`create_many`）之前，什么都不会发生。

### `count(n)` - 要多少个实例

```rust
let user = UserFactory::new().make();             // 1 个 user
let users = UserFactory::new().count(10).make_many();  // 10 个 user
let same = UserFactory::times(10).make_many();   // 一样的结果
```

`count(n)` 会被 `make` / `create` 忽略（永远是一个），而 `make_many` / `create_many` 会遵从它。`times(n)` 只是 `Self::new().count(n)` 的糖，与 Laravel 的 `Factory::times($n)` 一致。

### `with(|m| { … })` - 逐次调用的覆盖

`with` 会注册一个闭包，在 `definition()` 之后，针对每一个产出的实例运行。多次 `with` 调用会按注册顺序组合起来，所以在同一个字段上，后面的覆盖会盖掉前面的：

```rust
let admin = UserFactory::new()
    .with(|u| u.active = true)
    .with(|u| u.role = "admin".into())
    .make();
```

这些覆盖存成的是 `Box<dyn Fn(&mut M) + Send + Sync + 'static>`，这样构造器就能保持 `Send` - 这对异步的 `create` / `create_many` 路径很重要，它们会让这个构造器跨越一次 SeaORM 插入上的 `.await` 存活下来。

### `prepend(|m| { … })` - 调用者仍能覆盖的默认值

`prepend` 会把一个闭包插到覆盖链的**最前面**，所以它会在任何其他 `with(...)` **之前**运行。当您想在一个状态方法里提供一个默认值、又希望调用者仍能用之后的 `.with(...)` 盖掉它时，就用这个：

```rust
impl UserFactory {
    /// 状态方法 - 管理员的默认值，调用者仍能定制。
    pub fn admin() -> suprnova::FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.active = true)
    }
}

// 调用者在 `role` 上会赢，因为他们的 .with() 排在这些 prepend 之后。
let owner = UserFactory::admin()
    .with(|u| u.role = "owner".into())
    .make();
```

这是 Suprnova 对 Laravel `Factory::prependState` 的对应物。对状态方法来说，这正是那个合适的基本操作 - 用 `with` 的话，会输给调用者的 `.with(...)`，这和一个默认值该做的事恰好相反。

### `when(cond, |b| { … })` - 条件式链接

`when` 会把一个标志穿过一条链，而不打破这套流式风格。这个闭包接受构造器，返回构造器。当 `cond` 为假时，这个构造器会原样通过：

```rust
UserFactory::times(10)
    .with(|u| u.active = true)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

对应 Laravel 的 `Conditionable::when($cond, $cb)`。`FnOnce(Self) -> Self` 这个签名意味着，只要您在返回这个构造器之前 `.await`，就可以在闭包内部 `await`。

### 终结方法

| 方法 | 返回 | 会持久化吗？ |
|---|---|---|
| `make()` | 一个 `M` | 不会 |
| `make_one()` | 一个 `M`（强制 count = 1） | 不会 |
| `make_many()` | `count` 个项目组成的 `Vec<M>` | 不会 |
| `create()` | `Result<M, FrameworkError>` | 会 |
| `create_one()` | `Result<M, FrameworkError>`（强制 count = 1） | 会 |
| `create_many()` | `Result<Vec<M>, FrameworkError>` | 会 |

当一个状态方法已经在内部把 `count` 设成了别的值，而调用者恰好只想要一个结果时，`make_one` 和 `create_one` 就很有用：

```rust
pub fn admins_in_org(org_id: i64) -> suprnova::FactoryBuilder<User> {
    UserFactory::times(5)               // 对夹具来说是一个说得通的默认值
        .with(move |u| u.org_id = org_id)
        .with(|u| u.role = "admin".into())
}

// 测试只想要一个 - `create_one` 会丢弃这个 count(5)。
let admin = admins_in_org(42).create_one().await?;
```

## 状态：可复用的预设组合

Suprnova 不提供一个 `state("name")` 查找表。取而代之，状态是您工厂标记上的一些朴素方法，返回一个预先配置好的 `FactoryBuilder<M>`。这个模式靠继承来组合 - 每一个状态方法都返回同一个 `FactoryBuilder<M>` 类型，所以您可以在结果上链接更多方法：

```rust
use suprnova::FactoryBuilder;
use crate::models::users::User;

pub struct UserFactory;

impl suprnova::Factory for UserFactory {
    type Model = User;
    fn definition() -> User { /* … */ }
}

impl UserFactory {
    /// 未激活变体 - 叠加一个 `active: false` 的默认值。
    pub fn inactive() -> FactoryBuilder<User> {
        Self::new().prepend(|u| u.active = false)
    }

    /// 管理员变体 - 叠加角色 + 已验证的邮箱。
    pub fn admin() -> FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.email_verified_at = Some(chrono::Utc::now()))
    }

    /// 可组合：未激活的管理员。
    pub fn inactive_admin() -> FactoryBuilder<User> {
        Self::admin().prepend(|u| u.active = false)
    }
}
```

```rust
// 在调用点也能组合 - 随意链接更多覆盖。
let user = UserFactory::admin()
    .with(|u| u.name = "Alice".into())
    .create()
    .await?;

let batch = UserFactory::inactive().count(20).create_many().await?;
```

选 `prepend` 是有意为之的：一个状态的覆盖是*默认值*，调用者仍能重写它们。如果您想让一个状态的设置不容商榷，就改用 `with` - 它会排到链的末尾，并且胜出。

### 为什么没有 `state("name")` 查找表

一个以名字为键的状态注册表，会把一件编译器本可以检查的事，硬变成运行时的字符串匹配。状态方法给您的是编译期校验（写错成 `UserFactor::admn()` 是一个硬错误）和完整的 IDE 自动补全。可组合性 - 从 `inactive_admin()` 内部链接 `Self::admin()` - 是白得来的。

## 用 `Sequence` 生成确定性的 ID

`Sequence` 是一个单调递增的计数器，用来给那些逐次调用都要唯一的字段播种。每一次 `next()` 调用，都会跨线程原子地返回 1、2、3、……：

```rust
use suprnova::{Fake, Sequence};

static ORDER_IDS: Sequence = Sequence::new();

pub struct OrderFactory;
impl suprnova::Factory for OrderFactory {
    type Model = Order;
    fn definition() -> Order {
        Order {
            id: 0,
            number: format!("ORD-{:06}", ORDER_IDS.next()),
            total_cents: (100..=10_000).fake(),
            created_at: chrono::Utc::now(),
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

`Sequence::new()` 是 `const` 的，所以它能当一个 `static` 初始化器用。这个计数器从 0 开始，在第一次调用时递增到 1。如果您想要一份干净的计数，就在测试之间用 `reset()` - `#[suprnova_test]` 宏不会替您做这件事，因为框架没法知道哪些 sequence 是您的：

```rust
#[suprnova::suprnova_test]
async fn each_order_gets_a_unique_number(db: TestDatabase) {
    ORDER_IDS.reset();   // 让这个测试从 1 开始
    let orders = OrderFactory::new().count(5).create_many().await?;
    assert_eq!(orders[0].number, "ORD-000001");
    assert_eq!(orders[4].number, "ORD-000005");
}
```

`Sequence` 用的是 `SeqCst` 排序 - 对“给我一个唯一 id”这件事来说是过度设计，但它能让推理变得毫不费力。如果某个 Sequence 出现在了热路径上，您可以自己写一个用 `Relaxed` 的版本。

## `Persistable`：通往您存储层的接缝

只要模型实现了 `Persistable`，`create` 这一族方法就可用：

```rust
#[async_trait]
pub trait Persistable: Sized + Send {
    async fn persist(self) -> Result<Self, FrameworkError>;
}
```

`factory::persist` 里的一个兜底实现，覆盖了每一个能 `IntoActiveModel<ActiveModel>` 的 SeaORM 模型 - 也就是 `#[suprnova::model]` 宏发出的每一个模型。不需要逐模型的样板代码；如果 `User` 是一个模型，`UserFactory::new().create()` 就能用。

这个兜底实现会拉取 `DB::connection()` 并插入。返回的 `Self`，就是 SeaORM 在插入之后交回来的那个东西 - 已经指派好的 id、已经解析好的默认列，等等。

### 主键处理

一个 SeaORM 的 `IntoActiveModel` 实现，会把每一个字段 - 包括主键 - 都标成 `Set(value)`。对工厂产出的模型来说，主键是一个占位符（对 `AUTO_INCREMENT i64` 就是 `0`），所以一次直接的插入，在第二次调用时就会撞上一个 UNIQUE 约束失败。

`persist_via_seaorm`（撑起这个兜底实现的帮助函数）会在插入之前，把每一个主键列都翻成 `NotSet`，让数据库去指派它自己的 id - 这正是工厂实际需要的那种语义：

```rust
pub async fn persist_via_seaorm<M, E, C>(model: M, db: &C) -> Result<M, FrameworkError>
where
    M: ModelTrait<Entity = E> + IntoActiveModel<<E as EntityTrait>::ActiveModel> + Send,
    E: EntityTrait<Model = M>,
    /* … bounds … */
    C: ConnectionTrait,
{
    let mut active = model.into_active_model();
    for pk in <<E as EntityTrait>::PrimaryKey as Iterable>::iter() {
        active.not_set(pk.into_column());
    }
    active.insert(db).await.map_err(/* … */)
}
```

如果您确实*想要*指派一个具体的 id（重放测试、按 id 还原一份夹具），就绕开这个帮助函数，直接调用 `model.into_active_model().insert(db).await`。

### 针对一个显式连接持久化

`persist_via_seaorm` 把连接当作一个参数接受。当您想针对一个不是框架绑定的 `DB::connection()` 的连接来驱动持久化时很有用 - 最常见的场景是集成测试里一个具体的 `sqlite::memory:` 句柄：

```rust
use suprnova::factory::persist_via_seaorm;

let model = UserFactory::new().make();
let row = persist_via_seaorm(model, db.inner()).await?;
```

### 自定义的非 SeaORM 后端

因为这个兜底实现的目标是每一个 `ModelTrait` 类型，您没法在一个下游 crate 里写 `impl Persistable for MyOrm::Model` 而不冲突。对于非 SeaORM 的自定义持久化（Redis、Surreal、只存 blob 的存储），请把模型包进一个 newtype，在这个包装类型上实现 `Persistable`：

```rust
use suprnova::{FrameworkError, Persistable};
use suprnova::async_trait;

pub struct RedisCached<T>(pub T);

#[async_trait]
impl Persistable for RedisCached<MyValue> {
    async fn persist(self) -> Result<Self, FrameworkError> {
        let client = suprnova::App::make::<RedisClient>()
            .ok_or_else(|| FrameworkError::internal("redis client not bound"))?;
        client.set(&self.0.key, &serde_json::to_vec(&self.0)?).await?;
        Ok(self)
    }
}
```

这样，一个 `Factory<Model = RedisCached<MyValue>>` 就白得了 `create` / `create_many`。

## `make` 对比 `create`：什么时候用哪个

`make` 返回模型，不碰数据库：

```rust
// 给一个纯函数写的单元测试 - 不需要数据库。
let draft = PostFactory::new().with(|p| p.is_public = false).make();
let snippet = my_lib::extract_summary(&draft);
assert!(snippet.len() < 200);
```

`create` 会持久化，返回插入之后的版本：

```rust
// 集成测试 - 这个 action 需要一行真实的数据。
let post = PostFactory::new().create().await?;
let action = App::resolve::<PublishPostAction>().unwrap();
let published = action.execute(post.id).await?;
assert!(published.is_public);
```

只要测试不关心这一行是否真的存在，就用 `make`。当您会把这一行查回来、当一个外键需要一个真实的 id，或者当您在为一个会读数据库的子系统填充夹具时，就用 `create`。请注意，`create_many` 是顺序持久化的 - 如果后面某次插入失败，前面的插入**不会**被回滚。`create` / `create_many` 走的是 `Persistable` 那个兜底实现，它直接对话框架绑定的 `DB::connection()` - 它们**不会**加入一个环境里的 `DB::transaction(...)` 作用域。如果您需要一批插入具备原子性，就在闭包内部落到 `Model` trait 的 `Model::create(attrs!{...})` 上（那条路径会经过同一个遵从 `CURRENT_TX` 的执行器）：

```rust
use suprnova::{DB, Model, attrs};

DB::transaction(|_tx| Box::pin(async move {
    for i in 0..50 {
        User::create(attrs!{
            name: format!("user-{i}"),
            email: format!("user-{i}@example.test"),
        }).await?;
    }
    Ok::<_, suprnova::FrameworkError>(())
})).await?;
```

## “创建之后”的行为

Suprnova 不提供一个命名为 `after_creating(|m| { … })` 的回调。两个模式覆盖了 Laravel 里那个回调存在的使用场景：

**1. 链条 - 在 `create`/`create_many` 之后做后续工作：**

```rust
let user = UserFactory::new().create().await?;
ProfileFactory::new()
    .with(move |p| p.user_id = user.id)
    .create()
    .await?;
```

当一个模型的 id 需要流进一次后续插入时，这是那个典范模式。`create` 会返回持久化之后的那一行，所以这个 id 立刻就能拿到。

**2. 模型观察者 - 对模型生命周期做出反应，而不是对工厂：**

用[模型观察者](eloquent.md#observers)把插入之后的行为接到模型本身上，而不是接到工厂上。这个观察者会为 `User::create(...)`、`UserFactory::new().create()`，以及任何其他持久化路径触发 - 当您想要的行为是“每次这一行落地，就做 X”时，这正是您想要的：

```rust
use suprnova::{FrameworkError, Observer, async_trait, observer};

#[observer(User)]
pub struct AuditUser;

#[async_trait]
impl Observer<User> for AuditUser {
    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user_id = user.id, "user created");
        Ok(())
    }
}
```

只在工厂上生效的回调，会招来测试插入和真实插入之间的分歧。观察者在两者之间都能保持一致。

## 填充器

工厂产出实例；填充器负责编排它们。一个 `Seeder` 是一个零大小的类型，带着一个知道要填充什么的异步 `run`：

```rust
use suprnova::{Factory, FrameworkError, Seeder};
use suprnova::async_trait;

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // 先建 users - posts 引用的是 1..=50 范围内的 user id。
        UserFactory::new().count(50).create_many().await?;
        PostFactory::new().count(200).create_many().await?;
        Ok(())
    }
}
```

在 `bootstrap.rs` 里注册这个填充器，这样每个项目的 `console` 二进制文件的 `db:seed` 命令才会知道它：

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

通过项目的 `console` 二进制文件来运行（每一个脚手架生成的应用，都会在 `src/bin/console.rs` 发布一个）：

```bash
cargo run --bin console -- db:seed
```

填充器按注册顺序运行。幂等性是填充器自己的责任 - `run` 不会做快照，也不会回滚，所以一个无条件插入的填充器，重新运行时会产出重复数据。想要一个干净的起点，就先 `migrate:fresh`，再 `db:seed`。

## 整合起来：一份完整的测试夹具

```rust
use suprnova::{App, describe, test, expect};
use suprnova::events::{EventFacade, assert_dispatched_times};
use suprnova::testing::TestDatabase;
use crate::factories::{PostFactory, UserFactory};
use crate::actions::publish_post::PublishPostAction;

describe!("PublishPostAction", {
    test!("publishes a draft post", async fn(db: TestDatabase) {
        // 准备 - 一个作者，以及一篇归属于他们的草稿文章。
        let author = UserFactory::new()
            .with(|u| u.active = true)
            .create()
            .await
            .unwrap();

        let draft = PostFactory::new()
            .with(move |p| p.author_id = author.id)
            .with(|p| p.is_public = false)
            .create()
            .await
            .unwrap();

        // 执行。
        let action = App::resolve::<PublishPostAction>().unwrap();
        let published = action.execute(draft.id).await.unwrap();

        // 断言。
        expect!(published.is_public).to_equal(true);
        expect!(published.author_id).to_equal(author.id);
    });

    test!("publishing emits exactly one event", async fn(db: TestDatabase) {
        let _guard = EventFacade::fake();
        let post = PostFactory::new().create().await.unwrap();

        App::resolve::<PublishPostAction>().unwrap()
            .execute(post.id).await.unwrap();

        assert_dispatched_times::<crate::events::PostPublished>(1);
    });
});
```

有三个模式值得指出：

- 作者的 `id` 通过 `.with(...)` 内部的一个 `move` 闭包，流进了这篇文章里。捕获是显式的，这让这个关系在调用点保持可见。
- `create().await.unwrap()` 是测试里的惯用写法 - 测试被允许在准备阶段失败时 panic，因为一份坏掉的夹具就是一个坏掉的测试，不是一种得体的失败模式。
- 工厂能和测试表面的其他部分组合起来（`EventFacade::fake`、`Storage::fake`、`Mail::fake`、……） - 这些伪造实现都不知道工厂的存在，但您写的每一个测试都会把它们一起用上。

### 为什么 Suprnova 有所不同

Laravel 的工厂自带具名状态（`->state('admin')`）、运行时序列（`->sequence(['name' => 'A'], ['name' => 'B'])`），以及一个注册在工厂本身上的 `afterCreating` 回调。Suprnova 把这三者都去掉了，换成了 Rust 形状的基本构件：

- **状态是方法，不是字符串。** 编译期的错字检查和 IDE 自动补全都是白得的；唯一的代价是“您写的是 `pub fn admin()` 而不是 `protected function admin()`”，这根本不算代价。
- **序列是一个独立的基本构件。** `Sequence` 只做一件事（原子计数器），而且能在工厂表面之外复用 - 您可以把它扔进一个请求 id 生成器、一个工作流步骤计数器，或者一个测试装置里，都不需要解释它是什么。
- **“创建之后”是接到模型上的，不是接到工厂上的。** 框架已经有[模型观察者](eloquent.md#observers)专门做这件事。在工厂上再加一套并行的机制，会让测试时的行为和生产环境的行为，从构造上就产生分歧。

这套流式表面 - `count(10)`、`times(10)`、`with`、`prepend`、`when`、`make`、`create`、`create_many`、`make_one`、`create_one` - 直接对应着 Laravel 的那一套，所以肌肉记忆可以直接迁移过来，不需要一份术语表。

## 下一步

- [测试](testing.md) - `#[suprnova_test]`、`TestDatabase`，以及和工厂构建出来的夹具搭配使用的那些伪造门面。
- [Eloquent](eloquent.md) - 模型派生、观察者，以及 `create` 持久化您工厂产出时会运行的那条转换管道。
- [迁移](migrations.md) - 您的工厂需要针对之运行的那份架构；用 `migrate:fresh && db:seed` 来获得一个干净的夹具起点。
- [数据库](database.md) - `DB::transaction`、多连接路由、保存点 - 当 `create_many` 需要原子性时，该伸手去用的东西。
- [服务容器](container.md) - `App::resolve` 和 `App::make` 是怎么找到您测试里、和工厂一起被调用的那些操作和服务类型的。
