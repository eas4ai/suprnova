# Supervisoren

Ein Supervisor ist eine lang laufende Tokio-Task, die das Framework
beim Boot startet und beim Beenden automatisch neu startet.
Supervisoren sind für „Always-on“-Arbeit gedacht:
Hintergrund-Heartbeats, Metrik-Collectors, Connection-Warmer,
periodische Aufräum-Tasks oder jede asynchrone Schleife, die niemals
aufhören soll zu laufen. Sie unterscheiden sich von
[Queue-Workern](queues.md), die diskrete `Job`-Elemente aus einer
Warteschlange entnehmen. Ein Supervisor hat keine Job-Warteschlange -
er besitzt seine eigene Schleife und entscheidet selbst, wann er
schläft, wartet oder handelt.

Die `SupervisorRegistry` startet jeden registrierten Supervisor als
eine losgelöste Tokio-Task, beobachtet den `JoinHandle` jeder Task und
startet sie gemäß ihrer `RestartPolicy` neu, wenn sie endet - ob durch
Rückgabe von `Err`, Rückgabe von `Ok` oder durch einen Panic.
Neustarts sind durch einen exponentiellen Backoff getrennt, der bei
100ms beginnt und bei 60 Sekunden gekappt wird, damit ein abstürzender
Supervisor nicht in eine Schleife gerät, die die Logs überflutet.

## Schnellstart

Definieren Sie einen Supervisor, registrieren Sie ihn über
`inventory::submit!`, und rufen Sie `SupervisorRegistry::start_all()`
im Bootstrap auf.

**`src/supervisors/heartbeat.rs`:**

```rust
use async_trait::async_trait;
use std::time::Duration;
use suprnova::supervisor::{RestartPolicy, Supervisor};
use suprnova::{FrameworkError, SupervisorEntry};
use tokio_util::sync::CancellationToken;

pub struct LogHeartbeat;

#[async_trait]
impl Supervisor for LogHeartbeat {
    fn name(&self) -> &'static str { "heartbeat" }

    async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => return Ok(()),
                _ = tokio::time::sleep(Duration::from_secs(60)) => {
                    tracing::info!("supervisor heartbeat tick");
                }
            }
        }
    }

    fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }
}

// Das re-exportierte `suprnova::inventory` verwenden, damit eine per Scaffold
// erzeugte App `inventory` nicht als direkte Abhängigkeit hinzufügen muss.
suprnova::inventory::submit!(SupervisorEntry {
    factory: || Box::new(LogHeartbeat),
});
```

**`src/bootstrap.rs`:**

```rust
use suprnova::supervisor::SupervisorRegistry;

pub async fn register() {
    SupervisorRegistry::start_all().await;
}
```

Das ist das gesamte Setup. Der Supervisor `LogHeartbeat` startet beim
Boot, protokolliert alle 60 Sekunden - und weil
`RestartPolicy::Always` sowohl bei `Ok`- als auch bei `Err`-Beendigung
neu startet, wird er sofort neu gestartet, wenn die Schleife aus
irgendeinem Grund jemals endet.

## Restart-Richtlinien

Jeder Supervisor deklariert seine `RestartPolicy` über die
Trait-Methode. Der Standard ist `OnError`.

| Richtlinie | Startet neu, wenn... | Anwendungsfall |
|--------|-----------------|----------|
| `RestartPolicy::OnError` | `run()` `Err` zurückgibt oder in Panic gerät | Tasks, die bei Erfolg bis zum Abschluss laufen sollen (z. B. ein einmaliger Init-Job, als Supervisor verpackt). |
| `RestartPolicy::Always` | `run()` entweder `Ok` oder `Err` zurückgibt, oder in Panic gerät | Echte Daemons - Schleifen, die niemals zurückkehren sollten. Endet die Schleife aus irgendeinem Grund, ist das ein Bug, und ein Neustart ist die richtige Reaktion. |
| `RestartPolicy::Never` | (nie) | Einmalige Tasks, die einmal laufen und unabhängig vom Ausgang nicht neu gestartet werden sollen. |

```rust
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::OnError }   // Standard
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Always }    // Daemon-Schleife
fn restart_policy(&self) -> RestartPolicy { RestartPolicy::Never }     // einmalig
```

**Wann `Always` und wann `OnError` wählen.** Ein Supervisor mit einer
Endlosschleife (`loop { ... }`) sollte `Always` verwenden - kehrt die
Schleife jemals mit `Ok(())` zurück, ist etwas Unerwartetes passiert,
und ein Neustart ist die richtige Reaktion. Ein Supervisor, der
endliche Arbeit verrichtet und bei Erfolg `Ok` zurückgibt (z. B.
einmalig einen Cache auffrischen), sollte `OnError` verwenden, damit
ein sauberer Abschluss keinen Neustart auslöst.

**`Never` für einmalige Arbeit.** Bevorzugen Sie
[Queue-Worker](queues.md) oder [geplante Tasks](scheduling.md) für
Arbeit, die nach einem Zeitplan läuft. Verwenden Sie
`RestartPolicy::Never`, wenn das Supervisor-Muster praktisch für etwas
ist, das beim Start einmal laufen muss und nie wieder.

## Panic-Behandlung

Panics innerhalb von `run()` werden von der Registry abgefangen und
als Fehler behandelt - ein in Panic geratener Supervisor wird mit
Backoff neu gestartet, statt den Prozess abstürzen zu lassen. Die
Registry überwacht den `JoinHandle` jedes Supervisors und erkennt
Panics über den Standard-Tokio-Join-Mechanismus.

Aus Sicht der Restart-Richtlinie wird ein Panic unabhängig von der
Richtlinie immer als `Err`-Beendigung behandelt:

- `OnError` - startet nach einem Panic neu (Panic zählt als Fehler).
- `Always` - startet nach einem Panic neu (wie bei jeder anderen Beendigung).
- `Never` - startet nach einem Panic nicht neu (wie bei jeder anderen Beendigung).

Der Panic wird auf `error!`-Ebene mit dem Supervisor-Namen
protokolliert, bevor der Restart-Backoff beginnt.

## Backoff

Wenn ein Supervisor endet und seine Richtlinie einen Neustart
vorsieht, wartet die Registry, bevor sie den Ersatz startet:

| Aufeinanderfolgender Neustart | Verzögerung |
|---------|-------|
| 1. | 100ms |
| 2. | 200ms |
| 3. | 400ms |
| 4. | 800ms |
| ... | verdoppelt sich jedes Mal |
| Gekappt | 60s |

Der Backoff setzt sich nach einem gesunden Lauf zurück. Die
Verzögerung verdoppelt sich bei jedem *aufeinanderfolgenden* Neustart
bis zur 60-s-Grenze, aber ein Lauf, der mindestens 60 s (die Dauer der
Grenze) durchhält, gilt als gesund: Der nächste Neustart fällt auf den
100-ms-Boden zurück, statt den Backoff zu erben, der während einer
früheren Ausfallserie angestiegen ist. Ein Daemon, der stundenlang
sauber lief und dann kurz aussetzt, startet also prompt neu, nicht
nach einer 60-s-Wartezeit, die er lange zuvor angesammelt hat.

Der Reset ist liveness-basiert und bewusst konservativ: Nur ein Lauf,
der *die maximal mögliche Backoff-Dauer überlebt*, zählt als gesund.
Ein Lauf, der vor dieser Schwelle endet, trägt den aktuellen Backoff
weiter, sodass ein wirklich flatternder Supervisor - einer, dessen
Läufe die Schwelle nie erreichen - trotzdem bis zur 60-s-Grenze
hochfährt und dort bleibt. Der Reset verschleiert niemals einen
Supervisor, der sich in einer Absturzschleife befindet.

Die 60-Sekunden-Grenze verhindert, dass ein dauerhaft defekter
Supervisor unbegrenzt schläft oder externe Abhängigkeiten bei jedem
Versuch bombardiert. Kombinieren Sie das mit Protokollierung auf
`error!`-Ebene, um zu alarmieren, wenn ein Supervisor das
Hoch-Backoff-Band erreicht.

## Graceful Shutdown

Supervisoren erhalten ein `CancellationToken` als Parameter von
`run()`. Das Framework bricht dieses Token bei Ctrl-C / SIGTERM als
Teil der Shutdown-Sequenz von `Server::run` ab. Supervisoren, die
ihren Zustand flushen, in-flight Arbeit abschließen oder sonst sauber
beenden wollen, sollten auf `cancel.cancelled()` `tokio::select!`
verwenden:

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = tokio::time::sleep(Duration::from_secs(60)) => {
                tracing::info!("supervisor heartbeat tick");
            }
        }
    }
}
```

Das Framework leert das Supervisor-JoinSet nach dem Abbruch mit einem
5-Sekunden-Grace-Fenster. Supervisoren, die das Token innerhalb dieses
Fensters nicht beachten, werden über `JoinSet::abort_all` zwangsweise
abgebrochen. Das Leeren läuft nach dem Leeren der WebSocket-Handler
(sodass WS-Verbindungen zuerst aufräumen) und bevor die
Telemetrie-Puffer geflusht werden.

Supervisoren, die das Token vollständig ignorieren, laufen, bis das
5-Sekunden-Fenster abläuft, und werden dann zwangsweise abgebrochen.
Wenn Ihr Supervisor Ressourcen hält, die geflusht werden müssen
(offene Datei-Handles, in-flight HTTP-Anfragen, teilweise geschriebene
Datensätze), selecten Sie immer auf `cancel.cancelled()` und räumen
Sie auf, bevor Sie zurückkehren.

### Embedder und Integrationstests

`Server::run` ruft `SupervisorRegistry::shutdown(...)` für Sie auf.
Code, der `SupervisorRegistry::start_all()` außerhalb von
`Server::run` aufruft (Embedder, die das Framework von einer eigenen
Binary aus steuern, oder Integrationstests, die Supervisoren direkt
hochfahren), muss beim Teardown ebenfalls
`SupervisorRegistry::shutdown(timeout)` aufrufen, sonst leaken
Supervisor-Tasks über die Lebensdauer des Tests hinaus:

```rust
use std::time::Duration;
use suprnova::SupervisorRegistry;

// Test-Setup
SupervisorRegistry::start_all().await;

// ... den Supervisor ausüben ...

// Test-Teardown - bricht das geteilte Token ab, leert das JoinSet bis
// zu `timeout`, dann `abort_all` für Nachzügler.
SupervisorRegistry::shutdown(Duration::from_secs(1)).await;
```

`shutdown` ist ein No-Op, wenn `start_all` nie aufgerufen wurde, daher
ist es sicher, es beim Teardown bedingungslos aufzurufen.

## Beobachtbarkeit

Jeder Neustart auf dem Fehlerpfad emittiert einen Log-Eintrag auf
`error!`-Ebene mit strukturierten Feldern:

- `supervisor` - aus `Supervisor::name()`.
- `error` - die Fehlermeldung aus dem `Err`-Rückgabewert von `run()`, oder `"panic: <payload>"` für einen abgefangenen Panic, oder `"join error: <detail>"` für einen ungewöhnlichen Join-Fehlschlag.
- `backoff_ms` - die Backoff-Verzögerung in Millisekunden vor dem nächsten Spawn.

Panics werden über dasselbe Fehler-Log gemeldet - es gibt keine
separate „panicked“-Meldung:

```
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error=connection refused backoff_ms=400
ERROR suprnova::supervisor: supervisor errored; restarting after backoff supervisor=heartbeat error="panic: \"deliberate test panic\"" backoff_ms=800
```

Wenn `RestartPolicy::Always` `Ok(())` zurückgibt, emittiert das eine
`warn!` (nicht `error!`) mit denselben `supervisor`- /
`backoff_ms`-Feldern und der Meldung „supervisor returned Ok under
Always policy; restarting“ - nützlich, um Daemon-Schleifen
aufzuspüren, die sauber beendet wurden, obwohl sie es nicht sollten.

Supervisoren erhalten keinen automatischen Tracing-Span um `run()`
herum - die Registry umspannt den Lebenszyklus (Start, Neustart), aber
nicht das Innere der Task. Emittieren Sie Ihren eigenen `info_span!`
oder instrumentieren Sie Ihren Schleifenkörper, wenn Sie Span-Kontext
für Arbeit innerhalb des Supervisors möchten:

```rust
async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError> {
    loop {
        tokio::select! {
            _ = cancel.cancelled() => return Ok(()),
            _ = async {
                let span = tracing::info_span!("heartbeat.tick");
                let _guard = span.enter();
                do_work().await.ok();
                tokio::time::sleep(Duration::from_secs(60)).await;
            } => {}
        }
    }
}
```

### Warum Suprnova abweicht

Laravel hat keine direkte Entsprechung. PHPs
Prozess-pro-Anfrage-Modell macht dauerhaft laufende In-Process-Daemons
unmöglich - langlebige Arbeit muss außerhalb des Request-Lifecycle
leben, typischerweise als ein von `supervisord` verwalteter
Worker-Prozess, der eine Warteschlange konsumiert oder ein per Cron
geplantes Kommando. Laravels Queue-Worker (`php artisan queue:work`)
ist die nächste Entsprechung, aber es bleibt ein einmaliger
CLI-Prozess, den ein externer Supervisor neu startet.

Suprnova läuft auf Tokio innerhalb eines einzigen langlebigen
Prozesses. Dauerhaft laufende Hintergrund-Tasks passen natürlich als
überwachte Tokio-Tasks neben den HTTP-Server - keine zusätzliche
Prozessgrenze, kein externer Supervisor, kein separater IPC-Kanal für
Zustand. Der `Supervisor`-Trait ist das In-Process-Äquivalent zu
`supervisord`, begrenzt auf den eigenen Task-Baum des Frameworks, mit
denselben Garantien für Neustart-bei-Beendigung + Backoff.

`Queue`-Worker (die Laravel auch hat) gibt es weiterhin - siehe
[Warteschlange](queues.md) - für Arbeit in diskreten Jobs.
Supervisoren decken den Fall „tickt immer weiter“ ab, den Laravel
vollständig aus der Framework-Grenze herausdrängt.

## Außerhalb des v1-Umfangs

Die folgenden Punkte sind absichtlich zurückgestellt:

- **Supervisor-Bäume (Eltern/Kind).** Es gibt keine Hierarchie - alle Supervisoren sind Peers unter der einen `SupervisorRegistry`. Strukturierte Supervision (bei der ein Supervisor Kind-Supervisoren besitzt und neu startet) ist Orchestrator-Territorium.

- **Ressourcengrenzen (cgroup, Speicher, CPU).** Wenden Sie Ressourcenbeschränkungen über systemd-Unit-Dateien (`MemoryMax=`, `CPUQuota=`) oder Kubernetes-Resource-Requests/-Limits auf Pod-Ebene an. Das Framework erlegt einzelnen Supervisor-Tasks keine prozessinternen Ressourcengrenzen auf.

- **Multi-Maschinen-Supervision.** Supervisoren laufen innerhalb eines einzigen Prozesses auf einer einzigen Maschine. Supervisions-Entscheidungen über Maschinen hinweg zu verteilen ist Orchestrator-Territorium (Kubernetes, Nomad, systemd auf mehreren Hosts).

## Referenz

Die vier primären Typen - `Supervisor`, `RestartPolicy`,
`SupervisorEntry`, `SupervisorRegistry` - werden zusätzlich zum
längeren Pfad `suprnova::supervisor::*` an der Crate-Wurzel
re-exportiert (`suprnova::Supervisor` usw.). Die beiden freien
Zugriffsfunktionen bleiben unter `suprnova::supervisor::*`.

| Symbol | Zweck |
|--------|-------|
| `Supervisor` | Trait, den Sie auf Ihrer Supervisor-Struktur implementieren. Erforderliche Methoden: `name() -> &'static str`, `async fn run(&self, cancel: CancellationToken) -> Result<(), FrameworkError>`. Optional: `restart_policy() -> RestartPolicy` (Standard `OnError`). Das `cancel`-Token wird beim Prozess-Shutdown signalisiert; selecten Sie auf `cancel.cancelled()`, um sauber zu beenden, bevor das 5-Sekunden-Abbruchfenster abläuft. |
| `RestartPolicy` | Enum mit den Varianten `OnError`, `Always`, `Never`. Steuert, wann die Registry eine Ersatz-Task startet. |
| `SupervisorEntry` | Inventory-Eintrag. Deklarieren Sie `factory: fn() -> Box<dyn Supervisor>`. Reichen Sie pro Supervisor einen Eintrag über `suprnova::inventory::submit!(SupervisorEntry { factory: || Box::new(MySupervisor) })` ein. |
| `SupervisorRegistry::start_all()` | Async-Funktion. Durchläuft alle eingereichten `SupervisorEntry`-Werte, startet jeden Supervisor als losgelöste Tokio-Task in das Prozess-JoinSet und beginnt, auf Neustarts zu überwachen. Idempotent - die Prozess-Statics sind `OnceLock`s. Einmal aus Ihrem Bootstrap `register()` aufrufen. |
| `SupervisorRegistry::shutdown(timeout)` | Async-Funktion. Bricht das geteilte Cancellation-Token ab, sodass jeder Supervisor, der auf `cancel.cancelled()` wartet, endet, leert das JoinSet bis zu `timeout`, dann `abort_all` für Nachzügler. `Server::run` ruft dies als Teil seiner Shutdown-Sequenz auf; Embedder und Integrationstests, die `start_all` außerhalb von `Server::run` aufrufen, müssen dies selbst aufrufen, um kein Task-Leck zu verursachen. No-Op, wenn `start_all` nie aufgerufen wurde. |
| `suprnova::supervisor::supervisor_tasks()` / `supervisor_cancel_token()` | Zugriffsfunktionen, die `Option<&'static …>` auf das zugrunde liegende JoinSet und das Cancellation-Token zurückgeben. Werden von der Shutdown-Sequenz von `Server::run` verwendet; als `pub` exponiert, damit Embedder, die das Framework von einer eigenen Binary aus steuern, sich integrieren können. Anwendungscode sollte diese nicht brauchen. |

## Nächste Schritte

- [Warteschlange](queues.md) - die Entscheidung Supervisor vs. Queue-Worker und die Alternative mit diskreten Jobs
- [Task-Planung](scheduling.md) - für periodische Arbeit, die keine langlebige Schleife braucht
- [Workflows](workflows.md) - für zustandsbehaftete, lang laufende Arbeit, die eine dauerhafte Wiederaufnahme braucht
- [Broadcasting](broadcasting.md) - verwendet dieselbe Shutdown-Sequenz (Reihenfolge des Leerens)
- [Request-Lifecycle](lifecycle.md) - wo `Server::run` und das Shutdown-Leeren hineinpassen
