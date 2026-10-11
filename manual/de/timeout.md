# Request-Timeouts

`TimeoutMiddleware` legt eine harte Deadline auf jede HTTP-Anfrage.
Ein langsamer Handler - eine hängende Datenbankabfrage, eine nicht
antwortende Upstream-API, eine versehentliche Endlosschleife in einem
Hot-Path - würde sonst eine hyper-Verbindung offen halten, bis der
Client aufgibt oder das Betriebssystem den Prozess tötet. Die
Timeout-Middleware kappt diese Wartezeit, verwirft den in-flight
Handler und gibt `503 Service Unavailable` zurück, damit der Betreiber
den Fehlschlag sieht, statt dass die Anwendung stillschweigend
Verbindungen leakt.

Greifen Sie darauf zurück, wenn Sie irgendetwas bauen, das mit dem
öffentlichen Internet spricht, irgendetwas, das parallel an mehrere
Drittanbieter-APIs verteilt, oder irgendetwas, bei dem „die Datenbank
könnte heute langsam sein“ ein ganz gewöhnlicher Dienstag ist.

```rust
use suprnova::{global_middleware, TimeoutMiddleware};

pub async fn register() {
    // Jede HTTP-Route erhält eine 30-Sekunden-Obergrenze.
    global_middleware!(TimeoutMiddleware::default());
}
```

Diese einzige Zeile gibt der gesamten Anwendung dieselbe
Standard-Obergrenze, die Suprnova für seinen Datenbank-Connect-Timeout
verwendet - einmal festlegen, überall anwenden. Overrides pro Route
sind je eine Zeile. Der Rest dieses Kapitels erklärt genau, was die
Deadline begrenzt, was sie absichtlich nicht begrenzt, und wie sie mit
der Panic-Grenze, Streaming-Antworten und WebSockets zusammenspielt.

## Die Middleware

`TimeoutMiddleware` lebt unter `suprnova::TimeoutMiddleware`. Sie
stellt drei Konstruktoren und einen Accessor bereit:

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

`TimeoutMiddleware::default()` verwendet eine 30-Sekunden-Deadline.
Diese Zahl ist nicht willkürlich - sie stimmt mit `DB_CONNECT_TIMEOUT`
überein (ebenfalls 30s), sodass sich eine Anfrage, die auf eine
brandneue Datenbankverbindung wartet, und eine Anfrage, die im Handler
blockiert, eine Obergrenze teilen. Erhöhen Sie die eine, erhöhen Sie
auch die andere.

`TimeoutMiddleware::seconds(n)` ist die Kurzform für den üblichen Fall
ganzer Sekunden. `TimeoutMiddleware::new(Duration::…)` ist der
Notausgang, wenn Sie Millisekunden-Präzision brauchen (ein interner
Health-Check, der nie mehr als 200ms dauern sollte; eine synthetische
Probe mit einem 50-ms-Budget).

## Global installieren

Ein globaler Timeout ist der richtige Ausgangspunkt: Er gibt jeder
Route eine Obergrenze, ohne dass sich jemand daran erinnern muss, sie
hinzuzufügen. Installieren Sie ihn in `bootstrap.rs` neben Ihrer
übrigen globalen Middleware:

```rust
// src/bootstrap.rs
use suprnova::{
    global_middleware, CorsConfig, CorsMiddleware, DB, RequestIdMiddleware, TimeoutMiddleware,
};
use crate::middleware::LoggingMiddleware;

pub async fn register() {
    DB::init().await.expect("database connect");

    // Die Ausführungsreihenfolge ist wichtig: zuerst request-id (damit
    // Timeout-Logs sie tragen), dann logging (damit langsame Anfragen
    // weiterhin beobachtet werden), dann der Timeout selbst.
    global_middleware!(RequestIdMiddleware);
    global_middleware!(LoggingMiddleware);
    global_middleware!(TimeoutMiddleware::default());

    global_middleware!(CorsMiddleware::new(
        CorsConfig::allow_origins(["https://app.example"]),
    ));
}
```

Die Reihenfolge ist wichtig, weil globale Middleware den Rest der
Chain in Registrierungsreihenfolge umschließt: `RequestIdMiddleware`
läuft auf dem Weg hinein zuerst und auf dem Weg hinaus zuletzt, sodass
die Request-ID im Scope ist, während der Timeout sein `503` feuert.
Den Timeout vor dem Logging zu platzieren würde langsame Anfragen, die
letztlich doch abgeschlossen wurden, aus dem Access-Log verbergen.

## Pro Route verschärfen

Eine globale 30-Sekunden-Obergrenze ist absichtlich großzügig - sie
ist da, um außer Kontrolle geratene Handler abzufangen, nicht um SLAs
durchzusetzen. Wenn ein bestimmter Endpunkt schneller fehlschlagen
soll, hängen Sie einen Timeout pro Route an:

```rust
use suprnova::{Router, TimeoutMiddleware};

Router::new()
    // Öffentlicher Report-Endpunkt: muss in 5s antworten, sonst lieber 503
    // und den Client erneut versuchen lassen, statt zu blockieren.
    .get("/report", controllers::report::show)
    .middleware(TimeoutMiddleware::seconds(5));
```

Sie können auch einer Routengruppe einen strengeren Timeout anhängen.
Das ist die typische Form für eine öffentliche API, bei der jede
Anfrage schnell sein soll, während der Rest der App den
30-Sekunden-Standard behält:

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

### Global ist eine Obergrenze; pro Route kann nur verschärft werden

Globale Middleware läuft **außerhalb** der Routen-Middleware. Die
Chain umschließt von innen nach außen:

```
Globaler Timeout (30s) → Timeout der Route (3s) → Handler
```

Beide `tokio::time::timeout`-Futures sind aktiviert; das innere feuert
zuerst, weil es die kürzere Deadline hat. Ein Timeout pro Route kann
eine Route also nur *strenger* machen als die globale, niemals
lockerer.

Wenn ein einzelner Endpunkt legitim *länger* laufen muss als der
globale Standard - ein langsamer Report, ein großer Upload, ein
Long-Poll-Fallback - haben Sie zwei Optionen:

1. Den globalen Wert erhöhen. Am einfachsten, lockert aber die Obergrenze auch für jede andere Route.
2. Die globale Middleware auf eine Routengruppe beschränken, die den langen Endpunkt *ausschließt*, und der langsamen Route einen separaten Timeout (oder keinen) anhängen. Das behält den strengen Standard überall sonst bei.

Die zweite Option ist die richtige Form für einen einzelnen Ausreißer;
die erste ist richtig, wenn die gesamte Arbeitsklasse mehr Raum
braucht.

## Was die Deadline tatsächlich begrenzt

Die Deadline tritt gegen das von `next(request)` zurückgegebene Future
an. Dieses Future löst sich in dem Moment auf, in dem Ihr Handler
seine `HttpResponse` zurückgibt - nicht wenn der Body das Streaming
beendet. Diese Unterscheidung ist tragend:

- **Normale Handler** bauen ihren vollständigen Body auf, bevor sie zurückkehren, sodass die Deadline effektiv die gesamte Handler-Zeit begrenzt. Ein Handler, der eine JSON-Liste serialisiert, eine Inertia-Seite rendert oder eine HTML-Antwort zusammensetzt, hält das Future, bis die Arbeit fertig ist.
- **Streaming-Antworten** (`HttpResponse::sse(...)`, `HttpResponse::stream_bytes(...)`) kehren *sofort* mit einem lazy Body zurück. Die Middleware-Chain ist bereits abgeschlossen, bevor hyper beginnt, Bytes vom Stream zu ziehen, sodass die Deadline die Lebensdauer des Bodys nie beobachtet. Ein SSE-Event-Stream kann absichtlich stundenlang unter einem 30-Sekunden-Timeout offen bleiben - siehe [Server-Sent Events](sse.md) für das Streaming-Modell.
- **WebSocket-Upgrades** werden explizit übersprungen. Siehe den nächsten Abschnitt.

Das ist mit ziemlicher Sicherheit das Verhalten, das Sie wollen.
Hätten Sie einen langlebigen SSE-Stream in einen 30-Sekunden-Timeout
gehüllt, würde das Framework die Verbindung alle 30 Sekunden mitten im
Stream abreißen, und das Feature wäre unbrauchbar.

## WebSocket-Sonderfall

Die Middleware inspiziert die Anfrage, bevor sie die Deadline
aktiviert:

```rust
if is_websocket_upgrade(request.headers()) {
    return next(request).await;
}
```

Jede Anfrage, die `Upgrade: websocket` trägt, überspringt den Timeout
vollständig. Die Prüfung ist bei dem Token-Wert case-insensitive
(`WebSocket`, `websocket`, `WEBSOCKET` passen alle), und ein bloßes
`Connection: upgrade` ohne `Upgrade: websocket` wird *nicht* als
WS-Upgrade behandelt - das durchläuft den Timeout normal.

Heute nehmen WebSocket-Upgrades einen separaten Server-Pfad, der
überhaupt keine globale Middleware durchläuft, daher ist diese
Absicherung eine zusätzliche Verteidigungsebene - sie verhindert, dass
der Timeout jemals einen langlebigen bidirektionalen Kanal begrenzt,
an dem Tag, an dem sich das ändert. Siehe [WebSockets](websockets.md)
dafür, wie Upgrades dispatcht werden, und für die Lebensdauer eines
verbundenen Sockets.

## Was bei der Deadline passiert

Wenn `tokio::time::timeout` abläuft, bevor der Handler abschließt, tut
die Middleware der Reihe nach drei Dinge:

1. **Verwirft das in-flight Handler-Future.** Das Future wurde innerhalb des `timeout`-Kombinators gepollt; der Kombinator gibt `Err(Elapsed)` zurück, und das Future wird dort verworfen, wo es zuletzt suspendiert war.
2. **Protokolliert eine Warnung** mit dem Routen-Pfad und der Timeout-Dauer in Millisekunden:

   ```
   WARN suprnova::timeout request exceeded its timeout; returning 503 Service Unavailable
       route=/report timeout_ms=5000
   ```

   Das Log liegt auf `WARN`, sodass es standardmäßig in
   Betreiber-Dashboards auftaucht, getrennt von `INFO`-Access-Logs
   normaler Anfragen.
3. **Gibt `503 Service Unavailable`** mit einem Plain-Text-Body zurück:

   ```
   HTTP/1.1 503 Service Unavailable
   Content-Type: text/plain
   Content-Length: 42

   Service Unavailable: request timed out
   ```

Das 503 ist in `Err(HttpResponse::…)` verpackt, sodass es den Rest der
Chain kurzschließt, genau wie jede andere von Middleware
zurückgewiesene Anfrage. Äußere Middleware (logging, request-id, CORS)
führt ihre Post-Handler-Seite weiterhin aus, sodass die Antwort mit
den korrekten Headern hinausgeht.

### Warum 503 und nicht 504

`504 Gateway Timeout` ist der richtige Code, wenn *Sie* das Gateway
sind und ein *Upstream* einen Timeout hatte. `503 Service Unavailable`
ist der richtige Code, wenn *dieser* Dienst die Antwort nicht
rechtzeitig produzieren konnte. Die Timeout-Middleware begrenzt *Ihren
eigenen* Handler, daher gibt sie 503 zurück. Wenn Sie eine andere Form
wollen - einen JSON-Body, einen anderen Status, einen
maschinenlesbaren Code - hüllen Sie Ihre eigene äußere Middleware um
den Timeout und übersetzen Sie dessen 503-Antwort.

## Abbruchsicherheit

Wenn die Deadline abläuft, wird das Handler-Future an seinem aktuellen
`.await`-Punkt **verworfen**. Das ist normaler Tokio-Abbruch; dasselbe
passiert, wenn ein Client die Verbindung mitten in der Anfrage
schließt. Alles, was über die Await-Grenze hinweg gehalten wird, wird
durch seine `Drop`-Implementierung freigegeben:

- **Datenbank-Transaktionen** werden zurückgerollt. Ein SeaORM-`DatabaseTransaction` hat eine `Drop`-Implementierung, die auf der zugrunde liegenden Verbindung `ROLLBACK` ausgibt.
- **Mutex- und RwLock-Guards** werden freigegeben. Ein Guard der Standardbibliothek oder von `parking_lot` gibt beim Drop frei; ein anderer Wartender kann ihn sofort übernehmen.
- **Datei-Handles** schließen. Der Deskriptor auf OS-Ebene wird freigegeben, wenn das `tokio::fs::File` gedroppt wird.
- **Netzwerkverbindungen** kehren in den Pool zurück oder schließen, abhängig vom Drop-Verhalten des Pools.

Das Ergebnis ist, dass ein Handler mit abgelaufenem Timeout nichts
hängen lässt - der Betreiber sieht das 503, die Datenbank sieht das
Rollback, die nächste Anfrage sieht einen sauberen Pool.

### Was *nicht* abgebrochen wird

Alles, was Sie mit `tokio::spawn` aus der Anfrage herausbewegt haben,
ist **losgelöst**. Gespawnte Tasks leben auf der Runtime, nicht am
Request-Future, sodass das Verwerfen der Anfrage sie nicht stoppt. Das
ist relevant, wenn Sie so etwas geschrieben haben:

```rust
pub async fn webhook(req: Request) -> Response {
    let payload: WebhookPayload = req.json().await?;

    // Fire-and-forget-Hintergrundarbeit. Überlebt das Timeout der Anfrage.
    tokio::spawn(async move {
        if let Err(e) = process_webhook(payload).await {
            tracing::error!("webhook processing failed: {e}");
        }
    });

    Ok(HttpResponse::new().status(204))
}
```

Läuft die Anfrage *vor* der `spawn`-Zeile in den Timeout, passiert der
Spawn nie. Läuft die Anfrage *nach* dem Spawn in den Timeout, läuft
die Hintergrund-Task weiter - sie wird nicht mit der Anfrage
abgebrochen. Das ist fast immer das, was Sie für Arbeit im
Webhook-Stil wollen, bedeutet aber, dass Cleanup nach einem langen
`.await` innerhalb des Handlers **nicht** garantiert ausgeführt wird:

```rust
pub async fn upload(req: Request) -> Response {
    let temp_path = save_to_temp(&req).await?;

    // Läuft dies in den Timeout, wird das Cleanup unten NICHT AUSGEFÜHRT.
    let processed = long_running_processing(&temp_path).await?;

    // Unter einem Timeout nicht garantiert.
    tokio::fs::remove_file(&temp_path).await?;

    Ok(HttpResponse::json(serde_json::to_value(&processed)?))
}
```

Die Lösung ist, RAII zu verwenden. Hüllen Sie die temporäre Datei in
eine Struktur, deren `Drop`-Implementierung sie entfernt; dann läuft
das Cleanup, ob der Handler zurückkehrt, einen Fehler zurückgibt oder
mitten im `.await` durch den Timeout verworfen wird. Das ist dieselbe
Disziplin, die Sie für jede Abbruchquelle anwenden würden -
Client-Disconnect, Runtime-Shutdown, Panic-Recovery.

## Zusammenspiel mit der Panic-Grenze

Der Suprnova-Server hüllt die gesamte Middleware-Chain in
[`execute_chain_safely`](lifecycle.md), das
`AssertUnwindSafe(...).catch_unwind()` verwendet, um Panics in ein
bereinigtes `500 Internal Server Error` zu übersetzen. Eine Anfrage
mit abgelaufenem Timeout ist **kein** Panic - das Future wird sauber
verworfen -, sodass das `503` des Timeouts hinausgeht, ohne die
Panic-Grenze überhaupt zu berühren.

Die beiden Grenzen behandeln unterschiedliche Fehlermodi:

| Fehlschlag | Grenze | Status | Body |
|---|---|---|---|
| Handler `.await` überschreitet die Deadline | `TimeoutMiddleware` | `503` | `Service Unavailable: request timed out` |
| Handler gerät in Panic (`.unwrap()` auf `None` usw.) | `execute_chain_safely` | `500` | `{"message": "Internal Server Error"}` |
| Handler gibt `Err(HttpResponse)` zurück | normaler `Response`-Fluss | was auch immer der Handler gesetzt hat | was auch immer der Handler gesetzt hat |

Sie müssen sich nicht entscheiden - beide Grenzen sind immer
installiert. Ein Handler, der *nach* Überschreiten seines Timeouts in
Panic gerät, erzeugt trotzdem ein 503 (das Future wurde verworfen,
bevor der Panic passieren konnte). Ein Handler, der *vor* dem
Überschreiten seines Timeouts in Panic gerät, erzeugt ein 500.

## Betriebs-Tuning

Drei Überlegungen bei der Wahl von Timeout-Werten:

1. **Auf Ihren Datenbank-Connect-Timeout abstimmen.** Ist `DB_CONNECT_TIMEOUT=30` (der Standard), feuert ein Request-Timeout unter 30s, bevor ein langsamer Connect je abschließt - der Nutzer sieht `503` statt der Chance, sich zu erholen. Erhöhen Sie entweder den Connect-Timeout oder akzeptieren Sie, dass „30s“ der Boden ist.
2. **Den langsamsten legitimen Handler berücksichtigen.** Sehen Sie sich ein Histogramm Ihrer Request-Dauern auf `INFO`-Ebene an. Das p99 des langsamen Ausläufers sollte bequem unter dem Timeout liegen, mit Spielraum für Uhren-Drift und Event-Loop-Jitter. Ein Timeout, der bei gesundem Traffic routinemäßig feuert, ist eine Fehlkonfiguration, kein Feature.
3. **Timeouts pro Route sind Observability.** `TimeoutMiddleware::seconds(3)` auf `/api/*` zu verschärfen verwandelt eine degradierte API in einen sichtbaren Alarm (Logs voller WARN, 503er im Load Balancer) statt in ein schleichendes Latenzproblem. Verwenden Sie sie dort, wo Sie eine SLA haben und beim Verfehlen einen harten Fehlschlag wollen.

Die eigenen Integrationstests des Frameworks verwenden Dauern im
Millisekundenbereich
(`TimeoutMiddleware::new(Duration::from_millis(50))`), um die Deadline
deterministisch zu testen. Produktions-Deadlines liegen fast immer in
ganzen Sekunden.

### Warum Suprnova abweicht

In einem Laravel + PHP-FPM-Deployment leben Request-Timeouts außerhalb
der Anwendung: nginx' `proxy_read_timeout`, PHP-FPMs
`request_terminate_timeout`, der Idle-Timeout des Load Balancers. Der
PHP-Prozess wird getötet, wenn das Budget erschöpft ist, und jeder
offene Zustand - Datenbankverbindungen, Datei-Handles - leakt, bis die
nächste Anfrage den Worker wiederverwendet.

Suprnova begrenzt die Anfrage innerhalb der Anwendung, weil es das
kann. Der Handler ist ein Tokio-Future, kein PHP-Prozess, daher lässt
sein Verwerfen `Drop`-Implementierungen sauber laufen: Transaktionen
rollen zurück, Sperren werden freigegeben, Deskriptoren schließen, der
Connection-Pool bleibt gesund. Das 503 geht auch *als echte
HTTP-Antwort* hinaus - Clients sehen einen ordentlichen Statuscode
statt eines Upstream-Resets.

Das ist auch, warum die Middleware nicht versucht, eine
Tower-`Timeout`-Layer zu sein. Towers Layer ist generisch über jeden
Tokio-Service und gibt `tower::timeout::error::Elapsed` zurück, was
Aufrufer dann selbst auf einen HTTP-Status abbilden müssen. Die
Suprnova-Middleware weiß, dass sie eine HTTP-Request-Pipeline umhüllt;
sie gibt `503` direkt zurück, protokolliert die betroffene Route und
respektiert die WebSocket- und Streaming-Sonderfälle des Frameworks,
ohne dass der Aufrufer darüber nachdenken muss. Die Tower-Layer ist
das richtige Primitiv für einen generischen Tokio-Service; für eine
HTTP-Anfrage ist das hier die richtige Form.

## Nächste Schritte

- [Middleware](middleware.md) - der Trait, die Chain, globale vs. Pro-Route-Registrierung, terminierbare Hooks
- [Request-Lifecycle](lifecycle.md) - wo der Timeout in der Chain sitzt und wie `execute_chain_safely` mit Panics umgeht
- [Server-Sent Events](sse.md) - das Streaming-Antwortmodell, das der Timeout absichtlich nicht begrenzt
- [WebSockets](websockets.md) - der Upgrade-Pfad, der den Timeout vollständig umgeht
- [Fehlerbehandlung](errors.md) - wie 5xx-Antworten als `ErrorOccurred`-Events für Observability dispatcht werden
