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

Fünf Treiber werden im Baum ausgeliefert. Konfigurieren Sie sie über die
Umgebungsvariable `QUEUE_DRIVER` oder programmatisch über
`Queue::set_driver(...)`.

| Treiber | Wofür | Stärken |
| --- | --- | --- |
| `MemoryQueueDriver` | Tests, Einzelprozess-Apps | `tokio::time::DelayQueue` für `available_at`, mit virtueller Uhr kompatibel |
| `RedisQueueDriver` | Fan-out in der Produktion | Consumer-Gruppen + `XAUTOCLAIM` + ZSET-gestützte verzögerte Jobs |
| `DatabaseQueueDriver` | Apps mit einer einzigen DB | `FOR UPDATE SKIP LOCKED` auf Postgres/MySQL, `BEGIN`-serialisiert auf SQLite |
| `SyncQueueDriver` | Entwicklung, CI | führt den Handler bei `push` inline aus, ohne Worker |
| `NullQueueDriver` | Test-Wrapper | verwirft jeden Push, ohne ihn auszuführen |

`Queue::bootstrap_from_env()` liest `QUEUE_DRIVER` und verdrahtet den
passenden Treiber; `Queue::bootstrap_default()` verdrahtet immer den
Memory-Treiber. Der Boot-Pfad des Servers ruft eines von beiden für Sie
auf - die meisten Apps konfigurieren nur über die Umgebung.

`FailoverQueueDriver` ist kein sechster Treiber. Er umschließt eine
geordnete Liste der obigen Treiber, sodass ein Push, den eine Connection
ablehnt, an die nächste durchfällt. Siehe
[Failover-Connections](#failover-connections).

### Konfiguration über die Umgebung

```bash
QUEUE_DRIVER=redis
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_REDIS_STREAM=suprnova-queue
QUEUE_REDIS_GROUP=default
QUEUE_REDIS_CONSUMER=consumer-1
QUEUE_VISIBILITY_TIMEOUT_SECS=60

# Datenbank-Treiber - DB::init() muss zuerst laufen
QUEUE_DRIVER=database
QUEUE_DB_TABLE=jobs
```

Der Datenbank-Treiber validiert `QUEUE_DB_TABLE` bei der Konstruktion als
SQL-Bezeichner, sodass ein fehlerhafter Umgebungswert den Boot scheitern
lässt, statt bis zum Zusammensetzen des SQL zu gelangen. Redis nutzt
darunter sea-streamer-redis mit `AutoCommit::Disabled`; das
Visibility-Timeout wird bei der Konstruktion der Consumer-Gruppe
festgelegt, deshalb wird das Argument `visibility_timeout` pro Pop auf
Redis ignoriert (eine dokumentierte Abweichung vom Trait-Vertrag, die
Redis Streams erzwingt).

### Warum Suprnova abweicht

Laravel routet jedes Queueable über den Bus und unterscheidet
`ShouldQueue`-Jobs erst beim Dispatch. Suprnova trennt die beiden: `Bus`
für synchrone Arbeit, die ein typisiertes Ergebnis liefert, `Queue` für
asynchrone Arbeit, die einen Prozessabsturz überlebt. PHP braucht das
implizite Routing, weil sein Modell mit einer Anfrage pro Prozess es
sonst schwer macht, „mach das später, in einem anderen Prozess“
abzubilden. Tokio nicht - das explizite `Bus::dispatch` gegenüber
`Queue::push` ist klarer, schneller und macht die Entscheidung über die
Dauerhaftigkeit an der Aufrufstelle sichtbar. Das Nebeneinander steht in
[`bus.md`](bus.md).

## Failover-Connections

`FailoverQueueDriver` umschließt eine geordnete Liste von Connections.
Ein Push, den die erste Connection ablehnt, wird auf der nächsten
wiederholt, und so weiter die Liste hinunter, sodass ein Redis-Ausfall
nicht jeden Dispatch in einen verlorenen Job verwandelt.

Konfigurieren Sie ihn aus der Umgebung:

```bash
QUEUE_DRIVER=failover
QUEUE_FAILOVER_CONNECTIONS=redis,database

# Jede Connection liest ihre eigenen Variablen, genau so, wie sie es
# täte, wenn sie für sich allein QUEUE_DRIVER wäre.
QUEUE_REDIS_URL=redis://127.0.0.1:6379
QUEUE_DB_TABLE=jobs
```

Oder verdrahten Sie ihn selbst, wenn die Connections eine
Laufzeitkonfiguration brauchen, die sich in der Umgebung nicht ausdrücken
lässt:

```rust
use std::sync::Arc;
use std::time::Duration;
use suprnova::queue::{
    DatabaseQueueDriver, FailoverQueueDriver, Queue, QueueDriver, RedisQueueDriver,
};
use suprnova::{DB, FrameworkError};

pub async fn register() -> Result<(), FrameworkError> {
    let redis = RedisQueueDriver::connect(
        "redis://127.0.0.1:6379",
        "suprnova-queue",
        "default",
        "consumer-1",
        Duration::from_secs(60),
    )
    .await?;
    let database =
        DatabaseQueueDriver::new(DB::connection()?.inner().clone(), "jobs".to_string())?;

    let failover = FailoverQueueDriver::new(vec![
        ("redis".to_string(), Arc::new(redis) as Arc<dyn QueueDriver>),
        ("database".to_string(), Arc::new(database) as Arc<dyn QueueDriver>),
    ])?;
    Queue::set_driver(Arc::new(failover));
    Ok(())
}
```

Der `String` an jedem Eintrag ist die Bezeichnung der Connection, die im
`QueueFailedOver`-Ereignis gemeldet wird. Sie wird nicht aus dem
Treibertyp abgeleitet, denn zwei Connections können denselben Treiber
verwenden.

`QUEUE_FAILOVER_CONNECTIONS` ist erforderlich, wenn
`QUEUE_DRIVER=failover` gilt, und die Liste darf `failover` nicht selbst
enthalten. Ein Eintrag, der einen nicht existierenden Treiber benennt,
ist ein Boot-Fehler und nicht der Rückfall auf Memory mit Warnung, den
`QUEUE_DRIVER` auf sich selbst anwendet: Innerhalb einer Failover-Kette
würde ein Tippfehler, der still zu einer Connection im Speicher wird, ein
flüchtiges Backend in eine dauerhafte Liste stellen.

### Schreibzugriffe weichen aus, Lesezugriffe nicht

Nur `push` und `bulk_push` laufen die Connection-Liste ab. Jede andere
Operation - `pop`, `ack`, `nack`, `release`, `settle`, `clear`, die vier
Zähler und die drei Auflistungen zur Inspektion - geht an die **erste**
Connection und an keine andere.

Diese Asymmetrie ist der Vertrag, kein Versäumnis. Ein
Reservierungstoken hat nur für den Treiber Bedeutung, der es ausgestellt
hat, ein Ack gegen eine andere Connection würde also nichts abschließen
und beide beschädigen. Die Zähler und die Auflistungen folgen derselben
Regel, damit das, was Sie inspizieren, auch das ist, was der Worker auf
dieser Connection leert, statt einer Summe über Backends hinweg, die zur
Sicht keines Workers passt.

**Ein Worker auf der Failover-Connection leert nur die primäre.** Jobs,
die auf einen Fallback ausgewichen sind, brauchen einen Worker, der
direkt gegen diese Fallback-Connection läuft:

```bash
# Leert die primäre Connection der Failover-Kette.
QUEUE_DRIVER=failover QUEUE_FAILOVER_CONNECTIONS=redis,database ./app queue:work

# Leert, was auf die Datenbank ausgewichen ist. Starten Sie das ebenfalls.
QUEUE_DRIVER=database ./app queue:work
```

Laravels Dokumentation trägt dieselbe Warnung aus demselben Grund.

Das reicht bis zu den Chains, aber nur durch eine Tür. Ein Worker
schließt einen Job ab und reiht das nächste Glied einer
[eingereihten Chain](#eingereihte-chains) in einem einzigen Aufruf ein,
`settle`, und der Decorator delegiert diesen Aufruf allein an die
primäre Connection. Bei einer transaktionalen primären Connection wie dem
Datenbank-Treiber lässt eine ausgefallene primäre Connection den
Abschluss also fehlschlagen, und es weicht nichts aus: Der Worker lässt
die Reservierung intakt, und der Ablauf der Sichtbarkeit stellt den Job
erneut zu. Das Durchfallen passiert, wenn die primäre Connection mit
`Settled::Unsupported` antwortet, was der Memory- und der Redis-Treiber
tun, denn der Worker schiebt das nächste Glied dann wie jeden anderen
Push über den gebundenen Treiber - und dieser Push weicht aus. Der Rest
dieser Chain wartet dann auf einen Worker auf der Fallback-Connection.
Ohne einen solchen bleibt die Chain stehen - das Glied ist dauerhaft und
nichts geht verloren, aber es führt auch nichts aus.

### Das Ereignis `QueueFailedOver`

Jede Connection, die einen Push ablehnt, löst
`queue::events::QueueFailedOver { connection, job_name, exception }` aus,
aber nur bei dem Push, der diese Connection *in* den Fehlerzustand
bringt. Eine Connection, von der bereits bekannt ist, dass sie
fehlschlägt, bleibt still, bis ein späterer Push auf ihr gelingt und sie
neu scharf stellt. Ein vierstündiger Ausfall erzeugt ein Ereignis, nicht
eines pro Dispatch, und genau das macht es als Alarm brauchbar.

`connection` ist die Bezeichnung der Connection, die fehlgeschlagen ist,
nicht die derjenigen, die den Job angenommen hat.

Wenn jede Connection einen Push ablehnt, gibt der Push den Fehler der
letzten Connection zurück. `bulk_push` schiebt jedes Envelope einzeln,
sodass jedes für sich durchfällt: Ein Batch, den die primäre Connection
halb angenommen hat, wird nie geschlossen erneut auf den Fallback
geschoben, und jedes Envelope behält das `available_at`, mit dem es
gebaut wurde. Ein Batch ist nicht atomar. Wenn ein Envelope von jeder
Connection abgelehnt wird, gibt `bulk_push` den Fehler dieses Envelopes
zurück, während die früheren Envelopes bereits eingereiht sind.

Ein Failover ist keine Deduplizierung. Der Decorator versucht ein
Envelope, das eine Connection angenommen hat, nie erneut; eine
Connection, die das Envelope schreibt und *dann* einen Fehlschlag
meldet, erzeugt aber auf der nächsten Connection ein Duplikat, denn
„geschrieben und die Bestätigung verloren“ ist von „nie angenommen“
nicht zu unterscheiden. Beide Kopien tragen dieselbe Job-ID. Das ist der
At-least-once-Vertrag des Frameworks für die Zustellung, derselbe, der
die Idempotenz von Handlern auch überall sonst zur Pflicht macht - siehe
[Idempotenz ist der Vertrag des Workers mit Ihnen](#idempotenz-ist-der-vertrag-des-workers-mit-ihnen).

### Warum Suprnova abweicht

Laravels Failover-Connection ist ein `connections`-Array in
`config/queue.php`, aufgelöst über die Connection-Registry. Suprnova hat
keine Treiber-Registry pro Connection - ein Treiber wird prozessweit
gebunden -, deshalb kommen die Bezeichnungen aus
`QUEUE_FAILOVER_CONNECTIONS` (oder aus dem `String`, den Sie an
`FailoverQueueDriver::new` übergeben), und Lesezugriffe delegieren an den
ersten *Treiber* statt an eine benannte Connection.

Laravels `FailoverQueue::bulk` durchläuft die Jobs einzeln, damit die
Verzögerung jedes einzelnen erhalten bleibt. Suprnova löst die
Verzögerung auf das Envelope auf, bevor irgendein Treiber es sieht, die
Schleife pro Envelope bewahrt sie also ohne Zutun - aber die Schleife
ist weiterhin das, was einen halb gelandeten Batch vor einem doppelten
Push bewahrt, deshalb bleibt sie.

## Push-Varianten

Jede Push-Variante nimmt einen typisierten `J: Job`-Wert entgegen und
kehrt zurück, wenn das Envelope an den Treiber übergeben ist - nicht,
wenn der Handler läuft.

| Methode | Verhalten |
| --- | --- |
| `Queue::push(job)` | sofort einreihen |
| `Queue::push_later(job, at)` | verfügbar zu einem bestimmten `DateTime<Utc>` |
| `Queue::later(delay, job)` | verfügbar nach `delay` ab jetzt |
| `Queue::push_with(job, overrides)` | sofort einreihen, mit `EnvelopeOverrides` pro Push |
| `Queue::push_after_commit(job)` | einreihen, wenn die umgebende `DB::transaction` committet |
| `Queue::later_with(delay, job, overrides)` | verfügbar nach `delay` ab jetzt, mit `EnvelopeOverrides` pro Push |
| `Queue::push_unique(job)` | dedupliziert über `J::unique_id` innerhalb von `J::unique_for`, gibt `Ok(true)` zurück, wenn das Envelope geschoben wurde, und `Ok(false)`, wenn ein lebender Dedupe-Schlüssel es unterdrückt hat |
| `Queue::push_unique_later(job, at)` | unique + geplant |
| `Queue::later_unique(delay, job)` | unique + verzögert |
| `Queue::bulk(vec![job1, job2, ...])` | jeden Job schieben (der Treiber darf einen nativen Bulk-Pfad nutzen) |

`push_unique` setzt voraus, dass die Cache-Schicht gebootstrappt ist - die
Dedupe-Sperre lebt in [`Cache`](cache.md), über
[`Idempotency::commit_on_success`](idempotency.md). Ein fehlgeschlagener
Push gibt den Dedupe-Schlüssel frei, sodass der Aufrufer es erneut
versuchen kann; ein erfolgreicher Push hält ihn
`J::unique_for` Sekunden lang. Der Job muss `Job::unique_id(&self)`
überschreiben und `Some(id)` zurückgeben - `None` liefert einen internen
Fehler.

Der Boolean beantwortet eine Frage - „liegt dieser Job auf der Queue?“ -,
und dahinter steckt ein dritter Fall. Geht die Lease der Dedupe-Sperre
verloren, während der Push unterwegs ist, wird der Push trotzdem
abgeschlossen (die Idempotenzschicht bricht nie einen Rumpf ab, der schon
gewirkt haben könnte), und Sie bekommen weiterhin `Ok(true)`, dazu ein Log
auf `warn`-Ebene, das den Job und seinen Unique-Schlüssel benennt. Der Job
ist eingereiht; unbewiesen bleibt, dass nicht jemand anderes nebenläufig
denselben eingereiht hat. Ihr Handler muss ohnehin eine erneute Zustellung
vertragen, das braucht also keine zusätzliche Behandlung - aber das Log
ist da, weil ein Schwall davon bedeutet, dass der Cache hinter Ihrer
Dedupe-Sperre ins Straucheln gerät.

### Unique bis zur Verarbeitung

Eine Eindeutigkeitssperre hält normalerweise das gesamte
`unique_for`-Fenster, auch nachdem der Job gelaufen ist. Wenn die Sperre
dazu da ist, *eingereihte* Duplikate zusammenzufassen, statt die
Ausführung zu serialisieren, entscheiden Sie sich dafür, sie in dem
Moment freizugeben, in dem die Verarbeitung beginnt:

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct RebuildSearchIndex {
    index: String,
}

#[async_trait]
impl Job for RebuildSearchIndex {
    fn job_name() -> &'static str { "rebuild-search-index" }
    fn unique_id(&self) -> Option<String> { Some(self.index.clone()) }
    fn unique_until_processing() -> bool { true }
    fn unique_for() -> Duration { Duration::from_secs(3600) }

    async fn handle(self) -> Result<(), FrameworkError> {
        // Ein Rebuild, der 20 Minuten läuft, schluckt den erneuten
        // Dispatch aus Minute 2 nicht mehr.
        Ok(())
    }
}
```

Der Worker gibt die Sperre nach dem Middleware-Durchlauf des Jobs frei
und unmittelbar bevor der Handler läuft. Daraus folgen vier Dinge:

- Ein Job, den eine Middleware zurück auf die Queue legt, behält seine
  Sperre. Er hat die Verarbeitung nicht begonnen, für ein Duplikat hat
  sich also nichts geändert.
- Ein Job, den eine Middleware auf andere Weise kurzschließt, gibt seine
  Sperre auf, denn er wird überhaupt nicht mehr verarbeitet werden. Das
  umfasst das Löschen des Jobs, das Verschieben ins Dead-Letter und das
  Melden als abgeschlossen, ohne den Handler je aufzurufen.
- Ein Job, der fehlschlägt, gibt seine Sperre frei und wird trotzdem
  wiederholt. Die Sperre ging in dem Moment, in dem die Verarbeitung
  begann, ein Duplikat kann sich also einreihen, während der
  fehlgeschlagene Versuch seinen Backoff abwartet, und Sie enden mit zwei
  Envelopes für dieselbe Unique-ID. Das ist der Handel, den dieses Opt-in
  eingeht. Muss eine Wiederholung den Platz weiter halten, lassen Sie
  `unique_until_processing` aus und lassen Sie das `unique_for`-TTL die
  ganze Versuchskette abdecken.
- Die Freigabe ist an den Besitzer gebunden. `push_unique` vermerkt das
  Besitz-Token der Sperre auf dem Envelope, und der Worker gibt mit diesem
  Token frei, ein erneut zugestellter Versuch kann also nie eine Sperre
  freigeben, die inzwischen ein neuerer Dispatch erworben hat.

`unique_until_processing` braucht dieselben zwei Dinge wie `push_unique`:
ein `unique_id`, das `Some(id)` zurückgibt, und eine gebootstrappte
Cache-Schicht.

Unter dem `sync`-Treiber läuft der Handler direkt innerhalb des
`push_unique`-Aufrufs, der die Sperre genommen hat, der Job gibt also eine
Sperre frei, die sein eigener Aufrufer nominell noch hält. Läuft dieser
Handler länger als ein Drittel von `unique_for`, bemerkt der Erneuerer der
Dedupe-Lease, dass die Sperre weg ist, und protokolliert eine Warnung über
die verlorene Lease, und `push_unique` protokolliert obendrein seine
eigene Warnung, dass die Exklusivität nicht bewiesen werden konnte. Beides
ist hier erwartet und kein Fehler: Der Job lief, der Push gibt `Ok(true)`
zurück, und die Sperre ist weg, weil der Job sie selbst freigegeben hat.

### Warum Suprnova abweicht

Laravel gibt die Sperre eines *gewöhnlichen* Unique-Jobs frei, sobald der
Handler zurückkehrt. Suprnova lässt diese Sperre stattdessen mit dem
`unique_for`-TTL ablaufen, was das Dedupe-Fenster ehrlich hält, wenn ein
Worker mitten im Job stirbt: Das Fenster, das Sie konfiguriert haben, ist
das Fenster, das Sie bekommen, ganz gleich, ob der Handler je zurückkehrte.
`unique_until_processing` verhält sich in beiden Frameworks gleich.

Suprnova erzwingt außerdem nie die Freigabe einer Eindeutigkeitssperre.
Laravel fällt bei einem ersten Versuch, der kein Besitz-Token trägt, auf
eine erzwungene Freigabe zurück. Die einzigen Envelopes, die einen
Suprnova-Worker ohne ein solches erreichen, sind Envelopes, die eingereiht
wurden, bevor es das Token gab, und die behalten den TTL-Ablauf, statt eine
Freigabe zu riskieren, die die Sperre eines neueren Dispatches löscht.

### Entprellen - den letzten Dispatch behalten, nicht den ersten

`push_unique` unterdrückt ein Duplikat und behält den **ersten** Dispatch.
Entprellen ist das Gegenteil: Es behält den **letzten**. Ein Schwall von
zwanzig „diese Bestellung hat sich geändert“-Events wird zu einer
Neuindizierung, ein Fenster nach dem zwanzigsten, mit der neuesten Payload.

```rust
use std::time::Duration;
use suprnova::{FrameworkError, Job, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct ReindexOrder {
    order_id: u32,
}

#[async_trait]
impl Job for ReindexOrder {
    fn job_name() -> &'static str { "reindex-order" }
    fn debounce_for() -> Option<Duration> { Some(Duration::from_secs(30)) }
    fn max_debounce_wait() -> Option<Duration> { Some(Duration::from_secs(300)) }
    fn debounce_id(&self) -> Option<String> { Some(self.order_id.to_string()) }

    async fn handle(self) -> Result<(), FrameworkError> {
        Ok(())
    }
}
```

- `debounce_for` ist das Fenster: Jeder Dispatch stellt es neu scharf, der
  Lauf geschieht also 30 Sekunden nach dem *jüngsten*.
- `max_debounce_wait` verhindert, dass ein durchgehender Schwall die
  Arbeit ewig aufschiebt. Sobald der Schwall fünf Minuten lang
  aufgeschoben hat, wird der nächste Dispatch ohne Verzögerung eingereiht.
  Das Fenster startet dann neu, jeder Schwall misst seine maximale
  Wartezeit also ab seinem eigenen ersten Dispatch.
- `debounce_id` grenzt das Fenster ein. Zwanzig Aktualisierungen an
  Bestellung 7 werden zu einem Lauf; eine Aktualisierung an Bestellung 8
  bleibt davon unberührt. Lassen Sie es weg, und jeder Dispatch des Jobs
  teilt sich ein Fenster.

Jeder Dispatch wird trotzdem eingereiht. Das Zusammenfassen wird im Worker
entschieden: Jeder Push überschreibt ein Cache-Token, und der Worker
verwirft jedes Envelope, dessen Token ein neuerer Dispatch ersetzt hat,
bestätigt es und gibt `JobDebounced` aus. Genau dadurch trägt der
überlebende Lauf die neueste Payload statt der ältesten. Ist das Token
abgelaufen oder verdrängt worden, läuft der Job - das Entprellen ist
Fail-open, denn ein verlorenes Token ist kein Beleg dafür, dass jemand
anderes das Fenster besitzt.

Der [`sync`-Treiber](#treiber) hat keinen Worker, er führt daher jeden
Dispatch direkt aus, und nichts wird je zusammengefasst. Laravels
sync-Treiber verhält sich genauso. `Queue::bulk` schiebt auf Treiberebene
und stellt ebenfalls kein Fenster scharf, ein entprellter Job, der per
Bulk geschoben wird, läuft also in jeder Kopie. Laravels `Queue::bulk`
überspringt seinen eigenen Debounce-Erwerb aus demselben Grund.

Setzen Sie das Fenster stattdessen an der Aufrufstelle, wenn es dem
Aufrufer gehört:

```rust
use suprnova::queue::DebounceOptions;

Queue::push_debounced(
    ReindexOrder { order_id: 7 },
    DebounceOptions::new(Duration::from_secs(30))
        .max_wait(Duration::from_secs(300))
        .id("7"),
)
.await?;
```

Ein Job kann nicht zugleich `debounce_for` und `unique_id` deklarieren:
Eindeutigkeit behält den ersten Dispatch eines Schwalls und Entprellen den
letzten, der Push gibt daher einen Fehler zurück, der beide benennt.
Chains und Batches lehnen einen entprellten Job aus einem verwandten Grund
ab - ein überholtes Glied wird verworfen, was den Rest einer Chain
stranden ließe, und ein verworfener Batch-Job lässt den Zähler der
ausstehenden Jobs über null stehen, sodass die Callbacks des Batches nie
feuern.

### Overrides pro Push mit `EnvelopeOverrides`

`Queue::push_with` und `Queue::later_with` nehmen neben dem Job ein
`EnvelopeOverrides` entgegen, für den einen Dispatch, der eine andere
Queue, Connection, ein anderes Timeout oder ein anderes
Wiederholungsverhalten braucht als die Standardwerte des Jobs:

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

// Das verzögerte Gegenstück, das das Verhältnis von `Queue::later` zu
// `Queue::push` spiegelt.
Queue::later_with(Duration::from_secs(60), SendWelcomeEmail { user_id: 42 }, overrides).await?;
```

Jedes Feld ist standardmäßig `None` und überlässt die Auflösung dem
normalen Weg, den `Queue::push` ohnehin geht; ein `Some`-Feld sticht für
diesen einen Push alles davon aus, sowohl eine mit
[`Queue::route`](#queue-routing) registrierte Route als auch die
`Job::*`-Deklaration des Jobs für dieses Feld:

| Feld | Sticht aus |
| --- | --- |
| `queue` | `Queue::route`, `Job::queue()` |
| `connection` | `Queue::route`, `Job::connection()` |
| `timeout` | `Job::timeout()` |
| `fail_on_timeout` | `Job::fail_on_timeout()` |
| `max_tries` | `Job::max_tries()` |
| `backoff` | `Job::backoff()` |
| `after_commit` | `Job::after_commit()` |

`EnvelopeOverrides` ist das Primitiv, auf dem sowohl
`Mail::on_queue`/`.on_connection()` als auch die Queue-Feinabstimmung pro
Benachrichtigung von `Notify::queue` aufbauen - siehe
[Mail](mail.md#queueing) und [Benachrichtigungen](notifications.md).

### Vom Job deklarierte Verzögerung

Ein Job kann seine eigene Standardverzögerung tragen, statt dass jede
Aufrufstelle `Queue::later(Duration::from_secs(60), job)` wiederholt:

```rust
impl Job for SendDigest {
    // ...
    fn delay() -> Option<Duration> { Some(Duration::from_secs(60)) }
}
```

`Queue::push(job)`, `Queue::push_with(job, overrides)`,
`Queue::push_unique(job)` und `Queue::bulk(vec![job1, job2])` beachten sie
alle - `available_at` wird zu `now + J::delay()` statt zu `now`.
`Queue::bulk` löst die Verzögerung einmal pro Aufruf auf, da jeder Job im
Vektor dasselbe konkrete `J` und damit dasselbe `Job::delay()` teilt.

Eine ausdrückliche Verzögerung an der Aufrufstelle gewinnt immer:
`Queue::push_later(job, at)`, `Queue::later(delay, job)`,
`Queue::later_with(delay, job, overrides)`,
`Queue::push_unique_later(job, at)` und `Queue::later_unique(delay, job)`
nutzen alle den Zeitstempel oder die Verzögerung, die der Aufrufer
übergeben hat, wörtlich - `Job::delay()` wird für keine davon
herangezogen. Greifen Sie zur Trait-Methode, wenn jeder Dispatch eines
Job-Typs standardmäßig verzögert starten soll; greifen Sie zu einer der
`later`/`push_later`-Varianten, wenn ein bestimmter Dispatch eine
Verzögerung braucht, die der Typ sonst nicht deklariert.

Batches und Chains ziehen sie ebenfalls nicht heran:
`Queue::batch()...add(job)` und `Queue::chain()...add(job)?` bauen ihre
Envelopes beide mit einem `available_at`, das auf den Moment Ihres
`add`-Aufrufs gesetzt ist, ein Job mit deklariertem `Job::delay()` wird
als Teil eines Batches oder einer Chain also sofort ausgeliefert, obwohl
ein bloßes `Queue::push(job)` desselben Jobs warten würde. Geben Sie dem
Job seine Verzögerung auf anderem Weg - über ein Feld auf dem Job selbst,
angewandt in `handle()` -, wenn ein Schritt im Batch oder in der Chain
eine braucht.

### Warum Suprnova abweicht

Laravels `$job->delay` ist eine Instanz-Eigenschaft, pro Dispatch gesetzt
(`SendDigest::dispatch($user)->delay(60)`), zwei Dispatches derselben
Klasse können also unterschiedliche Verzögerungen tragen. `Job::delay()`
ist hier stattdessen ein Standardwert auf Klassenebene, wie `Job::queue()`
oder `Job::max_tries()` - ein Dispatch, der eine aus seinen eigenen Daten
berechnete Verzögerung braucht, nutzt `Queue::later`/`push_later`, was den
deklarierten Standard ohnehin aussticht.

### Dispatch nach dem Commit

Ein Job, der innerhalb einer
[`DB::transaction`](database.md#transactions) geschoben wird, liefert sich
ein Rennen mit dieser Transaktion. Ein Worker in einem anderen Prozess
kann das Envelope poppen, nach der Zeile suchen, die die Transaktion noch
offen hält, und scheitern - oder schlimmer: Die Transaktion rollt zurück,
und der Job läuft gegen Daten, die es nicht mehr gibt.

Melden Sie den Job zum Warten auf den Commit an:

```rust
use suprnova::{DB, FrameworkError, Job, Queue, async_trait};

#[derive(serde::Serialize, serde::Deserialize)]
struct SendReceipt {
    order_id: i64,
}

#[async_trait]
impl Job for SendReceipt {
    fn job_name() -> &'static str { "send-receipt" }
    fn after_commit() -> bool { true }

    async fn handle(self) -> Result<(), FrameworkError> {
        // Die Bestellzeile ist garantiert dauerhaft, wenn dies läuft.
        Ok(())
    }
}

DB::transaction(|_tx| {
    Box::pin(async move {
        let order = Order::create(suprnova::attrs! { total: 4999i64 }).await?;
        // Hier erreicht nichts den Treiber.
        Queue::push(SendReceipt { order_id: order.id }).await?;
        Ok::<(), FrameworkError>(())
    })
})
.await?;
// Das Envelope liegt jetzt auf der Queue, und erst jetzt.
```

Drei Regeln decken jeden Fall ab:

- **Innerhalb einer Transaktion wartet der gesamte Push auf den Commit.**
  Nicht nur der Schreibvorgang zum Treiber: Der Aufbau des Envelopes, das
  `JobQueueing`-Event und das `JobQueued`-Event geschehen ebenfalls zum
  Commit-Zeitpunkt, einem Listener wird also nie ein Job gemeldet, den ein
  Rollback danach verwirft.
- **Ein Rollback verwirft ihn.** Der Push findet schlicht nie statt. Hat er
  eine Eindeutigkeitssperre genommen, gibt der Rollback diese Sperre
  zurück.
- **Außerhalb einer Transaktion geschieht der Push sofort.** Genau das
  macht das Opt-in auf dem Job-Typ sicher deklarierbar: Eine
  Dispatch-Stelle muss nicht wissen, ob der Codepfad, auf dem sie sitzt,
  transaktional ist.

Ein Rollback zu einem [Savepoint](database.md#savepoints) zählt für alles,
was darin registriert wurde, als Rollback. `tx.rollback_to("name")`
verwirft die seit `tx.savepoint("name")` aufgeschobenen Pushes und gibt
die Sperren frei, die sie genommen haben, und zwar sofort, sodass ein
erneuter Dispatch innerhalb derselben Transaktion den Schlüssel wieder
gewinnt. Pushes von vor dem Savepoint bleiben unberührt, und ein
Savepoint, den Sie nie zurückrollen, behält alles, was darin registriert
wurde.

Nutzen Sie `EnvelopeOverrides::after_commit`, wenn Sie pro Dispatch statt
pro Job-Typ entscheiden wollen. `Some(true)` ist Laravels `afterCommit()`
und hat die Kurzform `Queue::push_after_commit(job)`; `Some(false)` ist
Laravels `beforeCommit()`, für den einen Dispatch, der für einen Worker
sichtbar sein muss, bevor der Commit landet:

```rust
use suprnova::queue::{EnvelopeOverrides, Queue};

// Einen Job aufschieben, dessen Typ sich nicht dafür anmeldet.
Queue::push_after_commit(SendWelcomeEmail { user_id: 42 }).await?;

// Sofort schieben, obwohl der Job-Typ sich dafür anmeldet.
Queue::push_with(
    SendReceipt { order_id: 7 },
    EnvelopeOverrides { after_commit: Some(false), ..Default::default() },
)
.await?;
```

Ein aufgeschobenes `Queue::push` löst
[`Job::delay()`](#vom-job-deklarierte-verzögerung) gegen den Commit neu
auf, nicht gegen den Push, denn die Verzögerung bedeutet „so lange nach
dem Dispatch warten“, und bei einem aufgeschobenen Job *ist* der Dispatch
der Commit. Ein ausdrücklicher Zeitstempel ist die Absicht des Aufrufers
zu einem Zeitpunkt, `Queue::push_later`, `Queue::later` und
`Queue::later_with` tragen ihren daher unverändert durch den Aufschub.

`Queue::push_unique` schiebt mit einer bewussten Asymmetrie auf: Die
Dedupe-Sperre wird sofort genommen, ein zweites `push_unique` für dieselbe
Unique-ID innerhalb derselben Transaktion wird also weiterhin unterdrückt
und meldet weiterhin `Ok(false)`. Nur das Envelope wartet. Der Gewinner
meldet `Ok(true)`, obwohl sein Push noch aussteht, denn der Push wird
stattfinden. Ein Rollback gibt die genommene Sperre besitzergebunden
frei, sodass das `unique_for`-Fenster nie von einem Dispatch blockiert
wird, der nie stattfand - und genauso verhält sich jedes andere Ende, bei
dem der Commit ausbleibt, ein abgelehntes `COMMIT` eingeschlossen. Die eine
Grenze dieser Garantie ist das TTL selbst: Bei einer Transaktion, die
länger offen bleibt als `unique_for`, kann die Sperre ablaufen und mitten
im Flug von einem anderen Dispatch erneut genommen werden, geben Sie
`unique_for` also Luft oberhalb Ihrer längsten Transaktion, wenn die
Deduplizierung zählt. Die `push_unique*`-Familie nimmt keine
`EnvelopeOverrides` entgegen, `Job::after_commit()` ist also das Einzige,
was darüber entscheidet, ob ein Unique-Push aufschiebt - es gibt dafür
kein Override pro Push.

Batches und Chains schieben nicht auf, genauso wenig wie sie
`Job::delay()` heranziehen: `Queue::batch()` und `Queue::chain()` bauen
ihre Envelopes und schieben sie direkt. Umschließen Sie den
`.dispatch()`-Aufruf so, dass er nach der Rückkehr der Transaktion läuft,
wenn ein Batch auf einen Commit warten muss.

Eingereihte [Mail](mail.md#queueing) und
[Benachrichtigungen](notifications.md) schieben ebenfalls nicht auf. Beide
reiten auf einem einzigen geteilten Job-Typ (`SendMailJob` /
`SendNotificationJob`), und es gibt auf `Mailable` oder `Notification`
noch keine Entsprechung zu `ShouldQueueAfterCommit`, ein Aufruf von
`Mail::queue` oder `Notify::queue` innerhalb einer Transaktion erreicht
den Treiber also sofort. Senden Sie diese nach der Rückkehr der
Transaktion.

Unter `Queue::fake()` wird ein Push sofort aufgezeichnet, Aufschub
inklusive, sodass ein Test darauf assertieren kann, ohne etwas zu
committen. Das entspricht Laravels `Bus::fake`, und genau das erlaubt es
einem Test, einen transaktionalen Handler anzutreiben und im selben Atemzug
auf dessen Dispatches zu assertieren.

### Warum Suprnova abweicht

`Queue::bulk` ist monomorph - jedes Element teilt ein konkretes `J` -,
seine Aufteilung nach dem Commit ist für den Aufruf also alles oder
nichts. Laravel teilt ein heterogenes Array in eine aufgeschobene und eine
sofortige Hälfte; hier gibt es nichts aufzuteilen.

Der Aufschub hängt an der Closure-Form. Ein Push innerhalb eines manuellen
[`DB::begin_transaction`](database.md#manual-form) geschieht **sofort**,
denn der manuelle Modus installiert keine umgebende Transaktion und hat
daher keinen Commit, an den sich ein Callback hängen ließe. Dort
aufzuschieben würde einen Callback einreihen, den nichts je ausführt, und
ein Dispatch, der stillschweigend verschwindet, ist schlimmer als einer,
der zu früh geschieht. Greifen Sie zu `DB::transaction`, wenn ein Dispatch
auf den Commit warten muss.

Laravel liest außerdem als letzten Ausweg in seiner Vorrangkette einen
Config-Schlüssel `after_commit` auf Connection-Ebene. Suprnova hört beim
Override pro Push und danach beim `Job::after_commit()` des Jobs auf:
Queue-Connections tragen hier keine eigene Dispatch-Richtlinie.

## Job-Konfiguration

Überschreiben Sie die assoziierten Funktionen von `Job`, um das Verhalten
pro Implementierung abzustimmen:

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
    fn unique_until_processing() -> bool { true }          // Standard: false (das TTL ist das Fenster)
    fn middleware() -> Vec<std::sync::Arc<dyn JobMiddleware>> {
        vec![/* siehe „Job-Middleware“ weiter unten */]
    }
}
```

## Queue-Routing

Standardmäßig geht jeder Job an eine Queue, und jeder Worker leert sie
vollständig. Sobald manche Jobs langsamer oder wichtiger sind als
andere, wollen Sie dedizierte Worker-Pools: Ein lang laufender Export
sollte nicht hinter tausend Willkommens-Mails feststecken.

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

1. ein an `Queue::push_with` / `Queue::later_with` übergebenes Override
   pro Push (siehe [Overrides pro Push mit
   `EnvelopeOverrides`](#overrides-pro-push-mit-envelopeoverrides))
2. eine mit `Queue::route` registrierte Route
3. die eigenen `Job::queue` / `Job::connection` des Jobs
4. der Treiber- / globale Standard

Wird für ein Feld `None` übergeben, bleibt diese Dimension unberührt,
sodass das Routen der Connection eines Jobs die bereits deklarierte
Queue nicht stört.

Die beiden Dimensionen laufen heute auf unterschiedlichen Tiefen. Die
**Queue** wird end-to-end respektiert - auf das Envelope gestempelt,
vom Treiber gespeichert, über `--queue` gefiltert. Die **Connection**
löst den Connection-*Namen* auf, der auf den Lifecycle-Events
`JobQueueing` / `JobQueued` mitgeführt wird - das ist es, was Listener
und Dashboards sehen; ein einziger prozessglobaler Treiber empfängt
weiterhin jeden Push, sodass das Routen der Connection eines Jobs noch
keinen anderen Treiber auswählt. Connections jetzt schon zu
deklarieren ist zukunftskompatibel für die Zeit, wenn
Pro-Connection-Treiber landen, nicht verhaltensrelevant.

Dedizieren Sie dann einen Worker dafür:

```bash
./app queue:work --queue=billing
./app queue:work --queue=exports,heavy
./app queue:work                       # leert wie zuvor jede Queue
```

Ein Job ohne Route gehört zu `default`, daher leert `--queue=default`
ungeroutete Arbeit, statt sie stranden zu lassen.

### Eine ganze Queue weiterleiten

`Queue::route` ist nach Job-Typ geschlüsselt. Wenn Sie einen Pool über
einen anderen leeren wollen - eine Queue stilllegen, einen Rückstau
aufnehmen, Arbeit von einem Pool wegholen, den Sie gleich abschalten -,
schlüsseln Sie die Umleitung stattdessen nach Queue-Namen:

```rust
// bootstrap::register()
use suprnova::Queue;

Queue::forward("default", "high");
Queue::forward_on("exports", "heavy", "redis");   // nur auf der `redis`-Connection
```

Die Connection in `forward_on` ist ein Gate und wird mit dem
Connection-Namen dieses Prozesses verglichen -
`Queue::set_connection_name`, falls Sie einen gesetzt haben, sonst der
eigene Name des Treibers. Sie wird nicht mit `Job::connection` des Jobs
verglichen, nicht mit der Connection eines `Queue::route` und nicht mit
einer `EnvelopeOverrides`-Connection pro Push: Diese benennen, was die
Lifecycle-Events melden, und einem Worker steht nur der Prozessname zur
Verfügung, um die Liste zu begrenzen, aus der er Jobs beansprucht. Beide
Hälften der Umleitung hängen an diesem einen Wert, eine Weiterleitung
kann den Push also niemals verschieben, ohne den Anspruch
mitzuverschieben.

Die Umleitung greift auf beiden Seiten, und genau das bewahrt sie
davor, Arbeit stranden zu lassen:

- **Auf der Push-Seite** wird der Name umgeschrieben, nachdem das
  Routing und der eigene `Job::queue` des Jobs ihr Wort hatten, und
  nach einer `EnvelopeOverrides`-Queue pro Push, falls Sie eine
  übergeben haben.
- **Auf der Pop-Seite** leert ein mit `--queue=default` gestarteter
  Worker `high`. Ohne diese Hälfte würde die Ziel-Queue Jobs sammeln,
  die kein Worker beansprucht.

Ein ganz ohne `--queue` gestarteter Worker leert ohnehin schon alles,
für ihn ändert eine Weiterleitung also nichts. `default`
weiterzuleiten fängt Jobs ein, die keine Queue benannt haben, denn ein
ungerouteter Job gehört zu `default`.

Eine Weiterleitung ist ein einzelner Nachschlag, nie eine Kette. Sind
`a -> b` und `b -> c` registriert, landet ein Push, der sich zu `a`
aufgelöst hat, auf `b`. `b -> a` zusätzlich zu einem bestehenden
`a -> b` zu registrieren ist deshalb ein stimmiger Pool-Tausch und
keine Schleife: Ein Push auf `a` landet weiterhin auf `b`, ein Push auf
`b` landet nun auf `a`, und ein auf einem der beiden Namen gestarteter
Worker beansprucht den jeweils anderen - nichts verkettet sich, also
strandet auch nichts. Eine längere Rotation über mehr Queue-Namen löst
sich genauso auf, ein unabhängiger Sprung nach dem anderen. Auch
Laravels `Queue::forward` hat keine Zyklusprüfung, und zwar aus
demselben Grund: Sein Resolver ist derselbe einzelne Nachschlag. Eine
Queue auf ihren eigenen Namen weiterzuleiten ist die Identität - gar
keine Umleitung -, und so neutralisieren Sie eine bereits registrierte
Weiterleitung.

Nur zukünftige Pushes wandern. Envelopes, die bereits auf der
Quell-Queue liegen, bleiben dort, und der Worker, der sie bislang
geleert hat, beansprucht nun das Ziel - leeren Sie den Quell-Pool also, bevor
Sie ihn weiterleiten. Dasselbe gilt für `queue:retry`: Ein
fehlgeschlagener Job wird wieder auf die Queue eingereiht, auf der er
gestorben ist.

Das Pausieren wird vor der Umleitung ausgewertet, und zwar auf den
Namen, mit denen der Worker gestartet wurde.
`Queue::pause(&connection, "default")` stoppt weiterhin einen mit
`--queue=default` gestarteten Worker, auch während `default` an `high`
weitergeleitet wird. Die Umkehrung gilt ebenso: Das *Ziel* der
Weiterleitung zu pausieren - `Queue::pause(&connection, "high")` -
stoppt einen mit `--queue=default` gestarteten Worker nicht, denn
dieser Worker wird über seinen Quellnamen erreicht, nicht über den
umgeschriebenen. Das Event `WorkerQueuePaused`, das dieser Übergang
auslöst, trägt `queue: default`, den konfigurierten Namen, niemals
`high` - Laravel wertet es in derselben Reihenfolge aus und meldet es
genauso.

Die Aufrufe zur Inspektion werden bewusst nicht weitergeleitet:
`Queue::pending_jobs(Some("default"))` listet auf, was buchstäblich auf
`default` liegt, nicht, was auf `high` liegt - so sehen Sie den
Rückstau, der auf einer gerade weitergeleiteten Quell-Queue
zurückgeblieben ist. Laravel löst die Weiterleitung auch dort auf;
siehe die Abweichungsnotiz weiter unten.

Lesen Sie eine registrierte Weiterleitung mit
`Queue::forward_for("default")` zurück; sie liefert das Ziel in `queue`
und das Connection-Gate in `connection`.

### Warum Suprnova abweicht

Laravels `Queue::route(...)` nimmt eine Klassen-Zeichenkette; Suprnova
nimmt den Job als Typparameter, sodass ein umbenannter oder gelöschter
Job ein Compile-Fehler ist, statt einer Route, die stillschweigend
aufhört zu matchen.

Die größere Abweichung ist, was passiert, wenn ein Treiber nicht
filtern kann. `QueueDriver::pop_from` **lehnt** einen Queue-Filter ab,
den er nicht einhalten kann, statt darauf zurückzufallen, alles zu
leeren. Ein Worker, der angewiesen wird, nur `billing` zu leeren, aber
stillschweigend alle Queues leert, sieht wie ein funktionierendes
Deployment aus, bis der falsche Pool die falschen Jobs konsumiert -
daher wird die Fehlkonfiguration beim ersten Poll sichtbar gemacht. Die
Memory- und Database-Treiber filtern nativ; ein Treiber, der das nicht
tut - der Redis-Treiber ist einer davon, da eine einzelne
Stream-Consumer-Group keine Speicherung pro Queue hat -, liefert einen
Fehler, statt in die Irre zu führen.

`Queue::forward` portiert die Queue-zu-Queue-Hälfte von Laravels
`Queue::forward` vollständig, und nur diese Hälfte. Laravels drittes
Argument kann eine weitergeleitete Queue auf eine andere *Connection*
verschieben, weil sein Queue-Manager pro Connection-Namen einen Treiber
auflöst. Suprnova hat einen einzigen prozessglobalen Treiber, und ein
Connection-Name etikettiert lediglich die Lifecycle-Events;
`Queue::forward_on(from, to, connection)` behandelt die Connection
deshalb als **Gate** - sie entscheidet, ob die Umleitung des
Queue-Namens greift - und niemals als Ziel. Aus demselben Grund ist
`to` hier erforderlich, während es bei Laravel optional ist: Ein
weggelassenes `to` bedeutet bei Laravel „verschiebe nur die
Connection“, also genau die Dimension, die Suprnova nicht einhalten
kann, sodass ein `forward(from, None)` ein No-op wäre, das sich als
Konfigurationsänderung ausgibt.

Laravels Aufrufe zur Inspektion folgen einer Weiterleitung, weil
`pendingJobs($queue)` und seine Geschwister durch dasselbe `getQueue()`
auf Treiber-Ebene laufen wie Push und Pop. Suprnovas
`Queue::pending_jobs` / `delayed_jobs` / `reserved_jobs` melden
stattdessen die buchstäbliche Queue, die Sie benennen. Mit einem
einzigen prozessglobalen Treiber ist die buchstäbliche Sicht der
einzige Weg, die Envelopes zu sehen, die auf einer gerade
weitergeleiteten Queue zurückgeblieben sind - der Rückstau, den dieser
Abschnitt Ihnen zuerst zu leeren aufträgt. Fragen Sie die Ziel-Queue
namentlich ab, um zu sehen, wo neue Arbeit landet.

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

Worker geben Lifecycle-Events in Laravel-Form über die
[`Event`](events.md)-Facade aus. Listener bekommen die Identität des
Envelopes (`id`, `job_name`, `attempts`, `max_tries`, `connection`), nicht
die typisierte Job-Instanz - der Worker ist über JSON-Payloads
typgelöscht. Fehler reisen als `String`, da `FrameworkError` kein `Clone`
ableitet.

| Event | Feuert, wenn |
| --- | --- |
| `JobQueueing` | bevor das Envelope den Treiber erreicht |
| `JobQueued` | nachdem der Treiber angenommen hat |
| `UniqueJobSkipped` | `push_unique` innerhalb des `unique_for`-Fensters ein Duplikat unterdrückt hat |
| `JobDebounced` | der Worker ein Envelope verworfen hat, das ein neuerer entprellter Dispatch überholt hat |
| `JobProcessing` | der Worker gepoppt hat und gleich ausliefert |
| `JobProcessed` | der Handler `Ok` zurückgegeben hat |
| `JobAttempted` | bei jedem endgültigen Abschluss (Erfolg, Fehlschlag, Timeout) |
| `JobExceptionOccurred` | der Handler `Err` zurückgegeben hat und wiederholt wird |
| `JobReleasedAfterException` | die erneute Einreihung nach einem Fehler stattgefunden hat |
| `JobReleased` | eine von Middleware ausgelöste Freigabe (ohne Fehlschlag) |
| `JobFailed` | ins Dead-Letter verschoben wurde |
| `JobTimedOut` | das Timeout pro Versuch überschritten wurde |
| `Looping` | bei jeder Schleifeniteration (vor dem Pop) |
| `WorkerStarting` / `WorkerStopping` | einmal pro Worker-Lebensdauer |
| `WorkerInterrupted` | das Signal von `Queue::restart()` bemerkt wurde |
| `QueuePaused` | `Queue::pause` den eigenen Schalter einer Queue gesetzt hat |
| `QueueResumed` | `Queue::resume` den eigenen Schalter einer Queue gelöscht hat |
| `QueuesPaused` | `Queue::pause_all` den globalen Schalter gesetzt hat |
| `QueuesResumed` | `Queue::resume_all` den globalen Schalter gelöscht hat |
| `WorkerQueuePaused` | ein laufender Worker eine Queue erstmals als pausiert gesehen hat |
| `WorkerQueueResumed` | ein laufender Worker gesehen hat, dass eine pausierte Queue wieder beanspruchbar wurde |

Abonnieren Sie über die normale `Event::listen`-API. Events sind
bestmöglich - `Event::dispatch` ohne Listener ist ein wirkungsloses
`Ok(())`, Worker in Deployments ohne `Event::init()` zahlen also nichts.

`UniqueJobSkipped` ist das eine Event, das auf der *Push*-Seite statt auf
der Worker-Seite feuert, und das eine, das keinen Fehlschlag meldet. Es
trägt `job_name`, `unique_id` und `connection` - die Dedupe-Entscheidung
fällt, bevor ein Envelope existiert, es gibt also keine Envelope-ID zu
melden. Der Push gibt weiterhin `Ok(false)` zurück; das Event ist das,
was eine sonst unsichtbare Unterdrückung beobachtbar macht.

`QueuePaused` / `QueueResumed` / `QueuesPaused` / `QueuesResumed` feuern
auf dieselbe Weise - aus `Queue::pause` / `resume` / `pause_all` /
`resume_all` selbst, nicht aus der Worker-Schleife. Auch sie tragen keine
Envelope-Identität; den vollständigen Vertrag beschreibt „Queues
pausieren“ weiter unten.

`WorkerQueuePaused` / `WorkerQueueResumed` sind das Paar auf der
Worker-Seite, und sie sind diejenigen, die Ihnen sagen, *warum ein
bestimmter Worker still geworden ist*. Sie feuern einmal pro Übergang aus
der Worker-Schleife heraus, tragen die Connection, die der Worker leert,
und tragen den Queue-Namen - oder `None`, wenn ein ungefilterter Worker
bei einer globalen Pause untätig ist und keine Queue-Namen zu melden hat.

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
Queue::reserved_size().await?;   // gerade gepoppt, noch nicht bestätigt
Queue::clear().await?;           // verwirft jedes Envelope, liefert die Anzahl
Queue::driver_name()?;           // konfigurierter Treibername für Logs / Admin
```

Der `QueueDriver`-Trait deklariert Standardimplementierungen für `size` /
`pending_size` / `reserved_size` / `delayed_size` / `clear`;
`MemoryQueueDriver`, `DatabaseQueueDriver` und `RedisQueueDriver`
implementieren sie alle nativ.

### Warteschlangen inspizieren

Zähler sagen Ihnen, wie viel in der Queue liegt; manchmal müssen Sie die
tatsächlichen Envelopes sehen - ein Admin-Dashboard, eine Debugging-Sitzung,
die Frage „was genau steckt fest“. `Queue::pending_jobs` / `delayed_jobs` /
`reserved_jobs` liefern dieselbe Information, die die Größenzähler zählen,
als Auflistung von `InspectedJob`-DTOs:

```rust
use suprnova::queue::{InspectedJob, Queue};

let pending: Vec<InspectedJob> = Queue::pending_jobs(None).await?;
let billing_only: Vec<InspectedJob> = Queue::pending_jobs(Some("billing")).await?;
let delayed = Queue::delayed_jobs(None).await?;
let reserved = Queue::reserved_jobs(None).await?;

for job in &pending {
    println!(
        "{} attempts={} queue={:?} payload={}",
        job.name, job.attempts, job.queue, job.payload
    );
}
```

`InspectedJob` trägt `id`, `queue`, `name`, `attempts`, `payload` und
`created_at`. `id` und `created_at` sind `Option`: Die Auflistungen des
Datenbank-Treibers melden eine Zeile, deren `envelope_json` sich nicht
dekodieren ließ, weiterhin - als `id: None` und
`payload: {"unparseable": true}` -, statt sie fallen zu lassen und einen
vergiftenden Job vor demjenigen zu verbergen, der gerade hinsieht; die
Projektion von `Queue::fake()` zeichnet nie einen Dispatch-Zeitstempel
getrennt von `available_at` auf, `created_at` ist dort also immer `None`.

Beim Memory-Treiber liest `delayed_size()` direkt die Länge des Speichers
für verzögerte Jobs, während `delayed_jobs()` und `pending_jobs()` zuerst
jeden Eintrag befördern, dessen `available_at` bereits vorbei ist. In dem
schmalen Fenster zwischen dem Fälligwerden eines Jobs und dem nächsten
50-ms-Tick des Hintergrund-Reapers kann `delayed_size()` noch einen Job
mitzählen, den `delayed_jobs()` schon in `pending_jobs()` befördert hat -
die Auflistungen sind die aktuellere Sicht; eine Abweichung dort ist
erwartet und kein Fehler.

Eine Reservierung, deren Visibility-Timeout abgelaufen ist, taucht in
`reserved_jobs()` weiter auf, bis ein `pop` oder der Hintergrund-Reaper sie
zurückholt. Nur diese beiden holen zurück, und das Zurückholen ist es, was
einen Versuch verbraucht; ein Aufruf einer Auflistung ändert die Versuchszahl
eines Jobs also nie, so oft Sie ihn auch aufrufen.

#### Warum Suprnova abweicht

- **Eine Methode mit `Option<&str>` statt eines Paares pro Auflistung.**
  Laravel liefert `pendingJobs($queue)` neben einem separaten
  `allPendingJobs()`; hier fasst `queue: None` die beiden zu einem Aufruf
  zusammen. Dieselbe Form gilt für `delayedJobs`/`allDelayedJobs` und
  `reservedJobs`/`allReservedJobs`.
- **Der Trait-Standard ist ein ehrliches `Err`, keine leere Collection.**
  Laravels Beanstalkd- und SQS-Treiber geben aus diesen Methoden selbst für
  eine Queue `[]` zurück, in der offensichtlich Jobs liegen - eine Lüge durch
  Auslassung, die ein Treiber-Autor von außerhalb unbemerkt kopieren könnte.
  Ein Suprnova-Treiber, der die Inspektion nicht implementiert hat, sagt das;
  `sync` und `null` überschreiben mit `Ok(vec![])`, weil für sie „es gibt nie
  etwas aufzulisten“ die buchstäbliche Wahrheit ist und keine nicht
  implementierte Methode.
- **Redis' `reserved_jobs` gilt pro Consumer.** Der Treiber kennt nur die
  Reservierungen, die er selbst im Prozess vergeben hat; die laufenden
  Einträge eines anderen Consumers sind nur über Redis' eigenes `XPENDING`
  sichtbar, nicht über diesen Aufruf.
- **Redis' `pending_jobs` bedeutet „noch nie an einen Consumer dieser Gruppe
  ausgeliefert“.** Es scannt `XRANGE (<last-delivered-id> +` - alles jenseits
  des Auslieferungs-Cursors der Gruppe (`XINFO GROUPS`) - statt des ganzen
  Streams, denn `ack` führt auf einen Eintrag nur ein `XACK` aus (dieser
  Treiber führt nie `XDEL`/`XTRIM` auf dem Stream aus), sodass ein Scan, der
  lediglich die prozessinternen Reservierungen eines Consumers ausschlösse,
  jeden bestätigten Job für immer als ausstehend melden würde. Ein
  freigegebener oder genackter Job wird unter einer frischen ID oberhalb des
  Cursors neu veröffentlicht und taucht daher wieder auf, sobald seine
  Wiederholung fällig ist. Dasselbe Register „obere Schranke“ wie bei
  `pending_size`: Der Cursor wird einmal gelesen, ein nebenläufiges `pop`
  kann sich also zwischen diesem Lesen und dem Scan einen Eintrag greifen. In
  der Praxis greift sich die Vorauslese-Task eines laufenden Consumers einen
  frisch geschobenen Eintrag meist binnen Millisekunden nach dem Push, lange
  bevor eine Anwendung überhaupt `pop` aufruft - `pending_jobs` spiegelt also
  überwiegend Arbeit wider, die geschoben wurde, während kein Consumer für
  diesen Stream aktiv abfragt, und nicht „jedes Envelope, das niemand
  ausdrücklich gepoppt hat“.

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

## Queues pausieren

`php artisan queue:pause` / `queue:resume` übersetzen sich zu:

```rust
Queue::pause(&connection, "billing").await?;
Queue::resume(&connection, "billing").await?;
Queue::pause_all().await?;
Queue::resume_all().await?;
```

oder von der CLI aus:

```bash
./app queue:pause billing
./app queue:pause --all
./app queue:resume billing
./app queue:resume --all      # Alias: queue:continue
```

Ein pausierter Worker beendet, was er bereits gepoppt hat - Pausieren
unterbricht nie einen laufenden Job - und beansprucht danach keine neue
Arbeit mehr, bis fortgesetzt wird. `pause_all` / `resume_all` sind der
globale Schalter; eine benannte Queue zu pausieren (oder fortzusetzen)
betrifft nur diese Queue. **`resume_all` hebt eine Pause pro Queue nicht
auf** - eine einzeln pausierte Queue bleibt nach einem globalen Fortsetzen
pausiert, wie bei Laravel. Heben Sie sie ausdrücklich mit
`Queue::resume(&connection, "billing")` auf.

Ein pausierter Worker sagt es auch. `queue:work` gibt eine Zeile pro
Übergang aus:

```text
  2026-08-25 14:03:11 Queue billing PAUSED
  2026-08-25 14:07:44 Queue billing RESUMED
```

Ein ohne `--queue` gestarteter Worker hat keine Queue-Namen zu melden, eine
globale Pause gibt daher stattdessen `All queues PAUSED` aus. Beide Zeilen
stammen aus den Events `WorkerQueuePaused` / `WorkerQueueResumed`, Sie
können also selbst darauf lauschen und sie dorthin leiten, wo Ihr Alerting
liegt.

Beide Signale leben im `Cache`, direkt neben dem Neustart-Signal weiter
oben:

| Schlüssel | Bedeutung |
| --- | --- |
| `suprnova:queues:paused` | globaler Schalter, von `pause_all` gesetzt |
| `suprnova:queue:paused:{connection}:{queue}` | Schalter einer Queue, von `pause` gesetzt |

Prüfen Sie den Zustand mit
`Queue::is_paused(&connection, "billing").await?` (true, wenn einer der
beiden Schlüssel gesetzt ist) oder mit
`Queue::paused_queues(&connection, &queues).await?` (welche aus `queues`
derzeit pausiert sind).

### Pausieren pro Queue braucht ein benanntes `--queue`

Ein mit `--queue=billing,exports` gestarteter Worker beansprucht nur aus
diesen beiden Queues, `billing` zu pausieren verengt diese Liste also für
die Dauer der Pause auf `exports`. Ein ganz ohne `--queue` gestarteter
Worker leert jede Queue, die der Treiber hält, und dagegen lässt sich
nicht „pausiere nur `billing`“ sagen - `QueueDriver::pop_from` meldet nie,
welche Queue-Namen existieren, es gibt also nichts, wogegen sich ein
Pausenschlüssel pro Queue prüfen ließe. `pause_all` stoppt einen
ungefilterten Worker weiterhin vollständig; eine benannte Pause pro Queue
greift erst, wenn Sie die Queues dieses Workers ebenfalls benennen.

### Das Pause-Polling abschalten

Setzen Sie `QUEUE_PAUSABLE=false`, und jeder Worker in diesem Prozess
ignoriert Pausensignale vollständig, ohne zusätzliche Kosten für einen
Cache-Lesezugriff pro Schleifendurchlauf. `queue:pause` (nicht
`queue:resume`) verweigert außerdem den Dienst und endet mit einem Wert
ungleich null, sodass ein Betreiber, der das Pausieren abgeschaltet hat,
es sofort erfährt, statt eine Pause auszulösen, die still nichts tut.
Spiegelt Laravels `Worker::$pausable`.

### Warum Suprnova abweicht

Ein nicht erreichbarer Cache ist **Fail-open**: Ein Worker, der die
Pausenschlüssel nicht lesen kann, verhält sich wie „nicht
pausiert“ und leert weiter - derselbe Fail-open-Vertrag, den das
Neustart-Signal des Workers weiter oben bereits nutzt. Ein transienter
Cache-Ausfall sollte eine Worker-Flotte auf „Pause ignorieren“
herunterstufen, niemals auf „jeder Worker friert still ein“ - der
Pausenzustand ist ein ausdrückliches Opt-in-Signal, und seine eigene
Nichtverfügbarkeit sollte nicht zum versteckten Not-Aus werden.

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
Mutex; er erfasst pro Push `(payload, available_at, overrides)` und räumt
bei `Drop` auf. Das Feld `overrides` ist bei jedem Einstiegspunkt außer
`push_with`/`later_with` ein `EnvelopeOverrides::default()` - siehe
[Mocking](mocking.md#queue---queuetestinginstall_fake) für
`assert_pushed_on_queue`/`assert_pushed_on_connection` und
`pushed_with_overrides`, die Assertions darüber. Im Fake-Modus zeichnet
`push_unique` den Push immer als frisch auf - Deduplizierung ist belanglos,
wenn kein Treiber verdrahtet ist.

Ein entprellter Push verhält sich genauso: Der Fake schreibt nichts in den
Cache, es wird also kein Fenster scharf gestellt, und das aufgezeichnete
`available_at` trägt keine Debounce-Verzögerung. `assert_pushed_later`
sieht ihn als unverzögert. Was der Fake sehr wohl noch abfängt, ist ein
Job, der sowohl `debounce_for` als auch `unique_id` deklariert - dieses
Paar kann in keiner Umgebung Bestand haben, der Push gibt unter
`Queue::fake()` also genau denselben Fehler zurück wie in Produktion.

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
