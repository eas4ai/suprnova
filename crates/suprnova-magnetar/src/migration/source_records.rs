//! Backend-neutral reads for durable legacy authentication records.

use sea_orm::{ConnectionTrait, DbBackend, QueryResult, Statement};
use secrecy::SecretString;

use crate::{Error, Result};

use super::SourceShape;
use super::database_error;
use super::records::{ImportedFailedLoginAttempt, ImportedUser, PendingAuthRecord};
use super::schema_guards::{has_column, has_table};

pub(crate) async fn users<C: ConnectionTrait + ?Sized>(
    database: &C,
    shape: SourceShape,
) -> Result<Vec<ImportedUser>> {
    if shape == SourceShape::Magnetar {
        return Ok(Vec::new());
    }
    let backend = database.get_database_backend();
    let (
        table,
        id,
        email,
        name,
        password,
        verified,
        locked,
        created,
        updated,
        auth_epoch,
        session_version,
    ) = match shape {
        SourceShape::Torii => (
            "users",
            text(backend, "id"),
            "email".to_owned(),
            "name".to_owned(),
            "password_hash".to_owned(),
            text(backend, "email_verified_at"),
            text(backend, "locked_at"),
            text(backend, "created_at"),
            text(backend, "updated_at"),
            "NULL".to_owned(),
            "NULL".to_owned(),
        ),
        SourceShape::SuprnovaWeb => (
            "users",
            text(backend, "id"),
            "email".to_owned(),
            "name".to_owned(),
            "password".to_owned(),
            text(backend, "email_verified_at"),
            "NULL".to_owned(),
            text(backend, "created_at"),
            text(backend, "updated_at"),
            "NULL".to_owned(),
            "NULL".to_owned(),
        ),
        SourceShape::SuprnovaApi => (
            "app_users",
            text(backend, "id"),
            "email".to_owned(),
            optional_text(database, "app_users", "name").await?,
            optional_text(database, "app_users", "password_hash").await?,
            optional_text(database, "app_users", "email_verified_at").await?,
            optional_text(database, "app_users", "locked_at").await?,
            optional_text(database, "app_users", "created_at").await?,
            optional_text(database, "app_users", "updated_at").await?,
            optional_integer(database, "app_users", "auth_epoch").await?,
            optional_integer(database, "app_users", "session_version").await?,
        ),
        SourceShape::Magnetar => unreachable!("handled above"),
    };
    let query = format!(
        "SELECT {id}, {email}, {name}, {password}, {verified}, {locked}, {created}, {updated}, {auth_epoch}, {session_version} FROM {table} ORDER BY {id}"
    );
    database.query_all_raw(Statement::from_string(backend, query))
        .await
        .map_err(|error| database_error("reading durable source users", error))?
        .into_iter()
        .map(|row| user_from_row(row, shape))
        .collect()
}

pub(crate) async fn auth_records<C: ConnectionTrait + ?Sized>(
    database: &C,
    shape: SourceShape,
) -> Result<Vec<PendingAuthRecord>> {
    let mut records = Vec::new();
    match shape {
        SourceShape::Torii => {
            read_linked_accounts(database, &mut records).await?;
            read_secure_tokens(database, &mut records).await?;
            read_failed_attempts(database, &mut records).await?;
        }
        SourceShape::SuprnovaWeb => read_two_factor(database, &mut records).await?,
        SourceShape::SuprnovaApi | SourceShape::Magnetar => {}
    }
    Ok(records)
}

pub(crate) async fn validate_schema<C: ConnectionTrait + ?Sized>(
    database: &C,
    shape: SourceShape,
) -> Result<()> {
    match shape {
        SourceShape::Torii => {
            validate_required_table(
                database,
                "users",
                &[
                    "id",
                    "email",
                    "name",
                    "password_hash",
                    "email_verified_at",
                    "locked_at",
                    "created_at",
                    "updated_at",
                ],
            )
            .await?;
            validate_optional_table(
                database,
                "passkeys",
                &["id", "user_id", "credential_id", "data_json"],
            )
            .await?;
            validate_optional_table(
                database,
                "oauth_accounts",
                &[
                    "id",
                    "user_id",
                    "provider",
                    "subject",
                    "created_at",
                    "updated_at",
                ],
            )
            .await?;
            validate_optional_table(
                database,
                "secure_tokens",
                &[
                    "id",
                    "user_id",
                    "token",
                    "purpose",
                    "used_at",
                    "expires_at",
                    "created_at",
                    "updated_at",
                ],
            )
            .await?;
            validate_optional_table(
                database,
                "failed_login_attempts",
                &["id", "email", "ip_address", "attempted_at"],
            )
            .await?;
        }
        SourceShape::SuprnovaWeb => {
            validate_required_table(
                database,
                "users",
                &[
                    "id",
                    "name",
                    "email",
                    "password",
                    "remember_token",
                    "email_verified_at",
                    "created_at",
                    "updated_at",
                ],
            )
            .await?;
            validate_optional_table(
                database,
                "two_factor_credentials",
                &[
                    "user_id",
                    "secret",
                    "confirmed_at",
                    "recovery_codes",
                    "last_used_timestep",
                    "created_at",
                    "updated_at",
                ],
            )
            .await?;
        }
        SourceShape::SuprnovaApi | SourceShape::Magnetar => {}
    }
    Ok(())
}

pub(crate) async fn validate_cleanup_schema<C: ConnectionTrait + ?Sized>(
    database: &C,
    shape: SourceShape,
) -> Result<()> {
    match shape {
        SourceShape::Torii => {
            validate_optional_table(
                database,
                "sessions",
                &[
                    "id",
                    "user_id",
                    "token",
                    "expires_at",
                    "created_at",
                    "updated_at",
                ],
            )
            .await?;
            validate_optional_table(
                database,
                "pkce_verifiers",
                &[
                    "id",
                    "csrf_state",
                    "verifier",
                    "expires_at",
                    "created_at",
                    "updated_at",
                ],
            )
            .await?;
            validate_optional_table(
                database,
                "passkey_challenges",
                &[
                    "id",
                    "challenge_id",
                    "challenge",
                    "expires_at",
                    "created_at",
                    "updated_at",
                ],
            )
            .await?;
            validate_optional_table(database, "torii_migrations", &["version", "applied_at"])
                .await?;
        }
        SourceShape::SuprnovaWeb => {
            validate_optional_table(
                database,
                "sessions",
                &["id", "user_id", "payload", "csrf_token", "last_activity"],
            )
            .await?;
            validate_optional_table(
                database,
                "remember_tokens",
                &[
                    "id",
                    "user_id",
                    "selector",
                    "token_hash",
                    "expires_at",
                    "created_at",
                    "last_used_at",
                ],
            )
            .await?;
            validate_optional_table(
                database,
                "auth_flow_tokens",
                &[
                    "id",
                    "user_id",
                    "token_hash",
                    "purpose",
                    "expires_at",
                    "used_at",
                    "created_at",
                ],
            )
            .await?;
            validate_optional_table(
                database,
                "auth_ceremony_tokens",
                &[
                    "id",
                    "selector",
                    "kind",
                    "payload",
                    "expires_at",
                    "created_at",
                ],
            )
            .await?;
        }
        SourceShape::SuprnovaApi | SourceShape::Magnetar => {}
    }
    Ok(())
}

fn user_from_row(row: QueryResult, shape: SourceShape) -> Result<ImportedUser> {
    let source_user_id: String = value(&row, 0, "source user id")?;
    let preferred_app_user_id = match shape {
        SourceShape::Torii | SourceShape::SuprnovaWeb | SourceShape::Magnetar => None,
        SourceShape::SuprnovaApi => {
            Some(
                source_user_id
                    .parse::<i64>()
                    .map_err(|_| Error::InvalidInput {
                        field: "source user id".to_owned(),
                        message: format!(
                            "source application user id {source_user_id:?} is not i64"
                        ),
                    })?,
            )
        }
    };
    Ok(ImportedUser {
        source_user_id,
        preferred_app_user_id,
        email: value(&row, 1, "source user email")?,
        name: optional(&row, 2, "source user name")?,
        password_hash: optional(&row, 3, "source password hash")?,
        email_verified_at: optional(&row, 4, "source verification timestamp")?,
        locked_at: optional(&row, 5, "source lockout timestamp")?,
        created_at: optional(&row, 6, "source creation timestamp")?,
        updated_at: optional(&row, 7, "source update timestamp")?,
        auth_epoch: optional(&row, 8, "source auth epoch")?,
        session_version: optional(&row, 9, "source session version")?,
    })
}

async fn read_linked_accounts<C: ConnectionTrait + ?Sized>(
    database: &C,
    records: &mut Vec<PendingAuthRecord>,
) -> Result<()> {
    if !validate_optional_table(
        database,
        "oauth_accounts",
        &[
            "id",
            "user_id",
            "provider",
            "subject",
            "created_at",
            "updated_at",
        ],
    )
    .await?
    {
        return Ok(());
    }
    let backend = database.get_database_backend();
    let query = format!(
        "SELECT {}, provider, subject, {}, {} FROM oauth_accounts ORDER BY id",
        text(backend, "user_id"),
        optional_text(database, "oauth_accounts", "created_at").await?,
        optional_text(database, "oauth_accounts", "updated_at").await?,
    );
    for row in query_all(database, query, "reading source linked accounts").await? {
        records.push(PendingAuthRecord::LinkedAccount {
            source_user_id: value(&row, 0, "linked-account owner")?,
            provider: value(&row, 1, "linked-account provider")?,
            subject: value(&row, 2, "linked-account subject")?,
            created_at: optional(&row, 3, "linked-account creation timestamp")?,
            updated_at: optional(&row, 4, "linked-account update timestamp")?,
        });
    }
    Ok(())
}

async fn read_secure_tokens<C: ConnectionTrait + ?Sized>(
    database: &C,
    records: &mut Vec<PendingAuthRecord>,
) -> Result<()> {
    if !validate_optional_table(
        database,
        "secure_tokens",
        &[
            "id",
            "user_id",
            "token",
            "purpose",
            "used_at",
            "expires_at",
            "created_at",
            "updated_at",
        ],
    )
    .await?
    {
        return Ok(());
    }
    let backend = database.get_database_backend();
    let query = format!(
        "SELECT {}, token, purpose, {}, {}, {}, {} FROM secure_tokens ORDER BY id",
        text(backend, "user_id"),
        optional_text(database, "secure_tokens", "used_at").await?,
        text(backend, "expires_at"),
        text(backend, "created_at"),
        text(backend, "updated_at"),
    );
    for row in query_all(database, query, "reading source secure tokens").await? {
        records.push(PendingAuthRecord::SecureToken {
            source_user_id: value(&row, 0, "secure-token owner")?,
            token: SecretString::from(value::<String>(&row, 1, "secure token")?),
            purpose: value(&row, 2, "secure-token purpose")?,
            used_at: optional(&row, 3, "secure-token use timestamp")?,
            expires_at: value(&row, 4, "secure-token expiry")?,
            created_at: value(&row, 5, "secure-token creation timestamp")?,
            updated_at: value(&row, 6, "secure-token update timestamp")?,
        });
    }
    Ok(())
}

async fn read_failed_attempts<C: ConnectionTrait + ?Sized>(
    database: &C,
    records: &mut Vec<PendingAuthRecord>,
) -> Result<()> {
    if !validate_optional_table(
        database,
        "failed_login_attempts",
        &["id", "email", "ip_address", "attempted_at"],
    )
    .await?
    {
        return Ok(());
    }
    let backend = database.get_database_backend();
    let query = format!(
        "SELECT {}, email, {}, {} FROM failed_login_attempts ORDER BY id",
        text(backend, "id"),
        optional_text(database, "failed_login_attempts", "ip_address").await?,
        text(backend, "attempted_at"),
    );
    for row in query_all(database, query, "reading failed-login history").await? {
        records.push(PendingAuthRecord::FailedLoginAttempt(
            ImportedFailedLoginAttempt {
                source_record_id: value(&row, 0, "failed-login source id")?,
                email: value(&row, 1, "failed-login email")?,
                ip_address: optional(&row, 2, "failed-login address")?,
                attempted_at: value(&row, 3, "failed-login timestamp")?,
            },
        ));
    }
    Ok(())
}

async fn read_two_factor<C: ConnectionTrait + ?Sized>(
    database: &C,
    records: &mut Vec<PendingAuthRecord>,
) -> Result<()> {
    if !validate_optional_table(
        database,
        "two_factor_credentials",
        &[
            "user_id",
            "secret",
            "confirmed_at",
            "recovery_codes",
            "last_used_timestep",
            "created_at",
            "updated_at",
        ],
    )
    .await?
    {
        return Ok(());
    }
    let backend = database.get_database_backend();
    let query = format!(
        "SELECT {}, secret, {}, {}, {}, {}, {} FROM two_factor_credentials ORDER BY {}",
        text(backend, "user_id"),
        optional_text(database, "two_factor_credentials", "confirmed_at").await?,
        optional_text(database, "two_factor_credentials", "recovery_codes").await?,
        optional_integer(database, "two_factor_credentials", "last_used_timestep").await?,
        text(backend, "created_at"),
        text(backend, "updated_at"),
        text(backend, "user_id"),
    );
    for row in query_all(database, query, "reading source two-factor credentials").await? {
        records.push(PendingAuthRecord::TwoFactorCredential {
            source_user_id: value(&row, 0, "two-factor owner")?,
            secret: SecretString::from(value::<String>(&row, 1, "two-factor secret")?),
            confirmed_at: optional(&row, 2, "two-factor confirmation")?,
            recovery_codes: optional::<String>(&row, 3, "two-factor recovery codes")?
                .map(SecretString::from),
            last_used_timestep: optional(&row, 4, "two-factor last timestep")?,
            created_at: value(&row, 5, "two-factor creation timestamp")?,
            updated_at: value(&row, 6, "two-factor update timestamp")?,
        });
    }
    Ok(())
}

async fn validate_required_table<C: ConnectionTrait + ?Sized>(
    database: &C,
    table: &str,
    required_columns: &[&str],
) -> Result<()> {
    if validate_optional_table(database, table, required_columns).await? {
        Ok(())
    } else {
        Err(Error::Conflict {
            resource: "durable source table".to_owned(),
            message: format!("{table} is required for the detected source shape"),
        })
    }
}

pub(crate) async fn validate_optional_table<C: ConnectionTrait + ?Sized>(
    database: &C,
    table: &str,
    required_columns: &[&str],
) -> Result<bool> {
    if !has_table(database, None, table).await? {
        return Ok(false);
    }
    let mut missing = Vec::new();
    for column in required_columns {
        if !has_column(database, None, table, column).await? {
            missing.push(*column);
        }
    }
    if missing.is_empty() {
        Ok(true)
    } else {
        Err(Error::Conflict {
            resource: "durable source table".to_owned(),
            message: format!("{table} is missing required columns {}", missing.join(",")),
        })
    }
}

async fn optional_text<C: ConnectionTrait + ?Sized>(
    database: &C,
    table: &str,
    column: &str,
) -> Result<String> {
    if has_column(database, None, table, column).await? {
        Ok(text(database.get_database_backend(), column))
    } else {
        Ok("NULL".to_owned())
    }
}

async fn optional_integer<C: ConnectionTrait + ?Sized>(
    database: &C,
    table: &str,
    column: &str,
) -> Result<String> {
    if has_column(database, None, table, column).await? {
        Ok(column.to_owned())
    } else {
        Ok("NULL".to_owned())
    }
}

fn text(backend: DbBackend, expression: &str) -> String {
    if backend == DbBackend::MySql {
        format!("CAST({expression} AS CHAR)")
    } else {
        format!("CAST({expression} AS TEXT)")
    }
}

async fn query_all<C: ConnectionTrait + ?Sized>(
    database: &C,
    query: String,
    context: &str,
) -> Result<Vec<QueryResult>> {
    database.query_all_raw(Statement::from_string(database.get_database_backend(),
    query,))
        .await
        .map_err(|error| database_error(context, error))
}

fn value<T>(row: &QueryResult, index: usize, context: &str) -> Result<T>
where
    T: sea_orm::TryGetable,
{
    row.try_get_by_index(index)
        .map_err(|error| database_error(context, error))
}

fn optional<T>(row: &QueryResult, index: usize, context: &str) -> Result<Option<T>>
where
    T: sea_orm::TryGetable,
{
    row.try_get_by_index(index)
        .map_err(|error| database_error(context, error))
}
