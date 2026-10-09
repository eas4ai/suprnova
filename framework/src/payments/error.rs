//! Error type shared across every payments trait, DTO, and provider adapter.

use std::fmt;

use thiserror::Error;

/// Errors returned by the payments subsystem.
///
/// Provider adapters translate their SDK error shapes into this enum so
/// application code can match on a single variant set regardless of
/// which rail it is talking to.
///
/// `Debug` prints [`Self::Database`] with the text of its source and not
/// with the `Debug` of it: a driver puts the detail of a violated
/// constraint into that, and the detail names the values of the key, which
/// for the payments tables are payment ids.
#[derive(Error)]
pub enum PaymentError {
    /// Provider-side failure with no more specific classification (5xx,
    /// unexpected payload shape, transport error wrapping, etc.).
    #[error("provider error: {0}")]
    Provider(String),

    /// Caller-supplied request failed pre-flight validation. The message
    /// names the offending field or constraint.
    #[error("request validation failed: {0}")]
    Validation(String),

    /// The provider does not implement the requested operation
    /// (e.g. server-side capture on a Merchant-of-Record).
    #[error("operation not supported by this provider: {0}")]
    NotSupported(String),

    /// The payment was declined by the issuer or risk system.
    #[error("payment was declined: {reason}")]
    Declined {
        /// Human-readable reason as reported by the provider.
        reason: String,
        /// Provider-specific decline code (e.g. Stripe's
        /// `insufficient_funds`), when the provider supplies one.
        decline_code: Option<String>,
    },

    /// API key / signing key / bearer token rejected by the provider.
    #[error("provider authentication failed: {0}")]
    Authentication(String),

    /// The requested resource (customer, payment method, transaction,
    /// subscription, etc.) does not exist on the provider.
    #[error("requested resource not found: {0}")]
    NotFound(String),

    /// Inbound webhook signature failed verification - payload was either
    /// forged or tampered with in transit.
    #[error("webhook signature verification failed: {0}")]
    WebhookSignature(String),

    /// Phone number could not be parsed as a valid E.164 value. See
    /// [`super::PhoneNumber::new`].
    #[error("invalid phone number: {0}")]
    InvalidPhoneNumber(String),

    /// Country code is not a valid ISO 3166-1 alpha-2 value. See
    /// [`super::CountryCode::new`].
    #[error("invalid country code: {0}")]
    InvalidCountryCode(String),

    /// Internal framework error - surfacing a bug, not a recoverable
    /// caller mistake. Operators should treat this as a paging condition.
    #[error("internal payments error: {0}")]
    Internal(String),

    /// A statement against the payments tables failed.
    ///
    /// The error of the database is the source, so a caller that has the
    /// error can ask what kind of failure it was: a connection that was
    /// lost and a lock that timed out are gone at the next attempt, and a
    /// constraint that was violated is not.
    ///
    /// ```rust
    /// use std::error::Error;
    /// use suprnova::payments::PaymentError;
    ///
    /// fn worth_a_retry(error: &PaymentError) -> bool {
    ///     error
    ///         .source()
    ///         .and_then(|source| source.downcast_ref::<suprnova::DbErr>())
    ///         .is_some_and(|database| {
    ///             matches!(
    ///                 database,
    ///                 suprnova::DbErr::ConnectionAcquire(_) | suprnova::DbErr::Conn(_)
    ///             )
    ///         })
    /// }
    /// # let _ = worth_a_retry;
    /// ```
    ///
    /// The text is the text of [`Self::Internal`] with the error of the
    /// database in it, and `context` in front of that where the statement
    /// that failed is one of several that fail the same way.
    #[error("internal payments error: {}{source}", context_prefix(.context))]
    Database {
        /// What was being done: `begin tx`, `commit`. `None` where the
        /// error of the database says enough.
        context: Option<&'static str>,
        /// The error of the database.
        #[source]
        source: sea_orm::DbErr,
    },
}

/// `context: ` in front of the error, or nothing.
fn context_prefix(context: &Option<&'static str>) -> String {
    context.map_or_else(String::new, |context| format!("{context}: "))
}

impl fmt::Debug for PaymentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Provider(text) => formatter.debug_tuple("Provider").field(text).finish(),
            Self::Validation(text) => formatter.debug_tuple("Validation").field(text).finish(),
            Self::NotSupported(text) => formatter.debug_tuple("NotSupported").field(text).finish(),
            Self::Declined {
                reason,
                decline_code,
            } => formatter
                .debug_struct("Declined")
                .field("reason", reason)
                .field("decline_code", decline_code)
                .finish(),
            Self::Authentication(text) => {
                formatter.debug_tuple("Authentication").field(text).finish()
            }
            Self::NotFound(text) => formatter.debug_tuple("NotFound").field(text).finish(),
            Self::WebhookSignature(text) => formatter
                .debug_tuple("WebhookSignature")
                .field(text)
                .finish(),
            Self::InvalidPhoneNumber(text) => formatter
                .debug_tuple("InvalidPhoneNumber")
                .field(text)
                .finish(),
            Self::InvalidCountryCode(text) => formatter
                .debug_tuple("InvalidCountryCode")
                .field(text)
                .finish(),
            Self::Internal(text) => formatter.debug_tuple("Internal").field(text).finish(),
            Self::Database { context, source } => formatter
                .debug_struct("Database")
                .field("context", context)
                .field("source", &format_args!("{source}"))
                .finish(),
        }
    }
}

impl PaymentError {
    /// A database error with what was being done when it happened.
    pub fn database(context: &'static str, source: sea_orm::DbErr) -> Self {
        Self::Database {
            context: Some(context),
            source,
        }
    }
}

/// So a statement against the payments tables ends in `?`.
impl From<sea_orm::DbErr> for PaymentError {
    fn from(source: sea_orm::DbErr) -> Self {
        Self::Database {
            context: None,
            source,
        }
    }
}

/// Convenience alias for `Result<T, PaymentError>`.
pub type PaymentResult<T> = Result<T, PaymentError>;

#[cfg(test)]
mod tests {
    use super::PaymentError;
    use std::error::Error;

    fn lost() -> sea_orm::DbErr {
        sea_orm::DbErr::Custom("the connection was lost".to_owned())
    }

    #[test]
    fn a_database_error_reads_as_the_internal_error_did() {
        let bare: PaymentError = lost().into();
        assert_eq!(
            bare.to_string(),
            PaymentError::Internal(lost().to_string()).to_string()
        );

        let named = PaymentError::database("commit", lost());
        assert_eq!(
            named.to_string(),
            PaymentError::Internal(format!("commit: {}", lost())).to_string()
        );
    }

    #[test]
    fn a_database_error_keeps_the_error_of_the_database() {
        for error in [lost().into(), PaymentError::database("begin tx", lost())] {
            let error: PaymentError = error;
            let source = error
                .source()
                .and_then(|source| source.downcast_ref::<sea_orm::DbErr>());
            assert!(
                matches!(source, Some(sea_orm::DbErr::Custom(text)) if text.contains("lost")),
                "{error:?}"
            );
        }
        assert!(
            PaymentError::Internal("a text".to_owned())
                .source()
                .is_none(),
            "an error that is a text has no source"
        );
    }

    /// The `Debug` of the error of a driver can name the values of a key.
    #[test]
    fn debug_prints_the_text_of_the_database_error_and_not_its_debug() {
        let error = PaymentError::database("commit", lost());
        assert_eq!(
            format!("{error:?}"),
            "Database { context: Some(\"commit\"), source: Custom Error: the connection was lost }"
        );
        assert_eq!(
            format!("{:?}", PaymentError::Internal("a text".to_owned())),
            "Internal(\"a text\")"
        );
        assert_eq!(
            format!(
                "{:?}",
                PaymentError::Declined {
                    reason: "no funds".to_owned(),
                    decline_code: None,
                }
            ),
            "Declined { reason: \"no funds\", decline_code: None }"
        );
    }
}
