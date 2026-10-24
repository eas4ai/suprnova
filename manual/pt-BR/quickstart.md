# Início rápido

Vamos construir um pequeno aplicativo de "links" - uma única página que lista
URLs com títulos, mais um endpoint de API para postar novos. Isso exercita
roteamento, controladores, um modelo Eloquent, uma migração e uma página
Inertia. Se você conseguir construir isto, você consegue construir qualquer
coisa que Suprnova faz.

Isto assume que você seguiu [Instalação](installation.md) e tem a CLI
`suprnova` no seu `PATH`.

## 1. Faça scaffold

```bash
suprnova new links --frontend svelte --no-interaction
cd links
suprnova migrate
npm install
suprnova serve
```

Abra `http://127.0.0.1:8765`. Você deve ver a página de boas-vindas. Pare
o servidor (`Ctrl+C`) - vamos adicionar um recurso.

## 2. Criar o modelo e a migração

Não há comando dedicado `make:model` - modelos são regenerados
do esquema por `db:sync --regenerate-models` uma vez que a migração
é executada. Comece com a migração:

```bash
suprnova make:migration create_links_table
```

Abra o novo arquivo de migração sob `src/migrations/`:

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

Crie o modelo manualmente em `src/models/link.rs`:

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

Adicione `pub mod link;` a `src/models/mod.rs` para que o novo módulo seja
acessível, então aplique a migração e regenere as entidades:

```bash
suprnova db:sync
```

`db:sync` executa migrações pendentes e regenera as entidades SeaORM. O
passo combinado é o padrão do dev-loop; em produção você usa simples
`suprnova migrate`.

## 3. Adicionar um controlador

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

Registre o módulo do controlador em `src/controllers/mod.rs`:

```rust
pub mod link;
```

## 4. Conectar as rotas

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

## 5. Construir a página Inertia

Crie `frontend/src/pages/Links/Index.svelte` (para o starter Svelte):

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

(Starters React e Vue equivalentes lhe dão a mesma forma com seus
próprios templates - a ponte Inertia é idêntica.)

## 6. Ver funcionando

```bash
suprnova serve
```

Visite `http://127.0.0.1:8765/links`. Adicione um par de links via o formulário.
Eles fazem post para `/links`, o controlador escreve na tabela `links`, e
a solicitação Inertia refetch os props do índice. Nenhuma cola JSON
marshalling - `InertiaProps` derivou o formato de transmissão para você.

## O que acabou de acontecer

Você tocou em oito arquivos. Aqui está o que eles realmente significam:

| Arquivo | Camada | Papel |
|---|---|---|
| `src/migrations/m_create_links_table.rs` | Esquema | Define a tabela `links` |
| `src/models/link.rs` | Domínio | Um struct, quatro linhas, modelo Eloquent completo |
| `src/controllers/link.rs` | HTTP | Dois handlers: `index` (página) e `store` (criar) |
| `src/routes.rs` | Roteador | Conecta URLs aos handlers via `routes!` |
| `src/controllers/mod.rs` | Integração | Re-exporta o novo módulo do controlador |
| `frontend/src/pages/Links/Index.svelte` | Frontend | A página que Inertia renderiza |
| (existente) `bootstrap.rs` | Inicialização | Onde você registraria observadores/serviços para este recurso |
| (existente) `.env` | Configuração | URL do BD, portas, segredos |

Esse é o ritmo padrão: migração - modelo - controlador -
rota - página frontend. Cada recurso, não importa o quão grande, se decompõe
nesses passos.

## O que ler a seguir

Você fez um corte vertical completo. Os próximos itens que você vai alcançar são:

- [Roteamento](routing.md) - agrupamento, middleware, rotas nomeadas, URLs
  assinadas, roteamento de recursos
- [Validação](validation.md) - o que `#[derive(Validate)]` oferece
- [Eloquent](eloquent.md) - relacionamentos, escopos, observadores, exclusões
  suaves, a superfície completa do construtor de consultas
- [Inertia + Frontend](frontend.md) - recarregamentos parciais, props tipadas,
  geração de tipos TypeScript
- [Autenticação](authentication.md) - o scaffold de auth que o starter
  forneceu
- [Console](console.md) - `cargo run --bin console <subcommand>` e
  escrevendo seus próprios comandos

Ou navegue em [`documentation.md`](documentation.md) para o sumário completo.
