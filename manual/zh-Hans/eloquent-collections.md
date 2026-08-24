# Eloquent 集合

`Collection<T>` 是 Suprnova 那个 Laravel 形状的集合类型 - 是 `Builder::get`、`Model::all`、每一个 `pluck`，以及每一个产出多于一行的关系加载终结方法的返回值。它是围着 `Vec<T>` 的一层薄包装，会解引用成 `&[T]`，所以每一个既有的切片方法（`.len()`、`.iter()`、索引、`.contains(&v)`）都不必改动就能用。叠在上面的是 Laravel 那套表面：`map`、`filter`、`pluck`、`group_by`、`sort_by`、`where_eq`、`sum`、`avg`，应有尽有。

本章是集合表面的独立参考。父章节 [Eloquent API](eloquent.md) 只是概述；本章会过一遍每一个方法、借用对比消费的契约、跳过就会反咬一口的序列化规则，以及什么时候该退回到 `Vec<T>`。

## 目录

- [集合从哪里来](#集合从哪里来)
- [两个 impl 块](#两个-impl-块)
- [通用表面 - 适用于任何 `Collection<T>`](#通用表面-适用于任何-collection-t)
- [感知模型的表面 - `Collection<M>`，其中 `M: Model`](#感知模型的表面-collection-m-其中-m-model)
- [在一个集合上预加载](#在一个集合上预加载)
- [序列化 - `to_array` 对比 serde](#序列化-to-array-对比-serde)
- [借用对比消费](#借用对比消费)
- [Collection 对比 `Vec`](#collection-对比-vec)
- [`LazyCollection<M>` - 流式结果](#lazycollection-m-流式结果)
- [为什么 Suprnova 有所不同](#为什么-suprnova-有所不同)
- [下一步](#下一步)

## 集合从哪里来

任何一个返回多于一行的终结方法，都会把一个 `Collection<M>` 交到您手上：

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::all().await?;
let admins: Collection<User> = User::query()
    .db_where("role", "=", "admin")
    .get()
    .await?;
let recent: Collection<User> = User::query()
    .order_by_desc("created_at")
    .limit(50)
    .get()
    .await?;
```

您也可以包装任何您手上已有的 `Vec<T>`：

```rust
let from_vec: Collection<User> = users_vec.into();
let from_vec2: Collection<User> = Collection::from_vec(users_vec);
let empty: Collection<User> = Collection::new();
```

`Collection<T>` 实现了 `Default`、`Clone`、`Serialize`、`Deserialize`、`PartialEq`，以及 `IntoIterator`（按值和按 `&` 两种都有）。当 `T: Send` 时，它也是 `Send` 的。

## 两个 impl 块

`Collection` 上的方法，会依据类型参数分成两个家族。

```rust
impl<T> Collection<T> { /* generic methods - work for any T */ }

impl<M> Collection<M> where M: Model { /* string-keyed model methods */ }
```

通用块给您 `map`、`filter`、`reject`、`chunk`、`first`、`last`、`unique`，以及每一个列访问器的闭包版本（`pluck_by`、`group_by_with`、`sort_with`、`key_by_with`）。这些方法对 `Collection<i32>`、`Collection<String>`、`Collection<MyDto>`，什么都能用。

感知模型的块加上了字符串键的糖（`pluck("name")`、`group_by("role")`、`sort_by("created_at")`、`sum::<f64>("balance")`），它会逐行经过宏发出的 `Model::field_value` 访问器。这些方法只在 `T` 实现了 `Model` 时才存在。

能用闭包形态就用闭包形态 - 类型检查器会校验字段访问。当您想匹配 Laravel 的语法，或者列名是一个运行时的值时，就用字符串键的形态。

## 通用表面 - 适用于任何 `Collection<T>`

### 读取

```rust
use suprnova::Collection;

let nums: Collection<i32> = Collection::from_vec(vec![3, 1, 4, 1, 5, 9, 2, 6]);

nums.len();                         // 8
nums.is_empty();                    // false
nums.is_not_empty();                // true
nums.first();                       // Some(&3)
nums.last();                        // Some(&6)
nums.first_where(|n| **n > 3);      // Some(&4)
nums.last_where(|n| **n > 3);       // Some(&6)
nums.contains(&4);                  // true - 来自 Deref<Target = [T]>
nums.contains_where(|n| *n > 5);    // true
```

`first_where` / `last_where` 接受 `&&T`，因为这个判定条件是通过 `Iter<'_, T>` 上的 `Iterator::find` 运行的。要解引用两次（`**n`）。

### 变换 - 消费 `self`，返回新集合

```rust
let doubled: Collection<i32>      = nums.clone().map(|n| n * 2);
let evens:   Collection<i32>      = nums.clone().filter(|n| n % 2 == 0);
let odds:    Collection<i32>      = nums.clone().reject(|n| n % 2 == 0);
let unique:  Collection<i32>      = nums.clone().unique();
let chunks:  Vec<Collection<i32>> = nums.clone().chunk(3);
let taken:   Collection<i32>      = nums.clone().take(4);
let skipped: Collection<i32>      = nums.clone().skip(2);
let middle:  Collection<i32>      = nums.clone().slice(2, 4);
let flipped: Collection<i32>      = nums.clone().reverse();
let shuffled: Collection<i32>     = nums.clone().shuffle();
```

`map` 会改变元素的类型：

```rust
let labels: Collection<String> = nums.clone().map(|n| format!("n={n}"));
```

`each` 会运行一个副作用，并保留这个集合供进一步链式调用（Suprnova 在这里故意与 Laravel 分道而行 - 见下文）：

```rust
let kept = nums.clone()
    .each(|n| tracing::debug!(value = n, "processing"))
    .filter(|n| *n > 2)
    .take(3);
```

### 用闭包做键的分组与排序

```rust
use std::collections::HashMap;

// 按闭包算出的键给条目分桶。
let by_parity: HashMap<bool, Collection<i32>> =
    nums.clone().group_by_with(|n| n % 2 == 0);

// 按闭包算出的键给条目建索引（后来的重复项会覆盖先前的）。
let by_value: HashMap<i32, i32> =
    nums.clone().key_by_with(|n| *n);

// 按闭包给出的比较器排序。
let sorted_desc: Collection<i32> =
    nums.clone().sort_with(|a, b| b.cmp(a));

// 按闭包算出的键去重。
let unique_mod3: Collection<i32> =
    nums.clone().unique_by(|n| n % 3);

// 用闭包把每一项投影进一个新集合。
let strs: Collection<String> =
    nums.pluck_by(|n| n.to_string());
```

`*_with` / `*_by` 这个后缀，是通用块里“这个方法接受一个闭包”的统一命名约定。感知模型的块会去掉这个后缀，改成接受一个列名字符串。

### 折叠与聚合

```rust
let sum: i32 = nums.clone().reduce(0, |acc, n| acc + n);  // 31
```

对模型集合上类型化的数值聚合，请参见感知模型那一节里的 `sum` / `avg` / `min` / `max` - 它们对任何能反序列化成数值类型的字段都能用。

### 集合运算

```rust
let a = Collection::from_vec(vec![1, 2, 3, 4]);
let b = Collection::from_vec(vec![3, 4, 5, 6]);

let joined = a.clone().concat(b.clone());    // [1,2,3,4,3,4,5,6]
let same   = a.clone().merge(b.clone());     // concat 的别名
let only_a = a.clone().diff(b.clone());      // [1,2]
let common = a.clone().intersect(b.clone()); // [3,4]
```

`concat` / `merge` 是别名 - Laravel 把两个名字都发布了出来。`diff` / `intersect` 是 O(n*m) 的；如果您的集合很大，先投影到一个 `HashSet` 里。

### 随机抽样

```rust
let one: Option<&i32>     = nums.random();        // 借用一个
let many: Collection<i32> = nums.clone().random_n(3); // 挑 3 个
```

两者都用的是线程本地的 RNG（`rand::rng()`）。如果您的测试需要确定性，就手动传入一个有种子的 RNG。

## 感知模型的表面 - `Collection<M>`，其中 `M: Model`

这些方法只在被包含的类型是一个 Suprnova 模型时才存在。它们会把逐行的读取，经由宏发出的 `Model::field_value(name)` 访问器来路由，这个访问器返回 `Option<serde_json::Value>`。字段不存在、或者反序列化不到目标类型的行，会被静默跳过 - 与 Laravel 缺键时的行为一致。

### 投影

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::query().get().await?;

let emails: Collection<String> = users.pluck::<String>("email");
let ids:    Collection<i64>    = users.pluck::<i64>("id");
```

`pluck` 是借用的（`&self`），所以之后原来的集合仍然可用。这个类型化参数（`::<String>`）就是 JSON 值要反序列化成的那个目标类型。

`pluck_keyed` 会从两列产出一个 `HashMap<K, V>`：

```rust
use std::collections::HashMap;

let email_by_id: HashMap<i64, String> =
    users.pluck_keyed::<i64, String>("id", "email");
```

对同一个键，后面的行会覆盖前面的行。

`model_keys` 是主键快捷方式，也是唯一一个返回普通 `Vec` 而不是 `Collection` 的投影：

```rust
let users: Collection<User> = User::query().get().await?;
let ids: Vec<i64> = users.model_keys();
```

它读取已经水合的键字段，因此不产生查询开销。如果您只想要键、还没有加载行，请改用构建器终端 - `User::query().model_keys().await?` 会投影键列，而不会水合任何内容。这里使用 `Vec` 而不是 `Collection` 是为了匹配 Laravel 的 `modelKeys()`，也让这一对 API 的两半保持同一种形状。

### 分组与索引

```rust
use std::collections::HashMap;

let by_role: HashMap<String, Collection<User>> = users.group_by("role");
let by_id:   HashMap<String, User>             = users.key_by("id");
```

这两个方法都会把列的值字符串化成一个 `String` 键。一个数值型的 `id` 列，会变成 `"1"` / `"2"` 这样传过来 - 这与 Laravel 的 `groupBy('team_id')` 契约一致，它的输出永远是字符串键，不管底层类型是什么。

如果您想要类型化的键，就用通用块上的闭包形态：

```rust
let by_id: HashMap<i64, User> = users.key_by_with(|u| u.id);
```

### 过滤

感知模型的这些 `where_*` 方法接受 `serde_json::Value`，因为它们比较的是这一列 JSON 编码之后的形态：

```rust
use serde_json::json;

let active: Collection<User>  = users.clone().where_eq("active", json!(true));
let admins: Collection<User>  = users.clone()
    .where_in("role", vec![json!("admin"), json!("owner")]);
let non_guests: Collection<User> = users.clone()
    .where_not_in("role", vec![json!("guest")]);
```

`where_eq` 和 `where_in` 会丢掉那些 `field_value` 返回 `None` 的行。`where_not_in` 则会*保留*字段缺失的行 - “在集合里”的否定，是“不在集合里，或者缺失”。

### 排序

```rust
let by_name_asc:  Collection<User> = users.clone().sort_by("name");
let by_name_desc: Collection<User> = users.clone().sort_by_desc("name");
```

比较是在 JSON 值的各种形态之间尽力而为的：数值对数值、字符串对字符串，在各自的类别内部排得干净；混杂的异质列会回退到 `Ordering::Equal`。`None` 会排在任何有值的项之前（对应 Postgres 升序时的 `NULL FIRST`）。

这两个方法在排序之前都会克隆底层的 `Vec<M>`，因为这个比较器借用了 `m.field_value(field)`，而 `sort_by` 需要 `&mut [M]`。如果您在一个紧凑的循环里，请改用通用块上的 `sort_with` 来排序 - 它是就地操作的。

### 聚合

```rust
let total: f64           = users.sum::<f64>("balance");
let avg:   Option<f64>   = users.avg::<f64>("balance");
let lo:    Option<i64>   = users.min::<i64>("login_count");
let hi:    Option<i64>   = users.max::<i64>("login_count");
```

当没有任何一行贡献一个值时，`sum` 会返回 `T::default()`（数值类型就是零）。另外三个会返回 `None`，这样调用者就不会除以零，也不会拿去和一个虚幻的默认值比较。

这个类型化参数（`::<f64>`）就是 JSON 反序列化的目标类型。选一个您的列合理会用到的、最宽的数值类型 - 整数列用 `i64`，小数/浮点用 `f64`，时间戳用 `chrono::DateTime<Utc>`，等等。

## 在一个集合上预加载

当您已经有一个 `Collection<M>`，想把关系加载到每一行上时，就用 `load` / `load_missing`：

```rust
let mut users: Collection<User> = User::query().get().await?;
users.load(["posts.comments"]).await?;

for u in &users {
    for p in u.posts_loaded() {
        println!("{}: {} comments", p.title, p.comments_loaded().len());
    }
}
```

这两个方法都接受 `&mut self`（它们会改变逐行的预加载缓存），并且都是 `async` 的。两者都接受和 `Builder::with([...])` 一样的点号路径语法 - `"posts"`、`"posts.comments"`、`"posts.comments.author"`。

`load_missing` 会逐行分区。已经缓存了这个关系的行会被放着不管；没有缓存的行会拿到批量加载：

```rust
let mut users: Collection<User> = User::query().with(["posts"]).get().await?;
// 有些行已经缓存了 posts。load_missing 只会触碰剩下的
// 那些 - 并且会为已经缓存的 posts 递归加载 `comments`。
users.load_missing(["posts.comments"]).await?;
```

对于一条更长的点号路径，这个递归会在每一段上都运行一次。对 `"a.b.c"`，每一行都会在每一层被分区：`a` 只在缺失的地方被加载；然后对那些已经有 `a` 的行，`b` 只在这些 `a` 上缺失的地方被加载，依此类推。

这两个方法都遵从 `#[model(connection = "...")]` 的路由 - 它们会解析到这一行最初加载时所用的那同一个连接。

## 序列化 - `to_array` 对比 serde

这是集合表面上唯一的一个陷阱。请仔细读一读。

`Collection<T>` 派生了 `Serialize`。所以这样写是能跑的：

```rust
let json: String = serde_json::to_string(&users)?;
```

但是 - serde 那个对 `Vec<T>` 的兜底 `Serialize` 实现，会直接对每一个元素调用 `T::serialize`。这会**绕开** `#[suprnova::model]` 宏发出的 `Model::to_array()` 覆盖实现。也就是说，它会绕开您的 `hidden = ["password"]`、`visible = [...]`，以及 `appends = [...]` 这些模型属性。

如果您的模型有隐藏字段，**不要**通过 serde 序列化这个集合。请用 `to_array()` 或 `to_json()`：

```rust
let value: serde_json::Value = users.to_array();
let body:  String            = users.to_json();
```

这两个方法都会为每一行经过 `Model::to_array()`，所以逐模型的过滤管道会生效 - 隐藏字段保持隐藏，可见允许列表会被强制执行，由访问器驱动的 `appends` 会出现。

同样的注意事项，也适用于任何在底层调用 `serde_json::to_value(&collection)` 的东西：当您把一个集合塞进 props 时的 `Inertia::render`；当您把裸模型而不是资源结构体交给它们时的 `JsonApi`/`Resource`；对载荷做 serde 编码的日志投递器。安全的模式是：在这个值触达任何 serde 代码路径之前，先通过一个资源类型（[JSON:API 资源](eloquent-resources.md)）或者 `to_array()` 转换它。

对于非模型类型的集合（`Collection<MyDto>`、`Collection<String>`），走 serde 路径没问题 - 这个问题只在 `T` 是一个声明了 hidden/visible/appends 的 `#[suprnova::model]` 结构体时才存在。

## 借用对比消费

这些方法干净地分成了两种契约：

| 接受 | 方法 |
|---|---|
| `&self`（借用） | `len`、`is_empty`、`is_not_empty`、`first`、`last`、`first_where`、`last_where`、`contains_where`、`random`、`as_slice`、`pluck_by`、`pluck`、`pluck_keyed`、`group_by`、`key_by`、`sum`、`avg`、`min`、`max`、`to_array`、`to_json` |
| `self`（消费） | `map`、`filter`、`reject`、`each`、`reduce`、`chunk`、`take`、`skip`、`slice`、`reverse`、`shuffle`、`random_n`、`unique`、`unique_by`、`sort_with`、`sort_by`、`sort_by_desc`、`where_eq`、`where_in`、`where_not_in`、`concat`、`merge`、`diff`、`intersect`、`group_by_with`、`key_by_with`、`map_to_map` |
| `&mut self` | `load`、`load_missing` |

如果您想在一次消费性调用之后仍然保留这个集合，就在调用之前 `.clone()`。当 `T: Clone` 时，`Collection<T>: Clone`。

一个实用的模式：先读取，最后再变换：

```rust
let users: Collection<User> = User::all().await?;

// 先做借用性的读取 - 每一次之后，这个集合依然存活。
let total       = users.sum::<f64>("balance");
let avg         = users.avg::<f64>("balance");
let count_admin = users.iter().filter(|u| u.role == "admin").count();
let emails      = users.pluck::<String>("email");

// 现在再消费。
let admins: Collection<User> = users.where_eq("role", json!("admin"));
```

## Collection 对比 `Vec`

这层包装是故意做得很薄的。转换路径两个方向都有，而且都很便宜：

```rust
let v: Vec<User>          = User::query().get().await?.into_vec();
let c: Collection<User>   = Collection::from(v);
let c2: Collection<User>  = Collection::from_vec(c.clone().into_vec());
```

`Deref<Target = [T]>` 会自动给您每一个切片方法。包括：

```rust
let users: Collection<User> = User::all().await?;

users.len();             // 切片方法
users.iter();            // 切片方法
users[0].name.clone();   // 切片索引
users.contains(&u);      // 切片方法
users.binary_search(&u); // 切片方法
&users[1..4];            // 切片取子片段
```

`IntoIterator` 被实现了两次 - 一次给 `Collection<T>`（按值），一次给 `&Collection<T>`（按引用），所以下面这两种写法都能用：

```rust
for user in &users {           // 按 &User 迭代
    /* ... */
}

for user in users.clone() {    // 按 User 迭代（消费）
    /* ... */
}
```

`DerefMut` 只会产出 `&mut [T]` - 一个切片，不是一个 `Vec`。这意味着就地改动元素字段是可行的：

```rust
let mut users: Collection<User> = User::all().await?;
for u in users.iter_mut() {
    u.last_seen_at = Some(Utc::now());
}
```

但拥有型的 `Vec` 变更（`push`、`pop`、`clear`、`truncate`）不能直接在这个集合上用 - 请先调用 `into_vec()`：

```rust
let mut v = users.into_vec();
v.push(new_user);
let users: Collection<User> = Collection::from(v);
```

这是故意的。Laravel 的表面把一个集合当作一份不可变的快照，您用链式方法去变换它；对内部序列的拥有型变更，是 `Vec` 的契约，不是 `Collection` 的契约。

### 什么时候该退回到 `Vec`

在下面这些情况下，伸手去用 `into_vec()`：

- 您需要 `Vec` 专属的方法（`push`、`pop`、`swap_remove`、`drain`、`with_capacity`）。
- 您要把数据交给一个按值接受 `Vec<T>` 的 API，而不想让这层包装出现在签名里。
- 您要把这些行长期存进自己的结构体里，而 Laravel 那套表面对您毫无用处。

对其他所有场景 - 处理程序的返回值、变换、Inertia props（只要您遵守那条[序列化规则](#序列化-to-array-对比-serde)） - 都请保留 `Collection<T>`。

## `LazyCollection<M>` - 流式结果

`Collection<M>` 会把每一行都具体化在内存里。对于大到装不下的数据集，构造器提供了三个流式的终结方法，返回的是 `LazyCollection<M>`：

```rust
use suprnova::Model;

let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

| 方法 | 策略 |
|---|---|
| `Builder::lazy()` | 用默认批大小（1000）做主键游标分页 |
| `Builder::lazy_by_id(n)` | 用批大小 `n` 做主键游标分页 |
| `Builder::cursor()` | `lazy()` 的 Laravel 别名 |

`LazyCollection<M>` 底层是一个 `Pin<Box<dyn Stream<Item = Result<M, FrameworkError>> + Send>>`，但直接暴露了 `.next().await`，所以您不需要导入 `futures::StreamExt`。每一次 `.next()` 都会触发下一行的投递；底层的批量取数只在批内缓冲区排空时才运行，所以一个慢消费者不会堆积行。

这层包装是 `Send` 的（所以能跨越 `tokio::spawn`），但不是 `Sync` 的 - 按构造，它是一个单消费者的流。

关于该挑哪种流式模式的完整指引，请参见 [Eloquent - 分块与惰性迭代](eloquent.md#chunking-and-lazy-iteration)。

## 为什么 Suprnova 有所不同

Laravel 的 `Illuminate\Support\Collection` 是可变的：`$c->filter(...)` 会改动同一个对象内部的数组，并返回 `$this` 供链式调用。PHP 没有所有权的概念，所以这份契约是不可见的。

Rust 是有所有权的，装作没有，会让这个集合表面显得不诚实。Suprnova 选的是值语义的形态：每一次变换都会消费 `self`，返回一个新的 `Collection`。这个代价您能在自己的代码里看到 - 想保留原来的，就 `.clone()`；不想，就不必。

这个选择会一路影响到这个表面的其余部分：

- **`each` 返回的是 `Self`**，不是 `&self`，这样一次带副作用的调用（日志、指标）就不会打断链条。PHP 的 `each` 是为了效果而运行的，会返回这个集合；如果不重新取一次，您没法干净地写出 `$c->each(...)->filter(...)`。在 Rust 里，我们把 `self` 一路移动下去，让这条链保持流畅。

- **每一个字符串键方法都有一个闭包键的替代品。** `pluck_by`、`group_by_with`、`key_by_with`、`sort_with`、`unique_by`、`map_to_map`、`contains_where`。闭包让您读取的是类型检查器会校验的字段，而不是编译器看不见的字符串。字符串键的形态是为了和 Laravel 的语法保持对等，以及应对运行时才决定的列名。

- **`sum` / `avg` / `min` / `max` 接受类型化的 `::<T>` 参数。** Laravel 的 PHP 版本是随手转换的；在 Rust 里，反序列化的目标类型是调用的一部分。值不能原样转换成 `T` 的行会被静默跳过（与 Laravel 缺键时的行为一致），但这个类型是您有意选的。

- **是 `Deref<Target = [T]>`，不是 `Deref<Target = Vec<T>>`。** 从概念上说，一个 `Collection` 是“若干行的一份快照”，不是一个可变的缓冲区。切片方法是通过 `Deref` 拿到的；如果您想要 `push`/`pop`，`into_vec()` 会给您那个原始的 `Vec`，不再装模作样。

- **序列化上的分歧，是为了服务于正确性。** `to_array` 和 `to_json` 会经过 `Model::to_array()`，所以逐模型的 hidden/visible/appends 会生效；serde 那个对 `Vec` 的兜底 `Serialize` 绕过，被明明白白记录成了它本来就是的那个[陷阱](#序列化-to-array-对比-serde)。Laravel 的 `toArray()` 做的是同样的路由；我们只是必须把这个缺口明说出来，因为 Rust 用户会条件反射式地伸手去用 `serde_json::to_string`。

这个权衡，正是 Suprnova 到处都在做的那一个：Laravel 的表面形状，Rust 的值语义。

## 下一步

- [Eloquent API](eloquent.md) - 父章节，包含查询构造器、关系、作用域，以及完整的模型生命周期。
- [JSON:API 资源](eloquent-resources.md) - 资源结构体会通过 `IntoJsonResource`，带着稀疏字段集和 `?include=` 链来序列化集合；这是任何离开您 API 的集合应有的正确形态。
- [前端 - Inertia 响应](frontend-inertia-responses.md) - 把集合交给 Inertia props、同时不踩中序列化陷阱的规则。
- [验证](validation.md) - 请求载荷经常会产出一些向量，您会把它们包装成 `Collection` 供下游处理。
- [测试](testing.md) - 在处理程序测试和模型测试里，对集合内容（长度、包含的元素、顺序）做断言的模式。
