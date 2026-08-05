# Eventos

Los eventos son el pub/sub tipado y en proceso de Suprnova. Un
controlador dispara `UserRegistered { user_id }`; un oyente le envía
un correo al usuario, otro escribe una fila de auditoría, un tercero
publica una difusión. Los tres ven el mismo payload, se ejecutan en
el orden de registro, y no tienen conocimiento en tiempo de
compilación unos de otros.

La superficie de cara al usuario es el struct `EventFacade`
(re-exportado como `suprnova::EventFacade`). El crate también
re-exporta el *trait* `Event` como `suprnova::Event` - el mismo
nombre que la fachada de Laravel, pero en Rust el trait es el
contrato tipado que implementa cada payload. Detrás de la fachada hay
un único `EventDispatcher` global de proceso (guardado en un
`OnceLock`): los oyentes registrados sobreviven a la solicitud que
los registró, y los despachos o bien se ejecutan en línea o generan
una tarea dentro de un conjunto acotado con reintento.

## Lo esencial

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
        // enviar el correo…
        let _ = e.user_id;
        Ok(())
    }
}

// En bootstrap.rs:
EventFacade::listen::<UserRegistered, SendWelcomeEmail>(Arc::new(SendWelcomeEmail)).await;

// En un controlador:
EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
```

`Event` requiere `Send + Sync + Clone + 'static + Debug` para que un
payload pueda cruzar límites de tarea (oyentes encolados) y el
despachador pueda registrarlo en el log. `Listener<E>` es
`Send + Sync + 'static` para que pueda sobrevivir a la llamada de
registro. No existe un `#[derive(Event)]` - el trait tiene dos
métodos (`event_name` y el `queued` con valor por defecto), así que
un impl escrito a mano son dos líneas.

## Modos de despacho

| Método | Semántica |
|---|---|
| `EventFacade::dispatch(event)` | Síncrono, fail-fast - el primer `Err` de un oyente aborta la cadena |
| `EventFacade::dispatch_best_effort(event)` | Síncrono, ejecuta-todos - devuelve el primer `Err` después de que cada oyente se ha ejecutado |
| `EventFacade::dispatch(event)` cuando `Event::queued() = true` | Cada oyente se genera como una tarea acotada con reintento; la llamada retorna después de generarla |

Usa `dispatch` (fail-fast) cuando un efecto secundario downstream
DEBE observar un éxito upstream - la mayoría de los hooks de ciclo de
vida de modelos caen aquí, así que un observador que veta un guardado
puede hacer cortocircuito. Usa `dispatch_best_effort` para la
dispersión donde un oyente que falla no debería silenciar al resto -
la mayoría de los eventos de observabilidad caen aquí.

Anula el método del trait para optar por la entrega encolada:

```rust
impl Event for ExpensiveAuditTrail {
    fn event_name() -> &'static str { "ExpensiveAuditTrail" }
    fn queued() -> bool { true }
}
```

Los oyentes encolados están acotados por un semáforo de todo el
proceso. El techo por defecto es de 256 tareas concurrentes; anúlalo
por despachador con `EventDispatcher::with_concurrency(n)` o
globalmente mediante la variable de entorno `EVENT_MAX_CONCURRENCY`.
Cada tarea reintenta hasta 3 veces con un backoff con jitter de
100ms→2s antes de rendirse - estos son reintentos de fallo transitorio
dentro del proceso, no el programa de minutos de la cola durable.

## Subscribers - agrupa los registros relacionados

Cuando varios oyentes pertenecen a una sola feature, un `Subscriber`
los registra como una unidad. Refleja el patrón subscriber del
`EventServiceProvider` de Laravel.

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

// En bootstrap.rs - una línea por subscriber en lugar de tres por oyente:
EventFacade::subscribe(Arc::new(UserEventSubscriber { db: db.clone() })).await;
```

`subscribe` toma `Arc<S>` para que los oyentes que necesiten
compartir estado con el subscriber puedan clonar el `Arc` y
capturarlo.

## Inspeccionar y eliminar oyentes

```rust
if EventFacade::has_listeners::<UserRegistered>() {
    EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
}

let removed: usize = EventFacade::forget::<UserRegistered>();
```

`has_listeners::<E>()` refleja el `Event::hasListeners($eventName)`
de Laravel. `forget::<E>()` descarta cada oyente registrado para ese
tipo de evento y devuelve la cantidad eliminada. El código de
producción rara vez necesita `forget` - el registro de oyentes
normalmente ocurre una sola vez en el bootstrap - pero el código de
hot-swap y de tests recurre a él.

Ambos métodos devuelven valores por defecto seguros cuando el
bloqueo del registro de oyentes está envenenado (`false` y `0`
respectivamente), con un `tracing::error!` registrado para que el
fallo sea observable.

## Push y flush

`push` captura un evento en un bucket por nombre de evento sin
dispararlo. `flush::<E>()` drena el bucket y despacha todo en el
orden de captura. Refleja el par `Event::push` / `Event::flush` de
Laravel.

```rust
// Dentro de un handler que hace su trabajo en dos fases:
EventFacade::push(UserRegistered { user_id: 42 }).await;
// … renderizado, validación, más trabajo …
EventFacade::flush::<UserRegistered>().await?;
```

Los eventos con push ignoran el alcance de `defer` - ya están
diferidos explícitamente. `forget_pushed()` descarta cada evento con
push sin despacharlo, devolviendo la cantidad descartada. Refleja
`Event::forgetPushed()`.

## defer - almacena en búfer cada despacho dentro de un callback

`defer(only, async { … })` ejecuta el callback con un búfer
task-local en alcance. Cada llamada a `dispatch` /
`dispatch_best_effort` hecha dentro del callback se captura y se
reproduce después de que el callback retorna. Refleja el
`Event::defer($callback, ?$events)` de Laravel.

```rust
let ((), flush_err) = EventFacade::defer::<_, ()>(None, async {
    do_work_part_one().await?;
    EventFacade::dispatch(WorkStarted).await?; // en búfer
    do_work_part_two().await?;
    EventFacade::dispatch(WorkFinished).await?; // en búfer
    Ok(())
})
.await?;
// En este punto, tanto WorkStarted como WorkFinished ya se dispararon en orden.
// `flush_err` lleva el primer error de despacho de la reproducción (si hay alguno).
```

Pasa `Some(&["EventOne", "EventTwo"])` para diferir SOLO esos nombres
de evento; todo lo demás se despacha en línea como de costumbre. Un
error del callback hace cortocircuito - los eventos en búfer se
descartan, el error se propaga.

El búfer de `defer` es por tarea de Tokio, así que dos llamadas
concurrentes a `defer` no se pisan el estado una a la otra.

## Oyentes encolados - en proceso frente a durables

Dos niveles distintos de "encolado", y el nombrado importa:

| Necesidad | Recurre a |
|---|---|
| El oyente debe ejecutarse fuera de la tarea; está bien perderlo ante una caída | `Event::queued() = true` en el trait del evento |
| El trabajo del oyente DEBE sobrevivir a una caída + reinicio | `QueuedListener<E, J>` (conecta el evento con un job durable) |

`Event::queued() = true` hace que el despachador genere cada oyente
como su propia tarea de Tokio, acotada por un semáforo de proceso,
con reintento acotado (3 intentos, backoff con jitter). El trabajo se
ejecuta en este proceso; una caída descarta los oyentes en vuelo. El
[drenaje del apagado ordenado](#drenaje-al-apagar) espera a las
tareas en vuelo hasta un plazo.

`QueuedListener<E, J>` es un oyente de fábrica que construye un
[`Job`](queues.md) a partir de cada evento y lo empuja a la cola
durable. El evento de todos modos se dispara de forma síncrona; el
oyente solo encola - lo cual es rápido - así que la latencia de la
solicitud se mantiene baja. El job en sí sobrevive a la caída porque
la cola es durable.

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

El `QueuedListener` solo necesita que el evento sea un evento
síncrono normal - la durabilidad vive en la cola, no en el
despachador.

## Drenaje al apagar

Los oyentes encolados en proceso se generan dentro de un `JoinSet`
rastreado por el despachador. La secuencia de apagado ordenado del
servidor llama a `EventFacade::drain_queued(timeout)` para
esperarlos:

```rust
let still_running = EventFacade::drain_queued(Duration::from_secs(30)).await;
if still_running > 0 {
    tracing::warn!(still_running, "queued listeners abandoned at shutdown");
}
```

El drenaje devuelve la cantidad que todavía se está ejecutando
cuando el plazo expiró (`0` = completamente drenado). Los rezagados
que superan el plazo se abortan para que el apagado no pueda
colgarse.

## Conectar los eventos con la difusión

`EventFacade::broadcast::<E>(hub)` conecta, en una línea, un puente
entre un evento despachado y un `BroadcastHub`. Cualquier tipo que
implemente `Broadcastable` y `Event` puede difundirse de esta forma;
los oyentes reciben el payload tipado, y los suscriptores en los
canales nombrados reciben el sobre de la difusión.

```rust
use suprnova::EventFacade;
use std::sync::Arc;

let hub: Arc<dyn suprnova::BroadcastHub> = Arc::new(broadcast_hub);
EventFacade::broadcast::<OrderShipped>(hub).await;

// Cualquier despacho posterior también se publica en los canales
// declarados por OrderShipped::broadcast_on():
EventFacade::dispatch(OrderShipped { order_id: 42, user_id: 99 }).await?;
```

Ver [Difusión](broadcasting.md) para el modelo de canales (público /
privado / presencia) y el trait `Broadcastable`.

## Eventos incorporados

El framework despacha un conjunto fijo de eventos desde sus propios
subsistemas. Participas registrando oyentes; si no se registra
ningún oyente los eventos son no-ops.

| Subsistema | Eventos | Despachado por |
|---|---|---|
| Manejo de errores | `ErrorOccurred` | Cada respuesta 5xx (un `FrameworkError` devuelto o un pánico recuperado) |
| Auth (guards) | `Auth\\Attempting`, `Auth\\Authenticated`, `Auth\\Login`, `Auth\\Logout`, `Auth\\Failed` | `StatefulGuard::attempt` / `login` / `logout` / `once` |
| Flujos de auth | `EmailVerified`, `PasswordResetLinkSent`, `PasswordResetCompleted`, `AccountLocked`, `AccountUnlocked`, `TwoFactorEnrolled`, `TwoFactorChallenged`, `TwoFactorChallengeFailed`, `TwoFactorDisabled` | `auth_flows::{EmailVerification, PasswordReset, BruteForce, TwoFactor}` |
| Base de datos | `Database\\ConnectionEstablished`, `Database\\QueryExecuted`, `Database\\TransactionBeginning`, `Database\\TransactionCommitted`, `Database\\TransactionRolledBack`, `Database\\DatabaseBusy` | `DbConnection::connect`, helpers de `ExecutorChoice`, `DB::transaction` |
| Correo | `Suprnova\\Mail\\MessageSending`, `Suprnova\\Mail\\MessageSent` | `MailBuilder::send` antes/después del transporte |
| Notificaciones | `Suprnova::Notifications::Sending`, `Suprnova::Notifications::Sent`, `Suprnova::Notifications::Failed` | Cada entrega de canal |
| Cola (worker) | `queue::JobQueueing`, `JobQueued`, `JobProcessing`, `JobProcessed`, `JobAttempted`, `JobExceptionOccurred`, `JobFailed`, `JobReleased`, `JobReleasedAfterException`, `JobTimedOut`, `Looping`, `WorkerStarting`, `WorkerStopping`, `WorkerInterrupted` | `Queue::push` / `run_worker` |
| Características | `FeatureUpdated`, `FeatureDeleted` | CRUD de `features::admin` |
| Eloquent (por modelo) | 16 eventos de ciclo de vida - `Retrieved`, `Saving`, `Saved`, `Creating`, `Created`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Restoring`, `Restored`, `ForceDeleting`, `ForceDeleted`, `Replicating`, `Pruning`, `Pruned` - emitidos bajo el submódulo `events::` de cada modelo | La macro `#[suprnova::model]` conecta estos con save/update/delete |

`ErrorOccurred` es el hook dedicado para enviar excepciones 5xx a
Sentry, Datadog, Slack, etc. El despacho es best-effort y se genera
como tarea aparte, así que un oyente roto de Sentry no puede
silenciar al resto, y la conversión de la respuesta nunca bloquea a
la espera de él. Ver [Modelo de errores](error-model.md) para el
contrato completo de recuperación de pánicos y conversión.

Los eventos de ciclo de vida de los modelos se disparan en modo
fail-fast: un oyente de `Saving` que devuelve `EventResult::Cancel`
(mediante el trait `CancellableListener`) aborta el guardado. Ver
[Observadores y eventos de ciclo de vida de Eloquent](eloquent.md).

## DB::listen - observar consultas

Para observabilidad por consulta puedes registrar ya sea un
`Listener<QueryExecuted>` tipado a través del despachador o, más
habitualmente, un callback de `DB::listen` que refleja la firma
`DB::listen(function ($q) { ... })` de Laravel:

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

El callback recibe un `QueryExecuted` que lleva el SQL, los
bindings, la duración en tiempo real, el nombre de la conexión, la
clasificación de lectura/escritura, y el `Result` final (así que las
consultas fallidas también son observables). `QueryExecuted::to_raw_sql()`
incrusta los bindings por comodidad de log - formato debug, NO seguro
para SQL.

Dos garantías, de reentrancia y de costo:

- **Guarda de reentrancia.** Un oyente que él mismo emite una
  consulta no volverá a disparar `QueryExecuted` desde esa consulta
  anidada - el despachador establece un flag task-local mientras un
  oyente se ejecuta, y el executor omite la emisión dentro de ese
  alcance. Un oyente que registra en la base de datos no entrará en
  bucle.
- **Overhead cero cuando nadie está escuchando.** El executor
  comprueba un `query_observation_active()` combinado (cualquier
  oyente directo, cualquier `Listener<QueryExecuted>` registrado, O
  el log de consultas activado) antes de construir el payload del
  evento. Cuando los tres están desactivados, toda la ruta de
  emisión hace cortocircuito.

## Pruebas - `EventFacade::fake()`

`EventFacade::fake()` sustituye el despachador global por un
recorder. Los eventos despachados van a la grabación en lugar de
ejecutar oyentes. El fake mantiene un serializador de todo el
proceso durante la vida de la guarda, así que los `#[tokio::test]`
paralelos que lo usan se ejecutan de uno en uno - los tests ya no
necesitan su propio mutex de `serial_test`.

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
| `assert_dispatched::<E>(pred)` | que se despachó al menos un `E` que coincide |
| `assert_dispatched_once::<E>()` | que se despachó exactamente un `E` |
| `assert_dispatched_times::<E>(n)` | que se despacharon exactamente `n` de `E` |
| `assert_not_dispatched::<E>(pred)` | que no se despachó ningún `E` que coincida |
| `assert_nothing_dispatched()` | que NO se despachó ningún evento de ningún tipo |
| `assert_listening::<E, L>()` | que se registró un oyente `L` para `E` |
| `has_dispatched::<E>()` | bool: si se registró algún `E` |
| `dispatched::<E>(pred)` | clones `Vec<E>` de los eventos que coinciden |
| `dispatched_count::<E>(pred)` | la cantidad de eventos que coinciden |
| `dispatched_events()` | `HashMap<&'static str, usize>` de todos los despachos |

### Fake selectivo

```rust
// Solo falsea estos eventos; todo lo demás se despacha normalmente.
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// Falsea todos los eventos EXCEPTO estos.
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

Refleja el `Event::fake([…])` y el `EventFake::except($events)` de
Laravel.

### Mute - descarta eventos sin registrarlos

`EventFacade::muted(async { … })` ejecuta el callback con un flag
task-local de "despachador silencioso" activado; cada evento
despachado dentro se descarta sin registrarse ni invocar oyentes. Es
el análogo en Suprnova del `NullDispatcher` de Laravel, acotado a un
callback.

```rust
EventFacade::muted(async {
    // No se dispara ningún oyente, no se registra ningún evento.
    run_bulk_import().await;
})
.await;
```

A diferencia de `fake()`, `muted` NO adquiere el serializador de
proceso - dos alcances muted pueden ejecutarse en paralelo.

### `assert_listening` - verifica que un oyente esté conectado

Úsalo para testear el cableado del bootstrap sin disparar un evento:

```rust
#[tokio::test]
async fn bootstrap_wires_welcome_listener() {
    let _guard = EventFacade::fake();
    bootstrap::register_listeners().await;
    suprnova::events::assert_listening::<UserRegistered, SendWelcomeEmail>();
}
```

El fake observa los registros mediante el método `listen` del
despachador, así que el registro debe ocurrir DENTRO del alcance del
fake - los oyentes registrados antes de `EventFacade::fake()` NO son
vistos por `assert_listening`.

## Matriz de paridad con Laravel

Cada método de la fachada `Event` y de `EventFake` de Laravel 13 que
tiene un equivalente tipado en Rust se entrega bajo el nombre más
parecido. Los métodos que Laravel expone y que no encajan en Rust
tipado se omiten con una nota breve.

| Laravel | Suprnova |
|---|---|
| `Event::dispatch($event)` | `EventFacade::dispatch(event).await` |
| `Event::dispatch($event)` (con el argumento halt) | usa `dispatch` (fail-fast ante `Err`) |
| `Event::until($event)` | `dispatch` (tipado: el primer `Err` detiene) |
| `Event::listen($event, $listener)` | `EventFacade::listen::<E, L>(Arc::new(L))` |
| `Event::hasListeners($name)` | `EventFacade::has_listeners::<E>()` |
| `Event::forget($event)` | `EventFacade::forget::<E>()` |
| `Event::push($event)` | `EventFacade::push(event).await` |
| `Event::flush($event)` | `EventFacade::flush::<E>().await` |
| `Event::forgetPushed()` | `EventFacade::forget_pushed().await` |
| `Event::defer($callback, ?$events)` | `EventFacade::defer(only, async {…}).await` |
| `Event::subscribe($subscriber)` | `EventFacade::subscribe(Arc::new(S)).await` |
| `Event::fake()` | `EventFacade::fake()` (guarda) |
| `Event::fake([$names])` | `EventFacade::fake_only(&["…"])` |
| `EventFake::except($names)` | `EventFacade::fake_except(&["…"])` |
| `EventFake::assertDispatched` | `assert_dispatched` |
| `EventFake::assertDispatchedOnce` | `assert_dispatched_once` |
| `EventFake::assertDispatchedTimes` | `assert_dispatched_times` |
| `EventFake::assertNotDispatched` | `assert_not_dispatched` |
| `EventFake::assertNothingDispatched` | `assert_nothing_dispatched` |
| `EventFake::assertListening` | `assert_listening` |
| `EventFake::hasDispatched` | `has_dispatched` |
| `EventFake::dispatched` | `dispatched` (devuelve `Vec<E>`) |
| `EventFake::dispatchedEvents` | `dispatched_events` (mapa nombre → cantidad) |
| `NullDispatcher` | `EventFacade::muted(async {…}).await` |
| `Event::wildcards` (patrones `User.*`) | no se incluye - usa oyentes tipados, o el trait `Observer<M>` para hooks de ciclo de vida por modelo |
| `Event::subscribe` (subscriber por string) | usa el trait tipado `Subscriber` |
| `DB::listen(function ($q) {…})` | `DB::listen(Arc::new(|q| {…}))` - misma forma, toma `&QueryExecuted` |

### Por qué Suprnova diverge

El despachador de Laravel se apoya en el runtime de tipado mediante
strings de PHP: los eventos son nombres de clase pasados como
strings, los oyentes son nombres de clase resueltos a través del
contenedor, y `Event::listen('User.*', ...)` funciona porque los
wildcards sobre strings de nombre de clase tienen sentido en PHP. En
Rust, el equivalente de "este oyente maneja `User.*`" es "este oyente
es genérico sobre `E: UserEvent`" - un trait, no una coincidencia de
strings. Así que Suprnova abandona los wildcards en favor del sistema
de tipos, y el resultado es que los refactors rotos se convierten en
errores de compilación en lugar de enrutamientos incorrectos en
tiempo de ejecución.

La otra divergencia es `defer`: el defer de Laravel se apoya en el
modelo de una solicitud por proceso para acotar el alcance del
diferimiento. Suprnova sirve muchas solicitudes concurrentes en un
solo proceso, así que el búfer de diferimiento es task-local. Dos
llamadas concurrentes a `defer` obtienen cada una su propio búfer;
las llamadas no pueden pisarse entre sí, y no hay ningún estado
global oculto que pueda filtrarse.

## Dónde vive cada pieza

| Pieza | Archivo |
|---|---|
| `Event` trait, `Listener<E>`, `Subscriber` | `framework/src/events/mod.rs` |
| `EventDispatcher`, `EventFacade` (facade struct) | `framework/src/events/dispatcher.rs` |
| `ErrorOccurred` | `framework/src/events/builtins.rs` |
| `QueuedListener<E, J>` | `framework/src/events/queued_listener.rs` |
| `assert_dispatched*`, `EventFakeGuard`, `muted` | `framework/src/events/testing.rs` |
| Payloads de eventos incorporados | `framework/src/{database,auth,auth_flows,mail,notifications,queue,features}/events.rs` |
| Eventos de ciclo de vida por modelo | generados por macro dentro del submódulo `events::` de cada modelo |

## Siguiente

- [Modelo de errores](error-model.md) - `ErrorOccurred` y la ruta de
  conversión de 5xx
- [Cola](queues.md) - jobs durables, el nivel tolerante a caídas;
  `QueuedListener` se conecta con esto
- [Difusión](broadcasting.md) - conecta eventos despachados con
  canales de WebSocket mediante `EventFacade::broadcast::<E>(hub)`
- [Eloquent](eloquent.md) - eventos de ciclo de vida de modelos y el
  trait `Observer<M>`
- [Base de datos](database.md) - `DB::listen` y el evento
  `Database\\QueryExecuted`
