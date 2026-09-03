# Live

Suprnova Live は、フレームワークのサーバー駆動インタラクションエンジンです。Live
コンポーネントは、状態がサーバー上にあり、ビューが Askama テンプレートで、アクションが
署名付きプロトコルを介して小さなブラウザーランタイムから実行される Rust の構造体です。
ランタイムは再レンダリングされた HTML をその場でモーフします。同期を保つべき
クライアント側の状態モデルはなく、同梱ランタイムを使うためにインストールする
ビルドツールもなく、ドキュメントにインライン JavaScript もありません。

この章では、アプリケーション側の表面を扱います。コンポーネントの作成、登録、
ドキュメントとアイランドの配信、すべての Live リクエストが越えるセキュリティ境界、
アップロード、非同期更新、アセット、テスト、診断、そして復旧です。ここで使うのは
`suprnova::live` と `suprnova::view` だけです。

## クイックスタート

`suprnova new` で作成したプロジェクトは Live に対応済みです。空のコンポーネント
レジストリと `routes()` 関数を持つ `src/live/mod.rs` を含み、ブートストラップが
レジストリをバインドし、`cmd/main.rs` がルートをインストールします。コンポーネントを
生成して、確認します:

```bash
suprnova live:make Counter
suprnova live:check
```

`live:make` は `src/live/counter.rs` と `templates/live/counter.html` を書き出し、
`src/live/mod.rs` にコンポーネントを登録し、次の手順を表示します。`live:check` は
アプリケーションをビルドし、登録済みのすべてのビューを統合チェッカーで証明します。

## コンポーネントを書く

```rust
use suprnova::live::{LiveComponent, live};

/// A counter rendered by `live/counter.html`.
#[derive(LiveComponent)]
#[live(name = "app.counter", view = "live/counter.html")]
pub struct Counter {
    /// Current count, exposed to the view.
    #[public]
    count: u64,
}

#[live]
impl Counter {
    /// Increments the counter in response to `live:click="increment"`.
    #[action]
    pub fn increment(&mut self) {
        self.count += 1;
    }
}
```

- `name` は登録されるコンポーネント名です。`app.counter` のようなドットで区切った
  ケバブケースの名前を使います。CLI は `<package>.<kebab>` を導出します。
- `view` はテンプレートルートからの相対的なテンプレート識別子です。
- `#[public]` フィールドはレンダリングされ、署名付きスナップショットに含まれます。
  `#[model]` フィールドはさらに `live:model` を通じてブラウザーからの提案を受け付けます。
- `#[action]` メソッドは、ブラウザーが呼び出せる唯一のエントリーポイントです。
  検証済みの引数を受け取り、リダイレクトやフラッシュなどの型付き結果を返せます。

すべてのフィールド型は `Default` を実装する必要があります。新しいアイランドは、
マウントフックが別に指定しない限り、これらの既定値から始まります。

## ビュー

ビューは Askama テンプレートです。テンプレートルートは、`askama.toml` が別の
ディレクトリを指定しない限り `templates/` なので、`live/counter.html` は
`templates/live/counter.html` に置きます:

```html
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

ディレクティブは閉じた `live:` 文法を使います。`live:click`、`live:submit`、
`live:model`、`live:upload`、`live:key`、`live:loading`、およびドキュメント化された
残りの集合です。チェッカーはすべてのディレクティブをコンポーネントに対して証明します。
未知のアクション、未知のモデルフィールド、生の `safe` フィルター、または
アクセシビリティ違反は、ファイル、行、列を示して `live:check` を失敗させます。

アイランドを配置するドキュメントは、`#[suprnova::view]` で宣言する通常のビューです。
それらが受け付ける唯一のエスケープされない値は、`trusted_html` フィルターを通した
`TrustedHtml` です。

## 登録とブートストラップ

`src/live/mod.rs` がレジストリとルートを所有します:

```rust
use suprnova::live::{LiveRegistry, RegistryError};

pub mod counter;

/// Builds the registry of every Live component in this application.
pub fn registry() -> Result<LiveRegistry, RegistryError> {
    let registry = LiveRegistry::builder()
        .register::<counter::Counter>()?
        .build();
    Ok(registry)
}
```

サーバー、ワーカー、そして `suprnova live:*` コマンドが同じコンポーネントを見るように、
ブートストラップ中にバインドします:

```rust
suprnova::App::singleton(crate::live::registry().expect("Live component registry"));
```

ランタイムが組み立てられた後、レジストリは不変です。重複したコンポーネント名や
ビュー、あるいは検証ポートなしで検証を必要とするアクションを持つコンポーネントは、
型付きの `RegistryError` で登録に失敗します。

## ルート

`Router::try_live()` は予約済み名前空間を正確に一度インストールします。
`/__live/v1/action`、`/__live/v1/upload`、`/__live/v1/async/*` の制御ルートと
WebSocket ハンドシェイク、そして不変の `/__live/v1/assets/*` ルートです。
アプリケーションルートが `/__live` を要求できる場合、起動は失敗します。

予約済みのリクエストルートは厳格なポリシーを持ちます。すべてのリクエストには
セッション、オリジン、CSRF、プリンシパル、テナント、レート制限の事実が必要です。
フレームワークはセッションと CSRF の証明を記録し、アプリケーションは残りを
ルートガードで取り付けます:

```rust
use std::sync::Arc;
use std::time::Duration;

use suprnova::live::{LiveTenantMiddleware, LiveTenantResolver};
use suprnova::rate_limit::memory::InMemoryRateLimiter;
use suprnova::{AuthMiddleware, FrameworkError, RateLimitMiddleware, Request, Router, SlidingWindowConfig, async_trait};

pub fn routes(router: Router) -> Result<Router, FrameworkError> {
    let limiter = Arc::new(InMemoryRateLimiter::new());
    router.try_live_with(|guard| {
        guard
            .middleware(AuthMiddleware::optional())
            .middleware(LiveTenantMiddleware::new(Arc::new(SingleTenant)))
            .middleware(RateLimitMiddleware::new(
                limiter,
                SlidingWindowConfig { max_requests: 600, window: Duration::from_secs(60) },
                |request: &Request| format!("live:{}", request.ip().unwrap_or_else(|| "anon".into())),
            ))
    })
}

struct SingleTenant;

#[async_trait]
impl LiveTenantResolver for SingleTenant {
    async fn resolve(&self, _request: &Request) -> Result<Option<String>, FrameworkError> {
        Ok(None)
    }
}
```

最初のリクエストの前にランタイムとマウントカタログが準備されるよう、エントリー
ポイントからルートをインストールします:

```rust
Application::new()
    .bootstrap(bootstrap::register)
    .try_routes(|| live::routes(routes::register()))
    .run()
    .await;
```

## ドキュメントとアイランド

ドキュメントルートはアイランドを一度宣言し、`LiveDocument` を通してレンダリングし、
ブートストラップタグを出力します:

```rust
use std::collections::BTreeMap;

use suprnova::live::{CanonicalValue, LiveBootstrapOptions, LiveDocument, LiveMount, MountFlags};
use suprnova::view::{AssetSet, DocumentResponseIntent, TrustedHtml, ViewName};
use suprnova::{FrameworkError, HttpResponse, Request, Response, Router, StatusCode};

mod filters {
    pub use suprnova::view::filters::trusted_html;
}

#[suprnova::view(path = "live/page.html")]
struct Page<'a> {
    bootstrap: &'a TrustedHtml,
    counter: &'a TrustedHtml,
}

pub fn install(router: Router) -> Result<Router, FrameworkError> {
    let mount = LiveMount::<Counter>::identity_bound("/dashboard", "counter", "dashboard-counter")?;
    let handler_mount = mount.clone();
    let router: Router = router
        .get("/dashboard", move |request: Request| {
            let mount = handler_mount.clone();
            async move { render(request, &mount).await }
        })
        .middleware(AuthMiddleware::redirect_to("/login"))
        .into();
    router.try_live_mount(&mount)
}

async fn render(request: Request, mount: &LiveMount<Counter>) -> Response {
    let result: Result<HttpResponse, FrameworkError> = async {
        let mut document = LiveDocument::from_request(&request)?;
        let counter = document
            .mount(mount, CanonicalValue::Object(BTreeMap::new()), MountFlags::empty())
            .await?;
        let bootstrap = document.bootstrap(LiveBootstrapOptions::esm())?;
        document
            .render(
                ViewName::parse("live/page.html").map_err(|_| FrameworkError::internal("view"))?,
                &Page { bootstrap: bootstrap.html(), counter: counter.html() },
                DocumentResponseIntent::html(StatusCode::OK).map_err(|_| FrameworkError::internal("intent"))?,
                AssetSet::empty(),
            )
            .map_err(FrameworkError::from)
    }
    .await;
    result.map_err(|_| HttpResponse::text("Live document failed").status(500))
}
```

- `LiveMount::public_seed` は、どの訪問者でもレンダリングできるアイランドを宣言します。
  その状態は再利用可能なシードで、最初のアクションでインスタンスに昇格します。
- `LiveMount::identity_bound` は、現在のセッションとプリンシパルに属するアイランドを
  宣言します。ドキュメントルートは認証しなければなりません。
- `bootstrap` の前にすべてのアイランドをマウントし、`bootstrap` は一度だけ呼びます。
  ブートストラップは不活性な設定要素と、ESM またはクラシック戦略のスクリプトタグを
  出力し、マウントされたコンポーネントが必要とする場合にアップロードと非同期のロールを、
  要求に応じて Stimulus ブリッジを追加します。
- ドキュメントテンプレートは `{{ bootstrap|trusted_html }}` を `<head>` に置き、
  各アイランドをあるべき場所に置きます。

## セキュリティ境界

Live はフレームワークのミドルウェアを決して迂回しません。各リクエストに必要なもの:

| 事実 | 記録するもの |
|---|---|
| セッション | `SessionMiddleware` |
| オリジンと CSRF | オリジン検証を有効にした `CsrfMiddleware` |
| プリンシパル | 認証済み分岐の `AuthMiddleware` |
| テナント | リゾルバー付きの `LiveTenantMiddleware` |
| レート制限 | 許可分岐の `RateLimitMiddleware` |

同梱のランタイムは Live メディアタイプとブラウザー自身の `Sec-Fetch-Site` ヘッダーを
送り、セッショントークンは持ちません。CSRF ミドルウェアは、設定されたオリジン
ポリシーにかかわらず、すべての Live リクエストについてこの証明を自ら検証します。
同一オリジンの Live リクエストはステートレス CSRF 判定で通過し、クロスサイトや
ヘッダーなしのリクエストはトークン検証にフォールバックして拒否されます。通常の
ルートは既定ポリシーのもとでトークン検証を維持し、Live を使っても他は何も緩みません:

```rust
global_middleware!(CsrfMiddleware::new());
```

匿名の訪問者は公開シードをレンダリングでき、ガードが `AuthMiddleware::optional()`
を使う場合はアクションも実行できます。サインイン済みのプリンシパルは記録され、
匿名の訪問者はそのまま進み、マウント種別が判断します。公開シードは最初のアクションで
訪問者自身のセッション向けに昇格し、アイデンティティ結合アイランドはプリンシパルの
証拠がないリクエストを引き続き拒否します。`AuthMiddleware::new()` の場合、ガードは
エンジンの処理より前にすべての匿名リクエストへ `401` で応答します。
アイデンティティ結合アイランドにはセッションとプリンシパルが必要です。リゾルバーが
テナントを名指しするたびに、テナントはアイランドのスコープに結合され、テナントを
判定できないリゾルバーは `None` ではなくエラーを返さなければなりません。すべての
拒否は閉じています。古い、または改ざんされたスナップショットへの `409` は本文を持たず、
本番のメッセージにはスナップショット、トークン、Cookie、レンダリング済み HTML が
決して含まれません。

## アップロード

モデルフィールドにアップロードポリシーを宣言します:

```rust
use suprnova::live::{LiveComponent, UploadPolicy, UploadReplacement, UploadScan, UploadType, live};

fn avatar_policy() -> UploadPolicy {
    UploadPolicy::builder()
        .maximum_files(1)
        .maximum_file_bytes(512 * 1024)
        .replacement(UploadReplacement::RetirePrevious)
        .accept(UploadType::Png)
        .scan(UploadScan::Disabled)
        .finalize_action("save_avatar")
        .build()
}

#[derive(LiveComponent)]
#[live(name = "app.avatar-uploader", view = "live/avatar-uploader.html")]
pub struct AvatarUploader {
    #[model]
    #[upload(policy = avatar_policy)]
    avatar: String,
}

#[live]
impl AvatarUploader {
    #[action]
    pub fn save_avatar(&mut self) {}
}
```

ビューは `<input type="file" live:upload="avatar">` でフィールドをバインドします。
ランタイムは `/__live/v1/upload` を通じてアップロードを作成、転送、完了させます。
ファイルは、宣言された確定アクションが実行されるまで隔離領域で待機し、そのとき
フレームワークが `UploadFinalizer` に渡します。ファイナライザーと、スキャナーや
バリデーターがあればそれも、ランタイムが組み立てられる前にバインドします:

```rust
App::singleton(LiveUploadHost::new().with_finalizer(Arc::new(AppUploadFinalizer::default())));
```

アップロードはゲートを通じてフィールドと制御ごとに認可されます。`Create`、
`Reacquire`、`Status`、`Queue`、`BeginTransfer`、`PutChunk`、`Complete`、`Accept`、
`BeginFinalize`、`CommitFinalize`、`Cancel`、`Reject`、`Expire`、`Fail` について
`live:<component>.upload.<field>.<Control>` の能力を定義します。

転送グラントを失ったブラウザーは、予約済み名前空間の外でアプリケーションが所有する
ルートを通じて再取得します:

```rust
let router: Router = router
    .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")?
    .middleware(AuthMiddleware::new())
    .into();
```

このルートはアクションと同じ事実を要求し、アップロードを作成したセッションと
プリンシパルにだけ応答し、現在の転送状態とともに新しいグラントを返します。

## 非同期更新

コンポーネントは待ち受けるストリームを宣言します。ブラウザーランタイムは SSE または
WebSocket で購読し、ポーリングにフォールバックします:

```rust
use suprnova::live::{EventPayloadMetadata, LiveComponent, live};

pub struct ActivityPosted;

impl EventPayloadMetadata for ActivityPosted {
    const NAME: &'static str = "activity.posted";
    const VERSION: u16 = 1;
}

#[derive(LiveComponent)]
#[live(
    name = "app.activity-feed",
    view = "live/activity-feed.html",
    minimum_protocol_version = 2,
    streams(stream(name = "activity", topics("activity"), events(ActivityPosted)))
)]
pub struct ActivityFeed {
    #[public]
    headline: String,
}
```

購読者のために `live:<component>.stream.<name>` の能力を定義し、アプリケーションの
どこからでも発行します:

```rust
let streams = LiveStreams::resolve()?;
streams.event::<ActivityPosted>("activity", LiveEventTarget::Island, payload).await?;
streams.refresh("activity").await?;
```

リフレッシュは購読中のアイランドに新規レンダリングを指示し、イベントはアイランドの
登録済みハンドラーに配信されます。ポーリングは通常の新規レンダリングです。
トランスポートが使えないときアイランドの状態は追いつきますが、その間に発行された
イベントペイロードはハンドラーへ再配信されず、ランタイムはそのストリームを最新ではなく
劣化として報告します。ストリームをちょうど 1 つ宣言するコンポーネントはアイランド
ルートがそれを購読し、複数のストリームを持つコンポーネントはランタイムの登録済み
呼び出しで各ストリームを購読します。

## アセットとビルド不要の利用

フレームワークは、精査済みのランタイム成果物そのものを
`/__live/v1/assets/<identity>/<file>` で、不変キャッシュ、強いバリデーター、
ブートストラップタグ内の整合性属性とともに配信します。ドキュメントにインライン
スクリプトが含まれないため、厳格な `script-src 'self'` ポリシーが成り立ちます。
同じバイト列を CDN や静的ディレクトリに公開するには:

```bash
suprnova live:assets --out public/__live
```

公開はアトミックで、`--replace` を渡さない限り、バイト列が異なるディレクトリの
置き換えを拒否します。

## テスト

`suprnova::live::testing` は、プロセス内テストのためにルーターのランタイムと
マウントカタログを準備します。`app/tests/live_*.rs` のアプリケーションテストが
完全なパターンを示します。インメモリデータベース、用意されたセッション Cookie、
実際のグローバルミドルウェアスタック、そして `handle_request` を通したリクエストです:

```rust
let router = app::live::routes(app::routes::register())?;
let runtime = prepare_live_router_for_test(&router)?;
App::singleton(runtime.clone());
```

アイランドの `data-suprnova-live-snapshot` 属性からスナップショットをデコードし、
セッション Cookie と `Sec-Fetch-Site: same-origin` を付けてアクションを送信し、
受理されたレンダリングを検証します。古いスナップショットは空の本文で `409` を返し、
プリンシパルがなければ `401` を返します。

## 診断と運用

- `suprnova live:check` は登録済みのすべてのビューを証明します。`--allow-unproved`
  は、チェッカーが意図的に主張しない動的構造を受け入れます。
- `suprnova live:inspect` は、状態や秘密を露出せずに、バインド済みレジストリ、
  設定の上限、インストール済みアップロード機能、組み立て済みランタイムサービス、
  アセット識別子を報告します。
- `LiveConfig` はリクエストとレスポンスのバイト数、および信頼済みコンテキストの
  寿命を制限します。ランタイムが組み立てられる前に独自のものをバインドします。
- エラーは `live_document_context_rejected` や `invalid_live_bootstrap` のような
  閉じた種類を持ち、テレメトリのラベルは閉じた列挙です。

## 復旧

- `409` はランタイムにアイランドの新規レンダリングを指示します。操作は再実行
  されません。
- 閉じられた非同期トランスポートは退役し、ランタイムは新しいトランスポート世代で
  再接続します。古い世代は拒否されます。
- 期限切れまたはローテーションしたセッションは、アイデンティティ結合の作業を無効に
  します。アプリケーションはサインインの経路を示し、訪問者は新しいドキュメントから
  続行します。

Live は RenderCache なしで完全に動作します。Live ドキュメントのキャッシュは別の
機能で、提供時に独自の章を持ちます。

## CLI リファレンス

| コマンド | 目的 |
|---|---|
| `suprnova live:make <name>` | コンポーネントとそのビューを生成して登録する |
| `suprnova live:check` | 登録済みのすべてのビューを統合チェッカーで証明する |
| `suprnova live:inspect` | ランタイム、レジストリ、プロバイダー、成果物の安全な状態を報告する |
| `suprnova live:assets --out <dir>` | 精査済みランタイム成果物をアトミックに公開する |
