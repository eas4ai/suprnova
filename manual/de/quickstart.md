# Schnellstart

Wir werden eine kleine "links"-App bauen - eine einzelne Seite, die URLs
mit Titeln auflistet, sowie einen API-Endpunkt zum Hinzufügen neuer Links.
Dies übt Routing, Controller, ein Eloquent-Modell, eine Migration und eine
Inertia-Seite. Falls Sie dies bauen können, können Sie alles bauen, was
Suprnova tut.

Dies setzt voraus, dass Sie [Installation](installation.md) gefolgt sind und
die `suprnova` CLI auf Ihrem `PATH` haben.

## 1. Scaffold

```bash
suprnova new links --frontend svelte --no-interaction
cd links
suprnova migrate
npm install
suprnova serve
```

Öffnen Sie `http://127.0.0.1:8765`. Sie sollten die Willkommensseite sehen.
Stoppen Sie den Server (`Ctrl+C`) - wir werden ein Feature hinzufügen.

## 2. Modell und Migration erstellen

Es gibt keinen dedizierten `make:model` Befehl - Modelle werden aus dem Schema
von `db:sync --regenerate-models` neu generiert, sobald die Migration läuft.
Beginnen Sie mit der Migration:

```bash
suprnova make:migration create_links_table
```

Öffnen Sie die neue Migrationsdatei unter `src/migrations/`:

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

Erstellen Sie das Modell von Hand bei `src/models/link.rs`:

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

Fügen Sie `pub mod link;` zu `src/models/mod.rs` hinzu, damit das neue Modul
erreichbar ist, wenden Sie dann die Migration an und regenerieren Sie die Entitäten:

```bash
suprnova db:sync
```

`db:sync` führt ausstehende Migrationen aus und regeneriert SeaORM-Entitäten.
Der kombinierte Schritt ist der Standard für die Entwicklungsschleife; in
der Produktion verwenden Sie einfach `suprnova migrate`.

## 3. Controller hinzufügen

`src/controllers/link.rs`:

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

Registrieren Sie das Controller-Modul in `src/controllers/mod.rs`:

```rust
pub mod link;
```

## 4. Routen verdrahten

`src/routes.rs`:

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

## 5. Inertia-Seite erstellen

Erstellen Sie `frontend/src/pages/Links/Index.svelte` (für den Svelte-Starter):

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

(Äquivalente React- und Vue-Starter geben Ihnen die gleiche Form mit ihrem
eigenen Templating - die Inertia-Verbindung ist identisch.)

## 6. Es funktioniert

```bash
suprnova serve
```

Besuchen Sie `http://127.0.0.1:8765/links`. Fügen Sie ein Paar Links über
das Formular hinzu. Sie posten zu `/links`, der Controller schreibt in die
`links`-Tabelle, und die Inertia-Anfrage ruft die Index-Props neu ab. Kein
JSON-Marshalling-Glue - `InertiaProps` leitete das Drahtformat für Sie ab.

## Was ist gerade passiert

Sie haben acht Dateien berührt. Hier ist, was sie tatsächlich bedeuten:

| Datei | Schicht | Rolle |
|---|---|---|
| `src/migrations/m_create_links_table.rs` | Schema | Definiert die `links`-Tabelle |
| `src/models/link.rs` | Domain | Eine Struktur, vier Zeilen, vollständiges Eloquent-Modell |
| `src/controllers/link.rs` | HTTP | Zwei Handler: `index` (Seite) und `store` (erstellen) |
| `src/routes.rs` | Router | Verdrahtet URLs mit Handlern über `routes!` |
| `src/controllers/mod.rs` | Verdrahtung | Re-exportiert das neue Controller-Modul |
| `frontend/src/pages/Links/Index.svelte` | Frontend | Die Seite, die Inertia rendert |
| (existiert) `bootstrap.rs` | Boot | Wo Sie Observer/Services für dieses Feature registrieren würden |
| (existiert) `.env` | Konfiguration | DB-URL, Ports, Geheimnisse |

Das ist der Standard-Rhythmus: Migration - Modell - Controller -
Route - Frontend-Seite. Jede Funktion, egal wie groß, wird in diese
Schritte zerlegt.

## Was Sie als Nächstes lesen sollten

Sie haben einen vollständigen vertikalen Schnitt durchgeführt. Die nächsten
Dinge, die Sie benötigen:

- [Routing](routing.md) - Gruppierung, Middleware, benannte Routen, signierte
  URLs, Resource Routing
- [Validierung](validation.md) - was `#[derive(Validate)]` Ihnen gibt
- [Eloquent](eloquent.md) - Beziehungen, Scopes, Observer, Soft Deletes,
  die gesamte Query Builder Oberfläche
- [Inertia + Frontend](frontend.md) - Partial Reloads, typisierte Props,
  TypeScript-Typengenerierung
- [Authentifizierung](authentication.md) - das Auth-Scaffold, das der Starter
  mitgebracht hat
- [Konsole](console.md) - `cargo run --bin console <subcommand>` und
  Schreiben eigener Befehle

Oder durchsuchen Sie [`documentation.md`](documentation.md) nach der vollständigen Inhaltsangabe.
