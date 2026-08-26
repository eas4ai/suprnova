# Datenbank

Suprnovas Datenbankschicht umhüllt SeaORM mit einer Laravel-förmigen
`DB`-Facade: rohe Query-Escapes, ein modell-loser Query Builder,
Transaktionen mit Savepoints und Wiederholung bei Deadlock, eine
Connection-Registry für Read-Replicas und Shards, und eine
vollständige Beobachtbarkeits-Oberfläche, die Laravel 13s
`DB::listen`- / `QueryExecuted`- / Query-Log-API spiegelt.

Das Eloquent-ORM (`use suprnova::eloquent::*`) baut auf dieser
Schicht auf und lebt in [eloquent.md](eloquent.md). Wollen Sie ein
typisiertes Model, gehen Sie dorthin; wollen Sie eine rohe Query
gegen eine nicht modellierte Tabelle oder wollen Sie jede Query
beobachten, die das Framework ausführt, ist das hier die richtige
Seite.

## Konfiguration

```rust
use suprnova::{Config, DB, DatabaseConfig};

// In bootstrap.rs
Config::register(DatabaseConfig::from_env());
DB::init().await.expect("DB::init failed");
```

`DatabaseConfig::from_env` liest `DATABASE_URL` und (optional) die
Pool-Stellschrauben `DB_MAX_CONNECTIONS`, `DB_MIN_CONNECTIONS`,
`DB_CONNECT_TIMEOUT`, `DB_LOGGING`. Ist `DATABASE_URL` nicht gesetzt,
fällt die Config auf `sqlite://./database.db` zurück - praktisch für die
Entwicklung ohne Einrichtung; ein Produktionsstart verweigert den
Fallback über `validate_for_environment`, sodass Sie nicht versehentlich
eine SQLite-Datei unter `APP_ENV=production` ausliefern.

URL → Treibererkennung:

```text
postgres://user:pass@host/db       → DatabaseType::Postgres
postgresql://user:pass@host/db     → DatabaseType::Postgres
mysql://user:pass@host/db          → DatabaseType::Mysql
sqlite://./file.db                 → DatabaseType::Sqlite
sqlite::memory:                    → DatabaseType::Sqlite
```

### Pool-Lebendigkeit

Ein NAT-Gateway, ein Load Balancer oder eine Firewall verwirft eine
TCP-Verbindung stillschweigend, wenn sie zu lange untätig war. Der Pool
erfährt davon nichts. Die nächste Query auf dieser Verbindung schlägt
fehl, und zwar bei einer Anfrage, die mit dem Ausfall nichts zu tun
hatte.

Laravel beantwortet das mit den libpq-DSN-Optionen `keepalives`,
`keepalives_idle`, `keepalives_interval` und `keepalives_count`, die den
Socket warm halten. **Aus Suprnova heraus sind sie nicht erreichbar.**
sqlx 0.9 parst aus einer Postgres-URL nur `sslmode`, `application_name`,
`options` und die Größe des Statement-Cache und bietet auf keiner Ebene
einen Setter für TCP-Keepalive, es gibt also nichts, wohin man sie
weiterreichen könnte.

Was Suprnova Ihnen stattdessen gibt, ist die Antwort auf der Pool-Seite:
alten Verbindungen nicht mehr vertrauen.

```bash
# Eine Verbindung schließen, die zwei Minuten lang untätig war.
DB_IDLE_TIMEOUT=120
# Jede Verbindung nach fünfzehn Minuten unabhängig davon recyceln.
DB_MAX_LIFETIME=900
# Eine Verbindung vor der Herausgabe anpingen, aber erst, wenn sie
# dreißig Sekunden untätig war. Heiße Verbindungen sparen sich den
# Round-Trip.
DB_PING_AFTER_IDLE=30
```

Oder programmatisch:

```rust
Config::register(
    DatabaseConfig::builder()
        .url(std::env::var("DATABASE_URL")?)
        .idle_timeout(120)
        .max_lifetime(900)
        .ping_after_idle(30)
        .build(),
);
```

Jede Stellschraube ist standardmäßig ungesetzt, das heißt, der Pool
behält die eigenen Standardwerte von sqlx: Verbindungen schließen nach
600 untätigen Sekunden, werden nach 1800 Sekunden recycelt und vor jedem
Checkout angepingt. Setzen Sie `DB_IDLE_TIMEOUT=0` oder
`DB_MAX_LIFETIME=0`, um diese Form der Ernte vollständig abzuschalten.

`DB_PING_AFTER_IDLE` und `DB_TEST_BEFORE_ACQUIRE` sind Alternativen, kein
Paar: Einen Schwellenwert zu setzen schaltet den Ping pro Checkout ab,
denn beides zusammen würde bei jedem Acquire pingen und den
Schwellenwert bedeutungslos machen.

### Warum Suprnova abweicht

Keepalives und Pool-Recycling lösen denselben Ausfall von zwei Seiten
her. Keepalives verhindern, dass eine Middlebox die Verbindung ablaufen
lässt; Recycling nimmt hin, dass sie es tun wird, und stellt sicher, dass
der Pool nie eine Verbindung herausgibt, die alt genug wäre, um
abgelaufen zu sein. Das Zweite ist das, was der Treiber-Stack anbietet,
und es deckt zusätzlich Ausfälle ab, die Keepalives nicht abdecken - ein
Replikat nach einem Failover, ein rotiertes Credential, ein
serverseitiger Idle-Disconnect. Wenn Sie speziell die libpq-Optionen
brauchen, ist das eine Änderung an sqlx, nicht an Suprnova.

## Rohe Queries

Die `DB`-Facade liefert die vollständige Raw-Escape-Oberfläche von
Laravel 13. Jeder Helfer läuft über denselben instrumentierten
Executor - jeder Aufruf löst `QueryExecuted` aus (siehe
[Beobachtbarkeit](#beobachtbarkeit)).

Bindings sind `sea_orm::Value` - einer der wenigen sea_orm-Typen, die
das Framework absichtlich NICHT neu maskiert, weil jeder Wert, der
auf den Wire geht, dadurch läuft. `Value::from(...)` funktioniert für
jedes Primitive, das die Datenbank versteht.

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - alle Zeilen als DynamicRow.
let users = DB::select(
    "SELECT * FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// SELECT - nur die erste Zeile.
let alice = DB::select_one(
    "SELECT * FROM users WHERE name = ?",
    vec![Value::from("alice")],
).await?;

// SELECT - erste Spalte der ersten Zeile als typisierter Wert.
let count: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users",
    vec![],
).await?;

// INSERT - liefert bool (true, wenn mindestens eine Zeile betroffen war).
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - liefern die Anzahl betroffener Zeilen.
let updated = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1)],
).await?;
let deleted = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// Jedes Prepared Statement mit Bindings.
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1), Value::from(42)],
).await?;

// DDL ohne Bindings - `unprepared` spiegelt Laravels `DB::unprepared`
// für Statements (CREATE INDEX, ALTER TABLE, VACUUM), die
// Platzhalter-Bindung zurückweisen.
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// affecting_statement ist die explizite Form, die update/delete
// intern verwenden - fallen Sie direkt darauf zurück für Operationen,
// die in keinen der beiden Namen passen (z. B. INSERT...ON CONFLICT
// DO UPDATE).
let affected = DB::affecting_statement(
    "INSERT INTO users (id, name) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name",
    vec![Value::from(1), Value::from("alice")],
).await?;
```

### Platzhalter-Syntax

`?` für SQLite + MySQL. `$1`, `$2`, ... für Postgres. Das aktive
Backend wird automatisch aus `DatabaseConfig::url` erkannt.

### DynamicRow

Untypisierte Zeilen materialisieren sich als `DynamicRow` - ein
`serde_json::Map`-Newtype mit typisierten Zugriffsmethoden:

```rust
for row in users {
    let id: i64 = row.get_int("id")?;
    let name: String = row.get_string("name")?;
    let nickname: Option<String> = row.get_optional_string("nickname")?;
    let score: Option<i64> = row.get_optional_int("score")?;
    // Ein beliebiges T deserialisieren (chrono::DateTime, Ihre eigene
    // Struktur usw.):
    let prefs: UserPrefs = row.get_as("prefs")?;
}
```

`get_*` schlägt fehl, wenn die Spalte fehlt ODER null ist.
`get_optional_*` schlägt nur fehl, wenn sie fehlt, und liefert
`Ok(None)` für SQL NULL. Die vollständige Liste der Zugriffsmethoden
ist `get_int` / `get_string` / `get_bool` / `get_float` / `get_value`
/ `get_as<T>` plus `get_optional_string` / `get_optional_int`; für
nullbare Typen ohne dediziertes `get_optional_*` greifen Sie zu
`get_value` + einem `serde_json::Value`-Match, oder zu
`get_as::<Option<T>>`.

## Modell-loser Query Builder - `DB::table`

Für Ad-hoc-Queries gegen Tabellen, die Sie sich nicht die Mühe
gemacht haben, mit `#[suprnova::model]` zu modellieren, liefert
`DB::table(...)` einen verkettbaren Builder in der Form des Eloquent-
`Builder<M>`, der Zeilen aber als `DynamicRow` materialisiert:

```rust
use suprnova::{DB, attrs};

let rows = DB::table("audit_log")
    .select(["id", "event", "actor_id"])
    .filter("actor_id", 42i64)
    .filter_op("created_at", ">=", "2025-01-01")
    .order_by_desc("id")
    .limit(50)
    .get()
    .await?;

let first = DB::table("audit_log")
    .filter("event", "user.deleted")
    .first()
    .await?;

let count = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .count()
    .await?;

let id = DB::table("audit_log")
    .insert(attrs! { event: "user.created", actor_id: 42 })
    .await?;

let updated = DB::table("audit_log")
    .filter("id", id)
    .update(attrs! { event: "user.created.v2" })
    .await?;

let deleted = DB::table("audit_log")
    .filter("actor_id", 42i64)
    .delete()
    .await?;
```

### Vertrauensgrenze für Identifier

Tabellennamen, Spaltennamen, ORDER-BY-Richtungen und SQL-Operatoren
werden wortwörtlich IN den SQL-String interpoliert - sie werden NICHT
als Parameter gebunden (SQL erlaubt keine platzhaltergebundenen
Identifier). Behandeln Sie jedes `impl Into<String>`-Argument als
VERTRAUENSWÜRDIGES Literal:

```rust
// Sicher - der Spaltenname ist eine Konstante.
DB::table("users").filter("email", request.email()).get().await?;

// UNSICHER - spleißen Sie niemals Nutzereingaben in einen Spaltennamen.
DB::table("users").filter(&request.column_name(), value).get().await?;
```

Werte (die rechte Seite von `filter` / `filter_op`) WERDEN als
Parameter gebunden und sind für Nutzereingaben sicher.

Das Framework erzwingt eine strikte Allowlist auf Identifier
(`[A-Za-z_][A-Za-z0-9_]*` mit einem optionalen `schema.`-Präfix) und
Operatoren (`=`, `<>`, `<`, `<=`, `>`, `>=`, `LIKE`, `NOT LIKE`,
`ILIKE`, `NOT ILIKE`, `IS`, `IS NOT`). Verstöße scheitern an der
I/O-Grenze, bevor der SQL-String gerendert wird.

## Transaktionen

Drei Einstiegspunkte, jeder mit den Beobachtungs-Hooks `QueryExecuted` /
`TransactionBeginning` / `TransactionCommitted` /
`TransactionRolledBack` verdrahtet.

### Closure-Form

```rust
use suprnova::DB;

DB::transaction(|_tx| {
    Box::pin(async move {
        let mut alice = User::query().filter("name", "alice").first_or_fail().await?;
        alice.balance -= 30;
        alice.save().await?;

        let mut bob = User::query().filter("name", "bob").first_or_fail().await?;
        bob.balance += 30;
        bob.save().await?;
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

Commit bei `Ok(_)`. Rollback und Weitergabe des Fehlers bei `Err(_)`.

Ein `Err` ist nicht immer ein Rollback. Wenn ein Callback
[nach dem Commit](queues.md#after-commit-dispatch) fehlschlägt, ist der
Commit bereits gelandet und dauerhaft; `DB::transaction` gibt trotzdem
`Err` zurück, und die Meldung lautet `after-commit callback failed (the
transaction itself committed): <the callback's error>`. Der Rückgabewert
des Closures geht verloren, seine Schreibvorgänge nicht, und
fehlgeschlagen ist nur ein verzögerter Dispatch. Jeder registrierte
Callback läuft trotzdem, und der erste Fehler ist der, den Sie bekommen.
`DB::transaction_with_attempts` wiederholt diesen Fehler nie, so
deadlock-förmig er auch klingt: Ein Closure erneut auszuführen, dessen
Schreibvorgänge bereits dauerhaft sind, würde sie zweimal anwenden.

Operationen innerhalb des Closures greifen die aktive Transaktion
automatisch über ein `tokio::task_local` auf - Sie müssen KEIN
`&tx`-Handle durch jeden Modellaufruf fädeln. Ein verschachteltes
`DB::transaction` gibt einen Datenbankfehler zurück; nehmen Sie
`tx.savepoint(...)` für verschachteltes Rollback-Verhalten.

Die Closure-Form ist außerdem die einzige Form, die Arbeit auf den
Commit verschieben kann. Ein Job, dessen Typ `Job::after_commit()`
deklariert (oder ein mit `Queue::push_after_commit` erfolgter Dispatch),
wartet innerhalb dieses Closures und erreicht den Queue-Treiber erst,
wenn der Commit gelingt; ein Rollback verwirft ihn. Siehe
[Dispatch nach dem Commit](queues.md#after-commit-dispatch).

Für typisierte Aggregate oder eigenes SQL, das auf derselben
angehefteten Connection laufen muss, verwenden Sie das
Transaktions-Handle direkt:

```rust
use sea_orm::{DbBackend, Statement};

DB::transaction(|tx| {
    Box::pin(async move {
        let backend = tx.backend();
        let rows = tx.query_all(Statement::from_string(
            backend,
            "SELECT CAST(COUNT(*) AS BIGINT) AS total FROM orders".to_owned(),
        )).await?;
        let total = rows[0].try_get::<i64>("", "total")?;
        Ok::<_, suprnova::FrameworkError>(total)
    })
}).await?;
```

`query_all` gibt normale `QueryExecuted`-Beobachtungen aus und liefert
typisierte SeaORM-`QueryResult`-Zeilen. Nehmen Sie für dynamische Werte
das gebundene `Statement::from_sql_and_values`; interpolieren Sie keine
nicht vertrauenswürdige Eingabe.

### Wiederholung bei Deadlock

```rust
DB::transaction_with_attempts(5, |_tx| {
    Box::pin(async move {
        // Derselbe Closure-Rumpf wie oben. Läuft von vorn bei
        // SQLSTATE 40001 / 40P01 / jedem Fehler, der "deadlock" enthält
        // (ohne Beachtung der Groß-/Kleinschreibung).
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

### Manuelle Form

```rust
use suprnova::{DB, attrs};

let tx = DB::begin_transaction().await?;

// Pro Modell: Die `*_with_tx`-Shims heften eine CRUD-Operation an die manuelle tx.
User::create_with_tx(&tx, attrs! { name: "alice" }).await?;
Order::create_with_tx(&tx, attrs! { user_id: 1, total: 30 }).await?;

// Pro Query: `Builder::with_tx(&tx)` heftet eine Builder-Chain an.
let stale = Order::query()
    .filter("status", "pending")
    .with_tx(&tx)
    .get()
    .await?;

if some_condition() {
    tx.rollback().await?;
} else {
    tx.commit().await?;
}
```

Der manuelle Modus installiert das Task-Local NICHT - jede Operation,
die innerhalb der Transaktion laufen soll, muss sich dafür anmelden,
entweder über `Builder::with_tx(&tx)` an einer verketteten Query oder
über einen der `Model::*_with_tx`-Shims (`create_with_tx`,
`save_with_tx`, `delete_with_tx` usw.). Operationen, die die Anmeldung
vergessen, laufen gegen den globalen Pool und sind NICHT Teil der
Transaktion.

Ein `Transaction`-Handle zu halten heftet für dessen Lebensdauer eine
Pool-Connection an; laden Sie alle Zeilen, die Sie lesen müssen, VOR dem
`begin_transaction()`-Aufruf vor, besonders auf SQLite (eine einzige
geteilte Connection).

Weil der manuelle Modus kein Task-Local installiert, hat er auch keinen
Commit, an den sich ein verzögerter Dispatch hängen könnte: Ein Job
[nach dem Commit](queues.md#after-commit-dispatch), der innerhalb einer
manuellen Transaktion geschoben wird, wird sofort geschoben. Nehmen Sie
die Closure-Form, wenn ein Dispatch auf den Commit warten muss.

### Savepoints

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        Order::create(/* ... */).await?;

        tx.savepoint("after_order").await?;
        if let Err(e) = Payment::charge().await {
            // Den Zahlungsversuch verwerfen, die Bestellung aber behalten.
            tx.rollback_to("after_order").await?;
        }
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

Alle drei erstklassigen Backends unterstützen `SAVEPOINT` /
`ROLLBACK TO SAVEPOINT` - SQLite eingeschlossen.

Ein Savepoint-Rollback wickelt auch die
[Registry für Callbacks nach dem Commit](queues.md#after-commit-dispatch)
ab. Ein Queue-Push, der innerhalb des Savepoints auf den Commit
verschoben wurde, wird zusammen mit den Zeilen verworfen, die er
beschrieb, und die mit ihm registrierte Kompensation läuft sofort, sodass
die Dedupe-Sperre eines verzögerten `push_unique` zurückgeht und ein
erneuter Dispatch innerhalb derselben Transaktion sie gewinnen kann.
Alles vor dem Savepoint Registrierte bleibt unangetastet, und ein
Savepoint, den Sie freigeben oder schlicht nie zurückrollen, behält
alles, was in ihm registriert wurde.

Einen Savepoint-Namen zu wiederholen ist erlaubt, und die Registry folgt
der Datenbank: `ROLLBACK TO SAVEPOINT x` wickelt bis zum jüngsten `x` ab
und zerstört die danach angelegten Savepoints. Manuelle Transaktionen
haben keine Registry für Callbacks nach dem Commit, ihre Savepoints
rollen also Zeilen zurück und sonst nichts.

Nur `Transaction::savepoint` markiert die Registry. Ein Savepoint, den
Sie mit rohem SQL anlegen, ist für sie unsichtbar; `rollback_to` rollt
diese Zeilen also zurück, protokolliert eine Warnung und lässt jeden
darin registrierten verzögerten Dispatch stehen - einen davon auf
Verdacht zu verwerfen wäre der schlimmere Fehlschlag. Nehmen Sie
`Transaction::savepoint`, wenn die verzögerten Dispatches gemeinsam mit
den Zeilen abgewickelt werden sollen.

## Beobachtbarkeit

Die `DB::listen`- / `QueryExecuted`- / Query-Log-Oberfläche von
Laravel 13, nach Rust portiert über Suprnovas Event-Dispatcher.

### `DB::listen` - direkter Callback

```rust
use suprnova::{DB, QueryExecuted};

// In bootstrap.rs (oder einem Service-Provider).
DB::listen(|event: &QueryExecuted| {
    tracing::debug!(
        sql = %event.sql,
        bindings = ?event.bindings,
        time_ms = event.time.as_millis(),
        connection = %event.connection_name,
        "query executed",
    );
})?;
```

Listener laufen **synchron innerhalb des Executor-Helfers**. Ein
langsamer Listener verlangsamt die Query - halten Sie direkte
Callbacks leichtgewichtig. Für alles, was fehlschlagen kann,
bevorzugen Sie den `EventFacade`-Pfad unten; er läuft über
`dispatch_best_effort` und toleriert Fehler.

### `EventFacade`-Dispatch-Pfad

`QueryExecuted` ist ein echtes `suprnova::Event` - hören Sie über den
Dispatcher zu, um eingereihte, fakebare, fehlertolerante Zustellung
zu bekommen:

```rust
use suprnova::{EventFacade, Listener, QueryExecuted, FrameworkError};
use std::sync::Arc;

struct LogToDatabase;

#[suprnova::async_trait]
impl Listener<QueryExecuted> for LogToDatabase {
    async fn handle(&self, event: &QueryExecuted) -> Result<(), FrameworkError> {
        // Selbst wenn DIESER Listener die Datenbank abfragt,
        // verhindert der Re-Entrancy-Guard unendliche Rekursion.
        DB::statement(
            "INSERT INTO query_log (sql, time_ms) VALUES (?, ?)",
            vec![event.sql.clone().into(), (event.time.as_millis() as i64).into()],
        ).await?;
        Ok(())
    }
}

// In bootstrap.rs.
EventFacade::listen::<QueryExecuted, _>(Arc::new(LogToDatabase)).await;
```

Listener auf diesem Pfad:

- Laufen über `dispatch_best_effort` - ein fehlschlagender Listener
  lässt die Query NICHT fehlschlagen.
- Werden kurzgeschlossen, wenn sie selbst eine Query auslösen
  (Re-Entrancy-Guard).
- Können `Event::fake()` in Tests verwenden, um den Dispatch zu
  assertieren, ohne Listener tatsächlich laufen zu lassen.

### In-Memory-Query-Log

```rust
DB::enable_query_log()?;

User::query().filter("active", true).get().await?;
Order::query().count().await?;

let log = DB::get_query_log()?;
for query in &log {
    println!("{} ({}ms)", query.sql, query.time.as_millis());
}

DB::flush_query_log()?;     // Einträge verwerfen, aktiviert lassen
DB::disable_query_log()?;   // Erfassung stoppen
let still_capturing = DB::logging();
```

Das Log ist **unbegrenzt** - jede erfasste Query lässt es wachsen,
bis der Prozess endet, `flush_query_log()` läuft, oder
`disable_query_log()` aufgerufen wird. Verwenden Sie es für die
Entwicklung, nicht als langlaufenden Produktions-Profiler.

### Transaktions-Lifecycle-Events

`TransactionBeginning`, `TransactionCommitted` und
`TransactionRolledBack` sind echte `suprnova::Event`-Typen - hören Sie
über `EventFacade::listen` auf sie, um Auditing, verteilte Sperren
oder Kompensationslogik zu treiben.

```rust
EventFacade::listen::<TransactionCommitted, _>(Arc::new(AuditCommit)).await;
EventFacade::listen::<TransactionRolledBack, _>(Arc::new(MetricRollback)).await;
```

Alle drei Transaktions-Einstiegspunkte
(`DB::transaction` / `DB::transaction_with_attempts` /
`DB::begin_transaction` + `Transaction::commit`/`rollback`) lösen die
Events aus. Ein geleakter manueller `Transaction`-Handle, der ohne
expliziten Commit/Rollback gedroppt wird, emittiert kein Event -
SeaORMs `Drop`-Impl ist synchron und kann den asynchronen Dispatcher
nicht erreichen.

### `QueryExecuted`-Payload

```rust
pub struct QueryExecuted {
    pub sql: String,
    pub bindings: Vec<String>,         // debug-gerendert (`{:?}`)
    pub time: std::time::Duration,
    pub connection_name: String,
    pub read_write_type: Option<ReadWriteType>,
    pub result: Result<(), String>,    // Err bei Treiberfehler
}
```

`to_raw_sql()` setzt die erfassten Bindings zur Anzeige in das SQL
ein:

```rust
let query = /* captured from a listener */;
println!("{}", query.to_raw_sql());
// SELECT * FROM users WHERE id = 42 AND active = true
```

Die Einsetzung ist im **Debug-Format** (kein SQL-sicheres Escaping)
und nur für Log-Ausgaben gedacht. Füttern Sie das Ergebnis niemals
zurück in eine Query.

### Abdeckungsbereich

Heute löst `QueryExecuted` für jede Query aus, die über die
instrumentierten `ExecutorChoice`-Helfer läuft:

- Jeder rohe Helfer auf `DB` (`select` / `select_one` / `scalar` /
  `insert` / `update` / `delete` / `statement` /
  `affecting_statement` / `unprepared`).
- Jede Abschlussmethode auf `DbTableBuilder` (dem modell-losen
  Builder).
- `DB::transaction` / `DB::begin_transaction` lösen BEGIN / COMMIT /
  ROLLBACK als Transaktions-Events aus.
- `DbConnection::connect` löst `ConnectionEstablished` aus.

Das Eloquent-ORM (`Builder<M>::get` / `first` / `count`, Model-CRUD)
passt heute direkt auf die `Tx`- / `Pool`-Arme von `ExecutorChoice`,
statt durch die instrumentierten Helfer zu laufen - das Übernehmen
der Helfer (und damit des Observation-Hooks) landet im
Eloquent-Modul.

## Connection-Metadaten

```rust
let name = DB::database_name()?;        // "myapp" für postgres://.../myapp
let driver = DB::driver_name()?;        // "postgres" | "mysql" | "sqlite"
let title = DB::driver_title()?;        // "Postgres" | "MySQL" | "SQLite"
let version = DB::server_version().await?;  // "15.5" | "8.0.36" | "3.42.0"
```

`server_version` führt eine backend-spezifische
Introspektions-Query aus (`SELECT VERSION()` für Postgres + MySQL,
`SELECT sqlite_version()` für SQLite). Cachen Sie das Ergebnis, wenn
Sie es oft aufrufen - jeder Aufruf ist ein Round-Trip.

## Benannte Connections

Für Read-Replicas, Shards oder Warehouse-Pools pro Model:

```rust
// In bootstrap.rs
DB::register_named("__read_replica__", read_config).await?;
DB::register_named("warehouse", warehouse_config).await?;

// Pro-Query-Routing:
let rows = User::query().on("__read_replica__").get().await?;
let warehouse_rows = DB::table("audit_log").on("warehouse").get().await?;
let raw = DB::select_on("warehouse", "SELECT ...", vec![]).await?;
```

Der Name `__read_replica__` ist wohlbekannt: Ist er registriert,
routet jede lesende Abschlussmethode automatisch darüber. Writes
ignorieren die Replica und zielen auf die primäre Connection.
Verwenden Sie `Builder::on_write_connection` (pro Query) oder
`#[model(connection = "...")]` (Standard pro Model), um für
bestimmte Operationen zur primären zurückzukehren.

Reservierte Namen:

- `__primary__` - der Standard-Pool. Kann nicht registriert werden
  (er ist der Rückgabewert von `DB::connection()`).
- `__read_replica__` - die wohlbekannte Read-Replica. JEDE unter
  diesem Namen registrierte Connection übernimmt das Read-Routing.

Siehe [eloquent.md → Multi-Connection-Routing](eloquent.md#multi-connection-routing) für
die vollständige Vorrangkette (Builder-Tx-Override → umgebende Tx →
Builder-`on(name)` → Model-Standard → `__read_replica__` → primäre).

## Testen

`TestDatabase` baut eine In-Memory-SQLite-Datenbank, registriert sie
im Test-Container, sodass `DB::connection()` sie auflöst, und führt
Ihre Migrationen aus:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn test_user_creation() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // Jeder Code, der DB::connection() aufruft, bekommt jetzt diese
    // In-Memory-DB.
    let _ = CreateUser::run("alice@example.com").await.unwrap();
}

// `test_database!()` ist die Makro-Abkürzung.
let db = test_database!();
```

Für Tests, die ihr eigenes Ad-hoc-Schema bauen:

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").await.unwrap();
```

Wird eine `TestDatabase` gedroppt, wird der Test-Container geleert und
die Connection-Registry gelöscht - keine Cross-Test-Leckage. Tests,
die prozessweiten Zustand verändern (die Registry, die
Listener-Registry, das Query-Log), sollten mit
`#[serial_test::serial]` annotiert werden, damit sie nicht
kollidieren.

## Nächste Schritte

- [Eloquent](eloquent.md) - das typisierte `#[suprnova::model]`-ORM,
  das auf dieser Schicht aufsetzt
- [Migrationen](migrations.md) - `Migrator`, `make:migration` und der
  `db:sync`-Workflow
- [Datenbank-Tests](database-testing.md) - `TestDatabase`,
  Fixture-Laden und die `serial-test`-Annotationen
- [Ereignisse](events.md) - der Dispatcher hinter den Listenern für
  `QueryExecuted` / `TransactionCommitted`
- [Konfiguration](configuration.md) - `DatabaseConfig` neben dem Rest
  Ihrer typisierten Config registrieren

## Oberflächen-Index

| Oberfläche | Laravel-Entsprechung |
| --- | --- |
| `DB::init` / `DB::init_with` / `DB::connection` / `DB::is_connected` / `DB::get` | `DB::connection()` |
| `DB::table(name)` → `DbTableBuilder` | `DB::table($name)` |
| `DB::select` / `select_one` / `scalar` / `insert` / `update` / `delete` / `statement` / `affecting_statement` / `unprepared` | `DB::select` / `selectOne` / `scalar` / `insert` / `update` / `delete` / `statement` / `affectingStatement` / `unprepared` |
| `DB::transaction` / `transaction_with_attempts` / `begin_transaction` | `DB::transaction($cb, $attempts)` / `DB::beginTransaction` |
| `Transaction::commit` / `rollback` / `savepoint` / `rollback_to` | `DB::commit` / `rollBack` / Savepoint-Helfer |
| `DB::listen(callback)` | `DB::listen` |
| `DB::enable_query_log` / `disable_query_log` / `get_query_log` / `flush_query_log` / `logging` | `DB::enableQueryLog` / `disableQueryLog` / `getQueryLog` / `flushQueryLog` / `logging` |
| `DB::database_name` / `driver_name` / `driver_title` / `server_version` | `getDatabaseName` / `getDriverName` / `getDriverTitle` / `getServerVersion` |
| `DB::register_named` / `named` / `select_on` / `table_on` / `statement_on` / `affecting_statement_on` | `DB::connection($name)` für mehrere Connections |
| `QueryExecuted` / `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` / `ConnectionEstablished` / `DatabaseBusy` | `Illuminate\Database\Events\*` |
| `DatabaseConfig::builder()` / `from_env` / `validate_for_environment` / `idle_timeout` / `max_lifetime` / `acquire_timeout` / `test_before_acquire` / `ping_after_idle` | `config/database.php` |
| `TestDatabase::fresh::<M>` / `sqlite_memory` / `execute_unprepared` / `fetch_one` / `fetch_all` | Testing-Trait `RefreshDatabase` |
