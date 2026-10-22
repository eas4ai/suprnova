//! Dynamic client registration (RFC 7591) for public clients.

use std::time::Duration;

use chrono::Utc;
use serde_json::{Value, json};

use super::client::{ClientMetadata, MetadataProblem, check_public_client};
use super::store::RegisteredClient;
use super::{AuthorizationServer, new_id};
use crate::Result;
use crate::abuse::Permit;

/// The prefix of every client identifier the registration endpoint mints.
/// It never looks like a URL, so it cannot be confused with a client
/// metadata document.
const CLIENT_ID_PREFIX: &str = "mcl_";

/// Why a registration request was refused (RFC 7591 section 3.2.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistrationError {
    /// `invalid_redirect_uri` or `invalid_client_metadata`.
    pub error: &'static str,
    /// What is wrong.
    pub description: String,
}

impl RegistrationError {
    /// The JSON body.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({"error": self.error, "error_description": self.description})
    }
}

impl From<MetadataProblem> for RegistrationError {
    fn from(problem: MetadataProblem) -> Self {
        match problem {
            MetadataProblem::RedirectUri(description) => Self {
                error: "invalid_redirect_uri",
                description,
            },
            MetadataProblem::Metadata(description) => Self {
                error: "invalid_client_metadata",
                description,
            },
        }
    }
}

/// What a registration request comes to.
#[derive(Clone, Debug, PartialEq)]
pub enum RegistrationOutcome {
    /// The client is registered; the body is the registration response.
    Registered(Value),
    /// The metadata was refused.
    Refused(RegistrationError),
    /// The address spent its registration budget.
    RateLimited {
        /// When another registration may be tried.
        retry_after: Duration,
    },
}

impl AuthorizationServer {
    /// Register a public client: redirect URIs checked, no secret, no
    /// client authentication, and at most the registration budget per
    /// client address.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails or the abuse limiter is
    /// unavailable; the limiter fails closed.
    pub async fn register_client(
        &self,
        metadata: ClientMetadata,
        client_address: Option<&str>,
    ) -> Result<RegistrationOutcome> {
        let key = crate::plugins::abuse_key(
            "authorization-server.register",
            client_address.unwrap_or("anonymous"),
        );
        if let Permit::Rejected { retry_after } = self
            .limiter
            .acquire(&key, self.config.registration_policy)
            .await?
        {
            return Ok(RegistrationOutcome::RateLimited { retry_after });
        }
        if metadata.client_id.is_some() {
            return Ok(RegistrationOutcome::Refused(RegistrationError {
                error: "invalid_client_metadata",
                description: "client_id is assigned by the server".to_owned(),
            }));
        }
        let checked = match check_public_client(metadata, &self.declared_scopes()) {
            Ok(checked) => checked,
            Err(problem) => return Ok(RegistrationOutcome::Refused(problem.into())),
        };
        let client = RegisteredClient {
            client_id: format!("{CLIENT_ID_PREFIX}{}", new_id()),
            client_name: checked.client_name,
            redirect_uris: checked.redirect_uris,
            grant_types: checked.grant_types,
            scope: checked.scope,
            client_uri: checked.client_uri,
            logo_uri: checked.logo_uri,
            created_at: Utc::now(),
        };
        self.store.insert_client(client.clone()).await?;
        let mut body = json!({
            "client_id": client.client_id,
            "client_id_issued_at": client.created_at.timestamp(),
            "redirect_uris": client.redirect_uris,
            "grant_types": client.grant_types,
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        });
        if let Value::Object(fields) = &mut body {
            if let Some(name) = client.client_name {
                fields.insert("client_name".to_owned(), Value::String(name));
            }
            if let Some(scope) = client.scope {
                fields.insert("scope".to_owned(), Value::String(scope.join(" ")));
            }
            if let Some(uri) = client.client_uri {
                fields.insert("client_uri".to_owned(), Value::String(uri));
            }
            if let Some(uri) = client.logo_uri {
                fields.insert("logo_uri".to_owned(), Value::String(uri));
            }
        }
        Ok(RegistrationOutcome::Registered(body))
    }
}
