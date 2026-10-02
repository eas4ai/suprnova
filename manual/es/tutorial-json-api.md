# Construir un JSON:API de tareas

Un recorrido de principio a fin por el camino de la API: migración,
modelo, solicitudes de formulario validadas, vinculación de modelo de
ruta, envolturas de recurso JSON:API, campos dispersos, paginación. Al
final tendrás un servicio de tareas con cinco endpoints que emite
respuestas [JSON:API](https://jsonapi.org/) conformes con la
especificación, con `?include=` y `?fields[todos]=...` respetados
automáticamente.

Lo que vas a construir:

| Método   | Ruta                 | Acción  |
|----------|----------------------|---------|
| `GET`    | `/api/todos`         | listar (paginado) |
| `GET`    | `/api/todos/{todo}`  | mostrar |
| `POST`   | `/api/todos`         | crear |
| `PUT`    | `/api/todos/{todo}`  | actualizar |
| `DELETE` | `/api/todos/{todo}`  | eliminar |

## Prerrequisitos

Un proyecto con andamiaje:

```bash
suprnova new todo-api
cd todo-api
```

## Paso 1: La migración

```bash
suprnova make:migration create_todos_table
```

Esto escribe `src/migrations/m<timestamp>_create_todos_table.rs`.
Sustituye el cuerpo por el esquema de `todos`:

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

Ejecútalo:

```bash
suprnova migrate
```

El cuerpo de `down` permite que `migrate:rollback` revierta el cambio
más adelante.

## Paso 2: El modelo

Un struct `#[suprnova::model]` *es* el modelo Eloquent - la macro emite
el `Entity`, `Column` y `ActiveModel` de SeaORM en un módulo interno y
le da al struct la superficie de consulta (`Todo::query()`,
`Todo::find`, `Todo::create`, `model.update`, `model.delete`,
timestamps autogestionados, eventos de ciclo de vida). Crea
`src/models/todo.rs`:

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

// Reexporta los tipos de SeaORM que la macro emite en el módulo
// interno `todo` para que los sitios de llamada puedan usarlos sin
// hurgar en los internos de la macro.
pub use todo::{ActiveModel, Column, Entity};
```

Conecta el módulo en `src/models/mod.rs`:

```rust
pub mod todo;
```

La lista `fillable` es la lista de campos permitidos para la
asignación masiva - solo esos campos se pueden establecer vía
`Todo::create(attrs!{...})` y `model.update(attrs!{...})`. Los campos
fuera de la lista quedan protegidos frente a escrituras accidentales
desde la entrada de la solicitud.

## Paso 3: Las solicitudes de formulario

La validación vive en un struct `#[request]`. `extract()` ejecuta el
validador antes de que el cuerpo del handler vea el valor; un fallo
cortocircuita a un 422 con la bolsa de errores Laravel/Inertia. Crea
`src/requests.rs`:

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

Y regístralo en `src/lib.rs`:

```rust
pub mod requests;
```

El atributo `#[request]` se expande al equivalente de
`#[derive(serde::Deserialize, validator::Validate)] + impl FormRequest`,
así que los campos del struct son también el esquema de entrada. Los
campos opcionales (`Option<T>`) son la forma correcta para las
actualizaciones parciales: una clave ausente en el cuerpo JSON
deserializa a `None`, y el handler trata `None` como "no cambies esta
columna".

## Paso 4: El recurso JSON:API

Un recurso es un struct `#[derive(Data)]` con `#[json_resource("type")]`.
La macro emite el impl `IntoJsonResource` que consumen
`Resource::single`, `Resource::collection` y `Resource::paginated`. Los
campos del recurso se convierten en el objeto `attributes` de
JSON:API - cada filtro de campos dispersos y cada cadena `?include=`
se despachan a través de este tipo. Crea
`src/resources/todo_resource.rs`:

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

Conéctalo en `src/resources/mod.rs`:

```rust
pub mod todo_resource;
```

Y vuelve a declarar el módulo en `src/lib.rs`:

```rust
pub mod resources;
```

El campo `id` provee el miembro `id` de JSON:API (convertido a cadena
según la especificación); todos los demás campos aterrizan en
`attributes` y quedan sujetos al filtrado de campos dispersos - una
solicitud que indica `?fields[todos]=title,done` recibe de vuelta solo
esos dos atributos, sin ningún trabajo del lado del handler.

## Paso 5: El controlador

El atributo `#[handler]` clasifica cada parámetro y genera el
extractor correspondiente:

- `i64` - `FromParam` analiza el param de ruta nombrado del mismo
  nombre. Una entrada incorrecta (`/api/todos/abc`) cortocircuita a
  400.
- `CreateTodoRequest` / `UpdateTodoRequest` - `FromRequest` deserializa
  el cuerpo, ejecuta la validación, y responde 422 si falla.
- `Request` - se pasa tal cual, sin cambios.

Cargar la fila pasa por la superficie de Eloquent:
`Todo::find_or_fail(id)` devuelve un 404 cuando ninguna fila coincide.

Crea `src/controllers/todos.rs`:

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
    // Reempaqueta el paginador alrededor de `TodoResource` para que el
    // renderizador de JSON:API vea objetos de recurso, no modelos en
    // crudo. La ventana de paginación (`total`, `per_page`,
    // `current_page`) se conserva.
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

Conéctalo en `src/controllers/mod.rs`:

```rust
pub mod todos;
```

El nombre del argumento debe coincidir con el placeholder de la ruta -
`{todo}` se corresponde con `todo: i64`. La macro analiza el segmento
de ruta vía `FromParam`, y el cuerpo del handler entonces conduce la
superficie de Eloquent para cargar, actualizar y eliminar la fila.

## Paso 6: Las rutas

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

La macro `routes!` devuelve un `Router` configurado que
`Application::routes(...)` consume en el arranque.

## Paso 7: Ejecutarlo

```bash
suprnova serve --backend-only
```

### Crear

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

### Listar (paginado)

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

### Campos dispersos

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

El `IncludeMiddleware` analiza `?fields[type]=...`, vincula el filtro a
un task-local, y `Resource::single` lo lee durante el render - el
handler no ve el parámetro de query en absoluto.

### Actualizar

```bash
curl -X PUT http://localhost:8765/api/todos/1 \
  -H "Content-Type: application/json" \
  -d '{"done": true}'
```

Un cuerpo parcial funciona porque cada campo de `UpdateTodoRequest` es
`Option<T>` - el handler solo escribe las claves que llegaron.

### Eliminar

```bash
curl -X DELETE http://localhost:8765/api/todos/1
# {"deleted": true}
```

### Fallo de validación

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

422 con la bolsa de errores Laravel/Inertia - el cuerpo del handler
nunca se ejecutó.

## Dónde vive cada pieza

| Archivo | Rol |
|------|------|
| `src/migrations/m*_create_todos_table.rs` | esquema |
| `src/models/todo.rs` | struct `#[suprnova::model]` |
| `src/requests.rs` | solicitudes de formulario `#[request]`, validadas por `extract()` |
| `src/resources/todo_resource.rs` | `#[derive(Data)]` + `#[json_resource("todos")]` |
| `src/controllers/todos.rs` | funciones `#[handler]` |
| `src/routes.rs` | registros `routes!` |

## Siguiente

- [Eloquent](eloquent.md) - la superficie completa de Model, el query
  builder, `attrs!`, eventos de ciclo de vida, soft deletes, relaciones
- [Validación](validation.md) - `#[request]`, `validate!`, `Unique`,
  ganchos asíncronos, reglas entre campos
- [Recursos JSON:API](eloquent-resources.md) - cadenas `?include=`,
  links/meta por recurso, atributos condicionales `Maybe<T>`
- [Solicitudes de formulario](requests.md) - el trait `FormRequest`, el
  despacho según content-type, `authorize(&Request)`
- [Controladores](controllers.md) - qué extrae `#[handler]` y cómo
  funciona la vinculación de modelo de ruta por debajo
