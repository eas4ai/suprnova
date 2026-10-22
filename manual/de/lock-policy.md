# Lock-Richtlinie

Suprnova ist ein einziger, lang laufender Tokio-Prozess, keine Flotte
kurzlebiger PHP-Worker. Jede prozessglobale Registry, jedes Singleton
und jeder geteilte Cache, den Sie beim Boot binden, überlebt jede
Anfrage, die ihn berührt. Das ändert eine kleine, aber folgenreiche
Sache daran, wie Sie zu `std::sync::Mutex` und `std::sync::RwLock`
greifen: Ein Panic, während ein Guard gehalten wird, *vergiftet die
Sperre* für den Rest der Lebenszeit des Prozesses, und der nächste
Aufrufer muss entscheiden, was er damit tut. Dieses Kapitel ist die
projektweite Richtlinie für diese Entscheidung - zwei zugelassene
Muster, wann welches zu wählen ist, und warum Sie in Framework- oder
Anwendungscode niemals zu einem rohen `.lock().unwrap()` greifen
sollten.

## Warum dieses Kapitel existiert

In Laravel haben Sie nie über vergiftete Sperren nachgedacht, weil es
keine gab. PHP ist Shared-Nothing: Ein fataler Fehler reißt den
Prozess einer Anfrage nieder, die nächste Anfrage startet in einem
frischen Prozess, kein Zustand im Arbeitsspeicher überlebt, um zu
korrumpieren. Suprnova läuft genau umgekehrt. Der Prozess bootet
einmal, Registries werden befüllt, und sie bleiben für die gesamte
Lebenszeit der Binary lebendig. Ein Handler, der einen Panic auslöst,
während er einen Write-Guard auf einem prozessglobalen `RwLock` hält,
lässt
diese Sperre *vergiftet* zurück - jedes nachfolgende `.read()` und
`.write()` gibt für immer `Err(PoisonError)` zurück, sofern nicht
jemand sie ausdrücklich wiederherstellt.

Das Standard-Rust-Idiom - `.lock().unwrap()` - wandelt dieses `Err`
in einen Panic um. Der dann irgendwo weiter oben im Stack zu einer
weiteren vergifteten Sperre wird. Die dann das nächste Subsystem mit
sich reißt, das sie berührt. Eine schlechte Anfrage kaskadiert zu
einem halb-toten Prozess.

Die Richtlinie unten verhindert diese Kaskade.

> **Geltungsbereich.** Dies gilt für `std::sync::Mutex` und
> `std::sync::RwLock`, die einen Vergiftungszustand tragen. Die
> asynchronen Geschwister in `tokio::sync` (`Mutex`, `RwLock`,
> `Semaphore`) vergiften *nicht* - ein Panic, während ein
> `tokio::sync::Mutex`-Guard gehalten wird, lässt den Guard sauber
> fallen, und das nächste `.lock().await` gelingt. Wenn Ihr Hot Path
> asynchron ist und Sie den Guard nicht aus einem synchronen Kontext
> heraus erwerben müssen (eine `Drop`-Implementierung, ein
> Framework-Callback, ein CLI-Subcommand), bevorzugen Sie die
> Tokio-Varianten, und die Frage stellt sich nicht mehr.

## Die zwei zugelassenen Muster

Jede Stelle im Framework, die eine `std::sync`-Sperre hält, verwendet
eines von genau zwei Mustern. Wählen Sie in Ihrem eigenen Code auf
dieselbe Weise.

### Muster 1 - Vergiftung auf einen zurückgegebenen Fehler abbilden

Wenn der Aufrufer bereits `Result<_, E>` zurückgibt und ein weiteres
`?` seine Form nicht verändert, machen Sie die Vergiftung als Fehler
sichtbar und lassen Sie die Anfrage sauber scheitern. Das Framework
verwendet interne `pub(crate)`-Helfer (`lock::read`, `lock::write`,
`lock::lock`), die einen vergifteten Guard auf
`FrameworkError::internal("<context> lock poisoned")` abbilden und
dabei ein vom Aufrufer mitgegebenes Label einbetten, damit Logs
erkennen können, welches Subsystem vergiftet wurde, ohne dass jede
Aufrufstelle den Fehler selbst umschließen muss.

Das Muster, das diese Helfer kodieren, ist kurz genug, um es inline
in Ihrem Anwendungscode zu schreiben:

```rust
use std::collections::HashMap;
use std::sync::RwLock;
use suprnova::FrameworkError;

static FEATURE_FLAGS: RwLock<HashMap<String, bool>> = RwLock::new(HashMap::new());

pub fn enable(flag: &str) -> Result<(), FrameworkError> {
    let mut guard = FEATURE_FLAGS
        .write()
        .map_err(|_| FrameworkError::internal("feature flags lock poisoned"))?;
    guard.insert(flag.to_string(), true);
    Ok(())
}

pub fn is_enabled(flag: &str) -> Result<bool, FrameworkError> {
    let guard = FEATURE_FLAGS
        .read()
        .map_err(|_| FrameworkError::internal("feature flags lock poisoned"))?;
    Ok(guard.get(flag).copied().unwrap_or(false))
}
```

Innerhalb eines Handlers fällt `is_enabled(...)?` durch denselben
`FrameworkError → HttpResponse`-Pfad, den jeder andere
Framework-Fehler nutzt: Der Client erhält ein bereinigtes 500 mit
`{"message": "Internal Server Error"}`, das strukturierte Log erfasst
die beschriftete Vergiftungsmeldung, die Request-ID bleibt end-to-end
erhalten, und der Rest des Prozesses bedient weiterhin Traffic. Siehe
das Kapitel [Fehlerbehandlung](errors.md) für den vollständigen
Konvertierungspfad.

Verwenden Sie dieses Muster, wenn:

- Der Aufrufer bereits `Result` zurückgibt (die meisten fehlbaren
  Operationen tun das).
- Eine vergiftete Sperre einen echten, nicht behebbaren Ausfall des
  Subsystems darstellt - es gibt keine vernünftige „halbe Wahrheit“,
  auf die man zurückfallen könnte.
- Sie möchten, dass Operatoren die Vergiftung in den Logs *sehen*,
  sobald das Subsystem das nächste Mal berührt wird. Die
  beschriftete Meldung ist Ihre forensische Spur.

Der Notifications-Dispatcher, der Mail-Transport, die
Mailable-Registry, die DB-Event-Listener und die benannte
Connection-Registry des Frameworks verwenden alle dieses Muster. Ein
Panic in einem von ihnen taucht als 500 bei der nächsten Anfrage auf,
die die Registry trifft; alles andere läuft weiter.

### Muster 2 - An Ort und Stelle wiederherstellen mit `into_inner()`

Wenn die Signatur des Aufrufers *nicht* fehlbar ist (ein
`bool`-Lookup, eine Routing-Prüfung auf dem Hot Path, ein Pfad, auf
den sich der Request-Lifecycle verlässt) oder wenn der geteilte
Zustand strukturell sicher weiterverwendet werden kann, nachdem ein
Write nur teilweise durchgelaufen ist, stellen Sie den Guard wieder
her und machen Sie weiter:

```rust
use std::collections::HashMap;
use std::sync::RwLock;

static ALLOWED_INCLUDES: RwLock<HashMap<&'static str, Vec<&'static str>>> =
    RwLock::new(HashMap::new());

pub fn allows(dto: &str, field: &str) -> bool {
    ALLOWED_INCLUDES
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(dto)
        .map(|fields| fields.contains(&field))
        .unwrap_or(false)
}

pub fn register(dto: &'static str, fields: &'static [&'static str]) {
    let mut guard = ALLOWED_INCLUDES
        .write()
        .unwrap_or_else(|e| e.into_inner());
    guard.insert(dto, fields.to_vec());
}
```

`PoisonError::into_inner()` gibt den Guard trotz der Vergiftung
zurück. Nachfolgende Reads und Writes laufen normal weiter - die
Sperre bleibt für `is_poisoned()`-Abfragen vergiftet, aber der
Datenfluss ist wiederhergestellt.

Das Framework verwendet dieses Muster in `data::registry` (die
Include-Set-Allowlist, die bei jeder JSON:API-Response gelesen wird),
`auth::manager` (die Map der benannten Auth-Provider), `app::paths`
(der Cache der aufgelösten Pfade), den Test-Fakes für Mail und Events
sowie der Map der geladenen Env-Keys in der Konfiguration. Jede
dieser Stellen ist ein Ort, an dem entweder kein Aufrufer ein
`Result` zurückzugeben hat, oder der Zustand nur angehängt wird
(Append-Only) und strukturell sicher weiterverwendet werden kann.

Verwenden Sie dieses Muster, wenn:

- Die Signatur des Aufrufers schlicht ist (`bool`, `&str`, ein Klon
  eines gespeicherten Werts) und eine Umstellung auf `Result` jeden
  Aufrufer - manchmal jedes Framework-Subsystem - zum Bubbling
  zwingen würde.
- Der geteilte Zustand einen teilweisen Write tolerieren kann.
  Append-Only-Maps und Caches sind die typische Form: Der
  Schlimmstfall ist ein fehlender oder veralteter Eintrag, den der
  Aufrufer bereits behandelt (Default-Deny, Rückfall auf die primäre
  Quelle, Neuberechnung).
- Der Hot Path so oft läuft, dass ein Fehler bei jeder nachfolgenden
  Anfrage operativ schlechter wäre, als degradiert weiterzulaufen.

## Wie Sie zwischen den beiden wählen

Die Entscheidungsregel in einem Satz: **Wenn der Schlimmstfall bei
der Nutzung von Zustand nach der Vergiftung eine falsche Antwort mit
Konsequenzen ist, bilden Sie auf einen Fehler ab; wenn es ein
fehlender oder veralteter Eintrag ist, den der Aufrufer bereits
behandelt, stellen Sie an Ort und Stelle wieder her.**

Gehen Sie es durch:

1. **Ist die Signatur des Aufrufers `Result<_, E>`?** Wenn nein,
   müssen Sie an Ort und Stelle wiederherstellen - `Result` zu einem
   `bool` hinzuzufügen ist meist ein projektweites Refactoring und
   für einen Vergiftungs-Randfall nicht lohnend.
2. **Würde die Anwendung, wenn ein halb geschriebener Wert
   beobachtet würde, eine falsche Entscheidung mit realen
   Konsequenzen treffen?** Den falschen Kunden zu belasten, ein
   nicht autorisiertes Include zuzulassen, Zugriff auf den falschen
   Tenant zu gewähren - das ist „ja, auf einen Fehler abbilden“.
   `false` auf „ist dieser Name registriert?“ zurückzugeben und auf
   den primären Pool zurückzufallen - das ist „nein, an Ort und
   Stelle wiederherstellen“.
3. **Ist der Zustand append-only oder bei erneuter Registrierung von
   Natur aus idempotent?** Wenn ja, ist Wiederherstellen an Ort und
   Stelle sicher. Wenn ein Write ein Zustandsübergang ist, der vom
   vorherigen Wert abhängt, bevorzugen Sie das Abbilden auf einen
   Fehler, damit Sie eine Korruption nicht verstärken.

Im Zweifel bilden Sie auf einen Fehler ab. Eine Anfrage, die 500
zurückgibt, ist ein sichtbares Signal, das Sie beheben können;
stille falsche Antworten sind das nicht.

## Greifen Sie niemals zu `.lock().unwrap()`

Die verbotene Form:

```rust
// NIEMALS - ein einziger Panic irgendwo im Call-Graphen unterhalb
// dieser Zeile vergiftet die Sperre, und jeder nachfolgende Aufrufer
// verwandelt die Vergiftung in einen weiteren Panic.
let mut guard = SOMETHING.lock().unwrap();
```

`.expect("…")` ist dasselbe mit einer netteren Meldung. Beide wandeln
ein vergiftetes-Sperre-`Err` in einen Panic um, den das Netz des
Request-Lifecycle - `AssertUnwindSafe(...).catch_unwind()` - fängt
und in ein 500 umwandelt - dieses Netz ist eine *letzte
Verteidigungslinie*, keine Erlaubnis, die obige Entscheidung zu
überspringen. Öffentliche Framework-APIs und Anwendungscode müssen
eines der beiden zugelassenen Muster wählen.

Die zwei Ausnahmen, bei denen `.unwrap()` auf einer
`std::sync`-Sperre akzeptabel ist:

- **Test-Setup, das *absichtlich* bestätigen will, dass die
  Vergiftung erreicht wurde** - der eigene
  Vergiftungs-Induktions-Helfer von `framework/src/lock.rs`
  verwendet `.unwrap()` innerhalb des Threads, der den Panic auslöst,
  mit Absicht.
- **Der Fehlerpfad einer Vergiftungsoperation, die bereits
  fehlgeschlagen ist** - sobald Sie innerhalb des Threads von
  `poison_rw(...)` sind, *ist* der Panic genau der Punkt.

Wenn Sie in keiner dieser beiden Situationen sind, wählen Sie ein
Muster aus dem Abschnitt oben.

## Was, wenn meine Funktion `bool` zurückgibt?

Das ist die Situation, in der sich `ConnectionRegistry::has`
befindet. Es ist ein `bool`-Lookup auf dem Hot Path des
Read-Replica-Routings des Executors, inline aufgerufen als
`if ConnectionRegistry::has("read_replica").await { … }`. Es auf
`Result<bool, FrameworkError>` zu verbreitern würde jeden Aufrufer im
Executor zum `?`-Bubbling zwingen und einen Internal-Error-Codepfad
in Routing-Entscheidungen einschleusen, die nur ein Ja/Nein wollen.

Das Recover-in-Place-Muster erledigt das - geben Sie `false` zurück
und lassen Sie die Fallback-Logik des Aufrufers einsetzen (hier fällt
der Executor zurück auf den primären Pool, was ohnehin das sichere
Verhalten ist). Damit Operatoren die Bedingung trotzdem sehen,
emittieren Sie ein einmaliges `tracing::warn!`, sobald die
Vergiftung zum ersten Mal beobachtet wird:

```rust
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::RwLock;
use std::collections::HashMap;

static REGISTRY: RwLock<HashMap<String, ()>> = RwLock::new(HashMap::new());
static POISON_WARNED: AtomicBool = AtomicBool::new(false);

pub fn has(name: &str) -> bool {
    match REGISTRY.read() {
        Ok(g) => g.contains_key(name),
        Err(_) => {
            // Race-safe: Nur der erste Beobachter protokolliert.
            if !POISON_WARNED.swap(true, Ordering::SeqCst) {
                tracing::warn!(
                    target: "myapp::registry",
                    "registry lock poisoned - `has({name})` degrading to false",
                );
            }
            false
        }
    }
}
```

Das `swap`-basierte Gate ist wichtig: Die Vergiftung eines `RwLock`
ist klebrig, also würde ohne das Gate jeder nachfolgende Aufruf die
Warnung erneut auslösen und Ihre Logs überfluten. Mit dem Gate
erhalten Sie genau eine Warnung pro Prozess und Registry, und ein
entsprechender `Result`-zurückgebender Getter (`get`, `register`) auf
derselben Registry macht die Vergiftung sichtbar, sobald irgendetwas
*tatsächlich* darauf angewiesen ist, dass der Lookup gelingt. Das
gibt Operatoren beide Signale: eine frühe „etwas stimmt
nicht“-Warnung und ein hartes 500 in dem Moment, in dem eine Anfrage
wirklich von der Registry abhing.

## Was das Framework bereits absichert

Sie müssen diese Richtlinie auf keinen Zustand anwenden, den das
Framework selbst besitzt - sie ist dort schon in Kraft. Konkret:

- Die benannte Connection-Registry (`ConnectionRegistry::register`,
  `get`, `has`) bildet die Vergiftung bei den Writes und den
  `Result`-zurückgebenden Reads auf `FrameworkError::internal` ab;
  `has` degradiert mit dem Warn-once-Gate auf `false`.
- Der Notifications-Dispatcher und die Factory-Registry, die
  Mailable-Registry, der Mail-Transport, die Mail-Memory-Capture und
  die DB-Event-Listener geben bei Vergiftung alle
  `FrameworkError::internal` zurück.
- Die Include-Allowlist von `data::registry`, die Provider-Map von
  `auth::manager`, `app::paths`, der Cache der geladenen Env-Keys
  und die In-Memory-Test-Fakes stellen alle an Ort und Stelle wieder
  her.

Wo Sie diese Subsysteme über ihre öffentliche API berühren
(`Notification::send`, `Mail::send`, `Auth::user`, `DB::connection`,
den JSON:API-Response-Pfad), taucht eine vergiftete Framework-Sperre
als ein sauberes 500 auf - niemals als Panic an Ihrer Aufrufstelle.

## Warum Suprnova abweicht

Laravel hat keine Lock-Richtlinie, weil es keinen lang lebenden
geteilten Zustand hat. Jede PHP-Anfrage bekommt ihren eigenen
Prozess, ihren eigenen Speicher, ihre eigenen Kopien jedes
Singletons. Es gibt keine In-Memory-Registry, die vergiftet werden
könnte, und kein Konzept davon, dass „die nächste Anfrage“ Schäden
von der vorherigen erbt - die Runtime garantiert einen sauberen
Neustart.

Suprnova baut auf Tokio auf, was Ihnen genau den lang lebenden
geteilten Zustand gibt, den PHP ausschließt. Günstige WebSockets,
In-Memory-Caches, Connection-Pools, die Sie nicht ständig neu
aufbauen müssen - all das braucht prozessglobale Registries, die
jede einzelne Anfrage überleben. Diese Fähigkeit ist der ganze Sinn
des Wechsels zu Rust für diese Art von App (siehe die
[Einführung](introduction.md) für die vollständige Motivation des
Frameworks). Der Preis dafür ist, dass Sie jetzt darüber nachdenken
müssen, was passiert, wenn ein Thread nach einem Panic geteilten Zustand in
einem gesperrten Zustand zurücklässt, weil es *tatsächlich*
geteilten Zustand gibt, den man zurücklassen kann.

Die Zwei-Muster-Richtlinie ist die kleinste Antwort, die die
Fähigkeit behält und den Preis entfernt. Stellen Sie dort wieder her,
wo der Zustand sicher weiterverwendet werden kann; bilden Sie dort
auf einen Fehler ab, wo Ihnen ein sauberes 500 lieber ist als eine
falsche Antwort. Beide Optionen lassen den Rest des Prozesses weiter
Traffic bedienen. Keine der beiden lässt ein Unwrap zurück, das einen
Panic auslöst und darauf wartet, das Subsystem darüber mit sich zu reißen.

Das hat dieselbe Form wie die
[Fail-Open-vs-Fail-Closed-Entscheidung](rate-limiting.md), die das
Framework auf unerreichbare Cache- und Rate-Limit-Backends anwendet:
eine explizite Richtlinien-Entscheidung an der Aufrufstelle, kein
Standardverhalten. Async überall gibt Ihnen lang lebenden Zustand;
das Framework gibt Ihnen das Playbook, um ihn ehrlich zu halten.

## Nächste Schritte

- [Fehlerbehandlung](errors.md) - wie `FrameworkError::internal` zum
  bereinigten 500 wird, den der Client erhält, wobei die
  beschriftete Vergiftungsmeldung in Ihrem strukturierten Log
  erhalten bleibt.
- [Service Container](container.md) - wo die prozessglobalen
  Registries, die diese Richtlinie schützt, tatsächlich leben, und
  warum Task-Local-/Thread-Local-Scoping verhindert, dass Tests die
  Bindings der jeweils anderen erben.
- [Request-Lifecycle](lifecycle.md) - die Panic-Grenze
  (`execute_chain_safely`), die das *letzte* Unwrap abfängt und in
  ein 500 umwandelt, damit Sie genau verstehen, was das
  Sicherheitsnetz tut und warum es kein Freibrief ist, die
  Richtlinie oben zu überspringen.
- [Ratenbegrenzung](rate-limiting.md) - die parallele
  `BackendErrorPolicy`-Geschichte für Backends, die *unerreichbar*
  sein können statt vergiftet; dasselbe Prinzip der expliziten
  Entscheidung, aber ein anderer Fehlermodus.
- [Testen](testing.md) - wie `TestContainer::fake` und die
  Thread-Local-Container-Schicht verhindern, dass parallele Tests
  die Registries der jeweils anderen verschmutzen - das
  Test-Zeit-Pendant zur Geschichte der Vergiftungsbehandlung.
