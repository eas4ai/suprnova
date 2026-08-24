# Modelo de erros

Este capítulo é o modelo por trás do tratamento de erros do Suprnova - os
tipos, o contrato de conversão, e as garantias de segurança que o framework
te dá de graça. Para os padrões práticos de handler do dia a dia (`?`,
retornar erros, construir erros de domínio customizados) veja
[Tratamento de erros](errors.md); este capítulo explica *por que* esses
padrões funcionam da forma como funcionam.

Se você lembrar de uma coisa só desta página: **erros no Suprnova são
valores, não exceções**. Todo erro eventualmente se torna uma
`HttpResponse` através de uma única conversão total. Não existe um
handler de exceção global porque não existe exceção global.

## A estrutura

O modelo de erros do Suprnova tem cinco peças móveis:

| Tipo | Papel |
|---|---|
| `Response = Result<HttpResponse, HttpResponse>` | O contrato que todo handler satisfaz - ambos os ramos já são responses |
| `FrameworkError` | O enum de erro canônico do framework; todo caminho de erro interno produz um |
| `AppError` | Erro de domínio ad-hoc para uso inline sem um tipo dedicado |
| `HttpError` (trait) | O que seus próprios erros de domínio tipados implementam para obter um status + mensagem |
| `ValidationErrors` | O conjunto de erros no formato Laravel/Inertia para falhas por campo |

`FrameworkError` e os tipos de erro concretos do framework usam
implementações de `From`. Um `HttpError` escrito à mão deve ser mapeado com
`FrameworkError::from_http_error` antes de `?`; não há uma implementação
blanket de `From<T: HttpError>`. A chain de middleware converte erros no limite
da solicitação, e o panic handler converte um unwind. Erros ordinários então
compartilham o renderer comum de corpo e a regra de sanitização para 5xx.

## `Response` é `Result<HttpResponse, HttpResponse>`

Todo handler retorna isto:

```rust
pub type Response = Result<HttpResponse, HttpResponse>;
```

Ambos os ramos carregam o mesmo tipo de payload, que é todo o ponto.
Quando a middleware chain termina de executar seu handler, ela colapsa o
resultado com uma linha:

```rust
result.unwrap_or_else(|e| e)
```

O framework não precisa saber se seu handler "teve sucesso" ou "falhou" -
ambos os ramos já são HTTP responses renderizadas. A distinção existe
apenas para que o `?` possa fazer seu trabalho:

```rust
use suprnova::{Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    // `?` faz short-circuit em Err. Cada conversão abaixo produz uma
    // HttpResponse via um impl From - a chain colapsa ambos os ramos.
    let id: i64 = req.param("id")?.parse().map_err(|_| {
        suprnova::FrameworkError::param_parse("id", "i64")
    })?;
    let user = User::find_or_fail(id).await?;  // 404 se ausente
    Ok(json_response!({ "user": user }))
}
```

Esse contrato único - todo caminho de erro produz uma `HttpResponse`
através de `From` - é o núcleo do modelo. Todo o resto deste capítulo é
o que os vários impls `From` realmente fazem.

### Por que Suprnova diverge

O Laravel lança exceções e as roteia através de uma classe `Handler`
global registrada em `app/Exceptions/Handler.php`. O framework captura
tudo, pergunta ao handler "o que eu renderizo?", e emite a resposta. O
modelo de exceções com unwinding do PHP torna isso natural.

Rust não tem exceções com unwinding em código de usuário. O equivalente
do Suprnova é o impl `From<FrameworkError> for HttpResponse` mais o
evento `ErrorOccurred`. A conversão é o renderer; o evento é onde você
conecta a observabilidade (Sentry, PagerDuty, structured shippers). Você
não registra uma classe handler - a conversão é uma função, e escutar
`ErrorOccurred` é o ponto de extensão. Mesma superfície, maquinaria
diferente.

## `FrameworkError` - o enum canônico

Todo caminho de erro dentro do framework - extractors, vinculação de
rota, o contêiner, validação, a camada de banco de dados, armazenamento -
produz um `FrameworkError`. É um enum com dezesseis variantes, cada uma
marcada com seu status HTTP:

```rust
pub enum FrameworkError {
    ServiceNotFound { type_name: &'static str },        // 500
    ParamError { param_name: String },                   // 400
    ValidationError { field: String, message: String },  // 422
    Database(String),                                    // 500
    Internal { message: String },                        // 500
    Domain { message: String, status_code: u16 },        // *
    Validation(ValidationErrors),                        // 422
    Unauthorized,                                        // 403
    ModelNotFound { model_name: String },                // 404
    ParamParse { param: String, expected_type: &'static str }, // 400
    UnsupportedMediaType,                                // 415
    PrecognitionSuccess,                                 // 204
    PrecognitionFailure(ValidationErrors),               // 422
    AlreadyReported,                                     // somente CLI
    RateLimited { retry_after: Option<Duration>, message: String }, // 429
    External { message: String, source: Arc<dyn Error + Send + Sync> }, // 500
}
```

Você raramente faz match na variante. Você constrói uma através de um
construtor de conveniência e deixa o `?` fazer o resto:

```rust
use suprnova::FrameworkError;

// Todos estes produzem um FrameworkError com o status certo:
FrameworkError::not_found("User");                    // → ModelNotFound, 404
FrameworkError::bad_request("Bad input");             // → Domain, 400
FrameworkError::param("user_id");                     // → ParamError, 400
FrameworkError::param_parse("user_id", "i64");        // → ParamParse, 400
FrameworkError::validation("email", "required");      // → ValidationError, 422
FrameworkError::domain("Conflict", 409);              // → Domain, 409
FrameworkError::internal("disk full");                // → Internal, 500
FrameworkError::database("timeout");                  // → Database, 500
```

Não existem construtores `unauthorized()` ou `forbidden()` em
`FrameworkError` - `Unauthorized` é uma variante fixa que carrega a
mensagem "This action is unauthorized." do Laravel em 403, e os casos
401 passam por `AppError::unauthorized` (próxima seção). Note: a
variante se chama `Unauthorized`, mas o status é 403 porque ela modela
a rejeição de autorização do Laravel, não autenticação HTTP.

### Conversão automática

`FrameworkError` implementa `From<sea_orm::DbErr>` e
`From<opendal::Error>`, então erros de banco de dados e de armazenamento
fluem através do `?` sem precisar de wrap:

```rust
use suprnova::{DB, FrameworkError};
use sea_orm::ActiveModelTrait;

pub async fn create_user(new_user: ActiveModel) -> Result<Model, FrameworkError> {
    // Ambas as chamadas `?` aqui convertem para FrameworkError automaticamente:
    // - DB::get retorna Result<_, FrameworkError>
    // - insert retorna Result<_, DbErr>, que tem From<DbErr> para FrameworkError
    let user = new_user.insert(&*DB::get()?).await?;
    Ok(user)
}
```

Se seu código retorna `Result<_, FrameworkError>`, todo erro comum que
suas dependências produzem já fala a linguagem certa. O `?` do
controller não faz nenhum trabalho além de converter um tipo de erro em
outro.

### Envolvendo contexto

Quando você precisa relançar um erro com contexto da operação, use
`.context()`:

```rust
db.insert(user).await
    .map_err(FrameworkError::from)
    .map_err(|e| e.context("creating new user"))?;
```

A mensagem se torna `"creating new user: <original>"`. A variante é
preservada onde importa - `Validation`, `ValidationError`,
`PrecognitionFailure`, `PrecognitionSuccess`, `Unauthorized`,
`ModelNotFound`, `ParamParse`, `UnsupportedMediaType`,
`AlreadyReported`, `RateLimited` e `External` mantêm sua estrutura para
que o renderer de resposta ainda emita a forma correta (e, no caso de
`External`, para que a fonte envolvida sobreviva). Variantes que só
carregam mensagem (`Internal`, `Database`, `Domain`) se achatam em uma
`Domain` com a mensagem prefixada.

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

`RateLimited` existe para que uma dica downstream de `Retry-After`
sobreviva à passagem pelo sistema de erros como uma `Duration`, em vez
de colapsar em texto de mensagem:

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

`retry_after()` retorna `None` para toda outra variante e para
throttles que chegaram sem dica. A variante renderiza como HTTP 429, e
`.context(...)` a preserva em vez de achatá-la para `Domain`, portanto a
duração nunca é removida ao adicionar contexto da operação.

## `AppError` - erros de domínio ad-hoc

Para erros pontuais, onde você não quer definir um tipo dedicado, use
`AppError`. Ele implementa `HttpError` e tem um `From` para
`FrameworkError`, então o `?` funciona diretamente:

```rust
use suprnova::{AppError, Request, Response, json_response};

pub async fn transfer(req: Request) -> Response {
    let amount: i64 = req.param("amount")?.parse()
        .map_err(|_| AppError::bad_request("amount must be a number"))?;

    if amount <= 0 {
        return Err(AppError::unprocessable("amount must be positive").into());
    }

    if amount > 1_000_000 {
        return Err(AppError::forbidden("amount exceeds daily limit").into());
    }

    Ok(json_response!({ "transferred": amount }))
}
```

Os construtores mapeiam de forma direta para a forma `abort($status,
$msg)` do Laravel:

| `AppError::*` | Status |
|---|---|
| `bad_request(msg)` | 400 |
| `unauthorized(msg)` | 401 |
| `forbidden(msg)` | 403 |
| `not_found(msg)` | 404 |
| `conflict(msg)` | 409 |
| `unprocessable(msg)` | 422 |
| `new(msg)` | 500 |
| `.status(code)` | qualquer |

Note que `AppError::unauthorized` é **401** (autenticação HTTP
ausente), enquanto `FrameworkError::Unauthorized` é **403** (autorização
negada, correspondendo à rejeição de policy do Laravel). Eles
significam coisas diferentes; escolha o que corresponde à falha.

## `HttpError` - erros tipados customizados

Quando o mesmo erro de domínio aparece em muitos lugares, modele-o como
um tipo. Implemente `HttpError` e a conversão é sua:

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

`HttpError` tem dois métodos, ambos com padrões:

```rust
pub trait HttpError: std::error::Error + Send + Sync + 'static {
    fn status_code(&self) -> u16 { 500 }
    fn error_message(&self) -> String { self.to_string() }
}
```

### Conectando com `?`

Um `impl<T: HttpError> From<T> for FrameworkError` ingênuo entraria em
conflito com o impl `From<AppError>` existente (porque o próprio
`AppError` implementa `HttpError`). Em vez disso, o Suprnova resolve o
problema da orphan rule com um construtor de ponte explícito:

```rust
use suprnova::{FrameworkError, HttpError};

pub async fn debit(account: &mut Account, amount: i64) -> Result<(), FrameworkError> {
    account.withdraw(amount)
        .map_err(FrameworkError::from_http_error)?;
    Ok(())
}
```

O status code e a mensagem são tirados de `HttpError::status_code` e
`HttpError::error_message` e armazenados em uma variante
`FrameworkError::Domain`. O response renderer então segue o caminho
normal de `Domain`.

### `#[domain_error]` para tipos sem boilerplate

Se você quer o padrão de erro tipado sem escrever os impls `Display`,
`Error`, e `HttpError` à mão, use o attribute macro `#[domain_error]`:

```rust
use suprnova::domain_error;

#[domain_error(status = 404, message = "User not found")]
pub struct UserNotFoundError;

#[domain_error(status = 402, message = "Insufficient funds")]
pub struct InsufficientFundsError {
    pub available: i64,
    pub requested: i64,
}
```

`#[domain_error]` gera o conjunto completo de impls, *incluindo*
`From<YourError> for FrameworkError`, então o `?` funciona diretamente
sem precisar de uma chamada de ponte:

```rust
pub async fn show(req: Request) -> Response {
    let id: i64 = req.param("id")?.parse()
        .map_err(|_| FrameworkError::param_parse("id", "i64"))?;
    let user = User::find(id).await?
        .ok_or_else(|| FrameworkError::from(UserNotFoundError))?;
    Ok(json_response!({ "user": user }))
}
```

As três camadas da história de erro customizado - `AppError` para uso
inline, `#[domain_error]` para tipado-com-macro, `HttpError` escrito à
mão para controle total - te dão a ferramenta certa em cada nível de
formalidade.

## `ValidationErrors` - o conjunto de erros no formato Laravel

Quando uma solicitação falha na validação, o Suprnova emite a mesma
forma JSON que os front-ends Laravel e Inertia esperam:

```json
{
    "message": "The given data was invalid.",
    "errors": {
        "email": ["The email field must be a valid email address."],
        "password": ["The password must be at least 8 characters."]
    },
    "request_id": "8f9e1a2b-c3d4-..."
}
```

Você normalmente não constrói isso à mão - `#[derive(Validate)]` em um
form request e o crate `validator` por trás dele produzem um
`validator::ValidationErrors`, que o Suprnova converte via
`ValidationErrors::from_validator`. Mas o tipo é público quando você
precisar dele:

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

`add_to_bag` delimita erros sob um conjunto nomeado (a forma
`withErrors($errors, 'profile')` do Laravel) prefixando o nome do
conjunto com um separador `.`:

```rust
let mut errs = ValidationErrors::new();
errs.add_to_bag("profile", "bio", "must be under 280 characters");
errs.add_to_bag("billing", "card", "expired");
// errors map: { "profile.bio": [...], "billing.card": [...] }
```

`retain_fields` mantém apenas as entradas listadas - usado internamente
pelo header `Precognition-Validate-Only` do Precognition para que o
servidor rode a validação completa mas só reporte erros para os campos
que o cliente perguntou sobre.

## O contrato de conversão

Quando um `FrameworkError` alcança um limite HTTP, ele passa por
`From<FrameworkError> for HttpResponse`. Três coisas acontecem, em
ordem:

1. **Roteamento de status**. O `status_code()` da variante é lido uma
   vez.
2. **Logging + observabilidade**. 5xx dispara `tracing::error!` e faz
   dispatch de `ErrorOccurred`; 4xx dispara `tracing::warn!`. Ambos
   carregam o request id quando um está em escopo.
3. **Renderização do corpo**. Um corpo JSON na forma do Laravel,
   sanitizado para 5xx.

### A forma ordinária do corpo

Respostas de erro ordinárias que alcançam o renderer comum seguem este
esqueleto JSON:

```json
{
    "message": "<human readable>",
    "errors": { "field": ["msg", ...] },
    "request_id": "<uuid>" | null,
    "debug_message": "<dev only>"
}
```

- `message` sempre está presente nessas respostas ordinárias.
- `errors` só aparece para erros no estilo de validação
  (`Validation`, `ValidationError`) - ambos renderizam a mesma forma para que os
  consumidores façam parse de um único caminho.
- `request_id` aparece nessas respostas ordinárias (`null` quando fora de um
  escopo de solicitação, como durante boot inicial ou em testes sem contexto de
  solicitação).
- `debug_message` só aparece para 5xx quando `APP_DEBUG=true`. É estritamente
  aditivo - clientes de produção não devem se basear nele.

Três variantes especiais retornam antes da injeção de request id:

- `PrecognitionSuccess` é uma resposta 204 sem corpo.
- `PrecognitionFailure` contém o corpo de validação mais headers de
  Precognition.
- Um sentinela `AlreadyReported` renderizado acidentalmente por HTTP é uma
  resposta 500 genérica que contém apenas `message`.

### A regra de sanitização para 5xx

Esta é a garantia de segurança que vale a pena memorizar. Para
qualquer erro com status ≥ 500, o `message` do corpo JSON é substituído
pela string literal:

```json
{ "message": "Internal Server Error", "request_id": "..." }
```

O detalhe bruto do erro **não** vaza para o corpo da resposta. Ele vai
para:

- a entrada de log do `tracing::error!`, com o request id e o status
- o evento `ErrorOccurred`, que qualquer listener pode capturar

Quando `APP_DEBUG=true` (falso por padrão fora de `local`/`dev`/`test`),
a resposta também carrega um campo `debug_message` com o detalhe bruto,
mas o `message` permanece genérico em ambos os modos, então frontends e
clientes não podem acidentalmente se acoplar a dados de somente-dev.

Este é o contrato que permite que você chame
`FrameworkError::internal("db connection refused: password mismatch on
user 'app_rw'")` sem vazar a senha para a rede. O `message` que você
passa é para operadores lendo logs; o `message` que o cliente vê é
`"Internal Server Error"`.

Para erros 4xx, a mensagem voltada ao chamador é preservada - `404 User
not found`, `400 Missing required parameter: user_id`. Estes são erros
de domínio sobre os quais o cliente precisa agir, não falhas internas.

### Onde o contrato vive

A conversão inteira é uma única função - `impl
From<FrameworkError> for HttpResponse` em
`framework/src/http/response.rs`. Leia-a uma vez e você terá lido toda
a superfície de renderização de erros do Suprnova. Não há outro
caminho.

## O limite de panic

Um panic em um middleware ou handler, de outra forma, se propagaria
pela task por conexão e derrubaria o serviço hyper no meio da resposta,
deixando o cliente com um TCP reset e nenhuma resposta HTTP. O
Suprnova o captura.

`execute_chain_safely` em `framework/src/server.rs` envolve a
middleware chain em `AssertUnwindSafe(...).catch_unwind().await`. Em
um panic, ela:

1. Extrai o payload do panic (trata payloads `&'static str` e
   `String`; qualquer outra coisa aparece como `"panic with
   non-string payload"`).
2. Registra `tracing::error!` com o método, o path e o id da
   solicitação.
3. Constrói `FrameworkError::internal(format!("request handler
   panicked: {msg}"))` e o roteia através da *mesma* conversão
   `From<FrameworkError> for HttpResponse` que todo outro 5xx usa.
4. Ecoa o request id de volta como `X-Request-Id`.

O payload do panic fica na entrada de log; o cliente recebe o corpo
sanitizado `{"message": "Internal Server Error"}`. Listeners de
observabilidade que disparam em `ErrorOccurred` para erros 5xx
retornados também disparam em panics - não há uma superfície de evento
de panic separada para conectar.

O mesmo padrão de recuperação de panic é usado por:

- handlers WebSocket (`framework/src/server.rs`)
- tarefas agendadas (`framework/src/schedule/mod.rs`)
- workflows (`framework/src/workflow/mod.rs`)
- o trait `Supervisor` (transmissão)

Um panic em um destes subsistemas é registrado em log e ou é traduzido
para um estado de erro ou reiniciado automaticamente; ele não derruba
a worker task.

## Conectando observabilidade com `ErrorOccurred`

`ErrorOccurred` é um evento built-in que o framework despacha em toda
resposta 5xx (incluindo as sintetizadas a partir de panics):

```rust
pub struct ErrorOccurred {
    pub error_message: String,
    pub status_code: u16,
    pub request_id: Option<String>,
}
```

Escute-o da mesma forma que você escuta qualquer evento:

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
EventFacade::listen::<ErrorOccurred, _>(Arc::new(SentryReporter)).await;
```

Este é o equivalente Suprnova do callback `report()` do Laravel no
handler de exceção global. O evento chega com o `error_message`
original não sanitizado (o corpo que o cliente vê continua
sanitizado), o status code, e o request id correlacionável.

### Renderizando a cadeia completa: `render_error_chain`

O `Display` gerado por `thiserror` imprime somente a própria mensagem de
um erro, portanto a `source` envolvida de um `FrameworkError::External`
fica invisível a menos que algo percorra a cadeia. `render_error_chain`
faz esse percurso e une o resultado com `": "`, o mesmo separador que
`.context()` usa - o framework o chama antes de construir
`error_message` acima e antes da linha de log 5xx correspondente, razão
pela qual um erro envolvido não perde sua causa em nenhum dos dois
lugares.

Use-o você mesmo quando um listener ou um destino de logs precisar da
mesma renderização de cadeia completa, por exemplo, para envolver
novamente `error_message` antes de encaminhá-lo a um destino que só
aceita uma string plana:

```rust
use suprnova::render_error_chain;

let chain = render_error_chain(&err);
// "loading users: connection refused (os error 111)"
```

## Auxiliares de abort

Três funções livres fazem short-circuit em um handler em um status
dado. Elas espelham o `abort` / `abort_if` / `abort_unless` do
Laravel:

```rust
use suprnova::{abort_with, abort_if, abort_unless, Auth, Request, Response, json_response};

pub async fn show(req: Request) -> Response {
    abort_unless(Auth::check(), 401, "must be logged in")?;
    abort_if(req.param("id")? == "0", 404, "User not found")?;
    abort_with(503, "scheduled maintenance")?;
    Ok(json_response!({ "ok": true }))
}
```

Cada uma retorna `Result<(), FrameworkError>`. Use-as com `?`. O erro
subjacente é `FrameworkError::Domain { message, status_code }`, então
ele renderiza através da mesma forma de corpo e das mesmas regras de
sanitização que todo outro erro. Status codes fora do intervalo válido
são coagidos para 500 pela validação de status do response renderer;
você não precisa se defender de input inválido no local da chamada.

## O sentinela de CLI: `AlreadyReported`

Uma variante de `FrameworkError` não tem significado HTTP.
`AlreadyReported` é construída via `FrameworkError::silent()` e usada
pelo console dispatcher quando o clap já formatou e imprimiu seu
próprio erro de parse de argumento. O `main` do binário traduz o
sentinela para um exit code não-zero sem `eprintln`, então usuários
nunca veem duas mensagens de erro para a mesma falha.

Se `AlreadyReported` alguma vez alcançar um conversor de resposta HTTP,
isso indica que um request handler retornou `silent()` acidentalmente.
O conversor registra um `tracing::error!` alto identificando o vazamento e
retorna um 500 genérico que contém apenas
`{"message": "Internal Server Error"}`. A variante não tem motivo para estar no
caminho de solicitação, e o log alto torna o bug observável em vez de silencioso.

Você normalmente não vê esta variante; ela está documentada aqui
porque o enum é `HTTP-flavoured` e a variante, do contrário
inexplicada, deixaria confuso qualquer um lendo o código-fonte.

## Garantias de segurança, em resumo

O contrato que o Suprnova te dá:

- **Conversão total**. Todo `FrameworkError` produz uma `HttpResponse`.
  Não há caminho de erro que derrube o servidor ou encerre a conexão
  silenciosamente.
- **5xx sanitizado**. O corpo que vai pela rede para qualquer 5xx é o
  genérico `{"message": "Internal Server Error", "request_id": "..."}`.
  O detalhe flui para os logs + `ErrorOccurred`.
- **Visibilidade de debug opcional**. `APP_DEBUG=true` adiciona um
  campo `debug_message` para 5xx, nunca `message`. Clientes de
  produção não podem se acoplar acidentalmente a dados de
  somente-dev.
- **Request ids correlacionáveis**. Todo corpo de erro carrega o
  request id (ou `null` quando não existe escopo de solicitação); o
  mesmo id aparece na linha de log e no evento `ErrorOccurred`.
- **Recuperação de panic**. Panics em handlers e middleware são
  capturados, registrados em log, e roteados através do mesmo impl
  `From` que erros retornados. Sem drop de conexão, sem lacuna de
  observabilidade.
- **Uma forma para tudo**. Erros de validação, erros de parâmetro,
  panics, erros de domínio customizados, e falhas de armazenamento
  todos colapsam para o mesmo esqueleto JSON. Código de frontend faz
  parse de uma única estrutura.

## Onde cada peça vive

| Peça | Arquivo |
|---|---|
| `FrameworkError`, `AppError`, `HttpError`, `ValidationErrors` | `framework/src/error.rs` |
| `From<FrameworkError> for HttpResponse` (conversão + sanitização) | `framework/src/http/response.rs` |
| `abort`, `abort_if`, `abort_unless` | `framework/src/http/abort.rs` |
| `execute_chain_safely` (limite de panic) | `framework/src/server.rs` |
| evento `ErrorOccurred` | `framework/src/events/builtins.rs` |
| macro `#[domain_error]` | `suprnova-macros/src/domain_error.rs` |
| `render_error_chain` | `framework/src/error.rs` |

## Próximos passos

- [Tratamento de erros](errors.md) - os padrões práticos de handler
  que usam este modelo
- [Ciclo de vida da solicitação](lifecycle.md) - em que ponto do fluxo
  da solicitação a conversão de erro roda
- [Validação](validation.md) - `#[derive(Validate)]`, form requests, e
  como `ValidationErrors` é populado
- [Respostas](responses.md) - builders de `HttpResponse`, headers,
  cookies, streaming
- [Eventos](events.md) - escutando `ErrorOccurred` e outros eventos
  built-in
