# Démarrage rapide

Nous allons construire une petite application « liens » - une seule page qui
répertorie les URL avec des titres, plus un endpoint API pour en poster de
nouvelles. Elle exerce le routage, les contrôleurs, un modèle Eloquent, une
migration et une page Inertia. Si vous pouvez construire cela, vous pouvez
construire n'importe quoi que Suprnova peut faire.

Ceci suppose que vous avez suivi [Installation](installation.md) et que vous
avez l'interface de ligne de commande `suprnova` sur votre `PATH`.

## 1. Créer la structure du projet

```bash
suprnova new links --frontend svelte --no-interaction
cd links
suprnova migrate
npm install
suprnova serve
```

Ouvrez `http://127.0.0.1:8765`. Vous devriez voir la page de bienvenue.
Arrêtez le serveur (`Ctrl+C`) - nous allons ajouter une fonctionnalité.

## 2. Créer le modèle et la migration

Il n'y a pas de commande `make:model` dédiée - les modèles sont régénérés
à partir du schéma par `db:sync --regenerate-models` une fois que la migration
s'exécute. Commencez par la migration :

```bash
suprnova make:migration create_links_table
```

Ouvrez le nouveau fichier de migration sous `src/migrations/` :

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(
            Table::create()
                .table(Alias::new("links"))
                .if_not_exists()
                .col(ColumnDef::new(Alias::new("id"))
                    .big_integer().primary_key().auto_increment().not_null())
                .col(ColumnDef::new(Alias::new("title")).string().not_null())
                .col(ColumnDef::new(Alias::new("url")).string().not_null())
                .col(ColumnDef::new(Alias::new("created_at"))
                    .timestamp_with_time_zone().not_null().default(Expr::current_timestamp()))
                .col(ColumnDef::new(Alias::new("updated_at"))
                    .timestamp_with_time_zone().not_null().default(Expr::current_timestamp()))
                .to_owned()
        ).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new("links")).to_owned()).await
    }
}
```

Créez le modèle à la main à `src/models/link.rs` :

```rust
use chrono::{DateTime, Utc};
use suprnova::{model, Model};

#[model(table = "links")]
pub struct Link {
    pub id: i64,
    pub title: String,
    pub url: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
```

Ajoutez `pub mod link;` à `src/models/mod.rs` pour que le nouveau module soit
accessible, puis appliquez la migration et régénérez les entités :

```bash
suprnova db:sync
```

`db:sync` exécute les migrations en attente et régénère les entités SeaORM.
L'étape combinée est la valeur par défaut de la boucle de développement ; en
production, vous utilisez simplement `suprnova migrate`.

## 3. Ajouter un contrôleur

`src/controllers/link.rs` :

```rust
use suprnova::{
    Data, InertiaProps, Model, Request, Response,
    handler, inertia_response, json_response,
};
use validator::Validate;
use crate::models::Link;

#[derive(InertiaProps)]
pub struct IndexProps {
    pub links: Vec<Link>,
}

pub async fn index(req: Request) -> Response {
    let links = Link::query().order_by_desc("created_at").get().await?;
    inertia_response!(&req, "Links/Index", IndexProps { links: links.into_vec() })
}

#[derive(Data, Validate)]
pub struct CreateLink {
    #[validate(length(min = 1, max = 200))]
    pub title: String,
    #[validate(url)]
    pub url: String,
}

#[handler]
pub async fn store(input: CreateLink) -> Response {
    let link = Link::create(suprnova::attrs! {
        title: input.title,
        url: input.url,
    }).await?;
    json_response!({ "link": link })
}
```

Enregistrez le module du contrôleur dans `src/controllers/mod.rs` :

```rust
pub mod link;
```

## 4. Câbler les routes

`src/routes.rs` :

```rust
use suprnova::{get, post, routes};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),

    // Links
    get!("/links", controllers::link::index).name("links.index"),
    post!("/links", controllers::link::store).name("links.store"),
}
```

## 5. Construire la page Inertia

Créez `frontend/src/pages/Links/Index.svelte` (pour le starter Svelte) :

```svelte
<script lang="ts">
    import { router } from '@inertiajs/svelte';

    let { links } = $props<{
        links: { id: number; title: string; url: string }[]
    }>();

    let title = $state('');
    let url = $state('');

    function submit(e: SubmitEvent) {
        e.preventDefault();
        router.post('/links', { title, url }, {
            onSuccess: () => { title = ''; url = ''; },
        });
    }
</script>

<div class="mx-auto max-w-2xl p-8">
    <h1 class="text-2xl font-bold">Links</h1>

    <form onsubmit={submit} class="mt-4 flex gap-2">
        <input bind:value={title} placeholder="Title"
               class="flex-1 rounded border p-2" />
        <input bind:value={url} placeholder="https://..."
               class="flex-1 rounded border p-2" />
        <button class="rounded bg-blue-600 px-4 py-2 text-white">Add</button>
    </form>

    <ul class="mt-8 space-y-2">
        {#each links as link}
            <li class="rounded border p-3">
                <a href={link.url} target="_blank"
                   class="text-blue-600 hover:underline">
                    {link.title}
                </a>
                <p class="text-sm text-gray-500">{link.url}</p>
            </li>
        {/each}
    </ul>
</div>
```

(Les starters équivalents React et Vue vous donnent la même structure avec
leurs propres modèles - le pont Inertia est identique.)

## 6. Voir ça fonctionner

```bash
suprnova serve
```

Visitez `http://127.0.0.1:8765/links`. Ajoutez quelques liens via le
formulaire. Ils publient vers `/links`, le contrôleur écrit dans la table
`links`, et la requête Inertia récupère à nouveau les props d'index. Aucune
colle de sérialisation JSON - `InertiaProps` a dérivé le format filaire pour
vous.

## Ce qui vient de se passer

Vous avez touché à huit fichiers. Voici ce qu'ils signifient réellement :

| Fichier | Couche | Rôle |
|---|---|---|
| `src/migrations/m_create_links_table.rs` | Schéma | Définit la table `links` |
| `src/models/link.rs` | Domaine | Une struct, quatre lignes, modèle Eloquent complet |
| `src/controllers/link.rs` | HTTP | Deux handlers : `index` (page) et `store` (créer) |
| `src/routes.rs` | Routeur | Câble les URL aux handlers via `routes!` |
| `src/controllers/mod.rs` | Câblage | Réexporte le nouveau module contrôleur |
| `frontend/src/pages/Links/Index.svelte` | Frontend | La page qu'Inertia rend |
| (existant) `bootstrap.rs` | Démarrage | Où vous enregistreriez les observateurs/services pour cette fonctionnalité |
| (existant) `.env` | Configuration | URL BD, ports, secrets |

C'est le rythme standard : migration - modèle - contrôleur -
route - page frontend. Chaque fonctionnalité, peu importe sa taille, se
décompose en ces étapes.

## Quoi lire ensuite

Vous avez fait une tranche verticale complète. Les prochaines choses que vous
utiliserez :

- [Routage](routing.md) - groupement, middleware, routes nommées, URL
  signées, routage des ressources
- [Validation](validation.md) - ce que `#[derive(Validate)]` vous donne
- [Eloquent](eloquent.md) - relations, portées, observateurs,
  suppressions logicielles, toute la surface du constructeur de requêtes
- [Inertia - Frontend](frontend.md) - rechargements partiels, props typées,
  génération de types TypeScript
- [Authentification](authentication.md) - le scaffolding d'authentification
  que le starter a livré
- [Console](console.md) - `cargo run --bin console <subcommand>` et
  écrire vos propres commandes

Ou parcourez [`documentation.md`](documentation.md) pour la table des matières
complète.
