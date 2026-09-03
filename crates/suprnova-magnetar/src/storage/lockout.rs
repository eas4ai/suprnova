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
use sea_orm::{
    ColumnTrait, Condition, DatabaseTransaction, EntityTrait, IntoActiveModel, QueryFilter,
    QueryOrder, QuerySelect,
};
use sha2::{Digest, Sha256};

use super::{SeaOrmStorage, db_error, in_transaction};
use crate::schema::{AuthSchema, EntityBinding, LockoutFields, UserFields};
use crate::{Error, Result};

/// Aggregated failed-attempt statistics inside one counting window.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttemptStats {
    /// Number of failed attempts in the window.
    pub count: u32,
    /// Timestamp of the most recent attempt, when any exists.
    pub latest_at: Option<DateTime<Utc>>,
}

/// Result of atomically reserving capacity for one verification attempt.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AttemptReservation {
    /// Whether the caller reserved one attempt row and may evaluate proof.
    pub admitted: bool,
    /// Exact finalized-failure state observed while holding the per-identity
    /// lock. Pending reservations consume capacity but are excluded here.
    pub stats: AttemptStats,
    /// Unique row identifier for the admitted pending reservation.
    pub reservation_id: Option<String>,
    /// True only when a rejected admission atomically repaired the durable
    /// user lock transition for the finalized cycle it observed.
    pub locked_event: bool,
}

/// Result of atomically finalizing one reserved failed attempt.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AttemptFinalization {
    /// Exact finalized-failure state committed with the user transition.
    pub stats: AttemptStats,
    /// True only when this transaction won the durable unlocked-to-locked
    /// transition for the finalized cycle.
    pub locked_event: bool,
}

const SERIALIZATION_SENTINEL_REASON: &str = "__magnetar_lockout_serialization__";
const PENDING_RESERVATION_REASON_PREFIX: &str = "__magnetar_lockout_pending__:";
const FINALIZED_RESERVATION_REASON_PREFIX: &str = "__magnetar_lockout_finalized__:";
const LOCKOUT_INTERNAL_REASON_MIN_BYTES: usize = 255;
const LOCKOUT_ATTEMPT_ID_LIMIT: u64 = i64::MAX as u64 / 2;

fn serialization_sentinel_id(identity: &str) -> String {
    let digest = Sha256::digest(identity.as_bytes());
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&digest[..8]);
    let sentinel_slots = i64::MAX as u64 - LOCKOUT_ATTEMPT_ID_LIMIT;
    (LOCKOUT_ATTEMPT_ID_LIMIT + 1 + u64::from_be_bytes(prefix) % sentinel_slots).to_string()
}

fn lockout_attempt_id() -> String {
    (rand::random::<u64>() % LOCKOUT_ATTEMPT_ID_LIMIT + 1).to_string()
}

fn reservation_reason(prefix: &str, reservation_id: &str, context: Option<&str>) -> Result<String> {
    let context = match context {
        None => "n".to_owned(),
        Some(value) => format!("s{}:{value}", value.len()),
    };
    let reason = format!("{prefix}{reservation_id}:{context}");
    if reason.len() > LOCKOUT_INTERNAL_REASON_MIN_BYTES {
        return Err(Error::DependencyUnavailable {
            dependency: "lockout reason storage".to_owned(),
            message: format!(
                "reserved attempt state requires at most {LOCKOUT_INTERNAL_REASON_MIN_BYTES} UTF-8 bytes; the supplied audit context is too long"
            ),
        });
    }
    Ok(reason)
}

fn pending_reservation_reason(reservation_id: &str, context: Option<&str>) -> Result<String> {
    reservation_reason(PENDING_RESERVATION_REASON_PREFIX, reservation_id, context)
}

fn finalized_reservation_reason(reservation_id: &str, context: Option<&str>) -> Result<String> {
    reservation_reason(FINALIZED_RESERVATION_REASON_PREFIX, reservation_id, context)
}

fn is_pending_reservation<S>(row: &<S::Lockout as EntityBinding>::Model) -> bool
where
    S: AuthSchema,
    S::Lockout: LockoutFields,
{
    let reservation_id = S::Lockout::read_lockout_id(row);
    let prefix = format!("{PENDING_RESERVATION_REASON_PREFIX}{reservation_id}:");
    S::Lockout::read_reason(row).is_some_and(|reason| reason.starts_with(&prefix))
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
    /// Atomically reserve one attempt when the current in-window count is
    /// below `max_attempts`. Rejected calls do not insert another row. A
    /// reservation left pending by a crashed verifier conservatively consumes
    /// capacity only until it falls outside `window_start` or cleanup removes it.
    /// When finalized failures already meet the threshold, implementations
    /// must repair the user lock stamp within the same identity transaction and
    /// return that exact transition through [`AttemptReservation::locked_event`].
    async fn admit_attempt_and_stats(
        &self,
        identity: &str,
        at: DateTime<Utc>,
        context: Option<&str>,
        window_start: DateTime<Utc>,
        max_attempts: u32,
    ) -> Result<AttemptReservation> {
        let _ = (identity, at, context, window_start, max_attempts);
        Err(Error::DependencyUnavailable {
            dependency: "lockout store".to_owned(),
            message: "atomic attempt admission is unavailable".to_owned(),
        })
    }
    /// Delete exactly one still-pending reservation.
    async fn cancel_attempt_reservation(
        &self,
        identity: &str,
        reservation_id: &str,
    ) -> Result<bool> {
        let _ = (identity, reservation_id);
        Err(Error::DependencyUnavailable {
            dependency: "lockout store".to_owned(),
            message: "attempt reservation cancellation is unavailable".to_owned(),
        })
    }
    /// Finalize exactly one pending reservation and atomically apply any user
    /// lock transition required at `max_attempts`.
    ///
    /// Implementations own both writes and the transition signal: returning
    /// success means the finalized row and user stamp committed together.
    /// The default fails closed so legacy stores cannot silently split them.
    async fn finalize_attempt_reservation(
        &self,
        identity: &str,
        reservation_id: &str,
        finalized_at: DateTime<Utc>,
        context: Option<&str>,
        window_start: DateTime<Utc>,
        max_attempts: u32,
    ) -> Result<AttemptFinalization> {
        let _ = (
            identity,
            reservation_id,
            finalized_at,
            context,
            window_start,
            max_attempts,
        );
        Err(Error::DependencyUnavailable {
            dependency: "lockout store".to_owned(),
            message: "attempt reservation finalization is unavailable".to_owned(),
        })
    }
    /// Clear prior finalized failures and the successful proof's exact pending
    /// reservation atomically with the user lock stamp. Other in-flight
    /// reservations remain pending so their owners can finalize or cancel.
    ///
    /// This is deliberately distinct from [`Self::clear_attempts`], whose
    /// legacy primary-auth callers retain their established split lifecycle.
    /// The default fails closed for source-compatible custom stores.
    async fn reset_admitted_attempts(
        &self,
        identity: &str,
        reservation_id: &str,
        context: Option<&str>,
    ) -> Result<u64> {
        let _ = (identity, reservation_id, context);
        Err(Error::DependencyUnavailable {
            dependency: "lockout store".to_owned(),
            message: "atomic admitted-attempt reset is unavailable".to_owned(),
        })
    }
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

impl<S> SeaOrmStorage<S>
where
    S: AuthSchema,
    S::Lockout: LockoutFields,
    S::User: UserFields,
    <S::Lockout as EntityBinding>::Entity: EntityTrait<
            Model = <S::Lockout as EntityBinding>::Model,
            ActiveModel = <S::Lockout as EntityBinding>::ActiveModel,
        >,
    <S::Lockout as EntityBinding>::Column: ColumnTrait,
    <S::User as EntityBinding>::Entity: EntityTrait<
            Model = <S::User as EntityBinding>::Model,
            ActiveModel = <S::User as EntityBinding>::ActiveModel,
        >,
    <S::User as EntityBinding>::Column: ColumnTrait,
{
    async fn lock_user_for_cycle(
        transaction: &DatabaseTransaction,
        identity: &str,
        locked_at: DateTime<Utc>,
        window_start: DateTime<Utc>,
    ) -> Result<bool> {
        let mut user = <S::User as EntityBinding>::ActiveModel::default();
        S::User::write_locked_at(&mut user, Some(locked_at));
        let lock_column = S::User::locked_at_column();
        let update = <S::User as EntityBinding>::Entity::update_many()
            .set(user)
            .filter(S::User::email_column().eq(identity.to_owned()))
            .filter(
                Condition::any()
                    .add(lock_column.is_null())
                    .add(S::User::locked_at_column().lt(window_start)),
            )
            .exec(transaction)
            .await
            .map_err(db_error)?;
        Ok(update.rows_affected > 0)
    }

    async fn clear_user_lock(transaction: &DatabaseTransaction, identity: &str) -> Result<()> {
        let mut user = <S::User as EntityBinding>::ActiveModel::default();
        S::User::write_locked_at(&mut user, None);
        let _ = <S::User as EntityBinding>::Entity::update_many()
            .set(user)
            .filter(S::User::email_column().eq(identity.to_owned()))
            .exec(transaction)
            .await
            .map_err(db_error)?;
        Ok(())
    }

    async fn identity_rows_after_lock(
        transaction: &DatabaseTransaction,
        identity: &str,
        at: DateTime<Utc>,
    ) -> Result<Vec<<S::Lockout as EntityBinding>::Model>> {
        let sentinel_id = serialization_sentinel_id(identity);
        let mut sentinel: <S::Lockout as EntityBinding>::ActiveModel = Default::default();
        S::Lockout::write_lockout_id(&mut sentinel, &sentinel_id);
        S::Lockout::write_user_id(&mut sentinel, identity);
        S::Lockout::write_attempted_at(&mut sentinel, at);
        S::Lockout::write_reason(&mut sentinel, Some(SERIALIZATION_SENTINEL_REASON));
        S::Lockout::write_locked_at(&mut sentinel, None);
        let _ = <S::Lockout as EntityBinding>::Entity::insert(sentinel)
            .on_conflict_do_nothing()
            .exec_without_returning(transaction)
            .await
            .map_err(db_error)?;

        let locked_rows = <S::Lockout as EntityBinding>::Entity::find()
            .filter(S::Lockout::user_id_column().eq(identity.to_owned()))
            .lock_exclusive()
            .all(transaction)
            .await
            .map_err(db_error)?;
        if !locked_rows
            .iter()
            .any(|row| S::Lockout::read_lockout_id(row) == sentinel_id)
        {
            return Err(Error::Internal {
                message: "lockout serialization sentinel could not be read".to_owned(),
            });
        }

        // PostgreSQL READ COMMITTED takes a statement snapshot before
        // waiting on `FOR UPDATE`. Count in a second statement so a waiter
        // observes rows committed by the previous lock holder.
        <S::Lockout as EntityBinding>::Entity::find()
            .filter(S::Lockout::user_id_column().eq(identity.to_owned()))
            .all(transaction)
            .await
            .map_err(db_error)
    }

    async fn record_lockout_attempt(
        &self,
        identity: &str,
        at: DateTime<Utc>,
        context: Option<&str>,
        window_start: DateTime<Utc>,
        max_attempts: Option<u32>,
    ) -> Result<AttemptReservation> {
        if identity.is_empty() {
            return Err(empty("identity"));
        }
        let identity = identity.to_owned();
        let context = context.map(ToOwned::to_owned);
        let sentinel_id = serialization_sentinel_id(&identity);
        in_transaction(self.database(), move |transaction| {
            Box::pin(async move {
                let rows =
                    Self::identity_rows_after_lock(transaction.connection(), &identity, at).await?;
                let mut capacity_count = 0_u32;
                let mut stats = AttemptStats::default();
                for row in &rows {
                    if S::Lockout::read_lockout_id(row) == sentinel_id {
                        continue;
                    }
                    let attempted_at = S::Lockout::read_attempted_at(row);
                    if attempted_at >= window_start {
                        capacity_count = capacity_count.saturating_add(1);
                        if !is_pending_reservation::<S>(row) {
                            stats.count = stats.count.saturating_add(1);
                            stats.latest_at = Some(
                                stats
                                    .latest_at
                                    .map_or(attempted_at, |latest| latest.max(attempted_at)),
                            );
                        }
                    }
                }
                if max_attempts.is_some_and(|maximum| capacity_count >= maximum) {
                    let locked_event = if max_attempts
                        .is_some_and(|maximum| stats.count >= maximum)
                    {
                        match stats.latest_at {
                            Some(cycle_at) => {
                                Self::lock_user_for_cycle(
                                    transaction.connection(),
                                    &identity,
                                    cycle_at,
                                    window_start,
                                )
                                .await?
                            }
                            None => false,
                        }
                    } else {
                        false
                    };
                    return Ok(AttemptReservation {
                        admitted: false,
                        stats,
                        reservation_id: None,
                        locked_event,
                    });
                }

                let reservation_id = lockout_attempt_id();
                let pending_reason = max_attempts
                    .map(|_| pending_reservation_reason(&reservation_id, context.as_deref()))
                    .transpose()?;
                // Validate both lifecycle states before reserving capacity so
                // an admitted proof can always be finalized losslessly.
                if max_attempts.is_some() {
                    let _ = finalized_reservation_reason(&reservation_id, context.as_deref())?;
                }
                let mut attempt: <S::Lockout as EntityBinding>::ActiveModel = Default::default();
                S::Lockout::write_lockout_id(&mut attempt, &reservation_id);
                S::Lockout::write_user_id(&mut attempt, &identity);
                S::Lockout::write_attempted_at(&mut attempt, at);
                S::Lockout::write_reason(
                    &mut attempt,
                    pending_reason.as_deref().or(context.as_deref()),
                );
                S::Lockout::write_locked_at(&mut attempt, None);
                <S::Lockout as EntityBinding>::Entity::insert(attempt)
                    .exec(transaction.connection())
                    .await
                    .map_err(db_error)?;

                if let Some(expected_reason) = pending_reason.as_deref() {
                    let inserted = <S::Lockout as EntityBinding>::Entity::find()
                        .filter(S::Lockout::user_id_column().eq(identity.clone()))
                        .all(transaction.connection())
                        .await
                        .map_err(db_error)?
                        .into_iter()
                        .find(|row| S::Lockout::read_lockout_id(row) == reservation_id);
                    if inserted.as_ref().and_then(S::Lockout::read_reason).as_deref()
                        != Some(expected_reason)
                    {
                        return Err(Error::DependencyUnavailable {
                            dependency: "lockout reason storage".to_owned(),
                            message: format!(
                                "reserved attempt state did not round-trip exactly; LockoutFields reason storage must preserve at least {LOCKOUT_INTERNAL_REASON_MIN_BYTES} UTF-8 bytes without truncation, normalization, or omission"
                            ),
                        });
                    }
                }

                if max_attempts.is_none() && at >= window_start {
                    stats.count = stats.count.saturating_add(1);
                    stats.latest_at = Some(stats.latest_at.map_or(at, |latest| latest.max(at)));
                }
                Ok(AttemptReservation {
                    admitted: true,
                    stats,
                    reservation_id: max_attempts.map(|_| reservation_id),
                    locked_event: false,
                })
            })
        })
        .await
    }
}

#[async_trait]
impl<S> LockoutStore for SeaOrmStorage<S>
where
    S: AuthSchema,
    S::Lockout: LockoutFields,
    S::User: UserFields,
    <S::Lockout as EntityBinding>::Entity: EntityTrait<
            Model = <S::Lockout as EntityBinding>::Model,
            ActiveModel = <S::Lockout as EntityBinding>::ActiveModel,
        >,
    <S::Lockout as EntityBinding>::Column: ColumnTrait,
    <S::User as EntityBinding>::Entity: EntityTrait<
            Model = <S::User as EntityBinding>::Model,
            ActiveModel = <S::User as EntityBinding>::ActiveModel,
        >,
    <S::User as EntityBinding>::Column: ColumnTrait,
{
    async fn record_attempt_and_stats(
        &self,
        identity: &str,
        at: DateTime<Utc>,
        context: Option<&str>,
        window_start: DateTime<Utc>,
    ) -> Result<AttemptStats> {
        Ok(self
            .record_lockout_attempt(identity, at, context, window_start, None)
            .await?
            .stats)
    }

    async fn admit_attempt_and_stats(
        &self,
        identity: &str,
        at: DateTime<Utc>,
        context: Option<&str>,
        window_start: DateTime<Utc>,
        max_attempts: u32,
    ) -> Result<AttemptReservation> {
        self.record_lockout_attempt(identity, at, context, window_start, Some(max_attempts))
            .await
    }

    async fn cancel_attempt_reservation(
        &self,
        identity: &str,
        reservation_id: &str,
    ) -> Result<bool> {
        if identity.is_empty() {
            return Err(empty("identity"));
        }
        if reservation_id.is_empty() {
            return Err(empty("reservation_id"));
        }
        let identity = identity.to_owned();
        let reservation_id = reservation_id.to_owned();
        in_transaction(self.database(), move |transaction| {
            Box::pin(async move {
                let rows =
                    Self::identity_rows_after_lock(transaction.connection(), &identity, Utc::now())
                        .await?;
                let Some(row) = rows.into_iter().find(|row| {
                    S::Lockout::read_lockout_id(row) == reservation_id
                        && is_pending_reservation::<S>(row)
                }) else {
                    return Ok(false);
                };
                let deleted =
                    <S::Lockout as EntityBinding>::Entity::delete(row.into_active_model())
                        .exec(transaction.connection())
                        .await
                        .map_err(db_error)?;
                Ok(deleted.rows_affected == 1)
            })
        })
        .await
    }

    async fn finalize_attempt_reservation(
        &self,
        identity: &str,
        reservation_id: &str,
        finalized_at: DateTime<Utc>,
        context: Option<&str>,
        window_start: DateTime<Utc>,
        max_attempts: u32,
    ) -> Result<AttemptFinalization> {
        if identity.is_empty() {
            return Err(empty("identity"));
        }
        if reservation_id.is_empty() {
            return Err(empty("reservation_id"));
        }
        let identity = identity.to_owned();
        let reservation_id = reservation_id.to_owned();
        let context = context.map(ToOwned::to_owned);
        in_transaction(self.database(), move |transaction| {
            Box::pin(async move {
                let rows = Self::identity_rows_after_lock(
                    transaction.connection(),
                    &identity,
                    finalized_at,
                )
                .await?;
                let expected_reason =
                    pending_reservation_reason(&reservation_id, context.as_deref())?;
                let finalized_reason =
                    finalized_reservation_reason(&reservation_id, context.as_deref())?;
                let Some(target_index) = rows.iter().position(|row| {
                    S::Lockout::read_lockout_id(row) == reservation_id
                        && matches!(
                            S::Lockout::read_reason(row).as_deref(),
                            Some(reason)
                                if reason == expected_reason || reason == finalized_reason
                        )
                }) else {
                    return Err(Error::Conflict {
                        resource: "attempt reservation".to_owned(),
                        message: "reservation is missing or already finalized".to_owned(),
                    });
                };

                let mut stats = AttemptStats::default();
                for (index, row) in rows.iter().enumerate() {
                    if S::Lockout::read_lockout_id(row) == serialization_sentinel_id(&identity)
                        || (index != target_index && is_pending_reservation::<S>(row))
                    {
                        continue;
                    }
                    let attempted_at = S::Lockout::read_attempted_at(row);
                    if attempted_at >= window_start {
                        stats.count = stats.count.saturating_add(1);
                        stats.latest_at = Some(
                            stats
                                .latest_at
                                .map_or(attempted_at, |latest| latest.max(attempted_at)),
                        );
                    }
                }

                let row = rows
                    .into_iter()
                    .nth(target_index)
                    .ok_or_else(|| Error::Internal {
                        message: "attempt reservation disappeared during finalization".to_owned(),
                    })?;
                if S::Lockout::read_reason(&row).as_deref() == Some(expected_reason.as_str()) {
                    let mut active = row.into_active_model();
                    S::Lockout::write_reason(&mut active, Some(&finalized_reason));
                    S::Lockout::write_locked_at(&mut active, Some(finalized_at));
                    <S::Lockout as EntityBinding>::Entity::update(active)
                        .exec(transaction.connection())
                        .await
                        .map_err(db_error)?;
                    let finalized = <S::Lockout as EntityBinding>::Entity::find()
                        .filter(S::Lockout::user_id_column().eq(identity.clone()))
                        .all(transaction.connection())
                        .await
                        .map_err(db_error)?
                        .into_iter()
                        .find(|row| S::Lockout::read_lockout_id(row) == reservation_id);
                    if finalized.as_ref().and_then(S::Lockout::read_reason).as_deref()
                        != Some(finalized_reason.as_str())
                    {
                        return Err(Error::DependencyUnavailable {
                            dependency: "lockout reason storage".to_owned(),
                            message: format!(
                                "finalized attempt state did not round-trip exactly; LockoutFields reason storage must preserve at least {LOCKOUT_INTERNAL_REASON_MIN_BYTES} UTF-8 bytes without truncation, normalization, or omission"
                            ),
                        });
                    }
                }
                let locked_event = if stats.count >= max_attempts {
                    match stats.latest_at {
                        Some(cycle_at) => {
                            Self::lock_user_for_cycle(
                                transaction.connection(),
                                &identity,
                                cycle_at,
                                window_start,
                            )
                            .await?
                        }
                        None => false,
                    }
                } else {
                    false
                };
                Ok(AttemptFinalization {
                    stats,
                    locked_event,
                })
            })
        })
        .await
    }

    async fn reset_admitted_attempts(
        &self,
        identity: &str,
        reservation_id: &str,
        context: Option<&str>,
    ) -> Result<u64> {
        if identity.is_empty() {
            return Err(empty("identity"));
        }
        if reservation_id.is_empty() {
            return Err(empty("reservation_id"));
        }
        let identity = identity.to_owned();
        let reservation_id = reservation_id.to_owned();
        let context = context.map(ToOwned::to_owned);
        in_transaction(self.database(), move |transaction| {
            Box::pin(async move {
                let rows =
                    Self::identity_rows_after_lock(transaction.connection(), &identity, Utc::now())
                        .await?;
                let expected_reason =
                    pending_reservation_reason(&reservation_id, context.as_deref())?;
                if !rows.iter().any(|row| {
                    S::Lockout::read_lockout_id(row) == reservation_id
                        && S::Lockout::read_reason(row).as_deref() == Some(expected_reason.as_str())
                }) {
                    return Err(Error::Conflict {
                        resource: "attempt reservation".to_owned(),
                        message: "reservation is missing or no longer pending".to_owned(),
                    });
                }

                let sentinel_id = serialization_sentinel_id(&identity);
                let mut removed = 0_u64;
                for row in rows {
                    let row_id = S::Lockout::read_lockout_id(&row);
                    if row_id == sentinel_id
                        || (row_id != reservation_id && is_pending_reservation::<S>(&row))
                    {
                        continue;
                    }
                    let deleted =
                        <S::Lockout as EntityBinding>::Entity::delete(row.into_active_model())
                            .exec(transaction.connection())
                            .await
                            .map_err(db_error)?;
                    removed = removed.saturating_add(deleted.rows_affected);
                }
                Self::clear_user_lock(transaction.connection(), &identity).await?;
                Ok(removed)
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
        let mut attempts = rows.iter().filter(|row| {
            S::Lockout::read_lockout_id(row) != sentinel_id && !is_pending_reservation::<S>(row)
        });
        let latest_at = attempts.next().map(S::Lockout::read_attempted_at);
        let count = u32::try_from(
            rows.iter()
                .filter(|row| {
                    S::Lockout::read_lockout_id(row) != sentinel_id
                        && !is_pending_reservation::<S>(row)
                })
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

#[cfg(test)]
mod tests {
    #[cfg(feature = "seaorm-postgres")]
    use chrono::TimeZone;
    #[cfg(feature = "seaorm-postgres")]
    use sea_orm::{DbBackend, MockDatabase, MockExecResult};

    use super::*;
    #[cfg(any(feature = "seaorm-postgres", feature = "seaorm-sqlite"))]
    use crate::default_schema::DefaultAuthSchema;
    #[cfg(feature = "seaorm-postgres")]
    use crate::default_schema::lockouts;

    mod unsigned_lockouts {
        use chrono::{DateTime, Utc};
        use sea_orm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
        #[sea_orm(table_name = "unsigned_lockouts")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub id: u64,
            pub identity: String,
            pub attempted_at: DateTime<Utc>,
            pub locked_at: Option<DateTime<Utc>>,
            pub reason: Option<String>,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }

    #[cfg(feature = "seaorm-sqlite")]
    mod lossy_lockouts {
        use chrono::{DateTime, Utc};
        use sea_orm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
        #[sea_orm(table_name = "lossy_lockouts")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub id: i64,
            pub identity: String,
            pub attempted_at: DateTime<Utc>,
            pub locked_at: Option<DateTime<Utc>>,
            pub reason: Option<String>,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }

    #[cfg(feature = "seaorm-sqlite")]
    mod finalized_lossy_lockouts {
        use chrono::{DateTime, Utc};
        use sea_orm::entity::prelude::*;

        #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
        #[sea_orm(table_name = "finalized_lossy_lockouts")]
        pub struct Model {
            #[sea_orm(primary_key, auto_increment = false)]
            pub id: i64,
            pub identity: String,
            pub attempted_at: DateTime<Utc>,
            pub locked_at: Option<DateTime<Utc>>,
            pub reason: Option<String>,
        }

        #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
        pub enum Relation {}

        impl ActiveModelBehavior for ActiveModel {}
    }

    #[cfg(feature = "seaorm-sqlite")]
    impl EntityBinding for lossy_lockouts::Entity {
        type Entity = lossy_lockouts::Entity;
        type Column = lossy_lockouts::Column;
        type PrimaryKey = lossy_lockouts::PrimaryKey;
        type Model = lossy_lockouts::Model;
        type ActiveModel = lossy_lockouts::ActiveModel;
    }

    #[cfg(feature = "seaorm-sqlite")]
    impl LockoutFields for lossy_lockouts::Entity {
        fn read_lockout_id(model: &Self::Model) -> String {
            model.id.to_string()
        }

        fn write_lockout_id(model: &mut Self::ActiveModel, value: &str) {
            model.id = sea_orm::ActiveValue::Set(value.parse().expect("lossy lockout id"));
        }

        fn read_user_id(model: &Self::Model) -> String {
            model.identity.clone()
        }

        fn user_id_column() -> Self::Column {
            lossy_lockouts::Column::Identity
        }

        fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
            model.identity = sea_orm::ActiveValue::Set(value.to_owned());
        }

        fn read_attempted_at(model: &Self::Model) -> DateTime<Utc> {
            model.attempted_at
        }

        fn attempted_at_column() -> Self::Column {
            lossy_lockouts::Column::AttemptedAt
        }

        fn write_attempted_at(model: &mut Self::ActiveModel, value: DateTime<Utc>) {
            model.attempted_at = sea_orm::ActiveValue::Set(value);
        }

        fn read_locked_at(model: &Self::Model) -> Option<DateTime<Utc>> {
            model.locked_at
        }

        fn read_reason(model: &Self::Model) -> Option<String> {
            model.reason.clone()
        }

        fn write_reason(model: &mut Self::ActiveModel, value: Option<&str>) {
            model.reason =
                sea_orm::ActiveValue::Set(value.map(|reason| reason.chars().take(12).collect()));
        }

        fn write_locked_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>) {
            model.locked_at = sea_orm::ActiveValue::Set(value);
        }
    }

    #[cfg(feature = "seaorm-sqlite")]
    impl EntityBinding for finalized_lossy_lockouts::Entity {
        type Entity = finalized_lossy_lockouts::Entity;
        type Column = finalized_lossy_lockouts::Column;
        type PrimaryKey = finalized_lossy_lockouts::PrimaryKey;
        type Model = finalized_lossy_lockouts::Model;
        type ActiveModel = finalized_lossy_lockouts::ActiveModel;
    }

    #[cfg(feature = "seaorm-sqlite")]
    impl LockoutFields for finalized_lossy_lockouts::Entity {
        fn read_lockout_id(model: &Self::Model) -> String {
            model.id.to_string()
        }

        fn write_lockout_id(model: &mut Self::ActiveModel, value: &str) {
            model.id = sea_orm::ActiveValue::Set(value.parse().expect("finalized-lossy id"));
        }

        fn read_user_id(model: &Self::Model) -> String {
            model.identity.clone()
        }

        fn user_id_column() -> Self::Column {
            finalized_lossy_lockouts::Column::Identity
        }

        fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
            model.identity = sea_orm::ActiveValue::Set(value.to_owned());
        }

        fn read_attempted_at(model: &Self::Model) -> DateTime<Utc> {
            model.attempted_at
        }

        fn attempted_at_column() -> Self::Column {
            finalized_lossy_lockouts::Column::AttemptedAt
        }

        fn write_attempted_at(model: &mut Self::ActiveModel, value: DateTime<Utc>) {
            model.attempted_at = sea_orm::ActiveValue::Set(value);
        }

        fn read_locked_at(model: &Self::Model) -> Option<DateTime<Utc>> {
            model.locked_at
        }

        fn read_reason(model: &Self::Model) -> Option<String> {
            model.reason.clone()
        }

        fn write_reason(model: &mut Self::ActiveModel, value: Option<&str>) {
            model.reason = sea_orm::ActiveValue::Set(value.map(|reason| {
                if reason.starts_with(FINALIZED_RESERVATION_REASON_PREFIX) {
                    reason
                        .strip_suffix(|_: char| true)
                        .expect("finalized marker is non-empty")
                        .to_owned()
                } else {
                    reason.to_owned()
                }
            }));
        }

        fn write_locked_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>) {
            model.locked_at = sea_orm::ActiveValue::Set(value);
        }
    }

    #[cfg(feature = "seaorm-sqlite")]
    struct LossySchema;

    #[cfg(feature = "seaorm-sqlite")]
    impl AuthSchema for LossySchema {
        type User = <DefaultAuthSchema as AuthSchema>::User;
        type Session = <DefaultAuthSchema as AuthSchema>::Session;
        type LinkedAccount = <DefaultAuthSchema as AuthSchema>::LinkedAccount;
        type Passkey = <DefaultAuthSchema as AuthSchema>::Passkey;
        type Token = <DefaultAuthSchema as AuthSchema>::Token;
        type Ceremony = <DefaultAuthSchema as AuthSchema>::Ceremony;
        type Lockout = lossy_lockouts::Entity;
        type TokenRecord = <DefaultAuthSchema as AuthSchema>::TokenRecord;
    }

    #[cfg(feature = "seaorm-sqlite")]
    struct FinalizedLossySchema;

    #[cfg(feature = "seaorm-sqlite")]
    impl AuthSchema for FinalizedLossySchema {
        type User = <DefaultAuthSchema as AuthSchema>::User;
        type Session = <DefaultAuthSchema as AuthSchema>::Session;
        type LinkedAccount = <DefaultAuthSchema as AuthSchema>::LinkedAccount;
        type Passkey = <DefaultAuthSchema as AuthSchema>::Passkey;
        type Token = <DefaultAuthSchema as AuthSchema>::Token;
        type Ceremony = <DefaultAuthSchema as AuthSchema>::Ceremony;
        type Lockout = finalized_lossy_lockouts::Entity;
        type TokenRecord = <DefaultAuthSchema as AuthSchema>::TokenRecord;
    }

    impl EntityBinding for unsigned_lockouts::Entity {
        type Entity = unsigned_lockouts::Entity;
        type Column = unsigned_lockouts::Column;
        type PrimaryKey = unsigned_lockouts::PrimaryKey;
        type Model = unsigned_lockouts::Model;
        type ActiveModel = unsigned_lockouts::ActiveModel;
    }

    impl LockoutFields for unsigned_lockouts::Entity {
        fn read_lockout_id(model: &Self::Model) -> String {
            model.id.to_string()
        }

        fn write_lockout_id(model: &mut Self::ActiveModel, value: &str) {
            model.id = sea_orm::ActiveValue::Set(value.parse().expect("unsigned lockout id"));
        }

        fn read_user_id(model: &Self::Model) -> String {
            model.identity.clone()
        }

        fn user_id_column() -> Self::Column {
            unsigned_lockouts::Column::Identity
        }

        fn write_user_id(model: &mut Self::ActiveModel, value: &str) {
            model.identity = sea_orm::ActiveValue::Set(value.to_owned());
        }

        fn read_attempted_at(model: &Self::Model) -> DateTime<Utc> {
            model.attempted_at
        }

        fn attempted_at_column() -> Self::Column {
            unsigned_lockouts::Column::AttemptedAt
        }

        fn write_attempted_at(model: &mut Self::ActiveModel, value: DateTime<Utc>) {
            model.attempted_at = sea_orm::ActiveValue::Set(value);
        }

        fn read_locked_at(model: &Self::Model) -> Option<DateTime<Utc>> {
            model.locked_at
        }

        fn read_reason(model: &Self::Model) -> Option<String> {
            model.reason.clone()
        }

        fn write_reason(model: &mut Self::ActiveModel, value: Option<&str>) {
            model.reason = sea_orm::ActiveValue::Set(value.map(ToOwned::to_owned));
        }

        fn write_locked_at(model: &mut Self::ActiveModel, value: Option<DateTime<Utc>>) {
            model.locked_at = sea_orm::ActiveValue::Set(value);
        }
    }

    #[test]
    fn lockout_serialization_ids_fit_signed_and_unsigned_bindings() {
        let sentinel = serialization_sentinel_id("unsigned@example.test");
        let attempt = lockout_attempt_id();

        assert!(sentinel.parse::<i64>().is_ok());
        assert!(sentinel.parse::<u64>().is_ok());
        assert!(attempt.parse::<i64>().is_ok());
        assert!(attempt.parse::<u64>().is_ok());
        assert!(attempt.parse::<u64>().unwrap() <= LOCKOUT_ATTEMPT_ID_LIMIT);
        assert!(sentinel.parse::<u64>().unwrap() > LOCKOUT_ATTEMPT_ID_LIMIT);

        let mut sentinel_model: unsigned_lockouts::ActiveModel = Default::default();
        unsigned_lockouts::Entity::write_lockout_id(&mut sentinel_model, &sentinel);
        let mut attempt_model: unsigned_lockouts::ActiveModel = Default::default();
        unsigned_lockouts::Entity::write_lockout_id(&mut attempt_model, &attempt);
        assert!(matches!(
            sentinel_model.id,
            sea_orm::ActiveValue::Set(value) if value > LOCKOUT_ATTEMPT_ID_LIMIT
        ));
        assert!(matches!(
            attempt_model.id,
            sea_orm::ActiveValue::Set(value) if value <= LOCKOUT_ATTEMPT_ID_LIMIT
        ));
    }

    #[cfg(feature = "seaorm-sqlite")]
    #[tokio::test]
    async fn lossy_reason_adapter_rolls_back_pending_admission() {
        use sea_orm::{ConnectionTrait, Database, DbBackend, Schema};

        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute(
            &Schema::new(DbBackend::Sqlite)
                .create_table_from_entity(lossy_lockouts::Entity)
                .to_owned(),
        )
        .await
        .unwrap();
        let storage = SeaOrmStorage::<LossySchema>::new(db.clone());
        let at = Utc::now();

        let error = storage
            .admit_attempt_and_stats(
                "lossy@example.test",
                at,
                Some("two-factor challenge"),
                at - chrono::Duration::minutes(15),
                5,
            )
            .await
            .expect_err("a lossy reserved-reason adapter must fail closed");

        assert!(matches!(
            error,
            Error::DependencyUnavailable { dependency, .. }
                if dependency == "lockout reason storage"
        ));
        assert!(
            lossy_lockouts::Entity::find()
                .all(&db)
                .await
                .unwrap()
                .is_empty(),
            "the failed admission transaction must not leave a misclassified row"
        );
    }

    #[cfg(feature = "seaorm-sqlite")]
    #[tokio::test]
    async fn lossy_finalized_reason_rolls_back_finalization() {
        use sea_orm::{ConnectionTrait, Database, DbBackend, Schema};

        let db = Database::connect("sqlite::memory:").await.unwrap();
        db.execute(
            &Schema::new(DbBackend::Sqlite)
                .create_table_from_entity(finalized_lossy_lockouts::Entity)
                .to_owned(),
        )
        .await
        .unwrap();
        let storage = SeaOrmStorage::<FinalizedLossySchema>::new(db.clone());
        let at = Utc::now();
        let identity = "finalized-lossy@example.test";
        let context = "two-factor challenge";
        let admission = storage
            .admit_attempt_and_stats(
                identity,
                at,
                Some(context),
                at - chrono::Duration::minutes(15),
                5,
            )
            .await
            .expect("pending marker is preserved");
        let reservation_id = admission.reservation_id.unwrap();

        let error = storage
            .finalize_attempt_reservation(
                identity,
                &reservation_id,
                at,
                Some(context),
                at - chrono::Duration::minutes(15),
                5,
            )
            .await
            .expect_err("a truncated finalized marker must fail closed");

        assert!(matches!(
            error,
            Error::DependencyUnavailable { dependency, .. }
                if dependency == "lockout reason storage"
        ));
        let rows = finalized_lossy_lockouts::Entity::find()
            .all(&db)
            .await
            .unwrap();
        let reservation = rows
            .iter()
            .find(|row| row.id.to_string() == reservation_id)
            .expect("rolled-back reservation remains pending");
        assert_eq!(
            reservation.reason.as_deref(),
            Some(
                pending_reservation_reason(&reservation_id, Some(context))
                    .unwrap()
                    .as_str()
            )
        );
    }

    #[cfg(feature = "seaorm-postgres")]
    #[tokio::test]
    async fn postgres_admission_reads_count_after_acquiring_identity_lock() {
        let at = Utc.with_ymd_and_hms(2026, 9, 2, 12, 0, 0).unwrap();
        let identity = "snapshot@example.test";
        let sentinel_id = serialization_sentinel_id(identity);
        let sentinel = lockouts::Model {
            id: sentinel_id.parse().unwrap(),
            identity: identity.to_owned(),
            attempted_at: at,
            ip_address: None,
            migration_source_id: None,
            locked_at: None,
            reason: Some(SERIALIZATION_SENTINEL_REASON.to_owned()),
        };
        let db = MockDatabase::new(DbBackend::Postgres)
            .append_exec_results([MockExecResult {
                last_insert_id: 0,
                rows_affected: 1,
            }])
            .append_query_results([[sentinel.clone()], [sentinel]])
            .into_connection();
        let storage = SeaOrmStorage::<DefaultAuthSchema>::new(db.clone());

        let reservation = storage
            .admit_attempt_and_stats(identity, at, None, at, 0)
            .await
            .unwrap();
        assert!(!reservation.admitted);

        let log = db.into_transaction_log();
        let statements = log[0].statements();
        let selects = statements
            .iter()
            .filter(|statement| statement.sql.trim_start().starts_with("SELECT"))
            .collect::<Vec<_>>();
        assert_eq!(
            selects.len(),
            2,
            "PostgreSQL READ COMMITTED requires a fresh count statement after the locking statement"
        );
        assert!(selects[0].sql.ends_with("FOR UPDATE"));
        assert!(!selects[1].sql.ends_with("FOR UPDATE"));
    }
}
