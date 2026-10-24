# Todo JSON:API を構築する

APIパスをエンドツーエンドで辿るウォークスルーです - マイグレーション、モデル、バリデーション済みのフォームリクエスト、ルートモデル結合、JSON:APIのリソースエンベロープ、スパースフィールドセット、ページネーションです。最後には、`?include=`と`?fields[todos]=...`が自動的に尊重される、仕様に準拠した[JSON:API](https://jsonapi.org/)レスポンスを返す、5つのエンドポイントを持つtodoサービスが手元にあります。

構築するもの:

| メソッド | ルート | アクション |
|----------|----------------------|---------|
| `GET`    | `/api/todos`         | 一覧（ページネーション済み） |
| `GET`    | `/api/todos/{todo}`  | 詳細 |
| `POST`   | `/api/todos`         | 作成 |
| `PUT`    | `/api/todos/{todo}`  | 更新 |
| `DELETE` | `/api/todos/{todo}`  | 削除 |

## 前提条件

スキャフォルドされたプロジェクト:

```bash
suprnova new todo-api
cd todo-api
```

## ステップ1: マイグレーション

```bash
suprnova make:migration create_todos_table
```

これは、`src/migrations/m<timestamp>_create_todos_table.rs`を書き出します。本体を`todos`のスキーマに置き換えます。

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Todos::Table)
                    .if_not_exists()
                    .col(
                        ColumnDef::new(Todos::Id)
                            .big_integer()
                            .not_null()
                            .auto_increment()
                            .primary_key(),
                    )
                    .col(ColumnDef::new(Todos::Title).string().not_null())
                    .col(ColumnDef::new(Todos::Description).text().null())
                    .col(
                        ColumnDef::new(Todos::Done)
                            .boolean()
                            .not_null()
                            .default(false),
                    )
                    .col(
                        ColumnDef::new(Todos::CreatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .col(
                        ColumnDef::new(Todos::UpdatedAt)
                            .timestamp_with_time_zone()
                            .not_null()
                            .default(Expr::current_timestamp()),
                    )
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Todos::Table).to_owned())
            .await
    }
}

#[derive(DeriveIden)]
enum Todos {
    Table,
    Id,
    Title,
    Description,
    Done,
    CreatedAt,
    UpdatedAt,
}
```

実行します:

```bash
suprnova migrate
```

`down`の本体があることで、後から`migrate:rollback`がこの変更を元に戻せます。

## ステップ2: モデル

`#[suprnova::model]`構造体は、Eloquentモデルそのものです - このマクロは、内部モジュールの中にSeaORMの`Entity`、`Column`、`ActiveModel`を生成し、構造体にクエリ表面（`Todo::query()`、`Todo::find`、`Todo::create`、`model.update`、`model.delete`、自動管理されるタイムスタンプ、ライフサイクルイベント）を与えます。`src/models/todo.rs`を作成します。

```rust
use chrono::{DateTime, Utc};
use suprnova::model;

#[model(
    table = "todos",
    fillable = ["title", "description", "done"],
    timestamps,
)]
pub struct Todo {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub done: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// マクロが内部の`todo`モジュールに生成するSeaORMの型を再エクスポートし、
// 呼び出し側がマクロの内部を触らずにそれらへ手を伸ばせるようにする。
pub use todo::{ActiveModel, Column, Entity};
```

モジュールを`src/models/mod.rs`に配線します。

```rust
pub mod todo;
```

`fillable`のリストは、マスアサインメントの許可リストです - `Todo::create(attrs!{...})`と`model.update(attrs!{...})`を通じて設定できるのは、それらのフィールドだけです。リストの外にあるフィールドは、リクエストの入力からの不用意な書き込みから保護されます。

## ステップ3: フォームリクエスト

バリデーションは、`#[request]`構造体の上に存在します。`extract()`は、ハンドラの本体がその値を目にする前にバリデータを実行します。失敗すると、Laravel/Inertia形式のエラーバッグを伴う422へショートサーキットします。`src/requests.rs`を作成します。

```rust
use suprnova::request;

#[request]
pub struct CreateTodoRequest {
    #[validate(length(min = 1, max = 255, message = "title is required"))]
    pub title: String,

    #[validate(length(max = 1000))]
    pub description: Option<String>,
}

#[request]
pub struct UpdateTodoRequest {
    #[validate(length(min = 1, max = 255))]
    pub title: Option<String>,

    #[validate(length(max = 1000))]
    pub description: Option<String>,

    pub done: Option<bool>,
}
```

そして、それを`src/lib.rs`に登録します。

```rust
pub mod requests;
```

`#[request]`アトリビュートは、`#[derive(serde::Deserialize, validator::Validate)] + impl FormRequest`と同等のものへ展開されるため、構造体のフィールドは入力スキーマも兼ねます。オプションのフィールド（`Option<T>`）は、部分的な更新にふさわしい形です - JSONボディの中で欠けているキーは`None`へデシリアライズされ、ハンドラは`None`を「このカラムは変更しない」という意味に扱います。

## ステップ4: JSON:APIリソース

リソースとは、`#[json_resource("type")]`を持つ`#[derive(Data)]`構造体です。このマクロは、`Resource::single`、`Resource::collection`、`Resource::paginated`が消費する`IntoJsonResource`のimplを生成します。リソースのフィールドは、JSON:APIの`attributes`オブジェクトになります - あらゆるスパースフィールドセットのフィルタと`?include=`チェーンは、この型を通じてディスパッチされます。`src/resources/todo_resource.rs`を作成します。

```rust
use crate::models::todo::Todo;
use suprnova::Data;
use validator::Validate;

#[derive(Debug, Clone, Data, Validate)]
#[json_resource("todos")]
pub struct TodoResource {
    pub id: i64,
    pub title: String,
    pub description: Option<String>,
    pub done: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Todo> for TodoResource {
    fn from(t: Todo) -> Self {
        Self {
            id: t.id,
            title: t.title,
            description: t.description,
            done: t.done,
            created_at: t.created_at.to_rfc3339(),
            updated_at: t.updated_at.to_rfc3339(),
        }
    }
}
```

`src/resources/mod.rs`に配線します。

```rust
pub mod todo_resource;
```

そして、`src/lib.rs`でこのモジュールを再宣言します。

```rust
pub mod resources;
```

`id`フィールドは、JSON:APIの`id`メンバーを供給します（仕様に従って文字列化されます）。それ以外のすべてのフィールドは`attributes`に収まり、スパースフィールドセットのフィルタリングの対象になります - `?fields[todos]=title,done`を指定するリクエストは、ハンドラ側での作業を一切必要とせずに、その2つの属性だけを受け取ります。

## ステップ5: コントローラー

`#[handler]`アトリビュートは、各パラメータを分類し、対応するエクストラクターを生成します。

- `i64` - `FromParam`が、同じ名前のルートパラメータをパースします。不正な入力（`/api/todos/abc`）は、400へショートサーキットします。
- `CreateTodoRequest` / `UpdateTodoRequest` - `FromRequest`がボディをデシリアライズし、バリデーションを実行し、失敗すれば422を返します。
- `Request` - そのまま素通りします。

行の読み込みは、Eloquentの表面を通じて行われます - `Todo::find_or_fail(id)`は、一致する行がなければ404を返します。

`src/controllers/todos.rs`を作成します。

```rust
use crate::models::todo::Todo;
use crate::requests::{CreateTodoRequest, UpdateTodoRequest};
use crate::resources::todo_resource::TodoResource;
use suprnova::{
    attrs, handler, LengthAwarePaginator, Model, Resource, Response,
};

// GET /api/todos?page=2
#[handler]
pub async fn index() -> Response {
    let page = Todo::query()
        .order_by_desc("created_at")
        .paginate(20)
        .await?;
    // ページネーターを`TodoResource`で再構成し、JSON:APIレンダラーが
    // 生のモデルではなくリソースオブジェクトを見るようにする。ページネーションの
    // ウィンドウ（`total`、`per_page`、`current_page`）は保持される。
    let total = page.total;
    let per_page = page.per_page;
    let current_page = page.current_page;
    let resources: Vec<TodoResource> =
        page.data.into_iter().map(TodoResource::from).collect();
    let paginator = LengthAwarePaginator::new(resources, total, per_page, current_page)
        .with_path("/api/todos");
    Resource::paginated(paginator).render().await
}

// GET /api/todos/{todo}
#[handler]
pub async fn show(todo: i64) -> Response {
    let todo = Todo::find_or_fail(todo).await?;
    Resource::single(TodoResource::from(todo)).render().await
}

// POST /api/todos
#[handler]
pub async fn store(form: CreateTodoRequest) -> Response {
    let todo = Todo::create(attrs! {
        title: form.title,
        description: form.description,
        done: false,
    })
    .await?;
    Resource::single(TodoResource::from(todo))
        .created()           // 201
        .render()
        .await
}

// PUT /api/todos/{todo}
#[handler]
pub async fn update(todo: i64, form: UpdateTodoRequest) -> Response {
    let row = Todo::find_or_fail(todo).await?;

    let mut changes = attrs!();
    if let Some(title) = form.title {
        changes.insert("title", title.into());
    }
    if let Some(description) = form.description {
        changes.insert("description", description.into());
    }
    if let Some(done) = form.done {
        changes.insert("done", done.into());
    }
    let updated = row.update(changes).await?;
    Resource::single(TodoResource::from(updated)).render().await
}

// DELETE /api/todos/{todo}
#[handler]
pub async fn destroy(todo: i64) -> Response {
    Todo::find_or_fail(todo).await?.delete().await?;
    suprnova::json_response!({ "deleted": true })
}
```

`src/controllers/mod.rs`に配線します。

```rust
pub mod todos;
```

引数名は、ルートのプレースホルダーと一致していなければなりません - `{todo}`は`todo: i64`に対応します。マクロは`FromParam`を介してパスセグメントをパースし、その後ハンドラの本体がEloquentの表面を駆動して、行の読み込み、更新、削除を行います。

## ステップ6: ルート

`src/routes.rs`:

```rust
use crate::controllers::todos;
use suprnova::{delete, get, post, put, routes};

routes! {
    get!("/api/todos",           todos::index   ).name("todos.index"),
    get!("/api/todos/{todo}",    todos::show    ).name("todos.show"),
    post!("/api/todos",          todos::store   ).name("todos.store"),
    put!("/api/todos/{todo}",    todos::update  ).name("todos.update"),
    delete!("/api/todos/{todo}", todos::destroy ).name("todos.destroy"),
}
```

`routes!`マクロは、起動時に`Application::routes(...)`が消費する、設定済みの`Router`を返します。

## ステップ7: 実行する

```bash
suprnova serve --backend-only
```

### 作成

```bash
curl -X POST http://localhost:8765/api/todos \
  -H "Content-Type: application/json" \
  -d '{"title": "Read JSON:API spec", "description": "All of it"}'
```

```json
{
  "data": {
    "type": "todos",
    "id": "1",
    "attributes": {
      "title": "Read JSON:API spec",
      "description": "All of it",
      "done": false,
      "created_at": "2026-05-30T12:00:00+00:00",
      "updated_at": "2026-05-30T12:00:00+00:00"
    }
  }
}
```

### 一覧（ページネーション済み）

```bash
curl http://localhost:8765/api/todos
```

```json
{
  "data": [
    { "type": "todos", "id": "1", "attributes": { … } }
  ],
  "meta": {
    "pagination": {
      "total": 1,
      "per_page": 20,
      "current_page": 1,
      "last_page": 1
    }
  },
  "links": {
    "first": "?page=1",
    "last":  "?page=1",
    "prev":  null,
    "next":  null
  }
}
```

### スパースフィールドセット

```bash
curl 'http://localhost:8765/api/todos/1?fields[todos]=title,done'
```

```json
{
  "data": {
    "type": "todos",
    "id": "1",
    "attributes": { "title": "Read JSON:API spec", "done": false }
  }
}
```

`IncludeMiddleware`が`?fields[type]=...`をパースし、そのフィルタをタスクローカルへ結びつけます。`Resource::single`は、レンダリングの間にそれを読み取ります - ハンドラは、そのクエリパラメータを一切目にしません。

### 更新

```bash
curl -X PUT http://localhost:8765/api/todos/1 \
  -H "Content-Type: application/json" \
  -d '{"done": true}'
```

部分的なボディが機能するのは、`UpdateTodoRequest`のすべてのフィールドが`Option<T>`だからです - ハンドラは、届いたキーだけを書き込みます。

### 削除

```bash
curl -X DELETE http://localhost:8765/api/todos/1
# {"deleted": true}
```

### バリデーション失敗

```bash
curl -X POST http://localhost:8765/api/todos \
  -H "Content-Type: application/json" \
  -d '{"title": ""}'
```

```json
{
  "message": "The given data was invalid.",
  "errors": { "title": ["title is required"] },
  "request_id": "8f9e1a2b-…"
}
```

Laravel/Inertia形式のエラーバッグを伴う422です - ハンドラの本体は実行されませんでした。

## 各要素の実装場所

| ファイル | 役割 |
|------|------|
| `src/migrations/m*_create_todos_table.rs` | スキーマ |
| `src/models/todo.rs` | `#[suprnova::model]`構造体 |
| `src/requests.rs` | `#[request]`のフォームリクエスト、`extract()`によってバリデーションされる |
| `src/resources/todo_resource.rs` | `#[derive(Data)]` + `#[json_resource("todos")]` |
| `src/controllers/todos.rs` | `#[handler]`関数 |
| `src/routes.rs` | `routes!`の登録 |

## 次のステップ

- [Eloquent](eloquent.md) - Modelの表面全体、クエリビルダー、`attrs!`、ライフサイクルイベント、ソフトデリート、リレーションシップ
- [バリデーション](validation.md) - `#[request]`、`validate!`、`Unique`、非同期フック、フィールド横断のルール
- [JSON:API リソース](eloquent-resources.md) - `?include=`チェーン、リソースごとのlinks/meta、`Maybe<T>`の条件付き属性
- [フォームリクエスト](requests.md) - `FormRequest`トレイト、コンテンツタイプによるディスパッチ、`authorize(&Request)`
- [コントローラー](controllers.md) - `#[handler]`が抽出するもの、そしてルートモデル結合が内部でどのように働くか
