# Eventos

Eventos são o pub/sub tipado em processo do Suprnova. Um controller
dispara `UserRegistered { user_id }`; um listener envia email ao
usuário, outro escreve uma linha de auditoria, um terceiro publica uma
transmissão. Todos os três veem o mesmo payload, executam na ordem de
registro, e não têm conhecimento em tempo de compilação um do outro.

A superfície voltada para o usuário é a struct `EventFacade`
(reexportada como `suprnova::EventFacade`). A crate também reexporta a
*trait* `Event` como `suprnova::Event` - mesmo nome da facade do
Laravel, mas em Rust a trait é o contrato tipado que todo payload
implementa. Por trás da facade há um único `EventDispatcher`
process-global (mantido em um `OnceLock`): listeners registrados
sobrevivem à solicitação que os registrou, e dispatches ou executam
inline ou spawnam em um conjunto de tasks limitado com retry.

## O básico

```rust
use suprnova::{EventFacade, Event, Listener, FrameworkError, async_trait};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct UserRegistered {
    pub user_id: i64,
}

impl Event for UserRegistered {
    fn event_name() -> &'static str {
        "UserRegistered"
    }
}

pub struct SendWelcomeEmail;

#[async_trait]
impl Listener<UserRegistered> for SendWelcomeEmail {
    async fn handle(&self, e: &UserRegistered) -> Result<(), FrameworkError> {
        // envia o email…
        let _ = e.user_id;
        Ok(())
    }
}

// In bootstrap.rs:
EventFacade::listen::<UserRegistered, SendWelcomeEmail>(Arc::new(SendWelcomeEmail)).await;

// In a controller:
EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
```

`Event` exige `Send + Sync + Clone + 'static + Debug` para que um
payload possa cruzar fronteiras de task (listeners em fila) e o
dispatcher possa logá-lo. `Listener<E>` é `Send + Sync + 'static` para
que possa sobreviver à chamada de registro. Não há `#[derive(Event)]` -
a trait tem dois métodos (`event_name` e o `queued` com valor padrão)
então um impl escrito à mão tem duas linhas.

## Modos de dispatch

| Método | Semântica |
|---|---|
| `EventFacade::dispatch(event)` | Síncrono, fail-fast - o primeiro `Err` de um listener aborta a chain |
| `EventFacade::dispatch_best_effort(event)` | Síncrono, executa todos - retorna o primeiro `Err` depois que todo listener já executou |
| `EventFacade::dispatch(event)` quando `Event::queued() = true` | Cada listener spawna como uma task limitada com retry; a chamada retorna depois de spawnar |

Use `dispatch` (fail-fast) quando um efeito colateral downstream
PRECISA observar um upstream bem-sucedido - a maioria dos hooks de
ciclo de vida de model se encaixa aqui, então um observer que veta um
save pode fazer short-circuit. Use `dispatch_best_effort` para fan-out
em que um listener falhando não deveria silenciar o resto - a maioria
dos eventos de observabilidade se encaixa aqui.

Sobrescreva o método da trait para optar por entrega em fila:

```rust
impl Event for ExpensiveAuditTrail {
    fn event_name() -> &'static str { "ExpensiveAuditTrail" }
    fn queued() -> bool { true }
}
```

Listeners em fila são limitados por um semáforo de todo o processo. O
teto padrão é 256 tasks concorrentes; sobrescreva por dispatcher com
`EventDispatcher::with_concurrency(n)` ou globalmente via a variável
de ambiente `EVENT_MAX_CONCURRENCY`. Cada task tenta de novo até 3
vezes com backoff com jitter de 100ms a 2s antes de desistir - esses
são retries em processo para falhas transitórias, não a agenda de
minutos da fila durável.

## Subscribers - agrupe registros relacionados

Quando vários listeners pertencem a uma feature, um `Subscriber` os
registra como uma unidade. Espelha o padrão de subscriber do
`EventServiceProvider` do Laravel.

```rust
use suprnova::{EventFacade, EventDispatcher, Subscriber, async_trait};
use std::sync::Arc;

pub struct UserEventSubscriber {
    db: Arc<crate::Db>,
}

#[async_trait]
impl Subscriber for UserEventSubscriber {
    async fn subscribe(self: Arc<Self>, d: &EventDispatcher) {
        let db = self.db.clone();
        d.listen::<UserRegistered, _>(Arc::new(SendWelcomeEmail::new(db.clone()))).await;
        d.listen::<UserDeleted, _>(Arc::new(CleanupUserData::new(db.clone()))).await;
        d.listen::<UserPromoted, _>(Arc::new(NotifyAdmins::new(db))).await;
    }
}

// Em bootstrap.rs - uma linha por subscriber em vez de três por listener:
EventFacade::subscribe(Arc::new(UserEventSubscriber { db: db.clone() })).await;
```

`subscribe` recebe `Arc<S>` para que listeners que precisem
compartilhar estado com o subscriber possam clonar o `Arc` e
capturá-lo.

## Inspecionando e removendo listeners

```rust
if EventFacade::has_listeners::<UserRegistered>() {
    EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
}

let removed: usize = EventFacade::forget::<UserRegistered>();
```

`has_listeners::<E>()` espelha o `Event::hasListeners($eventName)` do
Laravel. `forget::<E>()` descarta todo listener registrado para
aquele tipo de evento e retorna a contagem removida. Código de
produção raramente precisa de `forget` - o registro de listener
normalmente é feito uma vez no bootstrap - mas hot-swap e código de
teste recorrem a ele.

Os dois métodos retornam padrões seguros quando o lock do registry de
listener está envenenado (`false` e `0` respectivamente), com um
`tracing::error!` logado para que a falha seja observável.

## Push e flush

`push` captura um evento em um bucket por nome-de-evento sem
dispará-lo. `flush::<E>()` drena o bucket e despacha tudo na ordem de
captura. Espelha o par `Event::push` / `Event::flush` do Laravel.

```rust
// Dentro de um handler que faz trabalho em duas fases:
EventFacade::push(UserRegistered { user_id: 42 }).await;
// … renderização, validação, mais trabalho …
EventFacade::flush::<UserRegistered>().await?;
```

Eventos com push ignoram o scope `defer` - eles já estão explicitamente
deferidos. `forget_pushed()` descarta todo evento com push sem
despachar, retornando a contagem descartada. Espelha
`Event::forgetPushed()`.

## defer - armazene em buffer todo dispatch dentro de um callback

`defer(only, async { … })` executa o callback com um buffer
task-local em scope. Toda chamada `dispatch` / `dispatch_best_effort`
feita dentro do callback é capturada e reproduzida depois que o
callback retorna. Espelha o `Event::defer($callback, ?$events)` do
Laravel.

```rust
let ((), flush_err) = EventFacade::defer::<_, ()>(None, async {
    do_work_part_one().await?;
    EventFacade::dispatch(WorkStarted).await?; // em buffer
    do_work_part_two().await?;
    EventFacade::dispatch(WorkFinished).await?; // em buffer
    Ok(())
})
.await?;
// Neste ponto, tanto WorkStarted quanto WorkFinished já dispararam em ordem.
// `flush_err` carrega o primeiro erro de dispatch do replay (se houver).
```

Passe `Some(&["EventOne", "EventTwo"])` para deferir SOMENTE aqueles
nomes de evento; todo o resto despacha inline normalmente. Um erro no
callback faz short-circuit - eventos em buffer são descartados, o erro
se propaga.

O buffer de defer é por task do Tokio, então duas chamadas `defer`
concorrentes não pisam no estado uma da outra.

## Listeners em fila - em processo vs durável

Dois níveis distintos de "em fila", e a nomenclatura importa:

| Necessidade | Recorra a |
|---|---|
| O listener deveria executar fora da task; tudo bem perder em um crash | `Event::queued() = true` na trait do evento |
| O trabalho do listener PRECISA sobreviver a um crash + restart | `QueuedListener<E, J>` (liga evento → job durável) |

`Event::queued() = true` faz o dispatcher spawnar cada listener como
sua própria task Tokio, limitada por um semáforo de processo, com
retry limitado (3 tentativas, backoff com jitter). O trabalho executa
neste processo; um crash descarta listeners em voo. A
[drenagem no shutdown gracioso](#drenagem-no-shutdown) espera pelas
tasks em voo até um prazo.

`QueuedListener<E, J>` é um listener de fábrica que constrói um
[`Job`](queues.md) a partir de cada evento e faz push dele na fila
durável. O evento ainda dispara sincronamente; o listener só
enfileira - o que é rápido - então a latência da solicitação
permanece baixa. O job em si sobrevive ao crash porque a fila é
durável.

```rust
use suprnova::{EventFacade, QueuedListener};
use std::sync::Arc;

EventFacade::listen::<UserRegistered, _>(Arc::new(
    QueuedListener::<UserRegistered, SendWelcomeEmailJob>::new(|e| SendWelcomeEmailJob {
        user_id: e.user_id,
    }),
))
.await;
```

O `QueuedListener` só precisa que o evento seja um evento síncrono
comum - a durabilidade mora na fila, não no dispatcher.

## Drenagem no shutdown

Listeners em processo em fila spawnam em um `JoinSet` rastreado pelo
dispatcher. A sequência de shutdown gracioso do servidor chama
`EventFacade::drain_queued(timeout)` para esperar por eles:

```rust
let still_running = EventFacade::drain_queued(Duration::from_secs(30)).await;
if still_running > 0 {
    tracing::warn!(still_running, "queued listeners abandoned at shutdown");
}
```

O drain retorna a contagem ainda executando quando o prazo se esgotou
(`0` = totalmente drenado). Os que restarem depois do prazo são
abortados para que o shutdown não possa travar.

## Conectando eventos à transmissão

`EventFacade::broadcast::<E>(hub)` conecta em uma linha uma ponte de
um evento despachado para um `BroadcastHub`. Qualquer tipo que
implementa `Broadcastable` e `Event` pode ser transmitido dessa forma;
listeners recebem o payload tipado, e assinantes nos canais nomeados
recebem o envelope de transmissão.

```rust
use suprnova::EventFacade;
use std::sync::Arc;

let hub: Arc<dyn suprnova::BroadcastHub> = Arc::new(broadcast_hub);
EventFacade::broadcast::<OrderShipped>(hub).await;

// Todo dispatch posterior também é publicado nos canais declarados
// por OrderShipped::broadcast_on():
EventFacade::dispatch(OrderShipped { order_id: 42, user_id: 99 }).await?;
```

Veja [Transmissão](broadcasting.md) para o modelo de canal (público /
privado / presença) e a trait `Broadcastable`.

## Eventos embutidos

O framework despacha um conjunto fixo de eventos de seus próprios
subsistemas. Você opta por participar registrando listeners; se
nenhum listener está registrado os eventos são no-ops.

| Subsistema | Eventos | Despachado por |
|---|---|---|
| Tratamento de erro | `ErrorOccurred` | Toda resposta 5xx (`FrameworkError` retornado ou panic recuperado) |
| Auth (guards) | `Auth\\Attempting`, `Auth\\Authenticated`, `Auth\\Login`, `Auth\\Logout`, `Auth\\Failed` | `StatefulGuard::attempt` / `login` / `logout` / `once` |
| Fluxos de auth | `EmailVerified`, `PasswordResetLinkSent`, `PasswordResetCompleted`, `AccountLocked`, `AccountUnlocked`, `TwoFactorEnrolled`, `TwoFactorChallenged`, `TwoFactorChallengeFailed`, `TwoFactorDisabled` | `auth_flows::{EmailVerification, PasswordReset, BruteForce, TwoFactor}` |
| Database | `Database\\ConnectionEstablished`, `Database\\QueryExecuted`, `Database\\TransactionBeginning`, `Database\\TransactionCommitted`, `Database\\TransactionRolledBack`, `Database\\DatabaseBusy` | `DbConnection::connect`, helpers de `ExecutorChoice`, `DB::transaction` |
| Mail | `Suprnova\\Mail\\MessageSending`, `Suprnova\\Mail\\MessageSent` | `MailBuilder::send` antes/depois do transporte |
| Notificações | `Suprnova::Notifications::Sending`, `Suprnova::Notifications::Sent`, `Suprnova::Notifications::Failed` | Cada entrega de canal |
| Fila (worker) | `queue::JobQueueing`, `JobQueued`, `JobProcessing`, `JobProcessed`, `JobAttempted`, `JobExceptionOccurred`, `JobFailed`, `JobReleased`, `JobReleasedAfterException`, `JobTimedOut`, `Looping`, `WorkerStarting`, `WorkerStopping`, `WorkerInterrupted` | `Queue::push` / `run_worker` |
| Features | `FeatureUpdated`, `FeatureDeleted` | CRUD de `features::admin` |
| Eloquent (por model) | 16 eventos de ciclo de vida - `Retrieved`, `Saving`, `Saved`, `Creating`, `Created`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Restoring`, `Restored`, `ForceDeleting`, `ForceDeleted`, `Replicating`, `Pruning`, `Pruned` - emitidos sob o submódulo `events::` de cada model | A macro `#[suprnova::model]` conecta esses a save/update/delete |

`ErrorOccurred` é o hook dedicado para enviar exceções 5xx para
Sentry, Datadog, Slack, etc. O dispatch é best-effort e spawnado, então
um listener de Sentry quebrado não pode silenciar o resto, e a
conversão de resposta nunca bloqueia por causa dele. Veja
[Modelo de Erro](error-model.md) para o contrato completo de
recuperação de panic e conversão.

Eventos de ciclo de vida de model disparam fail-fast: um listener de
`Saving` que retorna `EventResult::Cancel` (via a trait
`CancellableListener`) aborta o save. Veja
[Observers e eventos de ciclo de vida do Eloquent](eloquent.md).

## DB::listen - observando queries

Para observabilidade por query você pode registrar tanto um
`Listener<QueryExecuted>` tipado através do dispatcher quanto, mais
comumente, um callback `DB::listen` que espelha a assinatura do
`DB::listen(function ($q) { ... })` do Laravel:

```rust
use suprnova::DB;
use std::sync::Arc;

DB::listen(Arc::new(|q| {
    tracing::debug!(
        sql = %q.sql,
        time_ms = q.time.as_millis(),
        connection = %q.connection_name,
        "query"
    );
}));
```

O callback recebe um `QueryExecuted` carregando o SQL, os bindings, a
duração de wall-clock, o nome da conexão, a classificação
leitura/escrita, e o `Result` final (então queries que falharam também
são observáveis). `QueryExecuted::to_raw_sql()` embute os bindings
inline para conveniência de log - formato debug, NÃO seguro para SQL.

Duas garantias de reentrância e custo:

- **Guarda de reentrância.** Um listener que ele mesmo emite uma query
  não vai fazer `QueryExecuted` disparar de novo a partir dessa query
  aninhada - o dispatcher define uma flag task-local enquanto um
  listener executa, e o executor pula a emissão dentro daquele scope.
  Um listener que loga para o DB não vai entrar em loop.
- **Overhead zero quando ninguém está escutando.** O executor verifica
  um `query_observation_active()` combinado (qualquer listener
  direto, qualquer `Listener<QueryExecuted>` registrado, OU log de
  query habilitado) antes de construir o payload do evento. Quando os
  três estão desligados, todo o caminho de emissão faz short-circuit.

## Testes - `EventFacade::fake()`

`EventFacade::fake()` troca o dispatcher global por um gravador.
Eventos despachados vão para o registro em vez de rodar listeners. O
fake mantém uma serialização de todo o processo durante a vida da
guard, então `#[tokio::test]`s paralelos que a usam executam um de
cada vez - testes não precisam mais do próprio mutex `serial_test`.

```rust
use suprnova::events::{
    EventFacade, assert_dispatched, assert_dispatched_once, assert_dispatched_times,
    assert_nothing_dispatched, has_dispatched, dispatched, dispatched_events,
};

#[tokio::test]
async fn registration_dispatches_welcome_event() {
    let _guard = EventFacade::fake();

    register_user("ada@example.com").await.unwrap();

    assert_dispatched_once::<UserRegistered>();
    assert_dispatched::<UserRegistered>(|e| e.email == "ada@example.com");
}
```

| Helper | Verifica |
|---|---|
| `assert_dispatched::<E>(pred)` | pelo menos um `E` correspondente foi despachado |
| `assert_dispatched_once::<E>()` | exatamente um `E` foi despachado |
| `assert_dispatched_times::<E>(n)` | exatamente `n` de `E` foram despachados |
| `assert_not_dispatched::<E>(pred)` | nenhum `E` correspondente foi despachado |
| `assert_nothing_dispatched()` | NENHUM evento de qualquer tipo foi despachado |
| `assert_listening::<E, L>()` | um listener `L` foi registrado para `E` |
| `has_dispatched::<E>()` | bool: algum `E` registrado |
| `dispatched::<E>(pred)` | clones `Vec<E>` dos eventos correspondentes |
| `dispatched_count::<E>(pred)` | contagem de eventos correspondentes |
| `dispatched_events()` | `HashMap<&'static str, usize>` de todos os dispatches |

### Faking seletivo

```rust
// Faz fake somente destes eventos; todo o resto despacha normalmente.
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// Faz fake de todo evento EXCETO estes.
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

Espelha o `Event::fake([…])` e o `EventFake::except($events)` do
Laravel.

### Mute - descarte eventos sem gravação

`EventFacade::muted(async { … })` executa o callback com uma flag
task-local de "dispatcher silencioso" definida; todo evento despachado
dentro é descartado sem gravação nem invocação de listeners. O
análogo do Suprnova ao `NullDispatcher` do Laravel, restrito a um
callback.

```rust
EventFacade::muted(async {
    // Nenhum listener dispara, nenhum evento é registrado.
    run_bulk_import().await;
})
.await;
```

Diferente de `fake()`, `muted` NÃO adquire o serializador de processo -
dois scopes mutados podem executar em paralelo.

### `assert_listening` - verifique que um listener está conectado

Use para testar a conexão do bootstrap sem disparar um evento:

```rust
#[tokio::test]
async fn bootstrap_wires_welcome_listener() {
    let _guard = EventFacade::fake();
    bootstrap::register_listeners().await;
    suprnova::events::assert_listening::<UserRegistered, SendWelcomeEmail>();
}
```

O fake observa registros via o método `listen` do dispatcher, então o
registro precisa acontecer DENTRO do scope do fake - listeners
registrados antes de `EventFacade::fake()` NÃO são vistos por
`assert_listening`.

## Matriz de paridade do Laravel

Todo método `Event` facade e `EventFake` do Laravel 13 que tem um
equivalente tipado em Rust é entregue sob o nome mais parecido.
Métodos que o Laravel expõe e que não se encaixam no Rust tipado são
omitidos com uma nota curta.

| Laravel | Suprnova |
|---|---|
| `Event::dispatch($event)` | `EventFacade::dispatch(event).await` |
| `Event::dispatch($event)` (halt arg) | use `dispatch` (fail-fast em `Err`) |
| `Event::until($event)` | `dispatch` (tipado: primeiro `Err` interrompe) |
| `Event::listen($event, $listener)` | `EventFacade::listen::<E, L>(Arc::new(L))` |
| `Event::hasListeners($name)` | `EventFacade::has_listeners::<E>()` |
| `Event::forget($event)` | `EventFacade::forget::<E>()` |
| `Event::push($event)` | `EventFacade::push(event).await` |
| `Event::flush($event)` | `EventFacade::flush::<E>().await` |
| `Event::forgetPushed()` | `EventFacade::forget_pushed().await` |
| `Event::defer($callback, ?$events)` | `EventFacade::defer(only, async {…}).await` |
| `Event::subscribe($subscriber)` | `EventFacade::subscribe(Arc::new(S)).await` |
| `Event::fake()` | `EventFacade::fake()` (guard) |
| `Event::fake([$names])` | `EventFacade::fake_only(&["…"])` |
| `EventFake::except($names)` | `EventFacade::fake_except(&["…"])` |
| `EventFake::assertDispatched` | `assert_dispatched` |
| `EventFake::assertDispatchedOnce` | `assert_dispatched_once` |
| `EventFake::assertDispatchedTimes` | `assert_dispatched_times` |
| `EventFake::assertNotDispatched` | `assert_not_dispatched` |
| `EventFake::assertNothingDispatched` | `assert_nothing_dispatched` |
| `EventFake::assertListening` | `assert_listening` |
| `EventFake::hasDispatched` | `has_dispatched` |
| `EventFake::dispatched` | `dispatched` (retorna `Vec<E>`) |
| `EventFake::dispatchedEvents` | `dispatched_events` (mapa nome → contagem) |
| `NullDispatcher` | `EventFacade::muted(async {…}).await` |
| `Event::wildcards` (padrões `User.*`) | não incluído - use listeners tipados, ou a trait `Observer<M>` para hooks de ciclo de vida por model |
| `Event::subscribe` (subscriber via string) | use a trait `Subscriber` tipada |
| `DB::listen(function ($q) {…})` | `DB::listen(Arc::new(|q| {…}))` - mesma forma, recebe `&QueryExecuted` |

### Por que Suprnova diverge

O dispatcher do Laravel se apoia no runtime tipado-por-string do PHP:
eventos são nomes de classe passados como strings, listeners são
nomes de classe resolvidos via o container, e
`Event::listen('User.*', ...)` funciona porque wildcards sobre strings
de nome de classe fazem sentido em PHP. Em Rust, o equivalente de
"este listener trata `User.*`" é "este listener é genérico sobre
`E: UserEvent`" - uma trait, não um match de string. Então o Suprnova
abandona wildcards em favor do sistema de tipos, e o resultado é que
refactors quebrados se tornam erros de compilação em vez de
mis-routes em runtime.

A outra divergência é `defer`: o defer do Laravel se apoia no modelo
de um-processo-por-solicitação para limitar o scope da deferral. O
Suprnova serve muitas solicitações concorrentes em um processo, então
o buffer de deferral é task-local. Duas chamadas `defer` concorrentes
recebem cada uma seu próprio buffer; as chamadas não podem pisar uma
na outra, e não há estado global escondido para vazar.

## Onde cada peça vive

| Peça | Arquivo |
|---|---|
| trait `Event`, `Listener<E>`, `Subscriber` | `framework/src/events/mod.rs` |
| `EventDispatcher`, `EventFacade` (struct facade) | `framework/src/events/dispatcher.rs` |
| `ErrorOccurred` | `framework/src/events/builtins.rs` |
| `QueuedListener<E, J>` | `framework/src/events/queued_listener.rs` |
| `assert_dispatched*`, `EventFakeGuard`, `muted` | `framework/src/events/testing.rs` |
| Payloads de evento embutidos | `framework/src/{database,auth,auth_flows,mail,notifications,queue,features}/events.rs` |
| Eventos de ciclo de vida por model | gerado por macro dentro do submódulo `events::` de cada model |

## Próximos passos

- [Modelo de Erro](error-model.md) - `ErrorOccurred` e o caminho de
  conversão 5xx
- [Filas](queues.md) - jobs duráveis, o nível tolerante a crash;
  `QueuedListener` liga a isso
- [Transmissão](broadcasting.md) - conecte eventos despachados a
  canais WebSocket via `EventFacade::broadcast::<E>(hub)`
- [Eloquent](eloquent.md) - eventos de ciclo de vida de model e a
  trait `Observer<M>`
- [Banco de dados](database.md) - `DB::listen` e o evento
  `Database\\QueryExecuted`
