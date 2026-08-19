//! Lettre-backed SMTP transport. Tokio + rustls.

use crate::error::FrameworkError;
use crate::mail::transport::{MailTransport, OutgoingMessage};
use async_trait::async_trait;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

/// SMTP transport built on `lettre`'s async submission client. Supports
/// STARTTLS and implicit-TLS submission ports.
pub struct SmtpMailTransport {
    inner: AsyncSmtpTransport<Tokio1Executor>,
}

impl SmtpMailTransport {
    /// STARTTLS submission. Pass `587` for the standard submission port,
    /// or a non-default port for relays that use one (gateway, proxy).
    pub fn starttls(
        host: &str,
        port: u16,
        user: &str,
        password: &str,
    ) -> Result<Self, FrameworkError> {
        let creds = Credentials::new(user.to_string(), password.to_string());
        let inner = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(|e| FrameworkError::internal(format!("smtp starttls: {e}")))?
            .port(port)
            .credentials(creds)
            .build();
        Ok(Self { inner })
    }

    /// TLS-wrapped SMTP. Pass `465` for the canonical implicit-TLS port,
    /// or a non-default port for a custom relay.
    pub fn tls(host: &str, port: u16, user: &str, password: &str) -> Result<Self, FrameworkError> {
        let creds = Credentials::new(user.to_string(), password.to_string());
        let inner = AsyncSmtpTransport::<Tokio1Executor>::relay(host)
            .map_err(|e| FrameworkError::internal(format!("smtp tls relay: {e}")))?
            .port(port)
            .credentials(creds)
            .build();
        Ok(Self { inner })
    }

    /// Plain unencrypted SMTP (for local Mailpit/MailHog dev only).
    pub fn unencrypted(host: &str, port: u16) -> Result<Self, FrameworkError> {
        let inner = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
            .port(port)
            .build();
        Ok(Self { inner })
    }
}

#[async_trait]
impl MailTransport for SmtpMailTransport {
    async fn send(&self, msg: &OutgoingMessage) -> Result<(), FrameworkError> {
        let builder = crate::mail::mime::base_builder(msg)?;
        // `false`: SMTP keeps the exact body shape it has always produced,
        // including the zero-part alternative for a message with no bodies.
        let multipart = crate::mail::mime::build_body(msg, false)?;
        let email = builder
            .multipart(multipart)
            .map_err(|e| FrameworkError::internal(format!("smtp build message: {e}")))?;

        self.inner
            .send(email)
            .await
            .map_err(|e| FrameworkError::internal(format!("smtp send: {e}")))?;
        Ok(())
    }

    fn name(&self) -> &'static str {
        "smtp"
    }
}
