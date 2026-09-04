//! Suprnova database transaction adaptation for Live execution.
//!
//! # Why `begin` does not hold a connection open (ruling R76)
//!
//! `HostTransaction` exposes only `commit` and `rollback` - there is no
//! query capability on it, and the Live executor never hands it to the
//! action body it wraps. Nothing an action does - `Model::save`,
//! `Builder::update_all`, any ordinary ORM call - writes through this
//! transaction; every one of those calls resolves its connection through
//! the ordinary `DB::connection()` pool path (or the ambient `CURRENT_TX`
//! installed by `DB::transaction`, which this port never installs) and
//! commits on its own, autonomously, the moment it runs. So a "Required"
//! transaction's `commit`/`rollback` outcome has never changed whether an
//! action's writes are durable; it changes nothing about the data at all.
//!
//! Holding a real `crate::database::Transaction` open for the action's
//! whole duration - as this port did before this fix - therefore bought
//! nothing, while pinning a pool connection for that entire span. That is
//! dangerous on a small pool: `RenderCache`'s write instrumentation
//! (`render_cache::orm::advance`) reacts to exactly those same writes by
//! opening its OWN transaction to advance a generation counter, and on a
//! single-connection pool (the default for this project's `TestDatabase`,
//! and a real possibility in production) that second transaction can only
//! ever get a connection once this one releases its - which does not
//! happen until the action finishes. That is a genuine circular wait, not
//! a slow path: nothing frees the one connection this transaction holds
//! until the very code that is blocked waiting for a connection returns.
//!
//! The fix keeps the fail-fast behavior of `begin` (a database that cannot
//! open a transaction right now still fails `TransactionBegin` immediately,
//! exactly as before) but releases the probe transaction before returning,
//! rather than holding it for the rest of the action. `commit`/`rollback`
//! become trivial - there is no real transaction left to finish - which is
//! an accurate reflection of what was already true: this handle never
//! gated any actual write.
//!
//! Deferring `RenderCache`'s advance until this handle's `commit`/`rollback`
//! resolves (the shape the payments hydration deadlock fix used - see
//! `payments::webhook_route::advance_touched_mirror_tables`) is NOT the
//! right fix here, and was rejected after tracing the actual call graph:
//! that payments fix defers correctly because the writes it defers around
//! really do sit inside the transaction whose commit or rollback decides
//! their fate. Here the row write has already committed, on its own,
//! before `commit`/`rollback` is ever called - deferring `RenderCache`'s
//! advance to align with an unrelated transaction's outcome would mean a
//! later, unconnected rollback (a response-sealing failure, say) silently
//! discarding the advance for a write that is permanently durable, which
//! is exactly the "serve stale forever" failure mode this whole subsystem
//! exists to prevent. Advancing immediately, when the write happens - the
//! existing, unmodified behavior of `render_cache::orm::advance` - is the
//! only choice that agrees with what is actually persisted.

use suprnova_live::component::LiveFuture;
use suprnova_live::execution::{HostError, HostErrorKind, HostTransaction, TransactionPort};

pub(crate) struct SuprnovaTransactionPort;

/// Marker returned by [`SuprnovaTransactionPort::begin`]. Carries no
/// database connection or transaction handle: see the module doc for why
/// holding one open for the action's whole duration is unnecessary and
/// unsafe on a small connection pool.
struct SuprnovaHostTransaction;

impl HostTransaction for SuprnovaHostTransaction {
    fn commit(self: Box<Self>) -> LiveFuture<'static, Result<(), HostError>> {
        Box::pin(async move { Ok(()) })
    }

    fn rollback(self: Box<Self>) -> LiveFuture<'static, Result<(), HostError>> {
        Box::pin(async move { Ok(()) })
    }
}

impl TransactionPort for SuprnovaTransactionPort {
    /// Probes that the database can open a transaction right now - so a
    /// database that is down still fails the action's `TransactionBegin`
    /// phase immediately, exactly as before this fix - then releases it
    /// before returning instead of holding it open. See the module doc.
    fn begin(&self) -> LiveFuture<'_, Result<Box<dyn HostTransaction>, HostError>> {
        Box::pin(async {
            let probe = crate::database::DB::begin_transaction()
                .await
                .map_err(|_| HostError::new(HostErrorKind::Begin))?;
            probe
                .rollback()
                .await
                .map_err(|_| HostError::new(HostErrorKind::Begin))?;
            Ok(Box::new(SuprnovaHostTransaction) as Box<dyn HostTransaction>)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use sea_orm_migration::{MigrationTrait, MigratorTrait};
    use suprnova_live::execution::TransactionPort as _;
    use suprnova_live::render_cache::generation::{DependencyIdentity, GenerationLedger as _};

    use super::SuprnovaTransactionPort;
    use crate::render_cache::ledger::SqlGenerationLedger;
    use crate::testing::TestDatabase;

    struct Migrator;

    #[async_trait::async_trait]
    impl MigratorTrait for Migrator {
        fn migrations() -> Vec<Box<dyn MigrationTrait>> {
            vec![Box::new(crate::render_cache::migration::Migration)]
        }
    }

    /// R76: on the project's own single-connection SQLite pool
    /// (`TestDatabase::fresh`), a `SuprnovaTransactionPort::begin()` that
    /// held its transaction open (the pre-fix behavior) starves
    /// `render_cache::orm::after_table_write`'s own dedicated-transaction
    /// fallback of the pool's only connection, because nothing releases
    /// that connection until the very call this test makes returns - a
    /// genuine circular wait, proven here rather than assumed: bounding the
    /// write in a timeout turns a silent hang into a failing assertion
    /// instead of a wedged test run.
    ///
    /// With this fix, `begin()` releases its probe transaction before
    /// returning, so the write proceeds immediately and the table's
    /// generation advances exactly once.
    #[tokio::test]
    async fn a_required_transaction_write_advances_once_and_does_not_hang_on_a_single_connection_pool()
     {
        let _db = TestDatabase::fresh::<Migrator>()
            .await
            .expect("render cache migration applies to a fresh single-connection sqlite pool");
        crate::render_cache::mark_installed();
        let table = DependencyIdentity::table("posts");
        let ledger = SqlGenerationLedger::new();
        assert_eq!(
            ledger
                .current(&[table.digest()])
                .await
                .expect("current")
                .get(&table),
            Some(0),
            "nothing has written yet"
        );

        let port = SuprnovaTransactionPort;
        let transaction = port.begin().await.expect("begin");

        let advanced = tokio::time::timeout(
            Duration::from_secs(5),
            crate::render_cache::orm::after_table_write("posts"),
        )
        .await
        .expect(
            "after_table_write hung: the Required-transaction handle must not hold the sole \
             pool connection open while write instrumentation opens its own transaction",
        );
        advanced.expect("advance succeeds");

        transaction.commit().await.expect("commit");

        assert_eq!(
            ledger
                .current(&[table.digest()])
                .await
                .expect("current")
                .get(&table),
            Some(1),
            "the write advanced the table's generation exactly once"
        );
    }

    /// A `begin()` that is never followed by any write - the shape a
    /// validation failure produces, since the action body never runs -
    /// must not itself advance anything: `begin`'s probe transaction issues
    /// no statement other than its own rollback, so it must not trip
    /// `after_unknown_write`'s broad-authority advance or any other one.
    #[tokio::test]
    async fn a_begin_with_no_write_and_then_rollback_advances_nothing() {
        let _db = TestDatabase::fresh::<Migrator>()
            .await
            .expect("render cache migration applies");
        crate::render_cache::mark_installed();
        let broad = DependencyIdentity::broad();
        let ledger = SqlGenerationLedger::new();

        let port = SuprnovaTransactionPort;
        let transaction = port.begin().await.expect("begin");
        transaction.rollback().await.expect("rollback");

        assert_eq!(
            ledger
                .current(&[broad.digest()])
                .await
                .expect("current")
                .get(&broad),
            Some(0),
            "an empty probe transaction must not advance the broad authority"
        );
    }
}
