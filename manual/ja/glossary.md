# 用語集

Suprnova固有の用語を、一度だけ定義します。あるチャプターが説明なしに単語を使っている場合、その定義はここにあります。エントリはアルファベット順です。文脈の中でその用語を使っているチャプターへのクロスリンクをたどってください。

このリストの残りを読む際に頭に入れておくべき、いくつかの慣例です:

- **トレイト** はRustのトレイトを意味します - 型に実装するふるまいの契約です。**ファサード** は、静的メソッドがサブシステムへの入り口となる、サイズゼロの構造体を意味します（`Cache`、`Mail`、`Auth`、`Storage`、`Bus`、`Notify`、`Vector`、`DB`、`Schedule`、`App`）。
- **ドライバー** は、ファサードやレジストリの背後にある、差し替え可能なバックエンドを意味します - `CacheStore`、`QueueDriver`、`VectorDriver`、`RateLimiterDriver`、`MailDriver`です。ドライバーは、環境変数を介して起動時に選ばれ、コンテナを通じてバインドされます。
- **レジストリ** は、`inventory` 経由でコンパイル時に、あるいは明示的な登録によって起動時に埋められる、プロセスグローバルなルックアップを意味します - `ConnectionRegistry`、`MiddlewareRegistry`、`InertiaRegistry`、`ChannelRegistry`、`VectorRegistry`、`SupervisorRegistry`、`PaymentProviderRegistry`、`ScopeRegistry`です。

## A

### アクセッサー

`#[accessor]` マクロでEloquentモデル上に宣言される、読み取り側の変換です。プロパティが読まれるたびに実行され、1つ以上の基礎となるカラムから導出された計算済みの値を返します（例えば、`first_name + last_name` からの `full_name`）。[ミューテータ](#ミューテータ)の対です。[Eloquent - アクセッサーとミューテータ](eloquent.md#accessors-and-mutators)を参照してください。

### アクション

1つのビジネスロジックをカプセル化する、注入可能なサービスクラスです - 単一のパブリックメソッドで、依存は `#[injectable]` マクロを介して注入されます。LaravelのシングルアクションのInvokableに相当するSuprnovaの仕組みです。アクションはコンテナ内でシングルトンとして自動的にバインドされ、ハンドラ、ジョブ、そして他のアクションから解決されます。[アクション](actions.md)を参照してください。

### アプリケーション

`Application::new()` の中にあるフルーエントビルダーで、あなたのconfig、bootstrap、routes、migrations関数を登録した後、`.run()` を呼んでバイナリのCLIサブコマンド（`serve`、`migrate`、`queue:work` など）をディスパッチします。バイナリごとに1つで、`src/app.rs` にあります。[リクエスト ライフサイクル](lifecycle.md)を参照してください。

### アトミックカウンター

読み取り-変更-書き込みの競合なしに、単一のラウンドトリップで数値を変異させるキャッシュ操作です（`Cache::increment`、`Cache::decrement`）。Redisストア上ではRedisの `INCR`/`DECR` に支えられ、インメモリストア上では保持されたガードに支えられます。[キャッシュ - アトミックカウンター](cache.md#atomic-counters)を参照してください。

### Authenticatable

認証済みのユーザー型が実装するトレイトです（`get_auth_identifier() -> String`、`get_auth_password()` など）。これにより、ガードとミドルウェアは、具体的なユーザー構造体を知ることなくそれと対話できます。[認証](authentication.md)を参照してください。

### Authorizable

[ゲート](#ゲート)が使う、ポリシーの入り口（`can`、`can_any`、`cannot`）をユーザー型に与えるトレイトです。[認可](authorization.md)を参照してください。

## B

### バックオフスケジュール

失敗したジョブのリトライの間で、キューワーカーが待機する遅延の並びです。`BackoffSchedule::linear`、`BackoffSchedule::exponential`、あるいはカスタムの `Vec<Duration>` です。[キュー - バックオフスケジュール](queues.md#backoff-schedules)を参照してください。

### バッチ（キュー）

まとめてディスパッチされ、1つの単位として追跡されるジョブのグループです - `PendingBatch::new().add(job).add(other).dispatch()` が、永続化されたバッチidを返します。作業をファンアウトし、バッチ全体が完了したときにコールバックを走らせたいときに便利です。[キュー - キューに入れられたバッチ](queues.md#queued-batches)を参照してください。

### `BelongsTo`

`HasOne`/`HasMany` の逆にあたるリレーションの種類です - 子が外部キーを持ち、親が反対側にあります。11あるEloquentのリレーション種別の1つです。[Eloquent - リレーションシップ](eloquent.md#relationships)を参照してください。

### `BelongsToMany`

第三の、ファーストクラスの[ピボット](#ピボット)モデルを経由する、多対多のリレーション種別です。`BelongsToMany<Local, Related, Pivot>` - ピボットは文字列の規約で合成されるのではなく、型の中で名指しされます。[Eloquent - リレーションシップ](eloquent.md#relationships)を参照してください。

### ブートストラップ

`Application` ビルダー上に登録し、起動時に一度だけ（configの後、servingの前に）実行される `bootstrap_fn` です。[コンテナ](#コンテナ)へサービスをバインドし、オブザーバーとイベントリスナーを登録し、デフォルトのヘッダーを設定する、といったことを行う場所です。Laravelのサービスプロバイダーに相当するSuprnovaの仕組みで、1つの関数に折り畳まれています。[アプリケーション ブートストラップ](bootstrap.md)を参照してください。

### Broadcastable

[イベント](#イベント)が、ローカルなインプロセスのリスナーの代わりに（あるいはそれに加えて）WebSocketの購読者へプッシュされるべきときに実装するトレイトです。イベントディスパッチャーと[Broadcast Hub](#broadcasthub)の間の橋渡しです。[ブロードキャスト](broadcasting.md)を参照してください。

### `BroadcastHub`

「チャネルのすべてのWebSocket購読者へメッセージをファンアウトするもの」を名指しするトレイトです - インメモリの実装（`InMemoryBroadcastHub`）がデフォルトで、sea-streamerの実装（`SeaStreamerBroadcastHub`）がマルチプロセスの本番デプロイです。[ブロードキャスト - マルチプロセスファンアウト](broadcasting.md#multi-process-fanout)を参照してください。

### ビルダー（Eloquent）

`Model::query()` が返すフルーエントなクエリオブジェクトです - `.get()`、`.first()`、`.paginate(...)` の前に `where`、`order_by`、`with`、`limit` などを組み立てる、チェーン可能な表面です。二重に名付けられています: すべてのフィルターメソッドは、Laravelの名前（`db_where`、`db_or_where`）とRustネイティブの同義語（`filter`、`or_filter`）の両方の下に存在します。[Eloquent - クエリビルダー](eloquent.md#query-builder--dual-api)を参照してください。

### Busコマンド

`Bus::dispatch(cmd)` を通じてディスパッチされる、シリアライズ可能な構造体で、単一の登録済み `Handler<C>` へルーティングされます。Busコマンドは、その結果を呼び出し元へ戻すべき、インプロセスの作業のためのものです - キューの[ジョブ](#ジョブ)は、バックグラウンドで永続化されリトライされるべき作業のためのものです。[コマンドバス](bus.md)を参照してください。

## C

### キャッシュドライバー

`Cache` ファサードの背後にある、選択されたバックエンド（`memory` または `redis`）です。`CACHE_DRIVER` を介して起動時に選ばれ、[CacheStore](#cachestore)トレイトを通じて表面化します。[キャッシュ](cache.md)を参照してください。

### `CacheStore`

キャッシュドライバーのSPIを定義するトレイトです - `get`、`put`、`forget`、`increment` などです。`InMemoryCache` と `RedisCache` が出荷済みの実装です。[キャッシュ - 設定](cache.md#configuration)を参照してください。

### キャスト（Eloquent）

Eloquentモデル上に `casts!` で宣言される、双方向の変換です - DBのカラム型 ↔ Rustの型。22の組み込みが出荷されます（`AsBool`、`AsDateTime`、`AsJson`、`AsEncrypted`、`AsArray` など）。それ以外は、ユーザーが実装した `Cast` トレイトでカバーします。[Eloquent - キャスト](eloquent.md#casts)を参照してください。

### チェーン（キュー）

前のものが成功したときにだけ次が走るよう連結された、[ジョブ](#ジョブ)の並びです。`PendingChain::dispatch` / `Queue::chain` で構築します。[キュー - キューに入れられたチェーン](queues.md#queued-chains)を参照してください。

### チャネル（ブロードキャスト）

イベントがブロードキャストする先のトレイトです - `PublicChannel`、`PrivateChannel`、あるいは `PresenceChannel` です。チャネルの構造体は自分自身に名前を付け（`fn name() -> String`）、コネクションを認可します（`fn authorize(...)`）。プライベートチャネルとプレゼンスチャネルは、より強いトレイト境界を追加します。[ブロードキャスト - チャネル](broadcasting.md#channels)を参照してください。

### チャネル（通知）

[通知](#通知)を配信メカニズムへルーティングするトレイトです - メール、データベース、ブロードキャスト、Webプッシュです。通知は `fn via(...)` の中で自分のチャネルを名指しします。各チャネルは、目的地を解決して送信します（`MailRendering` / `DatabaseChannel` のペイロードメソッドのような、チャネルごとのトレイトを介して）。同名のブロードキャストのトレイトとは別物です。[通知 - チャネル](notifications.md#channels)を参照してください。

### コンテナ

`App` ファサードを通じてサービスがバインドされ解決される、3層（タスクローカル → スレッドローカル → グローバル）のレジストリです。Laravelのサービスコンテナに相当するSuprnovaの仕組みで、リクエストごととテストごとの分離のための追加の層を持ちます。[サービス コンテナ](container.md)を参照してください。

### コンテキスト（リクエストごと）

同じ非同期タスク内のあらゆるコードから到達可能な、リクエストごとの型付きの値のバッグです - `Context::set::<T>(value)`、`Context::get::<T>()`。明示的に伝播させれば、タスクのスポーンをまたいで生き残ります。同じ名前を持つフィーチャーフラグのコンテキストとは別物です。[コンテキスト](context.md)を参照してください。

### CORS

Cross-Origin Resource Sharingです。オリジンAからオリジンBへのJavaScriptのfetchをゲートするブラウザのセキュリティルールです。Suprnovaは、どのクロスオリジンリクエストが許可されるかを伝えるレスポンスヘッダーを発するために `CorsMiddleware` を出荷します。[CORS](cors.md)を参照してください。

### CSRF

Cross-Site Request Forgeryです。ステートフルなセッションが防御しなければならない攻撃です。Suprnovaは、状態を変更するすべてのリクエストに一致するトークンを要求するために `CsrfMiddleware` を出荷します。[CSRF保護](csrf.md)を参照してください。

## D

### `DB` ファサード

データベースへの、モデルを介さない入り口です - `DB::table(...)`、`DB::transaction(...)`、`DB::raw(...)` です。Eloquentの形に収まらないクエリ（動的なカラム、結合された集計、生のSQL）のためのものです。[Eloquent - DBファサード](eloquent.md#db-facade--model-less-queries)を参照してください。

### ディスク

`Storage` ファサードを通じて登録される、名前付きのストレージバックエンドです - `Storage::disk("s3")`、`Storage::disk("local")` です。各ディスクは[DiskExt](#diskext)を実装し、登録名でキー付けされます。[ファイルシステムとストレージ](filesystem.md)を参照してください。

### `DiskExt`

すべてのストレージバックエンドが実装するトレイトです - `put`、`get`、`delete`、`list`、`signed_url` などです。内部は `opendal` に支えられています。ローカルfs、インメモリ、S3、Azure Blob、GCS向けのアダプターを出荷します。[ファイルシステムとストレージ](filesystem.md)を参照してください。

## E

### Eloquent

ORM層全体です - `Model` トレイト、`Builder<M>`、リレーション、キャスト、スコープ、オブザーバー、イベント、ソフトデリート、prunable、ファクトリーです。他のエコシステムがORMと呼ぶものに対するLaravelの呼び名です。Suprnovaでは、SeaORM（ユーザーには見えないはずのもの）の上に載っています。[Eloquent](eloquent.md)を参照してください。

### エンベロープ（キュー）

キュードライバーが実際にシリアライズして保存する、ラッパー構造体です（`Envelope { payload, attempts, max_attempts, delay, ... }`）。[ジョブ](#ジョブ)のペイロードを、キューの配管から絶縁します。[キュー](queues.md)を参照してください。

### イベント

`EventDispatcher::dispatch(evt)` を通じてディスパッチされ、登録済みのすべての `Listener<E>` へ配信される、クローン可能な構造体です。Suprnovaは、トレイト、ファサード（`EventFacade`）、`Subscriber` アグリゲーター、そして[キューに入れられたリスナー](#キューに入れられたリスナー)のためのフックを出荷します。[イベント](events.md)を参照してください。

### イベントリスナー

[リスナー](#リスナー)を参照してください。

## F

### ファサード

サブシステムの公開APIを `impl` ブロックが保持する、サイズゼロの構造体のための命名規約です - `Cache`、`Mail`、`Auth`、`Storage`、`Bus`、`Notify`、`Vector`、`DB`、`Schedule`、`App` です。Laravelから受け継がれたもので、Suprnovaでは、根底にある実装はPHPのマジックコールではなく、[コンテナ](#コンテナ)を通じて解決されます。[サービス コンテナ](container.md)を参照してください。

### ファクトリー（Eloquent）

`fake` 駆動のデフォルトを使って現実的なテスト用の行を生成する、`#[derive(Factory)]` マクロと `Factory` トレイトです - `UserFactory::times(5).create_many().await?`。Laravelのモデルファクトリーに対応するRust側です。[マクロ - ファクトリー](macros.md#factories)を参照してください。

### フェイルクローズ

バックエンドの障害がリクエストを5xxで拒否させる、ドライバー障害時のポリシーです - レートリミット、セッション、べき等性で、「漏らすより拒否する方がまし」なときに使われます。[フェイルオープン](#フェイルオープン)の逆です。`BackendErrorPolicy::FailClosed` 経由で設定します。[レート リミット](rate-limiting.md)を参照してください。

### フェイルオープン

バックエンドの障害があっても、（ログに記録された警告付きで）拒否する代わりにリクエストを通す、ドライバー障害時のポリシーです - 可用性がリミットに優先するときに使われます。`BackendErrorPolicy::FailOpen` 経由で設定します。[レート リミット](rate-limiting.md)を参照してください。

### フィーチャー フラグ

名前でキー付けされ、現在のユーザー / コンテキストに対して評価される真偽値（または型付きの値）です - `feature!(MyFeature)`。`Evaluator` トレイトに支えられ、データベースエバリュエーターと、その上のTTLキャッシュされたエバリュエーターを出荷します。[フィーチャー フラグ](feature-flags.md)を参照してください。

### Fillable

信頼されない属性のハッシュから、どのモデルのカラムがマスアサインメントされてよいかを言う、コンパイル時の許可リストです - `#[fillable]` アトリビュートか `Fillable` トレイトを介してモデル構造体上に宣言されます。`#[guarded]` の対です。[Eloquent - マスアサインメント](eloquent.md#mass-assignment)を参照してください。

### ファイルシステム

ストレージサブシステム全体です - `Storage` ファサード、登録済みの[ディスク](#ディスク)、[DiskExt](#diskext)トレイト、ディスクをまたぐストリーミングコピーです。[ファイルシステムとストレージ](filesystem.md)を参照してください。

### フォームリクエスト

ハンドラが走る前にリクエストボディを抽出し検証する、`FormRequest` を実装する（あるいは `#[request]` 経由で導出される）構造体です。Laravelのフォームリクエストクラスに対応する、合成可能で型安全な仕組みです。[バリデーション](validation.md)を参照してください。

### `FrameworkError`

フレームワーク内部のあらゆる失敗が変換される、単一のenumです。5xxのボディをサニタイズしリクエストidを刻印する、独自の `HttpResponse` 射影を運びます（`From<FrameworkError> for HttpResponse`）。[エラー モデル](error-model.md)を参照してください。

## G

### ゲート

認可の入り口です - `Gate::allows("update-post", user, post)`。登録済みのポリシー（`#[policy]` マクロ経由で宣言される）に対して解決し、allow/denyで短絡します。`GateResponse`（認可の `Response` として再エクスポートされます）を返します。[認可](authorization.md)を参照してください。

### グローバルスコープ

明示的に取り除かれるまで（`Builder::without_global_scope`）、すべての `Model::query()` 呼び出しに適用されるクエリ制約です。`GlobalScope` トレイトを介して実装され、bootstrapの中で登録されます。[Eloquent - スコープ](eloquent.md#scopes)を参照してください。

### 認証ガード

リクエストに付けられた、名前付きの認証戦略です - `session`（ステートフル、クッキーバックエンド）、`token`（ステートレス、bearerトークン）です。複数のガードが共存します。`Auth::guard("api")` が1つを選びます。[認証](authentication.md)を参照してください。

### Guarded

モデルのどのカラムがマスアサインメント*できない*かを言う、コンパイル時のブロックリストです。[Fillable](#fillable)の対です。[Eloquent - マスアサインメント](eloquent.md#mass-assignment)を参照してください。

## H

### `HasMany`

一対多のリレーション種別です - 親がローカルキーを持ち、子が外部キーを持ちます。11あるEloquentのリレーション種別の1つです。[Eloquent - リレーションシップ](eloquent.md#relationships)を参照してください。

### `HasManyThrough`

第三の中間モデルを経由して関連モデルへ到達するリレーションです - `Country -> User -> Post`。[Eloquent - リレーションシップ](eloquent.md#relationships)を参照してください。

### `HasOne`

[HasMany](#hasmany)の単一行版の兄弟です - 親がローカルキーを持ち、子が外部キーを持ち、最大でも1行しか返しません。[Eloquent - リレーションシップ](eloquent.md#relationships)を参照してください。

### Hash ファサード

パスワードハッシングの入り口です - `hash(password)`、`verify(password, hash)`。`HASH_DRIVER` を介してbcryptかargon2を選びます。`needs_rehash` は、ログイン時にユーザーをアルゴリズム間で移行させてくれます。[ハッシング](hashing.md)を参照してください。

### ハンドラ

一致したルートに対して `Response` を返す非同期関数です - `#[handler]` マクロによって、フレームワークの型付きハンドラの形に変えられます。ミドルウェアチェーンの内側の端で構成されます。[ルーティング](routing.md)、[コントローラー](controllers.md)を参照してください。

### `HttpError`

ユーザー定義のエラー型が、それがどのようにHTTPレスポンスとしてレンダリングされるべきかを指定するために実装するトレイトです - ステータス、ボディ、ヘッダーです。Laravelの `Renderable` 例外を反映しています。[エラーハンドリング](errors.md)を参照してください。

### `HttpResponse`

ハンドラとミドルウェアが生成する、具体的なHTTPレスポンス型です。ステータスコード、ヘッダー、そしてボディをラップします - 実際にレスポンスへ書き込まれるものです。[レスポンス](responses.md)を参照してください。

## I

### べき等キー

「このキーでリクエストをすでに処理済みなら、ハンドラを再度走らせる代わりに同じレスポンスをリプレイせよ」と言う、クライアントが供給するヘッダーです（`Idempotency-Key`）。リトライしても安全なPOST/PUT/PATCH/DELETEに必須です。Suprnovaは、ハンドラをラップするために `Idempotency`、`Idempotent`、`Replay` を出荷します。[べき等性](idempotency.md)を参照してください。

### Inertiaレスポンス

HTMLの代わりに、型付きのコンポーネント名とシリアライズされたpropsを返すレスポンスです - RustのハンドラとSvelte / React / Vueのページの間の橋渡しです。`Inertia::render(...)` か、`#[derive(InertiaProps)]` マクロと `inertia_response!` を使って構築します。[フロントエンド](frontend.md)、[Inertia レスポンス](frontend-inertia-responses.md)を参照してください。

### `InertiaProps`

Inertiaページのpropsとして使われる構造体のために、`Serialize` の実装とTypeScriptの型メタデータを生成するderiveマクロです。`suprnova generate-types` コマンドを駆動します。[TypeScript 型](frontend-typescript-types.md)を参照してください。

## J

### ジョブ

`Job` トレイトを実装する、シリアライズ可能な構造体です - `handle(self)` メソッドを持ち、`Queue::push(job)`（遅延ディスパッチには `Queue::push_later(job, when)`）を通じてエンキューされます。キュードライバーのストレージに永続化され、ワーカーによって実行されます。[キュー](queues.md)を参照してください。

### ジョブミドルウェア

ジョブの `handle` 呼び出しの周りを走る、合成可能なラッパーです（`WithoutOverlapping`、`RateLimited`、`ThrottlesExceptions`、`Skip`、`FailOnException`、`SkipIfBatchCancelled`）。HTTPミドルウェアのキュー版です。[キュー - ジョブミドルウェア](queues.md#job-middleware)を参照してください。

### `JobOutcome`

ジョブの決着が生成する、判別されたenumです - `Completed`、`Failed`、`Released`、`Deleted`、`Skipped` です。ジョブのライフサイクルイベントとキューのメトリクスカウンターを通じて報告されます。[キュー](queues.md)を参照してください。

## L

### 遅延コレクション

[コレクション](#collection-eloquent)のストリーミング版の相方です - `Model::query().lazy().await` は、すべての行をメモリへロードするのではなく、チャンク単位でデータベースから行を引いてくる `LazyCollection<M>` を返します。[Eloquent - チャンクと遅延反復](eloquent.md#chunking-and-lazy-iteration)を参照してください。

### Length-aware ページネーター

クエリと `COUNT(*)` を実行する古典的な番号付きページのページネーターです（`Builder::paginate(per_page)`）- 総行数を知っています。[Eloquent - ページネーション](eloquent.md#pagination)を参照してください。

### リスナー

イベントハンドラが実装するトレイトです - `Listener<E>::handle(evt)`。`EventDispatcher::listen::<E, _>(arc_listener)` で、あるいは `Subscriber` アグリゲーター経由で登録されます。[イベント](events.md)を参照してください。

### ロックガード（キャッシュ）

`Cache::lock(key, ttl).acquire()` が返す、プロセスをまたぐ相互排他を表すハンドルです - `LockGuard`。ガードを解放するとロックが解放されます。床に落として（dropして）しまった場合はTTLに頼ります。[キャッシュ](cache.md)を参照してください。

### ロック ポリシー

長命なプロセスの中で `std::sync::Mutex` / `std::sync::RwLock` のポイズニングを扱う、プロジェクト全体のポリシーです - 2つの是認されたパターン（map-to-errorかrecover-in-place）で、裸の `.lock().unwrap()` は決して使いません。[ロック ポリシー](lock-policy.md)を参照してください。

## M

### `Mailable`

メールメッセージが実装するトレイトです - `subject`、`to`、`cc`、`bcc`、`view`、添付ファイルです。手書きするか、`#[derive(NotificationMailable)]` マクロ経由で導出されるかのどちらかで、`Mail::to(...).send(MyMail).await` を通じて送信されます。[メール](mail.md)を参照してください。

### メンテナンスモード

許可リストにある人を除く全員に対して、アプリケーションをオフラインにする、リクエスト時の切り替えです - `maintenance_mode().set(payload)`。`FileMaintenanceMode`（デフォルト、番兵ファイル）か `CacheMaintenanceMode`（マルチインスタンスのデプロイ向けのキャッシュバックエンド）に支えられ、`MaintenanceMiddleware` によって配信されます。クレートのルートで再エクスポートされています。

### ミドルウェア

ハンドラの周りの合成可能なラッパーです - 前のリクエストを見て、後のレスポンスを見て、`Err(resp)` を返すことで短絡できます。グローバルに、ルートごとに、あるいはグループごとに登録され、固定された外から内への順序で実行されます。[ミドルウェア](middleware.md)を参照してください。

### モデル

`#[suprnova::model]` でアノテートされた、データベースのテーブルを名指しする構造体です。マクロが展開された後、その構造体自体が SeaORM の `Model` です - Suprnovaはそれをラップしません。`Model` トレイト経由でCRUDを運び、`Model::query()` 経由でクエリを構築し、ファクトリー、キャスト、スコープ、リレーション、オブザーバーを持ちます。[Eloquent](eloquent.md)を参照してください。

### Morph

「polymorphic（多態的）」の略です。morphリレーションは、単一のリレーションが複数のモデル型のうち1つを指すことを可能にします - `MorphTo`（いくつかの可能な型のうちの単一の所有者）、`MorphMany`/`MorphOne`（その逆、morphした子を集める）、`MorphToMany`/`MorphedByMany`（morphした型をまたぐ多対多）です。フレームワークは、判別子の文字列とRustの型の間の `MorphTypeEntry` マッピングのランタイム[レジストリ](#レジストリ)を保持します。[Eloquent - リレーションシップ](eloquent.md#relationships)を参照してください。

### ミューテータ

`#[mutator]` マクロで宣言される、書き込み側の変換です - 値がモデルに保存される前、プロパティがセットされるたびに実行されます。[アクセッサー](#アクセッサー)の対です。[Eloquent - アクセッサーとミューテータ](eloquent.md#accessors-and-mutators)を参照してください。

## N

### Notifiable

（通知を受け取れる、ユーザーあるいは任意のオブジェクトである）ユーザーが実装するトレイトです - `route_for(channel)` は、名指しされたチャネル（メールアドレス、プッシュの購読、ブロードキャストのユーザーidなど）のためのアドレスを返すか、スキップするために `None` を返します。[通知 - Notifiableトレイト](notifications.md#the-notifiable-trait)を参照してください。

### 通知

通知メッセージが実装するトレイトです - `channels()` は、ファンアウトすべきチャネル名のリストを返します。各チャネルは、チャネル固有のペイロードのために通知へコールバックします（`MailRendering` / `DatabaseChannel` のペイロードメソッドのような、チャネルごとのトレイトを介して）。`Notify::send(&user, &notif).await` を通じてディスパッチされます。[通知](notifications.md)を参照してください。

## O

### オブザーバー

Eloquentモデルのライフサイクルイベントをリッスンする、`Observer<M>` を実装する構造体です - `creating`、`created`、`updating`、`updated`、`deleting`、`deleted`、`saving`、`saved`、`retrieved`、`replicating` などです。`#[suprnova::observer(M)]` マクロ経由で登録され、起動時にインベントリから排出されます。[Eloquent - オブザーバーとライフサイクルイベント](eloquent.md#observers-and-lifecycle-events)を参照してください。

### `OriginPolicy`

状態を変更するリクエストにおける `Origin` ヘッダーに対する、CSRFミドルウェアの強制の選択です - `Strict`（ホストと一致しなければならない）、`AllowList`、あるいは `None` です。[CSRF保護](csrf.md)を参照してください。

## P

### ページネーター

`.paginate(...)` 呼び出しの結果です - 3つの風味のうちの1つです。`LengthAwarePaginator`（`COUNT(*)` を伴う番号付きページ）、`Paginator`（next/prev、合計なし）、`CursorPaginator`（動き続ける結果集合に対する安定した反復のための不透明なカーソル）です。3つとも、Laravel形のJSONペイロードにシリアライズされます。[Eloquent - ページネーション](eloquent.md#pagination)を参照してください。

### パニック境界

ミドルウェアチェーンの周り（そして各バックグラウンドワーカーのハンドラの周り）を包む `AssertUnwindSafe(...).catch_unwind()` ラッパーで、未処理のパニックを、サニタイズされた500と、ログに記録された `ErrorOccurred` イベントへ変換します。安全網であって契約ではありません - 公開APIは、それでも `Result` を返すべきです。[リクエスト ライフサイクル - パニック境界](lifecycle.md#5-panic-boundary--execute_chain_safely)を参照してください。

### 支払いプロバイダー

`PaymentProvider` スーパートレイト（= `Checkout` + `Subscription` + `CustomerStore` + `WebhookHandler`）を実装する型です。参照アダプター: `suprnova-payments-stripe`（ゲートウェイ、完全な `Payment` 実装）と `suprnova-payments-paddle`（merchant-of-record、`Payment` なし）です。[支払い](payments.md)、[支払い プロバイダー アダプターの作成](payments-provider-guide.md)を参照してください。

### ピボット

[BelongsToMany](#belongstomany)リレーションにおける中間モデルです - 自分自身の構造体、キャスト、タイムスタンプを持つ、ファーストクラスの `#[suprnova::model]` で、第三の型パラメータとして明示的に名付けられます（`BelongsToMany<L, R, P>`）。Suprnovaは、テーブル名から暗黙のピボットを合成しません。[Eloquent - リレーションシップ](eloquent.md#relationships)を参照してください。

### プレゼンスチャネル

サーバーが現在誰が購読しているかを追跡し、各メンバーのメタデータとともにjoin/leaveイベントを発する、[チャネル](#チャネル-ブロードキャスト)のバリアントです。「誰がオンラインか」インジケーターに便利です。[ブロードキャスト - プレゼンスチャネル](broadcasting.md#presence-channels)を参照してください。

### プライベートチャネル

購読時に認可を要求する、[チャネル](#チャネル-ブロードキャスト)のバリアントです - 購読するユーザーに対して `authorize(...)` がtrueを返さなければなりません。ユーザーごとの通知ストリームに便利です。[ブロードキャスト - チャネル](broadcasting.md#channels)を参照してください。

### Prunable

`model:prune` によるクリーンアップの対象として、ソフトデリートされた（あるいはクエリ可能な）モデルをマークするトレイトです - `Prunable::prunable_query()` が、消えるべき行のビルダーを返します。`MassPrunable` は単一の `DELETE WHERE` で削除します。デフォルトは、オブザーバーが発火するよう行ごとの削除を発行します。`#[prunable]` マクロ経由でレジストリのためにタグ付けされます。[Eloquent - Prunable](eloquent.md#prunable)を参照してください。

## Q

### キュー

バックグラウンド作業のサブシステム全体です - `Queue` ファサード、[ジョブ](#ジョブ)トレイト、[エンベロープ](#エンベロープ-キュー)、ドライバー（memory、sync、redis、database、null）、ワーカー、バッチ、チェーンです。[キュー](queues.md)を参照してください。

### キュードライバー

`QueueDriver`（push、pop、release など）を実装する型です - `MemoryQueueDriver`、`SyncQueueDriver`（インラインで実行）、`RedisQueueDriver`、`DatabaseQueueDriver`、`NullQueueDriver` を出荷します。`QUEUE_DRIVER` を介して起動時に選ばれます。[キュー - ドライバー](queues.md#drivers)を参照してください。

### キューワーカー

キュードライバーからエンベロープを引き出し、ハンドラの周りでジョブミドルウェアを走らせ、結果を報告する、長命なループです。オブザーバーとリスナーが同一に発火するよう、HTTPサーバーと同じライフサイクルを通じて起動します。`cargo run -- queue:work` によって起動されます。[キュー](queues.md)を参照してください。

### キューに入れられたリスナー

呼び出されたときに、イベントペイロードをキューへ永続化し、インプロセスではなくバックグラウンドワーカーの中で `handle` を走らせる `Listener<E>` です。イベントリスナーがディスパッチのパスをブロックすべきでないI/Oを行うときに便利です。`QueuedListener` アダプター経由でラップされます。[イベント](events.md)を参照してください。

## R

### レート リミッター

レートリミットのサブシステム全体です - `RateLimiter`（キャッシュバックエンドのファサード）、`Limit` ビルダー、`SlidingWindowConfig`（スライディングウィンドウドライバー）、`RateLimitMiddleware`（ルートにマウントされる）、`ThrottleRequestsMiddleware`（Laravelの名前を持つエイリアス）、`BackendErrorPolicy`（フェイルオープン対フェイルクローズ）です。[レート リミット](rate-limiting.md)を参照してください。

### リダイレクト

`Location` ヘッダーをラップする、特殊化された[HttpResponse](#httpresponse)です - `Redirect::to(...)`、`Redirect::route(...)`、`Redirect::back()` で構築され、フラッシュデータのための `.with(...)`/`.with_input(...)` チェーンを伴います。[URL 生成](urls.md)、[レスポンス](responses.md)を参照してください。

### レジストリ

`inventory` によってコンパイル時に（`ModelEntry`、`RelationEntry`、`MorphTypeEntry`、`ObserverEntry`、`PrunerEntry`、`TaskEntry`、`PaymentProviderEntry`、`CommandEntry`）、あるいは明示的な登録によって起動時に（`ConnectionRegistry`、`MiddlewareRegistry`、`InertiaRegistry`、`ChannelRegistry`、`VectorRegistry`、`SupervisorRegistry`）埋められる、プロセスグローバルなルックアップです。すべては、起動シーケンスの間に排出されるか問い合わせられます。

### リレーション

すべてのリレーション種別が実装するトレイトです - `BelongsTo`、`HasOne`、`HasMany`、`BelongsToMany`、`HasOneThrough`、`HasManyThrough`、`MorphTo`、`MorphOne`、`MorphMany`、`MorphToMany`、`MorphedByMany` です。モデルは、リレーションの構造体を返すメソッドとして自分のリレーションを宣言します。フレームワークは、トレイトから、イーガーロード、`with(...)`、リレーション存在クエリ、そしてカスケードするtouchを駆動します。[Eloquent - リレーションシップ](eloquent.md#relationships)を参照してください。

### リクエスト

フレームワークの型付きリクエスト構造体です - 根底にあるhyperのリクエストをラップし、`req.param("id")`、`req.json::<T>()`、`req.form_data()`、`req.flash()` などを公開します。`suprnova::Request` として再エクスポートされています。[リクエスト](requests.md)を参照してください。

### `Response`

Suprnovaは `http::Response` を `Result<HttpResponse, HttpResponse>` にバインドします - どちらの腕も `HttpResponse` を運びます。ハンドラの本体は `Response` を返し、`?` で失敗しうる作業を伝播させ、ランタイムは `result.unwrap_or_else(|e| e)` で両方の腕を畳みます。認可の判定型は、衝突を避けるために `GateResponse` として再エクスポートされています。[レスポンス](responses.md)、[リクエスト ライフサイクル](lifecycle.md#the-response-contract)を参照してください。

### リソース

無関係な2つのものが、この名前を共有しています。両方とも出荷されます。

1. **JSON:APIリソース** - モデルをスパースフィールドセットとインクルード付きのJSON:API形式へシリアライズする、`#[derive(Resource)]` 構造体です。[APIリソース](eloquent-resources.md)を参照してください。
2. **リソースルーティング** - `ResourceController` の実装に対して、CRUDの `index`/`show`/`store`/`update`/`destroy` の集合をマウントするルートヘルパーです。[ルーティング](routing.md)を参照してください。

### `routes!` マクロ

ルーティングDSL（`get!("/users", users::index)`、`group!`、`middleware!(Auth)`）を `Router` ファクトリー関数へ展開する、コンパイル時のマクロです。アプリケーションにとっての、ルートの唯一の真実の源です。[ルーティング](routing.md)、[マクロ](macros.md)を参照してください。

## S

### スコープ（ローカル）

`#[scopes(Model)]` マクロでEloquentモデル上に宣言される、再利用可能なクエリの断片です - `Post::query().published().recent().get()`。ローカルスコープはデフォルトではオフです。呼び出されたときにだけ走ります。[グローバルスコープ](#グローバルスコープ)の相方です。[Eloquent - スコープ](eloquent.md#scopes)を参照してください。

### シーダー

`suprnova db:seed` を通じて登録される、データベースに開始データを投入する `Seeder` トレイトを実装する型です。しばしば[ファクトリー](#ファクトリー-eloquent)に支えられます。[Eloquent](eloquent.md)を参照してください。

### 署名付きURL

クエリ文字列がHMAC署名を運ぶURLです（`?signature=...&expires=...`）。それがアプリケーションによって生成され、改ざんされていないことを証明します。`sign_url(...)` / `sign_route(...)` で構築され、ミドルウェアか `verify_signature(...)` 経由で検証されます。[URL 生成 - 署名付きURL](urls.md#signed-urls)を参照してください。

### ソフトデリート

モデルの行を削除することが、`DELETE` を発行する代わりに `deleted_at` タイムスタンプをセットするパターンです。`#[suprnova::model]` アトリビュート上の `soft_deletes = true` でモデルごとにオプトインします。`Model::query()` はゴミ箱行を自動的にフィルタリングして除きます。`with_trashed()` と `only_trashed()` でオプトバックインできます。[Eloquent - 削除とソフトデリート](eloquent.md#deleting-and-soft-deletes)を参照してください。

### `Storage` ファサード

ファイルシステムサブシステムへの入り口です - `Storage::disk("s3")`、`Storage::disk("local")` - [DiskExt](#diskext)の実装を返します。[ファイルシステムとストレージ](filesystem.md)を参照してください。

### Subscriber

1回の呼び出しで多くのリスナーを登録するアグリゲーターです - `Subscriber::subscribe(dispatcher)` を実装し、`EventDispatcher::subscribe(subscriber)` 経由で登録されます。[イベント](events.md)を参照してください。

### スーパーバイザー

`SupervisorRegistry` の下で生きるために、長命なバックグラウンドアクターが実装するトレイトです（`Supervisor::run`）。レジストリは、実行ループの中のパニックを捕まえ、`RestartPolicy` を適用し、再スポーンします。ErlangのSupervisorパターンである `gen_server` のRust版です。[スーパーバイザー](supervisors.md)を参照してください。

## T

### タスク

`Task` トレイトを実装する構造体です - cron式か、より高レベルな頻度（`daily()`、`every_minute()`）を宣言し、スケジューラー上で走ります。コンパイル時に `TaskEntry` インベントリを介して発見されます。[タスク スケジューリング](scheduling.md)を参照してください。

### 終了処理ミドルウェア

レスポンスがクライアントへ書き込まれた*後*に走るフックを登録するミドルウェアです - `Terminable` トレイトを介して実装され、`TerminationSnapshot` へキャプチャされ、`dispatch_termination` によってディスパッチされます。ロギング、メトリクスの書き出し、事後の監査に便利です。[ミドルウェア - 終了処理ミドルウェア](middleware.md#terminable-middleware-post-response-hooks)を参照してください。

### Through（リレーション）

第三の中間モデルを経由するリレーションです - [HasManyThrough](#hasmanythrough)と `HasOneThrough` です。[Eloquent - リレーションシップ](eloquent.md#relationships)を参照してください。

### タイムアウト

単一のリクエストの壁時計時間に上限を設け、上限を超えたときに504を返すミドルウェアです - `TimeoutMiddleware`。キューワーカーのタイムアウト（キュー側の `TimeoutExceeded`）や、HTTPクライアントのタイムアウトとは別物です。[リクエスト タイムアウト](timeout.md)を参照してください。

### `TypedCommand`

コンソール側のトレイトです - `#[derive(Command)]` 構造体によって実装されます - コンソールコマンドに、（`clap` 経由の）型付きの引数と非同期の `handle(self)` メソッドを与えます。コンパイル時に `CommandEntry` インベントリへ登録されます。[コンソール](console.md)を参照してください。

## U

### `UserId`

`Auth::id()` が返す不透明な文字列識別子です。フレームワークのガード/プロバイダー経路は、設定済みの `UserProvider` が使う安定したキーを運びます。`EloquentUserProvider<User>` では、通常は文字列化された主キーです。Magnetarのファサードは `UserId` ニュータイプを公開しますが、その値をフレームワークのセッション状態へ書き込む前に、アプリケーションの正規ユーザーIDへ戻してバインドします。リクエスト境界を文字列形状のままにすることで、数値ID、UUID、プロバイダー非依存の不透明なIDが、同じミドルウェアとイベントの契約を使用できます。[認証](authentication.md)を参照してください。

## V

### VAPID

Voluntary Application Server Identificationです - web-pushの送信者を識別するためのIETF仕様です。Suprnovaは、各プッシュリクエストに署名する `VapidKey`、`VapidSigner`、`VapidClaims`、そして `WebPushClient` を出荷します。[Web プッシュ](web-push.md)を参照してください。

### `Vector` ファサード

ベクトル検索サブシステムへの入り口です - `Vector::driver("qdrant").await?.upsert(...)`。`VectorDriver` の実装に支えられます: インメモリ、Qdrant、Pinecone（フィーチャーゲート付き）、MariaDBネイティブです。[ベクトル検索](vector.md)を参照してください。

### `VectorDriver`

すべてのベクトルバックエンドが実装するトレイトです - `upsert`、`search`、`delete`、`count`。フレームワークが、1つを強制することなく複数のベクトルDBをサポートできるようにします。[ベクトル検索](vector.md)を参照してください。

## W

### Web プッシュ

ウェブプラットフォームのプッシュ通知プロトコルです - 暗号化されたペイロードが、ユーザーエージェントのプッシュサービスを通じて配信されます。Suprnovaは、`WebPushClient`（VAPID署名者、retry-afterのパース、8 KiBの拒否上限）と、[通知](#通知)配信のための `WebPushChannel` を出荷します。[Web プッシュ](web-push.md)を参照してください。

### Webhook

第三者（支払いプロバイダー、IDプロバイダー、…）があなたのアプリケーションへイベントを報告するために送る、HTTPリクエストです。Suprnovaは、デフォルトですべてのwebhookをべき等として扱います - プロバイダーのアダプターは `WebhookHandler::verify(...)` を実装し、リプレイを拒否する `UNIQUE` 制約の中にプロバイダーのイベントidを保存します。[支払い - Webhookの取り扱い](payments.md#webhook-handling)、[べき等性](idempotency.md)を参照してください。

### ワークフロー

型付きのステップで構成される、長時間実行のステートフルなバックグラウンド作業です - `#[workflow]` と `#[workflow_step]` マクロです。各ステップの戻り値は永続化されるため、ワークフローの途中でのワーカーの再起動は、最後に完了したステップから再開します。単一の[ジョブ](#ジョブ)に収まらない、複数ステップのバックグラウンドプロセスに対するSuprnovaの答えです。[ワークフロー](workflows.md)を参照してください。

### `WsConfig`

ルートごとのWebSocket設定です - ペイロードサイズの上限（デフォルトはテキスト1 MiB / バイナリ64 KiB）、最大フレームサイズ、ping間隔、アイドルタイムアウト、オリジンポリシーです。`ws!()` ルートによって使われます。[WebSocket](websockets.md)を参照してください。

### `WsSocket`

`ws!()` ハンドラに渡される、フレームワークの型付きWebSocketハンドルです。`WsSocket::split()` 経由で、`Sink`（送信）と `Stream`（受信）の半分に分割されます。ping/pongは、`AbortHandle` を持つハートビートタスクによって管理されるため、ドロップされたハンドラは常にきれいに解体されます。[WebSocket](websockets.md)を参照してください。

## 次のステップ

- [Laravel パリティ マップ](parity.md) - Laravel 13との機能ごとの比較
- [環境変数](env-vars.md) - フレームワークが読み込むすべての `env!`
- [ドキュメント インデックス](documentation.md) - チャプターマップ
