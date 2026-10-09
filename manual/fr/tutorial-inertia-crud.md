# Construire une application Todo avec Inertia

Une tranche verticale de Suprnova qui met à l'épreuve toute la pile :
une migration, une struct `#[suprnova::model]`, des pages Svelte 5
rendues par Inertia, la liaison de modèle de route, la validation de
formulaire, et des helpers de route type-safe générés depuis
`routes.rs`. Travaillez ceci une fois et la boucle du projet -
migration, modèle, contrôleur, route, page - devient un réflexe.

Ceci suppose que vous avez suivi [Installation](installation.md) et
que vous avez la CLI `suprnova` sur votre `PATH`. Le scaffolder est par
défaut sur Svelte 5, ce que ce tutoriel utilise.

## Ce que vous allez construire

Une page todo avec création, liste, bascule d'achèvement, modification
et suppression. Pas d'API JSON séparée : Inertia sérialise les props
et la page Svelte les consomme comme `$props()` - la même struct
s'écoule de Rust jusqu'au navigateur.

## 1. Créer la structure du projet

```bash
suprnova new todo-app --frontend svelte --no-interaction
cd todo-app
npm install
```

## 2. Migration

```bash
suprnova make:migration create_todos_table
```

Ouvrez la nouvelle migration sous `src/migrations/` :

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

`created_at` et `updated_at` sont tous les deux présents parce que le
modèle de l'étape suivante utilise `timestamps`, qui attend les deux
colonnes et les gère automatiquement. Exécutez ensuite les migrations
et régénérez les entités :

```bash
suprnova db:sync
```

`db:sync` exécute les migrations en attente et actualise la couche
d'entités SeaORM sur laquelle repose la macro `#[suprnova::model]`.

## 3. Modèle

Créez `src/models/todo.rs` :

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

// La macro model émet un module interne `todo` avec les types SeaORM
// Entity, ActiveModel, Column et Model. Ré-exportez ceux que vous
// voulez atteindre depuis l'extérieur du fichier.
pub use todo::{ActiveModel, Column, Entity};
```

Câblez le nouveau module dans `src/models/mod.rs` :

```rust
pub mod todo;
```

La liste `fillable` filtre l'affectation de masse ; `timestamps` gère
automatiquement `created_at` / `updated_at` à chaque enregistrement.
La struct `Todo` orientée utilisateur est le type avec lequel vous
travaillerez dans les handlers ; le `todo::Model` interne est la forme
SeaORM que récupère la liaison de modèle de route.

## 4. Contrôleur

```bash
suprnova make:controller todo
```

Ouvrez `src/controllers/todo.rs` :

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

Quelques points à noter :

- **La liaison de modèle de route est automatique.** Déclarer
  `todo: todo::Model` indique à la macro `#[handler]` de chercher
  `{todo}` dans le chemin de la route, de récupérer la ligne SeaORM
  par clé primaire, et de retourner 404 si elle est absente. Le nom du
  paramètre doit correspondre au placeholder de la route.
- **La macro vous remet `todo::Model` ; la surface Eloquent vit sur
  `Todo`.** Les deux sont reliés par un impl `From` émis par
  `#[suprnova::model]`, si bien que `let todo: Todo = todo.into();`
  est la conversion en une ligne. `Todo` est le type qui porte
  `update`, `delete`, et le reste de l'API orientée utilisateur.
- **`#[request]` couvre la validation.** L'ajouter à une struct génère
  `Deserialize`, `Validate` et `FormRequest` - le framework rejette
  une entrée malformée avec un 422 avant que votre handler ne
  s'exécute. Nul besoin de dériver aussi `InertiaProps` sur un DTO de
  requête ; ce derive est pour les props de page *sortantes*.
- **L'affectation de masse passe par `attrs!`.**
  `Todo::create(attrs! { ... })` et `todo.update(attrs! { ... })`
  passent par le filtre `fillable`, si bien que les champs absents de
  la liste `fillable` du modèle disparaissent silencieusement au lieu
  de contourner le garde-fou.
- **`update` et `delete` consomment `self`.** C'est pourquoi `toggle`
  lit `!todo.completed` dans une variable locale avant d'appeler
  `todo.update(...)`.

Enregistrez le nouveau module de contrôleur dans
`src/controllers/mod.rs` :

```rust
pub mod todo;
```

### Pourquoi Suprnova diverge

En Laravel, le même contrôleur retournerait normalement du JSON pour
une API ou une vue Blade pour une page rendue serveur. Suprnova
retourne des réponses Inertia à la fois pour les chargements initiaux
et les navigations SPA - le framework détecte l'en-tête `X-Inertia` et
sert du HTML ou du JSON en conséquence, sans couche API parallèle.
Vous écrivez vos handlers une seule fois, votre frontend reste une
vraie SPA, et il n'y a pas de second routeur à garder synchronisé. Voir
[Réponses Inertia](frontend-inertia-responses.md) pour la mécanique.

## 5. Routes

`src/routes.rs` :

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

Le placeholder `{todo}` est ce sur quoi s'accroche la liaison de
modèle de route : il doit correspondre au nom du paramètre du handler
(`todo`), et il doit correspondre au type de clé primaire du modèle
SeaORM (ici, `i64`). Le suffixe facultatif `.name(...)` est ce
qu'utilise le générateur de types de route à l'étape suivante pour
construire les helpers frontend.

## 6. Générer les types TypeScript

```bash
suprnova generate-types
```

`generate-types` fait deux choses en une passe :

1. Parcourt chaque struct `#[derive(InertiaProps)]` dans `src/` et les
   écrit dans `frontend/src/types/inertia-props.ts`.
2. Parcourt `src/routes.rs` et écrit des builders d'URL typés pour
   chaque route nommée dans `frontend/src/types/routes.ts`.

Les helpers de route sortent comme un objet imbriqué -
`controllers.todos.toggle({ todo: "1" })` retourne une paire
`{ url, method }` que `Link` et `router` d'Inertia 3 acceptent
directement. Les paramètres de chemin sont typés ; le compilateur
attrape un argument `todo` manquant avant que la page n'atteigne le
navigateur.

Vous n'avez pas à modifier ces fichiers. Relancez
`suprnova generate-types` chaque fois que vous ajoutez ou renommez des
props/routes, ou passez `--watch` pour les garder synchronisés au fil
de l'eau.

## 7. Pages

Chaque page vit sous `frontend/src/pages/Todos/`. Les noms
correspondent aux chaînes que vous passez à `inertia_response!`, donc
`inertia_response!("Todos/Index", ...)` se résout en
`frontend/src/pages/Todos/Index.svelte`.

### Liste

`frontend/src/pages/Todos/Index.svelte` :

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

### Créer

`frontend/src/pages/Todos/Create.svelte` :

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

### Modifier

`frontend/src/pages/Todos/Edit.svelte` :

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

Les starters équivalents React 19 et Vue 3.5 reprennent les mêmes
props via leur propre système de gabarits - le backend ne change pas.

## 8. L'exécuter

```bash
suprnova serve
```

Visitez `http://127.0.0.1:8765/todos`, ajoutez quelques lignes,
basculez-les, modifiez-en une, supprimez-en une autre. Les transitions
de page se font via Inertia - pas de rechargement complet - et chaque
soumission de formulaire valide côté serveur avant que la redirection
n'arrive.

## Ce qui vient de se passer

| Couche | Fichier | Ce qu'elle fait |
|---|---|---|
| Schéma | `src/migrations/m_create_todos_table.rs` | Crée la table `todos` |
| Modèle | `src/models/todo.rs` | La struct `Todo` orientée utilisateur + le module SeaORM interne |
| HTTP | `src/controllers/todo.rs` | Sept `#[handler]`, y compris la liaison de modèle de route |
| Routeur | `src/routes.rs` | Routes nommées qui pilotent les helpers de route générés |
| Props | `frontend/src/types/inertia-props.ts` | Généré depuis `#[derive(InertiaProps)]` |
| Routes | `frontend/src/types/routes.ts` | Généré depuis les routes nommées dans `routes.rs` |
| Pages | `frontend/src/pages/Todos/*.svelte` | Les trois pages Svelte 5 qui consomment les props |

C'est la boucle de fonctionnalité standard de Suprnova :
migration -> modèle -> contrôleur -> route -> page, avec
`suprnova generate-types` qui régénère le pont TypeScript chaque fois
que vous remodelez des props ou renommez une route.

## Suivant

- [Eloquent](eloquent.md) - `attrs!`, le query builder, casts, scopes,
  observateurs
- [Validation](validation.md) - ce que vous donnent `#[request]` et
  `#[derive(Validate)]`
- [Routage](routing.md) - routes nommées, liaison de modèle de route,
  routage de ressource, URL signées
- [Réponses Inertia](frontend-inertia-responses.md) -
  `inertia_response!`, rechargements partiels, props partagées
- [Authentification](authentication.md) - ajouter des todos par
  utilisateur avec l'auth de session du starter
