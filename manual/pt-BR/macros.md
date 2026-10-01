# Macros

O Suprnova vem com cerca de três dezenas de macros, todas re-exportadas
de `suprnova::*`. Elas são as juntas onde o framework encontra o seu
código - `routes!` constrói o router, `#[handler]` adapta uma função
para virar uma, `#[suprnova::model]` transforma um struct em um model
Eloquent, `#[derive(Data)]` produz um payload Inertia tipado. Este
capítulo é o índice. Cada macro recebe uma descrição de um parágrafo,
um exemplo mínimo, e um ponteiro para o capítulo que a usa para
trabalho de verdade.

Alguns princípios que valem para toda a superfície:

- **Macros emitem paths totalmente qualificados.** O código gerado
  escreve `::suprnova::…` para que as macros funcionem quer você tenha
  importado os tipos subjacentes ou não.
- **Uso pesado de `inventory::submit!`.** Models, commands, policies,
  observers, provedores de pagamento, e mais se registram sozinhos em
  tempo de compilação, e o framework drena o registro na
  inicialização. Você quase nunca conecta o registro à mão.
- **Validação em tempo de compilação onde vale a pena.**
  `inertia_response!` verifica que o arquivo do componente nomeado
  existe. `redirect!` verifica que a rota nomeada existe. `routes!`
  rejeita paths que não começam com `/`. Erros que podem ser
  capturados em tempo de build são capturados.

## Roteamento

| Macro | Retorna | O que faz |
|---|---|---|
| `routes!` | `pub fn register() -> Router` | Lista de rotas de nível superior - exporta um `register()` que seu `app.rs` chama |
| `get!` / `post!` / `put!` / `delete!` / `patch!` / `head!` / `options!` / `any!` | `RouteDefBuilder<H>` | Uma rota HTTP - encadeável com `.name(...)` / `.middleware(...)` |
| `group!` | `GroupDef` | Prefixo + middleware aplicados a uma lista filha de rotas |
| `fallback!` | `FallbackDefBuilder<H>` | Handler de 404 customizado quando nenhuma rota corresponde |
| `ws!` | `WsRouteDef` | Uma rota WebSocket - encadeável com `.middleware(...)` / `.config(...)` |

```rust
use suprnova::{routes, get, post, ws, group};
use crate::{controllers, middleware::AuthMiddleware, ws::ChatHandler};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/users/{id}", controllers::user::show).name("users.show"),
    post!("/users", controllers::user::store).name("users.store"),

    group!("/admin", {
        get!("/dashboard", controllers::admin::dashboard),
    }).middleware(AuthMiddleware),

    ws!("/ws/chat", ChatHandler),
}
```

A string do path da rota é verificada em tempo de compilação -
`validate_route_path` rejeita qualquer coisa que não comece com `/`.
Nomes de rota registrados via `.name("…")` também são verificados
quanto à unicidade na inicialização através de `register_route_name`.
Veja [Roteamento](routing.md) para a expansão completa e
[WebSockets](websockets.md) para `ws!`.

## Handlers e solicitações

### `#[handler]`

Reescreve uma função de controller para que ela possa extrair
parâmetros tipados (via `FromRequest`) diretamente da solicitação
recebida - em vez de puxar campos manualmente de `Request`, você
declara o que o handler precisa e a macro conecta tudo.

```rust
use suprnova::{handler, Response, json_response, request};

#[request]
pub struct CreateUserRequest {
    #[validate(email)]
    pub email: String,

    #[validate(length(min = 8))]
    pub password: String,
}

#[handler]
pub async fn store(form: CreateUserRequest) -> Response {
    // `form` já está validado - 422 é retornado automaticamente em caso de falha
    json_response!({ "email": form.email })
}
```

Um primeiro parâmetro em forma de `Request` ainda é aceito como o
caso de identidade. Veja [Controladores](controllers.md).

### `#[request]` e `#[derive(FormRequest)]`

`#[request]` é a forma recomendada de declarar um tipo de request
validado. Ele auto-deriva `Deserialize`, `Validate`, e `FormRequest`,
então o struct funciona tanto com corpos `application/json` quanto
`application/x-www-form-urlencoded`.

`#[derive(FormRequestDerive)]` é o derive subjacente, caso você queira
abrir mão do attribute (você vai precisar derivar `Deserialize` e
`Validate` você mesmo). O attribute é o que recomendamos; o derive
existe para o caso extremo. Veja [Solicitações](requests.md) e
[Validação](validation.md).

### `#[derive(MultipartRequest)]`

Extractor fortemente tipado para `multipart/form-data` - vincula
campos de texto e arquivos enviados em um único struct, com
validadores por campo em nível de tipo.

```rust
use suprnova::{MultipartRequest};
use suprnova::http::upload::{ImageFile, MaxSize, UploadedFile};

#[derive(MultipartRequest)]
pub struct AvatarUpload {
    #[field("avatar")]
    pub avatar: UploadedFile<(ImageFile, MaxSize<5_242_880>)>,

    #[field("caption")]
    pub caption: Option<String>,
}
```

Validadores built-in (`ImageFile`, `MimeAllowlist<…>`, `MaxSize<…>`,
`MimeType<…>`) se compõem via tuples. Veja [Solicitações](requests.md).

## Respostas

### `json_response!` e `text_response!`

As duas macros de resposta em forma curta. Ambas envolvem
`HttpResponse::*` em `Ok(...)` para que se encaixem direto na posição
de retorno de um handler:

```rust
use suprnova::{handler, json_response, text_response, Response};

#[handler]
pub async fn health() -> Response {
    json_response!({ "status": "ok" })
}

#[handler]
pub async fn robots() -> Response {
    text_response!("User-agent: *\nDisallow:")
}
```

Veja [Respostas](responses.md).

### `inertia_response!`

Constrói uma resposta de página Inertia, validando em tempo de
compilação que o arquivo do componente nomeado (`.svelte` / `.tsx` /
`.jsx` / `.vue`) existe em `frontend/src/pages/`. Se você errar o
nome do componente, o build falha com sugestões:

```rust
use suprnova::{handler, inertia_response, InertiaProps, Request, Response};

#[derive(InertiaProps)]
struct HomeProps {
    title: String,
    user_count: i64,
}

#[handler]
pub async fn index(req: Request) -> Response {
    inertia_response!(&req, "Home", HomeProps {
        title: "Welcome".into(),
        user_count: 42,
    })
}
```

`#[derive(InertiaProps)]` gera o impl `Serialize` de que a forma de
resposta precisa. Veja [Respostas Inertia](frontend-inertia-responses.md).

### `redirect!`

Redirecionamento type-safe para uma rota nomeada - o nome da rota é
verificado em tempo de compilação contra os nomes registrados através
de `routes!`:

```rust
use suprnova::redirect;

// Só compila se "users.show" for um nome de rota registrado
let resp = redirect!("users.show").with("id", "42").into();
```

Veja [Geração de URLs](urls.md).

## Eloquent

### `#[suprnova::model]`

Transforma um struct simples em um model Eloquent completo: gera
stubs de `Entity`, `Model`, `ActiveModel`, `Column`, `Relation` do
SeaORM, além de todos os impls de trait que o Eloquent precisa.
Também faz `inventory::submit!` de um `ModelEntry` para que o
framework possa enumerar todo model na inicialização.

```rust
use suprnova::model;

#[model(table = "users")]
pub struct User {
    pub id: i64,
    pub name: String,
    pub email: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

As chaves de attribute incluem `table`, `primary_key`, `key_type`,
`auto_increment`, `connection`, `fillable`, `guarded`, `casts`,
`timestamps`, `soft_deletes`, `appends`, `hidden`, `visible`,
`mutators`, `touches`, e `unique_id` (para PKs UUID/ULID). Veja
[Eloquent](eloquent.md).

### `#[suprnova::scopes(Model)]`

Percorre um bloco `impl Model { … }` e transforma todo método cuja
assinatura corresponde a
`fn name(query: Builder<Self>[, args…]) -> Builder<Self>` em um scope -
gerando tanto `Model::scope_name(args)` quanto um `.scope_name(args)`
encadeável em `Builder<Model>`.

```rust
use suprnova::{scopes, Builder};

#[suprnova::scopes(User)]
impl User {
    pub fn active(query: Builder<Self>) -> Builder<Self> {
        query.filter("active", true)
    }

    pub fn popular(query: Builder<Self>, threshold: i64) -> Builder<Self> {
        query.filter_op("followers_count", ">", threshold)
    }

    // Não é um scope - passa direto, sem mudanças
    pub fn display_name(&self) -> String { self.name.clone() }
}

// Ambos os call sites compilam:
// User::active().popular(500).get().await?;
// User::query().filter_op("id", ">", 0).active().get().await?;
```

A forma encadeável exige que o trait gerado `HasScope_<scope>_<Model>`
esteja em escopo quando chamada a partir de um módulo diferente. Veja
[Eloquent](eloquent.md).

### `#[suprnova::observer(Model)]`

Conecta um bloco `impl Observer<M>` ao sistema de eventos de ciclo de
vida - cada um dos 16 métodos sobrescritos se torna um listener
registrado, submetido ao inventory e drenado na inicialização.

```rust
use async_trait::async_trait;
use suprnova::eloquent::observers::Observer;
use suprnova::eloquent::events::EventResult;
use suprnova::eloquent::attrs::Attrs;
use suprnova::FrameworkError;

pub struct AuditObserver;

#[suprnova::observer(User)]
#[async_trait]
impl Observer<User> for AuditObserver {
    async fn creating(&self, attrs: &mut Attrs) -> EventResult {
        if attrs.get("email").is_none() {
            return EventResult::cancel("email is required");
        }
        EventResult::ok()
    }

    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user.id = user.id, "user created");
        Ok(())
    }
}
```

**Ordem de attribute obrigatória: `#[suprnova::observer(M)]` precisa
vir antes de `#[async_trait]`.** Attribute macros se expandem de fora
para dentro - se `async_trait` rodar primeiro, ele reescreve toda
`async fn` em uma forma sem açúcar sintático, e o name-match da macro
observer contra os 16 nomes de método do trait silenciosamente não
encontra nada. Veja [Eventos](events.md).

### `#[suprnova::accessor]` e `#[suprnova::mutator]`

Marcadores em nível de função em métodos `impl Model { … }` que se
conectam aos caminhos `to_json()` / `fill()` do model. Referencie o
nome do campo em `#[model(appends = […])]` (accessor) ou
`#[model(mutators = […])]` (mutator) para a macro conectá-los.

```rust
#[suprnova::model(appends = ["full_name"], mutators = ["password"])]
pub struct User {
    pub id: i64,
    pub first_name: String,
    pub last_name: String,
    pub password: String,
}

impl User {
    #[suprnova::accessor]
    pub fn full_name(&self) -> String {
        format!("{} {}", self.first_name, self.last_name)
    }

    #[suprnova::mutator]
    pub fn set_password(
        &mut self,
        value: serde_json::Value,
    ) -> Result<(), suprnova::FrameworkError> {
        let raw: String = serde_json::from_value(value)
            .map_err(|e| suprnova::FrameworkError::validation("password", format!("{e}")))?;
        self.password = bcrypt(raw);
        Ok(())
    }
}
```

Veja [Mutadores e Casts](eloquent-mutators.md).

### `#[suprnova::prunable]`

Envolve um impl `Prunable` (ou `MassPrunable`) e submete um
`PrunerEntry` ao registro que `model:prune` percorre em runtime:

```rust
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::eloquent::Prunable;

#[suprnova::prunable]
#[async_trait]
impl Prunable for Session {
    fn prunable() -> suprnova::Builder<Self> {
        Self::query().filter_op(
            "expires_at",
            "<",
            (Utc::now() - Duration::days(30)).to_rfc3339(),
        )
    }
}
```

Veja [Eloquent](eloquent.md).

### `attrs!`

Constrói um map `Attrs` ordenado (`IndexMap<&'static str, serde_json::Value>`)
para `Model::create` / `Model::update` / `Model::fill`:

```rust
use suprnova::attrs;

let user = User::create(attrs! {
    name: "Alice",
    email: "alice@example.com",
    age: 32,
}).await?;
```

Veja [Eloquent](eloquent.md).

### `casts!`

Constrói um map de casts por consulta que você pode passar para
`Builder::with_casts`:

```rust
use suprnova::{casts, AsDate, AsJson};

let map = casts! {
    birthday = AsDate,
    metadata = AsJson<serde_json::Value>,
};
let rows = User::query().with_casts(map).get().await?;
```

Veja [Mutadores e Casts](eloquent-mutators.md).

### `route_binding!`

Implementa `RouteBinding` para uma entity SeaORM escrita à mão, para
que ela resolva automaticamente a partir de um parâmetro de rota.
Models definidos com `#[suprnova::model]` se registram
automaticamente e não precisam disso; recorra a `route_binding!`
quando você escreveu a entity à mão:

```rust
use suprnova::route_binding;

route_binding!(crate::entities::user::Entity, User, "user");
```

Depois disso, `get!("/users/{user}", controllers::user::show)` passa
um `User` totalmente carregado para o seu handler. Veja
[Roteamento](routing.md).

## Dados e Inertia

### `#[derive(Data)]`

O derive composto para payloads tipados. Produz um impl `Serialize`
que respeita campos `#[data(input_only)]`, além de um impl
`Deserialize` que rejeita payloads que tentam definir campos
`#[data(output_only)]`. Combine com `#[json_resource("type")]` para
saída JSON:API através do capítulo `Resource`.

```rust
use suprnova::{Data, Validate};

#[derive(Data, Validate)]
struct UserDto {
    pub id: i64,
    pub name: String,

    #[data(input_only)]
    #[validate(length(min = 8))]
    pub password: String,

    #[data(output_only)]
    pub computed_handle: String,

    #[data(allow_include)]
    pub posts: Vec<PostDto>,
}
```

`#[data(allow_include)]` registra o campo na allowlist de include de
partial-reload via `inventory::submit!`. Veja
[Objetos de dados](data.md) e [Recursos JSON:API](eloquent-resources.md).

### `#[derive(InertiaProps)]`

Gera o impl `Serialize` de que `inertia_response!` precisa. Um derive
marcador simples - a maioria das apps recorre a `#[derive(Data)]` em
vez disso, porque ele te dá includes de partial-reload de graça.

```rust
use suprnova::InertiaProps;

#[derive(InertiaProps)]
struct DashboardProps {
    title: String,
    user: User,
}
```

Veja [Respostas Inertia](frontend-inertia-responses.md).

### `when_loaded!`

Emite um `Prop::lazy(…)` apenas quando uma relação nomeada foi
eager-loaded na entity; caso contrário emite `Prop::absent()` para que a
prop seja completamente pulada na resposta:

```rust
use suprnova::when_loaded;

let songs_prop = when_loaded!(&artist, "songs", || async {
    serde_json::to_value(&artist.songs).unwrap()
});
```

Veja [Objetos de dados](data.md).

## Injeção de dependência

### `#[service]`

Adiciona `Send + Sync + 'static` a um trait para que ele se encaixe
no contêiner:

```rust
use suprnova::service;

#[service]
pub trait HttpClient {
    async fn get(&self, url: &str) -> Result<String, FrameworkError>;
}

// App::bind::<dyn HttpClient>(Arc::new(RealHttpClient::new()));
// let client = App::make::<dyn HttpClient>()?;
```

Veja [Contêiner de serviços](container.md).

### `#[injectable]`

Auto-registra um tipo concreto como singleton. Deriva `Default` +
`Clone` e submete um registro que roda na inicialização:

```rust
use suprnova::injectable;

#[injectable]
pub struct AppState {
    pub counter: u32,
}

// let state: AppState = App::get().unwrap();
```

Veja [Contêiner de serviços](container.md).

## Erros

### `#[domain_error]`

Define um erro de domínio que implementa `Display`, `Error`,
`HttpError`, e `From<T> for FrameworkError` - para que ele faça
short-circuit em um handler via `?`:

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFoundError {
    pub user_id: i32,
}

pub async fn get_user(id: i32) -> Result<User, FrameworkError> {
    let user = User::find(id).await?
        .ok_or_else(|| UserNotFoundError { user_id: id })?;
    Ok(user)
}
```

Veja [Tratamento de erros](errors.md).

## Console e trabalho em background

### `#[command]`

Marca uma `async fn(Vec<String>) -> Result<(), FrameworkError>` como
um comando de console. Submete um `CommandEntry` para que
`dispatch_argv` a encontre quando o binário console por projeto roda:

```rust
use suprnova::{command, FrameworkError};

#[command(name = "db:seed", description = "Run all registered seeders")]
async fn db_seed(_args: Vec<String>) -> Result<(), FrameworkError> {
    suprnova::seed::run_all().await
}
```

Veja [Console](console.md).

### `#[derive(Command)]`

A alternativa com args tipados. Vai em cima de
`#[derive(clap::Parser)]`, lê `#[console(...)]` para metadados, e
emite o runner que chama seu `TypedCommand::run`:

```rust
use async_trait::async_trait;
use suprnova::{Command, FrameworkError, TypedCommand};

#[derive(clap::Parser, Command)]
#[console(name = "greet", description = "Greet someone")]
pub struct Greet {
    #[arg(short, long)]
    name: Option<String>,
    #[arg(long)]
    loud: bool,
}

#[async_trait]
impl TypedCommand for Greet {
    async fn run(self) -> Result<(), FrameworkError> {
        let target = self.name.unwrap_or_else(|| "world".into());
        println!("{}", if self.loud { format!("HELLO {target}!") } else { format!("Hello {target}") });
        Ok(())
    }
}
```

Veja [Console](console.md).

### `#[workflow]` e `#[workflow_step]`

`#[workflow]` registra uma async fn como um workflow durável - estado
executável, steps que podem ser retentados, histórico persistido.
Cada `#[workflow_step]` dentro do corpo é um checkpoint do qual o
runtime pode retomar depois de um crash ou restart.

```rust
use suprnova::{workflow, workflow_step, FrameworkError};

#[workflow]
async fn onboard_user(user_id: i64) -> Result<(), FrameworkError> {
    send_welcome_email(user_id).await?;
    enable_default_features(user_id).await?;
    Ok(())
}

#[workflow_step]
async fn send_welcome_email(user_id: i64) -> Result<(), FrameworkError> {
    // …
    Ok(())
}
```

### `start_workflow!`

Dispara um workflow por path, serializando os args na forma de
envelope do runtime de workflow:

```rust
use suprnova::start_workflow;

let handle = start_workflow!(crate::workflows::onboard_user, 42).await?;
```

Veja [Fluxos de trabalho](workflows.md).

### `schedule_task!`

Açúcar sintático em torno de `TaskBuilder::from_async` para que uma
closure agende de forma limpa lado a lado com impls de `Task`
baseados em trait:

```rust
use suprnova::{schedule_task, FrameworkError};

let task = schedule_task!(|| async {
    println!("ticking");
    Ok::<(), FrameworkError>(())
})
    .every_minute()
    .name("tick");
```

Veja [Agendamento de tarefas](scheduling.md).

## Autorização

### `#[policy(UserType, ResourceType)]`

Envolve um bloco `impl Policy` e registra cada método como uma ação
de gate nomeada. O nome do gate combina o nome do método com o tipo
de resource em minúsculas - `fn view(...)` em `Comment` vira
`"view-comment"`:

```rust
use suprnova::policy;

struct CommentPolicy;

#[policy(User, Comment)]
impl CommentPolicy {
    fn view(_user: &User, _comment: &Comment) -> bool { true }
    fn update(user: &User, comment: &Comment) -> bool {
        comment.author_id == user.id
    }
}
```

`Server::run` chama `authorization::init_policies()` automaticamente.
Veja [Autorização](authorization.md).

## Notificações e correio

### `#[derive(NotificationMailable)]`

Auto-gera `to_mail` a partir de um attribute `#[mail(...)]` -
templates Tera inline ou apoiados em arquivo para subject, corpo
HTML, e corpo em texto. Verificações em tempo de compilação: subject
obrigatório, ao menos um corpo presente, html/html_template mútuos
exclusivos, `from_name` exige `from`:

```rust
use serde::{Serialize, Deserialize};
use suprnova::NotificationMailable;

#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Your order shipped - tracking {{ tracking }}",
    html    = "<p>Tracking: <code>{{ tracking }}</code></p>",
    text    = "Tracking: {{ tracking }}",
    from    = "orders@suprnova.dev",
)]
pub struct OrderShipped { pub tracking: String }
```

O trait de notification em si é implementado à mão - não existe
`#[derive(Notification)]`. Veja [Notificações](notifications.md) e
[Correio](mail.md).

## Validação

### `validate!`

Ponto de entrada de validação síncrono e declarativo. Cada linha
pareia um nome de campo com um ou mais valores `Rule` (ou
`ContextualRule`), com `?:` para "validar somente se presente" e
`?=>` para campos opcionais condicionalmente obrigatórios:

```rust
use suprnova::{validate, ValidationErrors};
use suprnova::validation::rules::*;

fn validate_form(self_ref: &SignupForm) -> Result<(), ValidationErrors> {
    validate! { self_ref =>
        email   => Required, Email;
        password => Required, Min(8);
        bio     ?: Max(500);
        card_number ?=> RequiredIf { other: "billing_type", value: "card" } => with ctx;
    }
}
```

`Validate` é re-exportado do crate `validator` - attributes
`#[validate(...)]` (por exemplo, `#[validate(email)]`) vêm do
`validator` e rodam através do caminho síncrono do `FormRequest`. Use
`validate!` quando você precisar de regras contextuais / entre
campos, regras assíncronas, ou regras da paleta
`suprnova::validation::rules`. Veja [Validação](validation.md).

## Factories

### `#[derive(Factory)]`

Gera um marcador irmão `<Model>Factory` e um impl `Factory` que
produz models via `fake::Faker`. O model precisa implementar
`fake::Dummy<fake::Faker>` - tipicamente via `#[derive(Dummy)]`:

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct User {
    pub id: i32,
    pub name: String,
    pub email: String,
}

// UserFactory existe:
let users = UserFactory::new().count(10).make_many();
```

Veja [Factories Eloquent](eloquent-factories.md).

## Testes

### `#[suprnova_test]`

Envolve um teste `async fn` com um banco de dados SQLite em memória
(rodando `crate::migrations::Migrator` por padrão), invoca
`App::init()` e `App::boot_services()`, e roda o corpo sob
`#[tokio::test]`. Testes paralelos permanecem herméticos através da
camada por-thread do contêiner - vincule serviços específicos de
teste através de `TestContainer::fake` (não `App::bind`) para que
cada thread veja seus próprios fakes:

```rust
use suprnova::suprnova_test;
use suprnova::testing::TestDatabase;

#[suprnova_test]
async fn creates_a_user(db: TestDatabase) {
    let user = User::create(attrs! { name: "A", email: "a@x.com" }).await.unwrap();
    assert!(user.id > 0);
}
```

Um migrator customizado vai via
`#[suprnova_test(migrator = MyMigrator)]`. Veja [Testes](testing.md).

### `test_database!`

O construtor `TestDatabase` de uma linha para testes que não recebem
o parâmetro `db` através de `#[suprnova_test]`:

```rust
let db = test_database!();
let db = test_database!(my_crate::CustomMigrator);
```

### `describe!`, `test!`, `expect!`

Agrupamento no estilo Jest + assertions fluentes. `describe!` é um
módulo, `test!` produz um `#[test]` (síncrono ou assíncrono, com ou
sem um parâmetro `TestDatabase`), e `expect!` envolve um valor para
assertions encadeadas com contexto de arquivo/linha em caso de
falha:

```rust
use suprnova::{describe, test, expect};

describe!("CreateUserAction", {
    test!("creates a user", async fn(db: TestDatabase) {
        let user = CreateUserAction::new()
            .execute("test@example.com").await.unwrap();
        expect!(user.email).to_equal("test@example.com".to_string());
    });
});
```

Veja [Testes](testing.md).

## Middleware

### `global_middleware!`

Registra um middleware que roda em toda solicitação, na ordem de
registro, antes de qualquer middleware específico de rota.
Idempotente por tipo:

```rust
use suprnova::global_middleware;
use crate::middleware;

pub fn register() {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
}
```

Precisa rodar antes de `Server::from_config` / `Server::new` - o
servidor tira um snapshot do registro global em tempo de build. Veja
[Middleware](middleware.md).

## Armadilhas

Uma lista curta de modos de falha fáceis de cair e fáceis de
corrigir.

### Ordem de attribute - `#[observer]` precisa vir antes de `#[async_trait]`

```rust
// CORRETO
#[suprnova::observer(User)]
#[async_trait]
impl Observer<User> for AuditObserver { … }

// ERRADO - silenciosamente emite zero listeners
#[async_trait]
#[suprnova::observer(User)]
impl Observer<User> for AuditObserver { … }
```

Attribute macros se expandem de fora para dentro. `async_trait`
reescreve toda `async fn` em uma forma `Pin<Box<dyn Future>>`
sem açúcar sintático. Se ele rodar primeiro, a macro observer não
consegue mais fazer o match por nome de método e não emite nada. A
mesma regra de fora-para-dentro se aplica sempre que você empilha
várias macros - coloque o attribute do Suprnova por fora quando
estiver em dúvida.

### A armadilha do impl inerente

Um método de `impl` inerente **não pode** sobrepor o método default
de um trait através de trait dispatch. Se você escrever uma macro
(ou código à mão) que define `fn save(&self)` em um model como um
método inerente, chamadas que passam pelo trait `Model`
(`some_model.save()`, onde o call site só o conhece como
`&dyn Model`) vão escolher o default do trait - não sua sobrescrita
inerente.

Correção: emita uma sobrescrita de método de trait, nunca um método
inerente, quando o comportamento gerado precisar participar de trait
dispatch. É por isso que as macros do framework (notavelmente
`#[suprnova::model]`) escrevem para o impl do trait. Se você estiver
escrevendo extensões Eloquent à mão, faça o mesmo.

### `global_middleware!` só tem efeito antes de `Server::from_config`

O servidor tira um snapshot do registro global quando é construído.
Chamar `global_middleware!(M)` depois de `Server::from_config(...)`
não se aplica retroativamente àquele servidor. Registre todo
middleware global em `bootstrap()`, antes que `Application::run()`
alcance a etapa de serve.

### `redirect!` e `inertia_response!` são verificações em tempo de build

Ambas as macros se recusam a compilar se o alvo nomeado não existir -
esse é o objetivo. Se um refactor remove uma rota ou um nome de
componente, todo call site que a menciona quebra o build, que é
exatamente o que você quer. Se o erro de build te surpreender,
procure a string literal no seu bloco `routes!` / diretório de pages
antes de "corrigir" a chamada da macro.

### `?:` pula em `None`; `?=>` roda mesmo em `None`

Em linhas de `validate!`, `?:` só roda regras quando o campo é
`Some`. Uma regra condicional-de-presença como `RequiredIf` em uma
linha `?:`, portanto, nunca pode falhar em um campo ausente. Use
`?=>` (que trata ausência como `""`) para o caso de
exigir-quando-X.

### `#[derive(Validate)]` é do crate `validator`, não do Suprnova

O Suprnova re-exporta `validator::Validate` para que você não precise
de uma dependência direta em `validator`. Os attributes
`#[validate(...)]` vêm do `validator`. A própria macro `validate!` do
Suprnova é o ponto de entrada de runtime para regras entre campos /
contextuais; as duas se complementam, mas vivem em namespaces
diferentes.

## Por que Suprnova diverge

O Laravel descobre routes, commands, mail templates, model classes,
factories, observers, e policies em runtime - através de reflection,
varredura de filesystem, e dispatch baseado em string. O PHP torna
isso barato (autoloading + opcache amortizam o custo), e a
experiência de desenvolvedor é excelente: solte um arquivo no
diretório certo e ele aparece.

Esse modelo não se encaixa no Rust. Não temos reflection em runtime
sobre trait impls, o runtime é um único binário linkado
estaticamente, e varreduras de filesystem na inicialização se
encaixam pior em um modelo de processo onde cada binário serve
milhões de solicitações.

Então o Suprnova faz o mesmo trabalho em tempo de compilação. Rotas
são validadas, nomes de componente são verificados contra o
diretório de pages, mail templates são embutidos via `include_str!`,
nomes de rota são verificados quanto à unicidade através do
inventory, models se registram sozinhos em um inventory que o
framework drena na inicialização, commands da mesma forma. A
experiência de desenvolvedor é parecida - solte um arquivo, adicione
um `#[command]` ou `#[suprnova::model]`, rode o binário - mas a
conexão acontece antes do `main`, em vez de na primeira solicitação.

A troca é que erros de digitação, componentes ausentes, e
referências quebradas são erros de build em vez de erros de runtime,
e não há custo de reflection por solicitação.

## Próximos passos

- [Roteamento](routing.md) - expansão completa de `routes!`,
  nomeação, model binding
- [Controladores](controllers.md) - `#[handler]` e `#[request]`
  juntos
- [Eloquent](eloquent.md) - `#[suprnova::model]` e afins em contexto
- [Validação](validation.md) - `validate!`, regras contextuais,
  regras assíncronas
- [Console](console.md) - `#[command]` e `#[derive(Command)]` de
  ponta a ponta
- [Testes](testing.md) - `#[suprnova_test]`, `expect!`, fakes
