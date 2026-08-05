# Base de datos

La capa de base de datos de Suprnova envuelve SeaORM con una fachada `DB` con
forma de Laravel: escapes de consulta en bruto, un constructor de consultas sin
modelo, transacciones con puntos de guardado y reintento ante deadlock, un
registro de conexiones para réplicas de lectura y shards, y una superficie de
observabilidad completa que refleja la API `DB::listen` / `QueryExecuted` /
log de consultas de Laravel 13.

El ORM Eloquent (`use suprnova::eloquent::*`) se construye sobre esta capa y
vive en [eloquent.md](eloquent.md). Cuando se quiera un modelo tipado, ve ahí;
cuando se quiera una consulta en bruto contra una tabla sin modelar o se quiera
observar cada consulta que ejecuta el framework, este es el capítulo.

## Configuración

```rust
use suprnova::{Config, DB, DatabaseConfig};

// En bootstrap.rs
Config::register(DatabaseConfig::from_env());
DB::init().await.expect("DB::init failed");
```

`DatabaseConfig::from_env` lee `DATABASE_URL` y (opcionalmente) los ajustes de
pool `DB_MAX_CONNECTIONS`, `DB_MIN_CONNECTIONS`, `DB_CONNECT_TIMEOUT`,
`DB_LOGGING`. Cuando `DATABASE_URL` no está establecida, la configuración
recurre a `sqlite://./database.db` - conveniente para desarrollo sin
configuración; los arranques de producción rechazan el valor por defecto vía
`validate_for_environment` para que no se pueda desplegar por accidente un
archivo SQLite con `APP_ENV=production`.

URL → detección de driver:

```text
postgres://user:pass@host/db       → DatabaseType::Postgres
postgresql://user:pass@host/db     → DatabaseType::Postgres
mysql://user:pass@host/db          → DatabaseType::Mysql
sqlite://./file.db                 → DatabaseType::Sqlite
sqlite::memory:                    → DatabaseType::Sqlite
```

## Consultas en bruto

La fachada `DB` ofrece toda la superficie de escape en bruto de Laravel 13.
Cada helper pasa por el mismo ejecutor instrumentado - cada llamada dispara
`QueryExecuted` (consulta [Observabilidad](#observabilidad)).

Los bindings son `sea_orm::Value` - uno de los pocos tipos de sea_orm que el
framework deliberadamente NO vuelve a enmascarar, porque cada valor que llega
al wire pasa por él. `Value::from(...)` funciona para cada primitivo que la
base de datos entiende.

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - todas las filas como DynamicRow.
let users = DB::select(
    "SELECT * FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// SELECT - solo la primera fila.
let alice = DB::select_one(
    "SELECT * FROM users WHERE name = ?",
    vec![Value::from("alice")],
).await?;

// SELECT - primera columna de la primera fila como valor tipado.
let count: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users",
    vec![],
).await?;

// INSERT - devuelve bool (true cuando al menos una fila fue afectada).
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - devuelven el conteo de filas afectadas.
let updated = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1)],
).await?;
let deleted = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// Cualquier statement preparado con bindings.
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1), Value::from(42)],
).await?;

// DDL sin bindings - `unprepared` refleja el `DB::unprepared` de Laravel
// para statements (CREATE INDEX, ALTER TABLE, VACUUM) que rechazan el
// binding de placeholders.
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// affecting_statement es la forma explícita que usan internamente
// update/delete - cae directamente a ella para operaciones que no
// encajan en ninguno de los dos nombres (p. ej. INSERT...ON CONFLICT
// DO UPDATE).
let affected = DB::affecting_statement(
    "INSERT INTO users (id, name) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name",
    vec![Value::from(1), Value::from("alice")],
).await?;
```

### Sintaxis de placeholder

`?` para SQLite + MySQL. `$1`, `$2`, ... para Postgres. El backend activo se
autodetecta a partir de `DatabaseConfig::url`.

### DynamicRow

Las filas sin tipo se materializan como `DynamicRow` - un newtype de
`serde_json::Map` con accesores tipados:

```rust
for row in users {
    let id: i64 = row.get_int("id")?;
    let name: String = row.get_string("name")?;
    let nickname: Option<String> = row.get_optional_string("nickname")?;
    let score: Option<i64> = row.get_optional_int("score")?;
    // Deserializa un T arbitrario (chrono::DateTime, un struct propio, etc.):
    let prefs: UserPrefs = row.get_as("prefs")?;
}
```

`get_*` falla cuando la columna está ausente O es nula. `get_optional_*` falla
solo cuando está ausente y devuelve `Ok(None)` para SQL NULL. La lista
completa de accesores es `get_int` / `get_string` / `get_bool` / `get_float` /
`get_value` / `get_as<T>` más `get_optional_string` / `get_optional_int`; para
tipos anulables sin un `get_optional_*` dedicado, recurre a `get_value` + una
comparación con `serde_json::Value`, o a `get_as::<Option<T>>`.

## Constructor de consultas sin modelo - `DB::table`

Para consultas ad hoc contra tablas que no se han modelado con
`#[suprnova::model]`, `DB::table(...)` devuelve un builder encadenable con la
forma del `Builder<M>` de Eloquent, pero que materializa las filas como
`DynamicRow`:

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

### Límite de confianza en los identificadores

Los nombres de tabla, los nombres de columna, las direcciones de ORDER BY, y
los operadores SQL se interpolan DENTRO de la cadena SQL textualmente - NO se
vinculan como parámetros (SQL no permite identificadores vinculados por
placeholder). Trata cada argumento `impl Into<String>` como un literal DE
CONFIANZA:

```rust
// Seguro - el nombre de la columna es una constante.
DB::table("users").filter("email", request.email()).get().await?;

// INSEGURO - nunca inyectes entrada del usuario en un nombre de columna.
DB::table("users").filter(&request.column_name(), value).get().await?;
```

Los valores (el lado derecho de `filter` / `filter_op`) SÍ se vinculan como
parámetros y son seguros para entrada de usuario.

El framework aplica una lista de permitidos estricta sobre los identificadores
(`[A-Za-z_][A-Za-z0-9_]*` con un prefijo `schema.` opcional) y los operadores
(`=`, `<>`, `<`, `<=`, `>`, `>=`, `LIKE`, `NOT LIKE`, `ILIKE`, `NOT ILIKE`,
`IS`, `IS NOT`). Las violaciones fallan en el límite de E/S antes de que se
genere la cadena SQL.

## Transacciones

Tres puntos de entrada, cada uno con los ganchos de observación
`QueryExecuted` / `TransactionBeginning` / `TransactionCommitted` /
`TransactionRolledBack` ya cableados.

### Forma con closure

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

Confirma en `Ok(_)`. Revierte y propaga el error en `Err(_)`.

Las operaciones dentro del closure recogen automáticamente la transacción
activa vía un `tokio::task_local` - no hay que hacer pasar un handle `&tx` por
cada llamada al modelo. Un `DB::transaction` anidado devuelve un error de base
de datos; usa `tx.savepoint(...)` para un comportamiento de reversión anidada.

Para agregados tipados o SQL personalizado que debe ejecutarse sobre la misma
conexión fijada, usa el handle de la transacción directamente:

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

`query_all` dispara observaciones normales de `QueryExecuted` y devuelve filas
`QueryResult` tipadas de SeaORM. Usa `Statement::from_sql_and_values` vinculado
para valores dinámicos; no interpoles entrada no confiable.

### Reintento ante deadlock

```rust
DB::transaction_with_attempts(5, |_tx| {
    Box::pin(async move {
        // Mismo cuerpo de closure que arriba. Se re-ejecuta desde cero
        // ante SQLSTATE 40001 / 40P01 / cualquier error que contenga
        // "deadlock" (sin distinguir mayúsculas/minúsculas).
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

### Forma manual

```rust
use suprnova::{DB, attrs};

let tx = DB::begin_transaction().await?;

// Por modelo: los shims `*_with_tx` fijan una operación CRUD a la tx manual.
User::create_with_tx(&tx, attrs! { name: "alice" }).await?;
Order::create_with_tx(&tx, attrs! { user_id: 1, total: 30 }).await?;

// Por consulta: `Builder::with_tx(&tx)` fija una cadena de builder.
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

El modo manual NO instala el task-local - cada operación que deba correr
dentro de la transacción tiene que optar por ello explícitamente, ya sea vía
`Builder::with_tx(&tx)` sobre una consulta encadenada o uno de los shims
`Model::*_with_tx` (`create_with_tx`, `save_with_tx`, `delete_with_tx`, etc.).
Las operaciones que se olvidan de optar corren contra el pool global y NO
forman parte de la transacción.

Sostener un handle `Transaction` fija una conexión del pool durante toda su
vida; precarga cualquier fila que se necesite leer ANTES de la llamada a
`begin_transaction()`, especialmente en SQLite (conexión única compartida).

### Puntos de guardado

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        Order::create(/* ... */).await?;

        tx.savepoint("after_order").await?;
        if let Err(e) = Payment::charge().await {
            // Descarta el intento de pago pero conserva el pedido.
            tx.rollback_to("after_order").await?;
        }
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

Los tres backends de primera clase soportan `SAVEPOINT` / `ROLLBACK TO
SAVEPOINT` - SQLite incluido.

## Observabilidad

La superficie `DB::listen` / `QueryExecuted` / log de consultas de Laravel 13,
trasladada a Rust a través del despachador de eventos de Suprnova.

### `DB::listen` - callback directo

```rust
use suprnova::{DB, QueryExecuted};

// En bootstrap.rs (o un service provider).
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

Los oyentes corren **de forma síncrona dentro del helper del ejecutor**. Un
oyente lento ralentiza la consulta - mantén los callbacks directos ligeros.
Para cualquier cosa que pueda fallar, prefiere la ruta de `EventFacade` de
abajo; corre a través de `dispatch_best_effort` y tolera errores.

### La ruta de despacho de `EventFacade`

`QueryExecuted` es un `suprnova::Event` real - escucha a través del
despachador para obtener entrega en cola, falsificable, y tolerante a fallos:

```rust
use suprnova::{EventFacade, Listener, QueryExecuted, FrameworkError};
use std::sync::Arc;

struct LogToDatabase;

#[suprnova::async_trait]
impl Listener<QueryExecuted> for LogToDatabase {
    async fn handle(&self, event: &QueryExecuted) -> Result<(), FrameworkError> {
        // Incluso si ESTE oyente consulta la base de datos, la guarda
        // de reentrancia previene la recursión infinita.
        DB::statement(
            "INSERT INTO query_log (sql, time_ms) VALUES (?, ?)",
            vec![event.sql.clone().into(), (event.time.as_millis() as i64).into()],
        ).await?;
        Ok(())
    }
}

// En bootstrap.rs.
EventFacade::listen::<QueryExecuted, _>(Arc::new(LogToDatabase)).await;
```

Los oyentes en esta ruta:

- Corren a través de `dispatch_best_effort` - un oyente que falla NO hace
  fallar la consulta.
- Se cortan en corto cuando ellos mismos emiten una consulta (guarda de
  reentrancia).
- Pueden usar `Event::fake()` en pruebas para afirmar el despacho sin
  ejecutar realmente los oyentes.

### Log de consultas en memoria

```rust
DB::enable_query_log()?;

User::query().filter("active", true).get().await?;
Order::query().count().await?;

let log = DB::get_query_log()?;
for query in &log {
    println!("{} ({}ms)", query.sql, query.time.as_millis());
}

DB::flush_query_log()?;     // descarta las entradas, sigue habilitado
DB::disable_query_log()?;   // deja de capturar
let still_capturing = DB::logging();
```

El log es **ilimitado** - cada consulta capturada lo hace crecer hasta que el
proceso termina, corre `flush_query_log()`, o se llama a
`disable_query_log()`. Úsalo para desarrollo, no como perfilador de
producción de larga duración.

### Eventos del ciclo de vida de la transacción

`TransactionBeginning`, `TransactionCommitted`, y `TransactionRolledBack` son
tipos `suprnova::Event` reales - escúchalos a través de `EventFacade::listen`
para conducir auditoría, bloqueos distribuidos, o lógica de compensación.

```rust
EventFacade::listen::<TransactionCommitted, _>(Arc::new(AuditCommit)).await;
EventFacade::listen::<TransactionRolledBack, _>(Arc::new(MetricRollback)).await;
```

Los tres puntos de entrada de transacción (`DB::transaction` /
`DB::transaction_with_attempts` / `DB::begin_transaction` +
`Transaction::commit`/`rollback`) disparan los eventos. Un handle
`Transaction` manual que se filtra y se descarta sin un commit/rollback
explícito no emite ningún evento - el impl de `Drop` de SeaORM es síncrono y
no puede alcanzar el despachador async.

### El payload de `QueryExecuted`

```rust
pub struct QueryExecuted {
    pub sql: String,
    pub bindings: Vec<String>,         // renderizado en formato debug (`{:?}`)
    pub time: std::time::Duration,
    pub connection_name: String,
    pub read_write_type: Option<ReadWriteType>,
    pub result: Result<(), String>,    // Err ante error de driver
}
```

`to_raw_sql()` sustituye los bindings capturados dentro del SQL para
mostrarlo:

```rust
let query = /* capturado desde un oyente */;
println!("{}", query.to_raw_sql());
// SELECT * FROM users WHERE id = 42 AND active = true
```

La sustitución tiene **formato debug** (no escapado seguro para SQL) y está
pensada solo para salida de log. Nunca reinyectes el resultado en una consulta.

### Alcance de la cobertura

Hoy, `QueryExecuted` se dispara por cada consulta que pasa por los helpers
instrumentados de `ExecutorChoice`:

- Cada helper en bruto sobre `DB` (`select` / `select_one` / `scalar` /
  `insert` / `update` / `delete` / `statement` / `affecting_statement` /
  `unprepared`).
- Cada método terminal sobre `DbTableBuilder` (el builder sin modelo).
- `DB::transaction` / `DB::begin_transaction` BEGIN / COMMIT / ROLLBACK
  disparan eventos de transacción.
- `DbConnection::connect` dispara `ConnectionEstablished`.

El ORM Eloquent (`Builder<M>::get` / `first` / `count`, CRUD de modelo)
coincide hoy directamente con los brazos `Tx` / `Pool` de `ExecutorChoice` en
lugar de llamar a través de los helpers instrumentados - adoptar los helpers
(y por tanto el gancho de observación) llega en el módulo de Eloquent.

## Metadatos de conexión

```rust
let name = DB::database_name()?;        // "myapp" para postgres://.../myapp
let driver = DB::driver_name()?;        // "postgres" | "mysql" | "sqlite"
let title = DB::driver_title()?;        // "Postgres" | "MySQL" | "SQLite"
let version = DB::server_version().await?;  // "15.5" | "8.0.36" | "3.42.0"
```

`server_version` emite una consulta de introspección específica del backend
(`SELECT VERSION()` para Postgres + MySQL, `SELECT sqlite_version()` para
SQLite). Cachea el resultado si se llama con frecuencia - cada llamada es un
viaje de ida y vuelta.

## Conexiones con nombre

Para réplicas de lectura, shards de una base de datos fragmentada, o pools de
almacén de datos por modelo:

```rust
// En bootstrap.rs
DB::register_named("__read_replica__", read_config).await?;
DB::register_named("warehouse", warehouse_config).await?;

// Enrutamiento por consulta:
let rows = User::query().on("__read_replica__").get().await?;
let warehouse_rows = DB::table("audit_log").on("warehouse").get().await?;
let raw = DB::select_on("warehouse", "SELECT ...", vec![]).await?;
```

El nombre `__read_replica__` es conocido: cuando está registrado, cada método
terminal de lectura se enruta a través de ella automáticamente. Las
escrituras ignoran la réplica y apuntan a la primaria. Usa
`Builder::on_write_connection` (por consulta) o
`#[model(connection = "...")]` (por defecto del modelo) para volver a la
primaria en operaciones específicas.

Nombres reservados:

- `__primary__` - el pool por defecto. No se puede registrar (es el valor de
  retorno de `DB::connection()`).
- `__read_replica__` - réplica de lectura conocida. CUALQUIER conexión
  registrada bajo este nombre toma el control del enrutamiento de lectura.

Consulta [eloquent.md → Enrutamiento multiconexión](eloquent.md#multi-connection-routing) para la
cadena de precedencia completa (sobrescritura de tx del builder → tx ambiental
→ `on(name)` del builder → valor por defecto del modelo →
`__read_replica__` → primaria).

## Pruebas

`TestDatabase` construye una base de datos SQLite en memoria, la registra en
el contenedor de pruebas para que `DB::connection()` la resuelva, y ejecuta
las migraciones:

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn test_user_creation() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // Cualquier código que llame a DB::connection() obtiene ahora esta DB en memoria.
    let _ = CreateUser::run("alice@example.com").await.unwrap();
}

// `test_database!()` es el atajo de macro.
let db = test_database!();
```

Para pruebas que construyen su propio esquema ad hoc:

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").await.unwrap();
```

Cuando un `TestDatabase` se descarta, el contenedor de pruebas se limpia y el
registro de conexiones se borra - sin fugas entre pruebas. Las pruebas que
mutan estado global al proceso (el registro, el registro de oyentes, el
log de consultas) deberían anotarse `#[serial_test::serial]` para que no
colisionen.

## Siguiente

- [Eloquent](eloquent.md) - el ORM tipado `#[suprnova::model]` que se sienta
  sobre esta capa
- [Migraciones](migrations.md) - `Migrator`, `make:migration`, y el flujo de
  trabajo `db:sync`
- [Pruebas de base de datos](database-testing.md) - `TestDatabase`, carga de
  fixtures, y anotaciones de prueba serial
- [Eventos](events.md) - el despachador detrás de los oyentes de
  `QueryExecuted` / `TransactionCommitted`
- [Configuración](configuration.md) - registrar `DatabaseConfig` junto al
  resto de la configuración tipada

## Índice de superficie

| Superficie | Análogo en Laravel |
| --- | --- |
| `DB::init` / `DB::init_with` / `DB::connection` / `DB::is_connected` / `DB::get` | `DB::connection()` |
| `DB::table(name)` → `DbTableBuilder` | `DB::table($name)` |
| `DB::select` / `select_one` / `scalar` / `insert` / `update` / `delete` / `statement` / `affecting_statement` / `unprepared` | `DB::select` / `selectOne` / `scalar` / `insert` / `update` / `delete` / `statement` / `affectingStatement` / `unprepared` |
| `DB::transaction` / `transaction_with_attempts` / `begin_transaction` | `DB::transaction($cb, $attempts)` / `DB::beginTransaction` |
| `Transaction::commit` / `rollback` / `savepoint` / `rollback_to` | `DB::commit` / `rollBack` / helpers de savepoint |
| `DB::listen(callback)` | `DB::listen` |
| `DB::enable_query_log` / `disable_query_log` / `get_query_log` / `flush_query_log` / `logging` | `DB::enableQueryLog` / `disableQueryLog` / `getQueryLog` / `flushQueryLog` / `logging` |
| `DB::database_name` / `driver_name` / `driver_title` / `server_version` | `getDatabaseName` / `getDriverName` / `getDriverTitle` / `getServerVersion` |
| `DB::register_named` / `named` / `select_on` / `table_on` / `statement_on` / `affecting_statement_on` | `DB::connection($name)` multiconexión |
| `QueryExecuted` / `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` / `ConnectionEstablished` / `DatabaseBusy` | `Illuminate\Database\Events\*` |
| `DatabaseConfig::builder()` / `from_env` / `validate_for_environment` | `config/database.php` |
| `TestDatabase::fresh::<M>` / `sqlite_memory` / `execute_unprepared` / `fetch_one` / `fetch_all` | trait de pruebas `RefreshDatabase` |
