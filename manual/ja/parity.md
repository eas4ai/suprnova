# Laravel パリティ マップ

Laravel 13.x と Suprnova を、機能ごとに正直に対応づけたマップです。「Suprnova に X はあるか？」と尋ねたいとき、1行で yes/no/どこにあるかという答えが欲しいときに使ってください。

各セクションは Laravel のドキュメント索引を反映しているため、Laravel 開発者は上から下へ一通り目を通せます。各セクション内で、列は常に同じです:

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|

**ステータス** 列は、次の4つの値を使います:

| 記号 | 意味 |
|---|---|
| **実装済み** | 同じ表面、同じふるまい（多くの場合、メソッド名も同じ） |
| **差異あり** | 仕事は同じですが、Rust がより良い選択を可能にするため形が異なります |
| **未実装** | 本当に計画されていますが、まだディスク上にはありません |
| **意図的に非対応** | 出荷しません - 理由は備考列に記載されています |

該当するチャプターがある場合は、**備考**列からリンクされています。

これは生きたマップです。Suprnova は、ドキュメント化された30のドメインにわたり Laravel 13.x の表面をすべて出荷しています。以下に挙げるギャップは、出荷済みのフレームワークにおける、現時点での本物のギャップです。

## アーキテクチャの概念

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| リクエスト ライフサイクル | `Application` → `Server` → `handle_request` chain | 実装済み | [ライフサイクル](lifecycle.md) |
| サービス コンテナ | `Container` + `App` ファサード、3層（タスク / スレッド / グローバル） | 差異あり | リクエストごとはタスクローカル、テストはスレッドローカル - [コンテナ](container.md) |
| サービス プロバイダー | `bootstrap()` 関数 + `#[service]`、`#[policy]`、`#[command]`、オブザーバーマクロ | 差異あり | 登録用のクラスはありません - bootstrap は1つの関数です。マクロはコンパイル時登録に `inventory` を使います。[ブートストラップ](bootstrap.md) |
| ファサード | 静的な `App::get`、`Cache::*`、`Mail::*`、`Auth::*`、`Storage::*`、`Queue::*`、`Bus::*`、`Event::*`、`Notification::*`、`Gate::*`、`Schedule::*`、`DB::*`、`Vector::*` | 実装済み | 呼び出しの形は同じです。ファサードはエイリアスではなく、実在の型です |
| コントラクト | トレイト - `Mailer`、`KeyValueStore`、`Hasher`、`Channel`、`VectorDriver`、`Evaluator`、`PaymentProvider` など | 実装済み | 公開されているすべての接続点はトレイトの上にあります。トレイトでバインドし、実装は自由に差し替えられます |

## はじめる

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| インストール | `cargo install --git …suprnova-cli` then `suprnova new <name>` | 実装済み | [インストール](installation.md) |
| 設定 | `#[derive(Config)]` + `Config::register` による型付き設定 | 差異あり | 配列バッグではなく、コンパイル時の型付き設定です。[設定](configuration.md) |
| エージェント型開発（AI） | フレームワークにファーストクラスのAI SDKはありません | 意図的に非対応 | どのみち使うことになるクレート（`async-openai`、`anthropic-rs`、`tokenizers` など）を `App::bind(Arc<dyn YourLlm>)` の下で使ってください |
| ディレクトリ構成 | `src/{actions,bootstrap,controllers,middleware,models,routes}` | 実装済み | 意図は同じ、Rustらしいレイアウトです。[ディレクトリ構成](structure.md) |
| フロントエンド | Svelte 5 / React 19 / Vue 3.5 の上で動く Inertia v3 | 実装済み | [フロントエンド](frontend.md)、[ページ](frontend-pages.md)、[TS 型](frontend-typescript-types.md) |
| スターターキット | **Nebula**（認証）と **Pulsar**（フルのプロダクトサイト）、それに素の `suprnova new` スキャフォルド | 実装済み | 今日、2つのキットが出荷されています - Nebula は Breeze の等価物です。Pulsar はドキュメント、ブログ、コミュニティ、RBAC を追加します。[スターターキット](starter-kits.md) |
| デプロイメント | 単一バイナリ。Docker / Railway / DO / Hetzner のレシピ | 差異あり | PHPランタイム + opcache + FPM ではなく、1つのアーティファクトです。[デプロイメント](deployment.md) |

## 基本

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| ルート定義 | `routes!` マクロ + `get!` / `post!` / `put!` / `patch!` / `delete!` / `any!` / `head!` / `options!` / `fallback!` / `ws!` | 実装済み | [ルーティング](routing.md) |
| ルートパラメータ | `{id}` パスパラメータ + `req.param("id")` | 実装済み | `{id?}` によるオプションのパラメータ、`where!()` による制約 |
| ルート名 | ルート上の `.name("posts.show")` + `url("posts.show", &[("id", "42")])` | 実装済み | [URL 生成](urls.md) |
| ルートグループ | `.prefix()` / `.middleware()` / `.name()` / `.controller()` を伴う `group!` マクロ | 実装済み | グループミドルウェアは、登録時に各ルートへ平坦化されます |
| リソースルート | `resource!("posts", PostController)` が7つの標準ルートを登録します | 実装済み | `apiResource!`、`only(...)`、`except(...)` はすべてサポートされています |
| 署名付きURL | `sign_url(...)`, `sign_route(...)`, `verify_signature(...)` | 実装済み | `APP_KEY` を使った HMAC-SHA256 |
| ルートモデルバインディング | `#[handler]` は、`RouteBinding` の実装を介して `{post}` から `Post` を抽出します | 実装済み | `AutoRouteBinding` derive が `#[suprnova::model]` 型に対して自動実装します |
| レート制限 | `throttle:60,1` ミドルウェア + `RateLimiter::for_signature` | 実装済み | [レート リミット](rate-limiting.md) |
| ミドルウェア | `impl Middleware` トレイト。グローバルまたはルートごとに登録します | 実装済み | [ミドルウェア](middleware.md) |
| ミドルウェアグループ + エイリアス | `register_middleware_group`, `register_middleware_alias` | 実装済み | ルート内で文字列名によって検索します |
| CSRF保護 | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | 実装済み | オリジンポリシーが同一オリジンの POST を強制します。[CSRF](csrf.md) |
| コントローラー | `#[handler] pub async fn show(req: Request) -> Response` | 実装済み | コントローラーは、クラスではなく自由関数のモジュールです。[コントローラー](controllers.md) |
| 単一アクションコントローラー | ハンドラはすでに単一の関数です。モジュールへグループ化します | 実装済み | Rustの慣例です - `__invoke` の儀式は不要です |
| リクエスト | `.input()`、`.param()`、`.query()`、`.header()`、`.cookie()`、`.json()`、`.file()` などを持つ `Request` 構造体 | 実装済み | [リクエスト](requests.md) |
| フォームリクエスト | `#[derive(Data, Validate, FormRequest)]` | 実装済み | 検証は抽出の際に実行されます |
| ファイルアップロード | `req.file("avatar")?` は `UploadedFile` を返します。サイズ + パート数の上限付きストリーミングmultipart | 実装済み | しきい値を超えると自動的に一時ファイルへ溢れます |
| レスポンス | `HttpResponse` ビルダー + `json!()` / `text!()` / `Redirect::to` / `view` | 実装済み | [レスポンス](responses.md) |
| ビュー（Blade） | サーバーレンダリングされたInertiaページ（Svelte/React/Vue） - Bladeに相当するものはありません | 差異あり | Inertia がビュー層です。Blade の代わりに[ページ](frontend-pages.md)を使ってください |
| アセットバンドリング（Vite） | Vite 8 はあらゆるスキャフォルドに出荷されます。`suprnova serve` が Vite + バックエンドを一緒に実行します | 実装済み | マニフェスト読み取り + HMR が自動的に配線されます |
| 静的アセット（`public/`。Laravelではウェブサーバーが配信） | `public/` をウェブルートで配信する、プロセス内のフォールバックハンドラ `StaticFiles::public()` | 実装済み | `StaticFiles::from_dir(...)` + `cache_control(...)`。別のウェブサーバーは不要です |
| URL 生成 | `url("posts.show", &[…])`, `route("posts.show", …)`, `redirect(...)`, `redirect_to(...)` | 実装済み | [URL 生成](urls.md) |
| セッション | `session()`、`session_mut()`、`req.flash()` によるフラッシュバッグ | 実装済み | `DatabaseSessionDriver` によるDBバックエンド、デフォルトはクッキーバックエンドです。[セッション](session.md) |
| バリデーション | `#[derive(Validate)]` + 17個の組み込みルール + `Rule`/`AsyncRule` トレイト | 実装済み | 非同期ルール（例えば `Unique`）はDBに触れます。[バリデーション](validation.md) |
| エラーハンドリング | `FrameworkError`、`AppError`、`HttpError` トレイト、`execute_chain_safely` の中のパニック境界 | 実装済み | [エラーハンドリング](errors.md)、[エラー モデル](error-model.md) |
| ロギング | 構造化フィールドを持つ `tracing` サブスクライバー、`LogFormat`（json / pretty / compact） | 差異あり | ログの1行は1つのJSONドキュメントです。`request_id` が常に存在します。[ロギング](logging.md) |
| Abort ヘルパー | `abort_if(cond, status, msg)`, `abort_unless(...)`, `abort_with(status, msg)` | 実装済み | Laravel の `abort_if` ファミリーと同じ形です |

## さらに掘り下げる

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| Artisan コンソール | `#[command]` + `#[derive(Command)]` から構築される、アプリごとの `console` バイナリ | 実装済み | [コンソール](console.md)。`cargo run --bin console <subcommand>` |
| Tinker（REPL） | REPLはありません | 意図的に非対応 | 使い捨ての `cargo run --bin xxx` スクリプトか `#[suprnova_test]` を書いてください |
| ブロードキャスト | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | 実装済み | マルチノード向けの sea-streamer ファンアウト。[ブロードキャスト](broadcasting.md) |
| キャッシュ | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`, `RedisCache` | 実装済み | アトミック操作 + タグ付きキャッシュ + キャッシュロック（`LockGuard`）。[キャッシュ](cache.md) |
| コレクション | Laravel形式のメソッドを持つ `eloquent::Collection<M>` | 実装済み | `Deref<Target = Vec<M>>` のため、既存のVecのイディオムがそのまま動きます。[コレクション](eloquent-collections.md) |
| 並行処理 | あらゆる場所でTokio - `tokio::spawn`, `tokio::join!`, `tokio::select!` | 実装済み | フレームワーク全体が非同期です。Laravel の `Concurrency::run([...])` ファサードは出荷されません - Tokio がその答えです |
| コンテキスト | `Context::put` / `Context::get` / `ContextStore` + キュー / メール / イベントへの自動注入 | 実装済み | [コンテキスト](context.md) |
| コントラクト | 公開されているすべての接続点はトレイトです | 実装済み | 上の「アーキテクチャ / コントラクト」の行を参照してください |
| イベント | `EventFacade::dispatch(e).await?`、`#[derive(Event)]`、`EventDispatcher`、キューに入れられたリスナー、購読者 | 実装済み | [イベント](events.md) |
| ファイルストレージ | OpenDAL の上の `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` | 実装済み | 同じ `put/get/delete/copy/move/exists/url` の表面です。パストラバーサル保護が組み込まれています。[ファイルシステム](filesystem.md) |
| ヘルパー | 等価物はそれぞれのホームモジュールにあります（何でも入りの `helpers.md` はありません） | 差異あり | 例えば URL ヘルパーは[urls.md](urls.md)に、文字列ヘルパーは `std`/`heck` に、配列ヘルパーは `std::collections` にあります - Rust はこれを、グローバルな名前空間ではなくクレートで行います |
| HTTPクライアント | `Http::get/post/...` ビルダー + テスト用の `Http::fake(...)` | 実装済み | リクエストを自動記録します。`assert_sent` / `assert_not_sent`。[HTTPクライアント](http-client.md) |
| ローカライゼーション | `Lang::get` / `get_with` / `try_get` / `has` + `lang/<locale>/` の Fluent `.ftl` カタログに対する `__!("key", name: value)` マクロ、`LocaleMiddleware` による検出、翻訳されたバリデーションメッセージ、ICU4Xによるフォーマット | 実装済み | 同じカタログが `/_suprnova/lang/<locale>.ftl` でブラウザにも提供され、`generate-types` によって型付けされます。[ローカライゼーション](localization.md) |
| メール | `Mail::to(...).send(MyMail { ... }).await?` + ドライバー `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory` | 実装済み | `Mailable` トレイト + Teraでレンダリングされる HTML/text 本文。[メール](mail.md) |
| 通知 | `Notify::send(&user, notif).await?` + channels `mail/database/broadcast/webpush` | 実装済み | `Notifiable` トレイト + チャネルごとの `Notification`。[通知](notifications.md)、[Web プッシュ](web-push.md) |
| パッケージ開発 | ワークスペースのアダプタークレート（例: `suprnova-payments-stripe`） | 実装済み | Laravel パッケージと同じ形です: フレームワークに依存し、コンテナへバインドし、必要ならマクロを公開します |
| プロセス（シェルコマンドの実行） | stdlibの `tokio::process::Command` | 意図的に非対応 | ファサードはありません - Tokio のAPIが、すでに正しい形です |
| キュー | `Queue::push(job).await?` + ドライバー `sync/memory/database/redis/null`、バッチ、チェーン、`JobMiddleware`、`FailedJobStore` | 実装済み | [キュー](queues.md) |
| レート リミット | `RateLimiter::for_signature(...)`, `ThrottleRequestsMiddleware`, `RateLimitMiddleware` | 実装済み | `SlidingWindowConfig` によるスライディングウィンドウ。[レート リミット](rate-limiting.md) |
| 検索（Scout） | ファーストパーティの全文検索アダプターはありません | 未実装 | ベクトル検索は今日、[ベクトル](vector.md)経由で出荷されています。キーワード検索の Scout 相当品は計画中です |
| 文字列（ヘルパー） | `heck` クレート（大文字小文字変換）、`std::str`、`regex` | 差異あり | Rust エコシステムの他の部分が使うのと同じクレートです。グローバルな `Str::camel($x)` はありません |
| タスクスケジューリング | `Schedule::call/command/task` + `#[derive(Task)]` + cron構文 + `schedule:run` ワーカー | 実装済み | [スケジューリング](scheduling.md) |
| べき等キー | `Idempotency::remember(key, ttl, body)` - Stripeスタイルのリプレイ保護 | 実装済み | 呼び出し元は、ルート + ユーザー / ビジネスの identity でキーを名前空間化します。[べき等性](idempotency.md) |
| リクエストタイムアウト | ルートごとに設定可能な `TimeoutMiddleware` | 実装済み | Rustネイティブです - 実行中のfutureをアボートし、ワーカーを解放します。[タイムアウト](timeout.md) |
| フィーチャーフラグ（Pennant） | `Feature` + `Evaluator` + `FeatureMiddleware` + 管理者用CRUD | 実装済み | `FeatureSync` トレイトによる秒未満の伝播。[フィーチャー フラグ](feature-flags.md) |
| 可観測性（Pulse） | `init_telemetry`、`Metrics` によるOpenTelemetry、あらゆる場所での `tracing` | 差異あり | OTel はRustの可観測性における共通語です - あなたのコレクターをバイナリに向けてください。[可観測性](observability.md) |
| Telescope（デバッグダッシュボード） | まだ相当するものはありません | 未実装 | v2以降へ先送りされています。フレームワークの tracing + OTel 出力が、ほとんどの診断ニーズをカバーします |
| Pulse（パフォーマンスダッシュボード） | まだ相当するものはありません | 未実装 | Telescope と同様です - ダッシュボードが出荷されるまでは、既存の可観測性スタックでメトリクスを表面化してください |
| ベクトル検索 | `Vector::driver("memory"\|"qdrant"\|"pinecone"\|"mariadb")` | 実装済み | 「Postgres の pgvector のみ」というゲートキーピングはありません。[ベクトル検索](vector.md) |

### Suprnova 独自（Laravel に相当するものなし）

| Suprnova | 内容 | 備考 / リンク |
|---|---|---|
| `ws!()` マクロ + WebSocketハンドラ | ルーター + ミドルウェアスタックを共有する、型付きのWSルート | [WebSocket](websockets.md) |
| Server-Sent イベント | `SseEvent` + `HttpResponse::sse(...)` | [SSE](sse.md) |
| ワークフロー | リトライ、スリープ、ステップ境界を伴う、長時間実行のステートフルな作業 | [ワークフロー](workflows.md) |
| スーパーバイザー | 長命なtokioタスクのための、パニックキャッチによる自動再起動を備えた `Supervisor` トレイト | [スーパーバイザー](supervisors.md) |
| Web プッシュ（VAPID） | ファーストクラスのチャネルとしてのブラウザプッシュ通知 | [Web プッシュ](web-push.md) |
| マルチコネクションの読み書き分割 | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [データベース](database.md) |
| 同じソケット上のHTTP/2 + WebSocket | `Server::run` の中の `hyper.with_upgrades()` | [ライフサイクル](lifecycle.md) |
| Markdownコンテンツ + ドキュメントパイプライン | `MarkdownRenderer` (sanitised comrak → syntect → ammonia) + `build_docs(DocsBuildConfig)` → 検索可能な `DocsChapter` の `DocsCatalog` | 見出し抽出 + `slugify_heading`。別の静的サイトジェネレーターなしで、Markdownのドキュメント / ブログを動かします |

## セキュリティ

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| 認証 | `Auth::user/check/login/logout/attempt`、`Authenticatable` トレイト、名前ごとの `Guard` | 実装済み | [認証](authentication.md) |
| 複数のガード | `AuthManager` 経由で名前（`web`、`api`、…）によって登録される `Guard` | 実装済み | `SessionGuard`、`TokenGuard`、カスタム実装 |
| ユーザープロバイダー | `EloquentUserProvider<U>`、`DatabaseUserProvider`、`UserProvider` トレイト経由のカスタム | 実装済み | [認証フロー](auth-flows.md) |
| メール確認 | `EmailVerification` + `EnsureEmailVerifiedMiddleware` + `EmailVerificationMail`。ユーザーモデル上の `MustVerifyEmail` contract | 実装済み | プロバイダーバックエンドです（toriiは不要） - [認証フロー](auth-flows.md) |
| パスワードリセット | `PasswordReset` + `PasswordResetMail` + `PasswordChangedMail`。ユーザーモデル上の `CanResetPassword` contract | 実装済み | プロバイダーバックエンドです（toriiは不要） - [認証フロー](auth-flows.md) |
| ブルートフォース制限 | `BruteForce` + `LoginThrottleMiddleware` | 実装済み | IPごと + ユーザーごとのアカウンティング |
| 二要素認証（TOTP） | `TwoFactor` + `TwoFactorChallengeMiddleware` + `TwoFactorUser` トレイト | 実装済み | リカバリーコード + リプレイ保護 |
| ログイン状態の保持（remember-me） | `SessionGuard` による長命な署名付きクッキー | 実装済み | フレームワークが所有する `auth::remember`: DB行 + bcrypt + 使い捨てのローテーション |
| OAuth（Socialite） | ベンダリングされた `torii_integration` フォーク経由（Google / GitHub / Apple など） | 実装済み | [認証](authentication.md) |
| Sanctum（APIトークン） | `TokenGuard` + torii経由のDBバックエンドトークン | 差異あり | トークンモデル + bearerミドルウェアは出荷されます。独立した Sanctum のAPI表面はありません |
| Passport（OAuthサーバー） | まだありません | 未実装 | OAuthプロバイダーが必要な場合は、Suprnovaの背後で専用のIDサービス（Keycloak、Hydra）を動かしてください |
| Fortify（認証バックエンド） | `auth_flows` モジュール + `auth_flows::*` 型に置き換えられています | 実装済み | 同じ仕事です。フロントエンドが Inertia のため、ヘッドレス対ヘッドありの分裂は不要です |
| 認可（Policies / Gates） | `Gate::allows/denies` + `#[policy] impl PostPolicy` + `Authorizable` トレイト + マクロ登録 | 実装済み | [認可](authorization.md) |
| ロールと権限（spatie/laravel-permission） | `HasRoles` トレイト + `roles` / `permissions` / `role_has_permissions` テーブル（`CreateRbacTables`） + `RoleMiddleware` / `PermissionMiddleware`（フェイルクローズ） | 実装済み | コミュニティパッケージではなく、ファーストパーティです。`create_role` / `give_permission_to_role` / `assign_role_to_model` ヘルパーは、Gate/Policyの上に積み重なります。[認可](authorization.md) |
| 暗号化 | `Crypt::encrypt/decrypt` + `CryptPurpose` によるAADバインディング | 実装済み | AES-256-GCM、`APP_KEY_PREVIOUS` によるキーローテーション。[暗号化](encryption.md) |
| ハッシング | `hash::*` + `BcryptHasher`, `Argon2idHasher`, `Argon2iHasher`, `needs_rehash`, `is_hashed`, `verify` | 実装済み | デフォルトは Bcrypt、argon2id も利用可能です。[ハッシング](hashing.md) |

## データベース

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| DB::table('users')->where(...)->get() | `DB::table("users").db_where("id", "=", 1).get().await?` | 実装済み | [データベース](database.md)、[クエリ](queries.md) |
| 複数のコネクション | `DB::on("read")` + `ConnectionRegistry` | 実装済み | 読み書き分割がファーストクラスです |
| トランザクション | `DB::transaction(\|tx\| async move { ... }).await?` | 実装済み | セーブポイント + デッドロック時のリトライ |
| クエリイベント | `QueryListener` + `QueryExecuted` イベント | 実装済み | `DB::listen(\|q\| { ... })` |
| Raw式 | `DB::raw("...")`, `DB::select("...", &[...])` | 実装済み | パラメータバインドが必須です（文字列補間はありません） |
| Postgres / MySQL / SQLite | 3つともSeaORM経由でファーストクラスです | 実装済み | `database::config::database_type()` でのURL検出 |
| MariaDB | それ自体の選択肢としてファーストクラスです（vector + JSON + temporal） | 差異あり | Laravel が Postgres専用として出荷するマルチパラダイム機能のため、別扱いされています |
| Redis | ドライバー（cache/queue/rate-limit）から使われます - 独立した `Redis::*` ファサードはありません | 差異あり | アドホックなコマンドが必要なときは `redis` クレートに直接手を伸ばしてください。cache/queue/rate-limit が典型的な用途の95%をカバーします |
| MongoDB | まだファーストパーティのアダプターはありません | 未実装 | `App::bind` 経由で `mongodb` クレートを直接使ってください |
| クエリビルダー | `db_where` / `or_where` / `where_in` / `where_between` / `where_null` / `where_has` / `with` / `with_count` / `order_by` / `group_by` / `having` / `paginate` などを持つ `Builder<M>` | 実装済み | [クエリ](queries.md) |
| ページネーション | `LengthAwarePaginator`、`Paginator`（シンプル）、`CursorPaginator` | 実装済み | 3つとも Laravel の形にシリアライズされます。[ページネーション](pagination.md) |
| マイグレーション | `#[derive(DeriveMigrationName)] struct M;` + `up`/`down` + `Migrator` | 実装済み | `suprnova migrate`/`migrate:rollback`/`migrate:status`/`migrate:fresh` 経由で実行します。[マイグレーション](migrations.md)、[CLI マイグレーション](cli-migrations.md) |
| シーダー | `Seeder` トレイト + `db:seed` サブコマンド | 実装済み | モデルごとのファクトリー。[シーディング](seeding.md) |

## Eloquent ORM

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| `class User extends Model` | `#[suprnova::model(table = "users")] struct User { ... }` | 実装済み | この構造体自体が SeaORM の `Model` です。[Eloquent](eloquent.md) |
| 検索 / first / get | `User::find(id)`, `User::query().first()`, `User::all()`, `Builder::get` | 実装済み | すべて非同期です |
| 作成 / update / delete | `User::create(attrs)`, `user.update(attrs)`, `user.delete()` | 実装済み | 部分的な属性のための `attrs! { name: "...", email: "..." }` マクロ |
| マスアサインメントガード | `#[model(fillable = [...])]` / `#[model(guarded = [...])]` + `unguarded \|\| { ... }` スコープ | 実装済み | 厳格モードのための `prevent_silently_discarding_attributes()` |
| ソフトデリート | `#[model(soft_deletes)]` が `deleted_at` + `SoftDeletes` トレイトを自動注入します | 実装済み | `with_trashed()`、`only_trashed()`、`restore()`、`force_delete()` |
| Prunable / MassPrunable | `#[prunable] impl Prunable for User { ... }` + `model:prune` ワーカー | 実装済み | リレーションにカスケードで固定されます |
| タイムスタンプ | カラムが存在すれば `created_at`/`updated_at` を自動設定します | 実装済み | `#[model(timestamps = false)]` で無効化できます |
| 主キーの型 | デフォルトはi64。`#[model(unique_id = "uuid")]` または `unique_id = "ulid"` 経由でUUID / ULID | 実装済み | 挿入時にidを自動生成します |
| ローカルスコープ | `#[scopes(User)] impl User { fn active(b: &mut Builder<User>) { ... } }` | 実装済み | `Builder<M>` へのメソッドディスパッチ |
| グローバルスコープ | `impl GlobalScope for ActiveOnly { ... }` + 登録 | 実装済み | `Builder::without_global_scope` で取り除けます |
| リレーションシップ（11種類） | `HasOne`, `HasMany`, `BelongsTo`, `BelongsToMany`, `HasOneThrough`, `HasManyThrough`, `MorphOne`, `MorphMany`, `MorphTo`, `MorphToMany`, `MorphedByMany` | 実装済み | ファミリーごとの morph enum。[リレーションシップ](eloquent-relationships.md) |
| イーガーロード | `User::query().with(&["posts", "posts.comments"]).get()` | 実装済み | `EagerLoadDispatch` はシールされています。マクロが生成したリレーションだけがそれを実装できます |
| レイジーロードの防止 | `prevent_silently_discarding_attributes(true)` | 実装済み | Laravel の `preventLazyLoading` と同じ形です |
| リレーション上の集計 | `with_count("posts")`, `with_sum("orders", "total")`, `with_avg`, `with_min`, `with_max` | 実装済み | 集計ごとに単一のサブクエリです |
| `whereHas` / `whereDoesntHave` | `where_has("posts", \|q\| q.db_where("published", "=", true))` | 実装済み | 相関する EXISTS エンジン |
| `loadMissing` | `user.load_missing(&["posts"]).await?` | 実装済み | コレクション全体に対して動作します |
| レコードの複製 | `user.replicate()` / `user.replicate_into::<OtherType>()` | 実装済み | `Replicating` イベントをディスパッチします |
| 親タイムスタンプへのtouch | `#[model(touches = ["post"])]` | 実装済み | スキップするには `without_touching \|\| { ... }` |
| オブザーバー | `impl Observer<User>` + `#[suprnova::observer(User)]` | 実装済み | 16のライフサイクルイベント |
| 16個のライフサイクルイベント | `Created`, `Creating`, `Saving`, `Saved`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Trashed`, `Restoring`, `Restored`, `Retrieved`, `Replicating`, `ForceDeleting`, `ForceDeleted`, `Pruning` | 実装済み | モデルごとの `events::*` サブモジュール。`EventResult::cancel(_)` が400で短絡します |
| ミューテータ / アクセッサー | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, v: String)` | 実装済み | [ミューテータ](eloquent-mutators.md) |
| キャスト（22種類組み込み） | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | 実装済み | カスタムのためには `Cast` を実装してください |
| コレクション | `pluck`、`filter`、`map`、`each`、`chunk`、`groupBy`、`keyBy`、`sort_by`、`where_`、`first`、`last`、`count`、`is_empty`、`to_array` などLaravel系のメソッドを持つ `Collection<M>`。`Deref<Target = Vec<M>>` のため、あらゆる `Vec` のイディオムがそのまま動きます | 実装済み | [コレクション](eloquent-collections.md) |
| APIリソース | `#[derive(Resource)]` + `IntoJsonResource` + `JsonApiResponse` + フィールドセット + インクルード | 実装済み | JSON:API の形と Laravelスタイルのリソースの形、両方が利用できます。[API リソース](eloquent-resources.md) |
| シリアライゼーション | `#[model(hidden = [...], visible = [...], appends = [...])]` | 実装済み | どの属性がシリアライズされるかを、同じように制御できます。[シリアライゼーション](eloquent-serialization.md) |
| ファクトリー | `#[derive(Factory)] struct UserFactory` + `UserFactory::new().count(5).create().await?`（または `UserFactory::times(5).create_many().await?`） | 実装済み | 値を循環させる `Sequence`。[ファクトリー](eloquent-factories.md) |
| ライフサイクル: chunking / lazy / cursor | `Builder::chunk(n, \|page\| async { ... })`, `lazy()`, `cursor()` | 実装済み | 大きなテーブルに対する、メモリに上限のあるイテレーション |
| 悲観的ロック | `Builder::lock_for_update()`, `shared_lock()` | 実装済み | トランザクションの内側で |
| `whereJsonContains` ファミリー | SeaORMのカラム式（ドライバー依存）経由で利用できます | 実装済み | 正確な綴りはバックエンドごとに異なります。一般的なケース向けのヘルパーが出荷されています |

## ページネーション

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| `LengthAwarePaginator` | `LengthAwarePaginator`（page + total + per_page + last_page） | 実装済み | `Builder::paginate(n).await?` |
| `Paginator`（シンプル） | `Paginator`（page + per_page + has_more、カウントなし） | 実装済み | `Builder::simple_paginate(n).await?` |
| `CursorPaginator` | `CursorPaginator`（不透明なカーソルトークン + 方向） | 実装済み | `Builder::cursor_paginate(n).await?`。無限スクロールに対して決定的です |
| Inertia統合 | `IntoInertiaScroll` trait + `ScrollMetadata` | 実装済み | Inertia の `WhenVisible` / `merge` へ直接配線されます |

## AI（Laravel は今日すでにネイティブで出荷しています。こちらはゲートキーピングしません）

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| AI SDK | ファーストパーティのAI SDKはありません | 意図的に非対応 | すでに使っているクレート（`async-openai`、`anthropic-sdk`、`ollama-rs`、`tokenizers` など）を持ち込んで、`App` の下にバインドしてください |
| MCP（Model Context Protocol） | ファーストパーティのMCPサーバーアダプターはありません | 意図的に非対応 | Rust の MCP クレート（`mcp-rs`、`mcp-sdk-rust`）は、既存のルーティング / スーパーバイザーの表面の下にきれいに収まります |
| Boost（Laravelのコーディングエージェント） | 該当なし | 意図的に非対応 | フレームワークのスコープ外です |

## テスト

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| `php artisan test` | `cargo test` | 実装済み | [テスト](testing.md) |
| Pest / PHPUnit スタイル | `#[suprnova_test]`（非同期対応） + Jestのような `expect!()` アサーション + `describe!()` / `test!()` BDDマクロ | 実装済み | 3つとも互換的に動きます |
| フィーチャーテスト（HTTP） | `handle_request(router, registry, req)` をプロセス内で駆動します - ソケットは開きません | 実装済み | [HTTPテスト](http-tests.md) |
| コンソールテスト | `dispatch_argv(["console", "..."])` を実行してアサートします | 実装済み | コンソールバイナリに対しても、HTTPテストと同じ形です |
| ブラウザテスト（Dusk） | フレームワークには該当なし - Playwright / WebdriverIO / `gstack` agent browser を使ってください | 意図的に非対応 | 言語をまたぐツールがすでに存在します。私たちはそれを再発明しません |
| データベーステスト | `TestDatabase::fresh::<Migrator>()` + テストごとのロールバック | 実装済み | [データベース テスト](database-testing.md) |
| モックとフェイク | ファサードごとのフェイク: `MailFake`、`NotifyFakeGuard`、`EventFakeGuard`、`Queue::fake`、`Bus::fake`、`Http::fake`、`Storage::fake` | 実装済み | 記録された呼び出し + アサーションヘルパー。[モック](mocking.md) |
| タイムトラベル | stdlibランタイムの `tokio::time::{pause, advance, resume}` | 実装済み | 独自のものは出荷しません - Tokio のAPIがすでにそれを行います |
| コンテナの分離 | `TestContainer::fake(\|tc\| tc.bind(...))` - スレッドローカル | 差異あり | 構造上、並列に対して安全です。[コンテナ](container.md) |

## 支払い（Laravel の Cashier；こちらはプロバイダー汎用です）

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| Cashier（Stripe） | 汎用の `Payment` / `Subscription` / `CustomerStore` / `WebhookHandler` トレイトの背後にある `suprnova-payments-stripe` アダプタークレート | 差異あり | 汎用の表面、具体的なアダプター。[支払い](payments.md)、[Stripe アダプター](payments-stripe.md) |
| Cashier（Paddle） | `suprnova-payments-paddle` アダプター | 差異あり | Merchant-of-Record フロー、直接の `Payment` 実装はありません（Paddle がゲートウェイを所有します）。[Paddle アダプター](payments-paddle.md) |
| カスタムプロバイダー | `PaymentProvider` + `SessionPayload` + `WebhookHandler` を実装します | 実装済み | [プロバイダーガイド](payments-provider-guide.md) |
| Inertiaのチェックアウトコンポーネント | `SessionPayload.flow` に対する、Svelte / React / Vue向けのドキュメント化されたディスパッチループ | 実装済み | [支払い フロントエンド](payments-frontend.md)。作り込み済みの請求ページは、計画中のスターターキットへの追加です（[スターターキット](starter-kits.md)） |
| サブスクリプションのライフサイクル | `Subscription::subscribe / update / cancel / get`（プロバイダーがサポートする場合） | 実装済み | プロバイダーがサポートしない箇所では `NotSupported` が返されます（例: Paddle の `subscribe` と価格セットの差し替え） |
| Webhookのべき等性 | `UNIQUE(provider, provider_event_id)` を持つ `payments_webhook_events` ミラーテーブル | 実装済み | Stripe形式のリプレイ保護 |
| ミラーテーブル | `payments_customers`, `payments_payment_methods`, `payments_subscriptions`, `payments_subscription_items`, `payments_transactions`, `payments_webhook_events` | 実装済み | アダプター固有のフィールドのため、それぞれに `provider_metadata` JSONBカラムがあります |

## フロントエンド（Laravel には Blade とスターターキットがあり、こちらには Inertia があります）

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| Blade | 該当なし - Inertia がビュー層です | 差異あり | [フロントエンド](frontend.md) |
| Inertia.js | ファーストクラス: Svelte 5 / React 19 / Vue 3.5 の上の v3 | 実装済み | [Inertia レスポンス](frontend-inertia-responses.md)、[ページ](frontend-pages.md) |
| 部分リロード | `#[derive(Data)]` + `req.includes("subset")` + Inertiaの部分リロードプロトコル | 実装済み | 型安全な include セット |
| 遅延 props | `Prop::deferred(...)` + `DeferConfig` | 実装済み | Inertia v3 の deferred-props プロトコル |
| マージ props | `MergeConfig` + `MergeStrategy::{Append, Prepend, Replace}` | 実装済み | Inertia v3 の merge プロトコル |
| 履歴の暗号化 | `EncryptHistoryMiddleware` | 実装済み | 履歴はクライアント上で保存時に暗号化されます |
| スクロール位置 | `ScrollConfig` + `ScrollMetadata` | 実装済み | ナビゲーション時に自動復元されます |
| TypeScript型 | `suprnova generate-types` が `#[derive(InertiaProps)]` を読み取り、`.d.ts` を出力します | 実装済み | [TypeScript 型](frontend-typescript-types.md) |
| Viteマニフェストの読み取り | `Inertia::root_view` 経由で自動的に配線されます | 実装済み | 開発ではHMR、本番ではハッシュ化されたアセットです |

## CLI

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| `php artisan` | `#[command]` マクロから構築される、アプリごとの `console` バイナリ | 実装済み | [コンソール](console.md)、[CLI 概要](cli.md) |
| `make:controller` / `make:model` / etc. | `suprnova make:controller / make:middleware / make:action / make:error / make:inertia / make:migration / make:task` | 実装済み | [ジェネレーター](cli-generators.md) |
| `serve` | `suprnova serve`（バックエンド + Vite開発サーバーを一緒に） | 実装済み | [起動](cli-serve.md) |
| `migrate` ファミリー | `suprnova migrate / migrate:rollback / migrate:status / migrate:fresh` | 実装済み | [CLI マイグレーション](cli-migrations.md) |
| `db:seed` | `cargo run --bin console db:seed`（アプリごとのconsole経由） | 実装済み | `Seeder` トレイト経由で登録されるシーダー |
| `schedule:run` / `schedule:work` / `schedule:list` | アプリごとのコンソールバイナリを介した同じ名前 | 実装済み | [スケジューリング コマンド](cli-scheduling.md) |
| `queue:work` | アプリごとのコンソールバイナリを介した同じ名前 | 実装済み | SIGTERM/SIGINT でのグレースフルシャットダウン |
| `tinker` | REPLはありません | 意図的に非対応 | 「さらに掘り下げる」の行を参照してください |

## デプロイメント

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| `php artisan optimize` | `cargo build --release` | 差異あり | 1つのバイナリ、opcacheのステップはありません |
| `php artisan config:cache` | 型付き設定はすでにコンパイル時にチェック済みです | 差異あり | 無効化すべきランタイムキャッシュはありません |
| `php artisan route:cache` | ルートはコンパイル時にマクロ展開されます | 差異あり | ルーターは、すでに型付けされたルートから起動時に構築されます |
| Envoy（SSHデプロイ） | 任意のオーケストレーターを使ってください - Docker、systemd、Kubernetes、fly.io、Railway | 意図的に非対応 | バイナリがデプロイのアーティファクトです |
| Forge / Vapor | 私たちが出荷するものではありません - ですがRailway、DO、Hetznerのレシピが同じ仕事をカバーします | 差異あり | [デプロイメント](deployment.md)、[Railway](deployment-railway.md)、[Digital Ocean](deployment-digital-ocean.md)、[Hetzner](deployment-hetzner.md) |
| Horizon（キューダッシュボード） | まだダッシュボードはありません | 未実装 | それまでは `cargo run --bin console queue:failed` による失敗ジョブの調査です |

## パッケージ（Laravel の公式パッケージ - こちらはコアで出荷するか、アダプターとして出荷するか、意図的なギャップのいずれかです）

| Laravelパッケージ | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| Cashier（Stripe） | `suprnova-payments-stripe` | 実装済み | 汎用 + アダプター。[支払い](payments.md) |
| Cashier（Paddle） | `suprnova-payments-paddle` | 実装済み | MoRフロー。[支払い](payments.md) |
| Dusk | 該当なし | 意図的に非対応 | 言語をまたぐブラウザツール（Playwright など）がすでに存在します |
| Envoy | 該当なし | 意図的に非対応 | コンテナ / systemd / オーケストレーターが仕事をします |
| Fortify | `auth_flows` に置き換えられています | 実装済み | 同じ仕事が統合されています。[認証フロー](auth-flows.md) |
| Folio | 該当なし - ページベースのルーティングはRustらしくありません | 意図的に非対応 | 明示的なルーティングには `routes!` を使ってください |
| Homestead | 該当なし - Docker / DevContainers を使ってください | 意図的に非対応 | [Docker レシピ](cli-docker.md) |
| Horizon | まだ該当なし | 未実装 | 失敗したジョブは、アプリごとのコンソールから確認できます |
| Mix | Viteに置き換えられています | 差異あり | Vite があらゆるスキャフォルドに出荷されています |
| Octane | 該当なし - すでに長命なTokioです | 意図的に非対応 | 単一バイナリ、常に温まっており、入れ替えるFPMはありません |
| Passport | まだ該当なし | 未実装 | 出荷されるまでは、Suprnovaの背後で専用のIdPを動かしてください |
| Pennant（フィーチャーフラグ） | `features::*` として再実装されています | 実装済み | [フィーチャー フラグ](feature-flags.md) |
| Pint（PHPコードスタイル） | `cargo fmt` + `cargo clippy` | 差異あり | 標準のRustツールチェーンです |
| Precognition | 部分リロード経由のInertia precognitiveリクエスト + 同じ `#[derive(Data, Validate, FormRequest)]` 型 | 実装済み | Precog の2つの半分（早期バリデーション + 軽量なリロード）は、どちらも Inertia v3 + form request から自然に得られます |
| Prompts（CLI UI） | 必要なときは `dialoguer` / `inquire` クレートを使ってください | 意図的に非対応 | Rustエコシステムがすでにこれをカバーしています |
| Pulse | まだ該当なし | 未実装 | 今日はOTel、ダッシュボードは後日です |
| Reverb（WebSocketサーバー） | Suprnova に組み込み（`ws!()` + `BroadcastHub`） | 差異あり | 別サーバーは不要です - 同じプロセスです |
| Sail（Docker開発） | `suprnova-cli` がDockerのレシピをインラインで出荷します | 実装済み | [CLI Docker](cli-docker.md) |
| Sanctum | `TokenGuard` + bearerミドルウェア | 差異あり | トークンモデルは出荷されます。独立したパッケージの表面はありません |
| Scout（全文検索） | まだ該当なし | 未実装 | ベクトル検索は出荷されています（[ベクトル](vector.md)）。キーワード版の Scout 相当品は後日です |
| Socialite | ベンダリングされた torii フォーク経由 | 実装済み | [認証](authentication.md) |
| Telescope | まだ該当なし | 未実装 | ダッシュボードが出荷されるまでは、Tracing + OTel が診断のギャップをカバーします |
| Valet | 該当なし - Rustアプリは直接実行されます | 意図的に非対応 | `suprnova serve` が開発ランナーです |

## マクロ（Rust固有の表面；文脈のための最も近い Laravel の類似物）

Suprnova は、Laravel に相当するものがない幅広いプロシージャルマクロ群を出荷しています。Laravel にはマクロがなく、代わりにランタイムのリフレクションを持っているからです。見逃さないよう、ここに含めています。

| マクロ | 最も近いLaravelの発想 | 何をするか |
|---|---|---|
| `#[suprnova::model]` | `extends Model` | SeaORMのエンティティを生成し、`Model` トレイトを実装します |
| `#[suprnova::observer(M)]` | `User::observe(UserObserver::class)` | `inventory` 経由で `Observer<M>` の実装を登録します |
| `#[scopes(M)]` | モデルのローカルスコープ | `Builder<M>` へメソッドを追加します |
| `#[accessor]` / `#[mutator]` | Eloquentのアクセッサー / ミューテータ | フィールド単位のget/setフック |
| `#[handler]` | コントローラーの `__invoke` | `Request` から型付きのパラメータを自動抽出します |
| `#[command]` / `#[derive(Command)]` | Artisanのコマンドクラス | コンソールのサブコマンドを登録します |
| `#[policy]` | ポリシークラス | `inventory` 経由で `Policy` の実装を登録します |
| `#[service(T)]` | サービスプロバイダーの `register` | `T` をコンテナへバインドします |
| `#[injectable]` | コンストラクタインジェクション | `App::make` に支えられたコンストラクタを生成します |
| `#[derive(InertiaProps)]` | Inertiaのprops | TypeScriptのコード生成 + Inertiaのシリアライゼーション |
| `#[derive(Data)]` | リクエストDTO | include-setをサポートし、`Request` から抽出可能です |
| `#[derive(FormRequest)]` | `FormRequest` クラス | バリデーション + 認可ゲート + 変換 |
| `#[derive(Factory)]` | モデルファクトリー | Fakerに支えられたテストデータ生成 |
| `#[derive(Resource)]` | APIリソース | JSON:API + Laravel形式のシリアライゼーション |
| `#[workflow]` / `#[workflow_step]` | Laravelには該当なし | 長時間実行のステートフルな作業 |
| `routes!` + `get!` / `post!` / `ws!` など | `Route::get` / `Route::post` | コンパイル時のルート登録 |
| `casts!` | `protected $casts = [...]` | モデルごとのキャスト宣言 |
| `attrs!` | マスアサインメント用の配列 | 部分的な属性のビルダー |
| `json_response!` / `text_response!` | `response()->json(...)` | 素早い `Ok(HttpResponse::...)` |

完全なリファレンスについては、[マクロ](macros.md)を参照してください。

## ヘルパー関数（Laravel のグローバルヘルパー；こちらは型付きです）

Laravel は、何百もの小さなグローバル関数（`str_replace_first`、`array_flatten`、`now()`、`tap()`、`optional()` …）を出荷しています。そのほとんどは `std` か小さな標準クレートの中に直接対応するRustの等価物を持っているため、Suprnova はそれらを単一の名前空間として再導入していません。エイリアスとして持つ価値が*本当に*あるものは、それぞれのホームモジュールの下で出荷されています。

| Laravelのヘルパー | Suprnova / Rustの等価物 | どこで |
|---|---|---|
| `auth()` | `Auth::user().await?` | [認証](authentication.md) |
| `cache()` | `Cache::get/put/...` | [キャッシュ](cache.md) |
| `config('app.name')` | `Config::get::<AppConfig>()?.name` | [設定](configuration.md) |
| `csrf_token()` | `csrf_token()` (same name) | [CSRF](csrf.md) |
| `dd()` | `Builder::dd()`（Eloquentのクエリdump-and-die） / stdlibの `dbg!()` | `Builder::dump()` / `Builder::dd()` はクエリ調査のために存在します。一般的な値には `dbg!()` を使ってください |
| `env('APP_KEY')` | `env("APP_KEY")` / `env_required("APP_KEY")` / `env_optional("APP_KEY")` | [設定](configuration.md)、[環境変数](env-vars.md) |
| `now()` | `chrono::Utc::now()`（`suprnova::chrono` として再エクスポート） | - |
| `optional($x)->y` | `x.as_ref().map(\|x\| x.y)` | Rust はこれを `Option<T>` で直接扱います |
| `redirect('/')` | `redirect("/")` (same name) | [ルーティング](routing.md) |
| `request()` | `Request` はハンドラへ渡されます | [リクエスト](requests.md) |
| `response()` | `HttpResponse::json/text/redirect/...` | [レスポンス](responses.md) |
| `route('posts.show', ['post' => 1])` | `url("posts.show", &[("post", "1")])` | [URL 生成](urls.md) |
| `session('key')` | `session().get("key")` | [セッション](session.md) |
| `str()` / `Str::camel($x)` | `heck` クレートのメソッド（`ToUpperCamelCase` など） | - |
| `tap($x, fn) → $x` | `tap` クレートの `tap`、または素早い確認のための `dbg!` | `tap` クレートをイディオムどおりに使ってください |
| `today()` | `chrono::Utc::now().date_naive()` | - |
| `value($x)` | クロージャを呼ぶだけです: `x()` | 該当なし - Rustのクロージャにヘルパーは不要です |
| `view('home', $data)` | Inertiaレスポンス: `Inertia::render("Home", data)` | [Inertia レスポンス](frontend-inertia-responses.md) |

## 私たちが本当にまだ持っていないもの

上記のすべての **未実装** を1か所にまとめたリストです。ギャップの形が一目でわかります:

| 領域 | 何が欠けているか | 出荷されるまでの回避策 |
|---|---|---|
| 検索（Scout - キーワード） | Algolia / Meilisearch / Elastic アダプター | 出荷されるまでは `meilisearch-sdk` / `elasticsearch` で自作してください。[ベクトル](vector.md)が今日、セマンティック検索を扱います |
| Passport（OAuthサーバー） | ファーストパーティのOAuth IDプロバイダー | Suprnovaの背後で Hydra / Keycloak を動かしてください |
| Telescope（デバッグダッシュボード） | リクエスト / クエリ / イベント / キャッシュヒットのためのWeb UI | OTel + tracing の出力を使ってください（[可観測性](observability.md)） |
| Pulse（パフォーマンスダッシュボード） | 遅いクエリ / エラー / ホットなルートのためのWeb UI | 同様です: 今日はOTelの表面、ダッシュボードは後日です |
| Horizon（キューダッシュボード） | キューの深さ / 失敗したジョブ / スループットのためのWeb UI | `cargo run --bin console queue:failed` とOTelのメトリクス |

## 私たちが出荷しないもの（そしてその理由）

| Laravelの機能 | Suprnovaがそれを持たない理由 |
|---|---|
| Tinker（REPL） | Rustには、コンパイル済みバイナリに対する生産的なREPLの物語がありません。短い `#[suprnova_test]` か、使い捨ての `cargo run --bin <thing>` スクリプトで仕事は済みます |
| Blade テンプレート | Inertia がビュー層です。並行するサーバーサイドレンダリングのテンプレートエンジンは出荷しません |
| `helpers.md` のような何でも入り | Rust は `std` + 小さな焦点の絞られたクレート（`heck`、`chrono`、`regex`）を出荷します。単一のグローバルな名前空間は再導入しません |
| Mix | Vite がそれをカバーし、あらゆるスキャフォルドに出荷されています |
| Octane | Suprnova はすでに長命な Tokio です。最適化して取り除くべきFPMモードはありません |
| Dusk（ブラウザテスト） | 言語をまたぐツール（Playwright、WebdriverIO、`gstack` agent browser）がすでにこれを解決しています |
| Sail（Docker開発） | Docker レシピがインラインで出荷されます（[CLI Docker](cli-docker.md)）。別パッケージは不要です |
| Valet | `suprnova serve` が開発サーバーです |
| Envoy（SSHデプロイ） | コンテナ / systemd / オーケストレーターが仕事をします。専用のSSH DSLは不要です |
| Concurrencyファサード（`Concurrency::run`） | Tokio（`tokio::join!` / `tokio::spawn` / `tokio::select!`）がその答えです。ファサードは不要です |
| Processesファサード | `tokio::process::Command` が、すでに正しい形です |
| ファーストパーティのAI SDK / MCP / Boost | すでに使っているRustのクレートを選んでください。私たちはゲートキーピングしません |
| 専用のRedisファサード | cache/queue/rate-limit が典型的な用途の95%をカバーします。アドホックなコマンドが必要なときは `redis` クレートに手を伸ばしてください |
| Stringsファサード | `heck`、`regex`、`std::str` がそれをカバーします。グローバルな `Str::camel($x)` はありません |
| Prompts（CLI UIライブラリ） | `dialoguer` / `inquire` がすでに存在します。私たちは再発明しません |
| Laravelスタイルの PHP/JSON 翻訳ファイル | ローカライゼーションは出荷されますが、カタログの形式は Fluent の `.ftl` です - サーバーとブラウザの両方が解析する、1つの形式です。`trans_choice` にも相当するものはありません: Fluent はメッセージの内側でCLDRの複数形カテゴリを選択します。[ローカライゼーション](localization.md) |

## このリストが正直であり続ける方法

**実装済み** 列のすべての行は、次の方法で検証できます:

1. `framework/src/lib.rs` を、名指しされたエクスポートについて grep する
2. フレームワークのテストスイート（`cargo test --workspace`）を実行する
3. リンクされたチャプターを読む

**未実装** 列のすべての行は、拒否ではなく意図された作業です。**意図的に非対応** 列のすべての行には、備考列に一文の理由があります。それらの理由は、[はじめに](introduction.md)にある設計原則を、特定の機能に適用したものです。

このマップに載っていない、あなたが使いたい Laravel の機能を見つけたら、issue を開いてください - それには行が欠けている Suprnova の答えがあるか、本物のギャップであり、私たちはそれを知りたいのです。

## 次のステップ

- [Laravel から](from-laravel.md) - 同じマップを、並列比較として語り直したもの
- [はじめに](introduction.md) - このパリティ作業が従う設計原則
- [`documentation.md`](documentation.md) - 全チャプターにわたるマスターTOC
