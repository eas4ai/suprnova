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
| リクエストライフサイクル | `Application` → `Server` → `handle_request` のチェーン | 実装済み | [リクエスト ライフサイクル](lifecycle.md) |
| サービスコンテナ | `Container` + `App` ファサード。3層（タスク / スレッド / グローバル） | 差異あり | リクエストごとにはタスクローカル、テストにはスレッドローカルです - [サービス コンテナ](container.md) |
| 文脈依存のバインディング（`when()->needs()->give()`） | 文脈依存のバインディングはありません - コンテナの層ごと、トレイトごとに1つのバインディングです | 意図的に非対応 | コンテナは `TypeId` をキーとしており、「誰が尋ねているのか」でバインディングをキー付けするための実行時リフレクションがありません。明示的に合成してください。依存を渡すか、利用者ごとに別々のニュータイプをバインドします。[サービス コンテナ](container.md) |
| サービスプロバイダー | `bootstrap()` 関数 + `#[service]`、`#[policy]`、`#[command]`、オブザーバーのマクロ | 差異あり | 登録用のクラスはありません - bootstrap は1つの関数です。マクロは、コンパイル時の登録に `inventory` を使います。[アプリケーション ブートストラップ](bootstrap.md) |
| ファサード | 静的な `App::get`、`Cache::*`、`Mail::*`、`Auth::*`、`Storage::*`、`Queue::*`、`Bus::*`、`Event::*`、`Notification::*`、`Gate::*`、`Schedule::*`、`DB::*`、`Vector::*` | 実装済み | 呼び出しの形は同じです。ファサードはエイリアスではなく、本物の型です |
| コントラクト | トレイト - `Mailer`、`KeyValueStore`、`Hasher`、`Channel`、`VectorDriver`、`Evaluator`、`PaymentProvider` など | 実装済み | すべての公開の継ぎ目はトレイトの上にあります。トレイトでバインドし、実装は自由に差し替えてください |

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
| ルートパラメータ | `{id}` のパスパラメータ + `req.param("id")` | 実装済み | 任意のパラメータは `{id?}` 経由、制約は `where!()` 経由です |
| ルート名 | ルート上の `.name("posts.show")` + `url("posts.show", &[("id", "42")])` | 実装済み | [URL 生成](urls.md) |
| ルートグループ | `.prefix()` / `.middleware()` / `.name()` / `.controller()` を伴う `group!` マクロ | 実装済み | グループのミドルウェアは、登録時に各ルートへ平坦化されます |
| リソースルート | `resource!("posts", PostController)` が7つの標準ルートを登録します | 実装済み | `apiResource!`、`only(...)`、`except(...)` はすべてサポートされています |
| 署名付きURL | `sign_url(...)`、`sign_route(...)`、`verify_signature(...)` | 実装済み | `APP_KEY` を用いたHMAC-SHA256です |
| ルートモデルバインディング | `#[handler]` が、`RouteBinding` の実装を介して `{post}` から `Post` を抽出します | 実装済み | `AutoRouteBinding` のderiveが、`#[suprnova::model]` の型に対して自動で実装します |
| レート制限 | `throttle:60,1` ミドルウェア + `RateLimiter::for_signature` | 実装済み | [レート リミット](rate-limiting.md) |
| ミドルウェア | `impl Middleware` トレイト。グローバルにも、ルートごとにも登録できます | 実装済み | [ミドルウェア](middleware.md) |
| ミドルウェアのグループ + エイリアス | `register_middleware_group`、`register_middleware_alias` | 実装済み | ルートの中では文字列の名前で参照します |
| CSRF保護 | `CsrfMiddleware` + `csrf_token()` / `csrf_field()` / `csrf_meta_tag()` | 実装済み | セッションごとのトークン検証がデフォルトです。オプションの `SameOriginOnly`、`AllowSameSite`、`OriginOnly` ポリシーは `Sec-Fetch-Site` を参照します。オリジンの強制はデフォルトで有効ではありません。[CSRF](csrf.md) |
| コントローラー | `#[handler] pub async fn show(req: Request) -> Response` | 実装済み | コントローラーはクラスではなく、自由関数のモジュールです。[コントローラー](controllers.md) |
| シングルアクションコントローラー | ハンドラはすでに単一の関数です。モジュールにまとめてください | 実装済み | Rustの慣例です - `__invoke` の儀式はありません |
| リクエスト | `.input()`、`.param()`、`.query()`、`.header()`、`.cookie()`、`.json()`、`.file()` などを持つ `Request` 構造体 | 実装済み | [リクエスト](requests.md) |
| フォームリクエスト | `#[derive(Data, Validate, FormRequest)]` | 実装済み | 抽出と同時にバリデーションが走ります |
| ファイルアップロード | `req.file("avatar")?` が `UploadedFile` を返します。サイズとパート数の上限を伴うストリーミングmultipartです | 実装済み | しきい値を超えると自動的にテンポラリファイルへ退避します |
| レスポンス | `HttpResponse` のビルダー + `json_response!()` / `text_response!()` / `Redirect::to` / Inertiaレスポンス | 実装済み | [レスポンス](responses.md) |
| ストリーミングレスポンス（`eventStream`、`stream`、`streamJson`） | `HttpResponse::sse(...)` / `event_stream(...)` / `stream_bytes(...)` / `stream_json(...)` | 実装済み | `@laravel/stream-{react,vue,svelte}` のフックが期待するのと同じレスポンス形状です。[SSE](sse.md) |
| `withoutCookie` / `withoutCookies` | `HttpResponse`、`Response`、`Redirect`、`RedirectRouteBuilder` 上の `.without_cookie(name)` / `.without_cookies([...])` | 実装済み | `/` で設定されていないクッキーには `Cookie::forget_with(name, path, domain)` |
| ビュー（Blade） | サーバーレンダリングされるInertiaのページ（Svelte/React/Vue） - Blade相当はありません | 差異あり | Inertiaがビュー層です。Bladeの代わりに[ページ](frontend-pages.md)を使ってください |
| アセットのバンドル（Vite） | Vite 8 があらゆるスキャフォルドに出荷されます。`suprnova serve` がViteとバックエンドを一緒に走らせます | 実装済み | マニフェストの読み取りとHMRが自動で配線されます |
| 静的アセット（Laravelでは `public/` をWebサーバーが配信します） | `public/` をWebのルートで配信する、プロセス内のフォールバックハンドラ `StaticFiles::public()` | 実装済み | `StaticFiles::from_dir(...)` + `cache_control(...)`。別途Webサーバーは必要ありません |
| URL生成 | `url("posts.show", &[…])`、`route("posts.show", …)`、`redirect(...)`、`redirect_to(...)` | 実装済み | [URL 生成](urls.md) |
| セッション | `session()`、`session_mut()`、`req.flash()` 経由のフラッシュバッグ | 実装済み | デフォルトでは `DatabaseSessionDriver` によるデータベースバックエンドです。暗号化されたブラウザークッキーが運ぶのはセッション識別子とアクティビティタッチのメタデータだけであり、セッションデータバッグではありません。[セッション](session.md) |
| クッキーのキュー（`Cookie::queue`） | `Cookie::queue`/`queued`/`unqueue`/`expire` - `SessionMiddleware` がレスポンスへドレインするタスクローカルのジャー | 実装済み | チェーンに `SessionMiddleware` が必要です。Laravelの `CookieJar` のように名前+パスではなく、名前でキューに入ります |
| バリデーション | `#[derive(Validate)]` + 27個の組み込みルール + `Rule`/`ValueRule`/`AsyncRule` トレイト | 実装済み | `Url` はLaravelのスキームの許可リストを使い、`Url::protocols([...])` は `url:http,https` をミラーします。非同期のルール（例えば `Unique`）はDBを叩きます。`ArrayKeys`/`Distinct` は `serde_json::Value` 上の `ValueRule` であり、Laravelの `array:keys` と `distinct` に対応します。[バリデーション](validation.md) |
| `Password` ルール（`Password::defaults()`、`uncompromised()`） | パスワード強度のルールファミリーはありません。`Min`、`Regex`、そしてカスタムの `Rule` を組み合わせてください | 未実装 | Have I Been Pwned の `uncompromised()` チェックを含みますが、これに相当するものは今日ありません |
| エラーハンドリング | `FrameworkError`、`AppError`、`HttpError` トレイト、`execute_chain_safely` のパニック境界 | 実装済み | [エラーハンドリング](errors.md)、[エラー モデル](error-model.md) |
| ロギング | 構造化されたフィールドを持つ `tracing` のサブスクライバー、`LogFormat`（json / pretty / compact） | 差異あり | 1つのログ行が1つのJSONドキュメントです。`request_id` は常に存在します。[ロギング](logging.md) |
| ログチャネル / ファイルドライバー（`single`、`daily`、`monthly`、`stack`） | `tracing` が構造化された行を標準出力へ書き出し、プラットフォームがそれをローテートして送り出します | 意図的に非対応 | コンテナ、systemd、そしてあらゆるログシッパーが、すでにローテーションと保持を行っています。それをプロセス内で再実装することは、プラットフォームを重複させ、そこからログを隠してしまいます。[ロギング](logging.md) |
| abortのヘルパー | `abort_if(cond, status, msg)`、`abort_unless(...)`、`abort_with(status, msg)` | 実装済み | Laravelの `abort_if` ファミリーと同じ形です |

## さらに掘り下げる

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| Artisanコンソール | `#[command]` + `#[derive(Command)]` から構築される、アプリごとの `console` バイナリ | 実装済み | [コンソール](console.md)。`cargo run --bin console <subcommand>` |
| Tinker（REPL） | REPLはありません | 意図的に非対応 | 使い捨ての `cargo run --bin xxx` スクリプトか、`#[suprnova_test]` を書いてください |
| ブロードキャスト | `BroadcastHub` + `Channel` / `PrivateChannel` / `PresenceChannel` + `Broadcastable` | 実装済み | マルチノードのための sea-streamer ファンアウトです。[ブロードキャスト](broadcasting.md) |
| キャッシュ | `Cache::get/put/forget/remember/rememberForever/increment/...` + `InMemoryCache`、`RedisCache` | 実装済み | アトミック操作 + タグ付きキャッシュ + キャッシュロック（`LockGuard`）です。[キャッシュ](cache.md) |
| コレクション | Laravel形のメソッドを備えた `eloquent::Collection<M>` | 実装済み | `Deref<Target = Vec<M>>` であるため、既存のVecのイディオムもそのまま動きます。[コレクション](eloquent-collections.md) |
| 並行性 | あらゆる場所でTokio - `tokio::spawn`、`tokio::join!`、`tokio::select!` | 実装済み | フレームワーク全体が非同期です。Laravelの `Concurrency::run([...])` ファサードは出荷しません。Tokioが答えです |
| コンテキスト | `Context::put` / `Context::get` / `ContextStore` + キュー / メール / イベントへの自動注入 | 実装済み | [コンテキスト](context.md) |
| コントラクト | すべての公開の継ぎ目はトレイトです | 実装済み | 上の「アーキテクチャ / コントラクト」の行を参照してください |
| イベント | `EventFacade::dispatch(e).await?`、`#[derive(Event)]`、`EventDispatcher`、キューに入れられるリスナー、サブスクライバー | 実装済み | [イベント](events.md) |
| ファイルストレージ | OpenDAL の上の `Storage::disk("local"\|"s3"\|"azblob"\|"gcs"\|"memory")` | 実装済み | 同じ `put/get/delete/copy/move/exists/url` の表面です。パストラバーサル保護が組み込まれています。[ファイルシステム](filesystem.md) |
| ヘルパー | 相当するものは、それぞれの本拠地のモジュールにあります（何でも入りの `helpers.md` はありません） | 差異あり | 例えば、URLのヘルパーは[urls.md](urls.md)に、文字列のヘルパーは `std`/`heck` に、配列のヘルパーは `std::collections` にあります - Rustはこれを、グローバルな名前空間ではなくクレートで行います |
| HTTPクライアント | `Http::get/post/...` のビルダー + テスト用の `Http::fake(...)` | 実装済み | リクエストを自動記録します。`assert_sent` / `assert_not_sent`。組み込みリトライポリシーを `RetryContext` で狭める `.retry_when(predicate)` もあります。[HTTP クライアント](http-client.md) |
| 画像（`Illuminate\Image`） | 画像処理の表面はありません | 未実装 | `image` クレートの上の `ImageDriver` トレイト（リサイズ / クロップ / 変換 / 主要色）が計画中です。それが出荷されるまでは、`image` クレートを直接使ってください |
| ローカライゼーション | `lang/<locale>/` の Fluent `.ftl` カタログの上の `Lang::get` / `get_with` / `try_get` / `has` と `__!("key", name: value)` マクロ、`LocaleMiddleware` による検出、翻訳されたバリデーションメッセージ、ICU4Xによるフォーマット | 実装済み | 同じカタログが `/_suprnova/lang/<locale>.ftl` でブラウザへ配信され、`generate-types` によって型付けされます。[ローカライゼーション](localization.md) |
| メール | `Mail::to(...).send(MyMail { ... }).await?` + ドライバー `smtp/ses/mailgun/postmark/sendgrid/resend/log/memory/file` | 実装済み | `Mailable` トレイト + TeraでレンダリングされるHTML/text本文。SES送信は `TenantName` / `ConfigurationSetName` / `ListManagementOptions` を運び、キューに入れられたディスパッチは `.on_queue(...)` / `.on_connection(...)` を通じてルーティングされ、`Queue::route` より優先されます。[メール](mail.md) |
| 通知 | `Notify::send(&user, notif).await?` + チャネル `mail/database/broadcast/webpush` | 実装済み | `Notifiable` トレイト + チャネルごとの `Notification`。キューに入れられたディスパッチ（`Notify::queue`）は、Mailが使うのと同じ `EnvelopeOverrides` プリミティブを通じて、通知ごとの `queue`/`timeout`/`fail_on_timeout`/`max_tries`/`backoff` を各チャネルのジョブへ運びます。[通知](notifications.md)、[Web プッシュ](web-push.md) |
| パッケージ開発 | ワークスペースのアダプタークレート（例えば `suprnova-payments-stripe`） | 実装済み | Laravelのパッケージと同じ形です。フレームワークに依存し、コンテナへバインドし、必要ならマクロを公開します |
| プロセス（シェルコマンドの実行） | 標準ライブラリの `tokio::process::Command` | 意図的に非対応 | ファサードはありません - TokioのAPIがすでに正しい形です |
| キュー | `Queue::push(job).await?` + ドライバー `sync/memory/database/redis/null`、バッチ、チェーン、`JobMiddleware`、`FailedJobStore` | 実装済み | [キュー](queues.md) |
| ジョブが宣言する遅延 | `Job` 上の `fn delay() -> Option<Duration>`。`Queue::push` と `Queue::bulk` が尊重します | 実装済み | 明示的な `Queue::push_later` / `Queue::later(delay, job)` 呼び出しは、ジョブ自身の既定値より常に優先されます。[キュー](queues.md) |
| 一意なジョブの抑制イベント | `queue::events::UniqueJobSkipped { job_name, unique_id, connection }` | 実装済み | `push_unique` が重複排除したときプッシュ側で発火します。呼び出しはなお `Ok(false)` を返します |
| キューの一時停止（`queue:pause` / `queue:resume`） | `Queue::pause`/`resume`/`pause_all`/`resume_all`/`is_paused`/`paused_queues`。キャッシュに支えられ、`QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` イベントを伴います | 実装済み | キューごとの停止は、明示的な `--queue=...` リストで起動したワーカーでのみ有効です。`resume_all` はキューごとの停止を解除しません。[キュー](queues.md) |
| コミット後のディスパッチ（`afterCommit()`） | トランザクションの内側でプッシュされたジョブは、ただちにドライバーから見えます | 未実装 | 今日のところ、ロールバックしてもジョブはキューに残ります。トランザクションスコープのディスパッチが出荷されるまでは、プッシュをトランザクションの外側で行ってください |
| キュー接続のフェイルオーバー | `failover` ドライバーはありません | 未実装 | `FailoverQueueDriver` が出荷されるまでは、プッシュごとに接続を明示的に選ぶか、2つをラップする自前の `QueueDriver` をバインドしてください |
| `ShouldBeUniqueUntilProcessing` | `Queue::push_unique` は、ジョブ全体の間ロックを保持します | 未実装 | （完了時ではなく）クレーム時に一意性のロックを解放することは、まだ配線されていない別のセマンティクスです |
| キューの検査（`pendingJobs` / `delayedJobs` / `reservedJobs`） | ドライバーレベルの検査APIはありません | 未実装 | 検査の表面が出荷されるまでは、ドライバーの背後のストア（`jobs` テーブル、Redisのキー）を直接クエリしてください |
| タスクごとのスケジュールのタイムゾーン | スケジュールは、プロセス全体で1つのタイムゾーンで評価されます | 未実装 | タスクごとの `timezone(...)` と、タイムゾーンを意識した `schedule:list` が計画中です。[タスク スケジューリング](scheduling.md) |
| レート制限 | `RateLimiter::for_signature(...)`、`ThrottleRequestsMiddleware`、`RateLimitMiddleware` | 実装済み | `SlidingWindowConfig` によるスライディングウィンドウです。[レート リミット](rate-limiting.md) |
| 検索（Scout） | ファーストパーティの全文検索アダプターはありません | 未実装 | ベクトル検索は今日[ベクトル](vector.md)経由で出荷されています。キーワード検索のScout相当は計画中です |
| 文字列（ヘルパー） | `heck` クレート（ケース変換）、`std::str`、`regex` | 差異あり | Rustのエコシステムの他の部分が使うのと同じクレートです。グローバルな `Str::camel($x)` はありません |
| タスクスケジューリング | `Schedule::call/command/task` + `#[derive(Task)]` + cron構文 + `schedule:run` ワーカー | 実装済み | [タスク スケジューリング](scheduling.md) |
| べき等キー | `Idempotency::remember(key, ttl, body)` - Stripe形のリプレイ保護です | 実装済み | 呼び出し元が、ルート + ユーザー / ビジネス上のアイデンティティでキーに名前空間を与えます。[べき等性](idempotency.md) |
| リクエストのタイムアウト | ルートごとに設定可能な `TimeoutMiddleware` | 実装済み | Rustネイティブです - 実行中のfutureを中断し、ワーカーを解放します。[リクエスト タイムアウト](timeout.md) |
| フィーチャーフラグ（Pennant） | `Feature` + `Evaluator` + `FeatureMiddleware` + 管理用CRUD | 実装済み | `FeatureSync` トレイトによる1秒未満の伝播です。[フィーチャー フラグ](feature-flags.md) |
| 可観測性（Pulse） | `init_telemetry` によるOpenTelemetry、`Metrics`、あらゆる場所での `tracing` | 差異あり | OTelはRustの可観測性における共通語です - あなたのコレクターをバイナリへ向けてください。[可観測性](observability.md) |
| Telescope（デバッグダッシュボード） | 相当するものはまだありません | 未実装 | v2以降へ先送りされています。フレームワークの tracing + OTel の出力が、診断上のニーズのほとんどをカバーします |
| Pulse（性能ダッシュボード） | 相当するものはまだありません | 未実装 | Telescopeと同じです - ダッシュボードが出荷されるまでは、既存の可観測性スタックでメトリクスを表面化させてください |
| ベクトル検索 | `Vector::driver("memory"\|"qdrant"\|"pinecone"\|"mariadb")` | 実装済み | 「Postgresのpgvectorだけ」というゲートキーピングはありません。[ベクトル](vector.md) |

### Suprnova固有（Laravelに相当するものなし）

| Suprnova | 何であるか | 備考 / リンク |
|---|---|---|
| `ws!()` マクロ + WebSocketハンドラ | ルーターとミドルウェアのスタックを共有する、型付きのWSルート | [WebSocket](websockets.md) |
| ワークフロー | リトライ、スリープ、ステップ境界を伴う、長時間実行のステートフルな作業 | [ワークフロー](workflows.md) |
| スーパーバイザー | 長命なtokioタスクのための、パニック捕捉と自動再起動を備えた `Supervisor` トレイト | [スーパーバイザー](supervisors.md) |
| Web Push（VAPID） | ファーストクラスのチャネルとしての、ブラウザのプッシュ通知 | [Web プッシュ](web-push.md) |
| マルチ接続の読み書き分割 | `READ_REPLICA_CONNECTION_NAME` + `DB::on("read").select(...)` | [データベース](database.md) |
| 同じソケット上のHTTP/2 + WebSocket | `Server::run` の中の `hyper.with_upgrades()` | [リクエスト ライフサイクル](lifecycle.md) |
| Markdownコンテンツ + ドキュメントのパイプライン | `MarkdownRenderer`（サニタイズされた comrak → syntect → ammonia） + `build_docs(DocsBuildConfig)` → `DocsChapter` の検索可能な `DocsCatalog` | 見出しの抽出 + `slugify_heading`。別途の静的サイトジェネレーターなしで、Markdownのドキュメントとブログを支えます |

## セキュリティ

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| 認証 | `Auth::user/check/login/logout/attempt`、`Authenticatable` トレイト、名前ごとの `Guard` | 実装済み | [認証](authentication.md) |
| 複数のガード | `AuthManager` 経由で名前（`web`、`api`、…）によって登録される `Guard` | 実装済み | `SessionGuard`、`TokenGuard`、カスタム実装 |
| ユーザープロバイダー | `EloquentUserProvider<U>`、`DatabaseUserProvider`、`UserProvider` トレイト経由のカスタム | 実装済み | [認証フロー](auth-flows.md) |
| メール確認 | `EmailVerification` + `EnsureEmailVerifiedMiddleware` + `EmailVerificationMail`。`MustVerifyEmail` コントラクト | 実装済み | プロバイダーに支えられ、アクターにバインドされます - [認証フロー](auth-flows.md) |
| パスワードリセット | `PasswordReset` + Magnetar による最初のメール証明トランザクション、または検証済み `UserProvider` へのフォールバック + リセットまたは変更メール | 実装済み | 最初のアトミックな証明は Magnetar が処理します。プロバイダーを使用するアプリでは、検証済みユーザーをリセットできます。 - [Auth flows](auth-flows.md) |
| ブルートフォース制限 | Magnetarのロックアウトエンジン + `BruteForce` + `LoginThrottleMiddleware` | 実装済み | アカウントのロックアウトに加えて、フレームワークのIP/ルート制限 |
| 二要素認証（TOTP） | フレームワークの `TwoFactor` 互換ファサード + Magnetarの要素エンジン | 実装済み | リカバリーコード、リプレイ保護、要素でゲートされた統合サインイン |
| ログイン状態の保持（remember-me） | フレームワークのCookieの背後にある、Magnetarの目的バインド型ローテーション・クレデンシャル | 実装済み | 認証エポック検査、ローテーション、異常処理、レガシーフォールバック |
| OAuth（Socialite） | Magnetarのプロバイダーレジストリと `Auth::oauth(provider)` ファサード | 実装済み | OAuth、Appleの `form_post`、PKCE/stateバインディング、検証済みアイデンティティポリシー - [OAuth](oauth.md) |
| Sanctum（APIトークン） | Magnetarのbearerセッション上の `BearerTokenMiddleware` | 差異あり | bearerセッションを認証します。独立したSanctumのトークン管理APIはありません |
| Passport（OAuthサーバー） | Magnetarのプロトコルおよびプラグインエンジン | 差異あり | エンジンのプリミティブは出荷されます。Laravel Passport互換のアプリケーションファサードはありません |
| Fortify（認証バックエンド） | Magnetarエンジン上のフレームワークの `Auth` / `auth_flows` ファサード | 実装済み | フレームワークがHTTP、メール、イベント、クッキー、アプリケーションバインディングを所有します |
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
| 親タイムスタンプへのtouch | `#[model(touches = ["post"])]` | 実装済み | `BelongsTo` の所有者ごとに1つの `UPDATE`。1レベル深く、イベントなし（祖父母への再帰なし、親の `saved` イベントなし）。スキップするには `without_touching` / `without_touching_on::<M, _, _>()`。[親のtouch](eloquent.md#parent-touching) |
| オブザーバー | `impl Observer<User>` + `#[suprnova::observer(User)]` | 実装済み | 16のライフサイクルイベント |
| 16個のライフサイクルイベント | `Created`, `Creating`, `Saving`, `Saved`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Trashed`, `Restoring`, `Restored`, `Retrieved`, `Replicating`, `ForceDeleting`, `ForceDeleted`, `Pruning` | 実装済み | モデルごとの `events::*` サブモジュール。`EventResult::cancel(_)` が400で短絡します |
| ミューテータ / アクセッサー | `#[accessor] fn full_name(&self) -> String { ... }` + `#[mutator] fn set_password(&mut self, v: String)` | 実装済み | [ミューテータ](eloquent-mutators.md) |
| キャスト（22種類組み込み） | `casts! { AsString, AsInt, AsFloat, AsBool, AsJson, AsArray, AsArrayObject, AsObject, AsCollection, AsDate, AsDateTime, AsImmutableDate, AsImmutableDateTime, AsOptionalDateTime, AsTimestamp, AsDecimal, AsEnum<E>, AsEncrypted, AsEncryptedObject, AsEncryptedArray, AsEncryptedCollection, AsHashed }` | 実装済み | カスタムのためには `Cast` を実装してください |
| コレクション | `pluck`、`filter`、`map`、`each`、`chunk`、`groupBy`、`keyBy`、`sort_by`、`where_`、`first`、`last`、`count`、`is_empty`、`to_array` などLaravel系のメソッドを持つ `Collection<M>`。`Deref<Target = Vec<M>>` のため、あらゆる `Vec` のイディオムがそのまま動きます | 実装済み | [コレクション](eloquent-collections.md) |
| APIリソース | `#[derive(Resource)]` + `IntoJsonResource` + `JsonApiResponse` + フィールドセット + インクルード | 実装済み | JSON:API の形と Laravelスタイルのリソースの形、両方が利用できます。[API リソース](eloquent-resources.md) |
| シリアライゼーション | `#[model(hidden = [...], visible = [...], appends = [...])]` | 実装済み | どの属性がシリアライズされるかを、同じように制御できます。[シリアライゼーション](eloquent-serialization.md) |
| ファクトリー | `#[derive(Factory)] struct UserFactory` + `UserFactory::new().count(5).create().await?`（または `UserFactory::times(5).create_many().await?`） | 実装済み | 値を循環させる `Sequence`。[ファクトリー](eloquent-factories.md) |
| `modelKeys()` | `Builder::model_keys().await?`（ハイドレーションなし、修飾されたキー）と `Collection::model_keys()` | 実装済み | どちらも `Vec<M::Key>` を返します。ビルダーの終端は `users.id` を射影するため、joinをまたいでも保持されます |
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
| Pest / PHPUnit スタイル | `#[suprnova_test]`（非同期を意識します） + Jestに似た `expect!()` のアサーション + `describe!()` / `test!()` のBDDマクロ | 実装済み | 3つとも、互いに置き換えて使えます |
| 機能テスト（HTTP） | `handle_request(router, registry, req)` を同一プロセスで駆動します。通常はループバックのhyperコネクションを通すため、サーバーは本物の `Incoming` ボディを受け取ります | 実装済み | [HTTP テスト](http-tests.md) |
| `TestResponse` のラッパー | `suprnova::testing::TestResponse` - 流暢な `assert_status` / `assert_json_path` / `assert_cookie` / `assert_session_has` など。すべて `&Self` をチェーンします | 実装済み | [HTTP テスト](http-tests.md#fluent-response-assertions-with-testresponse) |
| Inertiaテストヘルパー | `suprnova::testing::AssertableInertia` - `component`/`url`/`version`/`prop`/`has`/`missing`/`where_`/`count`/`has_flash`、さらに呼び出し側が提供する `with_reload` クロージャー経由の `reload_only`/`reload_except`/`load_deferred_props` | 実装済み | [HTTP テスト](http-tests.md#testing-inertia-responses) |
| コンソールのテスト | `dispatch_argv(["console", "..."])` を実行してアサートします | 実装済み | コンソールのバイナリについて、HTTPテストと同じ形です |
| ブラウザテスト（Dusk） | フレームワークには該当なし - Playwright / WebdriverIO / `gstack` agent browser を使ってください | 意図的に非対応 | 言語をまたぐツールがすでに存在します。私たちはそれを再発明しません |
| データベースのテスト | `TestDatabase::fresh::<Migrator>()` | 実装済み | テストごとに新しい独立したインメモリSQLiteデータベースを作成し、マイグレーションを適用してテストコンテナに登録し、その独立したデータベース/コンテナの状態をdrop時に破棄します。テストごとをロールバックトランザクションで囲むことはありません。[データベース テスト](database-testing.md) |
| モックとフェイク | ファサードごとのフェイク。`MailFake`、`NotifyFakeGuard`、`EventFakeGuard`、`Queue::fake`、`Bus::fake`、`Http::fake`、`Storage::fake` | 実装済み | 記録された呼び出し + アサーションのヘルパーです。[モックとフェイク](mocking.md) |
| `QueueFake` のジョブUUID | `queue::testing::pushed_with_id::<J>()` | 実装済み | フェイクはプッシュごとにエンベロープIDを付け、実際のプッシュと同じ `JobQueued` を発火します |
| タイムトラベル | 標準ライブラリのランタイムの `tokio::time::{pause, advance, resume}` | 実装済み | 自前のものは出荷しません - TokioのAPIがすでにそれを行います |
| コンテナの隔離 | `TestContainer::fake(\|tc\| tc.bind(...))` - スレッドローカルです | 差異あり | 構造上、並列で安全です。[サービス コンテナ](container.md) |

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

## フロントエンド（Laravel には Blade + スターターキットがあります。こちらには Inertia があります）

| Laravel | Suprnova | ステータス | 備考 / リンク |
|---|---|---|---|
| Blade | 該当なし - Inertiaがビュー層です | 差異あり | [フロントエンド](frontend.md) |
| Inertia.js | ファーストクラス。Svelte 5 / React 19 / Vue 3.5 の上のv3です | 実装済み | [Inertia レスポンス](frontend-inertia-responses.md)、[ページ](frontend-pages.md) |
| `Route::inertia($uri, $component, $props)` | `Router::inertia(path, component, props)` | 実装済み | `RouteBuilder` を返すため `.name(...)` / `.middleware(...)` をチェーンできます。`Router::view` は古いエイリアスです |
| ページURLの解決（`Inertia::resolveUrlUsing`） | `page.url` はパス + クエリです。`InertiaConfig::url_resolver` で上書きします | 実装済み | デフォルトの導出は、バージョンのミドルウェアの `X-Inertia-Location` とバイト単位で一致します。`url_resolver` が変えるのは `page.url` だけです |
| Inertiaプロトコルのミドルウェア（`Vary`、空のレスポンス、バージョンの跳ね返し） | `InertiaHeadersMiddleware` + `InertiaVersionMiddleware` + `Inertia303Middleware` - `Inertia::install` が配線する4つのミドルウェアのうち3つ（4つ目の検証エラーリダイレクトは次の行） | 実装済み | すべてのレスポンスに `Vary: X-Inertia`。Inertiaの訪問での空の `200` は `303` の戻しになります。409の跳ね返しはセッションを再フラッシュします |
| 検証エラーのリダイレクト（`Middleware::resolveValidationErrors`、`$withAllErrors`） | `Inertia::install` が配線する `InertiaValidationRedirectMiddleware`。`InertiaConfig::with_all_errors(bool)` | 実装済み | Inertia訪問の `422` は、エラーをフラッシュして `303` で戻ります。`with_all_errors(true)` でない限り、フィールドの値は最初のメッセージへ折りたたまれます。[Inertia レスポンス](frontend-inertia-responses.md#validation-failures) |
| 外部へのリダイレクト + 履歴のクリア | `InertiaResponse::location_for(&req, url)`、`App::clear_history()` | 実装済み | `location_for` は、XHRには `409`、ハードナビゲーションには `302` です。`App::clear_history()` はログアウトのリダイレクトを生き延びます |
| 部分的なリロード | `#[derive(Data)]` + `req.includes("subset")` + Inertiaの部分的リロードのプロトコル | 実装済み | 型安全なincludeの集合です。`?include=` は `lazy(deferred)` を含むすべてのlazyの形をゲートし、`X-Inertia-Partial-Data` より前に実行されるため、許可されないincludeでも400を返します。`errors` は `only` / `except` の対象外で、Laravelの `Inertia::always` 共有に一致します |
| `Inertia::share` / `getShared` / `flushShared` | `App::inertia_share` / `_lazy` / `_once`、`App::inertia_shared(key)`、`App::flush_inertia_shared()` | 実装済み | `Arr::set` セマンティクス経由のドットキーネストです。リクエストごとの `InertiaSharedData::share(&req, component)` はページごとに変わります。ドット付きの共有は、レスポンスの解凍パスまでフラットに保たれるため、`only` / `except` は祖先エントリに一致します（`only: ['auth']` は `auth.user` に到達します）。Laravel は share 時に `Arr::set` から同じ結果を得ます |
| ディファードプロップ | `.defer(…)` / `.defer_with(…, DeferOptions)`、または `Prop::…defer()` | 実装済み | Inertia v3のディファードプロッププロトコルです。`DeferOptions` がグループとrescueフラグを運びます。`deferredProps` は初回訪問だけに送られ、対応するpartialでは `resolveDeferredProps` が `[]` を返します |
| マージプロップ | `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with(MergeStrategy)` / `.merge_lazy` / `.merge_lazy_with`、または `Prop::…merge().merge_with_path(...)` | 実装済み | Inertia v3のマージプロトコルです。`match_on` は1つまたは複数のフィールドを取り、`merge_with_path` はプロップのルートではなくネストされたフィールドをマージします |
| プロップの合成（`defer()->merge()`、`merge()->once()`、`optional()->once()`） | `Prop` フラグビルダー + `InertiaResponse::prop(key, prop)` | 実装済み | `Prop` は直交するフラグの構造体で、PHPアダプターの `Deferrable` / `Mergeable` / `Onceable` インターフェースを反映します |
| 履歴の暗号化 | `EncryptHistoryMiddleware` | 実装済み | 履歴は、クライアント内で保存時に暗号化されます |
| スクロール位置 | `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.paginate` + `ScrollMetadata` / `ProvidesScrollMetadata` | 実装済み | ナビゲーション時に自動で復元します。`reset` は `X-Inertia-Reset` を読み取り、`resolveScrollProps` と一致します |
| TypeScriptの型 | `suprnova generate-types` が `#[derive(InertiaProps)]` を読み取り、`.d.ts` を出力します | 実装済み | [TypeScript 型](frontend-typescript-types.md) |
| Viteのマニフェストの読み取り | `InertiaConfig::manifest_path` 経由で自動配線されます | 実装済み | 開発ではHMR、本番ではハッシュ付きアセットです。マニフェストが欠けているとき、`Inertia::install` は本番でフェイルクローズします |
| ビルドマニフェストからのアセットバージョン | `InertiaConfig` のデフォルト: `VersionResolver::from_manifest(manifest_path)` | 実装済み | マニフェストのバイト列のハッシュです。ハッシュするビルドがない場合は静的な `"1.0"` にフォールバックします |
| Inertia SSR（`inertia:start-ssr`） | `Inertia::install` へ渡す設定の上の `InertiaConfig::ssr(...)`。ワーカーは `suprnova ssr:start` で起動します | 実装済み | HTTPのループバック越しのプロセス外ワーカーです。`ssr_throw_on_error(true)` でない限り、エラーやタイムアウトのときはCSRへフォールバックします。`InertiaConfig::ssr_bundle_path(...)` は、ディスパッチをビルド済みバンドルがディスクに存在する場合だけに制限し（`ensure_bundle_exists` に対応）、`.ssr_ensure_bundle_exists(bool)` で切り替えます（バンドルパスを設定するとデフォルトで有効）。`suprnova new` はすべてのスターターに `frontend/src/ssr.{ts,tsx}` と `build:ssr` スクリプトをスキャフォルドし、`suprnova ssr:check` はワーカーの `GET /health` ルートを検証します。[Inertia レスポンス](frontend-inertia-responses.md) |

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
| `php artisan optimize` | `cargo build --release` | 差異あり | 1つのバイナリであり、opcacheのステップはありません |
| `php artisan config:cache` | 型付き設定は、すでにコンパイル時に検査されています | 差異あり | 無効化すべき実行時のキャッシュはありません |
| `php artisan route:cache` | ルートは、コンパイル時にマクロ展開されます | 差異あり | ルーターは、すでに型付けされたルートから起動時に構築されます |
| Envoy（SSHデプロイ） | 任意のオーケストレーターを使ってください - Docker、systemd、Kubernetes、fly.io、Railway | 意図的に非対応 | バイナリがデプロイのアーティファクトです |
| Forge / Vapor | 私たちが出荷するものではありません - しかし、Railway、DO、Hetzner のレシピが同じ仕事をカバーします | 差異あり | [デプロイメント](deployment.md)、[Railway](deployment-railway.md)、[Digital Ocean](deployment-digital-ocean.md)、[Hetzner](deployment-hetzner.md) |
| メンテナンスモード（`php artisan down` / `up`） | `./app down` / `./app up` - バイパス用のシークレット、カスタムのretry/message/exceptパス、`file` または `cache` のドライバー | 実装済み | [デプロイメント](deployment.md) |
| Horizon（キューのダッシュボード） | ダッシュボードはまだありません | 未実装 | それまでは、`cargo run --bin console queue:failed` 経由で失敗したジョブを検査してください |

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
| Sanctum | `BearerTokenMiddleware` がMagnetarのbearerセッションの上にあります | 差異あり | 独立したパッケージも個人アクセストークン管理の表面もありません |
| Scout（フルテキスト検索） | まだ該当なし | 未実装 | ベクター検索が出荷されています（[ベクター](vector.md)）。キーワード Scout 相当物は後で |
| Socialite | Magnetarのプロバイダーレジストリと `Auth::oauth(provider)` | 実装済み | [OAuth](oauth.md) |
| Telescope | まだ該当なし | 未実装 | Tracing + OTel は、ダッシュボードが出荷されるまでの診断ギャップをカバーします |
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
| `csrf_token()` | `csrf_token()`（同じ名前） | [CSRF](csrf.md) |
| `dd()` | `Builder::dd()`（Eloquentのクエリdump-and-die） / stdlibの `dbg!()` | `Builder::dump()` / `Builder::dd()` はクエリ調査のために存在します。一般的な値には `dbg!()` を使ってください |
| `env('APP_KEY')` | `env("APP_KEY")` / `env_required("APP_KEY")` / `env_optional("APP_KEY")` | [設定](configuration.md)、[環境変数](env-vars.md) |
| `now()` | `chrono::Utc::now()`（`suprnova::chrono` として再エクスポート） | - |
| `optional($x)->y` | `x.as_ref().map(\|x\| x.y)` | Rust はこれを `Option<T>` で直接扱います |
| `redirect('/')` | `redirect("/")`（同じ名前） | [ルーティング](routing.md) |
| `request()` | `Request` はハンドラへ渡されます | [リクエスト](requests.md) |
| `response()` | `HttpResponse::json/text/redirect/...` | [レスポンス](responses.md) |
| `route('posts.show', ['post' => 1])` | `url("posts.show", &[("post", "1")])` | [URL 生成](urls.md) |
| `session('key')` | `session().get("key")` | [セッション](session.md) |
| `str()` / `Str::camel($x)` | `heck` クレートのメソッド（`ToUpperCamelCase` など） | - |
| `tap($x, fn) → $x` | `tap` クレートの `tap`、または素早い確認のための `dbg!` | `tap` クレートをイディオムどおりに使ってください |
| `today()` | `chrono::Utc::now().date_naive()` | - |
| `value($x)` | クロージャを呼ぶだけです: `x()` | 該当なし - Rustのクロージャにヘルパーは不要です |
| `view('home', $data)` | Inertiaレスポンス: `Inertia::render("Home", data)` | [Inertia レスポンス](frontend-inertia-responses.md) |

## 本当にまだ持っていないもの

上記のすべての**未実装**をまとめたリストです。ギャップの形を1か所で見られるようにするためのものです:

| 領域 | 何が欠けているか | 出荷されるまでの回避策 |
|---|---|---|
| 検索（Scout - キーワード） | Algolia / Meilisearch / Elastic のアダプター | 出荷されるまでは、`meilisearch-sdk` / `elasticsearch` で自作してください。[ベクトル](vector.md)は、今日すでにセマンティック検索を扱います |
| Passport（OAuthサーバー） | ファーストパーティのOAuth IDプロバイダー | Suprnovaの背後で Hydra / Keycloak を走らせてください |
| Telescope（デバッグダッシュボード） | リクエスト / クエリ / イベント / キャッシュヒットのWeb UI | OTel + tracing の出力を使ってください（[可観測性](observability.md)） |
| Pulse（性能ダッシュボード） | 遅いクエリ / エラー / ホットなルートのWeb UI | 同じです。今日はOTelの表面、ダッシュボードは後日です |
| Horizon（キューのダッシュボード） | キューの深さ / 失敗したジョブ / スループットのWeb UI | `cargo run --bin console queue:failed` とOTelのメトリクスです |
| 画像処理 | `Illuminate\Image` 相当（リサイズ / クロップ / 変換） | 自分自身の `App::bind` の背後で `image` クレートを直接使ってください |
| `Password` のバリデーションルール | 強度ルール + `uncompromised()` のHIBPチェック | `Min` + `Regex` + カスタムの `Rule` を組み合わせてください |
| コミット後のディスパッチ | トランザクションスコープのジョブのディスパッチ | トランザクションが返った後にプッシュしてください |
| キュー接続のフェイルオーバー | 順序付きのドライバーのリストの上の `failover` ドライバー | プッシュごとに接続を選んでください |
| `ShouldBeUniqueUntilProcessing` | クレーム時に解放されるロック | `push_unique` は、ジョブ全体の間ロックを保持します |
| キューの検査 | `pendingJobs` / `delayedJobs` / `reservedJobs` | ドライバーの背後のストアをクエリしてください |
| タスクごとのスケジュールのタイムゾーン | スケジュールされたタスクごとの `timezone(...)` | タイムゾーンごとにスケジューラーのプロセスを1つ走らせてください |

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
| `php artisan dev --tabs`（TUIマルチペイン開発プロセスモード） | 単一ターミナルの `[name]` プレフィックス付き出力はRust開発ツールの標準（`cargo watch`、`bacon`、`just`）です。`suprnova serve` はすでに各プロセス（バックエンド、フロントエンド、`Suprnova.toml` のエントリ）へ色付きの独自プレフィックスと自動再起動を与えます。タブ付きTUIは、すでに提供している信号のための2つ目の対話モデルです。`--stream` の仕事、つまりスクリプト可能なリアルタイム出力ストリームは `suprnova serve --json`（NDJSON、1行1イベント）として出荷します。[Serve](cli-serve.md#extra-dev-processes) |

## このリストが正直であり続ける方法

**実装済み**の列のすべての行は、次の方法で検証できます:

1. 名前を挙げられたエクスポートを `framework/src/lib.rs` でgrepする
2. フレームワークのテストスイートを実行する（`cargo test --workspace`）
3. リンクされたチャプターを読む

**未実装**の列のすべての行は、拒否ではなく、意図された作業です。**意図的に非対応**の列のすべての行には、備考の列に一文の理由があります。それらの理由は、[はじめに](introduction.md)にある設計原則を、特定の機能へ適用したものです。

最後に Laravel 13.25.0 に対して見直しました。

あなたが手を伸ばすLaravelの機能で、このマップに載っていないものを見つけたら、issueを開いてください - 行が抜けているだけでSuprnovaの答えが存在するか、あるいは本物のギャップであり、私たちはそれを知りたいのです。

## 次のステップ

- [Laravel から](from-laravel.md) - 同じマップを、並列比較として語り直したもの
- [はじめに](introduction.md) - このパリティ作業が従う設計原則
- [`documentation.md`](documentation.md) - 全チャプターにわたるマスターTOC
