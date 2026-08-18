# Inertia レスポンス

Inertiaレスポンスは、Suprnovaのハンドラが状態をSvelte / React / Vueのページコンポーネントへ送り出す方法です。Inertiaページをレンダリングするすべてのハンドラは、[`inertia_response!`](#inertia-response-マクロ)マクロ（型付きで、コンパイル時に検査されるeagerなプロップ向け）か、[`InertiaResponse`](#inertiaresponse-のビルダー)ビルダー（それ以外のすべて - レイジープロップ、ディファードプロップ、マージ、once、スクロール、フラッシュ）のいずれかを通じて組み立てられた、レスポンスを1つ返します。この章は、レスポンスの表面をエンドツーエンドで扱います - マクロ、ビルダー、v3プロトコルの機能（部分的なリロード、履歴の暗号化、バージョン検出）、`App::inertia_share*`による共有データ、そしてリダイレクトをまたいで運ばれるフラッシュバッグです。

まだフロントエンドを選んでいない場合は、[フロントエンド 概要](frontend.md)と[ページ コンポーネント](frontend-pages.md)を先に読んでください。この章は、SPAブリッジが配線済みであることを前提とし、あなたのハンドラが何を返すかに焦点を当てます。

## `inertia_response!` マクロ

このマクロは、ハンドラから型付きでeagerなページへ至る最短の経路です。現在のリクエスト、コンポーネント名、そしてプロップの式を取ります:

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

知っておくべきことが3つあります:

- **先頭の `&req` は必須です。** マクロは、リクエストから `X-Inertia` のヘッダー、URL、そして部分的なリロードのフィルタリング用ヘッダーを読み取るため、リクエストの値（またはその参照）を必要とします。これがなければ、部分的なリロードは静かに壊れてしまいます。
- **コンポーネントの存在は、コンパイル時に検査されます。** マクロは `frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}` を探します。一致するファイルがなければ、ディスク上の実際のファイル名から取られた「もしかして…?」という提案とともに、ビルドが失敗します。入れ子になったパスも同じように動作します - `inertia_response!(&req, "Admin/Dashboard", …)` は `frontend/src/pages/Admin/Dashboard.svelte`（またはあなたのフロントエンドの拡張子）を解決します。
- **マクロは、`await` された `Result` へ展開されます。** あなたのハンドラは、[`Response`](error-model.md)（これは `Result<HttpResponse, HttpResponse>` です）か、`?` / `From` を通じて `FrameworkError` を吸収する別の型を返さなければなりません。プロップのシリアライゼーションやレスポンスの構築の途中での失敗は、パニックではなく `Err` として返されます。

### JSON形式のプロップ

プロトタイピングや小さなページでは、型付きの構造体を省略できます:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

マクロは、それでもコンポーネントのファイルを検証します。トレードオフは、型付きプロップの連鎖を失うことです - `#[derive(InertiaProps)]` もなく、自動的なTypeScriptの生成もなく、フロントエンドが期待する形と一致するかのコンパイル時の検査もありません。

### 任意の設定オーバーライド

マクロは、レスポンスごとの上書き（異なるSSRの設定、1ページだけのカスタムなデフォルトタイトルなど）のために、末尾に任意の `InertiaConfig` を受け付けます:

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

ほとんどのアプリは、[`Inertia::install`](#ブートストラップ-inertia-install)経由で起動時に単一の設定を登録し、この引数に触れることは決してありません - インストールされた設定が、既にすべてのレスポンスの出発点だからです。1つのページについてインストール済みの設定を上書きしたいときにだけ、ここで渡してください。

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

マクロは、eagerな型付きプロップをカバーします。それ以外のすべて - レイジー、オプショナル、ディファード、マージ可能、クライアント側にキャッシュされるもの、フラッシュ、履歴の暗号化の上書き - は、ビルダーを直接使います:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: そのプロップが実際に送られるときにだけクロージャが走る
        //（初回訪問、またはこのキーを要求する部分的なリロード）。
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: 初回訪問では決して送られない。クライアントは
        // X-Inertia-Partial-Data 経由で、明示的にそのキーを要求しなければならない。
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: 初回のレンダリングではスキップされる。クライアントが追いかけの
        // XHRを発行し、そのときクロージャが走る。
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: 部分的なリロードで既存へ追加する（「もっと読み込む」）。
        .merge("rows", next_page().await?)
        // Once: ナビゲーションをまたいでクライアント側にキャッシュされる。サーバーが更新を
        // 強制しない限り、以降の訪問ではリゾルバがスキップされる。
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: ワンショットのトースト。`props` ではなく `page.flash` の下に現れる。
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| メソッド | 目的 | Laravelとの対応 |
|---|---|---|
| `.with(k, v)` | eagerなプロップ。部分的なリロードのフィルタリングを尊重する | 型付きプロップ |
| `.always(k, v)` | eagerなプロップ。部分的なリロードのフィルタを無視する | `Inertia::always(…)` |
| `.lazy(k, ‖)` | プロップが実際に送られるときにだけリゾルバが走る | `fn () => …` のクロージャ |
| `.optional(k, ‖)` | 初回訪問では決して送られない。明示的に要求されなければならない | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | 初回訪問ではスキップされる。追いかけのXHRが解決を引き起こす | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | 部分的なリロードで、既存のクライアントの状態と結合する | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | クライアントがナビゲーションをまたいでキャッシュする | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.paginate`（`Inertia::paginate` 経由） | 無限スクロールのページネーション | `Inertia::scroll(…)` |
| `.flash(k, v)` | `page.flash` の下のワンショットの値（`props` ではない） | `session()->flash(…)` |
| `.title(…)` | HTMLシェルのデフォルトの `<title>` | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | レスポンスごとの履歴の暗号化 | `Inertia::encryptHistory(…)` |
| `.clear_history()` | **この**ページで履歴キーのローテーションを強制する | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Inertiaの訪問の後も `#fragment` を保つ | `Inertia::preserveFragment()` |

eagerなビルダーのメソッドには、値の `Serialize` の実装が実行時に失敗しうるときに `Result<Self, FrameworkError>` を返す `try_*` の兄弟（`try_with`、`try_always`、`try_merge_with`、`try_scroll`、`try_flash`）があります - 失敗しないほうのメソッドは、[パニック境界](error-model.md)を介してパニックを500へ変換するため、失敗を明示的に扱いたいときは `try_*` に手を伸ばしてください。

`.clear_history()` は、あなたが構築しているレスポンスに印を付けます。ログアウトのハンドラはリダイレクトし、ブラウザはそのリダイレクトのレスポンスを捨てます - そのため、フラグを運ばなければならないのは、ログアウトのレスポンスではなくログインのページのほうです。`App::clear_history()` が、そのケースの解決策です - これはビルダーのメソッドではなく自由関数であるため、上の表には載っていません。これは、次のInertiaのページオブジェクトが `clearHistory: true` に変える、ワンショットのセッションフラグをフラッシュします。セッションのスコープを必要とし、ちょうど1ホップだけ生き延びます。

これは `Auth::logout()` / `Auth::logout_and_invalidate()` の**後**に呼び出してください - 前ではありません。無効化はセッション全体を消去し、フラグはそのセッションの中に存在するため、先にフラッシュしても、消去によって消されるだけです:

```rust
use suprnova::{App, Auth, Redirect, Response};

pub async fn logout() -> Response {
    Auth::logout_and_invalidate().await?;
    App::clear_history();
    Redirect::to("/login").into()
}
```

### マージ戦略と無限スクロール

`.merge`（末尾に追加）、`.merge_prepend`、`.deep_merge` は、よくある「もっと読み込む」のケースをカバーします。差分マージ - クライアントが既に保持している行を、複製するのではなく更新すること - を行うには、`match_on` のキーを運ぶ明示的な `MergeStrategy` とともに `.merge_with` に手を伸ばしてください:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // 新しいページのスライス
        MergeStrategy::Append { match_on: Some("id".into()) },
    )
```

`match_on` は、クライアントが重複排除に使うフィールドを名指しします（ページオブジェクトへは `matchPropsOn` として出力されます）。そのため、現在のウィンドウと重なる再取得は、コピーを追加するのではなく、一致する行をその場で置き換えます。`Prepend` と `Deep` も、同じ `match_on` を取ります。

無限スクロールは、ページネーションのメタデータが付いた、同じ機構です。`.scroll` / `.scroll_with` - あるいは、`LengthAwarePaginator` や `CursorPaginator` を直接適応させる `.paginate` - は、データの隣に `scrollProps` を出力し、クライアントの `<InfiniteScroll>` コンポーネントが次/前の取得を駆動します:

```rust
// `posts` は、クエリビルダー由来の CursorPaginator。
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

フレームワークは、クライアントが送る `X-Inertia-Infinite-Scroll-Merge-Intent` リクエストヘッダーからマージの方向を読み取ります（下へスクロールしているときは `append`、上へスクロールしているときは `prepend`）。新規の訪問 - intentのヘッダーなし - では、`scrollProps["posts"].reset` が `true` になるため、クライアントは最初のウィンドウをレンダリングする前に、自分のアキュムレータをクリアします。

## 部分的なリロード

Inertia 3のクライアントは、ページのプロップの部分集合を（あるいは、OptionalやDeferのキーを含めることで上位集合を）要求できます。プロトコルは、3つのリクエストヘッダーを使います:

| ヘッダー | 意味 |
|---|---|
| `X-Inertia-Partial-Component` | 部分的にリロードされているコンポーネント - フィルタリングが適用されるには、レスポンスのコンポーネントと一致しなければなりません。 |
| `X-Inertia-Partial-Data` | 許可リスト: 含めるプロップのキーを、カンマ区切りで指定します。 |
| `X-Inertia-Partial-Except` | 拒否リスト: 除外するプロップのキーを、カンマ区切りで指定します。キーが衝突したときは `Partial-Data` に勝ちます。 |

フィルタリングの規則:

- `Eager`、`Lazy`、`Merge`、`Once`、`Scroll` のプロップは、許可リスト / 拒否リストのセマンティクスに従います。
- `Always` のプロップは、いずれにせよ送られます。
- `Optional` と `Defer` のプロップは、標準の訪問では決して送られず、そのキーを明示的に列挙する、一致する部分的なリロードでのみ現れます。

ハンドラは、特別なことを何もする必要はありません - すべてのプロップをビルダーを通じて登録すれば、フレームワークがページオブジェクトをシリアライズするときにヘッダーを参照します。

`once` のプロップのクライアント側のキャッシュが尊重されるのは、**完全な**Inertiaの訪問のときだけです。そのキーを名指しする部分的なリロード（`router.reload({ only: ['stats'] })`）では、リゾルバが走り、値が送られます - クライアントは、まさに新しいものが欲しいからこそ要求したのであり、そこで古いキャッシュの主張を尊重すれば、要求されたキーについて何も返さないことになるからです。

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

フラッシュデータは、次のレンダリングで現れ、その後は消えるべきワンショットの状態です - トーストのメッセージ、「たった今作成された」ID、バリデーションのまとめなどです。Suprnovaは、それをすべてのInertiaレスポンスの `page.flash` の下に表面化させます。書き手は3つあります:

```rust
// 1. 現在のリクエストのフラッシュバッグへ押し込む。
App::flash("toast", "Saved");

// 2. 特定のレスポンスへ取り付ける（このレスポンスにだけ同じ効果）。
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Redirect ファサード経由で、リダイレクトをまたいで運ぶ。
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

`Redirect::with(key, value)` の形は、ハンドラをまたぐ経路です: その値はセッションの `_flash.new.*` の下に着地し、次のリクエストの[`SessionMiddleware`](csrf.md)がそれを `_flash.old.*` へと歳を取らせ、行き先の `InertiaResponse` がそれを `page.flash` の下に表面化させます。

同一リクエストのフラッシュ（タスクローカルのバッグ）は、キーが衝突したときに、継承されたセッションのフラッシュに勝ちます。そのため、行き先のハンドラは、そのキーを再フラッシュするだけで、入ってきた値を上書きできます。

内部のセッションキー（`_` が前置されたものすべて）は、`page.flash` から取り除かれます - フォームの再投入のための `_old_input` と、`_inertia.*` のプロトコルフラグは、クライアントへ漏れません。

### リダイレクトのヘルパー

`Redirect` は、Laravelの完全な表面です:

```rust
Redirect::to("/dashboard")                       // パスへの302
Redirect::route("posts.show").with("id", "42")   // 名前付きルートとルートパラメータ
Redirect::back("/")                              // セッションに記録された直前のURL
Redirect::refresh()                              // 同じURLへ、新しいGET
Redirect::guest(&req, "/login")                  // 目的のURLを退避する
Redirect::intended("/dashboard")                 // 退避したURLを取り出す
Redirect::signed_route("downloads.show", &[("id","42")])?  // 署名付きURL
Redirect::to("/posts/42").preserve_fragment()    // 訪問をまたいで #frag を保つ
```

すべての `Redirect` の変種は、`.with(k, v)`、`.with_input(map)`、`.with_errors(map)`、`.with_errors_bag(name, map)`、`.cookie(c)`、`.header(k, v)`、`.permanent()`、`.status(303)` などを受け付けます。連鎖の全体は、Laravelの `RedirectResponse` をミラーします。

GET以外のInertiaの訪問については、[`Inertia303Middleware`](#ブートストラップ-inertia-install)がインストールされていれば、フレームワークがレスポンスを `303 See Other` へ自動変換します。そのため、ブラウザは、元のPUT/PATCH/DELETEをリダイレクト先へ再送信するのではなく、きれいな追いかけのGETを発行します。

訪問者をInertiaアプリの**外**へ送るには - 決済プロバイダー、OAuthのauthorizeエンドポイント、ホスティングされた請求ポータルなど - `location_for` を使ってください:

```rust
use suprnova::{InertiaResponse, Request, Response};

pub async fn checkout(req: Request) -> Response {
    Ok(InertiaResponse::location_for(&req, "https://billing.example/checkout"))
}
```

InertiaのXHRは `409` + `X-Inertia-Location` を受け取り（クライアントは `window.location = url` を実行します）、ハードナビゲーションは素の `302` + `Location` を受け取ります。裸の `InertiaResponse::location(url)` は、常に409の形を返します - リクエストが既にInertiaの訪問だと分かっている場所でだけ使ってください。`Location` ヘッダーのない `409` に従うブラウザには、行き先がないからです。

## バージョン検出

Inertiaはアセットのマニフェストにバージョンを付けるため、長生きのクライアントが、昨日のバンドルのページを今日のサーバーに対してマウントしようとすることはありません。クライアントの `X-Inertia-Version` ヘッダーが、サーバーの設定済みのバージョンと一致しないとき、[`InertiaVersionMiddleware`](#ブートストラップ-inertia-install)は `409 Conflict` と、新しいURLを名指しする `X-Inertia-Location` ヘッダーで応答します - Inertiaのクライアントはそれを拾い上げ、ページ全体のリロードを行って、新しいバンドルを取得します。

この跳ね返しは、まずセッションを再フラッシュします。クライアントは409にページ全体のGETで応え、そのGETは新しいリクエストです - 再フラッシュがなければ、前のリクエストがフラッシュしたバリデーションエラーや成功メッセージは、行き先のページがそれを読み取る前に歳を取って消えてしまい、ユーザーは、送信の途中でデプロイが着地したというだけの理由でエラーメッセージを失います。これには、バージョンのミドルウェアより前に `SessionMiddleware` が登録されている必要があります。

バージョンは `InertiaConfig` を通じて設定します:

```rust
use suprnova::InertiaConfig;

// 静的 - ほとんどのアプリ。ビルド時の識別子を焼き込む。
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// 動的 - マニフェストのハッシュ、コンテナのデプロイID、何でも読み取る。
// このクロージャはバージョンの検査のたびに走る。安くないなら、内側でキャッシュすること。
let cfg = InertiaConfig::new().version_with(|| current_manifest_hash());
```

非同期あるいは失敗しうるバージョンの解決（例えば、S3からマニフェストのハッシュを読むなど）については、起動時に一度だけ読み取り、キャッシュした `String` を `.version(...)` へ渡してください。

## ブートストラップ: `Inertia::install`

ほとんどのアプリは、3つのプロトコルミドルウェアを1回の呼び出しでインストールします:

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register() -> Result<(), suprnova::FrameworkError> {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)?;
    // …他の共有データ、ルートなど。
    Ok(())
}
```

`Inertia::install` は `Result` を返し、次の順序で処理します:

1. `cfg` が本番モード（`development == false` - `APP_ENV=production` のときは常にこれがデフォルトです）に解決されるにもかかわらず、`cfg.manifest_path` からViteのマニフェストをロードできない場合、フェイルクローズします。これがCFG-01の保護機構です: フロントエンドがビルドされていない状態での本番の起動は、レガシーなハードコードされたアセットパスへ静かにフォールバックするのではなく、はっきりと失敗します。
2. `InertiaHeadersMiddleware` を登録します - すべてのレスポンスに `Vary: X-Inertia` を設定し、Inertiaの訪問での空の `200` を `303` の戻りへ変えます。
3. `InertiaVersionMiddleware` を登録します - クライアントとサーバーがアセットのバージョンで食い違ったときに、`409` + `X-Inertia-Location` を出力します。
4. `Inertia303Middleware` を登録します - GET以外のInertiaのリダイレクトで、`302` を `303` へ格上げします。

順序が重要です: ヘッダーのミドルウェアが最初に登録されるため、それが最も外側になり、すべてのレスポンスを目にします - バージョンのミドルウェアがハンドラの実行前に返す `409` も含めてです。

`install` は、**設定を保持もします**。その後に構築されるすべての `InertiaResponse` は、そこから出発します。そのため、ここで設定した `.frontend(...)`、`.version(...)`、`.default_title(...)`、`.ssr(...)`、`.encrypt_history(...)` は、ハンドラが何も渡さなくても、すべてのページへ届きます。1つのページについて異なる設定が欲しいハンドラは、それでも `.with_config(...)` で上書きできます。`Inertia::install` を一度も呼ばないアプリは `InertiaConfig::default()` を得ます。そして `install` をもう一度呼ぶと、保持されている設定を置き換えます。

`.with_config(...)` は、`version` も含めて設定をまるごと置き換えます。`InertiaVersionMiddleware` は、それでも `Inertia::install` に与えられたバージョンを解決するため、ここでの設定が同じ `.version(...)` を運んでいなければ、ページオブジェクトは、ミドルウェアが跳ね返すことになるバージョンを広告してしまいます - クライアントは、そのページを訪れた後に、ページ全体のロードを余分に1回行うことになります。一致させるには、上書き側にも `.version(...)` を設定してください。

フラッシュデータを使うなら、`SessionMiddleware` を `Inertia::install` **より前に**登録してください。バージョンのミドルウェアは、クライアントを跳ね返す前にセッションを再フラッシュするため、フラッシュされたエラーは、追いかけのページ全体のGETを生き延びます。それができるのは、セッションのスコープの内側だけです。

この呼び出しを省略するのは、これらのミドルウェアのどれかを本当に望まないときだけにしてください（まれです。3つとも、実在する失敗モードを塞いでいます - 1つのURLの2つの表現をまたぐキャッシュポイズニング、静かな古いバンドル、そしてリダイレクト時のフォームの再送信です）。

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

Suprnovaは、プロセス外のSSRワーカー - 典型的には、Node / Bun / Denoの下で走る `@inertiajs/{svelte,react,vue}/server` の `createServer()` バンドル - と、HTTPのループバック越しに話します。[`Inertia::install`](#ブートストラップ-inertia-install)へ渡す設定の上で、それを有効にしてください - その設定がすべてのレスポンスの出発点であるため、あなたのハンドラを通して配管するものは何もありません:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // ワーカーのURL
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

SSRはデフォルトでオフであり、これは設定のプロパティです: インストールされた設定から構築されるすべてのレスポンスではオン、それを設定しない `.with_config(...)` で上書きするレスポンスではオフです。有効なとき、フレームワークはページオブジェクトを `<url>/render` へPOSTし、`{ head, body }` をHTMLシェルの中へインライン展開します。ワーカーのエラーやタイムアウトのときは、レスポンスはCSR（クライアントがハイドレートする空の `<div id="app">`）へフォールバックし、`on_ssr_error(...)` のフックが発火します。CIでは `ssr_throw_on_error(true)` を切り替えて、それらの失敗を代わりに強い500にしてください。

ワーカーは別途起動してください - プロジェクトがSSRのエントリを出荷するようになれば、`suprnova ssr:start` が標準のランナーです。

## 設定

Inertiaの振る舞いは `InertiaConfig` を介してプログラム的に設定され、[`Inertia::install`](#ブートストラップ-inertia-install)へ渡す設定が、すべてのレスポンスの出発点になります。フレームワークが直接読み取る唯一の環境変数は `SUPRNOVA_FRONTEND`（`svelte` / `react` / `vue`）であり、これは設定が何も言わないときにだけ、デフォルトのエントリポイントのファイル名と、ページコンポーネントの拡張子を供給します - インストールされた設定の上の明示的な `.frontend(Frontend::React)` が勝ち、それが `suprnova new --frontend react` のスキャフォルドするものです。それ以外はすべて、ビルダーの形をしています:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // SUPRNOVA_FRONTEND を上書きする
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // レイジープロップのファンアウトの上限
    .url_resolver(|req| req.path_and_query()) // `page.url` の導出のしかた
    .production();                            // false → Vite開発サーバーからロードする
```

フロントエンド固有のデフォルト:

| フロントエンド | デフォルトのエントリポイント | ページの拡張子 |
|---|---|---|
| Svelte（デフォルト） | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`、`.jsx` |
| Vue | `src/main.ts` | `.vue` |

### `url` フィールド

`page.url` は、リクエストのパス**と**クエリ文字列です（`/users?page=2&sort=name`）。クライアントはそれを `history.state` へ書き込むため、戻る/進むのナビゲーションと `router.reload()` が再生するのはこれです - クエリを落とせば、ページネーションされた、あるいはフィルタされたページはすべて、静かに1ページ目へ戻ってしまいます。`InertiaVersionMiddleware` も、リクエストのパスとクエリから `X-Inertia-Location` を導出するため、デフォルトでは、409のアセットバージョンの跳ね返しは、ページオブジェクトが名指ししたのとまったく同じURLへブラウザを着地させます。

クライアントが記録すべきURLが、到着したURLと異なるとき - SPAがルーティングに使わないロケールのプレフィックスや、リバースプロキシが書き換えたパスなど - は、`url_resolver` で導出を上書きしてください:

```rust
use suprnova::InertiaConfig;

let cfg = InertiaConfig::new()
    .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
```

リゾルバは `InertiaRequestExt` を通じてリクエストを読み取り、[`Inertia::install`](#ブートストラップ-inertia-install)へ渡す設定から構築されるすべてのレスポンスに適用されます - アプリ全体に適用されるべきリゾルバの、通常の置き場所です。1つのレスポンスについては、`InertiaResponse::with_config(cfg)` で上書きしてください。リゾルバが変えるのは `page.url` だけです。409の跳ね返しは、実際に到着したURLを名指しし続けます - それがブラウザの取得しなければならないURLだからです - そのため、リゾルバがある場合、この2つは意図的に食い違います。

`manifest_path` にあるViteのマニフェストは、最初のリクエストで遅延ロードされ、プロセスの生存期間の間キャッシュされます - インストールされた設定から構築されるすべてのレスポンスが、その1つのキャッシュを共有するため、ファイルが読み取られてパースされるのは一度だけです。マニフェストが欠けているときは、本番のアセットタグはハードコードされたレガシーなパスへフォールバックし、`tracing::warn!` が発火して、その欠落がログに表面化します。

### Suprnovaが異なる設計を選んだ理由

LaravelのInertiaアダプターは、単一のグローバルな「共有データ」のレジストリに加えて、リクエストごとの `Inertia::share($k, $v)` の呼び出しを持ちます。PHPの、リクエストごとにプロセスというモデルが、これを安全にしています: リクエストごとに新しいプロセスということは、並行する訪問者の間に漏れがないということです。

Rustのプロセスモデルは正反対です - 1つのプロセスが、多数のスレッドをまたいで多数の並行リクエストを処理します。そのため、レジストリはプロセスグローバルなstaticではなく、[コンテナ](container.md)（タスクローカル → スレッドローカル → グローバル）の上に存在します。`App::inertia_share*` は、アクティブなコンテナの `InertiaRegistry` へ書き込みます。これによって、`TestContainer::fake()` を使うテストは、何も登録解除することなく、きれいな隔離を得られます。Laravelと同じ表面ですが、ランタイムが違うため、その下の機構は違います。

Rustの形をした、注記に値する他の5つの選択:

- **レイジープロップのリゾルバは並行して走ります。** 上限は `max_concurrent_resolvers`（デフォルトは16）です。12個のレイジープロップを持つページは、1つのTokioタスクの中で12個の並列クエリを発行します - 私たちがTokioの上にフレームワークを構築したのは、まさにそのためです。ページが多数のレイジープロップを持ち、そのそれぞれが外部サービスを叩くなら、この上限を調整してください。
- **コンパイル時のコンポーネントの検査**は、そもそもLaravelの機能ではありません。PHPは、コンパイル時にあなたのフロントエンドのファイルを見ることができないからです。Suprnovaにはそれができるため、`inertia_response!("Dashbaord", …)` のタイプミスは、後になって実行時の「コンポーネントが見つかりません」として表面化するのではなく、「もしかして Dashboard ですか?」という提案とともにビルドを失敗させます。
- **Inertiaの訪問での空の `200` は、`302` ではなく `303` になります。** Laravelの `onEmptyResponse` は `redirect()->back()`（302）を返し、PUT/PATCH/DELETEについてのみ、後段の `302 → 303` の変換に頼ります。置き換えられたリダイレクトは、決して元のメソッドの続きではありません - クライアントはGETを発行しなければなりません - そのため、Suprnovaは、GETの訪問を、クライアントが元の動詞で追いかけてしまう302の上に残すのではなく、直接 `303` と言います。
- **`Inertia::location($url)` は、ここでは1つではなく2つのメソッドです。** `location(url)` は、Laravelの常に `409` という契約を保ちます - これはリクエストを意識する形より前からあり、タグを固定した利用者は、その形が変わらないことに依存しています。`location_for(&req, url)` は、より新しい、リクエストを意識する形です: InertiaのXHRには `409`、ハードナビゲーションには素の `302` です。新しいコードでは `location_for` に手を伸ばしてください。
- **`Inertia::clearHistory()` も、ここでは1つではなく2つのメソッドです。** ビルダー上の `.clear_history()` は単一のレスポンスに印を付け、`App::clear_history()` は、リダイレクトを生き延びるようにフラグをセッションへフラッシュします。Laravelが1つのメソッドで済ませられるのは、それが既にセッションに支えられているからです - Suprnovaは、レスポンスローカルな形をデフォルト（セッションへの依存なし）に保ち、リダイレクトをまたぐケースを、代わりに明示的なオプトインにしています。

## 次のステップ

- [ページ コンポーネント](frontend-pages.md) - フロントエンドが、コンポーネント名をSvelte / React / Vueのモジュールへ解決する仕組み
- [TypeScript 型](frontend-typescript-types.md) - `suprnova generate-types`が、`#[derive(InertiaProps)]`構造体からTS定義を出力すること
- [データ オブジェクト](data.md) - 部分的なリロードと合成される、フィールドごとのinclude / 許可リストのゲーティングを備えたDTO用の`#[derive(Data)]`
- [エラー モデル](error-model.md) - `Response`、パニック境界、`FrameworkError`が、Inertiaレスポンスをどのように通り抜けるか
- [サービス コンテナ](container.md) - `App::inertia_share*`と`InertiaSharedData`の背後にあるルックアップモデル
