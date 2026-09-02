//! Idempotency keys backed by cache locks.
//!
//! Three entry points, increasing in guarantee:
//!
//! - [`Idempotency::once`] - dedupe only. Runs `body` for the first caller in
//!   the TTL window; duplicates get [`Idempotent::Duplicate`] and the body does
//!   NOT run. The lock is never released - `ttl` IS the dedupe window.
//! - [`Idempotency::commit_on_success`] - like `once`, but releases the lock if
//!   `body` returns `Err`, so a transient failure can be retried within the
//!   window.
//! - [`Idempotency::remember`] - dedupe WITH result replay. Stores the success
//!   value and replays it to duplicate callers ([`Replay::Replayed`]). This is
//!   the Stripe-style idempotency-key model for HTTP endpoints and queue jobs
//!   that must return the original outcome, not merely skip re-execution.
//!
//! ## Lease renewal
//!
//! All three keep the lock's lease alive for the duration of `body`, so a
//! body that runs longer than `ttl` does not let the lock expire and a
//! second caller execute concurrently. See `run_under_lease`.
//!
//! Renewal can still fail - the backend can go away, or the lock can be
//! taken over after an expiry the refresh interval did not beat. When that
//! happens the body is **not** cancelled: by the time a lease is lost it may
//! already have charged a card or sent a message, and cancelling would strand
//! that with no record. It runs to completion and the loss is reported as
//! [`Idempotent::FreshUnfenced`] / [`Replay::FreshUnfenced`].
//!
//! So the guarantee is precisely: *`Fresh` means exclusivity held throughout;
//! `FreshUnfenced` means the work completed but another caller may have run
//! it concurrently.* Callers that need certainty must handle the second
//! variant - and because it is a distinct variant rather than a flag, an
//! exhaustive `match` will not let it be ignored by accident.
//!
//! A transient periodic refresh *error* is not treated as loss on its own: it
//! means the backend could not be asked, not that somebody else holds the lock,
//! so renewal retries and only gives up after several consecutive failures.
//! After the body completes, however, one final owner-scoped refresh must prove
//! the token is still current before the result can be reported as fenced. An
//! error in that final check yields `FreshUnfenced` because ownership is then
//! unknown.
//!
//! ## Key material
//!
//! Caller-supplied key material is hashed before it touches the cache backend,
//! so arbitrary client-supplied keys cannot produce unbounded backend keys,
//! leak raw identifiers into cache tooling, or inject characters that collide
//! with backend key conventions.
//!
//! ## Shared backend
//!
//! Cross-process dedupe requires a cross-process cache (e.g. Redis). With the
//! in-memory backend - or a Redis bootstrap that fell back to memory - the
//! dedupe window is per-process only.

use crate::cache::Cache;
use crate::error::FrameworkError;
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::time::Duration;

/// The outcome of a dedupe-only idempotent operation
/// ([`Idempotency::once`] / [`Idempotency::commit_on_success`]).
#[derive(Debug, PartialEq, Eq)]
pub enum Idempotent<T> {
    /// First caller in the TTL window - `body` ran under an unbroken lease
    /// and produced this value.
    Fresh(T),
    /// `body` ran to completion and produced this value, but the lock's
    /// lease was lost partway through, so **exclusivity is not
    /// guaranteed** - another caller may have executed concurrently.
    ///
    /// The body is deliberately not cancelled when this happens: by the
    /// time a lease is lost the body may already have charged a card or
    /// sent a message, and cancelling would leave that half-done with no
    /// record. Finishing and reporting honestly beats aborting blind.
    ///
    /// Treat it as "probably fine, provably not exclusive". Reconcile,
    /// alert, or compensate - but do not silently treat it as [`Fresh`].
    ///
    /// [`Fresh`]: Self::Fresh
    FreshUnfenced(T),
    /// Duplicate caller within the TTL window - `body` was NOT run.
    Duplicate,
}

/// The outcome of a result-replaying idempotent operation
/// ([`Idempotency::remember`]).
#[derive(Debug, PartialEq, Eq)]
pub enum Replay<T> {
    /// First caller - `body` ran under an unbroken lease, produced this
    /// value, and recorded it for replay.
    Fresh(T),
    /// `body` ran to completion and its result was recorded, but the
    /// lock's lease was lost partway through, so **exclusivity is not
    /// guaranteed** - another caller may have executed concurrently.
    ///
    /// See [`Idempotent::FreshUnfenced`] for why the body is allowed to
    /// finish rather than being cancelled.
    FreshUnfenced(T),
    /// Duplicate caller - `body` already completed; this is the recorded result.
    Replayed(T),
    /// Duplicate caller arriving while the original `body` is still running, with
    /// no recorded result yet. The original has not finished, so there is nothing
    /// to replay. HTTP callers typically map this to `409 Conflict` and retry.
    InProgress,
}

/// Thin wrapper over [`Cache::lock`] providing idempotency-key semantics.
pub struct Idempotency;

impl Idempotency {
    /// Run `body` exactly once per `key` within the given `ttl` window.
    ///
    /// - First caller: acquires a lock at `idem:<hash>`, runs `body`, returns
    ///   `Ok(Idempotent::Fresh(result))`.
    /// - Subsequent callers within `ttl`: return `Ok(Idempotent::Duplicate)`
    ///   without running `body`.
    ///
    /// The lock is intentionally NOT released on success - the TTL IS the
    /// dedupe window. The lock's lease is refreshed in the background for the
    /// body's duration (see `run_under_lease`), so a body that runs longer
    /// than `ttl` does not collapse the window or allow concurrent execution;
    /// the effective window is "body duration + up to `ttl` after the last
    /// refresh".
    ///
    /// Choose [`commit_on_success`](Self::commit_on_success) instead when a
    /// failed `body` should be retryable within the window, or
    /// [`remember`](Self::remember) when duplicates must receive the original
    /// result rather than a bare `Duplicate` marker.
    ///
    /// # Errors
    ///
    /// Propagates any [`FrameworkError`] from the cache layer or from `body`.
    pub async fn once<F, Fut, T>(
        key: &str,
        ttl: Duration,
        body: F,
    ) -> Result<Idempotent<T>, FrameworkError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, FrameworkError>>,
    {
        let h = hashed(key);
        let guard = Cache::lock(&lock_key(&h), ttl).await?;
        match guard {
            Some(g) => {
                let (v, lease) = run_under_lease(&g, ttl, &h, body()).await?;
                // Do NOT release - the TTL is the dedupe window.
                Ok(match lease {
                    LeaseState::Held => Idempotent::Fresh(v),
                    LeaseState::Lost => Idempotent::FreshUnfenced(v),
                })
            }
            None => Ok(Idempotent::Duplicate),
        }
    }

    /// Like [`once`](Self::once), but releases the dedupe lock if `body` returns
    /// `Err`, allowing the operation to be retried within the TTL window.
    ///
    /// Use this when:
    /// - The body has retryable failure modes (transient network error,
    ///   rate limit, etc.)
    /// - You want at-least-once semantics for success and at-most-once
    ///   only after success
    ///
    /// Use [`once`](Self::once) instead when:
    /// - The body's side effects must not repeat regardless of outcome
    ///   (e.g., "I sent the email; even if I errored after sending,
    ///   don't try again")
    ///
    /// # Errors
    ///
    /// Propagates any [`FrameworkError`] from the cache layer or from `body`.
    pub async fn commit_on_success<F, Fut, T>(
        key: &str,
        ttl: Duration,
        body: F,
    ) -> Result<Idempotent<T>, FrameworkError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, FrameworkError>>,
    {
        Self::commit_on_success_owned(key, ttl, |_owner| body())
            .await
            .map(|(outcome, _owner)| outcome)
    }

    /// Like [`commit_on_success`](Self::commit_on_success), but hands `body`
    /// the cache-lock owner token and returns it alongside the outcome.
    ///
    /// Callers that must release the lock *later, from another task* - the
    /// queue worker releasing a
    /// [`Job::unique_until_processing`](crate::queue::Job::unique_until_processing)
    /// lock once the handler starts - need the owner token, because a
    /// [`LockGuard`](crate::cache::LockGuard) cannot cross that boundary: its
    /// lifetime is this call's. Persist the token (on a queue envelope, a row,
    /// a message header) and release with
    /// [`release_owned`](Self::release_owned).
    ///
    /// `body` receives `Some(owner)` whenever a lock was taken, which is every
    /// call that runs it at all. The *returned* owner is `Some` only for
    /// [`Idempotent::Fresh`]: on
    /// [`FreshUnfenced`](Idempotent::FreshUnfenced) the lease was lost, so the
    /// token can no longer be claimed to be held and handing it back would
    /// invite a release that must fail.
    ///
    /// # Errors
    ///
    /// Propagates any [`FrameworkError`] from the cache layer or from `body`.
    pub async fn commit_on_success_owned<F, Fut, T>(
        key: &str,
        ttl: Duration,
        body: F,
    ) -> Result<(Idempotent<T>, Option<String>), FrameworkError>
    where
        F: FnOnce(Option<&str>) -> Fut,
        Fut: Future<Output = Result<T, FrameworkError>>,
    {
        let h = hashed(key);
        let guard = Cache::lock(&lock_key(&h), ttl).await?;
        match guard {
            Some(g) => {
                let owner = g.owner().to_string();
                let fut = body(Some(&owner));
                match run_under_lease(&g, ttl, &h, fut).await {
                    Ok((v, LeaseState::Held)) => Ok((Idempotent::Fresh(v), Some(owner))),
                    Ok((v, LeaseState::Lost)) => Ok((Idempotent::FreshUnfenced(v), None)),
                    Err(e) => {
                        // Release the lock so a retry within the window can re-enter.
                        release_and_log(g, &h).await;
                        Err(e)
                    }
                }
            }
            None => Ok((Idempotent::Duplicate, None)),
        }
    }

    /// Release an idempotency lock owner-scoped, using a token recorded by
    /// [`commit_on_success_owned`](Self::commit_on_success_owned).
    ///
    /// Lives here rather than at the call site so the `idem:<hash>` key scheme
    /// stays in one file: an acquire path and a deferred release path that
    /// derive the backend key separately are one edit away from silently
    /// releasing nothing.
    ///
    /// Returns `Ok(true)` when the store released a lock this owner held, and
    /// `Ok(false)` when the lock was absent or is held by a different owner.
    /// Both are success cases: either way this owner no longer holds the lock,
    /// and a newer holder is never disturbed - which is the whole point of
    /// scoping the release to an owner instead of forcing it.
    ///
    /// # Errors
    ///
    /// Propagates any [`FrameworkError`] from the cache layer.
    pub async fn release_owned(key: &str, owner: &str) -> Result<bool, FrameworkError> {
        let store = Cache::store()?;
        store.release_lock(&lock_key(&hashed(key)), owner).await
    }

    /// Run `body` once per `key` and replay its recorded result to duplicate
    /// callers within the TTL window.
    ///
    /// Unlike [`once`](Self::once) (which only tells a duplicate that the work
    /// already happened), `remember` records the success value and hands it back
    /// to later callers - the Stripe-style idempotency-key contract that lets an
    /// HTTP endpoint or queue job return the original response on retry.
    ///
    /// Outcomes:
    /// - [`Replay::Fresh`] - first caller; `body` ran and the result was recorded.
    /// - [`Replay::Replayed`] - duplicate; the recorded result is returned and
    ///   `body` did NOT run.
    /// - [`Replay::InProgress`] - duplicate arriving while the original `body` is
    ///   still running; there is no recorded result yet. Map this to a retryable
    ///   `409 Conflict` at the HTTP layer.
    ///
    /// Semantics:
    /// - **Errors do not replay.** A failing `body` releases the lock and returns
    ///   the error, so the operation is retryable within the window (same policy
    ///   as [`commit_on_success`](Self::commit_on_success)). Only success values
    ///   are recorded.
    /// - **The result payload reaches the cache backend.** The *key* is hashed,
    ///   but `T` is serialized verbatim; do not place secrets in a replayed value
    ///   that must not appear in your cache store.
    /// - **`body` should be cancel-safe** if the caller may drop this future:
    ///   cancellation drops `body` mid-await like any other tokio future.
    ///
    /// # Keyspace responsibility (cross-endpoint isolation)
    ///
    /// `remember` keys the lock + result cache on `hashed(key)` only -
    /// the caller's `key` is the entire isolation surface. The caller
    /// **MUST** namespace the key with the route + user / business
    /// identity before passing it in; otherwise an attacker who
    /// captures an idempotency key issued for endpoint A can present
    /// the same key to endpoint B and receive A's recorded `T` as a
    /// `Replay::Replayed(...)` (the function happily replays the
    /// cached value regardless of which endpoint asks for it).
    ///
    /// The recommended shape:
    ///
    /// ```rust,ignore
    /// // GOOD - endpoint + user namespace isolates the cache cell.
    /// let cache_key = format!(
    ///     "{}:{}:{}",
    ///     request.method(),
    ///     request.path(),
    ///     idempotency_key_from_client,
    /// );
    /// Idempotency::remember(&cache_key, ttl, body).await?
    ///
    /// // BAD - bare client key leaks across endpoints.
    /// Idempotency::remember(idempotency_key_from_client, ttl, body).await?
    /// ```
    ///
    /// Mirrors Stripe's published contract: an idempotency key is
    /// scoped to a single operation, and the server is expected to
    /// detect mismatched request fingerprints. Suprnova provides the
    /// replay primitive; you provide the scope.
    ///
    /// # Errors
    ///
    /// Propagates any [`FrameworkError`] from the cache layer or from `body`.
    pub async fn remember<F, Fut, T>(
        key: &str,
        ttl: Duration,
        body: F,
    ) -> Result<Replay<T>, FrameworkError>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<T, FrameworkError>>,
        T: Serialize + DeserializeOwned,
    {
        let h = hashed(key);
        let lock = lock_key(&h);
        let result_key = format!("idem:{h}:result");

        // 1. Fast path: a result is already recorded - replay it without locking.
        if let Some(value) = Cache::get::<T>(&result_key).await? {
            return Ok(Replay::Replayed(value));
        }

        // 2. Try to claim the lock so we are the only caller running `body`.
        match Cache::lock(&lock, ttl).await? {
            Some(guard) => {
                // 2a. Re-check after acquiring: a result may have been stored
                //     between the step-1 read and the lock acquisition.
                if let Some(value) = Cache::get::<T>(&result_key).await? {
                    release_and_log(guard, &h).await;
                    return Ok(Replay::Replayed(value));
                }
                // 2b. Run the body (lease kept alive throughout), then record the
                //     result BEFORE releasing the lock so no duplicate can slip in
                //     and re-run between the store and the release. If the store
                //     fails, the guard drops un-released and the lock holds until
                //     TTL (fail-closed: duplicates see InProgress, never a second
                //     execution).
                match run_under_lease(&guard, ttl, &h, body()).await {
                    Ok((value, lease)) => {
                        // Record the result even when the lease was lost:
                        // the body did run and produce it, and a duplicate
                        // that replays a real result is strictly better off
                        // than one that re-executes.
                        Cache::put(&result_key, &value, Some(ttl)).await?;
                        release_and_log(guard, &h).await;
                        Ok(match lease {
                            LeaseState::Held => Replay::Fresh(value),
                            LeaseState::Lost => Replay::FreshUnfenced(value),
                        })
                    }
                    Err(e) => {
                        release_and_log(guard, &h).await;
                        Err(e)
                    }
                }
            }
            // 3. Contended: another caller holds the lock and is running `body`.
            //    Re-read once in case they finished between the attempt and now.
            None => match Cache::get::<T>(&result_key).await? {
                Some(value) => Ok(Replay::Replayed(value)),
                None => Ok(Replay::InProgress),
            },
        }
    }
}

/// Run `body` while keeping the idempotency lock's lease alive.
///
/// A background task refreshes the lock at one-third of `ttl` (floored at 50ms
/// to avoid a busy-loop on pathologically small TTLs) for as long as `body` is
/// running, so a body that outlives its original `ttl` cannot let the lock
/// expire and a second caller execute concurrently - the double-execution
/// window the bare lock left open. The renewal task parks (never resolves) so
/// the `select!` always completes via `body`. Periodic errors are retried up to
/// the configured limit. After `body` completes, an owner-scoped refresh is
/// required before the lease can be reported as held. Tested with `ttl >= 1s`;
/// a very short `ttl` may not refresh before the first expiry.
/// Whether exclusivity held for the whole of `body`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeaseState {
    /// No refresh established loss, and the final owner check succeeded.
    Held,
    /// The lease was lost, or could no longer be proven alive.
    Lost,
}

/// How many consecutive refresh *errors* to ride out before giving up.
///
/// A backend error is not evidence that anyone else took the lock - it is
/// evidence we could not ask. Treating the first blip as fatal meant one
/// dropped packet stopped renewal for the rest of a long body, guaranteeing
/// the lease would lapse even though the backend recovered milliseconds
/// later. Three consecutive failures spans a full interval of trouble,
/// which is a real outage rather than a hiccup.
const MAX_CONSECUTIVE_REFRESH_ERRORS: u32 = 3;

async fn run_under_lease<T>(
    guard: &crate::cache::LockGuard,
    ttl: Duration,
    hashed_key: &str,
    body: impl Future<Output = Result<T, FrameworkError>>,
) -> Result<(T, LeaseState), FrameworkError> {
    // Written by the renewal branch, read after the body completes. An
    // atomic rather than a `Cell` so the returned future stays `Send`.
    let lease_lost = std::sync::atomic::AtomicBool::new(false);

    let renew = async {
        let interval = (ttl / 3).max(Duration::from_millis(50));
        let mut consecutive_errors: u32 = 0;
        loop {
            tokio::time::sleep(interval).await;
            match guard.refresh(ttl).await {
                Ok(true) => {
                    consecutive_errors = 0;
                }
                Ok(false) => {
                    // Definite loss: the token no longer matches, which
                    // means somebody else holds this lock right now.
                    tracing::warn!(
                        idempotency_key = %hashed_key,
                        "idempotency lease lost (lock token no longer matches); \
                         body continues but is no longer fenced"
                    );
                    lease_lost.store(true, std::sync::atomic::Ordering::Release);
                    break;
                }
                Err(e) => {
                    consecutive_errors += 1;
                    if consecutive_errors >= MAX_CONSECUTIVE_REFRESH_ERRORS {
                        tracing::warn!(
                            idempotency_key = %hashed_key,
                            error = %e,
                            consecutive_errors,
                            "idempotency lease refresh failed repeatedly; giving up \
                             renewal - exclusivity can no longer be proven"
                        );
                        lease_lost.store(true, std::sync::atomic::Ordering::Release);
                        break;
                    }
                    tracing::warn!(
                        idempotency_key = %hashed_key,
                        error = %e,
                        consecutive_errors,
                        "idempotency lease refresh failed; retrying at the next interval"
                    );
                }
            }
        }
        // Park forever so `select!` only ever completes through the body branch.
        std::future::pending::<()>().await;
    };

    tokio::pin!(body);
    let result = tokio::select! {
        biased;
        result = &mut body => result,
        _ = renew => unreachable!("renewal future parks after the loop and never resolves"),
    }?;

    let state = if lease_lost.load(std::sync::atomic::Ordering::Acquire) {
        LeaseState::Lost
    } else {
        match guard.refresh(ttl).await {
            Ok(true) => LeaseState::Held,
            Ok(false) => {
                tracing::warn!(
                    idempotency_key = %hashed_key,
                    "idempotency lease ownership could not be confirmed after body completion"
                );
                LeaseState::Lost
            }
            Err(e) => {
                tracing::warn!(
                    idempotency_key = %hashed_key,
                    error = %e,
                    "idempotency lease ownership check failed after body completion"
                );
                LeaseState::Lost
            }
        }
    };
    Ok((result, state))
}

/// Release the lock, logging (never returning) on failure.
///
/// A failed release does not change the caller's primary result, but a token
/// mismatch or backend error means a retry may stay blocked until the TTL
/// lapses, so it must be observable. Logs the hashed key, never the raw key
/// material.
async fn release_and_log(guard: crate::cache::LockGuard, hashed_key: &str) {
    match guard.release().await {
        Ok(true) => {}
        Ok(false) => tracing::warn!(
            idempotency_key = %hashed_key,
            "idempotency lock release found a token mismatch (already expired or taken over)"
        ),
        Err(e) => tracing::warn!(
            idempotency_key = %hashed_key,
            error = %e,
            "idempotency lock release failed"
        ),
    }
}

/// Hash caller-supplied key material into a fixed-length hex digest.
///
/// Bounds backend key length to 64 hex chars regardless of input, keeps raw
/// identifiers (which may be PII or client-controlled) out of cache tooling,
/// and strips any characters that could collide with backend key conventions.
fn hashed(key: &str) -> String {
    crate::hashing::sha256_hex(key)
}

/// The backend key an idempotency lock lives under, given already-[`hashed`]
/// key material.
///
/// One definition of the namespace so every acquire path and the deferred
/// [`Idempotency::release_owned`] path address the same cell. A release that
/// derives a different key does not fail loudly - it reports "nothing to
/// release" and the lock lingers until its TTL.
fn lock_key(hashed_key: &str) -> String {
    format!("idem:{hashed_key}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashed_is_fixed_length_hex_regardless_of_input() {
        let short = hashed("k");
        let long = hashed(&"x".repeat(100_000));
        assert_eq!(short.len(), 64);
        assert_eq!(long.len(), 64);
        assert!(short.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(long.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hashed_does_not_leak_raw_key_material() {
        let raw = "user-4242-card-secret";
        let key = format!("idem:{}", hashed(raw));
        assert!(!key.contains(raw), "raw key material leaked into cache key");
        assert!(key.starts_with("idem:"));
    }

    #[test]
    fn distinct_keys_hash_distinctly() {
        assert_ne!(hashed("a"), hashed("b"));
    }
}
