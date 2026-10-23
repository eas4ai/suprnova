# はじめに

Suprnova は、Rust 用のウェブフレームワークであり、Tokio 上で Laravel の開発者体験を提供します。コントローラーと Eloquent 形式のモデルを書きます。フレームワークは並行処理、型安全性、単一バイナリのデプロイを提供します。

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0");
    json_response!({ "id": id, "name": "Alice" })
}
```

```rust
use suprnova::{model, Model};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// そして、どこからでも:
let user = User::find(42).await?;
let admins = User::query().db_where("role", "admin").get().await?;
let alice = User::create(attrs!{ name: "Alice", email: "alice@x.com" }).await?;
```

先週 Laravel でこのコードを書いた場合、上記の Rust 版は同じように感じるでしょう。チェーンの形状、メソッド名、デフォルト値が同じです。違いは内部で何が起こるかです。FPM の代わりに Tokio、PHP ランタイムの代わりに単一バイナリ、すべてのカラムでコンパイル時の型チェックです。

## Suprnova が存在する理由

Laravel はバックエンド Web 開発の生産性の問題を解決しました。このパターンは機能します。10 年の改善を経て、実際の製品を構築するときに、ほとんど何も邪魔になりません。しかし、PHP のリクエストごとのプロセスモデルは 2 つのことを手の届かない状態にします。低コストの長寿命接続（WebSocket、SSE、ポーリングなしのサーバープッシュ通知）と、1 つのリクエストハンドラ内での簡単な並行 I/O です。

Rust は Tokio で両方を無料で提供します。問題は、Rust Web エコシステムが生産性レイヤーを自分で構築させることです。HTTP クレートを選び、ORM を選び、マイグレーションツールを選び、キューを選び、すべてをまとめて、独自の規約を設計します。各アプリは、Laravel がすでに標準化したものを再発明します。

Suprnova は、Laravel の規約を Tokio にコピーしたときに起こることです。得られるもの：

- **同じ表面** - `routes!`、`Auth::user()`、`Cache::remember`、`Mail::send`、`Queue::push`、`Storage::disk("s3")`、`Notify::send`、`Schedule::call`、`Gate::allows`、Eloquent クエリビルダー、ソフトデリート、ファクトリー、オブザーバー、ブロードキャスト、すべてです
- **異なるエンジン** - 完全非同期、長寿命接続がファーストクラス、単一の静的リンクバイナリ、プリフォーキングなし、オペコードキャッシュなし、FPM なし
- **型安全性** - モデル、ルート、イベントペイロードはコンパイル時にチェックされます。破壊的なリファクタリングはステージングに到達しません
- **本物のフロントエンドストーリー** - Inertia.js は Svelte 5、React 19、または Vue 3.5 スターターにブリッジします。保守する別の API はありません

## 設計原則

これらはフレームワークの著者が自分たちに課す原則です。章が何を言っているかについて説明します。

**1. パリティは Laravel の変更履歴から来ます。** Laravel が機能をリリースすると、Suprnova はそれを追跡します。現在のベースラインは Laravel 13.x であり、すべてのリリースされたサブシステムは監査されています。[Laravel パリティ マップ](parity.md)は明確な機能ごとのテーブルです。

**2. Rust がより良いものにする場合は意図的に異なります。** Laravel が PHP 形の選択をし、Rust では選択する必要がない場合、Suprnova は Rust 形の選択をして、そのように言います。最大の例は並行処理です。WebSocket、ブロードキャスト、バックグラウンドワーカー、HTTP/2 サーバープッシュはファーストクラスであり、後付けではありません。章で呼び出されていることを見たら、**「Suprnovaが異なる設計を選んだ理由」**ボックスを探してください。

**3. ゲートキーピングなし。** Laravel は一部の機能を 1 つのバックエンドに制限します（例えば Postgres `pgvector` 経由のベクトル検索）。Suprnova はバックエンドをドライバーとして扱います - `Vector::driver("qdrant")`、`Vector::driver("pinecone")`、`Vector::driver("mariadb")`、`Cache::driver("redis")`、`Mail::driver("ses")`。適切なツールを選びます。私たちはあなたのために選びません。

**4. Suprnova はAPI 表面です。** 内部では SeaORM、hyper、Tokio、serde、sqlx、validator、lettre、その他多数を使用します。それらはコードに表示されるべきではありません。`suprnova::*`に依存します。タッチするすべてのもの - SeaORM の `Entity`、`Column`、`ActiveModel`、`QueryFilter` など - をフレームワークルートの下で再エクスポートします。エスケープハッチ（`use suprnova::sea_orm;`）はキュレーションされた表面がカバーしないまれなケースに存在しますが、ほぼ必要ありません。

## ボックスの中身

詳細ではないマッピング。完全なリストは [`documentation.md`](documentation.md) にあります。

| 領域 | 含まれるもの |
|---|---|
| **HTTP** | `routes!` マクロ、コントローラー、ミドルウェア、リクエスト、レスポンス、ルートモデルバインディング、署名付き URL、リソースルーティング、リダイレクトヘルパー、CORS、CSRF、べき等性キー、タイムアウト、レート制限、パニック回復付き構造化エラー |
| **データベース** | 内部的に SeaORM、マルチドライバー（Postgres、MySQL、MariaDB、SQLite）、マイグレーション、シーダー、クエリビルダー、セーブポイント付きトランザクション、マルチコネクション読み書き分割 |
| **Eloquent** | `#[suprnova::model]` マクロ、11 種類すべてのリレーション、イーガーロード、ソフトデリート、刈り取り可能、スコープ（ローカル＋グローバル）、16 個のライフサイクルイベント、オブザーバー、22 個の組み込みキャスト、アクセッサー/ミューテータ、3 つのページネーター、チャンク/遅延/カーソル反復、コレクション、レプリケーション |
| **認証** | フレームワークのガード、ミドルウェア、プロバイダー、ブラウザセッション。Magnetar対応のパスワード、パスキー、マジックリンク、OAuth、bearerセッション、ロックアウト、remember、auth-epoch、マイグレーションエンジン。プロバイダー対応のメール検証。フレームワークTOTP互換ファサード。ポリシーマクロとゲート |
| **フロントエンド** | Inertia v3 ブリッジ、Svelte 5 / React 19 / Vue 3.5 スターターテンプレート、型付き `#[derive(InertiaProps)]`、部分的なリロード、自動 TypeScript 型生成 |
| **バックグラウンド** | メモリ/同期/Redis/データベース/null ドライバー付きキュー、バッチ、チェーン、ジョブミドルウェア、失敗したジョブストア、`#[command]`/`#[derive(Command)]` コンソールバイナリ、`Task` トレイトスケジューラー、`#[workflow]` 長時間実行ステートフルワーク、パニックキャッチ自動再起動付き `Supervisor` トレイト、コマンドバス、イベントディスパッチャー |
| **リアルタイム** | 型付き WebSocket ハンドラ用 `ws!()` マクロ、ブロードキャストチャネル（パブリック、プライベート、プレゼンス）、sea-streamer ファンアウト、サーバー送信イベント、Web プッシュ（VAPID） |
| **キャッシュ & ストレージ** | メモリ、Redis、データベースキャッシュドライバー、アトミック操作、タグ付きキャッシュ、キャッシュロック、fs/メモリ/s3/azblob/gcs ドライバー付きファイルシステム、パストトラバーサル保護、複数のバックエンド付きベクトルストレージ |
| **メール & 通知** | `Mailable` トレイト、SMTP/SES/Mailgun/Postmark/SendGrid/Resendドライバー、RFC 5322ファイルプレビュー、メモリ内/ログトランスポート、メール/データベース/ブロードキャスト/webpushチャネル付き `Notifiable` |
| **バリデーション & データ** | `#[derive(Validate)]`、フォームリクエスト、非同期バリデーション、部分的なリロード include セット用 `#[derive(Data)]`、JSON:API 用 `#[derive(Resource)]` |
| **支払い** | ジェネリックプロバイダー表面（ゲートウェイ/MoR/リダイレクトフロー）、Stripe と Paddle の参照アダプター、Webhook べき等性付きミラーテーブル、Inertia チェックアウトコンポーネント |
| **フィーチャーフラグ** | データベース評価器、TTL 付きキャッシュ評価器、フィーチャーミドルウェア、同期トレイト経由のサブ秒伝播 |
| **テスト** | `#[suprnova_test]`、`expect!`、`TestDatabase`、すべての外部表面（メール、通知、キュー、バス、イベント、ストレージ、Http）のフェイク |
| **CLI** | `suprnova new` スキャフォルダー（Svelte/React/Vue）、`serve` 開発ランナー、`migrate*`、`db:sync`、`db:seed`、`make:*` ジェネレーター、`model:prune`、プロジェクトごとのコンソールバイナリ |

## 本番環境対応

フレームワークは本番運用水準の機能範囲を備え、テストされています。現在のHEAD時点では:

- 30 個の文書化されたドメイン全体で、すべての Laravel 13.x 表面がリリースされています
- 独立したコード審査によって引き起こされたすべての問題が解決されています
- ワークスペーステストスイートがすべての変更で合格しています
- `framework/src/lib.rs` のすべてのパブリック API が文書化されています - 文書化されていないパブリック項目はビルドに失敗します

**v1.0.0** 時点では、パブリック API は安定しています。アプリはリリースタグをピンします（`tag = "v<version>"` - タグがリリース、crates.io パブリッシュなし）。破壊的変更は、[CHANGELOG](changelog.md) セクションがそのように言うバージョンバンプの後ろにのみ来ます。

## 読む道筋を選ぶ

| あなたは… | 次から始めます |
|---|---|
| Laravel 開発者 | [Laravel から](from-laravel.md) |
| Axum/Actix/Rocket を使用した Rust 開発者 | [Rust のウェブから](from-rust-web.md) |
| 両方、またはどちらでもなく、構築したいだけ | [インストール](installation.md) → [クイックスタート](quickstart.md) |
| 特定の機能を探している | [`documentation.md`](documentation.md)（マスター TOC） |
| 「Suprnova は X を持っていますか？」と疑問に思っている | [Laravel パリティ マップ](parity.md) |
