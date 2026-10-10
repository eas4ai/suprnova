//! Turns a recorded [`Blueprint`] into the statements to run.
//!
//! Planning is pure: it reads the description and the backend and returns
//! either the ordered statements or the error. Nothing is sent to the
//! database here, which is what lets a refusal happen before the first
//! statement of a call.

use std::collections::HashSet;

use sea_orm::sea_query::{
    Expr, ForeignKey, ForeignKeyAction, ForeignKeyCreateStatement, ForeignKeyDropStatement, Index,
    IndexCreateStatement, IndexDropStatement, Table, TableAlterStatement, TableCreateStatement,
    Value,
};
use sea_orm::{DbBackend, DbErr};

use super::blueprint::{Blueprint, Command, FullTextSpec, IndexSpec, full_text_name};
use super::column::ColumnKind;
use super::foreign::ForeignSpec;
use super::{quote_ident, quote_table, sea_ident, sea_table};

/// Longest identifier Postgres keeps: 63 bytes. The builder applies it on
/// every backend so a migration that runs on one runs on all three. The plan
/// refuses a longer index or foreign key name instead of letting Postgres
/// truncate it into a name that may collide with another.
const MAX_NAME_BYTES: usize = 63;

/// One statement of a plan, in the form `SchemaManager` runs.
pub(crate) enum Step {
    CreateTable(TableCreateStatement),
    AlterTable(TableAlterStatement),
    CreateIndex(IndexCreateStatement),
    DropIndex(IndexDropStatement),
    CreateForeignKey(ForeignKeyCreateStatement),
    DropForeignKey(ForeignKeyDropStatement),
    /// A statement sea-query cannot build: adding a primary key to an
    /// existing table, or a full-text index. Its identifiers are quoted by
    /// [`quote_ident`].
    Raw(String),
}

impl Step {
    /// Renders the planned statement through the execution backend's grammar.
    pub(crate) fn sql(self, backend: DbBackend) -> String {
        match self {
            Self::CreateTable(statement) => backend.build(&statement).sql,
            Self::AlterTable(statement) => backend.build(&statement).sql,
            Self::CreateIndex(statement) => backend.build(&statement).sql,
            Self::DropIndex(statement) => backend.build(&statement).sql,
            Self::CreateForeignKey(statement) => backend.build(&statement).sql,
            Self::DropForeignKey(statement) => backend.build(&statement).sql,
            Self::Raw(sql) => sql,
        }
    }
}

fn refuse(message: String) -> DbErr {
    DbErr::Migration(message)
}

fn check_blueprint(blueprint: &Blueprint) -> Result<(), DbErr> {
    if blueprint.table().is_empty() {
        return Err(refuse(
            "schema: the table name is empty; give the table a name".to_owned(),
        ));
    }
    if let Some(fault) = blueprint.faults().first() {
        return Err(refuse(fault.clone()));
    }
    for column in blueprint.columns() {
        if column.name.is_empty() {
            return Err(refuse(format!(
                "schema: cannot add a column with an empty name to table `{}`",
                blueprint.table()
            )));
        }
        if column.kind == ColumnKind::Ulid && column.length == Some(0) {
            return Err(refuse(format!(
                "schema: ULID column `{}` of table `{}` needs a positive length",
                column.name,
                blueprint.table()
            )));
        }
        if column.kind == ColumnKind::Enum
            && let Some(Expr::Value(Value::String(Some(default)))) = &column.default
            && !column.allowed.contains(default)
        {
            return Err(refuse(format!(
                "schema: the default `{default}` of enumeration `{}` on table `{}` is not one of its values",
                column.name,
                blueprint.table()
            )));
        }
        if column.use_current && column.default.is_some() {
            return Err(refuse(format!(
                "schema: column `{}` of table `{}` has both use_current() and default(..); keep one",
                column.name,
                blueprint.table()
            )));
        }
    }
    Ok(())
}

/// Refuses a schema-qualified table name the builder cannot take: one with
/// more than one `.`, and any outside Postgres. SeaQuery writes an index
/// on a table in a named schema for Postgres only, and SQLite's
/// `CREATE INDEX` cannot name one at all.
fn check_qualified(table: &str, backend: DbBackend) -> Result<(), DbErr> {
    if table.matches('.').count() > 1 {
        return Err(refuse(format!(
            "schema: the table name `{table}` has more than one `.`; name a table as `table` or `schema.table`"
        )));
    }
    if backend != DbBackend::Postgres && table.contains('.') {
        return Err(refuse(format!(
            "schema: the schema-qualified table `{table}` is supported on Postgres only; on {backend:?} name the table without its schema"
        )));
    }
    Ok(())
}

/// Refuses a foreign key whose action sets its column to `NULL` when the
/// closure declares that column `NOT NULL`. MySQL would refuse the key only
/// after the column was added, and MySQL cannot roll the column back.
fn check_null_action(blueprint: &Blueprint, foreign: &ForeignSpec) -> Result<(), DbErr> {
    let sets_null =
        |action: Option<ForeignKeyAction>| matches!(action, Some(ForeignKeyAction::SetNull));
    if !sets_null(foreign.on_delete) && !sets_null(foreign.on_update) {
        return Ok(());
    }
    let declared_not_null = blueprint
        .columns()
        .iter()
        .any(|column| column.name == foreign.column && !column.nullable);
    if declared_not_null {
        return Err(refuse(format!(
            "schema: the foreign key on column `{}` of table `{}` sets it to NULL (null_on_delete or null_on_update), but the column is NOT NULL; call .nullable()",
            foreign.column,
            blueprint.table()
        )));
    }
    Ok(())
}

fn check_name_length(table: &str, name: &str, what: &str) -> Result<(), DbErr> {
    if name.len() > MAX_NAME_BYTES {
        return Err(refuse(format!(
            "schema: the {what} name `{name}` on table `{table}` is {} bytes, more than the {MAX_NAME_BYTES} bytes Postgres keeps, the limit the builder uses on every backend so a migration runs on all three; shorten the table or column names",
            name.len()
        )));
    }
    Ok(())
}

/// `name` as sea-query must be handed an index or foreign key name for
/// `backend`. sea-query writes such a name between the backend's quotes
/// without escaping it, so the quote character inside the name is doubled
/// here, the way [`quote_ident`] doubles it in the statements the builder
/// writes itself and sea-query doubles it in table and column names.
fn constraint_name(backend: DbBackend, name: &str) -> String {
    let quote = if backend == DbBackend::MySql {
        '`'
    } else {
        '"'
    };
    name.replace(quote, &format!("{quote}{quote}"))
}

fn index_statement(
    backend: DbBackend,
    table: &str,
    name: &str,
    spec: &IndexSpec,
) -> IndexCreateStatement {
    let mut statement = Index::create();
    statement
        .name(constraint_name(backend, name))
        .table(sea_table(table));
    for column in &spec.columns {
        statement.col(sea_ident(column));
    }
    if spec.unique {
        statement.unique();
    }
    statement
}

fn check_index(table: &str, spec: &IndexSpec, seen: &mut HashSet<String>) -> Result<String, DbErr> {
    check_named_index(table, &spec.columns, spec.name(table), seen)
}

/// Checks an index over `columns` called `name`: it has named columns, a
/// name Postgres keeps whole, and a name no other index or key of the
/// closure has. Returns the name.
fn check_named_index(
    table: &str,
    columns: &[String],
    name: String,
    seen: &mut HashSet<String>,
) -> Result<String, DbErr> {
    if columns.is_empty() || columns.iter().any(String::is_empty) {
        return Err(refuse(format!(
            "schema: cannot create an index on table `{table}` without named columns"
        )));
    }
    check_name_length(table, &name, "index")?;
    if !seen.insert(name.clone()) {
        return Err(refuse(format!(
            "schema: table `{table}` names two indexes or keys `{name}`, twice the same name; declare the index once, or give a key its own name with .name(..)"
        )));
    }
    Ok(name)
}

/// The refusal of a full-text operation on SQLite, which has neither a
/// `FULLTEXT` index nor a `GIN` one. `call` is the blueprint method.
fn sqlite_full_text_refusal(table: &str, call: &str, columns: &[String]) -> DbErr {
    refuse(format!(
        "schema: {call}({columns:?}) on table `{table}` is not supported on SQLite: SQLite has no FULLTEXT or GIN index; run full-text search on MySQL, MariaDB or Postgres, or create an FTS5 virtual table with SeaORM's SchemaManager"
    ))
}

/// Checks a full-text index for `backend` and returns its name: SQLite
/// refuses it, and otherwise it is checked as any index, with a language
/// that must be a plain name, since it is written into the SQL.
fn check_full_text(
    table: &str,
    spec: &FullTextSpec,
    backend: DbBackend,
    seen: &mut HashSet<String>,
) -> Result<String, DbErr> {
    if backend == DbBackend::Sqlite {
        return Err(sqlite_full_text_refusal(table, "full_text", &spec.columns));
    }
    let name = check_named_index(
        table,
        &spec.columns,
        full_text_name(table, &spec.columns),
        seen,
    )?;
    if let Some(language) = &spec.language {
        crate::database::full_text::check_language(language).map_err(|reason| {
            refuse(format!(
                "schema: the full-text index `{name}` on table `{table}` cannot be created: {reason}"
            ))
        })?;
    }
    Ok(name)
}

/// `CREATE FULLTEXT INDEX` on MySQL and MariaDB, and on Postgres
/// `CREATE INDEX .. USING gin` over the `to_tsvector` document
/// `where_full_text` searches, as Laravel's grammars write them. sea-query
/// has no expression index, so the builder writes the statement itself.
fn full_text_sql(backend: DbBackend, table: &str, name: &str, spec: &FullTextSpec) -> String {
    let index = quote_ident(backend, name);
    let on = quote_table(backend, table);
    if backend == DbBackend::Postgres {
        let language = spec
            .language
            .as_deref()
            .unwrap_or(crate::database::full_text::DEFAULT_LANGUAGE);
        let document = crate::database::full_text::postgres_document(
            spec.columns.iter().map(String::as_str),
            language,
            |column| quote_ident(backend, column),
        );
        format!("CREATE INDEX {index} ON {on} USING gin (({document}))")
    } else {
        let columns = spec
            .columns
            .iter()
            .map(|column| quote_ident(backend, column))
            .collect::<Vec<_>>()
            .join(", ");
        format!("CREATE FULLTEXT INDEX {index} ON {on} ({columns})")
    }
}

/// Returns the referenced table of `foreign`, or the error for a foreign key
/// that has actions but no table.
fn referenced_table<'a>(table: &str, foreign: &'a ForeignSpec) -> Result<Option<&'a str>, DbErr> {
    match &foreign.ref_table {
        Some(ref_table) if ref_table.is_empty() => Err(refuse(format!(
            "schema: the foreign key on column `{}` of table `{table}` references a table with an empty name",
            foreign.column
        ))),
        Some(ref_table) => Ok(Some(ref_table.as_str())),
        None if foreign.explicit => Err(refuse(format!(
            "schema: foreign(`{}`) on table `{table}` names no referenced table; call references(table, column) or constrained(table)",
            foreign.column
        ))),
        None if foreign.on_delete.is_some() || foreign.on_update.is_some() => Err(refuse(format!(
            "schema: column `{}` of table `{table}` has on_delete or on_update but no referenced table; call constrained(table) or references(table, column) first",
            foreign.column
        ))),
        None => Ok(None),
    }
}

fn foreign_statement(
    backend: DbBackend,
    table: &str,
    name: &str,
    foreign: &ForeignSpec,
    ref_table: &str,
) -> ForeignKeyCreateStatement {
    let mut statement = ForeignKeyCreateStatement::new();
    statement
        .name(constraint_name(backend, name))
        .from(sea_table(table), sea_ident(&foreign.column))
        .to(sea_table(ref_table), sea_ident(&foreign.ref_column));
    if let Some(action) = foreign.on_delete {
        statement.on_delete(action);
    }
    if let Some(action) = foreign.on_update {
        statement.on_update(action);
    }
    statement
}

fn check_foreign_name(
    table: &str,
    foreign: &ForeignSpec,
    seen: &mut HashSet<String>,
) -> Result<String, DbErr> {
    if foreign.column.is_empty() {
        return Err(refuse(format!(
            "schema: cannot create a foreign key on table `{table}` over a column with an empty name"
        )));
    }
    let name = foreign.name(table);
    if name.is_empty() {
        return Err(refuse(format!(
            "schema: the foreign key on column `{}` of table `{table}` has an empty name",
            foreign.column
        )));
    }
    check_name_length(table, &name, "foreign key")?;
    if !seen.insert(name.clone()) {
        return Err(refuse(format!(
            "schema: table `{table}` names two indexes or keys `{name}`, twice the same name; give each key its own name with .name(..)"
        )));
    }
    Ok(name)
}

fn check_primary_columns(table: &str, columns: &[String]) -> Result<(), DbErr> {
    if columns.is_empty() || columns.iter().any(String::is_empty) {
        return Err(refuse(format!(
            "schema: cannot make a primary key on table `{table}` without named columns"
        )));
    }
    Ok(())
}

fn second_primary_key(table: &str) -> DbErr {
    refuse(format!(
        "schema: table `{table}` declares a second primary key (id(), primary() or .primary()); a table has one"
    ))
}

/// `ALTER TABLE .. ADD PRIMARY KEY (..)`, which sea-query has no statement
/// for. The key is unnamed, as Laravel's is: Postgres names it
/// `{table}_pkey`, MySQL `PRIMARY`.
fn add_primary_sql(backend: DbBackend, table: &str, columns: &[String]) -> String {
    let columns = columns
        .iter()
        .map(|column| quote_ident(backend, column))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "ALTER TABLE {} ADD PRIMARY KEY ({columns})",
        quote_table(backend, table)
    )
}

/// Plans `Schema::create`: the table with its columns and inline foreign
/// keys first, then one statement per index.
///
/// On MySQL the table takes the blueprint's character set and collation, or
/// else `defaults`, as Laravel's `MySqlGrammar::compileCreateEncoding` does;
/// the other backends have no table encoding and ignore both.
pub(crate) fn plan_create(
    blueprint: &Blueprint,
    backend: DbBackend,
    defaults: &TableEncoding,
) -> Result<Vec<Step>, DbErr> {
    check_blueprint(blueprint)?;
    check_qualified(blueprint.table(), backend)?;
    let table = blueprint.table();
    let mut create = Table::create();
    create.table(sea_table(table));
    if backend == DbBackend::MySql {
        let charset = blueprint.table_charset().or(defaults.charset.as_deref());
        let collation = blueprint
            .table_collation()
            .or(defaults.collation.as_deref());
        if let Some(charset) = charset {
            check_encoding_name(table, "charset", charset)?;
            create.character_set(charset);
        }
        if let Some(collation) = collation {
            check_encoding_name(table, "collation", collation)?;
            create.collate(collation);
        }
    } else {
        // Checked on every backend, so a migration that is wrong on MySQL
        // fails on the SQLite a developer runs too.
        if let Some(charset) = blueprint.table_charset() {
            check_encoding_name(table, "charset", charset)?;
        }
        if let Some(collation) = blueprint.table_collation() {
            check_encoding_name(table, "collation", collation)?;
        }
    }
    let mut indexes = Vec::new();
    let mut seen_columns = HashSet::new();
    let mut nullable_columns = HashSet::new();
    // Index and foreign key names share one set: MySQL names the index
    // behind a foreign key after the key, so the two cannot collide.
    let mut seen_names = HashSet::new();
    let mut explicit_foreign_columns = Vec::new();
    let mut ids = 0_usize;
    let mut primary: Option<&[String]> = None;
    for command in blueprint.commands() {
        match command {
            Command::AddColumn(position) => {
                let Some(column) = blueprint.columns().get(*position) else {
                    continue;
                };
                if !seen_columns.insert(column.name.clone()) {
                    return Err(refuse(format!(
                        "schema: table `{table}` declares the column `{}` twice; declare it once",
                        column.name
                    )));
                }
                if column.kind == ColumnKind::Id {
                    ids += 1;
                    if ids > 1 {
                        return Err(refuse(format!(
                            "schema: table `{table}` declares id() twice; a table has one primary key"
                        )));
                    }
                    if primary.is_some() {
                        return Err(second_primary_key(table));
                    }
                }
                if let Some(after) = &column.after {
                    return Err(refuse(format!(
                        "schema: after(`{after}`) on column `{}` of table `{table}` positions a column added to an existing table; MySQL refuses it in CREATE TABLE, so declare the columns in order instead",
                        column.name
                    )));
                }
                if column.nullable {
                    nullable_columns.insert(column.name.clone());
                }
                create.col(column.to_column_def(backend));
            }
            Command::AddIndex(spec) => {
                let name = check_index(table, spec, &mut seen_names)?;
                indexes.push(Step::CreateIndex(index_statement(
                    backend, table, &name, spec,
                )));
            }
            Command::AddFullText(spec) => {
                let name = check_full_text(table, spec, backend, &mut seen_names)?;
                indexes.push(Step::Raw(full_text_sql(backend, table, &name, spec)));
            }
            Command::AddForeign(position) => {
                let Some(foreign) = blueprint.foreigns().get(*position) else {
                    continue;
                };
                let Some(ref_table) = referenced_table(table, foreign)? else {
                    continue;
                };
                check_null_action(blueprint, foreign)?;
                let name = check_foreign_name(table, foreign, &mut seen_names)?;
                if foreign.explicit {
                    explicit_foreign_columns.push((name.clone(), foreign.column.clone()));
                }
                create.foreign_key(&mut foreign_statement(
                    backend, table, &name, foreign, ref_table,
                ));
            }
            Command::AddPrimary(columns) => {
                check_primary_columns(table, columns)?;
                if primary.is_some() || ids > 0 {
                    return Err(second_primary_key(table));
                }
                primary = Some(columns.as_slice());
            }
            Command::RenameColumn { from, .. } => {
                return Err(only_in_table(table, "rename_column", from));
            }
            Command::DropColumn(name) => return Err(only_in_table(table, "drop_column", name)),
            Command::DropIndex(name) => return Err(only_in_table(table, "drop_index", name)),
            Command::DropFullText(columns) => {
                let name = full_text_name(table, columns);
                return Err(only_in_table(table, "drop_full_text", &name));
            }
            Command::DropForeign(name) => return Err(only_in_table(table, "drop_foreign", name)),
        }
    }
    if seen_columns.is_empty() {
        return Err(refuse(format!(
            "schema: cannot create table `{table}` without columns; add at least one"
        )));
    }
    for command in blueprint.commands() {
        let (name, columns) = match command {
            Command::AddIndex(spec) => (spec.name(table), &spec.columns),
            Command::AddFullText(spec) => (full_text_name(table, &spec.columns), &spec.columns),
            _ => continue,
        };
        for column in columns {
            if !seen_columns.contains(column) {
                return Err(refuse(format!(
                    "schema: the index `{name}` on table `{table}` names the column `{column}`, which the table does not declare; check the spelling or declare the column"
                )));
            }
        }
    }
    for (name, column) in &explicit_foreign_columns {
        if !seen_columns.contains(column) {
            return Err(refuse(format!(
                "schema: the foreign key `{name}` on table `{table}` names the column `{column}`, which the table does not declare; check the spelling or declare the column"
            )));
        }
    }
    if let Some(columns) = primary {
        let mut key = Index::create();
        for column in columns {
            if !seen_columns.contains(column) {
                return Err(refuse(format!(
                    "schema: the primary key of table `{table}` names the column `{column}`, which the table does not declare; check the spelling or declare the column"
                )));
            }
            if nullable_columns.contains(column) {
                return Err(nullable_primary(table, column));
            }
            key.col(sea_ident(column));
        }
        create.primary_key(&mut key);
    }
    let mut steps = vec![Step::CreateTable(create)];
    steps.extend(indexes);
    Ok(steps)
}

/// The character set and collation a new MySQL table takes when its
/// blueprint names none: the configured ones.
#[derive(Debug, Clone, Default)]
pub(crate) struct TableEncoding {
    pub(crate) charset: Option<String>,
    pub(crate) collation: Option<String>,
}

/// Refuses a character set or collation that is not a plain name: sea-query
/// writes a table's options unquoted.
fn check_encoding_name(table: &str, what: &str, name: &str) -> Result<(), DbErr> {
    if crate::database::config::is_encoding_name(name) {
        return Ok(());
    }
    Err(refuse(format!(
        "schema: the {what} {name:?} of table `{table}` is not a name; use letters, digits and underscores, such as utf8mb4_unicode_ci"
    )))
}

fn nullable_primary(table: &str, column: &str) -> DbErr {
    refuse(format!(
        "schema: column `{column}` of table `{table}` is nullable and cannot be part of the primary key; drop .nullable()"
    ))
}

fn only_in_table(table: &str, operation: &str, target: &str) -> DbErr {
    refuse(format!(
        "schema: {operation}(`{target}`) on table `{table}` changes an existing table; use Schema::table instead of Schema::create"
    ))
}

fn sqlite_foreign_refusal(table: &str, operation: &str, target: &str) -> DbErr {
    refuse(format!(
        "schema: cannot {operation} `{target}` on the existing table `{table}`: SQLite cannot add or drop a foreign key on an existing table; create the key with the table in Schema::create, or write this step with SeaORM's SchemaManager"
    ))
}

/// Refuses, for SQLite, every foreign key operation of an alteration. It runs
/// before the rest of the plan so that the refusal names the foreign key even
/// when the column that carries it would also be refused.
fn refuse_sqlite_foreign_keys(blueprint: &Blueprint) -> Result<(), DbErr> {
    let table = blueprint.table();
    for command in blueprint.commands() {
        match command {
            Command::AddForeign(position) => {
                let Some(foreign) = blueprint.foreigns().get(*position) else {
                    continue;
                };
                if referenced_table(table, foreign)?.is_some() {
                    let name = foreign.name(table);
                    return Err(sqlite_foreign_refusal(table, "add the foreign key", &name));
                }
            }
            Command::DropForeign(name) => {
                return Err(sqlite_foreign_refusal(table, "drop the foreign key", name));
            }
            _ => {}
        }
    }
    Ok(())
}

/// `DROP INDEX` for the index `name` on `table`, as `drop_index` and
/// `drop_full_text` drop it: `DROP INDEX name ON table` on MySQL, `DROP INDEX
/// name` on Postgres.
fn drop_index_statement(backend: DbBackend, table: &str, name: &str) -> IndexDropStatement {
    let mut statement = Index::drop();
    statement
        .name(constraint_name(backend, name))
        .table(sea_table(table));
    statement
}

/// Plans `Schema::table`: every alteration as its own statement, in the
/// order the closure recorded them.
///
/// SQLite accepts one alteration per `ALTER TABLE`, so no statement holds
/// more than one, on any backend. The refusals for SQLite are returned here,
/// before any statement of the call runs, because sea-query panics when it
/// builds them.
pub(crate) fn plan_alter(blueprint: &Blueprint, backend: DbBackend) -> Result<Vec<Step>, DbErr> {
    check_blueprint(blueprint)?;
    check_qualified(blueprint.table(), backend)?;
    let table = blueprint.table();
    for (operation, name) in [
        ("charset", blueprint.table_charset()),
        ("collation", blueprint.table_collation()),
    ] {
        if let Some(name) = name {
            return Err(refuse(format!(
                "schema: {operation}(`{name}`) on table `{table}` sets the encoding of a new table; an existing table keeps its own, so name it in Schema::create"
            )));
        }
    }
    let sqlite = backend == DbBackend::Sqlite;
    if sqlite {
        refuse_sqlite_foreign_keys(blueprint)?;
    }
    let mut steps = Vec::new();
    let mut seen_names = HashSet::new();
    let mut added_columns = HashSet::new();
    let mut nullable_added = HashSet::new();
    let mut primaries = 0_usize;
    for command in blueprint.commands() {
        match command {
            Command::AddColumn(position) => {
                let Some(column) = blueprint.columns().get(*position) else {
                    continue;
                };
                if !added_columns.insert(column.name.clone()) {
                    return Err(refuse(format!(
                        "schema: the call adds the column `{}` to table `{table}` twice; add it once",
                        column.name
                    )));
                }
                if column.kind == ColumnKind::Id {
                    primaries += 1;
                    if primaries > 1 {
                        return Err(second_primary_key(table));
                    }
                }
                if sqlite && column.kind == ColumnKind::Id {
                    return Err(refuse(format!(
                        "schema: cannot add the primary key column `{}` to the existing table `{table}`: SQLite cannot add a PRIMARY KEY column; create the table with id() instead",
                        column.name
                    )));
                }
                if sqlite && column.use_current {
                    return Err(refuse(format!(
                        "schema: cannot add the column `{}` with use_current() to the existing table `{table}`: SQLite refuses CURRENT_TIMESTAMP as the default of an added column; give it a constant default, or create it with the table",
                        column.name
                    )));
                }
                if column.nullable {
                    nullable_added.insert(column.name.clone());
                }
                if sqlite && !column.nullable && column.default.is_none() {
                    return Err(refuse(format!(
                        "schema: cannot add the NOT NULL column `{}` to the existing table `{table}` without a default: SQLite refuses it; call .nullable() or .default(value)",
                        column.name
                    )));
                }
                let mut alter = Table::alter();
                alter
                    .table(sea_table(table))
                    .add_column(column.to_column_def(backend));
                steps.push(Step::AlterTable(alter));
            }
            Command::RenameColumn { from, to } => {
                if from.is_empty() || to.is_empty() {
                    return Err(refuse(format!(
                        "schema: cannot rename a column of table `{table}` from or to an empty name"
                    )));
                }
                let mut alter = Table::alter();
                alter
                    .table(sea_table(table))
                    .rename_column(sea_ident(from), sea_ident(to));
                steps.push(Step::AlterTable(alter));
            }
            Command::DropColumn(name) => {
                if name.is_empty() {
                    return Err(refuse(format!(
                        "schema: cannot drop a column with an empty name from table `{table}`"
                    )));
                }
                let mut alter = Table::alter();
                alter.table(sea_table(table)).drop_column(sea_ident(name));
                steps.push(Step::AlterTable(alter));
            }
            Command::AddIndex(spec) => {
                let name = check_index(table, spec, &mut seen_names)?;
                steps.push(Step::CreateIndex(index_statement(
                    backend, table, &name, spec,
                )));
            }
            Command::DropIndex(name) => {
                if name.is_empty() {
                    return Err(refuse(format!(
                        "schema: cannot drop an index with an empty name from table `{table}`"
                    )));
                }
                steps.push(Step::DropIndex(drop_index_statement(backend, table, name)));
            }
            Command::AddFullText(spec) => {
                let name = check_full_text(table, spec, backend, &mut seen_names)?;
                steps.push(Step::Raw(full_text_sql(backend, table, &name, spec)));
            }
            Command::DropFullText(columns) => {
                if sqlite {
                    return Err(sqlite_full_text_refusal(table, "drop_full_text", columns));
                }
                if columns.is_empty() || columns.iter().any(String::is_empty) {
                    return Err(refuse(format!(
                        "schema: drop_full_text on table `{table}` names no columns; name the columns of the full-text index"
                    )));
                }
                let name = full_text_name(table, columns);
                steps.push(Step::DropIndex(drop_index_statement(backend, table, &name)));
            }
            Command::AddForeign(position) => {
                let Some(foreign) = blueprint.foreigns().get(*position) else {
                    continue;
                };
                let Some(ref_table) = referenced_table(table, foreign)? else {
                    continue;
                };
                check_null_action(blueprint, foreign)?;
                let name = check_foreign_name(table, foreign, &mut seen_names)?;
                steps.push(Step::CreateForeignKey(foreign_statement(
                    backend, table, &name, foreign, ref_table,
                )));
            }
            Command::AddPrimary(columns) => {
                check_primary_columns(table, columns)?;
                if sqlite {
                    return Err(refuse(format!(
                        "schema: cannot add a primary key to the existing table `{table}`: SQLite cannot; declare it in Schema::create"
                    )));
                }
                primaries += 1;
                if primaries > 1 {
                    return Err(second_primary_key(table));
                }
                if let Some(column) = columns.iter().find(|c| nullable_added.contains(*c)) {
                    return Err(nullable_primary(table, column));
                }
                steps.push(Step::Raw(add_primary_sql(backend, table, columns)));
            }
            Command::DropForeign(name) => {
                let mut statement = ForeignKey::drop();
                statement
                    .name(constraint_name(backend, name))
                    .table(sea_table(table));
                steps.push(Step::DropForeignKey(statement));
            }
        }
    }
    Ok(steps)
}
