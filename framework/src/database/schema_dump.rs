//! `schema:dump`: the database's schema and its migration ledger in one
//! file, loaded into an empty database before newer migrations run, so a
//! long migration history can give way to a snapshot. Laravel's
//! `schema:dump` and its `SchemaState` classes, which run the same tools.

use crate::error::FrameworkError;
use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement, TransactionTrait};
use sea_orm_migration::prelude::*;
use std::marker::PhantomData;
use std::path::{Path, PathBuf};
use std::process::Stdio;

/// The comment that opens the ledger rows in a dump, which `prune` reads.
const LEDGER_MARKER: &str = "-- Suprnova migration ledger";

/// The directory the dumps live in, under the database path.
const SCHEMA_DIRECTORY: &str = "database/schema";

/// Dump, load and prune a schema snapshot. The app binary's
/// `schema:dump`, `migrate` and `migrate:fresh` run these, and so can your
/// own code.
///
/// ```no_run
/// # async fn run() -> Result<(), suprnova::FrameworkError> {
/// # struct Migrator;
/// # #[suprnova::async_trait]
/// # impl sea_orm_migration::MigratorTrait for Migrator {
/// #     fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> { vec![] }
/// # }
/// use suprnova::SchemaDump;
///
/// let url = "postgres://app:secret@127.0.0.1/app";
/// let path = SchemaDump::default_path(url).await?;
/// SchemaDump::dump::<Migrator>(url, &path).await?;
/// # Ok(()) }
/// ```
pub struct SchemaDump;

impl SchemaDump {
    /// Where the dump for the database at `url` lives by default:
    /// `database/schema/<engine>-schema.sql`, the engine being `sqlite`,
    /// `postgres`, `mysql` or `mariadb`. A `mysql://` URL is asked which
    /// server it reaches, since MySQL and MariaDB share it.
    pub async fn default_path(url: &str) -> Result<PathBuf, FrameworkError> {
        let engine = match Engine::from_scheme(url)? {
            Some(engine) => engine,
            None => Engine::of(&connect(url).await?).await?,
        };
        Ok(engine.default_path())
    }

    /// Writes the schema of the database at `url` to `path`: the
    /// statements that create its tables, indexes, views and constraints,
    /// without their rows, then one `INSERT` for each row of `M`'s ledger.
    /// Postgres is dumped with `pg_dump`, MySQL with `mysqldump` and
    /// MariaDB with `mariadb-dump`, each found on `PATH` and given the
    /// password through its environment or a private option file; SQLite
    /// is read through its own connection. The file is replaced only when
    /// the dump succeeds.
    pub async fn dump<M: MigratorTrait>(url: &str, path: &Path) -> Result<(), FrameworkError> {
        let known = Engine::from_scheme(url)?;
        // A missing tool is reported before any connection is tried.
        if let Some(engine) = known
            && let Some(tool) = engine.dump_tool()
        {
            find_tool(tool)?;
        }
        let db = connect(url).await?;
        let engine = match known {
            Some(engine) => engine,
            None => Engine::of(&db).await?,
        };
        let mut sql = match engine {
            Engine::Sqlite => sqlite_schema(&db).await?,
            Engine::Postgres => postgres_schema(url).await?,
            Engine::Mysql | Engine::Mariadb => mysql_schema(url, engine).await?,
        };
        sql.push_str(&ledger_inserts::<M>(&db, engine).await?);
        write_replacing(path, &sql)
    }

    /// Loads the dump at `path` into the database at `url`: Postgres with
    /// `psql` in one transaction, MySQL with `mysql`, MariaDB with
    /// `mariadb`, and SQLite through its own connection in one
    /// transaction. It stops at the first statement that fails.
    pub async fn load(url: &str, path: &Path) -> Result<(), FrameworkError> {
        let db = connect(url).await?;
        let engine = Engine::of(&db).await?;
        load_into(url, &db, engine, path).await
    }

    /// `M::up`, after loading the dump when `M`'s ledger records no
    /// migration: the file `schema` names, or else the engine's default
    /// dump when it exists. Returns the file it loaded. A database whose
    /// ledger records a migration is never loaded; nor is one that holds
    /// tables while its ledger is missing or empty, which is an error, as
    /// is a `schema` that names no file. A failed load runs no migration.
    pub async fn migrate<M: MigratorTrait>(
        url: &str,
        schema: Option<&Path>,
    ) -> Result<Option<PathBuf>, FrameworkError> {
        let (db, loaded) = Self::prepare::<M>(url, schema).await?;
        M::up(&db, None).await.map_err(migration_failed)?;
        Ok(loaded)
    }

    /// The connection and the load [`migrate`](Self::migrate) makes before
    /// it runs `M::up`, for a caller that runs the migrations itself.
    pub(crate) async fn prepare<M: MigratorTrait>(
        url: &str,
        schema: Option<&Path>,
    ) -> Result<(DatabaseConnection, Option<PathBuf>), FrameworkError> {
        require_named_file(schema)?;
        let db = connect(url).await?;
        let loaded = load_when_empty::<M>(url, &db, schema).await?;
        Ok((db, loaded))
    }

    /// `M::fresh` with the dump: every table dropped, the dump loaded as
    /// [`migrate`](Self::migrate) loads it, then `M::up`. When there is a
    /// dump to load, the views, routines, sequences and types the dump
    /// would create again are dropped too.
    pub async fn fresh<M: MigratorTrait>(
        url: &str,
        schema: Option<&Path>,
    ) -> Result<Option<PathBuf>, FrameworkError> {
        require_named_file(schema)?;
        let db = connect(url).await?;
        let engine = Engine::of(&db).await?;
        let dump = schema.map_or_else(|| engine.default_path(), Path::to_path_buf);
        Emptied::<M>::fresh(&db).await.map_err(migration_failed)?;
        if dump.is_file() {
            drop_the_rest(&db, engine).await?;
        }
        let loaded = load_when_empty::<M>(url, &db, schema).await?;
        M::up(&db, None).await.map_err(migration_failed)?;
        Ok(loaded)
    }

    /// [`dump`](Self::dump), then [`prune`](Self::prune) the migrations
    /// in `migrations` the dump records. A dump that fails prunes nothing.
    pub async fn dump_and_prune<M: MigratorTrait>(
        url: &str,
        path: &Path,
        migrations: &Path,
    ) -> Result<Vec<String>, FrameworkError> {
        Self::dump::<M>(url, path).await?;
        Self::prune(migrations, path)
    }

    /// Prunes the migrations in `migrations` (a project's `src/migrations`)
    /// whose names the ledger rows of the dump at `dump` record: deletes
    /// each one's file (or directory module), removes its `mod` line from
    /// `mod.rs`, and puts [`PrunedMigration`] in place of its entry in the
    /// list, so the name stays without the code. A migration the dump does
    /// not record stays, and so does a name an earlier prune kept. Returns
    /// the pruned names. Nothing changes when a recorded name has neither a
    /// file nor a `PrunedMigration` entry, as for a migration whose
    /// `name()` differs from its file name, or when a pruned migration is
    /// not declared and listed the way `suprnova make:migration` writes it.
    pub fn prune(migrations: &Path, dump: &Path) -> Result<Vec<String>, FrameworkError> {
        let sql = read(dump)?;
        let module_path = migrations.join("mod.rs");
        let module = read(&module_path)?;
        let mut lines: Vec<String> = module.lines().map(str::to_owned).collect();
        let mut pruned = Vec::new();
        for name in ledger_versions(&sql) {
            let identifier =
                !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            let marker = format!("Box::new(suprnova::PrunedMigration::new(\"{name}\"))");
            if identifier && lines.iter().any(|line| line.contains(&marker)) {
                continue;
            }
            let Some(code) = identifier
                .then(|| migration_code(migrations, &name))
                .flatten()
            else {
                return Err(FrameworkError::internal(format!(
                    "the dump records `{name}`, but {} has no file of that name and mod.rs \
                     does not list it as pruned; a migration whose name() differs from its \
                     file name cannot be pruned for you, so nothing was pruned",
                    migrations.display()
                )));
            };
            let declarations = [
                format!("mod {name};"),
                format!("pub mod {name};"),
                format!("pub(crate) mod {name};"),
            ];
            let before = lines.len();
            lines.retain(|line| !declarations.iter().any(|d| line.trim() == d));
            let entry = format!("Box::new({name}::Migration)");
            let declared = lines.len() < before;
            let listed = lines.iter().any(|line| line.contains(&entry));
            if !declared || !listed {
                return Err(FrameworkError::internal(format!(
                    "`{name}` is in {} but mod.rs does not declare and list it the way \
                     `suprnova make:migration` writes it; nothing was pruned",
                    migrations.display()
                )));
            }
            for line in &mut lines {
                *line = line.replace(&entry, &marker);
            }
            pruned.push((name, code));
        }
        if pruned.is_empty() {
            return Ok(Vec::new());
        }
        let mut text = lines.join("\n");
        if module.ends_with('\n') {
            text.push('\n');
        }
        write(&module_path, &text)?;
        for (_, code) in &pruned {
            let removed = if code.is_dir() {
                std::fs::remove_dir_all(code)
            } else {
                std::fs::remove_file(code)
            };
            removed.map_err(|e| {
                FrameworkError::internal(format!("could not delete {}: {e}", code.display()))
            })?;
        }
        Ok(pruned.into_iter().map(|(name, _)| name).collect())
    }
}

/// The code of the migration called `name` in `migrations`: its file, or
/// its directory module.
fn migration_code(migrations: &Path, name: &str) -> Option<PathBuf> {
    let file = migrations.join(format!("{name}.rs"));
    if file.is_file() {
        return Some(file);
    }
    let directory = migrations.join(name);
    directory.join("mod.rs").is_file().then_some(directory)
}

/// A schema path the caller named must name a file.
fn require_named_file(schema: Option<&Path>) -> Result<(), FrameworkError> {
    match schema {
        Some(path) if !path.is_file() => Err(FrameworkError::database(format!(
            "the schema dump {} does not exist",
            path.display()
        ))),
        _ => Ok(()),
    }
}

/// A migration whose code was pruned into a schema dump: its name stays in
/// the Migrator's list so a database whose ledger records it migrates on,
/// while a database that would have to run it fails instead of skipping
/// its schema. `schema:dump --prune` writes these; you rarely write one.
///
/// ```
/// let migration = suprnova::PrunedMigration::new("m20260101_000001_create_users");
/// ```
pub struct PrunedMigration {
    name: &'static str,
}

impl PrunedMigration {
    /// The pruned migration called `name`, as its ledger row records it.
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }
}

impl MigrationName for PrunedMigration {
    fn name(&self) -> &str {
        self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for PrunedMigration {
    async fn up(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(format!(
            "`{}` was pruned into a schema dump in {SCHEMA_DIRECTORY}, so its code is gone; \
             an empty database loads that dump before it migrates, so add the dump for \
             this database's engine there",
            self.name
        )))
    }

    async fn down(&self, _manager: &SchemaManager) -> Result<(), DbErr> {
        Err(DbErr::Custom(format!(
            "`{}` was pruned into a schema dump in {SCHEMA_DIRECTORY} and cannot be rolled back",
            self.name
        )))
    }
}

/// `M`'s ledger with no migrations: its `fresh` drops every table and
/// leaves `M`'s ledger empty.
struct Emptied<M>(PhantomData<M>);

#[async_trait::async_trait]
impl<M: MigratorTrait> MigratorTrait for Emptied<M> {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        Vec::new()
    }

    fn migration_table_name() -> DynIden {
        M::migration_table_name()
    }
}

/// The engines a dump knows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Engine {
    Sqlite,
    Postgres,
    Mysql,
    Mariadb,
}

impl Engine {
    /// The engine a URL names without asking the server; `None` for a
    /// `mysql://` URL, which MySQL and MariaDB share.
    fn from_scheme(url: &str) -> Result<Option<Self>, FrameworkError> {
        match url.split(':').next().unwrap_or("") {
            "sqlite" => Ok(Some(Engine::Sqlite)),
            "postgres" | "postgresql" => Ok(Some(Engine::Postgres)),
            "mariadb" => Ok(Some(Engine::Mariadb)),
            "mysql" => Ok(None),
            other => Err(FrameworkError::database(format!(
                "a schema dump knows SQLite, Postgres, MySQL and MariaDB, not `{other}`"
            ))),
        }
    }

    /// The engine behind a connection.
    async fn of(db: &DatabaseConnection) -> Result<Self, FrameworkError> {
        match db.get_database_backend() {
            DbBackend::Sqlite => Ok(Engine::Sqlite),
            DbBackend::Postgres => Ok(Engine::Postgres),
            _ => {
                let row = db
                    .query_one_raw(Statement::from_string(
                        DbBackend::MySql,
                        "SELECT VERSION() AS version",
                    ))
                    .await
                    .map_err(database_error)?;
                let version: String = row
                    .map(|row| row.try_get("", "version"))
                    .transpose()
                    .map_err(database_error)?
                    .unwrap_or_default();
                Ok(if version.to_ascii_lowercase().contains("mariadb") {
                    Engine::Mariadb
                } else {
                    Engine::Mysql
                })
            }
        }
    }

    fn name(self) -> &'static str {
        match self {
            Engine::Sqlite => "sqlite",
            Engine::Postgres => "postgres",
            Engine::Mysql => "mysql",
            Engine::Mariadb => "mariadb",
        }
    }

    fn default_path(self) -> PathBuf {
        crate::database_path(format!("schema/{}-schema.sql", self.name()))
    }

    fn dump_tool(self) -> Option<&'static str> {
        match self {
            Engine::Sqlite => None,
            Engine::Postgres => Some("pg_dump"),
            Engine::Mysql => Some("mysqldump"),
            Engine::Mariadb => Some("mariadb-dump"),
        }
    }

    fn load_tool(self) -> Option<&'static str> {
        match self {
            Engine::Sqlite => None,
            Engine::Postgres => Some("psql"),
            Engine::Mysql => Some("mysql"),
            Engine::Mariadb => Some("mariadb"),
        }
    }
}

/// Loads the dump `schema` names, or the engine's default one, when `M`'s
/// ledger records no migration; an empty ledger table is dropped first,
/// since the dump creates it.
async fn load_when_empty<M: MigratorTrait>(
    url: &str,
    db: &DatabaseConnection,
    schema: Option<&Path>,
) -> Result<Option<PathBuf>, FrameworkError> {
    let manager = SchemaManager::new(db);
    let table = M::migration_table_name();
    let has_ledger = manager
        .has_table(table.to_string())
        .await
        .map_err(database_error)?;
    if has_ledger && !ledger_rows(db, table.clone()).await?.is_empty() {
        return Ok(None);
    }
    let engine = Engine::of(db).await?;
    let path = schema.map_or_else(|| engine.default_path(), Path::to_path_buf);
    if !path.is_file() {
        return Ok(None);
    }
    // A load runs into an empty database only: a dump loaded over tables
    // would collide with them, or on MySQL replace them.
    let ledger = table.to_string();
    let occupied: Vec<String> = relations(db, engine)
        .await?
        .into_iter()
        .filter(|name| *name != ledger)
        .collect();
    if !occupied.is_empty() {
        return Err(FrameworkError::database(format!(
            "the database holds {} while its migration ledger records no migration, so the \
             schema dump {} was not loaded over them",
            occupied.join(", "),
            path.display()
        )));
    }
    if has_ledger {
        manager
            .drop_table(Table::drop().table(table).to_owned())
            .await
            .map_err(database_error)?;
    }
    load_into(url, db, engine, &path).await?;
    Ok(Some(path))
}

async fn load_into(
    url: &str,
    db: &DatabaseConnection,
    engine: Engine,
    path: &Path,
) -> Result<(), FrameworkError> {
    let Some(tool) = engine.load_tool() else {
        return load_sqlite(db, &read(path)?).await;
    };
    let tool_path = find_tool(tool)?;
    let target = Target::parse(url)?;
    match engine {
        Engine::Postgres => {
            let mut args = vec![
                "--quiet".to_owned(),
                "--no-psqlrc".to_owned(),
                "--set=ON_ERROR_STOP=1".to_owned(),
                "--single-transaction".to_owned(),
                format!("--file={}", path.display()),
            ];
            args.extend(target.postgres_args());
            run_tool(&tool_path, tool, &args, &target.postgres_env(), None).await?;
        }
        _ => {
            let options = target.mysql_option_file()?;
            let mut args = vec![format!("--defaults-file={}", options.path.display())];
            args.extend(target.mysql_args(is_mariadb_client(&tool_path).await));
            args.push(format!("--database={}", target.database));
            run_tool(&tool_path, tool, &args, &[], Some(path)).await?;
        }
    }
    Ok(())
}

/// The tables and views of the database's current schema.
async fn relations(db: &DatabaseConnection, engine: Engine) -> Result<Vec<String>, FrameworkError> {
    let sql = match engine {
        Engine::Sqlite => {
            "SELECT name FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%'"
        }
        Engine::Postgres => {
            "SELECT table_name::text AS name FROM information_schema.tables \
             WHERE table_schema = current_schema()"
        }
        Engine::Mysql | Engine::Mariadb => {
            "SELECT table_name AS name FROM information_schema.tables WHERE table_schema = DATABASE()"
        }
    };
    Ok(rows_of(db, sql, &["name"])
        .await?
        .into_iter()
        .map(|mut row| row.remove(0))
        .collect())
}

/// After SeaORM's `fresh`, which drops tables and enum types, drops what
/// else a dump creates in the current schema: views, triggers, routines,
/// sequences and the other types, leaving the objects of extensions.
async fn drop_the_rest(db: &DatabaseConnection, engine: Engine) -> Result<(), FrameworkError> {
    let mut statements = Vec::new();
    match engine {
        Engine::Sqlite => {
            for row in rows_of(
                db,
                "SELECT type AS kind, name FROM sqlite_master WHERE type IN ('view', 'trigger') \
                 AND name NOT LIKE 'sqlite_%' ORDER BY type DESC",
                &["kind", "name"],
            )
            .await?
            {
                let kind = if row[0] == "view" { "VIEW" } else { "TRIGGER" };
                statements.push(format!(
                    "DROP {kind} IF EXISTS {}",
                    quote_identifier(engine, &row[1])
                ));
            }
        }
        Engine::Postgres => {
            let queries = [
                (
                    "SELECT 'VIEW' AS kind, table_name::text AS name FROM information_schema.views \
                     WHERE table_schema = current_schema()",
                    true,
                ),
                (
                    "SELECT 'MATERIALIZED VIEW' AS kind, matviewname::text AS name FROM pg_matviews \
                     WHERE schemaname = current_schema()",
                    true,
                ),
                (
                    "SELECT CASE p.prokind WHEN 'a' THEN 'AGGREGATE' ELSE 'ROUTINE' END AS kind, \
                     p.oid::regprocedure::text AS name FROM pg_proc p \
                     JOIN pg_namespace n ON n.oid = p.pronamespace \
                     WHERE n.nspname = current_schema() AND NOT EXISTS \
                     (SELECT 1 FROM pg_depend d WHERE d.objid = p.oid AND d.deptype = 'e')",
                    false,
                ),
                (
                    "SELECT 'SEQUENCE' AS kind, sequence_name::text AS name \
                     FROM information_schema.sequences WHERE sequence_schema = current_schema()",
                    true,
                ),
                (
                    "SELECT CASE t.typtype WHEN 'd' THEN 'DOMAIN' ELSE 'TYPE' END AS kind, \
                     t.typname::text AS name FROM pg_type t \
                     JOIN pg_namespace n ON n.oid = t.typnamespace \
                     WHERE n.nspname = current_schema() AND t.typtype IN ('c', 'd', 'e', 'r') \
                     AND (t.typrelid = 0 OR (SELECT c.relkind FROM pg_class c WHERE c.oid = t.typrelid) = 'c') \
                     AND NOT EXISTS (SELECT 1 FROM pg_depend d WHERE d.objid = t.oid AND d.deptype = 'e')",
                    true,
                ),
            ];
            for (sql, quote) in queries {
                for row in rows_of(db, sql, &["kind", "name"]).await? {
                    // A routine's name is already its quoted signature.
                    let name = if quote {
                        quote_identifier(engine, &row[1])
                    } else {
                        row[1].clone()
                    };
                    statements.push(format!("DROP {} IF EXISTS {name} CASCADE", row[0]));
                }
            }
        }
        Engine::Mysql | Engine::Mariadb => {
            for row in rows_of(
                db,
                "SELECT 'VIEW' AS kind, table_name AS name FROM information_schema.views \
                 WHERE table_schema = DATABASE() \
                 UNION ALL SELECT routine_type AS kind, routine_name AS name \
                 FROM information_schema.routines WHERE routine_schema = DATABASE()",
                &["kind", "name"],
            )
            .await?
            {
                statements.push(format!(
                    "DROP {} IF EXISTS {}",
                    row[0],
                    quote_identifier(engine, &row[1])
                ));
            }
        }
    }
    for statement in statements {
        db.execute_unprepared(&statement)
            .await
            .map_err(database_error)?;
    }
    Ok(())
}

/// The text of `columns` in each row `sql` returns.
async fn rows_of(
    db: &DatabaseConnection,
    sql: &str,
    columns: &[&str],
) -> Result<Vec<Vec<String>>, FrameworkError> {
    db.query_all_raw(Statement::from_string(db.get_database_backend(), sql))
        .await
        .map_err(database_error)?
        .iter()
        .map(|row| {
            columns
                .iter()
                .map(|column| row.try_get::<String>("", column).map_err(database_error))
                .collect()
        })
        .collect()
}

/// Whether the MySQL client at `tool` is MariaDB's, whose TLS options
/// differ from Oracle's.
async fn is_mariadb_client(tool: &Path) -> bool {
    tokio::process::Command::new(tool)
        .arg("--version")
        .stdin(Stdio::null())
        .output()
        .await
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("MariaDB"))
}

/// Runs a SQLite dump through the connection, in one transaction.
pub(crate) async fn load_sqlite(db: &DatabaseConnection, sql: &str) -> Result<(), FrameworkError> {
    let txn = db.begin().await.map_err(database_error)?;
    txn.execute_unprepared(sql)
        .await
        .map_err(|e| FrameworkError::database(format!("the schema dump did not load: {e}")))?;
    txn.commit().await.map_err(database_error)
}

/// The statements that create a SQLite database's tables, indexes, views
/// and triggers, tables first, leaving out SQLite's own tables and the
/// shadow tables a virtual table creates for itself.
async fn sqlite_schema(db: &DatabaseConnection) -> Result<String, FrameworkError> {
    let shadow: Vec<String> = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM pragma_table_list WHERE type = 'shadow'",
        ))
        .await
        .map_err(database_error)?
        .iter()
        .map(|row| row.try_get("", "name"))
        .collect::<Result<_, _>>()
        .map_err(database_error)?;
    let rows = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name, sql FROM sqlite_master WHERE sql IS NOT NULL AND name NOT LIKE 'sqlite_%' \
             ORDER BY CASE type WHEN 'table' THEN 0 WHEN 'index' THEN 1 WHEN 'view' THEN 2 ELSE 3 END, rowid",
        ))
        .await
        .map_err(database_error)?;
    let mut sql = String::new();
    for row in rows {
        let name: String = row.try_get("", "name").map_err(database_error)?;
        if shadow.contains(&name) {
            continue;
        }
        let statement: String = row.try_get("", "sql").map_err(database_error)?;
        sql.push_str(&statement);
        sql.push_str(";\n");
    }
    Ok(sql)
}

async fn postgres_schema(url: &str) -> Result<String, FrameworkError> {
    let tool = find_tool("pg_dump")?;
    let target = Target::parse(url)?;
    let mut args = vec![
        "--schema-only".to_owned(),
        "--no-owner".to_owned(),
        "--no-acl".to_owned(),
    ];
    args.extend(target.postgres_args());
    let out = run_tool(&tool, "pg_dump", &args, &target.postgres_env(), None).await?;
    Ok(String::from_utf8_lossy(&out).into_owned())
}

async fn mysql_schema(url: &str, engine: Engine) -> Result<String, FrameworkError> {
    let name = engine.dump_tool().unwrap_or("mysqldump");
    let tool = find_tool(name)?;
    let target = Target::parse(url)?;
    let options = target.mysql_option_file()?;
    let mut args = vec![format!("--defaults-file={}", options.path.display())];
    args.extend(target.mysql_args(is_mariadb_client(&tool).await));
    args.extend(
        [
            "--no-data",
            // Without it each CREATE TABLE comes after a DROP TABLE, and a
            // load would empty a table of the same name.
            "--skip-add-drop-table",
            "--routines",
            "--no-tablespaces",
            "--skip-add-locks",
            "--skip-comments",
            "--skip-set-charset",
            "--tz-utc",
        ]
        .map(str::to_owned),
    );
    let database = target.database.clone();
    let out = if engine == Engine::Mysql {
        // A server with GTIDs would otherwise write a GTID_PURGED that only
        // a privileged user can load; a mysqldump that does not know the
        // option, such as MariaDB's, gets the dump without it.
        let mut with_gtid = args.clone();
        with_gtid.push("--set-gtid-purged=OFF".to_owned());
        with_gtid.push(database.clone());
        match run_tool(&tool, name, &with_gtid, &[], None).await {
            Err(e) if e.to_string().contains("set-gtid-purged") => {
                args.push(database);
                run_tool(&tool, name, &args, &[], None).await?
            }
            other => other?,
        }
    } else {
        args.push(database);
        run_tool(&tool, name, &args, &[], None).await?
    };
    let sql = String::from_utf8_lossy(&out);
    let counters = regex::Regex::new(r"(?i)\s+AUTO_INCREMENT=[0-9]+")
        .map_err(|e| FrameworkError::internal(format!("the AUTO_INCREMENT pattern: {e}")))?;
    Ok(counters.replace_all(&sql, "").into_owned())
}

/// The versions and times `M`'s ledger records, in order; none when the
/// ledger table does not exist.
async fn ledger_rows(
    db: &DatabaseConnection,
    table: DynIden,
) -> Result<Vec<(String, i64)>, FrameworkError> {
    let select = Query::select()
        .columns([Alias::new("version"), Alias::new("applied_at")])
        .from(table)
        .order_by(Alias::new("version"), Order::Asc)
        .to_owned();
    let rows = db.query_all(&select).await.map_err(database_error)?;
    rows.iter()
        .map(|row| {
            Ok((
                row.try_get("", "version").map_err(database_error)?,
                row.try_get("", "applied_at").map_err(database_error)?,
            ))
        })
        .collect()
}

/// One `INSERT` per ledger row, after the marker `prune` looks for.
async fn ledger_inserts<M: MigratorTrait>(
    db: &DatabaseConnection,
    engine: Engine,
) -> Result<String, FrameworkError> {
    let table = M::migration_table_name();
    let name = table.to_string();
    if !SchemaManager::new(db)
        .has_table(&name)
        .await
        .map_err(database_error)?
    {
        return Ok(String::new());
    }
    let qualified = match engine {
        // A Postgres dump empties the search path, so the table is named
        // with its schema.
        Engine::Postgres => {
            let row = db
                .query_one_raw(Statement::from_string(
                    DbBackend::Postgres,
                    "SELECT current_schema() AS schema",
                ))
                .await
                .map_err(database_error)?;
            let schema: String = row
                .map(|row| row.try_get("", "schema"))
                .transpose()
                .map_err(database_error)?
                .unwrap_or_else(|| "public".to_owned());
            format!(
                "{}.{}",
                quote_identifier(engine, &schema),
                quote_identifier(engine, &name)
            )
        }
        _ => quote_identifier(engine, &name),
    };
    let mut sql = format!("\n{LEDGER_MARKER}\n");
    for (version, applied_at) in ledger_rows(db, table).await? {
        sql.push_str(&format!(
            "INSERT INTO {qualified} ({}, {}) VALUES ({}, {applied_at});\n",
            quote_identifier(engine, "version"),
            quote_identifier(engine, "applied_at"),
            quote_string(engine, &version),
        ));
    }
    Ok(sql)
}

/// The versions the ledger rows of a dump record, in order.
pub(crate) fn ledger_versions(sql: &str) -> Vec<String> {
    sql.lines()
        .skip_while(|line| line.trim() != LEDGER_MARKER)
        .filter(|line| line.trim_start().starts_with("INSERT"))
        .filter_map(|line| {
            let values = line.split_once("VALUES (")?.1;
            let rest = values.strip_prefix('\'')?;
            let mut version = String::new();
            let mut chars = rest.chars().peekable();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => version.push(chars.next()?),
                    '\'' if chars.peek() == Some(&'\'') => {
                        chars.next();
                        version.push('\'');
                    }
                    '\'' => return Some(version),
                    c => version.push(c),
                }
            }
            None
        })
        .collect()
}

fn quote_identifier(engine: Engine, name: &str) -> String {
    match engine {
        Engine::Mysql | Engine::Mariadb => format!("`{}`", name.replace('`', "``")),
        _ => format!("\"{}\"", name.replace('"', "\"\"")),
    }
}

fn quote_string(engine: Engine, value: &str) -> String {
    let escaped = value.replace('\'', "''");
    match engine {
        // MySQL reads a backslash in a string as an escape.
        Engine::Mysql | Engine::Mariadb => format!("'{}'", escaped.replace('\\', "\\\\")),
        _ => format!("'{escaped}'"),
    }
}

/// What a client tool needs from a database URL, the query parameters the
/// framework's own connection honors included, so the tools reach the same
/// server the same way: TLS settings, a socket, a password given as a
/// parameter.
struct Target {
    host: String,
    port: Option<u16>,
    user: String,
    password: Option<String>,
    database: String,
    params: Vec<(String, String)>,
}

/// Postgres URL parameters and the libpq variables that carry them.
const POSTGRES_PARAMETERS: [(&str, &str); 7] = [
    ("sslmode", "PGSSLMODE"),
    ("sslrootcert", "PGSSLROOTCERT"),
    ("sslcert", "PGSSLCERT"),
    ("sslkey", "PGSSLKEY"),
    ("hostaddr", "PGHOSTADDR"),
    ("application_name", "PGAPPNAME"),
    ("options", "PGOPTIONS"),
];

impl Target {
    fn parse(url: &str) -> Result<Self, FrameworkError> {
        // The message never repeats the URL, which may hold a password.
        let parsed = url::Url::parse(url)
            .map_err(|_| FrameworkError::database("the database URL is not a valid URL"))?;
        let decode = |s: &str| {
            percent_encoding::percent_decode_str(s)
                .decode_utf8_lossy()
                .into_owned()
        };
        Ok(Self {
            host: parsed
                .host_str()
                .unwrap_or("")
                .trim_start_matches('[')
                .trim_end_matches(']')
                .to_owned(),
            port: parsed.port(),
            user: decode(parsed.username()),
            password: parsed.password().map(decode),
            database: decode(parsed.path().trim_start_matches('/')),
            params: parsed
                .query_pairs()
                .map(|(key, value)| (key.into_owned(), value.into_owned()))
                .collect(),
        })
    }

    fn param(&self, key: &str) -> Option<&str> {
        self.params
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    fn postgres_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        let host = self.param("host").unwrap_or(&self.host);
        if !host.is_empty() {
            args.push(format!("--host={host}"));
        }
        match self.param("port") {
            Some(port) => args.push(format!("--port={port}")),
            None => {
                if let Some(port) = self.port {
                    args.push(format!("--port={port}"));
                }
            }
        }
        let user = self.param("user").unwrap_or(&self.user);
        if !user.is_empty() {
            args.push(format!("--username={user}"));
        }
        args.push("--no-password".to_owned());
        args.push(format!(
            "--dbname={}",
            self.param("dbname").unwrap_or(&self.database)
        ));
        args
    }

    fn postgres_env(&self) -> Vec<(&'static str, String)> {
        let mut env = Vec::new();
        if let Some(password) = self.password.as_deref().or(self.param("password")) {
            env.push(("PGPASSWORD", password.to_owned()));
        }
        for (key, variable) in POSTGRES_PARAMETERS {
            if let Some(value) = self.param(key) {
                env.push((variable, value.to_owned()));
            }
        }
        env
    }

    /// The connection arguments for a MySQL client: the socket, or the host
    /// and port; the user; and the URL's TLS settings in the client's own
    /// options, MariaDB's client having no `--ssl-mode`.
    fn mysql_args(&self, mariadb_client: bool) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(socket) = self.param("socket") {
            args.push(format!("--socket={socket}"));
        } else {
            if !self.host.is_empty() {
                args.push(format!("--host={}", self.host));
            }
            if let Some(port) = self.port {
                args.push(format!("--port={port}"));
            }
        }
        if !self.user.is_empty() {
            args.push(format!("--user={}", self.user));
        }
        for key in ["ssl-ca", "ssl-cert", "ssl-key"] {
            if let Some(value) = self.param(key) {
                args.push(format!("--{key}={value}"));
            }
        }
        if let Some(mode) = self.param("ssl-mode") {
            let mode = mode.to_ascii_uppercase().replace('-', "_");
            if !mariadb_client {
                args.push(format!("--ssl-mode={mode}"));
            } else {
                match mode.as_str() {
                    "DISABLED" => args.push("--skip-ssl".to_owned()),
                    "REQUIRED" => {
                        args.extend(["--ssl", "--skip-ssl-verify-server-cert"].map(str::to_owned))
                    }
                    "VERIFY_CA" | "VERIFY_IDENTITY" => {
                        args.extend(["--ssl", "--ssl-verify-server-cert"].map(str::to_owned))
                    }
                    _ => {}
                }
            }
        }
        args
    }

    /// An option file holding the password, readable only by this user,
    /// deleted when the returned value drops. The tools are given it with
    /// `--defaults-file`, so it is the only option file they read and a
    /// password in `~/.my.cnf` cannot take the URL's place.
    fn mysql_option_file(&self) -> Result<OptionFile, FrameworkError> {
        let dir = tempfile::tempdir().map_err(|e| {
            FrameworkError::internal(format!(
                "could not create a directory for the option file: {e}"
            ))
        })?;
        let path = dir.path().join("client.cnf");
        let mut contents = String::from("[client]\n");
        if let Some(password) = &self.password {
            contents.push_str(&format!(
                "password=\"{}\"\n",
                password.replace('\\', "\\\\").replace('"', "\\\"")
            ));
        }
        write_private(&path, &contents)?;
        Ok(OptionFile { _dir: dir, path })
    }
}

/// A MySQL option file, removed with its directory on drop.
struct OptionFile {
    _dir: tempfile::TempDir,
    path: PathBuf,
}

#[cfg(unix)]
fn write_private(path: &Path, contents: &str) -> Result<(), FrameworkError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(contents.as_bytes()))
        .map_err(|e| FrameworkError::internal(format!("could not write the option file: {e}")))
}

#[cfg(not(unix))]
fn write_private(path: &Path, contents: &str) -> Result<(), FrameworkError> {
    write(path, contents)
}

/// The executable called `name` on `PATH`.
fn find_tool(name: &str) -> Result<PathBuf, FrameworkError> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
        .ok_or_else(|| {
            FrameworkError::internal(format!(
                "`{name}` was not found on PATH; install the database's client tools to \
                 dump or load a schema"
            ))
        })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Runs a client tool and returns what it wrote to stdout; an exit that
/// is not success is an error carrying what it wrote to stderr.
async fn run_tool(
    tool: &Path,
    name: &str,
    args: &[String],
    env: &[(&'static str, String)],
    stdin: Option<&Path>,
) -> Result<Vec<u8>, FrameworkError> {
    let mut command = tokio::process::Command::new(tool);
    command
        .args(args)
        .envs(env.iter().map(|(key, value)| (*key, value.as_str())))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    match stdin {
        Some(path) => {
            let file = std::fs::File::open(path).map_err(|e| {
                FrameworkError::internal(format!("could not open {}: {e}", path.display()))
            })?;
            command.stdin(file);
        }
        None => {
            command.stdin(Stdio::null());
        }
    }
    let out = command
        .output()
        .await
        .map_err(|e| FrameworkError::internal(format!("`{name}` could not start: {e}")))?;
    if !out.status.success() {
        return Err(FrameworkError::internal(format!(
            "`{name}` failed ({}): {}",
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(out.stdout)
}

async fn connect(url: &str) -> Result<DatabaseConnection, FrameworkError> {
    sea_orm::Database::connect(crate::database::config::driver_url(url).as_ref())
        .await
        .map_err(|e| FrameworkError::database(format!("could not connect to the database: {e}")))
}

/// Writes `contents` to `path` through a temporary file beside it, so an
/// earlier file is replaced only by a complete one.
fn write_replacing(path: &Path, contents: &str) -> Result<(), FrameworkError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|e| {
            FrameworkError::internal(format!("could not create {}: {e}", parent.display()))
        })?;
    }
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "schema.sql".to_owned());
    let partial = path.with_file_name(format!(".{name}.partial"));
    write(&partial, contents)?;
    std::fs::rename(&partial, path).map_err(|e| {
        FrameworkError::internal(format!(
            "could not move the dump to {}: {e}",
            path.display()
        ))
    })
}

fn read(path: &Path) -> Result<String, FrameworkError> {
    std::fs::read_to_string(path)
        .map_err(|e| FrameworkError::internal(format!("could not read {}: {e}", path.display())))
}

fn write(path: &Path, contents: &str) -> Result<(), FrameworkError> {
    std::fs::write(path, contents)
        .map_err(|e| FrameworkError::internal(format!("could not write {}: {e}", path.display())))
}

fn database_error(e: DbErr) -> FrameworkError {
    FrameworkError::database(e.to_string())
}

fn migration_failed(e: DbErr) -> FrameworkError {
    FrameworkError::database(format!("Migration failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_versions_read_only_the_rows_after_the_marker() {
        let sql = "CREATE TABLE t (id INT);\nINSERT INTO t VALUES ('not a ledger row');\n\n\
                   -- Suprnova migration ledger\n\
                   INSERT INTO \"seaql_migrations\" (\"version\", \"applied_at\") VALUES ('m1_a', 1);\n\
                   INSERT INTO `seaql_migrations` (`version`, `applied_at`) VALUES ('m2_it''s\\\\b', 2);\n";
        assert_eq!(ledger_versions(sql), ["m1_a", "m2_it's\\b"]);
    }

    #[test]
    fn url_parameters_reach_the_tools() {
        let pg = Target::parse(
            "postgres://app@localhost/shop?host=/run/postgresql&password=s3cret&sslrootcert=/ca.pem&sslmode=verify-full",
        )
        .expect("a URL");
        assert_eq!(
            pg.postgres_args().join(" "),
            "--host=/run/postgresql --username=app --no-password --dbname=shop"
        );
        assert_eq!(
            pg.postgres_env(),
            [
                ("PGPASSWORD", "s3cret".to_owned()),
                ("PGSSLMODE", "verify-full".to_owned()),
                ("PGSSLROOTCERT", "/ca.pem".to_owned()),
            ]
        );
        let my =
            Target::parse("mysql://app:pw@db:3306/shop?ssl-mode=verify_identity&ssl-ca=/ca.pem")
                .expect("a URL");
        assert_eq!(
            my.mysql_args(false).join(" "),
            "--host=db --port=3306 --user=app --ssl-ca=/ca.pem --ssl-mode=VERIFY_IDENTITY"
        );
        assert_eq!(
            my.mysql_args(true).join(" "),
            "--host=db --port=3306 --user=app --ssl-ca=/ca.pem --ssl --ssl-verify-server-cert"
        );
        let socket =
            Target::parse("mysql://app@localhost/shop?socket=/run/mysqld.sock").expect("a URL");
        assert_eq!(
            socket.mysql_args(true).join(" "),
            "--socket=/run/mysqld.sock --user=app"
        );
    }

    #[test]
    fn a_url_gives_the_tools_its_parts_without_the_password_in_arguments() {
        let target =
            Target::parse("postgres://app%40x:p%40ss@db.internal:5433/shop?sslmode=require")
                .expect("a URL");
        let args = target.postgres_args().join(" ");
        assert_eq!(
            args,
            "--host=db.internal --port=5433 --username=app@x --no-password --dbname=shop"
        );
        assert!(!args.contains("p@ss"));
        assert_eq!(
            target.postgres_env(),
            [
                ("PGPASSWORD", "p@ss".to_owned()),
                ("PGSSLMODE", "require".to_owned())
            ]
        );
    }
}
