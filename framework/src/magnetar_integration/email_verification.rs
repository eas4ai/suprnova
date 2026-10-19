//! Email verification through the installed Magnetar engine: a link that
//! proves the signed-in user holds their address.
//!
//! The link proves the mailbox only to the account's own signed-in user, as
//! Laravel's `verification.verify` route sits behind `auth` and `signed`.
//! Anyone can register another person's address with a password of their
//! own. A link that verified whoever opened it would let that person's click
//! verify the account the registrant holds, and a verified address is what
//! agent auth binds an agent's claim to. Asking for the account's own
//! sign-in closes that: the registrant cannot read the mail, and the
//! address's owner cannot sign in to the registrant's account.
//!
//! Verifying changes no credential and starts no session. Magnetar's
//! first-proof sign-ins ([`MagicLinkAuth`](super::magic_link::MagicLinkAuth))
//! prove an address another way, by signing the user in and removing every
//! credential set before the proof, which is why they cannot verify an
//! account that keeps its password.

use std::fmt;

use super::User;
use super::abuse_limiter::{AuthAbuseRoute, check_auth_abuse};
use crate::auth::Auth;
use crate::error::FrameworkError;

/// What [`EmailVerificationAuth::issue`] answers.
#[derive(Debug)]
#[non_exhaustive]
pub enum EmailVerificationIssue {
    /// A single-use token was minted for the account's address: mail the
    /// link that carries it.
    Issued(EmailVerificationLink),
    /// The address is already verified, so nothing was minted.
    AlreadyVerified,
    /// Nobody is signed in, or the account no longer exists.
    SignIn,
}

/// The address to mail and the token the verification link carries.
pub struct EmailVerificationLink {
    /// The account's address, where the link goes.
    pub email: String,
    /// The single-use token. It belongs in the link alone: it proves the
    /// mailbox to whoever holds it and signs in to the account.
    pub token: String,
}

impl fmt::Debug for EmailVerificationLink {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EmailVerificationLink")
            .field("email", &self.email)
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// What presenting a verification token answers
/// ([`EmailVerificationAuth::verify`]).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EmailVerificationOutcome {
    /// The token was the signed-in user's, and the address is now verified.
    Verified,
    /// The token was the signed-in user's, and is spent; the address was
    /// already verified, so its first verification time stays.
    AlreadyVerified,
    /// The token is another account's. It stays unused, for its owner.
    OtherAccount,
    /// The token expired before it was used.
    Expired,
    /// No such token, or it was already used.
    Unknown,
    /// Nobody is signed in: sign in, then open the link again.
    SignIn,
}

/// Email verification facade returned by `Auth::email_verification`.
///
/// The application mails the link: [`issue`](Self::issue) returns the
/// address and the token, and the page the link opens calls
/// [`verify`](Self::verify) for the user signed in there. See the
/// [module documentation](self) for why that user must be signed in.
pub struct EmailVerificationAuth;

impl EmailVerificationAuth {
    /// Mint a verification link for the signed-in user, by bearer token or
    /// by session.
    ///
    /// The mint counts against the address's email-verification budget, the
    /// one Magnetar's own verification route spends.
    ///
    /// # Errors
    ///
    /// `429` when the address asked too often, and an error when the engine
    /// is not installed or its store fails.
    pub async fn issue(&self) -> Result<EmailVerificationIssue, FrameworkError> {
        let Some(user_id) = Auth::id() else {
            return Ok(EmailVerificationIssue::SignIn);
        };
        issue_for_id(&user_id).await
    }

    /// Mint a verification link for `user`, an account this request created
    /// (`Registration::Created`), which nobody has signed in to yet.
    ///
    /// Minting needs no sign-in: the link proves the address only to the
    /// account's own signed-in user, so a link for any account helps no one
    /// else. It counts against the same budget as [`issue`](Self::issue).
    ///
    /// # Errors
    ///
    /// As [`issue`](Self::issue).
    pub async fn issue_for(&self, user: &User) -> Result<EmailVerificationIssue, FrameworkError> {
        issue_for_id(&user.id.to_string()).await
    }

    /// Present `token` for the signed-in user, by bearer token or by
    /// session, and verify their address when the token is theirs, unused
    /// and unexpired.
    ///
    /// A guest gets [`EmailVerificationOutcome::SignIn`] and the token stays
    /// unused, so the page can send them to sign in and back. Another
    /// account's token is refused and stays unused too.
    ///
    /// # Errors
    ///
    /// An error when the engine is not installed or its store fails.
    pub async fn verify(&self, token: &str) -> Result<EmailVerificationOutcome, FrameworkError> {
        let Some(user_id) = Auth::id() else {
            return Ok(EmailVerificationOutcome::SignIn);
        };
        super::password_engine()?
            .email_verification_verify(&user_id, token)
            .await
            .map_err(map_magnetar_email_verification_error)
    }
}

/// Mint a link for the account `user_id` names, unless its address is
/// already verified.
async fn issue_for_id(user_id: &str) -> Result<EmailVerificationIssue, FrameworkError> {
    let Some(user) = super::factor_engine()?
        .user_by_id(user_id)
        .await
        .map_err(map_magnetar_email_verification_error)?
    else {
        return Ok(EmailVerificationIssue::SignIn);
    };
    if user.is_email_verified() {
        return Ok(EmailVerificationIssue::AlreadyVerified);
    }
    check_auth_abuse(AuthAbuseRoute::EmailVerificationSend, &user.email).await?;
    let token = super::password_engine()?
        .email_verification_issue(user_id)
        .await
        .map_err(map_magnetar_email_verification_error)?;
    Ok(EmailVerificationIssue::Issued(EmailVerificationLink {
        email: user.email,
        token,
    }))
}

fn map_magnetar_email_verification_error(error: magnetar::Error) -> FrameworkError {
    FrameworkError::internal(format!("Magnetar email verification: {error}"))
}
