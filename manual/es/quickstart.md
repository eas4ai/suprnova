# Inicio rápido

Vamos a construir una pequeña aplicación de "enlaces" - una página única que
lista URLs con títulos, más un punto final de API para publicar nuevos.
Ejercita el enrutamiento, controladores, un modelo Eloquent, una migración
y una página de Inertia. Si puedes construir esto, puedes construir cualquier
cosa que Suprnova puede hacer.

Esto asume que has seguido [Instalación](installation.md) y tienes la CLI
`suprnova` en tu `PATH`.

## 1. Andamiar

```bash
suprnova new links --frontend svelte --no-interaction
cd links
suprnova migrate
npm install
suprnova serve
```

Abre `http://127.0.0.1:8765`. Deberías ver la página de bienvenida. Detén
el servidor (`Ctrl+C`) - vamos a añadir una característica.

## 2. Crear el modelo y la migración

No hay un comando dedicado `make:model` - los modelos se regeneran desde el
esquema mediante `db:sync --regenerate-models` una vez que se ejecuta la
migración. Comienza con la migración:

```bash
suprnova make:migration create_links_table
```

Abre el nuevo archivo de migración bajo `src/migrations/`:

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

Crea el modelo a mano en `src/models/link.rs`:

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

Añade `pub mod link;` a `src/models/mod.rs` para que el nuevo módulo sea
accesible, luego aplica la migración y regenera las entidades:

```bash
suprnova db:sync
```

`db:sync` ejecuta las migraciones pendientes y regenera las entidades de SeaORM.
El paso combinado es el valor predeterminado del bucle de desarrollo; en producción
usas simple `suprnova migrate`.

## 3. Añadir un controlador

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

Registra el módulo del controlador en `src/controllers/mod.rs`:

```rust
pub mod link;
```

## 4. Conectar las rutas

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

## 5. Construir la página de Inertia

Crea `frontend/src/pages/Links/Index.svelte` (para el iniciador de Svelte):

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

(Los iniciadores equivalentes de React y Vue te dan la misma forma con su
propio templating - el puente de Inertia es idéntico.)

## 6. Verlo funcionar

```bash
suprnova serve
```

Visita `http://127.0.0.1:8765/links`. Añade un par de enlaces a través del
formulario. Se publican en `/links`, el controlador escribe en la tabla `links`,
y la solicitud de Inertia vuelve a obtener los props del índice. Sin pegamento
de serialización JSON - `InertiaProps` derivó el formato de respuesta para ti.

## Lo que acaba de suceder

Tocaste ocho archivos. Esto es lo que realmente significan:

| Archivo | Capa | Rol |
|---|---|---|
| `src/migrations/m_create_links_table.rs` | Esquema | Define la tabla `links` |
| `src/models/link.rs` | Dominio | Una estructura, cuatro líneas, modelo Eloquent completo |
| `src/controllers/link.rs` | HTTP | Dos handlers: `index` (página) y `store` (crear) |
| `src/routes.rs` | Enrutador | Conecta URLs a handlers vía `routes!` |
| `src/controllers/mod.rs` | Conexión | Re-exporta el nuevo módulo del controlador |
| `frontend/src/pages/Links/Index.svelte` | Frontend | La página que Inertia renderiza |
| (existente) `bootstrap.rs` | Arranque | Dónde registrarías observadores/servicios para esta característica |
| (existente) `.env` | Configuración | URL de BD, puertos, secretos |

Ese es el ritmo estándar: migración → modelo → controlador → ruta → página
frontend. Cada característica, sin importar qué tan grande, se descompone en
esos pasos.

## Qué leer a continuación

Has hecho un corte vertical completo. Las próximas cosas que querrás:

- [Enrutamiento](routing.md) - agrupación, middleware, rutas nombradas,
  URLs firmadas, enrutamiento de recursos
- [Validación](validation.md) - lo que `#[derive(Validate)]` te proporciona
- [Eloquent](eloquent.md) - relaciones, alcances, observadores, eliminación
  suave, la superficie completa del constructor de consultas
- [Inertia + Frontend](frontend.md) - recargues parciales, props tipados,
  generación automática de tipos de TypeScript
- [Autenticación](authentication.md) - el andamiaje de autenticación que
  vino con el iniciador
- [Consola](console.md) - `cargo run --bin console <subcommand>` y
  escribir tus propios comandos

O examina [`documentation.md`](documentation.md) para la TOC completa.
