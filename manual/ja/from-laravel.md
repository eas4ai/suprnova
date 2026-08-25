# Laravel から

Laravel アプリを本番運用した経験があれば、Suprnova の 80% は既に理解しています。このチャプターでは、あなたの習慣を Rust での対応物にマップし、素早く生産性を高めることができるようにします。毎日使う パターン、形が変わるパターン、そして PHP にはできない Rust が無料でくれる、ほんの少しのものを紹介します。

## 簡単対比表

| Laravel で書いたこと | Suprnova で書くこと |
|---|---|
| `composer create laravel/laravel my-app` | `suprnova new my-app --frontend svelte` |
| `php artisan serve` | `suprnova serve` |
| `php artisan migrate` | `suprnova migrate` |
| `php artisan make:controller PostController` | `suprnova make:controller post` |
| `Route::get('/posts/{id}', [PostController::class, 'show'])` | `get!("/posts/{id}", controllers::post::show)`（`routes!` の中） |
| `class Post extends Model` | `#[suprnova::model] struct Post { … }` |
| `Post::find($id)` | `Post::find(id).await?` |
| `Post::where('status', 'published')->get()` | `Post::query().db_where("status", "published").get().await?` |
| `Auth::user()` | `Auth::user().await?` |
| `Cache::remember('key', 60, fn() => …)` | `Cache::remember("key", Some(Duration::from_secs(60)), \|\| async { … }).await?` |
| `Queue::push(new SendEmail($user))` | `Queue::push(SendEmail { user_id }).await?` |
| `Mail::to($u)->send(new Welcome($u))` | `Mail::to(&u.email).send(WelcomeMail { user: u }).await?` |
| `Storage::disk('s3')->put($path, $bytes)` | `Storage::disk("s3")?.put(&path, bytes).await?` |
| `Notification::send($u, new Invoice($i))` | `Notify::send(&u, &InvoiceNotification { invoice }).await?` |
| `Gate::allows('update', $post)` | `Gate::allows::<PostPolicy, _>("update", &user, &post).await?` |
| `request()->validate([...])` | `#[handler]` extracts an `#[derive(Data, Validate)]` arg directly |
| `event(new OrderShipped($order))` | `EventFacade::dispatch(OrderShipped { order }).await?` |
| `Bus::dispatch(new ProcessFoo($x))` | `Bus::dispatch(ProcessFoo { x }).await?` |
| `php artisan schedule:list` | `suprnova schedule:list` |
| `php artisan tinker` | （REPL はありません - 使い捨ての `cargo run` スクリプトかテストを書きます）|
| `composer require league/csv` | `cargo add csv` |

## メンタルモデルの転換

### 非同期、すべてのところで

最も大きな変更点は、すべてのデータベースコール、HTTP コール、ファイル I/O、キャッシュコール、キューへのプッシュ（つまり、境界を超えるすべてのもの）が `async` であり、`.await?` で呼び出すということです。数時間これを行えば、リズムの中に消え去ります。それまでの間、コンパイラーは忘れたすべてのスポットを指摘してくれます。

```rust
// Laravel
$user = User::find($id);
$user->subscribe($plan);
Mail::to($user)->send(new Welcome($user));

// Suprnova
let user = User::find(id).await?;
user.subscribe(&plan).await?;
Mail::to(&user.email).send(WelcomeMail { user }).await?;
```

`?` は Rust の「エラー時の早期リターン」です。ハンドラは `Result<HttpResponse, HttpResponse>` （エイリアス `Response`） を返すため、DB エラーで `?` を使用するとエラーコンバーターに短絡され、クライアントは適切な 500 （またはエラーの種類に応じて 4xx） を取得します。`try/catch` を書く必要はほぼありません - `?` がそれをします。

### コンパイル時のモデル

Eloquent はランタイムで DB スキーマを読み込みますが、Suprnova はコンパイル時に読み込みます。

```rust
#[suprnova::model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

それで終わりです。その struct が Eloquent モデルです。`Post::find`、`Post::query()`、`Post::create`、`post.update(...)`、`post.delete()`、ソフトデリート（`#[model(soft_deletes)]` で）、タイムスタンプ、オブザーバー、すべてが揃っています。マクロは SeaORM の `Entity`、`Model`、`ActiveModel`、`Column` enum を生成し、Suprnova `Model` トレイトを実装します。しかし、あなたは `Post` に依存するため、これらのいずれにも依存しません。

マイグレーションでカラムの名前を変更した場合、struct は DB スキーマともう一致しません。そして、あなたのコンフィグによって、コンパイラーがビルド時にそれを検出するか、型強制キャストが最初のクエリで失敗するかのいずれかです。どちらの場合でも、ステージングの後ではなく、その前に判明します。

### 単一バイナリ

PHP-FPM はありません。`index.php` を読むための nginx コンフィグもありません。デプロイ時に `composer install` もありません。`cargo build --release` は 1 つの静的リンクされたバイナリをあなたに与えます。それをサーバーに `scp` し、`systemd` で実行すれば完了です。またはコンテナをビルドすれば - `FROM scratch` が機能します。

[デプロイメント レシピ](deployment.md)を Railway、Digital Ocean、Hetzner 用に用意しています。共通の形状は：バイナリをビルドし、バイナリをシップし、環境変数を設定し、実行します。

## フレームワークのマッピング

### ルーティング

`routes!` は `routes/web.php` と `routes/api.php` を組み合わせた役割を果たします。

```rust
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),

    // 共有のプレフィックス + ミドルウェアを持つルートグループ
    group("/admin")
        .middleware(crate::middleware::admin())
        .routes(routes! {
            get!("/users", controllers::admin::users::index).name("admin.users"),
            post!("/users", controllers::admin::users::store),
            put!("/users/{id}", controllers::admin::users::update),
            delete!("/users/{id}", controllers::admin::users::destroy),
        }),

    // リソースルーティング（Laravel の Route::resource）
    resource!("posts", controllers::post),
}
```

完全なリファレンス：[ルーティング](routing.md)。知っておく価値のある違い：

- グループミドルウェアは、登録時に各ルートのミドルウェアリストに **フラット化** されます（別個のチェーンレイヤーとして実行されません）。つまり、グループ化に追加のランタイムコストはありません。
- Laravel の `{id}` と Rails スタイルの `:id` 構文の両方が機能します。内部的に正規化されます。
- 名前付きルートは `route("posts.show", &[("id", "42")])` で解決され、時間制限付きリンク用の署名付き URL バリアントもあります。

### コントローラー

コントローラーは単に `Response` を返す自由関数です。

```rust
use suprnova::{Request, Response, json_response, HttpResponse};
use crate::models::Post;

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0").parse::<i64>()?;
    let post = Post::find_or_fail(id).await?;
    json_response!({ "post": post })
}
```

署名で型付きの引数（ルートパラメータ、クエリ、ボディ、リクエスト自体、コンテナサービス）を抽出するために、`#[handler]` マクロも使用できます。

```rust
use suprnova::handler;

#[handler]
pub async fn show(post: post::Model) -> Response {
    // ルートモデル結合が自動的に実行され、`post` は読み込まれた行です。
    json_response!({ "post": post })
}
```

`post::Model` 型はモデルの生成されたモジュールから来ます。これは `#[handler]` が、デフォルトのフォームリクエスト抽出よりもルートモデルバインディングを選択するために使用するシグナルです。行が存在しない場合、バインディングはコードが実行される前に 404 を返します。Laravel の暗黙的バインディングと同じ動作です。

アクション struct （単一メソッドの「呼び出し可能な」コントローラー、Laravel スタイル）もサポートされています。[アクション](actions.md)を参照してください。

### Eloquent

デュアル API クエリビルダーは Laravel 名または Rust イディオマティック名のいずれかを受け取ります。両方が機能するため、呼び出しサイトで最も読みやすいものを選択します。

```rust
// Laravel の表面
let active = User::query()
    .db_where("status", "active")
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;

// Rust の表面（結果は同一）
let active = User::query()
    .filter("status", "active")
    .order_by_desc("created_at")
    .take(20)
    .get()
    .await?;
```

`db_where` は Laravel 側の名前です（むき出しの `where` は Rust キーワードと衝突します）。`filter` は Rust イディオマティック alias です。どちらも存在し、両方が同じことを行います。非等号オペレーターの場合は、`db_where_op` （またはその `filter_op` alias）を使用します。`.db_where_op("status", "!=", "archived")` を参照してください。[Eloquent リファレンス](eloquent.md)を参照してください。理由があってこれが最も長いチャプターであり、表面は広いです。

### 認証

```rust
use suprnova::{Auth, Credentials};

// ハンドラの中で:
let user = Auth::user().await?;   // Option<Arc<dyn Authenticatable>>
let id = user.as_ref().map(|u| u.get_auth_identifier());

// ログイン（例えばログインコントローラーの中で）:
let creds = Credentials::password("alice@x.com", "secret");
Auth::attempt(&creds, false).await?;

// ログアウト:
Auth::logout().await?;
```

`Auth::attempt` は、デフォルトのステートフルガードとその設定済みの `UserProvider` を通じて認証情報を検証します。これは生成されたフルスタックスキャフォルドが使う経路です。 パスワードリセットでは、`EloquentUserProvider` など、リセット機能が明示されたプロバイダーを介して検証済みユーザーをサポートします。リセットを最初のメールボックスのアトミックな証明として使用する必要がある場合は、Magnetar をインストールしてください。`Auth::password()`、`BruteForce`、パスキー、マジックリンク、OAuth、Bearer セッション、Magnetar のセッション管理には、インストール済みの Magnetar エンジンが必要です。 See [Authentication](authentication.md), [Auth flows](auth-flows.md), and [OAuth and passwordless login](oauth.md).

### マイグレーション

SeaORM マイグレーターを書きます。構文が新しくても、形状は見覚えのあるものに見えます。

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(
            Table::create()
                .table(Alias::new("posts"))
                .if_not_exists()
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key().auto_increment())
                .col(ColumnDef::new(Alias::new("title")).string().not_null())
                .col(ColumnDef::new(Alias::new("body")).text().not_null())
                .to_owned()
        ).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new("posts")).to_owned()).await
    }
}
```

`suprnova make:migration create_posts_table` はファイルをスキャフォルドします。`suprnova migrate`、`migrate:rollback`、`migrate:status`、`migrate:fresh` はすべて期待通りに機能します。`suprnova db:sync` はマイグレーションを実行し、マクロレイヤーがコンパイルする SeaORM エンティティを再生成します。[マイグレーション](migrations.md)を参照してください。

### キューとスケジューリング

```rust
use suprnova::{FrameworkError, Job, Queue, async_trait};
use serde::{Deserialize, Serialize};

// ジョブを定義します - データは構造体に置かれ、契約は
// `impl Job` に置かれます。
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SendWelcomeEmail {
    pub user_id: i64,
}

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str {
        "SendWelcomeEmail"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}

// キューに積みます:
Queue::push(SendWelcomeEmail { user_id: user.id }).await?;

// あるいは遅延を付けて:
Queue::later(
    std::time::Duration::from_secs(60),
    SendWelcomeEmail { user_id },
).await?;
```

ワーカーは `cargo run -- queue:work` で実行されます。ドライバーには、`memory` と `sync`（プロセス内、テスト用）、`database`、`redis`、`null` があります。バッチ、チェーン、ユニークジョブ、リトライ、バックオフ、ミドルウェア、失敗したジョブストアもすべて揃っています。[キュー](queues.md)を参照してください。

スケジューリングは `Task` トレイトとプロジェクトごとのスケジューラーバイナリを使用します。

```rust
use suprnova::{Task, TaskResult, async_trait};

pub struct DailyDigest;

#[async_trait]
impl Task for DailyDigest {
    async fn handle(&self) -> TaskResult {
        // …
        Ok(())
    }
}

// bootstrap の中で登録します（例えば Schedule::call / .task / .add 経由で）:
//   schedule.add(schedule.task(DailyDigest).daily().at("03:00").name("daily-digest"));
```

[タスク スケジューリング](scheduling.md)を参照してください。

### メール、通知、ブロードキャスト

これらは Laravel と一対一で従います。`Mailable` は derive マクロです。`Notifiable` はユーザーモデルのトレイトです。チャネルは `mail`/`database`/`broadcast`/`webpush` です。ブロードキャストはパブリック、プライベート、プレゼンスチャネルをサポートしています。[メール](mail.md)、[通知](notifications.md)、[ブロードキャスト](broadcasting.md)を参照してください。

### フロントエンド

Blade はありません。代わりに、フロントエンドは Inertia.js 経由の本物の SPA であり、Rust から型付きプロップを渡します。

```rust
use suprnova::{inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps, serde::Serialize)]
pub struct ShowProps {
    pub post: Post,
    pub comments: Vec<Comment>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    let comments = post.comments().get().await?;
    inertia_response!(&req, "Posts/Show", ShowProps { post, comments })
}
```

`Posts/Show` は Svelte コンポーネント（または React、または Vue - あなたのスターターが選択）です。プロップの TypeScript 型は `InertiaProps` derive から自動的に生成されます。新しいプロップ struct を追加した後、`suprnova generate-types` を実行すると、フロントエンドは型付きバインディングを取得します。

Laravel で `inertia()` 経由で Inertia を使用したことがある場合、これは同じものです。ただエンドツーエンドで型付きです。[フロントエンド 概要](frontend.md)を参照してください。

## 形が変わるもの

Suprnova では異なる動き方をするものが少しあります。これらのいずれもブロッカーではありませんが、事前に知っておく価値があります。

### サービスプロバイダーなし

Laravel には、バインディング、オブザーバー、ビュー コンポーザーなどを登録する数十のサービスプロバイダーがあります。Suprnova は、アプリの `bootstrap.rs` に **1 つ** のブートストラップ関数を持っています。あなたはそこにすべてを順番に登録します。エレガントではありませんが、透明性があります。30 行でアプリがブートするものを正確に見ることができます。

```rust
// bootstrap.rs
use std::sync::Arc;

pub async fn register() {
    suprnova::App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
    crate::observers::register();
}
```

[コンテナ](container.md)と[ブートストラップ](bootstrap.md)チャプターが詳細を持っています。

### コンフィグは型付き

Laravel は `config('app.timezone')` を使用して、配列が言う何かを返します。Suprnova は型付きコンフィグ struct を持っています。

```rust
let cfg = suprnova::Config::get::<AppConfig>()?;
let tz = &cfg.timezone;   // mixed ではなく &str
```

あなた自身の型付きコンフィグセクションを登録できます。[設定](configuration.md)を参照してください。

### ファサード・アズ・エイリアスなし

`DB::` のような Laravel ファサードは、`config/app.php` で設定されたクラスエイリアスです。Suprnova ファサードはクレートルートの実モジュールです。

```rust
use suprnova::{Auth, Cache, DB, Event, Gate, Mail, Notify, Queue, Schedule, Storage};
```

同じ表面で、グローバルエイリアスは不要です。

### コンパイル時間は現実です

Rust のコンパイル時間は PHP ではありません。新しい Suprnova アプリのクリーンビルドには 1-2 分かかります。開発中のインクリメンタルビルドは数秒です。開発ワークフローは同じです。`suprnova serve` が変更を監視して再ビルドしますが、初めてマクロを変更してダウンストリームクレートを再コンパイルするときに感じることになります。キャッシング自体の代金を高速で支払います。

### ボローチェッカーが存在します

ほとんどのコントローラーとハンドラはライフタイムアノテーションに触れることはありません。フレームワークの署名がそれらを隠します。ボローチェッカーがあなたを叱ると、それはあなたが、ミューテックスを越えた `.await` にわたって参照を保持しようとした、または排他的アクセスが必要な待機呼び出しにわたって DB トランザクションを保持したからです。エラーは明確で、修正はほぼ常に `.clone()` または、より小さなスコープへの再構成です。

### `tinker` REPL なし

REPL はありません。最も近い同等物は、`examples/` の 1 回限りの `cargo run` スクリプト、またはデバッグしているものを実行する `#[suprnova_test]` テストです。tinker でやることのほとんど（モデルをいじる、通知を発火させる、ジョブをディスパッチする）は 5 行のテストです。

## Laravel チャプターが着陸するところ

何を探しているかは知っているが、どこに存在するかは知らない場合の素早い参照：

| Laravel トピック | Suprnova チャプター |
|---|---|
| ライフサイクル | [リクエスト ライフサイクル](lifecycle.md) |
| サービス コンテナ | [サービス コンテナ](container.md) |
| サービス プロバイダー | [アプリケーション ブートストラップ](bootstrap.md) |
| ファサード | [サービス コンテナ](container.md) |
| ルーティング | [ルーティング](routing.md) |
| ミドルウェア | [ミドルウェア](middleware.md) |
| CSRF 保護 | [CSRF 保護](csrf.md) |
| コントローラー | [コントローラー](controllers.md) |
| リクエスト | [リクエスト](requests.md) |
| レスポンス | [レスポンス](responses.md) |
| URL 生成 | [URL 生成](urls.md) |
| セッション | [セッション](session.md) |
| バリデーション | [バリデーション](validation.md) |
| エラー ハンドリング | [エラー ハンドリング](errors.md) |
| ロギング | [ロギング](logging.md) |
| Artisan コンソール | [コンソール](console.md) + [CLI リファレンス](cli.md) |
| ブロードキャスト | [ブロードキャスト](broadcasting.md) |
| キャッシュ | [キャッシュ](cache.md) |
| イベント | [イベント](events.md) |
| ファイル ストレージ | [ファイルシステムとストレージ](filesystem.md) |
| HTTP クライアント | [HTTP クライアント](http-client.md) |
| ローカライゼーション | [ローカライゼーション](localization.md) - Fluent `.ftl` カタログ、PHP 配列ではなく |
| メール | [メール](mail.md) |
| 通知 | [通知](notifications.md) |
| キュー | [キュー](queues.md) |
| レート リミット | [レート リミット](rate-limiting.md) |
| タスク スケジューリング | [タスク スケジューリング](scheduling.md) |
| 認証 | [認証](authentication.md) |
| 認可 | [認可](authorization.md) |
| メール 認証 | [認証フロー](auth-flows.md) |
| パスワード リセット | [認証フロー](auth-flows.md) |
| 暗号化 | [暗号化](encryption.md) |
| ハッシング | [ハッシング](hashing.md) |
| データベース | [データベース](database.md) |
| クエリ ビルダー | [クエリ ビルダー](queries.md) |
| ページネーション | [ページネーション](pagination.md) |
| マイグレーション | [マイグレーション](migrations.md) |
| シーディング | [シーディング](seeding.md) |
| Eloquent | [Eloquent API](eloquent.md) |
| Eloquent: リレーションシップ | [リレーションシップ](eloquent-relationships.md) |
| Eloquent: コレクション | [コレクション](eloquent-collections.md) |
| Eloquent: キャスト、アクセッサーとミューテータ | [キャスト、アクセッサーとミューテータ](eloquent-mutators.md) |
| Eloquent: API リソース | [JSON:API リソース](eloquent-resources.md) |
| Eloquent: シリアライゼーション | [シリアライゼーション](eloquent-serialization.md) |
| Eloquent: ファクトリー | [ファクトリー](eloquent-factories.md) |
| テスト | [テスト](testing.md) |
| HTTP テスト | [HTTP テスト](http-tests.md) |
| データベース テスト | [データベース テスト](database-testing.md) |
| モッキング | [モックとフェイク](mocking.md) |
| Cashier (Stripe) | [支払い - Stripe アダプター](payments-stripe.md) |
| Cashier (Paddle) | [支払い - Paddle アダプター](payments-paddle.md) |
| Sanctum / Passport | `BearerTokenMiddleware` を通じたMagnetar bearerセッション。個別のSanctumまたはPassport APIはありません |
| Horizon | キュー検査はフレームワークに組み込まれています。Horizonダッシュボードはありません |
| Telescope / Pulse | （v2+ に延期） |

Laravel が Suprnova にはまだない（まだ）ものがあります:

- Telescope / Pulseダッシュボード。基本的な[可観測性](observability.md)は出荷されています。
- Sanctum / PassportパッケージAPI。Magnetar bearerセッションと `BearerTokenMiddleware` がトークン認証を提供しますが、Laravelのトークン管理表面は提供しません。
- Horizonのダッシュボード。キュー検査はフレームワークに組み込まれています。
- Blade - 設計上；Inertia はフロントエンドストーリーです
- `trans_choice` - [ローカライゼーション](localization.md)は出荷されていますが、複数形は `trans_choice` が取る `[1,19]` スタイルの整数範囲ではなく、CLDR カテゴリーによってメッセージ内で選択されます

## 次のステップ

- [インストール](installation.md) - プロジェクトを実行
- [クイックスタート](quickstart.md) - 5 分で小さなアプリをビルド
- [ルーティング](routing.md) - ここから次の自然なチャプター

または [`documentation.md`](documentation.md) 経由でどこへでもジャンプしてください。
