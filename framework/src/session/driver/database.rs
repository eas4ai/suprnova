//! Database-backed session storage driver

use async_trait::async_trait;
use sea_orm::entity::prelude::*;
use sea_orm::sea_query::{Expr, OnConflict};
use sea_orm::{QueryFilter, Set, TransactionTrait};
use std::collections::HashMap;
use std::time::Duration;

use crate::database::DB;
use crate::error::FrameworkError;
use crate::session::store::{SessionData, SessionMigrationError, SessionStore};

/// Database session driver using SeaORM
///
/// Stores sessions in a `sessions` table with the following schema:
/// - id: VARCHAR (primary key) - session ID
/// - user_id: VARCHAR (nullable) - authenticated user ID (string, supports both numeric and opaque IDs)
/// - payload: TEXT - JSON serialized session data
/// - csrf_token: VARCHAR - CSRF protection token
/// - last_activity: TIMESTAMP - last access time
pub struct DatabaseSessionDriver {
    lifetime: Duration,
}

impl DatabaseSessionDriver {
    /// Create a new database session driver
    pub fn new(lifetime: Duration) -> Self {
        Self { lifetime }
    }
}

#[async_trait]
impl SessionStore for DatabaseSessionDriver {
    async fn read(&self, id: &str) -> Result<Option<SessionData>, FrameworkError> {
        let db = DB::connection()?;

        let result = sessions::Entity::find_by_id(id)
            .one(db.inner())
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;

        if let Some(session) = result {
            // Check if expired
            let now = chrono::Utc::now().naive_utc();
            let expiry =
                session.last_activity + chrono::Duration::seconds(self.lifetime.as_secs() as i64);

            if now > expiry {
                // Session expired, clean it up
                let _ = self.destroy(id).await;
                return Ok(None);
            }

            // Parse the payload
            let data: HashMap<String, serde_json::Value> =
                serde_json::from_str(&session.payload).unwrap_or_default();

            Ok(Some(SessionData {
                id: session.id,
                data,
                user_id: session.user_id,
                csrf_token: session.csrf_token,
                dirty: false,
                // This row was read from storage under its own id -
                // see `SessionData::loaded_from_store` (SEC-02(c)).
                loaded_from_store: true,
            }))
        } else {
            Ok(None)
        }
    }

    async fn write(&self, session: &SessionData) -> Result<(), FrameworkError> {
        let db = DB::connection()?;

        let payload = serde_json::to_string(&session.data)
            .map_err(|e| FrameworkError::internal(format!("Session serialize error: {}", e)))?;

        let now = chrono::Utc::now().naive_utc();

        // SEC-02(c): a session that was read from an existing row under
        // `session.id` must be written back as an UPDATE-ONLY - no
        // INSERT fallback. Without this branch, the upsert below would
        // silently recreate (resurrect) a row that was deleted between
        // this request's read and its write - e.g. a concurrent
        // `destroy_for_user` password-reset revocation, or `gc` - and
        // hand the resurrected row's session cookie right back to
        // whoever is holding it, including the party the revocation was
        // meant to lock out. `session.id` is guaranteed fresh (never
        // read as existing) whenever it was set via
        // `SessionData::rotate_id` this request, so the id-rotation
        // paths (login, 2FA promotion, remember-me hydration, manual
        // regenerate, `invalidate_session`) still fall through to the
        // upsert arm and create their new row exactly as before.
        if session.loaded_from_store {
            let result = sessions::Entity::update_many()
                .col_expr(
                    sessions::Column::UserId,
                    Expr::value(session.user_id.clone()),
                )
                .col_expr(sessions::Column::Payload, Expr::value(payload))
                .col_expr(
                    sessions::Column::CsrfToken,
                    Expr::value(session.csrf_token.clone()),
                )
                .col_expr(sessions::Column::LastActivity, Expr::value(now))
                .filter(sessions::Column::Id.eq(&session.id))
                .exec(db.inner())
                .await
                .map_err(|e| FrameworkError::database(e.to_string()))?;

            if result.rows_affected == 0 {
                // The row is gone - most likely a concurrent
                // revocation. Declining to resurrect it is the correct
                // outcome, not a failure: the next read of this
                // session id will correctly find nothing.
                tracing::debug!(
                    session_id = %session.id,
                    "session write skipped: row no longer exists (revoked or expired concurrently)"
                );
            }

            return Ok(());
        }

        // Fresh session (never read as existing under this id) - atomic
        // upsert: INSERT ... ON CONFLICT(id) DO UPDATE SET ...
        // The previous check-then-insert/update was a read-modify-write
        // race - two parallel writers persisting a fresh-but-shared
        // session id (e.g. a SPA reconnecting after the DB row was
        // gc'd while the cookie was still valid) could both see "no
        // existing row" and both attempt INSERT; one would win, the
        // other would fail the UNIQUE constraint, and the SessionMiddleware
        // fail-closed branch would 500 the loser. ON CONFLICT collapses
        // both branches into a single round-trip + skips the pre-read
        // on the happy path. SeaORM 1.x routes the OnConflict::column
        // setup to Postgres `ON CONFLICT DO UPDATE`, MySQL `ON
        // DUPLICATE KEY UPDATE`, and SQLite `ON CONFLICT DO UPDATE`.
        let model = sessions::ActiveModel {
            id: Set(session.id.clone()),
            user_id: Set(session.user_id.clone()),
            payload: Set(payload),
            csrf_token: Set(session.csrf_token.clone()),
            last_activity: Set(now),
        };

        sessions::Entity::insert(model)
            .on_conflict(
                OnConflict::column(sessions::Column::Id)
                    .update_columns([
                        sessions::Column::UserId,
                        sessions::Column::Payload,
                        sessions::Column::CsrfToken,
                        sessions::Column::LastActivity,
                    ])
                    .to_owned(),
            )
            .exec(db.inner())
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;

        Ok(())
    }

    async fn migrate_two_factor_session(
        &self,
        old_id: &str,
        session: &SessionData,
    ) -> Result<(), SessionMigrationError> {
        let db = DB::connection().map_err(SessionMigrationError::RolledBack)?;
        let payload = serde_json::to_string(&session.data).map_err(|e| {
            SessionMigrationError::RolledBack(FrameworkError::internal(format!(
                "Session serialize error: {e}"
            )))
        })?;
        let model = sessions::ActiveModel {
            id: Set(session.id.clone()),
            user_id: Set(session.user_id.clone()),
            payload: Set(payload),
            csrf_token: Set(session.csrf_token.clone()),
            last_activity: Set(chrono::Utc::now().naive_utc()),
        };

        let transaction = db.inner().begin().await.map_err(|e| {
            SessionMigrationError::RolledBack(FrameworkError::database(e.to_string()))
        })?;
        let migration = async {
            let deleted = sessions::Entity::delete_by_id(old_id)
                .exec(&transaction)
                .await
                .map_err(|e| FrameworkError::database(e.to_string()))?;
            if deleted.rows_affected != 1 {
                return Err(FrameworkError::internal(
                    "atomic 2FA session migration requires an existing old session",
                ));
            }

            sessions::Entity::insert(model)
                .exec(&transaction)
                .await
                .map_err(|e| FrameworkError::database(e.to_string()))?;
            Ok(())
        }
        .await;

        match migration {
            Ok(()) => transaction.commit().await.map_err(|e| {
                SessionMigrationError::OutcomeUnknown(FrameworkError::database(e.to_string()))
            }),
            Err(error) => {
                if let Err(rollback_error) = transaction.rollback().await {
                    tracing::error!(
                        operation = "two_factor_session_migration_rollback",
                        classification = "backend_failure",
                        "atomic session migration rollback failed"
                    );
                    return Err(SessionMigrationError::OutcomeUnknown(
                        FrameworkError::database(format!(
                            "session migration failed and rollback was not confirmed: {rollback_error}"
                        )),
                    ));
                }
                Err(SessionMigrationError::RolledBack(error))
            }
        }
    }

    async fn destroy(&self, id: &str) -> Result<(), FrameworkError> {
        let db = DB::connection()?;

        sessions::Entity::delete_by_id(id)
            .exec(db.inner())
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;

        Ok(())
    }

    async fn destroy_for_user(&self, user_id: &str) -> Result<u64, FrameworkError> {
        let db = DB::connection()?;

        let result = sessions::Entity::delete_many()
            .filter(sessions::Column::UserId.eq(user_id))
            .exec(db.inner())
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;

        Ok(result.rows_affected)
    }

    async fn gc(&self) -> Result<u64, FrameworkError> {
        let db = DB::connection()?;

        let threshold = chrono::Utc::now().naive_utc()
            - chrono::Duration::seconds(self.lifetime.as_secs() as i64);

        let result = sessions::Entity::delete_many()
            .filter(sessions::Column::LastActivity.lt(threshold))
            .exec(db.inner())
            .await
            .map_err(|e| FrameworkError::database(e.to_string()))?;

        Ok(result.rows_affected)
    }
}

/// Sessions table entity for SeaORM
pub mod sessions {
    use sea_orm::entity::prelude::*;

    /// SeaORM model for a single row in `sessions`.
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "sessions")]
    pub struct Model {
        /// Session id (the cookie value), kept as the primary key.
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: String,
        /// Authenticated user id, if any; null for guest sessions.
        pub user_id: Option<String>,
        /// Serialized session payload (encoded by the configured session encoder).
        #[sea_orm(column_type = "Text")]
        pub payload: String,
        /// Per-session CSRF token rotated when the session id rotates.
        pub csrf_token: String,
        /// Wall-clock time of the last activity on this session, used for sliding TTL.
        pub last_activity: chrono::NaiveDateTime,
    }

    /// SeaORM relation enum - `sessions` is a leaf table with no declared
    /// foreign-key relations.
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}
