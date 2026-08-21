//! TOTP construction and matched-step verification.
//!
//! The construction is the deployed one exactly: RFC 6238, SHA-1, six
//! digits, thirty-second step, one step of skew, issuer folded into the
//! otpauth URL alongside the account email. Verification deviates from the
//! deployed forward-edge stamp deliberately (the spec's FLAGGED hardening):
//! it reports *which* timestep matched so the caller can claim exactly that
//! step, closing the forward-edge replay the adversarial review found.

use secrecy::{ExposeSecret, SecretString};
use subtle::ConstantTimeEq;
use totp_rs::{Algorithm, Secret, TOTP};

use crate::{Error, Result};

/// TOTP step length in seconds.
pub const STEP_SECONDS: i64 = 30;
/// Accepted skew, in steps, on either side of the current step.
pub const SKEW_STEPS: i64 = 1;

/// A freshly provisioned secret with its enrollment artifacts.
pub struct ProvisionedSecret {
    /// Base32-encoded shared secret; encrypt before storage, never log.
    pub secret_b32: SecretString,
    /// The otpauth URL (contains the raw secret in its query string).
    pub otpauth_url: SecretString,
    /// Inline-SVG QR code wrapping the otpauth payload, deployed shape.
    pub qr_code_svg: String,
}

fn construction(secret_b32: &str, issuer: &str, account: &str) -> Result<TOTP> {
    let secret_bytes = Secret::Encoded(secret_b32.to_owned())
        .to_bytes()
        .map_err(|error| Error::Internal {
            message: format!("totp secret bytes: {error}"),
        })?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
        Some(issuer.to_owned()),
        account.to_owned(),
    )
    .map_err(|error| Error::Internal {
        message: format!("totp construction: {error}"),
    })
}

/// Mint a fresh secret plus its otpauth URL and QR SVG.
pub fn provision(issuer: &str, account: &str) -> Result<ProvisionedSecret> {
    let secret_bytes = Secret::generate_secret()
        .to_bytes()
        .map_err(|error| Error::Internal {
            message: format!("totp secret bytes: {error}"),
        })?;
    let secret_b32 = Secret::Raw(secret_bytes).to_encoded().to_string();
    let totp = construction(&secret_b32, issuer, account)?;
    let otpauth_url = totp.get_url();
    let qr_b64 = totp.get_qr_base64().map_err(|error| Error::Internal {
        message: format!("totp qr: {error}"),
    })?;
    let qr_code_svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 256 256\">\
         <image href=\"data:image/png;base64,{qr_b64}\" width=\"256\" height=\"256\"/></svg>"
    );
    Ok(ProvisionedSecret {
        secret_b32: SecretString::from(secret_b32),
        otpauth_url: SecretString::from(otpauth_url),
        qr_code_svg,
    })
}

/// The timestep containing `now`.
#[must_use]
pub fn timestep_at(now: chrono::DateTime<chrono::Utc>) -> i64 {
    now.timestamp() / STEP_SECONDS
}

/// Verify a submitted code against every step in the skew window and
/// return the step that matched, if any.
///
/// All candidate steps are evaluated and compared in constant time, so an
/// observer learns nothing about which edge (or whether any edge) matched.
/// The account label does not participate in code generation; a fixed
/// label keeps this function independent of user data.
pub fn matched_step(
    secret_b32: &SecretString,
    code: &str,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Option<i64>> {
    let totp = construction(secret_b32.expose_secret(), "verify", "verify")?;
    let current = timestep_at(now);
    let mut matched: Option<i64> = None;
    for step in (current - SKEW_STEPS)..=(current + SKEW_STEPS) {
        let expected = totp.generate((step * STEP_SECONDS) as u64);
        let equal: bool = expected.as_bytes().ct_eq(code.as_bytes()).into();
        if equal && matched.is_none() {
            matched = Some(step);
        }
    }
    Ok(matched)
}
