# Introdução

Suprnova é um framework web para Rust que oferece a experiência de desenvolvedor
do Laravel sobre Tokio. Você escreve controladores e modelos no estilo Eloquent;
o framework oferece concorrência, segurança de tipos e deploy de binário único.

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0");
    json_response!({ "id": id, "name": "Alice" })
}
```

```rust
use suprnova::{model, Model};

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

// Depois, em qualquer lugar:
let user = User::find(42).await?;
let admins = User::query().db_where("role", "admin").get().await?;
let alice = User::create(attrs!{ name: "Alice", email: "alice@x.com" }).await?;
```

Se você escreveu isso em Laravel semana passada, a versão Rust acima parecerá
idêntica - mesma forma de cadeia, mesmos nomes de método, mesmos padrões. A
diferença é o que acontece embaixo: Tokio em vez de FPM, um binário em vez de
um runtime PHP, verificações de tipo em tempo de compilação em cada coluna.

## Por que Suprnova existe

Laravel resolveu o problema de produtividade no desenvolvimento backend. Os
padrões funcionam. Após dez anos de refinamento, muito pouco fica no seu
caminho ao construir um produto real. Mas o modelo request-per-process do
PHP mantém duas coisas fora do alcance: conexões de longa duração e baratas
(WebSockets, SSE, notificações enviadas pelo servidor sem polling) e I/O
concorrente trivial dentro de um handler de solicitação.

Rust oferece ambos gratuitamente com Tokio. O problema é que o ecossistema
web Rust o força a construir a camada de produtividade você mesmo: escolha
um crate HTTP, escolha uma ORM, escolha uma ferramenta de migração, escolha
uma fila, monte tudo junto, projete suas próprias convenções. Cada app
reinventa o que Laravel já padronizou.

Suprnova é o que acontece quando você copia as convenções do Laravel para
Tokio. Você obtém:

- **Mesma superfície** - `routes!`, `Auth::user()`, `Cache::remember`,
  `Mail::send`, `Queue::push`, `Storage::disk("s3")`, `Notify::send`,
  `Schedule::call`, `Gate::allows`, o construtor de consultas Eloquent,
  soft deletes, factories, observers, transmissão, tudo isso
- **Mecanismo diferente** - async em todo o lugar, conexões de longa duração
  como cidadãos de primeira classe, binário único estaticamente vinculado,
  sem preforking, sem opcache, sem FPM
- **Segurança de tipos** - seus modelos, rotas e cargas de eventos são
  verificados em tempo de compilação; refatorações quebradas não chegam ao
  staging
- **Uma história real de frontend** - Inertia.js conecta Svelte 5, React 19 ou
  starters Vue 3.5, sem API separada para manter

## Princípios de design

Estes são os princípios que os autores do framework se comprometem a seguir.
Eles explicam por que um capítulo diz o que diz.

**1. Paridade vem do changelog do Laravel.** Quando Laravel lança um recurso,
Suprnova o rastreia. A baseline de hoje é Laravel 13.x e todos os subsistemas
enviados foram auditados contra ele. O [Mapa de paridade do Laravel](parity.md)
é a tabela explícita recurso por recurso.

**2. Divirja intencionalmente onde Rust torna as coisas melhores.** Onde Laravel
fez uma escolha em formato PHP que não precisamos fazer em Rust, Suprnova
escolhe a em formato Rust e o diz. O maior exemplo é concorrência: WebSockets,
transmissão, workers em background e HTTP/2 server-push são de primeira classe,
não anexos. Onde você verá isso destacado em um capítulo, procure por caixas
**"Por que Suprnova diverge"**.

**3. Sem gatekeeping.** Laravel restringe alguns recursos a um backend (por
exemplo, busca vetorial via Postgres `pgvector`). Suprnova trata backends como
drivers - `Vector::driver("qdrant")`, `Vector::driver("pinecone")`,
`Vector::driver("mariadb")`, `Cache::driver("redis")`, `Mail::driver("ses")`.
Você escolhe a ferramenta certa; não escolhemos para você.

**4. Suprnova é a superfície da API.** Internamente usamos SeaORM, hyper, Tokio,
serde, sqlx, validator, lettre e muitos mais. Nada disso deve aparecer em seu
código. Você depende de `suprnova::*`. Reexportamos tudo que você vai tocar -
incluindo `Entity`, `Column`, `ActiveModel`, `QueryFilter`, etc. do SeaORM -
sob a raiz do framework. A válvula de escape (`use suprnova::sea_orm;`) existe
para o raro caso que a superfície curada não cobre, mas você quase nunca deveria
precisar.

## O que vem na caixa

Um mapa não exaustivo. A lista completa está em [`documentation.md`](documentation.md).

| Área | O que acompanha |
|---|---|
| **HTTP** | Macro `routes!`, controladores, middleware, solicitações, respostas, binding de modelo de rota, URLs assinadas, roteamento de recursos, helpers de redirecionamento, CORS, CSRF, chaves de idempotência, timeout, limitação de taxa, erros estruturados com recuperação de pânico |
| **Banco de dados** | SeaORM sob o capô, multi-driver (Postgres, MySQL, MariaDB, SQLite), migrações, seeders, construtor de consultas, transações com savepoints, split leitura/escrita multi-conexão |
| **Eloquent** | Macro `#[suprnova::model]`, todos os 11 tipos de relação, eager loading, soft deletes, prunable, scopes (local + global), 16 eventos de ciclo de vida, observers, 22 casts embutidos, accessors/mutators, três paginadores, iteração chunk/lazy/cursor, coleções, replicação |
| **Auth** | Guards, middleware, provedores e sessões de navegador do framework; mecanismos Magnetar de senha, passkey, link mágico, OAuth, sessão bearer, bloqueio, lembrar-me, época de autenticação e migração; verificação de e-mail apoiada em provedor; facade de compatibilidade TOTP do framework; macros de política e gates |
| **Frontend** | Ponte Inertia v3, templates iniciais Svelte 5 / React 19 / Vue 3.5, `#[derive(InertiaProps)]` tipado, reloads parciais, geração automática de tipos TypeScript |
| **Background** | Fila com drivers memory/sync/redis/database/null, batches, chains, job middleware, armazém de jobs falhados, binário console `#[command]`/`#[derive(Command)]`, agendador de trait `Task`, trabalho stateful de longa duração `#[workflow]`, trait `Supervisor` com auto-restart catch-panic, command bus, event dispatcher |
| **Tempo real** | Macro `ws!()` para handlers WebSocket tipados, canais de transmissão (público, privado, presença), fanout sea-streamer, server-sent events, web push (VAPID) |
| **Cache e armazenamento** | Drivers de cache Memory, Redis, Database; operações atômicas; cache tagueado; locks de cache; filesystem com drivers fs/memory/s3/azblob/gcs; proteção path-traversal; armazenamento vetorial com múltiplos backends |
| **Mail e notificações** | Trait `Mailable`, drivers SMTP/SES/Mailgun/Postmark/SendGrid/Resend, prévias de arquivos RFC 5322, transportes in-memory/log e `Notifiable` com canais mail/database/broadcast/webpush |
| **Validação e dados** | `#[derive(Validate)]`, form requests, validação async, `#[derive(Data)]` para conjuntos de include de partial-reload, `#[derive(Resource)]` para JSON:API |
| **Pagamentos** | Superfície de provedor genérico (gateway/MoR/redirect-flow), adaptadores de referência para Stripe e Paddle, tabelas espelhadas com idempotência de webhook, componentes de checkout Inertia |
| **Sinalizadores de recurso** | Avaliador de banco de dados, avaliador em cache com TTL, middleware de recurso, propagação de sub-segundo via trait sync |
| **Testes** | `#[suprnova_test]`, `expect!`, `TestDatabase`, fakes para toda superfície externa (Mail, Notify, Queue, Bus, Events, Storage, Http) |
| **CLI** | Scaffolder `suprnova new` (Svelte/React/Vue), dev runner `serve`, `migrate*`, `db:sync`, `db:seed`, geradores `make:*`, `model:prune`, binário console por projeto |

## Prontidão para produção

O framework é production-grade em escopo e testado. A partir do HEAD atual:

- Toda superfície Laravel 13.x nos 30 domínios documentados é enviada
- Todos os problemas levantados por revisão de código independente foram resolvidos
- A suite de testes do workspace passa em toda mudança
- Toda API pública em `framework/src/lib.rs` está documentada - um item público
  não documentado falha na construção

A partir de **v1.0.0** a API pública é estável: apps fixam uma tag de release
(`tag = "v<version>"` - a tag é o release; não há publicação no crates.io), e
uma mudança quebrada só acontece atrás de um bump de versão cuja seção
[CHANGELOG](changelog.md) o diz.

## Escolha um caminho de leitura

| Você é… | Comece com |
|---|---|
| Um desenvolvedor Laravel | [Vindo do Laravel](from-laravel.md) |
| Um desenvolvedor Rust que usou Axum/Actix/Rocket | [Vindo do Rust web](from-rust-web.md) |
| Ambos, ou nenhum, e apenas quer construir | [Instalação](installation.md) → [Início rápido](quickstart.md) |
| Procurando um recurso específico | [`documentation.md`](documentation.md) (o TOC mestre) |
| Se perguntando "Suprnova tem X?" | [Mapa de paridade do Laravel](parity.md) |
