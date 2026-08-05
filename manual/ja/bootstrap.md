# アプリケーション ブートストラップ

`bootstrap.rs` は、アプリケーションが起動時に自身を配線する唯一の場所です。コンテナのバインディング、イベントリスナー、オブザーバー、スーパーバイザー、グローバルミドルウェア - 最初のリクエストがサーバーに届く（あるいは最初のジョブがキューから取り出される）前に存在しているべきものはすべて、単一の非同期 `bootstrap` 関数の中に登録されます。組み立てが必要なサービスプロバイダーのスキャフォルドはありません。一度だけ実行される一つの関数、それがAPIのすべてです。

## 全体像

スキャフォルドされたアプリのエントリーポイントは、フルーエントな記法で [`Application`](lifecycle.md) を構築し、それを実行します。`bootstrap` ステップは、そのビルダー上の1つのメソッドです。

```rust
// cmd/main.rs
use app::{bootstrap, config, migrations, routes};
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

### `#[tokio::main]` ではなく `#[suprnova::main]`

このアトリビュートは見た目だけのものではありません。元に戻すと、その理由を説明するメッセージとともに起動が失敗します。

`.env` の読み込みはプロセス環境への書き込みを伴い、`set_var` はプロセスがシングルスレッドである間しか安全ではありません。`#[tokio::main]` は `main` 全体を*取り囲む*形でランタイムを構築するため、最初の文が実行される時点で、すべてのワーカースレッドがすでに存在しています - そして、そのいずれもが、DNS解決や時刻のフォーマット、あるいはC言語で書かれた依存ライブラリを介して、間接的に `getenv` を呼び出す可能性があります。このレースコンディションは、うまくいかなかったときに沈黙したまま発生します。これは、レースコンディションが持ちうる中で最悪の性質です。

`#[suprnova::main]` は、どのみち書くことになる同じ `async fn main` をそのまま維持しつつ、2つのことの順序を入れ替えるだけです。まず環境を読み込み、次にランタイムを構築し、そのランタイム上であなたの本体を実行します。`#[tokio::main]` と同じ `flavor` および `worker_threads` 引数を受け付けます。

`Application::run` は、環境がシングルスレッドのコンテキストから読み込まれたことがないと判断した場合、警告するのではなく起動そのものを拒否します - `#[tokio::main]` の下で「問題なく」起動するアプリこそが、何週間も後になって無関係な環境変数の読み込みを破損させる、まさにそのアプリなのです。

フレームワークは、起動シーケンスの中であなたの `bootstrap_fn` を一度呼び出します - 環境が読み込まれ、ランタイムドライバー（Cache、Queue、RateLimit、Mail）が起動した後、しかしルーターが構築される前です。同じ呼び出しはバックグラウンドワーカー（`queue:work`、`workflow:work`、`schedule:work`）でも実行されるため、ここで登録したオブザーバーやリスナーは、キュージョブによる挿入とHTTPハンドラによる挿入とで、全く同じように発火します。[リクエスト ライフサイクル](lifecycle.md) で全シーケンスを解説しています。

この関数のシグネチャは `Application::bootstrap` によって固定されています。

```rust
// src/bootstrap.rs
pub async fn register() {
    // バインディング、オブザーバー、リスナー、スーパーバイザー、グローバルミドルウェア
}
```

戻り値は `()` です。失敗しうるセットアップには、対処方法を説明するメッセージを添えて `.expect("…")` を使います - 起動時こそ、はっきりと失敗するのにふさわしいタイミングです。サンプルアプリでの呼び出しは `DB::init().await.expect("Failed to connect to database");` であり、`DATABASE_URL` が未設定であれば、最初のリクエストで紛らわしい「接続が拒否されました」として表面化するのではなく、実際のエラーが表示された状態で起動時にプロセスを中断します。

## bootstrapに書くべきこと

実際の `bootstrap` 関数が行うことは、少数の明確に異なる種類の処理です。以下の各サブセクションは、そのうちの1つに対応しています。サンプルアプリの `app/src/bootstrap.rs` はそのすべてを実践しており、動作する参考実装になっています。

### データベース接続

```rust
use suprnova::DB;

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");
}
```

`DB::init` は（あなたの `config_fn` によって登録された）`DatabaseConfig` を読み取り、プールを開きます。この接続はシングルトンとして[コンテナ](container.md)に保存されます - `DB::connection()` / `DB::get()` は、どこからでもそれを解決できます。`DB::init_with(config)` は、環境変数から導かれるURL以外の場所を指定したいときのための、テストやツール向けのエスケープハッチです。

### グローバルミドルウェア

```rust
use suprnova::{global_middleware, SessionMiddleware, SessionConfig, TimeoutMiddleware};
use crate::middleware;

pub async fn register() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));
}
```

`global_middleware!` は、すべてのリクエスト（ルーティングされなかった404やOPTIONSプリフライトを含む）で実行される層を登録します。登録した順序が、チェーンが実行される順序になります - 外側から内側へ、です。フレームワークは自身の `RequestIdMiddleware` を最も外側に配置し、あなたが追加するものはすべてその内側に位置します。[ミドルウェア](middleware.md) では、ルートごとの層を含め、チェーン全体の形を説明しています。

### コンテナのバインディング

コンテナは、あなたが入れたものをそのまま受け取ります。これらのマクロは、[`App`](container.md) ファサードの上に被せられたシンタックスシュガーです。

```rust
use std::sync::Arc;
use suprnova::{App, bind, singleton, factory};
use crate::providers::DatabaseUserProvider;

pub async fn register() {
    // トレイト → シングルトン（Arc でラップされます）:
    bind!(dyn UserProvider, DatabaseUserProvider);

    // 具象型のシングルトン:
    singleton!(MyConfig { max_uploads_per_user: 100 });

    // ファクトリー（解決のたびに構築されます）:
    factory!(|| RequestLogger::new());

    // より細かく制御したい場合は、ファサードを直接呼び出します:
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(hub);
}
```

トレイトオブジェクトへのバインディングが最も一般的な形です - インターフェースをバインドし、ハンドラやテストがその実装を差し替えられるようにします。[サービス コンテナ](container.md) の章では、`bind_factory!`、`_if_absent` 系のバリエーション、そして3層のルックアップモデルを含む、バインディングAPIの全体を扱っています。

### イベントリスナーとオブザーバー

ディスパッチャーは、bootstrapが実行された時点ですでに稼働しています - ここで登録したリスナーは、それ以降のすべてのディスパッチを目にすることになります。

```rust
use std::sync::Arc;
use suprnova::EventFacade;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;

pub async fn register() {
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
}
```

Eloquentのオブザーバー（`#[suprnova::observer(M)]`）は、コンパイル時に `inventory::submit!` を通じて自分自身を集約します。1回の呼び出しで、そのインベントリをディスパッチャーへと流し出します。

```rust
suprnova::eloquent::observers::bootstrap_observers()
    .await
    .expect("observer install failed");
```

この呼び出しはべき等です - bootstrapを再実行しても（2回目に起動するワーカーであっても）、リスナーアダプターが二重に登録されることはありません。[イベント](events.md) ではディスパッチとリスナーの書き方を、[Eloquent API](eloquent.md) ではオブザーバーを扱っています。

### スーパーバイザー

`Supervisor` トレイトと `inventory::submit!` を通じて宣言された長時間実行のバックグラウンドタスクは、1回の呼び出しで開始します。

```rust
use suprnova::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

各スーパーバイザーは、パニック境界を備えた独自の再起動ループタスクの中で実行されます。パニックしたスーパーバイザーはログに記録されて再起動され、プロセス全体を道連れにすることは許されません。トレイトと再起動ポリシーについては、[スーパーバイザー](supervisors.md) を参照してください。

### ワーカージョブの登録

ワーカーが名前でディスパッチする必要があるキュージョブとメーラブルは、起動時に自分自身を登録します。

```rust
use suprnova::queue::worker::register_job;

pub async fn register() {
    register_job::<crate::jobs::welcome_log::WelcomeLog>();

    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();
}
```

これがなければ、ワーカーはキューに積まれたエンベロープを、それを処理する型へと対応付ける手段を持てません。

## 起動後のフック: `booted()`

bootstrapが行うのは*登録*であり、`booted()` が行うのは*解決*です。ビルダーは2つ目のコールバックを受け取り、これはサーバーが自身のサービス起動を終えた後、コネクションの受け付けを開始する前に発火します。フレームワーク自身が起動時にバインドした何かを読み取る必要があるときに使ってください。

```rust
Application::new()
    .config(config::register_all)
    .bootstrap(bootstrap::register)
    .routes(routes::register)
    .booted(|| {
        let cfg: MyConfig = suprnova::App::get().unwrap();
        tracing::info!(?cfg, "services booted");
    })
    .run()
    .await;
```

`booted` は同期的であり、`Server::from_config` の後に実行されます - ドライバーは起動済みで、暗号化キーは読み込まれ、あなたのバインディングも存在しています。ほとんどのアプリはこのフックを必要としません。完全に構築されたコンテナを必要とする、起動後のワンショットの副作用がある場合に使ってください。

## 完全な `bootstrap.rs`

サンプルアプリから抜き出した、簡略化されてはいるものの代表的な形です。

```rust
//! アプリケーションのブートストラップ - サービス、リスナー、
//! グローバルミドルウェアを登録します。

use std::sync::Arc;
use std::time::Duration;

use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::features::{FeatureMiddleware, bootstrap_database_cached};
use suprnova::queue::worker::register_job;
use suprnova::{
    App, DB, EventFacade, FrameworkError, Inertia, InertiaConfig,
    SessionConfig, SessionMiddleware, Storage, SupervisorRegistry,
    UserProvider, bind, global_middleware,
};

use crate::broadcasting::ChatChannel;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;
use crate::middleware;
use crate::providers::DatabaseUserProvider;

pub async fn register() {
    // ── データベース
    DB::init().await.expect("Failed to connect to database");

    // ── グローバルミドルウェア（登録順に外側から内側へ）
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(suprnova::TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));

    // ── 認証プロバイダー
    bind!(dyn UserProvider, DatabaseUserProvider);

    // ── Inertia プロトコル層
    Inertia::install(&InertiaConfig::new().version("1.0")).expect("Inertia install failed");

    // ── ブロードキャストハブ + チャネルレジストリ
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    let mut registry = ChannelRegistry::new();
    registry.register(ChatChannel);
    App::singleton(Arc::new(registry));

    // ── イベントリスナー + ブリッジ
    EventFacade::listen::<UserRegistered, _>(
        Arc::new(SendWelcomeEmailListener),
    ).await;
    EventFacade::broadcast::<UserRegistered>(Arc::clone(&hub)).await;

    // ── ストレージディスク（本番では環境変数で S3 を切り替え）
    Storage::register_fs("public", "./storage/public")
        .expect("register public disk");

    // ── ワーカージョブの登録
    register_job::<crate::jobs::welcome_log::WelcomeLog>();
    suprnova::mail::register_mailable_factory::<crate::mail::welcome::WelcomeEmail>()
        .expect("register at boot");
    register_job::<suprnova::mail::send_job::SendMailJob>();

    // ── オブザーバー + スーパーバイザー
    suprnova::eloquent::observers::bootstrap_observers()
        .await
        .expect("observer install failed");
    SupervisorRegistry::start_all().await;

    // ── フィーチャーフラグ
    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature-flag chain wired");
    global_middleware!(FeatureMiddleware::new());
}
```

そのリズムに注目してください - 各ブロックは1つのことだけを行い、1つか2つのAPIを呼び出し、成功するか、明確なメッセージとともに失敗します。ここには技巧的なものは何もありません - この関数が長いのは、アプリに可動部分が多いからであり、bootstrapというパターンが複雑だからではありません。

## bootstrapと `#[injectable]` の使い分け

`#[injectable]` は、コンパイル時にコンテナの `inventory` へシングルトンを自動登録するマクロです。構築に `#[inject]` の依存関係だけしか必要としないサービスにとって、これが正しい選択です。

```rust
use suprnova::injectable;

#[injectable]
pub struct UserService;

#[injectable]
pub struct OrderService {
    #[inject]
    user_service: UserService,
}
```

これらは自分自身を解決するため、bootstrapが触れる必要はありません。

構築に何かそれ以外のもの - 環境変数、構築済みのconfig構造体、`dyn Trait` のバインディング、ランタイム時の判断、非同期のセットアップ呼び出し、あるいはそれ自体はサービスではない何か（リスナー、オブザーバー、キュージョブのマッピング、グローバルミドルウェア層） - が必要な場合、bootstrapが正しい置き場所です。

| `#[injectable]` を使う場合 | `bootstrap` を使う場合 |
|---|---|
| ランタイム設定を必要としない具象型のシングルトン | `dyn Trait` に関するあらゆるもの |
| 他のinjectableから構築されるサービス | 起動時の非同期処理すべて |
| デフォルトのDIグラフ | 環境変数に依存する値 |
| | イベントリスナー、オブザーバー、スーパーバイザー |
| | グローバルミドルウェア |
| | ワーカージョブとメーラブルの登録 |

自由に混在させることができます。`#[injectable]` のサービスは、`bootstrap` が実行される時点までにコンテナ内で可視になっているため、bootstrap内のバインディングからそれらを読み取ることができます。

## 起動順序におけるbootstrapの位置

全体のシーケンスです（[リクエスト ライフサイクル](lifecycle.md) からの抜粋）。

1. `Config::init(".")` - `.env` を読み込み、環境を検出します
2. `init_policies()` - `#[policy]` インベントリを流し出します
3. あなたの `config_fn` が実行されます（型付き設定の登録）
4. マイグレーションが実行されます（`serve` では自動マイグレーション）
5. **あなたの `bootstrap_fn` が実行されます** ← `bootstrap::register`
6. あなたの `routes_fn` からルートが組み立てられます
7. `Server::from_config` がドライバーとコンテナを起動します
8. あなたの `booted_fn` 群が発火します
9. サーバーがコネクションの受け付けを開始します

バックグラウンドワーカー（`queue:work`、`workflow:work`、`schedule:work`）はステップ1〜5と7を共有するため、あなたが登録したリスナーやオブザーバーは、HTTPハンドラに届くのと全く同じようにワーカーのコードパスにも届きます。

### Suprnovaが異なる設計を選んだ理由

Laravelは起動処理を複数のサービスプロバイダーに分割します - 各プロバイダーが `register()` と `boot()` を実装し、それらは `config/app.php` に集約され、Laravelは2つのパス（まずすべての `register`、次にすべての `boot`）でそれらを巡回します。これにより、ユーザーコード側で順序を気にすることなく、あるサービスが別のプロバイダーのバインディングに依存できます。プロバイダークラスは、アプリが数十もの異なるサブシステムを抱えるようになったときの整理単位を与えてくれます。

Suprnovaは、それを1つの関数へと集約します。その理由は次のとおりです。

- **2パスの `register`/`boot` 分割は、Rustには存在しない順序の問題を解決するためのものです。** `#[injectable]` とコンテナの `bootstrap_singletons` は、ユーザーから見える順序付けなしに、依存グラフをすでに解決しています。バインディングはインラインで登録され、残りはルックアップの仕組みが処理します。
- **1つの関数は、10個の関数よりも読みやすいものです。** 新しいコントリビューターが `bootstrap.rs` を開けば、すべてのバインディング、すべてのリスナー、すべてのオブザーバー、すべてのミドルウェア層を一箇所で見渡せます。プロバイダー方式の分断は、アプリが実際に何をしているのかを見えにくくします。
- **インベントリ方式の自動登録が、残りをカバーします。** オブザーバー、スーパーバイザー、スケジュールされたタスク、ポリシー、キューハンドラはすべて、コンパイル時に `inventory::submit!` を通じて自分自身を集約します。bootstrapは、それぞれを列挙するのではなく、単一の呼び出し（`bootstrap_observers`、`SupervisorRegistry::start_all`）でインベントリを流し出します。

Laravelのプロバイダー分割が価値を発揮するのは、ライブラリの配布という場面です - 自身のバインディングを同梱するクレートは、アプリが自身のbootstrapを編集することなくオプトインできる登録用のエントリーポイントを求めるでしょう。Suprnovaにおけるそれに相当するものは、クレートのルートにある公開の `pub async fn register()` と、アプリの `bootstrap` からの1行の呼び出しです。そのための手間は1行だけであり、読みやすさの利得はすべてが一箇所にまとまっていることそのものです。

## 次のステップ

- [リクエスト ライフサイクル](lifecycle.md) - 起動順序の全体と、`bootstrap_fn` が発火する場所
- [サービス コンテナ](container.md) - `App::bind` / `App::singleton` /
  `App::factory` と3層のルックアップ
- [設定](configuration.md) - bootstrapの前に実行される、型付き設定の登録
- [ミドルウェア](middleware.md) - `global_middleware!` で登録された層のチェーン構成
- [イベント](events.md) - リスナーとオブザーバーが組み込まれるディスパッチャー
