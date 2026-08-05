# Datenbank-Tests

Das DB-spezifische Gegenstück zu [Testen](testing.md). Wo jenes
Kapitel den Test-Harness abdeckt - `#[suprnova_test]`, `describe!` /
`test!`, `expect!` und die In-Process-Fakes - deckt dieses ab, was
sich ändert, wenn Ihr Test eine Datenbank braucht: wie
`TestDatabase` eine für Sie baut, wie Isolation tatsächlich
funktioniert, wo Factories und Seeder anschließen, und wann ein
In-Memory-SQLite reicht und wann nicht.

## Die zwei Konstruktoren

Jeder Datenbank-Test beginnt damit, eine `TestDatabase` zu bauen. Zwei
Konstruktoren, zwei Absichten.

### `TestDatabase::fresh::<Migrator>()`

Baut eine In-Memory-SQLite-Datenbank, führt Ihren Migrator
end-to-end aus und registriert die Connection im Test-Container,
sodass jeder Code, der `DB::connection()` oder
`App::resolve::<DbConnection>()` aufruft, sie auflöst. Das ist der
richtige Standard für alles, was echtes Schema berührt.

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn user_lifecycle_end_to_end() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);
    // Direkt abfragen, wenn Sie die Model-Oberfläche umgehen wollen:
    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`Migrator` ist die `MigratorTrait`-Implementierung Ihrer Anwendung -
derselbe Typ, den der Produktionsbefehl `suprnova migrate` ausführt.
Indem Sie den echten Migrator durch das Test-Schema fädeln, machen
Sie Schema-Drift unmöglich: Eine Spalte, die der Migrator zu ergänzen
vergessen hat, kann in der Test-DB nicht stillschweigend vorhanden
sein.

Das Makro `test_database!()` ist Zucker für den häufigen Fall
(`crate::migrations::Migrator`):

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();          // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}

// Oder mit einem eigenen Migrator-Pfad:
let db = test_database!(my_crate::CustomMigrator);
```

### `TestDatabase::sqlite_memory()`

Dieselbe Container- und Registry-Verdrahtung, führt aber **keinen
Migrator aus**. Verwenden Sie dies, wenn der Test präzise Kontrolle
über die Spaltenform will - typischerweise Cast-Round-Trips, Tests
der SQL-Oberfläche des Query Builders oder treiberspezifische
Grenzfälle, bei denen ein vollständiger Migrator zu viel oder nur
Rauschen wäre:

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared(
    "CREATE TABLE casts_t (id INTEGER PRIMARY KEY, payload BLOB)",
)
.await
.unwrap();

// Dann direkt schreiben und mit den typisierten Helfern zurücklesen:
let row = db.fetch_one(
    "INSERT INTO casts_t (payload) VALUES (?) RETURNING id, payload",
    vec![sea_orm::Value::Bytes(Some(Box::new(b"hello".to_vec())))],
).await.unwrap();
```

`sqlite_memory()` ist das Fundament, auf dem `fresh()` aufbaut -
`fresh` ruft es auf und führt dann Ihren Migrator aus. Alles, was Sie
mit `fresh` tun können, können Sie auch hier tun; Sie bringen nur Ihr
eigenes DDL mit.

### `execute_unprepared`, `fetch_one`, `fetch_all`

`TestDatabase` re-exportiert die drei SeaORM-Ausführungsformen, zu
denen Sie in Tests am häufigsten greifen, damit Testdateien nicht
`ConnectionTrait` importieren müssen:

| Methode | Verwenden für |
| --- | --- |
| `execute_unprepared(sql)` | DDL oder DML ohne Platzhalter. Liefert `Result<(), FrameworkError>` |
| `fetch_one(sql, bindings)` | SELECT mit einer Zeile. Fehler bei null Zeilen |
| `fetch_all(sql, bindings)` | SELECT über alle Zeilen |

Die Bindings sind `Vec<sea_orm::Value>` - dieselbe Form, die der
Produktions-Query-Pfad verwendet. Das Backend der Connection (SQLite
bei beiden Konstruktoren) wird für Sie bereitgestellt, sodass ein
`?`-Platzhalter richtig ist.

## Wie Isolation tatsächlich funktioniert

Das Modell einer frischen Datenbank pro Test ist der
Isolationsmechanismus. Jeder Aufruf von `fresh()` oder
`sqlite_memory()` öffnet eine neue `sqlite::memory:`-Connection, die
unter SQLite eine vollständig separate Datenbankinstanz ist - kein
geteiltes Schema, keine geteilten Zeilen, kein anderer Test kann
hineinsehen. Es gibt keinen Transaktions-Wrapper, keinen
`RefreshDatabase`-Trait, in den man sich einklinken müsste, und kein
Rollback, an das man denken müsste: Der *nächste* Test bekommt eine
saubere, leere DB, weil er sich seine eigene baut.

Wenn der `TestDatabase`-Wert gedroppt wird, passieren drei Dinge, in
dieser Reihenfolge:

1. Der gehaltene `TestContainerGuard` leert den Thread-Local-
   Test-Container, sodass jedes nachfolgende
   `App::get::<DbConnection>()` die Test-Connection nicht mehr
   findet.
2. War dies der *letzte* lebende `TestContainerGuard` im Prozess,
   wird die benannte [`ConnectionRegistry`](database.md#named-connections)
   gelöscht. (Ein Refcount über `FAKE_GUARDS` garantiert, dass der
   Drop eines inneren Tests nicht einen Connection-Namen auslöschen
   kann, von dem ein gleichzeitig laufender äußerer Test noch
   abhängt - die stehende Falle, die den Refcount ausgelöst hat.)
3. Die SQLite-Connection selbst dropt, was die In-Memory-Datenbank
   zerstört.

Weil der Zustand neu aufgebaut statt zurückgerollt wird, ist die
Isolation stärker als `BEGIN`/`ROLLBACK`-Wrapping: Es gibt keinen
committeten Zustand, der versehentlich überlebt, keine
verschachtelten Transaktions-Eigenheiten, keine Sequenzzähler-Drift
zwischen Tests. Der Preis ist, dass Sie dafür zahlen, den Migrator
einmal pro Test auszuführen (bei den meisten Schemas mit SQLite
vernachlässigbar; wird es zu einem echten Kostenfaktor, siehe weiter
unten den Abschnitt zum Teilen einer migrierten Datenbank über Tests
hinweg).

## Warum der Pool auf eine Connection gepinnt ist

Beide Konstruktoren bauen die Datenbank mit `max_connections(1)` und
`min_connections(1)`. Das ist für `sqlite::memory:` tragend, keine
generelle Richtlinie.

`sqlite::memory:` ist eine Connection-pro-Datenbank: Jede *neue*
Connection im Pool wäre eine separate, leere SQLite-Instanz. Ein
Pool der Größe 2 würde bedeuten, dass die Hälfte Ihrer Queries die
migrierte Datenbank sieht und die andere Hälfte eine leere. Das
Pinnen des Pools auf eine Connection sorgt dafür, dass jede Query im
Test auf derselben In-Memory-Datenbank landet, gegen die der Migrator
lief.

Die Konsequenz: Ein Test, der echte Connection-Nebenläufigkeit
ausübt (zwei um die Wette laufende Transaktionen, Read-Replica-
Routing, ein Queue-Worker, der die DB trifft, während ein
Request-Handler das Gleiche tut), braucht eine echte Datenbank. Siehe
weiter unten den Abschnitt darüber, wann SQLite In-Memory nicht
ausreicht.

## Factories in Tests

Factories erzeugen randomisierte Model-Instanzen und persistieren sie
(optional). Der Persistenzpfad löst die gebundene Test-Connection
automatisch auf - es gibt keine Factory-seitige Verdrahtung für
Tests.

```rust
use crate::factories::UserFactory;

#[tokio::test]
async fn factory_round_trip() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // Nur In-Memory: am schnellsten, kein DB-Round-Trip.
    let alice = UserFactory::new()
        .with(|u| u.email = "alice@example.com".into())
        .make();
    assert_eq!(alice.email, "alice@example.com");

    // Eine persistieren + das Post-Insert-Model zurückgeben (id zugewiesen).
    let bob = UserFactory::new().create().await.unwrap();
    assert!(bob.id > 0);

    // Bulk: 50 nacheinander persistieren.
    let many = UserFactory::times(50).create_many().await.unwrap();
    assert_eq!(many.len(), 50);
}
```

Zwei Muster, die man kennen sollte:

**Factory-Inserts umgehen Model-Events.** Die `Persistable`-Impl, die
hinter `create()` / `create_many()` steht, schreibt direkt über
SeaORMs `ActiveModelTrait::insert` - sie geht *nicht* über die
`Model::create`-Oberfläche, die `Creating` / `Created` / `Saving` /
`Saved` auslöst. Ein Test, der assertiert „kein Observer feuert,
während wir die Fixture bauen“, braucht nichts Besonderes; ein Test,
der assertiert „der `Created`-Observer HAT gefeuert“, muss statt einer
Factory `Model::create(...)` (oder `save()`) verwenden.

**`create_many` transagiert nicht.** Inserts laufen sequenziell.
Schlägt eine spätere Zeile fehl, werden die vorherigen Zeilen nicht
zurückgerollt. Wickeln Sie den Aufruf in Ihre eigene
`DB::transaction`, wenn ein Test Atomarität braucht:

```rust
DB::transaction(|tx| async move {
    UserFactory::times(50).create_many().await?;
    PostFactory::times(200).create_many().await?;
    Ok::<_, FrameworkError>(())
}).await.unwrap();
```

Siehe [Eloquent → Factories](eloquent-factories.md) für die
vollständige Factory-Oberfläche (States, Sequenzen,
`with`-Relationen, `count`, `times`, `make_one` / `create_one`).

## Seeder in Tests

Seeder sind Funktionen, die Sie unter einem stabilen Namen in der
Seeder-Registry des Frameworks registriert haben. Zwei Muster, um sie
aus Tests zu treiben, eines für jede Achse der Absicht.

### Einen einzelnen Seeder namentlich ausführen

```rust
use suprnova::seed;
use my_app::seeders::UsersSeeder;

#[tokio::test]
async fn users_seeder_populates_fixtures() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<UsersSeeder>();
    seed::run_one("UsersSeeder").await.unwrap();

    let count = User::query().count().await.unwrap();
    assert!(count > 0);
}
```

### Das vollständige Bootstrap-Seeder-Set ausführen

```rust
use serial_test::serial;
use suprnova::seed;

#[tokio::test]
#[serial]
async fn full_seed_lands_expected_row_counts() {
    seed::clear();                              // von einer bekannt leeren Registry starten
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();

    seed::register::<my_app::seeders::UsersSeeder>();
    seed::register::<my_app::seeders::PostsSeeder>();
    seed::run_all().await.unwrap();

    let users = User::query().count().await.unwrap();
    let posts = Post::query().count().await.unwrap();
    assert_eq!(users, 50);
    assert_eq!(posts, 200);

    seed::clear();
}
```

Zwei wichtige Vertragsdetails:

**Die Seeder-Registry ist prozessglobal.** `seed::register::<S>()`
fügt in eine `RwLock<IndexMap>` ein, die nach `S::name()` schlüsselt.
Ein Test, der die Registry verändert, sollte bei Eintritt
`seed::clear()` aufrufen, die Seeder registrieren, die er braucht,
laufen lassen und bei Austritt erneut `clear()` aufrufen - und der
Test selbst sollte `#[serial_test::serial]` sein, damit sich zwei
parallele Tests nicht um die Registry streiten.
`#[suprnova_test]` registriert Seeder **nicht** automatisch; nur der
explizite Aufruf von `seed::register::<>()` in Ihrer eigenen
`bootstrap.rs` oder im Testkörper legt sie in die Registry.

**Model-getriebene Seeds vs. Factory-getriebene Seeds.** Ein Seeder,
der `User::create(...)` in einer `for`-Schleife durchläuft, löst pro
Zeile `Creating` / `Saving` / `Created` / `Saved` aus und ruft jeden
registrierten Observer auf. Für Bulk-Seeding, bei dem dieser Fanout
unerwünscht ist, wickeln Sie die Schleife in `seed::without_events`:

```rust
seed::without_events(async {
    for i in 0..50 {
        User::create(attrs! { name: format!("user{i}"), email: format!("user{i}@example.com") }).await?;
    }
    Ok::<_, FrameworkError>(())
}).await?;
```

Die Stummschaltung ist **task-scoped** - nur die Arbeit, die
innerhalb des Future ausgeführt wird, wird zum Schweigen gebracht;
gleichzeitige Request-Handler und Queue-Worker feuern Events weiter
normal. Factories (`create_many`) umgehen den Event-Pfad bereits,
sodass `without_events` um sie herum unnötig ist.

Siehe [Seeding](seeding.md) für die Oberfläche zum Schreiben von
Seedern und [Eloquent → Factories](eloquent-factories.md) für die
Beziehung zwischen beiden.

## Parallel-sichere Datenbank-Tests

`cargo test` führt Tests parallel nach Thread aus. Die
Standard-Expansion von `#[suprnova_test]` (die `#[tokio::test]` ist,
also eine `current_thread`-Runtime pro Test) verträgt sich aus zwei
Gründen sicher damit:

- **Jeder Test bekommt seine eigene `sqlite::memory:`-Connection.**
  Tests teilen keinen DB-Zustand.
- **Die gebundene Connection lebt im Thread-Local-`TestContainer`.**
  Tests teilen keine Container-Bindings.

Worüber Sie nicht nachdenken müssen: `DB::connection()`,
`App::resolve`, Factory-Persistenz, Model-Trait-Writes - all das
landet transparent auf der richtigen Pro-Test-Datenbank.

Worüber Sie *doch* nachdenken müssen:

| Oberfläche | Warum prozessglobal | Abhilfe |
| --- | --- | --- |
| `ConnectionRegistry` (`DB::register_named`, `__read_replica__`) | Einzelne, vom Prozess geteilte `RwLock<HashMap>` | `#[serial_test::serial]` für jeden Test, der benannte Connections registriert oder liest |
| Die Seeder-Registry | Einzelne `RwLock<IndexMap>` | `#[serial_test::serial]` + `seed::clear()` bei Eintritt und Austritt |
| Die Eloquent-Observer-/Scope-Registries | Nach `TypeId::<M>()` geschlüsselt | Jeder Test sollte eine eigene Model-Struktur verwenden, oder `#[serial]` sein und den `clear()`-Helfer der Registry aufrufen |
| Das benannte Query-Log (`DB::enable_query_log`) | Einzelner, prozessglobaler Ringpuffer | `#[serial]`, wenn Assertions das Log lesen |

Der Refcount der Connection-Registry macht das ungefährlicher, als es
klingt: Ein Test, der einen `TestContainerGuard` hält, hält die
Registry auch dann lebend, wenn der Guard eines *Geschwister*-Tests
dropt. Sie wollen trotzdem `#[serial]` für die Tests, die die
Registry tatsächlich verändern, damit sich ihre Reads und Writes
nicht verschachteln können.

### Vorbehalt bei der Multi-Thread-Runtime

`#[suprnova_test]` expandiert zu `#[tokio::test]` mit der
Standard-`current_thread`-Runtime, sodass der Thread-Local-
Container-Pfad immer funktioniert. Wenn Sie einen Test explizit auf
die Multi-Thread-Runtime umstellen:

```rust
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_io_test() {
    let _db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // PROBLEM: Mit `tokio::spawn` gespawnte Tasks können auf einem
    // anderen Worker-Thread laufen als dem, der die TestDatabase
    // gebaut hat. Sie sehen das Thread-Local-TestContainer-Binding
    // nicht und DB::connection() liefert den Wert des globalen
    // (Produktions-)Containers oder einen Fehler.
}
```

Zwei Lösungen, je nachdem, was der Test tut:

1. **Direkter Connection-Zugriff** - `db.conn()` liefert weiterhin
   die richtige `&DatabaseConnection`, egal welcher Worker-Thread sie
   liest. Wenn der Test nur über das `db`-Handle mit der DB spricht
   (nicht über `DB::connection()`), ist die Multi-Thread-Runtime in
   Ordnung.

2. **`TestContainer::scope`** - wickeln Sie den Testkörper in
   `TestContainer::scope(async { ... }).await` und binden Sie Ihre
   Fakes (und die DB-Connection) darin. Der Scope bindet den
   Container an die Task-Local-Ebene, die über Awaits hinweg erhalten
   bleibt, selbst wenn die Runtime das Future zwischen Worker-Threads
   hüpfen lässt. Für gespawnte Sub-Tasks verwenden Sie
   `TestContainer::spawn` (nicht das bloße `tokio::spawn`), damit der
   Task-Local-Container erfasst und im gespawnten Future wieder
   installiert wird.

Siehe [Service Container → Lookup-Reihenfolge](container.md) für die
vollständige Schichtung aus Task-Local, Thread-Local und global.

## SQLite In-Memory vs. echtes Postgres / MySQL / MariaDB

`TestDatabase` ist absichtlich SQLite-only. Der Treiber ist fest auf
`sqlite::memory:` verdrahtet; es gibt kein `TestDatabase::postgres()`,
kein `fresh_with_url()` und keine env-gesteuerte Variante. Für den
überwältigenden Großteil der Testoberfläche - Model-CRUD, Form des
Query Builders, Cast-Round-Trips, Laden von Relationen,
Auslösereihenfolge der Observer, Soft-Delete-Semantik - ist SQLite
In-Memory das richtige Werkzeug: null Setup, kein Netzwerk,
Millisekunden pro Test, perfekte Isolation, kein externer Dienst, den
man in der CI am Leben halten muss.

Es gibt vier Fälle, in denen SQLite In-Memory nicht ausreicht:

1. **Treiberspezifisches SQL.** Eine Query, die Postgres' `LATERAL`,
   `JSONB`-Operatoren, `ON CONFLICT ... WHERE`, MySQL-Window-
   Funktionen oder eine andere dialektspezifische Oberfläche
   verwendet, läuft auf SQLite nicht. Der Model+Builder-Pfad
   versucht, generisch zu bleiben, aber ein Raw-SQL-Test, der
   Postgres-förmige Ausgabe erwartet, braucht Postgres.
2. **Nebenläufigkeit unter echter Connection-Kontention.** SQLite
   In-Memory ist Single-Connection (siehe weiter oben „Warum der Pool
   auf eine Connection gepinnt ist“). Tests, die zwei Transaktionen
   um die Wette laufen lassen, Read-Replica-Routing unter Last
   ausüben oder die Wiederholung bei Deadlocks messen, brauchen einen
   Multi-Connection-Server.
3. **Vector-/NoSQL-/temporale Oberflächen.** Suprnovas
   MariaDB-`VECTOR`-Treiber, die Qdrant-Integration, die
   Pinecone-Integration und ähnliche Nicht-SQL-Treiber lassen sich in
   SQLite überhaupt nicht abbilden.
4. **Produktionsparitäts-Smoke-Tests.** Eine Handvoll Tests der Art
   „funktioniert das auch wirklich auf der echten DB, auf die wir
   deployen?“, auf die CI beschränkt, lohnt es sich zu behalten, auch
   wenn die Unit-Test-Schicht SQLite ist.

Für alle vier Fälle ist das Muster dasselbe: Treten Sie vollständig
aus `TestDatabase` heraus, bauen Sie eine `DbConnection` gegen eine
von Betreibern bereitgestellte `DATABASE_URL`-artige Env-Variable,
gaten Sie den Test env-abhängig, sodass er überspringt, wenn die
Variable fehlt, und markieren Sie ihn `#[serial]`, damit sich nicht
zwei von ihnen um die geteilte echte Datenbank streiten. Das Muster
`MARIADB_URL` in `framework/tests/vector_mariadb.rs` ist das
kanonische Beispiel:

```rust
use serial_test::serial;
use suprnova::database::{DatabaseConfig, DbConnection};

async fn maybe_real_db(test_name: &str) -> Option<DbConnection> {
    let url = match std::env::var("POSTGRES_TEST_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("[{test_name}] skipping: POSTGRES_TEST_URL not set");
            return None;
        }
    };
    let config = DatabaseConfig::builder().url(&url).build();
    Some(DbConnection::connect(&config).await.expect("real DB connects"))
}

#[tokio::test]
#[serial]
async fn jsonb_operator_works_against_postgres() {
    let Some(conn) = maybe_real_db("jsonb_operator_works_against_postgres").await else {
        return;
    };
    // Treiben Sie Postgres-spezifisches SQL direkt gegen `conn`.
}
```

Die stehende Konvention: Benennen Sie die Env-Variable nach dem
Ziel-Treiber (`POSTGRES_TEST_URL`, `MYSQL_TEST_URL`, `MARIADB_URL`),
geben Sie eine Skip-Zeile aus, damit ein Entwickler, der die Suite
lokal laufen lässt, sieht, dass der Test übersprungen wurde (nicht
stillschweigend bestanden hat), und dokumentieren Sie die
Env-Variable im einleitenden Doc-Comment des Testmoduls, damit die CI
sie verdrahten kann.

## Ein durchgearbeitetes Beispiel

Das vollständige Dogfooding-Muster der App, das alles aus diesem
Kapitel kombiniert:

```rust
use app::migrations::Migrator;
use app::models::posts::Post;
use app::models::users::User;
use serial_test::serial;
use suprnova::testing::TestDatabase;
use suprnova::{Model, attrs, seed, FrameworkError};

#[tokio::test]
#[serial]
async fn users_and_posts_full_seed_round_trip() {
    // 1. Leere Seeder-Registry.
    seed::clear();

    // 2. Frische In-Memory-DB mit dem Migrator der App.
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    // 3. Die Seeder registrieren, die den Test interessieren.
    seed::register::<app::seeders::UsersSeeder>();
    seed::register::<app::seeders::PostsSeeder>();

    // 4. Den Seed innerhalb von without_events treiben, damit der
    //    Observer-Fanout nicht versucht, Jobs einzureihen (hier läuft
    //    keine Queue).
    seed::without_events(async {
        seed::run_all().await
    }).await.unwrap();

    // 5. Über die Model-Oberfläche und die rohe Connection zurücklesen.
    let user_count = User::query().count().await.unwrap();
    assert_eq!(user_count, 50);

    let raw_post_count = db.fetch_one(
        "SELECT COUNT(*) AS n FROM posts",
        vec![],
    ).await.unwrap();
    let n: i64 = raw_post_count.try_get("", "n").unwrap();
    assert_eq!(n, 200);

    // 6. Den abbrechbaren Observer-Pfad an einem frischen Model üben.
    let alice = User::create(attrs! {
        name: "Alice", email: "alice@example.com",
    }).await.unwrap();
    assert!(alice.id > 0);

    seed::clear();
}
```

Schritt 5 ist der Teil, der die Verdrahtung beweist: Die Model-Query
und das rohe `fetch_one` lesen beide dieselbe In-Memory-Datenbank -
die Model-Oberfläche, weil der Lookup von `DB::connection()` das
`TestContainer`-Binding gefunden hat, das rohe `fetch_one`, weil
`db.conn()` genau diese Connection direkt zurückgibt.

## Querverweise

- [Testen](testing.md) - der Test-Harness, `expect!`, `describe!`,
  `test!`, Fakes.
- [Datenbank](database.md#testing) - der Abschnitt zur
  oberflächlichen Testebene, der `TestDatabase` einführt.
- [Eloquent → Factories](eloquent-factories.md) - Factory-
  Definitionssyntax, States, Sequenzen, Relationen.
- [Seeding](seeding.md) - das Schreiben von Seedern, Reihenfolge,
  Idempotenz.
- [Service Container](container.md) - Task-Local- vs. Thread-Local-
  vs. globales Lookup, das entscheidet, was `DB::connection()`
  innerhalb eines Tests auflöst.
- [Mocking und Fakes](mocking.md) - `Storage::fake`, `Mail::fake`,
  `Queue::fake`, `Notification::fake`, und das Trait-Bind-Muster zum
  Austauschen von Fake-HTTP-Clients und anderen externen
  Oberflächen.
- [HTTP-Tests](http-tests.md) - Handler durch den Routing-Stack
  treiben, mit einer gebundenen `TestDatabase`.
