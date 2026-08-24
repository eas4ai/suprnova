# Warteschlange

Die `Queue`-Facade dispatcht Hintergrundarbeit an einen Treiber und
lässt ihn von einem separaten Worker-Prozess leeren: HTTP-Handler
kehren schnell zurück, die schwere Arbeit läuft hinter den Kulissen.
Greifen Sie darauf zurück, wann immer eine Anfrage sonst auf etwas
blockieren würde, das später erledigt werden kann - eine Mail
versenden, einen Webhook aufrufen, einen Bericht erstellen.
Kombinieren Sie sie mit [`Bus`](bus.md), wenn Sie die Arbeit *jetzt* in
der aktuellen Task ausführen und ein typisiertes Ergebnis
zurückbekommen wollen; kombinieren Sie sie mit [`Events`](events.md),
wenn ein Signal sich auf viele Listener verteilen soll.

## Schnellstart

Definieren Sie einen Job, registrieren Sie ihn einmal beim Boot,
reihen Sie ihn ein:

```rust
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use suprnova::{error::FrameworkError, queue::{Job, Queue}};

#[derive(Serialize, Deserialize)]
struct SendWelcomeEmail { user_id: i64 }

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> {
        // … die Mail tatsächlich versenden
        Ok(())
    }
}

// Einmal beim Boot (sowohl der Worker-Prozess als auch der Dispatch-Prozess brauchen das).
Queue::set_driver(std::sync::Arc::new(suprnova::queue::MemoryQueueDriver::new()));
suprnova::queue::worker::register_job::<SendWelcomeEmail>();

// Aus einem Handler einreihen:
Queue::push(SendWelcomeEmail { user_id: 42 }).await?;
```

Ein Worker-Prozess leert den konfigurierten Treiber, bis er
abgebrochen wird:

```rust
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use suprnova::queue::{Queue, worker::{WorkerConfig, run_worker}};

let driver = Queue::driver()?;
let cfg = WorkerConfig {
    visibility_timeout: Duration::from_secs(60),
    poll_interval: Duration::from_millis(100),
    max_jobs: None,
    queues: Vec::new(),
};
let shutdown = CancellationToken::new();
run_worker(driver, cfg, shutdown).await;
```

In einer per Scaffold erzeugten App wird der Worker über den
Subcommand `queue:work` der Binary gestartet -
`cargo run -- queue:work` -, der denselben Bootstrap durchläuft wie
Ihr HTTP-Server, sodass Observer und Listener, die in `bootstrap()`
registriert sind, für Inserts aus einem Queue-Handler identisch
feuern.

## Treiber

Fünf Treiber sind im Framework enthalten. Konfigurieren Sie sie über
die Env-Variable `QUEUE_DRIVER` oder durch programmatisches Aufrufen
von `Queue::set_driver(...)`.

| Treiber | Verwendung für | Stärken |
| --- | --- | --- |
| `MemoryQueueDriver` | Tests, Single-Process-Apps | `tokio::time::DelayQueue` für `available_at`, virtual-clock-kompatibel |
| `RedisQueueDriver` | Produktions-Fan-out | Consumer-Groups + `XAUTOCLAIM` + ZSET-gestützte verzögerte Jobs |
| `DatabaseQueueDriver` | Single-DB-Apps | `FOR UPDATE SKIP LOCKED` auf Postgres/MySQL, `BEGIN`-serialisiert auf SQLite |
| `SyncQueueDriver` | Dev, CI | führt den Handler inline bei `push` aus, kein Worker |
| `NullQueueDriver` | Test-Wrapper | verwirft jedes `push`, ohne auszuführen |

`Queue::bootstrap_from_env()` liest `QUEUE_DRIVER` und verdrahtet den
passenden Treiber; `Queue::bootstrap_default()` verdrahtet immer den
Memory-Treiber. Der Server-Boot-Pfad ruft eine davon für Sie auf - die
meisten Apps konfigurieren nur über die Umgebung.

### Umgebungskonfiguration

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# Database-Treiber - DB::init() muss zuerst laufen
QUEUE_DRIVER=database
QUEUE_DB_TABLE=jobs
```

Der Database-Treiber validiert `QUEUE_DB_TABLE` bei der Konstruktion
als SQL-Identifier, sodass ein fehlerhafter Env-Wert den Boot
scheitern lässt, statt bis zur SQL-Komposition zu gelangen. Redis
verwendet unter der Haube sea-streamer-redis mit
`AutoCommit::Disabled`; das Visibility-Timeout wird zum Zeitpunkt der
Consumer-Group-Konstruktion fixiert, daher wird das Argument
`visibility_timeout` pro Pop auf Redis ignoriert (eine dokumentierte
Abweichung vom Trait-Vertrag, die von Redis Streams erzwungen wird).

### Warum Suprnova abweicht

Laravel routet jedes Queueable über den Bus und unterscheidet
`ShouldQueue`-Jobs zum Dispatch-Zeitpunkt. Suprnova trennt die beiden:
`Bus` für synchrone Arbeit, die ein typisiertes Ergebnis zurückgibt,
`Queue` für asynchrone Arbeit, die einen Prozessabsturz überlebt. PHP
braucht das implizite Routing, weil sein Prozess-pro-Anfrage-Modell
„das später erledigen, in einem anderen Prozess“ sonst schwer
modellierbar macht. Tokio braucht das nicht - explizites
`Bus::dispatch` vs. `Queue::push` ist klarer, schneller und macht die
Dauerhaftigkeits-Entscheidung an der Aufrufstelle sichtbar. Siehe
[`bus.md`](bus.md) für den Vergleich nebeneinander.

## Push-Varianten

Jede Push-Variante nimmt einen typisierten `J: Job`-Wert und kehrt
zurück, wenn die Envelope beim Treiber committet ist - nicht, wenn der
Handler läuft.

| Methode | Verhalten |
| --- | --- |
| `Queue::push(job)` | sofort einreihen |
| `Queue::push_later(job, at)` | verfügbar zu einem bestimmten `DateTime<Utc>` |
| `Queue::later(delay, job)` | verfügbar nach `delay` ab jetzt |
| `Queue::push_with(job, overrides)` | sofort einreihen mit Per-Push-`EnvelopeOverrides` |
| `Queue::later_with(delay, job, overrides)` | nach `delay` ab jetzt verfügbar, mit Per-Push-`EnvelopeOverrides` |

| `Queue::push_unique(job)` | dedupliziert nach `J::unique_id` innerhalb von `J::unique_for`; liefert `Ok(true)`, wenn die Envelope gepusht wurde, und `Ok(false)`, wenn ein lebender Dedupe-Key sie unterdrückt hat |
| `Queue::push_unique_later(job, at)` | unique + geplant |
| `Queue::later_unique(delay, job)` | unique + verzögert |
| `Queue::bulk(vec![job1, job2, ...])` | pusht jeden Job (der Treiber darf einen nativen Bulk-Pfad verwenden) |

`push_unique` setzt voraus, dass die Cache-Schicht gebootstrappt ist -
die Dedupe-Sperre lebt in [`Cache`](cache.md) über
[`Idempotency::commit_on_success`](idempotency.md). Ein
fehlgeschlagener Push gibt den Dedupe-Key frei, sodass der Aufrufer es
erneut versuchen kann; ein erfolgreicher Push hält ihn `J::unique_for`
Sekunden lang. Der Job muss `Job::unique_id(&self)` überschreiben, um
`Some(id)` zurückzugeben - `None` liefert einen internen Fehler.

Der Boolean beantwortet eine Frage - „ist dieser Job in der
Warteschlange?“ - und dahinter steckt ein dritter Fall. Geht das Lease
der Dedupe-Sperre verloren, während der Push unterwegs ist, wird der
Push trotzdem abgeschlossen (die Idempotenz-Schicht bricht nie einen
Body ab, der bereits eine Wirkung gehabt haben könnte), und Sie
bekommen weiterhin `Ok(true)`, mit einem Protokolleintrag auf
`warn`-Ebene, der den Job und seinen Unique-Key benennt. Der Job ist
eingereiht; unbewiesen ist, dass niemand sonst gleichzeitig denselben
eingereiht hat. Ihr Handler muss Redelivery ohnehin tolerieren, das
braucht also keine zusätzliche Behandlung - aber der Eintrag ist da,
weil eine Häufung davon bedeutet, dass der Cache hinter Ihrer
Dedupe-Sperre Mühe hat.

### Per-Push-Überschreibungen mit `EnvelopeOverrides`

`Queue::push_with` und `Queue::later_with` nehmen neben dem Job ein
`EnvelopeOverrides` entgegen - für den einen Dispatch, der sich bei
Warteschlange, Connection, Timeout oder Retry-Verhalten von den eigenen
Standards des Jobs unterscheiden muss:

```rust
use std::time::Duration;
use suprnova::queue::{EnvelopeOverrides, Queue};

let overrides = EnvelopeOverrides {
    queue: Some("priority".into()),
    timeout: Some(Duration::from_secs(10)),
    max_tries: Some(1),
    ..Default::default()
};

Queue::push_with(SendWelcomeEmail { user_id: 42 }, overrides.clone()).await?;

// Das verzögerte Gegenstück, entsprechend dem Verhältnis von
// `Queue::later` zu `Queue::push`.
Queue::later_with(Duration::from_secs(60), SendWelcomeEmail { user_id: 42 }, overrides).await?;
```

Jedes Feld ist standardmäßig `None` und überlässt die normale Auflösung,
die `Queue::push` bereits ausführt; ein `Some`-Feld gewinnt für diesen
einen Push gegen alles andere und hat Vorrang vor einer über
[`Queue::route`](#warteschlangen-routing) registrierten Route und der
eigenen `Job::*`-Deklaration des Jobs für dieses Feld:

| Feld | Hat Vorrang vor |
| --- | --- |
| `queue` | `Queue::route`, `Job::queue()` |
| `connection` | `Queue::route`, `Job::connection()` |
| `timeout` | `Job::timeout()` |
| `fail_on_timeout` | `Job::fail_on_timeout()` |
| `max_tries` | `Job::max_tries()` |
| `backoff` | `Job::backoff()` |

`EnvelopeOverrides` ist die Primitive, auf der sowohl
`Mail::on_queue`/`.on_connection()` als auch die
Warteschlangenabstimmung pro Benachrichtigung von `Notify::queue`
aufbauen - siehe [Mail](mail.md#einreihung) und
[Benachrichtigungen](notifications.md).

### Job-deklarierte Verzögerung

Ein Job kann seine eigene Standardverzögerung tragen, statt dass jede
Aufrufstelle erneut `Queue::later(Duration::from_secs(60), job)`
wiederholt:

```rust
impl Job for SendDigest {
    // ...
    fn delay() -> Option<Duration> { Some(Duration::from_secs(60)) }
}
```

`Queue::push(job)`, `Queue::push_with(job, overrides)`,
`Queue::push_unique(job)` und `Queue::bulk(vec![job1, job2])`
berücksichtigen sie alle - `available_at` wird zu `now + J::delay()`
statt zu `now`. `Queue::bulk` löst die Verzögerung einmal pro Aufruf
auf, da jeder Job im Vektor denselben konkreten `J` und damit dieselbe
`Job::delay()` teilt.

Eine explizite Verzögerung an der Aufrufstelle gewinnt immer:
`Queue::push_later(job, at)`, `Queue::later(delay, job)`,
`Queue::later_with(delay, job, overrides)`,
`Queue::push_unique_later(job, at)` und
`Queue::later_unique(delay, job)` verwenden jeweils den vom Aufrufer
übergebenen Zeitstempel oder die Verzögerung unverändert -
`Job::delay()` wird für keinen davon konsultiert. Verwenden Sie die
Trait-Methode, wenn jeder Dispatch eines Job-Typs standardmäßig verzögert
starten soll; verwenden Sie eine `later`-/`push_later`-Variante für eine
Verzögerung, die nur ein bestimmter Dispatch braucht, die der Typ sonst
nicht deklariert.

Batches und Chains konsultieren sie ebenfalls nicht:
`Queue::batch()...add(job)` und `Queue::chain()...add(job)?` bauen beide
ihre Envelopes mit `available_at` auf den Moment gesetzt, in dem Sie
`add` aufgerufen haben. Ein Job mit deklarierter `Job::delay()` wird
daher als Teil eines Batches oder einer Chain sofort dispatcht, obwohl
ein einfaches `Queue::push(job)` desselben Jobs warten würde. Geben Sie
dem Job auf andere Weise eine explizite Verzögerung - etwa über ein Feld
am Job, das in `handle()` angewendet wird -, wenn ein Batch- oder
Chain-Schritt eine braucht.

### Warum Suprnova abweicht

Laravels `$job->delay` ist eine Instanzeigenschaft, die pro Dispatch
gesetzt wird (`SendDigest::dispatch($user)->delay(60)`), sodass zwei
Dispatches derselben Klasse unterschiedliche Verzögerungen tragen können.
`Job::delay()` ist hier stattdessen ein Standard auf Klassenebene, wie
`Job::queue()` oder `Job::max_tries()` - ein Dispatch, der eine aus
seinen eigenen Daten berechnete Verzögerung braucht, verwendet
`Queue::later`/`push_later`, was den deklarierten Standard bereits
überstimmt.

## Job-Konfiguration

Überschreiben Sie die assoziierten Funktionen von `Job`, um das
Verhalten pro Implementierung einzustellen:

```rust
use std::time::Duration;
use suprnova::queue::{BackoffSchedule, JobMiddleware};

#[async_trait]
impl Job for SendWelcomeEmail {
    fn job_name() -> &'static str { "SendWelcomeEmail" }

    async fn handle(self) -> Result<(), FrameworkError> { /* … */ Ok(()) }
    fn delay() -> Option<Duration> { None }                // Standard: keine Verzögerung


    fn max_tries() -> u32 { 5 }                            // Standard: 3
    fn timeout() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn fail_on_timeout() -> bool { false }                 // Standard: false (Timeout wird wiederholt)
    fn backoff() -> BackoffSchedule {
        BackoffSchedule::Sequence { secs: vec![5, 15, 60, 300] }
    }
    fn unique_id(&self) -> Option<String> {
        Some(format!("welcome:{}", self.user_id))
    }
    fn unique_for() -> Duration { Duration::from_secs(600) }  // Standard: 5 Minuten
    fn middleware() -> Vec<std::sync::Arc<dyn JobMiddleware>> {
        vec![/* siehe "Job-Middleware" unten */]
    }
}
```

## Warteschlangen-Routing

Standardmäßig geht jeder Job an eine Warteschlange, und jeder Worker
leert sie vollständig. Sobald manche Jobs langsamer oder wichtiger
sind als andere, wollen Sie dedizierte Worker-Pools: Ein lang
laufender Export sollte nicht hinter tausend Willkommens-Mails
feststecken.

Ein Job kann angeben, wohin er gehört:

```rust
#[async_trait]
impl Job for GenerateExport {
    fn job_name() -> &'static str { "GenerateExport" }
    async fn handle(self) -> Result<(), FrameworkError> { Ok(()) }

    fn queue() -> Option<&'static str> { Some("exports") }
    fn connection() -> Option<&'static str> { None }   // Standard-Connection
}
```

…und ein Betreiber kann das zentral überschreiben, ohne den Job zu
berühren:

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::route::<GenerateExport>(None, Some("heavy"));
Queue::route::<SendInvoice>(Some("redis"), Some("billing"));
```

Die Auflösung läuft mit der höchsten Priorität zuerst:

1. eine an `Queue::push_with` / `Queue::later_with` übergebene Per-Push-
   Überschreibung (siehe [Per-Push-Überschreibungen mit
   `EnvelopeOverrides`](#per-push-überschreibungen-mit-envelopeoverrides))
2. eine mit `Queue::route` registrierte Route
3. die eigenen `Job::queue` / `Job::connection` des Jobs
4. der Treiber- / globale Standard

Wird für ein Feld `None` übergeben, bleibt diese Dimension unberührt,
sodass das Routen der Connection eines Jobs die bereits deklarierte
Warteschlange nicht stört.

Die beiden Dimensionen laufen heute auf unterschiedlichen Tiefen. Die
**Warteschlange** wird end-to-end respektiert - auf die Envelope
gestempelt, vom Treiber gespeichert, über `--queue` gefiltert. Die
**Connection** löst den Connection-*Namen* auf, der auf den
Lifecycle-Events `JobQueueing` / `JobQueued` mitgeführt wird - das ist
es, was Listener und Dashboards sehen; ein einziger prozessglobaler
Treiber empfängt weiterhin jeden Push, sodass das Routen der
Connection eines Jobs noch keinen anderen Treiber auswählt.
Connections jetzt schon zu deklarieren ist zukunftskompatibel für die
Zeit, wenn Pro-Connection-Treiber landen, nicht verhaltensrelevant.

Dedizieren Sie dann einen Worker dafür:

```bash
./app queue:work --queue=billing
./app queue:work --queue=exports,heavy
./app queue:work                       # leert wie zuvor jede Warteschlange
```

Ein Job ohne Route gehört zu `default`, daher leert `--queue=default`
ungeroutete Arbeit, statt sie stranden zu lassen.

### Warum Suprnova abweicht

Laravels `Queue::route(...)` nimmt eine Klassen-Zeichenkette; Suprnova
nimmt den Job als Typparameter, sodass ein umbenannter oder gelöschter
Job ein Compile-Fehler ist, statt einer Route, die stillschweigend
aufhört zu matchen.

Die größere Abweichung ist, was passiert, wenn ein Treiber nicht
filtern kann. `QueueDriver::pop_from` **lehnt** einen
Warteschlangenfilter ab, den er nicht einhalten kann, statt
stillschweigend auf das Leeren aller Warteschlangen zurückzufallen.
Ein Worker, der angewiesen wird, nur `billing` zu leeren, aber
stillschweigend alle Warteschlangen leert, sieht wie ein
funktionierendes Deployment aus, bis der falsche Pool die falschen
Jobs konsumiert - daher wird die Fehlkonfiguration beim ersten Poll
sichtbar gemacht. Die Memory- und Database-Treiber filtern nativ; ein
Treiber, der das nicht tut - der Redis-Treiber ist einer davon, da
eine einzelne Stream-Consumer-Group keine Speicherung pro
Warteschlange hat - liefert einen Fehler, statt in die Irre zu führen.

### Die Tabelle `jobs`

`DatabaseQueueDriver` erwartet dieses Schema. Die Spalte `queue` ist
das, was die Filterung über `--queue` ermöglicht:

```sql
CREATE TABLE jobs (
    id              TEXT PRIMARY KEY,
    job_name        TEXT NOT NULL,
    queue           TEXT NULL,
    envelope_json   TEXT NOT NULL,
    available_at    BIGINT NOT NULL,
    reserved_until  BIGINT NULL,
    reserved_token  TEXT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    created_at      BIGINT NOT NULL
);
CREATE INDEX idx_jobs_available_at ON jobs(available_at);
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

`queue` ist nullable, und ein ungerouteter Job speichert `NULL` statt
`'default'`. Das ist beabsichtigt: Eine von einer älteren Binary
geschriebene Zeile ist nicht von einer ungerouteten Zeile zu
unterscheiden, die eine neue Binary geschrieben hat, sodass eine
Flotte mit gemischten Versionen während eines Rolling-Upgrades
dieselbe Arbeit leert.

Die Spalte zu einer bestehenden Tabelle hinzuzufügen ist
**erforderlich**, nicht nur für die Filterung: `push` nennt die Spalte
`queue` in seinem `INSERT`, unabhängig davon, ob der Job geroutet ist,
sodass eine Binary ab 0.7.0 jedes `push` gegen eine Tabelle scheitern
lässt, der sie fehlt. Führen Sie zuerst die Migration aus, rollen Sie
dann die Binaries aus - ältere Binaries listen ihre Spalten explizit
und ignorieren die neue, daher ist diese Reihenfolge sicher:

```sql
ALTER TABLE jobs ADD COLUMN queue TEXT NULL;
CREATE INDEX idx_jobs_queue ON jobs(queue);
```

### Backoff-Zeitpläne

| Variante | Verhalten |
| --- | --- |
| `Fixed { secs }` | konstante Verzögerung pro Versuch |
| `Exponential { base_secs, cap_secs, jitter_ratio }` | `min(base * 2^(attempts-1), cap)` × zufällig in `[1±jitter]` |
| `Sequence { secs }` | ein Eintrag pro Versuch; der letzte Eintrag wiederholt sich, sobald er erschöpft ist |

Der Standard ist
`Exponential { base_secs: 2, cap_secs: 300, jitter_ratio: 0.25 }` - 2
Sekunden bis 5 Minuten mit ±25 % Jitter.

## Job-Middleware

Sechs Middlewares sind im Framework enthalten, alle nach dem Vorbild
von `Illuminate\Queue\Middleware\*`:

| Middleware | Verhalten |
| --- | --- |
| `WithoutOverlapping` | hält für die Dauer eine `Cache::lock`; Freigabe mit Verzögerung bei Konkurrenz |
| `RateLimited` | sperrt auf Basis des `RateLimiter`-Budgets; Freigabe, bis das Fenster zurücksetzt |
| `ThrottlesExceptions` | Rate-Limit auf aufeinanderfolgende *Fehlschläge*, nicht Anfragen |
| `Skip::when(cond)` / `Skip::unless(cond)` | verwirft den Job, wenn die Bedingung erfüllt ist |
| `FailOnException` | befördert passende Fehler zu permanenten Fehlschlägen (keine Wiederholung) |
| `SkipIfBatchCancelled` | verwirft den Job, wenn sein zugehöriger Batch abgebrochen wurde |

Verdrahten Sie sie auf der `Job`-Implementierung:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{JobMiddleware, RateLimited, WithoutOverlapping};

fn middleware() -> Vec<Arc<dyn JobMiddleware>> {
    vec![
        Arc::new(
            WithoutOverlapping::new("user-42")
                .expire_after(Duration::from_secs(120))
        ),
        Arc::new(
            RateLimited::new(10, Duration::from_secs(60))
                .by("send-mail")
        ),
    ]
}
```

`WithoutOverlapping` und `RateLimited` brauchen das gebootete
Cache-Subsystem (`Cache::init` oder `App::bind::<dyn CacheStore>(...)`
beim Start).

### Eine Sperre, die sich nicht freigeben lässt, lässt den Job nicht fehlschlagen

Kann `WithoutOverlapping` ihre Sperre nicht freigeben, nachdem der
Handler gelaufen ist - das Cache-Backend hatte einen Ausfall, die
Verbindung brach ab -, protokolliert sie auf `warn` und gibt trotzdem
das eigene Ergebnis des Handlers zurück. Die Sperre läuft dann bei
`expire_after` ab.

Das ist beabsichtigt. Zu dem Zeitpunkt, an dem die Freigabe läuft, hat
der Handler seine Seiteneffekte bereits committet: Zeilen geschrieben,
Mail versendet, Belastungen vorgenommen. Den Freigabe-Fehlschlag als
Job-Fehlschlag zu melden würde den Worker zu einer Wiederholung
veranlassen und alles ein zweites Mal tun lassen, was ein schlechteres
Ergebnis ist als ein Lock-Schlüssel, der seine TTL lang gehalten wird.
Ein Handler, der wirklich fehlgeschlagen ist, meldet seinen Fehlschlag
trotzdem - das Unterdrücken des Freigabe-Fehlers unterdrückt nicht den
des Handlers.

### Der Vertrag „Freigabe ohne Versuchsverbrauch“

Middleware gibt ein `JobOutcome` zurück statt `Result<()>`. Vier
Varianten:

- `JobOutcome::Completed` - Handler lief, bestätigt.
- `JobOutcome::Released { delay }` - reiht nach `delay` erneut ein, **ohne** `attempts` zu erhöhen. Verwendet von `WithoutOverlapping`, `RateLimited`. Der Worker übergibt die gesamte Operation an `QueueDriver::release`, und jeder Treiber im Framework reiht seine eigene gespeicherte Kopie an Ort und Stelle neu ein, sodass die Nachricht nie gleichzeitig reserviert und sichtbar ist, und nie keins von beiden. Die Versuchszahl bleibt erhalten, ohne dass der Worker eine Arithmetik vornimmt, mit der ein Treiber nicht übereinstimmen könnte - die gespeicherte Kopie wurde für diesen Lauf nie erhöht.
- `JobOutcome::Failed { reason }` - wird jetzt Dead-Letter, wird im Failed-Jobs-Store persistiert, keine Wiederholung.
- `JobOutcome::Deleted` - verwirft die Reservierung ohne Dead-Letter. Verwendet von `Skip`. Gehörte der Job zu einem Batch, dekrementiert sich das `pending_jobs` des Batches trotzdem, damit Callbacks feuern können.

Dieser Vertrag ist es, der dafür sorgt, dass sich „gedrosselt, weil
der Bucket voll war“ in Retry-Verbuchung, Metriken und
Lifecycle-Events anders anfühlt als „fehlgeschlagen, weil der Handler
einen Fehler hatte“.

### Was als Versuch zählt

Zwei Wege, wie ein Job einen Worker verlässt, ohne fertig zu werden,
und beide verbrauchen einen Versuch:

- **Der Handler ist fehlgeschlagen** - hat `Err` zurückgegeben, oder ist in Panic geraten, aufgefangen von der Grenze des Frameworks. Der Worker lehnt ab; der Treiber reiht mit `attempts + 1` neu ein.
- **Der Worker ist gestorben** - OOM-Kill, `abort()`, ein Segfault, `docker kill`, oder das SIGKILL, das ein Supervisor sendet, wenn ein Stopp in einen Timeout läuft. Nichts schließt irgendetwas ab; die Reservierung läuft einfach ab. Welcher Worker den Job auch zurückfordert, verbucht den Versuch an diesem Punkt.

Der zweite Fall war früher kostenlos, und das war ein Loch statt einer
Gnade: Ein Job, der zuverlässig seinen Worker tötet, konnte
`max_tries` nie erschöpfen und so nie zum Dead-Letter werden. Er würde
jeden Worker töten, der ihn übernahm, byteidentisch zurückkommen und
den nächsten töten, solange irgendetwas Worker weiter neu startete.

Alle drei im Framework enthaltenen Treiber verbuchen ihn, weil ein
Wechsel von `QUEUE_DRIVER` nicht ändern darf, ob ein solcher
vergiftender Job gestoppt werden kann. `database` erkennt ein
abgelaufenes `reserved_until`; `memory` verbucht ihn, wenn der Reaper
die Reservierung zurück auf sichtbar setzt; `redis` liest die
Delivery-Zahl des Eintrags aus `XPENDING`, da ein Redis-Stream-Eintrag
unveränderlich ist und sein eigener Zähler der einzige Nachweis ist.

`JobOutcome::Released` ist die bewusste Ausnahme - siehe den Vertrag
oben. Ein von `RateLimited` gedrosselter Job lief nie, daher schuldet
er nichts.

**Auf Redis hat das Zurückfordern zwei Uhren.** `--visibility-timeout`
legt fest, wie lange ein Eintrag unbestätigt liegen muss, bevor er
sich für das Zurückfordern qualifiziert; ein zweites Intervall
bestimmt, wie oft ein Consumer nachsieht. Der Treiber koppelt das
zweite an das erste, sodass ein verlorener Job innerhalb von ungefähr
dem doppelten konfigurierten Timeout zurückkommt, statt Timeout plus
feste 30 Sekunden.

**Das Budget wird geprüft, bevor der Handler läuft, nicht nur beim
Abschließen.** Jede andere Dead-Letter-Entscheidung passiert, nachdem
ein Handler zurückkehrt, was voraussetzt, dass der Handler
zurückkehrt. Ein Job, der seinen Worker tötet, kann diese Prüfung
nicht erreichen, daher verweigert der Worker auch das Dispatchen eines
Jobs, dessen `attempts` bereits aufgebraucht sind - er macht ihn
stattdessen zum Dead-Letter, bevor er einen weiteren Worker mitreißt.
Ohne das würde das Zählen des Versuchs nur eine Zahl steigen lassen,
während der Job weiter kreiste.

**Was das für Sie bedeutet.** `attempts` zählt *Zustellungen an einen
Worker*, nicht *Handler-Fehlschläge*. Ein Worker, der aus Gründen
verloren geht, die nichts mit dem Job zu tun haben - ein Host-Reboot,
ein durch einen lauten Nachbarn verursachtes OOM -, verbrennt
ebenfalls einen Versuch aus dem Budget dieses Jobs. Laravel verhält
sich genauso. Bemessen Sie `max_tries` mit diesem Wissen, und
bevorzugen Sie idempotente Handler: **At-least-once**-Zustellung war
immer der Vertrag, und das lässt den Redelivery-Pfad ehrlich zählen
statt stillschweigend.

## Lifecycle-Events

Worker emittieren Lifecycle-Events im Laravel-Stil über die
[`Event`](events.md)-Facade. Listener bekommen die Identität der
Envelope (`id`, `job_name`, `attempts`, `max_tries`, `connection`),
nicht die typisierte Job-Instanz - der Worker ist typ-gelöscht über
JSON-Payloads. Fehler reisen als `String`, da `FrameworkError` kein
`Clone` ableitet.

| Event | Feuert wenn |
| --- | --- |
| `JobQueueing` | bevor die Envelope den Treiber erreicht |
| `JobQueued` | nachdem der Treiber sie akzeptiert hat |
| `UniqueJobSkipped` | `push_unique` unterdrückte ein Duplikat innerhalb des `unique_for`-Fensters |

| `JobProcessing` | Worker hat entnommen, kurz vor dem Dispatch |
| `JobProcessed` | Handler hat `Ok` zurückgegeben |
| `JobAttempted` | jeder endgültige Abschluss (Erfolg, Fehlschlag, Timeout) |
| `JobExceptionOccurred` | Handler hat `Err` zurückgegeben, wird wiederholt |
| `JobReleasedAfterException` | Retry-nach-Fehler-Wiedereinreihung ist passiert |
| `JobReleased` | von Middleware ausgelöste Freigabe (kein Fehlschlag) |
| `JobFailed` | zum Dead-Letter geworden |
| `JobTimedOut` | Timeout pro Versuch überschritten |
| `Looping` | jede Schleifeniteration (vor dem Entnehmen) |
| `WorkerStarting` / `WorkerStopping` | einmal pro Worker-Lebenszeit |
| `WorkerInterrupted` | `Queue::restart()`-Signal beobachtet |
| `QueuePaused` | `Queue::pause` setzte den eigenen Schalter einer Warteschlange |
| `QueueResumed` | `Queue::resume` löschte den eigenen Schalter einer Warteschlange |
| `QueuesPaused` | `Queue::pause_all` setzte den globalen Schalter |
| `QueuesResumed` | `Queue::resume_all` löschte den globalen Schalter |


Abonnieren Sie mit der normalen `Event::listen`-API. Events sind
Best-Effort - `Event::dispatch` ohne Listener ist ein No-Op `Ok(())`,
sodass Worker in Deployments ohne `Event::init()` nichts kosten.
`UniqueJobSkipped` ist das einzige Event, das auf der *Push*-Seite
statt auf der Worker-Seite feuert, und das einzige, das keinen Fehler
meldet. Es trägt `job_name`, `unique_id` und `connection` - die
Dedupe-Entscheidung fällt, bevor eine Envelope existiert, daher gibt es
keine Envelope-ID zu melden. Der Push liefert weiterhin `Ok(false)`;
das Event macht eine ansonsten unsichtbare Unterdrückung beobachtbar.

`QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` feuern
ebenso direkt aus `Queue::pause` / `resume` / `pause_all` / `resume_all`,
nicht aus der Worker-Schleife. Sie tragen ebenfalls keine Envelope-
Identität; siehe unten „Warteschlangen pausieren“ für den vollständigen
Vertrag.

## Speicherung fehlgeschlagener Jobs

Zum Dead-Letter gewordene Jobs landen im konfigurierten
`FailedJobStore`:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryFailedJobStore};

Queue::set_failed_store(Arc::new(MemoryFailedJobStore::new()));

// In Admin-Tooling:
let store = Queue::failed_store().unwrap();
for record in store.all().await? {
    println!("{} failed: {}", record.job_name, record.exception);
}
store.forget(some_id).await?;
store.flush(None).await?;
```

Drei Backends:

- `MemoryFailedJobStore` - In-Process-`Vec`, geht beim Neustart verloren.
- `DatabaseFailedJobStore` - persistiert über SeaORM in eine Tabelle `failed_jobs`.
- `NullFailedJobStore` - verwirft jeden Datensatz. Spiegelt Laravels `NullFailedJobProvider`.

### Wenn der Store einen Datensatz zurückweist

Gibt der konfigurierte Store einen Fehler zurück, protokolliert der
Worker auf `error` und lässt die Reservierung intakt, statt sie zu
bestätigen. Der Job kehrt beim Ablauf der Visibility zurück und wird
wiederholt - er wird nicht stillschweigend verworfen.

Das ist beabsichtigt. Die Alternative, trotzdem zu bestätigen,
verwirft einen Job, der bereits seine Versuche erschöpft hat *und*
nirgendwo aufgezeichnet werden konnte, was nicht wiederherstellbar
ist. Ein Job, der immer wieder zurückkommt, ist wiederherstellbar:
Reparieren Sie den Store, und die nächste Zustellung landet.

Der praktische Fall ist ein `DatabaseFailedJobStore`, der auf eine
nicht migrierte Tabelle `failed_jobs` zeigt. Bis Sie migrieren,
kreisen zum Dead-Letter werdende Jobs mit einer Redelivery pro
Visibility-Timeout, jede protokolliert den Fehler des Stores. Wenn Sie
Fehlschläge wirklich verworfen haben wollen, konfigurieren Sie
`NullFailedJobStore` - das gelingt, sodass der Job bestätigt wird und
verschwindet.

### Wiederholen

```rust
use uuid::Uuid;

// Einzelner Datensatz - false, wenn die ID nicht im Store war.
Queue::retry_failed(some_id).await?;

// Bulk - optionaler Cutoff (wiederholt nur Datensätze älter als `before`).
let count = Queue::retry_all_failed(None).await?;
```

`retry_failed` lädt die Envelope, setzt `attempts`, `available_at` und
`idempotency_key` zurück, reiht sie über den konfigurierten Treiber
ein und löscht dann den Failed-Job-Datensatz. Spiegelt
`php artisan queue:retry <id>` plus die Semantik von `queue:flush`
(jede wiederholte Envelope wird eingereiht UND aus dem Store
entfernt).

### Schema von `failed_jobs`

`DatabaseFailedJobStore` erwartet diese Tabelle (verwaltet von Ihren
Migrationen):

```sql
CREATE TABLE failed_jobs (
    id              TEXT PRIMARY KEY,
    connection      TEXT NOT NULL,
    queue           TEXT NOT NULL,
    job_name        TEXT NOT NULL,
    envelope_json   TEXT NOT NULL,
    exception       TEXT NOT NULL,
    failed_at       BIGINT NOT NULL
);
CREATE INDEX idx_failed_jobs_failed_at ON failed_jobs(failed_at);
```

Das Argument `table` von `DatabaseFailedJobStore::new` wird bei der
Konstruktion als SQL-Identifier validiert.

## Eingereihte Batches

Dispatchen Sie eine Gruppe von Jobs mit Fortschrittsverfolgung und
Abschluss-Callbacks:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, MemoryBatchRepository, batch::register_callback};

Queue::set_batch_repository(Arc::new(MemoryBatchRepository::new()));

// Benannte Callbacks beim Boot registrieren.
register_callback(Arc::new(SendSummary));
register_callback(Arc::new(PageOnFail));

let id = Queue::batch()
    .name("import-users")
    .add(ImportUser { id: 1 })
    .add(ImportUser { id: 2 })
    .add(ImportUser { id: 3 })
    .then("send-summary-email")
    .catch("page-on-fail")
    .finally("cleanup-temp-tables")
    .dispatch()
    .await?;

// Fortschritt später einsehen:
let repo = Queue::batch_repository().unwrap();
let snap = repo.find(&id).await?.unwrap();
println!("{}/{} jobs done ({}%)", snap.processed_jobs(), snap.total_jobs, snap.progress());
```

Jeder Worker schließt seinen Job gegen den Batch ab, und wenn
`pending_jobs` null erreicht, feuert der Worker die registrierten
`then`/`catch`/`finally`-Callbacks. Standardmäßig bricht der erste
Fehlschlag den Batch ab; `.allow_failures()` lässt die restlichen Jobs
weiterlaufen.

### Dauerhafte Batches

`MemoryBatchRepository` geht beim Neustart verloren, was jeden
in-flight Batch stranden lässt: seine Zähler sind weg, `pending_jobs`
kann nie wieder null erreichen, und die Callbacks feuern nie.
Verwenden Sie in Produktion `DatabaseBatchRepository`:

```rust
use std::sync::Arc;
use suprnova::queue::{Queue, DatabaseBatchRepository};

Queue::set_batch_repository(Arc::new(DatabaseBatchRepository::new(db.clone())));
```

Zwei Tabellen, die das Framework nicht erstellt - fügen Sie sie zu
Ihren Migrationen hinzu, genau wie `jobs` und `failed_jobs`:

```sql
CREATE TABLE job_batches (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    total_jobs    INTEGER NOT NULL,
    options_json  TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    cancelled_at  INTEGER NULL,
    finished_at   INTEGER NULL
);

CREATE TABLE job_batch_settlements (
    batch_id   TEXT NOT NULL,
    job_id     TEXT NOT NULL,
    failed     INTEGER NOT NULL,
    settled_at INTEGER NOT NULL,
    PRIMARY KEY (batch_id, job_id)
);
```

Mit `DatabaseBatchRepository::with_tables(db, batches, settlements)`
benennen Sie sie selbst; beide Namen werden bei der Konstruktion als
SQL-Identifier validiert.

Beachten Sie, was `pending_jobs` und `failed_jobs` **nicht** sind:
Spalten. Sie werden bei jedem Lesevorgang aus den Abschluss-Zeilen
abgeleitet -

```text
pending_jobs = max(0, total_jobs - COUNT(settlements))
failed_jobs  = COUNT(settlements WHERE failed)
```
 -
weil Warteschlangen **at-least-once** sind, sodass derselbe Job mehr
als einmal abschließt, wann immer eine Redelivery passiert, ein
Bestätigen dupliziert wird, oder ein Worker zwischen dem Erledigen der
Arbeit und ihrer Aufzeichnung stirbt. Ein pro Abschluss
dekrementierter Zähler driftet bei jedem dieser Fälle, und die Drift
ist nicht kosmetisch: `pending_jobs` gatet die Callbacks, sodass eine
verfrühte Null `then` feuert, während andere Jobs im Batch noch
laufen. Mit abgeleiteten Zählern und dem Primärschlüssel auf
`(batch_id, job_id)` fügt ein wiederholter Abschluss nichts ein, und
es gibt keinen Zähler, den man falsch machen könnte - über Prozesse
hinweg, nicht nur innerhalb eines einzigen.

### Wenn ein Dispatch auf halbem Weg fehlschlägt

Schlägt ein `driver.push` auf halbem Weg durch `dispatch()` fehl, sind
die Jobs, die die Warteschlange bereits erreicht haben, real und
bereits mit der Batch-ID gestempelt. Der Batch wird daher
abgeschlossen statt entfernt: Jede Envelope, die *nicht* eingereiht
wurde, wird als fehlgeschlagener Job verzeichnet, und der Batch wird
abgebrochen.

`total_jobs` zählt weiterhin das, was Sie angefragt haben,
`failed_job_ids` benennt genau die Jobs, die es nie geschafft haben,
die bereits eingereihten schließen normal ab, und
`SkipIfBatchCancelled` verwirft den Rest - `pending_jobs` erreicht
also weiterhin null, und Ihre `catch`/`finally`-Callbacks laufen
weiterhin. Wurde überhaupt nichts eingereiht, feuert `dispatch` sie
selbst ab, weil kein Worker mehr übrig ist, der es täte. Den
ursprünglichen Push-Fehler bekommen Sie in jedem Fall zurück.

### Batch-Optionen

| Option | Builder-Methode | Effekt |
| --- | --- | --- |
| Fehlschläge zulassen | `.allow_failures()` | Weiterplanen nach einem fehlgeschlagenen Job |
| Then-Callback | `.then(name)` | läuft, wenn jeder Job erfolgreich war |
| Catch-Callback | `.catch(name)` | läuft beim ersten Fehlschlag |
| Finally-Callback | `.finally(name)` | läuft, nachdem der Batch in jedem Fall abgeschlossen ist |
| Abgebrochene überspringen | `SkipIfBatchCancelled`-Middleware auf dem Job | verwirft verbleibende Jobs, wenn der Batch abgebrochen wird |

### `BatchCallback`-Implementierung

```rust
use async_trait::async_trait;
use suprnova::queue::{Batch, BatchCallback};
use suprnova::error::FrameworkError;

pub struct SendSummary;

#[async_trait]
impl BatchCallback for SendSummary {
    fn name(&self) -> &'static str { "send-summary-email" }

    async fn handle(&self, batch: Batch, error: Option<String>) -> Result<(), FrameworkError> {
        let subject = match error {
            Some(_) => format!("Batch {} failed", batch.name),
            None    => format!("Batch {} done - {} jobs", batch.name, batch.total_jobs),
        };
        // … Mail versenden
        Ok(())
    }
}
```

Registrieren Sie beim Boot mit
`batch::register_callback(Arc::new(SendSummary))`. Callbacks werden
über `name()` identifiziert - die Optionen des Batches speichern
Callback-Namen, sodass ein Prozess-Neustart registrierte Callbacks per
Lookup aufgreift, statt zu versuchen, eine Closure zu deserialisieren
(Rust-Closures serialisieren nicht).

## Eingereihte Chains

Sequenzielle Abläufe, bei denen jedes Glied erst läuft, nachdem der
Handler des vorherigen bestätigt hat:

```rust
Queue::chain()
    .add(GenerateReport { id: 99 })?
    .add(UploadToBucket { id: 99 })?
    .add(NotifyOwner { id: 99 })?
    .dispatch()
    .await?;
```

Die erste Envelope wird sofort eingereiht; der Rest reist im
Payload-Feld `chain_remaining` mit. Bei jedem erfolgreichen Abschluss
entnimmt der Worker den nächsten Eintrag und dispatcht ihn. Ein
Fehlschlag bricht die Chain - nachfolgende Glieder werden nie
eingereiht.

### Endgültiger Abschluss

Einen verketteten Job abzuschließen bedeutet zwei Dinge: den
Nachfolger einreihen und den gerade beendeten Job freigeben. Als zwei
getrennte Operationen gibt es keine sichere Reihenfolge. Zuerst
bestätigen, und ein Absturz in der Lücke verliert den Rest der Chain
permanent - in der Warteschlange bleibt nichts übrig, von dem aus
wiederholt werden könnte. Zuerst einreihen, und derselbe Absturz
liefert den beendeten Job erneut zu, sodass sein Handler erneut läuft
und der Nachfolger zweimal eingereiht wird.

Der Worker übergibt daher beides gleichzeitig an den Treiber, über
`QueueDriver::settle(token, follow_ups)`:

| Ergebnis | Bedeutung |
| --- | --- |
| `Settled::Atomically` | Nachfolger eingereiht und Reservierung in einer Transaktion aufgehoben |
| `Settled::Stale` | die Reservierung wurde von einem anderen Consumer zurückgefordert; **nichts** wurde eingereiht oder aufgehoben |
| `Settled::Unsupported` | dieser Treiber kann nicht transaktional abschließen |

`DatabaseQueueDriver` implementiert das: Beide Effekte sind eine
Transaktion, und das über den Reservierungsschlüssel adressierte
`DELETE` fungiert zugleich als Schranke. Ist Ihr Visibility-Timeout
abgelaufen, während der Handler lief, und ein anderer Worker hat den
Job aufgenommen, matcht das Delete nichts, die Transaktion rollt
zurück, und Sie erhalten `Stale` - ohne dass etwas eingereiht wurde.
Ein zweistufiger Abschluss kann das überhaupt nicht ausdrücken: Ihr
Push gelingt, der Push des neuen Besitzers gelingt, und die Chain
gabelt sich.

Redis und der In-Memory-Treiber antworten mit `Unsupported` und
behalten die Push-vor-Bestätigen-Reihenfolge, was permanenten Verlust
gegen ein **At-least-once**-Duplikat eintauscht. Das ist der
dokumentierte Vertrag des Frameworks, und deshalb werden verkettete
Envelope-IDs von ihrem Vorgänger abgeleitet statt zufällig - ein
erneut zugestellter Schritt reiht dieselbe ID erneut ein, die er zuvor
eingereiht hat, sodass das Duplikat als derselbe logische Schritt
erkennbar ist.

Wenn Sie einen Treiber schreiben, dessen Folge-Schreibvorgang und
Bestätigung sich eine Transaktionsdomäne teilen, implementieren Sie
`settle`. Sein Standard gibt `Unsupported` zurück, sodass Treiber, die
vor dessen Existenz geschrieben wurden, unverändert
weiterfunktionieren.

## Introspektion

```rust
Queue::size().await?;            // gesamt
Queue::pending_size().await?;    // available_at <= jetzt, nicht reserviert
Queue::delayed_size().await?;    // available_at > jetzt
Queue::reserved_size().await?;   // aktuell entnommen, noch nicht bestätigt
Queue::clear().await?;           // verwirft jede Envelope, gibt die Anzahl zurück
Queue::driver_name()?;           // konfigurierter Treibername für Logs / Admin
```

Der `QueueDriver`-Trait deklariert Standardwerte für `size` /
`pending_size` / `reserved_size` / `delayed_size` / `clear`;
`MemoryQueueDriver` und `DatabaseQueueDriver` implementieren sie
nativ. `RedisQueueDriver` gibt für `size` / `clear` einen
„unsupported“-Fehler zurück - verwenden Sie dafür die
Admin-`redis-cli`.

## Worker-Neustart-Signal

`php artisan queue:restart` übersetzt sich zu:

```rust
Queue::restart().await?;
```

Das Signal lebt in `Cache` als Millisekunden-Zeitstempel. Worker
pollen einmal pro Schleifendurchlauf und beenden sich sauber, wenn der
Zeitstempel neuer ist als ihre Startzeit. Kombinieren Sie das mit
einem Supervisor (systemd, Kubernetes, dem `supervisor`-Modul), damit
ein frischer Worker dort weitermacht, wo der vorherige aufgehört hat.

## Warteschlangen pausieren

`php artisan queue:pause` / `queue:resume` übersetzen sich zu:

```rust
Queue::pause(&connection, "billing").await?;
Queue::resume(&connection, "billing").await?;
Queue::pause_all().await?;
Queue::resume_all().await?;
```

oder über die CLI:

```bash
./app queue:pause billing
./app queue:pause --all
./app queue:resume billing
./app queue:resume --all      # Alias: queue:continue
```

Ein pausierter Worker beendet, was er bereits entnommen hat - Pausieren
unterbricht nie einen laufenden Job -, und hört dann auf, neue Arbeit zu
beanspruchen, bis er fortgesetzt wird. `pause_all` / `resume_all` sind
der globale Schalter; das Pausieren (oder Fortsetzen) einer benannten
Warteschlange betrifft nur diese. **`resume_all` hebt keine Pause pro
Warteschlange auf** - eine einzeln pausierte Warteschlange bleibt nach
einem globalen Resume pausiert, entsprechend Laravel. Löschen Sie sie
explizit mit `Queue::resume(&connection, "billing")`.

Beide Signale leben in `Cache`, neben dem Neustart-Signal oben:

| Schlüssel | Bedeutung |
| --- | --- |
| `suprnova:queues:paused` | globaler Schalter, gesetzt durch `pause_all` |
| `suprnova:queue:paused:{connection}:{queue}` | Schalter einer Warteschlange, gesetzt durch `pause` |

Prüfen Sie den Zustand mit
`Queue::is_paused(&connection, "billing").await?` (true, wenn einer der
beiden Schlüssel gesetzt ist) oder
`Queue::paused_queues(&connection, &queues).await?` (welche der
`queues` aktuell pausiert sind).

### Pausieren pro Warteschlange braucht ein benanntes `--queue`

Ein mit `--queue=billing,exports` gestarteter Worker beansprucht nur aus
diesen beiden Warteschlangen, sodass das Pausieren von `billing` diese
Liste für die Dauer der Pause auf `exports` verkleinert. Ein Worker ohne
`--queue` leert jede Warteschlange, die der Treiber enthält, und es gibt
keine Möglichkeit, dagegen „nur `billing` pausieren“ zu verlangen -
`QueueDriver::pop_from` meldet nie, welche Warteschlangennamen existieren,
also gibt es nichts, worauf ein Pause-Key pro Warteschlange geprüft
werden könnte. `pause_all` stoppt einen ungefilterten Worker weiterhin
vollständig; eine benannte Pause pro Warteschlange wirkt erst, wenn Sie
auch die Warteschlangen des Workers benennen.

### Pause-Abfragen deaktivieren

Setzen Sie `QUEUE_PAUSABLE=false`, und jeder Worker in diesem Prozess
ignoriert Pausensignale vollständig, ohne zusätzliche Cache-Lesevorgänge
pro Schleife. `queue:pause` (nicht `queue:resume`) verweigert dann
ebenfalls die Ausführung und beendet sich mit einem Fehlercode, sodass
ein Betreiber, der das Pausieren deaktiviert hat, sofort davon erfährt,
statt einen Pause-Befehl auszuführen, der still nichts tut. Spiegelt
Laravels `Worker::$pausable`.

### Warum Suprnova abweicht

Ein nicht erreichbarer Cache schlägt **offen** fehl: Ein Worker, der die Pause-Schlüssel nicht lesen kann, verhält sich als „nicht pausiert“ und leert weiter - derselbe Fail-open-Vertrag, den das Worker-Neustart-Signal oben bereits verwendet. Ein vorübergehender Cache-Ausfall sollte eine Worker-Flotte auf „Pause ignorieren“ degradieren, niemals auf „jeder Worker friert still ein“ - der Pause-Zustand ist ein explizites Opt-in-Signal, und seine eigene Nichtverfügbarkeit sollte nicht zu einem versteckten Kill-Switch werden.

## Graceful Shutdown

Das `CancellationToken` des Workers feuert an der nächsten
Entnahme-Grenze, niemals mitten im Dispatch. Ein Handler, der bereits
entnommen wurde, läuft bis zum Abschluss (begrenzt durch sein eigenes
`Job::timeout()`, falls gesetzt), bevor der Worker sich beendet. Das
bedeutet, dass in-flight Seiteneffekte nicht mitten im Schritt
abgerissen werden, aber ein SIGTERM kann bis zum Timeout pro Job
brauchen, bis alles geleert ist. Setzen Sie `WorkerConfig::max_jobs`
für eine Strategie periodischer Neustarts bei langlebigen Workern; der
Worker beendet sich nach so vielen Abschlüssen sauber, unabhängig vom
Ergebnis.

## Abschluss-Metriken

Der Worker emittiert bei jedem Bestätigen-/Ablehnen-Fehlschlag einen
Zähler `queue.settlement.failures` über [`Metrics`](observability.md).
Attribute: `operation` (`"ack"` | `"nack"`), `driver` (der Name des
konfigurierten Treibers), `job` (der `job_name`), `outcome`
(`"success"`, `"dead_letter"`, `"retry"`, `"deleted"`,
`"timeout_dead_letter"`, `"timeout_retry"`, `"released"`).

Eine von null abweichende Rate hier bedeutet, dass
**At-least-once**-Zustellung einen erfolgreichen Seiteneffekt erneut
zustellen oder die Versuchsverbuchung verlieren könnte - richten Sie
dafür einen expliziten Alarm ein.

## Typisierte Fehler

`MaxAttemptsExceeded`, `TimeoutExceeded` und `ManuallyFailed` spiegeln
Laravels `MaxAttemptsExceededException` / `TimeoutExceededException` /
`ManuallyFailedException`. Der Worker hängt die relevante Ursache an
das Dead-Letter-Event `JobFailed`, sodass Listener per Pattern-Match
statt per Substring-Suche in der Fehlermeldung arbeiten können.

## Connection-Benennung

Worker markieren jedes Lifecycle-Event mit einem Connection-Namen.
Standardmäßig ist das der `name()` des Treibers (z. B. `"memory"`,
`"redis"`, `"database"`). Apps, die mehrere Connections gleichzeitig
betreiben, können das überschreiben:

```rust
Queue::set_connection_name("orders-redis");
```

## Testen

Die Semantik von `Queue::fake()` lebt in `queue::testing`:

```rust
let _guard = suprnova::queue::testing::install_fake();
my_code_that_dispatches_jobs().await;

suprnova::queue::testing::assert_pushed::<SendWelcomeEmail>(|j| j.user_id == 42);

// Für verzögerte Dispatches den geplanten Zeitstempel festnageln:
suprnova::queue::testing::assert_pushed_later::<SendWelcomeEmail>(|j, at| {
    j.user_id == 42 && at > chrono::Utc::now()
});
```

Der Fake-Guard serialisiert parallele Tests über einen prozessweiten
Mutex; er erfasst `(payload, available_at, overrides)` pro Push und
räumt beim Drop auf. Das Feld `overrides` ist für jeden Einstiegspunkt
außer `push_with`/`later_with` `EnvelopeOverrides::default()` - siehe
[Mocking](mocking.md#warteschlange---queuetestinginstall_fake) für
`assert_pushed_on_queue`/`assert_pushed_on_connection` und
`pushed_with_overrides`, die zugehörigen Assertions. Im Fake-Modus
verzeichnet `push_unique` den Push immer als neu - Dedupe ist irrelevant,
wenn kein Treiber verdrahtet ist.

## Idempotenz ist der Vertrag des Workers mit Ihnen

Redis-gestützte Queue-Treiber können das Ablehnen einer Nachricht
nicht atomar machen - `XADD` und `XACK` sind getrennte Befehle. Ein
Absturz zwischen ihnen stellt die Nachricht über `XAUTOCLAIM` erneut
zu. In-Memory- und Database-Treiber sind **Exactly-once-pro-Versuch**,
aber die Worker-Schleife unterscheidet nicht zwischen Treibern, daher
**muss jeder Job-Handler in einem Produktions-Deployment idempotent
sein**.

Für typische, command-artige Jobs hüllen Sie den Handler-Rumpf in
[`Idempotency::once`](idempotency.md) oder
[`Idempotency::commit_on_success`](idempotency.md), identifiziert
durch einen stabilen Schlüssel pro Operation (Entitäts-ID, vom
Aufrufer mitgegebene Request-ID usw.). Wenn eine Wiederholung das
*ursprüngliche* Ergebnis zurückgeben muss, statt die erneute
Ausführung zu überspringen, verwenden Sie `Idempotency::remember`, das
den Erfolgswert aufzeichnet und ihn bei späteren Zustellungen
wiedergibt.

## Nächste Schritte

- [Bus](bus.md) - synchroner Dispatcher mit typisierten Ergebnissen
- [Ereignisse](events.md) - Pub/Sub-Fan-out
- [Idempotenz](idempotency.md) - der Vertrag, den Handler für At-least-once-Zustellung einhalten
- [Cache](cache.md) - unterstützt `push_unique`, `WithoutOverlapping`, `RateLimited`
- [Mocking](mocking.md) - jeder Fake-Guard, einschließlich `Queue::fake`
