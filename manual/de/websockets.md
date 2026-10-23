# WebSockets

Suprnovas WebSocket-Routen sitzen im selben Router neben den
HTTP-Routen. Sie registrieren einen Pfad und einen Handler; das
Framework erkennt die `Upgrade: websocket`-Anfrage an diesem Pfad,
durchläuft dieselbe Middleware-Chain, die ein HTTP-GET auf diesen Pfad
durchlaufen würde, schließt den RFC-6455-Handshake ab und ruft Ihren
Handler mit einem typisierten `WsSocket` plus der ursprünglichen
`Request` auf. Es gibt keinen separaten WebSocket-Server -
Verbindungen erhalten ihr Upgrade vom selben hyper-Listener, der auch
Ihren HTTP-Traffic bedient. Das Framework verfolgt außerdem jeden
gespawnten Handler in einem Pro-Server-`JoinSet`, sodass ein Graceful
Shutdown in-flight Verbindungen leert, bevor der Listener endet.

## Schnellstart

Fügen Sie einen `EchoHandler` hinzu und registrieren Sie ihn in
`routes!`.

`src/ws/echo.rs`:

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct EchoHandler;

#[async_trait]
impl WebSocketHandler for EchoHandler {
    async fn handle(&self, mut socket: WsSocket, _req: Request) -> Result<(), FrameworkError> {
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("echo: {text}")).await?;
        }
        Ok(())
    }
}
```

`src/routes.rs` (innerhalb von `routes! { ... }`):

```rust
ws!("/ws/echo", app_ws::echo::EchoHandler),
```

Starten Sie die App und verbinden Sie sich mit `wscat`:

```bash
cargo run --bin app
```

```text
$ wscat -c ws://localhost:3000/ws/echo
Connected (press CTRL+C to quit)
> hello
< echo: hello
> suprnova
< echo: suprnova
```

Wenn `recv_text()` `Ok(None)` zurückgibt, hat der Peer die Verbindung
geschlossen; die Schleife endet, der Handler gibt `Ok(())` zurück, und
das Framework sendet einen sauberen Close(1000)-Frame.

## Lifecycle eines Upgrades

Ein WebSocket-Handshake ist ein HTTP-GET mit `Upgrade: websocket`. Das
Framework lässt die vollständige Request-Pipeline dagegen laufen,
bevor irgendwelche Frames fließen:

1. **Route-Match.** Der Router schlägt den Pfad in der WS-Routentabelle nach; bei einem Miss fällt die Anfrage durch zum HTTP-Fallback.
2. **Origin-Policy.** Die konfigurierte [`OriginPolicy`](#origin-policy) wird erzwungen. Ein Verstoß liefert HTTP 403 ohne Upgrade.
3. **Subprotokoll-Verhandlung.** Wenn die Route `accepted_protocols` hat, wird das erste vom Client angebotene, überlappende Token auf der 101-Response gespiegelt.
4. **Middleware-Chain.** `RequestIdMiddleware` läuft ganz außen, gefolgt von jeder global registrierten Middleware, gefolgt von der Pro-Route-Middleware der Route. Eine Non-2xx-Response aus irgendeiner Middleware unterbricht das Upgrade per Short-Circuit - der Peer erhält den HTTP-Fehler, und das WebSocket-Future wird sauber verworfen.
5. **Handshake.** `hyper_tungstenite::upgrade` erzeugt das Future, das sich zu einem `WebSocketStream` auflöst.
6. **Handler-Dispatch.** Die (möglicherweise von Middleware umgeschriebene) `Request` und ein frisch gebauter `WsSocket` werden an `WebSocketHandler::handle` übergeben.
7. **Herzschlag + Handler.** Das Framework spawnt eine Pro-Verbindung-Herzschlag-Task und awaitet das Handler-Future unter einem `ws.connection`-Tracing-Span, der die Request-ID trägt.
8. **Close-Handshake.** Bei `Ok(())` sendet das Framework Close(1000); bei `Err(_)` sendet es Close(1011 "internal error"). Der Forwarder wird awaitet, damit der Close-Frame auf das Wire geflusht ist, bevor die getrackte Task der Verbindung als beendet gemeldet wird.

Die Semantik des Rückgabewerts ist gegenüber HTTP umgekehrt: Es gibt
keinen Body. `Ok(())` bedeutet einen sauberen Verbindungsabbruch;
`Err(_)` wird protokolliert, und der Peer sieht Close(1011). In beiden
Fällen wird die Verbindung abgebaut.

## Die `WsSocket`-API

`WsSocket` ist das bidirektionale Handle, das das Framework an Ihren
Handler übergibt. Intern ist der zugrunde liegende tungstenite-Stream
in Sink- und Stream-Hälften aufgeteilt: Eine Forwarder-Task besitzt
die Sink und leert einen mpsc; die dem Handler zugewandten
Sendemethoden reihen sich in den mpsc ein. Der Handler liest direkt
aus der Stream-Hälfte. Diese Aufteilung bedeutet, dass das Framework
auch Frames pushen kann (Herzschlag-Pings, Broadcaster-Fan-out), ohne
mit dem Sendepfad des Handlers zu konkurrieren.

### `send_text`

```rust
socket.send_text("hello").await?;
socket.send_text(format!("user {id} joined")).await?;
```

Reiht einen UTF-8-Text-Frame ein. Gibt `Err` nur zurück, wenn die
Verbindung bereits geschlossen ist.

### `send_binary`

```rust
socket.send_binary(bytes).await?;
```

Reiht einen Binär-Frame ein. Akzeptiert alles, was `Into<Vec<u8>>`
ist. Dieselbe Fehlersemantik wie `send_text`.

### `recv_text`

```rust
while let Some(text) = socket.recv_text().await? {
    // text: String
}
// Ok(None) bedeutet, der Peer hat geschlossen.
```

Gibt die nächste Textnachricht zurück und verwirft dabei still
Frame-Arten, um die sich ein reiner Text-Handler nicht kümmern muss:

- `Message::Binary` - Binär-Payload des Peers
- `Message::Ping` - vom Peer initiierter Ping (tungstenite behandelt den Pong automatisch)
- `Message::Pong` - Pong-Antwort des Peers auf einen Framework-Herzschlag (der Missed-Ping-Zähler wird dabei als Seiteneffekt auf null zurückgesetzt)
- `Message::Frame` - rohe Frame-Varianten aus serverseitigen Kontexten; an dieser Stelle nie erwartet

Ein verschluckter Frame ist weg; es gibt keine nachträgliche
Möglichkeit, ihn zu sehen. Wenn der Handler Binär-Frames oder
Close-Codes beobachten muss, verwenden Sie [`recv`](#recv) ab dem
allerersten Read.

### `recv`

```rust
use tokio_tungstenite::tungstenite::Message;

while let Some(msg) = socket.recv().await? {
    match msg {
        Message::Text(t)   => { /* ... */ }
        Message::Binary(b) => { /* ... */ }
        Message::Close(_)  => break,
        _                  => {}
    }
}
```

Gibt die nächste Nachricht jeder Art zurück, einschließlich Binary,
Ping, Pong und Close. `Pong` setzt den Missed-Ping-Zähler weiterhin
als Seiteneffekt zurück, bevor sie zurückgegeben wird. `Ok(None)`
bedeutet, der zugrunde liegende Stream ist beendet.

### `close`

```rust
socket.close(1008, "policy violation").await?;
return Ok(());
```

Reiht einen Close-Frame ein und kehrt zurück. Der Forwarder schreibt
den Frame in die Sink, ruft `close()` auf der Sink auf und terminiert.
Nachfolgende Sends auf demselben Socket geben `Err` zurück, weil der
Forwarder weg ist. Geben Sie nach dem Aufruf von `close` immer sofort
`Ok(())` zurück.

`close` validiert seine Argumente vorab gegen RFC 6455 §7.4 + §5.5.1:

- `code` muss `CloseCode::is_allowed()` erfüllen. Reservierte oder ungültige Codes (1004, 1005, 1006, 1015, alles unter 1000, alles über 4999) werden mit `Err` zurückgewiesen, und **es wird kein Frame gesendet** - die Verbindung bleibt offen, und der Aufrufer kann es mit einem gültigen Code erneut versuchen. Verwenden Sie 1000 für normale Schließung, 1001-1013 für die definierten Gründe, 3000-3999 für IANA-registrierte Codes oder 4000-4999 für anwendungsspezifisch-private Codes.
- `reason` ist auf 123 Bytes begrenzt (das 125-Byte-Limit für Control-Frames minus der zwei Byte für den Code). Längere Gründe werden zurückgewiesen, ohne irgendetwas einzureihen.

### Warum Suprnova abweicht

PHP-Frameworks schrauben WebSocket-Unterstützung als separaten Prozess
an (ratchet, soketi, pusher). Suprnovas WebSocket-Route lebt im selben
`routes! { ... }` wie Ihre HTTP-Routen, bedient vom selben
hyper-Listener, geleert vom selben Graceful-Shutdown-Pfad. Es gibt
eine Binärdatei, eine Konfiguration, ein Deploy. Langlebige
Verbindungen sind erstklassig, weil Tokio sie günstig macht; das
Framework muss sich nicht für sie entschuldigen.

## Pfadparameter

WebSocket-Routen unterstützen dieselbe `{param}`-Capture-Syntax wie
HTTP-Routen. Erfasste Werte sind auf der `Request` verfügbar, die an
den Handler übergeben wird.

```rust
// In routes!:
ws!("/ws/rooms/{id}", RoomHandler),
```

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct RoomHandler;

#[async_trait]
impl WebSocketHandler for RoomHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let room_id = req.param("id")?;
        socket.send_text(format!("joined room {room_id}")).await?;
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[{room_id}] {text}")).await?;
        }
        Ok(())
    }
}
```

`req.param("id")` gibt `Result<&str, ParamError>` zurück; das `?`
propagiert einen `FrameworkError::ParamError`, falls das Segment
fehlt, was dazu führt, dass der Handler `Err` zurückgibt und das
Framework Close(1011) sendet. In der Praxis ist die Capture immer
vorhanden, wenn die Route gematcht hat - der Fehlerpfad ist ein
Sicherheitsnetz gegen Tippfehler im Parameternamen.

Express-artige `:id`-Segmente werden ebenfalls akzeptiert
(`ws!("/ws/rooms/:id", h)`) und werden intern in Matchit-Form
umgewandelt.

Für die vollständige `Request`-API - Header, Cookies, Query-String,
Adresse der Gegenstelle - siehe [die
Anfrage-Dokumentation](requests.md).

## Middleware pro Route

Verketten Sie `.middleware(M)` am `ws!`-Eintrag. Mehrere Middleware
komponieren sich von links nach rechts und laufen in derselben festen
Reihenfolge, in der eine HTTP-Anfrage an denselben Pfad laufen würde:
`RequestIdMiddleware` ganz außen, dann jede global registrierte
Middleware, dann die Pro-Route-Chain, dann der Handler.

```rust
ws!("/ws/private", PrivateHandler)
    .middleware(AuthMiddleware::new())
    .middleware(RateLimitMiddleware::connections_per_ip(100)),
```

Eine Non-2xx-Response aus irgendeiner Middleware unterbricht das
Upgrade per Short-Circuit. Der Peer erhält die Ablehnung (z. B. 401, 403) mit gesetztem `X-Request-Id`, das nie aufgewachte WebSocket-Future wird sauber verworfen, und der Handler wird nie aufgerufen. Das ist die richtige Schicht für Prüfungen auf
Transportebene: wer überhaupt die Verbindung öffnen darf, woher die
Verbindung kommt, wie viele gleichzeitige Verbindungen pro Identität
erlaubt sind.

Middleware kann eine modifizierte `Request` einsetzen, indem sie
`next(modified_req)` aufruft. Der Terminator erfasst das, was am Ende
durch die Chain durchgereicht wird, und genau das sieht der Handler
als sein `Request`-Argument. Middleware, die Identität auflöst (ein
Session-Lookup, eine Token-Prüfung), kann das Ergebnis über
`Request`-Extensions anhängen; der Handler liest es auf demselben Weg
zurück wie HTTP-Controller.

Direkt-auf-`Router`-Varianten (`Router::ws`,
`Router::ws_with_middleware`, `Router::ws_with_config`,
`Router::ws_with_middleware_and_config`) decken dieselbe Oberfläche
für Code ab, der einen `Router` außerhalb des Makros baut. Jede hat
ein fehlbares `try_*`-Geschwister, das bei doppelten oder fehlerhaften
Patterns `Err(FrameworkError)` zurückgibt, statt in Panic zu geraten.

### Warum Suprnova abweicht

Die meisten Ökosysteme überspringen Middleware bei WebSocket-Upgrades
entweder ganz (die Node-Konvention) oder erzwingen eine separate
Registrierungszeremonie für "WebSocket-Middleware" (die
.NET-/Spring-Konvention). Suprnova behandelt das Upgrade als das
HTTP-GET, das es tatsächlich ist: Dieselbe Chain läuft, in derselben
Reihenfolge, mit derselben Short-Circuit-Semantik. Es gibt kein
zweites Konzept zu lernen - `AuthMiddleware`, `RateLimitMiddleware`,
`RequestIdMiddleware`, `CorsMiddleware` funktionieren auf WS-Routen,
weil sie auf jeder Route funktionieren. Origin-Erzwingung ist die
einzige zusätzliche Falte, und sie ist eine Eigenschaft von
`WsConfig`, nicht eine separate Middleware.

## Auth beim Verbindungsaufbau

Der Handler empfängt die von der Middleware umgeschriebene `Request`.
Drei Muster funktionieren gut, in aufsteigender Reihenfolge der
Integration mit dem Rest des Frameworks:

**Muster 1 - Inline-Bearer-Token im Handler.** Am einfachsten.
Funktioniert ohne jede Auth-Middleware. `wscat`, Browser-Clients und
Load Balancer geben alle Header sauber durch.

```rust
use async_trait::async_trait;
use suprnova::{FrameworkError, http::Request, ws::{WebSocketHandler, WsSocket}};

pub struct PrivateChatHandler;

#[async_trait]
impl WebSocketHandler for PrivateChatHandler {
    async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
        let Some(token) = req.header("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            socket.close(1008, "missing bearer token").await?;
            return Ok(());
        };
        let Some(user_id) = verify_token(token).await else {
            socket.close(1008, "invalid bearer token").await?;
            return Ok(());
        };
        while let Some(text) = socket.recv_text().await? {
            socket.send_text(format!("[user {user_id}] {text}")).await?;
        }
        Ok(())
    }
}

async fn verify_token(_token: &str) -> Option<i64> { Some(42) }
```

**Muster 2 - das Upgrade mit einer Routen-Middleware gaten.** Weist
nicht autorisierte Öffnungen zurück, bevor irgendwelche Frames
fließen. Sauberere Trennung der Zuständigkeiten; der Handler sieht nur
authentifizierte Verbindungen.

```rust
ws!("/ws/private", PrivateChatHandler)
    .middleware(AuthMiddleware::new()),
```

`AuthMiddleware` gibt bei nicht authentifizierten Anfragen 401 zurück;
das Upgrade wird mit der Ablehnungs-Response abgebrochen, und der
Handler wird nie aufgerufen.

**Muster 3 - Middleware-Gate plus erneutes Lesen im Handler.**
Middleware unterbricht nicht autorisierte Öffnungen per Short-Circuit;
der Handler liest dann dieselbe Credential (Token, Cookie usw.)
erneut, von der er weiß, dass sie jetzt vorhanden ist, um zu
identifizieren, welcher Nutzer gerade verbunden hat:

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    // Die Middleware hat den Bearer bereits geprüft; wir kommen nur
    // hierhin, wenn er gültig war.
    let token = req.bearer_token().expect("auth middleware vetted bearer presence");
    let user_id = lookup_user_by_token(&token).await?;
    // ...
}
```

**Muster 4 - die Middleware authentifizieren lassen und das Ergebnis
lesen.** Bevorzugt, wenn beim Upgrade bereits eine Auth-Middleware
läuft. Die von ihr aufgelöste Identität wird auf der Anfrage selbst
mitgeführt:

```rust
async fn handle(&self, mut socket: WsSocket, req: Request) -> Result<(), FrameworkError> {
    let Some(user_id) = req.auth_user_id() else {
        socket.close(1008, "unauthenticated").await?;
        return Ok(());
    };
    // `user_id` kam von der Session-/Token-Middleware, nicht von
    // irgendetwas, das der Client in einem Frame gesendet hat.
    socket.send_text(format!("welcome, {user_id}")).await?;
    Ok(())
}
```

Das ist es, was den `authorize`-Hook eines privaten Broadcast-Kanals
bedeutsam macht: Er empfängt dieselbe `Request`, kann also auf
serverseitig abgeleiteter Identität gaten statt auf einem Wert, den
der Client gewählt hat. Bevor `auth_user_id` existierte, hatte ein
Kanal nichts Vertrauenswürdiges, das er konsultieren konnte, und der
naheliegende Platzhalter - "jeden Abonnenten akzeptieren, dessen
Subscribe-Frame ein Token trägt, das richtig aussieht" - ist überhaupt
kein Gate.

Die Thread-Local-Accessoren, die in HTTP-Controllern funktionieren -
`session()`, `Auth::user()`, die Pro-Request-`Context`-Bag - sind
innerhalb eines WebSocket-Handlers weiterhin **nicht** befüllt. Die
Task-Local-Scopes der Middleware-Chain wickeln sich ab, wenn die Chain
zurückkehrt; der Handler läuft in einer frisch gespawnten Task, die
nur die Request-ID und die aufgelöste Auth-ID erbt. Lesen Sie alles
andere, was der Handler braucht, direkt von der `Request` (Header,
Cookies über `req.cookie("...")`, erfasste Params, das Bearer-Token
über `req.bearer_token()`) - die überleben in die Handler-Task hinein.

### Warum Suprnova abweicht

Laravel autorisiert Broadcast-Kanäle über einen separaten
HTTP-Endpunkt (`/broadcasting/auth`), sodass der Kanal-Callback in
einer gewöhnlichen Anfrage mit voller verfügbarer Session läuft.
Suprnova autorisiert statt dessen in-process während des Upgrades -
eine Verbindung, kein zweiter Round-Trip -, was bedeutet, dass die
Identität explizit über die Spawn-Grenze getragen werden muss, statt
erneut nachgeschlagen zu werden.

## `WsConfig`

`WsConfig` steuert das Pro-Verbindung-Verhalten. Die Defaults zielen
auf öffentliche, browserseitige Endpunkte ab - jede aktive Verbindung
reserviert einen tungstenite-Buffer in der Größe von
`max_message_size`, sodass das Framework klein defaultet und Routen,
die mehr brauchen, die Limits explizit anheben lässt.

| Feld                  | Default        | Typ             | Effekt |
|-----------------------|----------------|-----------------|--------|
| `ping_interval`       | 30s            | `Duration`      | Wie oft das Framework einen Ping-Frame sendet, um die Verbindung am Leben zu halten. |
| `max_message_size`    | 1 MiB          | `usize`         | Maximale wieder zusammengesetzte Nachrichtengröße in Bytes. Größere Nachrichten werden von tungstenite zurückgewiesen. |
| `max_frame_size`      | 64 KiB         | `usize`         | Maximale Größe eines einzelnen WebSocket-Frames in Bytes. |
| `max_missed_pings`    | 2              | `usize`         | Aufeinanderfolgende verpasste Pongs, bevor der Herzschlag die Verbindung mit Code 1011 schließt. `usize::MAX` deaktiviert die Erzwingung. |
| `origin_policy`       | `SameOrigin`   | `OriginPolicy`  | Origin-Header-Prüfung, erzwungen zur Upgrade-Zeit. Siehe [Origin-Policy](#origin-policy). |
| `accepted_protocols`  | `vec![]`       | `Vec<String>`   | Vom Server akzeptierte `Sec-WebSocket-Protocol`-Tokens. Leer bedeutet keine Verhandlung. Siehe [Subprotokolle](#subprotokolle). |

Empfohlene Overrides nach Anwendungsfall:

- **Chat / Benachrichtigungen / Cursor-Positionen** - die Defaults passen. Senken Sie `ping_interval` auf 5-10s, wenn Ihr LB einen aggressiven Idle-Timeout hat.
- **Vertrauenswürdige interne Feeds** (Server-zu-Server-Fan-out, Bulk-Export, große Binärübertragungen) - starten Sie bei `WsConfig::generous()`, was `max_message_size` auf 64 MiB und `max_frame_size` auf 16 MiB anhebt, während die übrigen Defaults bleiben.
- **Spezifischer übergroßer Payload** (eine Route, die 256-MiB-Audiodateien hochlädt) - setzen Sie die Felder direkt; wenden Sie das größere Limit nicht auf Routen an, die es nicht brauchen.

Die Config-Struktur ist über `Default` konstruierbar, und jedes Feld
ist öffentlich:

```rust
use std::time::Duration;
use suprnova::ws::WsConfig;

let chat = WsConfig {
    ping_interval: Duration::from_secs(5),
    max_missed_pings: 1,
    ..Default::default()
};

let trusted = WsConfig::generous();
assert_eq!(trusted.max_message_size, 64 * 1024 * 1024);
assert_eq!(trusted.max_frame_size, 16 * 1024 * 1024);
```

Wenden Sie den Override pro Route entweder am `ws!`-Eintrag oder an
`Router::ws_with_config` an:

```rust
ws!("/ws/chat", ChatHandler).config(chat),
```

`WsConfig` wird bei der Routenregistrierung validiert. Ein
`ping_interval` von null oder ein `max_missed_pings` von null würde
die Herzschlag-Task korrumpieren; beide werden beim Boot
zurückgewiesen, statt bei der ersten Verbindung in Panic zu geraten.

### Herzschlag und Schließen bei fehlendem Pong

Für jede Verbindung nach dem Upgrade spawnt das Framework eine
Herzschlag-Task, die alle `ping_interval` ein `Ping(b"")` sendet. Bei
jedem Tick erhöht sich der Missed-Ping-Zähler; bei jedem Peer-Pong
wird er auf null zurückgesetzt. Erreicht der Zähler
`max_missed_pings`, sendet der Herzschlag Close(1011 "no pong
response"), und die Verbindung wird abgebaut. Setzen Sie
`max_missed_pings` auf `usize::MAX`, um die Erzwingung zu deaktivieren
(Pings fließen weiter, aber die Verbindung wird wegen fehlender Pongs
nie geschlossen).

Der erste Tick wird beim Start der Task konsumiert, damit der Peer vor
dem ersten Ping mindestens ein volles Intervall Gnadenzeit bekommt.

## Origin-Policy

Browser senden bei WebSocket-Handshakes immer einen `Origin`-Header.
Anders als `fetch()` / `XMLHttpRequest` sind WebSocket-Upgrades nicht
durch CSRF-Token-Middleware geschützt (der Handshake trägt kein
Token), sodass eine Same-Origin-`Origin`-Prüfung das Einzige ist, was
zwischen einer bösartigen Seite und einem privilegierten WS-Endpunkt
auf der Session eines angemeldeten Nutzers steht. Das Framework
erzwingt die konfigurierte Policy, bevor `hyper_tungstenite::upgrade`
aufgerufen wird; ein Verstoß liefert HTTP 403 ohne Upgrade.

```rust
use suprnova::ws::{OriginPolicy, WsConfig};

let cfg = WsConfig {
    origin_policy: OriginPolicy::AllowList(vec![
        "https://app.example.com".into(),
        "https://admin.example.com".into(),
    ]),
    ..Default::default()
};
```

| Variante     | Verhalten |
|--------------|----------|
| `SameOrigin` (Default) | Erlaubt nur, wenn der Host von `Origin` (und der Port, falls vorhanden) zum `Host`-Header der Anfrage passt. Fehlendes `Origin` wird zurückgewiesen. Das Schema wird nicht verglichen (TLS terminiert vorgeschaltet, sodass der Server nicht zuverlässig sagen kann, ob das öffentliche Schema https oder http war). |
| `AllowAny`   | Überspringt die Prüfung. Nur für Nicht-Browser-Endpunkte verwenden (Server-zu-Server, native Apps, Test-Mocks). |
| `AllowList(Vec<String>)` | Erlaubt nur, wenn `Origin` exakt (case-insensitive) einem der angegebenen Origins entspricht. Jeder Eintrag ist die vollständige Form `scheme://host[:port]`, die ein Browser senden würde. |

Nicht-Browser-Clients (CLI-Tools, Server, native Apps) senden
typischerweise keinen `Origin`-Header. Routen, die ausschließlich
solche Clients bedienen, sollten `AllowAny` verwenden; Routen, die
beide bedienen, sollten `AllowList` verwenden und jeden
Production-Frontend-Origin aufzählen.

## Subprotokolle

Ein WebSocket-Subprotokoll ist ein Token auf Anwendungsebene (z. B.
`graphql-transport-ws`, `jsonrpc-2.0`), auf das sich Client und Server
während des Handshakes einigen. Befüllen Sie `accepted_protocols`, um
teilzunehmen:

```rust
use suprnova::ws::WsConfig;

let cfg = WsConfig {
    accepted_protocols: vec![
        "graphql-transport-ws".into(),
        "graphql-ws".into(),
    ],
    ..Default::default()
};
```

Wenn der Client `Sec-WebSocket-Protocol` anbietet, wählt das Framework
das erste vom Client angebotene Token (in Client-Präferenzreihenfolge
gemäß RFC 6455 §4.2.2), das sich mit `accepted_protocols` überlappt,
case-insensitiv gematcht, und spiegelt es auf der 101-Response. Hat
der Client Protokolle angeboten, aber keines passte, gelingt das
Upgrade trotzdem, ohne `Sec-WebSocket-Protocol`-Header - RFC 6455
verlangt dann, dass der Browser die Verbindung clientseitig scheitern
lässt, was das richtige Verhalten ist (ein Server, der fortführe,
würde still das falsche Protokoll sprechen).

Ist `accepted_protocols` leer, wird die Verhandlung vollständig
übersprungen - die Upgrade-Response lässt `Sec-WebSocket-Protocol`
aus, und der Client fällt auf die Standard-Protokollbehandlung zurück.

## Production-Deployment

Das Framework kümmert sich um den Handshake und die Frame-I/O. Sie
brauchen keine zusätzliche Konfiguration auf der Framework-Seite für
Production.

**TLS-Terminierung passiert vorgeschaltet.** Clients verbinden sich
über `wss://` mit nginx, Caddy oder dem Cloud-Load-Balancer; der Proxy
entfernt TLS und leitet reines `ws://` an das Framework weiter. Das
Framework braucht kein `rustls`-Feature und kein TLS-Zertifikat.

### nginx

```nginx
location /ws/ {
    proxy_pass http://127.0.0.1:3000;
    proxy_http_version 1.1;
    proxy_set_header Upgrade $http_upgrade;
    proxy_set_header Connection "Upgrade";
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_read_timeout 3600s;
    proxy_send_timeout 3600s;
}
```

`proxy_read_timeout` und `proxy_send_timeout` müssen lang genug sein,
um Idle-Lücken zwischen Herzschlägen zu überbrücken. Mit dem
Standard-`ping_interval` von 30s ist 3600s eine komfortable
Obergrenze.

### Caddy

```caddy
reverse_proxy /ws/* localhost:3000 {
    header_up Upgrade {http.request.header.Upgrade}
    header_up Connection "Upgrade"
}
```

Caddy behandelt `Upgrade` / `Connection` beim Proxying automatisch;
die expliziten `header_up`-Direktiven oben dienen der Klarheit.

### Cloud-Load-Balancer (AWS ALB, GCP GLB)

Aktivieren Sie WebSocket-Unterstützung an der Listener-Regel (AWS ALB
tut das automatisch, wenn das Protokoll der Target-Gruppe HTTP/1.1 mit
ausgeschalteten Sticky-Sessions ist). Stellen Sie sicher, dass der
Idle-Timeout des Load Balancers mindestens so lang ist wie
`ping_interval`; der Herzschlag des Frameworks hält das Wire aktiv,
aber der LB trennt Verbindungen, die aus seiner Sicht idle aussehen.

## Graceful Shutdown

Jeder gespawnte WebSocket-Handler wird im `WS_TASKS`-`JoinSet` des
Servers getrackt. Bei `Ctrl-C` oder einem externen Shutdown-Signal
stoppt der Listener, neue Verbindungen anzunehmen, und `Server::run`
leert das Set, bevor der Prozess endet. Das Handler-Future löst sich
erst auf, wenn der Close-Handshake geflusht wurde: Nachdem der
`handle`-Aufruf des Nutzers zurückgekehrt ist, awaitet das Framework
den Forwarder, damit der finale Close(1000)- oder Close(1011)-Frame
auf das Wire geschrieben wird, bevor die Task der Verbindung als
beendet gemeldet wird. Bei einem sauberen Shutdown sehen Peers eine
normale Schließung, keinen TCP-Reset.

Abgeschlossene Handles werden opportunistisch während der Lebenszeit
des Servers eingesammelt, sodass das `JoinSet` unter langlaufendem
Betrieb nicht unbegrenzt wächst.

## Referenz

| Symbol | Zweck |
|---|---|
| `suprnova::ws::WebSocketHandler` | Trait: `async fn handle(&self, socket: WsSocket, request: Request) -> Result<(), FrameworkError>`. `Send + Sync + 'static`. |
| `suprnova::ws::WsSocket` | Bidirektionales Handle. Methoden: `send_text`, `send_binary`, `recv_text`, `recv`, `close`. `close` validiert Code + Länge des Grunds vorab. |
| `suprnova::ws::WsConfig` | Pro-Verbindung-Konfiguration. Felder: `ping_interval`, `max_message_size`, `max_frame_size`, `max_missed_pings`, `origin_policy`, `accepted_protocols`. Konstruktoren `Default` + `generous()`. Wird bei der Registrierung validiert. |
| `suprnova::ws::OriginPolicy` | `SameOrigin` (Default), `AllowAny`, `AllowList(Vec<String>)`. Wird zur Upgrade-Zeit erzwungen. |
| `ws!(path, Handler)` | Makro-Form für `routes! { ... }`. Gibt eine `WsRouteDef` zurück, die `.config(WsConfig)` und `.middleware(M)` in beliebiger Reihenfolge unterstützt. |
| `Router::ws(path, handler)` | Direkte Registrierung. Gibt `Router` zurück. |
| `Router::ws_with_config(path, handler, cfg)` | Pro-Route-Override für `WsConfig`. |
| `Router::ws_with_middleware(path, handler, mws)` | Pro-Route-Middleware-Liste. |
| `Router::ws_with_middleware_and_config(...)` | Beides. |
| `Router::try_ws*`-Familie | Fehlbare Geschwister - geben `Err(FrameworkError)` bei doppelten oder fehlerhaften Patterns zurück, statt in Panic zu geraten. |

## Nächste Schritte

- [Broadcasting](broadcasting.md) - Kanäle, Presence, das Wire-Protokoll oben auf `ws!`
- [Server-Sent Events](sse.md) - unidirektionaler Push für Browser hinter strikten Proxys
- [Routing](routing.md) - wozu `routes!` und `ws!` tatsächlich expandieren
- [Middleware](middleware.md) - eigene Middleware schreiben, die HTTP und WS einheitlich gated
- [Anfragen](requests.md) - Header, Cookies, Query, Extensions auf der `Request`, die Ihr Handler empfängt
