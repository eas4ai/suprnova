# Bus

Der Bus ist Suprnovas **synchroner** Command-Dispatcher. Sie
definieren ein typisiertes `Command` (`{ input, Output type }`),
registrieren beim Boot einen `Handler` dafür, und danach kann jeder
Code im Prozess `Bus::dispatch(cmd).await` aufrufen und ein
`Dispatched<T>` zurückbekommen, das das typisierte Ergebnis des
Handlers trägt.

Der Bus ist mit [`Queue`](queues.md) gepaart - seinem asynchronen
Pendant. Es sind zwei bewusst getrennte Facades, kein einzelner
routender Dispatcher:

| Wenn Sie ... wollen                                    | Verwenden Sie  |
|-------------------------------------------------------|----------------|
| Die Arbeit *jetzt* ausführen, in dieser Task, das Ergebnis zurückbekommen | `Bus`          |
| Die Arbeit an einen Worker schieben, mit Wiederholung bei Fehlschlag, dauerhaft  | `Queue`        |

Der Aufrufer entscheidet sich explizit. Suprnova liefert keinen
`ShouldQueue`-Marker - auf Tokio sind beide Pfade nicht-blockierend,
sodass die explizite Auswahl klarer und schneller ist als implizites
Routing.

## Schnellstart

Zehn Zeilen vom Command zum Dispatch:

```rust
use serde::{Deserialize, Serialize};
use suprnova::async_trait;
use suprnova::bus::command::{Command, Handler};
use suprnova::bus::Bus;
use suprnova::error::FrameworkError;

#[derive(Serialize, Deserialize)]
pub struct ChargeCustomer { pub customer_id: i64, pub cents: i64 }

#[async_trait]
impl Command for ChargeCustomer {
    type Output = String; // die Charge-ID, die wir zurückbekommen
    fn command_name() -> &'static str { "ChargeCustomer" }
}

pub struct ChargeCustomerHandler;

#[async_trait]
impl Handler<ChargeCustomer> for ChargeCustomerHandler {
    async fn handle(&self, cmd: ChargeCustomer) -> Result<String, FrameworkError> {
        Ok(format!("charge-{}-{}", cmd.customer_id, cmd.cents))
    }
}

// At boot (once):
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);

// In einem Request-Handler:
let charge_id = Bus::dispatch(ChargeCustomer { customer_id: 42, cents: 1999 })
    .await?
    .unwrap_executed();
```

## Befehle definieren

Ein `Command` ist jede serialisierbare Struktur mit einem zugehörigen
`Output`-Typ und einer eindeutigen `command_name()`:

```rust
#[async_trait]
pub trait Command: Serialize + DeserializeOwned + Send + Sync + 'static {
    type Output: Send + 'static;
    fn command_name() -> &'static str;
}
```

Der `Output` ist das, was der Handler zurückgibt. Er muss nur
`Send + 'static` sein - der eigentliche Dispatch-Pfad hält Werte nativ
über `Box<dyn Any>`, ohne serde-Roundtrip. Das bedeutet, dass
Nicht-serde-Ausgaben wie `Bytes`, opake Handles oder ein
`Arc<Mutex<…>>` als lebende Werte zum Aufrufer zurück-roundtripen. Die
`Serialize + DeserializeOwned`-Schranke auf `Command` selbst ist für
den Fake-Capture-Pfad gedacht: `Bus::fake()` zeichnet jedes dispatchte
Command als `serde_json::Value` auf, sodass prädikatbasierte
Assertions (`assert_dispatched`, `assert_dispatched_times`) es
decodieren und untersuchen können.

`command_name()` sollte eine stabile, pro konkreter
`Command`-Implementierung eindeutige Zeichenkette sein. Sie erscheint
in Fehlschlagsmeldungen von
`assert_dispatched`/`assert_dispatched_times` und in Fehler-Rückgaben,
wenn kein Handler registriert ist.

## Handler registrieren

Ein `Handler<C>` ist eine typisierte asynchrone Funktion, die das
Command entgegennimmt und `Result<C::Output, FrameworkError>`
zurückgibt:

```rust
#[async_trait]
pub trait Handler<C: Command>: Send + Sync + 'static {
    async fn handle(&self, cmd: C) -> Result<C::Output, FrameworkError>;
}
```

Rufen Sie `Bus::register::<C, H>(handler)` einmal pro Command-Typ beim
Boot auf. Die Registry ist global; ein erneutes Registrieren desselben
`C` überschreibt den vorherigen Handler (Tests verlassen sich darauf,
um Implementierungen auszutauschen) und emittiert ein
`tracing::warn!`, damit eine doppelte Bindung aus zwei
Boot-Zeit-Service-Registrierungen im Log sichtbar ist.

```rust
Bus::register::<ChargeCustomer, _>(ChargeCustomerHandler);
Bus::register::<RefundCustomer, _>(RefundCustomerHandler);
```

## Dispatchen

`Bus::dispatch::<C>(cmd)` führt den registrierten Handler in-process
aus und gibt ein `Dispatched<C::Output>`-Enum zurück:

```rust
pub enum Dispatched<T> {
    Executed(T),  // Handler lief, hier ist das Ergebnis
    Captured,    // Bus::fake() war aktiv, Handler lief NICHT
}
```

`Dispatched<T>` hat vier Helfer:

- `.unwrap_executed()` - gibt den Wert zurück, Panic bei `Captured`
- `.executed() -> Option<T>` - Umwandlung in `Option`
- `.is_executed()` - bool-Prädikat
- `.is_captured()` - bool-Prädikat

Für Aufrufstellen im echten Modus ist `.unwrap_executed()` die
idiomatische Form.

### `Bus::chain` - sequenziell

`Bus::chain(Vec<C>)` führt Commands eines nach dem anderen aus und
stoppt beim (einschließlich) ersten Fehler. Alle Commands müssen vom
gleichen Typ sein. Gibt
`Vec<Result<Dispatched<C::Output>, FrameworkError>>` zurück - einen
Eintrag pro versuchtem Command.

```rust
let results = Bus::chain(vec![
    ChargeCustomer { customer_id: 1, cents: 100 },
    ChargeCustomer { customer_id: 2, cents: 200 },
    ChargeCustomer { customer_id: 3, cents: 300 },
]).await;

// Erfolgreiche Charge-IDs bis zum ersten Fehlschlag sammeln:
let charge_ids: Vec<String> = results
    .into_iter()
    .filter_map(|r| r.ok().and_then(|d| d.executed()))
    .collect();
```

`Bus::chain` ist absichtlich nur für homogene Typen ausgelegt - der
Dispatcher gibt `Dispatched<C::Output>` zurück, was nur dann korrekt
typisiert ist, wenn jede Eingabe einen `Output` teilt. Für
Laravel-artige heterogene Chains (gemischte Job-Typen, bei denen jeder
Schritt den nächsten anstößt), verwenden Sie
[`Queue::chain`](queues.md) - die Queue verpackt jeden Job in eine
typisierte Envelope und unterliegt daher nicht derselben
Einschränkung.

### `Bus::batch` - nebenläufig

`Bus::batch(Vec<C>)` führt Commands nebenläufig über
`futures::join_all` aus und sammelt die Ergebnisse in
Eingabereihenfolge. Dieselbe Einschränkung auf homogene Typen wie bei
`chain`.

```rust
let results = Bus::batch(vec![
    SendWelcomeEmail { user_id: 1 },
    SendWelcomeEmail { user_id: 2 },
    SendWelcomeEmail { user_id: 3 },
]).await;
```

`Bus::batch` ist aus demselben Grund wie `chain` nur für homogene
Typen ausgelegt. Für heterogene, persistierte Batches mit
Progress-Callbacks, Lifecycle-Events und einem `BatchRepository`,
verwenden Sie [`Queue::batch`](queues.md).

## Testen

Installieren Sie den Fake am Anfang des Tests. `install_fake()`
erwirbt für die Lebensdauer des Guards einen prozessweiten
`FAKE_SERIAL`-Mutex, sodass zwei parallele `Bus::fake()`-Tests sich
nicht gegenseitig den erfassten Store zerstören können - der zweite
blockiert, bis der erste Guard verworfen wird. Sie markieren den Test
trotzdem mit `#[serial]`, wenn ein Geschwister-Test in derselben
Binary den echten `Bus::dispatch` aufruft: Ein Aufrufer mit echtem
Dispatch erwirbt `FAKE_SERIAL` nicht, sodass er ohne `#[serial]` mit
einem parallelen Fake-Test um den Zugriff konkurrieren und
`is_active() == true` beobachten kann. `FAKE_SERIAL` beseitigt die
Fake-gegen-Fake-Gefahr, `#[serial]` beseitigt die
Echt-gegen-Fake-Gefahr.

```rust
use serial_test::serial;
use suprnova::bus::Bus;
use suprnova::bus::testing::{
    assert_dispatched,
    assert_dispatched_times,
    assert_not_dispatched,
    assert_nothing_dispatched,
    install_fake,
};

#[tokio::test]
#[serial]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(/* … */).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

Der Fake erfasst dispatchte Commands, ohne deren Handler auszuführen.
Ein `Bus::dispatch`-Aufruf gibt `Ok(Dispatched::Captured)` zurück
(keine Handler-Ausgabe) statt `Executed`. Echte Fehler -
Encode-/Decode-Fehlschläge, ein fehlender registrierter Handler, bevor
der Fake installiert wurde - treten weiterhin als `Err(_)` auf.

`install_fake()` gibt einen `BusFakeGuard` zurück. Verwerfen Sie ihn
(er ist RAII), und der Fake wird gelöscht und der `FAKE_SERIAL`-Mutex
freigegeben. Das typische Idiom ist `let _guard = install_fake();` am
Anfang des Tests.

### Assertion-Oberfläche

| Assertion                                            | Prüft…                                                   |
|------------------------------------------------------|------------------------------------------------------------|
| `assert_dispatched::<C>(pred)`                       | mindestens ein Command vom Typ `C`, das auf `pred` passt           |
| `assert_not_dispatched::<C>(pred)`                   | null Commands vom Typ `C`, die auf `pred` passen                  |
| `assert_dispatched_times::<C>(pred, count)`          | genau `count` Commands vom Typ `C`, die auf `pred` passen       |
| `assert_nothing_dispatched()`                        | null dispatchte Commands jeglichen Typs unter dem aktiven Fake |

Alle vier geraten in Panic mit `Bus::fake() must be active`, wenn kein
Fake installiert ist. Die typgebundenen geraten in Panic mit
`expected … dispatched <command_name> …`, wenn die Anzahl nicht passt.
`assert_nothing_dispatched` gerät in Panic mit
`expected no dispatched commands but found <n>`.

## Wann stattdessen `Queue` verwenden

Greifen Sie zu [`Queue`](queues.md), wenn Sie eines der folgenden
wollen:

- **Dauerhaftigkeit über Neustarts hinweg.** Ein eingereihter Job überlebt einen Prozessabsturz, wenn der Treiber `database` oder `redis` ist.
- **Wiederholungen mit Backoff.** Der Queue-Worker wendet bei jedem Fehlschlag `Job::max_tries` + `Job::backoff` (exponential / fixed / sequence) an.
- **Timeout pro Job.** `Job::timeout` + `Job::fail_on_timeout` werden von der Worker-Schleife beachtet.
- **Verzögerte Ausführung.** `Queue::later(duration, job)` oder `Queue::push_later(job, at)`.
- **Dedupe / Idempotenz.** `Job::unique_id` + `Queue::push_unique` sperrt erneute Einreichungen für eine konfigurierbare TTL.
- **Entkopplung des Aufrufers vom Worker.** Führen Sie Jobs auf einer eigenen Flotte von `cargo run --bin app -- queue:work`-Workern aus.

Greifen Sie zu `Bus`, wenn Sie eines der folgenden wollen:

- **In-Process, sofort ausführen.** Keine Serialisierung über Prozesse hinweg.
- **Typisiertes Ergebnis zurück an den Aufrufer.** `Dispatched<C::Output>` trägt den typisierten Rückgabewert des Handlers zur Aufrufstelle.
- **Synchrone Komposition.** Ein Request-Handler, der Arbeit in kleinere `Command`-Aufrufe zerlegt und jedes Ergebnis der Reihe nach liest.

Eine typische App verwendet beides: synchrone Request-Pfade dispatchen
ergebnisliefernde Operationen über den `Bus`, und „fire and forget“ /
dauerhafte Arbeit wird über `Queue` eingereiht.

## Nächste Schritte

- [Warteschlange](queues.md) - das asynchrone Pendant, Treiber, Worker, Retry-Richtlinie, heterogene Chains und Batches
- [Ereignisse](events.md) - Pub/Sub-Dispatcher (ein Event → viele Listener)
- [Workflows](workflows.md) - lang laufende, zustandsbehaftete Arbeit, die Neustarts überlebt, wenn eine Chain nicht genug ist
- [Testen](testing.md) - `#[suprnova_test]`, Container-Fakes und das prozessweite Serialisierer-Muster, das `Bus::fake()` verwendet
