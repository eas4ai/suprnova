# Kontext

`Context` ist Suprnovas Key/Value-Bag pro Anfrage. Dort legen Sie Daten
ab, die jeder nachgelagerte Aufrufer in derselben Anfrage sehen soll -
eine Request-ID, einen Tenant-Slug, eine Benutzerrolle, einen
Audit-Trail -, ohne den Wert durch jede Funktionssignatur zu fädeln. Er
ist das Suprnova-Äquivalent zu Laravels `Context`-Facade.

```rust
use suprnova::Context;

Context::add("tenant_id", "acme");
Context::push("breadcrumbs", "checkout/start");
Context::hidden_add("api_key", secret);

let tenant: Option<String> = Context::get("tenant_id");
let page: Option<String> = Context::query_param("page");
```

Greifen Sie dazu, wenn:

- eine Log-Zeile, ein eingereihter Job oder eine Broadcast-Nachricht
  Metadaten aus dem Request-Scope braucht (Tenant-ID, Korrelations-ID,
  Benutzerrolle)
- ein tief verschachtelter Helfer einen Wert braucht, den der Handler
  bereits hat, die Aufrufkette den Parameter aber nicht durch jede
  Schicht tragen soll
- Sie den Query-String der aktuellen Anfrage (`?page=3`, `?cursor=…`)
  aus Code lesen wollen, der kein Handler ist

`Context` ist **nicht** für Zustand über Anfragen hinweg gedacht. Er
hängt an der aktuellen Tokio-Task und verschwindet, wenn die Anfrage
endet. Für Dinge, die eine Anfrage überdauern, verwenden Sie den
[Service Container](container.md) oder den [Cache](cache.md).

## Die zwei Bags

Jeder aktive `Context`-Scope trägt zwei Key/Value-Maps und einen
zusätzlichen Slot:

| Bag | Lesen mit | Erscheint in `Context::all()` |
|---|---|---|
| **Sichtbar** | `Context::get` | Ja |
| **Versteckt** | `Context::hidden_get` | Nein |
| **Query** | `Context::query_param` | Nein (separater Snapshot der `?key=value`-Paare aus der URL) |

Die Trennung zwischen sichtbar und versteckt ist der ganze Sinn zweier
Bags: Log-Serialisierer, die `Context::all()` in strukturierte Ausgabe
kippen, lassen keine Daten durchsickern, die Sie absichtlich verstecken.
Legen Sie Audit-Metadaten in die sichtbare Bag; API-Schlüssel,
OAuth-Bearer-Token und personenbezogene Daten, die nicht in Logs
gehören, in die versteckte.

Die Query-Bag wird von der Request-Middleware des Frameworks automatisch
aus dem Query-String der URL befüllt (siehe
[Die Paginierung liest Query-Parameter](#die-paginierung-liest-query-parameter)
weiter unten). Normalerweise lesen Sie sie nur und schreiben nie hinein.

## Der aktive Scope

Bei jeder eingehenden HTTP-Anfrage installiert das Framework einen
`Context`-Scope. In einem Handler, einer Middleware, einem
Model-Observer, einem Event-Listener oder allem sonst, was von der
Request-Task aus erreichbar ist, ist der Scope aktiv, und Lese- wie
Schreibzugriffe über `Context::*` funktionieren ohne Umstände.

Außerhalb eines Scopes - Code beim frühen Boot, ein blankes
`tokio::spawn`, das den Kontext nicht erbt, ein Unit-Test, der keinen
installiert - ist jede Mutation ein **stiller No-Op**, und jeder
Lesezugriff liefert `None`. Der Vertrag lautet: niemals ein Panic, egal
von wo Sie aufrufen.

```rust
// In einem Handler - der Scope ist aktiv, alles funktioniert:
Context::add("user_id", 42i64);
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, Some(42));

// Außerhalb eines Scopes - stiller No-Op + None:
Context::add("user_id", 42i64);            // verworfen
let id: Option<i64> = Context::get("user_id");
assert_eq!(id, None);
```

Der No-Panic-Vertrag ist Absicht. Bibliothekscode, der `Context`
berührt (ein eigener Log-Subscriber, eine SDK-Erweiterung), soll nicht
wissen müssen, ob er innerhalb einer Anfrage oder beim Boot läuft - er
soll einfach `Context::get` aufrufen und `None` als "gerade nicht
verfügbar" behandeln.

### Observability für stille Operationen

Ein wirklich stiller No-Op würde Bugs verstecken (Middleware in der
falschen Reihenfolge, Kontext nicht in eine gespawnte Task propagiert,
versehentliches Lesen zur Boot-Zeit). Die mutierenden Operationen des
Frameworks bleiben panic-frei, geben aber auf dem Target
`suprnova::context` ein `tracing::trace!`-Event aus, sooft sie etwas
verwerfen:

```text
TRACE suprnova::context: Context mutation discarded: no active scope on this task op="add"
TRACE suprnova::context: Context mutation discarded: value failed to serialize op="push" key="bad"
TRACE suprnova::context: Context read returned None: value present but did not deserialize op="get" key="user_id" expected="String"
```

Drei Klassen von Event:

| Event | Wann es feuert |
|---|---|
| `mutation discarded: no active scope` | `add`, `push`, `hidden_add`, `forget` außerhalb jedes Scopes aufgerufen |
| `mutation discarded: value failed to serialize` | die `Serialize`-Impl des Werts von `add`/`push`/`hidden_add` hat einen Fehler geliefert |
| `read returned None: value present but did not deserialize` | `get`/`hidden_get` hat den Schlüssel gefunden, aber das gespeicherte JSON passt nicht zum angeforderten `T` |

Schlichte Abwesenheit - ein `get` auf einen Schlüssel, der nie gesetzt
wurde - bleibt still, damit Abfragen der Art "ist das gesetzt?" die Logs
nicht fluten. Schalten Sie `RUST_LOG=suprnova::context=trace` ein, wenn
Sie einen Bug bei der Propagierung vermuten; der stille No-Op-Pfad wird
sichtbar, ohne dass sich das Verhalten von Produktionscode ändert.

## Werte hinzufügen

### `Context::add` - an einem Schlüssel ersetzen

```rust
use suprnova::Context;

Context::add("user_id", 42i64);
Context::add("tenant", "acme");
Context::add("plan", PlanTier::Pro);     // beliebiger Serialize-Wert
```

Der Schlüssel ist `Into<String>`; der Wert ist ein beliebiger
`Serialize`-Typ. Der Wert wird beim Schreiben einmal in ein
`serde_json::Value` konvertiert und so gespeichert. Ein weiteres `add`
auf denselben Schlüssel ersetzt.

### `Context::push` - an einen Stack anhängen

```rust
Context::push("trail", "home");
Context::push("trail", "settings");
Context::push("trail", "billing");

let trail: Vec<String> = Context::get("trail").unwrap();
assert_eq!(trail, vec!["home", "settings", "billing"]);
```

`push` initialisiert beim ersten Aufruf ein leeres Array und hängt bei
jedem weiteren an. Liegt am Schlüssel bereits ein Skalar, wird er in ein
`[scalar, new_value]`-Array umgewandelt - `push` ist nachsichtig
gegenüber früheren `add`s auf denselben Schlüssel.

### `Context::hidden_add` - in die versteckte Bag schreiben

```rust
Context::hidden_add("api_key", os_env_secret);
Context::hidden_add("oauth_bearer", token);

// Ein Dump der sichtbaren Bag (etwa ein JSON-Log-Emitter) sieht sie nicht:
let all = Context::all();
assert!(!all.contains_key("api_key"));

// Absichtlich lesen können Sie sie trotzdem:
let key: Option<String> = Context::hidden_get("api_key");
```

Die versteckte Bag hat einen von der sichtbaren unabhängigen
Schlüsselraum - ein `hidden_add("user_id", 99)` und ein
`add("user_id", "alice")` existieren kollisionsfrei nebeneinander.
`Context::forget(key)` entfernt mit einem Aufruf aus beiden Bags.

## Werte lesen

### `Context::get` - typisiertes Lesen aus der sichtbaren Bag

```rust
use suprnova::Context;

let user_id: Option<i64>       = Context::get("user_id");
let tenant:  Option<String>    = Context::get("tenant");
let trail:   Option<Vec<String>> = Context::get("trail");
```

`get` ist generisch über `T: DeserializeOwned`. Der gespeicherte
JSON-Wert wird bei jedem Lesezugriff deserialisiert. Liefert `None`,
wenn:

- Der Schlüssel nicht gesetzt ist
- Auf der aktuellen Task kein Scope aktiv ist
- Der gespeicherte Wert nicht zu `T` deserialisiert (Sie haben etwa ein
  `i64` abgelegt und nach einem `String` gefragt)

Der letzte Fall gibt ein `tracing::trace!` aus, damit der Bug mit dem
falschen Typ beobachtbar ist - wenn `Context::get` wie "der Wert ist
nicht gesetzt" aussieht, in Wahrheit aber "der Wert hat die falsche
Form" gilt, ist das die Art von Bug, die ohne eine Log-Zeile, die darauf
zeigt, eine Stunde kostet.

### `Context::hidden_get` - typisiertes Lesen aus der versteckten Bag

Gleiche Form wie `get`, liest aber die versteckte Bag. Gleiches
Tracing-Verhalten beim falschen Typ.

### `Context::has` - Existenzprüfung auf der sichtbaren Bag

```rust
if Context::has("user_id") {
    // …
}
```

`has` prüft nur die sichtbare Bag (verwenden Sie
`hidden_get(...).is_some()`, wenn Sie die versteckte abfragen müssen).

### `Context::all` - Snapshot der sichtbaren Bag

```rust
let snapshot: HashMap<String, serde_json::Value> = Context::all();
```

Liefert außerhalb eines Scopes eine leere `HashMap`. Genau das sollte
ein JSON-Log-Emitter aufrufen, um Felder aus dem Request-Scope in jede
Log-Zeile einzuspeisen - und genau darum gibt es die versteckte Bag
separat.

### `Context::forget` - einen Schlüssel aus beiden Bags entfernen

```rust
Context::forget("trail");          // entfernt aus sichtbar UND versteckt
```

Das Entfernen aus beiden Bags ist Absicht. Wenn Sie zusammengehörige
Daten in beiden Bags abgelegt haben (etwa `user_id` sichtbar,
`user_email` versteckt), räumt ein einziges `forget` beide auf.

## Query-Parameter lesen

`Context::query_param` liest aus den `?key=value`-Paaren der URL, die
beim Eintritt der Anfrage erfasst wurden. Die Request-Middleware parst
den Query-String einmal in die Query-Bag des Scopes, danach kann jeder
nachgelagerte Aufrufer einzelne Parameter über den Namen lesen, ohne
erneut zu parsen:

```rust
use suprnova::Context;

let page: Option<String>   = Context::query_param("page");
let cursor: Option<String> = Context::query_param("cursor");
let sort: Option<String>   = Context::query_param("sort");
```

Liefert `None`, wenn der Parameter fehlt oder kein Scope aktiv ist. Bei
doppelten Schlüsseln gilt Laravels Semantik "der letzte gewinnt" -
derselbe Wert, den Sie aus der geparsten Query-Map der Anfrage bekämen.

### Die Paginierung liest Query-Parameter

Deshalb gibt es die Query-Bag. Die Paginatoren von Eloquent lesen
`?page=` und `?cursor=` direkt über `Context::query_param`, sodass ein
Handler, der einen Paginator zurückgibt, die Seitenzahl nicht von Hand
durchreichen muss:

```rust
use suprnova::{json_response, Request, Response};
use crate::models::Post;

pub async fn index(_req: Request) -> Response {
    // Liest ?page=N aus der URL der Anfrage über Context::query_param
    // - kein req.query()-Boilerplate, kein Durchfädeln von Parametern.
    let posts = Post::query()
        .order_by_desc("created_at")
        .paginate(15)
        .await?;

    json_response!(posts)
}
```

Drei Einstiegspunkte des Paginators nutzen das:

- `Builder::paginate(per_page)` - liest `?page=`
- `Builder::simple_paginate(per_page)` - liest `?page=`
- `Builder::cursor_paginate(per_page)` - liest `?cursor=`

Siehe [Paginierung](pagination.md) für die vollständige Oberfläche.

## In gespawnte Tasks propagieren

`tokio::spawn` startet die Kind-Task mit einer frischen
Task-Local-Umgebung - der `Context`-Scope der übergeordneten Task
fließt **nicht** hinein. Ein blankes `tokio::spawn` innerhalb einer
Anfrage sieht einen leeren `Context`, und jeder Lesezugriff liefert
`None`.

Um den Scope in einen Spawn mitzunehmen, machen Sie mit
`Context::current()` einen Snapshot davon und betreten ihn im Kind
erneut mit `Context::scope`:

```rust
use suprnova::context::Context;

// In einem Request-Handler:
if let Some(store) = Context::current() {
    tokio::spawn(Context::scope(store, async move {
        // Jetzt sehen `Context::get`, `Context::query_param` usw. die
        // Bag der übergeordneten Anfrage.
        let request_id: Option<String> = Context::get("_request_id");
        do_background_work(request_id).await;
    }));
}
```

Der von `Context::current()` gelieferte Store teilt sich die
darunterliegenden Maps der übergeordneten Task über `Arc` -
Schreibzugriffe aus dem Kind sind für die übergeordnete Task sichtbar,
solange das Kind den Klon hält. Genau das wollen Spawns für Audit und
Logging: Das Kind kann zusätzliche Schlüssel stempeln
(`Context::add("audit.completed", true)`), und die abschließende
Log-Zeile der übergeordneten Task sieht sie.

Wenn Sie einen isolierten Snapshot brauchen (die Schreibzugriffe des
Kindes sollen nicht zurücksickern), bauen Sie einen frischen
`ContextStore` und kopieren nur die Schlüssel hinein, die Sie brauchen.

### Warum blankes `spawn` nicht propagiert

Die Task-Locals von Tokio (`tokio::task_local!`) sind bewusst auf die
Task begrenzt. Automatisches Erben über Spawns hinweg würde bedeuten:

- Lang laufende Hintergrund-Tasks würden die Kontext-Maps der
  übergeordneten Task für immer festhalten
- Ein Panic in einer Kind-Task könnte den Zustand der übergeordneten
  Task vergiften
- Die Runtime müsste bei jedem Task-Local-Lesezugriff eine Kette von
  Eltern-Zeigern ablaufen

Das explizite Zusammenspiel aus `Context::current()` und
`Context::scope` macht die Propagierung zu einer bewussten Entscheidung
statt zu einem versteckten Standard.

## Testen

In `#[tokio::test]` oder `#[suprnova_test]` wird standardmäßig kein
`Context`-Scope installiert. Der meiste getestete Code, der `Context`
berührt, kommt mit dem Fall "kein Scope" sauber zurecht (stiller No-Op +
`None` beim Lesen), schlichte Unit-Tests brauchen also kein Setup.

Zwei Situationen, in denen der Test Hilfe braucht:

### Wenn der zu testende Code `query_param` aufruft

Die Paginierungs-Helfer lesen `?page=` über `Context::query_param`. Ein
Unit-Test für "Seite 3 liefert den richtigen Offset" braucht ein
`query_param`, das `Some("3")` liefert. Es gibt zwei Wege:

**`test_query_guard` (empfohlen):**

```rust
use suprnova::Context;

#[tokio::test]
async fn paginate_reads_page_from_query() {
    let _q = Context::test_query_guard("page", "3");

    // Der zu testende Code sieht jetzt ?page=3
    assert_eq!(Context::query_param("page"), Some("3".into()));

    let posts = Post::query().paginate(15).await?;
    assert_eq!(posts.current_page(), 3);
}
// `_q` fällt am Ende des Scopes weg - die Thread-Local-Überschreibung ist gelöscht.
```

`test_query_guard` liefert einen RAII-Guard. Selbst wenn der Test-Body
einen Panic auslöst, läuft `Drop` und räumt die
Thread-Local-Überschreibung ab, bevor der OS-Thread wiederverwendet
wird. Der Guard ist `#[must_use]` - ihn an `_` zu binden, räumt sofort
ab, was fast nie das ist, was Sie wollen.

**Blankes `test_set_query` + `test_clear_query`:**

```rust
#[tokio::test]
async fn manual_pair() {
    Context::test_clear_query();        // Leck aus einem Geschwister-Test beseitigen
    Context::test_set_query("page", "5");

    // … Assertions …

    Context::test_clear_query();
}
```

Verwenden Sie die Guard-Form. Das manuelle Paar gibt es für Fälle, in
denen Sie mehrere Überschreibungen unabhängig voneinander setzen und
abräumen müssen, aber der `#[must_use]`-Guard ist schwerer falsch zu
benutzen.

Beide APIs hängen an `#[cfg(any(test, feature = "testing"))]` - sie
werden in Test-Binaries kompiliert und in Release-Builds, die das
`testing`-Feature für Integrationstest-Harnesses aktivieren. In
schlichten Release-Builds existieren sie nicht.

### Wenn der zu testende Code aus einem `Context`-Scope liest oder schreibt

Installieren Sie einen explizit über `Context::scope`:

```rust
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_tenant_id() {
    Context::scope(ContextStore::default(), async {
        Context::add("tenant_id", "acme");

        let resolved = my_helper_that_reads_tenant().await;
        assert_eq!(resolved, "acme");
    })
    .await;
}
```

Oder belegen Sie beim Anlegen des Scopes eine Query-Bag vor:

```rust
use std::collections::HashMap;
use suprnova::context::{Context, ContextStore};

#[tokio::test]
async fn handler_reads_query_from_scope() {
    let mut q = HashMap::new();
    q.insert("page".into(), "3".into());
    q.insert("sort".into(), "name".into());

    Context::scope(ContextStore::with_query(q), async {
        assert_eq!(Context::query_param("page"), Some("3".into()));
        assert_eq!(Context::query_param("sort"), Some("name".into()));
    })
    .await;
}
```

`ContextStore::with_query(HashMap)` ist derselbe Konstruktor, den die
Request-Middleware verwendet, sodass ein Test, der denselben Codepfad
wie die Produktion durchläuft, dieselbe Form von Query-Bag sieht.

### Warum die Thread-Local-Überschreibung existiert

Die Überschreibung der Query-Parameter ist ein `thread_local!`, kein
Task-Local. Das ist Absicht: Tests können damit Query-Parameter
installieren, **ohne jede Assertion in einen `Context::scope`-Aufruf zu
hüllen**. Das Zusammenspiel ist:

1. Lesezugriffe prüfen zuerst die Thread-Local-Überschreibung
2. Gibt es keine Überschreibung, wird die Query-Bag des
   Task-Local-`CONTEXT`-Scopes gelesen
3. Gibt es auch keinen Scope, wird `None` geliefert

Der Thread-Local-Lookup kostet in der Produktion praktisch nichts
(außerhalb von Test-Builds ist die Überschreibung immer leer) und
erspart Testautoren Boilerplate-Hüllen aus `Context::scope(...)` um jede
Assertion rund um die Paginierung.

## Häufige Muster

### Die Request-ID auf jede Log-Zeile stempeln

Das macht das Framework bereits. Die Request-Middleware belegt
`_request_id` in der sichtbaren Bag vor, damit nachgelagerte Jobs,
Broadcasts und Log-Dumps über `Context::all()` die ID über den Namen
lesen können. Dieselbe Middleware öffnet außerdem einen `tracing`-Span,
der die ID als Span-Feld trägt - das ist es, was sie auf jeder innerhalb
der Anfrage ausgegebenen Log-Zeile auftauchen lässt; siehe
[Protokollierung](logging.md) für die Seite des Subscribers. Die ID aus
`Context` zu lesen, ist der richtige Weg, wenn Sie den Wert als String
brauchen (etwa um ihn einer ausgehenden HTTP-Anfrage als
Korrelations-Header mitzugeben):

```rust
let request_id: Option<String> = Context::get("_request_id");
```

### Tenant-Kontext in einen eingereihten Job mitnehmen

`Context` propagiert nicht automatisch über die Serialisierungs- und
Deserialisierungsgrenze der Warteschlange - der Worker läuft in einem
anderen Prozess als der Dispatcher, oft auf einer anderen Maschine.
Geben Sie alles, was Sie brauchen, in den Payload des Jobs:

```rust
use suprnova::{Context, FrameworkError, Queue};

// In einem Handler:
let tenant_id: String = Context::get("tenant_id")
    .ok_or_else(|| FrameworkError::param("tenant_id missing"))?;

Queue::push(SendInvoice { tenant_id, invoice_id }).await?;
```

Wenn der Worker `SendInvoice` verarbeitet, installieren Sie oben in
`Job::handle` einen frischen `Context`-Scope und belegen die Schlüssel,
die Sie brauchen, aus dem Job-Payload erneut vor: ein
`Context::scope(ContextStore::default(), async { ... })` um den Body
herum. Dann sieht jede Protokollierung und jeder tief verschachtelte
Helfer, den der Job aufruft, dieselbe Tenant-ID wie innerhalb einer
Anfrage.

Hier zahlt sich auch `hidden_add` aus - der Job kann einen
API-Schlüssel einmal beim Betreten des Scopes holen und ablegen, und
jeder nachgelagerte HTTP-Aufruf innerhalb des Jobs liest ihn über
`Context::hidden_get`, ohne ihn erneut zu holen. Siehe
[Warteschlangen](queues.md) für die Form des `Job`-Traits.

### Audit-Trail über eine Anfrage hinweg

```rust
Context::push("audit.steps", "validated_input");
// … weitere Arbeit …
Context::push("audit.steps", "charged_card");
// … weitere Arbeit …
Context::push("audit.steps", "sent_receipt");

// In der Middleware zur Response-Zeit:
let steps: Vec<String> = Context::get("audit.steps").unwrap_or_default();
tracing::info!(?steps, "request audit trail");
```

Eine Middleware zur Response-Zeit, die nach dem Handler läuft, kann den
Audit-Trail in einer einzigen Log-Zeile ausgeben, statt für jeden
Schritt eine eigene Debug-Zeile über das Request-Log zu verstreuen.

### Versteckte Bag für Credentials einer SDK-Erweiterung

```rust
// Beim Eintritt der Anfrage, nach der Authentifizierung:
Context::hidden_add("sdk.api_key", load_api_key_for(user_id));

// Tief in einem SDK-Aufruf:
let key = Context::hidden_get::<String>("sdk.api_key")
    .ok_or_else(|| FrameworkError::param("api key not stashed"))?;
```

Logs, die `Context::all()` ausgeben, zeigen den Schlüssel nicht. Die
versteckte Bag ist der richtige Ort für jedes Credential, das der
Handler tief in einen Call-Stack reichen muss, ohne es Log-Oberflächen
preiszugeben.

## Warum Suprnova abweicht

Laravels `Context`-Facade (eingeführt in Laravel 11) ist die
Inspiration - dieselben Methodennamen, dieselbe Trennung in sichtbar und
versteckt, derselbe Vertrag "still außerhalb einer Anfrage". Zwei
Unterschiede kommen von Rusts Runtime:

**Asynchrone Propagierung ist explizit, nicht magisch.** Laravels
`Context` fließt automatisch durch eingereihte Jobs, weil Laravel die
Kontext-Bag beim Dispatch in den Job-Payload serialisiert. Rusts
Async-Modell kennt keine einzelne "aktuelle Anfrage", in die
Thread-Locals hineinfließen - `tokio::spawn` startet frisch, und die
Grenze der Warteschlange bringt Serialisierung über Prozesse hinweg mit
sich. Suprnova legt das Primitiv für die Propagierung offen
(`Context::current()` + `Context::scope`) und lässt Sie sich an der
Grenze bewusst dafür entscheiden, statt so zu tun, als würden Tasks
einen Kontext erben, den sie nicht erben.

**Lesezugriffe mit falschem Typ sind beobachtbar.** Ein `get::<T>` auf
einen Wert, der mit einem anderen Typ abgelegt wurde, liefert in Laravel
stillschweigend `None` (es ist PHP, die Typen wurden beim Schreiben
ohnehin nicht erzwungen). In Suprnova gibt der Lesezugriff ein
`tracing::trace!` aus, weil der Fall mit dem falschen Typ auf einen
echten Bug hinweist - der Wert wurde irgendwo geschrieben, nur nicht mit
dem Typ, mit dem Sie lesen. Der Trace lässt Sie ihn in instrumentierten
Läufen finden, ohne den No-Panic-Vertrag zu ändern.

Die dritte Abweichung ist mechanischer Natur: Suprnovas `Context` baut
auf `tokio::task_local!` auf, seine Lebensdauer hängt also an der
Tokio-Task und nicht an globalem Zustand. Lesezugriffe über Threads
hinweg sehen den Scope der **Task, die gerade auf diesem Thread läuft**,
und nicht irgendeinen zuletzt installierten Scope. Genau das macht es
sicher, dieselbe `Context`-Facade aus einem Thread-Pool, einem Actor
oder einem `spawn_blocking`-Body aufzurufen - sofern Sie den Scope in
den Spawn propagieren.

## Wo es lebt

| Thema | Datei |
|---|---|
| `Context`-Facade + `ContextStore` | `framework/src/context/mod.rs` |
| Installation des Scopes bei einer HTTP-Anfrage | `framework/src/logging/request_id.rs` |
| Aufrufer von `Context::query_param` (Paginierung) | `framework/src/eloquent/builder.rs` |
| Re-Exporte | `framework/src/lib.rs` (`pub use context::{Context, ContextStore}`) |

## Nächste Schritte

- [Request-Lifecycle](lifecycle.md) - wo der `Context`-Scope bei jeder
  Anfrage installiert wird
- [Service Container](container.md) - für Zustand über Anfragen hinweg,
  der eine einzelne Task überdauert
- [Protokollierung](logging.md) - wie `Context::all()` in strukturierten
  Log-Zeilen landet
- [Paginierung](pagination.md) - der wichtigste nachgelagerte Leser von
  `Context::query_param`
- [Testen](testing.md) - `test_query_guard`- und
  `Context::scope`-Muster für Unit-Tests
