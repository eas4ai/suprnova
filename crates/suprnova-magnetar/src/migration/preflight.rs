//! Source-shape detection, collision preflight, and source cleanup execution.

use std::collections::BTreeMap;

use sea_orm::{ConnectionTrait, DbBackend, Statement};

use crate::password::normalize_email;
use crate::{Error, Result};

use super::plan::{SacrificeableCleanup, SourceRowCount};
use super::schema_guards::{has_column, has_columns, has_table};
use super::{ShapeConfirmation, SourceShape, database_error};

/// A source row that owns one normalized-email key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CollisionOwner {
    /// The source table containing the owner.
    pub table: String,
    /// The source primary-key value rendered without lossy conversion.
    pub primary_key: String,
    /// The unnormalized email stored by the source row.
    pub email: String,
}

/// Every source owner sharing one normalized-email key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct CollisionGroup {
    /// The normalized email that is ambiguous.
    pub normalized_email: String,
    /// Every table and primary-key owner in stable order.
    pub owners: Vec<CollisionOwner>,
}

/// A source user row used for planning an application identity decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceUser {
    pub(crate) id: String,
    pub(crate) email: String,
    pub(crate) preferred_app_user_id: Option<i64>,
}

/// A source passkey envelope paired with its Torii owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourcePasskey {
    pub(crate) external_user_id: String,
    pub(crate) credential_id: String,
    pub(crate) data_json: String,
}

/// Detects exactly one source shape while allowing declared destination tables
/// to coexist during an in-place migration.
pub(crate) async fn detect_source_shape_for_targets<C: ConnectionTrait + ?Sized>(
    database: &C,
    target_tables: &[String],
) -> Result<SourceShape> {
    let mut torii = has_table(database, None, "users").await?
        && has_table(database, None, "torii_migrations").await?
        && has_columns(database, None, "users", &["id", "email", "password_hash"]).await?;
    let mut suprnova_web = has_table(database, None, "users").await?
        && has_columns(
            database,
            None,
            "users",
            &["id", "name", "email", "password", "remember_token"],
        )
        .await?;
    let mut suprnova_api = has_table(database, None, "app_users").await?
        && has_columns(database, None, "app_users", &["id", "email"]).await?;
    let magnetar = has_magnetar_marker(database).await?;
    if magnetar {
        torii = false;
        suprnova_web = false;
        suprnova_api = false;
    } else if (torii || suprnova_web) && target_tables.iter().any(|table| table == "app_users") {
        suprnova_api = false;
    }

    let shapes = [
        (torii, SourceShape::Torii),
        (suprnova_web, SourceShape::SuprnovaWeb),
        (suprnova_api, SourceShape::SuprnovaApi),
        (magnetar, SourceShape::Magnetar),
    ]
    .into_iter()
    .filter_map(|(matched, shape)| matched.then_some(shape))
    .collect::<Vec<_>>();

    match shapes.as_slice() {
        [shape] => Ok(*shape),
        [] => Err(Error::InvalidInput {
            field: "source-shape".to_owned(),
            message: "database does not match a complete supported source shape".to_owned(),
        }),
        _ => Err(Error::Conflict {
            resource: "source-shape".to_owned(),
            message: format!(
                "hybrid or half-transformed database matches {}",
                shapes
                    .iter()
                    .map(|shape| shape.cli_value())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        }),
    }
}

async fn has_magnetar_marker<C: ConnectionTrait + ?Sized>(database: &C) -> Result<bool> {
    if !has_table(database, None, "magnetar_migration_state").await? {
        return Ok(false);
    }
    if !has_columns(
        database,
        None,
        "magnetar_migration_state",
        &["key", "value"],
    )
    .await?
    {
        return Err(Error::Conflict {
            resource: "Magnetar migration marker".to_owned(),
            message: "marker table is missing key or value columns".to_owned(),
        });
    }
    let backend = database.get_database_backend();
    let query = match backend {
        DbBackend::MySql => {
            "SELECT `value` FROM `magnetar_migration_state` WHERE `key` = ? LIMIT 1"
        }
        DbBackend::Postgres => {
            "SELECT \"value\" FROM \"magnetar_migration_state\" WHERE \"key\" = $1 LIMIT 1"
        }
        DbBackend::Sqlite => {
            "SELECT \"value\" FROM \"magnetar_migration_state\" WHERE \"key\" = ? LIMIT 1"
        }
    };
    let row = database
        .query_one(Statement::from_sql_and_values(
            backend,
            query,
            vec!["schema_version".into()],
        ))
        .await
        .map_err(|error| database_error("reading Magnetar migration marker", error))?;
    let value = row
        .map(|row| {
            row.try_get_by_index::<String>(0)
                .map_err(|error| database_error("decoding Magnetar marker version", error))
        })
        .transpose()?;
    match value.as_deref() {
        Some("1") => Ok(true),
        Some(other) => Err(Error::Conflict {
            resource: "Magnetar migration marker".to_owned(),
            message: format!("unsupported existing marker version {other}"),
        }),
        None => Ok(false),
    }
}

/// Validates that advisory detection, operator selection, and fresh detection agree.
pub(crate) fn validate_confirmation(
    confirmation: &ShapeConfirmation,
    fresh_detection: SourceShape,
) -> Result<()> {
    if confirmation.detected != confirmation.operator_selected {
        return Err(Error::Conflict {
            resource: "source-shape confirmation".to_owned(),
            message: format!(
                "detected {} but operator selected {}",
                confirmation.detected, confirmation.operator_selected
            ),
        });
    }
    if confirmation.detected != fresh_detection {
        return Err(Error::Conflict {
            resource: "source-shape confirmation".to_owned(),
            message: format!(
                "stored confirmation {} no longer matches fresh detection {}",
                confirmation.detected, fresh_detection
            ),
        });
    }
    Ok(())
}

/// Enumerates every normalized-email source owner for one detected shape.
pub(crate) async fn normalized_collisions<C: ConnectionTrait + ?Sized>(
    database: &C,
    shape: SourceShape,
) -> Result<Vec<CollisionGroup>> {
    let (table, users) = match shape {
        SourceShape::Magnetar => return Ok(Vec::new()),
        _ => (
            source_user_table(shape),
            source_users(database, shape).await?,
        ),
    };
    let mut groups: BTreeMap<String, Vec<CollisionOwner>> = BTreeMap::new();
    for user in users {
        groups
            .entry(normalize_email(&user.email))
            .or_default()
            .push(CollisionOwner {
                table: table.to_owned(),
                primary_key: user.id,
                email: user.email,
            });
    }
    let mut collisions = groups
        .into_iter()
        .filter_map(|(normalized_email, mut owners)| {
            (owners.len() > 1).then(|| {
                owners.sort();
                CollisionGroup {
                    normalized_email,
                    owners,
                }
            })
        })
        .collect::<Vec<_>>();
    collisions.sort();
    Ok(collisions)
}

/// Builds a stable conflict error from collision groups.
pub(crate) fn collision_error(collisions: &[CollisionGroup]) -> Error {
    let details = collisions
        .iter()
        .map(|group| {
            let owners = group
                .owners
                .iter()
                .map(|owner| format!("{}:{}", owner.table, owner.primary_key))
                .collect::<Vec<_>>()
                .join(",");
            format!("{}=[{}]", group.normalized_email, owners)
        })
        .collect::<Vec<_>>()
        .join("; ");
    Error::Conflict {
        resource: "normalized source email".to_owned(),
        message: format!("ambiguous owners: {details}"),
    }
}

/// Returns stable warnings for application targets already owned by the source.
pub(crate) async fn foreign_target_table_warnings<C: ConnectionTrait + ?Sized>(
    database: &C,
    target_tables: &[String],
) -> Result<Vec<String>> {
    let mut targets = target_tables.to_vec();
    targets.sort();
    targets.dedup();
    let mut warnings = Vec::new();
    for table in targets {
        if has_table(database, None, &table).await? {
            warnings.push(format!(
                "source database already owns target table name {table}; choose a host binding that does not collide"
            ));
        }
    }
    Ok(warnings)
}

/// Counts every regular source table in stable table-name order.
pub(crate) async fn source_row_counts<C: ConnectionTrait + ?Sized>(
    database: &C,
) -> Result<Vec<SourceRowCount>> {
    let backend = database.get_database_backend();
    let list_tables = match backend {
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
        .query_all(Statement::from_string(backend, list_tables))
        .await
        .map_err(|error| database_error("listing source tables for dry run", error))?;
    let mut counts = Vec::with_capacity(tables.len());
    for row in tables {
        let table: String = row
            .try_get_by_index(0)
            .map_err(|error| database_error("reading source table name", error))?;
        let quoted = quote_identifier(backend, &table)?;
        let count_row = database
            .query_one(Statement::from_string(
                backend,
                format!("SELECT COUNT(*) FROM {quoted}"),
            ))
            .await
            .map_err(|error| database_error("counting source table for dry run", error))?
            .ok_or_else(|| Error::Internal {
                message: format!("missing count result for source table {table}"),
            })?;
        let rows: i64 = count_row
            .try_get_by_index(0)
            .map_err(|error| database_error("reading source table count", error))?;
        counts.push(SourceRowCount {
            table,
            rows: u64::try_from(rows).map_err(|_| Error::Internal {
                message: "database returned a negative row count".to_owned(),
            })?,
        });
    }
    Ok(counts)
}

/// Reads all supported source user rows without changing them.
pub(crate) async fn source_users<C: ConnectionTrait + ?Sized>(
    database: &C,
    shape: SourceShape,
) -> Result<Vec<SourceUser>> {
    if shape == SourceShape::Magnetar {
        return Ok(Vec::new());
    }
    let table = source_user_table(shape);
    let backend = database.get_database_backend();
    let cast_id = match backend {
        DbBackend::MySql => "CAST(id AS CHAR)",
        DbBackend::Postgres | DbBackend::Sqlite => "CAST(id AS TEXT)",
    };
    let query = format!("SELECT {cast_id}, email FROM {table} ORDER BY {cast_id}");
    database
        .query_all(Statement::from_string(backend, query))
        .await
        .map_err(|error| database_error("reading source users", error))?
        .into_iter()
        .map(|row| {
            let id: String = row
                .try_get_by_index(0)
                .map_err(|error| database_error("reading source user id", error))?;
            let preferred_app_user_id = match shape {
                SourceShape::Torii | SourceShape::SuprnovaWeb => None,
                SourceShape::SuprnovaApi => {
                    Some(id.parse::<i64>().map_err(|_| Error::InvalidInput {
                        field: "source user id".to_owned(),
                        message: format!("source application user id {id:?} is not i64"),
                    })?)
                }
                SourceShape::Magnetar => unreachable!("handled above"),
            };
            Ok(SourceUser {
                id,
                email: row
                    .try_get_by_index(1)
                    .map_err(|error| database_error("reading source user email", error))?,
                preferred_app_user_id,
            })
        })
        .collect()
}

/// Reads Torii passkey envelopes without transforming their `data_json` bytes.
pub(crate) async fn source_passkeys<C: ConnectionTrait + ?Sized>(
    database: &C,
) -> Result<Vec<SourcePasskey>> {
    if !super::source_records::validate_optional_table(
        database,
        "passkeys",
        &["id", "user_id", "credential_id", "data_json"],
    )
    .await?
    {
        return Ok(Vec::new());
    }
    let backend = database.get_database_backend();
    let owner = match backend {
        DbBackend::MySql => "CAST(user_id AS CHAR)",
        DbBackend::Postgres | DbBackend::Sqlite => "CAST(user_id AS TEXT)",
    };
    database
        .query_all(Statement::from_string(
            backend,
            format!("SELECT {owner}, credential_id, data_json FROM passkeys ORDER BY id"),
        ))
        .await
        .map_err(|error| database_error("reading source passkeys", error))?
        .into_iter()
        .map(|row| {
            Ok(SourcePasskey {
                external_user_id: row
                    .try_get_by_index(0)
                    .map_err(|error| database_error("reading passkey user", error))?,
                credential_id: row
                    .try_get_by_index(1)
                    .map_err(|error| database_error("reading passkey credential", error))?,
                data_json: row
                    .try_get_by_index(2)
                    .map_err(|error| database_error("reading passkey envelope", error))?,
            })
        })
        .collect()
}

/// Applies only the explicit sacrificeable source cleanup list.
pub(crate) async fn apply_cleanup<C: ConnectionTrait + ?Sized>(
    database: &C,
    cleanup: &SacrificeableCleanup,
) -> Result<usize> {
    let mut statements = 0;
    for target in &cleanup.targets {
        if let Some((table, column)) = target.split_once('.') {
            if has_table(database, None, table).await?
                && has_column(database, None, table, column).await?
            {
                execute(database, &format!("UPDATE {table} SET {column} = NULL")).await?;
                statements += 1;
            }
        } else if has_table(database, None, target).await? {
            execute(database, &format!("DELETE FROM {target}")).await?;
            statements += 1;
        }
    }
    Ok(statements)
}

fn quote_identifier(backend: DbBackend, identifier: &str) -> Result<String> {
    if identifier.is_empty()
        || !identifier
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return Err(Error::InvalidInput {
            field: "source table name".to_owned(),
            message: format!("unsupported catalog table name {identifier:?}"),
        });
    }
    let quote = if backend == DbBackend::MySql {
        '`'
    } else {
        '"'
    };
    Ok(format!("{quote}{identifier}{quote}"))
}

fn source_user_table(shape: SourceShape) -> &'static str {
    match shape {
        SourceShape::Torii | SourceShape::SuprnovaWeb => "users",
        SourceShape::SuprnovaApi => "app_users",
        SourceShape::Magnetar => unreachable!("Magnetar has no legacy source user table"),
    }
}

async fn execute<C: ConnectionTrait + ?Sized>(database: &C, query: &str) -> Result<()> {
    let backend = database.get_database_backend();
    database
        .execute(Statement::from_string(backend, query))
        .await
        .map_err(|error| database_error("invalidating sacrificeable migration state", error))?;
    Ok(())
}
