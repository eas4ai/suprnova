# Tratamento de erros

Este é o guia dos padrões do dia a dia para escrever código falível em
handlers, serviços e middleware do Suprnova. Para o modelo subjacente -
o contrato de conversão, o limite de panic, a regra de sanitização para
5xx, os hooks de observabilidade - leia
[Modelo de erros](error-model.md). Este capítulo mostra o que realmente
digitar.

A estrutura para memorizar:

- Handlers retornam `Response = Result<HttpResponse, HttpResponse>`.
- O operador `?` realiza uma conversão direta `From<E>` para o tipo de erro
  do handler; Rust não encadeia `DbErr -> FrameworkError -> HttpResponse`.
  Em um handler `Response`, converta explicitamente um erro SeaORM. Código que
  já retorna `Result<_, FrameworkError>` pode usar `.await?` diretamente.
- Três helpers livres (`abort_with`, `abort_if`, `abort_unless`) deixam
  você fazer short-circuit em um status code sem nomear um tipo de erro.

```rust
use sea_orm::EntityTrait;
use suprnova::{DB, FrameworkError, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let user = users::Entity::find_by_id(id)
        .one(&*DB::get()?)
        .await
        .map_err(FrameworkError::from)?
        .ok_or_else(|| FrameworkError::not_found("User"))?;
    json_response!({ "user": user })
}
```

O resto do capítulo é o catálogo dos produtores de erro - o que
construir, que status ele retorna, que forma o cliente vê.

## O `?` é a conversão

Todo `?` no corpo de um handler realiza uma conversão direta
`From<E> for HttpResponse`. O framework fornece conversões diretas para seus
tipos de erro voltados a handlers, mas Rust não encadeia várias implementações
de `From`. Converta explicitamente um erro intermediário quando ele não tiver
uma conversão direta para `HttpResponse`.

```rust
use suprnova::{DB, FrameworkError, Request, Response, json_response};
use sea_orm::EntityTrait;

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = users::Entity::find_by_id(id)
        .one(&*DB::get()?)
        .await
        .map_err(FrameworkError::from)?
        .ok_or_else(|| FrameworkError::not_found("User"))?;

    json_response!({ "user": user })
}
```

Quatro conversões ocorrem nesse trecho:

1. `req.param("id")?` converte diretamente `ParamError` em uma
   `HttpResponse` (400).
2. O erro de parse é mapeado explicitamente para `FrameworkError::ParamError`,
   que `?` então converte diretamente em uma `HttpResponse` (400).
3. O erro SeaORM é mapeado explicitamente de `DbErr` para
   `FrameworkError::Database`; `?` então converte diretamente esse
   `FrameworkError` em uma `HttpResponse` (500, sanitizado no wire).
4. `.ok_or_else(...)?` transforma `None` em
   `FrameworkError::ModelNotFound`, que é convertido em uma
   `HttpResponse` (404).

Cada `?` usa uma conversão direta. Código que retorna
`Result<_, FrameworkError>` em vez de `Response` pode usar `.await?` na chamada
SeaORM porque `DbErr` converte diretamente para `FrameworkError`.

Cada uma dessas conversões termina no corpo de erro JSON do framework -
`{ "message": …, "request_id": … }` no status correspondente. Essa é a
resposta certa para um cliente de API e a errada para uma visita
Inertia, que precisa de uma página. Nomeie uma
[página de erro](frontend-inertia-responses.md#error-pages) e um app
Inertia renderiza esses erros como uma página de verdade, enquanto os
clientes de API continuam recebendo o mesmo JSON.

## `AppError` - erros de domínio inline

Use `AppError` para erros pontuais que não merecem um tipo dedicado. Os
construtores mapeiam para a forma `abort($status, $msg)` do Laravel:

| Construtor | Status |
|---|---|
| `AppError::new(msg)` | 500 |
| `AppError::bad_request(msg)` | 400 |
| `AppError::unauthorized(msg)` | 401 |
| `AppError::forbidden(msg)` | 403 |
| `AppError::not_found(msg)` | 404 |
| `AppError::conflict(msg)` | 409 |
| `AppError::unprocessable(msg)` | 422 |
| `AppError::new(msg).status(code)` | qualquer |

`AppError` tem um `From` para `FrameworkError`, então o `?` funciona sem
cerimônia:

```rust
use suprnova::{AppError, Request, Response, json_response};

pub async fn transfer(req: Request) -> Response {
    let amount: i64 = req.param("amount")?.parse()
        .map_err(|_| AppError::bad_request("amount must be a number"))?;

    if amount <= 0 {
        return Err(AppError::unprocessable("amount must be positive").into());
    }

    if amount > balance() {
        return Err(AppError::forbidden("amount exceeds daily limit").into());
    }

    json_response!({ "transferred": amount })
}
```

Note a assimetria: `AppError::unauthorized` é **401** (credenciais de
autenticação ausentes), enquanto `FrameworkError::Unauthorized` é **403**
(uma policy negou um usuário autenticado). Eles significam coisas
diferentes; escolha o que corresponde à falha.

## `FrameworkError` - o enum canônico

Extractors internos, o contêiner, a vinculação de rota, a validação, a
camada de banco de dados e o armazenamento produzem todos um
`FrameworkError`. Você geralmente constrói um através de um construtor de
conveniência e deixa o `?` roteá-lo.

```rust
use suprnova::FrameworkError;

FrameworkError::not_found("User");                    // 404
FrameworkError::bad_request("Bad input");             // 400
FrameworkError::param("user_id");                     // 400
FrameworkError::param_parse("user_id", "i64");        // 400
FrameworkError::validation("email", "required");      // 422
FrameworkError::domain("Conflict", 409);              // 409 (qualquer código)
FrameworkError::internal("disk full");                // 500
FrameworkError::database("timeout");                  // 500
FrameworkError::service_not_found::<MyService>();     // 500
FrameworkError::model_not_found("Post");              // 404
```

O conjunto completo de variantes, com as implicações para a forma da
resposta, está em [Modelo de erros](error-model.md). Os construtores
acima cobrem todo caso comum; você recorre às variantes diretamente
apenas quando faz match em um erro que recebeu.

### Conversões automáticas

O `FrameworkError` já fala os dialetos que suas dependências emitem. Os
dois `?` abaixo convertem automaticamente:

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: users::ActiveModel)
    -> Result<users::Model, FrameworkError>
{
    // DB::get retorna Result<_, FrameworkError>.
    // .insert retorna Result<_, DbErr>, que tem From<DbErr> para FrameworkError.
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

O framework também implementa `From<opendal::Error>` para operações de
armazenamento e `From<ParamError>` para extração de parâmetros de
caminho.

### Relançando com contexto

Quando você quer anotar de onde um erro veio sem perder o status code,
use `.context()`:

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

A mensagem se torna `"creating new user: <original>"`. Variantes
estruturadas (`Validation`, `ValidationError`, `ModelNotFound`,
`ParamParse`, `PrecognitionFailure`, `PrecognitionSuccess`,
`Unauthorized`, `UnsupportedMediaType`, `AlreadyReported`,
`RateLimited`, `External`) mantêm sua variante para que o renderer de
resposta ainda emita a forma correta (e, no caso de `External`, para que
a fonte envolvida sobreviva); variantes planas que só carregam mensagem
(`Internal`, `Database`, `Domain`) se achatam em uma `Domain` com a
mensagem prefixada e o status original preservado.

### Transformando erros de chave duplicada em 422

A regra de validação `Unique` executa um `SELECT COUNT(*)` antes da
escrita, então ela é consultiva - duas solicitações concorrentes podem
ambas passar e depois ambas tentar o insert. A solicitação perdedora
recebe uma violação de restrição de unicidade do banco de dados, que de
outra forma vazaria como um 500. `from_unique_violation` a traduz para o
mesmo 422 que a regra consultiva teria produzido:

```rust
use suprnova::FrameworkError;

let user = new_user.insert(db).await.map_err(|e| {
    FrameworkError::from_unique_violation(
        "email",
        "That email address is already registered.",
        e,
    )
})?;
```

Se o `DbErr` subjacente não for uma violação de restrição de unicidade,
ele passa sem alterações como um erro `Database` da classe 500. A
cobertura de backends é o que o `DbErr::sql_err` do SeaORM reconhece -
Postgres, MySQL/MariaDB e SQLite mapeiam todos os seus erros de chave
duplicada por ali.

### Envolvendo um erro externo

Todas as outras variantes transformam em string aquilo que envolvem.
`from_external_with` mantém o erro original acessível, então os logs
podem renderizar a cadeia completa e o código ainda pode perguntar o que
de fato falhou:

```rust
use suprnova::FrameworkError;

let row = sqlx_like_query()
    .await
    .map_err(|e| FrameworkError::from_external_with("verify query failed", e))?;
```

`from_external(e)` é a mesma coisa, usando o próprio `Display` do erro
como mensagem. Ambos mapeiam para HTTP 500.

Para inspecionar o original, use `external_source()` em vez de `source()`:

```rust
if let Some(src) = err.external_source() {
    if let Some(db) = src.downcast_ref::<sea_orm::DbErr>() {
        // decida se vale a pena tentar novamente
    }
}
```

`std::error::Error::source()` devolve o handle `Arc` compartilhado, não
o erro envolvido, portanto o downcast por ele devolve `None`.
`external_source()` primeiro desreferencia o handle.

O framework renderiza a cadeia completa na linha de log 5xx e no campo
`debug_message` que adiciona quando `APP_DEBUG=true`, portanto o texto
de um erro envolvido nunca se perde.

### Preservando dicas de limite de taxa

Quando um serviço downstream limita suas solicitações e fornece uma dica
`Retry-After`, envolver a falha em `internal(...)` reduz a duração a
prosa. `rate_limited` a mantém estruturada:

```rust
use std::time::Duration;
use suprnova::FrameworkError;

let err = FrameworkError::rate_limited(
    Some(Duration::from_secs(30)),
    "push provider rejected the batch",
);

assert_eq!(err.retry_after(), Some(Duration::from_secs(30)));
assert_eq!(err.status_code(), 429);
```

Políticas de repetição da fila, agendamento com jitter e o cabeçalho de
resposta HTTP `Retry-After` leem a dica de volta por `retry_after()`,
que retorna `None` para toda outra variante e para throttles que
chegaram sem uma dica. `.context(...)` preserva a variante, portanto
adicionar contexto da operação não remove a duração.

## Erros de domínio customizados

Três níveis, dependendo de quão reutilizável o erro precisa ser.

### `#[domain_error]` para o caso tipado

A maioria dos erros reutilizáveis quer um nome, um status fixo, e um
template de mensagem fixo - sem mensagem por chamada. O attribute macro
`#[domain_error]` gera `Display`, `std::error::Error`, `HttpError`, e o
`From` para `FrameworkError` de uma só vez:

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFound;

#[domain_error(status = 402, message = "Insufficient funds")]
pub struct InsufficientFunds {
    pub available: i64,
    pub requested: i64,
}
```

Use-os no local da chamada com `?`:

```rust
use crate::errors::user_not_found::UserNotFound;

pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;

    let user = find_user(id).await
        .ok_or_else(|| FrameworkError::from(UserNotFound))?;

    json_response!({ "user": user })
}
```

A macro rejeita attributes malformados de forma evidente em tempo de
compilação - status codes que transbordam (`status = 70_000`), tipos de
literal errados (`message = 42`), chaves desconhecidas - então você não
pode acabar silenciosamente com o status errado por causa de um typo.

#### Crie um com scaffold pela CLI

```bash
suprnova make:error UserNotFound
```

Escreve `src/errors/user_not_found.rs` com um `status = 500` padrão e uma
mensagem inferida com a primeira letra maiúscula, e atualiza
`src/errors/mod.rs` para reexportá-lo. Edite o `status` e a `message` a
gosto.

### `HttpError` para o caso escrito à mão

Quando um erro de domínio precisa de estado em tempo de execução na
mensagem (por exemplo, os IDs envolvidos na falha), implemente
`HttpError` diretamente. A trait tem dois métodos com padrões sensatos:

```rust
use suprnova::HttpError;

#[derive(Debug)]
pub struct InsufficientFunds {
    pub available: i64,
    pub requested: i64,
}

impl std::fmt::Display for InsufficientFunds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Insufficient funds: have {}, need {}",
            self.available, self.requested)
    }
}

impl std::error::Error for InsufficientFunds {}

impl HttpError for InsufficientFunds {
    fn status_code(&self) -> u16 { 402 }
    fn error_message(&self) -> String {
        format!("Need {} units, only {} available.",
            self.requested, self.available)
    }
}
```

Para fazer a ponte de um `HttpError` escrito à mão até o `?`, chame
`FrameworkError::from_http_error`. Um `From<T: HttpError> for
FrameworkError` blanket entraria em conflito com o impl `From<AppError>`
existente, então a ponte é um construtor explícito:

```rust
account.withdraw(amount)
    .map_err(FrameworkError::from_http_error)?;
```

### Enums de erro para as falhas de um módulo

Quando um serviço tem várias falhas relacionadas, agrupe-as em um enum e
escreva um único `From` para o enum inteiro:

```rust
use suprnova::FrameworkError;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum OrderError {
    #[error("Order {0} not found")]
    NotFound(i64),

    #[error("Insufficient stock for product {product_id}")]
    InsufficientStock { product_id: i64 },

    #[error("Payment failed: {0}")]
    PaymentFailed(String),

    #[error("Order already shipped")]
    AlreadyShipped,
}

impl From<OrderError> for FrameworkError {
    fn from(err: OrderError) -> Self {
        let status = match &err {
            OrderError::NotFound(_) => 404,
            OrderError::InsufficientStock { .. } => 422,
            OrderError::PaymentFailed(_) => 402,
            OrderError::AlreadyShipped => 409,
        };
        FrameworkError::Domain {
            message: err.to_string(),
            status_code: status,
        }
    }
}
```

Uma vez que o `From` existe, o enum percorre o `?` como qualquer outro
tipo de erro.

## `abort_with` / `abort_if` / `abort_unless`

Três helpers fazem short-circuit em um handler em um status. Eles
espelham o `abort` / `abort_if` / `abort_unless` do Laravel. (A função
livre é exportada como `abort_with` em vez de `abort` para manter este
último disponível como nome de método em tipos do usuário.)

```rust
use suprnova::{abort_if, abort_unless, abort_with, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::user().await?.is_some(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;

    json_response!({ "ok": true })
}
```

Cada uma retorna `Result<(), FrameworkError>`, então o `?` faz o
trabalho. O erro subjacente é
`FrameworkError::Domain { message, status_code }`, que renderiza através
da mesma forma de corpo que todo outro erro. Status codes fora do
intervalo são coagidos para 500 pelo response renderer; você não precisa
se defender de input inválido no local da chamada.

## `ValidationErrors` - o conjunto de erros no formato Laravel

Quando a validação falha - no momento do `#[derive(Validate)]` ou dentro
de um corpo de `after_validation` - o framework emite a forma JSON que os
front-ends Laravel e Inertia esperam:

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["The email field must be a valid email address."],
        "password": ["The password field must be at least 8 characters."]
    },
    "request_id": "8f9e1a2b-c3d4-..."
}
```

Na maior parte do tempo você não constrói isso diretamente - o
`#[derive(Validate)]` roda e o framework converte o
`validator::ValidationErrors` para você. Quando você precisa adicionar
erros de forma imperativa (regras entre campos, verificações assíncronas
de unicidade que complementam o `Unique`), construa um
`ValidationErrors` e retorne-o:

```rust
use suprnova::{FrameworkError, ValidationErrors};

pub async fn after_validation(payload: &Signup) -> Result<(), FrameworkError> {
    let mut errs = ValidationErrors::new();

    if payload.email.ends_with("@example.com") {
        errs.add("email", "example.com addresses are not allowed");
    }
    if payload.password == payload.email {
        errs.add("password", "password must not match email");
    }

    errs.into_result().map_err(FrameworkError::Validation)
}
```

`add_to_bag` delimita um campo sob um conjunto nomeado (a forma
`withErrors($errors, 'profile')` do Laravel) prefixando o campo com o
nome do conjunto e um separador `.`. Útil quando uma resposta carrega erros de
vários subformulários que não podem compartilhar um namespace plano:

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// mapa de erros: { "profile.bio": [...], "billing.card": [...] }
```

`from_validator(ve)` converte um `validator::ValidationErrors`;
`retain_fields(&keep)` retorna uma cópia contendo apenas as entradas
listadas (usado internamente pelo header `Precognition-Validate-Only` do
Precognition).

## Conectando observabilidade com `ErrorOccurred`

Toda resposta 5xx dispara um evento `ErrorOccurred` - incluindo as
sintetizadas a partir de panics. Escute-o da mesma forma que você escuta
qualquer evento:

```rust
use std::sync::Arc;
use suprnova::{ErrorOccurred, EventFacade, FrameworkError, Listener};

pub struct SentryReporter;

#[suprnova::async_trait]
impl Listener<ErrorOccurred> for SentryReporter {
    async fn handle(&self, evt: &ErrorOccurred) -> Result<(), FrameworkError> {
        sentry::capture_message(&evt.error_message, sentry::Level::Error);
        Ok(())
    }
}

// Em bootstrap.rs:
// `listen` infere os dois genéricos a partir do tipo do listener. Ele
// retorna `()` (o registro não pode falhar), então sem `?` e sem Result.
EventFacade::listen::<ErrorOccurred, SentryReporter>(Arc::new(SentryReporter)).await;
```

O evento carrega a mensagem de erro crua (o corpo que vai pela rede
continua sanitizado - veja [Modelo de erros](error-model.md)), o status,
e o request id correlacionável. Este é o equivalente Suprnova do callback
`report()` do Laravel no handler de exceção.

## Padrões que você vai escrever muito

### Fazer parse de um parâmetro de caminho como valor tipado

```rust
let id: i64 = req.param("id")?.parse()
    .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
```

`ParamError` já converte para 400; `param_parse` é o equivalente para
falha de parse e renderiza a mesma forma.

### Buscar por ID, 404 quando ausente

```rust
let user = users::Entity::find_by_id(id)
    .one(&*DB::get()?)
    .await
    .map_err(FrameworkError::from)?
    .ok_or_else(|| FrameworkError::not_found("User"))?;
```

`map_err(FrameworkError::from)?` faz a ponte do `DbErr` do SeaORM através
de `From<DbErr> for FrameworkError` e depois através de
`From<FrameworkError> for HttpResponse`. O Rust não encadeia impls `From`
automaticamente através de dois saltos, então o `.map_err` explícito é
obrigatório.

Ou, com a camada Eloquent (que já envolve o SeaORM e retorna
`Result<_, FrameworkError>` diretamente):

```rust
use suprnova::Model;

let user = User::find_or_fail(id).await?;
```

`find_or_fail` é `find(id).ok_or(ModelNotFound)` empacotado.

### Autorizar uma ação

```rust
let user = Auth::user().await?
    .ok_or_else(|| AppError::unauthorized("login required"))?;
abort_unless(post.owner_id == user.id() || user.is_admin(), 403,
    "you don't own this post")?;
```

`abort_unless` retorna `Result<(), FrameworkError>`; o `?` o colapsa de
volta para o ramo de erro do seu handler.

### Serviço que retorna erros tipados

```rust
use suprnova::{App, FrameworkError, injectable};

#[injectable]
pub struct UserService;

impl UserService {
    pub async fn find_by_email(&self, email: &str)
        -> Result<users::Model, FrameworkError>
    {
        users::Entity::find()
            .filter(users::Column::Email.eq(email))
            .one(&*DB::get()?)
            .await?
            .ok_or_else(|| FrameworkError::not_found("User"))
    }
}

// Local da chamada:
pub async fn show(req: Request) -> Response {
    let email = req.param("email")?;
    let user = App::resolve::<UserService>()?
        .find_by_email(email)
        .await?;
    json_response!({ "user": user })
}
```

`App::resolve::<UserService>()?` retorna `Result<Arc<UserService>,
FrameworkError>`. O `?` encadeado colapsa tanto a falha de resolução
quanto a falha de lookup para uma resposta.

## Guia rápido

| Você quer… | Recorra a |
|---|---|
| Erro inline com um status | `AppError::bad_request("…")` e afins |
| Erro tipado reutilizável | `#[domain_error(status = …, message = "…")]` |
| Scaffold gerado | `suprnova make:error UserNotFound` |
| Escrito à mão com estado em tempo de execução | `impl HttpError for MyError` |
| Ponte do escrito à mão para o `?` | `FrameworkError::from_http_error(e)` |
| Short-circuit em um status | `abort_with` / `abort_if` / `abort_unless` |
| 404 em modelo ausente | `FrameworkError::not_found("User")` / `Model::find_or_fail` |
| Falha de parse em parâmetro de caminho | `FrameworkError::param_parse("id", "i64")` |
| Erro de validação em nível de campo | `FrameworkError::validation("email", "…")` |
| Conjunto de erros de vários campos | `ValidationErrors::new().add(…)` + `Validation(errs)` |
| Violação de chave duplicada → 422 | `FrameworkError::from_unique_violation(field, msg, e)` |
| Anotar um erro existente | `err.context("creating user")` |
| Observar todo 5xx | Escutar `ErrorOccurred` |
| Renderizar erros como uma página Inertia | `InertiaConfig::error_page("Error")` |

## Próximos passos

- [Modelo de erros](error-model.md) - variantes, contrato de conversão,
  sanitização para 5xx, limite de panic
- [Validação](validation.md) - `#[derive(Validate)]`, form requests, e
  `after_validation`
- [Respostas](responses.md) - builders de `HttpResponse`, status, headers
- [Eventos](events.md) - escutando `ErrorOccurred` e outros eventos
  built-in
- [Ciclo de vida da solicitação](lifecycle.md) - em que ponto do fluxo da
  solicitação a conversão de erro roda
