//! The typed outbound-mail catalog.
//!
//! 04's grounded union of auth messages: verification, password reset,
//! password changed, and magic link. Each constructor produces the one
//! stable [`MailMessage`](crate::plugin::MailMessage) shape for its flow, so hosts key template
//! selection off `name` and tests assert dispatch counts without string
//! drift across plugins. The welcome message belongs to hosts; Magnetar
//! never sends unprompted product mail.

use serde_json::json;

use crate::plugin::MailMessage;

/// Stable message name for the initial and re-sent verification link.
pub const EMAIL_VERIFICATION: &str = "email_verification";
/// Stable message name for the password-reset link.
pub const PASSWORD_RESET: &str = "password_reset";
/// Stable message name for the changed-password security notification.
pub const PASSWORD_CHANGED: &str = "password_changed";
/// Stable message name for the magic sign-in link.
pub const MAGIC_LINK: &str = "magic_link";
/// Stable message name for the OAuth email-completion link.
pub const OAUTH_EMAIL_COMPLETION: &str = "oauth_email_completion";

/// The verification-link message.
#[must_use]
pub fn email_verification(recipient: &str, verification_link: &str) -> MailMessage {
    MailMessage {
        name: EMAIL_VERIFICATION.to_owned(),
        recipient: recipient.to_owned(),
        payload: json!({
            "email": recipient,
            "verification_link": verification_link,
        }),
    }
}

/// The password-reset-link message.
#[must_use]
pub fn password_reset(recipient: &str, reset_link: &str) -> MailMessage {
    MailMessage {
        name: PASSWORD_RESET.to_owned(),
        recipient: recipient.to_owned(),
        payload: json!({
            "email": recipient,
            "reset_link": reset_link,
        }),
    }
}

/// The changed-password security notification.
#[must_use]
pub fn password_changed(recipient: &str) -> MailMessage {
    MailMessage {
        name: PASSWORD_CHANGED.to_owned(),
        recipient: recipient.to_owned(),
        payload: json!({"email": recipient}),
    }
}

/// The magic sign-in-link message.
#[must_use]
pub fn magic_link(recipient: &str, magic_link: &str) -> MailMessage {
    MailMessage {
        name: MAGIC_LINK.to_owned(),
        recipient: recipient.to_owned(),
        payload: json!({
            "email": recipient,
            "magic_link": magic_link,
        }),
    }
}

/// The OAuth email-completion link message.
#[must_use]
pub fn oauth_email_completion(recipient: &str, completion_link: &str) -> MailMessage {
    MailMessage {
        name: OAUTH_EMAIL_COMPLETION.to_owned(),
        recipient: recipient.to_owned(),
        payload: json!({
            "email": recipient,
            "completion_link": completion_link,
        }),
    }
}
