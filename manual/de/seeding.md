# Seeding

Seeder befüllen die Datenbank mit Fixture-Daten - den Zeilen, die
Ihre App braucht, bevor ein echter Nutzer irgendetwas getan hat. Man
denke an ein Standard-Admin-Konto, die kanonische Liste der Länder,
die Demo-Posts auf der Staging-Umgebung, die 50 Nutzer + 200 Posts,
von denen Ihre lokale Dev-Iterationsschleife abhängt. Sie sind das
Laufzeit-Geschwister der [Migrationen](migrations.md): Migrationen
bauen das leere Schema, Seeder befüllen es.

Ein Seeder ist eine Unit-Struktur, die den Trait `Seeder`
implementiert. Das Framework hält eine geordnete, prozessglobale
Registry; der projektspezifische Befehl `console db:seed` führt
jeden registrierten Seeder in Registrierungsreihenfolge aus, oder
einen bestimmten Seeder über `--class=<Name>`. Die meisten Seeder
laufen darauf hinaus, ein paar Zeilen zu sein, die eine
[Model-Factory](eloquent.md) aufrufen und die Factory die
Zeilenerzeugung machen lassen.

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

Registrieren Sie ihn einmal beim Boot:

```rust
// src/bootstrap.rs
suprnova::seed::register::<crate::seeders::UsersSeeder>();
```

Dann:

```bash
cargo run --bin console -- db:seed
# running seeder UsersSeeder
# (50 rows inserted)
```

Das ist die ganze Schleife. Der Rest dieses Kapitels deckt die
Layout-Konventionen ab, die größeren Muster zur
Registry-Komposition, das Ziel-Flag `--class`, die
Factory-Integration, den Ausweichhaken `without_events` und die
Entscheidung Seeder-vs-Migration-vs-Factory.

## Einen Seeder schreiben

Ein Seeder ist ein Unit-Typ plus eine `Seeder`-Impl. `name()` ist der
Registry-Key (auch das, worauf `db:seed --class=<Name>` matcht), und
`run()` ist die async fn, die die Inserts ausführt.

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

`Seeder` wird an der Crate-Wurzel re-exportiert, sodass
`use suprnova::Seeder` reicht - Sie müssen nicht bis zu
`suprnova::seed::Seeder` greifen. `async_trait` wird ebenfalls
re-exportiert (`use suprnova::async_trait`), weil die Trait-Methode
ein Future liefert und Rust `async fn` in Traits ohne das noch nicht
erlaubt.

Der Rückgabetyp `FrameworkError` ist derselbe Fehler-Umschlag, den
jede andere async-Oberfläche des Frameworks verwendet; einen `?` aus
einem Factory-Aufruf oder einem `Model::create` durchzureichen ist
die erwartete Form. Siehe [Fehlermodell](error-model.md) für die
vollständige Taxonomie.

### Layout-Konvention

Spiegelt Laravels Verzeichnis `database/seeders/`, aber an der
Quellwurzel:

```
src/
├── bootstrap.rs
├── factories/
│   ├── mod.rs
│   ├── user_factory.rs
│   └── post_factory.rs
├── seeders/
│   ├── mod.rs              // pub mod base_seeder; pub use base_seeder::BaseSeeder;
│   └── base_seeder.rs      // Seeder-Impl, registriert in bootstrap.rs
└── …
```

Generieren Sie die Datei von Hand - es gibt keinen
`make:seeder`-Generator (das ist eine Datei mit etwa zehn Zeilen
Boilerplate). Die Factories, die der Seeder aufruft, bekommen dieselbe
Behandlung.

### Ein Seeder, der andere Seeder ausführt

Das Laravel-Idiom eines einzigen übergeordneten
`DatabaseSeeder::run`, der die Pro-Model-Seeds orchestriert,
funktioniert auch hier. Statt fünf kleine Seeder in bootstrap zu
registrieren und auf deren Registrierungsreihenfolge zu vertrauen,
registrieren Sie einen zusammengesetzten Seeder und rufen Sie die
anderen selbst auf:

```rust
use suprnova::{async_trait, Factory, FrameworkError, Seeder};

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // Erst 50 Nutzer - die Post-Factory erzeugt author_id im
        // Bereich 1..=50, sodass die Referenzen aufgehen.
        UserFactory::new().count(50).create_many().await?;

        // 200 Posts, die auf die obigen Nutzer-IDs verweisen.
        PostFactory::new().count(200).create_many().await?;

        Ok(())
    }
}
```

Das ist der empfohlene Standard. Er hält die Abhängigkeitsreihenfolge
(`users` vor `posts`) innerhalb des Seeders statt über die
Bootstrap-Datei verstreut, und `db:seed --class=BaseSeeder` ist ein
Aufruf mit einem einzigen Ziel, der das ganze Bündel ausführt.

Wenn Sie Seeder lieber namentlich verketten wollen statt per
direktem Factory-Aufruf, verwenden Sie `seed::run_one` aus dem
zusammengesetzten Seeder heraus:

```rust
async fn run() -> Result<(), FrameworkError> {
    suprnova::seed::run_one("UsersSeeder").await?;
    suprnova::seed::run_one("PostsSeeder").await?;
    suprnova::seed::run_one("CommentsSeeder").await?;
    Ok(())
}
```

Die Unter-Seeder müssen trotzdem in `bootstrap.rs` registriert sein,
damit `run_one` sie findet.

## Die Seeder-Registry

Das Framework hält eine prozessglobale, geordnete Map
(`IndexMap<String, fn() -> _>`) jedes registrierten Seeders. Drei
Regler steuern sie.

### `register::<S>()`

Fügt einen Seeder unter seinem `Seeder::name()` zur Registry hinzu:

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

Zwei Dinge, die man über die Registry wissen sollte:

- **Reihenfolge zählt.** `run_all` besucht Seeder in der Reihenfolge,
  in der sie registriert wurden. Braucht `B` Zeilen von `A`,
  registrieren Sie `A` zuerst.
- **Erneutes Registrieren eines Namens ersetzt an Ort und Stelle.**
  Der Slot behält seine ursprüngliche Position, der Funktionszeiger
  ändert sich. Das ist beabsichtigt - es lässt einen Test einen
  Stub-Seeder über den echten binden, ohne die Reihenfolge zu
  verschieben. Im Produktionscode registrieren Sie jeden Seeder genau
  einmal beim Boot.

### `run_all()`

Führt jeden registrierten Seeder in Registrierungsreihenfolge aus.
Das ist es, was der bloße Aufruf `console db:seed` ausführt.

```rust
suprnova::seed::run_all().await?;
```

Stoppt beim ersten Fehler. Seeder, die schon liefen, werden nicht
zurückgerollt - `run_all` wickelt keine Transaktion um den Batch,
weil die meisten Seeder mehrere Statements umfassen und viele
Backends Transaktionen nicht sauber verschachteln. Brauchen Sie
Rollback-Semantik, öffnen Sie die Transaktion innerhalb des Seeders
und halten Sie seine gesamte Arbeit in diesem Scope.

### `run_one(name)`

Führt einen benannten Seeder aus, ohne die anderen laufen zu lassen.
Das ist die Engine für `db:seed --class=<Name>` und auch nützlich aus
Einmal-Skripten heraus:

```rust
suprnova::seed::run_one("AdminAccountSeeder").await?;
```

Verfehlt der Name, liefert es
`FrameworkError::not_found("no seeder registered for \`X\`")`. Der
Konsolenbefehl gibt das an einen Non-Zero-Exit und eine Stderr-Zeile
weiter - kein stiller No-op.

### `count()` und `is_registered(name)`

Zwei Lese-Helfer, beide nützlich in Tests, die assertieren „bootstrap
hat die erwarteten Seeder verdrahtet“:

```rust
assert_eq!(suprnova::seed::count(), 3);
assert!(suprnova::seed::is_registered("BaseSeeder"));
```

Beide liefern null / false bei einer vergifteten Sperre der Registry
(nach dem Protokollieren eines Fehlers), was Tests angesichts eines
vorgelagerten Panics deterministisch hält.

## Der Befehl `db:seed`

`db:seed` ist ein vom Framework bereitgestellter Konsolenbefehl - er
kommt mit dem Framework und landet automatisch in der `console`-Binary
Ihres Projekts, über dieselbe `inventory`-Registry, die auch Ihre
eigenen `#[command]`s aufgreift. Siehe [Konsole](console.md) für die
Mechanik der Binary; dieser Abschnitt deckt die seeder-spezifische
Oberfläche ab.

### Alles ausführen

```bash
cargo run --bin console -- db:seed
```

Führt jeden registrierten Seeder in Reihenfolge aus. Bei einer leeren
Registry gibt es eine Warnung auf Stderr aus
(`db:seed: no seeders registered - nothing to run`) und beendet sich
mit null - das ist das richtige Verhalten für „jemand hat den Befehl
ausgeführt, bevor irgendetwas registriert wurde“ und hält
Testsuiten, die noch nichts Bestimmtes geseedet haben, davon ab,
fehlzuschlagen.

### Einen Seeder ausführen

Drei akzeptierte Formen, in aufsteigender Reihenfolge, wie
Laravel-förmig sie sich anfühlen:

```bash
cargo run --bin console -- db:seed --class=UsersSeeder
cargo run --bin console -- db:seed --class UsersSeeder
cargo run --bin console -- db:seed UsersSeeder
```

Alle drei suchen den Seeder anhand des exakten Namens in der Registry
und führen ihn aus. Ein unbekannter Name schlägt fail-fast fehl:

```bash
cargo run --bin console -- db:seed --class=NotARealSeeder
# Error: no seeder registered for `NotARealSeeder`
# (exit 1)
```

Ein fehlerhaftes Flag (`--class` ohne folgenden Wert, `--class=` mit
leerem Wert, `--class --force`) schlägt ebenfalls fail-fast fehl, mit
einer Diagnose, die die erwartete Form benennt.

### Aus einer gebauten Binary

In einem containerisierten oder systemd-verwalteten Deployment lebt
die Console-Binary unter `target/release/console` (oder wo auch immer
Ihr Release-Artefakt landet). Gleiche Syntax, kein `cargo` davor:

```bash
./console db:seed
./console db:seed --class=BaseSeeder
```

Die Console-Binary ruft
`suprnova::console::dispatch_argv(std::env::args())` auf, was über
dieselbe Registry routet wie `cargo run --bin console --`. Es gibt
keinen separaten Dispatch-Pfad für gebaute Artefakte.

## Kombination mit Factories

Seeder rufen fast immer [Factories](eloquent.md) auf. Der
Factory-Trait weiß, wie man eine randomisierte Instanz eines Models
baut; der Seeder sequenziert die Factory-Aufrufe und jede nicht
randomisierbare Verdrahtung (deterministische Admin-Credentials,
Zeilen in verknüpften Tabellen, Datei-Uploads).

Das minimale Paar aus Factory und Seeder:

```rust
// src/factories/user_factory.rs
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        User {
            id: 0,                              // persist_via_seaorm stellt PK auf NotSet
            name: "Factory User".into(),
            email: "factory@example.suprnova.app".into(),
            password: "factory-placeholder".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            ..Default::default()
        }
    }
}
```

```rust
// src/seeders/users_seeder.rs
use suprnova::{async_trait, Factory, FrameworkError, Seeder};
use crate::factories::UserFactory;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        UserFactory::new().count(50).create_many().await?;
        Ok(())
    }
}
```

Der fließende Builder lebt auf `FactoryBuilder<M>`; was Sie vor
`create_many` verketten können, entspricht Laravel:

```rust
// Eine persistierte Zeile mit Overrides bauen:
let admin = UserFactory::new()
    .with(|u| u.email = "admin@example.com".into())
    .with(|u| u.role = "admin".into())
    .create()
    .await?;

// N persistierte Zeilen bauen, alle Admins:
UserFactory::times(5)
    .with(|u| u.role = "admin".into())
    .create_many()
    .await?;

// Bedingter State - wendet die Closure nur an, wenn das Flag gesetzt ist:
UserFactory::times(10)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

`make` / `make_one` / `make_many` sind die In-Memory-Geschwister
(kein Insert) für Unit-Tests, die keinen DB-Round-Trip wollen. Siehe
das Kapitel [Eloquent](eloquent.md) für die vollständige
Factory-Oberfläche (einschließlich `prepend`, `Sequence` und dem
Makro `#[derive(Factory)]`, das die Marker-Struktur aus einem
Attribut `#[factory(model = "…")]` generiert).

### Idempotenz ist die Verantwortung des Seeders

`run_all` erstellt keinen Snapshot und wickelt keine Transaktion; wenn
ein Seeder bedingungslos einfügt, erzeugt ein erneuter Lauf
Duplikate. Die zwei Standardwege, einen Seeder sicher für erneutes
Ausführen zu machen:

- **Zuerst zurücksetzen.** Die „wipe and reseed“-Schleife der lokalen
  Entwicklung macht meist
  `suprnova migrate:fresh && cargo run --bin console -- db:seed` -
  `migrate:fresh` löscht jede Tabelle und baut sie neu auf, sodass der
  Seeder immer von leer startet. Das ist die Form, die die meisten
  Projekte im Alltag verwenden.
- **Upsert / Erst-prüfen.** Für einen Seeder, der mit vorhandenen
  Daten koexistieren muss (ein Standard-Admin-Konto in Produktion,
  die kanonische Liste der Länder), sichern Sie den Insert mit einem
  Lookup ab oder verwenden Sie eine Upsert-Query.

```rust
async fn run() -> Result<(), FrameworkError> {
    let exists = User::query()
        .db_where("email", "admin@example.com")
        .exists()
        .await?;

    if !exists {
        let password_hash = suprnova::hashing::hash("change-me-on-first-login")?;
        User::create(attrs!{
            email: "admin@example.com",
            name: "Admin",
            password: password_hash,
        }).await?;
    }
    Ok(())
}
```

## Model-Events mit `without_events` stummschalten

Ein Seeder, der `Model::create` in einer Schleife aufruft, löst bei
jeder Zeile jedes Lifecycle-Event aus - `Creating`, `Saving`,
`Created`, `Saved`. Das weckt jeden registrierten `Observer<M>`, führt
jeden eingereihten Broadcast-Listener aus, und kann nebenbei hundert
Hintergrund-Jobs einreihen, die Sie eigentlich nicht wollen.
`seed::without_events` ist das Laravel-`WithoutModelEvents`-Analogon:

```rust
use suprnova::{async_trait, FrameworkError, Seeder, seed};
use crate::models::users::User;

pub struct UsersSeeder;

#[async_trait]
impl Seeder for UsersSeeder {
    fn name() -> &'static str { "UsersSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        seed::without_events(async {
            for i in 0..50 {
                User::create(attrs!{
                    name: format!("user{i}"),
                    email: format!("user{i}@example.com"),
                }).await?;
            }
            Ok(())
        }).await
    }
}
```

Während das innere Future wartet, kurzschließen sowohl der
abbrechbare Veto-Pfad (`dispatch_cancellable`) als auch der
Nach-Event-Fanout (`dispatch_after`) auf `Ok(())`. Observer bleiben
still, der Broadcaster wacht nicht auf, nachgelagerte Jobs reihen
sich nicht ein.

Der Effekt ist task-scoped - nur Arbeit, die innerhalb von `fut`
ausgeführt wird, wird stummgeschaltet. Gleichzeitige Arbeit auf
anderen Tasks (HTTP-Request-Handler, im Hintergrund laufende
Queue-Worker, andere Seeder) feuert Events weiter normal. Verschachtelte
Aufrufe komponieren: Ein innerer `without_events`-Block erbt das
äußere Flag.

### Factories umgehen Model-Events bereits

Gut zu wissen, weil es ändert, wann Sie zu `without_events` greifen:
Factories persistieren über `ActiveModelTrait::insert` (die
`Persistable`-Impl auf dem SeaORM-Model), was nicht über die
Methoden `create` / `save` des `Model`-Traits geht. Es gibt keinen
Model-Event-Dispatch, den man auf einem factory-getriebenen Pfad
stummschalten müsste. `seed::without_events` ist für Code, der den
`Model`-Trait direkt treibt - typischerweise, weil Sie die
Laufzeitform-Ergonomie brauchen, die Factories umgehen, oder weil Sie
mitten im Seed ein Model berühren, auf das ein Observer in Produktion
reagieren soll, aber nicht während eines Fixture-Ladevorgangs.

In der Praxis: Ist Ihr Seeder ein Stapel von
`UserFactory::new().create_many()`-Aufrufen, brauchen Sie
`without_events` nicht. Ist es eine handgerollte Schleife aus
`User::create(attrs)`, brauchen Sie es wahrscheinlich.

## Seeder in Tests verwenden

Dieselbe Registry, die die Console-Binary treibt, ist aus einem
`#[tokio::test]` heraus aufrufbar - nützlich, wenn Sie einen bekannten
Fixture-Satz vor einem Integrationstest wollen:

```rust
use serial_test::serial;
use suprnova::container::testing::TestContainer;
use suprnova::{DbConnection, seed};

use app::seeders::BaseSeeder;

#[tokio::test]
#[serial]
async fn dashboard_renders_seeded_posts() {
    // Registry zurücksetzen, damit die Registrierungen eines
    // vorherigen Tests nicht durchsickern.
    seed::clear();

    let _guard = TestContainer::fake();
    let conn = sea_orm::Database::connect("sqlite::memory:").await.unwrap();
    app::migrations::Migrator::up(&conn, None).await.unwrap();
    TestContainer::singleton(DbConnection::from_raw(conn.clone()));

    // Den gewünschten Seeder registrieren, ausführen und gegen die
    // frische Datenbank assertieren.
    seed::register::<BaseSeeder>();
    seed::run_all().await.unwrap();

    // …Controller-Test gegen die geseedeten Daten…

    seed::clear();
}
```

Zwei Anmerkungen zur Test-Form:

- `#[serial]` ist erforderlich, wenn der Test die prozessglobale
  Registry verändert - parallele Tests, die sich dieselbe Registry
  teilen, würden um die Wette laufen. Fügen Sie `serial_test` als
  Dev-Dependency in das `Cargo.toml` Ihres Projekts ein, um das
  Attribut zu bekommen.
- `seed::clear()` ist ein `#[doc(hidden)]`-Helfer nur für Tests.
  Rufen Sie ihn nicht aus Produktionscode auf; die Registry wird
  einmal beim Boot gebaut und nie zurückgesetzt.

Siehe [Testen](testing.md) für die breiteren Konventionen des
Test-Harness (`#[suprnova_test]`, `TestContainer`,
`TestDatabase::fresh::<Migrator>()`, die Fakes für jede externe
Oberfläche).

## Wann seeden, migrieren oder eine Factory verwenden

Diese drei Muster bringen alle Zeilen in Tabellen. Die Entscheidung
ist meist unkompliziert, aber es lohnt sich, die Abgrenzungen explizit
zu benennen, weil PHP-Teams sie oft verwischen.

| Sie wollen … | Verwenden Sie |
|---|---|
| Eine Spalte, die existiert | [Migration](migrations.md) |
| Eine Zeile, die existieren muss, damit die App booten kann (der Standard-Admin, die Singleton-Site-Config-Zeile, die kanonische Liste der Währungen) | **Seeder** - idempotent, läuft in jeder Umgebung, auch in der Produktion |
| Eine randomisierte Menge von Zeilen für lokale Entwicklung oder Staging (50 Nutzer, 200 Posts, 1000 Events) | Seeder, der eine Factory aufruft |
| Eine Zeile, die ein Unit-Test braucht | [Factory](eloquent.md), direkt im Test aufgerufen |
| Die Form einer Zeile | [Factory](eloquent.md) |

Die Fehler, die man vermeiden sollte:

- **Fügen Sie keine Daten aus einer Migration ein.** Migrationen
  beschreiben Schema, nicht Zustand. Eine Migration, die eine
  Standardzeile einfügt, läuft einmal auf der Produktionsdatenbank
  und dann nie wieder - in dem Moment, in dem sich eine Spalte
  ändert, haben Sie eine gespaltene Quelle der Wahrheit zwischen der
  Migrationshistorie und dem Seeder. Setzen Sie den Insert in einen
  Seeder; braucht die Produktion die Zeile, führen Sie
  `console db:seed --class=DefaultsSeeder` als Teil des Deploys aus.
- **Schreiben Sie keine Fixture-Daten von Hand in Ihren Test.**
  Greifen Sie zu einer Factory. Fünf `User::create(attrs!{ … })`-
  Blöcke in einem Test sind fünf Umschreibungen in dem Moment, in dem
  Sie eine NOT-NULL-Spalte hinzufügen. Ein
  `UserFactory::new().create()` übersteht das.
- **Legen Sie keine Produktionsdaten in einen Seeder.** Ein Seeder
  ist für die Zeilen da, die die Anwendung zum Funktionieren braucht,
  nicht für „hier sind die 8.000 historischen Datensätze, die wir
  importieren.“ Importe sind Einmal-Skripte (schreiben Sie dafür ein
  `#[command]`; siehe [Konsole](console.md)).

### Warum Suprnova abweicht

Laravel liefert eine Klasse `DatabaseSeeder` mit einem
Sonderfall-Helfer `call($seeders)`, den Eloquents Seeder-Loader
erkennt. Suprnova tut das nicht - die Registry ist eine flache
`IndexMap`, jeder Seeder ist ein Peer, und ein zusammengesetzter
Seeder ruft `seed::run_one(name)` auf (oder ruft die Unter-Factories
einfach direkt auf), um zu verketten.

Der Grund ist derselbe Trade-off, den man anderswo in Suprnova sieht:
Eine einzige generische Registry mit einer Ordnungsregel ist leichter
zu durchschauen als eine Klassenhierarchie mit einer magischen
Wurzel. Das Laravel-Muster funktioniert, weil PHPs Klassen-Autoloading
und die statische `make()`-Reflection `call([A::class, B::class])`
diese Klassen namentlich finden und instanziieren lassen; in Rust
müssten wir den Nutzer bitten, `dyn Seeder`-Trait-Objekte
herumzureichen, was umständlicher ist als die
Funktionszeiger-Registry, die schon da ist.

Die Konvention des zusammengesetzten Seeders gewinnt dieselbe
Ergonomie zurück - `BaseSeeder` spielt die Rolle, die
`DatabaseSeeder` in Laravel spielt -, ohne dass das Framework einen
Namen als besonders auszeichnen müsste.

## Bootstrap-Registrierung

Jeder Seeder braucht einen Aufruf von `seed::register` in
`bootstrap.rs`, neben der übrigen prozessglobalen Verdrahtung
(Config, Observer, Supervisoren, Queue-Jobs). Das Muster hat dieselbe
Form, die anderswo in der Bootstrap-Datei verwendet wird:

```rust
// src/bootstrap.rs
pub async fn register() {
    // …Config + Container-Bindings + Auth-Verdrahtung…

    // Seeder. Reihenfolge zählt - run_all besucht sie in
    // Registrierungsreihenfolge.
    suprnova::seed::register::<crate::seeders::BaseSeeder>();
    suprnova::seed::register::<crate::seeders::DemoContentSeeder>();

    // …Observer, Supervisoren, Queue-Jobs…
}
```

Vergessen Sie, einen Seeder zu registrieren, schlägt
`console db:seed --class=X` fehl mit „no seeder registered for `X`“ -
ein klares Signal statt eines stillen Überspringens. Die Helfer
`seed::count()` und `seed::is_registered("…")` existieren genau
dafür, damit ein Test assertieren kann, dass bootstrap jeden erwarteten
Seeder registriert hat.

Siehe [Bootstrap](bootstrap.md) für die vollständige Struktur der
Datei und die Reihenfolge, in der das Framework erwartet, dass jedes
Subsystem verdrahtet wird.

## Nächste Schritte

- [Migrationen](migrations.md) - die Schema-Hälfte des
  Seed/Migrate-Paars
- [Eloquent](eloquent.md) - Models, Factories und die
  `Persistable`-Maschinerie, die jeder Seeder aufruft
- [Konsole](console.md) - die projektspezifische `console`-Binary,
  die `db:seed` neben Ihren eigenen `#[command]`s hostet
- [Testen](testing.md) - `TestContainer`, `TestDatabase::fresh` und
  das `#[serial]`-Muster für Tests, die die Seeder-Registry berühren
- [Fehlermodell](error-model.md) - was `FrameworkError` ist und wie
  die `Result<(), _>`-Form von `run` mit dem Rest des Frameworks
  komponiert
