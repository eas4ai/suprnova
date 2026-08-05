# Construir una aplicación de tareas con Inertia

Un corte vertical de Suprnova que ejercita la pila completa: una
migración, un `#[suprnova::model]`, páginas Svelte 5 renderizadas por
Inertia, vinculación de modelo de ruta, validación de formularios, y
ayudantes de ruta con seguridad de tipos generados a partir de
`routes.rs`. Trabaja esto una vez y el bucle del proyecto - migración,
modelo, controlador, ruta, página - se te queda en la memoria
muscular.

Esto asume que ya has seguido [Instalación](installation.md) y que
tienes la CLI `suprnova` en tu `PATH`. El generador de andamiaje usa
Svelte 5 por defecto, que es lo que usa este tutorial.

## Lo que vas a construir

Una página de tareas con crear, listar, marcar como completada, editar
y eliminar. Sin API JSON separada: Inertia serializa los props y la
página Svelte los consume como `$props()` - el mismo struct fluye de
Rust al navegador.

## 1. Andamiar

```bash
suprnova new todo-app --frontend svelte --no-interaction
cd todo-app
npm install
```

## 2. Migración

```bash
suprnova make:migration create_todos_table
```

Abre la nueva migración bajo `src/migrations/`:

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

Tanto `created_at` como `updated_at` están presentes porque el modelo
del siguiente paso usa `timestamps`, que espera ambas columnas y las
gestiona automáticamente. Luego ejecuta las migraciones y regenera las
entidades:

```bash
suprnova db:sync
```

`db:sync` ejecuta las migraciones pendientes y refresca la capa de
entidades de SeaORM en la que se apoya la macro `#[suprnova::model]`.

## 3. Modelo

Crea `src/models/todo.rs`:

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

// La macro de modelo emite un módulo interno `todo` con los tipos
// Entity, ActiveModel, Column y Model de SeaORM. Reexporta los que
// quieras poder usar desde fuera del archivo.
pub use todo::{ActiveModel, Column, Entity};
```

Conecta el nuevo módulo en `src/models/mod.rs`:

```rust
pub mod todo;
```

La lista `fillable` acota la asignación masiva; `timestamps` gestiona
automáticamente `created_at` / `updated_at` en cada guardado. El
struct `Todo` de cara al usuario es el tipo con el que trabajarás en
los handlers; el `todo::Model` interno es la forma de SeaORM que la
vinculación de modelo de ruta obtiene.

## 4. Controlador

```bash
suprnova make:controller todo
```

Abre `src/controllers/todo.rs`:

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

Unas pocas cosas que notar:

- **La vinculación de modelo de ruta es automática.** Declarar
  `todo: todo::Model` le indica a la macro `#[handler]` que busque
  `{todo}` en la ruta, obtenga la fila de SeaORM por clave primaria, y
  devuelva 404 si falta. El nombre del parámetro debe coincidir con el
  placeholder de la ruta.
- **La macro te entrega `todo::Model`; la superficie de Eloquent vive
  en `Todo`.** Las dos se conectan mediante un impl `From` emitido por
  `#[suprnova::model]`, así que `let todo: Todo = todo.into();` es la
  conversión de una línea. `Todo` es el tipo que lleva `update`,
  `delete`, y el resto de la API de cara al usuario.
- **`#[request]` cubre la validación.** Añadirlo a un struct genera
  `Deserialize`, `Validate` y `FormRequest` - el framework rechaza la
  entrada malformada con un 422 antes de que se ejecute tu handler. No
  hace falta derivar también `InertiaProps` en un DTO de solicitud; ese
  derive es para los props de página *salientes*.
- **La asignación masiva pasa por `attrs!`.** `Todo::create(attrs! { ... })`
  y `todo.update(attrs! { ... })` se encaminan a través del filtro
  fillable, así que los campos que no están en la lista `fillable` del
  modelo se descartan en silencio en lugar de saltarse la salvaguarda.
- **`update` y `delete` consumen `self`.** Por eso `toggle` lee
  `!todo.completed` en una variable local antes de llamar a
  `todo.update(...)`.

Registra el nuevo módulo de controlador en `src/controllers/mod.rs`:

```rust
pub mod todo;
```

### Por qué Suprnova diverge

En Laravel, el mismo controlador normalmente devolvería JSON para una
API o una vista Blade para una página renderizada por el servidor.
Suprnova devuelve respuestas de Inertia tanto para las cargas
iniciales como para las navegaciones SPA - el framework detecta el
encabezado `X-Inertia` y sirve HTML o JSON según corresponda, sin una
capa de API paralela. Escribes tus handlers una sola vez, tu frontend
se queda como una SPA real, y no hay un segundo router que mantener
sincronizado. Consulta [Respuestas de Inertia](frontend-inertia-responses.md)
para los detalles internos.

## 5. Rutas

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

El placeholder `{todo}` es donde se engancha la vinculación de modelo
de ruta: tiene que coincidir con el nombre del parámetro del handler
(`todo`), y tiene que coincidir con el tipo de clave primaria del
modelo de SeaORM (aquí, `i64`). El sufijo opcional `.name(...)` es lo
que el generador de tipos de ruta del siguiente paso usa para
construir los ayudantes de frontend.

## 6. Generar tipos de TypeScript

```bash
suprnova generate-types
```

`generate-types` hace dos cosas en una sola pasada:

1. Recorre cada struct `#[derive(InertiaProps)]` en `src/` y los
   escribe en `frontend/src/types/inertia-props.ts`.
2. Recorre `src/routes.rs` y escribe builders de URL tipados para cada
   ruta nombrada en `frontend/src/types/routes.ts`.

Los ayudantes de ruta salen como un objeto anidado -
`controllers.todos.toggle({ todo: "1" })` devuelve un par
`{ url, method }` que `Link` y `router` de Inertia 3 aceptan
directamente. Los parámetros de ruta están tipados; el compilador
detecta un argumento `todo` faltante antes de que la página llegue al
navegador.

No tienes que editar estos archivos. Vuelve a ejecutar `suprnova
generate-types` cada vez que añadas o renombres props/rutas, o pasa
`--watch` para mantenerlos sincronizados sobre la marcha.

## 7. Páginas

Cada página vive bajo `frontend/src/pages/Todos/`. Los nombres
coinciden con las cadenas que pasas a `inertia_response!`, así que
`inertia_response!("Todos/Index", ...)` resuelve a
`frontend/src/pages/Todos/Index.svelte`.

### Listado

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

### Crear

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

Los starters equivalentes de React 19 y Vue 3.5 toman los mismos props
a través de su propio templating - el backend no cambia.

## 8. Ejecutarlo

```bash
suprnova serve
```

Visita `http://127.0.0.1:8765/todos`, añade algunas filas, márcalas,
edita una, elimina otra. Las transiciones de página ocurren a través
de Inertia - sin recarga completa - y cada envío de formulario valida
del lado del servidor antes de que llegue la redirección.

## Lo que acaba de suceder

| Capa | Archivo | Qué hace |
|---|---|---|
| Esquema | `src/migrations/m_create_todos_table.rs` | Crea la tabla `todos` |
| Modelo | `src/models/todo.rs` | El struct `Todo` de cara al usuario + el módulo interno de SeaORM |
| HTTP | `src/controllers/todo.rs` | Siete `#[handler]`, incluida la vinculación de modelo de ruta |
| Router | `src/routes.rs` | Rutas nombradas que impulsan los ayudantes de ruta generados |
| Props | `frontend/src/types/inertia-props.ts` | Generado a partir de `#[derive(InertiaProps)]` |
| Rutas | `frontend/src/types/routes.ts` | Generado a partir de las rutas nombradas en `routes.rs` |
| Páginas | `frontend/src/pages/Todos/*.svelte` | Las tres páginas Svelte 5 que consumen los props |

Ese es el bucle de features estándar de Suprnova: migración -> modelo
-> controlador -> ruta -> página, con `suprnova generate-types`
regenerando el puente de TypeScript cada vez que remodelas los props o
renombras una ruta.

## Siguiente

- [Eloquent](eloquent.md) - `attrs!`, el query builder, casts, scopes,
  observadores
- [Validación](validation.md) - qué te dan `#[request]` y
  `#[derive(Validate)]`
- [Enrutamiento](routing.md) - rutas nombradas, vinculación de modelo
  de ruta, enrutamiento de recursos, URLs firmadas
- [Respuestas de Inertia](frontend-inertia-responses.md) -
  `inertia_response!`, recargas parciales, props compartidos
- [Autenticación](authentication.md) - añadir tareas por usuario con
  la autenticación de sesión del starter
