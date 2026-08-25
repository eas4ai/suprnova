//! Transaction-scoped after-commit and rollback callbacks.
//!
//! Laravel's `DatabaseTransactionsManager`, minus everything Suprnova's
//! no-nesting rule makes unreachable. There are no parent chains here and
//! nothing to partition per connection: the registry is two `Vec`s hanging off
//! the current transaction's `TxState`, drained exactly once by
//! [`DB::transaction`](crate::DB::transaction)'s commit or rollback path.
//!
//! Nesting does reach this module in one shape. Laravel's
//! `DatabaseTransactionsManager::rollback($connection, $level)` discards every
//! callback staged above the level being unwound, and Suprnova's nested block is
//! a savepoint, so [`Transaction::rollback_to`](crate::Transaction::rollback_to)
//! is the same event: the rows a deferred dispatch was waiting on are gone, so
//! the dispatch has to go with them. [`SavepointMark`] records where both `Vec`s
//! stood when the savepoint was issued, and [`rollback_to_savepoint`] drops the
//! after-commit callbacks above that mark while running the rollback callbacks
//! that compensate for them. A savepoint that is never rolled back keeps
//! everything registered inside it.
//!
//! Manual transactions ([`DB::begin_transaction`](crate::DB::begin_transaction))
//! deliberately do not participate. They install no `CURRENT_TX`, so there is no
//! drain point, and a callback registered against them would never run - a
//! deferred dispatch that silently disappears is worse than one that happens
//! too early, so a push from inside a manual transaction happens immediately.

use crate::error::FrameworkError;
use futures::future::BoxFuture;

/// A deferred action to run after the surrounding transaction commits (or, for
/// the rollback registry, after it rolls back).
///
/// `FnOnce` rather than `Fn`: the queue's deferred push moves the job value
/// itself into the callback, and a job is not required to be `Clone`.
pub(crate) type AfterCommitCallback =
    Box<dyn FnOnce() -> BoxFuture<'static, Result<(), FrameworkError>> + Send>;

/// True when the calling task is inside a
/// [`DB::transaction`](crate::DB::transaction) closure.
///
/// `try_with` rather than `with`: outside any scope the task-local is not set
/// at all, which is the common case for every non-transactional code path.
pub(crate) fn in_transaction() -> bool {
    super::transaction::CURRENT_TX
        .try_with(|t| t.is_some())
        .unwrap_or(false)
}

/// Register `cb` to run after the current transaction commits.
///
/// With no open transaction the callback runs immediately, on this task, and
/// its error propagates - Laravel's `addCallback` rule. That immediate
/// execution is what lets a caller opt a job into after-commit dispatch without
/// also having to know whether the code path it sits on is transactional: the
/// same call is correct in both.
pub(crate) async fn register_callback(cb: AfterCommitCallback) -> Result<(), FrameworkError> {
    match queue_on_current_transaction(cb, Registry::AfterCommit) {
        // Queued against the open transaction; the drain runs it.
        None => Ok(()),
        // No open transaction - Laravel's immediate-execution rule.
        Some(cb) => cb().await,
    }
}

/// Register `cb` to run after the current transaction rolls back.
///
/// With no open transaction this drops `cb` and reports success: Laravel's
/// `addCallbackForRollback` has no immediate fallback, and running a
/// compensating action when nothing was ever deferred would undo work that did
/// happen.
pub(crate) async fn register_rollback_callback(
    cb: AfterCommitCallback,
) -> Result<(), FrameworkError> {
    // The returned callback (if any) is deliberately dropped: nothing rolled
    // back, so there is nothing to compensate for.
    let _ = queue_on_current_transaction(cb, Registry::Rollback);
    Ok(())
}

/// Which of the two per-transaction registries a callback belongs to.
#[derive(Clone, Copy)]
enum Registry {
    AfterCommit,
    Rollback,
}

/// Push `cb` onto `registry` of the ambient transaction.
///
/// Returns `None` when the callback was queued, and hands `cb` back when there
/// is no ambient transaction, so each caller can apply its own no-transaction
/// rule. Threading the callback through an `Option` is what makes that possible
/// at all: `task_local::try_with` hands out a `&Option<Arc<TxState>>`, so the
/// closure cannot both move `cb` into the vec and return it.
fn queue_on_current_transaction(
    cb: AfterCommitCallback,
    registry: Registry,
) -> Option<AfterCommitCallback> {
    let mut slot = Some(cb);
    let _ = super::transaction::CURRENT_TX.try_with(|t| {
        if let Some(state) = t.as_ref() {
            let vec = match registry {
                Registry::AfterCommit => &state.after_commit,
                Registry::Rollback => &state.on_rollback,
            };
            // Recover in place on a poisoned lock. The critical section is a
            // single `Vec::push` with no user code in it, so poisoning is
            // unreachable in practice; if it ever happened, the registry is
            // still structurally intact and dropping the callback (or running
            // it early, inside the very transaction it is meant to outlive)
            // would both be worse than using it.
            let mut guard = vec.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(cb) = slot.take() {
                guard.push(cb);
            }
        }
    });
    slot
}

/// Where both callback registries stood when a `SAVEPOINT` was issued.
///
/// Laravel keys the same bookkeeping by integer nesting level, because its
/// manager owns the counter. Suprnova's caller names the point instead
/// ([`Transaction::savepoint`](crate::Transaction::savepoint)), and SQL lets a
/// name repeat - SQLite, Postgres and MySQL all resolve
/// `ROLLBACK TO SAVEPOINT x` to the most recent `x` and destroy every savepoint
/// established after it. So the marks are a stack that carries the name and is
/// searched from the top: whichever savepoint the database rolls back to is the
/// one whose mark is used, and the registry can never disagree with the rows.
pub(crate) struct SavepointMark {
    /// The name the caller passed to `savepoint`, already validated as a SQL
    /// identifier by the time it reaches here.
    name: String,
    /// Length of the after-commit registry at the savepoint.
    after_commit_len: usize,
    /// Length of the rollback registry at the savepoint.
    on_rollback_len: usize,
}

/// Record where both registries stand, so a later `rollback_to` on `name` can
/// tell what was registered above this point.
///
/// Called only after the `SAVEPOINT` statement itself succeeded: a mark for a
/// savepoint the database never established would discard callbacks the rows
/// still back.
pub(crate) fn mark_savepoint(state: &super::transaction::TxState, name: &str) {
    let after_commit_len = registry_len(&state.after_commit);
    let on_rollback_len = registry_len(&state.on_rollback);
    let mut marks = state.savepoints.lock().unwrap_or_else(|e| e.into_inner());
    marks.push(SavepointMark {
        name: name.to_owned(),
        after_commit_len,
        on_rollback_len,
    });
}

/// Unwind both registries to `name`'s mark and hand back the rollback callbacks
/// that compensate for what was discarded.
///
/// Returns `None` when no mark carries `name`, which means the savepoint was
/// established out of band (raw `SAVEPOINT` SQL rather than
/// [`Transaction::savepoint`](crate::Transaction::savepoint)). Discarding
/// callbacks on a guess would be worse than leaving them: the caller warns and
/// leaves the registries alone.
///
/// The mark itself stays on the stack, because every backend keeps the named
/// savepoint usable after a `ROLLBACK TO` - the same name can be rolled back to
/// again, and the recorded lengths are still the right floor. Marks above it go,
/// because the database destroyed those savepoints.
pub(crate) fn rollback_to_savepoint(
    state: &super::transaction::TxState,
    name: &str,
) -> Option<Vec<AfterCommitCallback>> {
    let (after_commit_len, on_rollback_len) = {
        let mut marks = state.savepoints.lock().unwrap_or_else(|e| e.into_inner());
        // From the top: a repeated name means the innermost savepoint shadows
        // the outer one, which is exactly how the database resolves it.
        let idx = marks.iter().rposition(|m| m.name == name)?;
        let mark = (marks[idx].after_commit_len, marks[idx].on_rollback_len);
        marks.truncate(idx + 1);
        mark
    };

    // Both guards are taken and released without an `.await` between them: the
    // compensations run in the caller, after every lock is back.
    {
        let mut after_commit = state.after_commit.lock().unwrap_or_else(|e| e.into_inner());
        // `truncate` is a no-op when the list is already shorter, which is the
        // case for a `rollback_to` that follows another one on the same name.
        after_commit.truncate(after_commit_len);
    }
    let mut on_rollback = state.on_rollback.lock().unwrap_or_else(|e| e.into_inner());
    if on_rollback_len >= on_rollback.len() {
        return Some(Vec::new());
    }
    Some(on_rollback.split_off(on_rollback_len))
}

/// Current length of one registry.
///
/// Split out so the two reads in [`mark_savepoint`] share one poisoning policy
/// with the rest of the module: recover in place, because the critical section
/// holds no user code and dropping a registry would be worse than reusing it.
fn registry_len(vec: &std::sync::Mutex<Vec<AfterCommitCallback>>) -> usize {
    vec.lock().unwrap_or_else(|e| e.into_inner()).len()
}

/// Take both registries off `state`, leaving them empty.
///
/// Drained as a pair, before the physical commit or rollback, because
/// `TxState` holds an `Arc<DatabaseTransaction>`: `DB::transaction` has to drop
/// its `TxState` clone before `Arc::try_unwrap` can reach the transaction to
/// commit it. The callbacks then run after that, outside the `CURRENT_TX`
/// scope, so a callback that dispatches its own work sees no ambient
/// transaction and acts immediately rather than deferring into a registry
/// nothing will drain again.
pub(crate) fn drain(
    state: &super::transaction::TxState,
) -> (Vec<AfterCommitCallback>, Vec<AfterCommitCallback>) {
    let take = |vec: &std::sync::Mutex<Vec<AfterCommitCallback>>| {
        let mut guard = vec.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *guard)
    };
    (take(&state.after_commit), take(&state.on_rollback))
}

/// Run every after-commit callback in registration order.
///
/// The transaction is already committed and cannot be taken back, so one
/// failing callback must not skip the rest: each error is logged, the first is
/// remembered, and the whole list still runs. The first error is returned so
/// the caller learns that a deferred effect was lost, with wording that says
/// the durable half already happened.
pub(crate) async fn run_after_commit(
    callbacks: Vec<AfterCommitCallback>,
) -> Result<(), FrameworkError> {
    let mut first_err: Option<FrameworkError> = None;
    for cb in callbacks {
        if let Err(e) = cb().await {
            tracing::error!(
                target: "suprnova::database",
                error = %e,
                "after-commit callback failed (the transaction itself committed)",
            );
            if first_err.is_none() {
                first_err = Some(e);
            }
        }
    }
    match first_err {
        Some(e) => Err(FrameworkError::internal(format!(
            "after-commit callback failed (the transaction itself committed): {e}"
        ))),
        None => Ok(()),
    }
}

/// Compensate for a transaction that did not commit: discard the after-commit
/// list, then run the rollback list.
///
/// Every path that reaches a transaction's end without a durable commit funnels
/// through here - the closure returning `Err`, a leaked `TxHandle` that blocks
/// the commit, and a COMMIT the database refuses alike. They are the same event
/// as far as a deferred dispatch is concerned: it did not happen, so a
/// uniqueness lock taken on its behalf has to go back rather than block
/// re-dispatch for the rest of its TTL.
pub(crate) async fn compensate(
    after_commit: Vec<AfterCommitCallback>,
    on_rollback: Vec<AfterCommitCallback>,
) {
    // Explicit rather than letting the binding fall out of scope: dropping the
    // after-commit list is the decision, not a side effect of the borrow ending.
    drop(after_commit);
    run_rollback(on_rollback).await;
}

/// Run every rollback callback in registration order, log-and-continue.
///
/// Errors are never returned: the caller is on its way to surfacing the
/// original transaction error, and replacing that with "the compensating
/// action failed" would hide the reason the transaction rolled back in the
/// first place.
pub(crate) async fn run_rollback(callbacks: Vec<AfterCommitCallback>) {
    for cb in callbacks {
        if let Err(e) = cb().await {
            tracing::error!(
                target: "suprnova::database",
                error = %e,
                "rollback callback failed; the original transaction error is still \
                 surfaced to the caller",
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn no_ambient_transaction_reports_false() {
        assert!(!in_transaction());
        // Explicitly-empty scope, which is what a task-local reset would look
        // like, must read the same as no scope at all.
        super::super::transaction::CURRENT_TX
            .scope(None, async { assert!(!in_transaction()) })
            .await;
    }

    #[tokio::test]
    async fn register_callback_runs_immediately_without_a_transaction() {
        let ran = Arc::new(AtomicUsize::new(0));
        let seen = ran.clone();
        register_callback(Box::new(move || {
            Box::pin(async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }))
        .await
        .expect("no transaction means run now");
        assert_eq!(ran.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn register_callback_propagates_an_immediate_error() {
        let err = register_callback(Box::new(|| {
            Box::pin(async { Err(FrameworkError::internal("boom")) })
        }))
        .await
        .expect_err("the immediate path propagates");
        assert!(err.to_string().contains("boom"));
    }

    #[tokio::test]
    async fn register_rollback_callback_is_a_no_op_without_a_transaction() {
        let ran = Arc::new(AtomicUsize::new(0));
        let seen = ran.clone();
        register_rollback_callback(Box::new(move || {
            Box::pin(async move {
                seen.fetch_add(1, Ordering::SeqCst);
                Ok(())
            })
        }))
        .await
        .expect("silently dropped, reported Ok");
        assert_eq!(
            ran.load(Ordering::SeqCst),
            0,
            "nothing rolled back, so there is nothing to compensate for"
        );
    }

    #[tokio::test]
    async fn run_after_commit_runs_every_callback_and_returns_the_first_error() {
        let order = Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
        let mk = |n: u8, fail: bool| {
            let order = order.clone();
            Box::new(move || {
                let order = order.clone();
                Box::pin(async move {
                    order.lock().unwrap().push(n);
                    if fail {
                        Err(FrameworkError::internal(format!("cb {n}")))
                    } else {
                        Ok(())
                    }
                }) as BoxFuture<'static, Result<(), FrameworkError>>
            }) as AfterCommitCallback
        };

        let err = run_after_commit(vec![mk(1, false), mk(2, true), mk(3, true), mk(4, false)])
            .await
            .expect_err("a failing callback surfaces");
        assert!(err.to_string().contains("cb 2"), "first error wins: {err}");
        assert!(
            err.to_string()
                .contains("after-commit callback failed (the transaction itself committed)"),
            "the message must say the transaction committed: {err}"
        );
        assert_eq!(
            *order.lock().unwrap(),
            vec![1, 2, 3, 4],
            "every callback runs in registration order even after one fails"
        );
    }
}
