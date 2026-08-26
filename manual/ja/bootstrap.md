# アプリケーション ブートストラップ

`bootstrap.rs` は、アプリケーションが起動時に自身を配線する唯一の場所です。コンテナのバインディング、イベントリスナー、オブザーバー、スーパーバイザー - 最初のリクエストがサーバーに届く（あるいは最初のジョブがキューから取り出される）前に存在しているべきものはすべて、ここに登録されます。組み立てが必要なサービスプロバイダーのスキャフォルドはありません。

フックは一つではなく二つです。`register` はプロセス全体向けです。サーバーだけでなく、`queue:work`、`schedule:work`、`workflow:work`、コンソールバイナリを含むすべてのサブコマンドが実行します。データベース接続、コンテナバインディング、イベントリスナー、オブザーバー、スーパーバイザー、ワーカージョブの登録はここに置きます。`.http_bootstrap` で配線する `register_http_stack` は、サーバーの経路（`serve` / `web:run`）でのみ実行されます。グローバルミドルウェアと `Inertia::install` はここに置きます。下の「起動順序におけるbootstrapの位置」セクションが、なぜこの分割が存在するかを説明します。

## 全体像

スキャフォルドされたアプリのエントリーポイントは、フルーエントな記法で [`Application`](lifecycle.md) を構築し、それを実行します。bootstrapは、ビルダー上の二つのメソッドです。

```rust
// cmd/main.rs
use app::{bootstrap, config, migrations, routes};
use suprnova::Application;

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .http_bootstrap(|| async { bootstrap::register_http_stack() })
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

フレームワークは、起動シーケンスの中であなたの `bootstrap_fn` を一度呼び出します - 環境が読み込まれ、ランタイムドライバー（Cache、Queue、RateLimit、Mail）が起動した後、しかしルーターが構築される前です。同じ呼び出しはバックグラウンドワーカー（`queue:work`、`workflow:work`、`schedule:work`）でも実行されるため、ここで登録したオブザーバーやリスナーは、キュージョブによる挿入とHTTPハンドラによる挿入とで、全く同じように発火します。`http_bootstrap_fn` は `bootstrap_fn` の直後に実行されますが、サーバー経路でのみ実行されます。バックグラウンドワーカーとコンソールバイナリは決して呼び出しません。[リクエスト ライフサイクル](lifecycle.md) で全シーケンスを解説しています。

二つの関数のシグネチャは `Application::bootstrap` と `Application::http_bootstrap` によって固定されています。

```rust
// src/bootstrap.rs
pub async fn register() {
    // データベース、バインディング、オブザーバー、リスナー、
    // スーパーバイザー、ワーカージョブの登録
}

pub fn register_http_stack() {
    // グローバルミドルウェア、Inertia::install
}
```

`register` は `()` を返します。`register_http_stack` は `async` ではなく同期的です。プレーンな関数ポインタを、テストへ`async`を引き込まずにテストハーネスのエントリーポイントとしても使えるようにするため、両方を呼び出し箇所で非同期クロージャとして配線します（`.http_bootstrap(|| async { bootstrap::register_http_stack() })`）。失敗しうるセットアップには、対処方法を説明するメッセージを添えて `.expect("…")` を使います - 起動時こそ、はっきりと失敗するのにふさわしいタイミングです。サンプルアプリでの呼び出しは `DB::init().await.expect("Failed to connect to database");` であり、`DATABASE_URL` が未設定であれば、最初のリクエストで紛らわしい「接続が拒否されました」として表面化するのではなく、実際のエラーが表示された状態で起動時にプロセスを中断します。

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

### Magnetar認証エンジン

組み込みのパスワード、パスキー、マジックリンク、bearer、ロックアウト、remember、OAuthの各ファサードを使うアプリケーションは、データベースと `APP_KEY` の準備後にMagnetarを初期化します:

```rust
use suprnova::{DB, MagnetarConfig, PasskeyConfig, init_magnetar};

pub async fn register() {
    DB::init().await.expect("Failed to connect to database");

    let database = DB::connection().expect("DB not initialized");
    let config = MagnetarConfig::from_sea_orm(database.inner().clone())
        .passkey_config(PasskeyConfig {
            rp_id: "app.example.com".to_string(),
            rp_origin: "https://app.example.com".to_string(),
        });

    init_magnetar(config)
        .await
        .expect("Failed to initialize Magnetar");
}
```

Magnetarは、キューワーカー、スケジューラー、HTTPハンドラ、セッションミドルウェアが同じ資格情報とセッションストアを使うため、プロセス全体向けです。`init_magnetar` は `register_http_stack` ではなく `register` に置いてください。インストーラーは一度きりであり、別のエンジンがすでにインストール済みなら失敗します。

APIスキャフォルドは、アプリケーションbootstrapで `PASSKEY_RP_ID` と `PASSKEY_RP_ORIGIN` を読み取ります。これらの名前は、フレームワーク所有の環境変数ではなくスキャフォルドの規約です。デフォルトの `MagnetarConfig` は、アプリケーションのアイデンティティを正規の `app_users` テーブルへ束縛します。生成されるフルスタックスキャフォルドは `users` モデルを使用し、Magnetarを初期化しないため、デフォルトのイニシャライザーを変更せずにそのスキャフォルドへ追加しないでください。APIスキャフォルドの `app_users` モデルを使うか、既存の `users` テーブル用にカスタムの `MagnetarHostEngine` と `AuthSchema` の束縛を構築してください。フレームワークの `UserProvider` とMagnetarホスト束縛は、同じアプリケーションアイデンティティに合わせてください。デフォルトの `MagnetarConfig` 初期化の現在の実用的なリファレンスは、`app/src/bootstrap.rs` ではなくAPIスキャフォルドです。

### グローバルミドルウェア

グローバルミドルウェアはHTTP専用なので、`register` ではなく `register_http_stack` に置きます:

```rust
use suprnova::{global_middleware, SessionMiddleware, SessionConfig, TimeoutMiddleware};
use crate::middleware;

pub fn register_http_stack() {
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
    // トレイト → シングルトン（Arcで包む）:
    bind!(dyn UserProvider, DatabaseUserProvider);

    // 具体型のシングルトン:
    singleton!(MyConfig { max_uploads_per_user: 100 });

    // ファクトリー（解決のたびに構築される）:
    factory!(|| RequestLogger::new());

    // あるいは、より細かく制御するためにファサードを直接呼び出す:
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
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
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

これは例アプリからの逐語的な抜粋ではない、代表的な構成です。プロセス全体の登録は `register` に、HTTP専用のセットアップは `register_http_stack` に置きます。Magnetarの初期化は、アプリケーションユーザースキーマがフレームワークのユーザープロバイダーと一致しなければならないため、上で別に示しています。

```rust
//! アプリケーションのbootstrap - サービス、リスナー、グローバルミドルウェア、
//! そしてInertia層を登録する。

use std::sync::Arc;
use std::time::Duration;

use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::features::{FeatureMiddleware, bootstrap_database_cached};
use suprnova::queue::worker::register_job;
use suprnova::{
    App, DB, EloquentUserProvider, EventFacade, FrameworkError, Inertia,
    InertiaConfig, SessionConfig, SessionMiddleware, Storage, SupervisorRegistry,
    UserProvider, bind, global_middleware,
};

use crate::broadcasting::ChatChannel;
use crate::events::UserRegistered;
use crate::listeners::SendWelcomeEmailListener;
use crate::middleware;
use crate::models::users::User;

pub async fn register() {
    // ── データベース
    DB::init().await.expect("Failed to connect to database");

    // ── 認証プロバイダー
    bind!(dyn UserProvider, EloquentUserProvider::<User>::new());


    // ── ブロードキャストhub + チャネルレジストリ
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

    // ── ストレージディスク（本番では環境変数でゲートしたS3）
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
}

pub fn register_http_stack() {
    // ── グローバルミドルウェア（登録順に外側から内側へ）
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(suprnova::TimeoutMiddleware::default());
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));

    // ── Inertiaプロトコル層（バージョンのピン留めなし: デフォルトはViteの
    // ビルドマニフェストをハッシュするため、フロントエンドのビルドが
    // アセットバージョンを自分で上げる - frontend-inertia-responses.md の
    // 「バージョン検出」を参照）
    Inertia::install(&InertiaConfig::new()).expect("Inertia install failed");

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
6. **あなたの `http_bootstrap_fn` が、サーバー経路でのみ実行されます** ← `bootstrap::register_http_stack`
7. あなたの `routes_fn` からルートが組み立てられます
8. `Server::from_config` がドライバーとコンテナを起動します
9. あなたの `booted_fn` 群が発火します
10. サーバーがコネクションの受け付けを開始します

バックグラウンドワーカー（`queue:work`、`workflow:work`、`schedule:work`）とコンソールバイナリは、ステップ1〜5と8を共有します。これらは `bootstrap_fn` を実行しますが、`http_bootstrap_fn` を実行するのは `serve` / `web:run` だけなので、ステップ6は決して実行しません。これにより、`register` に登録したリスナーやオブザーバーはHTTPハンドラと同じようにワーカーのコードパスへ届く一方、`register_http_stack` のグローバルミドルウェアと `Inertia::install` はHTTPを提供しないプロセスでは実行されません。

### Suprnovaが異なる設計を選んだ理由

Laravelは、HTTPリクエストだけでなく、`artisan` コマンドやキューワーカーでも、すべてのサービスプロバイダーの `register()` と `boot()` を実行します。それでも問題にならないのは、Vite統合が、`@vite` Bladeディレクティブに要求された内容から、描画時にアセットURLを遅延解決するためです。ビューを一度も描画しないワーカーはマニフェストに触れないので、ビルドが欠けていても問題になりません。

Suprnovaの `Inertia::install` は、起動時にマニフェストを一度解決し、本番環境でマニフェストが欠けていればフェイルクローズします。これは、設定を誤ったデプロイが、誰も実行していないVite開発サーバーを指すアセットURLを配信しないための意図的な設計です。しかし、`public/assets` を正しく同梱していないワーカーやコンソールのイメージでは、この設計がそのままでは問題になります。Laravelがリクエスト時まで遅延する失敗に、Suprnovaはすべてのサブコマンドのプロセス起動時に遭遇してしまうためです。起動表面を `bootstrap` と `http_bootstrap` に分割することで、フェイルクローズ検査を維持しつつ、それを実際にInertiaページを描画するサーバー経路だけに限定します。

Laravelは起動処理自体も複数のサービスプロバイダーに分割します。各プロバイダーが `register()` と `boot()` を実装し、それらは `config/app.php` に集約され、Laravelは2段階（まずすべての `register`、次にすべての `boot`）で巡回します。これにより、ユーザーコードで順序を管理することなく、あるサービスが別のプロバイダーのバインディングに依存できます。プロバイダークラスは、アプリが数十もの異なるサブシステムを抱えるようになったときの整理単位になります。

Suprnovaは、それをプロバイダーごとの `register` / `boot` の組ではなく、`register` と `register_http_stack` の2つの関数へ集約します。理由は次のとおりです。

- **2段階の `register` / `boot` 分割は、Rustには存在しない順序の問題を解決するためのものです。** `#[injectable]` とコンテナの `bootstrap_singletons` は、ユーザーから見える順序付けなしに依存グラフをすでに解決します。バインディングはインラインで登録され、残りはルックアップの仕組みが処理します。
- **2つの関数は、10個の関数よりも読みやすいものです。** 新しいコントリビューターが `bootstrap.rs` を開けば、すべてのバインディング、リスナー、オブザーバー、ミドルウェア層を、2箇所のいずれかで確認できます。プロバイダー方式の分断は、アプリが実際に何をしているのかを見えにくくします。
- **インベントリ方式の自動登録が、残りをカバーします。** オブザーバー、スーパーバイザー、スケジュールされたタスク、ポリシー、キューハンドラはすべて、コンパイル時に `inventory::submit!` を通じて自分自身を集約します。bootstrapは、それぞれを列挙するのではなく、単一の呼び出し（`bootstrap_observers`、`SupervisorRegistry::start_all`）でインベントリを流し出します。

Laravelのプロバイダー分割が価値を発揮するのは、ライブラリの配布です。自身のバインディングを同梱するクレートは、アプリが自身のbootstrapを編集することなくオプトインできる登録用エントリポイントを求めるでしょう。Suprnovaにおけるそれに相当するものは、クレートのルートにある公開の `pub async fn register()` と、アプリのbootstrapからの1行の呼び出しです。その手間は1行だけであり、読みやすさの利得は、すべてが一箇所にまとまっていることです。

## 次のステップ

- [リクエスト ライフサイクル](lifecycle.md) - 起動順序の全体と、`bootstrap_fn` が発火する場所
- [サービス コンテナ](container.md) - `App::bind` / `App::singleton` /
  `App::factory` と3層のルックアップ
- [設定](configuration.md) - bootstrapの前に実行される、型付き設定の登録
- [ミドルウェア](middleware.md) - `global_middleware!` で登録された層のチェーン構成
- [イベント](events.md) - リスナーとオブザーバーが組み込まれるディスパッチャー
