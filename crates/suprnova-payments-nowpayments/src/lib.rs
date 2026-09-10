//! Hosted NOWPayments invoices and authenticated payment notifications.
//!
//! This adapter uses the non-custodial invoice API. An invoice identifier is
//! not a payment identifier: use [`NowPaymentsProvider::payment_status`] with
//! the payment ID from a verified IPN. The shared checkout session-status
//! operation deliberately returns `NotSupported` for invoice IDs.
//!
//! NOWPayments does not offer an idempotency guarantee for invoice creation.
//! Persist an application checkout attempt before calling the provider, and
//! reconcile an unknown outcome instead of automatically creating another
//! invoice. [`NowPaymentsProvider::create_invoice`] exposes that distinction.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

mod checkout;
mod client;
mod json;
mod status;
mod unsupported;
mod webhook;

#[cfg(test)]
mod tests;

pub use checkout::{InvoiceCreationError, NowPaymentsInvoice};
pub use status::{NowPaymentsPayment, NowPaymentsStatus};

use http::HeaderValue;
use std::fmt;
use std::time::Duration;
use suprnova::payments::{PaymentError, PaymentProvider, PaymentResult};
use url::Url;

const MAX_BODY_BYTES: usize = 64 * 1024;

/// Separates credentials, API endpoints and hosted checkout destinations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NowPaymentsEnvironment {
    /// Provider sandbox; no real funds should be sent.
    Sandbox,
    /// Production payment processing.
    Production,
}

impl NowPaymentsEnvironment {
    fn api_url(self) -> &'static str {
        match self {
            Self::Sandbox => "https://api-sandbox.nowpayments.io/v1/",
            Self::Production => "https://api.nowpayments.io/v1/",
        }
    }

    fn checkout_host(self) -> &'static str {
        match self {
            Self::Sandbox => "sandbox.nowpayments.io",
            Self::Production => "nowpayments.io",
        }
    }
}

/// A reusable invoice provider, registered with `PaymentProviderRegistry::bind`.
///
/// Clones share the HTTP connection pool. Credentials are excluded from Debug.
/// Return URLs must share the configured callback URL's HTTPS origin.
#[derive(Clone)]
pub struct NowPaymentsProvider {
    client: reqwest::Client,
    api_key: HeaderValue,
    ipn_secret: String,
    callback_url: Url,
    api_url: Url,
    environment: NowPaymentsEnvironment,
}

impl fmt::Debug for NowPaymentsProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NowPaymentsProvider")
            .field("environment", &self.environment)
            .finish_non_exhaustive()
    }
}

impl NowPaymentsProvider {
    /// Construct a provider with a trusted merchant callback URL.
    ///
    /// `ipn_callback_url` normally ends in `/webhooks/payments/nowpayments`.
    /// It must use HTTPS without credentials or a fragment. The same origin
    /// is required for success/cancel URLs, preventing untrusted redirects.
    /// The API client has a 30-second deadline, bounded bodies, and no redirects
    /// or automatic retries. Blank credentials fail before any network call.
    pub fn new(
        api_key: impl Into<String>,
        ipn_secret: impl Into<String>,
        ipn_callback_url: &str,
        environment: NowPaymentsEnvironment,
    ) -> PaymentResult<Self> {
        let api_key = nonempty("NOWPAYMENTS_API_KEY", api_key.into())?;
        let ipn_secret = nonempty("NOWPAYMENTS_IPN_SECRET", ipn_secret.into())?;
        let mut api_key = HeaderValue::from_str(&api_key).map_err(|_| {
            PaymentError::Validation("NOWPAYMENTS_API_KEY is not a valid header value".into())
        })?;
        api_key.set_sensitive(true);
        let callback_url = https_url(ipn_callback_url, "ipn_callback_url")?;
        let api_url = Url::parse(environment.api_url())
            .map_err(|_| PaymentError::Internal("invalid NOWPayments API constant".into()))?;
        let client = build_client(Duration::from_secs(30))?;
        Ok(Self {
            client,
            api_key,
            ipn_secret,
            callback_url,
            api_url,
            environment,
        })
    }

    /// Construct from `NOWPAYMENTS_API_KEY`, `NOWPAYMENTS_IPN_SECRET`,
    /// `NOWPAYMENTS_IPN_CALLBACK_URL`, and `NOWPAYMENTS_ENVIRONMENT`.
    ///
    /// The environment defaults to `sandbox`. Any value other than `sandbox`
    /// or `production` is rejected, including a present but blank value.
    pub fn from_env() -> PaymentResult<Self> {
        let read = |name: &str| {
            std::env::var(name).map_err(|_| {
                PaymentError::Validation(format!("{name} is missing or is not valid Unicode"))
            })
        };
        let environment = match std::env::var("NOWPAYMENTS_ENVIRONMENT") {
            Ok(value) if value == "sandbox" => NowPaymentsEnvironment::Sandbox,
            Ok(value) if value == "production" => NowPaymentsEnvironment::Production,
            Err(std::env::VarError::NotPresent) => NowPaymentsEnvironment::Sandbox,
            _ => {
                return Err(PaymentError::Validation(
                    "NOWPAYMENTS_ENVIRONMENT must be sandbox or production".into(),
                ));
            }
        };
        Self::new(
            read("NOWPAYMENTS_API_KEY")?,
            read("NOWPAYMENTS_IPN_SECRET")?,
            &read("NOWPAYMENTS_IPN_CALLBACK_URL")?,
            environment,
        )
    }

    /// The environment selected when this provider was constructed.
    pub fn environment(&self) -> NowPaymentsEnvironment {
        self.environment
    }
}

impl PaymentProvider for NowPaymentsProvider {
    fn name(&self) -> &'static str {
        "nowpayments"
    }
}

fn build_client(timeout: Duration) -> PaymentResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .build()
        .map_err(|_| PaymentError::Internal("could not construct NOWPayments HTTP client".into()))
}

fn nonempty(name: &str, value: String) -> PaymentResult<String> {
    if value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(PaymentError::Validation(format!(
            "{name} must be a nonblank credential without control characters"
        )));
    }
    Ok(value)
}

fn https_url(raw: &str, name: &str) -> PaymentResult<Url> {
    let invalid = || {
        PaymentError::Validation(format!(
            "{name} must be an absolute HTTPS URL without credentials or a fragment"
        ))
    };
    if raw.len() > 4096 || raw.trim() != raw || raw.chars().any(char::is_control) {
        return Err(invalid());
    }
    let url = Url::parse(raw).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(invalid());
    }
    Ok(url)
}
