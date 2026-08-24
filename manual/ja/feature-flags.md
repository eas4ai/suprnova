# フィーチャー フラグ

Suprnovaのフィーチャーフラグシステムは、コンパイル時の `Feature` 宣言と、`features` テーブルへ永続化されるランタイムのオーバーライドを組み合わせます。評価時におけるフラグの値は、次の順序で決まります:

1. `features` テーブル内のスコープ付きの行 - `user:42` や `team:staff` です。
2. `features` テーブル内のグローバルな行（スコープ `""`）です。
3. `Feature` 宣言に焼き込まれたコンパイル時の `default` です。

管理者CRUDを介したトグルは、ミューテーション呼び出しが返る前に、稼働中のエバリュエーターへ伝播します。キルスイッチ用のフラグは、「次のTTLウィンドウ以内」ではなく、実際にリアルタイムで無効化されます。

## クイックスタート

```rust
// app/src/features.rs - アプリが参照するすべてのフラグはここに置きます。
use suprnova::features::Feature;

pub const NEW_CHECKOUT_FLOW: Feature<'static> = Feature::new("new-checkout-flow", false);
```

```rust
// app/src/bootstrap.rs - 起動中に一度だけチェーンを配線します。
use std::time::Duration;
use suprnova::features::{bootstrap_database_cached, FeatureMiddleware};

pub async fn register() {
    // ... DB::init、セッションなど

    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature flags wired");

    global_middleware!(FeatureMiddleware::new());
}
```

```rust
// 任意のハンドラ - Feature::is_enabled() はリクエストごとのコンテキストに対して解決されます。
use crate::features::NEW_CHECKOUT_FLOW;

pub async fn index(req: Request) -> Response {
    let banner = if NEW_CHECKOUT_FLOW.is_enabled() {
        Some("Try the new checkout - faster, fewer steps.")
    } else {
        None
    };
    // ...
}
```

```rust
// 管理者ルートやCLIからフラグを切り替えます:
use suprnova::features::admin;

let actor_id = Auth::id();  // Option<String> - システム起因の変更ではNone
admin::upsert("new-checkout-flow", "", true, None, actor_id).await?;
//                                  ^   ^                  ^
//                                  |   |                  └ 監査: 誰が切り替えたか
//                                  |   └ enabled
//                                  └ scope_key: "" = グローバル、"user:42" = スコープ付きオーバーライド
```

次の `NEW_CHECKOUT_FLOW.is_enabled()` 呼び出しは `true` を観測します - `admin::upsert` の内部で同期的に無効化された、キャッシュ済みのエバリュエーターのエントリも含めてです。

## 構成要素

### `Feature<'a>`

コンパイル時の宣言です。フラグ名と、値が存在しないときのデフォルト値を運びます。

```rust
pub const KILL_SWITCH_PAYMENTS: Feature<'static> =
    Feature::new("kill-switch.payments", true);
//                                      ^ デフォルト: true（無効化されるまで支払いは有効）
```

すべての宣言を `app/src/features.rs` に集約すると、次が得られます:

- オペレーターに「どんなフラグが存在するのか」と聞かれたときにgrepする、単一の場所
- フラグ名のコンパイル時の一意性 - 呼び出し側でのタイプミスはコンパイルが通りません
- そのフラグが何を制御するのかを説明するdocコメントを置く、自明の場所

[`FeatureMiddleware`](#featuremiddleware)がセットアップする周囲のコンテキストに対して読み取るには `flag.is_enabled()` を、特定の[`Context`](https://docs.rs/featureflag/latest/featureflag/context/struct.Context.html)を渡すには `flag.is_enabled_in(Some(&ctx))` を呼んでください。

定数をインポートしたくない呼び出し側のために、`feature!` と `is_enabled!` マクロも `suprnova::*` から再エクスポートされています:

```rust
use suprnova::is_enabled;

if is_enabled!("new-checkout-flow", false) {
    // ...
}
```

### `DatabaseEvaluator`

起動時、および[`reload()`](#フロー制御-フラグの伝播)のたびに、`features` テーブルをインメモリのスナップショットへ読み込みます。ホットパス（`is_enabled`）は完全に同期的です - リクエストごとのDBクエリも、エバリュエーターの内側での `block_on` もありません。

ルックアップ時の解決順序は、最も具体的なものが先です:

1. `user:{id}` - リクエストコンテキストが `UserIdField` を運んでいる場合。
2. `team:{name}` - コンテキストが `TeamField` を運んでいる場合。
3. `""` - グローバルフラグです。
4. `None` - 行が存在せず、コンパイル時のデフォルトが引き継ぎます。

### `CachedEvaluator`

`(feature, user, team)` のルックアップを、あなたが選ぶTTLを伴う `DashMap` の裏側でメモ化します。ホットパスは同期のままです。[`admin::upsert`](#管理者crud)がフラグを書き込むと、エントリは同期的に破棄されます。

TTLがゼロの場合は「キャッシュなし」に縮退します - すべての呼び出しは内側のエバリュエーターへフォールスルーします。キャッシュなしで伝播の配線だけが欲しい、フラグ数の少ないアプリに便利です。

### `FeatureMiddleware`

ユーザー定義のエクストラクターによって値を埋められる、リクエストごとのフィーチャーフラグコンテキストを開きます。デフォルト:

- `user_id` - `Auth::id()` から取得します。
- `team` - なしです。

ビルダー経由でどちらも上書きできます:

```rust
let middleware = FeatureMiddleware::new()
    .with_user_id_extractor(|req| {
        // カスタム: セッションではなくヘッダーから取得します。
        req.header("X-User-Id").map(String::from)
    })
    .with_team_from_header("X-Team");
// あるいは: .with_team_extractor(|req| your_custom_team_resolver(req))

global_middleware!(middleware);
```

### 管理者CRUD

`suprnova::features::admin` は `features` テーブルの永続化層です。管理者ハンドラ、CLIツール、デプロイスクリプトなど、フラグを切り替える必要があるあらゆる場所から使ってください:

```rust
use suprnova::features::admin;

// グローバルフラグを作成、または更新します。
admin::upsert("kill-switch.payments", "", false, Some("ops-2026-05-19".into()), actor_id).await?;
// 引数: name、scope_key、enabled、description、actor_id

// ユーザースコープのオーバーライド（グローバルより優先されます）。
admin::upsert("new-checkout-flow", "user:42", true, None, actor_id).await?;

// 行をまるごと削除します - フラグはコンパイル時のデフォルトへフォールバックします。
admin::delete("kill-switch.payments", "", actor_id).await?;

// 管理者UIのテーブル用に読み取ります。
let all_flags = admin::list().await?;
let one_row = admin::get("kill-switch.payments", "").await?;
```

すべてのミューテーションは対応する[イベント](#イベント)を発し、[`features::sync::notify`](#フロー制御-フラグの伝播)を呼び出すため、Appコンテナに束縛された稼働中のエバリュエーターはすべて、呼び出しが返る前に更新されます。

`actor_id: Option<String>` は監査用のポインタです。オペレーターのユーザーid（あなたの認証層が発行しているのと同じもの）を渡してください。システム起因の変更（CLI、デプロイのマイグレーションなど）では `None` のままにしてください。

## フロー制御: フラグの伝播

「管理者のトグルが即座に見える」を成立させているトレイトです:

```rust
#[async_trait]
pub trait FeatureSync: Send + Sync + 'static {
    async fn on_flag_changed(&self, feature: &str, scope_key: &str);
}
```

実装はミューテーションに反応します:

- `DatabaseEvaluator::on_flag_changed` は `self.reload()` を呼びます - 完全なスナップショットを取得します。
- `CachedEvaluator::on_flag_changed` は `self.invalidate(feature)` を呼びます - その名前のキャッシュ済みエントリをすべて破棄します。

正典となる連鎖は `CompositeFeatureSync` であり、**データソースをキャッシュより前に並べます** - キャッシュは、データソースが更新された*あとに*無効化されなければなりません。そうしなければ、並行する読み取りが空のキャッシュに当たり、古いデータソースへフォールスルーし、キャッシュへ古い値を再投入してしまうことがあります。

```rust
let composite = CompositeFeatureSync::new(
    vec![database.clone() as Arc<dyn FeatureSync>], // データソースが先
    vec![cached.clone() as Arc<dyn FeatureSync>],   // キャッシュが後
);
App::bind::<dyn FeatureSync>(composite);
```

`features::sync::notify(feature, scope_key)` は、コンテナから `Arc<dyn FeatureSync>` を解決し、`on_flag_changed` をawaitします。syncが束縛されていない場合は何もしません - DBに書き込むだけで、更新すべき稼働中のエバリュエーターを持たない、プロセス外の管理者ツールにとって正しい振る舞いです。

## Bootstrapヘルパー

`bootstrap_database_cached(ttl)` は、一度の呼び出しですべてを配線します:

```rust
let features = bootstrap_database_cached(Duration::from_secs(60))
    .await
    .expect("feature flags wired");

// 任意: 定期的な再読み込みのスケジューリングや管理者向けの差分ビューの公開のために、
// features.database を保持しておくこともできます。ほとんどのアプリはこのハンドルを
// 手放し、notify駆動の更新に仕事をさせます。
```

これが行うこと:

1. プライマリのDB接続に対して `DatabaseEvaluator` を構築します。
2. 要求されたTTLで `CachedEvaluator` に包みます。
3. `install_evaluator(cached)` を呼びます - グローバルなフィーチャーフラグのデフォルトを設定し、*さらに*フレームワーク所有の「インストール済み」トラッカーを立てて、ミドルウェアが「エバリュエーターがない」という警告をログに出さないようにします。
4. 正しいスロット順序で `CompositeFeatureSync` を構築し、Appコンテナへ束縛します。

どちらの層への直接のハンドルも欲しい呼び出し側のために、`BootstrappedFeatures { database, cached }` を返します。

あなたのトポロジーが `Cached(Database)` でない場合 - Redisバックのキャッシュ、リモートの同期ソース、多層のチェーンなど - は、同じプリミティブを使ってチェーンを手動で配線してください。`bootstrap_database_cached` は利便性のためのものであり、契約ではありません。

## マイグレーション

フレームワークが `features` テーブルのスキーマを所有します:

```rust
// app/src/migrations/mod.rs
vec![
    // ... あなたのアプリのマイグレーション ...
    Box::new(suprnova::features::migrations::CreateFeaturesTable),
]
```

スキーマ:

```sql
features (
    id          BIGINT      PRIMARY KEY AUTO_INCREMENT,
    name        VARCHAR(255) NOT NULL,
    scope_key   VARCHAR(255) NOT NULL DEFAULT '',
    enabled     BOOLEAN     NOT NULL,
    description TEXT,
    updated_by  VARCHAR(255),
    created_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE INDEX (name, scope_key)
)
```

`scope_key` はスコープの種類をそのまま運びます（`"user:42"`、`"team:staff"`、グローバルなら `""`）。そのため、読み取り経路は一意インデックスに対する単一の文字列ルックアップのままです。

## ユーザーidとチームid

`UserIdField` と `TeamField` は、`featureflag::Context::extensions` に格納される型付きの拡張です。どちらも文字列型であるため、不透明なフレームワークまたはMagnetarのユーザーIDと、数値の `app_users.id` 値が、一つの評価形状を共有できます。

コンテキストを手動で（ミドルウェアの外側で）組み立てる場合:

```rust
use featureflag::context;
use std::sync::Arc;

let ctx = featureflag::evaluator::with_default(cached.clone(), || {
    // 文字列のユーザーid - UUID、ULID、その他の不透明な値、何でも構いません。
    context! { user_id = "01HZK6V3J7Q5G4P8X9N2D1B0M3".to_string(), team = "staff".to_string() }
});

// 数値のidも動きます - フレームワークが on_new_context 時に i64 → String へ強制変換します。
let ctx_numeric = featureflag::evaluator::with_default(cached.clone(), || {
    context! { user_id = 42_i64 }
});
```

## イベント

管理者CRUDの経路からは2つのイベントが発火します:

```rust
pub struct FeatureUpdated {
    pub name: String,
    pub scope_key: String,
    pub enabled: bool,
    pub actor_id: Option<String>,
}

pub struct FeatureDeleted {
    pub name: String,
    pub scope_key: String,
    pub actor_id: Option<String>,
}
```

監査ログ、Slackアラート、あるいは必要な下流のパイプラインへ流し込むために、フレームワークのイベントディスパッチャー経由でリスンしてください:

```rust
EventFacade::listen::<FeatureUpdated, _>(Arc::new(FlagChangeAuditor)).await;
```

**`is_enabled` は読み取り経路のイベントを発火しません。** フラグをチェックするすべてのリクエストが、チェックされたフラグの数だけイベント量を増やしてしまいます - ミューテーションの監査という話には合いますが、読み取り経路のトレーシングには法外です。あなたのデプロイがサンプリングされた読み取り経路の監査を必要とするなら、境界付きのログチャネル（Redisストリームかファンアウトキューかはスケールしだいで選んでください）へ記録する、カスタムのエバリュエーターを重ねてください。

## エバリュエーター未検出の検知

`FeatureMiddleware` はインストールされているが `install_evaluator` / `bootstrap_database_cached` を介してエバリュエーターが登録されていない場合、すべてのフラグは黙ってコンパイル時のデフォルトを返します - QAで捕まえるべき、深刻な設定ミスです。この状態を観測した最初のリクエストで、ミドルウェアはプロセスごとに正確に1回、`tracing::warn!` を発します:

```
WARN suprnova::features: FeatureMiddleware is in the stack but no feature-flag evaluator is installed.
     is_enabled!() calls will return compile-time defaults until features::bootstrap_database_cached(...)
     or features::install_evaluator(...) is called during app boot.
```

この切り替えは `AtomicBool::swap` を使うため、起動時の並行リクエストの殺到は、ワーカーごとに1回ではなく、単一の警告発行へと直列化されます。

## テスト

検証したいものに応じて、2つのパターンがあります。

### Featureを単体で単体テストする

同期クロージャの中でスタンドインのエバリュエーターをスコープするには、`featureflag::evaluator::with_default` を使ってください:

```rust
#[test]
fn flag_enabled_returns_new_path() {
    use featureflag::evaluator::with_default;
    use suprnova::features::DatabaseEvaluator;

    let flagger = Arc::new(tokio_test::block_on(async {
        let e = DatabaseEvaluator::new_in_memory().await.unwrap();
        e.set_flag("new-checkout-flow", "", true).await.unwrap();
        e
    }));

    with_default(flagger, || {
        assert!(crate::features::NEW_CHECKOUT_FLOW.is_enabled());
    });
}
```

`DatabaseEvaluator::new_in_memory()` はテスト専用のヘルパーで、独自のSQLiteを起動して `CreateFeaturesTable` を実行するため、テストは環境に依存しないままです。本番の経路では使わないでください。

### 伝播をエンドツーエンドで統合テストする

DBには `TestDatabase::fresh::<TestMigrator>()` を、FeatureSyncには（`App::bind` ではなく）`TestContainer::bind` を使ってください - そうしなければ、同一プロセス上の並列テストが、グローバルコンテナ経由で互いのバインディングを上書きしてしまいます:

```rust
#[tokio::test]
async fn admin_upsert_propagates_to_cached_chain() {
    use std::sync::Arc;
    use std::time::Duration;
    use suprnova::features::sync::FeatureSync;
    use suprnova::features::{admin, CachedEvaluator, CompositeFeatureSync, DatabaseEvaluator};
    use suprnova::features::migrations::CreateFeaturesTable;
    use suprnova::testing::{TestContainer, TestDatabase};

    struct TestMigrator;
    impl sea_orm_migration::MigratorTrait for TestMigrator {
        fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
            vec![Box::new(CreateFeaturesTable)]
        }
    }

    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();

    let database = Arc::new(DatabaseEvaluator::new().await.unwrap());
    let cached = Arc::new(CachedEvaluator::new(
        database.clone() as Arc<dyn featureflag::evaluator::Evaluator + Send + Sync>,
        Duration::from_secs(60),
    ));
    let composite = Arc::new(CompositeFeatureSync::new(
        vec![database.clone() as Arc<dyn FeatureSync>],
        vec![cached.clone() as Arc<dyn FeatureSync>],
    ));
    TestContainer::bind::<dyn FeatureSync>(composite);

    let ctx = featureflag::evaluator::with_default(cached.clone(), || {
        featureflag::context! { user_id = "user-42".to_string() }
    });

    assert_eq!(cached.is_enabled("new-feature", &ctx), None);
    admin::upsert("new-feature", "", true, None, None).await.unwrap();
    assert_eq!(cached.is_enabled("new-feature", &ctx), Some(true)); // 即座に伝播する
}
```

構成テストの全体像は `framework/tests/features.rs` を参照してください。

### Suprnovaが異なる設計を選んだ理由

Laravel Pennantは、すべてのフラグをオンデマンドでデータベースに対して解決します（リクエストごとの任意のドライバーレベルのメモ化を伴います）。PHPのリクエストごとにプロセスを立てるモデルでは、接続が専用でリクエストと共に死ぬため、リクエストごとのDBアクセスは安上がりです。

Suprnovaのプロセスモデルはその逆です - 1つの長時間稼働するバイナリが、何千もの並行リクエストを処理します。フラグをチェックするたびにDBへアクセスすれば、コネクションプールの負荷はフラグチェックの回数だけ倍増してしまいます。2層のチェーン（`DatabaseEvaluator` のスナップショット + `CachedEvaluator` のTTL）は、Rustらしい答えです: ホットパスはインメモリのデータに対して完全に同期的であり、`FeatureSync` トレイトは、ポーリングによる再読み込みなしに、オペレーター起因の変更へサブ秒の伝播を与えます。形はPennantと同じです - フラグを定義し、ハンドラの中でそれをチェックし、管理者ルートからそれを上書きします。ランタイムが違うから、配管が違うのです。

## 設計上の注意点

- **なぜasyncではなく同期のエバリュエーターなのか?** featureflagの `is_enabled` はホットパスです。非同期のエバリュエーターは、`block_on`（デッドロックしやすい）を強いるか、フラグの読み取りのたびにすべてのハンドラへ `.await` を押し付ける（エルゴノミクスの大惨事）ことになります。フレームワークは、`FeatureSync` によって非同期に更新されるインメモリのスナップショットを介して、同期と非同期を橋渡しします。

- **なぜ `Evaluator` を拡張するのではなく、別の `FeatureSync` トレイトなのか?** featureflagの `Evaluator` は上流のクレートが所有しており、私たちはそれにメソッドを追加できません。`FeatureSync` は、アプリが同じ具象型に実装する兄弟トレイトです。トレイトオブジェクトはAppコンテナに別で束縛されるため、プロセスは複数のエバリュエーターを重ねながらも、通知を正しくルーティングし続けられます。

- **なぜ `set_flag` は `DatabaseEvaluator` 上で `pub` なのか?** テストの利便性のためです。本番の書き込み経路は `admin::upsert` です。`set_flag` が存在するのは、テストが `EventFacade` のリスナーをセットアップせずにフラグを準備できるようにするためです。どちらの経路も `features::sync::notify` を呼ぶため、伝播の契約はどちらの場合も保たれます。

- **なぜ `FeatureRetrieved` イベントがないのか?** 量の問題です。1リクエストあたり10個のフラグをチェックするハンドラは、1リクエストあたり10個のイベントを発火します - 1,000 req/sのサービスであれば、それは1時間あたり3,600万イベントであり、どんな監査パイプラインのシグナル対ノイズ比をもはるかに超えます。出荷されるのはミューテーション経路の監査（`FeatureUpdated` / `FeatureDeleted`）です。読み取り経路のサンプリングが必要であれば、カスタムのエバリュエーターラッパーを介して上に重ねてください。

## 次のステップ

- [ミドルウェア](middleware.md) - `FeatureMiddleware` は `SessionMiddleware` の後に置きます。この章では順序とグローバルスタックを扱います
- [イベント](events.md) - `FeatureUpdated` / `FeatureDeleted` をリスンして、監査ログ、Slackアラート、下流のパイプラインを駆動すること
- [サービス コンテナ](container.md) - `dyn FeatureSync` の束縛がどのように解決されるか、そして並列テストのために `TestContainer::bind` が存在する理由
- [テスト](testing.md) - この章が頼っている `TestDatabase::fresh::<M>()` と `TestContainer::fake` のパターン
- [認証](authentication.md) - `Auth::id()` はデフォルトのuser-idエクストラクターであり、管理者ミューテーションのための `actor_id` を供給します

外部リンク: [featureflagクレートのドキュメント](https://docs.rs/featureflag)は、上流の `Evaluator`、`Context`、`Feature` のプリミティブを扱っています。`suprnova::features::admin` が完全なCRUDファサードです - `cargo doc --open -p suprnova` で参照してください。
