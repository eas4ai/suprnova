# Benachrichtigungen

Eine Benachrichtigung ist eine kurze Nachricht, die ein Nutzer (oder
"jeder mit einer E-Mail-Adresse") über einen oder mehrere Kanäle
empfangen soll - Mail, In-App-Posteingang, Browser-Push,
Echtzeit-WebSocket - von einer einzigen Aufrufstelle aus. Sie
schreiben `Notify::send(&user, &OrderShipped { … })`; der Dispatcher
fächert diese eine Benachrichtigung über jeden Kanal auf, den die
Benachrichtigung deklariert hat, und adressiert jeden über den
Empfänger.

Verwenden Sie Benachrichtigungen, wenn das *Was* (eine Bestellung
wurde versandt, eine Rechnung wurde bezahlt) für Ihren Code
interessanter ist als das *Wie* (welcher Transport sie letztlich
zugestellt hat). Für rohen Transport-Zugriff - einen eigenen
Mail-Body zusammenstellen, auf einem bestimmten Broadcast-Kanal
veröffentlichen, einen einmaligen Web Push senden - gehen Sie direkt
über [Mail](mail.md), [Broadcasting](broadcasting.md) oder
[Web Push](web-push.md).

## Schnellstart

```rust
use serde::{Deserialize, Serialize};
use suprnova::FrameworkError;
use suprnova::NotificationMailable;          // Derive-Makro
use suprnova::notifications::channels::mail::MailRendering;
use suprnova::{Notifiable, Notification, Notify};

#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Order shipped - tracking {{ tracking }}",
    html    = "<p>Your order is on its way.</p><p>Tracking: <code>{{ tracking }}</code></p>",
    text    = "Tracking: {{ tracking }}",
    from    = "orders@example.com",
    from_name = "Acme Orders",
)]
pub struct OrderShipped {
    pub tracking: String,
}

impl Notification for OrderShipped {
    fn notification_name() -> &'static str { "OrderShipped" }
    fn channels(&self) -> Vec<&'static str> { vec!["mail", "database"] }
    fn data(&self) -> serde_json::Value {
        serde_json::json!({ "tracking": self.tracking })
    }
}

struct User { id: i64, email: String }
impl Notifiable for User {
    fn route_for(&self, channel: &str) -> Option<String> {
        match channel {
            "mail"     => Some(self.email.clone()),
            "database" => Some(self.id.to_string()),
            _          => None,
        }
    }
}

async fn ship(user: &User, tracking: String) -> Result<(), FrameworkError> {
    Notify::send(user, &OrderShipped { tracking }).await
}
```

`Notify::send` dispatcht in einem Aufruf sowohl an den Mail-Kanal als
auch an den Datenbank-Kanal. Der Empfänger lehnt einen Kanal ab,
indem er `None` von `route_for` zurückgibt - nützlich für "nur
E-Mail"- oder "nur Push"-Nutzer.

## Die drei Traits

| Trait | Was er repräsentiert | Implementiert von |
|---|---|---|
| `Notification` | Eine typisierte Nachricht + die Kanäle, an die sie dispatcht | Ihre Benachrichtigungs-Strukturen |
| `Notifiable` | Ein Empfänger - legt ein `route_for` pro Kanal frei | Ihr `User`, `Order`, alles Adressierbare |
| `Channel` | Ein Transport - weiß, wie an eine Route zugestellt wird | Eingebaut: `MailChannel`, `DatabaseChannel`, `BroadcastChannel`, `WebPushChannel` |

### `Notifiable`

```rust
pub trait Notifiable: Send + Sync {
    fn route_for(&self, channel: &str) -> Option<String>;
}
```

Der Empfänger besitzt die Adressierung pro Kanal. `route_for("mail")`
gibt die E-Mail-Adresse zurück; `route_for("database")` gibt die
Entity-ID als Zeichenkette zurück; `route_for("webpush")` gibt ein
serialisiertes `SubscriptionInfo`-JSON zurück; `route_for("broadcast")`
gibt den Namen des Broadcast-Kanals zurück. Geben Sie `None` zurück,
um einen Kanal für diesen Empfänger zu überspringen.

### `Notification`

```rust
pub trait Notification: Serialize + DeserializeOwned + Send + Sync + 'static {
    fn notification_name() -> &'static str where Self: Sized;
    fn channels(&self) -> Vec<&'static str>;
    fn data(&self) -> serde_json::Value;

    fn should_send(&self, _channel: &str) -> bool { true }
    fn after_sending(&self, _channel: &str) -> Result<(), FrameworkError> { Ok(()) }
}
```

| Methode | Zweck |
|---|---|
| `notification_name()` | Stabiler Identifier, den der Datenbank-Kanal persistiert, der als Envelope-Schlüssel der Queue verwendet wird, und der Lookup-Schlüssel für die Mail-Renderer-Registry ist. |
| `channels(&self)` | Kanalnamen, an die diese Benachrichtigung dispatcht. Reihenfolge ist Iterationsreihenfolge. |
| `data(&self)` | JSON-serialisierbarer Payload, den Kanäle zustellen / persistieren. Typischerweise `serde_json::to_value(self)` der Teilmenge von Feldern, die die Kanäle brauchen. |
| `should_send(&self, channel)` | Veto pro Kanal, konsultiert auf sowohl dem synchronen als auch dem Queued-Pfad. Gibt es `false` zurück, wird dieser Kanal für diesen Dispatch übersprungen. Default: immer senden. |
| `after_sending(&self, channel)` | Post-Success-Hook, aufgerufen einmal pro Kanal, der abgeschlossen hat, auf sowohl dem synchronen als auch dem Queued-Pfad. Gibt er `Err` zurück, propagiert das genauso wie ein Kanal-Fehler. Default: No-op. |

`should_send` und `after_sending` werden auf **beiden** Pfaden
respektiert. `Notify::send` konsultiert sie im Dispatcher;
`Notify::queue` prüft `should_send`, bevor jeder Pro-Kanal-Job
eingereiht wird, und der Worker prüft `should_send` erneut vor der
Zustellung (der Zustand kann sich zwischen Einreihen und Ausführen
ändern) und führt `after_sending` nach einem erfolgreichen Send aus.
Die drei Lifecycle-*Events* (`NotificationSending` / `NotificationSent`
/ `NotificationFailed`) feuern weiterhin nur auf dem synchronen Pfad.

## Kanäle

### Mail

Der Mail-Kanal stellt über den gebundenen Mail-Transport zu (siehe
[Mail](mail.md)). Eine Benachrichtigung nimmt daran teil, indem sie
`NotificationMailable` implementiert:

```rust
pub trait NotificationMailable: Notification {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError>;
}
```

`MailRendering` ist die Rendering-Envelope - `subject` (erforderlich),
`html` und/oder `text` (mindestens eines erforderlich), optional
`from`, `cc`, `bcc`, `reply_to` und `attachments`. Der Mail-Kanal
setzt eine ausgehende Nachricht aus diesem Rendering plus dem
`route_for("mail")` des Empfängers zusammen, wendet die
konfigurierten Sender-Defaults an (`Mail::always_from(...)`,
`always_to(...)` usw.) und dispatcht über `Mail::current_transport`.

Gibt der Renderer ein Rendering ohne `html` und ohne `text` zurück,
schlägt die Zustellung fail-fast fehl - eine leere
Benachrichtigungs-Mail wird nie still versendet.

#### `#[derive(NotificationMailable)]`

Das Derive kollabiert das Pro-Notification-`to_mail`-`impl` in ein
einziges `#[mail(...)]`-Attribut. Templates verwenden
[Tera](https://keats.github.io/tera/); die serialisierten Felder von
`self` sind der Context.

```rust
#[derive(Serialize, Deserialize, NotificationMailable)]
#[mail(
    subject = "Welcome {{ name }}",
    html_template = "templates/welcome.html",
    text_template = "templates/welcome.txt",
    from = "hello@example.com",
    from_name = "Acme",
    cc = "ops@example.com, support@example.com",
)]
pub struct Welcome { pub name: String }
```

Unterstützte Keys:

| Key | Erforderlich? | Zweck |
|---|---|---|
| `subject` | ja | Tera-Template - gerendert mit `self` als Context. |
| `html` | dagger | Inline-HTML-Body-Tera-Template. |
| `html_template` | dagger | Pfad zu einem HTML-Body-Tera-Template (eingebettet über `include_str!`). |
| `text` | dagger | Inline-Plain-Text-Body-Tera-Template. |
| `text_template` | dagger | Pfad zu einem Plain-Text-Body-Tera-Template (eingebettet über `include_str!`). |
| `from` | nein | Sender-E-Mail - überschreibt den Default `noreply@localhost`. |
| `from_name` | nein | Anzeigename. Erfordert `from`. |
| `cc` | nein | Kommagetrennte CC-Liste. Whitespace und nachgestellte Kommas werden ignoriert. |
| `bcc` | nein | Kommagetrennte BCC-Liste. |
| `reply_to` | nein | Kommagetrennte Reply-To-Liste. |

(dagger) Mindestens eine Body-Variante muss vorhanden sein. `html`
und `html_template` schließen sich gegenseitig aus; dasselbe für
`text` und `text_template`.

Jede Invariante wird zur Compile-Zeit erzwungen - fehlendes
`subject`, leerer Body, widersprüchliche Varianten, `from_name` ohne
`from` oder unbekannte Keys lassen den Build fehlschlagen statt den
Dispatch.

Für Attachments (binäre Payloads) oder dynamische Empfänger pro
Instanz implementieren Sie `NotificationMailable` handgerollt und
bauen Sie das `MailRendering` direkt.

### Datenbank

Der Datenbank-Kanal persistiert jede Benachrichtigung als eine Zeile
in der Tabelle `notifications`:

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, NotificationDispatcher};

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")));
```

Das zweite Argument ist das polymorphe Typ-Tag des Empfängers (was
Sie in `notifiable_type` speichern, damit Sie Posteingangs-Zeilen
später zurückabfragen können). Das `route_for("database")` des
Empfängers wird zur `notifiable_id`. Die Migration wird mit dem
Framework mitgeliefert
(`framework/migrations/20260516_create_notifications_table.sql`);
führen Sie `suprnova migrate` aus, und die Tabelle erscheint.

#### Den Posteingang lesen

Die Lesezugriffs-Helfer leben in `suprnova::notifications` als freie
Funktionen über `(notifiable_type, notifiable_id)`:

```rust
use suprnova::notifications::{
    all_for, unread_for, read_for,
    mark_as_read, mark_as_unread, mark_all_as_read,
    delete_for, StoredNotification,
};

let unread: Vec<StoredNotification> = unread_for(&db, "users", "42").await?;
let count = mark_all_as_read(&db, "users", "42").await?;
let removed = delete_for(&db, "users", "42").await?;
```

`StoredNotification` trägt `id`, `type_name` (das
`Notification::notification_name`), `notifiable_type`,
`notifiable_id`, das dekodierte JSON `data`, `read_at`, `created_at`,
`updated_at`. `mark_as_read` / `mark_as_unread` sind idempotent
(entsprechend Laravels Vertrag).

### Web Push

Der Web-Push-Kanal verschlüsselt den Payload und POSTet ihn über den
VAPID-signierenden Client des Frameworks an den gespeicherten
Endpunkt des Browser-Push-Abonnements:

```rust
use std::sync::Arc;
use suprnova::WebPushChannel;
use suprnova::web_push::{VapidKey, WebPushClient};

let client = WebPushClient::new(
    VapidKey::from_pem(b"-----BEGIN PRIVATE KEY-----\n…")?,
    "mailto:ops@example.com",
)?;
let push_channel = WebPushChannel::new(Arc::new(client), 86_400 /* TTL Sekunden */);
```

Das `route_for("webpush")` des Empfängers gibt ein serialisiertes
`SubscriptionInfo`-JSON zurück (dieselbe Form, die der Browser über
`PushSubscription.toJSON()` zurückgibt - speichern Sie es wortgetreu,
geben Sie es unverändert zurück). Die TTL wird an den Push-Service
weitergereicht.

Wenn der Push-Service dem Kanal mitteilt, dass ein Abonnement weg ist
(HTTP 404/410), protokolliert der Kanal ein strukturiertes WARN und
gibt Erfolg zurück - die Benachrichtigung hat einen Endzustand
erreicht, ohne Empfänger, gegen den erneut versucht werden könnte.
Betreiber sehen das Log und entfernen das tote Abonnement; die
Zustellung schlägt nicht fehl.

Siehe [Web Push](web-push.md) für den vollständigen Client.

### Broadcast

Der Broadcast-Kanal veröffentlicht jede Benachrichtigung im
`BroadcastHub` der Anwendung, sodass WebSocket-Abonnenten sie in
Echtzeit empfangen. Das `route_for("broadcast")` des Empfängers ist
der Kanalname, der Benachrichtigungstyp ist das Event, und `data()`
ist der Payload:

```rust
use std::sync::Arc;
use suprnova::BroadcastChannel;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;

// Beim Boot - den Hub binden, bevor irgendein Broadcast-Dispatch
// passiert.
App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

let dispatcher = suprnova::NotificationDispatcher::new()
    .register_channel(Arc::new(BroadcastChannel::new()));
```

Der Kanal löst den Hub zur Zustellungszeit aus dem Container auf. Ist
kein `BroadcastHub` gebunden, wenn eine Benachrichtigung `"broadcast"`
deklariert, gibt der Kanal einen Fehler zurück - eine
fehlkonfigurierte Anwendung macht das Problem sichtbar, statt die
Nachricht still fallenzulassen. Auf einem Kanal mit null aktiven
Abonnenten zu veröffentlichen ist kein Fehler.

Siehe [Broadcasting](broadcasting.md) für Hub-Setup und
WebSocket-Verdrahtung.

## On-Demand-Benachrichtigungen

Manchmal wollen Sie *jemanden benachrichtigen, der nicht in Ihrer
Datenbank steht* - ein einmaliger Ops-Alarm an eine E-Mail-Adresse,
ein Webhook-Empfänger, ein Broadcast-Kanal, den kein Nutzer besitzt.
`AnonymousNotifiable` ist der "Nutzer ohne Zeile":

```rust
use suprnova::Notify;

let recipient = Notify::route("mail", "ops@example.com")?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;

// Mehrere Kanäle in einem Builder:
let recipient = Notify::routes([
    ("mail", "ops@example.com"),
    ("broadcast", "ops-channel"),
])?;
Notify::send(&recipient, &IncidentNotification { id: 7 }).await?;
```

`Notify::route("database", …)` und `Notify::routes([...,
("database", …)])` geben `Err` zurück - der Datenbank-Kanal
persistiert ein `(notifiable_type, notifiable_id)`-Paar, das ein
anonymer Empfänger nicht liefern kann.

## Der Dispatcher

`NotificationDispatcher` hält die Kanal-Registry. Bauen Sie ihn
einmal beim Boot und binden Sie ihn global:

```rust
use std::sync::Arc;
use suprnova::{DatabaseChannel, MailChannel, NotificationDispatcher, WebPushChannel};
use suprnova::notifications::set_dispatcher;

let dispatcher = NotificationDispatcher::new()
    .register_channel(Arc::new(MailChannel::new()))
    .register_channel(Arc::new(DatabaseChannel::new(db, "users")))
    .register_channel(Arc::new(WebPushChannel::new(push_client, 86_400)));

set_dispatcher(Arc::new(dispatcher))?;
```

`register_channel` ist last-write-wins auf dem Kanalnamen - zwei
Kanäle namens `"mail"` zu registrieren ersetzt den ersten still. Das
macht Test-Setups ergonomisch.

Eine Benachrichtigung, die einen Kanal deklariert, den der Dispatcher
nicht registriert, protokolliert ein WARN
(`no channel registered; skipping`) und fährt mit dem nächsten Kanal
fort - der Dispatch schlägt bei einem unbekannten Kanalnamen nicht
fehl.

`set_dispatcher` gibt `Result<(), FrameworkError>` zurück, weil die
Dispatcher-Registry hinter einem `RwLock` lebt; der Fehlerpfad greift
nur, wenn die Sperre vergiftet ist (ein vorheriger Writer ist in
Panic geraten). In der Praxis verwendet die Aufrufstelle beim Boot
`?`.

### Lifecycle-Events

Drei Events umgeben jede synchrone Kanal-Zustellung:

| Event | Wann | Verhalten bei Listener-Fehler |
|---|---|---|
| `NotificationSending` | Unmittelbar bevor der Kanal läuft | `Err` eines Listeners **belegt den Kanal für diesen Dispatch mit Veto** |
| `NotificationSent` | Nach einer erfolgreichen Zustellung | Best-effort-Dispatch - Listener-Fehler propagieren nicht |
| `NotificationFailed` | Wenn ein Kanal einen Fehler zurückgegeben hat | Best-effort-Dispatch; der zugrunde liegende Kanal-Fehler propagiert trotzdem gemäß dem First-failure-stops-Vertrag |

Alle drei tragen `(notification, channel, route, data)`. `Failed`
fügt den stringifizierten `error` hinzu. Lauschen Sie mit
`EventFacade::listen::<E, L>` - siehe [Ereignisse](events.md).

Diese Events feuern nur auf dem synchronen `Notify::send`-Pfad. Der
Queued-Worker stellt Kanäle direkt zu, ohne die Events zu dispatchen.

### Telemetrie

`NotificationDispatcher::notify` umschließt den Fan-out in einem
`notification.dispatch`-Tracing-Span:

- `notification` - `Notification::notification_name()`
- `channel_count` - deklarierte Kanalanzahl
- `duration_ms` - Fan-out-Latenz beim Abschluss
- abschließendes Log: `notification dispatched` (info) oder
  `notification dispatch failed` (warn)

Der Mail-Kanal verschachtelt seinen eigenen `mail.send`-Span darin.

### First-failure-stops-Vertrag

`Notify::send` kehrt beim ersten Kanal-Fehler zurück. Kanäle, die
bereits erfolgreich waren, werden nicht zurückgerollt; Kanäle, die
noch nicht gelaufen sind, werden nicht versucht. Derselbe Vertrag
gilt für den Queued-Worker.

Für At-least-once über mehrere Kanäle hinweg dispatchen Sie jeden
Kanal über seinen eigenen `Notify::queue`-Aufruf - die
Idempotenz-Schlüssel der Queue-Envelope schützen bei Wiederholung vor
Doppel-Sends.

## Queued Zustellung

`Notify::send` läuft in-process. `Notify::queue` pusht einen
`SendNotificationJob` auf die [Warteschlange](queues.md) und löst
dabei die Pro-Kanal-Routen des Empfängers vorab auf, sodass der
Worker zur Ausführungszeit kein `Notifiable`-Handle braucht:

```rust
use suprnova::notifications::register_notification_factory;
use suprnova::Notify;

// Beim Boot - einmal pro konkreter Benachrichtigung, die über
// Notify::queue erreichbar ist.
register_notification_factory::<OrderShipped>()?;

// Irgendwo:
Notify::queue(&user, OrderShipped { tracking }).await?;
```

Zum Dispatch-Zeitpunkt führt der Worker Folgendes aus:

1. Schlägt die Benachrichtigungs-Factory über `notification_name` nach
2. Baut die typisierte Benachrichtigung aus dem JSON-Payload wieder auf
3. Iteriert über die zum Zeitpunkt des Einreihens erfassten Kanäle
4. Prüft für jeden `should_send(channel)` erneut (überspringt Kanäle
   mit Veto), schlägt den Kanal auf dem gebundenen Dispatcher nach,
   ruft `deliver(route, &notification)` auf und führt dann
   `after_sending(channel)` aus

Kanäle, die zum Zeitpunkt des Einreihens deklariert wurden, aber
nicht registriert sind, wenn der Worker läuft, protokollieren ein
WARN und werden übersprungen - derselbe Vertrag wie beim synchronen
Pfad. Kanäle ohne vorab aufgelöste Route werden still übersprungen
(der Empfänger hat zum Zeitpunkt des Einreihens `None`
zurückgegeben).

`Notify::queue` evaluiert `should_send` außerdem zum Zeitpunkt des
Einreihens, sodass ein Kanal mit Veto von Anfang an nie eingereiht
wird; die erneute Prüfung des Workers deckt Zustand ab, der sich
zwischen Einreihen und Ausführen ändert. Der Queued-Pfad feuert die
drei Lifecycle-Events (`NotificationSending` / `NotificationSent` /
`NotificationFailed`) **NICHT** - die bleiben synchron-only. Wenn Sie
von den Events abhängen, senden Sie über `Notify::send`.

### Warum Suprnova abweicht

Laravel macht Queued-Benachrichtigungen am Marker-Interface
`ShouldQueue` fest - derselbe Aufruf
`Notification::send($user, $notification)` reiht ein, wenn die
Benachrichtigung `ShouldQueue` implementiert, und sendet inline, wenn
nicht. Das Verhalten hängt von einem Flag auf Typ-Ebene an der Stelle
der Benachrichtigung ab, das von der Aufrufstelle aus unsichtbar ist.

Suprnova macht diese Entscheidung an jedem Aufruf explizit:
`Notify::send` ist immer synchron; `Notify::queue` ist immer queued.
Es gibt keinen versteckten Modus-Schalter. (Das ist auch, warum es
kein `send_now` gibt - `send` ist bereits das synchrone.)

Die Empfängerseite weicht ebenfalls ab. Laravels Trait `Notifiable`
ist ein Mixin, der die Posteingangs-Relation, die
`routeNotificationFor*`-Methoden und den polymorphen Primärschlüssel
mit hineinzieht. Suprnovas `Notifiable` ist absichtlich minimal - nur
`route_for(channel) -> Option<String>` -, weil Rust-Traits sich nicht
per Mixin komponieren. Die zu Laravel äquivalente Leseseite wird als
freie Funktionen über `(notifiable_type, notifiable_id)`
mitgeliefert (`unread_for`, `mark_as_read`, …), sodass gewöhnliche
Strukturen notifiable sein können, ohne eine ORM-Relation zu erben.

## Testen

Zwei Fake-Oberflächen, die unterschiedliche Fragen beantworten.

### `Notify::fake()` - "wurde eine Benachrichtigung dispatcht?"

```rust
use suprnova::Notify;
use suprnova::notifications::{
    assert_count, assert_nothing_sent, assert_sent_named,
    assert_sent_times, assert_sent_to, assert_sent_to_on,
    recorded_notifications,
};

#[tokio::test]
async fn ship_dispatches_order_shipped() {
    let _fake = Notify::fake();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    assert_sent_named("OrderShipped");
    assert_sent_to("alice@example.org", "OrderShipped");
    assert_sent_to_on("alice@example.org", "mail", "OrderShipped");
    assert_sent_times("OrderShipped", 1);
    assert_count(2); // Mail + Datenbank
}
```

Während der Fake-Guard lebt, zeichnen sowohl `Notify::send` als auch
`Notify::queue` den Dispatch auf, statt Kanäle auszuführen oder einen
Job einzureihen - kein Kanal läuft, keine Queue-Zeile wird
geschrieben. Der Fake hält einen prozessweiten
Serialisierungs-Mutex, sodass parallele Tests ihre Aufzeichnungen
nicht verschachteln können; lassen Sie den `_fake`-Guard am Testende
droppen, um den Recorder zu leeren.

Verwenden Sie `recorded_notifications()` für vollen Zugriff auf die
erfassten Daten:

```rust
let records = recorded_notifications();
assert_eq!(records[0].notification, "OrderShipped");
assert_eq!(records[0].channel, "mail");
assert_eq!(records[0].data["tracking"], "1Z…");
```

### `Mail::fake()` + echter `MailChannel` - "hat die Benachrichtigung korrekt *gerendert*?"

`Notify::fake()` unterbricht per Short-Circuit, bevor der Kanal
erreicht wird. Um zu prüfen, dass der Mail-Body tatsächlich so
gerendert hat, wie Sie erwarten, treiben Sie den echten Kanal unter
`Mail::fake()`:

```rust
use serial_test::serial;
use std::sync::Arc;
use suprnova::mail::Mail;
use suprnova::notifications::{set_dispatcher, NotificationDispatcher};
use suprnova::{MailChannel, Notify, register_mail_renderer};

#[tokio::test]
#[serial]
async fn ordershipped_renders_tracking_in_subject() {
    let fake = Mail::fake();
    register_mail_renderer::<OrderShipped>().unwrap();
    set_dispatcher(Arc::new(
        NotificationDispatcher::new()
            .register_channel(Arc::new(MailChannel::new())),
    )).unwrap();

    Notify::send(
        &User { id: 1, email: "alice@example.org".into() },
        &OrderShipped { tracking: "1Z…".into() },
    ).await.unwrap();

    fake.assert_sent_count(1);
    fake.assert_sent(|m| m.subject.contains("1Z…"));
}
```

Tests, die den Dispatcher, den Renderer oder die Transport-Globals
berühren, müssen `#[serial_test::serial]` sein - das sind
prozessglobale Statics.

## Best Practices

### Jede Factory und jeden Renderer beim Boot registrieren

`Notify::queue` baut die Benachrichtigung beim Worker über die
Factory-Registry wieder auf, und `MailChannel` rendert über
`register_mail_renderer`. Registrieren Sie jede queueable / mailable
Benachrichtigung im Voraus:

```rust
// bootstrap.rs
use suprnova::notifications::register_notification_factory;
use suprnova::register_mail_renderer;

pub fn register() -> Result<(), FrameworkError> {
    // Benachrichtigungs-Factories (eine pro Notification, die über
    // Notify::queue erreichbar ist).
    register_notification_factory::<OrderShipped>()?;
    register_notification_factory::<InvoicePaid>()?;

    // Mail-Renderer (einer pro NotificationMailable).
    register_mail_renderer::<OrderShipped>()?;
    register_mail_renderer::<InvoicePaid>()?;
    Ok(())
}
```

Eine nicht registrierte Benachrichtigung auf der Queue taucht zur
Ausführungszeit des Workers als `unknown notification: {name}` auf
und durchläuft den Dead-Letter-Pfad mit Wiederholungen. Ein
`MailChannel`-Dispatch für einen nicht registrierten Renderer taucht
auf demselben Weg als Fehler `register via
suprnova::register_mail_renderer::<N>()` auf.

### Queue für Multi-Kanal-Fan-outs einsetzen

Der synchrone Dispatcher besucht Kanäle in Reihenfolge und kehrt beim
ersten Fehler zurück. Ein Fehlschlag an Kanal Nr. 2 lässt Kanal Nr. 1
committed und die Kanäle Nr. 3+ unversucht. Bevorzugen Sie für jede
Benachrichtigung, die mehr als einen Kanal betrifft, `Notify::queue`,
damit der Worker Wiederholungen mit Backoff handhabt und der Dispatch
einen Prozessabsturz überlebt.

### Kanal-Zustellungen idempotent machen

Worker-Wiederholungen bedeuten, dass derselbe `SendNotificationJob`
mehr als einmal ausführen kann. Die eingebauten Kanäle sind
idempotenz-freundlich: `MailChannel` leitet an Provider weiter, die
typischerweise per Message-ID deduplizieren; `DatabaseChannel` fügt
pro Ausführung eine frische UUID ein (was für eine Audit-Zeile das
richtige Verhalten ist); `WebPushChannel` POSTet an einen Provider,
der Duplikate verschluckt. Eigene Kanäle sollten auf idempotente
Operationen zielen - HTTP-POSTs mit stabilen clientseitigen
Dedupe-Keys, Upserts statt blinder Inserts, keine "einen Zähler
erhöhen"-Seiteneffekte auf dem Zustellungspfad.

### Den Dispatcher an einer Stelle binden

`register_channel` ist last-write-wins, sodass Tests im Setup einen
echten Kanal gegen einen Stub tauschen können. Behalten Sie die
Production-Bindung in `bootstrap.rs`, und lassen Sie Tests ihren
eigenen Dispatcher mit welchen Stubs auch immer sie brauchen bauen.
Rufen Sie `register_channel` nicht lazy innerhalb von
Request-Handlern auf - die globalen Sperr-Writes plus die
last-write-wins-Semantik werden unter gleichzeitiger Last
überraschend.

## Referenz

| Symbol | Pfad |
|---|---|
| `Notifiable`, `Notification`, `Channel`, `DynNotification` | `suprnova::` |
| `Notify` (Facade), `NotifyFakeGuard` | `suprnova::` |
| `NotificationDispatcher`, `NotificationFactory` | `suprnova::` |
| `AnonymousNotifiable` | `suprnova::` |
| `MailChannel`, `MailRendering`, `NotificationMailable` | `suprnova::` |
| `register_mail_renderer::<N>()` | `suprnova::` |
| `DatabaseChannel`, `StoredNotification` | `suprnova::` |
| `WebPushChannel` | `suprnova::` |
| `BroadcastChannel` | `suprnova::` |
| `SendNotificationJob` | `suprnova::` |
| `NotificationSending`, `NotificationSent`, `NotificationFailed` | `suprnova::` |
| `set_dispatcher`, `register_notification_factory` | `suprnova::notifications::` |
| `all_for`, `unread_for`, `read_for`, `mark_as_read`, `mark_as_unread`, `mark_all_as_read`, `delete_for` | `suprnova::notifications::` |
| `assert_sent`, `assert_sent_named`, `assert_sent_times`, `assert_sent_to`, `assert_sent_to_on`, `assert_nothing_sent`, `assert_nothing_sent_to`, `assert_count`, `recorded_notifications` | `suprnova::notifications::` |
| `#[derive(NotificationMailable)]` | `suprnova::` |

## Nächste Schritte

- [Mail](mail.md) - der Transport und die `Mailable`-Oberfläche, auf
  der der Mail-Kanal aufsetzt
- [Broadcasting](broadcasting.md) - der `BroadcastHub`, über den der
  Broadcast-Kanal veröffentlicht
- [Web Push](web-push.md) - VAPID, Verschlüsselung, Speicherung von
  Abonnements
- [Ereignisse](events.md) - auf `NotificationSending` / `Sent` /
  `Failed` lauschen
- [Warteschlange](queues.md) - der Worker, der `Notify::queue`
  antreibt
- [Testen](testing.md) - Fake-Oberflächen und Serial-Test-Muster
