# Observabilidade

O framework distribui três camadas de sinal visível para o operador:
logs estruturados (sempre ativos), correlação por id de solicitação
(sempre ativa, propaga para tasks spawnadas), e uma ponte opt-in com o
OpenTelemetry que transforma todo span do `tracing` em um span OTel
exportado. O mesmo `#[tracing::instrument]` que você escreveria para
logs locais se torna um span de trace distribuído quando a feature
OTel está ativa - sem uma segunda API de instrumentação.

```rust
use suprnova::telemetry::{init_telemetry, OtelConfig};
use suprnova::logging::LogConfig;

#[suprnova::main]
async fn main() {
    let guard = init_telemetry(LogConfig::from_env(), OtelConfig::from_env());

    // ... execute o app ...

    // Faça flush da telemetria em buffer antes de sair. Os batch
    // processors do OTel mantêm spans/métricas/logs em memória; dropar
    // a guarda sem `shutdown` perde tudo que ainda não foi exportado.
    guard.shutdown().await;
}
```

Um app com scaffold já tem seu `Server` chamando `init_telemetry` para
você e fazendo flush da guarda no sinal de shutdown - você só faz essa
ligação à mão quando embute o Suprnova no seu próprio runtime.

## As três camadas

| Camada | Sempre ativa | O que ela entrega |
|---|---|---|
| Log estruturado (`tracing`) | Sim | Logs em stdout no formato `pretty` (dev) ou `json` (produção), consciente do ambiente |
| Correlação por id de solicitação | Sim | Id por solicitação escopado por um `tokio::task_local!`, ecoado em `X-Request-Id`, propaga para tasks de `spawn_with_request_id` |
| Exportação OpenTelemetry | feature `otel` + endpoint de collector | Exportação OTLP HTTP/proto de traces, métricas e logs; propagação `traceparent` do W3C nos dois sentidos |

A camada OTel é **opt-in em tempo de compilação**, então builds padrão
não carregam dependências do OpenTelemetry e a facade
[`Metrics`](#métricas) compila para no-ops inertes. Com a feature
desligada, "trace" e "exportação de métrica" silenciosamente se tornam
no-ops - seus logs continuam funcionando.

### Por que Suprnova diverge

A história de observabilidade do Laravel se divide entre eventos
in-framework (`QueryExecuted`, `MessageSent`, `JobProcessed`) e
preocupações de runtime delegadas a extensões do PHP (OpenTelemetry,
Sentry, New Relic) plugadas na camada FPM. A superfície de eventos é
rica; a superfície de runtime é "instale a extensão que o seu
fornecedor de APM exige."

O Suprnova é um único processo assíncrono, então ele é dono das duas
metades. A superfície de eventos tem paridade (a mesma forma de
`QueryExecuted`/`NotificationSent`/`ErrorOccurred`), e a superfície de
runtime é uma ponte `tracing` → OpenTelemetry dentro do próprio
framework. Você não instala uma extensão; você ativa uma feature flag
e os mesmos spans que você já emite passam a ser exportados para o
OTel.

## Log estruturado

`LogConfig::from_env()` lê duas variáveis de ambiente:

| Var | Padrão | Notas |
|---|---|---|
| `LOG_LEVEL` | `"info"` | Sintaxe de env-filter do `tracing-subscriber` (ex.: `"debug,sqlx=warn,hyper=warn"`) |
| `LOG_FORMAT` | consciente do ambiente | `"json"` em produção, `"pretty"` em todo o resto; um valor explícito sempre vence |

O padrão de formato é detectado a partir de `APP_ENV` via
`Environment::detect()`: um deploy de produção recebe por padrão uma
saída de um-objeto-JSON-por-linha para agregadores de log, execuções
locais/dev recebem saída em várias linhas legível por humanos. Um
`LOG_FORMAT=pretty` explícito sobrescreve o padrão de produção se você
quiser stdout cru em produção.

```bash
# Dev local - sobrescritas explícitas vencem
LOG_LEVEL=debug,sqlx=warn,hyper=warn LOG_FORMAT=pretty cargo run

# Produção - APP_ENV=production muda o padrão de formato para json
APP_ENV=production LOG_LEVEL=info cargo run --release
```

Uma diretiva `LOG_LEVEL` malformada não derruba o boot - ela recai para
`"info"` e imprime um aviso de uma linha em stderr, para que a
configuração incorreta fique visível ao operador.

### Contexto de span em toda linha

Toda solicitação HTTP roteada executa dentro de um span `request`
criado pelo middleware mais externo do framework. O span carrega três
campos - `request_id`, `method`, `path` - e o formatador JSON os
aninha sob `span` em todo evento emitido dentro da solicitação. Seu
código de aplicação não precisa ler ou registrar o id em cada linha; o
span já o carrega implicitamente:

```rust
use tracing::info;

pub async fn show(req: suprnova::Request) -> suprnova::Response {
    info!(user_id = 42, "loaded dashboard");
    // A linha JSON carrega span.request_id / span.method / span.path
    // sem que o call site precise costurar nada.
    Ok(suprnova::json_response!({ "ok": true }))
}
```

## Correlação por id de solicitação

Toda solicitação recebe um id UUID v4 de 36 caracteres em minúsculas,
escopado por um `tokio::task_local!`. O middleware reaproveita um
`X-Request-Id` de entrada quando o valor do header passa por uma
verificação estrita de segurança (alfanuméricos ASCII mais `-_.:`,
máximo de 128 bytes); qualquer coisa fora desse conjunto de caracteres
é rejeitada e substituída por um UUID novo, para que um atacante não
consiga injetar caracteres de controle na saída de log nem inflar
pipelines downstream.

O mesmo id é ecoado em **toda** resposta - sucesso, erro e recuperação
de panic - como o header `X-Request-Id`, para que um frontend ou
serviço upstream possa incluí-lo em relatórios de bug e operadores
possam fazer grep dele no log estruturado.

### Lendo o id

```rust
use suprnova::{current_request_id, spawn_with_request_id};

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    // Dentro de uma solicitação, o id está sempre presente.
    let id = current_request_id().expect("inside a request");
    tracing::info!(request_id = %id, "checkout starting");

    // Trabalho em background spawnado a partir de um handler.
    // `tokio::spawn` inicia uma task com task-locals vazios - a future
    // spawnada perderia o request id sem ajuda. `spawn_with_request_id`
    // captura o id do chamador e o reescopa para a future spawnada, e
    // anexa o span atual do `tracing` para que os eventos da task
    // herdem `request_id` da mesma forma que os eventos in-request.
    spawn_with_request_id(async move {
        // Esta linha de log carrega o id da solicitação de origem.
        tracing::info!("post-checkout fanout running");
    });

    Ok(suprnova::ok!())
}
```

`current_request_id()` retorna `None` fora de uma solicitação - jobs em
background, tarefas agendadas e testes sem o middleware não veem
nenhum id, e o helper não inventa um. `spawn_with_request_id` fora de
um escopo de solicitação é exatamente `tokio::spawn`; nada de mágico
acontece.

### Onde o id também está disponível

| Superfície | Como |
|---|---|
| Eventos do `tracing` | `span.request_id` em toda linha dentro da solicitação |
| Header de resposta | `X-Request-Id` em respostas de sucesso, erro e recuperadas de panic |
| Conjunto `Context` | `Context::get("_request_id")` - legível a partir de observers, listeners, jobs que consultam o `Context` |
| Tasks spawnadas | `current_request_id()` depois de `spawn_with_request_id` |

## Eventos embutidos para observabilidade

O framework despacha eventos tipados nos pontos em que um operador
normalmente quer instrumentar. Cada um é um `suprnova::Event` que você
pode escutar (`listen`) via `EventFacade::listen::<E, _>(...)` e enviar
para o Sentry, Datadog, Slack, ou seu próprio pipeline de métricas.
Todos eles passam por `dispatch_best_effort`, então um listener que
falha não quebra a solicitação que o disparou.

| Evento | Quando dispara | Carrega |
|---|---|---|
| `ErrorOccurred` | Toda conversão de `FrameworkError` → 5xx (incluindo recuperação de panic) | contexto de erro + id de solicitação |
| `QueryExecuted` | Toda consulta roteada através dos helpers de executor instrumentados | sql, bindings, duração, conexão, classificação leitura/escrita, resultado |
| `ConnectionEstablished` | `DbConnection::connect` teve sucesso | nome da conexão |
| `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` | `DB::transaction` na forma de closure + handles manuais | nome da conexão |
| `NotificationSending` / `NotificationSent` / `NotificationFailed` | Antes/depois/erro por canal de `Notification::send` | notificação + canal + destinatário |

`ErrorOccurred` é o gancho para enviar exceções 5xx; `QueryExecuted` é
o gancho para alertas de consulta lenta; o trio de notificação é o
gancho para dashboards de entrega. Veja [Eventos](events.md) para a
API de listener e [Ciclo de vida da solicitação](lifecycle.md) para
onde cada evento dispara no caminho da solicitação.

### Observação direta de consultas no BD

`DB::listen` é um segundo gancho, síncrono, feito especificamente para
`QueryExecuted`. Ele dispara in-line dentro do executor, então um
listener lento deixa a consulta lenta - mantenha-o leve. O caminho do
dispatcher (`EventFacade::listen::<QueryExecuted, _>`) executa todos em
modo best-effort e tolera erros; prefira-o para qualquer coisa que
possa falhar.

```rust
use suprnova::DB;

// Em bootstrap.rs:
DB::listen(|q| {
    if q.time > std::time::Duration::from_millis(100) {
        tracing::warn!(
            sql = %q.sql,
            ms = q.time.as_millis(),
            "slow query"
        );
    }
})?;
```

Um listener que ele mesmo emite uma consulta ao banco de dados **não**
vai disparar `QueryExecuted` de novo para a chamada aninhada - uma
salvaguarda de reentrância thread-local impede o loop "listener de
log-para-BD → emite evento → log-para-BD → ...".

### Capturando um log de consultas para testes / debug

Para asserções de teste ou uma investigação pontual do tipo "o que
executou durante este bloco?":

```rust
use suprnova::DB;

DB::enable_query_log()?;
// ... execute o código que você quer inspecionar ...
let queries = DB::get_query_log()?;
for q in &queries {
    println!("{:>4}ms  {}", q.time.as_millis(), q.to_raw_sql());
}
DB::disable_query_log()?;
DB::flush_query_log()?;
```

O buffer é **ilimitado** - toda consulta capturada o faz crescer.
Use-o para testes e investigação de execução única, e faça flush
periodicamente se deixá-lo ativo em produção.

## Rastreamento distribuído (OTel)

Adicione a feature `otel` para ativar:

```toml
[dependencies]
suprnova = { git = "...", features = ["otel"] }
```

Configure através das variáveis de ambiente padrão do OTel:

```bash
# Mínimo: onde o collector vive.
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
OTEL_SERVICE_NAME=my-app          # padrão é "suprnova"
OTEL_SERVICE_VERSION=1.4.2        # padrão é a versão do seu crate
```

A telemetria é **habilitada** somente quando
`OTEL_EXPORTER_OTLP_ENDPOINT` está definido **e** o kill switch
`OTEL_SDK_DISABLED` não está ativo. Sem endpoint, a camada de log roda
sozinha, e a guarda retornada não possui nenhum provider, então
dropá-la sem `shutdown()` é silencioso (sem aviso espúrio de
"telemetria em buffer pode ter sido perdida" em todo processo de
teste).

### O contexto de trace se junta automaticamente

**Entrada.** Quando uma solicitação chega carregando um header
[`traceparent`](https://www.w3.org/TR/trace-context/) do W3C - ou
seja, foi feita por outro serviço rastreado - o middleware extrai esse
contexto e reparenta o span da solicitação sob o span do chamador. O
span do seu servidor aparece como filho no *mesmo* trace distribuído,
não como uma raiz nova. Uma solicitação sem `traceparent` (um acesso
direto do navegador) permanece um span raiz limpo.

**Saída.** O cliente HTTP do framework ([`Http`](http-client.md))
injeta o contexto de trace ativo como `traceparent` em toda chamada de
saída, para que o serviço downstream continue o mesmo trace.

Juntando os dois: `serviço upstream → seu handler → serviço downstream`
é um único trace conectado, sem nenhuma fiação manual de span nos seus
handlers.

**Status de erro.** Quando um handler retorna um 5xx, o span da
solicitação é marcado como com erro, para que o backend OTel mostre
`Status::Error`. (Um *panic* de handler é capturado e transformado em
um 500 com um log em nível de erro e um evento `ErrorOccurred`, mas o
status do span OTel não é definido nesse caminho - o panic desenrola a
future do span antes de o marcador executar.)

### Adicionando seus próprios spans

Como a ponte transforma todo span do `tracing` em um span OTel, você
instrumenta com o `tracing` puro - nenhuma API específica do OTel no
seu código:

```rust
use suprnova::DatabaseConnection;

#[tracing::instrument(skip(db))]
async fn load_dashboard(db: &DatabaseConnection, user_id: i64) -> anyhow::Result<()> {
    // Este span se aninha sob o span da solicitação automaticamente,
    // e é exportado para o seu collector quando a feature `otel`
    // está ativa.
    Ok(())
}
```

### Variáveis de ambiente que o Suprnova lê

| Var | Efeito |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | URL base do collector. Não definida → telemetria desabilitada. |
| `OTEL_SERVICE_NAME` | Atributo de recurso `service.name` (padrão `"suprnova"`). |
| `OTEL_SERVICE_VERSION` | Atributo de recurso `service.version` (padrão: versão do crate). |
| `OTEL_SDK_DISABLED` | Kill switch. `true` ou `1`, insensível a maiúsculas/minúsculas, desabilita a exportação mesmo com um endpoint definido. |

O restante dos controles OTLP padrão é lido pelo próprio SDK, então
configure-os da forma normal:

| Var | Lida por |
|---|---|
| `OTEL_EXPORTER_OTLP_HEADERS` | exportador (autenticação do collector, ex.: `Authorization=Bearer ...`) |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | exportador (`http/protobuf`, etc.) |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | exportador |
| `OTEL_EXPORTER_OTLP_COMPRESSION` | exportador |

Sobrescritas de endpoint por sinal (`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`,
`_METRICS_ENDPOINT`, `_LOGS_ENDPOINT`) atualmente sofrem shadowing do
endpoint base - os três sinais vão para `OTEL_EXPORTER_OTLP_ENDPOINT`.
Se você precisa direcionar sinais para collectors diferentes, rode um
collector local que os roteie.

## Métricas

`Metrics` é a facade para contadores, histogramas e gauges. Handles são
baratos de clonar e resolvem o meter global a cada construção:

```rust
use suprnova::telemetry::Metrics;

// Contador - monotônico.
let signups = Metrics::counter("user.signups");
signups.inc();                                  // +1
signups.inc_by(3);                              // +3
signups.inc_with(&[("plan", "pro")]);           // +1 com um label

// Histograma - distribuições (latência, tamanhos).
let latency = Metrics::histogram("request.latency_ms");
latency.record(42.0);
latency.record_with(42.0, &[("route", "/checkout")]);

// Gauge - valor no instante atual.
let queue_depth = Metrics::gauge("jobs.pending");
queue_depth.set(17.0);
queue_depth.set_with(17.0, &[("queue", "emails")]);
```

Sem a feature `otel`, toda chamada acima é um no-op sem nenhuma
alocação - deixe a instrumentação em hot paths e não pague nada em
builds padrão.

Handles de métrica se vinculam a qualquer meter provider que esteja
ativo quando o instrument subjacente é resolvido pela primeira vez.
Crie handles **depois** de `init_telemetry` já ter executado (ou de
forma lazy no primeiro uso) - um handle construído antes da
inicialização resolve contra o provider no-op e fica inerte. O padrão
idiomático é um handle `once_cell` / `LazyLock` resolvido na primeira
emissão, bem depois do boot.

Valores de atributo são tipados como string
(`&[(&'static str, &str)]`). Atributos numéricos e booleanos são uma
melhoria planejada; por ora, formate-os como strings no call site.

Nomenclatura: estável, ASCII, delimitada por ponto (ex.:
`"http.requests.total"`, `"http.request.duration"`). As convenções
semânticas padrão do OTel vivem em
`opentelemetry-semantic-conventions::metric::*`.

## O contrato de shutdown

`init_telemetry` retorna um `TelemetryGuard` que é dono dos handles de
provider do SDK. Os batch processors do OTel bufferizam spans /
métricas / logs em memória e fazem flush de forma assíncrona, então
você precisa fazer `guard.shutdown().await` antes de o processo sair,
ou perde o que ainda estiver bufferizado.

- Chamar `shutdown()` faz flush e é seguro chamar uma única vez (ele
  consome `self`).
- Dropar a guarda **sem** `shutdown()` registra um aviso - mas somente
  quando a guarda de fato contém providers. Uma execução com
  telemetria desabilitada (sem endpoint, ou `OTEL_SDK_DISABLED`, ou um
  build sem a feature `otel`) devolve uma guarda sem providers cujo
  drop é silencioso, então execuções de dev e teste sem collector não
  são inundadas de avisos.

## Resumo

| Tarefa | API |
|---|---|
| Habilitar o OTel | `features = ["otel"]` + `OTEL_EXPORTER_OTLP_ENDPOINT` |
| Inicializar | `init_telemetry(LogConfig::from_env(), OtelConfig::from_env())` |
| Fazer flush ao sair | `guard.shutdown().await` |
| Desabilitar em runtime | `OTEL_SDK_DISABLED=true` |
| Span customizado | `#[tracing::instrument]` (com ponte automática para o OTel) |
| Contador / histograma / gauge | `Metrics::counter/histogram/gauge(name)` |
| Junção de trace distribuído | Automática - `traceparent` de entrada extraído, de saída injetado |
| Ler o id de solicitação atual | `current_request_id()` |
| Propagar o id para um spawn | `spawn_with_request_id(future)` |
| Observador síncrono de consulta | `DB::listen(\|q\| { ... })` |
| Observador best-effort de consulta | `EventFacade::listen::<QueryExecuted, _>(...)` |
| Capturar consultas para testes | `DB::enable_query_log()` → `DB::get_query_log()` |

## Próximos passos

- [Eventos](events.md) - API de listener, modos de dispatch,
  `EventFacade::fake()` para testes
- [Ciclo de vida da solicitação](lifecycle.md) - onde no caminho da
  solicitação cada evento dispara e onde o span da solicitação é
  construído
- [Tratamento de erros](errors.md) - `ErrorOccurred`, `HttpError`,
  corpos 5xx sanitizados
- [Banco de dados](database.md) - `QueryExecuted`, `DB::transaction`,
  os helpers de executor que disparam os eventos
- [Cliente HTTP](http-client.md) - injeção de `traceparent` de saída
  que fecha o loop do trace distribuído
