# ページネーション

Suprnovaは、Laravelの表面に一行一行一致する、3つのページネーターを出荷します: length-aware（合計を知っている）、simple（ページごとに1クエリ）、そしてcursor（不透明なキーセット）です。3つとも、InertiaとJSON:APIの消費者がすでに理解しているLaravel形のJSONへ `Serialize` を導出します - あなたは1ページを取得し、それを返します。それ以外は何も必要ありません。

```rust
use crate::models::User;

let page = User::query()
    .filter("active", true)
    .order_by_desc("created_at")
    .paginate(20)
    .await?;
```

その1回の呼び出しは、`COUNT(*)` と `LIMIT/OFFSET` によるページの取得を実行し、アクティブなリクエストから `?page=N` をパースし、出荷可能な `LengthAwarePaginator<User>` を返します。2つの兄弟 - `simple_paginate(20)` と `cursor_paginate(20)` - は、異なるトレードオフを伴って同じ形の値を返します。この章の残りは、どれに手を伸ばすべきか、それぞれが何を犠牲にするか、そしてJSONがどのように届くか、についてです。

## ページネーターを選ぶ

選ぶための最も速い方法は、トレードオフの表です:

| メソッド | 型 | ページあたりのクエリ数 | 合計を知っている？ | 使うべき場面 |
|---|---|---|---|---|
| `paginate(n)` | `LengthAwarePaginator<M>` | 2（`COUNT(*)` + ページ） | はい | UIが数字のページ、あるいは「17ページ中3ページ目」を表示する |
| `simple_paginate(n)` | `Paginator<M>` | 1（`LIMIT n+1`） | いいえ | 大きなテーブル。「次へ」ボタンで十分 |
| `cursor_paginate(n)` | `CursorPaginator<M>` | 1（`LIMIT n+1`） | いいえ | 無限スクロール。よく使われるテーブルの深いページ |

このコストの違いは、テーブルが大きくなると重要になります。1億行にわたる `COUNT(*)` は、あなたのリクエストの予算の中で最も高価なクエリです。`simple_paginate` はカウントを節約します。`cursor_paginate` はカウントを節約し、*かつ*、大きなテーブルへの深いページのリクエストすべてを苦しめる `OFFSET N` の線形スキャンを避けます - 正しいインデックスがあれば、カーソルのシークは、結果集合の中でユーザーがどこにいるかにかかわらず、`O(1)` に近いものです。

### Suprnovaが異なる設計を選んだ理由

Laravelのページネーターは、URL構築用のヘルパー - `nextPageUrl()`、`previousPageUrl()`、Bladeがレンダリングする `{url, label, page, active}` 記述子の `links` 配列 - を運びます。Suprnovaの生の `Serialize` 実装は、データのスライスとカウンターを発します。URLの構築は、すでにURLのコンテキストを所有しているレスポンス形のコンストラクタの側にあります: [`Inertia::paginate`](frontend-inertia-responses.md) はInertiaのスクロールメタデータ（絶対URLではなく、ページの識別子）を添付します。[`Resource::paginated`](eloquent-resources.md) は、JSON:APIの推奨に従って、JSON:APIの `links.{self,first,last,prev,next}` を添付します。

この分割には、2つの理由があります。第一に、クライアントが見るべきURLは、どのプロトコル表面がそれをレンダリングしているかに依存します - Inertiaはページ識別子を軸にし、JSON:APIは絶対hrefを求めます。第二に、ページネーターはデフォルトでは、リクエストのベースURLを知りません。それを知っているヘルパーが、URLをふさわしい場所で一度だけ添付できます。裸のページネーター上でURLがどうしても必要な場合（カスタムのJSONエンベロープ、テレメトリのペイロード、テストのアサーション）は、`with_path(...)` を呼び、`url_for_page(n)` を使ってください - [URL 生成とパス](#url-生成とパス)のセクションでカバーされています。

## `paginate` - 合計を把握する

```rust
use suprnova::LengthAwarePaginator;
use crate::models::User;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let page: LengthAwarePaginator<User> = User::query()
        .filter("active", true)
        .order_by_desc("created_at")
        .paginate(20)
        .await?;

    Ok(suprnova::json_response!(page))
}
```

この構造体の公開フィールドです:

```rust
pub struct LengthAwarePaginator<T> {
    pub data: Vec<T>,           // このページの行
    pub current_page: u64,       // 1から始まる
    pub last_page: u64,          // 1から始まる。total == 0のときは0
    pub per_page: u64,
    pub total: u64,              // すべてのページをまたぐ、あらゆる行
    pub from: Option<u64>,       // このページの最初の行の、1から始まるインデックス
    pub to: Option<u64>,         // このページの最後の行の、1から始まるインデックス
    pub path: Option<String>,    // url_for_pageのためのベースURL（オプション）
}
```

導出された `Serialize` が発するJSONです:

```json
{
  "data": [...],
  "current_page": 1,
  "last_page": 3,
  "per_page": 10,
  "total": 25,
  "from": 1,
  "to": 10,
  "path": "/api/users"
}
```

`path` は、未設定のときはJSONから省略されます。`from` と `to` は、ページが空のとき（このページに行がない、あるいはリクエストされたページが最後のページを超えている）は `null` になります。

### `?page=N` を自動的に読み取る

`paginate(n)` は、`Context::query_param` を介して、アクティブなリクエストの `?page=N` から現在のページを読み取ります。欠落、空、数値でない、そしてゼロの値は `1` へクランプされます。配線する必要は何もありません - リクエストがスコープの中にあれば、パラメータは読み取られます。

### 1つのページの中の複数のページネーター

1つのページが2つ以上のページネーション済みリストをレンダリングするときは、`paginate_using` で、それぞれに自分自身のクエリ文字列キーを与えてください:

```rust
let posts = Post::query()
    .order_by_desc("created_at")
    .paginate_using("posts_page", 10)
    .await?;

let comments = Comment::query()
    .order_by_desc("created_at")
    .paginate_using("comments_page", 25)
    .await?;
```

`paginate_using` は、返されるページネーターに `page_name` も設定するため、`url_for_page` は同じキーでURLを構築します:

```rust
posts.url_for_page(2);     // "/posts?posts_page=2"（pathが設定されている場合）
comments.url_for_page(3);  // "/posts?comments_page=3"
```

### ページ位置の述語

Laravelの `AbstractPaginator` の述語集合全体が実装されています:

```rust
page.has_more_pages();   // current_page < last_page
page.on_first_page();    // current_page <= 1
page.on_last_page();     // !has_more_pages()
page.has_pages();        // ページ1にいないか、あるいはさらにページが存在する
page.is_empty();         // data.is_empty()
page.is_not_empty();     // !is_empty()
page.count();            // data.len() - ページのスライス、合計ではない
```

`count()` はスライスのサイズであり、合計ではありません - Laravelの `Countable` の形です。合計には `total` フィールドを直接使ってください。

## `simple_paginate` - 1クエリ、カウントなし

```rust
use suprnova::Paginator;
use crate::models::User;

let page: Paginator<User> = User::query()
    .order_by_desc("id")
    .simple_paginate(20)
    .await?;
```

```rust
pub struct Paginator<T> {
    pub data: Vec<T>,
    pub current_page: u64,
    pub per_page: u64,
    pub has_more: bool,          // per_pageを超える余分な行があったか？
    pub path: Option<String>,
}
```

JSON:

```json
{
  "data": [...],
  "current_page": 1,
  "per_page": 10,
  "has_more": true,
  "path": "/api/users"
}
```

仕掛けはSQLの中にあります。`simple_paginate(20)` は `LIMIT 21` を発行し、21番目の行が返ってきたかどうかを確認し、それから `has_more` を設定し、`data` を20件へ切り詰めます。ページあたり1クエリ。`COUNT(*)` はありません。

`total`、`last_page`、`from`、`to` を犠牲にします。その代わりに、ページ読み込みごとに `COUNT(*)` を実行するのが高価すぎるテーブルをページネーションできます。UIの表面は「次へ」/「前へ」ボタンであり、「142ページ中7ページ目」ではありません。

length-awareなページネーターと同じ述語集合が実装されています: `has_more_pages()`、`on_first_page()`、`on_last_page()`、`has_pages()`、`is_empty()`、`is_not_empty()`、`count()`。

## `cursor_paginate` - 不透明なキーセット

```rust
use suprnova::CursorPaginator;
use crate::models::User;

let page: CursorPaginator<User> = User::query()
    .cursor_paginate(20)
    .await?;
```

```rust
pub struct CursorPaginator<T> {
    pub data: Vec<T>,
    pub per_page: u64,
    pub next_cursor: Option<String>,  // 最後のページではNone
    pub prev_cursor: Option<String>,  // 最初のページではNone
    pub path: Option<String>,
}
```

JSON:

```json
{
  "data": [...],
  "per_page": 10,
  "next_cursor": "...",
  "prev_cursor": null,
  "path": "/api/users"
}
```

`next_cursor` と `prev_cursor` は、常にJSONのキーとして存在します（不在のときは `null`）。そのため、クライアントのスキーマはフィールドの存在に依存できます。`path` は未設定のときは省略されます。

### カーソルは通信上でどう働くか

クライアントは、前のページのカーソルを `?cursor=<opaque>` を通じて渡します:

```
GET /api/users?cursor=eyJ0IjoiQmlnSW50IiwidiI6MTAwLCJkIjoibmV4dCJ9...
```

`cursor_paginate` はカーソルをデコードし、キーセットのフィルタ（`next` には `pk > boundary ASC`、`prev` には `pk < boundary DESC` で、ASCへ戻すように反転される）をたどり、`LIMIT n+1` 行を取得し、ページの隣が存在するのに応じて `next_cursor` / `prev_cursor` を再び発します。これは双方向です - クライアントは、自分の位置を失うことなく、前後に歩けます。

カーソルページネーションは、ビルダー上の既存の `ORDER BY` を**置き換えます**。キーセットのフィルタが決定的にテーブルを切り分けるためには、主キーに対する安定した全順序が必要です。任意の `ORDER BY random_score()` のカーソルは、行を読み飛ばしたり重複させたりしてしまいます。主キー以外の並び順が必要な場合は、`paginate` / `simple_paginate` へ切り替えてください。

### カーソルは暗号化され認証されている

Suprnovaのカーソルは、Laravelのbase64-JSON平文では**ありません**。通信上のカーソルは、キーセットの境界（型付きの `sea_orm::Value` - `Int`、`BigInt`、`Uuid`、日時、10進数、文字列、バイト列）と方向タグを合わせたものであり、JSONエンコードされた上で、フレームワークの `Crypt` キーリングを介してAES-256-GCMで封じられます（`CryptPurpose::Cursor` に束縛されているため、カーソルの暗号文が、他のいかなる表面 - クッキー、2FAのシークレット、キャスト - へリプレイされることは決してありません）。

これは、実務上3つのことを意味します:

1. **改ざん不可。** `?cursor=` の中のビットを反転させるクライアントは、別のページのデータではなく、400の `Invalid pagination cursor` を受け取ります。
2. **情報の漏洩なし。** 境界値（多くは主キー、時にはタイムスタンプ）はカーソルの内側に封じられています - クライアントは、それを編集することで範囲を列挙することはできません。
3. **型付きの境界は、損失なく往復します。** 通信上のエンベロープはSeaORMのバリアント（`"BigInt"`、`"Uuid"` など）にタグを付けるため、デコード時に値は、元のカラムが発したのと同じSQL型で再バインドされます。Postgres / MySQL / SQLiteをまたぐ文字列強制のバグはありません。

平文へのフォールバックはありません。`Crypt` が初期化されていない場合 - `Server::from_config` の後ではあり得ないはずですが - は、偽造可能なカーソルを発する代わりに、エンコードがエラーになります。

### Suprnovaが異なる設計を選んだ理由

Laravelのカーソルページネーターは、デフォルトでは前方専用であり、通信上のカーソルはbase64エンコードされたJSONのブロブです - 読める、編集できる、リプレイできます。Suprnovaのカーソルは双方向であり（Laravelが後から追加した `cursorPaginate()` の表面と一致します）、エンドツーエンドで認証されているため、クライアントはそれを構築したり改変したりできません。Rustのエコシステムには、すでにAES-GCMがプリミティブとして存在します。それを使うコストは、フレームワークにとって1つ余分なトレイト実装であり、その見返りに、平文のbase64ペイロードでは提供できないセキュリティ特性を、すべてのカーソルに与えます。

## ファサード - `Pagination::length_aware` / `Pagination::cursor`

このマニュアルのほとんどの章は、ページネーションをEloquentビルダーを通じて示しています。それが一般的な経路だからです。SeaORMの `Select<E>` を直接構築している場合 - たとえば、レポートのために、モデルを持たないクエリへ結合している場合 - は、`Pagination` ファサードが等価な表面です:

```rust
use suprnova::{Pagination, LengthAwarePaginator};
use sea_orm::EntityTrait;

let select = User::find()  // あるいは任意のSeaORM Select<E>
    .filter(user::Column::Active.eq(true));

let page: LengthAwarePaginator<user::Model> =
    Pagination::length_aware(select, 20, 1).await?;
```

ファサードはまた、特定の名前付き接続へルーティングするための `length_aware_on(conn, ...)` と `cursor_on(conn, ...)` も提供します。そして、キーセットのカラムを明示的に取る、型付きの `cursor(query, cursor, per_page, order_col)` の形もあります - カーソルが主キー以外の何かでソートする場合に使われます。

ルーティングのルールは、Eloquentビルダーと一致します。周囲の `DB::transaction` は尊重されます（COUNTとページのクエリは、どちらもトランザクションの接続上で実行されます）。そして、登録済みの `__read_replica__` 接続は、読み取りに対して自動的に使われます。`__primary__` の番兵は、レプリカを迂回したいときに、デフォルトのプールを選びます。

## バリデーション - `per_page == 0`

3つのメソッドすべては `per_page == 0` を拒否します:

```rust
let result = User::query().paginate(0).await;
assert!(matches!(
    result,
    Err(FrameworkError::ParamError { ref param_name }) if param_name == "per_page",
));
```

このエラーは、標準のエラーボディを伴うHTTP 400としてレンダリングされます。サイレントな「空のページ」はありません - ゼロのページサイズは常に誤りであり、Eloquentビルダーと `Pagination` ファサードに一致する形で、呼び出し箇所で拒否されます。同じバリデーションが、`cursor_paginate`、`simple_paginate`、`Pagination::length_aware`、`Pagination::length_aware_on`、`Pagination::cursor`、`Pagination::cursor_on` にあります - 1つのルール、6つのエントリーポイントです。

`current_page` の値は、バリデーションされるのではなく、**クランプされます**: `0` は `1` になります。防御的なフロントエンドからの負の数は起こりえません（パーサーは `u64` です）。そして、`last_page` より大きい `?page=N` は、空の `data` と `None` の `from`/`to` を持つページネーターを返します。終わりを越えて歩くことは、クライアントの誤りであり、エラーではありません。

## エラーの形

| 条件 | バリアント | HTTP |
|---|---|---|
| `per_page == 0` | `FrameworkError::ParamError { param_name: "per_page" }` | 400 |
| 改ざんされた / 無効なカーソル | `FrameworkError::Domain`（`"Invalid pagination cursor"`） | 400 |
| カーソルのデコード時に `Crypt` が未初期化 | `FrameworkError::Internal` | 500 |
| `decode_cursor` でのカーソルバリアントの不一致 | `FrameworkError::Internal` | 500 |
| 基盤となるDBの失敗 | `FrameworkError::Database` | 500 |

覚えておくべきなのは、改ざんされたカーソルのケースです。カーソルは通信上から直接読み取られます - `?cursor=…` というクエリ文字列は、定義上、攻撃者の入力です。そして、ビットが反転したbase64や、リプレイされた暗号文は、サーバーのバグではなく、想定された失敗モードです。復号のステップは400の `Invalid pagination cursor` へ格下げされるため、クライアントが引き起こせる失敗が、500のテレメトリチャネルを汚染することはありません。静的なメッセージは、クライアントに探りを入れる材料を何も与えません。

復号後の失敗（JSONパース、バリアントタグのディスパッチ、方向のパース）は500のままです - AEAD認証を生き延びたバイト列は、*私たち自身*が生成したものです。そのため、その先で不正な形のペイロードがあれば、それは指摘する価値のあるフレームワークのバグです。

## URL 生成とパス

生のページネーターは、オプションの `path` フィールドを運びます。設定されている場合、`url_for_page(n)` とカーソルのリンク生成は、それを使ってクエリ文字列を構築します:

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/api/users");

page.url_for_page(1);    // "/api/users?page=1"
page.url_for_page(2);    // "/api/users?page=2"
```

ベースパスがすでにクエリ文字列を運んでいる場合、URLが正しい形のままであるよう、区切り文字は `&` に切り替わります:

```rust
let page = User::query()
    .paginate(20)
    .await?
    .with_path("/users?sort=name");

page.url_for_page(2);    // "/users?sort=name&page=2"
```

`path` が未設定の場合、`url_for_page` は裸の相対クエリ `?page=2` にフォールバックします。ページパラメータの名前は `with_page_name(...)`（デフォルトは `"page"`）から来ます。`paginate_using(name, n)` はそれを自動的に設定するため、生成されるURLは、ページネーターが駆動された元と同じキーを使います。パラメータ名はform-urlencodedされるため、予約文字を持つ名前であっても、URLを壊すことはできません。

カーソルのページネーターも同じ形です: `with_path(...)` がベースを設定し、`with_cursor_name(...)` がクエリのキーを上書きします（デフォルトは `"cursor"`）。そして、JSON:APIのリンクビルダーは、それらを自動的に拾い上げます。

ほとんどのアプリは、`url_for_page` を直接呼びません - それらは、ページネーターを下の2つの統合表面のどちらかに渡し、それがそれぞれのプロトコルにとって正しい方法でURLを構築します。

## Inertia統合 - 無限スクロールのprop

Inertiaのフロントエンドには、`Inertia::paginate(component, key, paginator)` ヘルパーが、ページネーターをスクロールpropとして添付します:

```rust
use suprnova::Inertia;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .order_by_desc("created_at")
        .cursor_paginate(20)
        .await?;

    Ok(Inertia::paginate("Users/Index", "users", users).into())
}
```

3つのページネーターすべてが、ここで機能します - `LengthAwarePaginator`、`Paginator`、そして `CursorPaginator` です。メタデータのページ名は、ページネーター自身から来ます: 2つのオフセットページネーターには `"page"`、`CursorPaginator` には `"cursor"` です。クライアントは、選ばれたpropキーの下の行と、`current_page`、`next_page`、`previous_page`（オフセットページネーターにはページ識別子、カーソルページネーターにはカーソル文字列）を持つ `ScrollMetadata` 記述子を受け取ります - これを `useInfiniteScroll` / `WhenVisible` のInertiaヘルパーが、無限スクロールのために消費します。

各ページネーターは、その記述子を `ProvidesScrollMetadata` を通じて構築します。これはLaravelのページネーターアダプターも満たす同じインターフェースです（`ProvidesScrollMetadata::getPageName` / `getPreviousPage` / `getNextPage` / `getCurrentPage`）。このクレートが知らないページネーター - サードパーティクレートのカーソル型や手書きのリポジトリ結果 - も、4つのメソッドを実装して同じ方法でフレームワークに `ScrollMetadata` を渡せます。[Inertiaレスポンス](frontend-inertia-responses.md#merge-strategies-and-infinite-scroll)を参照してください。

`simple_paginate` は特に触れる価値があります。`COUNT(*)` がリクエストの支配的なコストになるほど大きなテーブルにわたるリスティングこそが、まさにInertiaのコレクションページが痛むところだからです:

```rust
let users = User::query()
    .order_by_asc("id")
    .simple_paginate(20)     // COUNTなし、1クエリ
    .await?;

Ok(Inertia::paginate("Users/Index", "users", users).into())
```

その `next_page` は、計算された最後のページからではなく、`LIMIT n+1` のオーバーフロー確認から来ます。そこから計算すべき合計がそもそもないからです。クライアントは、「4,812ページある」ではなく「もう1ページある」を受け取ります - これは、無限スクロールのUIがこれまで読んできたすべてです。

### 行を送り出す前に投影する

ページネーターには、`map` / `through`（Laravelにはあります）がありません。代わりに、公開フィールドから再構築してください - カウンターとカーソルは*クエリ*を記述するものであるため、行の型が変わっても変化せずに持ち越されます:

```rust
let page = User::query().cursor_paginate(20).await?;

let page = suprnova::CursorPaginator::new(
    page.data.into_iter().map(PublicUser::from).collect(),
    page.per_page,
    page.next_cursor,
    page.prev_cursor,
);
```

ルートが未認証であり、モデルが呼び出し元が見るべきでない何かを運んでいるときは、モデルを直接シリアライズするよりも、これを行う価値があります。ユーザーテーブルにわたるカーソルは、一度に1ページずつ手渡しますが、最終的にはすべてのページを手渡してしまいます。

同じヘルパーは、ページネーターを他のpropと混ぜたい場合のための、`InertiaResponse::paginate(key, paginator)` 上の連鎖可能なメソッドとしても存在します:

```rust
inertia_response!("Dashboard")
    .with("stats", &stats)
    .paginate("recent_users", users)
    .into()
```

より広いpropモデルについては、[Inertia レスポンス](frontend-inertia-responses.md)を参照してください。

## JSON:API統合 - `Resource::paginated`

JSON:APIの消費者には、`Resource::paginated(paginator)` が完全なエンベロープを構築します:

```rust
use suprnova::Resource;

pub async fn index(_req: suprnova::Request) -> suprnova::Response {
    let users = User::query()
        .paginate(20)
        .await?
        .with_path("/api/users");

    Ok(Resource::paginated(users).into())
}
```

レスポンスは、次のものを運びます:

- `data` - モデルの `IntoJsonResource` を通じてレンダリングされたすべての行。
- `meta.pagination` - length-awareには `{ total, per_page, current_page, last_page }`、cursorには `{ next_cursor, prev_cursor }`。
- `links.{self,first,last,prev,next}` - length-awareなページネーターには絶対href（`path` から構築される）。cursorなページネーターには `links.{prev,next}`。

両方のページネーターの型は、`Resource::paginated` が消費する `Paginated<T>` トレイトを実装しています - length-aware対cursorのための、別個のコードパスはありません。`Paginated<T>` を実装する、独自のページネーターに似た型を構築した場合、それは同じように合成されます。

リソースモデルについては、[JSON:API リソース](eloquent-resources.md)を参照してください。

## カスタムのJSONエンベロープ

InertiaもJSON:APIも、あなたのクライアントに一致しない場合は、`json_response!` を通じてページネーターを直接出荷してください:

```rust
let page = User::query().paginate(20).await?;
Ok(suprnova::json_response!({
    "users": page.data,
    "pagination": {
        "current_page": page.current_page,
        "last_page": page.last_page,
        "per_page": page.per_page,
        "total": page.total,
    }
}))
```

あるいは、ページネーター全体をそのまま渡してください - 導出された `Serialize` 実装は、上で文書化した形を発します:

```rust
Ok(suprnova::json_response!(User::query().paginate(20).await?))
```

フィールドは公開されています。あなたの契約が要求するとおりに、形を変えてください。

## 接続をまたいだルーティング

ページネーションは、Eloquentビルダーが使うのと同じマルチ接続ルーティングを尊重します。`DB::transaction(...)` の内側では、COUNTとページのクエリは、どちらもトランザクションの接続上で実行されます - それらが複数の接続に分かれることは決してないため、カウントが、それが記述するページと食い違うことは決してありません。登録済みの `__read_replica__` は、トランザクションの外側の読み取りに対して自動的に使われます。ページネーターを特定の名前付き接続に固定するには、`Pagination` ファサード上の `_on(connection, ...)` バリアント、あるいはEloquent側の `Builder::on("replica_b").paginate(20)` を使ってください。

ルーティングの契約については、[Eloquent - マルチ接続ルーティング](eloquent.md)を参照してください。

## どれに手を伸ばすべきか

おおよその判断の木です:

- **数字のページUIが設計の一部である** → `paginate`。「17ページ中3ページ目」をレンダリングするには `last_page` が必要であり、あなたのテーブルのサイズであればCOUNTのコストは問題ありません。
- **「次へ」/「前へ」ボタンだけ、大きなテーブル** → `simple_paginate`。ページあたり1クエリ。`total` と `last_page` を犠牲にしますが、ページの読み込みは半分になります。
- **無限スクロール** → `cursor_paginate`。双方向のカーソルは、OFFSETが先に何千行もスキャンすることなく、クライアントが1000ページを超えてスクロールを続けられることを意味します。
- **よく使われる、追記専用フィードの末尾** → `cursor_paginate`。主キーによるキーセットの並び順は、並行性に対して安全です: 新しい行はカーソルの向こう側に着地し、内側に入り込むことは決してありません。OFFSETベースのページネーションは、挿入が起きている間、行を読み飛ばします。
- **Eloquentモデルの外側で `Select<E>` を構築する** → `Pagination::length_aware` / `Pagination::cursor`。トレードオフは同じです。ファサードは、モデルを持たない等価物です。

迷ったときは、`paginate` から始めてください。`COUNT(*)` があなたの遅いクエリログに現れるようになったら、`simple_paginate` へ移ってください。深いページがリクエスト時間を支配し始めたとき、あるいはUIが無限スクロールであるときは、`cursor_paginate` へ移ってください。

## 各要素の実装場所

| 要素 | ファイル |
|---|---|
| `Pagination` ファサード、`Paginated<T>` トレイト | `framework/src/pagination/mod.rs` |
| `LengthAwarePaginator<T>` | `framework/src/pagination/length_aware.rs` |
| `Paginator<T>`（simple） | `framework/src/pagination/simple.rs` |
| `CursorPaginator<T>`、`CursorDirection`、`encode_value`、`decode_value` | `framework/src/pagination/cursor.rs` |
| `IntoInertiaScroll` ブリッジ | `framework/src/pagination/inertia.rs` |
| `Builder::paginate` / `simple_paginate` / `cursor_paginate` | `framework/src/eloquent/builder.rs` |
| `Inertia::paginate`、`InertiaResponse::paginate` | `framework/src/inertia/facade.rs`、`framework/src/inertia/response.rs` |
| `Resource::paginated`、`JsonApi::paginated` | `framework/src/resources/response.rs` |

## 次のステップ

- [Eloquent API](eloquent.md) - `Builder::paginate*` から返されるすべてのページネーターを駆動するモデル層
- [クエリ ビルダー](queries.md) - `Pagination::length_aware` と `Pagination::cursor` と組み合わさる、モデルを持たないクエリ
- [Inertia レスポンス](frontend-inertia-responses.md) - スクロールpropがどのように、ページネーターをInertiaのページへ添付するか
- [JSON:API リソース](eloquent-resources.md) - `Resource::paginated`、links、meta、そして `Paginated<T>` トレイト
- [エラー モデル](error-model.md) - `FrameworkError::param` のバリデーションルールと、カーソル改ざん時の格下げ
