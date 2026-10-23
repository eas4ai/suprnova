# Criar um Todo JSON:API

Um guia passo a passo do caminho da API de ponta a ponta: migração,
model, form requests validados, binding de modelo de rota, envelopes de
recurso JSON:API, sparse fieldsets, paginação. Ao final você tem um
serviço de todo com cinco endpoints que emite respostas
[JSON:API](https://jsonapi.org/) conformes à spec, honrando
`?include=` e `?fields[todos]=...` automaticamente.

O que você vai construir:

| Método   | Rota                 | Ação    |
|----------|----------------------|---------|
| `GET`    | `/api/todos`         | listar (paginado) |
| `GET`    | `/api/todos/{todo}`  | exibir |
| `POST`   | `/api/todos`         | criar |
| `PUT`    | `/api/todos/{todo}`  | atualizar |
| `DELETE` | `/api/todos/{todo}`  | apagar |

## Pré-requisitos

Um projeto com scaffold:

```bash
suprnova new todo-api
cd todo-api
```

## Passo 1: A migração

```bash
suprnova make:migration create_todos_table
```

Isso escreve `src/migrations/m<timestamp>_create_todos_table.rs`.
Substitua o corpo pelo esquema de `todos`:

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

Execute-a:

```bash
suprnova migrate
```

O corpo de `down` permite que `migrate:rollback` reverta a mudança mais
tarde.

## Passo 2: O model

Um struct `#[suprnova::model]` *é* o model Eloquent - a macro emite a
`Entity`, `Column` e `ActiveModel` do SeaORM em um módulo interno e dá ao
struct a superfície de consulta (`Todo::query()`, `Todo::find`,
`Todo::create`, `model.update`, `model.delete`, timestamps
autogerenciados, eventos de ciclo de vida). Crie `src/models/todo.rs`:

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

// Reexporta os tipos SeaORM que a macro emite no módulo interno `todo`
// para que os call sites possam alcançá-los sem cutucar os internos da
// macro.
pub use todo::{ActiveModel, Column, Entity};
```

Ligue o módulo em `src/models/mod.rs`:

```rust
pub mod todo;
```

A lista `fillable` é a allowlist de atribuição em massa - somente esses
campos podem ser definidos via `Todo::create(attrs!{...})` e
`model.update(attrs!{...})`. Campos fora da lista são protegidos contra
escritas acidentais vindas da entrada da solicitação.

## Passo 3: Os form requests

A validação vive em um struct `#[request]`. `extract()` executa o
validador antes que o corpo do handler veja o valor; uma falha faz um
curto-circuito para um 422 com o bag de erros do Laravel/Inertia. Crie
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

E registre-o em `src/lib.rs`:

```rust
pub mod requests;
```

O atributo `#[request]` se expande para o equivalente de
`#[derive(serde::Deserialize, validator::Validate)] + impl FormRequest`,
então os campos do struct são também o schema de entrada. Campos
opcionais (`Option<T>`) são o formato certo para atualizações parciais:
uma chave ausente no corpo JSON desserializa para `None`, e o handler
trata `None` como "não mude esta coluna".

## Passo 4: O recurso JSON:API

Um recurso é um struct `#[derive(Data)]` com `#[json_resource("type")]`.
A macro emite a impl `IntoJsonResource` que `Resource::single`,
`Resource::collection`, e `Resource::paginated` consomem. Os campos do
recurso se tornam o objeto `attributes` do JSON:API - todo filtro de
sparse fieldset e toda cadeia `?include=` passa por esse tipo. Crie
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

Ligue-o em `src/resources/mod.rs`:

```rust
pub mod todo_resource;
```

E redeclare o módulo em `src/lib.rs`:

```rust
pub mod resources;
```

O campo `id` fornece o membro `id` do JSON:API (convertido para string
conforme a spec); todo outro campo cai em `attributes` e está sujeito à
filtragem de sparse fieldset - uma solicitação que nomeia
`?fields[todos]=title,done` recebe de volta somente esses dois
atributos, sem nenhum trabalho do lado do handler.

## Passo 5: O controlador

O atributo `#[handler]` classifica cada parâmetro e gera o extractor
correspondente:

- `i64` - `FromParam` faz parse do param de rota de mesmo nome. Entrada
  inválida (`/api/todos/abc`) faz um curto-circuito para 400.
- `CreateTodoRequest` / `UpdateTodoRequest` - `FromRequest` desserializa
  o corpo, executa a validação, e retorna 422 em caso de falha.
- `Request` - passado sem alterações.

Carregar a linha passa pela superfície Eloquent:
`Todo::find_or_fail(id)` retorna um 404 quando nenhuma linha
corresponde.

Crie `src/controllers/todos.rs`:

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
    // Reempacota o paginator em torno de `TodoResource` para que o
    // renderizador JSON:API veja objetos de recurso, não models brutos.
    // A janela de paginação (`total`, `per_page`, `current_page`) é
    // preservada.
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

Ligue-o em `src/controllers/mod.rs`:

```rust
pub mod todos;
```

O nome do argumento precisa corresponder ao placeholder da rota -
`{todo}` mapeia para `todo: i64`. A macro faz parse do segmento do
caminho via `FromParam`, e o corpo do handler então conduz a superfície
Eloquent para carregar, atualizar e apagar a linha.

## Passo 6: As rotas

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

A macro `routes!` retorna um `Router` configurado que
`Application::routes(...)` consome na inicialização.

## Passo 7: Execute-o

```bash
suprnova serve --backend-only
```

### Criar

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

### Sparse fieldsets

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

O `IncludeMiddleware` faz parse de `?fields[type]=...`, vincula o filtro
a um task-local, e `Resource::single` o lê durante a renderização - o
handler não vê o parâmetro de query em nenhum momento.

### Atualizar

```bash
curl -X PUT http://localhost:8765/api/todos/1 \
  -H "Content-Type: application/json" \
  -d '{"done": true}'
```

Um corpo parcial funciona porque todo campo em `UpdateTodoRequest` é
`Option<T>` - o handler só escreve as chaves que chegaram.

### Apagar

```bash
curl -X DELETE http://localhost:8765/api/todos/1
# {"deleted": true}
```

### Falha de validação

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

422 com o bag de erros do Laravel/Inertia - o corpo do handler nunca
executou.

## Onde cada peça vive

| Arquivo | Papel |
|------|------|
| `src/migrations/m*_create_todos_table.rs` | esquema |
| `src/models/todo.rs` | struct `#[suprnova::model]` |
| `src/requests.rs` | form requests `#[request]`, validados por `extract()` |
| `src/resources/todo_resource.rs` | `#[derive(Data)]` + `#[json_resource("todos")]` |
| `src/controllers/todos.rs` | funções `#[handler]` |
| `src/routes.rs` | registros de `routes!` |

## Próximos passos

- [Eloquent](eloquent.md) - a superfície completa de Model, o construtor
  de consultas, `attrs!`, eventos de ciclo de vida, soft deletes,
  relacionamentos
- [Validação](validation.md) - `#[request]`, `validate!`, `Unique`,
  hooks async, regras entre campos
- [Recursos JSON:API](eloquent-resources.md) - cadeias `?include=`,
  links/meta por recurso, atributos condicionais `Maybe<T>`
- [Form Requests](requests.md) - trait `FormRequest`, dispatch por
  content-type, `authorize(&Request)`
- [Controladores](controllers.md) - o que `#[handler]` extrai e como o
  binding de modelo de rota funciona por baixo dos panos
