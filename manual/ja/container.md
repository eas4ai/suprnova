# サービス コンテナ

コンテナは、Suprnovaがアプリケーションのサービス - DB接続プール、メールドライバー、あなたの `Arc<MyService>` などを保持する場所です。起動時に値をバインドし、ハンドラやワーカーの中でそれを解決します。これはLaravelのサービスコンテナに相当するSuprnovaの仕組みですが、1つ重要な違いがあります。ルックアップはまずタスクローカルで行われるため、並行して実行されるテスト同士が、互いのバインディングを見ることはありません。

## 2つの構成要素

| 型 | 役割 |
|---|---|
| `Container` | 基盤となるレジストリです。バインディング、ファクトリー、シングルトンを保持します |
| `App` | 実際に呼び出すグローバルファサードです - `App::bind`、`App::get` など |

`Container` を直接構築することはほとんどなく、ほぼ常に `App::*` を呼び出します。コンテナは裏方（内部機構）であり、`App` ファサードがAPIです。

## ルックアップの順序

`App::get` / `App::make` の呼び出しはすべて、**3つの層**を順番に確認します。

```
      タスクローカル
            │
            ▼  （ミス）
     スレッドローカル
            │
            ▼  （ミス）
        グローバル
            │
            ▼  （ミス）
          None
```

これが重要な理由は、次の通りです。

- **リクエストごとの状態はタスクローカルを経由します** - Inertiaの共有データ、フラッシュバッグ、リクエストIDなどです。各リクエストは、透過的に自分専用の層を持ちます。
- **テストはスレッドローカルを使います** - `let _g = TestContainer::fake();` に続けて `TestContainer::bind(...)` を呼ぶと、グローバルコンテナに触れることなく、1つのスレッド内でバインドされます。そのため、並列に実行されるテスト同士でサービスが混ざることはありません。このガードは、ドロップされる際にテストコンテナをクリアします。
- **アプリ全体のサービスはグローバルを経由します** - 起動時に一度だけバインドされ、どこからでも解決できます。

バインディングがどの層に存在するかを意識することは、めったにありません - `App::bind` は適切な場所にそれを配置し、`App::get` はそれがどこにあっても見つけ出します。このモデルが重要になるのは、並行処理のもとで何かが予期せぬ振る舞いをしたときだけであり、その詳細は[テスト](testing.md)の章にあります。

## 値をバインドする

手元にあるものに応じて、コンテナに何かを入れる方法は5通りあります。

### `App::singleton(value)` - 所有し、ルックアップ時に複製する

永続的に存在させたい、あらゆる `T: Any + Send + Sync + 'static` の値に使います。`Clone` 境界があるのはバインディング側ではなく*ゲッター*（`App::get`）側です - 値は `Arc` の中に一度だけ格納され、`get` のたびにその `Arc` から複製されます。

```rust
use suprnova::App;

App::singleton(MyConfig {
    timeout_secs: 30,
    retries: 3,
});

let cfg = App::get::<MyConfig>().expect("registered at boot");
println!("{}", cfg.timeout_secs);
```

値は一度だけ格納されます。`App::get::<MyConfig>()` は複製を返します。複製のコストが低い、設定のようなプレーンなデータにはこれを使ってください。

### `App::bind(Arc<T>)` - トレイトや共有サービス向け

トレイトオブジェクトや、`Arc` の背後に置きたいものに使います。

```rust
use std::sync::Arc;
use suprnova::App;

let store: Arc<dyn KeyValueStore> = Arc::new(RedisStore::connect(url)?);
App::bind(store);

let store = App::make::<dyn KeyValueStore>().expect("bound at boot");
store.put("hello", b"world").await?;
```

`App::make::<T>()` は `Arc<T>` の複製を返します（コストの低いアトミックな参照カウントの増加です）。スレッド間で共有されるサービス、特にトレイトオブジェクトには、これを使ってください。

### `App::factory(|| { … })` - オンデマンドで構築する

値の構築を、初回使用時（あるいは毎回）に行いたい場合です。

```rust
App::factory(|| {
    HttpClient::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .expect("http client config is hand-rolled and known-good")
});
```

`App::factory` は*具象型*のファクトリー（`Fn() -> T`）を登録し、`App::bind_factory` は*トレイトオブジェクト*のファクトリー（`Fn() -> Arc<T>`）を登録します。どちらのクロージャも `Result` を返しません - 構築の失敗はクロージャの中で処理する（起動時にパニックさせる、あるいは番兵となる値を作る）か、自分で `?` を使って値を構築した後に、通常の `App::singleton` / `App::bind` を使ってください。どちらも、コンテナのロックの外でクロージャを呼び出すため、コンテナに再入するファクトリーがデッドロックすることはなく、コストの高いコンストラクタが他のバインディングをブロックすることもありません。

### `App::*_if_absent(value)` - 起動順序にやさしい登録方法

デフォルトのサービスが、サービスを提供するクレートによって登録されており、アプリ側ではそれが存在するときにだけ上書きしたい、という場合があります。`_if_absent` のバリエーションを使うと、既存のバインディングを踏み潰さないデフォルトを登録できます。

```rust
// スターターやライブラリクレートの内部では:
App::singleton_if_absent(DefaultMailDriver::new());

// アプリ側の bootstrap.rs では:
App::singleton(MyCustomMailDriver::new());  // 後から実行されたので、こちらが勝ちます
```

`bind_if_absent`、`singleton_if_absent`、およびファクトリー版のバリエーションは、いずれも `bool` を返します - 実際に挿入した場合は `true`、すでにバインディングが存在していた場合は `false` です。

## 値を解決する

2つの読み取りメソッドと、それぞれに対応する `Result` を返す兄弟メソッドがあります。

```rust
// バインドされた値をクローンして取り出す:
let cfg: MyConfig = App::get::<MyConfig>().expect("bound at boot");

// Arc をクローンする:
let store: Arc<dyn KeyValueStore> = App::make().expect("bound at boot");

// 同じものを Result で。失敗しうる経路で `?` を使うイディオム向けです:
let cfg = App::resolve::<MyConfig>()?;
let store = App::resolve_make::<dyn KeyValueStore>()?;
```

`resolve` と `resolve_make` は `Result<_, FrameworkError>`（ルックアップに失敗した場合は特に `ServiceNotFound` バリアント）を返します - サービスが見つからないことが、パニックではなく適切なログ付きの500として表面化すべきハンドラの経路で役立ちます。

存在確認（めったに必要ありません）。

```rust
if App::has::<MyConfig>() { … }
if App::has_binding::<dyn KeyValueStore>() { … }
```

## バインディングが行われる場所

標準的な置き場所は `src/bootstrap.rs` です - 起動時に一度だけ実行される、1つの関数です。

```rust
use std::sync::Arc;
use suprnova::App;
use crate::services::{MyService, RealEmailGateway};

pub async fn register() {
    // 素のシングルトン
    App::singleton(MyAppConfig {
        max_uploads_per_user: 100,
    });

    // トレイトオブジェクトのサービス
    let gateway: Arc<dyn EmailGateway> = Arc::new(RealEmailGateway::new());
    App::bind(gateway);

    // 遅延サービス（初回使用時に構築されます）
    App::bind_factory::<dyn HttpClient, _>(|| {
        Arc::new(ReqwestClient::with_timeout(30))
    });
}
```

関数名 `register` は、スキャフォルドのデフォルト（`src/bootstrap.rs::register`）と一致しており、戻り値の型は `Result` ではなく `()` です。起動時に発生しうるバインドエラー（例えばドライバーの接続失敗など）は、`register` 自身からではなく、ドライバーやサービスのコンストラクタを通じて伝播させるべきです - 起動配線の全体像については[アプリケーション ブートストラップ](bootstrap.md)を参照してください。

フレームワーク自身も、起動中にコンテナへアクセスします。

- `App::init()` が最初に実行され、レジストリを初期化します
- `App::boot_services()` が起動時の依存関係（ドライバー、暗号化キーなど）を解決します - あなたのサービスは、完全に起動し終えたフレームワークを目にすることになります
- その後にあなたの `bootstrap_fn` が実行されるため、フレームワークのサービスが利用可能であることを前提にできます

起動順序の全体像については、[アプリケーション ブートストラップ](bootstrap.md)を参照してください。

## Inertiaの共有データ

コンテナは、Inertiaの共有データが存在する場所でもあります。3つの便利なAPIが、それを明示的にします。

```rust
use suprnova::App;

// 即時評価の値 - 一度だけシリアライズされ、すべての Inertia レスポンスで再利用されます。
App::inertia_share("appName", "Suprnova");

// 遅延評価の値 - 解決用のクロージャがレスポンスごとに実行されます。非同期処理を
// 伴うリクエストごとのデータに使ってください。
App::inertia_share_lazy("locale", || async {
    Ok::<_, suprnova::FrameworkError>(detect_locale().await)
});

// リクエストごとのフラッシュバッグに、エントリを1つ積みます。
App::flash("message", "Saved!");
```

これらは、`&Arc<InertiaRegistry>` を返す `Container::inertia()` から読み取ります - より低レベルなアクセスが必要な場合は、直接操作することもできます。共有データがページレスポンスにどう反映されるかについては、[Inertia / フロントエンド](frontend.md)を参照してください。

## なぜ3つの層があるのか

タスクローカル → スレッドローカル → グローバルという連鎖が存在する理由は1つです。**並行処理下での分離**です。これによって恩恵を受けるものが3つあります。

**リクエストごとの分離。** Inertiaのフラッシュバッグは、タスクローカル層を介してリクエストごとにバインドされます。2つの並行リクエストは、タスクローカルなコンテナが重ならないため、互いのフラッシュを見ることはありません。バインディングは、リクエストのタスクが終了すると消えます。

**テストごとの分離。** フェイクのメールドライバーをバインドするテストは、兄弟テストがバインドしたフェイクを見るべきではありません。`TestContainer::fake()` はスレッドローカルなガードを返し、`TestContainer::bind` / `TestContainer::singleton` は書き込みをアクティブなスコープへと振り分けます。並列テストは、互いに独立した状態を保ちます。

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn one_test_binds_a_fake() {
    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn Mailer>(Arc::new(FakeMailer::new()));

    // … このテストは FakeMailer を使います
    // 並行して走る兄弟テストからは見えません
}
```

マルチスレッドのtokioランタイムでは - フューチャーがワーカースレッド間を移動する可能性があるため - 代わりに `TestContainer::scope(async { ... })` を使ってください。これは、移動を生き延びるタスクローカルなオーバーライドをインストールします。

**起動時の上書き。** アプリケーションのコードは、ライブラリクレートが登録したデフォルトを上書きできます。`_if_absent` のバリエーションと、階層化されたルックアップが組み合わさることで、ライブラリクレートはアプリケーション側の上書きと衝突することなく、クリーンなデフォルト登録を行えます。

## よくあるパターン

### DBプールを保持する構造体をバインドする

これを直接行うことは、ほとんどありません - DBプール自体はフレームワークがバインドします。ですが、コストの高い共有リソースを持つ、独自のサブシステムがある場合は次のようにします。

```rust
let pool = MyResourcePool::connect(url).await?;
App::bind(Arc::new(pool));

// 後で取り出す:
let pool = App::resolve_make::<MyResourcePool>()?;
let conn = pool.checkout().await?;
```

`App::make` は `Option<Arc<T>>` を返し、`.expect(...)` と組み合わせます。`App::resolve_make` は `Result<Arc<T>, FrameworkError::ServiceNotFound>` を返し、失敗しうるコードの中で `?` と組み合わせます。呼び出し元のエラーの扱い方に合う方を使ってください。

### テストでデフォルトをフェイクに差し替える

```rust
use std::sync::Arc;
use suprnova::container::testing::TestContainer;
use suprnova::suprnova_test;

#[suprnova_test]
async fn order_dispatches_email() {
    let fake = Arc::new(FakeEmailGateway::new());
    let fake_for_assert = Arc::clone(&fake);

    let _guard = TestContainer::fake();
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.expect("place_order succeeds");

    assert_eq!(fake_for_assert.sent_count(), 1);
}
```

### 遅延させたコストの高い構築

```rust
// 埋め込みモデルを、起動時ではなく最初のリクエストで構築します。
App::bind_factory::<dyn EmbeddingModel, _>(|| {
    Arc::new(
        OnnxEmbedding::load_from_disk("/models/all-mini-lm.onnx")
            .expect("embedding model must load"),
    )
});
```

オペレーターに構造化されたエラーを表面化させる必要がある、失敗しうる構築の場合は、`bootstrap()` の中で `?` を使って自分で値を構築し、準備ができた時点で `App::bind(...)` を呼び出してください。

## Suprnovaが異なる設計を選んだ理由

Laravelのコンテナには、グローバルなスコープが1つしかありません - バインディングはグローバルであり、テスト間の分離には `setUp` / `tearDown` の規律と、フレームワークによるテストごとのデータベーストランザクションが必要です。PHPのリクエストごとにプロセスを立てるモデルは、これを偶然にも安全なものにしています。リクエストごとに新しいプロセスが立つということは、コンテナが毎回リセットされるということだからです。

Rustのプロセスモデルはその逆です - 1つのプロセスが、多数のスレッド上で多数の並行リクエストを処理します。グローバルのみのコンテナでは、あるスレッドのテストが別のスレッドがバインドしたフェイクを見てしまったり、あるリクエストが別のリクエストのリクエストごとのデータを見てしまったりする可能性があります。だからこそSuprnovaには、3層の連鎖があります。リクエストごとにタスクローカル、テストごとにスレッドローカル、アプリ全体にグローバルです。

コンテナのAPIはLaravelと同じですが、ルックアップの仕組みは、ランタイムが異なるために異なっています。

## 次のステップ

- [アプリケーション ブートストラップ](bootstrap.md) - バインディングのコードを置く場所
- [設定](configuration.md) - サービスと並ぶ、型付き設定の登録
- [テスト](testing.md) - `TestContainer::fake` と `#[suprnova_test]`
- [ロック ポリシー](lock-policy.md) - コンテナを基盤とするアプリケーションにおいて、ポイズンされたロックからの復旧がなぜ重要なのか
