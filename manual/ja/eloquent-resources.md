# JSON:API リソース

Suprnovaは、型付きREST APIのためのJSON:APIリソース層を出荷しています。`#[derive(Data)]` 構造体に `#[json_resource("type")]` を付けると、フレームワークは単一のエンベロープ、コレクション、ページネーション済みコレクション、スパースフィールドセット（`?fields[type]=...`）、複合的な `included` ドキュメント、そして多階層の `?include=a.b.c` チェーンを、すべて同じコードパスで処理する `IntoJsonResource` のimplを生成します。2つのファサード - `Resource` と `JsonApi` - は、2つの名前を持つ同じ型です。あなたのハウススタイルに合う方を使ってください。

## リソースを定義する

```rust
use suprnova::Data;

#[derive(Debug, Clone, Data)]
#[json_resource("users")]
pub struct UserResource {
    pub id: i64,
    pub email: String,

    // `input_only` は、`password` をフォームリクエスト側では使えるままにしつつ、
    // API出力からは除外する。
    #[data(input_only)]
    pub password: String,

    // フィールドを*リレーションシップ*としてマークする: `attributes` には決して
    // 現れず、代わりにJSON:APIのリレーションシップオブジェクトを生成し、
    // `?include=`の対象になる。フィールドの型は`IntoJsonResource`を
    // （直接、または`Vec<T>` / `Option<T>`経由で）実装していなければならない。
    #[data(allow_include)]
    pub posts: Vec<PostResource>,
}
```

`id_field` キーワードは、JSON:APIの `id` を提供するフィールドの名前を変更します:

```rust
#[derive(Data)]
#[json_resource("orders", id_field = "uuid")]
pub struct OrderResource {
    pub uuid: String,
    pub total_cents: i64,
}
```

## レスポンスを描画する

ハンドラから保留中のレスポンスを構築し、`.render().await` を呼んでください:

```rust
use suprnova::{LengthAwarePaginator, Resource};

#[handler]
async fn show_user(id: i64) -> Result<HttpResponse, FrameworkError> {
    let user: UserResource = User::find_or_fail(id).await?.into();
    Resource::single(user).render().await
}

#[handler]
async fn list_users() -> Result<HttpResponse, FrameworkError> {
    let users: Vec<UserResource> = User::all().await?.into_iter().map(Into::into).collect();
    Resource::collection(users).render().await
}

#[handler]
async fn paginate_users() -> Result<HttpResponse, FrameworkError> {
    // `paginate(per_page)` は、現在のリクエストから `?page=` を自動的に読み取る。
    let page = User::query().paginate(10).await?;
    // モデルのページネーターを、フィールドごとにリソースのページネーターへ変換する -
    // `data` は `pub` であり、残りのカウント/リンクはそのまま引き継がれる。
    let page = LengthAwarePaginator::new(
        page.data.into_iter().map(UserResource::from).collect(),
        page.total,
        page.per_page,
        page.current_page,
    )
    .with_base_url("/api/users");
    Resource::paginated(page).render().await
}
```

Laravel流の綴りを好むなら、`JsonApi::single` / `JsonApi::collection` / `JsonApi::paginated` が、まったく同じ働きをするエイリアスのエントリポイントです。

## 連鎖可能なミューテータ

`JsonApiResponse` は保留中のオブジェクトです。`.render().await` を呼ぶ前に、エンベロープをカスタマイズしてください。すべてのミューテータは `self` → `Self` なので、連鎖できます:

```rust
use suprnova::{Resource, JsonApiInfo};
use serde_json::json;

let info = JsonApiInfo::new()
    .with_version("1.1")
    .with_ext("https://jsonapi.org/ext/atomic")
    .with_meta("copyright", json!("2026 Acme Inc."));

Resource::single(user)
    .status(201)                                  // HTTPステータスの上書き
    .with_meta("trace_id", json!("req-7"))        // トップレベルのmeta KV
    .with_link("self", "/api/users/1")            // トップレベルのlink
    .with_jsonapi(info)                           // トップレベルの`jsonapi`
    .additional(json!({ "api_version": "2.0" }).as_object().unwrap().clone())
    .render()
    .await
```

| ミューテータ | Laravelでの対応 | 効果 |
|---|---|---|
| `.status(code)` | `ResourceResponse::calculateStatus` | HTTPステータスを上書きする。 |
| `.created()` | `wasRecentlyCreated → 201` | `.status(201)` の省略形。 |
| `.with_meta(k, v)` / `.meta(k, v)` | `with($request)` | トップレベルの `meta` KV。 |
| `.with_meta_map(m)` | `with($request)` の一括版 | マップをトップレベルの `meta` へマージする。 |
| `.with_link(rel, href)` / `.link(rel, href)` | `with($request)['links']` | トップレベルの `links` KV。 |
| `.with_link_value(rel, v)` | リンクオブジェクト形式 | トップレベルのlinkを `{href, meta}` として。 |
| `.with_additional(k, v)` | `additional($data)` | `data` と並ぶルートレベルのキー。 |
| `.additional(map)` | `additional($data)` | 追加キーの一括指定。 |
| `.with_jsonapi(info)` | `JsonApiResource::configure(...)` | トップレベルの `jsonapi` メンバー。 |

正規のメンバー（`data`、`included`、`links`、`meta`、`jsonapi`、`errors`）は、`.additional(...)` によって決して上書きされません。

## リソースごとの `links` と `meta`

`IntoJsonResource::resource_links` と `IntoJsonResource::resource_meta` のデフォルトをオーバーライドすると、ドキュメントのルートではなく*リソースオブジェクト*にリンク／メタデータを添付できます:

```rust
use suprnova::resources::IntoJsonResource;
use serde_json::{Map, Value};

impl IntoJsonResource for MyHandRolledPost {
    // ...

    fn resource_links(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("self".into(), Value::String(format!("/api/posts/{}", self.id)));
        m
    }

    fn resource_meta(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("kind".into(), Value::String("blog".into()));
        m
    }
}
```

マクロ導出のリソースでは、どちらもデフォルトで空の `Map` になるため、使われていないときはJSON:APIレンダラーがそのキーを省きます。`resource_top_level_meta` をオーバーライドすると、リソースごとのメタデータをエンベロープのトップレベルの `meta` メンバーへ引き上げられます。

## 条件付き属性 - `Maybe<T>` / `MissingValue<T>`

`Maybe` を使うと、実行時の条件に基づいて、描画される `attributes` オブジェクトからフィールドを省略できます。これは、Laravelの `MissingValue` と `when()` / `whenLoaded()` / `unless()` 系のSuprnovaにおける対応物です。

```rust
use suprnova::{Maybe, MissingValue};

// 両方の名前は同じ型を指す。
let m1: Maybe<&str> = Maybe::present("email@example.com");
let m2: MissingValue<&str> = MissingValue::missing();
let m3 = Maybe::when(user.is_verified, &user.verified_at);
let m4 = Maybe::unless(user.is_admin, &user.public_handle);
let m5 = Maybe::when_with(expensive_check(), || compute_value()); // レイジー
```

マクロ導出の構造体では、フィールドを `Maybe<T>` として宣言すれば、`Missing` のときにレンダラーが自動的にそれを落とします。手で書いた `resource_attributes` では、`insert_maybe(map, key, maybe)` ヘルパーを使ってください:

```rust
use suprnova::resources::{insert_maybe, Maybe};

fn resource_attributes(&self, _fs: Option<&[&str]>) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    insert_maybe(&mut map, "email", Maybe::present(&self.email));
    insert_maybe(
        &mut map,
        "phone",
        if self.show_phone { Maybe::present(&self.phone) } else { Maybe::missing() },
    );
    serde_json::Value::Object(map)
}
```

レンダラーは、attributesオブジェクト全体に対して `strip_missing_values(&mut value)` も呼び出します。そのため、任意のserde導出構造体の中にネストした `Maybe::Missing` の値も、再帰的に取り除かれます - 深くネストしたトランスフォーマーがサブフィールドを省略したい場合に便利です。

## スパースフィールドセット

フレームワークの `IncludeMiddleware` は、`?fields[type]=email,name` 形式のクエリパラメータをパースし、タスクローカルへ束縛します。マクロが発行する `resource_attributes` はそのフィールドセットを参照し、要求された属性だけを出力します。ハンドラ側の作業は不要です - ミドルウェアをインストールすれば、リソース層が自動的にそれを尊重します。

```rust
// リクエスト: GET /api/users/7?fields[users]=email
// レスポンス: { "data": { "type": "users", "id": "7", "attributes": { "email": "alice@example.com" } } }
```

## 複合ドキュメント - `?include=` チェーン

リレーションシップフィールドは `#[data(allow_include)]` で宣言してください。フレームワークは `?include=author.posts.tags,comments` から `IncludeTree` を構築し、すべてのノードを走査して、完全に解決されたリソースオブジェクトを `included` へプッシュします。重複排除は、JSON:API仕様の§8に従って `(type, id)` をキーとする `IncludedSink` を通じて、プッシュ時に実行されます - そのため、すべての項目が同じauthorを共有する1,000件のコレクションでも、authorはちょうど1回だけ解決されます。ピークのメモリとCPUは、リレーションシップのファンインではなく、重複を除いたincludedリソースの数に比例します。

```rust
#[derive(Data)]
#[json_resource("posts")]
pub struct PostResource {
    pub id: i64,
    pub title: String,

    #[data(allow_include)]
    pub author: Option<AuthorResource>,

    #[data(allow_include)]
    pub tags: Vec<TagResource>,
}
```

このリソースの許可リストに載っていないincludeパスを指定するリクエストは、JSON:APIの400 errorsエンベロープを受け取ります。

### 深さの上限

includeのパスが運べるのは、最大で5セグメントです。`?include=a.b.c.d.e.f` は、何かがそれを歩き始めるより前に `a.b.c.d.e` へ切り詰められ、Laravelの `JsonApiResource::$maxRelationshipDepth` に対応します。この天井は、起動時に一度だけ変更してください:

```rust
// bootstrap::register() にて
suprnova::max_relationship_depth(3);
```

この上限が重要なのは、リレーションのグラフが循環しうるからです: `?include=author.posts.author.posts...` は、クライアントが1セグメント打つごとに作業を増やし、それを境界づけるものは、クエリ文字列の長さのほかにありません。切り詰めはセグメントを取り除くだけで、決して追加せず、しかも各レベルは、降りる前に自分自身の許可リストを変わらず検査します - そのため、切り詰められたパスが、完全なパスでは届かなかったデータに届くことは決してありません。

知っておく価値のある帰結が1つあります: 上限を超えたセグメントは、許可リストがそれを目にするより前に落とされます。上限が2のとき、`?include=author.posts.secrets` は、完全なパスなら受け取るはずの400ではなく、`author` と `posts` をincludeした200を返します。何かがそれをバリデーションする時点では、`secrets` がもう存在しないからです。

`max_relationship_depth(0)` は、includeを完全に切ります。Laravelの0は、それでも最初の1ホップを出力します。Laravelのクランプは、先頭のセグメントが切り離された後の末尾にしか適用されないからです。Suprnovaの0は、リレーションがまったくないことを意味します。

### Suprnovaが異なる設計を選んだ理由

Laravelの `JsonApiResource` から、目に見える3つの相違点があります:

1. **`?include=` に対する厳格なデフォルト拒否。** Laravelのリソース層は、解決できないincludeパスをサイレントに無視します。Suprnovaは、JSON:APIのerrorsエンベロープを伴う `400 Bad Request` でそれらを拒否します。仕様の§5.2.2が定めるデフォルト拒否の姿勢は、クライアントがそれに対してプログラムを組める契約です - サイレントな無視は、クライアントのバグを隠し、複合ドキュメントの整合性を壊します。

2. **自動201の代わりに明示的な `.status(code)` / `.created()`。** Laravelは、背後のEloquentモデルの `wasRecentlyCreated` から `201` を自動的に設定します。SuprnovaはリソースのDTOを特定の永続化ライフサイクルから切り離しているため、ステータスはレスポンスオブジェクト自身に設定されます - 意図的に作成レスポンスを返したいときは `.created()` を、レスポンスが空のときは `.status(204)` を、といった具合です。1つのミューテータが、どんなフローの下でも正直なままです。

3. **深さの上限 `0` は、includeを完全に切ります。** Laravelがクランプするのは、先頭のセグメントが既に切り離された後の、パスの末尾だけであるため、その `0` は今も最初の1ホップを出力します。Suprnovaはパス全体を切り詰めるため、`max_relationship_depth(0)` はリレーションがまったくないことを意味します - 上の深さの上限を参照してください。
## ページネーション

`Resource::paginated(p)` は、`Paginated<T>` トレイトを実装するあらゆるページネーターと動作します - `suprnova::pagination` の `LengthAwarePaginator<T>` と `CursorPaginator<T>` は、どちらもこのimplを出荷しています。レンダラーは、`links.{self,first,prev,next,last}` と `meta.pagination` ブロックを自動的に添付します。

```rust
use suprnova::{LengthAwarePaginator, Resource};

let page = LengthAwarePaginator::new(items, total, per_page, current_page)
    .with_base_url("/api/users");
Resource::paginated(page).render().await
```

## エラーエンベロープ

すべての `FrameworkError` は、`into_json_api_response()` を通じて、自分自身をJSON:APIの `{"errors": [...]}` エンベロープとして描画する方法を知っています。このヘルパーが公開されているのは、`FrameworkError` がステータスコード、（`ValidationError` のための）フィールド名のソースポインタ、そして `meta.request_id` に入るリクエストIDの相関トークンを運ぶからです。5xxのレスポンスはサニタイズされます: 生のメッセージは、アクティブな環境で `APP_DEBUG=true` が設定されていない限りクライアントに届かず、設定されている場合は `meta.debug_message` の下に現れます。

```rust
let response = FrameworkError::validation("email", "email is invalid")
    .into_json_api_response();
// {
//   "errors": [{
//     "status": "422",
//     "title": "Validation failed",
//     "detail": "email is invalid",
//     "source": { "pointer": "/data/attributes/email" },
//     "meta": { "request_id": "..." }
//   }]
// }
```

## 表面のまとめ

| Suprnovaの表面 | Laravel 13での相当物 |
|---|---|
| `Resource` / `JsonApi` ファサード | `JsonResource::make`, `JsonApiResource` |
| `JsonApiResponse` | `ResourceResponse`, `JsonApiResource::toResponse` |
| `JsonApiBuilder` | （`ResourceResponse` のための内部ビルダー） |
| `IntoJsonResource` トレイト | `JsonResource::toArray`, `toAttributes`, `toRelationships`, `toLinks`, `toMeta`, `with` |
| `RelationshipValue` / `ResourceIdentifier` | `toRelationships` の内部にある配列の形 |
| `IncludeTree` | `JsonApiRequest` からパースされた `?include=` |
| `RequestFieldsetSet` | `JsonApiRequest` からパースされた `?fields[type]=` |
| `Maybe<T>` / `MissingValue<T>` | `MissingValue` + `whenLoaded` / `when` / `unless` |
| `JsonApiInfo` | `JsonApiResource::$jsonApiInformation` |
| `JsonApiResponse::status(code)` / `.created()` | `ResourceResponse::calculateStatus` |
| `JsonApiResponse::additional(map)` / `.with_additional(k, v)` | `JsonResource::additional($data)` |
| `JsonApiResponse::with_meta(k, v)` / `.meta(k, v)` | `JsonResource::with($request)['meta']` |
| `JsonApiResponse::with_link(rel, href)` / `.link(rel, href)` | `JsonResource::with($request)['links']` |
| `JsonApiResponse::with_jsonapi(info)` | `JsonApiResource::configure(...)` |
| `current_fieldset()` / `scope_fieldset(...)` | `IncludeMiddleware` が設定するタスクローカルなフィールドセット |
| `IncludeResolutionError` → 400エンベロープ | 厳格モードの `?include=` パーサー |

`suprnova::` の下でのトップレベルの再エクスポート: `Resource`、`JsonApi`、`JsonApiResponse`、`JsonApiBuilder`、`JsonApiInfo`、`IncludedSink`、`IntoJsonResource`、`RelationshipValue`、`ResourceIdentifier`、`IncludeTree`、`RequestFieldsetSet`、`Maybe`、`MissingValue`、`insert_maybe`、`strip_missing_values`、`AsRelationshipValue`、`PushIncluded`、`IncludeResolutionError`、`current_fieldset`、`scope_fieldset`。

## 次のステップ

- [Eloquent シリアライゼーション](eloquent-serialization.md) - `#[derive(Data)]`、hidden/visibleフィールド、リソースの属性に供給される `toArray` の相当物
- [Eloquent リレーションシップ](eloquent-relationships.md) - `#[data(allow_include)]` が消費するもの。複合ドキュメントを支える、型付けされたリレーションの種類
- [ページネーション](pagination.md) - `LengthAwarePaginator`、`CursorPaginator`、そして `Resource::paginated` が消費する `Paginated<T>` トレイト
- [データ オブジェクト](data.md) - Inertiaと共有される `#[derive(Data)]` マクロ、`?include=`/`?fields[type]=` ミドルウェア、そして `Maybe<T>` のパターン
- [エラー モデル](error-model.md) - `FrameworkError::into_json_api_response` が、変換の契約にどう適合するか
