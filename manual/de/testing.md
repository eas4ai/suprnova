# Testen

Dies ist das Hub-Kapitel für die Test-Oberfläche von Suprnova - die
Makros, die In-Process-Datenbank, die Container-Fakes und die
Verschlüsselungsschlüssel-Helfer, zu denen Ihre Test-Binaries greifen.
Die vertiefenden Kapitel liegen direkt daneben:
[HTTP-Tests](http-tests.md) für Routen + Middleware,
[Datenbank-Tests](database-testing.md) für alles rund um
`TestDatabase`, [Mocking und Fakes](mocking.md) für die sieben
externen Oberflächen (Mail, Notify, Queue, Bus, Events, Storage,
HTTP-Client). Lesen Sie dieses, um zu erfahren, was in der Box
steckt; springen Sie zu einem Geschwister-Kapitel, wenn Sie die
Langform brauchen.

## Die Bausteine

| Baustein | Rolle |
|---|---|
| `#[tokio::test]` + `TestDatabase::fresh::<Migrator>()` | Das Standard-Arbeitstier - jeder echte Test im Framework verwendet dies |
| `#[suprnova_test]` | Zucker als Attribut-Makro - führt `App::init()` + `App::boot_services()` aus und baut eine `TestDatabase` für Sie |
| `describe!` + `test!` | Jest-förmige Gruppierungs-Makros, gepaart mit `expect!` für benannte Fehlerausgabe |
| `expect!` | Fluent-Assertion-Makro mit typisierten Matchern (Equality, Option, Result, String, Vec, Ordering) |
| `TestDatabase::fresh` / `sqlite_memory` | In-Memory-SQLite + Container-Registrierung, mit oder ohne Ihren Migrator |
| `TestContainer::fake` / `scope` / `spawn` | Thread-lokale oder Task-lokale DI-Overrides, hermetisch über parallele Tests hinweg |
| `install_test_encryption_key[ring]` | Deterministischer `APP_KEY` für Tests, die verschlüsselte Casts oder signierte Payloads berühren |
| Pro-Oberfläche-`fake()`-Helfer | Mail, Notify, Queue, Bus, Events, Storage, HTTP - siehe [Mocking](mocking.md) |
| `TestResponse` | Fluent-Assertions über das `(status, headers, body)`-Tripel eines HTTP-Tests - siehe [HTTP-Tests](http-tests.md#fluent-response-assertions-with-testresponse) |
| `AssertableInertia` | Fluent-Assertions über ein Inertia-Seitenobjekt - siehe [HTTP-Tests](http-tests.md#testing-inertia-responses) |


Sie greifen nicht in jedem Test nach allem. Ein typischer Action-Test
verwendet die ersten drei; ein DI-schwerer Test fügt `TestContainer`
hinzu; ein HTTP-Test tauscht `TestDatabase` gegen die
`handle_request`-Pipeline; ein Payments-Test installiert den
Verschlüsselungs-Keyring.

## Das Standard-Arbeitstier

Jeder echte Test im Framework sieht so aus:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn create_user_persists_it() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();

    let alice = User::create(attrs! {
        name: "Alice",
        email: "alice@example.com",
    })
    .await
    .unwrap();

    assert!(alice.id > 0);

    let row = users::Entity::find_by_id(alice.id)
        .one(db.conn())
        .await
        .unwrap();
    assert!(row.is_some());
}
```

`TestDatabase::fresh::<M>()` öffnet eine frische
`sqlite::memory:`-Connection, führt Ihren Migrator end-to-end aus und
registriert die Connection im Test-Container. Jeder Code, der danach
`DB::connection()` oder `App::resolve::<DbConnection>()` aufruft,
löst zu ihr auf - einschließlich des `#[suprnova::model]`-Query-
Builders und jedes Service, den Sie aus dem Container aufgelöst
haben. Droppt die `TestDatabase`, geht die Registrierung mit ihr.

Das Makro `test_database!()` ist Einzeiler-Zucker für den Fall
`crate::migrations::Migrator`:

```rust
use suprnova::test_database;

#[tokio::test]
async fn shortcut() {
    let db = test_database!();         // == TestDatabase::fresh::<crate::migrations::Migrator>()
    // ...
}
```

Für Tests, die präzise Kontrolle über die Spaltenform wollen
(Cast-Round-Trips, SQL-Oberfläche des Query-Builders), verwenden Sie
`TestDatabase::sqlite_memory()` - dieselbe Container-Verdrahtung,
kein Migrator. Das DDL ist Ihres. Siehe
[Datenbank-Tests](database-testing.md) für den vollständigen Katalog
plus die Helfer `execute_unprepared` / `fetch_one` / `fetch_all`.

## `#[suprnova_test]` - wenn Sie den syntaktischen Zucker wollen

`#[suprnova_test]` ist ein Attribut-Makro, das `#[tokio::test]`
umschließt, `App::init()` + `App::boot_services()` aufruft, damit
sich `#[injectable]`-Typen auflösen lassen, und eine frische
`TestDatabase` bindet. Es ist optionaler Zucker über der expliziten
Form oben, nützlich, wenn ein Test container-registrierte Services
auflöst:

```rust
use suprnova::suprnova_test;
use suprnova::{App, testing::TestDatabase};

#[suprnova_test]
async fn create_user_via_action(db: TestDatabase) {
    let action = App::resolve::<CreateUserAction>().unwrap();
    let user = action.execute("test@example.com").await.unwrap();

    assert_eq!(user.email, "test@example.com");
    assert!(user.id > 0);
}
```

Nimmt die Funktion einen `TestDatabase`-Parameter (namentlich)
entgegen, bindet das Makro die frische Datenbank an diesen Namen. Tut
sie das nicht, wird die Datenbank trotzdem konstruiert und
registriert (sodass `DB::connection()` funktioniert) - sie ist nur
nicht an eine lokale Variable gebunden.

Überschreiben Sie den Migrator mit dem Schlüssel `migrator = …`:

```rust
#[suprnova_test(migrator = my_crate::tests::IsolatedMigrator)]
async fn create_user_with_isolated_schema(db: TestDatabase) {
    // ...
}
```

Unbekannte Schlüssel sind ein Compile-Fehler (ein Tippfehler
`migrtor = …` behält nicht stillschweigend den Standard-Migrator).

## `describe!` und `test!` - wenn Gruppierung hilft

Für Testdateien, in denen dieselbe Aktion viele Fälle hat, gibt Ihnen
das Jest-förmige Paar `describe!` + `test!` verschachtelte
Gruppierung und benannte Fehlerausgabe:

```rust
use suprnova::{App, describe, test, expect, testing::TestDatabase};
use crate::migrations::Migrator;

describe!("ListTodosAction", {
    test!("returns empty list when no todos exist", async fn(db: TestDatabase) {
        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_be_empty();
    });

    test!("returns all todos", async fn(db: TestDatabase) {
        Todo::create(attrs! { title: "Buy bread" }).await.unwrap();
        Todo::create(attrs! { title: "Walk dog" }).await.unwrap();

        let todos = App::resolve::<ListTodosAction>().unwrap().execute().await.unwrap();
        expect!(todos).to_have_length(2);
    });

    describe!("with pagination", {
        test!("returns first page", async fn(db: TestDatabase) {
            // verschachtelte Gruppen komponieren sich
        });
    });
});
```

`test!` akzeptiert drei Formen:

```rust
// Asynchroner Test mit TestDatabase-Parameter
test!("creates a user", async fn(db: TestDatabase) { … });

// Asynchroner Test ohne Datenbank
test!("calculates the right sum", async fn() { … });

// Synchroner Test
test!("adds numbers", fn() { … });
```

Der Named-Test-Wrapper fädelt den Testnamen durch die
`expect!`-Maschinerie, sodass ein Fehlschlag sichtbar wird:

```text
Test: "returns all todos"
  at src/actions/todo_action.rs:25

  expect!(actual).to_equal(expected)

  Expected: 2
  Received: 0
```

Ohne `describe!`/`test!` bekommen Sie die Standard-`panic!`-Ausgabe.
Mit ihnen führen der Ort und der menschenlesbare Testname die
Meldung an.

## `expect!` - der Matcher-Katalog

`expect!(value)` liefert einen `Expect<T>`-Wrapper. Die Matcher sind
auf `T` typisiert - `to_be_some()` auf einem `String` aufzurufen ist
ein Compile-Fehler, kein Laufzeit-Panic.

```rust
use suprnova::expect;

// Gleichheit (T: Debug + PartialEq)
expect!(actual).to_equal(expected);
expect!(actual).to_not_equal(unexpected);

// Boolesche Werte
expect!(condition).to_be_true();
expect!(condition).to_be_false();

// Option<T>
expect!(option).to_be_some();
expect!(option).to_be_none();
expect!(option).to_contain_value(5);     // Prüfung auf Some(5)

// Result<T, E>
expect!(result).to_be_ok();
expect!(result).to_be_err();

// String / &str
expect!(s).to_contain("substring");
expect!(s).to_start_with("prefix");
expect!(s).to_end_with("suffix");
expect!(s).to_have_length(10);
expect!(s).to_be_empty();

// Vec<T>
expect!(v).to_have_length(3);
expect!(v).to_contain(&item);
expect!(v).to_be_empty();

// Vergleich (T: Debug + PartialOrd)
expect!(10).to_be_greater_than(5);
expect!(5).to_be_less_than(10);
expect!(10).to_be_greater_than_or_equal(10);
expect!(5).to_be_less_than_or_equal(5);
```

Sie können `expect!` auch außerhalb von `test!` verwenden - die
Datei/Zeile in der Fehlermeldung kommt von
`concat!(file!(), ":", line!())`. Der Named-Test-Header ist das
Einzige, was das Makro nicht von selbst hinzufügt.

## `TestContainer` - DI-Fakes, die nicht durchsickern

Das Container-Kapitel behandelt den [dreistufigen
Lookup](container.md) im Detail. Für Tests sind die beiden
Einstiegspunkte `TestContainer::fake()` (thread-lokal) und
`TestContainer::scope(…).await` (task-lokal).

### Thread-lokal, der Normalfall

`TestContainer::fake()` liefert einen Guard. Bis der Guard droppt,
landen Schreibvorgänge von `TestContainer::singleton` / `bind` /
`factory` auf der thread-lokalen Override-Ebene und überschatten den
globalen Container:

```rust
use std::sync::Arc;
use suprnova::App;
use suprnova::testing::TestContainer;

#[tokio::test]
async fn order_dispatches_email() {
    let _guard = TestContainer::fake();

    let fake = Arc::new(FakeEmailGateway::new());
    let probe = Arc::clone(&fake);
    TestContainer::bind::<dyn EmailGateway>(fake);

    place_order(123).await.unwrap();

    assert_eq!(probe.sent_count(), 1);
}
```

`TestDatabase::fresh` / `sqlite_memory` installieren intern ihren
eigenen `TestContainer::fake`-Guard - Sie stapeln sie nicht, außer
Sie testen die Registry selbst.

### Task-lokal, für `multi_thread`-Runtimes

Die thread-lokale Ebene wird auf dem OS-Thread gesetzt, der `fake()`
aufgerufen hat. Eine `multi_thread`-Tokio-Runtime kann Ihr Future
über ein `.await` hinweg auf einen anderen Worker-Thread migrieren,
und der Override verschwindet stillschweigend. `TestContainer::scope`
löst das, indem es den Override statt dessen an das Future bindet:

```rust
use suprnova::testing::TestContainer;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn cross_worker_safe() {
    TestContainer::scope(async {
        TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
        do_async_work_that_may_hop_workers().await;
    })
    .await;
}
```

Mit `tokio::spawn` gespawnte Sub-Tasks erben Tokio-Task-Locals nicht;
verwenden Sie stattdessen `TestContainer::spawn` - es erfasst den
Container des aktuellen Scopes und installiert ihn im gespawnten
Future erneut:

```rust
TestContainer::scope(async {
    TestContainer::bind::<dyn HttpClient>(Arc::new(FakeHttpClient::new()));
    let h = TestContainer::spawn(async {
        App::make::<dyn HttpClient>().unwrap()  // sieht den Fake
    });
    let _client = h.await.unwrap();
})
.await;
```

### Warum es einen `FAKE_GUARDS`-Refcount gibt

Der thread-lokale Container ist pro Test, aber Suprnova hat außerdem
eine prozessglobale `ConnectionRegistry`, die nach Namen schlüsselt
(`__read_replica__`, benutzerdefinierte Connection-Labels) und einen
Thread-Local-Reset überlebt. Eine naive `Drop`-Implementierung würde
`ConnectionRegistry::clear()` jedes Mal aufrufen, wenn *irgendein*
`TestContainerGuard` verschwindet - und damit die benannte Connection
eines anderen gleichzeitig laufenden Tests mitten in dessen Lauf
auslöschen.

Die Lösung ist ein prozessweiter `AtomicUsize` (`FAKE_GUARDS`).
`fake()` erhöht ihn; `drop` verringert ihn; nur der Übergang zurück
auf null leert die benannte Registry. Zwei parallele Tests, die
`__read_replica__` verwenden, sind sicher: Welcher Guard auch immer
zuletzt droppt, dem gehört das Leeren.

Sie rufen das nicht aus einem Test heraus auf - es läuft aus dem
`Drop` von `TestContainerGuard`. Sie müssen nur wissen, dass es das
gibt, wenn Sie ein Symptom wie "benannte Connection mitten im Test
verschwunden" debuggen, was meist bedeutet, dass ein
Geschwister-Test vergessen hat, zuerst auf das Droppen seines eigenen
Guards zu warten.

## Test-Helfer für Verschlüsselungsschlüssel

Tests, die verschlüsselte Casts ausüben (`casts = { secret =
AsEncrypted }` auf einem `#[model(...)]`), signierte Payloads oder
den Previous-Key-Fallback des Keyrings, brauchen einen In-Process
installierten `APP_KEY`. Das Framework liefert zwei testexklusive
Helfer unter dem `testing`-Feature:

```rust
use suprnova::testing::install_test_encryption_key;

#[tokio::test]
async fn cast_roundtrip() {
    install_test_encryption_key();   // idempotent; deterministischer 32-Byte-Nullschlüssel
    let db = TestDatabase::sqlite_memory().await.unwrap();
    // … verschlüsseln + zurücklesen …
}
```

`install_test_encryption_key` ist idempotent - die zugrunde liegende
`Crypt`-Facade basiert auf `OnceLock`, sodass der zweite Aufruf ein
No-Op ist. Die meisten Cast-Test-Binaries rufen es aus jedem Test
auf, der einen verschlüsselten Cast berührt; der erste gewinnt, die
übrigen sind kostenlos.

Für Rotationstests (Schreiben unter dem alten Schlüssel, Lesen unter
dem neuen), verwenden Sie die Keyring-Variante:

```rust
use suprnova::crypto::EncryptionKey;
use suprnova::testing::install_test_encryption_keyring;

let new = EncryptionKey::from_base64("...").unwrap();
let old = EncryptionKey::from_base64("...").unwrap();
let installed = install_test_encryption_keyring(new, vec![old]);
assert!(installed, "first install wins");
```

Der Keyring-Helfer liefert `true` nur, wenn der Aufruf den Ring
tatsächlich installiert hat (der `OnceLock` war leer). Um
Chiffretext unter einem beliebigen Schlüssel für einen Rotationstest
zu prägen, verwenden Sie `suprnova::crypto::_test_encrypt_with`,
statt zweimal zu installieren.

Beide Helfer sind `#[doc(hidden)]` auf der Crypto-Ebene und
re-exportiert unter dem `testing`-Modul - sie sind testexklusiv und
umgehen den Produktions-Validierungspfad für `APP_KEY`.

## Das `testing`-Feature und Produktions-Builds

`suprnova` stellt seine Test-Helfer (`Storage::fake()`, `TestContainer`,
`TestDatabase`, Crypto-Rotations-Hooks wie `_test_install_key`) hinter
einem Cargo-Feature namens `testing` bereit. Das Feature liegt im
Default-Set, sodass konsumierende Testsuiten sie kostenlos bekommen:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.2" }

[dev-dependencies]
# `testing` ist über die Abhängigkeit oben transitiv aktiv - nichts weiter nötig.
```

Die Hooks sind `#[doc(hidden)]` und mit `_test_` präfigiert, sodass sie
selbst bei aktiviertem Feature aus idiomatischem Anwendungscode nicht
erreichbar sind. Die tragende Absicherung ist `Server::from_config`: Es
validiert `APP_KEY` bei **jedem** Boot, nicht nur dann, wenn der
Keyring uninitialisiert ist. Ein vorinstallierter Testschlüssel kann
diese Prüfung nicht umgehen - der Boot schlägt sofort fehl, wenn
`APP_KEY` fehlt oder fehlerhaft ist, unabhängig davon, ob irgendetwas
im Prozess vorab einen Schlüssel installiert hat.

Wenn Sie es vorziehen, dass die Helfer gar nicht erst in Ihr
Produktionsartefakt gelinkt werden (Defence in Depth), hängen Sie von
`suprnova` mit abgeschalteten Default-Features ab und aktivieren nur,
was Sie ausliefern:

```toml
[dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.2", default-features = false, features = ["..."] }

[dev-dependencies]
suprnova = { git = "https://github.com/eas4ai/suprnova.git", tag = "v1.3.2", features = ["testing", "..."] }
```

Das ist eine Verschärfung, kein Fix - die Boot-Validierung schließt den
eigentlichen Exploit unabhängig davon, welche Haltung Sie wählen.

### Warum Suprnova abweicht

Laravels PHP-Test-Harness bekommt die Isolation paralleler Tests fast
geschenkt, weil die Runtime pro Anfrage single-threaded ist und Tests
pro Datei einen neuen Prozess forken. Das Suprnova-Test-Binary ist ein
Prozess, der viele `#[tokio::test]`s gleichzeitig auf einem oder
mehreren Worker-Threads ausführt. Ein einziger globaler Container
würde bedeuten, dass der Fake des einen Tests in den Lookup des
nächsten Tests blutet, sobald sie sich auf einem Worker-Thread
überlappen.

Deshalb hat `TestContainer` beide Ausprägungen - thread-lokal für den
häufigen `current_thread`-Fall, task-lokal für `multi_thread`. Das über
`FAKE_GUARDS` refcount-gezählte Leeren der prozessglobalen
`ConnectionRegistry` existiert aus demselben Grund: Geteilter Zustand,
der sich nicht pro Test anlegen lässt, muss wenigstens wissen, dass er
sich nicht selbst wegwischen darf, während ein anderer Test sich noch
auf ihn stützt.

Der Matcher-Katalog (`expect!`) ist typisiert, weil Rust das zulässt.
Jests `expect(x).toBeSome()` weiß erst zur Laufzeit, ob `x` eine
`Option` ist; Suprnovas `Expect<T>` weiß es zur Compile-Zeit, sodass
ein falscher Matcher ein Build-Fehler ist und kein flakiger Test.

## Wo jedes Teil lebt

| Teil | Quelle |
|---|---|
| Attribut-Makro `#[suprnova_test]` | `suprnova-macros/src/suprnova_test.rs` |
| Proc-Makros `describe!` / `test!` | `suprnova-macros/src/describe.rs`, `test_macro.rs` |
| Makro `expect!` + `Expect<T>`-Matcher | `framework/src/lib.rs` (Makro), `framework/src/testing/expect.rs` (Impls) |
| `TestDatabase::fresh` / `sqlite_memory` / Helfer | `framework/src/database/testing.rs` |
| Makro `test_database!` | `framework/src/database/testing.rs` |
| `TestContainer` + `TestContainerGuard` + `FAKE_GUARDS` | `framework/src/container/testing.rs` |
| `install_test_encryption_key[ring]` | `framework/src/testing/mod.rs` |
| Pro-Oberfläche-Fakes (Mail, Notify, Queue, Bus, Events, Storage, HTTP) | Pro-Domäne-`testing`-Submodule - siehe [Mocking](mocking.md) |
| `TestResponse` | `framework/src/testing/response.rs` |
| `AssertableInertia`, `ReloadRequest` | `framework/src/testing/inertia.rs` |

## Tests ausführen

Die üblichen Cargo-Aufrufe gelten:

```bash
# Gesamter Workspace
cargo test --workspace

# Eine Crate
cargo test -p suprnova

# Ein Test nach Namen (Substring-Match)
cargo test create_user_persists_it

# Mit println!- und dbg!-Ausgabe
cargo test -- --nocapture
```

Suprnova liefert keinen eigenen Test-Runner; das Framework
integriert sich in den von Cargo. Datenbank-Tests laufen
standardmäßig parallel - der thread-lokale Container und die
In-Memory-SQLite pro Test sind genau dafür ausgelegt.

## Nächste Schritte

- [HTTP-Tests](http-tests.md) - die vollständige Request-Pipeline
  über `handle_request` treiben
- [Datenbank-Tests](database-testing.md) - `TestDatabase`, Factories
  in Tests, Seeder in Tests, paralleles sicheres DB-Testen
- [Mocking und Fakes](mocking.md) - die sieben Fakes für externe
  Oberflächen und die Muster, die sie teilen
- [Service Container](container.md) - der dreistufige Lookup, den
  `TestContainer` überschreibt
- [Fehlermodell](error-model.md) - `FrameworkError`-Formen, auf die
  Sie assertieren werden
