# Mocking und Fakes

Jede externe Oberfläche in Suprnova liefert einen In-Process-Fake,
der erfasst, was Ihr Code gesendet hätte - Mail, Benachrichtigungen,
eingereihte Jobs, dispatchte Commands, ausgelöste Events,
geschriebene Dateien, ausgehende HTTP-Aufrufe - plus ein passendes
Set von Assertions, das Sie danach ausführen. Die Form ist immer
dieselbe: den Fake installieren, den zu testenden Code ausführen,
assertieren, was erfasst wurde. Dieses Kapitel ist der konsolidierte
Überblick; jedes Subsystem-Kapitel ([Mail](mail.md),
[Benachrichtigungen](notifications.md), [Warteschlange](queues.md),
[Command-Bus](bus.md), [Ereignisse](events.md),
[Dateispeicher](filesystem.md), [HTTP-Client](http-client.md))
behandelt seinen Fake im Detail.

## Die sieben Fakes

| Oberfläche | Einstiegspunkt | Assertion-Stil | Parallelsicherheit | Kapitel |
|-----------------|---------------------------------------------------|---------------------------------------|----------------------------------------------------|--------------------------------------|
| Mail            | `Mail::fake()` → `MailFake`-Guard                 | Methoden auf dem Guard                | braucht `#[serial]` - globaler Transport, kein Serialisierungs-Mutex | [mail.md](mail.md)                   |
| Benachrichtigungen | `Notify::fake()` → `NotifyFakeGuard`           | freie Funktionen in `notifications::testing` | Guard hält prozessweiten Serialisierungs-Mutex | [notifications.md](notifications.md) |
| Warteschlange   | `suprnova::queue::testing::install_fake()`        | freie Funktionen in `queue::testing`  | Guard hält prozessweiten Serialisierungs-Mutex     | [queues.md](queues.md)               |
| Bus             | `suprnova::bus::testing::install_fake()`          | freie Funktionen in `bus::testing`    | Guard hält prozessweiten Serialisierungs-Mutex     | [bus.md](bus.md)                     |
| Ereignisse      | `EventFacade::fake()` → `EventFakeGuard`          | freie Funktionen in `events`          | Guard hält prozessweiten Serialisierungs-Mutex     | [events.md](events.md)               |
| Storage         | `Storage::fake()` → `StorageFakeGuard`            | `DiskAssertExt`-Methoden auf einer Disk | Guard hält prozessweiten Serialisierungs-Mutex   | [filesystem.md](filesystem.md)       |
| HTTP-Client     | `Http::fake(\|\| async { … }).await`              | `assert_sent` / `assert_not_sent`     | task-lokal - echt parallel über Tests hinweg       | [http-client.md](http-client.md)     |

Ein paar Invarianten gelten über alle sieben:

- **Der Fake zeichnet auf, das echte Backend läuft nicht.** Mail
  wird nicht gesendet, Jobs werden nicht an den Treiber gepusht,
  Handler laufen nicht, Events überspringen ihre Listener, HTTP
  erreicht das Netzwerk nicht, Dateischreibvorgänge gehen in eine
  Memory-Disk. Die erfasste Seite trägt genug Information, um zu
  assertieren, was passiert wäre.
- **Der Guard ist RAII.** Das Droppen des Guards stellt wieder her,
  was zuvor vorhanden war (der vorherige Mail-Transport, eine
  saubere Storage-Registry, keine Aufzeichnung für Events usw.).
  Tests brauchen keinen Teardown-Schritt.
- **Der Fake lügt nicht über Fehler.** Ruft Ihr Code
  `Bus::dispatch` für ein nicht registriertes Command auf, liefert
  der Fake trotzdem `Err(_)` - nur erfolgreiche Dispatches werden
  erfasst.

## Die Formen, und warum sie sich unterscheiden

Drei Muster wiederholen sich. Zu wissen, welches Muster ein Fake
verwendet, sagt Ihnen, ob Sie eine freie Funktion importieren, eine
Methode auf dem Guard aufrufen oder den Testkörper in eine Closure
einwickeln müssen.

### Guard-mit-Methoden (Mail)

`Mail::fake()` liefert einen `MailFake`, dessen eigene Methoden die
Assertions sind. Das ist praktisch, wenn der Fake selbst die
Assertions durchführt - Sie haben ihn bereits an eine lokale
Variable gebunden -, aber es ist der einzige Fake in dieser Form:

```rust,ignore
let fake = Mail::fake();
Mail::to("alice@example.org")
    .send(WelcomeEmail { name: "Alice".into() })
    .await?;
fake.assert_sent_count(1);
fake.assert_sent(|m| m.has_to("alice@example.org"));
```

### Guard plus freie Funktionen (Notify, Queue, Bus, Events)

Der Guard ist ein Nichts-tuendes Token, dessen einzige Aufgabe es
ist, den Fake installiert zu halten; die Assertions leben in einem
`testing`-Submodul neben den Internals des Fakes. Importieren Sie,
was Sie brauchen:

```rust,ignore
use suprnova::queue::testing::{install_fake, assert_pushed, pushed};

let _guard = install_fake();
schedule_welcome_email(user_id).await?;
assert_pushed::<WelcomeJob>(|j| j.user_id == user_id);
```

Das ist die häufigste Form, weil sie sich sauber über Typen hinweg
generalisiert - jede Assertion ist generisch über `J: Job` /
`C: Command` / `E: Event`, statt in einen Guard-Typ eingebacken zu
sein. Der Trade-off ist ein zusätzlicher Import.

### Scope-mit-Closure (HTTP)

`Http::fake` ist der Außenseiter. Ausgehendes HTTP läuft auf welcher
Tokio-Task auch immer gerade lebt, daher lebt der Fake-Zustand in
einem `tokio::task_local!`. Sie können ihn nicht einmal installieren
und laufen lassen - Sie müssen den Körper einwickeln, der den
Client aufruft:

```rust,ignore
use suprnova::{Http, fake_response, assert_sent};

Http::fake(|| async {
    fake_response("POST", "/api/users", 201, serde_json::json!({"id": 1}));

    let resp = Http::post("https://example.com/api/users")
        .json(&serde_json::json!({"name": "Ada"}))
        .send()
        .await?;

    assert_eq!(resp.status(), 201);
    assert_sent(|r| r.method == "POST" && r.url.contains("/api/users"));
})
.await;
```

Der Lohn: Jeder andere Fake hält einen prozessweiten
Serialisierungs-Mutex, sodass parallele Tests einer nach dem anderen
laufen, aber `Http::fake` ist echt gleichzeitig - jeder Test bekommt
seinen eigenen task-lokalen Recorder, und sie kollidieren nie.

### Storages Extension-Trait

`Storage::fake()` liefert einen Guard *und* eine
Standard-In-Memory-Disk, aber seine Assertions hängen über das
`DiskAssertExt`-Extension-Trait an der Disk selbst:

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

let _guard = Storage::fake();
let disk = Storage::disk("default")?;

disk.put("invoices/42.pdf", b"...").await?;
disk.assert_exists("invoices/42.pdf").await;
disk.assert_count("invoices/", 1, false).await;
```

Das Extension-Trait ist über
`#[cfg(any(test, feature = "testing"))]` gegated, sodass
Produktionscode nicht versehentlich `disk.assert_exists(…)` aufrufen
kann.

## Parallelsicherheit, in einem Absatz

Sechs der sieben Fakes bewachen ein prozessglobales Static. Jeder
Guard erwirbt bei der Konstruktion einen dedizierten
`FAKE_SERIAL`-`std::sync::Mutex` und hält ihn bis zum Drop. Der
Effekt ist, dass zwei beliebige `#[tokio::test]`s, die denselben
Fake installieren, unter einem Prozess serialisiert laufen - keine
`#[serial]` von der
[serial_test](https://crates.io/crates/serial_test)-Crate nötig.
**Mail ist die Ausnahme**: Der `MailFake`-Guard tauscht den globalen
`TRANSPORT` aus, ohne einen Mutex zu erwerben, sodass sich
gleichzeitige `Mail::fake()`-Tests gegenseitig zerstören *würden*.
Markieren Sie sie mit `#[serial]`. **`Http::fake` ist ebenfalls eine
Ausnahme**: Es ist task-lokal, nicht prozessglobal, sodass Tests
echt parallel laufen und nie `#[serial]` brauchen.

Verschachteln Sie echten Dispatch mit Fake-Dispatch für dieselbe
Oberfläche innerhalb einer Test-Binary, erwirbt der echte Pfad den
Mutex nicht, sodass er mit einem parallelen gefakten Test in ein
Race laufen kann. Markieren Sie die Tests mit echtem Dispatch in
diesem Fall mit `#[serial]` - die Kapitel-eigene Dokumentation weist
darauf hin, wo es zutrifft (siehe [Command-Bus](bus.md) für das
kanonische Beispiel).

## Mail - `Mail::fake()`

```rust,ignore
use serial_test::serial;
use suprnova::mail::{Mail, Address};

#[tokio::test]
#[serial]
async fn welcome_email_is_sent() {
    let fake = Mail::fake();

    register_user("alice@example.org").await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.has_to("alice@example.org"));
    fake.assert_sent(|m| m.subject.starts_with("Welcome"));
    fake.assert_not_sent_to("eve@example.org");
}
```

| Assertion                                  | Prüft…                                              |
|--------------------------------------------|-----------------------------------------------------|
| `fake.assert_sent(\|m\| pred)`             | mindestens eine erfasste Nachricht passt            |
| `fake.assert_sent_to("…")`                 | mindestens eine erfasste Nachricht wurde an diese E-Mail geroutet |
| `fake.assert_not_sent(\|m\| pred)`         | keine erfasste Nachricht passt                      |
| `fake.assert_not_sent_to("…")`             | keine erfasste Nachricht ging an diese E-Mail       |
| `fake.assert_sent_count(n)`                | genau `n` erfasste Nachrichten                      |
| `fake.assert_nothing_sent()`               | nichts wurde erfasst                                |
| `fake.assert_queued("MailableName")`       | mindestens ein eingereihtes Mailable dieses Namens  |
| `fake.assert_queued_with(name, \|q\| …)`   | ein eingereihtes Mailable passt auf das Prädikat    |
| `fake.assert_queued_to("…")`               | ein eingereihtes Mailable wurde an diese E-Mail geroutet |
| `fake.assert_not_queued("MailableName")`   | kein eingereihtes Mailable dieses Namens            |
| `fake.assert_queued_count(n)`              | genau `n` eingereihte Mailables                     |
| `fake.assert_nothing_queued()`             | nichts wurde eingereiht                             |
| `fake.assert_outgoing_count(n)`            | gesendet + eingereiht ergibt insgesamt `n`          |
| `fake.assert_nothing_outgoing()`           | nichts wurde gesendet und nichts wurde eingereiht   |

`fake.captured()`, `fake.queued()`, `fake.sent(pred)`,
`fake.sent_to(…)`, `fake.queued_named(…)` und `fake.queued_to(…)`
liefern die passenden Daten zurück, damit Sie eigene Assertions
bauen können. Siehe [Mail](mail.md) für die vollständige
Oberfläche, einschließlich wie `Mail::queue` in den Fake gespiegelt
wird, selbst wenn `Queue::fake` nicht installiert ist.

## Benachrichtigungen - `Notify::fake()`

```rust,ignore
use suprnova::notifications::{Notify, testing};

#[tokio::test]
async fn order_shipped_notifies_customer() {
    let _guard = Notify::fake();

    ship_order(order_id).await.unwrap();

    testing::assert_sent_to("alice@example.org", "OrderShipped");
    testing::assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    testing::assert_sent_times("OrderShipped", 1);
}
```

| Assertion                                            | Prüft…                                             |
|------------------------------------------------------|-----------------------------------------------------|
| `assert_sent(\|r\| pred)`                            | mindestens eine dispatchte Benachrichtigung passt   |
| `assert_sent_to(route, "Name")`                      | die benannte Benachrichtigung ging an diese Pro-Kanal-Route |
| `assert_sent_to_on(route, channel, "Name")`          | auf diesem Kanal an diese Route dispatcht           |
| `assert_sent_named("Name")`                          | die benannte Benachrichtigung wurde auf irgendeinem Kanal dispatcht |
| `assert_sent_times("Name", n)`                       | genau `n` der benannten Benachrichtigung            |
| `assert_nothing_sent()`                              | keine Benachrichtigungen dispatcht                  |
| `assert_count(n)`                                    | genau `n` insgesamt über alle Typen und Kanäle      |
| `assert_nothing_sent_to(route)`                      | nichts an diese Route dispatcht                     |

`testing::recorded()` liefert jeden `FakeRecord` (Name der
Benachrichtigung, Kanal, Route, JSON-Daten) für feingranularere
Assertions. Empfänger von Benachrichtigungen werden über den
Pro-Kanal-Wert `route_for` geschlüsselt, `assert_sent_to` nimmt also
den Route-String entgegen (eine E-Mail-Adresse für `"mail"`, die ID
als String für `"database"`, …) - siehe
[Benachrichtigungen](notifications.md) für das Routing-Modell.

## Warteschlange - `queue::testing::install_fake()`

```rust,ignore
use suprnova::Queue;
use suprnova::queue::testing::{
    install_fake, assert_pushed, assert_pushed_later, pushed,
};

#[tokio::test]
async fn order_placed_enqueues_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_pushed::<ChargeCustomerJob>(|j| j.order_id == 42);
}
```

| Assertion                                      | Prüft…                                                          |
|------------------------------------------------|------------------------------------------------------------------|
| `assert_pushed::<J>(\|j\| pred)`               | mindestens ein Push von `J` passt                                |
| `assert_pushed_later::<J>(\|j, at\| pred)`     | ein Push von `J` wurde für `at` geplant (verzögerter Dispatch)   |

Die Datenseite liefert die typisierten Jobs selbst zurück:

- `pushed::<J>() -> Vec<J>` - jeder erfasste Push von `J`
- `pushed_with_available_at::<J>() -> Vec<(J, DateTime<Utc>)>` -
  dasselbe, mit dem geplanten Zeitstempel jedes Jobs

Jedes `Queue::push`, `Queue::push_later`, `Queue::later`,
`Queue::push_unique*` und die Chain-/Batch-Dispatcher laufen alle in
denselben Recorder. Siehe [Warteschlange](queues.md) für die
`push_unique`-Semantik unter dem Fake (er erfasst immer und meldet
"pushed").

## Bus - `bus::testing::install_fake()`

```rust,ignore
use suprnova::Bus;
use suprnova::bus::testing::{
    install_fake, assert_dispatched, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched,
};

#[tokio::test]
async fn order_placed_dispatches_charge() {
    let _guard = install_fake();

    place_order(42).await.unwrap();

    assert_dispatched::<ChargeCustomer>(|c| c.customer_id == 42);
    assert_dispatched_times::<ChargeCustomer>(|_| true, 1);
    assert_not_dispatched::<RefundCustomer>(|_| true);
}
```

| Assertion                                           | Prüft…                                                        |
|-----------------------------------------------------|-----------------------------------------------------------------|
| `assert_dispatched::<C>(\|c\| pred)`                | mindestens ein dispatchtes Command vom Typ `C` passt             |
| `assert_not_dispatched::<C>(\|c\| pred)`            | kein dispatchtes Command vom Typ `C` passt                       |
| `assert_dispatched_times::<C>(\|c\| pred, n)`       | genau `n` dispatchte Commands vom Typ `C` passen                 |
| `assert_nothing_dispatched()`                       | null Commands irgendeines Typs unter dem aktiven Fake dispatcht |

Unter dem Fake liefert `Bus::dispatch` `Ok(Dispatched::Captured)`
zurück, statt den Handler auszuführen. Echte Fehlschläge -
Encode-/Decode-Fehler, kein registrierter Handler, bevor der Fake
installiert wurde - treten weiterhin als `Err(_)` auf. Siehe
[Command-Bus](bus.md).

## Ereignisse - `EventFacade::fake()`

```rust,ignore
use suprnova::EventFacade;
use suprnova::events::{
    assert_dispatched, assert_dispatched_once, assert_dispatched_times,
    assert_not_dispatched, assert_nothing_dispatched, dispatched,
    dispatched_count, dispatched_events, has_dispatched,
};

#[tokio::test]
async fn registration_dispatches_welcome_event() {
    let _guard = EventFacade::fake();

    register_user("ada@example.com").await.unwrap();

    assert_dispatched_once::<UserRegistered>();
    assert_dispatched::<UserRegistered>(|e| e.email == "ada@example.com");
}
```

| Assertion                              | Prüft…                                             |
|----------------------------------------|-----------------------------------------------------|
| `assert_dispatched::<E>(\|e\| pred)`   | mindestens ein dispatchtes `E` passt                |
| `assert_dispatched_once::<E>()`        | genau ein `E` wurde dispatcht                       |
| `assert_dispatched_times::<E>(n)`      | genau `n` von `E` wurden dispatcht                  |
| `assert_not_dispatched::<E>(\|e\| ..)` | kein passendes `E` wurde dispatcht                  |
| `assert_nothing_dispatched()`          | keine Events irgendeines Typs dispatcht             |
| `assert_listening::<E, L>()`           | Listener `L` ist für `E` registriert                |
| `has_dispatched::<E>()`                | `bool`: irgendein `E` erfasst                       |
| `dispatched::<E>(\|e\| pred)`          | `Vec<E>`-Klone passender Events                     |
| `dispatched_count::<E>(\|e\| pred)`    | Anzahl passender Events                             |
| `dispatched_events()`                  | `HashMap<&'static str, usize>` aller Dispatches     |

Zwei Varianten schränken ein, was gefaked wird:

```rust,ignore
// Nur diese faken - alles andere dispatcht normal.
let _guard = EventFacade::fake_only(&["UserRegistered", "UserDeleted"]);

// Jedes Event faken, AUSSER diesen.
let _guard = EventFacade::fake_except(&["TelemetryEvent"]);
```

Und eine Variante unterdrückt, ohne aufzuzeichnen:

```rust,ignore
EventFacade::muted(async {
    // Keine Listener feuern, keine Events werden aufgezeichnet.
    run_bulk_import().await;
})
.await;
```

`muted` erwirbt den Mutex NICHT, sodass gemutete Scopes parallel
laufen können. Siehe [Ereignisse](events.md) für die vollständige
Maschinerie, einschließlich `assert_listening` (das nur
Listener-Registrierungen beobachtet, die *innerhalb* des Scopes des
Fakes passieren).

## Storage - `Storage::fake()`

```rust,ignore
use suprnova::{Storage, DiskExt};
use suprnova::filesystem::testing::DiskAssertExt;

#[tokio::test]
async fn invoice_upload_persists() {
    let _guard = Storage::fake();
    let disk = Storage::disk("default").unwrap();

    upload_invoice(b"%PDF-1.7 …").await.unwrap();

    disk.assert_exists("invoices/2026/05/30/inv-00042.pdf").await;
    disk.assert_contents("invoices/2026/05/30/inv-00042.pdf", b"%PDF-1.7 …").await;
}
```

Der Guard registriert vorab eine In-Memory-Disk `"default"`, sodass
triviale Tests kein Disk-Setup brauchen. Registrieren Sie
zusätzliche Disks unter eigenen Namen mit
`Storage::register_memory("audit_logs")` innerhalb des Tests, wenn
der zu testende Code auf eine Nicht-Standard-Disk zugreift.

| Assertion                                        | Prüft…                                             |
|--------------------------------------------------|------------------------------------------------------|
| `disk.assert_exists(path).await`                 | der Pfad existiert                                   |
| `disk.assert_contents(path, &expected).await`    | die Datei stimmt byteweise mit `expected` überein    |
| `disk.assert_missing(path).await`                | der Pfad existiert nicht                             |
| `disk.assert_count(dir, n, recursive).await`     | `dir` enthält genau `n` Einträge                     |
| `disk.assert_directory_empty(dir).await`         | `dir` hat keine Einträge (rekursiv)                  |

Alle fünf geraten bei einer Abweichung in Panic, mit dem Disk-Pfad
in der Meldung. Siehe [Dateispeicher](filesystem.md) für die
`Storage`-Facade selbst und die Treiber-Geschichte
(memory / fs / s3 / azblob / gcs).

## HTTP-Client - `Http::fake`

```rust,ignore
use suprnova::{Http, fake_response, assert_sent, assert_not_sent};

#[tokio::test]
async fn payment_webhook_is_acked() {
    Http::fake(|| async {
        fake_response("POST", "/v1/charges", 201, serde_json::json!({
            "id": "ch_42",
            "status": "succeeded",
        }));

        let result = charge_card(amount_cents).await;

        assert!(result.is_ok());
        assert_sent(|r| r.method == "POST" && r.url.contains("/v1/charges"));
        assert_not_sent(|r| r.method == "DELETE");
    })
    .await;
}
```

`fake_response(method, url_substring, status, body)` reiht eine
vorgefertigte Response ein. Die Methode `"*"` passt auf jede
Methode. Jeder vorgefertigte Eintrag wird bei der ersten passenden
Anfrage verbraucht; nachfolgende passende Anfragen fallen entweder
zum nächsten vorgefertigten Eintrag durch oder liefern ein leeres
`200 {}`.

| Helfer                                       | Zweck                                                     |
|----------------------------------------------|-----------------------------------------------------------|
| `Http::fake(\|\| async { … }).await`         | installiert den task-lokalen Fake-Scope                   |
| `fake_response(method, url_substring, …)`    | reiht eine vorgefertigte Response ein                     |
| `assert_sent(\|r\| pred)`                    | assertiert, dass mindestens eine aufgezeichnete Anfrage passt |
| `assert_not_sent(\|r\| pred)`                | assertiert, dass keine aufgezeichnete Anfrage passt        |

### Gespawnte Tasks erben den Fake nicht standardmäßig

`tokio::spawn` trägt Task-Locals nicht in das gespawnte Future
hinein, sodass Arbeit, die dem Parent-Task entkommt, auch dem Fake
entkommt. Zwei Werkzeuge behandeln das:

```rust,ignore
// Zusätzliche Absicherung: jeden ungefakten ausgehenden Aufruf in einen harten Fehler verwandeln.
let _guard = suprnova::FailOnRealCallsGuard::install();

Http::fake(|| async {
    fake_response("GET", "/child", 204, serde_json::json!({}));

    // Explizites Opt-in: Dieses Kind sieht den Fake-Zustand des Parents.
    let handle = Http::spawn_with_fake_inheritance(async {
        Http::get("https://child.test").send().await
    });

    let response = handle.await.unwrap().unwrap();
    assert_eq!(response.status(), 204);
})
.await;
```

`FailOnRealCallsGuard` ist RAII - installieren Sie ihn am Anfang
eines Tests, und jeder ausgehende Aufruf, der keinen aktiven Fake
trifft, schlägt mit einem Fehler fehl, statt das Netzwerk zu
berühren. `Http::spawn_with_fake_inheritance` ist das explizite
Opt-in für Tasks, die den Fake-Zustand des Parents teilen sollen.
Siehe [HTTP-Client](http-client.md) für die vollständige
Besprechung.

## Broadcasting

WebSocket-Broadcasting hat eine parallele Test-Fixture, aber ihre
Form unterscheidet sich genug, dass sie in ihrem eigenen Kapitel
lebt: `RecordingBroadcastHub` ist ein echter `BroadcastHub`, der
jedes veröffentlichte Envelope aufzeichnet, während er weiterhin an
lebende Subscriber zustellt. Binden Sie ihn an Stelle von
`InMemoryBroadcastHub` und rufen Sie `hub.broadcasts()` /
`hub.assert_broadcast(channel, event)` auf. Siehe
[Broadcasting](broadcasting.md) für das Broadcasting-Modell und die
Verwendung des Recording-Hub.

## Wo jeder Fake lebt

| Oberfläche    | Quelle                                | Facade-Re-Export                             |
|---------------|----------------------------------------|----------------------------------------------|
| Mail          | `framework/src/mail/mod.rs`           | `suprnova::{Mail, MailFake}`                 |
| Benachrichtigungen | `framework/src/notifications/testing.rs` | `suprnova::{Notify, NotifyFakeGuard}` + `suprnova::notifications::testing::*` |
| Warteschlange | `framework/src/queue/testing.rs`      | `suprnova::queue::testing::*`                |
| Bus           | `framework/src/bus/testing.rs`        | `suprnova::bus::testing::*`                  |
| Ereignisse    | `framework/src/events/testing.rs`     | `suprnova::{EventFacade, EventFakeGuard}` + `suprnova::events::*` |
| Storage       | `framework/src/filesystem/testing.rs` | `suprnova::{Storage, DiskExt}` + `suprnova::filesystem::testing::DiskAssertExt` |
| HTTP          | `framework/src/http_client/fake.rs`   | `suprnova::{Http, fake_response, assert_sent, assert_not_sent, FailOnRealCallsGuard, RecordedRequest}` |

Die Module `testing` und `fake` sind hinter einem Cargo-Feature
namens `testing` gegated. Es ist im Standard-Feature-Set enthalten,
jeder Test, der von `suprnova` abhängt, bekommt die Helfer also
kostenlos. Die Hooks selbst sind `#[doc(hidden)]`, wo sie
versehentlich aus Anwendungscode erreicht werden könnten; die
tragende Absicherung ist die `APP_KEY`-Validierung von
`Server::from_config`, die bei jedem Boot läuft, unabhängig davon,
welche Test-Helfer eincompiliert sind. Siehe [Testen](testing.md)
für die Produktions-Build-Geschichte.

## Warum diese Formen, nicht eine Form

Eine einzige einheitliche Form wäre auf der Seite ordentlicher und
in der Praxis schlechter. Jede Form existiert, weil der zugrunde
liegende Zustand unterschiedliche Nebenläufigkeits-Semantik hat:

- **Mails** Transport ist ein globaler `Arc<dyn MailTransport>`, den
  der Guard austauscht. Methoden-Assertions auf dem zurückgegebenen
  Guard binden das Assertieren an die konkrete Installation, was es
  unmöglich macht, Assertions aufzurufen, wenn kein Fake aktiv ist.
- **Notify / Queue / Bus / Events** assertieren auf heterogenen
  typisierten Payloads - jede Assertion ist generisch über den
  Event-/Job-/Command-Typ. Freie Funktionen in einem
  `testing`-Modul komponieren mit Typparametern sauberer als ein
  handgeschriebenes Methoden-Set auf einem Guard.
- **Storage**-Assertions sind pro Disk, nicht pro Fake - derselbe
  `disk.assert_exists(…)` funktioniert gegen eine gefakte
  Memory-Disk oder eine echte `s3`-Disk in einer
  Integrations-Suite. Sie über ein Extension-Trait an die Disk zu
  hängen, erhält diese Symmetrie.
- **HTTP** muss Tasks folgen, nicht dem Aufruf-Stack. `Http::fake`
  ist der einzige Fake, dessen Scope sich nicht als Guard ausdrücken
  lässt - die Spawn-Semantik erzwingt eine Closure.

Wenn Sie jemals nach einem Helfer greifen, den es nicht gibt, lesen
Sie das betreffende Kapitel; die öffentliche Test-Oberfläche ist pro
Subsystem erschöpfend dokumentiert.

## Nächste Schritte

- [Testen](testing.md) - das `#[suprnova_test]`-Makro,
  `TestDatabase`, `expect!` und `TestContainer::fake`
- [HTTP-Tests](http-tests.md) - `handle_request` direkt treiben,
  ohne einen Socket zu öffnen
- [Datenbank-Tests](database-testing.md) - die Geschichte der
  In-Memory-Datenbank pro Test
- [Service Container](container.md) - `TestContainer::fake` zum
  Austauschen injizierter Services
