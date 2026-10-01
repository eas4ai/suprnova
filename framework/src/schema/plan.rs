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

use super::blueprint::{Blueprint, Command, IndexSpec};
use super::column::ColumnKind;
use super::foreign::ForeignSpec;
use super::{quote_ident, sea_ident};

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
    /// existing table. Its identifiers are quoted by [`quote_ident`].
    Raw(String),
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

fn index_statement(table: &str, name: &str, spec: &IndexSpec) -> IndexCreateStatement {
    let mut statement = Index::create();
    statement.name(name).table(sea_ident(table));
    for column in &spec.columns {
        statement.col(sea_ident(column));
    }
    if spec.unique {
        statement.unique();
    }
    statement
}

fn check_index(table: &str, spec: &IndexSpec, seen: &mut HashSet<String>) -> Result<String, DbErr> {
    if spec.columns.is_empty() || spec.columns.iter().any(String::is_empty) {
        return Err(refuse(format!(
            "schema: cannot create an index on table `{table}` without named columns"
        )));
    }
    let name = spec.name(table);
    check_name_length(table, &name, "index")?;
    if !seen.insert(name.clone()) {
        return Err(refuse(format!(
            "schema: table `{table}` names two indexes or keys `{name}`, twice the same name; declare the index once, or give a key its own name with .name(..)"
        )));
    }
    Ok(name)
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
    table: &str,
    name: &str,
    foreign: &ForeignSpec,
    ref_table: &str,
) -> ForeignKeyCreateStatement {
    let mut statement = ForeignKeyCreateStatement::new();
    statement
        .name(name)
        .from(sea_ident(table), sea_ident(&foreign.column))
        .to(sea_ident(ref_table), sea_ident(&foreign.ref_column));
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
        quote_ident(backend, table)
    )
}

/// Plans `Schema::create`: the table with its columns and inline foreign
/// keys first, then one statement per index.
pub(crate) fn plan_create(blueprint: &Blueprint, backend: DbBackend) -> Result<Vec<Step>, DbErr> {
    check_blueprint(blueprint)?;
    let table = blueprint.table();
    let mut create = Table::create();
    create.table(sea_ident(table));
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
                indexes.push(index_statement(table, &name, spec));
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
                create.foreign_key(&mut foreign_statement(table, &name, foreign, ref_table));
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
            Command::DropForeign(name) => return Err(only_in_table(table, "drop_foreign", name)),
        }
    }
    if seen_columns.is_empty() {
        return Err(refuse(format!(
            "schema: cannot create table `{table}` without columns; add at least one"
        )));
    }
    for command in blueprint.commands() {
        if let Command::AddIndex(spec) = command {
            for column in &spec.columns {
                if !seen_columns.contains(column) {
                    return Err(refuse(format!(
                        "schema: the index `{}` on table `{table}` names the column `{column}`, which the table does not declare; check the spelling or declare the column",
                        spec.name(table)
                    )));
                }
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
    steps.extend(indexes.into_iter().map(Step::CreateIndex));
    Ok(steps)
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

/// Plans `Schema::table`: every alteration as its own statement, in the
/// order the closure recorded them.
///
/// SQLite accepts one alteration per `ALTER TABLE`, so no statement holds
/// more than one, on any backend. The refusals for SQLite are returned here,
/// before any statement of the call runs, because sea-query panics when it
/// builds them.
pub(crate) fn plan_alter(blueprint: &Blueprint, backend: DbBackend) -> Result<Vec<Step>, DbErr> {
    check_blueprint(blueprint)?;
    let table = blueprint.table();
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
                    .table(sea_ident(table))
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
                    .table(sea_ident(table))
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
                alter.table(sea_ident(table)).drop_column(sea_ident(name));
                steps.push(Step::AlterTable(alter));
            }
            Command::AddIndex(spec) => {
                let name = check_index(table, spec, &mut seen_names)?;
                steps.push(Step::CreateIndex(index_statement(table, &name, spec)));
            }
            Command::DropIndex(name) => {
                if name.is_empty() {
                    return Err(refuse(format!(
                        "schema: cannot drop an index with an empty name from table `{table}`"
                    )));
                }
                let mut statement = Index::drop();
                statement.name(name.as_str()).table(sea_ident(table));
                steps.push(Step::DropIndex(statement));
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
                    table, &name, foreign, ref_table,
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
                statement.name(name.as_str()).table(sea_ident(table));
                steps.push(Step::DropForeignKey(statement));
            }
        }
    }
    Ok(steps)
}
