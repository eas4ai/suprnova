# Eloquent 转换、访问器和修改器

一个转换调解的是一列在磁盘上存的东西，和您模型在内存里携带的东西之间的边界。一个访问器会从您已经有的列里，凭空造出一个虚拟属性。一个修改器会把对一个字段的写入，路由经过您自己的转换。连同自动管理的时间戳，它们就是把一行扁平数据，变成一个类型化 Rust 值的四个活动部件。

本章覆盖完整的转换表面（每一种内置类型、`casts!` 运行时覆盖、加密与哈希）、`#[accessor]` 和 `#[mutator]` 这两个属性宏、包括 `touch()` 和 `without_touching` 的自动时间戳契约，以及当您用 `replicate()` 克隆一个模型时会触发的那个 `Replicating` 生命周期事件。

关于更广的模型表面（`#[suprnova::model]`、查询构造器、关系、观察者），请参见 [Eloquent API](eloquent.md) 一章。关于端到端的生命周期事件，请参见[事件与监听器](events.md)。关于加密转换所用的加密门面，请参见[加密](encryption.md)。

## 转换是怎么工作的

每一个转换都是一个实现了 `Cast` trait 的结构体：

```rust
pub trait Cast: Send + Sync {
    type Runtime;
    type Storage;

    fn to_storage(value: &Self::Runtime) -> Result<Self::Storage, FrameworkError>;
    fn from_storage(stored: &Self::Storage) -> Result<Self::Runtime, FrameworkError>;
}
```

`Runtime` 是您写在模型结构体里的那个 Rust 类型（`bool`、`chrono::NaiveDate`、`rust_decimal::Decimal`、您自己的枚举）。`Storage` 是 SeaORM 在这一列上看到的类型（对一个 SQLite 布尔列是 `i64`，对一个 TEXT 日期是 `String`）。两个方向都是可能失败的 - 时间和小数的解析都可能拒绝格式错误的输入 - 所以这个宏会把 `Result` 一路传播经过 `From<inner::Model>` 和 `ActiveModel` 的写入路径。

转换是显式的。一个 `Vec<String>` 字段不会隐式地变成 `AsArray<String>`，因为在宏展开时做字段类型检查，会在您改了一个别名的名字，或者导入了一个不同的 `Vec` 的那一刻就崩掉。您要在宏属性上声明转换：

```rust
use suprnova::{model, AsArray, AsBool, AsJson};

#[model(
    table = "posts",
    casts = {
        tags = AsArray<String>,
        published = AsBool,
        metadata = AsJson<serde_json::Value>,
    },
)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub tags: Vec<String>,
    pub published: bool,
    pub metadata: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

这个宏会把每一条 `field = CastType` 项，展开成每次读写都会调用的 `Cast::to_storage` 和 `Cast::from_storage`。您从不自己调用这个转换 - 您写的是运行时类型，转换负责把这一列的形态接好线。

### 为什么 Suprnova 有所不同

Laravel 把转换声明成 `protected $casts = ['tags' => 'array']`。字符串 `'array'` 是通过一次运行时查找才解析成一个类的，这意味着转换的名字，在运行之前都是活在无类型的字符串里的。Suprnova 直接拿类型本身 - `AsArray<String>` 是一个真正的 Rust 类型，宏会在编译期检查它。转换名字打错字是一个编译错误，不是部署三周之后才出现的运行时异常。

## 基元转换

五个转换覆盖了 SQL 的标量类型。

### `AsBool`

`bool` ↔ `INTEGER`（0 / 1）。SQLite 没有原生的布尔列；Postgres 和 MySQL 都能通过 SeaORM 的 `Value::Int` 边界，干净地对 `i64` 做往返转换。单一的存储形态，让您能对每一个后端都用同一个转换。

```rust
#[model(table = "settings", casts = { dark_mode = AsBool })]
pub struct Settings {
    pub id: i64,
    pub dark_mode: bool,
}
```

### `AsInt<I>`

一个更窄的整数（`i32`、`u32`、`i16`）↔ `i64`。SeaORM 在这一列上把整数存成 `i64`；这个转换在读取时收窄，在写入时放宽。超出范围的值会在读取时产出一个验证错误，而不是静默截断。

```rust
#[model(table = "counters", casts = { age = AsInt<u32> })]
pub struct Counter {
    pub id: i64,
    pub age: u32,
}
```

当运行时类型已经和存储匹配时，就用 `AsInt<i64>`（或者干脆省掉这个转换）。

### `AsFloat`

`f64` ↔ `REAL`。两个方向都是直通的 - 这个转换存在，是为了在命名上和 Laravel 的 `'float'` 转换对等；各个后端原生就能对浮点数做往返转换。

### `AsString`

`String` ↔ `TEXT`。同样是直通的；这个转换存在，是为了让 `Builder::with_casts(...)` 这个运行时覆盖，能像对待其他每一个转换一样，把它抹除成一个 `DynCast`。

### `AsDecimal<P>`

`rust_decimal::Decimal` ↔ `TEXT`。`P` 是精度（小数位数）；值在存储的路上会被四舍五入到 `P` 位。默认是 `P = 4`。存储是一个固定格式的字符串，所以往返转换与后端无关 - SeaORM 原生的 `Decimal` 列类型，在每个驱动程序上有不同的精度语义，而这个字符串往返避开了这一点。

```rust
use rust_decimal::Decimal;
use suprnova::AsDecimal;

#[model(
    table = "ledger",
    casts = { amount = AsDecimal<2> },  // 货币，2 位小数
)]
pub struct LedgerEntry {
    pub id: i64,
    pub amount: Decimal,
}
```

## 时间性转换

六个转换覆盖日期、日期时间、不可变变体，以及 Unix 时间戳。除了时间戳之外的每一个转换，都以 `TEXT`（ISO-8601 / RFC-3339）存储，这样往返转换在每一个驱动程序上都能用 - SQLite 原生就把日期时间存成字符串，Postgres / MySQL 会通过 SeaORM 的 `Value::String` 边界接受它们。

### `AsDate`

`chrono::NaiveDate` ↔ `TEXT`（`YYYY-MM-DD`）。

```rust
use chrono::NaiveDate;
use suprnova::AsDate;

#[model(table = "people", casts = { birthday = AsDate })]
pub struct Person {
    pub id: i64,
    pub birthday: NaiveDate,
}
```

### `AsDateTime`

`chrono::DateTime<Utc>` ↔ `TEXT`（RFC-3339）。当您想要一个挂钟时间的表示时，这是给任意时间戳用的默认转换。

写入会规范化为RFC-3339。读取也接受PostgreSQL生成的原生`CURRENT_TIMESTAMP`文本，以及不带时区的SQLite/MySQL值；不带时区的值按UTC解释。`AsImmutableDateTime`和`AsOptionalDateTime`使用同一个解析器。

### `AsImmutableDate` 和 `AsImmutableDateTime`

存储形态和 `AsDate` / `AsDateTime` 一样。Rust 的借用检查器已经通过 `&` 引用强制了不可变性，所以这些转换共享底层类型 - 它们存在，是为了和 Laravel 的 `immutable_date` / `immutable_datetime` 对等，也是为了在模型声明的地方记录意图。

### `AsOptionalDateTime`

`Option<DateTime<Utc>>` ↔ `Option<String>`。由 `#[model(soft_deletes)]` 这个标志自动注入，用于那个可空的墓碑列（默认是 `deleted_at` - 参见[软删除](eloquent.md#deleting-and-soft-deletes)）。这个被包装的 option，让存储列保持可空，这样软删除的行和存活的行，就能靠 `IS NULL` 来区分，不需要一个哨兵值。

对任何其他您想以 RFC-3339 文本形式往返转换的可空日期时间列，直接使用这个转换：

```rust
#[model(
    table = "subscriptions",
    casts = { cancelled_at = AsOptionalDateTime },
)]
pub struct Subscription {
    pub id: i64,
    pub cancelled_at: Option<chrono::DateTime<chrono::Utc>>,
}
```

### `AsTimestamp`

Unix 纪元的 `i64` ↔ `INTEGER`。当这一列会被当作数值范围来查询，或者用在算术运算里时使用。它和 `AsDateTime` 不同 - 当您想要 `WHERE created_unix > 1700000000` 时选 `AsTimestamp`，当您想要日志里的 RFC-3339 字符串时选 `AsDateTime`。

## 结构化转换

五个转换覆盖集合、结构体，以及任意的 JSON。它们全都会把运行时的值序列化成 JSON 文本，存进一个 `TEXT` 列。Postgres 原生的 `JSON` / `JSONB`，和 MySQL 的 `JSON` 列，都接受同样的字符串载荷 - 如果您想要一个原生的 JSON 列类型来建索引，就在一次迁移里手动声明它；这个转换层不会限制列的类型。

### `AsArray<T>`

`Vec<T>` ↔ 经过 JSON 编码的 `TEXT`。元素类型必须是 `Serialize + DeserializeOwned`。

```rust
use suprnova::AsArray;

#[model(table = "posts", casts = { tags = AsArray<String> })]
pub struct Post {
    pub id: i64,
    pub tags: Vec<String>,
}
```

### `AsObject<T>`

一个 `Serialize + DeserializeOwned` 的结构体 ↔ 经过 JSON 编码的 `TEXT`。当运行时的形态是一份键在静态上就已知的固定记录时使用。

```rust
use serde::{Deserialize, Serialize};
use suprnova::AsObject;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    pub theme: String,
    pub notifications: bool,
}

#[model(table = "users", casts = { prefs = AsObject<Prefs> })]
pub struct User {
    pub id: i64,
    pub prefs: Prefs,
}
```

### `AsCollection<T>`

`Collection<T>` ↔ 经过 JSON 编码的 `TEXT`。是 `AsArray` 上的一层薄包装，经由 Suprnova 的 `Collection<T>`（一个带着 Laravel 风格切片表面的 `Vec<T>` newtype - 参见[集合](eloquent.md#collections)）做往返转换。

### `AsJson<T>`

任何 `Serialize + DeserializeOwned` 类型 ↔ 经过 JSON 编码的 `TEXT`。当这个字段是一个 `serde_json::Value`，或者一个用 serde 的说法已经能完全描述、但又不适合 `AsObject` 那种固定形态模式的用户自定义结构体时使用（例如枚举载荷、无类型的映射）。

### `AsArrayObject<T>`

`IndexMap<String, T>` ↔ 经过 JSON 编码的 `TEXT`。当运行时的形态是一个动态键的映射、而键的顺序又很重要时使用（标签在 UI 上的顺序、一个配置块的规范顺序）。选 `IndexMap` 而不是 `HashMap` 是故意的：serde 会通过 `IndexMap` 保留插入顺序，出于同样的原因，Suprnova 的 `serde_json` 已经配置了 `preserve_order`。

对固定形态的记录，用 `AsObject`；对数组，用 `AsArray`。

## 枚举转换

### `AsEnum<E>`

`E: FromStr + AsRef<str>` ↔ `TEXT`。落进这一列的，是这个枚举的变体名（或者它经 `AsRefStr` 定制过的字符串）。框架不会把您锁定在 `strum` 上，但它是拿到这两个约束、又不必手工实现它们的最省事的办法：

```rust
use suprnova::AsEnum;

#[derive(Debug, Clone, Copy, strum::EnumString, strum::AsRefStr)]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

#[model(
    table = "users",
    casts = { role = AsEnum<Role> },
)]
pub struct User {
    pub id: i64,
    pub role: Role,
}
```

整数判别值的存储方式，故意没有被定为默认值。一个 `Role::Admin = 0`，在重新排序之后变成 `Role::Admin = 2`，会静默地把数据库里每一个管理员都换掉。变体名在数据库浏览器里是自解释的，而且在重新排序之后依然稳定。

## 加密与哈希

五个转换在存储边界上调解密码学变换。全部四个 `AsEncrypted*` 转换，共享同一个 [`Crypt`](encryption.md) 门面 - 在它们任何一个运行之前，这个门面都必须先被初始化。生产环境的应用是通过 `Server::from_config`（它会从环境里读取 `APP_KEY`）拿到这一点的；测试则是在启动时调用一次 `suprnova::testing::install_test_encryption_key()`。

### `AsEncrypted`

`String` ↔ 经 AES-256-GCM 加密的 `String`。磁盘上的这一列，存的是 `nonce || ciphertext_with_tag` 的 URL 安全 base64 编码。每一次写入都用一个全新的随机 nonce，所以对同一份明文的两次写入，会产出不同的密文 - 您的数据库管理员没法在静态数据里认出重复的机密信息。

```rust
use suprnova::AsEncrypted;

#[model(
    table = "secrets",
    casts = { api_key = AsEncrypted },
)]
pub struct Secret {
    pub id: i64,
    pub api_key: String,  // 运行时就是纯 UTF-8
}
```

运行时的值就是解密之后的 UTF-8 字符串；您像对待任何其他 `String` 一样读写它。

### `AsEncryptedArray<T>` / `AsEncryptedObject<T>` / `AsEncryptedCollection<T>`

`Vec<T>` / `T` / `Collection<T>` ↔ 经 AES-256-GCM 加密的 JSON。管道是：序列化成 JSON → 加密 → base64 → 存储；读取时反过来。元素/值类型必须是 `Serialize + DeserializeOwned`。

```rust
use suprnova::AsEncryptedObject;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize)]
pub struct CardOnFile {
    pub last4: String,
    pub exp_month: u8,
    pub exp_year: u16,
}

#[model(
    table = "billing",
    casts = { card = AsEncryptedObject<CardOnFile> },
)]
pub struct Billing {
    pub id: i64,
    pub card: CardOnFile,
}
```

### 密钥轮换

`Crypt` 门面通过 `APP_KEY_PREVIOUS` 支持轮换：加密永远用 `APP_KEY`，但解密会先试 `APP_KEY`，如果这个主密钥失败了，就回退到 `APP_KEY_PREVIOUS`。一种滚动式的重新加密策略是：把 `APP_KEY` 设成新密钥，把旧密钥挪到 `APP_KEY_PREVIOUS`，然后对每一个加密过的行调用 `save()`，把密文在新密钥下重写一遍。这个转换层不需要知道轮换这件事 - 它在每次读写时都会经由 `Crypt` 做往返转换，所以一次 `User::all().await?`，接上保存每一行，就能就地迁移这一列。完整的轮换协议请参见[加密](encryption.md)。

### `AsHashed`

`String` ↔ 写入时用当前生效的哈希驱动程序（`HASH_DRIVER` 环境变量 - 默认是 bcrypt，也支持 argon2i 和 argon2id）哈希出来的字符串。运行时的值*就是*这个哈希后的字符串；没有反方向。对应 Laravel 的 `hashed` 转换。

```rust
use suprnova::AsHashed;

#[model(
    table = "users",
    casts = { password = AsHashed },
)]
pub struct User {
    pub id: i64,
    pub password: String,
}
```

`AsHashed::to_storage` 是**幂等的**：一个看起来已经像任何一种可识别哈希（bcrypt 的 `$2*$`，argon2i / argon2id 的 PHC 格式）的值，会原样直通。没有这层防护，`User::find(id).await?.save().await?` 就会把已有的哈希再哈希一遍，变成一个哈希的哈希，弄坏 `Hash::check(plain, stored)`，让每一个已有的密码都失效。

当您需要在写入时做的不只是哈希 - 例如在哈希之前规范化空白字符，或者拒绝空密码 - 就把 `AsHashed` 和下面的 `#[mutator]` 模式配对使用。

## 运行时转换覆盖 - `casts!` 宏

在 `#[model(casts = { ... })]` 里声明的转换是静态的 - 它们会在这个模型的每一次读取上触发。当您在单次查询上需要一个不同的转换时（一个调试工具想要原始的存储形态，一个导出脚本想要不同的 JSON 表示），就用 `Builder::with_casts(...)`：

```rust
use suprnova::{casts, AsDate, AsJson, User};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

`casts!` 宏会构建一个 `HashMap<&'static str, Arc<dyn DynCast>>`。每一项都是 `field_name = CastType`；每一个内置的转换都实现了 `IntoDynCast`，所以这个类型擦除后的 `DynCast` 影子是自动的。这个运行时覆盖映射，只在这条链式查询期间生效 - 模型的静态转换管道不会受影响。

请节制地使用这个表面。对您想要每次读取都应用的转换，模型属性才是正确的地方；运行时覆盖是给一次性查询用的脱围机制。

## 访问器 - 从真实列变出虚拟属性

一个访问器，是模型上一个标了 `#[accessor]` 宏的 `impl` 方法。当您把这个方法的名字列进 `#[model(appends = [...])]` 时，模型的 `to_json()` 就会调用这个方法，把结果插入到这个键下面。

```rust
use suprnova::{accessor, model, Model};

#[model(
    table = "users",
    appends = ["full_name"],
)]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
}

impl User {
    #[accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }
}
```

现在，一次 `serde_json::to_value(&user)`（或者 `user.to_json()`）会包含：

```json
{
  "id": 1,
  "first_name": "Alice",
  "last_name": "Xu",
  "full_name": "Alice Xu"
}
```

这个方法也可以直接调用（`user.full_name()`） - `#[accessor]` 宏基本上只是一个标记，好让结构体级别的 `#[suprnova::model]` 宏能把 `to_json()` 的分发接好线。从您自己的代码里调用它没有任何代价。

`appends` 里的每一个名字，都必须按标识符匹配上一个真实的 `#[accessor]` 方法。写错字（方法是 `full_name`，却写成 `appends = ["fullName"]`）会在编译期被抓住，报出一条指着具体位置的错误消息。

### 返回非 `String` 的值

访问器可以返回任何 `Serialize` 类型。这个宏会在插入之前，把返回的值经过 `serde_json::to_value` 转换一遍，所以：

```rust
impl Post {
    #[accessor]
    pub fn word_count(&self) -> usize {
        self.body.split_whitespace().count()
    }
}
```

在 JSON 输出里会渲染成 `"word_count": 42`。

### 隐藏源列

当访问器的值才是消费者应该看到的东西，而底层的列只是噪声时，就把 `appends` 和 `hidden` 配对使用：

```rust
#[model(
    table = "users",
    appends = ["full_name"],
    hidden = ["first_name", "last_name"],
)]
```

`hidden` 会把指名的列从序列化输出里剥掉；`appends` 接着插入访问器的值。这个顺序是固定的 - 过滤器先运行，访问器的注入后运行。完整的表面请参见[隐藏、可见与 appends](eloquent.md#mass-assignment)。

## 修改器 - 把写入路由经过您的转换

修改器是写入侧的对应物。当一个字段的名字出现在 `#[model(mutators = [...])]` 里时，每一条批量赋值路径（`create` / `update`）都会把这个值路由经过 `self.set_<field>(value)?`，而不是直接赋值给这个字段。

```rust
use serde_json::Value;
use suprnova::{model, mutator, FrameworkError, Model};

#[model(
    table = "users",
    fillable = ["password"],
    mutators = ["password"],
)]
pub struct User {
    pub id: i64,
    pub password: String,
}

impl User {
    #[mutator]
    pub fn set_password(&mut self, value: Value) -> Result<(), FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            FrameworkError::validation("password", format!("{e}"))
        })?;
        // 规范化 + 哈希；AsHashed 自己就会做这个哈希，
        // 但修改器是您还能在这里强制执行策略的地方。
        let trimmed = raw.trim().to_string();
        if trimmed.len() < 12 {
            return Err(FrameworkError::validation(
                "password",
                "must be at least 12 characters",
            ));
        }
        self.password = suprnova::hashing::hash(&trimmed)?;
        Ok(())
    }
}
```

`set_password` 接受一个 `serde_json::Value`。这个方法体拥有反序列化 + 转换的过程 - 结构体上的字段类型可以保持 `String`，您的验证会在这一列被触碰之前运行。返回的一个错误，会以 `bad_request` 的形式经过 `create()` / `update()` 传播出去。

直接对字段赋值会绕开这个修改器：

```rust
user.password = "raw".to_string();  // 跳过 set_password
user.save().await?;                 // 保存的是 "raw"
```

这与 Laravel 的 `$user->password = ...` 对比 `$user->fill(...)` 的行为一致。当您想让这个修改器成为唯一路径时，就把所有写入都路由经过 `attrs!` + `create` / `update`。

### 把修改器和转换结合起来

一个修改器和一个转换可以共存在同一个字段上；修改器运行在写入路径上（当 `create` / `update` 被调用时），转换运行在读取路径上（当这一列从一次 SELECT 具体化出来时）。一个常见的模式是：用 `AsHashed` 来保证读取侧的幂等性，用修改器做写入侧的验证 - 修改器负责哈希，`AsHashed` 看到一个已经哈希过的值，就直接通过。

## 自动管理的时间戳

当一个模型同时带着 `created_at` 和 `updated_at` 字段（类型是 `chrono::DateTime<chrono::Utc>`）时，这个宏会：

- 在 `create()` 时，把两者都设成 `Utc::now()`。
- 在每一次 `save()` 和 `update(attrs)` 时，都推进 `updated_at`。
- 发出一个 `impl Touchable for YourStruct`，这样您就能调用 `.touch().await`，在不改动任何其他列的情况下推进 `updated_at`。

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model, Touchable};

#[model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// 在不做其他改动的情况下推进 updated_at：
let post = Post::find_or_fail(1).await?;
post.touch().await?;
```

存储用的是这个宏为时间戳列自动注入的 `AsDateTime` 转换。这个转换让同一个 `DateTime<Utc>` 值，能在全部三个 SeaORM 驱动程序（SQLite、MySQL、PostgreSQL）上往返转换，不强迫您去选一个数据库专属的时间戳类型。

### 退出选项与自定义列名

`#[model(timestamps = false)]` 会彻底关掉这套自动管理 - 时间戳由您自己控制。

`#[model(created_at = "creado_en", updated_at = "actualizado_en")]` 会保留这套自动管理，但给列改名。这个宏会检测到改了名的字段，把同样的逻辑接到它们上面。

当这个结构体只有这两个时间戳字段中的*一个*时，这个宏会发出一个 `compile_error!` - 这几乎总是一个打错的字（`craeted_at`），您会想让它醒目地浮现出来，而不是被静默吞掉。

### `without_touching` - 任务范围的抑制

有时候您想更新一行，却不想推进 `updated_at` - 运行一次回填，修一个错字，记录一次不该重置那些以 `updated_at` 为键的缓存 TTL 的内部同步。把这些工作包进 `without_touching` 里：

```rust
use suprnova::eloquent::without_touching;

without_touching(async {
    for post in Post::query().get().await? {
        post.touch().await?;  // 在这个作用域内部是空操作
    }
    Ok::<_, suprnova::FrameworkError>(())
}).await?;
```

这个标志是一个 `tokio::task_local!`，所以它不会跨越 `tokio::spawn` 的边界泄漏 - 其他任务上的并发请求，会继续遵从它们自己的作用域（或者没有作用域这件事）。这是 Suprnova 对 Laravel `Model::withoutTouching(closure)` 的对应物。

### 为什么 Suprnova 有所不同

Laravel 用的是一个静态的 `$timestamps = false` 属性，和一个由实例计数器撑起的全局 `Model::withoutTouching` 静态方法。这两种做法都假定了每进程一个请求的隔离性。Suprnova 在一个 Tokio 运行时上跑很多个请求，所以一个进程全局的标志，会让一个请求悄无声息地抑制另一个请求上的时间戳。这个 `tokio::task_local!` 作用域是异步感知的：它会在同一个任务内部，跨越 `.await` 点跟着 future 走，并且在这个 future 被丢弃时离开作用域 - 无论这个请求是怎么结束的。

## `Replicating` 生命周期事件

在全部 16 个模型生命周期事件里（参见[观察者与生命周期事件](eloquent.md#observers-and-lifecycle-events)），`Replicating` 是当您通过 `replicate()`，把一行已有的数据克隆成一份未保存的内存副本时会触发的那一个：

```rust
let original = Post::find_or_fail(1).await?;
let mut copy = original.replicate().await?;  // 未保存
copy.title = format!("{} (copy)", original.title);
copy.save().await?;  // 现在带着一个新的主键持久化了
```

`Replicating` 事件会在这份内存克隆构建*之后*、但在您有机会改动它*之前*触发。监听器接收到的是 `(&Self, Arc<Mutex<Self>>)` - 原件，以及藏在一个 `Mutex` 后面的、刚构建出来的副本，这样您就能在用户看到它之前，从监听器里改动这份副本：

```rust
use suprnova::{Listener, FrameworkError};

pub struct ResetReplicatedFlags;

#[async_trait::async_trait]
impl Listener<post::events::Replicating> for ResetReplicatedFlags {
    async fn handle(&self, event: &post::events::Replicating) -> Result<(), FrameworkError> {
        let mut replica = event.replica.lock().await;
        replica.published = false;       // 副本一开始是未发布的
        replica.view_count = 0;          // 计数器重置
        Ok(())
    }
}
```

在监听器运行的时候，这个副本的主键已经被清空了 - `replicate()` 会在触发这个事件之前调用 `reset_primary_key()`，所以您不会不小心用原来的 ID 重新保存。时间戳也会被重置；`created_at` / `updated_at` 会在随后的 `save()` 上触发，就像任何新的一行一样。

### `replicate_into<T>` - 跨类型复制

当副本是一个不同的类型时（比如 `Post` → `Draft`），就用 `replicate_into::<Draft>()`。`Replicating` 事件在这条路径上*不会*触发，因为这个事件结构体是按源类型区分的，一个注册给 `post::events::Replicating` 的监听器，会收到一个 `Arc<Mutex<Post>>`，不是 `Arc<Mutex<Draft>>`。这条跨类型的路径，是给那些您想要一个全新的目标类型、又不想被观察者干扰的场景用的；如果您想在构造时挂一个钩子，就在目标类型上注册一个普通的 `Creating` 监听器。

关于这套复制表面剩下的部分（`replicate_except`、副本的关系处理、可空主键的规则），请参见[复制](eloquent.md#replication)。

## 整合起来

一个用上了本章每一个表面的模型：

```rust
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use suprnova::{
    accessor, hashing, model, mutator, AsBool, AsDateTime,
    AsDecimal, AsEncryptedObject, AsEnum, AsHashed, AsJson,
    AsOptionalDateTime, FrameworkError, Model,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardOnFile {
    pub last4: String,
    pub exp_month: u8,
    pub exp_year: u16,
}

#[derive(Debug, Clone, Copy, strum::EnumString, strum::AsRefStr)]
pub enum Role {
    Admin,
    Editor,
    Viewer,
}

#[model(
    table = "users",
    soft_deletes,
    appends = ["display_name"],
    hidden = ["password", "card"],
    fillable = ["name", "email", "password", "role", "credit"],
    mutators = ["password"],
    casts = {
        role = AsEnum<Role>,
        verified = AsBool,
        credit = AsDecimal<2>,
        card = AsEncryptedObject<CardOnFile>,
        metadata = AsJson<serde_json::Value>,
        password = AsHashed,
        last_login_at = AsOptionalDateTime,
    },
)]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub password: String,
    pub role: Role,
    pub verified: bool,
    pub credit: Decimal,
    pub card: CardOnFile,
    pub metadata: serde_json::Value,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    // deleted_at 由 soft_deletes 自动注入（AsOptionalDateTime）
}

impl User {
    #[accessor]
    pub fn display_name(&self) -> String {
        if self.name.is_empty() { self.email.clone() } else { self.name.clone() }
    }

    #[mutator]
    pub fn set_password(&mut self, value: Value) -> Result<(), FrameworkError> {
        let raw: String = serde_json::from_value(value).map_err(|e| {
            FrameworkError::validation("password", format!("{e}"))
        })?;
        let trimmed = raw.trim().to_string();
        if trimmed.len() < 12 {
            return Err(FrameworkError::validation(
                "password",
                "must be at least 12 characters",
            ));
        }
        // 修改器负责哈希；AsHashed 在后续的保存里，
        // 看到一个已经哈希过的值，就会原样直通。
        self.password = hashing::hash(&trimmed)?;
        Ok(())
    }
}
```

这一份声明就能给您：

- 八个类型化的转换，把存储/运行时边界接好线。
- 一个从既有列合成出 `display_name` 的访问器。
- 一个校验并哈希密码的修改器。
- 自动管理的 `created_at` / `updated_at`。
- 带着自动注入的 `deleted_at` 列的软删除。
- 带着密钥轮换支持的、加密的存档卡片存储。

每一个转换都会在编译期被检查。这个双 API 查询构造器（参见 [Eloquent - 查询构造器](eloquent.md#query-builder--dual-api)）针对的是这些类型化的列运行的；到 Inertia / JSON 的序列化，会应用 hidden / appends 规则；而一次 `User::find(id).await?`，会通过八次 `Cast::from_storage` 调用来具体化这一行，不需要您写一行转换代码。

## 下一步

- [Eloquent API](eloquent.md) - 模型表面剩下的部分：查询构造器、关系、观察者、分页、事务。
- [加密](encryption.md) - 这些加密转换共享的 `Crypt` 门面、密钥轮换协议，以及更广的加密表面。
- [事件与监听器](events.md) - `Replicating` 和另外 15 个模型生命周期事件背后的那个派发器。
- [认证](authentication.md) - `Authenticatable` trait，以及 `AsHashed` 在密码流程里的位置。
- [验证](validation.md) - `FrameworkError::validation`，以及修改器用来浮现逐字段错误的那个模式。
