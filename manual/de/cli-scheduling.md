# Befehlsplanung

CLI-Oberfläche für den Minuten-Scheduler. Die drei
`schedule:*`-Subkommandos delegieren alle an den
`Application::run()`-Dispatch Ihrer Anwendungs-Binary, sodass sie
dieselbe Konfiguration, Services, Observer und Listener sehen wie
ein Request-Handler. Das vollständige Scheduler-Modell - der
`Task`-Trait, die fluent Cron-API, `without_overlapping`,
`run_in_background` - lebt in [Task-Planung](scheduling.md); dieses
Kapitel ist die Operator-Referenz für die Befehle selbst.

## Wie die Befehle laufen

`suprnova schedule:run`, `suprnova schedule:work` und
`suprnova schedule:list` sind dünne Shells, die
`cargo run -- schedule:<subcommand>` gegen das Projekt im aktuellen
Verzeichnis aufrufen. Dieselben Subkommandos sind in der Produktion
auch direkt auf der Anwendungs-Binary erreichbar:

```bash
# In der Entwicklung (aus dem Projekt-Root, Source-Build):
suprnova schedule:run

# In der Produktion (Binary auf dem PATH):
/usr/local/bin/myapp schedule:run
```

Die Runtime-Treiber (Cache, Queue, RateLimit, Mail) und Ihre
`bootstrap_fn` werden gebootet, bevor irgendein Task läuft, sodass
ein geplanter Task Services aus dem Container genauso auflösen kann
wie ein Controller - siehe [Application Bootstrap](bootstrap.md).

Sie müssen den Scheduler in den Application-Builder verdrahten,
damit die Subkommandos überhaupt Tasks finden:

```rust
// cmd/main.rs (Backend-Starter) oder src/main.rs (API-Starter)
Application::new()
    .config(my_app::config::register)
    .bootstrap(my_app::bootstrap::bootstrap)
    .routes(my_app::routes::register)
    .schedule(my_app::schedule::register)   // <-- der Scheduler-Hook
    .migrations::<my_app::migrations::Migrator>()
    .run()
    .await
```

`suprnova make:task <Name>` verdrahtet das automatisch; wenn Sie die
Chain von Hand bauen, fügen Sie den `.schedule(...)`-Aufruf selbst
hinzu.

## schedule:run

Bewertet jeden registrierten Task einmal und führt die aus, deren
Cron-Ausdruck auf die aktuelle Minute passt. Dafür gedacht, jede
Minute vom System-Cron aufgerufen zu werden. Beendet sich mit
Non-Zero, falls ein Task fehlgeschlagen ist; beendet sich mit Zero
(mit `No tasks were due.`), falls in dieser Minute nichts fällig
war.

```bash
suprnova schedule:run
```

### Beispielausgabe

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✓ send:reminders
```

Wenn ein Task einen Fehler zurückgibt, wird seiner Zeile ein `✗`
vorangestellt und die Fehlermeldung angehängt:

```
Running due scheduled tasks...
  ✓ cleanup:logs
  ✗ backup:database: connection refused
```

Wenn in dieser Minute kein Task fällig ist:

```
Running due scheduled tasks...
No tasks were due.
```

### Crontab-Eintrag

Ein einziger Eintrag führt den Scheduler jede Minute aus. Die
Anwendungs-Binary bewertet alle fälligen Tasks selbst, sodass dies
die einzige Crontab-Zeile ist, die ein Produktions-Host braucht:

```cron
* * * * * cd /path/to/your/project && /usr/local/bin/myapp schedule:run >> /var/log/myapp/schedule.log 2>&1
```

Wenn Sie `schedule:run` vom System-Cron auf mehr als einem Host
ausführen (oder neben einem `schedule:work`-Daemon), brauchen mit
`.without_overlapping()` markierte Tasks ein konfiguriertes
Cache-Backend (`CACHE_DRIVER=redis` ist die produktionstaugliche
Wahl), um sich über Prozesse hinweg zu koordinieren - siehe
[Überlappung verhindern](scheduling.md#preventing-overlapping) für
die Lock-Semantik.

## schedule:work

Führt den Scheduler als langlebigen Daemon aus. Der erste Tick ist
auf die nächste Minutengrenze ausgerichtet, danach bewertet die
Schleife fällige Tasks einmal pro Minute, bis sie `SIGINT` (Ctrl-C)
oder `SIGTERM` empfängt. Beim Shutdown wird auf alle noch laufenden
`run_in_background`-Tasks gewartet, bevor beendet wird, damit sie
nicht mitten im Schreibvorgang abgerissen werden.

```bash
suprnova schedule:work
```

### Beispielausgabe

```
Starting scheduler daemon...
Press Ctrl+C to stop

==============================================
  suprnova Scheduler Daemon
==============================================
  3 task(s) registered. Press Ctrl+C to stop.
==============================================
```

Jeder Tick ist ruhig - nur Fehlschläge werden protokolliert. Beim
Shutdown:

```
suprnova: scheduler shutting down.
suprnova: waiting for 1 background task(s) to finish…

Scheduler daemon stopped.
```

### Anwendungsfälle

- **Entwicklung.** Kein Crontab erforderlich - starten Sie den
  Daemon in einem Terminal und beobachten Sie ihn ticken.
- **Docker.** Als Hauptprozess des Containers verwenden, wenn ein
  Image die Scheduler-Rolle spielen soll.
- **Systemd.** Als langlaufende Unit verwalten (siehe
  [systemd-Unit](#systemd-unit) unten).

### systemd-Unit

```ini
# /etc/systemd/system/myapp-scheduler.service
[Unit]
Description=MyApp Scheduler
After=network.target

[Service]
Type=simple
User=www-data
WorkingDirectory=/path/to/your/project
ExecStart=/usr/local/bin/myapp schedule:work
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable myapp-scheduler
sudo systemctl start myapp-scheduler
```

`Restart=always` bringt den Daemon nach einem Crash wieder hoch;
`RestartSec=5` debounced eine Crash-Loop. Weil die Panic-Grenze des
Frameworks panickende Tasks abfängt und in `FrameworkError`
umwandelt, sollte ein einzelner schlechter Task den Daemon nicht
crashen - `Restart=always` ist für den seltenen prozessweiten
Fehlschlag (OOM, Parent-Kill).

## schedule:list

Gibt jeden registrierten Task mit seinem Cron-Ausdruck und seiner
Beschreibung aus.

```bash
suprnova schedule:list
```

### Beispielausgabe

```
Registered scheduled tasks:
  cleanup:logs [0 3 * * *] - Removes logs older than 30 days
  send:reminders [0 9 * * *] - Sends daily reminder emails
  backup:database [0 0 * * 0] - Weekly database backup
  heartbeat [* * * * *]
```

Tasks mit einem an den Builder gehängten `.description(...)` zeigen
die Beschreibung nach dem Cron-Ausdruck; Tasks ohne Beschreibung
zeigen nur den Cron-Ausdruck.

Wenn nichts registriert ist (der `.schedule(...)`-Builder-Aufruf
fehlt, oder `schedule::register` ein No-op ist):

```
No scheduled tasks registered.
Define tasks in src/schedule.rs and wire it with `Application::schedule(schedule::register)`.
```

## Einen Task generieren

Das Framework liefert einen Generator aus, der den Task erstellt,
ihn ins Projekt verdrahtet und den Scheduler-Aufruf zu Ihrer
`main.rs` hinzufügt:

```bash
suprnova make:task CleanupLogs
```

Das:

1. Erstellt `src/tasks/cleanup_logs_task.rs` (einen
   funktionierenden `Task`-Stub, der seine eigene Dauer
   protokolliert)
2. Erstellt `src/tasks/mod.rs` (re-exportiert `CleanupLogsTask`),
   falls sie nicht schon existiert
3. Erstellt `src/schedule.rs` (mit einer
   `register(&mut Schedule)`-Funktion), falls sie nicht schon
   existiert
4. Deklariert `pub mod schedule;` und `pub mod tasks;` in
   `src/lib.rs`
5. Fügt `.schedule(<crate>::schedule::register)` zur
   `Application`-Chain in `cmd/main.rs` hinzu (oder `src/main.rs`
   beim API-Starter)

Schritte 2-5 sind idempotent, sodass ein erneutes Ausführen von
`make:task` eine von Hand entfernte Verdrahtung repariert. Siehe
[Code-Generatoren](cli-generators.md) für die breitere `make:*`-
Familie.

Registrieren Sie den Task nach der Generierung in `src/schedule.rs`:

```rust
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

Die fluent Builder-API (`.daily()`, `.cron(...)`,
`.without_overlapping()`, `.run_in_background()`, tagesspezifische
Modifikatoren) ist vollständig in [Task-Planung](scheduling.md)
behandelt.

## Exit-Codes

| Befehl | Exit Zero | Exit Non-Zero |
|---|---|---|
| `schedule:run` | jeder fällige Task gab `Ok(())` zurück, oder kein Task war fällig | mindestens ein Task gab `Err(_)` zurück oder panickte |
| `schedule:work` | sauberer Shutdown über `SIGINT` / `SIGTERM` (der Wrapper behandelt Exit-Code 130 als sauberes Ctrl-C) | Bootstrap-Fehlschlag, oder der Daemon-Prozess brach ab |
| `schedule:list` | Auflistung erfolgreich (einschließlich der „no tasks registered“-Meldung) | Anwendung konnte nicht booten |

Fehlschläge von Background-Tasks innerhalb von `schedule:work`
werden nach stderr protokolliert, beenden aber nicht den Daemon -
die `catch_unwind`-Grenze des `JoinSet` bringt sie als
`FrameworkError` an die Oberfläche, und die Tick-Schleife läuft
weiter.

### Warum Suprnova abweicht

Laravels `schedule:run` ist der einzige erstklassige
Einstiegspunkt; die Daemon-Form (`schedule:work`) ist ein Backport
für Hosts ohne Crontab. PHP hat keinen langlebigen Prozess, sodass
jede Minute eine frische Runtime ist, die das Framework, den
Container und jedes Service-Binding neu booten muss.

In Suprnova ist der Daemon erstklassig. `schedule:work` läuft
innerhalb derselben Tokio-Runtime, die auch HTTP bedient, also:

- **Background-Tasks komponieren mit der Tick-Schleife.** Ein
  `.run_in_background()`-Task wird in ein `JoinSet` gespawnt; die
  Schleife pollt abgeschlossene vor dem nächsten Tick und leert den
  Rest beim Shutdown. Laravel spawnt einen Kindprozess pro
  Background-Task.
- **Graceful Shutdown leert In-Flight-Arbeit.** Ctrl-C / SIGTERM
  lässt inline-Tasks ihren aktuellen Aufruf beenden und wartet auf
  jeden Background-Spawn vor dem Beenden. Laravel verlässt sich
  darauf, dass das OS das Cron-Kind tötet.
- **Boot-Kosten werden einmal bezahlt.** Der Container, die Treiber
  und Ihre `bootstrap_fn` booten beim Daemon-Start, nicht bei jedem
  Tick. `schedule:run` bezahlt die Boot-Kosten weiterhin pro Aufruf
  (es ist ein einmaliges Subkommando), aber der Daemon-Pfad ist, wo
  sich das Runtime-Modell auszahlt.

`schedule:run` funktioniert weiterhin (und ist die richtige Wahl,
wenn System-Cron bereits die Source of Truth des Operators ist).
Wählen Sie, was zu Ihrer Deployment-Form passt - beide teilen sich
dieselben Task-Definitionen.

## Nächste Schritte

- [Task-Planung](scheduling.md) - der `Task`-Trait, die fluent
  Cron-API, `without_overlapping`, `run_in_background` und Dedup
  innerhalb derselben Minute
- [Code-Generatoren](cli-generators.md) - die vollständige
  `make:*`-Familie, einschließlich `make:task`
- [Konsole](console.md) - mit `#[command]` annotierte einmalige
  Operator-Tasks (nicht nach Zeitplan)
- [Warteschlange](queues.md) - für Arbeit, die von einem Worker
  aufgenommen werden soll, statt auf einer Uhr zu ticken
- [Application Bootstrap](bootstrap.md) - wie `.schedule(...)` in
  den Builder einklinkt, und was Tasks aus dem Container auflösen
  können
