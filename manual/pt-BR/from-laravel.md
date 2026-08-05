# Vindo do Laravel

Se você despachou aplicações Laravel, você já conhece 80% do Suprnova. Este
capítulo mapeia seus hábitos para o equivalente em Rust para que você possa
ser produtivo rapidamente. Vamos mostrar os padrões que você usa diariamente,
os padrões que mudam de forma, e as poucas coisas que Rust oferece gratuitamente
que PHP não consegue.

## Resumo lado a lado

| Você escreveu em Laravel | Você escreve em Suprnova |
|---|---|
| `composer create laravel/laravel my-app` | `suprnova new my-app --frontend svelte` |
| `php artisan serve` | `suprnova serve` |
| `php artisan migrate` | `suprnova migrate` |
| `php artisan make:controller PostController` | `suprnova make:controller post` |
| `Route::get('/posts/{id}', [PostController::class, 'show'])` | `get!("/posts/{id}", controllers::post::show)` (in `routes!`) |
| `class Post extends Model` | `#[suprnova::model] struct Post { … }` |
| `Post::find($id)` | `Post::find(id).await?` |
| `Post::where('status', 'published')->get()` | `Post::query().db_where("status", "published").get().await?` |
| `Auth::user()` | `Auth::user().await?` |
| `Cache::remember('key', 60, fn() => …)` | `Cache::remember("key", Some(Duration::from_secs(60)), \|\| async { … }).await?` |
| `Queue::push(new SendEmail($user))` | `Queue::push(SendEmail { user_id }).await?` |
| `Mail::to($u)->send(new Welcome($u))` | `Mail::to(&u.email).send(WelcomeMail { user: u }).await?` |
| `Storage::disk('s3')->put($path, $bytes)` | `Storage::disk("s3")?.put(&path, bytes).await?` |
| `Notification::send($u, new Invoice($i))` | `Notify::send(&u, &InvoiceNotification { invoice }).await?` |
| `Gate::allows('update', $post)` | `Gate::allows::<PostPolicy, _>("update", &user, &post).await?` |
| `request()->validate([...])` | `#[handler]` extracts an `#[derive(Data, Validate)]` arg directly |
| `event(new OrderShipped($order))` | `EventFacade::dispatch(OrderShipped { order }).await?` |
| `Bus::dispatch(new ProcessFoo($x))` | `Bus::dispatch(ProcessFoo { x }).await?` |
| `php artisan schedule:list` | `suprnova schedule:list` |
| `php artisan tinker` | (sem REPL - escreva um script ou teste `cargo run` pontual) |
| `composer require league/csv` | `cargo add csv` |

## Mudança no modelo mental

### Async em todo lugar

A maior mudança: toda chamada de banco de dados, chamada HTTP, I/O de arquivo,
chamada de cache, push de fila - qualquer coisa que cruze um limite - é `async`
e você a chama com `.await?`. Depois de fazer isso por algumas horas, desaparece
no ritmo. Até então, o compilador apontará cada lugar que você esqueceu.

```rust
// Laravel
$user = User::find($id);
$user->subscribe($plan);
Mail::to($user)->send(new Welcome($user));

// Suprnova
let user = User::find(id).await?;
user.subscribe(&plan).await?;
Mail::to(&user.email).send(WelcomeMail { user }).await?;
```

`?` é o "retorno antecipado em erro" do Rust. Um handler retorna
`Result<HttpResponse, HttpResponse>` (aliasado como `Response`), então um `?`
em um erro de banco de dados faz um atalho para seu conversor de erros e o cliente
obtém um 500 apropriado (ou 4xx, dependendo do tipo de erro). Você quase nunca
precisa escrever um `try/catch` - `?` faz isso.

### Modelos em tempo de compilação

Enquanto Eloquent lê seu esquema de banco de dados em tempo de execução,
Suprnova lê em tempo de compilação:

```rust
#[suprnova::model(table = "posts")]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub published_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Pronto - essa estrutura É o modelo Eloquent. Você obtém
`Post::find`, `Post::query()`, `Post::create`, `post.update(...)`,
`post.delete()`, soft deletes (com `#[model(soft_deletes)]`),
timestamps, observers, tudo. A macro gera `Entity`, `Model`,
`ActiveModel` e `Column` enum do SeaORM, e implementa a trait
`Model` do Suprnova - mas você depende de `Post`, não de nenhuma daquelas.

Se você renomear uma coluna em uma migração, a estrutura não corresponde mais
ao esquema de banco de dados - e dependendo da sua configuração, ou o compilador
a detecta em tempo de compilação ou o cast de tipo coerce falha na primeira
consulta. De qualquer forma você descobre antes da staging, não depois.

### Binário único

Não há PHP-FPM, sem configuração nginx lendo `index.php`, sem `composer
install` no deploy. `cargo build --release` oferece um binário estaticamente
vinculado. `scp` para um servidor, `systemd`, pronto. Ou compile um
contêiner - `FROM scratch` funciona.

Temos [receitas de implantação](deployment.md) para Railway, Digital
Ocean e Hetzner. A forma comum: compile o binário, envie o
binário, defina variáveis de ambiente, execute.

## Mapeando o framework

### Rotas

`routes!` desempenha o papel de `routes/web.php` e `routes/api.php`
combinados.

```rust
use suprnova::{routes, get, post, put, delete};
use crate::controllers;

routes! {
    get!("/", controllers::home::index).name("home"),

    // Grupo de rotas com prefixo + middleware compartilhados
    group("/admin")
        .middleware(crate::middleware::admin())
        .routes(routes! {
            get!("/users", controllers::admin::users::index).name("admin.users"),
            post!("/users", controllers::admin::users::store),
            put!("/users/{id}", controllers::admin::users::update),
            delete!("/users/{id}", controllers::admin::users::destroy),
        }),

    // Roteamento de recursos (o Route::resource do Laravel)
    resource!("posts", controllers::post),
}
```

Referência completa: [Roteamento](routing.md). Diferenças que vale a pena conhecer:

- O middleware de grupo é **achatado** na lista de middleware de cada rota
  no tempo de registro (não executado como uma camada de cadeia separada) - isso
  significa que não há custo extra em tempo de execução para agrupamento.
- Tanto a sintaxe `{id}` do Laravel quanto a sintaxe `:id` estilo Rails funcionam;
  são normalizadas internamente.
- Rotas nomeadas são resolvidas via `route("posts.show", &[("id", "42")])` e
  há uma variante de URL assinada para links com tempo limitado.

### Controladores

Um controlador é apenas uma função livre que retorna `Response`:

```rust
use suprnova::{Request, Response, json_response, HttpResponse};
use crate::models::Post;

pub async fn show(req: Request) -> Response {
    let id = req.param("id").unwrap_or("0").parse::<i64>()?;
    let post = Post::find_or_fail(id).await?;
    json_response!({ "post": post })
}
```

Você também pode usar a macro `#[handler]` para extrair argumentos tipados (parâmetros
de rota, consulta, corpo, a própria solicitação, serviços do contêiner) na
assinatura:

```rust
use suprnova::handler;

#[handler]
pub async fn show(post: post::Model) -> Response {
    // A vinculação de modelo de rota já rodou; `post` é a linha carregada.
    json_response!({ "post": post })
}
```

O tipo `post::Model` vem do módulo gerado do modelo - esse é
o sinal que `#[handler]` usa para escolher vinculação de modelo de rota sobre
a extração padrão de solicitação de formulário. Se a linha não existe, a vinculação
retorna um 404 antes que seu código seja executado - mesmo comportamento que a
vinculação implícita do Laravel.

Estruturas de ação (controladores "invocáveis" de método único, estilo Laravel) também
são suportadas: veja [Ações](actions.md).

### Eloquent

O construtor de consultas com API dupla usa nomes Laravel ou nomes idiomáticos
do Rust - ambos funcionam, escolha qualquer um que leia claramente no local de chamada.

```rust
// Superfície Laravel
let active = User::query()
    .db_where("status", "active")
    .order_by_desc("created_at")
    .limit(20)
    .get()
    .await?;

// Superfície Rust (resultado idêntico)
let active = User::query()
    .filter("status", "active")
    .order_by_desc("created_at")
    .take(20)
    .get()
    .await?;
```

`db_where` é o nome do lado Laravel (o bare `where` colide com a
palavra-chave do Rust). `filter` é o alias idiomático do Rust. Ambos existem;
ambos fazem a mesma coisa. Para operadores de não-igualdade, use `db_where_op`
(ou seu alias `filter_op`): `.db_where_op("status", "!=", "archived")`.
Veja a [referência Eloquent](eloquent.md) - é o capítulo mais longo
por uma razão, a superfície é ampla.

### Autenticação

```rust
use suprnova::{Auth, Credentials};

// Em um handler:
let user = Auth::user().await?;   // Option<Arc<dyn Authenticatable>>
let id = user.as_ref().map(|u| u.get_auth_identifier());

// Fazendo login (por exemplo, dentro de seu controlador de login):
let creds = Credentials::password("alice@x.com", "secret");
Auth::attempt(&creds, false).await?;

// Fazendo logout:
Auth::logout().await?;
```

Guardas, provedores, sessões, lembrar-me, verificação de email, redefinição de
senha, aceleração de força bruta, TOTP 2FA e OAuth estão tudo aqui. A superfície
de fluxos de autenticação espelha Laravel Fortify. Verificação de email e
redefinição de senha são suportadas por provedor (nenhum torii necessário):
seu modelo de usuário implementa `MustVerifyEmail` / `CanResetPassword` - os
análogos do Suprnova dos contratos do Laravel com os mesmos nomes - e o
`UserProvider` configurado conduz os fluxos. Veja [Autenticação](authentication.md)
e [Fluxos de autenticação](auth-flows.md).

### Migrações

Você escreve migradores SeaORM. A forma parecerá familiar mesmo que a
sintaxe seja nova:

```rust
use sea_orm_migration::prelude::*;

#[derive(DeriveMigrationName)]
pub struct Migration;

#[async_trait::async_trait]
impl MigrationTrait for Migration {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.create_table(
            Table::create()
                .table(Alias::new("posts"))
                .if_not_exists()
                .col(ColumnDef::new(Alias::new("id")).big_integer().primary_key().auto_increment())
                .col(ColumnDef::new(Alias::new("title")).string().not_null())
                .col(ColumnDef::new(Alias::new("body")).text().not_null())
                .to_owned()
        ).await
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        manager.drop_table(Table::drop().table(Alias::new("posts")).to_owned()).await
    }
}
```

`suprnova make:migration create_posts_table` cria o arquivo com scaffold.
`suprnova migrate`, `migrate:rollback`, `migrate:status`, `migrate:fresh`
todos fazem o que você esperaria. `suprnova db:sync` executa migrações e
regenera as entidades SeaORM que a camada de macro compila.
Veja [Migrações](migrations.md).

### Filas e agendamento

```rust
use suprnova::{FrameworkError, Job, Queue, async_trait};
use serde::{Deserialize, Serialize};

// Define um trabalho - os dados vivem na estrutura, o contrato vive em
// `impl Job`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SendWelcomeEmail {
    pub user_id: i64,
}

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str {
        "SendWelcomeEmail"
    }

    async fn handle(self) -> Result<(), FrameworkError> {
        let user = User::find_or_fail(self.user_id).await?;
        Mail::to(&user.email).send(WelcomeMail { user }).await?;
        Ok(())
    }
}

// Coloque na fila:
Queue::push(SendWelcomeEmail { user_id: user.id }).await?;

// Ou com um atraso:
Queue::later(
    std::time::Duration::from_secs(60),
    SendWelcomeEmail { user_id },
).await?;
```

Workers funcionam com `cargo run -- queue:work`. Drivers incluem
memória e sync (em-processo, para testes), banco de dados, redis e null.
Lotes, cadeias, trabalhos únicos, tentativas, backoff, middleware, armazenamento
de trabalhos com falha - tudo está aqui. Veja [Filas](queues.md).

O agendamento usa a trait `Task` e o binário do agendador por projeto:

```rust
use suprnova::{Task, TaskResult, async_trait};

pub struct DailyDigest;

#[async_trait]
impl Task for DailyDigest {
    async fn handle(&self) -> TaskResult {
        // …
        Ok(())
    }
}

// Registre dentro do bootstrap (por exemplo, via Schedule::call / .task / .add):
//   schedule.add(schedule.task(DailyDigest).daily().at("03:00").name("daily-digest"));
```

Veja [Agendamento de tarefas](scheduling.md).

### Correio, notificações, transmissão

Estes seguem um para um do Laravel. `Mailable` é uma macro de derivação;
`Notifiable` é uma trait no seu modelo de usuário; canais são
`mail`/`database`/`broadcast`/`webpush`; transmissão suporta
canais públicos, privados e de presença. Veja [Correio](mail.md),
[Notificações](notifications.md), [Transmissão](broadcasting.md).

### Frontend

Não há Blade. Em vez disso, o frontend é um SPA real via Inertia.js,
e você passa props tipadas do Rust:

```rust
use suprnova::{inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps, serde::Serialize)]
pub struct ShowProps {
    pub post: Post,
    pub comments: Vec<Comment>,
}

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id").unwrap_or("0").parse().unwrap_or(0);
    let post = Post::find_or_fail(id).await?;
    let comments = post.comments().get().await?;
    inertia_response!(&req, "Posts/Show", ShowProps { post, comments })
}
```

`Posts/Show` é um componente Svelte (ou React, ou Vue - seu starter
escolhe). Tipos TypeScript para as props são gerados automaticamente da
derivação `InertiaProps` - execute `suprnova generate-types` depois de
adicionar uma nova estrutura de props e o frontend recebe vinculações tipadas.

Se você usou Inertia no Laravel via `inertia()`, é a mesma
coisa - apenas tipada de ponta a ponta. Veja a [visão geral do Frontend](frontend.md).

## Coisas que mudam de forma

Algumas coisas se movem diferentemente no Suprnova. Nenhuma delas são bloqueadores,
mas vale a pena conhecer antecipadamente.

### Sem provedores de serviço

Laravel tem dezenas de provedores de serviço registrando vinculações, observers,
compositores de visualização, etc. Suprnova tem **uma** função de bootstrap em seu
`bootstrap.rs` de app. Você registra tudo lá, em ordem. Não é
elegante mas é transparente - você pode ver em 30 linhas exatamente o que
seu app inicia.

```rust
// bootstrap.rs
use std::sync::Arc;

pub async fn register() {
    suprnova::App::bind::<dyn MyService>(Arc::new(MyServiceImpl::new()));
    suprnova::Event::listen::<OrderShipped, _>(Arc::new(SendShipmentNotification)).await;
    crate::observers::register();
}
```

Os capítulos [Contêiner](container.md) e [Bootstrap](bootstrap.md)
têm o detalhe.

### Configuração é tipada

Onde Laravel usa `config('app.timezone')` retornando o que-quer-que-o-array-diga,
Suprnova tem estruturas de config tipadas:

```rust
let cfg = suprnova::Config::get::<AppConfig>()?;
let tz = &cfg.timezone;   // &str, não misto
```

Você pode registrar suas próprias seções de config tipadas. Veja [Configuração](configuration.md).

### Sem facades-como-aliases

Facades do Laravel como `DB::` são aliases de classe configurados em `config/app.php`.
Facades do Suprnova são módulos reais na raiz do crate:

```rust
use suprnova::{Auth, Cache, DB, Event, Gate, Mail, Notify, Queue, Schedule, Storage};
```

Mesma superfície, nenhum alias global necessário.

### Tempos de compilação são reais

Tempos de compilação do Rust não são PHP. Uma compilação limpa de um app
Suprnova novo leva 1–2 minutos; compilações incrementais durante o desenvolvimento
são alguns segundos. O fluxo de trabalho do dev é o mesmo - `suprnova serve`
observa mudanças e recompila - mas você sentirá na primeira vez que você mudar
uma macro e recompilar um crate downstream. Cache se paga rapidamente.

### O verificador de empréstimo existe

A maioria dos controladores e handlers nunca tocam em uma anotação de tempo
de vida - as assinaturas do framework os ocultam. Quando o verificador de empréstimo
grita com você, geralmente é porque você tentou manter uma referência através de um
`.await` que cruzou um mutex ou manteve uma transação de banco de dados através de
uma chamada aguardada que necessitava acesso exclusivo. Os erros são claros e os
corrigir geralmente é `.clone()` ou reestruturar-em-escopos-menores.

### Sem REPL `tinker`

Não há REPL. O equivalente mais próximo é um script `cargo run`
descartável em `examples/`, ou um teste `#[suprnova_test]` que exercita a
coisa que você está depurando. A maioria do que você faria em tinker
(fuçar em um modelo, disparar uma notificação, despachar um trabalho) é um teste
de 5 linhas.

## Onde os capítulos do Laravel caem

Pesquisa rápida se você sabe o que procura mas não onde vive:

| Tópico Laravel | Capítulo Suprnova |
|---|---|
| Ciclo de vida | [Ciclo de vida da solicitação](lifecycle.md) |
| Contêiner de serviço | [Contêiner de serviços](container.md) |
| Provedores de serviço | [Inicialização da aplicação](bootstrap.md) |
| Facades | [Contêiner de serviços](container.md) |
| Roteamento | [Roteamento](routing.md) |
| Middleware | [Middleware](middleware.md) |
| Proteção CSRF | [CSRF](csrf.md) |
| Controladores | [Controladores](controllers.md) |
| Solicitações | [Solicitações](requests.md) |
| Respostas | [Respostas](responses.md) |
| Geração de URL | [Geração de URLs](urls.md) |
| Sessão | [Sessões](session.md) |
| Validação | [Validação](validation.md) |
| Tratamento de erros | [Tratamento de erros](errors.md) |
| Logs | [Logs](logging.md) |
| Console Artisan | [Console](console.md) + [Referência da CLI](cli.md) |
| Transmissão | [Transmissão](broadcasting.md) |
| Cache | [Cache](cache.md) |
| Eventos | [Eventos](events.md) |
| Armazenamento de arquivo | [Sistema de arquivos e armazenamento](filesystem.md) |
| Cliente HTTP | [Cliente HTTP](http-client.md) |
| Localização | [Localização](localization.md) - catálogos Fluent `.ftl`, não arrays PHP |
| Correio | [Correio](mail.md) |
| Notificações | [Notificações](notifications.md) |
| Filas | [Filas](queues.md) |
| Limitação de taxa | [Limitação de taxa](rate-limiting.md) |
| Agendamento de tarefas | [Agendamento de tarefas](scheduling.md) |
| Autenticação | [Autenticação](authentication.md) |
| Autorização | [Autorização](authorization.md) |
| Verificação de email | [Fluxos de autenticação](auth-flows.md) |
| Redefinição de senha | [Fluxos de autenticação](auth-flows.md) |
| Criptografia | [Criptografia](encryption.md) |
| Hashing | [Hashing](hashing.md) |
| Banco de dados | [Banco de dados](database.md) |
| Construtor de consultas | [Construtor de consultas](queries.md) |
| Paginação | [Paginação](pagination.md) |
| Migrações | [Migrações](migrations.md) |
| Preenchimento de dados | [Preenchimento de dados](seeding.md) |
| Eloquent | [API Eloquent](eloquent.md) |
| Eloquent: Relacionamentos | [Relacionamentos](eloquent-relationships.md) |
| Eloquent: Coleções | [Coleções Eloquent](eloquent-collections.md) |
| Eloquent: Moldagens / Acessadores | [Eloquent - moldagens, acessadores e mutadores](eloquent-mutators.md) |
| Eloquent: Recursos API | [Recursos JSON:API](eloquent-resources.md) |
| Eloquent: Serialização | [Serialização Eloquent](eloquent-serialization.md) |
| Eloquent: Factories | [Factories Eloquent](eloquent-factories.md) |
| Testes | [Testes](testing.md) |
| Testes HTTP | [Testes HTTP](http-tests.md) |
| Testes de banco de dados | [Testes de banco de dados](database-testing.md) |
| Mocking | [Mocking e Fakes](mocking.md) |
| Cashier (Stripe) | [Pagamentos - adaptador Stripe](payments-stripe.md) |
| Cashier (Paddle) | [Pagamentos - adaptador Paddle](payments-paddle.md) |
| Sanctum / Passport | (ainda não - autenticação de token via integração torii) |
| Horizon | (ainda não - introspeção de fila é integrada) |
| Telescope / Pulse | (adiado para v2+) |

Coisas que o Laravel tem que o Suprnova não tem (ainda):

- Telescope / Pulse (superfície de observabilidade) - [observabilidade](observability.md) básica é enviada, os dashboards não
- Autenticação de token Sanctum / Passport - integração torii cobre OAuth e autenticação de sessão; autenticação dedicada de token é pretendida, não enviada
- Horizon - introspeção de fila é construída no framework, nenhum dashboard separado
- Blade - por design; Inertia é a história do frontend
- `trans_choice` - [Localização](localization.md) é enviada, mas plurais são
  selecionados dentro da mensagem por categoria CLDR em vez de pelos
  intervalos de inteiro estilo `[1,19]` que `trans_choice` usa

## Próximos passos

- [Instalação](installation.md) - prepare um projeto
- [Início rápido](quickstart.md) - construa um pequeno app em 5 minutos
- [Roteamento](routing.md) - o próximo capítulo natural daqui

Ou vá para qualquer lugar via [`documentação.md`](documentation.md).
