# データベース

Suprnovaのデータベース層は、SeaORMをLaravel形の `DB` ファサードでラップします: 生のクエリエスケープ、モデルを持たないクエリビルダー、セーブポイントとデッドロック時のリトライを伴うトランザクション、リードレプリカとシャードのための接続レジストリ、そしてLaravel 13の `DB::listen` / `QueryExecuted` / クエリログAPIを反映した、可観測性の表面全体です。

Eloquent ORM（`use suprnova::eloquent::*`）は、この層の上に構築されており、[eloquent.md](eloquent.md)にあります。型付きのモデルが欲しいときは、そちらへ行ってください。モデル化されていないテーブルへの生のクエリが欲しいとき、あるいはフレームワークが実行するすべてのクエリを観測したいときは、このページです。

## 設定

```rust
use suprnova::{Config, DB, DatabaseConfig};

// bootstrap.rsにて
Config::register(DatabaseConfig::from_env());
DB::init().await.expect("DB::init failed");
```

`DatabaseConfig::from_env` は `DATABASE_URL` と、（オプションで）プールの調整値 `DB_MAX_CONNECTIONS`、`DB_MIN_CONNECTIONS`、`DB_CONNECT_TIMEOUT`、`DB_LOGGING` を読み取ります。`DATABASE_URL` が未設定の場合、設定は `sqlite://./database.db` にフォールバックします - セットアップ不要の開発には便利です。本番環境の起動は、`validate_for_environment` を介してそのフォールバックを拒否するため、`APP_ENV=production` で誤ってSQLiteファイルを出荷してしまうことはありません。

URL → ドライバーの検出:

```text
postgres://user:pass@host/db       → DatabaseType::Postgres
postgresql://user:pass@host/db     → DatabaseType::Postgres
mysql://user:pass@host/db          → DatabaseType::Mysql
sqlite://./file.db                 → DatabaseType::Sqlite
sqlite::memory:                    → DatabaseType::Sqlite
```

## 生のクエリ

`DB` ファサードは、Laravel 13の生のエスケープ表面全体を出荷します。すべてのヘルパーは、同じ計測済みのエグゼキューターを通ります - すべての呼び出しが `QueryExecuted` を発火します（[可観測性](#可観測性)を参照）。

バインドパラメータは `sea_orm::Value` です - フレームワークが意図的に再マスクしていない、数少ないsea_ormの型の1つです。データベースへ送られるすべての値は、これを経由するからです。`Value::from(...)` は、データベースが理解するあらゆる基本型で機能します。

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - すべての行をDynamicRowとして。
let users = DB::select(
    "SELECT * FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// SELECT - 最初の行だけ。
let alice = DB::select_one(
    "SELECT * FROM users WHERE name = ?",
    vec![Value::from("alice")],
).await?;

// SELECT - 最初の行の最初のカラムを型付きの値として。
let count: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users",
    vec![],
).await?;

// INSERT - boolを返す（少なくとも1行が影響を受けたときtrue）。
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - 影響を受けた行数を返す。
let updated = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1)],
).await?;
let deleted = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// バインディングを伴う任意のプリペアドステートメント。
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1), Value::from(42)],
).await?;

// バインディングのないDDL - `unprepared` は、プレースホルダーの
// バインドを拒否するステートメント（CREATE INDEX、ALTER TABLE、VACUUM）
// のための、Laravelの `DB::unprepared` を反映している。
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// affecting_statementは、update/deleteが内部的に使う明示的な形である -
// どちらの名前にも当てはまらない操作（例: INSERT...ON CONFLICT DO UPDATE）
// には、これへ直接降りる。
let affected = DB::affecting_statement(
    "INSERT INTO users (id, name) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name",
    vec![Value::from(1), Value::from("alice")],
).await?;
```

### プレースホルダーの構文

SQLiteとMySQLには `?`。Postgresには `$1`、`$2`、…。有効なバックエンドは、`DatabaseConfig::url` から自動検出されます。

### DynamicRow

型を持たない行は、`DynamicRow` - 型付きアクセサを備えた `serde_json::Map` のニュータイプ - として実体化します:

```rust
for row in users {
    let id: i64 = row.get_int("id")?;
    let name: String = row.get_string("name")?;
    let nickname: Option<String> = row.get_optional_string("nickname")?;
    let score: Option<i64> = row.get_optional_int("score")?;
    // 任意のT（chrono::DateTime、自分の構造体など）をデシリアライズする:
    let prefs: UserPrefs = row.get_as("prefs")?;
}
```

`get_*` は、カラムが欠けているか、NULLであるときにエラーになります。`get_optional_*` は、欠けているときだけエラーになり、SQLのNULLに対しては `Ok(None)` を返します。アクセサの全リストは、`get_int` / `get_string` / `get_bool` / `get_float` / `get_value` / `get_as<T>`、そして `get_optional_string` / `get_optional_int` です。専用の `get_optional_*` を持たないNULL許容型には、`get_value` + `serde_json::Value` へのマッチ、あるいは `get_as::<Option<T>>` に手を伸ばしてください。

## モデルを持たないクエリビルダー - `DB::table`

`#[suprnova::model]` でモデリングする気になれなかったテーブルへのアドホックなクエリには、`DB::table(...)` が、Eloquentの `Builder<M>` と同じ形をした連鎖可能なビルダーを返します。ただし、行は `DynamicRow` として実体化します:

```rust
use suprnova::{DB, attrs};

let rows = DB::table("audit_log")
    .select(["id", "event", "actor_id"])
    .filter("actor_id", 42i64)
    .filter_op("created_at", ">=", "2025-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

let first = DB::table("audit_log")
    .filter("event", "user.deleted")
    .first()
    .await?;

let count = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .count()
    .await?;

let id = DB::table("audit_log")
    .insert(attrs! { event: "user.created", actor_id: 42 })
    .await?;

let updated = DB::table("audit_log")
    .filter("id", id)
    .update(attrs! { event: "user.created.v2" })
    .await?;

let deleted = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .delete()
    .await?;
```

### 識別子の信頼境界

テーブル名、カラム名、ORDER BYの方向、そしてSQL演算子は、そのままSQL文字列へ補間されます - パラメータとしてバインドされるわけでは**ありません**（SQLは、プレースホルダーでバインドされた識別子を許可しません）。あらゆる `impl Into<String>` 引数を、**信頼できる**リテラルとして扱ってください:

```rust
// 安全 - カラム名は定数である。
DB::table("users").filter("email", request.email()).get().await?;

// 危険 - ユーザー入力をカラム名へ差し込んでは決してならない。
DB::table("users").filter(&request.column_name(), value).get().await?;
```

値（`filter` / `filter_op` の右辺）は、パラメータとして**バインドされ**、ユーザー入力に対して安全です。

フレームワークは、識別子（オプションの `schema.` プレフィックスを1つ伴う `[A-Za-z_][A-Za-z0-9_]*`）と演算子（`=`、`<>`、`<`、`<=`、`>`、`>=`、`LIKE`、`NOT LIKE`、`ILIKE`、`NOT ILIKE`、`IS`、`IS NOT`）に厳格な許可リストを強制します。違反は、SQL文字列がレンダリングされる前に、I/O境界でエラーになります。

## トランザクション

3つのエントリーポイントがあり、いずれも `QueryExecuted` /
`TransactionBeginning` / `TransactionCommitted` /
`TransactionRolledBack` の観測フックが配線されています。

### クロージャの形式

```rust
use suprnova::DB;

DB::transaction(|_tx| {
    Box::pin(async move {
        let mut alice = User::query().filter("name", "alice").first_or_fail().await?;
        alice.balance -= 30;
        alice.save().await?;

        let mut bob = User::query().filter("name", "bob").first_or_fail().await?;
        bob.balance += 30;
        bob.save().await?;
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

`Ok(_)` でコミットします。`Err(_)` ではロールバックし、エラーを伝播します。

`Err` が常にロールバックを意味するとは限りません。[コミット後](queues.md#after-commit-dispatch)のコールバックが失敗した場合、コミットはすでに完了して永続化されています。`DB::transaction` はそれでも `Err` を返し、そのメッセージは `after-commit callback failed (the transaction itself committed): <コールバックのエラー>` となります。クロージャの戻り値は失われますが、その書き込みは失われません。失敗したのは、先送りされたディスパッチだけです。登録されたコールバックはすべて走り、あなたが受け取るのは最初のエラーです。`DB::transaction_with_attempts` は、どれほどデッドロックらしく読めても、そのエラーをリトライすることは決してありません: すでに永続化された書き込みを持つクロージャを再実行すれば、それらを二重に適用してしまいます。

クロージャの内側の操作は、`tokio::task_local` を介してアクティブなトランザクションを自動的に取得します - `&tx` のハンドルをすべてのモデル呼び出しに通す必要は**ありません**。ネストした `DB::transaction` はデータベースエラーを返します。ネストしたロールバックの挙動には `tx.savepoint(...)` を使ってください。

クロージャの形式は、作業をコミットまで先送りできる唯一の形式でもあります。型が `Job::after_commit()` を宣言しているジョブ（あるいは `Queue::push_after_commit` で行われたディスパッチ）は、このクロージャの内側で待ち、コミットが成功したときにだけキューのドライバーへ到達します。ロールバックはそれを捨てます。[コミット後のディスパッチ](queues.md#after-commit-dispatch)を参照してください。

同じ固定された接続の上で実行しなければならない、型付きの集計やカスタムSQLには、トランザクションのハンドルを直接使ってください:

```rust
use sea_orm::{DbBackend, Statement};

DB::transaction(|tx| {
    Box::pin(async move {
        let backend = tx.backend();
        let rows = tx.query_all(Statement::from_string(
            backend,
            "SELECT CAST(COUNT(*) AS BIGINT) AS total FROM orders".to_owned(),
        )).await?;
        let total = rows[0].try_get::<i64>("", "total")?;
        Ok::<_, suprnova::FrameworkError>(total)
    })
}).await?;
```

`query_all` は通常の `QueryExecuted` の観測を発行し、型付きのSeaORMの `QueryResult` の行を返します。動的な値にはバインドされた `Statement::from_sql_and_values` を使ってください。信頼できない入力を文字列に埋め込んではいけません。

### デッドロック時のリトライ

```rust
DB::transaction_with_attempts(5, |_tx| {
    Box::pin(async move {
        // 上と同じクロージャの本体です。SQLSTATE 40001 / 40P01、
        // あるいは "deadlock" を含む（大文字小文字を区別しない）
        // あらゆるエラーで、最初からやり直します。
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

### 手動の形

```rust
use suprnova::{DB, attrs};

let tx = DB::begin_transaction().await?;

// モデルごと: `*_with_tx` のシムは、1つのCRUD操作を手動のtxへ固定します。
User::create_with_tx(&tx, attrs! { name: "alice" }).await?;
Order::create_with_tx(&tx, attrs! { user_id: 1, total: 30 }).await?;

// クエリごと: `Builder::with_tx(&tx)` はビルダーのチェーンを固定します。
let stale = Order::query()
    .filter("status", "pending")
    .with_tx(&tx)
    .get()
    .await?;

if some_condition() {
    tx.rollback().await?;
} else {
    tx.commit().await?;
}
```

手動モードはtask-localをインストール**しません** - トランザクションの内側で実行されるべきすべての操作は、チェーンされたクエリに対する `Builder::with_tx(&tx)` か、`Model::*_with_tx` のシム（`create_with_tx`、`save_with_tx`、`delete_with_tx` など）のいずれかで、明示的にオプトインしなければなりません。オプトインを忘れた操作は、グローバルなプールに対して実行され、トランザクションの一部には**なりません**。

`Transaction` のハンドルを保持している間は、プールの接続が1つ、その生存期間のあいだ固定されます。読み取る必要のある行は、`begin_transaction()` の呼び出しより**前**に、とりわけSQLite（単一の共有接続）では先に読み込んでおいてください。

手動モードはtask-localをインストールしないため、先送りされたディスパッチがぶら下がるコミットも持ちません: 手動のトランザクションの内側でプッシュされた[コミット後](queues.md#after-commit-dispatch)のジョブは、即座にプッシュされます。ディスパッチがコミットを待たなければならないときは、クロージャの形式を使ってください。

### セーブポイント

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        Order::create(/* ... */).await?;

        tx.savepoint("after_order").await?;
        if let Err(e) = Payment::charge().await {
            // 支払いの試行は捨てますが、注文は残します。
            tx.rollback_to("after_order").await?;
        }
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

ファーストクラスの3つのバックエンドはすべて `SAVEPOINT` / `ROLLBACK TO SAVEPOINT` をサポートします - SQLiteも含みます。

セーブポイントのロールバックは、[コミット後のレジストリ](queues.md#after-commit-dispatch)も巻き戻します。セーブポイントの内側でコミットまで先送りされたキューへのプッシュは、それが記述していた行とともに捨てられ、それとともに登録された補償処理が即座に走ります。そのため、先送りされた `push_unique` の重複排除ロックは戻り、同じトランザクションの内側での再ディスパッチがそれを獲得できます。セーブポイントより前に登録されたものは手つかずのままで、解放した、あるいは単にロールバックしなかったセーブポイントは、その内側に登録されたものをすべて保ちます。

セーブポイント名の繰り返しは許されており、レジストリはデータベースに従います: `ROLLBACK TO SAVEPOINT x` は最も新しい `x` まで巻き戻し、そのあとに確立されたセーブポイントを破棄します。手動のトランザクションはコミット後のレジストリを持たないため、そのセーブポイントは行をロールバックするだけで、ほかには何もしません。

レジストリに印を付けるのは `Transaction::savepoint` だけです。生のSQLで作ったセーブポイントはレジストリからは見えないため、`rollback_to` はそれらの行をロールバックし、警告をログに記録し、その内側に登録された先送りされたディスパッチはすべてそのまま残します - 推測で1つ捨てるほうが、より悪い失敗になるからです。先送りされたディスパッチを行とともに巻き戻したいときは、`Transaction::savepoint` を使ってください。

## 可観測性

Laravel 13の `DB::listen` / `QueryExecuted` / クエリログの表面を、Suprnovaのイベントディスパッチャーを通じてRustへポーティングしたものです。

### `DB::listen` - 直接のコールバック

```rust
use suprnova::{DB, QueryExecuted};

// bootstrap.rsにて（あるいはサービスプロバイダーで）。
DB::listen(|event: &QueryExecuted| {
    tracing::debug!(
        sql = %event.sql,
        bindings = ?event.bindings,
        time_ms = event.time.as_millis(),
        connection = %event.connection_name,
        "query executed",
    );
})?;
```

リスナーは、**エグゼキューターヘルパーの内側で同期的に**実行されます。遅いリスナーはクエリを遅くします - 直接のコールバックは軽く保ってください。失敗する可能性のあるものには、下の `EventFacade` の経路を優先してください。それは `dispatch_best_effort` を通じて実行され、エラーを許容します。

### `EventFacade` のディスパッチ経路

`QueryExecuted` は、本物の `suprnova::Event` です - ディスパッチャーを通じてリスンすれば、キューに入れられ、フェイク可能で、失敗に耐性のある配信が手に入ります:

```rust
use suprnova::{EventFacade, Listener, QueryExecuted, FrameworkError};
use std::sync::Arc;

struct LogToDatabase;

#[suprnova::async_trait]
impl Listener<QueryExecuted> for LogToDatabase {
    async fn handle(&self, event: &QueryExecuted) -> Result<(), FrameworkError> {
        // このリスナー自身がデータベースにクエリを投げても、再入防止の
        // 保護機構が無限再帰を防ぐ。
        DB::statement(
            "INSERT INTO query_log (sql, time_ms) VALUES (?, ?)",
            vec![event.sql.clone().into(), (event.time.as_millis() as i64).into()],
        ).await?;
        Ok(())
    }
}

// bootstrap.rsにて。
EventFacade::listen::<QueryExecuted, _>(Arc::new(LogToDatabase)).await;
```

この経路のリスナーは:

- `dispatch_best_effort` を通じて実行されます - 失敗するリスナーが、クエリを失敗させることは**ありません**。
- 自分自身がクエリを発行するときは、ショートサーキットされます（再入防止の保護機構）。
- テストの中で `Event::fake()` を使い、リスナーを実際に実行することなく、ディスパッチをアサートできます。

### インメモリのクエリログ

```rust
DB::enable_query_log()?;

User::query().filter("active", true).get().await?;
Order::query().count().await?;

let log = DB::get_query_log()?;
for query in &log {
    println!("{} ({}ms)", query.sql, query.time.as_millis());
}

DB::flush_query_log()?;     // エントリを削除するが、有効なままにする
DB::disable_query_log()?;   // キャプチャを停止する
let still_capturing = DB::logging();
```

このログは**無制限**です - キャプチャされるクエリはすべて、プロセスが終了するか、`flush_query_log()` が実行されるか、`disable_query_log()` が呼ばれるまで、それを大きくし続けます。長時間動く本番環境のプロファイラーとしてではなく、開発のために使ってください。

### トランザクションのライフサイクルイベント

`TransactionBeginning`、`TransactionCommitted`、`TransactionRolledBack` は、本物の `suprnova::Event` 型です - 監査、分散ロック、あるいは補償ロジックを駆動するために、`EventFacade::listen` を通じてそれらをリスンしてください。

```rust
EventFacade::listen::<TransactionCommitted, _>(Arc::new(AuditCommit)).await;
EventFacade::listen::<TransactionRolledBack, _>(Arc::new(MetricRollback)).await;
```

3つのトランザクションのエントリーポイントすべて（`DB::transaction` / `DB::transaction_with_attempts` / `DB::begin_transaction` + `Transaction::commit`/`rollback`）が、イベントを発火します。明示的なコミット/ロールバックなしにドロップされる、リークした手動の `Transaction` ハンドルは、イベントを発しません - SeaORMの `Drop` 実装は同期的であり、非同期のディスパッチャーに到達できないからです。

### `QueryExecuted` のペイロード

```rust
pub struct QueryExecuted {
    pub sql: String,
    pub bindings: Vec<String>,         // debugでレンダリングされる（`{:?}`）
    pub time: std::time::Duration,
    pub connection_name: String,
    pub read_write_type: Option<ReadWriteType>,
    pub result: Result<(), String>,    // ドライバーエラーではErr
}
```

`to_raw_sql()` は、表示のために、キャプチャされたバインドパラメータをSQLへ代入します:

```rust
let query = /* captured from a listener */;
println!("{}", query.to_raw_sql());
// SELECT * FROM users WHERE id = 42 AND active = true
```

この代入は**デバッグ形式**であり（SQLに対して安全なエスケープではありません）、ログ出力のためだけを意図しています。その結果を、決してクエリへ戻して与えないでください。

### カバレッジの範囲

現在、`QueryExecuted` は、計測済みの `ExecutorChoice` ヘルパーを通るすべてのクエリに対して発火します:

- `DB` 上のすべての生のヘルパー（`select` / `select_one` / `scalar` / `insert` / `update` / `delete` / `statement` / `affecting_statement` / `unprepared`）。
- `DbTableBuilder`（モデルを持たないビルダー）上のすべての終端メソッド。
- `DB::transaction` / `DB::begin_transaction` のBEGIN / COMMIT / ROLLBACKは、トランザクションのイベントを発火します。
- `DbConnection::connect` は `ConnectionEstablished` を発火します。

Eloquent ORM（`Builder<M>::get` / `first` / `count`、モデルのCRUD）は、現在のところ、計測済みのヘルパーを通じて呼び出すのではなく、`ExecutorChoice` の `Tx` / `Pool` の分岐に直接マッチします - ヘルパー（したがって観測フック）を採用することは、Eloquentモジュールの範疇です。

## 接続のメタデータ

```rust
let name = DB::database_name()?;        // postgres://.../myapp の場合は "myapp"
let driver = DB::driver_name()?;        // "postgres" | "mysql" | "sqlite"
let title = DB::driver_title()?;        // "Postgres" | "MySQL" | "SQLite"
let version = DB::server_version().await?;  // "15.5" | "8.0.36" | "3.42.0"
```

`server_version` は、バックエンド固有の内省クエリを発行します（Postgres + MySQLには `SELECT VERSION()`、SQLiteには `SELECT sqlite_version()`）。頻繁に呼び出す場合は、結果をキャッシュしてください - 呼び出しはすべて、ラウンドトリップになります。

## 名前付き接続

リードレプリカ、シャーディングされたシャード、あるいはモデルごとのウェアハウスプールのために:

```rust
// bootstrap.rsにて
DB::register_named("__read_replica__", read_config).await?;
DB::register_named("warehouse", warehouse_config).await?;

// クエリごとのルーティング:
let rows = User::query().on("__read_replica__").get().await?;
let warehouse_rows = DB::table("audit_log").on("warehouse").get().await?;
let raw = DB::select_on("warehouse", "SELECT ...", vec![]).await?;
```

`__read_replica__` という名前は、よく知られたものです: 登録されると、読み取り形の終端メソッドはすべて、それを通じて自動的にルーティングされます。書き込みはレプリカを無視し、プライマリを対象とします。特定の操作でプライマリへオプトバックするには、`Builder::on_write_connection`（クエリごと）または `#[model(connection = "...")]`（モデルごとのデフォルト）を使ってください。

予約された名前です:

- `__primary__` - デフォルトのプール。登録できません（これは `DB::connection()` の戻り値です）。
- `__read_replica__` - よく知られた読み取りレプリカ。この名前のもとで登録された**あらゆる**接続が、読み取りのルーティングを引き継ぎます。

完全な優先順位の連鎖（ビルダーのtx上書き → 周囲のtx → ビルダーの `on(name)` → モデルのデフォルト → `__read_replica__` → プライマリ）については、[eloquent.md → マルチ接続ルーティング](eloquent.md#multi-connection-routing)を参照してください。

## テスト

`TestDatabase` は、インメモリのSQLiteデータベースを構築し、それをテストコンテナへ登録します。そのため `DB::connection()` はそれを解決します。そして、あなたのマイグレーションを実行します:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn test_user_creation() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // DB::connection()を呼び出すあらゆるコードは、今やこのインメモリDBを手に入れる。
    let _ = CreateUser::run("alice@example.com").await.unwrap();
}

// `test_database!()`はマクロによる短縮形。
let db = test_database!();
```

自分自身のアドホックなスキーマを構築するテストには:

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").await.unwrap();
```

`TestDatabase` がドロップすると、テストコンテナはクリアされ、接続レジストリは消去されます - テストをまたいだ漏れはありません。プロセス全体の状態（レジストリ、リスナーレジストリ、クエリログ）を変更するテストは、衝突しないよう `#[serial_test::serial]` を付けるべきです。

## 次のステップ

- [Eloquent](eloquent.md) - この層の上に位置する、型付きの `#[suprnova::model]` ORM
- [マイグレーション](migrations.md) - `Migrator`、`make:migration`、そして `db:sync` のワークフロー
- [データベース テスト](database-testing.md) - `TestDatabase`、フィクスチャのロード、そしてシリアルテストのアノテーション
- [イベント](events.md) - `QueryExecuted` / `TransactionCommitted` のリスナーの背後にあるディスパッチャー
- [設定](configuration.md) - あなたの型付き設定の残りと並んで `DatabaseConfig` を登録する

## 表面の索引

| 表面 | Laravel対応 |
| --- | --- |
| `DB::init` / `DB::init_with` / `DB::connection` / `DB::is_connected` / `DB::get` | `DB::connection()` |
| `DB::table(name)` → `DbTableBuilder` | `DB::table($name)` |
| `DB::select` / `select_one` / `scalar` / `insert` / `update` / `delete` / `statement` / `affecting_statement` / `unprepared` | `DB::select` / `selectOne` / `scalar` / `insert` / `update` / `delete` / `statement` / `affectingStatement` / `unprepared` |
| `DB::transaction` / `transaction_with_attempts` / `begin_transaction` | `DB::transaction($cb, $attempts)` / `DB::beginTransaction` |
| `Transaction::commit` / `rollback` / `savepoint` / `rollback_to` | `DB::commit` / `rollBack` / セーブポイントのヘルパー |
| `DB::listen(callback)` | `DB::listen` |
| `DB::enable_query_log` / `disable_query_log` / `get_query_log` / `flush_query_log` / `logging` | `DB::enableQueryLog` / `disableQueryLog` / `getQueryLog` / `flushQueryLog` / `logging` |
| `DB::database_name` / `driver_name` / `driver_title` / `server_version` | `getDatabaseName` / `getDriverName` / `getDriverTitle` / `getServerVersion` |
| `DB::register_named` / `named` / `select_on` / `table_on` / `statement_on` / `affecting_statement_on` | マルチ接続の `DB::connection($name)` |
| `QueryExecuted` / `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` / `ConnectionEstablished` / `DatabaseBusy` | `Illuminate\Database\Events\*` |
| `DatabaseConfig::builder()` / `from_env` / `validate_for_environment` | `config/database.php` |
| `TestDatabase::fresh::<M>` / `sqlite_memory` / `execute_unprepared` / `fetch_one` / `fetch_all` | `RefreshDatabase` テストトレイト |
