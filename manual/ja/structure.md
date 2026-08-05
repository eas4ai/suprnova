# ディレクトリ構成

`suprnova new my-app --frontend svelte` を実行すると、スキャフォルダーは次の構成を生成します：

```
my-app/
├── Cargo.toml                      # crate マニフェスト + 依存関係、2 つの [[bin]] ターゲット
├── .env                            # ローカル設定 - DB URL、アプリキー、ポート
├── .env.example                    # ops/CI 用のテンプレート
├── .gitignore                      # target/、.env、node_modules/、public/assets/ を除外
├── cmd/
│   └── main.rs                     # バイナリエントリーポイント；Application::new().run() を呼び出す
├── src/
│   ├── lib.rs                      # モジュールの配線（`pub mod controllers;` など）
│   ├── bootstrap.rs                # サービス、オブザーバー、リスナーを登録 -
│   │                               # Laravel のサービスプロバイダーに相当する Suprnova
│   ├── routes.rs                   # `routes!` マクロツリー - アプリが提供するすべての URL
│   ├── bin/
│   │   └── console.rs              # `cargo run --bin console <subcommand>` エントリー -
│   │                               # `php artisan` に相当する Suprnova
│   ├── actions/
│   │   ├── mod.rs
│   │   └── example_action.rs       # シングルメソッドの呼び出し可能なコントローラー
│   ├── commands/
│   │   └── mod.rs                  # `#[command]` アノテーション付きハンドラをここに登録
│   ├── config/
│   │   ├── mod.rs
│   │   ├── database.rs             # 型付き DB 設定（ドライバー、URL、プール）
│   │   └── mail.rs                 # 型付きメール設定
│   ├── controllers/
│   │   ├── mod.rs
│   │   ├── home.rs                 # GET / ハンドラ
│   │   ├── auth.rs                 # ログイン / 登録 / ログアウト
│   │   └── dashboard.rs            # 認証が必要；保護されたルートの例
│   ├── middleware/
│   │   ├── mod.rs
│   │   ├── logging.rs              # リクエスト/レスポンスログ
│   │   └── authenticate.rs         # セッションベースの認証ガード
│   ├── migrations/
│   │   ├── mod.rs
│   │   ├── m_*_create_users_table.rs
│   │   ├── m_*_create_sessions_table.rs
│   │   ├── m_*_create_remember_tokens_table.rs
│   │   ├── m_*_create_workflows_table.rs
│   │   └── m_*_create_workflow_steps_table.rs
│   └── models/
│       ├── mod.rs
│       └── user.rs                 # `#[suprnova::model]` User モデル
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html                  # Vite エントリー；SPA をマウント
│   └── src/
│       ├── main.{tsx,ts}           # Inertia クライアント設定（フレームワークごと）
│       ├── app.css                 # グローバルスタイル + Tailwind
│       ├── pages/
│       │   ├── Home.{tsx,svelte,vue}
│       │   ├── Dashboard.{tsx,svelte,vue}
│       │   └── auth/
│       │       ├── Login.{tsx,svelte,vue}
│       │       └── Register.{tsx,svelte,vue}
│       └── types/
│           └── inertia-props.ts    # #[derive(InertiaProps)] から自動生成
└── public/
    └── assets/                     # Vite 本番ビルド出力がここに格納される
```

Svelte は `frontend/svelte.config.js` と `frontend/src/app.d.ts` を追加します。
Vue は `frontend/src/shims-vue.d.ts` を追加します。

API スターター（`suprnova new my-api --api`）はよりシンプルです：
`frontend/` がなく、認証コントローラーがなく、`cmd/main.rs` は `src/main.rs` に置き換わります。

## 各ディレクトリの目的

### `cmd/main.rs`

バイナリエントリーポイント。短いファイル（通常 10～20 行）で、標準的なブートパイプラインを呼び出します：

```rust
use suprnova::Application;
use my_app::{bootstrap, config, migrations, routes};

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

`Application::run()` はバイナリの CLI（`serve` / `web:run` /
`migrate*` / `schedule:*` / `workflow:work` / `queue:work`）をパースし、
`.env` をロードして、設定関数を実行してから、サブコマンドをディスパッチします。
serve パスはまた、ブートストラップ関数を実行して HTTP サーバーを起動します。

初期スキャフォルド後は `cmd/main.rs` をほぼ編集することはありません。

### `src/lib.rs`

フラットなモジュール宣言ファイル：

```rust
pub mod actions;
pub mod bootstrap;
pub mod commands;
pub mod config;
pub mod controllers;
pub mod middleware;
pub mod migrations;
pub mod models;
pub mod routes;
```

これにより、`routes.rs` から `crate::controllers::home::index` に到達可能になります。

### `src/bootstrap.rs`

アプリを配線する単一の関数です。サービスコンテナバインディング、オブザーバー、イベントリスナー、カスタムミドルウェア、その他のブート時設定をここに登録します。Laravel の `AppServiceProvider`、
`EventServiceProvider`、`BroadcastServiceProvider` などすべてを 1 つのファイルに統合したものです：

```rust
use std::sync::Arc;
use suprnova::App;

pub async fn register() {
    // コンテナにサービスをバインド
    App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));

    // Eloquent オブザーバーを登録
    crate::models::user::register_observer();

    // イベントをリッスン
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
}
```

`register()` はプロセスごとに 1 回、設定ローダーの後で `serve` が最初のリクエストを受け入れる前に実行されます。ワーカー（`queue:work`、`schedule:run`、`workflow:work`）は同じブートストラップを再利用するため、同じサービスを参照します。[アプリケーション ブートストラップ](bootstrap.md) を参照してください。

### `src/routes.rs`

URL サーフェス。モジュールトップレベルの `routes!` マクロは
`pub fn register() -> Router` に展開され、`cmd/main.rs` がこれを
`Application::routes(...)` に渡します：

```rust
use suprnova::{get, post, put, delete, routes};
use crate::{controllers, middleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // Auth（登録済み + 保護済み）
    get!("/login", controllers::auth::show_login).name("login.show"),
    post!("/login", controllers::auth::login).name("login.attempt"),
    post!("/logout", controllers::auth::logout).name("logout"),
    get!("/register", controllers::auth::show_register).name("register.show"),
    post!("/register", controllers::auth::register).name("register"),

    // Dashboard は authenticate middleware が必要
    get!("/dashboard", controllers::dashboard::index)
        .middleware(middleware::authenticate::auth())
        .name("dashboard"),
}
```

[ルーティング](routing.md) を参照してください。

### `src/bin/console.rs`

プロジェクトごとのコンソールバイナリ。`cargo run --bin console <subcommand>` として実行され、フレームワークの `db:seed` ビルトインと `src/commands/` 内のすべての `#[command]` アノテーション付きハンドラ（または `#[derive(Command)]` 型付きストラクト）をディスパッチします。両方の形式はコンパイル時にインベントリを通じて登録されます：

```bash
cargo run --bin console db:seed           # フレームワーク ビルトイン
cargo run --bin console report:daily      # カスタムコマンド
```

長時間実行ワーカー（`queue:work`、`schedule:run`、`schedule:work`、`workflow:work`）はメインアプリバイナリに存在します。`Application::run()` がこれらをディスパッチするため、
`cargo run -- queue:work` として呼び出すか、
`suprnova schedule:run` / `suprnova workflow:work` を使用できます（傘型 CLI を好む場合）。

[コンソール](console.md) を参照してください。

### `src/commands/`

コンソールハンドラの保存場所。2 つの形式があります：clap 由来の引数と `impl TypedCommand` を持つ型付きストラクト、または `async fn(Vec<String>) -> Result<(), FrameworkError>` への raw `#[command]`。スキャフォルダーは型付き形式を生成します：

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "report:daily", description = "Generate the daily report")]
pub struct DailyReport {
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for DailyReport {
    async fn run(self) -> Result<(), FrameworkError> {
        // …
        Ok(())
    }
}
```

`suprnova make:command report-daily` はファイルをスキャフォルドして `src/commands/mod.rs` に追加します。
[コンソール](console.md) を参照してください。

### `src/config/`

型付き設定ストラクト。スキャフォルドは `database.rs` と `mail.rs` を提供します。アプリが必要とするサブシステムに対して独自のものを追加してください。各設定ストラクトは環境から値を読み取り、`config::register_all()` がフレームワークに登録します：

```rust
use suprnova::{env, env_required};

#[derive(Clone, Debug)]
pub struct AnalyticsConfig {
    pub api_key: String,
    pub max_batch: u32,
}

impl AnalyticsConfig {
    pub fn from_env() -> Self {
        Self {
            api_key: env_required::<String>("ANALYTICS_API_KEY"),
            max_batch: env("ANALYTICS_MAX_BATCH", 100u32),
        }
    }
}
```

`config/mod.rs` に配線してください：

```rust
use suprnova::Config;

pub fn register_all() {
    Config::register(AnalyticsConfig::from_env());
}
```

[設定](configuration.md) を参照してください。

### `src/controllers/`

HTTP ハンドラ関数。リソースごとに 1 つのモジュール。`Request` を取得して `Response` を返す
`pub async fn` はルートから呼び出し可能です。

### `src/middleware/`

Middleware 実装。スキャフォルドは `logging` と `authenticate` を提供します。
`pub struct Foo` と `impl Middleware for Foo` として独自のものをここに追加してください。
`bootstrap.rs` でグローバルに登録するか、`routes!` ツリーの `.middleware(…)` 経由でルートごとに適用してください。
[ミドルウェア](middleware.md) を参照してください。

### `src/migrations/`

SeaORM マイグレーター。スキャフォルドは認証とワークフロー テーブル用のいくつかを提供します。
`suprnova make:migration <name>` は新しいものを追加します。`suprnova migrate`、`migrate:rollback`、
`migrate:status`、`migrate:fresh`、`db:sync` はすべてこのディレクトリで動作します。
[マイグレーション](migrations.md) を参照してください。

### `src/models/`

Eloquent モデル。モデルごとに 1 つのファイル、それぞれ `#[suprnova::model]` ストラクト。スキャフォルドは `user.rs` を提供します。新しいモデルは手動で新しいファイルを書くか、スキーママイグレーション後に `suprnova db:sync --regenerate-models` を実行して追加してください。
[Eloquent](eloquent.md) を参照してください。

### `src/actions/`

シングルメソッドの呼び出し可能なコントローラー。オプションのパターン。コントローラーが正確に 1 つのメソッドを持つ場合に使用してください。ラップするよりも「Action」と呼ぶ方がよい場合です。スキャフォルドは削除または適応可能な例を提供します。[アクション](actions.md) を参照してください。

### `frontend/`

Vite + Inertia SPA。これは通常のフロントエンドプロジェクトです。`package.json`、`vite.config.ts`、
`tsconfig.json`、`index.html` Vite エントリー、`src/` 下のソース。Inertia クライアント設定は
`src/main.{tsx,ts}` に存在し、ページコンポーネントは `src/pages/` にあります。Rust の
`#[derive(InertiaProps)]` プロップ用の TypeScript 型は `suprnova generate-types` によって
`src/types/inertia-props.ts` に再生成されます。

[フロントエンド](frontend.md) を参照してください。

### `public/assets/`

Vite が本番ビルド（`npm run build`）をドロップする場所。Suprnova サーバーは本番環境でこのディレクトリを `/assets/*` で静的アセットとして提供します。

## アプリが成長するにつれて追加するディレクトリ

スキャフォルドは最小限のもの（ウェルカムフローと保護されたダッシュボードを配信するのに十分）を提供します。実際のアプリはより多くのサブシステムが成長します。一般的な追加：

| ディレクトリ | 追加するタイミング |
|---|---|
| `src/jobs/` | `Queue::push(SomeJob)` を最初に使用するとき。[キュー](queues.md) を参照してください。 |
| `src/listeners/` | `Event::listen` を最初に使用するとき。[イベント](events.md) を参照してください。 |
| `src/observers/` | `Observer<MyModel>` を最初に実装するとき。[Eloquent](eloquent.md#observers) を参照してください。 |
| `src/notifications/` | `Notification` を最初に実装するとき。[通知](notifications.md) を参照してください。 |
| `src/mail/` | `Mailable` を最初に実装するとき。[メール](mail.md) を参照してください。 |
| `src/policies/` | `#[policy]` を最初に書くとき。[認可](authorization.md) を参照してください。 |
| `src/factories/` | テスト用に `Factory<Model>` を最初に書くとき。[Eloquent ファクトリー](eloquent-factories.md) を参照してください。 |
| `src/seeders/` | `db:seed` 用に `Seeder` を最初に書くとき。[シーディング](seeding.md) を参照してください。 |
| `src/events/` | 独自のイベント型に `impl Event` を最初に実装するとき。[イベント](events.md) を参照してください。 |
| `src/broadcasting/` | private/presence `Channel` を最初に定義するとき。[ブロードキャスト](broadcasting.md) を参照してください。 |
| `src/ws/` | `ws!()` ハンドラを最初に書くとき。[WebSocket](websockets.md) を参照してください。 |
| `src/supervisors/` | 長時間実行 `Supervisor` を最初に実装するとき。[スーパーバイザー](supervisors.md) を参照してください。 |
| `src/payments/` | Stripe/Paddle をアプリに最初に接続するとき。[支払い](payments.md) を参照してください。 |
| `src/props/` | `#[derive(InertiaProps)]` ストラクトをコントローラーから分離したいとき。 |
| `resources/views/` | メール本文用に Tera テンプレートを最初に追加するとき。 |
| `storage/` | ローカルファイルシステムディスクにファイルを最初に書くとき（[ファイルストレージ](filesystem.md) を参照）。 |
| `tests/` | 統合テストを最初に書くとき。 |

許可を求める必要がありません。`mkdir src/jobs` を実行して `src/lib.rs` に `pub mod jobs;` を追加するだけです。フレームワークはディレクトリ名を強制しません。規約は他の Suprnova 開発者が素早く物を見つけられるように存在します。

## このリポジトリ内のドッグフード `app/`

Suprnova リポジトリ自体からこれを読んでいる場合は、ルートに `app/` ディレクトリがあり、すべてのフレームワーク機能を一緒に使用しています。これは内部テストベッドです。支払い、ブロードキャスト、Web プッシュ、ワークフロー、スーパーバイザーなどをすべて一度に実行します。新しいアプリの参照ではありません。上記のスキャフォルド出力は意図的に小さくて学びやすくなっています。ピースがどのように構成されるかの最大例を見たい場合は `app/` を読んでください。

## 次のステップ

- [設定](configuration.md) - `.env` がどのように型付き設定になるか
- [アプリケーション ブートストラップ](bootstrap.md) - `bootstrap.rs` が実際に何をするか
- [ルーティング](routing.md) - 最初のルート
- [サービス コンテナ](container.md) - `App::bind` と `App::get` の動作方法
