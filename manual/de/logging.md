# Protokollierung

Suprnova protokolliert über [`tracing`](https://docs.rs/tracing) - jede
Log-Zeile ist ein strukturiertes Event mit Feldern, kein formatierter
String. Beim Boot wird ein Subscriber installiert, der `LOG_LEVEL` und
`LOG_FORMAT` aus der Umgebung liest, in der Entwicklung hübsche
mehrzeilige Ausgabe und in der Produktion ein JSON-Objekt pro Zeile
ausgibt und eine ID pro Anfrage in jedes Event propagiert, das ein
Handler ausgibt.

Dieses Kapitel behandelt die Log-Oberfläche selbst: den Subscriber, die
Formate, die Level und die Korrelation über die Request-ID, die ein
Produktions-Log durchsuchbar macht. Für die OpenTelemetry-Brücke und das
Query-Logging siehe [Beobachtbarkeit](observability.md); für die
`Context`-Bag der Anfrage, die Emitter neben der ID lesen können, siehe
[Kontext](context.md).

## Was wohin protokolliert wird

Standardmäßig zwei Ausgaben:

| Wohin | Format | Wann |
|---|---|---|
| `stdout` | `LogFormat::Pretty` - mehrzeilig, farbig, menschenlesbar | Entwicklung (`APP_ENV` ist `local`, `dev`, `testing`, …) |
| `stdout` | `LogFormat::Json` - ein JSON-Objekt pro Zeile | Produktion (`APP_ENV=production` / `prod`) |

Der Standard für Entwicklung bzw. Produktion wird über
`Environment::detect()` aus `APP_ENV` berechnet. Überschreiben Sie ihn
mit `LOG_FORMAT=pretty` oder `LOG_FORMAT=json`, um einen davon explizit
zu erzwingen.

```env
# .env (dev)
LOG_LEVEL=info,sqlx=warn
LOG_FORMAT=pretty   # optional; das ist der Standard in der Entwicklung

# .env.production
LOG_LEVEL=info,sqlx=warn,suprnova::queue=debug
LOG_FORMAT=json     # optional; das ist der Standard in der Produktion
```

Das Framework schreibt ausschließlich nach `stdout`. Richten Sie in der
Produktion Ihre Container-Laufzeit, das systemd-Journal oder Ihren
Log-Aggregator darauf aus (`docker logs`, `kubectl logs`,
`journalctl -u my-app`, einen Loki-/Vector-Agenten usw.). Es gibt keinen
rotierenden File-Appender - überlassen Sie die Log-Persistenz der
Plattform.

## Events ausgeben

Verwenden Sie die `tracing`-Makros in Handlern, Jobs, Middleware,
überall:

```rust
use suprnova::{json_response, session, Request, Response};
use tracing::{debug, info, warn, error, instrument};

pub async fn checkout(_req: Request) -> Response {
    let user_id: i64 = session()
        .and_then(|s| s.get::<i64>("user_id"))
        .unwrap_or(0);

    info!(user_id, "checkout starting");

    let order = place_order(user_id).await.map_err(|e| {
        error!(user_id, error = %e, "checkout failed");
        e
    })?;

    info!(user_id, order_id = order.id, total = order.total_cents, "checkout succeeded");

    json_response!(order)
}
```

Jedes Feld wird in der JSON-Ausgabe zu einem Schlüssel auf oberster
Ebene und in der Pretty-Ausgabe zu einem farbigen `field=value`-Paar.
Bevorzugen Sie Felder gegenüber Interpolation - sie sind in JSON-Logs
durchsuchbar, und der Formatter rendert typbewusst.

Um eine Funktion in einen Span zu hüllen und jedes Event darin mit
gemeinsamen Feldern zu stempeln, verwenden Sie `#[instrument]`:

```rust
#[instrument(skip(db), fields(user_id = %user_id))]
pub async fn load_dashboard(
    db: &suprnova::DatabaseConnection,
    user_id: i64,
) -> Result<Dashboard, FrameworkError> {
    info!("loading"); // trägt automatisch user_id aus dem Span
    // … Abfragen …
}
```

Dasselbe `#[instrument]` wird zu einem OpenTelemetry-Span, wenn das
`otel`-Feature aktiviert ist - siehe
[Beobachtbarkeit](observability.md#opentelemetry).

## Log-Level

`LOG_LEVEL` ist eine [Env-Filter-Direktive von
`tracing-subscriber`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/filter/struct.EnvFilter.html),
kein einzelner Level. Die Grammatik besteht aus kommaseparierten
`target=level`-Paaren, wobei blanke Werte den Standard setzen:

```env
LOG_LEVEL=info                                  # alles ab info aufwärts
LOG_LEVEL=debug                                 # alles ab debug aufwärts
LOG_LEVEL=info,sqlx=warn                        # Standard info, sqlx leiser
LOG_LEVEL=warn,suprnova::queue=debug,my_app=info  # Standard warn, zwei Targets ausführlich
```

Targets sind normalerweise die ausgebende Crate oder der Modulpfad
(`suprnova::queue`, `hyper::server`, `my_app::services::checkout`). Ein
Target finden Sie, indem Sie die JSON-Log-Zeile lesen - das
`target`-Feld auf jedem Event ist sein Filterschlüssel.

Level nach zunehmender Ausführlichkeit: `error` < `warn` < `info`
(Standard) < `debug` < `trace`. Die Fehler-Response an den Client
wird unabhängig vom Level immer zu
`{"message": "Internal Server Error"}` bereinigt - das Detail geht
ausschließlich ins strukturierte Log.

### Ungültige Direktiven lassen den Boot nicht abstürzen

Ein fehlerhaftes `LOG_LEVEL` (etwa `LOG_LEVEL=app=notalevel`) fällt
auf `"info"` zurück und schreibt eine einzeilige Warnung nach `stderr`:

```text
suprnova: invalid LOG_LEVEL directive "app=notalevel" (...); falling back to "info". Fix LOG_LEVEL to silence this.
```

Das läuft über `stderr` statt über `tracing::warn!`, weil der Subscriber
noch nicht installiert ist - ein `warn!` würde stillschweigend verworfen.
Korrigieren Sie die Direktive, und die Warnung verschwindet.

## Pretty- vs. JSON-Ausgabe

Dasselbe `info!(user_id = 42, "saved")` wird je nach Format
unterschiedlich gerendert.

**Pretty (Entwicklung):**

```text
  2026-05-30T22:14:08.221341Z  INFO request{request_id=78a9...} my_app::handlers::checkout: saved
    at src/handlers/checkout.rs:48
    in checkout
    in request with request_id: 78a9..., method: POST, path: /checkout
```

**JSON (Produktion):**

```json
{
  "timestamp": "2026-05-30T22:14:08.221341Z",
  "level": "INFO",
  "fields": { "message": "saved", "user_id": 42 },
  "target": "my_app::handlers::checkout",
  "span": { "name": "checkout" },
  "spans": [
    { "name": "request", "request_id": "78a9...", "method": "POST", "path": "/checkout" }
  ]
}
```

Die JSON-Form ist das, was Produktions-Aggregatoren (Datadog, Loki,
Honeycomb, CloudWatch, …) ohne Zutun parsen. `span.request_id` ist der
Korrelationsschlüssel - siehe unten.

## Korrelation über die Request-ID

Jede HTTP-Anfrage bekommt von `RequestIdMiddleware`, der äußersten
Middleware jeder Chain, eine `RequestId`. Diese ID wird:

- **Wiederverwendet** aus einem sicheren eingehenden
  `X-Request-Id`-Header (alphanumerische Zeichen plus `- _ . :`, bis zu
  128 Bytes) oder **frisch erzeugt** als UUID v4, wenn er fehlt oder
  unsicher ist.
- **Zurückgespiegelt** auf der Response als `X-Request-Id` (in der 2xx-
  wie in der 5xx-Variante).
- **Eingebettet** in einen `request`-Span von `tracing`, sodass jedes
  Event aus jeder Middleware, jedem Handler oder jeder nachgelagerten
  Bibliothek automatisch `request_id` in seinem `spans`-Array trägt.
- **Vorbelegt** in der `Context`-Bag der Anfrage als `_request_id`,
  sodass Emitter, die den blanken String wollen (Jobs,
  Broadcast-Payloads, Fehlerberichte), sie über den Namen lesen können.

Im Code lesen Sie sie mit `current_request_id()`:

```rust
use suprnova::current_request_id;
use tracing::info;

if let Some(id) = current_request_id() {
    info!(request_id = %id, "checkpoint reached");
}
```

`current_request_id()` liefert `Option<RequestId>`, weil
Hintergrundarbeit (Jobs, geplante Tasks, Tests ohne installierte
Middleware) außerhalb jedes Request-Scopes läuft.

### Hintergrund-Tasks: mit der ID spawnen

`tokio::spawn` startet eine frische Task mit leeren Task-Locals - ein
Handler, der Arbeit mit Seiteneffekten spawnt, verliert
`current_request_id()`, und seine Log-Events verwaisen. Verwenden Sie
stattdessen `spawn_with_request_id`:

```rust
use suprnova::spawn_with_request_id;
use tracing::info;

pub async fn checkout(req: suprnova::Request) -> suprnova::Response {
    let order = place_order().await?;

    spawn_with_request_id(async move {
        // Diese Task sieht current_request_id() weiterhin.
        // Ihre Log-Events tragen dieselbe request_id wie die des Handlers.
        info!(order_id = order.id, "post-checkout fanout running");
        send_receipt(order.id).await;
        update_analytics(order.id).await;
    });

    suprnova::Response::ok().json(&order)
}
```

Die Hilfsfunktion propagiert sowohl das `RequestId`-Task-Local als auch
den aktuellen `tracing::Span`, sodass sich die Events des gespawnten
Futures im Log unter denselben `request`-Span schachteln. Außerhalb
eines aktiven Request-Scopes fällt sie auf ein blankes `tokio::spawn`
zurück - Sie können sie also bedenkenlos immer verwenden.

Nur die Request-ID und der Tracing-Span folgen der Task - die
`Context`-Bag der Anfrage bewusst nicht, weil Hintergrundarbeit nicht
die ursprüngliche HTTP-Anfrage bedient.

## Der Subscriber

Das Framework installiert beim Boot aus `Server::run()` heraus einen
globalen `tracing`-Subscriber. Sie rufen das fast nie selbst auf;
dokumentiert ist es, weil Tests, einbettende Anwendungen und
ungewöhnliche Einstiegspunkte es manchmal müssen.

```rust
use suprnova::{LogConfig, init_subscriber};

// LOG_LEVEL / LOG_FORMAT aus der Umgebung lesen:
init_subscriber(LogConfig::from_env());

// Oder programmatisch:
init_subscriber(LogConfig {
    level: "info,sqlx=warn".to_string(),
    format: suprnova::LogFormat::Json,
});
```

`init_subscriber` ist **idempotent**. Ein zweiter Aufruf lässt den
bestehenden Subscriber stehen und gibt ein `tracing::warn!` aus, damit
ein Betreiber sieht, dass die neue `LogConfig` nicht angewendet wurde.
Genau das verhindert, dass Tests, die jeweils `init_subscriber`
aufrufen, miteinander in ein Race laufen - der erste gewinnt, die
übrigen sind No-Ops.

Für die OTel-taugliche Variante (dieselbe `LogConfig`, plus Export für
verteiltes Tracing) verwenden Sie
[`init_telemetry`](observability.md#opentelemetry).

### Die Daemons

`queue:work`, `schedule:work`, `schedule:run` und `workflow:work` sind
Subkommandos Ihrer App-Binary und booten nicht über `Server::run()`,
deshalb installieren sie beim Hochfahren ihren eigenen Subscriber. Sie
lesen dasselbe `LOG_LEVEL` und `LOG_FORMAT` wie der Server, und Sie
selbst rufen nichts auf:

```bash
LOG_LEVEL=info,suprnova::queue=debug cargo run --bin my-app -- queue:work

# …oder, in einem Container, gegen die gebaute Binary:
LOG_LEVEL=info my-app queue:work
```

Vor 0.9.1 installierte dieser Pfad überhaupt nichts. Jede
`tracing::`-Zeile, die die Daemons ausgeben, ging ins Leere, und
`LOG_LEVEL` war für sie wirkungslos, sodass in einem Container das
Start-Banner die einzige Ausgabe blieb - ein Worker, der Jobs ins
Dead-Letter schiebt, ein Scheduler, der einen Tick überspringt, dessen
Leader-Wahl er verloren hat, und eine Sperre, die er nicht freigeben
konnte, sahen alle genauso aus wie ein untätiger Prozess. Wenn Sie einen
gepinnten Build älter als 0.9.1 fahren und sich fragen, warum ein Worker
nichts sagt: Das ist der Grund, und die Abhilfe ist das Upgrade und
keine Konfigurationsänderung.

Das meiste, was ein Worker zu sagen hat, sagt er auf `warn!` und
`error!` - ein Job, der seine Versuche aufbraucht, ein Dead-Letter, das
er nicht persistieren konnte, eine Sperre, die er nicht freigeben
konnte. Der Standard-Level `info` genügt also, um Ärger zu sehen. Gehen
Sie auf `debug` herunter, wenn Sie auch die leiseren Entscheidungen
brauchen.

## Testen

Tests müssen keinen Subscriber installieren - das Attribut
`#[suprnova_test]` und `TestContainer::fake` bauen genug Maschinerie
auf, damit Handler-Events fließen. Wenn Sie Assertions auf die
Log-Ausgabe machen wollen, fangen Sie sie über
[`tracing_subscriber::fmt::TestWriter`](https://docs.rs/tracing-subscriber/latest/tracing_subscriber/fmt/struct.TestWriter.html)
von `tracing-subscriber` oder über einen eigenen Layer ab; das Framework
liefert bewusst keinen Fake der Art "fange in diesem Test alle Logs ab",
weil die üblichen Testmuster von `tracing-subscriber` sauber
funktionieren.

## Warum Suprnova abweicht

Laravel verwendet [Monolog](https://github.com/Seldaek/monolog) -
Nachrichten-Strings mit optionalen Kontext-Arrays, Log-Kanäle und
Handler pro Kanal (Datei, Syslog, Slack, …). PHPs Modell mit einem
Prozess pro Anfrage macht einen einzigen globalen statischen Logger
sicher: Jede Anfrage bekommt ihren eigenen Prozess und ihren eigenen
Kontext.

Rusts Prozessmodell ist das Gegenteil - ein Prozess bedient viele
gleichzeitige Anfragen auf vielen Threads. Ein globaler
String-Formatter würde beim Kontext in ein Race laufen und es nötig
machen, `request_id` explizit durch jede Aufrufstelle zu reichen.
`tracing` löst beides mit strukturierten Feldern und Task-Local-Spans:
nichts durchzureichen, die Felder bleiben typisiert, und die Korrelation
ergibt sich von selbst, weil der Request-Span für jedes Event im Scope
ist, das die Chain ausgibt.

Die Ausgabe ausschließlich nach `stdout` ist ebenfalls Absicht. In
containerisierten Deployments (der einzigen Art, wie Suprnova
ausgeliefert wird) gehört die Log-Persistenz der Plattform und nicht der
Anwendung - Dateirotation, Aufbewahrung und Weiterleitung sind alle
Sache der Plattform.

## Nächste Schritte

- [Beobachtbarkeit](observability.md) - OpenTelemetry, Query-Log, die
  vollständige Oberfläche für Betreiber
- [Kontext](context.md) - die Bag pro Anfrage, in der `_request_id` und
  andere kontextbezogene Felder leben
- [Fehlerbehandlung](errors.md) - wie die Panic-Grenze des Frameworks
  und der 5xx-Pfad ihre eigenen strukturierten Events ausgeben
- [Umgebungsvariablen](env-vars.md) - Referenz zu `LOG_LEVEL` und
  `LOG_FORMAT`
