# Task-Planung

Geplante Tasks sind asynchrone Funktionen, die das Framework anhand
eines Cron-Ausdrucks ausführt - jede Minute, stündlich, täglich,
wöchentlich oder nach einem beliebigen benutzerdefinierten
5-Feld-Cron. Tasks leben innerhalb Ihrer Anwendungs-Binary;
`schedule:run` evaluiert fällige Tasks einmal (rufen Sie es aus dem
System-Cron auf), und `schedule:work` führt denselben Evaluator als
langlebigen Daemon aus.

## Tasks generieren

Der schnellste Weg, einen neuen geplanten Task zu erstellen, ist die
suprnova-CLI:

```bash
suprnova make:task CleanupLogs
```

Dieser Befehl wird:
1. `src/tasks/cleanup_logs_task.rs` mit einem funktionierenden Task-Stub erstellen
2. `src/tasks/mod.rs` erstellen, falls sie nicht existiert, und den Task re-exportieren
3. `src/schedule.rs` zum Registrieren von Tasks erstellen, falls sie nicht existiert
4. `pub mod schedule;` und `pub mod tasks;` in `src/lib.rs` deklarieren
5. `.schedule(<crate>::schedule::register)` in Ihren Application-Builder in `cmd/main.rs` verdrahten (oder `src/main.rs` beim API-Starter)

Schritte 2-5 sind idempotent, sodass ein erneuter Aufruf von
`make:task` eine Verdrahtung repariert, die von Hand entfernt wurde.
Der Scheduler läuft innerhalb Ihrer Anwendungs-Binary - es gibt keine
separate Scheduler-Executable zu bauen oder zu deployen.

```bash Examples
# Erzeugt CleanupLogsTask in src/tasks/cleanup_logs_task.rs
suprnova make:task CleanupLogs

# Erzeugt SendRemindersTask in src/tasks/send_reminders_task.rs
suprnova make:task SendReminders

# Sie können auch das Suffix "Task" angeben (gleiches Ergebnis)
suprnova make:task BackupDatabaseTask
```

```rust Generated File
//! CleanupLogsTask scheduled task
//!
//! Created with `suprnova make:task cleanup_logs_task`.

use std::time::Instant;

use async_trait::async_trait;
use suprnova::{Task, TaskResult};

/// CleanupLogsTask - A scheduled task.
///
/// Register the task in `src/schedule.rs` with the fluent API; the skeleton
/// below times its own run and prints a structured log line on each
/// invocation so it works end-to-end the first time you wire it up.
pub struct CleanupLogsTask;

impl CleanupLogsTask {
    /// Create a new instance of this task.
    pub fn new() -> Self {
        Self
    }
}

impl Default for CleanupLogsTask {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        let started_at = Instant::now();
        println!("[CleanupLogsTask] task started");

        // Replace this with the real job. The skeleton ships as a
        // no-op success so the task can be scheduled and observed
        // before the implementation is filled in.

        println!(
            "[CleanupLogsTask] task finished in {} ms",
            started_at.elapsed().as_millis(),
        );
        Ok(())
    }
}
```

## Zeitpläne definieren

suprnova unterstützt zwei Ansätze zum Definieren geplanter Tasks:

### 1. Trait-basierte Tasks (empfohlen)

Für komplexe Tasks, die Abhängigkeiten oder wiederverwendbare Logik
brauchen, implementieren Sie den `Task`-Trait und konfigurieren den
Zeitplan während der Registrierung:

```rust
// src/tasks/cleanup_logs_task.rs
use async_trait::async_trait;
use chrono::{Duration, Utc};
use suprnova::{Task, TaskResult};
use crate::models::Log;

pub struct CleanupLogsTask;

impl CleanupLogsTask {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Task for CleanupLogsTask {
    async fn handle(&self) -> TaskResult {
        // Eloquent funktioniert genau wie innerhalb eines Controllers; Tasks
        // sehen dieselben Container-Bindings (`DB::connection()`,
        // `App::get::<T>()`) wie ein Request-Handler - siehe Application
        // Bootstrap unten.
        let cutoff = Utc::now() - Duration::days(30);
        Log::query()
            .filter_op("created_at", "<", cutoff)
            .delete_all()
            .await?;

        println!("Old logs cleaned up successfully");
        Ok(())
    }
}
```

Registrieren Sie das dann mit der fluent Scheduling-API in
`src/schedule.rs`:

```rust
// src/schedule.rs
use suprnova::Schedule;
use crate::tasks::CleanupLogsTask;

pub fn register(schedule: &mut Schedule) {
    schedule.add(
        schedule.task(CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );
}
```

### 2. Closure-basierte Tasks

Für schnelle Inline-Tasks ohne separate Dateien:

```rust
// src/schedule.rs
use suprnova::Schedule;

pub fn register(schedule: &mut Schedule) {
    // Einfacher Closure-Task
    schedule.add(
        schedule.call(|| async {
            println!("Ping! Running every minute");
            Ok(())
        })
        .every_minute()
        .name("heartbeat")
    );

    // Konfigurierter Closure-Task
    schedule.add(
        schedule.call(|| async {
            // Ihre Task-Logik
            Ok(())
        })
        .daily()
        .at("09:00")
        .name("morning-report")
        .description("Sends daily morning report")
    );
}
```

## Tasks registrieren

Registrieren Sie Ihre Tasks in `src/schedule.rs`:

```rust
// src/schedule.rs
use suprnova::Schedule;
use crate::tasks;

pub fn register(schedule: &mut Schedule) {
    // Trait-basierte Tasks mit fluent Schedule-Konfiguration
    schedule.add(
        schedule.task(tasks::CleanupLogsTask::new())
            .daily()
            .at("03:00")
            .name("cleanup:logs")
            .description("Removes logs older than 30 days")
    );

    schedule.add(
        schedule.task(tasks::SendRemindersTask::new())
            .daily()
            .at("09:00")
            .name("send:reminders")
            .description("Sends daily reminder emails")
    );

    schedule.add(
        schedule.task(tasks::BackupDatabaseTask::new())
            .weekly()
            .at("00:00")
            .name("backup:database")
            .description("Weekly database backup")
            .without_overlapping()
    );

    // Closure-basierte Tasks
    schedule.add(
        schedule.call(|| async {
            println!("Quick task!");
            Ok(())
        })
        .hourly()
        .name("quick-task")
    );
}
```

## Zeitplan-Häufigkeitsoptionen

suprnova bietet eine fluent API, um zu definieren, wann Tasks laufen
sollen:

### Häufige Intervalle

| Methode | Beschreibung |
|--------|-------------|
| `.every_minute()` | Jede Minute ausführen |
| `.every_two_minutes()` | Alle 2 Minuten ausführen |
| `.every_five_minutes()` | Alle 5 Minuten ausführen |
| `.every_ten_minutes()` | Alle 10 Minuten ausführen |
| `.every_fifteen_minutes()` | Alle 15 Minuten ausführen |
| `.every_thirty_minutes()` | Alle 30 Minuten ausführen |
| `.hourly()` | Jede Stunde zur Minute 0 ausführen |
| `.hourly_at(30)` | Jede Stunde zur Minute 30 ausführen |
| `.every_two_hours()` / `.every_three_hours()` / `.every_four_hours()` / `.every_six_hours()` | Zur vollen Stunde alle N Stunden ausführen |
| `.daily()` | Täglich um Mitternacht ausführen |
| `.daily_at("03:00")` | Täglich um 3:00 Uhr ausführen |
| `.twice_daily(1, 13)` | Zweimal täglich ausführen (z. B. 1:00 und 13:00 Uhr) |
| `.weekly()` | Wöchentlich sonntags um Mitternacht ausführen |
| `.monthly()` | Monatlich am 1. um Mitternacht ausführen |
| `.monthly_on(15)` | Monatlich an einem bestimmten Tag ausführen |
| `.quarterly()` | Am 1. Jan./Apr./Jul./Okt. um Mitternacht ausführen |
| `.yearly()` | Am 1. Januar um Mitternacht ausführen |

### Tagesspezifische Zeitpläne

```rust
use suprnova::DayOfWeek;

// An bestimmten Tagen ausführen
.weekly_on(DayOfWeek::Monday)
.weekly_on(DayOfWeek::Friday)

// Kurzform-Methoden für Tage
.sundays()
.mondays()
.tuesdays()
.wednesdays()
.thursdays()
.fridays()
.saturdays()

// Mehrere Tage
.days(&[DayOfWeek::Monday, DayOfWeek::Wednesday, DayOfWeek::Friday])

// Wochentage/Wochenenden
.weekdays()  // Montag-Freitag
.weekends()  // Samstag-Sonntag
```

### Zeit-Modifikatoren

Verketten Sie `.at()` mit jedem Zeitplan, um eine bestimmte Uhrzeit
festzulegen:

```rust
.daily().at("14:30")           // Täglich um 14:30 Uhr
.weekly().at("09:00")          // Wöchentlich um 9:00 Uhr
.mondays().at("08:00")         // Jeden Montag um 8:00 Uhr
.monthly().at("00:00")         // Am Ersten des Monats um Mitternacht
```

### Benutzerdefinierte Cron-Ausdrücke

Für volle Kontrolle verwenden Sie Cron-Syntax:

```rust
// Standard-Cron-Format: Minute Stunde Tag-des-Monats Monat Wochentag
.cron("0 */2 * * *")    // Alle 2 Stunden
.cron("30 4 * * 1-5")   // 4:30 Uhr an Wochentagen
.cron("0 0 1,15 * *")   // Am 1. und 15. jedes Monats
```

`.cron(...)` gerät **in Panic**, wenn der Ausdruck fehlerhaft ist
(falsche Feldanzahl, nicht parsbares step/range/list). Verwenden Sie
`.try_cron(expr)`, wenn der Ausdruck zur Laufzeit bereitgestellt wird
(Konfiguration, Nutzereingabe) und Sie den Parse-Fehler lieber
weiterreichen möchten:

```rust
schedule.add(
    schedule.task(MyTask::new())
        .try_cron(env_expr)?   // gibt bei einem fehlerhaften Ausdruck Err(String) zurück
        .name("from-config")
);
```

Dasselbe Panic-/`try_*`-Paar existiert auf jeder numerischen
Bereichs-Builder-Methode: `try_hourly_at`, `try_daily_at`,
`try_twice_daily`, `try_monthly_on`. Die unfehlbaren Varianten geraten
bei Zahlen außerhalb des gültigen Bereichs in Panic (z. B.
`daily_at("25:00")` oder `monthly_on(40)`); die fehlbaren Geschwister
geben `Err(String)` zurück.

## Task-Konfiguration

### Überlappung verhindern

Überspringen Sie einen Tick, wenn ein vorheriger Lauf desselben Tasks
noch in-flight ist:

```rust
schedule.add(
    schedule.task(LongRunningTask::new())
        .daily()
        .name("long-task")
        .without_overlapping()
);
```

**Wie die Sperre funktioniert.** Ist das Flag gesetzt, versucht
suprnova, über das konfigurierte [`Cache`](cache.md)-Backend einen
verteilten Mutex zu erwerben (`schedule:lock:<task-name>`). Ein
erfolgreicher Erwerb führt den Task aus und gibt die Sperre frei; ein
umkämpfter Erwerb wird als erfolgreiches Überspringen gemeldet -
`Ok(())`, wobei der Skip-Zähler des Tasks hochgezählt wird, sodass
Observability-Oberflächen es sehen können, ohne den Exit-Code von
`schedule:run` zu verfälschen.

**Cache ist für prozessübergreifenden Schutz erforderlich.** Wenn Sie
mehrere Prozesse betreiben, die denselben Task planen (z. B. mehrere
Maschinen, die `suprnova schedule:run` aus dem System-Cron aufrufen,
oder `schedule:work`-Daemons hinter einem Load-Balancer), ist das
Cache-Backend das, was sie koordiniert. **Ohne konfigurierten Cache
degradiert `without_overlapping()` stillschweigend zu einem
Pro-Prozess-`AtomicBool`** - zwei getrennte Prozesse sehen die Sperren
des jeweils anderen nicht. Das Framework emittiert beim ersten Mal,
wenn dieser Fallback feuert, einmalig eine `WARN`
(`suprnova::schedule`), damit Betreiber die schwächere Garantie
bemerken:

> `without_overlapping() falling back to in-process AtomicBool protection - Cache is not bootstrapped. Multi-process deployments will NOT see each other's locks. Configure Cache (CACHE_DRIVER=memory|redis) before relying on cross-process overlap protection.`

**Benutzerdefinierte Lock-TTL.** Die Lock-TTL beträgt standardmäßig 30
Minuten - lang genug, damit die meisten Tasks fertig werden, kurz
genug, dass ein abgestürzter Task, der die Sperre hält, den nächsten
Tick ohne Eingreifen des Betreibers freigibt. Überschreiben Sie das
pro Task mit `.without_overlapping_for(Duration)`. `Duration::ZERO`
ist über Cache-Backends hinweg undefiniert (Redis meldet einen Fehler,
In-Memory läuft sofort ab, Memcached behandelt es als „läuft nie ab“),
daher zwingt der Builder es mit einer einmaligen `WARN` auf den
30-Minuten-Standard, damit der Betreiber die Aufrufstelle korrigieren
kann.

```rust
use std::time::Duration;

schedule.add(
    schedule.task(SlowBackupTask::new())
        .daily()
        .name("backup:full")
        // Dieser Job läuft legitim länger als der 30-Minuten-Standard;
        // geben Sie der Sperre eine 2-Stunden-TTL, damit ein langsamer
        // Lauf nicht vom nächsten Tick verdrängt wird.
        .without_overlapping_for(Duration::from_secs(2 * 3600))
);
```

### Auf einem Server ausführen

Führen Sie einen Task pro fälligem Tick exakt einmal aus, unabhängig
davon, wie viele Replikate den Scheduler ausführen:

```rust
schedule.add(
    schedule.task(NightlyBillingTask::new())
        .daily()
        .at("02:00")
        .name("billing:nightly")
        .on_one_server()
);
```

**Was ohne dieses Feature schiefgeht.** Jedes Replikat, das
`schedule:work` ausführt, evaluiert den Zeitplan unabhängig, und
nichts hindert alle daran, zu entscheiden, dass derselbe Tick ihnen
gehört. Bei drei Replikaten wurden drei Ausführungen desselben Tasks
gemessen, jede Minute, ohne Abweichung. Für einen nächtlichen
Billing-Job bedeutet das, dass jeder Kunde dreimal belastet wird.

**Warum `without_overlapping()` das nicht abdeckt.** Die beiden sehen
ähnlich aus und lösen unterschiedliche Probleme:

| | Lock-Schlüssel | Gehalten für | Verhindert |
|---|---|---|---|
| `without_overlapping()` | Task | die Dauer des Tasks | dass ein langsamer Lauf seinen eigenen nächsten Tick überlappt |
| `on_one_server()` | Task **+ der Tick** | das Tick-Fenster | dass ein zweites Replikat denselben Tick ausführt |

Der entscheidende Unterschied ist, wann die Sperre freigegeben wird.
`without_overlapping()` gibt frei, sobald der Handler zurückkehrt -
bei einem schnellen Task, bevor ein zweites Replikat überhaupt
nachgesehen hat, sodass trotzdem alle N laufen. `on_one_server()` hält
seine Sperre absichtlich über den Handler hinaus und lässt sie per TTL
ablaufen, weil ein Replikat, das später im selben Tick eintrifft, sie
als belegt vorfinden muss.

Sie lassen sich kombinieren. Ein lang laufender Task, der auch
Single-Server sein muss, nimmt beide.

**Erfordert einen geteilten Cache.** Die Wahl ist eine
[`Cache`](cache.md)-Sperre, also bedeutet „ein Server“ „ein Prozess
unter denen, die sich ein Cache-Backend teilen“. Unter
`CACHE_DRIVER=memory` lebt die Sperre im Heap eines einzelnen
Prozesses, jedes Replikat gewinnt seine eigene Wahl, und die Garantie
ist stillschweigend abwesend.

In Produktion ist das ein Boot-Fehlschlag, keine Warnung:

> `refusing to boot in production: 1 task(s) request single-server execution (billing:nightly) but CACHE_DRIVER is memory or unset, so the election lock lives in this process's heap. Every replica would win its own election and run the task, which is what on_one_server() exists to prevent. Set CACHE_DRIVER=redis with REDIS_URL, or set SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true to acknowledge per-process locking - which is only accurate if you run exactly one scheduler.`

Setzen Sie `SCHEDULE_ALLOW_MEMORY_LOCK_IN_PRODUCTION=true`, wenn Ihr
Deployment wirklich nur einen einzigen Scheduler betreibt. Außerhalb
der Produktion bleibt der Memory-Treiber nutzbar, und das Framework
warnt stattdessen nur einmal.

**Benutzerdefinierte Lock-TTL.** Standardmäßig 60 Sekunden - ein
minutenausgerichteter Tick. Beide Extreme sind wichtig: zu kurz, und
ein Replikat, dessen Tick ein paar Sekunden zu spät landet, findet
keine Sperre mehr vor und führt den Task erneut aus; zu lang, und die
Sperre überlebt ihren Tick, sodass der nächste fällige Lauf sie belegt
vorfindet und vollständig übersprungen wird. Verwenden Sie
`.on_one_server_for(Duration)` für gröbere Zeitpläne.

```rust
use std::time::Duration;

schedule.add(
    schedule.task(HourlyRollupTask::new())
        .hourly()
        .name("rollup:hourly")
        // Ein stündlicher Task braucht die Sperre nur so lange, wie das
        // Fenster dauert, in dem Replikate diesen Tick noch als fällig
        // ansehen könnten.
        .on_one_server_for(Duration::from_secs(300))
);
```

**Ist der Cache unerreichbar**, wird der Tick übersprungen statt
ausgeführt. Der Moment, in dem die Koordination verloren geht, ist der
denkbar schlechteste, um jedes Replikat durchzulassen: ein
übersprungener Tick ist beim nächsten Tick wiederherstellbar, doppelte
Seiteneffekte sind das im Allgemeinen nicht.

### Warum Suprnova abweicht

Laravels `onOneServer()` ist dasselbe Opt-in, und Suprnova behält das
bei: Pro-Server-Tasks - Log-Rotation, das Aufwärmen eines lokalen
Caches - sind legitim und bleiben ausdrückbar.

Wo es abweicht, ist der Fehlermodus. Laravel führt `onOneServer()`
bereitwillig gegen einen Cache-Treiber aus, der nicht koordinieren
kann. Suprnova verweigert stattdessen den Boot in Produktion, aus
derselben Überlegung wie beim In-Memory-Rate-Limiter: Eine Kontrolle,
die stillschweigend viel weniger tut, als sie behauptet, ist schlimmer
als eine, die sichtbar abwesend ist.

### Im Hintergrund ausführen

Lösen Sie Tasks vom kritischen Pfad pro Tick, damit sie andere fällige
Tasks nicht am Start hindern:

```rust
schedule.add(
    schedule.task(BackgroundTask::new())
        .hourly()
        .name("background-task")
        .run_in_background()
);
```

**Panic-Isolation.** Hintergrund-Tasks laufen innerhalb eines
`tokio::task::JoinSet` mit `catch_unwind`, sodass ein in Panic
geratener Task als `FrameworkError` gegen den Namen des Tasks verbucht
auftritt, statt den Scheduler mit sich zu reißen. Der
`schedule:work`-Daemon leert das JoinSet beim Shutdown (Ctrl-C /
SIGTERM), sodass in-flight Hintergrund-Tasks vor dem Beenden
abschließen.

**Kombinieren mit `without_overlapping`.** Die beiden Flags lassen
sich kombinieren - ein Hintergrund-Task mit `without_overlapping()`
spawnt in das JoinSet und erwirbt die Überlappungssperre von innerhalb
des gespawnten Futures, sodass die oben beschriebene Sperrsemantik
weiterhin gilt.

### Dedup in derselben Minute

Die Cron-Auflösung ist minutengenau, und suprnova erzwingt das: Wird
derselbe Task innerhalb desselben Prozesses gebeten, zweimal innerhalb
derselben Wanduhr-Minute zu laufen, ist der zweite Aufruf ein
No-Op-Skip - `Ok(())`, wobei der Skip-Zähler des Tasks hochgezählt
wird. Das schließt eine Bug-Klasse, bei der eine Daemon-Schleife oder
ein eng getakteter `schedule:run`-Aufruf einen `.every_minute()`-Task
mehrfach in derselben Minute ausführen könnte.

Dieses In-Process-Gate ist **immer aktiv**, unabhängig von
`without_overlapping`. Es spannt sich **nicht** über Prozesse (jeder
Prozess hat seinen eigenen Pro-Task-Zustand). Wenn Sie
prozessübergreifende Koordination innerhalb derselben Minute brauchen,
schichten Sie `without_overlapping` + ein konfiguriertes Cache-Backend
darüber - zusammen decken sie beide Richtungen ab.

## Den Scheduler ausführen

suprnova stellt CLI-Befehle zum Ausführen geplanter Tasks bereit:

### Einmal ausführen

Alle fälligen Tasks einmal ausführen (typischerweise jede Minute von
Cron aufgerufen):

```bash
suprnova schedule:run
```

### Daemon-Modus

Kontinuierlich laufen und jede Minute auf fällige Tasks prüfen:

```bash
suprnova schedule:work
```

Das ist ideal für die Entwicklung oder wenn Sie einen Prozessmanager
wie systemd verwenden.

### Tasks auflisten

Alle registrierten geplanten Tasks anzeigen:

```bash
suprnova schedule:list
```

Ausgabe:
```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] - Removes logs older than 30 days
  send:reminders [0 9 * * *] - Sends daily reminder emails
  backup:database [0 0 * * 0] - Weekly database backup
```

## Produktions-Setup

### Mit Cron

Fügen Sie einen einzelnen Cron-Eintrag hinzu, um den Scheduler jede
Minute auszuführen:

```bash
* * * * * cd /path/to/your/project && suprnova schedule:run >> /dev/null 2>&1
```

**Prozessübergreifende Koordination.** Wenn Sie `schedule:run` aus dem
System-Cron auf mehr als einem Host ausführen (oder neben einem
`schedule:work`-Daemon), brauchen Tasks mit `.without_overlapping()`
ein konfiguriertes **Cache**-Backend (`CACHE_DRIVER=redis` für
Produktion empfohlen), um sich über Prozesse hinweg zu koordinieren.
Ohne das degradiert das Overlap-Flag zu Pro-Prozess-Schutz, und
derselbe Task kann in derselben Minute auf mehreren Hosts laufen.
Siehe [Überlappung verhindern](#überlappung-verhindern) oben für die
vollständige Sperrsemantik.

### Mit Systemd

Erstellen Sie einen systemd-Service für den Scheduler-Daemon:

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/path/to/suprnova schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

## Auf den App-Kontext zugreifen

Geplante Tasks haben vollen Zugriff auf den Anwendungskontext, genau
wie Controller:

```rust
use async_trait::async_trait;
use suprnova::{App, Task, TaskResult};
use crate::actions::SendEmailAction;
use crate::models::User;

pub struct SendRemindersTask;

#[async_trait]
impl Task for SendRemindersTask {
    async fn handle(&self) -> TaskResult {
        // Eloquent: `.get()` gibt eine `Collection<User>` zurück, die Sie
        // iterieren können.
        let users = User::query()
            .filter("reminder_enabled", true)
            .get()
            .await?;

        // Alles, was in `bootstrap.rs` gebunden ist, ist auch hier erreichbar.
        let send_email = App::get::<SendEmailAction>()
            .expect("SendEmailAction bound in bootstrap()");

        for user in users.iter() {
            send_email.execute(&user.email, "Daily Reminder").await?;
        }

        Ok(())
    }
}
```

## Dateiorganisation

Die empfohlene Dateistruktur für geplante Tasks:

```
src/
├── tasks/
│   ├── mod.rs              # Re-exportiert alle Tasks (automatisch aktualisiert von make:task)
│   ├── cleanup_logs_task.rs
│   ├── send_reminders_task.rs
│   └── backup_database_task.rs
├── schedule.rs             # Registriert Tasks (ausgeführt von den schedule:*-Befehlen)
├── bootstrap.rs
├── routes.rs
└── lib.rs                  # Deklariert `pub mod schedule;` + `pub mod tasks;`
cmd/
└── main.rs                 # Ruft `.schedule(<crate>::schedule::register)` auf
```

**src/tasks/mod.rs:**
```rust
pub mod cleanup_logs_task;
pub mod send_reminders_task;
pub mod backup_database_task;

pub use cleanup_logs_task::CleanupLogsTask;
pub use send_reminders_task::SendRemindersTask;
pub use backup_database_task::BackupDatabaseTask;
```

## Den Scheduler in Ihre Anwendung verdrahten

`make:task` verdrahtet `.schedule(<crate>::schedule::register)`
automatisch in Ihren `Application`-Builder. Bauen Sie die Chain von
Hand, ist der relevante Aufruf auf `Application`:

```rust
// cmd/main.rs (oder src/main.rs beim API-Starter)
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)        // <- diese Zeile
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await;
```

Ohne `.schedule(...)` melden alle `schedule:*`-Subcommands, dass keine
Tasks registriert sind. `schedule:work` und `schedule:run` führen
außerdem dieselben Runtime-Treiber und dieselbe `bootstrap_fn` aus wie
der HTTP-Server, sodass Observer, Listener und Container-Bindings, die
beim Boot registriert werden, für Ihre Task-Handler genauso sichtbar
sind wie für Controller (siehe [Application Bootstrap](bootstrap.md)).

### Warum Suprnova abweicht

Laravels Scheduler ist selbst ein einzelner Artisan-Befehl
(`schedule:run`), den PHP-Cron jede Minute auslöst. Die PHP-Runtime
fährt hoch, evaluiert fällige Tasks, führt sie in-process aus oder
ruft eine Shell auf, und fährt die Runtime dann wieder herunter. PHP
hat keine langlebigen Prozesse, daher wurde die Daemon-Form
(`schedule:work`) von Lumen zurückportiert und wird in Laravel selbst
als Workaround für Sites ohne Crontab-Zugriff ausgeliefert.

In Suprnova ist der Daemon erstklassig. `schedule:work` läuft
innerhalb einer Tokio-Runtime, die bereits langlebig ist, daher:

- **Hintergrund-Tasks (`run_in_background`) lassen sich mit der Tick-Schleife kombinieren.** Laravel spawnt einen Kindprozess pro Hintergrund-Task; wir spawnen in ein JoinSet und lassen Abschlüsse beim nächsten Tick oder beim Shutdown zutage treten.
- **Graceful Shutdown ist ein `tokio::select!`-Arm.** Ctrl-C / SIGTERM leert in-flight Hintergrund-Tasks vor dem Beenden; In-Process-Tasks schließen ihren aktuellen Aufruf ab.
- **Dedup innerhalb derselben Minute ist In-Process-Zustand.** Ein `last_run_minute`-Atomic pro Task garantiert, dass ein einzelner Prozess einen minutenausgerichteten Task nicht doppelt feuern kann, selbst wenn die Schleife schnell tickt. PHP kann das nicht - jeder Cron-Tick ist ein frischer Prozess -, weshalb Laravel Dateisystem-Sperren als einzige Verteidigungslinie verwendet.

Das `Cache::lock`-gestützte `without_overlapping` existiert weiterhin
für den Multi-Prozess-Fall (System-Cron auf mehreren Hosts, mehrere
`schedule:work`-Daemons hinter einem Load Balancer). Es ist derselbe
Mechanismus, nur auf einer Schicht, die der Scheduler nicht immer
braucht.

## Zusammenfassung

| Feature | Verwendung |
|---------|-------|
| Task erstellen | `suprnova make:task TaskName` |
| Trait-basiert | `Task`-Trait implementieren, Zeitplan während der Registrierung konfigurieren |
| Closure-basiert | `schedule.call(\|\| async { ... })` |
| Tasks registrieren | `schedule.add(schedule.task(...).daily().name("..."))` |
| In App verdrahten | `Application::new().schedule(schedule::register)` |
| Einmal ausführen | `suprnova schedule:run` |
| Daemon ausführen | `suprnova schedule:work` |
| Tasks auflisten | `suprnova schedule:list` |
| Überlappung verhindern | `.without_overlapping()` (Standard: 30-Min-Lock-TTL über Cache-Backend) |
| Benutzerdefinierte Overlap-TTL | `.without_overlapping_for(Duration)` |
| Hintergrund | `.run_in_background()` (Panic-isoliert über JoinSet) |
| Dedup in derselben Minute | Immer aktiv pro Prozess; übersprungene Läufe geben `Ok(())` zurück |
| Validierter Cron zur Laufzeit | `.try_cron(expr)` / `.try_daily_at(s)` / `.try_hourly_at(n)` |

## Nächste Schritte

- [Befehlsplanung](cli-scheduling.md) - CLI-Referenz für `schedule:run` / `schedule:work` / `schedule:list`
- [Warteschlange](queues.md) - für Arbeit, die von einem Worker abgeholt werden soll, statt nach einer Uhr zu ticken
- [Konsole](console.md) - `#[command]` für einmalige Operator-Tasks (nicht nach Zeitplan)
- [Cache](cache.md) - das Backend, das prozessübergreifendes `without_overlapping` antreibt
- [Application Bootstrap](bootstrap.md) - wie sich `.schedule(...)` in den Builder einklinkt, und was Tasks aus dem Container auflösen können
