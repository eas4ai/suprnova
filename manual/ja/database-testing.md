# データベース テスト

[テスト](testing.md)のDB特化版の相棒です。あの章がテストハーネス - `#[suprnova_test]`、`describe!` / `test!`、`expect!`、そしてプロセス内フェイク - をカバーするのに対し、この章はテストがデータベースを必要とするときに何が変わるかをカバーします: `TestDatabase` がどのようにそれを構築するか、分離が実際にどう機能するか、ファクトリーとシーダーがどこに差し込まれるか、そしてインメモリのSQLiteがいつ十分でいつ不十分になるか、です。

## 2つのコンストラクタ

すべてのデータベーステストは、`TestDatabase` を構築することから始まります。2つのコンストラクタ、2つの意図です。

### `TestDatabase::fresh::<Migrator>()`

インメモリのSQLiteデータベースを構築し、あなたのマイグレーターをエンドツーエンドで実行し、その接続をテストコンテナへ登録します。そのため、`DB::connection()` や `App::resolve::<DbConnection>()` を呼び出すあらゆるコードは、それを解決します。これは、実際のスキーマに触れるすべてのもののための、正しいデフォルトです。

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn user_lifecycle_end_to_end() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);
    // モデルの表面を迂回したいときは、直接クエリする:
    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`Migrator` は、あなたのアプリケーションの `MigratorTrait` 実装です - 本番の `suprnova migrate` コマンドが実行するのと同じ型です。本物のマイグレーターをテストスキーマに通すことで、スキーマのドリフトを不可能にします。マイグレーターが追加を忘れたカラムが、テストDBの中にサイレントに存在してしまうことはありません。

`test_database!()` マクロは、よくあるケース（`crate::migrations::Migrator`）のための糖衣構文です:

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();          // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}

// あるいは、カスタムのマイグレーターパスを使う場合:
let db = test_database!(my_crate::CustomMigrator);
```

### `TestDatabase::sqlite_memory()`

同じコンテナとレジストリの配線ですが、**マイグレーターはまったく実行しません**。テストがカラムの形を精密に制御したいとき - 典型的には、キャストの往復、クエリビルダーのSQL表面のテスト、あるいはフルのマイグレーターがやり過ぎだったりノイズになったりするドライバーレベルのエッジケース - に使ってください:

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared(
    "CREATE TABLE casts_t (id INTEGER PRIMARY KEY, payload BLOB)",
)
.await
.unwrap();

// それから、直接書き込み、型付きヘルパーで読み返す:
let row = db.fetch_one(
    "INSERT INTO casts_t (payload) VALUES (?) RETURNING id, payload",
    vec![sea_orm::Value::Bytes(Some(Box::new(b"hello".to_vec())))],
).await.unwrap();
```

`sqlite_memory()` は、`fresh()` が組み立てられている基盤です - `fresh` はこれを呼び出した上で、あなたのマイグレーターを実行します。`fresh` でできることは何でもここでもできます。ただ、自分自身のDDLを持ち込むだけです。

### `execute_unprepared`、`fetch_one`、`fetch_all`

`TestDatabase` は、テストで最も頻繁に使う3つのSeaORM実行形式を再エクスポートします。そのため、テストファイルが `ConnectionTrait` を持ち込む必要はありません:

| メソッド | 用途 |
| --- | --- |
| `execute_unprepared(sql)` | プレースホルダーを持たないDDLまたはDML。`Result<(), FrameworkError>` を返す |
| `fetch_one(sql, bindings)` | 1行のSELECT。0行ならエラー |
| `fetch_all(sql, bindings)` | 全行のSELECT |

バインドパラメータは `Vec<sea_orm::Value>` です - 本番のクエリ経路が使うのと同じ形です。接続のバックエンド（どちらのコンストラクタでもSQLite）はあなたに代わって供給されるため、`?` プレースホルダーが正しい選択です。

## 分離が実際にどう機能するか

テストごとに新しいデータベースを使うモデルこそが、分離の仕組みです。`fresh()` や `sqlite_memory()` の呼び出しはそれぞれ、新しい `sqlite::memory:` 接続を開きます。これはSQLiteの下では完全に別個のデータベースインスタンスです - 共有されるスキーマはなく、共有される行もなく、他のどのテストもその中を見ることはできません。トランザクションのラッパーもなく、オプトインする `RefreshDatabase` トレイトもなく、覚えておくべきロールバックもありません: *次の* テストは、自分自身のDBを構築するからこそ、きれいな空のDBを手に入れます。

`TestDatabase` の値がドロップすると、次の3つのことが、この順序で起こります:

1. 保持されていた `TestContainerGuard` が、スレッドローカルなテストコンテナをクリアします。そのため、それ以降の `App::get::<DbConnection>()` は、もはやテスト接続を見つけません。
2. これがプロセス内で*最後の*生きている `TestContainerGuard` だった場合、名前付きの[`ConnectionRegistry`](database.md#named-connections)が消去されます。（`FAKE_GUARDS` に対する参照カウントは、内側のテストのドロップが、並行する外側のテストがまだ依存している接続名を消し去ってしまわないことを保証します - この参照カウントを生んだ、恒常的な落とし穴です。）
3. SQLite接続自体がドロップし、インメモリデータベースを破棄します。

状態がロールバックされるのではなく再構築されるため、この分離は `BEGIN`/`ROLLBACK` によるラッピングよりも強力です: 誤って生き残ってしまうコミット済みの状態はなく、ネストしたトランザクションの癖もなく、テスト間での連番カウンターのずれもありません。代償は、テストごとにマイグレーターを1回実行するコストを払うことです（ほとんどのスキーマではSQLiteにとって無視できるコストです。本当にコストになる場合は、下の「テストをまたいでマイグレーション済みのデータベースを共有する」を参照してください）。

## プールが1つの接続に固定されている理由

どちらのコンストラクタも、`max_connections(1)` と `min_connections(1)` を伴ってデータベースを構築します。これは `sqlite::memory:` にとって荷重を支える設計であり、汎用的なポリシーではありません。

`sqlite::memory:` は接続ごとのデータベースです - プール内の*新しい*接続はそれぞれ、別個の空のSQLiteインスタンスになってしまいます。サイズ2のプールは、クエリの半分がマイグレーション済みのデータベースを見て、残り半分が空のデータベースを見ることを意味してしまいます。プールを1つの接続に固定することで、テスト内のすべてのクエリが、マイグレーターが対象にしたのと同じインメモリデータベースに行き着くようになります。

その帰結として: 本当の接続の並行性を発生させるテスト（2つのトランザクションが競合する、レプリカへのルーティング、リクエストハンドラと同時にキューワーカーがDBに触れる）には、本物のデータベースが必要です。下の「SQLiteのインメモリが十分でないとき」を参照してください。

## テストの中のファクトリー

ファクトリーは、ランダム化されたモデルインスタンスを生成し、（オプションで）それらを永続化します。永続化の経路は、束縛されたテスト接続を自動的に解決します - テストのためにファクトリー側の配線をする必要はありません。

```rust
use crate::factories::UserFactory;

#[tokio::test]
async fn factory_round_trip() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // インメモリのみ: 最速、DBへの往復なし。
    let alice = UserFactory::new()
        .with(|u| u.email = "alice@example.com".into())
        .make();
    assert_eq!(alice.email, "alice@example.com");

    // 1件を永続化し、挿入後のモデル（idが割り当てられている）を返す。
    let bob = UserFactory::new().create().await.unwrap();
    assert!(bob.id > 0);

    // バルク: 50件を順番に永続化する。
    let many = UserFactory::times(50).create_many().await.unwrap();
    assert_eq!(many.len(), 50);
}
```

知っておく価値のある、2つのパターンです:

**ファクトリーの挿入は、モデルイベントを迂回します。** `create()` / `create_many()` を支える `Persistable` の実装は、SeaORMの `ActiveModelTrait::insert` を直接通じて書き込みます - `Creating` / `Created` / `Saving` / `Saved` をディスパッチする `Model::create` の表面を経由することは*ありません*。「フィクスチャを構築している間、オブザーバーは一切発火しない」ことを主張するテストは、何も特別なことを必要としません。「`Created` オブザーバーが本当に発火した」ことを主張するテストは、ファクトリーの代わりに `Model::create(...)`（または `save()`）を駆動しなければなりません。

**`create_many` はトランザクションを張りません。** 挿入は逐次的です。後の行が失敗しても、それより前の行はロールバックされません。テストが原子性を要求する場合は、その呼び出しを自分自身の `DB::transaction` でラップしてください:

```rust
DB::transaction(|tx| async move {
    UserFactory::times(50).create_many().await?;
    PostFactory::times(200).create_many().await?;
    Ok::<_, FrameworkError>(())
}).await.unwrap();
```

完全なファクトリーの表面（states、sequences、`with`-リレーション、`count`、`times`、`make_one` / `create_one`）については、[Eloquent → ファクトリー](eloquent-factories.md)を参照してください。

## テストの中のシーダー

シーダーは、フレームワークのシーダーレジストリに、安定した名前のもとで登録した関数です。テストからそれらを駆動するパターンは2つあり、それぞれ意図の軸ごとに1つです。

### 名前で単一のシーダーを実行する

```rust
use suprnova::seed;
use my_app::seeders::UsersSeeder;

#[tokio::test]
async fn users_seeder_populates_fixtures() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<UsersSeeder>();
    seed::run_one("UsersSeeder").await.unwrap();

    let count = User::query().count().await.unwrap();
    assert!(count > 0);
}
```

### ブートストラップのシーダー集合全体を実行する

```rust
use serial_test::serial;
use suprnova::seed;

#[tokio::test]
#[serial]
async fn full_seed_lands_expected_row_counts() {
    seed::clear();                              // 既知の空のレジストリから始める
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<my_app::seeders::UsersSeeder>();
    seed::register::<my_app::seeders::PostsSeeder>();
    seed::run_all().await.unwrap();

    let users = User::query().count().await.unwrap();
    let posts = Post::query().count().await.unwrap();
    assert_eq!(users, 50);
    assert_eq!(posts, 200);

    seed::clear();
}
```

契約上の重要な詳細が2つあります:

**シーダーレジストリはプロセスグローバルです。** `seed::register::<S>()` は、`S::name()` をキーとする `RwLock<IndexMap>` へ挿入します。レジストリを変更するテストは、入口で `seed::clear()` を呼び、必要なシーダーを登録し、実行し、出口で再び `clear()` を呼ぶべきです - そして、2つの並行テストがレジストリを奪い合わないよう、テスト自身が `#[serial_test::serial]` であるべきです。`#[suprnova_test]` はシーダーを自動登録**しません** - あなた自身の `bootstrap.rs` かテスト本体の中での、明示的な `seed::register::<>()` の呼び出しだけが、それらをレジストリに入れます。

**モデル駆動のシードと、ファクトリー駆動のシード。** `for` の中で `User::create(...)` をループするシーダーは、行ごとに `Creating` / `Saving` / `Created` / `Saved` を発火し、登録済みのすべてのオブザーバーを呼び出します。そのファンアウトが望ましくない一括シードには、ループを `seed::without_events` でラップしてください:

```rust
seed::without_events(async {
    for i in 0..50 {
        User::create(attrs! { name: format!("user{i}"), email: format!("user{i}@example.com") }).await?;
    }
    Ok::<_, FrameworkError>(())
}).await?;
```

この抑制は**タスクスコープ**です - futureの内側で実行される作業だけが無音化されます。並行するリクエストハンドラやキューワーカーは、通常どおりイベントを発火し続けます。ファクトリー（`create_many`）はすでにイベント経路を迂回しているため、それらを `without_events` で囲む必要はありません。

シーダーの書き方の表面については[シーディング](seeding.md)を、両者の関係については[Eloquent → ファクトリー](eloquent-factories.md)を参照してください。

## 並列に対して安全なデータベーステスト

`cargo test` は、スレッドによってテストを並列に実行します。デフォルトの `#[suprnova_test]` の展開（つまり `#[tokio::test]` - テストごとの `current_thread` ランタイム）は、2つの理由でこれと安全に相互作用します:

- **各テストは、自分自身の `sqlite::memory:` 接続を手に入れます。** テストはDBの状態を共有しません。
- **束縛された接続は、スレッドローカルな `TestContainer` の中に存在します。** テストはコンテナのバインディングを共有しません。

考えなくてよいこと: `DB::connection()`、`App::resolve`、ファクトリーの永続化、モデルトレイトの書き込み - これらはすべて、透過的にテストごとの正しいデータベースへ行き着きます。

*本当に*考える必要があること:

| 表面 | プロセスグローバルである理由 | 対処法 |
| --- | --- | --- |
| `ConnectionRegistry`（`DB::register_named`、`__read_replica__`） | プロセスで共有される単一の `RwLock<HashMap>` | 名前付き接続を登録または読み取るあらゆるテストに `#[serial_test::serial]` |
| シーダーレジストリ | 単一の `RwLock<IndexMap>` | 入口と出口での `#[serial_test::serial]` + `seed::clear()` |
| Eloquentのオブザーバー / スコープレジストリ | `TypeId::<M>()` をキーとする | 各テストは一意なモデル構造体を使うべきか、`#[serial]` でレジストリの `clear()` ヘルパーを呼ぶべき |
| 名前付きクエリログ（`DB::enable_query_log`） | 単一のプロセスグローバルなリングバッファ | アサーションがログを読む場合は `#[serial]` |

接続レジストリの参照カウントは、これを聞こえるよりも安全にしています: `TestContainerGuard` を保持しているテストは、*兄弟*テストのガードがドロップしても、レジストリを生かし続けます。それでも、レジストリを実際に変更するテストには `#[serial]` が欲しいところです - そうすれば、それらの読み取りと書き込みが入り交じることはありません。

### マルチスレッドランタイムの注意点

`#[suprnova_test]` は、デフォルトの `current_thread` ランタイムを伴う `#[tokio::test]` へ展開されるため、スレッドローカルなコンテナの経路は常に機能します。テストを明示的にマルチスレッドランタイムへオプトインさせる場合:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_io_test() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // 問題: `tokio::spawn` でspawnされたタスクは、TestDatabaseを
    // 構築したものとは異なるワーカースレッド上で走ることがある。
    // それらは、スレッドローカルなTestContainerのバインディングを
    // 見ることができず、DB::connection()はグローバル（本番用）の
    // コンテナの値を返すか、エラーになる。
}
```

テストが何をするかによって、2つの修正があります:

1. **直接の接続アクセス** - `db.conn()` は、どのワーカースレッドがそれを読むかにかかわらず、正しい `&DatabaseConnection` を返し続けます。テストが（`DB::connection()` を経由せず）`db` ハンドルを通じてのみDBと話すのであれば、マルチスレッドランタイムでも問題ありません。

2. **`TestContainer::scope`** - テスト本体を `TestContainer::scope(async { ... }).await` でラップし、その内側でフェイク（とDB接続）をバインドしてください。このスコープはコンテナをタスクローカルな層へ束縛します。これは、ランタイムがfutureをワーカースレッド間で飛び越えさせても、awaitをまたいで保持されます。spawnされたサブタスクには、（裸の `tokio::spawn` ではなく）`TestContainer::spawn` を使ってください - そうすれば、タスクローカルなコンテナがキャプチャされ、spawnされたfutureの内側に再インストールされます。

タスクローカル / スレッドローカル / グローバルの層構造の全体については、[サービス コンテナ → ルックアップ順序](container.md)を参照してください。

## SQLiteのインメモリと、本物のPostgres / MySQL / MariaDB

`TestDatabase` は、意図的にSQLite専用です。ドライバーは `sqlite::memory:` にハードコードされています - `TestDatabase::postgres()`、`fresh_with_url()`、あるいは環境変数駆動のバリアントはありません。テスト表面の圧倒的多数 - モデルのCRUD、クエリビルダーの形、キャストの往復、リレーションのロード、オブザーバーの発火順序、ソフトデリートのセマンティクス - にとって、SQLiteのインメモリは正しい道具です: セットアップ不要、ネットワーク不要、テストごとにミリ秒、完璧な分離、CIで生かし続ける外部サービスも不要です。

SQLiteのインメモリが十分でないケースが4つあります:

1. **ドライバー固有のSQL。** Postgresの `LATERAL`、`JSONB` 演算子、`ON CONFLICT ... WHERE`、MySQLのウィンドウ関数、あるいはその他の方言固有の表面を使うクエリは、SQLite上では動きません。モデル+ビルダーの経路は汎用であろうとしますが、Postgres形の出力をアサートする生SQLのテストには、Postgresが必要です。
2. **本物の接続の競合下での並行性。** SQLiteのインメモリは単一接続です（「プールが1つの接続に固定されている理由」を参照）。2つのトランザクションを競合させる、負荷の下でリードレプリカへのルーティングを行う、あるいはデッドロックのリトライを測定するテストには、複数接続のサーバーが必要です。
3. **ベクター / NoSQL /時系列の表面。** SuprnovaのMariaDBの `VECTOR` ドライバー、Qdrant連携、Pinecone連携、そして類似の非SQLドライバーは、SQLiteの中でモデリングすることが一切できません。
4. **本番環境との整合性を確かめるスモークテスト。** 「これは、実際にデプロイ先の本物のDB上で本当に動くのか」を確かめる一握りのテストは、CIに限定してでも、ユニットテストの層がSQLiteである場合でも、残しておく価値があります。

この4つのケースすべてにおいて、パターンは同じです: `TestDatabase` の外に完全に出て、運用者が与える `DATABASE_URL` 形式の環境変数に対して `DbConnection` を構築し、変数が不在ならスキップするようにテストを環境変数でゲートし、2つが共有される本物のデータベースを奪い合わないよう `#[serial]` を付けてください。`framework/tests/vector_mariadb.rs` の `MARIADB_URL` パターンが、その規範的な例です:

```rust
use serial_test::serial;
use suprnova::database::{DatabaseConfig, DbConnection};

async fn maybe_real_db(test_name: &str) -> Option<DbConnection> {
    let url = match std::env::var("POSTGRES_TEST_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("[{test_name}] skipping: POSTGRES_TEST_URL not set");
            return None;
        }
    };
    let config = DatabaseConfig::builder().url(&url).build();
    Some(DbConnection::connect(&config).await.expect("real DB connects"))
}

#[tokio::test]
#[serial]
async fn jsonb_operator_works_against_postgres() {
    let Some(conn) = maybe_real_db("jsonb_operator_works_against_postgres").await else {
        return;
    };
    // Postgres固有のSQLを、`conn` に対して直接実行する。
}
```

恒常的な規約は次のとおりです: 環境変数の名前を対象ドライバーにちなんで付ける（`POSTGRES_TEST_URL`、`MYSQL_TEST_URL`、`MARIADB_URL`）、ローカルでスイートを実行している開発者にテストがスキップされたことが見える（サイレントに合格したのではなく）よう、スキップ行を出力する、そしてCIが配線できるよう、テストモジュール先頭のdocコメントに環境変数を文書化する、ことです。

## 実例

この章のすべてを組み合わせた、アプリ全体のドッグフードパターンです:

```rust
use app::migrations::Migrator;
use app::models::posts::Post;
use app::models::users::User;
use serial_test::serial;
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, seed, FrameworkError};

#[tokio::test]
#[serial]
async fn users_and_posts_full_seed_round_trip() {
    // 1. シーダーレジストリを空にする。
    seed::clear();

    // 2. アプリのマイグレーターを伴う、新しいインメモリDB。
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // 3. テストが関心を持つシーダーを登録する。
    seed::register::<app::seeders::UsersSeeder>();
    seed::register::<app::seeders::PostsSeeder>();

    // 4. オブザーバーのファンアウトがジョブをエンキューしようと
    //    しないよう（ここではキューが動いていない）、without_eventsの
    //    内側でシードを駆動する。
    seed::without_events(async {
        seed::run_all().await
    }).await.unwrap();

    // 5. モデルの表面と、生の接続の両方を通じて読み返す。
    let user_count = User::query().count().await.unwrap();
    assert_eq!(user_count, 50);

    let raw_post_count = db.fetch_one(
        "SELECT COUNT(*) AS n FROM posts",
        vec![],
    ).await.unwrap();
    let n: i64 = raw_post_count.try_get("", "n").unwrap();
    assert_eq!(n, 200);

    // 6. 新しいモデルに対して、キャンセル可能なオブザーバーの経路を働かせる。
    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    }).await.unwrap();
    assert!(alice.id > 0);

    seed::clear();
}
```

配線を証明しているのは、ステップ5の部分です: モデルのクエリと生の `fetch_one` はどちらも、同じインメモリデータベースを読んでいます - モデルの表面は、`DB::connection()` のルックアップが `TestContainer` のバインディングを見つけたからであり、生の `fetch_one` は、`db.conn()` が同じ接続を直接返すからです。

## 関連項目

- [テスト](testing.md) - テストハーネス、`expect!`、`describe!`、`test!`、フェイク。
- [データベース](database.md#testing) - `TestDatabase` を導入する、表面レベルのテストのセクション。
- [Eloquent → ファクトリー](eloquent-factories.md) - ファクトリー定義の構文、states、sequences、リレーション。
- [シーディング](seeding.md) - シーダーの書き方、順序、べき等性。
- [サービス コンテナ](container.md) - タスクローカル対スレッドローカル対グローバルなルックアップ。これが、テストの中で `DB::connection()` が何を解決するかを決めます。
- [モックとフェイク](mocking.md) - `Storage::fake`、`Mail::fake`、`Queue::fake`、`Notification::fake`、そして、フェイクのHTTPクライアントや他の外部表面を差し替えるためのトレイトバインドパターン。
- [HTTP テスト](http-tests.md) - `TestDatabase` を束縛した状態で、ルーティングスタックを通じてハンドラを駆動する。
