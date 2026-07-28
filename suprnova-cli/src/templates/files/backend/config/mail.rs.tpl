use suprnova::env;

/// Mail configuration
#[derive(Debug, Clone)]
pub struct MailConfig {
    /// Mail driver (smtp, resend, sendgrid, etc.)
    pub driver: String,
    /// SMTP host
    pub host: String,
    /// SMTP port
    pub port: u16,
    /// SMTP username
    pub username: String,
    /// SMTP password
    pub password: String,
    /// Default from email address
    pub from_address: String,
    /// Default from name
    pub from_name: String,
}

impl MailConfig {
    /// Build config from environment variables
    pub fn from_env() -> Self {
        Self {
            driver: env("MAIL_DRIVER", "smtp".to_string()),
            host: env("MAIL_SMTP_HOST", "localhost".to_string()),
            port: env("MAIL_SMTP_PORT", 1025),
            username: env("MAIL_SMTP_USER", "".to_string()),
            password: env("MAIL_SMTP_PASS", "".to_string()),
            from_address: env("MAIL_FROM", "hello@example.com".to_string()),
            from_name: env("MAIL_FROM_NAME", "Suprnova App".to_string()),
        }
    }
}
