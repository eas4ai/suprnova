# Broadcasting

Broadcasting ist die Server-zu-Client-Benachrichtigungsschicht oben
auf Suprnovas [WebSocket-Primitive](websockets.md). Sie dispatchen
ein `Broadcastable`-Event über `EventFacade`; das Framework fächert
die JSON-Envelope des Events an jeden WebSocket-Abonnenten auf den
Kanälen auf, die das Event nennt. Sie verwalten nie einzelne
Verbindungen - Sie verwalten Kanal-Abonnements, und der Hub erledigt
den Rest.

Der `BroadcastHub` ist der Bus. Der Default `InMemoryBroadcastHub`
läuft vollständig in-process - perfekt für Single-Replica-Deployments
und die Test-Suite. Hinter dem Cargo-Feature `broadcasting-fanout`
routet `SeaStreamerBroadcastHub` dieselben Events durch einen
Stream-Broker (Redis Streams, Kafka, Datei, stdio), sodass ein
Publish in einem Prozess Abonnenten in jedem anderen Prozess
erreicht.

Alles aus dem Kapitel [WebSockets](websockets.md) gilt weiterhin,
darunter Herzschlag-Pings, `max_missed_pings`, `WsConfig`, Middleware
pro Route, Pfadparameter. Broadcasting fügt nur ein Wire-Protokoll
und eine Kanal-Registry oben drauf hinzu.

## Schnellstart

Vier Dateien, und der Browser sieht ein Event.

`src/channels/order_updates.rs`:

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
}
```

`src/events/order_placed.rs`:

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        vec!["order.updates".into()]
    }
}
```

`src/bootstrap.rs`:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry, InMemoryBroadcastHub};
use suprnova::container::App;
use suprnova::events::EventFacade;

pub async fn register() {
    // 1. Den Hub hinter dem Trait binden - Handler lösen ihn
    // einheitlich auf.
    let hub: Arc<dyn BroadcastHub> = Arc::new(InMemoryBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));

    // 2. Jeden Kanal im Voraus registrieren; der WS-Handler löst
    // über den Namen auf.
    let mut registry = ChannelRegistry::new();
    registry.register(OrderUpdates);
    App::singleton(Arc::new(registry));

    // 3. Die Brücke Event → Hub einmal pro Broadcastable-Typ
    // verdrahten.
    EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
}
```

`src/routes.rs` - baut pro Route einen `BroadcastingWsHandler`, indem
der gebootstrappte Hub und die Registry aus dem Container aufgelöst
werden:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{
    BroadcastHub, BroadcastingWsHandler, ChannelRegistry, InMemoryBroadcastHub,
};
use suprnova::container::App;
use suprnova::{routes, ws, AuthMiddleware};

fn broadcasting_handler() -> BroadcastingWsHandler {
    // Container-first; Fallback auf einen frischen In-Process-Hub +
    // leere Registry, damit Unit-Tests, die den Router ohne
    // Bootstrap zusammenbauen, weiterhin funktionieren.
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .unwrap_or_else(|| Arc::new(InMemoryBroadcastHub::new()));
    let registry: Arc<ChannelRegistry> = App::get::<Arc<ChannelRegistry>>()
        .unwrap_or_else(|| Arc::new(ChannelRegistry::new()));
    BroadcastingWsHandler::new(hub, registry)
}

routes! {
    ws!("/ws/broadcast", broadcasting_handler())
        .middleware(AuthMiddleware::new()),
}
```

Verbinden und beobachten:

```bash
wscat -c ws://localhost:3000/ws/broadcast
> {"action":"connected","socket_id":"6f1a3c2e-…"}
> {"action":"subscribe","channel":"order.updates","data":{}}
< {"action":"subscribed","channel":"order.updates"}
```

Dispatchen Sie von jedem Controller, Worker oder geplanten Task aus:

```rust
EventFacade::dispatch(OrderPlaced { order_id: 99, user_id: 42 }).await?;
```

```
< {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}
```

## Kanäle

Ein Kanal ist ein benanntes Abonnement-Ziel. Clients abonnieren nach
Namen; der Hub stellt Events an jeden aktiven Abonnenten dieses
Namens zu. Der Trait `Channel` hat asymmetrische Defaults, die bei
Schreibzugriffen geschlossen und bei Lesezugriffen offen
fehlschlagen - siehe [Warum Suprnova abweicht](#warum-suprnova-abweicht)
weiter unten.

### Öffentliche Kanäle

Der Default. Jeder Client darf abonnieren.

```rust
use async_trait::async_trait;
use suprnova::broadcasting::Channel;

pub struct OrderUpdates;

#[async_trait]
impl Channel for OrderUpdates {
    fn name(&self) -> &'static str { "order.updates" }
    // authorize() defaultet auf true - offen für alle Abonnenten.
}
```

### Private Kanäle

Überschreiben Sie `authorize`, um Abonnements zu gaten. Ein
abgelehntes Subscribe erzeugt einen `error`-Frame mit
`reason: "unauthorized"`; es wird kein `subscribed`-Frame gesendet.

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct PrivateChat;

#[async_trait]
impl Channel for PrivateChat {
    fn name(&self) -> &'static str { "chat.private" }

    async fn authorize(
        &self,
        _req: &Request,
        _params: &ChannelParams,
        data: &Value,
    ) -> bool {
        data["token"].as_str().map(|t| t == "valid").unwrap_or(false)
    }
}

impl PrivateChannel for PrivateChat {}
```

`data` ist, was auch immer der Client im `data`-Feld des
Subscribe-Frames gesendet hat - ein Bearer-Token, ein signierter
Kanal-Bind, alles Anwendungsdefinierte. `Request` ist die
ursprüngliche HTTP-Upgrade-Anfrage (Header und Cookies sind direkt
lesbar). `params` trägt die erfassten Werte aus einem parametrisierten
Namen und ist bei festen Namen leer.

`PrivateChannel` ist ein Marker-Trait. Das Framework prüft nicht zur
Laufzeit darauf - es ist ein Signal auf Typ-Ebene, dass der Kanal
`authorize` überschreibt, und ist für zukünftiges Tooling gedacht
(ein Clippy-Lint, ein Audit-Pass).

### Parametrisierte Kanäle

Betten Sie `{param}`-Segmente in `name()` ein, und eine Registrierung
bedient jedes konkrete Abonnement, das auf das Pattern passt -
dasselbe Modell wie Laravels `Broadcast::channel('orders.{id}', …)`.
Erfasste Werte erreichen jeden Hook als `ChannelParams`-Map.

```rust
use async_trait::async_trait;
use serde_json::Value;
use suprnova::broadcasting::{Channel, ChannelParams, PrivateChannel};
use suprnova::http::Request;

pub struct OrderChannel;

#[async_trait]
impl Channel for OrderChannel {
    fn name(&self) -> &'static str { "orders.{id}" }

    async fn authorize(
        &self,
        _req: &Request,
        params: &ChannelParams,
        _data: &Value,
    ) -> bool {
        let order_id = params.get("id").unwrap_or_default();
        // Anhand der erfassten id gaten - gehört diese Bestellung
        // dem Session-Nutzer?
        !order_id.is_empty()
    }
}

impl PrivateChannel for OrderChannel {}

// Eine Registrierung bedient orders.42, orders.99,
// orders.featured, …
registry.register(OrderChannel);
```

Jedes `{param}` bindet genau ein Punkt-Segment: `orders.{id}` passt
auf `orders.42`, aber nicht auf `orders` oder `orders.42.line`. Die
Auflösung bevorzugt eine exakte Fixname-Registrierung gegenüber jedem
Pattern (`orders.featured` schlägt `orders.{id}` für diesen einen
Namen), dann das spezifischste Pattern (die meisten literalen
Segmente), mit dem lexikografisch kleinsten Pattern als
deterministischem Tie-Break.

### Presence-Kanäle

Presence-Kanäle verfolgen Mitgliedschaft. Wenn ein Client abonniert,
stellt der Hub diesem Client einen `presence.here`-Snapshot zu und
broadcastet `presence.joined` an jeden anderen Abonnenten. Wenn ein
Client geht, broadcastet der Hub `presence.left`.

Der zweiteilige Vertrag lässt sich leicht nur halb implementieren:
Sie müssen sowohl `Channel::presence_info` überschreiben, um
`Some(self)` zurückzugeben, ALS AUCH `PresenceChannel::member_info`
implementieren. Wird `presence_info` vergessen, wird der Kanal als
Nicht-Presence verdrahtet - Subscribes funktionieren, aber
`presence.joined` / `presence.here` / `presence.left` feuern nie.

```rust
use async_trait::async_trait;
use serde_json::{json, Value};
use suprnova::FrameworkError;
use suprnova::broadcasting::{Channel, ChannelParams, PresenceChannel};
use suprnova::http::Request;

pub struct PresenceLobby;

#[async_trait]
impl Channel for PresenceLobby {
    fn name(&self) -> &'static str { "presence.lobby" }

    // Erforderlich - ohne diese Überschreibung ist PresenceChannel
    // verdrahtet, aber inert.
    fn presence_info(&self) -> Option<&dyn PresenceChannel> {
        Some(self)
    }
}

#[async_trait]
impl PresenceChannel for PresenceLobby {
    async fn member_info(
        &self,
        _req: &Request,
        _params: &ChannelParams,
    ) -> Result<Value, FrameworkError> {
        // Zurückgeben, was andere Abonnenten brauchen, um dieses
        // Mitglied zu identifizieren - typischerweise eine
        // Nutzer-ID. Niemals Secrets oder private PII einschließen.
        Ok(json!({ "user_id": 42, "display_name": "Alice" }))
    }
}
```

Siehe [Presence](#presence) für den vollständigen Event-Fluss und das
Self-Join-Echo.

### Reservierte Namen

Namen, die mit `__` beginnen, sind für Meta-Kanäle des Frameworks
reserviert (`__presence__` trägt die prozessübergreifende
Presence-Replikation). Der Aufruf von `registry.register(channel)`
mit einem `__`-präfigierten Namen gerät bei der Registrierung in
Panic, sodass der Fehler beim Boot abgefangen wird, nicht zur
Laufzeit.

### Warum Suprnova abweicht

Laravel bindet die Kanal-Autorisierung an einen
`$user`-Callback-Parameter, weil PHP den aktuell authentifizierten
Nutzer implizit injiziert. Suprnovas `authorize` nimmt stattdessen
die rohe `Request`, die erfassten `ChannelParams` und ein beliebiges
`data: Value` entgegen - drei orthogonale Eingaben, alle verfügbar,
ohne impliziten Kontext. Sie lesen das Session-Cookie oder das
Bearer-Token aus `Request` und die Routing-artigen Params aus
`ChannelParams`; der `data`-Payload ist ein freier Slot für Tokens,
die der Client zum Zeitpunkt des Subscribe liefert.

Die Defaults des Traits `Channel` sind **absichtlich asymmetrisch**:
`authorize` defaultet auf `true` (Subscribe ist standardmäßig
öffentlich), `authorize_publish` defaultet auf `false` (clientseitig
initiiertes Publish wird standardmäßig verweigert). Die gefährliche
Aktion schlägt geschlossen fehl; die sichere schlägt offen fehl. Im
Zweifel lassen Sie beide unangetastet.

## Der `Broadcastable`-Trait

`Broadcastable: Event + Serialize` - jedes `Broadcastable` ist auch
ein `Event`. Dispatch über `EventFacade::dispatch(event)` führt jeden
In-Process-Listener aus UND pusht den JSON-serialisierten Payload an
jeden WebSocket-Abonnenten auf den Kanälen, die das Event nennt.

```rust
use serde::{Deserialize, Serialize};
use suprnova::Event;
use suprnova::broadcasting::Broadcastable;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderPlaced {
    pub order_id: i64,
    pub user_id: i64,
}

impl Event for OrderPlaced {
    fn event_name() -> &'static str { "OrderPlaced" }
}

impl Broadcastable for OrderPlaced {
    fn broadcast_on(&self) -> Vec<String> {
        // Ein Event, mehrere Kanäle. Jeder Abonnent auf jedem Kanal
        // empfängt dieselbe Envelope.
        vec![
            format!("user.{}.orders", self.user_id),
            "orders.global".into(),
        ]
    }
}
```

Verdrahten Sie die Brücke einmal pro Broadcastable-Typ beim Boot:

```rust
EventFacade::broadcast::<OrderPlaced>(Arc::clone(&hub)).await;
```

Danach ist `EventFacade::dispatch(event).await?` die gesamte
Sendeseite - kein separater `publish`-Aufruf.

Standardmäßig wird das Event über `serde_json::to_value(&event)`
serialisiert und an jeden Abonnenten gepusht. Kanäle ohne Abonnenten
werden auf dem In-Process-Hub still übersprungen; der
prozessübergreifende Hub veröffentlicht sie trotzdem, damit andere
Prozesse eine Chance zur Zustellung bekommen.

Vier optionale Methoden verfeinern das Default-Verhalten:

**`broadcast_event_name(&self) -> &'static str`** - überschreibt den
Event-Namen auf dem Wire. Defaultet auf `Self::event_name()`.
Verwenden Sie das, um die In-Process-Event-Identität vom
Over-the-Wire-Namen zu entkoppeln.

**`broadcast_with(&self) -> Option<Value>`** - geben Sie `Some(value)`
zurück, um einen kuratierten Payload zu pushen statt der
vollständigen Event-Serialisierung (Laravels `broadcastWith()`).
Lassen Sie Secrets aus, oder formen Sie für den Client um, ohne den
Event-Typ zu ändern:

```rust
impl Broadcastable for AccountFunded {
    fn broadcast_on(&self) -> Vec<String> {
        vec![format!("account.{}", self.account_id)]
    }
    fn broadcast_with(&self) -> Option<serde_json::Value> {
        // Niemals den Kontostand auf das Wire legen - nur die
        // öffentliche ID.
        Some(serde_json::json!({ "account_id": self.account_id }))
    }
}
```

**`broadcast_when(&self) -> bool`** - geben Sie `false` zurück, um das
Event an In-Process-Listener zu dispatchen, aber den WebSocket-Push
zu überspringen (Laravels `broadcastWhen()`). Nur der Broadcast wird
gegated; der Rest der Event-Pipeline läuft unverändert:

```rust
impl Broadcastable for DraftSaved {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("doc.{}", self.doc_id)] }
    fn broadcast_when(&self) -> bool { self.publish } // nur beim Publish broadcasten
}
```

**`broadcast_to_others(&self) -> bool`** - geben Sie `true` zurück, um
die Verbindung auszuschließen, die den Broadcast ausgelöst hat
(Laravels `toOthers()`). Das Framework weist jeder
Broadcasting-Verbindung beim Connect eine `socket_id` zu (gesendet im
`connected`-Frame); der Browser spiegelt sie als `X-Socket-ID`-Header
auf HTTP-Anfragen zurück; ein `broadcast_to_others`-Event, das
während der Behandlung dieser Anfrage dispatcht wird, überspringt die
auslösende Verbindung. Außerhalb einer Anfrage (ein Worker oder Job)
oder wenn kein `X-Socket-ID` vorhanden ist, degradiert das zu einem
Broadcast an alle:

```rust
impl Broadcastable for MessagePosted {
    fn broadcast_on(&self) -> Vec<String> { vec![format!("chat.{}", self.room)] }
    fn broadcast_to_others(&self) -> bool { true } // der Sender hat es bereits
}
```

Das ist eine Entscheidung pro Event-Typ. Für einen Ausschluss pro
Dispatch veröffentlichen Sie direkt:

```rust
use suprnova::broadcasting::BroadcastEnvelope;

hub.publish(
    BroadcastEnvelope::new(channel, event, data).with_except(socket_id),
).await?;
```

### Dispatch-Reihenfolge mit Geschwister-Listenern

`EventFacade::dispatch` ist **fail-fast**: Wenn ein Hub-Publish `Err`
zurückgibt (z. B. eine Broker-Trennung bei einem prozessübergreifenden
Hub), gibt der `BroadcastListener` `Err` zurück, und alle
Geschwister-Listener, die **danach** registriert wurden, laufen
nicht. Zwei Wege, damit umzugehen:

- Registrieren Sie die Broadcast-Brücke NACH In-Process-Listenern,
  deren Seiteneffekte (DB-Writes, Log-Emission) unabhängig vom
  Broadcast-Ausgang laufen müssen.
- Wechseln Sie zu `EventFacade::dispatch_best_effort(event)`, wenn
  jeder Listener laufen muss, unabhängig davon, ob einer `Err`
  zurückgibt.

In-Memory-Hubs geben nie `Err` zurück - nur die prozessübergreifende
Variante macht Broker-Fehlschläge sichtbar.

## Das Wire-Protokoll

Jede Nachricht über die Broadcasting-Route ist ein UTF-8-JSON-Frame.
Zwei Formen: `ClientFrame` (Client → Server) und `ServerFrame`
(Server → Client).

### Client-Frames

| `action` | Erforderliche Felder | Optionale Felder | Bedeutung |
|----------|-----------------|-----------------|---------|
| `subscribe` | `channel` | `data` | Abonniert `channel`. `data` wird an `Channel::authorize` weitergereicht. |
| `unsubscribe` | `channel` | | Trennt sich von `channel`. |
| `publish` | `channel`, `event`, `data` | | Pusht ein Event an jeden Abonnenten auf `channel`. Gated durch `Channel::authorize_publish` UND erfordert ein aktives Abonnement. |

Clientseitig initiiertes `publish` wird durch **zwei** Prüfungen
gegated: Die Verbindung MUSS ein autorisiertes Abonnement des
Ziel-Kanals halten, UND `Channel::authorize_publish` muss `true`
zurückgeben (es defaultet auf `false`). Das spiegelt Pushers
Client-Event-Vertrag - Kanäle, die Client-Publishes wollen,
entscheiden sich explizit dafür, indem sie den Hook überschreiben.
Die meisten serverseitigen Broadcasting-Kanäle wollen nie
clientseitig initiierte Events, und die Default-Deny-Form entspricht
dieser Absicht.

```json
{"action":"subscribe","channel":"chat.42","data":{"token":"abc"}}
{"action":"unsubscribe","channel":"chat.42"}
{"action":"publish","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
```

### Server-Frames

| `action` | Felder | Bedeutung |
|----------|--------|---------|
| `connected` | `socket_id` | Wird einmal gesendet, zuerst. Spiegeln Sie `socket_id` als `X-Socket-ID`-HTTP-Header zurück, damit serverseitiges `broadcast_to_others` diese Verbindung ausschließen kann. |
| `subscribed` | `channel` | Abonnement akzeptiert. |
| `unsubscribed` | `channel` | Trennung bestätigt. |
| `event` | `channel`, `event`, `data` | Ein Event wurde auf `channel` broadcastet. |
| `lagged` | `channel`, `skipped` | Der Abonnent ist hinter den Ringpuffer des Servers pro Kanal zurückgefallen, und `skipped` Envelopes wurden auf dieser Verbindung verworfen. Der lokale Client-Zustand für `channel` ist veraltet; erneut abrufen, bevor weitere Events verarbeitet werden. |
| `error` | `channel` (nullable), `reason` | Die letzte Aktion ist fehlgeschlagen. `channel` ist `null` bei Fehlern auf Envelope-Ebene, die an keinen Kanal gebunden sind. |

```json
{"action":"connected","socket_id":"6f1a3c2e-…"}
{"action":"subscribed","channel":"chat.42"}
{"action":"unsubscribed","channel":"chat.42"}
{"action":"event","channel":"chat.42","event":"MessagePosted","data":{"text":"hi"}}
{"action":"lagged","channel":"chat.42","skipped":42}
{"action":"error","channel":"chat.42","reason":"unauthorized"}
{"action":"error","channel":null,"reason":"malformed envelope: …"}
```

#### Über `lagged`

Jeder Kanal hat einen Ringpuffer pro Prozess (256 Envelopes). Ein
Abonnent, der nicht schnell genug leert - ein langsamer Client, ein
hängender Forwarder -, fällt zurück, und der Puffer überschreibt die
ältesten Events. Passiert das, sendet der Server einen
`lagged`-Frame, der den Kanal und die Anzahl verworfener Events
nennt, und stellt danach weiterhin normal nachfolgende Frames zu. Die
Lücke ist von der Serverseite aus **nicht** wiederherstellbar; der
Client muss erneut abrufen oder resynchronisieren, bevor er weitere
Events auf diesem Kanal verarbeitet. Events still fallen zu lassen
würde Bugs sich als "wir haben einen Tick verloren" statt als "der
Zustand des Clients ist vom Zustand des Servers abgewichen"
verstecken lassen.

#### Publish-Fehlschläge

Wenn ein clientseitig initiiertes `publish` von `authorize_publish`
akzeptiert wird, aber das Hub-Publish selbst fehlschlägt
(Broker-Trennung beim prozessübergreifenden Hub), erhält der
auslösende Client einen `error`-Frame mit
`reason: "publish failed: …"`, damit er weiß, dass das Event andere
Prozesse nicht erreicht hat. Andere Abonnenten werden nicht
benachrichtigt.

### Beispiel-Session

```
S → C  {"action":"connected","socket_id":"6f1a3c2e-…"}
C → S  {"action":"subscribe","channel":"order.updates","data":{}}
S → C  {"action":"subscribed","channel":"order.updates"}

# Server dispatcht OrderPlaced:
S → C  {"action":"event","channel":"order.updates","event":"OrderPlaced","data":{"order_id":99,"user_id":42}}

C → S  {"action":"subscribe","channel":"chat.private","data":{"token":"bad"}}
S → C  {"action":"error","channel":"chat.private","reason":"unauthorized"}

C → S  {"action":"unsubscribe","channel":"order.updates"}
S → C  {"action":"unsubscribed","channel":"order.updates"}
```

## Middleware pro Route

Broadcasting-Routen unterstützen dieselbe
`.middleware(M)`-Verkettung wie reine WebSocket-Routen:

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new()),
```

Eine Non-2xx-Response aus irgendeiner Middleware unterbricht das
Upgrade per Short-Circuit - der Client erhält die
HTTP-Fehler-Response, und es passiert kein WebSocket-Handshake. Das
ist der richtige Ort, um Auth auf Transport-Ebene zu erzwingen
(Session-Gültigkeit, Origin-Prüfungen, Rate-Limits zur
Verbindungszeit), ohne die Prüfung im `authorize` jedes Kanals zu
duplizieren.

Mehrere Middleware komponieren sich von links nach rechts:

```rust
ws!("/ws/broadcast", broadcasting_handler())
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

Die Trennung ist beabsichtigt: **Transport-Ebene** (wer überhaupt die
Verbindung öffnen darf) lebt in Middleware; **Kanal-Ebene** (wer
welchen Kanal abonnieren darf) lebt in `Channel::authorize`.

### Pro-Route-`WsConfig`

Überschreiben Sie die prozessweiten WebSocket-Defaults pro Route.
Verketten Sie `.config(WsConfig { ... })` nach dem Handler - vor oder
nach `.middleware(M)` (die Reihenfolge spielt keine Rolle):

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

ws!("/ws/chat", broadcasting_handler())
    .config(WsConfig {
        ping_interval: Duration::from_secs(5),
        max_missed_pings: 1,
        ..Default::default()
    })
    .middleware(AuthMiddleware::new())
```

Die fünf konfigurierbaren Felder und wo jedes davon eine Rolle
spielt:

| Feld | Default | Anwendungsfall |
|-------|---------|----------|
| `ping_interval` | 30s | Chat / Presence: auf 5-10s verkürzen, um tote mobile Verbindungen schnell zu erkennen. Bulk-Daten-Streaming: verlängern, um Overhead zu reduzieren. |
| `max_missed_pings` | 2 | Auf `1` setzen für Chat, wo ein verpasster Pong sofort schließen soll. Auf `3+` setzen für instabile mobile Netzwerke. Auf `usize::MAX` setzen, um Close-bei-fehlendem-Pong zu deaktivieren. |
| `max_message_size` | 1 MiB | Für öffentliche Endpunkte sicherer Default. Bei `WsConfig::generous()` (64 MiB) für vertrauenswürdige interne Feeds starten. |
| `max_frame_size` | 64 KiB | Bemessen für Chat-/Benachrichtigungs-Frames mit Spielraum. Bei `WsConfig::generous()` (16 MiB) für große unfragmentierte Frames starten. |
| `origin_policy` | `SameOrigin` | Defaults lehnen Cross-Origin-Upgrades ab - der einzige CSRF-Schutz, den ein Browser-WS-Handshake hat. Verwenden Sie `AllowList(vec![...])` für explizite Cross-Origin-Frontends, oder `AllowAny` nur für Nicht-Browser-Endpunkte. |

Wird kein `.config(...)` angegeben, erbt die Route
`WsConfig::default()`. Explizite Pro-Route-Config gewinnt immer
gegenüber dem Default.

Für Routen, die vertrauenswürdige interne Feeds bedienen
(Server-zu-Server-Fanout, große Binärübertragungen), starten Sie bei
der Trusted-Feed-Factory und passen Sie nach Bedarf an:

```rust
use suprnova::ws::WsConfig;
use std::time::Duration;

ws!("/ws/internal/firehose", FirehoseHandler::new())
    .config(WsConfig {
        ping_interval: Duration::from_secs(10),
        ..WsConfig::generous() // 64 MiB Message / 16 MiB Frame
    })
```

## Presence

Wenn ein Client erfolgreich einen Presence-Kanal abonniert, tut der
Hub Folgendes:

1. Ruft `PresenceChannel::member_info` mit der Upgrade-`Request` und
   den erfassten `ChannelParams` auf, um die Daten des beitretenden
   Mitglieds zu sammeln.
2. Sendet einen `presence.here`-Event-Frame an den neuen Abonnenten
   mit `data: { "members": [...] }` - einem Snapshot aller aktuell
   verfolgten Mitglieder (ohne das neu beitretende).
3. Veröffentlicht ein `presence.joined`-Event mit
   `data: <member_info>` auf dem Kanal. Jeder Abonnent -
   einschließlich des neuen über seinen eigenen Forwarder - empfängt
   es; Clients filtern den Self-Join heraus, indem sie die Identität
   des beitretenden Mitglieds mit ihrer eigenen vergleichen.

Wenn ein Abonnent die Verbindung trennt oder einen
Unsubscribe-Frame sendet:

4. Der Hub veröffentlicht ein `presence.left`-Event mit den Daten des
   ausscheidenden Mitglieds. Jeder verbleibende Abonnent empfängt es.

Alle drei Frames kommen als `event`-Action-Frames mit reservierten
`event`-Namen an:

```json
{"action":"event","channel":"presence.lobby","event":"presence.here","data":{"members":[{"user_id":1},{"user_id":2}]}}
{"action":"event","channel":"presence.lobby","event":"presence.joined","data":{"user_id":3}}
{"action":"event","channel":"presence.lobby","event":"presence.left","data":{"user_id":3}}
```

Über Prozesse hinweg wird der Presence-Zustand über den reservierten
Meta-Kanal `__presence__` repliziert (siehe [Prozessübergreifender
Fan-out](#prozessübergreifender-fan-out)). Track- und
Untrack-Operationen auf jedem Prozess propagieren an alle Abonnenten;
`list_members` gibt die zusammengeführte Sicht zurück (lokal +
remote). Tote Prozesse, deren `untrack_member` nie gefeuert hat,
bekommen ihre Mitglieder über eine TTL bereinigt - Default 60 s.

## Prozessübergreifender Fan-out

Der Default-`InMemoryBroadcastHub` fächert nur an Abonnenten im
aktuellen Prozess auf. Für Deployments mit mehreren Replicas
aktivieren Sie das Cargo-Feature `broadcasting-fanout` und tauschen
`SeaStreamerBroadcastHub` ein:

`Cargo.toml`:

```toml
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.3", features = ["broadcasting-fanout"] }
```

`src/bootstrap.rs`:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, ChannelRegistry};
use suprnova::broadcasting::fanout::SeaStreamerBroadcastHub;
use suprnova::container::App;

pub async fn register() {
    let hub: Arc<dyn BroadcastHub> = Arc::new(
        SeaStreamerBroadcastHub::new(
            "redis://broker:6379",   // Streamer-URI (Backend wird aus dem Schema gewählt)
            "suprnova-broadcast",    // Stream-Key (von jedem Prozess im Cluster geteilt)
        )
        .await
        .expect("connect"),
    );
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub));
    // ... Rest des Bootstraps unverändert
}
```

Der Konstruktor nimmt zwei Argumente: die Streamer-URI (wählt das
Backend zur Laufzeit anhand des Schemas) und den Stream-Key (den
Topic-Namen, den jeder Prozess im Cluster teilt). Verwenden Sie auf
jeder Replica denselben Stream-Key, sonst sehen sie die Events der
jeweils anderen nicht.

`new_with_presence_ttl(uri, key, ttl)` überschreibt die
Standard-Presence-TTL von 60 s - nützlich für Tests, die den
Crash-Recovery-Pfad schnell durchspielen müssen.
`new_loopback(uri, key)` aktiviert stdio-Loopback für
Integrationstests in einem einzigen Prozess; die Duplikat-Absicherung
sorgt dafür, dass jedes App-Event lokal trotzdem genau einmal
zugestellt wird.

### Backends

Das Backend wird zur Laufzeit aus dem URI-Schema ausgewählt:

| URI-Schema | Backend | Produktionsreif | Hinweise |
|------------|---------|------------------|-------|
| `redis://`, `rediss://` | Redis Streams | **Ja** | Standardempfehlung. `rediss://` verwendet TLS. Im Default-Build aktiviert. |
| `kafka://`, `kafka+ssl://` | Kafka | **Ja** | Erfordert `kafka` im `sea-streamer`-Feature-Set (`framework/Cargo.toml`). |
| `stdio://` | stdin-/stdout-Pipes | Nein - nur Tests | Loopback in einem einzigen Prozess. |
| `file://` | Lokale Datei | Nein - Single-Host | Erfordert `file` im `sea-streamer`-Feature-Set. |

Der Default-Build von Suprnova aktiviert `stdio` + `redis` + `socket`.
Um Kafka oder Datei zu aktivieren, bearbeiten Sie
`framework/Cargo.toml` und fügen das entsprechende
`sea-streamer`-Feature hinzu.

### Architektur

Jedes `publish(envelope)` tut zwei Dinge parallel:

1. **Lokaler Fan-out** - der innere `InMemoryBroadcastHub` stellt
   sofort an Abonnenten in diesem Prozess zu. Lokale Abonnenten warten
   nie auf das Netzwerk.
2. **Stream-Schreibvorgang** - dieselbe Envelope wird serialisiert und
   in den sea-streamer-Stream gepusht, sodass die Consumer-Pump jedes
   anderen Prozesses sie aufnimmt und lokal zustellt.

Eine Absicherung gegen Doppelzustellung verhindert, dass jedes
App-Daten-Event zweimal gesehen wird: Die Hub-Instanz hat eine
zufällige UUID, jede von ihr erzeugte Envelope trägt diese UUID, und
die Consumer-Pump überspringt eingehende Envelopes, deren Instanz-ID
mit der des lokalen Hubs übereinstimmt. Nachrichten des
Presence-Meta-Kanals sind eine Ausnahme - jeder Hub braucht seine
eigenen Events in der prozessübergreifenden Sicht, damit der Lesepfad
einheitlich ist.

Der Backend-Dispatch ist Enum-basiert, nicht Trait-Objekt-basiert: Der
Hub hält einen konkreten `SeaProducer` / `SeaConsumer` aus dem
Socket-Adapter von sea-streamer, der ein Enum über jedes einkompilierte
Backend ist. Kein `dyn`-Overhead an der Publish-Aufrufstelle.

### Prozessübergreifende Presence

`SeaStreamerBroadcastHub` repliziert den Presence-Zustand automatisch
über Prozesse hinweg. Jede Instanz bekommt bei der Konstruktion eine
UUID als `instance_id`; `track_member` / `untrack_member` publizieren
`PresenceEvent`s auf den reservierten `__presence__`-Meta-Kanal. Jeder
Prozess pflegt eine `cross_process_view`, die von seiner Consumer-Task
aktualisiert wird; `list_members` liefert die zusammengeführte Sicht
(lokal und entfernt einheitlich).

Lebendigkeit: Jeder Prozess publiziert seine Mitglieder alle `ttl / 6`
erneut (10 s bei der Standard-TTL von 60 s), als Heartbeat. Veraltete
Einträge - Mitglieder, deren `last_seen` die TTL überschreitet - werden
alle `ttl / 2` weggeschnitten. Das fängt Prozessabstürze ab, die nicht
mehr dazu kamen, `MemberRemoved` zu publizieren.

## Schließen bei fehlendem Pong

Broadcasting-Routen nehmen am selben WebSocket-Herzschlag teil wie
reine `ws!`-Routen. Das Framework sendet alle `WsConfig::ping_interval`
einen Ping (Default 30 s). Antwortet eine Verbindung nicht innerhalb
von `max_missed_pings` aufeinanderfolgenden Intervallen (Default 2)
mit einem Pong, schließt das Framework mit Code 1011.

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let config = WsConfig {
    ping_interval: Duration::from_secs(15),
    max_missed_pings: 3,
    ..WsConfig::default()
};
```

Ein niedrigeres `ping_interval` erkennt tote Verbindungen schneller,
auf Kosten höheren Basis-Traffics. `max_missed_pings: 1` schließt
nach dem allerersten verpassten Pong - verwenden Sie das nur, wenn
Netzwerk-Glitches selten sind und Sie das schnellstmögliche
Aufräumen toter Verbindungen wollen. `max_missed_pings: usize::MAX`
deaktiviert Close-bei-fehlendem-Pong vollständig.

## Production-Deployment

Broadcasting-Routen sind HTTP-Verbindungen, die ein Upgrade
durchlaufen haben, auf demselben hyper-Listener wie Ihre HTTP-Routen.
TLS-Terminierung passiert vorgeschaltet, exakt wie im
[WebSocket-Kapitel](websockets.md#production-deployment) beschrieben.
Die nginx- und Caddy-Konfigurationen aus jenem Kapitel gelten
unverändert - erweitern Sie sie, um den Pfad `/ws/broadcast`
abzudecken.

Aktive WebSocket-Handler-Tasks (einschließlich
Broadcasting-Verbindungen) werden im `WS_TASKS`-Set des Frameworks
getrackt und beim Graceful Shutdown geleert, sodass in-flight
Event-Zustellungen abschließen, bevor der Prozess endet.

## Broadcasts testen

`RecordingBroadcastHub` ist das Suprnova-Analogon zu Laravels
`Broadcast::fake()` - ein `BroadcastHub`, der jede veröffentlichte
Envelope aufzeichnet, während er weiterhin an aktive Abonnenten
zustellt. Binden Sie ihn in einem Test an Stelle von
`InMemoryBroadcastHub` und assert, was broadcastet wurde, ohne vorher
zu abonnieren:

```rust
use std::sync::Arc;
use suprnova::broadcasting::{BroadcastHub, RecordingBroadcastHub};
use suprnova::container::App;

#[tokio::test]
async fn shipping_an_order_broadcasts_to_the_user_channel() {
    let hub = Arc::new(RecordingBroadcastHub::new());
    App::bind::<dyn BroadcastHub>(Arc::clone(&hub) as Arc<dyn BroadcastHub>);

    // ... Code ausführen, der veröffentlicht (direkt oder über ein
    // dispatchtes Broadcastable) ...

    hub.assert_broadcast("orders.42", "OrderShipped");
    assert_eq!(hub.count(), 1);
}
```

| Helfer                         | Prüft                                                    |
|--------------------------------|----------------------------------------------------------|
| `assert_broadcast(ch, ev)`     | mindestens eine Envelope auf `ch` mit Event-Namen `ev`   |
| `assert_nothing_broadcast()`   | nichts wurde veröffentlicht                              |
| `broadcasts()`                 | `Vec<BroadcastEnvelope>` - jede aufgezeichnete Envelope  |
| `count()`                      | Gesamtzahl aufgezeichneter Envelopes                     |

Um zu prüfen, dass ein `Broadcastable`-*Event* überhaupt dispatcht
wurde (statt was das Wire erreicht hat), zeichnet `EventFacade::fake()`
das Event selbst auf - siehe
[Ereignisse](events.md#testing--eventfacadefake).

## Laravel-Paritätsreferenz

| Laravel | Suprnova |
|---------|----------|
| `Broadcast::channel('name', fn(...))` | `Channel`-Trait-Impl + `registry.register(...)` |
| `Broadcast::channel('orders.{id}', ...)` | `fn name() -> "orders.{id}"`, Params in `ChannelParams` |
| `PrivateChannel` (Interface) | Marker-Trait `PrivateChannel` + Überschreiben von `authorize` |
| `PresenceChannel` (Interface) | `PresenceChannel` + Überschreiben von `Channel::presence_info` |
| `ShouldBroadcast` (Interface) | Trait `Broadcastable` |
| `broadcastOn()` | `broadcast_on(&self) -> Vec<String>` |
| `broadcastAs()` | `broadcast_event_name(&self) -> &'static str` |
| `broadcastWith()` | `broadcast_with(&self) -> Option<Value>` |
| `broadcastWhen()` | `broadcast_when(&self) -> bool` |
| `toOthers()` | `broadcast_to_others(&self) -> bool` |
| `Broadcast::fake()` | `RecordingBroadcastHub`, gebunden als `dyn BroadcastHub` |
| `assertBroadcasted` | `RecordingBroadcastHub::assert_broadcast(channel, event)` |
| Pusher-/Reverb-/Ably-Treiber | `InMemoryBroadcastHub` (Single-Process) oder `SeaStreamerBroadcastHub` (prozessübergreifend: Redis / Kafka / file / stdio) |
| Echo-Client-Bibliothek | nicht mitgeliefert - verdrahten Sie das JSON-Envelope-Protokoll vom Browser aus vorerst händisch |

## Referenz

| Symbol | Zweck |
|--------|---------|
| `suprnova::broadcasting::Channel` | Channel-Trait. Überschreiben Sie `name()` (erforderlich), `authorize`, `authorize_publish`, `presence_info`. |
| `suprnova::broadcasting::ChannelParams` | Erfasste Werte aus einem parametrisierten `name()`. `get(key) -> Option<&str>`. Leer bei festen Namen. |
| `suprnova::broadcasting::PrivateChannel` | Marker-Trait auf einem `Channel`, das `authorize` überschreibt. Keine erforderlichen Methoden. |
| `suprnova::broadcasting::PresenceChannel` | `async fn member_info(req, params) -> Result<Value, FrameworkError>`. Erfordert Überschreiben von `Channel::presence_info`. |
| `suprnova::broadcasting::ChannelRegistry` | Hält jeden registrierten Kanal. Gebunden als `Arc<ChannelRegistry>` im Container; aufgelöst von `BroadcastingWsHandler`. |
| `suprnova::broadcasting::Broadcastable` | Trait auf `Event + Serialize`. Erforderlich: `broadcast_on()`. Optional: `broadcast_event_name`, `broadcast_with`, `broadcast_when`, `broadcast_to_others`. |
| `suprnova::broadcasting::BroadcastHub` | Hub-Trait. `subscribe`, `publish`, `subscriber_count`, Presence Track/Untrack/List. |
| `suprnova::broadcasting::InMemoryBroadcastHub` | Default In-Process-Hub. Keine externen Abhängigkeiten. Publish gibt unbedingt `Ok` zurück. |
| `suprnova::broadcasting::RecordingBroadcastHub` | Test-Double. Zeichnet jedes Publish auf; stellt weiterhin an aktive Abonnenten zu. |
| `suprnova::broadcasting::BroadcastEnvelope` | Ein veröffentlichtes Event: `channel`, `event`, `data`, `except`. Builder `new(ch, ev, data)`; `.with_except(socket_id)` für Ausschluss pro Dispatch. |
| `suprnova::broadcasting::ClientFrame` / `ServerFrame` | Die JSON-Envelope-Wire-Typen. `ServerFrame::Lagged { channel, skipped }` macht Ringpuffer-Überläufe pro Kanal sichtbar. |
| `suprnova::broadcasting::BroadcastingWsHandler` | Der wiederverwendbare `WebSocketHandler` des Frameworks. Konstruktor: `BroadcastingWsHandler::new(hub, registry)`. An `ws!()` übergeben. |
| `suprnova::broadcasting::fanout::SeaStreamerBroadcastHub` | Prozessübergreifender Hub hinter `broadcasting-fanout`. `new(uri, stream_key)`, `new_with_presence_ttl(uri, key, ttl)`, `new_loopback(uri, key)`. |
| `EventFacade::broadcast::<E>(hub)` | Registriert die Brücke Event → Hub für `E`. Einmal pro `Broadcastable` beim Boot aufrufen. |
| `EventFacade::dispatch(event)` | Feuert In-Process-Listener UND veröffentlicht auf dem Hub auf jedem Kanal, den `E::broadcast_on()` zurückgibt. |
| `WsRouteDef::config(WsConfig)` | Pro-Route-Override für die WS-Config. Komponiert sich mit `.middleware(M)` in beliebiger Reihenfolge. |
| `WsRouteDef::middleware(M)` | Pro-Route-Middleware-Chain. Eine Non-2xx-Response unterbricht das Upgrade per Short-Circuit. |
| `WsConfig::generous()` | Trusted-Feed-Factory: 64 MiB Message / 16 MiB Frame, übrige Felder unverändert. NICHT auf öffentlichen Routen verwenden. |

## Nächste Schritte

- [WebSockets](websockets.md) - die zugrunde liegende Primitive,
  `WsSocket`, `OriginPolicy`
- [Ereignisse](events.md) - `EventFacade`, fail-fast vs. best-effort
  Dispatch
- [Server-Sent Events](sse.md) - unidirektionaler Push ohne
  Upgrade-Handshake
- [Benachrichtigungen](notifications.md) - der
  `BroadcastChannel`-Benachrichtigungstreiber
- [Web Push](web-push.md) - vom Server gepushte Benachrichtigungen an
  offline Nutzer
