# Eine Todo JSON:API erstellen

Eine Schritt-für-Schritt-Tour durch den kompletten API-Pfad:
Migration, Modell, validierte Form-Requests, Route-Model-Binding,
JSON:API-Ressourcen-Envelopes, Sparse Fieldsets, Paginierung. Am Ende
haben Sie einen Todo-Service mit fünf Endpunkten, der
spezifikationskonforme [JSON:API](https://jsonapi.org/)-Antworten
ausgibt, bei denen `?include=` und `?fields[todos]=...` automatisch
berücksichtigt werden.

Was Sie bauen werden:

| Methode  | Route                | Aktion  |
|----------|----------------------|---------|
| `GET`    | `/api/todos`         | Liste (paginiert) |
| `GET`    | `/api/todos/{todo}`  | Anzeigen |
| `POST`   | `/api/todos`         | Erstellen |
| `PUT`    | `/api/todos/{todo}`  | Aktualisieren |
| `DELETE` | `/api/todos/{todo}`  | Löschen |

## Voraussetzungen

Ein gescaffoldetes Projekt:

```bash
suprnova new todo-api
cd todo-api
```

## Schritt 1: Die Migration

```bash
suprnova make:migration create_todos_table
```

Das schreibt `src/migrations/m<timestamp>_create_todos_table.rs`.
Ersetzen Sie den Rumpf durch das Schema für `todos`:

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

Führen Sie sie aus:

```bash
suprnova migrate
```

Der `down`-Rumpf lässt `migrate:rollback` die Änderung später
rückgängig machen.

## Schritt 2: Das Modell

Eine `#[suprnova::model]`-Struktur *ist* das Eloquent-Modell - das
Makro gibt das SeaORM-`Entity`, `Column` und `ActiveModel` in einem
inneren Modul aus und gibt der Struktur die Query-Oberfläche
(`Todo::query()`, `Todo::find`, `Todo::create`, `model.update`,
`model.delete`, automatisch verwaltete Timestamps,
Lifecycle-Events). Erstellen Sie `src/models/todo.rs`:

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

// Re-exportiert die SeaORM-Typen, die das Makro im inneren `todo`-
// Modul ausgibt, damit Aufrufstellen darauf zugreifen können, ohne
// in den Makro-Interna zu stochern.
pub use todo::{ActiveModel, Column, Entity};
```

Verdrahten Sie das Modul in `src/models/mod.rs`:

```rust
pub mod todo;
```

Die `fillable`-Liste ist die Allowlist für Mass Assignment - nur
diese Felder können über `Todo::create(attrs!{...})` und
`model.update(attrs!{...})` gesetzt werden. Felder außerhalb der
Liste sind gegen versehentliche Schreibvorgänge aus Request-Eingaben
abgesichert.

## Schritt 3: Die Form-Requests

Die Validierung liegt auf einer `#[request]`-Struktur. `extract()`
lässt den Validator laufen, bevor der Handler-Body den Wert sieht;
ein Fehlschlag bricht per Short-Circuit zu einem 422 mit der
Laravel-/Inertia-Error-Bag ab. Erstellen Sie `src/requests.rs`:

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

Und registrieren Sie es in `src/lib.rs`:

```rust
pub mod requests;
```

Das `#[request]`-Attribut expandiert zum Äquivalent von
`#[derive(serde::Deserialize, validator::Validate)] + impl
FormRequest`, sodass die Felder der Struktur auch das Eingabeschema
sind. Optionale Felder (`Option<T>`) sind die richtige Form für
Teil-Updates: Ein fehlender Schlüssel im JSON-Body deserialisiert zu
`None`, und der Handler behandelt `None` als „diese Spalte nicht
ändern“.

## Schritt 4: Die JSON:API-Ressource

Eine Ressource ist eine `#[derive(Data)]`-Struktur mit
`#[json_resource("type")]`. Das Makro gibt die
`IntoJsonResource`-Impl aus, die `Resource::single`,
`Resource::collection` und `Resource::paginated` konsumieren. Die
Felder der Ressource werden zum JSON:API-`attributes`-Objekt - jeder
Sparse-Fieldset-Filter und jede `?include=`-Kette wird über diesen
Typ dispatcht. Erstellen Sie `src/resources/todo_resource.rs`:

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

Verdrahten Sie es in `src/resources/mod.rs`:

```rust
pub mod todo_resource;
```

Und deklarieren Sie das Modul erneut in `src/lib.rs`:

```rust
pub mod resources;
```

Das `id`-Feld liefert das JSON:API-`id`-Member (gemäß Spec
stringifiziert); jedes andere Feld landet in `attributes` und
unterliegt der Sparse-Fieldset-Filterung - eine Anfrage, die
`?fields[todos]=title,done` nennt, bekommt nur diese beiden Attribute
zurück, ohne jede handlerseitige Arbeit.

## Schritt 5: Der Controller

Das `#[handler]`-Attribut klassifiziert jeden Parameter und generiert
den passenden Extraktor:

- `i64` - `FromParam` parst den gleichnamigen Routenparameter.
  Fehlerhafte Eingabe (`/api/todos/abc`) bricht per Short-Circuit zu
  400 ab.
- `CreateTodoRequest` / `UpdateTodoRequest` - `FromRequest`
  deserialisiert den Body, führt die Validierung aus und liefert bei
  einem Fehlschlag 422.
- `Request` - unverändert durchgereicht.

Das Laden der Zeile läuft über die Eloquent-Oberfläche:
`Todo::find_or_fail(id)` liefert 404, wenn keine Zeile passt.

Erstellen Sie `src/controllers/todos.rs`:

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
    // Verpackt den Paginator neu um `TodoResource`, damit der
    // JSON:API-Renderer Ressourcenobjekte sieht, keine rohen Modelle.
    // Das Paginierungsfenster (`total`, `per_page`, `current_page`)
    // bleibt erhalten.
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

Verdrahten Sie es in `src/controllers/mod.rs`:

```rust
pub mod todos;
```

Der Argumentname muss zum Routenplatzhalter passen - `{todo}` bildet
auf `todo: i64` ab. Das Makro parst das Pfadsegment über `FromParam`,
und der Handler-Body steuert dann die Eloquent-Oberfläche, um die
Zeile zu laden, zu aktualisieren und zu löschen.

## Schritt 6: Die Routen

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

Das `routes!`-Makro liefert einen konfigurierten `Router`, den
`Application::routes(...)` beim Boot konsumiert.

## Schritt 7: Es ausführen

```bash
suprnova serve --backend-only
```

### Erstellen

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

### Liste (paginiert)

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

### Sparse Fieldsets

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

Die `IncludeMiddleware` parst `?fields[type]=...`, bindet den Filter
an ein Task-Local, und `Resource::single` liest ihn während des
Renderns - der Handler sieht den Query-Parameter überhaupt nicht.

### Aktualisieren

```bash
curl -X PUT http://localhost:8765/api/todos/1 \
  -H "Content-Type: application/json" \
  -d '{"done": true}'
```

Ein Teil-Body funktioniert, weil jedes Feld in `UpdateTodoRequest`
ein `Option<T>` ist - der Handler schreibt nur die Schlüssel, die
angekommen sind.

### Löschen

```bash
curl -X DELETE http://localhost:8765/api/todos/1
# {"deleted": true}
```

### Validierungsfehler

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

422 mit der Laravel-/Inertia-Error-Bag - der Handler-Body ist nie
gelaufen.

## Wo jedes Teil lebt

| Datei | Rolle |
|------|------|
| `src/migrations/m*_create_todos_table.rs` | Schema |
| `src/models/todo.rs` | `#[suprnova::model]`-Struktur |
| `src/requests.rs` | `#[request]`-Form-Requests, validiert durch `extract()` |
| `src/resources/todo_resource.rs` | `#[derive(Data)]` + `#[json_resource("todos")]` |
| `src/controllers/todos.rs` | `#[handler]`-Funktionen |
| `src/routes.rs` | `routes!`-Registrierungen |

## Nächste Schritte

- [Eloquent API](eloquent.md) - die vollständige Model-Oberfläche,
  der Query Builder, `attrs!`, Lifecycle-Events, Soft Deletes,
  Beziehungen
- [Validierung](validation.md) - `#[request]`, `validate!`, `Unique`,
  asynchrone Hooks, feldübergreifende Regeln
- [JSON:API Resources](eloquent-resources.md) - `?include=`-Ketten,
  Pro-Ressource-Links/Meta, `Maybe<T>`-bedingte Attribute
- [Form-Requests](requests.md) - das `FormRequest`-Trait,
  Content-Type-Dispatch, `authorize(&Request)`
- [Controller](controllers.md) - was `#[handler]` extrahiert und wie
  Route-Model-Binding unter der Haube funktioniert
