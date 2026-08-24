# Rust のウェブから

Axum、Actix、Rocket、または手作りの hyper で Rust サービスをリリースした経験があります。言語とランタイムを理解しています。では、Suprnova は実際のところ何をもたらすのでしょうか。

**生産性レイヤーです。** ルーティング、コントローラー、ORM、マイグレーション、キュー、スケジューリング、認証、メール、通知、ブロードキャスト、キャッシュ、ストレージ、バリデーション、型付きフロントエンドブリッジ - すべて連携した、同じコンベンションを使う、本番環境対応のものです。コントローラーとモデルを書きます。レイアウトは選びません。

Axum で本当のアプリを 1 つか 2 つ構築した経験があれば、そのすべての努力がいかに配線作業であって機能ではなかったかを知っています。Suprnova はその配線で、一度だけ、意見が問題になるところは意見が強く、そうでないところはプラグイン可能です。

## 30 秒のまとめ

```bash
suprnova new myapp --frontend svelte    # バックエンド + SPA + Vite をスキャフォルド
cd myapp
suprnova db:sync                        # マイグレーション実行、エンティティ再生成
suprnova serve                          # バックエンド + Vite 開発サーバー
```

これで以下が手に入ります：

- HTTP/1.1 と HTTP/2、WebSocket アップグレード、グレースフルシャットダウンを備えた hyper サーバー
- リレーション、イーガーローディング、ソフトデリートを持つ SeaORM 対応 Eloquent レイヤー
- Rust → Svelte 5 を型付き `#[derive(InertiaProps)]` で橋渡しする Inertia.js
- フレームワークのガードとミドルウェアに加え、Magnetar対応のパスワード、パスキー、マジックリンク、OAuth、bearerセッション、ロックアウト、rememberエンジンを備えた認証
- memory/sync/redis/database/null ドライバーを持つキュー
- `Task` トレイトで駆動するクーロンスケジューラー
- プロジェクトごとのコンソールバイナリ （`cargo run --bin console <cmd>` 用）
- キャッシュ、ストレージ (fs/s3/azblob/gcs)、メール （SMTP + 5 つのプロバイダー: SES、Mailgun、Postmark、SendGrid、Resend）、Web プッシュ
- プラグイン可能なハブ経由のブロードキャスト （デフォルトは sea-streamer）
- バリデーション、CSRF、CORS、レート制限、べき等性、リクエストタイムアウト、構造化されたエラー

そして `cargo build --release` の最後に、1 つの静的リンクバイナリです。

## 下層に何があるか

| 機能 | クレート |
|---|---|
| HTTP サーバー | `hyper` + tower-ish ミドルウェア （独自実装） |
| 非同期ランタイム | `tokio` |
| ルーター | `matchit` |
| ORM | `sea-orm` （`suprnova::sea_orm` として再エクスポート） |
| マイグレーション | `sea-orm-migration` |
| データベースドライバー | `sqlx` (postgres / mysql / mariadb / sqlite) |
| シリアライゼーション | `serde` / `serde_json` |
| バリデーション | `validator` |
| ブラウザセッション | フレームワークの `SessionMiddleware` とプラグイン可能なセッションストア |
| 認証エンジン | フレームワーク所有のファサードの背後にある `suprnova-magnetar` |
| テンプレート | `tera` （メール本文用。フロントエンドは Inertia） |
| 暗号 | `aes-gcm`、`argon2`、`bcrypt` |
| WebSocket | `hyper-tungstenite` |
| ストリーミング | `sea-streamer` （ブロードキャストファンアウトバックエンド） |
| OAuth | Magnetarのプロバイダーレジストリとセレモニーエンジン |
| トレーシング | `tracing` + `tracing-subscriber` |

通常、これらのいずれにも直接手を出すことはありません - Suprnova は必要なものを再エクスポートしています。SeaORM が最も深いパススルーです: `Entity`、`Column`、`ActiveModel`、`ConnectionTrait`、クエリビルダー、マイグレーションプリリュード。エスケープハッチは、キュレーションされたサーフェスがカバーしていないものが必要な場合、`use suprnova::sea_orm;` です。

## Suprnova が生の Axum に追加するもの

Axum は素晴らしいです。Actix も同じです。Rocket も同じです。Suprnova が存在する理由は、それらのフレームワークが悪いからではなく、それらの上で本当の製品を構築しているすべてのチームが、同じ生産性レイヤーを再実装することになるからです。Suprnova はそのレイヤーを出荷しています：

| 機能 | Axum で手作り | Suprnova で |
|---|---|---|
| 数百のルートまでスケールするルーティングマクロ | ビルダー API、うるさくなりがち | `routes!` マクロでグループ化、プリフィックス、ミドルウェア、命名 |
| ルートモデルバインディング （パス id → ロード済みモデル） | 型ごとのカスタムエクストラクター | `#[handler]` が `{id}` から `post::Model` を自動的に解決 |
| Eloquent スタイルのチェーン可能なクエリビルダー | SeaORM を直接使用 | `Post::query().db_where(...).order_by(...).get().await?` |
| ソフトデリート、オブザーバー、ライフサイクルイベント | モデルごとに構築 | `#[model(soft_deletes)] + impl Observer<Post>` |
| マイグレーション + エンティティ生成 | sea-orm-cli + スクリプトを配線 | `suprnova db:sync` がマイグレーション実行、エンティティ再生成 |
| 認証 （セッション、プロバイダー、認証ガード） | tower-sessions + 独自ロジックを縫い合わせ | `Auth::attempt`、`Auth::user`、ルートごとに `.middleware(AuthMiddleware)` |
| メール確認、パスワードリセット、2FA、ブルートフォース対策 | 4 つすべてを手作り | すべて組み込み、設定可能、べき等 |
| バックグラウンドキュー | ドライバーを選択、ワーカーを記述 | `Queue::push` + `cargo run -- queue:work` |
| クーロンスケジューリング | `tokio_cron_scheduler` で tokio タスクを記述 | `impl Task` + `Schedule::task(...).daily().at("03:00")` |
| Inertia ブリッジ | エクストラクター + JS アダプターを構築 | `inertia_response!(&req, "Page", props)` |
| 型付きフロントエンドプロップス (Rust → TS) | ジェネレーター生成 | `#[derive(InertiaProps)]` + `suprnova generate-types` |
| ブロードキャスト （パブリック / プライベート / プレゼンスチャネル） | ストリーミングバックエンド + 認証を配線 | `BroadcastHub` + `Channel`/`PrivateChannel`/`PresenceChannel` トレイト |
| 複数プロバイダーとのメール | 1 つを選択、独自抽象化を記述 | `Mail::driver("ses")` など、統一 `Mailable` API |
| Web プッシュ | スペックを読む、通知機能を構築 | `WebPushChannel` が付属、VAPID 組み込み |
| バリデーション + フォームリクエスト | `validator` + カスタムエクストラクター使用 | `#[derive(Data, Validate)]` フォームリクエスト、非同期バリデーション |
| JSON:API リソース | 手作業でレスポンスを形成 | `#[derive(Resource)]` |
| フェイルオープン/クローズドポリシーを伴うレート制限 | 構築 | `RateLimiter` + `BackendErrorPolicy` |
| べき等キー | 構築 | Stripe スタイルリプレイの `Idempotency::remember(key, ttl, body)` |
| CSRF （Laravel スタイルグロブ除外付き） | 構築 | `CsrfMiddleware` with `except` + `except_method` |
| サニタイズされた 5xx を伴う構造化エラー | 構築 | `FrameworkError` / `HttpError` トレイト、パニック回復 |
| タスクローカル → スレッドローカル → グローバルスコープのコンテナ | 独自実装 | `App::bind` / `singleton` / `factory` と適切な分離 |
| ヘルスエンドポイント、リクエスト ID、構造化ロギング | グルーコード結合 | すべてデフォルトで有効 |

トレードオフは意見です: Suprnova はレイアウトを選び、デフォルトドライバーを選び、命名規則を選びます。逸脱できます （ドライバーはプラグイン可能、設定は上書き可能、コンテナはサービス入れ替えを許可）。しかし、デフォルトは「素早く製品を構築する」ための正しい選択になるよう設計されています。

## 馴染みのある Rust パターン

以下の形を認識するでしょう：

```rust
// ハンドラは `Result<HttpResponse, HttpResponse>`（エイリアス Response）を返します。
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    Ok(HttpResponse::json(serde_json::json!({ "post": post })))
}

// ミドルウェアはトレイトであり、クロージャーではありません：
#[async_trait]
impl Middleware for RequireAdmin {
    async fn handle(&self, req: Request, next: Next) -> Response {
        let user = Auth::user_as::<User>().await?
            .ok_or_else(|| HttpResponse::text("Unauthorized").status(401))?;
        if !user.is_admin {
            return Err(HttpResponse::text("Forbidden").status(403));
        }
        next(req).await
    }
}

// バックグラウンドワークは `Job` トレイト - `handle(self)` がジョブを実行します：
#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}
```

Tower ミドルウェアに慣れている場合：Suprnova ミドルウェアは概念的に同じ （`next` の周りのラッパー） ですが、独自トレイトを使用します （Tower の `Service` ではなく）。理由は、tower のコンビネータータイプは、アプリケーション固有のエクストラクターを入れ子にし始めると複雑になってしまうからです。形はより単純で、メンタルモデルは変わりません。

Axum のエクストラクターパターンを使用したことがある場合：Suprnova の `#[handler]` マクロは同じ役割を果たしますが、トレイトではなくサービスコンテナを経由して解決するため、リクエストデータだけでなくアプリサービスも注入できます。ルートモデルバインディング （`{id}` からの `Post`） は組み込みです。

`sqlx` を直接使用した場合：Suprnova の ORM は SeaORM の上に乗り、SeaORM は sqlx の上に乗っています。`DB::select(...)` / `DB::select_one(...)` で生の SQL にドロップできます。あるいは `DB::table("name")` を動的行チェーンクエリに使用できます。Eloquent サーフェスがカバーしていないもの （例：カスタム結果マッピング付きの生 `Statement` クエリ） のために、SeaORM に完全にドロップできます。[Eloquent チャプター](eloquent.md)がエスケープハッチをカバーしています。

## 生産性の差は何か

生の Axum で以前構築した機能を選びます。Suprnova はそれをチャプターとして出荷しています：

- **「認証システムを一度構築し、2 週間かかった」** →
  [認証](authentication.md) + [認証フロー](auth-flows.md)。マイグレーションを設定、認証ガードを設定すれば終わりです。
- **「再試行/バックオフ付きのキューワーカーを自分で書いた」** →
  [キュー](queues.md)。`Queue::push` + `cargo run -- queue:work`。
- **「hyper-tungstenite で WebSocket を配線した」** →
  [WebSocket](websockets.md)。`ws!()` マクロはハンドラを型付けします。アップグレード、ping/pong ハートビート、クローズフレームハンドシェイク、バックプレッシャーはすべて処理されています。
- **「Inertia アダプターをゼロから構築した」** →
  [Inertia](frontend.md)。`inertia_response!(&req, "Page", props)`。`InertiaProps` が TS 型生成。
- **「テナント単位のレート制限装置を構築した」** →
  [レート制限](rate-limiting.md)。設定可能なキー、フェイルオープン vs フェイルクローズドポリシー、フェイルクローズドは 503 を返します。
- **「Stripe ウェブフック署名確認 + リプレイ保護を実装した」** →
  [支払い: Stripe](payments-stripe.md)。アダプターに組み込み、ウェブフックは UNIQUE べき等性を持つミラーテーブルに入ります。

手作業で 2 週間かけて構築するものを、1 行でインポートします。

## あなたがまだ「あなた自身の」と認識するもの

言語があなたにフレームワークの抽象化よりも何か優れたものを与えるために、いくつかのことは生の Rust に近くとどまります：

- **並行処理プリミティブ。** `tokio::spawn`、`Arc`、`Mutex`、チャネル - それらを使用してください。フレームワークはそれらを包みません。
- **エラー型。** ドメインエラーを定義します。`HttpError` トレイトを実装して、適切なステータスコード + メッセージをレスポンスで取得してください。フレームワークの `FrameworkError` と `AppError` は横断的 + アドホックエラーのエスケープハッチです。
- **カスタムドライバー。** キャッシュ、キュー、メール、ブロードキャスト、ベクトル、支払い - すべての「ドライバーレジストリー」サブシステムはカスタムドライバーを受け入れます。トレイトを実装、`bootstrap.rs` に登録、終了です。
- **必要に応じて生の SQL。** `DB::select(...)`、`DB::table(...).get()` 動的行用、または完全に SeaORM にドロップ。ORM は邪魔になりません。
- **独自の tower ミドルウェア?** Suprnova は Tower アダプターを出荷していません - ここのミドルウェアは `impl Middleware`。`tower::Service` ではありません。Tower のみのクレートを持ち込む必要がある場合、手作業で適応させます。実際には、組み込みミドルウェアシステムはほぼ何でもカバーしています。[ミドルウェア](middleware.md)を参照。

## 何を諦めるか

正直さはマーケティングより重要です：

- **コンベンション。** モデルはここに、コントローラーはそこに、マイグレーションはそこに、オブザーバーはそこに。スキャフォルダーが決めます。それと戦えます。おそらく戦わないべきです。コンベンションは Laravel のもの、監査され、実戦テスト済みです。
- **リクエストがどのように流れるかの柔軟性の一部。** ミドルウェアチェーンは最外側の順序が固定 （request-id → globals → ルートミドルウェア → ハンドラ）。ミドルウェアはどこにでも挿入できますが、request-id やパニック回復レイヤーは動かせません - それらは不変です。
- **PHP から引き継いだ設計。** Laravel が PHP のために何かをするところ、Suprnova は代わりに Rust 流のやり方をします - ただし、その時は明記してあります。チャプターで **「Suprnovaが異なる設計を選んだ理由」** というコールアウトを探してください。

## なぜ「Laravel インスパイア」がたとえ PHP を書いたことがなくてもあなたに関係あるのか

Rust ウェブエコシステムは大体 2009 年頃の PHP と同じような段階にあります。クレートは存在します。パターンは存在しません。Suprnova は、10 年以上の本番プレッシャーで形作られたフレームワークから、極めて洗練されたパターンのセットをポーティングしています。得られるパターンは、既に現実との接触に耐え抜いたものです。

コストは、Suprnova は **意見が強い** ことです。最小の「自分の選んだもの」フレームワークが必要な場合、Axum がすぐそこにありますし、素晴らしいです。「物事を決めるフレームワークで製品に集中できる」ことが必要な場合、それが Suprnova です。

## 次のステップ

- [インストール](installation.md) - `suprnova new`、スキャフォルドされるもの
- [クイックスタート](quickstart.md) - 5 分で小さいアプリを構築
- [リクエスト ライフサイクル](lifecycle.md) - リクエストがどのように流れるか、何がどこで実行されるか
- [サービス コンテナ](container.md) - サービスがどのように束縛・解決されるか
- [Eloquent](eloquent.md) - 最長チャプター。サーフェスは広い

または [`documentation.md`](documentation.md) 経由でどこへでもジャンプ。
