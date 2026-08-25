# Inertia レスポンス

Inertiaレスポンスは、Suprnovaのハンドラが状態をSvelte / React / Vueのページコンポーネントへ届ける方法です。Inertiaページをレンダリングするすべてのハンドラは、[`inertia_response!`](#the-inertia_response-macro)マクロ（型付きで、コンパイル時に検査されるeagerなプロップ向け）か、[`InertiaResponse`](#inertiaresponse-のビルダー)ビルダー（それ以外のすべて - レイジープロップ、ディファードプロップ、マージ、once、スクロール、フラッシュ）のいずれかを通じて構築したレスポンスを1つ返します。この章ではレスポンスの表面をエンドツーエンドで扱います: マクロ、ビルダー、v3プロトコルの機能（部分的なリロード、履歴の暗号化、バージョン検出）、`App::inertia_share*`による共有データ、そしてリダイレクトをまたいで運ばれるフラッシュバッグです。

まだフロントエンドを選んでいない場合は、[Frontend Overview](frontend.md)と[Page Components](frontend-pages.md)を先に読んでください。この章はSPAブリッジが配線済みであることを前提とし、ハンドラが何を返すかに焦点を当てます。

## `inertia_response!` マクロ

このマクロは、ハンドラから型付きのeagerなページへ至る最短経路です。現在のリクエスト、コンポーネント名、プロップの式を取ります:

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

- **先頭の`&req`は必須です。** マクロはリクエストから`X-Inertia`ヘッダー、URL、部分的なリロードのフィルタリング用ヘッダーを読み取るため、リクエストの値（または参照）を必要とします。これがなければ、部分的なリロードは静かに壊れます。
- **コンポーネントの存在はコンパイル時に検査されます。** マクロは`frontend/src/pages/<Component>.{svelte,tsx,jsx,vue}`を探します。一致するファイルがなければ、ディスク上の実際のファイル名から取得した「did you mean…?」という提案とともにビルドが失敗します。入れ子になったパスも同じように動作します - `inertia_response!(&req, "Admin/Dashboard", …)`は`frontend/src/pages/Admin/Dashboard.svelte`（またはフロントエンドの拡張子）を解決します。
- **マクロは`await`された`Result`へ展開されます。** ハンドラは[`Response`](error-model.md)（`Result<HttpResponse, HttpResponse>`）か、`?` / `From`を通じて`FrameworkError`を吸収する別の型を返さなければなりません。プロップのシリアライズやレスポンス構築中の失敗は、パニックではなく`Err`として返されます。

ロジックがまったくないページ - about、terms、privacy - なら、ハンドラを完全に省略してルートを宣言できます:

```rust
use suprnova::Router;
use serde_json::json;

let router = Router::new().inertia("/about", "About", json!({ "team_size": 4 }));
```

[Routing](routing.md#router-level-redirects-and-views)を参照してください。そこではコンポーネントが実行時文字列なので、このマクロのコンパイル時存在検査は行われません - ハンドラを書かないこととのトレードオフです。

### JSON形式のプロップ

プロトタイピングや小さなページでは、型付き構造体を省略できます:

```rust
inertia_response!(&req, "Dashboard", {
    "user": { "name": "John" },
    "stats": { "visits": 1234 }
})
```

それでもマクロはコンポーネントファイルを検証します。トレードオフは、型付きプロップの連鎖を失うことです - `#[derive(InertiaProps)]`も、自動的なTypeScript生成も、フロントエンドの期待する形と一致するかのコンパイル時検査もありません。

### 任意の設定オーバーライド

マクロは、レスポンス単位の上書き（異なるSSR設定、1ページだけのカスタムデフォルトタイトル）のため、末尾に任意の`InertiaConfig`を受け付けます:

```rust
let cfg = InertiaConfig::new().default_title("Reports");
inertia_response!(&req, "Reports/Index", props, cfg)
```

ほとんどのアプリは起動時に[`Inertia::install`](#ブートストラップ-inertia-install)経由で単一の設定を登録し、この引数に触れることはありません - インストールされた設定がすでにすべてのレスポンスの出発点だからです。ここで渡すのは、1ページだけインストール済み設定を上書きしたいときに限ります。

## `#[derive(InertiaProps)]`

`InertiaProps`は、フィールド名と一致するキー名の`Serialize` implを生成します。これが存在するのは、型付きプロップ経路を簡潔に保ち、TypeScriptジェネレーター（`suprnova generate-types`）が見つけられるマーカーを持たせるためです:

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

入れ子の型は通常どおり合成されます - フィールドは`Vec<T>`、`Option<T>`、入れ子の構造体など、`Serialize`可能なものなら何でもかまいません。入れ子の型自体は`InertiaProps`をderiveする必要はなく、`Serialize`だけが必要です。*トップレベルの*プロップ構造体に`#[derive(InertiaProps)]`を使うと、ツリー全体について自動的なTypeScript表面（[TypeScript Types](frontend-typescript-types.md)を参照）が得られます。

## `InertiaResponse` のビルダー

マクロはeagerな型付きプロップをカバーします。それ以外のすべて - レイジー、オプショナル、ディファード、マージ可能、クライアント側にキャッシュされるもの、フラッシュ、履歴暗号化の上書き - にはビルダーを直接使います:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: closure runs only when the prop will actually be sent
        // (initial visit, or partial reload that requests this key).
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: never sent on initial visits; the client must
        // explicitly ask for the key via X-Inertia-Partial-Data.
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: skipped on the initial render; the client issues a
        // follow-up XHR and the closure runs then.
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: append-into-existing on partial reloads ("load more").
        .merge("rows", next_page().await?)
        // Once: cached client-side across navigations; resolver skipped
        // on subsequent visits unless server forces refresh.
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: one-shot toast; appears under `page.flash`, not `props`.
        .flash("toast", serde_json::json!({"type":"info","msg":"Saved"}))
        .resolve(&req)
        .await
        .map_err(HttpResponse::from)?;
    Ok(resp)
}
```

| メソッド | 目的 | Laravelとの対応 |
|---|---|---|
| `.with(k, v)` | eagerなプロップ。部分的なリロードのフィルタリングを尊重する | typed prop |
| `.always(k, v)` | eagerなプロップ。部分的なリロードのフィルタを無視する | `Inertia::always(…)` |
| `.always_with(k, ‖)` | 非同期リゾルバ。部分的なリロードのフィルタを無視する | `Inertia::always(fn () => …)` |
| `.lazy(k, ‖)` | プロップが送られるときだけリゾルバが走る | `fn () => …` closure |
| `.optional(k, ‖)` | 初回訪問では決して送られず、明示的に要求する必要がある | `Inertia::optional(…)` |
| `.defer(k, ‖)` / `.defer_with(...)` | 初回訪問ではスキップされ、追いかけのXHRが解決を発生させる | `Inertia::defer(…)` |
| `.merge` / `.merge_prepend` / `.deep_merge` / `.merge_with` | 部分的なリロードで既存のクライアント状態と結合する | `Inertia::merge` / `deepMerge` |
| `.once(k, ‖)` / `.once_with(…)` | クライアントがナビゲーションをまたいでキャッシュする | `Inertia::once(…)` |
| `.scroll` / `.scroll_with` / `.scroll_wrapped` / `.scroll_with_wrapped` / `.paginate`（`Inertia::paginate`経由） | 無限スクロールのページネーション | `Inertia::scroll(…)` |
| `.flash(k, v)` | `page.flash`の下のワンショット値（`props`ではない） | `session()->flash(…)` |
| `.title(…)` | HTMLシェルのデフォルト`<title>` | `Inertia::render(…)->title(…)` |
| `.encrypt_history(bool)` | レスポンス単位の履歴暗号化 | `Inertia::encryptHistory(…)` |
| `.clear_history()` | **この**ページで履歴キーのローテーションを強制する | `Inertia::clearHistory()` |
| `.preserve_fragment(bool)` | Inertia訪問後も`#fragment`を保つ | `Inertia::preserveFragment()` |

Eagerなビルダーメソッドには`try_*`の兄弟（`try_with`、`try_always`、`try_merge_with`、`try_scroll`、`try_scroll_wrapped`、`try_flash`）があります。値の`Serialize` implが実行時に失敗しうるとき、これらは`Result<Self, FrameworkError>`を返します - 失敗しないメソッドは[パニック境界](error-model.md)を介してパニックを500へ変換するため、失敗を明示的に扱いたいなら`try_*`を使ってください。

`.clear_history()`は構築中のレスポンスに印を付けます。ログアウトハンドラはリダイレクトし、ブラウザはリダイレクトのレスポンスを捨てるため、フラグを持つべきなのはログアウトのレスポンスではなくログインページです。`App::clear_history()`がそのケースの修正です - これはビルダーメソッドではなく自由関数なので、上の表にはありません。これは、次のInertiaページオブジェクトが`clearHistory: true`へ変換するワンショットのセッションフラグをフラッシュします。セッションスコープが必要で、ちょうど1ホップだけ存続します。

`Auth::logout()` / `Auth::logout_and_invalidate()`の**後**に呼び出してください。前ではありません - 無効化はセッション全体をフラッシュし、フラグはそのセッションに存在するため、先にフラッシュしてもフラッシュ処理によって消されます:

```rust
use suprnova::{App, Auth, Redirect, Response};

pub async fn logout() -> Response {
    Auth::logout_and_invalidate().await?;
    App::clear_history();
    Redirect::to("/login").into()
}
```

### 1つのプロップへのフラグ合成

上のメソッドはそれぞれ1つのフラグを設定します。1つのプロップはいくつも保持でき、いくつかの組み合わせは、実際のページがInertiaプロトコルで動作するためのものです: クライアントがすでにレンダリングした内容へ追加するディファードリスト、ナビゲーションをまたいでクライアントがキャッシュするマージプロップ、独自のキャッシュキーを持つオプショナルプロップなどです。`Prop`でプロップを構築し、`.prop(key, prop)`で取り付けます:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::lazy(|| async { json!([{ "id": 1 }]) })
        .defer()
        .merge()
        .match_on("id"),
)
```

このプロップは初回レンダリングではスキップされ、`deferredProps`の下で通知されます。クライアントは追いかけのリクエストを発行し、そのリゾルバが走り、値は`mergeProps`命令とともに到着します。これにより、画面上にすでにあるリストを置き換えるのではなく、そこへ追加します。

フラグは5つのグループに分かれます:

| グループ | メソッド | 効果 |
|---|---|---|
| 可視性 | `.always()`、`.optional()`、`.defer()` | 相互排他的。最後の呼び出しが勝つ |
| Deferの詳細 | `.group(name)`、`.rescue()` | プロップがdeferredのときだけ読み取られる |
| Merge | `.merge()`、`.prepend()`、`.deep_merge()`、`.match_on(fields)`、`.merge_with_path(path)` | クライアントが値をどのように、どのパスへ折り込むか |
| クライアントキャッシュ | `.once()`、`.as_key(key)`、`.until(ms)`、`.fresh()` | クライアントがナビゲーションをまたいで値を保持するか |
| Scroll | `.scroll(metadata)`、`.scroll_wrap(key)` | 無限スクロールの`scrollProps`エントリと無条件のマージメタデータ。`.scroll_wrap`は`.scroll`が設定されているときだけ読み取られる |

ソースは`Prop::eager(value)`、`Prop::lazy(closure)`、自作リゾルバ用の`Prop::from_resolver(resolver)`、そしてレスポンスに決して届かないプロップ（未ロードのリレーションに対して`when_loaded!`が返すもの）用の`Prop::absent()`です。

合成する前に知っておくべき2つのルールがあります:

- **可視性は3つのフラグではなく、1つの設定です。** `.always().optional()`はoptionalプロップ、`.optional().always()`はalwaysプロップになります。どちらもエラーではなく、先の呼び出しが消去されます。
- **メタデータは値ではなく、部分リロードのリストに従います。** プロップの`mergeProps`、`onceProps`、`scrollProps`エントリは、キーが`X-Inertia-Partial-Data`と`X-Inertia-Partial-Except`を通過するたびに出力されます。値自体が抑制される訪問でも同じです。これが、ディファードプロップの2つのリクエストをまたいでマージ命令を運ぶ仕組みです。ここから2つの帰結があります:
  - 要求された集合の外にある`.always().merge()`プロップは、それでも値を送り、マージ命令は送りません。そのためクライアントは追加ではなく置換します。
  - `scrollProps`にはリスト以外に1つ条件があります。`.scroll().defer()`プロップは非部分訪問ではマージ命令を通知しますが、そこではカーソルを送りません。まだ画面上にカーソルが説明するものがないからです。一致する部分リロードでは、そのリクエストが値も解決するかどうかにかかわらず、毎回カーソルを受け取ります。
  - `deferredProps`は、リストが決して管理しない唯一のブロックです。一致する部分リロードでは、リストが何を言っていても全体が破棄されます - Laravelの`resolveDeferredProps`は、リクエストがpartialになった瞬間に`[]`を返します。部分リロードは、クライアントがすでに保持している通知を処理するものなので、このラウンドで省略したキーを再通知すると、またそれらを取りに戻されます。*別の*コンポーネントを対象にした部分リロードは、通知も含め、すべてのゲートにとって標準訪問です。

`.group(name)`と`.rescue()`はすべてのプロップに保存されますが、ディファードのときだけ読み取られるため、`.rescue().defer()`と`.defer().rescue()`は同じ意味です。Scrollプロップはクライアントの`X-Inertia-Infinite-Scroll-Merge-Intent`ヘッダーからマージ方向を取得するため、Scrollプロップ上の`.merge()`と`.prepend()`は冗長で読み取られません。`.deep_merge()`は例外で、`mergeProps`ではなく`deepMergeProps`へプロップを送ります。Laravelの`ScrollProp`も同じです。

### マージ戦略と無限スクロール

`.merge`（追加）、`.merge_prepend`、`.deep_merge`は、よくある「もっと読み込む」ケースをカバーします。クライアントがすでに保持している行を複製せずに更新する差分マージには、`match_on`キーを持つ明示的な`MergeStrategy`とともに`.merge_with`を使います:

```rust
use suprnova::{InertiaResponse, MergeStrategy};

InertiaResponse::new("Feed/Index")
    .merge_with(
        "posts",
        next_page,                                     // the new page slice
        MergeStrategy::Append { match_on: Some(vec!["id".into()]) },
    )
```

`match_on`は、クライアントが重複排除するフィールド名を指定します（ページオブジェクトには`matchPropsOn`として出力されます）。`Prop::match_on`（下記）と同じく、1つでも複数でもかまいません。そのため、現在のウィンドウと重なる再取得はコピーを追加せず、一致する行をその場で置換します。`Prepend`と`Deep`も同じ`match_on`を受け取ります。

`MergeStrategy`は1回の呼び出しで指定する形です。`Prop::merge()` / `.prepend()` / `.deep_merge()` / `.match_on(field)`は、別々のフラグとして同じ設定を表します。プロップに可視性やキャッシュフラグも必要な場合に使います - [1つのプロップへのフラグ合成](#1つのプロップへのフラグ合成)を参照してください。

`.match_on`は1回の呼び出しで1つまたは複数のフィールドを取ります - `.match_on(["id", "slug"])`と`.match_on("id").match_on("slug")`は同じ`matchPropsOn`を出力します。

プロップの値全体ではなく一部だけをマージするには、`.merge_with_path`で入れ子のフィールドを指定します:

```rust
use suprnova::{InertiaResponse, Prop};
use serde_json::json;

InertiaResponse::new("Feed/Index").prop(
    "posts",
    Prop::eager(json!({ "data": next_page, "meta": meta }))
        .merge()
        .merge_with_path("data")
        .match_on("data.id"),
)
```

`mergeProps`は`"posts"`ではなく`"posts.data"`を持つようになり、クライアントが既存の内容へ折り込むのは`props.posts.data`だけです。`props.posts.meta`は、マージされないプロップと同じく全面的に置換されます。呼び出しは累積するため、マージ可能なフィールドが2つあるプロップなら、それぞれを独立して指定できます。パスを指定すると、そのプロップではルートレベルのマージが完全に無効になります。パスマージのプロップが値全体も同時にマージすることはありません。`.match_on`はパスと合成され、フィールド名にパスを含めます（`"id"`ではなく`"data.id"`）。フレームワークが自動推論することはありません。`.deep_merge()`は`.merge_with_path`を無視します。deep mergeはすべての入れ子フィールドを再帰するため、パスで狭めるものがないからです。

マージプロップの値はリゾルバからも取得できます。`.merge_lazy` / `.merge_lazy_with`は`.merge` / `.merge_with`のリゾルバ版です:

```rust
InertiaResponse::new("Feed/Index").merge_lazy("posts", || async {
    Ok::<_, FrameworkError>(load_next_page().await?)
})
```

リゾルバが走るのは、マージプロップが実際に送られるときだけです。他のリゾルバ付きプロップと同様、部分リロードのフィルタリングや`.defer()`によってスキップされます。

無限スクロールは、ページネーションメタデータを付けた同じ仕組みです。`.scroll` / `.scroll_with`、または`LengthAwarePaginator`や`CursorPaginator`を直接適応させる`.paginate`は、データの隣に`scrollProps`を出力し、クライアントの`<InfiniteScroll>`コンポーネントが次/前の取得を駆動します:

```rust
// `posts` is a CursorPaginator from the query builder.
InertiaResponse::new("Feed/Index").paginate("posts", posts)
```

Scrollプロップは追いかけの取得だけでなく、常にマージメタデータを持ちます。デフォルトは追加で、クライアントの`X-Inertia-Infinite-Scroll-Merge-Intent`ヘッダーがそう指示したときだけprependへ切り替わります（下へスクロールするときは`append`、上へスクロールするときは`prepend`）。`reset`はそのヘッダーとは独立しており、通常のマージプロップが読むのと同じ`X-Inertia-Reset`でクライアントがキーを指定したときに限り`true`です。新鮮でフィルタされていない訪問ではどちらのヘッダーも送られないため、Laravelと同じく`reset: false`とappend命令になります。

`.merge_with_path`はScrollプロップに影響しません。Scrollブロックがマージ命令を計算するときに読むのは、`.merge_with_path`の累積パスリストではなく`Prop::scroll_wrap`の単一のwrapキーだからです。そのため、`.scroll(metadata).merge_with_path("data")`は誰も読まないパスを保存します。`.scroll_wrap`は`.prop(...)`から直接到達するか、下記の`.scroll_wrapped`レスポンスショートカットを通じて使う、Scrollプロップの入れ子版です。

Scrollプロップも、他のマージプロップと同じく`.match_on(...)`を尊重します。`.scroll`と`.match_on`を組み合わせたレスポンスレベルのショートカットはないため、`.prop(...)`を通じて使います:

```rust
InertiaResponse::new("Users/Index").prop(
    "users",
    Prop::eager(rows)
        .scroll(ScrollMetadata::new("page").current(1).next(2))
        .match_on("id"),
)
```

マッチフィールドは、プロップが実際にマージする場所に基づきます。unwrapされた場合は裸のキー（`matchPropsOn: ["users.id"]`）、`.scroll_wrap(...)`でwrapされた場合は`key.wrap_key`（`"data"`の下にwrapされたプロップなら`matchPropsOn: ["posts.data.id"]`）です。これにより、エントリはクライアントが折り込むマージパスと常に揃い、決して一致しない状態になることがありません。

プロップの値自体がwrapされた構造 - `{ data: [...], meta: {...} }`という、手作りのAPIリソースが通常返す形 - のとき、オブジェクト全体をマージすると毎回`meta`を上書きしてしまいます。`.scroll_wrapped`で配列フィールドを指定してマージ先にします:

```rust
InertiaResponse::new("Feed/Index").scroll_wrapped(
    "posts",
    "data",
    ScrollMetadata::new("page").current(2).next(3),
    serde_json::json!({ "data": rows, "meta": { "total": total } }),
)
```

`mergeProps`は`posts.data`を名指しするようになり、クライアントは新しい行を入れ子の配列へ折り込み、`meta`は毎回全面的に置換します。`.scroll_with_wrapped`と`try_scroll_wrapped`はリゾルバベースとfallibleの兄弟で、`.scroll_with` / `try_scroll`に対応します。

このcrateの`pagination`モジュール外の型 - サードパーティのpaginatorや手作りのcursor - は、`ScrollMetadata`をフィールドごとに構築する代わりに`ProvidesScrollMetadata`を実装して、`.scroll`へ自分自身を記述できます:

```rust
use suprnova::{ProvidesScrollMetadata, ScrollMetadata};

impl ProvidesScrollMetadata for MyCursorPage {
    fn page_name(&self) -> String { "cursor".to_string() }
    fn previous_page(&self) -> Option<serde_json::Value> { self.prev.clone().map(Into::into) }
    fn next_page(&self) -> Option<serde_json::Value> { self.next.clone().map(Into::into) }
    fn current_page(&self) -> Option<serde_json::Value> { Some(self.current.clone().into()) }
}

InertiaResponse::new("Feed/Index").scroll("posts", page.scroll_metadata(), page.rows)
```

`LengthAwarePaginator`、`Paginator`、`CursorPaginator`もこれを実装します。[Pagination](pagination.md#inertia-integration-infinite-scroll-props)を参照してください。

### ドット記法のネスト

`.`を含むキーは、リテラルな文字列キーとして送られるのではなく、レスポンス内にネストされます - Laravelの`Arr::set`ベースのドット記法（`Inertia::share('user.name', …)`、`resolveArrayableProperties`）です:

```rust
InertiaResponse::new("Dashboard")
    .with("user.name", "Todd")
    .with("user.locale", "es")
```

これは次のように送られます:

```json
{ "user": { "name": "Todd", "locale": "es" } }
```

`"user.name"` / `"user.locale"`という2つのリテラルキーにはなりません。同じプレフィックスを共有する2つの呼び出しは1つのオブジェクトへ累積し、ドットのないキーは影響を受けません。これはすべてのプロップ付加メソッド - `.with`、`.always`、`.lazy`、共有レジストリのキー - に適用され、それ以外には適用されません。プロップの*値*の中へ再帰することはないため、validationの`errors`オブジェクトが内部に持つドット付きフィールド名はそのままです。リテラルのドットを保持する必要があるキーのエスケープ手段はありません（`.with("config.json", …)`もネストされます） - `Arr::set`にエスケープ機構がないLaravelと同じ挙動です。

## 部分的なリロード

Inertia 3クライアントは、ページのプロップの部分集合（またはOptionalやDeferのキーを含めることで上位集合）を要求できます。プロトコルは3つのリクエストヘッダーを使います:

| ヘッダー | 意味 |
|---|---|
| `X-Inertia-Partial-Component` | 部分リロードされるコンポーネント。フィルタリングを適用するにはレスポンスのコンポーネントと一致しなければならない |
| `X-Inertia-Partial-Data` | 許可リスト: 含めるプロップキーをカンマ区切りで指定 |
| `X-Inertia-Partial-Except` | 拒否リスト: 除外するプロップキーをカンマ区切りで指定。キーが衝突した場合は`Partial-Data`に勝つ |

フィルタリングが読むのは1つだけです: `.always()`、`.optional()`、`.defer()`で設定されたプロップの可視性です。どれもないプロップにはデフォルトの可視性があります。

- デフォルト可視性のプロップは、許可リスト / 拒否リストのセマンティクスに従います。
- `.always()`のプロップは常に送られます。
- `.optional()`と`.defer()`のプロップは標準訪問では決して送られず、キーを明示的に列挙する一致した部分リロードにだけ現れます。

マージとScrollのフラグは関与しません。受信した値をクライアントがどう折り込むかを決めるもので、値を受信するかどうかを決めるものではないからです。そのため`.defer().merge()`プロップは、通常の`.defer()`とまったく同じようにフィルタされます。`.once()`も同様に関与しませんが、これは純粋な折り込み命令ではありません。クライアントがすでに値をキャッシュしていると報告した完全訪問では、下記の注記のとおり、サーバーはリゾルバをスキップして値を送りません。3つすべてが変えるのは、どのメタデータブロックを伴わせるかです - [1つのプロップへのフラグ合成](#1つのプロップへのフラグ合成)を参照してください。

ハンドラは何も特別なことをする必要がありません。すべてのプロップをビルダー経由で登録すれば、フレームワークがページオブジェクトをシリアライズするときにヘッダーを参照します。

`once`プロップのクライアント側キャッシュが尊重されるのは、**完全な**Inertia訪問だけです。キーを名指しする部分リロード（`router.reload({ only: ['stats'] })`）では、リゾルバが走り値が送られます。クライアントはまさに新しい値が欲しいから要求したのであり、そこで古いキャッシュの主張を尊重すると、要求したキーには何も返らなくなるからです。

### 入れ子になったonly/except（ドット記法）

`X-Inertia-Partial-Data`と`X-Inertia-Partial-Except`のエントリは、プロップ自身のキーだけでなく、プロップの値の内部のパスも指定できます。`router.reload({ only: ['user.name'] })`を呼ぶクライアントは`X-Inertia-Partial-Data: user.name`を送信し、レスポンスは`user`プロップをそのフィールドだけへ狭めます:

```json
{ "props": { "user": { "name": "Ada" } } }
```

`except`は狭める代わりに同じように削除します - `router.reload({
except: ['user.email'] })`は`user`の他のフィールドをすべて残します。

ルール:

- 裸のエントリ（`user`）は、今もプロップ全体を意味します。`only`が`user`と`user.name`の両方を指定する場合は、全体の値が送られます - 裸のエントリが勝ちます。
- エントリはドット付きプロップキーの*祖先*も指定できます。`"auth.user"`に登録されたプロップ（`.with("auth.user", …)`または`App::inertia_share("auth.user", …)`）は`only: ['auth']`に参加し、呼び出し側が`auth`ルート全体を要求したため全体が送られます。裸の`except: ['auth']`は同じ理由でそれを落とします。プレフィックスはセグメント境界で終わらなければならないため、無関係な`authAgent.user`プロップはどちらにも触れられません。
- 両方のヘッダーが同じパスを指定した場合は、トップレベルと同じく`except`が勝ちます。
- 値に対して解決できないパス（未知のフィールド、またはオブジェクトではなくスカラーや配列を通るパス）は、そのパスについて何も寄与しません。その隣で要求された兄弟フィールドは落とされません。
- `Always`プロップはドット記法も含め、`only`/`except`を完全に無視し、常に全体を送ります。
- `Optional`と`Defer`プロップは、そもそも解決するための明示的な要求を必要とします。ドット付きエントリ（`permissions.read`）はトップレベルキーへの要求として数えられ、解決された値は`Eager`プロップと同じように狭められます。
- オブジェクトでない現在値を持つプロップ（文字列、数値、配列）に対するドット付き`only`は、元の値ではなく`{}`へ狭めます。クライアントの調整は、キャッシュ値と受信値の**両方**がオブジェクトの場合だけdeep-mergeします（`inertia-3.6.1/packages/core/src/response.ts`の`nestedTopKeys`）。非オブジェクトのキャッシュに対しては、空オブジェクトも値ありのオブジェクトと同じくその検査に失敗するため、空オブジェクトはキャッシュされたスカラーへマージされず、完全に置換します。オブジェクト形状でないプロップには、ドット付き要求を送らないでください。
- ドット付き`except`はクライアント上のフィールドを削除しません。このレスポンスでのフィールド更新を止め、クライアントのマージがすでにキャッシュしている値から復元できるようにします。`deepMergeObjects`は最初にキャッシュ値をクローンし、サーバーが実際に送ったキーだけを上書きしてマージ済みオブジェクトを作ります。サーバーが省略したキーには触れないため、古い値のまま残ります。そのプロップをクライアントが初めて読み込む場合（まだ何もキャッシュされていない場合）、省略されたフィールドは本当に存在しません。フォールバックできるキャッシュがないためです。「キャッシュから復元する」挙動は、クライアントがすでに見たページにだけ適用されます。

## `App::inertia_share*`による共有データ

認証状態、CSRFトークン、現在のロケール、アプリ全体のフラグなど、すべてのInertiaページで同じプロップがあります。ブートストラップで一度登録すれば、すべてのレスポンスへマージされます:

```rust
use suprnova::App;
use std::sync::Arc;

pub fn register() {
    // Sync, materialized once at boot.
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // Async, resolved per response (skipped by partial reloads that
    // exclude the key).
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // Cached on the client across navigations - `share_once` runs on
    // the first page that needs it, then the client skips re-resolution
    // via `X-Inertia-Except-Once-Props` until the cache key changes.
    App::inertia_share_once("plans", || async {
        Ok::<_, suprnova::FrameworkError>(load_plan_catalog().await?)
    });
}
```

共有キーは`.with`と同じようにドットでネストされます - `"user.name"` / `"user.age"`の下にある2つの静的共有は、wire上の1つの`user`オブジェクトになります。共有値の読み取り、または静的レジストリ全体の消去には、Laravelの`Inertia::getShared` / `Inertia::flushShared`に対応する`App::inertia_shared` / `App::flush_inertia_shared`を使います:

```rust
use suprnova::App;

App::inertia_share("user.name", "Todd");
assert_eq!(App::inertia_shared("user.name"), Some(serde_json::json!("Todd")));

App::flush_inertia_shared();
assert_eq!(App::inertia_shared("user.name"), None);
```

`inertia_shared`が読むのは静的レジストリだけです。`inertia_share_lazy` / `inertia_share_once`で登録されたキー（解決するリクエストがないため。呼び出さずに生のクロージャを返すLaravelの`getShared`と同じ）や、リクエスト単位のトレイトプロバイダー共有については`None`を返します。`flush_inertia_shared`も静的レジストリだけを消去します。`register_inertia_shared`で登録されたプロバイダーには、消去すべきリクエスト単位の状態がありません。

リクエスト単位の共有データ（認証済みユーザー、リクエストスコープのフラグ）については、[`InertiaSharedData`](#リクエスト単位の共有データ)を実装してシングルトンを登録してください。フレームワークはすべてのInertiaレスポンスで`share(&req, component)`を呼び、その結果をマージします。`component`はレンダリング中のページなので、プロバイダーはページごとに出力を変えられます - 下記を参照してください。

### キー衝突時の優先順位

同じキーが複数の層に現れる場合、後から書き込まれたものが勝ちます:

1. 静的レジストリ（`App::inertia_share` / `App::inertia_share_lazy`）
2. リクエスト単位のトレイトプロバイダー（`InertiaSharedData::share`）
3. レスポンス単位のビルダーメソッド（`.with`、`.lazy`など）

これにより、ハンドラは何も登録解除せず、1ページだけグローバル共有のデフォルトを上書きできます。

### リクエスト単位の共有データ

このトレイトはInertiaレスポンスごとに1回、リクエスト**と**ページコンポーネント名へアクセスして実行されます。これはLaravelの`RenderContext`（`component`、`request`）であり、リクエストがもう一方をカバーするため、ラッパー構造体ではなく通常の引数として渡されます。実装には`async_trait`（`suprnova::__async_trait`として再エクスポート）と`IndexMap`（`suprnova::indexmap`として再エクスポート）が必要です:

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
        component: &str,
    ) -> Result<IndexMap<String, Prop>, FrameworkError> {
        let mut out = IndexMap::new();
        if let Some(user) = Auth::user().await? {
            out.insert(
                "auth".into(),
                Prop::eager(serde_json::json!({
                    "id": user.get_auth_identifier(),
                })),
            );
        }
        // Vary by page: only the admin dashboard needs the nav counts.
        if component == "Admin/Dashboard" {
            out.insert("pendingReviews".into(), Prop::eager(serde_json::json!(12)));
        }
        Ok(out)
    }
}

// In bootstrap:
App::register_inertia_shared(Arc::new(AuthShare));
```

ページごとに変える必要がないプロバイダーでは`component`（`_component`）を無視してください。

## フラッシュとリダイレクト

フラッシュデータは、次のレンダリングに現れてその後消えるべきワンショットの状態です - トーストメッセージ、「たった今作成された」ID、バリデーションのまとめなどです。SuprnovaはすべてのInertiaレスポンスで`page.flash`の下にそれを表面化します。書き手は3つあります:

```rust
// 1. Push into the current request's flash bag.
App::flash("toast", "Saved");

// 2. Attach to a specific response (same effect on this response only).
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Carry across a redirect via the Redirect facade.
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

`Redirect::with(key, value)`の形はハンドラをまたぐ経路です。値はセッションの`_flash.new.*`の下に着地し、次のリクエストの[`SessionMiddleware`](csrf.md)がそれを`_flash.old.*`へ歳を取らせ、行き先の`InertiaResponse`が`page.flash`の下に表面化させます。

同一リクエストのフラッシュ（タスクローカルバッグ）は、キーが衝突したとき継承されたセッションフラッシュに勝ちます。そのため行き先のハンドラは、キーを再フラッシュするだけで受信した値を上書きできます。

内部セッションキー（`_`が前置されたもの）は`page.flash`からフィルタされます。フォーム再投入用の`_old_input`と`_inertia.*`プロトコルフラグがクライアントへ漏れることはありません。

### リダイレクトのヘルパー

`Redirect`はLaravelの完全な表面です:

```rust
Redirect::to("/dashboard")                       // 302 to a path
Redirect::route("posts.show").with("id", "42")   // named route, route params
Redirect::back("/")                              // session-recorded previous URL
Redirect::refresh()                              // same URL, fresh GET
Redirect::guest(&req, "/login")                  // stashes intended URL
Redirect::intended("/dashboard")                 // pops the stashed URL
Redirect::signed_route("downloads.show", &[("id","42")])?  // signed URL
Redirect::to("/posts/42").preserve_fragment()    // keep #frag across visit
```

すべての`Redirect`変種は`.with(k, v)`、`.with_input(map)`、`.with_errors(map)`、`.with_errors_bag(name, map)`、`.cookie(c)`、`.header(k, v)`、`.permanent()`、`.status(303)`などを受け付けます。完全なチェーンはLaravelの`RedirectResponse`を反映します。

GET以外のInertia訪問では、[`Inertia303Middleware`](#ブートストラップ-inertia-install)がインストールされていると、フレームワークがレスポンスを`303 See Other`へ自動変換します。ブラウザは元のPUT/PATCH/DELETEをリダイレクト先へ再送信せず、きれいな追いかけのGETを発行します。

### バリデーション失敗

Inertia訪問でハンドラがバリデーションに失敗すると、フレームワークはRESTクライアントが受け取る`422` JSONの代わりに、エラーをフラッシュしてフォームページへ戻る`303 See Other`で応答します。これは見た目だけの違いではありません。`X-Inertia`ヘッダーのないレスポンスはInertiaクライアントが非Inertiaとして扱い、全画面エラーモーダルに表示するため、`422`は`form.errors`へ到達しません。ハンドラ側で変更するものはなく、このブリッジは`Inertia::install`が登録するミドルウェアの1つです。

行き先は、同一オリジンならリクエストの`Referer`、次にセッションが記録した直前のURL、最後に失敗したリクエスト自身のURLです。クロスオリジンの`Referer`は追従せず無視されます。同一オリジンに見えるだけのものも同様です。先頭が`//`または`/\\`の値（ブラウザはバックスラッシュをスラッシュへ折りたたんだ後、どちらもプロトコル相対として読み取ります）や、値のどこかにASCII制御バイトがある値は、同じようにフォールバックします。URLパーサーはオリジンを比較する前に文字列全体からタブと改行を取り除くため、制御バイトによってブラウザの移動時には安全そうなパスが別オリジンに変わることがあるからです。最後のURLフォールバックにも同じ検査が適用されるため、異常なリクエストパスでもオリジン外へのリダイレクトにはなりません。

フィールドの値は**最初の**メッセージで、プレーンな文字列です。これはInertia自身の`ErrorValue`型が記述する形であり、`$page.props.errors.email`がバインドする形です。すべてのメッセージを代わりに配列で取得するには`InertiaConfig::with_all_errors(true)`を設定します。その場合、クライアント側の型にも対応する拡張が必要です:

```ts
// global.d.ts
import '@inertiajs/core'

declare module '@inertiajs/core' {
  export interface InertiaConfig {
    errorValueType: string[]
  }
}
```

1ページ上の複数フォームは分離されたままです。訪問とともに`X-Inertia-Error-Bag: <name>`を送ると、エラーはそのバッグの下へフラッシュされ、そこから読み戻され、`errors.<name>.<field>`として到着します。

`errors`プロップはデフォルトで常に可視なので、部分リロードがそれをフィルタしたり狭めたりすることはありません。`only: ['users']`でもバッグは送られ、`except: ['errors']`でも送られます。`only: ['errors.email']`はそのフィールドだけでなくバッグ全体を送ります。これはLaravelの形です。Laravelのミドルウェアはバッグを`Inertia::always(...)`として共有し、`resolveAlways`は`only`/`except`の再構築後に生の値を再注入します。クライアントが部分レスポンスを`{...current.props, ...response.props}`で折り込むため、空の`errors`オブジェクトは画面にあるメッセージを消してしまう一方、フィルタされないバッグなら正しく残せます。このルールはセッションにフラッシュされたバッグと、ハンドラ自身の`.with("errors", …)`という両方のソースに適用されます。明示的な可視性フラグはそれでも優先されるため、`.prop("errors", Prop::eager(…).optional())`はoptionalとして動作します。

この仕組みがしないことは2つあります。古い入力を再フラッシュすることはありません。ブリッジが走る時点でリクエストボディはすでに消費されており、Inertiaの`useForm`は失敗した送信後も自身の状態を保持するため、再投入するものがないからです。またPrecognitionのレスポンスには決して触れません。dry-runの`422`は、クライアントが要求したとおりのものです。

訪問者をInertiaアプリの**外**へ送るには - 決済プロバイダー、OAuth authorizeエンドポイント、ホストされた請求ポータルなど - `location_for`を使います:

```rust
use suprnova::{InertiaResponse, Request, Response};

pub async fn checkout(req: Request) -> Response {
    Ok(InertiaResponse::location_for(&req, "https://billing.example/checkout"))
}
```

Inertia XHRは`409` + `X-Inertia-Location`を受け取り（クライアントは`window.location = url`を実行します）、ハードナビゲーションはプレーンな`302` + `Location`を受け取ります。裸の`InertiaResponse::location(url)`は常に409形式を返します。リクエストがすでにInertia訪問だと分かっている場所でだけ使ってください。`Location`ヘッダーのない`409`に従うブラウザには行き先がないからです。

## バージョン検出

Inertiaはアセットマニフェストにバージョンを付けるため、長生きするクライアントが昨日のバンドルのページを今日のサーバーに対してマウントしようとすることはありません。クライアントの`X-Inertia-Version`ヘッダーがサーバーの設定済みバージョンと一致しないとき、[`InertiaVersionMiddleware`](#ブートストラップ-inertia-install)は`409 Conflict`と新しいURLを名指しする`X-Inertia-Location`ヘッダーで応答します。Inertiaクライアントはそれを受け取り、ページ全体をリロードして新しいバンドルを取得します。

この跳ね返しの前に、セッションが再フラッシュされます。クライアントは409に対してページ全体のGETで応答し、そのGETは新しいリクエストです。再フラッシュがなければ、前のリクエストがフラッシュしたバリデーションエラーや成功メッセージは、行き先ページが読み取る前に歳を取って消えます。デプロイが送信中に着地しただけで、ユーザーはエラーメッセージを失うことになります。これには`SessionMiddleware`をバージョンミドルウェアより前に登録する必要があります。

デフォルトでは何も設定する必要がありません。`InertiaConfig`がViteビルドマニフェスト（`manifest_path`、デフォルトは`public/assets/.vite/manifest.json`）をハッシュし、そのSHA-256の先頭16バイトを16進エンコードして使います。マニフェストはすべてのビルドで変わり、それ以外では変わらない唯一のファイルなので、バージョンは自動的に上がります。読み取るマニフェストがない場合 - Viteがメモリから提供するローカル開発など - は静的文字列`"1.0"`にフォールバックし、`debug`でログを出します。

別の値にしたい場合は上書きします:

```rust
use suprnova::{InertiaConfig, VersionResolver};

// Default - hash the build manifest. Nothing to write.
let cfg = InertiaConfig::new();

// A different manifest location; the version follows it.
let cfg = InertiaConfig::new().manifest_path("dist/.vite/manifest.json");

// Static - bake in a build-time identifier. Survives a later
// `.manifest_path(...)` call: an explicit version is deliberate.
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// Dynamic - a container deployment id, anything. The closure runs on
// every version check; cache inside if it isn't cheap.
let cfg = InertiaConfig::new().version_with(|| deployment_id());
```

マニフェストは各バージョン検査で読み取られます。これはLaravelの`hash_file`も同じで、ページキャッシュから数KBを読むだけで、リビルドをすぐ拾います。測定した結果それをなくしたい場合は、起動時に一度だけ解決します:

```rust
use suprnova::{InertiaConfig, VersionResolver};

let version = VersionResolver::from_manifest("public/assets/.vite/manifest.json").resolve();
let cfg = InertiaConfig::new().version(version);
```

非同期またはfallibleなバージョン解決（S3からマニフェストハッシュを読む場合など）では、起動時に一度読み、キャッシュした`String`を`.version(...)`へ渡してください。

## ブートストラップ: `Inertia::install`

ほとんどのアプリは、`register_http_stack`から4つのプロトコルミドルウェアを1回の呼び出しでインストールします。これはHTTP専用のブートストラップフックで、サーバーパスは実行しますが、queue、schedule、workflow、consoleバイナリはスキップします（[Bootstrap](bootstrap.md)を参照）:

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)
        .expect("Inertia install failed (production needs a built frontend manifest)");
    // …global middleware, in the order you want it to run
}
```

```rust
// cmd/main.rs
Application::new()
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
```

`bootstrap::register`の中には置かないでください。`public/assets`を出荷しないworkerまたはconsoleイメージの状態がまさにそうであるように、`Inertia::install`はビルド済みフロントエンドマニフェストが本番で欠けているとfail closedします。プロセス全体のフックからインストールすると、そのバイナリも一緒に停止してしまいます。

`Inertia::install`は`Result`を返し、次の順序で処理します:

1. `cfg`が本番モード（`development == false` - `APP_ENV=production`のときは常にこれがデフォルト）に解決され、`cfg.manifest_path`からViteマニフェストをロードできない場合、fail closedします。これがCFG-01ガードです。未ビルドのフロントエンドで本番を起動すると、レガシーなハードコード済みアセットパスへ静かにフォールバックせず、はっきりエラーになります。
2. `InertiaHeadersMiddleware`を登録します - すべてのレスポンスに`Vary: X-Inertia`を設定し、Inertia訪問で空の`200`を`303`の戻りへ変えます。
3. `InertiaVersionMiddleware`を登録します - クライアントとサーバーがアセットバージョンで一致しない場合、`409` + `X-Inertia-Location`を出力します。
4. `Inertia303Middleware`を登録します - GET以外のInertiaリダイレクトで`302`を`303`へ格上げします。
5. `InertiaValidationRedirectMiddleware`を登録します - Inertia訪問の`422`を、エラーをフラッシュしたフォームページへの`303`へ変換します。[バリデーション失敗](#バリデーション失敗)を参照してください。

順序が重要です。ヘッダーミドルウェアが最初に登録されるため最も外側になり、ハンドラが実行される前にバージョンミドルウェアが返す`409`も含め、すべてのレスポンスを見ます。バリデーションリダイレクトミドルウェアは最後に登録されるため最も内側、つまりハンドラに最も近くなり、他の3つが触れる前の`422`を見ます。

`install`は**設定も保持します**。以後に構築されるすべての`InertiaResponse`はそこから出発するため、ここで設定した`.frontend(...)`、`.version(...)`、`.default_title(...)`、`.ssr(...)`、`.encrypt_history(...)`は、ハンドラが何も渡さなくてもすべてのページへ届きます。1ページだけ異なる設定を望むハンドラは`.with_config(...)`で上書きします。`Inertia::install`を呼ばないアプリは`InertiaConfig::default()`を得、`install`を再度呼ぶと保持された設定が置き換わります。

`.with_config(...)`は`version`も含めて設定を丸ごと置き換えます。`InertiaVersionMiddleware`は、それでも`Inertia::install`へ渡されたバージョンを解決するため、ここでの設定が同じ`.version(...)`を持たなければ、ページオブジェクトはミドルウェアが跳ね返すバージョンを広告してしまいます。そのページを訪れた後、クライアントはページ全体のロードをもう1回行うことになります。一致させるには、上書き側にも`.version(...)`を設定してください。

フラッシュデータを使う場合は、`SessionMiddleware`を`Inertia::install`**より前に**登録してください。バージョンミドルウェアはクライアントを跳ね返す前にセッションを再フラッシュするため、フラッシュされたエラーは追いかけのページ全体のGETを生き延びます。これはセッションスコープ内でのみ可能です。

これらのミドルウェアのどれかを本当に望まない場合にだけ呼び出しを省略してください（まれです。4つすべてが実際の失敗モードを塞ぎます - 1つのURLの2つの表現をまたぐキャッシュポイズニング、静かな古いバンドル、リダイレクト時のフォーム再送信、そしてクライアントのエラーモーダルで行き止まりになり`form.errors`へ届かないバリデーション`422`）。

## サーバー主導の`<head>`要素

Inertia 3.5は、`<head>`に何を入れるかをサーバーに決めさせるクライアントオプションを追加しました。これは、メタタグがたった今ロードしたレコードに依存し、titleとOGタグを2か所に置きたくない場合に便利です。

フレームワーク側のサポートは必要ありません。クライアントが要素を読むのは**通常のプロップ**からなので、どのハンドラでも供給できます:

```rust
#[handler]
async fn show(RouteParam(post): RouteParam<Post>, req: Request) -> Response {
    inertia_response!(&req, "Posts/Show", {
        "post": post,
        "head": [
            format!("<title>{}</title>", post.title),
            format!(r#"<meta property="og:title" content="{}">"#, post.title),
        ],
    })
}
```

クライアントでオプトインします:

```js
createInertiaApp({
  serverHead: true,        // reads the `head` prop
  // serverHead: 'meta',   // or read a differently-named prop
  // serverHead: (page) => [...],  // or compute from the whole page
})
```

各文字列はHTML要素です。クライアントは、`data-inertia`属性を持たないものへそれを刻み込み、ナビゲーションをまたいでhead要素をdiffできるようにします。位置によるマッチングではなく安定した識別子が必要なら、自分で`data-inertia="og-title"`を指定してください。

ユーザーデータから補間するものはすべてエスケープしてください。これらの文字列はHTMLとして注入されるため、通常のルールが適用されます。

## SSR

Suprnovaはプロセス外のSSR worker - 通常はNode / Bun / Denoの下で動く`@inertiajs/{svelte,react,vue}/server`の`createServer()`バンドル - とHTTPループバック経由で通信します。[`Inertia::install`](#ブートストラップ-inertia-install)に渡す設定で有効にしてください。その設定がすべてのレスポンスの出発点なので、ハンドラを通して配管するものはありません:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // worker URL
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

SSRはデフォルトでオフで、設定のプロパティです。インストールされた設定から構築されるすべてのレスポンスではオンになり、SSRを設定しない`.with_config(...)`で上書きするレスポンスではオフになります。有効な場合、フレームワークはページオブジェクトを`<url>/render`へPOSTし、`{ head, body }`をHTMLシェルにインライン化します。workerのエラーやタイムアウト時はレスポンスがCSR（クライアントがhydrateする空の`<div id="app">`）へフォールバックし、`on_ssr_error(...)`フックが発火します。代わりにCIで`ssr_throw_on_error(true)`を設定すると、失敗をハードな500にできます。

ディスパッチ前に、ゲートウェイがビルド済みSSRバンドルがディスクに存在するか確認することもできます。`.ssr_bundle_path(...)`をオプトインし、通常の`frontend/bootstrap/ssr/ssr.js`を指定してください（確認自体はデフォルトで有効な`.ssr_ensure_bundle_exists(true)`ですが、パスを設定するまで効果はありません。これは意図的に自動検出しないため、テストダブルでSSRを有効にしてもディスク上のバンドルをスタブする必要がありません）。バンドルが欠けていると即座にCSRへフォールバックし、決して成功しない接続で`ssr_timeout`を待つことがありません。これはLaravelの`ensure_bundle_exists`設定に対応します。

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")
        .ssr_bundle_path("frontend/bootstrap/ssr/ssr.js")
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

`suprnova new`はすべてのstarterで`frontend/src/ssr.{ts,tsx}`と`build:ssr` npmスクリプトをscaffoldします。ビルドしてからworkerを起動します:

```bash
cd frontend && npm run build:ssr
suprnova ssr:start
```

`suprnova ssr:check`はworkerが実際に応答していることを検証します。worker自身の`GET /health`ルートへアクセスしますが、これはすべての`createServer()`バンドルが追加コードなしで公開するものです。

## 設定

Inertiaの動作は`InertiaConfig`でプログラム的に設定され、[`Inertia::install`](#ブートストラップ-inertia-install)に渡した設定がすべてのレスポンスの出発点になります。フレームワークが直接読む環境変数は`SUPRNOVA_FRONTEND`（`svelte` / `react` / `vue`）だけです。設定に指定がない場合に限り、デフォルトのエントリポイントファイル名とページコンポーネント拡張子を供給します。インストール済み設定で明示した`.frontend(Frontend::React)`が勝ち、`suprnova new --frontend react`がscaffoldする内容になります。それ以外はすべてビルダー形状です:

```rust
use suprnova::{InertiaConfig, Frontend};

let cfg = InertiaConfig::new()
    .frontend(Frontend::Svelte)               // overrides SUPRNOVA_FRONTEND
    .vite_dev_server("http://localhost:5765")
    .entry_point("src/main.ts")
    .version(env!("CARGO_PKG_VERSION"))
    .default_title("My App")
    .manifest_path("public/assets/.vite/manifest.json")
    .assets_base_url("/assets")
    .max_concurrent_resolvers(16)             // cap lazy-prop fan-out
    .with_all_errors(false)                   // one message per field, or all
    .url_resolver(|req| req.path_and_query()) // how `page.url` is derived
    .production();                            // false → loads from Vite dev server
```

フロントエンド固有のデフォルト:

| フロントエンド | デフォルトエントリポイント | ページ拡張子 |
|---|---|---|
| Svelte（デフォルト） | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`、`.jsx` |
| Vue | `src/main.ts` | `.vue` |

### `url`フィールド

`page.url`はリクエストのパス**と**クエリ文字列です（`/users?page=2&sort=name`）。クライアントはこれを`history.state`へ書き込むため、戻る/進むナビゲーションと`router.reload()`が再生するのはこれです。クエリを落とすと、ページネーションされたページやフィルタされたページはすべて静かに1ページ目へリセットされます。`InertiaVersionMiddleware`もリクエストのパスとクエリから`X-Inertia-Location`を導出するため、デフォルトでは409のアセットバージョン跳ね返しが、ページオブジェクトが名指ししたURLへブラウザを正確に着地させます。

クライアントが記録すべきURLと到着したURLが異なる場合 - SPAがルーティングしないロケールプレフィックスや、リバースプロキシが書き換えたパスなど - `url_resolver`で導出を上書きします:

```rust
use suprnova::InertiaConfig;

let cfg = InertiaConfig::new()
    .url_resolver(|req| req.path_and_query().replacen("/en", "", 1));
```

リゾルバは`InertiaRequestExt`を通じてリクエストを読み取り、[`Inertia::install`](#ブートストラップ-inertia-install)へ渡す設定から構築されたすべてのレスポンスに適用されます。これはアプリ全体に適用するリゾルバの通常の場所です。1つのレスポンスでは`InertiaResponse::with_config(cfg)`で上書きします。リゾルバが変えるのは`page.url`だけです。409の跳ね返しは実際に到着したURLを名指しし続けます。それがブラウザが取得しなければならないURLだからです。そのためリゾルバがある場合、2つは意図的に異なります。

`manifest_path`のViteマニフェストは最初のリクエストで遅延ロードされ、プロセスの寿命の間キャッシュされます。インストールされた設定から構築されたすべてのレスポンスがそのキャッシュを共有するため、ファイルは一度だけ読み取られ、パースされます。欠けている場合、本番のアセットタグはハードコードされたレガシーパスへフォールバックし、`tracing::warn!`が発火して欠落がログに現れます。

### Suprnovaが異なる設計を選んだ理由

LaravelのInertiaアダプターには、単一のグローバル「共有データ」レジストリと、リクエスト単位の`Inertia::share($k, $v)`呼び出しがあります。PHPのリクエストごとのプロセスモデルでは、リクエストごとに新しいプロセスになるため、並行する訪問者間で漏洩せず安全です。

Rustのプロセスモデルは正反対です。1つのプロセスが多数のスレッドをまたいで多数の並行リクエストを処理します。そのためレジストリはプロセスグローバルなstaticではなく、[container](container.md)（task-local → thread-local → global）に存在します。`App::inertia_share*`はアクティブなコンテナの`InertiaRegistry`へ書き込みます。これにより`TestContainer::fake()`を使うテストは何も登録解除せずにきれいな分離を得られます。表面はLaravelと同じですが、ランタイムが異なるため下の機構が違います。

注記に値する、Rustらしい他の9つの選択:

- **レイジープロップのリゾルバは並行して走ります。** 上限は`max_concurrent_resolvers`（デフォルト16）です。レイジープロップを12個持つページは、1つのTokioタスク内で12個の並列クエリを発行します。これこそTokioの上にフレームワークを構築した理由です。多数のレイジープロップがそれぞれ外部サービスを叩くページでは上限を調整してください。
- **コンパイル時のコンポーネント検査はLaravelの機能ではありません。** PHPはコンパイル時にフロントエンドファイルを見られないからです。Suprnovaは見られるため、`inertia_response!("Dashbaord", …)`のタイプミスは実行時の「component not found」ではなく、「did you mean Dashboard?」という提案とともにビルドを失敗させます。
- **Inertia訪問で空の`200`は`302`ではなく`303`になります。** Laravelの`onEmptyResponse`は`redirect()->back()`（302）を返し、PUT/PATCH/DELETEでのみ後段の`302 → 303`変換に頼ります。置き換えられたリダイレクトは元のメソッドの続きではなく、クライアントはGETを発行しなければなりません。そのためSuprnovaは直接`303`を返し、GET訪問をクライアントが元の動詞で追う302のままにしません。
- **`Inertia::location($url)`はここでは1つではなく2つのメソッドです。** `location(url)`はLaravelの常に`409`という契約を保ちます。これはリクエストを意識する形式より前からあり、タグを固定した利用者は形が変わらないことに依存しています。`location_for(&req, url)`は新しいリクエスト対応形式で、Inertia XHRには`409`、ハードナビゲーションにはプレーンな`302`です。新しいコードでは`location_for`を使ってください。
- **`Inertia::clearHistory()`も、ここでは1つではなく2つのメソッドです。** ビルダー上の`.clear_history()`は単一レスポンスに印を付け、`App::clear_history()`はリダイレクトを生き延びるようセッションへフラグをフラッシュします。Laravelが1メソッドで済むのは、すでにセッションに支えられているからです。Suprnovaはレスポンスローカル形式をデフォルト（セッション依存なし）にし、リダイレクトをまたぐケースを明示的なオプトインにしています。
- **`.lazy()`はLaravelの`Inertia::lazy()`ではありません。** Laravelのメソッドは非推奨で`optional()`のように振る舞います。`LazyProp`は`OptionalProp`の単なるエイリアスで、初回訪問では完全にスキップされます（`ResponseFactory.php:174-181`）。Suprnovaの`.lazy()`は、Laravel自身がラッパーなしのcallableプロップに使う通常のクロージャ規約で、部分リロードのフィルタリングがキーを通せば標準訪問を含めて挿入されます。Laravelから来て「lazy」という名前が示す初回訪問スキップ動作が欲しい場合は`.optional()`を使ってください。
- **ネストした`only`/`except`は、解決前ではなく解決後に狭められます。** Laravelの`Response::resolvePartialProperties`は、まだ解決されていない生のプロップ配列をドット付きパスでたどるため、`LazyProp`や`DeferProp`内のパスは`null`へ劣化します。未解決のクロージャに当たって歩行が止まるからです（`inertia-laravel-2.0.25/src/Response.php:273-297`）。Suprnovaはすべてのプロップ値を先に解決し、その後に結果のJSON値を狭めます。リゾルバは非同期なので、Laravelのようにすべてがプレーンな配列になる同期点がないためです。未知または型の合わない入れ子パスは`null`として返さず破棄します。クライアント自身の調整も、狭めたオブジェクトを既存値へdeep-mergeすることを想定しているからです（`inertia-3.6.1/packages/core/src/response.ts:414-425`）。余計な`null`は既存フィールドを残す代わりに上書きしてしまいます。
- **`.scroll_wrapped`はオプトインで、自動ではありません。** Laravelの`Inertia::scroll($value, $wrapper = 'data', …)`は、通常Laravelのpaginator resourceが`{ data: [...], links: {...}, meta: {...} }`を返して配列だけをマージするため、すべてのScrollプロップのマージ命令をデフォルトで`"data"`の下へネストします。Suprnova組み込みpaginatorは裸の行配列（`Vec<T>`でenvelopeなし）を返すので、`.scroll` / `.paginate`はプロップのルートでマージし、入れ子パスが必要な場合に`.scroll_wrapped`を使います。
- **wrapされたScrollプロップは`match_on`フィールドに自動でプレフィックスを付けます。** `.scroll_wrapped("posts", "data")`プロップでは、`match_on("id")`が`"posts.data.id"`を出力します。Laravelはプレフィックスなしの`"posts.id"`を出力し、自身のクライアントもマージ対象に揃えられないため、matchが静かに発火しません。ここではネスト地点が明確です。Scrollプロップには最大1つのwrapperしかないので、Suprnovaがプレフィックスを導出します。パスではなく裸のフィールド名を書いてください。

## 次のステップ

- [Page Components](frontend-pages.md) - フロントエンドがコンポーネント名をSvelte / React / Vueモジュールへ解決する仕組み
- [TypeScript Types](frontend-typescript-types.md) - `suprnova generate-types`が`#[derive(InertiaProps)]`構造体からTS定義を出力する
- [Data Objects](data.md) - 部分的なリロードと合成される、フィールドごとのinclude / allowlistゲーティングを備えたDTO用の`#[derive(Data)]`
- [Error Model](error-model.md) - `Response`、パニック境界、`FrameworkError`がInertiaレスポンスをどのように通り抜けるか
- [Container](container.md) - `App::inertia_share*`と`InertiaSharedData`の背後にあるルックアップモデル
