//! A table's shape as the database's own catalog reports it: columns with
//! their types and nullability, the primary key, the indexes and the
//! foreign keys. Two tables compare equal when Laravel's migration and the
//! framework's would be interchangeable.
//!
//! Types are normalized per engine to what the engine acts on, so the
//! comparison is about the column, not the spelling: SQLite reports the
//! declared text, which differs between Laravel (`varchar`, `datetime`) and
//! sea-query (`varchar(255)`, `datetime_text`) for the same affinity.

use std::collections::BTreeSet;

use sea_orm::{ConnectionTrait, DatabaseConnection, DbBackend, Statement};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Column {
    pub name: String,
    pub kind: String,
    pub nullable: bool,
    /// The default as text, lowercased; only compared where a test asks.
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Index {
    pub name: String,
    pub columns: Vec<String>,
    pub unique: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ForeignKey {
    pub column: String,
    pub references: String,
    pub on_delete: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    pub columns: Vec<Column>,
    pub primary: Vec<String>,
    pub indexes: BTreeSet<Index>,
    pub foreign_keys: BTreeSet<ForeignKey>,
}

async fn query(
    conn: &DatabaseConnection,
    sql: &str,
    values: Vec<sea_orm::Value>,
) -> Vec<serde_json::Value> {
    let backend = conn.get_database_backend();
    conn.query_all_raw(Statement::from_sql_and_values(backend, sql, values))
        .await
        .unwrap_or_else(|e| panic!("catalog query failed: {e}\n{sql}"))
        .iter()
        .map(crate::support::row_json)
        .collect()
}

fn s(row: &serde_json::Value, key: &str) -> String {
    match &row[key] {
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn opt(row: &serde_json::Value, key: &str) -> Option<String> {
    match &row[key] {
        serde_json::Value::Null => None,
        serde_json::Value::String(text) => Some(text.to_lowercase()),
        other => Some(other.to_string()),
    }
}

/// SQLite's declared types, folded to the family the engine acts on.
fn sqlite_kind(declared: &str) -> String {
    let declared = declared.to_lowercase();
    if declared.contains("int") {
        "integer".into()
    } else if declared.contains("date") || declared.contains("time") {
        "datetime".into()
    } else if declared.contains("char") || declared.contains("text") || declared.contains("clob") {
        "text".into()
    } else {
        declared
    }
}

/// MySQL's `COLUMN_TYPE` without an integer's display width.
fn mysql_kind(column_type: &str) -> String {
    let lower = column_type.to_lowercase();
    for integer in ["tinyint", "smallint", "mediumint", "bigint", "int"] {
        if let Some(rest) = lower.strip_prefix(integer)
            && let Some((width, tail)) = rest.strip_prefix('(').and_then(|r| r.split_once(')'))
            && width.bytes().all(|b| b.is_ascii_digit())
        {
            return format!("{integer}{tail}");
        }
    }
    lower
}

pub async fn shape(conn: &DatabaseConnection, table: &str) -> Shape {
    match conn.get_database_backend() {
        DbBackend::Sqlite => sqlite_shape(conn, table).await,
        DbBackend::Postgres => postgres_shape(conn, table).await,
        DbBackend::MySql => mysql_shape(conn, table).await,
        other => panic!("no catalog for {other:?}"),
    }
}

async fn sqlite_shape(conn: &DatabaseConnection, table: &str) -> Shape {
    let rows = query(conn, &format!("PRAGMA table_info(\"{table}\")"), vec![]).await;
    assert!(!rows.is_empty(), "table {table} does not exist");
    let mut primary: Vec<(i64, String)> = Vec::new();
    let columns = rows
        .iter()
        .map(|row| {
            let pk = crate::support::int(row, "pk");
            if pk > 0 {
                primary.push((pk, s(row, "name")));
            }
            Column {
                name: s(row, "name"),
                kind: sqlite_kind(&s(row, "type")),
                // An `INTEGER PRIMARY KEY` is never NULL.
                nullable: crate::support::int(row, "notnull") == 0 && pk == 0,
                default: opt(row, "dflt_value"),
            }
        })
        .collect();
    primary.sort();
    let mut indexes = BTreeSet::new();
    for index in query(conn, &format!("PRAGMA index_list(\"{table}\")"), vec![]).await {
        if s(&index, "origin") == "pk" {
            continue;
        }
        let name = s(&index, "name");
        let columns = query(conn, &format!("PRAGMA index_info(\"{name}\")"), vec![])
            .await
            .iter()
            .map(|c| s(c, "name"))
            .collect();
        indexes.insert(Index {
            // An index SQLite made for an inline UNIQUE has no name of
            // Laravel's; name it by its columns instead.
            name: if name.starts_with("sqlite_autoindex") {
                String::new()
            } else {
                name
            },
            columns,
            unique: crate::support::int(&index, "unique") == 1,
        });
    }
    let foreign_keys = query(
        conn,
        &format!("PRAGMA foreign_key_list(\"{table}\")"),
        vec![],
    )
    .await
    .iter()
    .map(|fk| ForeignKey {
        column: s(fk, "from"),
        references: format!("{}.{}", s(fk, "table"), s(fk, "to")),
        on_delete: s(fk, "on_delete").to_lowercase(),
    })
    .collect();
    Shape {
        columns,
        primary: primary.into_iter().map(|(_, name)| name).collect(),
        indexes,
        foreign_keys,
    }
}

async fn postgres_shape(conn: &DatabaseConnection, table: &str) -> Shape {
    let rows = query(
        conn,
        "SELECT column_name::text AS name, data_type::text AS data_type, \
         is_nullable::text AS nullable, column_default::text AS dflt, \
         character_maximum_length::bigint AS len, datetime_precision::bigint AS fsp \
         FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
        vec![table.into()],
    )
    .await;
    assert!(!rows.is_empty(), "table {table} does not exist");
    let columns = rows
        .iter()
        .map(|row| {
            let data_type = s(row, "data_type");
            let kind = match data_type.as_str() {
                "character varying" | "character" => {
                    format!("{data_type}({})", s(row, "len"))
                }
                "timestamp without time zone" | "timestamp with time zone" => {
                    format!("{data_type}({})", s(row, "fsp"))
                }
                _ => data_type,
            };
            Column {
                name: s(row, "name"),
                kind,
                nullable: s(row, "nullable") == "YES",
                default: opt(row, "dflt"),
            }
        })
        .collect();
    let primary = query(
        conn,
        "SELECT a.attname::text AS name FROM pg_index i \
         JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY(i.indkey) \
         WHERE i.indrelid = $1::regclass AND i.indisprimary \
         ORDER BY array_position(i.indkey, a.attnum)",
        vec![table.into()],
    )
    .await
    .iter()
    .map(|row| s(row, "name"))
    .collect();
    let mut indexes = BTreeSet::new();
    for index in query(
        conn,
        "SELECT c.relname::text AS name, i.indisunique AS uniq, \
         array_to_string(ARRAY(SELECT a.attname FROM unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord) \
           JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum ORDER BY k.ord), ',') AS cols \
         FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid \
         WHERE i.indrelid = $1::regclass AND NOT i.indisprimary",
        vec![table.into()],
    )
    .await
    {
        indexes.insert(Index {
            name: s(&index, "name"),
            columns: s(&index, "cols").split(',').map(str::to_owned).collect(),
            unique: index["uniq"] == serde_json::Value::Bool(true),
        });
    }
    let foreign_keys = query(
        conn,
        "SELECT kcu.column_name::text AS col, ccu.table_name::text AS ref_table, \
         ccu.column_name::text AS ref_col, rc.delete_rule::text AS on_delete \
         FROM information_schema.referential_constraints rc \
         JOIN information_schema.key_column_usage kcu \
           ON kcu.constraint_name = rc.constraint_name AND kcu.constraint_schema = rc.constraint_schema \
         JOIN information_schema.constraint_column_usage ccu \
           ON ccu.constraint_name = rc.constraint_name AND ccu.constraint_schema = rc.constraint_schema \
         WHERE kcu.table_schema = current_schema() AND kcu.table_name = $1",
        vec![table.into()],
    )
    .await
    .iter()
    .map(|fk| ForeignKey {
        column: s(fk, "col"),
        references: format!("{}.{}", s(fk, "ref_table"), s(fk, "ref_col")),
        on_delete: s(fk, "on_delete").to_lowercase(),
    })
    .collect();
    Shape {
        columns,
        primary,
        indexes,
        foreign_keys,
    }
}

async fn mysql_shape(conn: &DatabaseConnection, table: &str) -> Shape {
    let rows = query(
        conn,
        "SELECT COLUMN_NAME AS name, COLUMN_TYPE AS column_type, IS_NULLABLE AS nullable, \
         COLUMN_DEFAULT AS dflt FROM information_schema.COLUMNS \
         WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ? ORDER BY ORDINAL_POSITION",
        vec![table.into()],
    )
    .await;
    assert!(!rows.is_empty(), "table {table} does not exist");
    let columns = rows
        .iter()
        .map(|row| Column {
            name: s(row, "name"),
            kind: mysql_kind(&s(row, "column_type")),
            nullable: s(row, "nullable") == "YES",
            default: opt(row, "dflt"),
        })
        .collect();
    let mut primary = Vec::new();
    let mut by_index: std::collections::BTreeMap<String, (bool, Vec<(i64, String)>)> =
        std::collections::BTreeMap::new();
    for row in query(
        conn,
        "SELECT INDEX_NAME AS name, NON_UNIQUE AS non_unique, SEQ_IN_INDEX AS seq, \
         COLUMN_NAME AS col FROM information_schema.STATISTICS \
         WHERE TABLE_SCHEMA = DATABASE() AND TABLE_NAME = ?",
        vec![table.into()],
    )
    .await
    {
        let name = s(&row, "name");
        let entry = by_index
            .entry(name)
            .or_insert((crate::support::int(&row, "non_unique") == 0, Vec::new()));
        entry
            .1
            .push((crate::support::int(&row, "seq"), s(&row, "col")));
    }
    let mut indexes = BTreeSet::new();
    for (name, (unique, mut cols)) in by_index {
        cols.sort();
        let columns: Vec<String> = cols.into_iter().map(|(_, c)| c).collect();
        if name == "PRIMARY" {
            primary = columns;
        } else {
            indexes.insert(Index {
                name,
                columns,
                unique,
            });
        }
    }
    let foreign_keys = query(
        conn,
        "SELECT k.COLUMN_NAME AS col, k.REFERENCED_TABLE_NAME AS ref_table, \
         k.REFERENCED_COLUMN_NAME AS ref_col, r.DELETE_RULE AS on_delete \
         FROM information_schema.KEY_COLUMN_USAGE k \
         JOIN information_schema.REFERENTIAL_CONSTRAINTS r \
           ON r.CONSTRAINT_NAME = k.CONSTRAINT_NAME AND r.CONSTRAINT_SCHEMA = k.CONSTRAINT_SCHEMA \
         WHERE k.TABLE_SCHEMA = DATABASE() AND k.TABLE_NAME = ? \
           AND k.REFERENCED_TABLE_NAME IS NOT NULL",
        vec![table.into()],
    )
    .await
    .iter()
    .map(|fk| ForeignKey {
        column: s(fk, "col"),
        references: format!("{}.{}", s(fk, "ref_table"), s(fk, "ref_col")),
        on_delete: s(fk, "on_delete").to_lowercase(),
    })
    .collect();
    Shape {
        columns,
        primary,
        indexes,
        foreign_keys,
    }
}

/// Every table in the database, by name.
pub async fn tables(conn: &DatabaseConnection) -> BTreeSet<String> {
    let sql = match conn.get_database_backend() {
        DbBackend::Sqlite => {
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%'"
        }
        DbBackend::Postgres => {
            "SELECT table_name::text AS name FROM information_schema.tables \
             WHERE table_schema = current_schema() AND table_type = 'BASE TABLE'"
        }
        _ => {
            "SELECT TABLE_NAME AS name FROM information_schema.TABLES \
             WHERE TABLE_SCHEMA = DATABASE() AND TABLE_TYPE = 'BASE TABLE'"
        }
    };
    query(conn, sql, vec![])
        .await
        .iter()
        .map(|row| s(row, "name"))
        .collect()
}
