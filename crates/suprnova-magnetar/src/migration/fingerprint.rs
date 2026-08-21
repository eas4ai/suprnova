//! Deterministic table fingerprints for shadow-copy verification.

use std::collections::{BTreeMap, BTreeSet};

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sha2::{Digest, Sha256};

use crate::{Error, Result};

/// A deterministic fingerprint over a table's expected fields and rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TableFingerprint {
    /// Number of rows represented by this fingerprint.
    pub row_count: usize,
    /// Stable sorted field names included in each row.
    pub fields: Vec<String>,
    /// Hex-encoded SHA-256 digest over a canonical field and row encoding.
    pub digest: String,
}

/// One named source-table fingerprint captured in a dry-run plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceTableFingerprint {
    /// Source table name.
    pub table: String,
    /// Canonical field and row digest.
    pub fingerprint: TableFingerprint,
    /// Hex-encoded SHA-256 digest of canonical schema and index metadata.
    pub schema_digest: String,
}

impl TableFingerprint {
    /// Builds a fingerprint from rows with exactly the supplied field set.
    ///
    /// Row order does not affect the digest. Field names and values are
    /// length-prefixed, avoiding delimiter ambiguity.
    pub fn from_rows(fields: &[&str], rows: Vec<BTreeMap<String, String>>) -> Result<Self> {
        if fields.is_empty() {
            return Err(Error::InvalidInput {
                field: "fingerprint fields".to_owned(),
                message: "at least one field is required".to_owned(),
            });
        }
        let mut fields = fields
            .iter()
            .map(|field| (*field).to_owned())
            .collect::<Vec<_>>();
        let declared_count = fields.len();
        fields.sort();
        fields.dedup();
        if fields.len() != declared_count {
            return Err(Error::InvalidInput {
                field: "fingerprint fields".to_owned(),
                message: "duplicate fields are not allowed".to_owned(),
            });
        }

        let mut encoded_rows = Vec::with_capacity(rows.len());
        for row in rows {
            if row.len() != fields.len() || fields.iter().any(|field| !row.contains_key(field)) {
                return Err(Error::InvalidInput {
                    field: "fingerprint row".to_owned(),
                    message: "row fields do not match the declared fingerprint fields".to_owned(),
                });
            }
            let mut encoded = Vec::new();
            for field in &fields {
                append(&mut encoded, field.as_bytes());
                let value = row.get(field).ok_or_else(|| Error::InvalidInput {
                    field: "fingerprint row".to_owned(),
                    message: "row field disappeared during canonical encoding".to_owned(),
                })?;
                append(&mut encoded, value.as_bytes());
            }
            encoded_rows.push(encoded);
        }
        encoded_rows.sort();
        let mut hasher = Sha256::new();
        for field in &fields {
            append_hash(&mut hasher, field.as_bytes());
        }
        for row in &encoded_rows {
            append_hash(&mut hasher, row);
        }
        Ok(Self {
            row_count: encoded_rows.len(),
            fields,
            digest: format!("{:x}", hasher.finalize()),
        })
    }
}
pub(crate) async fn source_database_fingerprints<C: ConnectionTrait + ?Sized>(
    database: &C,
    excluded_tables: &[String],
    excluded_columns: &BTreeMap<String, BTreeSet<String>>,
) -> Result<Vec<SourceTableFingerprint>> {
    let backend = database.get_database_backend();
    let table_query = match backend {
        DbBackend::Sqlite => {
            "SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name"
        }
        DbBackend::Postgres => {
            "SELECT table_name FROM information_schema.tables WHERE table_schema = current_schema() AND table_type = 'BASE TABLE' ORDER BY table_name"
        }
        DbBackend::MySql => {
            "SELECT table_name FROM information_schema.tables WHERE table_schema = DATABASE() AND table_type = 'BASE TABLE' ORDER BY table_name"
        }
    };
    let tables = database
        .query_all(Statement::from_string(backend, table_query))
        .await
        .map_err(|error| super::database_error("listing source fingerprint tables", error))?;
    let mut fingerprints = Vec::with_capacity(tables.len());
    for table_row in tables {
        let table: String = table_row
            .try_get_by_index(0)
            .map_err(|error| super::database_error("reading source fingerprint table", error))?;
        if excluded_tables.contains(&table) {
            continue;
        }
        let mut columns = table_columns(database, &table).await?;
        if let Some(excluded) = excluded_columns.get(&table) {
            columns.retain(|column| !excluded.contains(column));
        }
        if columns.is_empty() {
            return Err(Error::InvalidInput {
                field: "source fingerprint table".to_owned(),
                message: format!("source table {table:?} has no columns"),
            });
        }
        let select = columns
            .iter()
            .map(|column| {
                let quoted = quote_identifier(backend, column)?;
                Ok(match backend {
                    DbBackend::Sqlite => format!(
                        "CASE WHEN {quoted} IS NULL THEN NULL ELSE typeof({quoted}) || ':' || hex(CAST({quoted} AS BLOB)) END"
                    ),
                    DbBackend::Postgres => {
                        format!("encode(convert_to(CAST({quoted} AS TEXT), 'UTF8'), 'base64')")
                    }
                    DbBackend::MySql => format!("TO_BASE64(CAST({quoted} AS BINARY))"),
                })
            })
            .collect::<Result<Vec<_>>>()?
            .join(", ");
        let quoted_table = quote_identifier(backend, &table)?;
        let rows = database
            .query_all(Statement::from_string(
                backend,
                format!("SELECT {select} FROM {quoted_table}"),
            ))
            .await
            .map_err(|error| super::database_error("reading source fingerprint rows", error))?
            .into_iter()
            .map(|row| {
                columns
                    .iter()
                    .enumerate()
                    .map(|(index, column)| {
                        let value: Option<String> =
                            row.try_get_by_index(index).map_err(|error| {
                                super::database_error("reading source fingerprint value", error)
                            })?;
                        let encoded = match value {
                            Some(value) => format!("V{}:{value}", value.len()),
                            None => "N".to_owned(),
                        };
                        Ok((column.clone(), encoded))
                    })
                    .collect::<Result<BTreeMap<_, _>>>()
            })
            .collect::<Result<Vec<_>>>()?;
        let field_refs = columns.iter().map(String::as_str).collect::<Vec<_>>();
        fingerprints.push(SourceTableFingerprint {
            schema_digest: table_schema_digest(database, &table).await?,
            table,
            fingerprint: TableFingerprint::from_rows(&field_refs, rows)?,
        });
    }
    fingerprints.sort_by(|left, right| left.table.cmp(&right.table));
    Ok(fingerprints)
}

async fn table_schema_digest<C: ConnectionTrait + ?Sized>(
    database: &C,
    table: &str,
) -> Result<String> {
    let backend = database.get_database_backend();
    let mut hasher = Sha256::new();
    match backend {
        DbBackend::Sqlite => {
            hash_statement_rows(
                database,
                Statement::from_sql_and_values(
                    backend,
                    "SELECT type, name, COALESCE(sql, '') FROM sqlite_master WHERE (type = 'table' AND name = ?) OR (type = 'index' AND tbl_name = ?) ORDER BY type, name",
                    vec![table.to_owned().into(), table.to_owned().into()],
                ),
                3,
                &mut hasher,
            )
            .await?;
        }
        DbBackend::Postgres => {
            hash_statement_rows(
                database,
                Statement::from_sql_and_values(
                    backend,
                    "SELECT column_name, data_type, is_nullable, COALESCE(column_default, '') FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
                    vec![table.to_owned().into()],
                ),
                4,
                &mut hasher,
            )
            .await?;
            hash_statement_rows(
                database,
                Statement::from_sql_and_values(
                    backend,
                    "SELECT c.conname, pg_get_constraintdef(c.oid, true) FROM pg_constraint c JOIN pg_class r ON r.oid = c.conrelid JOIN pg_namespace n ON n.oid = r.relnamespace WHERE n.nspname = current_schema() AND r.relname = $1 ORDER BY c.conname",
                    vec![table.to_owned().into()],
                ),
                2,
                &mut hasher,
            )
            .await?;
            hash_statement_rows(
                database,
                Statement::from_sql_and_values(
                    backend,
                    "SELECT indexname, indexdef FROM pg_indexes WHERE schemaname = current_schema() AND tablename = $1 ORDER BY indexname",
                    vec![table.to_owned().into()],
                ),
                2,
                &mut hasher,
            )
            .await?;
        }
        DbBackend::MySql => {
            let quoted = quote_identifier(backend, table)?;
            let row = database
                .query_one(Statement::from_string(
                    backend,
                    format!("SHOW CREATE TABLE {quoted}"),
                ))
                .await
                .map_err(|error| super::database_error("reading MySQL table schema", error))?
                .ok_or_else(|| Error::NotFound {
                    resource: "MySQL table schema".to_owned(),
                    identifier: table.to_owned(),
                })?;
            let create_table: String = row
                .try_get_by_index(1)
                .map_err(|error| super::database_error("decoding MySQL table schema", error))?;
            let body_start = create_table.find('(').ok_or_else(|| Error::InvalidInput {
                field: "MySQL table schema".to_owned(),
                message: format!("SHOW CREATE TABLE returned an unexpected definition for {table}"),
            })?;
            let schema_body = normalize_mysql_schema_body(&create_table[body_start..]);
            append_hash(&mut hasher, schema_body.as_bytes());
        }
    }
    Ok(format!("{:x}", hasher.finalize()))
}

async fn hash_statement_rows<C: ConnectionTrait + ?Sized>(
    database: &C,
    statement: Statement,
    columns: usize,
    hasher: &mut Sha256,
) -> Result<()> {
    let rows = database
        .query_all(statement)
        .await
        .map_err(|error| super::database_error("reading source schema metadata", error))?;
    for row in rows {
        for index in 0..columns {
            let value: Option<String> = row
                .try_get_by_index(index)
                .map_err(|error| super::database_error("decoding source schema metadata", error))?;
            match value {
                Some(value) => {
                    append_hash(hasher, b"V");
                    append_hash(hasher, value.as_bytes());
                }
                None => append_hash(hasher, b"N"),
            }
        }
    }
    Ok(())
}

async fn table_columns<C: ConnectionTrait + ?Sized>(
    database: &C,
    table: &str,
) -> Result<Vec<String>> {
    let backend = database.get_database_backend();
    let (query, values) = match backend {
        DbBackend::Sqlite => (
            "SELECT name FROM pragma_table_info(?) ORDER BY cid",
            vec![table.to_owned().into()],
        ),
        DbBackend::Postgres => (
            "SELECT column_name FROM information_schema.columns WHERE table_schema = current_schema() AND table_name = $1 ORDER BY ordinal_position",
            vec![table.to_owned().into()],
        ),
        DbBackend::MySql => (
            "SELECT column_name FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = ? ORDER BY ordinal_position",
            vec![table.to_owned().into()],
        ),
    };
    database
        .query_all(Statement::from_sql_and_values(backend, query, values))
        .await
        .map_err(|error| super::database_error("listing source fingerprint columns", error))?
        .into_iter()
        .map(|row| {
            row.try_get_by_index(0)
                .map_err(|error| super::database_error("reading source fingerprint column", error))
        })
        .collect()
}

fn quote_identifier(backend: DbBackend, identifier: &str) -> Result<String> {
    if identifier.is_empty()
        || !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(Error::InvalidInput {
            field: "source fingerprint identifier".to_owned(),
            message: format!("unsupported catalog identifier {identifier:?}"),
        });
    }
    let quote = if backend == DbBackend::MySql {
        '`'
    } else {
        '"'
    };
    Ok(format!("{quote}{identifier}{quote}"))
}

fn normalize_mysql_schema_body(schema: &str) -> String {
    schema
        .split_whitespace()
        .filter(|token| !token.starts_with("AUTO_INCREMENT="))
        .collect::<Vec<_>>()
        .join(" ")
}

fn append(buffer: &mut Vec<u8>, value: &[u8]) {
    buffer.extend_from_slice(&(value.len() as u64).to_be_bytes());
    buffer.extend_from_slice(value);
}

fn append_hash(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

#[cfg(test)]
mod tests {
    use super::normalize_mysql_schema_body;

    #[test]
    fn mysql_schema_digest_ignores_next_auto_increment_value() {
        let first = "(id bigint NOT NULL AUTO_INCREMENT, PRIMARY KEY (id)) ENGINE=InnoDB AUTO_INCREMENT=2 DEFAULT CHARSET=utf8mb4";
        let second = "(id bigint NOT NULL AUTO_INCREMENT, PRIMARY KEY (id)) ENGINE=InnoDB AUTO_INCREMENT=9001 DEFAULT CHARSET=utf8mb4";
        assert_eq!(
            normalize_mysql_schema_body(first),
            normalize_mysql_schema_body(second)
        );
    }
}
