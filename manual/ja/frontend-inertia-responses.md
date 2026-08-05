# Inertia レスポンス

Inertiaレスポンスは、Suprnovaのハンドラが状態をSvelte / React / Vueのページコンポーネントへ送り出す方法です。Inertiaページをレンダリングするすべてのハンドラは、[`inertia_response!`](#inertia-response-マクロ)マクロ（型付きで、コンパイル時に検査されるeagerなプロップ向け）か、[`InertiaResponse`](#inertiaresponse-のビルダー)ビルダー（それ以外のすべて - レイジープロップ、ディファードプロップ、マージ、once、スクロール、フラッシュ）のいずれかを通じて組み立てられた、レスポンスを1つ返します。この章は、レスポンスの表面をエンドツーエンドで扱います - マクロ、ビルダー、v3プロトコルの機能（部分的なリロード、履歴の暗号化、バージョン検出）、`App::inertia_share*`による共有データ、そしてリダイレクトをまたいで運ばれるフラッシュバッグです。

まだフロントエンドを選んでいない場合は、[フロントエンド 概要](frontend.md)と[ページ コンポーネント](frontend-pages.md)を先に読んでください。この章は、SPAブリッジが配線済みであることを前提とし、あなたのハンドラが何を返すかに焦点を当てます。

## `inertia_response!` マクロ

このマクロは、ハンドラから型付きのeagerなページへの、最短の経路です。現在のリクエスト、コンポーネント名、プロップの式を受け取ります。

```rust
use suprnova::{Request, Response, inertia_response, InertiaProps};

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

知っておくべきことが3つあります。

- **先頭の`&req`は必須です。** このマクロは、`X-Inertia`ヘッダー、URL、部分的なリロードのフィルタリングヘッダーをリクエストから読み取るため、リクエストの値（あるいは参照）を必要とします。これがなければ、部分的なリロードは無言で壊れてしまいます。
- **コンポーネントの存在は、コンパイル時にチェックされます。** このマクロは、`frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`を探します。一致するファイルがなければ、ディスク上の実際のファイル名から得られる「did you mean…?」という提案とともに、ビルドが失敗します。入れ子になったパスも同じように機能します - `inertia_response!(&req, "Admin/Dashboard", …)`は、`frontend/src/pages/Admin/Dashboard.svelte`（あるいはあなたのフロントエンドの拡張子）へ解決されます。
- **このマクロは、`await`された`Result`へ展開されます。** あなたのハンドラは、[`Response`](error-model.md)（つまり`Result<HttpResponse, HttpResponse>`）か、`?` / `From`を通じて`FrameworkError`を吸収する別の型を返さなければなりません。プロップのシリアライズやレスポンスの構築の途中の失敗は、パニックではなく`Err`として返されます。

### JSON形式のプロップ

プロトタイピングや小さなページでは、型付きの構造体を省略できます。

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

このマクロは、それでもコンポーネントファイルを検証します。トレードオフは、型付きプロップの連鎖を失うことです - `#[derive(InertiaProps)]`もなく、自動的なTypeScript生成もなく、フロントエンドが期待する形が一致することのコンパイル時チェックもありません。

### 任意の設定オーバーライド

このマクロは、レスポンスごとのオーバーライド（異なるSSR設定、1つのページ用のカスタムなデフォルトタイトル）のために、末尾に任意の`InertiaConfig`を受け付けます。

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

ほとんどのアプリは、[`Inertia::install`](#ブートストラップ-inertia-install)を介して、起動時に単一の設定を登録するだけで、この引数に触ることはありません。

## `#[derive(InertiaProps)]`

`InertiaProps`は、キー名があなたのフィールド名と一致する`Serialize`のimplを生成します。これが存在するのは、型付きプロップの経路を簡潔なままにし、TypeScriptジェネレーター（`suprnova generate-types`）が見つけるためのマーカーを持てるようにするためです。

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
pub struct UserProps {
    pub name: String,
    pub email: String,
    pub role: String,
    pub is_active: bool,
}
```

入れ子になった型は、普通に合成されます - フィールドは`Vec<T>`、`Option<T>`、入れ子になった構造体、`Serialize`可能なものなら何でもかまいません。入れ子になった型自体は、`InertiaProps`をderiveする必要はありません - `Serialize`だけが必要です。*トップレベルの*プロップ構造体に`#[derive(InertiaProps)]`を使えば、ツリー全体について、自動的なTypeScriptの表面（[TypeScript 型](frontend-typescript-types.md)を参照）が手に入ります。

## `InertiaResponse` のビルダー

このマクロは、eagerな型付きプロップをカバーします。それ以外のすべて - lazy、optional、deferred、マージ可能、クライアント側にキャッシュされるもの、flash、履歴暗号化のオーバーライド - は、ビルダーを直接使います。

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: クロージャは、プロップが実際に送信されるときにだけ実行される
        // （初回訪問、あるいはこのキーを要求する部分的なリロード）。
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: 初回訪問では決して送信されない。クライアントは
        // X-Inertia-Partial-Data経由で、そのキーを明示的に要求しなければならない。
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: 初回のレンダリングではスキップされる。クライアントが
        // フォローアップのXHRを発行し、そのときにクロージャが実行される。
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: 部分的なリロードで既存のものへ追記する（「もっと読み込む」）。
        .merge("rows", next_page().await?)
        // Once: ナビゲーションをまたいでクライアント側にキャッシュされる。サーバーが
        // 更新を強制しない限り、以降の訪問ではリゾルバはスキップされる。
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: 一度だけのトースト。`props`ではなく`page.flash`の下に現れる。
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| メソッド | 目的 | Laravelでの対応 |
|---|---|---|
| `.with(k, v)` | Eagerなプロップ。部分的なリロードのフィルタリングを尊重する | 型付きプロップ |
| `.always(k, v)` | Eagerなプロップ。部分的なリロードのフィルタを無視する | `Inertia::always(…)` |
| `.lazy(k, ‖)` | リゾルバは、プロップが送信されるときにだけ実行される | `fn () => …`クロージャ |
| `.optional(k, ‖)` | 初回訪問では決して送られない。明示的に要求されなければならない | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | 初回訪問ではスキップされる。フォローアップのXHRが解決を引き起こす | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | 部分的なリロードで、既存のクライアントの状態と結合する | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | クライアントがナビゲーションをまたいでキャッシュする | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate`（`Inertia::paginate`経由） | 無限スクロールのページネーション | `Inertia::scroll(…)` |
| `.flash(k, v)` | `props`ではなく`page.flash`の下にある、一度だけの値 | `session()->flash(…)` |
| `.title(…)` | HTMLシェルのデフォルトの`<title>` | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | レスポンス単位の履歴暗号化 | `Inertia::encryptHistory(…)` |
| `.clear_history()` | 履歴キーのローテーションを強制する | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Inertia訪問の後も`#fragment`を保つ | `Inertia::preserveFragment()` |

Eagerなビルダーメソッドには、値の`Serialize`実装が実行時に失敗しうる場合に`Result<Self, FrameworkError>`を返す、`try_*`という兄弟（`try_with`、`try_always`、`try_merge_with`、`try_scroll`、`try_flash`）があります - 失敗を明示的に扱いたい場合は`try_*`に手を伸ばしてください。無条件に成功する方のメソッドは、パニックを[パニック境界](error-model.md)経由で500へ変換します。

### マージ戦略と無限スクロール

`.merge`（追記）、`.merge_prepend`、`.deep_merge`は、よくある「もっと読み込む」というケースをカバーします。diffマージ - クライアントがすでに持っている行を複製するのではなく更新する - を行うには、`match_on`キーを持つ明示的な`MergeStrategy`を伴う`.merge_with`を使ってください。

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // 新しいページのスライス
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on`は、クライアントが重複排除の基準とするフィールドを名指しします（ページオブジェクトへ`matchPropsOn`として出力されます）。そのため、現在のウィンドウと重なる再取得は、コピーを追記するのではなく、一致する行をその場で置き換えます。`Prepend`と`Deep`も、同じ`match_on`を受け取ります。

無限スクロールは、同じ仕組みにページネーションのメタデータが添付されたものです。`.scroll` / `.scroll_with` - あるいは、`LengthAwarePaginator`や`CursorPaginator`を直接適合させる`.paginate` - は、データの隣に`scrollProps`を出力し、クライアントの`<InfiniteScroll>`コンポーネントが、次/前の取得を駆動します。

```rust
// `posts`は、クエリビルダーからのCursorPaginatorである。
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

フレームワークは、クライアントが送る`X-Inertia-Infinite-Scroll-Merge-Intent`リクエストヘッダーから、マージの方向を読み取ります（下へスクロールしているときは`append`、上へスクロールしているときは`prepend`）。新規の訪問では - intentヘッダーがない場合 - `scrollProps["posts"].reset`は`true`になります。そのため、クライアントは最初のウィンドウをレンダリングする前に、自分のアキュムレータをクリアします。

## 部分的なリロード

Inertia 3のクライアントは、ページのプロップの一部だけを要求できます（あるいは、OptionalやDeferのキーを含めることで、それ以上を要求することもできます）。このプロトコルは、3つのリクエストヘッダーを使います。

| ヘッダー | 意味 |
|---|---|
| `X-Inertia-Partial-Component` | 部分的にリロードされているコンポーネントです - フィルタリングが適用されるには、レスポンスのコンポーネントと一致していなければなりません。 |
| `X-Inertia-Partial-Data` | 許可リスト: 含めるプロップのキーを、カンマ区切りで指定します。 |
| `X-Inertia-Partial-Except` | 拒否リスト: 除外するプロップのキーを、カンマ区切りで指定します。キーが衝突した場合、`Partial-Data`より優先されます。 |

フィルタリングのルール:

- `Eager`、`Lazy`、`Merge`、`Once`、`Scroll`のプロップは、許可リスト / 拒否リストのセマンティクスに従います。
- `Always`のプロップは、無条件に送信されます。
- `Optional`と`Defer`のプロップは、通常の訪問では決して現れず、そのキーを明示的に指定する、一致する部分的なリロードでのみ現れます。

ハンドラは、何も特別なことをする必要はありません - すべてのプロップをビルダーを通じて登録するだけで、フレームワークはページオブジェクトをシリアライズするときにヘッダーを参照します。

## `App::inertia_share*`による共有データ

いくつかのプロップは、どのInertiaページでも同じです - 認証状態、CSRFトークン、現在のロケール、アプリ全体のフラグなどです。ブートストラップで一度登録すれば、それらはすべてのレスポンスにマージされます。

```rust
use suprnova::App;
use std::sync::Arc;

pub fn register() {
    // 同期。起動時に一度だけ具現化される。
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // 非同期。レスポンス単位で解決される（そのキーを除外する
    // 部分的なリロードではスキップされる）。
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // ナビゲーションをまたいでクライアント側にキャッシュされる - `share_once`は、
    // それを必要とする最初のページで実行され、その後クライアントは、キャッシュキーが
    // 変わるまで`X-Inertia-Except-Once-Props`経由で再解決をスキップする。
    App::inertia_share_once("plans", || async {
        Ok::<_, suprnova::FrameworkError>(load_plan_catalog().await?)
    });
}
```

リクエストごとの共有データ（認証済みユーザー、リクエストスコープのフラグ）については、[`InertiaSharedData`](#リクエストごとの共有データ)を実装し、そのシングルトンを登録してください - フレームワークは、あらゆるInertiaレスポンスで`share(&req)`を呼び出し、その結果をマージします。

### キー衝突時の優先順位

同じキーが複数の層に現れる場合、後から書き込まれたものが勝ちます。

1. 静的なレジストリ（`App::inertia_share` / `App::inertia_share_lazy`）
2. リクエストごとのトレイトプロバイダー（`InertiaSharedData::share`）
3. レスポンスごとのビルダーメソッド（`.with`、`.lazy`など）

これによって、ハンドラは、何も登録解除することなく、グローバルに共有されたデフォルトを、1つのページのために上書きできます。

### リクエストごとの共有データ

このトレイトは、Inertiaレスポンスごとに一度、リクエストへのアクセスを持って実行されます。実装には、`async_trait`（`suprnova::__async_trait`として再エクスポート）と`IndexMap`（`suprnova::indexmap`として再エクスポート）が必要です。

```rust
use suprnova::{
    App, Auth, FrameworkError, InertiaRequestExt, InertiaSharedData, Prop,
    indexmap::IndexMap,
};
use std::sync::Arc;

pub struct AuthShare;

#[suprnova::__async_trait]
impl InertiaSharedData for AuthShare {
    async fn share(
        &self,
        _req: &dyn InertiaRequestExt,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::Eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        Ok(out)
    }
}

// ブートストラップの中で:
App::register_inertia_shared(Arc::new(AuthShare));
```

## フラッシュとリダイレクト

フラッシュデータは、次のレンダリングで現れ、その後は消えるべき、一度だけの状態です - トーストメッセージ、「作成したばかり」のID、バリデーションの要約などです。Suprnovaは、あらゆるInertiaレスポンスで、それを`page.flash`の下に表に出します。書き込み手段は3つあります。

```rust
// 1. 現在のリクエストのフラッシュバッグへ積む。
App::flash("toast", "Saved");

// 2. 特定のレスポンスに添付する（このレスポンスだけに同じ効果がある）。
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Redirectファサードを介して、リダイレクトをまたいで運ぶ。
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

`Redirect::with(key, value)`という形は、ハンドラをまたぐ経路です - 値はセッションの中の`_flash.new.*`に着地し、次のリクエストの[`SessionMiddleware`](csrf.md)がそれを`_flash.old.*`へ移し替え、送り先の`InertiaResponse`がそれを`page.flash`の下に表に出します。

同一リクエストのフラッシュ（タスクローカルなバッグ）は、キーが衝突した場合、継承されたセッションのフラッシュに優先します。そのため、送り先のハンドラは、そのキーを再びフラッシュするだけで、届いた値を上書きできます。

内部のセッションキー（`_`で始まるものすべて）は、`page.flash`から除外されます - フォームの再投入のための`_old_input`や、`_inertia.*`のプロトコルフラグは、クライアントへ漏れません。

### リダイレクトヘルパー

`Redirect`は、Laravelの表面全体です。

```rust
Redirect::to("/dashboard")                       // パスへの302
Redirect::route("posts.show").with("id", "42")   // 名前付きルート、ルートパラメータ
Redirect::back("/")                              // セッションに記録された直前のURL
Redirect::refresh()                              // 同じURL、新規のGET
Redirect::guest(&req, "/login")                  // 意図された遷移先URLを退避する
Redirect::intended("/dashboard")                 // 退避されたURLを取り出す
Redirect::signed_route("downloads.show", &[("id","42")])?  // 署名付きURL
Redirect::to("/posts/42").preserve_fragment()    // 訪問をまたいで#fragを保つ
```

`Redirect`のすべてのバリアントは、`.with(k, v)`、`.with_input(map)`、`.with_errors(map)`、`.with_errors_bag(name, map)`、`.cookie(c)`、`.header(k, v)`、`.permanent()`、`.status(303)`などを受け付けます。この一連のチェーンは、Laravelの`RedirectResponse`を反映しています。

非GETのInertia訪問については、[`Inertia303Middleware`](#ブートストラップ-inertia-install)がインストールされていると、フレームワークはレスポンスを自動的に`303 See Other`へ変換します。そのため、ブラウザーは、元のPUT/PATCH/DELETEをリダイレクト先へ再送信するのではなく、きれいなフォローアップのGETを発行します。

## バージョン検出

Inertiaはアセットマニフェストをバージョン管理します。そうしないと、長く生き続けるクライアントが、今日のサーバーに対して昨日のバンドルからページをマウントしようとしてしまいます。クライアントの`X-Inertia-Version`ヘッダーがサーバーの設定されたバージョンと一致しない場合、[`InertiaVersionMiddleware`](#ブートストラップ-inertia-install)は`409 Conflict`と、新しいURLを名指しする`X-Inertia-Location`ヘッダーで応答します - Inertiaクライアントはそれを受け取り、フルページのリロードを行い、新しいバンドルを取得します。

バージョンは`InertiaConfig`を通じて設定します。

```rust
use suprnova::InertiaConfig;

// 静的 - ほとんどのアプリ向け。ビルド時の識別子を焼き込む。
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// 動的 - マニフェストのハッシュ、コンテナのデプロイID、何でも読み取る。
// クロージャはバージョンチェックのたびに実行される。安くなければ内部でキャッシュする。
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

非同期、あるいは失敗しうるバージョン解決（たとえば、S3からマニフェストのハッシュを読み取る場合）については、起動時に一度だけ読み取りを行い、キャッシュした`String`を`.version(...)`に渡してください。

## ブートストラップ: `Inertia::install`

ほとんどのアプリは、1回の呼び出しで2つのプロトコルミドルウェアをインストールします。

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register() -> Result<(), suprnova::FrameworkError> {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)?;
    // …その他の共有データ、ルートなど。
    Ok(())
}
```

`Inertia::install`は`Result`を返し、順番に次のことを行います。

1. `cfg`が本番環境モード（`development == false` - `APP_ENV=production`のときのデフォルト）に解決されるが、`cfg.manifest_path`からViteのマニフェストを読み込めない場合、クローズされて失敗します。これはCFG-01ガードです - ビルドされていないフロントエンドを伴う本番環境の起動は、レガシーなハードコードされたアセットパスへ無言でフォールバックするのではなく、はっきりとエラーになります。
2. `InertiaVersionMiddleware`を登録します - クライアントとサーバーがアセットのバージョンについて一致しないとき、`409` + `X-Inertia-Location`を発行します。
3. `Inertia303Middleware`を登録します - 非GETのInertiaリダイレクトで、`302`を`303`へアップグレードします。

この呼び出しをスキップするのは、これらのミドルウェアのどちらかを本当に望まない場合だけにしてください（まれなケースです - どちらも、本物の失敗モード - 無言の古いバンドルと、リダイレクト時のフォーム再送 - を塞いでいます）。

## サーバー主導の`<head>`要素

Inertia 3.5は、`<head>`に何を入れるかをサーバーに決めさせるクライアントオプションを追加しました - たった今読み込んだレコードにメタタグが依存していて、titleとOGタグを2つの場所に置きたくない場合に便利です。

これには、フレームワーク側の対応は一切必要ありません。クライアントは、**ただのプロップ**からその要素を読み取ります。そのため、どんなハンドラでもそれらを供給できます。

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>) -> Response {
    Ok(inertia_response!("Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    }))
}
```

クライアント側でオプトインします。

```js
createInertiaApp({
  serverHead: true,        // `head`プロップを読み取る
  // serverHead: 'meta',   // あるいは、別の名前のプロップを読み取る
  // serverHead: (page) => [...],  // あるいは、ページ全体から計算する
})
```

各文字列は、1つのHTML要素です。クライアントは、`data-inertia`属性を持たないものすべてにそれを刻み込みます。そうすることで、ナビゲーションをまたいでhead要素をdiffできるからです。位置によるマッチングではなく、安定した識別子が欲しい場合は、自分自身で`data-inertia="og-title"`を指定してください。

ユーザーデータから補間するものは、必ずエスケープしてください - これらの文字列はHTMLとして注入されるため、いつもと同じルールが適用されます。

## SSR

Suprnovaは、プロセス外のSSRワーカー - 典型的には、Node / Bun / Denoの下で実行される`@inertiajs/{svelte,react,vue}/server`の`createServer()`バンドルです - と、HTTPループバックを介して通信します。設定で有効にします。

```rust
InertiaConfig::new()
    .ssr("http://127.0.0.1:13714")  // ワーカーのURL
    .ssr_timeout(std::time::Duration::from_millis(500))
    .ssr_exclude("/admin/**")
    .ssr_max_response_bytes(8 * 1024 * 1024)
```

SSRはデフォルトで無効です。有効にすると、フレームワークはページオブジェクトを`<url>/render`へPOSTし、`{ head, body }`をHTMLシェルにインライン化します。ワーカーのエラーやタイムアウトが起きると、レスポンスはCSR（クライアントがハイドレートする、空の`<div id="app">`）へフォールバックし、`on_ssr_error(...)`フックが発火します。CIでは`ssr_throw_on_error(true)`を切り替えて、代わりにこれらの失敗をハードな500にしてください。

ワーカーは別に起動します - プロジェクトがSSRのエントリーを出荷したら、`suprnova ssr:start`が標準的なランナーです。

## 設定

Inertiaの挙動は、`InertiaConfig`を介してプログラム的に設定されます。フレームワークが直接読み取る唯一の環境変数は`SUPRNOVA_FRONTEND`（`svelte` / `react` / `vue`）であり、デフォルトのエントリーポイントのファイル名とページコンポーネントの拡張子を選びます。それ以外のすべては、ビルダーの形をしています。

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)              // SUPRNOVA_FRONTENDを上書きする
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)            // レイジープロップのファンアウトに上限を設ける
    .production();                           // false → Vite開発サーバーから読み込む
```

フロントエンドごとのデフォルト:

| フロントエンド | デフォルトのエントリーポイント | ページの拡張子 |
|---|---|---|
| Svelte（デフォルト） | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`, `.jsx` |
| Vue | `src/main.ts` | `.vue` |

`manifest_path`にあるViteのマニフェストは、最初のリクエストで遅延ロードされ、プロセスの生存期間にわたってキャッシュされます。それが見つからない場合、本番環境のアセットタグはハードコードされたレガシーなパスへフォールバックし、`tracing::warn!`が発火して、その欠落がログに現れるようにします。

### Suprnovaが異なる設計を選んだ理由

LaravelのInertiaアダプターは、単一のグローバルな「共有データ」レジストリと、リクエストごとの`Inertia::share($k, $v)`呼び出しを持っています。PHPのリクエストごとにプロセスが立つモデルは、これを安全にしています - リクエストごとに新しいプロセスが立つということは、並行する訪問者の間で漏れが起きないということです。

Rustのプロセスモデルはその正反対です - 1つのプロセスが、多くのスレッドをまたいで、多くの並行リクエストを処理します。そのため、レジストリは（プロセスグローバルな静的変数ではなく）[コンテナ](container.md)（タスクローカル → スレッドローカル → グローバル）の上に存在します。`App::inertia_share*`は、アクティブなコンテナの`InertiaRegistry`へ書き込みます。これによって、`TestContainer::fake()`を使うテストは、何も登録解除することなく、きれいな分離を得られます。表面はLaravelと同じですが、ランタイムが異なるため、内部の仕組みは異なります。

他に2つ、Rustらしい選択として触れておく価値があるものがあります。

- **Lazy-propのリゾルバは並行して実行されます**。`max_concurrent_resolvers`（デフォルト16）で上限が定められます。12個のレイジープロップを持つページは、1つのTokioタスクの中で12個の並列クエリを発行します - これこそ、私たちがフレームワークをTokioの上に構築した理由です。それぞれが外部サービスへアクセスする、レイジープロップを多く持つページでは、この上限を調整してください。
- **コンパイル時のコンポーネントチェック**は、そもそもLaravelの機能ではありません。PHPは、コンパイル時にあなたのフロントエンドのファイルを見ることができないからです。Suprnovaはそれができるため、`inertia_response!("Dashbaord", …)`のようなタイプミスは、後から実行時の「コンポーネントが見つかりません」として表に出るのではなく、"did you mean Dashboard?"という提案とともにビルドを失敗させます。

## 次のステップ

- [ページ コンポーネント](frontend-pages.md) - フロントエンドが、コンポーネント名をSvelte / React / Vueのモジュールへ解決する仕組み
- [TypeScript 型](frontend-typescript-types.md) - `suprnova generate-types`が、`#[derive(InertiaProps)]`構造体からTS定義を出力すること
- [データ オブジェクト](data.md) - 部分的なリロードと合成される、フィールドごとのinclude / 許可リストのゲーティングを備えたDTO用の`#[derive(Data)]`
- [エラー モデル](error-model.md) - `Response`、パニック境界、`FrameworkError`が、Inertiaレスポンスをどのように通り抜けるか
- [サービス コンテナ](container.md) - `App::inertia_share*`と`InertiaSharedData`の背後にあるルックアップモデル
