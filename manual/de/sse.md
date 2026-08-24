# Server-Sent Events

Server-Sent Events (SSE) sind der minimale unidirektionale Push-Kanal vom
Server zum Browser: Der Browser öffnet `EventSource(url)`, der Server
hält eine `text/event-stream`-Response offen und pusht gerahmte Events,
sobald sie passieren. Kein WebSocket-Handshake, kein permessage-deflate,
keine Framing-Bibliotheken - nur `data:`-, `event:`-, `id:`-,
`retry:`-Zeilen, die mit einer Leerzeile enden, gemäß der Spezifikation
von
[WHATWG `EventSource`](https://html.spec.whatwg.org/multipage/server-sent-events.html).

Suprnovas SSE-Primitive klinkt sich in den Streaming-Body-Pfad ein:
Bauen Sie einen `Stream<Item = SseEvent>`, geben Sie ihn an
`HttpResponse::sse(...)` weiter, und das Framework übernimmt
Verbindungsmanagement, Framing, Header und Panic-Isolation. Die
Verbindung bleibt offen, bis der erzeugende Stream endet oder der Client
die Verbindung trennt.

## Wann Sie zu SSE statt WebSockets greifen

| Eigenschaft | SSE | WebSockets |
|----------|-----|------------|
| Richtung | Server → Browser | Bidirektional |
| Transport | Reines HTTP/1.1 oder HTTP/2 | Nur per Upgrade |
| Reconnect | Automatisch, mit `retry:` und `Last-Event-ID` | Manuell |
| Proxys / CDNs | Funktioniert durch alles, was lange HTTP-Responses erlaubt | Braucht oft explizite Upgrade-Unterstützung |
| Browser-API | `EventSource` (eingebaut) | `WebSocket` (eingebaut) |
| Binär-Frames | Nur Text (UTF-8) | Text oder Binär |
| Verbindungsobergrenze pro Tab | 6 (HTTP/1.1) / unbegrenzt (HTTP/2) | Unbegrenzt |

Greifen Sie zu SSE, wenn Sie nur Server-zu-Client-Push brauchen
(Activity-Feeds, Benachrichtigungen, Log-Tails, KI-Streaming). Greifen
Sie zu [WebSockets](websockets.md), wenn Sie bidirektionalen Traffic
oder Binär-Frames brauchen.

## Schnellstart

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

pub async fn stream_ticks(_req: Request) -> Response {
    let (tx, rx) = mpsc::channel::<SseEvent>(16);
    tokio::spawn(async move {
        for i in 0..10 {
            let evt = SseEvent::data(format!("tick {i}"))
                .with_event("tick")
                .with_id(i.to_string());
            if tx.send(evt).await.is_err() {
                break; // Client hat die Verbindung getrennt
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
    });
    Ok(HttpResponse::sse(ReceiverStream::new(rx)))
}
```

Wire-Ausgabe für einen Tick:

```text
event: tick
id: 0
data: tick 0

```

Der Browser parst das und feuert ein `tick`-Event mit
`evt.data === "tick 0"` und `evt.lastEventId === "0"`.

## Die `SseEvent`-API

`SseEvent` ist der Typ, den Sie auf den Stream pushen. Er hat zwei
Arten:

* **Frame** - ein normales Event mit optionalem `event` / `id` /
  `retry` und einem mehrzeiligen `data`-Payload. Gebaut über
  [`SseEvent::data`](#konstruktoren), `SseEvent::json` oder
  `SseEvent::error`.
* **Comment** - ein reiner Wire-only-Keep-alive (`:\n\n` oder
  `: <text>\n\n`). Gebaut über `SseEvent::comment(text)` oder
  `SseEvent::keep_alive()`. Der Browser ignoriert Comments laut Spec;
  die Bytes, die über die Verbindung laufen, sind es, die Idle-Proxys
  und Load Balancer davon abhalten, sie zu schließen.

### Konstruktoren

| Konstruktor | Erzeugt | Verwendung |
|-------------|----------|-----|
| `SseEvent::data(text)` | Frame mit nur `data:`-Zeilen | Das minimale Event |
| `SseEvent::json(event, &payload)` | Frame mit `event:` + JSON-`data:` | Der 95-%-Fall - `JSON.parse(evt.data)` auf dem Client |
| `SseEvent::error(message)` | Frame mit `event: error` | Fehler-Event auf Domänenebene, zu unterscheiden vom Fehler auf Verbindungsebene, den der Browser bei einem Transportfehler feuert |
| `SseEvent::comment(text)` | Comment | Keep-alive mit einem Marker, den der Betreiber in Logs erkennen kann |
| `SseEvent::keep_alive()` | Leerer Comment (`:\n\n`) | Kanonischer Herzschlag mit minimaler Byte-Zahl |

### Builder

| Builder | Effekt | Bei `Comment` |
|---------|--------|--------------|
| `.with_event(name)` | Setzt das Feld `event:` | Stiller No-Op |
| `.with_id(id)` | Setzt das Feld `id:` - erforderlich für Resume-Semantik | Stiller No-Op |
| `.with_retry(Duration)` | Setzt das Feld `retry:` (ms); die Spec sagt, `Duration::ZERO` bedeutet "sofort neu verbinden" | Stiller No-Op |
| `.try_with_event(name)` | Fehlbare Variante - siehe [Sicherheitsvertrag](#sicherheitsvertrag) | `Ok(self)` unverändert |
| `.try_with_id(id)` | Fehlbare Variante von `with_id` | `Ok(self)` unverändert |

Builder auf `Comment` sind absichtlich No-Ops - das Wire-Format hat
keine Möglichkeit, "Comment mit einem Event-Namen" auszudrücken. Eine
Fehlverwendung bleibt still, statt das Event in einen Frame zu
verwandeln und den Produzenten zu überraschen.

### Accessoren

| Methode | Rückgabe |
|--------|---------|
| `.event()` | `Option<&str>` - der Event-Name, falls gesetzt |
| `.id()` | `Option<&str>` - die letzte Event-ID, falls gesetzt |
| `.retry()` | `Option<Duration>` - die Reconnect-Verzögerung, falls gesetzt |
| `.payload()` | `&str` - der `data:`-Payload (oder `""` bei `Comment`) |
| `.is_comment()` | `bool` |
| `.comment_text()` | `Option<&str>` - der Comment-Text, falls dies ein `Comment` ist |

### Wire-Kodierung

`SseEvent::to_wire()` serialisiert das Event in `Bytes`, bereit für den
Body-Stream:

**Frame:**

```text
event: <event>\n   (nur falls Some)
id: <id>\n         (nur falls Some)
retry: <ms>\n      (nur falls Some)
data: <line>\n     (eine pro Zeile im Payload, nach \r/\r\n-Normalisierung)
\n                 (Terminator - von der Spec verlangt)
```

**Comment:**

```text
: <line>\n         (eine pro Zeile im Comment-Text; `:\n` für leere Zeilen)
\n                 (Flush-Grenze)
```

## Sicherheitsvertrag

Das SSE-Wire-Format verwendet CR / LF / NUL als Feld-Terminatoren, ohne
Escape-Mechanismus. Ein Produzent, der Nutzereingaben ungesäubert bis
zu `event:` oder `id:` durchlässt, würde eine
Field-Injection-Schwachstelle öffnen - ein Wert wie
`"legit\ndata: injected"` würde zwei `data:`-Felder auf dem Wire
erzeugen, und `"legit\n\nevent: spoofed"` würde das aktuelle Event
terminieren und ein neues beginnen.

Suprnovas `to_wire()` verteidigt sich auf zwei Ebenen:

* **Feldwerte von `event:` und `id:`** - jedes CR / LF / NUL wird beim
  Serialisieren entfernt. Für jede Entfernung feuert ein strukturiertes
  `WARN`: `target: "suprnova::sse"`, `field = "event"|"id"`. Die
  Warnung protokolliert den Wert nie - diese Bytes sind
  konstruktionsbedingt angreifer-kontrolliert.
* **`data:` und Comment-Text** - `\r\n` und einzelne `\r` werden vor
  dem Splitten zu `\n` normalisiert, sodass ein Produzent, der `\r` in
  einen Payload einbettet, nicht bewirken kann, dass der Parser des
  Empfängers beim Parsen ein `data:`- / `event:`- / `id:`-Feld
  synthetisiert. NUL wird aus dem Comment-Text entfernt, mit einem
  passenden `WARN`.

Wenn Sie bei schlechter Eingabe **schnell fehlschlagen** wollen statt
still zu entfernen, greifen Sie zu den `try_with_*`-Geschwistern:

```rust
use suprnova::{Response, sse::SseEvent};

let evt = SseEvent::data("hello")
    .try_with_event(&user_supplied_event)?     // gibt Err bei CR/LF/NUL zurück
    .try_with_id(&user_supplied_id)?;
```

Der zurückgegebene `FrameworkError::validation(field, ...)` nennt das
Feld; er gibt den Wert NICHT zurück, sodass ein dem Client angezeigtes
400 sicher zu protokollieren ist.

## Keep-alive und Idle-Timeouts von Proxys

Langlebige SSE-Verbindungen sind standardmäßig still. Die meisten
Production-Deployments sitzen hinter einem Proxy / Load Balancer / CDN,
der Idle-Verbindungen schließt, um Ressourcen freizugeben:

* nginx-Standard: 60 Sekunden
* AWS-ALB-Standard: 60 Sekunden
* Cloudflare-Standard: 100 Sekunden

Ein `keep_alive()`-Comment alle 15-30 Sekunden hält die Verbindung
durch all das hindurch am Leben, ohne ein `message`-Event an den
Browser zu dispatchen. Die Minimal-Byte-Form (`:\n\n`) reicht aus, um
die Write-Buffer von Proxys zu flushen, ohne irgendeinen Payload zu
senden.

```rust
use std::time::Duration;
use futures::StreamExt;
use suprnova::sse::SseEvent;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

let (tx, rx) = mpsc::channel::<SseEvent>(16);

// Herzschlag-Task - unabhängig vom Event-Produzenten.
let hb_tx = tx.clone();
tokio::spawn(async move {
    let mut ticker = tokio::time::interval(Duration::from_secs(20));
    loop {
        ticker.tick().await;
        if hb_tx.send(SseEvent::keep_alive()).await.is_err() {
            break; // Client ist weg
        }
    }
});

// Event-Produzent ... sendet Frames in `tx`, sobald sie passieren.
```

## Wiederaufnahme nach Verbindungsabbruch (`Last-Event-ID`)

Wenn der `EventSource` des Browsers die Verbindung abbricht, verbindet
er sich automatisch neu und sendet die zuletzt gesehene `id:` als
`Last-Event-ID`-Header auf der neuen Anfrage. Markieren Sie jedes Event
mit `.with_id(...)` und lesen Sie den Header bei der
Wiederaufnahme-Anfrage aus:

```rust
use futures::StreamExt;
use suprnova::{HttpResponse, Request, Response, sse::{self, SseEvent}};

pub async fn stream_from_resume(req: Request) -> Response {
    let resume_from: u64 = sse::last_event_id(&req)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    // Baut den Produzenten-Stream ab `resume_from + 1`. Die Closure
    // besitzt ihren eigenen laufenden Zähler, sodass die Mutation
    // innerhalb des Streams bleibt.
    let stream = futures::stream::iter(events_since(resume_from))
        .scan(resume_from + 1, |next_id, payload| {
            let id = *next_id;
            *next_id += 1;
            futures::future::ready(Some((id, payload)))
        })
        .map(|(id, payload)| {
            SseEvent::json("activity", &payload)
                .expect("payload is a Serialize value")
                .with_id(id.to_string())
        });

    Ok(HttpResponse::sse(stream))
}
```

`sse::last_event_id(&Request) -> Option<String>` gibt `None` zurück,
wenn der Header fehlt ODER wenn der Wert ein NUL-Byte enthält (laut
WHATWG-Spec macht NUL eine Last-Event-ID ungültig, und der Parser des
Browsers würde sie verwerfen). Der zurückgegebene `String` ist
andernfalls undurchsichtige Nutzereingabe - parsen Sie ihn als Ihren
eigenen Cursor / Ihre eigene Sequenz / Ihren eigenen Offset, bevor Sie
ihn verwenden.

## Fehler auf Domänenebene

`SseEvent::error("...")` erzeugt die übliche Form
`event: error\ndata: <msg>\n\n`. Abonnenten können separat darauf
lauschen, getrennt vom `error` auf Verbindungsebene, das der Browser
bei einem Transportfehler feuert:

```js
const es = new EventSource("/stream");

// Verbindungs- / Transportfehler (kein `data`).
es.onerror = (evt) => console.warn("transport error", evt);

// Fehler auf Domänenebene, ausgelöst von SseEvent::error(...).
es.addEventListener("error", (evt) => console.error("server-side:", evt.data));
```

Beim Mapping von `Stream<Item = Result<T, E>>` auf
`Stream<Item = SseEvent>` ist das idiomatische Muster
`map(|r| match r { Ok(x) => SseEvent::json(...), Err(e) => SseEvent::error(...) })` -
das Error-Mapping auf Consumer-Seite bleibt in der Hand des
Produzenten, und das Framework muss nie eine Default-Form erfinden.

## Einen Stream an viele Abonnenten broadcasten

Fan-out an viele SSE-Abonnenten ist bereits durch das
[Broadcasting-Subsystem](broadcasting.md) abgedeckt: Abonnieren Sie
einen `BroadcastHub`-Kanal und adaptieren Sie den `broadcast::Receiver`
in den `SseEvent`-Stream mit
`tokio_stream::wrappers::BroadcastStream` + `.map(...)`. Jede
Verbindung bekommt ihren eigenen Receiver; der Hub übernimmt die
Slow-Consumer-Policy (`Lagged(n)`-Fehler, wenn ein Abonnent
zurückfällt), und Sie entscheiden, wie Sie das an den Client
weitergeben.

Das funktionierende Dogfood-Beispiel unter
`app/src/controllers/sse_example.rs` implementiert das in ~25 Zeilen:

```rust
use futures::StreamExt;
use std::sync::Arc;
use suprnova::broadcasting::BroadcastHub;
use suprnova::container::App;
use suprnova::{HttpResponse, Request, Response, sse::SseEvent};
use tokio_stream::wrappers::BroadcastStream;

pub async fn stream(_req: Request) -> Response {
    let hub: Arc<dyn BroadcastHub> = App::make::<dyn BroadcastHub>()
        .expect("BroadcastHub not bootstrapped");
    let rx = hub.subscribe("user_registered");

    let stream = BroadcastStream::new(rx).map(|result| match result {
        Ok(envelope) => SseEvent::json("user.registered", &envelope.data)
            .unwrap_or_else(|_| {
                SseEvent::data(envelope.data.to_string())
                    .with_event("user.registered")
            }),
        Err(tokio_stream::wrappers::errors::BroadcastStreamRecvError::Lagged(n)) => {
            SseEvent::data(n.to_string()).with_event("lagged")
        }
    });

    Ok(HttpResponse::sse(stream))
}
```

Das `lagged`-Event erlaubt dem Client, einen vollständigen Refetch und
eine Wiederaufnahme auszulösen - die Verbindung bleibt trotz des Lags
offen.

## `event_stream` und `stream_json`

`HttpResponse::sse` übernimmt die vollständige Rahmung - Sie bauen
jeden `SseEvent` selbst. Zwei höherstufige Geschwister decken die
gängigen Formen ab:

```rust
use suprnova::sse::{EndSignal, StreamedEvent};
use suprnova::{HttpResponse, Request, Response};
use tokio::sync::mpsc;

pub async fn progress(_req: Request) -> Response {
    let (tx, rx) = mpsc::channel::<StreamedEvent>(16);
    tokio::spawn(async move {
        for pct in [25, 50, 75, 100] {
            let evt = StreamedEvent::message(pct).unwrap();
            if tx.send(evt).await.is_err() {
                break; // Client getrennt
            }
        }
    });
    let stream = tokio_stream::wrappers::ReceiverStream::new(rx);
    Ok(HttpResponse::event_stream(stream, EndSignal::default()))
}
```

`StreamedEvent::message(data)` setzt `event` standardmäßig auf
`"update"` - das, worauf `useEventStream` sofort hört;
`StreamedEvent::named(event, data)` überschreibt es für einen Produzenten,
der mehrere logische Kanäle über dieselbe Verbindung verteilt. `data`
erreicht das Wire bei einem einfachen String ohne Anführungszeichen,
andernfalls JSON-kodiert. Das Argument `end: EndSignal` von
`event_stream` steuert den abschließenden Frame nach dem Stream-Ende:
`EndSignal::default()` sendet
`event: update\ndata: </stream>\n\n` (Laravels eigener Standard und das,
was die Option `endSignal` von `useEventStream` prüft);
`EndSignal::None` lässt ihn weg; `EndSignal::text(...)` /
`EndSignal::Event(...)` passen ihn an. Das ist Suprnovas
`ResponseFactory::eventStream($callback, $headers, $endStreamWith)`.

`HttpResponse::stream_json(stream)` - Laravels
`ResponseFactory::streamJson` / `StreamedJsonResponse` - nimmt jeden
`Stream<Item = impl Serialize>` und flusht ihn als ein inkrementell
aufgebautes JSON-Array (`Content-Type: application/json`), statt die
gesamte Collection zuerst zu puffern. Die Bytes auf dem Wire sind exakt
`[item,item,...]`; die vollständige Response lässt sich mit jedem
JSON-Parser deserialisieren.

## Konsumieren aus React / Vue / Svelte

Die [`@laravel/stream-{react,vue,svelte}`](https://github.com/laravel/stream)
Packages besitzen die Client-Seite dieses Wire-Vertrags - Suprnova zielt
auf ihre Oberfläche, statt eine eigene auszuliefern:

| Hook | Spricht mit | Suprnova-Builder |
|---|---|---|
| `useEventStream(url, options)` | `EventSource` (GET, vom Browser verwaltetes Reconnect) | `HttpResponse::event_stream` |
| `useStream(url, options)` | `fetch` (manuelle `ReadableStream`-Leseschleife bei POST) | `HttpResponse::stream_bytes` |
| `useJsonStream(url, options)` | Wie `useStream`, parst das vollständig gepufferte Ergebnis mit `JSON.parse` | `HttpResponse::stream_json` |

```tsx
import { useEventStream, useJsonStream } from "@laravel/stream-react";

const { message } = useEventStream("/progress");          // gegen einen event_stream-Endpunkt
const { data, send } = useJsonStream<Order[]>("/export"); // gegen einen stream_json-Endpunkt
```

`useStream`/`useJsonStream` senden bei POST zwei Header, die Suprnova wie
jeden anderen Request-Header liest: `X-STREAM-ID` (eine einfache, nicht
authentifizierende Korrelations-ID, die der Hook clientseitig erzeugt)
und `X-CSRF-TOKEN`, gelesen aus `<meta name="csrf-token">`, wie es der
[CSRF-Schutz](csrf.md) bereits erwartet. `useEventStream` sendet keinen
der beiden - `EventSource` kann überhaupt keine benutzerdefinierten
Request-Header setzen und ist ein einfacher Browser-GET.

## Production-Setup

### Response-Header

`HttpResponse::sse(...)` setzt die erforderlichen Header für Sie:

| Header | Wert | Warum |
|--------|-------|-----|
| `Content-Type` | `text/event-stream` | Von der Spec vorgeschrieben; der `EventSource` des Browsers verlangt ihn |
| `Cache-Control` | `no-cache` | Verhindert, dass Zwischenstationen den Stream cachen |
| `Connection` | `keep-alive` | Langlebige HTTP/1.1-Response |
| `X-Accel-Buffering` | `no` | Deaktiviert das nginx-Proxy-Buffering - Events werden sofort geflusht. No-op bei Nicht-nginx |

### Reconnect-Tuning

Die Standard-Reconnect-Verzögerung des Browsers ist 3 Sekunden. Senden
Sie einmal am Anfang des Streams ein `retry:`-Feld, um sie zu
überschreiben:

```rust
let preamble = SseEvent::data("ready").with_retry(Duration::from_secs(5));
```

`Duration::ZERO` ist laut Spec gültig ("sofort neu verbinden") und wird
unverändert emittiert - keine Umwandlung. Für Production-Streams
schlägt eine Wiederholung von 5-15 Sekunden eine Balance zwischen schneller
Erholung und dem Vermeiden, den Server während eines regionalen
Ausfalls zu bombardieren.

### Warum Suprnova abweicht

Laravel liefert SSE als einmaligen Helper auf `Response`:
`Response::eventStream(fn () => ...)` nimmt eine Generator-liefernde
Closure entgegen und rahmt jeden gelieferten Wert als `data:`-Zeile.
Es modelliert `event:` / `id:` / `retry:` nicht als erstklassige
Felder, hat keine eingebaute Keep-alive-Primitive und säubert keine
Werte, die zusätzliche Felder auf dem Wire injizieren würden.

Suprnova behandelt SSE als echtes Subsystem statt als einmaligen
Helper:

- `SseEvent` ist ein typisierter Wert mit fehlbaren (`try_with_*`) und
  unfehlbaren (`with_*`) Buildern, unterscheidbaren `Frame`- und
  `Comment`-Arten und einem dokumentierten Sanitization-Vertrag auf
  jedem einzeiligen Feld.
- `HttpResponse::sse(stream)` klinkt sich in dieselbe
  `stream_bytes`-Body-Pipeline ein, die jede andere langlebige
  Response verwendet, sodass SSE sich einen Cancellation-, Header- und
  Panic-Isolation-Pfad mit dem Rest des Frameworks teilt.
- Produzenten komponieren jeden `Stream<Item = SseEvent>` -
  `tokio::sync::mpsc`, `tokio::sync::broadcast`,
  `futures::stream::iter` oder den Fan-out-Adapter des
  [BroadcastHub](broadcasting.md). Keiner davon braucht einen
  Notausgang des Frameworks.
- Ein `Last-Event-ID`-Reader (`sse::last_event_id`) und die
  WHATWG-NUL-Drop-Regel sind bereits eingebaut, sodass
  Resume-nach-Verbindungsabbruch nur einen Parse-Aufruf entfernt ist,
  statt ein eigenes Header-Utility pro App zu brauchen.

## Referenz

| Symbol | Zweck |
|--------|-------|
| `suprnova::sse::SseEvent` | Ein emittierbares Stück eines SSE-Streams. Zwei Arten - `Frame` (Event mit optionalem `event` / `id` / `retry` + `data`) und `Comment` (Keep-alive). |
| `SseEvent::data(text)` | Baut einen Frame mit nur `data:`-Zeilen. |
| `SseEvent::json(event, &payload)` | Baut einen Frame, dessen Payload `serde_json`-serialisiertes `payload` ist; setzt `event:` auf `event`. Gibt `Result<Self, serde_json::Error>` zurück. |
| `SseEvent::error(message)` | Baut einen Frame mit `event: error` und der übergebenen Nachricht als `data`. |
| `SseEvent::comment(text)` | Baut ein reines Comment-Event (`: <text>\n\n`). Für den Browser unsichtbar; hält Proxys wach. |
| `SseEvent::keep_alive()` | Kurzform für den leeren Comment `:\n\n`. Herzschlag mit minimaler Byte-Zahl. |
| `.with_event(name)` / `.with_id(id)` / `.with_retry(Duration)` | Unfehlbare Builder auf einem `Frame`; stiller No-Op bei einem `Comment`. Entfernen CR / LF / NUL zum Zeitpunkt von `to_wire()` mit einem strukturierten WARN. |
| `.try_with_event(name)` / `.try_with_id(id)` | Fehlbare Geschwister - geben `Err(FrameworkError::validation(...))` bei CR / LF / NUL zurück. Verwenden Sie sie, wenn der Wert aus Nutzereingaben stammt und Sie ein 4xx statt eines stillen Entfernens wollen. |
| `.event()` / `.id()` / `.retry()` / `.payload()` / `.is_comment()` / `.comment_text()` | Accessoren. `payload()` heißt so, um nicht mit dem Konstruktor `data` zu kollidieren. |
| `SseEvent::to_wire()` | Serialisiert in `Bytes` im SSE-Wire-Format. Öffentlich, damit Tests und Adapter kodieren können, ohne den Response-Builder zu durchqueren. |
| `suprnova::sse::last_event_id(&Request) -> Option<String>` | Liest den `Last-Event-ID`-Header. Gibt `None` zurück, wenn er fehlt ODER wenn der Wert ein NUL-Byte enthält (WHATWG verwirft ungültige IDs). |
| `suprnova::sse::last_event_id_from_value(Option<&str>)` | Reiner Helfer, der denselben Validierungsvertrag offenlegt - unit-testbar ohne einen `Request` zu bauen. |
| `HttpResponse::sse(stream)` | Baut eine Streaming-Response aus jedem `Stream<Item = SseEvent> + Send + Sync + 'static`. Setzt `Content-Type`, `Cache-Control`, `Connection`, `X-Accel-Buffering`. |
| `suprnova::sse::StreamedEvent` | Ein Element, das auf ein `event_stream` gepusht wird - `{ event: String, data: serde_json::Value }`. |
| `StreamedEvent::message(data)` / `StreamedEvent::named(event, data)` | Erzeugt mit dem Standardnamen `"update"` oder einem expliziten Namen. Beide liefern `Result<Self, serde_json::Error>`. |
| `suprnova::sse::EndSignal` | Der abschließende Frame, den `event_stream` nach dem Ende des Produzenten sendet - `None` / `Message(String)` / `Event(StreamedEvent)`. `Default` ist `text("</stream>")`. |
| `HttpResponse::event_stream(stream, end)` | Baut eine `event_stream`-Response aus jedem `Stream<Item = StreamedEvent> + Send + Sync + 'static>`. Baut auf `sse` auf. |
| `HttpResponse::stream_json(stream)` | Baut eine `stream_json`-Response aus jedem `Stream<Item = impl Serialize> + Send + Sync + 'static>`. Baut auf `stream_bytes` auf. |

## Nächste Schritte

- [WebSockets](websockets.md) - die andere langlebige Verbindung, wenn Sie bidirektionale oder binäre Frames brauchen.
- [Broadcasting](broadcasting.md) - `BroadcastHub`-Fan-out, geteilt mit WebSocket-Abonnenten.
- [Benachrichtigungen](notifications.md) - Kanal-Treiber für nicht-streamende Push-Zustellung (Mail, Datenbank, Broadcast).
- [Web Push](web-push.md) - vom Server gepushte Benachrichtigungen, die den Client erreichen, wenn kein `EventSource` offen ist.
- [Antworten](responses.md) - der Rest der `HttpResponse`-Builder-Oberfläche.
