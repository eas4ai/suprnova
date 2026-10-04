//! Two-Factor Authentication via TOTP (RFC 6238).
//!
//! Stores per-user TOTP secrets and recovery codes in the
//! framework-owned `two_factor_credentials` table. Secrets and
//! recovery codes are encrypted at rest via [`crate::crypto::Crypt`].
//!
//! # Flow
//!
//! 1. [`TwoFactor::enroll`] generates a fresh secret + 10 recovery
//!    codes, persists them encrypted, and returns the otpauth URL, a
//!    QR-code SVG, and the plaintext recovery codes (shown to the
//!    user once and only once).
//! 2. [`TwoFactor::confirm`] sets `confirmed_at` after the user
//!    submits a valid TOTP code from their authenticator app.
//!    Required before [`TwoFactor::verify`] / [`TwoFactor::is_enabled`]
//!    treat 2FA as active.
//! 3. [`TwoFactor::verify`] checks a TOTP code on subsequent logins.
//! 4. [`TwoFactor::consume_recovery_code`] consumes a single recovery
//!    code; subsequent attempts against the same code return false.
//! 5. [`TwoFactor::disable`] removes the row entirely.
//!
//! The user identity is opaque to this module - callers pass any
//! stringy `user_id` (typically `UserId::to_string()`). There
//! is no FK to a user table.

mod attempts;
pub mod entity;
pub mod lockout;
pub mod migration;
pub mod migration_attempts;
pub mod migration_replay;
pub mod migration_rotation;
pub mod recovery;
mod rotation;

#[cfg(any(
    feature = "database-sqlite",
    feature = "database-postgres",
    feature = "database-mysql"
))]
use crate::auth_flows::events::{TwoFactorChallengeFailed, TwoFactorChallenged};
use crate::auth_flows::events::{TwoFactorDisabled, TwoFactorEnrolled};
use crate::crypto::Crypt;
use crate::database::DB;
use crate::error::FrameworkError;
use sea_orm::sea_query::Expr;
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, Condition, EntityTrait, QueryFilter,
};
use totp_rs::{Algorithm, Secret, TOTP};

pub use lockout::TwoFactorLockout;

const ISSUER_ENV: &str = "APP_NAME";
const DEFAULT_ISSUER: &str = "Suprnova";
const RECOVERY_CODE_COUNT: usize = 10;

/// Forward-skew window the TOTP construction in [`check_code`] accepts
/// (`skew=1`). The replay-claim stamp uses `current + TOTP_SKEW_STEPS`
/// so the next-timestep replay of the same code is rejected - a bare
/// `current` stamp leaves a one-step gap where the captured code still
/// validates and the predicate `current_step <= last` does not yet
/// fire. See [`TwoFactor::verify`]'s replay-protection comment.
const TOTP_SKEW_STEPS: i64 = 1;

/// Static facade for the 2FA TOTP lifecycle. See module docs.
pub struct TwoFactor;

/// Successful enrollment payload returned from [`TwoFactor::enroll`].
///
/// The recovery codes are plaintext and MUST be displayed to the user
/// exactly once - there is no API for retrieving them again after the
/// response is dropped.
///
/// The `Debug` impl is hand-written rather than derived so a stray
/// `dbg!()` or `tracing::info!(?response)` does not leak the
/// `otpauth_url` (which contains the raw shared secret in its
/// `secret=` query parameter) or the plaintext recovery codes.
/// Pattern mirrors [`crate::EncryptionKey`]'s redacting `Debug`.
#[derive(Clone)]
pub struct EnrollmentResponse {
    /// `otpauth://totp/...` URL suitable for QR-app deep linking.
    pub otpauth_url: String,
    /// SVG wrapping a base64-encoded PNG QR code; safe to embed in
    /// HTML via `{{ enrollment.qr_code_svg | safe }}`.
    pub qr_code_svg: String,
    /// Ten single-use recovery codes. Plaintext - show once.
    pub recovery_codes: Vec<String>,
}

impl std::fmt::Debug for EnrollmentResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The otpauth URL embeds the shared secret in its query
        // string - treat the whole URL as opaque, just like the
        // recovery codes. The QR-code SVG is derived from the same
        // secret and equally sensitive, so it gets the same redacting
        // treatment (length is non-secret and useful for debugging
        // "did the QR encoder run?").
        f.debug_struct("EnrollmentResponse")
            .field("otpauth_url", &"[REDACTED]")
            .field(
                "qr_code_svg",
                &format!("[REDACTED; {} bytes]", self.qr_code_svg.len()),
            )
            .field(
                "recovery_codes",
                &format!("[REDACTED; {} codes]", self.recovery_codes.len()),
            )
            .finish()
    }
}

/// Minimum contract for a user the [`TwoFactor`] facade can act on.
///
/// `user_id` is the opaque storage key (e.g.
/// `UserId::to_string()`) and `email` is folded into the
/// `otpauth://` `account_name` segment so authenticator apps render
/// the row with a human-readable label.
pub trait TwoFactorUser: Send + Sync {
    /// Opaque storage key for this user (e.g. `UserId::to_string()`).
    fn user_id(&self) -> &str;
    /// Email folded into the `otpauth://` `account_name` segment so
    /// authenticator apps render a human-readable label.
    fn email(&self) -> &str;
}

impl TwoFactor {
    /// Begin enrollment: generate a fresh secret + 10 recovery codes,
    /// persist them encrypted, and return the otpauth URL, a QR-code
    /// SVG, and the plaintext recovery codes (shown to the user once).
    ///
    /// The user must call [`Self::confirm`] with a valid TOTP code
    /// before 2FA actually gates logins - until then
    /// [`Self::is_enabled`] returns `false` and [`Self::verify`]
    /// short-circuits to `Ok(false)`.
    ///
    /// # Errors
    ///
    /// Returns `FrameworkError::domain(.., 409)` when the user
    /// **already has a confirmed 2FA enrollment**. Overwriting a
    /// confirmed secret without proof of the existing one would let a
    /// session-hijacked attacker pivot from "I have a session" to "I
    /// have 2FA on this account." Call [`Self::re_enroll`] with a
    /// valid TOTP code or recovery code as proof.
    ///
    /// Re-enrolling on an unconfirmed (pending) row is allowed - the
    /// prior enrollment never became authoritative. The check is part of
    /// the write itself, so a confirmation that lands while `enroll` runs
    /// also gets the `409`.
    pub async fn enroll<U: TwoFactorUser>(user: &U) -> Result<EnrollmentResponse, FrameworkError> {
        // No separate "already enabled?" read: the write refuses a
        // confirmed row in the same statement that replaces a pending one,
        // so a confirmation cannot land between a check and the write.
        let (response, encrypted_secret, encrypted_recovery) = Self::new_secret(user)?;
        write_enrollment_row(user.user_id(), encrypted_secret, encrypted_recovery).await?;
        Ok(response)
    }

    /// Rotate the secret for an existing confirmed 2FA enrollment.
    /// Requires either a valid current TOTP code or an unused
    /// recovery code as proof of possession.
    ///
    /// On success, a fresh secret + 10 fresh recovery codes wait as a
    /// pending rotation until [`Self::confirm`] proves a code from the new
    /// secret. Until then the confirmed secret and its recovery codes keep
    /// gating sign-in, so a rotation nobody finishes never leaves the
    /// account without a second factor, and [`Self::enroll`], which takes
    /// no proof, cannot replace the pending secret. A second `re_enroll`
    /// replaces the pending rotation. The rotation waits in
    /// `two_factor_rotations`, which
    /// [`migration_rotation::Migration`] creates.
    ///
    /// # Errors
    ///
    /// - `FrameworkError::domain(.., 401)` when `proof` validates as
    ///   neither a TOTP code (current or within the replay-protected
    ///   window) nor a recovery code.
    /// - `FrameworkError::domain(.., 400)` when no confirmed
    ///   enrollment exists - call [`Self::enroll`] instead.
    /// - `FrameworkError::domain(.., 429)` while wrong codes have locked
    ///   the second factor (see [`Self::verify`]), and `.., 503` when the
    ///   attempt store or the rotation store fails.
    pub async fn re_enroll<U: TwoFactorUser>(
        user: &U,
        proof: &str,
    ) -> Result<EnrollmentResponse, FrameworkError> {
        if !Self::is_enabled(user).await? {
            return Err(FrameworkError::domain(
                "no confirmed 2FA enrollment to rotate; call enroll first",
                400,
            ));
        }

        // One reserved attempt covers both proof forms, so a bad proof
        // counts once, and a locked account is refused before the proof is
        // read.
        let proof_accepted = settle_attempt(user.user_id(), user.email(), || {
            Self::verify_totp_or_recovery(user, proof)
        })
        .await?;
        if !proof_accepted {
            return Err(FrameworkError::domain(
                "re-enrollment proof is neither a valid TOTP code nor a recovery code",
                401,
            ));
        }

        let (response, encrypted_secret, encrypted_recovery) = Self::new_secret(user)?;
        rotation::store(user.user_id(), encrypted_secret, encrypted_recovery).await?;
        Ok(response)
    }

    /// Internal helper - mint a fresh secret and recovery codes. Returns the
    /// one-time artifacts and their encrypted forms, for [`Self::enroll`]
    /// (a pending enrollment) and [`Self::re_enroll`] (a pending rotation).
    fn new_secret<U: TwoFactorUser>(
        user: &U,
    ) -> Result<(EnrollmentResponse, String, String), FrameworkError> {
        let secret_bytes = Secret::generate_secret()
            .to_bytes()
            .map_err(|e| FrameworkError::internal(format!("totp secret bytes: {e}")))?;
        let secret_b32 = Secret::Raw(secret_bytes.clone()).to_encoded().to_string();

        let issuer = std::env::var(ISSUER_ENV).unwrap_or_else(|_| DEFAULT_ISSUER.into());
        let totp = TOTP::new(
            Algorithm::SHA1,
            6,
            1,
            30,
            secret_bytes,
            Some(issuer),
            user.email().to_string(),
        )
        .map_err(|e| FrameworkError::internal(format!("totp new: {e}")))?;

        let otpauth_url = totp.get_url();
        let qr_b64 = totp
            .get_qr_base64()
            .map_err(|e| FrameworkError::internal(format!("totp qr: {e}")))?;
        let qr_code_svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\">\
             <image href=\"data:image/png;base64,{qr_b64}\" width=\"256\" height=\"256\"/></svg>"
        );

        let recovery_codes = recovery::generate(RECOVERY_CODE_COUNT);
        let encrypted_secret =
            Crypt::encrypt_string(crate::crypto::CryptPurpose::TwoFactorSecret, &secret_b32)?;
        let encrypted_recovery = Crypt::encrypt_string(
            crate::crypto::CryptPurpose::TwoFactorRecovery,
            &recovery_codes.join("\n"),
        )?;

        Ok((
            EnrollmentResponse {
                otpauth_url,
                qr_code_svg,
                recovery_codes,
            },
            encrypted_secret,
            encrypted_recovery,
        ))
    }

    /// Confirm a pending enrollment with a TOTP code from the user's
    /// authenticator app. On success, stamps `confirmed_at` and
    /// dispatches [`TwoFactorEnrolled`].
    ///
    /// The confirmation stamps exactly the enrollment the code was checked
    /// against. When a concurrent [`Self::enroll`] replaces the secret
    /// between the check and the stamp, nothing is confirmed: the code
    /// proved possession of the old secret, not of the new one.
    ///
    /// # Errors
    ///
    /// - `FrameworkError::domain(.., 401)` if no row exists for this
    ///   user, or if the supplied code does not match.
    /// - `FrameworkError::domain(.., 409)` if the enrollment was replaced
    ///   or removed while the code was being checked, or is already
    ///   confirmed.
    /// - `FrameworkError::domain(.., 429)` while wrong codes have locked
    ///   the second factor (see [`Self::verify`]), and `.., 503` when the
    ///   attempt store fails before the confirmation commits. A failure to
    ///   settle the attempt after it commits is logged, not returned: the
    ///   second factor is live, and [`TwoFactorEnrolled`] fires.
    pub async fn confirm<U: TwoFactorUser>(user: &U, code: &str) -> Result<(), FrameworkError> {
        let enrollment = load_secret(user.user_id())
            .await?
            .ok_or_else(|| FrameworkError::domain("no pending 2FA enrollment", 401))?;
        // A confirmed row has nothing to confirm unless a proven rotation
        // waits; the code is then checked against the rotation's secret.
        let pending_rotation = if enrollment.confirmed {
            match rotation::find(user.user_id()).await? {
                Some(rotation) => Some(rotation),
                None => {
                    return Err(FrameworkError::domain(
                        "2FA is already confirmed for this account",
                        409,
                    ));
                }
            }
        } else {
            None
        };
        let checked_secret = pending_rotation
            .as_ref()
            .map_or(enrollment.secret_b32.as_str(), |rotation| {
                rotation.secret_b32.as_str()
            });

        // Confirmation is throttled like every other code-checking path:
        // without it the 6-digit TOTP of a pending enrollment could be
        // ground online.
        let attempt = ProofAttempt::admit(user.user_id(), user.email()).await?;
        let stamped = async {
            let now = crate::clock::now();
            if !check_code(checked_secret, code, now.timestamp())? {
                return Ok(false);
            }
            let timestep = totp_timestep_at(now.timestamp());
            let committed = match &pending_rotation {
                None => {
                    stamp_confirmation(user.user_id(), &enrollment.ciphertext, timestep, now)
                        .await?
                }
                Some(rotation) => {
                    rotation::promote(
                        user.user_id(),
                        &enrollment.ciphertext,
                        &rotation.ciphertext,
                        timestep,
                        now,
                    )
                    .await?
                }
            };
            if !committed {
                return Err(FrameworkError::domain(
                    "the 2FA enrollment changed while it was being confirmed; confirm a code from the current enrollment",
                    409,
                ));
            }
            Ok(true)
        }
        .await;
        match stamped {
            Ok(true) => {
                // The confirmation has committed. A failure to settle the
                // attempt after it must not report the live second factor as
                // not enabled: the reservation stays counted until it ages
                // out, which errs toward a lock, and the error is logged.
                if let Err(error) = attempt.accepted().await {
                    tracing::error!(
                        error = %error,
                        "a two-factor confirmation committed but its attempt could not be settled; \
                         the attempt stays counted until the window passes"
                    );
                }
            }
            Ok(false) => {
                attempt.rejected().await?;
                return Err(FrameworkError::domain("invalid 2FA code", 401));
            }
            Err(error) => return Err(attempt.abandon(error).await),
        }

        // Discard dispatch errors - the confirmation has already
        // committed; a downstream listener failure must not surface
        // here. The dispatcher logs listener errors via tracing.
        let _ = crate::events::EventFacade::dispatch(TwoFactorEnrolled {
            user_id: user.user_id().to_string(),
        })
        .await;

        Ok(())
    }

    /// Verify a TOTP code for a user with a confirmed enrollment.
    ///
    /// Returns `Ok(false)` when 2FA is not enabled (no row, or row
    /// exists but `confirmed_at` is NULL) or the code does not match.
    /// Storage failures surface as `Err`.
    ///
    /// # Replay protection
    ///
    /// On a successful verify the row is stamped with `claim_to`,
    /// computed as `current_timestep + TOTP_SKEW_STEPS` (= `current + 1`,
    /// matching the `skew=1` the underlying `check_code` TOTP
    /// construction accepts). Subsequent verifications where
    /// `current_timestep <= last_used_timestep` are rejected even when
    /// the code itself is structurally valid.
    ///
    /// Stamping the *forward* edge of the skew window is what blocks
    /// the next-timestep replay that a bare `current_timestep` stamp
    /// would miss. An attacker who observes a code at server time T
    /// cannot replay it at T+1, even though `check_code` would still
    /// accept the same code in that window.
    ///
    /// The legitimate cost is that a user who needs to 2FA twice across
    /// the same skew window must wait until the next one - for the
    /// typical "verify once per login" flow this is invisible.
    ///
    /// The timestep claim is atomic. The stamp is written with a
    /// conditional `UPDATE ... WHERE last_used_timestep IS NULL OR
    /// last_used_timestep < :current`, and the verify only succeeds
    /// when that statement affects exactly one row. Two concurrent
    /// verifies therefore cannot both win, even when they straddle a
    /// timestep boundary: the first flips the column, the second's
    /// predicate no longer matches and it is treated as a replay. A plain read-modify-write
    /// would be a TOCTOU race - both verifies read the pre-stamp row,
    /// both validate the same code, both stamp - that silently defeats
    /// the guard under concurrency.
    ///
    /// # Brute-force throttling
    ///
    /// Each call reserves one attempt in the second-factor counter before
    /// the code is read - the counter every proof path of this facade
    /// shares, [`Self::complete_challenge`] included. A wrong code, a
    /// replay or a lost claim race turns the reservation into a failed
    /// attempt. The configured number of failures inside the configured
    /// window ([`TwoFactorLockout`]: five in fifteen minutes by default)
    /// lock the second factor
    /// until they age out or [`Self::unlock`] clears them. A locked user is
    /// refused before any code is evaluated, so the right code cannot open
    /// it either, and parallel guesses cannot all pass one status read. A
    /// successful verify clears the failures.
    ///
    /// The counter is the framework's `two_factor_attempts` table, keyed by
    /// the user id, not the per-email password counter: a successful
    /// password check does not clear second-factor failures, and the lock
    /// works with no Magnetar engine installed.
    ///
    /// # Errors
    ///
    /// - [`FrameworkError::domain`] with status `429` when the account is
    ///   locked by brute-force throttling.
    /// - [`FrameworkError::domain`] with status `503` when the attempt
    ///   store cannot reserve or settle the attempt, including after a
    ///   correct code whose success could not be recorded.
    /// - Storage and decryption failures.
    pub async fn verify<U: TwoFactorUser>(user: &U, code: &str) -> Result<bool, FrameworkError> {
        if !Self::is_enabled(user).await? {
            return Ok(false);
        }
        settle_attempt(user.user_id(), user.email(), || {
            Self::verify_internal(user, code)
        })
        .await
    }

    /// Try to consume one recovery code. Returns `true` if a code
    /// matched and was removed (single-use), `false` if no row, no
    /// codes, no match, or the enrollment has not been confirmed yet.
    ///
    /// Recovery codes are a backup for an **active** 2FA enrollment,
    /// so this method short-circuits to `Ok(false)` while
    /// `confirmed_at` is NULL - matching [`Self::verify`]'s symmetry.
    /// Without this gate, an attacker who triggered enrollment on a
    /// victim account (or any flow that creates the row without
    /// confirming) could authenticate using only a fresh recovery
    /// code, bypassing TOTP entirely.
    ///
    /// Brute-force throttling is the same as [`Self::verify`]'s: the
    /// attempt is reserved before the code is read, a wrong code counts
    /// as a failed attempt, and a locked account is refused before any
    /// code is consumed.
    ///
    /// # Errors
    ///
    /// The same as [`Self::verify`]: `429` while the account is locked,
    /// `503` when the lockout store fails, and storage failures.
    pub async fn consume_recovery_code<U: TwoFactorUser>(
        user: &U,
        code: &str,
    ) -> Result<bool, FrameworkError> {
        if !Self::is_enabled(user).await? {
            return Ok(false);
        }
        settle_attempt(user.user_id(), user.email(), || {
            recovery::consume(user.user_id(), code)
        })
        .await
    }

    /// Internal silent variant of [`Self::verify`]: runs the full
    /// security pipeline (replay-window check, code match, atomic
    /// timestep claim) but **does not** touch the brute-force
    /// counter. Used by [`Self::complete_challenge`] which needs to
    /// try TOTP and recovery in the same submission without double-
    /// counting a single failed attempt as two against the BF
    /// counter.
    ///
    /// Returns `Ok(false)` for any rejection - no-row, not-confirmed,
    /// replay, mismatch, or race lost. Callers that need to
    /// distinguish "not enabled" from "wrong code" can re-check
    /// [`Self::is_enabled`] (a single DB read).
    async fn verify_internal<U: TwoFactorUser>(
        user: &U,
        code: &str,
    ) -> Result<bool, FrameworkError> {
        let Some(current_timestep) = prepare_totp_claim(user.user_id(), code).await? else {
            return Ok(false);
        };
        claim_totp_timestep(user.user_id(), current_timestep).await
    }

    /// Internal silent variant of [`Self::consume_recovery_code`]:
    /// consumes the code single-use if it matches but **does not**
    /// touch the brute-force counter. Used by
    /// [`Self::complete_challenge`] for the same reason
    /// [`Self::verify_internal`] exists.
    async fn consume_recovery_internal<U: TwoFactorUser>(
        user: &U,
        code: &str,
    ) -> Result<bool, FrameworkError> {
        if !Self::is_enabled(user).await? {
            return Ok(false);
        }
        recovery::consume(user.user_id(), code).await
    }

    /// Check `proof` as a TOTP code, then as an unused recovery code,
    /// without touching the attempt counter. The caller settles the one
    /// attempt that covers both forms.
    async fn verify_totp_or_recovery<U: TwoFactorUser>(
        user: &U,
        proof: &str,
    ) -> Result<bool, FrameworkError> {
        if Self::verify_internal(user, proof).await? {
            return Ok(true);
        }
        Self::consume_recovery_internal(user, proof).await
    }

    /// Returns `true` when an active (confirmed) 2FA enrollment
    /// exists for this user. Sugar over [`Self::is_enabled_by_id`]
    /// for callers that already hold a [`TwoFactorUser`].
    pub async fn is_enabled<U: TwoFactorUser>(user: &U) -> Result<bool, FrameworkError> {
        Self::is_enabled_by_id(user.user_id()).await
    }

    /// Returns `true` when an active (confirmed) 2FA enrollment
    /// exists for `user_id`.
    ///
    /// String-id variant of [`Self::is_enabled`]. The underlying query
    /// never touches the user's email, so callers that have a bare
    /// user-id string (e.g. inside a login handler after `Auth::attempt`
    /// returns) don't need to construct a [`TwoFactorUser`] just to
    /// answer "should this login go through a challenge?"
    pub async fn is_enabled_by_id(user_id: &str) -> Result<bool, FrameworkError> {
        let db = DB::connection()?;
        let row = entity::Entity::find_by_id(user_id.to_string())
            .one(db.inner())
            .await
            .map_err(|e| FrameworkError::internal(format!("two_factor find: {e}")))?;
        Ok(matches!(row, Some(r) if r.confirmed_at.is_some()))
    }

    /// Begin a 2FA challenge for `user_id`: revoke the user's
    /// remember-me tokens, clear the fully-authenticated session slot
    /// (if any), stash `user_id` as the pending user, and remember
    /// whether the user opted into remember-me at password-login
    /// time. The caller - typically a password-login handler that
    /// just resolved a user for whom [`Self::is_enabled_by_id`]
    /// returned `true` - should then redirect to the challenge page.
    /// The session remains in pending state until
    /// `Self::complete_challenge` succeeds (promoting pending →
    /// authed) or the user explicitly cancels via
    /// [`Self::cancel_challenge`].
    ///
    /// `Auth::id()` returns `None` while a challenge is pending, so
    /// any route gated by [`crate::AuthMiddleware`] keeps the user
    /// out. Compose
    /// [`crate::auth_flows::TwoFactorChallengeMiddleware`] in front
    /// of `AuthMiddleware` to redirect pending users to the
    /// challenge page rather than letting them fall through to the
    /// login page.
    ///
    /// # Arguments
    ///
    /// * `user_id` - the id of the user whose password verified.
    /// * `remember` - whether the original login form requested
    ///   remember-me. The preference is stashed in the session and
    ///   consumed by `Self::complete_challenge`, which re-issues
    ///   the remember-me cookie on a successful challenge. Pass the
    ///   exact `remember` value the caller received from the login
    ///   form - `false` if the form had no remember-me checkbox.
    ///
    /// # Why revoke remember-me
    ///
    /// `Auth::attempt(creds, true)` issues the remember-me cookie
    /// and database row **before** the login handler sees the
    /// result and decides to gate on 2FA. Without an explicit revoke
    /// here, that cookie would outlive the demotion to pending: a
    /// user who closed their browser before completing the
    /// challenge would be auto-logged-in on the next visit via
    /// remember-me, bypassing 2FA entirely.
    ///
    /// # Fail-closed ordering
    ///
    /// We save `Auth::id()` to a local, **then** clear the session
    /// auth slot, **then** revoke remember-me against the saved id
    /// via the auth guard's remember-token revoke API. The reordering
    /// matters: if the revoke errors (DB transient failure, lock
    /// timeout), `start_challenge` returns `Err` - but the session
    /// is already in a safe state (`auth_user_id` cleared, pending
    /// set), so `AuthMiddleware` kicks the user to `/login` rather
    /// than letting them through as fully authenticated. An earlier
    /// implementation ran revoke first, then cleared auth - a
    /// transient revoke failure there left the session
    /// fully-authed bypassing the 2FA gate.
    ///
    /// The cookie + row that `start_challenge` revokes are the
    /// pre-challenge ones. If `remember` is `true`,
    /// `Self::complete_challenge` issues a **fresh** cookie + row
    /// on the post-challenge user-id, so remember-me-with-2FA users
    /// get the same UX as remember-me-without-2FA users without the
    /// caller having to remember to re-issue the cookie themselves.
    pub async fn start_challenge(
        user_id: impl Into<String>,
        remember: bool,
    ) -> Result<(), FrameworkError> {
        let user_id = user_id.into();
        // Capture the currently-authenticated id (if any) BEFORE we
        // tear down the auth slot - the revoke needs it to identify
        // whose remember-me rows to delete, but the slot has to go
        // first for fail-closed safety.
        let saved_id = crate::auth::Auth::id();

        // STEP 1: Tear down auth state. Pending and authed are
        // mutually exclusive - clear the auth slot, the request-
        // scoped current user, and install the pending slot. If any
        // subsequent step errors, the session is now in a safe
        // state (no `auth_user_id` → `AuthMiddleware` kicks to
        // /login, no bypass through stale session state).
        crate::session::middleware::clear_auth_user();
        crate::auth::request_state::clear_current_user();
        crate::session::middleware::set_two_factor_pending(user_id.clone());
        crate::session::middleware::set_two_factor_pending_remember(remember);

        // STEP 2: Revoke remember-me using the saved id. `Auth::id()`
        // is now `None` (we just cleared the slot), which is why we
        // can't use the bare `Auth::revoke_remember_tokens()` here -
        // it would no-op on the missing id and leave the row in
        // place. `_for_user` takes the id explicitly.
        if let Some(id) = saved_id {
            crate::auth::Auth::revoke_remember_tokens_for_user(&id).await?;
        } else {
            // No prior auth - queue a clear cookie anyway in case
            // the browser still holds a stale one from another
            // session. The cookie attributes match
            // `revoke_remember_tokens` so behaviour is symmetric.
            // A failed push (no scope) is harmless here: no DB state
            // depends on this cookie reaching the response.
            let config = crate::session::middleware::current_session_config();
            let clear = crate::session::middleware::create_forget_remember_cookie(&config);
            let _ = crate::session::middleware::push_pending_cookie(clear);
        }

        // STEP 3: With the Magnetar engine installed, the promoted login
        // will need a Magnetar session. Record the auth epoch the password
        // was checked at, so a password reset or sign-out-everywhere during
        // the challenge cancels it, and refuse now an account the engine
        // cannot sign in this way.
        match crate::magnetar_integration::admit_host_sign_in(&user_id).await {
            Ok(auth_epoch) => {
                crate::session::middleware::set_two_factor_pending_epoch(auth_epoch);
                Ok(())
            }
            Err(error) => {
                Self::cancel_challenge();
                Err(error)
            }
        }
    }

    /// Read the user-id of a session that has a 2FA challenge
    /// pending. Returns `None` outside a request scope or when no
    /// challenge is pending. Equivalent to
    /// [`crate::session::middleware::two_factor_pending_user_id`];
    /// exposed on the facade so consumers find it via
    /// `TwoFactor::*` autocomplete.
    pub fn pending_user_id() -> Option<String> {
        crate::session::middleware::two_factor_pending_user_id()
    }

    /// Cancel a pending 2FA challenge - clears both pending slots
    /// (user-id + remember preference) from the session without
    /// authenticating anyone. Typical use is a "back to login"
    /// button on the challenge page.
    ///
    /// Clearing both slots in lockstep is the contract: leaving
    /// `pending_remember` set after canceling would bleed the
    /// "remember me" preference into a next user's login flow on
    /// the same browser. Pending and pending-remember are paired
    /// state; tear-downs drop both.
    pub fn cancel_challenge() {
        crate::session::middleware::clear_two_factor_pending();
        crate::session::middleware::clear_two_factor_pending_remember();
    }

    #[cfg(any(
        feature = "database-sqlite",
        feature = "database-postgres",
        feature = "database-mysql"
    ))]
    /// Complete the 2FA challenge by verifying `code` against the
    /// session's pending user. On success, promotes the pending user
    /// to fully authenticated, rotates the session id and CSRF token
    /// to defeat session fixation, attempts to re-issue the remember-me
    /// cookie when the original login form requested it, and dispatches the
    /// standard [`crate::auth::events::Login`] +
    /// [`crate::auth::events::Authenticated`] pair followed by the
    /// 2FA-specific [`crate::auth_flows::events::TwoFactorChallenged`].
    /// On a bad code, dispatches
    /// [`crate::auth_flows::events::TwoFactorChallengeFailed`] and
    /// records exactly one failed attempt in the second-factor counter -
    /// single-attempt accounting even though both TOTP and recovery-code
    /// paths are tried. Returns the full
    /// [`crate::magnetar_integration::User`] on success so the caller
    /// can branch the post-login redirect on user attributes.
    ///
    /// Accepts either a current TOTP code or an unused recovery code -
    /// the recovery-code path matches Fortify's challenge controller,
    /// which lets users fall back to a recovery code if they've lost
    /// their authenticator. Recovery codes are consumed single-use on
    /// acceptance.
    ///
    /// # Brute-force gating
    ///
    /// The challenge reserves one attempt in the second-factor counter
    /// that every proof path shares (see [`Self::verify`]) before it reads
    /// the code, so a locked user cannot bypass the lock by submitting the
    /// right code: a 429 fires before any code is checked. A failed
    /// submission counts exactly once even though both the TOTP and the
    /// recovery-code forms are tried. The counter is not the password
    /// lockout that [`crate::auth_flows::LoginThrottleMiddleware`] checks:
    /// a successful password check does not clear second-factor failures.
    ///
    /// # Promotion contract
    ///
    /// The promotion mirrors [`crate::auth::Auth::login_id`] /
    /// [`crate::auth::Auth::login_remember`]: a fresh session id
    /// (so a session id planted before the challenge cannot ride
    /// the post-challenge auth), a fresh CSRF token (so any cached
    /// pre-auth token cannot be replayed under the new privilege
    /// level), and the auth user written into the session. The
    /// `Login` / `Authenticated` dispatches are the same shape and
    /// guard-attribution as a no-2FA password login, so listeners
    /// that hook those events (last-login timestamps, audit logs,
    /// post-login redirects, …) fire here too. With the Magnetar engine
    /// installed, [`crate::SessionMiddleware`] backs the promoted login
    /// with a Magnetar session at the end of the request, as it does for
    /// [`crate::auth::Auth::login_id`].
    ///
    /// Remember-me issuance is best-effort after an accepted factor proof:
    /// a failure cannot make a single-use TOTP timestep or recovery code
    /// retryable. The login still completes, the prior clear-cookie directive
    /// remains, and [`crate::auth::events::Login::remember`] is `false`.
    ///
    /// # Errors
    ///
    /// - [`FrameworkError::domain`] with status `400` if no challenge
    ///   is pending - the caller must invoke [`Self::start_challenge`]
    ///   (typically from a password-login handler) before
    ///   `complete_challenge` is meaningful.
    /// - [`FrameworkError::domain`] with status `401` if the pending
    ///   user-id no longer resolves to a Magnetar user (deleted mid-
    ///   challenge) or the supplied code validates as neither a TOTP
    ///   code nor a recovery code.
    /// - [`FrameworkError::domain`] with status `429` while wrong codes
    ///   have locked the second factor, and `503` when the attempt store
    ///   fails.
    pub async fn complete_challenge(
        code: &str,
    ) -> Result<crate::magnetar_integration::User, FrameworkError> {
        let Some(pending_id) = crate::session::middleware::two_factor_pending_user_id() else {
            return Err(FrameworkError::domain(
                "no 2FA challenge pending; submit credentials first",
                400,
            ));
        };

        let Some(user) = crate::magnetar_integration::find_user_by_id(&pending_id).await? else {
            return Err(FrameworkError::domain(
                "pending 2FA user no longer exists",
                401,
            ));
        };

        // Reject early if 2FA was disabled between start_challenge and
        // now. The user must re-login from scratch (without 2FA gating);
        // proceeding would either reject every code (no enrollment to
        // match against) or risk promoting on a disabled-2FA path.
        if !Self::is_enabled_by_id(&pending_id).await? {
            return Err(FrameworkError::domain(
                "2FA is no longer enabled for this account; restart the login flow",
                400,
            ));
        }

        // The promotion needs a Magnetar session when the engine requires
        // one. Ask before the code is read, so a refusal burns no code and
        // fires no event: an account with a Magnetar second factor this
        // challenge does not prove, or an auth epoch that moved since the
        // password was checked (a password reset, sign-out-everywhere).
        let recorded_epoch = crate::session::middleware::two_factor_pending_epoch();
        let host_auth_epoch = match crate::magnetar_integration::admit_host_sign_in(&pending_id)
            .await
        {
            Ok(Some(current)) if recorded_epoch.is_some_and(|recorded| recorded != current) => {
                Self::cancel_challenge();
                return Err(FrameworkError::domain(
                    "the sign-in expired because the account's sessions were revoked; sign in again",
                    401,
                ));
            }
            Ok(current) => recorded_epoch.or(current),
            Err(error) => {
                if matches!(error.status_code(), 401 | 409) {
                    Self::cancel_challenge();
                }
                return Err(error);
            }
        };

        // Reserve the attempt before touching proof material. The store
        // serializes reservations per user, so concurrent requests cannot
        // all pass a separate status read and then verify.
        let attempt = match ProofAttempt::admit(&pending_id, &user.email).await {
            Ok(attempt) => attempt,
            Err(error) => {
                if error.status_code() == 429 {
                    let _ = crate::events::EventFacade::dispatch(TwoFactorChallengeFailed {
                        user_id: pending_id.clone(),
                    })
                    .await;
                }
                return Err(error);
            }
        };

        // Adapter so the TwoFactorUser-keyed primitives can run against
        // the pending user.
        struct ChallengeAdapter<'a> {
            user_id: &'a str,
            email: &'a str,
        }
        impl TwoFactorUser for ChallengeAdapter<'_> {
            fn user_id(&self) -> &str {
                self.user_id
            }
            fn email(&self) -> &str {
                self.email
            }
        }

        let adapter = ChallengeAdapter {
            user_id: &pending_id,
            email: &user.email,
        };

        // TOTP first; fall back to a recovery code so the user isn't
        // locked out when they've lost their authenticator app. The one
        // reservation above is the canonical attempt for both forms.
        let accepted = match Self::verify_totp_or_recovery(&adapter, code).await {
            Ok(accepted) => accepted,
            Err(proof_error) => return Err(attempt.abandon(proof_error).await),
        };
        if !accepted {
            attempt.rejected().await?;
            let _ = crate::events::EventFacade::dispatch(TwoFactorChallengeFailed {
                user_id: pending_id.clone(),
            })
            .await;
            return Err(FrameworkError::domain("invalid 2FA code", 401));
        }
        // Clear the failures so a user who finally gets the code right
        // after a typo or two isn't carrying a stale count into their next
        // session.
        attempt.accepted().await?;

        // Read the remember-me preference the user supplied at
        // password-login time BEFORE clearing the pending bag - the
        // bag is about to be torn down as part of the promotion.
        let remember_requested = crate::session::middleware::two_factor_pending_remember();

        // Promote: pending → authed. Mirrors `Auth::login_id`'s
        // contract - rotate the session id to defeat session
        // fixation, set the user, clear pending state, rotate CSRF.
        // A planted pre-challenge session id cannot ride the
        // post-challenge auth.
        crate::session::regenerate_session_id();
        crate::session::middleware::set_auth_user(&pending_id);
        if let Some(auth_epoch) = host_auth_epoch {
            crate::session::middleware::record_host_sign_in_epoch(&pending_id, auth_epoch);
        }
        crate::session::middleware::clear_two_factor_pending();
        crate::session::middleware::clear_two_factor_pending_remember();
        crate::session::session_mut(|session| {
            session.csrf_token = crate::session::generate_csrf_token();
        });

        // The accepted TOTP timestep or recovery code is already consumed, so
        // remember-me issuance cannot turn the completed challenge back into a
        // retryable one. Treat it as an optional post-login enhancement and let
        // the Login event report whether issuance actually succeeded.
        let remember = if remember_requested {
            let ttl_minutes = (crate::session::SessionConfig::from_env()
                .remember_lifetime
                .as_secs()
                / 60) as i64;
            match crate::auth::Auth::issue_remember_cookie(&pending_id, ttl_minutes).await {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(
                        %error,
                        "remember-me issuance failed; completing login without a remembered session"
                    );
                    false
                }
            }
        } else {
            false
        };

        // Standard login lifecycle events first so listeners that
        // hook `Login` / `Authenticated` (last-login timestamps,
        // audit logs, post-login redirects) fire on the 2FA path
        // too - they cannot rely on `Auth::attempt` having fired
        // them, because attempt completed before 2FA gating
        // demoted the session. Then the 2FA-specific event for
        // code that wants to distinguish "logged in via challenge"
        // from "logged in via password alone."
        //
        // Dispatch errors are intentionally swallowed (logged by
        // the dispatcher) - the promotion has already committed; a
        // listener failure must not surface here.
        let guard = crate::auth::Auth::default_guard_name();
        let _ = crate::events::EventFacade::dispatch(crate::auth::events::Login {
            guard: guard.clone(),
            user_id: pending_id.clone(),
            remember,
        })
        .await;
        let _ = crate::events::EventFacade::dispatch(crate::auth::events::Authenticated {
            guard,
            user_id: pending_id.clone(),
        })
        .await;
        let _ = crate::events::EventFacade::dispatch(TwoFactorChallenged {
            user_id: pending_id,
        })
        .await;

        Ok(user)
    }

    /// Rotate the recovery codes for an active 2FA enrollment.
    ///
    /// Replaces the stored recovery-codes column with a fresh set of
    /// 10 codes. The plaintext codes are returned for one-time display -
    /// there is no API for retrieving them again. The secret and
    /// `confirmed_at` are left untouched (only the recovery-codes
    /// column rotates), so the user's existing authenticator app
    /// continues to work without re-pairing.
    ///
    /// Requires either a current TOTP code or an unused recovery code
    /// as proof of possession - same model as [`Self::re_enroll`]. A
    /// session-hijacked attacker that can reach this endpoint without
    /// proof would otherwise blow away the legitimate user's recovery
    /// codes (a denial-of-service against account recovery).
    ///
    /// # Errors
    ///
    /// - [`FrameworkError::domain`] with status `400` when no
    ///   confirmed enrollment exists - call [`Self::enroll`] /
    ///   [`Self::confirm`] first.
    /// - [`FrameworkError::domain`] with status `401` when `proof`
    ///   validates as neither a current TOTP code nor an unused
    ///   recovery code.
    /// - [`FrameworkError::domain`] with status `429` while wrong codes have locked
    ///   the second factor (see [`Self::verify`]), and `503` when the
    ///   attempt store fails.
    pub async fn regenerate_recovery_codes<U: TwoFactorUser>(
        user: &U,
        proof: &str,
    ) -> Result<Vec<String>, FrameworkError> {
        if !Self::is_enabled(user).await? {
            return Err(FrameworkError::domain(
                "no confirmed 2FA enrollment; cannot regenerate recovery codes",
                400,
            ));
        }

        // One reserved attempt covers both proof forms, and a locked
        // account is refused before the proof is read: a session-hijacked
        // attacker cannot grind proofs to blow away the user's codes.
        let proof_accepted = settle_attempt(user.user_id(), user.email(), || {
            Self::verify_totp_or_recovery(user, proof)
        })
        .await?;
        if !proof_accepted {
            return Err(FrameworkError::domain(
                "regenerate-recovery-codes proof is neither a valid TOTP code nor a recovery code",
                401,
            ));
        }

        let new_codes = recovery::generate(RECOVERY_CODE_COUNT);
        let encrypted = Crypt::encrypt_string(
            crate::crypto::CryptPurpose::TwoFactorRecovery,
            &new_codes.join("\n"),
        )?;

        let db = DB::connection()?;
        let conn = db.inner();
        let row = entity::Entity::find_by_id(user.user_id().to_string())
            .one(conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("two_factor find: {e}")))?
            .ok_or_else(|| FrameworkError::internal("two_factor row vanished mid-regenerate"))?;
        let mut active: entity::ActiveModel = row.into();
        active.recovery_codes = Set(Some(encrypted));
        active.updated_at = Set(crate::clock::now());
        active
            .update(conn)
            .await
            .map_err(|e| FrameworkError::internal(format!("two_factor update: {e}")))?;

        Ok(new_codes)
    }

    /// Clear the second-factor attempt counter for `user`, ending a
    /// lock before its window passes - the admin counterpart of
    /// [`crate::auth_flows::BruteForce::unlock_account`] for the
    /// second factor, whose failures have a counter of their own.
    ///
    /// Returns `true` when the user was locked, and dispatches
    /// [`super::events::AccountUnlocked`] only then, so audit listeners
    /// see one entry per real unlock.
    ///
    /// # Errors
    ///
    /// [`FrameworkError::domain`] with status `503` when the attempt
    /// store fails.
    pub async fn unlock<U: TwoFactorUser>(user: &U) -> Result<bool, FrameworkError> {
        let was_locked = attempts::clear(user.user_id()).await?;
        if was_locked {
            let _ = crate::events::EventFacade::dispatch(super::events::AccountUnlocked {
                email: user.email().to_owned(),
            })
            .await;
        }
        Ok(was_locked)
    }

    /// Disable 2FA entirely. Deletes the row and dispatches
    /// [`TwoFactorDisabled`] **only** when a row was actually
    /// removed.
    ///
    /// Idempotent: a no-op disable on a user who never enrolled is
    /// not an error. The event only fires on a real state transition
    /// (mirrors the [`super::events::AccountUnlocked`] contract) so
    /// audit listeners see one entry per actual disable, not one per
    /// click on a no-op button.
    pub async fn disable<U: TwoFactorUser>(user: &U) -> Result<(), FrameworkError> {
        // A pending rotation goes first: left behind, it could later be
        // confirmed over a new enrollment by whoever saw its secret.
        rotation::discard(user.user_id()).await?;
        let db = DB::connection()?;
        let result = entity::Entity::delete_by_id(user.user_id().to_string())
            .exec(db.inner())
            .await
            .map_err(|e| FrameworkError::internal(format!("two_factor delete: {e}")))?;

        if result.rows_affected > 0 {
            // Discard dispatch errors - the delete has already
            // committed; a listener failure must not surface here.
            let _ = crate::events::EventFacade::dispatch(TwoFactorDisabled {
                user_id: user.user_id().to_string(),
            })
            .await;
        }

        Ok(())
    }
}

/// Persist the secret and recovery codes of a new, unconfirmed enrollment.
///
/// The write is conditional, not a read-modify-write: the
/// `confirmed_at IS NULL` condition makes "not yet confirmed" part of the
/// write, so a confirmed secret - including one with a rotation pending -
/// is never replaced without the proof `re_enroll` takes, even when the
/// confirmation lands while `enroll` runs.
///
/// # Errors
///
/// `FrameworkError::domain(.., 409)` when the row is already confirmed.
async fn write_enrollment_row(
    user_id: &str,
    encrypted_secret: String,
    encrypted_recovery: String,
) -> Result<(), FrameworkError> {
    let db = DB::connection()?;
    let conn = db.inner();
    let now = crate::clock::now();
    let update = entity::Entity::update_many()
        .set(entity::ActiveModel {
            user_id: sea_orm::ActiveValue::NotSet,
            secret: Set(encrypted_secret.clone()),
            confirmed_at: Set(None),
            recovery_codes: Set(Some(encrypted_recovery.clone())),
            // A fresh secret makes any timestep remembered against the
            // old one meaningless.
            last_used_timestep: Set(None),
            created_at: sea_orm::ActiveValue::NotSet,
            updated_at: Set(now),
        })
        .filter(entity::Column::UserId.eq(user_id))
        .filter(entity::Column::ConfirmedAt.is_null());
    let replaced = update
        .exec(conn)
        .await
        .map_err(|e| FrameworkError::internal(format!("two_factor update: {e}")))?;
    if replaced.rows_affected > 0 {
        return Ok(());
    }

    // No row matched. Either there is no enrollment yet, or a fresh
    // enrollment met a row that is confirmed by now.
    let existing = entity::Entity::find_by_id(user_id.to_string())
        .one(conn)
        .await
        .map_err(|e| FrameworkError::internal(format!("two_factor find: {e}")))?;
    if existing.is_some() {
        return Err(FrameworkError::domain(
            "2FA is already enabled for this account; call re_enroll with a valid TOTP or recovery code as proof to rotate the secret",
            409,
        ));
    }
    // No enrollment at all: drop any rotation a deleted enrollment left
    // behind, so it cannot be confirmed over this one later.
    rotation::discard(user_id).await?;
    entity::ActiveModel {
        user_id: Set(user_id.to_string()),
        secret: Set(encrypted_secret),
        confirmed_at: Set(None),
        recovery_codes: Set(Some(encrypted_recovery)),
        // Fresh enrollment - no prior verification timestep to
        // guard against replay yet.
        last_used_timestep: Set(None),
        created_at: Set(now),
        updated_at: Set(now),
    }
    .insert(conn)
    .await
    .map_err(|e| FrameworkError::internal(format!("two_factor insert: {e}")))?;
    Ok(())
}

/// Stamp `confirmed_at` on the enrollment whose code was just checked, and
/// use the code up. Returns `false` when that enrollment is gone or already
/// confirmed, or its code's window is already claimed.
///
/// `checked_ciphertext` is the stored secret the code was checked against.
/// Each enrollment encrypts a fresh secret with a fresh nonce, so the
/// ciphertext names one enrollment exactly. Making the stamp conditional on
/// it means a concurrent enroll that replaced the secret after the check
/// leaves this stamp with nothing to match: the code proved possession of
/// the old secret, not of the new one.
///
/// The same write claims `current_timestep` the way [`claim_totp_timestep`]
/// does, so the confirmation code cannot be replayed at sign-in.
async fn stamp_confirmation(
    user_id: &str,
    checked_ciphertext: &str,
    current_timestep: i64,
    when: chrono::DateTime<chrono::Utc>,
) -> Result<bool, FrameworkError> {
    let db = DB::connection()?;
    let stamp = entity::Entity::update_many()
        .col_expr(entity::Column::ConfirmedAt, Expr::value(Some(when)))
        .col_expr(
            entity::Column::LastUsedTimestep,
            Expr::value(current_timestep + TOTP_SKEW_STEPS),
        )
        .col_expr(entity::Column::UpdatedAt, Expr::value(crate::clock::now()))
        .filter(entity::Column::UserId.eq(user_id))
        .filter(entity::Column::Secret.eq(checked_ciphertext))
        // A confirmation is stamped once. A second confirmation of the same
        // enrollment racing this one must neither re-stamp it nor fire
        // `TwoFactorEnrolled` again.
        .filter(entity::Column::ConfirmedAt.is_null())
        .filter(
            Condition::any()
                .add(entity::Column::LastUsedTimestep.is_null())
                .add(entity::Column::LastUsedTimestep.lt(current_timestep)),
        )
        .exec(db.inner())
        .await
        .map_err(|e| FrameworkError::internal(format!("two_factor confirm: {e}")))?;
    Ok(stamp.rows_affected > 0)
}

/// The stored secret of one enrollment: its ciphertext, which identifies
/// this exact enrollment, and the decrypted base32 secret.
struct StoredSecret {
    ciphertext: String,
    secret_b32: String,
    confirmed: bool,
}

async fn load_secret(user_id: &str) -> Result<Option<StoredSecret>, FrameworkError> {
    let db = DB::connection()?;
    let Some(row) = entity::Entity::find_by_id(user_id.to_string())
        .one(db.inner())
        .await
        .map_err(|e| FrameworkError::internal(format!("two_factor find: {e}")))?
    else {
        return Ok(None);
    };
    let secret_b32 =
        Crypt::decrypt_string(crate::crypto::CryptPurpose::TwoFactorSecret, &row.secret)?;
    Ok(Some(StoredSecret {
        ciphertext: row.secret,
        secret_b32,
        confirmed: row.confirmed_at.is_some(),
    }))
}

/// Read the enrollment and check `code` against it at the current timestep.
///
/// Returns the timestep the code was checked at when the code may be
/// claimed, and `None` for a missing or unconfirmed enrollment, a replay the
/// stored stamp already covers, or a mismatch. The stamp check here reads a
/// snapshot and is only a fast path: two racing requests can both pass it,
/// so [`claim_totp_timestep`] repeats it atomically against the same
/// timestep.
async fn prepare_totp_claim(user_id: &str, code: &str) -> Result<Option<i64>, FrameworkError> {
    let db = DB::connection()?;
    let Some(row) = entity::Entity::find_by_id(user_id.to_string())
        .one(db.inner())
        .await
        .map_err(|e| FrameworkError::internal(format!("two_factor find: {e}")))?
    else {
        return Ok(None);
    };
    if row.confirmed_at.is_none() {
        return Ok(None);
    }

    let now = crate::clock::now().timestamp();
    let current_timestep = totp_timestep_at(now);
    if let Some(last) = row.last_used_timestep
        && current_timestep <= last
    {
        return Ok(None);
    }

    let secret_b32 =
        Crypt::decrypt_string(crate::crypto::CryptPurpose::TwoFactorSecret, &row.secret)?;
    if !check_code(&secret_b32, code, now)? {
        return Ok(None);
    }
    Ok(Some(current_timestep))
}

/// Atomically claim `current_timestep` for `user_id`. Returns `true` for
/// exactly one caller per covered window.
///
/// The conditional WHERE turns check-and-stamp into one statement: the
/// first verify flips `last_used_timestep` to `claim_to`, and a racing
/// verify's predicate no longer matches, so it affects zero rows. A plain
/// read-modify-write would let two racing verifies both stamp and both
/// succeed.
///
/// The predicate compares the stored stamp with `current_timestep`, the
/// same test the snapshot check in [`prepare_totp_claim`] makes. Comparing
/// with `claim_to` instead would let a request at T+1 that read the row
/// before a request at T stamped it (T+1 < T+2) win as well, so one code
/// would be accepted twice across the boundary.
///
/// `claim_to` is the *forward* edge of the TOTP skew window
/// (`current + TOTP_SKEW_STEPS`). Stamping the bare `current` would leave
/// the same code replayable at the next timestep, because `check_code`
/// accepts codes for [T-1, T, T+1] at server time T - a captured code from
/// T is still in [T, T+1, T+2] at T+1, and a bare-current stamp would not
/// block it.
async fn claim_totp_timestep(user_id: &str, current_timestep: i64) -> Result<bool, FrameworkError> {
    let db = DB::connection()?;
    let claim_to = current_timestep + TOTP_SKEW_STEPS;
    let claim = entity::Entity::update_many()
        .col_expr(entity::Column::LastUsedTimestep, Expr::value(claim_to))
        .col_expr(entity::Column::UpdatedAt, Expr::value(crate::clock::now()))
        .filter(entity::Column::UserId.eq(user_id))
        .filter(
            Condition::any()
                .add(entity::Column::LastUsedTimestep.is_null())
                .add(entity::Column::LastUsedTimestep.lt(current_timestep)),
        )
        .exec(db.inner())
        .await
        .map_err(|e| FrameworkError::internal(format!("two_factor replay claim: {e}")))?;
    Ok(claim.rows_affected > 0)
}

/// The TOTP timestep of a Unix time. Used by [`TwoFactor::verify`]
/// for replay protection - a successful verify stamps the row from
/// this value, and subsequent verifies at the same or earlier
/// timestep are refused even when the code itself would structurally
/// validate. 30-second step matches the TOTP construction in
/// [`check_code`] / enrollment.
fn totp_timestep_at(unix_seconds: i64) -> i64 {
    unix_seconds / 30
}

/// One attempt reserved in the second-factor counter before a code is
/// read. Every proof path - [`TwoFactor::verify`],
/// [`TwoFactor::consume_recovery_code`], [`TwoFactor::confirm`],
/// [`TwoFactor::re_enroll`], [`TwoFactor::regenerate_recovery_codes`] and
/// [`TwoFactor::complete_challenge`] - goes through it, so they share one
/// counter and one threshold.
struct ProofAttempt<'a> {
    email: &'a str,
    reservation: attempts::Reservation,
}

impl<'a> ProofAttempt<'a> {
    /// Reserve the attempt, or refuse with `429` while the user is locked.
    async fn admit(user_id: &str, email: &'a str) -> Result<Self, FrameworkError> {
        match attempts::admit(user_id).await? {
            attempts::Admission::Admitted(reservation) => Ok(Self { email, reservation }),
            attempts::Admission::Locked => Err(FrameworkError::domain(
                "account is locked due to too many failed attempts",
                429,
            )),
        }
    }

    /// The proof was accepted: clear the user's failures.
    async fn accepted(self) -> Result<(), FrameworkError> {
        attempts::record_success(&self.reservation).await
    }

    /// The proof was rejected: count the failure, and announce the lock
    /// when this failure is the one that set it.
    async fn rejected(self) -> Result<(), FrameworkError> {
        let failure = attempts::record_failure(&self.reservation).await?;
        if failure.locked_now {
            let _ =
                crate::events::EventFacade::dispatch(crate::auth_flows::events::AccountLocked {
                    email: self.email.to_owned(),
                    failed_attempts: u32::try_from(failure.failed_attempts).unwrap_or(u32::MAX),
                })
                .await;
        }
        Ok(())
    }

    /// The proof could not be evaluated: release the reservation, and
    /// return the error the caller reports. A failed release wins, because
    /// the counter's state is then unknown.
    async fn abandon(self, proof_error: FrameworkError) -> FrameworkError {
        match attempts::release(&self.reservation).await {
            Ok(()) => proof_error,
            Err(release_error) => {
                tracing::error!(
                    original_error = %proof_error,
                    release_error = %release_error,
                    "two-factor proof failed and attempt release left state uncertain"
                );
                FrameworkError::domain("two-factor attempt state is uncertain", 503)
            }
        }
    }
}

/// Evaluate one proof under one reserved attempt and settle the attempt.
///
/// Returns whether the proof was accepted. A locked user gets `429` before
/// `evaluate` runs, a store failure gets `503`, and an error from
/// `evaluate` releases the reservation and is returned.
async fn settle_attempt<F, Fut>(
    user_id: &str,
    email: &str,
    evaluate: F,
) -> Result<bool, FrameworkError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<bool, FrameworkError>>,
{
    let attempt = ProofAttempt::admit(user_id, email).await?;
    match evaluate().await {
        Ok(true) => {
            attempt.accepted().await?;
            Ok(true)
        }
        Ok(false) => {
            attempt.rejected().await?;
            Ok(false)
        }
        Err(error) => Err(attempt.abandon(error).await),
    }
}

/// Verify a TOTP code against a base32-encoded secret at `unix_seconds`.
/// Centralised so `confirm` and `verify` share identical parameters
/// (SHA1 / 6 digits / skew=1 / 30s step - matching the enrollment-time
/// construction). The time comes from [`crate::clock::now`], like every
/// other time read in the framework, so the replay timestep and the code
/// check see the same instant and a test can move them together.
fn check_code(secret_b32: &str, code: &str, unix_seconds: i64) -> Result<bool, FrameworkError> {
    let secret_bytes = Secret::Encoded(secret_b32.into())
        .to_bytes()
        .map_err(|e| FrameworkError::internal(format!("decode totp secret: {e}")))?;
    let totp = TOTP::new(Algorithm::SHA1, 6, 1, 30, secret_bytes, None, "user".into())
        .map_err(|e| FrameworkError::internal(format!("totp new: {e}")))?;
    let time = u64::try_from(unix_seconds)
        .map_err(|_| FrameworkError::internal("totp check: clock is before the Unix epoch"))?;
    Ok(totp.check(code, time))
}

#[cfg(test)]
mod tests {
    //! Interleavings that the public API cannot pin from outside: each test
    //! runs one request's read phase, lets a second request finish, then runs
    //! the first request's write phase.

    use super::*;
    use crate::testing::{TestClock, TestDatabase};
    use chrono::{DateTime, Utc};

    struct Migrator;

    impl sea_orm_migration::MigratorTrait for Migrator {
        fn migrations() -> Vec<Box<dyn sea_orm_migration::MigrationTrait>> {
            vec![
                Box::new(migration::Migration),
                Box::new(migration_replay::Migration),
                Box::new(migration_attempts::Migration),
                Box::new(migration_rotation::Migration),
            ]
        }
    }

    struct User;

    impl TwoFactorUser for User {
        fn user_id(&self) -> &str {
            "interleaved-user"
        }
        fn email(&self) -> &str {
            "interleaved@example.test"
        }
    }

    fn ensure_crypt() {
        if !Crypt::is_initialized() {
            Crypt::init(crate::EncryptionKey::generate());
        }
    }

    fn at(unix_seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(unix_seconds, 0).expect("test time is in range")
    }

    /// The code the stored secret produces at `unix_seconds`.
    async fn stored_code_at(unix_seconds: i64) -> String {
        let db = DB::connection().expect("test connection");
        let row = entity::Entity::find_by_id(User.user_id().to_owned())
            .one(db.inner())
            .await
            .expect("read enrollment")
            .expect("enrollment exists");
        let secret =
            Crypt::decrypt_string(crate::crypto::CryptPurpose::TwoFactorSecret, &row.secret)
                .expect("decrypt secret");
        let bytes = Secret::Encoded(secret).to_bytes().expect("decode secret");
        TOTP::new(Algorithm::SHA1, 6, 1, 30, bytes, None, "user".into())
            .expect("totp")
            .generate(u64::try_from(unix_seconds).expect("positive time"))
    }

    #[tokio::test]
    async fn racing_verifications_across_a_timestep_boundary_accept_one_code_once() {
        ensure_crypt();
        let _db = TestDatabase::fresh::<Migrator>().await.expect("fresh db");

        // Request A runs in the last second of step S, request B in the
        // first second of step S+1. Both submit the code of step S, which
        // the skew window accepts at either time.
        let step = Utc::now().timestamp() / 30;
        let a_time = step * 30 + 29;
        let b_time = (step + 1) * 30;
        // Enroll and confirm two steps earlier: a confirmation uses its
        // code up, so it must not claim the window under test.
        let confirm_time = (step - 2) * 30 + 5;
        let clock = TestClock::travel_to(at(confirm_time));

        TwoFactor::enroll(&User).await.expect("enroll");
        TwoFactor::confirm(&User, &stored_code_at(confirm_time).await)
            .await
            .expect("confirm");
        clock.set(at(a_time));
        let code = stored_code_at(a_time).await;

        // B reads the row before A stamps it, so B's snapshot passes.
        clock.set(at(b_time));
        let b_timestep = prepare_totp_claim(User.user_id(), &code)
            .await
            .expect("B prepares")
            .expect("B's snapshot predates A's stamp");
        assert_eq!(b_timestep, step + 1);

        // A finishes first and stamps the forward edge of its window, S+1.
        clock.set(at(a_time));
        assert!(
            TwoFactor::verify_internal(&User, &code)
                .await
                .expect("A verifies"),
            "A is the first verification of this code"
        );

        // B's claim now runs against A's stamp. The stamp already covers
        // B's timestep, so B must lose: one code, one acceptance.
        clock.set(at(b_time));
        assert!(
            !claim_totp_timestep(User.user_id(), b_timestep)
                .await
                .expect("B claims"),
            "a code accepted at step S must not be accepted again at step S+1"
        );
    }
}
