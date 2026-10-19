//! Migrations for the tables users sign in from.
//!
//! [`UniqueEmailIgnoringCase`] makes the database refuse a second account
//! whose address differs from an existing one only in case. Sign-in,
//! password reset and agent auth all find a user by `LOWER(email)`, so two
//! such accounts would be one address that reaches either account.

use sea_orm::{ConnectionTrait, DbBackend, Statement};
use sea_orm_migration::prelude::*;
use sha2::{Digest, Sha256};

/// A unique index on the lower-cased address of a users table, so
/// `Alice@example.com` beside `alice@example.com` is refused.
///
/// ```rust,ignore
/// fn migrations() -> Vec<Box<dyn MigrationTrait>> {
///     vec![
///         Box::new(m20240101_000001_create_users_table::Migration),
///         Box::new(suprnova::auth::migrations::UniqueEmailIgnoringCase::on("users", "email")),
///     ]
/// }
/// ```
///
/// - **PostgreSQL and SQLite:** a unique index on `LOWER(column)`.
/// - **MySQL 8.0.13 and later:** the same, as a functional key part.
/// - **MariaDB, and MySQL before 8.0.13:** no index. They have no functional
///   indexes, and a column under their default `_ci` collations already
///   compares without case, so a unique index on the column itself refuses
///   the duplicate. The migration checks that the column has both, a
///   unique index of its own and a `_ci` collation, and fails naming what
///   is missing; with both, it logs that it relies on them.
///
/// When the table already holds addresses that differ only in case, the
/// migration fails and says how many there are, so they can be merged
/// first. It names no address: a migration log is not a place for
/// personal data.
#[derive(Debug, Clone)]
pub struct UniqueEmailIgnoringCase {
    table: String,
    column: String,
    name: String,
}

impl UniqueEmailIgnoringCase {
    /// The index on `column` of `table`. Both are plain SQL identifiers
    /// (letters, digits and `_`); any other name fails the migration rather
    /// than reach the SQL. The migration's name includes both, so one
    /// migrator can index more than one table:
    /// `m20261019_000003_{table}_{column}_unique_ignoring_case_{n}`, where
    /// `n` is the table name's length, so no two tables and columns share a
    /// name, as the index's name does.
    pub fn on(table: impl Into<String>, column: impl Into<String>) -> Self {
        let table = table.into();
        let column = column.into();
        let name = format!(
            "m20261019_000003_{table}_{column}_unique_ignoring_case_{}",
            table.len()
        );
        Self {
            table,
            column,
            name,
        }
    }

    /// The name of the index the migration creates, the same on every
    /// engine, and the name every check for it looks up.
    ///
    /// It is `{table}_{column}_lower_unique_{n}`, where `n` is the table
    /// name's length: the length tells where the table name ends, so `a_b`
    /// and `c` never share a name with `a` and `b_c`, whatever underscores
    /// either holds. A name longer than 63 bytes keeps its first bytes and
    /// ends with `_` and 16 hex characters of the SHA-256 of the table, a
    /// zero byte and the column, so it is exactly 63 bytes and still names
    /// one table and column: PostgreSQL would cut a longer name to 63 bytes
    /// without saying so, and MySQL refuses one over 64.
    pub fn index_name(&self) -> String {
        let full = format!(
            "{}_{}_lower_unique_{}",
            self.table,
            self.column,
            self.table.len()
        );
        if full.len() <= MAX_INDEX_NAME_BYTES {
            return full;
        }
        let mut digest = Sha256::new();
        digest.update(self.table.as_bytes());
        digest.update([0_u8]);
        digest.update(self.column.as_bytes());
        let suffix = format!("_{}", hex::encode(&digest.finalize()[..8]));
        let mut keep = MAX_INDEX_NAME_BYTES - suffix.len();
        // Identifiers are ASCII once checked; a name that is not still
        // never splits a character.
        while !full.is_char_boundary(keep) {
            keep -= 1;
        }
        format!("{}{suffix}", &full[..keep])
    }

    /// The table and column, checked to be identifiers the SQL can carry.
    fn identifiers(&self) -> Result<(&str, &str), DbErr> {
        for (what, value) in [("table", &self.table), ("column", &self.column)] {
            if !is_identifier(value) {
                return Err(DbErr::Migration(format!(
                    "UniqueEmailIgnoringCase: the {what} name must be a plain SQL identifier \
                     (letters, digits and _) of at most {MAX_IDENTIFIER_BYTES} bytes, not \
                     {value:?}"
                )));
            }
        }
        Ok((&self.table, &self.column))
    }
}

impl MigrationName for UniqueEmailIgnoringCase {
    fn name(&self) -> &str {
        &self.name
    }
}

#[async_trait::async_trait]
impl MigrationTrait for UniqueEmailIgnoringCase {
    async fn up(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let (table, column) = self.identifiers()?;
        let db = manager.get_connection();
        let backend = db.get_database_backend();
        let index = self.index_name();
        let sql = match backend {
            DbBackend::Postgres | DbBackend::Sqlite => {
                refuse_case_duplicates(db, table, column).await?;
                format!(
                    "CREATE UNIQUE INDEX IF NOT EXISTS \"{index}\" ON \"{table}\" (LOWER(\"{column}\"))"
                )
            }
            DbBackend::MySql => {
                if !has_functional_indexes(db).await? {
                    // No functional index here: the column's own unique
                    // index refuses a case-only duplicate only under a
                    // case-insensitive collation. Check both, so the
                    // migration never reports success while such a
                    // duplicate can still be stored.
                    let collation = mysql_column_collation(db, table, column).await?;
                    let unique = mysql_has_unique_column_index(db, table, column).await?;
                    if let Some(refusal) =
                        case_insensitive_refusal(table, column, collation.as_deref(), unique)
                    {
                        return Err(DbErr::Migration(refusal));
                    }
                    tracing::info!(
                        table,
                        column,
                        "this MySQL-family server has no functional indexes; the unique index on \
                         the address column, under its case-insensitive collation, refuses \
                         addresses that differ only in case"
                    );
                    return Ok(());
                }
                // MySQL has no `IF NOT EXISTS` for an index.
                if mysql_has_index(db, table, &index).await? {
                    return Ok(());
                }
                refuse_case_duplicates(db, table, column).await?;
                format!("CREATE UNIQUE INDEX `{index}` ON `{table}` ((LOWER(`{column}`)))")
            }
            _ => {
                return Err(DbErr::Migration(format!(
                    "UniqueEmailIgnoringCase: unsupported database backend {backend:?}"
                )));
            }
        };
        db.execute_unprepared(&sql).await.map(|_| ())
    }

    async fn down(&self, manager: &SchemaManager) -> Result<(), DbErr> {
        let (table, _) = self.identifiers()?;
        let db = manager.get_connection();
        let index = self.index_name();
        let sql = match db.get_database_backend() {
            DbBackend::Postgres | DbBackend::Sqlite => format!("DROP INDEX IF EXISTS \"{index}\""),
            DbBackend::MySql => {
                if !mysql_has_index(db, table, &index).await? {
                    return Ok(());
                }
                format!("DROP INDEX `{index}` ON `{table}`")
            }
            backend => {
                return Err(DbErr::Migration(format!(
                    "UniqueEmailIgnoringCase: unsupported database backend {backend:?}"
                )));
            }
        };
        db.execute_unprepared(&sql).await.map(|_| ())
    }
}

/// Why the column `table`.`column` cannot be left to its own unique index
/// on a server with no functional index: it needs a unique index on the
/// column alone and a case-insensitive (`_ci`) collation, or a case-only
/// duplicate can be stored. `None` when it has both.
fn case_insensitive_refusal(
    table: &str,
    column: &str,
    collation: Option<&str>,
    unique: bool,
) -> Option<String> {
    let case_insensitive =
        collation.is_some_and(|collation| collation.to_ascii_lowercase().ends_with("_ci"));
    let mut missing = Vec::new();
    if !unique {
        missing.push("a unique index on the column alone".to_owned());
    }
    if !case_insensitive {
        missing.push(format!(
            "a case-insensitive (_ci) collation, where it has {}",
            collation.unwrap_or("none")
        ));
    }
    if missing.is_empty() {
        return None;
    }
    Some(format!(
        "UniqueEmailIgnoringCase: this server has no functional indexes, so {table}.{column} \
         must refuse addresses that differ only in case by itself, and it lacks {}. Add them \
         to the column, then run the migration again.",
        missing.join(" and ")
    ))
}

/// The collation of `table`.`column` in the current MySQL-family database,
/// `None` for a column that has none.
async fn mysql_column_collation<C: ConnectionTrait + ?Sized>(
    db: &C,
    table: &str,
    column: &str,
) -> Result<Option<String>, DbErr> {
    let backend = db.get_database_backend();
    let row = db
        .query_one_raw(Statement::from_sql_and_values(
            backend,
            "SELECT COLLATION_NAME FROM information_schema.COLUMNS WHERE TABLE_SCHEMA = \
             DATABASE() AND TABLE_NAME = ? AND COLUMN_NAME = ?",
            [table.into(), column.into()],
        ))
        .await?
        .ok_or_else(|| {
            DbErr::Migration(format!(
                "UniqueEmailIgnoringCase: {table}.{column} does not exist"
            ))
        })?;
    row.try_get_by_index::<Option<String>>(0)
}

/// Whether `table` has a unique index whose only column is `column`.
async fn mysql_has_unique_column_index<C: ConnectionTrait + ?Sized>(
    db: &C,
    table: &str,
    column: &str,
) -> Result<bool, DbErr> {
    let backend = db.get_database_backend();
    Ok(db
        .query_one_raw(Statement::from_sql_and_values(
            backend,
            "SELECT INDEX_NAME FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = \
             DATABASE() AND TABLE_NAME = ? AND NON_UNIQUE = 0 GROUP BY INDEX_NAME \
             HAVING COUNT(*) = 1 AND MAX(COLUMN_NAME) = ? LIMIT 1",
            [table.into(), column.into()],
        ))
        .await?
        .is_some())
}

/// The longest index name every engine keeps whole: PostgreSQL's 63
/// bytes, one under MySQL's 64.
const MAX_INDEX_NAME_BYTES: usize = 63;

/// The longest table or column name the migration takes: MySQL's limit.
/// It also keeps the migration's name, which the migrator stores in a
/// `varchar(255)`, within 170 bytes.
const MAX_IDENTIFIER_BYTES: usize = 64;

/// Whether `value` is a plain SQL identifier no engine refuses for its
/// length, which needs no escaping inside quotes on any backend.
fn is_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    value.len() <= MAX_IDENTIFIER_BYTES
        && chars
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Fail when two addresses of `table` differ only in case, saying how many
/// such addresses there are and naming none.
async fn refuse_case_duplicates<C: ConnectionTrait + ?Sized>(
    db: &C,
    table: &str,
    column: &str,
) -> Result<(), DbErr> {
    let backend = db.get_database_backend();
    let quote = |name: &str| match backend {
        DbBackend::MySql => format!("`{name}`"),
        _ => format!("\"{name}\""),
    };
    let (table, column) = (quote(table), quote(column));
    let sql = format!(
        "SELECT COUNT(*) FROM (SELECT LOWER({column}) AS address FROM {table} \
         WHERE {column} IS NOT NULL GROUP BY LOWER({column}) HAVING COUNT(*) > 1) duplicates"
    );
    let row = db
        .query_one_raw(Statement::from_string(backend, sql))
        .await?
        .ok_or_else(|| DbErr::Migration("the duplicate address count returned no row".into()))?;
    let duplicated: i64 = row.try_get_by_index(0)?;
    if duplicated == 0 {
        return Ok(());
    }
    Err(DbErr::Migration(format!(
        "{duplicated} address(es) in {table}.{column} belong to more than one account, differing \
         only in case. Merge those accounts, keeping one per address, then run the migration \
         again."
    )))
}

/// Whether the server takes functional key parts: MySQL 8.0.13 and later,
/// never MariaDB.
async fn has_functional_indexes<C: ConnectionTrait + ?Sized>(db: &C) -> Result<bool, DbErr> {
    let backend = db.get_database_backend();
    let row = db
        .query_one_raw(Statement::from_string(backend, "SELECT VERSION()"))
        .await?
        .ok_or_else(|| DbErr::Migration("SELECT VERSION() returned no row".into()))?;
    let version: String = row.try_get_by_index(0)?;
    Ok(functional_indexes_in(&version))
}

/// Whether a MySQL-family `VERSION()` string names a server with
/// functional key parts.
fn functional_indexes_in(version: &str) -> bool {
    if version.to_ascii_lowercase().contains("mariadb") {
        return false;
    }
    let mut parts = version
        .split(|c: char| !c.is_ascii_digit())
        .filter(|part| !part.is_empty())
        .map(|part| part.parse::<u32>().unwrap_or(0));
    let (major, minor, patch) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    (major, minor, patch) >= (8, 0, 13)
}

/// Whether `table` of the current MySQL database has the index `index`.
async fn mysql_has_index<C: ConnectionTrait + ?Sized>(
    db: &C,
    table: &str,
    index: &str,
) -> Result<bool, DbErr> {
    let backend = db.get_database_backend();
    Ok(db
        .query_one_raw(Statement::from_sql_and_values(
            backend,
            "SELECT 1 FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = DATABASE() \
             AND TABLE_NAME = ? AND INDEX_NAME = ? LIMIT 1",
            [table.into(), index.into()],
        ))
        .await?
        .is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_008_only_plain_identifiers_reach_the_sql() {
        for good in ["users", "app_users", "_t", "Users2"] {
            assert!(is_identifier(good), "{good}");
        }
        for bad in [
            "",
            "2users",
            "users;drop",
            "us\"ers",
            "us`ers",
            "public.users",
            "u s",
        ] {
            assert!(!is_identifier(bad), "{bad}");
            let migration = UniqueEmailIgnoringCase::on(bad, "email");
            assert!(migration.identifiers().is_err(), "{bad}");
            let migration = UniqueEmailIgnoringCase::on("users", bad);
            assert!(migration.identifiers().is_err(), "{bad}");
        }
    }

    #[test]
    fn agent_008_migration_and_index_names_carry_the_table() {
        let migration = UniqueEmailIgnoringCase::on("users", "email");
        assert_eq!(
            migration.name(),
            "m20261019_000003_users_email_unique_ignoring_case_5"
        );
        assert_eq!(migration.index_name(), "users_email_lower_unique_5");
    }

    #[test]
    fn agent_008_no_two_tables_and_columns_share_an_index_name() {
        let pairs = [
            ("a_b", "c"),
            ("a", "b_c"),
            ("a_", "_b"),
            ("a", "__b"),
            ("a__", "b"),
            ("users", "email"),
            ("users_email", "x"),
            ("users", "email_x"),
        ];
        let mut names: Vec<String> = pairs
            .iter()
            .map(|(table, column)| UniqueEmailIgnoringCase::on(*table, *column).index_name())
            .collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), pairs.len(), "{names:?}");
    }

    #[test]
    fn agent_008_an_index_name_fits_every_engine() {
        let longest = "t".repeat(64);
        let column = "c".repeat(64);
        let name = UniqueEmailIgnoringCase::on(longest.as_str(), column.as_str()).index_name();
        assert!(name.len() <= 63, "{} bytes: {name}", name.len());
        // The engine run's table: a unique name per run.
        let table = "email_case_users_4e766f35571241e1a7be9eca7b7a9828";
        let name = UniqueEmailIgnoringCase::on(table, "email").index_name();
        assert_eq!(name.len(), 63, "{name}");
        assert!(name.starts_with("email_case_users_"), "{name}");
        // A name that fits is the readable one.
        assert_eq!(
            UniqueEmailIgnoringCase::on("users", "email").index_name(),
            "users_email_lower_unique_5"
        );
        assert_eq!(
            UniqueEmailIgnoringCase::on("app_users", "email_address").index_name(),
            "app_users_email_address_lower_unique_9"
        );
    }

    #[test]
    fn agent_008_long_names_sharing_a_prefix_stay_apart() {
        let table = "t".repeat(60);
        let first = UniqueEmailIgnoringCase::on(table.as_str(), "email_a").index_name();
        let second = UniqueEmailIgnoringCase::on(table.as_str(), "email_b").index_name();
        assert!(first.len() <= 63 && second.len() <= 63, "{first} {second}");
        assert_ne!(first, second);
        // The same pair always gets the same name, release after release: a
        // migration run by a later release finds the index an earlier one
        // made by this name. The value is pinned, so a change to the hash or
        // the cut fails here instead of orphaning existing indexes.
        assert_eq!(first, format!("{}_e09bd5316ac7c836", "t".repeat(46)));
        assert_eq!(
            UniqueEmailIgnoringCase::on(
                "email_case_users_4e766f35571241e1a7be9eca7b7a9828",
                "email"
            )
            .index_name(),
            "email_case_users_4e766f35571241e1a7be9eca7b7a9_39cf21e3ceff1ce5"
        );
    }

    #[test]
    fn agent_008_the_migration_name_fits_the_migrators_version_column() {
        // The longest table and column the migration takes; its name is
        // kept in `seaql_migrations.version`, a `varchar(255)` on MySQL.
        let longest = "t".repeat(64);
        let column = "c".repeat(64);
        let migration = UniqueEmailIgnoringCase::on(longest.as_str(), column.as_str());
        assert!(migration.identifiers().is_ok());
        assert!(migration.name().len() <= 255, "{}", migration.name().len());
        // A longer name is no identifier any engine takes.
        let too_long = "t".repeat(65);
        assert!(
            UniqueEmailIgnoringCase::on(too_long.as_str(), "email")
                .identifiers()
                .is_err()
        );
        assert!(
            UniqueEmailIgnoringCase::on("users", too_long.as_str())
                .identifiers()
                .is_err()
        );
    }

    #[test]
    fn agent_008_no_two_tables_and_columns_share_a_migration_name() {
        let pairs = [
            ("a_b", "c"),
            ("a", "b_c"),
            ("a_", "_b"),
            ("a", "__b"),
            ("a__", "b"),
            ("users", "email"),
            ("users_email", "x"),
            ("users", "email_x"),
        ];
        let mut names: Vec<String> = pairs
            .iter()
            .map(|(table, column)| {
                UniqueEmailIgnoringCase::on(*table, *column)
                    .name()
                    .to_owned()
            })
            .collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), pairs.len(), "{names:?}");
    }

    #[test]
    fn agent_008_a_case_insensitive_server_needs_a_unique_ci_column() {
        assert_eq!(
            case_insensitive_refusal("users", "email", Some("utf8mb4_unicode_ci"), true),
            None
        );
        assert_eq!(
            case_insensitive_refusal("users", "email", Some("uca1400_ai_ci"), true),
            None
        );
        let binary = case_insensitive_refusal("users", "email", Some("utf8mb4_bin"), true)
            .expect("a binary collation compares case");
        assert!(binary.contains("users.email"), "{binary}");
        assert!(binary.contains("utf8mb4_bin"), "{binary}");
        assert!(binary.contains("_ci"), "{binary}");
        let plain = case_insensitive_refusal("users", "email", Some("utf8mb4_general_ci"), false)
            .expect("a column without a unique index takes duplicates");
        assert!(plain.contains("users.email"), "{plain}");
        assert!(plain.contains("unique index"), "{plain}");
        let neither = case_insensitive_refusal("users", "email", None, false)
            .expect("a column with no collation and no unique index");
        assert!(
            neither.contains("unique index") && neither.contains("_ci"),
            "{neither}"
        );
    }

    #[test]
    fn agent_008_functional_indexes_start_at_mysql_8_0_13() {
        assert!(functional_indexes_in("8.4.3"));
        assert!(functional_indexes_in("8.0.13"));
        assert!(functional_indexes_in("9.1.0-commercial"));
        assert!(!functional_indexes_in("8.0.12"));
        assert!(!functional_indexes_in("5.7.44-log"));
        assert!(!functional_indexes_in("11.4.2-MariaDB-ubu2404"));
        assert!(!functional_indexes_in("12.3.1-MariaDB"));
    }
}
