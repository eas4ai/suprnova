# テスト

これはSuprnovaのテスト表面のためのハブとなる章です - マクロ、プロセス内のデータベース、コンテナのフェイク、そしてあなたのテストバイナリが手を伸ばす暗号化キーのヘルパーです。深く掘り下げる章は、これと並んで存在します - ルート + ミドルウェアのための[HTTP テスト](http-tests.md)、`TestDatabase` を取り巻くすべてのための[データベース テスト](database-testing.md)、7つの外部表面（Mail、Notify、Queue、Bus、Events、Storage、HTTPクライアント）のための[モックとフェイク](mocking.md)です。箱の中に何が入っているかを知るにはこれを読んでください - 詳しい話が必要なときは兄弟の章へ進んでください。

## 構成要素

| 要素 | 役割 |
|---|---|
| `#[tokio::test]` + `TestDatabase::fresh::<Migrator>()` | 主力となるデフォルト - フレームワークのあらゆる本物のテストがこれを使います |
| `#[suprnova_test]` | アトリビュートマクロのシンタックスシュガー - `App::init()` + `App::boot_services()` を実行し、あなたのために `TestDatabase` を構築します |
| `describe!` + `test!` | Jest形のグルーピングマクロで、名前付きの失敗出力のために `expect!` と組み合わせて使います |
| `expect!` | 型付きのマッチャー（等価性、Option、Result、文字列、Vec、順序）を持つ、流れるようなアサーションマクロ |
| `TestDatabase::fresh` / `sqlite_memory` | インメモリのSQLite + コンテナ登録で、あなたのマイグレーターの有無を問いません |
| `TestContainer::fake` / `scope` / `spawn` | スレッドローカルまたはタスクローカルなDIの上書きで、並列テストをまたいで完全に分離されます |
| `install_test_encryption_key[ring]` | 暗号化されたキャストや署名付きのペイロードに触れるテストのための、決定的な `APP_KEY` |
| 表面ごとの `fake()` ヘルパー | Mail、Notify、Queue、Bus、Events、Storage、HTTP - [モックとフェイク](mocking.md)を参照してください |
| `TestResponse` | HTTPテストの `(status, headers, body)` トリプルに対するフルーエントなアサーション - [HTTPテスト](http-tests.md#fluent-response-assertions-with-testresponse)を参照してください |
| `AssertableInertia` | Inertiaページオブジェクトに対するフルーエントなアサーション - [HTTPテスト](http-tests.md#testing-inertia-responses)を参照してください |

1つのテストであらゆるものに手を伸ばすことはありません。典型的なアクションのテストは最初の3つを使い、DIの多いテストは `TestContainer` を加え、HTTPのテストは `TestDatabase` を `handle_request` パイプラインに差し替え、決済のテストは暗号化キーリングをインストールします。

## 主力となる標準パターン

フレームワークのあらゆる本物のテストは、次のような形をしています。

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn create_user_persists_it() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice",
        email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);

    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`TestDatabase::fresh::<M>()` は、新しい `sqlite::memory:` コネクションを開き、あなたのマイグレーターをエンドツーエンドで実行し、そのコネクションをテストコンテナへ登録します。その後で `DB::connection()` や `App::resolve::<DbConnection>()` を呼ぶあらゆるコードは、`#[suprnova::model]` のクエリビルダーやコンテナから解決したあらゆるサービスを含め、それへ解決されます。`TestDatabase` がドロップすると、この登録もそれと共に消えます。

`test_database!()` マクロは、`crate::migrations::Migrator` のケースのための、1行のシンタックスシュガーです。

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();         // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}
```

正確なカラムの形の制御（キャストの往復、クエリビルダーのSQL表面）を求めるテストには、`TestDatabase::sqlite_memory()` を使ってください - 同じコンテナの配線ですが、マイグレーターはありません。DDLはあなた自身のものです。完全なカタログと `execute_unprepared` / `fetch_one` / `fetch_all` ヘルパーについては、[データベース テスト](database-testing.md)を参照してください。

## `#[suprnova_test]` - シンタックスシュガーが欲しいとき

`#[suprnova_test]` は、`#[tokio::test]` をラップするアトリビュートマクロで、`App::init()` + `App::boot_services()` を呼び出して `#[injectable]` の型が解決できるようにし、新しい `TestDatabase` をバインドします。これは上の明示的な形の上に被せる任意のシンタックスシュガーであり、テストがコンテナに登録されたサービスを解決するときに便利です。

```rust
use suprnova::suprnova_test;
use suprnova::{App, testing::TestDatabase};

#[suprnova_test]
async fn create_user_via_action(db: TestDatabase) {
    let action = App::resolve::<CreateUserAction>().unwrap();
    let user = action.execute("test@example.com").await.unwrap();

    assert_eq!(user.email, "test@example.com");
    assert!(user.id > 0);
}
```

関数が（名前による）`TestDatabase` パラメータを取る場合、マクロはその新しいデータベースをその名前へバインドします。取らない場合でも、データベースはそれでも構築され登録されます（そのため `DB::connection()` は機能します）- ただ、ローカル変数へバインドされないだけです。

`migrator = …` キーでマイグレーターを上書きできます。

```rust
#[suprnova_test(migrator = my_crate::tests::IsolatedMigrator)]
async fn create_user_with_isolated_schema(db: TestDatabase) {
    // ...
}
```

未知のキーはコンパイルエラーになります（`migrtor = …` というタイプミスが、無音でデフォルトのマイグレーターを使い続けることはありません）。

## `describe!` と `test!` - グルーピングが役立つとき

同じアクションが多くのケースを持つテストファイルのために、Jest形の `describe!` + `test!` の組は、入れ子になったグルーピングと名前付きの失敗出力を与えてくれます。

```rust
use suprnova::{App, describe, test, expect, testing::TestDatabase};
use crate::migrations::Migrator;

describe!("ListTodosAction", {
    test!("returns empty list when no todos exist", async fn(db: TestDatabase) {
        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_be_empty();
    });

    test!("returns all todos", async fn(db: TestDatabase) {
        Todo::create(attrs! { title: "Buy bread" }).await.unwrap();
        Todo::create(attrs! { title: "Walk dog" }).await.unwrap();

        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_have_length(2);
    });

    describe!("with pagination", {
        test!("returns first page", async fn(db: TestDatabase) {
            // 入れ子になったグループも組み合わせられます
        });
    });
});
```

`test!` は3つの形を受け付けます。

```rust
// TestDatabaseパラメータを伴う非同期テスト
test!("creates a user", async fn(db: TestDatabase) { … });

// データベースを伴わない非同期テスト
test!("calculates the right sum", async fn() { … });

// 同期テスト
test!("adds numbers", fn() { … });
```

名前付きテストのラッパーは、失敗が表面化するとき、テスト名を `expect!` の仕組みへ通します。

```text
Test: "returns all todos"
  at src/actions/todo_action.rs:25

  expect!(actual).to_equal(expected)

  Expected: 2
  Received: 0
```

`describe!` / `test!` がなければ、標準の `panic!` の出力になります。それらがあれば、場所と人間に読みやすいテスト名がメッセージの先頭に来ます。

## `expect!` - マッチャーカタログ

`expect!(value)` は `Expect<T>` というラッパーを返します。マッチャーは `T` に対して型付けられています - `String` に対して `to_be_some()` を呼ぶのはコンパイルエラーであり、実行時のパニックではありません。

```rust
use suprnova::expect;

// 等価性 (T: Debug + PartialEq)
expect!(actual).to_equal(expected);
expect!(actual).to_not_equal(unexpected);

// ブーリアン
expect!(condition).to_be_true();
expect!(condition).to_be_false();

// Option<T>
expect!(option).to_be_some();
expect!(option).to_be_none();
expect!(option).to_contain_value(5);     // Some(5) のチェック

// Result<T, E>
expect!(result).to_be_ok();
expect!(result).to_be_err();

// String / &str
expect!(s).to_contain("substring");
expect!(s).to_start_with("prefix");
expect!(s).to_end_with("suffix");
expect!(s).to_have_length(10);
expect!(s).to_be_empty();

// Vec<T>
expect!(v).to_have_length(3);
expect!(v).to_contain(&item);
expect!(v).to_be_empty();

// 順序 (T: Debug + PartialOrd)
expect!(10).to_be_greater_than(5);
expect!(5).to_be_less_than(10);
expect!(10).to_be_greater_than_or_equal(10);
expect!(5).to_be_less_than_or_equal(5);
```

`expect!` は `test!` の外側でも使えます - 失敗メッセージの中のファイル/行は、`concat!(file!(), ":", line!())` から来ます。このマクロが自分では追加しないのは、名前付きテストのヘッダーだけです。

## `TestContainer` - 漏れないDIフェイク

コンテナの章では、[3層のルックアップ](container.md)について詳しく扱っています。テストのための2つの入口は、`TestContainer::fake()`（スレッドローカル）と `TestContainer::scope(…).await`（タスクローカル）です。

### スレッドローカル - よくあるケース

`TestContainer::fake()` はガードを返します。ガードがドロップするまで、`TestContainer::singleton` / `bind` / `factory` への書き込みは、スレッドローカルな上書き層に着地し、グローバルコンテナを覆い隠します。

```rust
use std::sync::Arc;
use suprnova::App;
use suprnova::testing::TestContainer;

#[tokio::test]
async fn order_dispatches_email() {
    let _guard = TestContainer::fake();

    let fake = Arc::new(FakeEmailGateway::new());
    let probe = Arc::clone(&fake);
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.unwrap();

    assert_eq!(probe.sent_count(), 1);
}
```

`TestDatabase::fresh` / `sqlite_memory` は、内部で独自の `TestContainer::fake` ガードをインストールします - レジストリそのものをテストしているのでない限り、これらを重ねて使う必要はありません。

### タスクローカル - `multi_thread` ランタイム向け

スレッドローカルな層は、`fake()` を呼んだどちらかのOSスレッドの上に設定されます。`multi_thread` のtokioランタイムは、`.await` をまたいであなたのfutureを別のワーカースレッドへ移動させることがあり、そうなると上書きは静かに消えてしまいます。`TestContainer::scope` は、代わりに上書きをfutureへ束縛することでこれを解決します。

```rust
use suprnova::testing::TestContainer;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_worker_safe() {
    TestContainer::scope(async {
        TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
        do_async_work_that_may_hop_workers().await;
    })
    .await;
}
```

`tokio::spawn` でspawnされたサブタスクは、tokioのタスクローカルを継承しません - 代わりに `TestContainer::spawn` を使ってください。これは現在のスコープのコンテナを捕捉し、spawnされたfutureの内側でそれを再インストールします。

```rust
TestContainer::scope(async {
    TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
    let h = TestContainer::spawn(async {
        App::make::<dyn HttpClient>().unwrap()  // フェイクが見えます
    });
    let _client = h.await.unwrap();
})
.await;
```

### なぜ `FAKE_GUARDS` という参照カウントがあるのか

スレッドローカルなコンテナはテストごとのものですが、Suprnovaには、名前でキー付けされた（`__read_replica__`、カスタムのコネクションラベル）プロセスグローバルな `ConnectionRegistry` もあり、これはスレッドローカルのリセットを生き延びます。素朴な `Drop` 実装であれば、*どの* `TestContainerGuard` がなくなるときも `ConnectionRegistry::clear()` を呼んでしまい、実行中の別の並行テストの名前付きコネクションを、その途中で消し去ってしまうでしょう。

その修正は、プロセス全体の `AtomicUsize`（`FAKE_GUARDS`）です。`fake()` はそれをインクリメントし、`drop` はデクリメントし、ゼロへ戻る遷移だけが名前付きレジストリをクリアします。`__read_replica__` を使う2つの並列テストは安全です - 最後にドロップするガードが、そのクリアを担います。

これをテストから呼び出すことはありません - `TestContainerGuard` の `Drop` から実行されます。これがあることを知る必要があるのは、「名前付きコネクションがテストの途中で消えた」という症状をデバッグしているときだけです。それは通常、兄弟テストが自分自身のガードがドロップするのを待つのを忘れた、ということを意味します。

## 暗号化キーのテストヘルパー

暗号化されたキャスト（`#[model(...)]` 上の `casts = { secret = AsEncrypted }`）、署名付きのペイロード、あるいはキーリングの以前のキーへのフォールバックを行使するテストは、プロセス内にインストールされた `APP_KEY` を必要とします。フレームワークは、`testing` フィーチャーの下に、テスト専用の2つのヘルパーを出荷しています。

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn cast_roundtrip() {
    install_test_encryption_key();   // べき等です。決定的な32バイトのゼロキーです
    let db = TestDatabase::sqlite_memory().await.unwrap();
    // … 暗号化して読み戻す …
}
```

`install_test_encryption_key` はべき等です - その裏にある `Crypt` ファサードは `OnceLock` に支えられているため、2回目の呼び出しはno-opです。ほとんどのキャストのテストバイナリは、暗号化されたキャストに触れるあらゆるテストからこれを呼び出します - 最初の呼び出しが勝ち、残りは無償です。

ローテーションのテスト（古いキーの下で書き込み、新しいキーの下で読み取り）には、キーリングのバリアントを使ってください。

```rust
use suprnova::crypto::EncryptionKey;
use suprnova::testing::install_test_encryption_keyring;

let new = EncryptionKey::from_base64("...").unwrap();
let old = EncryptionKey::from_base64("...").unwrap();
let installed = install_test_encryption_keyring(new, vec![old]);
assert!(installed, "first install wins");
```

このキーリングのヘルパーは、その呼び出しが実際にリングをインストールした場合（`OnceLock` が空だった場合）だけ `true` を返します。ローテーションのテストのために任意のキーの下で暗号文を作るには、2回インストールするのではなく `suprnova::crypto::_test_encrypt_with` を使ってください。

どちらのヘルパーも、cryptoレイヤーでは `#[doc(hidden)]` であり、`testing` モジュールの下に再エクスポートされています - これらはテスト専用であり、本番の `APP_KEY` バリデーションの経路を回避します。

## `testing` フィーチャーと本番ビルド

`suprnova` は、テスト用のヘルパー（`Storage::fake()`、`TestContainer`、`TestDatabase`、`_test_install_key` のような暗号ローテーションのフック）を、`testing` という名前のCargoフィーチャーの背後に公開します。このフィーチャーはデフォルトの集合に含まれているため、利用側のテストスイートは、それらを何もせずに手に入れられます:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.0" }

[dev-dependencies]
# `testing` は上の依存を介して推移的に有効です - 追加は何も要りません。
```

これらのフックは `#[doc(hidden)]` であり、`_test_` が前置されているため、フィーチャーが有効なときでも、イディオマティックなアプリケーションのコードからは手が届きません。荷重を支える保護は `Server::from_config` です: これは、キーリングが未初期化のときだけでなく、**すべての**起動で `APP_KEY` を検証します。あらかじめインストールされたテスト用のキーが、このチェックを迂回することはできません - プロセス内で何かがキーを事前インストールしていたかどうかにかかわらず、`APP_KEY` が欠けているか不正な形式であれば、起動はフェイルファストします。

ヘルパーが本番のアーティファクトへまったくリンクされないほうがよいなら（多層防御です）、デフォルトフィーチャーをオフにして `suprnova` に依存し、出荷するものだけを有効にしてください:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.0", default-features = false, features = ["..."] }

[dev-dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.0", features = ["testing", "..."] }
```

これは修正ではなく引き締めです - どちらの姿勢を選ぼうと、実際の攻撃口を塞ぐのは起動時の検証です。

### Suprnovaが異なる設計を選んだ理由

LaravelのPHPのテストハーネスは、並列テストの隔離をほとんど何もせずに手に入れます。ランタイムがリクエストごとにシングルスレッドであり、テストがファイルごとに新しいプロセスをforkするからです。Suprnovaのテストバイナリは、1つ以上のワーカースレッド上で多数の `#[tokio::test]` を並行して走らせる、1つのプロセスです。単一のグローバルなコンテナは、あるテストのフェイクが、ワーカースレッド上で重なった瞬間に次のテストのルックアップへ漏れ出すことを意味します。

だからこそ `TestContainer` には2つの風味があります - よくある `current_thread` のケースにはスレッドローカル、`multi_thread` にはタスクローカルです。プロセスグローバルな `ConnectionRegistry` に対する、参照カウントされた `FAKE_GUARDS` のクリアが存在するのも、同じ理由からです: テストごとにできない共有状態は、少なくとも、他のテストがまだそれに寄りかかっている間に自分自身を消し去らないことを知っていなければなりません。

マッチャーのカタログ（`expect!`）が型付きなのは、Rustがそれを許すからです。Jestの `expect(x).toBeSome()` は、`x` が `Option` かどうかを実行時にしか知りません。Suprnovaの `Expect<T>` はコンパイル時に知っているため、間違ったマッチャーは、不安定なテストではなくビルドエラーになります。

## 各要素の実装場所

| 要素 | ソース |
|---|---|
| `#[suprnova_test]` アトリビュートマクロ | `suprnova-macros/src/suprnova_test.rs` |
| `describe!` / `test!` プロシージャルマクロ | `suprnova-macros/src/describe.rs`、`test_macro.rs` |
| `expect!` マクロ + `Expect<T>` マッチャー | `framework/src/lib.rs`（マクロ）、`framework/src/testing/expect.rs`（実装） |
| `TestDatabase::fresh` / `sqlite_memory` / ヘルパー | `framework/src/database/testing.rs` |
| `test_database!` マクロ | `framework/src/database/testing.rs` |
| `TestContainer` + `TestContainerGuard` + `FAKE_GUARDS` | `framework/src/container/testing.rs` |
| `install_test_encryption_key[ring]` | `framework/src/testing/mod.rs` |
| 表面ごとのフェイク（Mail、Notify、Queue、Bus、Events、Storage、HTTP） | ドメインごとの `testing` サブモジュール - [モックとフェイク](mocking.md)を参照してください |
| `TestResponse` | `framework/src/testing/response.rs` |
| `AssertableInertia`、`ReloadRequest` | `framework/src/testing/inertia.rs` |

## テストを実行する

標準のcargoの呼び出しがそのまま使えます。

```bash
# ワークスペース全体
cargo test --workspace

# 1つのクレート
cargo test -p suprnova

# 名前による1つのテスト（部分文字列マッチ）
cargo test create_user_persists_it

# println! と dbg! の出力付き
cargo test -- --nocapture
```

Suprnovaは自身のテストランナーを出荷しません - フレームワークはcargoのものと統合します。データベースのテストはデフォルトで並列に実行されます - スレッドローカルなコンテナと、テストごとのインメモリSQLiteは、まさにそのために設計されています。

## 次のステップ

- [HTTP テスト](http-tests.md) - `handle_request` を通じて完全なリクエストパイプラインを駆動する
- [データベース テスト](database-testing.md) - `TestDatabase`、テストの中でのファクトリー、テストの中でのシーダー、並列に対して安全なDBテスト
- [モックとフェイク](mocking.md) - 7つの外部表面のフェイクと、それらが共有するパターン
- [サービス コンテナ](container.md) - `TestContainer` が上書きする3層のルックアップ
- [エラー モデル](error-model.md) - あなたがアサートすることになる `FrameworkError` の形
