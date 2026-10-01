# Tempos limite de solicitação

`TimeoutMiddleware` impõe um prazo rígido em toda solicitação HTTP. Um
handler lento - uma consulta de banco de dados travada, uma API
upstream que não responde, um loop infinito acidental em algum hot
path - de outra forma manteria uma conexão hyper aberta até o cliente
desistir ou o OS matar o processo. O middleware de timeout limita essa
espera, descarta o handler em voo, e retorna `503 Service Unavailable`
para que o operador veja a falha em vez de a aplicação vazar conexões
silenciosamente.

Recorra a ele quando você estiver construindo qualquer coisa que fale
com a internet pública, qualquer coisa que faça fan-out para APIs de
terceiros, ou qualquer coisa em que "o banco de dados pode estar lento
hoje" seja uma terça-feira realista.

```rust
use suprnova::{global_middleware, TimeoutMiddleware};

pub async fn register() {
    // Toda rota HTTP recebe um teto de 30 segundos.
    global_middleware!(TimeoutMiddleware::default());
}
```

Essa única linha dá para a aplicação inteira o mesmo teto padrão que o
Suprnova usa para seu timeout de conexão de banco de dados - escolha
uma vez, aplique em todo lugar. Overrides por rota são uma linha cada.
O resto deste capítulo explica exatamente o que o prazo delimita, o
que ele intencionalmente não delimita, e como ele interage com o
limite de panic, respostas de streaming, e WebSockets.

## O middleware

`TimeoutMiddleware` vive em `suprnova::TimeoutMiddleware`. Ele expõe
três construtores e um acessor:

```rust
use std::time::Duration;
use suprnova::TimeoutMiddleware;

let default_30s = TimeoutMiddleware::default();
let custom      = TimeoutMiddleware::new(Duration::from_millis(2_500));
let whole_secs  = TimeoutMiddleware::seconds(5);

assert_eq!(default_30s.duration(), Duration::from_secs(30));
assert_eq!(custom.duration(),      Duration::from_millis(2_500));
assert_eq!(whole_secs.duration(),  Duration::from_secs(5));
```

`TimeoutMiddleware::default()` usa um prazo de 30 segundos. Esse
número não é arbitrário - ele corresponde ao `DB_CONNECT_TIMEOUT`
(também 30s) para que uma solicitação bloqueada esperando por uma
conexão de banco de dados nova em folha e uma solicitação bloqueada
dentro do handler compartilhem um único teto. Se você aumentar um,
aumente o outro.

`TimeoutMiddleware::seconds(n)` é um atalho para o caso comum de
segundos inteiros. `TimeoutMiddleware::new(Duration::…)` é a válvula
de escape quando você precisa de precisão de milissegundos (um health
check interno que nunca deveria levar mais que 200ms; uma sonda
sintética com um orçamento de 50ms).

## Instalando globalmente

Um timeout global é o ponto de partida certo: ele dá a toda rota um
teto sem que ninguém precise lembrar de adicioná-lo. Instale-o em
`bootstrap.rs` junto com seu outro middleware global:

```rust
// src/bootstrap.rs
use suprnova::{
    global_middleware, CorsConfig, CorsMiddleware, DB, RequestIdMiddleware, TimeoutMiddleware,
};
use crate::middleware::LoggingMiddleware;

pub async fn register() {
    DB::init().await.expect("database connect");

    // A ordem de execução importa: request-id primeiro (para que os logs
    // de timeout o carreguem), depois logging (para que solicitações lentas
    // ainda sejam observadas), depois o timeout em si.
    global_middleware!(RequestIdMiddleware);
    global_middleware!(LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());

    global_middleware!(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"]),
    ));
}
```

A ordem importa porque middleware global envolve o resto da chain na
ordem de registro: `RequestIdMiddleware` executa primeiro na entrada e
último na saída, então o request id está em scope enquanto o timeout
dispara seu `503`. Colocar o timeout antes do logging esconderia do
access log solicitações lentas que eventualmente completaram.

## Restringindo por rota

Um teto global de 30 segundos é generoso de propósito - ele está lá
para pegar handlers descontrolados, não para impor SLAs. Quando um
endpoint específico deveria falhar mais rápido, anexe um timeout por
rota:

```rust
use suprnova::{Router, TimeoutMiddleware};

Router::new()
    // Endpoint público de relatório: precisa responder em 5s ou preferimos
    // retornar 503 e deixar o cliente tentar de novo do que bloquear.
    .get("/report", controllers::report::show)
    .middleware(TimeoutMiddleware::seconds(5));
```

Você também pode anexar um timeout mais restrito a um grupo de rotas.
Esse é o formato típico para uma API pública em que cada solicitação
deveria ser rápida, enquanto o resto do app mantém o padrão de 30
segundos:

```rust
use suprnova::Router;
use suprnova::TimeoutMiddleware;

Router::new()
    .group("/api", |r| {
        r.get("/users",       controllers::api::users::index)
         .post("/users",      controllers::api::users::create)
         .get("/users/{id}",  controllers::api::users::show)
    })
    .middleware(TimeoutMiddleware::seconds(3));
```

### O global é um teto; por rota só pode restringir

Middleware global executa **fora** do middleware de rota. A chain
envolve de dentro para fora:

```
Timeout global (30s) → Timeout de rota (3s) → handler
```

As duas futures de `tokio::time::timeout` estão armadas; a interna
dispara primeiro porque tem o prazo mais curto. Então um timeout por
rota só pode tornar uma rota *mais restrita* que o global, nunca mais
permissiva.

Se um único endpoint legitimamente precisa executar por *mais tempo*
que o padrão global - um relatório lento, um upload grande, um
fallback de long-poll - você tem duas opções:

1. Aumente o valor global. Mais simples, mas relaxa o teto para toda outra rota também.
2. Restrinja o escopo do middleware global a um grupo de rotas que *exclua* o endpoint longo, e anexe um timeout separado (ou nenhum) à rota lenta. Isso mantém o padrão restrito em todo o resto.

A segunda opção é o formato certo para um outlier isolado; a primeira
é certa quando toda uma classe de trabalho precisa de mais espaço.

## O que o prazo realmente delimita

O prazo corre uma corrida contra a future retornada por
`next(request)`. Essa future resolve no momento em que seu handler
retorna sua `HttpResponse` - não quando o corpo termina de fazer
streaming. Essa distinção é estrutural:

- **Handlers normais** constroem o corpo completo antes de retornar, então o prazo efetivamente delimita o tempo total do handler. Um handler que serializa uma lista JSON, renderiza uma página Inertia, ou monta uma resposta HTML mantém a future até o trabalho terminar.
- **Respostas de streaming** (`HttpResponse::sse(...)`, `HttpResponse::stream_bytes(...)`) retornam *imediatamente* com um corpo lazy. A chain de middleware já terminou no momento em que o hyper começa a puxar bytes do stream, então o prazo nunca observa o tempo de vida do corpo. Um stream de eventos SSE pode ficar aberto por horas sob um timeout de 30 segundos, por design - veja [Eventos enviados pelo servidor](sse.md) para o modelo de streaming.
- **Upgrades de WebSocket** são pulados explicitamente. Veja a próxima seção.

Esse é o comportamento que você quase certamente quer. Se você
envolvesse um stream SSE de longa duração em um timeout de 30
segundos, o framework derrubaria a conexão no meio do stream a cada 30
segundos e a feature ficaria inutilizável.

## Exceção de WebSocket

O middleware inspeciona a solicitação antes de armar o prazo:

```rust
if is_websocket_upgrade(request.headers()) {
    return next(request).await;
}
```

Qualquer solicitação carregando `Upgrade: websocket` pula o timeout
completamente. A verificação não diferencia maiúsculas/minúsculas no
valor do token (`WebSocket`, `websocket`, `WEBSOCKET` todos
correspondem), e um `Connection: upgrade` isolado sem
`Upgrade: websocket` *não* é tratado como um upgrade de WS - isso
passa pelo timeout normalmente.

Hoje, upgrades de WebSocket seguem um caminho de servidor separado que
não executa middleware global de forma alguma, então essa salvaguarda é
defesa em profundidade - ela impede que o timeout algum dia delimite
um canal bidirecional de longa duração no dia em que isso mudar. Veja
[WebSockets](websockets.md) para como upgrades são despachados e o
tempo de vida de um socket conectado.

## O que acontece no prazo

Quando `tokio::time::timeout` decorre antes do handler completar, o
middleware faz três coisas, em ordem:

1. **Descarta a future do handler em voo.** Fazia-se poll da future dentro do combinator `timeout`; o combinator retorna `Err(Elapsed)` e a future é dropada onde estava suspensa por último.
2. **Loga um warning** com o caminho da rota e a duração do timeout em milissegundos:

   ```
   WARN suprnova::timeout request exceeded its timeout; returning 503 Service Unavailable
       route=/report timeout_ms=5000
   ```

   O log está em `WARN` para que apareça em dashboards de operador por
   padrão, separado dos access logs `INFO` de solicitações normais.
3. **Retorna `503 Service Unavailable`** com um corpo em texto puro:

   ```
   HTTP/1.1 503 Service Unavailable
   Content-Type: text/plain
   Content-Length: 42

   Service Unavailable: request timed out
   ```

O 503 é envolvido em `Err(HttpResponse::…)` então ele faz
short-circuit no resto da chain exatamente como qualquer outra
solicitação rejeitada por middleware. Middleware externo (logging,
request-id, CORS) ainda executa seu lado pós-handler, então a resposta
sai com os headers corretos.

### Por que 503 e não 504

`504 Gateway Timeout` é o código certo quando *você* é o gateway e um
*upstream* deu timeout. `503 Service Unavailable` é o código certo
quando *este* serviço não conseguiu produzir a resposta a tempo. O
middleware de timeout está delimitando *nosso próprio* handler, então
ele retorna 503. Se você quer um formato diferente - um corpo JSON, um
status diferente, um código legível por máquina - envolva seu próprio
middleware externo em torno do timeout e traduza sua resposta 503.

## Segurança de cancelamento

Quando o prazo decorre, a future do handler é **dropada** no seu ponto
atual de `.await`. Isso é cancelamento normal do Tokio; a mesma coisa
acontece quando um cliente fecha a conexão no meio da solicitação.
Qualquer coisa mantida através da fronteira do await é liberada pela
sua impl de `Drop`:

- **Transações de banco de dados** fazem rollback. Uma `DatabaseTransaction` do SeaORM tem uma impl de `Drop` que emite `ROLLBACK` na conexão subjacente.
- **Guardas de Mutex e RwLock** são liberadas. Uma guarda da standard library ou do `parking_lot` libera no drop; outro esperando pode tomá-la imediatamente.
- **Descritores de arquivo** fecham. O descritor no nível do OS é liberado quando o `tokio::fs::File` é dropado.
- **Conexões de rede** voltam para o pool ou fecham, dependendo do comportamento de drop do pool.

O resultado é que um handler que deu timeout não deixa nada pendurado -
o operador vê o 503, o banco de dados vê o rollback, a próxima
solicitação vê um pool limpo.

### O que *não* é cancelado

Qualquer coisa que você moveu para fora da solicitação com
`tokio::spawn` fica **desacoplada**. Tasks spawnadas vivem no runtime,
não na future da solicitação, então dropar a solicitação não as
interrompe. Isso importa quando você escreveu algo assim:

```rust
pub async fn webhook(req: Request) -> Response {
    let payload: WebhookPayload = req.json().await?;

    // Trabalho de background fire-and-forget. Sobrevive ao timeout da solicitação.
    tokio::spawn(async move {
        if let Err(e) = process_webhook(payload).await {
            tracing::error!("webhook processing failed: {e}");
        }
    });

    Ok(HttpResponse::new().status(204))
}
```

Se a solicitação der timeout *antes* da linha de `spawn` executar, o
spawn nunca acontece. Se a solicitação der timeout *depois* do spawn,
a task de background continua executando - ela não é cancelada junto
com a solicitação. Isso é quase sempre o que você quer para trabalho
no estilo webhook, mas significa que a limpeza depois de um `.await`
longo dentro do handler **não** tem garantia de executar:

```rust
pub async fn upload(req: Request) -> Response {
    let temp_path = save_to_temp(&req).await?;

    // Se isso é o que dá timeout, a limpeza abaixo NÃO EXECUTA.
    let processed = long_running_processing(&temp_path).await?;

    // Não garantido sob um timeout.
    tokio::fs::remove_file(&temp_path).await?;

    Ok(HttpResponse::json(serde_json::to_value(&processed)?))
}
```

A correção é usar RAII. Envolva o arquivo temporário em uma struct
cuja impl de `Drop` o remove; então a limpeza executa se o handler
retorna, retorna um erro, ou é dropado no meio de um `.await` pelo
timeout. Essa é a mesma disciplina que você aplicaria para qualquer
fonte de cancelamento - desconexão de cliente, shutdown do runtime,
recuperação de panic.

## Interação com o limite de panic

O servidor Suprnova envolve a chain de middleware inteira em
[`execute_chain_safely`](lifecycle.md), que usa
`AssertUnwindSafe(...).catch_unwind()` para traduzir panics em um
`500 Internal Server Error` sanitizado. Uma solicitação que deu
timeout **não** é um panic - a future é dropada de forma limpa - então
o `503` do timeout sai sem envolver o limite de panic de forma alguma.

Os dois limites tratam modos de falha diferentes:

| Falha | Limite | Status | Corpo |
|---|---|---|---|
| `.await` do handler excede o prazo | `TimeoutMiddleware` | `503` | `Service Unavailable: request timed out` |
| Handler sofre panic (`.unwrap()` em `None`, etc.) | `execute_chain_safely` | `500` | `{"message": "Internal Server Error"}` |
| Handler retorna `Err(HttpResponse)` | fluxo normal de `Response` | o que o handler definir | o que o handler definir |

Você não precisa escolher - os dois limites estão sempre instalados.
Um handler que sofre panic *depois* de exceder seu timeout ainda
produz um 503 (a future foi dropada antes que o panic pudesse
acontecer). Um handler que sofre panic *antes* de exceder seu timeout
produz um 500.

## Ajuste operacional

Três considerações ao escolher valores de timeout:

1. **Corresponda ao seu timeout de conexão de banco de dados.** Se `DB_CONNECT_TIMEOUT=30` (o padrão), um timeout de solicitação menor que 30s vai disparar antes que uma conexão lenta chegue a completar - o usuário vê `503` em vez da chance de se recuperar. Ou aumente o timeout de conexão ou aceite que "30s" é o piso.
2. **Leve em conta o handler legítimo mais lento.** Olhe um histograma das durações de solicitação no nível `INFO`. O p99 da cauda lenta deveria ficar confortavelmente abaixo do timeout, com margem para clock skew e jitter do event loop. Um timeout que dispara rotineiramente em tráfego saudável é uma má configuração, não uma feature.
3. **Timeouts por rota são observabilidade.** Restringir `TimeoutMiddleware::seconds(3)` em `/api/*` transforma uma API degradada em um alerta visível (logs cheios de WARN, 503s no load balancer) em vez de um problema de latência que vai se arrastando. Use-os onde você tem um SLA e quer uma falha dura quando não o cumprir.

Os próprios testes de integração do framework usam durações na faixa
de milissegundos (`TimeoutMiddleware::new(Duration::from_millis(50))`)
para exercitar o prazo de forma determinística. Prazos de produção são
quase sempre em segundos inteiros.

### Por que Suprnova diverge

Em um deployment Laravel + PHP-FPM, timeouts de solicitação vivem fora
da aplicação: o `proxy_read_timeout` do nginx, o
`request_terminate_timeout` do PHP-FPM, o timeout de idle do load
balancer. O processo PHP é matado quando o orçamento se esgota, e
qualquer estado aberto - conexões de banco de dados, descritores de arquivo -
vaza até a próxima solicitação reutilizar o worker.

O Suprnova delimita a solicitação dentro da aplicação porque pode. O
handler é uma future Tokio, não um processo PHP, então dropá-lo
executa impls de `Drop` de forma limpa: transações fazem rollback,
locks são liberados, descritores fecham, o connection pool permanece
saudável. O 503 também sai *como uma resposta HTTP real* - clientes
veem um status code apropriado em vez de um reset do upstream.

É por isso também que o middleware não tenta ser uma layer `Timeout`
do Tower. A layer do Tower é genérica sobre qualquer serviço Tokio e
retorna `tower::timeout::error::Elapsed`, que os chamadores então
precisam mapear para um status HTTP. O middleware do Suprnova sabe que
está envolvendo um pipeline de solicitação HTTP; ele retorna `503`
diretamente, loga a rota culpada, e respeita as exceções de WebSocket
e streaming do framework sem que o chamador precise raciocinar sobre
elas. A layer do Tower é a primitiva certa para um serviço Tokio
genérico; para uma solicitação HTTP, esse é o formato certo.

## Próximos passos

- [Middleware](middleware.md) - o trait, a chain, registro global vs por rota, hooks termináveis
- [Ciclo de vida da solicitação](lifecycle.md) - onde o timeout se encaixa na chain, e como `execute_chain_safely` trata panics
- [Eventos enviados pelo servidor](sse.md) - o modelo de resposta de streaming que o timeout intencionalmente não delimita
- [WebSockets](websockets.md) - o caminho de upgrade que contorna o timeout completamente
- [Erros](errors.md) - como respostas 5xx são despachadas como eventos `ErrorOccurred` para observabilidade
