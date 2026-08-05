# フロントエンド 概要

Suprnova は Rust ハンドラをシングルページフロントエンドに
[Inertia.js](https://inertiajs.com/) 3.4.0 を経由して接続します。Rust でコントローラーを書き、Svelte、React、または Vue でページを書きます。フレームワークは型付きプロップを両者の間で移動させるため、中間に別の HTTP API は不要です。

## 3 つのファーストクラススターター

`suprnova new <name>` は動作するプロジェクトをスキャフォルドします。`--frontend` フラグは SPA レイヤーを選択します。

```bash
suprnova new my-app                       # Svelte 5（デフォルト）
suprnova new my-app --frontend svelte     # Svelte 5
suprnova new my-app --frontend react      # React 19
suprnova new my-app --frontend vue        # Vue 3.5
```

3 つのスキャフォルドはすべて同じスタックを共有します。

| レイヤー | バージョン |
|---|---|
| Inertia クライアント アダプター | `@inertiajs/{svelte,react,vue3}` 3.4.0 |
| ビルドツール | Vite 8 |
| スタイリング | Tailwind v4 (`@tailwindcss/vite`) |
| TypeScript | strict モード |

選択はプロジェクト単位です。サーバー側に「プライマリー」フレームワークはありません - `inertia_response!` は選択したスキャフォルドが使用する拡張子（`.svelte`、`.tsx`、`.vue`）を解決し、`App::inertia_share`、部分的なリロード、TypeScript プロップ生成はすべて 3 つ全体で同じように動作します。

## アーキテクチャ

```
                       Browser
   +-------------------------------------------------+
   |               SPA (Svelte / React / Vue)        |
   |   +---------------+ +---------------+           |
   |   | Home.svelte   | | Users/Show.tsx|  ...      |
   |   +-------+-------+ +-------+-------+           |
   |           |  typed props from Rust struct       |
   |   +-------v-------------------------------+     |
   |   |        Inertia client adapter         |     |
   +---+------------------+------------------+--+----+
                          |
                          |   HTTP (JSON on XHR, HTML on first load)
                          v
   +-------------------------------------------------+
   |                  Suprnova server                |
   |   +------------------------------------------+  |
   |   |          Controllers / handlers          |  |
   |   |   inertia_response!(&req, "Home",        |  |
   |   |                     HomeProps { ... })   |  |
   |   +------------------------------------------+  |
   +-------------------------------------------------+
```

最初のリクエストはマウント ノードの `data-page` 属性に埋め込まれた初期ページ オブジェクトを含む HTML シェルを返します。その後の訪問は `<Link>` / `router.visit` を通じて行われ、`X-Inertia: true` を送信して、JSON ページ オブジェクトが返されます - アダプターは完全なリロードなしでコンポーネントを交換します。

## 完全なページ往復

コントローラーはプロップを Rust 構造体として定義し、`InertiaProps` から派生させ、値を `inertia_response!` マクロに渡します。

```rust
use suprnova::{InertiaProps, Request, Response, inertia_response};

#[derive(InertiaProps)]
pub struct HomeProps {
    pub title: String,
    pub message: String,
}

pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        message: "Hello from Suprnova!".into(),
    })
}
```

マクロがあなたのためにいくつかのことを行います。最初に、ページ コンポーネント ファイルが実際に `frontend/src/pages/Home.{svelte,tsx,jsx,vue}` 下に存在することをコンパイル時に検証します - タイプミスはブラウザーの 404 ではなく、ビルド エラーとして表示されます。第二に、`HomeProps` 構造体をシリアライズし、トップレベル キーごとに 1 つのプロップに展開して部分的なリロードがフィルタリングでき、返す前に `&req` に対してレイジー プロップまたはディファード プロップを解決します。マクロは `Result<HttpResponse, FrameworkError>` に評価され、`Response` 戻り値の型が直接受け入れます。

一致する Svelte ページ（デフォルト スキャフォルド）。

```svelte
<!-- frontend/src/pages/Home.svelte -->
<script lang="ts">
  import type { HomeProps } from '../types/inertia-props'

  let { title, message }: HomeProps = $props()
</script>

<div class="font-sans p-8 max-w-xl mx-auto">
  <h1 class="text-3xl font-bold">{title}</h1>
  <p class="mt-2">{message}</p>
</div>
```

React と Vue の同等物については、[ページ コンポーネント](frontend-pages.md) を参照してください。

## TypeScript 型の生成

`src/` 内のすべての `#[derive(InertiaProps)]` 構造体は `frontend/src/types/inertia-props.ts` の TypeScript インターフェースになります。

```bash
suprnova generate-types
```

`--routes` を渡すと、同じコマンドは `frontend/src/types/routes.ts` も生成します - `routes!` マクロから抽出され、Inertia v2+ API と直接連携する型安全な URL + メソッド ペア。完全な型マッピング テーブルとルート ヘルパー形状は [TypeScript 型](frontend-typescript-types.md) に存在します。

## 共有データ

すべてのページに表示される必要があるもの（認証されたユーザー、現在のロケール、アプリ メタデータ）は起動時に一度登録され、すべての Inertia レスポンスにマージされます。

```rust
// bootstrap.rs で
App::inertia_share("appName", "Suprnova");
App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

// 非同期/リクエスト単位の共有データはトレイトを通じて行われます。
App::register_inertia_shared(Arc::new(AppSharedData));
```

3 つのバリエーション、優先順位の順（後の方が同じキーで勝つ）。

| API | 値が具現化されるとき |
|---|---|
| `App::inertia_share(k, v)` | 同期、起動時に一度設定 |
| `App::inertia_share_lazy(k, \|\| async { ... })` | レスポンス単位、再計算 |
| `App::inertia_share_once(k, \|\| async { ... })` | レスポンス単位、その後クライアント キャッシュ |
| `App::register_inertia_shared(Arc::new(impl))` | リクエスト単位、`&req` を参照 |

レスポンス ビルダーに添付されたページ単位のプロップは、常に同じキーでの共有データを上書きします。

## 部分的なリロードとレイジープロップ

同じ `InertiaResponse` ビルダーは Inertia v3 の完全なプロップ ツールキット - eager、lazy、optional、deferred、merge、once - を公開し、Suprnova は v3 部分的リロード ヘッダー（`X-Inertia-Partial-Data`、`X-Inertia-Partial-Except`、`X-Inertia-Reset`、`X-Inertia-Except-Once-Props`）を自動的に認識します。以下の例は異なる評価ルールで 3 つのプロップを添付します。

```rust
use suprnova::{InertiaResponse, FrameworkError, Request, Response};

pub async fn dashboard(req: Request) -> Response {
    let resp = InertiaResponse::new("Dashboard")
        .with("title", "Dashboard")
        .lazy("recent_orders", || async {
            Ok::<_, FrameworkError>(load_recent_orders().await?)
        })
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        .resolve(&req)
        .await?;
    Ok(resp)
}
```

`inertia_response!` は eager-props ケースをカバーします。それより先のすべてはビルダーを通じて行われます。完全なサーフェス - `optional`、`merge`、`once`、`scroll`、`flash`、`paginate`、SSR、version mismatch、history encryption - は [Inertia レスポンス](frontend-inertia-responses.md) に文書化されています。

## ブートストラップ

スキャフォルドされたアプリは `bootstrap.rs` の内部で 1 つの呼び出しで 2 つのプロトコル重要なミドルウェアをインストールします。

```rust
use suprnova::{Inertia, InertiaConfig};

Inertia::install(&InertiaConfig::new().version(env!("CARGO_PKG_VERSION")))
    .expect("Inertia install failed");
```

`install` は `Result` を返します - `InertiaConfig` が本番環境モード（`APP_ENV=production` のデフォルト）に解決されるが、Vite マニフェストが見つからない場合、レガシー アセット パスに無音でフォールバックするのではなく、クローズされて失敗します。下の [開発 vs 本番環境](#開発-vs-本番環境) を参照してください。

これにより `InertiaVersionMiddleware`（アセット バージョンのミスマッチで 409 + `X-Inertia-Location` を送信して古いクライアントをリロード）と `Inertia303Middleware`（非 GET Inertia 訪問で 302 → 303 に書き直して、フォローアップが明確に GET である）が登録されます。両方以前はオプトイン でしたが、`Inertia::install` はそれらをデフォルトにします。

## 開発 vs 本番環境

開発中、Vite 開発サーバーはバックエンドとともに実行され、HMR 対応アセットを提供します。

```bash
suprnova serve
```

これは Rust サーバーと `vite` を一緒に起動します。HTML シェルは `http://localhost:5765` からモジュールをロードします。

本番環境では、フロントエンドを一度ビルドし、バックエンドを `public/assets/` 下のハッシュされたマニフェストにポイントします。

```bash
cd frontend && npm run build
APP_ENV=production suprnova serve --backend-only
```

`InertiaConfig::default()` は本番環境 vs 開発環境モードを `APP_ENV`（`Environment::detect().is_production()` 経由）から派生させます - `APP_ENV=production` は HTML シェルが Vite 開発サーバーの代わりにビルド済みアセットをロードするように指定します。`Inertia::install` は古いハードコード パスに無音でフォールバックするのではなく、その決定をバックアップするマニフェストが見つからない場合、起動時に大きく失敗します。

Suprnova は `public/assets/.vite/manifest.json` を読み取り、`modulepreload` のハッシュされたエントリー ポイントと推移的インポートを解決します。SSR はオプション - `InertiaConfig::ssr(...)` を実行中の `@inertiajs/{vue3,react,svelte}/server` ワーカーにポイントしてオプトインします。

### Suprnovaが異なる設計を選んだ理由

一般的な Inertia セットアップが他の場所でどのように見えるかからの 3 つの意図的な逸脱です。

- **コンパイル時のコンポーネント検証。** `inertia_response!` マクロはビルド時に `frontend/src/pages/` をウォークし、コンポーネント ファイルが見つからない場合は展開を拒否して、最も近いマッチを示唆します。削除されたページをポイントするコントローラーを出荷することはできません。
- **型付きプロップが真実のソース。** ページ プロップは `#[derive(InertiaProps)]` を持つ Rust 構造体です。`suprnova generate-types` はそれらを読み取り、TypeScript インターフェースを書き込みます - フロントエンド型はバックエンドから派生し、並行して維持されません。
- **デフォルトとしての Svelte。** Inertia のドキュメンテーションは Vue と React に最初に到達します。Suprnova スキャフォルダーは Svelte 5（runes-on）をデフォルトにします。React 19 と Vue 3.5 はファーストクラスであり、事後思考ではありません - 同じプロトコル、同じプロップ パイプライン、同じジェネレーター出力。

## 次のステップ

- [ページ コンポーネント](frontend-pages.md)
- [Inertia レスポンス](frontend-inertia-responses.md)
- [TypeScript 型](frontend-typescript-types.md)
- [ルーティング](routing.md)
- [コントローラー](controllers.md)
