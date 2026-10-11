# Measurement brief

## The draft
{
  "question": "How does PAR-007 apply to a transaction started with DB::begin_transaction, which nothing ambient can see?",
  "recommendation": "Handle-bound: tx.after_commit and a with_tx job dispatch defer to the handle's commit; DB::after_commit follows the ambient DB::transaction",
  "because": "DB::begin_transaction returns a handle and sets no ambient state: ambient writes never join it, only _with_tx calls do, so ambient code is outside that transaction; a per-connection or per-task lookup would mix concurrent requests (one pool) or fail in root futures where tokio try_id is None",
  "if_wrong": "A port that calls DB::after_commit between DB::begin_transaction and commit sees its callback run at once and must switch to tx.after_commit",
  "instead": "Escalate to reword PAR-007 so it names only the ambient DB::transaction"
}

## The recommended option (draft.options[0])
Handle-bound: tx.after_commit and a with_tx job dispatch defer to the handle's commit; DB::after_commit follows the ambient DB::transaction

Touched files:

framework/src/database/after_commit.rs:
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
use std::future::Future;
use tokio::task::JoinHandle;

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
    /// Validated, backend-canonical SQL identity, shared with the emitted
    /// savepoint statement rather than the caller's original spelling.
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

/// Run every after-commit callback in registration order, consuming the
/// list front to back.
///
/// The transaction is already committed and cannot be taken back, so one
/// failing callback must not skip the rest: each error is logged, the first is
/// remembered, and the whole list still runs. The first error is returned so
/// the caller learns that a deferred effect was lost, with wording that says
/// the durable half already happened.
///
/// The owning runner awaits each callback before starting the next, even
/// when its caller stops waiting for the result.
async fn run_after_commit_list(
    callbacks: &mut Vec<AfterCommitCallback>,
) -> Result<(), FrameworkError> {
    let mut first_err: Option<FrameworkError> = None;
    while !callbacks.is_empty() {
        let cb = callbacks.remove(0);
        if let Err(e) = run_one_callback(cb).await {
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

/// Run one callback to completion and report its outcome.
///
/// The callback executes as an awaited child task rather than inline: an
/// abort of the awaiting task cannot recall work already handed to the
/// runtime, so a callback in flight when cancellation lands still runs to
/// completion instead of being dropped mid-effect. Order is unchanged -
/// callers await each child before starting the next. A panicking
/// callback surfaces as an error (and no longer skips the callbacks
/// behind it); a child cancelled only by runtime shutdown is logged as
/// lost work.
async fn run_one_callback(cb: AfterCommitCallback) -> Result<(), FrameworkError> {
    let child = spawn_owned(async move { cb().await });
    match child.await {
        Ok(outcome) => outcome,
        Err(join_err) => Err(FrameworkError::internal(format!(
            "transaction callback task ended without a result: {join_err}"
        ))),
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
    // Savepoint rollback also enters here after slicing callbacks out of
    // the registry. Its caller can be cancelled, so own the whole list
    // before awaiting the first callback.
    if let Err(error) = spawn_owned(async move {
        let mut callbacks = callbacks;
        run_rollback_list(&mut callbacks).await;
    })
    .await
    {
        tracing::error!(
            target: "suprnova::database",
            error = %error,
            "rollback callback runner ended without completing compensation",
        );
    }
}

/// [`run_rollback`] over a borrowed list, consuming it front to back.
///
/// The owning runner retains the entire sequence across caller cancellation.
async fn run_rollback_list(callbacks: &mut Vec<AfterCommitCallback>) {
    while !callbacks.is_empty() {
        let cb = callbacks.remove(0);
        if let Err(e) = run_one_callback(cb).await {
            tracing::error!(
                target: "suprnova::database",
                error = %e,
                "rollback callback failed; the original transaction error is still \
                 surfaced to the caller",
            );
        }
    }
}

/// A callback runner that survives its own task's cancellation.
///
/// Before awaiting anything, transfers the entire list to one owned task.
/// That task keeps the in-flight callback and the remainder together, so
/// cancellation cannot start the remainder ahead of the current callback.
/// Dropping a guard before starting it also transfers its list to a runner.
/// Task-local container bindings follow every spawn; thread-local bindings
/// do not (production resolves from the global container).
pub(crate) struct GuardedCallbacks {
    after_commit: Vec<AfterCommitCallback>,
    on_rollback: Vec<AfterCommitCallback>,
    /// `true`: the transaction committed, so a diverted remainder must
    /// still RUN the after-commit list. `false`: no commit, so divert
    /// compensates (drop after-commit, run rollback).
    committed: bool,
    complete: bool,
}

impl GuardedCallbacks {
    /// Runner for the post-commit path: the remainder keeps after-commit
    /// semantics when diverted.
    pub(crate) fn after_commit(callbacks: Vec<AfterCommitCallback>) -> Self {
        Self {
            after_commit: callbacks,
            on_rollback: Vec::new(),
            committed: true,
            complete: false,
        }
    }

    /// Runner for the no-commit paths: the remainder compensates when
    /// diverted.
    pub(crate) fn compensating(
        after_commit: Vec<AfterCommitCallback>,
        on_rollback: Vec<AfterCommitCallback>,
    ) -> Self {
        Self {
            after_commit,
            on_rollback,
            committed: false,
            complete: false,
        }
    }

    /// Transfer the whole sequence before waiting for its result.
    pub(crate) async fn run_after_commit(mut self) -> Result<(), FrameworkError> {
        let mut callbacks = std::mem::take(&mut self.after_commit);
        self.complete = true;
        spawn_owned(async move { run_after_commit_list(&mut callbacks).await })
            .await
            .map_err(|error| FrameworkError::internal(format!(
                "after-commit runner ended without a result (the transaction itself committed): {error}"
            )))?
    }

    /// Transfer all compensations to one serial runner before waiting.
    pub(crate) async fn compensate(mut self) {
        let after_commit = std::mem::take(&mut self.after_commit);
        let on_rollback = std::mem::take(&mut self.on_rollback);
        self.complete = true;
        compensate(after_commit, on_rollback).await;
    }
}

impl Drop for GuardedCallbacks {
    fn drop(&mut self) {
        if self.complete {
            return;
        }
        if self.after_commit.is_empty() && self.on_rollback.is_empty() {
            return;
        }
        let after_commit = std::mem::take(&mut self.after_commit);
        let on_rollback = std::mem::take(&mut self.on_rollback);
        let committed = self.committed;
        spawn_detached(async move {
            if committed {
                let mut rest = after_commit;
                // Errors are already logged inside; nothing to surface to.
                let _ = run_after_commit_list(&mut rest).await;
            } else {
                compensate(after_commit, on_rollback).await;
            }
        });
    }
}

/// Run `future` on a detached task that outlives the spawning task's
/// cancellation.
///
/// Used from drop guards, including during runtime shutdown, when failure
/// to find a runtime must be logged rather than panicking.
pub(crate) fn spawn_detached(future: impl Future<Output = ()> + Send + 'static) {
    if tokio::runtime::Handle::try_current().is_ok() {
        spawn_owned(future);
    } else {
        tracing::error!(
            target: "suprnova::database",
            "transaction callbacks diverted after task cancellation, but no \
             async runtime is running to complete them; deferred work is lost",
        );
    }
}

/// Own finalization work independently of its caller while retaining the
/// task-local container used by database listeners and callback services,
/// and the caller's container scope.
pub(crate) fn spawn_owned<T: Send + 'static>(
    future: impl Future<Output = T> + Send + 'static,
) -> JoinHandle<T> {
    let task_container = crate::container::TASK_CONTAINER
        .try_with(|c| c.clone())
        .ok();
    // The callbacks belong to the unit of work that registered them, so they
    // carry its container scope: they resolve the scoped values it resolved,
    // and the scope ends when the last of the two ends.
    let future = crate::container::App::in_current_scope(future);
    tokio::spawn(async move {
        match task_container {
            Some(container) => {
                crate::container::TASK_CONTAINER
                    .scope(container, future)
                    .await
            }
            None => future.await,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;
    use tokio::sync::Notify;
    use tokio::time::timeout;

    #[tokio::test]
    async fn bare_rollback_runner_keeps_sliced_savepoint_callbacks_after_abort() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let completed = Arc::new(AtomicUsize::new(0));
        let first: AfterCommitCallback = {
            let entered = entered.clone();
            let release = release.clone();
            let completed = completed.clone();
            Box::new(move || {
                Box::pin(async move {
                    entered.notify_one();
                    release.notified().await;
                    assert_eq!(completed.fetch_add(1, Ordering::SeqCst), 0);
                    Ok(())
                })
            })
        };
        let second: AfterCommitCallback = {
            let completed = completed.clone();
            Box::new(move || {
                Box::pin(async move {
                    assert_eq!(completed.fetch_add(1, Ordering::SeqCst), 1);
                    Ok(())
                })
            })
        };
        // rollback_to has already removed this list from the transaction
        // registry, so the scope finalizer cannot recover it on cancellation.
        let caller = tokio::spawn(run_rollback(vec![first, second]));
        timeout(Duration::from_secs(2), entered.notified())
            .await
            .expect("first callback entered");
        caller.abort();
        assert!(caller.await.expect_err("caller aborted").is_cancelled());
        release.notify_one();
        timeout(Duration::from_secs(2), async {
            while completed.load(Ordering::SeqCst) != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("all sliced savepoint callbacks must complete in order");
    }

    #[tokio::test]
    async fn guarded_compensation_preserves_order_after_caller_abort() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let finished = Arc::new(Notify::new());
        let order = Arc::new(Mutex::new(Vec::new()));
        let first: AfterCommitCallback = {
            let entered = entered.clone();
            let release = release.clone();
            let order = order.clone();
            Box::new(move || {
                Box::pin(async move {
                    entered.notify_one();
                    release.notified().await;
                    order.lock().unwrap().push(1);
                    Ok(())
                })
            })
        };
        let second: AfterCommitCallback = {
            let finished = finished.clone();
            let order = order.clone();
            Box::new(move || {
                Box::pin(async move {
                    order.lock().unwrap().push(2);
                    finished.notify_one();
                    Ok(())
                })
            })
        };
        let caller = tokio::spawn(async move {
            GuardedCallbacks::compensating(Vec::new(), vec![first, second])
                .compensate()
                .await;
        });
        timeout(Duration::from_secs(2), entered.notified())
            .await
            .expect("first callback entered");
        caller.abort();
        assert!(caller.await.expect_err("caller aborted").is_cancelled());
        let second_started = timeout(Duration::from_millis(100), finished.notified())
            .await
            .is_ok();
        release.notify_one();
        timeout(Duration::from_secs(2), async {
            while order.lock().unwrap().len() != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("both callbacks complete");
        assert!(
            !second_started,
            "second compensation started before first completed"
        );
        assert_eq!(*order.lock().unwrap(), [1, 2]);
    }

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

        let err = GuardedCallbacks::after_commit(vec![
            mk(1, false),
            mk(2, true),
            mk(3, true),
            mk(4, false),
        ])
        .run_after_commit()
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



## All options (draft.options, in order)
- [0] (recommended) Handle-bound: tx.after_commit and a with_tx job dispatch defer to the handle's commit; DB::after_commit follows the ambient DB::transaction
- [1] Escalate to reword PAR-007 so it names only the ambient DB::transaction

## Contract
{
  "keystone": "# Suprnova - keystone\n\nStatus: Observed\n\nThis file says what Suprnova is and where each part is specified. It holds\nno requirements of its own. Each domain spec owns its prefix. Text in this\nspec set marked Observed describes what the code does at the cited\nrevision; only text the developer has confirmed as Agreed is contract.\n\nWritten 2026-09-13 at framework main `31fb0ead` from the code, by an agent\nadopting the repository under Cairn. The developer corrects it.\n\n## What Suprnova is\n\nSuprnova is a Laravel-inspired full-stack web framework for Rust: hyper and\nTokio for HTTP, SeaORM for the database, Inertia 3 for the SPA-style\nfrontends, with Laravel-shaped facades (`Auth::login`, `Cache::remember`,\n`Event::dispatch`, `#[model]`, `routes!`) on top. This repository is the\nframework, not an application: users scaffold their own applications with\n`suprnova new`, and everything here is library, macro, CLI, or the internal\ndogfood application.\n\nSuprnova Live is the framework's server-driven interactive layer: a\ncomponent is a Rust struct with a typed state, actions, and an Askama view;\nthe server renders every document completely, mounts components as islands\ninside it, and updates each island by morphing its markup after an action.\nNavigation is real routing over plain HTTP; no client application owns the\npage (Live overview, `crates/suprnova-live/docs/specs/suprnova-live/00-overview.md`).\n\n## What Suprnova is not\n\n- Not a crates.io package. Distribution is a git tag per release; the\n  `v<version>` tag is the release. This absorbs API churn without downstream\n  SemVer bumps (`README.md`, Distribution model).\n- Not an application. The `app/` crate exists to exercise every subsystem\n  together and to host end-to-end tests.\n- Not a single-page application framework. Live's dynamism is island-scoped\n  inside a server-rendered document; Inertia frontends are a separate,\n  optional path.\n\n## Components and how they communicate\n\n| Component | Owns | Talks to |\n|---|---|---|\n| `framework/` (the `suprnova` crate) | ~60 subsystem modules under `framework/src/`; the crate-root re-exports in `framework/src/lib.rs` are the public surface | Applications call it; it drives the engine crates below |\n| `suprnova-macros/` | Every proc macro (`#[handler]`, `#[model]`, `routes!`, `#[command]`, `InertiaProps`, `#[workflow]`, `LiveComponent`) | Emits `inventory::submit!` registrations the framework collects at link time |\n| `suprnova-cli/` | The `suprnova` binary: `new`, `serve`, `make:*`, `migrate*`, `generate-types`, `live:check`, `live:make`, `live:inspect`, `live:assets`; scaffold templates under `suprnova-cli/src/templates/files/` | Generates applications that depend on the framework by git tag |\n| `crates/suprnova-live/` | The Live engine: actions, components, endpoint admission, mounts, snapshots, protocol, uploads, async updates, render cache, view contracts; a TypeScript browser runtime with tracked `dist/`; its own gate, numbered spec set, and budgets | The framework's `framework/src/live/` facade adapts it to Suprnova's request model |\n| `crates/suprnova-magnetar/` | Auth-service foundations (primary auth, factor gates, passkeys, OAuth, abuse prevention) | Applications reach it only through the framework API (`init_magnetar`, the `magnetar-oauth` feature) |\n| `crates/suprnova-payments-{stripe,paddle,nowpayments}`, `crates/suprnova-web-push/` | Adapter crates behind framework traits | Same git-tag distribution |\n| `app/` | The dogfood application: every subsystem wired, seven Live views under `app/templates/live/`, end-to-end tests under `app/tests/` | Runs against the in-tree framework |\n| `manual/` | The user manual, 112 markdown files, mirrored 1:1 into six locales under a blob-hash ledger (`.manual-translations.lock`) | Published surface; the translation ledger blocks a release on a stale mirror |\n\nCompile-time registration through the `inventory` crate is the backbone of\n\"declare it anywhere\": models, relations, observers, policies, commands,\nservices, supervisors, workflows, payments, and data registries each have an\n`inventory::collect!` point in `framework/src/` (for example\n`framework/src/eloquent/registry.rs`, `framework/src/console/mod.rs`,\n`framework/src/container/provider.rs`). Anything with a plausible\nalternative backend is a trait with drivers behind it (cache, queue,\nfilesystem, mail, vector, payments, broadcasting, rate limiting, database);\na new backend is a new driver, never a fork of the surface\n(`manual/introduction.md`, design principle 3).\n\n## Verification\n\nThe gate is the CI: `scripts/gate.sh` runs `scripts/gate-runner.py` over a\ndefault tier (format, prose-dash and doc-reference greps, Live document\ncontracts, clippy, the rustdoc link check, workspace tests, database steps\nagainst standing services, scaffold compile checks, Live browser artifact\nparity) and a full tier that adds the feature matrix, MSRV, audit, manual\ntranslation currency, release smoke, and the whole Live gate. The pre-push\nhook refuses a tip without a green stamp. The scripts are local tooling in\nevery clone and are not published; the reader of the public tree does not\nhave them, and the relevant rule is always stated inline in this spec set\nrather than cited to them.\n\nSuprnova Live keeps its own gate at `crates/suprnova-live/scripts/gate.sh`\n(22 phases: document contracts, spec structure, license inventory, Rust\nlints and boundaries, MSRV, fuzz build, browser conformance, tracked\nartifact parity, the browser unit suite, and the Playwright matrix on\nchromium, firefox and webkit with a real BFCache lifecycle). The repository\ngate calls it in the full tier.\n\n## Spec map\n\nReading order is the order below. A domain without a spec file has no\nrequirements written yet; its published reference is the manual chapter\nnamed. Live has an in-depth reference of its own that this spec set cites\nrather than duplicates.\n\n| Domain | Prefix | File | State |\n|---|---|---|---|\n| RenderCache (document cache: policy, keys, storage, coherence) | CACHE | render-cache.md | Agreed 2026-09-13; ten requirements from the audit of that day, each refining Live specs 15-18; reference: `crates/suprnova-live/docs/specs/suprnova-live/15-...` to `18-...` and `manual/render-cache.md` |\n| Live (engine, facade, browser runtime, tooling) | LIVE | live.md | Observed; reference: `crates/suprnova-live/docs/specs/suprnova-live/` (26 numbered specs, glossary, conventions, iteration contracts 001-006) and `manual/live.md` |\n| Component library - foundations (inherited by every commitment) | UI | component-library-foundations.md | Draft; refines Live spec 20 |\n| Component library - form and input | FORM | component-library-forms.md | Draft; refines Live spec 21; the first commitment with foundations |\n| Component library - navigation | NAV | component-library-navigation.md | Draft; refines Live spec 22 |\n| Component library - overlay and disclosure | OVL | component-library-overlays.md | Draft; refines Live spec 23 |\n| Component library - feedback and status | FDB | component-library-feedback.md | Draft; refines Live spec 24 |\n| Component library - data display and layout | DATA | component-library-data-display.md | Draft; refines Live spec 25 |\n| Sessions (concurrent requests on one session) | SESS | sessions.md | Draft; SESS-001 Agreed 2026-09-15 on escalation `form-008` |\n| Laravel parity (the developer's rulings on the parity map, one commitment at a time) | PAR | laravel-parity.md | Draft; PAR-001 to PAR-009 Agreed 2026-10-01; the full ruled list is `feature-map/laravel/parity.jsonl` |\n| The manual checked against the code, and remediating what the check finds | MAN | manual-check.md | Agreed 2026-09-27 (delegated by the developer); tooling in `feature-map/`, commands in `feature-map/README.md` |\n\nDomains with no spec file yet, and the manual chapters that are their\npublished reference:\n\n| Domain | Reference |\n|---|---|\n| Application boot, config, container, console | `manual/lifecycle.md`, `manual/configuration.md`, `manual/container.md`, `manual/artisan.md` |\n| HTTP: routing, middleware, requests, responses, CSRF, CORS | `manual/routing.md`, `manual/middleware.md`, `manual/requests.md`, `manual/responses.md`, `manual/session.md` |\n| Views and Inertia frontends | `manual/views.md`, `manual/inertia.md`, `manual/frontend.md` |\n| Authentication, authorization, Magnetar | `manual/authentication.md`, `manual/authorization.md`, `manual/magnetar.md` |\n| Eloquent ORM, migrations, seeding | `manual/eloquent.md`, `manual/migrations.md`, `manual/seeding.md` |\n| Validation and localization | `manual/validation.md`, `manual/localization.md` |\n| Cache, RenderCache | `manual/cache.md`, `manual/render-cache.md` |\n| Queue, bus, events, schedule, workflow, supervisor | `manual/queues.md`, `manual/events.md`, `manual/scheduling.md` |\n| Mail, notifications, broadcasting, web push | `manual/mail.md`, `manual/notifications.md`, `manual/broadcasting.md` |\n| Filesystem, media, uploads | `manual/filesystem.md`, `manual/media.md` |\n| Payments | `manual/payments.md` |\n| Vector stores | `manual/vector.md` |\n| CLI and scaffolding | `manual/installation.md`, `manual/artisan.md` |\n| Testing surfaces | `manual/testing.md`, `manual/http-tests.md`, `manual/database-testing.md`, `manual/mocking.md` |\n| Release and distribution | `README.md` (Distribution model), `CHANGELOG.md` |\n\nManual chapter names above are the published reference for each domain;\nwhere a chapter is named differently in `manual/documentation.md`, that\nindex wins.\n\n## Technology choices with recorded reasons\n\n- Git tags instead of crates.io: absorb pre-1.0 API churn without SemVer\n  bumps downstream (`README.md`).\n- `inventory` for registration: declare-anywhere ergonomics; the cost is\n  that only linked code registers, so an unreachable module registers\n  nothing (framework module docs).\n- Askama for Live views: compile-time templates that the checker can prove\n  directive by directive (`manual/live.md`, Views).\n- Idiomorph as the browser runtime's one dependency; Stimulus 3.2 as an\n  optional bridge role the application supplies itself\n  (`framework/src/live/assets.rs:129-134`; Live conventions, pinned\n  dependencies).\n- Tailwind CSS 4 and semantic theme tokens for the official component\n  library, with the Live runtime itself CSS-agnostic: decided 2026-08-21 in\n  `crates/suprnova-live/docs/specs/suprnova-live/20-component-library-foundations.md`\n  (Decisions and revisions). The developer reopened the styling question on\n  2026-09-13; see `component-library-foundations.md`.\n\n## Status of this specification\n\nEvery file carries a `Status:` line; a requirement's own `Status:` line\noverrides its file's. Observed text is not contract. The loop refuses a\ncommitment that names an Observed or Draft requirement until the developer\nconfirms its text and falsifier as Agreed.\n",
  "glossary": "# Glossary\n\nStatus: Observed\n\nThis file owns the vocabulary this spec set uses. Suprnova Live has a\nglossary of its own at\n`crates/suprnova-live/docs/specs/suprnova-live/glossary.md`, agreed term by\nterm during Live's iterations; for any Live term, that file is the\nauthority and the entry here is a pointer with the line cited. Terms the\nlibrary work introduces are marked Draft until the developer confirms\nthem. The code's name for a thing wins over a synonym.\n\n**Live island.** A region of a server-rendered document that one Live\ncomponent owns: mounted with a signed snapshot, updated by morphing after an\naccepted action. Live glossary, \"Live island\". Avoid: widget (see below),\nfragment, partial.\n\n**Canonical document.** The complete server-rendered HTML a plain GET\nreturns, legible without JavaScript. Live glossary, \"Canonical document\".\nThe developer's fetchability bar (2026-09-13): an LLM or crawler fetching\nthe page gets all meaningful content.\n\n**Morph.** The runtime's replacement of an island's markup with the\nserver's new render while preserving identity under `live:key`, focus, and\ncompatible local state. Live glossary, \"Morph\".\n\n**Live directive.** An attribute in the closed `live:` grammar (`live:click`,\n`live:submit`, `live:model`, `live:key`, `live:loading`, ...) that the\nchecker proves against the component. Live glossary, \"Live directive\";\n`manual/live.md`, Views.\n\n**Local signal.** Browser-local state declared with `live:signal` and\nprojected through `live:toggle`, `live:show`, `live:class`, `live:attr`,\n`live:expanded` and siblings; no server round trip. Live glossary, \"Local\nsignal\".\n\n**Component state.** Server-owned typed state of a Live component:\n`#[public]` fields render and ride the signed snapshot; `#[model]` fields\nalso accept browser proposals. Live glossary, \"Component state\",\n\"Model-bindable field\", \"Transient model field\".\n\n**Browser effect.** A registered client-side behavior the reviewed runtime\nperforms on the server's instruction. Adding one is engine work, not\nlibrary work. Live glossary, \"Browser effect\".\n\n**Runtime feature artifact.** One reviewed, integrity-checked browser\nartifact served under the reserved `__live/assets` namespace; roles are\ncore, stimulus, uploads, and async, each in ESM and classic form\n(`crates/suprnova-live/src/artifacts.rs:87-103`). Live glossary, \"Runtime\nfeature artifact\".\n\n**Reviewed artifact-size baseline.** The recorded byte size each artifact\nmay not exceed without review; the Live gate's tracked-artifact parity\nphase rebuilds and compares. Live glossary, \"Reviewed artifact-size\nbaseline\". Avoid: byte budget (the informal name used in the 2026-09-12\nscoping notes).\n\n**Theme token.** A versioned semantic design value (color, typography,\nspacing, radius, motion, density, interaction state) that drives official\ncomponent presentation. Live glossary line 791. Avoid: raw palette value,\nTailwind utility class, component-specific hard-code.\n\n**RenderCache, Stitch slot, Cache variance.** The document cache, the\nper-principal segment stitched into a shared cached shell, and the declared\naxes a cached document varies on. Live glossary, same names. A library\ncomponent documents whether its render is principal-bound (a stitch slot)\nor shell bytes.\n\n**Progressive enhancement.** The canonical document is complete before the\nruntime loads; the runtime adds behavior. Live glossary, \"Progressive\nenhancement\".\n\n**Official component library.** The versioned set of components Suprnova\nships for Live, composable as application markup rather than an opaque\nclient-widget runtime; specified at capability level in Live specs 20-25.\nLive spec 20, \"Official library and application ownership\". Avoid: kit,\ndesign system, Flux (the developer's ruling, 2026-09-12: no reference to\nthat product).\n\n**Component family.** One of the five groups Live's specs partition the\nlibrary into: form and input (21), navigation (22), overlay and disclosure\n(23), feedback and status (24), data display and layout (25); one\nspec file per family (`component-library-*.md`), components inside each\nordered by complexity. Avoid:\ncategory (the 2026-09-12 inventory's six-way grouping, which the developer\ngave as examples; the spec families are the structure).\n\n**Presentational component.** Status: Draft. A library component with no\nserver state: an Askama macro over markup and tokens. Avoid: Tier 1 (the\n2026-09-12 scoping name; \"tier\" already means a RenderCache deployment tier\nand a Live ledger provider tier in the Live glossary, so the scoping name\ncollides and is retired).\n\n**Behavioral component.** Status: Draft. A library component that is a real\n`LiveComponent` with state, actions, validation, uploads, or streams.\nAvoid: Tier 2 (same collision).\n\n**Custom-element enhancement.** Status: Draft. A light-DOM custom element\nthat upgrades server-rendered markup with behavior the platform does not\ngive natively (the combobox listbox machinery, one-time-code auto-advance,\ndate-strip snap selection); form-associated through `ElementInternals` so\n`live:model` sees a real control. The developer's direction (2026-09-13):\nlight DOM only, an Elena-class helper as the base. Avoid: widget (ambiguous\nwith island), web component (implies shadow DOM).\n\n**Qualified engines.** Status: Draft. The three browser engines the Live\ngate pins and runs the Playwright matrix on: chromium, firefox, webkit\n(`crates/suprnova-live/browser/playwright.config.ts:58-62`, Playwright\n1.62.1). The developer's ruling (2026-09-13): the support matrix is these\nthree; a feature they disagree on waits or degrades.\n\n**Checker.** `suprnova live:check`: proves every directive in every\nregistered view against its component and fails on an unknown action, an\nunknown model field, a raw `safe` filter, or an accessibility violation,\nreporting file, line, and column; `--allow-unproved` accepts dynamic\nstructures it makes no claim about (`manual/live.md`, Views and\nDiagnostics; `suprnova-cli/src/commands/live_check.rs:145-147`). Avoid:\nlinter.\n",
  "commitment": {
    "slug": "laravel-port-requests",
    "requirements": [
      "PAR-001",
      "PAR-002",
      "PAR-003",
      "PAR-004",
      "PAR-005",
      "PAR-006",
      "PAR-007",
      "PAR-008",
      "PAR-009"
    ]
  },
  "requirements": [
    {
      "id": "PAR-001",
      "obligation": "The `DB::table` builder and the model query builder MUST offer joins: `join`, `left_join`, `right_join` and `cross_join` against a table with an optional alias; a closure form whose conditions combine `on`, `or_on`, `where` and `or_where`; `join_sub` and `left_join_sub` against another builder under an alias; and `where_exists` and `where_not_exists` taking a builder, which may correlate with `where_column`. Every value in a join or exists clause MUST be bound as a parameter, never written into the SQL text, and every table, alias and column MUST be quoted for the active backend. A model query with a join MUST select the model's own table columns unless told otherwise, so a joined table's `id` never overwrites the model's.",
      "falsifier": "one of the three query shapes in issue #125 (chained left joins with a table alias and aliased columns, a left join against a grouped subquery, a correlated `where_exists`) returns rows that differ from the same query written as raw SQL; a value given to a join's `where` appears in the generated SQL text; or a model query joining another table hydrates the joined table's `id` into the model.",
      "text_digest": "sha256:e72362c66d3709ee3232ac01f0065344c8e453552f10773a3c56d87607dc4313"
    },
    {
      "id": "PAR-002",
      "obligation": "The framework MUST offer file responses: a response streaming a file from a path with the content type its extension implies and `Content-Disposition: inline`; a download response with `attachment` and a filename the caller chooses; a download of in-memory bytes with a given content type; and the same two responses for a path on a named storage disk, resolved through the disk so its path guard applies. Every `Content-Disposition` header it writes MUST follow RFC 6266: an ASCII `filename` fallback, plus `filename*=UTF-8''` with percent-encoding when the name is not plain printable ASCII, and no character of the name may change the header's structure.",
      "falsifier": "a download named `Certificat·Joan Pérez.pdf` produces a header without a correct `filename*` or with a raw non-ASCII byte in `filename`; a name holding a quote, backslash, CR or LF changes the header's structure; a disk download of `../secret` reads outside the disk root; or a file response sends a content type or disposition other than the one specified.",
      "text_digest": "sha256:e01dafd8678cc5692e98555aa65eb21b38373714be68f39e29199d61aa023041"
    },
    {
      "id": "PAR-003",
      "obligation": "`#[handler]` MUST accept `#[authorize(ability, Type)]`, which authorizes the ability against a model type, and `#[authorize(ability, param)]`, which authorizes it against the route-bound model the handler receives as `param`. The check MUST run after route model binding and before the handler body, through the async gate, so policies and the RBAC gate bridge both apply. It MUST answer 401 when no user is authenticated, 403 when the gate denies, and 404 when the policy denies as not found. A `param` the handler does not take MUST be a compile error.",
      "falsifier": "a handler declaring `#[authorize(\"update\", post)]` runs its body for a user the policy denies; answers 403 to a guest; answers 403 where the policy denies as not found; answers 403 instead of 404 for a route model that does not exist; or compiles when `post` is not one of its parameters.",
      "text_digest": "sha256:3319c6c93161301c8ff8343ac21e41599b655c3d6a639db4de8b6de91884c749"
    },
    {
      "id": "PAR-004",
      "obligation": "After a successful save, a model MUST report which attributes that save changed: `was_changed` for one attribute or several, and `get_changes` for all of them. While the save's `updated` and `saved` observers run, `get_original` and `get_raw_original` MUST return each attribute's value as it was loaded before the save; once the save returns, the original values are the saved ones. The rest follows Laravel: a save that changes nothing leaves the previous save's changes in place, and an insert reports no changes.",
      "falsifier": "in an `updated` observer for a save that flips `is_admin`, `was_changed(\"is_admin\")` is false, `was_changed` reports an attribute the save did not change, `get_changes` omits or adds an attribute, or `get_raw_original(\"is_admin\")` is not the value loaded before the save; or after the save returns, `get_original(\"is_admin\")` still reports the value loaded before it.",
      "text_digest": "sha256:1e5f76b57881f32c195044d6b5bf11826e3a64f36f8ac158a5be3e7fe123013f"
    },
    {
      "id": "PAR-005",
      "obligation": "A belongs-to-many relation MUST offer `sync_without_detaching`, which attaches each given id the relation does not already hold and leaves every existing pivot row, its pivot columns included, untouched.",
      "falsifier": "after `sync_without_detaching` with id 3 on a relation that holds 1 and 2, the relation holds anything other than 1, 2 and 3, or a pivot column of the rows for 1 or 2 changed.",
      "text_digest": "sha256:20c2f12c8f4432a6cf4edaac4f845c89956a70bf275c3605de5168193890f51c"
    },
    {
      "id": "PAR-006",
      "obligation": "The `DB::table` builder and the model query builder MUST offer `where_any`, `or_where_any`, `where_all`, `or_where_all`, `where_none` and `or_where_none` (one comparison across several columns, grouped in parentheses), `or_where_in` and `or_where_not_in` with a list or a subquery, `where_in` and `where_not_in` with a subquery, `or_where_raw` with bound values, `or_where_null` and `or_where_not_null`, and `reorder`, which drops the orderings already set and may set a new one.",
      "falsifier": "one of these helpers returns rows that differ from the equivalent raw SQL; a grouped helper lets an `or` escape its parentheses, so `where(\"a\", 1).where_any([\"b\", \"c\"], \"=\", 2)` returns a row whose `a` is not 1; or a subquery used by `where_in` loses its own bound values.",
      "text_digest": "sha256:5e28f01c974d87cc95f9f163661a2e4f2e23418c057f2f52a32707abaca068e5"
    },
    {
      "id": "PAR-007",
      "obligation": "`DB::after_commit` MUST run a callback after the outermost open transaction commits, at once when no transaction is open, and never when that transaction rolls back.",
      "falsifier": "the callback runs inside a transaction that rolls back, runs before the outermost commit, or does not run when no transaction is open.",
      "text_digest": "sha256:6ae461f3c1a86525c70bda9a33ab4a257902946905d9008d51e8817f96ce3f95"
    },
    {
      "id": "PAR-008",
      "obligation": "`inertia_response!` MUST read an optional page lookup from the application crate's `Cargo.toml`, under `[package.metadata.suprnova.inertia]`: `pages_dir`, a directory relative to the crate, and `page_file`, a file pattern in which `{dir}` is the component name up to its last `/`, `{name}` the last segment, and a `|lower`, `|kebab` or `|snake` filter may follow either. When a lookup is set, the macro MUST accept exactly the components whose file exists at the resolved path and name that path in its error. Without one, the lookup MUST stay `frontend/src/pages/{Component}` with the extensions `svelte`, `tsx`, `jsx` and `vue`.",
      "falsifier": "with `pages_dir = \"resources/angular/pages\"` and `page_file = \"{dir}/{name|lower}.page.ts\"`, `Tramits/BaixaMatricula/Create` fails although `resources/angular/pages/Tramits/BaixaMatricula/create.page.ts` exists, or compiles although it does not; or, without the keys, a starter project's page lookup changes.",
      "text_digest": "sha256:40684adb7a4899a70685d0c5db3716389565f8df99b487367942a90c8be7ebfc"
    },
    {
      "id": "PAR-009",
      "obligation": "The queue fake MUST offer `except`, which fakes every job but the named job types and sends those to the real queue; MUST record raw pushes and return them on request; and MUST offer `assert_pushed_without_chain`, which passes only for a job pushed with no chain.",
      "falsifier": "under `except`, a named job is recorded instead of dispatched or another job is dispatched instead of recorded; a raw push cannot be read back; or `assert_pushed_without_chain` passes for a job pushed with a chain.",
      "text_digest": "sha256:d888bb84588c7ba54fd2189dde57c9dc6f5004fe3f2a9aef9c9f8762de337e56"
    }
  ],
  "decisions": []
}

## Facts
{
  "concerns": [
    "PAR-007"
  ],
  "option_index": 0,
  "option_count": 2,
  "path_counts": {
    "plain": 1
  },
  "paths_known": true,
  "protected": {
    "data": false,
    "contract": false,
    "agreement": false,
    "reserved": false
  },
  "attempts": {
    "PAR-007": 1
  },
  "open_obligations": {
    "escalations": 0,
    "findings": 0,
    "defects": 0,
    "breaches": 0
  },
  "cited": []
}

## Score five dimensions, 0 to 4 each

### evidence
Score `state.five.because`: how concretely does it ground its claim, from no evidence to quoted output naming the failing test and the falsifier it maps to.

0: none
1: a claim
2: names a command or file
3: quotes output or a diff
4: quotes output and names the test that fails and the falsifier it maps to

### reach
Score `state.option` (the recommended option, draft.options[0]): how far does choosing it extend, from wording or message text only to data that cannot be regenerated, a migration, or a rewrite of user files. Compare it against the alternatives in `state.options`.

0: wording or message text
1: internal structure, nothing visible
2: behavior inside an agreed requirement
3: output or flags existing callers depend on
4: data that cannot be regenerated, a migration, or a rewrite of user files

### contract
Score `state.option`'s fit with the cited requirement's contract (draft.options[0]): from implementing the cited requirement as written to changing a requirement's text or falsifier. Compare it against the alternatives in `state.options`.

0: implements the cited requirement as written
1: chooses between readings the text allows
2: adds behavior no requirement names
3: conflicts with a cited decision
4: changes a requirement's text or falsifier

### surface
Score `state.option`'s new surface (draft.options[0]): from no new surface to a network call, credential, or external service. Compare it against the alternatives in `state.options`.

0: no new surface
1: a new file or module
2: a new flag or output
3: a new dependency
4: a network call, credential, or external service

### ambiguity
Score `state.five.question`: from one reading the draft names, to a question that cannot be answered without facts the draft lacks.

0: one reading, the draft names it
1: two readings, the draft picks one with a reason
2: two readings, no reason
3: the question asks the developer to choose a policy
4: the question cannot be answered without facts the draft lacks

## Boundary
Harness claude_code, model any, transport any.
Boundary: unenforced.

## Your work
You have none of the drafting agent's conversation context. Score each of the five dimensions above from the state given, 0 to 4, with a confidence in [0,1] and a probability distribution over the five levels summing to 1.
write its five Score answers as JSON: {"model": "<your model id>", "session": "<your session id>", "transport": "local"|"remote", "usage": {"input_tokens": N, "output_tokens": N}, "answers": {"evidence": {"type":"score","score":N,"confidence":N,"probabilities":{"0":N,...,"4":N}}, "reach": {...}, "contract": {...}, "surface": {...}, "ambiguity": {...}}}
to a file, then run: sudus measure <slug> --file <path>
