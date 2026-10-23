# Inertia で Todo アプリを構築する

スタック全体を運動させる、Suprnovaのバーティカルスライスです - マイグレーション、`#[suprnova::model]`、InertiaがレンダリングするSvelte 5のページ、ルートモデル結合、フォームバリデーション、そして`routes.rs`から生成される型安全なルートヘルパーです。これを一度通してやってみれば、マイグレーション → モデル → コントローラー → ルート → ページというプロジェクトのループが、体に染みつきます。

これは、[インストール](installation.md)にすでに従っていて、`suprnova` CLIが`PATH`に存在することを前提としています。スキャフォルダーはデフォルトでSvelte 5を使い、このチュートリアルもそれを使います。

## 構築するもの

作成、一覧、完了の切り替え、編集、削除を備えたtodoページです。別個のJSON APIはありません - Inertiaがプロップをシリアライズし、Svelteページはそれを`$props()`として消費します。同じ構造体が、Rustからブラウザまでそのまま流れます。

## 1. スキャフォルド

```bash
suprnova new todo-app --frontend svelte --no-interaction
cd todo-app
npm install
```

## 2. マイグレーション

```bash
suprnova make:migration create_todos_table
```

`src/migrations/`の下の新しいマイグレーションを開きます。

```rust
use suprnova::sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .create_table(
                Table::create()
                    .table(Alias::new("todos"))
                    .if_not_exists()
                    .col(ColumnDef::new(Alias::new("id"))
                        .big_integer().primary_key().auto_increment().not_null())
                    .col(ColumnDef::new(Alias::new("title")).string().not_null())
                    .col(ColumnDef::new(Alias::new("completed"))
                        .boolean().not_null().default(false))
                    .col(ColumnDef::new(Alias::new("created_at"))
                        .timestamp_with_time_zone().not_null()
                        .default(Expr::current_timestamp()))
                    .col(ColumnDef::new(Alias::new("updated_at"))
                        .timestamp_with_time_zone().not_null()
                        .default(Expr::current_timestamp()))
                    .to_owned(),
            )
            .await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager
            .drop_table(Table::drop().table(Alias::new("todos")).to_owned())
            .await
    }
}
```

`created_at`と`updated_at`の両方が存在するのは、次のステップのモデルが`timestamps`を使うためです - これは両方のカラムを想定し、自動的に管理します。次に、マイグレーションを実行し、エンティティを再生成します。

```bash
suprnova db:sync
```

`db:sync`は保留中のマイグレーションを実行し、`#[suprnova::model]`マクロが依拠しているSeaORMのエンティティ層を更新します。

## 3. モデル

`src/models/todo.rs`を作成します。

```rust
use chrono::{DateTime, Utc};
use suprnova::model;

#[model(
    table = "todos",
    fillable = ["title", "completed"],
    timestamps,
)]
pub struct Todo {
    pub id: i64,
    pub title: String,
    pub completed: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

// モデルマクロは、SeaORMのEntity、ActiveModel、Column、Model型を持つ
// 内部の`todo`モジュールを生成する。ファイルの外から手を伸ばしたい
// ものを再エクスポートする。
pub use todo::{ActiveModel, Column, Entity};
```

新しいモジュールを`src/models/mod.rs`に配線します。

```rust
pub mod todo;
```

`fillable`のリストはマスアサインメントを制限します。`timestamps`は、保存ごとに`created_at` / `updated_at`を自動的に管理します。ユーザー向けの`Todo`構造体は、ハンドラの中で扱う型です。内部の`todo::Model`は、ルートモデル結合が取得するSeaORMの形です。

## 4. コントローラー

```bash
suprnova make:controller todo
```

`src/controllers/todo.rs`を開きます。

```rust
use suprnova::{
    attrs, handler, inertia_response, redirect_to, request, InertiaProps,
    Model, Request, Response,
};

use crate::models::todo::{todo, Todo};

#[derive(InertiaProps)]
pub struct TodoIndexProps {
    pub todos: Vec<Todo>,
}

#[derive(InertiaProps)]
pub struct TodoFormProps {
    pub todo: Option<Todo>,
}

#[request]
pub struct TodoForm {
    #[validate(length(min = 1, max = 200, message = "Title is required"))]
    pub title: String,
}

#[handler]
pub async fn index(_req: Request) -> Response {
    let todos = Todo::all().await?.into_vec();
    inertia_response!("Todos/Index", TodoIndexProps { todos })
}

#[handler]
pub async fn create(_req: Request) -> Response {
    inertia_response!("Todos/Create", TodoFormProps { todo: None })
}

#[handler]
pub async fn store(form: TodoForm) -> Response {
    Todo::create(attrs! {
        title: form.title,
        completed: false,
    })
    .await?;
    redirect_to("/todos").into()
}

#[handler]
pub async fn edit(todo: todo::Model) -> Response {
    let todo: Todo = todo.into();
    inertia_response!("Todos/Edit", TodoFormProps { todo: Some(todo) })
}

#[handler]
pub async fn update(todo: todo::Model, form: TodoForm) -> Response {
    let todo: Todo = todo.into();
    todo.update(attrs! { title: form.title }).await?;
    redirect_to("/todos").into()
}

#[handler]
pub async fn toggle(todo: todo::Model) -> Response {
    let todo: Todo = todo.into();
    let next = !todo.completed;
    todo.update(attrs! { completed: next }).await?;
    redirect_to("/todos").into()
}

#[handler]
pub async fn destroy(todo: todo::Model) -> Response {
    let todo: Todo = todo.into();
    todo.delete().await?;
    redirect_to("/todos").into()
}
```

いくつか、注目しておくべき点があります。

- **ルートモデル結合は自動です。** `todo: todo::Model`と宣言することで、`#[handler]`マクロはルートパスの中の`{todo}`を探し、主キーでSeaORMの行を取得し、見つからなければ404を返すようになります。パラメータ名は、ルートのプレースホルダーと一致していなければなりません。
- **マクロが手渡すのは`todo::Model`です。Eloquentの表面は`Todo`の上に存在します。** 両者は、`#[suprnova::model]`が生成する`From`のimplで橋渡しされています。そのため、`let todo: Todo = todo.into();`が1行での変換になります。`Todo`は、`update`、`delete`、そしてその他のユーザー向けAPIを備える型です。
- **`#[request]`がバリデーションをカバーします。** これを構造体に追加すると、`Deserialize`、`Validate`、`FormRequest`が生成されます - フレームワークは、ハンドラが実行される前に、不正な形式の入力を422で拒否します。リクエストDTOに`InertiaProps`も一緒にderiveする必要はありません - そのderiveは、*送り出す*ページプロップのためのものです。
- **マスアサインメントは`attrs!`を通じて行われます。** `Todo::create(attrs! { ... })`と`todo.update(attrs! { ... })`は、fillableのフィルタを経由します。そのため、モデルの`fillable`リストにないフィールドは、保護をすり抜けるのではなく、黙って落とされます。
- **`update`と`delete`は`self`を消費します。** そのため、`toggle`は`todo.update(...)`を呼ぶ前に、`!todo.completed`をローカル変数へ読み込んでいます。

新しいコントローラーモジュールを`src/controllers/mod.rs`に登録します。

```rust
pub mod todo;
```

### Suprnovaが異なる設計を選んだ理由

Laravelでは、同じコントローラーが通常、APIのためにJSONを返すか、サーバーレンダリングされるページのためにBladeビューを返すか、どちらかになります。Suprnovaは、初回ロードとSPAナビゲーションの両方に対して、Inertiaのレスポンスを返します - フレームワークは`X-Inertia`ヘッダーを検出し、それに応じてHTMLかJSONを配信します。並行するAPI層はありません。ハンドラを一度書くだけで、フロントエンドは本物のSPAのままであり、同期させておくべき2つ目のルーターも存在しません。仕組みについては[Inertia レスポンス](frontend-inertia-responses.md)を参照してください。

## 5. ルート

`src/routes.rs`:

```rust
use suprnova::{delete, get, post, put, routes};

use crate::controllers::todo;

routes! {
    get!("/todos", todo::index).name("todos.index"),
    get!("/todos/create", todo::create).name("todos.create"),
    post!("/todos", todo::store).name("todos.store"),
    get!("/todos/{todo}/edit", todo::edit).name("todos.edit"),
    put!("/todos/{todo}", todo::update).name("todos.update"),
    post!("/todos/{todo}/toggle", todo::toggle).name("todos.toggle"),
    delete!("/todos/{todo}", todo::destroy).name("todos.destroy"),
}
```

`{todo}`というプレースホルダーは、ルートモデル結合が引っかける対象です - これは、ハンドラのパラメータ名（`todo`）と一致していなければならず、SeaORMモデルの主キーの型（ここでは`i64`）とも一致していなければなりません。任意の`.name(...)`という接尾辞は、次のステップのルート型ジェネレーターが、フロントエンドのヘルパーを組み立てるために使うものです。

## 6. TypeScript 型を生成する

```bash
suprnova generate-types
```

`generate-types`は、1回のパスで2つのことを行います。

1. `src/`にあるすべての`#[derive(InertiaProps)]`構造体を走査し、それらを`frontend/src/types/inertia-props.ts`に書き出します。
2. `src/routes.rs`を走査し、名前付きのルートごとに型付きのURLビルダーを`frontend/src/types/routes.ts`に書き出します。

ルートヘルパーは、入れ子になったオブジェクトとして出てきます - `controllers.todos.toggle({ todo: "1" })`は、Inertia 3の`Link`と`router`が直接受け付ける`{ url, method }`のペアを返します。パスパラメータは型付けされているため、コンパイラは、ページがブラウザに届く前に、欠けている`todo`引数を捕まえます。

これらのファイルを編集する必要はありません。プロップやルートを追加・改名するたびに`suprnova generate-types`を再実行してください。あるいは`--watch`を渡して、作業しながら同期させ続けてください。

## 7. ページ

各ページは、`frontend/src/pages/Todos/`の下に存在します。その名前は、`inertia_response!`に渡す文字列と一致します - そのため、`inertia_response!("Todos/Index", ...)`は`frontend/src/pages/Todos/Index.svelte`へ解決されます。

### 一覧

`frontend/src/pages/Todos/Index.svelte`:

```svelte
<script lang="ts">
  import { Link, router } from '@inertiajs/svelte'
  import type { Todo, TodoIndexProps } from '../../types/inertia-props'
  import { controllers } from '../../types/routes'

  let { todos }: TodoIndexProps = $props()

  function toggle(todo: Todo) {
    router.visit(controllers.todos.toggle({ todo: String(todo.id) }))
  }

  function remove(todo: Todo) {
    if (confirm('Delete this todo?')) {
      router.visit(controllers.todos.destroy({ todo: String(todo.id) }))
    }
  }
</script>

<div class="mx-auto max-w-2xl p-8">
  <div class="mb-6 flex items-center justify-between">
    <h1 class="text-2xl font-bold">My Todos</h1>
    <Link
      href={controllers.todos.create()}
      class="rounded bg-blue-600 px-4 py-2 text-white hover:bg-blue-700"
    >
      Add todo
    </Link>
  </div>

  {#if todos.length === 0}
    <p class="text-center text-gray-500">No todos yet.</p>
  {:else}
    <ul class="space-y-2">
      {#each todos as todo (todo.id)}
        <li class="flex items-center gap-3 rounded border p-3">
          <input
            type="checkbox"
            checked={todo.completed}
            onchange={() => toggle(todo)}
            class="h-5 w-5"
          />
          <span class={todo.completed ? 'flex-1 text-gray-400 line-through' : 'flex-1'}>
            {todo.title}
          </span>
          <Link
            href={controllers.todos.edit({ todo: String(todo.id) })}
            class="text-blue-600 hover:underline"
          >
            Edit
          </Link>
          <button
            onclick={() => remove(todo)}
            class="text-red-600 hover:underline"
          >
            Delete
          </button>
        </li>
      {/each}
    </ul>
  {/if}
</div>
```

### 作成

`frontend/src/pages/Todos/Create.svelte`:

```svelte
<script lang="ts">
  import { Link, useForm } from '@inertiajs/svelte'
  import { controllers } from '../../types/routes'

  const form = useForm({ title: '' })

  function submit(e: SubmitEvent) {
    e.preventDefault()
    form.post(controllers.todos.store().url)
  }
</script>

<div class="mx-auto max-w-md p-8">
  <h1 class="mb-6 text-2xl font-bold">Create todo</h1>

  <form onsubmit={submit} class="space-y-4">
    <div>
      <label for="title" class="mb-1 block text-sm font-medium">Title</label>
      <input
        id="title"
        type="text"
        bind:value={form.title}
        class="w-full rounded border px-3 py-2"
        placeholder="What needs to be done?"
      />
      {#if form.errors?.title}
        <p class="mt-1 text-sm text-red-600">{form.errors.title}</p>
      {/if}
    </div>

    <div class="flex gap-3">
      <button
        type="submit"
        disabled={form.processing}
        class="rounded bg-blue-600 px-4 py-2 text-white hover:bg-blue-700 disabled:opacity-50"
      >
        {form.processing ? 'Creating...' : 'Create'}
      </button>
      <Link
        href={controllers.todos.index()}
        class="px-4 py-2 text-gray-600 hover:underline"
      >
        Cancel
      </Link>
    </div>
  </form>
</div>
```

### 編集

`frontend/src/pages/Todos/Edit.svelte`:

```svelte
<script lang="ts">
  import { Link, useForm } from '@inertiajs/svelte'
  import type { TodoFormProps } from '../../types/inertia-props'
  import { controllers } from '../../types/routes'

  const props: TodoFormProps = $props()
  const todo = props.todo!

  const form = useForm({ title: todo.title })

  function submit(e: SubmitEvent) {
    e.preventDefault()
    form.put(controllers.todos.update({ todo: String(todo.id) }).url)
  }
</script>

<div class="mx-auto max-w-md p-8">
  <h1 class="mb-6 text-2xl font-bold">Edit todo</h1>

  <form onsubmit={submit} class="space-y-4">
    <div>
      <label for="title" class="mb-1 block text-sm font-medium">Title</label>
      <input
        id="title"
        type="text"
        bind:value={form.title}
        class="w-full rounded border px-3 py-2"
      />
      {#if form.errors?.title}
        <p class="mt-1 text-sm text-red-600">{form.errors.title}</p>
      {/if}
    </div>

    <div class="flex gap-3">
      <button
        type="submit"
        disabled={form.processing}
        class="rounded bg-blue-600 px-4 py-2 text-white hover:bg-blue-700 disabled:opacity-50"
      >
        {form.processing ? 'Saving...' : 'Save'}
      </button>
      <Link
        href={controllers.todos.index()}
        class="px-4 py-2 text-gray-600 hover:underline"
      >
        Cancel
      </Link>
    </div>
  </form>
</div>
```

同等のReact 19とVue 3.5のスターターも、それぞれ自分自身のテンプレート機構を通じて同じプロップを受け取ります - バックエンドは変わりません。

## 8. 実行する

```bash
suprnova serve
```

`http://127.0.0.1:8765/todos`にアクセスし、いくつか行を追加し、それらを切り替え、1つを編集し、もう1つを削除してみます。ページの遷移はInertiaを通じて行われます - フルリロードはありません - そして、あらゆるフォーム送信は、リダイレクトが着地する前にサーバー側でバリデーションされます。

## 何が起きたのか

| レイヤー | ファイル | 何をするか |
|---|---|---|
| スキーマ | `src/migrations/m_create_todos_table.rs` | `todos`テーブルを作成する |
| モデル | `src/models/todo.rs` | ユーザー向けの`Todo`構造体 + 内部のSeaORMモジュール |
| HTTP | `src/controllers/todo.rs` | ルートモデル結合を含む、7つの`#[handler]` |
| ルーター | `src/routes.rs` | 生成されるルートヘルパーを駆動する、名前付きルート |
| プロップ | `frontend/src/types/inertia-props.ts` | `#[derive(InertiaProps)]`から生成される |
| ルート | `frontend/src/types/routes.ts` | `routes.rs`の名前付きルートから生成される |
| ページ | `frontend/src/pages/Todos/*.svelte` | プロップを消費する、3つのSvelte 5ページ |

これが、Suprnovaの標準的な機能ループです - マイグレーション → モデル → コントローラー → ルート → ページであり、プロップを作り直したりルートを改名したりするたびに、`suprnova generate-types`がTypeScriptのブリッジを再生成します。

## 次のステップ

- [Eloquent](eloquent.md) - `attrs!`、クエリビルダー、キャスト、スコープ、オブザーバー
- [バリデーション](validation.md) - `#[request]`と`#[derive(Validate)]`が与えてくれるもの
- [ルーティング](routing.md) - 名前付きルート、ルートモデル結合、リソースルーティング、署名付きURL
- [Inertia レスポンス](frontend-inertia-responses.md) - `inertia_response!`、部分的なリロード、共有プロップ
- [認証](authentication.md) - スターターのセッション認証を使って、ユーザーごとのtodoを追加すること
