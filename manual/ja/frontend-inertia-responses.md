# Inertia レスポンス

Inertiaレスポンスは、Suprnovaのハンドラが状態をSvelte / React / Vueのページコンポーネントへ届ける方法です。Inertiaページをレンダリングするすべてのハンドラは、[`inertia_response!`](#the-inertia_response-macro)マクロ（型付きで、コンパイル時に検査されるeagerなプロップ向け）か、[`InertiaResponse`](#inertiaresponse-のビルダー)ビルダー（それ以外のすべて - レイジープロップ、ディファードプロップ、マージ、once、スクロール、フラッシュ）のいずれかを通じて構築したレスポンスを1つ返します。この章ではレスポンスの表面をエンドツーエンドで扱います: マクロ、ビルダー、v3プロトコルの機能（部分的なリロード、履歴の暗号化、バージョン検出）、`App::inertia_share*`による共有データ、そしてリダイレクトをまたいで運ばれるフラッシュバッグです。

まだフロントエンドを選んでいない場合は、[フロントエンド 概要](frontend.md)と[ページ コンポーネント](frontend-pages.md)を先に読んでください。この章はSPAブリッジが配線済みであることを前提とし、ハンドラが何を返すかに焦点を当てます。

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

[ルーティング](routing.md#router-level-redirects-and-views)を参照してください。そこではコンポーネントが実行時文字列なので、このマクロのコンパイル時存在検査は行われません - ハンドラを書かないこととのトレードオフです。

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

入れ子の型は通常どおり合成されます - フィールドは`Vec<T>`、`Option<T>`、入れ子の構造体など、`Serialize`可能なものなら何でもかまいません。入れ子の型自体は`InertiaProps`をderiveする必要はなく、`Serialize`だけが必要です。*トップレベルの*プロップ構造体に`#[derive(InertiaProps)]`を使うと、ツリー全体について自動的なTypeScript表面（[TypeScript 型](frontend-typescript-types.md)を参照）が得られます。

## `InertiaResponse` のビルダー

マクロはeagerな型付きプロップをカバーします。それ以外のすべて - レイジー、オプショナル、ディファード、マージ可能、クライアント側にキャッシュされるもの、フラッシュ、履歴暗号化の上書き - にはビルダーを直接使います:

```rust
use suprnova::{InertiaResponse, Request, Response, FrameworkError, HttpResponse};

pub async fn show(req: Request) -> Response {
    let resp = InertiaResponse::new("Posts/Show")
        .with("title", "Welcome")
        .with("post", load_post(42).await?)
        // Lazy: プロップが実際に送られるときにだけクロージャが走る
        // （初回訪問、またはこのキーを要求する部分リロード）。
        .lazy("recent_activity", || async {
            Ok::<_, FrameworkError>(load_activity().await?)
        })
        // Optional: 初回訪問では決して送られない。クライアントは
        // X-Inertia-Partial-Data で明示的にキーを要求する必要がある。
        .optional("permissions", || async {
            Ok::<_, FrameworkError>(load_permissions().await?)
        })
        // Defer: 初回のレンダリングではスキップされる。クライアントが
        // 追いかけのXHRを発行し、そのときクロージャが走る。
        .defer("notifications", || async {
            Ok::<_, FrameworkError>(load_notifications().await?)
        })
        // Merge: 部分リロードで既存のものへ追加する（「もっと読み込む」）。
        .merge("rows", next_page().await?)
        // Once: ナビゲーションをまたいでクライアント側にキャッシュされる。
        // サーバーが更新を強制しない限り、以降の訪問ではリゾルバがスキップされる。
        .once("plans", || async {
            Ok::<_, FrameworkError>(load_plan_catalog().await?)
        })
        // Flash: ワンショットのトースト。`props`ではなく`page.flash`の下に現れる。
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
        next_page,                                     // 新しいページのスライス
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
// `posts` はクエリビルダーから来た CursorPaginator。
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

`LengthAwarePaginator`、`Paginator`、`CursorPaginator`もこれを実装します。[ページネーション](pagination.md#inertia-integration-infinite-scroll-props)を参照してください。

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
    // 同期。起動時に一度だけ実体化される。
    App::inertia_share("appName", "Suprnova");
    App::inertia_share("appVersion", env!("CARGO_PKG_VERSION"));

    // 非同期。レスポンスごとに解決される（そのキーを除外する部分リロード
    // ではスキップされる）。
    App::inertia_share_lazy("locale", || async {
        Ok::<_, suprnova::FrameworkError>(detect_locale().await)
    });

    // ナビゲーションをまたいでクライアントにキャッシュされる - `share_once` はそれを
    // 必要とする最初のページで走り、その後クライアントはキャッシュキーが変わるまで
    // `X-Inertia-Except-Once-Props` 経由で再解決をスキップする。
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
        // ページごとに変える: ナビのカウントが必要なのは管理ダッシュボードだけ。
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
// 1. 現在のリクエストのフラッシュバッグへ入れる。
App::flash("toast", "Saved");

// 2. 特定のレスポンスへ付ける（このレスポンスにだけ同じ効果）。
InertiaResponse::new("Posts/Show").flash("toast", "Saved")

// 3. Redirectファサード経由でリダイレクトをまたいで運ぶ。
use suprnova::Redirect;

Redirect::to("/posts").with("toast", "Created")
```

`Redirect::with(key, value)`の形はハンドラをまたぐ経路です。値はセッションの`_flash.new.*`の下に着地し、次のリクエストの[`SessionMiddleware`](csrf.md)がそれを`_flash.old.*`へ歳を取らせ、行き先の`InertiaResponse`が`page.flash`の下に表面化させます。

同一リクエストのフラッシュ（タスクローカルバッグ）は、キーが衝突したとき継承されたセッションフラッシュに勝ちます。そのため行き先のハンドラは、キーを再フラッシュするだけで受信した値を上書きできます。

内部セッションキー（`_`が前置されたもの）は`page.flash`からフィルタされます。フォーム再投入用の`_old_input`と`_inertia.*`プロトコルフラグがクライアントへ漏れることはありません。

### リダイレクトのヘルパー

`Redirect`はLaravelの完全な表面です:

```rust
Redirect::to("/dashboard")                       // パスへの302
Redirect::route("posts.show").with("id", "42")   // 名前付きルート、ルートパラメータ
Redirect::back("/")                              // セッションに記録された直前のURL
Redirect::refresh()                              // 同じURL、新しいGET
Redirect::guest(&req, "/login")                  // intended URLを退避する
Redirect::intended("/dashboard")                 // 退避したURLを取り出す
Redirect::signed_route("downloads.show", &[("id","42")])?  // 署名付きURL
Redirect::to("/posts/42").preserve_fragment()    // 訪問をまたいで#fragを保つ
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

// デフォルト - ビルドマニフェストをハッシュする。何も書かなくてよい。
let cfg = InertiaConfig::new();

// マニフェストの場所を変える。バージョンはそれに従う。
let cfg = InertiaConfig::new().manifest_path("dist/.vite/manifest.json");

// 静的 - ビルド時の識別子を焼き込む。後続の `.manifest_path(...)` 呼び出しを
// 生き延びる: 明示的なバージョンは意図的なものだから。
let cfg = InertiaConfig::new().version(env!("CARGO_PKG_VERSION"));

// 動的 - コンテナのデプロイメントid、何でもよい。クロージャはバージョン検査の
// たびに走る。安価でないなら内側でキャッシュすること。
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

ほとんどのアプリは、`register_http_stack`からプロトコルミドルウェアを1回の呼び出しでインストールします。これはHTTP専用のブートストラップフックで、サーバーパスは実行しますが、queue、schedule、workflow、consoleバイナリはスキップします（[アプリケーション ブートストラップ](bootstrap.md)を参照）:

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    let cfg = InertiaConfig::new()
        .version(env!("CARGO_PKG_VERSION"))
        .default_title("My App");

    Inertia::install(&cfg)
        .expect("Inertia install failed (production needs a built frontend manifest)");
    // …残りのグローバルミドルウェアを、実行させたい順序で
}
```

Inertiaの層が依存するもの - `SessionMiddleware` - と、エラーページが読む必要のあるもの - `LocaleMiddleware` - は、この呼び出しの*上*に置きます。[後述の順序の規則](#ブートストラップ-inertia-install)を参照してください。

```rust
// cmd/main.rs
Application::new()
    .bootstrap(bootstrap::register)
    .http_bootstrap(|| async { bootstrap::register_http_stack() })
```

`bootstrap::register`の中には置かないでください。`public/assets`を出荷しないワーカーまたはconsoleイメージの状態がまさにそうであるように、`Inertia::install`はビルド済みフロントエンドマニフェストが本番で欠けているとfail closedします。プロセス全体のフックからインストールすると、そのバイナリも一緒に停止してしまいます。

`Inertia::install`は`Result`を返し、次の順序で処理します:

1. `cfg`が本番モード（`development == false` - `APP_ENV=production`のときは常にこれがデフォルト）に解決され、`cfg.manifest_path`からViteマニフェストをロードできない場合、fail closedします。これがCFG-01ガードです。未ビルドのフロントエンドで本番を起動すると、レガシーなハードコード済みアセットパスへ静かにフォールバックせず、はっきりエラーになります。
2. `InertiaHeadersMiddleware`を登録します - すべてのレスポンスに`Vary: X-Inertia`を設定し、Inertia訪問で空の`200`を`303`の戻りへ変えます。
3. `InertiaVersionMiddleware`を登録します - クライアントとサーバーがアセットバージョンで一致しない場合、`409` + `X-Inertia-Location`を出力します。
4. `Inertia303Middleware`を登録します - GET以外のInertiaリダイレクトで`302`を`303`へ格上げします。
5. `InertiaValidationRedirectMiddleware`を登録します - Inertia訪問の`422`を、エラーをフラッシュしたフォームページへの`303`へ変換します。[バリデーション失敗](#バリデーション失敗)を参照してください。
6. `cfg`が`.error_page(...)`を名指ししている**ときだけ**、`InertiaErrorPageMiddleware`を登録します - フレームワーク自身のエラーレスポンスを、そのページへ変えます。[エラーページ](#エラーページ)を参照してください。より外側に自分で登録している場合は、あなたのものがその位置と、それが名指ししているコンポーネントを保ち、このステップはスキップされます - [ページが描画される場所](#ページが描画される場所)を参照してください。

順序が重要です。ヘッダーミドルウェアが最初に登録されるため最も外側になり、ハンドラが実行される前にバージョンミドルウェアが返す`409`も含め、すべてのレスポンスを見ます。バリデーションリダイレクトミドルウェアは最後に登録されるため最も内側、つまりハンドラに最も近くなり、他の3つが触れる前の`422`を見ます。

`install`は**設定も保持します**。以後に構築されるすべての`InertiaResponse`はそこから出発するため、ここで設定した`.frontend(...)`、`.version(...)`、`.default_title(...)`、`.ssr(...)`、`.encrypt_history(...)`は、ハンドラが何も渡さなくてもすべてのページへ届きます。1ページだけ異なる設定を望むハンドラは`.with_config(...)`で上書きします。`Inertia::install`を呼ばないアプリは`InertiaConfig::default()`を得、`install`を再度呼ぶと保持された設定が置き換わります。

`.with_config(...)`は`version`も含めて設定を丸ごと置き換えます。`InertiaVersionMiddleware`は、それでも`Inertia::install`へ渡されたバージョンを解決するため、ここでの設定が同じ`.version(...)`を持たなければ、ページオブジェクトはミドルウェアが跳ね返すバージョンを広告してしまいます。そのページを訪れた後、クライアントはページ全体のロードをもう1回行うことになります。一致させるには、上書き側にも`.version(...)`を設定してください。

フラッシュデータを使う場合は、`SessionMiddleware`を`Inertia::install`**より前に**登録してください。バージョンミドルウェアはクライアントを跳ね返す前にセッションを再フラッシュするため、フラッシュされたエラーは追いかけのページ全体のGETを生き延びます。これはセッションスコープ内でのみ可能です。

[`LocaleMiddleware`](localization.md)も、[エラーページ](#エラーページ)を使うのであれば**その前に**登録してください。ミドルウェアの`next`より後のコードは、その内側にあるすべてがすでに戻ってから走ります。そのため、エラーページのミドルウェアが描画するのは、その内側で開かれたスコープがすべて取り払われた後です - ロケールミドルウェアにとってこれは、ページが訪問者のロケールではなくアプリのデフォルトのロケールを受け取ってしまう、ということです。Inertiaの層はローカライゼーションから何も読まないため、ロケールをその外側に置いてもコストはありません。スキャフォルドされる`bootstrap.rs`は、すでにそうしています。同じ理屈は、エラーページが読む必要のあるリクエストスコープを持つ、あなた自身のあらゆるミドルウェアに当てはまります。

この呼び出しより**後**に登録するものはすべて、エラーページにカバーされます。その上にあるものはカバーされません。`next`を呼ばずに自分で答えるミドルウェアは、自分のレスポンスを、その内側にある何にも手渡さないからです。`CsrfMiddleware`、レートリミッター、あるいは認証ガードをインストールより上に置かなければならないのなら、エラーページのミドルウェアをその間に自分で登録してください - [ページが描画される場所](#ページが描画される場所)を参照してください。

これらのミドルウェアのどれかを本当に望まない場合にだけ呼び出しを省略してください（まれです。それぞれが実際の失敗モードを塞ぎます - 1つのURLの2つの表現をまたぐキャッシュポイズニング、静かな古いバンドル、リダイレクト時のフォーム再送信、そしてクライアントのエラーモーダルで行き止まりになり`form.errors`へ届かないバリデーション`422`）。

## エラーページ

フレームワークから2xx以外が返ってきたInertiaの訪問は、エラーページを表示しません - クラッシュ画面を表示します:

```
All Inertia requests must receive a valid Inertia response, however a
plain JSON response was received.
```

クライアントが何かを描画する前に確認するのは、1つだけです: レスポンスの`X-Inertia: true`ヘッダーです。[認可](authorization.md)のチェックやRBACの権限ミドルウェアからの`403`、ルートのないパスに対する`404`、[レート リミット](rate-limiting.md)からの`429`、[失敗するハンドラ](errors.md)からの`500` - これらはどれもフレームワークのJSONのエラーボディを運び、そのヘッダーを持たないため、クライアントはそれらをモーダルへ引き渡します。ロールの合っていないユーザーがナビゲーションリンクをクリックすると、アプリは壊れたように見えます。

ページコンポーネントを名指しすれば、フレームワークは代わりにそのページを通してこれらのレスポンスを描画し、ステータスコードは保ちます:

```rust
use suprnova::{Inertia, InertiaConfig};

pub fn register_http_stack() {
    Inertia::install(
        &InertiaConfig::new()
            .version(env!("CARGO_PKG_VERSION"))
            .error_page("Error"),
    )
    .expect("Inertia install failed (production needs a built frontend manifest)");
}
```

`"Error"`は、ほかのページ名とまったく同じように解決されるため、`frontend/src/pages/Error.svelte`（あるいは`.tsx`、`.vue`）を置くだけで済みます。**3つのスターターは、すでにこれを同梱し、`.error_page("Error")`を設定しています** - 新しいプロジェクトは、何もしなくてもカバーされます。

### ページが描画される場所

`Inertia::install`は`InertiaErrorPageMiddleware`をInertiaの層の**最も内側**として登録します。そのため、ハンドラとルートのミドルウェアが実際に生み出したレスポンスを見ます。その呼び出しより*後*に登録するものも、すべてカバーされます - スキャフォルドが`CsrfMiddleware`をその下に置いているのは、これが理由です。

呼び出しより**上**に登録されたものは、カバーされません。`next`を呼ばずに自分で答えるミドルウェアは、自分のレスポンスを、その内側に登録された何にも手渡さないため、その拒否はそもそもInertiaの層へ届きません。噛みついてくるのは、セッションが切れたままフォームを送信する場合です: `CsrfMiddleware`は`{"message":"CSRF token mismatch."}`を伴う`419`で答えるため、それが`Inertia::install`より上にあると、ユーザーは、最も踏みやすい、まさにその1つのフローでクラッシュモーダルを見ることになります。外側のレートリミッターの`429`と認証ガードの`401`も、同じように振る舞います。

あなたのアプリがその形をしているのなら、その拒否をカバーさせたいミドルウェアの外側に、自分でミドルウェアを登録してください。これは1.3.6でも副作用として動いていました - 型は公開されており、グローバルな登録は型ごとにべき等なので、先に行われた登録はその位置を保っていたのです - けれども、そう述べたものはどこにもありませんでした。1.3.7からは、ドキュメント化された契約です: `install`はあなたの登録を確認し、`debug`でログに記録し、自分自身の登録を省きます。

```rust
use suprnova::{
    global_middleware, CsrfMiddleware, Inertia, InertiaConfig,
    InertiaErrorPageMiddleware, LocaleMiddleware, SessionConfig, SessionMiddleware,
};

pub fn register_http_stack() -> Result<(), suprnova::FrameworkError> {
    global_middleware!(SessionMiddleware::new(SessionConfig::from_env()));
    global_middleware!(LocaleMiddleware::from_env()?);

    // CSRFの外側なので、その下の層へ決して届かない419を見ることができる。
    global_middleware!(InertiaErrorPageMiddleware::new("Error"));
    global_middleware!(CsrfMiddleware::new());

    Inertia::install(&InertiaConfig::new().error_page("Error"))
}
```

`Inertia::install`はその登録を見て、自分自身のものを省き、そのことを`debug`で告げます。あなたが選んだ位置がそのまま通り、あなたが名指ししたコンポーネントもそのまま通ります - チェーンの中にいるのは、そのインスタンスです。ページを名指しするのは、自分自身の登録での**一度だけ**です。そのため、ここでは設定の`.error_page(...)`は省略可能になります: 残しても外してもかまいません。ほかにそれを読むものはありません。それでもなお、自分ではミドルウェアを置かないアプリのために`install`にミドルウェアを登録させるのは、この設定です。

自分で置く場合には、順序の規則が2つ付いてきます。

**`SessionMiddleware`と[`LocaleMiddleware`](localization.md)より後に。** このページはあなたの共有プロップ - `auth.user`、フラッシュ、ロケールの共有 - を運び、しかも出ていく*途中*で、その内側に登録されたすべてのミドルウェアが戻り、自分が開いたリクエストスコープを取り払った後に組み立てられます。この2つより上に登録されると、あらゆるエラーページは訪問者のセッションを失い、訪問者のものではなくアプリのデフォルトのロケールで描画されます。同じことは、エラーページの共有プロップがその状態を読む、あなた自身のあらゆるリクエストスコープのミドルウェアにも当てはまります。

**その拒否をカバーさせたいミドルウェアより前に**、そしてそれより外へは出さないでください。そこを通り抜けるレスポンスは、どれもそれが分類しなければならないボディを1つ増やすことになり、その外側にいるミドルウェアは、それが走る前に答えてしまうことが依然としてできます。

自分では何も登録しないのであれば、`Inertia::install`がこれをすべてやってくれます - そしてスキャフォルドされる`bootstrap.rs`は、すでに`SessionMiddleware`と`LocaleMiddleware`をその呼び出しの上に、`CsrfMiddleware`をその下に置いています。

### ページが受け取るもの

| プロップ | 型 | 常に存在するか | 何であるか |
|---|---|---|---|
| `status` | `number` | はい | 元のHTTPステータスです - `403`、`404`、`500`。 |
| `message` | `string` | はい | エラーボディの`message`、あるいはそれを運んでいなかった場合はステータスの理由句です。すでにサニタイズ済みです: `5xx`は`"Internal Server Error"`となり、根底のエラーが出ることは決してありません - これは`APP_DEBUG=true`の下でも変わりません。そこでJSONの経路が付け加える開発専用の`debug_message`フィールドは、意図的に読まれません。そのため、生のエラーはログとJSONのレスポンスの中に留まり、ページへ描画されることは決してありません。 |
| `request_id` | `string` | いいえ | エラーボディがそれを運んでいたときにだけ存在します。構造化ログが記録するのと同じIDなので、ページは運用者が検索できる参照番号を表示できます。 |

```svelte
<script lang="ts">
  interface ErrorProps {
    status: number
    message: string
    request_id?: string
  }

  let { status, message, request_id }: ErrorProps = $props()
</script>

<h1>{status}</h1>
<p>{message}</p>
{#if request_id}<p>Reference: {request_id}</p>{/if}
```

プロップは`types/inertia-props.ts`からインポートするのではなく、コンポーネントの中で宣言してください: [`suprnova generate-types`](frontend-typescript-types.md)はそのファイルをあなた自身の`#[derive(InertiaProps)]`構造体から書き直しますが、これらのプロップはフレームワークから来るものだからです。

### 差し替えを生き延びるもの

ステータスコードは保たれ、元のレスポンスが設定したすべてのヘッダーも保たれます。**例外は**2つのグループです。

**置き換えられるボディを説明していたもの。** すべての`Content-*`フィールド（置き換えたJSONの4倍の大きさのページに載る`Content-Length`は、フレーミングのバグです）と`Transfer-Encoding`です。`Content-Security-Policy`は、名指しでこの規則から除外されています - 歴史的な偶然でこのプレフィックスを共有しているだけで、表現のメタデータではなくレスポンスのポリシーだからです。

**そのボディをどう保存できるかを統制していたもの。** `Cache-Control`、`Expires`、`Age`、`ETag`、`Last-Modified`です。ページはあなたの共有プロップ - `auth.user`、フラッシュ、ロケールの共有 - を運びますが、それが置き換えたエラーボディは誰にとっても同じものでした。ですからページは、共有キャッシュに保存されて別の訪問者へ手渡される許可も、自分のものではないエンティティに属する検証子も、決して受け継いではなりません。代わりにページは、自分自身のために`Cache-Control: no-cache, private`を設定します - Laravelがセッションを運ぶレスポンスに与えるのと同じデフォルトです。

それ以外はすべて引き継がれます: `429`の`Retry-After`は引き続きクライアントにいつ戻ってくればよいかを伝え、`401`の`WWW-Authenticate`は引き続きチャレンジを運び、`Vary`、`Set-Cookie`、そしてあなたのリクエストIDのヘッダーも、そのまま届きます。この規則は、何が保たれるかではなく何が落とされるかとして述べられています。そのため、フレームワークが一度も耳にしたことのないヘッダーは、静かに消えるのではなく生き延びます。

どちらの相手もカバーされています。InertiaのXHRの訪問は`X-Inertia: true`付きのJSONのページオブジェクトを受け取り、ハードナビゲーション - 誰かが`/admin/articles`をアドレスバーに貼り付ける - は、どのページでも初回ロードで受け取るのと同じ、完全なHTMLシェルを受け取ります。ですからエラーページは、ユーザーがSPAを通して来たかどうかにかかわらず機能します。

### 決して手を触れないもの

このミドルウェアは、ほかの誰も答えを持っていないところでだけ、代わりを務めます。次のものは、そのままにします:

- **バリデーションの`422`。** それらは`InertiaValidationRedirectMiddleware`が所有します - [バリデーション失敗](#バリデーション失敗)を参照してください。そのミドルウェアを生き延びた`422`（`errors`オブジェクトがない、あるいはPrecognitionのドライラン）も、ボディを保ちます。
- **`X-Inertia-Location`を運ぶもの。** `409`のバージョンの跳ね返しと、RBACミドルウェアの`redirect_to`の形です。クライアントはボディではなくヘッダーに従って動きます。
- **リダイレクト。** 対象は`400`から`599`だけです。
- **APIクライアント。** `Accept`が`text/html`より`application/json`を好むリクエストは、これまでどおりのJSONの契約を保ちます。`curl`の`*/*`は好みなしとして扱われるため、こちらもJSONのままです。ページを受け取るのは、Inertiaの訪問かブラウザのナビゲーションだけです。
- **すでにInertiaのページであるレスポンス。** 自分自身のページを描画して`410`を与えたハンドラは、自分のコンポーネントを保ちます。
- **フレームワークのエラーの形ではないボディ。** あなた自身のHTMLのエラーページ、ルーター自身の`404 Not Found`ではない平文、あるいはキーの付け方が違うJSONのエンベロープ - どれも覆されることはありません。
- **`error_page`が未設定なら、すべて。** ミドルウェアはそもそも登録されないため、オプトインしていないアプリは、以前と寸分違わぬコードを実行します。

### どのボディが書き換えられるか

ゲートになるのは**ボディの形**であって、誰がそれを書いたかではありません。`400`から`599`のステータスで置き換えられる形は、ちょうど3つです:

- 空のボディ。
- `message`が文字列であるJSONオブジェクト - フレームワーク自身のエラーのエンベロープと、それと同じ形をしたすべて。
- ルーターの固定された`404 Not Found`という平文のボディ。

それ以外はすべて素通りします。つまり、あなたのミドルウェアが`HttpResponse::json(json!({ "message": "Unauthenticated." }))`で答える`401`はエラーページに**なります** - そうでなければクライアントがモーダルにしてしまうのは、まさにそのレスポンスなのですから、それが狙いです - そして、プロップまで生き残るのは`message`と`request_id`だけだ、ということでもあります。`errors`や`code`、そのほか何かを運ぶエンベロープは、ページになるときにそれらのフィールドを失います。

あなたのミドルウェアが、エラーのステータスで自分自身のJSONのボディを保たなければならない場合は、ゲートが一致しない形を与える - 人が読めるテキストのキーを`message`以外にする - か、レスポンスに自分で`X-Inertia: true`を設定してください。後者は、そのレスポンスがすでにInertiaのレスポンスであるという印になり、対象から外します。どちらも、そのレスポンスを組み立てる場所での1行です。

知っておく価値のある穴が1つあります: **パニックする**ハンドラには手が届きません。パニックの網はミドルウェアチェーン全体を包んでいるため、合成される`500`は、すべてのミドルウェアのフレームがすでに巻き戻された後に組み立てられます。パニックするハンドラは、それでもクライアントのモーダルを表面化させます。パニックする代わりに`Err(...)`を返せば（[エラーハンドリング](errors.md)を参照）、エラーページがそれをカバーします。

ページ自身の描画が失敗した場合 - コンポーネントを解決できない、SSRが落ちている、共有プロップがエラーになる - フレームワークはリクエストIDを伴う`warn`をログに記録し、元のエラーレスポンスを返します。壊れたエラーページが、それが描画していたエラーを覆い隠すことは決してありません。

### Suprnovaが異なる設計を選んだ理由

Laravelはこれを例外ハンドラに置きます: `bootstrap/app.php`を編集し、ステータスに自分でマッチさせ、`Inertia::render('Error', ['status' => $response->getStatusCode()])`を呼び、`$response->setStatusCode(...)`でコードを戻します。それは柔軟ですが、同時に、どのプロジェクトも手で書き直すフレームワークの配管でもあり、たいていは本番でモーダルを目にした後にそうすることになります。

ここでは、それが設定1行です。判断はどのアプリでも同じだからです: Inertiaの訪問かブラウザのナビゲーションはページを受け取り、APIクライアントはJSONを受け取り、ほかの契約が所有するものはすべてそのままにされます。その引き換えに、規則はあなたが書く`match`ではなく固定されたものになります。ですから、特定のレスポンスを対象から外すには、ゲートが認識しないボディを与えるか、すでにInertiaであると印を付けることになります - [どのボディが書き換えられるか](#どのボディが書き換えられるか)を参照してください。

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
  serverHead: true,        // `head`プロップを読む
  // serverHead: 'meta',   // あるいは別名のプロップを読む
  // serverHead: (page) => [...],  // あるいはページ全体から計算する
})
```

各文字列はHTML要素です。クライアントは、`data-inertia`属性を持たないものへそれを刻み込み、ナビゲーションをまたいでhead要素をdiffできるようにします。位置によるマッチングではなく安定した識別子が必要なら、自分で`data-inertia="og-title"`を指定してください。

ユーザーデータから補間するものはすべてエスケープしてください。これらの文字列はHTMLとして注入されるため、通常のルールが適用されます。

## SSR

Suprnovaはプロセス外のSSR ワーカー - 通常はNode / Bun / Denoの下で動く`@inertiajs/{svelte,react,vue}/server`の`createServer()`バンドル - とHTTPループバック経由で通信します。[`Inertia::install`](#ブートストラップ-inertia-install)に渡す設定で有効にしてください。その設定がすべてのレスポンスの出発点なので、ハンドラを通して配管するものはありません:

```rust
Inertia::install(
    &InertiaConfig::new()
        .ssr("http://127.0.0.1:13714")  // ワーカーの URL
        .ssr_timeout(std::time::Duration::from_millis(500))
        .ssr_exclude("/admin/**")
        .ssr_max_response_bytes(8 * 1024 * 1024),
)?;
```

SSRはデフォルトでオフで、設定のプロパティです。インストールされた設定から構築されるすべてのレスポンスではオンになり、SSRを設定しない`.with_config(...)`で上書きするレスポンスではオフになります。有効な場合、フレームワークはページオブジェクトを`<url>/render`へPOSTし、`{ head, body }`をHTMLシェルにインライン化します。自分自身の`<title>`を運ぶワーカーのhead - Inertiaの`Head`コンポーネントを使うページはすべてこれに当たります - は、シェルのタイトルに加わるのではなく、それを**置き換えます**。これは設定の`.default_title(...)`と、レスポンスごとの`.title(...)`の両方に当てはまります: タイトルが2つあるドキュメントは先頭のものを表示するため、シェルのタイトルが、タブでも、検索結果でも、あらゆるリンクプレビューでも、ページの本当のタイトルに勝ってしまうからです。SSRがオンなら、タイトルはレスポンスではなく`Head`で設定してください。タイトルを持たないheadは、シェルのタイトルをそのままの場所に残します。ワーカーのエラーやタイムアウト時はレスポンスがCSR（クライアントがhydrateする空の`<div id="app">`）へフォールバックし、`on_ssr_error(...)`フックが発火します。代わりにCIで`ssr_throw_on_error(true)`を設定すると、失敗をハードな500にできます。

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

`suprnova new`はすべてのstarterで`frontend/src/ssr.{ts,tsx}`と`build:ssr` npmスクリプトをscaffoldします。ビルドしてからワーカーを起動します:

```bash
cd frontend && npm run build:ssr
suprnova ssr:start
```

`suprnova ssr:check`はワーカーが実際に応答していることを検証します。ワーカー自身の`GET /health`ルートへアクセスしますが、これはすべての`createServer()`バンドルが追加コードなしで公開するものです。

## 設定

Inertiaの動作は`InertiaConfig`でプログラム的に設定され、[`Inertia::install`](#ブートストラップ-inertia-install)に渡した設定がすべてのレスポンスの出発点になります。フレームワークが直接読む環境変数は`SUPRNOVA_FRONTEND`（`svelte` / `react` / `vue`）だけです。設定に指定がない場合に限り、デフォルトのエントリポイントファイル名とページコンポーネント拡張子を供給します。インストール済み設定で明示した`.frontend(Frontend::React)`が勝ち、`suprnova new --frontend react`がscaffoldする内容になります。それ以外はすべてビルダー形状です:

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
    .with_all_errors(false)                   // フィールドごとに1メッセージ、またはすべて
    .url_resolver(|req| req.path_and_query()) // `page.url` の導出方法
    .production();                            // false → Vite開発サーバーから読み込む
```

フロントエンド固有のデフォルト:

| フロントエンド | デフォルトエントリポイント | ページ拡張子 |
|---|---|---|
| Svelte（デフォルト） | `src/main.ts` | `.svelte` |
| React | `src/main.tsx` | `.tsx`、`.jsx` |
| Vue | `src/main.ts` | `.vue` |

HTMLシェルの属性のうち、2つは特筆に値します。

`<title>`は、レスポンスの`.title(...)`から来ます。レスポンスが何も設定しなかった場合は`.default_title(...)`から来ます。[SSR](#ssr)の下では、ページ自身のheadが**その両方**に勝ちます: `<title>`を運ぶワーカーのheadがドキュメントの唯一のタイトルになり、シェルは自分のタイトルをまったく出力しません。

`<html lang="...">`は、あなたが設定できない唯一の属性です。正しい値がすでに分かっているからです - それは、そのリクエストで有効なロケール、つまり`LocaleMiddleware`が検出したもの、あるいは何も検出しなかった場合は設定された`APP_LOCALE`です。[ローカライゼーション](localization.md)を参照してください。スクリーンリーダーはその属性から声を選び、検索エンジンはそれをページの言語として読むため、複数の言語を配信するアプリは、それを直すために出来上がったドキュメントを書き換える必要が、もうありません。

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

Rustのプロセスモデルは正反対です。1つのプロセスが多数のスレッドをまたいで多数の並行リクエストを処理します。そのためレジストリはプロセスグローバルなstaticではなく、[サービス コンテナ](container.md)（task-local → thread-local → global）に存在します。`App::inertia_share*`はアクティブなコンテナの`InertiaRegistry`へ書き込みます。これにより`TestContainer::fake()`を使うテストは何も登録解除せずにきれいな分離を得られます。表面はLaravelと同じですが、ランタイムが異なるため下の機構が違います。

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

- [ページ コンポーネント](frontend-pages.md) - フロントエンドがコンポーネント名をSvelte / React / Vueモジュールへ解決する仕組み
- [TypeScript 型](frontend-typescript-types.md) - `suprnova generate-types`が`#[derive(InertiaProps)]`構造体からTS定義を出力する
- [データ オブジェクト](data.md) - 部分的なリロードと合成される、フィールドごとのinclude / allowlistゲーティングを備えたDTO用の`#[derive(Data)]`
- [エラー モデル](error-model.md) - `Response`、パニック境界、`FrameworkError`がInertiaレスポンスをどのように通り抜けるか
- [サービス コンテナ](container.md) - `App::inertia_share*`と`InertiaSharedData`の背後にあるルックアップモデル
