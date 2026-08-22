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
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};
use sha2::{Digest, Sha256};

use super::{SeaOrmStorage, db_error, in_transaction, random_id};
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

const SERIALIZATION_SENTINEL_REASON: &str = "__magnetar_lockout_serialization__";

fn serialization_sentinel_id(identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    let positive = u64::from_be_bytes(prefix) & (i64::MAX as u64);
    (-(positive as i64) - 1).to_string()
}

/// Storage API for failed sign-in attempt records.
#[async_trait]
pub trait LockoutStore: Send + Sync {
    /// Atomically record one failed attempt and return statistics from the
    /// exact post-insert state inside the same counting window.
    async fn record_attempt_and_stats(
        &self,
        identity: &str,
        at: DateTime<Utc>,
        context: Option<&str>,
        window_start: DateTime<Utc>,
    ) -> Result<AttemptStats>;
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
    async fn record_attempt_and_stats(
        &self,
        identity: &str,
        at: DateTime<Utc>,
        context: Option<&str>,
        window_start: DateTime<Utc>,
    ) -> Result<AttemptStats> {
        if identity.is_empty() {
            return Err(empty("identity"));
        }
        let identity = identity.to_owned();
        let context = context.map(ToOwned::to_owned);
        let sentinel_id = serialization_sentinel_id(&identity);
        in_transaction(self.database(), move |transaction| {
            Box::pin(async move {
                let mut sentinel = <S::Lockout as EntityBinding>::ActiveModel::default();
                S::Lockout::write_lockout_id(&mut sentinel, &sentinel_id);
                S::Lockout::write_user_id(&mut sentinel, &identity);
                S::Lockout::write_attempted_at(&mut sentinel, at);
                S::Lockout::write_reason(&mut sentinel, Some(SERIALIZATION_SENTINEL_REASON));
                S::Lockout::write_locked_at(&mut sentinel, None);
                let _ = <S::Lockout as EntityBinding>::Entity::insert(sentinel)
                    .on_conflict_do_nothing()
                    .exec_without_returning(transaction.connection())
                    .await
                    .map_err(db_error)?;

                // Every writer contends on the deterministic sentinel before
                // observing the window, so each caller owns an exact
                // post-insert state on PostgreSQL, MySQL, and SQLite.
                let rows = <S::Lockout as EntityBinding>::Entity::find()
                    .filter(S::Lockout::user_id_column().eq(identity.clone()))
                    .lock_exclusive()
                    .all(transaction.connection())
                    .await
                    .map_err(db_error)?;
                if !rows
                    .iter()
                    .any(|row| S::Lockout::read_lockout_id(row) == sentinel_id)
                {
                    return Err(Error::Internal {
                        message: "lockout serialization sentinel could not be read".to_owned(),
                    });
                }

                let mut attempt = <S::Lockout as EntityBinding>::ActiveModel::default();
                S::Lockout::write_lockout_id(&mut attempt, &random_id());
                S::Lockout::write_user_id(&mut attempt, &identity);
                S::Lockout::write_attempted_at(&mut attempt, at);
                S::Lockout::write_reason(&mut attempt, context.as_deref());
                S::Lockout::write_locked_at(&mut attempt, None);
                <S::Lockout as EntityBinding>::Entity::insert(attempt)
                    .exec(transaction.connection())
                    .await
                    .map_err(db_error)?;

                let mut count = u32::from(at >= window_start);
                let mut latest_at = (at >= window_start).then_some(at);
                for row in &rows {
                    if S::Lockout::read_lockout_id(row) == sentinel_id {
                        continue;
                    }
                    let attempted_at = S::Lockout::read_attempted_at(row);
                    if attempted_at >= window_start {
                        count = count.saturating_add(1);
                        latest_at =
                            Some(latest_at.map_or(attempted_at, |latest| latest.max(attempted_at)));
                    }
                }
                Ok(AttemptStats { count, latest_at })
            })
        })
        .await
    }

    async fn attempt_stats(
        &self,
        identity: &str,
        window_start: DateTime<Utc>,
    ) -> Result<AttemptStats> {
        if identity.is_empty() {
            return Err(empty("identity"));
        }
        let sentinel_id = serialization_sentinel_id(identity);
        let rows = <S::Lockout as EntityBinding>::Entity::find()
            .filter(S::Lockout::user_id_column().eq(identity.to_owned()))
            .filter(S::Lockout::attempted_at_column().gte(window_start))
            .order_by_desc(S::Lockout::attempted_at_column())
            .all(self.database())
            .await
            .map_err(db_error)?;
        let mut attempts = rows
            .iter()
            .filter(|row| S::Lockout::read_lockout_id(row) != sentinel_id);
        let latest_at = attempts.next().map(S::Lockout::read_attempted_at);
        let count = u32::try_from(
            rows.iter()
                .filter(|row| S::Lockout::read_lockout_id(row) != sentinel_id)
                .count(),
        )
        .unwrap_or(u32::MAX);
        Ok(AttemptStats { count, latest_at })
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
