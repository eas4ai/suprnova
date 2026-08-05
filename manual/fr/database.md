# Base de données

La couche base de données de Suprnova enveloppe SeaORM avec une
façade `DB` à la forme Laravel : des escapes de requête brute, un
générateur de requêtes sans modèle, des transactions avec savepoints
et réessai sur deadlock, un registre de connexions pour les répliques
de lecture et les shards, et une surface d'observabilité complète qui
reflète l'API `DB::listen` / `QueryExecuted` / journal de requêtes de
Laravel 13.

L'ORM Eloquent (`use suprnova::eloquent::*`) se construit par-dessus
cette couche et vit dans [eloquent.md](eloquent.md). Quand vous
voulez un modèle typé, allez là-bas ; quand vous voulez une requête
brute contre une table non modélisée, ou observer chaque requête que
le framework exécute, c'est ici la bonne page.

## Configuration

```rust
use suprnova::{Config, DB, DatabaseConfig};

// Dans bootstrap.rs
Config::register(DatabaseConfig::from_env());
DB::init().await.expect("DB::init failed");
```

`DatabaseConfig::from_env` lit `DATABASE_URL` et (facultativement) les
réglages de pool `DB_MAX_CONNECTIONS`, `DB_MIN_CONNECTIONS`,
`DB_CONNECT_TIMEOUT`, `DB_LOGGING`. Quand `DATABASE_URL` n'est pas
défini, la config retombe sur `sqlite://./database.db` - pratique
pour un développement sans configuration ; les démarrages de
production refusent ce repli via `validate_for_environment`, pour que
vous ne puissiez pas accidentellement livrer un fichier SQLite en
`APP_ENV=production`.

Détection du driver depuis l'URL :

```text
postgres://user:pass@host/db       → DatabaseType::Postgres
postgresql://user:pass@host/db     → DatabaseType::Postgres
mysql://user:pass@host/db          → DatabaseType::Mysql
sqlite://./file.db                 → DatabaseType::Sqlite
sqlite::memory:                    → DatabaseType::Sqlite
```

## Requêtes brutes

La façade `DB` livre la surface complète des escapes bruts de
Laravel 13. Chaque helper passe par le même exécuteur instrumenté -
chaque appel déclenche `QueryExecuted` (voir
[Observabilité](#observabilité)).

Les bindings sont des `sea_orm::Value` - l'un des rares types sea_orm
que le framework NE remasque PAS intentionnellement, parce que chaque
valeur qui atteint le réseau y passe. `Value::from(...)` fonctionne
pour chaque primitif que la base de données comprend.

```rust
use suprnova::DB;
use sea_orm::Value;

// SELECT - toutes les lignes en DynamicRow.
let users = DB::select(
    "SELECT * FROM users WHERE active = ?",
    vec![Value::from(true)],
).await?;

// SELECT - première ligne seulement.
let alice = DB::select_one(
    "SELECT * FROM users WHERE name = ?",
    vec![Value::from("alice")],
).await?;

// SELECT - première colonne de la première ligne comme valeur
// typée.
let count: i64 = DB::scalar(
    "SELECT COUNT(*) FROM users",
    vec![],
).await?;

// INSERT - retourne bool (true quand au moins une ligne a été
// affectée).
DB::insert(
    "INSERT INTO users (name, active) VALUES (?, ?)",
    vec![Value::from("bob"), Value::from(true)],
).await?;

// UPDATE / DELETE - retournent le compte de lignes affectées.
let updated = DB::update(
    "UPDATE users SET active = ? WHERE id = ?",
    vec![Value::from(false), Value::from(1)],
).await?;
let deleted = DB::delete(
    "DELETE FROM users WHERE active = ?",
    vec![Value::from(false)],
).await?;

// N'importe quelle instruction préparée avec des bindings.
DB::statement(
    "UPDATE users SET votes = votes + ? WHERE id = ?",
    vec![Value::from(1), Value::from(42)],
).await?;

// DDL sans binding - `unprepared` reflète le `DB::unprepared` de
// Laravel pour les instructions (CREATE INDEX, ALTER TABLE, VACUUM)
// qui rejettent la liaison par placeholder.
DB::unprepared("CREATE INDEX idx_users_name ON users(name)").await?;

// affecting_statement est la forme explicite utilisée en interne
// par update/delete - redescendez-y directement pour les opérations
// qui n'entrent dans aucun des deux noms (ex.
// INSERT...ON CONFLICT DO UPDATE).
let affected = DB::affecting_statement(
    "INSERT INTO users (id, name) VALUES (?, ?) ON CONFLICT(id) DO UPDATE SET name = excluded.name",
    vec![Value::from(1), Value::from("alice")],
).await?;
```

### Syntaxe des placeholders

`?` pour SQLite + MySQL. `$1`, `$2`, ... pour Postgres. Le backend
actif est détecté automatiquement depuis `DatabaseConfig::url`.

### DynamicRow

Les lignes non typées se matérialisent en `DynamicRow` - un newtype
`serde_json::Map` avec des accesseurs typés :

```rust
for row in users {
    let id: i64 = row.get_int("id")?;
    let name: String = row.get_string("name")?;
    let nickname: Option<String> = row.get_optional_string("nickname")?;
    let score: Option<i64> = row.get_optional_int("score")?;
    // Désérialise un T arbitraire (chrono::DateTime, votre propre
    // struct, etc.) :
    let prefs: UserPrefs = row.get_as("prefs")?;
}
```

`get_*` échoue en erreur quand la colonne est absente OU null.
`get_optional_*` échoue seulement en cas d'absence et retourne
`Ok(None)` pour un NULL SQL. La liste complète des accesseurs est
`get_int` / `get_string` / `get_bool` / `get_float` / `get_value` /
`get_as<T>` plus `get_optional_string` / `get_optional_int` ; pour les
types nullables sans `get_optional_*` dédié, tournez-vous vers
`get_value` + un match sur `serde_json::Value`, ou vers
`get_as::<Option<T>>`.

## Générateur de requêtes sans modèle - `DB::table`

Pour les requêtes ad hoc contre des tables que vous n'avez pas pris
la peine de modéliser avec `#[suprnova::model]`, `DB::table(...)`
retourne un builder chaînable à la forme du `Builder<M>` Eloquent,
mais matérialisant les lignes en `DynamicRow` :

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

### Frontière de confiance des identifiants

Les noms de table, les noms de colonne, les directions ORDER BY, et
les opérateurs SQL sont interpolés TELS QUELS dans la chaîne SQL - ils
ne sont PAS liés comme des paramètres (SQL n'autorise pas les
identifiants liés par placeholder). Traitez chaque argument
`impl Into<String>` comme un littéral DE CONFIANCE :

```rust
// Sûr - le nom de colonne est une constante.
DB::table("users").filter("email", request.email()).get().await?;

// DANGEREUX - n'épissez jamais une entrée utilisateur dans un nom
// de colonne.
DB::table("users").filter(&request.column_name(), value).get().await?;
```

Les valeurs (le membre de droite de `filter` / `filter_op`) SONT
liées comme des paramètres et sûres pour une entrée utilisateur.

Le framework impose une allowlist stricte sur les identifiants
(`[A-Za-z_][A-Za-z0-9_]*` avec un préfixe `schema.` optionnel) et sur
les opérateurs (`=`, `<>`, `<`, `<=`, `>`, `>=`, `LIKE`, `NOT LIKE`,
`ILIKE`, `NOT ILIKE`, `IS`, `IS NOT`). Les violations font erreur à la
frontière d'E/S avant que la chaîne SQL ne soit rendue.

## Transactions

Trois points d'entrée, chacun avec les hooks d'observation
`QueryExecuted` / `TransactionBeginning` / `TransactionCommitted` /
`TransactionRolledBack` câblés.

### Forme closure

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

Commit sur `Ok(_)`. Rollback + propage l'erreur sur `Err(_)`.

Les opérations à l'intérieur de la closure récupèrent automatiquement
la transaction active via un `tokio::task_local` - vous n'avez PAS à
faire circuler un handle `&tx` à travers chaque appel de modèle. Un
`DB::transaction` imbriqué retourne une erreur de base de données ;
utilisez `tx.savepoint(...)` pour un comportement de rollback
imbriqué.

Pour un agrégat typé ou du SQL personnalisé qui doit s'exécuter sur la
même connexion épinglée, utilisez directement le handle de
transaction :

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

`query_all` émet des observations `QueryExecuted` normales et
retourne des lignes `QueryResult` SeaORM typées. Utilisez
`Statement::from_sql_and_values` liée pour les valeurs dynamiques ;
n'interpolez pas d'entrée non fiable.

### Réessai sur deadlock

```rust
DB::transaction_with_attempts(5, |_tx| {
    Box::pin(async move {
        // Même corps de closure que ci-dessus. Relance depuis zéro
        // sur SQLSTATE 40001 / 40P01 / toute erreur contenant
        // « deadlock » (insensible à la casse).
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

### Forme manuelle

```rust
use suprnova::{DB, attrs};

let tx = DB::begin_transaction().await?;

// Par modèle : les shims `*_with_tx` épinglent une opération CRUD
// à la tx manuelle.
User::create_with_tx(&tx, attrs! { name: "alice" }).await?;
Order::create_with_tx(&tx, attrs! { user_id: 1, total: 30 }).await?;

// Par requête : `Builder::with_tx(&tx)` épingle une chaîne de
// builder.
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

Le mode manuel n'installe PAS le task-local - chaque opération qui
devrait s'exécuter à l'intérieur de la transaction doit y adhérer
explicitement, soit via `Builder::with_tx(&tx)` sur une requête
chaînée, soit via l'un des shims `Model::*_with_tx` (`create_with_tx`,
`save_with_tx`, `delete_with_tx`, etc.). Les opérations qui oublient
d'y adhérer s'exécutent contre le pool global et NE font PAS partie de
la transaction.

Détenir un handle `Transaction` épingle une connexion du pool pour
toute sa durée de vie ; préchargez toute ligne dont vous avez besoin
AVANT l'appel à `begin_transaction()`, en particulier sur SQLite
(connexion unique partagée).

### Savepoints

```rust
DB::transaction(|tx| {
    Box::pin(async move {
        Order::create(/* ... */).await?;

        tx.savepoint("after_order").await?;
        if let Err(e) = Payment::charge().await {
            // Abandonne la tentative de paiement mais garde la
            // commande.
            tx.rollback_to("after_order").await?;
        }
        Ok::<(), suprnova::FrameworkError>(())
    })
}).await?;
```

Les trois backends de premier rang prennent en charge `SAVEPOINT` /
`ROLLBACK TO SAVEPOINT` - SQLite compris.

## Observabilité

La surface `DB::listen` / `QueryExecuted` / journal de requêtes de
Laravel 13, portée en Rust à travers le dispatcher d'événements de
Suprnova.

### `DB::listen` - callback direct

```rust
use suprnova::{DB, QueryExecuted};

// Dans bootstrap.rs (ou un fournisseur de service).
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

Les écouteurs s'exécutent **de façon synchrone à l'intérieur du
helper d'exécution**. Un écouteur lent ralentit la requête - gardez
les callbacks directs légers. Pour tout ce qui peut échouer, préférez
le chemin `EventFacade` ci-dessous ; il passe par
`dispatch_best_effort` et tolère les erreurs.

### Chemin de dispatch `EventFacade`

`QueryExecuted` est un vrai `suprnova::Event` - écoutez à travers le
dispatcher pour obtenir une livraison mise en file d'attente,
fakeable, et tolérante aux pannes :

```rust
use suprnova::{EventFacade, Listener, QueryExecuted, FrameworkError};
use std::sync::Arc;

struct LogToDatabase;

#[suprnova::async_trait]
impl Listener<QueryExecuted> for LogToDatabase {
    async fn handle(&self, event: &QueryExecuted) -> Result<(), FrameworkError> {
        // Même si CET écouteur interroge la base de données, le
        // garde-fou de réentrance empêche une récursion infinie.
        DB::statement(
            "INSERT INTO query_log (sql, time_ms) VALUES (?, ?)",
            vec![event.sql.clone().into(), (event.time.as_millis() as i64).into()],
        ).await?;
        Ok(())
    }
}

// Dans bootstrap.rs.
EventFacade::listen::<QueryExecuted, _>(Arc::new(LogToDatabase)).await;
```

Les écouteurs sur ce chemin :

- S'exécutent via `dispatch_best_effort` - un écouteur qui échoue NE
  fait PAS échouer la requête.
- Sont court-circuités quand ils émettent eux-mêmes une requête
  (garde-fou de réentrance).
- Peuvent utiliser `Event::fake()` dans les tests pour affirmer le
  dispatch sans réellement exécuter les écouteurs.

### Journal de requêtes en mémoire

```rust
DB::enable_query_log()?;

User::query().filter("active", true).get().await?;
Order::query().count().await?;

let log = DB::get_query_log()?;
for query in &log {
    println!("{} ({}ms)", query.sql, query.time.as_millis());
}

DB::flush_query_log()?;     // supprime les entrées, reste activé
DB::disable_query_log()?;   // arrête la capture
let still_capturing = DB::logging();
```

Le journal est **non borné** - chaque requête capturée le fait
grossir jusqu'à ce que le processus se termine, que
`flush_query_log()` s'exécute, ou que `disable_query_log()` soit
appelée. Utilisez-le pour le développement, pas comme profileur de
production de longue durée.

### Événements de cycle de vie de transaction

`TransactionBeginning`, `TransactionCommitted`, et
`TransactionRolledBack` sont de vrais types `suprnova::Event` -
écoutez-les via `EventFacade::listen` pour piloter l'audit, des
verrous distribués, ou une logique de compensation.

```rust
EventFacade::listen::<TransactionCommitted, _>(Arc::new(AuditCommit)).await;
EventFacade::listen::<TransactionRolledBack, _>(Arc::new(MetricRollback)).await;
```

Les trois points d'entrée de transaction (`DB::transaction` /
`DB::transaction_with_attempts` / `DB::begin_transaction` +
`Transaction::commit`/`rollback`) déclenchent les événements. Un
handle `Transaction` manuel qui fuite et se droppe sans
commit/rollback explicite n'émet aucun événement - l'impl `Drop` de
SeaORM est synchrone et ne peut pas atteindre le dispatcher async.

### Payload de `QueryExecuted`

```rust
pub struct QueryExecuted {
    pub sql: String,
    pub bindings: Vec<String>,         // rendu en debug (`{:?}`)
    pub time: std::time::Duration,
    pub connection_name: String,
    pub read_write_type: Option<ReadWriteType>,
    pub result: Result<(), String>,    // Err en cas d'erreur driver
}
```

`to_raw_sql()` substitue les bindings capturés dans le SQL pour
l'affichage :

```rust
let query = /* capturé depuis un écouteur */;
println!("{}", query.to_raw_sql());
// SELECT * FROM users WHERE id = 42 AND active = true
```

La substitution est au **format debug** (pas un échappement sûr pour
SQL) et n'est destinée qu'à la sortie de journal. Ne réinjectez
jamais le résultat dans une requête.

### Portée de couverture

Aujourd'hui, `QueryExecuted` se déclenche pour chaque requête qui
passe par les helpers instrumentés `ExecutorChoice` :

- Chaque helper brut sur `DB` (`select` / `select_one` / `scalar` /
  `insert` / `update` / `delete` / `statement` /
  `affecting_statement` / `unprepared`).
- Chaque méthode terminale sur `DbTableBuilder` (le builder sans
  modèle).
- `DB::transaction` / `DB::begin_transaction` BEGIN / COMMIT /
  ROLLBACK déclenchent des événements de transaction.
- `DbConnection::connect` déclenche `ConnectionEstablished`.

L'ORM Eloquent (`Builder<M>::get` / `first` / `count`, le CRUD de
modèle) fait aujourd'hui correspondance directe avec les branches
`Tx` / `Pool` d'`ExecutorChoice` plutôt que d'appeler à travers les
helpers instrumentés - adopter les helpers (et donc le hook
d'observation) est à faire dans le module Eloquent.

## Métadonnées de connexion

```rust
let name = DB::database_name()?;        // "myapp" pour postgres://.../myapp
let driver = DB::driver_name()?;        // "postgres" | "mysql" | "sqlite"
let title = DB::driver_title()?;        // "Postgres" | "MySQL" | "SQLite"
let version = DB::server_version().await?;  // "15.5" | "8.0.36" | "3.42.0"
```

`server_version` émet une requête d'introspection propre au backend
(`SELECT VERSION()` pour Postgres + MySQL, `SELECT sqlite_version()`
pour SQLite). Mettez le résultat en cache si vous l'appelez souvent -
chaque appel est un aller-retour.

## Connexions nommées

Pour les répliques de lecture, les shards, ou les pools d'entrepôt par
modèle :

```rust
// Dans bootstrap.rs
DB::register_named("__read_replica__", read_config).await?;
DB::register_named("warehouse", warehouse_config).await?;

// Routage par requête :
let rows = User::query().on("__read_replica__").get().await?;
let warehouse_rows = DB::table("audit_log").on("warehouse").get().await?;
let raw = DB::select_on("warehouse", "SELECT ...", vec![]).await?;
```

Le nom `__read_replica__` est bien connu : quand elle est enregistrée,
chaque méthode terminale de forme lecture s'y route automatiquement.
Les écritures ignorent la réplique et ciblent la primaire. Utilisez
`Builder::on_write_connection` (par requête) ou
`#[model(connection = "...")]` (par défaut de modèle) pour revenir à
la primaire pour des opérations spécifiques.

Noms réservés :

- `__primary__` - le pool par défaut. Ne peut pas être enregistré
  (c'est la valeur de retour de `DB::connection()`).
- `__read_replica__` - réplique de lecture bien connue. TOUTE
  connexion enregistrée sous ce nom prend le contrôle du routage de
  lecture.

Voir [eloquent.md → Routage multi-connexion](eloquent.md#multi-connection-routing)
pour la chaîne de priorité complète (redéfinition tx du builder → tx
ambiante → `on(name)` du builder → défaut du modèle →
`__read_replica__` → primaire).

## Tests

`TestDatabase` construit une base de données SQLite en mémoire,
l'enregistre dans le conteneur de test pour que `DB::connection()` s'y
résolve, et exécute vos migrations :

```rust
use suprnova::testing::TestDatabase;
use crate::migrations::Migrator;

#[tokio::test]
async fn test_user_creation() {
    let db = TestDatabase::fresh::<Migrator>().await.unwrap();
    // Tout code appelant DB::connection() obtient maintenant cette
    // BD en mémoire.
    let _ = CreateUser::run("alice@example.com").await.unwrap();
}

// `test_database!()` est le raccourci en macro.
let db = test_database!();
```

Pour les tests qui construisent leur propre schéma ad hoc :

```rust
let db = TestDatabase::sqlite_memory().await.unwrap();
db.execute_unprepared("CREATE TABLE t (id INTEGER PRIMARY KEY, name TEXT)").await.unwrap();
```

Quand un `TestDatabase` est droppé, le conteneur de test est vidé et
le registre de connexions est effacé - aucune fuite entre les tests.
Les tests qui mutent l'état global au processus (le registre, le
registre des écouteurs, le journal de requêtes) devraient être
annotés `#[serial_test::serial]` pour qu'ils n'entrent pas en
collision.

## Suivant

- [Eloquent](eloquent.md) - l'ORM `#[suprnova::model]` typé qui
  repose sur cette couche
- [Migrations](migrations.md) - `Migrator`, `make:migration`, et le
  flux de travail `db:sync`
- [Tests de base de données](database-testing.md) - `TestDatabase`,
  le chargement de fixture, et les annotations de test sériel
- [Événements](events.md) - le dispatcher derrière les écouteurs de
  `QueryExecuted` / `TransactionCommitted`
- [Configuration](configuration.md) - enregistrer `DatabaseConfig`
  à côté du reste de votre config typée

## Index de la surface

| Surface | Équivalent Laravel |
| --- | --- |
| `DB::init` / `DB::init_with` / `DB::connection` / `DB::is_connected` / `DB::get` | `DB::connection()` |
| `DB::table(name)` → `DbTableBuilder` | `DB::table($name)` |
| `DB::select` / `select_one` / `scalar` / `insert` / `update` / `delete` / `statement` / `affecting_statement` / `unprepared` | `DB::select` / `selectOne` / `scalar` / `insert` / `update` / `delete` / `statement` / `affectingStatement` / `unprepared` |
| `DB::transaction` / `transaction_with_attempts` / `begin_transaction` | `DB::transaction($cb, $attempts)` / `DB::beginTransaction` |
| `Transaction::commit` / `rollback` / `savepoint` / `rollback_to` | `DB::commit` / `rollBack` / helpers de savepoint |
| `DB::listen(callback)` | `DB::listen` |
| `DB::enable_query_log` / `disable_query_log` / `get_query_log` / `flush_query_log` / `logging` | `DB::enableQueryLog` / `disableQueryLog` / `getQueryLog` / `flushQueryLog` / `logging` |
| `DB::database_name` / `driver_name` / `driver_title` / `server_version` | `getDatabaseName` / `getDriverName` / `getDriverTitle` / `getServerVersion` |
| `DB::register_named` / `named` / `select_on` / `table_on` / `statement_on` / `affecting_statement_on` | `DB::connection($name)` multi-connexion |
| `QueryExecuted` / `TransactionBeginning` / `TransactionCommitted` / `TransactionRolledBack` / `ConnectionEstablished` / `DatabaseBusy` | `Illuminate\Database\Events\*` |
| `DatabaseConfig::builder()` / `from_env` / `validate_for_environment` | `config/database.php` |
| `TestDatabase::fresh::<M>` / `sqlite_memory` / `execute_unprepared` / `fetch_one` / `fetch_all` | trait de test `RefreshDatabase` |
