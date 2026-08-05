# Eloquent Factories

Factories erzeugen randomisierte Modellinstanzen für Tests und
Seeder. Die Form ist die von Laravel:
`UserFactory::new().count(10).create_many().await?`. Der Vertrag ist
ein Trait plus ein Fluent Builder, mit einer Abkürzung
`#[derive(Factory)]` für den häufigen Fall, in dem das Modell
bereits eine sinnvolle randomisierte Repräsentation hat.

Dieses Kapitel behandelt das Definieren von Factories von Hand und
per Derive, das Komponieren von Overrides zu wiederverwendbaren
„States“, deterministische IDs über `Sequence`, die Nahtstelle
`Persistable`, die `create` antreibt, und den Unterschied zwischen
`make` (im Speicher) und `create` (persistiert). Für den
Test-Schreib-Kontext, in dem Factories am nützlichsten sind, siehe
[Testen](testing.md).

## Der Trait `Factory`

Der Trait hat genau eine erforderliche Methode:

```rust
pub trait Factory {
    type Model;

    fn definition() -> Self::Model
    where
        Self: Sized;
}
```

`definition()` gibt ein vollständig befülltes Modell zurück, bei dem
jedes Feld auf einen sinnvollen Standard randomisiert ist. Der Trait
trägt keinen Pro-Instanz-Zustand - Implementierer sind typischerweise
größenlose Marker (`struct UserFactory;`), sodass ein Aufrufer die
Factory über ihren Namen erreichen kann, ohne ein Handle zu halten.

Der Trait stellt außerdem zwei Builder-Einstiegspunkte mit
Standardimplementierungen bereit:

```rust
fn new() -> FactoryBuilder<Self::Model>;       // count = 1, keine Overrides
fn times(n: usize) -> FactoryBuilder<Self::Model>;  // Zucker für new().count(n)
```

Jede andere Methode, die Sie aufrufen werden (`with`, `count`,
`make`, `create`, `create_many`, …), lebt auf `FactoryBuilder<M>`.

## Eine Factory von Hand definieren

Die minimale handgeschriebene Form paart eine Marker-Struktur mit
einer `Factory`-Implementierung, die weiß, wie eine Instanz gebaut
wird. Dazu greifen Sie typischerweise, wenn das Modell nicht
`fake::Dummy` ableitet - vielleicht weil manche Felder
deterministische Seeds brauchen (Relations-IDs in einem bekannten
Bereich) oder die randomisierte Repräsentation
Geschäftsregel-Bewusstsein braucht:

```rust
use suprnova::Factory;
use crate::models::users::User;

pub struct UserFactory;

impl Factory for UserFactory {
    type Model = User;

    fn definition() -> User {
        let now = chrono::Utc::now();
        User {
            // `0` ist ein Platzhalter - `persist_via_seaorm` klappt
            // Primärschlüssel-Spalten vor dem Insert auf `NotSet`
            // um, sodass die Datenbank die echte id zuweist.
            id: 0,
            name: format!("Factory User #{}", next_seq()),
            email: format!("factory-{}@example.test", next_seq()),
            password: "factory-placeholder".into(),
            remember_token: None,
            active: true,
            created_at: now,
            updated_at: now,
            deleted_at: None,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

Die Felder `__eager` und `__pivot` sind der Eager-Load- und
Pivot-Hilfszustand, den das Makro `#[suprnova::model]` in jede
Eloquent-Struktur einfügt. Setzen Sie sie immer auf ihren
Standardwert - sie werden vom Query Builder befüllt, nicht von
Factories.

`next_seq()` ist, was immer Sie wollen - ein `static AtomicU64`, eine
`Sequence` (unten behandelt), oder ein Thread-lokaler Zähler. Der
Punkt ist, dass `definition()` bei jedem Aufruf innerhalb von
`make_many` / `create_many` frisch läuft, sodass jede benötigte
Eindeutigkeit von einem Zähler kommen muss, den die Funktion
erreichen kann.

## `#[derive(Factory)]` für den häufigen Fall

Wenn das Modell selbst `fake::Dummy` implementiert - entweder über
`#[derive(Dummy)]` oder ein handgeschriebenes `impl Dummy<Faker> for
Model` - kollabiert das Derive Marker + Implementierung zu einer
Zeile auf dem Modell:

```rust
use suprnova::{Dummy, Factory};

#[derive(Dummy, Factory)]
pub struct Post {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub author_id: i64,
    pub is_public: bool,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}
```

Das Derive gibt `pub struct PostFactory;` als Schwestertyp aus, plus
ein `impl Factory for PostFactory`, dessen `definition()`
`Faker.fake::<Post>()` aufruft. Die Sichtbarkeit der Factory
spiegelt die Sichtbarkeit des Modells - ein `pub`-Modell bekommt eine
`pub`-Factory, ein `pub(crate)`-Modell bekommt eine
`pub(crate)`-Factory.

### Den generierten Namen überschreiben

Standardmäßig gibt `#[derive(Factory)]` `<Model>Factory` aus.
Überschreiben Sie das über das Attribut `name`:

```rust
#[derive(Dummy, Factory)]
#[factory(name = "AccountFactory")]
pub struct User { /* … */ }
```

Der Wert muss als Rust-Identifier parsen - `name = "User Factory"`
oder `name = "user-factory"` scheitert am Compile mit einem klaren,
span-verweisenden Fehler. Das Makro gibt `pub struct <Name>;`
wörtlich aus, also kann alles, was kein Typname sein kann, auch kein
Factory-Name sein.

### Handgerolltes `Dummy` für reichhaltigere Randomisierung

`#[derive(Dummy)]` funktioniert für primitiv-typisierte Strukturen,
gibt Ihnen aber keine Kontrolle über Verteilungen oder
Feldübergreifende Invarianten. Schreiben Sie für alles Nichttriviale
die `Dummy`-Implementierung von Hand und paaren Sie sie mit
`#[derive(Factory)]`:

```rust
use suprnova::__fake::rand::Rng;
use suprnova::__fake::{Dummy, Fake, Faker, faker::lorem::en::{Paragraph, Sentence}};
use suprnova::Factory;

#[derive(Factory)]
pub struct Post { /* Felder … */ }

impl Dummy<Faker> for Post {
    fn dummy_with_rng<R: Rng + ?Sized>(_: &Faker, rng: &mut R) -> Self {
        let title: String = Sentence(3..7).fake_with_rng(rng);
        let body: String = Paragraph(3..6).fake_with_rng(rng);
        let author_id: i64 = (1..=50i64).fake_with_rng(rng);
        let now = chrono::Utc::now();

        Post {
            id: 0,
            author_id,
            title,
            body,
            is_public: Faker.fake_with_rng::<bool, _>(rng),
            created_at: now,
            updated_at: now,
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

Der Crate `fake` wird als `suprnova::__fake` re-exportiert, damit
Konsumenten keine eigene `fake = "…"`-Zeile in der `Cargo.toml`
brauchen. Gängige Typen werden außerdem unter der Crate-Wurzel
re-exportiert: `suprnova::{Dummy, Fake, Faker}`.

### Warum `#[derive(Factory)]` nur einfache Strukturen nimmt

Das Derive lehnt Enums, Unions und generische Modelle mit einem
klaren Compile-Fehler ab. Enums und Unions haben keine sinnvolle
Standard-Repräsentation. Generics würden eine Entscheidung darüber
erzwingen, wie der Factory-Typ sein Modell parametrisiert - und es
gibt keinen guten Standard, also weigert sich das Derive zu raten.
Schreiben Sie für diese Fälle `impl Factory` von Hand.

## Der Fluent Builder

`Factory::new()` / `Factory::times(n)` geben einen
`FactoryBuilder<M>` zurück. Jede Operation ist verkettbar; nichts
passiert, bis Sie eine terminale Methode aufrufen (`make`,
`make_one`, `make_many`, `create`, `create_one`, `create_many`).

### `count(n)` - wie viele Instanzen

```rust
let user = UserFactory::new().make();             // 1 User
let users = UserFactory::new().count(10).make_many();  // 10 User
let same = UserFactory::times(10).make_many();   // identisch
```

`count(n)` wird von `make` / `create` ignoriert (immer eine) und von
`make_many` / `create_many` respektiert. `times(n)` ist nur Zucker
für `Self::new().count(n)` und entspricht Laravels
`Factory::times($n)`.

### `with(|m| { … })` - Pro-Aufruf-Overrides

`with` registriert eine Closure, die gegen jede erzeugte Instanz
nach `definition()` läuft. Mehrere `with`-Aufrufe komponieren sich
in Registrierungsreihenfolge, sodass ein späteres Override ein
früheres auf demselben Feld überschreibt:

```rust
let admin = UserFactory::new()
    .with(|u| u.active = true)
    .with(|u| u.role = "admin".into())
    .make();
```

Overrides werden als `Box<dyn Fn(&mut M) + Send + Sync + 'static>`
gespeichert, damit der Builder `Send` bleibt - wichtig für die
asynchronen `create`- / `create_many`-Pfade, die den Builder über ein
`.await` auf dem SeaORM-Insert hinweg halten.

### `prepend(|m| { … })` - Standardwerte, die Aufrufer noch überschreiben können

`prepend` fügt eine Closure am **Anfang** der Override-Kette ein,
sodass sie **vor** jedem anderen `with(...)` läuft. Verwenden Sie es
innerhalb einer State-Methode, wenn Sie einen Standard bereitstellen
wollen, den der Aufrufer noch mit einem späteren `.with(...)`
überschreiben kann:

```rust
impl UserFactory {
    /// State-Methode - Admin-Standardwerte, Aufrufer kann trotzdem anpassen.
    pub fn admin() -> suprnova::FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.active = true)
    }
}

// Der Aufrufer gewinnt bei `role`, weil sein .with() nach den prepends kommt.
let owner = UserFactory::admin()
    .with(|u| u.role = "owner".into())
    .make();
```

Das ist Suprnovas Äquivalent zu Laravels `Factory::prependState`. Es
ist die richtige Primitive speziell für State-Methoden - `with`
würde gegen ein `.with(...)` des Aufrufers verlieren, was das
Gegenteil von dem ist, was ein Standard tun soll.

### `when(cond, |b| { … })` - bedingtes Verketten

`when` fädelt ein Flag durch eine Kette, ohne den fluenten Stil zu
brechen. Die Closure erhält den Builder, gibt den Builder zurück.
Wenn `cond` false ist, läuft der Builder unverändert durch:

```rust
UserFactory::times(10)
    .with(|u| u.active = true)
    .when(seed_admins, |b| b.with(|u| u.role = "admin".into()))
    .create_many()
    .await?;
```

Spiegelt Laravels `Conditionable::when($cond, $cb)`. Die Signatur
`FnOnce(Self) -> Self` bedeutet, Sie können innerhalb der Closure
`await`en, solange Sie vor der Rückgabe des Builders `.await`en.

### Terminale Methoden

| Methode | Gibt zurück | Persistiert? |
|---|---|---|
| `make()` | ein `M` | nein |
| `make_one()` | ein `M` (erzwingt count = 1) | nein |
| `make_many()` | `Vec<M>` mit `count` Elementen | nein |
| `create()` | `Result<M, FrameworkError>` | ja |
| `create_one()` | `Result<M, FrameworkError>` (erzwingt count = 1) | ja |
| `create_many()` | `Result<Vec<M>, FrameworkError>` | ja |

`make_one` und `create_one` sind nützlich, wenn eine State-Methode
`count` intern auf etwas anderes gesetzt hat und der Aufrufer genau
ein Ergebnis will:

```rust
pub fn admins_in_org(org_id: i64) -> suprnova::FactoryBuilder<User> {
    UserFactory::times(5)               // sinnvoller Standard für Fixtures
        .with(move |u| u.org_id = org_id)
        .with(|u| u.role = "admin".into())
}

// Der Test will nur einen - `create_one` verwirft das count(5).
let admin = admins_in_org(42).create_one().await?;
```

## States: wiederverwendbare Preset-Kombinationen

Suprnova liefert keine `state("name")`-Lookup-Tabelle. Stattdessen
sind States gewöhnliche Methoden auf Ihrem Factory-Marker, die einen
vorkonfigurierten `FactoryBuilder<M>` zurückgeben. Das Muster
komponiert sich durch Vererbung - jede State-Methode gibt denselben
Typ `FactoryBuilder<M>` zurück, sodass Sie weitere Methoden an das
Ergebnis anhängen können:

```rust
use suprnova::FactoryBuilder;
use crate::models::users::User;

pub struct UserFactory;

impl suprnova::Factory for UserFactory {
    type Model = User;
    fn definition() -> User { /* … */ }
}

impl UserFactory {
    /// Inaktive Variante - überlagert einen Standard `active: false`.
    pub fn inactive() -> FactoryBuilder<User> {
        Self::new().prepend(|u| u.active = false)
    }

    /// Admin-Variante - überlagert Rolle + verifizierte E-Mail.
    pub fn admin() -> FactoryBuilder<User> {
        Self::new()
            .prepend(|u| u.role = "admin".into())
            .prepend(|u| u.email_verified_at = Some(chrono::Utc::now()))
    }

    /// Komponierbar: inaktiver Admin.
    pub fn inactive_admin() -> FactoryBuilder<User> {
        Self::admin().prepend(|u| u.active = false)
    }
}
```

```rust
// Auch an der Aufrufstelle komponieren - hängen Sie frei weitere Overrides an.
let user = UserFactory::admin()
    .with(|u| u.name = "Alice".into())
    .create()
    .await?;

let batch = UserFactory::inactive().count(20).create_many().await?;
```

Die Entscheidung für `prepend` ist bewusst: Die Overrides eines
States sind *Standardwerte*, die der Aufrufer noch umschreiben kann.
Wenn Sie wollen, dass die Einstellung eines States nicht verhandelbar
ist, verwenden Sie stattdessen `with` - das geht ans Ende der Kette
und gewinnt.

### Warum kein `state("name")`-Lookup

Eine namensgeschlüsselte State-Registry würde Laufzeit-String-Matching
für etwas erzwingen, das der Compiler prüfen kann. State-Methoden
geben Ihnen Compile-Zeit-Verifikation (der Tippfehler
`UserFactor::admn()` ist ein harter Fehler) und vollständige
IDE-Autovervollständigung. Die Komponierbarkeit - `Self::admin()` von
innerhalb `inactive_admin()` zu verketten - fällt kostenlos ab.

## Deterministische IDs mit `Sequence`

`Sequence` ist ein monotoner Zähler zum Seeden von
eindeutigen-pro-Aufruf-Feldern. Jeder `next()`-Aufruf gibt 1, 2, 3, …
zurück, atomar über Threads hinweg:

```rust
use suprnova::{Fake, Sequence};

static ORDER_IDS: Sequence = Sequence::new();

pub struct OrderFactory;
impl suprnova::Factory for OrderFactory {
    type Model = Order;
    fn definition() -> Order {
        Order {
            id: 0,
            number: format!("ORD-{:06}", ORDER_IDS.next()),
            total_cents: (100..=10_000).fake(),
            created_at: chrono::Utc::now(),
            __eager: Default::default(),
            __pivot: None,
        }
    }
}
```

`Sequence::new()` ist `const`, funktioniert also als
`static`-Initialisierer. Der Zähler startet bei 0 und erhöht sich
beim ersten Aufruf auf 1. Verwenden Sie `reset()` zwischen Tests,
wenn Sie einen sauberen Zählerstand wollen - das Makro
`#[suprnova_test]` tut das nicht für Sie, weil das Framework nicht
wissen kann, welche Sequences Ihnen gehören:

```rust
#[suprnova::suprnova_test]
async fn each_order_gets_a_unique_number(db: TestDatabase) {
    ORDER_IDS.reset();   // für diesen Test bei 1 starten
    let orders = OrderFactory::new().count(5).create_many().await?;
    assert_eq!(orders[0].number, "ORD-000001");
    assert_eq!(orders[4].number, "ORD-000005");
}
```

`Sequence` verwendet `SeqCst`-Ordering - Overkill für „gib mir eine
eindeutige id“, hält die Argumentation aber trivial. Sollte eine
Sequence jemals in einem Hot-Path auftauchen, können Sie sich eine
eigene mit `Relaxed` schreiben.

## `Persistable`: die Nahtstelle zu Ihrem Speicher

Die `create`-Methodenfamilie ist verfügbar, wann immer das Modell
`Persistable` implementiert:

```rust
#[async_trait]
pub trait Persistable: Sized + Send {
    async fn persist(self) -> Result<Self, FrameworkError>;
}
```

Eine Blanket-Implementierung in `factory::persist` deckt jedes
SeaORM-Modell ab, das `IntoActiveModel<ActiveModel>` kann - also
jedes Modell, das das Makro `#[suprnova::model]` generiert. Keine
Pro-Modell-Boilerplate; wenn `User` ein Modell ist, funktioniert
`UserFactory::new().create()`.

Die Blanket-Implementierung zieht `DB::connection()` und fügt ein.
Das zurückgegebene `Self` ist, was SeaORM vom Insert zurückgibt -
zugewiesene id, aufgelöste Standard-Spalten usw.

### Umgang mit dem Primärschlüssel

Eine `IntoActiveModel`-Implementierung von SeaORM markiert jedes
Feld - einschließlich des Primärschlüssels - als `Set(value)`. Für
factory-erzeugte Modelle ist der Primärschlüssel ein Platzhalter
(`0` für `AUTO_INCREMENT i64`), sodass ein direktes Insert beim
zweiten Aufruf mit einem UNIQUE-Constraint-Fehler kollidiert.

`persist_via_seaorm` (der Helfer, der die Blanket-Implementierung
trägt) klappt vor dem Insert jede Primärschlüssel-Spalte auf
`NotSet` um, was die Datenbank ihre eigene id zuweisen lässt - die
Semantik, die Factories tatsächlich brauchen:

```rust
pub async fn persist_via_seaorm<M, E, C>(model: M, db: &C) -> Result<M, FrameworkError>
where
    M: ModelTrait<Entity = E> + IntoActiveModel<<E as EntityTrait>::ActiveModel> + Send,
    E: EntityTrait<Model = M>,
    /* … Bounds … */
    C: ConnectionTrait,
{
    let mut active = model.into_active_model();
    for pk in <<E as EntityTrait>::PrimaryKey as Iterable>::iter() {
        active.not_set(pk.into_column());
    }
    active.insert(db).await.map_err(/* … */)
}
```

Wenn Sie tatsächlich eine *bestimmte* id zuweisen wollen (Replay-Test,
Wiederherstellen einer Fixture per id), umgehen Sie den Helfer und
rufen Sie direkt `model.into_active_model().insert(db).await` auf.

### Persistieren gegen eine explizite Connection

`persist_via_seaorm` nimmt die Connection als Argument. Nützlich,
wenn Sie die Persistenz gegen eine Connection treiben wollen, die
nicht die gebundene `DB::connection()` des Frameworks ist - meist ein
bestimmtes `sqlite::memory:`-Handle in einem Integrationstest:

```rust
use suprnova::factory::persist_via_seaorm;

let model = UserFactory::new().make();
let row = persist_via_seaorm(model, db.inner()).await?;
```

### Benutzerdefinierte Nicht-SeaORM-Backends

Weil die Blanket-Implementierung jeden `ModelTrait`-Typ trifft,
können Sie aus einem nachgelagerten Crate kein `impl Persistable for
MyOrm::Model` schreiben, ohne zu kollidieren. Für
Nicht-SeaORM-eigene Persistenz (Redis, Surreal, reine
Blob-Speicher), wickeln Sie das Modell in einen Newtype und
implementieren Sie `Persistable` auf dem Wrapper:

```rust
use suprnova::{FrameworkError, Persistable};
use suprnova::async_trait;

pub struct RedisCached<T>(pub T);

#[async_trait]
impl Persistable for RedisCached<MyValue> {
    async fn persist(self) -> Result<Self, FrameworkError> {
        let client = suprnova::App::make::<RedisClient>()
            .ok_or_else(|| FrameworkError::internal("redis client not bound"))?;
        client.set(&self.0.key, &serde_json::to_vec(&self.0)?).await?;
        Ok(self)
    }
}
```

Ein `Factory<Model = RedisCached<MyValue>>` bekommt dann `create` /
`create_many` kostenlos dazu.

## `make` vs `create`: wann welches verwenden

`make` gibt das Modell zurück, ohne die Datenbank zu berühren:

```rust
// Unit-Test für eine reine Funktion - keine DB nötig.
let draft = PostFactory::new().with(|p| p.is_public = false).make();
let snippet = my_lib::extract_summary(&draft);
assert!(snippet.len() < 200);
```

`create` persistiert und gibt die Post-Insert-Version zurück:

```rust
// Integrationstest - die Aktion braucht eine echte Zeile.
let post = PostFactory::new().create().await?;
let action = App::resolve::<PublishPostAction>().unwrap();
let published = action.execute(post.id).await?;
assert!(published.is_public);
```

Greifen Sie zu `make`, wann immer dem Test egal ist, dass die Zeile
existiert. Greifen Sie zu `create`, wenn Sie die Zeile
zurückabfragen werden, wenn ein Fremdschlüssel eine echte id
braucht, oder wenn Sie Fixtures für ein Subsystem befüllen, das die
DB liest. Beachten Sie, dass `create_many` sequenziell persistiert -
wenn ein späteres Insert fehlschlägt, werden die vorherigen Inserts
NICHT zurückgerollt. `create` / `create_many` laufen durch die
`Persistable`-Blanket-Implementierung, die direkt mit der gebundenen
`DB::connection()` des Frameworks spricht - sie treten **nicht**
einem ambienten `DB::transaction(...)`-Scope bei. Wenn Sie
Atomizität über einen Batch von Inserts brauchen, wechseln Sie
innerhalb der Closure zur Methode `Model::create(attrs!{...})` des
Trait `Model` (dieser Pfad läuft durch denselben Executor, der
`CURRENT_TX` respektiert):

```rust
use suprnova::{DB, Model, attrs};

DB::transaction(|_tx| Box::pin(async move {
    for i in 0..50 {
        User::create(attrs!{
            name: format!("user-{i}"),
            email: format!("user-{i}@example.test"),
        }).await?;
    }
    Ok::<_, suprnova::FrameworkError>(())
})).await?;
```

## Verhalten „nach dem Erstellen“

Suprnova liefert keinen benannten Callback `after_creating(|m| { …
})`. Zwei Muster decken die Anwendungsfälle ab, für die dieser
Callback in Laravel existiert:

**1. Die Kette - die Nacharbeit nach `create`/`create_many` erledigen:**

```rust
let user = UserFactory::new().create().await?;
ProfileFactory::new()
    .with(move |p| p.user_id = user.id)
    .create()
    .await?;
```

Das ist das kanonische Muster, wenn die id eines Modells in einen
nachfolgenden Insert fließen muss. `create` gibt die persistierte
Zeile zurück, sodass die id sofort verfügbar ist.

**2. Model-Observer - auf den Modell-Lifecycle reagieren, nicht auf
die Factory:**

Verwenden Sie [Model-Observer](eloquent.md#observers), um
Post-Insert-Verhalten an das Modell selbst zu verdrahten statt an
die Factory. Der Observer feuert für `User::create(...)`,
`UserFactory::new().create()`, und jeden anderen
Persistenz-Pfad - genau das, was Sie wollen, wenn das Verhalten
lautet „jedes Mal, wenn diese Zeile landet, tue X“:

```rust
use suprnova::{FrameworkError, Observer, async_trait, observer};

#[observer(User)]
pub struct AuditUser;

#[async_trait]
impl Observer<User> for AuditUser {
    async fn created(&self, user: &User) -> Result<(), FrameworkError> {
        tracing::info!(user_id = user.id, "user created");
        Ok(())
    }
}
```

Factory-eigene Callbacks würden Divergenz zwischen Test-Inserts und
echten Inserts einladen. Observer bleiben über beide hinweg
konsistent.

## Seeder

Factories erzeugen Instanzen; Seeder orchestrieren sie. Ein `Seeder`
ist ein größenloser Typ mit einem asynchronen `run`, der weiß, was
zu befüllen ist:

```rust
use suprnova::{Factory, FrameworkError, Seeder};
use suprnova::async_trait;

use crate::factories::{PostFactory, UserFactory};

pub struct BaseSeeder;

#[async_trait]
impl Seeder for BaseSeeder {
    fn name() -> &'static str { "BaseSeeder" }

    async fn run() -> Result<(), FrameworkError> {
        // Zuerst User - Posts referenzieren User-IDs in 1..=50.
        UserFactory::new().count(50).create_many().await?;
        PostFactory::new().count(200).create_many().await?;
        Ok(())
    }
}
```

Registrieren Sie den Seeder in `bootstrap.rs`, damit der Befehl
`db:seed` der `console`-Binary des Projekts davon weiß:

```rust
suprnova::seed::register::<crate::seeders::BaseSeeder>();
```

Ausführen über die `console`-Binary des Projekts (jede
gescaffoldete App liefert eine unter `src/bin/console.rs`):

```bash
cargo run --bin console -- db:seed
```

Seeder laufen in Registrierungsreihenfolge. Idempotenz ist die
Verantwortung des Seeders - `run` erstellt keinen Snapshot und
rollt nichts zurück, sodass ein Seeder, der bedingungslos einfügt,
bei erneutem Lauf Duplikate erzeugt. Verwenden Sie `migrate:fresh`
gefolgt von `db:seed` für einen sauberen Neuanfang.

## Alles zusammenführen: eine vollständige Test-Fixture

```rust
use suprnova::{App, describe, test, expect};
use suprnova::events::{EventFacade, assert_dispatched_times};
use suprnova::testing::TestDatabase;
use crate::factories::{PostFactory, UserFactory};
use crate::actions::publish_post::PublishPostAction;

describe!("PublishPostAction", {
    test!("publishes a draft post", async fn(db: TestDatabase) {
        // Arrange - ein Autor und ein Entwurfs-Post, der ihm gehört.
        let author = UserFactory::new()
            .with(|u| u.active = true)
            .create()
            .await
            .unwrap();

        let draft = PostFactory::new()
            .with(move |p| p.author_id = author.id)
            .with(|p| p.is_public = false)
            .create()
            .await
            .unwrap();

        // Act.
        let action = App::resolve::<PublishPostAction>().unwrap();
        let published = action.execute(draft.id).await.unwrap();

        // Assert.
        expect!(published.is_public).to_equal(true);
        expect!(published.author_id).to_equal(author.id);
    });

    test!("publishing emits exactly one event", async fn(db: TestDatabase) {
        let _guard = EventFacade::fake();
        let post = PostFactory::new().create().await.unwrap();

        App::resolve::<PublishPostAction>().unwrap()
            .execute(post.id).await.unwrap();

        assert_dispatched_times::<crate::events::PostPublished>(1);
    });
});
```

Drei Muster sind es wert, hervorgehoben zu werden:

- Die `id` des Autors fließt über eine `move`-Closure innerhalb von
  `.with(...)` in den Post. Captures sind explizit, was die
  Relation an der Aufrufstelle sichtbar hält.
- `create().await.unwrap()` ist das Test-Idiom - der Test darf beim
  Setup-Fehlschlag panicken, weil eine kaputte Fixture ein kaputter
  Test ist und keinen kontrollierten Fehlerpfad braucht.
- Factories komponieren mit dem Rest der Test-Oberfläche
  (`EventFacade::fake`, `Storage::fake`, `Mail::fake`, …) - keiner
  der Fakes weiß etwas von Factories, aber jeder Test, den Sie
  schreiben, wird sie zusammen verwenden.

### Warum Suprnova abweicht

Laravels Factories liefern benannte States (`->state('admin')`),
Laufzeit-Sequenzen (`->sequence(['name' => 'A'], ['name' => 'B'])`)
und einen `afterCreating`-Callback, der auf der Factory selbst
registriert wird. Suprnova lässt alle drei weg und ersetzt sie durch
Rust-förmige Primitiven:

- **States sind Methoden, keine Strings.** Compile-Zeit-Tippfehlerprüfung
  und IDE-Autovervollständigung sind beide kostenlos; die einzigen
  Kosten sind „Sie schreiben `pub fn admin()` statt `protected
  function admin()`“, was gar keine Kosten sind.
- **Sequences sind eine eigenständige Primitive.** `Sequence` tut
  eine Sache (atomarer Zähler) und ist außerhalb der
  Factory-Oberfläche wiederverwendbar - Sie können eine in einen
  Request-ID-Generator, einen Workflow-Schritt-Zähler oder ein
  Test-Harness fallen lassen, ohne zu erklären, was sie ist.
- **After-Creating ist an das Modell verdrahtet, nicht an die
  Factory.** Das Framework hat für genau diesen Zweck schon
  [Model-Observer](eloquent.md#observers). Einen parallelen Mechanismus auf
  der Factory hinzuzufügen würde Test-Zeit-Verhalten und
  Produktions-Zeit-Verhalten konstruktionsbedingt divergieren
  lassen.

Die fluente Oberfläche - `count(10)`, `times(10)`, `with`, `prepend`,
`when`, `make`, `create`, `create_many`, `make_one`, `create_one` -
spiegelt Laravel direkt, sodass das Muskelgedächtnis ohne Glossar
überträgt.

## Nächste Schritte

- [Testen](testing.md) - `#[suprnova_test]`, `TestDatabase`, die
  Fake-Facades, die mit factory-gebauten Fixtures zusammenspielen.
- [Eloquent API](eloquent.md) - Modell-Ableitung, Observer, die
  Cast-Pipeline, die läuft, wenn `create` Ihre Factory-Ausgabe
  persistiert.
- [Migrationen](migrations.md) - das Schema, gegen das Ihre
  Factories existieren müssen; verwenden Sie `migrate:fresh &&
  db:seed` für einen sauberen Fixture-Neuanfang.
- [Datenbank](database.md) - `DB::transaction`,
  Multi-Connection-Routing, Savepoints - worauf Sie zurückgreifen,
  wenn `create_many` Atomizität braucht.
- [Service Container](container.md) - wie `App::resolve` und
  `App::make` die Action- und Service-Typen finden, die Ihre Tests
  neben Factories aufrufen.
