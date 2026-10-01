# Criar um app Todo com Inertia

Uma fatia vertical do Suprnova que exercita a stack completa: uma
migração, um `#[suprnova::model]`, páginas Svelte 5 renderizadas via
Inertia, binding de modelo de rota, validação de formulário, e helpers de
rota com tipos seguros gerados a partir de `routes.rs`. Percorra isso uma
vez e o loop do projeto - migração, model, controlador, rota, página -
vira memória muscular.

Isso assume que você seguiu [Instalação](installation.md) e tem a CLI
`suprnova` no seu `PATH`. O scaffolder usa Svelte 5 por padrão, que é o
que este tutorial usa.

## O que você vai construir

Uma página de todo com criar, listar, alternar-concluído, editar e
apagar. Sem uma API JSON separada: o Inertia serializa as props e a
página Svelte as consome como `$props()` - o mesmo struct flui do Rust
até o navegador.

## 1. Faça scaffold

```bash
suprnova new todo-app --frontend svelte --no-interaction
cd todo-app
npm install
```

## 2. Migração

```bash
suprnova make:migration create_todos_table
```

Abra a nova migração em `src/migrations/`:

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

Tanto `created_at` quanto `updated_at` estão presentes porque o model no
próximo passo usa `timestamps`, que espera ambas as colunas e as
autogerencia. Então execute as migrações e regenere as entidades:

```bash
suprnova db:sync
```

`db:sync` executa migrações pendentes e atualiza a camada de entidades
SeaORM em que a macro `#[suprnova::model]` se apoia.

## 3. Model

Crie `src/models/todo.rs`:

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

// A macro de model emite um módulo interno `todo` com os tipos Entity,
// ActiveModel, Column e Model do SeaORM. Reexporte os que você quer
// alcançar de fora do arquivo.
pub use todo::{ActiveModel, Column, Entity};
```

Ligue o novo módulo em `src/models/mod.rs`:

```rust
pub mod todo;
```

A lista `fillable` restringe a atribuição em massa; `timestamps`
autogerencia `created_at` / `updated_at` em todo save. O struct `Todo`
voltado ao usuário é o tipo com que você vai trabalhar nos handlers; o
`todo::Model` interno é o formato SeaORM que o binding de modelo de rota
busca.

## 4. Controlador

```bash
suprnova make:controller todo
```

Abra `src/controllers/todo.rs`:

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

Algumas coisas a notar:

- **O binding de modelo de rota é automático.** Declarar
  `todo: todo::Model` diz à macro `#[handler]` para procurar `{todo}` no
  caminho da rota, buscar a linha SeaORM pela chave primária, e retornar
  404 se estiver faltando. O nome do parâmetro precisa corresponder ao
  placeholder da rota.
- **A macro te entrega `todo::Model`; a superfície Eloquent vive em
  `Todo`.** Os dois são conectados por uma impl `From` emitida por
  `#[suprnova::model]`, então `let todo: Todo = todo.into();` é a
  conversão de uma linha. `Todo` é o tipo que carrega `update`, `delete`,
  e o resto da API voltada ao usuário.
- **`#[request]` cobre a validação.** Adicioná-lo a um struct gera
  `Deserialize`, `Validate`, e `FormRequest` - o framework rejeita
  entrada malformada com um 422 antes que seu handler execute. Não há
  necessidade de também derivar `InertiaProps` em um DTO de solicitação;
  esse derive é para props de página *de saída*.
- **A atribuição em massa passa por `attrs!`.** `Todo::create(attrs!
  { ... })` e `todo.update(attrs! { ... })` passam pelo filtro fillable,
  então campos que não estão na lista `fillable` do model são
  descartados silenciosamente em vez de contornar a proteção.
- **`update` e `delete` consomem `self`.** É por isso que `toggle` lê
  `!todo.completed` em uma variável local antes de chamar
  `todo.update(...)`.

Registre o novo módulo de controlador em `src/controllers/mod.rs`:

```rust
pub mod todo;
```

### Por que Suprnova diverge

No Laravel, o mesmo controlador normalmente retornaria JSON para uma API
ou uma view Blade para uma página renderizada no servidor. O Suprnova
retorna respostas Inertia tanto para carregamentos iniciais quanto para
navegações SPA - o framework detecta o header `X-Inertia` e serve HTML ou
JSON de acordo, sem uma camada de API paralela. Você escreve seus
handlers uma vez, seu frontend permanece uma SPA de verdade, e não há um
segundo router para manter sincronizado. Veja [Respostas
Inertia](frontend-inertia-responses.md) para a mecânica.

## 5. Rotas

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

O placeholder `{todo}` é o que o binding de modelo de rota usa como
gancho: ele precisa corresponder ao nome do parâmetro do handler
(`todo`), e precisa corresponder ao tipo de chave primária do model
SeaORM (aqui, `i64`). O sufixo opcional `.name(...)` é o que o gerador de
tipos de rota no próximo passo usa para construir os helpers do frontend.

## 6. Gerar tipos TypeScript

```bash
suprnova generate-types
```

`generate-types` faz duas coisas em uma única passada:

1. Percorre todo struct `#[derive(InertiaProps)]` em `src/` e os escreve
   em `frontend/src/types/inertia-props.ts`.
2. Percorre `src/routes.rs` e escreve construtores de URL tipados para
   toda rota nomeada em `frontend/src/types/routes.ts`.

Os helpers de rota saem como um objeto aninhado -
`controllers.todos.toggle({ todo: "1" })` retorna um par `{ url, method }`
que o `Link` e o `router` do Inertia 3 aceitam diretamente. Parâmetros de
caminho são tipados; o compilador captura um argumento `todo` ausente
antes de a página chegar ao navegador.

Você não precisa editar esses arquivos. Execute `suprnova generate-types`
de novo sempre que adicionar ou renomear props/rotas, ou passe `--watch`
para mantê-los sincronizados enquanto você trabalha.

## 7. Páginas

Cada página vive em `frontend/src/pages/Todos/`. Os nomes correspondem
às strings que você passa para `inertia_response!`, então
`inertia_response!("Todos/Index", ...)` resolve para
`frontend/src/pages/Todos/Index.svelte`.

### Índice

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

### Criar

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

### Editar

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

Os starters equivalentes de React 19 e Vue 3.5 recebem as mesmas props
através de seu próprio templating - o backend não muda.

## 8. Execute-o

```bash
suprnova serve
```

Visite `http://127.0.0.1:8765/todos`, adicione algumas linhas, alterne-as,
edite uma, apague outra. As transições de página acontecem através do
Inertia - sem reload completo - e toda submissão de formulário valida no
lado do servidor antes de o redirecionamento acontecer.

## O que acabou de acontecer

| Camada | Arquivo | O que faz |
|---|---|---|
| Esquema | `src/migrations/m_create_todos_table.rs` | Cria a tabela `todos` |
| Model | `src/models/todo.rs` | O struct `Todo` voltado ao usuário + o módulo SeaORM interno |
| HTTP | `src/controllers/todo.rs` | Sete `#[handler]`s, incluindo binding de modelo de rota |
| Router | `src/routes.rs` | Rotas nomeadas que alimentam os helpers de rota gerados |
| Props | `frontend/src/types/inertia-props.ts` | Gerado a partir de `#[derive(InertiaProps)]` |
| Rotas | `frontend/src/types/routes.ts` | Gerado a partir de rotas nomeadas em `routes.rs` |
| Páginas | `frontend/src/pages/Todos/*.svelte` | As três páginas Svelte 5 que consomem as props |

Esse é o loop de feature padrão do Suprnova: migração -> model ->
controlador -> rota -> página, com `suprnova generate-types` regerando a
ponte TypeScript sempre que você reformular props ou renomear uma rota.

## Próximos passos

- [Eloquent](eloquent.md) - `attrs!`, o construtor de consultas, casts,
  scopes, observers
- [Validação](validation.md) - o que `#[request]` e
  `#[derive(Validate)]` te dão
- [Roteamento](routing.md) - rotas nomeadas, binding de modelo de rota,
  roteamento de recursos, URLs assinadas
- [Respostas Inertia](frontend-inertia-responses.md) -
  `inertia_response!`, reloads parciais, props compartilhadas
- [Autenticação](authentication.md) - adicionando todos por usuário com
  a autenticação de sessão do starter
