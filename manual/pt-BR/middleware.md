# Middleware

Middleware envolve um handler de solicitação. Ele executa antes de o
handler ver a solicitação e novamente depois de o handler retornar uma
resposta, então é o lugar para colocar trabalho transversal - auth,
logging, CORS, throttling, timing, transformando a solicitação ou a
resposta. A superfície do Suprnova é a mesma que usuários do Laravel já
conhecem: um método `handle(request, next)` que decide se encaminha a
solicitação, faz short-circuit nela, ou modifica a resposta na volta.

## A trait

Um middleware é um struct que implementa `Middleware`:

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct LoggingMiddleware;

#[async_trait]
impl Middleware for LoggingMiddleware {
    async fn handle(&self, request: Request, next: Next) -> Response {
        // Pré-processamento: executa antes do handler.
        println!("--> {} {}", request.method(), request.path());

        // Encaminha para o próximo middleware (ou o handler, se esta
        // for a última camada).
        let response = next(request).await;

        // Pós-processamento: executa depois que o handler retorna.
        println!("<-- complete");

        response
    }
}
```

`handle` tem três coisas a fazer, e você só precisa fazer uma delas em
qualquer solicitação dada:

- **Encaminhar.** Chame `next(request).await` para passar o controle
  para a próxima camada. O `Response` retornado é o que toda camada
  acima vai ver.
- **Short-circuit.** Retorne `Err(HttpResponse::...)` sem chamar
  `next`. O framework colapsa ambos os ramos de `Response`
  (`Result<HttpResponse, HttpResponse>`) em uma única resposta - um
  `Err` é uma resposta, não um crash. Veja [Modelo de erros](error-model.md).
- **Modificar.** Modifique a solicitação antes de encaminhar, ou
  modifique a resposta depois.

`Next` é `Arc<dyn Fn(Request) -> MiddlewareFuture + Send + Sync>` -
trate-o como uma função async de `Request` para `Response`.

## Gerando um stub

A CLI gera com scaffold um arquivo de middleware funcional:

```bash
suprnova make:middleware Auth         # → src/middleware/auth.rs (AuthMiddleware)
suprnova make:middleware RateLimit    # → src/middleware/rate_limit.rs
suprnova make:middleware CorsMiddleware  # sufixo "Middleware" tudo bem, mesmo resultado
```

O arquivo gerado não é um stub de TODO - é um middleware de verdade que
cronometra a solicitação envolvida e registra em log os eventos de
entrada/saída com o id por solicitação instalado por
`RequestIdMiddleware`. Substitua o corpo pelo que você realmente
precisar.

## Registrando middleware

Três lugares para instalá-lo, dependendo do escopo:

### Global

Executa em toda solicitação, na ordem de registro. Use a macro
`global_middleware!` dentro de `bootstrap()`:

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, FrameworkError};
use crate::middleware;

pub async fn bootstrap() -> Result<(), FrameworkError> {
    global_middleware!(middleware::LoggingMiddleware);
    global_middleware!(middleware::CorsMiddleware);
    Ok(())
}
```

`global_middleware!(M)` se expande para `register_global_middleware(M)`.
O registro é **idempotente por tipo concreto** - registrar o mesmo
struct duas vezes mantém o primeiro registro e emite um log de debug.
Isso torna reexecutar a inicialização (testes, hot-reload, múltiplas
instâncias de `Server` em um processo) seguro. Para instalar várias
cópias do mesmo comportamento com config diferente, envolva cada uma em
um newtype distinto.

### Por rota

Encadeie `.middleware(M)` em uma definição de rota da macro `routes!`:

```rust
// src/routes.rs
use suprnova::{routes, get};
use crate::{controllers, middleware::AuthMiddleware};

routes! {
    get!("/", controllers::home::index).name("home"),
    get!("/public", controllers::home::public),

    get!("/protected", controllers::dashboard::index)
        .middleware(AuthMiddleware),
    get!("/admin", controllers::admin::index)
        .middleware(AuthMiddleware),
}
```

### Por grupo

Aplique middleware a toda rota em um bloco `group(...)`:

```rust
use suprnova::Router;
use crate::middleware::{ApiMiddleware, AuthMiddleware};
use crate::controllers::{user, admin};

Router::new()
    // Rotas públicas - sem middleware.
    .get("/", home_handler)
    .get("/login", login_handler)

    // Toda rota sob /api carrega ApiMiddleware.
    .group("/api", |r| {
        r.get("/users", user::index)
         .post("/users", user::store)
         .get("/users/{id}", user::show)
    })
    .middleware(ApiMiddleware)

    // Rotas admin compartilham auth.
    .group("/admin", |r| {
        r.get("/dashboard", admin::dashboard)
         .get("/settings", admin::settings)
    })
    .middleware(AuthMiddleware);
```

## Ordem de execução

Em runtime a chain executa de fora para dentro:

```
Solicitação  →  RequestId  →  globais  →  MW de grupo  →  MW de rota  →  handler
                                                                            │
Resposta     ←  RequestId  ←  globais  ←  MW de grupo  ←  MW de rota  ←  handler
```

O primeiro middleware adicionado executa primeiro. Na volta, a ordem se
inverte - `MiddlewareChain::execute` aninha o pós-processamento de cada
camada dentro da anterior.

Se um middleware faz short-circuit com `Err(response)`, a chain se
desenrola imediatamente: toda camada ACIMA do short-circuit ainda vê a
resposta na volta, mas as camadas ABAIXO (mais perto do handler) não
executam.

### Middleware de grupo é achatado, não empilhado

Este ponto importa e vale a pena destacar. **Middleware de grupo não é
uma camada de runtime separada.** Quando `GroupBuilder::try_finalize`
executa, ele copia o middleware do grupo para a lista de middleware
`(method, pattern)` de cada rota agrupada. No momento da execução,
middleware de grupo é indistinguível de middleware anexado diretamente
à rota.

Duas consequências:

- A ordenação em runtime continua correta (middleware de grupo executa
  antes do middleware de rota porque é registrado primeiro), mas **a
  introspecção não consegue diferenciar middleware de grupo de
  middleware de rota**.
- Middleware é chaveado pelo padrão correspondido (`"/posts/{id}"`),
  não pelo caminho bruto (`/posts/42`), então middleware de grupo em
  rotas parametrizadas dispara de forma confiável.

Veja `framework/src/routing/group.rs` para o passo de achatamento e
`framework/src/middleware/chain.rs` para o loop de execução.

## Fazendo short-circuit

Retorne antecipadamente para bloquear uma solicitação antes que ela
chegue ao handler:

```rust
use suprnova::{async_trait, HttpResponse, Middleware, Next, Request, Response};

pub struct RequireApiKey;

#[async_trait]
impl Middleware for RequireApiKey {
    async fn handle(&self, request: Request, next: Next) -> Response {
        if request.header("X-Api-Key").is_none() {
            return Err(HttpResponse::text("Unauthorized").status(401));
        }
        next(request).await
    }
}
```

A chain colapsa `Result<HttpResponse, HttpResponse>` para uma única
resposta, então `Err(...)` é apenas uma resposta com um papel
diferente. As camadas acima deste middleware ainda a observam na volta
e podem pós-processá-la.

## Segurança contra panic

`MiddlewareChain::execute` NÃO captura panics - um panic em qualquer
middleware ou no handler se desenrola direto para fora, como qualquer
outra função async. A rede de segurança do caminho de solicitação vive
um nível acima, na fronteira do servidor, em `execute_chain_safely`,
que envolve a chain em `catch_unwind` e converte um panic em um 500
sanitizado com o request id, despachando `ErrorOccurred` para qualquer
listener de observabilidade. Veja
[Ciclo de vida da solicitação](lifecycle.md) para o fluxo completo de
recuperação de panic.

Essa divisão é deliberada: o tratamento padronizado de panic acontece
exatamente uma vez, onde o ciclo de vida da solicitação é dono dele, em
vez de ser duplicado dentro do primitivo agnóstico de camada. Um
consumidor que executa uma chain fora dessa fronteira é responsável
pelo seu próprio `catch_unwind`.

## Middleware embutido

Um mapa não exaustivo. Cada um vem pronto para instalar - a maioria
precisa de uma struct de config, nenhum precisa de scaffolding.

| Middleware | Propósito |
|---|---|
| `RequestIdMiddleware` | Camada sempre mais externa; atribui um UUID por solicitação e o marca através dos logs + `X-Request-Id` |
| `TimeoutMiddleware` | Limita o tempo até a resposta; retorna 503 quando excedido (veja abaixo) |
| `CorsMiddleware` | Trata o preflight de CORS + decora respostas cross-origin (veja abaixo) |
| `CsrfMiddleware` | Proteção CSRF por double-submit de cookie, com `OriginPolicy` configurável |
| `RateLimitMiddleware` / `ThrottleRequestsMiddleware` | Throttling de token bucket e de janela deslizante; veja [Limitação de taxa](rate-limiting.md) |
| `SessionMiddleware` | Carrega/persiste a sessão sobre cookies; alimenta `req.session()` |
| `AuthMiddleware` / `GuestMiddleware` / `BearerTokenMiddleware` | Verificações de participação em guard; veja [Autenticação](authentication.md) |
| `LoginThrottleMiddleware` / `EnsureEmailVerifiedMiddleware` / `TwoFactorChallengeMiddleware` | Gates de fluxo de autenticação; veja [Fluxos de autenticação](auth-flows.md) |
| `MaintenanceMiddleware` | Retorna 503 quando a flag de manutenção do cache ou do filesystem está definida |
| `InertiaHeadersMiddleware` / `InertiaVersionMiddleware` / `Inertia303Middleware` / `InertiaValidationRedirectMiddleware` / `EncryptHistoryMiddleware` | Protocolo Inertia: `Vary: X-Inertia` em toda resposta e redirecionamento de volta em 200 vazio; bounce 409 de versão de ativo; 302→303 em redirecionamentos não-GET; um 422 em uma visita Inertia torna-se um 303 de volta com os erros em flash; criptografia de histórico. `Inertia::install` registra os quatro primeiros; `EncryptHistoryMiddleware` é opt-in separadamente. Veja [Respostas Inertia](frontend-inertia-responses.md#bootstrap-inertia-install) |
| `IncludeMiddleware` | Conjuntos de include por campo para reloads parciais de `#[derive(Data)]` |

### Tempos limite de solicitação

`TimeoutMiddleware` limita quanto tempo um handler pode levar para
*produzir* uma resposta. Um handler lento ou uma consulta de banco de
dados travada poderia, de outra forma, manter uma conexão aberta
indefinidamente; o timeout retorna `503 Service Unavailable` assim que
o prazo é excedido.

```rust
// src/bootstrap.rs - teto de 30 segundos em toda rota HTTP.
use suprnova::{global_middleware, TimeoutMiddleware};

global_middleware!(TimeoutMiddleware::default()); // DEFAULT_TIMEOUT = 30s
```

```rust
// Aperta um único endpoint para 5 segundos.
use suprnova::{Router, TimeoutMiddleware};

Router::new()
    .get("/report", heavy_report_handler)
    .middleware(TimeoutMiddleware::seconds(5));
```

`TimeoutMiddleware::new(Duration)` aceita qualquer duração;
`TimeoutMiddleware::seconds(n)` é um atalho para segundos inteiros.

Middleware global roda **fora** do middleware de rota, então um timeout
global é um teto externo e um timeout por rota só pode tornar uma rota
específica *mais estrita* - o prazo mais curto dispara primeiro. Para
deixar uma rota rodar por mais tempo que o padrão global, aumente o
valor global ou dê escopo ao middleware global para um grupo de rotas
que exclua aquele endpoint.

Respostas de streaming (`HttpResponse::sse(...)`,
`HttpResponse::stream_bytes(...)`) são naturalmente isentas: o handler
retorna imediatamente com um corpo lazy que o hyper drena depois que a
chain de middleware completa. Upgrades de WebSocket também são pulados
explicitamente. Veja [Timeouts](timeout.md) para a semântica de
cancel-safety.

### CORS

`CorsMiddleware` adiciona os headers `Access-Control-*` de que um
navegador precisa para deixar uma página cross-origin ler suas
respostas, e responde à solicitação `OPTIONS` de preflight que os
navegadores enviam antes de chamadas cross-origin não simples. Apps de
mesma origem (a configuração Inertia padrão) não precisam dele - ele só
importa quando um navegador em uma origem *diferente* chama sua API.

O CORS precisa ser instalado **globalmente** para que os preflights
cheguem até ele (um preflight nunca corresponde a uma rota, então um
middleware de CORS por rota nunca veria um). Não existe,
intencionalmente, um padrão permissivo - escolha uma política de origem
explicitamente:

```rust
// src/bootstrap.rs
use suprnova::{global_middleware, CorsConfig, CorsMiddleware};

global_middleware!(CorsMiddleware::new(
    CorsConfig::allow_origins(["https://app.example", "https://admin.example"])
        .allow_credentials(true)
        .max_age(std::time::Duration::from_secs(600)),
));
```

`CorsConfig::any_origin()` opta explicitamente por
`Access-Control-Allow-Origin: *`. Métodos do builder: `.methods([...])`,
`.allow_headers([...])` / `.allow_any_headers()`,
`.expose_headers([...])`, `.paths([...])` (dá escopo ao CORS por
padrões de URL), `.allow_origin_patterns([regex...])`,
`.skip_when(|req| bool)`, `.allow_credentials(bool)`,
`.max_age(Duration)`. Aliases com nomes do Laravel vêm junto (por
exemplo, `.supports_credentials`, `.allowed_methods`) para que uma
config do Laravel mapeie diretamente.

`Access-Control-Allow-Origin: *` é inválido junto com credenciais - o
navegador o rejeita. Quando `.allow_credentials(true)` está definido, o
middleware sempre ecoa o `Origin` específico da solicitação em vez de
`*`, então a combinação inválida nunca pode ser emitida. Respostas sem
wildcard também recebem `Vary: Origin` para que caches compartilhados
permaneçam corretos. Veja [CORS](cors.md).

## Pipeline - o `Illuminate\Pipeline\Pipeline` do Laravel

`Pipeline` é o análogo Suprnova da classe pipeline do Laravel - um
builder fluente sobre `MiddlewareChain` que espelha a forma `send /
through / pipe / then / then_return / finally_with` que usuários do
Laravel já conhecem. Útil quando você quer montar uma chain de
middleware fora do ciclo de vida da solicitação (um job, um comando
CLI, um teste de integração avulso):

```rust
use suprnova::{Pipeline, Request};

let response = Pipeline::new()
    .send(request)
    .through([AuthMiddleware, LoggingMiddleware])
    .pipe(CorsMiddleware::new(cors_config))
    .finally_with(|| tracing::info!("pipeline complete"))
    .then(|req| async move { handler(req).await })
    .await;
```

Aliases do lado Rust vêm junto com os nomes do Laravel: `with_request`
para `send`, `with_middleware` para `through`, `push` para `pipe`,
`on_finally` para `finally_with`, `execute` para `then`. Use o que
melhor se ler no seu código.

| Método do Pipeline | Laravel | Alias Rust | Finalidade |
|---|---|---|---|
| `send(request)` | `send($passable)` | `with_request(request)` | Define a solicitação sendo passada pela chain |
| `through(iter)` | `through($pipes)` | `with_middleware(iter)` | Substitui a lista de pipes |
| `through_boxed(iter)` | - | - | Substitui a lista de pipes com middleware pré-boxado |
| `pipe(M)` | `pipe($pipes)` | `push(M)` | Adiciona um único middleware |
| `pipe_boxed(M)` | - | - | Adiciona um middleware pré-boxado |
| `then(destination)` | `then($destination)` | `execute(destination)` | Executa a chain com o handler de destino |
| `then_with(req, dst)` | - | - | Sobrescreve o passable inline |
| `then_return()` | `thenReturn()` | - | Executa a chain, retorna 204 No Content |
| `finally_with(F)` | `finally($callback)` | `on_finally(F)` | Executa depois que o destino resolve |

## Middleware terminável - hooks pós-resposta

Middleware terminável executa *depois* que a resposta foi enviada ao
cliente. Use-o para IO lento que não precisa bloquear a resposta:
persistência de sessão, audit logging, flushes de métricas.

O Suprnova entrega isso como uma trait `Terminable` dedicada, separada
de `Middleware`, para que o caminho de solicitação e o caminho de
terminação permaneçam claramente tipados. Um tipo pode implementar uma,
a outra, ou ambas:

```rust
use suprnova::{Terminable, TerminationSnapshot, register_terminable, async_trait};

pub struct AuditLogTerminator;

#[async_trait]
impl Terminable for AuditLogTerminator {
    async fn terminate(&self, snapshot: &TerminationSnapshot) {
        tracing::info!(
            method = %snapshot.method,
            path = %snapshot.path,
            status = snapshot.status,
            "request handled",
        );
    }
}

// Em bootstrap.rs
register_terminable(AuditLogTerminator);
```

O servidor itera os terminables registrados na ordem de registro depois
de toda resposta (4xx e 5xx incluídos) e aguarda cada um. Erros são
registrados em log via `tracing::error!` e engolidos - a resposta já
saiu, então não há mais ninguém para quem reportá-los.

O registro é idempotente por tipo concreto. `registered_terminables()`,
`terminable_count()`, e `has_terminable::<T>()` fornecem introspecção
para testes e diagnósticos de boot-time.

## Aliases e grupos nomeados

Para consumidores que preferem middleware chaveado por string
(`middlewareAliases` / `middlewareGroups` do Laravel), o Suprnova
entrega um registro process-global de aliases + grupos:

```rust
use suprnova::middleware::{
    register_middleware_alias, register_middleware_group,
    resolve_middleware_group,
};

// Aliases são closures de factory - invocadas do zero a cada
// resolução, então cada registro de rota produz uma instância de
// middleware independente.
register_middleware_alias("auth", || AuthMiddleware::new());
register_middleware_alias("throttle", || ThrottleRequestsMiddleware::default());

// Grupos agrupam aliases. Grupos aninhados são suportados.
register_middleware_group("api", ["auth".into(), "throttle".into()]);
register_middleware_group("web", ["session".into(), "auth".into()]);

// Resolve para um Vec<BoxedMiddleware> na inicialização ou por rota.
let api_mws = resolve_middleware_group("api")?;
```

`resolve_middleware_group` retorna `Err(MiddlewareResolveError)` em:

- `UnknownGroup(name)` - o grupo nomeado nunca foi registrado;
- `UnknownAlias { group, missing }` - uma entrada do grupo não é um
  alias conhecido;
- `UnknownNestedGroup { group, missing }` - a referência a um grupo
  aninhado falha ao resolver;
- `CycleDetected { group }` - a definição do grupo é recursiva.

O registro de um alias ou grupo é **o último que vence** para o mesmo
nome, espelhando o array de kernel reatribuível do Laravel.

## Prioridade de middleware

`prepend_middleware_priority::<M>()` /
`append_middleware_priority::<M>()` registram um `TypeId` na lista de
prioridade process-global - o análogo Suprnova de
`Kernel::$middlewarePriority` do Laravel. Middleware cujo tipo aparece
mais cedo na lista é ordenado para a frente da chain independentemente
da ordem de registro:

```rust
use suprnova::{append_middleware_priority};

// SessionMiddleware sempre executa antes de AuthMiddleware,
// independentemente da ordem em que foram registrados.
append_middleware_priority::<SessionMiddleware>();
append_middleware_priority::<AuthMiddleware>();
```

`middleware_priority()` retorna um snapshot do `Vec<TypeId>` atual para
diagnósticos ou para um embarcador que queira usar seu próprio sorter.

## Introspecção do registro

Além de `register_global_middleware`, o registro expõe:

| Superfície | Laravel | Finalidade |
|---|---|---|
| `prepend_global_middleware(M)` | `prependMiddleware` | Insere no início da chain |
| `has_global_middleware::<M>()` | `hasMiddleware` | Se o tipo `M` está registrado |
| `global_middleware_count()` | - | Número de globais atualmente registrados |
| `MiddlewareRegistry::from_global()` | - | Tira um snapshot do registro global para um registro por servidor |
| `MiddlewareRegistry::prepend(M)` | - | Prepend no estilo builder em uma instância de registro |
| `MiddlewareRegistry::append_boxed(M)` | - | Adiciona um middleware pré-boxado |
| `MiddlewareRegistry::prepend_boxed(M)` | - | Insere no início um middleware pré-boxado |
| `MiddlewareRegistry::len()` / `is_empty()` | - | Introspecção do builder |

`MiddlewareRegistry::from_global()` tira um snapshot do registro
global no momento da chamada. Registre todo middleware global ANTES de
construir o servidor - uma chamada a `global_middleware!` feita DEPOIS
que o servidor é construído não se aplica retroativamente, então a
pilha de middleware de um servidor em execução não pode mudar debaixo
dele.

## Layout de arquivos

Um layout típico assim que você tem alguns middlewares:

```
src/
├── middleware/
│   ├── mod.rs          # mod + pub use
│   ├── auth.rs         # AuthMiddleware
│   ├── logging.rs      # LoggingMiddleware
│   └── audit.rs        # AuditLogTerminator
├── bootstrap.rs        # global_middleware! + register_terminable
├── routes.rs           # .middleware(M) per-route
└── main.rs
```

`make:middleware` mantém `src/middleware/mod.rs` sincronizado - ele
anexa a nova declaração `mod foo;` e o `pub use foo::FooMiddleware;`
correspondente quando o arquivo é gerado.

## Por que Suprnova diverge

O Laravel registra classes de middleware em `app/Http/Kernel.php` e as
resolve através do contêiner, que faz reflection nos type-hints do
construtor para injetar dependências. O modelo request-per-process do
PHP significa que o kernel é reconstruído a cada solicitação, então o
custo da resolução reflexiva é pago uma vez por solicitação e
desaparece entre solicitações.

O modelo de processo do Suprnova é um binário servindo muitas
solicitações concorrentes através de muitas threads. Construir uma
chain nova por solicitação forçaria um ponto de sincronização na lista
de middleware global e realocaria `Arc<dyn Middleware>` para cada
camada em cada solicitação. Em vez disso:

- Middleware global é registrado em um `OnceLock<RwLock<Vec<...>>>` na
  inicialização, chaveado por `TypeId` para registro idempotente.
- `MiddlewareRegistry::from_global()` tira um snapshot da lista global
  uma vez na construção do servidor; a chain por solicitação reusa
  esse snapshot.
- A própria chain é composta aninhando closures `Arc<dyn Fn>`, então o
  trabalho por solicitação é um `Arc::clone` por camada em vez de uma
  alocação nova.

A superfície voltada ao usuário - `handle(request, next)`, a macro
`global_middleware!`, aliases nomeados, listas de prioridade, hooks
termináveis - é a mesma que um desenvolvedor Laravel busca. A maquinaria
por baixo troca a reconstrução por solicitação do PHP por um modelo
Rust de snapshot-na-inicialização para que o framework possa servir
solicitações concorrentes sem disputar o registro.

## Próximos passos

- [Ciclo de vida da solicitação](lifecycle.md) - onde a chain executa e como
  panics são capturados na fronteira do servidor
- [Modelo de erros](error-model.md) - o que `Result<HttpResponse, HttpResponse>`
  realmente significa e como short-circuits colapsam
- [Timeouts](timeout.md) - cancel-safety de `TimeoutMiddleware` em detalhe
- [CORS](cors.md) - tratamento de preflight, padrões de origem, escopo por caminho
- [Limitação de taxa](rate-limiting.md) - `RateLimitMiddleware` /
  `ThrottleRequestsMiddleware` e `BackendErrorPolicy`
- [Roteamento](routing.md) - no que `routes!`, `Router`, e `group(...)`
  se expandem
