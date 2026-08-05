# Estrutura de diretórios

Quando você executa `suprnova new my-app --frontend svelte`, o gerador de scaffold
oferece isto:

```
my-app/
├── Cargo.toml                      # manifesto do crate + dependências, dois [[bin]] targets
├── .env                            # configuração local - URL do BD, chave de app, portas
├── .env.example                    # modelo para ops/CI
├── .gitignore                      # exclui target/, .env, node_modules/, public/assets/
├── cmd/
│   └── main.rs                     # entrada do binário; chama Application::new().run()
├── src/
│   ├── lib.rs                      # organização de módulos (`pub mod controllers;` etc.)
│   ├── bootstrap.rs                # registra serviços, observers, listeners - o
│   │                               # análogo Suprnova dos service providers do Laravel
│   ├── routes.rs                   # a árvore da macro `routes!` - todas as URLs que o app serve
│   ├── bin/
│   │   └── console.rs              # entrada de `cargo run --bin console <subcommand>` -
│   │                               # o análogo Suprnova do `php artisan`
│   ├── actions/
│   │   ├── mod.rs
│   │   └── example_action.rs       # controladores invocáveis de um único método
│   ├── commands/
│   │   └── mod.rs                  # handlers anotados com `#[command]` registram-se aqui
│   ├── config/
│   │   ├── mod.rs
│   │   ├── database.rs             # configuração de BD tipada (driver, URL, pool)
│   │   └── mail.rs                 # configuração de mail tipada
│   ├── controllers/
│   │   ├── mod.rs
│   │   ├── home.rs                 # handler GET /
│   │   ├── auth.rs                 # login / registro / logout
│   │   └── dashboard.rs            # requer autenticação; rota protegida de exemplo
│   ├── middleware/
│   │   ├── mod.rs
│   │   ├── logging.rs              # logging de solicitação/resposta
│   │   └── authenticate.rs         # guard de autenticação baseado em sessão
│   ├── migrations/
│   │   ├── mod.rs
│   │   ├── m_*_create_users_table.rs
│   │   ├── m_*_create_sessions_table.rs
│   │   ├── m_*_create_remember_tokens_table.rs
│   │   ├── m_*_create_workflows_table.rs
│   │   └── m_*_create_workflow_steps_table.rs
│   └── models/
│       ├── mod.rs
│       └── user.rs                 # modelo `#[suprnova::model]` User
├── frontend/
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── index.html                  # entrada Vite; monta o SPA
│   └── src/
│       ├── main.{tsx,ts}           # configuração do cliente Inertia (por framework)
│       ├── app.css                 # estilos globais + Tailwind
│       ├── pages/
│       │   ├── Home.{tsx,svelte,vue}
│       │   ├── Dashboard.{tsx,svelte,vue}
│       │   └── auth/
│       │       ├── Login.{tsx,svelte,vue}
│       │       └── Register.{tsx,svelte,vue}
│       └── types/
│           └── inertia-props.ts    # auto-gerado a partir de #[derive(InertiaProps)]
└── public/
    └── assets/                     # saída do build de produção do Vite fica aqui
```

Svelte adiciona `frontend/svelte.config.js` e `frontend/src/app.d.ts`.
Vue adiciona `frontend/src/shims-vue.d.ts`.

O starter de API (`suprnova new my-api --api`) é mais enxuto: sem
`frontend/`, sem controladores de autenticação, e `cmd/main.rs` é substituído por
`src/main.rs`.

## Para que serve cada diretório

### `cmd/main.rs`

O ponto de entrada do binário. Um arquivo curto - normalmente 10-20 linhas - que
chama o pipeline de inicialização padrão:

```rust
use suprnova::Application;
use my_app::{bootstrap, config, migrations, routes};

#[suprnova::main]
async fn main() {
    Application::new()
        .config(config::register_all)
        .bootstrap(bootstrap::register)
        .routes(routes::register)
        .migrations::<migrations::Migrator>()
        .run()
        .await;
}
```

`Application::run()` analisa a CLI do binário (`serve` / `web:run` /
`migrate*` / `schedule:*` / `workflow:work` / `queue:work`), carrega
`.env`, executa sua função de configuração, depois despacha o subcomando. O
caminho serve também executa sua função bootstrap e inicia o servidor
HTTP.

Você quase nunca edita `cmd/main.rs` após o scaffold inicial.

### `src/lib.rs`

Um arquivo de declaração de módulos simples:

```rust
pub mod actions;
pub mod bootstrap;
pub mod commands;
pub mod config;
pub mod controllers;
pub mod middleware;
pub mod migrations;
pub mod models;
pub mod routes;
```

Isto é o que faz `crate::controllers::home::index` acessível a partir de
`routes.rs`.

### `src/bootstrap.rs`

A função única que conecta seu app. Você registra bindings do contêiner de
serviços, observers, listeners de eventos, middleware customizado, e qualquer outra
configuração de tempo de inicialização aqui. É o análogo do `AppServiceProvider`,
`EventServiceProvider`, `BroadcastServiceProvider`, etc. do Laravel, tudo em um
arquivo:

```rust
use std::sync::Arc;
use suprnova::App;

pub async fn register() {
    // Vincula um serviço no contêiner
    App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));

    // Registra um observer Eloquent
    crate::models::user::register_observer();

    // Ouve eventos
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
}
```

`register()` executa uma vez por processo, depois do carregador de configuração mas antes
de `serve` aceitar a primeira solicitação. Workers (`queue:work`,
`schedule:run`, `workflow:work`) reutilizam o mesmo bootstrap para que vejam
os mesmos serviços. Veja [Inicialização da aplicação](bootstrap.md).

### `src/routes.rs`

Sua superfície de URLs. A macro `routes!` no nível de módulo expande
um `pub fn register() -> Router` que `cmd/main.rs` passa para
`Application::routes(...)`:

```rust
use suprnova::{get, post, put, delete, routes};
use crate::{controllers, middleware};

routes! {
    get!("/", controllers::home::index).name("home"),

    // Autenticação (registrado + protegido)
    get!("/login", controllers::auth::show_login).name("login.show"),
    post!("/login", controllers::auth::login).name("login.attempt"),
    post!("/logout", controllers::auth::logout).name("logout"),
    get!("/register", controllers::auth::show_register).name("register.show"),
    post!("/register", controllers::auth::register).name("register"),

    // Dashboard requer middleware de autenticação
    get!("/dashboard", controllers::dashboard::index)
        .middleware(middleware::authenticate::auth())
        .name("dashboard"),
}
```

Veja [Roteamento](routing.md).

### `src/bin/console.rs`

Seu binário de console por projeto. Executa como `cargo run --bin console
<subcommand>` e despacha o `db:seed` embutido do framework mais
cada handler anotado com `#[command]` (ou struct tipado `#[derive(Command)]`)
em `src/commands/` - ambas as formas se registram através de inventory em
tempo de compilação:

```bash
cargo run --bin console db:seed           # embutido do framework
cargo run --bin console report:daily      # seu comando customizado
```

Os workers de longa duração (`queue:work`, `schedule:run`,
`schedule:work`, `workflow:work`) vivem no binário principal do app
porque `Application::run()` os despacha - chame-os como
`cargo run -- queue:work` (ou via `suprnova schedule:run` /
`suprnova workflow:work` se preferir a CLI guarda-chuva).

Veja [Console](console.md).

### `src/commands/`

Onde seus handlers de console vivem. Dois estilos: uma struct tipada com
args derivados de clap e `impl TypedCommand`, ou um `#[command]` puro em um
`async fn(Vec<String>) -> Result<(), FrameworkError>`. O scaffolder
gera a forma tipada:

```rust
use async_trait::async_trait;
use clap::Parser;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(Parser, Command, Debug)]
#[console(name = "report:daily", description = "Gera o relatório diário")]
pub struct DailyReport {
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
}

#[async_trait]
impl TypedCommand for DailyReport {
    async fn run(self) -> Result<(), FrameworkError> {
        // …
        Ok(())
    }
}
```

`suprnova make:command report-daily` faz scaffold do arquivo e adiciona-o a
`src/commands/mod.rs`. Veja [Console](console.md).

### `src/config/`

Structs de configuração tipadas. O scaffold oferece `database.rs` e
`mail.rs`; adicione o seu próprio para qualquer subsistema que seu app se importe. Cada
struct de configuração lê seus valores do ambiente, e
`config::register_all()` os registra com o framework:

```rust
use suprnova::{env, env_required};

#[derive(Clone, Debug)]
pub struct AnalyticsConfig {
    pub api_key: String,
    pub max_batch: u32,
}

impl AnalyticsConfig {
    pub fn from_env() -> Self {
        Self {
            api_key: env_required::<String>("ANALYTICS_API_KEY"),
            max_batch: env("ANALYTICS_MAX_BATCH", 100u32),
        }
    }
}
```

Conecte-a em `config/mod.rs`:

```rust
use suprnova::Config;

pub fn register_all() {
    Config::register(AnalyticsConfig::from_env());
}
```

Veja [Configuração](configuration.md).

### `src/controllers/`

Funções handler HTTP. Um módulo por recurso. Cada `pub async fn`
que aceita um `Request` e retorna uma `Response` é chamável a partir de uma
rota.

### `src/middleware/`

Implementações de middleware. O scaffold oferece `logging` e
`authenticate`; você adiciona o seu próprio aqui como `pub struct Foo` com
`impl Middleware for Foo`. Registre-os globalmente em `bootstrap.rs`
ou aplique por rota via `.middleware(…)` na árvore `routes!`. Veja
[Middleware](middleware.md).

### `src/migrations/`

Migradores SeaORM. O scaffold oferece alguns para as tabelas de autenticação +
workflow. `suprnova make:migration <name>` adiciona um novo. `suprnova
migrate`, `migrate:rollback`, `migrate:status`, `migrate:fresh`,
`db:sync` todos operam neste diretório. Veja [Migrações](migrations.md).

### `src/models/`

Seus modelos Eloquent. Um arquivo por modelo, cada um uma struct `#[suprnova::model]`.
O scaffold oferece `user.rs`; adicione novos modelos escrevendo um novo
arquivo à mão ou executando `suprnova db:sync --regenerate-models` após uma
migração de schema. Veja [Eloquent](eloquent.md).

### `src/actions/`

Controladores invocáveis de um único método. Padrão opcional - use-os quando
um controlador teria exatamente um método e você preferiria chamá-lo
"Action" em vez de envolvê-lo. O scaffold oferece um exemplo que você pode deletar ou
adaptar. Veja [Ações](actions.md).

### `frontend/`

A SPA Vite + Inertia. Este é um projeto frontend normal - `package.json`,
`vite.config.ts`, `tsconfig.json`, uma entrada Vite `index.html`, código-fonte
sob `src/`. A configuração do cliente Inertia fica em `src/main.{tsx,ts}` e
os componentes de página em `src/pages/`. Tipos TypeScript para seus props
Rust `#[derive(InertiaProps)]` são regenerados em
`src/types/inertia-props.ts` por `suprnova generate-types`.

Veja [Frontend](frontend.md).

### `public/assets/`

Onde o Vite coloca o build de produção (`npm run build`). O servidor Suprnova
serve este diretório como ativos estáticos em `/assets/*` em
produção.

## Diretórios que você adicionará conforme o app cresce

O scaffold oferece o mínimo - o suficiente para enviar o fluxo de boas-vindas
e um painel protegido. Apps reais crescem mais subsistemas. Adições
comuns:

| Diretório | Quando você adiciona |
|---|---|
| `src/jobs/` | Primeira vez que você `Queue::push(SomeJob)`. Veja [Filas](queues.md). |
| `src/listeners/` | Primeira vez que você `Event::listen`. Veja [Eventos](events.md). |
| `src/observers/` | Primeira vez que você implementa `Observer<MyModel>`. Veja [Eloquent](eloquent.md#observers). |
| `src/notifications/` | Primeira vez que você implementa uma `Notification`. Veja [Notificações](notifications.md). |
| `src/mail/` | Primeira vez que você implementa uma `Mailable`. Veja [Correio](mail.md). |
| `src/policies/` | Primeira vez que você escreve um `#[policy]`. Veja [Autorização](authorization.md). |
| `src/factories/` | Primeira vez que você escreve uma `Factory<Model>` para testes. Veja [Factories Eloquent](eloquent-factories.md). |
| `src/seeders/` | Primeira vez que você escreve um `Seeder` para `db:seed`. Veja [Preenchimento de dados](seeding.md). |
| `src/events/` | Primeira vez que você `impl Event` para seu próprio tipo de evento. Veja [Eventos](events.md). |
| `src/broadcasting/` | Primeira vez que você define um `Channel` privado/presença. Veja [Transmissão](broadcasting.md). |
| `src/ws/` | Primeira vez que você escreve um handler `ws!()`. Veja [WebSockets](websockets.md). |
| `src/supervisors/` | Primeira vez que você implementa um `Supervisor` de longa duração. Veja [Supervisores](supervisors.md). |
| `src/payments/` | Primeira vez que você conecta Stripe/Paddle para seu app. Veja [Pagamentos](payments.md). |
| `src/props/` | Quando você quer manter structs `#[derive(InertiaProps)]` separados de controladores. |
| `resources/views/` | Primeira vez que você adiciona um template Tera para corpos de mail. |
| `storage/` | Primeira vez que você escreve arquivos para o disco do sistema de arquivos local (veja [Armazenamento de arquivos](filesystem.md)). |
| `tests/` | Primeira vez que você escreve um teste de integração. |

Você não precisa pedir permissão - `mkdir src/jobs` e adicione
`pub mod jobs;` a `src/lib.rs`, e pronto. O framework
não força os nomes de diretório; as convenções existem para que outros
desenvolvedores Suprnova possam encontrar as coisas rapidamente.

## O `app/` dogfood neste repo

Se você está lendo isto de dentro do repo Suprnova em si, você
verá um diretório `app/` na raiz que usa todos os recursos do framework
juntos. Este é nosso banco de testes interno - ele exercita pagamentos,
transmissão, web push, fluxos de trabalho, supervisores, etc. tudo de uma vez. Não
é uma referência limpa para um novo app; a saída de scaffold acima é
deliberadamente menor e mais fácil de aprender. Leia `app/` uma vez que
você quer ver um exemplo máximo de como os pedaços se compõem.

## Próximos passos

- [Configuração](configuration.md) - como `.env` se torna configuração tipada
- [Inicialização da aplicação](bootstrap.md) - o que `bootstrap.rs` realmente
  faz
- [Roteamento](routing.md) - sua primeira rota
- [Contêiner de serviços](container.md) - como `App::bind` e `App::get`
  funcionam
