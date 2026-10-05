//! Table-backed user provider - Laravel's `DatabaseUserProvider`.
//!
//! Authenticates against a raw table via [`crate::DB::table`], returning
//! a [`GenericUser`]. The fully-generic provider: no model type, no
//! macro - point it at a table and it works, so the common case needs no
//! hand-written [`UserProvider`].
//!
//! ```rust,no_run
//! # use std::sync::Arc;
//! # use suprnova::{Auth, DatabaseUserProvider};
//! # fn ex() -> Result<(), Box<dyn std::error::Error>> {
//! // In bootstrap.rs:
//! Auth::register_provider("users", Arc::new(DatabaseUserProvider::new("users")))?;
//! # Ok(()) }
//! ```

use std::sync::Arc;

use async_trait::async_trait;
use sea_orm::Value as SeaValue;
use serde_json::Value;

use super::authenticatable::Authenticatable;
use super::generic_user::GenericUser;
use super::provider::UserProvider;
use crate::database::{DB, DynamicRow};
use crate::error::FrameworkError;
use crate::hashing;

/// The default id binder: numeric ids bind as integers (matching integer
/// primary keys), everything else binds as a string (UUIDs, opaque ids).
///
/// Trade-off: a zero-padded string id like `"007"` parses to the integer
/// `7` and would mis-bind against a *text* primary key. Apps with such
/// keys override the binder with [`DatabaseUserProvider::with_id_parser`].
///
/// An id above `i64::MAX` binds as the `u64` it is: Laravel's
/// `BIGINT UNSIGNED` `users.id` reaches it on MySQL, and as text MySQL did
/// not match it while Postgres refused the comparison. On Postgres and
/// SQLite no row holds such an id, so the lookup finds no user.
fn default_id_parser(id: &str) -> SeaValue {
    if let Ok(n) = id.parse::<i64>() {
        return SeaValue::from(n);
    }
    match id.parse::<u64>() {
        Ok(n) => SeaValue::from(n),
        Err(_) => SeaValue::from(id.to_string()),
    }
}

/// Convert a JSON credential value into a SQL bind. A whole number above
/// `i64::MAX` binds as the `u64` it is rather than as a rounded `f64`.
fn json_to_sea_value(v: &Value) -> SeaValue {
    match v {
        Value::String(s) => SeaValue::from(s.clone()),
        Value::Bool(b) => SeaValue::from(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                SeaValue::from(i)
            } else if let Some(u) = n.as_u64() {
                SeaValue::from(u)
            } else if let Some(f) = n.as_f64() {
                SeaValue::from(f)
            } else {
                SeaValue::from(n.to_string())
            }
        }
        other => SeaValue::from(other.to_string()),
    }
}

/// Stringify a JSON value for the id field (numbers become their decimal
/// form; strings pass through).
fn value_to_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A [`UserProvider`] that resolves users from a raw database table.
///
/// Configure the table and (optionally) the id / password / credential
/// columns. `retrieve_by_credentials` filters **only** on the configured
/// [`credential_columns`](Self::credential_columns) allowlist, so extra
/// keys in an attacker-influenced credential map can never inject extra
/// `WHERE` predicates.
pub struct DatabaseUserProvider {
    table: String,
    identifier_column: String,
    password_column: String,
    credential_columns: Vec<String>,
    id_parser: fn(&str) -> SeaValue,
}

impl DatabaseUserProvider {
    /// A provider for `table`, with `id` / `password` columns and an
    /// `email` credential lookup by default.
    pub fn new(table: impl Into<String>) -> Self {
        Self {
            table: table.into(),
            identifier_column: "id".to_string(),
            password_column: "password".to_string(),
            credential_columns: vec!["email".to_string()],
            id_parser: default_id_parser,
        }
    }

    /// Set the primary-key / id column (default `"id"`).
    pub fn identifier_column(mut self, column: impl Into<String>) -> Self {
        self.identifier_column = column.into();
        self
    }

    /// Set the password-hash column (default `"password"`).
    pub fn password_column(mut self, column: impl Into<String>) -> Self {
        self.password_column = column.into();
        self
    }

    /// Set the credential-lookup allowlist (default `["email"]`).
    ///
    /// `retrieve_by_credentials` filters on the intersection of these
    /// columns and the supplied credential keys - and nothing else. To
    /// allow login by email *or* username, pass `["email", "username"]`.
    pub fn credential_columns<I, S>(mut self, columns: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.credential_columns = columns.into_iter().map(Into::into).collect();
        self
    }

    /// Override how a string id is bound into the SQL lookup. The
    /// default parses numeric ids as integers and treats anything else
    /// (UUIDs, ULIDs, zero-padded codes) as a string - pass a custom
    /// parser when a string PK happens to look numeric (e.g. zero-padded
    /// `"007"`) so it doesn't bind as `7`.
    pub fn with_id_parser(mut self, parser: fn(&str) -> SeaValue) -> Self {
        self.id_parser = parser;
        self
    }

    /// How an id above `i64::MAX` binds against the identifier column.
    ///
    /// The default parser binds it as a `u64`, the number an integer key
    /// holds, and `DB::table` compares it as that number. A text identifier
    /// column holds such an id as its digits, though, and the number does
    /// not find it: Postgres refuses to compare text with a number, and
    /// MySQL compares the two as doubles, which cannot tell neighbouring
    /// 20-digit ids apart and so could answer with another user. On those
    /// two databases the column's type is read, for such an id only, and a
    /// text column gets the digits. SQLite compares the digits itself, by
    /// the column's affinity.
    ///
    /// The type comes from the table the lookup reads, and no other. On
    /// Postgres `to_regclass` resolves the lookup's own quoted name, so an
    /// unqualified table is the first one the search path holds, as for the
    /// query itself; a table of the same name in a later schema, whose
    /// column may have another type, never decides the bind. MySQL has no
    /// search path: an unqualified table is the current database's.
    async fn bind_large_id(&self, n: u64) -> Result<SeaValue, FrameworkError> {
        let backend = DB::connection()?.inner().get_database_backend();
        let (sql, values) = match backend {
            sea_orm::DbBackend::Postgres => (
                "SELECT format_type(a.atttypid, a.atttypmod) AS data_type \
                 FROM pg_catalog.pg_attribute a \
                 WHERE a.attrelid = to_regclass($1) AND a.attname = $2 \
                 AND a.attnum > 0 AND NOT a.attisdropped"
                    .to_owned(),
                vec![
                    SeaValue::from(crate::database::clauses::quote_identifier(
                        backend,
                        &self.table,
                    )),
                    SeaValue::from(self.identifier_column.clone()),
                ],
            ),
            sea_orm::DbBackend::MySql => {
                let (schema_test, schema) = match self.table.split_once('.') {
                    Some((schema, table)) => ("table_schema = ?", Some((schema, table))),
                    None => ("table_schema = DATABASE()", None),
                };
                let table = schema.map_or(self.table.as_str(), |(_, table)| table);
                let mut values = vec![
                    SeaValue::from(table.to_owned()),
                    SeaValue::from(self.identifier_column.clone()),
                ];
                if let Some((schema, _)) = schema {
                    values.push(SeaValue::from(schema.to_owned()));
                }
                (
                    format!(
                        "SELECT CAST(data_type AS CHAR) AS data_type \
                         FROM information_schema.columns \
                         WHERE table_name = ? AND column_name = ? AND {schema_test}"
                    ),
                    values,
                )
            }
            _ => return Ok(SeaValue::BigUnsigned(Some(n))),
        };
        let rows = DB::select(&sql, values).await?;
        let text = rows
            .first()
            .and_then(|row| row.get_string("data_type").ok())
            .is_some_and(|data_type| {
                let data_type = data_type.to_lowercase();
                data_type.contains("char") || data_type.contains("text")
            });
        Ok(if text {
            SeaValue::String(Some(n.to_string()))
        } else {
            SeaValue::BigUnsigned(Some(n))
        })
    }

    /// Build a [`GenericUser`] from a row.
    fn row_to_user(&self, row: DynamicRow) -> Arc<dyn Authenticatable> {
        let map = row.into_map();
        let id = map
            .get(&self.identifier_column)
            .map(value_to_string)
            .unwrap_or_default();
        let password = map
            .get(&self.password_column)
            .and_then(|v| v.as_str().map(String::from));
        Arc::new(GenericUser::new(id, password, map))
    }
}

#[async_trait]
impl UserProvider for DatabaseUserProvider {
    async fn retrieve_by_id(
        &self,
        id: &str,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        let value = match (self.id_parser)(id) {
            SeaValue::BigUnsigned(Some(n)) if n > i64::MAX as u64 => self.bind_large_id(n).await?,
            value => value,
        };
        let row = DB::table(&self.table)
            .filter(self.identifier_column.clone(), value)
            .first()
            .await?;
        Ok(row.map(|r| self.row_to_user(r)))
    }

    async fn retrieve_by_credentials(
        &self,
        credentials: &Value,
    ) -> Result<Option<Arc<dyn Authenticatable>>, FrameworkError> {
        let object = match credentials.as_object() {
            Some(o) => o,
            None => return Ok(None),
        };

        let mut query = DB::table(&self.table);
        let mut matched_any = false;
        for column in &self.credential_columns {
            // The password is verified separately and never used as a lookup
            // filter, even if it appears in the allowlist by mistake.
            if column == &self.password_column {
                continue;
            }
            if let Some(value) = object.get(column) {
                query = query.filter(column.clone(), json_to_sea_value(value));
                matched_any = true;
            }
        }

        // No allowlisted credential present → nothing to look up. This also
        // stops a bare `{}` (or a body carrying only non-allowlisted keys)
        // from matching the first row in the table.
        if !matched_any {
            return Ok(None);
        }

        let row = query.first().await?;
        Ok(row.map(|r| self.row_to_user(r)))
    }

    async fn validate_credentials(
        &self,
        user: &dyn Authenticatable,
        credentials: &Value,
    ) -> Result<bool, FrameworkError> {
        let password = credentials.get("password").and_then(|v| v.as_str());
        match (password, user.get_auth_password()) {
            (Some(plaintext), Some(hash)) => hashing::verify_async(plaintext, hash).await,
            // A user row with no stored password (OAuth-only / passwordless). Run a
            // dummy verify so this path costs the same as a wrong-password attempt,
            // closing the account-type timing oracle. Mirrors EloquentUserProvider.
            (Some(_), None) => {
                self.dummy_verify().await?;
                Ok(false)
            }
            (None, _) => Ok(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Instant;

    struct TestUser {
        password: Option<String>,
    }
    impl Authenticatable for TestUser {
        fn get_auth_identifier(&self) -> String {
            "1".into()
        }
        fn get_auth_password(&self) -> Option<&str> {
            self.password.as_deref()
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
        fn into_arc_any(self: Arc<Self>) -> Arc<dyn std::any::Any + Send + Sync> {
            self
        }
    }

    #[tokio::test]
    async fn passwordless_account_does_constant_work_no_timing_oracle() {
        let provider = DatabaseUserProvider::new("users");
        let creds = serde_json::json!({ "password": "guess" });
        let hash = crate::hashing::hash_async("correct horse").await.unwrap();

        // Warm up the hasher so one-time init doesn't skew the first measurement.
        let warm = TestUser {
            password: Some(hash.clone()),
        };
        let _ = provider.validate_credentials(&warm, &creds).await.unwrap();

        // Wrong-password path: a full KDF verify against a real stored hash.
        let with_hash = TestUser {
            password: Some(hash),
        };
        let t0 = Instant::now();
        assert!(
            !provider
                .validate_credentials(&with_hash, &creds)
                .await
                .unwrap()
        );
        let wrong_pw = t0.elapsed();

        // Passwordless (OAuth-only) user: must ALSO run a dummy KDF, not return
        // instantly - otherwise its wall-clock reveals the account is passwordless.
        let passwordless = TestUser { password: None };
        let t1 = Instant::now();
        assert!(
            !provider
                .validate_credentials(&passwordless, &creds)
                .await
                .unwrap()
        );
        let no_pw = t1.elapsed();

        // Same order of magnitude. Pre-fix the passwordless path is ~microseconds
        // vs ~tens of ms for a real verify (>1000x). 4x tolerance absorbs jitter.
        assert!(
            no_pw * 4 >= wrong_pw,
            "passwordless path ({no_pw:?}) far faster than wrong-password ({wrong_pw:?}): timing oracle present"
        );
    }
}
