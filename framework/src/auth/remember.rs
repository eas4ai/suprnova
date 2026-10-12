//! Remember-me tokens - a hardened selector+verifier scheme.
//!
//! "Remember me" is the cookie that re-authenticates a user after their
//! session expires or their browser is closed. Done wrong it is a long-
//! lived bearer credential sitting in plaintext on a user's disk and in
//! a database backup - done right it is a rotating, single-use,
//! short-fuse re-auth handle with constant-cost verification.
//!
//! # Design - selector + verifier scheme
//!
//! Each issued cookie is a two-part composite plaintext token
//! `"{selector}.{verifier}"` where:
//!
//! - **selector** - 16 random bytes (128 bits), URL-safe base64
//!   (22 chars, no padding). Stored plaintext on a UNIQUE indexed
//!   column. The selector is the lookup key - one indexed query
//!   identifies the exactly-one candidate row to verify, so
//!   verification cost is O(1) regardless of how many active tokens
//!   the table holds.
//!
//! - **verifier** - 32 random bytes (256 bits), URL-safe base64
//!   (43 chars, no padding). Bcrypt-hashed before storage. The
//!   plaintext verifier lives only inside the encrypted `remember_me`
//!   cookie. Verification is exactly one constant-time
//!   `bcrypt::verify` against the selector-matched row.
//!
//! # Why both parts
//!
//! A pure hash-only design (no selector) had two problems:
//!
//! 1. **Unbounded bcrypt scan on forged cookies.** Each row's bcrypt
//!    hash is per-row salted, so without a selector we had to scan
//!    every unexpired row and bcrypt-verify each - O(N) bcrypt work per
//!    request, attacker-controlled.
//!
//! 2. **Non-single-use rotation under concurrency.** Two concurrent
//!    requests could both load the same row, both pass `bcrypt::verify`,
//!    and one of the DELETEs would affect zero rows - but both still
//!    minted replacement tokens. The selector-keyed atomic-DELETE
//!    pattern (`DELETE ... WHERE id = ? AND selector = ?`, then check
//!    `rows_affected`) makes rotation single-use even under concurrency.
//!
//! A pure-selector design (no verifier hashed) would mean a DB dump
//! yields re-authenticating credentials - same standard as why
//! passwords are hashed. The verifier preserves the "DB dump is not
//! enough" property.
//!
//! # Lifecycle
//!
//! - **Issuance**: random selector + random verifier → bcrypt-hash
//!   the verifier bound to the user's epoch (see "Epoch binding" below)
//!   → INSERT row → encrypt `"{selector}.{verifier}"`
//!   inside a versioned, guard-named carrier under `Crypt` → place the
//!   encrypted blob in the `remember_me` cookie.
//!
//! - **Verification**: decrypt cookie → split into
//!   `(selector, verifier)` → `SELECT row WHERE selector = ? AND
//!   expires_at > now LIMIT 1` → read the user's current epoch →
//!   `bcrypt::verify("{verifier}.{epoch}", row.token_hash)`. On match,
//!   atomic conditional DELETE keyed on `(id, selector)`; the loser of a
//!   race sees `rows_affected == 0` and returns `None` (replay defeated).
//!   On mismatch, the row is deleted and the token fails.
//!
//! - **Rotation**: a successful verify DELETEs the matched row and
//!   issues a fresh selector+verifier pair. The middleware re-sets the
//!   cookie with the new composite plaintext.
//!
//! - **Epoch binding (HRD-007)**: a row is bound to the user's
//!   authentication epoch at issue (`crate::auth::epoch`): the hash stored
//!   is the hash of `"{verifier}.{epoch}"`, not of the verifier alone.
//!   Verification reads the user's current epoch through the guard's
//!   provider and checks the verifier mixed with *that* epoch, so a row
//!   issued before a password change fails verification after it, is
//!   deleted, and its cookie is cleared, whether or not the reset's
//!   revocation of the user's rows succeeded. A user without an epoch mixes
//!   nothing in. No column changes: the epoch lives only inside the hash.
//!
//! - **Revocation**: [`revoke_all_for_user`] wipes every row for a user
//!   in one DELETE. `Auth::logout` chains this so a real logout
//!   actually clears persistent state. [`prune_expired`] cleans up
//!   expired rows on a schedule.
//!
//! # Cookie format
//!
//! New on-wire cookies encrypt a private, versioned carrier containing the
//! complete guard name and `"{selector}.{verifier}"`. During the compatibility
//! window, an unwrapped decrypted composite is accepted as a default-guard
//! carrier. The framework never persists the verifier or plaintext carrier.
//! Tokens whose credential lacks the `.` separator are silently rejected (no
//! DB hit, no bcrypt cost).
//!
//! # Why bcrypt, not "store the verifier plaintext"
//!
//! A DB dump must not yield credentials that re-authenticate the user.
//! Same reason passwords are hashed.

use chrono::Duration;
use sea_orm::{
    ColumnTrait, EntityTrait, FromQueryResult, QueryFilter, QuerySelect, Select, Set,
    TransactionTrait,
};

use crate::database::DB;
use crate::database::stored_datetime::storable_expiry;
use crate::error::FrameworkError;
use crate::hashing;

/// Width of the selector. 16 bytes = 128 bits - enough collision
/// resistance for the lookup key over the lifetime of the deployment.
/// URL-safe base64 encodes to 22 chars (no padding).
const SELECTOR_BYTES: usize = 16;

/// Width of the verifier plaintext. 32 bytes = 256 bits of entropy.
/// URL-safe base64 encodes to 43 chars (no padding).
const VERIFIER_BYTES: usize = 32;

/// The length of the encoded verifier: URL-safe base64 without padding.
const VERIFIER_ENCODED_LEN: usize = (VERIFIER_BYTES * 4).div_ceil(3);

/// bcrypt hashes only the first 72 bytes of its input and ignores the rest
/// without a word. An epoch past that limit would be cut off, and a row
/// would verify under any epoch: the binding would be silently void. The
/// longest bound input is the verifier, a `.` and the 20 digits of
/// `u64::MAX`, so it is checked here, at compile time, that it fits.
const _: () = assert!(VERIFIER_ENCODED_LEN + 1 + 20 <= 72);

/// Cookie name carrying the encrypted plaintext token.
///
/// Exposed as `pub` (rather than `pub(crate)`) so framework integration
/// tests can probe the cookie by name. `#[doc(hidden)]` keeps it out of
/// the published API surface - consumers should not reference it
/// directly; the contract is "the framework owns the remember-me
/// cookie."
#[doc(hidden)]
pub const COOKIE_NAME: &str = "remember_me";

/// The columns verification and revocation read from a token row.
///
/// The time columns are left out: the queries filter on `expires_at` in
/// SQL and never need its value.
#[derive(FromQueryResult)]
struct TokenRow {
    id: i64,
    user_id: String,
    selector: String,
    token_hash: String,
}

/// Narrow `select` to the [`TokenRow`] columns.
fn token_rows(select: Select<entity::Entity>) -> sea_orm::Selector<sea_orm::SelectModel<TokenRow>> {
    select
        .select_only()
        .columns([
            entity::Column::Id,
            entity::Column::UserId,
            entity::Column::Selector,
            entity::Column::TokenHash,
        ])
        .into_model::<TokenRow>()
}

/// Generate a fresh `(selector, verifier_plaintext, verifier_hash)`
/// triple.
///
/// The composite `"{selector}.{verifier_plaintext}"` is what we hand
/// the user via the cookie. The selector is stored plaintext (indexed,
/// for O(1) lookup) and the hash is what we store in
/// `remember_tokens.token_hash`. Bcrypt because the framework already
/// uses bcrypt for passwords (`framework/src/hashing/`), so we do not
/// multiply hash schemes.
///
/// `pub` + `#[doc(hidden)]` so integration tests in
/// `framework/tests/remember_me.rs` can build "real-shaped" rows whose
/// hash matches a known verifier (used for the expired-token test).
#[doc(hidden)]
pub async fn generate_token() -> Result<(String, String, String), FrameworkError> {
    let (selector, verifier_plaintext) = generate_parts()?;
    // Audit HIGH `hashing` #1 - bcrypt is CPU-bound (~250ms @ cost 12).
    // Use the async variant so the Tokio worker thread isn't blocked
    // while the framework issues a token under request load.
    let verifier_hash = hashing::hash_async(&verifier_plaintext).await?;
    Ok((selector, verifier_plaintext, verifier_hash))
}

/// The input a row's hash is computed over: the verifier, bound to the
/// user's authentication epoch when the user has one (HRD-007).
///
/// The verifier is URL-safe base64 and never holds a `.`, so
/// `"{verifier}.{epoch}"` is unambiguous: no verifier of another row and no
/// other epoch spells the same input. A user without an epoch mixes nothing
/// in, so such a row verifies exactly as a row did before epochs existed.
fn bound_verifier(verifier: &str, epoch: Option<u64>) -> String {
    match epoch {
        Some(epoch) => format!("{verifier}.{epoch}"),
        None => verifier.to_owned(),
    }
}

/// A fresh `(selector, verifier_plaintext)` pair from the OS RNG.
fn generate_parts() -> Result<(String, String), FrameworkError> {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    // High-entropy OS randomness for both halves. `getrandom::fill`
    // is the direct binding to the OS RNG (e.g. `getrandom(2)` on
    // Linux, `BCryptGenRandom` on Windows). rand 0.10 removed `OsRng`
    // from its public surface, so call getrandom directly.
    let mut selector_bytes = [0u8; SELECTOR_BYTES];
    getrandom::fill(&mut selector_bytes)
        .map_err(|e| FrameworkError::internal(format!("OS RNG failure (selector): {e}")))?;
    let selector = URL_SAFE_NO_PAD.encode(selector_bytes);

    let mut verifier_bytes = [0u8; VERIFIER_BYTES];
    getrandom::fill(&mut verifier_bytes)
        .map_err(|e| FrameworkError::internal(format!("OS RNG failure (verifier): {e}")))?;
    let verifier_plaintext = URL_SAFE_NO_PAD.encode(verifier_bytes);
    Ok((selector, verifier_plaintext))
}

/// `ttl_minutes` as a lifetime, and the moment a token issued now with it
/// expires.
///
/// # Errors
///
/// Returns [`FrameworkError`] naming the lifetime when no date can hold
/// it: the minutes overflow a duration, or the expiry runs past the dates
/// the clock represents. `ttl_minutes` comes from configuration and from
/// callers of the public API, so a value that large is an error, not a
/// panic.
pub(crate) fn remember_expiry(
    ttl_minutes: i64,
) -> Result<(Duration, chrono::DateTime<chrono::Utc>), FrameworkError> {
    Duration::try_minutes(ttl_minutes)
        .and_then(|lifetime| {
            crate::clock::now()
                .checked_add_signed(lifetime)
                .map(|expires_at| (lifetime, expires_at))
        })
        .ok_or_else(|| {
            FrameworkError::internal(format!(
                "remember-me lifetime of {ttl_minutes} minutes runs past the dates the clock can hold"
            ))
        })
}

/// Issue a new remember token for `user_id` of the default guard. Inserts
/// a row keyed on a random selector and storing the bcrypt hash of the
/// verifier bound to the user's authentication epoch; returns the
/// composite plaintext `"{selector}.{verifier}"` to the caller. The caller
/// is responsible for shipping the plaintext to the client (via the
/// session middleware's pending-cookies slot, set by
/// `Auth::login_remember`).
///
/// The epoch is the one the default guard's provider reports for the user
/// (HRD-007), so a password change after the issue makes the token fail
/// [`verify_and_rotate`]. Without a registered provider, or for a user that
/// exposes no epoch, the token is bound to none: verification then reads
/// none either.
///
/// # Errors
///
/// Returns [`FrameworkError`] when `ttl_minutes` puts the expiry outside
/// the dates the clock can hold, or when the provider, hashing or the
/// database fails.
pub async fn issue(user_id: &str, ttl_minutes: i64) -> Result<String, FrameworkError> {
    // A lifetime no date can hold is reported before the provider is asked.
    remember_expiry(ttl_minutes)?;
    let guard_name = crate::auth::Auth::default_guard_name();
    // A provider error fails the issue rather than binding the token to no
    // epoch: a token that skipped the binding would survive the next
    // password change.
    let epoch = crate::auth::epoch::guard_user_epoch(&guard_name, user_id).await?;
    issue_for_guard(&guard_name, user_id, ttl_minutes, epoch).await
}

/// Issue a remember token for `user_id` of the guard `guard_name`, bound to
/// `epoch`: the epoch the caller read for the user through the guard's
/// provider, `None` for a user that exposes none.
///
/// Verification ([`verify_and_rotate_for_guard`]) reads the user's epoch
/// through the provider it finds by the guard name the cookie carries
/// ([`crate::auth::epoch::guard_provider`]). A token bound to an epoch is
/// therefore issued only for a guard name that lookup finds: for any other
/// name, such as a `SessionGuard` built by hand under a name the
/// [`AuthManager`](crate::auth::AuthManager) does not know, no verification
/// could ever read the epoch back, and a token bound to none would survive
/// the next password change.
///
/// # Errors
///
/// As [`issue`], and an error that names the guard, with nothing stored,
/// when `epoch` is `Some` and no provider is found for `guard_name`.
pub(crate) async fn issue_for_guard(
    guard_name: &str,
    user_id: &str,
    ttl_minutes: i64,
    epoch: Option<u64>,
) -> Result<String, FrameworkError> {
    if epoch.is_some() && crate::auth::epoch::guard_provider(guard_name).is_none() {
        return Err(FrameworkError::internal(format!(
            "refusing to issue a remember-me credential for the guard \"{guard_name}\": the \
             AuthManager does not know the guard, so the credential's epoch could never be \
             verified. Register the guard with the AuthManager, or sign in through the \
             manager's guard, to use remember-me."
        )));
    }
    issue_at_epoch(user_id, ttl_minutes, epoch).await
}

/// Insert a row for `user_id` whose hash binds a fresh verifier to `epoch`,
/// and return the composite plaintext.
async fn issue_at_epoch(
    user_id: &str,
    ttl_minutes: i64,
    epoch: Option<u64>,
) -> Result<String, FrameworkError> {
    let (_, expires_at) = remember_expiry(ttl_minutes)?;
    let (selector, verifier_plaintext) = generate_parts()?;
    // Audit HIGH `hashing` #1: the async variant keeps the CPU-bound hash
    // off the request worker thread.
    let verifier_hash = hashing::hash_async(&bound_verifier(&verifier_plaintext, epoch)).await?;
    let now = crate::clock::now();

    let conn = DB::connection()?;
    let backend = conn.inner().get_database_backend();
    let model = entity::ActiveModel {
        user_id: Set(user_id.to_string()),
        selector: Set(selector.clone()),
        token_hash: Set(verifier_hash),
        expires_at: Set(storable_expiry(backend, expires_at.naive_utc()).into()),
        created_at: Set(now.into()),
        last_used_at: Set(None),
        ..Default::default()
    };

    entity::Entity::insert(model)
        .exec(conn.inner())
        .await
        .map_err(|e| FrameworkError::database(format!("issue remember token: {e}")))?;

    Ok(format!("{selector}.{verifier_plaintext}"))
}

/// A remember token that verified and was rotated: the user it signs in,
/// the replacement composite plaintext, and the authentication epoch the
/// replacement is bound to, which the session records for the sign-in.
pub(crate) struct RotatedRemember {
    /// The user the token signs in.
    pub(crate) user_id: String,
    /// The replacement `"{selector}.{verifier}"` for the cookie.
    pub(crate) credential: String,
    /// The user's epoch at verification, `None` when the user has none.
    pub(crate) epoch: Option<u64>,
}

/// Verify a plaintext remember-me token of the default guard. On success:
///
/// 1. Atomic conditional DELETE of the matched row keyed on
///    `(id, selector)`. Race-loser sees `rows_affected == 0` and bails.
/// 2. Issue a fresh row for the same user.
/// 3. Return `(user_id, new_composite_plaintext)` so the caller can
///    rotate the cookie.
///
/// On failure (malformed token, no candidate matches, verifier
/// mismatch, or rotation race lost), returns `Ok(None)`. The cookie
/// should then be cleared so the client stops sending a token that can
/// never authenticate.
///
/// The verifier is checked bound to the user's current authentication
/// epoch, read through the default guard's provider (HRD-007). A token
/// issued before a password change, or before 4.0.0 when tokens were bound
/// to no epoch, fails here once its user has an epoch, and its row is
/// deleted: a row whose verifier does not verify is never kept.
///
/// # Verification cost
///
/// Exactly one indexed SELECT, one lookup of the row's user by id, and at
/// most one `bcrypt::verify` per request - independent of how many active
/// tokens the table holds. Forged cookies cost only the SELECT when the
/// selector matches no row or `expires_at` already passed.
pub async fn verify_and_rotate(
    token: &str,
    ttl_minutes: i64,
) -> Result<Option<(String, String)>, FrameworkError> {
    Ok(
        verify_and_rotate_for_guard(token, ttl_minutes, &crate::auth::Auth::default_guard_name())
            .await?
            .map(|rotated| (rotated.user_id, rotated.credential)),
    )
}

/// [`verify_and_rotate`] for a token the guard `guard_name` issued: the
/// user's epoch is read through that guard's provider, the one
/// [`issue_for_guard`] bound the token with, so the session middleware
/// verifies a named guard's cookie against the same epoch it was issued
/// at.
///
/// # A row that fails verification is deleted
///
/// The epoch is not stored beside the hash, so a verifier that does not
/// verify cannot be told apart from one bound to an earlier epoch. The row
/// is deleted either way, and it is safe to: the selector reaches this
/// function only inside the authenticated, encrypted remember cookie the
/// framework issued (or from server code), so a request that presents an
/// existing selector holds the whole credential, and a credential that no
/// longer verifies must not survive. A delete that fails is logged and the
/// token still fails: the row can never verify again, because a password
/// hash once replaced never comes back.
///
/// # Errors
///
/// A database error, or the provider's error while reading the user's
/// epoch. The middleware then continues without remember-me: an epoch that
/// could not be read never counts as "no epoch".
pub(crate) async fn verify_and_rotate_for_guard(
    token: &str,
    ttl_minutes: i64,
    guard_name: &str,
) -> Result<Option<RotatedRemember>, FrameworkError> {
    // Parse the composite token. Malformed → no auth (and no DB hit).
    let (selector, verifier) = match token.split_once('.') {
        Some(pair) => pair,
        None => return Ok(None),
    };

    let conn = DB::connection()?;
    let now = crate::clock::now().naive_utc();

    // O(1) indexed lookup: the UNIQUE constraint on `selector` means
    // this returns 0 or 1 rows.
    let rows = token_rows(
        entity::Entity::find()
            .filter(entity::Column::Selector.eq(selector))
            .filter(entity::Column::ExpiresAt.gt(now)),
    )
    .all(conn.inner())
    .await
    .map_err(|e| FrameworkError::database(format!("look up remember token: {e}")))?;

    let mut rows = rows.into_iter().filter(|row| row.selector == selector);
    let row = match rows.next() {
        Some(r) => r,
        None => return Ok(None),
    };
    if rows.next().is_some() {
        return Err(FrameworkError::database(
            "remember selector matched multiple exact rows",
        ));
    }

    // The epoch the row must be bound to: the user's current one, read
    // through the provider of the guard the cookie names.
    let epoch = crate::auth::epoch::guard_user_epoch(guard_name, &row.user_id).await?;

    // Exactly one `bcrypt::verify` per request - constant-time
    // comparison, no scanning. Audit HIGH `hashing` #1: the async
    // variant runs the CPU-bound bcrypt verification on
    // `spawn_blocking` so the request worker thread stays free.
    if !hashing::verify_async(&bound_verifier(verifier, epoch), &row.token_hash).await? {
        let deleted = entity::Entity::delete_many()
            .filter(entity::Column::Id.eq(row.id))
            .filter(entity::Column::Selector.eq(&row.selector))
            .exec(conn.inner())
            .await;
        if let Err(error) = deleted {
            tracing::warn!(
                target: "suprnova::auth",
                user_id = %row.user_id,
                %error,
                "a remember-me row that failed verification could not be deleted; \
                 it can no longer verify"
            );
        }
        return Ok(None);
    }

    // Atomic conditional DELETE - succeeds for exactly one concurrent
    // verifier of this token. If two requests both reach this point,
    // exactly one DELETE affects 1 row and the other affects 0. The
    // loser MUST NOT issue a fresh token: that would defeat the
    // single-use rotation invariant by minting two replacements for
    // the same captured cookie.
    let delete_result = entity::Entity::delete_many()
        .filter(entity::Column::Id.eq(row.id))
        .filter(entity::Column::Selector.eq(&row.selector))
        .exec(conn.inner())
        .await
        .map_err(|e| FrameworkError::database(format!("delete remember token: {e}")))?;

    if delete_result.rows_affected != 1 {
        // Lost the rotation race. Another concurrent request already
        // deleted this row and minted a fresh one. Treat this attempt
        // as "no auth" - replay defeated.
        return Ok(None);
    }

    // The replacement is bound to the epoch just verified, so the rotation
    // reads the provider once.
    let credential = issue_at_epoch(&row.user_id, ttl_minutes, epoch).await?;
    Ok(Some(RotatedRemember {
        user_id: row.user_id,
        credential,
        epoch,
    }))
}

/// Revoke every remember token for `user_id` in one DELETE.
///
/// Called from `Auth::logout` so a logout actually clears persistent
/// re-auth state. Also the right hook for a "log me out everywhere"
/// account-security button.
pub async fn revoke_all_for_user(user_id: &str) -> Result<u64, FrameworkError> {
    let conn = DB::connection()?;
    let result = entity::Entity::delete_many()
        .filter(entity::Column::UserId.eq(user_id))
        .exec(conn.inner())
        .await
        .map_err(|e| FrameworkError::database(format!("revoke remember tokens: {e}")))?;
    Ok(result.rows_affected)
}

pub(crate) async fn revoke_other_for_user(
    user_id: &str,
    current_selector: Option<&str>,
) -> Result<u64, FrameworkError> {
    let conn = DB::connection()?;
    let mut delete = entity::Entity::delete_many().filter(entity::Column::UserId.eq(user_id));
    if let Some(selector) = current_selector {
        delete = delete.filter(entity::Column::Selector.ne(selector));
    }
    let result = delete
        .exec(conn.inner())
        .await
        .map_err(|e| FrameworkError::database(format!("revoke other remember tokens: {e}")))?;
    Ok(result.rows_affected)
}

/// Revoke exactly one remember token by owner and non-secret selector.
pub(crate) async fn revoke_by_selector(
    user_id: &str,
    selector: &str,
) -> Result<bool, FrameworkError> {
    let conn = DB::connection()?;
    let transaction = conn.inner().begin().await.map_err(|error| {
        FrameworkError::database(format!("begin remember selector revocation: {error}"))
    })?;
    let result = async {
        let rows = token_rows(
            entity::Entity::find()
                .filter(entity::Column::UserId.eq(user_id))
                .filter(entity::Column::Selector.eq(selector)),
        )
        .all(&transaction)
        .await
        .map_err(|error| {
            FrameworkError::database(format!(
                "find remember token for selector revocation: {error}"
            ))
        })?;
        let mut rows = rows
            .into_iter()
            .filter(|row| row.user_id == user_id && row.selector == selector);
        let Some(row) = rows.next() else {
            return Ok(0);
        };
        if rows.next().is_some() {
            return Err(FrameworkError::database(
                "remember selector matched multiple exact rows",
            ));
        }
        entity::Entity::delete_many()
            .filter(entity::Column::Id.eq(row.id))
            .exec(&transaction)
            .await
            .map(|deleted| deleted.rows_affected)
            .map_err(|error| {
                FrameworkError::database(format!("revoke remember token by selector: {error}"))
            })
    }
    .await;
    match result {
        Ok(1) => {
            transaction.commit().await.map_err(|error| {
                FrameworkError::database(format!("commit remember selector revocation: {error}"))
            })?;
            Ok(true)
        }
        Ok(0) => {
            transaction.rollback().await.map_err(|error| {
                FrameworkError::database(format!("rollback remember selector revocation: {error}"))
            })?;
            Ok(false)
        }
        Ok(_) => {
            transaction.rollback().await.map_err(|error| {
                FrameworkError::database(format!("rollback remember selector revocation: {error}"))
            })?;
            Err(FrameworkError::database(
                "remember selector matched multiple rows",
            ))
        }
        Err(error) => match transaction.rollback().await {
            Ok(()) => Err(error),
            Err(rollback_error) => Err(FrameworkError::database(format!(
                "revoke remember token by selector failed: {error}; rollback failed: {rollback_error}"
            ))),
        },
    }
}

/// Delete a single remember-token row by id. Useful for "log out this
/// device" UIs where the user picks a specific session.
pub async fn revoke_by_id(id: i64) -> Result<bool, FrameworkError> {
    let conn = DB::connection()?;
    let result = entity::Entity::delete_by_id(id)
        .exec(conn.inner())
        .await
        .map_err(|e| FrameworkError::database(format!("revoke remember token by id: {e}")))?;
    Ok(result.rows_affected == 1)
}

/// Delete all rows whose `expires_at` is in the past. Returns the
/// number of rows removed.
///
/// Wire this up to a scheduled task (see `framework/src/schedule/`) so
/// the table does not accumulate dead rows.
pub async fn prune_expired() -> Result<u64, FrameworkError> {
    let conn = DB::connection()?;
    let now = crate::clock::now().naive_utc();
    let result = entity::Entity::delete_many()
        .filter(entity::Column::ExpiresAt.lte(now))
        .exec(conn.inner())
        .await
        .map_err(|e| FrameworkError::database(format!("prune remember tokens: {e}")))?;
    Ok(result.rows_affected)
}

/// SeaORM entity for the `remember_tokens` table.
///
/// Schema mirrors the migration consumer apps ship (see
/// `app/src/migrations/m20251208_230000_create_remember_tokens_table.rs`
/// and the corresponding CLI scaffolder template):
///
/// - `id`           INTEGER PK auto-increment
/// - `user_id`      VARCHAR not null - opaque string id (post-Phase-3 String-everywhere)
/// - `selector`     VARCHAR not null UNIQUE - 22-char URL-safe base64 lookup key
/// - `token_hash`   VARCHAR not null - bcrypt hash of the verifier plaintext
/// - `expires_at`   DATETIME not null - token TTL boundary
/// - `created_at`   DATETIME not null
/// - `last_used_at` DATETIME null - currently informational (rotation deletes the row before update)
///
/// Older scaffolds created the time columns as `TIMESTAMP` on MySQL and
/// MariaDB, and a table may have `timestamptz` on Postgres. The fields are
/// [`StoredDateTime`](crate::database::StoredDateTime), which reads each of
/// those as well as `DATETIME`, `timestamp` and SQLite text, so a
/// whole-row read through this entity works on every one.
pub mod entity {
    use sea_orm::entity::prelude::*;

    /// SeaORM model for a single row in `remember_tokens`.
    #[derive(Clone, Debug, PartialEq, Eq, DeriveEntityModel)]
    #[sea_orm(table_name = "remember_tokens")]
    pub struct Model {
        /// Auto-increment primary key.
        #[sea_orm(primary_key)]
        pub id: i64,
        /// Opaque string id of the user the token authenticates.
        pub user_id: String,
        /// 22-char URL-safe base64 lookup key (the public half of the token).
        #[sea_orm(unique)]
        pub selector: String,
        /// Bcrypt hash of the verifier - the private half of the token.
        pub token_hash: String,
        /// TTL boundary; the cookie is rejected once `now > expires_at`.
        pub expires_at: crate::database::StoredDateTime,
        /// Wall-clock time the token row was created.
        pub created_at: crate::database::StoredDateTime,
        /// Informational; current rotation strategy deletes the row instead of updating this.
        pub last_used_at: Option<crate::database::StoredDateTime>,
    }

    /// SeaORM relation enum - `remember_tokens` is a leaf table with no
    /// declared foreign-key relations.
    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `generate_token` produces a fresh selector + verifier + matching
    /// hash on each call.
    #[tokio::test]
    async fn generate_token_round_trips_through_bcrypt() {
        let (sel, ver, hash) = generate_token().await.expect("token gen");
        // 16 bytes → 22 base64 chars, 32 bytes → 43 base64 chars.
        assert_eq!(sel.len(), 22, "selector is 22 base64 chars (16 bytes)");
        assert_eq!(ver.len(), 43, "verifier is 43 base64 chars (32 bytes)");
        assert!(hash.starts_with("$2"), "bcrypt hash prefix expected");
        assert!(
            hashing::verify(&ver, &hash).expect("verify"),
            "freshly generated verifier must verify against its hash"
        );

        // Different generations must produce different selectors AND
        // different verifiers.
        let (sel2, ver2, _h2) = generate_token().await.expect("token gen 2");
        assert_ne!(sel, sel2, "selectors must not collide");
        assert_ne!(ver, ver2, "verifiers must not collide");
        assert!(
            !hashing::verify(&ver2, &hash).expect("verify"),
            "wrong verifier must not verify"
        );
    }

    /// A lifetime no date can hold is an error, not a panic: `i64::MAX`
    /// minutes overflows the duration, and a smaller one that fits the
    /// duration still runs past the last date chrono represents.
    #[tokio::test]
    async fn issue_refuses_a_lifetime_no_date_can_hold() {
        for ttl_minutes in [i64::MAX, i64::MIN, 100_000_000_000_000] {
            let outcome = tokio::spawn(issue("overflow-user", ttl_minutes)).await;
            // The error names the lifetime: no database is registered here,
            // so any other error would come from a later step.
            assert!(
                matches!(&outcome, Ok(Err(error)) if error.to_string().contains("lifetime")),
                "ttl_minutes {ttl_minutes} must return an error, got {outcome:?}"
            );
        }
    }
}
