//! The authorization endpoint and the approval it hands to the host's page.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

use super::client::{
    ClientSource, ResolvedClient, check_public_client, is_loopback_redirect,
    names_metadata_document,
};
use super::metadata_document::ClientMetadataError;
use super::store::AuthorizationRecord;
use super::{
    AuthorizationServer, CODE_REUSE_WINDOW, ScopeDefinition, expiry, is_s256_challenge, new_id,
    random_hex, token_digest,
};
use crate::abuse::Permit;
use crate::sessions::VerifiedSession;
use crate::{Error, Result};

pub(super) const PENDING_KIND: &str = "pending";
pub(super) const CODE_KIND: &str = "code";
const PENDING_STATE: &str = "pending";
pub(super) const CODE_ISSUED: &str = "issued";
pub(super) const CODE_REDEEMING_PREFIX: &str = "redeeming:";
pub(super) const CODE_REPLAYED_PREFIX: &str = "replayed:";
pub(super) const CODE_REDEEMED_PREFIX: &str = "redeemed:";

/// One authorization endpoint request.
#[derive(Clone, Copy, Debug)]
pub struct AuthorizeRequest<'a> {
    /// The query parameters.
    pub query: &'a BTreeMap<String, String>,
    /// The signed-in user's session, if there is one.
    pub session: Option<&'a VerifiedSession>,
    /// The client's address, for the metadata-fetch budget.
    pub client_address: Option<&'a str>,
    /// The path and query to come back to after signing in. Only a
    /// relative path is used.
    pub return_to: &'a str,
}

/// What the authorization endpoint answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthorizeOutcome {
    /// The error is shown here and nothing redirects: the client or its
    /// redirect URI cannot be trusted, or a signed-in user's request is
    /// malformed and its redirect URI is not a loopback one.
    Refused {
        /// The OAuth error code.
        error: String,
        /// What is wrong.
        description: String,
    },
    /// Send the browser on: to sign in, to the approval page, or, for a
    /// signed-in user's malformed request, to a loopback redirect URI with
    /// the error, `state` and `iss`.
    Redirect(String),
}

/// A pending authorization as the approval page shows it.
///
/// The client's name is its own claim and every text field is client
/// supplied: a page must escape them. Links are always `https` URLs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingAuthorizationView {
    /// The identifier the approval page posts the decision to.
    pub id: String,
    /// The client identifier.
    pub client_id: String,
    /// The client's name (its own claim) or, without one, its host.
    pub client_name: String,
    /// The host the page shows beside the name: the metadata document's
    /// host, or the redirect URI's for a registered client.
    pub client_host: String,
    /// The client's home page, if it gave one.
    pub client_uri: Option<String>,
    /// The client's logo, if it gave one.
    pub logo_uri: Option<String>,
    /// Where the code will be sent.
    pub redirect_uri: String,
    /// Every requested scope with its description.
    pub scopes: Vec<ScopeDefinition>,
    /// The resource the tokens will be bound to.
    pub resource: String,
    /// When the pending authorization lapses.
    pub expires_at: DateTime<Utc>,
}

impl PendingAuthorizationView {
    /// The JSON the plugin route answers.
    #[must_use]
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "client": {
                "client_id": self.client_id,
                "name": self.client_name,
                "host": self.client_host,
                "client_uri": self.client_uri,
                "logo_uri": self.logo_uri,
            },
            "redirect_uri": self.redirect_uri,
            "scopes": self
                .scopes
                .iter()
                .map(|scope| json!({"name": scope.name, "description": scope.description}))
                .collect::<Vec<_>>(),
            "resource": self.resource,
            "expires_at": self.expires_at.to_rfc3339(),
        })
    }
}

/// What a decision on a pending authorization comes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DecisionOutcome {
    /// Send the browser to the client with the code or `access_denied`.
    Redirect(String),
    /// No such pending authorization for this user, or it lapsed or was
    /// already decided.
    NotFound,
    /// The session's authentication epoch is no longer the user's.
    StaleSession,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PendingAuthorization {
    user_id: String,
    client_id: String,
    client_name: String,
    client_host: String,
    client_uri: Option<String>,
    logo_uri: Option<String>,
    redirect_uri: String,
    scopes: Vec<String>,
    resource: String,
    state: Option<String>,
    code_challenge: String,
    /// Whether the client registered the refresh token grant.
    refresh: bool,
}

/// What an authorization code stands for. The code itself is never
/// stored: the step's identifier is its digest.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(super) struct CodePayload {
    pub user_id: String,
    pub auth_epoch: u64,
    pub client_id: String,
    pub client_name: String,
    pub client_host: String,
    pub redirect_uri: String,
    pub scopes: Vec<String>,
    pub resource: String,
    pub code_challenge: String,
    pub code_expires_at: DateTime<Utc>,
    pub refresh: bool,
}

fn encode<T: Serialize>(value: &T) -> Result<String> {
    serde_json::to_string(value).map_err(|error| Error::Internal {
        message: format!("encode authorization state: {error}"),
    })
}

pub(super) fn decode<T: for<'de> Deserialize<'de>>(json: &str) -> Result<T> {
    serde_json::from_str(json).map_err(|error| Error::Internal {
        message: format!(
            "stored authorization state {}",
            crate::crypto::decode_failure(&error)
        ),
    })
}

/// What the error page says about a refused metadata document. Failures to
/// reach the document collapse into one message, so the endpoint cannot be
/// used to learn which internal names exist or where they point; problems
/// with a document that was fetched are the client's own and are spelled
/// out.
fn refusal_reason(error: &ClientMetadataError) -> String {
    match error {
        ClientMetadataError::Unresolvable(_)
        | ClientMetadataError::ForbiddenAddress(_)
        | ClientMetadataError::Transport(_)
        | ClientMetadataError::TimedOut => {
            "the client metadata document could not be fetched".to_owned()
        }
        other => other.to_string(),
    }
}

/// Append query parameters to a URL the server trusts.
fn with_query(base: &str, params: &[(&str, &str)]) -> Result<String> {
    let mut url = Url::parse(base).map_err(|error| Error::Internal {
        message: format!("redirect target does not parse: {error}"),
    })?;
    {
        let mut pairs = url.query_pairs_mut();
        for (name, value) in params {
            pairs.append_pair(name, value);
        }
    }
    Ok(url.to_string())
}

/// Append one query parameter to a host-relative or absolute address from
/// the configuration.
fn append_parameter(base: &str, name: &str, value: &str) -> String {
    let separator = if base.contains('?') { '&' } else { '?' };
    let pair = url::form_urlencoded::Serializer::new(String::new())
        .append_pair(name, value)
        .finish();
    format!("{base}{separator}{pair}")
}

impl AuthorizationServer {
    /// Find the client a request names: fetch its metadata document for an
    /// `https` identifier, or look up its registration. `Ok(Err(reason))`
    /// means the client cannot be trusted.
    async fn resolve_client(
        &self,
        client_id: &str,
        client_address: Option<&str>,
    ) -> Result<std::result::Result<ResolvedClient, String>> {
        if !names_metadata_document(client_id) {
            return Ok(match self.store.find_client(client_id).await? {
                Some(client) => Ok(ResolvedClient {
                    client_id: client.client_id,
                    source: ClientSource::Registered,
                    client_name: client.client_name,
                    redirect_uris: client.redirect_uris,
                    scope: client.scope,
                    client_uri: client.client_uri,
                    logo_uri: client.logo_uri,
                    grant_types: client.grant_types,
                }),
                None => Err("the client is not registered".to_owned()),
            });
        }
        if !self.fetcher.is_cached(client_id) {
            let key = crate::plugins::abuse_key(
                "authorization-server.client-metadata",
                client_address.unwrap_or("anonymous"),
            );
            if let Permit::Rejected { .. } = self
                .limiter
                .acquire(&key, self.config.metadata_fetch_policy)
                .await?
            {
                return Ok(Err(
                    "too many client metadata lookups from this address; try again later"
                        .to_owned(),
                ));
            }
        }
        let document = match self.fetcher.fetch(client_id).await {
            Ok(document) => document,
            Err(error) => {
                tracing::debug!(client_id, reason = %error, "client metadata document refused");
                return Ok(Err(refusal_reason(&error)));
            }
        };
        Ok(
            match check_public_client(document, &self.declared_scopes()) {
                Ok(checked) => Ok(ResolvedClient {
                    client_id: client_id.to_owned(),
                    source: ClientSource::MetadataDocument,
                    client_name: checked.client_name,
                    redirect_uris: checked.redirect_uris,
                    scope: checked.scope,
                    client_uri: checked.client_uri,
                    logo_uri: checked.logo_uri,
                    grant_types: checked.grant_types,
                }),
                Err(problem) => Err(problem.description().to_owned()),
            },
        )
    }

    /// Whether the session's epoch is still the user's.
    async fn session_is_current(&self, session: &VerifiedSession) -> Result<bool> {
        Ok(self.epochs.current_epoch(session.user_id()).await? == Some(session.auth_epoch()))
    }

    /// Report an authorization request error for a signed-in user: by
    /// redirect, with `state` and `iss`, only to a loopback redirect URI;
    /// otherwise as the refusal page, so no registered client can make this
    /// endpoint redirect to a page of its choosing.
    fn report(
        &self,
        redirect_uri: &str,
        error: &str,
        description: &str,
        state: Option<&str>,
    ) -> Result<AuthorizeOutcome> {
        if !is_loopback_redirect(redirect_uri) {
            return Ok(AuthorizeOutcome::Refused {
                error: error.to_owned(),
                description: description.to_owned(),
            });
        }
        let mut params = vec![("error", error), ("error_description", description)];
        if let Some(state) = state {
            params.push(("state", state));
        }
        params.push(("iss", self.issuer.as_str()));
        Ok(AuthorizeOutcome::Redirect(with_query(
            redirect_uri,
            &params,
        )?))
    }

    /// Answer the authorization endpoint.
    ///
    /// The client and its redirect URI are checked first; a request that
    /// fails there is refused without a redirect, since redirecting would
    /// send the browser somewhere the client never registered. A user who
    /// is not signed in, or whose session is from an earlier epoch, is sent
    /// to sign in next, before any other problem is reported. A signed-in
    /// user's malformed request goes back by redirect, with `state` and
    /// `iss`, only to a loopback redirect URI, and is otherwise refused here
    /// (RFC 9700 section 4.11.2); a valid one becomes a pending
    /// authorization for the approval page.
    ///
    /// # Errors
    ///
    /// Returns an error when storage or the abuse limiter fails.
    pub async fn authorize(&self, request: AuthorizeRequest<'_>) -> Result<AuthorizeOutcome> {
        let param = |name: &str| {
            request
                .query
                .get(name)
                .map(String::as_str)
                .filter(|value| !value.is_empty())
        };
        let refuse = |error: &str, description: &str| {
            Ok(AuthorizeOutcome::Refused {
                error: error.to_owned(),
                description: description.to_owned(),
            })
        };
        let Some(client_id) = param("client_id") else {
            return refuse("invalid_request", "client_id is required");
        };
        let client = match self
            .resolve_client(client_id, request.client_address)
            .await?
        {
            Ok(client) => client,
            Err(reason) => return refuse("invalid_client", &reason),
        };
        let Some(redirect_uri) = param("redirect_uri") else {
            return refuse("invalid_request", "redirect_uri is required");
        };
        if !client.allows_redirect(redirect_uri) {
            return refuse(
                "invalid_request",
                "redirect_uri does not match a registered redirect URI",
            );
        }

        // Sign-in comes before any other answer: the sign-in page is on the
        // application's own domain, and an anonymous visitor must not learn
        // anything, or be sent anywhere, through the client's redirect URI.
        let session = match request.session {
            Some(session) if self.session_is_current(session).await? => session,
            _ => {
                let return_to =
                    // `//host` and `/\host` both name another host to a
                    // browser, so only a path that starts with one `/` and
                    // no second slash of either kind is a return address.
                    if request.return_to.starts_with('/')
                        && !request.return_to.starts_with("//")
                        && !request.return_to.starts_with("/\\")
                    {
                        request.return_to.to_owned()
                    } else {
                        self.route_path("authorize")
                    };
                return Ok(AuthorizeOutcome::Redirect(append_parameter(
                    &self.config.sign_in_url,
                    &self.config.return_to_parameter,
                    &return_to,
                )));
            }
        };

        let state = param("state");
        if param("response_type") != Some("code") {
            return self.report(
                redirect_uri,
                "unsupported_response_type",
                "only response_type=code is supported",
                state,
            );
        }
        let Some(code_challenge) = param("code_challenge") else {
            return self.report(
                redirect_uri,
                "invalid_request",
                "code_challenge is required",
                state,
            );
        };
        if param("code_challenge_method") != Some("S256") {
            return self.report(
                redirect_uri,
                "invalid_request",
                "code_challenge_method must be S256",
                state,
            );
        }
        if !is_s256_challenge(code_challenge) {
            return self.report(
                redirect_uri,
                "invalid_request",
                "code_challenge is not an S256 challenge",
                state,
            );
        }
        let Some(resource_param) = param("resource") else {
            return self.report(
                redirect_uri,
                "invalid_request",
                "resource is required",
                state,
            );
        };
        let Some(resource) = self.accepted_resource(resource_param) else {
            return self.report(
                redirect_uri,
                "invalid_target",
                "the resource is not served here",
                state,
            );
        };
        let scopes = match self.parse_scope(param("scope")) {
            Ok(scopes) => scopes,
            Err(unknown) => {
                return self.report(
                    redirect_uri,
                    "invalid_scope",
                    &format!("scope {unknown:?} is not offered"),
                    state,
                );
            }
        };
        if let Some(limit) = &client.scope
            && let Some(outside) = scopes.iter().find(|scope| !limit.contains(scope))
        {
            return self.report(
                redirect_uri,
                "invalid_scope",
                &format!("the client did not register scope {outside:?}"),
                state,
            );
        }

        let pending = PendingAuthorization {
            user_id: session.user_id().to_owned(),
            client_id: client.client_id.clone(),
            client_name: client.display_name(redirect_uri),
            client_host: client.display_host(redirect_uri),
            client_uri: client.client_uri.clone(),
            logo_uri: client.logo_uri.clone(),
            redirect_uri: redirect_uri.to_owned(),
            scopes,
            resource,
            state: state.map(str::to_owned),
            code_challenge: code_challenge.to_owned(),
            refresh: client.allows_refresh(),
        };
        let id = new_id();
        let now = Utc::now();
        self.store
            .create_authorization(AuthorizationRecord {
                id: id.clone(),
                kind: PENDING_KIND.to_owned(),
                state: PENDING_STATE.to_owned(),
                user_id: pending.user_id.clone(),
                payload: encode(&pending)?,
                expires_at: expiry(now, self.config.pending_lifetime)?,
                created_at: now,
            })
            .await?;
        // Another site can open this endpoint in the user's browser as often
        // as it likes; only the newest pending authorizations are kept.
        self.store
            .trim_authorizations(
                &pending.user_id,
                PENDING_KIND,
                u64::try_from(self.config.max_pending_per_user).unwrap_or(u64::MAX),
                &id,
            )
            .await?;
        Ok(AuthorizeOutcome::Redirect(append_parameter(
            &self.config.approval_url,
            "authorization",
            &id,
        )))
    }

    /// The pending authorization `id`, when it belongs to the session's
    /// user and has not lapsed.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails.
    pub async fn pending_authorization(
        &self,
        id: &str,
        session: &VerifiedSession,
    ) -> Result<Option<PendingAuthorizationView>> {
        let Some(record) = self
            .store
            .find_authorization(id, PENDING_KIND, Utc::now())
            .await?
        else {
            return Ok(None);
        };
        if record.user_id != session.user_id() {
            return Ok(None);
        }
        let pending: PendingAuthorization = decode(&record.payload)?;
        Ok(Some(PendingAuthorizationView {
            id: id.to_owned(),
            client_id: pending.client_id,
            client_name: pending.client_name,
            client_host: pending.client_host,
            client_uri: pending.client_uri,
            logo_uri: pending.logo_uri,
            redirect_uri: pending.redirect_uri,
            scopes: self.describe_scopes(&pending.scopes),
            resource: pending.resource,
            expires_at: record.expires_at,
        }))
    }

    /// Record the user's decision on a pending authorization. Approval
    /// issues a one-time code bound to the client, redirect URI, resource,
    /// scopes, PKCE challenge and the user's current epoch; denial sends
    /// `access_denied`. Either way the redirect carries `state` and `iss`,
    /// and the pending authorization is spent.
    ///
    /// # Errors
    ///
    /// Returns an error when storage fails.
    pub async fn decide(
        &self,
        id: &str,
        session: &VerifiedSession,
        approve: bool,
    ) -> Result<DecisionOutcome> {
        let Some(record) = self
            .store
            .find_authorization(id, PENDING_KIND, Utc::now())
            .await?
        else {
            return Ok(DecisionOutcome::NotFound);
        };
        if record.user_id != session.user_id() {
            return Ok(DecisionOutcome::NotFound);
        }
        let pending: PendingAuthorization = decode(&record.payload)?;
        if !self.session_is_current(session).await? {
            return Ok(DecisionOutcome::StaleSession);
        }
        // Spending the pending authorization is the single-winner step: a
        // double submit decides once.
        if !self
            .store
            .consume_authorization(id, PENDING_KIND, Utc::now())
            .await?
        {
            return Ok(DecisionOutcome::NotFound);
        }
        let mut params: Vec<(&str, &str)> = Vec::new();
        let code;
        if approve {
            code = random_hex();
            let now = Utc::now();
            let payload = CodePayload {
                user_id: pending.user_id.clone(),
                auth_epoch: session.auth_epoch(),
                client_id: pending.client_id.clone(),
                client_name: pending.client_name.clone(),
                client_host: pending.client_host.clone(),
                redirect_uri: pending.redirect_uri.clone(),
                scopes: pending.scopes.clone(),
                resource: pending.resource.clone(),
                code_challenge: pending.code_challenge.clone(),
                code_expires_at: expiry(now, self.config.code_lifetime)?,
                refresh: pending.refresh,
            };
            let window = CODE_REUSE_WINDOW.max(self.config.code_lifetime);
            self.store
                .create_authorization(AuthorizationRecord {
                    id: token_digest(&code),
                    kind: CODE_KIND.to_owned(),
                    state: CODE_ISSUED.to_owned(),
                    user_id: pending.user_id.clone(),
                    payload: encode(&payload)?,
                    expires_at: expiry(now, window)?,
                    created_at: now,
                })
                .await?;
            params.push(("code", code.as_str()));
        } else {
            params.push(("error", "access_denied"));
            params.push(("error_description", "the user denied the request"));
        }
        if let Some(state) = pending.state.as_deref() {
            params.push(("state", state));
        }
        params.push(("iss", self.issuer.as_str()));
        Ok(DecisionOutcome::Redirect(with_query(
            &pending.redirect_uri,
            &params,
        )?))
    }
}
