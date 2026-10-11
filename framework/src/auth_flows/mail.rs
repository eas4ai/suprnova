//! Transactional [`Mailable`] types for the auth flows.
//!
//! These three mailables back the email-verification, password-reset, and
//! password-changed lifecycle the [`EmailVerification`](crate::auth_flows::EmailVerification)
//! / [`PasswordReset`](crate::auth_flows::PasswordReset) facades drive. The
//! verification message goes out as a [`VerifyEmailNotification`], a mail
//! notification through [`Notify`] that renders [`EmailVerificationMail`];
//! the others are dispatched via the ordinary [`Mail`] facade:
//!
//! ```rust,no_run
//! # use suprnova::Mail;
//! # use suprnova::auth_flows::EmailVerificationMail;
//! # async fn ex(mail: EmailVerificationMail) -> Result<(), Box<dyn std::error::Error>> {
//! Mail::to(mail.to_address.as_str()).send(mail).await?;
//! # Ok(()) }
//! ```
//!
//! `to_address` and `from_address` live on each struct as plain `String`s so
//! they (a) participate in the serialized Tera context the template renders
//! against and (b) survive the JSON round-trip the queue worker performs.
//! The `from()` impl converts `from_address` into an [`Address`] for the
//! mail dispatcher.
//!
//! # Escaping
//!
//! The mail crate disables Tera autoescape (see
//! `framework/src/mail/mailable.rs`). User-controllable fields rendered into
//! the HTML body are piped through Tera's built-in `escape` filter so an
//! attacker cannot smuggle markup through a chosen display name. The text
//! body does not escape because its consumers (mail clients in
//! plaintext-mode) render it verbatim and `&` / `<` are not special there.

use crate::error::FrameworkError;
use crate::mail::{Address, Mail, Mailable};
use crate::notifications::channels::mail::{
    MailChannel, MailRendering, NotificationMailable, register_mail_renderer,
};
use crate::notifications::{Notification, NotificationDispatcher, Notify};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Read `MAIL_FROM_NAME` for the outgoing display name, treating unset/blank as
/// "no name". Read at send time so it survives the mailable's serde round-trip
/// through the queue worker (the mailable stores only the bare `from_address`).
fn mail_from_name() -> Option<String> {
    std::env::var("MAIL_FROM_NAME")
        .ok()
        .filter(|n| !n.trim().is_empty())
}

/// Build the envelope `From`, attaching an optional display name so the header
/// renders as `"Name <email>"` (RFC 5322) instead of a bare address. `MAIL_FROM`
/// must remain a bare address; the display name comes from `MAIL_FROM_NAME`.
fn build_from(from_address: &str, from_name: Option<String>) -> Address {
    let addr = Address::new(from_address);
    match from_name {
        Some(name) => addr.with_name(name),
        None => addr,
    }
}

// ──────────────────────────────────────────────────────────────────────
// EmailVerificationMail
// ──────────────────────────────────────────────────────────────────────

/// "Verify your email" message dispatched after signup (and on resend).
///
/// The `verification_link` is the fully-qualified URL the
/// [`EmailVerification`](crate::auth_flows::EmailVerification) facade builds
/// (base URL + the issued token as a query parameter) - the mailable does not
/// construct or sign the token itself.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct EmailVerificationMail {
    /// Recipient. Used both as the Tera context and on the call site
    /// (`Mail::to(&mail.to_address).send(mail)`).
    pub to_address: String,
    /// Optional display name. When `None` the templates fall back to
    /// "there" via Tera's `default` filter.
    pub user_name: Option<String>,
    /// Fully-qualified verification URL.
    pub verification_link: String,
    /// Display name of the sending application (interpolated into the
    /// subject and the body's branding line).
    pub app_name: String,
    /// Envelope `From`. Plain `String` for serde-friendliness; the
    /// `from()` impl lifts it into an [`Address`].
    pub from_address: String,
}

#[async_trait]
impl Mailable for EmailVerificationMail {
    fn mailable_name() -> &'static str {
        "EmailVerificationMail"
    }

    fn subject(&self) -> String {
        format!("Verify your email for {}", self.app_name)
    }

    fn html_template_source(&self) -> Option<String> {
        // Autoescape is OFF - pipe user-controllable fields through `escape`
        // explicitly. `app_name` and `verification_link` originate from
        // framework-controlled config, but we still escape them so a
        // future config typo (`<` in the brand string) can't break rendering.
        Some(
            r#"<!doctype html>
<html>
  <body style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; color: #1a1a1a;">
    <h1 style="font-size: 20px;">Hi {{ user_name | default(value="there") | escape }},</h1>
    <p>Welcome to {{ app_name | escape }}. Please confirm your email address by clicking the link below:</p>
    <p><a href="{{ verification_link | escape }}" style="display: inline-block; padding: 10px 16px; background: #2563eb; color: #fff; text-decoration: none; border-radius: 6px;">Verify email</a></p>
    <p>Or copy this URL into your browser:<br><span style="word-break: break-all;">{{ verification_link | escape }}</span></p>
    <p>This link expires in 24 hours. If you didn't sign up for {{ app_name | escape }}, you can safely ignore this email.</p>
  </body>
</html>"#
                .to_string(),
        )
    }

    fn text_template_source(&self) -> Option<String> {
        Some(
            "Hi {{ user_name | default(value=\"there\") }},\n\
             \n\
             Welcome to {{ app_name }}. Please confirm your email address by visiting:\n\
             \n\
             {{ verification_link }}\n\
             \n\
             This link expires in 24 hours. If you didn't sign up for {{ app_name }}, \
             you can safely ignore this email.\n"
                .to_string(),
        )
    }

    fn from(&self) -> Option<Address> {
        Some(build_from(&self.from_address, mail_from_name()))
    }
}

// ──────────────────────────────────────────────────────────────────────
// VerifyEmailNotification
// ──────────────────────────────────────────────────────────────────────

/// The framework's "verify your email" notification, Laravel's
/// `VerifyEmail`.
///
/// [`MustVerifyEmail::send_email_verification_notification`](crate::MustVerifyEmail::send_email_verification_notification)
/// and the provider's default send it to the verification address as an
/// on-demand mail notification. It goes out through the mail channel of the
/// dispatcher bound with
/// [`notifications::set_dispatcher`](crate::notifications::set_dispatcher)
/// when that dispatcher has one, and through the framework's
/// [`MailChannel`] otherwise, so an application needs no notification setup
/// for verification mail. Under [`Notify::fake`] it is recorded instead,
/// and a test reads the link back with `sent::<VerifyEmailNotification>`.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct VerifyEmailNotification {
    /// The message the mail channel renders: the subject, bodies and sender
    /// of [`EmailVerificationMail`], so a notification reads exactly as the
    /// mail the facade sent before.
    pub mail: EmailVerificationMail,
}

impl Notification for VerifyEmailNotification {
    fn notification_name() -> &'static str {
        "suprnova.auth.verify_email"
    }

    fn channels(&self) -> Vec<&'static str> {
        vec!["mail"]
    }

    fn data(&self) -> serde_json::Value {
        // The link carries a single-use bearer token. `data()` reaches the
        // `NotificationSending` and `NotificationSent` listeners, so the
        // link stays out of it; the mail channel renders from `self.mail`.
        serde_json::json!({
            "to_address": self.mail.to_address,
            "app_name": self.mail.app_name,
        })
    }
}

impl NotificationMailable for VerifyEmailNotification {
    fn to_mail(&self) -> Result<MailRendering, FrameworkError> {
        Ok(MailRendering {
            subject: self.mail.render_subject()?,
            html: self.mail.render_html()?,
            text: self.mail.render_text()?,
            from: Mailable::from(&self.mail),
            ..Default::default()
        })
    }
}

/// Send the framework's [`VerifyEmailNotification`] for `verification_link`
/// to `address`, greeting `user_name`. The default of both verification
/// hooks, the model's and the provider's.
///
/// Reads `APP_NAME` and the fail-closed `MAIL_FROM` at send time, as the
/// mailables do.
pub(crate) async fn send_verification_notification(
    address: &str,
    user_name: Option<String>,
    verification_link: &str,
) -> Result<(), FrameworkError> {
    let notification = VerifyEmailNotification {
        mail: EmailVerificationMail {
            to_address: address.to_owned(),
            user_name,
            verification_link: verification_link.to_owned(),
            app_name: crate::auth_flows::app_name(),
            from_address: crate::auth_flows::require_mail_from()?,
        },
    };
    notify_by_mail(address, &notification).await
}

/// Send `notification` to `address` as an on-demand mail notification.
///
/// Under [`Notify::fake`] it is recorded. Otherwise it goes through the
/// bound dispatcher when that dispatcher has a `mail` channel, and through a
/// dispatcher holding only the framework's [`MailChannel`] when no
/// dispatcher is bound or the bound one has no mail channel: a framework
/// message must not be dropped because the application never set up
/// notifications. The renderer is registered on each send for the same
/// reason; registering it again replaces the entry with the same renderer.
async fn notify_by_mail<N: NotificationMailable>(
    address: &str,
    notification: &N,
) -> Result<(), FrameworkError> {
    let recipient = Notify::route("mail", address)?;
    if crate::notifications::testing::is_active() {
        return Notify::send(&recipient, notification).await;
    }
    register_mail_renderer::<N>()?;
    let dispatcher = crate::notifications::dispatcher_for_queue()
        .ok()
        .filter(|dispatcher| dispatcher.channel("mail").is_some())
        .unwrap_or_else(|| {
            Arc::new(NotificationDispatcher::new().register_channel(Arc::new(MailChannel::new())))
        });
    dispatcher.notify(&recipient, notification).await
}

/// Send the framework's [`PasswordResetMail`] for `reset_link` to
/// `address`, greeting `user_name`. The default of both reset hooks, the
/// model's and the provider's.
///
/// Reads `APP_NAME` and the fail-closed `MAIL_FROM` at send time.
pub(crate) async fn send_password_reset_mail(
    address: &str,
    user_name: Option<String>,
    reset_link: &str,
) -> Result<(), FrameworkError> {
    let mail = PasswordResetMail {
        to_address: address.to_owned(),
        user_name,
        reset_link: reset_link.to_owned(),
        app_name: crate::auth_flows::app_name(),
        from_address: crate::auth_flows::require_mail_from()?,
    };
    Mail::to(address).send(mail).await
}

// ──────────────────────────────────────────────────────────────────────
// PasswordResetMail
// ──────────────────────────────────────────────────────────────────────

/// "Reset your password" message dispatched when the user requests a
/// password-reset link from the forgot-password endpoint.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PasswordResetMail {
    /// Recipient address - must be the user's on-file email.
    pub to_address: String,
    /// Display name interpolated into the greeting; `None` falls back to the email local-part.
    pub user_name: Option<String>,
    /// Fully-qualified reset URL.
    pub reset_link: String,
    /// Application name used in the subject + body.
    pub app_name: String,
    /// Envelope-from address for the outgoing message.
    pub from_address: String,
}

#[async_trait]
impl Mailable for PasswordResetMail {
    fn mailable_name() -> &'static str {
        "PasswordResetMail"
    }

    fn subject(&self) -> String {
        format!("Reset your {} password", self.app_name)
    }

    fn html_template_source(&self) -> Option<String> {
        Some(
            r#"<!doctype html>
<html>
  <body style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; color: #1a1a1a;">
    <h1 style="font-size: 20px;">Hi {{ user_name | default(value="there") | escape }},</h1>
    <p>We received a request to reset your {{ app_name | escape }} password. Click the link below to choose a new one:</p>
    <p><a href="{{ reset_link | escape }}" style="display: inline-block; padding: 10px 16px; background: #2563eb; color: #fff; text-decoration: none; border-radius: 6px;">Reset password</a></p>
    <p>Or copy this URL into your browser:<br><span style="word-break: break-all;">{{ reset_link | escape }}</span></p>
    <p>This link expires in 15 minutes. If you didn't request a password reset, you can safely ignore this email - your password will stay the same.</p>
  </body>
</html>"#
                .to_string(),
        )
    }

    fn text_template_source(&self) -> Option<String> {
        Some(
            "Hi {{ user_name | default(value=\"there\") }},\n\
             \n\
             We received a request to reset your {{ app_name }} password. \
             Visit the link below to choose a new one:\n\
             \n\
             {{ reset_link }}\n\
             \n\
             This link expires in 15 minutes. If you didn't request a password reset, \
             you can safely ignore this email - your password will stay the same.\n"
                .to_string(),
        )
    }

    fn from(&self) -> Option<Address> {
        Some(build_from(&self.from_address, mail_from_name()))
    }
}

// ──────────────────────────────────────────────────────────────────────
// PasswordChangedMail
// ──────────────────────────────────────────────────────────────────────

/// "Your password was changed" confirmation dispatched after a successful
/// password change (via the reset flow, the change-password endpoint, or any
/// other lifecycle event that mutates the password hash).
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct PasswordChangedMail {
    /// Recipient address - the user's on-file email.
    pub to_address: String,
    /// Display name interpolated into the greeting; `None` falls back to the email local-part.
    pub user_name: Option<String>,
    /// Application name used in the subject + body.
    pub app_name: String,
    /// Envelope-from address for the outgoing message.
    pub from_address: String,
}

#[async_trait]
impl Mailable for PasswordChangedMail {
    fn mailable_name() -> &'static str {
        "PasswordChangedMail"
    }

    fn subject(&self) -> String {
        format!("Your {} password was changed", self.app_name)
    }

    fn html_template_source(&self) -> Option<String> {
        Some(
            r#"<!doctype html>
<html>
  <body style="font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif; color: #1a1a1a;">
    <h1 style="font-size: 20px;">Hi {{ user_name | default(value="there") | escape }},</h1>
    <p>Your {{ app_name | escape }} password was just changed.</p>
    <p>If this was you, no further action is required.</p>
    <p>If this <strong>wasn't</strong> you, please contact our support team immediately so we can secure your account.</p>
  </body>
</html>"#
                .to_string(),
        )
    }

    fn text_template_source(&self) -> Option<String> {
        Some(
            "Hi {{ user_name | default(value=\"there\") }},\n\
             \n\
             Your {{ app_name }} password was just changed.\n\
             \n\
             If this was you, no further action is required.\n\
             \n\
             If this WASN'T you, please contact our support team immediately so we can \
             secure your account.\n"
                .to_string(),
        )
    }

    fn from(&self) -> Option<Address> {
        Some(build_from(&self.from_address, mail_from_name()))
    }
}

// ──────────────────────────────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail::Mailable;

    #[test]
    fn email_verification_html_escapes_user_name() {
        let m = EmailVerificationMail {
            to_address: "x@example.com".into(),
            user_name: Some("<script>alert(1)</script>".into()),
            verification_link: "https://example.com/v?t=x".into(),
            app_name: "App".into(),
            from_address: "no@reply.com".into(),
        };
        let html = m.render_html().unwrap().unwrap();
        assert!(!html.contains("<script>"), "raw script tag escaped: {html}");
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn password_reset_text_preserves_link_verbatim() {
        let m = PasswordResetMail {
            to_address: "x@example.com".into(),
            user_name: None,
            reset_link: "https://example.com/r?t=abc&u=42".into(),
            app_name: "App".into(),
            from_address: "no@reply.com".into(),
        };
        let text = m.render_text().unwrap().unwrap();
        // Plain text doesn't HTML-escape, so the & is preserved.
        assert!(text.contains("https://example.com/r?t=abc&u=42"));
    }

    #[test]
    fn missing_user_name_falls_back_to_there() {
        let m = PasswordChangedMail {
            to_address: "x@example.com".into(),
            user_name: None,
            app_name: "App".into(),
            from_address: "no@reply.com".into(),
        };
        let html = m.render_html().unwrap().unwrap();
        assert!(html.contains("Hi there"), "missing name fallback: {html}");
    }

    #[test]
    fn subject_includes_app_name() {
        let m = EmailVerificationMail {
            to_address: "x@example.com".into(),
            user_name: None,
            verification_link: "https://x".into(),
            app_name: "MyCorp".into(),
            from_address: "no@reply.com".into(),
        };
        assert_eq!(m.render_subject().unwrap(), "Verify your email for MyCorp");
    }

    #[test]
    fn build_from_attaches_display_name() {
        let a = build_from("shawn@eas4ai.com", Some("Shawn McAllister".into()));
        assert_eq!(a.to_string(), "Shawn McAllister <shawn@eas4ai.com>");
    }

    #[test]
    fn build_from_is_bare_without_name() {
        assert_eq!(
            build_from("shawn@eas4ai.com", None).to_string(),
            "shawn@eas4ai.com"
        );
    }
}
