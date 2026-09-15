# Live

Suprnova Live は、フレームワークのサーバー駆動インタラクションエンジンです。Live
コンポーネントは、状態がサーバー上にあり、ビューが Askama テンプレートで、アクションが署名付きプロトコルを介して小さなブラウザーランタイムから実行される Rust の構造体です。ランタイムは再レンダリングされた HTML をその場でモーフします。同期を保つべきクライアント側の状態モデルはなく、同梱ランタイムを使うためにインストールするビルドツールもなく、ドキュメントにインライン JavaScript もありません。

この章では、アプリケーション側の表面を扱います。コンポーネントの作成、登録、ドキュメントとアイランドの配信、すべての Live リクエストが越えるセキュリティ境界、アップロード、非同期更新、アセット、テスト、診断、そして復旧です。ここで使うのは
`suprnova::live` と `suprnova::view` だけです。

## クイックスタート

`suprnova new` で作成したプロジェクトは Live に対応済みです。空のコンポーネントレジストリと `routes()` 関数を持つ `src/live/mod.rs` を含み、ブートストラップがレジストリをバインドし、`cmd/main.rs` がルートをインストールします。コンポーネントを生成して、確認します:

```bash
suprnova live:make Counter
suprnova live:check
```

`live:make` は `src/live/counter.rs` と `templates/live/counter.html` を書き出し、
`src/live/mod.rs` にコンポーネントを登録し、次の手順を表示します。`live:check` はアプリケーションをビルドし、登録済みのすべてのビューを統合チェッカーで証明します。

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

- `name` は登録されるコンポーネント名です。`app.counter` のようなドットで区切ったケバブケースの名前を使います。CLI は `<package>.<kebab>` を導出します。
- `view` はテンプレートルートからの相対的なテンプレート識別子です。
- `#[public]` フィールドはレンダリングされ、署名付きスナップショットに含まれます。
  `#[model]` フィールドはさらに `live:model` を通じてブラウザーからの提案を受け付けます。
- `#[action]` メソッドは、ブラウザーが呼び出せる唯一のエントリーポイントです。検証済みの引数を受け取り、リダイレクトやフラッシュなどの型付き結果を返せます。

すべてのフィールド型は `Default` を実装する必要があります。新しいアイランドは、マウントフックが別に指定しない限り、これらの既定値から始まります。

## ビュー

ビューは Askama テンプレートです。テンプレートルートは、`askama.toml` が別のディレクトリを指定しない限り `templates/` なので、`live/counter.html` は
`templates/live/counter.html` に置きます:

```html
<div>
<p>Count: {{ count }}</p>
<button type="button" live:click="increment">Increment</button>
</div>
```

ディレクティブは閉じた `live:` 文法を使います。`live:click`、`live:submit`、
`live:model`、`live:upload`、`live:key`、`live:loading`、およびドキュメント化された残りの集合です。チェッカーはすべてのディレクティブをコンポーネントに対して証明します。未知のアクション、未知のモデルフィールド、生の `safe` フィルター、またはアクセシビリティ違反は、ファイル、行、列を示して `live:check` を失敗させます。

アイランドを配置するドキュメントは、`#[suprnova::view]` で宣言する通常のビューです。それらが受け付ける唯一のエスケープされない値は、`trusted_html` フィルターを通した
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

サーバー、ワーカー、そして `suprnova live:*` コマンドが同じコンポーネントを見るように、ブートストラップ中にバインドします:

```rust
suprnova::App::singleton(crate::live::registry().expect("Live component registry"));
```

ランタイムが組み立てられた後、レジストリは不変です。重複したコンポーネント名やビュー、あるいは検証ポートなしで検証を必要とするアクションを持つコンポーネントは、型付きの `RegistryError` で登録に失敗します。

## ルーティング

`Router::try_live()` は予約済み名前空間を正確に一度インストールします。
`/__live/action`、`/__live/upload`、`/__live/async/*` の制御ルートと
WebSocket ハンドシェイク、そして不変の `/__live/assets/*` ルートです。アプリケーションルートが `/__live` を要求できる場合、起動は失敗します。

予約済みのリクエストルートは厳格なポリシーを持ちます。すべてのリクエストにはセッション、オリジン、CSRF、プリンシパル、テナント、レート制限の事実が必要です。フレームワークはセッションと CSRF の証明を記録し、アプリケーションは残りをルートガードで取り付けます:

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

最初のリクエストの前にランタイムとマウントカタログが準備されるよう、エントリーポイントからルートをインストールします:

```rust
Application::new()
    .bootstrap(bootstrap::register)
    .try_routes(|| live::routes(routes::register()))
    .run()
    .await;
```

## ドキュメントとアイランド

ドキュメントルートはアイランドを一度宣言し、`LiveDocument` を通してレンダリングし、ブートストラップタグを出力します:

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

- `LiveMount::public_seed` は、どの訪問者でもレンダリングできるアイランドを宣言します。その状態は再利用可能なシードで、最初のアクションでインスタンスに昇格します。
- `LiveMount::identity_bound` は、現在のセッションとプリンシパルに属するアイランドを宣言します。ドキュメントルートは認証しなければなりません。
- `bootstrap` の前にすべてのアイランドをマウントし、`bootstrap` は一度だけ呼びます。ブートストラップは不活性な設定要素と、ESM またはクラシック戦略のスクリプトタグを出力し、マウントされたコンポーネントが必要とする場合にアップロードと非同期のロールを、要求に応じて Stimulus ブリッジを追加します。
- ドキュメントテンプレートは `{{ bootstrap|trusted_html }}` を `<head>` に置き、各アイランドをあるべき場所に置きます。

## セキュリティ境界

Live はフレームワークのミドルウェアを決して迂回しません。各リクエストに必要なもの:

| 事実 | 記録するもの |
|---|---|
| セッション | `SessionMiddleware` |
| オリジンと CSRF | オリジン検証を有効にした `CsrfMiddleware` |
| プリンシパル | 認証済み分岐の `AuthMiddleware` |
| テナント | リゾルバー付きの `LiveTenantMiddleware` |
| レート制限 | 許可分岐の `RateLimitMiddleware` |

同梱のランタイムは Live メディアタイプとブラウザー自身の `Sec-Fetch-Site` ヘッダーを送り、セッショントークンは持ちません。CSRF ミドルウェアは、設定されたオリジンポリシーにかかわらず、すべての Live リクエストについてこの証明を自ら検証します。同一オリジンの Live リクエストはステートレス CSRF 判定で通過し、クロスサイトやヘッダーなしのリクエストはトークン検証にフォールバックして拒否されます。通常のルートは既定ポリシーのもとでトークン検証を維持し、Live を使っても他は何も緩みません:

```rust
global_middleware!(CsrfMiddleware::new());
```

匿名の訪問者は公開シードをレンダリングでき、ガードが `AuthMiddleware::optional()`
を使う場合はアクションも実行できます。サインイン済みのプリンシパルは記録され、匿名の訪問者はそのまま進み、マウント種別が判断します。公開シードは最初のアクションで訪問者自身のセッション向けに昇格し、アイデンティティ結合アイランドはプリンシパルの証拠がないリクエストを引き続き拒否します。`AuthMiddleware::new()` の場合、ガードはエンジンの処理より前にすべての匿名リクエストへ `401` で応答します。アイデンティティ結合アイランドにはセッションとプリンシパルが必要です。リゾルバーがテナントを名指しするたびに、テナントはアイランドのスコープに結合され、テナントを判定できないリゾルバーは `None` ではなくエラーを返さなければなりません。すべての拒否は閉じています。古い、または改ざんされたスナップショットへの `409` は本文を持たず、本番のメッセージにはスナップショット、トークン、Cookie、レンダリング済み HTML が決して含まれません。

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

ビューは `<input type="file" live:upload="avatar">` でフィールドをバインドします。ランタイムは `/__live/upload` を通じてアップロードを作成、転送、完了させます。ファイルは、宣言された確定アクションが実行されるまで隔離領域で待機し、そのときフレームワークが `UploadFinalizer` に渡します。ファイナライザーと、スキャナーやバリデーターがあればそれも、ランタイムが組み立てられる前にバインドします:

```rust
App::singleton(LiveUploadHost::new().with_finalizer(Arc::new(AppUploadFinalizer::default())));
```

アップロードはゲートを通じてフィールドと制御ごとに認可されます。`Create`、
`Reacquire`、`Status`、`Queue`、`BeginTransfer`、`PutChunk`、`Complete`、`Accept`、
`BeginFinalize`、`CommitFinalize`、`Cancel`、`Reject`、`Expire`、`Fail` について
`live:<component>.upload.<field>.<Control>` の能力を定義します。

転送グラントを失ったブラウザーは、予約済み名前空間の外でアプリケーションが所有するルートを通じて再取得します:

```rust
let router: Router = router
    .try_live_upload_reacquisition("/account/uploads/{handle}/reacquire")?
    .middleware(AuthMiddleware::new())
    .into();
```

このルートはアクションと同じ事実を要求し、アップロードを作成したセッションとプリンシパルにだけ応答し、現在の転送状態とともに新しいグラントを返します。

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

購読者のために `live:<component>.stream.<name>` の能力を定義し、アプリケーションのどこからでも発行します:

```rust
let streams = LiveStreams::resolve()?;
streams.event::<ActivityPosted>("activity", LiveEventTarget::Island, payload).await?;
streams.refresh("activity").await?;
```

リフレッシュは購読中のアイランドに新規レンダリングを指示し、イベントはアイランドの登録済みハンドラに配信されます。ポーリングは通常の新規レンダリングです。トランスポートが使えないときアイランドの状態は追いつきますが、その間に発行されたイベントペイロードはハンドラへ再配信されず、ランタイムはそのストリームを最新ではなく劣化として報告します。ストリームをちょうど 1 つ宣言するコンポーネントはアイランドルートがそれを購読し、複数のストリームを持つコンポーネントはランタイムの登録済み呼び出しで各ストリームを購読します。

ストリームは、それを開いたセッションとともに終了します。ストリームを保持するノードでセッションが破棄されると、通常のログアウト、通常のログアウト、無効化、ID の再生成、「すべての端末からログアウト」のいずれであっても、そのセッションがそこで開いたメンバーシップはすべて直ちに退会させられ、以後のイベントは届きません。別のノードで破棄されたセッションは配信そのものが捕捉します。各メンバーシップのセッションは 10 秒に 1 回を上限としてセッションストアに対して再確認されるため、イベントはその間隔のうちに止まります。ストリームのゲートはいずれにせよ配信のたびに再度問い合わせられるので、ポリシーの変更はどのノードでも直ちに配信を終了させます。

## アセットとビルド不要の利用

フレームワークは、精査済みのランタイム成果物そのものを
`/__live/assets/<identity>/<file>` で、不変キャッシュ、強いバリデーター、ブートストラップタグ内の整合性属性とともに配信します。ドキュメントにインラインスクリプトが含まれないため、厳格な `script-src 'self'` ポリシーが成り立ちます。同じバイト列を CDN や静的ディレクトリに公開するには:

```bash
suprnova live:assets --out public/__live
```

公開はアトミックで、`--replace` を渡さない限り、バイト列が異なるディレクトリの置き換えを拒否します。

## コンポーネントライブラリ

Suprnova は Live 向けコンポーネントライブラリの基盤を同梱しています。ベースレイヤーを備えたトークンスタイルシートと、ネイティブコントロールおよび `live:model`、`live:error`、`live:loading` の語彙の上に構築されたプレゼンテーション用フォームコンポーネント群です。ベースはランタイムアーティファクトです。ドキュメントがオプトインすると、ランタイムスクリプトと同じ ID、整合性、キャッシュの契約のもとで、ひとつのスタイルシートリンクとして届きます。

```rust
let bootstrap = document.bootstrap(LiveBootstrapOptions::esm().with_suprnova_ui())?;
```

その中のすべてのルールはカスケードレイヤー `suprnova-ui` の内側にあるため、レイヤーに属さない独自のスタイルが詳細度の争いなしに勝ちます。すべての視覚的な値は、色、フォント、間隔、角丸、影、モーション、密度、状態のための `--sn-` カスタムプロパティで、ライトとダークの値を持ちます。`:root` でトークンを上書きすればテーマを変えられ、レイヤーを外しても振る舞い、名前、状態属性はすべて保たれます。コンポーネントは状態を、チェッカーが証明する属性（`aria-invalid`、`aria-busy`、`aria-expanded`、`aria-pressed`、`aria-current`、`aria-selected`、`:disabled`）から装飾し、クラスからは決して装飾しないからです。Tailwind CSS 4 の `@theme` プリセットがトークンを Tailwind の名前空間へ対応付けますが、Tailwind が必須になることはありません。コンポーネントは `live:add` でインストールされ、予約されたルート `templates/suprnova-ui/` の下にコンポーネントごとにひとつのディレクトリを持ちます。Askama マクロのビュー、スタイルシート、コンポーネントが持つ場合の JavaScript、そしてそれらを名指しするマニフェストです。

```bash
suprnova live:add field
suprnova live:add password-input
```

編集済みのファイルは後の実行でも保持され、`--force` で置き換えられます。サードパーティのコンポーネントは `--manifest` で独自のマニフェストから、独自のルートの下にインストールされます。ビューからマクロを呼び出し、`try_live_ui_assets()` でスタイルシートとスクリプトを配信し、ドキュメントからリンクします。

```html
{% import "suprnova-ui/field/field.html" as field %}
{% import "suprnova-ui/input/input.html" as input %}
{% call field::field("email", "Email", required=true) %}
{% call input::input("email", kind="email", required=true) %}{% endcall %}
{% endcall %}
```

チェッカーはマクロを展開するため、`live:check` はライブラリのビューを他のビューと同じように証明します。現在のフォームファミリーは、フィールド、ラベル、入力、テキストエリア、数値入力、スライダー、検索入力、表示切替付きパスワード入力、チェックボックスとチェックボックスグループ、ラジオグループ、スイッチ、セレクト、ボタンとリンクボタン、ボタングループ、フィールドセット、フォームアクション、バリデーションサマリー、ファイル入力です。ライブラリのコンポーネントは `suprnova.*` と名付けられ、レジストリは他のクレートからのこの接頭辞を拒否します。カスタム要素はライト DOM で、`sn-` 接頭辞を持ちます。

オーバーレイの一族も同じ基盤の上に出荷されます。ツールチップ、コラプシブルとアコーディオン、ポップオーバー、単一階層のドロップダウンメニュー、ダイアログ、シート、ドロワーです。それぞれはスクリプトが動く前に、ブラウザー自身のプリミティブで開閉状態を保持します。ディスクロージャーには `details`、ポップオーバーとメニューには `popover` 属性、三つのモーダルには `dialog` で、これらはベンダリングされた `sn-dialog`、`sn-sheet`、`sn-drawer` 要素が `showModal()` で開き、閉じるときにフォーカスをトリガーへ返します。開閉は決して Live リクエストを発行しません。発行するのは、オーバーレイの中に置いたアクションだけです。すべてのオーバーレイのルートは安定したキーと `live:preserve.self` を持つので、開いたオーバーレイは、その領域を置き換えなかったモーフを生き延びます。

```html
{% import "suprnova-ui/dialog/dialog.html" as dialog %}
{% call dialog::dialog_trigger("confirm", "Delete everything", variant="danger") %}{% endcall %}
{% call dialog::dialog("confirm", "confirm", "Delete everything?") %}
<p>This removes every note.</p>
{% call button::button("Delete", action="confirm_delete", variant="danger") %}{% endcall %}
{% call dialog::dialog_close("confirm", "Cancel") %}{% endcall %}
{% endcall %}
```

`popover` 属性により、サポートする基準は Chrome と Edge 114、Firefox 128、Safari 17 になります。CSS のアンカー配置があるところでは、ポップオーバーとメニューはトリガーの下に置かれ、それ以外ではブラウザーが中央に配置します。アコーディオンの単一オープンモードは `details name` に依存しており、古いサポート対象バージョンではそれぞれ独立したディスクロージャーとして扱われます。

続いてフィードバックファミリーとナビゲーションファミリーです。フィードバックはアラート、スケルトン、スピナー、プログレス、空状態、そしてフラッシュ領域を隣に持つトースト領域です。それぞれはサーバーまたはランタイムがすでに持つ状態を表示します。アラートはバリアントからロールを選び、色だけでなく記号と非表示のラベルで各バリアントを示します。スピナーやスケルトンは `live:loading.show` で登録済みアクションに結び付けられ、非表示で出力されるため、ランタイムが自身の遅延の後に表示し最小時間を超えて保持し、速いアクションで点滅することはありません。プログレスはラベルとテキストの読み上げを持つネイティブの `progress` 要素で、確定的な作業のときだけ値を持ちます。空状態は理由（空、結果なし、権限なし、切断）をサーバーで描画された状態から取り、呼び出し側が描画する場合にだけ次のアクションを提示します。トーストは丁寧なステータス領域から一度だけ通知し、フォーカスを奪いません。ベンダリングされた `sn-toast-region` 要素はトーストをタイムアウトさせ、ホバーやフォーカス中は一時停止し、同時に表示する数を制限し、閉じるボタンに応答します。各トーストはキー付きで保持されるため、閉じたトーストはモーフをまたいでも閉じたままです。重大なエラーはアラートにも置くべきで、トーストがその唯一の表示面になることはありません。トーストはループの中で描画されるため、そのキーは `live_key` フィルターを通り、トーストをマウントするアイランドは `pub mod filters { pub use suprnova::view::filters::live_key; }` でこのフィルターを公開します。フラッシュ領域は前のリクエストがセッションに残したものを一度だけ描画します。

```html
{% import "suprnova-ui/alert/alert.html" as alert %}
{% import "suprnova-ui/spinner/spinner.html" as spinner %}
{% call alert::alert("saved", variant="success") %}<p>Your changes are saved.</p>{% endcall %}
{% call button::button("Save", action="save") %}{% endcall %}
{% call spinner::spinner(action="save", label="Saving") %}{% endcall %}
```

ナビゲーションはヘッダーバー、フッター、折りたたみグループを持つサイドバー、パンくずリスト、タブ、ページネーション、さらに読み込むです。すべての移動先は実ルート URL を持つアンカーで、すべてのアクションはボタンです。現在の項目はブラウザーの位置ではなく、あなたが結び付けた値から `aria-current` を持ちます。サイドバーのグループはキー付きで保持されるネイティブの `details` です。タブにはモードが必須です。`local` はタブリストの意味論を持つパネルで、ベンダリングされた `sn-tabs` 要素が矢印キーを扱い、切り替えでリクエストを発行しません。`route` はアンカーとしてのタブです。ページネーションにもモードが必須です。ルートページは正規リンクで、Live ページはあなたのアクションに対するボタンであり、その結果が `url_intent` を通じて新しいクエリを現在の履歴エントリに反映し、ページごとの履歴エントリは作りません。さらに読み込むは登録済みアクションに対するボタンでキー付きリストに追記するため、モーフはすでにある行をすべて保持し、使い切ったものとして描画するとコントロールはビューから消えます。URL の反映はプロトコル 2 の結果なので、`url_intent` でページ送りするアイランドは `minimum_protocol_version = 2` を宣言します。そのキー付きの行はトーストと同じように `live_key` を通ります。

```html
{% import "suprnova-ui/tabs/tabs.html" as tabs %}
{% call tabs::tabs("details", mode="local", label="Details") %}
{% call tabs::tab_list("Details") %}
{% call tabs::tab("tab-summary", "panel-summary", "Summary", selected=true) %}{% endcall %}
{% call tabs::tab("tab-history", "panel-history", "History") %}{% endcall %}
{% endcall %}
{% call tabs::tab_panel("panel-summary", "tab-summary", selected=true) %}<p>Summary</p>{% endcall %}
{% call tabs::tab_panel("panel-history", "tab-history") %}<p>History</p>{% endcall %}
{% endcall %}
```

### Suprnova が異なる理由

Laravel は Blade コンポーネントとスターターキットのマークアップを同梱しますが、Suprnova はライブラリをフレームワーク自身を通じて、Live 固有の語彙の上で提供し、クライアントアプリケーションがページを所有することはありません。スキンは既定で有効で、外しても何も壊れません。それがここでのヘッドレスの意味です。

## テスト

`suprnova::live::testing` は、プロセス内テストのためにルーターのランタイムとマウントカタログを準備します。`app/tests/live_*.rs` のアプリケーションテストが完全なパターンを示します。インメモリデータベース、用意されたセッション Cookie、実際のグローバルミドルウェアスタック、そして `handle_request` を通したリクエストです:

```rust
let router = app::live::routes(app::routes::register())?;
let runtime = prepare_live_router_for_test(&router)?;
App::singleton(runtime.clone());
```

アイランドの `data-suprnova-live-snapshot` 属性からスナップショットをデコードし、セッション Cookie と `Sec-Fetch-Site: same-origin` を付けてアクションを送信し、受理されたレンダリングを検証します。古いスナップショットは空の本文で `409` を返し、プリンシパルがなければ `401` を返します。

## 診断と運用

- `suprnova live:check` は登録済みのすべてのビューを証明します。`--allow-unproved`
  は、チェッカーが意図的に主張しない動的構造を受け入れます。
- `suprnova live:inspect` は、状態や秘密を露出せずに、バインド済みレジストリ、設定の上限、インストール済みアップロード機能、組み立て済みランタイムサービス、アセット識別子を報告します。
- `LiveConfig` はリクエストとレスポンスのバイト数、および信頼済みコンテキストの寿命を制限します。ランタイムが組み立てられる前に独自のものをバインドします。
- エラーは `live_document_context_rejected` や `invalid_live_bootstrap` のような閉じた種類を持ち、テレメトリのラベルは閉じた列挙です。

## 復旧

- `409` はランタイムにアイランドの新規レンダリングを指示します。操作は再実行されません。
- 閉じられた非同期トランスポートは退役し、ランタイムは新しいトランスポート世代で再接続します。古い世代は拒否されます。
- 期限切れまたはローテーションしたセッションは、アイデンティティ結合の作業を無効にします。アプリケーションはサインインの経路を示し、訪問者は新しいドキュメントから続行します。

Live は RenderCache なしで完全に動作します。Live ドキュメントのキャッシュは
RenderCache の仕事です。[RenderCache](render-cache.md) を参照してください。

## CLI リファレンス

| コマンド | 目的 |
|---|---|
| `suprnova live:make <name>` | コンポーネントとそのビューを生成して登録する |
| `suprnova live:check` | 登録済みのすべてのビューを統合チェッカーで証明する |
| `suprnova live:inspect` | ランタイム、レジストリ、プロバイダー、成果物の安全な状態を報告する |
| `suprnova live:assets --out <dir>` | 精査済みランタイム成果物をアトミックに公開する |
