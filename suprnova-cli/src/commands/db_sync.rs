//! db:sync command - Run migrations and sync entity files from database schema
//!
//! # Trust boundary
//!
//! Everything this command learns about the schema — table names, column
//! names, column types — comes back from the database, and the database is
//! not necessarily trustworthy: a developer runs `db:sync` against a dump
//! someone handed them, a shared staging box, or a container image from a
//! registry. Two of those names then reach dangerous places:
//!
//! - the **filesystem**, as `src/models/entities/<table>.rs`, where a name
//!   like `../../../.cargo/config.toml` escapes the entity directory; and
//! - **generated Rust source**, where a column named
//!   `x: i32, } fn evil() { …` injects code that compiles on the next build.
//!
//! So no schema name is used for either purpose until [`SafeName::parse`] has
//! proven it is a bare ASCII identifier, every output path is proven to land
//! directly inside the canonical entity directory, and every write goes
//! through [`write_generated_file`], which cannot be redirected by a symlink
//! planted at the destination.

use sea_orm::{ConnectionTrait, Database, DbBackend, Statement, Value};
use std::env;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::templates;
use crate::templates::{ColumnInfo, TableInfo};
use crate::ui;

pub fn run(skip_migrations: bool, regenerate_models: bool) {
    if let Err(e) = run_inner(skip_migrations, regenerate_models) {
        ui::error(&e);
        std::process::exit(1);
    }
}

fn run_inner(skip_migrations: bool, regenerate_models: bool) -> Result<(), String> {
    if !Path::new("src/models").exists() && !Path::new("src/migrations").exists() {
        return Err("Not in a Suprnova project directory".to_string());
    }

    if !skip_migrations {
        run_migrations()?;
    }

    generate_entities(regenerate_models)
}

fn run_migrations() -> Result<(), String> {
    if !Path::new("src/migrations").exists() {
        ui::warning("No migrations directory found, skipping migrations");
        return Ok(());
    }

    if !Path::new("src/bin/migrate.rs").exists() {
        ui::warning("Migration binary not found, skipping migrations");
        return Ok(());
    }

    ui::info("Running pending migrations...");

    let status = crate::commands::cargo_run(&["migrate"])
        .status()
        .map_err(|e| format!("Failed to execute `cargo run --quiet -- migrate`: {e}"))?;

    if !status.success() {
        return Err(format!(
            "Migration failed (`cargo run --quiet -- migrate` exited with {})",
            status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string()),
        ));
    }
    ui::success("Migrations complete");
    Ok(())
}

fn generate_entities(regenerate_models: bool) -> Result<(), String> {
    // Load DATABASE_URL from .env
    dotenvy::dotenv().ok();

    let database_url =
        env::var("DATABASE_URL").map_err(|_| "DATABASE_URL not set in .env".to_string())?;

    ui::info("Discovering database schema...");

    let rt = tokio::runtime::Runtime::new()
        .map_err(|e| format!("Failed to start the async runtime: {e}"))?;
    rt.block_on(discover_and_generate(&database_url, regenerate_models))
}

/// Every Rust keyword — strict, reserved, and weak — plus the path
/// qualifiers. A schema name matching one of these is a perfectly legal SQL
/// identifier that cannot be emitted as `pub mod <name>;` or `pub <name>:`,
/// so it is rejected rather than silently producing a file that will not
/// compile. `Self` is in the list because `singularize` turns the table
/// `selfs` into exactly that.
const RUST_KEYWORDS: &[&str] = &[
    "Self", "abstract", "as", "async", "await", "become", "box", "break", "const", "continue",
    "crate", "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if",
    "impl", "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub",
    "ref", "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// Upper bound on a schema name we are willing to turn into a file name.
/// Well under every filesystem's per-component limit, with room to spare for
/// the `.rs` suffix and the temp-file prefix `write_generated_file` adds.
const MAX_SCHEMA_NAME_LEN: usize = 100;

/// A schema name proven safe to use *both* as a Rust identifier and as a
/// single filesystem path component.
///
/// One rule covers both jobs, which is why they share a type: an ASCII
/// identifier (`[A-Za-z][A-Za-z0-9_]*`) contains no path separator, no `..`,
/// no quote, no newline, no terminal escape, and no shell metacharacter. The
/// alternative — sanitising by replacing bad characters — silently maps two
/// distinct tables onto one file, so this refuses instead.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SafeName(String);

impl SafeName {
    /// Reject anything that is not a bare ASCII identifier.
    ///
    /// The leading character must be alphabetic rather than `_`: a name
    /// starting with `_` is already filtered out upstream as an internal
    /// table, and a bare `_` would produce `pub mod _;`, which is a wildcard
    /// pattern rather than an identifier.
    fn parse(raw: &str) -> Result<Self, String> {
        if raw.is_empty() {
            return Err("the name is empty".to_string());
        }
        if raw.len() > MAX_SCHEMA_NAME_LEN {
            return Err(format!(
                "the name is {} bytes long; the limit is {MAX_SCHEMA_NAME_LEN}",
                raw.len()
            ));
        }

        let mut chars = raw.chars();
        // `unwrap` is unreachable: the emptiness check above guarantees one char.
        let first = chars.next().unwrap_or('\0');
        if !first.is_ascii_alphabetic() {
            return Err(format!(
                "it must start with an ASCII letter, but starts with `{}`",
                display_name(&first.to_string())
            ));
        }
        for c in chars {
            if !(c.is_ascii_alphanumeric() || c == '_') {
                return Err(format!(
                    "`{}` is not allowed — only ASCII letters, digits and `_` are",
                    display_name(&c.to_string())
                ));
            }
        }
        if RUST_KEYWORDS.contains(&raw) {
            return Err(format!("`{raw}` is a Rust keyword"));
        }

        Ok(Self(raw.to_string()))
    }

    fn as_str(&self) -> &str {
        &self.0
    }
}

/// Render an untrusted schema name for terminal output.
///
/// The rejection warnings echo the offending name back at the user, and a
/// name is free to contain ANSI escapes or control characters. Escaping them
/// keeps a hostile schema from repainting the terminal with a fake "success".
fn display_name(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.chars().take(MAX_SCHEMA_NAME_LEN) {
        if c.is_ascii_graphic() || c == ' ' {
            out.push(c);
        } else {
            out.extend(c.escape_debug());
        }
    }
    if raw.chars().count() > MAX_SCHEMA_NAME_LEN {
        out.push_str("...");
    }
    out
}

/// Check a discovered table end to end: its own name, the Rust struct name
/// derived from it, and every column name.
///
/// A table is accepted or skipped whole. Dropping just the offending column
/// would emit a model that silently lacks a field, which is worse than a
/// visible refusal.
fn vet_table(table: &TableInfo) -> Result<(), String> {
    SafeName::parse(&table.name).map_err(|e| format!("table name rejected: {e}"))?;

    // `user_model_template` names its type `to_pascal_case(singularize(table))`.
    // That transform can turn an accepted table name into an unusable one —
    // `selfs` becomes `Self`, `___` becomes the empty string — so the derived
    // name gets the same check as the source name.
    let struct_name = to_pascal_case(&singularize(&table.name));
    SafeName::parse(&struct_name).map_err(|e| {
        format!(
            "the model struct name `{}` derived from this table is unusable: {e}",
            display_name(&struct_name)
        )
    })?;

    for column in &table.columns {
        SafeName::parse(&column.name)
            .map_err(|e| format!("column `{}` rejected: {e}", display_name(&column.name)))?;
    }

    Ok(())
}

/// Path for a file named after a schema object, inside `dir`.
///
/// Taking a [`SafeName`] rather than a `&str` is the point: an output path
/// derived from a schema name cannot be constructed without first passing
/// validation, so a future call site can't skip the check by accident. The
/// containment proof still runs on top of it.
fn generated_path(dir: &Path, stem: &SafeName) -> Result<PathBuf, String> {
    contained_path(dir, &format!("{}.rs", stem.as_str()))
}

/// Join `file_name` onto `dir`, proving the result lands *directly* inside
/// the canonical `dir`.
///
/// `SafeName` already makes traversal impossible, but the containment proof
/// is what makes that guarantee auditable at the point of use: if the name
/// rules are ever loosened, this still refuses to write outside the entity
/// directory instead of quietly following the new name wherever it points.
fn contained_path(dir: &Path, file_name: &str) -> Result<PathBuf, String> {
    let mut components = Path::new(file_name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => {}
        _ => {
            return Err(format!(
                "Refusing to write `{}`: a generated file name must be a single \
                 path component",
                display_name(file_name)
            ));
        }
    }

    let base = dir.canonicalize().map_err(|e| {
        format!(
            "Failed to resolve the output directory {}: {e}",
            dir.display()
        )
    })?;
    let path = base.join(file_name);
    if path.parent() != Some(base.as_path()) {
        return Err(format!(
            "Refusing to write `{}`: it resolves outside {}",
            display_name(file_name),
            base.display()
        ));
    }
    Ok(path)
}

/// Write a generated file without ever following a symlink standing at
/// `path`.
///
/// A plain `fs::write` opens the destination with `O_CREAT|O_TRUNC`, which
/// follows a symlink: anything that can plant `src/models/entities/users.rs`
/// as a link to `~/.bashrc` gets that file overwritten with our content.
/// Writing to a fresh temp file with `O_EXCL` and then `rename`ing it into
/// place removes that: `O_EXCL` refuses to open through a link, and
/// `rename(2)` replaces the link itself rather than its target. It is also
/// atomic, so an interrupted run can't leave a half-written entity behind.
fn write_generated_file(path: &Path, contents: &str) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| {
        format!(
            "Refusing to write {}: it has no parent directory",
            path.display()
        )
    })?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| format!("Refusing to write {}: it has no file name", path.display()))?;

    let tmp = parent.join(format!(".{file_name}.{}.tmp", std::process::id()));
    // Clear a temp file left by a crashed run, or `create_new` fails. Unlink
    // acts on the link itself, so this cannot reach outside `parent`.
    let _ = fs::remove_file(&tmp);

    let mut handle = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .map_err(|e| format!("Failed to create temporary file {}: {e}", tmp.display()))?;

    if let Err(e) = handle.write_all(contents.as_bytes()) {
        drop(handle);
        let _ = fs::remove_file(&tmp);
        return Err(format!("Failed to write {}: {e}", tmp.display()));
    }
    drop(handle);

    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!(
            "Failed to move {} into place at {}: {e}",
            tmp.display(),
            path.display()
        )
    })
}

/// Which introspection dialect a `DATABASE_URL` selects.
///
/// Previously this was a bare `starts_with("sqlite")` boolean whose `else`
/// branch issued Postgres `information_schema` statements — so a MySQL URL
/// silently ran the wrong dialect. Naming the third case turns that into a
/// clear refusal.
enum SyncBackend {
    Sqlite,
    Postgres,
}

fn classify_backend(database_url: &str) -> Result<SyncBackend, String> {
    if database_url.starts_with("sqlite") {
        Ok(SyncBackend::Sqlite)
    } else if database_url.starts_with("postgres") {
        Ok(SyncBackend::Postgres)
    } else if database_url.starts_with("mysql") {
        Err(
            "db:sync does not support MySQL yet — its schema introspection \
             uses Postgres-specific information_schema queries. Use \
             hand-written SeaORM migrations for MySQL projects."
                .to_string(),
        )
    } else {
        Err(format!(
            "db:sync cannot determine the database backend from DATABASE_URL. \
             Expected a sqlite://, postgres://, or mysql:// URL, got: {}",
            database_url.split(':').next().unwrap_or("(empty)")
        ))
    }
}

async fn discover_and_generate(database_url: &str, regenerate_models: bool) -> Result<(), String> {
    let backend = classify_backend(database_url)?;

    let db = Database::connect(database_url)
        .await
        .map_err(|e| format!("Failed to connect to database: {e}"))?;

    let tables = match backend {
        SyncBackend::Sqlite => discover_sqlite_tables(&db).await?,
        SyncBackend::Postgres => discover_postgres_tables(&db).await?,
    };

    // Filter out migration tables
    let tables: Vec<_> = tables
        .into_iter()
        .filter(|t| t.name != "seaql_migrations" && !t.name.starts_with("_"))
        .collect();

    if tables.is_empty() {
        ui::warning("No tables found in database");
        return Ok(());
    }

    // Everything past this point turns names into file paths and Rust source,
    // so vet first and drop anything that can't be rendered safely. Skipping
    // per table (rather than aborting the run) keeps one odd table from
    // blocking a sync of the twenty good ones next to it.
    let discovered = tables.len();
    let mut rejected = 0usize;
    let tables: Vec<TableInfo> = tables
        .into_iter()
        .filter(|table| match vet_table(table) {
            Ok(()) => true,
            Err(reason) => {
                rejected += 1;
                ui::warning(&format!(
                    "Skipping table `{}`: {reason}",
                    display_name(&table.name)
                ));
                false
            }
        })
        .collect();

    if tables.is_empty() {
        return Err(format!(
            "None of the {discovered} discovered table(s) can be turned into Rust \
             models — every one was skipped for the reason printed above. Rename \
             them to plain identifiers (letters, digits and underscores) or write \
             the models by hand."
        ));
    }
    if rejected > 0 {
        ui::warning(&format!(
            "{rejected} of {discovered} table(s) were skipped; generating the rest"
        ));
    }

    ui::success(&format!(
        "Found {} table(s): {}",
        tables.len(),
        tables
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    ));

    let models_dir = Path::new("src/models");
    if !models_dir.exists() {
        fs::create_dir_all(models_dir).map_err(|e| {
            format!(
                "Failed to create models directory {}: {e}",
                models_dir.display()
            )
        })?;
        ui::success("Created src/models directory");
    }

    let entities_dir = models_dir.join("entities");
    if !entities_dir.exists() {
        fs::create_dir_all(&entities_dir).map_err(|e| {
            format!(
                "Failed to create entities directory {}: {e}",
                entities_dir.display()
            )
        })?;
        ui::success("Created src/models/entities directory");
    }

    for table in &tables {
        generate_entity_file(table, &entities_dir)?;
        if regenerate_models {
            generate_user_file(table, models_dir)?;
        } else {
            generate_user_file_if_not_exists(table, models_dir)?;
        }
    }

    update_entities_mod(&tables, &entities_dir)?;
    update_models_mod(&tables, models_dir)?;

    ui::br();
    ui::success("Entity files generated!");
    ui::br();
    for table in &tables {
        ui::hint(&format!(
            "src/models/entities/{}.rs (auto-generated)",
            table.name
        ));
        ui::hint(&format!(
            "src/models/{}.rs (user customizations)",
            table.name
        ));
    }

    Ok(())
}

async fn discover_sqlite_tables(
    db: &sea_orm::DatabaseConnection,
) -> Result<Vec<TableInfo>, String> {
    let rows = db
        .query_all_raw(Statement::from_string(
            DbBackend::Sqlite,
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
        ))
        .await
        .map_err(|e| format!("Failed to query sqlite_master for table list: {e}"))?;

    let table_names: Vec<String> = rows
        .iter()
        .filter_map(|row| row.try_get_by_index::<String>(0).ok())
        .collect();

    let mut tables = Vec::with_capacity(table_names.len());
    for table_name in table_names {
        let columns = discover_sqlite_columns(db, &table_name).await?;
        tables.push(TableInfo {
            name: table_name,
            columns,
        });
    }

    Ok(tables)
}

async fn discover_sqlite_columns(
    db: &sea_orm::DatabaseConnection,
    table_name: &str,
) -> Result<Vec<ColumnInfo>, String> {
    // `PRAGMA table_info(x)` takes no bind parameters, so the table name used
    // to be interpolated into the statement. The `pragma_table_info(?)`
    // table-valued function is the same introspection behind an ordinary
    // SELECT, which *does* bind — so the name travels as a value and can
    // never be read as SQL, whatever quoting the schema was created with.
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::Sqlite,
            r#"SELECT "name", "type", "notnull", "pk" FROM pragma_table_info(?)"#,
            [Value::from(table_name)],
        ))
        .await
        .map_err(|e| {
            format!(
                "Failed to read columns for sqlite table `{}`: {e}",
                display_name(table_name)
            )
        })?;

    Ok(rows
        .iter()
        .filter_map(|row| {
            let name: String = row.try_get_by_index(0).ok()?;
            let col_type: String = row.try_get_by_index(1).ok()?;
            let notnull: i32 = row.try_get_by_index(2).ok()?;
            let pk: i32 = row.try_get_by_index(3).ok()?;

            Some(ColumnInfo {
                name,
                col_type,
                is_nullable: notnull == 0,
                is_primary_key: pk > 0,
            })
        })
        .collect())
}

async fn discover_postgres_tables(
    db: &sea_orm::DatabaseConnection,
) -> Result<Vec<TableInfo>, String> {
    let rows = db.query_all_raw(Statement::from_string(DbBackend::Postgres,
    "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE'",))
        .await
        .map_err(|e| format!("Failed to query information_schema.tables: {e}"))?;

    let table_names: Vec<String> = rows
        .iter()
        .filter_map(|row| row.try_get_by_index::<String>(0).ok())
        .collect();

    let mut tables = Vec::with_capacity(table_names.len());
    for table_name in table_names {
        let columns = discover_postgres_columns(db, &table_name).await?;
        tables.push(TableInfo {
            name: table_name,
            columns,
        });
    }

    Ok(tables)
}

async fn discover_postgres_columns(
    db: &sea_orm::DatabaseConnection,
    table_name: &str,
) -> Result<Vec<ColumnInfo>, String> {
    // The table name is compared against `information_schema` *values*, not
    // used as an identifier — so it binds as a parameter. Hand-escaping a
    // literal into the statement (the previous `''`-doubling) is one review
    // slip away from an injection; `$1` cannot be misread as SQL at all.
    // Postgres allows the same placeholder in both predicates.
    let rows = db
        .query_all_raw(Statement::from_sql_and_values(
            DbBackend::Postgres,
            r#"
            SELECT
    c.column_name,
    c.data_type,
    c.is_nullable,
    CASE WHEN pk.column_name IS NOT NULL THEN true ELSE false END as is_pk
            FROM information_schema.columns c
            LEFT JOIN (
    SELECT ku.column_name
    FROM information_schema.table_constraints tc
    JOIN information_schema.key_column_usage ku
        ON tc.constraint_name = ku.constraint_name
        AND tc.constraint_schema = ku.constraint_schema
    WHERE tc.constraint_type = 'PRIMARY KEY'
        AND tc.table_name = $1
        AND tc.table_schema = 'public'
            ) pk ON c.column_name = pk.column_name
            WHERE c.table_name = $1
    AND c.table_schema = 'public'
            ORDER BY c.ordinal_position
            "#,
            [Value::from(table_name)],
        ))
        .await
        .map_err(|e| {
            format!(
                "Failed to read columns for postgres table `{}`: {e}",
                display_name(table_name)
            )
        })?;

    Ok(rows
        .iter()
        .filter_map(|row| {
            let name: String = row.try_get_by_index(0).ok()?;
            let col_type: String = row.try_get_by_index(1).ok()?;
            let is_nullable_str: String = row.try_get_by_index(2).ok()?;
            let is_pk: bool = row.try_get_by_index(3).ok()?;

            Some(ColumnInfo {
                name,
                col_type,
                is_nullable: is_nullable_str == "YES",
                is_primary_key: is_pk,
            })
        })
        .collect())
}

fn generate_entity_file(table: &TableInfo, entities_dir: &Path) -> Result<(), String> {
    let entity_file = generated_path(entities_dir, &SafeName::parse(&table.name)?)?;
    let content = templates::entity_template(&table.name, &table.columns);

    write_generated_file(&entity_file, &content)?;
    ui::success(&format!("Generated src/models/entities/{}.rs", table.name));
    Ok(())
}

fn generate_user_file_if_not_exists(table: &TableInfo, models_dir: &Path) -> Result<(), String> {
    let user_file = generated_path(models_dir, &SafeName::parse(&table.name)?)?;

    // `symlink_metadata` rather than `exists`: a *dangling* symlink parked at
    // this path reports `exists() == false`, and hand-written models are the
    // one thing db:sync promises never to clobber.
    if user_file.symlink_metadata().is_ok() {
        ui::hint(&format!(
            "Skipped src/models/{}.rs (already exists)",
            table.name
        ));
        return Ok(());
    }

    let struct_name = to_pascal_case(&singularize(&table.name));
    let content = templates::user_model_template(&table.name, &struct_name, &table.columns);

    write_generated_file(&user_file, &content)?;
    ui::success(&format!("Created src/models/{}.rs", table.name));
    Ok(())
}

fn generate_user_file(table: &TableInfo, models_dir: &Path) -> Result<(), String> {
    let user_file = generated_path(models_dir, &SafeName::parse(&table.name)?)?;
    let struct_name = to_pascal_case(&singularize(&table.name));
    let content = templates::user_model_template(&table.name, &struct_name, &table.columns);

    write_generated_file(&user_file, &content)?;
    ui::success(&format!("Regenerated src/models/{}.rs", table.name));
    Ok(())
}

fn update_entities_mod(tables: &[TableInfo], entities_dir: &Path) -> Result<(), String> {
    let mod_file = contained_path(entities_dir, "mod.rs")?;
    let content = templates::entities_mod_template(tables);

    write_generated_file(&mod_file, &content)?;
    ui::success("Updated src/models/entities/mod.rs");
    Ok(())
}

fn update_models_mod(tables: &[TableInfo], models_dir: &Path) -> Result<(), String> {
    let mod_file = contained_path(models_dir, "mod.rs")?;

    // Read existing content (or seed default if absent).  Surface real read
    // failures — silently defaulting on EPERM/EIO would obliterate the user's
    // mod.rs on the subsequent write.
    let existing_content = if mod_file.exists() {
        fs::read_to_string(&mod_file).map_err(|e| {
            format!(
                "Failed to read existing models/mod.rs {}: {e}",
                mod_file.display()
            )
        })?
    } else {
        "//! Application models\n\n".to_string()
    };

    let mut lines: Vec<String> = existing_content.lines().map(String::from).collect();

    let has_entities_mod = lines.iter().any(|l| {
        let trimmed = l.trim();
        trimmed == "pub mod entities;" || trimmed == "mod entities;"
    });

    let mut insert_idx = 0;
    for (i, line) in lines.iter().enumerate() {
        if line.starts_with("//!") || line.is_empty() {
            insert_idx = i + 1;
        } else {
            break;
        }
    }

    if !has_entities_mod {
        lines.insert(insert_idx, "pub mod entities;".to_string());
        insert_idx += 1;
    }

    for table in tables {
        let mod_decl = format!("pub mod {};", table.name);
        let alt_mod_decl = format!("mod {};", table.name);

        if !lines
            .iter()
            .any(|l| l.trim() == mod_decl || l.trim() == alt_mod_decl)
        {
            let mut last_mod_idx = insert_idx;
            for (i, line) in lines.iter().enumerate() {
                if line.trim().starts_with("pub mod ") || line.trim().starts_with("mod ") {
                    last_mod_idx = i + 1;
                }
            }
            lines.insert(last_mod_idx, mod_decl);
        }
    }

    let content = lines.join("\n") + "\n";
    write_generated_file(&mod_file, &content)?;
    ui::success("Updated src/models/mod.rs");
    Ok(())
}

fn to_pascal_case(s: &str) -> String {
    let mut result = String::new();
    let mut capitalize_next = true;

    for c in s.chars() {
        if c == '_' || c == '-' || c == ' ' {
            capitalize_next = true;
        } else if capitalize_next {
            // `char::to_uppercase` yields at least one char by the std::char
            // documented contract — this `.next()` is infallible on any char.
            result.push(c.to_uppercase().next().unwrap_or(c));
            capitalize_next = false;
        } else {
            result.push(c);
        }
    }
    result
}

fn singularize(word: &str) -> String {
    // Basic singularization
    if let Some(stem) = word.strip_suffix("ies") {
        format!("{}y", stem)
    } else if word.ends_with("es") && !word.ends_with("ses") && !word.ends_with("xes") {
        word[..word.len() - 2].to_string()
    } else if word.ends_with("s") && !word.ends_with("ss") && !word.ends_with("us") {
        word[..word.len() - 1].to_string()
    } else {
        word.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str) -> ColumnInfo {
        ColumnInfo {
            name: name.to_string(),
            col_type: "TEXT".to_string(),
            is_nullable: true,
            is_primary_key: false,
        }
    }

    fn table(name: &str, columns: &[&str]) -> TableInfo {
        TableInfo {
            name: name.to_string(),
            columns: columns.iter().map(|c| column(c)).collect(),
        }
    }

    #[test]
    fn safe_name_accepts_ordinary_schema_names() {
        for name in ["users", "user_profiles", "oauth2_tokens", "A", "t1"] {
            assert_eq!(
                SafeName::parse(name).map(|n| n.as_str().to_string()),
                Ok(name.to_string()),
                "`{name}` is a plain identifier and must be accepted"
            );
        }
    }

    #[test]
    fn safe_name_rejects_path_traversal_and_separators() {
        for hostile in [
            "../../../../etc/passwd",
            "..",
            ".",
            "a/b",
            "a\\b",
            "/etc/shadow",
            "..%2fetc",
            "~root",
        ] {
            assert!(
                SafeName::parse(hostile).is_err(),
                "`{hostile}` must never become a file name"
            );
        }
    }

    #[test]
    fn safe_name_rejects_source_injection_payloads() {
        // Each of these compiles into something the generator did not intend
        // if it reaches `pub {name}: T,` or `pub mod {name};` unchecked.
        for hostile in [
            "x: i32, } pub struct Evil { pub y",
            "users\"; fn evil() {}",
            "users'",
            "users\n#[allow(unsafe_code)]",
            "users\u{1b}[2J",
        ] {
            assert!(
                SafeName::parse(hostile).is_err(),
                "`{}` must never reach generated source",
                display_name(hostile)
            );
        }
    }

    #[test]
    fn safe_name_rejects_rust_keywords() {
        for keyword in ["type", "struct", "impl", "crate", "self", "super", "Self"] {
            let err = SafeName::parse(keyword)
                .expect_err("a Rust keyword cannot be emitted as an identifier");
            assert!(
                err.contains("keyword"),
                "the error should say why; got: {err}"
            );
        }
    }

    #[test]
    fn safe_name_rejects_non_ascii_and_leading_digits() {
        for hostile in ["táble", "таблица", "表", "2fa_codes", "_hidden", ""] {
            assert!(
                SafeName::parse(hostile).is_err(),
                "`{hostile}` is not a bare ASCII identifier"
            );
        }
    }

    #[test]
    fn safe_name_rejects_overlong_names() {
        let long = "a".repeat(MAX_SCHEMA_NAME_LEN + 1);
        assert!(SafeName::parse(&long).is_err(), "overlong names must fail");
        let at_limit = "a".repeat(MAX_SCHEMA_NAME_LEN);
        assert!(SafeName::parse(&at_limit).is_ok(), "the limit is inclusive");
    }

    #[test]
    fn vet_table_rejects_a_table_whose_derived_struct_name_is_a_keyword() {
        // `selfs` is a legal identifier and a legal file name, but
        // singularize+pascal-case turns it into `Self`, which cannot be a type
        // name — the failure only shows up in the *derived* name.
        let err = vet_table(&table("selfs", &["id"]))
            .expect_err("a derived struct name of `Self` must be caught");
        assert!(
            err.contains("Self"),
            "the error should name the derived struct; got: {err}"
        );

        // And the empty derived name (`___` pascal-cases to "") too.
        assert!(vet_table(&table("___", &["id"])).is_err());
    }

    #[test]
    fn vet_table_rejects_a_hostile_column_and_keeps_the_reason() {
        let err = vet_table(&table("widgets", &["id", "x: i32, } fn evil() {"]))
            .expect_err("an unusable column must reject the whole table");
        assert!(
            err.contains("column"),
            "the error should point at the column; got: {err}"
        );
    }

    #[test]
    fn vet_table_accepts_an_ordinary_table() {
        assert!(vet_table(&table("widgets", &["id", "name", "created_at"])).is_ok());
    }

    #[test]
    fn display_name_escapes_terminal_control_sequences() {
        let rendered = display_name("evil\u{1b}[2Kok\n");
        assert!(
            !rendered.contains('\u{1b}') && !rendered.contains('\n'),
            "control characters must not survive into terminal output: {rendered:?}"
        );
        assert_eq!(display_name("widgets"), "widgets");
    }

    #[test]
    fn contained_path_accepts_a_plain_file_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = contained_path(dir.path(), "users.rs").expect("a plain name is contained");
        assert_eq!(path.file_name().and_then(|n| n.to_str()), Some("users.rs"));
        assert_eq!(
            path.parent(),
            Some(dir.path().canonicalize().expect("canonicalize").as_path())
        );
    }

    #[test]
    fn contained_path_rejects_escaping_names() {
        let dir = tempfile::tempdir().expect("tempdir");
        for hostile in ["../escape.rs", "sub/dir.rs", "/abs.rs", "..", ""] {
            assert!(
                contained_path(dir.path(), hostile).is_err(),
                "`{hostile}` must not resolve to a writable path"
            );
        }
    }

    #[test]
    fn write_generated_file_creates_and_replaces() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("out.rs");

        write_generated_file(&path, "first").expect("write a new file");
        assert_eq!(fs::read_to_string(&path).expect("read back"), "first");

        write_generated_file(&path, "second").expect("overwrite in place");
        assert_eq!(fs::read_to_string(&path).expect("read back"), "second");

        // No temp files left behind.
        let leftovers: Vec<_> = fs::read_dir(dir.path())
            .expect("read dir")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "temp files leaked: {leftovers:?}");
    }

    #[cfg(unix)]
    #[test]
    fn write_generated_file_does_not_follow_a_planted_symlink() {
        let dir = tempfile::tempdir().expect("tempdir");
        let outside = dir.path().join("precious.txt");
        fs::write(&outside, "DO NOT OVERWRITE").expect("seed the victim file");

        let entities = dir.path().join("entities");
        fs::create_dir(&entities).expect("mkdir entities");
        let target = entities.join("users.rs");
        std::os::unix::fs::symlink(&outside, &target).expect("plant the symlink");

        write_generated_file(&target, "generated").expect("write must succeed");

        assert_eq!(
            fs::read_to_string(&outside).expect("victim still readable"),
            "DO NOT OVERWRITE",
            "the write followed the symlink and clobbered the link target"
        );
        assert_eq!(
            fs::read_to_string(&target).expect("read output"),
            "generated"
        );
        assert!(
            !target
                .symlink_metadata()
                .expect("stat output")
                .file_type()
                .is_symlink(),
            "the symlink should have been replaced by a regular file"
        );
    }

    #[test]
    fn write_generated_file_errors_when_the_directory_is_missing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("nope").join("out.rs");
        let err = write_generated_file(&path, "x").expect_err("a missing parent must be an error");
        assert!(
            err.contains("temporary file"),
            "the error should say what failed; got: {err}"
        );
    }
}
