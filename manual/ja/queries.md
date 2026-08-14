# クエリ ビルダー

型付きの `#[suprnova::model]` 構造体としてモデリングせずにテーブルへクエリを投げたいときは、`DB::table(name)` に手を伸ばしてください。これは、型付きのEloquentの `Builder<M>` と同じ形をした連鎖可能なビルダーを返しますが、行は `DynamicRow` - 型付きアクセサを備えた `serde_json::Map` のニュータイプ - として実体化します。この章は、監査ログ、アドホックなレポート、ダッシュボードの集計、そしてモデリングする気になれなかったすべてのテーブルのためのものです。型付きの等価物については[Eloquent](eloquent.md)を参照してください。トランザクションの内側での生の `DB::select`、あるいは `DB::listen` による観測については、[データベース](database.md)を参照してください。

```rust
use suprnova::DB;

let rows = DB::table("audit_log")
    .select(["id", "event", "actor_id"])
    .filter("actor_id", 42i64)
    .filter_op("created_at", ">=", "2026-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

for row in rows.iter() {
    let id: i64 = row.get_int("id")?;
    let event: String = row.get_string("event")?;
    println!("{id}: {event}");
}
```

## どの表面を使うべきか

3つのクエリ表面が重なり合っています。テーブルに合った正しいものを選んでください。

| テーブルは… | 使うもの | 戻り値 |
|---|---|---|
| `#[suprnova::model]` でモデリングされている | `Model::query()` → `Builder<M>` | 型付きの `M` 値 |
| モデリングされていないが、連鎖可能なWHERE/ORDER/LIMITの形が欲しい | `DB::table(name)` → `DbTableBuilder` | `DynamicRow` |
| ビルダーで表現できないもの全部 - CTE、ウィンドウ関数、バックエンド固有のDDL | `DB::select` / `DB::statement` / `DB::affecting_statement` | `DynamicRow` / `bool` / `u64` |

`DbTableBuilder` は、この中間のケースのために存在します。`#[suprnova::model]` 構造体にコミットすることなく、かつ生のSQL文字列まで降りることもなく、WHERE / ORDER / LIMITの連鎖が手に入ります。

## 連鎖可能な表面

`DB::table(name)` は `DbTableBuilder` を返します。それを組み立てた上で、終端メソッドを呼び出して実行してください。

### フィルタリング

```rust
// 等価。
DB::table("users").filter("email", "alice@example.com").get().await?;

// 任意の演算子。許可リスト: =, <>, <, <=, >, >=, LIKE, NOT LIKE,
// ILIKE, NOT ILIKE, IS, IS NOT。
DB::table("orders").filter_op("total", ">=", 100i64).get().await?;
DB::table("posts").filter_op("title", "LIKE", "%rust%").get().await?;

// 複数のフィルタをANDで結合する。
DB::table("audit_log")
    .filter("actor_id", 42i64)
    .filter_op("event", "<>", "noop")
    .get()
    .await?;
```

`filter` と `filter_op` はどちらも、右辺の値として任意の `Into<SeaValue>` を受け付けます。これは `i64`、`String`、`&str`、`bool`、`f64`、`Option<T>`、`chrono::*`、`uuid::Uuid`、`serde_json::Value` をカバーします - バックエンドが理解するすべてのカラム型です。

### カラムを選択する

```rust
// デフォルトはSELECT *。
DB::table("users").get().await?;

// 一部だけが必要なときは、カラムを絞り込む。
DB::table("users").select(["id", "email"]).get().await?;
```

### 並び順とウィンドウ処理

```rust
DB::table("posts")
    .order_by_desc("created_at")
    .order_by_asc("title")
    .limit(20)
    .offset(40)
    .get()
    .await?;
```

`order_by_desc` と `order_by_asc` は挿入順に連鎖し、生成されるSQLはその順序を保持します。

### 終端メソッド

```rust
// マッチするすべての行。
let rows: Collection<DynamicRow> = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .get()
    .await?;

// 最初の行、なければNone。
let first: Option<DynamicRow> = DB::table("audit_log")
    .filter("event", "user.deleted")
    .first()
    .await?;

// カウントだけ（レンダリング前にselect/order/limit/offsetがあれば
// すべてクリアする - カウントのセマンティクスはそれらを気にしないため）。
let n: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .count()
    .await?;
```

`get()` は `Collection<DynamicRow>` を返します - 型付きモデルが使うのと同じコレクションラッパーであり、同じ `.iter()`、`.len()`、`.into_vec()` の表面を持ちます。詳しくは[Eloquent コレクション](eloquent-collections.md)を参照してください。

### 挿入、更新、削除

```rust
use suprnova::attrs;

// INSERT。新しい行の自動増分idを返す。
let id: i64 = DB::table("audit_log")
    .insert(attrs! { event: "user.created", actor_id: 42 })
    .await?;

// UPDATE。影響を受けた行数を返す。
let updated: u64 = DB::table("audit_log")
    .filter("id", id)
    .update(attrs! { event: "user.created.v2" })
    .await?;

// DELETE。影響を受けた行数を返す。
let deleted: u64 = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .delete()
    .await?;
```

`attrs!` マクロは、呼び出し箇所でカラムから値へのマップを組み立てます。キーはSQL識別子であり（検証されます）、値はパラメータとしてバインドされます。明示的なnull値は、JSON属性マップが元のRust型を保持しなくなるため、SQLの`NULL`として出力されます。nullでない値はすべて、引き続きパラメータとしてバインドされます。同じ規則が、型付きEloquentの一括書き込みと多対多ピボットの追加属性にも適用されます。

#### `update_all` と `delete_all` のエイリアス

`update` と `delete` は、Laravelに忠実な名前です。`Builder<M>` 流のエイリアスである `update_all` と `delete_all` は、同じ実装を呼び出します。テーブル全体を対象とする意図が呼び出し箇所の要点であるときは、`_all` の形を優先してください - `filter` の欠落をレビュアーに見えるようにします。

```rust
// DB::table("rate_limits").delete().await? と同じ振る舞いだが、
// _all という接尾辞が「はい、テーブルを空にするつもりです」とレビュアーに伝える。
DB::table("rate_limits").delete_all().await?;

// WHEREを伴う一括更新 - ここでの_all接尾辞は、同じ操作に対する
// 型付きBuilder<M>の規約と一致する。
DB::table("sessions")
    .filter_op("expires_at", "<", chrono::Utc::now())
    .update_all(attrs! { status: "expired" })
    .await?;
```

#### WHEREなしのupdateまたはdeleteは、すべての行に対して働く

`DB::table("x").delete().await?` は、テーブル内のすべての行を削除します。これは設計によって許容されています - 本当にテーブルを空にしたいときもあるからです - が、正しいことはまれです。`delete()` / `delete_all()` の呼び出しを見たら、必ず、その前に `filter` があるかどうかを確認してください。`update` / `update_all` についても同じことが言えます。

#### INSERTのバックエンドによる分岐

`RETURNING id` は、PostgresとSQLiteで使われます。MySQLは `RETURNING` をサポートしないため、ビルダーはINSERTを実行し、結果からドライバーの接続ごとの `last_insert_id()` を読み取ります。モデルを持たないビルダーは、標準の `id` 自動増分主キーを前提とします。UUID、複合、リネームされた、あるいは整数でない主キーは、この表面ではサポートされません - 代わりに、主キーの形をモデル定義から参照する、型付きの[Eloquent](eloquent.md) `Model` インターフェースを使ってください。

## `DynamicRow` - JSONマップの上の型付きアクセサ

`DB::table` または `DB::select` が返すすべての行は、`DynamicRow` - 型付きアクセサを備えた `serde_json::Map<String, Value>` のニュータイプ - として実体化します。各ゲッターは `Result<T, FrameworkError>` を返し、キーの欠落や型の不一致に対しては明確なエラーメッセージを伴います。

```rust
for row in rows.iter() {
    let id: i64                 = row.get_int("id")?;
    let event: String           = row.get_string("event")?;
    let active: bool            = row.get_bool("active")?;
    let weight: f64             = row.get_float("weight")?;
    let payload: serde_json::Value = row.get_value("payload")?;
}
```

NULL許容のカラムには、`get_optional_*` を使ってください。これらは、「カラムが欠けている」（エラー - スキーマの不一致）と「カラムは存在し、値はSQLのNULL」（`Ok(None)`）を区別します。

```rust
let title: Option<String> = row.get_optional_string("title")?;
let score: Option<i64>    = row.get_optional_int("score")?;
```

今のところ、optional系は `String` と `i64` をカバーしています。他のNULL許容型には、`get_value` を使って自分で `serde_json::Value::Null` に対してマッチしてください。あるいは、`get_as::<Option<T>>`（任意の `T: DeserializeOwned`）を通じてカラムを読み取ってください。

カラムを任意の構造体やコンテナ型へデシリアライズするには、`get_as` を使ってください。`serde_json` のデシリアライズ表面全体が利用できます:

```rust
#[derive(serde::Deserialize)]
struct UserPrefs {
    theme: String,
    notifications: bool,
}

let prefs: UserPrefs    = row.get_as("prefs")?;
let tags: Vec<String>   = row.get_as("tags")?;
let when: chrono::DateTime<chrono::Utc> = row.get_as("created_at")?;
```

`DynamicRow` は `Map<String, Value>` へderefするため、反復やキーの存在確認は直接機能します:

```rust
for (key, value) in row.iter() {
    println!("{key} = {value}");
}

if row.contains_key("deleted_at") { /* … */ }
```

## 識別子の信頼境界

テーブル名、カラム名、ORDER BYの方向、そしてSQL演算子は、そのままSQL文字列へ補間されます - パラメータとしてバインドされるわけでは**ありません**（SQLは、プレースホルダーでバインドされた識別子を許可しません）。あらゆる `impl Into<String>` 引数を、信頼できるコンパイル時のリテラルとして扱ってください。

```rust
// 安全 - カラム名は定数であり、値はバインドされる。
DB::table("users").filter("email", request.email()).get().await?;

// 危険 - ユーザー入力をカラム名へ差し込んでは決してならない。
DB::table("users")
    .filter(request.user_supplied_column(), value)
    .get()
    .await?;
```

フレームワークは、I/O境界で厳格な許可リストを強制します - 識別子は、オプションの `schema.` プレフィックスを1つ伴う `[A-Za-z_][A-Za-z0-9_]*` に一致しなければならず、演算子は固定リストから来なければなりません。違反は、SQLが一切レンダリングされる前に、`FrameworkError::Database` を伴って閉じる方向に失敗します。これは安全網であり、免罪符ではありません - コード内では識別子をリテラルのまま保ってください。

`filter` / `filter_op` の右辺の値は常にパラメータとしてバインドされ、リクエストのデータからそのまま差し込んでも安全です。

## 生のクエリ

ビルダーが必要なものを表現できないとき - 再帰CTE、ウィンドウ関数、バックエンド固有のDDL、`INSERT … ON CONFLICT DO UPDATE` - は、生の文字列まで降りてください。プレースホルダーは、有効なバックエンドに合わせます（Postgresなら `$1, $2, …`、MySQLとSQLiteなら `?`）- フレームワークは `DatabaseConfig::url` から自動検出します。

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - すべての行をDynamicRowとして。
let rows = DB::select(
    "SELECT u.name, COUNT(p.id) AS post_count
     FROM users u LEFT JOIN posts p ON p.user_id = u.id
     GROUP BY u.id
     HAVING COUNT(p.id) > ?",
    vec![Value::from(5i64)],
).await?;

// SELECT - 最初の行だけ。LaravelのDB::selectOneを反映している。
let alice = DB::select_one(
    "SELECT * FROM users WHERE email = ?",
    vec![Value::from("alice@example.com")],
).await?;

// SELECT - 最初の行の最初のカラムを型付きスカラーとして。
let total: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// INSERT - 少なくとも1行が影響を受けたときにtrue。
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - 影響を受けた行数を返す。
let updated: u64 = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1i64)],
).await?;

let deleted: u64 = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// バインディングを伴う任意のプリペアドステートメント。
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1i64), Value::from(42i64)],
).await?;

// プレースホルダーのバインドを拒否するDDLや、その他のバインドなしのステートメント。
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// 汎用の「影響を受けた行数」の経路 - アップサートや、名前付き
// ヘルパーに当てはまらない操作のため。
let n: u64 = DB::affecting_statement(
    "INSERT INTO counters (k, n) VALUES ($1, 1)
     ON CONFLICT (k) DO UPDATE SET n = counters.n + 1",
    vec![Value::from("page_views")],
).await?;
```

### 集計カラムの落とし穴

`SELECT COUNT(*) AS n FROM t` のような型を持たない集計は、ビルダーの `.count()` ヘルパーを通じては機能しますが、SQLite上での生の `DB::select` の行からは、サイレントに脱落して返ってくることがあります。基盤となる行の実体化ロジックはsqlxのカラムごとの型情報をたどりますが、裸の集計はそれを何も運ばないからです。SQLite上で集計を伴う生の `DB::select` が必要な場合は、式を `CAST(… AS BIGINT)` でラップして型タグを与えるか、`query_one` + `try_get` を通じてカラムごとの型検出に依存しない `DB::scalar::<i64>` を使ってください。

## 型付きEloquentへの橋渡し

テーブルが `#[suprnova::model]` 構造体に値するようになったら、この連鎖可能な形はそのまま持ち越されます。`Model::query()` は `Builder<M>` を返し、これは同じ `filter` / `filter_op` / `order_by_*` / `limit` / `offset` / `get` / `first` / `count` の表面を備えています - さらに、はるかに広いWHEREの語彙（`filter_in`、`filter_between`、`filter_null`、`filter_has`、`filter_raw`、…）と、Laravel流のエイリアス（`db_where`、`where_in`、`where_between`、`where_null`、`where_has`、`where_raw`、…）も備えています。

```rust
use suprnova::Model;

let admins = User::query()
    .filter("role", "admin")
    .filter_op("created_at", ">=", since)
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;     // Collection<User> - 型付き、DynamicRowではない

let alice = User::query().filter("email", &email).first().await?;
let total = User::query().filter("active", true).count().await?;
// 注: Builder<M>::countはi64を返す（LaravelのEloquentと一致する）。
// 一方でDbTableBuilder::countはu64を返す。どちらの表面も、負にならない
// SQLのCOUNTを返す - 異なるのはその戻り値の型だけである。
```

`Builder<M>` の表面全体 - あらゆるWHEREの形、集計、リレーション、eager loading、スコープ、ページネーター、チャンクによる反復 - は、[Eloquent](eloquent.md)にあります。上で学んだ連鎖可能な形は同じ形です。異なるのは型付けと到達範囲です。

## 名前付き接続へのルーティング

`DB::table` と生のヘルパーは、デフォルトでプライマリ接続を対象とします。リードレプリカ、シャード、あるいはウェアハウス用のプールを対象にするには、呼び出しを固定してください:

```rust
// 名前付き接続に固定されたビルダー。
let rows = DB::table("audit_log").on("warehouse").get().await?;

// 等価な短縮形。
let rows = DB::table_on("warehouse", "audit_log").get().await?;

// 生のエスケープにも_onバリアントがある。
let rows = DB::select_on("warehouse", "SELECT …", vec![]).await?;
let n    = DB::affecting_statement_on(
    "warehouse",
    "UPDATE …",
    vec![],
).await?;
```

`__read_replica__` が登録されている場合、読み取り形の終端メソッドはすべて、それを通じて自動的にルーティングされます。書き込み（`insert` / `update` / `delete` / `update_all` / `delete_all`）は常にプライマリを対象とします。`DB::transaction` のクロージャの内側では、アクティブなトランザクションの接続が絶対的に優先します - `on(name)` は、原子性を保つためにサイレントに無視されます。優先順位の全体については、[データベース - 名前付き接続](database.md)を参照してください。

### Suprnovaが異なる設計を選んだ理由

Laravelの `DB::table(...)` は、そのモデルを持たないクエリビルダーです。内部では、行ごとに `stdClass`（プロパティがカラムであるPHPオブジェクト）を返します。Suprnovaは代わりに `DynamicRow` を返します - 型付きアクセサを備えた `serde_json::Map` のニュータイプです。このアクセサの形は、カラムの欠落や型の誤りによるエラーを境界でキャッチします。ユーザーコードの奥深くで、プロパティアクセスの例外としてパニックすることはありません。

`update`/`update_all` と `delete`/`delete_all` という二重の名前が存在するのは、型付きEloquentの `Builder<M>` 表面が、テーブル全体を対象とする意図を呼び出し箇所で明示するために `_all` 接尾辞を使っているからです。どちらか一方を選ぶのではなく、モデルを持たないビルダーは両方を出荷します - `update` と `delete` は、Laravelの `DB::table($t)->update(...)` と `->delete()` に文字どおり一致します。`update_all` と `delete_all` は、`M` の利用者がすでに体で覚えている規約に一致します。

## 次のステップ

- [データベース](database.md) - `DB` ファサード、セーブポイントを伴うトランザクション、`DB::listen` による可観測性、名前付き接続
- [Eloquent](eloquent.md) - 型付きの `#[suprnova::model]` 構造体と `Builder<M>` の表面全体
- [ページネーション](pagination.md) - 型付きビルダー上の `paginate` / `simple_paginate` / `cursor_paginate`
- [Eloquent コレクション](eloquent-collections.md) - 両方の表面で `get()` が返す `Collection<T>`
- [マイグレーション](migrations.md) - ビルダーがクエリするスキーマを定義する
