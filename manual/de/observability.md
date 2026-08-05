# Beobachtbarkeit

Das Framework liefert drei für Betreiber sichtbare Signal-Schichten:
strukturierte Logs (immer aktiv), Korrelation über die Request-ID
(immer aktiv, propagiert in gespawnte Tasks) und eine optionale
OpenTelemetry-Brücke, die jeden `tracing`-Span in einen exportierten
OTel-Span verwandelt. Dasselbe `#[tracing::instrument]`, das Sie für
lokale Logs schreiben würden, wird zu einem Distributed-Trace-Span,
wenn das OTel-Feature aktiv ist - keine zweite Instrumentierungs-API.

```rust
use suprnova::telemetry::{init_telemetry, OtelConfig};
use suprnova::logging::LogConfig;

#[suprnova::main]
async fn main() {
    let guard = init_telemetry(LogConfig::from_env(), OtelConfig::from_env());

    // ... die App laufen lassen ...

    // Gepufferte Telemetrie vor dem Beenden flushen. Die OTel-Batch-
    // Prozessoren halten Spans/Metriken/Logs im Speicher; wird der
    // Guard ohne `shutdown` gedroppt, geht alles verloren, was noch
    // nicht exportiert wurde.
    guard.shutdown().await;
}
```

Der `Server` einer gescaffoldeten App ruft `init_telemetry` bereits für
Sie auf und flusht den Guard beim Shutdown-Signal - Sie verdrahten das
nur von Hand, wenn Sie Suprnova in Ihre eigene Runtime einbetten.

## Die drei Schichten

| Schicht | Immer aktiv | Was sie bietet |
|---|---|---|
| Strukturiertes Logging (`tracing`) | Ja | Stdout-Logs im Format `pretty` (Entwicklung) oder `json` (Produktion), umgebungsbewusst |
| Korrelation über die Request-ID | Ja | Pro-Request-ID, gescoped über ein `tokio::task_local!`, gespiegelt auf `X-Request-Id`, propagiert in `spawn_with_request_id`-Tasks |
| OpenTelemetry-Export | `otel`-Feature + Collector-Endpunkt | OTLP-HTTP/proto-Export von Traces, Metriken und Logs; W3C-`traceparent`-Propagation in beide Richtungen |

Die OTel-Schicht ist **zur Compile-Zeit optional**, sodass Standard-Builds
keine OpenTelemetry-Abhängigkeiten tragen und die
[`Metrics`](#metriken)-Facade zu wirkungslosen No-Ops kompiliert. Ist das
Feature aus, werden "Trace" und "Metrik-Export" stillschweigend zu
No-Ops - Ihre Logs funktionieren weiterhin.

### Warum Suprnova abweicht

Laravels Beobachtbarkeits-Geschichte teilt sich zwischen
Framework-internen Events (`QueryExecuted`, `MessageSent`,
`JobProcessed`) und Laufzeit-Belangen, die an PHP-Extensions
(OpenTelemetry, Sentry, New Relic) delegiert werden, welche auf der
FPM-Ebene eingehängt werden. Die Event-Oberfläche ist reichhaltig; die
Laufzeit-Oberfläche lautet "installieren Sie die Extension, die Ihr
APM-Anbieter braucht."

Suprnova ist ein einziger asynchroner Prozess und besitzt daher beide
Hälften selbst. Die Event-Oberfläche ist gleichwertig (dieselbe
`QueryExecuted`-/`NotificationSent`-/`ErrorOccurred`-Form), und die
Laufzeit-Oberfläche ist eine `tracing`-→-OpenTelemetry-Brücke innerhalb
des Frameworks. Sie installieren keine Extension; Sie schalten ein
Feature-Flag um, und dieselben Spans, die Sie bereits ausgeben, werden
zu OTel exportiert.

## Strukturiertes Logging

`LogConfig::from_env()` liest zwei Env-Variablen:

| Variable | Standard | Hinweise |
|---|---|---|
| `LOG_LEVEL` | `"info"` | Env-Filter-Syntax von `tracing-subscriber` (z. B. `"debug,sqlx=warn,hyper=warn"`) |
| `LOG_FORMAT` | umgebungsbewusst | `"json"` in Produktion, `"pretty"` überall sonst; ein expliziter Wert gewinnt immer |

Der Standard fürs Format wird über `Environment::detect()` aus
`APP_ENV` ermittelt: Ein Produktions-Deployment bekommt standardmäßig
ein JSON-Objekt pro Zeile für Log-Aggregatoren, lokale/Dev-Läufe
bekommen menschenlesbare mehrzeilige Ausgabe. Ein explizites
`LOG_FORMAT=pretty` überschreibt den Produktions-Standard, wenn Sie
rohes Stdout in Produktion wollen.

```bash
# Lokale Entwicklung - explizite Overrides gewinnen
LOG_LEVEL=debug,sqlx=warn,hyper=warn LOG_FORMAT=pretty cargo run

# Produktion - APP_ENV=production kippt den Format-Standard auf json
APP_ENV=production LOG_LEVEL=info cargo run --release
```

Eine fehlerhafte `LOG_LEVEL`-Direktive lässt den Boot nicht abstürzen -
sie fällt auf `"info"` zurück und druckt eine einzeilige Warnung nach
stderr, damit die Fehlkonfiguration für den Betreiber sichtbar ist.

### Span-Kontext in jeder Zeile

Jede geroutete HTTP-Anfrage läuft innerhalb eines `request`-Spans, den
die äußerste Middleware des Frameworks anlegt. Der Span trägt drei
Felder - `request_id`, `method`, `path` - und der JSON-Formatter
verschachtelt sie unter `span` in jedem Event, das innerhalb der
Anfrage ausgegeben wird. Ihr Anwendungscode muss die ID nicht in jeder
Zeile lesen oder mitschreiben; der Span trägt sie implizit:

```rust
use tracing::info;

pub async fn show(req: suprnova::Request) -> suprnova::Response {
    info!(user_id = 42, "loaded dashboard");
    // Die JSON-Zeile trägt span.request_id / span.method / span.path,
    // ohne dass die Aufrufstelle irgendetwas durchfädeln müsste.
    Ok(suprnova::json_response!({ "ok": true }))
}
```

## Korrelation über die Request-ID

Jede Anfrage bekommt eine 36 Zeichen lange, kleingeschriebene UUID
v4-ID, gescoped über ein `tokio::task_local!`. Die Middleware
verwendet einen eingehenden `X-Request-Id` weiter, wenn der
Header-Wert eine strikte Sicherheitsprüfung besteht (ASCII
alphanumerisch plus `-_.:`, max. 128 Bytes); alles außerhalb dieses
Zeichensatzes wird abgelehnt und durch eine frische UUID ersetzt,
damit ein Angreifer keine Steuerzeichen in die Log-Ausgabe einschleusen
oder nachgelagerte Pipelines aufblähen kann.

Dieselbe ID wird auf **jeder** Response - Erfolg, Fehler und
Panic-Recovery - als `X-Request-Id`-Header zurückgespiegelt, sodass ein
Frontend oder ein vorgelagerter Dienst sie in Bug-Reports aufnehmen
kann und Betreiber im strukturierten Log nach ihr grep-en können.

### Die ID lesen

```rust
use suprnova::{current_request_id, spawn_with_request_id};

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    // Innerhalb einer Anfrage ist die ID immer vorhanden.
    let id = current_request_id().expect("inside a request");
    tracing::info!(request_id = %id, "checkout starting");

    // Hintergrundarbeit, aus einem Handler gespawnt. `tokio::spawn`
    // startet eine Task mit leeren Task-Locals - das gespawnte Future
    // würde die Request-ID ohne Hilfe verlieren. `spawn_with_request_id`
    // erfasst die ID des Aufrufers und scoped sie erneut für das
    // gespawnte Future, und hängt den aktuellen `tracing`-Span an,
    // sodass die Events der Task `request_id` genauso erben wie
    // Events innerhalb der Anfrage.
    spawn_with_request_id(async move {
        // Diese Log-Zeile trägt die ID der ursprünglichen Anfrage.
        tracing::info!("post-checkout fanout running");
    });

    Ok(suprnova::ok!())
}
```

`current_request_id()` liefert außerhalb einer Anfrage `None` -
Hintergrund-Jobs, geplante Tasks und Tests ohne die Middleware sehen
keine ID, und der Helfer erfindet keine. `spawn_with_request_id`
außerhalb eines Request-Scopes ist exakt `tokio::spawn`; es passiert
nichts Magisches.

### Wo die ID sonst noch verfügbar ist

| Oberfläche | Wie |
|---|---|
| `tracing`-Events | `span.request_id` auf jeder Zeile innerhalb der Anfrage |
| Response-Header | `X-Request-Id` bei Erfolg, Fehler und panic-recovered Responses |
| `Context`-Bag | `Context::get("_request_id")` - lesbar aus Observern, Listenern, Jobs, die `Context` konsultieren |
| Gespawnte Tasks | `current_request_id()` nach `spawn_with_request_id` |

## Eingebaute Events für Beobachtbarkeit

Das Framework dispatcht typisierte Events an den Stellen, an denen ein
Betreiber üblicherweise instrumentieren möchte. Jedes ist ein
`suprnova::Event`, auf das Sie über `EventFacade::listen::<E, _>(...)`
`listen` können und an Sentry, Datadog, Slack oder Ihre
Metrik-Pipeline versenden. Alle laufen über `dispatch_best_effort`, ein
fehlschlagender Listener bricht also nicht die Anfrage ab, die ihn
ausgelöst hat.

| Event | Wann es feuert | Trägt |
|---|---|---|
| `ErrorOccurred` | Jede `FrameworkError`-→-5xx-Umwandlung (einschließlich Panic-Recovery) | Fehlerkontext + Request-ID |
| `QueryExecuted` | Jede Query, die durch die instrumentierten Executor-Helfer geroutet wird | SQL, Bindings, Dauer, Connection, Read-/Write-Klassifikation, Ergebnis |
| `ConnectionEstablished` | `DbConnection::connect` erfolgreich | Connection-Name |
| `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` | Closure-Form `DB::transaction` + manuelle Handles | Connection-Name |
| `NotificationSending` / `NotificationSent` / `NotificationFailed` | Pro-Kanal vorher/nachher/Fehler von `Notification::send` | Notification + Kanal + Empfänger |

`ErrorOccurred` ist der Hook zum Versenden von 5xx-Ausnahmen;
`QueryExecuted` ist der Hook für Slow-Query-Alerts; das
Notification-Trio ist der Hook für Zustellungs-Dashboards. Siehe
[Ereignisse](events.md) für die Listener-API und
[Lifecycle](lifecycle.md) dafür, wo im Request-Pfad jedes Event
feuert.

### Direkte DB-Query-Beobachtung

`DB::listen` ist ein zweiter, synchroner Hook, speziell auf
`QueryExecuted` zugeschnitten. Er feuert inline innerhalb des
Executors, ein langsamer Listener verlangsamt also die Query - halten
Sie ihn leichtgewichtig. Der Dispatcher-Pfad
(`EventFacade::listen::<QueryExecuted, _>`) läuft alle Listener
best-effort durch und toleriert Fehler; bevorzugen Sie ihn für alles,
was fehlschlagen kann.

```rust
use suprnova::DB;

// In bootstrap.rs:
DB::listen(|q| {
    if q.time > std::time::Duration::from_millis(100) {
        tracing::warn!(
            sql = %q.sql,
            ms = q.time.as_millis(),
            "slow query"
        );
    }
})?;
```

Ein Listener, der selbst eine Datenbank-Query ausführt, löst
`QueryExecuted` für den verschachtelten Aufruf **nicht** erneut aus -
ein Task-Local-Wiedereintritts-Guard verhindert die Schleife
"Log-zu-DB-Listener → gibt Event aus → Log-zu-DB → ...".

### Ein Query-Log für Tests/Debugging aufzeichnen

Für Test-Assertions oder einmaliges "was lief während dieses Blocks?"-
Debugging:

```rust
use suprnova::DB;

DB::enable_query_log()?;
// ... den zu untersuchenden Code ausführen ...
let queries = DB::get_query_log()?;
for q in &queries {
    println!("{:>4}ms  {}", q.time.as_millis(), q.to_raw_sql());
}
DB::disable_query_log()?;
DB::flush_query_log()?;
```

Der Puffer ist **unbegrenzt** - jede erfasste Query lässt ihn wachsen.
Verwenden Sie ihn für Tests und einmalige Untersuchungen; flushen Sie
ihn regelmäßig, wenn Sie ihn in Produktion eingeschaltet lassen.

## Distributed Tracing (OTel)

Fügen Sie das `otel`-Feature hinzu, um es zu aktivieren:

```toml
[dependencies]
suprnova = { git = "...", features = ["otel"] }
```

Konfigurieren Sie über die Standard-OTel-Umgebungsvariablen:

```bash
# Minimum: wo der Collector lebt.
OTEL_EXPORTER_OTLP_ENDPOINT=http://localhost:4318
OTEL_SERVICE_NAME=my-app          # Standard ist "suprnova"
OTEL_SERVICE_VERSION=1.4.2        # Standard ist Ihre Crate-Version
```

Telemetrie ist nur **aktiviert**, wenn `OTEL_EXPORTER_OTLP_ENDPOINT`
gesetzt ist **und** der Kill-Switch `OTEL_SDK_DISABLED` nicht an ist.
Ohne Endpunkt läuft die Logging-Schicht allein, und der zurückgegebene
Guard hält keine Provider, sodass ihn ohne `shutdown()` zu droppen
lautlos bleibt (keine unnötige "gepufferte Telemetrie könnte verloren
gehen"-Warnung bei jedem Testprozess).

### Trace-Kontext verbindet sich automatisch

**Inbound.** Trifft eine Anfrage mit einem W3C-
[`traceparent`](https://www.w3.org/TR/trace-context/)-Header ein - sie
wurde also von einem anderen getracten Dienst gestellt -, extrahiert
die Middleware diesen Kontext und hängt den Request-Span als Kind an
den Span des Aufrufers. Ihr Server-Span erscheint als Kind im
**selben** Distributed Trace, nicht als frischer Root. Eine Anfrage
ohne `traceparent` (ein direkter Browser-Zugriff) bleibt ein sauberer
Root-Span.

**Outbound.** Der HTTP-Client des Frameworks
([`Http`](http-client.md)) injiziert den aktiven Trace-Kontext als
`traceparent` auf jedem ausgehenden Aufruf, sodass der nachgelagerte
Dienst denselben Trace fortsetzt.

Zusammen ergibt `vorgelagerter Dienst → Ihr Handler → nachgelagerter
Dienst` einen einzigen zusammenhängenden Trace, ohne manuelle
Span-Verdrahtung in Ihren Handlern.

**Fehlerstatus.** Gibt ein Handler ein 5xx zurück, wird der
Request-Span als fehlerhaft markiert, sodass das OTel-Backend
`Status::Error` zeigt. (Ein *Panic* im Handler wird abgefangen und in
ein 500 mit einem Log auf Error-Level und einem `ErrorOccurred`-Event
umgewandelt, aber der OTel-Span-Status wird auf diesem Pfad nicht
gesetzt - der Panic wickelt das Future des Spans ab, bevor der Marker
läuft.)

### Eigene Spans hinzufügen

Weil die Brücke jeden `tracing`-Span in einen OTel-Span verwandelt,
instrumentieren Sie mit purem `tracing` - keine OTel-spezifische API
in Ihrem Code:

```rust
use suprnova::DatabaseConnection;

#[tracing::instrument(skip(db))]
async fn load_dashboard(db: &DatabaseConnection, user_id: i64) -> anyhow::Result<()> {
    // Dieser Span verschachtelt sich automatisch unter den
    // Request-Span und exportiert zu Ihrem Collector, wenn das
    // `otel`-Feature aktiv ist.
    Ok(())
}
```

### Von Suprnova gelesene Umgebungsvariablen

| Variable | Wirkung |
|---|---|
| `OTEL_EXPORTER_OTLP_ENDPOINT` | Basis-URL des Collectors. Nicht gesetzt → Telemetrie deaktiviert. |
| `OTEL_SERVICE_NAME` | Ressourcen-Attribut `service.name` (Standard `"suprnova"`). |
| `OTEL_SERVICE_VERSION` | Ressourcen-Attribut `service.version` (Standard: Crate-Version). |
| `OTEL_SDK_DISABLED` | Kill-Switch. `true` oder `1`, ohne Berücksichtigung von Groß-/Kleinschreibung, deaktiviert den Export selbst bei gesetztem Endpunkt. |

Die übrigen Standard-OTLP-Regler werden vom SDK selbst gelesen,
konfigurieren Sie sie also auf die normale Art:

| Variable | Gelesen von |
|---|---|
| `OTEL_EXPORTER_OTLP_HEADERS` | Exporter (Collector-Auth, z. B. `Authorization=Bearer ...`) |
| `OTEL_EXPORTER_OTLP_PROTOCOL` | Exporter (`http/protobuf` usw.) |
| `OTEL_EXPORTER_OTLP_TIMEOUT` | Exporter |
| `OTEL_EXPORTER_OTLP_COMPRESSION` | Exporter |

Pro-Signal-Endpunkt-Overrides (`OTEL_EXPORTER_OTLP_TRACES_ENDPOINT`,
`_METRICS_ENDPOINT`, `_LOGS_ENDPOINT`) werden derzeit vom Basis-Endpunkt
überschattet - alle drei Signale gehen an
`OTEL_EXPORTER_OTLP_ENDPOINT`. Wenn Sie Signale zu unterschiedlichen
Collectors fächern müssen, betreiben Sie einen lokalen Collector, der
sie weiterleitet.

## Metriken

`Metrics` ist die Facade für Counter, Histogramme und Gauges. Handles
sind billig zu klonen und lösen den globalen Meter bei jeder
Konstruktion neu auf:

```rust
use suprnova::telemetry::Metrics;

// Counter - monoton.
let signups = Metrics::counter("user.signups");
signups.inc();                                  // +1
signups.inc_by(3);                              // +3
signups.inc_with(&[("plan", "pro")]);           // +1 mit einem Label

// Histogramm - Verteilungen (Latenz, Größen).
let latency = Metrics::histogram("request.latency_ms");
latency.record(42.0);
latency.record_with(42.0, &[("route", "/checkout")]);

// Gauge - Punkt-in-Zeit-Wert.
let queue_depth = Metrics::gauge("jobs.pending");
queue_depth.set(17.0);
queue_depth.set_with(17.0, &[("queue", "emails")]);
```

Ohne das `otel`-Feature ist jeder Aufruf oben ein No-Op ohne jede
Allokation - lassen Sie Instrumentierung in Hot-Paths stehen und
zahlen Sie nichts dafür in Standard-Builds.

Metrik-Handles binden sich an denjenigen Meter-Provider, der aktiv
ist, wenn das zugrunde liegende Instrument zuerst aufgelöst wird.
Erzeugen Sie Handles **nachdem** `init_telemetry` gelaufen ist (oder
lazy bei der ersten Verwendung) - ein Handle, das vor der
Initialisierung konstruiert wird, löst sich gegen den No-Op-Provider
auf und bleibt wirkungslos. Das idiomatische Muster ist ein
`once_cell`-/`LazyLock`-Handle, aufgelöst bei der ersten Emission,
deutlich nach dem Boot.

Attributwerte sind string-typisiert (`&[(&'static str, &str)]`).
Numerische und boolesche Attribute sind eine geplante Erweiterung;
formatieren Sie sie vorerst als Strings an der Aufrufstelle.

Benennung: stabil, ASCII, punktgetrennt (z. B.
`"http.requests.total"`, `"http.request.duration"`). Die
Standard-OTel-Semantic-Conventions liegen in
`opentelemetry-semantic-conventions::metric::*`.

## Der Shutdown-Vertrag

`init_telemetry` liefert einen `TelemetryGuard`, der die
SDK-Provider-Handles besitzt. Die OTel-Batch-Prozessoren puffern
Spans/Metriken/Logs im Speicher und flushen asynchron; Sie müssen
also `guard.shutdown().await` aufrufen, bevor der Prozess beendet
wird, sonst verlieren Sie, was noch gepuffert ist.

- Der Aufruf von `shutdown()` flusht und ist ungefährlich, einmal
  aufzurufen (er nimmt `self` entgegen).
- Wird der Guard **ohne** `shutdown()` gedroppt, protokolliert das
  eine Warnung - aber nur, wenn der Guard tatsächlich Provider hält.
  Ein Lauf mit deaktivierter Telemetrie (kein Endpunkt, oder
  `OTEL_SDK_DISABLED`, oder ein Build ohne `otel`) gibt einen
  providerlosen Guard zurück, dessen Drop lautlos bleibt, sodass
  Dev- und Testläufe ohne Collector nicht zugespammt werden.

## Zusammenfassung

| Aufgabe | API |
|---|---|
| OTel aktivieren | `features = ["otel"]` + `OTEL_EXPORTER_OTLP_ENDPOINT` |
| Initialisieren | `init_telemetry(LogConfig::from_env(), OtelConfig::from_env())` |
| Beim Beenden flushen | `guard.shutdown().await` |
| Zur Laufzeit deaktivieren | `OTEL_SDK_DISABLED=true` |
| Eigener Span | `#[tracing::instrument]` (automatisch zu OTel gebrückt) |
| Counter / Histogramm / Gauge | `Metrics::counter/histogram/gauge(name)` |
| Distributed-Trace-Beitritt | Automatisch - inbound `traceparent` extrahiert, outbound injiziert |
| Aktuelle Request-ID lesen | `current_request_id()` |
| ID in einen Spawn propagieren | `spawn_with_request_id(future)` |
| Synchroner Query-Observer | `DB::listen(|q| { ... })` |
| Best-effort-Query-Observer | `EventFacade::listen::<QueryExecuted, _>(...)` |
| Queries für Tests aufzeichnen | `DB::enable_query_log()` → `DB::get_query_log()` |

## Nächste Schritte

- [Ereignisse](events.md) - Listener-API, Dispatch-Modi,
  `EventFacade::fake()` für Tests
- [Lifecycle](lifecycle.md) - wo im Request-Pfad jedes Event feuert
  und wo der Request-Span konstruiert wird
- [Fehlerbehandlung](errors.md) - `ErrorOccurred`, `HttpError`,
  bereinigte 5xx-Bodys
- [Datenbank](database.md) - `QueryExecuted`, `DB::transaction`, die
  Executor-Helfer, die die Events auslösen
- [HTTP-Client](http-client.md) - outbound `traceparent`-Injektion,
  die die Distributed-Trace-Schleife schließt
