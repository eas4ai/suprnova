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
//! # The probe must not go through `DB::begin_transaction` (fix round 2, item 1)
//!
//! The first version of this fix implemented the probe as
//! `DB::begin_transaction()` followed by `Transaction::rollback()`. Both of
//! those emit a framework transaction event - `TransactionBeginning` and
//! `TransactionRolledBack` - as an unconditional side effect of the call.
//! Since this probe now runs, and releases, on every `begin`, every action
//! with a Required transaction policy fired that pair, and never a
//! `TransactionCommitted`, regardless of whether the action succeeded. An
//! application listening on those events for audit or metrics logging would
//! see a rollback recorded for every Live action, including ones that
//! completed normally - the events asserted something that did not happen.
//!
//! The fix below probes through `DB::connection()` and sea_orm's own
//! `TransactionTrait` directly, never through `DB::begin_transaction` or
//! `crate::database::Transaction`. That still opens and rolls back a real
//! database transaction - so a database that cannot begin one still fails
//! `TransactionBegin` immediately, exactly as before - but it does not pass
//! through the framework's event-emitting wrapper, so it emits nothing.
//!
//! # What a Required policy does and does not guarantee today
//!
//! Because `begin` only probes and releases, and `commit`/`rollback` on the
//! returned handle do nothing, a "Required" transaction policy on a Live
//! action does not currently give that action atomicity: nothing rolls back
//! the action's own writes if a later step in the same action fails. Each
//! ORM write the action makes still commits on its own, autonomously, the
//! moment it runs, exactly as it would with no transaction policy at all.
//! This was already true before this fix (see the ruling above); this fix
//! only stops that fact from also corrupting the transaction event stream.
//!
//! The real fix, for whoever picks this up, is for `begin` to open a real
//! transaction and install it as the ambient `CURRENT_TX` task-local (the
//! same mechanism `DB::transaction` uses), so that the ordinary
//! `DB::connection()` path an action's writes already go through joins that
//! transaction instead of resolving a fresh, autocommitting connection. That
//! is a bigger change - it reintroduces the single-connection-pool deadlock
//! risk this module's history describes unless it is paired with a way to
//! keep `RenderCache`'s write instrumentation from needing a second
//! connection while the action's transaction is open - so it is left as a
//! documented gap rather than attempted here.
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
    ///
    /// This probes through sea_orm's own `TransactionTrait` on the raw
    /// connection, not through `DB::begin_transaction`/`Transaction`: those
    /// emit `TransactionBeginning`/`TransactionRolledBack` framework events
    /// as a side effect, which would misrepresent every successful action as
    /// one that rolled back. See "The probe must not go through
    /// `DB::begin_transaction`" above.
    fn begin(&self) -> LiveFuture<'_, Result<Box<dyn HostTransaction>, HostError>> {
        Box::pin(async {
            use sea_orm::TransactionTrait as _;

            let conn = crate::database::DB::connection()
                .map_err(|_| HostError::new(HostErrorKind::Begin))?;
            let probe = conn
                .inner()
                .begin()
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
    use std::sync::Arc;
    use std::time::Duration;

    use sea_orm_migration::{MigrationTrait, MigratorTrait};
    use suprnova_live::execution::TransactionPort as _;
    use suprnova_live::render_cache::generation::{DependencyIdentity, GenerationLedger as _};

    use super::SuprnovaTransactionPort;
    use crate::database::events::{
        TransactionBeginning, TransactionCommitted, TransactionRolledBack,
    };
    use crate::events::Listener;
    use crate::events::testing::assert_not_dispatched;
    use crate::render_cache::ledger::SqlGenerationLedger;
    use crate::testing::TestDatabase;
    use crate::{EventFacade, FrameworkError};

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

    /// A no-op listener whose only job is to make `EventFacade::has_listeners`
    /// return true for the transaction lifecycle events, so `emit_tx_event`'s
    /// own no-listeners short-circuit does not skip the dispatch before the
    /// fake ever sees it. `EventFacade::fake()` intercepts the dispatch this
    /// listener would otherwise receive and records it instead.
    struct NoOpListener;

    #[async_trait::async_trait]
    impl Listener<TransactionBeginning> for NoOpListener {
        async fn handle(&self, _event: &TransactionBeginning) -> Result<(), FrameworkError> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Listener<TransactionCommitted> for NoOpListener {
        async fn handle(&self, _event: &TransactionCommitted) -> Result<(), FrameworkError> {
            Ok(())
        }
    }

    #[async_trait::async_trait]
    impl Listener<TransactionRolledBack> for NoOpListener {
        async fn handle(&self, _event: &TransactionRolledBack) -> Result<(), FrameworkError> {
            Ok(())
        }
    }

    /// Fix round 2, item 1, proven to discriminate: before the fix, `begin`
    /// probed through `DB::begin_transaction()` and `Transaction::rollback()`,
    /// which fire `TransactionBeginning` and `TransactionRolledBack`
    /// unconditionally, so this test failed against the pre-fix code (both
    /// events were recorded, and `TransactionCommitted` was never reachable at
    /// all since the returned handle's own `commit` never touched the
    /// database). Confirmed by temporarily restoring the old
    /// `DB::begin_transaction`/`probe.rollback()` body and re-running: the
    /// `assert_not_dispatched` calls below failed with the events present, as
    /// expected. With the fix, `begin` probes through sea_orm's
    /// `TransactionTrait` directly, which never touches
    /// `crate::database::events`, so a full begin-then-commit lifecycle - the
    /// shape a successful Required-transaction action takes - emits none of
    /// the three transaction lifecycle events.
    #[tokio::test]
    async fn begin_then_commit_emits_no_transaction_lifecycle_events() {
        let _db = TestDatabase::fresh::<Migrator>()
            .await
            .expect("render cache migration applies");
        let _fake = EventFacade::fake();
        EventFacade::listen::<TransactionBeginning, NoOpListener>(Arc::new(NoOpListener)).await;
        EventFacade::listen::<TransactionCommitted, NoOpListener>(Arc::new(NoOpListener)).await;
        EventFacade::listen::<TransactionRolledBack, NoOpListener>(Arc::new(NoOpListener)).await;

        let port = SuprnovaTransactionPort;
        let handle = port.begin().await.expect("begin");
        handle.commit().await.expect("commit");

        assert_not_dispatched::<TransactionBeginning>(|_| true);
        assert_not_dispatched::<TransactionCommitted>(|_| true);
        assert_not_dispatched::<TransactionRolledBack>(|_| true);

        EventFacade::forget::<TransactionBeginning>();
        EventFacade::forget::<TransactionCommitted>();
        EventFacade::forget::<TransactionRolledBack>();
    }
}
