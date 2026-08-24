# Feature Flags

Suprnovas Feature-Flag-System kombiniert `Feature`-Deklarationen zur
Compile-Zeit mit Laufzeit-Overrides, die in einer `features`-Tabelle
persistiert werden. Der Wert eines Flags wird zum Zeitpunkt der
Auswertung in dieser Reihenfolge bestimmt:

1. Eine gescopte Zeile in der `features`-Tabelle - `user:42` oder
   `team:staff`.
2. Die globale Zeile in der `features`-Tabelle (Scope `""`).
3. Der `default`, der zur Compile-Zeit fest in die
   `Feature`-Deklaration eingebacken ist.

Umschaltungen über das Admin-CRUD propagieren zu den aktiven
Evaluatoren, bevor der Mutations-Aufruf zurückkehrt. Kill-Switch-Flags
deaktivieren tatsächlich in Echtzeit, nicht „irgendwann im nächsten
TTL-Fenster“.

## Schnellstart

```rust
// app/src/features.rs - hier lebt jedes Flag, das Ihre App referenziert.
use suprnova::features::Feature;

pub const NEW_CHECKOUT_FLOW: Feature<'static> = Feature::new("new-checkout-flow", false);
```

```rust
// app/src/bootstrap.rs - die Kette einmal beim Boot verdrahten.
use std::time::Duration;
use suprnova::features::{bootstrap_database_cached, FeatureMiddleware};

pub async fn register() {
    // ... DB::init, Session usw.

    bootstrap_database_cached(Duration::from_secs(60))
        .await
        .expect("feature flags wired");

    global_middleware!(FeatureMiddleware::new());
}
```

```rust
// beliebiger Handler - Feature::is_enabled() löst gegen den Kontext dieser Anfrage auf.
use crate::features::NEW_CHECKOUT_FLOW;

pub async fn index(req: Request) -> Response {
    let banner = if NEW_CHECKOUT_FLOW.is_enabled() {
        Some("Try the new checkout - faster, fewer steps.")
    } else {
        None
    };
    // ...
}
```

```rust
// das Flag von einer Admin-Route oder der CLI aus umschalten:
use suprnova::features::admin;

let actor_id = Auth::id();  // Option<String> - None für systeminitiierte Änderungen
admin::upsert("new-checkout-flow", "", true, None, actor_id).await?;
//                                  ^   ^                  ^
//                                  |   |                  └ Audit: wer es umgeschaltet hat
//                                  |   └ enabled
//                                  └ scope_key: "" = global, "user:42" = gescopter Override
```

Der nächste Aufruf von `NEW_CHECKOUT_FLOW.is_enabled()` sieht `true` -
einschließlich jedes zwischengespeicherten Evaluator-Eintrags, der
innerhalb von `admin::upsert` synchron invalidiert wurde.

## Die Bausteine

### `Feature<'a>`

Die Deklaration zur Compile-Zeit. Trägt den Namen des Flags und einen
`default`-Wert für den Fall, dass kein Eintrag existiert.

```rust
pub const KILL_SWITCH_PAYMENTS: Feature<'static> =
    Feature::new("kill-switch.payments", true);
//                                      ^ default: true (Zahlungen aktiviert, bis sie deaktiviert werden)
```

Alle Deklarationen zentral in `app/src/features.rs` zu sammeln gibt
Ihnen:

- eine einzige Stelle zum Grep, wenn ein Operator fragt „welche Flags
  gibt es?“
- Eindeutigkeit des Flag-Namens zur Compile-Zeit - ein Tippfehler an
  der Aufrufstelle kompiliert nicht
- die naheliegende Stelle für einen Doc-Kommentar, der erklärt, was
  das Flag steuert

Rufen Sie `flag.is_enabled()` auf, um gegen den umgebenden Kontext zu
lesen (aufgebaut von [`FeatureMiddleware`](#featuremiddleware)), oder
`flag.is_enabled_in(Some(&ctx))`, um einen konkreten
[`Context`](https://docs.rs/featureflag/latest/featureflag/context/struct.Context.html)
zu übergeben.

Die Makros `feature!` und `is_enabled!` werden ebenfalls aus
`suprnova::*` re-exportiert, für Aufrufstellen, die die Konstante
nicht importieren wollen:

```rust
use suprnova::is_enabled;

if is_enabled!("new-checkout-flow", false) {
    // ...
}
```

### `DatabaseEvaluator`

Liest die `features`-Tabelle beim Boot und bei jedem
[`reload()`](#ablauf-flag-propagation) in einen In-Memory-Snapshot.
Der Hot Path (`is_enabled`) ist vollständig synchron - keine
DB-Query pro Anfrage, kein `block_on` innerhalb des Evaluators.

Auflösungsreihenfolge beim Lookup, spezifischste Zeile zuerst:

1. `user:{id}` - wenn der Request-Kontext ein `UserIdField` trägt.
2. `team:{name}` - wenn der Kontext ein `TeamField` trägt.
3. `""` - das globale Flag.
4. `None` - die Zeile existiert nicht, der `default` zur Compile-Zeit
   übernimmt.

### `CachedEvaluator`

Memoisiert `(feature, user, team)`-Lookups hinter einer `DashMap` mit
einer TTL Ihrer Wahl. Der Hot Path bleibt synchron; Einträge werden
synchron verworfen, wenn [`admin::upsert`](#admin-crud) ein Flag
schreibt.

Eine TTL von null entartet zu „kein Cache“ - jeder Aufruf fällt durch
zum inneren Evaluator. Nützlich für Apps mit wenigen Flags, die die
Propagations-Verdrahtung wollen, ohne den Cache.

### `FeatureMiddleware`

Öffnet einen Featureflag-Kontext pro Anfrage, befüllt von
benutzerdefinierten Extraktoren. Defaults:

- `user_id` - aus `Auth::id()`.
- `team` - keiner.

Überschreiben Sie beides über den Builder:

```rust
let middleware = FeatureMiddleware::new()
    .with_user_id_extractor(|req| {
        // Eigene Lösung: aus einem Header statt aus der Session lesen.
        req.header("X-User-Id").map(String::from)
    })
    .with_team_from_header("X-Team");
// oder: .with_team_extractor(|req| your_custom_team_resolver(req))

global_middleware!(middleware);
```

### Admin-CRUD

`suprnova::features::admin` ist die Persistenzschicht für die
`features`-Tabelle. Verwenden Sie sie aus Admin-Handlern,
CLI-Tools, Deployment-Skripten - überall, wo ein Flag umgeschaltet
werden muss:

```rust
use suprnova::features::admin;

// Ein globales Flag anlegen oder aktualisieren.
admin::upsert("kill-switch.payments", "", false, Some("ops-2026-05-19".into()), actor_id).await?;
// Argumente: name, scope_key, enabled, description, actor_id

// Benutzer-gescopter Override (schlägt das globale Flag).
admin::upsert("new-checkout-flow", "user:42", true, None, actor_id).await?;

// Eine Zeile vollständig entfernen - das Flag fällt auf den
// Default zur Compile-Zeit zurück.
admin::delete("kill-switch.payments", "", actor_id).await?;

// Für eine Admin-UI-Tabelle lesen.
let all_flags = admin::list().await?;
let one_row = admin::get("kill-switch.payments", "").await?;
```

Jede Mutation feuert das passende [Ereignis](#ereignisse) und ruft
[`features::sync::notify`](#ablauf-flag-propagation) auf, damit jeder
im App-Container gebundene aktive Evaluator sich aktualisiert, bevor
der Aufruf zurückkehrt.

`actor_id: Option<String>` ist der Audit-Zeiger. Übergeben Sie die
User-ID des Operators (dieselbe, die Ihre Auth-Schicht ausstellt);
lassen Sie `None` für systeminitiierte Änderungen (CLI,
Deploy-Migration usw.).

## Ablauf: Flag-Propagation

Der Trait, der „Admin-Umschaltung sofort sichtbar“ funktionieren
lässt:

```rust
#[async_trait]
pub trait FeatureSync: Send + Sync + 'static {
    async fn on_flag_changed(&self, feature: &str, scope_key: &str);
}
```

Implementierungen reagieren auf Mutationen:

- `DatabaseEvaluator::on_flag_changed` ruft `self.reload()` auf -
  zieht den vollständigen Snapshot.
- `CachedEvaluator::on_flag_changed` ruft `self.invalidate(feature)`
  auf - verwirft jeden zwischengespeicherten Eintrag für diesen
  Namen.

Die kanonische Kette ist ein `CompositeFeatureSync`, der
**Datenquellen vor Caches ordnet** - Caches müssen sich *nach* der
Aktualisierung der Datenquelle invalidieren, sonst kann ein
gleichzeitiger Leser auf den leeren Cache treffen, zur veralteten
Datenquelle durchfallen und den Cache mit dem alten Wert erneut
befüllen.

```rust
let composite = CompositeFeatureSync::new(
    vec![database.clone() as Arc<dyn FeatureSync>], // Datenquellen zuerst
    vec![cached.clone() as Arc<dyn FeatureSync>],   // Caches danach
);
App::bind::<dyn FeatureSync>(composite);
```

`features::sync::notify(feature, scope_key)` löst `Arc<dyn
FeatureSync>` aus dem Container auf und wartet auf
`on_flag_changed`. No-op, wenn kein Sync gebunden ist - das richtige
Verhalten für prozessexterne Admin-Tools, die nur in die DB
schreiben und keinen aktiven Evaluator zum Aktualisieren haben.

## Bootstrap-Helfer

`bootstrap_database_cached(ttl)` verdrahtet alles in einem Aufruf:

```rust
let features = bootstrap_database_cached(Duration::from_secs(60))
    .await
    .expect("feature flags wired");

// Optional: features.database aufheben, um periodische Reloads zu
// planen oder Admin-Diff-Ansichten anzubieten. Die meisten Apps
// lassen das Handle fallen und überlassen die Aktualisierung dem
// notify-getriebenen Refresh.
```

Was es tut:

1. Baut `DatabaseEvaluator` gegen die primäre DB-Connection.
2. Umschließt ihn mit `CachedEvaluator` mit der angeforderten TTL.
3. Ruft `install_evaluator(cached)` auf - setzt den globalen
   Featureflag-Default *und* schaltet einen framework-eigenen
   „installiert“-Tracker um, damit die Middleware nicht die Warnung
   „kein Evaluator“ protokolliert.
4. Baut ein `CompositeFeatureSync` mit der richtigen Slot-Reihenfolge
   und bindet es in den App-Container.

Liefert `BootstrappedFeatures { database, cached }` für Aufrufer, die
direkte Handles auf beide Schichten wollen.

Wenn Ihre Topologie nicht `Cached(Database)` ist - ein
Redis-gestützter Cache, eine entfernte Sync-Quelle, eine
mehrstufige Kette - verdrahten Sie die Kette manuell mit denselben
Bausteinen. `bootstrap_database_cached` ist Komfort, kein Vertrag.

## Migrationen

Das Framework besitzt das Schema der `features`-Tabelle:

```rust
// app/src/migrations/mod.rs
vec![
    // ... die Migrationen Ihrer App ...
    Box::new(suprnova::features::migrations::CreateFeaturesTable),
]
```

Schema:

```sql
features (
    id          BIGINT      PRIMARY KEY AUTO_INCREMENT,
    name        VARCHAR(255) NOT NULL,
    scope_key   VARCHAR(255) NOT NULL DEFAULT '',
    enabled     BOOLEAN     NOT NULL,
    description TEXT,
    updated_by  VARCHAR(255),
    created_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at  TIMESTAMP   NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE INDEX (name, scope_key)
)
```

`scope_key` trägt die Art des Scopes inline (`"user:42"`,
`"team:staff"`, `""` für global), sodass der Lesepfad ein einzelner
String-Lookup gegen einen Unique-Index bleibt.

## Benutzer- und Team-IDs

`UserIdField` und `TeamField` sind typisierte Erweiterungen, die in
`featureflag::Context::extensions` gespeichert werden. Beide haben den Typ
String, sodass opake Benutzer-IDs des Frameworks oder von Magnetar und
numerische Werte aus `app_users.id` dieselbe Evaluierungsform verwenden.

Einen Kontext von Hand aufbauen (außerhalb der Middleware):

```rust
use featureflag::context;
use std::sync::Arc;

let ctx = featureflag::evaluator::with_default(cached.clone(), || {
    // String-Benutzer-IDs - UUIDs, ULIDs, alles Opake.
    context! { user_id = "01HZK6V3J7Q5G4P8X9N2D1B0M3".to_string(), team = "staff".to_string() }
});

// numerische IDs funktionieren weiterhin - das Framework zwingt i64 → String zum Zeitpunkt von on_new_context.
let ctx_numeric = featureflag::evaluator::with_default(cached.clone(), || {
    context! { user_id = 42_i64 }
});
```

## Ereignisse

Zwei Ereignisse feuern auf dem Admin-CRUD-Pfad:

```rust
pub struct FeatureUpdated {
    pub name: String,
    pub scope_key: String,
    pub enabled: bool,
    pub actor_id: Option<String>,
}

pub struct FeatureDeleted {
    pub name: String,
    pub scope_key: String,
    pub actor_id: Option<String>,
}
```

Lauschen Sie über den Event-Dispatcher des Frameworks darauf, um ein
Audit-Log, einen Slack-Alert oder jede andere nachgeschaltete
Pipeline zu füttern, die Sie brauchen:

```rust
EventFacade::listen::<FeatureUpdated, _>(Arc::new(FlagChangeAuditor)).await;
```

**`is_enabled` feuert kein Read-Path-Ereignis.** Jede Anfrage, die ein
Flag prüft, würde das Ereignisvolumen mit der Zahl der geprüften
Flags multiplizieren - für eine Audit-der-Mutationen-Geschichte in
Ordnung, für Read-Path-Tracing untragbar. Wenn Ihr Deployment
gesampeltes Read-Path-Audit braucht, legen Sie einen eigenen
Evaluator darüber, der in einen begrenzten Log-Kanal aufzeichnet
(einen Redis-Stream oder eine Fanout-Queue, je nach Skala).

## Erkennung eines fehlenden Evaluators

Wenn `FeatureMiddleware` installiert ist, aber kein Evaluator über
`install_evaluator` / `bootstrap_database_cached` registriert wurde,
liefert jedes Flag stillschweigend seinen `default` zur Compile-Zeit
zurück - eine harte Fehlkonfiguration, die in der QA auffallen soll.
Die Middleware gibt genau eine `tracing::warn!` pro Prozess aus, bei
der ersten Anfrage, die diesen Zustand beobachtet:

```
WARN suprnova::features: FeatureMiddleware is in the stack but no feature-flag evaluator is installed.
     is_enabled!() calls will return compile-time defaults until features::bootstrap_database_cached(...)
     or features::install_evaluator(...) is called during app boot.
```

Die Umschaltung nutzt ein `AtomicBool::swap`, sodass ein
gleichzeitiger Anfragen-Sturm beim Boot auf eine einzige
Warnungsausgabe serialisiert wird, nicht auf eine pro Worker.

## Testen

Zwei Muster, abhängig davon, was Sie prüfen.

### Ein Feature isoliert per Unit-Test prüfen

Verwenden Sie `featureflag::evaluator::with_default`, um einen
Platzhalter-Evaluator innerhalb eines synchronen Closures zu scopen:

```rust
#[test]
fn flag_enabled_returns_new_path() {
    use featureflag::evaluator::with_default;
    use suprnova::features::DatabaseEvaluator;

    let flagger = Arc::new(tokio_test::block_on(async {
        let e = DatabaseEvaluator::new_in_memory().await.unwrap();
        e.set_flag("new-checkout-flow", "", true).await.unwrap();
        e
    }));

    with_default(flagger, || {
        assert!(crate::features::NEW_CHECKOUT_FLOW.is_enabled());
    });
}
```

`DatabaseEvaluator::new_in_memory()` ist ein testeigener Helfer, der
seine eigene SQLite bootet und `CreateFeaturesTable` ausführt, damit
der Test hermetisch bleibt. Verwenden Sie ihn nicht in
Produktionspfaden.

### Propagation Ende-zu-Ende per Integrationstest prüfen

Verwenden Sie `TestDatabase::fresh::<TestMigrator>()` für die DB und
`TestContainer::bind` (NICHT `App::bind`) für den FeatureSync -
parallele Tests im selben Prozess würden sich sonst über den
globalen Container gegenseitig die Bindung überschreiben:

```rust
#[tokio::test]
async fn admin_upsert_propagates_to_cached_chain() {
    use std::sync::Arc;
    use std::time::Duration;
    use suprnova::features::sync::FeatureSync;
    use suprnova::features::{admin, CachedEvaluator, CompositeFeatureSync, DatabaseEvaluator};
    use suprnova::features::migrations::CreateFeaturesTable;
    use suprnova::testing::{TestContainer, TestDatabase};

    struct TestMigrator;
    impl sea_orm_migration::MigratorTrait for TestMigrator {
        fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
            vec![Box::new(CreateFeaturesTable)]
        }
    }

    let _db = TestDatabase::fresh::<TestMigrator>().await.unwrap();

    let database = Arc::new(DatabaseEvaluator::new().await.unwrap());
    let cached = Arc::new(CachedEvaluator::new(
        database.clone() as Arc<dyn featureflag::evaluator::Evaluator + Send + Sync>,
        Duration::from_secs(60),
    ));
    let composite = Arc::new(CompositeFeatureSync::new(
        vec![database.clone() as Arc<dyn FeatureSync>],
        vec![cached.clone() as Arc<dyn FeatureSync>],
    ));
    TestContainer::bind::<dyn FeatureSync>(composite);

    let ctx = featureflag::evaluator::with_default(cached.clone(), || {
        featureflag::context! { user_id = "user-42".to_string() }
    });

    assert_eq!(cached.is_enabled("new-feature", &ctx), None);
    admin::upsert("new-feature", "", true, None, None).await.unwrap();
    assert_eq!(cached.is_enabled("new-feature", &ctx), Some(true)); // propagiert sofort
}
```

Die vollständige Sammlung an Kompositions-Tests finden Sie in
`framework/tests/features.rs`.

### Warum Suprnova abweicht

Laravel Pennant löst jedes Flag bei Bedarf gegen die Datenbank auf
(mit optionaler Memoisierung auf Treiber-Ebene pro Anfrage). Das
Request-pro-Prozess-Modell von PHP macht einen DB-Zugriff pro
Anfrage günstig, weil die Connection dediziert ist und mit der
Anfrage stirbt.

Suprnovas Prozessmodell ist das Gegenteil - eine einzige,
lang laufende Binary bedient Tausende gleichzeitiger Anfragen. Ein
DB-Zugriff pro Anfrage bei jeder Flag-Prüfung würde die Last des
Connection-Pools mit der Zahl der Flag-Prüfungen multiplizieren. Die
zweistufige Kette (`DatabaseEvaluator`-Snapshot +
`CachedEvaluator`-TTL) ist die Rust-native Antwort: Der Hot Path ist
vollständig synchron gegen In-Memory-Daten, und der `FeatureSync`-
Trait gibt operatorseitig ausgelösten Änderungen eine Propagation
unter einer Sekunde, ohne pollenden Reload. Die Form ist dieselbe
wie bei Pennant - ein Flag definieren, es in einem Handler prüfen,
es von einer Admin-Route überschreiben. Die Verdrahtung ist anders,
weil die Runtime anders ist.

## Anmerkungen zum Design

- **Warum ein synchroner statt ein asynchroner Evaluator?** Der
  `is_enabled` von featureflag ist der Hot Path. Ein asynchroner
  Evaluator würde entweder ein `block_on` erzwingen
  (deadlock-anfällig) oder jeden Handler zwingen, bei
  Flag-Lesevorgängen `.await` zu verwenden (ergonomische
  Katastrophe). Das Framework überbrückt sync ↔ async über einen
  In-Memory-Snapshot, der asynchron von `FeatureSync` aktualisiert
  wird.

- **Warum ein eigener `FeatureSync`-Trait statt einer Erweiterung von
  `Evaluator`?** Der `Evaluator` von featureflag gehört einer
  Upstream-Crate; wir können ihm keine Methoden hinzufügen.
  `FeatureSync` ist ein Schwester-Trait, den Apps auf denselben
  konkreten Typen implementieren. Das Trait-Objekt wird separat im
  App-Container gebunden, damit ein Prozess mehrere Evaluatoren
  schichten kann, während Benachrichtigungen weiterhin korrekt
  geroutet werden.

- **Warum ist `set_flag` auf `DatabaseEvaluator` `pub`?** Zur
  Bequemlichkeit in Tests. Der Produktions-Schreibpfad ist
  `admin::upsert`; `set_flag` existiert, damit Tests Flags seeden
  können, ohne einen `EventFacade`-Listener aufzusetzen. Beide Pfade
  rufen `features::sync::notify` auf, sodass der
  Propagations-Vertrag in jedem Fall hält.

- **Warum kein `FeatureRetrieved`-Ereignis?** Volumen. Ein Handler,
  der zehn Flags pro Anfrage prüft, feuert zehn Ereignisse pro
  Anfrage - für einen Dienst mit 1.000 Req/s sind das 36 Millionen
  Ereignisse pro Stunde, weit über dem Signal-Rausch-Verhältnis
  jeder Audit-Pipeline. Was ausgeliefert wird, ist Audit auf dem
  Mutationspfad (`FeatureUpdated` / `FeatureDeleted`); Read-Path-
  Sampling, falls nötig, legt sich darüber, über einen eigenen
  Evaluator-Wrapper.

## Nächste Schritte

- [Middleware](middleware.md) - `FeatureMiddleware` gehört nach
  `SessionMiddleware`; dieses Kapitel behandelt die Reihenfolge und
  den globalen Stack
- [Ereignisse](events.md) - lauschen Sie auf `FeatureUpdated` /
  `FeatureDeleted`, um Audit-Logs, Slack-Alerts oder nachgeschaltete
  Pipelines zu treiben
- [Service Container](container.md) - wie die `dyn FeatureSync`-
  Bindung aufgelöst wird, und warum `TestContainer::bind` für
  parallele Tests existiert
- [Testen](testing.md) - `TestDatabase::fresh::<M>()` und
  `TestContainer::fake`-Muster, auf die sich dieses Kapitel stützt
- [Authentifizierung](authentication.md) - `Auth::id()` ist der
  Default-Extraktor für die Benutzer-ID und füttert `actor_id` für
  Admin-Mutationen

Extern: Die
[Doku der featureflag-Crate](https://docs.rs/featureflag) behandelt
die Upstream-Primitive `Evaluator`, `Context` und `Feature`.
`suprnova::features::admin` ist die vollständige CRUD-Fassade -
`cargo doc --open -p suprnova` zum Durchstöbern.
