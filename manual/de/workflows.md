# Workflows

Workflows sind dauerhafte, lang laufende asynchrone Funktionen, deren
Zwischenzustand Abstürze, Neustarts und Panics überlebt. Greifen Sie
zu ihnen, wenn eine Arbeitseinheit mehrere Schritte umspannt - jeder
davon potenziell langsam, fehlbar oder mit Seiteneffekten -
und Sie es sich nicht leisten können, den Fortschritt auf halbem Weg
zu verlieren. Der Rumpf eines Workflows läuft einmal; die Ausgabe
jedes Schritts wird persistiert; eine Wiederholung setzt beim ersten
noch nicht abgeschlossenen Schritt wieder an. Kombinieren Sie sie mit
[`Queue`](queues.md), wenn die Arbeit ein einmaliger Job ist;
kombinieren Sie sie mit [`Bus`](bus.md), wenn die Arbeit synchron in
der Request-Task läuft.

## Schnellstart

Ein Workflow ist eine asynchrone Funktion, die
`Result<T, FrameworkError>` zurückgibt; ihr Rumpf ruft eine oder
mehrere `#[workflow_step]`-Funktionen auf; Sie reihen ihn über das
Makro `start_workflow!` ein, und ein Worker-Prozess arbeitet ihn ab.

```rust
use suprnova::{workflow, workflow_step, start_workflow, FrameworkError};

#[workflow_step]
async fn fetch_user(user_id: i64) -> Result<String, FrameworkError> {
    Ok(format!("user:{}", user_id))
}

#[workflow_step]
async fn send_welcome_email(user: String) -> Result<(), FrameworkError> {
    // … die Mail tatsächlich versenden
    Ok(())
}

#[workflow]
async fn welcome_flow(user_id: i64) -> Result<(), FrameworkError> {
    let user = fetch_user(user_id).await?;
    send_welcome_email(user).await?;
    Ok(())
}

// Aus einem Handler oder einem beliebigen asynchronen Kontext:
let handle = start_workflow!(welcome_flow, 123).await?;
```

Das Makro serialisiert die Argumente zu JSON, fügt eine Zeile in die
Tabelle `workflows` ein und gibt ein
[`WorkflowHandle`](#auf-ergebnisse-warten) zurück, das die eingereihte
Instanz identifiziert. Ein separater Worker-Prozess übernimmt die
Zeile, führt den Rumpf aus und persistiert dabei laufend die Ausgabe
jedes Schritts.

`#[workflow]` sammelt die Funktion unter ihrem vollqualifizierten Pfad
(`module_path::fn_name`) im Workflow-Inventory. Doppelte
Registrierungen unter demselben Namen brechen den Worker-Boot über
`registry::assert_no_duplicates` ab - stilles Shadowing wäre nicht
debugbar, daher scheitert das Framework sichtbar.

## Schema

Workflows persistieren in zwei Tabellen: `workflows` (eine Zeile pro
Instanz) und `workflow_steps` (eine Zeile pro Schritt-Aufruf, mit dem
Schlüssel `(workflow_id, step_index)`). Das Framework besitzt das
Schema; Sie entscheiden, wann Sie es anwenden.

Zwei Wege, die Migrationen zu verdrahten.

### Generierte Migrationsdateien

Die CLI scaffoldet Kopien der Framework-Migrationen in Ihre App:

```bash
suprnova workflow:install
suprnova migrate
```

`workflow:install` schreibt `m_create_workflows_table.rs` und
`m_create_workflow_steps_table.rs` unter `src/migrations/` und
registriert sie dann in Ihrem `Migrator`. Verwenden Sie das, wenn Sie
das Schema zusammen mit Ihren anderen App-Migrationen versioniert
haben möchten.

### Programmatische Registrierung

Alternativ registrieren Sie die framework-eigenen
Migrations-Strukturen direkt:

```rust
use sea_orm_migration::MigratorTrait;
use suprnova::workflow::migrations::{
    CreateWorkflowsTable, CreateWorkflowStepsTable,
};

pub struct Migrator;

impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
        vec![
            Box::new(CreateWorkflowsTable),
            Box::new(CreateWorkflowStepsTable),
        ]
    }
}
```

Beide Wege erzeugen identisches SQL. Dieselbe Konvention wird von
[`features::migrations`](feature-flags.md) und
[`payments::migrations`](payments.md) verwendet.

## Den Worker ausführen

In einer per Scaffold erzeugten App wird der Worker über den
Subcommand `workflow:work` der Binary gestartet:

```bash
suprnova workflow:work
```

Der Worker durchläuft denselben Bootstrap wie Ihr HTTP-Server, sodass
Observer, Listener und Container-Bindings, die in `bootstrap()`
registriert sind, für Workflow-Schritte sichtbar sind. Bei `SIGINT` /
`SIGTERM` hört der Worker auf, neue Claims zu übernehmen, und wartet
auf jeden in-flight Workflow, bevor er sich beendet - bei einem
sauberen Shutdown bleibt kein Workflow mitten im Schritt verwaist.

Der Übernahme-Pfad (`claim_next_workflow`) verwendet
`FOR UPDATE SKIP LOCKED` gegen die Tabelle `workflows`, daher
**erfordert** der Worker-Prozess Postgres. SQLite und MySQL
funktionieren für Tests und für den Einreihungs-/Persistenz-Pfad, aber
der Worker-Daemon beendet sich bei der ersten Übernahme mit einem
Fehler, wenn die Verbindung nicht Postgres ist.

## Konfiguration

Fünf Umgebungsvariablen stellen den Worker ein. Werte außerhalb des
gültigen Bereichs werden mit einem `tracing::warn!` auf sichere
Mindestwerte gekappt, damit ein Tippfehler in `.env` den Daemon nicht
lahmlegen kann.

| Variable | Standard | Hinweise |
|---|---|---|
| `WORKFLOW_POLL_INTERVAL_MS` | `1000` | Ruhepause zwischen leeren Übernahme-Runden |
| `WORKFLOW_CONCURRENCY` | `4` | Maximal laufende Workflows pro Worker (min. 1) |
| `WORKFLOW_LOCK_TIMEOUT_SECS` | `30` | Lease-Dauer, bevor ein anderer Worker zurückfordern darf |
| `WORKFLOW_MAX_ATTEMPTS` | `3` | Versuchs-Budget pro Workflow (min. 1) |
| `WORKFLOW_RETRY_BACKOFF_SECS` | `5` | Linearer Backoff: `attempts * value` (min. 0) |

Für programmatische Konfigurationen (im Code gebaut statt aus der
Umgebung geparst) rufen Sie `WorkflowConfig::validate()` auf, um bei
denselben Invarianten früh sichtbar zu scheitern, bevor Sie einen
`WorkflowWorker` konstruieren.

## Crash-Recovery

Drei Schutzschichten verhindern, dass Workflows bei
Worker-Fehlschlägen hängen bleiben.

**Panic-Grenze.** Der Workflow-Rumpf läuft innerhalb von
`AssertUnwindSafe(...).catch_unwind()`. Ein Panic in einem beliebigen
Schritt wird abgefangen, die Payload wird in die Fehlerspalte erfasst,
und die Zeile durchläuft dieselbe Retry-/Fail-Verbuchung wie ein
zurückgegebener `Err`. Ohne die Grenze würde ein Panic den
Abschlusspfad überspringen und die Zeile für immer bei
`status='running'` zurücklassen.

**Lease-Heartbeat.** Ein lang laufender Schritt, der
`WORKFLOW_LOCK_TIMEOUT_SECS` überlebt, könnte sich sonst die eigene
Lease unter den Füßen ablaufen lassen. Der Worker startet eine
Heartbeat-Task, die `locked_until` in der halben
Lock-Timeout-Intervalldauer auffrischt, bis der Rumpf abschließt. Der
Heartbeat bricht beim Drop ab, sodass ein zurückgegebenes `?` keine
Erneuerungs-Task leaken und die Lease für einen Workflow einfrieren
kann, den niemand ausführt.

**Zurückfordern abgelaufener Leases.** Wenn ein Worker stirbt, ohne je
seine Sperre freizugeben (Hard Kill, Host-Absturz, Kernel-OOM), bleibt
die Zeile bis zum Verstreichen von `locked_until` bei
`status='running'`. Die Übernahme-Abfrage nimmt solche Zeilen
ausdrücklich auf: Jeder `running`-Workflow, dessen Lease abgelaufen
ist, wird in der nächsten Runde von einem anderen Worker übernehmbar,
wobei `attempts` erhöht wird. Crash-Recovery ist automatisch - es gibt
nichts zu skripten und keinen Admin-Befehl, an den man sich erinnern
müsste.

## Zustellsemantik - At-least-once

Schritt-Rümpfe laufen mit **At-least-once**-Semantik. Ein Schritt kann
in zwei Situationen mehr als einmal ausgeführt werden:

1. **Zurückgegebenes `Err`** - der Workflow wird erneut eingereiht; bei der Wiederholung läuft der fehlgeschlagene Schritt erneut, und alle früheren Schritte werden aus dem Cache wiedergegeben.
2. **Absturz nach dem Seiteneffekt, bevor `mark_step_succeeded` committet** - die Lease läuft ab, ein anderer Worker fordert sie zurück, sieht an diesem Schritt-Index keine zwischengespeicherte Ausgabe und führt den Rumpf erneut aus.

Das Framework persistiert Schritt-**Ausgaben** dauerhaft, kann aber
den Seiteneffekt selbst nicht beobachten. Schritt-Rümpfe idempotent zu
machen liegt in Ihrer Verantwortung. Zwei Muster funktionieren für
fast jeden Fall.

**Bedingte Schreibvorgänge.** Verwenden Sie
`INSERT ... ON CONFLICT DO NOTHING`, Idempotenzschlüssel-Spalten oder
`seen_event_id`-Marker. Leiten Sie einen stabilen Schlüssel pro
Schritt aus bereits im Scope vorhandenen Daten ab: Die
Eingabeargumente des Workflows plus ein wörtliches Schritt-Tag
(`("wf-charge", customer_id)`) reichen aus, weil dieselben Argumente
über Wiederholungen hinweg auf dieselbe Workflow-Zeile abbilden.

**Externe Idempotenzschlüssel.** Die meisten Drittanbieter-APIs
(Stripe, SES, SQS) akzeptieren einen `Idempotency-Key`-Header.
Übergeben Sie einen Schlüssel, der aus der Eingabe des Workflows plus
einem schritt-lokalen Tag abgeleitet ist
(`format!("wf-charge-{}", customer_id)`), damit wiederholte Anfragen
beim Provider dedupliziert werden.

Gehen Sie **nicht** davon aus, dass ein Schritt, der `Ok`
zurückgegeben hat, nicht ein zweites Mal laufen kann - ein Absturz
kann diesen zweiten Lauf auf einem beliebigen nachfolgenden Worker
landen lassen, auch nach einem Neustart auf einem anderen Host. Siehe
das Kapitel [Idempotenz](idempotency.md) für `Idempotency::once`,
`Idempotency::commit_on_success` und `Idempotency::remember` - alles
gültige Wrapper um einen Schritt-Rumpf.

## Determinismus-Vertrag

Workflows müssen über Replays hinweg deterministisch sein. Jeder
Schritt wird durch `(step_name, step_index)` identifiziert, und das
Framework cacht seine serialisierte Eingabe zusammen mit der Ausgabe.
Wird ein Schritt am selben Index mit einer anderen serialisierten
Eingabe wiedergegeben, gibt das Framework einen Fehler zurück, statt
die Korruption zu verschleiern, indem es die zwischengespeicherte
Ausgabe der vorherigen Eingabe zurückgibt.

In der Praxis bedeutet das:

- Verzweigen Sie nicht anhand von `Utc::now()`, `rand::random()` oder anderen nicht-deterministischen Quellen außerhalb eines `#[workflow_step]`. Schritt-Rümpfe können sie frei aufrufen - ihr Ergebnis wird im Schritt-Ausgabe-Cache erfasst.
- Fügen Sie Schritte nicht bedingt ein. Trifft eine Wiederholung vor einem gegebenen Index auf eine andere Anzahl von Schritten, erhalten Sie einen Schritt-Namen-Fehler wegen Nichtübereinstimmung. Verlegen Sie Verzweigungslogik in einen Schritt hinein.
- Ändern Sie die Form der Schritt-Argumente zwischen Deployments nicht, ohne den Schritt umzubenennen. Ein Umbenennen ändert `step_name`, was das Caching für diesen Schritt von Grund auf neu startet.

## Auf Ergebnisse warten

`WorkflowHandle` erlaubt es dem Aufrufer, die Zeile zu pollen, auf
ihren Abschluss zu warten oder die serialisierte Ausgabe abzurufen.

```rust
use std::time::Duration;
use suprnova::{FrameworkError, WorkflowStatus};

let handle = start_workflow!(welcome_flow, 123).await?;

match handle.wait_with_timeout(Duration::from_secs(30)).await {
    Ok(WorkflowStatus::Succeeded) => { /* fertig */ }
    Ok(WorkflowStatus::Failed) => { /* persistierte Fehlerspalte */ }
    Ok(_) => unreachable!("wait_* only returns terminal status"),
    Err(FrameworkError::Internal { message }) if message.contains("Timed out") => {
        // Der Workflow läuft noch; weiter zum asynchronen UX-Pfad.
    }
    Err(other) => return Err(other),
}
```

`wait()` pollt unbegrenzt - verwenden Sie es nur in Tests oder
kurzlebigen Skripten, wo ein für immer blockierender Aufruf akzeptabel
ist. Für HTTP-Request-Pfade gewinnt `wait_with_timeout(Duration)`
immer gegen die innere Poll-Schleife, selbst wenn die zugrunde
liegende Status-Abfrage stockt. Ein Timeout-Fehler bricht den Workflow
**nicht** ab - der Worker läuft weiter, und `handle.status().await`
liefert später den Live-Zustand.

`wait_with_options(Some(poll), Some(deadline))` exponiert beide
Regler, wenn die Standardwerte nicht passen.

Für typisierte Ausgaben definieren Sie auf dem Workflow einen
Rückgabetyp `T: Serialize + DeserializeOwned` und rufen
`handle.output::<T>().await?` auf. Das rohe JSON ist über
`output_raw()` verfügbar.

## Schritt-Caching im Detail

Schritt-Caching wird durch Schrittname + Schritt-Index identifiziert.
Der erste Aufruf eines Schritts persistiert sein Eingabe-JSON, führt
den Rumpf aus und persistiert bei Erfolg das Ausgabe-JSON. Ein Replay
am selben Index:

- Gibt die zwischengespeicherte Ausgabe zurück, wenn der Schritt `succeeded` ist und die wiedergegebene Eingabe mit der zwischengespeicherten Eingabe übereinstimmt.
- Gibt einen Fehler zurück, wenn sich die Eingabe unterscheidet (die Determinismus-Absicherung).
- Führt den Rumpf erneut aus, wenn der Schritt `running` oder `failed` ist (keine zwischengespeicherte Ausgabe zum Zurückgeben).

Schritt-Indizes werden von einem `AtomicI32` pro Workflow-Kontext
zugewiesen, sodass die Reihenfolge durch die Aufrufe bestimmt wird,
die Ihr Workflow-Rumpf tätigt. Verzweigung, die bei einer Wiederholung
am selben Index einen anderen Schritt erzeugt, tritt als
Schritt-Namen-Fehler wegen Nichtübereinstimmung zutage, statt
nachgelagerte Schritte still zu korrumpieren.

Ausgaben und Eingaben werden als JSON-TEXT gespeichert, daher müssen
alle Rückgabetypen und Argumente von Schritten
`Serialize + DeserializeOwned` sein.

## Workflow-Kontext aus einem Helfer erkennen

`WorkflowContext::is_active()` gibt zurück, ob die aktuelle Task unter
einem Workflow läuft. Verwenden Sie es aus Helfern, die sich innerhalb
vs. außerhalb des Workers unterschiedlich verhalten müssen - zum
Beispiel ein Logger, der das Workflow-Tag nur anhängt, wenn eines
existiert:

```rust
use suprnova::workflow::WorkflowContext;

fn maybe_workflow_tagged(message: &str) -> String {
    if WorkflowContext::is_active() {
        format!("[workflow] {message}")
    } else {
        message.to_string()
    }
}
```

Außerhalb eines Workflows (direkt aus einem Test oder Handler
aufgerufen) läuft eine `#[workflow_step]`-Funktion trotzdem -
`WorkflowContext::current()` gibt einfach `None` zurück, der Rumpf
wird ohne Persistenz ausgeführt, und der Schritt umgeht den Cache
vollständig. Das ist beabsichtigt: Es macht Schritt-Funktionen einzeln
testbar, ohne einen Worker aufzusetzen.

### Warum Suprnova abweicht

Laravel hat kein erstklassiges Workflow-Primitiv - Jobs sind die
nächste Entsprechung, aber sie wiederholen, indem sie den gesamten
Job-Rumpf erneut ausführen, nicht indem sie beim letzten erfolgreichen
Schritt wieder ansetzen. Suprnova liefert Workflows als eigenständiges
Konstrukt, weil Tokio das Muster „eine Stunde lang in einer langsamen
asynchronen Funktion eingecheckt bleiben“ billig macht, und weil
Persistenz auf Schritt-Ebene die richtige Abstraktion für jede
mehrstufige externe Interaktion ist (einen Kunden provisionieren, eine
Saga über zwei Zahlungsanbieter hinweg ausführen, einen Bericht
erstellen, der mehrere Upstream-APIs involviert).

Das Design steht [DBOS](https://www.dbos.dev/) und Cadence/Temporal
näher als einer Queue: dauerhafter Zustand, deterministisches Replay,
explizite Schritt-Grenzen. Der Unterschied zu Temporal ist das
operative Gewicht - es gibt keinen separaten Workflow-Dienst
auszuführen; der Worker ist einfach `suprnova workflow:work` gegen
Ihre Anwendungsdatenbank.

## Hinweise

- Schritt-Rümpfe können jeden `Serialize + DeserializeOwned`-Typ zurückgeben. Der Unit-Typ `()` funktioniert für Schritte, die nur für ihren Seiteneffekt existieren.
- Eine außerhalb eines Workflow-Kontexts aufgerufene `#[workflow_step]`-Funktion läuft inline - kein Caching, kein Replay. So üben Tests Schritt-Rümpfe direkt aus.
- Schritt-Caching wird mit `(step_name, step_index)` identifiziert; benennen Sie einen Schritt um (oder ordnen Sie Aufrufe neu an), setzt sich das Caching für diesen Schritt beim nächsten Replay zurück.
- `start_workflow!` akzeptiert jedes Tupel serialisierbarer Argumente. Tupel bewahren die Argumentreihenfolge, sodass das Umbenennen positionaler Parameter sicher ist; das Ändern von Argumenttypen ist ein Schema-Bruch für jeden in-flight Workflow.
- Die [Beobachtbarkeit](observability.md)s-Schicht des Frameworks erfasst strukturierte Worker-Logs (`worker_id`, `workflow_id`, `attempts`, `max_attempts`) auf jedem Abschluss-Pfad, sodass Sie Retry-Budgets in Produktion prüfen können, ohne Ihre Schritte zu instrumentieren.

## Nächste Schritte

- [Warteschlange](queues.md) - einmalige Hintergrund-Jobs mit sync-/redis-/database-Treibern
- [Idempotenz](idempotency.md) - Wrapper für At-least-once-Zustellung
- [Bus](bus.md) - synchroner Command-Dispatch mit typisierten Ergebnissen
- [Supervisoren](supervisors.md) - langlebige Task-Supervision mit Panic-Catch-Auto-Restart
- [Fehlermodell](error-model.md) - `FrameworkError`, die Panic-Grenze und warum der Abschluss über `?` läuft
