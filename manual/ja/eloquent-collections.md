# Eloquent コレクション

`Collection<T>` は、SuprnovaのLaravel形のコレクション型です - `Builder::get`、`Model::all`、あらゆる `pluck`、複数行を返すあらゆるリレーションロードの終端メソッドの戻り値です。`&[T]` へderefする、`Vec<T>` の薄いラッパーであるため、既存のスライスメソッド（`.len()`、`.iter()`、インデックス付け、`.contains(&v)`）はすべて、変更なしに動作します。その上に重なっているのが、Laravelの表面です: `map`、`filter`、`pluck`、`group_by`、`sort_by`、`where_eq`、`sum`、`avg`、その他一式です。

この章は、コレクションの表面のための、単独のリファレンスです。親である[Eloquent API](eloquent.md)はそれを要約しています。この章は、すべてのメソッド、借用対消費の契約、飛ばすと噛みつくシリアライゼーションの規則、そして代わりに `Vec<T>` へ落とすべきときを、一通り説明します。

## 目次

- [コレクションはどこから来るのか](#コレクションはどこから来るのか)
- [2つのimplブロック](#2つのimplブロック)
- [ジェネリックな表面 - あらゆる `Collection<T>` で動く](#ジェネリックな表面-あらゆる-collection-t-で動く)
- [モデルを意識した表面 - `M: Model` の `Collection<M>`](#モデルを意識した表面-m-model-の-collection-m)
- [コレクションに対するイーガーロード](#コレクションに対するイーガーロード)
- [シリアライゼーション - `to_array` 対 serde](#シリアライゼーション-to-array-対-serde)
- [借用と消費](#借用と消費)
- [`Collection` 対 `Vec`](#collection-対-vec)
- [`LazyCollection<M>` - ストリーミングの結果](#lazycollection-m-ストリーミングの結果)
- [Suprnovaが異なる設計を選んだ理由](#suprnovaが異なる設計を選んだ理由)
- [次のステップ](#次のステップ)

## コレクションはどこから来るのか

複数行を返すあらゆる終端メソッドは、`Collection<M>` を手渡します:

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

すでに持っているあらゆる `Vec<T>` をラップすることもできます:

```rust
let from_vec: Collection<User> = users_vec.into();
let from_vec2: Collection<User> = Collection::from_vec(users_vec);
let empty: Collection<User> = Collection::new();
```

`Collection<T>` は、`Default`、`Clone`、`Serialize`、`Deserialize`、`PartialEq`、そして（値渡しと `&` 渡しの両方の）`IntoIterator` を実装します。`T: Send` のとき、`Send` です。

## 2つのimplブロック

`Collection` 上のメソッドは、型パラメータに基づいて2つの系列に分かれます。

```rust
impl<T> Collection<T> { /* ジェネリックなメソッド - どんなTでも動く */ }

impl<M> Collection<M> where M: Model { /* 文字列キーのモデルメソッド */ }
```

ジェネリックブロックは、`map`、`filter`、`reject`、`chunk`、`first`、`last`、`unique`、そして、あらゆるカラムアクセサーのクロージャベースの版（`pluck_by`、`group_by_with`、`sort_with`、`key_by_with`）を与えます。これらは、`Collection<i32>`、`Collection<String>`、`Collection<MyDto>`、何であっても動きます。

モデルを意識したブロックは、マクロが発行する `Model::field_value` アクセサーを通じて行ごとにルーティングされる、文字列キーのシュガー（`pluck("name")`、`group_by("role")`、`sort_by("created_at")`、`sum::<f64>("balance")`）を追加します。これらは、`T` が `Model` を実装している場合にだけ存在します。

できるときはクロージャ形式を選んでください - 型チェッカーがフィールドアクセスを検証します。Laravelの構文に合わせたいとき、あるいはカラム名が実行時の値であるときは、文字列キー形式を選んでください。

## ジェネリックな表面 - あらゆる `Collection<T>` で動く

### 読み取る

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
nums.contains(&4);                  // true - Deref<Target = [T]>から
nums.contains_where(|n| *n > 5);    // true
```

`first_where` / `last_where` が `&&T` を取るのは、述語が `Iter<'_, T>` 上の `Iterator::find` を通じて実行されるからです。2回デリファレンスしてください（`**n`）。

### 変換する - `self` を消費して、新しいコレクションを返す

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

`map` は要素の型を変えます:

```rust
let labels: Collection<String> = nums.clone().map(|n| format!("n={n}"));
```

`each` は副作用を実行し、さらに連鎖させるためにコレクションを保持します（Suprnovaはここで意図的にLaravelと異なる設計をしています - 下記を参照）:

```rust
let kept = nums.clone()
    .each(|n| tracing::debug!(value = n, "processing"))
    .filter(|n| *n > 2)
    .take(3);
```

### クロージャキーのグルーピングとソート

```rust
use std::collections::HashMap;

// クロージャが導くキーで項目をバケットに分ける。
let by_parity: HashMap<bool, Collection<i32>> =
    nums.clone().group_by_with(|n| n % 2 == 0);

// クロージャが導くキーで項目をインデックス化する（後の重複が上書きする）。
let by_value: HashMap<i32, i32> =
    nums.clone().key_by_with(|n| *n);

// クロージャが導く比較関数でソートする。
let sorted_desc: Collection<i32> =
    nums.clone().sort_with(|a, b| b.cmp(a));

// クロージャが導くキーで重複を除く。
let unique_mod3: Collection<i32> =
    nums.clone().unique_by(|n| n % 3);

// クロージャによって、各項目を新しいコレクションへ射影する。
let strs: Collection<String> =
    nums.pluck_by(|n| n.to_string());
```

`*_with` / `*_by` という接尾辞は、ジェネリックブロック全体にわたる、「このメソッドはクロージャを取る」という普遍的な命名規則です。モデルを意識したブロックは、その接尾辞を落とし、代わりにカラム名の文字列を取ります。

### 畳み込みと集計

```rust
let sum: i32 = nums.clone().reduce(0, |acc, n| acc + n);  // 31
```

モデルのコレクションに対する型付けされた数値集計については、モデルを意識したセクションの `sum` / `avg` / `min` / `max` を参照してください - これらは、数値型へデシリアライズできるあらゆるフィールドで動作します。

### 集合演算

```rust
let a = Collection::from_vec(vec![1, 2, 3, 4]);
let b = Collection::from_vec(vec![3, 4, 5, 6]);

let joined = a.clone().concat(b.clone());    // [1,2,3,4,3,4,5,6]
let same   = a.clone().merge(b.clone());     // concatのエイリアス
let only_a = a.clone().diff(b.clone());      // [1,2]
let common = a.clone().intersect(b.clone()); // [3,4]
```

`concat` / `merge` はエイリアスです - Laravelは両方の名前を出荷しています。`diff` / `intersect` はO(n*m)です。大きなコレクションを持つ場合は、先に `HashSet` へ射影してください。

### ランダムサンプリング

```rust
let one: Option<&i32>     = nums.random();        // 1つを借用する
let many: Collection<i32> = nums.clone().random_n(3); // 3つを選ぶ
```

どちらも、スレッドローカルなRNG（`rand::rng()`）を使います。テストで決定性が必要な場合は、シード付きのRNGを手で通してください。

## モデルを意識した表面 - `M: Model` の `Collection<M>`

これらのメソッドは、含まれる型がSuprnovaのモデルである場合にだけ存在します。マクロが発行する `Model::field_value(name)` アクセサー（`Option<serde_json::Value>` を返す）を通じて、行ごとの読み取りをルーティングします。フィールドが存在しない行、あるいは対象の型へデシリアライズできない行は、サイレントにスキップされます - Laravelの、キーが欠けているときの振る舞いと一致します。

### 射影

```rust
use suprnova::{Collection, Model};

let users: Collection<User> = User::query().get().await?;

let emails: Collection<String> = users.pluck::<String>("email");
let ids:    Collection<i64>    = users.pluck::<i64>("id");
```

`pluck` は借用します（`&self`）。そのため、元のコレクションはその後も利用できます。型付けされたパラメータ（`::<String>`）は、JSONの値がデシリアライズされる対象の型です。

`pluck_keyed` は、2つのカラムから `HashMap<K, V>` を生成します:

```rust
use std::collections::HashMap;

let email_by_id: HashMap<i64, String> =
    users.pluck_keyed::<i64, String>("id", "email");
```

同じキーについては、後の行が前の行を上書きします。

`model_keys` は主キーのショートカットであり、`Collection` ではなく素の `Vec` を返す唯一の射影です:

```rust
let users: Collection<User> = User::query().get().await?;
let ids: Vec<i64> = users.model_keys();
```

これはすでにhydrate済みのキーフィールドを読み取るため、クエリのコストはかかりません。キーだけが欲しく、まだ行をロードしていない場合は、代わりにビルダーの終端操作を使ってください。`User::query().model_keys().await?` は何もhydrateせずにキーカラムを射影します。`Collection` ではなく `Vec` であることはLaravelの `modelKeys()` に一致し、この対の二つの半分が一つの形状で一致するようにします。

### グルーピングとインデックス化

```rust
use std::collections::HashMap;

let by_role: HashMap<String, Collection<User>> = users.group_by("role");
let by_id:   HashMap<String, User>             = users.key_by("id");
```

どちらのメソッドも、カラムの値を `String` のキーへ文字列化します。数値の `id` カラムは `"1"` / `"2"` として現れます - 出力が、背後の型にかかわらず常に文字列キーになるという、Laravelの `groupBy('team_id')` の契約と一致します。

型付けされたキーが欲しい場合は、ジェネリックブロック上のクロージャ形式を使ってください:

```rust
let by_id: HashMap<i64, User> = users.key_by_with(|u| u.id);
```

### フィルタリング

モデルを意識した `where_*` メソッドが `serde_json::Value` を取るのは、カラムのJSONエンコードされた形に対して比較を行うからです:

```rust
use serde_json::json;

let active: Collection<User>  = users.clone().where_eq("active", json!(true));
let admins: Collection<User>  = users.clone()
    .where_in("role", vec![json!("admin"), json!("owner")]);
let non_guests: Collection<User> = users.clone()
    .where_not_in("role", vec![json!("guest")]);
```

`where_eq` と `where_in` は、`field_value` が `None` を返す行を落とします。`where_not_in` は、フィールドが欠けている行を*保持します* - 「集合の中にある」の否定は、「集合の中にない、または存在しない」だからです。

### ソート

```rust
let by_name_asc:  Collection<User> = users.clone().sort_by("name");
let by_name_desc: Collection<User> = users.clone().sort_by_desc("name");
```

比較は、JSONの値の形をまたいでベストエフォートです: 数値対数値、文字列対文字列は、それぞれの種類の内側できれいにソートされます。混在した異種混合のカラムは、`Ordering::Equal` にフォールバックします。`None` は、存在するどの値よりも前にソートされます（ASCに対するPostgresの `NULLS FIRST` を反映しています）。

どちらのメソッドも、ソートの前に背後の `Vec<M>` をクローンします。比較関数が `m.field_value(field)` を借用する一方で、`sort_by` は `&mut [M]` を必要とするからです。きついループを持つ場合は、代わりにジェネリックブロックの `sort_with` でソートしてください - それはインプレースで動作します。

### 集計

```rust
let total: f64           = users.sum::<f64>("balance");
let avg:   Option<f64>   = users.avg::<f64>("balance");
let lo:    Option<i64>   = users.min::<i64>("login_count");
let hi:    Option<i64>   = users.max::<i64>("login_count");
```

`sum` は、値を提供する行が1つもないとき `T::default()` を返します（数値型ではゼロです）。残りの3つは `None` を返します。これは、呼び出し元がゼロ除算をしたり、幻のデフォルト値と比較したりしないようにするためです。

型付けされたパラメータ（`::<f64>`）は、JSONのデシリアライゼーションの対象です。あなたのカラムが妥当に使う、最も広い数値型を選んでください - 整数カラムには `i64`、10進数/浮動小数点数には `f64`、タイムスタンプには `chrono::DateTime<Utc>`、といった具合です。

## コレクションに対するイーガーロード

すでに `Collection<M>` を持っていて、すべての行にリレーションをロードしたい場合は、`load` / `load_missing` を使ってください:

```rust
let mut users: Collection<User> = User::query().get().await?;
users.load(["posts.comments"]).await?;

for u in &users {
    for p in u.posts_loaded() {
        println!("{}: {} comments", p.title, p.comments_loaded().len());
    }
}
```

どちらのメソッドも `&mut self`（行ごとのイーガーキャッシュを変更します）であり、`async` です。どちらも、`Builder::with([...])` が受け付けるのと同じドット区切りのパス構文を受け付けます - `"posts"`、`"posts.comments"`、`"posts.comments.author"`。

`load_missing` は、行ごとに分割します。すでにリレーションがキャッシュされている行はそのままにされ、されていない行は一括ロードを受けます:

```rust
let mut users: Collection<User> = User::query().with(["posts"]).get().await?;
// すでにpostsがキャッシュされている行もある。load_missingが触れるのは
// 残りだけ - そして、すでにキャッシュされたpostsに対しては`comments`について再帰する。
users.load_missing(["posts.comments"]).await?;
```

この再帰は、より長いドット区切りパスのすべてのセグメントで実行されます。`"a.b.c"` では、各行がすべてのレベルで分割されます: `a` は欠けている場所にだけロードされ、次に、すでに `a` を持っていた行については、`b` が、それらの `a` の上で欠けている場所にだけロードされる、という具合です。

どちらのメソッドも `#[model(connection = "...")]` によるルーティングを尊重します - 行がもともとロードされたのと同じ接続を解決します。

## シリアライゼーション - `to_array` 対 serde

これが、コレクションの表面における唯一のフットガンです。注意深く読んでください。

`Collection<T>` は `Serialize` を導出します。そのため、これは動作します:

```rust
let json: String = serde_json::to_string(&users)?;
```

しかし - serdeの `Vec<T>` に対する全面的な `Serialize` の実装は、すべての要素に対して `T::serialize` を直接呼び出します。これは、`#[suprnova::model]` マクロが発行する `Model::to_array()` のオーバーライドを**バイパスします**。つまり、あなたの `hidden = ["password"]`、`visible = [...]`、`appends = [...]` というモデルの属性をバイパスしてしまうのです。

モデルに隠しフィールドがある場合、コレクションをserde経由でシリアライズしては**いけません**。`to_array()` か `to_json()` を使ってください:

```rust
let value: serde_json::Value = users.to_array();
let body:  String            = users.to_json();
```

どちらのメソッドも、すべての行に対して `Model::to_array()` を経由するため、モデルごとのフィルタパイプラインが適用されます - 隠しフィールドは隠されたままになり、可視の許可リストは強制され、アクセッサー主導の `appends` が現れます。

同じ注意点は、内部で `serde_json::to_value(&collection)` を呼び出すあらゆるものに当てはまります: コレクションをpropsに詰め込むときの `Inertia::render`、リソース構造体の代わりに生のモデルを手渡す場合の `JsonApi`/`Resource`、ペイロードをserdeでエンコードするログの発送者、などです。安全なパターンは、値がどんなserdeの経路にも触れる前に、リソース型（[JSON:API リソース](eloquent-resources.md)）を経由するか、`to_array()` を経由して変換することです。

モデルでない型のコレクション（`Collection<MyDto>`、`Collection<String>`）については、serde経路で問題ありません - この問題が当てはまるのは、`T` が、`hidden`/`visible`/`appends` を宣言した `#[suprnova::model]` 構造体である場合だけです。

## 借用と消費

メソッドは、2つの契約へきれいに分かれます:

| 取るもの | メソッド |
|---|---|
| `&self`（借用） | `len`、`is_empty`、`is_not_empty`、`first`、`last`、`first_where`、`last_where`、`contains_where`、`random`、`as_slice`、`pluck_by`、`pluck`、`pluck_keyed`、`group_by`、`key_by`、`sum`、`avg`、`min`、`max`、`to_array`、`to_json` |
| `self`（消費） | `map`、`filter`、`reject`、`each`、`reduce`、`chunk`、`take`、`skip`、`slice`、`reverse`、`shuffle`、`random_n`、`unique`、`unique_by`、`sort_with`、`sort_by`、`sort_by_desc`、`where_eq`、`where_in`、`where_not_in`、`concat`、`merge`、`diff`、`intersect`、`group_by_with`、`key_by_with`、`map_to_map` |
| `&mut self` | `load`、`load_missing` |

消費する呼び出しの後もコレクションを保持したい場合は、呼び出しの前に `.clone()` してください。`T: Clone` のとき、`Collection<T>: Clone` です。

実践的なパターンです: 先に読み取り、最後に変換します:

```rust
let users: Collection<User> = User::all().await?;

// 借用による読み取りをまず行う - コレクションは、そのたびの後も生きている。
let total       = users.sum::<f64>("balance");
let avg         = users.avg::<f64>("balance");
let count_admin = users.iter().filter(|u| u.role == "admin").count();
let emails      = users.pluck::<String>("email");

// ここで消費する。
let admins: Collection<User> = users.where_eq("role", json!("admin"));
```

## `Collection` 対 `Vec`

このラッパーは、意図的に薄く作られています。変換の経路は両方向にあり、安価なままです:

```rust
let v: Vec<User>          = User::query().get().await?.into_vec();
let c: Collection<User>   = Collection::from(v);
let c2: Collection<User>  = Collection::from_vec(c.clone().into_vec());
```

`Deref<Target = [T]>` は、あらゆるスライスメソッドを自動的に与えます。それには、次が含まれます:

```rust
let users: Collection<User> = User::all().await?;

users.len();             // スライスのメソッド
users.iter();            // スライスのメソッド
users[0].name.clone();   // スライスのインデックス付け
users.contains(&u);      // スライスのメソッド
users.binary_search(&u); // スライスのメソッド
&users[1..4];            // スライスの部分取得
```

`IntoIterator` は2回実装されています - `Collection<T>`（値渡し）と `&Collection<T>`（参照渡し）です。そのため、次の両方が動作します:

```rust
for user in &users {           // &Userとしてiterする
    /* ... */
}

for user in users.clone() {    // Userとしてiterする（消費する）
    /* ... */
}
```

`DerefMut` が生み出すのは `&mut [T]` だけです - `Vec` ではなくスライスです。つまり、要素のフィールドのインプレースな変更は動作します:

```rust
let mut users: Collection<User> = User::all().await?;
for u in users.iter_mut() {
    u.last_seen_at = Some(Utc::now());
}
```

しかし、所有された `Vec` の変更（`push`、`pop`、`clear`、`truncate`）は、コレクション上では直接利用できません - まず `into_vec()` を呼んでください:

```rust
let mut v = users.into_vec();
v.push(new_user);
let users: Collection<User> = Collection::from(v);
```

これは意図的なものです。Laravelの表面は、コレクションを、連鎖したメソッドで変換する不変のスナップショットとして扱います。内部のシーケンスの所有された変更は、`Collection` の契約ではなく、`Vec` の契約です。

### いつ `Vec` に落とすか

次の場合には `into_vec()` に手を伸ばしてください:

- `Vec` 特有のメソッド（`push`、`pop`、`swap_remove`、`drain`、`with_capacity`）が必要な場合。
- データを、値渡しで `Vec<T>` を取るAPIへ渡していて、シグネチャの中にラッパーを持ちたくない場合。
- 行を自分自身の構造体の中に長期的に格納していて、Laravelの表面が何の得にもならない場合。

それ以外のすべて - ハンドラの戻り値、変換、（[シリアライゼーションの規則](#シリアライゼーション-to-array-対-serde)を守っている限りの）Inertiaのprops - については、`Collection<T>` を保ってください。

## `LazyCollection<M>` - ストリーミングの結果

`Collection<M>` は、すべての行をメモリ上に実体化します。収まりきらないほど大きなデータセットのために、ビルダーは、代わりに `LazyCollection<M>` を返す3つのストリーミング終端メソッドを提供します:

```rust
use suprnova::Model;

let mut stream = User::query().lazy();
while let Some(row) = stream.next().await {
    let user = row?;
    println!("{}", user.email);
}
```

| メソッド | 戦略 |
|---|---|
| `Builder::lazy()` | デフォルトのバッチサイズ（1000）によるPKカーソルのページネーション |
| `Builder::lazy_by_id(n)` | バッチサイズ `n` によるPKカーソルのページネーション |
| `Builder::cursor()` | `lazy()` のLaravelエイリアス |

`LazyCollection<M>` は、内部では `Pin<Box<dyn Stream<Item = Result<M, FrameworkError>> + Send>>` ですが、`.next().await` を直接公開するため、`futures::StreamExt` をインポートする必要はありません。各 `.next()` は次の行の配信を引き起こします。背後のバッチ取得は、バッチ内バッファがドレインされたときにだけ実行されるため、遅いコンシューマーが行を積み上げることはありません。

このラッパーは `Send` です（そのため `tokio::spawn` を越えられます）が、`Sync` ではありません - 構造上、単一コンシューマーのストリームだからです。

どのストリーミングパターンを選ぶべきかについての完全な指針は、[Eloquent - チャンクと遅延反復](eloquent.md#chunking-and-lazy-iteration)を参照してください。

## Suprnovaが異なる設計を選んだ理由

Laravelの `Illuminate\Support\Collection` は可変です: `$c->filter(...)` は、同じオブジェクトの内部配列を変更し、連鎖のために `$this` を返します。PHPには所有権がないため、その契約は目に見えません。

Rustには所有権があり、それがないふりをすることは、コレクションの表面を不誠実なものにしてしまいます。Suprnovaは代わりに、値セマンティクスの形を選びます: すべての変換は `self` を消費し、新しい `Collection` を返します。そのコストは、あなた自身のコードの中で目に見えます - 元のものを保持したければ、`.clone()` します。そうでなければ、しません。

この選択は、表面の残りの部分へも連鎖していきます:

- **`each` は `&self` ではなく `Self` を返します** - 副作用のための呼び出し（ロギング、メトリクス）が連鎖を壊さないようにするためです。PHPの `each` は副作用のために実行され、コレクションを返します。再取得なしに `$c->each(...)->filter(...)` をきれいに行うことはできませんでした。Rustでは `self` をそのまま通すことで、連鎖を流れるようにしています。

- **あらゆる文字列キーのメソッドに対する、クロージャキーの代替。** `pluck_by`、`group_by_with`、`key_by_with`、`sort_with`、`unique_by`、`map_to_map`、`contains_where`です。クロージャは、コンパイラには見えない文字列の代わりに、型チェッカーが検証するフィールドを読ませてくれます。文字列キー形式は、Laravel構文とのパリティのため、そして実行時に決まるカラム名のために存在します。

- **`sum` / `avg` / `min` / `max` は、型付けされた `::<T>` パラメータを取ります。** LaravelのPHP版は、その場でキャストします。Rustでは、デシリアライゼーションの対象が呼び出しの一部です。値が `T` へ往復しない行はサイレントにスキップされます（Laravelの、キーが欠けているときの振る舞いと一致します）が、型はあなたが意図的に選びます。

- **`Deref<Target = Vec<T>>` ではなく、`Deref<Target = [T]>`。** `Collection` は、概念的には「行のスナップショット」であり、可変なバッファではありません。スライスメソッドは `Deref` を通じてやってきます。`push`/`pop` が欲しければ、`into_vec()` が生の `Vec` を与え、あらゆる見せかけを取り除きます。

- **シリアライゼーションは、正しさに奉仕するために異なる設計をしています。** `to_array` と `to_json` は `Model::to_array()` を経由するため、モデルごとのhidden/visible/appendsが適用されます。serdeの `Vec` に対する全面的な `Serialize` によるバイパスは、まさにその通りの[フットガン](#シリアライゼーション-to-array-対-serde)として文書化されています。Laravelの `toArray()` も同じルーティングを行います。Rustのユーザーは反射的に `serde_json::to_string` に手を伸ばしてしまうため、私たちはただ、そのギャップに明示的に名前を付けなければならないだけです。

このトレードオフは、まさにSuprnovaがあらゆる場所で行っているものです: Laravelの表面の形と、Rustの値セマンティクスです。

## 次のステップ

- [Eloquent API](eloquent.md) - 親の章です。クエリビルダー、リレーション、スコープ、そしてモデルのライフサイクル全体を含みます。
- [JSON:API リソース](eloquent-resources.md) - リソース構造体は、スパースフィールドセットと `?include=` チェーンを伴う `IntoJsonResource` を通じてコレクションをシリアライズします。あなたのAPIを離れるあらゆるコレクションにとって、正しい形です。
- [Inertia レスポンス](frontend-inertia-responses.md) - シリアライゼーションのフットガンに引っかからずに、コレクションをInertiaのpropsへ渡すための規則です。
- [バリデーション](validation.md) - リクエストのペイロードは、下流の処理のために `Collection` へラップするベクタを、頻繁に生成します。
- [テスト](testing.md) - ハンドラとモデルのテストの内側で、コレクションの内容（長さ、含まれる要素、順序）についてアサートするためのパターンです。
