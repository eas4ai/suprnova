# Eine Todo-App mit Inertia erstellen

Ein vertikaler Schnitt durch Suprnova, der den vollen Stack
durchspielt: eine Migration, ein `#[suprnova::model]`,
Inertia-gerenderte Svelte-5-Seiten, Route-Model-Binding,
Formularvalidierung und typsichere Routen-Helfer, generiert aus
`routes.rs`. Arbeiten Sie das einmal durch, und die Projektschleife -
Migration, Modell, Controller, Route, Seite - wird zur Routine.

Das setzt voraus, dass Sie [Installation](installation.md) gefolgt
sind und die `suprnova`-CLI auf Ihrem `PATH` haben. Der Scaffolder
verwendet standardmäßig Svelte 5, das dieses Tutorial einsetzt.

## Was Sie bauen werden

Eine Todo-Seite mit Erstellen, Auflisten, Erledigt-Umschalten,
Bearbeiten und Löschen. Keine separate JSON-API: Inertia serialisiert
Props, und die Svelte-Seite konsumiert sie als `$props()` - dieselbe
Struktur fließt von Rust zum Browser.

## 1. Scaffold

```bash
suprnova new todo-app --frontend svelte --no-interaction
cd todo-app
npm install
```

## 2. Migration

```bash
suprnova make:migration create_todos_table
```

Öffnen Sie die neue Migration unter `src/migrations/`:

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

Sowohl `created_at` als auch `updated_at` sind vorhanden, weil das
Modell im nächsten Schritt `timestamps` verwendet, was beide Spalten
erwartet und sie automatisch verwaltet. Führen Sie dann Migrationen
aus und regenerieren Sie die Entitäten:

```bash
suprnova db:sync
```

`db:sync` führt ausstehende Migrationen aus und aktualisiert die
SeaORM-Entitätsschicht, auf die sich das `#[suprnova::model]`-Makro
verlässt.

## 3. Modell

Erstellen Sie `src/models/todo.rs`:

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

// Das Model-Makro gibt ein inneres `todo`-Modul mit den SeaORM-Typen
// Entity, ActiveModel, Column und Model aus. Re-exportieren Sie die,
// die Sie von außerhalb der Datei erreichen wollen.
pub use todo::{ActiveModel, Column, Entity};
```

Verdrahten Sie das neue Modul in `src/models/mod.rs`:

```rust
pub mod todo;
```

Die `fillable`-Liste steuert Mass Assignment; `timestamps` verwaltet
`created_at` / `updated_at` bei jedem Save automatisch. Die dem
Benutzer zugewandte `Todo`-Struktur ist der Typ, mit dem Sie in
Handlern arbeiten; das innere `todo::Model` ist die SeaORM-Form, die
Route-Model-Binding holt.

## 4. Controller

```bash
suprnova make:controller todo
```

Öffnen Sie `src/controllers/todo.rs`:

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

Ein paar Dinge, die auffallen:

- **Route-Model-Binding ist automatisch.** `todo: todo::Model` zu
  deklarieren sagt dem `#[handler]`-Makro, `{todo}` im Routenpfad
  nachzuschlagen, die SeaORM-Zeile über den Primärschlüssel zu holen
  und 404 zu liefern, wenn sie fehlt. Der Parametername muss zum
  Routenplatzhalter passen.
- **Das Makro gibt Ihnen `todo::Model`; die Eloquent-Oberfläche liegt
  auf `Todo`.** Die beiden werden durch eine `From`-Impl verbrückt,
  die `#[suprnova::model]` ausgibt, sodass `let todo: Todo =
  todo.into();` die einzeilige Umwandlung ist. `Todo` ist der Typ,
  der `update`, `delete` und den Rest der dem Benutzer zugewandten
  API trägt.
- **`#[request]` deckt die Validierung ab.** Es einer Struktur
  hinzuzufügen generiert `Deserialize`, `Validate` und `FormRequest` -
  das Framework weist fehlerhafte Eingaben mit einem 422 zurück, bevor
  Ihr Handler läuft. Es besteht keine Notwendigkeit, zusätzlich
  `InertiaProps` auf einem Request-DTO abzuleiten; dieses Derive ist
  für *ausgehende* Seiten-Props.
- **Mass Assignment läuft über `attrs!`.** `Todo::create(attrs! {
  ... })` und `todo.update(attrs! { ... })` laufen durch den
  Fillable-Filter, sodass Felder, die nicht in der `fillable`-Liste
  des Modells stehen, stillschweigend fallen gelassen werden, statt
  die Schutzmaßnahme zu umgehen.
- **`update` und `delete` konsumieren `self`.** Deshalb liest
  `toggle` `!todo.completed` in eine lokale Variable, bevor
  `todo.update(...)` aufgerufen wird.

Registrieren Sie das neue Controller-Modul in
`src/controllers/mod.rs`:

```rust
pub mod todo;
```

### Warum Suprnova abweicht

In Laravel würde derselbe Controller normalerweise JSON für eine API
oder eine Blade-View für eine serverseitig gerenderte Seite liefern.
Suprnova liefert Inertia-Responses sowohl für initiale Loads als
auch für SPA-Navigationen - das Framework erkennt den
`X-Inertia`-Header und liefert entsprechend HTML oder JSON, ohne eine
parallele API-Schicht. Sie schreiben Ihre Handler einmal, Ihr
Frontend bleibt eine echte SPA, und es gibt keinen zweiten Router,
der synchron gehalten werden muss. Siehe [Inertia Responses](frontend-inertia-responses.md)
für die Mechanik.

## 5. Routen

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

Der `{todo}`-Platzhalter ist das, worauf Route-Model-Binding einhakt:
Er muss zum Handler-Parameternamen (`todo`) passen, und er muss zum
Primärschlüssel-Typ des SeaORM-Modells passen (hier `i64`). Das
optionale `.name(...)`-Suffix ist das, was der Routen-Typ-Generator
im nächsten Schritt verwendet, um die Frontend-Helfer zu bauen.

## 6. TypeScript-Typen generieren

```bash
suprnova generate-types
```

`generate-types` tut zwei Dinge in einem Durchgang:

1. Es durchläuft jede `#[derive(InertiaProps)]`-Struktur in `src/`
   und schreibt sie nach `frontend/src/types/inertia-props.ts`.
2. Es durchläuft `src/routes.rs` und schreibt typisierte
   URL-Builder für jede benannte Route nach
   `frontend/src/types/routes.ts`.

Die Routen-Helfer kommen als verschachteltes Objekt heraus -
`controllers.todos.toggle({ todo: "1" })` liefert ein `{ url, method
}`-Paar, das Inertia 3s `Link` und `router` direkt akzeptieren.
Pfadparameter sind typisiert; der Compiler fängt ein fehlendes
`todo`-Argument, bevor die Seite den Browser erreicht.

Sie müssen diese Dateien nicht bearbeiten. Führen Sie `suprnova
generate-types` erneut aus, wann immer Sie Props/Routen hinzufügen
oder umbenennen, oder übergeben Sie `--watch`, um sie dabei synchron
zu halten.

## 7. Seiten

Jede Seite liegt unter `frontend/src/pages/Todos/`. Die Namen passen
zu den Strings, die Sie an `inertia_response!` übergeben, sodass
`inertia_response!("Todos/Index", ...)` zu
`frontend/src/pages/Todos/Index.svelte` aufgelöst wird.

### Index

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

### Erstellen

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

### Bearbeiten

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

Die äquivalenten React-19- und Vue-3.5-Starter nehmen dieselben Props
über ihr eigenes Templating entgegen - das Backend ändert sich
nicht.

## 8. Es ausführen

```bash
suprnova serve
```

Besuchen Sie `http://127.0.0.1:8765/todos`, fügen Sie ein paar Zeilen
hinzu, schalten Sie sie um, bearbeiten Sie eine, löschen Sie eine
andere. Die Seitenübergänge laufen über Inertia - kein vollständiges
Neuladen - und jede Formularübermittlung validiert serverseitig,
bevor der Redirect landet.

## Was ist gerade passiert

| Schicht | Datei | Was es tut |
|---|---|---|
| Schema | `src/migrations/m_create_todos_table.rs` | Erstellt die `todos`-Tabelle |
| Modell | `src/models/todo.rs` | Die dem Benutzer zugewandte `Todo`-Struktur + das innere SeaORM-Modul |
| HTTP | `src/controllers/todo.rs` | Sieben `#[handler]`s, einschließlich Route-Model-Binding |
| Router | `src/routes.rs` | Benannte Routen, die die generierten Routen-Helfer steuern |
| Props | `frontend/src/types/inertia-props.ts` | Generiert aus `#[derive(InertiaProps)]` |
| Routen | `frontend/src/types/routes.ts` | Generiert aus benannten Routen in `routes.rs` |
| Seiten | `frontend/src/pages/Todos/*.svelte` | Die drei Svelte-5-Seiten, die die Props konsumieren |

Das ist die Standard-Feature-Schleife von Suprnova: Migration ->
Modell -> Controller -> Route -> Seite, wobei `suprnova
generate-types` die TypeScript-Brücke regeneriert, wann immer Sie
Props umformen oder eine Route umbenennen.

## Nächste Schritte

- [Eloquent](eloquent.md) - `attrs!`, der Query Builder, Casts,
  Scopes, Observer
- [Validierung](validation.md) - was `#[request]` und
  `#[derive(Validate)]` Ihnen geben
- [Routing](routing.md) - benannte Routen, Route-Model-Binding,
  Resource-Routing, signierte URLs
- [Inertia Responses](frontend-inertia-responses.md) -
  `inertia_response!`, Partial Reloads, gemeinsame Props
- [Authentifizierung](authentication.md) - Pro-Benutzer-Todos mit der
  Session-Auth des Starters hinzufügen
