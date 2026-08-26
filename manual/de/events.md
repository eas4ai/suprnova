# Ereignisse

Events sind Suprnovas typisiertes In-Process-Pub/Sub. Ein Controller
feuert `UserRegistered { user_id }`; ein Listener verschickt eine
E-Mail an den Nutzer, ein weiterer schreibt eine Audit-Zeile, ein
dritter broadcastet. Alle drei sehen denselben Payload, laufen in
Registrierungsreihenfolge und haben keine Compile-Zeit-Kenntnis
voneinander.

Die nutzerseitige Oberfläche ist die `EventFacade`-Struktur
(re-exportiert als `suprnova::EventFacade`). Die Crate re-exportiert
außerdem den *Trait* `Event` als `suprnova::Event` - derselbe Name
wie Laravels Facade, aber in Rust ist der Trait der typisierte
Vertrag, den jeder Payload implementiert. Hinter der Facade steht ein
einziger, prozessglobaler `EventDispatcher` (gehalten in einem
`OnceLock`): Registrierte Listener überleben die Anfrage, die sie
registriert hat, und Dispatches laufen entweder inline oder spawnen
in ein begrenztes, wiederholendes Task-Set.

## Die Grundlagen

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
        // die E-Mail senden…
        let _ = e.user_id;
        Ok(())
    }
}

// In bootstrap.rs:
EventFacade::listen::<UserRegistered, SendWelcomeEmail>(Arc::new(SendWelcomeEmail)).await;

// In einem Controller:
EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
```

`Event` verlangt `Send + Sync + Clone + 'static + Debug`, damit ein
Payload Task-Grenzen überschreiten kann (Queued Listener) und der
Dispatcher ihn protokollieren kann. `Listener<E>` ist
`Send + Sync + 'static`, damit er den Registrierungsaufruf überleben
kann. Es gibt kein `#[derive(Event)]` - der Trait hat zwei Methoden
(`event_name` und das mit Default versehene `queued`), sodass eine
handgerollte Implementierung zwei Zeilen umfasst.

## Dispatch-Modi

| Methode | Semantik |
|---|---|
| `EventFacade::dispatch(event)` | Synchron, fail-fast - der erste `Err` eines Listeners bricht die Chain ab |
| `EventFacade::dispatch_best_effort(event)` | Synchron, alle-ausführen - gibt den ersten `Err` zurück, nachdem jeder Listener gelaufen ist |
| `EventFacade::dispatch(event)` bei `Event::queued() = true` | Jeder Listener spawnt als begrenzte, wiederholende Task; der Aufruf kehrt nach dem Spawnen zurück |

Verwenden Sie `dispatch` (fail-fast), wenn ein nachgelagerter
Seiteneffekt einen erfolgreichen vorgelagerten Schritt beobachten
MUSS - die meisten Model-Lifecycle-Hooks fallen hierunter, sodass ein
Observer, der einen Save mit Veto belegt, per Short-Circuit
abbrechen kann. Verwenden Sie `dispatch_best_effort` für Fan-out, bei
dem ein fehlschlagender Listener nicht die übrigen zum Schweigen
bringen soll - die meisten Observability-Events fallen hierunter.

Überschreiben Sie die Trait-Methode, um sich für Queued-Zustellung zu
entscheiden:

```rust
impl Event for ExpensiveAuditTrail {
    fn event_name() -> &'static str { "ExpensiveAuditTrail" }
    fn queued() -> bool { true }
}
```

Queued Listener sind durch ein prozessweites Semaphore begrenzt. Die
Standardobergrenze ist 256 gleichzeitige Tasks; überschreiben Sie sie
pro Dispatcher mit `EventDispatcher::with_concurrency(n)` oder global
über die Env-Variable `EVENT_MAX_CONCURRENCY`. Jede Task wiederholt
bis zu 3 Versuche mit gejittertem Backoff von 100ms bis 2s, bevor sie
aufgibt - das sind in-process Wiederholungen bei transienten
Fehlern, nicht der minutenlange Zeitplan der dauerhaften Queue.

## Subscriber - verwandte Registrierungen bündeln

Wenn mehrere Listener zu einem Feature gehören, registriert ein
`Subscriber` sie als Einheit. Entspricht dem Subscriber-Muster von
Laravels `EventServiceProvider`.

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

// In bootstrap.rs - eine Zeile pro Subscriber statt drei pro Listener:
EventFacade::subscribe(Arc::new(UserEventSubscriber { db: db.clone() })).await;
```

`subscribe` nimmt `Arc<S>` entgegen, damit Listener, die sich Zustand
mit dem Subscriber teilen müssen, den `Arc` klonen und einfangen
können.

## Listener inspizieren und entfernen

```rust
if EventFacade::has_listeners::<UserRegistered>() {
    EventFacade::dispatch(UserRegistered { user_id: 42 }).await?;
}

let removed: usize = EventFacade::forget::<UserRegistered>();
```

`has_listeners::<E>()` entspricht Laravels
`Event::hasListeners($eventName)`. `forget::<E>()` verwirft jeden für
diesen Event-Typ registrierten Listener und gibt die Anzahl der
Entfernten zurück. Production-Code braucht `forget` selten - die
Listener-Registrierung passiert normalerweise einmalig beim
Bootstrap -, aber Hot-Swap- und Testcode greifen dazu.

Beide Methoden geben sichere Defaults zurück, wenn die Sperre der
Listener-Registry vergiftet ist (`false` beziehungsweise `0`), wobei
ein `tracing::error!` protokolliert wird, damit der Fehlschlag
beobachtbar ist.

## Push und Flush

`push` erfasst ein Event in einem Pro-Event-Name-Bucket, ohne es zu
feuern. `flush::<E>()` leert den Bucket und dispatcht alles in
Erfassungsreihenfolge. Entspricht Laravels Paar `Event::push` /
`Event::flush`.

```rust
// In einem Handler, der die Arbeit in zwei Phasen erledigt:
EventFacade::push(UserRegistered { user_id: 42 }).await;
// … Rendering, Validierung, weitere Arbeit …
EventFacade::flush::<UserRegistered>().await?;
```

Gepushte Events ignorieren den `defer`-Scope - sie sind bereits
explizit deferred. `forget_pushed()` verwirft jedes gepushte Event
ohne Dispatch und gibt die Anzahl der Verworfenen zurück. Entspricht
`Event::forgetPushed()`.

## defer - jeden Dispatch innerhalb eines Callbacks puffern

`defer(only, async { … })` führt den Callback mit einem Task-lokalen
Puffer im Scope aus. Jeder `dispatch`- / `dispatch_best_effort`-Aufruf
innerhalb des Callbacks wird erfasst und nach der Rückkehr des
Callbacks erneut abgespielt. Entspricht Laravels
`Event::defer($callback, ?$events)`.

```rust
let ((), flush_err) = EventFacade::defer::<_, ()>(None, async {
    do_work_part_one().await?;
    EventFacade::dispatch(WorkStarted).await?; // gepuffert
    do_work_part_two().await?;
    EventFacade::dispatch(WorkFinished).await?; // gepuffert
    Ok(())
})
.await?;
// An diesem Punkt haben WorkStarted und WorkFinished beide in
// Reihenfolge gefeuert.
// `flush_err` trägt den ersten Dispatch-Fehler aus dem Replay (falls
// vorhanden).
```

Übergeben Sie `Some(&["EventOne", "EventTwo"])`, um NUR diese
Event-Namen zu deferren; alles andere dispatcht wie üblich inline.
Ein Callback-Fehler unterbricht per Short-Circuit - gepufferte Events
werden verworfen, der Fehler propagiert.

Der defer-Puffer ist pro Tokio-Task, sodass zwei gleichzeitige
`defer`-Aufrufe sich nicht gegenseitig den Zustand zerstören können.

## Queued Listener - prozessintern vs. dauerhaft

Zwei klar getrennte „queued“-Ebenen, und die Benennung ist wichtig:

| Bedarf | Greifen Sie zu |
|---|---|
| Der Listener soll außerhalb der Task laufen; ein Verlust beim Absturz ist in Ordnung | `Event::queued() = true` auf dem Event-Trait |
| Die Arbeit des Listeners MUSS Absturz und Neustart überleben | `QueuedListener<E, J>` (Brücke Event → dauerhafter Job) |

`Event::queued() = true` lässt den Dispatcher jeden Listener als eigene
Tokio-Task starten, begrenzt durch eine prozessweite Semaphore, mit
begrenzter Wiederholung (3 Versuche, Backoff mit Jitter). Die Arbeit
läuft in diesem Prozess; ein Absturz verwirft die gerade laufenden
Listener. Die [Leerung beim Graceful Shutdown](#leeren-beim-shutdown)
wartet bis zu einer Frist auf die laufenden Tasks.

`QueuedListener<E, J>` ist ein fertiger Listener, der aus jedem Event
einen [`Job`](queues.md) baut und ihn auf die dauerhafte Queue schiebt.
Das Event feuert weiterhin synchron; der Listener reiht nur ein - was
schnell geht -, sodass die Latenz der Anfrage niedrig bleibt. Der Job
selbst übersteht den Absturz, weil die Queue dauerhaft ist.

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

Der `QueuedListener` braucht das Event nur als gewöhnliches synchrones
Event - die Dauerhaftigkeit lebt in der Queue, nicht im Dispatcher.

### Einen Queued Listener entprellen

Ein `QueuedListener` läuft durch `Queue::push`, ein Listener ist also in
dem Moment entprellt, in dem sein **Job** `Job::debounce_for` deklariert -
nichts weiter zu verdrahten, und `Job::debounce_id` gibt Ihnen ein
Fenster pro Entität.

Gehört das Fenster zur Registrierung statt zum Job, nutzen Sie
`DebouncedListener` und leiten Sie den Schlüssel aus dem Event ab:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::events::{DebouncedListener, EventFacade};

EventFacade::listen::<OrderUpdated, _>(Arc::new(
    DebouncedListener::<OrderUpdated, ReindexOrder>::new(
        Duration::from_secs(30),
        |e| ReindexOrder { order_id: e.order_id },
    )
    .max_wait(Duration::from_secs(300))
    .keyed_by(|e| e.order_id.to_string()),
))
.await;
```

Vier `OrderUpdated`-Events für Bestellung 55 reihen vier Jobs ein und
führen einen aus. Den vollständigen Vertrag beschreibt
[Warteschlange](queues.md).

## Leeren beim Shutdown

Queued In-Process-Listener spawnen in ein `JoinSet`, das vom
Dispatcher getrackt wird. Die Graceful-Shutdown-Sequenz des Servers
ruft `EventFacade::drain_queued(timeout)` auf, um auf sie zu warten:

```rust
let still_running = EventFacade::drain_queued(Duration::from_secs(30)).await;
if still_running > 0 {
    tracing::warn!(still_running, "queued listeners abandoned at shutdown");
}
```

Drain gibt die Anzahl zurück, die beim Ablauf der Deadline noch
liefen (`0` = vollständig geleert). Nachzügler nach der Deadline
werden abgebrochen, damit der Shutdown nicht hängen bleiben kann.

## Events mit Broadcasting verdrahten

`EventFacade::broadcast::<E>(hub)` verdrahtet eine einzeilige Brücke
von einem dispatchten Event zu einem `BroadcastHub`. Jeder Typ, der
`Broadcastable` und `Event` implementiert, kann auf diesem Weg
gebroadcastet werden; Listener empfangen den typisierten Payload, und
Abonnenten auf den benannten Kanälen empfangen die Broadcast-Envelope.

```rust
use suprnova::EventFacade;
use std::sync::Arc;

let hub: Arc<dyn suprnova::BroadcastHub> = Arc::new(broadcast_hub);
EventFacade::broadcast::<OrderShipped>(hub).await;

// Jeder spätere Dispatch wird auch auf den Kanälen veröffentlicht,
// die OrderShipped::broadcast_on() deklariert:
EventFacade::dispatch(OrderShipped { order_id: 42, user_id: 99 }).await?;
```

Siehe [Broadcasting](broadcasting.md) für das Kanal-Modell
(öffentlich / privat / Presence) und den Trait `Broadcastable`.

## Eingebaute Events

Das Framework dispatcht eine feste Menge von Events aus seinen
eigenen Subsystemen. Sie entscheiden sich durch das Registrieren von
Listenern dafür; ist kein Listener registriert, sind die Events
No-Ops.

| Subsystem | Events | Dispatcht von |
|---|---|---|
| Fehlerbehandlung | `ErrorOccurred` | Jede 5xx-Response (zurückgegebener `FrameworkError` oder abgefangener Panic) |
| Auth (Guards) | `Auth\\Attempting`, `Auth\\Authenticated`, `Auth\\Login`, `Auth\\Logout`, `Auth\\Failed` | `StatefulGuard::attempt` / `login` / `logout` / `once` |
| Auth-Flows | `EmailVerified`, `PasswordResetLinkSent`, `PasswordResetCompleted`, `AccountLocked`, `AccountUnlocked`, `TwoFactorEnrolled`, `TwoFactorChallenged`, `TwoFactorChallengeFailed`, `TwoFactorDisabled` | `auth_flows::{EmailVerification, PasswordReset, BruteForce, TwoFactor}` |
| Datenbank | `Database\\ConnectionEstablished`, `Database\\QueryExecuted`, `Database\\TransactionBeginning`, `Database\\TransactionCommitted`, `Database\\TransactionRolledBack`, `Database\\DatabaseBusy` | `DbConnection::connect`, `ExecutorChoice`-Helfer, `DB::transaction` |
| Mail | `Suprnova\\Mail\\MessageSending`, `Suprnova\\Mail\\MessageSent` | `MailBuilder::send` vor/nach dem Transport |
| Benachrichtigungen | `Suprnova::Notifications::Sending`, `Suprnova::Notifications::Sent`, `Suprnova::Notifications::Failed` | Jede Kanal-Zustellung |
| Queue (Worker) | `queue::JobQueueing`, `JobQueued`, `JobProcessing`, `JobProcessed`, `JobAttempted`, `JobExceptionOccurred`, `JobFailed`, `JobReleased`, `JobReleasedAfterException`, `JobTimedOut`, `Looping`, `WorkerStarting`, `WorkerStopping`, `WorkerInterrupted`, `UniqueJobSkipped`, `QueuePaused`, `QueueResumed`, `QueuesPaused`, `QueuesResumed` | `Queue::push` / `Queue::push_unique` / `run_worker` / `Queue::pause` / `resume` / `pause_all` / `resume_all` |
| Features | `FeatureUpdated`, `FeatureDeleted` | `features::admin`-CRUD |
| Eloquent (pro Model) | 16 Lifecycle-Events - `Retrieved`, `Saving`, `Saved`, `Creating`, `Created`, `Updating`, `Updated`, `Deleting`, `Deleted`, `Restoring`, `Restored`, `ForceDeleting`, `ForceDeleted`, `Replicating`, `Pruning`, `Pruned` - emittiert unter dem `events::`-Submodul jedes Models | Das Makro `#[suprnova::model]` verdrahtet diese in Save/Update/Delete |

`ErrorOccurred` ist der dedizierte Hook, um 5xx-Exceptions an Sentry,
Datadog, Slack usw. zu verschicken. Der Dispatch ist best-effort und
gespawnt, sodass ein defekter Sentry-Listener nicht die übrigen zum
Schweigen bringen kann und die Response-Konvertierung nie darauf
wartet. Siehe [Fehlermodell](error-model.md) für den vollständigen
Panic-Recovery- und Konvertierungsvertrag.

Model-Lifecycle-Events feuern fail-fast: Ein `Saving`-Listener, der
`EventResult::Cancel` zurückgibt (über den Trait
`CancellableListener`), bricht den Save ab. Siehe
[Eloquent-Observer und Lifecycle-Events](eloquent.md).

## DB::listen - Queries beobachten

Für Observability pro Query können Sie entweder einen typisierten
`Listener<QueryExecuted>` über den Dispatcher registrieren oder,
üblicher, einen `DB::listen`-Callback, der Laravels
`DB::listen(function ($q) { ... })`-Signatur entspricht:

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

Der Callback empfängt ein `QueryExecuted`, das SQL, Bindings,
Wall-Clock-Dauer, Connection-Namen, die Read-/Write-Klassifikation
und das finale `Result` trägt (sodass auch fehlgeschlagene Queries
beobachtbar sind). `QueryExecuted::to_raw_sql()` inlined Bindings zur
Bequemlichkeit im Log - Debug-Format, NICHT SQL-sicher.

Zwei Garantien zu Reentranz und Kosten:

- **Reentranz-Guard.** Ein Listener, der selbst eine Query ausführt,
  feuert `QueryExecuted` nicht erneut für diese verschachtelte
  Query - der Dispatcher setzt ein Task-lokales Flag, während ein
  Listener läuft, und der Executor überspringt die Emission
  innerhalb dieses Scopes. Ein Log-zu-DB-Listener wird also nicht in
  eine Schleife laufen.
- **Null Overhead, wenn niemand lauscht.** Der Executor prüft ein
  kombiniertes `query_observation_active()` (irgendein direkter
  Listener, irgendein registrierter `Listener<QueryExecuted>`, ODER
  Query-Log aktiviert), bevor er den Event-Payload baut. Sind alle
  drei aus, wird der gesamte Emissionspfad per Short-Circuit
  abgebrochen.

## Testen - `EventFacade::fake()`

`EventFacade::fake()` tauscht den globalen Dispatcher gegen einen
Recorder aus. Dispatchte Events gehen in die Aufzeichnung, statt
Listener auszuführen. Der Fake hält für die Lebensdauer des Guards
einen prozessweiten Serializer, sodass parallele `#[tokio::test]`s,
die ihn verwenden, einer nach dem anderen laufen - Tests brauchen
keinen eigenen `serial_test`-Mutex mehr.

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

| Helfer | Prüft |
|---|---|
| `assert_dispatched::<E>(pred)` | mindestens ein passendes `E` wurde dispatcht |
| `assert_dispatched_once::<E>()` | genau ein `E` wurde dispatcht |
| `assert_dispatched_times::<E>(n)` | genau `n` von `E` wurden dispatcht |
| `assert_not_dispatched::<E>(pred)` | kein passendes `E` wurde dispatcht |
| `assert_nothing_dispatched()` | KEINE Events irgendeines Typs wurden dispatcht |
| `assert_listening::<E, L>()` | ein Listener `L` wurde für `E` registriert |
| `has_dispatched::<E>()` | bool: irgendein `E` aufgezeichnet |
| `dispatched::<E>(pred)` | `Vec<E>`-Klone der passenden Events |
| `dispatched_count::<E>(pred)` | Anzahl der passenden Events |
| `dispatched_events()` | `HashMap<&'static str, usize>` aller Dispatches |

### Selektives Faken

```rust
// Nur diese Events faken; alles andere dispatcht normal.
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// Jedes Event faken, AUSSER diesen.
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

Entspricht Laravels `Event::fake([…])` und
`EventFake::except($events)`.

### Mute - Events verwerfen, ohne sie aufzuzeichnen

`EventFacade::muted(async { … })` führt den Callback mit einem
gesetzten Task-lokalen "silent dispatcher"-Flag aus; jedes Event, das
darin dispatcht wird, wird verworfen, ohne aufgezeichnet zu werden
oder Listener aufzurufen. Das Suprnova-Analogon zu Laravels
`NullDispatcher`, gescoped auf einen Callback.

```rust
EventFacade::muted(async {
    // Keine Listener feuern, keine Events werden aufgezeichnet.
    run_bulk_import().await;
})
.await;
```

Anders als `fake()` erwirbt `muted` NICHT den Prozess-Serializer -
zwei muted-Scopes können parallel laufen.

### `assert_listening` - überprüfen, dass ein Listener verdrahtet ist

Verwenden Sie das, um Bootstrap-Verdrahtung zu testen, ohne ein Event
zu feuern:

```rust
#[tokio::test]
async fn bootstrap_wires_welcome_listener() {
    let _guard = EventFacade::fake();
    bootstrap::register_listeners().await;
    suprnova::events::assert_listening::<UserRegistered, SendWelcomeEmail>();
}
```

Der Fake beobachtet Registrierungen über die `listen`-Methode des
Dispatchers, sodass die Registrierung INNERHALB des Scopes des Fakes
passieren muss - Listener, die vor `EventFacade::fake()` registriert
wurden, werden von `assert_listening` NICHT gesehen.

## Laravel-Paritätsreferenz

Jede Methode der Laravel-13-Facade `Event` und von `EventFake`, die
ein typisiertes Rust-Äquivalent hat, wird unter dem am nächsten
passenden Namen mitgeliefert. Methoden, die Laravel bereitstellt und
die nicht zu typisiertem Rust passen, werden mit einer kurzen
Anmerkung ausgelassen.

| Laravel | Suprnova |
|---|---|
| `Event::dispatch($event)` | `EventFacade::dispatch(event).await` |
| `Event::dispatch($event)` (halt arg) | verwenden Sie `dispatch` (fail-fast bei `Err`) |
| `Event::until($event)` | `dispatch` (typisiert: erstes `Err` hält an) |
| `Event::listen($event, $listener)` | `EventFacade::listen::<E, L>(Arc::new(L))` |
| `Event::hasListeners($name)` | `EventFacade::has_listeners::<E>()` |
| `Event::forget($event)` | `EventFacade::forget::<E>()` |
| `Event::push($event)` | `EventFacade::push(event).await` |
| `Event::flush($event)` | `EventFacade::flush::<E>().await` |
| `Event::forgetPushed()` | `EventFacade::forget_pushed().await` |
| `Event::defer($callback, ?$events)` | `EventFacade::defer(only, async {…}).await` |
| `Event::subscribe($subscriber)` | `EventFacade::subscribe(Arc::new(S)).await` |
| `Event::fake()` | `EventFacade::fake()` (Guard) |
| `Event::fake([$names])` | `EventFacade::fake_only(&["…"])` |
| `EventFake::except($names)` | `EventFacade::fake_except(&["…"])` |
| `EventFake::assertDispatched` | `assert_dispatched` |
| `EventFake::assertDispatchedOnce` | `assert_dispatched_once` |
| `EventFake::assertDispatchedTimes` | `assert_dispatched_times` |
| `EventFake::assertNotDispatched` | `assert_not_dispatched` |
| `EventFake::assertNothingDispatched` | `assert_nothing_dispatched` |
| `EventFake::assertListening` | `assert_listening` |
| `EventFake::hasDispatched` | `has_dispatched` |
| `EventFake::dispatched` | `dispatched` (gibt `Vec<E>` zurück) |
| `EventFake::dispatchedEvents` | `dispatched_events` (Name → Anzahl-Map) |
| `NullDispatcher` | `EventFacade::muted(async {…}).await` |
| `Event::wildcards` (`User.*`-Patterns) | nicht mitgeliefert - verwenden Sie typisierte Listener oder den Trait `Observer<M>` für Lifecycle-Hooks pro Model |
| `Event::subscribe` (String-Subscriber) | verwenden Sie den typisierten Trait `Subscriber` |
| `DB::listen(function ($q) {…})` | `DB::listen(Arc::new(|q| {…}))` - dieselbe Form, nimmt `&QueryExecuted` entgegen |

### Warum Suprnova abweicht

Laravels Dispatcher stützt sich auf PHPs stringly-typed Runtime:
Events sind Klassennamen, die als Zeichenketten übergeben werden,
Listener sind Klassennamen, die über den Container nachgeschlagen
werden, und `Event::listen('User.*', ...)` funktioniert, weil
Wildcards über Klassennamen-Zeichenketten in PHP Sinn ergeben. In
Rust ist das Äquivalent zu "dieser Listener behandelt `User.*`"
"dieser Listener ist generisch über `E: UserEvent`" - ein Trait, kein
String-Match. Suprnova lässt Wildcards also zugunsten des Typsystems
weg, mit dem Ergebnis, dass fehlerhafte Refactors zu Compile-Fehlern
statt zu Laufzeit-Fehlleitungen werden.

Die andere Abweichung ist `defer`: Laravels defer verlässt sich auf
das Request-pro-Prozess-Modell, um den Deferral-Scope zu begrenzen.
Suprnova bedient viele gleichzeitige Anfragen in einem Prozess, daher
ist der Deferral-Puffer Task-lokal. Zwei gleichzeitige
`defer`-Aufrufe bekommen jeweils ihren eigenen Puffer; die Aufrufe
können sich nicht gegenseitig zerstören, und es gibt keinen
versteckten globalen Zustand, der lecken könnte.

## Wo jedes Teil lebt

| Teil | Datei |
|---|---|
| Trait `Event`, `Listener<E>`, `Subscriber` | `framework/src/events/mod.rs` |
| `EventDispatcher`, `EventFacade` (Facade-Struktur) | `framework/src/events/dispatcher.rs` |
| `ErrorOccurred` | `framework/src/events/builtins.rs` |
| `QueuedListener<E, J>` | `framework/src/events/queued_listener.rs` |
| `assert_dispatched*`, `EventFakeGuard`, `muted` | `framework/src/events/testing.rs` |
| Eingebaute Event-Payloads | `framework/src/{database,auth,auth_flows,mail,notifications,queue,features}/events.rs` |
| Lifecycle-Events pro Model | makro-generiert in das `events::`-Submodul jedes Models |

## Nächste Schritte

- [Fehlermodell](error-model.md) - `ErrorOccurred` und der
  5xx-Konvertierungspfad
- [Warteschlange](queues.md) - dauerhafte Jobs, die
  absturztolerante Stufe; `QueuedListener` verbrückt in diese
- [Broadcasting](broadcasting.md) - dispatchte Events über
  `EventFacade::broadcast::<E>(hub)` mit WebSocket-Kanälen verdrahten
- [Eloquent](eloquent.md) - Model-Lifecycle-Events und der Trait
  `Observer<M>`
- [Datenbank](database.md) - `DB::listen` und das Event
  `Database\\QueryExecuted`
