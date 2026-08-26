# シーディング

シーダーは、フィクスチャデータ - 本物のユーザーが何かをする前に、あなたのアプリが必要とする行 - をデータベースへ投入します。デフォルトの管理者アカウント、国の正規リスト、ステージング環境上のデモ投稿、あなたのローカル開発の反復ループが依存する50人のユーザー+200件の投稿を思い浮かべてください。これらは、[マイグレーション](migrations.md)のランタイム上の兄弟です: マイグレーションは空のスキーマを構築し、シーダーはそれを満たします。

シーダーは、`Seeder` トレイトを実装する、サイズ0の型です。フレームワークは、順序付けられたプロセスグローバルなレジストリを保持します。プロジェクトごとの `console db:seed` コマンドは、登録済みのすべてのシーダーを登録順に実行するか、`--class=<Name>` を介して1つの特定のシーダーを実行します。ほとんどのシーダーは、[モデルファクトリー](eloquent.md)を呼び出し、行の生成をファクトリーに任せる、数行のコードに行き着きます。

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

起動時に一度だけ、それを登録します:

```rust
// src/bootstrap.rs
suprnova::seed::register::<crate::seeders::UsersSeeder>();
```

それから:

```bash
cargo run --bin console -- db:seed
# running seeder UsersSeeder
# (50 rows inserted)
```

これが、ループの全体です。この章の残りは、レイアウトの規約、より大きなレジストリの構成パターン、`--class` によるターゲティングフラグ、ファクトリーとの統合、`without_events` の逃げ道、そして、シード対マイグレーション対ファクトリーをどう選ぶかについて扱います。

## シーダーを書く

シーダーは、ユニット型と `Seeder` の実装を合わせたものです。`name()` はレジストリのキーです（`db:seed --class=<Name>` が照合する対象でもあります）。`run()` は、挿入を実行する非同期fnです。

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

`Seeder` はクレートのルートで再エクスポートされているため、`use suprnova::Seeder` で十分です - `suprnova::seed::Seeder` まで踏み込む必要はありません。`async_trait` も再エクスポートされています（`use suprnova::async_trait`）。トレイトのメソッドがfutureを返し、Rustはそれなしではトレイトの中で `async fn` をまだ許可していないからです。

`FrameworkError` の戻り値の型は、フレームワークの他のあらゆる非同期表面が使うのと同じエラーのエンベロープです。ファクトリーの呼び出しや `Model::create` から `?` を伝播させるのが、期待される形です。分類の全体については[エラー モデル](error-model.md)を参照してください。

### レイアウトの規約

Laravelの `database/seeders/` ディレクトリを反映しますが、ソースのルートに置きます:

```
src/
├── bootstrap.rs
├── factories/
│   ├── mod.rs
│   ├── user_factory.rs
│   └── post_factory.rs
├── seeders/
│   ├── mod.rs              // pub mod base_seeder; pub use base_seeder::BaseSeeder;
│   └── base_seeder.rs      // Seeder実装、bootstrap.rsで登録される
└── …
```

ファイルは手で生成してください - `make:seeder` ジェネレーターはありません（これは、10行ほどのボイラープレートを持つファイルです）。シーダーが呼び出すファクトリーも、同じ扱いを受けます。

### 他のシーダーを実行するシーダー

モデルごとのシードを統率する、単一のトップレベルの `DatabaseSeeder::run` というLaravelのイディオムは、ここでも機能します。5つの小さなシーダーをbootstrapに登録し、その登録順序を信頼する代わりに、1つの合成シーダーを登録し、残りは自分で呼び出してください:

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // 先に50人のユーザー - postファクトリーはauthor_idを
        // 1..=50の範囲で生成するため、参照が解決される。
        UserFactory::new().count(50).create_many().await?;

        // 上のユーザーidを参照する200件の投稿。
        PostFactory::new().count(200).create_many().await?;

        Ok(())
    }
}
```

これが推奨されるデフォルトです。依存の順序（`posts` より前に `users`）を、bootstrapファイルのあちこちに散らばらせるのではなく、シーダーの内側に保ちます。そして、`db:seed --class=BaseSeeder` は、束全体を実行する単一ターゲットの呼び出しです。

直接のファクトリー呼び出しではなく、名前によってシーダーを連鎖させたい場合は、合成シーダーの内側から `seed::run_one` を使ってください:

```rust
async fn run() -> Result<(), FrameworkError> {
    suprnova::seed::run_one("UsersSeeder").await?;
    suprnova::seed::run_one("PostsSeeder").await?;
    suprnova::seed::run_one("CommentsSeeder").await?;
    Ok(())
}
```

サブシーダーは、`run_one` がそれらを見つけられるように、それでも `bootstrap.rs` に登録される必要があります。

## シーダーレジストリ

フレームワークは、登録済みのすべてのシーダーの、プロセスグローバルな順序付きマップ（`IndexMap<String, fn() -> _>`）を保持します。これを制御するつまみは3つです。

### `register::<S>()`

シーダーを、その `Seeder::name()` のもとでレジストリに追加します:

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

レジストリについて知っておくべきことが2つあります:

- **順序が重要です。** `run_all` は、登録された順序でシーダーを訪れます。`B` が `A` からの行を必要とするなら、先に `A` を登録してください。
- **同じ名前を再登録すると、その場で置き換わります。** スロットは元の位置を保ち、関数ポインタが変わります。これは意図的なものです - テストが、順序を動かすことなく、本物のシーダーの上にスタブのシーダーを束縛できるようにします。本番のコードでは、各シーダーを起動時にちょうど1回だけ登録してください。

### `run_all()`

登録済みのすべてのシーダーを、登録順に実行します。これは、裸の `console db:seed` の呼び出しが呼ぶものです。

```rust
suprnova::seed::run_all().await?;
```

最初のエラーで停止します。すでに実行されたシーダーはロールバックされません - `run_all` はバッチの周りにトランザクションを張りません。ほとんどのシーダーは複数のステートメントにまたがり、多くのバックエンドはトランザクションをきれいにネストできないからです。ロールバックのセマンティクスが必要な場合は、シーダーの内側でトランザクションを開き、その作業のすべてをそのスコープの内側に保ってください。

### `run_one(name)`

名前を指定した1つのシーダーを、他のシーダーを実行せずに実行します。これは `db:seed --class=<Name>` のエンジンであり、単発のスクリプトからも有用です:

```rust
suprnova::seed::run_one("AdminAccountSeeder").await?;
```

見つからない場合は `FrameworkError::not_found("no seeder registered for \`X\`")` を返します。コンソールコマンドは、それを非ゼロの終了コードとstderrの1行へ伝播させます - サイレントなno-opはありません。

### `count()` と `is_registered(name)`

2つの読み取り用ヘルパーです。どちらも、「bootstrapが期待されたシーダーを配線した」ことを主張するテストで有用です:

```rust
assert_eq!(suprnova::seed::count(), 3);
assert!(suprnova::seed::is_registered("BaseSeeder"));
```

どちらも、レジストリのロックがポイズニングされている場合は（エラーをログに記録した後で）ゼロ/falseを返します。これにより、上流のパニックに直面しても、テストは決定的なままです。

## `db:seed` コマンド

`db:seed` は、フレームワークが提供するコンソールコマンドです - フレームワークに同梱されており、あなた自身の `#[command]` を拾い上げるのと同じ `inventory` レジストリを通じて、あなたのプロジェクトの `console` バイナリへ自動的に行き着きます。バイナリの仕組みについては[コンソール](console.md)を参照してください。このセクションは、シーダー固有の表面をカバーします。

### すべてを実行する

```bash
cargo run --bin console -- db:seed
```

登録済みのすべてのシーダーを順番に実行します。レジストリが空の場合は、stderrへ警告を出力し（`db:seed: no seeders registered - nothing to run`）、ゼロで終了します - これは「誰かが何かを登録する前にコマンドを実行した」ことに対する正しい振る舞いであり、特定の何かをシードしていないテストスイートが失敗するのを防ぎます。

### 1つのシーダーを実行する

受け付けられる3つの形です。Laravelらしく感じられる度合いが増していく順に並んでいます:

```bash
cargo run --bin console -- db:seed --class=UsersSeeder
cargo run --bin console -- db:seed --class UsersSeeder
cargo run --bin console -- db:seed UsersSeeder
```

3つとも、正確な名前でレジストリの中のシーダーを調べ、それを実行します。

対象を絞った実行は、その進捗を報告します:

```text
  UsersSeeder .......................................................... RUNNING
  UsersSeeder ...................................................... 812 ms DONE

```

これらの行はstdoutへ行きます。素の `db:seed` は静かなままです - そうでなければ、フルのシードは、シーダーごとに1行を積み上げて、自分自身の出力を埋もれさせてしまうからです。各シーダーが発する `tracing` のレコードは変わっておらず、機械向けのチャネルであり続けます。

未知の名前は早く失敗します:

```bash
cargo run --bin console -- db:seed --class=NotARealSeeder
# Error: no seeder registered for `NotARealSeeder`
# (exit 1)
```

形式が不正なフラグ（値の続かない `--class`、空の値を持つ `--class=`、`--class --force`）も同様に早く失敗し、期待される形を名指しする診断メッセージを伴います。

### ビルド済みバイナリから

コンテナ化された、あるいはsystemdで管理されるデプロイでは、consoleバイナリは `target/release/console`（あるいは、あなたのリリースアーティファクトが行き着く場所）に存在します。構文は同じで、前に `cargo` は付きません:

```bash
./console db:seed
./console db:seed --class=BaseSeeder
```

consoleバイナリは `suprnova::console::dispatch_argv(std::env::args())` を呼び出し、これは `cargo run --bin console --` と同じレジストリを通じてルーティングされます。ビルド済みのアーティファクトのための、別個のディスパッチ経路はありません。

## ファクトリーとの組み合わせ

シーダーは、ほとんど常に[ファクトリー](eloquent.md)を呼び出すことに行き着きます。ファクトリートレイトは、1つのモデルのランダム化されたインスタンスを構築する方法を知っています。シーダーは、ファクトリーの呼び出しと、ランダム化できない配線（決定的な管理者の認証情報、結合テーブルの行、ファイルのアップロード）を順序立てます。

最小限のファクトリー+シーダーの組です:

```rust
// src/factories/user_factory.rs
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        User {
            id: 0,                              // persist_via_seaormがPKをNotSetへ切り替える
            name: "Factory User".into(),
            email: "factory@example.suprnova.app".into(),
            password: "factory-placeholder".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ..Default::default()
        }
    }
}
```

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

フルーエントビルダーは `FactoryBuilder<M>` の上に存在します。`create_many` の前に連鎖できるものは、Laravelと一致します:

```rust
// 上書きを伴う、永続化された1行を組み立てる:
let admin = UserFactory::new()
    .with(|u| u.email = "admin@example.com".into())
    .with(|u| u.role = "admin".into())
    .create()
    .await?;

// 永続化されたN行を組み立てる。すべて管理者:
UserFactory::times(5)
    .with(|u| u.role = "admin".into())
    .create_many()
    .await?;

// 条件付きのstate - フラグが立っているときだけクロージャを適用する:
UserFactory::times(10)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

`make` / `make_one` / `make_many` は、データベースへの往復を望まないユニットテストのための、インメモリの兄弟です（挿入なし）。完全なファクトリーの表面（`prepend`、`Sequence`、そして `#[factory(model = "…")]` アトリビュートからマーカー構造体を生成する `#[derive(Factory)]` マクロを含みます）については、[Eloquent](eloquent.md)の章を参照してください。

### べき等性はシーダーの責任

`run_all` はスナップショットを取らず、トランザクションを張りません。シーダーが無条件に挿入する場合、それを再実行すると重複が生じます。シーダーを再実行しても安全にする、2つの標準的な方法です:

- **先にリセットする。** ローカル開発の「消してシードし直す」ループは、通常 `suprnova migrate:fresh && cargo run --bin console -- db:seed` を行います - `migrate:fresh` はすべてのテーブルを削除して再構築するため、シーダーは常に空の状態から始まります。これは、ほとんどのプロジェクトが日常的に使う形です。
- **アップサート / 事前確認。** 既存のデータと共存しなければならないシーダー（本番環境のデフォルトの管理者アカウント、国の正規リスト）には、挿入をルックアップで保護するか、アップサートのクエリを使ってください。

```rust
async fn run() -> Result<(), FrameworkError> {
    let exists = User::query()
        .db_where("email", "admin@example.com")
        .exists()
        .await?;

    if !exists {
        let password_hash = suprnova::hashing::hash("change-me-on-first-login")?;
        User::create(attrs!{
            email: "admin@example.com",
            name: "Admin",
            password: password_hash,
        }).await?;
    }
    Ok(())
}
```

## `without_events` でモデルイベントを消音する

ループの中で `Model::create` を呼ぶシーダーは、行ごとに、あらゆるライフサイクルイベント - `Creating`、`Saving`、`Created`、`Saved` - を発火します。それは、登録済みのあらゆる `Observer<M>` を起こし、キューに入れられたあらゆるブロードキャストリスナーを実行し、そして、あなたが実際には望んでいない100件のバックグラウンドジョブを、偶発的にエンキューしてしまうことがあります。`seed::without_events` は、Laravelの `WithoutModelEvents` に相当するものです:

```rust
use suprnova::{async_trait, FrameworkError, Seeder, seed};
use crate::models::users::User;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        seed::without_events(async {
            for i in 0..50 {
                User::create(attrs!{
                    name: format!("user{i}"),
                    email: format!("user{i}@example.com"),
                }).await?;
            }
            Ok(())
        }).await
    }
}
```

内側のfutureがawaitしている間、キャンセル可能な拒否権の経路（`dispatch_cancellable`）と、イベント後のファンアウト（`dispatch_after`）はどちらも `Ok(())` へショートサーキットします。オブザーバーは静かなままであり、ブロードキャスターは起きず、下流のジョブはエンキューされません。

この効果はタスクスコープです - `fut` の内側で実行される作業だけが消音されます。他のタスク上の並行する作業（HTTPリクエストハンドラ、バックグラウンドで動くキューワーカー、他のシーダー）は、通常どおりイベントを発火し続けます。ネストした呼び出しは合成されます: 内側の `without_events` ブロックは、外側のフラグを継承します。

### ファクトリーはすでにモデルイベントを迂回している

知っておく価値があります。これは、`without_events` に手を伸ばすべきタイミングを変えるからです: ファクトリーは `ActiveModelTrait::insert`（SeaORMモデル上の `Persistable` の実装）を介して永続化を行い、これは `Model` トレイトの `create` / `save` メソッドを経由しません。ファクトリー駆動の経路には、消音すべきモデルイベントのディスパッチがそもそも存在しません。`seed::without_events` は、`Model` トレイトを直接駆動するコードのためのものです - 典型的には、ファクトリーが回避しているランタイム形のエルゴノミクスが必要な場合、あるいは、本番環境ではオブザーバーが反応するはずだがフィクスチャの読み込み中はそうすべきでないモデルに、シードの途中で触れている場合です。

実務上: あなたのシーダーが `UserFactory::new().create_many()` 呼び出しの積み重ねであれば、`without_events` は必要ありません。それが `User::create(attrs)` の手作りのループであれば、おそらく必要です。

## テストの中でシーダーを使う

consoleバイナリが駆動するのと同じレジストリは、`#[tokio::test]` からも呼び出せます - 統合テストの前に、既知のフィクスチャ集合が欲しいときに便利です:

```rust
use serial_test::serial;
use suprnova::container::testing::TestContainer;
use suprnova::{DbConnection, seed};

use app::seeders::BaseSeeder;

#[tokio::test]
#[serial]
async fn dashboard_renders_seeded_posts() {
    // 以前のテストの登録が漏れないよう、レジストリをリセットする。
    seed::clear();

    let _guard = TestContainer::fake();
    let conn = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    app::migrations::Migrator::up(&conn, None).await.unwrap();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    // 欲しいシーダーを登録し、実行し、
    // 新しいデータベースに対してアサートする。
    seed::register::<BaseSeeder>();
    seed::run_all().await.unwrap();

    // …シードされたデータに対するコントローラーテスト…

    seed::clear();
}
```

テストの形についての、2つの注意点です:

- テストがプロセスグローバルなレジストリを変更するときは、`#[serial]` が必要です - 同じレジストリを共有する並行テストは競合してしまいます。このアトリビュートを手に入れるには、あなたのプロジェクトの `Cargo.toml` に `serial_test` を開発依存として追加してください。
- `seed::clear()` は、`#[doc(hidden)]` のテスト専用ヘルパーです。本番のコードから呼ばないでください - レジストリは起動時に一度だけ構築され、決してリセットされません。

より広いテストハーネスの規約（`#[suprnova_test]`、`TestContainer`、`TestDatabase::fresh::<Migrator>()`、あらゆる外部表面のためのフェイク）については、[テスト](testing.md)を参照してください。

## シード、マイグレーション、ファクトリーのどれを選ぶか

この3つのパターンは、いずれもテーブルに行を入れます。判断は通常わかりやすいものですが、PHPのチームはしばしばその境界を曖昧にしてしまうため、明示的に線を引く価値があります。

| 欲しいもの… | 使うもの |
|---|---|
| カラムが存在すること | [マイグレーション](migrations.md) |
| アプリの起動のために存在しなければならない行（デフォルトの管理者、シングルトンのサイト設定行、通貨の正規リスト） | **シーダー** - べき等で、本番環境を含むあらゆる環境で実行される |
| ローカル開発やステージングのための、ランダム化された行の集合（50人のユーザー、200件の投稿、1000件のイベント） | ファクトリーを呼ぶシーダー |
| ユニットテストが必要とする行 | テストの内側で直接呼ばれる[ファクトリー](eloquent.md) |
| 行の形 | [ファクトリー](eloquent.md) |

避けるべき間違いです:

- **マイグレーションからデータを挿入しないでください。** マイグレーションはスキーマを記述するものであり、状態ではありません。デフォルトの行を挿入するマイグレーションは、本番データベース上で一度だけ実行され、二度と実行されません - カラムが変わった瞬間、マイグレーション履歴とシーダーの間で、正としての情報源が分岐してしまいます。挿入はシーダーに置いてください。本番環境がその行を必要とするなら、デプロイの一部として `console db:seed --class=DefaultsSeeder` を実行してください。
- **フィクスチャデータをテストの中に手で書かないでください。** ファクトリーに手を伸ばしてください。テストの中の5つの `User::create(attrs!{ … })` ブロックは、NOT NULLカラムを追加した瞬間に5つの書き直しになります。1つの `UserFactory::new().create()` は生き残ります。
- **本番のデータをシーダーに入れないでください。** シーダーは、アプリケーションが機能するために必要とする行のためのものであり、「インポートしようとしている8,000件の履歴レコードがこれです」というためのものではありません。インポートは単発のスクリプトです（それらのために `#[command]` を書いてください。[コンソール](console.md)を参照してください）。

### Suprnovaが異なる設計を選んだ理由

Laravelは、Eloquentのシーダーローダーが認識する、特別扱いの `call($seeders)` ヘルパーを備えた `DatabaseSeeder` クラスを出荷します。Suprnovaはそうしません - レジストリはフラットな `IndexMap` であり、すべてのシーダーは同格であり、合成シーダーは `seed::run_one(name)` を呼ぶ（あるいは単にサブファクトリーを直接呼ぶ）ことで連鎖します。

その理由は、Suprnovaの他の場所でも見られる、同じトレードオフです: 1つの順序付けルールを持つ単一の汎用レジストリは、魔法のようなルートを持つクラス階層よりも、考えやすいのです。Laravelのパターンが機能するのは、PHPのクラスの自動ロードと静的な `make()` リフレクションが、`call([A::class, B::class])` にそれらのクラスを名前で見つけて、インスタンス化させてくれるからです。Rustでは、ユーザーに `dyn Seeder` トレイトオブジェクトを持ち回らせることになり、それは、すでにそこにある関数ポインタのレジストリよりも扱いにくいものです。

合成シーダーの規約は、同じエルゴノミクスを回復します - `BaseSeeder` は、Laravelにおける `DatabaseSeeder` の役割を演じます - フレームワークが1つの名前を特別なものとして祝福する必要はありません。

シーダーの進捗行は、80カラム固定のプレーンテキストです。Laravelはドットリーダーを端末に合わせてサイズ調整し、ステータスの語に色を付けます。実際の端末幅を読むということは、フレームワークが抱えていない依存関係を意味しますし、この出力は、日常的にログへパイプされるstdoutへ行き、そこではエスケープコードはノイズです。経過時間は、桁区切りなしの整数ミリ秒として出力されます。
## ブートストラップでの登録

すべてのシーダーは、他のプロセスグローバルな配線（config、オブザーバー、スーパーバイザー、キュージョブ）と並んで、`bootstrap.rs` の中に `seed::register` の呼び出しを必要とします。このパターンは、bootstrapファイルの他の場所で使われているのと同じ形です:

```rust
// src/bootstrap.rs
pub async fn register() {
    // …config + コンテナのバインディング + 認証の配線…

    // シーダー。順序が重要 - run_allは登録順に訪れる。
    suprnova::seed::register::<crate::seeders::BaseSeeder>();
    suprnova::seed::register::<crate::seeders::DemoContentSeeder>();

    // …オブザーバー、スーパーバイザー、キュージョブ…
}
```

シーダーの登録を忘れると、`console db:seed --class=X` は「no seeder registered for `X`」で失敗します - サイレントなスキップではなく、明確な信号です。`seed::count()` と `seed::is_registered("…")` ヘルパーが存在するのは、まさに、bootstrapが期待したすべてのシーダーを登録したことを、テストがアサートできるようにするためです。

ファイル全体の構造と、フレームワークが各サブシステムに配線されることを期待する順序については、[アプリケーション ブートストラップ](bootstrap.md)を参照してください。

## 次のステップ

- [マイグレーション](migrations.md) - シード/マイグレーションの組の、スキーマ側の半分
- [Eloquent](eloquent.md) - モデル、ファクトリー、そしてすべてのシーダーが呼び込む `Persistable` の仕組み
- [コンソール](console.md) - あなた自身の `#[command]` と並んで `db:seed` をホストする、プロジェクトごとの `console` バイナリ
- [テスト](testing.md) - `TestContainer`、`TestDatabase::fresh`、そしてシーダーレジストリに触れるテストのための `#[serial]` パターン
- [エラー モデル](error-model.md) - `FrameworkError` とは何か、そして `run` の `Result<(), _>` の形が、フレームワークの残りの部分とどのように合成されるか
