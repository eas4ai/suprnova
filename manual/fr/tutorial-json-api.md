# Construire une application Todo JSON:API

Un parcours guidé du chemin API de bout en bout : migration, modèle,
requêtes de formulaire validées, liaison de modèle de route,
enveloppes de ressource JSON:API, sparse fieldsets, pagination. À la
fin, vous avez un service todo à cinq points de terminaison qui émet
des réponses [JSON:API](https://jsonapi.org/) conformes à la spec,
avec `?include=` et `?fields[todos]=...` honorés automatiquement.

Ce que vous allez construire :

| Méthode  | Route                | Action  |
|----------|----------------------|---------|
| `GET`    | `/api/todos`         | liste (paginée) |
| `GET`    | `/api/todos/{todo}`  | affichage |
| `POST`   | `/api/todos`         | création |
| `PUT`    | `/api/todos/{todo}`  | mise à jour |
| `DELETE` | `/api/todos/{todo}`  | suppression |

## Prérequis

Un projet scaffoldé :

```bash
suprnova new todo-api
cd todo-api
```

## Étape 1 : la migration

```bash
suprnova make:migration create_todos_table
```

Cela écrit `src/migrations/m<timestamp>_create_todos_table.rs`.
Remplacez le corps par le schéma de `todos` :

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

Exécutez-la :

```bash
suprnova migrate
```

Le corps de `down` permet à `migrate:rollback` d'annuler le changement
plus tard.

## Étape 2 : le modèle

Une struct `#[suprnova::model]` *est* le modèle Eloquent - la macro
émet l'`Entity`, la `Column` et l'`ActiveModel` de SeaORM dans un
module interne, et donne à la struct la surface de requête
(`Todo::query()`, `Todo::find`, `Todo::create`, `model.update`,
`model.delete`, timestamps auto-gérés, événements de cycle de vie).
Créez `src/models/todo.rs` :

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

// Ré-exporte les types SeaORM que la macro émet dans le module
// interne `todo`, pour que les sites d'appel puissent les atteindre
// sans aller fouiller dans les internals de la macro.
pub use todo::{ActiveModel, Column, Entity};
```

Câblez le module dans `src/models/mod.rs` :

```rust
pub mod todo;
```

La liste `fillable` est l'allowlist d'affectation de masse - seuls ces
champs peuvent être définis via `Todo::create(attrs!{...})` et
`model.update(attrs!{...})`. Les champs hors de la liste sont protégés
contre les écritures accidentelles depuis l'entrée de la requête.

## Étape 3 : les requêtes de formulaire

La validation vit sur une struct `#[request]`. `extract()` exécute le
validateur avant que le corps du handler ne voie la valeur ; un échec
court-circuite vers un 422 avec le sac d'erreurs Laravel/Inertia.
Créez `src/requests.rs` :

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

Et enregistrez-la dans `src/lib.rs` :

```rust
pub mod requests;
```

L'attribut `#[request]` se développe en l'équivalent de
`#[derive(serde::Deserialize, validator::Validate)] + impl FormRequest`,
si bien que les champs de la struct sont aussi le schéma d'entrée. Les
champs facultatifs (`Option<T>`) sont la bonne forme pour les mises à
jour partielles : une clé absente dans le corps JSON se désérialise en
`None`, et le handler traite `None` comme « ne pas changer cette
colonne ».

## Étape 4 : la ressource JSON:API

Une ressource est une struct `#[derive(Data)]` avec
`#[json_resource("type")]`. La macro émet l'impl `IntoJsonResource` que
consomment `Resource::single`, `Resource::collection` et
`Resource::paginated`. Les champs de la ressource deviennent l'objet
`attributes` de JSON:API - chaque filtre sparse-fieldset et chaque
chaîne `?include=` passe par ce type. Créez
`src/resources/todo_resource.rs` :

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

Câblez-la dans `src/resources/mod.rs` :

```rust
pub mod todo_resource;
```

Et redéclarez le module dans `src/lib.rs` :

```rust
pub mod resources;
```

Le champ `id` fournit le membre `id` de JSON:API (converti en chaîne
selon la spec) ; chaque autre champ atterrit dans `attributes` et est
soumis au filtrage sparse-fieldset - une requête qui nomme
`?fields[todos]=title,done` ne récupère que ces deux attributs, sans
aucun travail côté handler.

## Étape 5 : le contrôleur

L'attribut `#[handler]` classe chaque paramètre et génère l'extracteur
correspondant :

- `i64` - `FromParam` analyse le param de route nommé du même nom. Une
  entrée incorrecte (`/api/todos/abc`) court-circuite vers 400.
- `CreateTodoRequest` / `UpdateTodoRequest` - `FromRequest`
  désérialise le corps, exécute la validation, et retourne 422 en cas
  d'échec.
- `Request` - transmise telle quelle.

Charger la ligne passe par la surface Eloquent :
`Todo::find_or_fail(id)` retourne un 404 quand aucune ligne ne
correspond.

Créez `src/controllers/todos.rs` :

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
    // Remballe le paginator autour de `TodoResource`, pour que le
    // renderer JSON:API voie des objets ressource, pas des modèles
    // bruts. La fenêtre de pagination (`total`, `per_page`,
    // `current_page`) est préservée.
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

Câblez-le dans `src/controllers/mod.rs` :

```rust
pub mod todos;
```

Le nom de l'argument doit correspondre au placeholder de la
route - `{todo}` correspond à `todo: i64`. La macro analyse le segment
de chemin via `FromParam`, puis le corps du handler pilote la surface
Eloquent pour charger, mettre à jour et supprimer la ligne.

## Étape 6 : les routes

`src/routes.rs` :

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

La macro `routes!` retourne un `Router` configuré que
`Application::routes(...)` consomme à l'amorçage.

## Étape 7 : l'exécuter

```bash
suprnova serve --backend-only
```

### Créer

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

### Liste (paginée)

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

`IncludeMiddleware` analyse `?fields[type]=...`, lie le filtre à un
task-local, et `Resource::single` le lit pendant le rendu - le handler
ne voit pas du tout le paramètre de requête.

### Mettre à jour

```bash
curl -X PUT http://localhost:8765/api/todos/1 \
  -H "Content-Type: application/json" \
  -d '{"done": true}'
```

Un corps partiel fonctionne parce que chaque champ de
`UpdateTodoRequest` est un `Option<T>` - le handler n'écrit que les
clés qui sont arrivées.

### Supprimer

```bash
curl -X DELETE http://localhost:8765/api/todos/1
# {"deleted": true}
```

### Échec de validation

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

422 avec le sac d'erreurs Laravel/Inertia - le corps du handler n'a
jamais tourné.

## Où réside chaque élément

| Fichier | Rôle |
|------|------|
| `src/migrations/m*_create_todos_table.rs` | schéma |
| `src/models/todo.rs` | struct `#[suprnova::model]` |
| `src/requests.rs` | requêtes de formulaire `#[request]`, validées par `extract()` |
| `src/resources/todo_resource.rs` | `#[derive(Data)]` + `#[json_resource("todos")]` |
| `src/controllers/todos.rs` | fonctions `#[handler]` |
| `src/routes.rs` | enregistrements `routes!` |

## Suivant

- [Eloquent](eloquent.md) - la surface complète du Model, le query
  builder, `attrs!`, les événements de cycle de vie, les suppressions
  douces, les relations
- [Validation](validation.md) - `#[request]`, `validate!`, `Unique`,
  hooks async, règles inter-champs
- [Ressources JSON:API](eloquent-resources.md) - chaînes `?include=`,
  liens/meta par ressource, attributs conditionnels `Maybe<T>`
- [Requêtes de formulaire](requests.md) - trait `FormRequest`,
  dispatch selon le content-type, `authorize(&Request)`
- [Contrôleurs](controllers.md) - ce que `#[handler]` extrait et
  comment la liaison de modèle de route fonctionne sous le capot
