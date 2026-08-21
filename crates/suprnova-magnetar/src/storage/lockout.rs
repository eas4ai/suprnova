//! Failed-attempt records backing account lockout.
//!
//! Rows are keyed by the attempted identity (Magnetar uses the normalized
//! email), which deliberately may not correspond to a stored user: unknown
//! addresses accumulate attempts with exactly the same writes as real
//! accounts, so lockout state cannot be used as an enumeration oracle. The
//! lockout *policy* (threshold, window, transitions) lives in
//! [`crate::password::lockout`]; this module owns only the row mechanics.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder};

use super::{SeaOrmStorage, db_error, random_id};
use crate::schema::{AuthSchema, EntityBinding, LockoutFields};
use crate::{Error, Result};

/// Aggregated failed-attempt statistics inside one counting window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttemptStats {
    /// Number of failed attempts in the window.
    pub count: u32,
    /// Timestamp of the most recent attempt, when any exists.
    pub latest_at: Option<DateTime<Utc>>,
}

/// Storage API for failed sign-in attempt records.
#[async_trait]
pub trait LockoutStore: Send + Sync {
    /// Record one failed attempt for an identity key.
    async fn record_attempt(
        &self,
        identity: &str,
        at: DateTime<Utc>,
        context: Option<&str>,
    ) -> Result<()>;
    /// Aggregate attempts recorded at or after `window_start`.
    async fn attempt_stats(
        &self,
        identity: &str,
        window_start: DateTime<Utc>,
    ) -> Result<AttemptStats>;
    /// Delete every attempt row for an identity and return the count removed.
    async fn clear_attempts(&self, identity: &str) -> Result<u64>;
    /// Delete audit rows older than `before` for identities with no newer
    /// activity, so a currently locked identity keeps its evidence.
    async fn cleanup_attempts_before(&self, before: DateTime<Utc>) -> Result<u64>;
}

fn empty(field: &str) -> Error {
    Error::InvalidInput {
        field: field.to_owned(),
        message: "must not be empty".to_owned(),
    }
}

#[async_trait]
impl<S> LockoutStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::Lockout: LockoutFields,
    <S::Lockout as EntityBinding>::Entity: EntityTrait<
            Model = <S::Lockout as EntityBinding>::Model,
            ActiveModel = <S::Lockout as EntityBinding>::ActiveModel,
        >,
    <S::Lockout as EntityBinding>::Column: ColumnTrait,
{
    async fn record_attempt(
        &self,
        identity: &str,
        at: DateTime<Utc>,
        context: Option<&str>,
    ) -> Result<()> {
        if identity.is_empty() {
            return Err(empty("identity"));
        }
        let mut model = <S::Lockout as EntityBinding>::ActiveModel::default();
        S::Lockout::write_lockout_id(&mut model, &random_id());
        S::Lockout::write_user_id(&mut model, identity);
        S::Lockout::write_attempted_at(&mut model, at);
        S::Lockout::write_reason(&mut model, context);
        S::Lockout::write_locked_at(&mut model, None);
        <S::Lockout as EntityBinding>::Entity::insert(model)
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn attempt_stats(
        &self,
        identity: &str,
        window_start: DateTime<Utc>,
    ) -> Result<AttemptStats> {
        if identity.is_empty() {
            return Err(empty("identity"));
        }
        let rows = <S::Lockout as EntityBinding>::Entity::find()
            .filter(S::Lockout::user_id_column().eq(identity.to_owned()))
            .filter(S::Lockout::attempted_at_column().gte(window_start))
            .order_by_desc(S::Lockout::attempted_at_column())
            .all(self.database())
            .await
            .map_err(db_error)?;
        Ok(AttemptStats {
            count: u32::try_from(rows.len()).unwrap_or(u32::MAX),
            latest_at: rows.first().map(S::Lockout::read_attempted_at),
        })
    }

    async fn clear_attempts(&self, identity: &str) -> Result<u64> {
        if identity.is_empty() {
            return Err(empty("identity"));
        }
        let deleted = <S::Lockout as EntityBinding>::Entity::delete_many()
            .filter(S::Lockout::user_id_column().eq(identity.to_owned()))
            .exec(self.database())
            .await
            .map_err(db_error)?;
        Ok(deleted.rows_affected)
    }

    async fn cleanup_attempts_before(&self, before: DateTime<Utc>) -> Result<u64> {
        // Identities with any attempt at or after `before` are still active
        // (a locked identity always has recent attempts because the lockout
        // window is far shorter than retention); keep their whole trail.
        let recent = <S::Lockout as EntityBinding>::Entity::find()
            .filter(S::Lockout::attempted_at_column().gte(before))
            .all(self.database())
            .await
            .map_err(db_error)?;
        let mut active: Vec<String> = recent.iter().map(S::Lockout::read_user_id).collect();
        active.sort_unstable();
        active.dedup();
        let mut delete = <S::Lockout as EntityBinding>::Entity::delete_many()
            .filter(S::Lockout::attempted_at_column().lt(before));
        if !active.is_empty() {
            delete = delete.filter(S::Lockout::user_id_column().is_not_in(active));
        }
        let deleted = delete.exec(self.database()).await.map_err(db_error)?;
        Ok(deleted.rows_affected)
    }
}
