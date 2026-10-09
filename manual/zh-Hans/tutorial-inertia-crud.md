# 用 Inertia 构建待办事项应用

一个贯穿 Suprnova 全栈的垂直切片：一次迁移、一个 `#[suprnova::model]`、由 Inertia 渲染的 Svelte 5 页面、路由模型绑定、表单验证，以及从 `routes.rs` 生成出来的类型安全路由辅助函数。把这套流程完整走一遍，迁移、模型、控制器、路由、页面这个项目循环，就会变成您的肌肉记忆。

这假定您已经跟着[安装](installation.md)操作过，并且您的 `PATH` 上已经有 `suprnova` 这个 CLI。脚手架工具默认使用 Svelte 5，这也正是本教程所用的。

## 您将构建的东西

一个带有创建、列表、切换完成状态、编辑和删除功能的待办事项页面。没有单独的 JSON API：Inertia 序列化 props，Svelte 页面把它们当作 `$props()` 来消费 - 同一个结构体从 Rust 一路流到浏览器。

## 1. 脚手架

```bash
suprnova new todo-app --frontend svelte --no-interaction
cd todo-app
npm install
```

## 2. 迁移

```bash
suprnova make:migration create_todos_table
```

打开 `src/migrations/` 下这个新的迁移文件：

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

`created_at` 和 `updated_at` 两者都在，是因为下一步里的这个模型用了 `timestamps`，它需要这两个列，并会自动管理它们。接下来运行迁移，并重新生成实体：

```bash
suprnova db:sync
```

`db:sync` 会运行所有待处理的迁移，并刷新 `#[suprnova::model]` 这个宏所依赖的那个 SeaORM 实体层。

## 3. 模型

创建 `src/models/todo.rs`：

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

// 这个 model 宏会发出一个内部的 `todo` 模块，带着 SeaORM 的
// Entity、ActiveModel、Column 和 Model 类型。把您想从这个文件
// 外部拿到的那些类型重新导出。
pub use todo::{ActiveModel, Column, Entity};
```

把这个新模块接到 `src/models/mod.rs` 里：

```rust
pub mod todo;
```

这个 `fillable` 列表管控着批量赋值；`timestamps` 会在每次保存时自动管理 `created_at` / `updated_at`。这个面向用户的 `Todo` 结构体，就是您在处理程序里会用到的那个类型；内部的 `todo::Model` 则是路由模型绑定获取的那个 SeaORM 形态。

## 4. 控制器

```bash
suprnova make:controller todo
```

打开 `src/controllers/todo.rs`：

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

有几件事值得注意：

- **路由模型绑定是自动的。** 声明 `todo: todo::Model` 会告诉 `#[handler]` 这个宏，去路由路径里查找 `{todo}`，按主键取出这一行 SeaORM 记录，如果它缺失就返回 404。参数名必须和路由占位符一致。
- **这个宏交给您的是 `todo::Model`；Eloquent 接口活在 `Todo` 上。** 两者由 `#[suprnova::model]` 发出的一个 `From` 实现桥接起来，所以 `let todo: Todo = todo.into();` 就是那一行转换。`Todo` 是携带着 `update`、`delete` 以及其它面向用户 API 的那个类型。
- **`#[request]` 覆盖了验证。** 把它加到一个结构体上，会生成 `Deserialize`、`Validate` 和 `FormRequest` - 框架会在您的处理程序运行之前，用一个 422 拒绝掉格式错误的输入。不需要在一个请求 DTO 上也派生 `InertiaProps`；那个派生是给*出站*的页面 props 用的。
- **批量赋值要经过 `attrs!`。** `Todo::create(attrs! { ... })` 和 `todo.update(attrs! { ... })` 都会经过这个 fillable 过滤器，所以不在模型 `fillable` 列表里的字段，会被静默丢弃，而不是绕过这道防护。
- **`update` 和 `delete` 会消费 `self`。** 这就是为什么 `toggle` 会先把 `!todo.completed` 读进一个局部变量，再去调用 `todo.update(...)`。

把这个新的控制器模块注册到 `src/controllers/mod.rs` 里：

```rust
pub mod todo;
```

### 为什么 Suprnova 有所不同

在 Laravel 里，同一个控制器通常会为一个 API 返回 JSON，或者为一个服务器渲染的页面返回一个 Blade 视图。Suprnova 对首次加载和 SPA 导航都返回 Inertia 响应 - 框架会检测 `X-Inertia` 请求头，并相应地提供 HTML 或者 JSON，不需要一层并行的 API。您只需要写一次处理程序，您的前端也依然是一个真正的 SPA，也不需要再维护第二个路由器让它保持同步。具体机制请参见[Inertia 响应](frontend-inertia-responses.md)。

## 5. 路由

`src/routes.rs`：

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

`{todo}` 这个占位符就是路由模型绑定挂靠的地方：它必须和处理程序的参数名（`todo`）一致，也必须和 SeaORM 模型的主键类型（这里是 `i64`）一致。这个可选的 `.name(...)` 后缀，就是下一步里那个路由类型生成器用来构建前端辅助函数的东西。

## 6. 生成 TypeScript 类型

```bash
suprnova generate-types
```

`generate-types` 在一次运行里做两件事：

1. 遍历 `src/` 里每一个 `#[derive(InertiaProps)]` 结构体，把它们写入 `frontend/src/types/inertia-props.ts`。
2. 遍历 `src/routes.rs`，为每一个具名路由写出类型化的 URL 构造函数，写入 `frontend/src/types/routes.ts`。

这些路由辅助函数会以一个嵌套对象的形式产出 - `controllers.todos.toggle({ todo: "1" })` 会返回一个 `{ url, method }` 对，Inertia 3 的 `Link` 和 `router` 可以直接接受它。路径参数是类型化的；编译器会在这个页面到达浏览器之前，就抓住一个缺失的 `todo` 参数。

您不需要编辑这些文件。每当您新增或者重命名了 props/路由，就重新运行一次 `suprnova generate-types`，或者传入 `--watch`，让它们在您改动的过程中保持同步。

## 7. 页面

每个页面都放在 `frontend/src/pages/Todos/` 下面。这些名字和您传给 `inertia_response!` 的字符串是对应的，所以 `inertia_response!("Todos/Index", ...)` 会解析到 `frontend/src/pages/Todos/Index.svelte`。

### 列表

`frontend/src/pages/Todos/Index.svelte`：

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

### 创建

`frontend/src/pages/Todos/Create.svelte`：

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

### 编辑

`frontend/src/pages/Todos/Edit.svelte`：

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

对等的 React 19 和 Vue 3.5 起步套件，会通过它们自己的模板语法拿到同样的 props - 后端不需要改动。

## 8. 运行它

```bash
suprnova serve
```

访问 `http://127.0.0.1:8765/todos`，添加几行，切换它们的状态，编辑一个，再删除另一个。页面之间的过渡是通过 Inertia 完成的 - 没有整页重新加载 - 而且每一次表单提交，都会在服务器端完成验证之后，才落地到那次重定向。

## 刚刚发生了什么

| 层 | 文件 | 作用 |
|---|---|---|
| 架构 | `src/migrations/m_create_todos_table.rs` | 创建 `todos` 表 |
| 模型 | `src/models/todo.rs` | 面向用户的 `Todo` 结构体，加上内部的那个 SeaORM 模块 |
| HTTP | `src/controllers/todo.rs` | 七个 `#[handler]`，包括路由模型绑定 |
| 路由器 | `src/routes.rs` | 驱动着生成出来的路由辅助函数的那些具名路由 |
| Props | `frontend/src/types/inertia-props.ts` | 从 `#[derive(InertiaProps)]` 生成而来 |
| 路由 | `frontend/src/types/routes.ts` | 从 `routes.rs` 里的具名路由生成而来 |
| 页面 | `frontend/src/pages/Todos/*.svelte` | 消费这些 props 的那三个 Svelte 5 页面 |

这就是 Suprnova 标准的功能循环：迁移 -> 模型 -> 控制器 -> 路由 -> 页面，每当您重塑了 props 或者重命名了一个路由，`suprnova generate-types` 就会重新生成这座 TypeScript 桥梁。

## 下一步

- [Eloquent](eloquent.md) - `attrs!`、查询构造器、类型转换、作用域、观察者
- [验证](validation.md) - `#[request]` 和 `#[derive(Validate)]` 能带给您什么
- [路由](routing.md) - 具名路由、路由模型绑定、资源路由、签名 URL
- [Inertia 响应](frontend-inertia-responses.md) - `inertia_response!`、部分重新加载、共享 props
- [认证](authentication.md) - 借助起步套件的会话认证，让待办事项归属到各个用户
